# Bench: odrl-manager (`Prometheus-X-association/odrl-manager`, `develop`)

TypeScript/Node ODRL policy evaluator used in Prometheus-X's Contract
building block. Bench targets the `develop` branch at commit `8842b6b`
(verified, at the time of this bench, to be the more current branch — 12
commits and 13 days ahead of `main` — though the whole repository had no
commit newer than `2025-01-20` at bench time; check `git log` on both
branches yourself before assuming that's still true).

## Setup

odrl-manager's own module resolution (`tsconfig.json`'s `baseUrl: "./src"`)
means these harness files must physically live *inside* a checkout of
odrl-manager itself to import its internals (`PolicyEvaluator`,
`PolicyInstanciator`, etc.) at all — they can't be run standalone the way the
other three engines' harnesses can.

```sh
git clone https://github.com/Prometheus-X-association/odrl-manager.git
cd odrl-manager
git checkout 8842b6b
npm install
npm install --no-save n3 @types/n3      # this harness's own dependency, not odrl-manager's
mkdir -p src/bench
cp <this-directory>/*.ts src/bench/
```

## Run

```sh
ODRL_TEST_SUITE_DATA=/path/to/ds-odrl-engine-rs/compliance/vendor/odrl-test-suite/data \
OUT=results-native.txt \
npx ts-node -r tsconfig-paths/register src/bench/run.ts native

# Adapter-assisted mode (pre-decides assignee/collection/duty-state
# questions the same way compliance-runner does for ds-odrl-engine-rs):
ODRL_TEST_SUITE_DATA=/path/to/ds-odrl-engine-rs/compliance/vendor/odrl-test-suite/data \
OUT=results-assisted.txt \
npx ts-node -r tsconfig-paths/register src/bench/run.ts assisted
```

`ODRL_TEST_SUITE_DATA` defaults to this repo's own
`compliance/vendor/odrl-test-suite/data` (resolved relative to `suite.ts`'s
own file location) if unset — convenient only when these files happen to
still be sitting in their original `bench/odrl-manager/` location rather than
copied into an odrl-manager checkout, which is not the normal case; set it
explicitly once you've copied these files into `src/bench/` as above.

To cross-check this bench's own independently re-derived ground truth against
`ds-odrl-engine-rs`'s own recorded one:

```sh
LATEST_CASES_JSON=/path/to/ds-odrl-engine-rs/compliance/reports/latest-cases.json \
npx ts-node -r tsconfig-paths/register src/bench/crosscheck.ts
```

## What's here

- `suite.ts` — loads and indexes the vendored fixture corpus, and
  independently re-derives the Allow/Deny ground truth from each fixture's
  own `report:*` graph (matching `ground_truth.rs`'s reduction rule).
- `run.ts` — the harness, in two modes documented in its own header comment:
  **native** (structural translation only — a rule with no declared action
  or target falls back to the request's own, the same stand-in
  `compliance-runner`'s adapter uses and for the identical reason) and
  **assisted** (adds the same party/collection/duty-state pre-decisions
  `compliance-runner` makes for `ds-odrl-engine-rs`, for a fair,
  equal-generosity comparison).
- `probe.ts`, `probe2.ts` — vocabulary/capability probes (action coverage,
  numeric comparison, logical constraints, set operators, duties, policy
  classes, assignee handling).
- `crosscheck.ts` — cross-checks this file's own re-derived ground truth
  against `ds-odrl-engine-rs`'s committed one.

## Results

**61/68 native, 67/68 assisted.** The published comparative report argues the
assisted number is the fair one to compare against `ds-odrl-engine-rs`'s own
68/68, since that score is *also* only reached with `compliance-runner`'s
adapter doing the same party/collection/duty-state work — see that report's
own §2.1 for the remaining single difference (`testcase-009`, an action
taxonomy gap: odrl-manager's hardcoded action-inclusion map omits `write`
from `use`'s 45 entries).

## Reproduced

Verified with a fresh `git clone` of `develop` at `8842b6b` + `npm install`
in an isolated scratch location, harness copied into `src/bench/` as
documented above: **61/68 native**, exact match to the recorded result.
