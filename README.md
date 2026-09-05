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
