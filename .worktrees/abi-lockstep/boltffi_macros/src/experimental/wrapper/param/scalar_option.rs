use boltffi_binding::{Native, Primitive, Wasm32};
use quote::quote;
use syn::{Ident, Type};

use crate::experimental::{
    error::Error,
    wrapper::{Render, names},
};

use super::Tokens;

pub struct Renderer;

pub struct Input {
    primitive: Primitive,
    rust_type: Type,
    ident: Ident,
    failure: proc_macro2::TokenStream,
}

impl Input {
    pub fn new(
        primitive: Primitive,
        rust_type: Type,
        ident: Ident,
        failure: proc_macro2::TokenStream,
    ) -> Self {
        Self {
            primitive,
            rust_type,
            ident,
            failure,
        }
    }
}

impl Render<Native, Input> for Renderer {
    type Output = Tokens;

    fn render(self, input: Input) -> Result<Self::Output, Error> {
        let ident = &input.ident;
        let locals = names::Parameter::new(ident);
        let pointer = locals.pointer();
        let length = locals.length();
        let rust_type = &input.rust_type;
        let failure = input.failure;
        Ok(Tokens {
            items: Vec::new(),
            ffi_parameters: vec![quote! { #pointer: *const u8 }, quote! { #length: usize }],
            ffi_parameter_types: vec![quote! { *const u8 }, quote! { usize }],
            conversions: vec![quote! {
                let #ident: #rust_type = if #pointer.is_null() {
                    None
                } else {
                    match ::boltffi::__private::wire::decode::<#rust_type>(unsafe {
                        ::core::slice::from_raw_parts(#pointer, #length)
                    }) {
                        Ok(value) => value,
                        Err(error) => {
                            ::boltffi::__private::set_last_error(format!(
                                "{}: invalid optional scalar payload: {} (buf_len={})",
                                stringify!(#ident),
                                error,
                                #length
                            ));
                            #failure
                        }
                    }
                };
            }],
            writebacks: Vec::new(),
            argument: quote! { #ident },
        })
    }
}

impl Render<Wasm32, Input> for Renderer {
    type Output = Tokens;

    fn render(self, input: Input) -> Result<Self::Output, Error> {
        let ident = &input.ident;
        let rust_type = &input.rust_type;
        let value = Scalar::new(input.primitive, ident.clone()).some_value()?;
        Ok(Tokens {
            items: Vec::new(),
            ffi_parameters: vec![quote! { #ident: f64 }],
            ffi_parameter_types: vec![quote! { f64 }],
            conversions: vec![quote! {
                let #ident: #rust_type = if #ident.is_nan() {
                    None
                } else {
                    Some(#value)
                };
            }],
            writebacks: Vec::new(),
            argument: quote! { #ident },
        })
    }
}

struct Scalar {
    primitive: Primitive,
    value: Ident,
}

impl Scalar {
    fn new(primitive: Primitive, value: Ident) -> Self {
        Self { primitive, value }
    }

    fn some_value(self) -> Result<proc_macro2::TokenStream, Error> {
        let value = self.value;
        Ok(match self.primitive {
            Primitive::Bool => quote! { #value != 0.0 },
            Primitive::F64 => quote! { #value },
            Primitive::I8
            | Primitive::U8
            | Primitive::I16
            | Primitive::U16
            | Primitive::I32
            | Primitive::U32
            | Primitive::I64
            | Primitive::U64
            | Primitive::ISize
            | Primitive::USize
            | Primitive::F32 => quote! { #value as _ },
            _ => return Err(Error::UnsupportedExpansion("scalar option primitive")),
        })
    }
}
