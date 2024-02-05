use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Ident};
use uuid::Uuid;

#[proc_macro]
pub fn target(item: TokenStream) -> TokenStream {
    let event_ident = parse_macro_input!(item as Ident);

    let uuid = Uuid::new_v4().as_u128();

    quote! {
        pub static #event_ident: hala_tracing::Uuid = hala_tracing::Uuid::from_u128(#uuid);
    }
    .into()
}

#[proc_macro_attribute]
pub fn cpu_profiling(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
