use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{Attribute, Block, Error, Expr, Item, ItemMod, Stmt};

#[derive(Clone, Copy)]
enum BlockKind {
    Locally,
    Externally,
}

pub fn check_module(module: &ItemMod, errors: &mut Vec<Error>) {
    let Some((_, items)) = &module.content else {
        return;
    };
    for item in items {
        if let Item::Fn(f) = item {
            if !has_allow_reentrancy(&f.attrs) {
                check_block(&f.block, errors);
            }
        }
    }
}

fn has_allow_reentrancy(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let segs: Vec<_> = attr.path().segments.iter().collect();
        let is_allow = matches!(
            segs.as_slice(),
            [ns, name]
                if (ns.ident == "pvmsafe" || ns.ident == "pvmsafe_macros")
                    && name.ident == "allow"
        );
        if !is_allow {
            return false;
        }
        attr.parse_args::<syn::Ident>()
            .map(|id| id == "reentrancy")
            .unwrap_or(false)
    })
}

fn check_block(block: &Block, errors: &mut Vec<Error>) {
    let mut first_externally: Option<Span> = None;
    for stmt in &block.stmts {
        let Some((kind, span)) = classify(stmt) else {
            continue;
        };
        match kind {
            BlockKind::Externally => {
                if first_externally.is_none() {
                    first_externally = Some(span);
                }
            }
            BlockKind::Locally => {
                if let Some(earlier) = first_externally {
                    let mut err = Error::new(
                        span,
                        "pvmsafe: `locally` block (state mutation) appears after an \
                         `externally` block (external call); reentrancy risk",
                    );
                    err.combine(Error::new(earlier, "note: earlier `externally` block here"));
                    errors.push(err);
                }
            }
        }
    }
}

fn classify(stmt: &Stmt) -> Option<(BlockKind, Span)> {
    let expr_block = match stmt {
        Stmt::Expr(Expr::Block(b), _) => b,
        _ => return None,
    };
    classify_attrs(&expr_block.attrs).map(|k| (k, expr_block.span()))
}

fn classify_attrs(attrs: &[Attribute]) -> Option<BlockKind> {
    for attr in attrs {
        let segs: Vec<_> = attr.path().segments.iter().collect();
        let name = match segs.as_slice() {
            [ns, name] if ns.ident == "pvmsafe" || ns.ident == "pvmsafe_macros" => {
                name.ident.to_string()
            }
            _ => continue,
        };
        match name.as_str() {
            "locally" => return Some(BlockKind::Locally),
            "externally" => return Some(BlockKind::Externally),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> Vec<String> {
        let module: ItemMod = syn::parse_str(src).expect("parse module");
        let mut errors = Vec::new();
        check_module(&module, &mut errors);
        errors.into_iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn accepts_locally_then_externally() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe::locally] { let _ = 1; }
                    #[pvmsafe::externally] { let _ = 2; }
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn rejects_externally_then_locally() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe::externally] { let _ = 1; }
                    #[pvmsafe::locally] { let _ = 2; }
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("after an `externally` block"));
    }

    #[test]
    fn rejects_locally_after_externally_interleaved() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe::locally] { let _ = 1; }
                    #[pvmsafe::externally] { let _ = 2; }
                    #[pvmsafe::locally] { let _ = 3; }
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn accepts_pvmsafe_macros_prefix() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe_macros::locally] { let _ = 1; }
                    #[pvmsafe_macros::externally] { let _ = 2; }
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn allow_reentrancy_skips_check() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::allow(reentrancy)]
                fn f() {
                    #[pvmsafe::externally] { let _ = 1; }
                    #[pvmsafe::locally] { let _ = 2; }
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn allow_reentrancy_only_affects_annotated_fn() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::allow(reentrancy)]
                fn safe() {
                    #[pvmsafe::externally] { let _ = 1; }
                    #[pvmsafe::locally] { let _ = 2; }
                }
                fn unsafe_fn() {
                    #[pvmsafe::externally] { let _ = 1; }
                    #[pvmsafe::locally] { let _ = 2; }
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn allow_other_rule_does_not_skip_reentrancy() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::allow(something_else)]
                fn f() {
                    #[pvmsafe::externally] { let _ = 1; }
                    #[pvmsafe::locally] { let _ = 2; }
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn ignores_plain_blocks() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    { let _ = 1; }
                    { let _ = 2; }
                }
            }
            "#,
        );
        assert!(errs.is_empty());
    }
}
