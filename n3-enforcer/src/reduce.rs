//! Compliance report -> Allow/Deny.
//!
//! The same reduction `compliance-runner/src/ground_truth.rs` applies to
//! the suite's expected reports (and the Evaluator benchmark's
//! `reduceToDecision`): any active prohibition denies; otherwise any
//! active permission allows; otherwise deny (ODRL Formal Semantics'
//! *closed* default). A rule reported both `Active` and `Inactive` is a
//! reasoner-consistency failure and is surfaced in the summary rather than
//! silently resolved.

use std::collections::{BTreeMap, BTreeSet};

use engine::{Response, WireDecision};
use eyeron::Term;

const REPORT: &str = "https://w3id.org/force/compliance-report#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReportSummary {
    pub active_permissions: Vec<String>,
    pub active_prohibitions: Vec<String>,
    /// Rule reports that carry both `Active` and `Inactive`.
    pub contradictory: Vec<String>,
}

pub fn reduce(turtle: &str) -> Result<ReportSummary, String> {
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    let mut states: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let doc = eyeron::parse_n3(turtle, None).map_err(|e| e.to_string())?;
    for t in &doc.facts {
        let s = match &t.s {
            Term::Iri(iri) => iri.clone(),
            Term::Blank(label) => format!("_:{label}"),
            _ => continue,
        };
        let (Term::Iri(p), Term::Iri(o)) = (&t.p, &t.o) else { continue };
        if p == RDF_TYPE {
            types.entry(s).or_insert_with(|| o.clone());
        } else if p == &format!("{REPORT}activationState") {
            states.entry(s).or_default().insert(o.clone());
        }
    }
    let mut out = ReportSummary::default();
    for (s, st) in &states {
        let active = st.contains(&format!("{REPORT}Active"));
        if active && st.contains(&format!("{REPORT}Inactive")) {
            out.contradictory.push(s.clone());
        }
        if !active {
            continue;
        }
        match types.get(s).map(String::as_str) {
            Some(t) if t == format!("{REPORT}ProhibitionReport") => out.active_prohibitions.push(s.clone()),
            Some(t) if t == format!("{REPORT}PermissionReport") => out.active_permissions.push(s.clone()),
            _ => {}
        }
    }
    Ok(out)
}

pub fn to_response(dataset_id: &str, s: &ReportSummary) -> Response {
    let (decision, reason) = if !s.active_prohibitions.is_empty() {
        (WireDecision::Deny, format!("n3: {} active prohibition report(s)", s.active_prohibitions.len()))
    } else if !s.active_permissions.is_empty() {
        (WireDecision::Allow, format!("n3: {} active permission report(s)", s.active_permissions.len()))
    } else {
        (WireDecision::Deny, "n3: no active permission report (closed default)".to_string())
    };
    let reason = if s.contradictory.is_empty() {
        reason
    } else {
        format!("{reason}; WARNING {} rule report(s) both Active and Inactive", s.contradictory.len())
    };
    Response { dataset_id: dataset_id.to_string(), decision, reason, duties: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(rules: &[(&str, &str, &str)]) -> String {
        let mut s = format!("@prefix r: <{REPORT}> .\n");
        for (id, ty, state) in rules {
            s += &format!("<urn:x:{id}> a r:{ty} ; r:activationState r:{state} .\n");
        }
        s
    }

    #[test]
    fn prohibition_overrides_permission() {
        let s = reduce(&report(&[("a", "PermissionReport", "Active"), ("b", "ProhibitionReport", "Active")])).unwrap();
        assert_eq!(to_response("d", &s).decision, WireDecision::Deny);
    }

    #[test]
    fn active_permission_allows_and_nothing_active_denies() {
        let s = reduce(&report(&[("a", "PermissionReport", "Active")])).unwrap();
        assert_eq!(to_response("d", &s).decision, WireDecision::Allow);
        let s = reduce(&report(&[("a", "PermissionReport", "Inactive")])).unwrap();
        assert_eq!(to_response("d", &s).decision, WireDecision::Deny);
        assert_eq!(to_response("d", &reduce("").unwrap()).decision, WireDecision::Deny);
    }

    #[test]
    fn active_and_inactive_is_flagged_not_resolved() {
        let s = reduce(&report(&[("a", "PermissionReport", "Active"), ("a", "PermissionReport", "Inactive")])).unwrap();
        assert_eq!(s.contradictory.len(), 1);
        assert!(to_response("d", &s).reason.contains("WARNING"));
    }
}
