use crate::ir::Receiver;

#[derive(Debug, Clone)]
pub struct DartNativeFunctionParam {
    pub name: String,
    pub native_type: super::DartNativeType,
}

#[derive(Debug, Clone)]
pub enum DartNativeFunctionCallMode {
    Sync,
    Async {
        poll_symbol: String,
        complete_symbol: String,
        complete_ty: super::DartNativeType,
        cancel_symbol: String,
        free_symbol: String,
    },
}

#[derive(Debug, Clone)]
pub struct DartNativeFunction {
    pub symbol: String,
    pub params: Vec<DartNativeFunctionParam>,
    pub return_type: super::DartNativeType,
    pub is_leaf: bool,
    pub call_mode: DartNativeFunctionCallMode,
}

#[derive(Debug, Clone)]
pub struct DartNative {
    pub functions: Vec<DartNativeFunction>,
}

#[derive(Debug, Clone)]
pub struct DartFunctionParam {
    pub name: String,
    pub ty: super::DartType,
}

#[derive(Debug, Clone)]
pub struct DartFunction {
    pub name: String,
    /// The `@Native`-declared binding this method calls through (and, for
    /// async methods, its poll/complete/cancel/free siblings) — rendered via
    /// `native_function.txt`, the same template constructors and streams use.
    pub native: DartNativeFunction,
    pub params: Vec<DartFunctionParam>,
    pub ret_ty: super::DartType,
    pub receiver: Receiver,
    /// Whether the native call this method drives is async; the template
    /// wraps `ret_ty` in `Future<...>` when set.
    pub is_async: bool,
    /// The full Dart source of the method body (the statements between its
    /// braces), already assembled by the lowerer's call-body renderer.
    pub body: String,
}

impl DartFunction {
    pub fn is_static(&self) -> bool {
        matches!(self.receiver, Receiver::Static)
    }
}
