use crate::app_route::AppRoute;
use crate::engine_module::fetch_engine_wasm_len;
use patternfly_yew::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;
use yew_nested_router::components::Link;

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

/// Placeholder Home page. Doubles as a live wiring check for this stage:
/// fetches `engine.wasm` over `fetch()` (see engine_module.rs) and reports
/// its byte length, proving the relative-path/`<base href>` mechanism this
/// site depends on to work under a GitHub Pages subpath actually resolves.
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
      <Content>
        <Title level={Level::H1}>{ "ds-odrl-engine-rs" }</Title>
        <p>
          { "A portable WebAssembly ODRL Policy Decision Engine: a pure, stateless " }
          <code>{ "(policy, claims) -> decision" }</code>
          { " evaluator compiled to " }
          <code>{ "wasm32-unknown-unknown" }</code>
          { " and invoked identically from a Rust host (" }
          <code>{ "wasmi" }</code>
          { ") or a JVM host (Chicory) through a minimal four-export ABI over guest linear memory." }
        </p>
        <p>
          { "Try the " }
          <Link<AppRoute> to={AppRoute::Demo}>{ "Demonstrator" }</Link<AppRoute>>
          { " to build a Section 5.2 request and evaluate it against a real "}
          <code>{ "engine.wasm" }</code>
          { " instance. The " }
          <Link<AppRoute> to={AppRoute::Compliance}>{ "Compliance Results" }</Link<AppRoute>>
          { " page is still a placeholder -- a later stage fills it in." }
        </p>
      </Content>
      { engine_status }
      { case_study_credit() }
    </>
  )
}

/// Placeholder Compliance Results page: a later stage renders
/// `compliance/reports/latest.md`'s per-case pass/fail/skip breakdown here.
#[component]
pub fn CompliancePage() -> Html {
  html!(
    <>
      <Content>
        <Title level={Level::H1}>{ "Compliance Results" }</Title>
        <p>{ "This page will render the compliance-runner's results against the vendored ODRL Test Suite. Not wired up yet." }</p>
      </Content>
      { case_study_credit() }
    </>
  )
}
