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

Re-verified again at the start of the performance pass below, on its own fresh
clone of the same commit: **61/68 native, 67/68 assisted** — both exact.
`results/verify-conformance-native.stdout.txt` and
`…-assisted.stdout.txt` are that run's own output, and every performance number
below comes from the same checkout that produced them.

---

# Performance, resources and load

A separate pass from the conformance work above, run on 2026-09-06. It adds
five instrumentation files beside the existing harness and writes raw output to
`results/`; nothing here changes `run.ts`, `suite.ts` or the 61/68 tally.

## What this engine physically is

A TypeScript library. No service, no port, no database, no daemon. A host
imports it, hands `PolicyInstanciator.genPolicyFrom()` an ODRL-JSON object, and
calls `evaluator.isActionPerformable(action, target)`, which returns a
`Promise<boolean>`. The library never sees RDF: the Turtle parse and the
translation to its JSON shape are this harness's own adapter.

That single fact shapes everything below — there is no HTTP axis to load-test,
no request rate, and the only two things that can be timed are the engine call
and the adapter around it. Both are.

## Environment, and the baseline load

Captured by `run_perf.sh` step 0 *before* anything heavy ran →
`results/environment.txt`.

| | |
|---|---|
| Kernel | Linux 6.8.0-138-generic x86_64 |
| CPU | Intel Core Ultra 9 185H — 22 threads, 2 threads/core, hybrid |
| `nproc` | 22 |
| `free -h` total / available | 93 GiB / 71 GiB, **no swap configured** |
| `MemAvailable` at start | 75,088,068 kB (71.6 GiB) |
| Node actually used | **v24.18.0** (V8 13.6.233.17-node.50) |
| npm | 12.0.1 |
| git | 2.43.0 |

Baseline at 12:57:21 local: `load average: 2.63, 10.18, 6.60`. The 5- and
15-minute figures are **residual decay from this same pass's own earlier,
discarded driver run, which ended at 12:54:42** — not competing work. The `ps`
listing captured in the same file at the same instant contains no benchmark
process of any kind (only idle desktop `firefox-bin`/`Xorg`/`cinnamon` and
`claude` CLIs), and a `node`-process count taken immediately before the driver
started returned 0.

The elevated 1-minute figure is worth one more check rather than one more
sentence: the discarded run's own per-case medians were 0.048 / 0.049 / 0.050 ms
against this run's 0.047 / 0.046 / 0.049 ms, on a machine whose 1-minute load
average differed by 2.1. The measurement is not sensitive to it.

## The invocation path, and why it is the conformance harness's

`perf_corpus.ts` is `run.ts`'s fixture reading, constraint/policy translation,
host fetchers and engine call lifted **verbatim** into exported functions, with
its module-level `MODE` const turned into a parameter. `run.ts` is untouched and
still produces 61/68 on its own.

Two units are timed, because odrl-manager's API boundary is not where the RDF is:

- **engine-only** — `EntityRegistry.cleanReferences()` → `new PolicyInstanciator()`
  → `genPolicyFrom(json)` → `new PolicyEvaluator()` → `setPolicy(...)` →
  `await isActionPerformable(action, target)`, on an already-translated policy.
  This is the engine's own cost and the **primary** latency figure.
- **end-to-end** — `run.ts`'s entire per-case body: three `n3` Turtle parses
  (policy, request, state-of-the-world), the translation, then the same engine
  call. This is what a host starting from RDF on disk pays.

Both are reported everywhere. Neither is presented as the other. Both agree with
the conformance harness on every decision: each timed repeat records **7
mismatches out of 68 in native mode — exactly the 61/68 — and 0 errors**.

### Reproducing the whole pass

```sh
SCRATCH=/some/isolated/dir \
RESULTS=<this-directory>/results \
BENCH_SRC=<this-directory> \
CORPUS=<ds-odrl-engine-rs>/compliance/vendor/odrl-test-suite/data \
bash run_perf.sh
```

`run_perf.sh` records the environment, clones at the pinned commit, installs
(cold cache then warm), builds, measures the footprint, re-runs the
**conformance** harness first, then runs `perf_bench.ts` (three separate
processes, plus one assisted-mode run) and `load_bench.ts` under
`/usr/bin/time -v`, then sweeps for stragglers. It runs nothing in the
background. Whole pass: **5m14s** wall as originally run, plus the **35 s** soak
(§4) that was added to the driver afterwards — so ≈5m50s for a fresh run of the
script as it now stands.

## 1. Warmup and smoke test

**5 discarded full corpus passes = 340 evaluations**, before any timed
measurement, in every perf process; **2 passes = 136 evaluations** in every one
of the 44 load workers. It doubles as the smoke test — a warmup exception aborts
the process and refuses to print timings rather than letting a broken engine
produce numbers. Every run completed warmup cleanly. Raw: `warmup_pass_ms` and
`warmup_first_call_ms` in `results/perf-*.json`.

| run | pass walls, ms (in order) | first call of each pass, ms |
|---|---|---|
| native p1 | 31.4, 22.9, 19.6, 23.8, 19.4 | **1.697**, 0.066, 0.059, 0.060, 0.054 |
| native p2 | 30.9, 22.7, 18.7, 22.1, 18.6 | **1.604**, 0.068, 0.057, 0.060, 0.053 |
| native p3 | 32.5, 21.4, 19.0, 32.2, 31.9 | **1.783**, 0.077, 0.059, 0.053, 0.077 |
| assisted | 28.8, 19.7, 18.3, 22.0, 18.6 | **1.617**, 0.062, 0.091, 0.050, 0.055 |

There **is** a real cold-start cliff here, and it is sharp and short: the very
first evaluation in a process costs **1.60–1.78 ms against a warm 0.047 ms —
about 34×** — and it is gone by the second call. At the pass level the first
pass is 1.6× the third. This is why warmup is 340 evaluations and not 10: a
10-iteration warmup would still have been on the descending part of the curve.

## 2. Per-case latency, post-warmup

**20 repeats of all 68 fixtures = 1,360 timed evaluations per path per process**,
timed individually with `process.hrtime.bigint()`, in **3 separate native
processes** plus one assisted process. Figures below are the median of the three
native processes; the per-process spread is in §6.

| path | mean | median | p95 | p99 | min | max |
|---|--:|--:|--:|--:|--:|--:|
| **engine-only** | 0.292 | **0.047** | 0.238 | 5.035 | 0.025 | 12.536 |
| end-to-end (incl. RDF parse) | 0.984 | 0.161 | 0.490 | 18.917 | 0.114 | 26.235 |

All values ms. Assisted mode is indistinguishable (engine-only mean 0.290,
median 0.047, p99 4.976) — dropping out-of-scope rules at translation time does
not change the engine's cost measurably.

The 27,200-evaluation soak of §4 gives the same shape with more JIT exposure and
lands slightly faster: engine-only mean 0.275, **median 0.042**, p95 0.183,
p99 4.805, min 0.024, max 44.620 (that max is one major GC pause, not a policy).
The 0.047 ms above is the conservative figure and the one quoted.

**The distribution is bimodal, and it is the corpus, not the engine.** Three of
the 68 fixtures (`testcase-062/063/064-big-policy`) carry policies an order of
magnitude larger than the rest. Split out (native p1):

| subset | n | mean | median | p95 | p99 | min | max |
|---|--:|--:|--:|--:|--:|--:|--:|
| engine-only, 65 light fixtures | 1300 | 0.052 | 0.047 | 0.090 | 0.197 | 0.025 | 0.347 |
| engine-only, 3 big-policy fixtures | 60 | 5.512 | 4.642 | — | — | 3.934 | 12.536 |
| end-to-end, 65 light fixtures | 1300 | 0.166 | 0.156 | 0.259 | 0.428 | 0.114 | 0.538 |
| end-to-end, 3 big-policy fixtures | 60 | 18.684 | 18.389 | — | — | 17.321 | 35.588 |

So the p99 of 5.0 ms is entirely the big-policy fixtures being 4.4% of the
corpus. A reader who cares about typical policies should read the 0.047 ms
median and the 0.197 ms light p99; a reader who cares about the tail should
read 4.6 ms and know exactly which three fixtures produce it.

**The RDF adapter costs more than the engine.** End-to-end median 0.161 ms
against engine-only 0.047 ms: **~70% of a from-Turtle evaluation is `n3` parsing
and translation, not policy reasoning.** On the big-policy fixtures the ratio is
worse still — 18.4 ms end-to-end against 4.6 ms in the engine. That is a
property of this harness's adapter, not of odrl-manager, and is stated here
precisely so the engine-only number is not read as an end-to-end one.

Extremes, per fixture (engine-only, native p1, `per_case` in the JSON): slowest
`testcase-064-big-policy-past` at 4.781 ms median, then 063 (4.604) and 062
(4.585); then a two-order-of-magnitude gap to `testcase-065-alice` (0.202);
fastest are `testcase-058-bob-write-y` and three others at 0.030 ms.

## 3. System requirements

Everything actually needed to get from nothing to a running evaluation, measured
by `/usr/bin/time -v` on each step (`results/*.time.txt`).

| step | wall | CPU (user+sys) | peak RSS |
|---|--:|--:|--:|
| `git clone` | 1.04 s | 0.10 s | 16.5 MB |
| `npm install`, **cold** cache | **8.16 s** | 2.53 s | 226 MB |
| `npm install`, warm cache | 0.68 s | 1.11 s | 261 MB |
| `npm install n3 @types/n3` (harness's own dep) | 2.10 s | 0.69 s | 134 MB |
| `npm run build` (tsup — **not** on the measured path) | 1.14 s | 2.09 s | 390 MB |
| conformance re-run, 68 cases (native) | 1.10 s | 2.26 s | 398 MB |

- **Runtime**: Node **v24.18.0**, npm 12.0.1. No JVM, no Python, no database, no
  container. `git` 2.43.0 for the clone.
- **Build step**: there is one (`npm run build` → tsup → `dist/`, 1.14 s), but
  **nothing measured here uses it**. The conformance harness runs from `src/`
  through `ts-node`, so the perf instrumentation does too, to stay on the same
  code path. The build is timed because "what does it cost to build this
  engine" is a fair question, not because anything below depends on it.
- **On-disk footprint** (`results/footprint.txt`, `du -sh`):

  | | |
  |---|--:|
  | whole checkout incl. `node_modules` | **86 MB** |
  | `node_modules` (213 packages + 12 for `n3`) | 85 MB, 2,961 files |
  | odrl-manager's own `src/` | 520 KB |
  | built `dist/` | 544 KB |
  | `.git` | 628 KB |

  The dependency tree is almost entirely **dev** tooling: `typescript` 31 MB,
  `@esbuild` 9.1 MB, `prettier` 8.2 MB, `@rollup` 5.1 MB, `mocha` 2.2 MB,
  `ts-node` 1.6 MB. The engine's own runtime surface is the 520 KB of `src/`
  (544 KB built). An 86 MB tree is what it costs to *develop against* this
  library at its pinned commit, not what it costs to ship it.

## 4. Resource consumption during the performance run

Two independent instruments, both real:

- `/usr/bin/time -v` around the whole `perf_bench.ts` process — `Maximum
  resident set size` and `User`/`System time` straight from the kernel
  (`results/perf-native-p*.time.txt`).
- an in-process series: `/proc/self/status` `VmRSS` sampled at each phase
  boundary, plus `process.cpuUsage()` (`rss_series`, `cpu_ms` in the JSON).

| | p1 | p2 | p3 | assisted |
|---|--:|--:|--:|--:|
| wall (whole process) | 3.12 s | 3.05 s | 3.14 s | 3.05 s |
| CPU user + sys | 5.02 s | 4.89 s | 4.99 s | 4.88 s |
| % CPU | 160% | 160% | 159% | 159% |
| **peak RSS (`time -v`)** | **686 MB** | 684 MB | 700 MB | 681 MB |
| RSS after import (in-process) | 349 MB | 347 MB | 342 MB | 347 MB |
| RSS after warmup | 391 MB | 390 MB | 381 MB | 390 MB |
| RSS peak (in-process) | 666 MB | 664 MB | 688 MB | 661 MB |

Reading these honestly:

- **~349 MB is resident before a single evaluation happens**, and that is mostly
  `ts-node`: it loads the TypeScript compiler and type-checks the program at
  startup. `startup_ms` (import to first line of `main`) is **779–824 ms** and
  is the same cost the conformance harness pays. It is harness weight, not
  engine weight — a host consuming the built `dist/` would not pay it.
- **RSS rises monotonically through a run, and that needed a real answer rather
  than a reassuring sentence.** Across p1's 20 engine-only repeats
  `rss_end_kb` climbs 401 → 478 MB with only 2 reclaims, and through the
  end-to-end repeats 493 → 666 MB with no reclaim at all. A monotonic rise over
  1,360 evaluations is exactly what a leak looks like, so a **soak run** was
  added: `perf_bench.ts --repeats 400 --no-raw-samples`, i.e. **27,200
  evaluations per path** (`results/perf-native-soak.json`, 35.05 s wall,
  45.08 s CPU, `time -v` peak RSS **1,084 MB**).

  | engine-only repeat | 0 | 40 | 80 | 160 | 240 | 320 | 399 |
  |---|--:|--:|--:|--:|--:|--:|--:|
  | RSS at end of repeat (MB) | 386 | 596 | 829 | 832 | 888 | 926 | **926** |

  The curve **decelerates hard and plateaus**: the first 5,440 evaluations add
  434 MB (**81.7 KB/evaluation**), the next 21,760 add only 106 MB
  (**5.0 KB/evaluation**) — a **16× drop in growth rate** — and the end-to-end
  phase that follows oscillates in a 927–1,056 MB band with six reclaims, the
  largest 32.7 MB. That is V8 growing its heap toward an equilibrium under a
  tight allocation loop, not unbounded retention. Three independent cross-checks
  agree: throughput does not degrade across the soak (last ten repeat walls
  15.6–20.4 ms against the first ten's 16.3–30.0 ms, and all 400 repeats still
  report the same 7 mismatches and 0 errors); the 44 independent load workers,
  each doing ~33,000 evaluations, settle in the same **0.75–1.0 GiB** band
  rather than climbing until the guard trips; and a first soak run (before
  `--no-raw-samples` was added, so its JSON is not kept) independently reached
  941 MB / `time -v` 1,076 MB on the same trajectory.

  **Practical reading: size a long-lived odrl-manager process at ~1 GiB
  resident, not at the 391 MB it shows after warmup.**
- **The engine uses more than one core even single-threaded.** The engine-only
  phase is 400.3 ms wall for 690.3 ms CPU (**1.72 cores**); the end-to-end phase
  1339.3 ms wall for 1725.8 ms CPU (**1.29 cores**). The excess is V8's
  background GC and optimising-compiler threads. Anyone sizing this engine as
  "one core per single-threaded Node process" would under-provision by ~30–70%
  under sustained load.

## 5. Load and peak behaviour

### What "concurrency" means for THIS engine — and it is not what it looks like

**One unit of concurrency = one OS process**: a forked Node process running
`load_worker.ts`, with its own module import, its own translated corpus and its
own 136-evaluation warmup. Nothing is shared.

The tempting alternative was in-process async concurrency: odrl-manager is
Promise-based end to end, so `Promise.all` over N evaluations *looks* like the
engine's own natural model. `perf_bench.ts`'s `asyncProbe` measured it instead
of assuming, at five widths, in all four perf processes:

| width | sequential ms | `Promise.all` ms | speedup | **wrong answers vs. sequential** |
|--:|--:|--:|--:|--:|
| 2 | 0.114 | 2.642 | 0.043× | **1 / 2** |
| 4 | 0.245 | 0.609 | 0.402× | **3 / 4** |
| 8 | 0.347 | 1.056 | 0.329× | **4 / 8** |
| 16 | 0.629 | 1.571 | 0.400× | **6 / 16** |
| 68 | 19.928 | 11.596 | 1.719× | **26 / 68** |

(native p1; p2, p3 and assisted reproduce the wrong-answer counts **1, 3, 4, 6,
26 exactly**, in all four processes, with the same fixture slugs.)

**In-process concurrency does not work on this engine, and the failure is
silent.** `EntityRegistry` keeps its state in `private static` fields — one
entity table per Node *process*, not per policy — and `run.ts` clears it with
`EntityRegistry.cleanReferences()` before every `genPolicyFrom` precisely
because of that. Interleaved evaluations therefore tear each other's entity
graph out mid-flight. The engine prints its own
`Error in "isActionPerformable"` lines from an internal catch (85 of them in
p1, **all inside the probe's stderr fence, zero in the sequential timed runs** —
verified by the `ASYNC PROBE BEGIN/END` markers in
`results/perf-native-p1.time.txt`) and then returns a boolean anyway. The caller
gets `false`, not an exception.

The apparent 1.72× "speedup" at width 68 is not a speedup: that run returned 26
wrong answers, so it did less work. There is no width at which interleaving is
both faster and correct.

A reader comparing this axis to another engine's must not assume
apples-to-apples. A worker here is a whole Node process carrying a V8 heap **and
`ts-node`'s TypeScript compiler** (~367 MB resident before any load) — harness
weight, not engine weight.

### Method

Levels **1, 2, 4, 8, 16, 22, 32, 44**; **10 s** nominal step; **3 full ramp
repeats**; 2 warmup passes per worker before the host will time it; spawns
staggered 150 ms (pool of 44 ready in 9.9–13.1 s). The pool is spawned once per
ramp repeat at the maximum level; at each level only the first `c` workers are
told to go and the rest block on their IPC channel — resident, zero CPU. Per
step the host samples `/proc/<pid>/status` `VmRSS` and `/proc/<pid>/stat`
`utime+stime` for the **active** workers every **250 ms**, so RSS and CPU are
kernel-reported, not self-reported. Steady-state RSS is the median of samples
after the first second. A memory guard skips any step with `MemAvailable` below
`MIN_FREE_MB=8192`; **no step was skipped** (`MemAvailable` never fell below
39.7 GB). The timed unit is the engine-only path, so the c=1 row is directly
comparable to §2's median.

Latency percentiles are pooled from each worker's **uniform random** subsample
of 4,000 (slug, ms) pairs, cross-checked against the median of every worker's
*exact* stats over all of its samples — the `[exact …]` figures in
`results/load.stdout.txt` agree with the pooled ones throughout. `L-*` columns
repeat the stats with the three big-policy fixtures removed: a second view, not
an exclusion.

> The random sampling is load-bearing, and a first version of `load_worker.ts`
> got it wrong in a way worth recording. It shipped every *k*-th latency with
> *k* = ⌈n/2000⌉ ≈ 17. The corpus is cycled round-robin with period 68 and
> gcd(17, 68) = 17, so the "subsample" only ever landed on 4 of the 68 fixtures
> and never once on `testcase-062/063/064-big-policy` — the entire tail. It
> reported a c=1 p99 of **0.1 ms** where the single-process measurement on the
> identical corpus says **5.1 ms**. The disagreement between the two is what
> caught it. After the fix, c=1 pooled p99 is 5.294 ms against §2's 5.035 ms.

### Results — `results/load.json`

Medians across the 3 ramp repeats. Latency ms, RSS GiB. `L-` columns exclude the
three big-policy fixtures.

| c | tput/s | per-worker | med | p95 | p99 | L-med | L-p95 | L-p99 | cores | RSS steady | err | rel.range |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 1 | 3,358 | 3,358 | 0.043 | 0.216 | 5.294 | 0.042 | 0.081 | 0.187 | 1.36 | 0.85 | 0 | 0.032 |
| 2 | 6,393 | 3,196 | 0.044 | 0.181 | 5.701 | 0.043 | 0.078 | 0.178 | 2.68 | 1.70 | 0 | 0.042 |
| 4 | 10,363 | 2,591 | 0.051 | 0.245 | 7.540 | 0.050 | 0.095 | 0.227 | 5.27 | 3.92 | 0 | 0.038 |
| 8 | 13,942 | 1,743 | 0.068 | 0.325 | 14.689 | 0.067 | 0.161 | 0.286 | 10.32 | 7.07 | 0 | 0.089 |
| 16 | 16,149 | 1,009 | 0.122 | 0.585 | 20.700 | 0.119 | 0.277 | 0.535 | 18.20 | 13.64 | 0 | 0.010 |
| **22** | **17,223** | 783 | 0.137 | 3.283 | 29.522 | 0.133 | 0.312 | 1.967 | 20.90 | 18.36 | 0 | 0.023 |
| 32 | 16,572 | 518 | 0.149 | 6.287 | 47.842 | 0.145 | 0.492 | 4.026 | 21.29 | 24.34 | 0 | 0.017 |
| 44 | 16,187 | 368 | 0.153 | 9.267 | 64.511 | 0.149 | 1.508 | 6.626 | 21.00 | 32.05 | 0 | 0.011 |

**Throughput peaks at c=22 (17,223 eval/s) and degrades from there** — 16,572 at
c=32, 16,187 at c=44, a 6.0% fall for double the concurrency. c=22 is `nproc`,
and busy cores pin at **20.9–21.3 of 22** from c=22 onward: CPU is the binding
constraint, and it binds exactly where the core count says it should. Memory
never binds — 32.05 GiB at c=44 against 71 GiB available.

Latency is the real cost of pushing past the knee. Median rises 0.043 → 0.153 ms
(**3.6×**) from c=1 to c=44 for **6% less** throughput than c=22, and the tail is
far worse: p95 0.216 → 9.267 ms (**43×**), p99 5.294 → 64.511 ms (**12×**). The
`L-` columns show this is genuine queueing rather than corpus composition —
L-p95 goes 0.081 → 1.508 ms (**19×**) with the big-policy fixtures removed
entirely.

Scaling is sublinear well before saturation: per-worker throughput is already
down to 1,743/s at c=8 with only 10.3 cores busy. That is the machine's hybrid
core topology (performance cores first, then E-cores at lower clock), not the
engine.

Per-worker steady RSS is **0.75–1.0 GiB** at every level (`rss_per_worker_steady_kb`),
against ~367 MB after warmup — the growth is V8's heap under a sustained
allocation loop, and it *shrinks* slightly at the highest levels (764 MB/worker
at c=44 vs 888 MB at c=1) as the collector runs more often under pressure.

### The ceiling, and why it is 44

44 = **2× `nproc`**, chosen because it is comfortably past the observed
degradation point rather than in place of finding one: throughput turned over at
c=22, CPU was pinned at ~95% of 22 threads for three consecutive levels, and
memory was nowhere near binding. Going further would only have deepened the
queue. It was not a memory ceiling — at c=44 the pool held 32 GiB with 39.7 GB
still available.

### Errors under load

**Zero exceptions, at every level, in every repeat: 3,071,373 evaluations across
24 timed steps, error rate 0.000000 throughout.** The separate `mismatch`
counter is not an error rate — its baseline is the conformance result itself (7
of 68 in native mode) — and the set of distinct mismatching fixtures across the
entire ramp is **exactly those 7**: `testcase-009`, `-016`, `-025`, `-052`,
`-053`, `-055`, `-059`, identical to `verify-conformance-native.stdout.txt`'s
own failing list. **Load produced no new wrong answers.**

That is worth stating alongside §5's async result: multi-process concurrency is
correct on this engine at 3 million evaluations, and in-process concurrency is
wrong at width 2. The difference is entirely the process-global `EntityRegistry`.

## 6. Outlier and stability gates

Three stated numeric rules, constants in `perf_corpus.ts`. Nothing they catch is
discarded; it is reported and marked.

1. **Per-measurement, Tukey fence k = 1.5** (`TUKEY_K`). A measurement is flagged
   when it falls outside `[Q1 − 1.5·IQR, Q3 + 1.5·IQR]` of that run's own pooled
   distribution.
2. **Per-case cross-repeat instability** (`CASE_INSTABILITY = 0.25`). A case is
   flagged `unstable` when `(max − min) / median` over its 20 measurements
   exceeds 0.25.
3. **Per-level ramp instability** (`LEVEL_INSTABILITY = 0.15`). A level is
   flagged `unstable_across_repeats` when its throughput `(max − min) / median`
   across the 3 ramp repeats exceeds 0.15.

| gate | engine-only (native p1) | end-to-end (native p1) | load ramp |
|---|---|---|---|
| rule 1 fence | `[−0.003, 0.098]` ms | `[0.058, 0.262]` ms | — |
| rule 1 flagged | **108 / 1360** | **124 / 1360** | — |
| rule 2 unstable cases | **68 / 68** (worst 7.83) | **54 / 68** (worst 1.82) | — |
| rule 3 unstable levels | — | — | **0 / 8** (worst 0.089 at c=8) |

Reading each row honestly:

- **Rule 1's 108 flags are the corpus's own bimodality, not noise.** All 60
  big-policy measurements are flagged (20 each for 062/063/064), plus all 20 of
  `testcase-065-alice` (0.202 ms median) and 5 of `testcase-067-alice-past`;
  the remaining 3 are single GC blips. A fence built on a distribution whose
  IQR is ~0.03 ms will flag anything from the 4.6 ms mode by construction. The
  flags are correct and uninformative — which is why §2 reports the light/heavy
  split explicitly rather than relying on this gate to convey it.
- **Rule 2 flags 68 of 68 cases, and that is the gate failing, not the engine.**
  A median of 0.047 ms with a 20-sample range makes `(max − min)/median > 0.25`
  the *expected* outcome of a single 2 ms GC pause anywhere in the 20 repeats;
  the worst case (`testcase-026`, 7.83) is one 0.24 ms sample against a 0.030 ms
  median. At tens of microseconds this rule measures the garbage collector's
  duty cycle. Reported as flagged, and paired here with the aggregate stability
  that actually matters:

  | statistic | p1 | p2 | p3 | rel. range |
  |---|--:|--:|--:|--:|
  | engine-only mean | 0.292 | 0.289 | 0.303 | **0.048** |
  | engine-only median | 0.047 | 0.046 | 0.049 | **0.064** |
  | engine-only p99 | 5.035 | 4.953 | 5.478 | **0.104** |
  | end-to-end mean | 0.983 | 0.984 | 0.996 | **0.013** |
  | end-to-end median | 0.159 | 0.161 | 0.163 | **0.025** |
  | end-to-end p99 | 18.662 | 19.092 | 18.917 | **0.023** |

  Three independent processes agree on every aggregate to within 1.3–10.4%,
  and the 20 in-process repeat walls span 15.6–28.8 ms (rel. range 0.74) —
  itself a GC artifact, since the *end-to-end* walls, where each repeat is 4×
  longer and GC is amortised, span only 64.5–84.4 ms (rel. range 0.30).
- **The load ramp is the most stable measurement in the pass**: worst throughput
  relative range **0.089** at c=8, everything else ≤0.042, no level flagged.

**One individually flagged measurement, reported not dropped**: the assisted-mode
async probe's width-8 sequential leg read **8.251 ms** where p1/p2/p3 read
0.347/0.334/0.346 ms — a 24× excursion, plainly one GC pause landing inside a
0.3 ms window. It is left in `results/perf-assisted.json` and marked here; it
affects only that one probe row, and the corresponding `Promise.all` leg
(1.263 ms) and wrong-answer count (4) are in line with the other three runs.

## What was not measured, and why

- **An HTTP/service axis.** There isn't one. odrl-manager is a library with no
  server, so there is no request-rate dimension to ramp — unlike, say, an engine
  whose conformance number came from a REST call.
- **The built `dist/` bundle as the timed path.** The conformance number comes
  from `src/` via `ts-node`; timing the bundle would measure a different code
  path and would not be comparable to 61/68.
- **In-process async concurrency as a load axis.** Measured (§5) and rejected on
  correctness: it returns wrong answers from width 2 upward.
- **Tuned V8 flags** (`--max-semi-space-size`, `--max-old-space-size`). Given how
  much of the tail here is GC, a tuned heap would likely flatten the rule-2
  flags the way disabling the collector would. Not attempted; every number above
  is stock Node v24.18.0.
- **Nothing was infeasible for this engine.** Unlike `odrl-pap`, odrl-manager
  needs no external infrastructure — clone, `npm install`, run — so every one of
  the five dimensions was measured for real.

## Files added by this pass

Scripts, beside the existing conformance harness:

- `perf_corpus.ts` — `run.ts`'s fixture reading, translation, host fetchers and
  engine call as a shared module (`MODE` becomes a parameter), plus the two
  timed paths, the percentile/IQR helpers, the stated gate constants, and
  `/proc` readers for RSS and CPU. No new evaluation logic.
- `perf_bench.ts` — startup/import cost, warmup and smoke test, per-case latency
  over 20 repeats on both paths, the in-process RSS series and CPU accounting,
  the gates, and the async-concurrency correctness probe. Single process.
  `--no-raw-samples` keeps everything derived but omits the per-evaluation
  records, for the soak run only.
- `load_worker.ts` — one load-generator process; IPC line protocol, exact
  per-fixture stats plus a uniform random subsample.
- `load_bench.ts` — the ramp host: spawns the pool, walks the levels, samples
  `/proc` for active workers, enforces the memory guard, aggregates, and kills
  its pool on every exit path including `SIGINT`/`SIGTERM`/`uncaughtException`.
- `run_perf.sh` — the driver that produced everything in `results/`, in order.

Raw results in `results/` (nothing here overwrites a conformance artifact; the
conformance harness's own `OUT` files are the `verify-conformance-*` pair):

| file | what |
|---|---|
| `environment.txt` | pre-run `uptime`, `free -h`, `nproc`, `lscpu`, versions, `ps` |
| `pinned-commit.txt` | the commit actually measured (`8842b6b…`, dated 2025-01-20) |
| `clone.time.txt`, `npm-install-*.time.txt`/`.log`, `build.time.txt`/`.log` | `/usr/bin/time -v` for every setup step |
| `footprint.txt` | `du -sh` breakdown, largest packages, `npm ls --depth=0` |
| `verify-conformance-native.*`, `verify-conformance-assisted.*` | the conformance harness re-run on this checkout: 61/68 and 67/68 |
| `perf-native-p1/p2/p3.json` + `.stdout.txt` + `.time.txt` | the three native perf processes; `.time.txt` also holds the fenced async-probe stderr |
| `perf-native-soak.json` + `.stdout.txt` + `.time.txt` | the 400-repeat / 27,200-evaluation soak that settles the RSS-growth question |
| `perf-assisted.json` + `.stdout.txt` + `.time.txt` | assisted-mode perf run |
| `load.json` (88 KB), `load.stdout.txt`, `load.time.txt` | the ramp: per-level pooled and exact latency, `/proc` RSS/CPU, per-worker rows |
| `sweep.txt` | post-run straggler count and machine state |

## Cleanup

`load_bench.ts` kills its pool after every ramp repeat, in a `finally`-equivalent
path and on `exit`/`SIGINT`/`SIGTERM`/`uncaughtException`; each worker also exits
on IPC `disconnect` so it can never outlive its host. `run_perf.sh` sweeps as its
last step and records the result. After the final run `sweep.txt` reports **0**
straggler processes, and an independent
`ps -eo pid,comm,args | awk '$2=="node" && /load_worker|load_bench|perf_bench/'`
afterwards returned nothing. No process, server or load generator was left
running.
