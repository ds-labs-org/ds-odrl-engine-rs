# The wire contract, logical constraints, action refinement and per-rule targets

The JSON request/response shape `engine::wire::evaluate_request` actually
implements, plus the three wire-level extensions built on top of it:
native logical constraints (`odrl:and`/`odrl:or`/`odrl:xone`/
`odrl:andSequence`), action refinement (`odrl:refinement`), and per-rule
assets (`odrl:target`). See the top-level [README](../README.md) for a
quick-start and a map of the rest of the documentation.

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
  consulted at all. `agreementAssigneeClaim` is a fifth, independent
  setting of the same shape — off by default, and checked only against a
  policy whose `kind` is exactly `"Agreement"` — so a host can enforce that
  one class's own MUST without switching on `partyIdentityClaim`'s
  every-`kind` scoping. Neither setting is ever consulted for a policy
  whose `kind` is `"Offer"`: that `kind`'s own `odrl:assignee` is
  unconditionally inert. See "Party-role evaluation (`odrl:assignee`),
  opt-in" below.
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

## Native logical constraints (`odrl:and`/`odrl:or`/`odrl:xone`/`odrl:andSequence`)

`engine::Constraint` — the element type of a `Rule`'s `constraints` list
above — can now, on top of its original flat `left_operand`/`operator`/
`right_operand` shape, itself be a nested `odrl:and`/`odrl:or`/`odrl:xone`/
`odrl:andSequence` grouping of further `Constraint`s (W3C ODRL 2.2's
`odrl:LogicalConstraint`). This is purely additive: `Constraint` keeps its
original three fields at their original JSON keys, and gains four new,
optional fields — `and`/`or`/`xone`/`and_sequence` — each serialized under
its own `odrl:`-namespaced key. A flat constraint (every existing fixture
in this workspace) carries none of them and round-trips exactly as before;
see `engine/src/constraint.rs`'s own doc comment on `Constraint` for the
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
logical object (at least one of `and`/`or`/`xone`/`and_sequence` present)
may omit the atomic fields. See `engine/src/constraint.rs`'s
`a_constraint_object_missing_every_known_field_is_a_parse_error_not_an_inert_false`
test. `odrl:andSequence` was added after the original three, closing a
gap the vocabulary gap analysis and `coverage-probes`' own
`lc-andsequence-ignored` probe both named: before it, `odrl:andSequence`
was simply an unrecognized key — indistinguishable, wire-side, from a
typo — so a constraint carrying it *alongside* a complete atomic
`left_operand`/`operator`/`right_operand` triple silently let the atomic
fields decide instead, with no diagnostic that the sequence was ever seen.
See `an_odrl_andsequence_present_alongside_a_full_atomic_constraint_is_honoured_not_dropped`
and the wire-level
`a_permission_whose_constraint_carries_odrl_and_sequence_alongside_its_atomic_fields_is_honoured_at_the_wire_level`
test for that exact regression case, now fixed.

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

`odrl:and`/`odrl:or`/`odrl:xone`/`odrl:andSequence` each take an array of
nested `Constraint` values (flat or themselves logical, nested arbitrarily)
and combine them, at a fixed `xone` > `or` > `and` > `and_sequence`
precedence for the one case a hand-written object can set more than one of
them at once:

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
- **`odrl:andSequence`** — per the W3C ODRL 2.2 Vocabulary, "satisfied when
  each of the Constraints are satisfied in the order specified." This
  engine evaluates a `Constraint` tree against one instantaneous snapshot
  of `claims` — there is no execution trace of *when* any child became
  satisfied, so "in the order specified" has no separate effect to check
  here, and `and_sequence` reuses `and`'s own `.all()` test verbatim
  (`Constraint::evaluate_bounded`'s `and_sequence` branch, and
  `describe_constraint`'s `reason`-trace rendering, both literally repeat
  the `and` branch's own logic). **This is a narrowing, not silently
  assumed away**:
  a host that genuinely needs to enforce *temporal* ordering between
  constraint satisfactions gets no such enforcement from this engine, only
  ordinary conjunction over one snapshot. See `Constraint`'s own
  `and_sequence` field doc comment, and `coverage-probes`' `logical.and-sequence`
  row (status `Partial`, for exactly this caveat).

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
made by this change. `odrl:andSequence` is still not mapped by
`compliance-runner`'s host-side adapter: no vendored `ODRL-Test-Suite`
fixture uses it, so `to_dnf` has nothing to translate. `dsp-odrl-adapter`'s
own `ingest.rs::constraint_from` — a separate, JSON-LD-shaped mapping from
a real DSP contract offer, not this wire contract — now maps it too,
alongside `xone`/`or`/`and`, onto `Constraint::and_sequence` (see its own
README's "The mapping, term by term"); that closed what was a real
ingestion gap for a real document's `odrl:andSequence` constraint, which
used to fail closed as `ConstraintWithoutLeftOperand` rather than being
silently mistranslated. Engine-level support and adapter-level ingestion
remain separate decisions — `dsp-odrl-adapter` closing its own gap here,
on its own schedule, is that same posture in action, not an exception to
it.

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
  nested `odrl:and`/`odrl:or`/`odrl:xone`/`odrl:andSequence` group ("Native
  logical constraints" above) — the ODRL shape for an action narrowed on several
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
  requested_target, asset_collections)`, and `performable_actions(policy,
  claims, config, requested_target, asset_collections)` — `asset_collections`
  is `requested_target`'s own later, purely additive widening (see below),
  each one argument wider than before it existed. That mirrors how
  `requested_action` itself arrived, and `requested_target` is deliberately
  a required `&str` rather than an `Option`: a caller that does not name the
  asset it is deciding about would silently make every targeted rule
  inapplicable, which for a prohibition is fail-open. `evaluate_request` is
  unchanged in signature and passes `req.dataset_id`/`req.asset_collections`
  itself.
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
- **Matched as an opaque string, and that is the honest limit — but the
  string can now be a collection's, too.** There is still no IRI
  normalization and no relative-reference resolution, but `Request` now
  carries `asset_collections: Vec<String>` alongside `dataset_id`: every
  `odrl:AssetCollection` (ODRL 2.2 Vocabulary §3.4.2) the host asserts
  `dataset_id` is `odrl:partOf` (§3.8.1). `Rule::target_applies` matches
  when a rule's target equals `dataset_id` **or** appears in
  `asset_collections`, so a permission or prohibition scoped to a
  collection IRI now covers a request for an asserted member of it, not
  only a request naming the collection IRI itself. This closes a real
  fail-open gap: before this field existed, a prohibition on
  `urn:asset:collection-X` did nothing for a request naming
  `urn:asset:member-1`, even when a host's own catalog knew perfectly well
  that member-1 is part of collection-X — the exact construct the vendored
  compliance corpus's own testcase-053 through -058 fixtures exercise,
  passing today only because `compliance-runner`'s adapter resolves
  membership before ever calling `evaluate_request`. Membership is still
  entirely host-resolved: this engine computes no graph traversal and no
  transitive closure of its own — a two-level `odrl:partOf` chain
  matches only if the host flattens it into `asset_collections` itself,
  one entry per ancestor — "the same asset" and "an asserted member of
  this collection" both still mean "the same characters". `#[serde(default)]`
  and skipped when empty, so a request naming no collection membership at
  all — every existing fixture — parses and re-serializes byte for byte as
  before (`engine/src/wire.rs`'s
  `a_request_with_no_asset_collections_key_round_trips_unchanged`). See
  `engine/src/decision.rs`'s `Rule::target_applies` for the exact
  semantics, and the coverage catalog's `assets.collections` row (now
  `Partial`, was `NotImplemented`) for what "partial" precisely excludes.
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
is not redundant even now that `engine` can express collection
membership natively.** That adapter still resolves `odrl:target`,
individual and `odrl:AssetCollection` alike, at translate time
(`translate.rs`, `is_member_of`) against the fixture's own
state-of-the-world graph — the resolution step itself, not merely a
wire-contract limitation, since `engine` still performs no such
resolution and never will. Migrating the adapter onto
`Request::asset_collections` (asserting a `sub eq`-style fact instead of
rewriting the rule) is a real option now, but it would rewrite every
exported request in `compliance/reports/latest-cases.json` — the corpus
an independent host re-runs — for zero behavioural change, so it is left
as a separate deliberate decision, exactly as it is for the native
logical constraints above. `compliance-runner` therefore leaves
`Request::asset_collections` empty on every request it builds
(`translate.rs`), and the suite's 68/68 result is unchanged by this
addition, byte for byte.

