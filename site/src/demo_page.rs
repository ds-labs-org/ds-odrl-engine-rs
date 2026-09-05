//! The real Demonstrator page (Section 5.2): a friendly form over
//! Section 5.2's JSON wire contract, evaluated against `engine.wasm`
//! through `engine_bridge::evaluate` (unchanged -- see that module's own
//! doc comment for the alloc/write/evaluate/read/dealloc round trip this
//! reuses as-is). Replaces the earlier proof-of-concept single button.

use crate::demo_form::{ClaimRow, DemoForm, RuleRow};
use crate::demo_widgets::{ClaimRowsEditor, RuleRowsEditor, DEFAULT_LEFT_OPERAND_SUGGESTIONS, LEFT_OPERAND_DATALIST_ID};
use crate::engine_bridge;
use crate::pages::case_study_credit;
use crate::profile_load::LoadedProfile;
use crate::profile_panel::ProfilePanel;
use crate::wire;
use patternfly_yew::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

#[derive(Clone, PartialEq)]
enum EvalStatus {
  Idle,
  Running,
  Done(Result<wire::Response, String>),
}

fn field_onchange(form: UseStateHandle<DemoForm>, set: fn(&mut DemoForm, String)) -> Callback<String> {
  Callback::from(move |value: String| {
    let mut next = (*form).clone();
    set(&mut next, value);
    form.set(next);
  })
}

fn rules_onchange(form: UseStateHandle<DemoForm>, set: fn(&mut DemoForm, Vec<RuleRow>)) -> Callback<Vec<RuleRow>> {
  Callback::from(move |value: Vec<RuleRow>| {
    let mut next = (*form).clone();
    set(&mut next, value);
    form.set(next);
  })
}

#[component]
pub fn DemoPage() -> Html {
  let form = use_state(DemoForm::example);
  let status = use_state(|| EvalStatus::Idle);
  let show_raw = use_state(|| false);
  let loaded_profile = use_state(|| None::<LoadedProfile>);

  let recognized_actions_from_profile: Vec<String> =
    loaded_profile.as_ref().map(|p| p.recognized_actions.clone()).unwrap_or_default();
  let left_operand_suggestions: Vec<String> = {
    let mut suggestions: Vec<String> = DEFAULT_LEFT_OPERAND_SUGGESTIONS.iter().map(|s| s.to_string()).collect();
    if let Some(profile) = loaded_profile.as_ref() {
      suggestions.extend(profile.declared_left_operands.iter().cloned());
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions
  };

  let on_profile_loaded = {
    let loaded_profile = loaded_profile.clone();
    Callback::from(move |profile: LoadedProfile| loaded_profile.set(Some(profile)))
  };

  let request = crate::demo_form::to_request(&form);
  let request_json = serde_json::to_string_pretty(&request)
    .unwrap_or_else(|err| format!("<could not serialize this form as JSON: {err}>"));

  let on_dataset_id = field_onchange(form.clone(), |f, v| f.dataset_id = v);
  let on_recognized_actions = field_onchange(form.clone(), |f, v| f.recognized_actions = v);
  let on_pick_recognized_action = {
    let form = form.clone();
    Callback::from(move |picked: Option<String>| {
      let Some(picked) = picked else { return };
      let mut next = (*form).clone();
      let mut actions = crate::demo_form::split_csv(&next.recognized_actions);
      if !actions.iter().any(|a| a == &picked) {
        actions.push(picked);
      }
      next.recognized_actions = actions.join(", ");
      form.set(next);
    })
  };
  let on_duty_mode = {
    let form = form.clone();
    Callback::from(move |value: Option<String>| {
      let mut next = (*form).clone();
      next.duty_mode = value.unwrap_or_else(|| "advise".to_string());
      form.set(next);
    })
  };
  let on_policy_kind = field_onchange(form.clone(), |f, v| f.policy_kind = v);
  let on_assigner = field_onchange(form.clone(), |f, v| f.assigner = v);
  let on_permissions = rules_onchange(form.clone(), |f, v| f.permissions = v);
  let on_prohibitions = rules_onchange(form.clone(), |f, v| f.prohibitions = v);
  let on_obligations = rules_onchange(form.clone(), |f, v| f.obligations = v);
  let on_claims = {
    let form = form.clone();
    Callback::from(move |value: Vec<ClaimRow>| {
      let mut next = (*form).clone();
      next.claims = value;
      form.set(next);
    })
  };

  let on_load_example = {
    let form = form.clone();
    let status = status.clone();
    Callback::from(move |_: MouseEvent| {
      form.set(DemoForm::example());
      status.set(EvalStatus::Idle);
    })
  };
  let on_clear = {
    let form = form.clone();
    let status = status.clone();
    Callback::from(move |_: MouseEvent| {
      form.set(DemoForm::empty());
      status.set(EvalStatus::Idle);
    })
  };
  let on_toggle_raw = {
    let show_raw = show_raw.clone();
    Callback::from(move |checked: bool| show_raw.set(checked))
  };

  let on_evaluate = {
    let status = status.clone();
    let request_json = request_json.clone();
    Callback::from(move |_: MouseEvent| {
      let status = status.clone();
      let request_json = request_json.clone();
      status.set(EvalStatus::Running);
      spawn_local(async move {
        let outcome = engine_bridge::evaluate(&request_json).await.and_then(|response_json| {
          serde_json::from_str::<wire::Response>(&response_json)
            .map_err(|err| format!("engine.wasm returned JSON this page could not parse: {err} (raw: {response_json})"))
        });
        status.set(EvalStatus::Done(outcome));
      });
    })
  };

  let result_view = match &*status {
    EvalStatus::Idle => html!(),
    EvalStatus::Running => html!(
      <Alert inline=true r#type={AlertType::Info} title="Evaluating...">
        { "Calling engine.wasm's evaluate() export via the WASM bridge." }
      </Alert>
    ),
    EvalStatus::Done(Err(message)) => html!(
      <Alert inline=true r#type={AlertType::Danger} title="Evaluation failed">
        <pre style="white-space: pre-wrap;">{ message.clone() }</pre>
      </Alert>
    ),
    EvalStatus::Done(Ok(response)) => {
      let (color, label): (Color, String) = match response.decision.as_str() {
        "Allow" => (Color::Green, "Allow".to_string()),
        "Deny" => (Color::Red, "Deny".to_string()),
        other => (Color::Orange, if other.is_empty() { "Error".to_string() } else { other.to_string() }),
      };
      html!(
        <div style="margin-top: 1rem; padding: 1rem 1.25rem; border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2); border-radius: 6px;">
          <div style="display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.5rem;">
            <Label label={label} color={color} />
            <strong>{ "Decision" }</strong>
          </div>
          <p>{ response.reason.clone() }</p>
          if !response.duties.is_empty() {
            <table class="pf-v6-c-table" role="grid" style="max-width: 40rem;">
              <thead>
                <tr role="row">
                  <th role="columnheader">{ "Policy" }</th>
                  <th role="columnheader">{ "Action" }</th>
                  <th role="columnheader">{ "Resolved" }</th>
                </tr>
              </thead>
              <tbody>
                { for response.duties.iter().map(|duty| html!(
                  <tr role="row" key={format!("{}-{}", duty.policy_id, duty.action)}>
                    <td role="cell">{ duty.policy_id.clone() }</td>
                    <td role="cell">{ duty.action.clone() }</td>
                    <td role="cell">{ if duty.resolved { "yes" } else { "no" } }</td>
                  </tr>
                )) }
              </tbody>
            </table>
          }
          <div style="margin-top: 0.75rem;">
            <Tooltip text="Coming in a future update — will let you submit this test case as a fixture for review.">
              <Button variant={ButtonVariant::Secondary} label="Report this test result" disabled=true />
            </Tooltip>
          </div>
        </div>
      )
    }
  };

  html!(
    <>
      <Content>
        <Title level={Level::H1}>{ "Demonstrator" }</Title>
        <p>
          { "Builds a Section 5.2 request from the form below, then drives " }
          <code>{ "engine.wasm" }</code>{ "'s raw " }
          <code>{ "alloc" }</code>{ "/" }<code>{ "evaluate" }</code>{ "/" }<code>{ "dealloc" }</code>
          { " C ABI by hand (see " }<code>{ "engine_bridge.rs" }</code>{ ") to get a real decision back." }
        </p>
        <div style="display: flex; gap: 0.5rem; margin-bottom: 1rem;">
          <Button variant={ButtonVariant::Secondary} label="Load Section 5.2 example" onclick={on_load_example} />
          <Button variant={ButtonVariant::Tertiary} label="Clear" onclick={on_clear} />
        </div>
      </Content>

      <ProfilePanel duty_mode={form.duty_mode.clone()} on_loaded={on_profile_loaded} />
      <datalist id={LEFT_OPERAND_DATALIST_ID}>
        { for left_operand_suggestions.iter().map(|s| html!(<option value={s.clone()} />)) }
      </datalist>

      <Content>
        <Title level={Level::H2}>{ "Dataset" }</Title>
        <label>
          { "dataset_id" }
          <TextInput value={form.dataset_id.clone()} onchange={on_dataset_id} />
        </label>
      </Content>

      <Content>
        <Title level={Level::H2}>{ "Claims" }</Title>
        <p>{ "The identity claims presented to the engine (Section 4.1) -- toggle \"list\" for a comma-separated, multi-valued claim." }</p>
        <ClaimRowsEditor claims={form.claims.clone()} on_change={on_claims} />
      </Content>

      <Content>
        <Title level={Level::H2}>{ "Config" }</Title>
        <div style="display: flex; flex-direction: column; gap: 0.7rem; max-width: 30rem;">
          <label>
            { "recognized_actions (comma-separated)" }
            <div style="display: flex; align-items: flex-start; gap: 0.5rem;">
              <TextInput placeholder="use, distribute, notify" value={form.recognized_actions.clone()} onchange={on_recognized_actions} />
              if !recognized_actions_from_profile.is_empty() {
                <FormSelect<String> value={None::<String>} placeholder="insert from profile..." onchange={on_pick_recognized_action}>
                  { for recognized_actions_from_profile.iter().map(|action| yew::html_nested!(
                      <FormSelectOption<String> value={action.clone()} description={action.clone()} />
                    )) }
                </FormSelect<String>>
              }
            </div>
          </label>
          <label>
            { "duty_mode" }
            <FormSelect<String> value={Some(form.duty_mode.clone())} onchange={on_duty_mode}>
              <FormSelectOption<String> value="advise" description="advise — unresolved duties are recorded, decision can still Allow" />
              <FormSelectOption<String> value="deny" description="deny — an unresolved duty forces Deny" />
            </FormSelect<String>>
          </label>
        </div>
      </Content>

      <Content>
        <Title level={Level::H2}>{ "Policy" }</Title>
        <p>{ "This demo evaluates one policy at a time (Section 5.2's request carries a " }<code>{"policies"}</code>{ " array, but a single entry is enough to exercise the wire contract)." }</p>
        <div style="display: flex; flex-direction: column; gap: 0.7rem; max-width: 30rem; margin-bottom: 1rem;">
          <label>
            { "kind" }
            <TextInput placeholder="Offer / Agreement / Set" value={form.policy_kind.clone()} onchange={on_policy_kind} />
          </label>
          <label>
            { "assigner" }
            <TextInput placeholder="did:web:provider.example" value={form.assigner.clone()} onchange={on_assigner} />
          </label>
        </div>

        <Title level={Level::H3}>{ "Permissions" }</Title>
        <RuleRowsEditor
          rules={form.permissions.clone()}
          on_change={on_permissions}
          add_label="Add permission"
          action_placeholder="action (e.g. use)"
          recognized_actions={recognized_actions_from_profile.clone()}
        />

        <Title level={Level::H3}>{ "Prohibitions" }</Title>
        <RuleRowsEditor
          rules={form.prohibitions.clone()}
          on_change={on_prohibitions}
          add_label="Add prohibition"
          action_placeholder="action (e.g. distribute)"
          recognized_actions={recognized_actions_from_profile.clone()}
        />

        <Title level={Level::H3}>{ "Obligations" }</Title>
        <RuleRowsEditor
          rules={form.obligations.clone()}
          on_change={on_obligations}
          add_label="Add obligation"
          action_placeholder="action (e.g. notify)"
          recognized_actions={recognized_actions_from_profile.clone()}
        />
      </Content>

      <Content>
        <Switch checked={*show_raw} label="Advanced: show raw JSON" onchange={on_toggle_raw} />
        if *show_raw {
          <pre style="max-width: 100%; overflow-x: auto; padding: 0.75rem; border: 1px solid var(--pf-t--global--border--color--default, #d2d2d2); border-radius: 6px;">{ request_json.clone() }</pre>
        }
      </Content>

      <Content>
        <Button variant={ButtonVariant::Primary} label="Evaluate" onclick={on_evaluate} />
      </Content>

      { result_view }
      { case_study_credit() }
    </>
  )
}
