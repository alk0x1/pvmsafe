use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemMod, parse_macro_input};

pub fn run(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let module = parse_macro_input!(item as ItemMod);
    quote! { #module }.into()
}
