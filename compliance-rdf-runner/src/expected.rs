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

/// Parses `dsc:TestCase`'s optional `dsc:expectedDecision` literal into
/// the engine's own `WireDecision`. `None` when the fixture states no
/// opinion about the coarse decision (every fixture before this term
/// existed, and any future fixture that prefers to state its
/// expectations only via the per-policy `report:` tree).
pub fn parse_expected_decision(
    g: &Graph,
    testcase: &str,
) -> Result<Option<engine::WireDecision>, String> {
    let Some(literal) = g.literal(testcase, &dsc_expectedDecision()) else {
        return Ok(None);
    };
    match literal.as_str() {
        "Allow" => Ok(Some(engine::WireDecision::Allow)),
        "Deny" => Ok(Some(engine::WireDecision::Deny)),
        "Error" => Ok(Some(engine::WireDecision::Error)),
        other => Err(format!(
            "{testcase}: dsc:expectedDecision {other:?} is not one of \
             engine::wire::WireDecision's own variant spellings \
             \"Allow\"/\"Deny\"/\"Error\""
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PREFIXES: &str = "\
@base <https://example.org/tc> .
@prefix : <#> .
@prefix dsc: <https://ds-labs-org.github.io/ds-odrl-compliance-rdf/ns#> .
";

    /// `Graph::parse` only reads from a path, so each test writes its own
    /// tiny scratch fixture to a uniquely-named file under the OS temp dir
    /// and cleans it up afterward -- no dependency on the external
    /// `ds-odrl-compliance-rdf` corpus, per this unit's own scope (that
    /// corpus's own new fixtures are a separate repo's concern).
    fn parse_ttl(unique: &str, ttl: &str) -> Graph {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "compliance-rdf-runner-expected-decision-test-{unique}-{}.ttl",
            std::process::id()
        ));
        {
            let mut f = std::fs::File::create(&path).expect("create scratch ttl fixture");
            f.write_all(ttl.as_bytes())
                .expect("write scratch ttl fixture");
        }
        let g = Graph::parse(&path).expect("parse scratch ttl fixture");
        let _ = std::fs::remove_file(&path);
        g
    }

    const TESTCASE: &str = "https://example.org/tc#testcase";

    #[test]
    fn expected_decision_allow_parses_to_wire_decision_allow() {
        let g = parse_ttl(
            "allow",
            &format!("{PREFIXES}:testcase a dsc:TestCase ; dsc:expectedDecision \"Allow\" .\n"),
        );
        assert_eq!(
            parse_expected_decision(&g, TESTCASE).unwrap(),
            Some(engine::WireDecision::Allow)
        );
    }

    #[test]
    fn expected_decision_deny_parses_to_wire_decision_deny() {
        let g = parse_ttl(
            "deny",
            &format!("{PREFIXES}:testcase a dsc:TestCase ; dsc:expectedDecision \"Deny\" .\n"),
        );
        assert_eq!(
            parse_expected_decision(&g, TESTCASE).unwrap(),
            Some(engine::WireDecision::Deny)
        );
    }

    #[test]
    fn expected_decision_error_parses_to_wire_decision_error() {
        let g = parse_ttl(
            "error",
            &format!("{PREFIXES}:testcase a dsc:TestCase ; dsc:expectedDecision \"Error\" .\n"),
        );
        assert_eq!(
            parse_expected_decision(&g, TESTCASE).unwrap(),
            Some(engine::WireDecision::Error)
        );
    }

    #[test]
    fn expected_decision_absent_is_none() {
        let g = parse_ttl("absent", &format!("{PREFIXES}:testcase a dsc:TestCase .\n"));
        assert_eq!(parse_expected_decision(&g, TESTCASE).unwrap(), None);
    }

    #[test]
    fn expected_decision_unrecognized_value_is_an_error() {
        let g = parse_ttl(
            "bad",
            &format!("{PREFIXES}:testcase a dsc:TestCase ; dsc:expectedDecision \"Maybe\" .\n"),
        );
        let err = parse_expected_decision(&g, TESTCASE).unwrap_err();
        assert!(
            err.contains("Maybe"),
            "error should name the bad value: {err}"
        );
    }
}
