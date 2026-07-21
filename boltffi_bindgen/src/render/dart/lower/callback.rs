use boltffi_ffi_rules::callable::ExecutionKind;

use crate::{
    ir::{
        AbiCallbackInvocation, AbiCallbackMethod, AbiParam, CallbackId, CallbackKind,
        CallbackMethodDef, CallbackTraitDef, ErrorTransport, ParamRole, PrimitiveType,
        ReturnShape, ScalarOrigin, SpanContent, Transport, WriteSeq,
    },
    render::dart::{
        DartCallback, DartCallbackMethod, DartNativeCallback, DartNativeCallbackMethod,
        DartNativeFunctionKind, DartNativeFunctionParam, DartNativeType, DartType,
        NamingConvention, emit, should_dispatch_via_listener,
    },
};

/// `_k$<ClassName>HandleMap`, the module-level singleton each callback
/// trait's handle map is bound to (`callback.txt`'s
/// `{{ cb.handle_map_instance_name }}`). Shared with `lower::call`, which
/// needs the same name to box a Dart implementation into a
/// `_$$BoltFFICallbackHandle` when passing it *into* a native call.
pub(super) fn handle_map_instance_name(callback_id: &CallbackId) -> String {
    format!(
        "_k${}HandleMap",
        NamingConvention::class_name(callback_id.as_str())
    )
}

/// `0`/`false`/`0.0` for a primitive type — used on the async error path,
/// where the completion callback's fixed signature still has a slot for the
/// (unused) result value and Rust unconditionally unpacks it before checking
/// the status (see `native.rs`'s `async_returning_impl_body`).
fn primitive_zero_literal(primitive: PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Bool => "false",
        PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
        _ => "0",
    }
}

/// The raw tag read off a c-style enum value before it crosses the FFI
/// boundary as a scalar — mirrors `lower::call`'s outgoing `dart_name.value`
/// convention (`enum.txt` generates a plain `value` field), just read instead
/// of written.
fn scalar_value_expr(var: &str, returns: &ReturnShape) -> String {
    match &returns.transport {
        Some(Transport::Scalar(ScalarOrigin::CStyleEnum { .. })) => format!("{var}.value"),
        _ => var.to_string(),
    }
}

/// Builds the wire-encode step (`_$$WireWriter.transferOut(size)` +
/// `writer.write...`) as its own nested `{ }` block, then hands back the
/// finished writer through a `late final` declared in the *caller's* scope.
///
/// This exists solely to dodge a real Dart scoping trap: the codec's
/// `size`/`write` ops are baked (at IR-build time) against a fixed
/// identifier — `value` for a plain return, `result` for a `Result`-shaped
/// one (`ir::lower::abi::callback_return_shape_and_error`) — so this
/// renderer has no choice but to bind the call's result to a local of that
/// exact name. If a callback method's *own parameter* happens to share that
/// name (`fn on_value(&self, value: i32) -> i32` is a real one, caught by a
/// live-fixture smoke test: `dart analyze` reported "Local variable 'value'
/// can't be referenced before it is declared"), a flat
/// `final value = impl.onValue(value);` shadows the parameter for the
/// *entire enclosing block* — including its own initializer, which is
/// exactly backwards from what a reader expects and what a param named
/// `n`/`s`/anything-else never triggers. Confining the codec-mandated name to
/// its own nested block keeps the call site's reference to the real
/// parameter unambiguous; only code *inside* the block ever sees the
/// shadowing local.
fn wrap_wire_encode(codec_var: &str, hygienic_var: &str, encode_ops: &WriteSeq) -> String {
    let size_expr = emit::emit_size_expr(&encode_ops.size);
    let write_stmt = emit::emit_writer_write(encode_ops, "_p$w", codec_var);
    format!(
        "late final _$$WireWriter _p$w;\n{{\nfinal {codec_var} = {hygienic_var};\n_p$w = _$$WireWriter.transferOut({size_expr});\n{write_stmt}\n}}"
    )
}

/// The wire codec's own baked-in variable name for the value being encoded
/// (`ir::lower::abi::callback_return_shape_and_error` builds `encode_ops`
/// against `ValueExpr::Var("result")` for a `Result`-shaped return, and
/// `ValueExpr::Var("value")` otherwise) — the local this renderer binds the
/// call's result to must match exactly, or `emit_size_expr`'s *nested* reads
/// of that name (the top-level `emit_writer_write` call takes its own
/// explicit override, but nested ops don't) silently reference an undefined
/// identifier.
fn result_value_var_name(error: &ErrorTransport) -> &'static str {
    if matches!(error, ErrorTransport::Encoded { .. }) {
        "result"
    } else {
        "value"
    }
}

/// The `.asFunction<...>()` signature for `_p$callback`, the completion
/// pointer an async method's vtable entry is handed. Always leads with the
/// `callback_data` handle (`int`); the payload shape then follows
/// `return_type` exactly as built in [`push_async_callback_params`] — kept as
/// one function so the declared vtable signature and the call site can never
/// drift apart.
fn async_completion_dart_signature(return_type: &DartNativeType) -> String {
    let mut params = vec!["int".to_string()];
    match return_type {
        DartNativeType::Void => {}
        DartNativeType::Primitive(p) => params.push(DartNativeType::Primitive(*p).dart_sub_type()),
        _ => {
            params.push(
                DartNativeType::Pointer(Box::new(DartNativeType::Primitive(PrimitiveType::U8)))
                    .dart_sub_type(),
            );
            params.push(DartNativeType::Primitive(PrimitiveType::USize).dart_sub_type());
        }
    }
    params.push(DartNativeType::Primitive(PrimitiveType::I32).dart_sub_type());
    format!("void Function({})", params.join(", "))
}

/// The async vtable field's trailing `(callback, callback_data)` pair native
/// param shape, appended to `params` in [`lower_native_callback_method`].
/// Mirrors the Rust macro side (`native.rs`'s `expand_async`): the completion
/// payload is `(result_ptr, result_len)` for anything but a genuinely
/// `Passable` scalar — `uses_wire_payload()` there is exactly
/// `!matches!(strategy, Scalar(_))`, checked here against `return_type`
/// instead of re-deriving the same strategy a second time.
fn push_async_callback_params(callback_params: &mut Vec<DartNativeType>, return_type: &DartNativeType) {
    match return_type {
        DartNativeType::Void => {}
        DartNativeType::Primitive(p) => callback_params.push(DartNativeType::Primitive(*p)),
        _ => callback_params.extend([
            DartNativeType::Pointer(Box::new(DartNativeType::Primitive(PrimitiveType::U8))),
            DartNativeType::Primitive(PrimitiveType::USize),
        ]),
    }
    callback_params.push(DartNativeType::Primitive(PrimitiveType::I32));
}

/// Whether `param` is the pointer half of a wire-encoded callback
/// parameter — the one `decode_input_args` reconstructs a `Pointer<Uint8>`
/// for on a deferred slot (see `lower_native_callback_method`'s param-type
/// override and `decode_input_args`'s `Transport::Span(SpanContent::Encoded(_))`
/// arm, which this must match exactly).
fn encoded_ptr_param(param: &AbiParam) -> bool {
    matches!(
        &param.role,
        ParamRole::Input {
            transport: Transport::Span(SpanContent::Encoded(_)),
            ..
        }
    )
}

/// One decoded input argument's setup (if any: a `_$$WireReader` over a
/// ptr+len pair) plus the expression passed positionally to `impl.<method>`.
///
/// `frees` is kept separate from `setup` rather than interleaved: every
/// free must run exactly once regardless of whether the handle lookup, a
/// later param's decode, or the registered implementation itself throws
/// (Codex review finding 4) — `render_native_method_body` places `setup`
/// inside a `try` and `frees` inside its `finally`, so an inline free
/// sitting between two `setup` statements would never be reached by an
/// exception raised earlier in the same list.
struct DecodedArgs {
    setup: Vec<String>,
    frees: Vec<String>,
    call_args: Vec<String>,
}

/// Decodes a callback method's *logical* input params (`m.params` minus the
/// leading handle and, for sync methods, the trailing out/status params —
/// `ParamRole::Input` is the only role that survives this filter) into Dart
/// expressions ready to pass to the registered implementation.
///
/// `ir::lower::abi::lower_callback_param` only ever produces `Scalar` (direct
/// passthrough — dart:ffi already marshals the native type) or `Encoded`
/// (wire ptr+len, decoded through the same `_$$WireReader`/`emit_reader_read`
/// pair `lower::call::decode_return` uses for owned-buffer returns) —
/// `Direct` (composite-by-value) is declared in the IR but never constructed;
/// a param transport this renderer doesn't recognize fails loudly rather than
/// silently mis-marshaling.
///
/// `is_deferred` must be `should_dispatch_via_listener`'s answer for this
/// same method: on a deferred slot, `native.rs`'s matching macro hands this
/// trampoline a transferred, Rust-allocator-owned buffer (via
/// `transfer_deferred_callback_bytes` — a stack-scoped one would already be
/// gone by the time this — a `NativeCallable.listener`-posted, not
/// synchronous — call runs), so the decode is forced eager (into a plain
/// local, not left as a lazy inline read expression) so a paired
/// `boltffi_free_deferred_callback_bytes` call can follow it before the
/// value is used. A non-deferred slot's buffer is still Rust-stack-scoped
/// for the (synchronous) duration of this call — freeing it here would be a
/// use-after-free on Rust's side instead.
fn decode_input_args(params: &[AbiParam], is_deferred: bool) -> DecodedArgs {
    let mut setup = Vec::new();
    let mut frees = Vec::new();
    let mut call_args = Vec::new();

    for param in params {
        let ParamRole::Input { transport, .. } = &param.role else {
            continue;
        };

        match transport {
            Transport::Scalar(origin) => {
                let raw = NamingConvention::param_name(param.name.as_str());
                let expr = match origin {
                    ScalarOrigin::CStyleEnum { enum_id, .. } => format!(
                        "{}._m$fromValue({raw})",
                        NamingConvention::class_name(enum_id.as_str())
                    ),
                    ScalarOrigin::Primitive(_) => raw,
                };
                call_args.push(expr);
            }
            Transport::Span(SpanContent::Encoded(_)) => {
                let ParamRole::Input {
                    decode_ops: Some(decode_ops),
                    ..
                } = &param.role
                else {
                    unreachable!("an Encoded callback param always carries decode_ops");
                };
                let base = NamingConvention::param_name(param.name.as_str());
                // Must match `native_function.rs::lower_native_function_param`'s
                // naming for this same `AbiParam` exactly — that's what
                // declares these identifiers in the enclosing native
                // signature this body is spliced into.
                let raw_ptr_name = super::native_function::encoded_ptr_name(param.name.as_str());
                let len_name = super::native_function::encoded_len_name(param.name.as_str());
                let reader_var = format!("_p$r${base}");

                // On a deferred slot, `raw_ptr_name` is declared as a plain
                // `int` address, not a `Pointer<Uint8>` (see
                // `lower_native_callback_method` — a raw `Pointer` isn't
                // guaranteed to survive the `NativeCallable.listener`
                // isolate hop; a pointer-sized integer is). Reconstruct the
                // real pointer here, now safely on the mutator isolate,
                // before using it.
                let ptr_name = if is_deferred {
                    let reconstructed = format!("_p$ptr${base}");
                    setup.push(format!(
                        "final {reconstructed} = $$ffi.Pointer<$$ffi.Uint8>.fromAddress({raw_ptr_name});"
                    ));
                    reconstructed
                } else {
                    raw_ptr_name.clone()
                };

                setup.push(format!(
                    "final {reader_var} = _$$WireReader({ptr_name}, {len_name});"
                ));
                let read_expr = emit::emit_reader_read(decode_ops, &reader_var);
                if is_deferred {
                    let decoded_var = format!("_p$d${base}");
                    setup.push(format!("final {decoded_var} = {read_expr};"));
                    // Rust transferred ownership of this buffer via its own
                    // global allocator (`transfer_deferred_callback_bytes`),
                    // not `malloc`/`calloc` — free it through the paired
                    // native export rather than `calloc.free`, which would
                    // reach for a foreign allocator (`CoTaskMemAlloc` on
                    // Windows) that never allocated it. Collected into
                    // `frees` (run from a `finally`), not appended to
                    // `setup` inline — a later param's decode, or the
                    // registered implementation itself, throwing must not
                    // skip this free (Codex review finding 4). Rebuilds the
                    // pointer from `raw_ptr_name` (the native `int` param,
                    // in scope for the whole function) rather than reusing
                    // `ptr_name` — that local is declared inside the `try`
                    // this `finally` sits outside of, and Dart doesn't
                    // share a `try`'s scope with its own `finally`.
                    frees.push(format!(
                        "_f$boltffi_free_deferred_callback_bytes($$ffi.Pointer<$$ffi.Uint8>.fromAddress({raw_ptr_name}), {len_name});"
                    ));
                    call_args.push(decoded_var);
                } else {
                    call_args.push(read_expr);
                }
            }
            other => call_args.push(format!(
                "(throw UnsupportedError('unsupported callback param transport: {other:?}'))"
            )),
        }
    }

    DecodedArgs {
        setup,
        frees,
        call_args,
    }
}

/// The try-body / catch-body pair a native trampoline's outer
/// `try { ... } catch (e) { ... }` fills in — split so the error path can
/// still report failure with the completion mechanism's own correct arity
/// (an out-status write for sync, a same-shaped completion-pointer call for
/// async) regardless of which branch of the return shape below threw.
struct CompletionBody {
    try_body: String,
    catch_body: String,
}

fn render_sync_completion(
    call_expr: &str,
    m: &AbiCallbackMethod,
    return_type: &DartNativeType,
) -> CompletionBody {
    // A `void` return has no `_p$outStatus` param at all (this slot
    // dispatches through a deferred `NativeCallable.listener`, and Rust
    // never reads a status back for it — see `lower_native_callback_method`
    // and `native.rs`'s matching `expand_sync`) — an exception here has
    // nowhere left to report to; the `try`/`catch` still exists so it
    // doesn't escape uncaught into the isolate's top-level error zone.
    if matches!(return_type, DartNativeType::Void) {
        return CompletionBody {
            try_body: format!("{call_expr};"),
            catch_body: String::new(),
        };
    }

    let catch_body = "_p$outStatus.ref.code = -1;".to_string();

    let try_body = match return_type {
        DartNativeType::Void => unreachable!("handled above"),
        DartNativeType::Primitive(_) => {
            // `_p$value` (not the codec's `value`/`result` names — no codec
            // is involved for a bare scalar) can never collide with a
            // decoded parameter: Rust identifiers can't contain `$`.
            let value_expr = scalar_value_expr("_p$value", &m.returns);
            format!(
                "final _p$value = {call_expr};\n_p$outPtr.value = {value_expr};\n_p$outStatus.ref.code = 0;"
            )
        }
        _ => match &m.returns.encode_ops {
            Some(encode_ops) => {
                let codec_var = result_value_var_name(&m.error);
                let encode_block = wrap_wire_encode(codec_var, "_p$value", encode_ops);
                format!(
                    "final _p$value = {call_expr};\n{encode_block}\n_p$outPtr.value = _p$w.ptr;\n_p$outLen.value = _p$w.len;\n_p$outStatus.ref.code = 0;"
                )
            }
            None => "throw UnsupportedError('this callback return shape is not yet supported by this renderer');"
                .to_string(),
        },
    };

    CompletionBody {
        try_body,
        catch_body,
    }
}

fn render_async_completion(
    call_expr: &str,
    m: &AbiCallbackMethod,
    return_type: &DartNativeType,
) -> CompletionBody {
    let dart_sig = async_completion_dart_signature(return_type);
    let invoke = format!("_p$callback.asFunction<{dart_sig}>()");

    match return_type {
        DartNativeType::Void => CompletionBody {
            try_body: format!("await {call_expr};\n{invoke}(_p$callbackData, 0);"),
            catch_body: format!("{invoke}(_p$callbackData, -1);"),
        },
        DartNativeType::Primitive(p) => {
            // Same reasoning as the sync scalar arm: `_p$value` sidesteps
            // any collision with a decoded parameter of the same name.
            let value_expr = scalar_value_expr("_p$value", &m.returns);
            CompletionBody {
                try_body: format!(
                    "final _p$value = await {call_expr};\n{invoke}(_p$callbackData, {value_expr}, 0);"
                ),
                catch_body: format!(
                    "{invoke}(_p$callbackData, {}, -1);",
                    primitive_zero_literal(*p)
                ),
            }
        }
        _ => {
            let codec_var = result_value_var_name(&m.error);
            let catch_body = format!("{invoke}(_p$callbackData, $$ffi.nullptr, 0, -1);");
            let try_body = match &m.returns.encode_ops {
                Some(encode_ops) => {
                    let encode_block = wrap_wire_encode(codec_var, "_p$value", encode_ops);
                    format!(
                        "final _p$value = await {call_expr};\n{encode_block}\n{invoke}(_p$callbackData, _p$w.ptr, _p$w.len, 0);"
                    )
                }
                None => "throw UnsupportedError('this callback return shape is not yet supported by this renderer');"
                    .to_string(),
            };
            CompletionBody {
                try_body,
                catch_body,
            }
        }
    }
}

/// The full trampoline body: look up the registered implementation, decode
/// native arguments, invoke it, and report the result back through the
/// out-param (sync) or completion callback pointer (async). One `try/catch`
/// covers the handle lookup too, so an invalid handle reports failure
/// through the same channel as any other exception instead of escaping
/// uncaught across the `Pointer.fromFunction` boundary (which — for a
/// `void`-returning native callback — the Dart VM swallows silently, leaving
/// Rust's zero-initialized `FfiStatus::default()` looking like success).
fn render_native_method_body(
    class_name: &str,
    handle_map_instance_name: &str,
    method_dart_name: &str,
    m: &AbiCallbackMethod,
    return_type: &DartNativeType,
) -> String {
    let is_deferred = should_dispatch_via_listener(m.execution_kind, return_type);
    let decoded = decode_input_args(&m.params[1..], is_deferred);
    let call_expr = format!("impl.{method_dart_name}({})", decoded.call_args.join(", "));

    let completion = match m.execution_kind {
        ExecutionKind::Sync => render_sync_completion(&call_expr, m, return_type),
        ExecutionKind::Async => render_async_completion(&call_expr, m, return_type),
    };

    let mut setup = String::new();
    for stmt in &decoded.setup {
        setup.push_str(stmt);
        setup.push('\n');
    }

    let lookup_and_call = format!(
        "final impl = {handle_map_instance_name}.get(_p$handle);\n\
         if (impl == null) {{\n\
         throw _$$FFIException(-1, \"{class_name}: invalid handle `${{_p$handle}}`\");\n\
         }}\n\
         {setup}{try_body}",
        try_body = completion.try_body,
    );

    // Every param's owned buffer must be freed exactly once regardless of
    // where an exception originates — an invalid handle, a later param's
    // decode, or the registered implementation itself throwing (Codex
    // review finding 4: a free sitting inline between other statements is
    // skipped by any exception raised earlier in the same list). Nesting
    // this in its own `try`/`finally`, inside the outer reporting `catch`,
    // guarantees the frees run before the exception is caught and reported
    // — without a decoded param at all (`frees` empty), the extra nesting
    // buys nothing, so it's skipped.
    let guarded_body = if decoded.frees.is_empty() {
        lookup_and_call
    } else {
        let frees = decoded.frees.join("\n");
        format!("try {{\n{lookup_and_call}\n}} finally {{\n{frees}\n}}")
    };

    format!(
        "try {{\n\
         {guarded_body}\n\
         }} catch (e) {{\n\
         {catch_body}\n\
         }}\n",
        catch_body = completion.catch_body,
    )
}

impl<'a> super::DartLowerer<'a> {
    fn abi_callback_for(&self, id: &CallbackId) -> Option<&AbiCallbackInvocation> {
        self.abi.callbacks.iter().find(|cb| cb.callback_id == *id)
    }

    fn lower_native_callback_method(
        &self,
        class_name: &str,
        handle_map_instance_name: &str,
        m: &AbiCallbackMethod,
    ) -> DartNativeCallbackMethod {
        assert!(matches!(
            m.params[0].role,
            ParamRole::Input {
                transport: Transport::Callback { .. },
                ..
            }
        ));

        let mut params = vec![DartNativeFunctionParam {
            name: "_p$handle".to_string(),
            native_type: DartNativeType::Primitive(PrimitiveType::U64),
        }];

        let return_type =
            DartNativeType::from_return_shape_and_error_transport(&m.returns, &m.error);
        let is_deferred = should_dispatch_via_listener(m.execution_kind, &return_type);

        params.extend(m.params[1..].iter().map(|p| {
            let mut param = self.lower_native_function_param(p);
            // A deferred slot's dispatch call crosses through
            // `NativeCallable.listener`, which posts its arguments to the
            // isolate's `SendPort` — and a raw `Pointer` is not among the
            // types Dart guarantees survive that trip intact (dart-lang/sdk
            // #50457: isolate messaging has no supported way to pass a
            // `Pointer` between isolates). An encoded param's ptr half is
            // exactly such a `Pointer<Uint8>`; declare it as a plain
            // pointer-sized integer instead (`IntPtr`/`UintPtr` and a raw
            // `Pointer<T>` are ABI-identical on the native side — this
            // changes nothing about what Rust passes) and reconstruct the
            // real `Pointer` from that integer inside the trampoline body
            // (`decode_input_args`) once it's safely on the other side of
            // the isolate hop.
            if is_deferred && encoded_ptr_param(p) {
                param.native_type = DartNativeType::Primitive(PrimitiveType::USize);
            }
            param
        }));

        match m.execution_kind {
            // A `void` return means this slot dispatches through a deferred
            // `NativeCallable.listener` (`dispatch_via_listener`) — the
            // matching Rust macro (`native.rs`'s `expand_sync`) drops the
            // status out-param from this exact vtable slot entirely
            // (nothing there ever reads it back), so this trampoline must
            // not declare one either, or its native signature no longer
            // matches the vtable field it's assigned to.
            ExecutionKind::Sync if !matches!(return_type, DartNativeType::Void) => {
                params.push(DartNativeFunctionParam {
                    name: "_p$outStatus".to_string(),
                    native_type: DartNativeType::Pointer(Box::new(DartNativeType::Status)),
                });
            }
            ExecutionKind::Sync => {}
            ExecutionKind::Async => {
                let mut callback_params = vec![];
                push_async_callback_params(&mut callback_params, &return_type);

                params.extend([
                    DartNativeFunctionParam {
                        name: "_p$callback".to_string(),
                        native_type: DartNativeType::Function {
                            kind: DartNativeFunctionKind::Callback,
                            params: callback_params,
                            return_ty: Box::new(DartNativeType::Void),
                        },
                    },
                    DartNativeFunctionParam {
                        name: "_p$callbackData".to_string(),
                        native_type: DartNativeType::Primitive(PrimitiveType::U64),
                    },
                ]);
            }
        };

        let method_dart_name = NamingConvention::function_name(m.id.as_str());
        let body = render_native_method_body(
            class_name,
            handle_map_instance_name,
            &method_dart_name,
            m,
            &return_type,
        );

        DartNativeCallbackMethod {
            vtable_field_name: NamingConvention::property_name(m.vtable_field.as_str()),
            params,
            return_type,
            kind: m.execution_kind,
            body,
        }
    }

    fn lower_callback_method(&self, cb: &CallbackMethodDef) -> DartCallbackMethod {
        let params = cb.params.iter().map(|p| self.lower_param(p)).collect();

        DartCallbackMethod {
            name: NamingConvention::function_name(cb.id.as_str()),
            params,
            ret_ty: DartType::from_return_def(&cb.returns, &self.ffi.catalog),
            kind: cb.execution_kind,
        }
    }

    fn lower_one_callback(&self, cb_def: &CallbackTraitDef) -> DartCallback {
        let abi_cb = self.abi_callback_for(&cb_def.id).unwrap();

        let class_name = NamingConvention::class_name(cb_def.id.as_str());
        let impl_class_name = format!("_I${}", class_name);
        let vtable_struct_name = format!(
            "_I${}",
            NamingConvention::class_name(abi_cb.vtable_type.as_str())
        );
        let handle_map_class_name = format!("{}HandleMap", impl_class_name);
        let handle_map_instance_name = handle_map_instance_name(&cb_def.id);

        let methods = cb_def
            .methods
            .iter()
            .map(|m| self.lower_callback_method(m))
            .collect();

        let native_methods = abi_cb
            .methods
            .iter()
            .map(|m| self.lower_native_callback_method(&class_name, &handle_map_instance_name, m))
            .collect();

        DartCallback {
            class_name,
            impl_class_name,
            handle_map_class_name,
            handle_map_instance_name,
            methods,
            native: DartNativeCallback {
                vtable_struct_name,
                methods: native_methods,
                teardown_symbol: teardown_symbol_for(abi_cb.register_fn.as_str()),
            },
        }
    }

    pub(super) fn lower_callbacks(&self) -> Vec<DartCallback> {
        self.ffi
            .catalog
            .all_callbacks()
            .filter(|cb| matches!(cb.kind, CallbackKind::Trait))
            .map(|cb| self.lower_one_callback(cb))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use boltffi_ffi_rules::callable::ExecutionKind;

    use crate::{
        ir::{
            CallbackId, CallbackKind, CallbackMethodDef, CallbackTraitDef, ParamDef, ParamName,
            ParamPassing, PrimitiveType, ReturnDef, TypeExpr,
        },
        render::dart::{DartNativeType, test},
    };

    fn callback_with_method(method: CallbackMethodDef) -> CallbackTraitDef {
        CallbackTraitDef {
            qualified_path: String::new(),
            id: CallbackId::new("Listener"),
            methods: vec![method],
            kind: CallbackKind::Trait,
            doc: None,
        }
    }

    fn lower_first_native_method(
        method: CallbackMethodDef,
    ) -> crate::render::dart::DartNativeCallbackMethod {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_callback(callback_with_method(method));
        let library = test::lower(&ffi);
        library.callbacks[0].native.methods[0].clone()
    }

    #[test]
    fn sync_void_return_calls_impl_with_no_status_channel() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("on_event"),
            params: vec![ParamDef {
                name: ParamName::new("text"),
                type_expr: TypeExpr::String,
                passing: ParamPassing::Value,
                doc: None,
            }],
            returns: ReturnDef::Void,
            doc: None,
        });

        // No `_p$outStatus` param at all: this slot dispatches through a
        // deferred `NativeCallable.listener`, and the matching Rust macro
        // (`native.rs`'s `expand_sync`) drops the status out-param from
        // this exact vtable slot — nothing there ever reads it back, and
        // writing to one here would need a param this signature no longer
        // declares.
        assert!(
            !native
                .params
                .iter()
                .any(|p| p.name == "_p$outStatus"),
            "params: {:?}",
            native.params
        );
        assert!(
            !native.body.contains("_p$outStatus"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("impl.onEvent("),
            "body: {}",
            native.body
        );
        // The native param is a plain `int` address, not a `Pointer` — a
        // raw `Pointer` isn't guaranteed to survive the
        // `NativeCallable.listener` isolate hop (dart-lang/sdk #50457) —
        // reconstructed into a real pointer before use.
        assert!(
            native
                .params
                .iter()
                .any(|p| p.name == "_p$textPtr"
                    && matches!(p.native_type, DartNativeType::Primitive(PrimitiveType::USize))),
            "params: {:?}",
            native.params
        );
        assert!(
            native
                .body
                .contains("final _p$ptr$text = $$ffi.Pointer<$$ffi.Uint8>.fromAddress(_p$textPtr);"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("_$$WireReader(_p$ptr$text, _p$textLen)"),
            "body: {}",
            native.body
        );
        // Regression (Codex review finding, 2026-07-20, finding 1): the
        // encoded param must be decoded eagerly, into a local, *before* the
        // call — never left as a lazy inline read expression evaluated at
        // the call site. Rust transfers ownership of this buffer to this
        // trampoline for exactly this slot shape (see `native.rs`'s
        // `is_deferred`/`transfer_deferred_callback_bytes`), so it must be
        // freed here (via the paired native export, not `calloc.free` —
        // Rust's own allocator owns it, not Dart's), never borrowed
        // indefinitely.
        assert!(
            native.body.contains("final _p$d$text = _p$r$text.readString();"),
            "body: {}",
            native.body
        );
        // The `finally` reconstructs the pointer fresh from the native
        // `int` param (`_p$textPtr`), rather than reusing the `try`-scoped
        // `_p$ptr$text` local — a `finally` block does not share its
        // paired `try`'s local scope in Dart.
        assert!(
            native.body.contains(
                "_f$boltffi_free_deferred_callback_bytes($$ffi.Pointer<$$ffi.Uint8>.fromAddress(_p$textPtr), _p$textLen);"
            ),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("impl.onEvent(_p$d$text);"),
            "body: {}",
            native.body
        );
        // Regression (Codex review finding 4, 2026-07-20): the free must
        // run regardless of whether the handle lookup, the decode, or
        // `impl.onEvent` itself throws — placed in a `finally` wrapping all
        // three, not inline between decode and call (where an exception
        // from `impl.onEvent` would skip it).
        let decode_pos = native
            .body
            .find("final _p$d$text = _p$r$text.readString();")
            .expect(&native.body);
        let call_pos = native.body.find("impl.onEvent(_p$d$text);").expect(&native.body);
        let finally_pos = native.body.find("} finally {").expect(&native.body);
        let free_pos = native
            .body
            .find("_f$boltffi_free_deferred_callback_bytes($$ffi.Pointer<$$ffi.Uint8>.fromAddress(_p$textPtr), _p$textLen);")
            .expect(&native.body);
        assert!(
            decode_pos < call_pos && call_pos < finally_pos && finally_pos < free_pos,
            "body: {}",
            native.body
        );
        // The handle-lookup null check still reports through *a* channel
        // (an exception caught locally) rather than escaping uncaught
        // across the deferred trampoline boundary — but there is nowhere
        // left to report it to, so the catch body is empty.
        assert!(
            native.body.contains("throw _$$FFIException(-1,"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("} catch (e) {\n\n}"),
            "body: {}",
            native.body
        );
    }

    #[test]
    fn sync_scalar_param_is_passed_through_directly() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("on_count"),
            params: vec![ParamDef {
                name: ParamName::new("n"),
                type_expr: TypeExpr::Primitive(PrimitiveType::U32),
                passing: ParamPassing::Value,
                doc: None,
            }],
            returns: ReturnDef::Void,
            doc: None,
        });

        assert!(
            native.body.contains("impl.onCount(n)"),
            "body: {}",
            native.body
        );
    }

    #[test]
    fn sync_scalar_return_writes_out_pointer() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("map_u32"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::U32)),
            doc: None,
        });

        assert!(
            native.body.contains("final _p$value = impl.mapU32();"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("_p$outPtr.value = _p$value;"),
            "body: {}",
            native.body
        );
    }

    // Regression (Codex review finding, 2026-07-20): a callback method
    // returning a handle (an exported class) has no `encode_ops` —
    // `ir::lower::abi::return_shape_from_transport`'s `Transport::Handle`
    // arm leaves it `None`, and the wire-codec layer itself has no
    // representation for this shape at all (`codec_from_transport` panics
    // on `Transport::Handle`/`Transport::Callback` rather than silently
    // miscoding them — there's no codec to "just implement" here without a
    // parallel non-codec, handle-registration path).
    //
    // A generation-time panic was tried and reverted: unlike this file's
    // other "unsupported shape" arms (which degrade *per method*, so the
    // rest of the crate still generates), a panic here aborts the whole
    // render pass — confirmed against the fork's own `examples/demo` fixture
    // (`make_incrementing_callback`, a function that *returns* a callback),
    // which stopped generating at all. Reverted to the established
    // per-method runtime-throw convention this file already uses ~7 other
    // places for a genuinely out-of-scope shape.
    #[test]
    fn sync_handle_return_still_throws_at_runtime_not_generation_time() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("make_widget"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::Handle(crate::ir::ClassId::new("Widget"))),
            doc: None,
        });

        assert!(
            native
                .body
                .contains("throw UnsupportedError('this callback return shape is not yet supported by this renderer');"),
            "body: {}",
            native.body
        );
    }

    #[test]
    fn async_callback_handle_return_still_throws_at_runtime_not_generation_time() {
        let mut ffi = test::empty_contract();
        // The returned callback type must itself resolve in the catalog —
        // unrelated to the shape under test here, `DartType::from_type_expr`
        // needs it for the callback's own *public* method signature.
        ffi.catalog.insert_callback(CallbackTraitDef {
            qualified_path: String::new(),
            id: CallbackId::new("Other"),
            methods: vec![CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: crate::ir::MethodId::new("call"),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: CallbackKind::Trait,
            doc: None,
        });
        ffi.catalog.insert_callback(callback_with_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Async,
            id: crate::ir::MethodId::new("make_listener"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::Callback(CallbackId::new("Other"))),
            doc: None,
        }));
        let library = test::lower(&ffi);
        let listener = library
            .callbacks
            .iter()
            .find(|cb| cb.class_name == "Listener")
            .expect("Listener callback lowered");

        assert!(
            listener.native.methods[0]
                .body
                .contains("throw UnsupportedError('this callback return shape is not yet supported by this renderer');"),
            "body: {}",
            listener.native.methods[0].body
        );
    }

    #[test]
    fn sync_wire_return_builds_transfer_out_writer() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("map_string"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::String),
            doc: None,
        });

        assert!(
            native.body.contains("_$$WireWriter.transferOut("),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("_p$outPtr.value = _p$w.ptr;"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("_p$outLen.value = _p$w.len;"),
            "body: {}",
            native.body
        );
    }

    // Regression: caught by a live-fixture smoke test (`examples/demo`'s
    // `ValueCallback.onValue(&self, value: i32) -> i32` — a real trait with a
    // param named the same as the codec's baked result-variable name).
    // `dart analyze` reported "Local variable 'value' can't be referenced
    // before it is declared" because a flat
    // `final value = impl.onValue(value);` shadows the *parameter* for the
    // whole enclosing block, including its own initializer.
    #[test]
    fn sync_scalar_return_does_not_collide_with_a_same_named_param() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("on_value"),
            params: vec![ParamDef {
                name: ParamName::new("value"),
                type_expr: TypeExpr::Primitive(PrimitiveType::I32),
                passing: ParamPassing::Value,
                doc: None,
            }],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::I32)),
            doc: None,
        });

        assert!(
            native.body.contains("impl.onValue(value)"),
            "the call site must still read the real parameter: {}",
            native.body
        );
        assert!(
            !native.body.contains("final value ="),
            "the result local must not be named the same as the parameter: {}",
            native.body
        );
    }

    #[test]
    fn sync_wire_return_does_not_collide_with_a_same_named_param() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Sync,
            id: crate::ir::MethodId::new("format_value"),
            params: vec![ParamDef {
                name: ParamName::new("value"),
                type_expr: TypeExpr::String,
                passing: ParamPassing::Value,
                doc: None,
            }],
            returns: ReturnDef::Value(TypeExpr::String),
            doc: None,
        });

        // The call site (outer scope) reads the real `value` parameter —
        // decoded straight from its `_$$WireReader`, never rebound to a bare
        // `value` local anywhere the call expression itself could see it.
        assert!(
            native.body.contains("impl.formatValue(_p$r$value.readString())"),
            "body: {}",
            native.body
        );
        // The codec-mandated `value` name is confined to its own nested
        // block (aliasing the hygienic `_p$value`) — never declared in the
        // outer scope the call site's `value` parameter reference lives in,
        // which is what the sync/async scalar-return regression tests above
        // pin directly (a *flat*, same-scope `final value = ...`).
        assert!(
            native.body.contains("{\nfinal value = _p$value;\n"),
            "the codec-mandated `value` local must live inside its own nested block: {}",
            native.body
        );
    }

    // Regression: an async callback method with a plain scalar return used
    // to always declare the completion pointer with the wire (ptr, len,
    // status) shape, even for a `Passable` scalar return — a real ABI
    // mismatch against the Rust macro side (`native.rs`'s `expand_async`
    // only widens to the wire shape when `uses_wire_payload()`, i.e. the
    // return isn't `ValueReturnStrategy::Scalar`).
    #[test]
    fn async_scalar_return_uses_direct_value_not_wire_payload() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Async,
            id: crate::ir::MethodId::new("map_u32"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::U32)),
            doc: None,
        });

        let callback_param = native
            .params
            .iter()
            .find(|p| p.name == "_p$callback")
            .expect("callback param");
        let signature = callback_param.native_type.native_type();
        assert!(
            signature.contains("$$ffi.Uint32, $$ffi.Int32"),
            "expected a direct scalar + status completion signature, got: {signature}"
        );
        assert!(
            !signature.contains("$$ffi.Pointer<$$ffi.Uint8>"),
            "a scalar return must not widen to a wire payload: {signature}"
        );

        assert!(
            native
                .body
                .contains("asFunction<void Function(int, int, int)>()"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("(_p$callbackData, _p$value, 0);"),
            "body: {}",
            native.body
        );
    }

    #[test]
    fn async_void_return_awaits_and_invokes_completion_with_status_only() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Async,
            id: crate::ir::MethodId::new("on_event"),
            params: vec![],
            returns: ReturnDef::Void,
            doc: None,
        });

        assert!(
            native.body.contains("await impl.onEvent();"),
            "body: {}",
            native.body
        );
        assert!(
            native
                .body
                .contains("asFunction<void Function(int, int)>()(_p$callbackData, 0);"),
            "body: {}",
            native.body
        );
        assert!(
            native
                .body
                .contains("asFunction<void Function(int, int)>()(_p$callbackData, -1);"),
            "body: {}",
            native.body
        );
    }

    #[test]
    fn async_wire_return_encodes_and_invokes_completion_with_ptr_len() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Async,
            id: crate::ir::MethodId::new("map_string"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::String),
            doc: None,
        });

        assert!(
            native.body.contains("final _p$value = await impl.mapString();"),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains("_$$WireWriter.transferOut("),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains(
                "asFunction<void Function(int, $$ffi.Pointer<$$ffi.Uint8>, int, int)>()(_p$callbackData, _p$w.ptr, _p$w.len, 0);"
            ),
            "body: {}",
            native.body
        );
        assert!(
            native.body.contains(
                "asFunction<void Function(int, $$ffi.Pointer<$$ffi.Uint8>, int, int)>()(_p$callbackData, $$ffi.nullptr, 0, -1);"
            ),
            "body: {}",
            native.body
        );
    }

    // Same collision as the sync case, exercised on the async completion
    // path (`_p$callback.asFunction<...>()(_p$callbackData, value, 0)` would
    // have shadowed a same-named parameter identically).
    #[test]
    fn async_scalar_return_does_not_collide_with_a_same_named_param() {
        let native = lower_first_native_method(CallbackMethodDef {
            execution_kind: ExecutionKind::Async,
            id: crate::ir::MethodId::new("on_value"),
            params: vec![ParamDef {
                name: ParamName::new("value"),
                type_expr: TypeExpr::Primitive(PrimitiveType::I32),
                passing: ParamPassing::Value,
                doc: None,
            }],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::I32)),
            doc: None,
        });

        assert!(
            native.body.contains("await impl.onValue(value)"),
            "body: {}",
            native.body
        );
        assert!(
            !native.body.contains("final value ="),
            "body: {}",
            native.body
        );
    }

    #[test]
    fn handle_map_instance_name_matches_callback_class_naming() {
        assert_eq!(
            super::handle_map_instance_name(&CallbackId::new("live_query_listener")),
            "_k$LiveQueryListenerHandleMap"
        );
    }
}

/// The teardown export's name, derived from the register export's — the one place (mirrored by
/// the macro side) that encodes the convention for BOTH register-naming schemes in the codebase.
fn teardown_symbol_for(register_symbol: &str) -> String {
    if register_symbol.contains("_register_callback_") {
        register_symbol.replace("_register_callback_", "_teardown_callback_")
    } else {
        register_symbol.replacen("_register_", "_teardown_", 1)
    }
}
