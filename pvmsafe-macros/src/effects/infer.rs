use std::collections::{HashMap, HashSet};
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprMethodCall, ExprPath, Ident, Item, ItemFn, ItemMod, Result};

use super::atoms::{EffectSet, extract_effect_decl};

#[derive(Debug, Default)]
pub struct Analysis {
    pub declared: HashMap<Ident, EffectSet>,
    pub inferred: HashMap<Ident, EffectSet>,
}

impl Analysis {
    pub fn effective_of(&self, name: &Ident) -> Option<&EffectSet> {
        self.declared.get(name).or_else(|| self.inferred.get(name))
    }
}

pub fn analyze_module(module: &ItemMod) -> Result<Analysis> {
    let Some((_, items)) = &module.content else {
        return Ok(Analysis::default());
    };

    let fns: Vec<&ItemFn> = items
        .iter()
        .filter_map(|i| match i {
            Item::Fn(f) => Some(f),
            _ => None,
        })
        .collect();

    let mut declared: HashMap<Ident, EffectSet> = HashMap::new();
    for f in &fns {
        if let Some(set) = extract_effect_decl(&f.attrs)? {
            declared.insert(f.sig.ident.clone(), set);
        }
    }

    let mut callees: HashMap<Ident, HashSet<Ident>> = HashMap::new();
    for f in &fns {
        let mut v = CalleeCollector::default();
        v.visit_block(&f.block);
        callees.insert(f.sig.ident.clone(), v.callees);
    }

    let mut inferred: HashMap<Ident, EffectSet> = HashMap::new();
    for f in &fns {
        inferred.insert(f.sig.ident.clone(), EffectSet::empty());
    }

    loop {
        let mut changed = false;
        for f in &fns {
            let name = &f.sig.ident;
            let mut new_set = EffectSet::empty();
            for callee in &callees[name] {
                if let Some(s) = declared.get(callee).or_else(|| inferred.get(callee)) {
                    new_set.union_with(s);
                }
            }
            let current = inferred.get_mut(name).expect("fn registered above");
            if current.union_with(&new_set) {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    Ok(Analysis { declared, inferred })
}

#[derive(Default)]
struct CalleeCollector {
    callees: HashSet<Ident>,
}

impl<'ast> Visit<'ast> for CalleeCollector {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(ExprPath { path, .. }) = &*call.func {
            if let Some(ident) = path.get_ident() {
                self.callees.insert(ident.clone());
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        self.callees.insert(call.method.clone());
        visit::visit_expr_method_call(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::atoms::Effect;

    fn module(src: &str) -> ItemMod {
        syn::parse_str(src).expect("parse")
    }

    fn id(s: &str) -> Ident {
        syn::parse_str(s).expect("ident")
    }

    #[test]
    fn empty_module_yields_empty_analysis() {
        let m = module("mod m {}");
        let a = analyze_module(&m).unwrap();
        assert!(a.declared.is_empty());
        assert!(a.inferred.is_empty());
    }

    #[test]
    fn leaf_fn_without_declaration_has_empty_effects() {
        let m = module("mod m { fn leaf() {} }");
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("leaf")].is_empty());
        assert!(a.declared.get(&id("leaf")).is_none());
    }

    #[test]
    fn declared_effects_are_recorded() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn leaf() {}
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.declared[&id("leaf")].contains(Effect::Write));
    }

    #[test]
    fn pure_is_recorded_as_empty_declared_set() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(pure)]
                fn leaf() {}
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.declared[&id("leaf")].is_empty());
    }

    #[test]
    fn caller_inherits_effect_from_declared_callee() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn leaf() {}
                fn caller() { leaf(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("caller")].contains(Effect::Write));
    }

    #[test]
    fn transitive_propagation_through_chain() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn c() {}
                fn b() { c(); }
                fn a() { b(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("a")].contains(Effect::Write));
        assert!(a.inferred[&id("b")].contains(Effect::Write));
    }

    #[test]
    fn mutual_recursion_converges() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(read)]
                fn leaf() {}
                fn a() { b(); leaf(); }
                fn b() { a(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("a")].contains(Effect::Read));
        assert!(a.inferred[&id("b")].contains(Effect::Read));
    }

    #[test]
    fn unknown_external_callee_contributes_nothing() {
        let m = module(
            r#"
            mod m {
                fn caller() { external_unknown(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("caller")].is_empty());
    }

    #[test]
    fn multiple_effects_propagate_together() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(read, write, emit)]
                fn leaf() {}
                fn caller() { leaf(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        let set = &a.inferred[&id("caller")];
        assert!(set.contains(Effect::Read));
        assert!(set.contains(Effect::Write));
        assert!(set.contains(Effect::Emit));
    }

    #[test]
    fn method_call_is_captured_as_callee() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(call)]
                fn outbound(x: u8) {}
                fn caller(x: u8) { x.outbound(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("caller")].contains(Effect::Call));
    }

    #[test]
    fn declared_at_callee_hides_body_from_caller() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(revert)]
                fn internal() {}
                #[pvmsafe::effect(write)]
                fn proxy() { internal(); }
                fn caller() { proxy(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("caller")].contains(Effect::Write));
        assert!(!a.inferred[&id("caller")].contains(Effect::Revert));
    }

    #[test]
    fn effective_of_prefers_declared_over_inferred() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn leaf() {}
                fn caller() { leaf(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        let caller_eff = a.effective_of(&id("caller")).unwrap();
        assert!(caller_eff.contains(Effect::Write));
        let leaf_eff = a.effective_of(&id("leaf")).unwrap();
        assert!(leaf_eff.contains(Effect::Write));
    }

    #[test]
    fn nested_control_flow_still_collects_callees() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(write)]
                fn sink() {}
                fn caller(cond: bool, n: u8) {
                    if cond {
                        for _ in 0..n { sink(); }
                    } else {
                        match cond { _ => { sink(); } }
                    }
                }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("caller")].contains(Effect::Write));
    }

    #[test]
    fn body_inferred_reflects_unwrapped_callees_for_self() {
        let m = module(
            r#"
            mod m {
                #[pvmsafe::effect(revert)]
                fn inner() {}
                #[pvmsafe::effect(write)]
                fn over_declared() { inner(); }
            }
            "#,
        );
        let a = analyze_module(&m).unwrap();
        assert!(a.inferred[&id("over_declared")].contains(Effect::Revert));
        assert!(!a.inferred[&id("over_declared")].contains(Effect::Write));
        assert!(a.declared[&id("over_declared")].contains(Effect::Write));
    }
}
