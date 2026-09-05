use crate::app_route::AppRoute;
use crate::engine_bridge;
use crate::engine_module::fetch_engine_wasm_len;
use patternfly_yew::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;
use yew_nested_router::components::Link;

/// Section 5.2's worked example, verbatim: a policy granting `use` to
/// anyone claiming German nationality plus a `notify` duty, evaluated
/// against a claim set that satisfies the constraint. Expected response
/// decision: `"Allow"`, with one unresolved `notify` duty.
const SECTION_5_2_EXAMPLE_REQUEST: &str = r#"{"dataset_id":"urn:uuid:example-dataset-1","config":{"recognized_actions":["use","distribute","notify"],"duty_mode":"advise"},"policies":[{"id":"policy-1","kind":"Offer","assigner":"did:web:provider.example","assignee":null,"permissions":[{"action":"use","constraints":[{"left_operand":"nationality","operator":"eq","right_operand":"DE"}]}],"prohibitions":[],"obligations":[{"action":"notify","constraints":[]}]}],"claims":{"sub":"user-42","nationality":"DE","scope":["catalog:read","sparql:read"]}}"#;

/// Case-study credit, shown on every page (see this crate's own top-level
/// task note): links back to the ds42.org dataspace study's case study by
/// naming its file path and repository rather than inventing a URL for a
/// repo this crate doesn't know a real address for.
fn case_study_credit() -> Html {
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
          { "This site is a documentation and demonstrator shell. The " }
          <Link<AppRoute> to={AppRoute::Demo}>{ "Demonstrator" }</Link<AppRoute>>
          { " and " }
          <Link<AppRoute> to={AppRoute::Compliance}>{ "Compliance Results" }</Link<AppRoute>>
          { " pages are placeholders for now -- a later stage fills them in." }
        </p>
      </Content>
      { engine_status }
      { case_study_credit() }
    </>
  )
}

#[derive(Clone, PartialEq)]
enum EvaluationStatus {
  Idle,
  Running,
  Done(Result<String, String>),
}

/// Proof-of-concept Demonstrator page: a single button drives a real
/// `alloc`/write/`evaluate`/read/`dealloc` round trip against `engine.wasm`
/// (see `engine_bridge.rs` and `engine/src/abi.rs`) using Section 5.2's
/// worked example, and renders the raw response JSON (or the error) it
/// gets back. A later stage replaces this with an editable request form.
#[component]
pub fn DemoPage() -> Html {
  let status = use_state(|| EvaluationStatus::Idle);

  let onclick = {
    let status = status.clone();
    Callback::from(move |_: MouseEvent| {
      let status = status.clone();
      status.set(EvaluationStatus::Running);
      spawn_local(async move {
        let outcome = engine_bridge::evaluate(SECTION_5_2_EXAMPLE_REQUEST).await;
        status.set(EvaluationStatus::Done(outcome));
      });
    })
  };

  let result_view = match &*status {
    EvaluationStatus::Idle => html!(),
    EvaluationStatus::Running => html!(
      <Alert inline=true r#type={AlertType::Info} title="Evaluating...">
        { "Calling engine.wasm's evaluate() export via the WASM bridge." }
      </Alert>
    ),
    EvaluationStatus::Done(Ok(response_json)) => html!(
      <Alert inline=true r#type={AlertType::Success} title="Response received">
        <pre>{ response_json.clone() }</pre>
      </Alert>
    ),
    EvaluationStatus::Done(Err(message)) => html!(
      <Alert inline=true r#type={AlertType::Danger} title="Evaluation failed">
        <pre>{ message.clone() }</pre>
      </Alert>
    ),
  };

  html!(
    <>
      <Content>
        <Title level={Level::H1}>{ "Demonstrator" }</Title>
        <p>
          { "This button fetches and instantiates " }
          <code>{ "engine.wasm" }</code>
          { " (once, then caches the instance), then drives its raw " }
          <code>{ "alloc" }</code>{ "/" }<code>{ "evaluate" }</code>{ "/" }<code>{ "dealloc" }</code>
          { " C ABI by hand across the WebAssembly-in-WebAssembly boundary, using Section 5.2's worked example request." }
        </p>
        <Button label="Run Section 5.2 example" variant={ButtonVariant::Primary} onclick={onclick} />
      </Content>
      { result_view }
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
