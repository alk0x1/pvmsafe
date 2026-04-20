use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Block, Error, Expr, ExprCall, ExprForLoop, ExprIf, ExprLoop, ExprMatch, ExprMethodCall,
    ExprPath, ExprWhile, Ident, Item, ItemFn, ItemMod,
};

use super::atoms::{AllowKind, Effect, EffectSet, extract_effect_allow};
use super::infer::{Analysis, analyze_module};

pub fn check_module(module: &ItemMod, errors: &mut Vec<Error>) {
    let analysis = match analyze_module(module) {
        Ok(a) => a,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    let Some((_, items)) = &module.content else {
        return;
    };
    for item in items {
        if let Item::Fn(f) = item {
            check_fn(f, &analysis, errors);
        }
    }
}

fn check_fn(f: &ItemFn, a: &Analysis, errors: &mut Vec<Error>) {
    let allow = match extract_effect_allow(&f.attrs) {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    let ctx = TraceCtx {
        analysis: a,
        allow_write_after_call: allow.contains(&AllowKind::WriteAfterCall),
        allow_emit_after_call: allow.contains(&AllowKind::EmitAfterCall),
    };

    let name = &f.sig.ident;
    if let Some(declared) = a.declared.get(name) {
        let inferred = a.inferred.get(name).cloned().unwrap_or_default();
        let diff = inferred.difference(declared);
        if !diff.is_empty() {
            let labels: Vec<_> = diff.iter().map(|e| e.name()).collect();
            errors.push(Error::new_spanned(
                &f.sig.ident,
                format!(
                    "pvmsafe: function `{name}` declares effects but its body exhibits undeclared effect(s): {}",
                    labels.join(", "),
                ),
            ));
        }
    }

    let mut walker = Walker {
        ctx: &ctx,
        state: TraceState::default(),
        errors,
    };
    walker.visit_block(&f.block);
}

#[derive(Default, Clone)]
struct TraceState {
    first_call_span: Option<Span>,
}

impl TraceState {
    fn merge(&mut self, other: &Self) {
        if self.first_call_span.is_none() {
            self.first_call_span = other.first_call_span;
        }
    }

    fn seen_call(&self) -> bool {
        self.first_call_span.is_some()
    }
}

struct TraceCtx<'a> {
    analysis: &'a Analysis,
    allow_write_after_call: bool,
    allow_emit_after_call: bool,
}

impl<'a> TraceCtx<'a> {
    fn effects_of(&self, name: &Ident) -> Option<&EffectSet> {
        self.analysis.effective_of(name)
    }
}

struct Walker<'a, 'e> {
    ctx: &'a TraceCtx<'a>,
    state: TraceState,
    errors: &'e mut Vec<Error>,
}

impl<'a, 'e> Walker<'a, 'e> {
    fn apply(&mut self, name: &Ident, span: Span) {
        let Some(eff) = self.ctx.effects_of(name).cloned() else {
            return;
        };
        if eff.contains(Effect::Write) && self.state.seen_call() && !self.ctx.allow_write_after_call
        {
            let mut err = Error::new(
                span,
                "pvmsafe: state write after external call; reentrancy risk",
            );
            if let Some(call_span) = self.state.first_call_span {
                err.combine(Error::new(call_span, "note: earlier external call here"));
            }
            self.errors.push(err);
        }
        if eff.contains(Effect::Emit) && self.state.seen_call() && !self.ctx.allow_emit_after_call {
            let mut err = Error::new(span, "pvmsafe: event emit after external call; reentrancy risk");
            if let Some(call_span) = self.state.first_call_span {
                err.combine(Error::new(call_span, "note: earlier external call here"));
            }
            self.errors.push(err);
        }
        if eff.contains(Effect::Call) && self.state.first_call_span.is_none() {
            self.state.first_call_span = Some(span);
        }
    }

    fn walk_loop_body(&mut self, body: &Block) {
        if !self.state.seen_call() && body_contains_call_effect(body, self.ctx) {
            self.state.first_call_span = Some(body.span());
        }
        self.visit_block(body);
    }
}

impl<'a, 'e, 'ast> Visit<'ast> for Walker<'a, 'e> {
    fn visit_expr_call(&mut self, c: &'ast ExprCall) {
        self.visit_expr(&c.func);
        for arg in &c.args {
            self.visit_expr(arg);
        }
        if let Expr::Path(ExprPath { path, .. }) = &*c.func {
            if let Some(id) = path.get_ident() {
                self.apply(id, c.span());
            }
        }
    }

    fn visit_expr_method_call(&mut self, c: &'ast ExprMethodCall) {
        self.visit_expr(&c.receiver);
        for arg in &c.args {
            self.visit_expr(arg);
        }
        self.apply(&c.method, c.span());
    }

    fn visit_expr_if(&mut self, e: &'ast ExprIf) {
        self.visit_expr(&e.cond);
        let entry = self.state.clone();
        self.visit_block(&e.then_branch);
        let after_then = std::mem::replace(&mut self.state, entry.clone());
        if let Some((_, eb)) = &e.else_branch {
            self.visit_expr(eb);
        }
        let after_else = std::mem::replace(&mut self.state, entry);
        self.state.merge(&after_then);
        self.state.merge(&after_else);
    }

    fn visit_expr_match(&mut self, e: &'ast ExprMatch) {
        self.visit_expr(&e.expr);
        let entry = self.state.clone();
        let mut merged = entry.clone();
        for arm in &e.arms {
            self.state = entry.clone();
            if let Some((_, g)) = &arm.guard {
                self.visit_expr(g);
            }
            self.visit_expr(&arm.body);
            merged.merge(&self.state);
        }
        self.state = merged;
    }

    fn visit_expr_while(&mut self, e: &'ast ExprWhile) {
        self.visit_expr(&e.cond);
        self.walk_loop_body(&e.body);
        self.visit_expr(&e.cond);
    }

    fn visit_expr_for_loop(&mut self, e: &'ast ExprForLoop) {
        self.visit_expr(&e.expr);
        self.walk_loop_body(&e.body);
    }

    fn visit_expr_loop(&mut self, e: &'ast ExprLoop) {
        self.walk_loop_body(&e.body);
    }
}

fn body_contains_call_effect(block: &Block, ctx: &TraceCtx) -> bool {
    struct Scan<'a> {
        ctx: &'a TraceCtx<'a>,
        found: bool,
    }
    impl<'a, 'ast> Visit<'ast> for Scan<'a> {
        fn visit_expr_call(&mut self, c: &'ast ExprCall) {
            if let Expr::Path(ExprPath { path, .. }) = &*c.func {
                if let Some(id) = path.get_ident() {
                    if let Some(eff) = self.ctx.effects_of(id) {
                        if eff.contains(Effect::Call) {
                            self.found = true;
                        }
                    }
                }
            }
            visit::visit_expr_call(self, c);
        }
        fn visit_expr_method_call(&mut self, c: &'ast ExprMethodCall) {
            if let Some(eff) = self.ctx.effects_of(&c.method) {
                if eff.contains(Effect::Call) {
                    self.found = true;
                }
            }
            visit::visit_expr_method_call(self, c);
        }
    }
    let mut s = Scan { ctx, found: false };
    s.visit_block(block);
    s.found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> Vec<String> {
        let module: ItemMod = syn::parse_str(src).expect("parse");
        let mut errs = Vec::new();
        check_module(&module, &mut errs);
        errs.into_iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn clean_linear_call_then_write_is_rejected() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f() {
                    ext();
                    sink();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn write_then_call_is_ok() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f() {
                    sink();
                    ext();
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn emit_after_call_is_rejected() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(emit)]
                fn ev() {}
                fn f() {
                    ext();
                    ev();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("event emit after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn effect_allow_write_after_call_suppresses_error() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                #[pvmsafe::effect_allow(write_after_call)]
                fn f() {
                    ext();
                    sink();
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn effect_allow_emit_after_call_suppresses_error() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(emit)]
                fn ev() {}
                #[pvmsafe::effect_allow(emit_after_call)]
                fn f() {
                    ext();
                    ev();
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn effect_allow_write_does_not_suppress_emit_violation() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(emit)]
                fn ev() {}
                #[pvmsafe::effect_allow(write_after_call)]
                fn f() {
                    ext();
                    ev();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("event emit after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn transitive_call_effect_triggers_cei_check() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn leaf() {}
                fn indirect() { leaf(); }
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f() {
                    indirect();
                    sink();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn write_after_call_across_if_branches_is_flagged() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f(c: bool) {
                    if c { ext(); }
                    sink();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn write_inside_one_branch_only_is_flagged() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f(c: bool) {
                    ext();
                    if c { sink(); }
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn no_call_no_violation() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f() { sink(); sink(); }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn match_arms_merge_seen_call_state() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f(x: u8) {
                    match x { _ => { ext(); } }
                    sink();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn loop_treats_write_after_body_call_as_cross_iteration() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext() {}
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f(n: u8) {
                    for _ in 0..n {
                        sink();
                        ext();
                    }
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn method_call_participates_in_cei() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn ext(x: u8) {}
                #[pvmsafe::effect(write)]
                fn sink(x: u8) {}
                fn f(x: u8) {
                    x.ext();
                    x.sink();
                }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("state write after external call")),
            "{errs:?}"
        );
    }

    #[test]
    fn assertion_diff_flags_undeclared_effect() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn sink() {}
                #[pvmsafe::effect(read)]
                fn f() { sink(); }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("undeclared effect")),
            "{errs:?}"
        );
    }

    #[test]
    fn assertion_diff_ignores_underclaimed_is_rejected() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(read, write)]
                fn f() {}
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn assertion_diff_ok_when_subset() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn sink() {}
                #[pvmsafe::effect(write, emit)]
                fn f() { sink(); }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn unknown_external_leaf_contributes_nothing_to_cei() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn f() {
                    external_unknown();
                    sink();
                }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn pure_declaration_and_pure_body_passes() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(pure)]
                fn f() { let _ = 1 + 2; }
            }
            "#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn pure_declaration_with_side_effect_body_is_rejected() {
        let errs = check(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn sink() {}
                #[pvmsafe::effect(pure)]
                fn f() { sink(); }
            }
            "#,
        );
        assert!(
            errs.iter().any(|e| e.contains("undeclared effect")),
            "{errs:?}"
        );
    }
}
