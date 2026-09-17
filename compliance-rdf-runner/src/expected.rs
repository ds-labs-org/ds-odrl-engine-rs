//! Parses a `dsc:expectedOutcome` `report:PolicyReport` tree (per
//! `docs/vocabulary-spec.md` section 3's "expected outcomes are the FORCE
//! `report:` compliance-report vocabulary, used verbatim") into a
//! comparable structure -- the ground truth `compare.rs` checks the
//! engine's real `engine::DetailedEvaluation` against.
//!
//! Deliberately loose on one axis: which of `report:PermissionReport`/
//! `ProhibitionReport`/`DutyReport` a rule report is, and which rule
//! (`report:rule`) it is about, are read and used to associate an
//! expected report with the matching actual one, but the states
//! themselves (`report:activationState` etc.) are read as plain strings
//! (`"Active"`/`"Inactive"`/...) rather than re-parsed into the engine's
//! own `report::ActivationState` etc. enums -- `compare.rs` formats the
//! engine's own enum values to the identical strings for comparison, so
//! there is exactly one place (`compare::state_name`) that has to agree
//! with this vocabulary's own term spelling, not two independent enum
//! tables that could drift apart.

use crate::graph::{local_name, Graph};
use crate::vocab::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpectedRuleKind {
    Permission,
    Prohibition,
    Duty,
}

/// One `report:PermissionReport`/`ProhibitionReport`/`DutyReport` node.
/// `rule` is the local name of the `report:rule` this report is about --
/// used to match against the engine's own `rule_index`-addressed reports
/// by walking the *same* source rule lists this crate's own `translate.rs`
/// already built (see `compare.rs`).
#[derive(Debug, Clone)]
pub struct ExpectedRuleReport {
    pub kind: ExpectedRuleKind,
    pub rule: String,
    pub activation_state: Option<String>,
    pub attempt_state: Option<String>,
    pub performance_state: Option<String>,
    pub deontic_state: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExpectedPolicyReport {
    /// Local name of `report:policy` -- correlates to `WirePolicy::id`.
    pub policy_id: String,
    pub rule_reports: Vec<ExpectedRuleReport>,
}

fn rule_kind(g: &Graph, node: &str) -> Result<ExpectedRuleKind, String> {
    if g.has_type(node, &report_PermissionReport()) {
        Ok(ExpectedRuleKind::Permission)
    } else if g.has_type(node, &report_ProhibitionReport()) {
        Ok(ExpectedRuleKind::Prohibition)
    } else if g.has_type(node, &report_DutyReport()) {
        Ok(ExpectedRuleKind::Duty)
    } else {
        Err(format!("{node}: report:ruleReport member has none of PermissionReport/ProhibitionReport/DutyReport"))
    }
}

fn parse_rule_report(g: &Graph, node: &str) -> Result<ExpectedRuleReport, String> {
    let kind = rule_kind(g, node)?;
    let rule = g
        .object_id(node, &report_rule())
        .map(|r| local_name(&r).to_string())
        .ok_or_else(|| format!("{node}: report:ruleReport member has no report:rule"))?;

    let state = |predicate: String| {
        g.object_id(node, &predicate)
            .map(|s| local_name(&s).to_string())
    };

    Ok(ExpectedRuleReport {
        kind,
        rule,
        activation_state: state(report_activationState()),
        attempt_state: state(report_attemptState()),
        performance_state: state(report_performanceState()),
        deontic_state: state(report_deonticState()),
    })
}

/// Parses one `dsc:expectedOutcome` node -- a `report:PolicyReport` --
/// into an `ExpectedPolicyReport`. Recurses into a rule report's own
/// nested duty positions is unnecessary: `docs/vocabulary-spec.md`'s own
/// convention (section 5, "reported as a SEPARATE, sibling
/// report:ruleReport") is that every duty/remedy/consequence report is a
/// flat sibling under `report:ruleReport`, never nested -- matching
/// `engine::DetailedPolicyReport::rule_reports`'s own flat shape exactly,
/// so no tree-walk is needed on either side.
pub fn parse_expected_policy_report(g: &Graph, node: &str) -> Result<ExpectedPolicyReport, String> {
    let policy_id = g
        .object_id(node, &report_policy())
        .map(|p| local_name(&p).to_string())
        .ok_or_else(|| format!("{node}: report:PolicyReport has no report:policy"))?;
    let rule_reports = g
        .object_ids(node, &report_ruleReport())
        .iter()
        .map(|r| parse_rule_report(g, r))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExpectedPolicyReport {
        policy_id,
        rule_reports,
    })
}
