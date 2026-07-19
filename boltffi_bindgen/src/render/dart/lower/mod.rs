use crate::{
    ir::{
        AbiCall, AbiContract, CallId, ConstructorDef, FfiContract, FunctionId, MethodDef, ParamDef,
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

    pub fn library(&self) -> DartLibrary {
        let custom_types = self.lower_custom_types();
        let records = self.lower_records();
        let native_functions = self.lower_native_functions();
        let enums = self.lower_enums();
        let callbacks = self.lower_callbacks();
        let classes = self.lower_classes();

        DartLibrary {
            custom_types,
            native: DartNative {
                functions: native_functions,
            },
            records,
            enums,
            callbacks,
            classes,
        }
    }
}
