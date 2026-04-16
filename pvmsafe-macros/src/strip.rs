use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, ExprBinary, ExprBlock, ExprCall, ExprMethodCall, ExprParen, ItemFn, ItemMod, Local,
    PatType,
};

pub fn strip_pvmsafe_attrs(module: &mut ItemMod) {
    Stripper.visit_item_mod_mut(module);
}

struct Stripper;

impl VisitMut for Stripper {
    fn visit_expr_block_mut(&mut self, b: &mut ExprBlock) {
        b.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_block_mut(self, b);
    }

    fn visit_expr_binary_mut(&mut self, b: &mut ExprBinary) {
        b.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_binary_mut(self, b);
    }

    fn visit_expr_paren_mut(&mut self, p: &mut ExprParen) {
        p.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_paren_mut(self, p);
    }

    fn visit_expr_call_mut(&mut self, c: &mut ExprCall) {
        c.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_call_mut(self, c);
    }

    fn visit_expr_method_call_mut(&mut self, c: &mut ExprMethodCall) {
        c.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_method_call_mut(self, c);
    }

    fn visit_item_fn_mut(&mut self, f: &mut ItemFn) {
        f.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_item_fn_mut(self, f);
    }

    fn visit_item_mod_mut(&mut self, m: &mut ItemMod) {
        m.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_item_mod_mut(self, m);
    }

    fn visit_local_mut(&mut self, local: &mut Local) {
        local.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_local_mut(self, local);
    }

    fn visit_pat_type_mut(&mut self, pt: &mut PatType) {
        pt.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_pat_type_mut(self, pt);
    }
}

fn is_pvmsafe(attr: &Attribute) -> bool {
    attr.path()
        .segments
        .first()
        .map(|s| s.ident == "pvmsafe" || s.ident == "pvmsafe_macros")
        .unwrap_or(false)
}
