//! Library half of `profile-interpreter`: `src/main.rs` is a thin CLI shell
//! over this crate so `site/` (a wasm32 target with no filesystem/CLI) can
//! call `graph::Graph::from_turtle`/`from_json_ld` and `interpret::interpret`
//! directly instead of shelling out to a binary that doesn't exist in a
//! browser.

pub mod graph;
pub mod interpret;
