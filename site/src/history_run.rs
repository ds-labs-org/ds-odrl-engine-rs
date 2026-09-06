//! The Release History page's one piece of browser plumbing: fetch the
//! build-time artifact and parse it.
//!
//! Deliberately tiny, and deliberately separate from
//! `history_catalog.rs`. That module is the whole pure half — parsing,
//! validation, and the derived quantities the page draws — and is
//! ungated so `cargo test --workspace` actually runs its tests natively.
//! This module is the part that cannot exist without a browser, and it is
//! one function long precisely because this page has no run to sequence:
//! unlike `compliance_run.rs` and `coverage_run.rs`, there is no
//! `engine.wasm` to instantiate and no corpus to drive here, only a
//! document to load. Giving it a four-stage `ProgressStepper` like those
//! two would be theatre over a single `fetch`.

use crate::history_catalog::{parse_release_history, HistoryFile, HISTORY_URL};
use crate::run_support::fetch_text;

/// Fetches and validates `compliance-data/release-history.json`.
pub async fn fetch_history() -> Result<HistoryFile, String> {
  let text = fetch_text(HISTORY_URL).await?;
  parse_release_history(&text)
}
