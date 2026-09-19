//! Compares one case's real `engine::DetailedEvaluation` (and coarse
//! `engine::Response`) against the `report:`-tree ground truth
//! `expected.rs` parsed out of the same case file's `dsc:expectedOutcome`.
//!
//! Four independent checks, per `docs/vocabulary-spec.md`'s own two-layer
//! API surface -- the first over the detailed tree, the other three over
//! what the coarse `Response` and the request's policy set must say if
//! that tree is right (added after an independent audit of v0.23.2 showed
//! that with only check 1 and the duties half of check 4, a fixture could
//! pass while `Response.decision` said the opposite of its own tree, and
//! while a second, unmentioned policy denied the whole request):
//!
//! 1. **The detailed, per-rule/per-duty `report:` tree** -- every
//!    activation/attempt/performance/deontic state the expected tree
//!    states an opinion on must match the engine's own
//!    `DetailedRuleReport` for that same RDF rule node exactly, and no
//!    actual rule report may go unaccounted for by the expected tree
//!    either (an un-asserted actual rule report is exactly as much a
//!    finding as a wrong state on an asserted one -- either means this
//!    fixture's own expectations are stale against the real engine).
//! 2. **Policy-level coverage**: every policy the engine evaluated (or
//!    skipped by party-role scoping, or refused with `Decision::Error`)
//!    must be one the expected tree carries a `report:PolicyReport` for.
//! 3. **The coarse `Response.decision` cross-check**: the per-policy
//!    verdict each expected tree implies (`policy_verdict`), combined
//!    under the engine's own deny-override-across-the-set rule, must equal
//!    `Response.decision` -- asserted only when the trees pin it.
//! 4. **The coarse `Response.duties` cross-check** (this is what
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

use std::collections::HashMap;

use engine::{
    ActivationState, AttemptState, Behaviour, DeonticState, DetailedEvaluation, DetailedRuleReport,
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
    behaviour: Behaviour,
    detailed: &DetailedEvaluation,
    response: &Response,
) -> Result<Mismatches, String> {
    let mut mismatches = Mismatches::new();
    let mut expected_outstanding: Vec<(String, String)> = Vec::new(); // (policy_id, action)
    let mut verdicts: Vec<PolicyVerdict> = Vec::new();

    for exp_policy in expected {
        let merged = merged_policy_ids(policy_ids, &exp_policy.policy_id)
            .map_err(|e| format!("{}: {e}", exp_policy.policy_id))?;
        verdicts.push(policy_verdict(&merged, exp_policy, duty_mode, behaviour));

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
        if let Some(error) = &actual_policy.evaluation_error {
            mismatches.push(format!(
                "policy '{}': the engine refused to evaluate this policy at all \
                 (Decision::Error: {error}) -- no report: tree can describe that, so the \
                 fixture's expectations for it cannot hold",
                exp_policy.policy_id
            ));
        }

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

    // -- Part 2: policy-level coverage --------------------------------------
    // The rule-level "engine produced a report the expected outcome never
    // mentions" check above only ever runs *inside* a policy the expected
    // tree names. A whole policy the tree omits -- one that may deny the
    // entire request on its own under the engine's deny-override-across-
    // the-set combining rule -- used to go unexamined, so a fixture could
    // pass by simply not mentioning the policy that decided it.
    let mut every_policy_accounted_for = true;
    for actual_policy in &detailed.policy_reports {
        if !expected
            .iter()
            .any(|e| e.policy_id == actual_policy.policy_id)
        {
            every_policy_accounted_for = false;
            mismatches.push(format!(
                "the engine evaluated policy '{}' but the expected outcome carries no \
                 report:PolicyReport for it -- every policy in dsc:policy must be accounted for, \
                 since any one of them can decide the whole request",
                actual_policy.policy_id
            ));
        }
    }
    for skipped in &detailed.skipped_policies {
        every_policy_accounted_for = false;
        mismatches.push(format!(
            "the engine skipped policy '{}' before evaluating it ({}) -- the report: vocabulary \
             has no term for a policy addressed to somebody else, so this fixture cannot be \
             stating what it means to state",
            skipped.policy_id, skipped.reason
        ));
    }

    // -- Part 3: the coarse Response.decision cross-check ------------------
    // The `report:` vocabulary has no "decision" term, so the one thing a
    // host actually acts on was, until this check existed, never compared
    // against anything: the whole tree could match while `Response.decision`
    // said the opposite. What the tree *does* state per policy is enough to
    // pin the decision in most shapes (`policy_verdict`); where it is not,
    // this deliberately asserts nothing rather than guessing.
    if every_policy_accounted_for {
        let expected_decision = if verdicts.contains(&PolicyVerdict::Deny) {
            Some(WireDecision::Deny)
        } else if verdicts.iter().all(|v| *v == PolicyVerdict::Allow) {
            Some(WireDecision::Allow)
        } else {
            None
        };
        if let Some(expected_decision) = expected_decision {
            if response.decision != expected_decision {
                mismatches.push(format!(
                    "Response.decision cross-check: the expected report: tree implies \
                     {expected_decision:?} (per-policy verdicts {verdicts:?}) but the engine's \
                     coarse Response.decision is {:?} -- the detailed and coarse paths disagree, \
                     or the fixture's tree is wrong about which rules are Active",
                    response.decision
                ));
            }
        }
    }

    // -- Part 4: the coarse Response.duties cross-check --------------------
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

    Ok(mismatches)
}

/// What one policy's expected `report:` tree says about that policy's own
/// share of the request's decision -- the per-policy input to the
/// deny-override-across-the-set rule `wire::response_from_scaffolding`
/// applies (`Deny` from any policy denies the request; `Allow` needs every
/// policy to allow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyVerdict {
    Allow,
    Deny,
    /// The tree does not pin this policy's verdict: no permission is
    /// `Active`, but under `dutyMode=advise` a permission can be reported
    /// `Inactive` purely because an advisory duty chain is outstanding
    /// while the coarse path still grants (the one disclosed place the two
    /// paths read the same chain differently). Nothing is asserted about
    /// `Response.decision` when any policy lands here.
    Indeterminate,
}

/// Derives [`PolicyVerdict`] from what the expected tree states, in the
/// same precedence `decision::decide` applies:
///
/// 1. An `Active` `ProhibitionReport` denies -- a prohibition is only
///    `Active` when it fired and was not superseded by `odrl:conflict`.
/// 2. Under `dutyMode=deny`, an outstanding policy-level obligation chain
///    (an `Active`+`Unperformed` terminal `DutyReport` rooted in
///    `odrl:obligation`) forces `Deny` regardless of any permission.
/// 3. An `Active` `PermissionReport` grants.
/// 4. No permission at all: `behaviour` decides (`Open` allows vacuously,
///    `Closed` denies).
/// 5. Permissions exist and none is `Active`: under `dutyMode=deny` the
///    report's activation gate is the identical predicate `Rule::grants`
///    uses, so nothing granted -- `Deny`; under `advise` see
///    [`PolicyVerdict::Indeterminate`].
fn policy_verdict(
    merged: &PolicyIds,
    expected: &ExpectedPolicyReport,
    duty_mode: DutyMode,
    behaviour: Behaviour,
) -> PolicyVerdict {
    let active = |kind: ExpectedRuleKind| {
        expected
            .rule_reports
            .iter()
            .any(|r| r.kind == kind && r.activation_state.as_deref() == Some("Active"))
    };
    let outstanding_obligation = duty_mode == DutyMode::Deny
        && expected.rule_reports.iter().any(|r| {
            r.kind == ExpectedRuleKind::Duty
                && r.activation_state.as_deref() == Some("Active")
                && r.performance_state.as_deref() == Some("Unperformed")
                && is_terminal(merged, &r.rule)
                && merged
                    .obligations
                    .iter()
                    .any(|o| find_in_chain(o, &r.rule).is_some())
        });

    if active(ExpectedRuleKind::Prohibition) || outstanding_obligation {
        PolicyVerdict::Deny
    } else if active(ExpectedRuleKind::Permission) {
        PolicyVerdict::Allow
    } else if merged.permissions.is_empty() {
        match behaviour {
            Behaviour::Open => PolicyVerdict::Allow,
            Behaviour::Closed => PolicyVerdict::Deny,
        }
    } else if duty_mode == DutyMode::Deny {
        PolicyVerdict::Deny
    } else {
        PolicyVerdict::Indeterminate
    }
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
    use engine::{
        DetailedPermissionReport, DetailedPolicyReport, DetailedPolicyRequest,
        DetailedProhibitionReport, WireDecision,
    };

    fn ids(policy_id: &str, perms: &[&str], prohs: &[&str]) -> PolicyIds {
        let rule = |id: &str| RuleIds {
            rule_id: id.to_string(),
            action: "read".to_string(),
            duty: vec![],
            remedy: vec![],
            consequence: None,
        };
        PolicyIds {
            id: policy_id.to_string(),
            permissions: perms.iter().map(|p| rule(p)).collect(),
            prohibitions: prohs.iter().map(|p| rule(p)).collect(),
            obligations: vec![],
            inherit_from: None,
        }
    }

    fn permission(rule_index: usize, active: bool) -> DetailedRuleReport {
        DetailedRuleReport::Permission(DetailedPermissionReport {
            rule_index,
            activation_state: if active {
                ActivationState::Active
            } else {
                ActivationState::Inactive
            },
            attempt_state: AttemptState::Attempted,
            performance_state: if active {
                PerformanceState::Performed
            } else {
                PerformanceState::Unperformed
            },
            deontic_state: if active {
                DeonticState::Fulfilled
            } else {
                DeonticState::NonSet
            },
            premise_reports: vec![],
            condition_report: None,
        })
    }

    fn prohibition(rule_index: usize, active: bool) -> DetailedRuleReport {
        DetailedRuleReport::Prohibition(DetailedProhibitionReport {
            rule_index,
            activation_state: if active {
                ActivationState::Active
            } else {
                ActivationState::Inactive
            },
            attempt_state: AttemptState::Attempted,
            performance_state: if active {
                PerformanceState::Performed
            } else {
                PerformanceState::Unknown
            },
            deontic_state: if active {
                DeonticState::Violated
            } else {
                DeonticState::NonSet
            },
            premise_reports: vec![],
        })
    }

    fn policy_report(
        policy_id: &str,
        rule_reports: Vec<DetailedRuleReport>,
    ) -> DetailedPolicyReport {
        DetailedPolicyReport {
            policy_id: policy_id.to_string(),
            policy_request: DetailedPolicyRequest {
                requested_action: "read".to_string(),
                requested_target: "asset".to_string(),
            },
            rule_reports,
            evaluation_error: None,
            declared_conflict: engine::ConflictStrategy::Invalid,
            conflict_forced_invalid: false,
        }
    }

    fn evaluation(policy_reports: Vec<DetailedPolicyReport>) -> DetailedEvaluation {
        DetailedEvaluation {
            dataset_id: "asset".to_string(),
            requested_action: "read".to_string(),
            policy_reports,
            skipped_policies: vec![],
        }
    }

    fn response(decision: WireDecision) -> Response {
        Response {
            dataset_id: "asset".to_string(),
            decision,
            reason: String::new(),
            duties: vec![],
        }
    }

    fn expected(
        policy_id: &str,
        kind: ExpectedRuleKind,
        rule: &str,
        activation: &str,
    ) -> ExpectedPolicyReport {
        ExpectedPolicyReport {
            policy_id: policy_id.to_string(),
            rule_reports: vec![ExpectedRuleReport {
                kind,
                rule: rule.to_string(),
                activation_state: Some(activation.to_string()),
                attempt_state: None,
                performance_state: None,
                deontic_state: None,
            }],
        }
    }

    #[test]
    fn an_expected_active_permission_beside_a_coarse_deny_is_a_mismatch() {
        // The `report:` vocabulary has no "decision" term, so the coarse
        // `Response.decision` used to go entirely unchecked: a fixture
        // whose whole point was "this permission grants" passed even when
        // the engine answered Deny. An expected Active PermissionReport
        // is a statement that the permission granted -- the request must
        // then be Allow, unless something the same tree states (an Active
        // prohibition, or a deny-mode obligation) overrides it.
        let shadow = [ids("policy-a", &["perm"], &[])];
        let exp = [expected(
            "policy-a",
            ExpectedRuleKind::Permission,
            "perm",
            "Active",
        )];
        let detailed = evaluation(vec![policy_report("policy-a", vec![permission(0, true)])]);

        let ok = compare_case(
            &shadow,
            &exp,
            DutyMode::Advise,
            Behaviour::Closed,
            &detailed,
            &response(WireDecision::Allow),
        )
        .unwrap();
        assert!(ok.is_empty(), "{ok:?}");

        let bad = compare_case(
            &shadow,
            &exp,
            DutyMode::Advise,
            Behaviour::Closed,
            &detailed,
            &response(WireDecision::Deny),
        )
        .unwrap();
        assert!(
            bad.iter().any(|m| m.contains("Response.decision")),
            "{bad:?}"
        );
    }

    #[test]
    fn an_expected_active_prohibition_beside_a_coarse_allow_is_a_mismatch() {
        let shadow = [ids("policy-a", &[], &["proh"])];
        let exp = [expected(
            "policy-a",
            ExpectedRuleKind::Prohibition,
            "proh",
            "Active",
        )];
        let detailed = evaluation(vec![policy_report("policy-a", vec![prohibition(0, true)])]);

        let bad = compare_case(
            &shadow,
            &exp,
            DutyMode::Advise,
            Behaviour::Open,
            &detailed,
            &response(WireDecision::Allow),
        )
        .unwrap();
        assert!(
            bad.iter().any(|m| m.contains("Response.decision")),
            "{bad:?}"
        );
    }

    #[test]
    fn under_duty_mode_deny_no_active_permission_means_the_request_must_be_denied() {
        // Under `dutyMode=deny`, `grants()` and the report's activation
        // gate are the same predicate, so every permission Inactive means
        // no permission granted -- the request cannot be Allow. (Under
        // `advise` this is indeterminate from the tree alone: a permission
        // may be Inactive purely because an advisory duty chain is
        // outstanding while the coarse path still grants.) This is the
        // shape of the audit's finding #1: a consequence-satisfied duty
        // chain reported the permission Inactive under deny mode while
        // `Response.decision` said Allow.
        let shadow = [ids("policy-a", &["perm"], &[])];
        let exp = [expected(
            "policy-a",
            ExpectedRuleKind::Permission,
            "perm",
            "Inactive",
        )];
        let detailed = evaluation(vec![policy_report("policy-a", vec![permission(0, false)])]);

        let bad = compare_case(
            &shadow,
            &exp,
            DutyMode::Deny,
            Behaviour::Closed,
            &detailed,
            &response(WireDecision::Allow),
        )
        .unwrap();
        assert!(
            bad.iter().any(|m| m.contains("Response.decision")),
            "{bad:?}"
        );

        let indeterminate = compare_case(
            &shadow,
            &exp,
            DutyMode::Advise,
            Behaviour::Closed,
            &detailed,
            &response(WireDecision::Allow),
        )
        .unwrap();
        assert!(indeterminate.is_empty(), "{indeterminate:?}");
    }

    #[test]
    fn an_evaluated_policy_the_expected_tree_never_mentions_is_a_mismatch() {
        // A second policy that denies the whole request, left out of the
        // expected tree, used to pass silently: the rule-level "engine
        // produced a report the expected outcome never mentions" check
        // only ran *inside* a mentioned policy.
        let shadow = [
            ids("policy-a", &["perm"], &[]),
            ids("policy-b", &[], &["proh"]),
        ];
        let exp = [expected(
            "policy-a",
            ExpectedRuleKind::Permission,
            "perm",
            "Active",
        )];
        let detailed = evaluation(vec![
            policy_report("policy-a", vec![permission(0, true)]),
            policy_report("policy-b", vec![prohibition(0, true)]),
        ]);

        let bad = compare_case(
            &shadow,
            &exp,
            DutyMode::Advise,
            Behaviour::Closed,
            &detailed,
            &response(WireDecision::Deny),
        )
        .unwrap();
        assert!(bad.iter().any(|m| m.contains("policy-b")), "{bad:?}");
    }
}
