//! Writes `compliance/reports/latest-coverage.json`: the catalog the
//! site's ODRL 2.2 Coverage page fetches and executes, live, against the
//! compiled `engine.wasm` in a visitor's browser.
//!
//! Run it the same way the compliance runner is run:
//!
//! ```text
//! cargo run -p coverage-probes --release
//! ```
//!
//! The artifact lands beside `latest.json`/`latest-cases.json` in
//! `compliance/reports/` deliberately: that directory is already what
//! `pages.yml` redeploys on, already what CI's `git diff --exit-code`
//! covers, and already copied into `dist/compliance-data/` — whose name
//! exists precisely to avoid the route-vs-directory collision
//! `site/index.html` documents at length. A new `dist/coverage/` beside a
//! `/coverage` route would reproduce that exact 307-redirect bug.

mod catalog;
mod patch;
mod render;
mod taxonomy;

use std::fs;
use std::path::PathBuf;

use render::{CoverageFile, GENERATED_BY, NOTE, SCHEMA, SOURCE_ANALYSIS, SPEC};

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().expect("coverage-probes has a parent directory");

    let categories = catalog::categories();
    let rows = catalog::rows();
    let probes = catalog::probes();

    let text = render::render(&CoverageFile {
        schema: SCHEMA,
        generated_by: GENERATED_BY,
        spec: SPEC,
        source_analysis: SOURCE_ANALYSIS,
        note: NOTE,
        categories: &categories,
        rows: &rows,
        probes: &probes,
    });

    let reports_dir = repo_root.join("compliance/reports");
    fs::create_dir_all(&reports_dir).expect("create compliance/reports");
    let path = reports_dir.join("latest-coverage.json");
    fs::write(&path, &text).expect("write latest-coverage.json");

    let live = rows.iter().filter(|row| !row.probe_ids.is_empty()).count();
    println!(
        "ODRL 2.2 coverage catalog: {} rows across {} categories ({live} live-probeable, {} documented-only), \
         {} probes -> {}",
        rows.len(),
        categories.len(),
        rows.len() - live,
        probes.len(),
        path.display()
    );
}
