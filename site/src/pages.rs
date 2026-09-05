use crate::app_route::AppRoute;
use crate::engine_module::fetch_engine_wasm_len;
use patternfly_yew::prelude::*;
use serde::Deserialize;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;
use yew_nested_router::components::Link;

/// This crate's own copy of `compliance/reports/latest.json`'s shape --
/// only the four summary counts the Home page shows; the per-case
/// `cases` array is parsed away rather than modeled, since nothing here
/// reads it (a later stage's real `/compliance` page can add its own
/// richer struct without this one needing to change).
#[derive(Debug, Deserialize)]
struct ComplianceSummary {
  total: u64,
  passed: u64,
  failed: u64,
  skipped: u64,
}

/// Embedded at compile time so the Home page's compliance summary can
/// never drift from what `compliance-runner` last actually wrote --
/// there is no server here to fetch it from at runtime, and copying the
/// numbers into prose by hand is exactly the kind of thing that goes
/// stale the next time the runner re-executes. Mirrors this repo's own
/// `docs/benchmarks/` convention (a point-in-time mirror that says so)
/// at the scale of one JSON file instead of a whole doc.
const COMPLIANCE_LATEST_JSON: &str = include_str!("../../compliance/reports/latest.json");

fn compliance_summary() -> Result<ComplianceSummary, String> {
  serde_json::from_str(COMPLIANCE_LATEST_JSON)
    .map_err(|err| format!("could not parse compliance/reports/latest.json: {err}"))
}

/// Home page's own layout CSS: a hero band, its decorative mesh
/// background, and the "what this is not" panel. Kept page-scoped
/// (loaded via an inline `<style>` tag, `ds-oe-`-prefixed classes) rather
/// than added to assets/theme.css, since theme.css re-points shared
/// PatternFly design tokens every page picks up, while this is
/// Home-specific layout built on top of those tokens -- same split
/// ds42.org's own site keeps between its site-wide theme.css and its
/// landing page's own CSS constant.
const HOME_CSS: &str = r#"
.ds-oe-hero {
  position: relative;
  z-index: 0;
  padding: 2.25rem 0 1.75rem;
  margin: 0 0 1.5rem;
  border-bottom: 1px solid var(--pf-t--global--border--color--default, #d2d2d2);
}
.ds-oe-hero-mesh {
  position: absolute;
  z-index: -1;
  top: -2rem;
  right: -2rem;
  width: 22rem;
  max-width: 60%;
  height: auto;
  opacity: 0.14;
  color: var(--pf-t--global--color--brand--default, #14b8a6);
  pointer-events: none;
}
.ds-oe-eyebrow {
  margin: 0 0 0.75rem;
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 0.75rem;
  font-weight: 600;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
.ds-oe-title {
  margin: 0 0 1rem;
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: clamp(2.25rem, 6vw, 3.5rem);
  line-height: 1;
  font-weight: 700;
  letter-spacing: -0.03em;
}
.ds-oe-lede {
  margin: 0 0 1rem;
  max-width: 68ch;
  font-size: clamp(1.05rem, 2vw, 1.25rem);
  line-height: 1.55;
}
.ds-oe-cite {
  margin: 0 0 1.5rem;
  max-width: 68ch;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
  line-height: 1.5;
}
.ds-oe-cta { display: flex; flex-wrap: wrap; gap: 0.75rem; }
.ds-oe-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.6rem 1.15rem;
  border-radius: 4px;
  border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2);
  color: var(--pf-t--global--text--color--regular, #151515);
  font-weight: 600;
  text-decoration: none;
  cursor: pointer;
}
.ds-oe-btn:hover { border-color: var(--pf-t--global--color--brand--default, #14b8a6); text-decoration: none; }
.ds-oe-btn--primary {
  background: var(--pf-t--global--color--brand--default, #14b8a6);
  border-color: var(--pf-t--global--color--brand--default, #14b8a6);
  color: #fff;
}
.ds-oe-btn--primary:hover { filter: brightness(1.1); color: #fff; }
.ds-oe-not {
  margin: 0 0 1.75rem;
  padding: 1.15rem 1.35rem;
  border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2);
  border-left: 3px solid var(--pf-t--global--color--brand--default, #14b8a6);
  border-radius: 6px;
}
.ds-oe-not ul { margin: 0.5rem 0 0; padding-left: 1.2rem; }
.ds-oe-not li { margin: 0.35rem 0; line-height: 1.5; }
"#;

/// The compliance stat-row markup and its CSS, factored out of the Home
/// page so the real Compliance Results page (`compliance_page.rs`) can
/// render the identical stat row over its own runtime-fetched counts
/// instead of duplicating this markup. `pub(crate)` for that reuse.
pub(crate) const STAT_ROW_CSS: &str = r#"
.ds-oe-stats { display: flex; flex-wrap: wrap; gap: 1.5rem; margin: 0.75rem 0 1rem; }
.ds-oe-stat { text-align: center; }
.ds-oe-stat-value {
  display: block;
  font-family: var(--ds-oe-font-mono, ui-monospace, monospace);
  font-size: 1.9rem;
  font-weight: 700;
  line-height: 1.1;
}
.ds-oe-stat-value.is-total { color: var(--pf-t--global--text--color--regular, #151515); }
.ds-oe-stat-value.is-passed { color: #3e8635; }
.ds-oe-stat-value.is-failed { color: #c9190b; }
.ds-oe-stat-value.is-skipped { color: #8a8d90; }
.ds-oe-stat-label {
  display: block;
  margin-top: 0.15rem;
  font-size: 0.75rem;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--pf-t--global--text--color--subtle, #6a6e73);
}
"#;

/// Renders the `total`/`passed`/`failed`/`skipped` stat row shared by the
/// Home page (compile-time counts) and the Compliance Results page
/// (runtime-fetched counts) -- see [`STAT_ROW_CSS`] for its styling.
pub(crate) fn stat_row_html(total: u64, passed: u64, failed: u64, skipped: u64) -> Html {
  html!(
    <div class="ds-oe-stats">
      <div class="ds-oe-stat">
        <span class="ds-oe-stat-value is-total">{ total }</span>
        <span class="ds-oe-stat-label">{ "total" }</span>
      </div>
      <div class="ds-oe-stat">
        <span class="ds-oe-stat-value is-passed">{ passed }</span>
        <span class="ds-oe-stat-label">{ "passed" }</span>
      </div>
      <div class="ds-oe-stat">
        <span class="ds-oe-stat-value is-failed">{ failed }</span>
        <span class="ds-oe-stat-label">{ "failed" }</span>
      </div>
      <div class="ds-oe-stat">
        <span class="ds-oe-stat-value is-skipped">{ skipped }</span>
        <span class="ds-oe-stat-label">{ "skipped" }</span>
      </div>
    </div>
  )
}

/// Case-study credit, shown on every page (see this crate's own top-level
/// task note): links back to the ds42.org dataspace study's case study by
/// naming its file path and repository rather than inventing a URL for a
/// repo this crate doesn't know a real address for. `pub(crate)` so the
/// Demonstrator page (`demo_page.rs`) can reuse it too.
pub(crate) fn case_study_credit() -> Html {
  html!(
    <Content>
      <p>
        <em>
          { "ds-odrl-engine-rs implements the design proposed in " }
          <code>{ "docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md" }</code>
          { " (\"Attribute-Based ODRL Policy Enforcement over Eclipse EDC\"), in the " }
          <code>{ "Deepthought-Solutions/dataspace" }</code>
          { " repository's ds42.org dataspace study. Read that document for the design rationale behind every decision this site demonstrates." }
        </em>
      </p>
    </Content>
  )
}

#[derive(Clone, PartialEq)]
enum EngineModuleStatus {
  Loading,
  Ready { byte_len: usize },
  Failed { message: String },
}

/// The real Home page: a branded hero, an honest "what this is not"
/// summary condensed from the README, the live compliance-suite tally
/// (embedded at compile time from `compliance/reports/latest.json`), and
/// nav cards into the Demonstrator and Compliance pages. Also keeps this
/// stage's original wiring check -- fetching `engine.wasm` over `fetch()`
/// (see engine_module.rs) and reporting its byte length -- as quiet proof
/// that the relative-path/`<base href>` mechanism this whole site depends
/// on to work under a GitHub Pages subpath actually resolves.
#[component]
pub fn HomePage() -> Html {
  let status = use_state(|| EngineModuleStatus::Loading);

  {
    let status = status.clone();
    use_effect_with((), move |()| {
      spawn_local(async move {
        status.set(match fetch_engine_wasm_len().await {
          Ok(byte_len) => EngineModuleStatus::Ready { byte_len },
          Err(message) => EngineModuleStatus::Failed { message },
        });
      });
      || ()
    });
  }

  let engine_status = match &*status {
    EngineModuleStatus::Loading => html!(
      <Alert inline=true r#type={AlertType::Info} title="Fetching engine.wasm...">
        { "Requesting engine.wasm relative to this page's own base URL." }
      </Alert>
    ),
    EngineModuleStatus::Ready { byte_len } => html!(
      <Alert inline=true r#type={AlertType::Success} title="engine.wasm reachable">
        <p>{ format!("Fetched {byte_len} bytes via a relative fetch(\"engine.wasm\") resolved against this page's <base href>.") }</p>
      </Alert>
    ),
    EngineModuleStatus::Failed { message } => html!(
      <Alert inline=true r#type={AlertType::Danger} title="engine.wasm fetch failed">
        <p>{ message.clone() }</p>
      </Alert>
    ),
  };

  html!(
    <>
      <style>{ HOME_CSS }</style>
      <style>{ STAT_ROW_CSS }</style>

      <section class="ds-oe-hero">
        <svg class="ds-oe-hero-mesh" viewBox="0 0 200 200" aria-hidden="true" focusable="false">
          <g stroke="currentColor" stroke-width="1.4" fill="none">
            <line x1="100" y1="20" x2="170" y2="70" />
            <line x1="170" y1="70" x2="150" y2="150" />
            <line x1="150" y1="150" x2="60" y2="170" />
            <line x1="60" y1="170" x2="20" y2="90" />
            <line x1="20" y1="90" x2="100" y2="20" />
            <line x1="100" y1="20" x2="150" y2="150" />
            <line x1="170" y1="70" x2="60" y2="170" />
            <line x1="20" y1="90" x2="150" y2="150" />
          </g>
          <g fill="currentColor">
            { for [(100, 20), (170, 70), (150, 150), (60, 170), (20, 90)].iter()
                .map(|(x, y)| html!(<circle cx={x.to_string()} cy={y.to_string()} r="5" />)) }
          </g>
        </svg>

        <p class="ds-oe-eyebrow">{ "ODRL policy decision engine · compiled to WebAssembly" }</p>
        <h1 class="ds-oe-title">{ "ds-odrl-engine-rs" }</h1>
        <p class="ds-oe-lede">
          { "A portable, stateless " }
          <code>{ "(policy, claims) -> decision" }</code>
          { " evaluator, built once to " }
          <code>{ "wasm32-unknown-unknown" }</code>
          { " and driven identically from a Rust host (" }
          <code>{ "wasmi" }</code>
          { "), a JVM host (Chicory), or -- as the page below proves -- a browser, all speaking \
             the same Section 5.2 JSON request/response contract over a four-export C ABI." }
        </p>
        <p class="ds-oe-cite">
          { "Implements the design proposed in " }
          <code>{ "docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md" }</code>
          { " (\"Attribute-Based ODRL Policy Enforcement over Eclipse EDC\") in the " }
          <code>{ "Deepthought-Solutions/dataspace" }</code>
          { " repository's ds42.org dataspace study -- see the credit at the foot of this page \
             for the full citation." }
        </p>
        <div class="ds-oe-cta">
          <a class="ds-oe-btn ds-oe-btn--primary" href="https://github.com/ds-labs-org/ds-odrl-engine-rs" target="_blank" rel="noopener noreferrer">
            { "View on GitHub" }
          </a>
          <Link<AppRoute> to={AppRoute::Demo} class={classes!("ds-oe-btn")}>{ "Try the Demonstrator" }</Link<AppRoute>>
          <Link<AppRoute> to={AppRoute::Compliance} class={classes!("ds-oe-btn")}>{ "Compliance results" }</Link<AppRoute>>
        </div>
      </section>

      { engine_status }

      <Content>
        <Title level={Level::H2}>{ "What this is not" }</Title>
      </Content>
      <div class="ds-oe-not">
        <p>
          { "This is " }<strong>{ "not a full ODRL implementation" }</strong>{ ". Condensed from the \
             repository's own README -- read it in full before relying on any of this:" }
        </p>
        <ul>
          <li>
            { "The Default Profile has seven constraint operators (" }
            <code>{ "eq" }</code>{ "/" }<code>{ "neq" }</code>{ "/" }<code>{ "isAnyOf" }</code>
            { ", plus " }<code>{ "lt" }</code>{ "/" }<code>{ "lteq" }</code>{ "/" }<code>{ "gt" }</code>{ "/" }<code>{ "gteq" }</code>
            { " for UTC " }<code>{ "dateTime" }</code>{ " comparison) over a flat string/string-array claims model." }
          </li>
          <li>
            { "Actions are matched by exact string, with one narrow, vocabulary-sourced exception (" }
            <code>{ "odrl:use" }</code>{ " covers everything except the transfer-category actions) -- \
               no general " }<code>{ "includedIn" }</code>{ "/" }<code>{ "implies" }</code>{ " inference otherwise." }
          </li>
          <li>
            { "Nested " }<code>{ "odrl:and" }</code>{ "/" }<code>{ "odrl:or" }</code>{ " logical constraints and \
               party/asset collection membership are resolved by " }<code>{ "compliance-runner" }</code>
            { "'s own adapter, not by any change to the engine's wire contract -- a real host would \
               need equivalent adapter logic, not just this engine." }
          </li>
          <li>
            <code>{ "odrl:xone" }</code>{ " remains genuinely unsupported (no \"exactly one\" exclusivity)." }
          </li>
          <li>
            { "Per-permission " }<code>{ "odrl:duty" }</code>{ " is resolved only by this specific compliance \
               suite's own state-of-the-world fact; the engine itself still evaluates policy-level \
               obligations only." }
          </li>
        </ul>
        <p style="margin: 0.75rem 0 0;">
          { "The README also records known adapter fragility (local-name-only node matching, blank-node \
             duties, first-triple-wins lookups) found by an independent review -- none exercised by the \
             vendored corpus, but not fixed either. See the " }
          <a href="https://github.com/ds-labs-org/ds-odrl-engine-rs#what-this-is-not" target="_blank" rel="noopener noreferrer">
            { "README's own \"What this is not\" section" }
          </a>
          { " for the complete list." }
        </p>
      </div>

      <Content>
        <Title level={Level::H2}>{ "Current compliance summary" }</Title>
      </Content>
      { compliance_summary_view() }

      <Content>
        <Title level={Level::H2}>{ "Get hands-on" }</Title>
      </Content>
      <Gallery gutter=true style={AttrValue::from("margin-bottom: 1.5rem;")}>
        <Card full_height=true>
          <CardTitle><Title level={Level::H3}>{ "Try it in your browser" }</Title></CardTitle>
          <CardBody>
            <p>
              { "Build a Section 5.2 request by hand -- claims, a policy, permissions, prohibitions, \
                 obligations -- and evaluate it against a real " }<code>{ "engine.wasm" }</code>
              { " instance running right here, driven through its raw " }
              <code>{ "alloc" }</code>{ "/" }<code>{ "evaluate" }</code>{ "/" }<code>{ "dealloc" }</code>
              { " C ABI, exactly as a real host would call it." }
            </p>
            <Link<AppRoute> to={AppRoute::Demo} class={classes!("ds-oe-btn", "ds-oe-btn--primary")}>
              { "Open the Demonstrator" }
            </Link<AppRoute>>
          </CardBody>
        </Card>
        <Card full_height=true>
          <CardTitle><Title level={Level::H3}>{ "Compliance results" }</Title></CardTitle>
          <CardBody>
            <p>
              { "See every case from the vendored ODRL Test Suite the engine is checked against \
                 today -- expected vs. actual decision, and, for anything skipped, the specific, \
                 cited reason." }
            </p>
            <Link<AppRoute> to={AppRoute::Compliance} class={classes!("ds-oe-btn", "ds-oe-btn--primary")}>
              { "View compliance results" }
            </Link<AppRoute>>
          </CardBody>
        </Card>
      </Gallery>

      { case_study_credit() }
    </>
  )
}

/// Renders the compliance stat row from the compile-time-embedded
/// `compliance/reports/latest.json`, or a plain error `Alert` if that
/// file's shape ever stops matching `ComplianceSummary` -- fails loudly
/// on the page rather than silently showing stale or fabricated numbers.
fn compliance_summary_view() -> Html {
  match compliance_summary() {
    Ok(summary) => stat_row_html(summary.total, summary.passed, summary.failed, summary.skipped),
    Err(message) => html!(
      <Alert inline=true r#type={AlertType::Danger} title="Could not read compliance/reports/latest.json">
        <p>{ message }</p>
      </Alert>
    ),
  }
}
