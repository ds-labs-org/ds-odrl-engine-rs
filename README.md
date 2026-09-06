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
has ten constraint operators now (`eq`/`neq`/`isAnyOf`/`isAllOf`/
`isNoneOf`/`isPartOf`, plus `lt`/`lteq`/`gt`/`gteq` for ordering
comparison) over a flat string/string-array claims model. `lt`/`lteq`/
`gt`/`gteq` are no longer dateTime-only: each one first tries both sides
as a recognized `xsd:dateTime`/`xsd:date` — the strict UTC `...Z` form,
a `dateTime` with a numeric UTC offset (`+02:00`/`-05:30`, converted to
the equivalent UTC instant), or a bare `xsd:date` (`YYYY-MM-DD`, treated
as midnight UTC of that date for comparison purposes) — and only if
either side isn't one of those falls back to comparing both sides as a
plain, *finite* `f64` number, closing Section 7's "no age predicate is
expressible" gap for a numeric claim without adding an operator. Either
dispatch still misses (not errors) when neither reading applies to both
sides — this now includes `NaN` and `inf`/`-inf`/`infinity` (every case
spelling Rust's `str::parse::<f64>` itself accepts) on either side,
rejected deliberately rather than left to silently compare: an
unrejected `"inf"` would make `gt`/`gteq` match every finite number and
`lt`/`lteq` match none, in either direction, for a claim or
right_operand of exactly that lexical form (found by an adversarial
review before release). See `engine/src/constraint.rs`'s
`ordering_matches` and `engine/src/temporal.rs`'s
`parse_xsd_temporal_nanos` for the exact rules. The three
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
`odrl:and`/`odrl:or`/`odrl:xone` logical groupings, `odrl:xone` (exactly
one child, not "one or more") included — see "Native logical constraints"
below for the JSON shape and semantics. `odrl:refinement` — a Constraint
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
`compliance-runner` has migrated onto. `odrl:PartyCollection`/
`odrl:AssetCollection` membership is still resolved only by
`compliance-runner`'s own adapter (SOTW-graph `odrl:partOf` lookups)
rather than by any change to `engine`'s wire contract — a real host
wanting that would still need the equivalent adapter logic, not just this
engine, and a per-rule `odrl:target` naming a collection IRI matches
only a request for that exact IRI, never a member of it. `engine` no
longer evaluates policy-level obligations only: a permission's own
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
opt-in" below. Deny-overrides is no longer hardcoded either: a policy
carries its own `odrl:conflict` term (`perm`/`prohibit`/`invalid`), read
only where a permission that grants and a prohibition that denies really do
both hold for the same request. **This is the one change in this engine's
history that alters an existing decision's meaning rather than adding to
it** — a policy declaring nothing used to be resolved prohibition-first and
is now void, ODRL's own stated default — and it was made on the measured
basis that no fixture in the vendored compliance corpus contains a policy
with both a permission and a prohibition, so nothing there moves in either
direction. See "Conflict strategy (`odrl:conflict`)" below. See
"Per-permission duties, consequences and remedies" below for the full
duty semantics and for the reasoning behind that remedy choice, the design
rationale below for what's load-bearing versus what's
compliance-suite-specific, and
[`compliance/reports/latest.md`](compliance/reports/latest.md) for
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

## The wire contract

Section 5.2 of the case study specifies the JSON request/response shape;
this is exactly what `engine::wire::evaluate_request` implements today.

Request:

```json
{
  "dataset_id": "urn:uuid:example-dataset-1",
  "action": "use",
  "config": {
    "@type": "odrl:Profile",
    "@id": "https://example.org/profiles/default",
    "odrl:action": [
      {"@id": "use"},
      {"@id": "distribute", "odrl:includedIn": {"@id": "use"}},
      {"@id": "notify"}
    ],
    "dutyMode": "advise",
    "behaviour": "open"
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

- `dataset_id` is the one asset this whole request is *about* — this
  contract's `odrl:target`, not merely a value echoed back in the
  response. A rule carrying its own `odrl:target` (below) is in play only
  when that target is this exact string; a rule carrying none is in play
  whatever this says, which is how every rule behaved before per-rule
  targets existed. There is deliberately no second, separate `target`
  field beside it: two asset handles in one request would be two sources
  of truth with no stated rule for what a host should do when they
  disagree.
- `action` is the one action this whole request is *about* — what the
  caller is actually asking to do. A permission/prohibition rule is only
  in play if it *covers* `action` (`engine::ResolvedConfig::covers`: an
  exact match, or a declared `odrl:includedIn` chain — see `engine/src/
  profile.rs`'s doc comment); a real host no longer pre-filters a policy's
  rules to one action or rewrites `Rule.action` before calling this engine.
- `config` is the host's already-resolved union of every ODRL Profile it
  has loaded, expressed as real ODRL/JSON-LD vocabulary — every declared
  action plus any `odrl:includedIn` parent it names, the strictest loaded
  `dutyMode`, and the strictest loaded `behaviour` — resolved once at host
  startup, travelling in the request so the engine itself stays stateless.
  `dutyMode` (not `odrl:dutyMode`) deliberately stays outside the `odrl:`
  namespace: ODRL defines no property for a profile's own enforcement
  behavior, and namespacing this engine's own invention as if it were real
  ODRL vocabulary would misrepresent it. `behaviour` is different — it
  *is* the ODRL Community Group's own named concept (its Formal Semantics
  draft, Section 3.6: `"open"`/`"closed"`, with `"closed"` also accepting
  the draft's own `"default"` spelling on input) — but it stays outside
  the `odrl:` namespace too, since the draft describes it as an input to
  the evaluation *process*, not a property a Profile document declares
  about itself. `"open"` (the default if omitted) is Section 4.3's own
  original, unconditional choice: an empty `permissions` list is
  vacuously met. `"closed"` requires an actual covering, matching
  permission instead — what a host wanting XACML's `deny-unless-permit`
  posture, or matching an external ODRL evaluator's closed-world ground
  truth (as this engine's own compliance suite does), should choose.
  `partyIdentityClaim` is an optional fourth setting alongside those, and
  is absent from the example above because absent is its default: naming a
  claim key there switches on party-role scoping of each policy's
  `odrl:assignee` against that key. Omitted, no policy's `assignee` is
  consulted at all. See "Party-role evaluation (`odrl:assignee`), opt-in"
  below.
- `policies` mirrors the host's own `Policy`/`Rule`/`Constraint` shape
  field for field — each rule keeps its **own** declared `action`, not
  the request's. `constraints` supports ten operators: `eq`, `neq`, the
  four set-based operators `isAnyOf`/`isAllOf`/`isNoneOf`/`isPartOf`
  (each splitting `right_operand` on commas, with no escaping
  convention), and the four ordering comparisons `lt`/`lteq`/`gt`/`gteq`
  — a UTC `dateTime`, an offset-qualified `dateTime`, or a bare
  `xsd:date` on both sides compared chronologically, falling back to a
  plain numeric comparison when either side isn't one of those (see
  "What this is not" above). `isPartOf` is a documented degenerate case —
  flat set membership identical to `isAnyOf`, not general range/hierarchy
  containment. A rule's `constraints` list matches vacuously when empty.
  A rule may also carry an optional `odrl:refinement` — one `Constraint`
  narrowing its **action** rather than the rule ("print, at most 2
  copies"), checked as part of the action requirement alongside coverage;
  a rule without the key behaves exactly as it did before the key
  existed. See "Action refinement" below. A rule may also carry an
  optional `odrl:target` — the one asset that rule is about, matched
  against the request's own `dataset_id`; a rule without that key applies
  to whatever asset is requested, again exactly as it did before the key
  existed. See "Per-rule assets (`odrl:target`)" below. Three further
  optional keys attach duties to individual rules — `odrl:duty` on a
  permission, `odrl:remedy` on a prohibition, `odrl:consequence` on any
  duty — each holding rules of this very same shape, and each absent from
  every rule this workspace's fixtures build. See "Per-permission duties,
  consequences and remedies" below. A **policy** (not a rule) may also
  carry `odrl:conflict`, one of ODRL's three ConflictTerms — `"perm"`,
  `"prohibit"`, `"invalid"` — governing what it means when one of its own
  permissions and one of its own prohibitions both hold for the same
  request. Absent means `"invalid"`, ODRL's own default; a value outside
  those three fails deserialization rather than being substituted. See
  "Conflict strategy (`odrl:conflict`)" below.
- `claims` is the flat claims map: each value is a JSON string or array
  of strings, sourced from whatever identity the host already trusts —
  this engine never decodes a JWT or other identity-presentation format
  itself.

Response:

```json
{
  "dataset_id": "urn:uuid:example-dataset-1",
  "decision": "Allow",
  "reason": "permission[0] of policy 'policy-1' matched: action 'use': nationality eq DE",
  "duties": [
    { "policy_id": "policy-1", "action": "notify", "resolved": false }
  ]
}
```

- `decision` is one of `"Allow"`, `"Deny"`, or `"Error"` (an `Error`
  means a rule named an action outside every loaded profile's declared
  `odrl:action` list — a configuration gap, not a policy decision — and
  a caller **must** treat it as fail-closed).
- `reason` is a short, human-readable trace of which rule or constraint
  drove the outcome. It is diagnostic text, not a machine-parseable
  contract to branch on.
- `duties` lists any duty this engine could not confirm from the claims it
  was given — a policy-level obligation, a permission's own `odrl:duty`,
  or a prohibition's `odrl:remedy`, each after its `odrl:consequence`
  chain has been followed. It is empty whenever every duty was absent or
  already satisfied. A **policy-level obligation** is additionally
  suppressed under `duty_mode: "deny"`, because its unresolved state is
  exactly what the resulting `"Deny"` already says; the two narrower
  attachments are not suppressed, since neither one's state is carried by
  the decision (an unresolved per-permission duty removes one permission
  from consideration and the request may still be allowed by another; an
  unresolved remedy never drove the decision at all). An entry for
  anything other than a plain policy-level obligation carries an
  additional `source` key naming where it was attached —
  `"permission[0].duty[0]"`, `"prohibition[0].remedy[0]"`, either with one
  `.consequence` segment per hop walked. The key is skipped when absent,
  so an entry for a policy-level obligation is the exact three fields it
  always was.
- Multiple policies in one request combine by **deny-override across the
  whole set** (`Error` > `Deny` > `Allow`), with an empty `policies` array
  treated as a default deny. This combining rule is this implementation's
  own choice, documented in `engine/src/wire.rs` — the case study leaves
  N-policy combining formally undefined (Section 7). A policy's
  `odrl:conflict` term does **not** reach this level: ODRL states it of one
  Policy, about that policy's own permissions and prohibitions, so a
  permission in policy A and a prohibition in policy B are not a conflict
  in its sense and the set-level rule above decides them unchanged.

The wasm32 guest exposes exactly four `extern "C"` exports —
`alloc(len) -> ptr`, `dealloc(ptr, len)`, `evaluate(req_ptr, req_len) ->
packed_ptr_len`, plus the toolchain's default `memory` export — see
`engine/src/abi.rs`. A native host (such as the compliance runner below)
skips the ABI entirely and calls `engine::wire::evaluate_request`
directly.

## Native logical constraints (`odrl:and`/`odrl:or`/`odrl:xone`)

`engine::Constraint` — the element type of a `Rule`'s `constraints` list
above — can now, on top of its original flat `left_operand`/`operator`/
`right_operand` shape, itself be a nested `odrl:and`/`odrl:or`/`odrl:xone`
grouping of further `Constraint`s (W3C ODRL 2.2's `odrl:LogicalConstraint`).
This is purely additive: `Constraint` keeps its original three fields at
their original JSON keys, and gains three new, optional fields —
`and`/`or`/`xone` — each serialized under its own `odrl:`-namespaced key.
A flat constraint (every existing fixture in this workspace) carries none
of them and round-trips exactly as before; see
`engine/src/constraint.rs`'s own doc comment on `Constraint` for the
full design rationale, including the alternatives tried and rejected
before this one.

**`Deserialize` is hand-written, not derived**, specifically to keep this
addition honestly additive rather than accidentally lenient: an object
supplying none of the three atomic fields *and* no logical field (`{}`,
or a typo'd/mis-prefixed key like `"and"` instead of `"odrl:and"`) is
still a hard parse error, exactly as it always was before this type had
any logical fields to be confused with — an earlier version of this
change (caught by an adversarial review before release, not shipped)
let `#[serde(default)]` on the atomic fields silently turn such a
malformed prohibition constraint into an inert, always-`false` atomic
constraint instead, which is a fail-*open* regression for exactly the
rule kind where that direction of mistake matters most. Only a genuinely
logical object (at least one of `and`/`or`/`xone` present) may omit the
atomic fields. See `engine/src/constraint.rs`'s
`a_constraint_object_missing_every_known_field_is_a_parse_error_not_an_inert_false`
test.

A worked example — a permission whose one constraint is an `odrl:and` of
two flat conditions:

```json
{
  "action": "use",
  "constraints": [
    {
      "odrl:and": [
        { "left_operand": "nationality", "operator": "eq", "right_operand": "DE" },
        { "left_operand": "scope", "operator": "isAnyOf", "right_operand": "read,write" }
      ]
    }
  ]
}
```

`odrl:and`/`odrl:or`/`odrl:xone` each take an array of nested `Constraint`
values (flat or themselves logical, nested arbitrarily) and combine them:

- **`odrl:and`** — satisfied when *every* child is satisfied (an empty
  list is vacuously satisfied, same as `Rule`'s own empty `constraints`).
- **`odrl:or`** — satisfied when *at least one* child is satisfied (an
  empty list is never satisfied).
- **`odrl:xone`** — satisfied when **exactly one** child is satisfied: 0
  matching children is not satisfied, and — the part a DNF expansion
  genuinely cannot express — 2-or-more matching children is *also* not
  satisfied. This is the one capability this repo's own "What this is
  not" section above used to name as a flat limitation of the host-side
  `and`/`or` adapter pattern: expanding into an `odrl:or` of pairwise
  `odrl:and` combinations can express "one or more of these", never "this
  one, and not also that other one." `Constraint::evaluate`'s `Xone`
  handling checks the actual count, not a disjunction over combinations.

Evaluation recurses into nested children up to `engine::MAX_CONSTRAINT_DEPTH`
(64) levels deep; a constraint nested past that bound is treated as a
deterministic non-match rather than recursed into further, so a
pathologically deep tree — built directly in Rust, or received as JSON —
cannot grow the evaluator's call stack unboundedly (relevant in
particular to the `wasm32-unknown-unknown` guest, which typically runs
with a smaller stack than a native host). Unlike `ResolvedConfig::covers`'s
`includedIn`-chain walk, which guards against a real graph cycle via a
`visited` set, a `Constraint` tree is owned by value throughout (no
shared or interior-mutable references), so a literal cycle isn't
representable in memory here at all — the bound exists for depth, not
cycle detection. See `engine/src/constraint.rs`'s `MAX_CONSTRAINT_DEPTH`
doc comment and its
`nesting_past_max_constraint_depth_is_a_deterministic_non_match_not_a_panic`
test for the exact boundary.

**This is a new capability the engine now offers a host, not a change to
what any host in this repo actually uses today.** `compliance-runner`'s
own `translate.rs` adapter — which turns the vendored ODRL-Test-Suite's
`odrl:and`/`odrl:or`/`odrl:xone` constraint trees into flat, host-side DNF
before ever building a `Request` (see "What this is not" above,
and `to_dnf` in `compliance-runner/src/translate.rs`) — is completely
untouched by this addition and remains exactly how every one of the
suite's 68 passing cases is translated; it still declines `odrl:xone`
fixtures with a cited, honest reason rather than silently mistranslating
them, since DNF cannot express "exactly one." Migrating that adapter onto
this native support instead is a deliberate, separate later decision, not
made by this change.

## Action refinement (`odrl:refinement`)

The ODRL 2.2 Information Model lets a rule narrow *the action itself*,
not just the circumstances under which the rule applies: its own
canonical example is a permission to **print, at most 2 copies** — a
`Constraint` attached to the Action, distinct from the Rule's own
`odrl:constraint`. `engine::Rule` now carries that as an optional
`action_refinement`, serialized at the wire key `odrl:refinement`.

Until this addition the engine had no representation for it at all, and
did not name it anywhere in these docs. A request that sent one was not
rejected — the key was simply ignored, so a permission for "print, at
most 2 copies" evaluated as a permission for bare `print`, and a
prohibition on "print more than 2 copies" denied *every* print. Both
directions are wrong, and the fail-open one (an ignored refinement on a
permission granting more than the policy author wrote) is why this is a
gap rather than a missing nicety.

```json
{
  "action": "print",
  "constraints": [
    { "left_operand": "sub", "operator": "eq", "right_operand": "alice" }
  ],
  "odrl:refinement": {
    "left_operand": "copies", "operator": "lteq", "right_operand": "2"
  }
}
```

- **The refinement is part of the action requirement, not one more rule
  constraint.** A permission or prohibition applies only if its declared
  `action` covers the request's (exact match or a declared
  `odrl:includedIn` chain — unchanged) **and** its refinement, if it has
  one, is satisfied by the claims. The rule's own `constraints` are then
  a separate condition on top, exactly as before. Both must hold; neither
  substitutes for the other. `engine::decision::Rule::action_applies` is
  the whole action requirement; `covers_action` remains only the bare
  action-string half of it.
- **It reuses `Constraint` verbatim**, so a refinement can itself be a
  nested `odrl:and`/`odrl:or`/`odrl:xone` group ("Native logical
  constraints" above) — the ODRL shape for an action narrowed on several
  axes at once — and `Constraint`'s hand-written, strict `Deserialize`
  applies here too: `"odrl:refinement": {}`, or one missing a
  `right_operand`, is a hard parse error rather than something inert.
  Inert would mean, for a prohibition, that the prohibition applies to
  the *unrefined* action: fail-open again.
- **A duty's refinement is an additional requirement for that duty to
  resolve.** A duty's `action` is what must be *done*, so refining it
  narrows what counts as having done it (`notify`, refined to "by
  email"). Since this engine only ever confirms a duty from claims, a
  refinement it cannot confirm leaves the duty unresolved — the safe
  direction, and one that can never move a duty from unresolved to
  resolved.
- **Its claim keys count.** `referenced_left_operands` (next section)
  reports a refinement's own `left_operand`s alongside the rule
  constraints', nested ones included. A host told to gather less than the
  engine actually reads would leave the refinement unfed, which for a
  prohibition is silently fail-open.
- **It is visible in the `reason` trace**, in both directions: a rule that
  matched prints its refinement (`action 'print' refined by [copies lteq
  2], unconstrained`), and a permission that covered the requested action
  and satisfied all its own constraints but failed *only* on its
  refinement says exactly that (`permission[0] of policy 'p' covers
  requested action 'print' but its action refinement was not satisfied:
  [copies lteq 2]`) instead of the generic "no permission covered and
  matched". That second branch is narrow on purpose: a rule whose own
  constraints also miss is an ordinary non-match and keeps the ordinary
  trace, so a refinement is never credited with a decision it did not
  solely drive.

**Scope: Action only, and deliberately so.** The Information Model also
allows `odrl:refinement` on a **Party** and on an **Asset** (a party
collection narrowed to members in a given role; an asset collection
narrowed to a subset). Neither is implemented, and that is a scope
decision rather than an oversight: `decision::Policy` models no party or
asset at all — `wire::WirePolicy`'s `assigner`/`assignee` are opaque
strings, compared (when party-role scoping is switched on at all) by bare
equality against one claim key and never resolved into a structure with
members, and the dataset is a bare `dataset_id` — so there is no evaluable
node for such a refinement to attach to without first modelling parties
and assets as structures with claims of their own. That is a much larger change than this one, and
naming it here is the point: "supports `odrl:refinement`" without
qualification would overstate what this engine does.

This addition is additive on the wire in the same sense the logical
constraints above are: `odrl:refinement` is `#[serde(default)]` and
skipped on serialization when absent, so a rule that carries none — every
fixture in the vendored compliance corpus, and everything `Rule::new`
builds — parses and re-serializes byte for byte as it did before the
field existed (`engine/src/wire.rs`'s
`an_existing_fixture_rule_without_a_refinement_key_round_trips_unchanged`
asserts exactly that, against a rule copied verbatim out of
`compliance/reports/latest-cases.json`). The vendored corpus exercises no
refinement at all, so the suite's 68/68 result is unchanged by this, and
`compliance-runner`'s `translate.rs` adapter is untouched: it does not
read `odrl:refinement` out of a test-suite policy, so a future fixture
using one would be translated as if unrefined — an honest adapter
limitation of the same kind as the ones listed under "What this is not"
above, recorded here rather than left to be rediscovered by a wrong
verdict.

## Per-rule assets (`odrl:target`)

In the ODRL 2.2 Information Model every Rule carries its **own**
`odrl:target`: the asset that one permission or prohibition is about. This
contract had no representation for that. `Request.dataset_id` was the only
asset handle anywhere in it, so every rule of every policy was implicitly
about that one asset, and the ordinary ODRL policy below — a permission on
one asset and a prohibition on another, in one document — could not be
expressed at all. Sending it anyway got the wrong answer in the direction
that matters: the prohibition applied to *everything*, including the asset
it says nothing about.

`engine::Rule` now carries an optional `target`, serialized at the wire key
`odrl:target`:

```json
{
  "dataset_id": "urn:asset:A",
  "action": "use",
  "config": {
    "@type": "odrl:Profile",
    "@id": "https://example.org/profiles/default",
    "odrl:action": [{"@id": "use"}],
    "dutyMode": "advise"
  },
  "policies": [
    {
      "id": "policy-two-assets",
      "kind": "Set",
      "assigner": "did:web:provider.example",
      "assignee": null,
      "permissions": [
        { "action": "use", "odrl:target": "urn:asset:A", "constraints": [] }
      ],
      "prohibitions": [
        { "action": "use", "odrl:target": "urn:asset:B", "constraints": [] }
      ],
      "obligations": []
    }
  ],
  "claims": {}
}
```

Evaluated as written, that request is an `Allow`, with the reason
`permission[0] of policy 'policy-two-assets' matched: action 'use' on
target 'urn:asset:A', unconstrained`. The identical request with
`dataset_id` changed to `urn:asset:B` is a `Deny`, reasoned
`prohibition[0] of policy 'policy-two-assets' matched: action 'use' on
target 'urn:asset:B', unconstrained` — one policy, two assets, two
opposite answers. Both are `engine/src/wire.rs`'s own
`a_permission_on_one_asset_and_a_prohibition_on_another_are_evaluated_per_rule`
test.

- **`dataset_id` is the request's target.** The asset a rule's
  `odrl:target` is compared against is the `dataset_id` the request already
  carried, not a new field beside it — see the wire-contract section above
  for why one handle rather than two. At the `decision` layer this is an
  explicit parameter: `decide(policy, claims, config, requested_action,
  requested_target)`, and `performable_actions(policy, claims, config,
  requested_target)`, both one argument wider than before. That mirrors how
  `requested_action` itself arrived, and it is deliberately a required
  `&str` rather than an `Option`: a caller that does not name the asset it
  is deciding about would silently make every targeted rule inapplicable,
  which for a prohibition is fail-open. `evaluate_request` is unchanged in
  signature and passes `req.dataset_id` itself.
- **No target means "whatever is being requested", not "no asset".** A rule
  that names none applies to whatever the request is about, which is
  precisely the implicit behaviour every fixture in this workspace already
  relied on. This is what makes the change additive: `odrl:target` is
  `#[serde(default)]` and skipped on serialization when absent, so a rule
  that carries none — every rule in the vendored compliance corpus, and
  everything `Rule::new` builds — parses and re-serializes byte for byte as
  before (`engine/src/wire.rs`'s
  `an_existing_fixture_rule_without_a_target_key_round_trips_unchanged`,
  against a rule copied verbatim out of
  `compliance/reports/latest-cases.json`).
- **Matched as an opaque string, and that is the honest limit.** There is
  no IRI normalization, no relative-reference resolution, and no
  `odrl:partOf`/`odrl:AssetCollection` membership: "the same asset" means
  "the same characters", so a permission targeting a collection IRI does
  not cover a request for a member of that collection. Collection
  membership stays exactly where it already was — resolved by a host
  against its own graph before the request is built (`compliance-runner`'s
  `is_member_of`). Calling this support for `odrl:AssetCollection` would
  overstate it, which is why the coverage catalog records this term as
  `Partial`, not `Implemented`.
- **A target is never an `Error`, unlike an action.** Section 4.4's
  unrecognized-action check exists because a profile declares the action
  vocabulary, so an action outside it is a demonstrable configuration gap.
  Nothing declares an asset vocabulary anywhere here, so a rule naming an
  unheard-of target is indistinguishable from a rule about an asset this
  request simply is not about: an ordinary non-match.
- **A duty's target is carried, not evaluated.** A policy-level duty says
  what must be *done* — and its target is the asset to do it *to* (write
  this audit log, delete that copy), which need not be the asset under
  request. Scoping duties by the requested target would silently drop
  obligations a policy really does attach, so `decide` checks a duty's
  target no more than it checks a duty's action against `requested_action`
  (`engine/src/decision.rs`'s `a_duty_is_not_scoped_by_the_requested_target`).
- **It is visible in the `reason` trace, distinctly from an action
  mismatch.** A rule that matched prints its target (`action 'use' on
  target 'urn:asset:A'`), and a permission that covered the requested
  action and satisfied all its constraints but is about a *different*
  asset says exactly that: `permission[0] of policy 'p' covers requested
  action 'use' but targets 'urn:asset:B', not the requested 'urn:asset:A'`
  — so a denied request says which of the two actually failed rather than
  the generic "no permission covered and matched". As with the refinement
  branch above, this is narrow on purpose: a rule that also misses on its
  own constraints is an ordinary non-match and keeps the ordinary trace.

**`compliance-runner`'s adapter is untouched, and its own target scoping
is not redundant.** That adapter resolves `odrl:target` at translate time
(`translate.rs`, `is_member_of`) and continues to: its two target shapes
are an individual asset *and* an `odrl:AssetCollection`, and the second
needs the fixture's state-of-the-world graph, which the engine never sees.
Moving only the individual case into the engine would leave the
collection case in the adapter regardless, split one rule across two
layers, and rewrite every exported request in
`compliance/reports/latest-cases.json` — the corpus an independent host
re-runs. So the suite's 68/68 result is unchanged by this addition,
byte for byte, and migrating that adapter is a separate deliberate
decision, exactly as it is for the native logical constraints above.

## Per-permission duties, consequences and remedies

Section 4.5 gave this engine one place to hang a duty: `Policy.obligations`,
the whole policy's. ODRL 2.2 has three more, and until this addition
`engine` had no representation for any of them — a request carrying one was
not rejected, the key was simply ignored. `engine::Rule` now carries all
three, as optional fields serialized under their own `odrl:`-namespaced
keys:

| ODRL term | Field | Attached to | Read when that rule is a… |
|---|---|---|---|
| `odrl:duty` | `Rule::duty` (`Vec<Rule>`) | a Permission | permission |
| `odrl:remedy` | `Rule::remedy` (`Vec<Rule>`) | a Prohibition | prohibition |
| `odrl:consequence` | `Rule::consequence` (`Option<Box<Rule>>`) | a Duty | duty, in any of the four positions |

**All three are the same mechanism at a different position, not new
philosophy.** A duty resolves exactly as a policy-level obligation already
did — its own `constraints` all match the claims map, and it has at least
one (`Rule::duty_satisfied`; an unconditional duty stays unresolved, since
there is nothing to check) — and an unresolved one is then governed by the
same `dutyMode` axis. There is deliberately **no second claims-lookup
mechanism**: a host asserts "this duty is fulfilled" by supplying an
ordinary claim the duty's own constraint tests.

```json
{
  "action": "use",
  "constraints": [],
  "odrl:duty": [
    { "action": "compensate",
      "constraints": [
        { "left_operand": "duty:compensate", "operator": "eq", "right_operand": "fulfilled" }
      ],
      "odrl:consequence": {
        "action": "notify",
        "constraints": [
          { "left_operand": "duty:notify", "operator": "eq", "right_operand": "fulfilled" }
        ]
      }
    }
  ]
}
```

That generalizes into the engine what `compliance-runner`'s adapter already
does for one vendored corpus, where the same fact arrives as a
`report:DutyReport`/`report:deonticState` triple in a fixture's
state-of-the-world graph and is resolved at translate time
(`translate.rs`'s `duty_is_violated`). **The stateless boundary is
unchanged and this is not execution-state tracking**: this engine still
cannot observe whether anything was performed, and "satisfied" here means
"the claims the host supplied say so", exactly as it always has for a
policy-level obligation.

### `odrl:duty` on a permission — a pre-condition, scoped to that permission

A permission's duty is a pre-condition of *that one permission*. The
`dutyMode` axis applies to it at that narrower scope:

- **`advise`** — the permission still grants, and the duty is reported in
  the response's `duties` with a `source` of `permission[0].duty[0]`.
- **`deny`** — *that permission* does not grant. A sibling permission with
  nothing outstanding still does, and the request can still be allowed by
  it; that is the whole difference from a policy-level obligation, which
  denies the request outright. `engine/src/decision.rs`'s
  `a_per_permission_duty_gates_only_its_own_permission_under_duty_mode_deny`
  asserts both halves against one policy.

Only permissions actually **in play** contribute a duty — applicable
(`Rule::applies`: action coverage, refinement, target) and matching their
own constraints. A permission this request never reaches imposes nothing,
so reporting its duty would send a host chasing an obligation it does not
have.

### `odrl:consequence` — the duty that applies on non-fulfilment

A duty that is not fulfilled does not immediately fall through to
`dutyMode`. If it carries an `odrl:consequence`, that becomes the duty
actually evaluated; if the consequence resolves, nothing is outstanding at
all. Only when the chain runs out does `dutyMode` govern, exactly as it did
before. The duty reported outstanding is the **last one evaluated** — what
the policy now requires — not the one it replaced, and its `source` says so
with a `.consequence` segment per hop (`duty[0].consequence`).

**The chain is bounded at `engine::decision::MAX_CONSEQUENCE_DEPTH` (4)
hops**, mirroring `MAX_CONSTRAINT_DEPTH`'s intent and deliberately not its
value: a constraint tree really does nest a few levels in real policies,
whereas a consequence chain is a deontic escalation that no policy in the
corpora this workspace tracks — nor in ODRL 2.2's own examples — chains
more than once. Past the bound a duty stays **unresolved**, never resolved:
a tail the evaluator declined to walk must not report itself done.
`a_consequence_chain_is_followed_up_to_max_consequence_depth_and_bounded_past_it`
pins both sides of that boundary.

### `odrl:remedy` — reported, never enforced away

**A satisfied remedy does not lift its prohibition.** A prohibition that
applies and matches denies; its remedy — resolved or not — only ever adds
an entry to `duties` and a clause to the `reason` trace. This is the one
sub-decision inside the decided framing that had a real fork in it, so the
reasoning is stated in full rather than assumed. The rejected reading is
ODRL's own "the remedy substitutes for the violation", which would turn a
would-be `Deny` into an `Allow`-with-a-duty.

1. **Duties in this engine only ever tighten a decision.** Section 4.5's
   duty step can move `Allow` to `Deny` under `dutyMode: "deny"` and can
   never move `Deny` to `Allow`. A remedy that flipped a denial would be
   the first duty here that loosens one — contradicting the very pattern
   this addition extends. The instruction was to make the call *most
   consistent with how `dutyMode`/obligation already work*, and an
   unresolved remedy behaving analogously to an unresolved obligation only
   holds if a *satisfied* one behaves analogously to a satisfied one, which
   is to say: it changes nothing.
2. **"Satisfied" is a host-supplied claim, not an observation.** This
   engine cannot see that a remedy was performed; it sees that the claims
   map says so. Letting one claim erase a prohibition would make the
   engine's most consequential rule the easiest thing in the contract to
   switch off — a strictly worse fail-open than the adapter bug this
   README already records for exactly this construct.
3. **A remedy is consequent on the violation, not a licence for it.** The
   prohibited act still happened; the policy's response is to demand
   something further, which is what an outstanding duty entry and a named
   clause in the trace say.

This directly closes the fail-open hazard named under "Known adapter
fragility" above — "a violated duty attached to a prohibition would drop
the prohibition, fail-open" — at the engine level, and the test asserting
it is deliberately the strong form: a violated remedy must produce `Deny`
**and** leave a trace, not merely avoid an `Allow`.

Only prohibitions that actually **fired** contribute a remedy, mirroring
the permission-duty scoping at the opposite polarity.

### What the `reason` trace says

A decision driven by any of the three names it distinctly from a
policy-level obligation. A rule carrying none of the keys produces exactly
the trace it always did.

| Situation | `reason` |
|---|---|
| permission matched, its duty satisfied | `permission[0] of policy 'p' matched: action 'use', unconstrained; odrl:duty[0] 'compensate' satisfied` |
| permission matched, duty outstanding, `advise` | `…; odrl:duty[0] 'compensate' unresolved (advisory under duty_mode: advise)` |
| permission blocked by its own duty, `deny` | `permission[0] of policy 'p' matched, but its odrl:duty[0] 'compensate' is unresolved under duty_mode: deny` |
| obligation's consequence outstanding, `deny` | `duty[0].consequence 'compensate' of policy 'p' is unresolved under duty_mode: deny` |
| prohibition fired, remedy outstanding | `prohibition[0] of policy 'p' matched: action 'use': nationality eq US; its odrl:remedy[0] 'anonymize' is unresolved and does not lift the prohibition` |
| prohibition fired, remedy satisfied | `…; its odrl:remedy[0] 'anonymize' is satisfied, which does not lift the prohibition` |

The satisfied-remedy row is printed rather than left silent on purpose:
"the prohibition denied and the remedy is done" is precisely the reading a
caller might otherwise expect to have produced an `Allow`.

### The rest of the contract, adjusted consistently

- **Section 4.4's unrecognized-action check covers nested duties too.** A
  policy-level obligation naming an action outside every loaded profile's
  vocabulary was already `Decision::Error`; an identical duty attached to a
  permission is the same configuration gap, and leaving it unchecked would
  make the outcome depend on where the author put the duty. The message
  names the duty's path (`unrecognized action "anonymize" in the duty at
  permission[0].duty[0]: …`); a policy with no nested duty reports exactly
  the message it always did, because the nested walk is a separate pass
  after all three original loops.
- **`referenced_left_operands` reports nested duties' claim keys**, down
  the consequence chain and stopping at the same `MAX_CONSEQUENCE_DEPTH`
  bound evaluation stops at. A host told to gather less than the engine
  reads would leave a duty unfeedable — which under `deny` is a permission
  that can never grant, and for a remedy an obligation the host is never
  told it has.
- **`performable_actions` inherits the gating** rather than re-deciding it,
  as it does for every other semantic: an action reachable only through a
  permission whose duty is outstanding is not performable under `deny`.
- **A duty's own `odrl:target` is still carried, not evaluated**, at all
  four positions, for the reason "Per-rule assets" already gives.

### Additive, and unexercised by the vendored corpus

All three keys are `#[serde(default)]` and skipped on serialization when
empty/absent, so a rule that carries none — every rule in
`compliance/reports/latest-cases.json`, and everything `Rule::new` builds —
parses and re-serializes byte for byte as before
(`an_existing_fixture_rule_gains_no_duty_consequence_or_remedy_key`), and
a request built only from policy-level obligations produces a
byte-identical response, `duties` entries included
(`an_existing_policy_level_obligation_fixture_evaluates_byte_identically`).
The vendored corpus exercises none of the three, so the suite's 68/68
result is unchanged, and `compliance-runner`'s `translate.rs` adapter is
untouched: it does not read `odrl:duty`, `odrl:consequence` or
`odrl:remedy` out of a test-suite policy and keeps resolving per-permission
duty state at translate time from the SOTW graph, exactly as before. The
same is true of `dsp-odrl-adapter`, which still drops a per-rule
`odrl:duty` it finds in a DSP contract offer — with a warning that now says
plainly this is an adapter limitation rather than an engine one.

## Party-role evaluation (`odrl:assignee`), opt-in

A `WirePolicy` has always carried `assigner` and `assignee`, and this
engine has always dropped both before deciding anything: `decision::Policy`
models a policy as its three rule lists and no party at all. A policy
addressed to `did:web:alice.example` therefore granted just as much to
anybody else who presented it, which is not what an ODRL **Agreement**
means.

`config.partyIdentityClaim` closes that, without changing anything for a
host that does not ask for it. It names **which key of `claims` carries the
caller's own identity**:

```json
{
  "config": {
    "@type": "odrl:Profile",
    "@id": "https://example.org/profiles/default",
    "odrl:action": [{"@id": "use"}],
    "dutyMode": "advise",
    "behaviour": "closed",
    "partyIdentityClaim": "sub"
  },
  "policies": [
    {
      "id": "agreement-7",
      "kind": "Agreement",
      "assigner": "did:web:provider.example",
      "assignee": "did:web:alice.example",
      "permissions": [{"action": "use", "constraints": []}],
      "prohibitions": [],
      "obligations": []
    }
  ],
  "claims": { "sub": "did:web:alice.example" }
}
```

- **Off unless the key is present.** No `partyIdentityClaim`, and no
  policy's `assignee` is consulted at all — exactly what this engine did
  unconditionally until now. The field is `#[serde(default)]` and skipped
  on serialization when unset, so a config that never names one is
  byte-for-byte the object it always was, and every fixture in this
  workspace evaluates identically
  (`party_role_evaluation_is_off_by_default_so_a_mismatched_assignee_still_grants`).
- **The claim key is the host's to name, not `sub` by convention.** This
  engine never decodes an identity token, so it has no basis for deciding
  which key an arbitrary host puts a caller's identifier under. `sub` is
  the obvious choice for an OIDC-shaped deployment; nothing here privileges
  it.
- **A policy with no `assignee` is unaffected either way.** There is no
  party role to check, so it behaves exactly as today whether or not the
  capability is configured — which is every policy in the vendored corpus.
- **The comparison is this engine's own `eq` semantics**
  (`ClaimValue::matches`): opaque string equality for a single-valued
  claim, membership for a multi-valued one, so a caller presenting several
  identifiers under one key matches a policy naming any of them. No IRI
  normalization, no `odrl:PartyCollection` membership — "the same party"
  means "the same characters", exactly as `odrl:target` says for assets.
- **A claim key absent from the map is a mismatch**, not a bypass and not
  an error. It is the same direction `Constraint::evaluate` already takes
  for an absent key, and the only safe one: treating "I could not identify
  the caller" as "the caller is whoever the policy names" would make an
  unauthenticated request the cheapest way to collect somebody else's
  agreement.

### A non-matching policy is *absent from the request*

This is the interpretation decision, and it is stronger than "its
permissions do not apply". A policy the caller is not the assignee of
contributes **nothing in either direction**:

- its permissions do not grant;
- its prohibitions do not deny — a policy forbidding alice something says
  nothing whatsoever about bob;
- an unrecognized action inside it is not this caller's configuration gap,
  so it does not produce `Decision::Error`;
- and it does not meet a permission requirement vacuously under
  `behaviour: "open"`. The alternative reading — "a policy stripped of its
  rules" — would have made a policy addressed to someone else *allow* an
  arbitrary caller under the open default, which is the worst answer
  available.

If every policy in a request is addressed to someone else, the effective
policy set is empty, and an empty policy set is a default deny — the same
answer an empty `policies` array already gives.

### What the `reason` trace says

A party-role skip gets its own line, deliberately distinct from a
constraint miss, because the two send a debugging host to entirely
different places:

```text
no policy in the request applies to this caller: policy 'agreement-7' names
odrl:assignee 'did:web:alice.example', which does not match the caller's
'sub' claim ("did:web:mallory.example")
```

and, when the identity claim was not supplied at all,
`... 'sub' claim (absent from the claims map)`. One clause per skipped
policy, joined by `; `. Where some policy *does* apply, the trace is that
policy's own, unchanged: the skipped ones are absent, so there is nothing
for it to report about them.

### Scope: assignee only, deliberately

**`assigner` is not evaluated, and that is not an oversight.** An
`odrl:assigner` identifies who *granted* a policy, not who is requesting,
so comparing it against the caller's identity would be checking the wrong
party. The genuine assigner question — was this party entitled to grant
what it granted — is a trust and provenance question about the policy's
issuance, which a stateless engine handed a JSON document has nothing to
evaluate against. It stays a host concern.

Equally out of scope here: `odrl:PartyCollection` membership (still
resolved only by `compliance-runner`'s own SOTW-graph adapter), the
inverse properties `assignerOf`/`assigneeOf`, ODRL's twelve common party
functions, and Party refinement — all of which remain exactly what the
coverage report already says they are.

### Where the setting lives, and where it does not

`partyIdentityClaim` is a field of the wire `config` and of
`engine::ResolvedConfig` (`with_party_identity_claim`), beside `dutyMode`
and `behaviour`. Unlike those two it is **not** a `Profile` field, and
`engine::resolve` can never set it: `dutyMode` and `behaviour` are
statements about how policies should be evaluated, which is what a profile
document is for, whereas this is a statement about the shape of the host's
own claims map — deployment configuration, not something a published,
shareable ODRL profile can assert about somebody else's identity provider.
`profile-interpreter` therefore never emits the key.

Only the wire layer reads it, because only `wire::WirePolicy` carries a
party: `decision::decide` takes a `decision::Policy`, which has no party
and is unaffected by this setting.
`wire::performable_actions_for_request` inherits the scoping (it goes
through `evaluate_request`); `wire::left_operands_for_request`
deliberately does *not* report the configured identity claim among the
keys a host should gather, since that call answers "which claims do these
policies read" off the policies alone, and the host is by construction the
party that named this key.

`compliance-runner` leaves it unset and the vendored corpus's 68/68 result
is unchanged: that adapter already resolves `odrl:assignee` itself, per
*rule*, against the suite's state-of-the-world graph (`odrl:partOf`
collection membership included) and mirrors it into a `sub` constraint.
Switching the engine's policy-level scoping on there as well would layer a
second, coarser check on top of the one the ground truth is actually
stated in terms of. `dsp-odrl-adapter` leaves it unset too, for the same
reason it cannot guess any other host-identity detail.

## Conflict strategy (`odrl:conflict`)

ODRL 2.2 puts a `conflict` property on the **Policy** (Information Model
§2.10, the `odrl:ConflictTerm` vocabulary): what the policy means when one
of its own permissions and one of its own prohibitions both hold for the
same request. This engine now reads it, and evaluates all three terms.

```json
{
  "id": "urn:uuid:policy-1",
  "kind": "Set",
  "assigner": "did:web:provider.example",
  "assignee": null,
  "odrl:conflict": "perm",
  "permissions": [{ "action": "use", "constraints": [] }],
  "prohibitions": [{ "action": "use", "constraints": [] }],
  "obligations": []
}
```

- `"perm"` — the permission wins. **The one ODRL combining rule this
  engine had no way to express at all** before this key existed.
- `"prohibit"` — the prohibition wins. Deny-overrides, which is exactly
  what this engine did unconditionally before the key existed, now a value
  a policy has to ask for.
- `"invalid"` — the conflicting policy is **void**: neither rule resolves
  the other, so the policy authorizes nothing.

A term outside those three is a **parse failure**, not a silently
substituted default — the same closed-enum posture `Operator`, `dutyMode`
and `behaviour` already have, and the reason a profile-declared strategy
(`ex:assigneeWins`) cannot be selected here rather than being quietly
ignored.

### The default is `invalid`, and that is a real behaviour change

Absent, the key means `"invalid"` — ODRL's own stated default, and **not**
this engine's own prior implicit behaviour, which was an unconditional,
unnamed `prohibit`. This is the only change in this engine's history that
alters what an existing policy shape decides, rather than adding a key that
does nothing unless set, so the reasoning is worth stating plainly:

- It was checked first, not assumed. **Zero of the 68 fixtures in the
  vendored compliance suite contain a policy carrying both a permission and
  a prohibition** (measured against
  [`compliance/reports/latest-cases.json`](compliance/reports/latest-cases.json)),
  so no case in that corpus is affected in either direction, and the
  68/68/0/0 result is byte-identically unchanged. There was nothing here to
  stay compatible *with*.
- That is precisely the position `behaviour` is *not* in. An `Offer` with
  an empty `permissions` list is common real input, so `Behaviour::Open`
  keeps diverging from the Formal Semantics draft's `closed` default for an
  operational reason. No equivalent reason exists here, so this follows the
  spec instead.

### What counts as a conflict, precisely

Only a genuine collision reaches the strategy at all: for one policy, a
prohibition that **applies** (right asset, right action, refinement
satisfied) and matches its own constraints, **and** a permission that
**grants** — all for the identical requested action and requested target.
When at most one of the two holds, the decision is whatever that one rule
already made it, identically under every strategy, which is every policy
shape any fixture in this workspace has.

Two consequences of using `grants` rather than a looser test on the
permission side, both deliberate:

- A permission whose own `odrl:duty` is outstanding under
  `dutyMode: "deny"` is not in force, so it is not a party to a conflict
  either: `perm` cannot promote a permission through a duty gate the
  decision algorithm itself treats as closed.
- An **empty** `permissions` list under `behaviour: "open"` meets the
  permission requirement vacuously, but there is no permission there to win
  anything. A matching prohibition denies under every strategy, `perm`
  included.

### `invalid` is a `Deny` with its own reason, not a fourth decision

A void policy surfaces as `Decision::Deny` / `"Deny"`, under a distinctly
worded `reason`:

```
policy 'urn:uuid:policy-1' is void: permission[0] and prohibition[0] both
matched requested action 'use', and the policy's odrl:conflict strategy is
'invalid' (ODRL's own default), which voids a conflicting policy rather
than resolving it
```

It is deliberately **not** a `Decision::Error`. That outcome exists to say
"this is a *configuration gap* — load a profile that recognizes this
action", which a caller fixes in its own setup; a void policy is not that.
The policy parsed, every action in it was recognized, and the policy itself
says the two rules cannot be reconciled. Adding a fourth `WireDecision`
would also break every existing consumer, where this addition breaks none.

The other two strategies name themselves in the trace as well, so
"prohibition-first because this policy chose it" and "prohibition-first
because nothing contested it" are never the same string:

```
prohibition[0] of policy 'p' matched: action 'use', unconstrained; odrl:conflict
'prohibit' resolves the conflict with permission[0] in the prohibition's favour

permission[0] of policy 'p' matched: action 'use', unconstrained; odrl:conflict
'perm' resolves the conflict with prohibition[0] in the permission's favour
```

Both clauses are appended only for a genuine collision, so every `reason`
this engine produced before this key existed is byte-for-byte what it was.

### Per policy, not per host, and not ingested by every adapter

The term travels with the document that contains the conflicting rules, as
ODRL puts it, rather than being one more knob in `config` that would let a
host silently reinterpret somebody else's policy. A host that controls the
policies it sends sets it on the policies it builds.

`compliance-runner` leaves it at the default and the 68/68 result is
unchanged: no Turtle document in the vendored suite declares
`odrl:conflict` at all. `dsp-odrl-adapter` does **not** ingest one — mapping
an IRI-or-literal `odrl:perm`/`odrl:prohibit`/`odrl:invalid`, and deciding
what an unrecognized term should do, is its own decision rather than a side
effect of the engine gaining the field — so it emits a warning naming the
dropped term rather than substituting a strategy silently. The demonstrator
site's own request types mirror the engine's only as far as they always
did, and still do not model `odrl:conflict` (see `site/README.md`).

## Asking which claims a set of policies actually reads

The claims model above is one-directional: the host pushes a flat map it
assembled from identity it already trusts, and a `left_operand` absent
from that map is a **miss, not an error**. That posture is deliberate and
unchanged — but on its own it left a host with no way to know *which*
claims a given set of policies wants, so it had to push everything it had
or guess. Guessing low is the dangerous direction: an unsupplied claim key
silently turns a prohibition into a non-match, which is fail-*open* for
exactly the rule kind where that direction of mistake matters most.

Three calls now answer that question, at the three levels a caller might
hold the input at:

```rust
// engine crate root re-exports, alongside `decide` and `evaluate_request`:
engine::Constraint::referenced_left_operands(&self)      -> Vec<String>
engine::Policy::referenced_left_operands(&self)          -> Vec<String>
engine::referenced_left_operands(&[Policy])              -> Vec<String>
engine::left_operands_for_request(&Request)              -> Vec<String>
```

Each returns the claim-map keys the input could actually test — **sorted
and deduplicated** (the same stable ordering convention
`profile-interpreter`'s own `declared_left_operands` already set; a set of
claim keys has no meaningful intrinsic order, and a stable one is
diffable and safe to print). The walk covers permissions, prohibitions
and obligations alike, and recurses into nested `odrl:and`/`odrl:or`/
`odrl:xone` groupings at any depth — a walk reading only each rule's
top-level constraints would report *nothing at all* for a policy whose
conditions live inside a logical grouping, which is precisely the richest
constraint shape this engine supports. The `Request` form reads only
`policies`, never `claims` — the caller asking is by construction the one
still deciding what to put in `claims`, so a request built for this call
can carry an empty claims map and get the same answer it would once
populated.

Three specifics worth knowing, each with its own test in
`engine/src/constraint.rs`:

- A **logical node contributes nothing of its own**. `Constraint::and`
  (and a `{"odrl:and": [...]}` object) carries a defaulted `left_operand`
  of `""` that evaluation never reads; reporting it would name an
  empty-string claim key no host can sensibly supply. An *atomic*
  constraint that genuinely names `""` is still reported — that key really
  does get looked up.
- The walk stops at the **same `MAX_CONSTRAINT_DEPTH` bound evaluation
  stops at**. A node nested past it is never evaluated, so naming its
  claim key would send a host to gather a claim that provably cannot
  change any decision.
- Where evaluation resolves an object setting several of
  `odrl:and`/`odrl:or`/`odrl:xone` at once by a fixed `xone > or > and`
  precedence, this walk reports the **superset** of all of them:
  gathering a claim that turns out unused costs nothing, missing a used
  one silently changes a decision.

This is a **reachability** answer, not a requirement. It says which keys
could be consulted, not which must be present for any particular outcome:
a rule the requested action never covers still contributes its operands
(coverage depends on the requested action and the resolved config,
neither of which these calls take), and `isNoneOf` is satisfied precisely
*by* an absent key. A host wanting "which of these am I not carrying?"
diffs the list against its own claims map — that diff is the host's
policy call, not this engine's.

Note also that this is a different list from `profile-interpreter`'s
similarly-named `declared_left_operands`: that one is what a **profile
document** declares as vocabulary (`odrl:LeftOperand`-typed subjects),
this one is what **actual policies** reference. Neither constrains the
other — this engine's `left_operand` is a free-form claims-map key, never
validated against a profile's declared vocabulary.

**Native Rust entry points only — not a JSON wire shape, and not a fifth
WASM export.** The request/response shapes above are untouched, and the
wasm32 guest still exposes exactly the four exports documented earlier. A
`wasm32` guest reaching this would need a new `extern "C"` export
alongside `evaluate` in `engine/src/abi.rs`; that is an additive but real
change to an ABI stated as fixed in three places (that file, this README,
and `site/`'s own bridge, whose export lookup is fatal-on-missing and so
would refuse to load an older `engine.wasm` outright), so it is left as
its own decision rather than made as a side effect here. The consequence
worth stating plainly: `site/`'s Demonstrator page does **not** surface
this today, and cannot without that ABI change — see `site/README.md`.

## Asking which actions a caller could actually perform

Every entry point above answers one yes/no question about one action the
caller already had in mind: `decide(policy, claims, config,
requested_action, requested_target)`, `evaluate_request(req)` for
`req.action` on `req.dataset_id`. A broker
rendering a catalog has the opposite question — not "may this caller `use`
dataset 7", asked once per action per dataset, but "which of the actions my
vocabulary declares could this caller perform at all", so it can filter or
grey out what it shows. Two calls answer that, at the two levels a caller
might hold the input at:

```rust
// engine crate root re-exports, alongside `decide` and `evaluate_request`:
engine::performable_actions(&Policy, &Claims, &ResolvedConfig, &str)  -> Vec<String>
engine::performable_actions_for_request(&Request)                    -> Vec<String>
```

Each returns the subset of `config`'s **declared** actions that come back
`Allow` — sorted and deduplicated, the same stable ordering convention
`referenced_left_operands` above already set. The `&str` on the first is
the asset being asked about (`Request.dataset_id` for the second): since a
rule may scope itself to one asset with `odrl:target`, "what may I do" is
only ever answerable one asset at a time.

Worked example, against the Section 5.2 request documented at the top of
this file — `config` declares `use`, `distribute odrl:includedIn use` and
`notify`; the policy carries one permission for `use` constrained
`nationality eq DE` plus one unconstrained `notify` obligation; the claims
carry `nationality: "DE"`. Both assertions below are
`engine/src/wire.rs`'s own
`the_section_5_2_allow_example_is_performable_for_use_and_the_action_included_in_it`
test, run against that same request:

```rust
let req: engine::Request = serde_json::from_str(SECTION_5_2_EXAMPLE)?;
assert_eq!(engine::evaluate_request(&req).decision, engine::WireDecision::Allow);
assert_eq!(
    engine::performable_actions_for_request(&req),
    vec!["distribute".to_string(), "use".to_string()],
);
```

`distribute` is in the list even though no rule in that policy mentions it:
it is declared `odrl:includedIn use`, so the `use` permission covers it via
`ResolvedConfig::covers`, exactly as it would for a `decide` call naming
`distribute` directly. `notify` is not, because nothing permits it — it is
only ever that policy's obligation. This is precisely why the enumeration
domain is the resolved config's declared actions and **not** the actions
the policy's own rules happen to name: the latter would miss every action
reachable only through a declared `includedIn` edge, which is the coverage
`decide` exists to resolve.

**Both are thin wrappers, not a second decision algorithm.** Every
semantic is inherited from the function each loops over, which is the point:

- `performable_actions` loops `decide` over `ResolvedConfig::declared_actions`;
  `performable_actions_for_request` loops the same evaluation
  `evaluate_request` performs, so a request's **whole policy set** is
  combined by Section 5.2's own deny-override rule (`Error` > `Deny` >
  `Allow`) rather than by unioning per-policy answers. Enumerating per
  policy and unioning would contradict that rule in the one case that
  matters: an action one policy permits and another prohibits would be
  reported as performable. There is exactly one N-policy combining rule in
  this crate and this addition does not add a second.
- **`Decision::Error` yields an empty list.** A rule naming an action
  outside every loaded profile's declared vocabulary makes `decide` answer
  `Error` for *every* action; reporting the remaining ones as performable
  would launder a fail-closed configuration gap into a partial allow-list.
- **`behaviour` is honoured as-is.** Under `"open"` (the default), a policy
  with an empty `permissions` list is vacuously met, so *every* declared
  action comes back. That is the honest answer for that configuration, and
  a caller who finds it surprising wants `"closed"`, which is the parameter
  for exactly that.
- **`dutyMode: "deny"` is honoured as-is**: an unresolved duty denies every
  action, so the list is empty. An action *in* the list may still carry
  unresolved advisory duties — this call reports which actions allow, never
  which duties came with them. A caller proceeding with a specific action
  calls `decide`/`evaluate_request` for it and reads `duties` there.

There is deliberately no `&[Policy]` form at the `decision` layer, unlike
`referenced_left_operands`: unioning claim keys across policies is
well-defined there, but combining *decisions* across them is not — that
rule lives in `wire`, and `performable_actions_for_request` is the
policy-set form.

**Cost**: one full evaluation per declared action, linear in the
vocabulary — ~51 actions for the W3C ODRL 2.2 Common Vocabulary
(`profile-interpreter/examples/odrl-2.2-common-actions.ttl`). Cheap for one
dataset, and worth a host's notice before calling it once per dataset
across a large catalog, since the natural catalog-filtering use is exactly
that loop.

**Native Rust entry points only — not a JSON wire shape, and not a fifth
WASM export**, on the same boundary and for the same reason as
`referenced_left_operands` above: Section 5.2's request/response shapes are
untouched, the wasm32 guest still exposes exactly four exports, and
expanding that ABI is its own decision rather than a side effect here. So
`site/`'s Demonstrator page does not surface this today either — see
`site/README.md`.

`evaluate_request` itself is unchanged by this addition. It now delegates
to a private `evaluate_request_for_action(req, action)` — introduced only
so the enumeration can ask about each declared action without cloning the
request per action, and without a second copy of the combining rule
existing anywhere — and its answer for `req.action`, `reason` string
included, is byte for byte what it was (`engine/src/wire.rs`'s
`evaluate_request_is_byte_for_byte_unchanged_by_the_enumeration_refactor`).

## Producing `config` from a real ODRL Profile document

`config` above has to come from somewhere — [`profile-interpreter`](profile-interpreter/)
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
own [README](profile-interpreter/README.md) for precisely what is and
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
*vocabulary* half. [`dsp-odrl-adapter`](dsp-odrl-adapter/) reads the other
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
[`dsp-odrl-adapter/README.md`](dsp-odrl-adapter/README.md): it produces a
`WirePolicy` (rules, per-rule and pushed-down policy-level `odrl:target`,
constraints including nested `odrl:and`/`odrl:or`/`odrl:xone`, and an
action's `odrl:refinement`), and nothing else — no per-rule `odrl:duty`,
`odrl:consequence` or `odrl:remedy` (all three modelled by `engine` now,
none of them ingested here; a rule carrying a duty is translated without
it and the run says so in a warning), no negotiation, no
signature or credential verification, no collection-membership resolution,
no evaluation. `@base`/relative-IRI resolution, property-scoped contexts,
`@container`/`@list`, language maps, `@reverse`, `@nest`, `@graph` and RDF
conversion are all unimplemented and named as such. `minimal_config` is a
floor that declares the actions the policy names so `engine` does not
answer `Error` for a vocabulary gap — it declares no `odrl:includedIn`
edges, so real action-taxonomy coverage still comes from
`profile-interpreter` and real Profile documents. A `rightOperand` is
carried byte for byte (including one that itself begins `odrl:`) while
actions, policy classes and `leftOperand`s are compacted out of the ODRL
namespace, since the first is data and the rest are vocabulary.

**Not yet corpus-tested against a real DSP conformance suite.** Everything
else in this repo is measured against an external corpus — 68 vendored
ODRL-Test-Suite fixtures, a 52-row ODRL 2.2 coverage catalog — and this
adapter is not. It is checked against its own two authored fixtures
(grounded in the IDSA specification's published contract-message examples
and the four pinned context documents, all fetched 2026-09-06) plus unit
tests, and the compliance suite's 68/68 result is untouched by it: nothing
in `engine`, `compliance-runner` or `compliance/reports/` changed.

## Building

Native build and test:

```sh
cargo build --workspace
cargo test --workspace
```

`dsp-odrl-adapter`'s capability is behind a default-off Cargo feature (see
above), so the plain `--workspace` run above compiles it as an empty
library and runs none of its tests. To exercise them:

```sh
cargo test --workspace --features dsp-odrl-adapter/dsp-ingest
```

Both runs are expected to pass; the second additionally runs that crate's
19 tests.

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
natively (no WASM host needed for this), and (re)writes three artifacts
under `compliance/reports/`.

The first two are this run's own report:
[`latest.md`](compliance/reports/latest.md) and
`latest.json` — pass/fail/skip counts, a table of any failing cases
(expected vs. actual decision and why), and a table of any skipped cases,
each citing a specific, real reason (today: only `odrl:xone`, or a
constraint operator outside `eq`/`neq`/`isAnyOf`/`lt`/`lteq`/`gt`/`gteq` —
see `translate.rs`'s `unsupported_operator`/`xone_unsupported`). A case is
only ever skipped for one of those named, cited reasons — never to avoid
a fail.

The third, `latest-cases.json`, is not a report at all: it is the **test
corpus itself**, exported so an independent host can re-run it. For each
case it carries the exact `engine::wire::Request` this run fed
`engine::evaluate_request`, plus the vendored suite's own expected
decision (`ground_truth::expected_decision`) — and deliberately no tally
and no decision this engine produced, since nothing in that file may
pre-decide the outcome a re-running host is supposed to compute. `site/`'s
Compliance Results page fetches it and re-executes every case against the
compiled `engine.wasm` in the visitor's browser (see below); any other
host willing to speak the Section 5.2 wire contract can do the same. See
`compliance-runner/src/cases.rs`. All three artifacts are written by one
invocation and are meant to be regenerated and committed together.

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
translation convention (each rule keeps its own declared action; the
translated request's top-level `action` carries the fixture's requested
one; `engine::decide`'s own coverage check, not translate-time filtering,
decides whether a rule's action applies — and why its `odrl:target`
scoping stays a translate-time concern even though rules now carry their
own `odrl:target` on the wire: the collection half of it needs the
fixture's state-of-the-world graph, which the engine never sees) and
`compliance-runner/src/ground_truth.rs` for how a single Allow/Deny
verdict is derived from the vendored suite's own `report:*`
compliance-report vocabulary.

## Documentation and demonstrator site

`site/` is a Yew + Trunk single-page app with five pages: a landing page
explaining what this engine is and is not, an in-browser demonstrator
that lets you edit a Section 5.2 request by hand and evaluate it against
a *real* compiled `engine.wasm` (fetched and driven over its raw C ABI —
`alloc`/`dealloc`/`evaluate` — exactly as a JS or JVM host would, with no
Rust-level dependency on the `engine` crate; see `site/README.md` for
why), and a Compliance Results page that **re-runs the entire vendored
compliance corpus live, in the visitor's own browser**, against that same
`engine.wasm` — fetching `compliance/reports/latest-cases.json` (every
case's exact Section 5.2 request plus the suite's expected decision) and
computing its own pass/fail tally over the raw ABI, case by case, with a
four-step progress display. `compliance/reports/latest.json` is still
fetched, now as the *native* run's recorded baseline, and the page
cross-checks its live result against it per case — a real native-vs-wasm
consistency check over one corpus and one engine source. The page is
explicit about the boundary: the Turtle→request translation and the
`report:*` ground truth were computed natively and travel in the
artifact; what runs in the browser is the engine and its ABI. A fourth
page, **ODRL 2.2 Coverage**, does the same thing for this study's
vocabulary claims: it executes all 125 probes of
`compliance/reports/latest-coverage.json` against that same
`engine.wasm`, live, and derives a per-row verdict that can come back
*Contradicted*. The fifth, **Release History**, is the one page here that
is computed at build time rather than in your browser, and says so —
see "Release history dashboard" below for what it shows and why it is
the exception. It
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
document" section for exactly what that configures: action pickers on
every action field (including the top-level requested-action field), an
inline "not among the loaded profile's declared actions" cue, the
profile's own declared `odrl:includedIn` edges flowing straight into the
constructed request's `config.odrl:action` (not just the picker UI), and
free-form `leftOperand` suggestions via `<datalist>` — Section 4.2's
leftOperand stays open-ended by design, so this is a suggestion, not a
restriction.

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
`compliance/reports/`. Live at
<https://ds-labs-org.github.io/ds-odrl-engine-rs/>.

## Release history dashboard

The site's fifth page, **Release History** (`/history`), is a per-release
record of what every tagged version of this engine *actually did* —
measured by re-running this repo's two instruments against each tag, not
by copying numbers out of commit messages.

For each of the 19 tags from `v0.1.0` to `v0.12.1`:

* **ODRL-Test-Suite** — that tag is checked out, and **that tag's own
  `compliance-runner`** is built and run against **the suite revision that
  tag pinned**. The pass rate shown is the one that release genuinely
  recorded, including the releases where it was not 68/68.
* **ODRL 2.2 coverage** — that tag's `engine.wasm` is built for
  `wasm32-unknown-unknown --release`, and **today's** 125-probe catalog
  (`compliance/reports/latest-coverage.json`) is replayed against it
  through its own four-export C ABI (`alloc`/`dealloc`/`evaluate` over
  `memory`) in a [`wasmi`](https://crates.io/crates/wasmi) interpreter.
  A row that comes back *contradicted* names a capability that release
  did not have yet.

This works because the request wire shape has been **additive** since
v0.6.0: every field added after it arrived with `#[serde(default)]`, so an
older engine simply ignores the keys it does not model and answers the
request anyway. Driving the compiled artifact rather than recompiling each
tag's Rust source is what makes it a re-execution of history rather than a
rebuild of history against today's toolchain — and the row's engine
`reason` string comes back with it, so "this release lacked X" is
readable, not inferred.

### Where the additive premise breaks, and what the dashboard does about it

The premise was tested rather than assumed, and it does **not** hold
before v0.6.0. That release reshaped `config` from the bare
`{"recognized_actions": [...]}` object into real JSON-LD
(`@type`/`@id`/`odrl:action`/`odrl:includedIn`) — a rename, not an
addition, and `RequestConfig::recognized_actions` had no
`#[serde(default)]` to fall back on. Every one of today's 125 requests is
therefore refused by a `v0.5.0`-or-earlier engine with

```text
request did not parse as the documented Section 5.2 JSON shape:
missing field `recognized_actions` at line 14 column 9
```

before a single line of policy logic runs. The generator detects this
rather than papering over it: a release whose deserializer refuses
*every* request is recorded with `coverage: null` plus the engine's own
rejection message, and the page renders "not addressable" — never 49
contradictions that would only restate one envelope mismatch, and never a
zero that would read as "this release supported nothing". Its historical
compliance number is still shown, because the wire break stops the
coverage replay, not the suite run that release actually performed.

*Partial* rejection is the opposite case and is kept as real signal: a
release whose `Operator` enum has no `isAllOf` variant refuses exactly the
`isAllOf` probes and answers the other 124 normally. Every release carries
its own `envelope_rejected` count so the two are distinguishable. (The
current engine rejects 4 probes this way and still agrees with all 125 —
four probes *expect* an `Error` decision.)

### The real historical numbers

Rows are verified/contradicted/inconclusive out of the 49 probeable
vocabulary rows (3 of the catalog's 52 rows carry no probe at all);
probes are agreed/disagreed/errored out of 125.

| tag | cut | ODRL-Test-Suite | rows V/C/I | probes A/D/E | what shipped |
|---|---|---|---|---|---|
| `v0.1.0` | 09:56 | 20/68 (48 skipped) | not addressable | — | rewrite README for v0.1.0 release |
| `v0.1.1` | 10:08 | 32/68 (36 skipped) | not addressable | — | recognize `odrl:use` as covering read/write/distribute |
| `v0.1.2` | 10:09 | 32/68 (36 skipped) | not addressable | — | update README compliance numbers |
| `v0.2.0` | 10:34 | 68/68 | not addressable | — | close the remaining 36 compliance skips |
| `v0.2.1` | 10:42 | 68/68 | not addressable | — | reject out-of-range and signed datetime fields |
| `v0.2.2` | 10:45 | 68/68 | not addressable | — | record adapter fragility found by independent review |
| `v0.3.0` | 12:34 | 68/68 | not addressable | — | document the site and the ABI-bridge rationale |
| `v0.4.0` | 14:45 | 68/68 | not addressable | — | `profile-interpreter`: read a real ODRL Profile document |
| `v0.5.0` | 15:17 | 68/68 | not addressable | — | load a real ODRL Profile into the Demonstrator form |
| `v0.6.0` | 16:58 | **66/68** (2 failed) | 30 / 19 / 0 | 84 / 41 / 0 | `odrl:includedIn` action-coverage revision (JSON-LD `config`) |
| `v0.7.0` | 18:06 | 68/68 | 30 / 19 / 0 | 84 / 41 / 0 | the `Behaviour` parameter |
| `v0.8.0` | 19:41 | 68/68 | 39 / 10 / 0 | 105 / 20 / 0 | 4 low-risk gaps: taxonomy, set operators, numeric/date comparison, native logical constraints |
| `v0.8.1` | 19:54 | 68/68 | 40 / 9 / 0 | 106 / 19 / 0 | fix a fail-open regression in v0.8.0's logical-constraint parsing |
| `v0.9.0` | 21:07 | 68/68 | 40 / 9 / 0 | 106 / 19 / 0 | live in-browser compliance runner (site only) |
| `v0.10.0` | 22:32 | 68/68 | 41 / 8 / 0 | 108 / 17 / 0 | live in-browser ODRL 2.2 coverage report (site only) |
| `v0.10.1` | 22:43 | 68/68 | 41 / 8 / 0 | 108 / 17 / 0 | fix direct-hit/reload 404s on Pages sub-routes (site only) |
| `v0.11.0` | 01:00 | 68/68 | 42 / 7 / 0 | 111 / 14 / 0 | 4 additive ODRL gaps + feature-flagged DSP/IDSA ingestion |
| `v0.12.0` | 02:35 | 68/68 | 49 / 0 / 0 | 125 / 0 / 0 | per-permission duty/consequence/remedy, party-role evaluation, real `odrl:conflict` |
| `v0.12.1` | 05:15 | 68/68 | 49 / 0 / 0 | 125 / 0 / 0 | note `dsp-odrl-adapter` on the Coverage page |

Three things in that table are worth reading twice, because none of them
came from a changelog:

* **v0.6.0 really was 66/68.** The `odrl:includedIn` action-coverage
  revision regressed two vendored fixtures, and v0.7.0 fixed them. The
  dashboard found that by re-running the suite at each tag, not by being
  told.
* **v0.8.0 → v0.8.1 shows up as one row flipping back.** Probe
  `lo-count-infinity-rejected` expected `Deny` and v0.8.0 answered
  `Allow` — exactly the `inf`/`infinity` fail-open in the `lt`/`lteq`/
  `gt`/`gteq` numeric fallback that v0.8.1's commit message describes
  fixing. The replay re-detected a historical regression from the binary
  alone.
* **v0.11.0's 7 contradicted rows are precisely v0.12.0's changelog**:
  `party.assigner-assignee`, `duty.per-permission`, `duty.consequence`,
  `duty.remedy`, `conflict.property`, `conflict.fixed-strategy`,
  `conflict.profile-strategies`. `git show v0.11.0:engine/src/wire.rs`
  contains none of `odrl:assignee`, `odrl:conflict`, `odrl:duty`,
  `odrl:remedy` or `odrl:consequence` — those wire keys arrive at v0.12.0,
  so a v0.11.0 engine ignores them (additively) and answers `Allow` where
  the current catalog expects `Deny`. Genuinely absent capability, not a
  harness artefact.

### Why this page is not live in your browser

The Compliance Results and ODRL 2.2 Coverage pages both re-execute their
whole corpus against `engine.wasm` in the visitor's browser, and say so.
This one cannot: its subject is **nineteen different historical
`engine.wasm` binaries**, 3.9 MB of them, which would have to be shipped
and instantiated to recompute 2,375 probe evaluations on page load — for
figures that can only change when someone cuts a new tag. The page states
this in an Alert above the dashboard, and carries each release's
`engine.wasm` SHA-256 so the claim is checkable rather than asserted:
rebuild any tag and compare.

Two things keep the artifact honest anyway. The verdicts are derived by
**`site/src/coverage_catalog.rs` itself** — the very module the live
Coverage page runs in the browser — pulled into the generator with
`#[path]` rather than reimplemented, so the two cannot drift into
disagreeing about what "contradicted" means. And a workspace test asserts
that the newest release's row agrees with the catalog it was generated
from, so a stale regeneration fails `cargo test` instead of quietly
showing an old dashboard. (Confirmed empirically here: the live Coverage
page reports 49 verified / 0 contradicted / 0 inconclusive / 3 documented
in a headless browser, matching this dashboard's `v0.12.1` row exactly.)

### Regenerating it

Two stages, both checked in — this is a repeatable procedure, not a
one-off.

```sh
# Stage 1: build every tag's engine.wasm and run every tag's own
# compliance-runner, in a detached, isolated git worktree.
scripts/build-release-history.sh [STAGE_DIR] [WORKTREE_DIR]

# Stage 2: replay the current probe catalog against each staged
# engine.wasm and write compliance/reports/release-history.json.
cargo run -p release-history --release -- [STAGE_DIR]
```

Both arguments default under `target/release-history/` (git-ignored).
The script never touches your own working tree: it adds its own detached
worktree, checks each tag out there, re-syncs the ODRL-Test-Suite
submodule per tag (the pin is part of the tag), and removes the worktree
when it finishes unless `KEEP_WORKTREE=1`. Re-running is cheap — a tag
already staged is skipped, so an interrupted sweep resumes; `FORCE=1`
rebuilds everything. The whole sweep takes a couple of minutes on a warm
Cargo cache, because a shared `CARGO_TARGET_DIR` means the second tag
onward mostly relinks.

Deliberately **not** part of the ordinary CI loop. `.github/workflows/ci.yml`
regenerates `latest.json`/`latest-cases.json`/`latest-coverage.json` and
fails on `git diff --exit-code compliance/reports/` — those are
per-commit artifacts. `release-history.json` is a *tag-time* artifact:
regenerating it means rebuilding nineteen historical engines, and its
content cannot change on a commit that does not add a tag or change the
probe catalog. Regenerate it by hand when you cut a release, and commit
it with the tag.

**Determinism, measured rather than argued.** `release-history` renders
through `serde_json::to_value` before `to_string_pretty`, for the same
reason `coverage-probes/src/render.rs` documents at length: `Value::Object`
is a `BTreeMap` here, so every object's keys are canonically sorted and no
`HashMap`-backed input can leak iteration-order noise into a committed
file. `--check-determinism` renders the whole artifact and prints only its
SHA-256, so the property can be measured across independent processes
rather than asserted:

```sh
for i in $(seq 8); do
  cargo run -q -p release-history --release -- STAGE_DIR --check-determinism
done
# 8 × sha256 7522ff1f57f3121bc97a7633c50b6ad6acfa5dabfe3b9f36e808f97ce7d67768
# ... identical to sha256sum compliance/reports/release-history.json
```

A second, unplanned reproducibility result fell out of the sweep: tags
whose `engine/` tree is byte-identical produce a **byte-identical**
`engine.wasm`. `v0.1.0`/`v0.1.1`/`v0.1.2`, `v0.2.1`–`v0.5.0`,
`v0.8.1`/`v0.9.0`, `v0.10.0`/`v0.10.1` and `v0.12.0`/`v0.12.1` each share
one SHA-256, and `git diff <a> <b> -- engine` is empty for every one of
those pairs. The engine build is reproducible across checkouts on this
toolchain.

## Current compliance summary

As of the fixtures currently vendored (68 cases), from the native
`compliance-runner` run recorded in `compliance/reports/latest.json`:

| total | passed | failed | skipped |
|---|---|---|---|
| 68 | 68 | 0 | 0 |

These numbers are also reproducible *without* trusting this file or that
one: the site's Compliance Results page re-runs all 68 exported cases
against the compiled `engine.wasm` in your own browser and computes the
same tally there, then cross-checks it against the table above.

The largest fixture in the corpus — `policy-20.ttl`'s "business hours on
every weekday of 2024," an `odrl:or` of 262 `odrl:and`-of-two-`dateTime`-
constraints branches, expanded by `to_dnf` into 262 sibling permission
rules — still passes, evaluated exactly like any other. `read`/`write`/
`distribute` are declared `odrl:includedIn use` (per the W3C ODRL
Vocabulary's own "Included In: use" declarations, `write` confirmed
empirically against this corpus's own ground truth), and `sell`/`give`
`odrl:includedIn transfer`, as real `ActionDecl` data in
`compliance-runner/src/translate.rs`'s `base_action_vocabulary` — resolved
by `engine::ResolvedConfig::covers` itself now, not a host-side special
case. See that module's doc comment for the citation, and for how
`dateTime` constraints, logical `and`/`or` groups, party/asset collection
membership, and per-permission duty state are each resolved (new
`lt`/`lteq`/`gt`/`gteq` operators in `engine`, or SOTW-graph lookups in the
adapter) without weakening the mapping or silently forcing a pass. Per-
permission duty state stays a translate-time, SOTW-graph concern there even
though `engine::Rule` now models `odrl:duty` itself: routing it through the
engine would mean minting a claim per duty node and rewriting every
exported request in `latest-cases.json`, which is a separate deliberate
decision on exactly the footing the logical-constraint and `odrl:target`
migrations already sit on.

**A real regression was found here, and fixed with a real parameter, not
a workaround.** Between the previous compliance-suite baseline (68/68)
and the action-coverage revision above, two fixtures
(`testcase-014-alice-sell`, `testcase-020-bob-sell`) briefly failed: each
fixture's policy has exactly one rule, a prohibition on `use`, and no
permissions at all. Requested against a `sell` action (which `use` does
not cover), that prohibition never applies — leaving the policy's
`permissions` list empty, which `engine::decide`'s own Section 4.3 "empty
permissions is open" departure treats as Allow regardless of an
unrelated, non-covering prohibition being present. This vendored suite's
own closed-world ground truth expects Deny. Earlier revisions of this
adapter never surfaced this: a translate-time action pre-filter used to
discard that sole rule outright whenever its action didn't (loosely)
match the request, which routed the request through Section 5.2's
*different*, closed empty-`policies`-array default instead — masking the
divergence rather than resolving it. **Fixed properly, not patched
around:** the ODRL Community Group's own `Behaviour` axis (Section 3.6:
`open`/`closed`) is now a real, host-configurable `engine` parameter
(`profile.rs`'s `Behaviour`, resolved strictest-wins exactly like
`duty_mode`) rather than a fixed choice baked into `decide`. This
compliance runner sets `Behaviour::Closed` in its base config, matching
what `ground_truth.rs`'s own doc comment already established this suite
assumes — restoring 68/68 for a principled reason, with Section 4.3's
`Open` default fully intact and unchanged for any other host. See
`compliance-runner/src/translate.rs`'s `base_request_config` and
[`compliance/reports/latest.md`](compliance/reports/latest.md) for the
full account.

Nothing in this corpus exercises `odrl:xone` or a numeric/date-time
operator this Default Profile doesn't have — a case that did would still
be honestly skipped, cited, and counted, not silently dropped. See "What
this is not" above for the real, remaining gap between this engine and a
general ODRL implementation, which is wider than these numbers might
suggest.

## Comparing against other ODRL engines

`bench/` holds reproducible harnesses for a five-engine comparison against
the identical 68-fixture corpus above — `SolidLabResearch`'s own reference
evaluator, `DIPS-Tools/odrl-Engine` (OVAL), `SEAMWARE/odrl-pap` (FIWARE's
ODRL→Rego→OPA component), and `Prometheus-X-association/odrl-manager` — each
with its own README stating the exact pinned commit/version and the exact
setup a reproducer runs first, since none of them is vendored here. Full
comparative analysis, including how fair (and unfair) that comparison
actually is, is published as
[a benchmark report](https://github.com/Deepthought-Solutions/dataspace/blob/main/docs/benchmarks/2026-09-06-odrl-engine-comparative-coverage.md)
in the sibling `dataspace` repository. This covers conformance/coverage
reproducibility only — performance, resource, and load-behavior comparison
remain a separate, not-yet-attempted follow-up.

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
request contract, which has a requested `action` and a single asset
handle per request, no RDF, and none of the ODRL constructs listed under
"What this is not" above. A fixture that cannot be represented in that contract
is skipped, cited by name, rather than silently passed or force-fitted.
Upstream license terms apply to the vendored submodule content; they do
not extend to this repository's own code.

## License

Licensed under the Apache License, Version 2.0 — see [LICENSE](LICENSE).
