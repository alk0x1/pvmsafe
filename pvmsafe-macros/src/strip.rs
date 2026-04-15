use syn::visit_mut::{self, VisitMut};
use syn::{Attribute, ExprBlock, ExprCall, ExprMethodCall, ItemMod, PatType};

pub fn strip_pvmsafe_attrs(module: &mut ItemMod) {
    Stripper.visit_item_mod_mut(module);
}

struct Stripper;

impl VisitMut for Stripper {
    fn visit_expr_block_mut(&mut self, b: &mut ExprBlock) {
        b.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_block_mut(self, b);
    }

    fn visit_expr_call_mut(&mut self, c: &mut ExprCall) {
        c.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_call_mut(self, c);
    }

    fn visit_expr_method_call_mut(&mut self, c: &mut ExprMethodCall) {
        c.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_method_call_mut(self, c);
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
