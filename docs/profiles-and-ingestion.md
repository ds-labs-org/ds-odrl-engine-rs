# Profile interpretation and DSP ingestion

The two adapters that turn real-world documents into this engine's own
wire contract: `profile-interpreter`, which reads an ODRL Profile document
into `config`'s declared-action vocabulary, and `dsp-odrl-adapter`, which
reads a real Dataspace Protocol contract offer into a `WirePolicy`. See
the top-level [README](../README.md) for a quick-start and a map of the
rest of the documentation.

## Producing `config` from a real ODRL Profile document

`config` above has to come from somewhere — [`profile-interpreter`](../profile-interpreter/)
reads a real ODRL Profile document (Turtle or JSON-LD) and produces it,
rather than requiring a host to hand-write the JSON:

```sh
cargo run -p profile-interpreter -- interpret my-profile.ttl --duty-mode advise --behaviour open
cargo run -p profile-interpreter -- resolve default-profile.ttl gaia-x-profile.jsonld --duty-mode deny --behaviour closed
```

`interpret` reads one document into its own `engine::Profile` record
(Section 4.4's per-profile shape: `id`, `actions: Vec<ActionDecl>` — each
optionally naming an `odrl:includedIn` parent — `duty_mode`, and
`behaviour`); `resolve` reads several and merges them (union of declared
actions and their `includedIn` edges, strictest `duty_mode`, strictest
`behaviour`) into exactly the wire-shaped `config` object above. See its
own [README](../profile-interpreter/README.md) for precisely what is and
isn't derived from the document — `duty_mode` and `behaviour` are both
never read from it, always caller-supplied flags (`--behaviour` defaults
to `open`).
`profile-interpreter` is also a library (`pub mod graph; pub mod
interpret;`), not just this CLI binary — `site/`'s Demonstrator page
calls it directly to load a pasted profile document in-browser (see
`site/README.md`).

`profile-interpreter/examples/odrl-2.2-common-actions.ttl` is one such
document worth calling out specifically: the W3C ODRL 2.2 Vocabulary's
own full Action taxonomy (both Core Vocabulary roots plus all 49 Common
Vocabulary actions, <https://www.w3.org/TR/odrl-vocab/>), transcribed
`odrl:includedIn` edge by edge from the live spec — not this repo's own
narrow, corpus-driven vocabulary the way `compliance-runner`'s is (see
that crate's `translate.rs`). See `profile-interpreter/README.md` for
what it contains and a spec quirk (a mis-spelled Creative Commons IRI)
it deliberately preserves rather than silently fixing.

## Ingesting a real DSP contract offer (`dsp-odrl-adapter`, opt-in)

`profile-interpreter` above reads the ODRL Profile document — the
*vocabulary* half. [`dsp-odrl-adapter`](../dsp-odrl-adapter/) reads the other
half: the **policy** a real Dataspace Protocol connector actually sends.
A DSP `ContractRequestMessage`/`ContractOfferMessage`/
`ContractAgreementMessage` carries its ODRL as JSON-LD with a real
`@context`, and until this crate existed nothing here turned that document
into a Section 5.2 `WirePolicy` — a host had to hand-translate it, which is
exactly the step where a mistake silently becomes a wrong allow.

```sh
cargo run -p dsp-odrl-adapter --features dsp-ingest -- \
  ingest dsp-odrl-adapter/examples/dsp-2024-1-contract-request.jsonld
cargo run -p dsp-odrl-adapter --features dsp-ingest -- \
  request dsp-odrl-adapter/examples/dsp-2024-1-contract-request.jsonld \
  --dataset-id urn:uuid:3dd1add8-4d2d-569e-d634-8394a8836a88 --action use \
  --claim purpose=odrl:internal-use-only
```

**Opt-in behind the default-off `dsp-ingest` Cargo feature**, and that is a
real gate rather than a label: with the feature off the crate is an empty
library whose `engine`/`serde`/`serde_json` dependencies are all
`optional = true` and switched off with it, and whose CLI binary
(`required-features`) is not built. A compile-time feature rather than a
runtime toggle because ingestion happens strictly *before* the engine is
ever called — it produces the input — so there is no runtime code path a
switch could sit on, and because it gates a JSON-LD parser pointed at
attacker-supplied bytes out of any host that does not speak DSP. A host
opts in with `dsp-odrl-adapter = { path = "…", features = ["dsp-ingest"] }`.
`site/` does not depend on it, on the same boundary that keeps `site/` from
depending on `engine`.

**Real JSON-LD expansion against the document's declared `@context`**, not
a prefix strip. `src/jsonld.rs` implements the slice of JSON-LD 1.1 a DSP
contract policy uses — inline/array/string contexts, compact-IRI and
`@vocab` expansion, `@id`/`@vocab` value coercion, `@value` objects, and
type-scoped `@context` with `@import` and `@propagate` — over a **bundled,
pinned registry of four context documents** (the W3C ODRL 2.2 context, DSP
2024/1, DSP 2025/1 and its ODRL profile), with **no network fetching ever**:
an unbundled `@context` URL is a hard, named error, because ignoring it
would leave every term unexpandable and yield an empty policy, and a policy
that lost its prohibitions is fail-open. It adds **no third-party
dependency at all** beyond `serde`/`serde_json`/`engine`; `oxjsonld` and
the `json-ld` crate were both considered and rejected for stated reasons
(chiefly that an RDF graph loses the array order `permissions[0]` means).
The crate's two example documents are the same policy in the DSP 2024/1
`odrl:`-prefixed shape and the DSP 2025/1 bare-term shape — sharing not one
property-key spelling — and both ingest to one identical `WirePolicy`.

**Scope boundary of this first cut**, stated in full in
[`dsp-odrl-adapter/README.md`](../dsp-odrl-adapter/README.md): it produces a
`WirePolicy` (rules, per-rule and pushed-down policy-level `odrl:target`
and `odrl:action` alike — Information Model §2.7.1's "Compact Policy"
shorthand, the spec's own Example 28 — with a rule's own action still
winning when it names one,
constraints including nested `odrl:and`/`odrl:or`/`odrl:xone`/
`odrl:andSequence`, a permission's `odrl:duty`, a prohibition's
`odrl:remedy` and a duty's `odrl:consequence`, an action's
`odrl:refinement`, a policy's `odrl:inheritFrom` and its `odrl:conflict`
strategy), and nothing else — no negotiation, no signature or credential
verification, no collection-membership resolution, no evaluation.
`@base`/relative-IRI resolution, property-scoped contexts,
`@container`/`@list`, language maps, `@reverse`, `@nest`, `@graph` and RDF
conversion are all unimplemented and named as such. `minimal_config` is a
floor that declares the actions the policy names so `engine` does not
answer `Error` for a vocabulary gap — it declares no `odrl:includedIn`
edges, so real action-taxonomy coverage still comes from
`profile-interpreter` and real Profile documents. A `rightOperand` is
carried byte for byte (including one that itself begins `odrl:`) while
actions, policy classes, `leftOperand`s and `odrl:conflict` are compacted
out of the ODRL namespace, since the first is data and the rest are
vocabulary; `odrl:inheritFrom` is IRI-typed and so, like `target`, never
compacted.

**Not yet corpus-tested against a real DSP conformance suite.** Everything
else in this repo is measured against an external corpus — 68 vendored
ODRL-Test-Suite fixtures, a 52-row ODRL 2.2 coverage catalog — and this
adapter is not. It is checked against its own two authored fixtures
(grounded in the IDSA specification's published contract-message examples
and the four pinned context documents, all fetched 2026-09-06) plus unit
tests, and the compliance suite's 68/68 result is untouched by it: nothing
in `engine`, `compliance-runner` or `compliance/reports/` changed.

