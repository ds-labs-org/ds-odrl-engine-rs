//! CLI: turns a Dataspace Protocol contract offer/agreement (or a bare
//! ODRL policy document) into the JSON this engine's wire contract
//! (Section 5.2) actually consumes. A thin shell over `dsp_odrl_adapter`'s
//! library half, exactly as `profile-interpreter`'s own `main.rs` is.
//!
//! Usage:
//!   dsp-odrl-adapter ingest <file>
//!     -> prints one engine::wire::WirePolicy as JSON
//!   dsp-odrl-adapter request <file> --dataset-id <iri> --action <action>
//!                     [--claim key=value]... [--duty-mode advise|deny]
//!                     [--behaviour open|closed]
//!     -> prints a complete Section 5.2 Request around that policy, ready
//!        to feed engine::evaluate_request (or engine.wasm's `evaluate`)
//!   dsp-odrl-adapter contexts
//!     -> lists the @context URLs this build can resolve offline
//!
//! Warnings go to stderr, JSON to stdout, so the output pipes cleanly.

use std::path::PathBuf;
use std::process::ExitCode;

use engine::claims::{ClaimValue, Claims};
use engine::{Behaviour, DutyMode};

use dsp_odrl_adapter::{bundled_context_urls, ingest_policy, request_for};

/// Every JSON artifact this binary prints goes through
/// `serde_json::to_value` first. `engine::Claims` is `HashMap`-backed, so
/// serializing a `Request` directly would emit its claims in whatever order
/// that map's per-instance `RandomState` produced — a non-deterministic
/// artifact. `serde_json::Value::Object` is a `BTreeMap` in this workspace
/// (no crate here enables the `preserve_order` feature), so routing through
/// it key-sorts every object exactly once. A real, previously-shipped bug
/// in this repository was exactly this step being skipped.
fn print_canonical<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let canonical = serde_json::to_value(value).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&canonical).map_err(|e| e.to_string())?);
    Ok(())
}

fn parse_duty_mode(s: &str) -> Result<DutyMode, String> {
    match s {
        "advise" => Ok(DutyMode::Advise),
        "deny" => Ok(DutyMode::Deny),
        other => Err(format!("--duty-mode must be \"advise\" or \"deny\", got {other:?}")),
    }
}

fn parse_behaviour(s: &str) -> Result<Behaviour, String> {
    match s {
        "open" => Ok(Behaviour::Open),
        "closed" | "default" => Ok(Behaviour::Closed),
        other => Err(format!("--behaviour must be \"open\", \"closed\", or \"default\", got {other:?}")),
    }
}

/// `--claim key=value`, repeatable. A key given more than once becomes a
/// multi-valued claim (`engine::ClaimValue::Multi`), which is the shape
/// `scope`/`nationality` already use in Section 4.1.
fn add_claim(claims: &mut Claims, arg: &str) -> Result<(), String> {
    let (key, value) = arg.split_once('=').ok_or_else(|| format!("--claim expects key=value, got {arg:?}"))?;
    match claims.remove(key) {
        None => {
            claims.insert(key.to_string(), ClaimValue::Single(value.to_string()));
        }
        Some(ClaimValue::Single(first)) => {
            claims.insert(key.to_string(), ClaimValue::Multi(vec![first, value.to_string()]));
        }
        Some(ClaimValue::Multi(mut values)) => {
            values.push(value.to_string());
            claims.insert(key.to_string(), ClaimValue::Multi(values));
        }
    }
    Ok(())
}

struct Args {
    files: Vec<PathBuf>,
    dataset_id: Option<String>,
    action: Option<String>,
    claims: Claims,
    duty_mode: DutyMode,
    behaviour: Behaviour,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        files: Vec::new(),
        dataset_id: None,
        action: None,
        claims: Claims::new(),
        duty_mode: DutyMode::Advise,
        behaviour: Behaviour::Closed,
    };
    let mut i = 0;
    while i < args.len() {
        let need = |name: &str| args.get(i + 1).cloned().ok_or_else(|| format!("{name} needs a value"));
        match args[i].as_str() {
            "--dataset-id" => {
                parsed.dataset_id = Some(need("--dataset-id")?);
                i += 2;
            }
            "--action" => {
                parsed.action = Some(need("--action")?);
                i += 2;
            }
            "--claim" => {
                add_claim(&mut parsed.claims, &need("--claim")?)?;
                i += 2;
            }
            "--duty-mode" => {
                parsed.duty_mode = parse_duty_mode(&need("--duty-mode")?)?;
                i += 2;
            }
            "--behaviour" => {
                parsed.behaviour = parse_behaviour(&need("--behaviour")?)?;
                i += 2;
            }
            other => {
                parsed.files.push(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(parsed)
}

fn run() -> Result<(), String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        return Err(
            "usage: dsp-odrl-adapter <ingest|request|contexts> <file> [--dataset-id <iri>] \
             [--action <action>] [--claim key=value]... [--duty-mode advise|deny] \
             [--behaviour open|closed]"
                .to_string(),
        );
    }
    let command = argv[0].clone();
    if command == "contexts" {
        for url in bundled_context_urls() {
            println!("{url}");
        }
        return Ok(());
    }

    let args = parse_args(&argv[1..])?;
    let [path] = args.files.as_slice() else {
        return Err(format!("{command} takes exactly one input document, got {}", args.files.len()));
    };
    let body = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ingested = ingest_policy(&body).map_err(|e| format!("{}: {e}", path.display()))?;
    for warning in &ingested.warnings {
        eprintln!("warning: {warning}");
    }

    match command.as_str() {
        "ingest" => print_canonical(&ingested.policy),
        "request" => {
            let dataset_id = args.dataset_id.ok_or("request needs --dataset-id (the request's own odrl:target)")?;
            let action = args.action.ok_or("request needs --action")?;
            let request = request_for(
                &ingested.policy,
                &dataset_id,
                &action,
                args.claims,
                args.duty_mode,
                args.behaviour,
            );
            print_canonical(&request)
        }
        other => Err(format!("unknown command {other:?} — expected \"ingest\", \"request\" or \"contexts\"")),
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
