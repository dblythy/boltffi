use crate::{
    ir::{
        AbiCall, AbiContract, CallId, ConstructorDef, FfiContract, FunctionDef, FunctionId,
        MethodDef, ParamDef, Receiver,
    },
    render::dart::{
        DartConstructor, DartConstructorKind, DartFunction, DartFunctionParam, DartLibrary,
        DartNative, DartType, NamingConvention,
    },
};

mod call;
mod callback;
mod class;
mod custom_type;
mod enumeration;
mod native_function;
mod record;

pub struct DartLowerer<'a> {
    ffi: &'a FfiContract,
    abi: &'a AbiContract,
    package_name: &'a str,
}

impl<'a> DartLowerer<'a> {
    pub fn new(ffi: &'a FfiContract, abi: &'a AbiContract, package_name: &'a str) -> Self {
        Self {
            ffi,
            abi,
            package_name,
        }
    }

    pub fn abi_call_for_function(&self, function: &FunctionId) -> &AbiCall {
        self.abi
            .calls
            .iter()
            .find(|c| match &c.id {
                CallId::Function(id) => id == function,
                _ => false,
            })
            .unwrap()
    }

    pub fn abi_call_for_call_id(&self, call_id: &CallId) -> &AbiCall {
        self.abi.calls.iter().find(|c| &c.id == call_id).unwrap()
    }

    fn lower_param(&self, param: &ParamDef) -> DartFunctionParam {
        DartFunctionParam {
            name: NamingConvention::param_name(param.name.as_str()),
            ty: DartType::from_type_expr(&param.type_expr, &self.ffi.catalog),
        }
    }

    fn lower_constructor(&self, ctor: &ConstructorDef, id: CallId) -> DartConstructor {
        let abi_call = self.abi_call_for_call_id(&id);

        let native = self.lower_one_native_function(abi_call);

        let self_type_name = match &id {
            CallId::Constructor { class_id, .. } => NamingConvention::class_name(class_id.as_str()),
            CallId::RecordConstructor { record_id, .. } => {
                NamingConvention::class_name(record_id.as_str())
            }
            CallId::EnumConstructor { enum_id, .. } => {
                NamingConvention::class_name(enum_id.as_str())
            }
            _ => unreachable!("constructor CallId is always a *Constructor variant"),
        };
        let return_info = call::DartReturnInfo::for_constructor(ctor.is_fallible(), self_type_name);
        let is_async = matches!(abi_call.mode, crate::ir::CallMode::Async(_));
        let body = call::render_body(abi_call, &native.return_type, &return_info, true);

        DartConstructor {
            native,
            params: ctor
                .params()
                .iter()
                .map(|param| self.lower_param(param))
                .collect(),
            kind: match ctor {
                ConstructorDef::Default { .. } => DartConstructorKind::Default,
                ConstructorDef::NamedFactory { name, .. }
                | ConstructorDef::NamedInit { name, .. } => DartConstructorKind::Named {
                    name: NamingConvention::function_name(name.as_str()),
                },
            },
            is_fallible: ctor.is_fallible(),
            is_async,
            body,
        }
    }

    fn lower_method(&self, meth: &MethodDef, id: CallId) -> DartFunction {
        let abi_call = self.abi_call_for_call_id(&id);

        let native = self.lower_one_native_function(abi_call);
        let return_info = call::DartReturnInfo::for_method(&meth.returns);
        let is_async = matches!(abi_call.mode, crate::ir::CallMode::Async(_));
        let body = call::render_body(abi_call, &native.return_type, &return_info, false);

        DartFunction {
            name: NamingConvention::function_name(meth.id.as_str()),
            native,
            params: meth.params.iter().map(|p| self.lower_param(p)).collect(),
            ret_ty: DartType::from_return_def(&meth.returns, &self.ffi.catalog),
            receiver: meth.receiver,
            is_async,
            body,
        }
    }

    /// Lowers one top-level free function (e.g. `set_http_transport`) into its
    /// public Dart wrapper — the `DartFunction`'s `body` marshals into/out of
    /// the `_f$<symbol>` native declaration `lower_native_functions` already
    /// emits for it, via the exact same `call::render_body` a static class
    /// method's body is built from (`lower_method`, above). A free function
    /// has no receiver to marshal, so `plan_call` never sees a "self" param —
    /// `Receiver::Static` here only feeds `DartFunction::is_static`, which the
    /// top-level function template never consults.
    fn lower_function(&self, def: &FunctionDef) -> DartFunction {
        let call_id = CallId::Function(def.id.clone());
        let abi_call = self.abi_call_for_call_id(&call_id);

        let native = self.lower_one_native_function(abi_call);
        let return_info = call::DartReturnInfo::for_method(&def.returns);
        let is_async = matches!(abi_call.mode, crate::ir::CallMode::Async(_));
        let body = call::render_body(abi_call, &native.return_type, &return_info, false);

        DartFunction {
            name: NamingConvention::function_name(def.id.as_str()),
            native,
            params: def.params.iter().map(|p| self.lower_param(p)).collect(),
            ret_ty: DartType::from_return_def(&def.returns, &self.ffi.catalog),
            receiver: Receiver::Static,
            is_async,
            body,
        }
    }

    fn lower_functions(&self) -> Vec<DartFunction> {
        self.ffi
            .functions
            .iter()
            .map(|f| self.lower_function(f))
            .collect()
    }

    pub fn library(&self) -> DartLibrary {
        let custom_types = self.lower_custom_types();
        let records = self.lower_records();
        let native_functions = self.lower_native_functions();
        let enums = self.lower_enums();
        let callbacks = self.lower_callbacks();
        let classes = self.lower_classes();
        let functions = self.lower_functions();

        DartLibrary {
            custom_types,
            native: DartNative {
                functions: native_functions,
            },
            records,
            enums,
            callbacks,
            classes,
            functions,
        }
    }
}

/// One test per shape of top-level free function the ABI contract can
/// express — parse-core-rs's global setters (`set_http_transport`,
/// `set_client_platform`, `sdk_version`, `validate_role_name`, ...) exercise
/// void/scalar/encoded returns, fallible (encoded-error) returns, string and
/// `BoxedDyn` callback params, and async — the exact matrix that previously
/// got only a raw `@Native` declaration with no public wrapper.
#[cfg(test)]
mod tests {
    use boltffi_ffi_rules::callable::ExecutionKind;

    use crate::{
        ir::{
            CallbackId, CallbackKind, CallbackMethodDef, CallbackTraitDef, FieldDef, FieldName,
            FunctionDef, FunctionId, MethodId, ParamDef, ParamName, ParamPassing, PrimitiveType,
            RecordDef, RecordId, ReturnDef, TypeExpr,
        },
        render::dart::test,
    };

    fn string_param(name: &str) -> ParamDef {
        ParamDef {
            name: ParamName::new(name),
            type_expr: TypeExpr::String,
            passing: ParamPassing::Value,
            doc: None,
        }
    }

    fn parse_error_record() -> RecordDef {
        RecordDef {
            qualified_path: String::new(),
            id: RecordId::new("ParseError"),
            is_repr_c: false,
            is_error: true,
            fields: vec![FieldDef {
                name: FieldName::new("message"),
                type_expr: TypeExpr::String,
                doc: None,
                default: None,
            }],
            constructors: vec![],
            methods: vec![],
            doc: None,
            deprecated: None,
        }
    }

    #[test]
    fn free_function_sync_void_renders_top_level_with_no_receiver() {
        let mut ffi = test::empty_contract();
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("set_client_platform"),
            params: vec![string_param("tag")],
            returns: ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert_eq!(function.name, "setClientPlatform");
        assert!(!function.is_async);
        // A free function has no receiver — `plan_call` never substitutes a
        // "this" the way a class method's `self` param does.
        assert!(!function.body.contains("this"), "body: {}", function.body);
        assert!(function.params.iter().any(|p| p.name == "tag"));
    }

    #[test]
    fn free_function_sync_scalar_in_and_out() {
        let mut ffi = test::empty_contract();
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("live_query_reconnect_delay_ms"),
            params: vec![ParamDef {
                name: ParamName::new("attempt"),
                type_expr: TypeExpr::Primitive(PrimitiveType::U32),
                passing: ParamPassing::Value,
                doc: None,
            }],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::U64)),
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert_eq!(function.ret_ty.dart_type(), "int");
        assert!(!function.is_async);
        assert!(
            function.body.contains("return _p$result;"),
            "body: {}",
            function.body
        );
    }

    #[test]
    fn free_function_encoded_string_return_frees_the_wire_buffer() {
        let mut ffi = test::empty_contract();
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("sdk_version"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::String),
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert_eq!(function.ret_ty.dart_type(), "String");
        assert!(
            function.body.contains("_f$boltffi_free_buf(_p$result);"),
            "body: {}",
            function.body
        );
    }

    #[test]
    fn free_function_fallible_encoded_error_returns_bolt_ffi_result_not_a_throw() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_record(parse_error_record());
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("validate_role_name"),
            params: vec![string_param("name")],
            returns: ReturnDef::Result {
                ok: TypeExpr::Void,
                err: TypeExpr::Record(RecordId::new("ParseError")),
            },
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert_eq!(
            function.ret_ty.dart_type(),
            "BoltFFIResult<void, ParseError>"
        );
        // Exactly one `readResult` (the whole tag+ok+err decode is one unit —
        // see the regression note on `call::decode_return`), and a free
        // function's declared return type is the `BoltFFIResult` itself
        // (never unwrap-or-throw — that's constructor-only).
        assert_eq!(
            function.body.matches("readResult").count(),
            1,
            "body: {}",
            function.body
        );
        assert!(
            !function.body.contains("okOrThrow"),
            "body: {}",
            function.body
        );
        // Regression (consumer-repo finding, 2026-07-20): `Ok(())`'s wire
        // payload is zero bytes — the response buffer for a void-ok Result
        // is exactly the 1-byte tag `readResult` already consumed before
        // calling this closure. The Ok arm used to read a phantom extra
        // byte (`readU8()`), throwing `StateError: Buffer overflow` against
        // every real void-returning fallible call (save/fetch/destroy,
        // setAcl, or/and/nor, every set*Storage free function, ...) the
        // instant it ran against a real response. The Ok arm for a void
        // Result must read nothing.
        assert!(
            !function.body.contains("readU8()"),
            "the Ok arm of a void Result must not read a phantom byte: {}",
            function.body
        );
        assert!(
            function.body.contains("(_p$reader) {},\n  (_p$reader) => ParseError._m$wireDecode(_p$reader)"),
            "the Ok arm of a void Result must be a true no-op read: {}",
            function.body
        );
    }

    fn boxed_dyn_callback(id: &str) -> CallbackTraitDef {
        CallbackTraitDef {
            qualified_path: String::new(),
            id: CallbackId::new(id),
            methods: vec![CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: MethodId::new("fetch"),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: CallbackKind::Trait,
            doc: None,
        }
    }

    #[test]
    fn free_function_boxed_dyn_callback_param_boxes_through_the_handle_map() {
        let mut ffi = test::empty_contract();
        ffi.catalog
            .insert_callback(boxed_dyn_callback("HttpTransport"));
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("set_http_transport"),
            params: vec![ParamDef {
                name: ParamName::new("transport"),
                type_expr: TypeExpr::Callback(CallbackId::new("HttpTransport")),
                passing: ParamPassing::BoxedDyn,
                doc: None,
            }],
            returns: ReturnDef::Void,
            execution_kind: ExecutionKind::Sync,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert_eq!(function.name, "setHttpTransport");
        assert!(
            function
                .body
                .contains("HttpTransportHandleMap.createHandle(transport)"),
            "body: {}",
            function.body
        );
        assert!(
            !function.body.contains("UnsupportedError"),
            "body: {}",
            function.body
        );
    }

    #[test]
    fn free_function_async_drives_bolt_ffi_async_create() {
        let mut ffi = test::empty_contract();
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("fetch_remote_config"),
            params: vec![],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::U32)),
            execution_kind: ExecutionKind::Async,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert!(function.is_async);
        assert_eq!(function.ret_ty.dart_type(), "int");
        assert!(
            function.body.contains("_$$BoltFFIAsync.create("),
            "body: {}",
            function.body
        );
        assert!(
            function.body.contains("completeFuture: (handle)"),
            "body: {}",
            function.body
        );
    }

    #[test]
    fn free_function_async_fallible_wraps_bolt_ffi_result() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_record(parse_error_record());
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("fetch_session"),
            params: vec![],
            returns: ReturnDef::Result {
                ok: TypeExpr::String,
                err: TypeExpr::Record(RecordId::new("ParseError")),
            },
            execution_kind: ExecutionKind::Async,
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let function = &library.functions[0];

        assert!(function.is_async);
        assert_eq!(
            function.ret_ty.dart_type(),
            "BoltFFIResult<String, ParseError>"
        );
        assert!(
            function.body.contains("_$$BoltFFIAsync.create("),
            "body: {}",
            function.body
        );
    }
}
