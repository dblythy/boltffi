use std::collections::HashSet;

use crate::ir::abi::{AbiCall, AbiEnum, AbiEnumField, AbiEnumPayload, AbiEnumVariant, CallId};
use crate::ir::definitions::{ConstructorDef, EnumDef, EnumRepr, MethodDef, Receiver};
use crate::ir::ids::EnumId;
use crate::ir::types::TypeExpr;

use super::super::ast::{
    CSharpClassName, CSharpComment, CSharpEnumUnderlyingType, CSharpExpression, CSharpIdentity,
    CSharpLocalName, CSharpMethodName,
};
use super::super::plan::{
    CSharpEnumKind, CSharpEnumPlan, CSharpEnumVariantPlan, CSharpFieldPlan, CSharpMethodPlan,
    CSharpParamPlan, CSharpReceiver,
};
use super::lowerer::CSharpLowerer;
use super::records::constructor_return_def;
use super::wire_writers::self_wire_writer;
use super::{decode, encode, size};

impl<'a> CSharpLowerer<'a> {
    /// Lowers a Rust enum definition into the C# plan, or returns `None`
    /// when the enum is not in the supported set.
    ///
    /// The two `EnumRepr` arms carry different numbering semantics:
    ///
    /// - **C-style enums** render as `public enum X : Backing`. Each C#
    ///   member's numeric value IS the Rust discriminant, because the
    ///   value crosses P/Invoke as its backing primitive and must be
    ///   bit-for-bit identical on both sides. Gapped or negative
    ///   discriminants must be preserved.
    /// - **Data enums** render as nested `sealed record` variants
    ///   dispatched by a wire tag. Tags come from the variant's ordinal
    ///   position (`EnumTagStrategy::OrdinalIndex`). The Rust
    ///   discriminant is not part of the codec.
    pub(super) fn lower_enum(&self, enum_def: &EnumDef) -> Option<CSharpEnumPlan> {
        if !self.supported_enums.contains(&enum_def.id) {
            return None;
        }
        let class_name: CSharpClassName = (&enum_def.id).into();
        let wire_class_name = CSharpClassName::wire_helper(&class_name);
        // Variant names become nested `sealed record` types; inside the
        // abstract record's body they shadow any module-level type sharing
        // a name. Collect the set so emit helpers can qualify outer
        // references (`Demo.Point.Decode(reader)`) instead of letting them
        // resolve to the shadowing variant. Only data enums introduce
        // a nested body where shadowing applies.
        let abi_enum_for_data = match &enum_def.repr {
            EnumRepr::Data { .. } => self.abi.enums.iter().find(|e| e.id == enum_def.id),
            _ => None,
        };
        let shadowed_variant_names: HashSet<CSharpClassName> = abi_enum_for_data
            .map(|abi_enum| abi_enum.variants.iter().map(|v| (&v.name).into()).collect())
            .unwrap_or_default();
        // Constructors/methods share one C# enclosing scope too (the sealed body for data
        // enums, a methods-companion static class for C-style enums): a decode call — inside a
        // METHOD's body, or inside a VARIANT's own field decode expression, both of which live
        // in that same sealed body for a data enum — shadowed by a *sibling* constructor/method
        // name needs the same qualification a self-collision would, same widening as
        // `scope_shadow` gives classes/records/free functions. Union with the variant-name
        // shadow above rather than replacing it — either a sibling variant or a sibling member
        // can shadow, so both methods AND variant fields are qualified against the combined set.
        let member_names = enum_def
            .constructors
            .iter()
            .map(|ctor| ctor.name().map(|id| id.as_str()).unwrap_or("new"))
            .chain(enum_def.methods.iter().map(|m| m.id.as_str()));
        let mut combined_method_shadow = shadowed_variant_names.clone();
        if let Some(member_shadow) = self.scope_shadow(member_names) {
            combined_method_shadow.extend(member_shadow);
        }
        let method_shadowed = (!combined_method_shadow.is_empty()).then_some(&combined_method_shadow);
        let methods = self.lower_enum_methods(enum_def, &class_name, method_shadowed);
        let methods_class_name = if methods.is_empty() {
            None
        } else {
            Some(CSharpClassName::methods_companion(&class_name))
        };
        match &enum_def.repr {
            EnumRepr::CStyle { tag_type, variants } => {
                let lowered_variants = variants
                    .iter()
                    .enumerate()
                    .map(|(ordinal, variant)| CSharpEnumVariantPlan {
                        summary_doc: CSharpComment::from_str_option(variant.doc.as_deref()),
                        name: (&variant.name).into(),
                        tag: variant.discriminant as i32,
                        wire_tag: ordinal as i32,
                        fields: Vec::new(),
                    })
                    .collect();
                Some(CSharpEnumPlan {
                    summary_doc: CSharpComment::from_str_option(enum_def.doc.as_deref()),
                    class_name,
                    wire_class_name,
                    methods_class_name,
                    kind: CSharpEnumKind::CStyle,
                    underlying_type: Some(
                        CSharpEnumUnderlyingType::for_primitive(*tag_type)
                            .expect("supported-set filter admits only legal underlying types"),
                    ),
                    variants: lowered_variants,
                    methods,
                    is_error: enum_def.is_error,
                })
            }
            EnumRepr::Data { .. } => {
                let abi_enum = abi_enum_for_data?;
                // Share one encode/size context across all variants of
                // this enum because `WireEncodedSize` and `WireEncodeTo`
                // render all variant fields inside one method body
                // (via a switch statement). A separate decode context
                // keeps decode rendering independent. `Decode` builds
                // each variant in its own constructor call so no
                // pattern-binding leakage happens across variants.
                let mut size_locals = size::SizeLocalCounters::default();
                let mut encode_locals = encode::EncodeLocalCounters::default();
                let mut decode_locals = decode::DecodeLocalCounters::default();
                let variant_docs = enum_def.variant_docs();
                let variants = abi_enum
                    .variants
                    .iter()
                    .enumerate()
                    .map(|(ordinal, variant)| {
                        self.lower_data_enum_variant(
                            abi_enum,
                            variant,
                            variant_docs.get(ordinal).cloned().flatten(),
                            ordinal,
                            &combined_method_shadow,
                            &mut size_locals,
                            &mut encode_locals,
                            &mut decode_locals,
                        )
                    })
                    .collect();
                Some(CSharpEnumPlan {
                    summary_doc: CSharpComment::from_str_option(enum_def.doc.as_deref()),
                    class_name,
                    wire_class_name,
                    methods_class_name,
                    kind: CSharpEnumKind::Data,
                    underlying_type: None,
                    variants,
                    methods,
                    is_error: enum_def.is_error,
                })
            }
        }
    }

    /// Lowers one variant of a data enum, including its codec tag (resolved
    /// via `EnumTagStrategy`) and any payload fields.
    #[allow(clippy::too_many_arguments)]
    fn lower_data_enum_variant(
        &self,
        abi_enum: &AbiEnum,
        variant: &AbiEnumVariant,
        doc: Option<String>,
        ordinal: usize,
        shadowed: &HashSet<CSharpClassName>,
        size_locals: &mut size::SizeLocalCounters,
        encode_locals: &mut encode::EncodeLocalCounters,
        decode_locals: &mut decode::DecodeLocalCounters,
    ) -> CSharpEnumVariantPlan {
        let tag = abi_enum.resolve_codec_tag(ordinal, variant.discriminant) as i32;
        let fields = match &variant.payload {
            AbiEnumPayload::Unit => Vec::new(),
            AbiEnumPayload::Tuple(fields) | AbiEnumPayload::Struct(fields) => fields
                .iter()
                .map(|f| {
                    self.lower_variant_field(f, shadowed, size_locals, encode_locals, decode_locals)
                })
                .collect(),
        };
        CSharpEnumVariantPlan {
            summary_doc: CSharpComment::from_str_option(doc.as_deref()),
            name: (&variant.name).into(),
            tag,
            // For data enums the public surface is a `sealed record`,
            // not a numbered enum, so `tag` and `wire_tag` converge:
            // both are the ordinal dispatch value used on the wire.
            wire_tag: tag,
            fields,
        }
    }

    /// Lowers one variant payload field. Write expressions are retargeted
    /// from `this.X` to `_v.X` because the template binds each variant in
    /// its switch arm (`case Circle _v: …`), not via `this`. Decode
    /// expressions pass through the shadowing scope so outer-type
    /// references survive being rendered inside the enum's body.
    fn lower_variant_field(
        &self,
        field: &AbiEnumField,
        shadowed: &HashSet<CSharpClassName>,
        size_locals: &mut size::SizeLocalCounters,
        encode_locals: &mut encode::EncodeLocalCounters,
        decode_locals: &mut decode::DecodeLocalCounters,
    ) -> CSharpFieldPlan {
        let prefixed = Self::prefix_write_seq(&field.encode, "_v");
        let csharp_type = self
            .lower_type(&field.type_expr)
            .expect("variant field type must be supported")
            .qualify_if_shadowed(shadowed, &self.namespace);
        CSharpFieldPlan {
            // Variant payload field docs are dropped by the ABI, so we
            // can't recover them here without a wider refactor; leave
            // empty for now.
            summary_doc: None,
            name: (&field.name).into(),
            csharp_type,
            wire_decode_expr: decode::lower_decode_expr(
                &field.decode,
                &CSharpExpression::Identity(CSharpIdentity::Local(CSharpLocalName::new("reader"))),
                Some(shadowed),
                &self.namespace,
                decode_locals,
            ),
            wire_size_expr: size::lower_size_expr(
                &prefixed.size,
                &super::value::Renames::new(),
                size_locals,
            ),
            wire_encode_stmts: encode::lower_encode_expr(
                &prefixed,
                &CSharpExpression::Identity(CSharpIdentity::Local(CSharpLocalName::new("wire"))),
                &super::value::Renames::new(),
                encode_locals,
            ),
        }
    }

    /// Walks an enum's `#[data(impl)]` constructors and methods and
    /// produces the corresponding [`CSharpMethodPlan`]s. Async methods
    /// and `&mut self` / `self` receivers are dropped silently — data
    /// enums are immutable in C#, and re-binding the receiver is a wider
    /// shape question the backend doesn't model yet. Fallible
    /// (`Result<Self, _>`) and optional (`Option<Self>`) constructors
    /// are supported for data enums; C-style enum constructors need scalar
    /// return handling first (#344).
    fn lower_enum_methods(
        &self,
        enum_def: &EnumDef,
        enum_class_name: &CSharpClassName,
        shadowed: Option<&HashSet<CSharpClassName>>,
    ) -> Vec<CSharpMethodPlan> {
        let is_data = matches!(enum_def.repr, EnumRepr::Data { .. });
        let mut methods = Vec::new();

        for (index, ctor) in enum_def.constructors.iter().enumerate() {
            if !is_data && (ctor.is_fallible() || ctor.is_optional()) {
                continue;
            }
            let call_id = CallId::EnumConstructor {
                enum_id: enum_def.id.clone(),
                index,
            };
            let Some(call) = self.abi.calls.iter().find(|c| c.id == call_id) else {
                continue;
            };
            if let Some(method) =
                self.lower_enum_constructor(ctor, call, &enum_def.id, enum_class_name, shadowed)
            {
                methods.push(method);
            }
        }

        for method_def in &enum_def.methods {
            if method_def.is_async() {
                continue;
            }
            if matches!(
                method_def.receiver,
                Receiver::RefMutSelf | Receiver::OwnedSelf
            ) {
                continue;
            }
            let call_id = CallId::EnumMethod {
                enum_id: enum_def.id.clone(),
                method_id: method_def.id.clone(),
            };
            let Some(call) = self.abi.calls.iter().find(|c| c.id == call_id) else {
                continue;
            };
            if let Some(method) =
                self.lower_enum_method(method_def, call, enum_class_name, is_data, shadowed)
            {
                methods.push(method);
            }
        }

        methods
    }

    /// Lowers a `#[data(impl)]` constructor into a static factory method
    /// on the enum's container. Fallible/optional constructors
    /// synthesize a `Result<Self, String>` / `Option<Self>` return so
    /// they ride the same `return_kind` paths as everything else —
    /// errors throw, optionals become `Shape?`.
    fn lower_enum_constructor(
        &self,
        ctor: &ConstructorDef,
        call: &AbiCall,
        enum_id: &EnumId,
        enum_class_name: &CSharpClassName,
        shadowed: Option<&HashSet<CSharpClassName>>,
    ) -> Option<CSharpMethodPlan> {
        let raw_name: &str = match ctor.name() {
            Some(id) => id.as_str(),
            None => "new",
        };
        let name = CSharpMethodName::from_source(raw_name);
        let return_def = constructor_return_def(ctor, TypeExpr::Enum(enum_id.clone()));
        let return_type = self.lower_return(&return_def)?;
        let return_kind = self.return_kind(
            &return_def,
            &return_type,
            call.returns.decode_ops.as_ref(),
            shadowed,
        );
        let mut ctor_size_locals = size::SizeLocalCounters::default();
        let mut ctor_encode_locals = encode::EncodeLocalCounters::default();
        let wire_writers: Vec<_> = call
            .params
            .iter()
            .filter_map(|p| {
                self.wire_writer_for_param(p, &mut ctor_size_locals, &mut ctor_encode_locals)
            })
            .collect();
        let param_defs = ctor.params();
        let params: Vec<CSharpParamPlan> = param_defs
            .iter()
            .map(|p| self.lower_param_for_call(p, &call.params, &wire_writers))
            .collect::<Option<Vec<_>>>()?;
        Some(CSharpMethodPlan {
            summary_doc: CSharpComment::from_str_option(ctor.doc()),
            native_method_name: CSharpMethodName::native_for_owner(enum_class_name, &name),
            name,
            ffi_name: (&call.symbol).into(),
            async_call: None,
            receiver: CSharpReceiver::Static,
            params,
            return_type,
            return_kind,
            wire_writers,
            owner_is_blittable: false,
        })
    }

    /// Lowers a `#[data(impl)]` method, mapping the receiver to one of
    /// [`CSharpReceiver::Static`], [`CSharpReceiver::InstanceNative`] (data
    /// enums), or [`CSharpReceiver::InstanceExtension`] (C-style enums).
    fn lower_enum_method(
        &self,
        method_def: &MethodDef,
        call: &AbiCall,
        enum_class_name: &CSharpClassName,
        owner_is_data: bool,
        shadowed: Option<&HashSet<CSharpClassName>>,
    ) -> Option<CSharpMethodPlan> {
        let name: CSharpMethodName = (&method_def.id).into();
        let return_type = self
            .lower_return(&method_def.returns)?
            .qualify_if_shadowed_opt(shadowed, &self.namespace);
        let return_kind = self.return_kind(
            &method_def.returns,
            &return_type,
            call.returns.decode_ops.as_ref(),
            shadowed,
        );

        let receiver = match method_def.receiver {
            Receiver::Static => CSharpReceiver::Static,
            Receiver::RefSelf | Receiver::RefMutSelf | Receiver::OwnedSelf if owner_is_data => {
                CSharpReceiver::InstanceNative
            }
            Receiver::RefSelf | Receiver::RefMutSelf | Receiver::OwnedSelf => {
                CSharpReceiver::InstanceExtension
            }
        };
        // Instance methods have a synthetic `self` prepended to the ABI
        // param list; skip it when building wire writers and mapping
        // back to the explicit IR params, which never include `self`.
        let explicit_abi_params = if matches!(receiver, CSharpReceiver::Static) {
            &call.params[..]
        } else {
            &call.params[1..]
        };
        let mut method_size_locals = size::SizeLocalCounters::default();
        let mut method_encode_locals = encode::EncodeLocalCounters::default();
        let mut wire_writers: Vec<_> = Vec::new();
        if matches!(receiver, CSharpReceiver::InstanceNative) {
            wire_writers.push(self_wire_writer());
        }
        wire_writers.extend(explicit_abi_params.iter().filter_map(|p| {
            self.wire_writer_for_param(p, &mut method_size_locals, &mut method_encode_locals)
        }));
        let params: Vec<CSharpParamPlan> = method_def
            .params
            .iter()
            .map(|p| self.lower_param_for_call(p, explicit_abi_params, &wire_writers))
            .collect::<Option<Vec<_>>>()?;
        Some(CSharpMethodPlan {
            summary_doc: CSharpComment::from_str_option(method_def.doc.as_deref()),
            native_method_name: CSharpMethodName::native_for_owner(enum_class_name, &name),
            name,
            ffi_name: (&call.symbol).into(),
            async_call: None,
            receiver,
            params,
            return_type,
            return_kind,
            wire_writers,
            owner_is_blittable: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Lowerer as IrLowerer;
    use crate::ir::contract::{FfiContract, PackageInfo};
    use crate::ir::definitions::{FieldDef, RecordDef, ReturnDef, VariantPayload};
    use crate::ir::ids::{FieldName, MethodId, RecordId};
    use crate::ir::types::PrimitiveType;
    use boltffi_ffi_rules::callable::ExecutionKind;

    use super::super::super::CSharpOptions;

    /// Regression test for a gap the C# sibling-shadow widening (item 3 refinement) left open,
    /// found by independent adversarial review: a data enum's own VARIANT field decode
    /// expression lives in the identical sealed-class body as the enum's METHODS (both render
    /// inside `sealed class {{ enumeration.name() }} { ... }`), but only methods were qualified
    /// against the combined (variant-name ∪ sibling-member-name) shadow set — variant fields
    /// still saw only the narrower variant-name-only set. A data enum with a variant field
    /// decoding record `Point`, where the SAME enum also declares a method literally named
    /// `point` (no variant is named `Point`, so the narrower set misses this), reproduces the
    /// identical CS0119 shape everywhere else in this file already guards against.
    #[test]
    fn data_enum_variant_field_decode_is_qualified_when_shadowed_by_a_sibling_method_name() {
        let mut contract = FfiContract {
            package: PackageInfo {
                name: "demo_lib".to_string(),
                version: None,
            },
            functions: vec![],
            catalog: Default::default(),
        };
        contract.catalog.insert_record(RecordDef {
            id: RecordId::new("point"),
            is_repr_c: false,
            is_error: false,
            fields: vec![FieldDef {
                name: FieldName::new("x"),
                type_expr: TypeExpr::Primitive(PrimitiveType::F64),
                doc: None,
                default: None,
            }],
            constructors: vec![],
            methods: vec![],
            doc: None,
            deprecated: None,
        });

        let enum_def = EnumDef {
            id: EnumId::new("container"),
            repr: EnumRepr::Data {
                tag_type: PrimitiveType::I32,
                variants: vec![crate::ir::definitions::DataVariant {
                    name: "Wrap".into(),
                    discriminant: 0,
                    payload: VariantPayload::Struct(vec![FieldDef {
                        name: FieldName::new("point"),
                        type_expr: TypeExpr::Record(RecordId::new("point")),
                        doc: None,
                        default: None,
                    }]),
                    doc: None,
                }],
            },
            is_error: false,
            constructors: vec![],
            // No matching AbiCall is registered for this method -- `scope_shadow`'s input is the
            // enum's own method NAMES (`enum_def.methods`), computed independently of whether
            // the method itself renders, so this alone is enough to reproduce the shadow.
            methods: vec![MethodDef {
                id: MethodId::new("point"),
                receiver: Receiver::Static,
                params: vec![],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            doc: None,
            deprecated: None,
        };
        contract.catalog.insert_enum(enum_def.clone());

        let abi = IrLowerer::new(&contract).to_abi_contract();
        let options = CSharpOptions::default();
        let lowerer = CSharpLowerer::new(&contract, &abi, &options);

        let plan = lowerer
            .lower_enum(&enum_def)
            .expect("container should be admitted: its only field is a supported record");

        let decode_expr = plan.variants[0].fields[0].wire_decode_expr.to_string();
        assert_eq!(
            decode_expr, "global::DemoLib.Point.Decode(reader)",
            "expecting the variant field's Point decode to be fully qualified because the \
             enum's own sibling method `point` collides with it, same as a sibling method \
             collision would for a class/record method body; got: {decode_expr}"
        );
    }
}
