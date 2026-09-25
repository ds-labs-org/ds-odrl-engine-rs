# Introspection: claims and actions

Two native Rust query capabilities for a host that needs to know something
about a policy set *before* deciding a specific request: which claim keys
it actually reads (`referenced_left_operands`), and which declared actions
a given caller could actually perform (`performable_actions`). See the
top-level [README](../README.md) for a quick-start and a map of the rest
of the documentation.

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
`odrl:xone`/`odrl:andSequence` groupings at any depth — a walk reading only each rule's
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
  `odrl:and`/`odrl:or`/`odrl:xone`/`odrl:andSequence` at once by a fixed
  `xone > or > and > andSequence` precedence, this walk reports the
  **superset** of all of them: gathering a claim that turns out unused
  costs nothing, missing a used one silently changes a decision.

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

