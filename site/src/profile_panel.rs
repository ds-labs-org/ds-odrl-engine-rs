//! The "Load ODRL Profile" panel: a paste textarea plus a Turtle/JSON-LD
//! format selector (a paste has no filename to infer a format from, unlike
//! `profile-interpreter`'s own CLI, which infers from extension -- so this
//! panel asks explicitly instead, the same way `duty_mode`/`operator`
//! already ask explicitly via a `FormSelect` elsewhere on this page) and a
//! "Load profile" button that runs `profile_load::load_profile` and
//! reports the result. Only the *successful* result is handed up to
//! `DemoPage` via `on_loaded` -- an error stays local to this panel so a
//! bad paste doesn't clear whatever profile (if any) the form is already
//! configured against.

use patternfly_yew::prelude::*;
use yew::prelude::*;

use crate::profile_load::{load_profile, LoadedProfile, ProfileFormat};

#[derive(Clone, PartialEq)]
enum PanelStatus {
  Idle,
  Loaded(LoadedProfile),
  Error(String),
}

#[derive(Properties, PartialEq)]
pub struct ProfilePanelProps {
  /// The Demonstrator form's current `duty_mode` ("advise"/"deny") --
  /// `interpret()` needs one, and it is always caller-supplied, never
  /// read from the document itself (Section 4.4).
  pub duty_mode: String,
  pub on_loaded: Callback<LoadedProfile>,
}

#[component]
pub fn ProfilePanel(props: &ProfilePanelProps) -> Html {
  let text = use_state(String::new);
  let format = use_state(|| "turtle".to_string());
  let status = use_state(|| PanelStatus::Idle);

  let on_text_change = {
    let text = text.clone();
    Callback::from(move |value: String| text.set(value))
  };
  let on_format_change = {
    let format = format.clone();
    Callback::from(move |value: Option<String>| format.set(value.unwrap_or_else(|| "turtle".to_string())))
  };
  let on_dismiss_status = {
    let status = status.clone();
    Callback::from(move |()| status.set(PanelStatus::Idle))
  };

  let on_load = {
    let text = text.clone();
    let format = format.clone();
    let status = status.clone();
    let duty_mode = props.duty_mode.clone();
    let on_loaded = props.on_loaded.clone();
    Callback::from(move |_: MouseEvent| {
      let parsed_format = if *format == "jsonld" { ProfileFormat::JsonLd } else { ProfileFormat::Turtle };
      match load_profile(&text, parsed_format, &duty_mode) {
        Ok(profile) => {
          on_loaded.emit(profile.clone());
          status.set(PanelStatus::Loaded(profile));
        }
        Err(message) => status.set(PanelStatus::Error(message)),
      }
    })
  };

  let status_view = match &*status {
    PanelStatus::Idle => html!(),
    PanelStatus::Error(message) => html!(
      <Alert inline=true r#type={AlertType::Danger} title="Could not load this profile" onclose={on_dismiss_status.clone()}>
        <pre style="white-space: pre-wrap;">{ message.clone() }</pre>
      </Alert>
    ),
    PanelStatus::Loaded(profile) => html!(
      <>
        <Alert inline=true r#type={AlertType::Success} title="Profile loaded" onclose={on_dismiss_status.clone()}>
          <p>
            { format!(
              "{} — declares {} recognized action(s) and {} left operand(s).",
              profile.id,
              profile.recognized_actions.len(),
              profile.declared_left_operands.len(),
            ) }
          </p>
        </Alert>
        if !profile.warnings.is_empty() {
          <Alert inline=true r#type={AlertType::Warning} title="Interpreter warnings">
            <ul style="margin: 0; padding-left: 1.2rem;">
              { for profile.warnings.iter().map(|w| html!(<li>{ w.clone() }</li>)) }
            </ul>
          </Alert>
        }
      </>
    ),
  };

  html!(
    <Content>
      <Title level={Level::H2}>{ "Load ODRL Profile" }</Title>
      <p>
        { "Paste a real ODRL Profile document (Turtle or JSON-LD) to configure the fields below from \
           it: recognized actions become selectable, and declared left operands become suggestions \
           (Section 4.2's leftOperand stays free-form -- see " }
        <code>{ "profile-interpreter/README.md" }</code>{ " for what is and isn't derived from it." }
      </p>
      <div style="display: flex; flex-direction: column; gap: 0.7rem; max-width: 40rem;">
        <label>
          { "format" }
          <FormSelect<String> value={Some((*format).clone())} onchange={on_format_change}>
            <FormSelectOption<String> value="turtle" description="Turtle (.ttl)" />
            <FormSelectOption<String> value="jsonld" description="JSON-LD (.jsonld/.json)" />
          </FormSelect<String>>
        </label>
        <TextArea
          rows={8}
          placeholder="@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n..."
          value={(*text).clone()}
          onchange={on_text_change}
        />
        <div>
          <Button variant={ButtonVariant::Secondary} label="Load profile" onclick={on_load} />
        </div>
      </div>
      { status_view }
    </Content>
  )
}
