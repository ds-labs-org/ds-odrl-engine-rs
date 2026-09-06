//! Pure, browser-free logic for the live ODRL 2.2 coverage run: parsing
//! the probe catalog (`compliance/reports/latest-coverage.json`, see
//! `coverage-probes/src/catalog.rs`), judging one live `engine.wasm`
//! response against the outcome a row's documented status predicts, and
//! compiling the finished report.
//!
//! Deliberately **not** `#[cfg(target_arch = "wasm32")]`-gated, for the
//! same reason `compliance_cases.rs` isn't: `cargo test --workspace` is a
//! native build, so a gated module's unit tests would silently never
//! compile, let alone run — and this is exactly the part that must be
//! testable with no browser and no wasm instance. Only this module's
//! callers (`coverage_run.rs`, `coverage_page.rs`) touch the DOM or the
//! ABI; nothing here does.
//!
//! **`ProbeFixture::request` is a `serde_json::value::RawValue`, and that
//! is not a detail.** Most of this catalog's negative probes assert that
//! some real ODRL property is *inert* — `conflict`, a per-permission
//! `duty`, a per-rule `target`, `odrl:andSequence`, `odrl:implies`. Every
//! one of those is, by construction, a JSON key `crate::wire`'s types do
//! not model. Deserializing a probe's request into those types and
//! re-serializing it would silently drop exactly the key the probe exists
//! to inject, turning every such probe into a vacuous no-op that still
//! reports "verified live". The bytes go to `engine.wasm` verbatim.

use serde::Deserialize;
use serde_json::value::RawValue;

/// The catalog, copied to `dist/compliance-data/` by `index.html`'s own
/// `copy-file` directive (same target directory, and same
/// route-collision reason, as the two compliance artifacts beside it).
pub const COVERAGE_URL: &str = "compliance-data/latest-coverage.json";

/// Must equal `coverage-probes/src/render.rs`'s own `SCHEMA`. Checked
/// rather than assumed: `copy-file` assets are not content-hashed, so a
/// returning visitor can be served a browser-cached artifact of an older
/// shape, and that must fail loudly instead of half-parsing.
pub const COVERAGE_SCHEMA: &str = "ds-odrl-engine-rs/odrl-coverage@1";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Category {
  pub id: String,
  pub number: u32,
  pub title: String,
  pub spec_ref: String,
}

/// One vocabulary claim from the source gap analysis.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogRow {
  pub id: String,
  pub category: String,
  pub term: String,
  pub status: String,
  pub why: String,
  pub evidence: String,
  pub asserts: String,
  pub probe_ids: Vec<String>,
  pub documented_because: Option<String>,
  pub caveat: Option<String>,
}

impl CatalogRow {
  pub fn is_documented_only(&self) -> bool {
    self.probe_ids.is_empty()
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DutyExpect {
  pub policy_id: String,
  pub action: String,
  pub resolved: bool,
}

/// What the browser must observe for one probe to agree with its row.
/// `duties`/`dataset_id` are `None` on most probes — "not asserted",
/// which is a different thing from `Some(vec![])`, "asserted to be
/// empty".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Expectation {
  pub decision: String,
  pub reason_contains: Vec<String>,
  pub reason_excludes: Vec<String>,
  pub duties: Option<Vec<DutyExpect>>,
  pub dataset_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProbeFixture {
  pub id: String,
  pub kind: String,
  pub title: String,
  pub asserts: String,
  pub falsified_by: String,
  /// Kept as raw bytes — see this module's header. Never round-tripped.
  pub request: Box<RawValue>,
  pub expect: Expectation,
}

#[derive(Debug, Deserialize)]
pub struct CoverageFile {
  pub schema: String,
  pub generated_by: String,
  pub spec: String,
  pub source_analysis: String,
  pub note: String,
  pub categories: Vec<Category>,
  pub rows: Vec<CatalogRow>,
  pub probes: Vec<ProbeFixture>,
}

/// Section 5.2's response envelope, as much of it as this page judges.
#[derive(Debug, Deserialize)]
pub struct EngineResponse {
  pub decision: String,
  pub reason: String,
  #[serde(default)]
  pub duties: Vec<DutyExpect>,
  #[serde(default)]
  pub dataset_id: String,
}

/// What this browser made of one probe.
///
/// `Errored` is not a synonym for `Disagreed`: a probe this page could not
/// judge (a malformed response, a decision outside the three the contract
/// defines, an ABI call that itself failed) must not be counted as
/// agreement *or* as a contradiction of the documented status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
  Agreed,
  Disagreed,
  Errored,
}

/// What this run made of one row's documented status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowVerdict {
  /// Every probe agreed: the documented status held, live.
  Verified,
  /// At least one probe disagreed — the loud one.
  Contradicted,
  /// No disagreement, but at least one probe could not be judged.
  Inconclusive,
  /// The row has no probes: its claim is not about the wire contract.
  Documented,
}

impl RowVerdict {
  pub fn label(self) -> &'static str {
    match self {
      RowVerdict::Verified => "Verified live",
      RowVerdict::Contradicted => "Contradicted",
      RowVerdict::Inconclusive => "Inconclusive",
      RowVerdict::Documented => "Documented claim",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeOutcome {
  pub id: String,
  pub title: String,
  pub kind: String,
  pub asserts: String,
  pub falsified_by: String,
  pub expected_decision: String,
  pub status: ProbeStatus,
  /// What `engine.wasm` answered in this browser, just now.
  pub decision: Option<String>,
  /// The engine's own `reason` — for an agreeing probe as much as a
  /// disagreeing one, which is the thing a hand-written coverage table
  /// cannot show you.
  pub reason: Option<String>,
  /// Which clause of the expectation failed, when one did.
  pub mismatch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowOutcome {
  pub row: CatalogRow,
  pub verdict: RowVerdict,
  /// This row's probes, in the order the row names them.
  pub probes: Vec<ProbeOutcome>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageReport {
  pub generated_by: String,
  pub spec: String,
  pub source_analysis: String,
  /// The artifact's own statement that it records no decision -- rendered
  /// on the page in the artifact's words rather than paraphrased, so the
  /// claim a visitor reads is the one the generator actually makes.
  pub note: String,
  pub categories: Vec<Category>,
  pub rows: Vec<RowOutcome>,
  pub total_probes: u64,
  pub agreed: u64,
  pub disagreed: u64,
  pub errored: u64,
  pub verified: u64,
  pub contradicted: u64,
  pub inconclusive: u64,
  pub documented: u64,
  pub implemented: u64,
  pub partial: u64,
  pub not_implemented: u64,
  pub out_of_scope: u64,
  /// Wall-clock milliseconds the probe loop actually took in this
  /// browser. Measured, never padded.
  pub elapsed_ms: f64,
  /// Byte length of the `engine.wasm` that was driven.
  pub engine_bytes: usize,
}

impl CoverageReport {
  pub fn contradicted_rows(&self) -> Vec<&RowOutcome> {
    self.rows.iter().filter(|row| row.verdict == RowVerdict::Contradicted).collect()
  }
}

/// The four documented statuses, spelled for a reader.
pub fn status_display(status: &str) -> &'static str {
  match status {
    "Implemented" => "Implemented",
    "Partial" => "Partial",
    "NotImplemented" => "Not implemented",
    "OutOfScope" => "Out of scope",
    _ => "Unknown status",
  }
}

/// The three decision strings Section 5.2 defines.
fn is_known_decision(decision: &str) -> bool {
  matches!(decision, "Allow" | "Deny" | "Error")
}

/// The raw request bytes to hand `engine.wasm`, verbatim.
pub fn probe_json(probe: &ProbeFixture) -> &str {
  probe.request.get()
}

fn render_duties(duties: &[DutyExpect]) -> String {
  if duties.is_empty() {
    return "[]".to_string();
  }
  duties.iter().map(|d| format!("{}/{} resolved={}", d.policy_id, d.action, d.resolved)).collect::<Vec<_>>().join(", ")
}

/// Compares one expectation against one response the engine actually
/// returned, naming the clause that failed rather than only that
/// something did.
///
/// An out-of-contract decision string is [`ProbeStatus::Errored`], never a
/// silent agreement and never an ordinary contradiction: a probe this page
/// could not judge must not be able to turn a row red *or* green.
pub fn classify_probe(expect: &Expectation, response: &EngineResponse) -> (ProbeStatus, Option<String>) {
  if !is_known_decision(&response.decision) {
    return (
      ProbeStatus::Errored,
      Some(format!("engine.wasm answered `{}`, which is not one of Allow/Deny/Error", response.decision)),
    );
  }
  if response.decision != expect.decision {
    return (
      ProbeStatus::Disagreed,
      Some(format!("expected {}, observed {}", expect.decision, response.decision)),
    );
  }
  for needle in &expect.reason_contains {
    if !response.reason.contains(needle.as_str()) {
      return (ProbeStatus::Disagreed, Some(format!("reason missing `{needle}`")));
    }
  }
  for needle in &expect.reason_excludes {
    if response.reason.contains(needle.as_str()) {
      return (ProbeStatus::Disagreed, Some(format!("reason contained excluded `{needle}`")));
    }
  }
  if let Some(expected) = &expect.duties {
    if &response.duties != expected {
      return (
        ProbeStatus::Disagreed,
        Some(format!(
          "duties differed: expected [{}], observed [{}]",
          render_duties(expected),
          render_duties(&response.duties)
        )),
      );
    }
  }
  if let Some(expected) = &expect.dataset_id {
    if &response.dataset_id != expected {
      return (
        ProbeStatus::Disagreed,
        Some(format!("dataset_id expected `{expected}`, observed `{}`", response.dataset_id)),
      );
    }
  }
  (ProbeStatus::Agreed, None)
}

fn outcome_shell(probe: &ProbeFixture, status: ProbeStatus) -> ProbeOutcome {
  ProbeOutcome {
    id: probe.id.clone(),
    title: probe.title.clone(),
    kind: probe.kind.clone(),
    asserts: probe.asserts.clone(),
    falsified_by: probe.falsified_by.clone(),
    expected_decision: probe.expect.decision.clone(),
    status,
    decision: None,
    reason: None,
    mismatch: None,
  }
}

/// Turns one `engine.wasm` response into an outcome. Never panics: a
/// response that isn't the documented envelope is an `Errored` probe
/// carrying the raw text, not a contradiction of anything.
pub fn evaluated_probe_outcome(probe: &ProbeFixture, response_json: &str) -> ProbeOutcome {
  let response: EngineResponse = match serde_json::from_str(response_json) {
    Ok(response) => response,
    Err(err) => {
      let mut outcome = outcome_shell(probe, ProbeStatus::Errored);
      outcome.reason =
        Some(format!("engine.wasm response did not match Section 5.2's envelope ({err}): {response_json}"));
      return outcome;
    }
  };

  let (status, mismatch) = classify_probe(&probe.expect, &response);
  let mut outcome = outcome_shell(probe, status);
  outcome.decision = Some(response.decision);
  outcome.reason = Some(response.reason);
  outcome.mismatch = mismatch;
  outcome
}

/// A probe whose `evaluate()` call itself failed at the ABI boundary.
pub fn errored_probe_outcome(probe: &ProbeFixture, message: &str) -> ProbeOutcome {
  let mut outcome = outcome_shell(probe, ProbeStatus::Errored);
  outcome.reason = Some(message.to_string());
  outcome
}

/// One row's verdict over its own probes.
///
/// `Contradicted` deliberately out-ranks `Inconclusive`: a row with one
/// disagreement and one unjudgeable probe has a real, actionable finding
/// in it, and burying that under "inconclusive" would be the quiet
/// direction to fail in.
pub fn derive_verdict(row: &CatalogRow, outcomes: &[&ProbeOutcome]) -> RowVerdict {
  if row.is_documented_only() {
    return RowVerdict::Documented;
  }
  // A non-documented row with zero outcomes is not currently reachable
  // from a live run (`parse_coverage_catalog` rejects a row naming an
  // unknown probe, and `coverage_run::run` produces exactly one outcome
  // per catalog probe) -- but an adversarial review found the *test*
  // helper that builds a report straight from a bare `Vec<ProbeOutcome>`
  // could construct exactly this shape, and without this guard it fell
  // through to `Verified`: the one silent-green result this module is
  // otherwise careful never to produce. Guarded explicitly rather than
  // relying on that upstream invariant to hold everywhere forever.
  if outcomes.is_empty() {
    return RowVerdict::Inconclusive;
  }
  if outcomes.iter().any(|o| o.status == ProbeStatus::Disagreed) {
    return RowVerdict::Contradicted;
  }
  if outcomes.iter().any(|o| o.status == ProbeStatus::Errored) {
    return RowVerdict::Inconclusive;
  }
  RowVerdict::Verified
}

/// Parses and validates the fetched catalog. Every rejection below is an
/// error rather than a degraded run: a catalog that half-parsed would let
/// this page report "0 contradicted" over rows it never actually probed.
pub fn parse_coverage_catalog(text: &str) -> Result<CoverageFile, String> {
  let file: CoverageFile =
    serde_json::from_str(text).map_err(|err| format!("{COVERAGE_URL} did not match the expected shape: {err}"))?;

  if file.schema != COVERAGE_SCHEMA {
    return Err(format!("{COVERAGE_URL} declares schema `{}`, this page speaks `{COVERAGE_SCHEMA}`", file.schema));
  }
  if file.probes.is_empty() {
    return Err(format!("{COVERAGE_URL} carries no probes at all"));
  }
  if file.rows.is_empty() {
    return Err(format!("{COVERAGE_URL} carries no vocabulary rows at all"));
  }

  for row in &file.rows {
    match (row.probe_ids.is_empty(), row.documented_because.is_some()) {
      (true, false) => {
        return Err(format!("{COVERAGE_URL}: row `{}` has neither probes nor a documented_because", row.id))
      }
      (false, true) => {
        return Err(format!("{COVERAGE_URL}: row `{}` has both probes and a documented_because", row.id))
      }
      _ => {}
    }
    for probe_id in &row.probe_ids {
      if !file.probes.iter().any(|probe| &probe.id == probe_id) {
        return Err(format!("{COVERAGE_URL}: row `{}` references unknown probe `{probe_id}`", row.id));
      }
    }
  }

  Ok(file)
}

/// The last stage's actual work: attach every probe outcome to the rows
/// that name it, derive 52 row verdicts, and tally both axes. Infallible
/// by construction — a pure function over owned data, so the `Compiling`
/// stage has no failure path to strand the UI on.
pub fn compile_coverage_report(
  catalog: &CoverageFile,
  outcomes: Vec<ProbeOutcome>,
  elapsed_ms: f64,
  engine_bytes: usize,
) -> CoverageReport {
  let find = |id: &str| outcomes.iter().find(|outcome| outcome.id == id);

  let rows: Vec<RowOutcome> = catalog
    .rows
    .iter()
    .map(|row| {
      let probes: Vec<&ProbeOutcome> = row.probe_ids.iter().filter_map(|id| find(id)).collect();
      let verdict = derive_verdict(row, &probes);
      RowOutcome { row: row.clone(), verdict, probes: probes.into_iter().cloned().collect() }
    })
    .collect();

  let count_status = |status: ProbeStatus| outcomes.iter().filter(|o| o.status == status).count() as u64;
  let count_verdict = |verdict: RowVerdict| rows.iter().filter(|r| r.verdict == verdict).count() as u64;
  let count_documented_status =
    |status: &str| rows.iter().filter(|r| r.row.status == status).count() as u64;

  CoverageReport {
    generated_by: catalog.generated_by.clone(),
    spec: catalog.spec.clone(),
    source_analysis: catalog.source_analysis.clone(),
    note: catalog.note.clone(),
    categories: catalog.categories.clone(),
    total_probes: outcomes.len() as u64,
    agreed: count_status(ProbeStatus::Agreed),
    disagreed: count_status(ProbeStatus::Disagreed),
    errored: count_status(ProbeStatus::Errored),
    verified: count_verdict(RowVerdict::Verified),
    contradicted: count_verdict(RowVerdict::Contradicted),
    inconclusive: count_verdict(RowVerdict::Inconclusive),
    documented: count_verdict(RowVerdict::Documented),
    implemented: count_documented_status("Implemented"),
    partial: count_documented_status("Partial"),
    not_implemented: count_documented_status("NotImplemented"),
    out_of_scope: count_documented_status("OutOfScope"),
    rows,
    elapsed_ms,
    engine_bytes,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The real, committed catalog this page fetches at runtime — embedded
  /// here **only** for these tests (`#[cfg(test)]`), never in the wasm
  /// build, which fetches it instead.
  const LATEST_COVERAGE_JSON: &str = include_str!("../../compliance/reports/latest-coverage.json");

  fn response(decision: &str, reason: &str) -> EngineResponse {
    EngineResponse {
      decision: decision.to_string(),
      reason: reason.to_string(),
      duties: vec![],
      dataset_id: "urn:uuid:coverage-probe".to_string(),
    }
  }

  fn expectation(decision: &str) -> Expectation {
    Expectation {
      decision: decision.to_string(),
      reason_contains: vec![],
      reason_excludes: vec![],
      duties: None,
      dataset_id: None,
    }
  }

  fn duty(action: &str) -> DutyExpect {
    DutyExpect { policy_id: "probe".to_string(), action: action.to_string(), resolved: false }
  }

  fn catalog_row(id: &str, probe_ids: &[&str], documented: Option<&str>) -> CatalogRow {
    CatalogRow {
      id: id.to_string(),
      category: "actions".to_string(),
      term: "t".to_string(),
      status: "Partial".to_string(),
      why: "w".to_string(),
      evidence: "e".to_string(),
      asserts: "a".to_string(),
      probe_ids: probe_ids.iter().map(|s| s.to_string()).collect(),
      documented_because: documented.map(str::to_string),
      caveat: None,
    }
  }

  fn probe_outcome(id: &str, status: ProbeStatus) -> ProbeOutcome {
    ProbeOutcome {
      id: id.to_string(),
      title: String::new(),
      kind: "positive".to_string(),
      asserts: String::new(),
      falsified_by: String::new(),
      expected_decision: "Allow".to_string(),
      status,
      decision: None,
      reason: None,
      mismatch: None,
    }
  }

  #[test]
  fn an_exactly_matching_response_agrees() {
    let (status, mismatch) = classify_probe(&expectation("Allow"), &response("Allow", "anything"));
    assert_eq!(status, ProbeStatus::Agreed);
    assert_eq!(mismatch, None);
  }

  #[test]
  fn a_differing_decision_disagrees_and_names_both_sides() {
    let (status, mismatch) = classify_probe(&expectation("Deny"), &response("Allow", "r"));
    assert_eq!(status, ProbeStatus::Disagreed);
    assert_eq!(mismatch.as_deref(), Some("expected Deny, observed Allow"));
  }

  #[test]
  fn a_missing_reason_substring_disagrees_naming_the_substring() {
    let mut expect = expectation("Deny");
    expect.reason_contains = vec!["(closed default)".to_string()];
    let (status, mismatch) = classify_probe(&expect, &response("Deny", "some other reason"));
    assert_eq!(status, ProbeStatus::Disagreed);
    assert_eq!(mismatch.as_deref(), Some("reason missing `(closed default)`"));
  }

  #[test]
  fn an_excluded_reason_substring_disagrees() {
    // The uid rows turn on this arm specifically: the decision and every
    // required substring can be right while the reason still leaks a rule
    // uid it is documented never to carry.
    let mut expect = expectation("Allow");
    expect.reason_excludes = vec!["r-7".to_string()];
    let (status, mismatch) = classify_probe(&expect, &response("Allow", "permission[0] (uid urn:rule:r-7)"));
    assert_eq!(status, ProbeStatus::Disagreed);
    assert_eq!(mismatch.as_deref(), Some("reason contained excluded `r-7`"));
  }

  #[test]
  fn an_asserted_duties_list_is_compared_and_an_unasserted_one_is_not() {
    let mut expect = expectation("Allow");
    let mut with_duty = response("Allow", "r");
    with_duty.duties = vec![duty("notify")];

    // Not asserted: a duties list of any shape agrees.
    assert_eq!(classify_probe(&expect, &with_duty).0, ProbeStatus::Agreed);

    // Asserted empty: the same response now disagrees. This distinction
    // is the whole point of the fail-open duty rows -- `null` and `[]`
    // must not mean the same thing.
    expect.duties = Some(vec![]);
    let (status, mismatch) = classify_probe(&expect, &with_duty);
    assert_eq!(status, ProbeStatus::Disagreed);
    assert!(mismatch.unwrap().contains("duties differed"));

    // Asserted to be exactly that duty: agrees again.
    expect.duties = Some(vec![duty("notify")]);
    assert_eq!(classify_probe(&expect, &with_duty).0, ProbeStatus::Agreed);
  }

  #[test]
  fn an_asserted_dataset_id_is_compared() {
    let mut expect = expectation("Deny");
    expect.dataset_id = Some("urn:asset:A".to_string());
    let mut response = response("Deny", "r");
    response.dataset_id = "urn:asset:B".to_string();

    let (status, mismatch) = classify_probe(&expect, &response);
    assert_eq!(status, ProbeStatus::Disagreed);
    assert!(mismatch.unwrap().contains("dataset_id expected `urn:asset:A`"));
  }

  #[test]
  fn an_out_of_contract_decision_is_errored_never_agreed_and_never_disagreed() {
    for decision in ["Permit", "allow", ""] {
      let (status, mismatch) = classify_probe(&expectation("Allow"), &response(decision, "r"));
      assert_eq!(status, ProbeStatus::Errored, "decision {decision:?}");
      assert!(mismatch.unwrap().contains("not one of Allow/Deny/Error"));
    }
    // Even an exact match on an unknown string must not score agreement.
    assert_eq!(classify_probe(&expectation("Maybe"), &response("Maybe", "r")).0, ProbeStatus::Errored);
  }

  #[test]
  fn a_malformed_or_truncated_response_is_errored_and_does_not_panic() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses");
    let probe = &file.probes[0];

    for body in ["", "not json at all", r#"{"decision":"Allow""#, r#"{"reason":"no decision key"}"#, "null", "[]"] {
      let outcome = evaluated_probe_outcome(probe, body);
      assert_eq!(outcome.status, ProbeStatus::Errored, "body {body:?}");
      assert_eq!(outcome.decision, None, "body {body:?}");
      assert!(outcome.reason.unwrap().contains("Section 5.2's envelope"), "body {body:?}");
    }
  }

  #[test]
  fn an_abi_level_failure_becomes_an_errored_probe_keeping_the_expected_decision() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses");
    let outcome = errored_probe_outcome(&file.probes[0], "evaluate() did not return a BigInt (i64)");

    assert_eq!(outcome.status, ProbeStatus::Errored);
    assert_eq!(outcome.expected_decision, file.probes[0].expect.decision);
    assert_eq!(outcome.reason.as_deref(), Some("evaluate() did not return a BigInt (i64)"));
  }

  #[test]
  fn derive_verdict_covers_all_four_verdicts_and_the_contradicted_precedence() {
    let row = catalog_row("r", &["a", "b"], None);
    let agreed = probe_outcome("a", ProbeStatus::Agreed);
    let disagreed = probe_outcome("b", ProbeStatus::Disagreed);
    let errored = probe_outcome("b", ProbeStatus::Errored);

    assert_eq!(derive_verdict(&row, &[&agreed, &agreed]), RowVerdict::Verified);
    assert_eq!(derive_verdict(&row, &[&agreed, &disagreed]), RowVerdict::Contradicted);
    assert_eq!(derive_verdict(&row, &[&agreed, &errored]), RowVerdict::Inconclusive);
    // Contradicted out-ranks Inconclusive: a real finding must not be
    // buried under an unjudgeable probe beside it.
    assert_eq!(derive_verdict(&row, &[&errored, &disagreed]), RowVerdict::Contradicted);

    let documented = catalog_row("d", &[], Some("no request can encode this"));
    assert_eq!(derive_verdict(&documented, &[]), RowVerdict::Documented);
  }

  #[test]
  fn a_non_documented_row_with_zero_outcomes_is_inconclusive_not_silently_verified() {
    // The exact edge case an adversarial review found: not reachable from
    // a live run (every catalog probe gets exactly one outcome), but
    // constructible by a test helper that builds a report straight from
    // an empty outcomes vec -- and without this guard, `any()` over an
    // empty slice is vacuously false for both Disagreed and Errored, so
    // this fell through to `Verified`. That would be the one silent-green
    // result this module otherwise goes out of its way to prevent.
    let row = catalog_row("r", &["a", "b"], None);
    assert_eq!(derive_verdict(&row, &[]), RowVerdict::Inconclusive);
  }

  #[test]
  fn a_catalog_with_the_wrong_schema_or_nothing_in_it_is_rejected() {
    let base = |schema: &str, rows: &str, probes: &str| {
      format!(
        r#"{{"schema":"{schema}","generated_by":"g","spec":"s","source_analysis":"a","note":"n",
            "categories":[],"rows":[{rows}],"probes":[{probes}]}}"#
      )
    };
    let probe = r#"{"id":"p","kind":"positive","title":"t","asserts":"a","falsified_by":"f",
                    "request":{},"expect":{"decision":"Allow","reason_contains":[],"reason_excludes":[],
                    "duties":null,"dataset_id":null}}"#;
    let row = r#"{"id":"r","category":"actions","term":"t","status":"Partial","why":"w","evidence":"e",
                  "asserts":"a","probe_ids":["p"],"documented_because":null,"caveat":null}"#;

    assert!(parse_coverage_catalog(&base("something-else@9", row, probe)).unwrap_err().contains("declares schema"));
    assert!(parse_coverage_catalog(&base(COVERAGE_SCHEMA, row, "")).unwrap_err().contains("no probes"));
    assert!(parse_coverage_catalog(&base(COVERAGE_SCHEMA, "", probe)).unwrap_err().contains("no vocabulary rows"));

    let truncated = &LATEST_COVERAGE_JSON[..LATEST_COVERAGE_JSON.len() / 2];
    assert!(parse_coverage_catalog(truncated).unwrap_err().contains("did not match the expected shape"));
  }

  #[test]
  fn a_row_referencing_an_unknown_probe_is_rejected_rather_than_silently_probing_nothing() {
    let text = r#"{"schema":"ds-odrl-engine-rs/odrl-coverage@1","generated_by":"g","spec":"s",
      "source_analysis":"a","note":"n","categories":[],
      "rows":[{"id":"r","category":"actions","term":"t","status":"Partial","why":"w","evidence":"e",
               "asserts":"a","probe_ids":["nope"],"documented_because":null,"caveat":null}],
      "probes":[{"id":"p","kind":"positive","title":"t","asserts":"a","falsified_by":"f","request":{},
                 "expect":{"decision":"Allow","reason_contains":[],"reason_excludes":[],"duties":null,
                 "dataset_id":null}}]}"#;
    assert!(parse_coverage_catalog(text).unwrap_err().contains("references unknown probe `nope`"));
  }

  #[test]
  fn a_row_that_is_both_probed_and_documented_or_neither_is_rejected() {
    let with_rows = |row: &str| {
      format!(
        r#"{{"schema":"ds-odrl-engine-rs/odrl-coverage@1","generated_by":"g","spec":"s",
           "source_analysis":"a","note":"n","categories":[],"rows":[{row}],
           "probes":[{{"id":"p","kind":"positive","title":"t","asserts":"a","falsified_by":"f","request":{{}},
           "expect":{{"decision":"Allow","reason_contains":[],"reason_excludes":[],"duties":null,
           "dataset_id":null}}}}]}}"#
      )
    };
    let both = r#"{"id":"r","category":"actions","term":"t","status":"Partial","why":"w","evidence":"e",
                   "asserts":"a","probe_ids":["p"],"documented_because":"because","caveat":null}"#;
    let neither = r#"{"id":"r","category":"actions","term":"t","status":"Partial","why":"w","evidence":"e",
                      "asserts":"a","probe_ids":[],"documented_because":null,"caveat":null}"#;

    assert!(parse_coverage_catalog(&with_rows(both)).unwrap_err().contains("both probes and a documented_because"));
    assert!(parse_coverage_catalog(&with_rows(neither)).unwrap_err().contains("neither probes nor"));
  }

  #[test]
  fn status_display_spells_all_four_statuses_and_flags_an_unknown_one() {
    assert_eq!(status_display("Implemented"), "Implemented");
    assert_eq!(status_display("Partial"), "Partial");
    assert_eq!(status_display("NotImplemented"), "Not implemented");
    assert_eq!(status_display("OutOfScope"), "Out of scope");
    assert_eq!(status_display("Whatever"), "Unknown status");
  }

  #[test]
  fn compile_coverage_report_tallies_both_axes_and_keeps_catalog_order() {
    let catalog = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses");
    // Every probe agrees: the shape a healthy run produces.
    let outcomes: Vec<ProbeOutcome> =
      catalog.probes.iter().map(|p| probe_outcome(&p.id, ProbeStatus::Agreed)).collect();
    let report = compile_coverage_report(&catalog, outcomes, 12.5, 232_881);

    assert_eq!(report.total_probes, catalog.probes.len() as u64);
    assert_eq!(report.agreed, catalog.probes.len() as u64);
    assert_eq!((report.disagreed, report.errored), (0, 0));
    assert_eq!(report.rows.len(), catalog.rows.len());
    assert_eq!(
      report.rows.iter().map(|r| r.row.id.as_str()).collect::<Vec<_>>(),
      catalog.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(report.verified + report.documented, catalog.rows.len() as u64);
    assert_eq!((report.contradicted, report.inconclusive), (0, 0));
    assert_eq!(
      report.implemented + report.partial + report.not_implemented + report.out_of_scope,
      catalog.rows.len() as u64,
      "every row's documented status must fall in one of the four buckets"
    );
    assert_eq!(report.elapsed_ms, 12.5);
    assert_eq!(report.engine_bytes, 232_881);
    assert!(report.contradicted_rows().is_empty());
  }

  #[test]
  fn one_disagreeing_probe_turns_exactly_the_rows_that_name_it_contradicted() {
    let catalog = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses");
    let target = "act-base-exact";
    let expected_rows: Vec<&str> = catalog
      .rows
      .iter()
      .filter(|row| row.probe_ids.iter().any(|id| id == target))
      .map(|row| row.id.as_str())
      .collect();
    assert!(!expected_rows.is_empty(), "the probe this test perturbs must be referenced by at least one row");

    let outcomes: Vec<ProbeOutcome> = catalog
      .probes
      .iter()
      .map(|p| {
        probe_outcome(&p.id, if p.id == target { ProbeStatus::Disagreed } else { ProbeStatus::Agreed })
      })
      .collect();
    let report = compile_coverage_report(&catalog, outcomes, 1.0, 1);

    let contradicted: Vec<&str> = report.contradicted_rows().iter().map(|r| r.row.id.as_str()).collect();
    assert_eq!(contradicted, expected_rows);
    assert_eq!(report.contradicted, expected_rows.len() as u64);
  }

  // ---- guards on the committed artifact itself ----------------------

  #[test]
  fn the_committed_catalog_parses_and_declares_this_pages_schema() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed artifact parses");

    assert_eq!(file.schema, COVERAGE_SCHEMA);
    assert_eq!(file.rows.len(), 52, "the source gap analysis enumerates 52 vocabulary rows");
    assert_eq!(file.categories.len(), 10);
    assert_eq!(
      file.probes.len(),
      136,
      "grew by 4 more (132 -> 136) when kind: Agreement and kind: Offer both gained real party-role \
       probes -- agreementAssigneeClaim's own hit/miss pair (pc-kind-agreement-assignee-claim-hit, \
       pc-kind-agreement-assignee-claim-excludes-a-mismatch) and Offer's unconditional-inertness pair \
       (pc-kind-offer-assignee-inert-even-on-a-match, pc-kind-offer-assignee-inert-even-on-a-mismatch), \
       on top of the 131 -> 132 growth when odrl:andSequence moved from a single dropped-key negative \
       probe to a real hit/miss pair (lc-andsequence-honored, lc-andsequence-miss) alongside the \
       pre-existing odrl:and control, the 129 -> 131 growth for odrl:conflict cross-policy \
       voiding when a parent and child joined by odrl:inheritFrom declare differing conflict values \
       over a genuine collision (inheritfrom-conflict-divergence-*), the 127 -> 129 growth when \
       odrl:inheritFrom itself moved from documented-only to two real hit/control pairs \
       (inheritfrom-safe-direction-*, inheritfrom-fail-open-*), and the 125 -> 127 growth \
       odrl:AssetCollection membership (odrl:partOf) already added"
    );
    assert!(file.spec.contains("odrl-vocab"));
  }

  #[test]
  fn every_committed_row_names_a_real_category_and_every_category_carries_rows() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed artifact parses");

    for row in &file.rows {
      assert!(
        file.categories.iter().any(|c| c.id == row.category),
        "row {} names unknown category {}",
        row.id,
        row.category
      );
      assert!(matches!(row.status.as_str(), "Implemented" | "Partial" | "NotImplemented" | "OutOfScope"));
    }
    for category in &file.categories {
      assert!(
        file.rows.iter().any(|row| row.category == category.id),
        "category {} carries no rows",
        category.id
      );
    }
  }

  #[test]
  fn every_committed_probe_is_referenced_by_at_least_one_row() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed artifact parses");
    for probe in &file.probes {
      assert!(
        file.rows.iter().any(|row| row.probe_ids.contains(&probe.id)),
        "probe {} is referenced by no row, so nothing this page renders would ever show it",
        probe.id
      );
    }
  }

  #[test]
  fn exactly_two_committed_rows_are_documented_only_and_each_says_why() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed artifact parses");
    let documented: Vec<&CatalogRow> = file.rows.iter().filter(|row| row.is_documented_only()).collect();

    // Party collections and hasPolicy remain documented-only; asset
    // collections moved to probed once Request::asset_collections gave
    // evaluate() a real wire fact to test odrl:partOf membership against.
    assert_eq!(documented.len(), 2);
    for row in documented {
      assert!(row.documented_because.as_ref().is_some_and(|why| !why.is_empty()), "row {} has an empty why", row.id);
    }
  }

  #[test]
  fn every_committed_probes_request_is_a_complete_section_5_2_envelope() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed artifact parses");
    for probe in &file.probes {
      let raw = probe_json(probe);
      for key in ["dataset_id", "action", "config", "policies", "claims"] {
        assert!(raw.contains(key), "{}: request is missing `{key}`", probe.id);
      }
      assert!(matches!(probe.expect.decision.as_str(), "Allow" | "Deny" | "Error"), "{}", probe.id);
      assert!(matches!(probe.kind.as_str(), "positive" | "negative"), "{}", probe.id);
    }
  }

  /// The falsifiability check, over every committed probe: a probe that
  /// cannot be made to fail is not a probe.
  ///
  /// For each one this synthesizes the response its expectation
  /// describes, asserts that agrees, then flips the decision to a
  /// different one the contract defines and asserts *that* disagrees. If
  /// any probe's expectation were vacuous — an empty `reason_contains`
  /// paired with a decision the engine reaches for every input, say — the
  /// second half would still agree and this test would say so, naming the
  /// probe.
  #[test]
  fn every_committed_probe_can_both_agree_and_be_made_to_disagree() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses");

    for probe in &file.probes {
      let expect = &probe.expect;
      let synthetic_reason = expect.reason_contains.join(" | ");
      for excluded in &expect.reason_excludes {
        assert!(
          !synthetic_reason.contains(excluded.as_str()),
          "{}: this probe's own required substrings contain its excluded one `{excluded}`, which \
           would make the expectation unsatisfiable",
          probe.id
        );
      }

      let agreeing = EngineResponse {
        decision: expect.decision.clone(),
        reason: synthetic_reason,
        duties: expect.duties.clone().unwrap_or_default(),
        dataset_id: expect.dataset_id.clone().unwrap_or_default(),
      };
      let (status, mismatch) = classify_probe(expect, &agreeing);
      assert_eq!(status, ProbeStatus::Agreed, "{}: {mismatch:?}", probe.id);

      let other = if expect.decision == "Allow" { "Deny" } else { "Allow" };
      let disagreeing = EngineResponse { decision: other.to_string(), ..agreeing };
      let (status, mismatch) = classify_probe(expect, &disagreeing);
      assert_eq!(status, ProbeStatus::Disagreed, "{}: flipping the decision must disagree", probe.id);
      assert_eq!(
        mismatch.as_deref(),
        Some(format!("expected {}, observed {other}", expect.decision).as_str()),
        "{}",
        probe.id
      );
    }
  }

  /// The end-to-end half of the same check, at row level: perturbing one
  /// probe's expectation must turn its row red, raise the page's
  /// contradiction count, and put the row in `contradicted_rows()` (which
  /// is what drives the danger alert and the row tint). Run across a
  /// sample spanning both probe kinds and all four documented statuses,
  /// so it is not just the one easy row.
  #[test]
  fn perturbing_one_probes_expectation_turns_its_rows_contradicted() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses");

    let sample = [
      "act-base-exact",                    // Implemented, positive
      "act-includedin-undeclared-gap",     // Partial, negative
      "lc-andsequence-miss",               // Partial, negative
      "op-profile-operator-unparseable",   // OutOfScope, negative
      "duty-per-permission-advisory",      // Partial, duties asserted with a source
      "asset-per-rule-target-hit",         // Partial, positive, per-rule odrl:target
      "beh-closed-empty",                  // Implemented, reason_excludes
      "op-isnoneof-absent-satisfies",      // Implemented, positive
      "conflict-default-invalid-voids",    // Implemented, positive, reason_excludes
      "inheritfrom-fail-open-hit",         // Partial, positive, multi-policy
    ];

    for probe_id in sample {
      assert!(file.probes.iter().any(|p| p.id == probe_id), "no probe {probe_id}");
      let expected_rows: Vec<&str> = file
        .rows
        .iter()
        .filter(|row| row.probe_ids.iter().any(|id| id == probe_id))
        .map(|row| row.id.as_str())
        .collect();
      assert!(!expected_rows.is_empty(), "{probe_id} is referenced by no row");

      let outcomes: Vec<ProbeOutcome> = file
        .probes
        .iter()
        .map(|p| {
          probe_outcome(&p.id, if p.id == probe_id { ProbeStatus::Disagreed } else { ProbeStatus::Agreed })
        })
        .collect();
      let report = compile_coverage_report(&file, outcomes, 1.0, 1);

      let contradicted: Vec<&str> = report.contradicted_rows().iter().map(|r| r.row.id.as_str()).collect();
      assert_eq!(contradicted, expected_rows, "perturbing {probe_id}");
      assert_eq!(report.contradicted, expected_rows.len() as u64, "perturbing {probe_id}");
      assert_eq!(report.disagreed, 1);
    }
  }

  /// The bytes handed to `engine.wasm` must be exactly the bytes the
  /// generator wrote — not a re-serialization. This checks the property
  /// directly on the committed artifact: several probes carry keys
  /// `crate::wire`'s types do not model, and a round trip would drop the
  /// one key each of those probes turns on. Most are keys the *engine*
  /// does not model either — that is what those probes assert. The
  /// exceptions are `odrl:target` and `odrl:duty`, which the engine models
  /// and this site's own mirror of `Rule` deliberately still does not
  /// (`site/README.md`): a round trip would drop either here just the
  /// same, silently turning a probe about per-rule assets into one about
  /// none, or a probe about a per-permission duty into one about a bare
  /// permission.
  #[test]
  fn a_probes_raw_request_keeps_the_keys_this_sites_own_types_do_not_model() {
    let file = parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed artifact parses");
    let raw_of = |id: &str| {
      probe_json(file.probes.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("no probe {id}"))).to_string()
    };

    for (probe_id, key) in [
      ("lc-andsequence-miss", "odrl:andSequence"),
      ("conflict-perm-allows", "odrl:conflict"),
      ("duty-per-permission-advisory", "odrl:duty"),
      ("asset-per-rule-target-hit", "odrl:target"),
      ("act-implies-ignored", "odrl:implies"),
      ("uid-rule-index-not-uid", "urn:rule:r-7"),
      ("ror-reference-key-ignored", "rightOperandReference"),
      ("pf-common-functions-inert", "trackedParty"),
      ("op-isa-unparseable", "\"isA\""),
    ] {
      assert!(raw_of(probe_id).contains(key), "{probe_id}: the raw request no longer carries {key}");
    }
  }
}
