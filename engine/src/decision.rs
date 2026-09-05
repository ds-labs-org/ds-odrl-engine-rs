//! Enforcement decision semantics — this proposal's choice (case study
//! Section 4.3).
//!
//! Stated per `(Policy, claims)`, with no requested-action parameter: this
//! is a whole-policy decision, not (yet) a per-action one. `Rule::action`
//! exists here because a `Rule` carries one in the ODRL model this engine
//! consumes, but Section 4.3's algorithm itself never branches on it —
//! that is reserved for the profile-driven action/vocabulary check
//! (Section 4.4, not implemented by this module), which is why `Decision`
//! below has no `Error` variant yet.

use serde::{Deserialize, Serialize};

use crate::claims::Claims;
use crate::constraint::Constraint;

/// One permission or prohibition rule: an action plus the constraints that
/// must all be satisfied for the rule to "match" a claims set (Section
/// 4.3). An empty `constraints` list matches vacuously — an unconstrained
/// rule always applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
}

impl Rule {
    pub fn new(action: impl Into<String>, constraints: Vec<Constraint>) -> Self {
        Self {
            action: action.into(),
            constraints,
        }
    }

    fn matches(&self, claims: &Claims) -> bool {
        self.constraints.iter().all(|c| c.evaluate(claims))
    }
}

/// An ODRL Policy reduced to what Section 4.3's decision algorithm needs:
/// its permission and prohibition rules. Duties (Section 4.5) and the
/// profile/action machinery (Section 4.4) are deliberately not part of
/// this shape yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub permissions: Vec<Rule>,
    #[serde(default)]
    pub prohibitions: Vec<Rule>,
}

/// The result of Section 4.3's algorithm. No `Error` variant: that result
/// belongs to Section 4.4's unrecognized-action check, which this module
/// does not implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

/// Section 4.3's decision algorithm: deny-overrides, then a permission
/// requirement.
///
/// The ODRL Community Group's `Behaviour` axis (Section 3.6) names the
/// alternative this proposal departs from: a strict `closed` reading
/// ("anything not permitted is prohibited") would deny a policy with an
/// empty `permissions` list outright; this proposal instead treats that
/// one degenerate case as `open`, because an empty-`permissions` `Offer`
/// is the common harvested-data case, not the exception (Section 4.3).
pub fn decide(policy: &Policy, claims: &Claims) -> Decision {
    let denied_by_prohibition = policy.prohibitions.iter().any(|rule| rule.matches(claims));
    if denied_by_prohibition {
        return Decision::Deny;
    }

    let permission_requirement_met =
        policy.permissions.is_empty() || policy.permissions.iter().any(|rule| rule.matches(claims));

    if permission_requirement_met {
        Decision::Allow
    } else {
        Decision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::ClaimValue;
    use crate::constraint::Operator;

    fn claims_with(pairs: &[(&str, ClaimValue)]) -> Claims {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn permission_only_unconstrained_allows() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims), Decision::Allow);
    }

    #[test]
    fn permission_only_satisfied_constraint_allows() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            prohibitions: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims), Decision::Allow);
    }

    #[test]
    fn prohibition_overrides_a_matching_permission() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims),
            Decision::Deny,
            "deny-overrides: a matching prohibition wins even though a permission also matches"
        );
    }

    #[test]
    fn unsatisfied_prohibition_does_not_deny() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "bob")],
            )],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims), Decision::Allow);
    }

    #[test]
    fn no_matching_permission_denies_closed_default() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "bob")],
            )],
            prohibitions: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims),
            Decision::Deny,
            "closed default: a non-empty permissions list that never matches must deny"
        );
    }

    #[test]
    fn empty_permissions_list_is_the_open_exception() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![],
        };
        let claims = claims_with(&[]);
        assert_eq!(
            decide(&policy, &claims),
            Decision::Allow,
            "Section 4.3's named departure: no permission rules at all is treated as open"
        );
    }

    #[test]
    fn empty_permissions_still_yields_to_a_matching_prohibition() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new("read", vec![])],
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims), Decision::Deny);
    }

    #[test]
    fn any_one_matching_permission_is_enough_not_all() {
        let policy = Policy {
            permissions: vec![
                Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")]),
                Rule::new("write", vec![]),
            ],
            prohibitions: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims),
            Decision::Allow,
            "multiple permission rules are alternative grants, not a conjunction"
        );
    }

    #[test]
    fn any_one_matching_prohibition_is_enough() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![
                Rule::new("write", vec![Constraint::new("sub", Operator::Eq, "bob")]),
                Rule::new("read", vec![]),
            ],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims), Decision::Deny);
    }

    #[test]
    fn deserializes_from_the_documented_json_shape() {
        let json = r#"{
            "permissions": [{"action": "read", "constraints": []}],
            "prohibitions": []
        }"#;
        let policy: Policy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.permissions.len(), 1);
        assert_eq!(policy.permissions[0].action, "read");
    }

    #[test]
    fn missing_permissions_and_prohibitions_fields_default_to_empty() {
        let policy: Policy = serde_json::from_str("{}").unwrap();
        assert!(policy.permissions.is_empty());
        assert!(policy.prohibitions.is_empty());
    }
}
