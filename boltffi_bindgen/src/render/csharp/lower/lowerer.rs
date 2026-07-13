use std::collections::HashSet;

use boltffi_ffi_rules::naming;

use crate::ir::definitions::CallbackKind;
use crate::ir::ids::{EnumId, RecordId};
use crate::ir::{AbiContract, FfiContract};

use super::super::CSharpOptions;
use super::super::ast::{CSharpClassName, CSharpNamespace};
use super::super::plan::{
    CFunctionName, CSharpBuiltinSet, CSharpCallbackPlan, CSharpClassPlan, CSharpClosurePlan,
    CSharpEnumPlan, CSharpFunctionPlan, CSharpModulePlan, CSharpRecordPlan,
};

/// Produces a [`CSharpModulePlan`] from the IR contracts.
pub struct CSharpLowerer<'a> {
    pub(super) ffi: &'a FfiContract,
    pub(super) abi: &'a AbiContract,
    pub(super) options: &'a CSharpOptions,
    /// The C# namespace every generated file lands in. Used by
    /// `qualify_if_shadowed` to fully-qualify type references when shadowed.
    pub(super) namespace: CSharpNamespace,
    /// Records that are fully supported: every field resolves to a type the
    /// C# backend can render. Computed jointly with `supported_enums`
    /// up-front since admission can depend on other records and on data enums.
    pub(super) supported_records: HashSet<RecordId>,
    /// Enums (C-style and data) that are fully supported. C-style admit when
    /// their `repr` is a legal C# enum backing type. Data enums admit when
    /// every variant's payload fields resolve to supported types.
    pub(super) supported_enums: HashSet<EnumId>,
}

impl<'a> CSharpLowerer<'a> {
    pub fn new(ffi: &'a FfiContract, abi: &'a AbiContract, options: &'a CSharpOptions) -> Self {
        let (supported_records, supported_enums) = Self::compute_supported_sets(ffi);
        let namespace = options
            .namespace
            .as_deref()
            .map(CSharpNamespace::new)
            .unwrap_or_else(|| CSharpNamespace::from_source(&ffi.package.name));
        Self {
            ffi,
            abi,
            options,
            namespace,
            supported_records,
            supported_enums,
        }
    }

    /// Every record/data-enum class name this pack renders a static
    /// `Decode(reader)` for.
    fn decodable_class_names(&self) -> HashSet<CSharpClassName> {
        self.supported_records
            .iter()
            .map(CSharpClassName::from)
            .chain(self.supported_enums.iter().map(CSharpClassName::from))
            .collect()
    }

    /// If `raw_name` (a method/function/constructor's own snake_case
    /// source name) renders to the same PascalCase identifier as a
    /// record or data-enum this pack decodes, returns a one-element
    /// shadow set containing it — otherwise `None`.
    ///
    /// C# member lookup resolves a member of the *enclosing*
    /// class/module before an outer-scope type of the same simple name,
    /// so an unqualified `<Type>.Decode(reader)` call inside e.g. a
    /// `ServerInfo()` method that decodes a `ServerInfo` record resolves
    /// to the method group, not the type (CS0119). Scoping the shadow
    /// set to just the colliding name (rather than qualifying every
    /// decodable type unconditionally) keeps non-colliding decode
    /// expressions in their plain, unqualified form.
    pub(super) fn self_name_shadow(&self, raw_name: &str) -> Option<HashSet<CSharpClassName>> {
        let candidate = CSharpClassName::from_source(raw_name);
        self.decodable_class_names()
            .contains(&candidate)
            .then(|| HashSet::from([candidate]))
    }

    /// Walks the contracts and produces a C# module plan.
    pub fn lower(&self) -> CSharpModulePlan {
        let lib_name = self
            .options
            .library_name
            .clone()
            .unwrap_or_else(|| naming::library_name(&self.ffi.package.name));

        let class_name = CSharpClassName::from_source(&self.ffi.package.name);
        let namespace = self.namespace.clone();
        let free_buf_ffi_name = CFunctionName::new(format!("{}_free_buf", naming::ffi_prefix()));

        let records: Vec<CSharpRecordPlan> = self
            .ffi
            .catalog
            .all_records()
            .filter(|r| self.supported_records.contains(&r.id))
            .map(|r| self.lower_record(r))
            .collect();

        let enums: Vec<CSharpEnumPlan> = self
            .ffi
            .catalog
            .all_enums()
            .filter_map(|e| self.lower_enum(e))
            .collect();

        let functions: Vec<CSharpFunctionPlan> = self
            .ffi
            .functions
            .iter()
            .filter_map(|f| self.lower_function(f))
            .collect();

        let classes: Vec<CSharpClassPlan> = self
            .ffi
            .catalog
            .all_classes()
            .map(|c| self.lower_class(c))
            .collect();

        let callbacks: Vec<CSharpCallbackPlan> = self
            .ffi
            .catalog
            .all_callbacks()
            .filter(|c| matches!(c.kind, CallbackKind::Trait))
            .map(|c| self.lower_callback(c))
            .collect();

        let closures: Vec<CSharpClosurePlan> = self
            .ffi
            .catalog
            .all_callbacks()
            .filter(|c| matches!(c.kind, CallbackKind::Closure))
            .map(|c| self.lower_closure(c))
            .collect();

        let builtins = CSharpBuiltinSet::from_kinds(
            self.ffi
                .catalog
                .all_builtins()
                .map(|builtin| builtin.kind)
                .collect(),
        );

        CSharpModulePlan {
            namespace,
            class_name,
            lib_name,
            free_buf_ffi_name,
            records,
            enums,
            functions,
            classes,
            callbacks,
            closures,
            builtins,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Lowerer as IrLowerer;
    use crate::ir::contract::{FfiContract, PackageInfo};
    use crate::ir::definitions::{
        ClassDef, ConstructorDef, FieldDef, FunctionDef, MethodDef, ParamDef, ParamPassing,
        Receiver, RecordDef, ReturnDef, StreamDef, StreamMode,
    };
    use crate::ir::ids::{ClassId, FieldName, FunctionId, MethodId, ParamName, RecordId, StreamId};
    use crate::ir::types::{PrimitiveType, TypeExpr};
    use boltffi_ffi_rules::callable::ExecutionKind;

    use super::super::super::CSharpOptions;
    use super::super::super::plan::CSharpStreamDelivery;

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

    fn primitive_param(name: &str, primitive: PrimitiveType) -> ParamDef {
        ParamDef {
            name: ParamName::new(name),
            type_expr: TypeExpr::Primitive(primitive),
            passing: ParamPassing::Value,
            doc: None,
        }
    }

    #[test]
    fn lowerer_preserves_doc_comments_on_supported_surface_items() {
        let mut contract = empty_contract();
        contract.functions.push(FunctionDef {
            id: FunctionId::new("ping"),
            params: vec![],
            returns: ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: Some("Pings <native> & returns.".to_string()),
            deprecated: None,
        });
        contract.catalog.insert_record(RecordDef {
            id: RecordId::new("person"),
            is_repr_c: false,
            is_error: false,
            fields: vec![FieldDef {
                name: FieldName::new("age"),
                type_expr: TypeExpr::Primitive(PrimitiveType::U32),
                doc: Some("Age in years.".to_string()),
                default: None,
            }],
            constructors: vec![],
            methods: vec![],
            doc: Some("A person record.".to_string()),
            deprecated: None,
        });
        contract.catalog.insert_class(ClassDef {
            id: ClassId::new("counter"),
            constructors: vec![ConstructorDef::Default {
                params: vec![],
                is_fallible: false,
                is_optional: false,
                doc: Some("Creates a counter.".to_string()),
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: MethodId::new("get"),
                receiver: Receiver::RefSelf,
                params: vec![primitive_param("scale", PrimitiveType::I32)],
                returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::I32)),
                execution_kind: ExecutionKind::Sync,
                doc: Some("Returns the current value.".to_string()),
                deprecated: None,
            }],
            streams: vec![],
            doc: Some("Mutable counter.".to_string()),
            deprecated: None,
        });

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let module = CSharpLowerer::new(&contract, &abi, &options).lower();

        assert_eq!(
            module.functions[0]
                .summary_doc
                .as_ref()
                .map(ToString::to_string),
            Some("Pings &lt;native&gt; &amp; returns.".to_string())
        );
        assert_eq!(
            module.records[0]
                .summary_doc
                .as_ref()
                .map(ToString::to_string),
            Some("A person record.".to_string())
        );
        assert_eq!(
            module.records[0].fields[0]
                .summary_doc
                .as_ref()
                .map(ToString::to_string),
            Some("Age in years.".to_string())
        );
        assert_eq!(
            module.classes[0]
                .summary_doc
                .as_ref()
                .map(ToString::to_string),
            Some("Mutable counter.".to_string())
        );
        assert_eq!(
            module.classes[0].constructors[0]
                .summary_doc
                .as_ref()
                .map(ToString::to_string),
            Some("Creates a counter.".to_string())
        );
        assert_eq!(
            module.classes[0].methods[0]
                .summary_doc
                .as_ref()
                .map(ToString::to_string),
            Some("Returns the current value.".to_string())
        );
    }

    #[test]
    fn lowerer_uses_configured_namespace_override() {
        let contract = empty_contract();
        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions {
            namespace: Some("CounterApp.Shared".to_string()),
            ..Default::default()
        };
        let module = CSharpLowerer::new(&contract, &abi, &options).lower();

        assert_eq!(module.namespace.to_string(), "CounterApp.Shared");
    }

    #[test]
    fn lowerer_matches_public_param_when_synthetic_len_name_collides() {
        let mut contract = empty_contract();
        contract.catalog.insert_record(RecordDef {
            id: RecordId::new("point"),
            is_repr_c: true,
            is_error: false,
            fields: vec![
                FieldDef {
                    name: FieldName::new("x"),
                    type_expr: TypeExpr::Primitive(PrimitiveType::F64),
                    doc: None,
                    default: None,
                },
                FieldDef {
                    name: FieldName::new("y"),
                    type_expr: TypeExpr::Primitive(PrimitiveType::F64),
                    doc: None,
                    default: None,
                },
            ],
            constructors: vec![],
            methods: vec![],
            doc: None,
            deprecated: None,
        });
        contract.functions.push(FunctionDef {
            id: FunctionId::new("path_length"),
            params: vec![
                ParamDef {
                    name: ParamName::new("points"),
                    type_expr: TypeExpr::Vec(Box::new(TypeExpr::Record(RecordId::new("point")))),
                    passing: ParamPassing::Ref,
                    doc: None,
                },
                ParamDef {
                    name: ParamName::new("points_len"),
                    type_expr: TypeExpr::Vec(Box::new(TypeExpr::Record(RecordId::new("point")))),
                    passing: ParamPassing::Value,
                    doc: None,
                },
            ],
            returns: ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let module = CSharpLowerer::new(&contract, &abi, &options).lower();

        assert_eq!(module.functions.len(), 1);
        assert_eq!(
            module.functions[0].native_call_args().to_string(),
            "(IntPtr)_pointsPtr, (UIntPtr)points.Length, (IntPtr)_pointsLenPtr, (UIntPtr)(pointsLen.Length * Unsafe.SizeOf<Point>())"
        );
    }

    #[test]
    fn lowerer_accepts_wire_encoded_stream_item() {
        let mut contract = empty_contract();
        contract.catalog.insert_class(ClassDef {
            id: ClassId::new("event_bus"),
            constructors: vec![],
            methods: vec![],
            streams: vec![StreamDef {
                id: StreamId::new("subscribe_labels"),
                item_type: TypeExpr::String,
                mode: StreamMode::Async,
                doc: None,
                deprecated: None,
            }],
            doc: None,
            deprecated: None,
        });

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let module = CSharpLowerer::new(&contract, &abi, &options).lower();

        let stream = &module.classes[0].streams[0];
        assert!(matches!(
            &stream.delivery,
            CSharpStreamDelivery::WireEncoded { .. }
        ));
    }
}
