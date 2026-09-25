# Detailed evaluation (`evaluate_request_detailed`)

A native Rust entry point that returns a full `report:`-vocabulary-shaped
`DetailedEvaluation` alongside the ordinary `Response` — the *why* behind
every rule and every constraint, for a form-based test-case editor or
anything rendering a full compliance report rather than a pass/fail line.
See the top-level [README](../README.md) for a quick-start and a map of
the rest of the documentation.

Every entry point above answers the question `evaluate_request` was built
to answer: `Allow`/`Deny`/`Error`, one `reason` string, and a flat `duties`
list. That is deliberately coarse — Section 5.2's own wire contract keeps
the diagnostic detail out of the response shape a broker actually consumes
on every request. A **form-based test-case editor**, or anything that wants
to render a full `report:`-vocabulary-shaped compliance report
(`ComplianceReportModel.ttl`'s `PermissionReport`/`ProhibitionReport`/
`DutyReport`/`ConstraintReport` family) rather than a pass/fail line, needs
the *why* behind every rule and every constraint, not just the one rule
that happened to decide the request.

```rust
engine::wire::evaluate_request_detailed(&Request) -> (Response, engine::report::DetailedEvaluation)
```

Purely additive: same `Request`, and the exact same `Response` a plain
`evaluate_request(req)` call on the identical input would produce — the two
share every bit of per-policy computation (`build_scaffolding` internally),
so they cannot disagree about which rule decided anything. The wire JSON
contract, `evaluate_request`'s own signature and behavior, and every
existing public type are untouched by this function's existence.

### The shape

```
DetailedEvaluation {
    dataset_id, requested_action,
    policy_reports:    Vec<DetailedPolicyReport>,   // one per applicable policy
    skipped_policies:  Vec<SkippedPolicyReport>,     // party-role mismatches, same as the coarse path
}

DetailedPolicyReport {
    policy_id, policy_request: { requested_action, requested_target },
    rule_reports:          Vec<DetailedRuleReport>,   // Permission | Prohibition | Duty
    evaluation_error:      Option<UnrecognizedAction>, // Some only when decide() short-circuited on this policy
    declared_conflict:     ConflictStrategy,           // this policy's own odrl:conflict, pre-force
    conflict_forced_invalid: bool,                     // true iff odrl:inheritFrom forced it to Invalid
}
```

A `DetailedPermissionReport`/`DetailedProhibitionReport`/`DetailedDutyReport`
each carry `report:activationState`, `report:performanceState` and
`report:deonticState` (a Duty additionally skips `attemptState`, whose
declared domain in the real vocabulary is Permission/Prohibition only —
enforced structurally: the field does not exist on `DetailedDutyReport` at
all), plus a `premise_reports` list — one `Target`/`Action` entry and one
`Constraint` entry per top-level constraint for a Permission or Prohibition;
constraints only (no Action/Target — a duty's own action is what must be
*done*, never matched against the request) for a Duty. Each `Constraint`
premise is itself a small tree (`DetailedConstraintReport`), mirroring
`odrl:and`/`odrl:or`/`odrl:xone`/`odrl:andSequence` nesting node-for-node
rather than only reporting leaves, so a form-based editor can point at
*which* child of an `odrl:xone` group caused it to fail (zero matches and
two-or-more matches are different authoring mistakes, and a leaves-only
report cannot tell them apart). Whether the real `ComplianceReportModel.ttl`
actually nests `ConstraintReport -> ConstraintReport` this way is
unconfirmed by this implementation — the tree is a Rust-side convenience an
RDF-emitting caller may flatten to leaves-only `report:premiseReport`
entries, or keep as a project-local extension; verify against the live
vocabulary before building that side.

A duty reached through `odrl:duty`, `odrl:remedy` or a chained
`odrl:consequence` gets its own independent sibling `DetailedRuleReport::Duty`
entry in the owning policy's flat `rule_reports` — never nested inside the
duty it hangs off, because the real vocabulary defines no
`DutyReport -> DutyReport` link for that. A permission's own `odrl:duty`
list is additionally cross-linked from `report:conditionReport`
(`DetailedPermissionReport::condition_report: Vec<DetailedDutyReport>`) —
one entry per `duty[j]`, in order, alongside its own sibling entry in
`rule_reports`. The reference implementation this vocabulary was designed
around, `SolidLabResearch/ODRL-Evaluator`, types the equivalent field
`conditionReport: NamedNode[]` in
`src/util/report/ComplianceReportTypes.ts` — an array, confirming the
cardinality is per-duty, not a single `RuleReport` as an earlier revision
of this document assumed. `Rule::duty` is itself `Vec<Rule>` (a permission
can carry more than one `odrl:duty`), and `duty_gate_violated` already
checks every sibling duty when deciding whether the permission is gated —
`condition_report` must be able to point at whichever one actually gated
it, not only `duty[0]`. A duty's chain is walked for real up through
`MAX_CONSEQUENCE_DEPTH`; exactly one entry past that bound is still
reported if the data nests that deep, honestly forced `Inactive`/`NonSet`/
`Unknown` because nothing past the bound is ever reached by this evaluator's
own in-force test.

### The one hard rule, and the one disclosed exception to the obvious invariant

**A `DetailedPermissionReport` can never report `activation_state: Active`
while any of its own linked `condition_report` entries reports
`deontic_state: Violated`.**
This is not a convention an implementer has to remember to check both
places for — both fields are derived from the identical
`duty_gate_violated` boolean, computed exactly once in
`decision::derive_detailed_rule_reports`, so they cannot diverge by
construction.

That fix has one real, disclosed consequence for this crate's own
consistency story. The obvious invariant — "the coarse `Response.decision`
is `Allow` iff some permission's detailed report is `Active`" — **holds
under `duty_mode: "deny"`** (the two share the identical duty-resolution
gate) but **does not hold under `duty_mode: "advise"`**: `Advise` mode lets
a permission grant regardless of whether its own gating duty resolved, so
`Response.decision == Allow` can occur while **zero** permissions in that
policy's report are `Active` — every granting permission's own linked duty
is `Violated`. Two independently-true rules ("Advise mode allows regardless
of duty resolution" and "a Permission Report cannot be Active while its
linked Duty Report is Violated") are both real; this divergence is their
unavoidable, honest intersection, not a bug. A caller diffing the coarse and
detailed outputs under Advise mode should expect it.

A second, unrelated case reaches the same "coarse Allow, zero Active
permissions" shape for an entirely different reason: `Behaviour::Open` with
an empty `permissions` list vacuously allows (Section 4.3's original "no
permissions (open default)" case), and there is simply nothing in
`rule_reports` to report as `Active` because there is no permission at all.

### `odrl:conflict`, and what it does and doesn't nullify

`declared_conflict`/`conflict_forced_invalid` on `DetailedPolicyReport`
exist because `describe_reason` already keeps these two facts apart (see
"Policy inheritance" above) and the detailed API preserves the distinction
rather than collapsing it. Every per-rule state in `rule_reports` is
derived using the same *effective* (possibly `odrl:inheritFrom`-forced)
strategy `decide()` itself used for this policy — there is exactly one
`decision::Policy` value in play per policy, read by both the coarse and
detailed paths, never two independently-forced copies.

`odrl:conflict` decides which side the coarse decision follows, and a
prohibition that *loses* that decision is reported as **superseded**, not
as having genuinely fired. A permission `perm` lets win reports
`Active`/`Performed`/`Fulfilled`; the prohibition it beat — even though it
did apply and match — reports `Inactive`/`Unknown`/`NonSet`, exactly as
one that never applied at all, because a superseded prohibition never
actually took effect: nothing came of its having matched. That
supersession propagates down into the prohibition's own `odrl:remedy`
chain too — a remedy of a superseded prohibition never fires, so every
duty in that chain is `Inactive`/`Unknown`/`NonSet` as well, regardless of
whether its own constraints would otherwise have been satisfied. (An
earlier cut of this function reported the losing prohibition, and its
remedy, as genuinely `Active` on the theory that `odrl:conflict` "only
decides which side the coarse decision follows" — this was found to
disagree with `ds-odrl-compliance-rdf`'s own hand-authored ground truth
and has been corrected; see
`engine/src/wire.rs`'s
`detailed_evaluation_reports_a_prohibition_superseded_by_conflict_perm_as_inactive_with_its_remedy_chain_also_inactive`
test.) Conversely, a permission whose own outstanding duty already
excludes it from `Rule::grants` — the same predicate the conflict test
itself uses — was never a party to the collision at all: `decide()` falls
through to the ordinary prohibition-denies branch, and the `reason` trace
never mentions conflict resolution, because there was none to resolve; a
prohibition denying in that ordinary way is not superseded and reports
its own true `Active`/`Performed`/`Violated` state.

### Sharing logic with `evaluate_request`, not duplicating it

The combining rule `decide()` has always applied when a permission and a
prohibition collide is extracted into `decision::resolve_conflict`
(`pub(crate)`), called by both `decide()` and
`decision::derive_detailed_rule_reports` — so the precedence exists in
exactly one place rather than three independent copies (the case this
crate's own history already flags as a real recurring bug source: see
`conflicting_rules`'s own doc comment). Likewise, `wire::evaluate_request`
and `wire::evaluate_request_detailed` both delegate to one private
`build_scaffolding` helper that runs the empty-policies check,
`odrl:inheritFrom` resolution, party-role scoping, and each applicable
policy's own single `decide()` call — `response_from_scaffolding` and
`detailed_from_scaffolding` are two independent, cheap consumers of that
same already-computed scaffold, so an existing `evaluate_request` caller
pays nothing extra for the detailed path's existence, and the two can never
disagree about which rule decided anything.

**Native Rust entry point only — not a JSON wire shape, and not a fifth
WASM export**, on the identical footing the claims/actions entry points
above already state: a `wasm32` guest wanting this would need a new
`extern "C"` export alongside `evaluate` in `engine/src/abi.rs`, left as its
own decision rather than a side effect of adding this function.

