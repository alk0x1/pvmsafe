use proc_macro::TokenStream;

mod pipeline;

#[proc_macro_attribute]
pub fn contract(attr: TokenStream, item: TokenStream) -> TokenStream {
    pipeline::run(attr, item)
}
