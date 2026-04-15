use super::lir::{Constraint, LinearExpr};

const OVERFLOW_LIMIT: i128 = 1 << 60;

#[derive(Debug)]
pub enum FmError {
    Overflow,
}

pub fn entails(assumptions: &[Constraint], goal: &Constraint) -> Result<bool, FmError> {
    let mut constraints: Vec<Constraint> = assumptions.to_vec();
    constraints.push(negate(goal).ok_or(FmError::Overflow)?);

    loop {
        let mut filtered = Vec::new();
        for c in constraints {
            if c.expr.terms.is_empty() {
                if c.expr.constant > 0 {
                    return Ok(true);
                }
            } else {
                filtered.push(c);
            }
        }
        constraints = filtered;

        for c in &constraints {
            if c.expr.constant.abs() > OVERFLOW_LIMIT {
                return Err(FmError::Overflow);
            }
            for v in c.expr.terms.values() {
                if v.abs() > OVERFLOW_LIMIT {
                    return Err(FmError::Overflow);
                }
            }
        }

        let var = constraints
            .iter()
            .flat_map(|c| c.expr.terms.keys())
            .next()
            .cloned();

        let Some(var) = var else {
            return Ok(false);
        };

        constraints = eliminate(&var, &constraints)?;
    }
}

pub(super) fn negate(c: &Constraint) -> Option<Constraint> {
    let neg_terms = c.expr.neg()?;
    let with_one = neg_terms.add(&LinearExpr::constant(1))?;
    Some(Constraint::new(with_one))
}

fn eliminate(var: &str, constraints: &[Constraint]) -> Result<Vec<Constraint>, FmError> {
    let mut pos: Vec<&Constraint> = Vec::new();
    let mut neg: Vec<&Constraint> = Vec::new();
    let mut zero: Vec<Constraint> = Vec::new();

    for c in constraints {
        match c.expr.terms.get(var).copied().unwrap_or(0) {
            x if x > 0 => pos.push(c),
            x if x < 0 => neg.push(c),
            _ => zero.push(c.clone()),
        }
    }

    let mut out = zero;
    for p in &pos {
        for n in &neg {
            let a_p = p.expr.terms[var];
            let a_n = n.expr.terms[var];
            let m_p = -a_n;
            let m_n = a_p;
            let scaled_p = p.expr.scale(m_p).ok_or(FmError::Overflow)?;
            let scaled_n = n.expr.scale(m_n).ok_or(FmError::Overflow)?;
            let combined = scaled_p.add(&scaled_n).ok_or(FmError::Overflow)?;
            out.push(Constraint::new(combined));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::translate::translate_predicate;
    use super::*;

    fn constraints(src: &str) -> Vec<Constraint> {
        let expr: syn::Expr = syn::parse_str(src).unwrap();
        translate_predicate(&expr).unwrap()
    }

    fn one(src: &str) -> Constraint {
        constraints(src).into_iter().next().unwrap()
    }

    #[test]
    fn proves_same_predicate() {
        let assumps = constraints("amount > 0");
        let goal = one("amount > 0");
        assert!(entails(&assumps, &goal).unwrap());
    }

    #[test]
    fn proves_weaker_from_stronger() {
        let assumps = constraints("x >= 5");
        let goal = one("x >= 1");
        assert!(entails(&assumps, &goal).unwrap());
    }

    #[test]
    fn refuses_stronger_from_weaker() {
        let assumps = constraints("x >= 1");
        let goal = one("x >= 5");
        assert!(!entails(&assumps, &goal).unwrap());
    }

    #[test]
    fn proves_ge_1_from_gt_0_over_integers() {
        let assumps = constraints("amount > 0");
        let goal = one("amount >= 1");
        assert!(entails(&assumps, &goal).unwrap());
    }

    #[test]
    fn proves_via_transitivity() {
        let assumps = constraints("x >= 10 && y >= x");
        let goal = one("y >= 10");
        assert!(entails(&assumps, &goal).unwrap());
    }

    #[test]
    fn refuses_unrelated_vars() {
        let assumps = constraints("x > 0");
        let goal = one("y > 0");
        assert!(!entails(&assumps, &goal).unwrap());
    }

    #[test]
    fn proves_sum_positive() {
        let assumps = constraints("x >= 1 && y >= 1");
        let goal = one("x + y >= 2");
        assert!(entails(&assumps, &goal).unwrap());
    }
}
