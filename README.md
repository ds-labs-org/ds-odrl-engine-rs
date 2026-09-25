# ds-odrl-engine-rs

A portable WebAssembly ODRL Policy Decision Engine: a pure, stateless
`(policy, claims) -> decision` evaluator compiled to `wasm32-unknown-unknown`
and invoked identically from a Rust host (`wasmi`) or a JVM host (Chicory)
through a minimal four-export ABI over guest linear memory. It is a
companion module for [ds-labs-org/ds-catalog-broker-rs](https://github.com/ds-labs-org/ds-catalog-broker-rs),
built to sit behind that project's opt-in policy-enforcement hook (or any
other host willing to speak its JSON wire contract).

It implements the design proposed in the ds42.org dataspace study's case
study, filed at
`docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md`
in the [Deepthought-Solutions/dataspace](https://github.com/Deepthought-Solutions/dataspace)
repository ("Attribute-Based ODRL Policy Enforcement over Eclipse EDC").
That document is the authoritative source for *why* each decision below
was made; see [`docs/design-rationale.md`](docs/design-rationale.md) for
this repo's own summary and honest scope boundary — **this is not a full
ODRL implementation**.

## Documentation

Each topic below is a standalone reference doc under [`docs/`](docs/),
also browsable on the live site once deployed
(<https://ds-labs-org.github.io/ds-odrl-engine-rs/docs>).

| Topic | In this repo | Live site |
|---|---|---|
| What this is not, and the design rationale | [docs/design-rationale.md](docs/design-rationale.md) | [/docs/design-rationale](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/design-rationale) |
| Wire contract, logical constraints, action refinement, per-rule targets | [docs/wire-contract.md](docs/wire-contract.md) | [/docs/wire-contract](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/wire-contract) |
| Per-permission duties, consequences and remedies | [docs/duties-consequences-remedies.md](docs/duties-consequences-remedies.md) | [/docs/duties-consequences-remedies](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/duties-consequences-remedies) |
| Party-role evaluation (`odrl:assignee`, opt-in) | [docs/party-role-evaluation.md](docs/party-role-evaluation.md) | [/docs/party-role-evaluation](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/party-role-evaluation) |
| Conflict strategy and policy inheritance | [docs/conflict-and-inheritance.md](docs/conflict-and-inheritance.md) | [/docs/conflict-and-inheritance](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/conflict-and-inheritance) |
| Detailed evaluation (`evaluate_request_detailed`) | [docs/detailed-evaluation.md](docs/detailed-evaluation.md) | [/docs/detailed-evaluation](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/detailed-evaluation) |
| Introspection: claims and actions | [docs/introspection.md](docs/introspection.md) | [/docs/introspection](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/introspection) |
| Profile interpretation and DSP ingestion | [docs/profiles-and-ingestion.md](docs/profiles-and-ingestion.md) | [/docs/profiles-and-ingestion](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/profiles-and-ingestion) |
| Compliance methodology and suite attribution | [docs/compliance-methodology.md](docs/compliance-methodology.md) | [/docs/compliance-methodology](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/compliance-methodology) |
| Release history dashboard methodology | [docs/release-history-methodology.md](docs/release-history-methodology.md) | [/docs/release-history-methodology](https://ds-labs-org.github.io/ds-odrl-engine-rs/docs/release-history-methodology) |

## Building and testing

```sh
cargo build --workspace
cargo test --workspace
```

`dsp-odrl-adapter`'s capability is behind a default-off Cargo feature; to
exercise its own 32 tests too:

```sh
cargo test --workspace --features dsp-odrl-adapter/dsp-ingest
```

WebAssembly guest module (no WASI — pure JSON-in/JSON-out):

```sh
rustup target add wasm32-unknown-unknown   # once
cargo build -p engine --target wasm32-unknown-unknown --release
# -> target/wasm32-unknown-unknown/release/engine.wasm
```

## Running the compliance suite

```sh
cargo run -p compliance-runner
```

Adapts every vendored `ODRL-Test-Suite` fixture into this engine's JSON
wire contract, evaluates it natively, and (re)writes
[`compliance/reports/latest.md`](compliance/reports/latest.md),
`latest.json`, and `latest-cases.json` (the exported corpus an
independent host can re-run). See
[`docs/compliance-methodology.md`](docs/compliance-methodology.md) for the
full methodology and current pass/fail numbers.

## Documentation and demonstrator site

`site/` is a Yew + Trunk single-page app: a landing page, an in-browser
demonstrator that evaluates a request against a *real* compiled
`engine.wasm` over its raw C ABI, a Compliance Results page that re-runs
the whole vendored corpus live in your browser, a Capability Audit page
that checks this study's own documentation claims, an ODRL 2.2 Full
Compliance page that checks the live engine against the spec ideal
instead, a Release History dashboard, and this new docs browser. Run it
locally:

```sh
cd site && trunk serve
```

Then open <http://localhost:8080>. Deployed automatically to GitHub Pages
on every push to `main` that touches `site/`, `engine/`, or
`compliance/reports/` — live at
<https://ds-labs-org.github.io/ds-odrl-engine-rs/>, with the Compliance
Results at `/compliance`, the Capability Audit at `/coverage`, the ODRL
2.2 Full Compliance page at `/full-compliance`, and the Release History
dashboard at `/history`.

## Current compliance summary

As of the fixtures currently vendored (68 cases), from the native
`compliance-runner` run recorded in
[`compliance/reports/latest.json`](compliance/reports/latest.json):

| total | passed | failed | skipped |
|---|---|---|---|
| 68 | 68 | 0 | 0 |

Reproducible without trusting this file: the site's Compliance Results
page re-runs all 68 exported cases against the compiled `engine.wasm` in
your own browser and cross-checks the same tally. See
[`docs/compliance-methodology.md`](docs/compliance-methodology.md) for the
full account, including a real regression this corpus once caught.

## Comparing against other ODRL engines

`bench/` holds reproducible harnesses for a five-engine comparison against
the identical 68-fixture corpus above. Full comparative analysis is
published as
[a benchmark report](https://github.com/Deepthought-Solutions/dataspace/blob/main/docs/benchmarks/2026-09-06-odrl-engine-comparative-coverage.md)
in the sibling `dataspace` repository.

## License

Licensed under the Apache License, Version 2.0 — see [LICENSE](LICENSE).
