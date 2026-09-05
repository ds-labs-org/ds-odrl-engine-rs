//! The Compliance Results page. It no longer *displays* a verdict
//! somebody else committed -- it computes one, here, now, in the
//! visitor's own browser.
//!
//! On mount it drives four stages (see `compliance_run.rs`): fetch and
//! instantiate the real compiled `engine.wasm`; fetch the corpus of
//! per-case Section 5.2 requests `compliance-runner` exported to
//! `compliance/reports/latest-cases.json`; run every one of them through
//! `engine.wasm`'s raw `alloc`/`evaluate`/`dealloc` C ABI, comparing each
//! answer against the vendored suite's own expected decision; then tally.
//!
//! `compliance-data/latest.json` -- what this page used to render
//! outright -- keeps its `copy-file` and is still fetched, but in a
//! better job: it is now the *native* run's recorded baseline, and stage
//! four cross-checks the live tally against it. Same corpus, same engine
//! source, one run through `engine::evaluate_request` natively and one
//! through the compiled `engine.wasm` ABI in a browser; a divergence is a
//! real cross-host finding (or the signal that someone regenerated one
//! artifact and not the other, which `compliance_cases.rs`'s own tests
//! also catch at `cargo test` time).
//!
//! Scope honesty, stated on the page itself as well as here: the live run
//! proves that the real compiled `engine.wasm`, instantiated in this
//! browser and driven over its four-export ABI, returns the suite's
//! expected decision for every translated request. It does **not**
//! re-derive the Turtle -> `Request` translation (`translate.rs`) or the
//! `report:*` -> Allow/Deny ground truth in-browser; both were computed
//! natively and travel inside the artifact.

use crate::compliance_cases::{CaseOutcome, CaseStatus, LiveReport};
use crate::compliance_run::{RunProgress, RunState, Stage, run};
use crate::pages::{STAT_ROW_CSS, case_study_credit, stat_row_html};
use patternfly_yew::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusFilter {
  All,
  Passed,
  Failed,
  Skipped,
  Errored,
}

impl StatusFilter {
  fn matches(self, status: CaseStatus) -> bool {
    match self {
      StatusFilter::All => true,
      StatusFilter::Passed => status == CaseStatus::Passed,
      StatusFilter::Failed => status == CaseStatus::Failed,
      StatusFilter::Skipped => status == CaseStatus::Skipped,
      StatusFilter::Errored => status == CaseStatus::Errored,
    }
  }
}

/// This page's own layout CSS: the run banner, the search/filter toolbar
/// and the table's slug/detail cell styling. Kept page-scoped the same
/// way `pages.rs`'s `HOME_CSS` is -- an inline `<style>` tag, `ds-oe-`-
/// prefixed classes -- rather than added to `assets/theme.css`, since
/// only this page uses it.
const COMPLIANCE_CSS: &str = r#"
.ds-oe-compliance-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 1rem;
  margin: 0.25rem 0 0.5rem;
}
.ds-oe-compliance-search { max-width: 24rem; flex: 1 1 18rem; }
.ds-oe-compliance-count {
  margin: 0 0 0.75rem;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-compliance-table-wrap { overflow-x: auto; }
.ds-oe-compliance-slug {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.85rem;
  white-space: nowrap;
}
.ds-oe-compliance-detail {
  display: block;
  max-width: 34rem;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-compliance-empty {
  padding: 1.5rem;
  text-align: center;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-run-panel {
  margin: 1rem 0 1.25rem;
  padding: 1rem 1.25rem;
  border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2);
  border-radius: 0.35rem;
}
.ds-oe-run-bar { max-width: 32rem; margin-top: 1rem; }
.ds-oe-run-provenance {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.75rem;
  margin: 0.75rem 0 0.25rem;
  font-size: 0.9rem;
}
.ds-oe-run-provenance code { font-size: 0.85em; }
.ds-oe-run-note {
  margin: 0.35rem 0 0;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
"#;

fn status_label(status: CaseStatus) -> Html {
  let (color, text) = match status {
    CaseStatus::Passed => (Color::Green, "Passed"),
    CaseStatus::Failed => (Color::Red, "Failed"),
    CaseStatus::Skipped => (Color::Grey, "Skipped"),
    CaseStatus::Errored => (Color::Orange, "Errored"),
  };
  html!(<Label label={text.to_string()} color={color} compact=true />)
}

/// Per-case detail. Strictly richer than the static file this page used
/// to render: `latest.json` carries a `reason` only for failed and
/// skipped cases, while a live run gets the engine's own reason for
/// *every* evaluated case -- so a passing row can now show what actually
/// matched, not just that something did.
fn detail_html(outcome: &CaseOutcome) -> Html {
  let reason = outcome.reason.clone().unwrap_or_default();
  match outcome.status {
    CaseStatus::Passed => html!(
      <span class="ds-oe-compliance-detail">
        { outcome.actual.as_deref().map(|d| format!("decision: {d}")).unwrap_or_default() }
        if !reason.is_empty() { { format!(" — {reason}") } }
      </span>
    ),
    CaseStatus::Failed | CaseStatus::Errored => {
      let expected = outcome.expected.clone().unwrap_or_else(|| "?".to_string());
      let actual = outcome.actual.clone().unwrap_or_else(|| "?".to_string());
      html!(
        <span class="ds-oe-compliance-detail">
          <strong>{ format!("expected {expected}, actual {actual}") }</strong>
          if !reason.is_empty() { { format!(" — {reason}") } }
        </span>
      )
    }
    CaseStatus::Skipped => html!(<span class="ds-oe-compliance-detail">{ reason }</span>),
  }
}

/// `ToggleGroup` requires `ChildrenWithProps<ToggleGroupItem>` -- a
/// literal child type it checks at macro-expansion time -- so each item
/// has to be a real `<ToggleGroupItem>` tag rather than a wrapper
/// component; this just builds the one piece (its `onchange`) that
/// differs per filter value, so the five call sites below stay a
/// one-liner each.
fn filter_onchange(filter: &UseStateHandle<StatusFilter>, value: StatusFilter) -> Callback<()> {
  let filter = filter.clone();
  Callback::from(move |()| filter.set(value))
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

/// The "Performing tests" step's live description -- the one place where
/// a number changes several times per run.
fn evaluating_description(state: &RunState) -> Option<String> {
  let progress = state.progress()?;
  let mut text = format!("{} / {} evaluated — {} passed, {} failed", progress.done, progress.total, progress.passed, progress.failed);
  if progress.errored > 0 {
    text.push_str(&format!(", {} errored", progress.errored));
  }
  if progress.skipped > 0 {
    text.push_str(&format!(", {} skipped", progress.skipped));
  }
  Some(text)
}

/// `ProgressStepper` takes a `ChildrenRenderer<ProgressStepperChildVariant>`,
/// i.e. its children must be literal `<ProgressStepperStep>` tags -- the
/// same macro-expansion constraint `ToggleGroup` imposes above -- so the
/// four steps are written out rather than generated from `Stage::ALL` in
/// a loop. `Stage::ALL` still drives their *labels* and statuses, so the
/// ordering lives in one place.
fn stepper(state: &RunState) -> Html {
  let [loading_wasm, loading_cases, evaluating, compiling] = Stage::ALL;
  html!(
    <ProgressStepper>
      <ProgressStepperStep
        status={step_status(state, loading_wasm)}
        is_current={state.current_stage() == Some(loading_wasm)}
      >
        <span>{ loading_wasm.label() }</span>
      </ProgressStepperStep>
      <ProgressStepperStep
        status={step_status(state, loading_cases)}
        is_current={state.current_stage() == Some(loading_cases)}
      >
        <span>{ loading_cases.label() }</span>
      </ProgressStepperStep>
      <ProgressStepperStep
        status={step_status(state, evaluating)}
        is_current={state.current_stage() == Some(evaluating)}
        description={evaluating_description(state)}
      >
        <span>{ evaluating.label() }</span>
      </ProgressStepperStep>
      <ProgressStepperStep
        status={step_status(state, compiling)}
        is_current={state.current_stage() == Some(compiling)}
      >
        <span>{ compiling.label() }</span>
      </ProgressStepperStep>
    </ProgressStepper>
  )
}

fn progress_bar(progress: &RunProgress) -> Html {
  let total = progress.total.max(1) as f64;
  html!(
    <div class="ds-oe-run-bar">
      <Progress
        value={progress.done as f64}
        range={0f64..total}
        value_text={format!("{} / {}", progress.done, progress.total)}
      />
    </div>
  )
}

/// The run panel, shown while a run is in flight and after a *failed*
/// one: the four-step stepper, the live progress bar, and -- on failure
/// -- which stage broke and its raw error text, with the later steps left
/// `Pending` rather than spinning forever. A finished run replaces this
/// with one line of provenance instead (see [`provenance`]); the stepper
/// has nothing left to say once every step is green.
fn run_panel(state: &RunState, on_rerun: Callback<MouseEvent>) -> Html {
  html!(
    <div class="ds-oe-run-panel">
      { stepper(state) }
      if let Some(progress) = state.progress() {
        { progress_bar(progress) }
      }
      if let RunState::Failed { stage, message } = state {
        <div class="ds-oe-run-bar">
          <Alert inline=true r#type={AlertType::Danger} title={format!("{} failed", stage.label())}>
            <p>{ message.clone() }</p>
          </Alert>
        </div>
        { rerun_button(on_rerun) }
      }
    </div>
  )
}

/// Re-running does not re-fetch `engine.wasm` (the instance is cached in
/// `engine_bridge`'s thread-local), so this genuinely re-executes every
/// case against the already-loaded module, in milliseconds, on demand.
fn rerun_button(on_rerun: Callback<MouseEvent>) -> Html {
  html!(
    <p class="ds-oe-run-note">
      <Button variant={ButtonVariant::Secondary} onclick={on_rerun}>{ "Re-run in this browser" }</Button>
    </p>
  )
}

/// One line of provenance for a finished run: what was executed, where,
/// how long it took, and whether it agrees with the native run recorded
/// in `compliance/reports/latest.json`.
fn provenance(report: &LiveReport) -> Html {
  html!(
    <>
      <p class="ds-oe-run-provenance">
        <span>
          { format!(
            "Ran {} cases against engine.wasm ({} bytes) in your browser in {:.0} ms — {} passed, {} failed, {} errored, {} skipped.",
            report.total, report.engine_bytes, report.elapsed_ms, report.passed, report.failed, report.errored, report.skipped
          ) }
        </span>
      </p>
      <p class="ds-oe-run-note">
        { "Corpus: " }<code>{ report.suite.clone() }</code>
        { ", fetched from " }<code>{ "compliance-data/latest-cases.json" }</code>
        { " — the artifact this run actually read, named here so a browser-cached copy is visible rather than silent." }
      </p>
      { baseline_note(report) }
    </>
  )
}

fn baseline_note(report: &LiveReport) -> Html {
  match &report.baseline {
    None => html!(
      <p class="ds-oe-run-note">
        { "The native run's recorded baseline (" }<code>{ "compliance-data/latest.json" }</code>
        { ") could not be loaded, so no native-vs-wasm cross-check was made. The live numbers above stand on their own." }
      </p>
    ),
    Some(comparison) if comparison.matches() => html!(
      <p class="ds-oe-run-note">
        { format!(
          "Matches the native compliance-runner run recorded in compliance/reports/latest.json ({} total, {} passed, {} failed, {} skipped), case for case.",
          comparison.total, comparison.passed, comparison.failed, comparison.skipped
        ) }
      </p>
    ),
    Some(comparison) => html!(
      <div class="ds-oe-run-bar">
        <Alert
          inline=true
          r#type={AlertType::Warning}
          title={format!("{} case(s) disagree with the native run", comparison.divergences.len())}
        >
          <p>
            { "This browser's run over " }<code>{ "engine.wasm" }</code>
            { " and the native run recorded in " }<code>{ "compliance/reports/latest.json" }</code>
            { " reached different verdicts (native / live):" }
          </p>
          <ul>
            { for comparison.divergences.iter().map(|d| html!(
              <li key={d.slug.clone()}>
                <code>{ d.slug.clone() }</code>{ format!(" — {} / {}", d.native, d.live) }
              </li>
            )) }
          </ul>
        </Alert>
      </div>
    ),
  }
}

fn results_table(report: &LiveReport, filter: &UseStateHandle<StatusFilter>, search: &UseStateHandle<String>) -> Html {
  let query = search.trim().to_lowercase();
  let filtered: Vec<&CaseOutcome> = report
    .outcomes
    .iter()
    .filter(|outcome| filter.matches(outcome.status))
    .filter(|outcome| {
      query.is_empty() || outcome.slug.to_lowercase().contains(&query) || outcome.title.to_lowercase().contains(&query)
    })
    .collect();

  let on_search = {
    let search = search.clone();
    Callback::from(move |value: String| search.set(value))
  };

  html!(
    <>
      <style>{ STAT_ROW_CSS }</style>
      { stat_row_html(report.total, report.passed, report.failed, report.skipped) }

      <div class="ds-oe-compliance-toolbar">
        <div class="ds-oe-compliance-search">
          <TextInput placeholder="Search by slug or title..." value={(**search).clone()} onchange={on_search} />
        </div>
        <ToggleGroup>
          <ToggleGroupItem
            text={format!("All ({})", report.total)}
            selected={**filter == StatusFilter::All}
            onchange={filter_onchange(filter, StatusFilter::All)}
          />
          <ToggleGroupItem
            text={format!("Passed ({})", report.passed)}
            selected={**filter == StatusFilter::Passed}
            onchange={filter_onchange(filter, StatusFilter::Passed)}
          />
          <ToggleGroupItem
            text={format!("Failed ({})", report.failed)}
            selected={**filter == StatusFilter::Failed}
            onchange={filter_onchange(filter, StatusFilter::Failed)}
          />
          <ToggleGroupItem
            text={format!("Skipped ({})", report.skipped)}
            selected={**filter == StatusFilter::Skipped}
            onchange={filter_onchange(filter, StatusFilter::Skipped)}
          />
          <ToggleGroupItem
            text={format!("Errored ({})", report.errored)}
            selected={**filter == StatusFilter::Errored}
            onchange={filter_onchange(filter, StatusFilter::Errored)}
          />
        </ToggleGroup>
      </div>
      <p class="ds-oe-compliance-count">
        { format!("Showing {} of {} cases.", filtered.len(), report.outcomes.len()) }
      </p>

      <div class="ds-oe-compliance-table-wrap">
        <table class="pf-v6-c-table" role="grid">
          <thead>
            <tr role="row">
              <th role="columnheader">{ "Slug" }</th>
              <th role="columnheader">{ "Title" }</th>
              <th role="columnheader">{ "Status" }</th>
              <th role="columnheader">{ "Details" }</th>
            </tr>
          </thead>
          <tbody>
            { for filtered.iter().map(|outcome| html!(
              <tr role="row" key={outcome.slug.clone()}>
                <td role="cell"><span class="ds-oe-compliance-slug">{ outcome.slug.clone() }</span></td>
                <td role="cell">{ outcome.title.clone() }</td>
                <td role="cell">{ status_label(outcome.status) }</td>
                <td role="cell">{ detail_html(outcome) }</td>
              </tr>
            )) }
            if filtered.is_empty() {
              <tr role="row">
                <td role="cell" colspan="4">
                  <div class="ds-oe-compliance-empty">{ "No cases match this search/filter." }</div>
                </td>
              </tr>
            }
          </tbody>
        </table>
      </div>
    </>
  )
}

/// The real Compliance Results page: a live, in-browser run of the whole
/// vendored corpus against the compiled `engine.wasm`, then the same
/// searchable/filterable table over what *this* run produced.
#[component]
pub fn CompliancePage() -> Html {
  let state = use_state(|| RunState::LoadingWasm);
  // Bumped by the Re-run button; the effect below re-runs the whole
  // sequence whenever it changes (and once, on mount, at 0).
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

  let filter = use_state(|| StatusFilter::All);
  let search = use_state(String::new);

  html!(
    <>
      <style>{ COMPLIANCE_CSS }</style>
      <Content>
        <Title level={Level::H1}>{ "Compliance Results" }</Title>
        <p>
          { "Every case from the vendored " }
          <a href="https://github.com/SolidLabResearch/ODRL-Test-Suite" target="_blank" rel="noopener noreferrer">{ "SolidLabResearch/ODRL-Test-Suite" }</a>
          { ", run " }<strong>{ "right now, in this browser" }</strong>{ ", against the real compiled " }
          <code>{ "engine.wasm" }</code>{ " over its raw " }<code>{ "alloc" }</code>{ "/" }
          <code>{ "evaluate" }</code>{ "/" }<code>{ "dealloc" }</code>
          { " C ABI — the same way any independent JS or JVM host would drive it. The numbers below are \
             computed here, not read from a committed file: this page fetches " }
          <code>{ "compliance-data/latest-cases.json" }</code>
          { " (the per-case Section 5.2 requests " }<code>{ "compliance-runner" }</code>
          { " exported) and evaluates each one itself." }
        </p>
        <p>
          { "What this does " }<em>{ "not" }</em>{ " re-derive in your browser: the vendored suite's \
             Turtle-to-request translation (" }<code>{ "compliance-runner/src/translate.rs" }</code>
          { ") and its " }<code>{ "report:*" }</code>{ " expected-decision ground truth (" }
          <code>{ "ground_truth.rs" }</code>
          { ") were both computed natively and travel inside the fetched artifact. The live run proves the \
             engine and its ABI across a real host boundary, not the adapter that produced the corpus. The \
             native run's own recorded verdicts (" }<code>{ "compliance/reports/latest.json" }</code>
          { ") are fetched too, purely as a baseline to cross-check this run against. See the full generated \
             report as Markdown at " }
          <a href="https://github.com/ds-labs-org/ds-odrl-engine-rs/blob/main/compliance/reports/latest.md" target="_blank" rel="noopener noreferrer">
            { "compliance/reports/latest.md" }
          </a>
          { "." }
        </p>
      </Content>

      if let RunState::Done(report) = &*state {
        { provenance(report) }
        { rerun_button(on_rerun) }
        { results_table(report, &filter, &search) }
      } else {
        { run_panel(&state, on_rerun) }
      }

      { case_study_credit() }
    </>
  )
}
