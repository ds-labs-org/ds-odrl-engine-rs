//! Enforcement decision semantics — this proposal's choice (case study
//! Section 4.3), extended with Section 4.4's profile-driven action check.
//!
//! Stated per `(Policy, claims, config)`, with no requested-action
//! parameter: this is a whole-policy decision, not (yet) a per-action
//! one. Section 4.3's deny-overrides/permission-requirement algorithm
//! never branches on `Rule::action` itself — only Section 4.4's
//! unrecognized-action check does, ahead of that algorithm.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::claims::Claims;
use crate::constraint::Constraint;
use crate::profile::ResolvedConfig;

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

/// Which of a `Policy`'s two rule lists an `UnrecognizedAction` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Permission,
    Prohibition,
}

impl fmt::Display for RuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleKind::Permission => write!(f, "permission"),
            RuleKind::Prohibition => write!(f, "prohibition"),
        }
    }
}

/// Section 4.4's unrecognized-action outcome: naming both the action and
/// which rule of the policy raised it, since `catalog_core::Policy` has
/// no policy-level identifier of its own for this evaluation to cite
/// (Section 4.4's decision is per-`(Policy, claims, config)`, one policy
/// at a time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecognizedAction {
    pub action: String,
    pub rule_kind: RuleKind,
    pub rule_index: usize,
}

impl fmt::Display for UnrecognizedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unrecognized action \"{}\" in {} rule at index {}: no loaded profile's \
             recognized_actions includes it",
            self.action, self.rule_kind, self.rule_index
        )
    }
}

/// The result of Section 4.3's algorithm, extended by Section 4.4's
/// unrecognized-action check.
///
/// `Error` is deliberately its own outcome, not folded into `Deny`: per
/// Section 4.4, a broker consuming it **must** treat it as fail-closed at
/// its own boundary, but it is a configuration gap ("load a profile that
/// recognizes this action"), not a policy decision, and collapsing it
/// into `Deny` would make that distinction unrecoverable downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Error(UnrecognizedAction),
}

/// Section 4.4's unrecognized-action check, ahead of Section 4.3's
/// algorithm: prohibitions first, then permissions, so the earliest rule
/// (in the same precedence order `decide` itself evaluates) that names an
/// action outside `config.recognized_actions` is the one reported.
///
/// This checks *every* rule's action regardless of whether its
/// constraints would otherwise match — Section 4.4 is explicit that an
/// unrecognized action is unsafe to treat as an ordinary non-match in
/// either direction (silent fail-open inside a `Prohibition`, silent
/// fail-closed indistinguishable from an intended `Deny` inside a
/// `Permission`), so it cannot be left to `Rule::matches` to notice only
/// incidentally.
fn first_unrecognized_action(policy: &Policy, config: &ResolvedConfig) -> Option<UnrecognizedAction> {
    for (rule_index, rule) in policy.prohibitions.iter().enumerate() {
        if !config.recognizes(&rule.action) {
            return Some(UnrecognizedAction {
                action: rule.action.clone(),
                rule_kind: RuleKind::Prohibition,
                rule_index,
            });
        }
    }

    for (rule_index, rule) in policy.permissions.iter().enumerate() {
        if !config.recognizes(&rule.action) {
            return Some(UnrecognizedAction {
                action: rule.action.clone(),
                rule_kind: RuleKind::Permission,
                rule_index,
            });
        }
    }

    None
}

/// Section 4.3's decision algorithm: deny-overrides, then a permission
/// requirement — gated by Section 4.4's unrecognized-action check.
///
/// The ODRL Community Group's `Behaviour` axis (Section 3.6) names the
/// alternative this proposal departs from: a strict `closed` reading
/// ("anything not permitted is prohibited") would deny a policy with an
/// empty `permissions` list outright; this proposal instead treats that
/// one degenerate case as `open`, because an empty-`permissions` `Offer`
/// is the common harvested-data case, not the exception (Section 4.3).
pub fn decide(policy: &Policy, claims: &Claims, config: &ResolvedConfig) -> Decision {
    if let Some(unrecognized) = first_unrecognized_action(policy, config) {
        return Decision::Error(unrecognized);
    }

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
    use crate::profile::{DutyMode, Profile};

    fn claims_with(pairs: &[(&str, ClaimValue)]) -> Claims {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn config_recognizing(actions: &[&str]) -> ResolvedConfig {
        crate::profile::resolve(&[Profile {
            id: "https://example.org/profiles/test".to_string(),
            recognized_actions: actions.iter().map(|a| a.to_string()).collect(),
            duty_mode: DutyMode::Advise,
        }])
    }

    fn all_actions_config() -> ResolvedConfig {
        config_recognizing(&["read", "write"])
    }

    #[test]
    fn permission_only_unconstrained_allows() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims, &all_actions_config()), Decision::Allow);
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
        assert_eq!(decide(&policy, &claims, &all_actions_config()), Decision::Allow);
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
            decide(&policy, &claims, &all_actions_config()),
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
        assert_eq!(decide(&policy, &claims, &all_actions_config()), Decision::Allow);
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
            decide(&policy, &claims, &all_actions_config()),
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
            decide(&policy, &claims, &all_actions_config()),
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
        assert_eq!(decide(&policy, &claims, &all_actions_config()), Decision::Deny);
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
            decide(&policy, &claims, &all_actions_config()),
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
        assert_eq!(decide(&policy, &claims, &all_actions_config()), Decision::Deny);
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

    #[test]
    fn recognized_action_is_evaluated_normally() {
        let policy = Policy {
            permissions: vec![Rule::new("distribute", vec![])],
            prohibitions: vec![],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["distribute"]);
        assert_eq!(
            decide(&policy, &claims, &config),
            Decision::Allow,
            "an action present in the resolved config's recognized_actions is evaluated by \
             Section 4.3's algorithm as usual"
        );
    }

    #[test]
    fn unrecognized_action_in_a_permission_yields_error_naming_the_action_and_rule() {
        let policy = Policy {
            permissions: vec![Rule::new("anonymize", vec![])],
            prohibitions: vec![],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read", "write"]);
        match decide(&policy, &claims, &config) {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Permission);
                assert_eq!(unrecognized.rule_index, 0);
                let message = unrecognized.to_string();
                assert!(
                    message.contains("anonymize"),
                    "message should name the unrecognized action: {message}"
                );
                assert!(
                    message.contains("permission"),
                    "message should name which rule list raised it: {message}"
                );
            }
            other => panic!("expected Decision::Error, got {other:?}"),
        }
    }

    #[test]
    fn unrecognized_action_in_a_prohibition_yields_error_and_is_not_silently_permissive() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new("anonymize", vec![])],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read", "write"]);
        match decide(&policy, &claims, &config) {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Prohibition);
                assert_eq!(unrecognized.rule_index, 0);
            }
            other => panic!(
                "expected Decision::Error — a Prohibition with an unrecognized action must \
                 never be silently treated as a non-match (that would fail open), got {other:?}"
            ),
        }
    }

    #[test]
    fn unrecognized_action_is_reported_even_when_its_constraints_would_not_have_matched() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "anonymize",
                vec![Constraint::new("sub", Operator::Eq, "nobody")],
            )],
            prohibitions: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_recognizing(&["read"]);
        assert!(
            matches!(decide(&policy, &claims, &config), Decision::Error(_)),
            "the action check must not be skipped just because the rule would have missed anyway"
        );
    }

    #[test]
    fn profile_union_recognizes_actions_from_either_loaded_profile() {
        let policy = Policy {
            permissions: vec![Rule::new("modify", vec![])],
            prohibitions: vec![],
        };
        let claims = claims_with(&[]);
        let config = crate::profile::resolve(&[
            Profile {
                id: "https://example.org/profiles/a".to_string(),
                recognized_actions: vec!["read".to_string()],
                duty_mode: DutyMode::Advise,
            },
            Profile {
                id: "https://example.org/profiles/b".to_string(),
                recognized_actions: vec!["modify".to_string()],
                duty_mode: DutyMode::Deny,
            },
        ]);
        assert_eq!(
            decide(&policy, &claims, &config),
            Decision::Allow,
            "an action recognized by only one of two loaded profiles is still recognized by \
             the union (Section 4.4's named fail-open choice, implemented as specified)"
        );
        assert_eq!(
            config.duty_mode,
            DutyMode::Deny,
            "the resolved config's duty_mode is the strictest across loaded profiles"
        );
    }
}
