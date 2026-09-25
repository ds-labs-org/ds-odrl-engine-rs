# What this is not, and the design rationale behind it

This document states the honest scope boundary of `ds-odrl-engine-rs` — what
it deliberately does not implement, and the reasoning (identity-claims
model, deny-override, profile-driven actions, the WASM ABI) behind the
decisions that do ship. See the top-level [README](../README.md) for a
quick-start and a map of the rest of the documentation.

## What this is not

This is **not a full ODRL implementation**. `engine`'s own Default Profile
has ten constraint operators now (`eq`/`neq`/`isAnyOf`/`isAllOf`/
`isNoneOf`/`isPartOf`, plus `lt`/`lteq`/`gt`/`gteq` for ordering
comparison) over a flat string/string-array claims model. `lt`/`lteq`/
`gt`/`gteq` are no longer dateTime-only: each one first tries both sides
as a recognized `xsd:dateTime`/`xsd:date` — the strict UTC `...Z` form,
a `dateTime` with a numeric UTC offset (`+02:00`/`-05:30`, converted to
the equivalent UTC instant), or a bare `xsd:date` (`YYYY-MM-DD`, treated
as midnight UTC of that date for comparison purposes); if neither side
parses that way, tries both sides as an ISO-8601 `xsd:duration`
(`PnYnMnDTnHnMnS`, the type ODRL 2.2 Vocabulary Section 4.5 gives
`odrl:delayPeriod`/`elapsedTime`/`meteredTime`/`timeInterval`) — **not**
XSD's own duration ordering, which is only a partial order because a
calendar `Y`/`M` component has no fixed length; this engine instead
converts `Y` to a fixed 365 days and `M` to a fixed 30 days so every two
durations it parses always compare as a total order, wrong for the small
minority of `Y`/`M`-bearing pairs XSD itself calls indeterminate, right
for the overwhelmingly `D`/`H`/`M`/`S`-shaped durations those four
leftOperand terms are actually used with; and only if none of those
readings apply falls back to comparing both sides as a plain, *finite*
`f64` number, closing Section 7's "no age predicate is expressible" gap
for a numeric claim without adding an operator. Every dispatch still
misses (not errors) when no reading applies to both sides — this now
includes `NaN` and `inf`/`-inf`/`infinity` (every case spelling Rust's
`str::parse::<f64>` itself accepts) on either side, rejected deliberately
rather than left to silently compare: an unrejected `"inf"` would make
`gt`/`gteq` match every finite number and `lt`/`lteq` match none, in
either direction, for a claim or right_operand of exactly that lexical
form (found by an adversarial review before release). See
`engine/src/constraint.rs`'s `ordering_matches` and
`engine/src/temporal.rs`'s `parse_xsd_temporal_nanos` and
`parse_xsd_duration_nanos` for the exact rules. The three
set-based operators `isAllOf`, `isNoneOf`, `isPartOf` all reuse `isAnyOf`'s own
established adaptation of treating `right_operand` as a comma-delimited
list rather than a JSON-LD array (`Constraint::right_operand` is a single
`String`); `isPartOf` in particular is a documented degenerate case, not
general range/hierarchy-membership — it runs the exact same flat
set-membership test as `isAnyOf`, under a different name, because this
engine's opaque string-claims model has no general notion of one value
"containing" another. See `engine/src/constraint.rs`'s `Operator` doc
comment for the exact semantics and honest limitations of each, including
`isNoneOf`'s own deliberate exception to the "absent claim key is a miss"
rule every other operator here follows. Action-taxonomy coverage is still
limited to
*declared* `odrl:includedIn` edges (`engine::ResolvedConfig::covers`): a
permission for a broader action covers a request for a narrower one only
if every hop of that chain is an `ActionDecl` some loaded profile actually
declared — an edge nothing ever declared is never inferred, and an action
never separately declared as its own `ActionDecl` contributes nothing even
as someone else's `includedIn` target. This closes the general
action-implication gap earlier revisions of this README described as
unsupported; what remains is honestly narrower than full RDFS-style
subsumption reasoning. `engine::Constraint` now natively evaluates nested
`odrl:and`/`odrl:or`/`odrl:xone`/`odrl:andSequence` logical groupings,
`odrl:xone` (exactly one child, not "one or more") included —
`odrl:andSequence` reuses `odrl:and`'s own `.all()` evaluation verbatim,
since this engine reads one instantaneous claims snapshot with no
execution-order state to check the Vocabulary's own "in the order
specified" clause against — see "Native logical constraints" below for
the JSON shape and semantics. `odrl:refinement` — a Constraint
narrowing the *Action* itself rather than the Rule ("print, at most 2
copies") — is likewise now evaluated natively, but **only on an Action**:
the Information Model's Party and Asset refinements are not implemented,
since this engine models neither a party nor an asset as anything it can
evaluate against. See "Action refinement" below for the wire shape and
the full scope decision. Each `Rule` now also carries its own optional
`odrl:target` — **which asset that one rule is about** — so one policy can
say "permission on asset A, prohibition on asset B", which this contract
could not express at all while `Request.dataset_id` was its only asset
handle; the match is opaque string equality against that same
`dataset_id`, with no IRI normalization and no collection membership, so
this is narrower than ODRL's own Asset model. See "Per-rule assets
(`odrl:target`)" below. `compliance-runner`'s own adapter
(DNF expansion of `odrl:and`/`odrl:or` into sibling/combined rules ahead
of ever calling `engine`) remains the pattern the vendored compliance
suite is actually translated through today — untouched by this addition
and still how every one of its 68 passing cases gets there; native support
is a new option a host can adopt instead, not a replacement
`compliance-runner` has migrated onto. `odrl:PartyCollection` membership
is still resolved only by `compliance-runner`'s own adapter (SOTW-graph
`odrl:partOf` lookups) — nothing in this addition touches party matching.
`odrl:AssetCollection` membership (`odrl:partOf` on the asset side) is
different: `Request` now carries its own `asset_collections`, a
host-supplied fact naming every collection `dataset_id` is asserted to
belong to, and a rule's `odrl:target` naming a collection IRI matches a
request for any asset in that list, not only the collection IRI itself
(`Rule::target_applies`). This is still opt-in and still host-resolved —
this engine computes no membership itself, no graph, no transitive
closure, no IRI normalization — but it is now a real wire fact `evaluate`
reads, not solely an adapter-side rewrite; see "Per-rule assets
(`odrl:target`)" below for the exact shape and the distinguishing example.
`engine` no longer evaluates policy-level obligations only: a permission's own
`odrl:duty`, a duty's `odrl:consequence` and a prohibition's
`odrl:remedy` are all evaluated natively now — but strictly as
**claims-asserted facts**, the same precondition reading Section 4.5
already gave a policy-level obligation, never as an observation that
anything was performed. This engine is stateless and cannot see execution
state; that boundary is unchanged, and "duty satisfied" here means "the
host supplied claims this duty's own constraints match." A satisfied
`odrl:remedy` deliberately does **not** lift its prohibition. A policy's
`odrl:assignee` is no longer unconditionally inert either — a host that
names `partyIdentityClaim` in its `config` gets policy-level party-role
scoping, where a policy addressed to somebody else is treated as absent
from the request — but that is **opt-in and off by default**, is
`assignee` only (an `odrl:assigner` names who granted a policy, not who is
asking, and is deliberately never evaluated), and resolves no
`odrl:PartyCollection`: see "Party-role evaluation (`odrl:assignee`),
opt-in" below. Two class-specific gaps in that same mechanism are now
closed as well. An `odrl:Agreement`'s own MUST (Vocabulary & Expression
§3.2.1: grant the Policy's terms from the Assigner to the Assignee) now
has a second, dedicated, and fully independent opt-in —
`config.agreementAssigneeClaim`, checked only where `kind == "Agreement"`
and off by default exactly like `partyIdentityClaim` — so a host that has
configured nothing can still enforce this one MUST without switching on
party-role scoping for every other `kind`. And an `odrl:Offer`'s own MUST
NOT (§3.2.2: "MUST not grant any privileges to that Party") is now
enforced unconditionally: an Offer's own `odrl:assignee` is inert to
`partyIdentityClaim` and `agreementAssigneeClaim` alike, in both
directions, regardless of any config — a matching assignee no longer
narrows the grant to that party, and a mismatching one no longer excludes
the policy either. See "Party-role evaluation (`odrl:assignee`), opt-in"
below for all three. Deny-overrides is no longer hardcoded either: a policy
carries its own `odrl:conflict` term (`perm`/`prohibit`/`invalid`), read
only where a permission that grants and a prohibition that denies really do
both hold for the same request. **This is the one change in this engine's
history that alters an existing decision's meaning rather than adding to
it** — a policy declaring nothing used to be resolved prohibition-first and
is now void, ODRL's own stated default — and it was made on the measured
basis that no fixture in the vendored compliance corpus contains a policy
with both a permission and a prohibition, so nothing there moves in either
direction. See "Conflict strategy (`odrl:conflict`)" below. A policy's
`odrl:inheritFrom` is no longer parsed away as an unrecognized key either:
`WirePolicy.inherit_from` names zero or more parent policy `id`s within
this same request's own `policies` list, and each child replicates its
parents' rules and unset `assigner`/`assignee` before party-role scoping
or `decide` ever sees it — closing a real fail-open gap for the single
most natural real-world shape ("child adds nothing, inherits everything"),
which used to fall through to a vacuous `Decision::Allow` under
`behaviour: "open"` regardless of what a parent explicitly prohibited. A
circular chain, or an `inheritFrom` naming an `id` absent from the same
request, is rejected as `Decision::Error` rather than looped or silently
ignored; `odrl:conflict` is deliberately not among what is replicated
(no wire representation for "unset" distinct from its own default), and
this contract has no policy-level Asset or `odrl:profile` field to
replicate in the first place. Not replicating the term does not exempt a
merge from Information Model §2.10's own validation rule 4, though:
when a parent and a child it merged rules into *both explicitly* declared
*differing* `odrl:conflict` values and those rules produce a genuine
collision, the
entire (child) policy is now void — reusing the same `ConflictStrategy::
Invalid` machinery a single policy's own undeclared strategy already
triggers — rather than silently resolved by whichever value happened to
be the child's own. See "Policy inheritance (`odrl:inheritFrom`)" and
"Conflict strategy (`odrl:conflict`)" below. See
"Per-permission duties, consequences and remedies" below for the full
duty semantics and for the reasoning behind that remedy choice, the design
rationale below for what's load-bearing versus what's
compliance-suite-specific, and
[`compliance/reports/latest.md`](../compliance/reports/latest.md) for
exactly which constructs pass, fail, or are skipped today, case by case,
against a real external ODRL test suite.

**Known adapter fragility, not exercised by the vendored corpus** (found
by an independent review of v0.2.0, none of it changes the current
pass/fail result since no vendored fixture triggers them — recorded here
rather than silently left for the next person to rediscover):

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
  corpus — no vendored fixture attaches a duty to a prohibition. **This
  remains true of `translate.rs`, which is untouched, and is now false of
  `engine`**: a prohibition's `odrl:remedy` is a modelled field there and
  a violated one cannot drop the prohibition by construction
  (`engine/src/wire.rs`'s
  `a_violated_remedy_does_not_drop_the_prohibition_and_leaves_a_trace`).
  A host that stops going through this adapter therefore no longer
  inherits the hazard; one that keeps going through it still does, which
  is why the bullet stays.

None of these are hard to fix; they're recorded because a case passing
does not mean they don't exist, and a future contributor extending the
vendored fixtures (or pointing this adapter at a different policy source)
should not have to rediscover them by a wrong verdict.

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

