//! Writes `compliance/reports/release-history.json`: the per-release
//! dashboard the site's Release History page renders.
//!
//! Stage 2 of a two-stage pipeline. `scripts/build-release-history.sh` is
//! stage 1: it checks every tag out in a detached, isolated git worktree,
//! builds that tag's `engine.wasm`, runs that tag's own
//! `compliance-runner` against the ODRL-Test-Suite revision that tag
//! pinned, and leaves both results in a staging directory. This binary
//! reads that staging directory, replays the **current** ODRL 2.2
//! coverage catalog (`compliance/reports/latest-coverage.json`, read
//! fresh off disk below -- never a count baked in here, which would only
//! ever describe the catalog as it stood on some earlier day) against
//! each historical `engine.wasm` through its own four-export C ABI, and
//! renders the artifact.
//!
//! ```text
//! scripts/build-release-history.sh
//! cargo run -p release-history --release
//! ```
//!
//! (The script prints the staging path it used; pass it as this binary's
//! first argument if you overrode the default.)
//!
//! **Why running today's catalog against an old binary is meaningful —
//! and the one range where it is not.** Most wire-shape changes in this
//! engine's history were strictly additive: each new request field
//! arrived with `#[serde(default)]`, so a request carrying fields an
//! older engine never heard of deserializes cleanly *in that engine's own
//! deserializer*, with the unknown keys ignored, and what comes back is a
//! real answer to "did this release actually do this?".
//!
//! That premise was tested rather than assumed, and it does **not** hold
//! all the way back. v0.6.0 reshaped `config` from the bare
//! `{"recognized_actions": [...]}` object earlier revisions used into
//! real JSON-LD vocabulary (`@type`/`@id`/`odrl:action`/
//! `odrl:includedIn`) — a rename, not an addition, and
//! `RequestConfig::recognized_actions` had no `#[serde(default)]` to fall
//! back on. Every request in the current catalog is therefore refused by
//! a v0.5.0-or-earlier engine with `missing field
//! \`recognized_actions\``, before a single line of policy logic runs.
//!
//! This binary detects that rather than papering over it: a release whose
//! deserializer rejected *every* request is recorded with `coverage:
//! null` and the engine's own rejection message, not as a release that
//! contradicted every probeable vocabulary row. Partial rejection is the
//! opposite case and is kept as real signal — a release with no
//! `isAllOf` variant in its `Operator` enum rejects exactly the
//! `isAllOf` probes and answers every other probe normally, which is
//! exactly the kind of historical fact this dashboard exists to show.
//! Both counts are on every release (`envelope_rejected`), so a reader
//! can see which is which.
//!
//! A `contradicted` row for an old release is the expected, wanted
//! signal: the current catalog documents a capability that release did
//! not have yet. Each one is recorded with the engine's own `reason`
//! string so a reader can tell a genuinely absent capability from a
//! harness artefact.

mod host;
mod render;

/// The Coverage page's own logic — catalog parsing, per-probe
/// classification, per-row verdict derivation, and the tally — included
/// by path rather than reimplemented.
///
/// This is deliberate and load-bearing. The alternative (a second copy of
/// `classify_probe`/`derive_verdict` living here) would mean the historical
/// dashboard and the live Coverage page could silently drift into
/// disagreeing about what "contradicted" means, and the one release whose
/// numbers *can* be cross-checked — the HEAD tag, which the live page also
/// runs — would be the only place that drift ever showed up. `site` is a
/// binary crate with a wasm-only dependency set, so it cannot be a normal
/// path dependency; `#[path]` gets the exact same source text compiled
/// into this generator instead. The module is browser-free by
/// construction (its own header says so, and `cargo test --workspace`
/// already runs its unit tests natively for that reason), and its only
/// dependencies are `serde` and `serde_json`, both already here.
///
/// One consequence worth knowing: `cargo test --workspace` now runs that
/// module's unit tests twice, once in each crate. That is a duplicate
/// test run, not a duplicate implementation.
#[path = "../../site/src/coverage_catalog.rs"]
#[allow(dead_code)]
mod coverage_catalog;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use coverage_catalog::{
    compile_coverage_report, errored_probe_outcome, evaluated_probe_outcome, parse_coverage_catalog, probe_json,
    CoverageFile, ProbeOutcome, ProbeStatus, RowVerdict,
};
use host::HistoricalEngine;
use render::{
    CatalogInfo, ComplianceTally, ContradictedRow, CoverageTally, HistoryFile, Release, GENERATED_BY, METHOD, NOTE,
    SCHEMA,
};

/// `meta.json`, written per tag by stage 1.
#[derive(Debug, Deserialize)]
struct Meta {
    tag: String,
    commit: String,
    date: String,
    subject: String,
}

/// As much of a historical `compliance/reports/latest.json` as this
/// dashboard reports. Those four fields have been in that artifact's
/// shape since v0.1.0 (checked against every tag), which is what makes
/// one reader work across the whole range.
#[derive(Debug, Deserialize)]
struct HistoricalCompliance {
    total: u64,
    passed: u64,
    failed: u64,
    skipped: u64,
}

/// Sorts `vMAJOR.MINOR.PATCH` the way a human reads it, so v0.10.0 comes
/// after v0.9.0 rather than before it (which is what a plain string sort
/// does, and what makes a release table look subtly wrong).
fn version_key(tag: &str) -> (u64, u64, u64, String) {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = stripped.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0), tag.to_string())
}

/// The exact prefix `engine::wire::parse_error_response` puts on a
/// request its own deserializer refused. Byte-identical in every tag from
/// v0.1.0 to v0.12.1 (checked across the range), which is what lets one
/// detector work over the whole history.
const PARSE_REJECTION_PREFIX: &str = "request did not parse as the documented Section 5.2 JSON shape";

/// Did this probe's request get refused by the historical engine's own
/// deserializer, before any policy logic ran?
///
/// Both halves are required. `decision == "Error"` alone would also match
/// a hypothetical future error decision that is not a parse failure; the
/// reason prefix alone would match an engine that happened to quote the
/// phrase. Together they identify exactly `parse_error_response`'s output.
fn is_envelope_rejection(outcome: &ProbeOutcome) -> bool {
    outcome.decision.as_deref() == Some("Error")
        && outcome.reason.as_deref().is_some_and(|reason| reason.starts_with(PARSE_REJECTION_PREFIX))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Replays every probe in the current catalog against one historical
/// `engine.wasm`.
///
/// A trapping or unjudgeable probe becomes an `Errored` outcome (never a
/// contradiction and never a silent pass) exactly as it would in the
/// browser — `errored_probe_outcome` is the Coverage page's own
/// constructor for that case.
fn replay(catalog: &CoverageFile, wasm: &[u8]) -> Result<(Vec<ProbeOutcome>, f64), String> {
    let mut engine = HistoricalEngine::instantiate(wasm)?;
    let started = Instant::now();
    let mut outcomes = Vec::with_capacity(catalog.probes.len());
    for probe in &catalog.probes {
        let outcome = match engine.evaluate(probe_json(probe)) {
            Ok(response) => evaluated_probe_outcome(probe, &response),
            Err(err) => errored_probe_outcome(probe, &err),
        };
        outcomes.push(outcome);
    }
    Ok((outcomes, started.elapsed().as_secs_f64() * 1000.0))
}

fn stage_release(catalog: &CoverageFile, dir: &Path) -> Result<Release, String> {
    let meta: Meta = serde_json::from_str(
        &fs::read_to_string(dir.join("meta.json")).map_err(|e| format!("{}: {e}", dir.join("meta.json").display()))?,
    )
    .map_err(|e| format!("{}: {e}", dir.join("meta.json").display()))?;

    let wasm = fs::read(dir.join("engine.wasm")).map_err(|e| format!("{}: {e}", dir.join("engine.wasm").display()))?;

    // Absent only when that tag's compliance run genuinely did not
    // complete — stage 1 leaves the stderr behind and omits the file
    // rather than inventing a number.
    let compliance: Option<ComplianceTally> = match fs::read_to_string(dir.join("compliance.json")) {
        Ok(text) => {
            let c: HistoricalCompliance =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", dir.join("compliance.json").display()))?;
            Some(ComplianceTally { total: c.total, passed: c.passed, failed: c.failed, skipped: c.skipped })
        }
        Err(_) => None,
    };

    let (coverage, coverage_error, contradicted_rows) = match replay(catalog, &wasm) {
        Ok((outcomes, elapsed_ms)) => {
            let envelope_rejected = outcomes.iter().filter(|o| is_envelope_rejection(o)).count();

            // Not a capability finding: this release and the current
            // catalog are not speaking the same wire dialect at all, so
            // no probe reached the policy logic and nothing about what
            // this release could or could not decide is observable
            // through it. Reporting the (real, but vacuous) 49
            // contradictions would be 49 restatements of one envelope
            // mismatch dressed up as vocabulary coverage.
            if envelope_rejected == outcomes.len() && !outcomes.is_empty() {
                let first = outcomes
                    .iter()
                    .find_map(|o| o.reason.clone())
                    .unwrap_or_else(|| "request rejected by this release's deserializer".to_string());
                return Ok(Release {
                    tag: meta.tag,
                    date: meta.date,
                    commit: meta.commit,
                    summary: meta.subject,
                    engine_wasm_bytes: wasm.len() as u64,
                    engine_wasm_sha256: sha256_hex(&wasm),
                    compliance,
                    coverage: None,
                    coverage_error: Some(format!(
                        "this release's own deserializer refused all {} of the current catalog's requests \
                         before any policy logic ran, so the catalog cannot address it: {first}",
                        outcomes.len()
                    )),
                    contradicted_rows: Vec::new(),
                });
            }

            let report = compile_coverage_report(catalog, outcomes, elapsed_ms, wasm.len());
            let tally = CoverageTally {
                envelope_rejected,
                probes_total: report.total_probes as usize,
                agreed: report.agreed as usize,
                disagreed: report.disagreed as usize,
                errored: report.errored as usize,
                verified: report.verified as usize,
                contradicted: report.contradicted as usize,
                inconclusive: report.inconclusive as usize,
                documented: report.documented as usize,
            };
            let rows = report
                .rows
                .iter()
                .filter(|row| row.verdict == RowVerdict::Contradicted)
                .map(|row| {
                    let probe = row
                        .probes
                        .iter()
                        .find(|p| p.status == ProbeStatus::Disagreed)
                        .expect("a Contradicted row has at least one disagreeing probe, by derive_verdict");
                    ContradictedRow {
                        id: row.row.id.clone(),
                        category: row.row.category.clone(),
                        term: row.row.term.clone(),
                        documented_status: row.row.status.clone(),
                        probe_id: probe.id.clone(),
                        mismatch: probe.mismatch.clone().unwrap_or_default(),
                        engine_reason: probe.reason.clone().unwrap_or_default(),
                    }
                })
                .collect();
            (Some(tally), None, rows)
        }
        Err(err) => (None, Some(err), Vec::new()),
    };

    Ok(Release {
        tag: meta.tag,
        date: meta.date,
        commit: meta.commit,
        summary: meta.subject,
        engine_wasm_bytes: wasm.len() as u64,
        engine_wasm_sha256: sha256_hex(&wasm),
        compliance,
        coverage,
        coverage_error,
        contradicted_rows,
    })
}

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().expect("release-history has a parent directory").to_path_buf();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let check_determinism = args.iter().any(|a| a == "--check-determinism");
    let stage_dir = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("target/release-history/stage"));

    if !stage_dir.is_dir() {
        eprintln!(
            "no staging directory at {} -- run scripts/build-release-history.sh first (it builds every tag's \
             engine.wasm and runs every tag's own compliance-runner)",
            stage_dir.display()
        );
        std::process::exit(1);
    }

    let catalog_path = repo_root.join("compliance/reports/latest-coverage.json");
    let catalog_text = fs::read_to_string(&catalog_path).unwrap_or_else(|e| {
        eprintln!("{}: {e} -- run `cargo run -p coverage-probes --release` first", catalog_path.display());
        std::process::exit(1);
    });
    let catalog = parse_coverage_catalog(&catalog_text).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    // BTreeMap keyed by parsed version, so the release table's order is a
    // property of the data rather than of readdir().
    let mut staged: BTreeMap<(u64, u64, u64, String), PathBuf> = BTreeMap::new();
    for entry in fs::read_dir(&stage_dir).expect("read staging directory") {
        let entry = entry.expect("read staging entry");
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            staged.insert(version_key(&name), entry.path());
        }
    }
    if staged.is_empty() {
        eprintln!("{} holds no staged releases", stage_dir.display());
        std::process::exit(1);
    }

    let mut releases = Vec::with_capacity(staged.len());
    for dir in staged.values() {
        match stage_release(&catalog, dir) {
            Ok(release) => {
                match (&release.compliance, &release.coverage) {
                    (Some(c), Some(v)) => println!(
                        "{:>8}  compliance {}/{} (skipped {})  rows {} verified / {} contradicted / {} inconclusive \
                         / {} documented  probes {} agreed / {} disagreed / {} errored ({} refused at the \
                         deserializer)",
                        release.tag,
                        c.passed,
                        c.total,
                        c.skipped,
                        v.verified,
                        v.contradicted,
                        v.inconclusive,
                        v.documented,
                        v.agreed,
                        v.disagreed,
                        v.errored,
                        v.envelope_rejected
                    ),
                    (_, None) => println!(
                        "{:>8}  no coverage tally: {}",
                        release.tag,
                        release.coverage_error.as_deref().unwrap_or("unknown")
                    ),
                    (None, _) => println!("{:>8}  no historical compliance run staged", release.tag),
                }
                releases.push(release);
            }
            Err(err) => {
                eprintln!("{}: {err}", dir.display());
                std::process::exit(1);
            }
        }
    }

    let count_status =
        |status: &str| catalog.rows.iter().filter(|row| row.status == status).count();
    let file = HistoryFile {
        schema: SCHEMA,
        generated_by: GENERATED_BY,
        note: NOTE,
        method: METHOD,
        catalog: CatalogInfo {
            generated_by: catalog.generated_by.clone(),
            spec: catalog.spec.clone(),
            source_analysis: catalog.source_analysis.clone(),
            rows: catalog.rows.len(),
            probes: catalog.probes.len(),
            implemented: count_status("Implemented"),
            partial: count_status("Partial"),
            not_implemented: count_status("NotImplemented"),
            out_of_scope: count_status("OutOfScope"),
        },
        releases,
    };

    let text = render::render(&file);

    if check_determinism {
        // Prints the rendered bytes' digest and nothing else, so two
        // independent processes can be diffed without either of them
        // touching the committed artifact. Used to *measure* determinism
        // rather than assert it in prose -- see README.
        println!("sha256 {}", sha256_hex(text.as_bytes()));
        return;
    }

    let out = repo_root.join("compliance/reports/release-history.json");
    fs::write(&out, &text).expect("write release-history.json");
    println!("\nrelease history: {} releases -> {}", file.releases.len(), out.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_numerically_not_lexically() {
        let mut tags = ["v0.9.0", "v0.10.0", "v0.1.0", "v0.12.1", "v0.2.0"];
        tags.sort_by_key(|t| version_key(t));
        assert_eq!(tags, ["v0.1.0", "v0.2.0", "v0.9.0", "v0.10.0", "v0.12.1"]);
    }

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// The committed artifact must be the shape the site parses, and must
    /// actually carry per-release history rather than an empty envelope.
    /// A regeneration that silently produced zero releases (an empty
    /// staging directory, say) would otherwise ship a blank dashboard.
    #[test]
    fn the_committed_artifact_parses_and_carries_every_tag() {
        const COMMITTED: &str = include_str!("../../compliance/reports/release-history.json");
        let value: serde_json::Value = serde_json::from_str(COMMITTED).expect("release-history.json parses");
        assert_eq!(value["schema"], SCHEMA);
        let releases = value["releases"].as_array().expect("releases is an array");
        assert!(releases.len() >= 19, "expected every tag from v0.1.0 onward, found {}", releases.len());
        for release in releases {
            assert!(release["tag"].as_str().is_some_and(|t| t.starts_with('v')));
            assert_eq!(release["engine_wasm_sha256"].as_str().map(str::len), Some(64));
        }
    }
}
