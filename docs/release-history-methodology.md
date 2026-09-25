# Release history dashboard methodology

How the site's Release History page (`/history`) measures what every
tagged release actually did — by re-running this repo's own compliance
and coverage instruments against each historical `engine.wasm`, not by
transcribing changelog claims — and why it is the one page computed at
build time rather than live in the visitor's browser. See the top-level
[README](../README.md) for a quick-start and a map of the rest of the
documentation.

## Release history dashboard

The site's sixth page, **Release History** (`/history`), is a per-release
record of what every tagged version of this engine *actually did* —
measured by re-running this repo's two instruments against each tag, not
by copying numbers out of commit messages.

For each of the 30 tags from `v0.1.0` to `v0.19.2`:

* **ODRL-Test-Suite** — that tag is checked out, and **that tag's own
  `compliance-runner`** is built and run against **the suite revision that
  tag pinned**. The pass rate shown is the one that release genuinely
  recorded, including the releases where it was not 68/68.
* **ODRL 2.2 coverage** — that tag's `engine.wasm` is built for
  `wasm32-unknown-unknown --release`, and **today's** 136-probe catalog
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
`#[serde(default)]` to fall back on. Every one of today's 136 requests is
therefore refused by a `v0.5.0`-or-earlier engine with

```text
request did not parse as the documented Section 5.2 JSON shape:
missing field `recognized_actions` at line 14 column 9
```

before a single line of policy logic runs. The generator detects this
rather than papering over it: a release whose deserializer refuses
*every* request is recorded with `coverage: null` plus the engine's own
rejection message, and the page renders "not addressable" — never 50
contradictions that would only restate one envelope mismatch, and never a
zero that would read as "this release supported nothing". Its historical
compliance number is still shown, because the wire break stops the
coverage replay, not the suite run that release actually performed.

*Partial* rejection is the opposite case and is kept as real signal: a
release whose `Operator` enum has no `isAllOf` variant refuses exactly the
`isAllOf` probes and answers every other probe normally. Every release
carries its own `envelope_rejected` count so the two are distinguishable.
(The current engine rejects 4 probes this way — each one expects an
`Error` decision — and still agrees with all 136.)

### The real historical numbers

Rows are verified/contradicted/inconclusive out of the 50 probeable
vocabulary rows (2 of the catalog's 52 rows carry no probe at all);
probes are agreed/disagreed/errored out of 136.

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
| `v0.11.0` | 01:00 | 68/68 | 38 / 12 / 0 | 112 / 24 / 0 | 4 additive ODRL gaps + feature-flagged DSP/IDSA ingestion |
| `v0.12.0` | 02:35 | 68/68 | 45 / 5 / 0 | 126 / 10 / 0 | per-permission duty/consequence/remedy, party-role evaluation, real `odrl:conflict` |
| `v0.12.1` | 05:15 | 68/68 | 45 / 5 / 0 | 126 / 10 / 0 | note `dsp-odrl-adapter` on the Coverage page |
| `v0.13.0` | 06:05 | 68/68 | 45 / 5 / 0 | 126 / 10 / 0 | add the Release History dashboard itself (this page) |
| `v0.13.1` | 06:09 | 68/68 | 45 / 5 / 0 | 126 / 10 / 0 | fix stale narrative in `compliance-runner`'s own `latest.md` commentary |
| `v0.14.0` | 07:53 | 68/68 | 45 / 5 / 0 | 126 / 10 / 0 | reproducible cross-engine conformance bench harnesses |
| `v0.15.0` | 13:35 | 68/68 | 45 / 5 / 0 | 126 / 10 / 0 | performance, resource and load comparison across all five ODRL engines |
| `v0.16.0` | 18:26 | 68/68 | 49 / 1 / 0 | 134 / 2 / 0 | native `inheritFrom`/conflict-voiding, `AssetCollection` membership, `xsd:duration` comparison, `odrl:andSequence`, `dsp-odrl-adapter` duty/consequence/remedy ingestion + IRI expansion + action pushdown |
| `v0.17.0` | 21:51 | 68/68 | 50 / 0 / 0 | 136 / 0 / 0 | `agreementAssigneeClaim` opt-in, `odrl:Offer` assignee inertness, Release History dashboard repair |
| `v0.17.1` | 22:04 | 68/68 | 50 / 0 / 0 | 136 / 0 / 0 | close the release-history staleness gap in ci.yml too, not just pages.yml |
| `v0.18.0` | 11:28 | 68/68 | 50 / 0 / 0 | 136 / 0 / 0 | per-release Implemented/Partial/NotImplemented/OutOfScope chart, History page typography pass |
| `v0.19.0` | 14:03 | 68/68 | 50 / 0 / 0 | 136 / 0 / 0 | probe-level agreement stat (Coverage page) and history line (History page) |
| `v0.19.1` | 14:45 | 68/68 | 50 / 0 / 0 | 136 / 0 / 0 | fix a disagreeing probe being hard to spot: vertical-align: top, per-probe agreed/disagreed labels |
| `v0.19.2` | 15:21 | 68/68 | 50 / 0 / 0 | 136 / 0 / 0 | preview a Partial/NotImplemented row's own boundary probe by default, not just its first (usually positive) probe |

Four things in that table are worth reading twice, because none of them
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
* **Every tag from `v0.12.0` through `v0.15.0` shares one contradicted
  count (5 rows, 10 probes), and every tag from `v0.17.0` through
  `v0.19.2` shares another (0 rows, 0 probes), because none of them touch
  `engine/` at all** (Release History itself, bench harnesses, dashboard
  presentation, Coverage/History page additions — see "byte-identical
  `engine.wasm`" below). Only `v0.16.0` and `v0.17.0` move the count,
  each by adding real engine capability; `v0.17.1`, `v0.18.0`, `v0.19.0`,
  `v0.19.1` and `v0.19.2` are CI/presentation-only and inherit
  `v0.17.0`'s numbers exactly.
* **`v0.11.0`'s 12 contradicted rows today are exactly the coverage rows
  this study has added since it shipped, in full**:
  `assets.collections`, `conflict.fixed-strategy`,
  `conflict.profile-strategies`, `conflict.property`, `duty.consequence`,
  `duty.per-permission`, `duty.remedy`, `left-operands.durations`,
  `logical.and-sequence`, `other.inherit-from`, `party.assigner-assignee`
  and `policy-classes.discrimination` (measured against the committed
  `compliance/reports/release-history.json`, not asserted). `git show
  v0.11.0:engine/src/wire.rs` contains none of `odrl:assignee`,
  `odrl:conflict`, `odrl:duty`, `odrl:remedy`, `odrl:consequence`,
  `odrl:inheritFrom`, `odrl:andSequence`, `odrl:AssetCollection`, `xsd:duration`
  or the `Agreement`/`Offer` party-role work this pass adds — every one of
  those wire keys and behaviours arrives later, so a v0.11.0 engine
  ignores what it does not model (additively) and answers `Allow` where
  the current catalog expects `Deny`. This count is not fixed: it grows
  by exactly one capability-shaped row every time a later pass adds real
  coverage `v0.11.0` predates, which is the whole reason this bullet says
  "today" rather than naming a number that would need updating by hand
  forever. Genuinely absent capability at every step, never a harness
  artefact.

### A per-release full-compliance breakdown, replacing the documentation-facing one

Through v0.20.3, this page's stacked chart and its two non-compliance
line-chart series were measured against **this study's own
documentation** — the exact axis `/coverage` (now the Capability Audit
page) judges. Once `/full-compliance` shipped in v0.20.0 to judge the
same probes against the **spec ideal** instead, keeping the History
dashboard's headline charts on the documentation axis read as though this
page tracked progress toward ODRL 2.2 itself, when it did not. Both
charts were rewired to the full-compliance axis instead: the stacked
chart's four bands are now **Meets full spec** / **Undetermined** /
**Falls short** / **Structural gap**, and the line chart's non-compliance
pair is now **in-scope rows meeting full spec** (row grain) and
**individual probes meeting full spec** (probe grain), both fractions of
the 45 rows / 130 probes `/full-compliance` actually judges — not the
52 rows / 136 probes `/coverage` does, since the seven `OutOfScope` rows
sit outside the wire contract for either axis but the full-compliance one
excludes them from its own denominator rather than folding them into a
bucket.

`Release` now carries `full_compliance: Option<FullComplianceTally>`
instead of the retired `row_status: Option<RowStatusBreakdown>` —
computed by replaying that release's own probe outcomes through
`site/src/full_compliance.rs`'s `compile_full_compliance_report`, the
identical pure function the live `/full-compliance` page calls in the
browser, included here by path for the same no-drift reason
`coverage_catalog.rs` already was (see `release-history/src/main.rs`'s
own `mod full_compliance` doc comment). No bespoke per-release
classification rule was needed this time, unlike the retired
`classify_row_for_release`: `compile_full_compliance_report`'s own
row verdict (`RowSpecVerdict::MeetsFullSpec`/`FallsShort`/
`StructuralGap`/`Undetermined`) already treats `party.collections`'
structural gap as release-invariant by construction — the check runs
*before* any probe outcome is even consulted — so the historical replay
needed no equivalent of the old pinning logic to keep that band honest.

A few real, addressable releases across the range (rows, out of the 45
in-scope; `Meets` / `Undetermined` / `Falls short` / `Structural gap`):

| tag | Meets | Undetermined | Falls short | Structural gap |
|---|---|---|---|---|
| `v0.6.0` (oldest addressable) | 15 | 20 | 9 | 1 |
| `v0.11.0` | 24 | 10 | 10 | 1 |
| `v0.16.0` | 31 | 0 | 13 | 1 |
| `v0.20.3` (newest) | 31 | 0 | 13 | 1 |

**Worth reading twice: `Falls short` grew from 9 to 13 as capability
landed, and that is not a regression.** Every row this catalog could not
yet judge at all sits in `Undetermined` until the engine answers its
probes one way or the other — v0.6.0's 20 `Undetermined` rows are mostly
rows whose probes did not exist yet, not rows genuinely ambiguous today.
As v0.6.0 through v0.16.0 added real coverage, all twenty of those rows
resolved to exactly one of `Meets` or `Falls short` — sixteen to `Meets`,
four to `Falls short`, a real, disclosed shortfall against the spec ideal
rather than the harness previously being unable to ask the question.
`Structural gap` (`party.collections`) is the one band that never moves,
for the identical structural reason `OutOfScope` never moved on the
retired chart. `Falls short` has held at 13 (its underlying probe count
at 15) since v0.16.0 — this study's own full-compliance research is
current as of the same commit that added `/full-compliance`, not
something later engine work has quietly resolved.

**Schema bump.** This is a real shape change — `row_status` retired and
`full_compliance` added on every release, plus four new
`full_compliance_*` fields on `CatalogInfo` — so
`release-history/src/render.rs`'s `SCHEMA` (and
`site/src/history_catalog.rs`'s matching `HISTORY_SCHEMA`) moved from
`ds-odrl-engine-rs/release-history@2` to `@3`. A browser holding an `@2`
copy of the artifact now fails loudly (`declares schema ds-odrl-engine-rs/
release-history@2, this page speaks ds-odrl-engine-rs/release-history@3`)
rather than rendering a dashboard with the new axis silently absent
everywhere.

### Row and probe grain, on both live pages and on this one

Both of this page's non-compliance line-chart series, and every band in
the stacked chart above, are at **row** granularity: a row counts as
meeting full spec only when *every one* of its own judged probes does.
That is the right grain for "does this vocabulary claim hold against the
spec," but it hides a real fact the raw data already carries — one
falls-short probe among several sinks its whole row, so the row-level
share understates how much of the corpus actually meets the spec.
`Release::meets_full_spec_probe_fraction` (`site/src/history_catalog.rs`)
adds the finer-grained axis directly from each release's own
already-committed `FullComplianceTally.probes_meets`/probe total. A test,
`meets_full_spec_probe_fraction_is_a_finer_grain_than_the_row_fraction`,
pins this as a real invariant against the committed data: for every
addressable release, the row-meets share never exceeds the probe-meets
share.

The same row-vs-probe distinction was added to the **Capability Audit**
page's own live report back at v0.19.0, on the *documentation* axis
rather than this one: `CoverageReport.agreed`/`.disagreed`/`.errored`/
`.total_probes` were computed and used internally (to derive the
row-level `verified`/`contradicted`/`inconclusive` figures next to them)
but never surfaced as their own headline until that pass added a third
stat row (probes run/agreed/disagreed/errored) alongside the two existing
row-granularity ones.

### Why this page is not live in your browser

The Compliance Results, Capability Audit and ODRL 2.2 Full Compliance
pages all re-execute their whole corpus against `engine.wasm` in the
visitor's browser, and say so. This one cannot: its subject is **34
different historical `engine.wasm` binaries**, 8.9 MB of them combined,
which would have to be shipped and instantiated to recompute 4,624 probe
evaluations (34 releases × 136 probes) on page load, twice over (once for
each judging axis) — for figures that can only change when someone cuts a
new tag. Both the page's own intro paragraph and this figure are read
off the same `HistoryFile` at render time (`site/src/history_page.rs`'s
`intro`/`build_time_alert`), not repeated here or there as a literal that
would need updating by hand at the next tag — this README passage is the
one place that still is, and is judged worth it for a document a reader
skims rather than fetches. The page states the mechanism in an Alert
above the dashboard, and carries each release's `engine.wasm` SHA-256 so
the claim is checkable rather than asserted: rebuild any tag and compare.

Two things keep the artifact honest anyway. The verdicts are derived by
**`site/src/coverage_catalog.rs`** (the Capability Audit axis) **and
`site/src/full_compliance.rs`** (the ODRL 2.2 Full Compliance axis) —
the very two modules the live pages of the same names run in the
browser — both pulled into the generator with `#[path]` rather than
reimplemented (`release-history/src/main.rs`'s own doc comments on those
two `mod` declarations state the reasoning), so neither can drift into
disagreeing with its live counterpart about what its own verdicts mean;
the probe catalog itself is likewise read fresh off
`compliance/reports/latest-coverage.json` at generation time
(`release-history/src/main.rs`'s `main`), never a copy baked in earlier,
so regenerating after a `coverage-probes` change picks up new probes for
every historical binary automatically. And two workspace tests,
`site::history_catalog::tests::the_newest_release_agrees_with_the_current_catalog`
and `release-history`'s own
`the_newest_releases_full_compliance_tally_sums_to_the_catalogs_in_scope_rows`,
assert that the newest staged release's own tallies are addressable and
internally consistent, so a stale regeneration fails `cargo test
--workspace` instead of quietly shipping an old dashboard. (As of this
artifact: 50 verified / 0 contradicted / 0 inconclusive / 2 documented,
and 31 meets / 13 falls short / 1 structural gap of 45 in-scope rows, for
`v0.20.3`, the newest tag — matching what the live Capability Audit and
ODRL 2.2 Full Compliance pages themselves report for the same catalog.)

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

**Automated for every deploy, unlike its sibling `compliance/reports/`
artifacts** — and this was not always true. `latest.json`/
`latest-cases.json`/`latest-coverage.json` are *per-commit* artifacts:
`.github/workflows/ci.yml` regenerates them and fails on `git diff
--exit-code compliance/reports/`, which only works because they are cheap
to regenerate on every push. `release-history.json` was left out of that
loop for the opposite reason — regenerating it means rebuilding every
historical engine, not just today's — and "regenerate it by hand when you
cut a release" is exactly the manual step that let it go stale for five
real tags (`v0.13.0` through `v0.16.0`) plus this one, unnoticed for a
full compliance-widening pass, before this repair.

The fix is not "remember harder": `.github/workflows/pages.yml`'s `build`
job now runs both stages itself, on every push that redeploys the site,
before `trunk build`. It is not committed back to the repository — a bot
commit touching `compliance/reports/release-history.json` would
re-trigger the same workflow via its own `paths` filter, and avoiding
that loop outright is simpler than guarding against it — so the checked-in
copy in this repository stays a point-in-time snapshot, refreshed by hand
alongside a tagged release exactly as described above; only the
*deployed* dashboard is guaranteed current. What makes rebuilding
history-on-every-push affordable rather than wasteful: an `actions/cache`
step keyed on a hash of the current `git tag -l` output caches
`target/release-history/stage`, so a push that adds no new tag is a cache
hit and stage 1 above is a no-op, and a push that adds exactly one tag
restores every older tag's staged artifacts (via the cache's own
`restore-keys` prefix fallback) and builds only the new one — never a
full rebuild of every historical tag on an ordinary `site/**` or
`engine/**` change. See that workflow file's own comments ahead of
`compute release-history cache key` for the full reasoning, including why
`fetch-depth: 0` is required on the checkout step (`git tag -l` and this
script both need real tag history, which the default shallow clone does
not carry).

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
# 8 × sha256 32be2fbadf0e1f31eca3c7f2ad01a13425fe7e282ac70b4c18be9f7105686eaa
# ... identical to sha256sum compliance/reports/release-history.json
```

A second, unplanned reproducibility result fell out of the sweep: tags
whose `engine/` tree is byte-identical produce a **byte-identical**
`engine.wasm`. `v0.1.0`/`v0.1.1`/`v0.1.2`, `v0.2.1`–`v0.5.0`,
`v0.8.1`/`v0.9.0`, `v0.10.0`/`v0.10.1`, `v0.12.0`–`v0.15.0` (a group
that grew from a pair to six tags across this repair, since none of
`v0.13.0`, `v0.13.1`, `v0.14.0` or `v0.15.0` touch `engine/` either) and
now `v0.17.0`/`v0.17.1`/`v0.18.0`/`v0.19.0`/`v0.19.1`/`v0.19.2` (a CI-only
tag and four presentation-only tags, per their own commit messages) each
share one SHA-256, and `git diff <a> <b> -- engine` is empty for every
one of those pairs. The engine build is reproducible across checkouts on this
toolchain.

