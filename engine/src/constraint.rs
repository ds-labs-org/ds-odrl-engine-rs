//! The leftOperand-to-claim mapping and supported operators (case study
//! Section 4.2).

use serde::{Deserialize, Serialize};

use crate::claims::Claims;

/// The three operators the Default Profile supports (Section 4.2). Named
/// explicitly, per the case study, so what is *not* supported — numeric
/// and date/time comparisons, `isPartOf`, nested constraint groups — is
/// never implied by omission from this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    #[serde(rename = "eq")]
    Eq,
    #[serde(rename = "neq")]
    Neq,
    #[serde(rename = "isAnyOf")]
    IsAnyOf,
}

/// One atomic ODRL constraint, resolved against a `Claims` map by exact
/// string match on `left_operand` (Section 4.2). This mirrors
/// `catalog_core::Constraint`'s atomic shape referenced by the case
/// study — no nested `and`/`or`/`xone` groups, which that type does not
/// model either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    pub left_operand: String,
    pub operator: Operator,
    pub right_operand: String,
}

impl Constraint {
    pub fn new(
        left_operand: impl Into<String>,
        operator: Operator,
        right_operand: impl Into<String>,
    ) -> Self {
        Self {
            left_operand: left_operand.into(),
            operator,
            right_operand: right_operand.into(),
        }
    }

    /// Evaluates this constraint against `claims`.
    ///
    /// A `left_operand` absent from `claims` is a **miss, not an error**
    /// (Section 4.2) — this holds uniformly across all three operators,
    /// `neq` included: an absent claim does not satisfy `neq` merely
    /// because it fails to satisfy `eq`. The claims-map lookup, not the
    /// operator's own logic, decides the absent-key case.
    pub fn evaluate(&self, claims: &Claims) -> bool {
        let Some(value) = claims.get(&self.left_operand) else {
            return false;
        };

        match self.operator {
            Operator::Eq => value.matches(&self.right_operand),
            Operator::Neq => !value.matches(&self.right_operand),
            Operator::IsAnyOf => {
                let candidates: Vec<&str> = self.right_operand.split(',').collect();
                value.matches_any(&candidates)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::ClaimValue;

    fn claims_with(pairs: &[(&str, ClaimValue)]) -> Claims {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn eq_matches_single_valued_claim() {
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let constraint = Constraint::new("sub", Operator::Eq, "alice");
        assert!(constraint.evaluate(&claims));

        let constraint = Constraint::new("sub", Operator::Eq, "bob");
        assert!(!constraint.evaluate(&claims));
    }

    #[test]
    fn eq_matches_membership_in_multi_valued_claim() {
        let claims = claims_with(&[(
            "nationality",
            ClaimValue::Multi(vec!["FR".into(), "DE".into()]),
        )]);
        assert!(Constraint::new("nationality", Operator::Eq, "DE").evaluate(&claims));
        assert!(!Constraint::new("nationality", Operator::Eq, "US").evaluate(&claims));
    }

    #[test]
    fn neq_negates_eq_when_the_claim_is_present() {
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(Constraint::new("sub", Operator::Neq, "bob").evaluate(&claims));
        assert!(!Constraint::new("sub", Operator::Neq, "alice").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_eq() {
        let claims = claims_with(&[]);
        assert!(!Constraint::new("sub", Operator::Eq, "alice").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_neq_too_not_the_negation_of_a_miss() {
        let claims = claims_with(&[]);
        // The Section 4.2 posture is: absent key => not satisfied, full
        // stop, for every operator. It would be wrong (double-negative)
        // for `neq` to read a missing claim as satisfying itself just
        // because `eq` would have missed.
        assert!(!Constraint::new("sub", Operator::Neq, "alice").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_is_any_of() {
        let claims = claims_with(&[]);
        assert!(!Constraint::new("scope", Operator::IsAnyOf, "read,write").evaluate(&claims));
    }

    #[test]
    fn is_any_of_splits_the_right_operand_on_commas() {
        let claims = claims_with(&[("scope", ClaimValue::Single("write".into()))]);
        assert!(Constraint::new("scope", Operator::IsAnyOf, "read,write,delete").evaluate(&claims));
        assert!(!Constraint::new("scope", Operator::IsAnyOf, "read,delete").evaluate(&claims));
    }

    #[test]
    fn is_any_of_matches_any_element_of_a_multi_valued_claim() {
        let claims = claims_with(&[(
            "nationality",
            ClaimValue::Multi(vec!["FR".into(), "DE".into()]),
        )]);
        assert!(Constraint::new("nationality", Operator::IsAnyOf, "US,DE,GB").evaluate(&claims));
        assert!(!Constraint::new("nationality", Operator::IsAnyOf, "US,GB").evaluate(&claims));
    }

    #[test]
    fn is_any_of_with_empty_right_operand_matches_nothing_but_an_empty_claim_value() {
        // Section 7's own documented limitation: no escaping convention,
        // so an empty string just splits into a single empty candidate.
        let claims = claims_with(&[("scope", ClaimValue::Single("read".into()))]);
        assert!(!Constraint::new("scope", Operator::IsAnyOf, "").evaluate(&claims));

        let empty_claim = claims_with(&[("scope", ClaimValue::Single(String::new()))]);
        assert!(Constraint::new("scope", Operator::IsAnyOf, "").evaluate(&empty_claim));
    }

    #[test]
    fn eq_against_a_multi_valued_claim_does_not_match_a_concatenation() {
        // Type-mismatch edge case: the right_operand must equal one
        // element, not the claim's list "as if" it were a single string.
        let claims = claims_with(&[(
            "scope",
            ClaimValue::Multi(vec!["read".into(), "write".into()]),
        )]);
        assert!(!Constraint::new("scope", Operator::Eq, "read,write").evaluate(&claims));
        assert!(Constraint::new("scope", Operator::Eq, "read").evaluate(&claims));
    }

    #[test]
    fn deserializes_from_the_documented_json_shape() {
        let json = r#"{"left_operand":"nationality","operator":"isAnyOf","right_operand":"FR,DE"}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();
        assert_eq!(constraint.left_operand, "nationality");
        assert_eq!(constraint.operator, Operator::IsAnyOf);
        assert_eq!(constraint.right_operand, "FR,DE");
    }
}
