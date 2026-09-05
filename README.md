# ds-odrl-engine-rs

A portable WebAssembly ODRL Policy Decision Engine, companion to
[ds-labs-org/ds-catalog-broker-rs](https://github.com/ds-labs-org/ds-catalog-broker-rs).
Its design and rationale are documented in the ds42.org dataspace study's
case study,
`docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md`
("Attribute-Based ODRL Policy Enforcement over Eclipse EDC"). More will
follow.

This repository is dual-licensed MIT/Apache-2.0 Rust, following
ds-labs-org conventions.

`compliance/vendor/odrl-test-suite` vendors
[SolidLabResearch/ODRL-Test-Suite](https://github.com/SolidLabResearch/ODRL-Test-Suite)
as a git submodule — upstream compliance-suite fixtures, pinned at
checkout time (commit `7958238e72511059478e43ec9e57b053504cfd2c`, checked
out 2026-09-05) — see that commit sha for provenance.

## `compliance-runner`

`cargo run -p compliance-runner` adapts every `(policy, request,
state-of-the-world, expected-report)` fixture the vendored suite indexes
in `data/index.ttl` into `engine`'s Section 5.2 JSON request contract,
calls `engine::evaluate_request` natively (no WASM host needed for this),
and writes `compliance/reports/latest.md` and `latest.json` — pass/fail/
skip counts, a table of any failing cases (expected vs. actual decision
and why), and a table of skipped cases, each citing the specific Section 7
limitation that makes it unrepresentable in this engine's current wire
contract (numeric/date-time operators, nested `odrl:and`/`or`/`xone`
groups, party/asset-collection membership, per-permission nested duties,
ODRL action implication). A case is only ever skipped for one of those
named, cited reasons — never to avoid a fail.

**RDF stack**: parsing uses `oxrdf`/`oxttl` (the Oxigraph project)
throughout — `oxttl::TurtleParser` yields `oxrdf::Triple`/`Term` directly,
and `compliance-runner/src/graph.rs` is a thin, generic lookup layer over
`Vec<Triple>`, not a conversion to strings or a hand-rolled parser. This
follows `ds-catalog-broker-rs`'s own `rdf-store` crate, which already
standardizes the organization on Oxigraph, rather than introducing a
second RDF stack (`sophia`, `rio_turtle`, or similar) for one runner.
`oxigraph`'s full in-memory `Store`/SPARQL layer is deliberately not
used: every vendored fixture is a handful of triples, so plain iteration
over parsed triples is simpler than standing up a queryable store for
lookups no more elaborate than "objects of this subject/predicate."

See `compliance-runner/src/translate.rs` for the adapter's own stated
translation convention (there is no requested-action/target parameter in
`engine`'s wire contract at all, so a host — here, the runner itself —
must already have scoped a policy's rules to the one action/target under
evaluation before calling it) and `compliance-runner/src/ground_truth.rs`
for how a single Allow/Deny verdict is derived from the vendored suite's
own `report:*` compliance-report vocabulary.
