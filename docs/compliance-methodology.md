# Compliance methodology

The methodology behind the site's Full ODRL 2.2 Compliance page (judging
live behaviour against the spec ideal, not just this study's own
documentation), the current compliance-suite summary, and attribution for
the vendored ODRL-Test-Suite fixtures the whole compliance corpus is built
from. See the top-level [README](../README.md) for a quick-start and a
map of the rest of the documentation.

## Full ODRL 2.2 compliance page

The site's fifth page, **ODRL 2.2 Full Compliance** (`/full-compliance`),
asks a question the Capability Audit page deliberately does not.

* **`/coverage`** asks: *does the engine match what this study documents
  about it?* Its target is each probe's `expect`, which records what this
  engine **does** — honestly-documented gaps included. A red row there
  means the **documentation** is wrong, which is why several `expect`s
  record a narrowing as the correct outcome.
* **`/full-compliance`** asks: *assuming the engine **should** fully
  implement ODRL 2.2 for every row that is not structurally out of scope,
  does its real, live behaviour meet that ideal?* Its target is each
  probe's `ideal` (added to the catalog in schema
  `ds-odrl-engine-rs/odrl-coverage@2`), which records what the **spec**
  requires. A row falling short there means the engine is right about
  itself and still short of ODRL 2.2.

Both pages drive the **same** compiled `engine.wasm` over the same
`alloc`/`evaluate`/`dealloc` C ABI with the same 136 request payloads,
through the same replay function (`coverage_run::replay_all`, shared
rather than copied), so any difference between them is a difference of
question and never of execution.

### Its own vocabulary, and no red

`/coverage`'s *Agreed*, *Disagreed*, *Verified* and *Contradicted* already
mean matches-the-documentation, so none of the four appears on this page.
The words here are **Meets full spec**, **Falls short**, and — for the one
row no request can pose at all — **Falls short (structural)**, plus
**Undetermined** for a probe this browser could not judge. Nothing on the
page is red either: amber marks a demonstrated shortfall and purple a
structural one, because falling short of the full spec is what a
*partial* status *means*, not a failure. Red stays reserved for
`/coverage`, where it means something louder and rarer.

`site/src/full_compliance.rs` carries a unit test
(`the_two_axes_use_none_of_the_coverage_pages_four_words`) asserting the
separation rather than trusting it to survive editing.

### The judgment is the presence of an ideal, not a comparison

A probe carrying an `Ideal` falls short; one carrying `None` meets the
spec. That is not derivable by comparing `ideal.decision` against
`expect.decision`, and the catalog contains the probe that proves it:
`duty-consequence-itself-unresolved`'s ideal decision **equals** its
current one (both `Allow`) and what falls short there is the reported
`duties` list — a compliant engine must report *both* outstanding duties,
and this engine reports only the consequence, silently dropping the
primary the spec says is still required. A decision comparison would score
that probe fully compliant. Pinned by
`a_probe_whose_ideal_decision_equals_its_current_one_still_falls_short`.

### Scope, and the real numbers

Of the catalog's 52 vocabulary rows:

* **7 documented `OutOfScope` are excluded outright.** They name
  profile-extension points and `odrl:hasPolicy`, which sit outside the
  wire contract entirely — and being outside the contract is not the same
  thing as falling short of the spec.
* **11 documented `Implemented` meet full spec trivially.** No probe on
  any of them carries an ideal, asserted over the real catalog rather than
  assumed (it is load-bearing: `act-base-exact` and
  `pf-assignee-null-control` are each shared between an `Implemented` row
  and an implementable one).
* **34 documented `Partial` or `NotImplemented` are the researched ones**,
  and **not all of them fall short**: a row is partial for one *specific*
  reason, and 20 of the 34 reach the spec-ideal answer on every probe they
  currently carry.

That leaves **45 in-scope rows: 31 meet full ODRL 2.2, 14 fall short — 13
demonstrated by a probe, and 1 a structural wire-contract gap with no
probe possible — 0 undetermined.**

At probe grain: **130 of the catalog's 136 probes are judged here** (the
other 6 are named only by rows this page excludes), and of those **115
meet full spec, 15 fall short, 0 undetermined**. That second number is the
honest one and the page says so rather than quoting a bare `N/136`: only
those **15** probes distinguish this page's question from `/coverage`'s.
The other 115 already produce the answer full ODRL 2.2 requires, so they
would look identical on either page.

The fifteen, with the answer that would have to change:

| Probe | This engine | Full ODRL 2.2 |
| --- | --- | --- |
| `op-ispartof-no-hierarchy` | Deny | **Allow** |
| `act-includedin-undeclared-gap` | Deny | **Allow** |
| `lo-datetime-absent-no-clock` | Deny | **Allow** |
| `pf-assignee-scoped-miss` | Deny | **Allow** |
| `op-eq-multi-membership` | Allow | **Deny** |
| `lo-policyusage-literal` | Allow | **Deny** |
| `pc-kind-agreement-ignores-assignee` | Allow | **Deny** |
| `duty-per-permission-advisory` | Allow | **Deny** |
| `duty-consequence-resolves-where-the-primary-did-not` | Allow | **Deny** |
| `op-isa-unparseable` | Error | **Deny** |
| `op-haspart-unparseable` | Error | **Deny** |
| `pc-kind-nonsense` | Allow | **Error** |
| `profile-union-not-per-policy` | Allow | **Error** |
| `ror-reference-key-ignored` | Deny | **Error** |
| `duty-consequence-itself-unresolved` | Allow | Allow — the `duties` list is what falls short |

The headline is `op-ispartof-no-hierarchy`: Vocabulary 3.16.9 defines
`isPartOf` as *containment*, geonames' own RDF has Berlin
`gn:parentCountry` Germany (verified by fetching both `about.rdf`
documents, not assumed), and this engine denies only because
`Operator::IsAnyOf | Operator::IsPartOf` share one arm in
`engine/src/constraint.rs:595-598`. `pf-assignee-scoped-miss` is the one
place the engine is short of the spec by being **stricter** than it, not
laxer.

### The structural gap: `party.collections`

One row has **zero probes** and is judged at row level rather than by any
probe, because the wire contract carries no `PartyCollection`/`odrl:partOf`
concept at all — so no request can pose the question and there is no
answer to be wrong. It renders its `full_compliance_gap` prose directly,
styled as a structural gap rather than a probe verdict: Information Model
2.3/2.3.2 and Examples 9–10 for what the spec requires, then the three
wire-contract additions full support would need — a caller-side
`Request::party_collections` channel (with `Request::asset_collections` as
the asset-side precedent showing the addition is small rather than
architectural), a widened `party_role_mismatch`, and IM 2.5.6's structured
assignee for the refined form.

### Contested judgments: the sign-off list, on the page as well as here

The live half of this page is **measured** — the engine really is driven,
and its real answer really is what you see. The ideal half is a *reading*
of two W3C documents, and on fifteen row/probe entries (fourteen distinct
probes) that reading is contested rather than obvious. The page carries
the whole list in an expandable Info alert above the table, and marks each
affected probe inline, so a reader can find "these specific calls were
judged one way but the spec genuinely admits another" without archaeology.
Five of the fourteen probes were judged to fall short; on the other nine,
what is contested is whether they should have fallen short at all.

| Row | Probe | The fork, in one line |
| --- | --- | --- |
| `actions.included-in-transitive` | `act-includedin-undeclared-gap` | Vocabulary-aware (shipped, ideal Allow, row falls short) vs. closed-profile (ideal = current Deny, **row would meet full spec**). The single highest-leverage judgment on the page. |
| `left-operands.spatial` | `lo-spatial-no-containment` | `eq` is literal equality (shipped, ideal = current) vs. the deployment reading where `spatial eq <region>` means containment (ideal Allow). Picking the latter commits the page to asserting `eq` is not equality. |
| `left-operands.opaque` | `lo-language-no-bcp47` | BCP 47 fixes lexical form only (shipped, ideal = current) vs. BCP 47 pulling in RFC 4647 basic filtering, where `en` matches `en-GB` (ideal Allow). The `/coverage` row's own premise asserts the second reading, so either this page contradicts that row or the row is re-scoped. |
| `left-operands.coordinates` | `lo-absoluteposition-no-ordering` | No ordering over a 2-D tuple (shipped, ideal = current) vs. the componentwise product order (ideal Allow). Separately: `absoluteTemporalPosition` (4.5.3) genuinely *is* orderable, a real gap this probe does not reach. |
| `operators.ordering` | `op-lt-mixed-type-miss` | A datatype violation makes the constraint unsatisfiable (shipped, ideal = current Deny) vs. Vocabulary 4.5.6's MUST making the policy invalid (Error). Under either reading the refusal to coerce is itself correct. |
| `operators.isa-haspart` | `op-haspart-unparseable` | That the current Error is wrong is **settled** (3.14.4's closed twelve). Only the resulting decision forks: `contains` as a set relation (shipped, Deny) vs. mereological reflexivity (Allow). The row falls short either way. |
| `policy-classes.discrimination` | `pc-kind-offer-assignee-inert-even-on-a-match` | Only the Offer's named assignee is inert (shipped, ideal = current Allow) vs. IM 2.1.2's "an Offer does not grant any Rules" read literally, under which a closed-world engine must never Allow from an Offer alone — which is what most dataspace connectors do in practice. |
| `policy-classes.discrimination` | `pc-kind-offer-assignee-inert-even-on-a-mismatch` | The same fork; whichever branch is picked must apply to **both** Offer probes, since the pair exists to show the answer does not depend on the assignee's direction. |
| `policy-classes.discrimination` | `pc-kind-ticket-with-assignee` | Evaluate and ignore the property 4.1.4 says MUST NOT be there (shipped, Allow) vs. Error on the structurally invalid Ticket; plus a third arguable ideal (Deny for want of holder evidence, for which the wire carries no channel). |
| `policy-classes.discrimination` | `pc-kind-nonsense` | Error for consistency with `act-unrecognized-error` (shipped, row falls short) vs. refusing an undeclared Policy subclass being an ODRL *Validator*'s duty (IM 1.3), leaving the current Allow defensible. |
| `policy-classes.set-default` | `pc-kind-nonsense` | The same probe, judged identically in both rows deliberately — a shared probe must not carry two different ideals. Under the Allow branch **this row would have no falls-short probe at all**. |
| `party.assigner-assignee` | `pf-assignee-scoped-miss` | 3.2.3 strips a Set's assignee of all evaluative force (shipped, ideal Allow, row falls short) vs. it denying only *conferral*, leaving a host free to narrow (ideal = current Deny). **The judgment in the whole set most likely to be contested**, and note the engine is short here by being stricter, not laxer. |
| `other.uid` | `uid-rule-index-not-uid` | Whether "fully implements ODRL 2.2" reaches **diagnostics**. Answered once, page-wide: this page judges *decisions*, so ideal = current. |
| `other.uid` | `uid-constraint-no-uid` | The same scope answer — but with a latent decision-level bite: IM 2.5.2's operands-by-reference would be unevaluable in an engine that drops constraint uids, and the wire contract has no reference-by-uid operand form today. |
| `other.right-operand-reference` | `ror-reference-key-ignored` | Structural refusal of a constraint declaring both `rightOperand` and `rightOperandReference` (shipped, Error) vs. honouring the reference charitably (decision stays the current Deny, only the reason changes). Either way this probe does not exhibit the row's *actual* shortfall. |

### The other half of the honesty: 20 rows whose narrowing nothing reaches

Twenty of the 34 implementable rows come back **Meets full spec** — and
that is not a claim that they are now fully compliant. It means their
*current probes* all reach the spec-ideal answer while the narrowing their
own documentation describes is demonstrated by nothing. Each such row
renders the reason inline, and two remedies apply depending on the row: a
new probe, or a re-scoped documented status where the source gap analysis
turned out to be wrong about the engine rather than about the spec.

Nineteen came from the research pass's own needs-a-new-probe backlog:
`actions.implies`, `actions.spec-taxonomy`, `left-operands.numeric`,
`left-operands.spatial`, `left-operands.opaque`,
`left-operands.durations`, `left-operands.coordinates`,
`left-operands.unit-of-count`, `operators.neq`, `operators.ordering`,
`party.inverse-properties`, `party.common-functions`, `duty.obligation`,
`duty.remedy`, `assets.collections`, `assets.target`, `assets.output`,
`other.uid`, `other.inherit-from`. The twentieth,
**`logical.and-sequence`**, did not: it was found by the catalog's own
invariant test, because with two stateless attribute predicates
"satisfied in sequence" degenerates to "satisfied", so Vocabulary 3.17.4's
"in the order specified" — precisely the narrowing that row's caveat names
— is reached by none of its three probes.

Three rows the backlog listed as needing a new probe are conversely
*absent* from that list, having turned out to carry falls-short evidence
anyway: `policy-classes.set-default` (via the shared `pc-kind-nonsense`),
`party.assigner-assignee` (via `pf-assignee-scoped-miss`), and
`other.right-operand-reference` (via `ror-reference-key-ignored`).

`site/src/full_compliance.rs` holds these notes as a constant and asserts,
as an **exact set comparison**
(`the_unreached_narrowing_notes_cover_exactly_the_implementable_rows_that_meet_full_spec`),
that they describe exactly the rows the live judgment puts in that bucket
— so a row silently gaining shortfall evidence and keeping a now-false
note fails `cargo test --workspace` just as loudly as one losing it.

### What this page does not do

It is presentation and catalog-data work only: **no engine decision logic
changed**, and compliance stays 68/68/0/0 with `latest.md` and
`latest.json` byte-identical. It is a **live snapshot** with no Release
History integration in this pass: `site/src/history_page.rs` and
`site/src/history_catalog.rs` are untouched, and nothing on the History
dashboard reads this axis.

`release-history/` carries exactly one line of it, and that line could not
be avoided. That generator compiles `site/src/coverage_catalog.rs` in by
`#[path]` (see "Two things keep the artifact honest anyway" below), so
`CatalogRow` gaining `full_compliance_gap` put the new field in the
generator's crate too, where a `#[cfg(test)]` struct-literal fixture has
to name every field. The fixture sets it to `None`. It compiles out of the
shipped generator binary, adds no History-side consumption of the axis,
and changes no output: `compliance/reports/release-history.json` is
byte-identical, still SHA-256
`32be2fbadf0e1f31eca3c7f2ad01a13425fe7e282ac70b4c18be9f7105686eaa`, and
every release's derived row and probe tallies are unchanged. Keeping the
directory boundary literally intact would have meant storing a row's own
gap somewhere other than that row — a worse shared type bought for a
cosmetic win — so the coupling is recorded here instead of designed
around. And it never asserts full compliance
over an answer it did not observe: a probe that errored at the ABI, or one
whose live answer departed from its own documented behaviour (which
`/coverage` would report as a contradiction, invalidating the premise the
researched ideal was written against), is **Undetermined** — never counted
as meeting the spec *or* as falling short of it.

One ideal is wall-clock dependent by construction and is worth naming
here: `lo-datetime-absent-no-clock`'s, because a compliant engine compares
the *moment of evaluation* against the constraint's bound. On a page with
no fixed evaluation date that would make the demonstrated shortfall expire
on a calendar day, so the probe's right operand is deliberately
`2999-01-01T00:00:00Z` — beyond any instant at which this artifact will be
replayed — rather than a near-term bound carrying a disclosed expiry. The
row's two sibling probes supply `dateTime` as a claim and so are not
time-dependent at all.


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
[`compliance/reports/latest.md`](../compliance/reports/latest.md) for the
full account.

Nothing in this corpus exercises `odrl:xone` or a numeric/date-time
operator this Default Profile doesn't have — a case that did would still
be honestly skipped, cited, and counted, not silently dropped. See "What
this is not" above for the real, remaining gap between this engine and a
general ODRL implementation, which is wider than these numbers might
suggest.


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

