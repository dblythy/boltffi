use boltffi_ffi_rules::transport::ParamValueStrategy;

use crate::{
    ir::{AbiCall, AbiParam, AbiType, CallMode, ParamRole},
    render::dart::{
        DartNativeFunction, DartNativeFunctionCallMode, DartNativeFunctionParam, DartNativeType,
        NamingConvention,
    },
};

/// The hygienic native ptr param name for an encoded param whose *logical*
/// (Rust-derived) name is `base` — shared with the callback trampoline
/// renderer (`callback.rs`'s `decode_input_args`), which must reference the
/// exact same identifier this function declares in the native signature.
pub(super) fn encoded_ptr_name(base: &str) -> String {
    format!("_p${}Ptr", NamingConvention::param_name(base))
}

/// The hygienic native len param name for an encoded param whose *logical*
/// (Rust-derived) name is `base` — see [`encoded_ptr_name`].
pub(super) fn encoded_len_name(base: &str) -> String {
    format!("_p${}Len", NamingConvention::param_name(base))
}

impl<'a> super::DartLowerer<'a> {
    pub(super) fn lower_native_function_param(
        &self,
        abi_param: &AbiParam,
    ) -> DartNativeFunctionParam {
        let name = match &abi_param.role {
            ParamRole::Input { contract, .. } => match contract.value_strategy() {
                ParamValueStrategy::DirectBuffer(..)
                | ParamValueStrategy::WireEncoded(..)
                | ParamValueStrategy::Utf8String
                | ParamValueStrategy::CompositeValue => encoded_ptr_name(abi_param.name.as_str()),
                _ => NamingConvention::param_name(abi_param.name.as_str()),
            },
            // A derived ptr/len name is invented by suffixing the *source*
            // param's own name — a real Rust param can legitimately be named
            // e.g. `payload_ptr`/`payload_len` and would otherwise collide
            // with the ptr/len pair this renderer invents for an unrelated
            // encoded param named `payload`. `_p$` can't appear in a Rust
            // identifier (`NamingConvention::param_name` never produces it),
            // so prefixing every *invented* name with it — the same hygiene
            // marker used for every other synthetic local in this renderer
            // (`_p$handle`, `_p$outStatus`, `_p$value`, ...) — makes the
            // collision structurally impossible rather than merely unlikely.
            ParamRole::SyntheticLen { for_param } => encoded_len_name(for_param.as_str()),
            ParamRole::OutDirect => String::from("_p$outPtr"),
            ParamRole::OutLen { .. } => String::from("_p$outLen"),
            _ => NamingConvention::param_name(abi_param.name.as_str()),
        };

        DartNativeFunctionParam {
            name,
            native_type: DartNativeType::from_abi_param(abi_param),
        }
    }

    pub(super) fn lower_one_native_function(&self, abi_call: &AbiCall) -> DartNativeFunction {
        let symbol = abi_call.symbol.to_string();

        let params = abi_call
            .params
            .iter()
            .map(|p| self.lower_native_function_param(p))
            .collect();

        let is_not_leaf =
            Self::call_has_callback_param(abi_call) || self.class_owns_a_callback(abi_call);

        let call_mode = match &abi_call.mode {
            CallMode::Sync => DartNativeFunctionCallMode::Sync,
            CallMode::Async(call) => DartNativeFunctionCallMode::Async {
                poll_symbol: call.poll.to_string(),
                complete_symbol: call.complete.to_string(),
                complete_ty: DartNativeType::from_return_shape_and_error_transport(
                    &call.result,
                    &call.error,
                ),
                cancel_symbol: call.cancel.to_string(),
                free_symbol: call.free.to_string(),
            },
        };

        DartNativeFunction {
            symbol,
            params,
            return_type: match &call_mode {
                DartNativeFunctionCallMode::Sync => {
                    DartNativeType::from_return_shape_and_error_transport(
                        &abi_call.returns,
                        &abi_call.error,
                    )
                }
                DartNativeFunctionCallMode::Async { .. } => {
                    DartNativeType::Pointer(Box::new(DartNativeType::Void))
                }
            },
            is_leaf: !is_not_leaf,
            call_mode,
        }
    }

    fn call_has_callback_param(call: &AbiCall) -> bool {
        call.params.iter().any(|p| {
            matches!(
                p.abi_type,
                AbiType::InlineCallbackFn { .. } | AbiType::CallbackHandle
            )
        })
    }

    /// True when `call`'s class has ANY constructor or method (not
    /// necessarily `call` itself) that takes a callback-handle-shaped
    /// parameter — meaning some instance of this class holds a
    /// host-implemented callback it may invoke synchronously from inside
    /// ANY of its methods, not only the one that received it as a param.
    ///
    /// `dart:ffi`'s `isLeaf: true` promises the Dart VM the native call will
    /// never call back into Dart; declaring it on a method that reenters a
    /// stored callback aborts the process (`Cannot invoke native callback
    /// from a leaf call`) — confirmed for real by `LiveQueryClient::disconnect`/
    /// `release` and `WatchHandle::unsubscribe`, none of which take a
    /// callback param themselves, all of which reenter a transport/listener
    /// their class's constructor stored. The IR has no per-method "may
    /// reenter" fact to check directly, so this is deliberately
    /// conservative at the whole-class granularity: a sibling method that
    /// genuinely never reenters only loses the `isLeaf` fast path, never
    /// correctness — the failure mode on the other side is a process abort.
    fn class_owns_a_callback(&self, call: &AbiCall) -> bool {
        let Some(class_id) = call.id.class_id() else {
            return false;
        };
        self.abi
            .calls
            .iter()
            .filter(|c| c.id.class_id() == Some(class_id))
            .any(Self::call_has_callback_param)
    }

    pub(super) fn lower_native_functions(&self) -> Vec<DartNativeFunction> {
        self.ffi
            .functions
            .iter()
            .map(|f| {
                let abi_call = self.abi_call_for_function(&f.id);
                self.lower_one_native_function(abi_call)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use boltffi_ffi_rules::callable::ExecutionKind;

    use crate::{
        ir::{
            CallbackId, CallbackKind, CallbackTraitDef, ClassDef, ClassId, ConstructorDef,
            FunctionDef, FunctionId, MethodDef, ParamDef, ParamName, ParamPassing, PrimitiveType,
            Receiver, ReturnDef, TypeExpr,
        },
        render::dart::test,
    };

    use super::*;

    #[test]
    pub fn native_function_primitive_in() {
        let mut ffi = test::empty_contract();
        ffi.functions.insert(
            0,
            FunctionDef {
                qualified_path: String::new(),
                id: FunctionId::new("echo_u64"),
                params: vec![ParamDef {
                    name: ParamName::new("v"),
                    type_expr: TypeExpr::Primitive(PrimitiveType::U64),
                    passing: ParamPassing::Value,
                    doc: None,
                }],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            },
        );

        let library = test::lower(&ffi);

        assert!(matches!(
            library.native.functions[0].params[0].native_type,
            DartNativeType::Primitive(PrimitiveType::U64)
        ));

        assert_eq!(
            library.native.functions[0].params[0]
                .native_type
                .dart_sub_type(),
            "int".to_string()
        );
    }

    #[test]
    pub fn native_function_primitive_out() {
        let mut ffi = test::empty_contract();
        ffi.functions.insert(
            0,
            FunctionDef {
                qualified_path: String::new(),
                id: FunctionId::new("echo_f32"),
                params: vec![],
                returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::F32)),
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            },
        );
        let library = test::lower(&ffi);

        assert!(matches!(
            library.native.functions[0].return_type,
            DartNativeType::Primitive(PrimitiveType::F32)
        ));
        assert_eq!(
            library.native.functions[0].return_type.dart_sub_type(),
            "double".to_string()
        );
    }

    #[test]
    pub fn native_function_void_out() {
        let mut ffi = test::empty_contract();
        ffi.functions.insert(
            0,
            FunctionDef {
                qualified_path: String::new(),
                id: FunctionId::new("noop"),
                params: vec![],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            },
        );
        let library = test::lower(&ffi);

        assert!(matches!(
            library.native.functions[0].return_type,
            DartNativeType::Void,
        ));
        assert_eq!(
            library.native.functions[0].return_type.dart_sub_type(),
            "void".to_string()
        );
    }

    // Regression: an encoded param named `payload` derives its native ptr/len
    // names by suffixing the param's own name (`payloadPtr`/`payloadLen`,
    // pre-fix). A second, literally-named param `payload_ptr` lower-camel
    // cases to that exact same identifier, so the two params silently
    // collide in the generated native function's parameter list — Codex
    // review finding (2026-07-20).
    #[test]
    pub fn encoded_param_ptr_name_does_not_collide_with_a_literally_named_param() {
        let mut ffi = test::empty_contract();
        ffi.functions.insert(
            0,
            FunctionDef {
                qualified_path: String::new(),
                id: FunctionId::new("send"),
                params: vec![
                    ParamDef {
                        name: ParamName::new("payload"),
                        type_expr: TypeExpr::String,
                        passing: ParamPassing::Value,
                        doc: None,
                    },
                    ParamDef {
                        name: ParamName::new("payload_ptr"),
                        type_expr: TypeExpr::Primitive(PrimitiveType::U64),
                        passing: ParamPassing::Value,
                        doc: None,
                    },
                ],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            },
        );

        let library = test::lower(&ffi);

        let names: Vec<&str> = library.native.functions[0]
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "duplicate native param names, generated code will fail to compile: {names:?}"
        );
    }

    // Regression ("Fork bug 3", docs/tracks/dart.md): a class whose
    // constructor stores a callback-handle param (e.g.
    // `LiveQueryClient::new(transport, listener)`) may reenter it
    // synchronously from ANY of its methods later, including ones that take
    // no callback param themselves (`LiveQueryClient::disconnect`/`release`,
    // `WatchHandle::unsubscribe`) -- declaring those `isLeaf: true` aborts
    // the whole process the first time the reentrant call actually happens.
    #[test]
    pub fn method_on_a_class_whose_constructor_takes_a_callback_is_not_leaf() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_callback(CallbackTraitDef {
            qualified_path: String::new(),
            id: CallbackId::new("Listener"),
            methods: vec![crate::ir::CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: crate::ir::MethodId::new("on_event"),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: CallbackKind::Trait,
            doc: None,
        });
        ffi.catalog.insert_class(ClassDef {
            qualified_path: String::new(),
            id: ClassId::new("Connection"),
            constructors: vec![ConstructorDef::Default {
                params: vec![ParamDef {
                    name: ParamName::new("listener"),
                    type_expr: TypeExpr::Callback(CallbackId::new("Listener")),
                    passing: ParamPassing::BoxedDyn,
                    doc: None,
                }],
                is_fallible: false,
                is_optional: false,
                doc: None,
                deprecated: None,
            }],
            methods: vec![MethodDef {
                id: crate::ir::MethodId::new("disconnect"),
                receiver: Receiver::RefSelf,
                params: vec![],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            }],
            streams: vec![],
            doc: None,
            deprecated: None,
        });

        let library = test::lower(&ffi);
        let class = &library.classes[0];

        assert!(
            !class.constructors[0].native.is_leaf,
            "constructor takes the callback param directly, must not be leaf"
        );
        assert!(
            !class.methods[0].native.is_leaf,
            "sibling method takes no callback param itself but may reenter the \
             one the constructor stored -- must not be declared isLeaf"
        );
    }

    #[test]
    pub fn native_function_closure_in() {
        let mut ffi = test::empty_contract();
        ffi.catalog.insert_callback(CallbackTraitDef {
            qualified_path: String::new(),
            id: CallbackId::new("ClosureCb"),
            methods: vec![crate::ir::CallbackMethodDef {
                execution_kind: ExecutionKind::Sync,
                id: crate::ir::MethodId::new("call"),
                params: vec![],
                returns: ReturnDef::Void,
                doc: None,
            }],
            kind: CallbackKind::Closure,
            doc: None,
        });
        ffi.functions.insert(
            0,
            FunctionDef {
                qualified_path: String::new(),
                id: FunctionId::new("function_with_callback"),
                params: vec![ParamDef {
                    name: ParamName::new("cb"),
                    type_expr: TypeExpr::Callback(CallbackId::new("ClosureCb")),
                    passing: ParamPassing::ImplTrait,
                    doc: None,
                }],
                returns: ReturnDef::Void,
                execution_kind: ExecutionKind::Sync,
                doc: None,
                deprecated: None,
            },
        );
        let library = test::lower(&ffi);

        assert!(
            library.native.functions[0].params[0]
                .native_type
                .native_type()
                .contains("$$ffi.Pointer<$$ffi.NativeFunction<")
        );
        assert!(!library.native.functions[0].is_leaf);
    }

    #[test]
    pub fn native_function_async() {
        let mut ffi = test::empty_contract();
        ffi.functions.push(FunctionDef {
            qualified_path: String::new(),
            id: FunctionId::new("async_add"),
            params: vec![
                ParamDef {
                    name: ParamName::new("a"),
                    type_expr: TypeExpr::Primitive(PrimitiveType::I32),
                    passing: ParamPassing::Value,
                    doc: None,
                },
                ParamDef {
                    name: ParamName::new("b"),
                    type_expr: TypeExpr::Primitive(PrimitiveType::I32),
                    passing: ParamPassing::Value,
                    doc: None,
                },
            ],
            returns: ReturnDef::Value(TypeExpr::Primitive(PrimitiveType::I32)),
            execution_kind: ExecutionKind::Async,
            deprecated: None,
            doc: None,
        });

        let library = test::lower(&ffi);

        let func = &library.native.functions[0];
        match &func.call_mode {
            DartNativeFunctionCallMode::Sync => panic!("CallMode should be async"),
            DartNativeFunctionCallMode::Async {
                poll_symbol,
                complete_symbol,
                complete_ty,
                cancel_symbol,
                free_symbol,
            } => {
                assert_eq!(poll_symbol, "boltffi_async_add_poll");
                assert_eq!(complete_symbol, "boltffi_async_add_complete");
                assert!(matches!(
                    complete_ty,
                    DartNativeType::Primitive(PrimitiveType::I32)
                ));
                assert_eq!(complete_symbol, "boltffi_async_add_complete");
                assert_eq!(cancel_symbol, "boltffi_async_add_cancel");
                assert_eq!(free_symbol, "boltffi_async_add_free");
            }
        };
    }
}
