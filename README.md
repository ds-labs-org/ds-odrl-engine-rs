# ds-odrl-engine-rs

A portable WebAssembly ODRL Policy Decision Engine: a pure, stateless
`(policy, claims) -> decision` evaluator compiled to `wasm32-unknown-unknown`
and invoked identically from a Rust host (`wasmi`) or a JVM host (Chicory)
through a minimal four-export ABI over guest linear memory. It is a
companion module for [ds-labs-org/ds-catalog-broker-rs](https://github.com/ds-labs-org/ds-catalog-broker-rs),
built to sit behind that project's opt-in policy-enforcement hook (or any
other host willing to speak its JSON wire contract).

## What this is not

This is **not a full ODRL implementation**. It implements a deliberately
narrowed subset of ODRL's Common Vocabulary and Profile Mechanism — three
constraint operators (`eq`/`neq`/`isAnyOf`) over a flat string/
string-array claims model, exact-string action recognition with no
`includedIn`/`implies` inference, atomic constraints with no nested
`odrl:and`/`or`/`xone` groups, no numeric or date/time comparison, no
`odrl:PartyCollection`/`odrl:AssetCollection` membership, and
policy-level duties only (no per-permission nested duties). Every one of
these gaps is load-bearing design, not an oversight: see the design
rationale below for why, and
[`compliance/reports/latest.md`](compliance/reports/latest.md) for
exactly which constructs pass, fail, or are skipped today, case by case,
against a real external ODRL test suite.

## Design rationale

This engine implements the design proposed in the ds42.org dataspace
study's case study, filed at
`docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md`
in the [Deepthought-Solutions/dataspace](https://github.com/Deepthought-Solutions/dataspace)
repository ("Attribute-Based ODRL Policy Enforcement over Eclipse EDC").
That document is the authoritative source for *why* each decision below
was made — the identity-claims model, the deny-override/permission-
requirement enforcement algorithm, the profile-driven action mechanism,
duty semantics, the WASM ABI's alternatives analysis, and a full
Limitations and Threats to Validity section this README's "What this is
not" summarizes. Read it before extending this engine's scope.

## The wire contract

Section 5.2 of the case study specifies the JSON request/response shape;
this is exactly what `engine::wire::evaluate_request` implements today.

Request:

```json
{
  "dataset_id": "urn:uuid:example-dataset-1",
  "config": {
    "recognized_actions": ["use", "distribute", "notify"],
    "duty_mode": "advise"
  },
  "policies": [
    {
      "id": "policy-1",
      "kind": "Offer",
      "assigner": "did:web:provider.example",
      "assignee": null,
      "permissions": [
        {
          "action": "use",
          "constraints": [
            { "left_operand": "nationality", "operator": "eq", "right_operand": "DE" }
          ]
        }
      ],
      "prohibitions": [],
      "obligations": [
        { "action": "notify", "constraints": [] }
      ]
    }
  ],
  "claims": {
    "sub": "user-42",
    "nationality": "DE",
    "scope": ["catalog:read", "sparql:read"]
  }
}
```

- `config` is the host's already-resolved union of every ODRL Profile it
  has loaded (recognized actions, and the strictest loaded `duty_mode`)
  — resolved once at host startup, travelling in the request so the
  engine itself stays stateless.
- `policies` mirrors the host's own `Policy`/`Rule`/`Constraint` shape
  field for field. `constraints` supports exactly `eq`, `neq`, and
  `isAnyOf` (which splits `right_operand` on commas, with no escaping
  convention). A rule's `constraints` list matches vacuously when empty.
- `claims` is the flat claims map: each value is a JSON string or array
  of strings, sourced from whatever identity the host already trusts —
  this engine never decodes a JWT or other identity-presentation format
  itself.

Response:

```json
{
  "dataset_id": "urn:uuid:example-dataset-1",
  "decision": "Allow",
  "reason": "permission[0] of policy 'policy-1' matched: nationality eq DE",
  "duties": [
    { "policy_id": "policy-1", "action": "notify", "resolved": false }
  ]
}
```

- `decision` is one of `"Allow"`, `"Deny"`, or `"Error"` (an `Error`
  means a rule named an action outside every loaded profile's
  `recognized_actions` — a configuration gap, not a policy decision — and
  a caller **must** treat it as fail-closed).
- `reason` is a short, human-readable trace of which rule or constraint
  drove the outcome. It is diagnostic text, not a machine-parseable
  contract to branch on.
- `duties` lists any policy-level obligation this engine could not
  confirm from the claims it was given; it is empty whenever every duty
  was absent, already satisfied, or (under `duty_mode: "deny"`) already
  forced the decision to `"Deny"`.
- Multiple policies in one request combine by **deny-override across the
  whole set** (`Error` > `Deny` > `Allow`), with an empty `policies` array
  treated as a default deny. This combining rule is this implementation's
  own choice, documented in `engine/src/wire.rs` — the case study leaves
  N-policy combining formally undefined (Section 7).

The wasm32 guest exposes exactly four `extern "C"` exports —
`alloc(len) -> ptr`, `dealloc(ptr, len)`, `evaluate(req_ptr, req_len) ->
packed_ptr_len`, plus the toolchain's default `memory` export — see
`engine/src/abi.rs`. A native host (such as the compliance runner below)
skips the ABI entirely and calls `engine::wire::evaluate_request`
directly.

## Building

Native build and test:

```sh
cargo build --workspace
cargo test --workspace
```

WebAssembly guest module (no WASI — pure JSON-in/JSON-out, no filesystem,
clock, or network):

```sh
rustup target add wasm32-unknown-unknown   # once
cargo build -p engine --target wasm32-unknown-unknown --release
# -> target/wasm32-unknown-unknown/release/engine.wasm
```

## Running the compliance suite

```sh
cargo run -p compliance-runner
```

This adapts every `(policy, request, state-of-the-world, expected-report)`
fixture the vendored suite indexes in `data/index.ttl` into `engine`'s
Section 5.2 JSON request contract, calls `engine::evaluate_request`
natively (no WASM host needed for this), and (re)writes
[`compliance/reports/latest.md`](compliance/reports/latest.md) and
`latest.json` — pass/fail/skip counts, a table of any failing cases
(expected vs. actual decision and why), and a table of skipped cases,
each citing the specific Section 7 limitation of the case study that
makes it unrepresentable in this engine's current wire contract
(numeric/date-time operators, nested `odrl:and`/`or`/`xone` groups,
party/asset-collection membership, per-permission nested duties, ODRL
action implication). A case is only ever skipped for one of those named,
cited reasons — never to avoid a fail.

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

## Current compliance summary

As of the fixtures currently vendored (68 cases):

| total | passed | failed | skipped |
|---|---|---|---|
| 68 | 32 | 0 | 36 |

Zero failures: every case this engine's wire contract can represent at
all currently agrees with the suite's expected verdict. `odrl:use` is
recognized as covering `read`/`write`/`distribute` (per the W3C ODRL
Vocabulary's own "Included In: use" declarations) while correctly
excluding transfer-category actions (`sell`, `give`, `transfer`) — see
`compliance-runner/src/translate.rs`'s module doc for the citation. The
remaining 36 skips are each attributable to one of the ODRL constructs
this engine's Default Profile does not model — general action-taxonomy
implication beyond the `use` special case above, numeric/date-time
constraints, nested logical constraint groups, party/asset-collection
membership, and per-permission nested duties — see the table in
[`compliance/reports/latest.md`](compliance/reports/latest.md) for the
case-by-case citation, and Section 7 of the case study for why each is
out of scope in this revision.

## Compliance suite attribution

`compliance/vendor/odrl-test-suite` vendors
[SolidLabResearch/ODRL-Test-Suite](https://github.com/SolidLabResearch/ODRL-Test-Suite)
(imec, 2019–2025, MIT License) as a git submodule — upstream
compliance-suite fixtures, pinned at checkout time (commit
`7958238e72511059478e43ec9e57b053504cfd2c`, checked out 2026-09-05) — see
that commit sha for provenance.

**Its fixtures are adapted, not run verbatim.** `compliance-runner`
translates each upstream `(policy, request, state-of-the-world,
expected-report)` fixture — expressed in full ODRL/Turtle against that
suite's own vocabulary — into this engine's own narrower Section 5.2 JSON
request contract, which has no notion of a requested action/target
parameter, no RDF, and none of the ODRL constructs listed under "What
this is not" above. A fixture that cannot be represented in that contract
is skipped, cited by name, rather than silently passed or force-fitted.
Upstream license terms apply to the vendored submodule content; they do
not extend to this repository's own code.

## License

Licensed under the Apache License, Version 2.0 — see [LICENSE](LICENSE).
