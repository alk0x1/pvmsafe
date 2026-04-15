use syn::visit_mut::{self, VisitMut};
use syn::{Attribute, ExprBlock, ItemMod, PatType};

pub fn strip_pvmsafe_attrs(module: &mut ItemMod) {
    Stripper.visit_item_mod_mut(module);
}

struct Stripper;

impl VisitMut for Stripper {
    fn visit_expr_block_mut(&mut self, b: &mut ExprBlock) {
        b.attrs.retain(|a| !is_pvmsafe(a));
        visit_mut::visit_expr_block_mut(self, b);
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
