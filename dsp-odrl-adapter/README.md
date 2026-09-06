# dsp-odrl-adapter

Reads the ODRL policy carried by a **Dataspace Protocol contract
offer/agreement** — `odrl:`-prefixed (or bare-term) JSON-LD with a real
`@context` — and produces this engine's own Section 5.2
`engine::wire::WirePolicy`. An adapter, like `compliance-runner` and
`profile-interpreter`: nothing here changes `engine`, its wire contract, or
its four-export WASM ABI.

**The whole capability is behind the default-off `dsp-ingest` Cargo
feature.** Without it this crate is an empty library with no dependencies
at all.

## Why this exists

`profile-interpreter` reads an ODRL *Profile* document — the vocabulary
declaration — and produces a request's `config`. Nothing in this workspace
read an actual **contract policy**. But a real Dataspace Protocol
connector's `ContractRequestMessage`/`ContractOfferMessage`/
`ContractAgreementMessage` carries its ODRL as JSON-LD, and that policy —
its permissions, prohibitions, constraints and targets — is precisely what
a policy decision engine sitting behind a DSP connector has to evaluate.
Until now a host had to hand-translate it into Section 5.2's JSON by hand,
which is the step where a mistake silently becomes a wrong allow.

## Why a Cargo feature, and not a runtime switch

A compile-time feature, for three reasons, in order of weight:

1. **There is no runtime path for a switch to sit on.** Ingestion happens
   strictly *before* the engine is ever called — it produces the input.
   A runtime toggle would only ever guard a call the host already chose to
   make, which is not a control, it is a comment.
2. **It gates the surface, not just the behaviour.** This adapter parses
   attacker-supplied JSON-LD from a remote connector. A host that does not
   speak DSP should not link that parser at all; with the feature off,
   `dsp-odrl-adapter` compiles to an empty `lib` and even its `engine`,
   `serde` and `serde_json` dependencies are `optional = true` and switched
   off with it (`[features] dsp-ingest = ["dep:engine", "dep:serde",
   "dep:serde_json"]`). The CLI binary is `required-features =
   ["dsp-ingest"]`, so it is not built either.
3. **Default-off is the honest default for something not yet corpus-tested.**
   See the caveat at the bottom of this file: unlike `engine` itself, this
   mapping has never been run against an external DSP conformance suite.
   Shipping it on by default would imply a level of validation it has not
   earned.

A host opts in:

```toml
[dependencies]
dsp-odrl-adapter = { path = "../dsp-odrl-adapter", features = ["dsp-ingest"] }
```

and, for the CLI:

```sh
cargo run -p dsp-odrl-adapter --features dsp-ingest -- ingest <file>
```

`site/` deliberately does **not** depend on this crate, for the same reason
it does not depend on `engine`: see `site/Cargo.toml`'s header comment.

## Usage

Library:

```rust
use dsp_odrl_adapter::{ingest_policy, request_for};
use engine::{Behaviour, DutyMode};

let ingested = ingest_policy(&document_bytes_as_str)?;
for w in &ingested.warnings {
    eprintln!("warning: {w}");   // everything taken verbatim, guessed, or dropped
}
let request = request_for(
    &ingested.policy,
    "urn:uuid:3dd1add8-…",     // the request's own odrl:target
    "use",                      // the action being asked about
    claims,                     // whatever identity the host already trusts
    DutyMode::Advise,
    Behaviour::Closed,
);
let response = engine::evaluate_request(&request);
```

CLI:

```sh
# the ingested policy, as Section 5.2 JSON
cargo run -p dsp-odrl-adapter --features dsp-ingest -- \
  ingest examples/dsp-2024-1-contract-request.jsonld

# a complete Section 5.2 request around it, ready for engine::evaluate_request
cargo run -p dsp-odrl-adapter --features dsp-ingest -- \
  request examples/dsp-2024-1-contract-request.jsonld \
  --dataset-id urn:uuid:3dd1add8-4d2d-569e-d634-8394a8836a88 \
  --action use \
  --claim dateTime=2026-01-01T00:00:00Z \
  --claim purpose=odrl:internal-use-only

# which @context URLs this build can resolve offline
cargo run -p dsp-odrl-adapter --features dsp-ingest -- contexts
```

Warnings go to stderr and JSON to stdout, so the output pipes cleanly.
Every JSON artifact is printed through `serde_json::to_value` first:
`engine::Claims` is `HashMap`-backed, and this workspace's
`serde_json::Value::Object` is a `BTreeMap` (no crate here enables
`preserve_order`), so that step is what makes the printed `request` byte-
identical run to run. It is not a precaution in the abstract — a
previously-shipped artifact in this repository was non-deterministic from
skipping exactly this step, and a direct `serde_json::to_string` of the
same `Request` still reorders its `claims` object between processes today.

## What it ingests

Two document shapes, decided by content rather than by a flag:

- A **DSP negotiation message** whose payload is an ODRL policy —
  `ContractRequestMessage`, `ContractOfferMessage`,
  `ContractAgreementMessage`, or a catalog entry's `odrl:hasPolicy`. The
  policy node is found by its ODRL `@type` (`odrl:Offer`,
  `odrl:Agreement`, `odrl:Set`, …), **not** by a hardcoded `dspace:offer`
  property IRI — which is what lets one code path read both DSP 2024/1
  (`https://w3id.org/dspace/2024/1/offer`) and DSP 2025/1
  (`https://w3id.org/dspace/2025/1/offer`).
- A **bare ODRL policy document**, for a host that already unwrapped the
  message.

A document containing more than one policy node (a whole catalog, say) is a
named error — `IngestError::SeveralPolicyNodes`, listing their `@id`s —
rather than a silent pick of the first. Ingesting a catalog is a separate
job with its own "which offer applies" question.

## The mapping, term by term

| ODRL / JSON-LD | `engine::wire` | Notes |
|---|---|---|
| policy node `@id` | `WirePolicy.id` | empty string + warning if absent |
| `odrl:Offer`/`Agreement`/`Set`/… | `WirePolicy.kind` | local name; `odrl:Policy` maps to `Set` |
| `odrl:assigner` / `odrl:assignee` | `assigner` / `assignee` | opaque strings, as the engine already treats them |
| `odrl:permission[]` | `permissions[]` | array order preserved — it is what `permission[0]` in the engine's `reason` trace means |
| `odrl:prohibition[]` | `prohibitions[]` | |
| `odrl:obligation[]` | `obligations[]` | policy-level duties, which is the only kind `engine` evaluates |
| `odrl:action` (term or `@id`) | `Rule.action` | compacted (see below) |
| `odrl:action` as a node with `rdf:value` + `odrl:refinement` | `Rule.action` + `Rule.action_refinement` | several refinements become one `odrl:and` |
| `odrl:target` on a rule | `Rule.target` (`odrl:target`) | |
| `odrl:target` on the **policy** | pushed down onto every rule that names none | ODRL scopes a policy-level target to its rules; `engine::Rule` has no policy-level target to hold it |
| `odrl:constraint[]` | `Rule.constraints` | |
| `odrl:and` / `odrl:or` / `odrl:xone` | `Constraint::and`/`or`/`xone` | nested to `engine::MAX_CONSTRAINT_DEPTH`, the same bound evaluation stops at; an object setting more than one resolves by the engine's own `xone > or > and` precedence, so an ingested policy decides identically to the same policy hand-written into Section 5.2 JSON |
| `odrl:leftOperand` | `Constraint.left_operand` | compacted (see below) |
| `odrl:operator` | `Constraint.operator` | the ten this engine has; anything else is a named error |
| `odrl:rightOperand` | `Constraint.right_operand` | **never** compacted; several values join with `,`, this engine's own convention for `isAnyOf` and friends |
| `{"@value": v, "@type": t}` | the lexical form of `v` | the datatype is dropped — `right_operand` is one opaque `String` |

### Two naming conventions, and why they differ

- **Vocabulary is compacted out of the ODRL namespace.** An action, a
  policy class and a `leftOperand` are vocabulary, so
  `http://www.w3.org/ns/odrl/2/use` becomes `use` and
  `http://www.w3.org/ns/odrl/2/dateTime` becomes `dateTime`. That is what
  makes an ingested policy line up with the vocabulary the rest of this
  workspace already speaks — `engine`'s own Section 5.2 example,
  `compliance-runner`'s `base_action_vocabulary`,
  `profile-interpreter/examples/odrl-2.2-common-actions.ttl` — and with the
  flat, short claim-map keys `engine::Claims` is built from. An IRI outside
  the ODRL namespace (`https://example.org/claims/region`) is left exactly
  as written.
- **A `rightOperand` is never compacted.** It is *data* — the value a host
  claim is compared against — not vocabulary, so it is carried byte for
  byte, `odrl:`-looking prefixes included.

## Real JSON-LD expansion, and specifically not a prefix strip

The one existing reference implementation of this idea found while scoping
this work — Prometheus-X `odrl-manager`'s
`policy-helper/idsa.parser.json.ts` — recursively strips the literal string
`"odrl:"` from every key **and every string value** in the document. That
is not JSON-LD compaction, and it is wrong in two directions this adapter
had to get right instead:

1. **It corrupts data.** A right operand a provider actually wrote as
   `"odrl:internal-use-only"` arrives as `"internal-use-only"` and silently
   compares unequal to the claim the provider meant. Here the active
   context decides: `odrl:rightOperand` carries no `@type` coercion in any
   of the four bundled contexts, so its value is a plain literal and a
   plain literal is taken exactly as written
   (`a_right_operand_literal_that_starts_with_odrl_is_kept_verbatim`).
2. **It only works on one document shape.** A DSP 2025/1 document has no
   `odrl:` prefix on any key at all — its terms come from a type-scoped
   `@context`. Stripping a prefix that is not there yields nothing.
   `examples/dsp-2024-1-contract-request.jsonld` and
   `examples/dsp-2025-1-contract-request.jsonld` are the same policy in
   those two shapes, sharing not one property-key spelling, and this
   adapter ingests both to one identical `WirePolicy`
   (`the_same_policy_in_the_dsp_2025_1_bare_term_shape_ingests_to_an_identical_wire_policy`).

### What of JSON-LD 1.1 is implemented

`src/jsonld.rs`, and deliberately nothing beyond it:

- context processing: an inline object, an array of contexts merged
  left-to-right, a string reference resolved against the bundled registry
  below, and `"@context": null` as a reset;
- prefix (compact-IRI) expansion, including the spec's own rule that a
  suffix beginning `//` is never a compact IRI — such a string (a key or a
  value) is recognized as *already* an absolute IRI instead and returned
  verbatim, the same as an unmatched `prefix:suffix` like `urn:uuid:…`, so
  writing `"http://www.w3.org/ns/odrl/2/prohibition"` in place of the
  compact `odrl:prohibition` term (legal, RDF-equivalent JSON-LD per the
  W3C ODRL 2.2 context's own 1:1 mapping) ingests identically rather than
  being dropped or, under a document-level `@vocab`, corrupted by
  concatenation;
- `@vocab`;
- term definitions in both forms (`"t": "iri"` and `{"@id": …}`), keyword
  aliases (`"uid": "@id"`), and term removal (`"t": null`);
- `@type` coercion of *values* to `@id` or `@vocab`;
- value objects (`{"@value": …}`);
- **type-scoped `@context`**, with `@import` and `@propagate` — the JSON-LD
  1.1 feature the entire DSP 2025/1 bare-term shape rests on. `@propagate`
  defaults to `false` per spec and is honoured in both directions, not
  assumed `true` because that is what DSP happens to set.

### What is not, and what happens instead

- **Remote context fetching. Never, under any circumstances.** A
  `"@context"` naming a URL outside the bundled registry is
  `IngestError::UnknownContext`, a hard error. Ignoring it would leave
  every term unexpandable and yield an *empty* policy — and a policy that
  lost its prohibitions is fail-open.
- **`@base` and relative-IRI resolution.** A policy that arrived over a
  wire protocol has no reliable document base, so a relative reference is
  taken verbatim and warned about rather than resolved against a guess.
- **Property-scoped contexts, `@container` (`@list`/`@index`/`@language`),
  language maps, `@reverse`, `@nest`, `@graph`, `@json`, `@protected`
  enforcement, term-definition `@prefix` control, blank-node relabelling,
  and RDF conversion.** `@container: @set` is inert here rather than
  unsupported: every position in this adapter already accepts a single
  value or an array.
- **Anything past the policy.** A DSP message's `dspace:providerPid`,
  `dspace:callbackAddress`, `dcat:` catalog structure and the rest expand
  fine and are then ignored — this crate produces a policy, not a
  connector.

### Bundled contexts

Byte-for-byte copies, pinned in `contexts/`, fetched 2026-09-06:

| `@context` URL | file |
|---|---|
| `http://www.w3.org/ns/odrl.jsonld` (and the `https` spelling) | `contexts/w3c-odrl-2.2.jsonld` |
| `https://w3id.org/dspace/2024/1/context.json` | `contexts/dsp-2024-1-context.json` |
| `https://w3id.org/dspace/2025/1/context.jsonld` | `contexts/dsp-2025-1-dspace.jsonld` |
| `https://w3id.org/dspace/2025/1/odrl-profile.jsonld` | `contexts/dsp-2025-1-odrl-profile.jsonld` |

`cargo run -p dsp-odrl-adapter --features dsp-ingest -- contexts` prints
this list from the binary itself. Re-pin by re-fetching the URL and
updating this table's date, the same way `compliance/vendor/` records its
own pin.

## Zero new dependencies, and why not a JSON-LD crate

This crate adds **no third-party dependency at all** beyond `serde`,
`serde_json` and `engine` — the same discipline the `engine` crate holds
itself to, which there was no reason to break here. That was a decision,
not an oversight; two candidates were considered:

- **`oxjsonld`**, already a dependency of `profile-interpreter`, would
  give a real, standards-complete JSON-LD → RDF conversion. But an RDF
  graph has no array order, and `WirePolicy`'s `permissions[0]` /
  `prohibitions[1]` indices — which the engine prints in its own `reason`
  trace — *are* array order. Recovering it would mean re-deriving from
  `rdf:List`/`@set` shapes what the JSON document already states plainly.
- **The `json-ld` crate** is built around a document loader for remote
  contexts, which this adapter must never use (above), so most of its
  machinery would be inert weight around the small part actually needed.

## What is warned about rather than silently dropped

`Ingested.warnings` — the same `{ value, warnings }` shape
`profile-interpreter::interpret` already returns, for the same reason: an
adapter that silently discards what it could not map is an adapter you
cannot audit.

- a coerced value no term, prefix or `@vocab` resolves (taken verbatim);
- a per-rule `odrl:duty` (this engine evaluates policy-level obligations
  only — root README, Section 4.5);
- a rule naming several `odrl:action`s (only the first is ingested);
- an `odrl:profile` declaration (not loaded, so any term it defines stays
  an opaque string) and `odrl:inheritFrom` (policy inheritance is not
  resolved);
- an `odrl:conflict` declaration. The engine really evaluates that term
  now (root README, "Conflict strategy (`odrl:conflict`)"), and this
  adapter ingests none: mapping an IRI-or-literal
  `odrl:perm`/`odrl:prohibit`/`odrl:invalid`, and deciding what an
  unrecognized term should do, is its own decision rather than a side
  effect of the engine gaining the field. The engine's default (`invalid`
  — a policy whose permission and prohibition both match is void) applies
  instead, which is the *opposite* answer for an offer asking for `perm`,
  so the warning names it rather than letting one strategy stand in for
  another;
- a missing `@id` or `odrl:assigner`.

Everything else is an error rather than a warning, all for one reason —
carrying on would leave a rule *less* constrained, or a policy *fewer*
rules, than its author wrote, which is the fail-open direction:

- `IngestError::ConstraintWithoutOperator` / `WithoutLeftOperand` /
  `WithoutRightOperand`. An `"odrl:and": []` lands here too: an empty array
  contributes no values, so it is indistinguishable from an absent one
  after expansion and falls through to the atomic path. Still an error,
  which is the part that matters — an empty `odrl:and` is *vacuously
  satisfied* in this engine, so accepting one would make a permission
  unconditional.
- `IngestError::UnsupportedOperator(iri)` — a real ODRL 2.2 operator this
  engine has no evaluation for (`odrl:isA`, `odrl:hasPart`), or one a
  profile invented.
- `IngestError::RuleIsABareReference(list, iri)` — a rule stated as
  `{"@id": …}`, pointing at a body defined in a document this adapter does
  not resolve. Skipping it would drop a whole prohibition on the floor.
- `IngestError::ConstraintNestedTooDeep(depth)` — past
  `engine::MAX_CONSTRAINT_DEPTH`, the same bound the evaluator itself stops
  recursing at.
- `IngestError::RuleWithoutAction(list)`, `NoPolicyNode`,
  `SeveralPolicyNodes(ids)`, `UnknownContext(url)`, `MalformedContext`,
  `NotANodeObject`, `Json`.

One specific case worth naming, because a reader will hit it: the IDSA
specification's own published `contract-request-message.json` example
writes `odrl:operand` where ODRL 2.2 names the property `odrl:operator`.
This adapter requires the real property and reports
`ConstraintWithoutOperator` for a document using the example verbatim,
rather than quietly accepting the typo and making its own behaviour depend
on it.

## Scope: a policy, not a negotiation

This produces one `WirePolicy`. It does not implement contract
negotiation, does not verify a signature or a Verifiable Credential, does
not resolve `odrl:AssetCollection`/`odrl:PartyCollection` membership (which
stays exactly where the root README already puts it — a host resolving it
against its own graph before the request is built), and does not evaluate
anything: `engine` does that, unchanged.

`minimal_config` is a **floor**, not a profile. It declares every action
the ingested policy's own rules name, purely so `engine` does not answer
`Decision::Error` for a vocabulary gap before it evaluates a single rule.
It declares no `odrl:includedIn` edge, because a contract policy declares
no action taxonomy — so a permission for `transfer` ingested through here
will *not* cover a request for `sell`. A host that wants real
action-taxonomy coverage builds its `config` with `profile-interpreter`
from actual ODRL Profile documents and uses `minimal_config` only as a
fallback.

## Not yet corpus-tested against a real DSP conformance suite

This is worth stating plainly, because everything else in this workspace
is measured against an external corpus and this is not. `engine`'s own
behaviour is checked against 68 vendored SolidLabResearch ODRL-Test-Suite
fixtures (`compliance/reports/latest.md`) and against a 52-row ODRL 2.2
vocabulary coverage catalog. This adapter is checked against **its own two
authored fixtures** and a handful of unit tests — nothing more.

The fixtures are grounded in real published material (the IDSA
specification's own contract-message examples, and the four pinned context
documents above, all fetched 2026-09-06), and the pair of them is a real
test of the property that matters most: two genuinely different DSP context
shapes, one identical result. But that is not the same as running against
a DSP conformance suite, and no claim is made here that it is. Until one
is wired up, treat a surprising ingest as an adapter bug first.
