//! The Release History page.
//!
//! Every tagged release, each one rebuilt from its own tag and put
//! through this repo's two measuring instruments: the vendored
//! ODRL-Test-Suite (run by *that tag's own* `compliance-runner`, against
//! the suite revision *that tag* pinned) and the current ODRL 2.2
//! coverage catalog (replayed against *that tag's* compiled
//! `engine.wasm`, driven through its own `alloc`/`dealloc`/`evaluate` C
//! ABI in a `wasmi` interpreter). The release count and the catalog's own
//! probe count both come out of the rendered `HistoryFile` at runtime
//! (`intro`, `build_time_alert`) rather than being repeated here as
//! literals that would need to be kept in lockstep with it by hand —
//! this doc comment deliberately names no specific counts for the same
//! reason.
//!
//! **This is the one page on this site that does not recompute what it
//! shows, and it says so in its own first paragraph.** The Compliance
//! Results and Coverage pages both re-execute their corpora against
//! `engine.wasm` in the visitor's browser. Reproducing *this* page's
//! numbers live would mean shipping every historical `engine.wasm`
//! binary — several megabytes of them — instantiating every one, and
//! running the whole probe catalog against each, on page load, in order
//! to recompute figures that can only change when someone cuts a new
//! tag. That is not a reasonable thing to ask of a visitor's browser, so
//! the numbers are computed at build time by `release-history` and
//! rendered here, with the per-release `engine.wasm` SHA-256 on the page
//! so a reader can rebuild any tag and check they are looking at the
//! same binary.
//!
//! The other honesty this page owes its reader: the nine releases before
//! v0.6.0 are **not** shown as having supported nothing. v0.6.0 reshaped
//! the request's `config` object from `{"recognized_actions": [...]}`
//! into JSON-LD, which is a rename rather than an addition, so every one
//! of the current catalog's requests is refused by an earlier engine at
//! its own deserializer — before any policy logic runs. Those releases
//! get their real compliance numbers and an explicit "not addressable by
//! this catalog" note, never a zero.

use crate::history_catalog::{ContradictedRow, HistoryFile, Release};
use crate::history_run::fetch_history;
use crate::pages::case_study_credit;
use patternfly_yew::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

/// This page's own layout CSS, page-scoped as an inline `<style>` with
/// `ds-oe-hist-`-prefixed classes — the same choice `COVERAGE_CSS`,
/// `COMPLIANCE_CSS` and `HOME_CSS` already make, rather than growing
/// `assets/theme.css` for one page. Colours are PatternFly 6 design
/// tokens with literal hex fallbacks, so they stay legible if a token is
/// ever renamed upstream.
const HISTORY_CSS: &str = r#"
/* Presentation polish, requested by the project owner and scoped to this
   page alone -- this whole <style> block mounts with `HistoryPage` and
   unmounts with it on route change, so nothing below reaches any other
   page. No new colours, no restructuring, no reworded prose: geometry
   only.

   Left/right margin. There is no page-local content wrapper narrower than
   the shared `Page` layout: what actually controls horizontal whitespace
   around every page's content today is PatternFly's own
   `.pf-v6-c-page__main-section` padding
   (`--pf-v6-c-page__main-section--PaddingInline{Start,End}`, both
   defaulting to `--pf-t--global--spacer--lg` = 1.5rem -- see
   `site/target/node_modules/@patternfly/patternfly/6.4.0/components/Page/page.css`).
   Doubled to 3rem on *both* sides here, not just one.

   Base font size, +10%. PatternFly's own body-text token
   (`--pf-t--global--font--size--body--default`) is the real base every
   piece of prose on this page reads from -- `Content` paragraphs, `Alert`
   bodies, `Card` bodies. It resolves to 0.875rem
   (`--pf-t--global--font--size--sm` = `--pf-t--global--font--size--200`,
   `site/target/node_modules/@patternfly/patternfly/6.4.0/base/patternfly-variables.css`).
   0.875rem * 1.10 = 0.9625rem, computed from that real number rather than
   guessed. Redeclaring only the top-level `--pf-t--global--...` token here
   does NOT reach any of those, though: each component's own
   `--pf-v6-c-*--FontSize` variable is declared once, at PatternFly's own
   `:root` rule, as `var(--pf-t--global--font--size--body--default)` -- and
   a custom property's `var()` references resolve against the environment
   *where that property is declared*, not where it is later read, so
   redeclaring only the leaf token here would leave every one of those
   already-resolved 0.875rem values untouched (verified empirically: it
   measurably does nothing to a rendered paragraph's `font-size` without
   the component-level overrides below). Each component-level token this
   page's own prose actually reads is therefore redeclared directly below,
   at the same 1.10 scale. This page's tables are a deliberate, honest
   exception, unaffected either way: `release_table`/`longest_standing_gaps`
   render plain `<table class="pf-v6-c-table"><tr><th>/<td>` with none of
   PatternFly's own `.pf-v6-c-table__tr`/`__th`/`__td` classes (compiled
   CSS gates per-cell typography behind `tr:where(.pf-v6-c-table__tr) >
   :where(th, td)`, which a plain `<tr><td>` never matches) -- so table
   text already rendered at the plain browser default before this change,
   governed by no PatternFly token in either direction, and stays that way
   here rather than gaining a new, page-local special case for a gap this
   pass did not introduce and was not asked to close. */
.pf-v6-c-page__main-section {
  padding-inline-start: 3rem !important;
  padding-inline-end: 3rem !important;
  --pf-t--global--font--size--body--default: 0.9625rem;
  --pf-v6-c-content--FontSize: 0.9625rem;
  --pf-v6-c-alert--FontSize: 0.9625rem;
  --pf-v6-c-card__body--FontSize: 0.9625rem;
}
.ds-oe-hist-table-wrap { overflow-x: auto; }
.ds-oe-hist-tag {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-weight: 700;
  white-space: nowrap;
}
.ds-oe-hist-when {
  font-size: 0.78rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
  white-space: nowrap;
}
/* Same subtle treatment as .ds-oe-hist-when, deliberately WITHOUT its
   `white-space: nowrap`. That rule exists so a table cell's date, time and
   short commit each stay on one line; reusing the same class for the
   provenance paragraphs under the method card pushed a long generator path
   straight off the right edge of the card instead of wrapping -- caught in
   a headless render, not in review. */
.ds-oe-hist-note {
  font-size: 0.78rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-hist-num {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  white-space: nowrap;
}
.ds-oe-hist-good { color: #3e8635; font-weight: 700; }
.ds-oe-hist-bad { color: #c9190b; font-weight: 700; }
.ds-oe-hist-muted { color: var(--pf-t--global--text--color--subtle, #6a6e73); }
.ds-oe-hist-summary { font-size: 0.85rem; max-width: 34rem; }
.ds-oe-hist-sha {
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.7rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
  word-break: break-all;
}
.ds-oe-hist-chart { width: 100%; max-width: 60rem; height: auto; margin: 1rem 0 0.25rem; }
.ds-oe-hist-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 1.25rem;
  margin: 0 0 1.5rem;
  font-size: 0.8rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-hist-swatch {
  display: inline-block;
  width: 0.85rem;
  height: 0.85rem;
  border-radius: 2px;
  margin-right: 0.4rem;
  vertical-align: -1px;
}
.ds-oe-hist-details { margin: 0.35rem 0 0; font-size: 0.82rem; }
.ds-oe-hist-details > summary { cursor: pointer; color: var(--pf-t--global--text--color--link--default, #0066cc); }
.ds-oe-hist-gap {
  margin: 0.4rem 0 0.1rem 1rem;
  padding-left: 0.6rem;
  border-left: 2px solid var(--pf-t--global--border--color--default, #d2d2d2);
}
.ds-oe-hist-gap code { font-size: 0.78rem; }
.ds-oe-hist-reason {
  display: block;
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.72rem;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
  margin-top: 0.1rem;
}
.ds-oe-hist-section { margin-top: 3.5rem; }
/* Vertical space ahead of the two Alert boxes and the chart(s) -- a
   clearly larger, consistent top margin (matches `.ds-oe-hist-section`'s
   own bump above), not a token nudge. */
.ds-oe-hist-lead { margin-top: 3rem; }
"#;

/// Series colours, shared by the chart and its legend so the two cannot
/// drift apart.
const COMPLIANCE_COLOR: &str = "#0066cc";
const COVERAGE_COLOR: &str = "#3e8635";

/// Stacked-status-chart colours, one per [`RowStatusBreakdown`] field.
/// Reused from elsewhere on this same page rather than invented fresh,
/// but one: `STATUS_IMPLEMENTED_COLOR` is `COVERAGE_COLOR` itself (the
/// existing "good" green), `STATUS_NOT_IMPLEMENTED_COLOR` is
/// `.ds-oe-hist-bad`'s red, `STATUS_OUT_OF_SCOPE_COLOR` is the same subtle
/// grey `.ds-oe-hist-muted`/`.ds-oe-hist-when` already use for
/// not-applicable text. `STATUS_PARTIAL_COLOR` is the one new hex value
/// this page introduces, an amber roughly at PatternFly's own warning hue,
/// chosen to sit visually between the green and the red it is stacked
/// between. All four keep enough contrast against both a light and a dark
/// page background to stay legible in either `prefers-color-scheme`
/// (the SVG draws no other page-controlled background of its own to clash
/// against, same as the line chart above it).
const STATUS_IMPLEMENTED_COLOR: &str = COVERAGE_COLOR;
const STATUS_PARTIAL_COLOR: &str = "#f0ab00";
const STATUS_NOT_IMPLEMENTED_COLOR: &str = "#c9190b";
const STATUS_OUT_OF_SCOPE_COLOR: &str = "#6a6e73";

/// Chart geometry, in the SVG's own `viewBox` units.
const CHART_W: f64 = 760.0;
const CHART_H: f64 = 280.0;
const PLOT_LEFT: f64 = 46.0;
const PLOT_RIGHT: f64 = 736.0;
const PLOT_TOP: f64 = 18.0;
const PLOT_BOTTOM: f64 = 224.0;

fn x_at(index: usize, count: usize) -> f64 {
  if count <= 1 {
    return (PLOT_LEFT + PLOT_RIGHT) / 2.0;
  }
  PLOT_LEFT + (PLOT_RIGHT - PLOT_LEFT) * (index as f64) / ((count - 1) as f64)
}

fn y_at(fraction: f64) -> f64 {
  PLOT_BOTTOM - (PLOT_BOTTOM - PLOT_TOP) * fraction.clamp(0.0, 1.0)
}

/// Turns a run of `(index, fraction)` points into an SVG polyline, or
/// `None` when there is nothing to draw.
///
/// Points are *not* interpolated across a release with no value: the
/// coverage series genuinely has no value before v0.6.0, and drawing a
/// line through that region would invent nine data points that the
/// generator explicitly declined to produce.
fn polyline_points(points: &[(usize, f64)], count: usize) -> Option<String> {
  if points.len() < 2 {
    return None;
  }
  Some(points.iter().map(|(i, f)| format!("{:.1},{:.1}", x_at(*i, count), y_at(*f))).collect::<Vec<_>>().join(" "))
}

fn series_dots(points: &[(usize, f64)], count: usize, color: &'static str) -> Html {
  html!(
    <>
      { for points.iter().map(|(i, f)| html!(
          <circle cx={format!("{:.1}", x_at(*i, count))} cy={format!("{:.1}", y_at(*f))} r="3.2" fill={color} />
        )) }
    </>
  )
}

/// Two series over the release axis: the ODRL-Test-Suite pass rate each
/// release actually recorded, and the share of probeable vocabulary rows
/// that release's own binary verifies against today's catalog.
///
/// Drawn as inline SVG with no charting library, matching this site's
/// existing diagrams: it has to render in a theme it does not control and
/// stay legible at whatever width the page is.
fn chart(file: &HistoryFile) -> Html {
  let count = file.releases.len();

  let compliance: Vec<(usize, f64)> =
    file.releases.iter().enumerate().filter_map(|(i, r)| r.compliance_fraction().map(|f| (i, f))).collect();
  let coverage: Vec<(usize, f64)> =
    file.releases.iter().enumerate().filter_map(|(i, r)| r.verified_fraction().map(|f| (i, f))).collect();

  // The contiguous leading run of releases the current catalog cannot
  // address, shaded and labelled rather than left as a mysterious gap in
  // the green series.
  let unaddressable_upto = file.releases.iter().position(|r| r.coverage.is_some());
  let shade = unaddressable_upto.filter(|&first| first > 0).map(|first| {
    let x0 = PLOT_LEFT - 6.0;
    let x1 = x_at(first, count) - 6.0;
    html!(
      <>
        <rect x={format!("{x0:.1}")} y={format!("{PLOT_TOP:.1}")} width={format!("{:.1}", x1 - x0)}
              height={format!("{:.1}", PLOT_BOTTOM - PLOT_TOP)} fill="currentColor" opacity="0.06" />
        <text x={format!("{:.1}", (x0 + x1) / 2.0)} y={format!("{:.1}", PLOT_TOP + 14.0)}
              text-anchor="middle" font-size="10" fill="currentColor" opacity="0.65">
          { "catalog cannot address" }
        </text>
      </>
    )
  });

  html!(
    <>
      <svg class="ds-oe-hist-chart" viewBox={format!("0 0 {CHART_W} {CHART_H}")} role="img"
           aria-label="Per-release ODRL-Test-Suite pass rate and share of probeable ODRL 2.2 vocabulary rows verified">
        { for [0.0, 0.25, 0.5, 0.75, 1.0].iter().map(|f| html!(
            <>
              <line x1={format!("{PLOT_LEFT:.1}")} y1={format!("{:.1}", y_at(*f))}
                    x2={format!("{PLOT_RIGHT:.1}")} y2={format!("{:.1}", y_at(*f))}
                    stroke="currentColor" stroke-width="0.5" opacity="0.18" />
              <text x={format!("{:.1}", PLOT_LEFT - 8.0)} y={format!("{:.1}", y_at(*f) + 3.5)}
                    text-anchor="end" font-size="10" fill="currentColor" opacity="0.6">
                { format!("{}%", (f * 100.0).round() as i64) }
              </text>
            </>
          )) }

        { shade }

        { for polyline_points(&compliance, count).map(|points| html!(
            <polyline points={points} fill="none" stroke={COMPLIANCE_COLOR} stroke-width="2" />
          )) }
        { series_dots(&compliance, count, COMPLIANCE_COLOR) }

        { for polyline_points(&coverage, count).map(|points| html!(
            <polyline points={points} fill="none" stroke={COVERAGE_COLOR} stroke-width="2" />
          )) }
        { series_dots(&coverage, count, COVERAGE_COLOR) }

        { for file.releases.iter().enumerate().map(|(i, release)| {
            // Every other label at 19 releases, or they collide.
            let show = count <= 10 || i % 2 == 0 || i + 1 == count;
            html!(
              <>
                <line x1={format!("{:.1}", x_at(i, count))} y1={format!("{PLOT_BOTTOM:.1}")}
                      x2={format!("{:.1}", x_at(i, count))} y2={format!("{:.1}", PLOT_BOTTOM + 4.0)}
                      stroke="currentColor" stroke-width="0.6" opacity="0.4" />
                if show {
                  <text x={format!("{:.1}", x_at(i, count))} y={format!("{:.1}", PLOT_BOTTOM + 20.0)}
                        text-anchor="middle" font-size="9.5" fill="currentColor" opacity="0.75"
                        transform={format!("rotate(-38 {:.1} {:.1})", x_at(i, count), PLOT_BOTTOM + 20.0)}>
                    { release.tag.clone() }
                  </text>
                }
              </>
            )
          }) }

        <line x1={format!("{PLOT_LEFT:.1}")} y1={format!("{PLOT_BOTTOM:.1}")}
              x2={format!("{PLOT_RIGHT:.1}")} y2={format!("{PLOT_BOTTOM:.1}")}
              stroke="currentColor" stroke-width="1" opacity="0.45" />
      </svg>

      <div class="ds-oe-hist-legend">
        <span>
          <span class="ds-oe-hist-swatch" style={format!("background: {COMPLIANCE_COLOR};")}></span>
          { "ODRL-Test-Suite fixtures passed, as that release's own runner reported them" }
        </span>
        <span>
          <span class="ds-oe-hist-swatch" style={format!("background: {COVERAGE_COLOR};")}></span>
          { "vocabulary rows verified, out of the rows today's catalog can probe" }
        </span>
      </div>
    </>
  )
}

/// Geometry for the stacked status chart below, sharing [`x_at`]'s x-axis
/// mapping (and therefore the same release positions) with the line chart
/// above it, so the two charts read as one coordinated pair rather than
/// two unrelated figures.
const STATUS_PLOT_TOP: f64 = 16.0;
const STATUS_PLOT_BOTTOM: f64 = 190.0;
const STATUS_CHART_H: f64 = 250.0;

fn status_y_at(rows: f64, total_rows: f64) -> f64 {
  STATUS_PLOT_BOTTOM - (STATUS_PLOT_BOTTOM - STATUS_PLOT_TOP) * (rows / total_rows).clamp(0.0, 1.0)
}

/// The new per-release stacked chart: one bar per release, left to right
/// chronological (same x-axis as the line chart above), each split into
/// its own [`RowStatusBreakdown`] -- Implemented/Partial/NotImplemented
/// stacked bottom to top by "how close to the documented reading", with
/// OutOfScope on top. See [`dashboard`]'s caption, printed immediately
/// below this chart, for the exact classification rule in prose.
///
/// The nine releases the current catalog cannot address at all (no
/// `row_status`, same releases [`chart`]'s own shaded region already
/// marks) get the identical shaded gap and label here rather than an
/// absent bar -- an absent bar with nothing beside it could read as "zero
/// everywhere", which is exactly the misreading the line chart's own gap
/// treatment above already exists to avoid.
fn status_chart(file: &HistoryFile) -> Html {
  let count = file.releases.len();
  let total_rows = file.catalog.rows as f64;
  let bar_w = ((PLOT_RIGHT - PLOT_LEFT) / (count.max(1) as f64) * 0.6).max(2.0);

  let addressable_from = file.releases.iter().position(|r| r.row_status.is_some());
  let shade = addressable_from.filter(|&first| first > 0).map(|first| {
    let x0 = PLOT_LEFT - 6.0;
    let x1 = x_at(first, count) - 6.0;
    html!(
      <>
        <rect x={format!("{x0:.1}")} y={format!("{STATUS_PLOT_TOP:.1}")} width={format!("{:.1}", x1 - x0)}
              height={format!("{:.1}", STATUS_PLOT_BOTTOM - STATUS_PLOT_TOP)} fill="currentColor" opacity="0.06" />
        <text x={format!("{:.1}", (x0 + x1) / 2.0)} y={format!("{:.1}", STATUS_PLOT_TOP + 14.0)}
              text-anchor="middle" font-size="10" fill="currentColor" opacity="0.65">
          { "catalog cannot address" }
        </text>
      </>
    )
  });

  let bars = file.releases.iter().enumerate().filter_map(|(i, release)| {
    let rs = release.row_status.as_ref()?;
    let cx = x_at(i, count);
    let x = cx - bar_w / 2.0;
    // Bottom to top: Implemented (closest to the documented reading),
    // Partial, NotImplemented, OutOfScope on top -- a green foundation a
    // reader expects to grow over time, with the always-8-rows OutOfScope
    // band riding unchanged at the top (see `release-history`'s own
    // `classify_row_for_release`: every OutOfScope-documented row, and
    // every zero-probe row regardless of its own documented status, pins
    // to OutOfScope release after release).
    let segments = [
      (rs.implemented as f64, STATUS_IMPLEMENTED_COLOR),
      (rs.partial as f64, STATUS_PARTIAL_COLOR),
      (rs.not_implemented as f64, STATUS_NOT_IMPLEMENTED_COLOR),
      (rs.out_of_scope as f64, STATUS_OUT_OF_SCOPE_COLOR),
    ];
    let mut cumulative = 0.0;
    let rects: Vec<Html> = segments
      .iter()
      .map(|(value, color)| {
        let y0 = status_y_at(cumulative, total_rows);
        cumulative += value;
        let y1 = status_y_at(cumulative, total_rows);
        html!(
          <rect x={format!("{x:.1}")} y={format!("{y1:.1}")} width={format!("{bar_w:.1}")}
                height={format!("{:.1}", (y0 - y1).max(0.0))} fill={*color}>
            <title>{ format!("{}: {} rows", release.tag, *value as u64) }</title>
          </rect>
        )
      })
      .collect();
    Some(html!(<>{ for rects }</>))
  });

  html!(
    <>
      <svg class="ds-oe-hist-chart" viewBox={format!("0 0 {CHART_W} {STATUS_CHART_H}")} role="img"
           aria-label="Per-release Implemented/Partial/NotImplemented/OutOfScope breakdown of the 52 ODRL 2.2 vocabulary rows, derived from that release's own probe agreement">
        { for [0.0, 0.25, 0.5, 0.75, 1.0].iter().map(|f| {
            let rows = f * total_rows;
            html!(
              <>
                <line x1={format!("{PLOT_LEFT:.1}")} y1={format!("{:.1}", status_y_at(rows, total_rows))}
                      x2={format!("{PLOT_RIGHT:.1}")} y2={format!("{:.1}", status_y_at(rows, total_rows))}
                      stroke="currentColor" stroke-width="0.5" opacity="0.18" />
                <text x={format!("{:.1}", PLOT_LEFT - 8.0)} y={format!("{:.1}", status_y_at(rows, total_rows) + 3.5)}
                      text-anchor="end" font-size="10" fill="currentColor" opacity="0.6">
                  { format!("{}", rows.round() as i64) }
                </text>
              </>
            )
          }) }

        { shade }
        { for bars }

        { for file.releases.iter().enumerate().map(|(i, release)| {
            let show = count <= 10 || i % 2 == 0 || i + 1 == count;
            html!(
              <>
                <line x1={format!("{:.1}", x_at(i, count))} y1={format!("{STATUS_PLOT_BOTTOM:.1}")}
                      x2={format!("{:.1}", x_at(i, count))} y2={format!("{:.1}", STATUS_PLOT_BOTTOM + 4.0)}
                      stroke="currentColor" stroke-width="0.6" opacity="0.4" />
                if show {
                  <text x={format!("{:.1}", x_at(i, count))} y={format!("{:.1}", STATUS_PLOT_BOTTOM + 20.0)}
                        text-anchor="middle" font-size="9.5" fill="currentColor" opacity="0.75"
                        transform={format!("rotate(-38 {:.1} {:.1})", x_at(i, count), STATUS_PLOT_BOTTOM + 20.0)}>
                    { release.tag.clone() }
                  </text>
                }
              </>
            )
          }) }

        <line x1={format!("{PLOT_LEFT:.1}")} y1={format!("{STATUS_PLOT_BOTTOM:.1}")}
              x2={format!("{PLOT_RIGHT:.1}")} y2={format!("{STATUS_PLOT_BOTTOM:.1}")}
              stroke="currentColor" stroke-width="1" opacity="0.45" />
      </svg>

      <div class="ds-oe-hist-legend">
        <span>
          <span class="ds-oe-hist-swatch" style={format!("background: {STATUS_IMPLEMENTED_COLOR};")}></span>
          { "Implemented" }
        </span>
        <span>
          <span class="ds-oe-hist-swatch" style={format!("background: {STATUS_PARTIAL_COLOR};")}></span>
          { "Partial" }
        </span>
        <span>
          <span class="ds-oe-hist-swatch" style={format!("background: {STATUS_NOT_IMPLEMENTED_COLOR};")}></span>
          { "Not implemented" }
        </span>
        <span>
          <span class="ds-oe-hist-swatch" style={format!("background: {STATUS_OUT_OF_SCOPE_COLOR};")}></span>
          { "Out of scope" }
        </span>
      </div>
      <Content>
        <p class="ds-oe-hist-note">
          <strong>{ "How each release's bar is classified. " }</strong>
          { "A row with no probe at all (a documented-only claim, no wire request can encode it) is always "}
          <em>{ "Out of scope" }</em>
          { ", every release alike. A row this catalog documents as " }<em>{ "Not implemented" }</em>
          { " or " }<em>{ "Out of scope" }</em>{ " today stays pinned to that same reading for every release, \
             regardless of this release's own probe agreement: both are non-capability statuses whose probes \
             are controls proving a disclosed gap or a permanent boundary is still correctly there, not tests \
             of positive capability -- a still-open, disclosed gap has no \"more implemented in the past\" \
             reading, and treating its probes' agreement as a positive signal would misrepresent a real, \
             disclosed limitation as historically closed. Every other row -- documented " }
          <em>{ "Implemented" }</em>{ " or " }<em>{ "Partial" }</em>
          { " today, the two statuses whose own probes DO test positive capability -- is classified by this \
             release's own agreement against that row's own probes: every probe agreed keeps the row's \
             documented status unchanged (a " }<em>{ "Partial" }</em>
          { " row whose calibrated probes all agree stays " }<em>{ "Partial" }</em>
          { ", never upgraded to " }<em>{ "Implemented" }</em>
          { " for matching exactly the behaviour its own documented status already describes as partial); \
             zero probes agreed means this release predates the documented behaviour entirely (" }
          <em>{ "Not implemented" }</em>{ "); anything in between is " }<em>{ "Partial" }</em>
          { "." }
        </p>
      </Content>
    </>
  )
}

fn compliance_cell(release: &Release) -> Html {
  match &release.compliance {
    Some(c) => {
      let class = if c.failed == 0 && c.skipped == 0 { "ds-oe-hist-good" } else { "ds-oe-hist-bad" };
      html!(
        <>
          <span class={classes!("ds-oe-hist-num", class)}>{ format!("{}/{}", c.passed, c.total) }</span>
          if c.failed > 0 || c.skipped > 0 {
            <div class="ds-oe-hist-when">
              { format!("{} failed, {} skipped", c.failed, c.skipped) }
            </div>
          }
        </>
      )
    }
    None => html!(<span class="ds-oe-hist-muted">{ "not reproduced" }</span>),
  }
}

fn gap_detail(row: &ContradictedRow) -> Html {
  html!(
    <div class="ds-oe-hist-gap">
      <code>{ row.id.clone() }</code>
      { " — " }
      { row.term.clone() }
      <span class="ds-oe-hist-reason">{ format!("{} · {}", row.probe_id.clone(), row.mismatch.clone()) }</span>
      if !row.engine_reason.is_empty() {
        <span class="ds-oe-hist-reason">{ format!("engine said: {}", row.engine_reason) }</span>
      }
    </div>
  )
}

fn coverage_cell(release: &Release) -> Html {
  match &release.coverage {
    None => html!(
      <>
        <span class="ds-oe-hist-muted">{ "not addressable" }</span>
        <details class="ds-oe-hist-details">
          <summary>{ "why" }</summary>
          <div class="ds-oe-hist-gap">
            { release.coverage_error.clone().unwrap_or_default() }
          </div>
        </details>
      </>
    ),
    Some(coverage) => html!(
      <>
        <span class="ds-oe-hist-num ds-oe-hist-good">{ coverage.verified }</span>
        { " verified · " }
        <span class={classes!(
          "ds-oe-hist-num",
          if coverage.contradicted > 0 { "ds-oe-hist-bad" } else { "ds-oe-hist-muted" }
        )}>{ coverage.contradicted }</span>
        { " contradicted" }
        <div class="ds-oe-hist-when">
          { format!(
              "{} agreed / {} disagreed / {} errored of {} probes · {} documented-only rows",
              coverage.agreed, coverage.disagreed, coverage.errored, coverage.probes_total, coverage.documented
            ) }
        </div>
        if !release.contradicted_rows.is_empty() {
          <details class="ds-oe-hist-details">
            <summary>{ format!("{} capabilities this release did not have yet", release.contradicted_rows.len()) }</summary>
            { for release.contradicted_rows.iter().map(gap_detail) }
          </details>
        }
      </>
    ),
  }
}

fn release_row(release: &Release) -> Html {
  html!(
    <tr role="row">
      <td role="cell">
        <div class="ds-oe-hist-tag">{ release.tag.clone() }</div>
        <div class="ds-oe-hist-when" title={release.date.clone()}>
          { format!("{} {}", release.day(), release.time_of_day()) }
        </div>
        <div class="ds-oe-hist-when">{ release.short_commit().to_string() }</div>
      </td>
      <td role="cell">{ compliance_cell(release) }</td>
      <td role="cell">{ coverage_cell(release) }</td>
      <td role="cell">
        <div class="ds-oe-hist-summary">{ release.summary.clone() }</div>
        <div class="ds-oe-hist-sha" title="SHA-256 of that tag's compiled engine.wasm">
          { format!("{} B · {}", release.engine_wasm_bytes, release.engine_wasm_sha256) }
        </div>
      </td>
    </tr>
  )
}

fn release_table(file: &HistoryFile) -> Html {
  html!(
    <div class="ds-oe-hist-table-wrap">
      <table class="pf-v6-c-table" role="grid">
        <thead>
          <tr role="row">
            <th role="columnheader">{ "Release" }</th>
            <th role="columnheader">{ "ODRL-Test-Suite" }</th>
            <th role="columnheader">{ "ODRL 2.2 rows, today's catalog" }</th>
            <th role="columnheader">{ "What shipped" }</th>
          </tr>
        </thead>
        <tbody role="rowgroup">
          { for file.releases.iter().map(release_row) }
        </tbody>
      </table>
    </div>
  )
}

/// The cross-release view no single row shows: which vocabulary rows
/// stayed contradicted across the most releases, i.e. what took longest
/// to land.
fn longest_standing_gaps(file: &HistoryFile) -> Html {
  let counts = file.contradiction_counts();
  if counts.is_empty() {
    return html!();
  }
  let addressable = file.releases.len() - file.unaddressable().len();
  html!(
    <div class="ds-oe-hist-section">
      <Title level={Level::H2}>{ "What took longest to land" }</Title>
      <Content>
        <p>
          { format!(
              "Of the {addressable} releases today's catalog can address, how many each vocabulary row \
               was still contradicted in. A row near the top is one this engine carried as a documented \
               gap for most of its history."
            ) }
        </p>
      </Content>
      <div class="ds-oe-hist-table-wrap">
        <table class="pf-v6-c-table" role="grid">
          <thead>
            <tr role="row">
              <th role="columnheader">{ "Row" }</th>
              <th role="columnheader">{ "Term" }</th>
              <th role="columnheader">{ "Releases contradicted" }</th>
            </tr>
          </thead>
          <tbody role="rowgroup">
            { for counts.iter().map(|(id, term, n)| html!(
                <tr role="row">
                  <td role="cell"><code>{ id.clone() }</code></td>
                  <td role="cell">{ term.clone() }</td>
                  <td role="cell">
                    <span class="ds-oe-hist-num">{ format!("{n} of {addressable}") }</span>
                  </td>
                </tr>
              )) }
          </tbody>
        </table>
      </div>
    </div>
  )
}

fn provenance(file: &HistoryFile) -> Html {
  html!(
    <Card>
      <CardBody>
        <Content>
          <p>
            <strong>{ "How these numbers were produced. " }</strong>
            { file.method.clone() }
          </p>
          <p class="ds-oe-hist-note">
            { format!("Generated by {} · catalog: {}", file.generated_by, file.catalog.generated_by) }
          </p>
          if let Some(latest) = file.latest() {
            <p>
              <strong>{ "One row here is independently checkable. " }</strong>
              { format!(
                  "{} is the release this site itself is built from, so the ODRL 2.2 Coverage page \
                   re-runs that same catalog against that same engine in your own browser. Its numbers \
                   there and its row here must agree — and a workspace test asserts they do, so a \
                   regeneration that went stale fails the build rather than quietly showing you an old \
                   dashboard.",
                  latest.tag
                ) }
            </p>
          }
          <p class="ds-oe-hist-note">
            { format!(
                "Catalog under replay: {} rows ({} implemented, {} partial, {} not implemented, {} out of \
                 scope) and {} probes, from {}",
                file.catalog.rows,
                file.catalog.implemented,
                file.catalog.partial,
                file.catalog.not_implemented,
                file.catalog.out_of_scope,
                file.catalog.probes,
                file.catalog.source_analysis
              ) }
          </p>
        </Content>
      </CardBody>
    </Card>
  )
}

fn dashboard(file: &HistoryFile) -> Html {
  html!(
    <>
      <div class="ds-oe-hist-lead">
        { chart(file) }
      </div>
      <div class="ds-oe-hist-lead">
        <Title level={Level::H2}>{ "Implemented / Partial / Not implemented / Out of scope, per release" }</Title>
        { status_chart(file) }
      </div>
      { provenance(file) }
      <div class="ds-oe-hist-section">
        <Title level={Level::H2}>{ "Every tagged release" }</Title>
        { release_table(file) }
      </div>
      { longest_standing_gaps(file) }
    </>
  )
}

/// The page's own introductory paragraph — reads `file.catalog.probes`
/// rather than a literal, so it never drifts from the catalog it actually
/// describes the way a hardcoded probe count already had (found stale at
/// 125 when the real count was 132, and would have drifted again with
/// every future catalog change). Only renderable once `HistoryFile` has
/// loaded, which is why `HistoryPage` calls this inside its own
/// `Some(Ok(file))` arm rather than unconditionally.
fn intro(file: &HistoryFile) -> Html {
  html!(
    <Content>
      <p>
        { "Every tagged release of this engine, rebuilt from its own tag and put back through both of \
           this repo's measuring instruments: the vendored " }
        <a href="https://github.com/SolidLabResearch/ODRL-Test-Suite" target="_blank" rel="noopener noreferrer">
          { "ODRL-Test-Suite" }
        </a>
        { ", run by that release's own " }<code>{ "compliance-runner" }</code>
        { " against the suite revision that release pinned; and the current " }
        { file.catalog.probes.to_string() }
        { "-probe ODRL 2.2 coverage catalog, replayed against that release's compiled " }
        <code>{ "engine.wasm" }</code>
        { " through the same " }<code>{ "alloc" }</code>{ "/" }<code>{ "evaluate" }</code>{ "/" }
        <code>{ "dealloc" }</code>{ " C ABI a browser drives, in a " }<code>{ "wasmi" }</code>
        { " interpreter." }
      </p>
    </Content>
  )
}

/// The "computed at build time" explainer — same reasoning as `intro`
/// above: the release count and the total evaluation count are both
/// derived from `file` rather than repeated as separate literals that
/// would need to be kept in lockstep with it by hand.
fn build_time_alert(file: &HistoryFile) -> Html {
  let releases = file.releases.len();
  let evaluations = releases * file.catalog.probes;
  html!(
    <Alert inline=true r#type={AlertType::Info} title={"Computed at build time, not in your browser"}>
      <Content>
        <p>
          { "The " }<strong>{ "Compliance Results" }</strong>{ " and " }
          <strong>{ "ODRL 2.2 Coverage" }</strong>
          { " pages both re-execute their whole corpus against " }<code>{ "engine.wasm" }</code>
          { " in your browser, live, and the numbers they show are computed there. This page does not, \
             and the difference is deliberate: its subject is " }
          { releases.to_string() }{ " " }<em>{ "different" }</em>
          { " historical engine binaries. Reproducing it live would mean downloading and instantiating \
             all " }{ releases.to_string() }{ " — several megabytes of wasm — and running " }
          { evaluations.to_string() }
          { " evaluations on page load, to recompute figures that can only change when someone cuts a \
             new tag." }
        </p>
        <p>
          { "So these are build-time figures with their provenance attached: every release's row carries \
             the SHA-256 of the exact " }<code>{ "engine.wasm" }</code>
          { " that produced it, so anyone can rebuild that tag and check the binary matches. The \
             generator is checked in (" }<code>{ "scripts/build-release-history.sh" }</code>{ " and the " }
          <code>{ "release-history" }</code>
          { " crate) and the verdicts come from the very same module the live Coverage page runs." }
        </p>
      </Content>
    </Alert>
  )
}

/// The Release History page: a build-time-computed record of what every
/// tagged release of this engine actually did, measured by re-running the
/// instruments rather than by reading commit messages.
#[component]
pub fn HistoryPage() -> Html {
  let state: UseStateHandle<Option<Result<HistoryFile, String>>> = use_state(|| None);

  {
    let state = state.clone();
    use_effect_with((), move |_| {
      spawn_local(async move {
        state.set(Some(fetch_history().await));
      });
      || ()
    });
  }

  html!(
    <>
      <style>{ HISTORY_CSS }</style>
      <Content>
        <Title level={Level::H1}>{ "Release History" }</Title>
      </Content>

      {
        match &*state {
          None => html!(
            <Card><CardBody>
              <Spinner />
              { " Loading the release record…" }
            </CardBody></Card>
          ),
          Some(Err(err)) => html!(
            <Alert inline=true r#type={AlertType::Danger} title={"Could not load the release record"}>
              <Content><p>{ err.clone() }</p></Content>
            </Alert>
          ),
          Some(Ok(file)) => {
            let unaddressable = file.unaddressable().len();
            html!(
              <>
                { intro(file) }
                <div class="ds-oe-hist-lead">
                  { build_time_alert(file) }
                </div>
                if unaddressable > 0 {
                  <div class="ds-oe-hist-lead">
                    <Alert inline=true r#type={AlertType::Warning}
                           title={format!("{unaddressable} early releases predate the current wire shape")}>
                      <Content>
                        <p>
                          { "v0.6.0 reshaped the request's " }<code>{ "config" }</code>{ " object from " }
                          <code>{ "{\"recognized_actions\": [...]}" }</code>
                          { " into real JSON-LD vocabulary. That was a rename, not an addition, and the field \
                             it replaced had no " }<code>{ "#[serde(default)]" }</code>
                          { " to fall back on — so an engine built before it refuses every one of today's \
                             probe requests at its own deserializer, before any policy logic runs. Those \
                             releases therefore show their real ODRL-Test-Suite results and " }
                          <em>{ "no" }</em>{ " coverage figure at all, rather than a zero that would read as \
                             \"this release supported nothing\". Every wire change after v0.6.0 was additive, \
                             which is exactly why the replay works from there on." }
                        </p>
                      </Content>
                    </Alert>
                  </div>
                }
                { dashboard(file) }
              </>
            )
          }
        }
      }

      { case_study_credit() }
    </>
  )
}
