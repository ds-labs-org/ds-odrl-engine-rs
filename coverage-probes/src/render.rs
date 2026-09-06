//! The exported artifact's shape, and the one serialization detail that
//! makes it committable at all.
//!
//! Deliberately carries **no decision this engine produced**. Every probe
//! records a request and the outcome that would *demonstrate* its row's
//! documented status; the browser computes the decisions themselves, live,
//! against the compiled `engine.wasm`. Anything else would let the page
//! display a verdict it was handed rather than one it computed — the same
//! rule `compliance-runner/src/cases.rs` already holds itself to.

use serde::Serialize;
use serde_json::Value;

/// Version tag for this file's shape, checked by the site before it trusts
/// a fetched artifact (a stale, browser-cached copy of an older shape must
/// fail loudly rather than half-parse).
pub const SCHEMA: &str = "ds-odrl-engine-rs/odrl-coverage@1";

pub const GENERATED_BY: &str = "coverage-probes (cargo run -p coverage-probes --release)";

pub const SPEC: &str = "W3C ODRL 2.2 Vocabulary & Expression, https://www.w3.org/TR/odrl-vocab/";

pub const SOURCE_ANALYSIS: &str =
    "docs/spikes/2026-09-05-odrl-2.2-vocabulary-gap-analysis.md (Deepthought-Solutions/dataspace)";

pub const NOTE: &str = "Each `probe` is one exact engine::wire::Request plus the outcome that would \
                        demonstrate its row's documented status. No decision this engine produced is \
                        recorded here -- the browser computes every one.";

/// One of the ten sections of the source gap analysis.
#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub id: &'static str,
    pub number: u32,
    pub title: &'static str,
    pub spec_ref: &'static str,
}

/// One vocabulary claim: a term, the status this study documents for it,
/// and either the probes that put that status to the test in a browser or
/// the reason no request can.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub category: &'static str,
    pub term: String,
    /// `Implemented` | `Partial` | `NotImplemented` | `OutOfScope`.
    pub status: &'static str,
    pub why: String,
    pub evidence: String,
    pub asserts: String,
    pub probe_ids: Vec<String>,
    /// Non-null exactly when `probe_ids` is empty: the reason this row's
    /// claim is not about the wire contract `evaluate()` implements, and so
    /// cannot be probed by any request at all.
    pub documented_because: Option<String>,
    /// A limit on what this row's own probes establish, rendered on the row
    /// in the same place `documented_because` is.
    pub caveat: Option<String>,
}

/// What the browser must observe for one probe to agree with its row.
///
/// `duties` and `dataset_id` are `null` on most probes and asserted only
/// where the row's claim actually turns on them — `null` means "not
/// asserted", `[]` means "asserted to be empty", which are very different
/// things for a fail-open duty claim.
#[derive(Debug, Clone, Serialize)]
pub struct Expect {
    pub decision: &'static str,
    pub reason_contains: Vec<String>,
    pub reason_excludes: Vec<String>,
    pub duties: Option<Vec<DutyExpect>>,
    pub dataset_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DutyExpect {
    pub policy_id: String,
    pub action: String,
    pub resolved: bool,
    /// `DutyEntry::source` — where in the policy the duty was attached,
    /// present only for a duty that is not a plain policy-level obligation
    /// (`permission[0].duty[0]`, `prohibition[0].remedy[0]`, either with a
    /// `.consequence` suffix). Skipped when absent so every duty
    /// expectation recorded before nested duties existed serializes
    /// exactly as it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One `evaluate()` call, and the outcome that would demonstrate its row's
/// claim.
#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub id: &'static str,
    /// `positive` (the feature works) or `negative` (and here is the input
    /// that makes it *not* fire / the property that is inert).
    pub kind: &'static str,
    pub title: String,
    pub asserts: String,
    pub falsified_by: String,
    /// The complete, already-patched Section 5.2 request.
    pub request: Value,
    pub expect: Expect,
}

#[derive(Debug, Serialize)]
pub struct CoverageFile<'a> {
    pub schema: &'static str,
    pub generated_by: &'static str,
    pub spec: &'static str,
    pub source_analysis: &'static str,
    pub note: &'static str,
    pub categories: &'a [Category],
    pub rows: &'a [Row],
    pub probes: &'a [Probe],
}

/// Pretty-printed, matching `compliance-runner/src/report.rs`'s and
/// `cases.rs`'s own choice: this is a committed, reviewable artifact, and
/// a catalog change should show up as a readable diff rather than one
/// 400 KB line.
///
/// **Where this artifact's determinism actually comes from — measured,
/// not reasoned about.** `engine::Request::claims` is a `HashMap`, whose
/// iteration order (and so whose key order on the wire) is randomized per
/// process by `RandomState`. Serializing a `Request` straight to a
/// `Write`r reproduces that randomness verbatim: a throwaway binary built
/// against this workspace's own `engine`, serializing one two-claim
/// request once per process across eight processes, emitted the two keys
/// in a different order in three of them. Routed through
/// `serde_json::to_value` first, all eight were identical.
/// `serde_json::Value::Object` is a `BTreeMap` in this workspace (nothing
/// anywhere enables serde_json's `preserve_order` feature — checked), so
/// the conversion re-sorts every object's keys regardless of which field
/// happened to be hash-ordered.
///
/// For a probe's `claims` specifically, the conversion that neutralizes
/// this is `catalog::build`'s own `serde_json::to_value(&request)` — which
/// that function needs anyway, since patches apply to a `Value` — so the
/// `Value` reaching this function is already canonical below the
/// envelope. The `to_value` here canonicalizes the **envelope's** own key
/// order, and, more usefully, keeps the guarantee from resting on a
/// coincidence of the current struct layout: the moment any type in this
/// file gains a `HashMap`-shaped field, this line is what stops the
/// artifact silently becoming non-deterministic again and failing this
/// repo's `git diff --exit-code compliance/reports/` CI check on pure
/// key-order noise. Both halves have their own test —
/// `catalog::tests::two_independently_randomized_claims_maps_build_to_identical_probe_json`
/// for the probe path, this module's
/// `object_keys_are_canonically_sorted_in_the_emitted_bytes` for the
/// envelope.
pub fn render(file: &CoverageFile<'_>) -> String {
    let value = serde_json::to_value(file).expect("CoverageFile always serializes");
    serde_json::to_string_pretty(&value).expect("a serde_json::Value always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{ClaimValue, Claims};
    use std::collections::HashMap;

    fn probe_with_claims(claims: Claims) -> Probe {
        let mut request = crate::catalog::base_request();
        request.claims = claims;
        Probe {
            id: "probe-x",
            kind: "positive",
            title: "t".to_string(),
            asserts: "a".to_string(),
            falsified_by: "f".to_string(),
            request: serde_json::to_value(&request).unwrap(),
            expect: Expect {
                decision: "Allow",
                reason_contains: vec![],
                reason_excludes: vec![],
                duties: None,
                dataset_id: None,
            },
        }
    }

    fn rendered(probe: Probe) -> String {
        let probes = [probe];
        render(&CoverageFile {
            schema: SCHEMA,
            generated_by: GENERATED_BY,
            spec: SPEC,
            source_analysis: SOURCE_ANALYSIS,
            note: NOTE,
            categories: &[],
            rows: &[],
            probes: &probes,
        })
    }

    #[test]
    fn rendering_is_byte_identical_across_reorderings_of_a_multi_claim_request() {
        // End-to-end over the whole pipeline (a request converted to
        // `Value`, then rendered), which is how the real generator runs.
        // `HashMap::new()` gets a fresh, independently randomized
        // `RandomState` per call, so two separately built maps holding the
        // same entries are a real (not staged) instance of the same
        // non-determinism an adversarial review caught in
        // `compliance-runner/src/cases.rs` -- see `render`'s own doc
        // comment for the eight-process measurement showing it does bite
        // here, and `catalog::tests::\
        // two_independently_randomized_claims_maps_build_to_identical_probe_json`
        // for the same property asserted at the exact point that
        // neutralizes it.
        let mut a: Claims = HashMap::new();
        a.insert("sub".to_string(), ClaimValue::Single("alice".to_string()));
        a.insert("dateTime".to_string(), ClaimValue::Single("2026-09-05T12:00:00Z".to_string()));
        a.insert("nationality".to_string(), ClaimValue::Single("DE".to_string()));
        a.insert("scope".to_string(), ClaimValue::Multi(vec!["read".to_string(), "write".to_string()]));

        let mut b: Claims = HashMap::new();
        b.insert("scope".to_string(), ClaimValue::Multi(vec!["read".to_string(), "write".to_string()]));
        b.insert("nationality".to_string(), ClaimValue::Single("DE".to_string()));
        b.insert("dateTime".to_string(), ClaimValue::Single("2026-09-05T12:00:00Z".to_string()));
        b.insert("sub".to_string(), ClaimValue::Single("alice".to_string()));

        assert_eq!(
            rendered(probe_with_claims(a)),
            rendered(probe_with_claims(b)),
            "two logically identical claims maps, built via independently-randomized HashMap \
             instances, must render to byte-identical JSON"
        );
    }

    #[test]
    fn the_envelope_carries_the_schema_tag_and_no_decision_of_its_own() {
        let text = rendered(probe_with_claims(Claims::new()));
        let value: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["schema"], SCHEMA);
        assert_eq!(value["probes"].as_array().unwrap().len(), 1);
        // The whole point of this artifact: nothing in it may pre-decide
        // what the browser is supposed to compute.
        for forbidden in ["decision", "actual", "observed", "total", "passed", "failed"] {
            assert!(value.get(forbidden).is_none(), "envelope must not carry `{forbidden}`");
            assert!(value["probes"][0].get(forbidden).is_none(), "a probe must not carry `{forbidden}`");
        }
        // `expect.decision` is the one decision-shaped field, and it is the
        // *expectation*, not an observation.
        assert_eq!(value["probes"][0]["expect"]["decision"], "Allow");
    }

    #[test]
    fn object_keys_are_canonically_sorted_in_the_emitted_bytes() {
        let text = rendered(probe_with_claims(Claims::new()));
        let schema_at = text.find("\"schema\"").expect("schema key present");
        let probes_at = text.find("\"probes\"").expect("probes key present");
        assert!(
            probes_at < schema_at,
            "serde_json::Value's BTreeMap sorts keys, so `probes` must precede `schema` in the \
             emitted bytes regardless of struct field order -- if this fails, the Value \
             indirection was dropped and the artifact is no longer deterministic"
        );
    }
}
