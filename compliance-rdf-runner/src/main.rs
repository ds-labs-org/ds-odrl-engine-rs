//! CI-integrated compliance runner for the `ds-odrl-compliance-rdf` RDF
//! test corpus (vendored as a git submodule at
//! `compliance/vendor/ds-odrl-compliance-rdf`) -- the real successor to
//! this project's earlier by-hand verification of each case file against
//! a scratch `engine::evaluate_request_detailed` call. Every `cases/*.ttl`
//! file is parsed, translated into the engine's real `wire::Request`
//! (`translate.rs`), evaluated for real, and its actual
//! `DetailedEvaluation`/`Response` compared against the file's own
//! `dsc:expectedOutcome` tree (`expected.rs`/`compare.rs`). Exits non-zero
//! if any case fails, for CI gating (`.github/workflows/ci.yml`).
//!
//! Modeled loosely on `compliance-runner`'s own CLI shape (parse, run,
//! report pass/fail, exit code) -- a new, separate binary, not required to
//! match it exactly, since it drives a structurally different corpus (one
//! self-contained `dsc:` file per case vs. that corpus's split
//! policy/request/state-of-the-world triple).

mod compare;
mod expected;
mod graph;
mod translate;
mod vocab;

use std::path::PathBuf;

use graph::Graph;
use vocab::*;

enum CaseOutcome {
    Passed,
    Failed(Vec<String>),
    Error(String),
}

fn run_case(path: &std::path::Path) -> CaseOutcome {
    let g = match Graph::parse(path) {
        Ok(g) => g,
        Err(e) => return CaseOutcome::Error(e),
    };

    let testcase = match g.subject_with_type(&dsc_TestCase()) {
        Some(t) => t,
        None => return CaseOutcome::Error("no dsc:TestCase root node found".to_string()),
    };

    let request_node = match g.object_id(&testcase, &dsc_request()) {
        Some(r) => r,
        None => return CaseOutcome::Error(format!("{testcase}: no dsc:request")),
    };

    let (request, policy_ids) = match translate::translate_request(&g, &request_node) {
        Ok(r) => r,
        Err(e) => return CaseOutcome::Error(format!("translating dsc:request: {e}")),
    };

    let expected_outcome_nodes = g.object_ids(&testcase, &dsc_expectedOutcome());
    if expected_outcome_nodes.is_empty() {
        return CaseOutcome::Error(format!("{testcase}: no dsc:expectedOutcome"));
    }
    let expected = match expected_outcome_nodes
        .iter()
        .map(|n| expected::parse_expected_policy_report(&g, n))
        .collect::<Result<Vec<_>, String>>()
    {
        Ok(e) => e,
        Err(e) => return CaseOutcome::Error(format!("parsing dsc:expectedOutcome: {e}")),
    };

    let expected_decision = match expected::parse_expected_decision(&g, &testcase) {
        Ok(d) => d,
        Err(e) => return CaseOutcome::Error(e),
    };

    let duty_mode = request.config.duty_mode;
    let (response, detailed) = engine::evaluate_request_detailed(&request);

    match compare::compare_case(
        &policy_ids,
        &expected,
        duty_mode,
        expected_decision,
        &detailed,
        &response,
    ) {
        Ok(mismatches) if mismatches.is_empty() => CaseOutcome::Passed,
        Ok(mismatches) => CaseOutcome::Failed(mismatches),
        Err(e) => CaseOutcome::Error(e),
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .expect("compliance-rdf-runner has a parent directory");
    let cases_dir = repo_root.join("compliance/vendor/ds-odrl-compliance-rdf/cases");

    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&cases_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ttl"))
            .collect(),
        Err(e) => {
            eprintln!("failed to read {}: {e}", cases_dir.display());
            std::process::exit(1);
        }
    };
    entries.sort();

    if entries.is_empty() {
        eprintln!("no .ttl case files found under {}", cases_dir.display());
        std::process::exit(1);
    }

    let mut passed = 0;
    let mut failed = 0;
    let mut errored = 0;

    for path in &entries {
        let slug = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        match run_case(path) {
            CaseOutcome::Passed => {
                passed += 1;
                println!("PASS  {slug}");
            }
            CaseOutcome::Failed(mismatches) => {
                failed += 1;
                println!("FAIL  {slug}");
                for m in mismatches {
                    println!("        - {m}");
                }
            }
            CaseOutcome::Error(e) => {
                errored += 1;
                println!("ERROR {slug}");
                println!("        - {e}");
            }
        }
    }

    println!(
        "ds-odrl-compliance-rdf compliance: {} total, {passed} passed, {failed} failed, {errored} errored",
        entries.len()
    );

    if failed > 0 || errored > 0 {
        std::process::exit(1);
    }
}
