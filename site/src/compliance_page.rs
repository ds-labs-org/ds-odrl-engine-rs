//! The real Compliance Results page: fetches `compliance-data/latest.json` at
//! *runtime* -- via `fetch()`, exactly like `engine_module.rs` fetches
//! `engine.wasm` -- rather than baking it into this crate at Rust compile
//! time the way the Home page's own compliance summary does (`pages.rs`).
//! Those two pages deliberately differ: the Home page's summary is four
//! numbers a stale build can't get wrong for long (it's rebuilt whenever
//! this site is), while this page's whole point is the full ~70-row
//! breakdown -- and a future `compliance-runner` re-run should be able to
//! update what this page shows via nothing more than re-copying
//! `compliance/reports/latest.json` and redeploying the already-built
//! site, with no Rust code change in between.
//!
//! `index.html`'s own `copy-file` directive lands that file at
//! `dist/compliance-data/latest.json`; the relative fetch below resolves it
//! against this page's `<base href>` the same way `engine_module.rs`
//! documents for `engine.wasm`. The asset's own directory is named
//! `compliance-data`, not `compliance` -- see `index.html`'s copy-file
//! comment for why reusing this page's own `/compliance` route name for
//! the physical directory breaks direct loads of that route.

use crate::pages::{STAT_ROW_CSS, case_study_credit, stat_row_html};
use patternfly_yew::prelude::*;
use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::Response;
use yew::prelude::*;

/// Mirrors `compliance-runner/src/report.rs`'s `JsonCase` field-for-field,
/// including which fields are only ever populated for one `status`:
/// `decision` for `"passed"`, `expected`/`actual` for `"failed"`, and
/// `reason` for both `"failed"` (why the divergence) and `"skipped"` (the
/// Section 7 citation).
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ComplianceCase {
  slug: String,
  title: String,
  status: String,
  decision: Option<String>,
  expected: Option<String>,
  actual: Option<String>,
  reason: Option<String>,
}

/// Mirrors `compliance-runner/src/report.rs`'s `JsonReport`. The two id
/// lists are redundant with `cases[].status` (this page filters on the
/// latter) and aren't otherwise used here, but are kept as fields --
/// rather than dropped from the struct -- so a shape mismatch against the
/// real file would still show up as a `serde` error instead of silently
/// ignoring unknown keys.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ComplianceReport {
  total: u64,
  passed: u64,
  failed: u64,
  skipped: u64,
  #[allow(dead_code)]
  failing_case_ids: Vec<String>,
  #[allow(dead_code)]
  skipped_case_ids: Vec<String>,
  cases: Vec<ComplianceCase>,
}

async fn fetch_compliance_report() -> Result<ComplianceReport, String> {
  let window = web_sys::window().ok_or_else(|| "no `window` (not running in a browser)".to_string())?;

  let response: Response = JsFuture::from(window.fetch_with_str("compliance-data/latest.json"))
    .await
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "fetch() did not resolve to a Response".to_string())?;

  if !response.ok() {
    return Err(format!("compliance-data/latest.json fetch returned HTTP {}", response.status()));
  }

  let text = JsFuture::from(response.text().map_err(describe_js_error)?)
    .await
    .map_err(describe_js_error)?
    .as_string()
    .ok_or_else(|| "response.text() did not resolve to a string".to_string())?;

  serde_json::from_str(&text).map_err(|err| format!("compliance-data/latest.json did not match the expected shape: {err}"))
}

fn describe_js_error(err: JsValue) -> String {
  err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

#[derive(Clone, PartialEq)]
enum FetchState {
  Loading,
  Ready(ComplianceReport),
  Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusFilter {
  All,
  Passed,
  Failed,
  Skipped,
}

impl StatusFilter {
  fn matches(self, status: &str) -> bool {
    match self {
      StatusFilter::All => true,
      StatusFilter::Passed => status == "passed",
      StatusFilter::Failed => status == "failed",
      StatusFilter::Skipped => status == "skipped",
    }
  }
}

/// This page's own layout CSS: the search/filter toolbar and the table's
/// slug/detail cell styling. Kept page-scoped the same way `pages.rs`'s
/// `HOME_CSS` is -- an inline `<style>` tag, `ds-oe-`-prefixed classes --
/// rather than added to `assets/theme.css`, since only this page uses it.
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
"#;

fn status_label(status: &str) -> Html {
  let (color, text) = match status {
    "passed" => (Color::Green, "Passed"),
    "failed" => (Color::Red, "Failed"),
    "skipped" => (Color::Grey, "Skipped"),
    other => (Color::Orange, other),
  };
  html!(<Label label={text.to_string()} color={color} compact=true />)
}

fn detail_html(case: &ComplianceCase) -> Html {
  match case.status.as_str() {
    "failed" => {
      let expected = case.expected.clone().unwrap_or_else(|| "?".to_string());
      let actual = case.actual.clone().unwrap_or_else(|| "?".to_string());
      html!(
        <span class="ds-oe-compliance-detail">
          <strong>{ format!("expected {expected}, actual {actual}") }</strong>
          if let Some(reason) = &case.reason {
            { format!(" — {reason}") }
          }
        </span>
      )
    }
    "skipped" => html!(
      <span class="ds-oe-compliance-detail">{ case.reason.clone().unwrap_or_default() }</span>
    ),
    "passed" => html!(
      <span class="ds-oe-compliance-detail">
        { case.decision.as_deref().map(|d| format!("decision: {d}")).unwrap_or_default() }
      </span>
    ),
    _ => html!(),
  }
}

/// `ToggleGroup` requires `ChildrenWithProps<ToggleGroupItem>` -- a
/// literal child type it checks at macro-expansion time -- so each item
/// has to be a real `<ToggleGroupItem>` tag rather than a wrapper
/// component; this just builds the one piece (its `onchange`) that
/// differs per filter value, so the four call sites below stay a
/// one-liner each.
fn filter_onchange(filter: &UseStateHandle<StatusFilter>, value: StatusFilter) -> Callback<()> {
  let filter = filter.clone();
  Callback::from(move |()| filter.set(value))
}

/// The real Compliance Results page: a live-fetched summary stat row,
/// then a search/status-filterable table over every case in
/// `compliance-data/latest.json`.
#[component]
pub fn CompliancePage() -> Html {
  let state = use_state(|| FetchState::Loading);
  {
    let state = state.clone();
    use_effect_with((), move |()| {
      spawn_local(async move {
        state.set(match fetch_compliance_report().await {
          Ok(report) => FetchState::Ready(report),
          Err(message) => FetchState::Failed(message),
        });
      });
      || ()
    });
  }

  let filter = use_state(|| StatusFilter::All);
  let search = use_state(String::new);
  let on_search = {
    let search = search.clone();
    Callback::from(move |value: String| search.set(value))
  };

  let body = match &*state {
    FetchState::Loading => html!(
      <Alert inline=true r#type={AlertType::Info} title="Loading compliance-data/latest.json...">
        { "Fetching the compliance-runner's latest results." }
      </Alert>
    ),
    FetchState::Failed(message) => html!(
      <Alert inline=true r#type={AlertType::Danger} title="Could not load compliance results">
        <p>{ message.clone() }</p>
      </Alert>
    ),
    FetchState::Ready(report) => {
      let query = search.trim().to_lowercase();
      let filtered: Vec<&ComplianceCase> = report
        .cases
        .iter()
        .filter(|case| filter.matches(&case.status))
        .filter(|case| query.is_empty() || case.slug.to_lowercase().contains(&query) || case.title.to_lowercase().contains(&query))
        .collect();

      html!(
        <>
          <style>{ STAT_ROW_CSS }</style>
          { stat_row_html(report.total, report.passed, report.failed, report.skipped) }

          <div class="ds-oe-compliance-toolbar">
            <div class="ds-oe-compliance-search">
              <TextInput placeholder="Search by slug or title..." value={(*search).clone()} onchange={on_search} />
            </div>
            <ToggleGroup>
              <ToggleGroupItem
                text={format!("All ({})", report.total)}
                selected={*filter == StatusFilter::All}
                onchange={filter_onchange(&filter, StatusFilter::All)}
              />
              <ToggleGroupItem
                text={format!("Passed ({})", report.passed)}
                selected={*filter == StatusFilter::Passed}
                onchange={filter_onchange(&filter, StatusFilter::Passed)}
              />
              <ToggleGroupItem
                text={format!("Failed ({})", report.failed)}
                selected={*filter == StatusFilter::Failed}
                onchange={filter_onchange(&filter, StatusFilter::Failed)}
              />
              <ToggleGroupItem
                text={format!("Skipped ({})", report.skipped)}
                selected={*filter == StatusFilter::Skipped}
                onchange={filter_onchange(&filter, StatusFilter::Skipped)}
              />
            </ToggleGroup>
          </div>
          <p class="ds-oe-compliance-count">
            { format!("Showing {} of {} cases.", filtered.len(), report.cases.len()) }
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
                { for filtered.iter().map(|case| html!(
                  <tr role="row" key={case.slug.clone()}>
                    <td role="cell"><span class="ds-oe-compliance-slug">{ case.slug.clone() }</span></td>
                    <td role="cell">{ case.title.clone() }</td>
                    <td role="cell">{ status_label(&case.status) }</td>
                    <td role="cell">{ detail_html(case) }</td>
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
  };

  html!(
    <>
      <style>{ COMPLIANCE_CSS }</style>
      <Content>
        <Title level={Level::H1}>{ "Compliance Results" }</Title>
        <p>
          { "Every case from the vendored " }
          <a href="https://github.com/SolidLabResearch/ODRL-Test-Suite" target="_blank" rel="noopener noreferrer">{ "SolidLabResearch/ODRL-Test-Suite" }</a>
          { ", run by " }<code>{ "compliance-runner" }</code>{ " against " }<code>{ "engine.wasm" }</code>
          { "'s Section 5.2 contract. This table is fetched at runtime from " }
          <code>{ "compliance-data/latest.json" }</code>{ " (a served copy of " }
          <code>{ "compliance/reports/latest.json" }</code>
          { "), so it reflects whatever the compliance-runner last wrote without needing a rebuild of this \
             site -- unlike the Home page's own summary tally, which is embedded at this site's own compile \
             time. See the full generated report, including the same breakdown as Markdown, at " }
          <a href="https://github.com/ds-labs-org/ds-odrl-engine-rs/blob/main/compliance/reports/latest.md" target="_blank" rel="noopener noreferrer">
            { "compliance/reports/latest.md" }
          </a>
          { "." }
        </p>
      </Content>

      { body }

      { case_study_credit() }
    </>
  )
}
