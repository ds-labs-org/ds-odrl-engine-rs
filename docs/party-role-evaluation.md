# Party-role evaluation (`odrl:assignee`), opt-in

How `config.partyIdentityClaim` and `config.agreementAssigneeClaim` scope
a policy's own `odrl:assignee` against the caller's identity claims — both
opt-in, off by default, and independent of each other. See the top-level
[README](../README.md) for a quick-start and a map of the rest of the
documentation.

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

### `kind == "Agreement"`: `agreementAssigneeClaim`, a second and independent opt-in

`partyIdentityClaim` above is opt-in for every `kind` alike, which is
correct only once a host has asked for party-role scoping generally. ODRL
2.2 Vocabulary & Expression §3.2.1 states the Agreement's own MUST
unconditionally: "The Agreement Policy will grant the terms of the Policy
from the Assigner to the Assignee." A host that has configured nothing at
all still leaves that MUST unenforced — `partyIdentityClaim` closes it,
but only as a side effect of turning on scoping for every other `kind`
too, which nothing in the spec asks for.

`config.agreementAssigneeClaim` closes exactly the Agreement gap and
nothing wider. It names its own claims key, entirely independent of
`partyIdentityClaim`'s, and is consulted only for a policy whose `kind` is
exactly `"Agreement"`:

```json
{
  "config": {
    "@type": "odrl:Profile",
    "@id": "https://example.org/profiles/default",
    "odrl:action": [{"@id": "use"}],
    "dutyMode": "advise",
    "behaviour": "closed",
    "agreementAssigneeClaim": "sub"
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
  "claims": { "sub": "did:web:mallory.example" }
}
```

This request denies — mallory is excluded from `agreement-7` exactly as
`partyIdentityClaim` would exclude her, even though `partyIdentityClaim`
itself is unset here.

- **Off unless the key is present**, `#[serde(default)]` and skipped on
  serialization when unset, on the identical footing `partyIdentityClaim`
  is — a config that never names it is byte-for-byte the object it always
  was.
- **`kind`-scoped, unlike `partyIdentityClaim`.** A `Set`, `Ticket`, or any
  other `kind` naming the identical mismatched `assignee` is completely
  unaffected by this setting, at any value — only `kind == "Agreement"` is
  ever read.
- **An Agreement with no `assignee` at all is unaffected either way** —
  structurally invalid per the Agreement's own MUST, but validating
  document structure is out of this engine's scope (the same treatment
  `partyIdentityClaim` already gives a missing assignee).
- **The comparison and the exclusion mechanism are both reused, not
  reinvented**: the identical `ClaimValue::matches` semantics, and the
  identical "a non-matching policy is absent from the request" outcome
  described below.
- **The two switches do not fight each other.** A host may configure
  either, both, or neither. When both are configured and both apply to the
  same mismatched Agreement, the policy is excluded either way — there is
  no merged reason to construct, because either check alone is already
  sufficient.

See `engine/src/wire.rs::party_role_mismatch` for the exact comparison and
`engine/src/profile.rs::ResolvedConfig::agreement_assignee_claim` for the
full rationale.

### `kind == "Offer"`: unconditional assignee inertness

ODRL 2.2 Vocabulary & Expression §3.2.2 is explicit that an Offer's own
assignee carries no privilege in either direction: "the Offer Policy MAY
contain a Party with Assignee function, but MUST not grant any privileges
to that Party." Before this engine enforced that, turning on
`partyIdentityClaim` had it backwards for an Offer: a *matching*
`assignee` wrongly let the policy apply as if the match meant something
(over-granting to a party the spec says gets nothing special), and a
*mismatching* one wrongly excluded the policy under the same "addressed to
someone else" logic that is correct for an Agreement but was never meant
for an Offer.

Both are fixed by making an Offer's own `assignee` **inert,
unconditionally** — regardless of `partyIdentityClaim`, regardless of
`agreementAssigneeClaim`, and regardless of what the assignee's value is
or what the caller presents. A policy with `kind == "Offer"` evaluates
exactly as if it carried no `assignee` at all, in every request, with no
config able to change that. This is checked first, ahead of both claim
comparisons, inside `party_role_mismatch` itself.

### Where the settings live, and where they do not

`partyIdentityClaim` and `agreementAssigneeClaim` are both fields of the
wire `config` and of `engine::ResolvedConfig`
(`with_party_identity_claim`, `with_agreement_assignee_claim`), beside
`dutyMode` and `behaviour`. Unlike those two, neither is a `Profile`
field, and `engine::resolve` can never set either: `dutyMode` and
`behaviour` are statements about how policies should be evaluated, which
is what a profile document is for, whereas both of these are statements
about the shape of the host's own claims map — deployment configuration,
not something a published, shareable ODRL profile can assert about
somebody else's identity provider. `profile-interpreter` therefore never
emits either key.

Only the wire layer reads either one, because only `wire::WirePolicy`
carries a party: `decision::decide` takes a `decision::Policy`, which has
no party and is unaffected by either setting.
`wire::performable_actions_for_request` inherits the scoping (it goes
through `evaluate_request`); `wire::left_operands_for_request`
deliberately does *not* report either configured claim among the keys a
host should gather, since that call answers "which claims do these
policies read" off the policies alone, and the host is by construction the
party that named these keys.

`compliance-runner` leaves both unset and the vendored corpus's 68/68
result is unchanged: that adapter already resolves `odrl:assignee` itself,
per *rule*, against the suite's state-of-the-world graph (`odrl:partOf`
collection membership included) and mirrors it into a `sub` constraint.
Switching the engine's policy-level scoping on there as well would layer a
second, coarser check on top of the one the ground truth is actually
stated in terms of. `dsp-odrl-adapter` leaves both unset too, for the same
reason it cannot guess any other host-identity detail.

