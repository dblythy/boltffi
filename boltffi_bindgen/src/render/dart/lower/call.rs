//! Renders the Dart source body of a class/record constructor or method.
//!
//! The native `@Native` declaration (symbol, params, return type) is already
//! computed by [`super::native_function`]; this module supplies the missing
//! piece — the marshaling body that encodes public Dart parameters into the
//! native call's argument list and decodes its result back into the public
//! Dart return type. It walks the same [`AbiCall`] the native declaration was
//! built from, reusing the crate's existing wire codec (`emit::emit_writer_write`/
//! `emit::emit_reader_read`/`emit::emit_size_expr`, already proven by the record
//! field codec) rather than inventing a parallel one.

use super::callback::handle_map_instance_name;
use crate::{
    ir::{
        AbiCall, CallMode, CallbackStyle, ErrorTransport, ParamRole, ReturnDef, ReturnShape,
        Transport, TypeExpr,
    },
    render::dart::{DartNativeType, NamingConvention, emit},
};

/// One native argument slot, in native call order.
enum ArgSlot {
    /// A plain expression, e.g. a scalar param passed straight through.
    Expr(String),
    /// The length half of a wire-encoded buffer built for `for_param`. The
    /// pointer half is pushed as a plain `Expr` at the `Input` param's own
    /// position; the length's *expression* is recorded in `len_exprs`
    /// (`Input` and its paired `SyntheticLen` are two separate entries in
    /// `abi_call.params`, so the length can't be computed until this later
    /// entry is reached, but the accessor differs by buffer kind — `.len` on
    /// a `_$$WireWriter`, `.length` on a raw typed list).
    BufLen { for_param: String },
    /// The out-pointer to the scratch `_$$FFIStatus` this call writes into.
    StatusOutPtr,
}

struct CallPlan {
    /// Statements executed before the native call (building writers, status).
    setup: Vec<String>,
    /// Native call argument expressions, in order.
    args: Vec<String>,
    /// Whether a `_$$FFIStatus` scratch value was allocated and needs an
    /// explicit, immediate free (wire-buffer scratch values do not: see
    /// `render_sync_body`).
    has_status_scratch: bool,
    /// Whether `setup` registers at least one callback handle
    /// (`Transport::Callback { style: BoxedDyn, .. }`). When true, `setup`
    /// also contains one `_p$cleanup.add(...)` line per registration, and
    /// the renderer must wrap the whole setup sequence so a later setup
    /// statement's failure rolls every already-registered handle back
    /// (see [`render_setup`]) instead of leaking it in the handle map.
    has_callback_handles: bool,
}

fn buffer_var(param_name: &str) -> String {
    format!("_p$w${param_name}")
}

fn status_var() -> &'static str {
    "_p$status"
}

/// The rollback queue's name — a plain `List<void Function()>`, not a
/// `final` bound to any one handle, so referencing it from the `catch` block
/// [`wrap_with_cleanup`] wraps the body in is always well-defined regardless
/// of which statement threw.
fn cleanup_queue_var() -> &'static str {
    "_p$cleanup"
}

/// Renders `plan.setup` as flat statements, in order. Each callback-handle
/// registration is immediately followed (same position, same `plan.setup`
/// entry list) by a rollback-queue registration — see [`wrap_with_cleanup`],
/// which wraps the *caller's* full assembled body (setup and everything
/// after it) so those queued closures actually get a chance to run.
fn emit_setup(plan: &CallPlan) -> String {
    let mut body = String::new();
    for stmt in &plan.setup {
        body.push_str(stmt);
        body.push('\n');
    }
    body
}

/// Wraps a fully-assembled method body in a rollback `try`/`catch` when its
/// setup registered one or more callback handles.
///
/// `createHandle` inserts into the handle map's `_map` immediately, before
/// the native call. Without this wrapper, a *later* setup statement throwing
/// (an encoded-buffer write, another `createHandle`, ...) would leave that
/// insertion in place forever — nothing else ever calls `.remove()` on it,
/// so the boxed listener stays reachable (and pinned) for the process's
/// lifetime. Each callback-handle setup line (`emit_setup`) is immediately
/// followed by a closure registration
/// (`_p$cleanup.add(() => <handleMap>.remove(<handle>))`) — the closure
/// captures the just-declared `final` local, which is only ever reachable
/// once its own declaration has already run, so this holds regardless of
/// where a later statement fails. The `catch` drains that queue in reverse
/// (matching normal drop order) and rethrows, so the original failure still
/// reaches the caller.
///
/// This must wrap the *whole* body — setup through the native call and
/// decode — not just `setup` on its own: `setup`'s `final` locals (the
/// wire-writer buffers, the boxed callback handles, ...) are declared with
/// block scope, so a `try { setup } catch { ... }` that closes before the
/// native call would take those declarations out of scope right where the
/// call needs to read them (an `undefined_identifier` `dart analyze` error
/// on every method with a callback param, caught by a real-generation
/// regen against parse-core-rs — a live-fixture-only mistake unit tests over
/// string contents alone couldn't catch, since the wrap and its content were
/// individually well-formed).
///
/// The `catch` must not roll back unconditionally, though: the native call
/// itself is the point where Rust takes ownership of any handle this setup
/// registered (a `BoxFromCallbackHandle`/`ArcFromCallbackHandle` impl reads
/// the handle number verbatim, with no re-registration — see
/// `boltffi_macros`'s `trait_export/mod.rs`). A failure *after* the call
/// returns (a fallible constructor's null-handle check, a `StatusOut`
/// check, ...) means Rust already retained the handle — rolling back here
/// would remove it from the Dart-side map while Rust still holds (and will
/// later invoke) it. [`transferred_flag_var`] is flipped to `true`
/// immediately after the call by the caller ([`render_sync_body`]/
/// [`render_async_body`]); the `catch` only drains the queue while it's
/// still `false`, i.e. only for a setup-time failure.
fn wrap_with_cleanup(plan: &CallPlan, body: String) -> String {
    if !plan.has_callback_handles {
        return body;
    }

    let queue = cleanup_queue_var();
    let transferred = transferred_flag_var();
    format!(
        "final {queue} = <void Function()>[];\nvar {transferred} = false;\ntry {{\n{body}}} catch (e) {{\nif (!{transferred}) {{\nfor (final _p$c in {queue}.reversed) {{\n_p$c();\n}}\n}}\nrethrow;\n}}\n"
    )
}

/// The flag [`wrap_with_cleanup`]'s `catch` reads to decide whether the
/// native call already transferred ownership — see its doc comment.
fn transferred_flag_var() -> &'static str {
    "_p$transferred"
}

/// Replaces the standalone identifier `self` with `this` in a generated
/// expression string. Used only where a value-level codec helper renders
/// straight from an embedded `ValueExpr::Named("self")` with no way to pass
/// an override (see the call site).
fn replace_self_identifier(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < expr.len() {
        if expr[i..].starts_with("self") {
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after_idx = i + 4;
            let after_ok = after_idx == expr.len() || !is_ident_byte(bytes[after_idx]);
            if before_ok && after_ok {
                out.push_str("this");
                i = after_idx;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Builds the pre-call setup/argument plan by walking `abi_call.params` in
/// native order. Each [`ParamRole`] maps to exactly one argument slot (or, for
/// `Input` roles carrying wire-encoded data, a pointer+len pair sharing one
/// scratch writer with the paired `SyntheticLen` entry).
///
/// `is_leaf` must be the exact same fact the native `@Native` declaration for
/// this call was rendered with (`DartNativeFunction::is_leaf`,
/// `native_function.rs`'s `lower_one_native_function`) — it governs whether a
/// raw scalar byte buffer (`Transport::Span` with no `encode_ops`, not UTF-8)
/// may take its Dart buffer's `.address` directly. `.address` is legal only
/// as an argument to a leaf native call; a call that lost `isLeaf` (a
/// callback anywhere in the contract may reenter it — see
/// `contract_has_any_callback`) must copy the bytes into calloc'd memory
/// instead, same as the `encode_ops`/UTF-8 arms already do.
fn plan_call(abi_call: &AbiCall, is_leaf: bool) -> CallPlan {
    let mut setup = Vec::new();
    let mut has_status_scratch = false;
    let mut has_callback_handles = false;
    let mut slots = Vec::new();
    let mut len_exprs: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for param in &abi_call.params {
        match &param.role {
            ParamRole::Input {
                transport,
                encode_ops,
                ..
            } => {
                // The receiver (`&self`/`&mut self`/`self`) lowers to an
                // `Input` param literally named "self", for every transport
                // shape (handle, composite, or wire-encoded value-type
                // receivers alike) — Dart has no `self` identifier, so every
                // reference to it reads the enclosing `this` instead.
                let is_receiver = param.name.as_str() == "self";
                let dart_name = if is_receiver {
                    "this".to_string()
                } else {
                    NamingConvention::param_name(param.name.as_str())
                };
                match transport {
                    Transport::Scalar(_) => {
                        // A direct scalar native param (`$$ffi.Bool`,
                        // `$$ffi.Int32`, ...) already marshals to/from the
                        // matching Dart type (`bool`, `int`, ...) — dart:ffi
                        // itself does that conversion. No manual
                        // int/bool juggling here (that's only needed for
                        // *blittable-struct* byte access via `ByteData`,
                        // which has no direct bool accessor — see
                        // `emit::num_as_primitive`/`primitive_as_num`).
                        let expr = if is_cstyle_enum(transport) {
                            format!("{dart_name}.value")
                        } else {
                            dart_name
                        };
                        slots.push(ArgSlot::Expr(expr));
                    }
                    Transport::Handle { nullable, .. } => {
                        let expr = if is_receiver || !*nullable {
                            format!("{dart_name}._handle")
                        } else {
                            format!("({dart_name}?._handle ?? $$ffi.nullptr)")
                        };
                        slots.push(ArgSlot::Expr(expr));
                    }
                    Transport::Composite(_) => {
                        // repr(C) record passed by value: the record's own
                        // blittable struct conversion already exists in
                        // record.txt. `_m$toStruct()` builds its own
                        // throwaway `_$$WireWriter` (a `calloc`'d buffer with
                        // a GC finalizer) and returns a view into it — hoist
                        // the call into a named `setup` local (matching the
                        // `Transport::Span` case below) so that buffer stays
                        // reachable for the whole function body. Calling it
                        // inline as a bare call argument, with nothing else
                        // referencing it, would leave it eligible for GC
                        // (and its finalizer eligible to run) before the
                        // native call reads through the struct-by-value
                        // argument — a real hazard with two or more
                        // composite params in the same call.
                        let var = buffer_var(&dart_name);
                        setup.push(format!("final {var} = {dart_name}._m$toStruct();"));
                        slots.push(ArgSlot::Expr(var));
                    }
                    Transport::Span(content) => {
                        let is_utf8 = matches!(content, crate::ir::SpanContent::Utf8);
                        let var = buffer_var(&dart_name);
                        let len_accessor = if let Some(encode_ops) = encode_ops {
                            // `emit_size_expr` renders straight from the
                            // ops' own embedded `ValueExpr::Named("self")`
                            // (it takes no value-string override, unlike
                            // `emit_writer_write`), so the receiver
                            // substitution has to happen textually here.
                            let size_expr = emit::emit_size_expr(&encode_ops.size);
                            let size_expr = if is_receiver {
                                replace_self_identifier(&size_expr)
                            } else {
                                size_expr
                            };
                            let write_stmt =
                                emit::emit_writer_write(encode_ops, "_p$w", &dart_name);
                            setup.push(format!(
                                "final {var} = _$$WireWriter({size_expr});\n{{ final _p$w = {var}; {write_stmt} }}"
                            ));
                            format!("{var}.len")
                        } else if is_utf8 {
                            setup.push(format!(
                                "final {var} = _$$WireWriter(({dart_name}.length * 3));\n{{ final _p$w = {var}; _p$w.writeTypedList($$convert.utf8.encode({dart_name})); }}"
                            ));
                            format!("{var}.len")
                        } else {
                            // Direct scalar-element buffer (e.g. Vec<u8>). A
                            // `Vec<u8>` is already publicly typed as
                            // `Uint8List` (`DartType::from_type_expr`'s
                            // `Vec<u8>` case) — only non-u8 element vecs (a
                            // plain `List<T>`) need converting to a typed
                            // buffer first, either way `src` below ends up
                            // holding actual `TypedData`.
                            let is_u8 = matches!(
                                content,
                                crate::ir::SpanContent::Scalar(origin)
                                    if origin.primitive() == crate::ir::PrimitiveType::U8
                            );
                            let src = if is_u8 {
                                dart_name.clone()
                            } else {
                                let conv = format!("{var}Src");
                                setup.push(format!(
                                    "final {conv} = $$typed_data.Uint8List.fromList({dart_name});"
                                ));
                                conv
                            };
                            if is_leaf {
                                // Zero-copy: `.address` (below) reads straight
                                // out of `src`'s own backing memory. Legal
                                // only because no callback anywhere in the
                                // contract can reenter this call.
                                setup.push(format!("final {var} = {src};"));
                                format!("{var}.length")
                            } else {
                                // `.address` is illegal here (this call isn't
                                // leaf), so copy into calloc'd native memory
                                // via `_$$WireWriter` instead — the same
                                // buffer technique the `encode_ops`/UTF-8 arms
                                // above already use.
                                setup.push(format!(
                                    "final {var} = _$$WireWriter({src}.length);\n{{ final _p$w = {var}; _p$w.writeTypedList({src}); }}"
                                ));
                                format!("{var}.len")
                            }
                        };
                        let ptr_expr = if encode_ops.is_some() || is_utf8 || !is_leaf {
                            format!("{var}.ptr")
                        } else {
                            format!("{var}.address.cast()")
                        };
                        slots.push(ArgSlot::Expr(ptr_expr));
                        // The paired `SyntheticLen` entry (a *separate*
                        // `AbiParam` later in `abi_call.params`) supplies
                        // this param's length argument slot; record the
                        // expression here since only this branch knows
                        // which accessor the buffer kind needs.
                        len_exprs.insert(dart_name, len_accessor);
                    }
                    Transport::Callback {
                        callback_id,
                        nullable,
                        style: CallbackStyle::BoxedDyn,
                    } => {
                        // Boxes the Dart implementation into a
                        // `_$$BoltFFICallbackHandle` (handle + vtable
                        // pointer, passed by value like a `Composite` param)
                        // through the same handle map `callback.txt` already
                        // builds for the *returning* direction — `createHandle`
                        // registers the impl and clones the shared vtable.
                        let handle_map = handle_map_instance_name(callback_id);
                        let var = buffer_var(&dart_name);
                        let create_expr = format!("{handle_map}.createHandle({dart_name})");
                        let expr = if *nullable {
                            format!(
                                "({dart_name} == null ? ($$ffi.Struct.create<_$$BoltFFICallbackHandle>()..handle = 0..vtable = $$ffi.nullptr) : {create_expr})"
                            )
                        } else {
                            create_expr
                        };
                        setup.push(format!("final {var} = {expr};"));
                        setup.push(format!(
                            "{}.add(() => {handle_map}.remove({var}.handle));",
                            cleanup_queue_var()
                        ));
                        has_callback_handles = true;
                        slots.push(ArgSlot::Expr(var));
                    }
                    Transport::Callback {
                        style: CallbackStyle::ImplTrait,
                        ..
                    } => {
                        // A closure passed *into* a native call would need a
                        // `NativeCallable` minted per call from an arbitrary
                        // runtime closure — `Pointer.fromFunction` (used for
                        // the trait/vtable style above) only accepts a
                        // static function known at compile time. Out of
                        // scope here: this renderer's callback support is
                        // the trait/vtable style (`BoxedDyn`) LiveQuery-style
                        // listeners need.
                        slots.push(ArgSlot::Expr(format!(
                            "(throw UnsupportedError('{dart_name}: closure-typed parameters are not yet supported by this renderer'))"
                        )));
                    }
                }
            }
            ParamRole::SyntheticLen { for_param } => {
                let for_param = if for_param.as_str() == "self" {
                    "this".to_string()
                } else {
                    NamingConvention::param_name(for_param.as_str())
                };
                slots.push(ArgSlot::BufLen { for_param });
            }
            ParamRole::CallbackContext { .. } => {
                slots.push(ArgSlot::Expr("$$ffi.nullptr".to_string()));
            }
            ParamRole::StatusOut => {
                has_status_scratch = true;
                slots.push(ArgSlot::StatusOutPtr);
            }
            // `OutLen`/`OutDirect` describe writing the *return* value through
            // out-parameters instead of the native return slot. None of the
            // return shapes this renderer targets need it (handles, scalars,
            // and encoded buffers all come back through the direct return
            // value); a call that requires it renders a loud, compiling
            // failure rather than a silently wrong body.
            ParamRole::OutLen { .. } | ParamRole::OutDirect => {
                slots.push(ArgSlot::Expr(
                    "(throw UnsupportedError('out-parameter return delivery is not yet supported by this renderer'))"
                        .to_string(),
                ));
            }
        }
    }

    if has_status_scratch {
        setup.insert(
            0,
            format!("final {} = $$extffi.calloc<_$$FFIStatus>();", status_var()),
        );
    }

    let args = slots
        .into_iter()
        .map(|slot| match slot {
            ArgSlot::Expr(e) => e,
            ArgSlot::BufLen { for_param } => len_exprs
                .get(&for_param)
                .cloned()
                .unwrap_or_else(|| format!("{}.len", buffer_var(&for_param))),
            ArgSlot::StatusOutPtr => status_var().to_string(),
        })
        .collect();

    CallPlan {
        setup,
        args,
        has_status_scratch,
        has_callback_handles,
    }
}

fn is_cstyle_enum(transport: &Transport) -> bool {
    matches!(
        transport,
        Transport::Scalar(crate::ir::ScalarOrigin::CStyleEnum { .. })
    )
}

fn last_error_throw_stmt() -> String {
    "throw _$$FFIException(-1, _$$takeLastErrorMessage());".to_string()
}

/// Decodes the raw native call result (`result_expr`) into the public Dart
/// value, given the already-computed native return shape.
///
/// `is_constructor` governs the *encoded-Result* case specifically: a
/// constructor can't declare `BoltFFIResult<...>` as "its own type" (the
/// object it returns *is* the value), so it throws on failure via
/// `BoltFFIResult.okOrThrow()`; an ordinary method's declared return type is
/// always `BoltFFIResult<Ok, Err>` (`DartType::from_return_def`), so it
/// always returns the decoded `BoltFFIResult` as-is, regardless of
/// `dart_return.throws` (which governs the unrelated null-handle case below).
fn decode_return(
    result_expr: &str,
    native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
    error: &ErrorTransport,
    returns: &ReturnShape,
    is_constructor: bool,
) -> String {
    match native_return {
        DartNativeType::Void => String::new(),
        DartNativeType::Status => {
            format!(
                "if ({result_expr}.code != 0) {{ {} }}",
                last_error_throw_stmt()
            )
        }
        DartNativeType::Primitive(_) => {
            // See the matching note in `plan_call`'s `Transport::Scalar` arm:
            // a direct scalar native return already comes back as the
            // matching Dart type, no manual conversion needed here.
            let value = result_expr.to_string();
            let value = match &dart_return.enum_wrap {
                Some(enum_class) => format!("{enum_class}._m$fromValue({value})"),
                None => value,
            };
            format!("return {value};")
        }
        DartNativeType::Pointer(inner) if matches!(**inner, DartNativeType::Void) => {
            let class_name = match &returns.transport {
                Some(Transport::Handle { class_id, .. }) => {
                    NamingConvention::class_name(class_id.as_str())
                }
                _ => dart_return
                    .self_type_name
                    .clone()
                    .unwrap_or_else(|| "dynamic".to_string()),
            };
            let nullable = matches!(returns.transport, Some(Transport::Handle{nullable,..}) if nullable);
            if dart_return.throws {
                format!(
                    "if ({result_expr} == $$ffi.nullptr) {{ {} }}\nreturn {class_name}._({result_expr});",
                    last_error_throw_stmt()
                )
            } else if nullable {
                format!(
                    "if ({result_expr} == $$ffi.nullptr) {{ return null; }}\nreturn {class_name}._({result_expr});"
                )
            } else {
                format!("return {class_name}._({result_expr});")
            }
        }
        DartNativeType::Custom(_) => {
            let record_name = dart_return
                .self_type_name
                .clone()
                .unwrap_or_else(|| "dynamic".to_string());
            format!("return {record_name}._m$fromStruct({result_expr});")
        }
        DartNativeType::OwnedBuffer => {
            let reader_var = "_p$reader";
            let setup = format!(
                "final {reader_var} = _$$WireReader({result_expr}.ptr, {result_expr}.len);"
            );
            // `returns.decode_ops` is already the *complete* decode sequence
            // for whatever crossed in this buffer. When the error is
            // encoded, that sequence is a `ReadOp::Result` covering the tag
            // byte AND both branches as one unit (confirmed against real
            // generated output: `emit_reader_read`'s `ReadOp::Result` arm
            // already renders `reader.readResult(okFn, errFn)`) — it is not
            // "just the Ok payload" needing a manually-read tag byte in
            // front of it. Reading the tag again here, separately, both
            // duplicates the read (misreading the Ok payload's own first
            // byte as a second tag on every successful call) and ignores
            // that `error`'s own `decode_ops` describe the identical error
            // branch already folded into this same sequence.
            let decode_expr = match &returns.decode_ops {
                Some(ops) => emit::emit_reader_read(ops, reader_var),
                None => String::new(),
            };
            let is_encoded_error = matches!(error, ErrorTransport::Encoded { .. });
            let body = if is_encoded_error && is_constructor {
                // A constructor can't declare `BoltFFIResult<...>` as its
                // own return type, so unwrap-or-throw via the same
                // `BoltFFIResult.okOrThrow()` helper `prelude.txt` already
                // exposes for exactly this.
                format!("return ({decode_expr}).okOrThrow();")
            } else {
                format!("return {decode_expr};")
            };
            format!(
                "{setup}\ntry {{\n{body}\n}} finally {{ _f$boltffi_free_buf({result_expr}); }}"
            )
        }
        DartNativeType::CallbackHandle => {
            "throw UnsupportedError('callback-handle returns are not yet supported by this renderer');"
                .to_string()
        }
        DartNativeType::Pointer(_)
        | DartNativeType::Composite(_)
        | DartNativeType::Function { .. } => {
            "throw UnsupportedError('this return shape is not yet supported by this renderer');"
                .to_string()
        }
    }
}

/// Everything the body renderer needs about the *public* return type that
/// isn't already on [`AbiCall`].
pub(super) struct DartReturnInfo {
    /// The class/record name being produced, when the ok/plain value is a
    /// handle or a composite (blittable-struct) value — used to render
    /// `ClassName._(handle)` / `RecordName._m$fromStruct(value)`.
    pub self_type_name: Option<String>,
    /// `Some(EnumClassName)` when the ok/plain value is a c-style enum
    /// returned as a bare scalar, so the raw tag needs `._m$fromValue(...)`.
    pub enum_wrap: Option<String>,
    /// `true` when a handle-typed return should throw (not just return null)
    /// on failure — a `Result<Self, Error>` shape, mirroring the fix already
    /// applied to this fork's TypeScript renderer.
    pub throws: bool,
}

fn self_type_name_of(ty: &TypeExpr) -> Option<String> {
    match ty {
        TypeExpr::Handle(class_id) => Some(NamingConvention::class_name(class_id.as_str())),
        TypeExpr::Record(record_id) => Some(NamingConvention::class_name(record_id.as_str())),
        _ => None,
    }
}

fn enum_wrap_of(ty: &TypeExpr) -> Option<String> {
    match ty {
        TypeExpr::Enum(enum_id) => Some(NamingConvention::class_name(enum_id.as_str())),
        _ => None,
    }
}

impl DartReturnInfo {
    pub(super) fn for_constructor(is_fallible: bool, self_type_name: String) -> Self {
        Self {
            self_type_name: Some(self_type_name),
            enum_wrap: None,
            throws: is_fallible,
        }
    }

    pub(super) fn for_method(returns: &ReturnDef) -> Self {
        let (self_type_name, enum_wrap, throws) = match returns {
            ReturnDef::Void => (None, None, false),
            ReturnDef::Value(ty) => (self_type_name_of(ty), enum_wrap_of(ty), false),
            ReturnDef::Result { ok, .. } => (self_type_name_of(ok), enum_wrap_of(ok), true),
        };
        Self {
            self_type_name,
            enum_wrap,
            throws,
        }
    }
}

/// Renders the full Dart source body (the statements between the method's
/// braces) for a synchronous native call.
fn render_sync_body(
    abi_call: &AbiCall,
    native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
    is_constructor: bool,
    is_leaf: bool,
) -> String {
    let plan = plan_call(abi_call, is_leaf);
    let symbol_fn = format!("_f${}", abi_call.symbol);

    let mut out = emit_setup(&plan);

    let call_expr = format!("{symbol_fn}({})", plan.args.join(", "));

    let needs_result_var = !matches!(native_return, DartNativeType::Void);
    let result_expr = if needs_result_var {
        out.push_str(&format!("final _p$result = {call_expr};\n"));
        "_p$result"
    } else {
        out.push_str(&format!("{call_expr};\n"));
        ""
    };
    if plan.has_callback_handles {
        out.push_str(&format!("{} = true;\n", transferred_flag_var()));
    }

    let mut decode = decode_return(
        result_expr,
        native_return,
        dart_return,
        &abi_call.error,
        &abi_call.returns,
        is_constructor,
    );
    if plan.has_status_scratch {
        // `ParamRole::StatusOut` writes failure into this out-param
        // separately from the return value (which the `StatusOut` role
        // exists precisely because it does *not* also carry a `Status`
        // return type); check it before trusting `result_expr`, or a
        // reported failure is silently swallowed and the (garbage) result
        // gets decoded and returned anyway.
        decode = format!(
            "if ({}.ref.code != 0) {{ {} }}\n{decode}",
            status_var(),
            last_error_throw_stmt()
        );
    }

    // Wire-encoded param buffers are NOT freed here: `_$$WireWriter`'s own
    // factory already attaches a `calloc.nativeFree` `NativeFinalizer` to the
    // typed-list view backing the buffer (see `prelude.txt`), so an explicit
    // free here would race the GC-driven one and double-free the same
    // pointer. Only the bare `calloc<_$$FFIStatus>()` scratch value (no
    // finalizer attached — it's a plain struct alloc, not a `_$$WireWriter`)
    // needs an explicit, immediate free.
    if plan.has_status_scratch {
        out.push_str("try {\n");
        out.push_str(&decode);
        out.push_str("\n} finally {\n");
        out.push_str(&format!("$$extffi.calloc.free({});\n", status_var()));
        out.push_str("}\n");
    } else {
        out.push_str(&decode);
        out.push('\n');
    }

    wrap_with_cleanup(&plan, out)
}

/// Renders the full Dart source body for an async native call, driving the
/// already-implemented `_$$BoltFFIAsync.create` poll/complete/cancel/free
/// protocol (`prelude.txt`) rather than a new one.
fn render_async_body(
    abi_call: &AbiCall,
    async_call: &crate::ir::AsyncCall,
    complete_native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
    is_leaf: bool,
) -> String {
    let plan = plan_call(abi_call, is_leaf);

    if plan.has_status_scratch {
        // A `StatusOut` role on the *create* call itself (as opposed to the
        // complete call, which always carries one — handled below) would
        // need its scratch value freed after `createFuture()` returns, but
        // that call happens lazily inside `_$$BoltFFIAsync.create`, outside
        // this function's own try/finally shape. Not seen in practice yet;
        // rendering a loud failure instead of a silent leak until a real
        // case justifies the extra plumbing.
        return "throw UnsupportedError('a status-out parameter on the initiating call of an async method is not yet supported by this renderer');\n".to_string();
    }

    let symbol_fn = format!("_f${}", abi_call.symbol);
    let poll_fn = format!("_f${}", async_call.poll);
    let complete_fn = format!("_f${}", async_call.complete);
    let cancel_fn = format!("_f${}", async_call.cancel);
    let free_fn = format!("_f${}", async_call.free);

    let mut out = emit_setup(&plan);

    let create_args = plan.args.join(", ");

    // The `complete` native declaration (`native_function.txt`) always takes
    // an out-param `Pointer<_$$FFIStatus>` as its second argument — a
    // panic/unexpected-failure channel that exists independently of whatever
    // `async_call.error` says about the call's own `Result` shape — so this
    // scratch value is allocated unconditionally, not gated on
    // `plan.has_status_scratch` (which only reflects roles seen in
    // `abi_call.params`, i.e. the *create* call's params, never the fixed
    // complete-call signature).
    let needs_result_var = !matches!(complete_native_return, DartNativeType::Void);
    let complete_call = format!("{complete_fn}(handle, _p$asyncStatus)");

    let decode = decode_return(
        if needs_result_var { "_p$asyncResult" } else { "" },
        complete_native_return,
        dart_return,
        &async_call.error,
        &async_call.result,
        // Async constructors are rejected before this function is ever
        // reached (see `render_body`) — every caller here is a method.
        false,
    );

    let fetch_stmt = if needs_result_var {
        format!("final _p$asyncResult = {complete_call};")
    } else {
        format!("{complete_call};")
    };

    let complete_stmt = format!(
        "final _p$asyncStatus = $$extffi.calloc<_$$FFIStatus>();\ntry {{\n{fetch_stmt}\nif (_p$asyncStatus.ref.code != 0) {{ {} }}\n{decode}\n}} finally {{\n$$extffi.calloc.free(_p$asyncStatus);\n}}",
        last_error_throw_stmt()
    );

    // The *create* call is where Rust takes ownership of any callback
    // handle this method's setup registered (same reasoning as the sync
    // path — see `wrap_with_cleanup`) — flip the flag the instant it
    // returns, before anything that could still throw (the `complete`/poll
    // protocol below runs later, independently of setup's rollback).
    let create_call_expr = format!("{symbol_fn}({create_args})");
    let create_future = if plan.has_callback_handles {
        format!(
            "() {{\n    final _p$createResult = {create_call_expr};\n    {} = true;\n    return _p$createResult;\n  }}",
            transferred_flag_var()
        )
    } else {
        format!("() => {create_call_expr}")
    };

    out.push_str(&format!(
        "return _$$BoltFFIAsync.create(\n  createFuture: {create_future},\n  pollFuture: {poll_fn},\n  completeFuture: (handle) {{\n{complete_stmt}\n  }},\n  freeFuture: {free_fn},\n  cancelFuture: {cancel_fn},\n);\n"
    ));

    wrap_with_cleanup(&plan, out)
}

/// `is_constructor` matters because Dart `factory` constructors cannot be
/// `async` or return a `Future` — an async constructor call renders a loud,
/// compiling failure (rather than a silently wrong synchronous body) until
/// this renderer grows the static-factory-method reshaping such a source API
/// needs.
pub(super) fn render_body(
    abi_call: &AbiCall,
    native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
    is_constructor: bool,
    is_leaf: bool,
) -> String {
    match &abi_call.mode {
        CallMode::Sync => {
            render_sync_body(abi_call, native_return, dart_return, is_constructor, is_leaf)
        }
        CallMode::Async(_) if is_constructor => {
            "throw UnsupportedError('async constructors are not representable as a Dart factory constructor; this renderer does not yet reshape them into a static async factory method');\n".to_string()
        }
        CallMode::Async(async_call) => render_async_body(
            abi_call,
            async_call,
            &async_call_complete_native_type(async_call),
            dart_return,
            is_leaf,
        ),
    }
}

fn async_call_complete_native_type(async_call: &crate::ir::AsyncCall) -> DartNativeType {
    DartNativeType::from_return_shape_and_error_transport(&async_call.result, &async_call.error)
}

#[cfg(test)]
mod tests {
    use boltffi_ffi_rules::callable::ExecutionKind;

    use crate::{
        ir::{
            CallbackId, CallbackKind, CallbackMethodDef, CallbackTraitDef, ClassDef, ClassId,
            ConstructorDef, FunctionDef, FunctionId, MethodDef, MethodId, ParamDef, ParamName,
            ParamPassing, PrimitiveType, Receiver, RecordDef, ReturnDef, TypeExpr,
        },
        render::dart::test,
    };

    fn bytes_param(name: &str) -> ParamDef {
        ParamDef {
            name: ParamName::new(name),
            type_expr: TypeExpr::Vec(Box::new(TypeExpr::Primitive(PrimitiveType::U8))),
            passing: ParamPassing::Value,
            doc: None,
        }
    }

    fn class_with_a_bytes_setter(class_id: &str, method_id: &str) -> ClassDef {
        ClassDef {
            qualified_path: String::new(),
            id: ClassId::new(class_id),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new(method_id),
                receiver: Receiver::RefSelf,
                params: vec![bytes_param("bytes")],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        }
    }

    fn string_param(name: &str) -> ParamDef {
        ParamDef {
            name: ParamName::new(name),
            type_expr: TypeExpr::String,
            passing: ParamPassing::Value,
            doc: None,
        }
    }

    // Regression (parse-core-sdks pin-roll, task-7 follow-up): a raw
    // scalar-element byte buffer (`Vec<u8>`, e.g. `ParseObject.setBytes`)
    // zero-copies via `.address` on its Dart typed-list argument -- legal
    // ONLY on a leaf native call (Dart's own FFI rule). Once ANY callback
    // exists anywhere in the contract, `contract_has_any_callback` forces
    // every sync native non-leaf (fixing the cross-class reentrancy bug),
    // including a class with no callback param anywhere near it, like this
    // one. The two constraints are mutually exclusive on the SAME call if
    // the renderer doesn't switch technique: `.address` on a non-leaf call
    // is a Dart compile error, not a runtime one -- reproduced for real by
    // `verify-dart.sh` going red on `set_storage_encryption_key`/
    // `upload_file`/`upload_file_with_options`/`ParseObject.setBytes` after
    // the crate-wide fix landed. The method must still compile: copy into
    // calloc'd memory via `_$$WireWriter` instead of taking `.address`.
    #[test]
    fn bytes_param_copies_into_a_wire_writer_instead_of_address_when_a_callback_exists_anywhere() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_callback(CallbackTraitDef {
            qualified_path: String::new(),
            id: CallbackId::new("ObjectObserver"),
            methods: vec![CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: MethodId::new("on_object_changed"),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: CallbackKind::Trait,
            doc: None,
        });
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("set_object_observer"),
            params: vec![ParamDef {
                name: ParamName::new("observer"),
                type_expr: TypeExpr::Callback(CallbackId::new("ObjectObserver")),
                passing: ParamPassing::BoxedDyn,
                doc: None,
            }],
            returns: ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });
        ffi.catalog
            .insert_class(class_with_a_bytes_setter("ParseObject", "set_bytes"));

        let library = test::lower(&ffi);
        let method = &library.classes[0].methods[0];

        assert!(
            !method.native.is_leaf,
            "a callback exists elsewhere in the contract -- must not be leaf"
        );
        assert!(
            !method.body.contains(".address"),
            "`.address` is illegal on a non-leaf native call -- must not appear: {}",
            method.body
        );
        assert!(
            method.body.contains("_$$WireWriter(") && method.body.contains(".ptr"),
            "must copy the buffer through _$$WireWriter instead: {}",
            method.body
        );
    }

    // Contrast: the same bytes-taking method, alone in a contract with NO
    // callback anywhere, keeps the zero-copy `.address` fast path -- the
    // perf cost of the fix above is confined to callback-bearing contracts.
    #[test]
    fn bytes_param_keeps_the_zero_copy_address_fast_path_when_no_callback_exists() {
        let mut ffi = test::empty_contract();
        ffi.catalog
            .insert_class(class_with_a_bytes_setter("PlainStore", "set_bytes"));

        let library = test::lower(&ffi);
        let method = &library.classes[0].methods[0];

        assert!(method.native.is_leaf, "no callback anywhere -- stays leaf");
        assert!(
            method.body.contains(".address"),
            "no callback in this contract -- must keep the zero-copy fast path: {}",
            method.body
        );
    }

    #[test]
    fn sync_constructor_with_string_and_scalar_params_builds_one_writer_and_wraps_handle() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Person"),
            constructors: vec![ConstructorDef::Default {
                params: vec![
                    string_param("name"),
                    ParamDef {
                        name: ParamName::new("age"),
                        type_expr: TypeExpr::Primitive(PrimitiveType::U32),
                        passing: ParamPassing::Value,
                        doc: None,
                    },
                ],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].constructors[0].body;

        // Regression: the length argument must appear exactly once (the
        // `Input` param and its paired `SyntheticLen` AbiParam are two
        // separate native-call slots, not two lengths to compute).
        assert_eq!(
            body.matches("_p$w$name.len").count(),
            1,
            "body: {body}"
        );
        assert!(
            body.contains("_f$boltffi_person_new(_p$w$name.ptr, _p$w$name.len, age)"),
            "body: {body}"
        );
        assert!(body.contains("return Person._(_p$result);"), "body: {body}");
        // The WireWriter's own GC finalizer owns its buffer — no explicit
        // free here (double-free guard).
        assert!(!body.contains("calloc.free"), "body: {body}");
    }

    #[test]
    fn fallible_constructor_throws_on_null_handle() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Widget"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: true,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].constructors[0].body;

        assert!(
            body.contains("if (_p$result == $$ffi.nullptr)")
                && body.contains("_$$takeLastErrorMessage()"),
            "body: {body}"
        );
        assert!(body.contains("throw _$$FFIException"), "body: {body}");
    }

    #[test]
    fn method_returning_result_with_encoded_error_wraps_bolt_ffi_result() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_record(RecordDef {
            qualified_path: String::new(),
            id: crate::ir::RecordId::new("AppError"),
            is_repr_c: false,
            is_error: true,
            fields: vec![crate::ir::FieldDef {
                name: crate::ir::FieldName::new("message"),
                type_expr: TypeExpr::String,
                doc: None,
                default: None,
            }],
            constructors: vec![],
            methods: vec![],
            doc: None,
            deprecated: None,
        });
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Widget"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("rename"),
                receiver: Receiver::RefSelf,
                params: vec![string_param("name")],
                returns: ReturnDef::Result {
                    ok: TypeExpr::Void,
                    err: TypeExpr::Record(crate::ir::RecordId::new("AppError")),
                },
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].methods[0].body;

        // Exactly one `readResult` call: `returns.decode_ops` for an
        // encoded-error `Result` is already the whole tag+ok+err decode as
        // one unit. A regression here previously read the tag a *second*
        // time manually before also calling `readResult` (which reads its
        // own tag byte) — silently misreading the Ok payload's first byte
        // as a second tag on every successful call, invisible to
        // `dart analyze` since both branches are `BoltFFIResult`-typed.
        assert_eq!(body.matches("readResult").count(), 1, "body: {body}");
        assert!(!body.contains("_p$tag"), "body: {body}");
        // A method's declared return type is always `BoltFFIResult<T, E>`
        // (never a bare throw) — `okOrThrow()` is constructor-only.
        assert!(!body.contains("okOrThrow"), "body: {body}");
        assert!(body.contains("_f$boltffi_free_buf(_p$result);"), "body: {body}");
        // The receiver reads `this`, never the Rust-side "self" identifier.
        assert!(body.contains("this._handle"), "body: {body}");
        assert!(!body.contains(" self"), "body should not leak `self`: {body}");
    }

    #[test]
    fn fallible_constructor_with_encoded_error_unwraps_or_throws() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_record(RecordDef {
            qualified_path: String::new(),
            id: crate::ir::RecordId::new("Acl"),
            is_repr_c: false,
            is_error: false,
            fields: vec![crate::ir::FieldDef {
                name: crate::ir::FieldName::new("owner"),
                type_expr: TypeExpr::String,
                doc: None,
                default: None,
            }],
            constructors: vec![ConstructorDef::Default {
                params: vec![string_param("owner")],
                is_fallible: true,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.records[0].constructors[0].body;

        // A constructor can't declare `BoltFFIResult<...>` as its own return
        // type, so — unlike a method — it unwraps via `okOrThrow()` (which
        // itself calls `readResult` exactly once internally).
        assert_eq!(body.matches("readResult").count(), 1, "body: {body}");
        assert!(body.contains("okOrThrow()"), "body: {body}");
        assert!(!body.contains("_p$tag"), "body: {body}");
    }

    #[test]
    fn async_method_drives_bolt_ffi_async_create() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Widget"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("fetch_count"),
                receiver: Receiver::RefSelf,
                params: vec![],
                returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::U32)),
                execution_kind: ExecutionKind::Async,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let method = &library.classes[0].methods[0];

        assert!(method.is_async);
        assert!(method.body.contains("_$$BoltFFIAsync.create("), "body: {}", method.body);
        assert!(
            method.body.contains("completeFuture: (handle)"),
            "body: {}",
            method.body
        );
        // The complete call always takes the out-status pointer, unconditionally.
        assert!(
            method.body.contains("(handle, _p$asyncStatus)"),
            "body: {}",
            method.body
        );
    }

    fn callback_trait(id: &str, method_id: &str) -> crate::ir::CallbackTraitDef {
        crate::ir::CallbackTraitDef {
            qualified_path: String::new(),
            id: crate::ir::CallbackId::new(id),
            methods: vec![crate::ir::CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: crate::ir::MethodId::new(method_id),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: crate::ir::CallbackKind::Trait,
            doc: None,
        }
    }

    #[test]
    fn boxed_dyn_callback_param_is_boxed_through_the_handle_map() {
        let mut ffi = test::empty_contract();
        ffi.catalog
            .insert_callback(callback_trait("Listener", "on_event"));
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Subject"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("subscribe"),
                receiver: Receiver::RefSelf,
                params: vec![ParamDef {
                    name: ParamName::new("listener"),
                    type_expr: TypeExpr::Callback(crate::ir::CallbackId::new("Listener")),
                    passing: ParamPassing::BoxedDyn,
                    doc: None,
                }],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].methods[0].body;

        assert!(
            body.contains("_k$ListenerHandleMap.createHandle(listener)"),
            "body: {body}"
        );
        assert!(!body.contains("UnsupportedError"), "body: {body}");
    }

    // Regression: `createHandle` inserts into the handle map's `_map` before
    // the native call happens. If the call has a *second* param whose own
    // setup throws (an encoded-buffer write, another `createHandle`, ...),
    // the first handle is already inserted and nothing ever calls
    // `.remove()` on it — a leaked entry pinning the Dart listener object
    // forever. Codex review finding (2026-07-20): the setup sequence must
    // roll back any handle it already registered before letting the
    // exception propagate.
    #[test]
    fn boxed_dyn_callback_param_setup_registers_a_rollback_on_later_setup_failure() {
        let mut ffi = test::empty_contract();
        ffi.catalog
            .insert_callback(callback_trait("Listener", "on_event"));
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Subject"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("subscribe"),
                receiver: Receiver::RefSelf,
                params: vec![
                    ParamDef {
                        name: ParamName::new("listener"),
                        type_expr: TypeExpr::Callback(crate::ir::CallbackId::new("Listener")),
                        passing: ParamPassing::BoxedDyn,
                        doc: None,
                    },
                    ParamDef {
                        name: ParamName::new("label"),
                        type_expr: TypeExpr::String,
                        passing: ParamPassing::Value,
                        doc: None,
                    },
                ],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].methods[0].body;

        // The handle is registered, and a rollback closure for it is queued
        // immediately after — before any later param's own setup (which
        // might throw) runs.
        let create_pos = body
            .find("_k$ListenerHandleMap.createHandle(listener)")
            .expect(body);
        let cleanup_add_pos = body.find("_p$cleanup.add(").expect(body);
        assert!(
            cleanup_add_pos > create_pos,
            "the rollback closure must be queued right after the handle is created: {body}"
        );
        assert!(
            body.contains("_k$ListenerHandleMap.remove("),
            "the rollback closure must call the same handle map's remove: {body}"
        );
        // The whole setup sequence (including the later param's own writer
        // setup) runs inside a try whose catch drains the rollback queue —
        // in reverse, matching normal drop order — then rethrows so the
        // original failure still surfaces to the caller.
        assert!(body.contains("try {"), "body: {body}");
        assert!(
            body.contains("_p$cleanup.reversed"),
            "body: {body}"
        );
        assert!(body.contains("rethrow;"), "body: {body}");
        // Regression: an earlier version of this fix wrapped only the
        // *setup* statements in `try { ... }`, closing the block — and
        // taking `final _p$w$listener`/`_p$w$label` out of scope — before
        // the native call that reads them. Undetectable by string-content
        // assertions alone (a real `dart analyze` regen against
        // parse-core-rs caught it as ~49 `undefined_identifier` errors);
        // pin the structural fact directly: the native call site must be
        // textually *inside* the try block, not after it closes.
        let try_pos = body.find("try {").expect(body);
        let call_pos = body
            .find("_f$boltffi_subject_subscribe(")
            .expect(body);
        let try_close_pos = body.rfind("} catch (e) {").expect(body);
        assert!(
            try_pos < call_pos && call_pos < try_close_pos,
            "the native call must be inside the try block, not after its locals go out of scope: {body}"
        );
    }

    // Regression (Codex review finding, 2026-07-20): the rollback `catch`
    // used to unconditionally drain the cleanup queue on ANY throw,
    // including one that happens *after* the native call already
    // transferred the callback handle's ownership to Rust (e.g. a fallible
    // constructor's own null-handle check). Rolling back in that case
    // removes the handle from the Dart-side map while Rust is still holding
    // (and will later invoke) it — every subsequent call against that
    // handle then misses. The native call must disarm the rollback the
    // moment it returns, so only a *setup* failure (before the call) rolls
    // anything back.
    #[test]
    fn boxed_dyn_callback_param_rollback_does_not_run_after_the_native_call_succeeds() {
        let mut ffi = test::empty_contract();
        ffi.catalog
            .insert_callback(callback_trait("Listener", "on_event"));
        ffi.catalog.insert_class(ClassDef {
            id: ClassId::new("Widget"),
            qualified_path: String::new(),
            constructors: vec![ConstructorDef::Default {
                params: vec![ParamDef {
                    name: ParamName::new("listener"),
                    type_expr: TypeExpr::Callback(crate::ir::CallbackId::new("Listener")),
                    passing: ParamPassing::BoxedDyn,
                    doc: None,
                }],
                is_fallible: true,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].constructors[0].body;

        assert!(
            body.contains("_k$ListenerHandleMap.createHandle(listener)"),
            "body: {body}"
        );

        // A flag, declared outside the try, starts false and flips to true
        // right after the native call — before the fallible decode's own
        // null-handle check (which can also throw) ever runs.
        assert!(
            body.contains("var _p$transferred = false;"),
            "body: {body}"
        );
        let result_pos = body.find("final _p$result = ").expect(body);
        let transferred_set_pos = body.find("_p$transferred = true;").expect(body);
        let null_check_pos = body.find("if (_p$result == $$ffi.nullptr)").expect(body);
        assert!(
            result_pos < transferred_set_pos && transferred_set_pos < null_check_pos,
            "the native call's result must be captured, then the flag flipped, \
             before the decode's own throwing check: {body}"
        );

        // The catch only drains the rollback queue when the call itself
        // never went through.
        assert!(
            body.contains("if (!_p$transferred) {"),
            "body: {body}"
        );
    }

    #[test]
    fn nullable_boxed_dyn_callback_param_falls_back_to_a_zero_handle() {
        let mut ffi = test::empty_contract();
        ffi.catalog
            .insert_callback(callback_trait("Listener", "on_event"));
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Subject"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("subscribe"),
                receiver: Receiver::RefSelf,
                params: vec![ParamDef {
                    name: ParamName::new("listener"),
                    type_expr: TypeExpr::Option(Box::new(TypeExpr::Callback(
                        crate::ir::CallbackId::new("Listener"),
                    ))),
                    passing: ParamPassing::BoxedDyn,
                    doc: None,
                }],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].methods[0].body;

        assert!(body.contains("listener == null ?"), "body: {body}");
        assert!(
            body.contains("_k$ListenerHandleMap.createHandle(listener)"),
            "body: {body}"
        );
        assert!(body.contains("..handle = 0"), "body: {body}");
    }

    // Regression (Codex review finding, 2026-07-20): a function/method that
    // *returns* a callback handle (`Box<dyn Trait>` handed back to the
    // caller, as opposed to a callback *param* — this renderer has no
    // "returned callback" client wrapper yet, unlike e.g. the JNI bridge's
    // dedicated `CallbackHandleMethod` machinery for exactly this
    // direction) compiles a Dart method that `throw`s an `UnsupportedError`
    // the instant it's actually called.
    //
    // A generation-time panic was tried and reverted: it aborts the whole
    // render pass for the entire crate, not just this one function —
    // confirmed against the fork's own `examples/demo` fixture
    // (`make_incrementing_callback`, exactly this shape), whose Dart target
    // stopped generating at all. Reverted to the established per-function
    // runtime-throw convention this file already uses in several other
    // "unsupported shape" arms (`ImplTrait` closure params, out-parameter
    // returns, ...).
    #[test]
    fn function_returning_a_callback_handle_still_throws_at_runtime_not_generation_time() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_callback(callback_trait("Listener", "on_event"));
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("make_listener"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::Callback(crate::ir::CallbackId::new("Listener"))),
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.functions[0].body;

        assert!(
            body.contains("throw UnsupportedError('callback-handle returns are not yet supported by this renderer');"),
            "body: {body}"
        );
    }

    // Regression guard: a closure-style (`ImplTrait`) callback param must
    // keep failing loudly — it has no handle map to box into (only
    // `CallbackKind::Trait` callbacks get one; `lower_callbacks` skips
    // `Closure`), and boxing it as if it were `BoxedDyn` would reference a
    // handle map that was never generated.
    #[test]
    fn impl_trait_callback_param_still_reports_unsupported() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_callback(crate::ir::CallbackTraitDef {
            qualified_path: String::new(),
            id: crate::ir::CallbackId::new("ClosureCb"),
            methods: vec![crate::ir::CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: crate::ir::MethodId::new("call"),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: crate::ir::CallbackKind::Closure,
            doc: None,
        });
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Subject"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("on_tick"),
                receiver: Receiver::RefSelf,
                params: vec![ParamDef {
                    name: ParamName::new("cb"),
                    type_expr: TypeExpr::Callback(crate::ir::CallbackId::new("ClosureCb")),
                    passing: ParamPassing::ImplTrait,
                    doc: None,
                }],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let body = &library.classes[0].methods[0].body;

        assert!(body.contains("UnsupportedError"), "body: {body}");
        assert!(!body.contains("HandleMap"), "body: {body}");
    }
}
