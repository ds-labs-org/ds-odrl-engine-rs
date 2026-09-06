# Bench: SolidLab ODRL Evaluator

The vendored `SolidLabResearch/ODRL-Test-Suite` corpus's **own** reference
evaluator (`odrl-evaluator` on npm) — scoring it against its own suite is a
genuinely different exercise from scoring a stranger's engine (see the
published comparative report's own "how fair is this" section), but it is
also the one comparator that needs zero translation: the fixtures are already
its native input format.

## Setup

```sh
git clone https://github.com/SolidLabResearch/ODRL-Test-Suite.git
cd ODRL-Test-Suite
npm install                    # pulls odrl-evaluator@^0.4.0, the suite's own pin
mkdir -p bench
cp <this-directory>/bench/*.ts bench/
```

To bench the newer `0.6.0` instead of the pinned `0.4.0`:

```sh
npm install odrl-evaluator@0.6.0
```

## Run

```sh
OUT=allow-deny-results.json npx ts-node bench/allow-deny-bench.ts
```

`bench/probes.ts`, `bench/dump-case.ts` and `bench/full-report-compare.ts` are
the supporting vocabulary-probe and single-case-inspection tools used while
producing the comparative report's own vocabulary/capability section — run
them the same way, reading each file's own header comment for its exact
invocation.

## What's here

- `bench/allow-deny-bench.ts` — the harness. Reduces the suite's own
  `report:*` compliance report (both the fixture's expected one and the one
  the evaluator actually produces) to Allow/Deny by the identical rule
  `ground_truth.rs` uses, and scores every case.
- `bench/probes.ts` — targeted vocabulary probes (nested logical constraints,
  set operators, numeric comparison, duty handling) used for the comparative
  report's capability-comparison table.
- `bench/dump-case.ts`, `bench/full-report-compare.ts` — single-case
  inspection tools, used while diagnosing specific fixture disagreements.
- `results/allow-deny-results.json` — `0.4.0` result: **63 pass, 5 fail, 0
  error**, out of 68.
- `results/allow-deny-results-060.json` — `0.6.0` result: **67 pass, 1 fail,
  0 error**.

## Reproduced

Verified with a fresh `git clone` + `npm install` in an isolated scratch
location: **63/68** at `0.4.0`, exact match to the committed result.
