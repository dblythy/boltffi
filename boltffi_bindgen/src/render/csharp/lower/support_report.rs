//! Loud diagnostics for methods and functions the C# backend silently drops.
//!
//! `lower_function`/`lower_class_method` return `None` when a param or return type fails the
//! backend's admission gate (`predicates.rs`) — most commonly a bare recursive `#[data]` enum
//! like `ParseValue`, which the fixed-point in `admission.rs` can never admit (it needs itself
//! already admitted to admit itself; issue #186 tracks the missing C# tagged-union support this
//! would need). Before this module, that `None` was indistinguishable from "there was nothing to
//! render here" — the method just vanished from the generated `.cs` output with zero signal, so
//! a consumer's C# API surface could silently narrow on every regeneration. [`dropped_apis`]
//! re-runs the exact same admission checks the real render path uses and reports every free
//! function and class method that fails one, with the specific offending type — the CLI surfaces
//! this as a build-time warning (see `boltffi_cli`'s C# generator).
//!
//! Scoped to free functions and class methods deliberately: they're the concrete, evidenced
//! failure mode (34 of 130 real `ParseClient` methods vanished in one pack). Record/enum methods
//! and whole dropped records/enums are a coarser, currently-undiagnosed case of the same root
//! issue — noted as follow-up scope, not covered here.

use crate::ir::abi::CallId;
use crate::ir::definitions::{ClassDef, FunctionDef, MethodDef, Receiver, ReturnDef};
use crate::ir::types::TypeExpr;

use super::super::ast::CSharpClassName;
use super::lowerer::CSharpLowerer;

/// One free function or class method the C# backend could not render, and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CSharpDroppedApi {
    /// `"function"` or `"method"`.
    pub kind: &'static str,
    /// The owning class's Rust id, for a dropped method. `None` for a free function.
    pub owner: Option<String>,
    /// The function/method's Rust id (snake_case source name).
    pub name: String,
    /// Human-readable reason naming the first unsupported type found.
    pub reason: String,
}

impl<'a> CSharpLowerer<'a> {
    /// Every free function and class method this pack's admission gate rejects. Empty when
    /// every callable in the contract renders — the common case, and cheap to check even then
    /// (this re-walks the same lists `lower()` already walks, once).
    pub fn dropped_apis(&self) -> Vec<CSharpDroppedApi> {
        let mut dropped = Vec::new();

        for function in &self.ffi.functions {
            if self.lower_function(function).is_some() {
                continue;
            }
            dropped.push(CSharpDroppedApi {
                kind: "function",
                owner: None,
                name: function.id.as_str().to_string(),
                reason: self.function_rejection_reason(function),
            });
        }

        for class in self.ffi.catalog.all_classes() {
            self.dropped_class_methods(class, &mut dropped);
        }

        dropped
    }

    fn dropped_class_methods(&self, class: &ClassDef, dropped: &mut Vec<CSharpDroppedApi>) {
        let class_name = CSharpClassName::from_source(class.id.as_str());
        for method in &class.methods {
            // `OwnedSelf` is unconditionally filtered by `lower_class_methods` itself (a
            // deliberate design choice, not a type-support failure) — not a silent drop.
            if matches!(method.receiver, Receiver::OwnedSelf) {
                continue;
            }
            let call = self.abi.calls.iter().find(|c| {
                c.id == CallId::Method {
                    class_id: class.id.clone(),
                    method_id: method.id.clone(),
                }
            });
            let renders = call.is_some_and(|call| {
                self.lower_class_method(method, call, &class_name, None)
                    .is_some()
            });
            if renders {
                continue;
            }
            dropped.push(CSharpDroppedApi {
                kind: "method",
                owner: Some(class.id.as_str().to_string()),
                name: method.id.as_str().to_string(),
                reason: match call {
                    Some(_) => self.method_rejection_reason(method),
                    None => "no matching ABI call found for this method".to_string(),
                },
            });
        }
    }

    fn function_rejection_reason(&self, function: &FunctionDef) -> String {
        if let Some(reason) = self.return_rejection_reason(&function.returns) {
            return format!("return type: {reason}");
        }
        function
            .params
            .iter()
            .find(|param| !self.is_supported_param(param))
            .map(|param| {
                format!(
                    "parameter `{}`: unsupported type {}",
                    param.name.as_str(),
                    Self::type_description(&param.type_expr)
                )
            })
            .unwrap_or_else(|| {
                "unsupported call shape (no matching ABI call, or an internal lowering step \
                 failed)"
                    .to_string()
            })
    }

    fn method_rejection_reason(&self, method: &MethodDef) -> String {
        if let Some(reason) = self.return_rejection_reason(&method.returns) {
            return format!("return type: {reason}");
        }
        method
            .params
            .iter()
            .find(|param| !self.is_supported_param(param))
            .map(|param| {
                format!(
                    "parameter `{}`: unsupported type {}",
                    param.name.as_str(),
                    Self::type_description(&param.type_expr)
                )
            })
            .unwrap_or_else(|| {
                "unsupported call shape (no matching ABI call, or an internal lowering step \
                 failed)"
                    .to_string()
            })
    }

    fn return_rejection_reason(&self, returns: &ReturnDef) -> Option<String> {
        match returns {
            ReturnDef::Void => None,
            ReturnDef::Value(ty) => (!self.is_supported_type(ty))
                .then(|| format!("unsupported type {}", Self::type_description(ty))),
            ReturnDef::Result { ok, err } => {
                if !self.is_supported_result_type(ok) {
                    Some(format!(
                        "result ok: unsupported type {}",
                        Self::type_description(ok)
                    ))
                } else if !self.is_supported_result_type(err) {
                    Some(format!(
                        "result err: unsupported type {}",
                        Self::type_description(err)
                    ))
                } else {
                    None
                }
            }
        }
    }

    /// A short, human-readable name for a type, for diagnostic text only — never used to decide
    /// support (that's `predicates.rs`'s job), so it doesn't need to be exhaustive or precise
    /// about nested shapes.
    fn type_description(ty: &TypeExpr) -> String {
        match ty {
            TypeExpr::Void => "()".to_string(),
            TypeExpr::Primitive(p) => format!("{p:?}"),
            TypeExpr::String => "String".to_string(),
            TypeExpr::Builtin(id) => format!("builtin {}", id.as_str()),
            TypeExpr::Record(id) => format!("record {}", id.as_str()),
            TypeExpr::Enum(id) => format!("enum {}", id.as_str()),
            TypeExpr::Custom(id) => format!("custom type {}", id.as_str()),
            TypeExpr::Callback(id) => format!("callback {}", id.as_str()),
            TypeExpr::Vec(inner) => format!("Vec<{}>", Self::type_description(inner)),
            TypeExpr::Option(inner) => format!("Option<{}>", Self::type_description(inner)),
            TypeExpr::Result { ok, err } => format!(
                "Result<{}, {}>",
                Self::type_description(ok),
                Self::type_description(err)
            ),
            _ => "an unrecognized type shape".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{data_enum, struct_variant};
    use super::*;
    use crate::ir::Lowerer as IrLowerer;
    use crate::ir::contract::{FfiContract, PackageInfo};
    use crate::ir::definitions::{ClassDef, ConstructorDef, MethodDef, Receiver};
    use crate::ir::ids::{ClassId, EnumId, FunctionId, MethodId, ParamName};
    use crate::ir::types::TypeExpr;
    use boltffi_ffi_rules::callable::ExecutionKind;

    use super::super::super::CSharpOptions;

    fn empty_contract() -> FfiContract {
        FfiContract {
            package: PackageInfo {
                name: "demo_lib".to_string(),
                version: None,
            },
            functions: vec![],
            catalog: Default::default(),
        }
    }

    /// A self-referential `#[data]` enum (`Value { Array(Vec<Value>) }`) is exactly
    /// `parse-core-rs`'s real `ParseValue` shape: `admission.rs`'s fixed point can never admit
    /// it (it needs itself already admitted before it can admit itself), so every function or
    /// method touching it is silently dropped by the un-instrumented render path. This is the
    /// evidenced real-world failure (34 of 130 real `ParseClient` methods vanished in one pack)
    /// `dropped_apis` exists to make loud.
    fn recursive_data_enum_contract() -> FfiContract {
        let mut contract = empty_contract();
        contract.catalog.insert_enum(data_enum(
            "value",
            vec![struct_variant(
                "Array",
                0,
                vec![("items", TypeExpr::Vec(Box::new(TypeExpr::Enum(EnumId::new("value")))))],
            )],
        ));
        contract
    }

    #[test]
    fn zzz_scratch_probe_handle_divergence() {
        use crate::ir::ids::ClassId;
        let contract = empty_contract();
        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let lowerer = CSharpLowerer::new(&contract, &abi, &options);

        let handle_ty = TypeExpr::Handle(ClassId::new("other_class"));
        eprintln!("is_supported_type(Handle) = {}", lowerer.is_supported_type(&handle_ty));
        eprintln!("lower_type(Handle).is_some() = {}", lowerer.lower_type(&handle_ty).is_some());
        eprintln!(
            "lower_return(Value(Handle)).is_some() = {}",
            lowerer.lower_return(&crate::ir::definitions::ReturnDef::Value(handle_ty.clone())).is_some()
        );
    }

    #[test]
    fn dropped_apis_is_empty_when_every_callable_renders() {
        let mut contract = empty_contract();
        contract.functions.push(crate::ir::definitions::FunctionDef {
            id: FunctionId::new("ping"),
            params: vec![],
            returns: crate::ir::definitions::ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let lowerer = CSharpLowerer::new(&contract, &abi, &options);

        assert!(
            lowerer.dropped_apis().is_empty(),
            "expecting a fully-supported contract to report zero dropped APIs",
        );
    }

    #[test]
    fn dropped_apis_reports_a_free_function_with_an_unsupported_recursive_enum_param() {
        let mut contract = recursive_data_enum_contract();
        contract.functions.push(crate::ir::definitions::FunctionDef {
            id: FunctionId::new("set_value"),
            params: vec![crate::ir::definitions::ParamDef {
                name: ParamName::new("value"),
                type_expr: TypeExpr::Enum(EnumId::new("value")),
                passing: crate::ir::definitions::ParamPassing::Value,
                doc: None,
            }],
            returns: crate::ir::definitions::ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let lowerer = CSharpLowerer::new(&contract, &abi, &options);

        assert!(
            lowerer.lower_function(&contract.functions[0]).is_none(),
            "sanity: the real render path really does drop this function",
        );

        let dropped = lowerer.dropped_apis();
        assert_eq!(dropped.len(), 1, "expecting exactly the one dropped function: {dropped:?}");
        assert_eq!(dropped[0].kind, "function");
        assert_eq!(dropped[0].owner, None);
        assert_eq!(dropped[0].name, "set_value");
        assert!(
            dropped[0].reason.contains("value"),
            "expecting the reason to name the offending enum, got: {}",
            dropped[0].reason
        );
    }

    /// The exact real-world shape: a class method (`ParseClient`-style) whose param carries the
    /// unsupported recursive enum, sitting alongside other methods that DO render fine — proves
    /// the report doesn't over-fire on the whole class, only the one method that actually fails.
    #[test]
    fn dropped_apis_reports_only_the_class_method_carrying_the_unsupported_type() {
        let mut contract = recursive_data_enum_contract();
        let mut client = ClassDef {
            id: ClassId::new("parse_client"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![
                MethodDef {
                    id: MethodId::new("set_field"),
                    receiver: Receiver::RefSelf,
                    params: vec![crate::ir::definitions::ParamDef {
                        name: ParamName::new("value"),
                        type_expr: TypeExpr::Enum(EnumId::new("value")),
                        passing: crate::ir::definitions::ParamPassing::Value,
                        doc: None,
                    }],
                    returns: crate::ir::definitions::ReturnDef::Void,
                    execution_kind: ExecutionKind::Sync,
                    doc: None,
                    deprecated: None,
                },
                MethodDef {
                    id: MethodId::new("ping"),
                    receiver: Receiver::RefSelf,
                    params: vec![],
                    returns: crate::ir::definitions::ReturnDef::Void,
                    execution_kind: ExecutionKind::Sync,
                    doc: None,
                    deprecated: None,
                },
            ],
            streams: vec![],
            doc: None,
            deprecated: None,
        };
        client.constructors.truncate(0); // no ABI call registered for a made-up constructor below
        contract.catalog.insert_class(client);

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let lowerer = CSharpLowerer::new(&contract, &abi, &options);

        let dropped = lowerer.dropped_apis();
        assert_eq!(dropped.len(), 1, "expecting only set_field dropped, not ping too: {dropped:?}");
        assert_eq!(dropped[0].kind, "method");
        assert_eq!(dropped[0].owner.as_deref(), Some("parse_client"));
        assert_eq!(dropped[0].name, "set_field");
    }
}
