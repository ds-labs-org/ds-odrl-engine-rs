//! The flat identity-claims model (case study Section 4.1).
//!
//! A claims map is sourced from whatever identity the host already trusts;
//! this engine never decodes a JWT or other identity-presentation format
//! itself (Section 4.1). The vocabulary in the Default Profile table is
//! illustrative — `sub`, `nationality`, `scope`, etc. — not an exhaustive
//! enum, so `Claims` is a plain open-ended string-keyed map, not a struct
//! with named fields.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single claim's value: either one string, or a list of strings for a
/// multi-valued claim such as `scope` or `nationality` (Section 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClaimValue {
    Single(String),
    Multi(Vec<String>),
}

impl ClaimValue {
    /// `eq` semantics: `candidate` string-equals the value, or is a member
    /// of the value list for a multi-valued claim (Section 4.2).
    pub fn matches(&self, candidate: &str) -> bool {
        match self {
            ClaimValue::Single(value) => value == candidate,
            ClaimValue::Multi(values) => values.iter().any(|v| v == candidate),
        }
    }

    /// `isAnyOf` semantics: satisfied if the value (or any element of a
    /// multi-valued claim) matches any of `candidates` (Section 4.2).
    pub fn matches_any(&self, candidates: &[&str]) -> bool {
        match self {
            ClaimValue::Single(value) => candidates.contains(&value.as_str()),
            ClaimValue::Multi(values) => values
                .iter()
                .any(|v| candidates.contains(&v.as_str())),
        }
    }
}

impl From<String> for ClaimValue {
    fn from(value: String) -> Self {
        ClaimValue::Single(value)
    }
}

impl From<&str> for ClaimValue {
    fn from(value: &str) -> Self {
        ClaimValue::Single(value.to_string())
    }
}

impl From<Vec<String>> for ClaimValue {
    fn from(values: Vec<String>) -> Self {
        ClaimValue::Multi(values)
    }
}

/// A flat claims map: `HashMap<String, ClaimValue>` (Section 4.1). An
/// unrecognized key is not an error at this layer — `Constraint::evaluate`
/// treats it as a miss (Section 4.2).
pub type Claims = HashMap<String, ClaimValue>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_value_matches_itself_only() {
        let value = ClaimValue::Single("alice".to_string());
        assert!(value.matches("alice"));
        assert!(!value.matches("bob"));
    }

    #[test]
    fn multi_value_matches_any_member() {
        let value = ClaimValue::Multi(vec!["FR".to_string(), "DE".to_string()]);
        assert!(value.matches("FR"));
        assert!(value.matches("DE"));
        assert!(!value.matches("US"));
    }

    #[test]
    fn matches_any_checks_every_candidate_against_every_member() {
        let single = ClaimValue::Single("read".to_string());
        assert!(single.matches_any(&["write", "read"]));
        assert!(!single.matches_any(&["write", "delete"]));

        let multi = ClaimValue::Multi(vec!["FR".to_string(), "DE".to_string()]);
        assert!(multi.matches_any(&["US", "DE"]));
        assert!(!multi.matches_any(&["US", "GB"]));
    }

    #[test]
    fn rejects_a_claim_value_that_is_neither_string_nor_string_array() {
        // Section 4.1: a top-level string or array-of-strings field
        // becomes a claim; anything else (a number, a nested object such
        // as OIDC4IDA's verified_claims.claims) is not auto-flattened and
        // must not silently coerce into a ClaimValue.
        assert!(serde_json::from_str::<ClaimValue>("42").is_err());
        assert!(serde_json::from_str::<ClaimValue>(r#"{"nested":"object"}"#).is_err());
        assert!(serde_json::from_str::<ClaimValue>("null").is_err());
    }

    #[test]
    fn deserializes_from_json_string_or_array() {
        let single: ClaimValue = serde_json::from_str("\"alice\"").unwrap();
        assert_eq!(single, ClaimValue::Single("alice".to_string()));

        let multi: ClaimValue = serde_json::from_str("[\"FR\", \"DE\"]").unwrap();
        assert_eq!(
            multi,
            ClaimValue::Multi(vec!["FR".to_string(), "DE".to_string()])
        );
    }
}
