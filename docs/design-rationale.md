# What this is not, and the design rationale behind it

This document states the honest scope boundary of `ds-odrl-engine-rs` — what
it deliberately does not implement, and the reasoning (identity-claims
model, deny-override, profile-driven actions, the WASM ABI) behind the
decisions that do ship.

See the top-level [README](../README.md) for a quick-start and a map of the
rest of the documentation.

## What this is not

This is **not a full ODRL implementation**. The sections below state, topic
by topic, what is and isn't covered.

### Constraint operators

`engine`'s own Default Profile has ten constraint operators:

- `eq`, `neq`
- `isAnyOf`, `isAllOf`, `isNoneOf`, `isPartOf`
- `lt`, `lteq`, `gt`, `gteq`

All ten run over a flat string/string-array claims model — there is no
richer claim type.

**Ordering operators (`lt`/`lteq`/`gt`/`gteq`) are no longer dateTime-only.**
Each one tries, in order, until one side matches both operands:

1. A recognized `xsd:dateTime`/`xsd:date` — the strict UTC `...Z` form, a
   `dateTime` with a numeric UTC offset (`+02:00`/`-05:30`, converted to the
   equivalent UTC instant), or a bare `xsd:date` (`YYYY-MM-DD`, treated as
   midnight UTC).
2. An ISO-8601 `xsd:duration` (`PnYnMnDTnHnMnS`, the type ODRL 2.2
   Vocabulary §4.5 gives `odrl:delayPeriod`/`elapsedTime`/`meteredTime`/
   `timeInterval`) — **not** XSD's own duration ordering, which is only a
   partial order because a calendar `Y`/`M` component has no fixed length.
   This engine instead converts `Y` to a fixed 365 days and `M` to a fixed
   30 days, so every two durations it parses always compare as a total
   order — wrong for the small minority of `Y`/`M`-bearing pairs XSD itself
   calls indeterminate, right for the overwhelmingly `D`/`H`/`M`/`S`-shaped
   durations those four leftOperand terms are actually used with.
3. A plain, *finite* `f64` number — closing Section 7's "no age predicate is
   expressible" gap for a numeric claim without adding a new operator.

Every dispatch still **misses** (never errors) when no reading applies to
both sides. This now also covers `NaN` and `inf`/`-inf`/`infinity` (every
spelling Rust's `str::parse::<f64>` itself accepts) on either side —
rejected deliberately rather than left to silently compare, since an
unrejected `"inf"` would make `gt`/`gteq` match every finite number and
`lt`/`lteq` match none, for a claim or `right_operand` of exactly that
lexical form. (Found by an adversarial review before release.)

See `engine/src/constraint.rs`'s `ordering_matches` and
`engine/src/temporal.rs`'s `parse_xsd_temporal_nanos`/
`parse_xsd_duration_nanos` for the exact rules.

The three set-based operators `isAllOf`, `isNoneOf`, `isPartOf` all reuse
`isAnyOf`'s own adaptation of treating `right_operand` as a
comma-delimited list rather than a JSON-LD array (`Constraint::right_operand`
is a single `String`). `isPartOf` in particular is a **documented degenerate
case**, not general range/hierarchy membership — it runs the exact same flat
set-membership test as `isAnyOf`, under a different name, because this
engine's opaque string-claims model has no general notion of one value
"containing" another.

See `engine/src/constraint.rs`'s `Operator` doc comment for the exact
semantics and honest limitations of each — including `isNoneOf`'s own
deliberate exception to the "absent claim key is a miss" rule every other
operator here follows.

### Action taxonomy

Coverage is still limited to *declared* `odrl:includedIn` edges
(`engine::ResolvedConfig::covers`):

- A permission for a broader action covers a request for a narrower one
  only if **every hop** of that chain is an `ActionDecl` some loaded
  profile actually declared.
- An edge nothing ever declared is never inferred.
- An action never separately declared as its own `ActionDecl` contributes
  nothing, even as someone else's `includedIn` target.

This closes the general action-implication gap earlier revisions of this
document described as unsupported — what remains is honestly narrower than
full RDFS-style subsumption reasoning.

### Logical constraints and action refinement

`engine::Constraint` natively evaluates nested `odrl:and`/`odrl:or`/
`odrl:xone`/`odrl:andSequence` logical groupings — `odrl:xone` means
*exactly one* child, not "one or more." `odrl:andSequence` reuses `odrl:and`'s
own `.all()` evaluation verbatim, since this engine reads one instantaneous
claims snapshot with no execution-order state to check the Vocabulary's own
"in the order specified" clause against. See [Native logical
constraints](wire-contract.md) for the JSON shape and semantics.

`odrl:refinement` — a Constraint narrowing the *Action* itself rather than
the Rule ("print, at most 2 copies") — is likewise evaluated natively, but
**only on an Action**. The Information Model's Party and Asset refinements
are not implemented, since this engine models neither a party nor an asset
as anything it can evaluate against. See [Action refinement](wire-contract.md)
for the wire shape and the full scope decision.

### Per-rule targets and asset collections

Each `Rule` carries its own optional `odrl:target` — **which asset that one
rule is about** — so one policy can say "permission on asset A, prohibition
on asset B," which this contract could not express at all while
`Request.dataset_id` was its only asset handle.

- The match is opaque string equality against that same `dataset_id`.
- No IRI normalization, no collection membership by itself.

See [Per-rule assets (`odrl:target`)](wire-contract.md).

`odrl:AssetCollection` membership (`odrl:partOf` on the asset side) is a
real wire fact today: `Request` carries its own `asset_collections`, a
host-supplied list naming every collection `dataset_id` belongs to, and a
rule's `odrl:target` naming a collection IRI matches a request for any asset
in that list (`Rule::target_applies`). This is still **opt-in and
host-resolved** — this engine computes no membership itself, no graph, no
transitive closure, no IRI normalization.

`odrl:PartyCollection` membership is a **separate, narrower story**: it is
resolved only by `compliance-runner`'s own adapter (SOTW-graph `odrl:partOf`
lookups), not by `engine` itself. Nothing in the asset-collection work above
touches party matching.

### Duties, consequences, remedies

`engine` no longer evaluates policy-level obligations only. A permission's
own `odrl:duty`, a duty's `odrl:consequence`, and a prohibition's
`odrl:remedy` are all evaluated natively — but strictly as **claims-asserted
facts**, the same precondition reading §4.5 already gave a policy-level
obligation, never as an observation that anything was actually performed.

- This engine is stateless and cannot see execution state; that boundary is
  unchanged.
- "Duty satisfied" here means "the host supplied claims this duty's own
  constraints match" — nothing more.
- A satisfied `odrl:remedy` deliberately does **not** lift its prohibition.

See [Per-permission duties, consequences and remedies](duties-consequences-remedies.md)
for the full semantics and the reasoning behind the remedy choice.

### Party-role evaluation

A policy's `odrl:assignee` is no longer unconditionally inert. A host that
names `partyIdentityClaim` in its `config` gets policy-level party-role
scoping, where a policy addressed to somebody else is treated as absent from
the request. But:

- This is **opt-in and off by default**.
- It is `assignee` only — `odrl:assigner` names who granted a policy, not
  who is asking, and is deliberately never evaluated.
- It resolves no `odrl:PartyCollection`.

Two class-specific gaps in this mechanism are closed too:

- **`odrl:Agreement`'s own MUST** (Vocabulary & Expression §3.2.1: grant the
  Policy's terms from the Assigner to the Assignee) now has a second,
  dedicated, fully independent opt-in — `config.agreementAssigneeClaim`,
  checked only where `kind == "Agreement"` and off by default exactly like
  `partyIdentityClaim` — so a host with nothing configured can still enforce
  this one MUST without switching on party-role scoping for every other
  `kind`.
- **`odrl:Offer`'s own MUST NOT** (§3.2.2: "MUST not grant any privileges to
  that Party") is now enforced unconditionally: an Offer's own
  `odrl:assignee` is inert to `partyIdentityClaim` and
  `agreementAssigneeClaim` alike, in both directions, regardless of any
  config.

See [Party-role evaluation (`odrl:assignee`), opt-in](party-role-evaluation.md)
for all three.

### Conflict strategy

Deny-override is no longer hardcoded. A policy carries its own
`odrl:conflict` term (`perm`/`prohibit`/`invalid`), read only where a
permission that grants and a prohibition that denies really do both hold for
the same request.

> **This is the one change in this engine's history that alters an existing
> decision's meaning rather than adding to it.** A policy declaring nothing
> used to be resolved prohibition-first and is now void — ODRL's own stated
> default. This was made on the measured basis that no fixture in the
> vendored compliance corpus contains a policy with both a permission and a
> prohibition, so nothing there moves in either direction.

See [Conflict strategy (`odrl:conflict`)](conflict-and-inheritance.md).

### Policy inheritance

A policy's `odrl:inheritFrom` is no longer parsed away as an unrecognized
key. `WirePolicy.inherit_from` names zero or more parent policy `id`s within
this same request's own `policies` list, and each child replicates its
parents' rules and unset `assigner`/`assignee` before party-role scoping or
`decide` ever sees it.

This closes a real fail-open gap for the single most natural real-world
shape ("child adds nothing, inherits everything"), which used to fall
through to a vacuous `Decision::Allow` under `behaviour: "open"` regardless
of what a parent explicitly prohibited.

- A circular chain, or an `inheritFrom` naming an `id` absent from the same
  request, is rejected as `Decision::Error` — never looped, never silently
  ignored.
- `odrl:conflict` is deliberately **not** replicated (no wire representation
  for "unset" distinct from its own default).
- This contract has no policy-level Asset or `odrl:profile` field to
  replicate in the first place.

Not replicating `odrl:conflict` does not exempt a merge from Information
Model §2.10's own validation rule 4: when a parent and a child it merged
rules into *both explicitly* declare *differing* `odrl:conflict` values, and
those rules produce a genuine collision, the entire child policy is void —
reusing the same `ConflictStrategy::Invalid` machinery a single policy's own
undeclared strategy already triggers, rather than silently resolved by
whichever value happened to be the child's own.

See [Policy inheritance (`odrl:inheritFrom`)](conflict-and-inheritance.md)
and [Conflict strategy](conflict-and-inheritance.md).

### Where to read more

- [Per-permission duties, consequences and remedies](duties-consequences-remedies.md)
  — the full duty semantics and the remedy design rationale.
- [`compliance/reports/latest.md`](../compliance/reports/latest.md) — exactly
  which constructs pass, fail, or are skipped today, case by case, against a
  real external ODRL test suite (68/68 passing as of this writing).

## Known adapter fragility

Three separate, independently-maintained adapters translate a real-world
policy document into this engine's own wire shape. Each has its own scope
and its own known rough edges — recorded here rather than left for a future
contributor to rediscover by a wrong verdict.

### `compliance-runner` (SolidLab ODRL-Test-Suite adapter)

Not exercised by the vendored corpus (found by an independent review of
v0.2.0; none of it changes the current pass/fail result since no vendored
fixture triggers them):

- `translate.rs`'s `is_member_of`/`duty_is_violated` match SOTW-graph nodes
  by **local name only**, not full IRI — a same-named node in a different
  namespace would false-positive a membership or duty-state check. A
  **blank-node** `odrl:duty` is worse: the policy and SOTW files are parsed
  as separate graphs, so a parser-assigned blank-node label can't be relied
  on to correlate between them — a violated duty on a blank-node-identified
  rule could silently stay in play. The vendored corpus only uses
  `urn:uuid:`-identified duties, which sidesteps both.
- `graph.rs`'s `first_literal_for_predicate` (used for "now") returns the
  *first* `dct:issued` triple in file order across the whole SOTW graph — a
  second one in some future SOTW fixture would silently win or lose by
  parse order, not by any stated rule.
- `odrl.rs`'s `parse_rule` reads only the *first* `odrl:constraint` and
  `odrl:duty` triple per rule; ODRL permits more than one of each (an
  implicit AND). A rule with two constraint triples would silently drop one
  rather than combine them.
- An IRI-valued `odrl:rightOperand` (rather than a literal) is read as an
  empty string by `literal_value` — a silent miss, fail-open for a
  prohibition's constraint, fail-closed for a permission's.
- `odrl.rs` never parses a Policy-level `odrl:obligation` at all —
  `WirePolicy.obligations` is always empty from this adapter, independent of
  §4.5's own duty-mode support in `engine`.
- `duty_is_violated`'s exclusion is applied identically to prohibitions as
  to permissions; a violated duty attached to a *prohibition* would drop the
  prohibition (fail-open) rather than the intended asymmetric handling
  ODRL's own `odrl:remedy` construct implies. Untested by this corpus — no
  vendored fixture attaches a duty to a prohibition. **This remains true of
  `translate.rs`, which is untouched, and is now false of `engine`**: a
  prohibition's `odrl:remedy` is a modelled field there, and a violated one
  cannot drop the prohibition by construction
  (`engine/src/wire.rs`'s
  `a_violated_remedy_does_not_drop_the_prohibition_and_leaves_a_trace`). A
  host that stops going through this adapter no longer inherits the hazard;
  one that keeps going through it still does, which is why this bullet
  stays.

### `dsp-odrl-adapter` (real Dataspace-Protocol / EDC ingestion)

Ingests a real DSP contract offer/agreement's ODRL JSON-LD into
`WirePolicy` — see [Profiles and ingestion](profiles-and-ingestion.md) for
the full mapping table. Known, currently-accepted scope limits:

- **Rule-level `odrl:assigneeOf`/`odrl:assignerOf` are internalized at the
  JSON-LD graph level, but go nowhere from there.** `engine::decision::Rule`
  has no per-rule `assignee`/`assigner` field, so an inverse reference
  naming a *rule* is normalized in the graph but never reaches `WirePolicy`.
  The same inverse references *do* reach `WirePolicy.assignee`/`.assigner`
  when they name the *policy* node instead — a real, deliberate, narrower
  scope, not a bug. Closing the rule-level half would require a new field on
  `engine`'s own wire contract (a real ADR-worthy change, not an adapter-side
  fix) and is out of scope here.
- **Inconsistency detection sees only one policy's own directly-declared
  rules.** An inherited prohibition (via `odrl:inheritFrom`) is never
  checked against a permission declared directly on the child, even though
  this adapter does ingest `inheritFrom` and `engine` does resolve it at
  evaluation time. Reading `inconsistencies: []` on an inheriting policy
  does **not** mean "conflict-free across the merged chain" — only "no
  conflict among this policy's own declared rules."
  Paper-faithful (the paper this feature is based on defers inheritance
  resolution to future work too), but worth stating plainly here.
- **Compound-rule expansion (N5) does not deduplicate.** Two identical
  fanned-out atomic rules (e.g. `action: ["read","read"]`) are kept as two
  byte-identical rules rather than collapsed to one. Cosmetic, not a
  correctness issue, but not the "irreducible atomic form" the underlying
  normalization paper describes either.

### `compliance-rdf-runner` (the `dsc:` RDF test-corpus adapter)

A third, separate adapter, translating the `ds-odrl-compliance-rdf`
project's own Turtle test-case corpus into `WirePolicy` — a different
vocabulary (`dsc:`, not raw DSP/EDC JSON-LD) for a different purpose (an
external, engine-verified ground-truth compliance corpus, not real-world
ingestion). See that project's own `docs/vocabulary-spec.md` for its scope
and conventions; nothing about its own known limitations is duplicated here
since it lives in a separate repository with its own documentation.

None of the above are hard to fix. They're recorded because a case passing
does not mean they don't exist, and a future contributor extending a
vendored fixture set (or pointing an adapter at a different policy source)
should not have to rediscover them by a wrong verdict.

## Design rationale

This engine implements the design proposed in the ds42.org dataspace
study's case study, filed at
[`docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md`](https://github.com/Deepthought-Solutions/dataspace/blob/main/docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md)
in the [Deepthought-Solutions/dataspace](https://github.com/Deepthought-Solutions/dataspace)
repository ("Attribute-Based ODRL Policy Enforcement over Eclipse EDC").

That document is the authoritative source for *why* each decision above was
made:

- the identity-claims model,
- the deny-override/permission-requirement enforcement algorithm,
- the profile-driven action mechanism,
- duty semantics,
- the WASM ABI's alternatives analysis,

and a full Limitations and Threats to Validity section this page
summarizes. Read it before extending this engine's scope.
