use boltffi_binding::CodecNode;
use proc_macro2::TokenStream;
use quote::quote;

use crate::experimental::{error::Error, expansion::Expansion, surface::RenderSurface};

pub struct Value<'expansion, 'lowered, S: RenderSurface> {
    codec: &'lowered CodecNode,
    expansion: &'expansion Expansion<'lowered, S>,
}

impl<'expansion, 'lowered, S: RenderSurface> Value<'expansion, 'lowered, S> {
    pub const fn new(
        codec: &'lowered CodecNode,
        expansion: &'expansion Expansion<'lowered, S>,
    ) -> Self {
        Self { codec, expansion }
    }

    pub fn buffer(&self, value: TokenStream) -> Result<TokenStream, Error> {
        super::require_runtime_wire(self.codec)?;
        let conversion = super::custom::Outgoing::new(self.codec, self.expansion);
        if !conversion.has_custom_conversion() {
            return Ok(quote! { ::boltffi::__private::FfiBuf::wire_encode(&#value) });
        }
        let value = conversion.convert(value)?;
        Ok(quote! {
            {
                let __boltffi_wire = #value;
                ::boltffi::__private::FfiBuf::wire_encode(&__boltffi_wire)
            }
        })
    }

    pub fn borrowed_buffer(&self, value: TokenStream) -> Result<TokenStream, Error> {
        super::require_runtime_wire(self.codec)?;
        let conversion = super::custom::BorrowedOutgoing::new(self.codec, self.expansion);
        if !conversion.has_custom_conversion() {
            return Ok(quote! { ::boltffi::__private::FfiBuf::wire_encode(&#value) });
        }
        let value = conversion.convert(value)?;
        Ok(quote! {
            {
                let __boltffi_wire = #value;
                ::boltffi::__private::FfiBuf::wire_encode(&__boltffi_wire)
            }
        })
    }
}
