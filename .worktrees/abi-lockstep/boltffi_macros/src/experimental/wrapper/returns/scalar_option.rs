use boltffi_binding::{Native, Primitive, Wasm32};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Type;

use crate::experimental::{
    error::Error,
    wrapper::{Render, names, returns::Tokens},
};

pub struct Renderer;
pub struct Failure;
pub struct FailureInput;
pub struct Empty;
pub struct Incoming;

pub struct Input {
    primitive: Primitive,
    value: syn::Ident,
}

pub struct IncomingInput {
    primitive: Primitive,
    rust_type: Type,
    value: TokenStream,
}

impl Input {
    pub fn new(primitive: Primitive, value: syn::Ident) -> Self {
        Self { primitive, value }
    }
}

impl IncomingInput {
    pub fn new(primitive: Primitive, rust_type: Type, value: TokenStream) -> Self {
        Self {
            primitive,
            rust_type,
            value,
        }
    }
}

impl Render<Native, Input> for Renderer {
    type Output = Tokens;

    fn render(self, input: Input) -> Result<Self::Output, Error> {
        let value = input.value;
        Ok(Tokens {
            items: Vec::new(),
            ffi_parameters: Vec::new(),
            return_type: quote! { -> ::boltffi::__private::FfiBuf },
            body: quote! {
                ::boltffi::__private::FfiBuf::wire_encode(&#value)
            },
        })
    }
}

impl Render<Wasm32, Input> for Renderer {
    type Output = Tokens;

    fn render(self, input: Input) -> Result<Self::Output, Error> {
        let value = input.value;
        let present = names::Locals::new(value.span()).value();
        let some = Scalar::new(input.primitive, present.clone()).tokens()?;
        Ok(Tokens {
            items: Vec::new(),
            ffi_parameters: Vec::new(),
            return_type: quote! { -> f64 },
            body: quote! {
                match #value {
                    Some(#present) => #some,
                    None => f64::NAN,
                }
            },
        })
    }
}

impl Render<Native, FailureInput> for Failure {
    type Output = TokenStream;

    fn render(self, _input: FailureInput) -> Result<Self::Output, Error> {
        let empty = <Renderer as Render<Native, Empty>>::render(Renderer, Empty)?;
        let body = empty.body();
        Ok(quote! {
            return #body;
        })
    }
}

impl Render<Wasm32, FailureInput> for Failure {
    type Output = TokenStream;

    fn render(self, _input: FailureInput) -> Result<Self::Output, Error> {
        let empty = <Renderer as Render<Wasm32, Empty>>::render(Renderer, Empty)?;
        let body = empty.body();
        Ok(quote! {
            return #body;
        })
    }
}

impl Render<Native, Empty> for Renderer {
    type Output = Tokens;

    fn render(self, _input: Empty) -> Result<Self::Output, Error> {
        Ok(Tokens {
            items: Vec::new(),
            ffi_parameters: Vec::new(),
            return_type: quote! { -> ::boltffi::__private::FfiBuf },
            body: quote! { ::boltffi::__private::FfiBuf::default() },
        })
    }
}

impl Render<Wasm32, Empty> for Renderer {
    type Output = Tokens;

    fn render(self, _input: Empty) -> Result<Self::Output, Error> {
        Ok(Tokens {
            items: Vec::new(),
            ffi_parameters: Vec::new(),
            return_type: quote! { -> f64 },
            body: quote! { f64::NAN },
        })
    }
}

impl Render<Native, IncomingInput> for Incoming {
    type Output = TokenStream;

    fn render(self, input: IncomingInput) -> Result<Self::Output, Error> {
        let rust_type = input.rust_type;
        let value = input.value;
        Ok(quote! {
            {
                let __boltffi_result = #value;
                match ::boltffi::__private::wire::decode::<#rust_type>(unsafe {
                    __boltffi_result.as_byte_slice()
                }) {
                    Ok(__boltffi_value) => __boltffi_value,
                    Err(error) => {
                        panic!("callback method optional scalar return conversion failed: {:?}", error)
                    }
                }
            }
        })
    }
}

impl Render<Wasm32, IncomingInput> for Incoming {
    type Output = TokenStream;

    fn render(self, input: IncomingInput) -> Result<Self::Output, Error> {
        let value = input.value;
        let result = names::Locals::new(Span::call_site()).result();
        let some = Scalar::new(input.primitive, result.clone()).tokens()?;
        Ok(quote! {
            {
                let #result = #value;
                if #result.is_nan() {
                    None
                } else {
                    Some(#some)
                }
            }
        })
    }
}

pub struct Scalar {
    primitive: Primitive,
    value: syn::Ident,
}

impl Scalar {
    pub fn new(primitive: Primitive, value: syn::Ident) -> Self {
        Self { primitive, value }
    }

    pub fn tokens(self) -> Result<TokenStream, Error> {
        let value = self.value;
        Ok(match self.primitive {
            Primitive::Bool => quote! {
                if #value { 1.0 } else { 0.0 }
            },
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
            | Primitive::F32 => quote! {
                #value as f64
            },
            _ => return Err(Error::UnsupportedExpansion("scalar option primitive")),
        })
    }
}
