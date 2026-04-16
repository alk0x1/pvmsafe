use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemMod, parse_macro_input};

use crate::{reentrancy, refine, strip};

pub fn run(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut module = parse_macro_input!(item as ItemMod);
    run_on_module(&mut module).into()
}

pub fn run_on_module(module: &mut ItemMod) -> TokenStream2 {
    let mut errors = Vec::new();
    reentrancy::check_module(module, &mut errors);
    refine::check_module(module, &mut errors);
    strip::strip_pvmsafe_attrs(module);

    let mut out = TokenStream2::new();
    for err in errors {
        out.extend(err.to_compile_error());
    }
    out.extend(quote! { #module });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(src: &str) -> Vec<String> {
        let mut module: ItemMod = syn::parse_str(src).expect("parse");
        let mut errs = Vec::new();
        reentrancy::check_module(&module, &mut errs);
        refine::check_module(&module, &mut errs);
        strip::strip_pvmsafe_attrs(&mut module);
        errs.into_iter().map(|e| e.to_string()).collect()
    }

    fn output_contains_no_pvmsafe(src: &str) -> bool {
        let mut module: ItemMod = syn::parse_str(src).expect("parse");
        let out = run_on_module(&mut module);
        let s = out.to_string();
        !s.contains("pvmsafe ::") && !s.contains("pvmsafe::")
    }

    #[test]
    fn pipeline_emits_reentrancy_error() {
        let errs = errors(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe::externally] { let _ = 1; }
                    #[pvmsafe::locally] { let _ = 2; }
                }
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("reentrancy")));
    }

    #[test]
    fn pipeline_emits_refinement_error() {
        let errs = errors(
            r#"
            mod m {
                fn caller(x: u64) {
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("not provable")));
    }

    #[test]
    fn pipeline_emits_subtraction_error() {
        let errs = errors(
            r#"
            mod m {
                fn f(a: u64, b: u64) {
                    let _ = a - b;
                }
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("underflow")));
    }

    #[test]
    fn pipeline_emits_addition_error() {
        let errs = errors(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = x + y;
                }
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("overflow")));
    }

    #[test]
    fn pipeline_emits_division_error() {
        let errs = errors(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = x / y;
                }
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("divide by zero")));
    }

    #[test]
    fn pipeline_emits_ensures_error() {
        let errs = errors(
            r#"
            mod m {
                #[pvmsafe::ensures(v > 0)]
                fn bad() -> u64 { 0 }
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("ensures")));
    }

    #[test]
    fn pipeline_emits_let_refine_error() {
        let errs = errors(
            r#"
            mod m {
                fn f(x: u64) {
                    #[pvmsafe::refine(v > 0)]
                    let y = x;
                    let _ = y;
                }
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("let refinement")));
    }

    #[test]
    fn pipeline_emits_entrypoint_error() {
        let errs = errors(
            r#"
            mod m {
                #[pvm_contract_macros::method]
                pub fn transfer(amount: u64) {}
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("must carry")));
    }

    #[test]
    fn pipeline_strips_all_pvmsafe_attrs() {
        assert!(output_contains_no_pvmsafe(
            r#"
            mod m {
                #[pvmsafe::ensures(v > 0)]
                fn good() -> u64 { 1 }
                fn caller(#[pvmsafe::refine(x > 0)] x: u64) {
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        ));
    }

    #[test]
    fn pipeline_emits_conservation_error() {
        let errs = errors(
            r#"
            #[pvmsafe::invariant(conserves)]
            mod m {
                #[pvm_contract_macros::method]
                pub fn f(#[pvmsafe::refine(amount > 0)] amount: u64) {
                    #[pvmsafe::delta(-amount)]
                    a(amount);
                }
                fn a(x: u64) {}
            }
            "#,
        );
        assert!(errs.iter().any(|e| e.contains("conservation")), "{errs:?}");
    }

    #[test]
    fn pipeline_strips_invariant_and_delta_attrs() {
        assert!(output_contains_no_pvmsafe(
            r#"
            #[pvmsafe::invariant(conserves)]
            mod m {
                #[pvm_contract_macros::method]
                pub fn f(#[pvmsafe::refine(amount > 0)] amount: u64) {
                    #[pvmsafe::delta(-amount)]
                    a(amount);
                    #[pvmsafe::delta(amount)]
                    b(amount);
                }
                fn a(x: u64) {}
                fn b(x: u64) {}
            }
            "#,
        ));
    }

    #[test]
    fn pipeline_clean_code_produces_no_errors() {
        let errs = errors(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(x > 0)] x: u64) {
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }
}
