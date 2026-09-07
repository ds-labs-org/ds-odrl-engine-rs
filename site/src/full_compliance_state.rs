//! The Full Compliance page's run state machine: its four stages, its
//! live counters, and the queries its stepper draws itself from.
//!
//! Kept separate from `coverage_state.rs` rather than shared with it, for
//! the reason that module's own header already gives for not sharing with
//! the Compliance page: the two runs differ in what their counters count.
//! `/coverage` tallies agreed/disagreed/errored — agreement with the
//! *documentation*. This run tallies meets/falls-short/undetermined —
//! conformance to the *spec*. They are not the same three buckets wearing
//! different labels, and a generic state machine parameterised over both
//! would be more code than the sixty lines it replaced while coupling two
//! pages that must stay free to answer different questions.
//!
//! What genuinely is identical between the two runs — loading
//! `engine.wasm`, fetching and parsing the catalog, and driving every
//! probe through the real ABI — is shared rather than copied, in
//! `coverage_run::replay_all` and `run_support`.
//!
//! Ungated, like `coverage_state.rs` and for the same reason: an
//! off-by-one in stage ordering paints the wrong step as current for a
//! visitor watching a run, and a miscounted tally misreports it. Neither
//! should need a browser to catch.

use crate::full_compliance::{FullComplianceReport, SpecOutcome};

/// The four steps the page shows, in the order they run.
///
/// The first three are word-for-word what `/coverage` shows, because they
/// are the same three actions over the same artifacts. The fourth is not:
/// what happens there is a judgment against ODRL 2.2, not against this
/// study's own documentation, and calling it "Compiling coverage report"
/// would misdescribe the only stage where the two pages diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
  LoadingWasm,
  LoadingCatalog,
  Replaying,
  Judging,
}

impl Stage {
  pub const ALL: [Stage; 4] = [Stage::LoadingWasm, Stage::LoadingCatalog, Stage::Replaying, Stage::Judging];

  pub fn label(self) -> &'static str {
    match self {
      Stage::LoadingWasm => "Loading engine.wasm",
      Stage::LoadingCatalog => "Loading probe catalog",
      Stage::Replaying => "Replaying probes",
      Stage::Judging => "Judging against ODRL 2.2",
    }
  }

  fn order(self) -> usize {
    match self {
      Stage::LoadingWasm => 0,
      Stage::LoadingCatalog => 1,
      Stage::Replaying => 2,
      Stage::Judging => 3,
    }
  }
}

/// Live counts while the probe loop runs — this page's own axis, and
/// deliberately not `/coverage`'s agreed/disagreed/errored.
///
/// `total` counts only the probes this page actually judges (those named
/// by at least one in-scope row), so the progress line a visitor watches
/// and the tally the finished report prints are the same denominator.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpecProgress {
  pub done: usize,
  pub total: usize,
  pub meets: usize,
  pub falls_short: usize,
  pub undetermined: usize,
}

impl SpecProgress {
  pub fn record(&mut self, outcome: SpecOutcome) {
    self.done += 1;
    match outcome {
      SpecOutcome::MeetsFullSpec => self.meets += 1,
      SpecOutcome::FallsShort => self.falls_short += 1,
      SpecOutcome::Undetermined => self.undetermined += 1,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
  LoadingWasm,
  LoadingCatalog { engine_bytes: usize },
  Replaying { engine_bytes: usize, progress: SpecProgress },
  Judging { engine_bytes: usize, progress: SpecProgress },
  Done(Box<FullComplianceReport>),
  Failed { stage: Stage, message: String },
}

impl RunState {
  /// Which step the stepper paints as current. `None` once the run is
  /// `Done` — a finished run has nothing left to say.
  pub fn current_stage(&self) -> Option<Stage> {
    match self {
      RunState::LoadingWasm => Some(Stage::LoadingWasm),
      RunState::LoadingCatalog { .. } => Some(Stage::LoadingCatalog),
      RunState::Replaying { .. } => Some(Stage::Replaying),
      RunState::Judging { .. } => Some(Stage::Judging),
      RunState::Done(_) => None,
      RunState::Failed { stage, .. } => Some(*stage),
    }
  }

  pub fn progress(&self) -> Option<&SpecProgress> {
    match self {
      RunState::Replaying { progress, .. } | RunState::Judging { progress, .. } => Some(progress),
      _ => None,
    }
  }

  /// Whether `stage` has already finished successfully. A failed run marks
  /// only the steps *before* the failure complete, so the later ones sit
  /// pending rather than spinning forever.
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

  fn progress(done: usize) -> SpecProgress {
    SpecProgress { done, total: 130, meets: done, falls_short: 0, undetermined: 0 }
  }

  #[test]
  fn the_four_stages_are_ordered_and_the_last_one_names_the_spec_not_the_documentation() {
    let labels: Vec<&str> = Stage::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(
      labels,
      ["Loading engine.wasm", "Loading probe catalog", "Replaying probes", "Judging against ODRL 2.2"]
    );
    assert_eq!(Stage::ALL.iter().map(|s| s.order()).collect::<Vec<_>>(), [0, 1, 2, 3]);
    // The one stage where the two pages diverge must not describe itself
    // in the other page's terms.
    assert!(!Stage::Judging.label().contains("coverage"));
  }

  #[test]
  fn walking_the_happy_path_marks_exactly_the_earlier_stages_complete_at_each_step() {
    let expected_complete: [&[Stage]; 4] = [
      &[],
      &[Stage::LoadingWasm],
      &[Stage::LoadingWasm, Stage::LoadingCatalog],
      &[Stage::LoadingWasm, Stage::LoadingCatalog, Stage::Replaying],
    ];
    let sequence = [
      RunState::LoadingWasm,
      RunState::LoadingCatalog { engine_bytes: 1 },
      RunState::Replaying { engine_bytes: 1, progress: progress(0) },
      RunState::Judging { engine_bytes: 1, progress: progress(130) },
    ];

    for (state, complete) in sequence.iter().zip(expected_complete) {
      for stage in Stage::ALL {
        assert_eq!(state.is_complete(stage), complete.contains(&stage), "stage {:?}", stage.label());
        assert!(!state.failed_at(stage));
      }
    }
  }

  #[test]
  fn a_failure_marks_only_the_earlier_stages_complete_and_leaves_the_later_ones_pending() {
    let state = RunState::Failed { stage: Stage::LoadingCatalog, message: "HTTP 404".to_string() };

    assert!(state.is_complete(Stage::LoadingWasm));
    assert!(!state.is_complete(Stage::LoadingCatalog));
    assert!(!state.is_complete(Stage::Replaying));
    assert!(!state.is_complete(Stage::Judging));

    assert!(state.failed_at(Stage::LoadingCatalog));
    for stage in [Stage::LoadingWasm, Stage::Replaying, Stage::Judging] {
      assert!(!state.failed_at(stage));
    }
  }

  #[test]
  fn progress_is_exposed_only_while_replaying_and_judging() {
    assert_eq!(RunState::LoadingWasm.progress(), None);
    assert_eq!(RunState::LoadingCatalog { engine_bytes: 1 }.progress(), None);
    assert_eq!(RunState::Replaying { engine_bytes: 1, progress: progress(7) }.progress(), Some(&progress(7)));
    assert_eq!(RunState::Failed { stage: Stage::Replaying, message: "x".to_string() }.progress(), None);
  }

  #[test]
  fn recording_outcomes_advances_done_and_exactly_one_tally_each_time() {
    let mut progress = SpecProgress { total: 3, ..SpecProgress::default() };

    progress.record(SpecOutcome::MeetsFullSpec);
    assert_eq!((progress.done, progress.meets, progress.falls_short, progress.undetermined), (1, 1, 0, 0));
    progress.record(SpecOutcome::FallsShort);
    assert_eq!((progress.done, progress.meets, progress.falls_short, progress.undetermined), (2, 1, 1, 0));
    progress.record(SpecOutcome::Undetermined);
    assert_eq!((progress.done, progress.meets, progress.falls_short, progress.undetermined), (3, 1, 1, 1));

    // The three tallies partition `done`, which is what makes the
    // stepper's live description add up.
    assert_eq!(progress.meets + progress.falls_short + progress.undetermined, progress.done);
    assert_eq!(progress.done, progress.total);
  }
}
