//! Ingests the ODRL policy carried by a Dataspace Protocol contract
//! offer/agreement — `odrl:`-prefixed (or bare-term) JSON-LD with a real
//! `@context` — into `engine`'s own Section 5.2 wire shape
//! (`engine::wire::WirePolicy`). An adapter, exactly like
//! `compliance-runner` and `profile-interpreter`: nothing here changes the
//! engine, its wire contract, or its four-export WASM ABI.
//!
//! **The whole crate is behind the default-off `dsp-ingest` Cargo
//! feature.** Without it this library is empty and has no dependencies;
//! see `README.md` for why a compile-time feature rather than a runtime
//! switch, and for the precise scope boundary of this first cut.

#[cfg(feature = "dsp-ingest")]
mod jsonld;

#[cfg(feature = "dsp-ingest")]
mod ingest;

#[cfg(feature = "dsp-ingest")]
pub use ingest::{ingest_policy, ingest_policy_value, minimal_config, request_for, IngestError, Ingested};

#[cfg(feature = "dsp-ingest")]
pub use jsonld::{bundled_context_urls, ODRL_NS};
