//! Adapts one (policy, request, state-of-the-world) fixture triple into
//! Section 5.2's JSON request contract, or determines that the fixture
//! needs a capability this engine genuinely lacks and returns a one-line,
//! cited reason instead.
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
//! **State-of-the-world facts become claims or membership checks, not
//! wire-contract changes.** Three more capabilities, added after the
//! above, all resolved by reading the fixture's own SOTW graph rather than
//! by extending `engine`'s Section 5.2 schema:
//!
//! - `odrl:dateTime` constraints: `engine::Operator` gained `lt`/`lteq`/
//!   `gt`/`gteq` (a real, additive Default Profile extension — see
//!   `engine::constraint`'s doc comment). This adapter resolves what "the
//!   current time" is from the SOTW's own `temp:currentTime dct:issued
//!   "..."` fact and injects it as an ordinary `dateTime` claim — the
//!   engine still just does claims-map lookups, unaware this one happens
//!   to mean "now" rather than an identity attribute.
//! - `odrl:and`/`odrl:or` logical constraints: expanded into disjunctive
//!   normal form (`to_dnf`) — an `and` becomes multiple constraints on the
//!   SAME rule (the engine's own `Rule::matches` already ANDs its
//!   `constraints` list), an `or` becomes MULTIPLE sibling rules, one per
//!   disjunct (`decide` already ORs a policy's permissions: "any one
//!   matching permission is enough, not all"). No engine change needed for
//!   either. `odrl:xone` is NOT modeled — nothing in this corpus uses it,
//!   and this engine can express "at least one" but not xone's "exactly
//!   one, not more" exclusivity, so it stays a genuine, cited skip rather
//!   than a silent (and wrong) approximation as `or`.
//! - `odrl:PartyCollection`/`odrl:AssetCollection` membership: resolved
//!   directly against the SOTW graph's own `<member> odrl:partOf
//!   <collection>` facts (`is_member_of`), the same way an
//!   `odrl:assignee`/`odrl:target` match already was for an individual —
//!   still no claim or wire-contract change, since membership is decided
//!   once, at translate time, not evaluated inside the engine.
//! - A per-permission `odrl:duty`: this engine still has no nested-duty
//!   *model* (Section 4.5 evaluates only policy-level obligations) — what
//!   this adapter does is narrower than that: it reads the SOTW's own
//!   `report:DutyReport` fact for that duty node (this vendored corpus's
//!   own way of stating "here is what actually happened" for a duty) and
//!   excludes the rule entirely when `report:deonticState` is
//!   `report:Violated`. A `Fulfilled` or `NonSet` (unknown) state leaves
//!   the rule in play, matching this corpus's own expected reports. This
//!   resolves the three fixture cases that exercise it; it is not a claim
//!   that per-permission duties are modeled in general.
//!
//! If **no** rule of a policy survives all of the above, the policy has
//! nothing to say about the request under evaluation and is omitted from
//! `policies` entirely — deliberately not included as an empty-rules
//! shell, which would instead trigger `decide`'s own *open* exception for
//! a policy with zero permissions (Section 4.3) and silently invert the
//! intended "this policy doesn't apply here" outcome into an unconditional
//! Allow.

use engine::{ClaimValue, Claims, Constraint, DutyMode, Operator, Request, RequestConfig, Rule, WirePolicy};

use crate::graph::{dct, local_name, odrl, report_ns, Graph};
use crate::odrl::{ConstraintForm, PartyRef, PolicyInfo, RequestInfo, RuleKind, TargetRef};

pub enum Translation {
    Ready(Request),
    Skip(String),
}

/// ODRL Vocabulary actions formally "Included In: transfer" rather than
/// "Included In: use" (<https://www.w3.org/TR/vocab-odrl/> §3.12) — the set
/// a generic `odrl:use` rule does NOT cover, per this module's doc comment.
const TRANSFER_CATEGORY_ACTIONS: &[&str] = &["transfer", "sell", "give"];

fn unsupported_operator(op: &str) -> String {
    format!(
        "constraint operator odrl:{op} has no equivalent in the Default Profile's eq/neq/isAnyOf/lt/lteq/gt/gteq set; Section 7: \"...isPartOf/range-membership beyond isAnyOf remain unimplemented in the Default Profile (Section 4.2)\"."
    )
}

fn xone_unsupported(branch_count: usize) -> String {
    format!(
        "constraint is an odrl:LogicalConstraint using odrl:xone over {branch_count} branch(es); this engine can express \"at least one\" via sibling permission rules (odrl:or) but not xone's \"exactly one, not more\" exclusivity, and nothing in this vendored corpus exercises it, so it is left genuinely unsupported rather than silently approximated as odrl:or."
    )
}

fn resolve_operator(local: &str) -> Result<Operator, String> {
    match local {
        "eq" => Ok(Operator::Eq),
        "neq" => Ok(Operator::Neq),
        "lt" => Ok(Operator::Lt),
        "lteq" => Ok(Operator::Lteq),
        "gt" => Ok(Operator::Gt),
        "gteq" => Ok(Operator::Gteq),
        other => Err(unsupported_operator(other)),
    }
}

/// Expands a (possibly Boolean-combined) constraint into disjunctive
/// normal form: an OR of AND-conjunctions, each a flat `Vec<Constraint>`
/// ready to attach to one engine `Rule`. See this module's doc comment for
/// why `and`/`or` need no engine change, and why `xone` is a hard error
/// here rather than an approximation.
fn to_dnf(form: &ConstraintForm) -> Result<Vec<Vec<Constraint>>, String> {
    match form {
        ConstraintForm::Atomic { left_operand, operator, right_operand } => {
            let op = resolve_operator(operator)?;
            Ok(vec![vec![Constraint::new(left_operand.clone(), op, right_operand.clone())]])
        }
        ConstraintForm::And(children) => {
            let mut conjunctions: Vec<Vec<Constraint>> = vec![Vec::new()];
            for child in children {
                let child_dnf = to_dnf(child)?;
                let mut expanded = Vec::with_capacity(conjunctions.len() * child_dnf.len());
                for prefix in &conjunctions {
                    for disjunct in &child_dnf {
                        let mut combined = prefix.clone();
                        combined.extend(disjunct.iter().cloned());
                        expanded.push(combined);
                    }
                }
                conjunctions = expanded;
            }
            Ok(conjunctions)
        }
        ConstraintForm::Or(children) => {
            let mut disjuncts = Vec::new();
            for child in children {
                disjuncts.extend(to_dnf(child)?);
            }
            Ok(disjuncts)
        }
        ConstraintForm::Xone(children) => Err(xone_unsupported(children.len())),
    }
}

/// Does the SOTW graph assert `<member> odrl:partOf <collection>` (by
/// local name, matching the rest of this adapter's convention)?
fn is_member_of(sotw: &Graph, member_local: &str, collection_local: &str) -> bool {
    sotw.objects_by_subject_local_name(member_local, &odrl("partOf"))
        .iter()
        .any(|object| local_name(object) == collection_local)
}

/// `true` only if the SOTW graph carries a `report:DutyReport` for this
/// duty node whose `report:deonticState` is explicitly `report:Violated`.
/// No matching report, or any other state (`Fulfilled`, `NonSet`), is
/// "not violated" — the same not-proven-guilty default this corpus's own
/// fixtures expect (Section 7's per-permission-duty limitation is
/// otherwise still real; see this module's doc comment).
fn duty_is_violated(sotw: &Graph, duty_id: &str) -> bool {
    let duty_local = local_name(duty_id);
    sotw.subjects_by_object_local_name(&report_ns("rule"), duty_local).iter().any(|report_node| {
        sotw.type_of(report_node).as_deref() == Some(report_ns("DutyReport").as_str())
            && sotw.object_node(report_node, &report_ns("deonticState")).as_deref().map(local_name)
                == Some("Violated")
    })
}

fn current_time(sotw: &Graph) -> Option<String> {
    sotw.first_literal_for_predicate(&dct("issued"))
}

pub fn translate(policy: &PolicyInfo, req: &RequestInfo, sotw: &Graph, dataset_id: &str) -> Translation {
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

        let assignee_matches = match &rule.assignee {
            None => true,
            Some(PartyRef::Individual(name)) => name == &req.assignee,
            Some(PartyRef::Collection(collection)) => is_member_of(sotw, &req.assignee, collection),
        };
        let target_matches = match &rule.target {
            None => true,
            Some(TargetRef::Individual(name)) => Some(name) == req.target.as_ref(),
            Some(TargetRef::Collection(collection)) => {
                req.target.as_deref().is_some_and(|t| is_member_of(sotw, t, collection))
            }
        };
        if !(assignee_matches && target_matches) {
            continue;
        }

        if let Some(duty_id) = &rule.nested_duty {
            if duty_is_violated(sotw, duty_id) {
                continue;
            }
        }

        let mut base_constraints = Vec::new();
        match &rule.assignee {
            Some(PartyRef::Individual(name)) => {
                base_constraints.push(Constraint::new("sub", Operator::Eq, name.clone()));
            }
            Some(PartyRef::Collection(_)) => {
                base_constraints.push(Constraint::new("sub", Operator::Eq, req.assignee.clone()));
            }
            None => {}
        }

        let disjuncts = match &rule.constraint {
            None => vec![Vec::new()],
            Some(form) => match to_dnf(form) {
                Ok(d) => d,
                Err(reason) => return Translation::Skip(reason),
            },
        };

        for disjunct in disjuncts {
            let mut constraints = base_constraints.clone();
            constraints.extend(disjunct);
            let engine_rule = Rule::new(req.action.clone(), constraints);
            match rule.kind {
                RuleKind::Permission => permissions.push(engine_rule),
                RuleKind::Prohibition => prohibitions.push(engine_rule),
            }
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
    if let Some(now) = current_time(sotw) {
        claims.insert("dateTime".to_string(), ClaimValue::from(now));
    }

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
        RuleInfo { kind, assignee: None, action: None, target: None, constraint: None, nested_duty: None }
    }

    fn allow(policy: &PolicyInfo, req: &RequestInfo, sotw: &Graph) -> engine::WireDecision {
        match translate(policy, req, sotw, "ds1") {
            Translation::Ready(wire) => engine::evaluate_request(&wire).decision,
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn unconstrained_permission_allows_any_assignee_and_action() {
        let p = policy("p1", vec![unconstrained(RuleKind::Permission)]);
        let r = req("alice", "read", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Allow);
    }

    #[test]
    fn unconstrained_prohibition_denies_any_assignee_and_action() {
        let p = policy("p2", vec![unconstrained(RuleKind::Prohibition)]);
        let r = req("bob", "sell", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Deny);
    }

    #[test]
    fn assignee_scoped_permission_denies_a_non_matching_caller_by_omitting_the_policy() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p3", vec![rule]);
        let r = req("bob", "read", None);
        match translate(&p, &r, &Graph::empty(), "ds1") {
            Translation::Ready(wire) => {
                assert!(
                    wire.policies.is_empty(),
                    "a rule scoped to a different assignee must not survive translation as an \
                     empty-permissions shell, which would trigger decide()'s own open exception"
                );
                assert_eq!(engine::evaluate_request(&wire).decision, engine::WireDecision::Deny);
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
            nested_duty: None,
        };
        let p = policy("p4", vec![rule]);
        let r = req("alice", "read", Some("y"));
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Deny);
    }

    #[test]
    fn generic_use_permission_allows_a_more_specific_request_action() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p7", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Allow);
    }

    #[test]
    fn generic_use_prohibition_denies_a_more_specific_request_action() {
        let rule = RuleInfo {
            kind: RuleKind::Prohibition,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p8", vec![rule]);
        let r = req("alice", "write", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Deny);
    }

    #[test]
    fn generic_use_rule_still_respects_assignee_scoping() {
        let rule = RuleInfo {
            kind: RuleKind::Prohibition,
            assignee: Some(PartyRef::Individual("bob".to_string())),
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p9", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &Graph::empty()),
            engine::WireDecision::Deny,
            "assignee mismatch (bob vs alice) excludes the rule regardless of the generic 'use' action matching 'read'"
        );
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
            nested_duty: None,
        };
        let p = policy("p10", vec![rule]);
        let r = req("alice", "sell", None);
        match translate(&p, &r, &Graph::empty(), "ds1") {
            Translation::Ready(wire) => {
                assert!(wire.policies.is_empty(), "a generic 'use' permission must not be treated as covering 'sell'");
                assert_eq!(engine::evaluate_request(&wire).decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    fn sotw_with_current_time(iso: &str) -> Graph {
        Graph::parse_str(&format!(
            r#"@prefix dct: <http://purl.org/dc/terms/>.
@prefix temp: <http://example.com/request/>.
@prefix xsd: <http://www.w3.org/2001/XMLSchema#>.
temp:currentTime dct:issued "{iso}"^^xsd:dateTime."#
        ))
        .unwrap()
    }

    #[test]
    fn datetime_atomic_constraint_is_evaluated_not_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Atomic {
                left_operand: "dateTime".to_string(),
                operator: "gt".to_string(),
                right_operand: "2024-01-01T00:00:00Z".to_string(),
            }),
            nested_duty: None,
        };
        let p = policy("p11", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-06-01T00:00:00Z")),
            engine::WireDecision::Allow
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2017-01-01T00:00:00Z")),
            engine::WireDecision::Deny
        );
    }

    #[test]
    fn and_logical_constraint_becomes_multiple_constraints_on_one_rule() {
        // "between 2024-01-01 and 2024-12-31" — mirrors policy-15/-21.
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::And(vec![
                ConstraintForm::Atomic {
                    left_operand: "dateTime".to_string(),
                    operator: "gt".to_string(),
                    right_operand: "2024-01-01T00:00:00Z".to_string(),
                },
                ConstraintForm::Atomic {
                    left_operand: "dateTime".to_string(),
                    operator: "lt".to_string(),
                    right_operand: "2024-12-31T23:59:59Z".to_string(),
                },
            ])),
            nested_duty: None,
        };
        let p = policy("p12", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(allow(&p, &r, &sotw_with_current_time("2024-06-01T00:00:00Z")), engine::WireDecision::Allow);
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2025-06-01T00:00:00Z")),
            engine::WireDecision::Deny,
            "outside the AND'd window on the far side"
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2017-06-01T00:00:00Z")),
            engine::WireDecision::Deny,
            "outside the AND'd window on the near side"
        );
    }

    #[test]
    fn or_logical_constraint_becomes_sibling_permission_rules() {
        // "9-17 on day 1, OR 9-17 on day 2" — the same shape as policy-20's
        // 262-branch "business hours in 2024", shrunk to two branches.
        let branch = |start: &str, end: &str| ConstraintForm::And(vec![
            ConstraintForm::Atomic { left_operand: "dateTime".into(), operator: "gt".into(), right_operand: start.into() },
            ConstraintForm::Atomic { left_operand: "dateTime".into(), operator: "lt".into(), right_operand: end.into() },
        ]);
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Or(vec![
                branch("2024-01-01T09:00:00Z", "2024-01-01T17:00:00Z"),
                branch("2024-01-02T09:00:00Z", "2024-01-02T17:00:00Z"),
            ])),
            nested_duty: None,
        };
        let p = policy("p13", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-01-01T11:00:00Z")),
            engine::WireDecision::Allow,
            "inside the first disjunct"
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-01-02T11:00:00Z")),
            engine::WireDecision::Allow,
            "inside the second disjunct"
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-01-01T20:00:00Z")),
            engine::WireDecision::Deny,
            "inside neither disjunct"
        );
    }

    #[test]
    fn xone_logical_constraint_is_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Xone(vec![
                ConstraintForm::Atomic { left_operand: "dateTime".into(), operator: "gt".into(), right_operand: "2024-01-01T00:00:00Z".into() },
            ])),
            nested_duty: None,
        };
        let p = policy("p14", vec![rule]);
        let r = req("alice", "read", None);
        assert!(matches!(translate(&p, &r, &Graph::empty(), "ds1"), Translation::Skip(_)));
    }

    #[test]
    fn unsupported_operator_is_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Atomic {
                left_operand: "count".into(),
                operator: "isPartOf".into(),
                right_operand: "5".into(),
            }),
            nested_duty: None,
        };
        let p = policy("p15", vec![rule]);
        let r = req("alice", "read", None);
        assert!(matches!(translate(&p, &r, &Graph::empty(), "ds1"), Translation::Skip(_)));
    }

    fn sotw_with_membership(member: &str, collection: &str) -> Graph {
        Graph::parse_str(&format!(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:{member} odrl:partOf ex:{collection}."#
        ))
        .unwrap()
    }

    #[test]
    fn party_collection_assignee_resolves_via_sotw_membership() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Collection("partyCollection".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p16", vec![rule]);
        let member = req("alice", "read", None);
        assert_eq!(
            allow(&p, &member, &sotw_with_membership("alice", "partyCollection")),
            engine::WireDecision::Allow
        );
        let non_member = req("mallory", "read", None);
        assert_eq!(
            allow(&p, &non_member, &sotw_with_membership("alice", "partyCollection")),
            engine::WireDecision::Deny,
            "the SOTW graph asserts alice's membership, not mallory's"
        );
    }

    #[test]
    fn asset_collection_target_resolves_via_sotw_membership() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: Some(TargetRef::Collection("assetCollection".to_string())),
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p17", vec![rule]);
        let in_collection = req("alice", "read", Some("x"));
        assert_eq!(
            allow(&p, &in_collection, &sotw_with_membership("x", "assetCollection")),
            engine::WireDecision::Allow
        );
        let outside_collection = req("alice", "read", Some("y"));
        assert_eq!(
            allow(&p, &outside_collection, &sotw_with_membership("x", "assetCollection")),
            engine::WireDecision::Deny
        );
        let no_target_named = req("alice", "read", None);
        assert_eq!(
            allow(&p, &no_target_named, &sotw_with_membership("x", "assetCollection")),
            engine::WireDecision::Deny,
            "membership cannot be checked without a target to check it for"
        );
    }

    fn sotw_with_duty_state(duty_id: &str, state: &str) -> Graph {
        Graph::parse_str(&format!(
            r#"@prefix report: <https://w3id.org/force/compliance-report#>.
@prefix ex: <http://example.org/>.
ex:report1 a report:DutyReport;
    report:rule <{duty_id}>;
    report:deonticState report:{state}."#
        ))
        .unwrap()
    }

    #[test]
    fn permission_with_a_violated_nested_duty_is_excluded() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: Some("urn:uuid:duty-1".to_string()),
        };
        let p = policy("p18", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_duty_state("urn:uuid:duty-1", "Violated")),
            engine::WireDecision::Deny
        );
    }

    #[test]
    fn permission_with_a_fulfilled_or_nonset_nested_duty_still_grants() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: Some("urn:uuid:duty-1".to_string()),
        };
        let p = policy("p19", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_duty_state("urn:uuid:duty-1", "Fulfilled")),
            engine::WireDecision::Allow
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_duty_state("urn:uuid:duty-1", "NonSet")),
            engine::WireDecision::Allow
        );
        assert_eq!(
            allow(&p, &r, &Graph::empty()),
            engine::WireDecision::Allow,
            "no DutyReport at all is not evidence of violation"
        );
    }
}
