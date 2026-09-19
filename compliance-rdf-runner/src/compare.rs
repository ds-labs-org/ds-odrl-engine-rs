//! Compares one case's real `engine::DetailedEvaluation` (and coarse
//! `engine::Response`) against the `report:`-tree ground truth
//! `expected.rs` parsed out of the same case file's `dsc:expectedOutcome`.
//!
//! Three independent checks, per `docs/vocabulary-spec.md`'s own two-layer
//! API surface plus the optional `dsc:expectedDecision` term:
//!
//! 1. **The detailed, per-rule/per-duty `report:` tree** -- every
//!    activation/attempt/performance/deontic state the expected tree
//!    states an opinion on must match the engine's own
//!    `DetailedRuleReport` for that same RDF rule node exactly, and no
//!    actual rule report may go unaccounted for by the expected tree
//!    either (an un-asserted actual rule report is exactly as much a
//!    finding as a wrong state on an asserted one -- either means this
//!    fixture's own expectations are stale against the real engine).
//! 2. **The coarse `Response.duties` cross-check** (new; this is what
//!    proves ds-odrl-engine-rs's Bug A is real): every expected
//!    `report:DutyReport` that is `Active`+`Unperformed` must appear in
//!    the coarse `Response.duties` list, and every one that is not (an
//!    `Inactive` duty, or one that is `Active`+`Performed`) must not --
//!    except a plain policy-level `odrl:obligation` (not a permission's
//!    `odrl:duty` or a prohibition's `odrl:remedy`) under `dutyMode=deny`,
//!    which `wire::response_from_scaffolding` deliberately omits from
//!    `Response.duties` even while outstanding (the `Decision::Deny` it
//!    already forced says the same thing, so listing it again would be
//!    noise -- see that function's own doc comment). This one exception
//!    is resolved by walking the very same `PolicyIds` shadow
//!    `translate.rs` built, not guessed from the expected tree alone.
//! 3. **The explicit `dsc:expectedDecision` cross-check** -- a direct
//!    comparison against what the fixture actually STATES about the
//!    coarse `Response.decision`, when it states anything
//!    (`expected_decision` is `None` for the 10+ fixtures that predate
//!    this term and state no opinion). Independent of checks 1 and 2: a
//!    fixture may satisfy some, all, or none.

use std::collections::HashMap;

use engine::{
    ActivationState, AttemptState, DeonticState, DetailedEvaluation, DetailedRuleReport,
    DutyAttachment, DutyMode, PerformanceState, Response, WireDecision,
};

use crate::expected::{ExpectedPolicyReport, ExpectedRuleKind, ExpectedRuleReport};
use crate::translate::{merged_policy_ids, PolicyIds, RuleIds};

fn activation_name(s: ActivationState) -> &'static str {
    match s {
        ActivationState::Active => "Active",
        ActivationState::Inactive => "Inactive",
    }
}
fn attempt_name(s: AttemptState) -> &'static str {
    match s {
        AttemptState::Attempted => "Attempted",
        AttemptState::NotAttempted => "NotAttempted",
    }
}
fn performance_name(s: PerformanceState) -> &'static str {
    match s {
        PerformanceState::Performed => "Performed",
        PerformanceState::Unperformed => "Unperformed",
        PerformanceState::Unknown => "Unknown",
    }
}
fn deontic_name(s: DeonticState) -> &'static str {
    match s {
        DeonticState::NonSet => "NonSet",
        DeonticState::Violated => "Violated",
        DeonticState::Fulfilled => "Fulfilled",
    }
}

/// One human-readable mismatch, accumulated rather than returned on the
/// first failure, so one failing case reports everything wrong with it in
/// a single run rather than needing the fixture fixed and rerun once per
/// mismatch.
pub type Mismatches = Vec<String>;

fn rule_id_of(
    merged: &PolicyIds,
    report: &DetailedRuleReport,
) -> Option<(ExpectedRuleKind, String)> {
    match report {
        DetailedRuleReport::Permission(p) => merged
            .permissions
            .get(p.rule_index)
            .map(|r| (ExpectedRuleKind::Permission, r.rule_id.clone())),
        DetailedRuleReport::Prohibition(p) => merged
            .prohibitions
            .get(p.rule_index)
            .map(|r| (ExpectedRuleKind::Prohibition, r.rule_id.clone())),
        DetailedRuleReport::Duty(d) => duty_root(merged, d.attachment, d.duty_index)
            .and_then(|root| walk_consequence(root, d.consequence_depth))
            .map(|r| (ExpectedRuleKind::Duty, r.rule_id.clone())),
    }
}

fn duty_root(
    merged: &PolicyIds,
    attachment: DutyAttachment,
    duty_index: usize,
) -> Option<&RuleIds> {
    match attachment {
        DutyAttachment::Obligation => merged.obligations.get(duty_index),
        DutyAttachment::PermissionDuty { rule_index } => {
            merged.permissions.get(rule_index)?.duty.get(duty_index)
        }
        DutyAttachment::ProhibitionRemedy { rule_index } => {
            merged.prohibitions.get(rule_index)?.remedy.get(duty_index)
        }
    }
}

fn walk_consequence(root: &RuleIds, depth: usize) -> Option<&RuleIds> {
    let mut current = root;
    for _ in 0..depth {
        current = current.consequence.as_deref()?;
    }
    Some(current)
}

/// `true` iff `rule_id` names exactly a policy-level `odrl:obligation`
/// itself (not one of its `odrl:consequence` hops) -- the one shape
/// `wire::response_from_scaffolding` omits from `Response.duties` under
/// `dutyMode=deny` regardless of outstanding status. Walks
/// `merged.obligations` at depth 0 only, matching
/// `UnresolvedDuty::is_plain_policy_obligation`'s own `consequence_depth
/// == 0` condition exactly.
fn is_plain_policy_obligation(merged: &PolicyIds, rule_id: &str) -> bool {
    merged.obligations.iter().any(|r| r.rule_id == rule_id)
}

/// Finds `rule_id` anywhere in one duty/remedy/obligation chain
/// (`root` itself, or one of its `odrl:consequence` hops), by reference.
fn find_in_chain<'a>(root: &'a RuleIds, rule_id: &str) -> Option<&'a RuleIds> {
    if root.rule_id == rule_id {
        return Some(root);
    }
    root.consequence
        .as_deref()
        .and_then(|c| find_in_chain(c, rule_id))
}

/// Finds `rule_id` anywhere in `merged`'s three duty attachment points
/// (policy obligations, every permission's own duties, every
/// prohibition's own remedies), each searched through its own
/// `odrl:consequence` chain.
fn find_rule_ids<'a>(merged: &'a PolicyIds, rule_id: &str) -> Option<&'a RuleIds> {
    merged
        .obligations
        .iter()
        .find_map(|r| find_in_chain(r, rule_id))
        .or_else(|| {
            merged
                .permissions
                .iter()
                .flat_map(|p| &p.duty)
                .find_map(|r| find_in_chain(r, rule_id))
        })
        .or_else(|| {
            merged
                .prohibitions
                .iter()
                .flat_map(|p| &p.remedy)
                .find_map(|r| find_in_chain(r, rule_id))
        })
}

/// `true` iff `rule_id` is the *terminal* node of its own duty chain --
/// carries no `odrl:consequence` of its own. Only a terminal node is ever
/// what `decision::outstanding_duty` actually reports as outstanding
/// (`decision::unresolved_permission_duties`/`unresolved_obligations`/
/// `unresolved_remedies` all resolve through it): an earlier hop with a
/// consequence is superseded by that consequence regardless of its own
/// individual Active/Unperformed state, even though `derive_detailed_
/// rule_reports`'s own `duty_chain_reports` still reports every hop
/// individually as a sibling `DetailedRuleReport::Duty`. Unknown ids
/// (a translator bug, or an expected `report:DutyReport` naming a rule
/// this crate's own `PolicyIds` shadow never saw) are treated as
/// terminal, the fail-loud direction: `compare_case`'s own "engine
/// produced a rule report the expected outcome never mentions" check
/// already flags a genuinely unknown id from the opposite direction, so
/// this default never silently hides one.
fn is_terminal(merged: &PolicyIds, rule_id: &str) -> bool {
    find_rule_ids(merged, rule_id)
        .map(|r| r.consequence.is_none())
        .unwrap_or(true)
}

/// Compares one case's actual `DetailedEvaluation` and `Response` against
/// its parsed expected outcome, returning every mismatch found (empty
/// means the case passed both checks).
pub fn compare_case(
    policy_ids: &[PolicyIds],
    expected: &[ExpectedPolicyReport],
    duty_mode: DutyMode,
    expected_decision: Option<WireDecision>,
    detailed: &DetailedEvaluation,
    response: &Response,
) -> Result<Mismatches, String> {
    let mut mismatches = Mismatches::new();
    let mut expected_outstanding: Vec<(String, String)> = Vec::new(); // (policy_id, action)

    for exp_policy in expected {
        let merged = merged_policy_ids(policy_ids, &exp_policy.policy_id)
            .map_err(|e| format!("{}: {e}", exp_policy.policy_id))?;

        let actual_policy = detailed
            .policy_reports
            .iter()
            .find(|p| p.policy_id == exp_policy.policy_id);
        let Some(actual_policy) = actual_policy else {
            mismatches.push(format!(
                "expected report:PolicyReport for policy '{}' but the engine produced no \
                 DetailedPolicyReport for it (skipped by party-role scoping, or a request-level error?)",
                exp_policy.policy_id
            ));
            continue;
        };

        let mut actual_index: HashMap<(ExpectedRuleKind, String), &DetailedRuleReport> =
            HashMap::new();
        for report in &actual_policy.rule_reports {
            if let Some(key) = rule_id_of(&merged, report) {
                actual_index.insert(key, report);
            } else {
                mismatches.push(format!(
                    "policy '{}': engine produced a rule report this translator's own PolicyIds \
                     shadow could not resolve back to an RDF rule id -- translator bug: {report:?}",
                    exp_policy.policy_id
                ));
            }
        }

        let mut matched_keys = std::collections::HashSet::new();
        for exp_rule in &exp_policy.rule_reports {
            let key = (exp_rule.kind, exp_rule.rule.clone());
            let actual = actual_index.get(&key).copied();
            match actual {
                None => mismatches.push(format!(
                    "policy '{}': expected a {:?} report for rule '{}' but the engine produced none",
                    exp_policy.policy_id, exp_rule.kind, exp_rule.rule
                )),
                Some(actual) => {
                    matched_keys.insert(key.clone());
                    compare_rule_report(&exp_policy.policy_id, exp_rule, actual, &mut mismatches);
                }
            }

            if exp_rule.kind == ExpectedRuleKind::Duty
                && exp_rule.activation_state.as_deref() == Some("Active")
                && exp_rule.performance_state.as_deref() == Some("Unperformed")
                && is_terminal(&merged, &exp_rule.rule)
                && !(duty_mode == DutyMode::Deny
                    && is_plain_policy_obligation(&merged, &exp_rule.rule))
            {
                if let Some(DetailedRuleReport::Duty(d)) = actual {
                    expected_outstanding
                        .push((exp_policy.policy_id.clone(), duty_action(&merged, d)));
                }
            }
        }

        for (key, report) in &actual_index {
            if !matched_keys.contains(key) {
                mismatches.push(format!(
                    "policy '{}': engine produced a {:?} report for rule '{}' that the expected outcome \
                     never mentions: {report:?}",
                    exp_policy.policy_id, key.0, key.1
                ));
            }
        }
    }

    // -- Part 2: the coarse Response.duties cross-check --------------------
    let actual_outstanding: Vec<(String, String)> = response
        .duties
        .iter()
        .map(|d| (d.policy_id.clone(), d.action.clone()))
        .collect();

    let mut expected_sorted = expected_outstanding.clone();
    let mut actual_sorted = actual_outstanding.clone();
    expected_sorted.sort();
    actual_sorted.sort();
    if expected_sorted != actual_sorted {
        mismatches.push(format!(
            "Response.duties cross-check: expected outstanding (policy_id, action) pairs {expected_sorted:?} \
             but Response.duties actually contains {actual_sorted:?} -- every report:DutyReport that is \
             Active+Unperformed in the expected tree must appear in the coarse Response.duties list, and \
             nothing else should"
        ));
    }

    // -- Part 3: the explicit dsc:expectedDecision cross-check -------------
    if let Some(expected_decision) = expected_decision {
        if response.decision != expected_decision {
            mismatches.push(format!(
                "dsc:expectedDecision cross-check: this TestCase declares {expected_decision:?} \
                 but the engine's actual Response.decision is {:?}",
                response.decision
            ));
        }
    }

    Ok(mismatches)
}

/// The `action` string `wire::DutyEntry` will carry for this outstanding
/// duty -- per `decision::outstanding_duty`, that is the duty actually
/// found outstanding at `consequence_depth` hops from the attachment
/// point, not the root duty's own action when a consequence chain was
/// walked. `rule_id_of`'s own `walk_consequence` already knows how to
/// find that exact node; reused here so the two can never disagree about
/// which node a given `(attachment, duty_index, depth)` names.
fn duty_action(merged: &PolicyIds, d: &engine::DetailedDutyReport) -> String {
    duty_root(merged, d.attachment, d.duty_index)
        .and_then(|root| walk_consequence(root, d.consequence_depth))
        .map(|r| r.action.clone())
        .unwrap_or_default()
}

fn compare_rule_report(
    policy_id: &str,
    expected: &ExpectedRuleReport,
    actual: &DetailedRuleReport,
    mismatches: &mut Mismatches,
) {
    let (activation, attempt, performance, deontic) = match actual {
        DetailedRuleReport::Permission(p) => (
            Some(p.activation_state),
            Some(p.attempt_state),
            Some(p.performance_state),
            Some(p.deontic_state),
        ),
        DetailedRuleReport::Prohibition(p) => (
            Some(p.activation_state),
            Some(p.attempt_state),
            Some(p.performance_state),
            Some(p.deontic_state),
        ),
        DetailedRuleReport::Duty(d) => (
            Some(d.activation_state),
            None,
            Some(d.performance_state),
            Some(d.deontic_state),
        ),
    };

    let mut check = |dimension: &str,
                     expected_value: &Option<String>,
                     actual_value: Option<&'static str>| {
        let Some(expected_value) = expected_value else {
            return;
        };
        match actual_value {
            None => mismatches.push(format!(
                "policy '{policy_id}' rule '{}': expected report:{dimension} '{expected_value}' but the \
                 engine's report shape carries no {dimension} for this rule kind",
                expected.rule
            )),
            Some(actual_value) if actual_value != expected_value => mismatches.push(format!(
                "policy '{policy_id}' rule '{}': report:{dimension} expected '{expected_value}' but engine \
                 produced '{actual_value}'",
                expected.rule
            )),
            _ => {}
        }
    };

    check(
        "activationState",
        &expected.activation_state,
        activation.map(activation_name),
    );
    check(
        "attemptState",
        &expected.attempt_state,
        attempt.map(attempt_name),
    );
    check(
        "performanceState",
        &expected.performance_state,
        performance.map(performance_name),
    );
    check(
        "deonticState",
        &expected.deontic_state,
        deontic.map(deontic_name),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_detailed() -> DetailedEvaluation {
        DetailedEvaluation {
            dataset_id: String::new(),
            requested_action: String::new(),
            policy_reports: vec![],
            skipped_policies: vec![],
        }
    }

    fn response_with(decision: WireDecision) -> Response {
        Response {
            dataset_id: String::new(),
            decision,
            reason: String::new(),
            duties: vec![],
        }
    }

    #[test]
    fn expected_decision_mismatch_is_reported() {
        let detailed = empty_detailed();
        let response = response_with(WireDecision::Deny);
        let mismatches = compare_case(
            &[],
            &[],
            DutyMode::Advise,
            Some(WireDecision::Allow),
            &detailed,
            &response,
        )
        .unwrap();
        assert_eq!(mismatches.len(), 1, "{mismatches:?}");
        assert!(
            mismatches[0].contains("dsc:expectedDecision"),
            "{mismatches:?}"
        );
    }

    #[test]
    fn expected_decision_match_produces_no_mismatch() {
        let detailed = empty_detailed();
        let response = response_with(WireDecision::Allow);
        let mismatches = compare_case(
            &[],
            &[],
            DutyMode::Advise,
            Some(WireDecision::Allow),
            &detailed,
            &response,
        )
        .unwrap();
        assert!(mismatches.is_empty(), "{mismatches:?}");
    }

    #[test]
    fn expected_decision_absent_is_a_no_op() {
        let detailed = empty_detailed();
        let response = response_with(WireDecision::Deny);
        let mismatches =
            compare_case(&[], &[], DutyMode::Advise, None, &detailed, &response).unwrap();
        assert!(mismatches.is_empty(), "{mismatches:?}");
    }
}
