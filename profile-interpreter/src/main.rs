//! CLI: turns a real ODRL Profile document into the JSON this engine's
//! wire contract (Section 5.2) actually consumes — see `interpret.rs`'s
//! module doc for exactly what is and isn't derived from the document.
//!
//! Usage:
//!   profile-interpreter interpret <file> [--format ttl|jsonld] [--id <uri>] [--duty-mode advise|deny] [--behaviour open|closed]
//!     -> prints one engine::Profile as JSON (this profile's own
//!        actions/duty_mode/behaviour; Section 4.4's per-profile shape —
//!        internal, not wire-shaped, since one profile alone is not a
//!        request config)
//!   profile-interpreter resolve <file>... [--duty-mode advise|deny] [--behaviour open|closed]
//!     -> interprets every file, engine::resolve()s them, and prints the
//!        result as a wire-shaped engine::wire::RequestConfig
//!        (`@type`/`@id`/`odrl:action`/`odrl:includedIn`/`dutyMode`/
//!        `behaviour`) — exactly Section 5.2's request `config` field,
//!        ready to paste in
//!
//! Format is inferred from each file's extension (.ttl/.turtle,
//! .jsonld/.json) unless overridden with --format. --duty-mode and
//! --behaviour default to "advise" and "open" respectively, and neither
//! is ever read from the document itself (see interpret.rs's doc comment
//! for why: ODRL defines no property for either — `behaviour` is the
//! ODRL Community Group's own named concept, but its own Formal
//! Semantics draft describes it as an *evaluator* input, not something a
//! Profile document declares about itself).

use std::path::PathBuf;
use std::process::ExitCode;

use engine::profile::ActionDecl;
use engine::wire::WireActionDecl;
use engine::{Behaviour, DutyMode};

use profile_interpreter::graph::{parse_by_extension, Graph};
use profile_interpreter::interpret::interpret;

/// `resolve` merges N documents, each with its own id — there is no single
/// profile IRI to carry as the merged config's `@id`, and `RequestConfig`'s
/// own doc comment is explicit that this field is "carried for shape, not
/// validated." A fixed, self-describing placeholder rather than a guessed
/// or synthesized IRI.
const RESOLVED_CONFIG_ID: &str = "urn:profile-interpreter:resolved-config";

fn parse_duty_mode(s: &str) -> Result<DutyMode, String> {
    profile_interpreter::interpret::duty_mode_from_str(s)
        .map_err(|_| format!("--duty-mode must be \"advise\" or \"deny\", got {s:?}"))
}

fn parse_behaviour(s: &str) -> Result<Behaviour, String> {
    profile_interpreter::interpret::behaviour_from_str(s)
        .map_err(|_| format!("--behaviour must be \"open\", \"closed\", or \"default\", got {s:?}"))
}

fn load(path: &PathBuf, format: Option<&str>) -> Result<Graph, String> {
    match format {
        Some("ttl") | Some("turtle") => {
            Graph::from_turtle(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        }
        Some("jsonld") | Some("json") => {
            Graph::from_json_ld(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        }
        Some(other) => Err(format!("--format must be \"ttl\" or \"jsonld\", got {other:?}")),
        None => parse_by_extension(path),
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Err("usage: profile-interpreter <interpret|resolve> <file>... [--format ttl|jsonld] [--id <uri>] [--duty-mode advise|deny]".to_string());
    }
    let command = args.remove(0);

    let mut files = Vec::new();
    let mut format: Option<String> = None;
    let mut id: Option<String> = None;
    let mut duty_mode = DutyMode::Advise;
    let mut behaviour = Behaviour::Open;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                format = Some(args.get(i + 1).ok_or("--format needs a value")?.clone());
                i += 2;
            }
            "--id" => {
                id = Some(args.get(i + 1).ok_or("--id needs a value")?.clone());
                i += 2;
            }
            "--duty-mode" => {
                duty_mode = parse_duty_mode(args.get(i + 1).ok_or("--duty-mode needs a value")?)?;
                i += 2;
            }
            "--behaviour" => {
                behaviour = parse_behaviour(args.get(i + 1).ok_or("--behaviour needs a value")?)?;
                i += 2;
            }
            other => {
                files.push(PathBuf::from(other));
                i += 1;
            }
        }
    }
    if files.is_empty() {
        return Err("no input file given".to_string());
    }

    match command.as_str() {
        "interpret" => {
            if files.len() != 1 {
                return Err("interpret takes exactly one file — did you mean `resolve` for multiple?".to_string());
            }
            let g = load(&files[0], format.as_deref())?;
            let interpreted = interpret(&g, id, duty_mode, behaviour);
            for warning in &interpreted.warnings {
                eprintln!("warning: {warning}");
            }
            println!("{}", serde_json::to_string_pretty(&interpreted.profile).map_err(|e| e.to_string())?);
            Ok(())
        }
        "resolve" => {
            if id.is_some() {
                return Err("--id doesn't apply to `resolve` (it interprets multiple files, each with its own detected/placeholder id) — use `interpret` for a single file with a known id".to_string());
            }
            let mut profiles = Vec::new();
            for path in &files {
                let g = load(path, format.as_deref())?;
                let interpreted = interpret(&g, None, duty_mode, behaviour);
                for warning in &interpreted.warnings {
                    eprintln!("warning ({}): {warning}", path.display());
                }
                profiles.push(interpreted.profile);
            }
            // `engine::ResolvedConfig` deliberately keeps its merged
            // `actions` list private (profile.rs: "exposing the raw list
            // would invite a caller to reimplement one of [recognizes/
            // covers] slightly differently") — only `duty_mode` is public,
            // so that part comes from `engine::resolve` itself; the
            // actions union is redone here using the exact same rule
            // `resolve()` documents (dedup by id, first profile wins),
            // since this CLI's whole job is producing that union as wire
            // JSON, which `ResolvedConfig` has no accessor for.
            let resolved = engine::resolve(&profiles);
            let mut actions: Vec<ActionDecl> = Vec::new();
            for profile in &profiles {
                for action in &profile.actions {
                    if !actions.iter().any(|a| a.id == action.id) {
                        actions.push(action.clone());
                    }
                }
            }
            actions.sort_by(|a, b| a.id.cmp(&b.id));
            let config = engine::wire::RequestConfig {
                type_: "odrl:Profile".to_string(),
                id: RESOLVED_CONFIG_ID.to_string(),
                actions: actions.iter().map(WireActionDecl::from).collect(),
                duty_mode: resolved.duty_mode,
                behaviour: resolved.behaviour,
                // Carried from the resolved config for the same reason
                // `duty_mode` and `behaviour` above are — but always `None`
                // here, so the key is never emitted: `engine::resolve`
                // cannot set it, because which claim key carries the
                // caller's identity is host deployment configuration rather
                // than something an ODRL Profile document declares. See
                // `engine::ResolvedConfig::party_identity_claim`; a host
                // wanting party-role scoping adds `partyIdentityClaim` to
                // the config this prints.
                party_identity_claim: resolved.party_identity_claim.clone(),
            };
            println!("{}", serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?);
            Ok(())
        }
        other => Err(format!("unknown command {other:?} — expected \"interpret\" or \"resolve\"")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
