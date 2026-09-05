//! Enforcement decision semantics — this proposal's choice (case study
//! Section 4.3), extended with Section 4.4's profile-driven action check
//! and Section 4.5's policy-level duty evaluation.
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
use crate::profile::{DutyMode, ResolvedConfig};

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

    /// Section 4.5's duty-satisfaction check — deliberately *not* the same
    /// as `matches`. An unconstrained permission/prohibition matches
    /// vacuously (an unconditional grant always applies), but an
    /// unconstrained duty is the opposite case: an unconditional "must do
    /// Y" this engine has no claims-based way to verify, so it is
    /// unresolved, not satisfied. Satisfied therefore requires at least
    /// one constraint, all of which match.
    fn duty_satisfied(&self, claims: &Claims) -> bool {
        !self.constraints.is_empty() && self.matches(claims)
    }
}

/// An ODRL Policy reduced to what Section 4.3's decision algorithm needs
/// — its permission and prohibition rules — plus Section 4.5's
/// policy-level duties. The profile/action machinery of Section 4.4
/// applies uniformly across all three lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub permissions: Vec<Rule>,
    #[serde(default)]
    pub prohibitions: Vec<Rule>,
    #[serde(default)]
    pub obligations: Vec<Rule>,
}

/// Which of a `Policy`'s rule lists an `UnrecognizedAction` (or, for
/// `Duty`, an `UnresolvedDuty`) came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Permission,
    Prohibition,
    Duty,
}

impl fmt::Display for RuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleKind::Permission => write!(f, "permission"),
            RuleKind::Prohibition => write!(f, "prohibition"),
            RuleKind::Duty => write!(f, "duty"),
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

/// Section 4.5's outcome for one policy-level duty that did not resolve
/// (its constraints did not all match, or it is unconditional): named and
/// indexed the same way `UnrecognizedAction` is, since `catalog_core::Rule`
/// carries no duty-level identifier of its own either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedDuty {
    pub action: String,
    pub duty_index: usize,
}

/// `decide`'s full result: Section 4.3/4.4's `Decision`, plus Section
/// 4.5's list of duties this engine could not confirm. The list is
/// populated independently of `duty_mode` — it is the caller's advisory
/// record of every unresolved duty, whether or not that duty also forced
/// `decision` to `Deny` under `duty_mode: "deny"`. Section 5.2's `duties`
/// response field, in the next revision, filters this by `duty_mode`
/// before it reaches a caller; this type is the pre-filter source of
/// truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionOutcome {
    pub decision: Decision,
    pub unresolved_duties: Vec<UnresolvedDuty>,
}

/// Section 4.4's unrecognized-action check, ahead of Section 4.3's
/// algorithm: prohibitions first, then permissions, then Section 4.5's
/// obligations, so the earliest rule (in the same precedence order
/// `decide` itself evaluates) that names an action outside
/// `config.recognized_actions` is the one reported. A duty's `action` is
/// checked on the same footing as a permission's or prohibition's — an
/// obligation this engine cannot even identify is no safer to guess about
/// than a permission or prohibition it cannot identify (Section 4.4).
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

    for (rule_index, rule) in policy.obligations.iter().enumerate() {
        if !config.recognizes(&rule.action) {
            return Some(UnrecognizedAction {
                action: rule.action.clone(),
                rule_kind: RuleKind::Duty,
                rule_index,
            });
        }
    }

    None
}

/// Section 4.5's duty evaluation: every policy-level duty whose
/// constraints do not all match (or which has none at all — an
/// unconditional duty, Section 4.5) is unresolved.
fn unresolved_duties(policy: &Policy, claims: &Claims) -> Vec<UnresolvedDuty> {
    policy
        .obligations
        .iter()
        .enumerate()
        .filter(|(_, duty)| !duty.duty_satisfied(claims))
        .map(|(duty_index, duty)| UnresolvedDuty {
            action: duty.action.clone(),
            duty_index,
        })
        .collect()
}

/// Section 4.3's decision algorithm: deny-overrides, then a permission
/// requirement — gated by Section 4.4's unrecognized-action check, and
/// followed by Section 4.5's duty evaluation.
///
/// The ODRL Community Group's `Behaviour` axis (Section 3.6) names the
/// alternative this proposal departs from: a strict `closed` reading
/// ("anything not permitted is prohibited") would deny a policy with an
/// empty `permissions` list outright; this proposal instead treats that
/// one degenerate case as `open`, because an empty-`permissions` `Offer`
/// is the common harvested-data case, not the exception (Section 4.3).
///
/// Duty evaluation (Section 4.5) runs after the permission/prohibition
/// decision is reached, and can only ever *tighten* it: under
/// `duty_mode: "deny"`, an unresolved duty overrides an otherwise-`Allow`
/// decision to `Deny` (a decision already `Deny` via a matching
/// prohibition is unaffected — it cannot be denied twice). Under
/// `duty_mode: "advise"`, unresolved duties never change `decision`,
/// only the returned `unresolved_duties` list.
pub fn decide(policy: &Policy, claims: &Claims, config: &ResolvedConfig) -> DecisionOutcome {
    if let Some(unrecognized) = first_unrecognized_action(policy, config) {
        return DecisionOutcome {
            decision: Decision::Error(unrecognized),
            unresolved_duties: Vec::new(),
        };
    }

    let denied_by_prohibition = policy.prohibitions.iter().any(|rule| rule.matches(claims));

    let permission_requirement_met =
        policy.permissions.is_empty() || policy.permissions.iter().any(|rule| rule.matches(claims));

    let mut decision = if denied_by_prohibition {
        Decision::Deny
    } else if permission_requirement_met {
        Decision::Allow
    } else {
        Decision::Deny
    };

    let unresolved_duties = unresolved_duties(policy, claims);
    if config.duty_mode == DutyMode::Deny && !unresolved_duties.is_empty() {
        decision = Decision::Deny;
    }

    DecisionOutcome {
        decision,
        unresolved_duties,
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
        config_with_duty_mode(actions, DutyMode::Advise)
    }

    fn config_with_duty_mode(actions: &[&str], duty_mode: DutyMode) -> ResolvedConfig {
        crate::profile::resolve(&[Profile {
            id: "https://example.org/profiles/test".to_string(),
            recognized_actions: actions.iter().map(|a| a.to_string()).collect(),
            duty_mode,
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
            obligations: vec![],
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims, &all_actions_config()).decision, Decision::Allow);
    }

    #[test]
    fn permission_only_satisfied_constraint_allows() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            prohibitions: vec![],
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims, &all_actions_config()).decision, Decision::Allow);
    }

    #[test]
    fn prohibition_overrides_a_matching_permission() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config()).decision,
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
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims, &all_actions_config()).decision, Decision::Allow);
    }

    #[test]
    fn no_matching_permission_denies_closed_default() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "bob")],
            )],
            prohibitions: vec![],
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config()).decision,
            Decision::Deny,
            "closed default: a non-empty permissions list that never matches must deny"
        );
    }

    #[test]
    fn empty_permissions_list_is_the_open_exception() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![],
            obligations: vec![],
        };
        let claims = claims_with(&[]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config()).decision,
            Decision::Allow,
            "Section 4.3's named departure: no permission rules at all is treated as open"
        );
    }

    #[test]
    fn empty_permissions_still_yields_to_a_matching_prohibition() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new("read", vec![])],
            obligations: vec![],
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims, &all_actions_config()).decision, Decision::Deny);
    }

    #[test]
    fn any_one_matching_permission_is_enough_not_all() {
        let policy = Policy {
            permissions: vec![
                Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")]),
                Rule::new("write", vec![]),
            ],
            prohibitions: vec![],
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config()).decision,
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
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims, &all_actions_config()).decision, Decision::Deny);
    }

    #[test]
    fn deserializes_from_the_documented_json_shape() {
        let json = r#"{
            "permissions": [{"action": "read", "constraints": []}],
            "prohibitions": [],
            "obligations": [{"action": "notify", "constraints": []}]
        }"#;
        let policy: Policy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.permissions.len(), 1);
        assert_eq!(policy.permissions[0].action, "read");
        assert_eq!(policy.obligations.len(), 1);
        assert_eq!(policy.obligations[0].action, "notify");
    }

    #[test]
    fn missing_permissions_and_prohibitions_fields_default_to_empty() {
        let policy: Policy = serde_json::from_str("{}").unwrap();
        assert!(policy.permissions.is_empty());
        assert!(policy.prohibitions.is_empty());
        assert!(policy.obligations.is_empty());
    }

    #[test]
    fn recognized_action_is_evaluated_normally() {
        let policy = Policy {
            permissions: vec![Rule::new("distribute", vec![])],
            prohibitions: vec![],
            obligations: vec![],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["distribute"]);
        assert_eq!(
            decide(&policy, &claims, &config).decision,
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
            obligations: vec![],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read", "write"]);
        match decide(&policy, &claims, &config).decision {
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
            obligations: vec![],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read", "write"]);
        match decide(&policy, &claims, &config).decision {
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
            obligations: vec![],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_recognizing(&["read"]);
        assert!(
            matches!(decide(&policy, &claims, &config).decision, Decision::Error(_)),
            "the action check must not be skipped just because the rule would have missed anyway"
        );
    }

    #[test]
    fn profile_union_recognizes_actions_from_either_loaded_profile() {
        let policy = Policy {
            permissions: vec![Rule::new("modify", vec![])],
            prohibitions: vec![],
            obligations: vec![],
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
            decide(&policy, &claims, &config).decision,
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

    #[test]
    fn already_satisfied_duty_produces_no_unresolved_entry_and_does_not_affect_decision() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new(
                "notify",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Deny);
        let outcome = decide(&policy, &claims, &config);
        assert_eq!(
            outcome.decision,
            Decision::Allow,
            "a duty whose constraints all match is already satisfied, even under duty_mode: deny"
        );
        assert!(
            outcome.unresolved_duties.is_empty(),
            "a satisfied duty must not appear in the unresolved list: {:?}",
            outcome.unresolved_duties
        );
    }

    #[test]
    fn duty_mode_deny_forces_an_otherwise_allow_decision_to_deny_on_an_unresolved_duty() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("notify", vec![])],
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Deny);
        let outcome = decide(&policy, &claims, &config);
        assert_eq!(
            outcome.decision,
            Decision::Deny,
            "duty_mode: deny must override an otherwise-Allow decision when a duty is unresolved"
        );
        assert_eq!(
            outcome.unresolved_duties,
            vec![UnresolvedDuty {
                action: "notify".to_string(),
                duty_index: 0,
            }]
        );
    }

    #[test]
    fn duty_mode_deny_with_an_unconditional_duty_is_unresolved_not_vacuously_satisfied() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("delete-after-30-days", vec![])],
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "delete-after-30-days"], DutyMode::Deny);
        let outcome = decide(&policy, &claims, &config);
        assert_eq!(
            outcome.decision,
            Decision::Deny,
            "Section 4.5: an unconstrained duty has no claims-based way to be verified, so it \
             is unresolved (not vacuously satisfied the way an unconstrained permission is)"
        );
        assert_eq!(outcome.unresolved_duties.len(), 1);
    }

    #[test]
    fn duty_mode_advise_does_not_change_the_decision_but_surfaces_the_unresolved_duty() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("notify", vec![])],
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Advise);
        let outcome = decide(&policy, &claims, &config);
        assert_eq!(
            outcome.decision,
            Decision::Allow,
            "duty_mode: advise must never block an otherwise-Allow decision"
        );
        assert_eq!(
            outcome.unresolved_duties,
            vec![UnresolvedDuty {
                action: "notify".to_string(),
                duty_index: 0,
            }],
            "the unresolved duty must still be surfaced for the caller to act on"
        );
    }

    #[test]
    fn duty_mode_advise_does_not_rescue_a_decision_already_denied_by_a_prohibition() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new("read", vec![])],
            obligations: vec![Rule::new("notify", vec![])],
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Advise);
        let outcome = decide(&policy, &claims, &config);
        assert_eq!(outcome.decision, Decision::Deny);
        assert_eq!(outcome.unresolved_duties.len(), 1);
    }

    #[test]
    fn multiple_unresolved_duties_are_all_reported_with_their_own_indices() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![
                Rule::new("notify", vec![]),
                Rule::new(
                    "delete-after-30-days",
                    vec![Constraint::new("sub", Operator::Eq, "nobody")],
                ),
            ],
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_with_duty_mode(&["read", "notify", "delete-after-30-days"], DutyMode::Advise);
        let outcome = decide(&policy, &claims, &config);
        assert_eq!(outcome.decision, Decision::Allow);
        assert_eq!(
            outcome.unresolved_duties,
            vec![
                UnresolvedDuty {
                    action: "notify".to_string(),
                    duty_index: 0,
                },
                UnresolvedDuty {
                    action: "delete-after-30-days".to_string(),
                    duty_index: 1,
                },
            ]
        );
    }

    #[test]
    fn unrecognized_action_in_an_obligation_yields_error() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("anonymize", vec![])],
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read"]);
        match decide(&policy, &claims, &config).decision {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Duty);
                assert_eq!(unrecognized.rule_index, 0);
            }
            other => panic!(
                "an obligation naming an action outside recognized_actions must yield Error \
                 exactly as an unrecognized permission/prohibition action would, got {other:?}"
            ),
        }
    }
}
