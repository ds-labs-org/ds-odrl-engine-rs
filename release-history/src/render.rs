//! The `compliance/reports/release-history.json` artifact's shape.
//!
//! Unlike `coverage-probes`' catalog and `compliance-runner`'s
//! `latest-cases.json` — both of which deliberately carry *no* decision,
//! because the browser recomputes every one of them live — this artifact
//! carries results, and has to: the site cannot recompute them. Doing so
//! in a visitor's browser would mean shipping and instantiating nineteen
//! historical `engine.wasm` binaries (3.9 MB of them) and running 2,375
//! probe evaluations on page load, to reproduce numbers that can only
//! change when someone cuts a new tag. The page says so in as many words,
//! and links here.
//!
//! What keeps it honest instead is that every number in it was produced by
//! re-execution rather than by transcription: the compliance figures are
//! whatever that tag's own `compliance-runner` printed when it was re-run
//! against the ODRL-Test-Suite revision that tag pinned, and the coverage
//! figures come from driving that tag's compiled `engine.wasm` through its
//! own ABI. `engine_wasm_sha256` is recorded per release so a reader can
//! rebuild the tag and check they are looking at the same binary.

use serde::Serialize;
use serde_json::Value;

/// Version tag for this file's shape, checked by the site before it
/// trusts a fetched artifact — same reasoning as the coverage catalog's
/// own `SCHEMA`: `copy-file` assets are not content-hashed, so a
/// returning visitor can be handed a browser-cached copy of an older
/// shape, and that must fail loudly rather than half-parse.
///
/// Bumped `@2` -> `@3` when [`Release::row_status`]/[`RowStatusBreakdown`]
/// were retired in favour of [`Release::full_compliance`]/
/// [`FullComplianceTally`]: the per-release stacked chart and the line
/// chart's row/probe series on `/history` now measure against full ODRL
/// 2.2 compliance (`site/src/full_compliance.rs`'s own judgment) rather
/// than against this study's documentation (`site/src/coverage_catalog.rs`'s
/// Agreed/Disagreed axis) — a real shape change, not an additive one, so a
/// stale cached `@2` artifact must fail loudly rather than render a chart
/// built from fields that no longer exist.
pub const SCHEMA: &str = "ds-odrl-engine-rs/release-history@3";

pub const GENERATED_BY: &str =
    "release-history (scripts/build-release-history.sh, then cargo run -p release-history --release)";

pub const NOTE: &str = "Build-time historical record, not a live in-browser run. Every compliance figure is \
                        what that tag's own compliance-runner printed when re-executed against the \
                        ODRL-Test-Suite revision that tag pinned; every coverage figure comes from driving \
                        that tag's compiled engine.wasm through its own alloc/dealloc/evaluate ABI with the \
                        current probe catalog.";

pub const METHOD: &str = "One detached git worktree per tag: build engine.wasm for wasm32-unknown-unknown \
                          --release, run compliance-runner --release, then replay the current \
                          latest-coverage.json catalog against that binary in a wasmi interpreter. The \
                          per-row/per-probe outcomes are derived by site/src/coverage_catalog.rs, and the \
                          full-ODRL-2.2-compliance tally by site/src/full_compliance.rs -- the same two \
                          modules the Capability Audit and ODRL 2.2 Full Compliance pages run in the \
                          browser -- both included here by path, not reimplemented.";

/// The current catalog these historical engines were replayed against.
/// Recorded so the page can state which catalog produced the numbers: a
/// re-run after the catalog grows will legitimately move every release's
/// figures at once, and a reader has to be able to see that.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogInfo {
    pub generated_by: String,
    pub spec: String,
    pub source_analysis: String,
    pub rows: usize,
    pub probes: usize,
    /// The documented-status distribution of the catalog's own rows —
    /// a property of the catalog, identical for every release, which is
    /// exactly why it lives here and not on each release.
    pub implemented: usize,
    pub partial: usize,
    pub not_implemented: usize,
    pub out_of_scope: usize,
    /// Rows `/full-compliance` judges at all — the catalog's rows minus the
    /// ones documented `OutOfScope` (see `full_compliance::is_in_scope`).
    /// Also a static catalog property, identical for every release.
    pub full_compliance_rows_in_scope: usize,
    pub full_compliance_rows_excluded: usize,
    /// Distinct probes named by at least one in-scope row — smaller than
    /// `probes` above whenever a probe is named only by an excluded row.
    pub full_compliance_probes_judged: usize,
    pub full_compliance_probes_in_catalog: usize,
}

/// One tag's ODRL-Test-Suite result, as that tag's own runner reported it.
#[derive(Debug, Clone, Serialize)]
pub struct ComplianceTally {
    pub total: u64,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
}

/// One tag's replay of the current catalog.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageTally {
    pub probes_total: usize,
    pub agreed: usize,
    pub disagreed: usize,
    pub errored: usize,
    pub verified: usize,
    pub contradicted: usize,
    pub inconclusive: usize,
    pub documented: usize,
    /// How many of this release's probe requests its own deserializer
    /// refused outright, before any policy logic ran.
    ///
    /// Recorded separately because it is the one number that says whether
    /// a contradiction means anything. A handful of rejections is a real
    /// capability signal — a release whose `Operator` enum has no
    /// `isAllOf` variant rejects exactly the `isAllOf` probes and answers
    /// the rest normally, which is precisely the historical fact this
    /// dashboard exists to show. *All* of them being rejected is not a
    /// capability signal at all: it means the catalog and that release
    /// are not speaking the same wire dialect, and `coverage` is left
    /// null in that case rather than reporting 49 contradictions that
    /// only restate one envelope mismatch. See `Release::coverage_error`.
    pub envelope_rejected: usize,
}

/// One release's own reading against **full ODRL 2.2 compliance**, not
/// against this study's documentation -- the same distinction
/// `site/src/full_compliance.rs`'s own module doc comment draws between
/// `/coverage` and `/full-compliance`. Derived by replaying that release's
/// probe outcomes through `full_compliance::compile_full_compliance_report`
/// -- the exact function the live `/full-compliance` page calls in the
/// browser, included here by path rather than reimplemented, for the same
/// no-drift reason `coverage_catalog.rs` is.
///
/// Unlike [`CatalogInfo`]'s static, catalog-wide counts, these fields vary
/// release to release: they are what *that release's own compiled
/// `engine.wasm`* actually answered, judged against the spec-ideal target
/// rather than against the documented one. Row counts always sum to
/// [`CatalogInfo::full_compliance_rows_in_scope`] when present at all.
/// `None` on a [`Release`] exactly when [`Release::coverage`] is `None` --
/// a release the current catalog cannot address at all has no probe
/// outcomes to judge either, same reasoning as [`Release::coverage_error`].
#[derive(Debug, Clone, Serialize)]
pub struct FullComplianceTally {
    pub rows_meets: usize,
    pub rows_falls_short: usize,
    pub rows_structural_gap: usize,
    pub rows_undetermined: usize,
    pub probes_meets: usize,
    pub probes_falls_short: usize,
    pub probes_undetermined: usize,
}

/// A row the current catalog documents as holding, that this historical
/// engine did not satisfy. Expected, and the whole point: it names a
/// capability the release genuinely did not have yet.
#[derive(Debug, Clone, Serialize)]
pub struct ContradictedRow {
    pub id: String,
    pub category: String,
    pub term: String,
    /// The status the *current* catalog documents for this row.
    pub documented_status: String,
    /// The first probe of this row that disagreed, and how — the engine's
    /// own words, so a reader can tell a missing capability from a
    /// harness bug without leaving the page.
    pub probe_id: String,
    pub mismatch: String,
    pub engine_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub tag: String,
    pub date: String,
    pub commit: String,
    /// The tag commit's own subject line: what actually shipped, in the
    /// words of the commit that shipped it, not a summary written later.
    pub summary: String,
    pub engine_wasm_bytes: u64,
    pub engine_wasm_sha256: String,
    /// `None` only if that tag's compliance run could not be reproduced —
    /// which is then stated on the page rather than shown as a zero.
    pub compliance: Option<ComplianceTally>,
    /// `None` if that tag's `engine.wasm` could not be driven at all, or
    /// if it rejected every single one of the catalog's requests at its
    /// own deserializer — see `coverage_error`.
    pub coverage: Option<CoverageTally>,
    /// Why this release has no coverage tally, when it has none. Stated
    /// in the engine's own words wherever the engine had any.
    pub coverage_error: Option<String>,
    pub contradicted_rows: Vec<ContradictedRow>,
    /// This release's own full-ODRL-2.2-compliance tally -- `None` exactly
    /// when `coverage` is `None`. See [`FullComplianceTally`] for what this
    /// is and is not.
    pub full_compliance: Option<FullComplianceTally>,
}

#[derive(Debug, Serialize)]
pub struct HistoryFile {
    pub schema: &'static str,
    pub generated_by: &'static str,
    pub note: &'static str,
    pub method: &'static str,
    pub catalog: CatalogInfo,
    pub releases: Vec<Release>,
}

/// Pretty-printed and routed through `serde_json::to_value` first, for
/// exactly the reason `coverage-probes/src/render.rs` documents at
/// length: `Value::Object` is a `BTreeMap` in this workspace (nothing
/// enables serde_json's `preserve_order`), so the conversion sorts every
/// object's keys and the artifact cannot pick up key-order noise from a
/// `HashMap`-backed field. Nothing in *this* file is `HashMap`-backed
/// today — but the inputs are (`engine::Request::claims` is, and probe
/// requests flow through this crate as raw bytes precisely so nothing
/// re-serializes them), and the guarantee should not rest on a
/// coincidence of the current struct layout. Determinism is asserted
/// empirically rather than argued: see `main.rs`'s
/// `--check-determinism` mode, which renders the whole artifact twice in
/// two independent processes and diffs the bytes.
pub fn render(file: &HistoryFile) -> String {
    let value: Value = serde_json::to_value(file).expect("HistoryFile always serializes");
    let mut text = serde_json::to_string_pretty(&value).expect("a serde_json::Value always serializes");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HistoryFile {
        HistoryFile {
            schema: SCHEMA,
            generated_by: GENERATED_BY,
            note: NOTE,
            method: METHOD,
            catalog: CatalogInfo {
                generated_by: "coverage-probes".to_string(),
                spec: "spec".to_string(),
                source_analysis: "analysis".to_string(),
                rows: 52,
                probes: 125,
                implemented: 11,
                partial: 18,
                not_implemented: 16,
                out_of_scope: 7,
                full_compliance_rows_in_scope: 45,
                full_compliance_rows_excluded: 7,
                full_compliance_probes_judged: 120,
                full_compliance_probes_in_catalog: 125,
            },
            releases: vec![Release {
                tag: "v0.1.0".to_string(),
                date: "2026-09-05T09:56:59+02:00".to_string(),
                commit: "deadbeef".to_string(),
                summary: "first tag".to_string(),
                engine_wasm_bytes: 198862,
                engine_wasm_sha256: "abc".to_string(),
                compliance: Some(ComplianceTally { total: 68, passed: 20, failed: 0, skipped: 48 }),
                coverage: Some(CoverageTally {
                    probes_total: 125,
                    agreed: 40,
                    disagreed: 80,
                    errored: 5,
                    verified: 10,
                    contradicted: 35,
                    inconclusive: 4,
                    documented: 3,
                    envelope_rejected: 0,
                }),
                coverage_error: None,
                contradicted_rows: vec![],
                full_compliance: Some(FullComplianceTally {
                    rows_meets: 31,
                    rows_falls_short: 13,
                    rows_structural_gap: 1,
                    rows_undetermined: 0,
                    probes_meets: 105,
                    probes_falls_short: 15,
                    probes_undetermined: 0,
                }),
            }],
        }
    }

    #[test]
    fn object_keys_are_canonically_sorted_in_the_emitted_bytes() {
        let text = render(&sample());
        let catalog_at = text.find("\"catalog\"").expect("catalog key present");
        let schema_at = text.find("\"schema\"").expect("schema key present");
        assert!(
            catalog_at < schema_at,
            "serde_json::Value's BTreeMap sorts keys, so `catalog` must precede `schema` in the emitted \
             bytes regardless of struct field order -- if this fails, the Value indirection was dropped \
             and the artifact is no longer guaranteed deterministic"
        );
    }

    #[test]
    fn the_envelope_carries_its_schema_and_ends_in_a_newline() {
        let text = render(&sample());
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["schema"], SCHEMA);
        assert_eq!(value["releases"].as_array().unwrap().len(), 1);
        assert!(text.ends_with("}\n"), "committed artifacts end in exactly one newline");
    }

    #[test]
    fn a_release_that_could_not_be_driven_serializes_its_reason_rather_than_a_zero() {
        let mut file = sample();
        file.releases[0].coverage = None;
        file.releases[0].coverage_error = Some("engine.wasm exports no `evaluate`".to_string());
        file.releases[0].full_compliance = None;
        let value: Value = serde_json::from_str(&render(&file)).unwrap();
        assert!(value["releases"][0]["coverage"].is_null());
        assert_eq!(value["releases"][0]["coverage_error"], "engine.wasm exports no `evaluate`");
        assert!(
            value["releases"][0]["full_compliance"].is_null(),
            "full_compliance must be null exactly when coverage is, same as coverage_error's own reasoning"
        );
    }

    #[test]
    fn a_releases_full_compliance_tally_round_trips_and_its_rows_sum_to_the_catalogs_in_scope_count() {
        let file = sample();
        let value: Value = serde_json::from_str(&render(&file)).unwrap();
        let full_compliance = &value["releases"][0]["full_compliance"];
        assert_eq!(full_compliance["rows_meets"], 31);
        assert_eq!(full_compliance["rows_falls_short"], 13);
        assert_eq!(full_compliance["rows_structural_gap"], 1);
        assert_eq!(full_compliance["rows_undetermined"], 0);
        let sum = full_compliance["rows_meets"].as_u64().unwrap()
            + full_compliance["rows_falls_short"].as_u64().unwrap()
            + full_compliance["rows_structural_gap"].as_u64().unwrap()
            + full_compliance["rows_undetermined"].as_u64().unwrap();
        assert_eq!(sum, file.catalog.full_compliance_rows_in_scope as u64);
    }
}
