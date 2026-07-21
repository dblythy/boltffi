use boltffi_ffi_rules::callable::ExecutionKind;

#[derive(Debug, Clone)]
pub struct DartNativeCallbackMethod {
    pub vtable_field_name: String,
    pub params: Vec<super::DartNativeFunctionParam>,
    pub return_type: super::DartNativeType,
    pub kind: ExecutionKind,
    /// The full trampoline body: looks up the registered implementation,
    /// decodes native arguments, invokes the Dart method, and reports the
    /// result back through the out-param (sync) or completion callback
    /// pointer (async) — see `lower::callback::render_native_method_body`.
    pub body: String,
}

impl DartNativeCallbackMethod {
    pub fn is_async(&self) -> bool {
        matches!(self.kind, ExecutionKind::Async)
    }

    /// Whether this vtable slot is safe to dispatch through
    /// `NativeCallable.listener` rather than `Pointer.fromFunction`.
    ///
    /// A `Pointer.fromFunction`-created native function **must** be invoked
    /// on the isolate's mutator thread — the Dart VM aborts the whole
    /// process otherwise. `NativeCallable.listener` is the documented
    /// escape hatch: it can be invoked from *any* thread because it posts
    /// the call to the isolate's event port and returns immediately,
    /// running the real Dart body later on the mutator thread. That
    /// deferral is only safe when the native caller never reads a value
    /// this call was supposed to have written before returning:
    ///
    /// - an async method's *dispatch* call is fire-and-forget on the Rust
    ///   side — `native.rs`'s `expand_async` declares the vtable slot
    ///   itself `void`-returning and never reads anything back from it
    ///   directly; the real result travels later through a *separate*,
    ///   Rust-owned completion function pointer that Dart calls, never the
    ///   other way around.
    /// - a `void`-returning sync method's status out-param
    ///   (`sync_void_impl_body`'s `callback_status`) is written but never
    ///   read back by the generated Rust caller — the wrapping trait
    ///   method itself returns `()`, so a stale/default status value is
    ///   never observed.
    ///
    /// A sync method with a real return value (`sync_returning_impl_body`)
    /// reads its out-param(s) immediately after the call returns and
    /// cannot use this path — Dart has no synchronous cross-thread
    /// callback primitive (unlike a JVM, which lets an arbitrary native
    /// thread call back into Java synchronously once attached via
    /// `AttachCurrentThread`). Those slots keep `Pointer.fromFunction` and
    /// rely on the host never invoking them off the mutator thread.
    pub fn dispatch_via_listener(&self) -> bool {
        should_dispatch_via_listener(self.kind, &self.return_type)
    }
}

/// The classification [`DartNativeCallbackMethod::dispatch_via_listener`]
/// wraps — factored out so `lower::callback`'s trampoline-body renderer
/// (which needs the same answer *before* a `DartNativeCallbackMethod`
/// exists) can never drift from it. Both sides of a deferred slot's ABI
/// depend on agreeing here: the Rust macro side
/// (`boltffi_macros::callbacks::trait_export::native`'s `is_deferred`)
/// hands owned, heap-allocated bytes for any encoded parameter on exactly
/// these slots, and drops the status out-param for a `void` sync method —
/// the Dart trampoline this file renders must free that buffer (never read
/// it as a borrow) and never write to a status parameter that no longer
/// exists.
pub(crate) fn should_dispatch_via_listener(
    kind: ExecutionKind,
    return_type: &super::DartNativeType,
) -> bool {
    matches!(kind, ExecutionKind::Async) || matches!(return_type, super::DartNativeType::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::dart::DartNativeType;

    fn method(kind: ExecutionKind, return_type: DartNativeType) -> DartNativeCallbackMethod {
        DartNativeCallbackMethod {
            vtable_field_name: "on_event".to_string(),
            params: vec![],
            return_type,
            kind,
            body: String::new(),
        }
    }

    // Thread-safety regression (Codex review finding, 2026-07-20): a
    // `Pointer.fromFunction` native callback aborts the whole process if
    // invoked off the isolate's mutator thread — and parse-core-rs proves
    // several callback traits (`WatchListener`, `NetworkStateListener`,
    // `EventuallyQueueListener`, `ObservabilitySink`) really are invoked
    // from a background tokio worker thread, not just the FFI caller's own
    // thread. `NativeCallable.listener` is safe cross-thread but can only
    // ever be used where Rust never reads a value the deferred call was
    // supposed to have written back — exactly the void-sync and async
    // cases.
    #[test]
    fn void_sync_method_dispatches_via_listener() {
        assert!(method(ExecutionKind::Sync, DartNativeType::Void).dispatch_via_listener());
    }

    #[test]
    fn async_method_dispatches_via_listener_regardless_of_return_type() {
        assert!(
            method(
                ExecutionKind::Async,
                DartNativeType::Primitive(crate::ir::PrimitiveType::U32)
            )
            .dispatch_via_listener()
        );
        assert!(method(ExecutionKind::Async, DartNativeType::Void).dispatch_via_listener());
    }

    #[test]
    fn sync_method_with_a_real_return_value_keeps_pointer_from_function() {
        assert!(
            !method(
                ExecutionKind::Sync,
                DartNativeType::Primitive(crate::ir::PrimitiveType::U32)
            )
            .dispatch_via_listener()
        );
    }
}

#[derive(Debug, Clone)]
pub struct DartNativeCallback {
    pub vtable_struct_name: String,
    pub methods: Vec<DartNativeCallbackMethod>,
}

#[derive(Debug, Clone)]
pub struct DartCallbackMethod {
    pub name: String,
    pub params: Vec<super::DartFunctionParam>,
    pub ret_ty: super::DartType,
    pub kind: ExecutionKind,
}

impl DartCallbackMethod {
    pub fn is_async(&self) -> bool {
        matches!(self.kind, ExecutionKind::Async)
    }
}

#[derive(Debug, Clone)]
pub struct DartCallback {
    pub class_name: String,
    pub impl_class_name: String,
    pub handle_map_class_name: String,
    pub handle_map_instance_name: String,
    pub methods: Vec<DartCallbackMethod>,
    pub native: DartNativeCallback,
}
