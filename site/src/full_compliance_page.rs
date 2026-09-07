//! The Full ODRL 2.2 Compliance page.
//!
//! `/coverage` asks: **does the engine match what this study documents
//! about it?** This page asks a different question: **assuming the engine
//! *should* fully implement ODRL 2.2 for every row that is not
//! structurally out of scope, does its real, live behaviour meet that
//! ideal?**
//!
//! Both pages drive the same compiled `engine.wasm` over the same C ABI
//! with the same 136 requests, through the *same* replay
//! (`coverage_run::replay_all`), so any difference between them is a
//! difference of question and never of execution. What differs is the
//! target each response is judged against: `expect`, which records what
//! this engine does — honestly-documented gaps included — versus `ideal`,
//! which records what ODRL 2.2 requires.
//!
//! Three consequences the page states out loud:
//!
//! * **None of `/coverage`'s vocabulary appears here.** Agreed, Disagreed,
//!   Verified and Contradicted already mean matches-the-documentation.
//!   This page says *meets full spec* and *falls short*, which is a
//!   different claim about a different thing.
//! * **Nothing here is red, and falling short is not a failure.** A red
//!   row on `/coverage` means the engine and its own documentation
//!   disagree, which is louder and rarer. Falling short of full ODRL 2.2
//!   is what a `Partial` status *means*; this page's job is to show which
//!   specific answer would have to change, and why.
//! * **Most probes do not distinguish the two questions.** 115 of the 130
//!   probes this page judges already produce the spec-ideal answer, so the
//!   page quotes the 15 that do not rather than a bare `N/136` that would
//!   imply the whole catalog changed meaning.
//!
//! Everything computable without a browser — the judging, the tallies, the
//! sign-off list and the unreached-narrowing notes — lives in
//! `full_compliance.rs`, where `cargo test --workspace` can actually run
//! it. This module renders.

use crate::coverage_catalog::{status_display, Category, Ideal};
use crate::full_compliance::{
  contested_readings_for, unreached_narrowing, ContestedReading, FullComplianceReport, ProbeJudgment,
  RowJudgment, RowSpecVerdict, SpecOutcome, CONTESTED_READINGS,
};
use crate::full_compliance_run::run;
use crate::full_compliance_state::{RunState, SpecProgress, Stage};
use crate::pages::{case_study_credit, STAT_ROW_CSS};
use patternfly_yew::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusFilter {
  All,
  Implemented,
  Partial,
  NotImplemented,
}

impl StatusFilter {
  fn matches(self, status: &str) -> bool {
    match self {
      StatusFilter::All => true,
      StatusFilter::Implemented => status == "Implemented",
      StatusFilter::Partial => status == "Partial",
      StatusFilter::NotImplemented => status == "NotImplemented",
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VerdictFilter {
  All,
  Meets,
  Short,
}

impl VerdictFilter {
  fn matches(self, verdict: RowSpecVerdict) -> bool {
    match self {
      VerdictFilter::All => true,
      VerdictFilter::Meets => verdict == RowSpecVerdict::MeetsFullSpec,
      VerdictFilter::Short => verdict.is_short_of_spec(),
    }
  }
}

/// This page's own layout CSS, page-scoped as an inline `<style>` with
/// `ds-oe-fc-`-prefixed classes — the same choice `COVERAGE_CSS` and
/// `COMPLIANCE_CSS` already make rather than growing `assets/theme.css`
/// for one page.
///
/// **The palette is the argument.** There is no red anywhere on this page,
/// deliberately: on `/coverage` red means "the engine disagrees with its
/// own documentation", and a shortfall against full ODRL 2.2 is neither
/// that nor a failure. Amber marks a demonstrated shortfall, purple a
/// structural one, and both use a PatternFly 6 design token with a literal
/// hex fallback so they stay legible if a token is renamed upstream.
const FULL_COMPLIANCE_CSS: &str = r#"
.ds-oe-fc-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 1rem;
  margin: 0.5rem 0 0.5rem;
}
.ds-oe-fc-search { max-width: 24rem; flex: 1 1 18rem; }
.ds-oe-fc-count {
  margin: 0 0 0.75rem;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-legend {
  margin: 0.5rem 0 0;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-section { margin-top: 2rem; }
.ds-oe-fc-specref {
  margin: 0.1rem 0 0.6rem;
  font-size: 0.8rem;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-table-wrap { overflow-x: auto; }
/* Same reason /coverage anchors its cells to the row top: the last column
   grows to several hundred pixels once a row's probe list opens, and
   middle-aligning the short Term/Status cells against that strands them
   far from the content they describe. */
.ds-oe-fc-table-wrap td { vertical-align: top; }
.ds-oe-fc-term {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.85rem;
  max-width: 22rem;
  display: block;
}
.ds-oe-fc-sub {
  display: block;
  margin-top: 0.2rem;
  max-width: 26rem;
  font-size: 0.8rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-sub.is-caveat { font-style: italic; }
.ds-oe-fc-target {
  display: block;
  max-width: 36rem;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-row.is-short {
  border-left: 4px solid var(--pf-t--global--border--color--status--warning--default, #c46100);
  background: var(--pf-t--global--background--color--status--warning--default, #fdf7e7);
}
.ds-oe-fc-row.is-structural {
  border-left: 4px solid var(--pf-t--global--border--color--status--info--default, #6753ac);
  background: var(--pf-t--global--background--color--status--info--default, #f2f0fc);
}
.ds-oe-fc-probe {
  margin: 0 0 0.85rem;
  padding: 0.1rem 0 0.1rem 0.6rem;
  font-size: 0.85rem;
}
.ds-oe-fc-probe:last-child { margin-bottom: 0; }
.ds-oe-fc-probe.is-short {
  border-left: 3px solid var(--pf-t--global--border--color--status--warning--default, #c46100);
  background: var(--pf-t--global--background--color--status--warning--default, #fdf7e7);
}
.ds-oe-fc-probe-id {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.85em;
}
.ds-oe-fc-line {
  display: block;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-delta {
  display: block;
  margin: 0.35rem 0 0.1rem;
  font-size: 0.9em;
}
.ds-oe-fc-delta .is-observed { color: var(--pf-t--global--text--color--regular, #151515); font-weight: 600; }
.ds-oe-fc-delta .is-ideal { color: #a34c00; font-weight: 600; }
.ds-oe-fc-cite {
  display: block;
  margin-top: 0.35rem;
  padding-left: 0.6rem;
  border-left: 2px solid var(--pf-t--global--border--color--default, #d2d2d2);
  font-size: 0.95em;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-gap {
  display: block;
  max-width: 42rem;
  font-size: 0.85rem;
  line-height: 1.5;
}
.ds-oe-fc-unreached { color: #6753ac; font-style: italic; }
.ds-oe-fc-contested { color: #6753ac; }
.ds-oe-fc-empty {
  padding: 1.5rem;
  text-align: center;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-fc-run-panel {
  margin: 1rem 0 1.25rem;
  padding: 1rem 1.25rem;
  border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2);
  border-radius: 0.35rem;
}
.ds-oe-fc-bar { max-width: 32rem; margin-top: 1rem; }
.ds-oe-fc-provenance {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.75rem;
  margin: 0.75rem 0 0.25rem;
  font-size: 0.9rem;
}
.ds-oe-fc-note {
  margin: 0.35rem 0 0;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
/* The two summary panels share one width so the page reads as one column:
   32rem (the run panel's `-bar` width, inherited from /coverage) is fine
   for a progress bar and cramped for a fourteen-item list. */
.ds-oe-fc-signoff, .ds-oe-fc-summary { max-width: 52rem; margin: 1rem 0 0.5rem; }
.ds-oe-fc-signoff li { margin-bottom: 0.75rem; }
.ds-oe-fc-stat-value.is-meets { color: #3e8635; }
.ds-oe-fc-stat-value.is-short { color: #c46100; }
.ds-oe-fc-stat-value.is-structural { color: #6753ac; }
.ds-oe-fc-stat-value.is-undetermined { color: #8a8d90; }
.ds-oe-fc-stat-value.is-implemented { color: #3e8635; }
.ds-oe-fc-stat-value.is-partial { color: #b98412; }
.ds-oe-fc-stat-value.is-not-implemented { color: #c46100; }
"#;

/// The documented `/coverage` status a row carries, unchanged from that
/// page so the same row wears the same label on both — it is the same
/// claim, being asked a different question.
fn status_label(status: &str) -> Html {
  let color = match status {
    "Implemented" => Color::Green,
    "Partial" => Color::Yellow,
    "NotImplemented" => Color::Orange,
    _ => Color::Purple,
  };
  html!(<Label label={status_display(status).to_string()} color={color} compact=true />)
}

fn verdict_label(verdict: RowSpecVerdict) -> Html {
  let (color, icon) = match verdict {
    RowSpecVerdict::MeetsFullSpec => (Color::Green, Icon::CheckCircle),
    RowSpecVerdict::FallsShort => (Color::Orange, Icon::ExclamationTriangle),
    RowSpecVerdict::StructuralGap => (Color::Purple, Icon::InfoCircle),
    RowSpecVerdict::Undetermined => (Color::Grey, Icon::ExclamationCircle),
  };
  html!(<Label label={verdict.label().to_string()} color={color} icon={icon} compact=true />)
}

fn probe_outcome_label(outcome: SpecOutcome) -> Html {
  let (color, icon) = match outcome {
    SpecOutcome::MeetsFullSpec => (Color::Green, Icon::CheckCircle),
    SpecOutcome::FallsShort => (Color::Orange, Icon::ExclamationTriangle),
    SpecOutcome::Undetermined => (Color::Grey, Icon::ExclamationCircle),
  };
  html!(<Label label={outcome.label().to_string()} color={color} icon={icon} compact=true />)
}

/// The one line a reader takes away from a falls-short probe: what the
/// engine just answered, and what a fully spec-compliant engine would have
/// answered to the same bytes.
fn decision_delta(observed: &Option<String>, ideal: &Ideal) -> Html {
  let observed = observed.clone().unwrap_or_else(|| "—".to_string());
  html!(
    <span class="ds-oe-fc-delta">
      { "this engine, just now: " }<span class="is-observed">{ observed }</span>
      { " · full ODRL 2.2: " }<span class="is-ideal">{ ideal.decision.clone() }</span>
    </span>
  )
}

/// One row's "What full ODRL 2.2 requires" cell.
///
/// The structural gap is rendered as prose rather than as a probe list,
/// because there is no probe: expanding it would show an empty list, and
/// inventing a placeholder probe would be inventing a measurement.
fn target_html(row: &RowJudgment) -> Html {
  match row.verdict {
    RowSpecVerdict::StructuralGap => html!(
      <span class="ds-oe-fc-gap">
        <strong>{ "No request can pose this question." }</strong>
        { " " }
        { row.row.full_compliance_gap.clone().unwrap_or_default() }
      </span>
    ),
    RowSpecVerdict::FallsShort => {
      let probe = row.probes.iter().find(|p| p.outcome == SpecOutcome::FallsShort);
      let ideal = probe.and_then(|p| p.ideal.clone());
      html!(
        <span class="ds-oe-fc-target">
          if let (Some(probe), Some(ideal)) = (probe, ideal) {
            { decision_delta(&probe.observed_decision, &ideal) }
            <span class="ds-oe-fc-line">{ ideal.reason.clone() }</span>
            <span class="ds-oe-fc-line">{ "probe " }<code>{ probe.id.clone() }</code></span>
          } else {
            { "falls short" }
          }
        </span>
      )
    }
    RowSpecVerdict::MeetsFullSpec => {
      // Only an implementable row can have an unreached narrowing: a row
      // documented `Implemented` claims no gap for a probe to be missing.
      // Guarded on the status rather than left to the lookup returning
      // `None`, so the scope rule is visible where it applies.
      let unreached = row.row.is_implementable().then(|| unreached_narrowing(&row.row.id)).flatten();
      html!(
        <span class="ds-oe-fc-target">
          <strong>{ "Every probe this row carries reaches the spec-ideal answer." }</strong>
          if let Some(note) = unreached {
            <span class="ds-oe-fc-line ds-oe-fc-unreached">{ note }</span>
          }
        </span>
      )
    }
    RowSpecVerdict::Undetermined => {
      let probe = row.probes.iter().find(|p| p.outcome == SpecOutcome::Undetermined);
      html!(
        <span class="ds-oe-fc-target">
          <strong>{ "This run could not judge this row." }</strong>
          if let Some(probe) = probe {
            <span class="ds-oe-fc-line">{ probe.undetermined_because.clone().unwrap_or_default() }</span>
            <span class="ds-oe-fc-line">{ "probe " }<code>{ probe.id.clone() }</code></span>
          }
        </span>
      )
    }
  }
}

fn probe_detail(probe: &ProbeJudgment) -> Html {
  let class =
    if probe.outcome == SpecOutcome::FallsShort { "ds-oe-fc-probe is-short" } else { "ds-oe-fc-probe" };
  let observed = probe.observed_decision.clone().unwrap_or_else(|| "—".to_string());
  let reason = probe.observed_reason.clone().unwrap_or_default();
  let contested = CONTESTED_READINGS.iter().any(|entry| entry.probe == probe.id);

  html!(
    <div class={class} key={probe.id.clone()}>
      <div>
        { probe_outcome_label(probe.outcome) }
        { " " }
        <code class="ds-oe-fc-probe-id">{ probe.id.clone() }</code>
        { " " }
        <Label
          label={probe.kind.clone()}
          color={if probe.kind == "negative" { Color::Purple } else { Color::Blue }}
          compact=true
          outline=true
        />
        if contested {
          { " " }
          <Label label="contested reading" color={Color::Purple} compact=true outline=true />
        }
        { " " }
        { probe.title.clone() }
      </div>
      <span class="ds-oe-fc-line">{ probe.asserts.clone() }</span>
      if let Some(ideal) = &probe.ideal {
        { decision_delta(&probe.observed_decision, ideal) }
        <span class="ds-oe-fc-line">{ ideal.reason.clone() }</span>
        <span class="ds-oe-fc-cite">{ ideal.spec_citation.clone() }</span>
      } else {
        <span class="ds-oe-fc-line">
          { format!("observed {observed} — the answer full ODRL 2.2 requires, which is also the answer this study documents") }
        </span>
      }
      if let Some(because) = &probe.undetermined_because {
        <span class="ds-oe-fc-line">{ because.clone() }</span>
      }
      if !reason.is_empty() {
        <span class="ds-oe-fc-line">{ "engine reason: " }<code>{ reason }</code></span>
      }
    </div>
  )
}

fn row_html(row: &RowJudgment) -> Html {
  let class = match row.verdict {
    RowSpecVerdict::FallsShort => "ds-oe-fc-row is-short",
    RowSpecVerdict::StructuralGap => "ds-oe-fc-row is-structural",
    _ => "ds-oe-fc-row",
  };
  let short_probes = row.probes.iter().filter(|p| p.outcome == SpecOutcome::FallsShort).count();
  let contested = contested_readings_for(&row.row.id);

  html!(
    <tr role="row" class={class} key={row.row.id.clone()}>
      <td role="cell">
        <span class="ds-oe-fc-term">{ row.row.term.clone() }</span>
        <span class="ds-oe-fc-sub">{ row.row.why.clone() }</span>
        <span class="ds-oe-fc-sub"><code>{ row.row.evidence.clone() }</code></span>
      </td>
      <td role="cell">{ status_label(&row.row.status) }</td>
      <td role="cell">
        { verdict_label(row.verdict) }
        if row.probes.is_empty() {
          <span class="ds-oe-fc-sub">{ "no probe possible" }</span>
        } else {
          <span class="ds-oe-fc-sub">
            { format!(
                "{} of {} probe{} fall{} short",
                short_probes,
                row.probes.len(),
                if row.probes.len() == 1 { "" } else { "s" },
                if short_probes == 1 { "s" } else { "" }
            ) }
          </span>
        }
        if !contested.is_empty() {
          <span class="ds-oe-fc-sub ds-oe-fc-contested">
            { format!(
                "{} judgment{} here the spec genuinely admits another reading of — see the sign-off list above.",
                contested.len(),
                if contested.len() == 1 { "" } else { "s" }
            ) }
          </span>
        }
        if let Some(caveat) = &row.row.caveat {
          <span class="ds-oe-fc-sub is-caveat">{ caveat.clone() }</span>
        }
      </td>
      <td role="cell">
        { target_html(row) }
        if !row.probes.is_empty() {
          <ExpandableSection
            toggle_text_hidden="Show probes"
            toggle_text_expanded="Hide probes"
          >
            { for row.probes.iter().map(probe_detail) }
          </ExpandableSection>
        }
      </td>
    </tr>
  )
}

fn category_section(category: &Category, rows: Vec<&RowJudgment>) -> Html {
  if rows.is_empty() {
    return html!();
  }
  html!(
    <div class="ds-oe-fc-section" key={category.id.clone()}>
      <Title level={Level::H2}>{ format!("{}. {}", category.number, category.title) }</Title>
      <p class="ds-oe-fc-specref">{ category.spec_ref.clone() }</p>
      <div class="ds-oe-fc-table-wrap">
        <table class="pf-v6-c-table" role="grid">
          <thead>
            <tr role="row">
              <th role="columnheader">{ "Term" }</th>
              <th role="columnheader">{ "Documented status" }</th>
              <th role="columnheader">{ "Against full ODRL 2.2" }</th>
              <th role="columnheader">{ "What full ODRL 2.2 requires" }</th>
            </tr>
          </thead>
          <tbody>
            { for rows.iter().map(|row| row_html(row)) }
          </tbody>
        </table>
      </div>
    </div>
  )
}

fn step_status(state: &RunState, stage: Stage) -> ProgressStepperStepStatus {
  if state.failed_at(stage) {
    ProgressStepperStepStatus::Danger
  } else if state.is_complete(stage) {
    ProgressStepperStepStatus::Success
  } else if state.current_stage() == Some(stage) {
    ProgressStepperStepStatus::Info
  } else {
    ProgressStepperStepStatus::Pending
  }
}

fn replaying_description(state: &RunState) -> Option<String> {
  let progress = state.progress()?;
  let mut text = format!(
    "{} / {} judged — {} meet full spec, {} fall short",
    progress.done, progress.total, progress.meets, progress.falls_short
  );
  if progress.undetermined > 0 {
    text.push_str(&format!(", {} undetermined", progress.undetermined));
  }
  Some(text)
}

/// `ProgressStepper` takes literal `<ProgressStepperStep>` children (a
/// macro-expansion constraint), so the four steps are written out rather
/// than looped — `Stage::ALL` still drives their labels and statuses, so
/// the ordering lives in one place.
fn stepper(state: &RunState) -> Html {
  let [loading_wasm, loading_catalog, replaying, judging] = Stage::ALL;
  html!(
    <ProgressStepper>
      <ProgressStepperStep
        status={step_status(state, loading_wasm)}
        is_current={state.current_stage() == Some(loading_wasm)}
      >
        <span>{ loading_wasm.label() }</span>
      </ProgressStepperStep>
      <ProgressStepperStep
        status={step_status(state, loading_catalog)}
        is_current={state.current_stage() == Some(loading_catalog)}
      >
        <span>{ loading_catalog.label() }</span>
      </ProgressStepperStep>
      <ProgressStepperStep
        status={step_status(state, replaying)}
        is_current={state.current_stage() == Some(replaying)}
        description={replaying_description(state)}
      >
        <span>{ replaying.label() }</span>
      </ProgressStepperStep>
      <ProgressStepperStep
        status={step_status(state, judging)}
        is_current={state.current_stage() == Some(judging)}
      >
        <span>{ judging.label() }</span>
      </ProgressStepperStep>
    </ProgressStepper>
  )
}

fn progress_bar(progress: &SpecProgress) -> Html {
  let total = progress.total.max(1) as f64;
  html!(
    <div class="ds-oe-fc-bar">
      <Progress
        value={progress.done as f64}
        range={0f64..total}
        value_text={format!("{} / {}", progress.done, progress.total)}
      />
    </div>
  )
}

fn rerun_button(on_rerun: Callback<MouseEvent>) -> Html {
  html!(
    <p class="ds-oe-fc-note">
      <Button variant={ButtonVariant::Secondary} onclick={on_rerun}>{ "Re-run in this browser" }</Button>
    </p>
  )
}

fn run_panel(state: &RunState, on_rerun: Callback<MouseEvent>) -> Html {
  html!(
    <div class="ds-oe-fc-run-panel">
      { stepper(state) }
      if let Some(progress) = state.progress() {
        { progress_bar(progress) }
      }
      if let RunState::Failed { stage, message } = state {
        <div class="ds-oe-fc-bar">
          <Alert inline=true r#type={AlertType::Danger} title={format!("{} failed", stage.label())}>
            <p>{ message.clone() }</p>
          </Alert>
        </div>
        { rerun_button(on_rerun) }
      }
    </div>
  )
}

/// The headline summary. `AlertType::Warning`, never `Danger`: a shortfall
/// against full ODRL 2.2 is a measured gap, not a defect, and `Danger` on
/// this site already means a live run disagreed with something committed.
fn shortfall_alert(report: &FullComplianceReport) -> Html {
  let short = report.short_rows();
  if short.is_empty() {
    return html!();
  }
  html!(
    <div class="ds-oe-fc-summary">
      <Alert
        inline=true
        r#type={AlertType::Warning}
        title={format!(
          "{} of {} in-scope vocabulary rows fall short of full ODRL 2.2",
          short.len(),
          report.rows_in_scope
        )}
      >
        <p>
          { "Every one of these is a row this study already documents as " }<em>{ "partial" }</em>
          { " or " }<em>{ "not implemented" }</em>
          { ", so none of them contradicts anything. What this page adds is the specific answer that would \
             have to change, and the ODRL 2.2 clause that settles it:" }
        </p>
        <ul>
          { for short.iter().map(|row| {
              let probe = row.probes.iter().find(|p| p.outcome == SpecOutcome::FallsShort);
              html!(
                <li key={row.row.id.clone()}>
                  <strong>{ row.row.term.clone() }</strong>
                  if let Some(probe) = probe {
                    if let Some(ideal) = &probe.ideal {
                      { format!(
                          " — probe {}: this engine answers {}, full ODRL 2.2 requires {}",
                          probe.id,
                          probe.observed_decision.clone().unwrap_or_else(|| "—".to_string()),
                          ideal.decision
                      ) }
                    }
                  } else {
                    { " — a structural gap: the wire contract cannot pose the question at all, so there is \
                       no probe and no wrong answer, only a missing capability." }
                  }
                </li>
              )
          }) }
        </ul>
      </Alert>
    </div>
  )
}

/// The sign-off caveat, on the page itself rather than only in the README:
/// a reader must be able to find "these specific calls were judged one way
/// but the spec genuinely admits another reading" without archaeology.
fn signoff_section() -> Html {
  html!(
    <div class="ds-oe-fc-signoff">
      <Alert
        inline=true
        r#type={AlertType::Info}
        title={format!(
          "{} of these judgments are contested: the spec genuinely admits another reading",
          CONTESTED_READINGS.len()
        )}
      >
        <p>
          { "The live half of this page is measured — the engine really is driven, and its real answer is \
             what you see. The ideal half is a " }<em>{ "reading" }</em>
          { " of two W3C documents, and on the calls below that reading is contested rather than obvious. \
             Each one names the reading that shipped and the one it rejects, including what the row's \
             verdict would become under the alternative. Five of these fourteen probes were judged to fall \
             short; on the other nine, what is contested is whether they should have fallen short at all." }
        </p>
        <ExpandableSection
          toggle_text_hidden="Show the contested judgments"
          toggle_text_expanded="Hide the contested judgments"
        >
          <ul>
            { for CONTESTED_READINGS.iter().map(signoff_entry) }
          </ul>
        </ExpandableSection>
      </Alert>
    </div>
  )
}

fn signoff_entry(entry: &ContestedReading) -> Html {
  html!(
    <li key={format!("{}/{}", entry.row, entry.probe)}>
      <code class="ds-oe-fc-probe-id">{ entry.probe }</code>
      { " on " }<code class="ds-oe-fc-probe-id">{ entry.row }</code>
      <span class="ds-oe-fc-line">{ "Shipped: " }{ entry.shipped }</span>
      <span class="ds-oe-fc-line">{ "Rejected: " }{ entry.alternative }</span>
    </li>
  )
}

fn provenance(report: &FullComplianceReport) -> Html {
  html!(
    <>
      <p class="ds-oe-fc-provenance">
        <span>
          { format!(
            "Replayed {} probes against engine.wasm ({} bytes) in your browser in {:.0} ms, and judged the \
             {} of them that at least one in-scope row names. {} in-scope vocabulary rows: {} meet full \
             ODRL 2.2, {} fall short (including {} structural wire-contract gap), {} undetermined. The \
             other {} rows are documented out of scope and are not judged here at all.",
            report.probes_in_catalog,
            report.engine_bytes,
            report.elapsed_ms,
            report.probes_judged,
            report.rows_in_scope,
            report.rows_meets,
            report.rows_short_of_spec(),
            report.rows_structural_gap,
            report.rows_undetermined,
            report.rows_excluded,
          ) }
        </span>
      </p>
      <p class="ds-oe-fc-note">
        { "Catalog: " }<code>{ report.generated_by.clone() }</code>
        { ", fetched from " }<code>{ "compliance-data/latest-coverage.json" }</code>
        { " — the same artifact the Coverage page reads, judged against its " }<code>{ "ideal" }</code>
        { " field rather than its " }<code>{ "expect" }</code>{ " one. Vocabulary: " }
        <code>{ report.spec.clone() }</code>
        { ". Row statuses and their reasoning come from " }<code>{ report.source_analysis.clone() }</code>{ "." }
      </p>
    </>
  )
}

fn stat(value: u64, label: &str, modifier: &str) -> Html {
  html!(
    <div class="ds-oe-stat">
      <span class={format!("ds-oe-stat-value ds-oe-fc-stat-value {modifier}")}>{ value }</span>
      <span class="ds-oe-stat-label">{ label.to_string() }</span>
    </div>
  )
}

/// Three stat rows: what this study documents for the in-scope rows, what
/// this run made of them against the spec, and the probe-level grain.
///
/// The third row is where the page is most at risk of overclaiming, so it
/// is followed on the page by a sentence naming the real number of probes
/// that distinguish full compliance from documented compliance. Most
/// judged probes reach the spec-ideal answer already; quoting a bare
/// `N/136` would imply the whole catalog changed meaning, and it did not.
fn stat_rows(report: &FullComplianceReport) -> Html {
  html!(
    <>
      <div class="ds-oe-stats">
        { stat(report.rows_in_scope, "rows in scope", "is-total") }
        { stat(report.implemented_rows, "implemented", "is-implemented") }
        { stat(report.partial_rows, "partial", "is-partial") }
        { stat(report.not_implemented_rows, "not implemented", "is-not-implemented") }
      </div>
      <div class="ds-oe-stats">
        { stat(report.rows_meets, "rows meet full spec", "is-meets") }
        { stat(report.rows_falls_short, "rows fall short", "is-short") }
        { stat(report.rows_structural_gap, "structural wire gap", "is-structural") }
        { stat(report.rows_undetermined, "rows undetermined", "is-undetermined") }
      </div>
      <div class="ds-oe-stats">
        { stat(report.probes_judged, "probes judged", "is-total") }
        { stat(report.probes_meets, "probes meet full spec", "is-meets") }
        { stat(report.probes_falls_short, "probes fall short", "is-short") }
        { stat(report.probes_undetermined, "probes undetermined", "is-undetermined") }
      </div>
    </>
  )
}

fn status_filter_onchange(filter: &UseStateHandle<StatusFilter>, value: StatusFilter) -> Callback<()> {
  let filter = filter.clone();
  Callback::from(move |()| filter.set(value))
}

fn verdict_filter_onchange(filter: &UseStateHandle<VerdictFilter>, value: VerdictFilter) -> Callback<()> {
  let filter = filter.clone();
  Callback::from(move |()| filter.set(value))
}

struct Filters {
  status: UseStateHandle<StatusFilter>,
  verdict: UseStateHandle<VerdictFilter>,
  search: UseStateHandle<String>,
  shortfalls_only: UseStateHandle<bool>,
}

fn matches_filters(row: &RowJudgment, filters: &Filters, query: &str) -> bool {
  if !filters.status.matches(&row.row.status) {
    return false;
  }
  if !filters.verdict.matches(row.verdict) {
    return false;
  }
  if *filters.shortfalls_only && !row.verdict.is_short_of_spec() {
    return false;
  }
  if query.is_empty() {
    return true;
  }
  row.row.term.to_lowercase().contains(query)
    || row.row.id.to_lowercase().contains(query)
    || row.row.why.to_lowercase().contains(query)
    || row.probes.iter().any(|probe| probe.id.to_lowercase().contains(query))
}

fn results(report: &FullComplianceReport, filters: &Filters) -> Html {
  let query = filters.search.trim().to_lowercase();
  let visible: Vec<&RowJudgment> = report.rows.iter().filter(|row| matches_filters(row, filters, &query)).collect();

  let on_search = {
    let search = filters.search.clone();
    Callback::from(move |value: String| search.set(value))
  };
  let on_shortfalls_only = {
    let shortfalls_only = filters.shortfalls_only.clone();
    Callback::from(move |value: bool| shortfalls_only.set(value))
  };

  html!(
    <>
      <style>{ STAT_ROW_CSS }</style>
      { stat_rows(report) }
      <p class="ds-oe-fc-legend">
        { format!(
          "Read the probe row honestly: only {} of the {} probes judged here distinguish this page's \
           question from the Coverage page's. The other {} already produce the answer full ODRL 2.2 \
           requires, so they would look identical on either page. ",
          report.probes_falls_short, report.probes_judged, report.probes_meets
        ) }
        { "And nothing on this page is red: falling short of the full spec is what a " }
        <em>{ "partial" }</em>{ " status " }<em>{ "means" }</em>
        { ", not a failure. Red is reserved for the Coverage page, where it means the engine and its own \
           documentation disagree." }
      </p>

      <div class="ds-oe-fc-toolbar">
        <div class="ds-oe-fc-search">
          <TextInput placeholder="Search term, row id or probe id..." value={(*filters.search).clone()} onchange={on_search} />
        </div>
        <ToggleGroup>
          <ToggleGroupItem
            text={format!("All ({})", report.rows_in_scope)}
            selected={*filters.status == StatusFilter::All}
            onchange={status_filter_onchange(&filters.status, StatusFilter::All)}
          />
          <ToggleGroupItem
            text={format!("Implemented ({})", report.implemented_rows)}
            selected={*filters.status == StatusFilter::Implemented}
            onchange={status_filter_onchange(&filters.status, StatusFilter::Implemented)}
          />
          <ToggleGroupItem
            text={format!("Partial ({})", report.partial_rows)}
            selected={*filters.status == StatusFilter::Partial}
            onchange={status_filter_onchange(&filters.status, StatusFilter::Partial)}
          />
          <ToggleGroupItem
            text={format!("Not implemented ({})", report.not_implemented_rows)}
            selected={*filters.status == StatusFilter::NotImplemented}
            onchange={status_filter_onchange(&filters.status, StatusFilter::NotImplemented)}
          />
        </ToggleGroup>
        <ToggleGroup>
          <ToggleGroupItem
            text="Any outcome"
            selected={*filters.verdict == VerdictFilter::All}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::All)}
          />
          <ToggleGroupItem
            text={format!("Meets full spec ({})", report.rows_meets)}
            selected={*filters.verdict == VerdictFilter::Meets}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::Meets)}
          />
          <ToggleGroupItem
            text={format!("Falls short ({})", report.rows_short_of_spec())}
            selected={*filters.verdict == VerdictFilter::Short}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::Short)}
          />
        </ToggleGroup>
        <Switch
          label="Shortfalls only"
          checked={*filters.shortfalls_only}
          onchange={on_shortfalls_only}
        />
      </div>
      <p class="ds-oe-fc-count">
        { format!("Showing {} of {} in-scope vocabulary rows.", visible.len(), report.rows_in_scope) }
      </p>

      if visible.is_empty() {
        <div class="ds-oe-fc-empty">{ "No vocabulary rows match this search/filter." }</div>
      }
      { for report.categories.iter().map(|category| {
          let rows: Vec<&RowJudgment> =
            visible.iter().copied().filter(|row| row.row.category == category.id).collect();
          category_section(category, rows)
      }) }
    </>
  )
}

/// The Full ODRL 2.2 Compliance page: 45 in-scope vocabulary rows, 130
/// judged probes, and a per-row verdict against the spec computed in this
/// browser.
#[component]
pub fn FullCompliancePage() -> Html {
  let state = use_state(|| RunState::LoadingWasm);
  let run_token = use_state(|| 0u32);

  {
    let state = state.clone();
    use_effect_with(*run_token, move |_| {
      spawn_local(run(state));
      || ()
    });
  }

  let on_rerun = {
    let run_token = run_token.clone();
    Callback::from(move |_: MouseEvent| run_token.set(*run_token + 1))
  };

  let filters = Filters {
    status: use_state(|| StatusFilter::All),
    verdict: use_state(|| VerdictFilter::All),
    search: use_state(String::new),
    shortfalls_only: use_state(|| false),
  };

  html!(
    <>
      <style>{ FULL_COMPLIANCE_CSS }</style>
      <Content>
        <Title level={Level::H1}>{ "Full ODRL 2.2 Compliance" }</Title>
        <p>
          { "The " }<strong>{ "ODRL 2.2 Coverage" }</strong>{ " page asks: " }
          <em>{ "does this engine match what this study documents about it?" }</em>
          { " This page asks a different question, of the same engine and the same requests: " }
          <em>{ "assuming the engine " }<strong>{ "should" }</strong>{ " fully implement the ODRL 2.2 \
             specification for every row that is not structurally out of scope, does its real behaviour \
             meet that ideal?" }</em>
        </p>
        <p>
          { "The two questions come apart because most of this engine's gaps are " }<em>{ "documented" }</em>
          { ". A row recorded as " }<em>{ "partial" }</em>{ " has a probe whose expected outcome is the \
             narrowing itself, so the Coverage page reports it as agreeing with its documentation — \
             correctly, and that page's red would be wrong there. This page ignores the documentation and \
             judges the same live answer against the " }
          <a href="https://www.w3.org/TR/odrl-vocab/" target="_blank" rel="noopener noreferrer">
            { "W3C ODRL 2.2 Vocabulary & Expression" }
          </a>
          { " and the " }
          <a href="https://www.w3.org/TR/odrl-model/" target="_blank" rel="noopener noreferrer">
            { "Information Model" }
          </a>
          { ", quoting the clause that settles each call." }
        </p>
        <p>
          { "The probes and the replay are shared with the Coverage page — the same " }
          <code>{ "engine.wasm" }</code>{ ", the same " }<code>{ "alloc" }</code>{ "/" }
          <code>{ "evaluate" }</code>{ "/" }<code>{ "dealloc" }</code>
          { " C ABI, the same request bytes, driven here in your browser right now — so any difference \
             between the two pages is a difference of question, never of execution. What differs is the \
             target: the Coverage page judges each response against what this study says the engine does; \
             this page judges it against what ODRL 2.2 requires." }
        </p>
        <p>
          { "Scope, and what each outcome means. The seven rows documented " }<em>{ "out of scope" }</em>
          { " are excluded outright: they name profile-extension points and " }<code>{ "odrl:hasPolicy" }</code>
          { ", which sit outside the wire contract entirely, and being outside the contract is not the same \
             thing as falling short of the spec. The eleven documented " }<em>{ "implemented" }</em>
          { " meet full spec trivially. The remaining thirty-four are the ones actually researched — and \
             not all of them fall short: a row is partial for one " }<em>{ "specific" }</em>
          { " reason, and twenty of the thirty-four reach the spec-ideal answer on every probe they \
             currently carry, which those rows say plainly rather than being quietly counted as failures." }
        </p>
        <p>
          { "One row falls short with no probe at all. " }<code>{ "party.collections" }</code>
          { " needs a wire-contract addition before any request can even pose the question, so it is judged \
             at row level, rendered as prose, and marked as a structural gap rather than a wrong answer." }
        </p>
      </Content>

      { signoff_section() }

      if let RunState::Done(report) = &*state {
        { shortfall_alert(report) }
        { provenance(report) }
        { rerun_button(on_rerun) }
        { results(report, &filters) }
      } else {
        { run_panel(&state, on_rerun) }
      }

      { case_study_credit() }
    </>
  )
}
