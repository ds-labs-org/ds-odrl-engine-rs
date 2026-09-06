//! Section 5.2's JSON wire contract, restated on the *site* side as its
//! own small serde types. This crate deliberately has no Rust-level link
//! to the `engine` crate (see `site/Cargo.toml`'s header comment) -- the
//! Demonstrator page has to build the exact same request shape a real,
//! unrelated JS or JVM host would, from scratch, over the documented
//! contract alone. `engine::wire::{Request, WirePolicy, ...}` is what this
//! mirrors; keep the two in sync by hand if Section 5.2 ever changes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One atomic ODRL constraint (Section 4.2/5.2). `operator` is one of the
/// seven lowercase strings the engine recognizes: `eq`, `neq`, `isAnyOf`,
/// `lt`, `lteq`, `gt`, `gteq` -- kept as a plain `String` here (rather
/// than an enum) so a value the demo's own `<select>` didn't anticipate
/// still round-trips to the raw-JSON preview unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
  pub left_operand: String,
  pub operator: String,
  pub right_operand: String,
}

/// One permission/prohibition/obligation rule: an action plus the
/// constraints that gate it (Section 5.2's shared rule shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
  pub action: String,
  pub constraints: Vec<Constraint>,
}

/// One policy exactly as Section 5.2 documents it on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
  pub id: String,
  pub kind: String,
  pub assigner: String,
  pub assignee: Option<String>,
  pub permissions: Vec<Rule>,
  pub prohibitions: Vec<Rule>,
  pub obligations: Vec<Rule>,
}

/// A JSON-LD reference to another node by IRI -- `{"@id": "..."}`, used
/// here for `WireActionDecl`'s `odrl:includedIn`. Mirrors
/// `engine::wire::WireNodeRef`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNodeRef {
  #[serde(rename = "@id")]
  pub id: String,
}

/// One entry of `RequestConfig`'s `odrl:action` list. Mirrors
/// `engine::wire::WireActionDecl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireActionDecl {
  #[serde(rename = "@id")]
  pub id: String,
  #[serde(rename = "odrl:includedIn", default, skip_serializing_if = "Option::is_none")]
  pub included_in: Option<WireNodeRef>,
}

/// Section 5.2's `config` object -- real ODRL/JSON-LD vocabulary
/// (`@type`/`@id`/`odrl:action`/`odrl:includedIn`), not the earlier bare
/// `{"recognized_actions": [...], "duty_mode": ...}` shape. `dutyMode`
/// stays outside the `odrl:` namespace deliberately -- see
/// `engine::wire::RequestConfig`'s own doc comment: ODRL defines no
/// property for a profile's own enforcement behavior, so namespacing it as
/// real ODRL vocabulary would misrepresent this engine's own invention.
/// `behaviour` (new) is the ODRL Community Group's own Formal Semantics
/// axis (Section 3.6) -- `"open"` (an empty `permissions` list is
/// vacuously met, Section 4.3's own original default) or `"closed"`
/// (nothing actively permits denies, full stop).
///
/// `engine::wire::RequestConfig`'s fourth setting, `partyIdentityClaim`,
/// is deliberately **not** mirrored: it switches on comparison of a
/// policy's `odrl:assignee` against the caller, and the Demonstrator's
/// form has no assignee field to compare against (`demo_form.rs`'s own
/// module doc). Omitting the key is that capability's documented default,
/// so every request this page builds evaluates exactly as it would have
/// before the key existed. See `site/README.md`'s "Known limitations".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestConfig {
  #[serde(rename = "@type")]
  pub type_: String,
  #[serde(rename = "@id")]
  pub id: String,
  #[serde(rename = "odrl:action")]
  pub actions: Vec<WireActionDecl>,
  #[serde(rename = "dutyMode")]
  pub duty_mode: String,
  pub behaviour: String,
}

/// A single claim's value: either one string, or a list of strings for a
/// multi-valued claim (Section 4.1) -- `#[serde(untagged)]` so a single
/// value serializes as a bare JSON string and a list as a JSON array,
/// matching `engine::claims::ClaimValue` exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClaimValue {
  Single(String),
  Multi(Vec<String>),
}

/// Section 5.2's request envelope. `action` (new) is the one action the
/// whole request is about -- evaluated against every policy's own rule
/// actions via `engine::ResolvedConfig::covers`, distinct from a rule's own
/// declared `action` on `Rule` above. `claims` is a `BTreeMap` (not the
/// insertion-ordered map the form itself keeps) purely for a
/// deterministic raw-JSON preview -- the engine itself is indifferent to
/// key order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
  pub dataset_id: String,
  pub action: String,
  pub config: RequestConfig,
  pub policies: Vec<Policy>,
  pub claims: BTreeMap<String, ClaimValue>,
}

/// One entry of Section 5.2's `duties` list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DutyEntry {
  pub policy_id: String,
  pub action: String,
  pub resolved: bool,
}

/// Section 5.2's response envelope. `decision` is kept as a plain
/// `String` (`"Allow"`/`"Deny"`/`"Error"`) rather than an enum -- serde's
/// default unit-variant encoding already produces exactly those three
/// strings, and a `String` here is one fewer type to keep in sync with
/// `engine::wire::WireDecision` for a value this page only ever compares
/// against those three literals or displays verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Response {
  #[allow(dead_code)] // carried through for completeness; the page keys its badge off `decision` alone
  pub dataset_id: String,
  pub decision: String,
  pub reason: String,
  pub duties: Vec<DutyEntry>,
}
