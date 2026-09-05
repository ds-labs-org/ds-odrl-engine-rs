# ds-odrl-engine-rs

A portable WebAssembly ODRL Policy Decision Engine: a pure, stateless
`(policy, claims) -> decision` evaluator compiled to `wasm32-unknown-unknown`
and invoked identically from a Rust host (`wasmi`) or a JVM host (Chicory)
through a minimal four-export ABI over guest linear memory. It is a
companion module for [ds-labs-org/ds-catalog-broker-rs](https://github.com/ds-labs-org/ds-catalog-broker-rs),
built to sit behind that project's opt-in policy-enforcement hook (or any
other host willing to speak its JSON wire contract).

## What this is not

This is **not a full ODRL implementation**. `engine`'s own Default Profile
has seven constraint operators now (`eq`/`neq`/`isAnyOf`, plus `lt`/
`lteq`/`gt`/`gteq` for UTC `dateTime` comparison) over a flat string/
string-array claims model, and exact-string action recognition with one
narrow, vocabulary-sourced exception (`odrl:use` covers everything except
the transfer-category actions) — no general `includedIn`/`implies`
inference otherwise. Nested `odrl:and`/`odrl:or` logical constraints and
`odrl:PartyCollection`/`odrl:AssetCollection` membership are resolved by
`compliance-runner`'s own adapter (DNF expansion into sibling/combined
rules; SOTW-graph `odrl:partOf` lookups) rather than by any change to
`engine`'s wire contract — a real host would need the equivalent adapter
logic, not just this engine. `odrl:xone` remains genuinely unsupported (no
"exactly one" exclusivity), and per-permission `odrl:duty` is resolved
only by reading this specific compliance suite's own SOTW-embedded
`report:DutyReport` fact — `engine` itself still evaluates policy-level
obligations only (Section 4.5); this is not general per-permission duty
modeling. See the design rationale below for what's load-bearing versus
what's compliance-suite-specific, and
[`compliance/reports/latest.md`](compliance/reports/latest.md) for
exactly which constructs pass, fail, or are skipped today, case by case,
against a real external ODRL test suite.

**Known adapter fragility, not exercised by the vendored corpus** (found
by an independent review of v0.2.0, none of it changes the 68/68 result
since no vendored fixture triggers them — recorded here rather than
silently left for the next person to rediscover):

- `translate.rs`'s `is_member_of`/`duty_is_violated` match SOTW-graph
  nodes by **local name only**, not full IRI — a same-named node in a
  different namespace would false-positive a membership or duty-state
  check. A **blank-node** `odrl:duty` is worse: the policy and SOTW files
  are parsed as separate graphs, so a parser-assigned blank-node label
  can't be relied on to correlate between them, meaning a violated duty
  on a blank-node-identified rule could silently stay in play. The vendored
  corpus only uses `urn:uuid:`-identified duties, which sidesteps both.
- `graph.rs`'s `first_literal_for_predicate` (used for "now") returns the
  *first* `dct:issued` triple in file order across the whole SOTW graph —
  a second one in some future SOTW fixture would silently win or lose by
  parse order, not by any stated rule.
- `odrl.rs`'s `parse_rule` reads only the *first* `odrl:constraint` and
  `odrl:duty` triple per rule; ODRL permits more than one of each (an
  implicit AND). A rule with two constraint triples would silently drop
  one rather than combine them.
- An IRI-valued `odrl:rightOperand` (rather than a literal) is read as an
  empty string by `literal_value`, which is a silent miss — fail-open for
  a prohibition's constraint, fail-closed for a permission's.
- `odrl.rs` never parses a Policy-level `odrl:obligation` at all —
  `WirePolicy.obligations` is always empty from this adapter, independent
  of Section 4.5's own duty-mode support in `engine`.
- `duty_is_violated`'s exclusion is applied identically to prohibitions
  as to permissions; a violated duty attached to a *prohibition* would
  drop the prohibition (fail-open) rather than the intended asymmetric
  handling ODRL's own `odrl:remedy` construct implies. Untested by this
  corpus — no vendored fixture attaches a duty to a prohibition.

None of these are hard to fix; they're recorded because the corpus
passing 68/68 does not mean they don't exist, and a future contributor
extending the vendored fixtures (or pointing this adapter at a different
policy source) should not have to rediscover them by a wrong verdict.

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

## Producing `config` from a real ODRL Profile document

`config` above has to come from somewhere — [`profile-interpreter`](profile-interpreter/)
reads a real ODRL Profile document (Turtle or JSON-LD) and produces it,
rather than requiring a host to hand-write the JSON:

```sh
cargo run -p profile-interpreter -- interpret my-profile.ttl --duty-mode advise
cargo run -p profile-interpreter -- resolve default-profile.ttl gaia-x-profile.jsonld --duty-mode deny
```

`interpret` reads one document into its own `engine::Profile` record
(Section 4.4's per-profile shape: `id`, `recognized_actions`,
`duty_mode`); `resolve` reads several and merges them with
`engine::resolve()` (union of `recognized_actions`, strictest
`duty_mode`) into exactly the `config` object above. See its own
[README](profile-interpreter/README.md) for precisely what is and isn't
derived from the document — `duty_mode` in particular is never read from
it (ODRL defines no property for that), always a caller-supplied flag.
`profile-interpreter` is also a library (`pub mod graph; pub mod
interpret;`), not just this CLI binary — `site/`'s Demonstrator page
calls it directly to load a pasted profile document in-browser (see
`site/README.md`).

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
(expected vs. actual decision and why), and a table of any skipped cases,
each citing a specific, real reason (today: only `odrl:xone`, or a
constraint operator outside `eq`/`neq`/`isAnyOf`/`lt`/`lteq`/`gt`/`gteq` —
see `translate.rs`'s `unsupported_operator`/`xone_unsupported`). A case is
only ever skipped for one of those named, cited reasons — never to avoid
a fail.

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

## Documentation and demonstrator site

`site/` is a Yew + Trunk single-page app with three pages: a landing page
explaining what this engine is and is not, an in-browser demonstrator
that lets you edit a Section 5.2 request by hand and evaluate it against
a *real* compiled `engine.wasm` (fetched and driven over its raw C ABI —
`alloc`/`dealloc`/`evaluate` — exactly as a JS or JVM host would, with no
Rust-level dependency on the `engine` crate; see `site/README.md` for
why), and a Compliance Results page rendering the vendored suite's
current pass/fail/skip counts from `compliance/reports/latest.json`. It
shares its visual identity (teal brand ramp, monospace heading/code
stack, mesh logo) with the [ds42.org dataspace
study](https://github.com/Deepthought-Solutions/dataspace)'s own docs
site, and every page links back to the case study this engine
implements, filed at
`docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md`
in that repository.

The Demonstrator page can also load a real ODRL Profile document (paste
Turtle or JSON-LD) using `profile-interpreter`'s own parsing logic
client-side — see `site/README.md`'s "Loading a real ODRL Profile
document" section for exactly what that configures (recognized-action
pickers, an inline "not recognized by this profile" cue, and free-form
`leftOperand` suggestions via `<datalist>` — Section 4.2's leftOperand
stays open-ended by design, so this is a suggestion, not a restriction).

Run it locally:

```sh
cd site && trunk serve
```

Then open <http://localhost:8080>. `trunk serve` rebuilds `engine.wasm`
from the `engine` crate's current source on every change (see
`site/Trunk.toml`'s `pre_build` hook) so the demonstrator always reflects
whatever the engine currently does, not a stale compiled snapshot.

The repository's GitHub Actions workflow
(`.github/workflows/pages.yml`) builds this site with `trunk build
--release --public-url /ds-odrl-engine-rs/` and deploys it to GitHub
Pages on every push to `main` that touches `site/`, `engine/`, or
`compliance/reports/`. Its eventual URL will be
<https://ds-labs-org.github.io/ds-odrl-engine-rs/> — **enabling GitHub
Pages itself (repo Settings -> Pages -> Source: GitHub Actions) is a
manual, one-time step that has not been done yet as of this writing**;
until it is, the workflow's deploy job will fail even though the build
succeeds.

## Current compliance summary

As of the fixtures currently vendored (68 cases):

| total | passed | failed | skipped |
|---|---|---|---|
| 68 | 68 | 0 | 0 |

Every vendored case passes, including the largest fixture in the corpus —
`policy-20.ttl`'s "business hours on every weekday of 2024," an
`odrl:or` of 262 `odrl:and`-of-two-`dateTime`-constraints branches,
expanded by `to_dnf` into 262 sibling permission rules and evaluated
exactly like any other. `odrl:use` is recognized as covering
`read`/`write`/`distribute` (per the W3C ODRL Vocabulary's own "Included
In: use" declarations) while correctly excluding transfer-category
actions (`sell`, `give`, `transfer`) — see
`compliance-runner/src/translate.rs`'s module doc for that citation, and
for how `dateTime` constraints, logical `and`/`or` groups, party/asset
collection membership, and per-permission duty state are each resolved
(new `lt`/`lteq`/`gt`/`gteq` operators in `engine`, or SOTW-graph lookups
in the adapter) without weakening the mapping or silently forcing a pass.
Nothing in this corpus exercises `odrl:xone` or a numeric/date-time
operator this Default Profile doesn't have — a case that did would still
be honestly skipped, cited, and counted, not silently dropped. See "What
this is not" above for the real, remaining gap between this engine and a
general ODRL implementation, which is wider than "0 skips" might suggest.

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
