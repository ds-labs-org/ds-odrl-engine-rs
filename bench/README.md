# ODRL engine bench

Reproducible harnesses for the five-engine comparison published as
[`docs/benchmarks/2026-09-06-odrl-engine-comparative-coverage.md`](https://github.com/Deepthought-Solutions/dataspace/blob/main/docs/benchmarks/2026-09-06-odrl-engine-comparative-coverage.md)
in the sibling `dataspace` repository (ds42.org) — every external
engine scored against the identical 68-fixture `SolidLabResearch/ODRL-Test-Suite`
corpus this repo already vendors at `compliance/vendor/odrl-test-suite`, reduced
to Allow/Deny by the same rule `compliance-runner/src/ground_truth.rs` uses.

This is conformance/coverage reproducibility only — the harnesses reproduce a
pass/fail tally against the fixture corpus, not performance, resource
consumption, or load/peak behavior. Those dimensions were scoped out of this
pass; extending the harnesses here to measure them is a real, separate
follow-up, not implied by anything below.

## What's here, and what deliberately isn't

Each subdirectory holds the harness code actually written and run for that
engine, plus its point-in-time result JSON as a committed record. **No
third-party engine's own source tree is vendored here** — every subdirectory's
own README states the exact `git clone`/`checkout`/`npm install` a reproducer
runs first, against the exact pinned commit or version this comparison used.
That is the same discipline `compliance/vendor/odrl-test-suite` already
applies to this repo's own primary corpus, applied here to four more external
targets that don't fit a single git submodule cleanly (one is an npm package
at two different pinned versions, one needs real infrastructure beyond a
checkout).

| Directory | Engine | Language / runtime | Result reproduced |
|---|---|---|---|
| [`solidlab-evaluator/`](solidlab-evaluator/) | `SolidLabResearch/odrl-evaluator` (the vendored suite's own reference implementation) | TypeScript / npm | 63/68 (`0.4.0`, the version the suite itself pins), 67/68 (`0.6.0`) |
| [`oval/`](oval/) | `DIPS-Tools/odrl-Engine` | Python | 59/68 |
| [`odrl-pap/`](odrl-pap/) | `SEAMWARE/odrl-pap` (FIWARE's own ODRL→Rego→OPA component) | Java/Quarkus → Rego → OPA | 30/1/37 with a mapping overlay, 20/11/37 stock |
| [`odrl-manager/`](odrl-manager/) | `Prometheus-X-association/odrl-manager` (`develop` branch) | TypeScript / Node | 61/68 native, 67/68 with adapter assistance |

`ds-odrl-engine-rs` itself is the fifth row of that comparison and needs no
harness here — its own number is `compliance/reports/latest.json`, produced by
`cargo run -p compliance-runner --release` in this same repo.

## Reproduced, for real, in this pass

Three of the four were verified with a genuine clean reproduction — a fresh
clone/install of the target engine at its pinned version, in an isolated
scratch location, running the rescued harness exactly as its own README
documents — and each reproduced the exact recorded tally:

- **`solidlab-evaluator`**: fresh `git clone` of `SolidLabResearch/ODRL-Test-Suite`
  (the harness's own host project) + `npm install` → **63/68**.
- **`oval`**: fresh `git clone` of `DIPS-Tools/odrl-Engine` at `a427e71` + a
  Python venv from `requirements.txt` → **59/68**.
- **`odrl-manager`**: fresh `git clone` of the `develop` branch at `8842b6b` +
  `npm install` (plus `n3`/`@types/n3`, not one of odrl-manager's own
  dependencies but needed by this harness) → **61/68** native.

**`odrl-pap` was not given a full clean-infrastructure reproduction in this
pass** — it genuinely needs PostgreSQL, OPA, and a running Quarkus service
before one fixture can be evaluated, which is a real cost to pay just to
re-confirm a number, not a quick check. Its harness was verified for
path-independence (no hardcoded absolute path or scratch-specific reference
survived the rescue) but not re-run end to end. Its own README says so
plainly; treat its numbers as carried forward from the original run, not
re-verified here.

## A real, honest limitation of this rescue

These four harnesses were originally written and run in one long working
session, then existed only in that session's own temporary scratch directory
— not committed anywhere — until this pass moved them here. Some
hard-learned specifics from that original work are preserved in the code's
own comments (a cwd-relative ontology path in OVAL's upstream that silently
breaks its own action-taxonomy reasoning if the working directory is wrong; a
mutable-default-argument bug in the same upstream that leaks state across a
evaluation loop; an upstream `FORCE_translator.py` `__main__` block that
calls the wrong-direction function). Read each subdirectory's own harness
source comments before assuming a number here still holds against a newer
commit of the target engine — none of these four projects' own APIs are
frozen, and this rescue captures one specific pinned commit's behavior, not
an ongoing tracking relationship.
