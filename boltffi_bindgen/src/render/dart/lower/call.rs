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

use crate::{
    ir::{AbiCall, CallMode, ErrorTransport, ParamRole, ReturnDef, ReturnShape, Transport, TypeExpr},
    render::dart::{DartNativeType, NamingConvention, emit},
};

/// One native argument slot, in native call order.
enum ArgSlot {
    /// A plain expression, e.g. a scalar param passed straight through.
    Expr(String),
    /// The pointer half of a wire-encoded buffer built for `for_param`.
    BufPtr { for_param: String },
    /// The length half of a wire-encoded buffer built for `for_param`.
    BufLen { for_param: String },
    /// The out-pointer to the scratch `_$$FFIStatus` this call writes into.
    StatusOutPtr,
}

struct CallPlan {
    /// Statements executed before the native call (building writers, status).
    setup: Vec<String>,
    /// Native call argument expressions, in order.
    args: Vec<String>,
    /// Whether a `_$$FFIStatus` scratch value was allocated and needs freeing.
    has_status_scratch: bool,
    /// Whether any wire buffers were allocated and need freeing.
    buffer_vars: Vec<String>,
}

fn buffer_var(param_name: &str) -> String {
    format!("_p$w${param_name}")
}

fn status_var() -> &'static str {
    "_p$status"
}

/// Builds the pre-call setup/argument plan by walking `abi_call.params` in
/// native order. Each [`ParamRole`] maps to exactly one argument slot (or, for
/// `Input` roles carrying wire-encoded data, a pointer+len pair sharing one
/// scratch writer with the paired `SyntheticLen` entry).
fn plan_call(abi_call: &AbiCall) -> CallPlan {
    let mut setup = Vec::new();
    let mut buffer_vars = Vec::new();
    let mut has_status_scratch = false;
    let mut slots = Vec::new();

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
                    Transport::Scalar(origin) => {
                        let expr = if origin.primitive() == crate::ir::PrimitiveType::Bool {
                            format!("({dart_name} ? 1 : 0)")
                        } else {
                            match &param.abi_type {
                                _ if is_cstyle_enum(transport) => format!("{dart_name}.value"),
                                _ => dart_name,
                            }
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
                        // record.txt.
                        slots.push(ArgSlot::Expr(format!("{dart_name}._m$toStruct()")));
                    }
                    Transport::Span(content) => {
                        let is_utf8 = matches!(content, crate::ir::SpanContent::Utf8);
                        if let Some(encode_ops) = encode_ops {
                            // `emit_size_expr` renders straight from the ops'
                            // own embedded `ValueExpr::Named("self")` (it
                            // takes no value-string override, unlike
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
                            let var = buffer_var(&dart_name);
                            setup.push(format!(
                                "final {var} = _$$WireWriter({size_expr});\n{{ final _p$w = {var}; {write_stmt} }}"
                            ));
                            buffer_vars.push(var);
                        } else if is_utf8 {
                            let var = buffer_var(&dart_name);
                            setup.push(format!(
                                "final {var} = _$$WireWriter(({dart_name}.length * 3));\n{{ final _p$w = {var}; _p$w.writeTypedList($$convert.utf8.encode({dart_name})); }}"
                            ));
                            buffer_vars.push(var);
                        } else {
                            // Direct scalar-element buffer (e.g. Vec<u8>):
                            // pass the typed list's own backing memory.
                            let var = buffer_var(&dart_name);
                            setup.push(format!(
                                "final {var} = {dart_name} is $$typed_data.Uint8List ? {dart_name} : $$typed_data.Uint8List.fromList({dart_name});"
                            ));
                            slots.push(ArgSlot::Expr(format!("{var}.address.cast()")));
                            slots.push(ArgSlot::Expr(format!("{var}.length")));
                            continue;
                        }
                        slots.push(ArgSlot::BufPtr {
                            for_param: dart_name.clone(),
                        });
                        slots.push(ArgSlot::BufLen {
                            for_param: dart_name,
                        });
                    }
                    Transport::Callback { .. } => {
                        // Passing a Dart closure/interface INTO a native call
                        // (as opposed to a stream/async completion, both of
                        // which already work) is not yet implemented here.
                        slots.push(ArgSlot::Expr(format!(
                            "(throw UnsupportedError('{dart_name}: callback-typed parameters are not yet supported by this renderer'))"
                        )));
                    }
                }
            }
            ParamRole::SyntheticLen { for_param } => {
                let for_param = NamingConvention::param_name(for_param.as_str());
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
            ArgSlot::BufPtr { for_param } => format!("{}.ptr", buffer_var(&for_param)),
            ArgSlot::BufLen { for_param } => format!("{}.len", buffer_var(&for_param)),
            ArgSlot::StatusOutPtr => status_var().to_string(),
        })
        .collect();

    CallPlan {
        setup,
        args,
        has_status_scratch,
        buffer_vars,
    }
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

fn is_cstyle_enum(transport: &Transport) -> bool {
    matches!(
        transport,
        Transport::Scalar(crate::ir::ScalarOrigin::CStyleEnum { .. })
    )
}

fn last_error_throw_stmt() -> String {
    "throw _$$FFIException(-1, _$$takeLastErrorMessage());".to_string()
}

/// Decodes the raw native call result (`resultExpr`) into the public Dart
/// value, given the already-computed native return shape.
///
/// `wrap_handle` is `Some(class_name)` for a class-typed return (constructor
/// or method); `throws_on_absent` controls whether a null handle throws
/// (fallible) or returns `null` (a plain optional handle, no failure
/// implied).
#[allow(clippy::too_many_arguments)]
fn decode_return(
    result_expr: &str,
    native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
    error: &ErrorTransport,
    returns: &ReturnShape,
) -> String {
    match native_return {
        DartNativeType::Void => String::new(),
        DartNativeType::Status => {
            format!(
                "if ({result_expr}.code != 0) {{ {} }}",
                last_error_throw_stmt()
            )
        }
        DartNativeType::Primitive(p) => {
            let value = emit::num_as_primitive(*p, result_expr);
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
            let body = match error {
                ErrorTransport::Encoded { decode_ops, .. } => {
                    let ok_expr = match &returns.decode_ops {
                        Some(ok_ops) => emit::emit_reader_read(ok_ops, reader_var),
                        None => String::new(),
                    };
                    let err_expr = emit::emit_reader_read(decode_ops, reader_var);
                    if dart_return.throws {
                        format!(
                            "final _p$tag = {reader_var}.readU8();\nif (_p$tag == 1) {{ throw {err_expr}; }}\nreturn {ok_expr};"
                        )
                    } else {
                        format!(
                            "return {reader_var}.readResult((({reader_var}) => {ok_expr}), (({reader_var}) => {err_expr}));"
                        )
                    }
                }
                _ => {
                    let ok_expr = match &returns.decode_ops {
                        Some(ok_ops) => emit::emit_reader_read(ok_ops, reader_var),
                        None => String::new(),
                    };
                    format!("return {ok_expr};")
                }
            };
            format!(
                "{setup}\ntry {{\n{body}\n}} finally {{ _f$boltffi_free_buf({result_expr}); }}"
            )
        }
        DartNativeType::CallbackHandle => {
            "throw UnsupportedError('callback-handle returns are not yet supported by this renderer');"
                .to_string()
        }
        // Non-`Void`-inner pointer returns (e.g. a pointer to a nested struct) aren't wired up
        // yet -- same placeholder convention as the two arms below, not a real implementation.
        DartNativeType::Pointer(_) => {
            "throw UnsupportedError('this return shape is not yet supported by this renderer');"
                .to_string()
        }
        DartNativeType::Composite(_) | DartNativeType::Function { .. } => {
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
pub(super) fn render_sync_body(
    abi_call: &AbiCall,
    native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
) -> String {
    let plan = plan_call(abi_call);
    let symbol_fn = format!("_f${}", abi_call.symbol);

    let mut out = String::new();
    for stmt in &plan.setup {
        out.push_str(stmt);
        out.push('\n');
    }

    let call_expr = format!("{symbol_fn}({})", plan.args.join(", "));

    let needs_result_var = !matches!(native_return, DartNativeType::Void);
    let result_expr = if needs_result_var {
        out.push_str(&format!("final _p$result = {call_expr};\n"));
        "_p$result"
    } else {
        out.push_str(&format!("{call_expr};\n"));
        ""
    };

    let decode = decode_return(
        result_expr,
        native_return,
        dart_return,
        &abi_call.error,
        &abi_call.returns,
    );

    if plan.has_status_scratch || !plan.buffer_vars.is_empty() {
        out.push_str("try {\n");
        out.push_str(&decode);
        out.push_str("\n} finally {\n");
        if plan.has_status_scratch {
            out.push_str(&format!("$$extffi.calloc.free({});\n", status_var()));
        }
        for var in &plan.buffer_vars {
            out.push_str(&format!("$$extffi.calloc.free({var}.ptr);\n"));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&decode);
        out.push('\n');
    }

    out
}

/// Renders the full Dart source body for an async native call, driving the
/// already-implemented `_$$BoltFFIAsync.create` poll/complete/cancel/free
/// protocol (`prelude.txt`) rather than a new one.
pub(super) fn render_async_body(
    abi_call: &AbiCall,
    async_call: &crate::ir::AsyncCall,
    complete_native_return: &DartNativeType,
    dart_return: &DartReturnInfo,
) -> String {
    let plan = plan_call(abi_call);
    let symbol_fn = format!("_f${}", abi_call.symbol);
    let poll_fn = format!("_f${}", async_call.poll);
    let complete_fn = format!("_f${}", async_call.complete);
    let cancel_fn = format!("_f${}", async_call.cancel);
    let free_fn = format!("_f${}", async_call.free);

    let mut out = String::new();
    for stmt in &plan.setup {
        out.push_str(stmt);
        out.push('\n');
    }

    let create_args = plan.args.join(", ");

    let needs_result_var = !matches!(complete_native_return, DartNativeType::Void);
    let complete_call = if plan.has_status_scratch {
        format!("{complete_fn}(handle, {})", status_var())
    } else {
        format!("{complete_fn}(handle)")
    };

    let decode = decode_return(
        if needs_result_var { "_p$asyncResult" } else { "" },
        complete_native_return,
        dart_return,
        &async_call.error,
        &async_call.result,
    );

    let complete_stmt = if needs_result_var {
        format!("final _p$asyncResult = {complete_call};\n{decode}")
    } else {
        format!("{complete_call};\n{decode}")
    };

    out.push_str(&format!(
        "return _$$BoltFFIAsync.create(\n  createFuture: () => {symbol_fn}({create_args}),\n  pollFuture: {poll_fn},\n  completeFuture: (handle) {{\n{complete_stmt}\n  }},\n  freeFuture: {free_fn},\n  cancelFuture: {cancel_fn},\n);\n"
    ));

    out
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
) -> String {
    match &abi_call.mode {
        CallMode::Sync => render_sync_body(abi_call, native_return, dart_return),
        CallMode::Async(_) if is_constructor => {
            "throw UnsupportedError('async constructors are not representable as a Dart factory constructor; this renderer does not yet reshape them into a static async factory method');\n".to_string()
        }
        CallMode::Async(async_call) => render_async_body(
            abi_call,
            async_call,
            &async_call_complete_native_type(async_call),
            dart_return,
        ),
    }
}

fn async_call_complete_native_type(async_call: &crate::ir::AsyncCall) -> DartNativeType {
    DartNativeType::from_return_shape_and_error_transport(&async_call.result, &async_call.error)
}

