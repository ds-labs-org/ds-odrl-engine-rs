# Conflict strategy and policy inheritance

How a policy's own `odrl:conflict` term resolves a permission/prohibition
collision, and how `odrl:inheritFrom` merges a child policy with its
parents before either is evaluated. See the top-level
[README](../README.md) for a quick-start and a map of the rest of the
documentation.

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
  [`compliance/reports/latest-cases.json`](../compliance/reports/latest-cases.json)),
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
`odrl:conflict` at all. `dsp-odrl-adapter` **does** ingest one now —
mapping the compacted `odrl:perm`/`odrl:prohibit`/`odrl:invalid` vocabulary
term onto the matching `ConflictStrategy`, and deciding what an
unrecognized term should do, was its own decision rather than a side
effect of the engine gaining the field: an unrecognized term falls back to
`ConflictStrategy::default()` (`invalid`) with a warning naming the term,
rather than an error, since falling back to `invalid` never resolves a
genuine collision any more permissively than `prohibit` already does (see
its own README's "What is warned about rather than silently dropped"). The
demonstrator site's own request types mirror the engine's only as far as
they always did, and still do not model `odrl:conflict` (see
`site/README.md`).

## Policy inheritance (`odrl:inheritFrom`)

Information Model §2.9 lets a (child) Policy declare `odrl:inheritFrom`,
naming one or more (parent) Policies whose Assets, Parties, Actions,
profile identifiers, conflict properties and Rules it inherits — "the
`inheritFrom` property MUST be used" to do it, and "inheritance MUST NOT
be circular." `WirePolicy.inherit_from` (`inheritFrom` on the wire) now
carries this rather than being parsed away as an unrecognized key: zero or
more parent `id`s, looked up **within this same request's own `policies`
list** — this contract has no separate policy store, so a parent has to
be sent alongside its child for either to be resolved at all.

```json
{
  "policies": [
    { "id": "parent", "kind": "Set", "prohibitions": [{ "action": "use", "constraints": [] }] },
    { "id": "child", "kind": "Offer", "inheritFrom": ["parent"], "permissions": [], "prohibitions": [] }
  ]
}
```

Before `decide` or party-role scoping ever sees a policy,
`engine::wire::resolve_inherit_from` replicates each parent's
`permissions`, `prohibitions` and `obligations` into the child (the
child's own rules first, the parent's appended), and its `assigner`/
`assignee` **only where the child leaves them unset** — a child that
names its own `odrl:assignee` is not overridden by a parent's. The merge
has **set semantics over ancestors**: each *distinct* ancestor's own
declared rules are appended exactly once, in depth-first preorder over
the `inheritFrom` lists, so a multi-level chain (grandparent → parent →
child) and a diamond (two parents sharing a common ancestor) both
replicate a grandparent's rules once — not once per path. An earlier
version appended each parent's already-merged rules and did duplicate a
shared grandparent's rules (and its `duties` entries) in a diamond, while
this paragraph claimed otherwise; pinned by `engine/src/wire.rs`'s
`a_diamond_inherit_from_replicates_a_shared_grandparents_rules_exactly_once`.

**Why this closes a real fail-open gap, not a cosmetic one.** The single
most natural real-world shape for `odrl:inheritFrom` is "child adds
nothing, inherits everything" — a child with empty `permissions` and
`prohibitions`, relying entirely on its parent. Before this addition,
`inheritFrom` was silently dropped, that child was evaluated exactly as
declared (genuinely empty), and `Behaviour::Open`'s own documented
vacuous-permission reading (an empty `permissions` list is satisfied)
granted `Decision::Allow` — for a child whose parent explicitly
prohibited the very action being asked for, under this engine's own
default configuration. See `engine/src/wire.rs`'s
`a_child_declaring_no_rules_of_its_own_inherits_its_parents_prohibition_and_denies`
for the worked example (and its own doc comment for why isolating the
parent from independent applicability, via the unrelated, already-existing
party-role mechanism, is what makes the example actually distinguish
"inherited" from "coincidentally denied by the parent's own separate entry
in the same request" — this contract's deny-override-across-the-whole-
policy-set combining rule means a parent sent as an ordinary, unscoped
sibling would decide the request by itself either way).

**Circular inheritance, and a parent absent from the request, are both a
`Decision::Error`, not a `Deny` or a silent truncation.** A chain that
returns to a policy already being resolved, or an `inheritFrom` naming an
`id` that is not any policy in the same request, is a caller
configuration gap — the request's own policy set does not parse into a
tree — exactly the distinction `Decision::Error` already exists to
preserve for an unrecognized action.

**Partial, against §2.9's own MUST list.** `odrl:conflict` is
deliberately **not** replicated: `ConflictStrategy` has no wire
representation for "unset" distinct from its own default (`invalid`), the
same ambiguity `#[serde(default)]` already accepts for a policy that never
declares the key at all, so there is no way to tell "the child left this
to inherit" apart from "the child declared `invalid` itself." This
contract has no policy-level Asset field (only `Rule::target`, per rule,
untouched by inheritance) and no policy-level `odrl:profile` field at all
(`config` is a whole-request setting, not a per-policy one — see the
`other.profile-property` coverage row) — so neither has anything to
replicate in the first place. `kind` is the child's own declared class and
is never touched either; it is not in §2.9's replicated list, and nothing
here selects a semantics from it regardless.

### Differing `odrl:conflict` values across a merge void the policy

Not replicating the term (above) does not exempt a merge from Information
Model §2.10's own validation rule 4: **"Multiple conflict values with
conflicts: the entire Policy MUST be void."** §2.10 states `conflict` of
*both* a single policy's own permission/prohibition clash (the section
above) *and* "conflicts that arise from the merging of Policies" — and
once `odrl:inheritFrom` actually produces a merge, the second reading
applies. `resolve_inherit_from` therefore also tracks, per policy,
**every distinct `odrl:conflict` value *explicitly* declared anywhere in
its own inheritance chain** (itself and every ancestor it merged rules
from, folded transitively across a multi-level chain the same way rules
and party fields already are) — structural information, independent of
any one request. Whether that divergence actually **voids** the policy
still depends on this exact request producing a genuine collision
(`conflicting_rules`), exactly as a single policy's own `odrl:conflict`
already does: a merge that never triggers a real permission/prohibition
clash is unaffected by carrying more than one declared value.

**"Explicitly" is load-bearing.** An ancestor that never wrote
`odrl:conflict` at all — falling back to ODRL's own default, `invalid` —
casts no vote in this check; only ancestors that actually named a
strategy are compared against each other for disagreement, and an unset
ancestor defers to whichever explicit value exists elsewhere in the chain
(the child's own, if the child is the only one that declared one). A
child that declares its own `odrl:conflict: perm` and inherits from a
parent that never declares the term at all is therefore **not** a
divergence — the parent's silent default is not a second, disagreeing
value, and the child's explicit `perm` decides the collision exactly as
if there were no inheritance involved at all (see the canonical worked
example in `ds-odrl-compliance-rdf`'s
`cases/inherited-agreement-duty-chain-01.ttl`, and
`engine/src/wire.rs`'s
`detailed_evaluation_a_childs_explicit_conflict_perm_is_not_overridden_by_a_silently_defaulted_parent`
test). This is the one direction the wire format's own inherent ambiguity
— it has no way to represent "unset" distinct from the default value
itself — can be resolved without inventing a new wire field: a value
equal to the default is always read as "no opinion," never as an
explicit `invalid` that could disagree with a sibling ancestor's explicit
choice. The one case this cannot get right — an ancestor that genuinely,
explicitly writes `odrl:conflict: invalid` disagreeing with another that
explicitly writes `perm` or `prohibit` — is indistinguishable on the wire
from an ancestor that simply never mentioned the term, and is resolved
the same way (deferring, not voiding) until a wire-level presence marker
distinct from the enum value exists to tell the two apart.

```json
{
  "policies": [
    {
      "id": "parent", "kind": "Set", "odrl:conflict": "perm",
      "permissions": [{ "action": "use", "constraints": [] }],
      "prohibitions": [{ "action": "use", "constraints": [] }]
    },
    {
      "id": "child", "kind": "Set", "odrl:conflict": "prohibit",
      "inheritFrom": ["parent"], "permissions": [], "prohibitions": []
    }
  ]
}
```

`child` inherits both of `parent`'s rules and now carries a genuine
collision (a permission and a prohibition both matching the same
requested action). `parent`'s own `perm` and `child`'s own `prohibit`
are two distinct values over that one merged collision, so `child` is
**void** — not resolved by `child`'s own `prohibit` (which would
coincidentally also deny, for the wrong reason) and not by `parent`'s
`perm`:

```
policy 'child' is void: permission[0] and prohibition[0] both matched
requested action 'use', and odrl:inheritFrom merged more than one
distinct odrl:conflict value into this policy (its own declared value is
'prohibit') — Information Model §2.10's validation rule 4 requires the
entire policy be void when a merge carries differing conflict values
over a genuine collision, rather than resolved by any one of them
```

This reuses the exact `ConflictStrategy::Invalid` decision path a single
policy's own undeclared strategy already takes — `engine::wire`
constructs the `decision::Policy` handed to `decide` with `conflict`
forced to `Invalid` whenever the divergence flag is set, so the void
branch `decide` already has is what actually fires — rather than adding a
second conflict-resolution mechanism. The *reason* text is kept distinct
from the single-policy `invalid` wording, though: it names the policy's
own **actually declared** value (`'prohibit'` above, never `'invalid'`
unless that really is what the policy declared or defaulted to), because
`describe_reason` is told about the divergence separately from
`policy.conflict` itself and checks for it first. A child and every
ancestor it merged rules from declaring the *same* `odrl:conflict` value
— including every fixture and coverage probe that predates this
addition, none of which uses `odrl:inheritFrom` alongside a differing
`odrl:conflict` — is entirely unaffected: see `coverage-probes`'
`inheritfrom-conflict-divergence-hit`/`-control` pair (`other.inherit-
from` coverage row) and `engine/src/wire.rs`'s
`an_inherited_and_inheriting_policy_declaring_differing_conflict_values_voids_the_policy`
and `the_same_declared_conflict_value_across_inheritance_is_not_a_divergence`
tests.

`compliance-runner` still never sets this field: no Turtle document in the
vendored 68-fixture suite declares `odrl:inheritFrom`, so there is nothing
for its own adapter to translate. `dsp-odrl-adapter`'s `ingest.rs` **does**
now — every `odrl:inheritFrom` IRI on the ingested policy node is carried,
uncompacted and in document order, into `WirePolicy.inherit_from`, which
`engine::wire::resolve_inherit_from` then resolves the same as any other
populated field, once per `evaluate_request` call (see its own README's
mapping table). Mapping a DSP contract's own `odrl:inheritFrom` node
references was a separate decision from the engine gaining the field,
exactly as `odrl:conflict` ingestion already was — both have since been
closed.

