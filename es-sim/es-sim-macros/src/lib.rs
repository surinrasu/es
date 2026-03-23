use proc_macro::TokenStream;

use quote::quote;
use syn::{parse_macro_input, Item};

#[proc_macro_attribute]
pub fn test(_args: TokenStream, item: TokenStream) -> TokenStream {
    let _ = parse_macro_input!(item as Item);
    TokenStream::new()
}

#[proc_macro_attribute]
pub fn export(_args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    quote!(
        #[allow(dead_code)]
        #item
    )
    .into()
}
