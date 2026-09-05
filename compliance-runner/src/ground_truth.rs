//! Derives a single Allow/Deny verdict from one `test_cases/*.ttl`
//! expected-report fixture, in the `https://w3id.org/force/compliance-report#`
//! vocabulary.
//!
//! **The mapping rule, investigated directly from the vendored fixtures**
//! (grepped every `report:` predicate/class across all of
//! `data/test_cases/*.ttl` before writing this, per the task's own
//! instruction — see `compliance-runner`'s README note for the sample
//! set this was built from): each `report:PolicyReport` carries one
//! `report:ruleReport` per permission/prohibition the source `Policy`
//! declares, typed `report:PermissionReport` or `report:ProhibitionReport`,
//! each with its own `report:activationState` of `report:Active` or
//! `report:Inactive` — "did this specific rule's premises (party, action,
//! target, constraint) all hold for this request." From that:
//!
//! - Any `ProhibitionReport` with `activationState: Active` → **Deny**
//!   (deny-overrides — matches this engine's own `decide` precedence, and
//!   XACML's, Section 3.2/4.3).
//! - Else any `PermissionReport` with `activationState: Active` → **Allow**.
//! - Else → **Deny**. This is the ODRL Formal Semantics draft's own
//!   *closed* default Behaviour (Section 3.6: "anything that is not
//!   permitted is prohibited") — confirmed against `testcase-014-alice-sell.ttl`,
//!   whose sole rule is a `ProhibitionReport` reported `Inactive` (the
//!   prohibited action is `use`, the request's is `sell`) with **no**
//!   `PermissionReport` at all, yet the fixture's own title still reads
//!   "results into no." A policy with nothing that actively grants a
//!   request denies it, full stop — unlike this engine's own Section 4.3
//!   departure (a policy with a literally *empty* permissions list is
//!   treated as open), which is exactly why that departure is named in
//!   Section 4.3 as a reviewable, non-default choice: this reference
//!   suite is built against the *other* reading. A translated request
//!   whose sole policy nets no surviving rules is therefore an empty
//!   `policies` array (see `translate.rs`), not an empty-permissions
//!   shell — matching this closed default instead of accidentally
//!   invoking the engine's own open one.
//!
//! `report:DutyReport` rule-reports are ignored here: every fixture that
//! carries one is already skipped by `translate.rs` before this function
//! is ever consulted (per-permission nested duties, Section 7).

use engine::WireDecision;

use crate::graph::{report_ns, Graph};

pub fn expected_decision(g: &Graph) -> WireDecision {
    let permission_report = report_ns("PermissionReport");
    let prohibition_report = report_ns("ProhibitionReport");
    let activation_state = report_ns("activationState");
    let active = report_ns("Active");

    let mut any_prohibition_active = false;
    let mut any_permission_active = false;

    for triple in g.triples() {
        if triple.predicate.as_str() != activation_state {
            continue;
        }
        let subject = match &triple.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            oxrdf::NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
        };
        let is_active = matches!(&triple.object, oxrdf::Term::NamedNode(n) if n.as_str() == active);
        match g.type_of(&subject) {
            Some(t) if t == prohibition_report && is_active => any_prohibition_active = true,
            Some(t) if t == permission_report && is_active => any_permission_active = true,
            _ => {}
        }
    }

    if any_prohibition_active {
        WireDecision::Deny
    } else if any_permission_active {
        WireDecision::Allow
    } else {
        WireDecision::Deny
    }
}
