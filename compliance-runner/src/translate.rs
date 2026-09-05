//! Adapts one (policy, request) fixture pair into Section 5.2's JSON
//! request contract, or determines that the pair needs a capability
//! Section 7 names as out of scope and returns a one-line, cited reason
//! instead.
//!
//! **The adapter's own translation convention, stated once here because
//! nothing in Section 5.2 specifies it:** the engine's `decide` algorithm
//! (`engine::decision`) takes no requested-action or target parameter at
//! all — it is a whole-`(Policy, claims)` decision (`decision.rs`'s own
//! module doc says so plainly). A real host is therefore responsible for
//! having already scoped a harvested `Policy`'s rules to the one
//! action/target actually being evaluated *before* it ever reaches this
//! engine (Section 5.2: "the host serializes the harvested `Vec<Policy>`
//! for one dataset directly, with no translation layer"). This compliance
//! runner plays that host role: for each of a policy's permission/
//! prohibition rules, it keeps the rule only if the rule's own
//! `odrl:assignee`/`odrl:action`/`odrl:target` (when present) match the
//! request's — an `odrl:assignee` becomes a `sub eq <name>` constraint
//! (Section 4.1's `sub` claim), since `WirePolicy.assignee` itself is
//! purely descriptive and has no effect on `decide` (`as_decision_policy`
//! drops it). A rule that survives this scoping has its `action` field
//! set to the request's own action — the wire contract has nowhere else
//! to put "the action being evaluated" (no `action` field on `Request`
//! itself), so this is the one place that phrase can take effect.
//!
//! One narrow exception to exact-string action matching, taken directly
//! from the W3C ODRL Vocabulary & Expression (<https://www.w3.org/TR/vocab-odrl/>)
//! rather than the general action-implication problem: `odrl:read` and
//! `odrl:distribute` are formally declared "Included In: use" in that
//! vocabulary, and this fixture corpus's own expected-report ground truth
//! confirms the same holds for `write` (Active/Satisfied whenever a `use`
//! rule meets a `read`/`write` request). `odrl:sell` and `odrl:give`,
//! by contrast, are declared "Included In: transfer" — a sibling category,
//! not a child of `use` — and the ground truth agrees: a `use` rule against
//! a `sell` request reports Inactive/Unsatisfied. So a rule scoped to `use`
//! is treated as pertaining to any request action EXCEPT the vocabulary's
//! own transfer-category actions (`transfer`, `sell`, `give`) — a fixed,
//! citable vocabulary fact for the terms this corpus actually exercises,
//! not a general solution to action-taxonomy implication. Any other
//! includedIn/implies chain (a *profile's own* declared extensions, per
//! Section 3.5's Profile Mechanism) remains unevaluated, per Section 7.
//!
//! If **no** rule of a policy survives this scoping, the policy has
//! nothing to say about the request under evaluation and is omitted from
//! `policies` entirely — deliberately not included as an empty-rules
//! shell, which would instead trigger `decide`'s own *open* exception for
//! a policy with zero permissions (Section 4.3) and silently invert the
//! intended "this policy doesn't apply here" outcome into an unconditional
//! Allow.

use engine::{ClaimValue, Claims, Constraint, DutyMode, Operator, Request, RequestConfig, Rule, WirePolicy};

use crate::odrl::{ConstraintForm, PartyRef, PolicyInfo, RequestInfo, RuleKind, TargetRef};

pub enum Translation {
    Ready(Request),
    Skip(String),
}

const S7_PARTY_COLLECTION: &str = "odrl:assignee is an odrl:PartyCollection; membership (odrl:partOf) is asserted only in the state-of-the-world graph, not carried in the caller's claims — the flat claims model admits only a top-level string/array-of-string field per claim (Section 4.1), the same representability limit Section 7 names for structured attributes (\"Structured PID attributes cannot be carried at all\"), and Section 4.4 separately disclaims ODRL's Party functional roles as outside this engine's narrowed Profile Mechanism reading.";

const S7_ASSET_COLLECTION: &str = "odrl:target is an odrl:AssetCollection; this engine's wire contract carries no target/resource concept at all (decision::Rule is {action, constraints} only, Section 5.2), so resource-collection membership cannot be represented — the same class of gap Section 7 names for structured claims and Party functional roles (Section 4.1, 4.4).";

const S7_NESTED_DUTY: &str = "permission carries a per-permission odrl:duty (ODRL's finer pre/post-condition form nested inside one Permission); Section 7: \"Per-permission nested duties ... are not modeled: catalog_core::Rule has no nested-duty field\" — only policy-level obligations (Section 4.5) are evaluated.";

const S7_LOGICAL_CONSTRAINT: &str = "constraint is an odrl:LogicalConstraint (odrl:and/odrl:or/odrl:xone); Section 7: \"Nested ODRL logical constraint groups ... remain inherited-limitation out of scope (Section 4.2)\" — catalog_core::Constraint only models atomic constraints.";

const S7_DATETIME: &str = "constraint's odrl:leftOperand is odrl:dateTime; Section 7: \"Numeric and date/time comparison operators ... remain unimplemented in the Default Profile (Section 4.2)\".";

/// ODRL Vocabulary actions formally "Included In: transfer" rather than
/// "Included In: use" (<https://www.w3.org/TR/vocab-odrl/> §3.12) — the set
/// a generic `odrl:use` rule does NOT cover, per this module's doc comment.
const TRANSFER_CATEGORY_ACTIONS: &[&str] = &["transfer", "sell", "give"];

fn unsupported_operator(op: &str) -> String {
    format!(
        "constraint operator odrl:{op} has no equivalent in the Default Profile's eq/neq/isAnyOf set; Section 7: \"Numeric and date/time comparison operators ... and isPartOf/range-membership beyond isAnyOf remain unimplemented in the Default Profile (Section 4.2)\"."
    )
}

pub fn translate(policy: &PolicyInfo, req: &RequestInfo, dataset_id: &str) -> Translation {
    let mut permissions = Vec::new();
    let mut prohibitions = Vec::new();

    for rule in &policy.rules {
        let action_matches_exactly = match &rule.action {
            None => true,
            Some(a) => a == &req.action,
        };
        let action_is_generic_use = matches!(&rule.action, Some(a) if a == "use")
            && !TRANSFER_CATEGORY_ACTIONS.contains(&req.action.as_str());
        if !(action_matches_exactly || action_is_generic_use) {
            continue;
        }

        if matches!(rule.assignee, Some(PartyRef::Collection)) {
            return Translation::Skip(S7_PARTY_COLLECTION.to_string());
        }
        if matches!(rule.target, Some(TargetRef::Collection)) {
            return Translation::Skip(S7_ASSET_COLLECTION.to_string());
        }

        let assignee_matches = match &rule.assignee {
            None => true,
            Some(PartyRef::Individual(name)) => name == &req.assignee,
            Some(PartyRef::Collection) => unreachable!("handled above"),
        };
        let target_matches = match &rule.target {
            None => true,
            Some(TargetRef::Individual(name)) => Some(name) == req.target.as_ref(),
            Some(TargetRef::Collection) => unreachable!("handled above"),
        };
        if !(assignee_matches && target_matches) {
            continue;
        }

        if rule.has_nested_duty {
            return Translation::Skip(S7_NESTED_DUTY.to_string());
        }

        let mut constraints = Vec::new();
        if let Some(PartyRef::Individual(name)) = &rule.assignee {
            constraints.push(Constraint::new("sub", Operator::Eq, name.clone()));
        }
        match &rule.constraint {
            None => {}
            Some(ConstraintForm::Logical) => return Translation::Skip(S7_LOGICAL_CONSTRAINT.to_string()),
            Some(ConstraintForm::Atomic { left_operand, operator, right_operand }) => {
                if left_operand == "dateTime" {
                    return Translation::Skip(S7_DATETIME.to_string());
                }
                let op = match operator.as_str() {
                    "eq" => Operator::Eq,
                    "neq" => Operator::Neq,
                    other => return Translation::Skip(unsupported_operator(other)),
                };
                constraints.push(Constraint::new(left_operand.clone(), op, right_operand.clone()));
            }
        }

        let engine_rule = Rule::new(req.action.clone(), constraints);
        match rule.kind {
            RuleKind::Permission => permissions.push(engine_rule),
            RuleKind::Prohibition => prohibitions.push(engine_rule),
        }
    }

    let policies = if permissions.is_empty() && prohibitions.is_empty() {
        Vec::new()
    } else {
        vec![WirePolicy {
            id: policy.id.clone(),
            kind: "Set".to_string(),
            assigner: "urn:uuid:unspecified-assigner".to_string(),
            assignee: None,
            permissions,
            prohibitions,
            obligations: Vec::new(),
        }]
    };

    let mut claims: Claims = Claims::new();
    claims.insert("sub".to_string(), ClaimValue::from(req.assignee.clone()));

    Translation::Ready(Request {
        dataset_id: dataset_id.to_string(),
        config: RequestConfig {
            recognized_actions: vec![req.action.clone()],
            duty_mode: DutyMode::Advise,
        },
        policies,
        claims,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::odrl::RuleInfo;

    fn req(assignee: &str, action: &str, target: Option<&str>) -> RequestInfo {
        RequestInfo { assignee: assignee.to_string(), action: action.to_string(), target: target.map(String::from) }
    }

    fn policy(id: &str, rules: Vec<RuleInfo>) -> PolicyInfo {
        PolicyInfo { id: id.to_string(), rules }
    }

    fn unconstrained(kind: RuleKind) -> RuleInfo {
        RuleInfo { kind, assignee: None, action: None, target: None, constraint: None, has_nested_duty: false }
    }

    #[test]
    fn unconstrained_permission_allows_any_assignee_and_action() {
        let p = policy("p1", vec![unconstrained(RuleKind::Permission)]);
        let r = req("alice", "read", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Allow);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn unconstrained_prohibition_denies_any_assignee_and_action() {
        let p = policy("p2", vec![unconstrained(RuleKind::Prohibition)]);
        let r = req("bob", "sell", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn assignee_scoped_permission_denies_a_non_matching_caller_by_omitting_the_policy() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p3", vec![rule]);
        let r = req("bob", "read", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                assert!(
                    wire.policies.is_empty(),
                    "a rule scoped to a different assignee must not survive translation as an \
                     empty-permissions shell, which would trigger decide()'s own open exception"
                );
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn target_scoped_permission_denies_a_different_target() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: Some(TargetRef::Individual("x".to_string())),
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p4", vec![rule]);
        let r = req("alice", "read", Some("y"));
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                assert!(wire.policies.is_empty());
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn party_collection_assignee_is_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Collection),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p5", vec![rule]);
        let r = req("alice", "read", None);
        assert!(matches!(translate(&p, &r, "ds1"), Translation::Skip(_)));
    }

    #[test]
    fn datetime_constraint_is_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Atomic {
                left_operand: "dateTime".to_string(),
                operator: "eq".to_string(),
                right_operand: "2024-02-12T11:20:10.999Z".to_string(),
            }),
            has_nested_duty: false,
        };
        let p = policy("p6", vec![rule]);
        let r = req("alice", "read", None);
        assert!(matches!(translate(&p, &r, "ds1"), Translation::Skip(_)));
    }

    #[test]
    fn generic_use_permission_allows_a_more_specific_request_action() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p7", vec![rule]);
        let r = req("alice", "read", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Allow);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn generic_use_prohibition_denies_a_more_specific_request_action() {
        let rule = RuleInfo {
            kind: RuleKind::Prohibition,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p8", vec![rule]);
        let r = req("alice", "write", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn generic_use_rule_still_respects_assignee_scoping() {
        let rule = RuleInfo {
            kind: RuleKind::Prohibition,
            assignee: Some(PartyRef::Individual("bob".to_string())),
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p9", vec![rule]);
        let r = req("alice", "read", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                assert!(wire.policies.is_empty());
                let response = engine::evaluate_request(&wire);
                assert_eq!(
                    response.decision,
                    engine::WireDecision::Deny,
                    "assignee mismatch (bob vs alice) excludes the rule regardless of the \
                     generic 'use' action matching 'read'"
                );
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn generic_use_permission_does_not_cover_a_transfer_category_action() {
        // Mirrors the vendored fixture testcase-010-alice-sell: policy-3
        // ("everybody can do use") against a `sell` request. The upstream
        // expected report is Inactive/Unsatisfied, not Active — `sell` is
        // "Included In: transfer" per the W3C ODRL Vocabulary, a sibling
        // category to `use`, not a child of it.
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            has_nested_duty: false,
        };
        let p = policy("p10", vec![rule]);
        let r = req("alice", "sell", None);
        match translate(&p, &r, "ds1") {
            Translation::Ready(wire) => {
                assert!(
                    wire.policies.is_empty(),
                    "a generic 'use' permission must not be treated as covering 'sell'"
                );
                let response = engine::evaluate_request(&wire);
                assert_eq!(response.decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }
}
