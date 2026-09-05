//! Parses a pasted ODRL Profile document (Turtle or JSON-LD) into the
//! fields the Demonstrator form needs to configure itself -- the non-UI
//! half of the "Load ODRL Profile" panel (see `profile_panel.rs`).
//!
//! Deliberately extracts plain `String`/`Vec<String>` fields into this
//! module's own [`LoadedProfile`] rather than storing `profile_interpreter`'s
//! `Interpreted`/`engine::Profile` directly: `site` has no Rust-level
//! dependency on the `engine` crate itself (see `site/Cargo.toml`'s header
//! comment on why `engine.wasm` stays an opaque compiled artifact), and
//! everything below crosses that boundary by field access on
//! `profile_interpreter`'s own public structs, never by naming
//! `engine::Profile`/`engine::DutyMode` -- `duty_mode_from_str` hands back
//! an opaque `DutyMode` value this module never has to spell the type of.
//! This mirrors `wire.rs`'s own convention of restating a wire/adapter
//! shape independently rather than linking the crate that defines it.

use profile_interpreter::graph::Graph;
use profile_interpreter::interpret::{duty_mode_from_str, interpret};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileFormat {
  Turtle,
  JsonLd,
}

/// Mirrors `engine::ActionDecl` field for field (`id`, `included_in`) --
/// site has no Rust-level dependency on the `engine` crate itself (see
/// this module's own doc comment), so this is `interpreted.profile.actions`
/// copied out by field access, never `engine::ActionDecl` named directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedAction {
  pub id: String,
  pub included_in: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedProfile {
  pub id: String,
  pub actions: Vec<LoadedAction>,
  pub declared_left_operands: Vec<String>,
  pub warnings: Vec<String>,
}

/// Parses `text` per `format`, then interprets it under `duty_mode`
/// (`"advise"`/`"deny"`, the Demonstrator form's own current selection --
/// Section 4.4's `duty_mode` is never read from a profile document itself,
/// see `profile_interpreter::interpret`'s module doc). Always interprets
/// with no `--id`-equivalent override: the panel has no field for one, so
/// a document without its own `odrl:Profile`-typed subject falls back to
/// interpret()'s own placeholder id, surfaced as one of its warnings.
///
/// `interpret()`'s own `behaviour` parameter is passed a fixed value
/// here, not threaded from the form: `Profile.behaviour` only ever gets
/// stored, never branched on, by `interpret()`'s own logic -- it changes
/// nothing this function returns (`id`/`actions`/`declared_left_operands`/
/// `warnings` are all behaviour-independent), so exposing it as a real
/// parameter here would be API surface with no observable effect. The
/// form's own `behaviour` selection reaches the engine directly through
/// `demo_form::to_request`'s `config.behaviour` instead.
pub fn load_profile(text: &str, format: ProfileFormat, duty_mode: &str) -> Result<LoadedProfile, String> {
  let graph = match format {
    ProfileFormat::Turtle => Graph::from_turtle(text.as_bytes()),
    ProfileFormat::JsonLd => Graph::from_json_ld(text.as_bytes()),
  }?;
  let duty_mode = duty_mode_from_str(duty_mode)?;
  let interpreted = interpret(&graph, None, duty_mode, profile_interpreter::interpret::default_behaviour());
  Ok(LoadedProfile {
    id: interpreted.profile.id,
    actions: interpreted
      .profile
      .actions
      .iter()
      .map(|a| LoadedAction { id: a.id.clone(), included_in: a.included_in.clone() })
      .collect(),
    declared_left_operands: interpreted.declared_left_operands,
    warnings: interpreted.warnings,
  })
}
