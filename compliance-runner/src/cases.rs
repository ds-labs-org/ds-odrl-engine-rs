//! The per-case *fixture* export: the exact `engine::Request` this runner
//! fed `engine::evaluate_request` for each case, plus the vendored suite's
//! own expected decision — so an independent host (this repo's own site,
//! driving the compiled `engine.wasm` in a visitor's browser) can re-run
//! the identical corpus and reach its own verdict, instead of displaying
//! one this native run baked into `latest.json`.
//!
//! Deliberately carries NO tally and NO decision this run produced. The
//! only verdict-shaped field here is `expected_decision`, which is the
//! vendored suite's own ground truth (`ground_truth::expected_decision`
//! over the fixture's `report:*` expected report) — i.e. the thing under
//! test, not this engine's answer to it. Anything else would let the
//! browser's page display a number it was handed rather than one it
//! computed, which is exactly what the live runner exists not to do.
//!
//! Written next to (never instead of) `report.rs`'s `latest.md`/
//! `latest.json`: those stay byte-for-byte what they were, and the site
//! now treats `latest.json` as the *native* run's recorded baseline to
//! cross-check its own live, in-browser run against.

use engine::{Request, WireDecision};
use serde::Serialize;

/// Version tag for this file's shape, checked by the site before it
/// trusts a fetched artifact (a stale, browser-cached copy of an older
/// shape must fail loudly rather than half-parse).
pub const SCHEMA: &str = "ds-odrl-engine-rs/compliance-cases@1";

const SUITE: &str = "SolidLabResearch/ODRL-Test-Suite, vendored at compliance/vendor/odrl-test-suite";

const NOTE: &str = "Each `request` is the exact engine::wire::Request compliance-runner fed to \
                    engine::evaluate_request natively for this case; `expected_decision` is \
                    ground_truth::expected_decision over the fixture's own report:* expected \
                    report. No decision this engine produced is recorded here -- the browser \
                    recomputes them.";

/// What one case contributed, kept alongside its `CaseResult` so both are
/// built from the *same* bindings in `main.rs`'s per-case closure rather
/// than reconstructed (and possibly diverging) afterwards.
pub enum FixtureData {
    Ready { request: Request, expected: WireDecision },
    Skipped { reason: String },
}

/// One case as it appears in the exported file. A case is either
/// evaluable (`request` + `expected_decision`, no `skip_reason`) or
/// skipped (`skip_reason` only) — never both, never neither.
#[derive(Serialize)]
pub struct CaseFixture<'a> {
    pub slug: &'a str,
    pub title: &'a str,
    /// Present exactly when this case was evaluated; absent for a skip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<&'a Request>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<&'a str>,
}

#[derive(Serialize)]
struct CaseFile<'a> {
    schema: &'static str,
    suite: &'static str,
    note: &'static str,
    cases: &'a [CaseFixture<'a>],
}

/// Same three arms as `report.rs`'s own `decision_str`, deliberately not
/// shared with it: that one names the decision *this run produced*, this
/// one names the *suite's expected* decision, and the two files are meant
/// to be able to change independently without one silently re-spelling
/// the other's vocabulary.
pub fn decision_str(d: WireDecision) -> &'static str {
    match d {
        WireDecision::Allow => "Allow",
        WireDecision::Deny => "Deny",
        WireDecision::Error => "Error",
    }
}

/// Borrows one owned [`FixtureData`] into its serializable view.
pub fn fixture_view<'a>(slug: &'a str, title: &'a str, data: &'a FixtureData) -> CaseFixture<'a> {
    match data {
        FixtureData::Ready { request, expected } => CaseFixture {
            slug,
            title,
            request: Some(request),
            expected_decision: Some(decision_str(*expected)),
            skip_reason: None,
        },
        FixtureData::Skipped { reason } => CaseFixture {
            slug,
            title,
            request: None,
            expected_decision: None,
            skip_reason: Some(reason),
        },
    }
}

/// Pretty-printed, matching `report.rs`'s own `to_string_pretty` choice:
/// this is a committed, reviewable artifact, and a translation change
/// that alters one fixture's request should show up as a readable diff in
/// a pull request rather than one 280 KB line.
///
/// Serializes through `serde_json::Value` rather than calling
/// `to_string_pretty` on `CaseFile` directly — load-bearing, not style.
/// `engine::Request::claims` is a `HashMap`, whose iteration order (and so
/// whose *key order on the wire*) is randomized per process
/// (`RandomState`); serializing it straight to a `Write`r reproduces that
/// randomness verbatim in the output. `serde_json::Value::Object` is a
/// `BTreeMap` in this workspace (no crate anywhere enables serde_json's
/// `preserve_order` feature — checked), so converting to `Value` first
/// re-sorts every object's keys deterministically before printing,
/// independent of which field happens to be hash-ordered. Confirmed by
/// this module's own
/// `rendering_is_byte_identical_across_reorderings_of_a_multi_claim_request`
/// test, and the reason this artifact can be a `git diff --exit-code`
/// CI check at all (an adversarial review of this feature caught a
/// version of this file that lacked this indirection producing a
/// different byte sequence on every single run).
pub fn render(cases: &[CaseFixture<'_>]) -> String {
    let file = CaseFile { schema: SCHEMA, suite: SUITE, note: NOTE, cases };
    let value = serde_json::to_value(&file).expect("CaseFile always serializes");
    serde_json::to_string_pretty(&value).expect("a serde_json::Value always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::profile::{Behaviour, DutyMode};
    use engine::wire::WireActionDecl;
    use engine::{Constraint, Operator, RequestConfig, Rule, WirePolicy};
    use serde_json::Value;

    fn a_request() -> Request {
        Request {
            dataset_id: "urn:uuid:dataset-1".to_string(),
            action: "read".to_string(),
            config: RequestConfig {
                type_: "odrl:Profile".to_string(),
                id: "https://ds42.org/profiles/compliance-runner".to_string(),
                actions: vec![WireActionDecl { id: "read".to_string(), included_in: None }],
                duty_mode: DutyMode::Advise,
                behaviour: Behaviour::Closed,
            },
            policies: vec![WirePolicy {
                id: "policy-1".to_string(),
                kind: "Set".to_string(),
                assigner: "urn:uuid:assigner".to_string(),
                assignee: None,
                permissions: vec![Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "alice")])],
                prohibitions: vec![],
                obligations: vec![],
            }],
            claims: Default::default(),
        }
    }

    fn rendered(data: FixtureData) -> Value {
        let view = fixture_view("testcase-001-alice", "A title.", &data);
        serde_json::from_str(&render(std::slice::from_ref(&view))).expect("render emits valid JSON")
    }

    #[test]
    fn a_ready_fixture_serializes_the_request_verbatim_and_no_skip_reason() {
        let request = a_request();
        let expected_request = serde_json::to_value(&request).unwrap();
        let file = rendered(FixtureData::Ready { request, expected: WireDecision::Allow });

        let case = &file["cases"][0];
        assert_eq!(case["slug"], "testcase-001-alice");
        assert_eq!(case["title"], "A title.");
        assert_eq!(case["expected_decision"], "Allow");
        assert_eq!(case["request"], expected_request);
        assert!(case.get("skip_reason").is_none(), "a ready case must carry no skip_reason");
    }

    #[test]
    fn a_skipped_fixture_serializes_the_reason_and_neither_request_nor_expected_decision() {
        let file = rendered(FixtureData::Skipped { reason: "odrl:xone is not expressible".to_string() });

        let case = &file["cases"][0];
        assert_eq!(case["skip_reason"], "odrl:xone is not expressible");
        assert!(case.get("request").is_none(), "a skipped case must carry no request");
        assert!(case.get("expected_decision").is_none(), "a skipped case must carry no expected_decision");
    }

    #[test]
    fn the_envelope_carries_the_schema_tag_and_no_tally_of_its_own() {
        let file = rendered(FixtureData::Ready { request: a_request(), expected: WireDecision::Deny });

        assert_eq!(file["schema"], SCHEMA);
        assert!(file["suite"].as_str().unwrap().contains("ODRL-Test-Suite"));
        assert_eq!(file["cases"].as_array().unwrap().len(), 1);
        // The whole point of this artifact: nothing in it may pre-decide
        // the outcome the browser is supposed to compute.
        for forbidden in ["total", "passed", "failed", "skipped", "decision", "actual"] {
            assert!(file.get(forbidden).is_none(), "envelope must not carry `{forbidden}`");
            assert!(file["cases"][0].get(forbidden).is_none(), "a case must not carry `{forbidden}`");
        }
    }

    #[test]
    fn every_wire_decision_has_a_stable_string_spelling() {
        assert_eq!(decision_str(WireDecision::Allow), "Allow");
        assert_eq!(decision_str(WireDecision::Deny), "Deny");
        assert_eq!(decision_str(WireDecision::Error), "Error");
    }

    #[test]
    fn rendering_is_byte_identical_across_reorderings_of_a_multi_claim_request() {
        // The exact regression an adversarial review caught: `render`
        // used to call `serde_json::to_string_pretty` directly on the
        // struct, which serializes `Request::claims` (a `HashMap`) in
        // that map's own randomized iteration order -- five consecutive
        // `cargo run` invocations produced five differently-byte-ordered
        // files, which would fail this repo's own `git diff --exit-code
        // compliance/reports/` CI check on pure key-order noise, not a
        // real change. `HashMap::new()` gets a fresh, independently
        // randomized `RandomState` each call, so two separately built
        // maps holding the same four entries are a real (not staged)
        // instance of the same non-determinism the bug report found --
        // this asserts `render`'s canonicalization neutralizes it
        // regardless of which iteration order either happened to land on.
        fn request_with_claims(claims: engine::Claims) -> Request {
            let mut request = a_request();
            request.claims = claims;
            request
        }

        let mut claims_a: engine::Claims = std::collections::HashMap::new();
        claims_a.insert("sub".to_string(), "alice".to_string().into());
        claims_a.insert("dateTime".to_string(), "2024-02-12T11:20:10.999Z".to_string().into());
        claims_a.insert("nationality".to_string(), "DE".to_string().into());
        claims_a.insert("scope".to_string(), "read".to_string().into());

        let mut claims_b: engine::Claims = std::collections::HashMap::new();
        claims_b.insert("scope".to_string(), "read".to_string().into());
        claims_b.insert("nationality".to_string(), "DE".to_string().into());
        claims_b.insert("dateTime".to_string(), "2024-02-12T11:20:10.999Z".to_string().into());
        claims_b.insert("sub".to_string(), "alice".to_string().into());

        let data_a = FixtureData::Ready { request: request_with_claims(claims_a), expected: WireDecision::Allow };
        let data_b = FixtureData::Ready { request: request_with_claims(claims_b), expected: WireDecision::Allow };
        let view_a = fixture_view("testcase-x", "title", &data_a);
        let view_b = fixture_view("testcase-x", "title", &data_b);

        assert_eq!(
            render(std::slice::from_ref(&view_a)),
            render(std::slice::from_ref(&view_b)),
            "two logically identical claims maps, built via independently-randomized HashMap \
             instances, must render to byte-identical JSON"
        );
    }
}
