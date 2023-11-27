use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{parse_macro_input, spanned::Spanned, ItemFn};

#[proc_macro_attribute]
pub fn test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_fn = parse_macro_input!(item as ItemFn);

    if item_fn.sig.asyncness.is_none() {
        return TokenStream::from(quote_spanned! { item_fn.span() =>
            compile_error!("the async keyword is missing from the function declaration"),
        });
    }

    let fn_name = &item_fn.sig.ident;

    let test_name = item_fn.sig.ident.to_string();

    quote! {
        #[::core::prelude::v1::test]
        fn #fn_name() {
            #item_fn

            hala_io_test::socket_tester(#test_name,#fn_name);
        }


    }
    .into()
}
