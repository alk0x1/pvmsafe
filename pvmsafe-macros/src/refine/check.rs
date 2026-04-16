use super::fm::{self, FmError};
use super::lir::Constraint;
use super::translate::{translate_predicate, translate_term};
use std::collections::HashMap;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, Block, Error, Expr, ExprCall, ExprIf, ExprPath, FnArg, Item, ItemFn, ItemMod, Pat,
    Stmt,
};

struct MethodInfo {
    params: Vec<String>,
    refinements: Vec<(String, Expr)>,
    ensures: Option<Expr>,
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
    let ensures = extract_ensures(&f.attrs);
    MethodInfo {
        params,
        refinements,
        ensures,
    }
}

fn extract_ensures(attrs: &[Attribute]) -> Option<Expr> {
    for attr in attrs {
        let segs: Vec<_> = attr.path().segments.iter().collect();
        let is_ensures = matches!(
            segs.as_slice(),
            [ns, name]
                if (ns.ident == "pvmsafe" || ns.ident == "pvmsafe_macros")
                    && name.ident == "ensures"
        );
        if !is_ensures {
            continue;
        }
        if let Ok(expr) = attr.parse_args::<Expr>() {
            return Some(expr);
        }
    }
    None
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

    let ensures = info.and_then(|i| i.ensures.as_ref());

    let mut walker = CallWalker {
        methods,
        assumptions,
        ensures,
        in_given: false,
        errors,
    };
    walker.visit_block(&f.block);
}

struct CallWalker<'a> {
    methods: &'a HashMap<String, MethodInfo>,
    assumptions: Vec<Constraint>,
    ensures: Option<&'a Expr>,
    in_given: bool,
    errors: &'a mut Vec<Error>,
}

impl<'ast, 'a> Visit<'ast> for CallWalker<'a> {
    fn visit_block(&mut self, block: &'ast Block) {
        let snapshot = self.assumptions.clone();

        for stmt in &block.stmts {
            self.visit_stmt(stmt);

            if let Stmt::Expr(Expr::If(expr_if), _) = stmt {
                if expr_if.else_branch.is_none() && block_diverges(&expr_if.then_branch) {
                    for c in translate_predicate(&expr_if.cond).unwrap_or_default() {
                        if let Some(neg) = fm::negate(&c) {
                            self.assumptions.push(neg);
                        }
                    }
                }
            }
        }

        if let Some(ensures) = self.ensures {
            if let Some(Stmt::Expr(expr, None)) = block.stmts.last() {
                self.check_ensures(ensures, expr, expr.span());
            }
        }

        self.assumptions = snapshot;
    }

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

    fn visit_expr(&mut self, e: &'ast Expr) {
        let given = extract_given(e);
        if given.is_empty() {
            visit::visit_expr(self, e);
            return;
        }
        let snapshot = self.assumptions.clone();
        let was_in_given = self.in_given;
        self.assumptions.extend(given);
        self.in_given = true;
        visit::visit_expr(self, e);
        self.assumptions = snapshot;
        self.in_given = was_in_given;
    }

    fn visit_expr_return(&mut self, ret: &'ast syn::ExprReturn) {
        if let (Some(ensures), Some(expr)) = (self.ensures, &ret.expr) {
            self.check_ensures(ensures, expr, ret.span());
        }
        visit::visit_expr_return(self, ret);
    }

    fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
        if matches!(b.op, syn::BinOp::Sub(_)) {
            self.check_sub_safety(b);
        }
        if matches!(b.op, syn::BinOp::Add(_) | syn::BinOp::Mul(_)) {
            self.check_overflow_safety(b);
        }
        if matches!(b.op, syn::BinOp::Div(_) | syn::BinOp::Rem(_)) {
            self.check_div_safety(b);
        }
        visit::visit_expr_binary(self, b);
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
        self.inject_ensures(local);
        self.check_local_refine(local);
    }
}

fn extract_given(e: &Expr) -> Vec<Constraint> {
    let attrs: &[Attribute] = match e {
        Expr::Call(c) => &c.attrs,
        Expr::MethodCall(c) => &c.attrs,
        Expr::Block(b) => &b.attrs,
        Expr::If(i) => &i.attrs,
        Expr::Binary(b) => &b.attrs,
        Expr::Paren(p) => &p.attrs,
        _ => return Vec::new(),
    };
    for attr in attrs {
        let segs: Vec<_> = attr.path().segments.iter().collect();
        let is_given = matches!(
            segs.as_slice(),
            [ns, name]
                if (ns.ident == "pvmsafe" || ns.ident == "pvmsafe_macros")
                    && name.ident == "given"
        );
        if !is_given {
            continue;
        }
        if let Ok(pred) = attr.parse_args::<Expr>() {
            if let Ok(cs) = translate_predicate(&pred) {
                return cs;
            }
        }
    }
    Vec::new()
}

fn block_diverges(block: &Block) -> bool {
    match block.stmts.last() {
        Some(Stmt::Expr(expr, _)) => expr_diverges(expr),
        _ => false,
    }
}

fn expr_diverges(expr: &Expr) -> bool {
    match expr {
        Expr::Return(_) | Expr::Break(_) | Expr::Continue(_) => true,
        Expr::Block(b) => block_diverges(&b.block),
        _ => false,
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
    fn check_ensures(&mut self, ensures: &Expr, ret_expr: &Expr, span: proc_macro2::Span) {
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        bindings.insert("v".to_string(), ret_expr.clone());
        let mut substituted = ensures.clone();
        Substitute { bindings: &bindings }.visit_expr_mut(&mut substituted);

        let goals = match translate_predicate(&substituted) {
            Ok(cs) => cs,
            Err(_) => return,
        };

        for goal in goals {
            match fm::entails(&self.assumptions, &goal) {
                Ok(true) => {}
                Ok(false) => {
                    self.errors.push(Error::new(
                        span,
                        format!(
                            "pvmsafe: ensures `{}` not provable at return site",
                            quote::ToTokens::to_token_stream(ensures)
                        ),
                    ));
                }
                Err(FmError::Overflow) => {
                    self.errors.push(Error::new(
                        span,
                        "pvmsafe: ensures check exceeded Fourier-Motzkin complexity bound",
                    ));
                }
            }
        }
    }

    fn check_overflow_safety(&mut self, b: &syn::ExprBinary) {
        if self.in_given {
            return;
        }
        if matches!(*b.left, Expr::Lit(_)) && matches!(*b.right, Expr::Lit(_)) {
            return;
        }
        if translate_term(&b.left).is_err() || translate_term(&b.right).is_err() {
            return;
        }
        let (op, checked, saturating) = match b.op {
            syn::BinOp::Add(_) => ("+", "checked_add", "saturating_add"),
            syn::BinOp::Mul(_) => ("*", "checked_mul", "saturating_mul"),
            _ => return,
        };
        self.errors.push(Error::new(
            b.span(),
            format!(
                "pvmsafe: `{} {} {}` may overflow; use `{}` or `{}`",
                quote::ToTokens::to_token_stream(&b.left),
                op,
                quote::ToTokens::to_token_stream(&b.right),
                checked,
                saturating,
            ),
        ));
    }

    fn check_div_safety(&mut self, b: &syn::ExprBinary) {
        if self.in_given {
            return;
        }
        let Ok(rhs) = translate_term(&b.right) else { return };
        let Some(neg_rhs) = rhs.neg() else { return };
        let Some(goal_expr) = neg_rhs.add(&super::lir::LinearExpr::constant(1)) else { return };
        let goal = Constraint::new(goal_expr);
        let op = if matches!(b.op, syn::BinOp::Div(_)) { "/" } else { "%" };
        match fm::entails(&self.assumptions, &goal) {
            Ok(true) => {}
            Ok(false) => {
                self.errors.push(Error::new(
                    b.span(),
                    format!(
                        "pvmsafe: `{} {} {}` may divide by zero; \
                         divisor not provably non-zero",
                        quote::ToTokens::to_token_stream(&b.left),
                        op,
                        quote::ToTokens::to_token_stream(&b.right),
                    ),
                ));
            }
            Err(FmError::Overflow) => {
                self.errors.push(Error::new(
                    b.span(),
                    "pvmsafe: division-by-zero check exceeded Fourier-Motzkin complexity bound",
                ));
            }
        }
    }

    fn check_sub_safety(&mut self, b: &syn::ExprBinary) {
        let Ok(lhs) = translate_term(&b.left) else { return };
        let Ok(rhs) = translate_term(&b.right) else { return };
        let Some(diff) = rhs.sub(&lhs) else { return };
        let goal = Constraint::new(diff);
        match fm::entails(&self.assumptions, &goal) {
            Ok(true) => {}
            Ok(false) => {
                self.errors.push(Error::new(
                    b.span(),
                    format!(
                        "pvmsafe: subtraction `{} - {}` may underflow; not provable from caller's assumptions",
                        quote::ToTokens::to_token_stream(&b.left),
                        quote::ToTokens::to_token_stream(&b.right),
                    ),
                ));
            }
            Err(FmError::Overflow) => {
                self.errors.push(Error::new(
                    b.span(),
                    "pvmsafe: underflow check exceeded Fourier-Motzkin complexity bound",
                ));
            }
        }
    }

    fn inject_ensures(&mut self, local: &syn::Local) {
        let bind_name = match local_ident(&local.pat) {
            Some(n) => n,
            None => return,
        };
        let init = match &local.init {
            Some(init) => init,
            None => return,
        };
        let init_expr = match &*init.expr {
            Expr::Try(t) => &*t.expr,
            other => other,
        };
        let Expr::Call(call) = init_expr else { return };
        let Expr::Path(ExprPath { path, .. }) = &*call.func else { return };
        let Some(last) = path.segments.last() else { return };
        let Some(info) = self.methods.get(&last.ident.to_string()) else { return };
        let Some(ensures) = &info.ensures else { return };

        let mut bindings: HashMap<String, Expr> = HashMap::new();
        bindings.insert("v".to_string(), syn::parse_str::<Expr>(&bind_name).unwrap());
        for (param, arg) in info.params.iter().zip(call.args.iter()) {
            bindings.insert(param.clone(), arg.clone());
        }

        let mut substituted = ensures.clone();
        Substitute { bindings: &bindings }.visit_expr_mut(&mut substituted);

        if let Ok(cs) = translate_predicate(&substituted) {
            self.assumptions.extend(cs);
        }
    }

    fn check_local_refine(&mut self, local: &syn::Local) {
        let pred = match extract_refine(&local.attrs) {
            Some(p) => p,
            None => return,
        };
        let bind_name = match local_ident(&local.pat) {
            Some(n) => n,
            None => return,
        };

        if let Some(init) = &local.init {
            let mut bindings: HashMap<String, Expr> = HashMap::new();
            bindings.insert("v".to_string(), (*init.expr).clone());
            let mut obligation = pred.clone();
            Substitute { bindings: &bindings }.visit_expr_mut(&mut obligation);

            let goals = match translate_predicate(&obligation) {
                Ok(cs) => cs,
                Err(e) => {
                    self.errors.push(Error::new(
                        pred.span(),
                        format!("pvmsafe: cannot translate let refinement: {e}"),
                    ));
                    return;
                }
            };

            for goal in &goals {
                match fm::entails(&self.assumptions, goal) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.errors.push(Error::new(
                            local.span(),
                            format!(
                                "pvmsafe: let refinement `{}` not provable from assumptions",
                                quote::ToTokens::to_token_stream(&pred)
                            ),
                        ));
                        return;
                    }
                    Err(FmError::Overflow) => {
                        self.errors.push(Error::new(
                            local.span(),
                            "pvmsafe: let refinement check exceeded Fourier-Motzkin complexity bound",
                        ));
                        return;
                    }
                }
            }
        }

        let mut inject_bindings: HashMap<String, Expr> = HashMap::new();
        inject_bindings.insert(
            "v".to_string(),
            syn::parse_str::<Expr>(&bind_name).unwrap(),
        );
        let mut injected = pred.clone();
        Substitute {
            bindings: &inject_bindings,
        }
        .visit_expr_mut(&mut injected);
        if let Ok(cs) = translate_predicate(&injected) {
            self.assumptions.extend(cs);
        }
    }

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
    fn given_attribute_adds_assumption_for_single_call() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64) {
                    #[pvmsafe::given(a > 0)]
                    callee(a);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn given_does_not_leak_to_next_statement() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64) {
                    #[pvmsafe::given(a > 0)]
                    callee(a);
                    callee(a);
                }
                fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn given_composes_with_walker_inferred_facts() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a > 0 {
                        #[pvmsafe::given(b > 0)]
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
    fn given_with_conjunction_exposes_both_facts() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    #[pvmsafe::given(a > 0 && b > 0)]
                    callee(a, b);
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
    fn subtraction_proven_safe_from_refinements() {
        let errs = check(
            r#"
            mod m {
                fn caller(
                    #[pvmsafe::refine(a >= b)] a: u64,
                    #[pvmsafe::refine(b >= 0)] b: u64,
                ) {
                    let _ = a - b;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn subtraction_rejected_without_refinements() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    let _ = a - b;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may underflow"), "{errs:?}");
    }

    #[test]
    fn subtraction_guarded_by_if_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a >= b {
                        let _ = a - b;
                    }
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn subtraction_discharged_by_given_attribute() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    #[pvmsafe::given(a >= b)]
                    { let _ = a - b; }
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn subtraction_with_literal_is_rejected_when_unprovable() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64) {
                    let _ = a - 5;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may underflow"));
    }

    #[test]
    fn subtraction_with_literal_accepted_when_lower_bound_known() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(a >= 5)] a: u64) {
                    let _ = a - 5;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn non_integer_subtraction_is_silently_skipped() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64) {
                    let _ = foo(a) - bar(a);
                }
                fn foo(x: u64) -> u64 { x }
                fn bar(x: u64) -> u64 { x }
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

    #[test]
    fn ensures_injects_assumption_at_call_site() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    let x = producer();
                    consumer(x);
                }
                #[pvmsafe::ensures(v > 0)]
                fn producer() -> u64 { 1 }
                fn consumer(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_without_let_binding_does_not_inject() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    producer();
                    let x = 0;
                    consumer(x);
                }
                #[pvmsafe::ensures(v > 0)]
                fn producer() -> u64 { 1 }
                fn consumer(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn ensures_substitutes_params_at_call_site() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(n > 0)] n: u64) {
                    let result = add_one(n);
                    consumer(result);
                }
                #[pvmsafe::ensures(v > x)]
                fn add_one(x: u64) -> u64 { #[pvmsafe::given(x + 1 > x)] (x + 1) }
                fn consumer(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_scoped_to_binding_name() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    let a = producer();
                    consumer(a);
                }
                #[pvmsafe::ensures(v > 0)]
                fn producer() -> u64 { 1 }
                fn consumer(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_invalidated_by_shadowing() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    let x = producer();
                    let x: u64 = 0;
                    consumer(x);
                }
                #[pvmsafe::ensures(v > 0)]
                fn producer() -> u64 { 1 }
                fn consumer(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn vault_contract_withdraw_is_safe() {
        let errs = check(
            r#"
            mod vault {
                #[pvmsafe::ensures(v >= 0)]
                fn get_balance(addr: u64) -> u64 { 0 }

                fn set_balance(addr: u64, amount: u64) {}
                fn transfer_to(addr: u64, amount: u64) {}

                fn withdraw(
                    #[pvmsafe::refine(amount > 0)] amount: u64,
                    caller: u64,
                ) {
                    let balance = get_balance(caller);
                    if balance >= amount {
                        let new_balance = balance - amount;
                        set_balance(caller, new_balance);
                        transfer_to(caller, amount);
                    }
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn vault_contract_withdraw_without_guard_fails() {
        let errs = check(
            r#"
            mod vault {
                #[pvmsafe::ensures(v >= 0)]
                fn get_balance(addr: u64) -> u64 { 0 }

                fn set_balance(addr: u64, amount: u64) {}

                fn withdraw(
                    #[pvmsafe::refine(amount > 0)] amount: u64,
                    caller: u64,
                ) {
                    let balance = get_balance(caller);
                    let new_balance = balance - amount;
                    set_balance(caller, new_balance);
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may underflow"), "{errs:?}");
    }

    #[test]
    fn early_return_guard_adds_negation() {
        let errs = check(
            r#"
            mod m {
                fn caller(x: u64) {
                    if x < 5 {
                        return;
                    }
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(y >= 5)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn early_return_negation_scoped_to_block() {
        let errs = check(
            r#"
            mod m {
                fn outer(x: u64) {
                    {
                        if x < 5 {
                            return;
                        }
                    }
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(y >= 5)] y: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn multiple_guards_accumulate() {
        let errs = check(
            r#"
            mod m {
                fn caller(a: u64, b: u64) {
                    if a < 1 {
                        return;
                    }
                    if b < 1 {
                        return;
                    }
                    callee(a, b);
                }
                fn callee(
                    #[pvmsafe::refine(x >= 1)] x: u64,
                    #[pvmsafe::refine(y >= 1)] y: u64,
                ) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn non_diverging_then_branch_not_treated_as_guard() {
        let errs = check(
            r#"
            mod m {
                fn caller(x: u64) {
                    if x < 5 {
                        let _ = 0;
                    }
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(y >= 5)] y: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn early_return_discharges_subtraction() {
        let errs = check(
            r#"
            mod m {
                fn withdraw(balance: u64, amount: u64) {
                    if balance < amount {
                        return;
                    }
                    let _ = balance - amount;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn early_return_with_expr_body_diverges() {
        let errs = check(
            r#"
            mod m {
                fn caller(x: u64) -> u64 {
                    if x < 5 {
                        return 0;
                    }
                    callee(x);
                    0
                }
                fn callee(#[pvmsafe::refine(y >= 5)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_discharges_subtraction_safety() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(amount > 0)] amount: u64) {
                    let balance = get_balance();
                    if balance < amount { return; }
                    let _ = balance - amount;
                }
                #[pvmsafe::ensures(v >= 0)]
                fn get_balance() -> u64 { 0 }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_works_through_try_operator() {
        let errs = check(
            r#"
            mod m {
                fn caller() {
                    let x = producer()?;
                    consumer(x);
                }
                #[pvmsafe::ensures(v > 0)]
                fn producer() -> u64 { 1 }
                fn consumer(#[pvmsafe::refine(y > 0)] y: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_through_try_discharges_subtraction() {
        let errs = check(
            r#"
            mod m {
                fn caller(#[pvmsafe::refine(amount > 0)] amount: u64) {
                    let balance = get_balance()?;
                    if balance < amount {
                        return;
                    }
                    let _ = balance - amount;
                }
                #[pvmsafe::ensures(v >= 0)]
                fn get_balance() -> u64 { 0 }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_body_that_violates_predicate_is_rejected() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::ensures(v > 0)]
                fn bad() -> u64 { 0 }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn ensures_explicit_return_is_checked() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::ensures(v > 0)]
                fn bad(x: u64) -> u64 {
                    return 0;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn ensures_body_satisfying_predicate_is_accepted() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::ensures(v > 0)]
                fn good() -> u64 { 1 }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn ensures_uses_param_refinements_as_assumptions() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::ensures(v >= x)]
                fn identity(#[pvmsafe::refine(x > 0)] x: u64) -> u64 { x }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn let_refine_accepted_when_provable() {
        let errs = check(
            r#"
            mod m {
                fn f(#[pvmsafe::refine(x > 0)] x: u64) {
                    #[pvmsafe::refine(v > 0)]
                    let y = x;
                    callee(y);
                }
                fn callee(#[pvmsafe::refine(a > 0)] a: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn let_refine_rejected_when_unprovable() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64) {
                    #[pvmsafe::refine(v > 0)]
                    let y = x;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn let_refine_injects_assumption_for_subsequent_code() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64) {
                    if x < 5 { return; }
                    #[pvmsafe::refine(v >= 5)]
                    let y = x;
                    callee(y);
                }
                fn callee(#[pvmsafe::refine(a >= 5)] a: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn let_refine_with_literal() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe::refine(v > 0)]
                    let x = 1;
                    callee(x);
                }
                fn callee(#[pvmsafe::refine(a > 0)] a: u64) {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn let_refine_literal_zero_rejected() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    #[pvmsafe::refine(v > 0)]
                    let x = 0;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("not provable"));
    }

    #[test]
    fn let_refine_discharges_subtraction() {
        let errs = check(
            r#"
            mod m {
                fn f(#[pvmsafe::refine(amount > 0)] amount: u64) {
                    #[pvmsafe::refine(v >= amount)]
                    let balance = amount;
                    let _ = balance - amount;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn let_refine_invalidated_by_shadowing() {
        let errs = check(
            r#"
            mod m {
                fn f(#[pvmsafe::refine(x > 0)] x: u64) {
                    #[pvmsafe::refine(v > 0)]
                    let y = x;
                    let y = 0;
                    callee(y);
                }
                fn callee(#[pvmsafe::refine(a > 0)] a: u64) {}
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn addition_between_variables_is_flagged() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = x + y;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may overflow"));
    }

    #[test]
    fn addition_between_literals_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    let _ = 1 + 2;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn addition_variable_plus_literal_is_flagged() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64) {
                    let _ = x + 1;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may overflow"));
    }

    #[test]
    fn addition_suppressed_by_given() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = #[pvmsafe::given(x + y >= x)] (x + y);
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn addition_on_non_integer_is_skipped() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    let _ = foo() + bar();
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn multiplication_between_variables_is_flagged() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = x * y;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may overflow"));
    }

    #[test]
    fn multiplication_between_literals_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn f() {
                    let _ = 2 * 3;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn multiplication_suppressed_by_given() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = #[pvmsafe::given(x > 0)] (x * y);
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn division_by_variable_without_proof_is_flagged() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = x / y;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("divide by zero"));
    }

    #[test]
    fn division_by_proven_nonzero_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, #[pvmsafe::refine(y > 0)] y: u64) {
                    let _ = x / y;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn division_by_positive_literal_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64) {
                    let _ = x / 2;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn division_by_zero_literal_is_flagged() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64) {
                    let _ = x / 0;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("divide by zero"));
    }

    #[test]
    fn modulo_by_variable_without_proof_is_flagged() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = x % y;
                }
            }
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("divide by zero"));
    }

    #[test]
    fn modulo_by_proven_nonzero_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, #[pvmsafe::refine(y > 0)] y: u64) {
                    let _ = x % y;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn division_guarded_by_if_is_accepted() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    if y < 1 { return; }
                    let _ = x / y;
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn division_suppressed_by_given() {
        let errs = check(
            r#"
            mod m {
                fn f(x: u64, y: u64) {
                    let _ = #[pvmsafe::given(y > 0)] (x / y);
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }
}
