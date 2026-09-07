//! The Full Compliance page's async driver: fetch and instantiate the real
//! compiled `engine.wasm`, fetch the same probe catalog `/coverage` uses,
//! replay every probe through the *same* `coverage_run::replay_all` — so
//! both pages report on one execution of one engine, not two — and then
//! judge what came back against ODRL 2.2 rather than against this study's
//! own documentation.
//!
//! Everything computable without a browser lives elsewhere, where `cargo
//! test --workspace` can actually run it: the judging and tallying in
//! `full_compliance.rs`, the state machine in `full_compliance_state.rs`.
//! What is left here is the part that cannot be tested without a browser:
//! the async sequencing.
//!
//! **Every stage boundary is separated by a real await**, the discipline
//! established after an adversarial review of the Compliance page's runner
//! found a stage that existed in the state machine and never once painted.
//! `LoadingWasm` takes an explicit `yield_for_paint()` because
//! `ensure_loaded()` resolves straight from its cached thread-local on a
//! re-run, reaching no suspension point at all; `Replaying -> Judging ->
//! Done` takes one for the same reason, having no natural await between
//! the last two.

use yew::prelude::*;

use crate::coverage_catalog::{parse_coverage_catalog, COVERAGE_URL};
use crate::coverage_run::replay_all;
use crate::engine_bridge;
use crate::full_compliance::{compile_full_compliance_report, is_in_scope, judge_probe};
use crate::full_compliance_state::{RunState, SpecProgress, Stage};
use crate::run_support::{fetch_text, yield_for_paint};

pub async fn run(state: UseStateHandle<RunState>) {
  state.set(RunState::LoadingWasm);
  yield_for_paint().await;
  let engine_bytes = match engine_bridge::ensure_loaded().await {
    Ok(bytes) => bytes,
    Err(message) => return state.set(RunState::Failed { stage: Stage::LoadingWasm, message }),
  };

  state.set(RunState::LoadingCatalog { engine_bytes });
  let catalog = match fetch_text(COVERAGE_URL).await.and_then(|text| parse_coverage_catalog(&text)) {
    Ok(catalog) => catalog,
    Err(message) => return state.set(RunState::Failed { stage: Stage::LoadingCatalog, message }),
  };

  // Every probe in the catalog is replayed — the engine is driven exactly
  // as `/coverage` drives it — but the live counter counts only the probes
  // this page actually judges, so the number a visitor watches climb and
  // the number the finished report prints share one denominator. The six
  // probes named only by `OutOfScope` rows are replayed and not counted.
  let judged_ids: Vec<&str> = {
    let mut ids: Vec<&str> = Vec::new();
    for row in catalog.rows.iter().filter(|row| is_in_scope(row)) {
      for id in &row.probe_ids {
        if !ids.contains(&id.as_str()) {
          ids.push(id.as_str());
        }
      }
    }
    ids
  };

  let mut progress = SpecProgress { total: judged_ids.len(), ..SpecProgress::default() };
  state.set(RunState::Replaying { engine_bytes, progress: progress.clone() });

  let (outcomes, elapsed_ms) = replay_all(&catalog, |outcome| {
    if !judged_ids.contains(&outcome.id.as_str()) {
      return;
    }
    // The same `judge_probe` the finished report runs, called here purely
    // so the live counter is the real judgment rather than a placeholder
    // that a visitor would later see contradicted by the summary.
    if let Some(fixture) = catalog.probes.iter().find(|probe| probe.id == outcome.id) {
      progress.record(judge_probe(fixture, outcome).0);
      state.set(RunState::Replaying { engine_bytes, progress: progress.clone() });
    }
  })
  .await;

  state.set(RunState::Judging { engine_bytes, progress: progress.clone() });
  // Without a real await between this `set` and the next, Yew coalesces
  // the two updates into one render and "Judging against ODRL 2.2" is
  // never painted. That stage is not decoration: it derives 45 row
  // verdicts and both tallies.
  yield_for_paint().await;
  state.set(RunState::Done(Box::new(compile_full_compliance_report(
    &catalog,
    outcomes,
    elapsed_ms,
    engine_bytes,
  ))));
}
