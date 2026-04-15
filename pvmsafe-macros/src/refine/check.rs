use super::fm::{self, FmError};
use super::lir::Constraint;
use super::translate::translate_predicate;
use std::collections::HashMap;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::visit_mut::{self, VisitMut};
use syn::{Attribute, Error, Expr, ExprCall, ExprIf, ExprPath, FnArg, Item, ItemFn, ItemMod, Pat};

struct MethodInfo {
    params: Vec<String>,
    refinements: Vec<(String, Expr)>,
}

pub fn check_module(module: &ItemMod, errors: &mut Vec<Error>) {
    let Some((_, items)) = &module.content else {
        return;
    };

    for item in items {
        if let Item::Fn(f) = item {
            check_entrypoint_coverage(f, errors);
        }
    }

    let mut methods: HashMap<String, MethodInfo> = HashMap::new();
    for item in items {
        if let Item::Fn(f) = item {
            methods.insert(f.sig.ident.to_string(), method_info(f));
        }
    }

    for item in items {
        let Item::Fn(f) = item else { continue };
        check_fn(f, &methods, errors);
    }
}

fn is_entrypoint(f: &ItemFn) -> bool {
    f.attrs.iter().any(|attr| {
        let segs: Vec<_> = attr.path().segments.iter().collect();
        matches!(
            segs.as_slice(),
            [ns, name]
                if ns.ident == "pvm_contract_macros"
                    && (name.ident == "method"
                        || name.ident == "constructor"
                        || name.ident == "fallback")
        )
    })
}

fn is_pvmsafe_unchecked(attr: &Attribute) -> bool {
    let segs: Vec<_> = attr.path().segments.iter().collect();
    matches!(
        segs.as_slice(),
        [ns, name]
            if (ns.ident == "pvmsafe" || ns.ident == "pvmsafe_macros")
                && name.ident == "unchecked"
    )
}

fn check_entrypoint_coverage(f: &ItemFn, errors: &mut Vec<Error>) {
    if !is_entrypoint(f) {
        return;
    }
    let fn_name = f.sig.ident.to_string();
    for input in &f.sig.inputs {
        let FnArg::Typed(pt) = input else { continue };
        let has_refine = extract_refine(&pt.attrs).is_some();
        let has_unchecked = pt.attrs.iter().any(is_pvmsafe_unchecked);
        if has_refine || has_unchecked {
            continue;
        }
        let param = param_name(&pt.pat);
        errors.push(Error::new(
            pt.span(),
            format!(
                "pvmsafe: parameter `{param}` on entrypoint `{fn_name}` must carry \
                 `#[pvmsafe::refine(...)]` or `#[pvmsafe::unchecked]`"
            ),
        ));
    }
}

fn method_info(f: &ItemFn) -> MethodInfo {
    let mut params = Vec::new();
    let mut refinements = Vec::new();
    for input in &f.sig.inputs {
        let FnArg::Typed(pt) = input else { continue };
        let name = param_name(&pt.pat);
        params.push(name.clone());
        if let Some(pred) = extract_refine(&pt.attrs) {
            refinements.push((name, pred));
        }
    }
    MethodInfo { params, refinements }
}

fn param_name(pat: &Pat) -> String {
    match pat {
        Pat::Ident(id) => id.ident.to_string(),
        other => quote::ToTokens::to_token_stream(other).to_string(),
    }
}

fn extract_refine(attrs: &[Attribute]) -> Option<Expr> {
    for attr in attrs {
        let segs: Vec<_> = attr.path().segments.iter().collect();
        let is_refine = matches!(
            segs.as_slice(),
            [ns, name]
                if (ns.ident == "pvmsafe" || ns.ident == "pvmsafe_macros")
                    && name.ident == "refine"
        );
        if !is_refine {
            continue;
        }
        if let Ok(expr) = attr.parse_args::<Expr>() {
            return Some(expr);
        }
    }
    None
}

fn check_fn(
    f: &ItemFn,
    methods: &HashMap<String, MethodInfo>,
    errors: &mut Vec<Error>,
) {
    let name = f.sig.ident.to_string();
    let info = methods.get(&name);

    let mut assumptions: Vec<Constraint> = Vec::new();
    if let Some(info) = info {
        for (_param, pred) in &info.refinements {
            match translate_predicate(pred) {
                Ok(cs) => assumptions.extend(cs),
                Err(e) => errors.push(Error::new(
                    pred.span(),
                    format!("pvmsafe: cannot translate refinement: {e}"),
                )),
            }
        }
    }

    let mut walker = CallWalker {
        methods,
        assumptions,
        errors,
    };
    walker.visit_block(&f.block);
}

struct CallWalker<'a> {
    methods: &'a HashMap<String, MethodInfo>,
    assumptions: Vec<Constraint>,
    errors: &'a mut Vec<Error>,
}

impl<'ast, 'a> Visit<'ast> for CallWalker<'a> {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(ExprPath { path, .. }) = &*call.func {
            if let Some(last) = path.segments.last() {
                if let Some(info) = self.methods.get(&last.ident.to_string()) {
                    if !info.refinements.is_empty() {
                        self.check_call(call, info);
                    }
                }
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_if(&mut self, expr_if: &'ast ExprIf) {
        let added: Vec<Constraint> = translate_predicate(&expr_if.cond).unwrap_or_default();
        let snapshot = self.assumptions.clone();

        self.assumptions.extend(added.iter().cloned());
        self.visit_block(&expr_if.then_branch);
        self.assumptions = snapshot.clone();

        if let Some((_, else_expr)) = &expr_if.else_branch {
            if added.len() == 1 {
                if let Some(neg) = fm::negate(&added[0]) {
                    self.assumptions.push(neg);
                }
            }
            self.visit_expr(else_expr);
            self.assumptions = snapshot;
        }
    }

    fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
        if let Some(name) = lhs_ident(&assign.left) {
            self.drop_var(&name);
        }
        visit::visit_expr_assign(self, assign);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(name) = local_ident(&local.pat) {
            self.drop_var(&name);
        }
        visit::visit_local(self, local);
    }
}

fn lhs_ident(e: &Expr) -> Option<String> {
    if let Expr::Path(ExprPath { path, .. }) = e {
        if let Some(id) = path.get_ident() {
            return Some(id.to_string());
        }
    }
    None
}

fn local_ident(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(id) => Some(id.ident.to_string()),
        Pat::Type(pt) => local_ident(&pt.pat),
        _ => None,
    }
}

impl<'a> CallWalker<'a> {
    fn drop_var(&mut self, name: &str) {
        self.assumptions
            .retain(|c| !c.expr.terms.contains_key(name));
    }

    fn check_call(&mut self, call: &ExprCall, info: &MethodInfo) {
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        for (param, arg) in info.params.iter().zip(call.args.iter()) {
            bindings.insert(param.clone(), arg.clone());
        }

        for (_param, pred) in &info.refinements {
            let mut substituted = pred.clone();
            Substitute { bindings: &bindings }.visit_expr_mut(&mut substituted);

            let goals = match translate_predicate(&substituted) {
                Ok(cs) => cs,
                Err(e) => {
                    self.errors.push(Error::new(
                        call.span(),
                        format!(
                            "pvmsafe: cannot translate refinement `{}` at call site: {e}",
                            quote::ToTokens::to_token_stream(pred)
                        ),
                    ));
                    continue;
                }
            };

            for goal in goals {
                match fm::entails(&self.assumptions, &goal) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.errors.push(Error::new(
                            call.span(),
                            format!(
                                "pvmsafe: refinement `{}` not provable from caller's assumptions",
                                quote::ToTokens::to_token_stream(pred)
                            ),
                        ));
                    }
                    Err(FmError::Overflow) => {
                        self.errors.push(Error::new(
                            call.span(),
                            "pvmsafe: refinement check exceeded Fourier-Motzkin complexity bound",
                        ));
                    }
                }
            }
        }
    }
}

struct Substitute<'a> {
    bindings: &'a HashMap<String, Expr>,
}

impl<'a> VisitMut for Substitute<'a> {
    fn visit_expr_mut(&mut self, e: &mut Expr) {
        if let Expr::Path(p) = e {
            if let Some(ident) = p.path.get_ident() {
                if let Some(replacement) = self.bindings.get(&ident.to_string()) {
                    *e = replacement.clone();
                    return;
                }
            }
        }
        visit_mut::visit_expr_mut(self, e);
    }
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
    fn accepts_call_where_refinement_is_proved() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(amount > 0)] amount: u64) {
                    callee(amount);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn rejects_call_without_caller_assumption() {
        let errs = check(
            r#"
            mod m {
                fn caller(amount: u64) {
                    callee(amount);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn accepts_literal_argument_satisfying_refinement() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    callee(5);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn rejects_literal_argument_violating_refinement() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    callee(0);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn proves_weaker_from_stronger_at_call_site() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(a >= 10)] a: u64) {
                    callee(a);
                }
                fn callee(#[pvmsafe::refine(x >= 1)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ignores_calls_to_unrefined_methods() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    helper(0);
                }
                fn helper(x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty());
    }

    #[test]
    fn entrypoint_with_refined_int_and_unchecked_address_is_ok() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::method]
                pub fn transfer(
                    #[pvmsafe::unchecked] to: Address,
                    #[pvmsafe::refine(amount > 0)] amount: U256,
                ) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn entrypoint_with_bare_int_param_errors_with_param_name() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::method]
                pub fn transfer(amount: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("`amount`"), "{errs:?}");
        assert!(errs[0].contains("`transfer`"), "{errs:?}");
        assert!(errs[0].contains("pvmsafe::refine"), "{errs:?}");
        assert!(errs[0].contains("pvmsafe::unchecked"), "{errs:?}");
    }

    #[test]
    fn entrypoint_with_unchecked_everywhere_is_ok() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::method]
                pub fn f(
                    #[pvmsafe::unchecked] a: u64,
                    #[pvmsafe::unchecked] b: Address,
                ) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn constructor_with_no_params_is_ok() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::constructor]
                pub fn new() {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn fallback_with_no_params_is_ok() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::fallback]
                pub fn fallback() {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn non_entrypoint_helper_with_bare_params_is_ok() {
        let errs = check(
            r#"
            mod m {
                fn helper(a: u64, b: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn each_bare_param_produces_its_own_error() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::method]
                pub fn f(a: u64, b: u64, c: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 3);
        assert!(errs.iter().any(|e| e.contains("`a`")));
        assert!(errs.iter().any(|e| e.contains("`b`")));
        assert!(errs.iter().any(|e| e.contains("`c`")));
    }

    #[test]
    fn partial_coverage_only_errors_on_missing_param() {
        let errs = check(
            r#"
            mod m {
                #[pvm_contract_macros::method]
                pub fn f(
                    #[pvmsafe::refine(a > 0)] a: u64,
                    b: u64,
                ) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("`b`"));
    }

    #[test]
    fn if_condition_discharges_refinement_in_then_branch() {
        let errs = check(
            r#"
            mod m {
                fn caller(amount: u64) {
                    if amount > 0 {
                        callee(amount);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn else_branch_uses_negated_condition() {
        let errs = check(
            r#"
            mod m {
                fn caller(amount: u64) {
                    if amount <= 0 {
                    } else {
                        callee(amount);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn assumption_does_not_leak_past_if() {
        let errs = check(
            r#"
            mod m {
                fn caller(amount: u64) {
                    if amount > 0 {
                    }
                    callee(amount);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn nested_ifs_accumulate_assumptions() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a > 0 {
                        if b > 0 {
                            callee(a, b);
                        }
                    }
                }
                fn callee(
                    #[pvmsafe::refine(x > 0)] x: u64,
                    #[pvmsafe::refine(y > 0)] y: u64,
                ) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn inner_if_scope_does_not_leak_to_sibling() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a > 0 {
                    }
                    if b > 0 {
                        callee(b);
                    }
                    callee(a);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn untranslatable_condition_is_ignored_soundly() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a * b > 0 {
                        callee(a);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn conjunctive_condition_both_facts_available() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a > 0 && b > 0 {
                        callee(a, b);
                    }
                }
                fn callee(
                    #[pvmsafe::refine(x > 0)] x: u64,
                    #[pvmsafe::refine(y > 0)] y: u64,
                ) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn disjunctive_condition_not_negated_in_else() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a > 0 && b > 0 {
                    } else {
                        callee(a);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn reassignment_invalidates_assumption() {
        let errs = check(
            r#"
            mod m {
                fn caller(mut amount: u64) {
                    if amount > 0 {
                        amount = 0;
                        callee(amount);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn reassignment_does_not_affect_other_vars() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, mut b: u64) {
                    if a > 0 {
                        b = 0;
                        callee(a);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn let_shadowing_invalidates_prior_refinement() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(x > 0)] x: u64) {
                    let x: u64 = 0;
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn mutation_inside_then_branch_does_not_leak_to_sibling() {
        let errs = check(
            r#"
            mod m {
                fn caller(mut amount: u64) {
                    if amount > 0 {
                        amount = 0;
                    }
                    if amount > 0 {
                        callee(amount);
                    }
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn path_sensitivity_chains_through_transitivity() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64) {
                    if a >= 10 {
                        callee(a);
                    }
                }
                fn callee(#[pvmsafe::refine(x >= 1)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }
}
