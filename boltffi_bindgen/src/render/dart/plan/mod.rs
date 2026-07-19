mod callback;
mod class;
mod custom_type;
mod enumeration;
mod function;
mod record;
mod stream;
mod r#type;

pub use callback::*;
pub use class::*;
pub use custom_type::*;
pub use enumeration::*;
pub use function::*;
pub use record::*;
pub use stream::*;
pub use r#type::*;

#[derive(Debug, Clone)]
pub enum DartConstructorKind {
    Default,
    Named { name: String },
}

#[derive(Debug, Clone)]
pub struct DartConstructor {
    pub native: DartNativeFunction,
    pub kind: DartConstructorKind,
    pub params: Vec<DartFunctionParam>,
    pub is_fallible: bool,
    /// Whether the native call this constructor drives is async. Dart
    /// `factory` constructors cannot be `async`/return a `Future`, so this
    /// currently only gates a clear "unsupported" body — see `body`.
    pub is_async: bool,
    /// The full Dart source of the constructor body (the statements between
    /// its braces), already assembled by the lowerer's call-body renderer.
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct DartLibrary {
    pub custom_types: Vec<DartCustomType>,
    pub native: DartNative,
    pub records: Vec<DartRecord>,
    pub enums: Vec<DartEnum>,
    pub callbacks: Vec<DartCallback>,
    pub classes: Vec<DartClass>,
    /// Top-level free functions (e.g. `set_http_transport`-style global
    /// setters) — public wrappers over the `@Native` declarations already
    /// emitted for `native.functions`. Not part of any class.
    pub functions: Vec<DartFunction>,
}
