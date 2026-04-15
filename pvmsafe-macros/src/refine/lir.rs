use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinearExpr {
    pub terms: BTreeMap<String, i128>,
    pub constant: i128,
}

impl LinearExpr {
    pub fn constant(c: i128) -> Self {
        Self {
            terms: BTreeMap::new(),
            constant: c,
        }
    }

    pub fn var(name: &str) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(name.to_string(), 1);
        Self {
            terms,
            constant: 0,
        }
    }

    pub fn add(&self, other: &Self) -> Option<Self> {
        let mut terms = self.terms.clone();
        for (k, v) in &other.terms {
            let entry = terms.entry(k.clone()).or_insert(0);
            *entry = entry.checked_add(*v)?;
            if *entry == 0 {
                terms.remove(k);
            }
        }
        Some(Self {
            terms,
            constant: self.constant.checked_add(other.constant)?,
        })
    }

    pub fn neg(&self) -> Option<Self> {
        let mut terms = BTreeMap::new();
        for (k, v) in &self.terms {
            terms.insert(k.clone(), v.checked_neg()?);
        }
        Some(Self {
            terms,
            constant: self.constant.checked_neg()?,
        })
    }

    pub fn sub(&self, other: &Self) -> Option<Self> {
        self.add(&other.neg()?)
    }

    pub fn scale(&self, k: i128) -> Option<Self> {
        let mut terms = BTreeMap::new();
        for (name, v) in &self.terms {
            let scaled = v.checked_mul(k)?;
            if scaled != 0 {
                terms.insert(name.clone(), scaled);
            }
        }
        Some(Self {
            terms,
            constant: self.constant.checked_mul(k)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Constraint {
    pub expr: LinearExpr,
}

impl Constraint {
    pub fn new(expr: LinearExpr) -> Self {
        Self { expr }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_combines_coefficients() {
        let a = LinearExpr::var("x").add(&LinearExpr::var("x")).unwrap();
        assert_eq!(a.terms.get("x"), Some(&2));
    }

    #[test]
    fn add_drops_zeroed_terms() {
        let a = LinearExpr::var("x")
            .add(&LinearExpr::var("x").neg().unwrap())
            .unwrap();
        assert!(a.terms.is_empty());
        assert_eq!(a.constant, 0);
    }

    #[test]
    fn sub_negates() {
        let a = LinearExpr::var("x").sub(&LinearExpr::var("y")).unwrap();
        assert_eq!(a.terms.get("x"), Some(&1));
        assert_eq!(a.terms.get("y"), Some(&-1));
    }

    #[test]
    fn scale_multiplies() {
        let a = LinearExpr::var("x").scale(5).unwrap();
        assert_eq!(a.terms.get("x"), Some(&5));
    }

    #[test]
    fn scale_by_zero_clears() {
        let a = LinearExpr::var("x").scale(0).unwrap();
        assert!(a.terms.is_empty());
        assert_eq!(a.constant, 0);
    }

    #[test]
    fn overflow_detected_on_add() {
        let big = LinearExpr::constant(i128::MAX);
        assert!(big.add(&LinearExpr::constant(1)).is_none());
    }

    #[test]
    fn overflow_detected_on_scale() {
        let big = LinearExpr::constant(i128::MAX / 2 + 1);
        assert!(big.scale(3).is_none());
    }
}
