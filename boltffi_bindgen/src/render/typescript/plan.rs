use crate::ir::abi::SpanLengthUnit;
use crate::ir::ops::{ReadSeq, WriteSeq};
use crate::ir::plan::AbiType;
use crate::render::typescript::emit;

#[derive(Debug, Clone)]
pub struct TsModule {
    pub module_name: String,
    pub abi_version: u32,
    pub records: Vec<TsRecord>,
    pub enums: Vec<TsEnum>,
    pub error_exceptions: Vec<TsErrorException>,
    pub functions: Vec<TsFunction>,
    pub async_functions: Vec<TsAsyncFunction>,
    pub classes: Vec<TsClass>,
    pub callbacks: Vec<TsCallback>,
    pub wasm_imports: Vec<TsWasmImport>,
}

#[derive(Debug, Clone)]
pub struct TsErrorException {
    pub type_name: String,
    pub class_name: String,
    pub is_c_style_enum: bool,
}

#[derive(Debug, Clone)]
pub struct TsAsyncFunction {
    pub name: String,
    pub entry_ffi_name: String,
    pub poll_sync_ffi_name: String,
    /// The native-protocol registration symbol (`{base}_poll`, distinct from
    /// `poll_sync_ffi_name`'s wasm-only `{base}_poll_sync`) — only referenced by the generated
    /// output when `native_async` is set.
    pub poll_ffi_name: String,
    pub complete_ffi_name: String,
    pub panic_message_ffi_name: String,
    pub cancel_ffi_name: String,
    pub free_ffi_name: String,
    /// Selects the native-async dispatch mode (`@boltffi/runtime`'s `pollAsyncNative`) over the
    /// default wasm `pollAsync`/`poll_sync` convention in `async_function.txt`
    /// (docs/tracks/react-native.md, parse-core-sdks repo, stage 2). Defaults to `false`
    /// (`TypeScriptExperimental::default()`), so every existing caller's output is unchanged.
    pub native_async: bool,
    pub params: Vec<TsParam>,
    pub return_type: Option<String>,
    pub return_route: TsOutputRoute,
    pub return_callback: Option<TsCallbackHandleReturn>,
    pub throws: bool,
    pub err_type: String,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TsClass {
    pub class_name: String,
    pub ffi_free: String,
    pub constructors: Vec<TsClassConstructor>,
    pub methods: Vec<TsClassMethod>,
    pub doc: Option<String>,
}

impl TsClass {
    pub fn has_default_constructor(&self) -> bool {
        self.constructors
            .iter()
            .any(|constructor| constructor.is_default)
    }

    pub fn default_constructor(&self) -> Option<&TsClassConstructor> {
        self.constructors
            .iter()
            .find(|constructor| constructor.is_default)
    }

    pub fn named_constructors(&self) -> Vec<&TsClassConstructor> {
        self.constructors
            .iter()
            .filter(|constructor| !constructor.is_default)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct TsClassConstructor {
    pub ts_name: String,
    pub ffi_name: String,
    pub is_default: bool,
    pub params: Vec<TsParam>,
    pub returns_nullable_handle: bool,
    /// True when a `null` handle means the constructor failed (`Result<Self, Error>`),
    /// as opposed to a genuinely optional constructor (`Option<Self>`). A throwing
    /// constructor never returns `null` — it throws with the real failure message
    /// instead, via `BoltFFIModule.takeLastErrorMessage()`.
    pub throws: bool,
    pub doc: Option<String>,
}

impl TsClassConstructor {
    pub fn wrapper_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::wrapper_code)
            .collect::<Vec<_>>()
            .join("\n    ")
    }

    pub fn cleanup_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::cleanup_code)
            .collect::<Vec<_>>()
            .join("\n      ")
    }

    pub fn ffi_call_args(&self) -> String {
        flatten_ffi_args(&self.params).join(", ")
    }
}

#[derive(Debug, Clone)]
pub struct TsClassMethod {
    pub ts_name: String,
    pub ffi_name: String,
    pub is_static: bool,
    pub params: Vec<TsParam>,
    pub return_type: Option<String>,
    pub return_handle: Option<TsHandleReturn>,
    pub return_callback: Option<TsCallbackHandleReturn>,
    pub mode: TsClassMethodMode,
    /// True when a `null` handle return means the call failed (`Result<Handle, Error>`),
    /// as opposed to a genuinely optional return (`Option<Handle>`). Mirrors
    /// `TsClassConstructor::throws` — see its doc for the failure-vs-absence distinction.
    pub throws: bool,
    pub doc: Option<String>,
}

impl TsClassMethod {
    pub fn wrapper_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::wrapper_code)
            .collect::<Vec<_>>()
            .join("\n    ")
    }

    pub fn cleanup_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::cleanup_code)
            .collect::<Vec<_>>()
            .join("\n      ")
    }

    pub fn ffi_call_args(&self) -> String {
        let mut call_args = Vec::new();
        if !self.is_static {
            call_args.push("this._handle".to_string());
        }
        call_args.extend(flatten_ffi_args(&self.params));
        call_args.join(", ")
    }

    pub fn ffi_call_args_with_out(&self) -> String {
        let call_args = self.ffi_call_args();
        if call_args.is_empty() {
            "__outPtr".to_string()
        } else {
            format!("__outPtr, {call_args}")
        }
    }

    pub fn is_async(&self) -> bool {
        matches!(self.mode, TsClassMethodMode::Async(_))
    }
}

#[derive(Debug, Clone)]
pub struct TsHandleReturn {
    pub class_name: String,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
pub struct TsCallbackHandleReturn {
    pub interface_name: String,
    pub wrap_fn: String,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
pub enum TsClassMethodMode {
    Sync(TsClassSyncMethod),
    Async(TsClassAsyncMethod),
}

#[derive(Debug, Clone)]
pub struct TsClassSyncMethod {
    pub return_route: TsOutputRoute,
}

#[derive(Debug, Clone)]
pub struct TsClassAsyncMethod {
    pub poll_sync_ffi_name: String,
    pub complete_ffi_name: String,
    pub panic_message_ffi_name: String,
    pub cancel_ffi_name: String,
    pub free_ffi_name: String,
    pub return_route: TsOutputRoute,
}

#[derive(Debug, Clone)]
pub struct TsCallback {
    pub interface_name: String,
    pub trait_name_snake: String,
    pub create_handle_fn: String,
    pub local_free_fn: String,
    pub wrap_handle_fn: String,
    pub proxy_class_name: String,
    pub methods: Vec<TsCallbackMethod>,
    pub async_methods: Vec<TsAsyncCallbackMethod>,
    pub closure_fn_type: Option<String>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TsCallbackMethod {
    pub ts_name: String,
    pub import_name: String,
    pub params: Vec<TsCallbackParam>,
    pub proxy_export_name: String,
    pub proxy_params: Vec<TsParam>,
    pub return_type: Option<String>,
    pub import_return: TsCallbackImportReturn,
    pub proxy_return_route: TsOutputRoute,
    pub doc: Option<String>,
}

impl TsCallbackMethod {
    pub fn proxy_wrapper_code(&self) -> String {
        self.proxy_params
            .iter()
            .filter_map(TsParam::wrapper_code)
            .collect::<Vec<_>>()
            .join("\n    ")
    }

    pub fn proxy_cleanup_code(&self) -> String {
        self.proxy_params
            .iter()
            .filter_map(TsParam::cleanup_code)
            .collect::<Vec<_>>()
            .join("\n      ")
    }

    pub fn proxy_call_args(&self) -> String {
        flatten_ffi_args(&self.proxy_params).join(", ")
    }
}

#[derive(Debug, Clone)]
pub enum TsCallbackImportReturn {
    Void,
    Direct { wasm_type: String },
    Encoded(TsEncodedCallbackReturn),
    PackedUtf8,
}

#[derive(Debug, Clone)]
pub struct TsEncodedCallbackReturn {
    pub encode_expr: String,
    pub size_expr: String,
}

#[derive(Debug, Clone)]
pub struct TsCallbackParam {
    pub name: String,
    pub ts_type: String,
    pub kind: TsCallbackParamKind,
}

#[derive(Debug, Clone)]
pub enum TsCallbackParamKind {
    Primitive {
        import_ts_type: String,
        call_expr: String,
    },
    WireEncoded {
        decode_expr: String,
    },
}

#[derive(Debug, Clone)]
pub struct TsAsyncCallbackMethod {
    pub ts_name: String,
    pub start_import_name: String,
    pub complete_export_name: String,
    pub params: Vec<TsCallbackParam>,
    pub return_type: Option<String>,
    pub encode_expr: Option<String>,
    pub size_expr: Option<String>,
    pub direct_write_method: Option<String>,
    pub direct_write_value_expr: Option<String>,
    pub direct_size: Option<usize>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TsRecord {
    pub name: String,
    pub fields: Vec<TsField>,
    pub constructors: Vec<TsValueTypeConstructor>,
    pub methods: Vec<TsValueTypeMethod>,
    pub is_blittable: bool,
    pub wire_size: Option<usize>,
    pub tail_padding: usize,
    pub doc: Option<String>,
}

impl TsRecord {
    pub fn has_companion(&self) -> bool {
        !self.constructors.is_empty() || !self.methods.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct TsField {
    pub name: String,
    pub ts_type: String,
    pub decode: ReadSeq,
    pub encode: WriteSeq,
    pub doc: Option<String>,
}

impl TsField {
    pub fn wire_decode_expr(&self) -> String {
        emit::emit_reader_read(&self.decode)
    }

    pub fn wire_encode_expr(&self, writer: &str, value: &str) -> String {
        emit::emit_writer_write(&self.encode, writer, value)
    }

    pub fn wire_size_expr(&self, value: &str) -> String {
        emit::emit_size_expr(&self.encode.size, value)
    }
}

#[derive(Debug, Clone)]
pub struct TsEnum {
    pub name: String,
    pub variants: Vec<TsVariant>,
    pub constructors: Vec<TsValueTypeConstructor>,
    pub methods: Vec<TsValueTypeMethod>,
    pub kind: TsEnumKind,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum TsEnumKind {
    CStyle,
    Data,
}

impl TsEnum {
    pub fn is_c_style(&self) -> bool {
        matches!(self.kind, TsEnumKind::CStyle)
    }

    pub fn has_companion(&self) -> bool {
        !self.constructors.is_empty() || !self.methods.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct TsValueTypeConstructor {
    pub ts_name: String,
    pub ffi_name: String,
    pub params: Vec<TsParam>,
    pub return_type: String,
    pub return_route: TsOutputRoute,
    pub doc: Option<String>,
}

impl TsValueTypeConstructor {
    pub fn wrapper_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::wrapper_code)
            .collect::<Vec<_>>()
            .join("\n    ")
    }

    pub fn cleanup_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::cleanup_code)
            .collect::<Vec<_>>()
            .join("\n      ")
    }

    pub fn ffi_call_args(&self) -> String {
        flatten_ffi_args(&self.params).join(", ")
    }
}

#[derive(Debug, Clone)]
pub struct TsValueTypeMethod {
    pub ts_name: String,
    pub ffi_name: String,
    pub is_static: bool,
    pub params: Vec<TsParam>,
    pub return_type: Option<String>,
    pub return_handle: Option<TsHandleReturn>,
    pub return_callback: Option<TsCallbackHandleReturn>,
    pub mode: TsValueTypeMethodMode,
    pub doc: Option<String>,
}

impl TsValueTypeMethod {
    pub fn wrapper_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::wrapper_code)
            .collect::<Vec<_>>()
            .join("\n    ")
    }

    pub fn cleanup_code(&self) -> String {
        self.params
            .iter()
            .filter_map(TsParam::cleanup_code)
            .collect::<Vec<_>>()
            .join("\n      ")
    }

    pub fn ffi_call_args(&self) -> String {
        flatten_ffi_args(&self.params).join(", ")
    }

    pub fn ffi_call_args_with_out(&self) -> String {
        let call_args = self.ffi_call_args();
        if call_args.is_empty() {
            "__outPtr".to_string()
        } else {
            format!("__outPtr, {call_args}")
        }
    }

    pub fn is_async(&self) -> bool {
        matches!(self.mode, TsValueTypeMethodMode::Async(_))
    }
}

#[derive(Debug, Clone)]
pub enum TsValueTypeMethodMode {
    Sync(TsValueTypeSyncMethod),
    Async(TsValueTypeAsyncMethod),
}

#[derive(Debug, Clone)]
pub struct TsValueTypeSyncMethod {
    pub return_route: TsOutputRoute,
}

#[derive(Debug, Clone)]
pub struct TsValueTypeAsyncMethod {
    pub poll_sync_ffi_name: String,
    pub complete_ffi_name: String,
    pub panic_message_ffi_name: String,
    pub cancel_ffi_name: String,
    pub free_ffi_name: String,
    pub return_route: TsOutputRoute,
}

#[derive(Debug, Clone)]
pub struct TsVariant {
    pub name: String,
    pub discriminant: i128,
    pub fields: Vec<TsVariantField>,
    pub doc: Option<String>,
}

impl TsVariant {
    pub fn is_unit(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn size_expr(&self) -> String {
        if self.fields.is_empty() {
            "4".to_string()
        } else {
            let field_sizes: Vec<String> =
                self.fields.iter().map(|f| f.wire_size_expr("v")).collect();
            format!("4 + {}", field_sizes.join(" + "))
        }
    }
}

#[derive(Debug, Clone)]
pub struct TsVariantField {
    pub name: String,
    pub ts_type: String,
    pub decode: ReadSeq,
    pub encode: WriteSeq,
}

impl TsVariantField {
    pub fn wire_decode_expr(&self) -> String {
        emit::emit_reader_read(&self.decode)
    }

    pub fn wire_encode_expr(&self, writer: &str, value: &str) -> String {
        emit::emit_writer_write(&self.encode, writer, value)
    }

    pub fn wire_size_expr(&self, value: &str) -> String {
        emit::emit_size_expr(&self.encode.size, value)
    }
}

#[derive(Debug, Clone)]
pub struct TsFunction {
    pub name: String,
    pub ffi_name: String,
    pub params: Vec<TsParam>,
    pub return_type: Option<String>,
    pub return_route: TsOutputRoute,
    pub return_callback: Option<TsCallbackHandleReturn>,
    pub throws: bool,
    pub err_type: String,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TsParam {
    pub name: String,
    pub ts_type: String,
    pub input_route: TsInputRoute,
}

impl TsParam {
    pub fn wrapper_code(&self) -> Option<String> {
        match &self.input_route {
            TsInputRoute::Direct => None,
            TsInputRoute::String => Some(format!(
                "const {}_alloc = _module.allocString({});",
                self.name, self.name
            )),
            TsInputRoute::Bytes => Some(format!(
                "const {}_alloc = _module.allocBytes({});",
                self.name, self.name
            )),
            TsInputRoute::PrimitiveBuffer { element_abi } => Some(format!(
                "const {}_alloc = _module.{}({});",
                self.name,
                primitive_buffer_alloc_method(element_abi),
                self.name
            )),
            TsInputRoute::CompositeBuffer {
                codec_name,
                element_size,
                ..
            } => {
                let writer_name = format!("{}_writer", self.name);
                Some(format!(
                    "const {writer_name} = _module.allocCompositeBuffer({}, {element_size}, (writer, item) => {{ {codec_name}Codec.encode(writer, item); }});",
                    self.name
                ))
            }
            TsInputRoute::Callback {
                interface_name,
                nullable,
            } => Some(if *nullable {
                format!(
                    "const {}_handle = {} === null ? 0 : register{}({});",
                    self.name, self.name, interface_name, self.name
                )
            } else {
                format!(
                    "const {}_handle = register{}({});",
                    self.name, interface_name, self.name
                )
            }),
            TsInputRoute::StructValue { codec_name } => {
                let writer_name = format!("{}_writer", self.name);
                Some(format!(
                    "const {writer_name} = _module.allocWriter({codec_name}Codec.size({}));\n  {codec_name}Codec.encode({writer_name}, {});",
                    self.name, self.name
                ))
            }
            TsInputRoute::CodecEncoded { codec_name } => {
                let writer_name = format!("{}_writer", self.name);
                Some(format!(
                    "const {writer_name} = _module.allocWriter({codec_name}Codec.size({}));\n  {codec_name}Codec.encode({writer_name}, {});",
                    self.name, self.name
                ))
            }
            TsInputRoute::OtherEncoded { encode } => {
                let writer_name = format!("{}_writer", self.name);
                let size_expr = emit::emit_size_expr(&encode.size, &self.name);
                let encode_expr = emit::emit_writer_write(encode, &writer_name, &self.name);
                Some(format!(
                    "const {writer_name} = _module.allocWriter({size_expr});\n  {encode_expr};",
                ))
            }
        }
    }

    pub fn ffi_args(&self) -> Vec<String> {
        match &self.input_route {
            TsInputRoute::Direct => vec![self.name.clone()],
            TsInputRoute::String | TsInputRoute::Bytes => {
                vec![
                    format!("{}_alloc.ptr", self.name),
                    format!("{}_alloc.len", self.name),
                ]
            }
            TsInputRoute::PrimitiveBuffer { .. } => {
                vec![
                    format!("{}_alloc.ptr", self.name),
                    format!("{}_alloc.len", self.name),
                ]
            }
            TsInputRoute::CompositeBuffer { length_unit, .. } => {
                vec![
                    format!("{}_writer.ptr", self.name),
                    match length_unit {
                        SpanLengthUnit::Elements => format!("{}.length", self.name),
                        SpanLengthUnit::Bytes => format!("{}_writer.len", self.name),
                    },
                ]
            }
            TsInputRoute::Callback { .. } => {
                vec![format!("{}_handle", self.name)]
            }
            TsInputRoute::StructValue { .. } => {
                vec![format!("{}_writer.ptr", self.name)]
            }
            TsInputRoute::CodecEncoded { .. } | TsInputRoute::OtherEncoded { .. } => {
                vec![
                    format!("{}_writer.ptr", self.name),
                    format!("{}_writer.len", self.name),
                ]
            }
        }
    }

    pub fn cleanup_code(&self) -> Option<String> {
        match &self.input_route {
            TsInputRoute::Direct | TsInputRoute::Callback { .. } => None,
            TsInputRoute::String | TsInputRoute::Bytes => {
                Some(format!("_module.freeAlloc({}_alloc);", self.name))
            }
            TsInputRoute::PrimitiveBuffer { .. } => {
                Some(format!("_module.freePrimitiveBuffer({}_alloc);", self.name))
            }
            TsInputRoute::CompositeBuffer { .. }
            | TsInputRoute::StructValue { .. }
            | TsInputRoute::CodecEncoded { .. }
            | TsInputRoute::OtherEncoded { .. } => {
                Some(format!("_module.freeWriter({}_writer);", self.name))
            }
        }
    }

    pub fn needs_cleanup(&self) -> bool {
        !matches!(self.input_route, TsInputRoute::Direct)
    }
}

fn flatten_ffi_args(params: &[TsParam]) -> Vec<String> {
    params.iter().flat_map(TsParam::ffi_args).collect()
}

#[derive(Debug, Clone)]
pub enum TsInputRoute {
    Direct,
    String,
    Bytes,
    PrimitiveBuffer {
        element_abi: AbiType,
    },
    CompositeBuffer {
        codec_name: String,
        element_size: usize,
        length_unit: SpanLengthUnit,
    },
    Callback {
        interface_name: String,
        nullable: bool,
    },
    StructValue {
        codec_name: String,
    },
    CodecEncoded {
        codec_name: String,
    },
    OtherEncoded {
        encode: WriteSeq,
    },
}

fn primitive_buffer_alloc_method(abi_type: &AbiType) -> &'static str {
    match abi_type {
        AbiType::Bool => "allocBoolArray",
        AbiType::I8 => "allocI8Array",
        AbiType::U8 => "allocU8Array",
        AbiType::I16 => "allocI16Array",
        AbiType::U16 => "allocU16Array",
        AbiType::I32 => "allocI32Array",
        AbiType::U32 => "allocU32Array",
        AbiType::I64 => "allocI64Array",
        AbiType::U64 => "allocU64Array",
        AbiType::ISize => "allocI32Array",
        AbiType::USize => "allocU32Array",
        AbiType::F32 => "allocF32Array",
        AbiType::F64 => "allocF64Array",
        AbiType::Void
        | AbiType::Pointer(_)
        | AbiType::OwnedBuffer
        | AbiType::InlineCallbackFn { .. }
        | AbiType::Handle(_)
        | AbiType::CallbackHandle
        | AbiType::Struct(_) => {
            panic!("unsupported primitive buffer abi type: {abi_type:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_buffer_param_generates_expected_wrapper_and_cleanup() {
        let param = TsParam {
            name: "values".to_string(),
            ts_type: "number[]".to_string(),
            input_route: TsInputRoute::PrimitiveBuffer {
                element_abi: AbiType::I32,
            },
        };

        assert_eq!(
            param.wrapper_code(),
            Some("const values_alloc = _module.allocI32Array(values);".to_string())
        );
        assert_eq!(
            param.ffi_args(),
            vec![
                "values_alloc.ptr".to_string(),
                "values_alloc.len".to_string()
            ]
        );
        assert_eq!(
            param.cleanup_code(),
            Some("_module.freePrimitiveBuffer(values_alloc);".to_string())
        );
        assert!(param.needs_cleanup());
    }

    #[test]
    fn primitive_buffer_param_uses_alloc_method_for_bigint_vectors() {
        let param = TsParam {
            name: "values".to_string(),
            ts_type: "bigint[]".to_string(),
            input_route: TsInputRoute::PrimitiveBuffer {
                element_abi: AbiType::I64,
            },
        };

        assert_eq!(
            param.wrapper_code(),
            Some("const values_alloc = _module.allocI64Array(values);".to_string())
        );
    }

    #[test]
    fn composite_buffer_param_generates_expected_wrapper_and_cleanup() {
        let param = TsParam {
            name: "points".to_string(),
            ts_type: "Point[]".to_string(),
            input_route: TsInputRoute::CompositeBuffer {
                codec_name: "Point".to_string(),
                element_size: 16,
                length_unit: SpanLengthUnit::Bytes,
            },
        };

        assert_eq!(
            param.wrapper_code(),
            Some(
                "const points_writer = _module.allocCompositeBuffer(points, 16, (writer, item) => { PointCodec.encode(writer, item); });".to_string()
            )
        );
        assert_eq!(
            param.ffi_args(),
            vec![
                "points_writer.ptr".to_string(),
                "points_writer.len".to_string()
            ]
        );
        assert_eq!(
            param.cleanup_code(),
            Some("_module.freeWriter(points_writer);".to_string())
        );
        assert!(param.needs_cleanup());
    }

    #[test]
    fn borrowed_composite_buffer_param_passes_element_count() {
        let param = TsParam {
            name: "points".to_string(),
            ts_type: "Point[]".to_string(),
            input_route: TsInputRoute::CompositeBuffer {
                codec_name: "Point".to_string(),
                element_size: 16,
                length_unit: SpanLengthUnit::Elements,
            },
        };

        assert_eq!(
            param.ffi_args(),
            vec!["points_writer.ptr".to_string(), "points.length".to_string()]
        );
    }
}

#[derive(Debug, Clone)]
pub struct TsOutputRoute {
    is_void: bool,
    is_direct: bool,
    is_packed: bool,
    is_raw_packed: bool,
    is_nan_boxed_optional: bool,
    is_void_slot: bool,
    is_struct_return_slot: bool,
    is_async_scalar: bool,
    return_slot_size: Option<usize>,
    ts_cast: String,
    decode_expr: String,
}

impl TsOutputRoute {
    pub fn void() -> Self {
        Self {
            is_void: true,
            is_direct: false,
            is_packed: false,
            is_raw_packed: false,
            is_nan_boxed_optional: false,
            is_void_slot: false,
            is_struct_return_slot: false,
            is_async_scalar: false,
            return_slot_size: None,
            ts_cast: String::new(),
            decode_expr: String::new(),
        }
    }

    pub fn direct(ts_cast: String) -> Self {
        Self {
            is_void: false,
            is_direct: true,
            is_packed: false,
            is_raw_packed: false,
            is_nan_boxed_optional: false,
            is_void_slot: false,
            is_struct_return_slot: false,
            is_async_scalar: false,
            return_slot_size: None,
            ts_cast,
            decode_expr: String::new(),
        }
    }

    pub fn packed(decode_expr: String) -> Self {
        Self {
            is_void: false,
            is_direct: false,
            is_packed: true,
            is_raw_packed: false,
            is_nan_boxed_optional: false,
            is_void_slot: false,
            is_struct_return_slot: false,
            is_async_scalar: false,
            return_slot_size: None,
            ts_cast: String::new(),
            decode_expr,
        }
    }

    pub fn raw_packed(decode_expr: String) -> Self {
        Self {
            is_void: false,
            is_direct: false,
            is_packed: false,
            is_raw_packed: true,
            is_nan_boxed_optional: false,
            is_void_slot: false,
            is_struct_return_slot: false,
            is_async_scalar: false,
            return_slot_size: None,
            ts_cast: String::new(),
            decode_expr,
        }
    }

    pub fn nan_boxed_optional(decode_expr: String) -> Self {
        Self {
            is_void: false,
            is_direct: false,
            is_packed: false,
            is_raw_packed: false,
            is_nan_boxed_optional: true,
            is_void_slot: false,
            is_struct_return_slot: false,
            is_async_scalar: false,
            return_slot_size: None,
            ts_cast: String::new(),
            decode_expr,
        }
    }

    pub fn async_scalar(ts_cast: String) -> Self {
        Self {
            is_void: false,
            is_direct: false,
            is_packed: false,
            is_raw_packed: false,
            is_nan_boxed_optional: false,
            is_void_slot: false,
            is_struct_return_slot: false,
            is_async_scalar: true,
            return_slot_size: None,
            ts_cast,
            decode_expr: String::new(),
        }
    }

    pub fn void_slot(decode_expr: String) -> Self {
        Self {
            is_void: false,
            is_direct: false,
            is_packed: false,
            is_raw_packed: false,
            is_nan_boxed_optional: false,
            is_void_slot: true,
            is_struct_return_slot: false,
            is_async_scalar: false,
            return_slot_size: None,
            ts_cast: String::new(),
            decode_expr,
        }
    }

    pub fn struct_return_slot(return_slot_size: usize, decode_expr: String) -> Self {
        Self {
            is_void: false,
            is_direct: false,
            is_packed: false,
            is_raw_packed: false,
            is_nan_boxed_optional: false,
            is_void_slot: false,
            is_struct_return_slot: true,
            is_async_scalar: false,
            return_slot_size: Some(return_slot_size),
            ts_cast: String::new(),
            decode_expr,
        }
    }

    pub fn is_void(&self) -> bool {
        self.is_void
    }

    pub fn is_direct(&self) -> bool {
        self.is_direct
    }

    pub fn is_packed(&self) -> bool {
        self.is_packed
    }

    pub fn is_raw_packed(&self) -> bool {
        self.is_raw_packed
    }

    pub fn is_nan_boxed_optional(&self) -> bool {
        self.is_nan_boxed_optional
    }

    pub fn is_void_slot(&self) -> bool {
        self.is_void_slot
    }

    pub fn is_struct_return_slot(&self) -> bool {
        self.is_struct_return_slot
    }

    pub fn is_async_scalar(&self) -> bool {
        self.is_async_scalar
    }

    pub fn ts_cast(&self) -> &str {
        self.ts_cast.as_str()
    }

    pub fn decode_expr(&self) -> &str {
        self.decode_expr.as_str()
    }

    pub fn return_slot_size(&self) -> Option<usize> {
        self.return_slot_size
    }

    pub fn with_ts_cast(mut self, ts_cast: String) -> Self {
        self.ts_cast = ts_cast;
        self
    }
}

#[derive(Debug, Clone)]
pub struct TsWasmImport {
    pub ffi_name: String,
    pub params: Vec<TsWasmParam>,
    pub return_wasm_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TsWasmParam {
    pub name: String,
    pub wasm_type: String,
}
