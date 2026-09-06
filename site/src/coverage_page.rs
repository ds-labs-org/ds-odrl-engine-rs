//! The ODRL 2.2 Coverage page.
//!
//! A hand-written coverage table tells you what somebody believed about
//! an engine when they last edited the table. This page does something
//! else: it fetches 115 exact Section 5.2 requests, runs every one of them
//! through the real compiled `engine.wasm` over its
//! `alloc`/`evaluate`/`dealloc` C ABI in the visitor's own browser, and
//! then reports, per vocabulary row, whether what the engine *just did*
//! agrees with the status this study documents for that row.
//!
//! Three things follow from that, and the page says all three out loud:
//!
//! * A row can come back **Contradicted** — the documented status and the
//!   live engine disagree. That is the only thing on this page rendered in
//!   red, and it is a finding about the documentation as much as about the
//!   engine.
//! * "Not implemented" is **not** a failure. It is an honest, deliberate
//!   gap, and it is verified live exactly like everything else: the probe
//!   for a missing feature shows the feature not firing.
//! * Three of the 52 rows carry no probe at all, and say why. Their claims
//!   are about native tooling or about a pre-request concern that no
//!   request can encode, so no browser run could establish them.
//!
//! Scope honesty, stated on the page as well as here: the probe
//! *requests* were authored natively by `coverage-probes` and travel
//! inside the fetched artifact. Every *decision* on this page is computed
//! here, now, by the engine.

use crate::coverage_catalog::{
  status_display, Category, CoverageReport, ProbeOutcome, ProbeStatus, RowOutcome, RowVerdict,
};
use crate::coverage_run::run;
use crate::coverage_state::{CoverageProgress, RunState, Stage};
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
  OutOfScope,
}

impl StatusFilter {
  fn matches(self, status: &str) -> bool {
    match self {
      StatusFilter::All => true,
      StatusFilter::Implemented => status == "Implemented",
      StatusFilter::Partial => status == "Partial",
      StatusFilter::NotImplemented => status == "NotImplemented",
      StatusFilter::OutOfScope => status == "OutOfScope",
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VerdictFilter {
  All,
  Verified,
  Contradicted,
  Inconclusive,
  Documented,
}

impl VerdictFilter {
  fn matches(self, verdict: RowVerdict) -> bool {
    match self {
      VerdictFilter::All => true,
      VerdictFilter::Verified => verdict == RowVerdict::Verified,
      VerdictFilter::Contradicted => verdict == RowVerdict::Contradicted,
      VerdictFilter::Inconclusive => verdict == RowVerdict::Inconclusive,
      VerdictFilter::Documented => verdict == RowVerdict::Documented,
    }
  }
}

/// This page's own layout CSS, page-scoped as an inline `<style>` with
/// `ds-oe-cov-`-prefixed classes -- the same choice `COMPLIANCE_CSS` and
/// `HOME_CSS` already make, rather than growing `assets/theme.css` for one
/// page.
///
/// The contradicted-row treatment follows the existing convention of a
/// PatternFly 6 design token with a literal hex fallback, so it degrades
/// to something legible if the token is ever renamed upstream.
const COVERAGE_CSS: &str = r#"
.ds-oe-cov-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 1rem;
  margin: 0.5rem 0 0.5rem;
}
.ds-oe-cov-search { max-width: 24rem; flex: 1 1 18rem; }
.ds-oe-cov-count {
  margin: 0 0 0.75rem;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-legend {
  margin: 0.5rem 0 0;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-section { margin-top: 2rem; }
.ds-oe-cov-specref {
  margin: 0.1rem 0 0.6rem;
  font-size: 0.8rem;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-table-wrap { overflow-x: auto; }
.ds-oe-cov-term {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.85rem;
  max-width: 22rem;
  display: block;
}
.ds-oe-cov-sub {
  display: block;
  margin-top: 0.2rem;
  max-width: 26rem;
  font-size: 0.8rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-sub.is-caveat { font-style: italic; }
.ds-oe-cov-observed {
  display: block;
  max-width: 34rem;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-observed code { font-size: 0.95em; }
.ds-oe-cov-row.is-contradicted {
  border-left: 4px solid var(--pf-t--global--border--color--status--danger--default, #c9190b);
  background: var(--pf-t--global--background--color--status--danger--default, #faeae8);
}
.ds-oe-cov-probe {
  margin: 0 0 0.85rem;
  font-size: 0.85rem;
}
.ds-oe-cov-probe:last-child { margin-bottom: 0; }
.ds-oe-cov-probe-id {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.85em;
}
.ds-oe-cov-probe-line {
  display: block;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-empty {
  padding: 1.5rem;
  text-align: center;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-run-panel {
  margin: 1rem 0 1.25rem;
  padding: 1rem 1.25rem;
  border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2);
  border-radius: 0.35rem;
}
.ds-oe-cov-bar { max-width: 32rem; margin-top: 1rem; }
.ds-oe-cov-provenance {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.75rem;
  margin: 0.75rem 0 0.25rem;
  font-size: 0.9rem;
}
.ds-oe-cov-note {
  margin: 0.35rem 0 0;
  font-size: 0.85rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-cov-stat-value.is-implemented { color: #3e8635; }
.ds-oe-cov-stat-value.is-partial { color: #b98412; }
.ds-oe-cov-stat-value.is-not-implemented { color: #c46100; }
.ds-oe-cov-stat-value.is-out-of-scope { color: #8a8d90; }
.ds-oe-cov-stat-value.is-verified { color: #005f60; }
.ds-oe-cov-stat-value.is-contradicted { color: #c9190b; }
.ds-oe-cov-stat-value.is-inconclusive { color: #c46100; }
.ds-oe-cov-stat-value.is-documented { color: #0066cc; }
"#;

/// The documented status a row carries, as a PatternFly label.
///
/// **The colour rule this page turns on:** "Not implemented" is Orange,
/// not Red. It is a documented, honest gap, not a failure. `Color::Red`
/// appears in exactly one place on this page — a live probe disagreeing
/// with a documented status — so red always means the same thing here.
fn status_label(status: &str) -> Html {
  let color = match status {
    "Implemented" => Color::Green,
    "Partial" => Color::Yellow,
    "NotImplemented" => Color::Orange,
    "OutOfScope" => Color::Grey,
    _ => Color::Purple,
  };
  html!(<Label label={status_display(status).to_string()} color={color} compact=true />)
}

fn verdict_label(verdict: RowVerdict) -> Html {
  let (color, icon) = match verdict {
    RowVerdict::Verified => (Color::Teal, Icon::CheckCircle),
    RowVerdict::Contradicted => (Color::Red, Icon::ExclamationCircle),
    RowVerdict::Inconclusive => (Color::Orange, Icon::ExclamationTriangle),
    RowVerdict::Documented => (Color::Blue, Icon::InfoCircle),
  };
  html!(<Label label={verdict.label().to_string()} color={color} icon={icon} compact=true />)
}

/// One line naming what the engine actually did for this row, just now.
///
/// This is where the page earns its existence: a hand-authored coverage
/// table cannot show you the sentence the engine produced forty
/// milliseconds ago.
fn observed_html(row: &RowOutcome) -> Html {
  match row.verdict {
    RowVerdict::Documented => html!(<span class="ds-oe-cov-observed">{ "—" }</span>),
    RowVerdict::Verified => {
      let first = row.probes.first();
      let decision = first.and_then(|p| p.decision.clone()).unwrap_or_else(|| "?".to_string());
      let reason = first.and_then(|p| p.reason.clone()).unwrap_or_default();
      html!(
        <span class="ds-oe-cov-observed">
          <strong>{ decision }</strong>
          if !reason.is_empty() { { format!(" — {reason}") } }
        </span>
      )
    }
    RowVerdict::Contradicted => {
      let probe = row.probes.iter().find(|p| p.status == ProbeStatus::Disagreed);
      let mismatch = probe.and_then(|p| p.mismatch.clone()).unwrap_or_else(|| "disagreed".to_string());
      let reason = probe.and_then(|p| p.reason.clone()).unwrap_or_default();
      let id = probe.map(|p| p.id.clone()).unwrap_or_default();
      html!(
        <span class="ds-oe-cov-observed">
          <strong>{ mismatch }</strong>
          if !reason.is_empty() { { format!(" — {reason}") } }
          <span class="ds-oe-cov-probe-line">{ "probe " }<code>{ id }</code></span>
        </span>
      )
    }
    RowVerdict::Inconclusive => {
      let probe = row.probes.iter().find(|p| p.status == ProbeStatus::Errored);
      let reason = probe.and_then(|p| p.reason.clone()).unwrap_or_else(|| "no detail".to_string());
      let id = probe.map(|p| p.id.clone()).unwrap_or_default();
      html!(
        <span class="ds-oe-cov-observed">
          <strong>{ "could not be judged" }</strong>{ format!(" — {reason}") }
          <span class="ds-oe-cov-probe-line">{ "probe " }<code>{ id }</code></span>
        </span>
      )
    }
  }
}

fn probe_detail(probe: &ProbeOutcome) -> Html {
  let observed = probe.decision.clone().unwrap_or_else(|| "—".to_string());
  let reason = probe.reason.clone().unwrap_or_default();
  html!(
    <div class="ds-oe-cov-probe" key={probe.id.clone()}>
      <div>
        <code class="ds-oe-cov-probe-id">{ probe.id.clone() }</code>
        { " " }
        <Label label={probe.kind.clone()} color={if probe.kind == "negative" { Color::Purple } else { Color::Blue }} compact=true outline=true />
        { " " }
        { probe.title.clone() }
      </div>
      <span class="ds-oe-cov-probe-line">{ probe.asserts.clone() }</span>
      <span class="ds-oe-cov-probe-line">{ format!("Falsified by: {}", probe.falsified_by) }</span>
      <span class="ds-oe-cov-probe-line">
        { format!("expected {} · observed {observed}", probe.expected_decision) }
        if let Some(mismatch) = &probe.mismatch { { format!(" · {mismatch}") } }
      </span>
      if !reason.is_empty() {
        <span class="ds-oe-cov-probe-line">{ "engine reason: " }<code>{ reason }</code></span>
      }
    </div>
  )
}

fn row_html(row: &RowOutcome) -> Html {
  let contradicted = row.verdict == RowVerdict::Contradicted;
  let class = if contradicted { "ds-oe-cov-row is-contradicted" } else { "ds-oe-cov-row" };

  let verification_note = match row.verdict {
    RowVerdict::Documented => row.row.documented_because.clone(),
    _ => Some(format!(
      "{} probe{}",
      row.probes.len(),
      if row.probes.len() == 1 { "" } else { "s" }
    )),
  };

  html!(
    <tr role="row" class={class} key={row.row.id.clone()}>
      <td role="cell">
        <span class="ds-oe-cov-term">{ row.row.term.clone() }</span>
        <span class="ds-oe-cov-sub">{ row.row.why.clone() }</span>
        <span class="ds-oe-cov-sub"><code>{ row.row.evidence.clone() }</code></span>
      </td>
      <td role="cell">{ status_label(&row.row.status) }</td>
      <td role="cell">
        { verdict_label(row.verdict) }
        if let Some(note) = verification_note { <span class="ds-oe-cov-sub">{ note }</span> }
        if let Some(caveat) = &row.row.caveat {
          <span class="ds-oe-cov-sub is-caveat">{ caveat.clone() }</span>
        }
      </td>
      <td role="cell">
        { observed_html(row) }
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

fn category_section(category: &Category, rows: Vec<&RowOutcome>) -> Html {
  if rows.is_empty() {
    return html!();
  }
  html!(
    <div class="ds-oe-cov-section" key={category.id.clone()}>
      <Title level={Level::H2}>{ format!("{}. {}", category.number, category.title) }</Title>
      <p class="ds-oe-cov-specref">{ category.spec_ref.clone() }</p>
      <div class="ds-oe-cov-table-wrap">
        <table class="pf-v6-c-table" role="grid">
          <thead>
            <tr role="row">
              <th role="columnheader">{ "Term" }</th>
              <th role="columnheader">{ "Documented status" }</th>
              <th role="columnheader">{ "Verification" }</th>
              <th role="columnheader">{ "What this run observed" }</th>
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

/// The "Performing probes" step's live description -- the one place where
/// a number changes many times per run.
fn probing_description(state: &RunState) -> Option<String> {
  let progress = state.progress()?;
  let mut text =
    format!("{} / {} probed — {} agreed, {} disagreed", progress.done, progress.total, progress.agreed, progress.disagreed);
  if progress.errored > 0 {
    text.push_str(&format!(", {} errored", progress.errored));
  }
  Some(text)
}

/// `ProgressStepper` takes literal `<ProgressStepperStep>` children (a
/// macro-expansion constraint, same as `ToggleGroup`'s), so the four steps
/// are written out rather than generated from `Stage::ALL` in a loop --
/// `Stage::ALL` still drives their labels and statuses, so the ordering
/// lives in one place.
fn stepper(state: &RunState) -> Html {
  let [loading_wasm, loading_catalog, probing, compiling] = Stage::ALL;
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
        status={step_status(state, probing)}
        is_current={state.current_stage() == Some(probing)}
        description={probing_description(state)}
      >
        <span>{ probing.label() }</span>
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

fn progress_bar(progress: &CoverageProgress) -> Html {
  let total = progress.total.max(1) as f64;
  html!(
    <div class="ds-oe-cov-bar">
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
    <p class="ds-oe-cov-note">
      <Button variant={ButtonVariant::Secondary} onclick={on_rerun}>{ "Re-run in this browser" }</Button>
    </p>
  )
}

fn run_panel(state: &RunState, on_rerun: Callback<MouseEvent>) -> Html {
  html!(
    <div class="ds-oe-cov-run-panel">
      { stepper(state) }
      if let Some(progress) = state.progress() {
        { progress_bar(progress) }
      }
      if let RunState::Failed { stage, message } = state {
        <div class="ds-oe-cov-bar">
          <Alert inline=true r#type={AlertType::Danger} title={format!("{} failed", stage.label())}>
            <p>{ message.clone() }</p>
          </Alert>
        </div>
        { rerun_button(on_rerun) }
      }
    </div>
  )
}

/// The loudest of the three redundant signals a contradiction gets (the
/// others: a red verdict label with an icon, and a tinted table row).
fn contradiction_alert(report: &CoverageReport) -> Html {
  let contradicted = report.contradicted_rows();
  if contradicted.is_empty() {
    return html!();
  }
  html!(
    <div class="ds-oe-cov-bar">
      <Alert
        inline=true
        r#type={AlertType::Danger}
        title={format!(
          "{} documented status(es) disagree with what the engine just did",
          contradicted.len()
        )}
      >
        <p>
          { "This browser's run over " }<code>{ "engine.wasm" }</code>
          { " observed something other than what this study documents for these rows. \
             That is a finding about the documentation as much as about the engine:" }
        </p>
        <ul>
          { for contradicted.iter().map(|row| {
              let probe = row.probes.iter().find(|p| p.status == ProbeStatus::Disagreed);
              let mismatch = probe.and_then(|p| p.mismatch.clone()).unwrap_or_default();
              let reason = probe.and_then(|p| p.reason.clone()).unwrap_or_default();
              let id = probe.map(|p| p.id.clone()).unwrap_or_default();
              html!(
                <li key={row.row.id.clone()}>
                  <strong>{ row.row.term.clone() }</strong>
                  { " — probe " }<code>{ id }</code>{ format!(" — {mismatch}") }
                  if !reason.is_empty() { { format!(" — {reason}") } }
                </li>
              )
          }) }
        </ul>
      </Alert>
    </div>
  )
}

fn provenance(report: &CoverageReport) -> Html {
  html!(
    <>
      <p class="ds-oe-cov-provenance">
        <span>
          { format!(
            "Ran {} probes across {} vocabulary rows against engine.wasm ({} bytes) in your browser in {:.0} ms — \
             {} rows verified live, {} documented claims, {} contradicted, {} inconclusive.",
            report.total_probes,
            report.rows.len(),
            report.engine_bytes,
            report.elapsed_ms,
            report.verified,
            report.documented,
            report.contradicted,
            report.inconclusive
          ) }
        </span>
      </p>
      <p class="ds-oe-cov-note">
        { "Catalog: " }<code>{ report.generated_by.clone() }</code>
        { ", fetched from " }<code>{ "compliance-data/latest-coverage.json" }</code>
        { " — the artifact this run actually read, named here so a browser-cached copy is visible rather than silent. \
           Vocabulary: " }<code>{ report.spec.clone() }</code>
        { ". Row statuses and their reasoning come from " }<code>{ report.source_analysis.clone() }</code>{ "." }
      </p>
      <p class="ds-oe-cov-note">
        { "The catalog says of itself: " }<em>{ report.note.clone() }</em>
      </p>
    </>
  )
}

fn stat(value: u64, label: &str, modifier: &str) -> Html {
  html!(
    <div class="ds-oe-stat">
      <span class={format!("ds-oe-stat-value ds-oe-cov-stat-value {modifier}")}>{ value }</span>
      <span class="ds-oe-stat-label">{ label.to_string() }</span>
    </div>
  )
}

/// Two stat rows, one per axis: what this study *documents*, and what this
/// run *observed*. Reuses `pages::STAT_ROW_CSS`'s layout and adds only the
/// per-bucket colours this page needs.
fn stat_rows(report: &CoverageReport) -> Html {
  html!(
    <>
      <div class="ds-oe-stats">
        { stat(report.rows.len() as u64, "rows", "is-total") }
        { stat(report.implemented, "implemented", "is-implemented") }
        { stat(report.partial, "partial", "is-partial") }
        { stat(report.not_implemented, "not implemented", "is-not-implemented") }
        { stat(report.out_of_scope, "out of scope", "is-out-of-scope") }
      </div>
      <div class="ds-oe-stats">
        { stat(report.verified, "verified live", "is-verified") }
        { stat(report.contradicted, "contradicted", "is-contradicted") }
        { stat(report.inconclusive, "inconclusive", "is-inconclusive") }
        { stat(report.documented, "documented claims", "is-documented") }
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
  contradictions_only: UseStateHandle<bool>,
}

fn matches_filters(row: &RowOutcome, filters: &Filters, query: &str) -> bool {
  if !filters.status.matches(&row.row.status) {
    return false;
  }
  if !filters.verdict.matches(row.verdict) {
    return false;
  }
  if *filters.contradictions_only && row.verdict != RowVerdict::Contradicted {
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

fn results(report: &CoverageReport, filters: &Filters) -> Html {
  let query = filters.search.trim().to_lowercase();
  let visible: Vec<&RowOutcome> = report.rows.iter().filter(|row| matches_filters(row, filters, &query)).collect();

  let on_search = {
    let search = filters.search.clone();
    Callback::from(move |value: String| search.set(value))
  };
  let on_contradictions_only = {
    let contradictions_only = filters.contradictions_only.clone();
    Callback::from(move |value: bool| contradictions_only.set(value))
  };

  html!(
    <>
      <style>{ STAT_ROW_CSS }</style>
      { stat_rows(report) }
      <p class="ds-oe-cov-legend">
        { "Red means one thing on this page, and only one: a live probe disagreed with the documented status. \
           A documented gap (" }<em>{ "not implemented" }</em>{ ") is orange, and is verified live exactly like \
           everything else — its probes show the feature not firing." }
      </p>

      <div class="ds-oe-cov-toolbar">
        <div class="ds-oe-cov-search">
          <TextInput placeholder="Search term, row id or probe id..." value={(*filters.search).clone()} onchange={on_search} />
        </div>
        <ToggleGroup>
          <ToggleGroupItem
            text={format!("All ({})", report.rows.len())}
            selected={*filters.status == StatusFilter::All}
            onchange={status_filter_onchange(&filters.status, StatusFilter::All)}
          />
          <ToggleGroupItem
            text={format!("Implemented ({})", report.implemented)}
            selected={*filters.status == StatusFilter::Implemented}
            onchange={status_filter_onchange(&filters.status, StatusFilter::Implemented)}
          />
          <ToggleGroupItem
            text={format!("Partial ({})", report.partial)}
            selected={*filters.status == StatusFilter::Partial}
            onchange={status_filter_onchange(&filters.status, StatusFilter::Partial)}
          />
          <ToggleGroupItem
            text={format!("Not implemented ({})", report.not_implemented)}
            selected={*filters.status == StatusFilter::NotImplemented}
            onchange={status_filter_onchange(&filters.status, StatusFilter::NotImplemented)}
          />
          <ToggleGroupItem
            text={format!("Out of scope ({})", report.out_of_scope)}
            selected={*filters.status == StatusFilter::OutOfScope}
            onchange={status_filter_onchange(&filters.status, StatusFilter::OutOfScope)}
          />
        </ToggleGroup>
        <ToggleGroup>
          <ToggleGroupItem
            text="Any verification"
            selected={*filters.verdict == VerdictFilter::All}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::All)}
          />
          <ToggleGroupItem
            text={format!("Verified ({})", report.verified)}
            selected={*filters.verdict == VerdictFilter::Verified}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::Verified)}
          />
          <ToggleGroupItem
            text={format!("Contradicted ({})", report.contradicted)}
            selected={*filters.verdict == VerdictFilter::Contradicted}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::Contradicted)}
          />
          <ToggleGroupItem
            text={format!("Inconclusive ({})", report.inconclusive)}
            selected={*filters.verdict == VerdictFilter::Inconclusive}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::Inconclusive)}
          />
          <ToggleGroupItem
            text={format!("Documented ({})", report.documented)}
            selected={*filters.verdict == VerdictFilter::Documented}
            onchange={verdict_filter_onchange(&filters.verdict, VerdictFilter::Documented)}
          />
        </ToggleGroup>
        <Switch
          label="Contradictions only"
          checked={*filters.contradictions_only}
          onchange={on_contradictions_only}
        />
      </div>
      <p class="ds-oe-cov-count">
        { format!("Showing {} of {} vocabulary rows.", visible.len(), report.rows.len()) }
      </p>

      if visible.is_empty() {
        <div class="ds-oe-cov-empty">{ "No vocabulary rows match this search/filter." }</div>
      }
      { for report.categories.iter().map(|category| {
          let rows: Vec<&RowOutcome> =
            visible.iter().copied().filter(|row| row.row.category == category.id).collect();
          category_section(category, rows)
      }) }
    </>
  )
}

/// The ODRL 2.2 Coverage page: 52 documented vocabulary claims, 115 live
/// `evaluate()` calls, and a per-row verdict computed in this browser.
#[component]
pub fn CoveragePage() -> Html {
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

  let filters = Filters {
    status: use_state(|| StatusFilter::All),
    verdict: use_state(|| VerdictFilter::All),
    search: use_state(String::new),
    contradictions_only: use_state(|| false),
  };

  html!(
    <>
      <style>{ COVERAGE_CSS }</style>
      <Content>
        <Title level={Level::H1}>{ "ODRL 2.2 Vocabulary Coverage" }</Title>
        <p>
          { "Every claim this study makes about which parts of the " }
          <a href="https://www.w3.org/TR/odrl-vocab/" target="_blank" rel="noopener noreferrer">
            { "W3C ODRL 2.2 Vocabulary & Expression" }
          </a>
          { " this engine implements, put to the test " }<strong>{ "right now, in this browser" }</strong>
          { ", against the real compiled " }<code>{ "engine.wasm" }</code>{ " over its raw " }
          <code>{ "alloc" }</code>{ "/" }<code>{ "evaluate" }</code>{ "/" }<code>{ "dealloc" }</code>
          { " C ABI. Each row below is one documented status; each probe under it is one exact Section 5.2 \
             request plus the outcome that would demonstrate that status. The verdicts are computed here, \
             not read from a committed file." }
        </p>
        <p>
          { "Two things this page is careful about. " }<strong>{ "A gap is not a failure:" }</strong>
          { " most of the vocabulary is deliberately unimplemented, and those rows are verified live in exactly \
             the same way — by a probe that shows the feature not firing, next to a control showing that the \
             same request shape does work when the engine does support it. And " }
          <strong>{ "a row can contradict its own documentation" }</strong>
          { ": if the engine's live answer differs from the recorded status, the row turns red and says so, \
             rather than the page quietly rendering the status it was handed." }
        </p>
        <p>
          { "What this does " }<em>{ "not" }</em>{ " compute in your browser: the probe " }<em>{ "requests" }</em>
          { " were authored natively by " }<code>{ "coverage-probes" }</code>
          { " and travel inside the fetched artifact, and three of the 52 rows carry no probe at all — their \
             claims are about native tooling or about a pre-request concern no request can encode, and each \
             such row says so where its probes would be." }
        </p>
        <p>
          { "One capability on top of this vocabulary doesn't appear as a row at all, deliberately: " }
          <code>{ "dsp-odrl-adapter" }</code>
          { " (a separate, feature-flagged crate ingesting real Dataspace-Protocol-shaped ODRL contract \
             policies — JSON-LD 1.1, opt-in, off by default) runs " }<em>{ "before" }</em>
          { " a wire request exists, so it has no evaluate() call for a probe to make. It isn't a claim \
             about this engine's ODRL 2.2 vocabulary support, so it doesn't belong in this table's own \
             terms — and it isn't yet corpus-tested against a real DSP conformance suite either. See the \
             " }<code>{ "dsp-odrl-adapter" }</code>{ " crate's own README, not a row here, for what it \
             actually does and doesn't handle." }
        </p>
      </Content>

      if let RunState::Done(report) = &*state {
        { contradiction_alert(report) }
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
