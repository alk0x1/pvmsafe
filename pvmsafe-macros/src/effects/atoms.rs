use std::collections::HashSet;
use syn::punctuated::Punctuated;
use syn::{Attribute, Error, Ident, Path, Result, Token};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Effect {
    Read,
    Write,
    Call,
    Revert,
    Emit,
}

impl Effect {
    pub fn name(&self) -> &'static str {
        match self {
            Effect::Read => "read",
            Effect::Write => "write",
            Effect::Call => "call",
            Effect::Revert => "revert",
            Effect::Emit => "emit",
        }
    }

    fn from_ident(ident: &Ident) -> Option<Self> {
        match ident.to_string().as_str() {
            "read" => Some(Effect::Read),
            "write" => Some(Effect::Write),
            "call" => Some(Effect::Call),
            "revert" => Some(Effect::Revert),
            "emit" => Some(Effect::Emit),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectSet {
    atoms: HashSet<Effect>,
}

impl EffectSet {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, e: Effect) -> bool {
        self.atoms.insert(e)
    }

    pub fn contains(&self, e: Effect) -> bool {
        self.atoms.contains(&e)
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    pub fn union_with(&mut self, other: &EffectSet) -> bool {
        let mut changed = false;
        for e in &other.atoms {
            changed |= self.atoms.insert(*e);
        }
        changed
    }

    pub fn is_subset_of(&self, other: &EffectSet) -> bool {
        self.atoms.iter().all(|e| other.atoms.contains(e))
    }

    pub fn difference(&self, other: &EffectSet) -> Vec<Effect> {
        let mut out: Vec<Effect> = self
            .atoms
            .iter()
            .filter(|e| !other.atoms.contains(e))
            .copied()
            .collect();
        out.sort_by_key(|e| e.name());
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllowKind {
    WriteAfterCall,
    EmitAfterCall,
}

pub fn extract_effect_decl(attrs: &[Attribute]) -> Result<Option<EffectSet>> {
    let mut found: Option<(&Attribute, EffectSet)> = None;
    for attr in attrs {
        if !is_pvmsafe_path(attr.path(), "effect") {
            continue;
        }
        let set = parse_effect_args(attr)?;
        if let Some((prev, _)) = &found {
            let mut err = Error::new_spanned(
                attr,
                "pvmsafe: duplicate `#[pvmsafe::effect(...)]` on this item",
            );
            err.combine(Error::new_spanned(*prev, "note: earlier declaration here"));
            return Err(err);
        }
        found = Some((attr, set));
    }
    Ok(found.map(|(_, set)| set))
}

pub fn extract_effect_allow(attrs: &[Attribute]) -> Result<Vec<AllowKind>> {
    let mut out = Vec::new();
    for attr in attrs {
        if !is_pvmsafe_path(attr.path(), "effect_allow") {
            continue;
        }
        let parser = Punctuated::<Ident, Token![,]>::parse_terminated;
        let idents = attr.parse_args_with(parser)?;
        for ident in &idents {
            let kind = match ident.to_string().as_str() {
                "write_after_call" => AllowKind::WriteAfterCall,
                "emit_after_call" => AllowKind::EmitAfterCall,
                other => {
                    return Err(Error::new(
                        ident.span(),
                        format!(
                            "pvmsafe: unknown effect_allow kind `{other}`; \
                             expected `write_after_call` or `emit_after_call`"
                        ),
                    ));
                }
            };
            out.push(kind);
        }
    }
    Ok(out)
}

fn parse_effect_args(attr: &Attribute) -> Result<EffectSet> {
    let parser = Punctuated::<Ident, Token![,]>::parse_terminated;
    let idents = attr.parse_args_with(parser)?;
    let mut set = EffectSet::empty();
    let saw_pure = idents.iter().any(|i| i == "pure");
    if saw_pure && idents.len() > 1 {
        let extra = idents.iter().find(|i| *i != "pure").unwrap();
        return Err(Error::new(
            extra.span(),
            "pvmsafe: `pure` cannot be combined with other effect atoms",
        ));
    }
    for ident in &idents {
        if ident == "pure" {
            continue;
        }
        match Effect::from_ident(ident) {
            Some(e) => {
                set.insert(e);
            }
            None => {
                return Err(Error::new(
                    ident.span(),
                    format!(
                        "pvmsafe: unknown effect `{ident}`; \
                         expected one of: read, write, call, revert, emit, pure"
                    ),
                ));
            }
        }
    }
    Ok(set)
}

fn is_pvmsafe_path(path: &Path, name: &str) -> bool {
    let segs: Vec<_> = path.segments.iter().collect();
    matches!(
        segs.as_slice(),
        [ns, n] if ns.ident == "pvmsafe" && n.ident == name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fn_attrs(src: &str) -> Vec<Attribute> {
        let f: syn::ItemFn = syn::parse_str(src).expect("parse");
        f.attrs
    }

    #[test]
    fn single_effect_is_parsed() {
        let attrs = fn_attrs("#[pvmsafe::effect(write)] fn f() {}");
        let set = extract_effect_decl(&attrs).unwrap().unwrap();
        assert!(set.contains(Effect::Write));
        assert!(!set.contains(Effect::Read));
    }

    #[test]
    fn multiple_effects_are_parsed() {
        let attrs = fn_attrs("#[pvmsafe::effect(read, write, revert)] fn f() {}");
        let set = extract_effect_decl(&attrs).unwrap().unwrap();
        assert!(set.contains(Effect::Read));
        assert!(set.contains(Effect::Write));
        assert!(set.contains(Effect::Revert));
        assert!(!set.contains(Effect::Call));
        assert!(!set.contains(Effect::Emit));
    }

    #[test]
    fn pure_is_the_empty_set() {
        let attrs = fn_attrs("#[pvmsafe::effect(pure)] fn f() {}");
        let set = extract_effect_decl(&attrs).unwrap().unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn empty_parens_are_the_empty_set() {
        let attrs = fn_attrs("#[pvmsafe::effect()] fn f() {}");
        let set = extract_effect_decl(&attrs).unwrap().unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn pure_combined_with_other_atoms_is_rejected() {
        let attrs = fn_attrs("#[pvmsafe::effect(pure, read)] fn f() {}");
        let err = extract_effect_decl(&attrs).unwrap_err();
        assert!(
            err.to_string().contains("`pure` cannot be combined"),
            "{err}"
        );
    }

    #[test]
    fn unknown_atom_is_rejected() {
        let attrs = fn_attrs("#[pvmsafe::effect(teleport)] fn f() {}");
        let err = extract_effect_decl(&attrs).unwrap_err();
        assert!(err.to_string().contains("unknown effect `teleport`"));
    }

    #[test]
    fn missing_attribute_returns_none() {
        let attrs = fn_attrs("fn f() {}");
        assert!(extract_effect_decl(&attrs).unwrap().is_none());
    }

    #[test]
    fn duplicate_effect_attribute_is_rejected() {
        let attrs = fn_attrs(
            "#[pvmsafe::effect(read)] #[pvmsafe::effect(write)] fn f() {}",
        );
        let err = extract_effect_decl(&attrs).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn allow_write_after_call_is_parsed() {
        let attrs = fn_attrs("#[pvmsafe::effect_allow(write_after_call)] fn f() {}");
        let kinds = extract_effect_allow(&attrs).unwrap();
        assert_eq!(kinds, vec![AllowKind::WriteAfterCall]);
    }

    #[test]
    fn allow_emit_after_call_is_parsed() {
        let attrs = fn_attrs("#[pvmsafe::effect_allow(emit_after_call)] fn f() {}");
        let kinds = extract_effect_allow(&attrs).unwrap();
        assert_eq!(kinds, vec![AllowKind::EmitAfterCall]);
    }

    #[test]
    fn multiple_allow_kinds_are_parsed() {
        let attrs = fn_attrs(
            "#[pvmsafe::effect_allow(write_after_call, emit_after_call)] fn f() {}",
        );
        let kinds = extract_effect_allow(&attrs).unwrap();
        assert!(kinds.contains(&AllowKind::WriteAfterCall));
        assert!(kinds.contains(&AllowKind::EmitAfterCall));
    }

    #[test]
    fn unknown_allow_kind_is_rejected() {
        let attrs = fn_attrs("#[pvmsafe::effect_allow(teleport)] fn f() {}");
        let err = extract_effect_allow(&attrs).unwrap_err();
        assert!(err.to_string().contains("unknown effect_allow kind"));
    }

    #[test]
    fn effect_set_union_and_subset() {
        let mut a = EffectSet::empty();
        a.insert(Effect::Read);
        let mut b = EffectSet::empty();
        b.insert(Effect::Read);
        b.insert(Effect::Write);
        assert!(a.is_subset_of(&b));
        assert!(!b.is_subset_of(&a));

        let mut c = EffectSet::empty();
        assert!(c.union_with(&a));
        assert!(c.union_with(&b));
        assert!(!c.union_with(&a));
        assert!(c.contains(Effect::Read));
        assert!(c.contains(Effect::Write));
    }

    #[test]
    fn effect_set_difference() {
        let mut a = EffectSet::empty();
        a.insert(Effect::Read);
        a.insert(Effect::Write);
        a.insert(Effect::Call);

        let mut b = EffectSet::empty();
        b.insert(Effect::Read);

        let diff = a.difference(&b);
        assert_eq!(diff.len(), 2);
        assert!(diff.contains(&Effect::Write));
        assert!(diff.contains(&Effect::Call));
    }
}
