use proc_macro::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse_macro_input, Ident, ItemFn, LitStr, Token};
use uuid::Uuid;

struct TargetInput {
    ident: Ident,
    #[allow(unused)]
    token: Token![,],
    doc: LitStr,
}

impl Parse for TargetInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            ident: input.parse()?,
            token: input.parse()?,
            doc: input.parse()?,
        })
    }
}

#[proc_macro]
pub fn def_target(item: TokenStream) -> TokenStream {
    let TargetInput {
        ident,
        token: _,
        doc,
    } = parse_macro_input!(item as TargetInput);

    let uuid = Uuid::new_v4().to_string();

    quote! {
        #[doc=#doc]
        pub static #ident: &'static str = #uuid;
    }
    .into()
}

#[proc_macro_attribute]
pub fn cpu_profiling(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = parse_macro_input!(item as ItemFn);

    quote! {
        #(#attrs)*
        #vis #sig {
            let instant = std::time::Instant::now();

            let r = #block;

            hala_pprof::profiler::cpu_profiler_sample(instant);

            return r
        }
    }
    .into()
}
