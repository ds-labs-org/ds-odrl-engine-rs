//! The Demonstrator page's editable form state: friendlier than a raw-JSON
//! textarea (a labelled row per constraint/rule/claim, with add/remove),
//! short of a full ODRL policy editor (one policy, flat rule lists, no
//! logical constraint groups -- exactly what `engine::wire::Request`
//! itself models). [`to_request`] is the only place that turns this into
//! Section 5.2's actual wire shape.

use std::collections::BTreeMap;

use crate::profile_load::LoadedProfile;
use crate::wire;

/// A single `left_operand`/`operator`/`right_operand` constraint row.
/// `operator` defaults to `"eq"`, the most common case and the first
/// entry the operator `<select>` offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintRow {
  pub left_operand: String,
  pub operator: String,
  pub right_operand: String,
}

impl Default for ConstraintRow {
  fn default() -> Self {
    Self { left_operand: String::new(), operator: "eq".to_string(), right_operand: String::new() }
  }
}

/// One permission/prohibition/obligation row: an action plus its own
/// constraint rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuleRow {
  pub action: String,
  pub constraints: Vec<ConstraintRow>,
}

/// One claims-editor row: a key plus either a single value or (when
/// `is_list` is toggled) a comma-separated value the demo splits into a
/// JSON array on submit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaimRow {
  pub key: String,
  pub value: String,
  pub is_list: bool,
}

/// The whole Demonstrator form: one policy at a time (Section 5.2's own
/// request shape allows a `policies` array, but this demo evaluates one),
/// identified by a fixed `id`/`assignee`-less identity since the task's
/// input list has no field for either -- only `kind` and `assigner` are
/// user-editable, matching how the case study's Section 5.2 example
/// itself only varies those two per policy.
///
/// `action` is the new top-level "requested action" (Section 5.2's
/// `Request.action`) -- what this whole request is *about*, distinct from
/// each permission/prohibition/obligation's own declared action in
/// `permissions`/`prohibitions`/`obligations` below, which `engine::decide`
/// now compares against it via coverage rather than exact identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoForm {
  pub dataset_id: String,
  pub action: String,
  pub recognized_actions: String,
  pub duty_mode: String,
  pub behaviour: String,
  pub policy_kind: String,
  pub assigner: String,
  pub permissions: Vec<RuleRow>,
  pub prohibitions: Vec<RuleRow>,
  pub obligations: Vec<RuleRow>,
  pub claims: Vec<ClaimRow>,
}

/// The wire identity this demo always evaluates a single policy under --
/// not user-editable (see the struct doc above).
const DEMO_POLICY_ID: &str = "policy-1";

impl DemoForm {
  /// Section 5.2's worked example, verbatim, decomposed into this form's
  /// editable rows. Populates the "Load Section 5.2 example" button and
  /// this page's own initial state.
  pub fn example() -> Self {
    Self {
      dataset_id: "urn:uuid:example-dataset-1".to_string(),
      action: "use".to_string(),
      recognized_actions: "use, distribute->use, notify".to_string(),
      duty_mode: "advise".to_string(),
      behaviour: "open".to_string(),
      policy_kind: "Offer".to_string(),
      assigner: "did:web:provider.example".to_string(),
      permissions: vec![RuleRow {
        action: "use".to_string(),
        constraints: vec![ConstraintRow {
          left_operand: "nationality".to_string(),
          operator: "eq".to_string(),
          right_operand: "DE".to_string(),
        }],
      }],
      prohibitions: vec![],
      obligations: vec![RuleRow { action: "notify".to_string(), constraints: vec![] }],
      claims: vec![
        ClaimRow { key: "sub".to_string(), value: "user-42".to_string(), is_list: false },
        ClaimRow { key: "nationality".to_string(), value: "DE".to_string(), is_list: false },
        ClaimRow { key: "scope".to_string(), value: "catalog:read, sparql:read".to_string(), is_list: true },
      ],
    }
  }

  /// An empty starting point for the "Clear" button -- one blank rule/
  /// constraint/claim row each, so the editable rows are visible rather
  /// than needing an "Add" click before the shape of the form is obvious.
  pub fn empty() -> Self {
    Self {
      dataset_id: String::new(),
      action: String::new(),
      recognized_actions: String::new(),
      duty_mode: "advise".to_string(),
      behaviour: "open".to_string(),
      policy_kind: String::new(),
      assigner: String::new(),
      permissions: vec![RuleRow::default()],
      prohibitions: vec![],
      obligations: vec![],
      claims: vec![ClaimRow::default()],
    }
  }
}

/// `pub(crate)`, not private: `demo_page.rs`'s "insert from profile" picker
/// for the `recognized_actions` field needs to check whether an action is
/// already present in the CSV text before appending it.
pub(crate) fn split_csv(raw: &str) -> Vec<String> {
  raw.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Parses one `recognized_actions` CSV token into a `WireActionDecl`. This
/// field has no per-action UI for declaring `odrl:includedIn` (unlike a
/// loaded profile, which carries real edges -- see `to_request` below), so
/// a `child->parent` token is this form's own minimal stand-in: typing
/// `distribute->use` declares `distribute includedIn use`. A token with no
/// `->` is a parentless action, same as before this field could express
/// `includedIn` at all.
fn parse_action_token(token: &str) -> wire::WireActionDecl {
  match token.split_once("->") {
    Some((child, parent)) => wire::WireActionDecl {
      id: child.trim().to_string(),
      included_in: Some(wire::WireNodeRef { id: parent.trim().to_string() }),
    },
    None => wire::WireActionDecl { id: token.trim().to_string(), included_in: None },
  }
}

const CONFIG_ID: &str = "https://example.org/profiles/demonstrator";

fn to_wire_rule(row: &RuleRow) -> wire::Rule {
  wire::Rule {
    action: row.action.trim().to_string(),
    constraints: row
      .constraints
      .iter()
      .map(|c| wire::Constraint {
        left_operand: c.left_operand.trim().to_string(),
        operator: c.operator.clone(),
        right_operand: c.right_operand.trim().to_string(),
      })
      .collect(),
  }
}

/// Builds Section 5.2's request shape from the current form state. Rows
/// with an empty key/action are still included as-is (an empty action
/// simply won't be covered by `config.odrl:action`, surfacing as an
/// ordinary engine-level outcome rather than a client-side validation
/// error) -- the one exception is claim rows, where an empty key has no
/// wire representation at all and is dropped.
///
/// `loaded_profile`'s own declared actions (and their `includedIn` edges)
/// flow into `config.odrl:action` here, not just into the form's
/// suggestion pickers as before -- a profile action already present (by
/// id) wins over a same-named manual `recognized_actions` entry, mirroring
/// `engine::profile::resolve`'s own "first declared wins" merge rule.
pub fn to_request(form: &DemoForm, loaded_profile: Option<&LoadedProfile>) -> wire::Request {
  let mut claims = BTreeMap::new();
  for row in &form.claims {
    let key = row.key.trim();
    if key.is_empty() {
      continue;
    }
    let value = if row.is_list {
      wire::ClaimValue::Multi(split_csv(&row.value))
    } else {
      wire::ClaimValue::Single(row.value.clone())
    };
    claims.insert(key.to_string(), value);
  }

  let mut actions: Vec<wire::WireActionDecl> = Vec::new();
  if let Some(profile) = loaded_profile {
    for action in &profile.actions {
      actions.push(wire::WireActionDecl {
        id: action.id.clone(),
        included_in: action.included_in.clone().map(|id| wire::WireNodeRef { id }),
      });
    }
  }
  for token in split_csv(&form.recognized_actions) {
    let decl = parse_action_token(&token);
    if !actions.iter().any(|a| a.id == decl.id) {
      actions.push(decl);
    }
  }

  wire::Request {
    dataset_id: form.dataset_id.clone(),
    action: form.action.trim().to_string(),
    config: wire::RequestConfig {
      type_: "odrl:Profile".to_string(),
      id: loaded_profile.map(|p| p.id.clone()).unwrap_or_else(|| CONFIG_ID.to_string()),
      actions,
      duty_mode: form.duty_mode.clone(),
      behaviour: form.behaviour.clone(),
    },
    policies: vec![wire::Policy {
      id: DEMO_POLICY_ID.to_string(),
      kind: form.policy_kind.clone(),
      assigner: form.assigner.clone(),
      assignee: None,
      permissions: form.permissions.iter().map(to_wire_rule).collect(),
      prohibitions: form.prohibitions.iter().map(to_wire_rule).collect(),
      obligations: form.obligations.iter().map(to_wire_rule).collect(),
    }],
    claims,
  }
}
