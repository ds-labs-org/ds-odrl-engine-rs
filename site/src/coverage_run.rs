//! The live ODRL 2.2 coverage run's own state machine and async driver:
//! fetch and instantiate the real compiled `engine.wasm`, fetch the probe
//! catalog `coverage-probes` exported, drive all 115 probes through the
//! *same* `engine_bridge::evaluate` the Demonstrator and Compliance pages
//! use (so every probe inherits that module's fresh-`memory.buffer()`
//! discipline by construction, not by a second implementation remembering
//! to), then derive 52 row verdicts from what came back.
//!
//! Everything that can be computed without a browser lives elsewhere,
//! where `cargo test --workspace` can actually run it: the catalog
//! parsing, judging and tallying in `coverage_catalog.rs`, and the run's
//! own state machine -- its stages, its counters, and the queries the
//! stepper is drawn from -- in `coverage_state.rs`. What is left here is
//! exactly the part that cannot be tested without a browser: the async
//! sequencing itself.
//!
//! The browser plumbing (`fetch_text`, `yield_for_paint`, `FRAME_MS`) is
//! shared with `compliance_run.rs` via `run_support.rs`; the state machine
//! deliberately is not -- see that module's header for why the idiom is
//! copied and only the plumbing shared.

use yew::prelude::*;

use crate::coverage_catalog::{
  compile_coverage_report, errored_probe_outcome, evaluated_probe_outcome, parse_coverage_catalog, probe_json,
  CoverageFile, ProbeOutcome, COVERAGE_URL,
};
use crate::coverage_state::{CoverageProgress, RunState, Stage};
use crate::engine_bridge;
use crate::run_support::{fetch_text, yield_for_paint, FRAME_MS};

/// Drives every probe in `catalog` through the real `engine.wasm` over its
/// `alloc`/`evaluate`/`dealloc` C ABI, in catalog order, and returns what
/// came back plus the wall-clock milliseconds it genuinely took.
///
/// **Shared with `/full-compliance`**, which asks a different question of
/// the same responses (see `full_compliance.rs`). Only the judging and the
/// presentation fork; the replay itself must not, or the two pages would
/// be reporting on two different executions of the engine and any
/// disagreement between them would be unattributable. Sharing it also
/// keeps both pages on `engine_bridge::evaluate`'s fresh-`memory.buffer()`
/// discipline by construction rather than by a second implementation
/// remembering to.
///
/// `on_outcome` is called once per probe, after that probe's outcome is
/// recorded and before the next one starts, so each caller can drive its
/// own progress counters — which is the one thing the two runs genuinely
/// do differently while probing.
///
/// The request bytes go over the ABI verbatim: never deserialized and
/// re-serialized, which would drop exactly the unknown ODRL keys most of
/// the negative probes exist to inject (see `coverage_catalog.rs`'s
/// header). A probe failing at the ABI boundary is recorded as an errored
/// outcome and the loop continues, so one failure can neither hide the
/// others nor strand the UI on this stage.
pub async fn replay_all(
  catalog: &CoverageFile,
  mut on_outcome: impl FnMut(&ProbeOutcome),
) -> (Vec<ProbeOutcome>, f64) {
  let mut outcomes: Vec<ProbeOutcome> = Vec::with_capacity(catalog.probes.len());
  let started = js_sys::Date::now();
  let mut last_yield = started;

  for probe in &catalog.probes {
    let outcome = match engine_bridge::evaluate(probe_json(probe)).await {
      Ok(response) => evaluated_probe_outcome(probe, &response),
      Err(message) => errored_probe_outcome(probe, &message),
    };

    on_outcome(&outcome);
    outcomes.push(outcome);

    if js_sys::Date::now() - last_yield >= FRAME_MS {
      yield_for_paint().await;
      last_yield = js_sys::Date::now();
    }
  }

  (outcomes, js_sys::Date::now() - started)
}

/// Runs the whole four-stage sequence, publishing each transition through
/// `state`. Every terminal path is either `Done` or `Failed`: no stage can
/// stay active forever.
///
/// **Every stage boundary here is separated by a real await**, which is
/// the discipline an adversarial review of the Compliance page's own
/// runner established after finding a stage that existed in the state
/// machine and never once painted:
///
/// * `LoadingWasm` is set before the first await of any kind, so it
///   paints on a first load. On a *re-run*, though, `ensure_loaded()`
///   resolves straight from its cached thread-local with no real
///   suspension point reached at all — which an adversarial review of
///   *this* module found coalesces the `LoadingWasm` and `LoadingCatalog`
///   `set`s into one render, exactly the "stage that never painted" bug
///   class, just on the re-run path instead of the first-load one. The
///   explicit `yield_for_paint()` right after this `set` closes it: it
///   forces a real macrotask boundary regardless of whether the awaited
///   future below actually suspends.
/// * `LoadingCatalog -> Probing` is separated by the catalog `fetch`,
///   which is a real network round trip even when the browser serves it
///   from its HTTP cache, so no two `set`s land in one render there.
/// * `Probing -> Compiling -> Done` has no natural await between the last
///   two, which is exactly the case that broke before, so it takes an
///   explicit `yield_for_paint()`.
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

  let mut progress = CoverageProgress { total: catalog.probes.len(), ..CoverageProgress::default() };
  state.set(RunState::Probing { engine_bytes, progress: progress.clone() });

  // The loop itself lives in `replay_all`, shared verbatim with
  // `/full-compliance`. What stays here is this run's own axis: an errored
  // probe makes its rows Inconclusive, never silently Verified.
  let (outcomes, elapsed_ms) = replay_all(&catalog, |outcome| {
    progress.record(outcome.status);
    state.set(RunState::Probing { engine_bytes, progress: progress.clone() });
  })
  .await;

  state.set(RunState::Compiling { engine_bytes, progress: progress.clone() });
  // Without a real await between this `set` and the next, Yew coalesces
  // the two updates into a single render and "Compiling coverage report"
  // is never painted at all. This stage is not decoration: it derives 52
  // row verdicts over 115 probe outcomes and tallies both axes.
  yield_for_paint().await;
  state.set(RunState::Done(Box::new(compile_coverage_report(&catalog, outcomes, elapsed_ms, engine_bytes))));
}
