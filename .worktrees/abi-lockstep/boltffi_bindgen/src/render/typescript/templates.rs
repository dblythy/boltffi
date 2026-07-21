use askama::Template;

use super::plan::*;

pub fn ts_doc_block(doc: &Option<String>, indent: &str) -> String {
    match doc {
        Some(text) => {
            let mut result = format!("{indent}/**\n");
            text.lines().for_each(|line| {
                if line.is_empty() {
                    result.push_str(&format!("{indent} *\n"));
                } else {
                    result.push_str(&format!("{indent} * {line}\n"));
                }
            });
            result.push_str(&format!("{indent} */\n"));
            result
        }
        None => String::new(),
    }
}

#[derive(Template)]
#[template(path = "render_typescript/preamble.txt", escape = "none")]
pub struct PreambleTemplate {
    pub abi_version: u32,
    /// Selects the native bootstrap (`instantiateBoltFFINative`/`NativeBoltFFIModule`) over the
    /// default wasm one (`instantiateBoltFFI`/`BoltFFIModule`) — see `TsModule::native_async`'s
    /// doc. Consumers previously had to mechanically patch this preamble by hand after
    /// generation (`scripts/patch-react-native-bootstrap.mjs` in the parse-core-sdks repo); this
    /// flag makes that patch unnecessary.
    pub native_async: bool,
}

#[derive(Template)]
#[template(path = "render_typescript/preamble_node.txt", escape = "none")]
pub struct NodePreambleTemplate {
    pub abi_version: u32,
    pub module_name: String,
}

#[derive(Template)]
#[template(path = "render_typescript/footer_node.txt", escape = "none")]
pub struct NodeFooterTemplate;

#[derive(Template)]
#[template(path = "render_typescript/record.txt", escape = "none")]
pub struct RecordTemplate<'a> {
    pub name: &'a str,
    pub fields: &'a [TsField],
    pub is_blittable: bool,
    pub wire_size: Option<usize>,
    pub tail_padding: usize,
    pub size_expr: String,
    pub doc: &'a Option<String>,
}

impl<'a> RecordTemplate<'a> {
    pub fn from_record(record: &'a TsRecord) -> Self {
        let size_expr = if let Some(size) = record.wire_size {
            size.to_string()
        } else {
            record
                .fields
                .iter()
                .map(|f| f.wire_size_expr("v"))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        Self {
            name: &record.name,
            fields: &record.fields,
            is_blittable: record.is_blittable,
            wire_size: record.wire_size,
            tail_padding: record.tail_padding,
            size_expr,
            doc: &record.doc,
        }
    }
}

#[derive(Template)]
#[template(path = "render_typescript/value_type_companion.txt", escape = "none")]
pub struct ValueTypeCompanionTemplate<'a> {
    pub name: &'a str,
    pub constructors: &'a [TsValueTypeConstructor],
    pub methods: &'a [TsValueTypeMethod],
}

#[derive(Template)]
#[template(path = "render_typescript/enum_c_style.txt", escape = "none")]
pub struct EnumCStyleTemplate<'a> {
    pub name: &'a str,
    pub variants: &'a [TsVariant],
    pub doc: &'a Option<String>,
}

#[derive(Template)]
#[template(path = "render_typescript/enum_namespace.txt", escape = "none")]
pub struct EnumNamespaceTemplate<'a> {
    pub name: &'a str,
    pub constructors: &'a [TsValueTypeConstructor],
    pub methods: &'a [TsValueTypeMethod],
}

#[derive(Template)]
#[template(path = "render_typescript/enum_data.txt", escape = "none")]
pub struct EnumDataTemplate<'a> {
    pub name: &'a str,
    pub variants: &'a [TsVariant],
    pub doc: &'a Option<String>,
}

#[derive(Template)]
#[template(path = "render_typescript/error_exception.txt", escape = "none")]
pub struct ErrorExceptionTemplate<'a> {
    pub type_name: &'a str,
    pub class_name: &'a str,
    pub is_c_style_enum: bool,
}

#[derive(Template)]
#[template(path = "render_typescript/function.txt", escape = "none")]
pub struct FunctionTemplate<'a> {
    pub name: &'a str,
    pub params: &'a [TsParam],
    pub return_type_str: &'a str,
    pub return_route: &'a TsOutputRoute,
    pub return_callback: &'a Option<TsCallbackHandleReturn>,
    pub ffi_name: &'a str,
    pub call_args: &'a str,
    pub call_args_with_out: &'a str,
    pub wrapper_code: &'a str,
    pub cleanup_code: &'a str,
    pub doc: &'a Option<String>,
}

#[derive(Template)]
#[template(path = "render_typescript/class.txt", escape = "none")]
pub struct ClassTemplate<'a> {
    pub cls: &'a TsClass,
}

#[derive(Template)]
#[template(path = "render_typescript/callback.txt", escape = "none")]
pub struct CallbackTemplate<'a> {
    pub callback: &'a TsCallback,
}

#[derive(Template)]
#[template(path = "render_typescript/async_function.txt", escape = "none")]
pub struct AsyncFunctionTemplate<'a> {
    pub name: &'a str,
    pub params: &'a [TsParam],
    pub return_type_str: &'a str,
    pub entry_ffi_name: &'a str,
    pub poll_sync_ffi_name: &'a str,
    pub poll_ffi_name: &'a str,
    pub native_async: bool,
    pub complete_ffi_name: &'a str,
    pub panic_message_ffi_name: &'a str,
    pub free_ffi_name: &'a str,
    pub call_args: &'a str,
    pub wrapper_code: &'a str,
    pub cleanup_code: &'a str,
    pub return_route: &'a TsOutputRoute,
    pub return_callback: &'a Option<TsCallbackHandleReturn>,
    pub doc: &'a Option<String>,
}

#[derive(Template)]
#[template(path = "render_typescript/wasm_exports.txt", escape = "none")]
pub struct WasmExportsTemplate<'a> {
    pub wasm_imports: &'a [TsWasmImportView<'a>],
}

pub struct TsWasmImportView<'a> {
    pub ffi_name: &'a str,
    pub params: &'a [TsWasmParam],
    pub return_wasm_type_str: &'a str,
}

pub struct TypeScriptEmitter;

impl TypeScriptEmitter {
    pub fn emit(module: &TsModule) -> String {
        let mut output = String::new();

        output.push_str(
            &PreambleTemplate {
                abi_version: module.abi_version,
                native_async: module.native_async,
            }
            .render()
            .unwrap(),
        );
        output.push('\n');

        for record in &module.records {
            output.push_str(&RecordTemplate::from_record(record).render().unwrap());
            if record.has_companion() {
                output.push_str("\n\n");
                output.push_str(
                    &ValueTypeCompanionTemplate {
                        name: &record.name,
                        constructors: &record.constructors,
                        methods: &record.methods,
                    }
                    .render()
                    .unwrap(),
                );
            }
            output.push_str("\n\n");
        }

        for enumeration in &module.enums {
            if enumeration.is_c_style() {
                output.push_str(
                    &EnumCStyleTemplate {
                        name: &enumeration.name,
                        variants: &enumeration.variants,
                        doc: &enumeration.doc,
                    }
                    .render()
                    .unwrap(),
                );
                if enumeration.has_companion() {
                    output.push_str("\n\n");
                    output.push_str(
                        &EnumNamespaceTemplate {
                            name: &enumeration.name,
                            constructors: &enumeration.constructors,
                            methods: &enumeration.methods,
                        }
                        .render()
                        .unwrap(),
                    );
                }
            } else {
                output.push_str(
                    &EnumDataTemplate {
                        name: &enumeration.name,
                        variants: &enumeration.variants,
                        doc: &enumeration.doc,
                    }
                    .render()
                    .unwrap(),
                );
                if enumeration.has_companion() {
                    output.push_str("\n\n");
                    output.push_str(
                        &ValueTypeCompanionTemplate {
                            name: &enumeration.name,
                            constructors: &enumeration.constructors,
                            methods: &enumeration.methods,
                        }
                        .render()
                        .unwrap(),
                    );
                }
            }
            output.push_str("\n\n");
        }

        for error_exception in &module.error_exceptions {
            output.push_str(
                &ErrorExceptionTemplate {
                    type_name: &error_exception.type_name,
                    class_name: &error_exception.class_name,
                    is_c_style_enum: error_exception.is_c_style_enum,
                }
                .render()
                .unwrap(),
            );
            output.push_str("\n\n");
        }

        for function in &module.functions {
            let call_args = function
                .params
                .iter()
                .flat_map(|p| p.ffi_args())
                .collect::<Vec<_>>()
                .join(", ");
            let call_args_with_out = if call_args.is_empty() {
                "outPtr".to_string()
            } else {
                format!("outPtr, {call_args}")
            };

            let wrapper_code = function
                .params
                .iter()
                .filter_map(|p| p.wrapper_code())
                .collect::<Vec<_>>()
                .join("\n  ");

            let cleanup_code = function
                .params
                .iter()
                .filter_map(|p| p.cleanup_code())
                .collect::<Vec<_>>()
                .join("\n  ");

            let return_type_str = function.return_type.as_deref().unwrap_or("void");

            output.push_str(
                &FunctionTemplate {
                    name: &function.name,
                    params: &function.params,
                    return_type_str,
                    return_route: &function.return_route,
                    return_callback: &function.return_callback,
                    ffi_name: &function.ffi_name,
                    call_args: &call_args,
                    call_args_with_out: &call_args_with_out,
                    wrapper_code: &wrapper_code,
                    cleanup_code: &cleanup_code,
                    doc: &function.doc,
                }
                .render()
                .unwrap(),
            );
            output.push_str("\n\n");
        }

        for async_function in &module.async_functions {
            let call_args = async_function
                .params
                .iter()
                .flat_map(|p| p.ffi_args())
                .collect::<Vec<_>>()
                .join(", ");

            let wrapper_code = async_function
                .params
                .iter()
                .filter_map(|p| p.wrapper_code())
                .collect::<Vec<_>>()
                .join("\n    ");

            let cleanup_code = async_function
                .params
                .iter()
                .filter_map(|p| p.cleanup_code())
                .collect::<Vec<_>>()
                .join("\n    ");

            let return_type_str = async_function.return_type.as_deref().unwrap_or("void");

            output.push_str(
                &AsyncFunctionTemplate {
                    name: &async_function.name,
                    params: &async_function.params,
                    return_type_str,
                    entry_ffi_name: &async_function.entry_ffi_name,
                    poll_sync_ffi_name: &async_function.poll_sync_ffi_name,
                    poll_ffi_name: &async_function.poll_ffi_name,
                    native_async: async_function.native_async,
                    complete_ffi_name: &async_function.complete_ffi_name,
                    panic_message_ffi_name: &async_function.panic_message_ffi_name,
                    free_ffi_name: &async_function.free_ffi_name,
                    call_args: &call_args,
                    wrapper_code: &wrapper_code,
                    cleanup_code: &cleanup_code,
                    return_route: &async_function.return_route,
                    return_callback: &async_function.return_callback,
                    doc: &async_function.doc,
                }
                .render()
                .unwrap(),
            );
            output.push_str("\n\n");
        }

        for class in &module.classes {
            output.push_str(&ClassTemplate { cls: class }.render().unwrap());
            output.push_str("\n\n");
        }

        for callback in &module.callbacks {
            output.push_str(&CallbackTemplate { callback }.render().unwrap());
            output.push_str("\n\n");
        }

        let wasm_import_views: Vec<TsWasmImportView> = module
            .wasm_imports
            .iter()
            .map(|import| TsWasmImportView {
                ffi_name: &import.ffi_name,
                params: &import.params,
                return_wasm_type_str: import.return_wasm_type.as_deref().unwrap_or("void"),
            })
            .collect();

        output.push_str(
            &WasmExportsTemplate {
                wasm_imports: &wasm_import_views,
            }
            .render()
            .unwrap(),
        );
        output.push('\n');

        output
    }

    pub fn emit_node(module: &TsModule, module_name: &str) -> String {
        let mut output = String::new();

        output.push_str(
            &NodePreambleTemplate {
                abi_version: module.abi_version,
                module_name: module_name.to_string(),
            }
            .render()
            .unwrap(),
        );
        output.push('\n');

        for record in &module.records {
            output.push_str(&RecordTemplate::from_record(record).render().unwrap());
            if record.has_companion() {
                output.push_str("\n\n");
                output.push_str(
                    &ValueTypeCompanionTemplate {
                        name: &record.name,
                        constructors: &record.constructors,
                        methods: &record.methods,
                    }
                    .render()
                    .unwrap(),
                );
            }
            output.push_str("\n\n");
        }

        for enumeration in &module.enums {
            if enumeration.is_c_style() {
                output.push_str(
                    &EnumCStyleTemplate {
                        name: &enumeration.name,
                        variants: &enumeration.variants,
                        doc: &enumeration.doc,
                    }
                    .render()
                    .unwrap(),
                );
                if enumeration.has_companion() {
                    output.push_str("\n\n");
                    output.push_str(
                        &EnumNamespaceTemplate {
                            name: &enumeration.name,
                            constructors: &enumeration.constructors,
                            methods: &enumeration.methods,
                        }
                        .render()
                        .unwrap(),
                    );
                }
            } else {
                output.push_str(
                    &EnumDataTemplate {
                        name: &enumeration.name,
                        variants: &enumeration.variants,
                        doc: &enumeration.doc,
                    }
                    .render()
                    .unwrap(),
                );
                if enumeration.has_companion() {
                    output.push_str("\n\n");
                    output.push_str(
                        &ValueTypeCompanionTemplate {
                            name: &enumeration.name,
                            constructors: &enumeration.constructors,
                            methods: &enumeration.methods,
                        }
                        .render()
                        .unwrap(),
                    );
                }
            }
            output.push_str("\n\n");
        }

        for error_exception in &module.error_exceptions {
            output.push_str(
                &ErrorExceptionTemplate {
                    type_name: &error_exception.type_name,
                    class_name: &error_exception.class_name,
                    is_c_style_enum: error_exception.is_c_style_enum,
                }
                .render()
                .unwrap(),
            );
            output.push_str("\n\n");
        }

        for callback in &module.callbacks {
            output.push_str(&CallbackTemplate { callback }.render().unwrap());
            output.push_str("\n\n");
        }

        let wasm_import_views: Vec<TsWasmImportView> = module
            .wasm_imports
            .iter()
            .map(|import| TsWasmImportView {
                ffi_name: &import.ffi_name,
                params: &import.params,
                return_wasm_type_str: import.return_wasm_type.as_deref().unwrap_or("void"),
            })
            .collect();

        output.push_str(
            &WasmExportsTemplate {
                wasm_imports: &wasm_import_views,
            }
            .render()
            .unwrap(),
        );
        output.push('\n');

        output.push_str(&NodeFooterTemplate.render().unwrap());
        output.push_str("\n\n");

        for function in &module.functions {
            let call_args = function
                .params
                .iter()
                .flat_map(|p| p.ffi_args())
                .collect::<Vec<_>>()
                .join(", ");
            let call_args_with_out = if call_args.is_empty() {
                "outPtr".to_string()
            } else {
                format!("outPtr, {call_args}")
            };

            let wrapper_code = function
                .params
                .iter()
                .filter_map(|p| p.wrapper_code())
                .collect::<Vec<_>>()
                .join("\n  ");

            let cleanup_code = function
                .params
                .iter()
                .filter_map(|p| p.cleanup_code())
                .collect::<Vec<_>>()
                .join("\n  ");

            let return_type_str = function.return_type.as_deref().unwrap_or("void");

            output.push_str(
                &FunctionTemplate {
                    name: &function.name,
                    params: &function.params,
                    return_type_str,
                    return_route: &function.return_route,
                    return_callback: &function.return_callback,
                    ffi_name: &function.ffi_name,
                    call_args: &call_args,
                    call_args_with_out: &call_args_with_out,
                    wrapper_code: &wrapper_code,
                    cleanup_code: &cleanup_code,
                    doc: &function.doc,
                }
                .render()
                .unwrap(),
            );
            output.push_str("\n\n");
        }

        for async_function in &module.async_functions {
            let call_args = async_function
                .params
                .iter()
                .flat_map(|p| p.ffi_args())
                .collect::<Vec<_>>()
                .join(", ");

            let wrapper_code = async_function
                .params
                .iter()
                .filter_map(|p| p.wrapper_code())
                .collect::<Vec<_>>()
                .join("\n  ");

            let cleanup_code = async_function
                .params
                .iter()
                .filter_map(|p| p.cleanup_code())
                .collect::<Vec<_>>()
                .join("\n  ");

            let return_type_str = async_function.return_type.as_deref().unwrap_or("void");

            output.push_str(
                &AsyncFunctionTemplate {
                    name: &async_function.name,
                    params: &async_function.params,
                    return_type_str,
                    entry_ffi_name: &async_function.entry_ffi_name,
                    poll_sync_ffi_name: &async_function.poll_sync_ffi_name,
                    poll_ffi_name: &async_function.poll_ffi_name,
                    native_async: async_function.native_async,
                    complete_ffi_name: &async_function.complete_ffi_name,
                    panic_message_ffi_name: &async_function.panic_message_ffi_name,
                    free_ffi_name: &async_function.free_ffi_name,
                    call_args: &call_args,
                    wrapper_code: &wrapper_code,
                    cleanup_code: &cleanup_code,
                    return_route: &async_function.return_route,
                    return_callback: &async_function.return_callback,
                    doc: &async_function.doc,
                }
                .render()
                .unwrap(),
            );
            output.push_str("\n\n");
        }

        for class in &module.classes {
            output.push_str(&ClassTemplate { cls: class }.render().unwrap());
            output.push_str("\n\n");
        }

        output
    }
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;
    use crate::ir::ids::FieldName;
    use crate::ir::ops::{
        OffsetExpr, ReadOp, ReadSeq, SizeExpr, ValueExpr, WireShape, WriteOp, WriteSeq,
    };
    use crate::ir::types::PrimitiveType;

    fn primitive_size(p: PrimitiveType) -> usize {
        match p {
            PrimitiveType::Bool | PrimitiveType::I8 | PrimitiveType::U8 => 1,
            PrimitiveType::I16 | PrimitiveType::U16 => 2,
            PrimitiveType::I32 | PrimitiveType::U32 | PrimitiveType::F32 => 4,
            PrimitiveType::I64
            | PrimitiveType::U64
            | PrimitiveType::F64
            | PrimitiveType::ISize
            | PrimitiveType::USize => 8,
        }
    }

    fn primitive_read(primitive: PrimitiveType) -> ReadSeq {
        ReadSeq {
            size: SizeExpr::Fixed(primitive_size(primitive)),
            ops: vec![ReadOp::Primitive {
                primitive,
                offset: OffsetExpr::Base,
            }],
            shape: WireShape::Value,
        }
    }

    fn primitive_write(primitive: PrimitiveType, field: &str) -> WriteSeq {
        WriteSeq {
            size: SizeExpr::Fixed(primitive_size(primitive)),
            ops: vec![WriteOp::Primitive {
                primitive,
                value: ValueExpr::Field(
                    Box::new(ValueExpr::Var("value".to_string())),
                    FieldName::new(field),
                ),
            }],
            shape: WireShape::Value,
        }
    }

    fn string_read() -> ReadSeq {
        ReadSeq {
            size: SizeExpr::Runtime,
            ops: vec![ReadOp::String {
                offset: OffsetExpr::Base,
            }],
            shape: WireShape::Value,
        }
    }

    fn string_write(field: &str) -> WriteSeq {
        WriteSeq {
            size: SizeExpr::StringLen(ValueExpr::Field(
                Box::new(ValueExpr::Var("value".to_string())),
                FieldName::new(field),
            )),
            ops: vec![WriteOp::String {
                value: ValueExpr::Field(
                    Box::new(ValueExpr::Var("value".to_string())),
                    FieldName::new(field),
                ),
            }],
            shape: WireShape::Value,
        }
    }

    #[test]
    fn snapshot_preamble() {
        let output = PreambleTemplate {
            abi_version: 1,
            native_async: false,
        }
        .render()
        .unwrap();
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_preamble_native_async() {
        // The react-native track's stage-4 gap this closes: native_async generation used to emit
        // the byte-identical wasm bootstrap (`instantiateBoltFFI`/`BoltFFIModule`), requiring the
        // consumer repo to hand-patch it after every generation
        // (`scripts/patch-react-native-bootstrap.mjs`). This snapshot pins the native bootstrap
        // this flag now emits instead.
        let output = PreambleTemplate {
            abi_version: 1,
            native_async: true,
        }
        .render()
        .unwrap();
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_record_with_primitive_fields() {
        let record = TsRecord {
            name: "Point".to_string(),
            fields: vec![
                TsField {
                    name: "x".to_string(),
                    ts_type: "number".to_string(),
                    decode: primitive_read(PrimitiveType::F64),
                    encode: primitive_write(PrimitiveType::F64, "x"),
                    doc: None,
                },
                TsField {
                    name: "y".to_string(),
                    ts_type: "number".to_string(),
                    decode: primitive_read(PrimitiveType::F64),
                    encode: primitive_write(PrimitiveType::F64, "y"),
                    doc: None,
                },
            ],
            constructors: vec![],
            methods: vec![],
            is_blittable: true,
            wire_size: Some(16),
            tail_padding: 0,
            doc: None,
        };

        let template = RecordTemplate::from_record(&record);
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_record_with_string_field() {
        let record = TsRecord {
            name: "User".to_string(),
            fields: vec![
                TsField {
                    name: "id".to_string(),
                    ts_type: "number".to_string(),
                    decode: primitive_read(PrimitiveType::I32),
                    encode: primitive_write(PrimitiveType::I32, "id"),
                    doc: None,
                },
                TsField {
                    name: "name".to_string(),
                    ts_type: "string".to_string(),
                    decode: string_read(),
                    encode: string_write("name"),
                    doc: Some("The user's display name".to_string()),
                },
            ],
            constructors: vec![],
            methods: vec![],
            is_blittable: false,
            wire_size: None,
            tail_padding: 0,
            doc: Some("A user record".to_string()),
        };

        let template = RecordTemplate::from_record(&record);
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_enum_c_style() {
        let doc = Some("A color enum".to_string());
        let variants = vec![
            TsVariant {
                name: "Red".to_string(),
                discriminant: 0,
                fields: vec![],
                doc: None,
            },
            TsVariant {
                name: "Green".to_string(),
                discriminant: 1,
                fields: vec![],
                doc: None,
            },
            TsVariant {
                name: "Blue".to_string(),
                discriminant: 2,
                fields: vec![],
                doc: Some("The blue channel".to_string()),
            },
        ];
        let template = EnumCStyleTemplate {
            name: "Color",
            variants: &variants,
            doc: &doc,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_enum_c_style_u8_tag() {
        let doc = Some("Byte-sized enum".to_string());
        let variants = vec![
            TsVariant {
                name: "None".to_string(),
                discriminant: 0,
                fields: vec![],
                doc: None,
            },
            TsVariant {
                name: "Some".to_string(),
                discriminant: 255,
                fields: vec![],
                doc: None,
            },
        ];
        let template = EnumCStyleTemplate {
            name: "ByteState",
            variants: &variants,
            doc: &doc,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_enum_data() {
        let doc: Option<String> = None;
        let variants = vec![
            TsVariant {
                name: "Circle".to_string(),
                discriminant: 0,
                fields: vec![TsVariantField {
                    name: "radius".to_string(),
                    ts_type: "number".to_string(),
                    decode: primitive_read(PrimitiveType::F64),
                    encode: primitive_write(PrimitiveType::F64, "radius"),
                }],
                doc: None,
            },
            TsVariant {
                name: "Rectangle".to_string(),
                discriminant: 1,
                fields: vec![
                    TsVariantField {
                        name: "width".to_string(),
                        ts_type: "number".to_string(),
                        decode: primitive_read(PrimitiveType::F64),
                        encode: primitive_write(PrimitiveType::F64, "width"),
                    },
                    TsVariantField {
                        name: "height".to_string(),
                        ts_type: "number".to_string(),
                        decode: primitive_read(PrimitiveType::F64),
                        encode: primitive_write(PrimitiveType::F64, "height"),
                    },
                ],
                doc: None,
            },
            TsVariant {
                name: "Nothing".to_string(),
                discriminant: 2,
                fields: vec![],
                doc: Some("An empty shape".to_string()),
            },
        ];
        let template = EnumDataTemplate {
            name: "Shape",
            variants: &variants,
            doc: &doc,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_function_void() {
        let doc: Option<String> = None;
        let template = FunctionTemplate {
            name: "reset",
            params: &[],
            return_type_str: "void",
            return_route: &TsOutputRoute::void(),
            return_callback: &None,
            ffi_name: "boltffi_reset",
            call_args: "",
            call_args_with_out: "outPtr",
            wrapper_code: "",
            cleanup_code: "",
            doc: &doc,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_function_direct_return() {
        let doc = Some("Adds two numbers".to_string());
        let params = vec![
            TsParam {
                name: "a".to_string(),
                ts_type: "number".to_string(),
                input_route: TsInputRoute::Direct,
            },
            TsParam {
                name: "b".to_string(),
                ts_type: "number".to_string(),
                input_route: TsInputRoute::Direct,
            },
        ];
        let template = FunctionTemplate {
            name: "add",
            params: &params,
            return_type_str: "number",
            return_route: &TsOutputRoute::direct(String::new()),
            return_callback: &None,
            ffi_name: "boltffi_add",
            call_args: "a, b",
            call_args_with_out: "outPtr, a, b",
            wrapper_code: "",
            cleanup_code: "",
            doc: &doc,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_function_wire_encoded_return() {
        let doc: Option<String> = None;
        let template = FunctionTemplate {
            name: "getUsers",
            params: &[],
            return_type_str: "User[]",
            return_route: &TsOutputRoute::packed(
                "reader.readArray(() => decodeUser(reader))".to_string(),
            ),
            return_callback: &None,
            ffi_name: "boltffi_get_users",
            call_args: "",
            call_args_with_out: "",
            wrapper_code: "",
            cleanup_code: "",
            doc: &doc,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn function_template_uses_number_carrier_for_nan_boxed_optional_return() {
        let doc: Option<String> = None;
        let template = FunctionTemplate {
            name: "findEven",
            params: &[],
            return_type_str: "number | null",
            return_route: &TsOutputRoute::nan_boxed_optional(
                "_module.unpackOptionI32(packed)".to_string(),
            ),
            return_callback: &None,
            ffi_name: "boltffi_find_even",
            call_args: "",
            call_args_with_out: "",
            wrapper_code: "",
            cleanup_code: "",
            doc: &doc,
        };

        let rendered = template.render().unwrap();
        assert!(
            rendered
                .contains("const packed = (_exports.boltffi_find_even as Function)() as number;")
        );
        assert!(rendered.contains("return _module.unpackOptionI32(packed);"));
    }

    #[test]
    fn async_function_param_cleanup_runs_after_await() {
        let doc: Option<String> = None;
        let params = vec![TsParam {
            name: "message".to_string(),
            ts_type: "Message".to_string(),
            input_route: TsInputRoute::CodecEncoded {
                codec_name: "MessageCodec".to_string(),
            },
        }];
        let rendered = AsyncFunctionTemplate {
            name: "sendMessage",
            params: &params,
            return_type_str: "Response",
            entry_ffi_name: "boltffi_send_message",
            poll_sync_ffi_name: "boltffi_send_message_poll_sync",
            poll_ffi_name: "boltffi_send_message_poll",
            native_async: false,
            complete_ffi_name: "boltffi_send_message_complete",
            panic_message_ffi_name: "boltffi_send_message_panic_message",
            free_ffi_name: "boltffi_send_message_free",
            call_args: "message_writer.ptr, message_writer.len",
            wrapper_code: "const message_writer = _module.allocWriter(MessageCodec.size(message));\n  MessageCodec.encode(message_writer, message);",
            cleanup_code: "_module.freeWriter(message_writer);",
            return_route: &TsOutputRoute::packed("ResponseCodec.decode(reader)".to_string()),
            return_callback: &None,
            doc: &doc,
        }
        .render()
        .unwrap();

        let cleanup_index = rendered
            .find("_module.freeWriter(message_writer);")
            .unwrap();
        let await_index = rendered
            .find("const awaitedHandle = await _module.asyncManager.pollAsync(")
            .unwrap();
        assert!(cleanup_index > await_index);
    }

    fn native_async_scalar_template<'a>(
        params: &'a [TsParam],
        doc: &'a Option<String>,
        return_route: &'a TsOutputRoute,
        cleanup_code: &'a str,
    ) -> AsyncFunctionTemplate<'a> {
        AsyncFunctionTemplate {
            name: "delayedAdd",
            params,
            return_type_str: "number",
            entry_ffi_name: "boltffi_method_class_counter_delayed_add",
            poll_sync_ffi_name: "boltffi_async_method_class_counter_delayed_add_poll_sync",
            poll_ffi_name: "boltffi_async_method_class_counter_delayed_add_poll",
            native_async: true,
            complete_ffi_name: "boltffi_async_method_class_counter_delayed_add_complete",
            panic_message_ffi_name: "boltffi_async_method_class_counter_delayed_add_panic_message",
            free_ffi_name: "boltffi_async_method_class_counter_delayed_add_free",
            call_args: "handle, amount",
            wrapper_code: "",
            cleanup_code,
            return_route,
            return_callback: &None,
            doc,
        }
    }

    // react-native track (docs/tracks/react-native.md), stage 2 -- generated-output regression
    // coverage for the native-async codegen mode, mirroring the rn_poc fixture's real
    // Counter::delayed_add(i32) -> i32 shape (@boltffi/runtime's own bun test drives the real
    // compiled equivalent of this exact output against a real dylib).
    #[test]
    fn native_async_scalar_return_dispatches_through_poll_async_native() {
        let doc: Option<String> = None;
        let return_route = TsOutputRoute::async_scalar(String::new());
        let rendered = native_async_scalar_template(&[], &doc, &return_route, "")
            .render()
            .unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsyncNative("));
        assert!(rendered.contains("boltffi_async_method_class_counter_delayed_add_poll"));
        assert!(rendered.contains("boltffi_async_method_class_counter_delayed_add_complete"));
        assert!(rendered.contains("boltffi_async_method_class_counter_delayed_add_free"));
        // The native protocol never references the wasm-only poll_sync/panic_message symbols or
        // completeAsync (a wasm-linear-memory-specific BoltFFIModule method).
        assert!(!rendered.contains("poll_sync"));
        assert!(!rendered.contains("panic_message"));
        assert!(!rendered.contains("_module.completeAsync"));
        assert!(rendered.contains("_module.checkStatus(_module.readStatusCode(statusBuf))"));
    }

    #[test]
    fn native_async_void_and_scalar_share_completeasyncs_status_taxonomy() {
        // Codex finding 5 (MEDIUM status semantics): the native_async void/scalar branches used to
        // hand-roll their own `statusCode !== 0` check inline, which always threw a generic
        // `Error` -- never `BoltFFICancelledError` for status 4, never recognizing status 3 at
        // all, unlike the packed route's `_module.completeAsync`/`checkStatus`. Both branches must
        // now delegate to the SAME `_module.checkStatus` helper `completeAsync` uses, so
        // cancellation/invalid-argument report identically regardless of return route.
        let doc: Option<String> = None;
        for return_route in [
            TsOutputRoute::void(),
            TsOutputRoute::async_scalar(String::new()),
        ] {
            let rendered = native_async_scalar_template(&[], &doc, &return_route, "")
                .render()
                .unwrap();
            assert!(
                rendered.contains("_module.checkStatus(_module.readStatusCode(statusBuf))"),
                "expected the shared status-check helper for route {return_route:?}, got:\n{rendered}"
            );
            assert!(
                !rendered.contains("if (statusCode !== 0)"),
                "must not hand-roll its own status check for route {return_route:?}, got:\n{rendered}"
            );
        }
    }

    #[test]
    fn native_async_scalar_return_with_param_cleanup_still_frees_after_await() {
        let doc: Option<String> = None;
        let return_route = TsOutputRoute::async_scalar(String::new());
        let rendered = native_async_scalar_template(&[], &doc, &return_route, "_module.freeWriter(w);")
            .render()
            .unwrap();

        let await_index = rendered
            .find("_module.asyncManager.pollAsyncNative(")
            .unwrap();
        let cleanup_index = rendered.find("_module.freeWriter(w);").unwrap();
        assert!(cleanup_index > await_index);
    }

    #[test]
    fn native_async_param_needing_wasm_wrapper_code_now_dispatches_through_pollasyncnative() {
        // Stage 4 (docs/tracks/react-native.md) closed the stage-2 gap this test used to pin:
        // `NativeBoltFFIModule` (`@boltffi/runtime`'s native.ts) now implements the full
        // `allocString`/`allocBytes`/`allocWriter`/... surface `BoltFFIModule` has, against its
        // own native memory arena -- so a param needing wrapper_code renders exactly like any
        // other native_async call, not a loud "unsupported" throw.
        let doc: Option<String> = None;
        let return_route = TsOutputRoute::async_scalar(String::new());
        let mut template = native_async_scalar_template(&[], &doc, &return_route, "");
        let param_alloc_statement = "const w = _module.allocString(the_message_param);";
        template.wrapper_code = param_alloc_statement;
        let rendered = template.render().unwrap();

        assert!(rendered.contains(param_alloc_statement));
        assert!(rendered.contains("pollAsyncNative("));
        assert!(!rendered.contains("does not yet support parameters requiring"));
    }

    #[test]
    fn native_async_buffer_encoded_return_now_decodes_through_the_shared_buf_descriptor_path() {
        // Stage 4 closed the stage-2 gap this test used to pin: `_module.allocBufDescriptor`/
        // `completeAsync`/`readerFromBuf`/`freeBuf` are backend-agnostic (`NativeBoltFFIModule`
        // implements the same surface as `BoltFFIModule`), so a packed/buffer-encoded async
        // return route now decodes for real instead of throwing.
        let doc: Option<String> = None;
        let return_route = TsOutputRoute::packed("ResponseCodec.decode(reader)".to_string());
        let rendered = native_async_scalar_template(&[], &doc, &return_route, "")
            .render()
            .unwrap();

        assert!(rendered.contains("_module.allocBufDescriptor()"));
        assert!(rendered.contains("_module.readerFromBuf(outPtr)"));
        assert!(rendered.contains("ResponseCodec.decode(reader)"));
        assert!(!rendered.contains("does not yet support buffer-encoded return routes"));
    }

    #[test]
    fn default_native_async_false_keeps_wasm_dispatch_unchanged() {
        let doc: Option<String> = None;
        let return_route = TsOutputRoute::async_scalar(String::new());
        let mut template = native_async_scalar_template(&[], &doc, &return_route, "");
        template.native_async = false;
        let rendered = template.render().unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsync("));
        assert!(!rendered.contains("pollAsyncNative"));
        assert!(rendered.contains("poll_sync"));
    }

    fn sync_callback_fixture() -> TsCallback {
        TsCallback {
            interface_name: "ValueHandler".to_string(),
            trait_name_snake: "value_handler".to_string(),
            create_handle_fn: "boltffi_create_value_handler_handle".to_string(),
            local_free_fn: "__boltffi_local_value_handler_free".to_string(),
            wrap_handle_fn: "wrapValueHandler".to_string(),
            proxy_class_name: "ValueHandlerProxy".to_string(),
            methods: vec![TsCallbackMethod {
                ts_name: "onValue".to_string(),
                import_name: "__boltffi_callback_value_handler_on_value".to_string(),
                proxy_export_name: "__boltffi_local_value_handler_on_value".to_string(),
                params: vec![TsCallbackParam {
                    name: "value".to_string(),
                    ts_type: "number".to_string(),
                    kind: TsCallbackParamKind::Primitive {
                        import_ts_type: "number".to_string(),
                        call_expr: "value".to_string(),
                    },
                }],
                proxy_params: vec![TsParam {
                    name: "value".to_string(),
                    ts_type: "number".to_string(),
                    input_route: TsInputRoute::Direct,
                }],
                return_type: Some("number".to_string()),
                import_return: TsCallbackImportReturn::Direct {
                    wasm_type: "number".to_string(),
                },
                proxy_return_route: TsOutputRoute::direct(String::new()),
                doc: None,
            }],
            async_methods: vec![],
            closure_fn_type: None,
            doc: None,
        }
    }

    fn async_callback_fixture() -> TsCallback {
        TsCallback {
            interface_name: "AsyncFetcher".to_string(),
            trait_name_snake: "async_fetcher".to_string(),
            create_handle_fn: "boltffi_create_async_fetcher_handle".to_string(),
            local_free_fn: "__boltffi_local_async_fetcher_free".to_string(),
            wrap_handle_fn: "wrapAsyncFetcher".to_string(),
            proxy_class_name: "AsyncFetcherProxy".to_string(),
            methods: vec![],
            async_methods: vec![TsAsyncCallbackMethod {
                ts_name: "fetch".to_string(),
                start_import_name: "__boltffi_callback_async_fetcher_fetch_start".to_string(),
                complete_export_name: "boltffi_callback_async_fetcher_fetch_complete".to_string(),
                params: vec![TsCallbackParam {
                    name: "key".to_string(),
                    ts_type: "number".to_string(),
                    kind: TsCallbackParamKind::Primitive {
                        import_ts_type: "number".to_string(),
                        call_expr: "key".to_string(),
                    },
                }],
                return_type: Some("number".to_string()),
                encode_expr: None,
                size_expr: None,
                direct_write_method: Some("writeI32".to_string()),
                direct_write_value_expr: Some("result".to_string()),
                direct_size: Some(4),
                doc: None,
            }],
            closure_fn_type: None,
            doc: None,
        }
    }

    #[test]
    fn callback_registry_emits_refcounted_lifecycle_contract() {
        let callback = sync_callback_fixture();
        let rendered = CallbackTemplate {
            callback: &callback,
        }
        .render()
        .unwrap();

        assert!(rendered.contains("const _value_handler_ref_counts = new Map<number, number>();"));
        assert!(
            rendered.contains("let _value_handler_next_id = _callback_handle_js_namespace_start;")
        );
        assert!(rendered.contains("const handle_key = _callback_handle_key(handle);"));
        assert!(rendered.contains("_value_handler_ref_counts.set(id, 1);"));
        assert!(rendered.contains("return _value_handler_retain(handle);"));
        assert!(rendered.contains("_value_handler_release(handle);"));
        assert!(rendered.contains("const impl = _value_handler_lookup(handle);"));
    }

    #[test]
    fn callback_registry_emits_invalid_handle_and_no_resurrection_guards() {
        let callback = sync_callback_fixture();
        let rendered = CallbackTemplate {
            callback: &callback,
        }
        .render()
        .unwrap();

        assert!(rendered.contains(
            "Cannot clone unknown callback handle ${handle_key} in ValueHandler registry"
        ));
        assert!(rendered.contains(
            "Cannot free unknown callback handle ${handle_key} in ValueHandler registry"
        ));
        assert!(
            rendered.contains("Callback handle ${handle_key} not found in ValueHandler registry")
        );
        assert!(rendered.contains("if (currentCount === 1) {"));
        assert!(rendered.contains("_value_handler_ref_counts.delete(handle_key);"));
        assert!(rendered.contains("_value_handler_registry.delete(handle_key);"));
        assert!(rendered.contains("return handle_key;"));
    }

    #[test]
    fn async_callback_invalid_handle_is_reported_through_completion() {
        let callback = async_callback_fixture();
        let rendered = CallbackTemplate {
            callback: &callback,
        }
        .render()
        .unwrap();

        assert!(rendered.contains("let impl: AsyncFetcher;"));
        assert!(rendered.contains("impl = _async_fetcher_lookup(handle);"));
        assert!(rendered.contains("completeError(err);"));
        assert!(rendered.contains("return;"));
    }

    #[test]
    fn snapshot_class_with_constructor_and_methods() {
        let class = TsClass {
            class_name: "Counter".to_string(),
            ffi_free: "boltffi_counter_free".to_string(),
            constructors: vec![TsClassConstructor {
                ts_name: "new".to_string(),
                ffi_name: "boltffi_counter_new".to_string(),
                is_default: true,
                params: vec![],
                returns_nullable_handle: false,
                throws: false,
                doc: Some("Creates a counter".to_string()),
            }],
            methods: vec![
                TsClassMethod {
                    ts_name: "increment".to_string(),
                    ffi_name: "boltffi_counter_increment".to_string(),
                    is_static: false,
                    params: vec![TsParam {
                        name: "delta".to_string(),
                        ts_type: "number".to_string(),
                        input_route: TsInputRoute::Direct,
                    }],
                    return_type: Some("number".to_string()),
                    return_handle: None,
                    return_callback: None,
                    mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                        return_route: TsOutputRoute::direct(String::new()),
                    }),
                    throws: false,
                    doc: None,
                },
                TsClassMethod {
                    ts_name: "nextValue".to_string(),
                    ffi_name: "boltffi_counter_next_value".to_string(),
                    is_static: false,
                    params: vec![],
                    return_type: Some("number".to_string()),
                    return_handle: None,
                    return_callback: None,
                    mode: TsClassMethodMode::Async(TsClassAsyncMethod {
                        poll_sync_ffi_name: "boltffi_counter_next_value_poll_sync".to_string(),
                        poll_ffi_name: "boltffi_counter_next_value_poll".to_string(),
                        complete_ffi_name: "boltffi_counter_next_value_complete".to_string(),
                        panic_message_ffi_name: "boltffi_counter_next_value_panic_message"
                            .to_string(),
                        cancel_ffi_name: "boltffi_counter_next_value_cancel".to_string(),
                        free_ffi_name: "boltffi_counter_next_value_free".to_string(),
                        native_async: false,
                        return_route: TsOutputRoute::packed("reader.readI32()".to_string()),
                    }),
                    throws: false,
                    doc: None,
                },
            ],
            doc: Some("A counter class".to_string()),
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn class_sync_struct_return_slot_route_pins_the_unified_alloc_scratch_call() {
        // Codex finding 4 (MEDIUM wasm byte-identity): stage 4 (8de23d4c) replaced this route's
        // hardcoded `_module.exports.boltffi_wasm_alloc`/`boltffi_wasm_free` calls with the
        // backend-agnostic `_module.allocScratch`/`freeScratch` -- changing wasm-mode's generated
        // TEXT (though not its runtime behaviour: `BoltFFIModule.allocScratch`/`freeScratch` in
        // module.ts are a 1:1 delegate to the exact same two wasm exports). The merge commit
        // claimed wasm output "stays byte-identical when the flag is off" -- true of every OTHER
        // route this stage touched, but not of this one. No codegen test caught the discrepancy
        // because no test exercised the sync struct-return-slot route at all.
        //
        // Deliberate resolution (not a bug fix): keep the unified call. Gating this one route on
        // a backend flag would special-case the ONE route stage 4 left inconsistent, reintroducing
        // exactly the per-backend branching the rest of this design eliminates, for a difference
        // that is textual only -- module.ts's `allocScratch`/`freeScratch` calls the identical two
        // wasm exports, in the identical order, with the identical arguments. This test pins that
        // reality (rather than the stale "byte-identical" claim) so it can't silently drift again.
        let class = TsClass {
            class_name: "Grid".to_string(),
            ffi_free: "boltffi_grid_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "cells".to_string(),
                ffi_name: "boltffi_grid_cells".to_string(),
                is_static: false,
                params: vec![],
                return_type: Some("Int32Array".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::struct_return_slot(
                        16,
                        "_module.takeSlotI32Array()".to_string(),
                    ),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };

        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.allocScratch(16)"));
        assert!(rendered.contains("_module.freeScratch(__outPtr, 16)"));
        assert!(!rendered.contains("boltffi_wasm_alloc"));
        assert!(!rendered.contains("boltffi_wasm_free"));
    }

    #[test]
    fn class_nullable_constructor_preserves_null_contract() {
        let class = TsClass {
            class_name: "Session".to_string(),
            ffi_free: "boltffi_session_free".to_string(),
            constructors: vec![TsClassConstructor {
                ts_name: "open".to_string(),
                ffi_name: "boltffi_session_open".to_string(),
                is_default: false,
                params: vec![TsParam {
                    name: "path".to_string(),
                    ts_type: "string".to_string(),
                    input_route: TsInputRoute::String,
                }],
                returns_nullable_handle: true,
                throws: false,
                doc: None,
            }],
            methods: vec![],
            doc: None,
        };

        let rendered = ClassTemplate { cls: &class }.render().unwrap();
        assert!(rendered.contains("static open(path: string): Session | null {"));
        assert!(rendered.contains("if (handle === 0) {\n        return null;\n      }"));
    }

    #[test]
    fn class_fallible_constructor_throws_instead_of_returning_null() {
        let class = TsClass {
            class_name: "Inventory".to_string(),
            ffi_free: "boltffi_inventory_free".to_string(),
            constructors: vec![TsClassConstructor {
                ts_name: "tryNew".to_string(),
                ffi_name: "boltffi_inventory_try_new".to_string(),
                is_default: false,
                params: vec![TsParam {
                    name: "capacity".to_string(),
                    ts_type: "number".to_string(),
                    input_route: TsInputRoute::Direct,
                }],
                returns_nullable_handle: true,
                throws: true,
                doc: None,
            }],
            methods: vec![],
            doc: None,
        };

        let rendered = ClassTemplate { cls: &class }.render().unwrap();
        assert!(
            rendered.contains("static tryNew(capacity: number): Inventory {"),
            "a throwing constructor must not advertise `| null` in its return type; got:\n{rendered}"
        );
        assert!(
            rendered.contains(
                "if (handle === 0) {\n      throw new Error(_module.takeLastErrorMessage());\n    }"
            ),
            "a fallible constructor must throw the real last-error message instead of returning null; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("return null;"),
            "a throwing constructor must never silently return null; got:\n{rendered}"
        );
    }

    #[test]
    fn class_async_return_frees_handles_on_decode_failures() {
        let class = TsClass {
            class_name: "Counter".to_string(),
            ffi_free: "boltffi_counter_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "nextValue".to_string(),
                ffi_name: "boltffi_counter_next_value".to_string(),
                is_static: false,
                params: vec![],
                return_type: Some("number".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Async(TsClassAsyncMethod {
                    poll_sync_ffi_name: "boltffi_counter_next_value_poll_sync".to_string(),
                    poll_ffi_name: "boltffi_counter_next_value_poll".to_string(),
                    complete_ffi_name: "boltffi_counter_next_value_complete".to_string(),
                    panic_message_ffi_name: "boltffi_counter_next_value_panic_message".to_string(),
                    cancel_ffi_name: "boltffi_counter_next_value_cancel".to_string(),
                    free_ffi_name: "boltffi_counter_next_value_free".to_string(),
                    native_async: false,
                    return_route: TsOutputRoute::packed("reader.readI32()".to_string()),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };

        let rendered = ClassTemplate { cls: &class }.render().unwrap();
        assert!(rendered.contains("let completeCompleted = false;"));
        assert!(rendered.contains("_module.freeBuf(outPtr);"));
        assert!(rendered.contains("_module.freeBufDescriptor(outPtr);"));
        assert!(
            rendered
                .contains("(_exports.boltffi_counter_next_value_free as Function)(awaitedHandle);")
        );
    }

    #[test]
    fn class_async_param_cleanup_runs_after_await() {
        let class = TsClass {
            class_name: "Database".to_string(),
            ffi_free: "boltffi_database_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "query".to_string(),
                ffi_name: "boltffi_database_query".to_string(),
                is_static: false,
                params: vec![TsParam {
                    name: "sql".to_string(),
                    ts_type: "string".to_string(),
                    input_route: TsInputRoute::String,
                }],
                return_type: Some("QueryResult".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Async(TsClassAsyncMethod {
                    poll_sync_ffi_name: "boltffi_database_query_poll_sync".to_string(),
                    poll_ffi_name: "boltffi_database_query_poll".to_string(),
                    complete_ffi_name: "boltffi_database_query_complete".to_string(),
                    panic_message_ffi_name: "boltffi_database_query_panic_message".to_string(),
                    cancel_ffi_name: "boltffi_database_query_cancel".to_string(),
                    free_ffi_name: "boltffi_database_query_free".to_string(),
                    native_async: false,
                    return_route: TsOutputRoute::packed(
                        "QueryResultCodec.decode(reader)".to_string(),
                    ),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };

        let rendered = ClassTemplate { cls: &class }.render().unwrap();
        let cleanup_index = rendered.find("_module.freeAlloc(sql_alloc);").unwrap();
        let await_index = rendered
            .find("const awaitedHandle = await _module.asyncManager.pollAsync(")
            .unwrap();
        assert!(cleanup_index > await_index);
    }

    #[test]
    fn snapshot_wasm_exports() {
        let params = vec![
            TsWasmParam {
                name: "a".to_string(),
                wasm_type: "number".to_string(),
            },
            TsWasmParam {
                name: "b".to_string(),
                wasm_type: "number".to_string(),
            },
        ];
        let imports = vec![TsWasmImportView {
            ffi_name: "boltffi_add",
            params: &params,
            return_wasm_type_str: "number",
        }];
        let template = WasmExportsTemplate {
            wasm_imports: &imports,
        };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn wasm_exports_renders_encoded_return_with_out_param() {
        let params = vec![
            TsWasmParam {
                name: "out".to_string(),
                wasm_type: "number".to_string(),
            },
            TsWasmParam {
                name: "payload".to_string(),
                wasm_type: "number".to_string(),
            },
        ];
        let imports = vec![TsWasmImportView {
            ffi_name: "boltffi_echo_payload",
            params: &params,
            return_wasm_type_str: "void",
        }];
        let template = WasmExportsTemplate {
            wasm_imports: &imports,
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("boltffi_echo_payload(out: number, payload: number): void;"));
    }

    #[test]
    fn snapshot_class_with_static_method() {
        let class = TsClass {
            class_name: "MathUtils".to_string(),
            ffi_free: "boltffi_math_utils_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "add".to_string(),
                ffi_name: "boltffi_math_utils_add".to_string(),
                is_static: true,
                params: vec![
                    TsParam {
                        name: "a".to_string(),
                        ts_type: "number".to_string(),
                        input_route: TsInputRoute::Direct,
                    },
                    TsParam {
                        name: "b".to_string(),
                        ts_type: "number".to_string(),
                        input_route: TsInputRoute::Direct,
                    },
                ],
                return_type: Some("number".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::direct(String::new()),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_class_with_void_method() {
        let class = TsClass {
            class_name: "Logger".to_string(),
            ffi_free: "boltffi_logger_free".to_string(),
            constructors: vec![TsClassConstructor {
                ts_name: "new".to_string(),
                ffi_name: "boltffi_logger_new".to_string(),
                is_default: true,
                params: vec![],
                returns_nullable_handle: false,
                throws: false,
                doc: None,
            }],
            methods: vec![TsClassMethod {
                ts_name: "log".to_string(),
                ffi_name: "boltffi_logger_log".to_string(),
                is_static: false,
                params: vec![TsParam {
                    name: "message".to_string(),
                    ts_type: "string".to_string(),
                    input_route: TsInputRoute::String,
                }],
                return_type: None,
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::void(),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_class_with_handle_return() {
        let class = TsClass {
            class_name: "Factory".to_string(),
            ffi_free: "boltffi_factory_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "createChild".to_string(),
                ffi_name: "boltffi_factory_create_child".to_string(),
                is_static: false,
                params: vec![],
                return_type: Some("Child".to_string()),
                return_handle: Some(TsHandleReturn {
                    class_name: "Child".to_string(),
                    nullable: false,
                }),
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::direct(String::new()),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_class_with_nullable_handle_return() {
        let class = TsClass {
            class_name: "Cache".to_string(),
            ffi_free: "boltffi_cache_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "get".to_string(),
                ffi_name: "boltffi_cache_get".to_string(),
                is_static: false,
                params: vec![TsParam {
                    name: "key".to_string(),
                    ts_type: "string".to_string(),
                    input_route: TsInputRoute::String,
                }],
                return_type: Some("Entry | null".to_string()),
                return_handle: Some(TsHandleReturn {
                    class_name: "Entry".to_string(),
                    nullable: true,
                }),
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::direct(String::new()),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_class_with_encoded_param() {
        let class = TsClass {
            class_name: "Renderer".to_string(),
            ffi_free: "boltffi_renderer_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "draw".to_string(),
                ffi_name: "boltffi_renderer_draw".to_string(),
                is_static: false,
                params: vec![TsParam {
                    name: "point".to_string(),
                    ts_type: "Point".to_string(),
                    input_route: TsInputRoute::CodecEncoded {
                        codec_name: "Point".to_string(),
                    },
                }],
                return_type: None,
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::void(),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn snapshot_class_async_with_encoded_return() {
        let class = TsClass {
            class_name: "Database".to_string(),
            ffi_free: "boltffi_database_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "query".to_string(),
                ffi_name: "boltffi_database_query".to_string(),
                is_static: false,
                params: vec![TsParam {
                    name: "sql".to_string(),
                    ts_type: "string".to_string(),
                    input_route: TsInputRoute::String,
                }],
                return_type: Some("QueryResult".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Async(TsClassAsyncMethod {
                    poll_sync_ffi_name: "boltffi_database_query_poll_sync".to_string(),
                    poll_ffi_name: "boltffi_database_query_poll".to_string(),
                    complete_ffi_name: "boltffi_database_query_complete".to_string(),
                    panic_message_ffi_name: "boltffi_database_query_panic_message".to_string(),
                    cancel_ffi_name: "boltffi_database_query_cancel".to_string(),
                    free_ffi_name: "boltffi_database_query_free".to_string(),
                    native_async: false,
                    return_route: TsOutputRoute::packed(
                        "QueryResultCodec.decode(reader)".to_string(),
                    ),
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        insta::assert_snapshot!(template.render().unwrap());
    }

    #[test]
    fn class_fallible_sync_method_returning_handle_throws_instead_of_returning_null() {
        let class = TsClass {
            class_name: "Map".to_string(),
            ffi_free: "boltffi_map_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "tryClone".to_string(),
                ffi_name: "boltffi_map_try_clone".to_string(),
                is_static: false,
                params: vec![],
                return_type: Some("Map".to_string()),
                return_handle: Some(TsHandleReturn {
                    class_name: "Map".to_string(),
                    nullable: true,
                }),
                return_callback: None,
                mode: TsClassMethodMode::Sync(TsClassSyncMethod {
                    return_route: TsOutputRoute::direct(String::new()),
                }),
                throws: true,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        let rendered = template.render().unwrap();
        assert!(
            rendered.contains("tryClone(): Map {"),
            "a throwing method must not advertise `| null` in its return type; got:\n{rendered}"
        );
        assert!(
            rendered.contains(
                "if (result === 0) {\n      throw new Error(_module.takeLastErrorMessage());\n    }"
            ),
            "a fallible handle-returning method must throw the real last-error message instead of returning null; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("return null;"),
            "a throwing method must never silently return null; got:\n{rendered}"
        );
    }

    #[test]
    fn class_fallible_async_method_returning_handle_throws_instead_of_returning_null() {
        let class = TsClass {
            class_name: "Map".to_string(),
            ffi_free: "boltffi_map_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "tryCloneAsync".to_string(),
                ffi_name: "boltffi_map_try_clone_async".to_string(),
                is_static: false,
                params: vec![],
                return_type: Some("Map".to_string()),
                return_handle: Some(TsHandleReturn {
                    class_name: "Map".to_string(),
                    nullable: true,
                }),
                return_callback: None,
                mode: TsClassMethodMode::Async(TsClassAsyncMethod {
                    poll_sync_ffi_name: "boltffi_map_try_clone_async_poll_sync".to_string(),
                    poll_ffi_name: "boltffi_map_try_clone_async_poll".to_string(),
                    complete_ffi_name: "boltffi_map_try_clone_async_complete".to_string(),
                    panic_message_ffi_name: "boltffi_map_try_clone_async_panic_message"
                        .to_string(),
                    cancel_ffi_name: "boltffi_map_try_clone_async_cancel".to_string(),
                    free_ffi_name: "boltffi_map_try_clone_async_free".to_string(),
                    native_async: false,
                    return_route: TsOutputRoute::packed("reader.readU64()".to_string()),
                }),
                throws: true,
                doc: None,
            }],
            doc: None,
        };
        let template = ClassTemplate { cls: &class };
        let rendered = template.render().unwrap();
        assert!(
            rendered.contains("async tryCloneAsync(): Promise<Map> {"),
            "a throwing async method must not advertise `| null` in its return type; got:\n{rendered}"
        );
        assert!(
            rendered.contains("throw new Error(_module.takeLastErrorMessage());"),
            "a fallible async handle-returning method must throw the real last-error message instead of returning null; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("return null;"),
            "a throwing async method must never silently return null; got:\n{rendered}"
        );
    }

    // react-native track (docs/tracks/react-native.md), stage 3 -- class methods are the
    // remaining native_async gap async_function.txt already closed for top-level functions
    // (ParseClient IS a class, so class-method dispatch must support native_async too, not just
    // free functions). Mirrors the `native_async_scalar_template` free-function coverage above,
    // one test per return-route branch class.txt's new native_async arm can take.

    fn class_with_native_async_method(
        native_async: bool,
        return_route: TsOutputRoute,
        params: Vec<TsParam>,
        cleanup_code_param: Option<TsParam>,
    ) -> TsClass {
        let mut method_params = params;
        if let Some(p) = cleanup_code_param {
            method_params.push(p);
        }
        TsClass {
            class_name: "Counter".to_string(),
            ffi_free: "boltffi_counter_free".to_string(),
            constructors: vec![],
            methods: vec![TsClassMethod {
                ts_name: "delayedAdd".to_string(),
                ffi_name: "boltffi_method_class_counter_delayed_add".to_string(),
                is_static: false,
                params: method_params,
                return_type: Some("number".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsClassMethodMode::Async(TsClassAsyncMethod {
                    poll_sync_ffi_name: "boltffi_async_method_class_counter_delayed_add_poll_sync"
                        .to_string(),
                    poll_ffi_name: "boltffi_async_method_class_counter_delayed_add_poll"
                        .to_string(),
                    complete_ffi_name: "boltffi_async_method_class_counter_delayed_add_complete"
                        .to_string(),
                    panic_message_ffi_name:
                        "boltffi_async_method_class_counter_delayed_add_panic_message"
                            .to_string(),
                    cancel_ffi_name: "boltffi_async_method_class_counter_delayed_add_cancel"
                        .to_string(),
                    free_ffi_name: "boltffi_async_method_class_counter_delayed_add_free"
                        .to_string(),
                    native_async,
                    return_route,
                }),
                throws: false,
                doc: None,
            }],
            doc: None,
        }
    }

    #[test]
    fn class_native_async_scalar_return_dispatches_through_poll_async_native() {
        let class = class_with_native_async_method(
            true,
            TsOutputRoute::async_scalar(String::new()),
            vec![],
            None,
        );
        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsyncNative("));
        assert!(rendered.contains("boltffi_async_method_class_counter_delayed_add_poll"));
        assert!(rendered.contains("boltffi_async_method_class_counter_delayed_add_complete"));
        assert!(rendered.contains("boltffi_async_method_class_counter_delayed_add_free"));
        assert!(!rendered.contains("poll_sync"));
        assert!(!rendered.contains("panic_message"));
        assert!(!rendered.contains("_module.completeAsync"));
        assert!(rendered.contains("_module.checkStatus(_module.readStatusCode(statusBuf))"));
    }

    #[test]
    fn class_native_async_void_and_scalar_share_completeasyncs_status_taxonomy() {
        // Same Codex finding-5 fix as the free-function test above, for class methods.
        for return_route in [
            TsOutputRoute::void(),
            TsOutputRoute::async_scalar(String::new()),
        ] {
            let class =
                class_with_native_async_method(true, return_route.clone(), vec![], None);
            let rendered = ClassTemplate { cls: &class }.render().unwrap();
            assert!(
                rendered.contains("_module.checkStatus(_module.readStatusCode(statusBuf))"),
                "expected the shared status-check helper for route {return_route:?}, got:\n{rendered}"
            );
            assert!(!rendered.contains("if (statusCode !== 0)"));
        }
    }

    #[test]
    fn class_native_async_void_return_completes_and_frees() {
        let class =
            class_with_native_async_method(true, TsOutputRoute::void(), vec![], None);
        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsyncNative("));
        assert!(
            rendered.contains(
                "(_exports.boltffi_async_method_class_counter_delayed_add_complete as Function)(awaitedHandle, statusBuf);"
            )
        );
        assert!(
            rendered.contains(
                "(_exports.boltffi_async_method_class_counter_delayed_add_free as Function)(awaitedHandle);"
            )
        );
    }

    #[test]
    fn class_native_async_buffer_encoded_return_now_decodes_through_the_shared_buf_descriptor_path() {
        // Stage 4 closed the gap this test used to pin (see the free-function equivalent's doc).
        let class = class_with_native_async_method(
            true,
            TsOutputRoute::packed("reader.readI32()".to_string()),
            vec![],
            None,
        );
        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.allocBufDescriptor()"));
        assert!(rendered.contains("_module.readerFromBuf(outPtr)"));
        assert!(rendered.contains("reader.readI32()"));
        assert!(!rendered.contains("does not yet support buffer-encoded return routes"));
    }

    #[test]
    fn class_native_async_wrapper_code_param_now_dispatches_through_pollasyncnative() {
        // Stage 4 closed the gap this test used to pin (see the free-function equivalent's doc):
        // `NativeBoltFFIModule.allocString` now exists, so a String param renders and dispatches
        // normally instead of hitting the (now permanently-false) wrapper guard.
        let class = class_with_native_async_method(
            true,
            TsOutputRoute::async_scalar(String::new()),
            vec![TsParam {
                name: "sql".to_string(),
                ts_type: "string".to_string(),
                input_route: TsInputRoute::String,
            }],
            None,
        );
        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.allocString(sql)"));
        assert!(rendered.contains("pollAsyncNative("));
        assert!(!rendered.contains("does not yet support parameters requiring"));
    }

    #[test]
    fn class_native_async_method_with_cleanup_needing_param_now_dispatches_through_pollasyncnative() {
        // Unlike a free function's `AsyncFunctionTemplate` (where `wrapper_code`/`cleanup_code`
        // are independent raw-string template fields a test can set separately), a class
        // method's `wrapper_code()`/`cleanup_code()` are both DERIVED from the same `TsParam`
        // list: every `TsInputRoute` that produces cleanup code (`String`, `Bytes`,
        // `PrimitiveBuffer`, ...) also produces wrapper code for that same param, exercising the
        // `cleanup_code().is_empty() == false` Async arm specifically (mirrored into
        // class.txt/value_type_companion.txt for structural symmetry with the wasm arm).
        let class = class_with_native_async_method(
            true,
            TsOutputRoute::async_scalar(String::new()),
            vec![],
            Some(TsParam {
                name: "sql".to_string(),
                ts_type: "string".to_string(),
                input_route: TsInputRoute::String,
            }),
        );
        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.allocString(sql)"));
        assert!(rendered.contains("pollAsyncNative("));
        assert!(rendered.contains("_module.freeAlloc(sql_alloc);"));
    }

    #[test]
    fn class_default_native_async_false_keeps_wasm_dispatch_unchanged() {
        let class = class_with_native_async_method(
            false,
            TsOutputRoute::async_scalar(String::new()),
            vec![],
            None,
        );
        let rendered = ClassTemplate { cls: &class }.render().unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsync("));
        assert!(!rendered.contains("pollAsyncNative"));
    }

    fn record_with_native_async_method(
        native_async: bool,
        return_route: TsOutputRoute,
    ) -> (String, Vec<TsValueTypeMethod>) {
        (
            "Point".to_string(),
            vec![TsValueTypeMethod {
                ts_name: "delayedScale".to_string(),
                ffi_name: "boltffi_method_value_type_point_delayed_scale".to_string(),
                is_static: false,
                params: vec![],
                return_type: Some("number".to_string()),
                return_handle: None,
                return_callback: None,
                mode: TsValueTypeMethodMode::Async(TsValueTypeAsyncMethod {
                    poll_sync_ffi_name:
                        "boltffi_method_value_type_point_delayed_scale_poll_sync".to_string(),
                    poll_ffi_name: "boltffi_method_value_type_point_delayed_scale_poll"
                        .to_string(),
                    complete_ffi_name: "boltffi_method_value_type_point_delayed_scale_complete"
                        .to_string(),
                    panic_message_ffi_name:
                        "boltffi_method_value_type_point_delayed_scale_panic_message".to_string(),
                    cancel_ffi_name: "boltffi_method_value_type_point_delayed_scale_cancel"
                        .to_string(),
                    free_ffi_name: "boltffi_method_value_type_point_delayed_scale_free"
                        .to_string(),
                    native_async,
                    return_route,
                }),
                doc: None,
            }],
        )
    }

    #[test]
    fn value_type_native_async_scalar_return_dispatches_through_poll_async_native() {
        let (name, methods) =
            record_with_native_async_method(true, TsOutputRoute::async_scalar(String::new()));
        let rendered = ValueTypeCompanionTemplate {
            name: &name,
            constructors: &[],
            methods: &methods,
        }
        .render()
        .unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsyncNative("));
        assert!(rendered.contains("boltffi_method_value_type_point_delayed_scale_poll"));
        assert!(rendered.contains("boltffi_method_value_type_point_delayed_scale_complete"));
        assert!(rendered.contains("boltffi_method_value_type_point_delayed_scale_free"));
        assert!(!rendered.contains("poll_sync"));
        assert!(!rendered.contains("_module.completeAsync"));
    }

    #[test]
    fn value_type_native_async_void_and_scalar_share_completeasyncs_status_taxonomy() {
        // Same Codex finding-5 fix as the free-function/class tests above, for value-type
        // companion methods.
        for return_route in [
            TsOutputRoute::void(),
            TsOutputRoute::async_scalar(String::new()),
        ] {
            let (name, methods) = record_with_native_async_method(true, return_route.clone());
            let rendered = ValueTypeCompanionTemplate {
                name: &name,
                constructors: &[],
                methods: &methods,
            }
            .render()
            .unwrap();
            assert!(
                rendered.contains("_module.checkStatus(_module.readStatusCode(statusBuf))"),
                "expected the shared status-check helper for route {return_route:?}, got:\n{rendered}"
            );
            assert!(!rendered.contains("if (statusCode !== 0)"));
        }
    }

    #[test]
    fn value_type_native_async_void_return_completes_and_frees() {
        let (name, methods) = record_with_native_async_method(true, TsOutputRoute::void());
        let rendered = ValueTypeCompanionTemplate {
            name: &name,
            constructors: &[],
            methods: &methods,
        }
        .render()
        .unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsyncNative("));
        assert!(
            rendered.contains(
                "(_exports.boltffi_method_value_type_point_delayed_scale_free as Function)(awaitedHandle);"
            )
        );
    }

    #[test]
    fn value_type_default_native_async_false_keeps_wasm_dispatch_unchanged() {
        let (name, methods) =
            record_with_native_async_method(false, TsOutputRoute::async_scalar(String::new()));
        let rendered = ValueTypeCompanionTemplate {
            name: &name,
            constructors: &[],
            methods: &methods,
        }
        .render()
        .unwrap();

        assert!(rendered.contains("_module.asyncManager.pollAsync("));
        assert!(!rendered.contains("pollAsyncNative"));
    }
}
