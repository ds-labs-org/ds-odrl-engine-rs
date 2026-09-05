//! The Coverage page's run state machine: its four stages, its live
//! counters, and the queries the stepper draws itself from.
//!
//! Split out of `coverage_run.rs` — which keeps only the `async fn run`
//! that touches `window`, `js_sys` and the ABI bridge — and left
//! **ungated**, because none of this needs a browser and all of it is the
//! kind of logic that goes quietly wrong: an off-by-one in stage ordering
//! paints the wrong step as current, and a miscounted progress tally
//! misreports a run that a visitor is watching. Under a
//! `#[cfg(target_arch = "wasm32")]` gate these tests would silently never
//! compile under `cargo test --workspace`, let alone run.
//!
//! The Compliance page's equivalent types still live inside its own gated
//! `compliance_run.rs`; this module is not shared with it, deliberately.
//! The two runs differ in stage labels, in what their counters count
//! (passed/failed/skipped versus agreed/disagreed/errored) and in their
//! terminal payload, so a generic `RunState<P, R>` plus a stages trait
//! would be more code than the ~60 lines it replaced and would couple two
//! pages that should stay free to grow different stage lists. What is
//! genuinely identical between them — the browser plumbing — is shared,
//! in `run_support.rs`.

use crate::coverage_catalog::{CoverageReport, ProbeStatus};

/// The four steps the page shows, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
  LoadingWasm,
  LoadingCatalog,
  Probing,
  Compiling,
}

impl Stage {
  pub const ALL: [Stage; 4] = [Stage::LoadingWasm, Stage::LoadingCatalog, Stage::Probing, Stage::Compiling];

  pub fn label(self) -> &'static str {
    match self {
      Stage::LoadingWasm => "Loading engine.wasm",
      Stage::LoadingCatalog => "Loading probe catalog",
      Stage::Probing => "Performing probes",
      Stage::Compiling => "Compiling coverage report",
    }
  }

  fn order(self) -> usize {
    match self {
      Stage::LoadingWasm => 0,
      Stage::LoadingCatalog => 1,
      Stage::Probing => 2,
      Stage::Compiling => 3,
    }
  }
}

/// Live counts while the probe loop runs. These three tallies are this
/// page's own axis — agreement with the documented status — deliberately
/// not the Compliance page's passed/failed/skipped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CoverageProgress {
  pub done: usize,
  pub total: usize,
  pub agreed: usize,
  pub disagreed: usize,
  pub errored: usize,
}

impl CoverageProgress {
  pub fn record(&mut self, status: ProbeStatus) {
    self.done += 1;
    match status {
      ProbeStatus::Agreed => self.agreed += 1,
      ProbeStatus::Disagreed => self.disagreed += 1,
      ProbeStatus::Errored => self.errored += 1,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
  LoadingWasm,
  LoadingCatalog { engine_bytes: usize },
  Probing { engine_bytes: usize, progress: CoverageProgress },
  Compiling { engine_bytes: usize, progress: CoverageProgress },
  Done(Box<CoverageReport>),
  Failed { stage: Stage, message: String },
}

impl RunState {
  /// Which step the stepper should paint as current. `None` once the run
  /// is `Done` — a finished run has nothing left to say.
  pub fn current_stage(&self) -> Option<Stage> {
    match self {
      RunState::LoadingWasm => Some(Stage::LoadingWasm),
      RunState::LoadingCatalog { .. } => Some(Stage::LoadingCatalog),
      RunState::Probing { .. } => Some(Stage::Probing),
      RunState::Compiling { .. } => Some(Stage::Compiling),
      RunState::Done(_) => None,
      RunState::Failed { stage, .. } => Some(*stage),
    }
  }

  pub fn progress(&self) -> Option<&CoverageProgress> {
    match self {
      RunState::Probing { progress, .. } | RunState::Compiling { progress, .. } => Some(progress),
      _ => None,
    }
  }

  /// Whether `stage` has already finished successfully — every step
  /// before the current one, and (for a completed run) all of them. A
  /// failed run marks only the steps *before* the failure complete, so
  /// the later ones sit `Pending` rather than spinning forever.
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

#[cfg(test)]
mod tests {
  use super::*;

  fn progress(done: usize) -> CoverageProgress {
    CoverageProgress { done, total: 113, agreed: done, disagreed: 0, errored: 0 }
  }

  #[test]
  fn the_four_stages_are_ordered_and_each_carries_its_own_label() {
    let labels: Vec<&str> = Stage::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(
      labels,
      ["Loading engine.wasm", "Loading probe catalog", "Performing probes", "Compiling coverage report"]
    );
    // Strictly increasing order, so `is_complete`'s comparisons mean what
    // they say.
    let orders: Vec<usize> = Stage::ALL.iter().map(|s| s.order()).collect();
    assert_eq!(orders, [0, 1, 2, 3]);
  }

  #[test]
  fn each_state_reports_itself_as_the_current_stage_and_done_reports_none() {
    assert_eq!(RunState::LoadingWasm.current_stage(), Some(Stage::LoadingWasm));
    assert_eq!(RunState::LoadingCatalog { engine_bytes: 1 }.current_stage(), Some(Stage::LoadingCatalog));
    assert_eq!(
      RunState::Probing { engine_bytes: 1, progress: progress(0) }.current_stage(),
      Some(Stage::Probing)
    );
    assert_eq!(
      RunState::Compiling { engine_bytes: 1, progress: progress(113) }.current_stage(),
      Some(Stage::Compiling)
    );
    assert_eq!(
      RunState::Failed { stage: Stage::LoadingCatalog, message: "boom".to_string() }.current_stage(),
      Some(Stage::LoadingCatalog)
    );
  }

  /// The transition sequence the driver actually publishes, walked
  /// end to end. Each state must mark exactly the stages before it
  /// complete and no more -- the property the stepper's Success/Info/
  /// Pending rendering is derived from, which is otherwise only ever
  /// checked by looking at a browser.
  #[test]
  fn walking_the_happy_path_marks_exactly_the_earlier_stages_complete_at_each_step() {
    let expected_complete: [&[Stage]; 4] = [
      &[],
      &[Stage::LoadingWasm],
      &[Stage::LoadingWasm, Stage::LoadingCatalog],
      &[Stage::LoadingWasm, Stage::LoadingCatalog, Stage::Probing],
    ];
    let sequence = [
      RunState::LoadingWasm,
      RunState::LoadingCatalog { engine_bytes: 1 },
      RunState::Probing { engine_bytes: 1, progress: progress(0) },
      RunState::Compiling { engine_bytes: 1, progress: progress(113) },
    ];

    for (state, complete) in sequence.iter().zip(expected_complete) {
      for stage in Stage::ALL {
        assert_eq!(
          state.is_complete(stage),
          complete.contains(&stage),
          "stage {:?} at state {:?}",
          stage.label(),
          state.current_stage().map(Stage::label)
        );
        assert!(!state.failed_at(stage), "a healthy run never marks a stage failed");
      }
    }
  }

  #[test]
  fn a_failure_marks_only_the_earlier_stages_complete_and_leaves_the_later_ones_pending() {
    let state = RunState::Failed { stage: Stage::LoadingCatalog, message: "HTTP 404".to_string() };

    assert!(state.is_complete(Stage::LoadingWasm));
    assert!(!state.is_complete(Stage::LoadingCatalog), "the failing stage is not complete");
    // The stages after the failure must sit Pending, not spin forever.
    assert!(!state.is_complete(Stage::Probing));
    assert!(!state.is_complete(Stage::Compiling));

    assert!(state.failed_at(Stage::LoadingCatalog));
    for stage in [Stage::LoadingWasm, Stage::Probing, Stage::Compiling] {
      assert!(!state.failed_at(stage));
    }
  }

  #[test]
  fn a_finished_run_marks_every_stage_complete_and_none_current() {
    let report = crate::coverage_catalog::compile_coverage_report(
      &crate::coverage_catalog::parse_coverage_catalog(include_str!(
        "../../compliance/reports/latest-coverage.json"
      ))
      .expect("the committed catalog parses"),
      vec![],
      1.0,
      1,
    );
    let state = RunState::Done(Box::new(report));

    assert_eq!(state.current_stage(), None);
    for stage in Stage::ALL {
      assert!(state.is_complete(stage));
      assert!(!state.failed_at(stage));
    }
    assert_eq!(state.progress(), None, "a finished run's payload is the report, not a counter");
  }

  #[test]
  fn progress_is_exposed_only_while_probing_and_compiling() {
    assert_eq!(RunState::LoadingWasm.progress(), None);
    assert_eq!(RunState::LoadingCatalog { engine_bytes: 1 }.progress(), None);
    assert_eq!(
      RunState::Probing { engine_bytes: 1, progress: progress(7) }.progress(),
      Some(&progress(7))
    );
    assert_eq!(
      RunState::Compiling { engine_bytes: 1, progress: progress(113) }.progress(),
      Some(&progress(113))
    );
    assert_eq!(RunState::Failed { stage: Stage::Probing, message: "x".to_string() }.progress(), None);
  }

  #[test]
  fn recording_probe_outcomes_advances_done_and_exactly_one_tally_each_time() {
    let mut progress = CoverageProgress { total: 3, ..CoverageProgress::default() };

    progress.record(ProbeStatus::Agreed);
    assert_eq!((progress.done, progress.agreed, progress.disagreed, progress.errored), (1, 1, 0, 0));

    progress.record(ProbeStatus::Disagreed);
    assert_eq!((progress.done, progress.agreed, progress.disagreed, progress.errored), (2, 1, 1, 0));

    progress.record(ProbeStatus::Errored);
    assert_eq!((progress.done, progress.agreed, progress.disagreed, progress.errored), (3, 1, 1, 1));

    // The three tallies partition `done`: no outcome is counted twice or
    // dropped, which is what makes the stepper's live description add up.
    assert_eq!(progress.agreed + progress.disagreed + progress.errored, progress.done);
    assert_eq!(progress.done, progress.total);
  }
}
