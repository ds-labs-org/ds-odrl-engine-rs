//! Small, reusable "list of rows with add/remove" editors shared by the
//! Demonstrator page's constraint lists, permission/prohibition/
//! obligation rule lists, and claims editor -- the same repeatable shape
//! (a `Vec<Row>` plus a `Callback<Vec<Row>>` to report edits back up)
//! appears at three different nesting depths (claims are flat; rules
//! nest a `ConstraintRowsEditor` per row), so it's factored out once
//! rather than hand-rolled three times.

use crate::demo_form::{ClaimRow, ConstraintRow, RuleRow};
use patternfly_yew::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;
use yew::prelude::*;

const OPERATORS: &[(&str, &str)] = &[
  ("eq", "eq — equals"),
  ("neq", "neq — not equals"),
  ("isAnyOf", "isAnyOf — value is one of"),
  ("lt", "lt — before (datetime)"),
  ("lteq", "lteq — at or before (datetime)"),
  ("gt", "gt — after (datetime)"),
  ("gteq", "gteq — at or after (datetime)"),
];

/// id of the single, page-level `<datalist>` (rendered once by
/// `DemoPage`) every `left_operand` field's `list` attribute points at --
/// one shared datalist rather than one per row, since HTML ids must be
/// unique and a `<datalist>` produces no visible markup of its own to
/// duplicate anyway.
pub const LEFT_OPERAND_DATALIST_ID: &str = "left-operand-suggestions";

/// The Default Profile's own illustrative claim names (README Section
/// 5.2's worked example: `sub`/`nationality`/`scope`, plus `dateTime` for
/// the `lt`/`lteq`/`gt`/`gteq` operators' own worked comparisons) -- seeded
/// into the left-operand datalist so suggestions exist even before any
/// profile document is loaded. `pub(crate)` so `DemoPage` can merge in a
/// loaded profile's own `declared_left_operands`.
pub(crate) const DEFAULT_LEFT_OPERAND_SUGGESTIONS: &[&str] = &["sub", "nationality", "scope", "dateTime"];

fn row_style() -> &'static str {
  "display: flex; align-items: flex-start; gap: 0.5rem; margin-bottom: 0.5rem;"
}

fn input_event_value(e: InputEvent) -> String {
  e.target().and_then(|t| t.dyn_into::<HtmlInputElement>().ok()).map(|el| el.value()).unwrap_or_default()
}

#[derive(Properties, PartialEq)]
pub struct ConstraintRowsEditorProps {
  pub constraints: Vec<ConstraintRow>,
  pub on_change: Callback<Vec<ConstraintRow>>,
}

/// One rule's constraint list: `left_operand` / `operator` / `right_operand`
/// per row, with an "Add constraint" row underneath.
#[component]
pub fn ConstraintRowsEditor(props: &ConstraintRowsEditorProps) -> Html {
  let constraints = props.constraints.clone();
  let on_change = props.on_change.clone();

  let on_add = {
    let constraints = constraints.clone();
    let on_change = on_change.clone();
    Callback::from(move |_: MouseEvent| {
      let mut next = constraints.clone();
      next.push(ConstraintRow::default());
      on_change.emit(next);
    })
  };

  html!(
    <div style="margin-left: 1.25rem; margin-top: 0.35rem;">
      { for constraints.iter().enumerate().map(|(index, row)| {
        let constraints = constraints.clone();
        let on_change = on_change.clone();
        let on_left = {
          let constraints = constraints.clone();
          let on_change = on_change.clone();
          Callback::from(move |e: InputEvent| {
            let mut next = constraints.clone();
            next[index].left_operand = input_event_value(e);
            on_change.emit(next);
          })
        };
        let on_operator = {
          let constraints = constraints.clone();
          let on_change = on_change.clone();
          Callback::from(move |value: Option<String>| {
            let mut next = constraints.clone();
            next[index].operator = value.unwrap_or_else(|| "eq".to_string());
            on_change.emit(next);
          })
        };
        let on_right = {
          let constraints = constraints.clone();
          let on_change = on_change.clone();
          Callback::from(move |value: String| {
            let mut next = constraints.clone();
            next[index].right_operand = value;
            on_change.emit(next);
          })
        };
        let on_remove = {
          let constraints = constraints.clone();
          let on_change = on_change.clone();
          Callback::from(move |_: MouseEvent| {
            let mut next = constraints.clone();
            next.remove(index);
            on_change.emit(next);
          })
        };
        html!(
          <div style={row_style()} key={index}>
            <div class="pf-v6-c-form-control">
              <input
                type="text"
                placeholder="leftOperand (e.g. nationality) -- suggestions only, this stays free-form (Section 4.2)"
                value={row.left_operand.clone()}
                list={LEFT_OPERAND_DATALIST_ID}
                oninput={on_left}
              />
            </div>
            <FormSelect<String> value={Some(row.operator.clone())} onchange={on_operator}>
              { for OPERATORS.iter().map(|(value, description)| yew::html_nested!(
                  <FormSelectOption<String> value={value.to_string()} description={description.to_string()} />
                )) }
            </FormSelect<String>>
            <TextInput placeholder="rightOperand (e.g. DE)" value={row.right_operand.clone()} onchange={on_right} />
            <Button icon={Icon::Trash} variant={ButtonVariant::Plain} aria_label="Remove constraint" onclick={on_remove} />
          </div>
        )
      }) }
      <Button icon={Icon::Plus} variant={ButtonVariant::Link} label="Add constraint" onclick={on_add} />
    </div>
  )
}

#[derive(Properties, PartialEq)]
pub struct RuleRowsEditorProps {
  pub rules: Vec<RuleRow>,
  pub on_change: Callback<Vec<RuleRow>>,
  pub add_label: AttrValue,
  pub action_placeholder: AttrValue,
  /// A loaded profile's `recognized_actions` -- empty when no profile is
  /// loaded, in which case [`ActionField`] behaves exactly like the plain
  /// `TextInput` it replaces (see that component's own doc comment).
  #[prop_or_default]
  pub recognized_actions: Vec<String>,
}

/// A permission/prohibition/obligation list: an action `TextInput` per
/// row plus that row's own [`ConstraintRowsEditor`], with an "Add rule"
/// row underneath. Used identically for all three rule kinds -- what
/// distinguishes them (which `Vec` on `DemoForm` this reads from/writes
/// to) lives entirely in the caller's `rules`/`on_change` wiring.
#[component]
pub fn RuleRowsEditor(props: &RuleRowsEditorProps) -> Html {
  let rules = props.rules.clone();
  let on_change = props.on_change.clone();

  let on_add = {
    let rules = rules.clone();
    let on_change = on_change.clone();
    Callback::from(move |_: MouseEvent| {
      let mut next = rules.clone();
      next.push(RuleRow::default());
      on_change.emit(next);
    })
  };

  html!(
    <div>
      { for rules.iter().enumerate().map(|(index, rule)| {
        let rules = rules.clone();
        let on_change = on_change.clone();
        let on_action = {
          let rules = rules.clone();
          let on_change = on_change.clone();
          Callback::from(move |value: String| {
            let mut next = rules.clone();
            next[index].action = value;
            on_change.emit(next);
          })
        };
        let on_remove = {
          let rules = rules.clone();
          let on_change = on_change.clone();
          Callback::from(move |_: MouseEvent| {
            let mut next = rules.clone();
            next.remove(index);
            on_change.emit(next);
          })
        };
        let on_constraints_change = {
          let rules = rules.clone();
          let on_change = on_change.clone();
          Callback::from(move |value: Vec<ConstraintRow>| {
            let mut next = rules.clone();
            next[index].constraints = value;
            on_change.emit(next);
          })
        };
        html!(
          <div style="border-left: 2px solid var(--pf-t--global--border--color--default, #d2d2d2); padding-left: 0.75rem; margin-bottom: 0.75rem;" key={index}>
            <div style={row_style()}>
              <ActionField
                placeholder={props.action_placeholder.clone()}
                value={rule.action.clone()}
                onchange={on_action}
                recognized_actions={props.recognized_actions.clone()}
              />
              <Button icon={Icon::Trash} variant={ButtonVariant::Plain} aria_label="Remove rule" onclick={on_remove} />
            </div>
            <ConstraintRowsEditor constraints={rule.constraints.clone()} on_change={on_constraints_change} />
          </div>
        )
      }) }
      <Button icon={Icon::Plus} variant={ButtonVariant::Secondary} label={props.add_label.to_string()} onclick={on_add} />
    </div>
  )
}

#[derive(Properties, PartialEq)]
pub struct ActionFieldProps {
  pub value: String,
  pub onchange: Callback<String>,
  pub placeholder: AttrValue,
  /// Empty when no profile is loaded -- see [`ActionField`]'s own doc.
  #[prop_or_default]
  pub recognized_actions: Vec<String>,
}

/// A single action field: the same free-form `TextInput` this page has
/// always used, so with no profile loaded this behaves identically to
/// before this feature existed. Once `recognized_actions` is non-empty
/// (a profile is loaded), it additionally offers a `FormSelect` "insert
/// from profile" picker next to the text field -- reusing the same
/// select component the operator dropdown already uses, for interaction
/// consistency -- and, if the field's current value is non-empty and not
/// among `recognized_actions`, a `HelperText` cue explaining evaluation
/// could produce `Error` (for a rule's own action) or `Deny` (for the
/// top-level requested action, this component's other use -- see
/// `demo_page.rs`'s "Requested Action" section). The text field itself is
/// never restricted to the picker's options: a value picked before this
/// feature existed, or typed by hand, must remain editable, since the cue
/// -- not a closed `<select>` -- is what this task asks to gate on.
#[component]
pub fn ActionField(props: &ActionFieldProps) -> Html {
  let trimmed = props.value.trim();
  let not_recognized = !props.recognized_actions.is_empty() && !trimmed.is_empty() && !props.recognized_actions.iter().any(|a| a == trimmed);
  let state = if not_recognized { InputState::Warning } else { InputState::Default };

  let picker = if props.recognized_actions.is_empty() {
    html!()
  } else {
    let onchange = props.onchange.clone();
    let on_pick = Callback::from(move |value: Option<String>| {
      if let Some(value) = value {
        onchange.emit(value);
      }
    });
    html!(
      <FormSelect<String> value={None::<String>} placeholder="insert from profile..." onchange={on_pick}>
        { for props.recognized_actions.iter().map(|action| yew::html_nested!(
            <FormSelectOption<String> value={action.clone()} description={action.clone()} />
          )) }
      </FormSelect<String>>
    )
  };

  html!(
    <div style="display: flex; flex-direction: column; gap: 0.25rem; flex: 1;">
      <div style="display: flex; align-items: flex-start; gap: 0.5rem;">
        <TextInput placeholder={props.placeholder.clone()} value={props.value.clone()} state={state} onchange={props.onchange.clone()} />
        { picker }
      </div>
      if not_recognized {
        <HelperText>
          <HelperTextItem variant={HelperTextItemVariant::Warning} icon={HelperTextItemIcon::Visible}>
            { format!("{trimmed:?} is not among the loaded profile's declared actions -- this can change the evaluation outcome: Error if this is a rule's own action (Section 4.4's unrecognized-action check), or Deny if this is the requested action and nothing declared covers it.") }
          </HelperTextItem>
        </HelperText>
      }
    </div>
  )
}

#[derive(Properties, PartialEq)]
pub struct ClaimRowsEditorProps {
  pub claims: Vec<ClaimRow>,
  pub on_change: Callback<Vec<ClaimRow>>,
}

/// The claims editor: a key/value row per claim, with a `Switch` toggling
/// whether the value is one string or a comma-separated list mapped to a
/// JSON array on submit (Section 4.1's `ClaimValue::Single`/`Multi`).
#[component]
pub fn ClaimRowsEditor(props: &ClaimRowsEditorProps) -> Html {
  let claims = props.claims.clone();
  let on_change = props.on_change.clone();

  let on_add = {
    let claims = claims.clone();
    let on_change = on_change.clone();
    Callback::from(move |_: MouseEvent| {
      let mut next = claims.clone();
      next.push(ClaimRow::default());
      on_change.emit(next);
    })
  };

  html!(
    <div>
      { for claims.iter().enumerate().map(|(index, row)| {
        let claims = claims.clone();
        let on_change = on_change.clone();
        let on_key = {
          let claims = claims.clone();
          let on_change = on_change.clone();
          Callback::from(move |value: String| {
            let mut next = claims.clone();
            next[index].key = value;
            on_change.emit(next);
          })
        };
        let on_value = {
          let claims = claims.clone();
          let on_change = on_change.clone();
          Callback::from(move |value: String| {
            let mut next = claims.clone();
            next[index].value = value;
            on_change.emit(next);
          })
        };
        let on_toggle_list = {
          let claims = claims.clone();
          let on_change = on_change.clone();
          Callback::from(move |checked: bool| {
            let mut next = claims.clone();
            next[index].is_list = checked;
            on_change.emit(next);
          })
        };
        let on_remove = {
          let claims = claims.clone();
          let on_change = on_change.clone();
          Callback::from(move |_: MouseEvent| {
            let mut next = claims.clone();
            next.remove(index);
            on_change.emit(next);
          })
        };
        let value_placeholder = if row.is_list { "comma-separated values (e.g. catalog:read, sparql:read)" } else { "value (e.g. DE)" };
        html!(
          <div style={row_style()} key={index}>
            <TextInput placeholder="claim key (e.g. nationality)" value={row.key.clone()} onchange={on_key} />
            <TextInput placeholder={value_placeholder} value={row.value.clone()} onchange={on_value} />
            <Switch checked={row.is_list} label="list" onchange={on_toggle_list} />
            <Button icon={Icon::Trash} variant={ButtonVariant::Plain} aria_label="Remove claim" onclick={on_remove} />
          </div>
        )
      }) }
      <Button icon={Icon::Plus} variant={ButtonVariant::Link} label="Add claim" onclick={on_add} />
    </div>
  )
}
