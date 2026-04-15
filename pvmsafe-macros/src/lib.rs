use proc_macro::TokenStream;

mod pipeline;
mod reentrancy;
mod refine;
mod strip;

#[proc_macro_attribute]
pub fn contract(attr: TokenStream, item: TokenStream) -> TokenStream {
    pipeline::run(attr, item)
}
