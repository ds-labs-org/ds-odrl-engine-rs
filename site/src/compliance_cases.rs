//! Pure, browser-free logic for the live compliance runner: parsing the
//! cases artifact (`compliance/reports/latest-cases.json`, see
//! `compliance-runner/src/cases.rs`), comparing one live `engine.wasm`
//! decision against the vendored suite's expected one, and tallying the
//! finished report — including the cross-check against the *native* run's
//! own recorded baseline (`latest.json`).
//!
//! Deliberately **not** `#[cfg(target_arch = "wasm32")]`-gated like every
//! other module in this crate. `cargo test --workspace` is a native
//! build, so a gated module's unit tests would silently never compile,
//! let alone run — and these are exactly the parts that must be testable
//! with no browser and no wasm instance. Only this module's callers
//! (`compliance_run.rs`, `compliance_page.rs`) touch the DOM or the ABI;
//! nothing here does.
//!
//! What this module deliberately does *not* do: deserialize a case's
//! `request` into `crate::wire`'s types and re-serialize it. Two reasons.
//! `crate::wire::Constraint` models only the flat
//! `left_operand`/`operator`/`right_operand` shape, so a native
//! `odrl:and`/`odrl:or`/`odrl:xone` constraint (which `engine` supports
//! and `translate.rs` could one day emit) would be silently dropped; and
//! `serde_json::Value`'s map is key-sorted, so a round trip through it is
//! not byte-identical to what the native run evaluated. The raw bytes go
//! to `engine.wasm` verbatim, via `serde_json::value::RawValue`.

use serde::Deserialize;
use serde_json::value::RawValue;

/// The live corpus, copied to `dist/compliance-data/` by `index.html`'s
/// own `copy-file` directive (same target directory, and same
/// `/compliance`-route-collision reason, as `latest.json` beside it).
pub const CASES_URL: &str = "compliance-data/latest-cases.json";

/// The native `compliance-runner` run's own recorded verdicts — no longer
/// this page's source of truth, now the thing the live run is compared
/// *against*.
pub const BASELINE_URL: &str = "compliance-data/latest.json";

/// Must equal `compliance-runner/src/cases.rs`'s own `SCHEMA`. Checked
/// rather than assumed: `copy-file` assets are not content-hashed, so a
/// returning visitor can be served a browser-cached artifact of an older
/// shape, and that must fail loudly instead of half-parsing.
pub const CASES_SCHEMA: &str = "ds-odrl-engine-rs/compliance-cases@1";

/// One case exactly as the artifact carries it. `request` is kept as a
/// `RawValue` — the literal bytes handed to `engine.wasm`, never
/// re-serialized (see this module's header).
#[derive(Debug, Deserialize)]
pub struct CaseFixture {
  pub slug: String,
  pub title: String,
  #[serde(default)]
  pub request: Option<Box<RawValue>>,
  #[serde(default)]
  pub expected_decision: Option<String>,
  #[serde(default)]
  pub skip_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CaseFile {
  pub schema: String,
  pub suite: String,
  pub cases: Vec<CaseFixture>,
}

/// Mirrors `compliance-runner/src/report.rs`'s `JsonCase` field for
/// field. Moved here from `compliance_page.rs` (where it was
/// `ComplianceCase`) and renamed, because this file is now the *native
/// baseline* rather than the page's own data — and because a type this
/// module compares against ought to be unit-testable.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BaselineCase {
  pub slug: String,
  pub title: String,
  pub status: String,
  pub decision: Option<String>,
  pub expected: Option<String>,
  pub actual: Option<String>,
  pub reason: Option<String>,
}

/// Mirrors `compliance-runner/src/report.rs`'s `JsonReport`. The two id
/// lists are redundant with `cases[].status` and aren't read here, but
/// are kept as fields — rather than dropped from the struct — so a shape
/// mismatch against the real file still shows up as a `serde` error
/// instead of being silently ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BaselineReport {
  pub total: u64,
  pub passed: u64,
  pub failed: u64,
  pub skipped: u64,
  pub failing_case_ids: Vec<String>,
  pub skipped_case_ids: Vec<String>,
  pub cases: Vec<BaselineCase>,
}

/// What this browser's own run made of one case.
///
/// `Errored` is not a synonym for `Failed`: a case this page could not
/// judge (a malformed response, a decision string outside the three the
/// contract defines, an ABI call that itself failed) must not be quietly
/// counted as a pass *or* as a policy-level disagreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseStatus {
  Passed,
  Failed,
  Skipped,
  Errored,
}

impl CaseStatus {
  /// The lower-case spelling `latest.json` uses for the three statuses
  /// the native runner also has — so a live outcome and a baseline case
  /// can be compared without either side guessing at the other's
  /// vocabulary. `Errored` has no native counterpart (the native runner
  /// exits non-zero instead), and says so.
  pub fn as_baseline_str(self) -> &'static str {
    match self {
      CaseStatus::Passed => "passed",
      CaseStatus::Failed => "failed",
      CaseStatus::Skipped => "skipped",
      CaseStatus::Errored => "errored",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseOutcome {
  pub slug: String,
  pub title: String,
  pub status: CaseStatus,
  /// The vendored suite's expected decision, carried through from the
  /// artifact (absent for a skipped case).
  pub expected: Option<String>,
  /// What `engine.wasm` answered in this browser, just now.
  pub actual: Option<String>,
  /// The engine's own `reason` for an evaluated case (passed *or*
  /// failed — richer than `latest.json`, which only records one for a
  /// failure), the artifact's cited skip reason for a skipped case, or
  /// the raw error text for an errored one.
  pub reason: Option<String>,
}

/// One case where the live run and the recorded native run disagree
/// about what happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
  pub slug: String,
  pub native: String,
  pub live: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BaselineComparison {
  pub total: u64,
  pub passed: u64,
  pub failed: u64,
  pub skipped: u64,
  pub divergences: Vec<Divergence>,
}

impl BaselineComparison {
  pub fn matches(&self) -> bool {
    self.divergences.is_empty()
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveReport {
  pub outcomes: Vec<CaseOutcome>,
  /// The corpus artifact's own `suite` line, carried through so the
  /// finished page can name exactly which vendored suite it just ran
  /// rather than asserting one from memory. (`copy-file` assets are not
  /// content-hashed, so a returning visitor can be served a cached
  /// artifact -- what was actually fetched should be visible, not
  /// assumed.)
  pub suite: String,
  pub total: u64,
  pub passed: u64,
  pub failed: u64,
  pub skipped: u64,
  pub errored: u64,
  /// Wall-clock milliseconds the evaluation loop actually took in this
  /// browser — the honest answer to "did work really happen here".
  /// Measured, never padded; it includes the handful of one-frame yields
  /// the loop makes so the progress count can paint (see
  /// `compliance_run::yield_for_paint`), because that is genuinely how
  /// long the visitor waited.
  pub elapsed_ms: f64,
  /// Byte length of the `engine.wasm` that was driven.
  pub engine_bytes: usize,
  /// `None` when `latest.json` could not be fetched or parsed: the live
  /// run is the authority and is never blocked by the file it is merely
  /// being cross-checked against.
  pub baseline: Option<BaselineComparison>,
}

/// Section 5.2's response envelope, as much of it as this page reads.
/// Extra fields (`dataset_id`, `duties`) are tolerated rather than
/// required, so an engine that grows its response shape doesn't turn
/// every case red here; a *missing* `decision`/`reason`, or a truncated
/// body, still fails to parse and becomes an `Errored` case.
#[derive(Debug, Deserialize)]
struct EngineResponse {
  decision: String,
  reason: String,
}

/// The three decision strings Section 5.2 defines. Anything else is not
/// a decision this page knows how to judge.
fn is_known_decision(decision: &str) -> bool {
  matches!(decision, "Allow" | "Deny" | "Error")
}

/// Compares one expected decision against one the engine actually
/// returned. An unrecognized decision on either side is [`CaseStatus::Errored`],
/// never a silent pass and never an ordinary policy disagreement.
pub fn classify(expected: &str, actual: &str) -> CaseStatus {
  if !is_known_decision(expected) || !is_known_decision(actual) {
    CaseStatus::Errored
  } else if expected == actual {
    CaseStatus::Passed
  } else {
    CaseStatus::Failed
  }
}

/// The raw request bytes to hand `engine.wasm`, or `None` when this case
/// isn't one to evaluate (a skip, or a malformed artifact entry — see
/// [`non_evaluated_outcome`]).
pub fn request_json(fixture: &CaseFixture) -> Option<&str> {
  fixture.request.as_ref().map(|raw| raw.get())
}

/// Turns one `engine.wasm` response into an outcome. Never panics: a
/// response that isn't the documented envelope, or a decision outside
/// `Allow`/`Deny`/`Error`, is an `Errored` case carrying the raw text in
/// `reason` — a case this page could not judge is not quietly a pass.
pub fn evaluated_outcome(fixture: &CaseFixture, response_json: &str) -> CaseOutcome {
  let Some(expected) = fixture.expected_decision.clone() else {
    return errored_outcome(fixture, "artifact case carries a `request` but no `expected_decision`");
  };

  let response: EngineResponse = match serde_json::from_str(response_json) {
    Ok(response) => response,
    Err(err) => {
      return CaseOutcome {
        slug: fixture.slug.clone(),
        title: fixture.title.clone(),
        status: CaseStatus::Errored,
        expected: Some(expected),
        actual: None,
        reason: Some(format!("engine.wasm response did not match Section 5.2's envelope ({err}): {response_json}")),
      };
    }
  };

  CaseOutcome {
    slug: fixture.slug.clone(),
    title: fixture.title.clone(),
    status: classify(&expected, &response.decision),
    expected: Some(expected),
    actual: Some(response.decision),
    reason: Some(response.reason),
  }
}

/// A case the native runner declined to translate, or (defensively) an
/// artifact entry carrying neither a `request` nor a `skip_reason` —
/// which is a malformed artifact, not a skip, and says so.
pub fn non_evaluated_outcome(fixture: &CaseFixture) -> CaseOutcome {
  match &fixture.skip_reason {
    Some(reason) => CaseOutcome {
      slug: fixture.slug.clone(),
      title: fixture.title.clone(),
      status: CaseStatus::Skipped,
      expected: None,
      actual: None,
      reason: Some(reason.clone()),
    },
    None => errored_outcome(fixture, "artifact case carries neither a `request` nor a `skip_reason`"),
  }
}

/// A case whose `evaluate()` call itself failed at the ABI boundary.
pub fn errored_outcome(fixture: &CaseFixture, message: &str) -> CaseOutcome {
  CaseOutcome {
    slug: fixture.slug.clone(),
    title: fixture.title.clone(),
    status: CaseStatus::Errored,
    expected: fixture.expected_decision.clone(),
    actual: None,
    reason: Some(message.to_string()),
  }
}

/// Parses and validates the fetched corpus. A wrong `schema`, or an empty
/// `cases` array, is an error rather than a zero-case "success": a run
/// that evaluated nothing must not be able to report 0 failures.
pub fn parse_case_file(text: &str) -> Result<CaseFile, String> {
  let file: CaseFile =
    serde_json::from_str(text).map_err(|err| format!("{CASES_URL} did not match the expected shape: {err}"))?;

  if file.schema != CASES_SCHEMA {
    return Err(format!("{CASES_URL} declares schema `{}`, this page speaks `{CASES_SCHEMA}`", file.schema));
  }
  if file.cases.is_empty() {
    return Err(format!("{CASES_URL} carries no cases at all"));
  }
  Ok(file)
}

pub fn parse_baseline(text: &str) -> Result<BaselineReport, String> {
  serde_json::from_str(text).map_err(|err| format!("{BASELINE_URL} did not match the expected shape: {err}"))
}

/// Compares this browser's own per-case verdicts against the native run's
/// recorded ones. Same corpus, same engine source — one run through
/// `engine::evaluate_request` natively, one through the compiled
/// `engine.wasm` ABI here — so any divergence is a real cross-host
/// finding (or the signal that someone regenerated one artifact and not
/// the other).
pub fn compare_to_baseline(outcomes: &[CaseOutcome], baseline: &BaselineReport) -> BaselineComparison {
  let mut divergences = Vec::new();

  for outcome in outcomes {
    match baseline.cases.iter().find(|case| case.slug == outcome.slug) {
      Some(case) if case.status == outcome.status.as_baseline_str() => {}
      Some(case) => divergences.push(Divergence {
        slug: outcome.slug.clone(),
        native: case.status.clone(),
        live: outcome.status.as_baseline_str().to_string(),
      }),
      None => divergences.push(Divergence {
        slug: outcome.slug.clone(),
        native: "absent".to_string(),
        live: outcome.status.as_baseline_str().to_string(),
      }),
    }
  }

  for case in &baseline.cases {
    if !outcomes.iter().any(|outcome| outcome.slug == case.slug) {
      divergences.push(Divergence { slug: case.slug.clone(), native: case.status.clone(), live: "absent".to_string() });
    }
  }

  BaselineComparison {
    total: baseline.total,
    passed: baseline.passed,
    failed: baseline.failed,
    skipped: baseline.skipped,
    divergences,
  }
}

/// The last stage's actual work: tally the live outcomes (preserving the
/// artifact's own case order) and cross-check them against the native
/// run. Infallible by construction — a pure function over owned data, so
/// the `Compiling` stage has no failure path to strand the UI on.
pub fn compile_report(
  outcomes: Vec<CaseOutcome>,
  suite: String,
  elapsed_ms: f64,
  engine_bytes: usize,
  baseline: Option<&BaselineReport>,
) -> LiveReport {
  let count = |status: CaseStatus| outcomes.iter().filter(|o| o.status == status).count() as u64;
  let (passed, failed, skipped, errored) =
    (count(CaseStatus::Passed), count(CaseStatus::Failed), count(CaseStatus::Skipped), count(CaseStatus::Errored));
  let baseline = baseline.map(|report| compare_to_baseline(&outcomes, report));

  LiveReport {
    suite,
    total: outcomes.len() as u64,
    passed,
    failed,
    skipped,
    errored,
    outcomes,
    elapsed_ms,
    engine_bytes,
    baseline,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The real, committed corpus this page fetches at runtime — embedded
  /// here **only** for these tests (`#[cfg(test)]`), never in the wasm
  /// build, which fetches it instead.
  const LATEST_CASES_JSON: &str = include_str!("../../compliance/reports/latest-cases.json");
  const LATEST_JSON: &str = include_str!("../../compliance/reports/latest.json");

  fn fixture(expected: Option<&str>, skip_reason: Option<&str>) -> CaseFixture {
    CaseFixture {
      slug: "testcase-001-alice".to_string(),
      title: "Any request results into yes (Alice Request).".to_string(),
      request: expected.map(|_| RawValue::from_string("{\"action\":\"read\"}".to_string()).unwrap()),
      expected_decision: expected.map(str::to_string),
      skip_reason: skip_reason.map(str::to_string),
    }
  }

  fn outcome(slug: &str, status: CaseStatus) -> CaseOutcome {
    CaseOutcome { slug: slug.to_string(), title: String::new(), status, expected: None, actual: None, reason: None }
  }

  #[test]
  fn classify_covers_all_nine_expected_actual_pairs() {
    for decision in ["Allow", "Deny", "Error"] {
      assert_eq!(classify(decision, decision), CaseStatus::Passed, "{decision} vs itself");
      for other in ["Allow", "Deny", "Error"] {
        if other != decision {
          assert_eq!(classify(decision, other), CaseStatus::Failed, "{decision} vs {other}");
        }
      }
    }
  }

  #[test]
  fn an_unrecognized_decision_is_errored_not_failed_and_never_passed() {
    assert_eq!(classify("Allow", "allow"), CaseStatus::Errored);
    assert_eq!(classify("Allow", "Maybe"), CaseStatus::Errored);
    assert_eq!(classify("Allow", ""), CaseStatus::Errored);
    // Even an exact string match is Errored when it isn't a decision the
    // contract defines -- otherwise a garbled artifact could agree with a
    // garbled response and score a pass.
    assert_eq!(classify("Maybe", "Maybe"), CaseStatus::Errored);
  }

  #[test]
  fn evaluated_outcome_reads_a_real_engine_response_envelope() {
    let response = r#"{"dataset_id":"urn:uuid:e21","decision":"Allow","reason":"permission[0] of policy 'p' matched: action 'read'","duties":[]}"#;
    let result = evaluated_outcome(&fixture(Some("Allow"), None), response);

    assert_eq!(result.status, CaseStatus::Passed);
    assert_eq!(result.expected.as_deref(), Some("Allow"));
    assert_eq!(result.actual.as_deref(), Some("Allow"));
    // Richer than latest.json, which records a reason only for failures.
    assert_eq!(result.reason.as_deref(), Some("permission[0] of policy 'p' matched: action 'read'"));
  }

  #[test]
  fn a_disagreement_with_the_suites_expected_decision_is_a_failure_carrying_both_sides() {
    let response = r#"{"dataset_id":"d","decision":"Deny","reason":"no permission matched","duties":[]}"#;
    let result = evaluated_outcome(&fixture(Some("Allow"), None), response);

    assert_eq!(result.status, CaseStatus::Failed);
    assert_eq!(result.expected.as_deref(), Some("Allow"));
    assert_eq!(result.actual.as_deref(), Some("Deny"));
    assert_eq!(result.reason.as_deref(), Some("no permission matched"));
  }

  #[test]
  fn a_malformed_or_truncated_response_is_errored_and_does_not_panic() {
    for body in ["", "not json at all", r#"{"decision":"Allow""#, r#"{"reason":"no decision key"}"#, "null", "[]"] {
      let result = evaluated_outcome(&fixture(Some("Allow"), None), body);
      assert_eq!(result.status, CaseStatus::Errored, "body {body:?}");
      assert_eq!(result.actual, None, "body {body:?}");
      assert!(result.reason.unwrap().contains("Section 5.2's envelope"), "body {body:?}");
    }
  }

  #[test]
  fn an_out_of_contract_decision_string_is_errored_with_the_raw_value_kept() {
    let result = evaluated_outcome(&fixture(Some("Allow"), None), r#"{"decision":"Permit","reason":"x"}"#);
    assert_eq!(result.status, CaseStatus::Errored);
    assert_eq!(result.actual.as_deref(), Some("Permit"));
  }

  #[test]
  fn a_case_with_a_request_but_no_expected_decision_is_errored() {
    let mut fixture = fixture(Some("Allow"), None);
    fixture.expected_decision = None;
    let result = evaluated_outcome(&fixture, r#"{"decision":"Allow","reason":"ok"}"#);

    assert_eq!(result.status, CaseStatus::Errored);
    assert!(result.reason.unwrap().contains("no `expected_decision`"));
  }

  #[test]
  fn a_skipped_case_carries_the_artifacts_cited_reason_through_unchanged() {
    let result = non_evaluated_outcome(&fixture(None, Some("odrl:xone is not expressible (translate.rs::xone_unsupported)")));

    assert_eq!(result.status, CaseStatus::Skipped);
    assert_eq!(result.reason.as_deref(), Some("odrl:xone is not expressible (translate.rs::xone_unsupported)"));
    assert_eq!(result.expected, None);
  }

  #[test]
  fn a_case_with_neither_request_nor_skip_reason_is_errored_not_skipped() {
    let result = non_evaluated_outcome(&fixture(None, None));
    assert_eq!(result.status, CaseStatus::Errored);
    assert!(result.reason.unwrap().contains("neither"));
  }

  #[test]
  fn an_abi_level_failure_becomes_an_errored_case_keeping_the_expected_decision() {
    let result = errored_outcome(&fixture(Some("Deny"), None), "evaluate() did not return a BigInt (i64)");
    assert_eq!(result.status, CaseStatus::Errored);
    assert_eq!(result.expected.as_deref(), Some("Deny"));
    assert_eq!(result.reason.as_deref(), Some("evaluate() did not return a BigInt (i64)"));
  }

  #[test]
  fn compile_report_tallies_every_status_and_preserves_case_order() {
    let outcomes = vec![
      outcome("a", CaseStatus::Passed),
      outcome("b", CaseStatus::Failed),
      outcome("c", CaseStatus::Skipped),
      outcome("d", CaseStatus::Errored),
      outcome("e", CaseStatus::Passed),
    ];
    let report = compile_report(outcomes, "a suite".to_string(), 41.5, 232_881, None);

    assert_eq!((report.total, report.passed, report.failed, report.skipped, report.errored), (5, 2, 1, 1, 1));
    assert_eq!(report.elapsed_ms, 41.5);
    assert_eq!(report.suite, "a suite");
    assert_eq!(report.engine_bytes, 232_881);
    assert_eq!(report.baseline, None);
    assert_eq!(report.outcomes.iter().map(|o| o.slug.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "e"]);
  }

  fn baseline_case(slug: &str, status: &str) -> BaselineCase {
    BaselineCase {
      slug: slug.to_string(),
      title: String::new(),
      status: status.to_string(),
      decision: None,
      expected: None,
      actual: None,
      reason: None,
    }
  }

  fn baseline_of(cases: Vec<BaselineCase>) -> BaselineReport {
    let passed = cases.iter().filter(|c| c.status == "passed").count() as u64;
    BaselineReport {
      total: cases.len() as u64,
      passed,
      failed: cases.iter().filter(|c| c.status == "failed").count() as u64,
      skipped: cases.iter().filter(|c| c.status == "skipped").count() as u64,
      failing_case_ids: vec![],
      skipped_case_ids: vec![],
      cases,
    }
  }

  #[test]
  fn an_agreeing_baseline_reports_no_divergence() {
    let baseline = baseline_of(vec![baseline_case("a", "passed"), baseline_case("b", "skipped")]);
    let outcomes = vec![outcome("a", CaseStatus::Passed), outcome("b", CaseStatus::Skipped)];
    let report = compile_report(outcomes, "a suite".to_string(), 1.0, 1, Some(&baseline));

    let comparison = report.baseline.expect("a baseline was supplied");
    assert!(comparison.matches());
    assert_eq!(comparison.total, 2);
  }

  #[test]
  fn a_per_case_disagreement_with_the_native_run_is_reported_by_slug() {
    let baseline = baseline_of(vec![baseline_case("a", "passed"), baseline_case("b", "passed")]);
    let outcomes = vec![outcome("a", CaseStatus::Passed), outcome("b", CaseStatus::Failed)];
    let comparison = compare_to_baseline(&outcomes, &baseline);

    assert!(!comparison.matches());
    assert_eq!(comparison.divergences, vec![Divergence {
      slug: "b".to_string(),
      native: "passed".to_string(),
      live: "failed".to_string()
    }]);
  }

  #[test]
  fn a_case_present_in_only_one_of_the_two_artifacts_is_a_divergence_in_both_directions() {
    let baseline = baseline_of(vec![baseline_case("a", "passed"), baseline_case("only-native", "passed")]);
    let outcomes = vec![outcome("a", CaseStatus::Passed), outcome("only-live", CaseStatus::Passed)];
    let comparison = compare_to_baseline(&outcomes, &baseline);

    assert_eq!(comparison.divergences, vec![
      Divergence { slug: "only-live".to_string(), native: "absent".to_string(), live: "passed".to_string() },
      Divergence { slug: "only-native".to_string(), native: "passed".to_string(), live: "absent".to_string() },
    ]);
  }

  #[test]
  fn an_errored_case_diverges_from_a_native_pass_rather_than_matching_it() {
    let baseline = baseline_of(vec![baseline_case("a", "passed")]);
    let comparison = compare_to_baseline(&[outcome("a", CaseStatus::Errored)], &baseline);
    assert_eq!(comparison.divergences.len(), 1);
    assert_eq!(comparison.divergences[0].live, "errored");
  }

  #[test]
  fn the_committed_corpus_parses_and_declares_this_pages_schema() {
    let file = parse_case_file(LATEST_CASES_JSON).expect("the committed artifact parses");

    assert_eq!(file.schema, CASES_SCHEMA);
    assert!(file.suite.contains("ODRL-Test-Suite"));
    assert_eq!(file.cases.len(), 68, "the vendored corpus currently indexes 68 cases");

    for case in &file.cases {
      assert!(!case.slug.is_empty(), "every case has a slug");

      match (&case.request, &case.expected_decision, &case.skip_reason) {
        (Some(request), Some(expected), None) => {
          assert!(is_known_decision(expected), "{}: expected_decision `{expected}`", case.slug);
          let raw = request.get();
          for key in ["dataset_id", "action", "config", "policies", "claims"] {
            assert!(raw.contains(key), "{}: request is missing `{key}`", case.slug);
          }
        }
        (None, None, Some(reason)) => assert!(!reason.is_empty(), "{}: empty skip_reason", case.slug),
        other => panic!("{}: a case must be exactly evaluable or skipped, got {other:?}", case.slug),
      }
    }
  }

  /// The compile-time guard against regenerating one artifact and not the
  /// other: `compliance-runner` writes both from one run, in one order,
  /// so a slug-order mismatch means the two committed files came from
  /// different runs — which the live page would surface as a baseline
  /// divergence to a *visitor*, long after it could have been caught here.
  #[test]
  fn the_committed_corpus_and_the_native_baseline_list_the_same_slugs_in_the_same_order() {
    let cases = parse_case_file(LATEST_CASES_JSON).expect("the committed artifact parses");
    let baseline = parse_baseline(LATEST_JSON).expect("the committed baseline parses");

    let case_slugs: Vec<&str> = cases.cases.iter().map(|c| c.slug.as_str()).collect();
    let baseline_slugs: Vec<&str> = baseline.cases.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(case_slugs, baseline_slugs);
    assert_eq!(baseline.total as usize, case_slugs.len());
  }

  #[test]
  fn a_corpus_with_the_wrong_schema_or_no_cases_is_rejected_rather_than_half_parsed() {
    let wrong_schema = r#"{"schema":"something-else@9","suite":"s","cases":[{"slug":"a","title":"t"}]}"#;
    assert!(parse_case_file(wrong_schema).unwrap_err().contains("declares schema"));

    let no_cases = format!(r#"{{"schema":"{CASES_SCHEMA}","suite":"s","cases":[]}}"#);
    assert!(parse_case_file(&no_cases).unwrap_err().contains("no cases"));

    let truncated = &LATEST_CASES_JSON[..LATEST_CASES_JSON.len() / 2];
    assert!(parse_case_file(truncated).unwrap_err().contains("did not match the expected shape"));
  }
}
