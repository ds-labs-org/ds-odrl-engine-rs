# Per-permission duties, consequences and remedies

How `odrl:duty` (on a permission), `odrl:remedy` (on a prohibition) and
`odrl:consequence` (on any duty) are evaluated, on top of the
policy-level `odrl:obligation` the [wire contract](wire-contract.md)
already covers. See the top-level [README](../README.md) for a
quick-start and a map of the rest of the documentation.

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
duty state at translate time from the SOTW graph, exactly as before.
`dsp-odrl-adapter`, by contrast, **does** ingest all three now — it is the
one production ingestion path for a real DSP contract offer in this
workspace, so silently dropping a permission's `odrl:duty` there was a
real fail-open, not merely an unexercised construct (see its own README's
"What it ingests" and "What is warned about rather than silently
dropped").

