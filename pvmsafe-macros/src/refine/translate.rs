use super::lir::{Constraint, LinearExpr};
use syn::{BinOp, Expr, ExprBinary, ExprLit, ExprParen, ExprPath, ExprUnary, Lit, UnOp};

#[derive(Debug)]
pub enum TranslateError {
    NonLinear,
    Unsupported(&'static str),
    Overflow,
}

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonLinear => write!(f, "non-linear expression"),
            Self::Unsupported(s) => write!(f, "unsupported syntax: {s}"),
            Self::Overflow => write!(f, "coefficient overflow"),
        }
    }
}

pub fn translate_predicate(expr: &Expr) -> Result<Vec<Constraint>, TranslateError> {
    match expr {
        Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
            let mut left = translate_predicate(&b.left)?;
            let right = translate_predicate(&b.right)?;
            left.extend(right);
            Ok(left)
        }
        Expr::Binary(b) => translate_comparison(b),
        Expr::Paren(ExprParen { expr, .. }) => translate_predicate(expr),
        _ => Err(TranslateError::Unsupported(
            "predicate must be a comparison or && of comparisons",
        )),
    }
}

fn translate_comparison(b: &ExprBinary) -> Result<Vec<Constraint>, TranslateError> {
    let lhs = translate_term(&b.left)?;
    let rhs = translate_term(&b.right)?;
    match b.op {
        BinOp::Le(_) => Ok(vec![Constraint::new(sub(&lhs, &rhs)?)]),
        BinOp::Ge(_) => Ok(vec![Constraint::new(sub(&rhs, &lhs)?)]),
        BinOp::Lt(_) => {
            let diff = sub(&lhs, &rhs)?;
            Ok(vec![Constraint::new(add(&diff, &LinearExpr::constant(1))?)])
        }
        BinOp::Gt(_) => {
            let diff = sub(&rhs, &lhs)?;
            Ok(vec![Constraint::new(add(&diff, &LinearExpr::constant(1))?)])
        }
        BinOp::Eq(_) => Ok(vec![
            Constraint::new(sub(&lhs, &rhs)?),
            Constraint::new(sub(&rhs, &lhs)?),
        ]),
        _ => Err(TranslateError::Unsupported("comparison operator")),
    }
}

fn translate_term(expr: &Expr) -> Result<LinearExpr, TranslateError> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => {
            let n: i128 = i.base10_parse().map_err(|_| TranslateError::Overflow)?;
            Ok(LinearExpr::constant(n))
        }
        Expr::Path(ExprPath { path, .. }) => path
            .get_ident()
            .map(|id| LinearExpr::var(&id.to_string()))
            .ok_or(TranslateError::Unsupported("qualified path in refinement")),
        Expr::Paren(ExprParen { expr, .. }) => translate_term(expr),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr,
            ..
        }) => translate_term(expr)?
            .neg()
            .ok_or(TranslateError::Overflow),
        Expr::Binary(b) => match b.op {
            BinOp::Add(_) => add(&translate_term(&b.left)?, &translate_term(&b.right)?),
            BinOp::Sub(_) => sub(&translate_term(&b.left)?, &translate_term(&b.right)?),
            BinOp::Mul(_) => mul(&translate_term(&b.left)?, &translate_term(&b.right)?),
            _ => Err(TranslateError::NonLinear),
        },
        _ => Err(TranslateError::Unsupported("term form")),
    }
}

fn add(a: &LinearExpr, b: &LinearExpr) -> Result<LinearExpr, TranslateError> {
    a.add(b).ok_or(TranslateError::Overflow)
}

fn sub(a: &LinearExpr, b: &LinearExpr) -> Result<LinearExpr, TranslateError> {
    a.sub(b).ok_or(TranslateError::Overflow)
}

fn mul(a: &LinearExpr, b: &LinearExpr) -> Result<LinearExpr, TranslateError> {
    if a.terms.is_empty() {
        b.scale(a.constant).ok_or(TranslateError::Overflow)
    } else if b.terms.is_empty() {
        a.scale(b.constant).ok_or(TranslateError::Overflow)
    } else {
        Err(TranslateError::NonLinear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Expr {
        syn::parse_str(src).expect("parse expr")
    }

    #[test]
    fn translates_gt_to_tightened() {
        let cs = translate_predicate(&parse("amount > 0")).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].expr.terms.get("amount"), Some(&-1));
        assert_eq!(cs[0].expr.constant, 1);
    }

    #[test]
    fn translates_ge_without_tightening() {
        let cs = translate_predicate(&parse("amount >= 1")).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].expr.terms.get("amount"), Some(&-1));
        assert_eq!(cs[0].expr.constant, 1);
    }

    #[test]
    fn translates_eq_as_two_constraints() {
        let cs = translate_predicate(&parse("x == 5")).unwrap();
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn splits_conjunction() {
        let cs = translate_predicate(&parse("x > 0 && y < 10")).unwrap();
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn accepts_literal_coefficient() {
        let cs = translate_predicate(&parse("2 * x >= 10")).unwrap();
        assert_eq!(cs[0].expr.terms.get("x"), Some(&-2));
        assert_eq!(cs[0].expr.constant, 10);
    }

    #[test]
    fn rejects_nonlinear_two_unknowns() {
        let err = translate_predicate(&parse("x * y > 0"));
        assert!(matches!(err, Err(TranslateError::NonLinear)));
    }

    #[test]
    fn rejects_division() {
        let err = translate_predicate(&parse("x / 2 > 0"));
        assert!(matches!(err, Err(TranslateError::NonLinear)));
    }

    #[test]
    fn handles_unary_negation() {
        let cs = translate_predicate(&parse("-x <= 0")).unwrap();
        assert_eq!(cs[0].expr.terms.get("x"), Some(&-1));
    }

    #[test]
    fn handles_parenthesized() {
        let cs = translate_predicate(&parse("(x + 1) >= 2")).unwrap();
        assert_eq!(cs[0].expr.terms.get("x"), Some(&-1));
        assert_eq!(cs[0].expr.constant, 1);
    }
}
