//! The live compliance run's own state machine and async driver: fetch
//! and instantiate the real compiled `engine.wasm`, fetch the corpus of
//! per-case requests `compliance-runner` exported, drive every one of
//! them through the *same* `engine_bridge::evaluate` the Demonstrator
//! page uses (so every case inherits that module's fresh-`memory.buffer()`
//! discipline by construction, not by a second implementation remembering
//! to), then compile the tally.
//!
//! Everything that can be computed without a browser lives in
//! `compliance_cases.rs` instead, where `cargo test --workspace` can
//! actually run it. This module is `#[cfg(target_arch = "wasm32")]`-gated
//! like the rest of the crate because every line of it touches `window`,
//! `js_sys`, or the ABI bridge.

use yew::prelude::*;

use crate::compliance_cases::{
  BASELINE_URL, CASES_URL, CaseOutcome, CaseStatus, LiveReport, compile_report, errored_outcome, evaluated_outcome,
  non_evaluated_outcome, parse_baseline, parse_case_file, request_json,
};
use crate::engine_bridge;

/// The four steps the page shows, in the order they run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
  LoadingWasm,
  LoadingCases,
  Evaluating,
  Compiling,
}

impl Stage {
  pub const ALL: [Stage; 4] = [Stage::LoadingWasm, Stage::LoadingCases, Stage::Evaluating, Stage::Compiling];

  pub fn label(self) -> &'static str {
    match self {
      Stage::LoadingWasm => "Loading engine.wasm",
      Stage::LoadingCases => "Loading test cases",
      Stage::Evaluating => "Performing tests",
      Stage::Compiling => "Compiling result report",
    }
  }

  fn order(self) -> usize {
    match self {
      Stage::LoadingWasm => 0,
      Stage::LoadingCases => 1,
      Stage::Evaluating => 2,
      Stage::Compiling => 3,
    }
  }
}

/// Live counts while the evaluation loop runs — what the "Performing
/// tests" step's description and progress bar are drawn from.
#[derive(Clone, PartialEq, Default)]
pub struct RunProgress {
  pub done: usize,
  pub total: usize,
  pub passed: usize,
  pub failed: usize,
  pub errored: usize,
  pub skipped: usize,
}

impl RunProgress {
  fn record(&mut self, status: CaseStatus) {
    self.done += 1;
    match status {
      CaseStatus::Passed => self.passed += 1,
      CaseStatus::Failed => self.failed += 1,
      CaseStatus::Skipped => self.skipped += 1,
      CaseStatus::Errored => self.errored += 1,
    }
  }
}

#[derive(Clone, PartialEq)]
pub enum RunState {
  LoadingWasm,
  LoadingCases { engine_bytes: usize },
  Evaluating { engine_bytes: usize, progress: RunProgress },
  Compiling { engine_bytes: usize, progress: RunProgress },
  Done(LiveReport),
  Failed { stage: Stage, message: String },
}

impl RunState {
  /// Which step the stepper should paint as current, and whether the run
  /// has ended. `None` once the run is `Done`.
  pub fn current_stage(&self) -> Option<Stage> {
    match self {
      RunState::LoadingWasm => Some(Stage::LoadingWasm),
      RunState::LoadingCases { .. } => Some(Stage::LoadingCases),
      RunState::Evaluating { .. } => Some(Stage::Evaluating),
      RunState::Compiling { .. } => Some(Stage::Compiling),
      RunState::Done(_) => None,
      RunState::Failed { stage, .. } => Some(*stage),
    }
  }

  pub fn progress(&self) -> Option<&RunProgress> {
    match self {
      RunState::Evaluating { progress, .. } | RunState::Compiling { progress, .. } => Some(progress),
      _ => None,
    }
  }

  /// Whether `stage` has already finished successfully — every step
  /// before the current one, and (for a completed run) all of them.
  pub fn is_complete(&self, stage: Stage) -> bool {
    match self {
      RunState::Done(_) => true,
      RunState::Failed { stage: failed_at, .. } => stage.order() < failed_at.order(),
      _ => self.current_stage().is_some_and(|current| stage.order() < current.order()),
    }
  }

  pub fn failed_at(&self, stage: Stage) -> bool {
    matches!(self, RunState::Failed { stage: failed_at, .. } if *failed_at == stage)
  }
}

/// Hands control back to the browser's event loop so a pending Yew render
/// can actually paint. A resolved-`Promise` await would only reach the
/// *microtask* queue, which drains before the browser paints -- so this
/// goes through a real macrotask (`setTimeout(0)`) instead.
///
/// Called at most once per ~16 ms (one frame), never once per case: on
/// real hardware the whole 68-case run finishes in a few tens of
/// milliseconds, and a timeout per case would be manufacturing hundreds
/// of milliseconds of delay purely to make a fast thing look busy. This
/// module contains no artificial delay of any other kind either -- the
/// elapsed time the finished report prints is real work, measured.
async fn yield_for_paint() {
  let promise = js_sys::Promise::new(&mut |resolve, _reject| {
    match web_sys::window() {
      Some(window) => {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
      }
      // No `window` never happens in a browser, but a never-resolving
      // promise here would hang the whole run rather than degrade it --
      // so resolve immediately instead.
      None => {
        let _ = resolve.call0(&wasm_bindgen::JsValue::undefined());
      }
    }
  });
  let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// One frame at 60 Hz: the shortest interval at which yielding could let
/// anything new actually appear on screen.
const FRAME_MS: f64 = 16.0;

async fn fetch_text(url: &str) -> Result<String, String> {
  use wasm_bindgen::JsCast;

  let window = web_sys::window().ok_or_else(|| "no `window` (not running in a browser)".to_string())?;

  let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
    .await
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "fetch() did not resolve to a Response".to_string())?;

  if !response.ok() {
    return Err(format!("{url} fetch returned HTTP {}", response.status()));
  }

  wasm_bindgen_futures::JsFuture::from(response.text().map_err(describe_js_error)?)
    .await
    .map_err(describe_js_error)?
    .as_string()
    .ok_or_else(|| format!("{url}: response.text() did not resolve to a string"))
}

fn describe_js_error(err: wasm_bindgen::JsValue) -> String {
  err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

/// Runs the whole four-stage sequence, publishing each transition through
/// `state`. Every terminal path is either `Done` or `Failed`: no stage
/// can stay active forever.
///
/// One deliberate asymmetry: a failed *baseline* fetch is not fatal. The
/// live run is the authority here, and must not be blocked by the file it
/// is merely being cross-checked against; the page then says the
/// cross-check was unavailable.
pub async fn run(state: UseStateHandle<RunState>) {
  state.set(RunState::LoadingWasm);
  let engine_bytes = match engine_bridge::ensure_loaded().await {
    Ok(bytes) => bytes,
    Err(message) => return state.set(RunState::Failed { stage: Stage::LoadingWasm, message }),
  };

  state.set(RunState::LoadingCases { engine_bytes });
  let case_file = match fetch_text(CASES_URL).await.and_then(|text| parse_case_file(&text)) {
    Ok(file) => file,
    Err(message) => return state.set(RunState::Failed { stage: Stage::LoadingCases, message }),
  };
  let baseline = match fetch_text(BASELINE_URL).await {
    Ok(text) => parse_baseline(&text).ok(),
    Err(_) => None,
  };

  let mut progress = RunProgress { total: case_file.cases.len(), ..RunProgress::default() };
  state.set(RunState::Evaluating { engine_bytes, progress: progress.clone() });

  let mut outcomes: Vec<CaseOutcome> = Vec::with_capacity(case_file.cases.len());
  let started = js_sys::Date::now();
  let mut last_yield = started;

  for fixture in &case_file.cases {
    let outcome = match request_json(fixture) {
      None => non_evaluated_outcome(fixture),
      Some(request) => match engine_bridge::evaluate(request).await {
        Ok(response) => evaluated_outcome(fixture, &response),
        // One case failing at the ABI boundary can never hide the other
        // 67, nor strand the UI on this stage: it is recorded as an
        // errored case and the loop continues.
        Err(message) => errored_outcome(fixture, &message),
      },
    };

    progress.record(outcome.status);
    outcomes.push(outcome);
    state.set(RunState::Evaluating { engine_bytes, progress: progress.clone() });

    if js_sys::Date::now() - last_yield >= FRAME_MS {
      yield_for_paint().await;
      last_yield = js_sys::Date::now();
    }
  }

  let elapsed_ms = js_sys::Date::now() - started;
  state.set(RunState::Compiling { engine_bytes, progress: progress.clone() });
  // Without a real await between this `set` and the next one, Yew has no
  // chance to render in between -- the two updates coalesce into a
  // single render and "Compiling result report" is never actually
  // painted, found by an adversarial review driving this exact code path
  // under CPU throttling with a per-millisecond DOM sampler. `and_then`
  // et al on a `UseStateHandle` don't yield either; only a real
  // microtask/macrotask boundary does.
  yield_for_paint().await;
  state.set(RunState::Done(compile_report(outcomes, case_file.suite, elapsed_ms, engine_bytes, baseline.as_ref())));
}
