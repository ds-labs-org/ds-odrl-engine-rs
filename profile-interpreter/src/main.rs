//! CLI: turns a real ODRL Profile document into the JSON this engine's
//! wire contract (Section 5.2) actually consumes — see `interpret.rs`'s
//! module doc for exactly what is and isn't derived from the document.
//!
//! Usage:
//!   profile-interpreter interpret <file> [--format ttl|jsonld] [--id <uri>] [--duty-mode advise|deny]
//!     -> prints one engine::Profile as JSON (this profile's own
//!        recognized_actions/duty_mode; Section 4.4's per-profile shape)
//!   profile-interpreter resolve <file>... [--duty-mode advise|deny]
//!     -> interprets every file, then engine::resolve()s them into one
//!        {recognized_actions, duty_mode} object — exactly Section 5.2's
//!        request `config` field, ready to paste in
//!
//! Format is inferred from each file's extension (.ttl/.turtle,
//! .jsonld/.json) unless overridden with --format. --duty-mode defaults
//! to "advise" and is never read from the document itself (see
//! interpret.rs's doc comment for why).

mod graph;
mod interpret;

use std::path::PathBuf;
use std::process::ExitCode;

use engine::DutyMode;
use serde::Serialize;

use graph::{parse_by_extension, Graph};
use interpret::interpret;

#[derive(Serialize)]
struct ResolvedConfigOutput {
    recognized_actions: Vec<String>,
    duty_mode: DutyMode,
}

fn parse_duty_mode(s: &str) -> Result<DutyMode, String> {
    match s {
        "advise" => Ok(DutyMode::Advise),
        "deny" => Ok(DutyMode::Deny),
        other => Err(format!("--duty-mode must be \"advise\" or \"deny\", got {other:?}")),
    }
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
            let interpreted = interpret(&g, id, duty_mode);
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
                let interpreted = interpret(&g, None, duty_mode);
                for warning in &interpreted.warnings {
                    eprintln!("warning ({}): {warning}", path.display());
                }
                profiles.push(interpreted.profile);
            }
            let resolved = engine::resolve(&profiles);
            let mut recognized_actions: Vec<String> = resolved.recognized_actions.into_iter().collect();
            recognized_actions.sort();
            let output = ResolvedConfigOutput { recognized_actions, duty_mode: resolved.duty_mode };
            println!("{}", serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?);
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
