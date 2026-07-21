use boltffi_binding::{
    BindingMetadataEnvelope, BindingMetadataSection, BindingMetadataSurface, SerializedBindings,
};
use proc_macro2::{Literal, TokenStream};
use quote::quote;

use crate::experimental::error::Error;

pub fn render(bindings: SerializedBindings) -> Result<TokenStream, Error> {
    Ok(Static::new(bindings)?.render())
}

struct Static {
    envelope: BindingMetadataEnvelope,
    bytes: Vec<u8>,
}

impl Static {
    fn new(bindings: SerializedBindings) -> Result<Self, Error> {
        let envelope = BindingMetadataEnvelope::new(bindings)?;
        let bytes = envelope.to_section_bytes()?;
        Ok(Self { envelope, bytes })
    }

    fn render(&self) -> TokenStream {
        let cfg = self.cfg();
        let mach_o_section = BindingMetadataSection::MachO.link_section();
        let object_section = BindingMetadataSection::Object.link_section();
        let length = self.bytes.len();
        let bytes = Literal::byte_string(&self.bytes);

        quote! {
            #[allow(unexpected_cfgs)]
            const _: () = {
                #cfg
                #[cfg_attr(any(target_os = "macos", target_os = "ios"), unsafe(link_section = #mach_o_section))]
                #[cfg_attr(target_os = "windows", unsafe(link_section = #object_section))]
                #[cfg_attr(not(any(target_os = "macos", target_os = "ios", target_os = "windows")), unsafe(link_section = #object_section))]
                #[used]
                static __BOLTFFI_BINDINGS: [u8; #length] = *#bytes;
            };
        }
    }

    fn cfg(&self) -> TokenStream {
        match self.envelope.surface() {
            BindingMetadataSurface::Native => quote! {
                #[cfg(all(boltffi_metadata, not(target_arch = "wasm32")))]
            },
            BindingMetadataSurface::Wasm32 => quote! {
                #[cfg(all(boltffi_metadata, target_arch = "wasm32"))]
            },
        }
    }
}
