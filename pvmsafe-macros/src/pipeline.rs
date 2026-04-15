use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemMod, parse_macro_input};

use crate::{reentrancy, strip};

pub fn run(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut module = parse_macro_input!(item as ItemMod);
    let mut errors = Vec::new();
    reentrancy::check_module(&module, &mut errors);
    strip::strip_pvmsafe_attrs(&mut module);

    let mut out = TokenStream2::new();
    for err in errors {
        out.extend(err.to_compile_error());
    }
    out.extend(quote! { #module });
    out.into()
}
