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

The perf-instrumentation files added later (`bench/perf-corpus.ts`,
`bench/perf-bench.ts`, `bench/load-bench.ts`, `bench/load-worker.ts`,
`bench/run-perf.sh`, and everything in `results/` whose name is not
`allow-deny-*`) are described in their own section below.

## Reproduced

Verified with a fresh `git clone` + `npm install` in an isolated scratch
location: **63/68** at `0.4.0`, exact match to the committed result.

Re-verified again on 2026-09-06 as step 2 of the perf pass below, from a
fresh clone at suite commit `7958238e`, at both pins:
`results/verify-conformance-040.json` is **63/5/0** and byte-identical
per-case to the committed `results/allow-deny-results.json`;
`results/verify-conformance-060.json` is **67/1/0**. The perf runs then
re-checked every timed evaluation against the fixture's expected decision and
got 63/5/0 and 67/1/0 in every one of their five repeats, so the latency
numbers below were produced by an engine that was still answering exactly as
the conformance table says it does.

---

# Performance, resources and load

Added 2026-09-06. Everything in this section is a real measurement taken on
this machine, in one sequential pass with nothing else benchmarking
alongside; every number points at a file in `results/`. Both pins the
conformance bench reports — `0.4.0` (the suite's own) and `0.6.0` — were
measured, separately and identically.

## What this engine actually is, physically

`odrl-evaluator` is an npm package, not a service: there is nothing to start,
no port, no database. But it is not plain JavaScript either. It delegates to
`eyereasoner` (16.34.1 in this tree), which ships SWI-Prolog compiled to
WebAssembly (`swipl-wasm` 4.0.13, `swipl-web.wasm`, **2.09 MB** at the `0.4.0`
tree / **2.15 MB** at `0.6.0`). That single fact explains most of what
follows: the ~0.5 GB resident set of an idle worker, the absence of any
intra-process parallelism, and why an evaluation cannot be interrupted once
it has started.

## Environment, and the baseline load

Recorded by `run-perf.sh` step 0 *before* anything heavy ran, into
`results/environment-040.txt` and `results/environment-060.txt`.

| | |
|---|---|
| Kernel | Linux 6.8.0-138-generic x86_64 |
| `nproc` | 22 |
| `free -h` total / available | 93 GiB / 72 GiB, **no swap configured** |
| Node | v24.18.0 (nvm) |
| npm | 12.0.1 |

Baseline before the `0.4.0` pass (09:12:33 local): `load average: 0.87, 0.73,
1.30`, 72 GiB available. Baseline before the `0.6.0` pass (10:01:40):
`load average: 10.06, 11.28, 8.70`, 71 GiB available — that 1-minute figure
is **residual decay from this pass's own `0.4.0` load ramp, which had ended
13 seconds earlier**, not competing work: the `ps` listing captured in the
same file at the same instant shows no benchmark process at all, only the
same idle desktop processes (`firefox-bin`, `Xorg`, `cinnamon`) present in
the `0.4.0` baseline. The load ramps' own
three-repeat spread is the check that matters here, and it is reported under
the stability gate below: no concurrency level on either version varied by
more than 14.3% across repeats, well inside the 15% gate.

## The invocation path, and why it is the conformance harness's

`bench/perf-corpus.ts` is `allow-deny-bench.ts`'s case selection, source-URL
rewriting and `ground_truth.rs` Allow/Deny reduction lifted verbatim into a
module. Every timed evaluation in this section is the same three-step region
the conformance harness puts its own `ms` around:

```
parseFile(policy) + parseFile(request) + parseFile(sotw)
  -> ODRLEvaluator(new ODRLEngineMultipleSteps()).evaluate(...)
  -> reduceToDecision(report)
```

on the same 68 cases, in the same order, with one evaluator instance reused
for the process — exactly as `allow-deny-bench.ts` does it. The conformance
harness itself was not modified; it keeps its own inline copy of those rules
and still runs standalone as documented above.

A second, secondary path (`"engine"`) times the same thing with the Turtle
already parsed, to separate the reasoner from n3.

### Reproducing the whole pass

```sh
VERSION=0.4.0 \
SCRATCH=/some/isolated/dir \
RESULTS=<this-directory>/results \
BENCH_SRC=<this-directory>/bench \
bash bench/run-perf.sh          # then again with VERSION=0.6.0
```

`run-perf.sh` records the environment, clones, installs the pin, re-runs the
**conformance** harness first, then runs `perf-bench.ts` and `load-bench.ts`
under `/usr/bin/time -v`, measures the footprint, and sweeps any straggler
process. It runs nothing in the background.

## 1. Warmup and smoke test

**10 discarded iterations**, cycling testcases 001–010 of the same corpus,
before any timed measurement. It doubles as the smoke test: an `ERROR` in
warmup aborts the run rather than letting a broken engine produce numbers.
Both versions completed warmup cleanly. Raw: `warmup_ms` in
`results/perf-0*.json`.

| version | warmup latencies, ms (in order) |
|---|---|
| 0.4.0 | 614.9, 640.1, 624.5, 664.1, 658.2, 658.0, 658.2, 662.1, 659.6, 656.0 |
| 0.6.0 | **454.6**, 297.4, 280.7, 303.5, 281.7, 286.3, 274.7, 307.6, 295.5, 275.8 |

`0.6.0` shows a real cold-start cost — the first evaluation is **1.6×** the
steady value — so discarding it matters. `0.4.0` shows none worth speaking of
(its first call is actually its *fastest*): at 0.65 s per evaluation the WASM
instantiation is lost in the noise. Each load worker does its own warmup
(default 5 evaluations) before the host will time it.

## 2. Per-case latency, post-warmup

Full 68-case corpus, **5 repeats** of the conformance path (340 measurements)
and 2 of the pre-parsed path (136). Per-case, not just totals; every
measurement is in `samples[]` and every case's own spread in
`per_case_full[]` of `results/perf-0*.json`.

| ms | 0.4.0 | 0.6.0 |
|---|---:|---:|
| n | 340 | 340 |
| mean | 1267.91 | 564.92 |
| **median** | **706.59** | **296.54** |
| p95 | 797.57 | 342.44 |
| p99 | 13759.27 | 6471.42 |
| min | 612.29 | 257.36 |
| max | 13962.69 | 6518.07 |
| stddev | 2632.61 | 1249.65 |
| IQR | 73.17 | 28.47 |

**0.6.0 is 2.38× faster at the median** and 2.13× at p99 — on identical
inputs, on the same machine, in the same hour. A whole 68-case pass takes
86.6 s at `0.4.0` and 38.5 s at `0.6.0` (`per_repeat[].wall_ms`).

The mean sitting at ~1.8× the median, and p99 at ~19× it, are not noise. The
distribution is sharply **bimodal**: 65 cases sit in a tight band, three do
not.

| | 0.4.0 | 0.6.0 |
|---|---:|---:|
| 65 "light" cases, per-case median range | 629.0 – 782.2 ms | 265.4 – 331.0 ms |
| `testcase-062-big-policy` | 13390.3 ms | 6298.0 ms |
| `testcase-063-big-policy-OoO` | 13463.6 ms | 6436.1 ms |
| `testcase-064-big-policy-past` | 13599.6 ms | 6449.3 ms |
| heavy/median ratio | ~19× | ~22× |

Three fixtures out of 68 therefore account for ~47% of the corpus's total
evaluation time at `0.4.0` (40.4 s of an 86.6 s pass) and ~50% at `0.6.0`
(19.2 s of 38.5 s). This is a property of the engine on this corpus, not a
measurement artefact — it reproduced in all five repeats of both versions,
with a per-case relative range under 6%.

Turtle parsing is **not** where the time goes: the median gap between the
conformance path and the pre-parsed path is 15.2 ms at `0.4.0` (2.2% of the
median) and 1.3 ms at `0.6.0` (0.4%).

## 3. System requirements

No JVM, no database, no container, no service to stand up. Node and npm only.
Raw: `results/clone.time.txt`, `results/npm-install-cold.time.txt`,
`results/npm-install-0*.time.txt`, `results/footprint-0*.txt`.

| | measured |
|---|---|
| Runtime actually used | `node --version` → **v24.18.0**; `npm --version` → **12.0.1** |
| `git clone` | **1.62 s** wall, 21 MB peak RSS |
| Build step | there is one, inside the install: the suite's `prepare` script runs `npm run build` (`tsc` + `createIndex`) |
| **Cold `npm install`** (fresh clone, `npm cache clean --force` first) | **22.55 s** wall — 13.70 s user + 2.34 s sys, 71% CPU, **618.8 MiB** peak RSS, npm reported "added 498 packages" |
| Switching the pin in an existing tree (`npm install odrl-evaluator@0.6.0`) | **9.62 s** wall, 293.3 MiB peak RSS |

On-disk footprint (`du -sh`), which differs by pin because `0.6.0` pulls a
different dependency closure:

| | 0.4.0 | 0.6.0 |
|---|---:|---:|
| whole checkout incl. `.git` + `node_modules` | 150 MB | 184 MB |
| `node_modules` | 142 MB | 175 MB |
| entries in `node_modules/` | 310 | 416 |
| `odrl-evaluator` itself (`dist/`) | 648 KB (624 KB) | 828 KB (804 KB) |
| `swipl-web.wasm` — the actual reasoner | 2,088,107 B | 2,150,992 B |

The engine you are benchmarking is 0.6 MB of TypeScript output on top of a
142–175 MB dependency tree whose single largest artefact is a 2 MB Prolog
WASM image.

## 4. Resource consumption during the performance run

Tool: **`/usr/bin/time -v`** wrapped around the whole `perf-bench.ts` process
by `run-perf.sh`, which reports `Maximum resident set size` and CPU seconds
directly from `wait4()`. Raw: `results/perf-0*.time.txt`. In addition
`perf-bench.ts` samples `process.memoryUsage().rss` after **every** timed
evaluation, giving steady-state and growth rather than just a peak
(`memory_in_process` in `results/perf-0*.json`).

| | 0.4.0 | 0.6.0 |
|---|---:|---:|
| Elapsed | 10:12.78 | 4:35.04 |
| User CPU | 663.50 s | 323.03 s |
| System CPU | 8.47 s | 8.46 s |
| Percent of CPU | **109%** | **120%** |
| **Peak RSS (`time -v`)** | **741.4 MiB** | **816.7 MiB** |
| Major page faults / swaps | 0 / 0 | 0 / 0 |
| Minor page faults | 6,048,777 | 6,433,679 |
| In-process RSS, first sample | 478 MiB | 503 MiB |
| In-process RSS, **steady (median)** | 658 MiB | 720 MiB |
| In-process RSS, last sample | 703 MiB | 723 MiB |
| Growth over the run | **+225 MiB** | **+220 MiB** |

Two things to read off this. First, "percent of CPU" barely above 100%
confirms a **single-threaded** reasoner — the extra 9–20% is V8's GC and
compiler threads, not evaluation. Second, resident memory **grows ~220 MiB
across the 476 timed evaluations of a single long-lived process** and does not come
back. For a batch job that is irrelevant; for a long-running service that
reuses one evaluator, it is the number to watch. It is reported here, not
diagnosed — nothing in this pass establishes whether it is a leak or a heap
that simply has not been pressured into collecting.

## 5. Load and peak behaviour

### What "concurrency" means for THIS engine — do not read it as apples-to-apples

**One unit of concurrency = one OS process.** `load-bench.ts` spawns
`node node_modules/.bin/ts-node bench/load-worker.ts`, each with its own
`ODRLEvaluator` and its own WASM reasoner instance, each looping the same
68-case corpus from a different offset.

That choice was **measured, not assumed**. `perf-bench.ts` runs an
`intra_process_concurrency_probe`: N `evaluate()` calls under `Promise.all`
against the same N run sequentially.

| version | 4 sequential | 4 under `Promise.all` | speedup |
|---|---:|---:|---:|
| 0.4.0 | 2501.5 ms | 2462.9 ms | **1.016×** |
| 0.6.0 | 1101.6 ms | 1095.5 ms | **1.006×** |

`evaluate()` is an async *signature* over a synchronous WASM run: it holds
Node's single thread for its whole duration, so awaiting several at once
interleaves nothing. In-process async concurrency for this engine is a fiction,
and a reader comparing this table to a thread-based or async-runtime
concurrency figure from another engine is comparing different things. A
process here carries a whole Node runtime plus its own reasoner heap
(~0.5 GB); a native thread does not.

### Ceiling, and why it is 28

Set by **memory, not cores**. At ~0.5 GiB resident per worker, 28 workers is
11.9 GiB (`0.4.0`) to 14.8 GiB (`0.6.0`) of active RSS, and because the pool
is spawned once per ramp repeat the whole 28 are resident even during the
`c=1` step (9.1 / 10.2 GiB). Going further on a shared 93 GiB workstation
with **no swap** risks the machine rather than the measurement. `load-bench.ts`
enforces this: it reads `MemAvailable` from `/proc/meminfo` before each pool
and each step and skips the step (recording why) below `MIN_FREE_MB=6144`.
No step was ever skipped — `MemAvailable` before the first step was 65.6 GiB
(`0.4.0`) and 63.3 GiB (`0.6.0`). CPU was not the binding constraint either:
busy cores plateau at 12.5 (`0.4.0`) and 16.1 (`0.6.0`) of 22.

### Method

Levels **1, 2, 4, 8, 16, 22, 28**; **10 s** nominal step; **3 full ramp
repeats**; 5 warmup evaluations per worker before the host will time it;
spawns staggered 150 ms. Per step the host samples `/proc/<pid>/status`
`VmRSS` and `/proc/<pid>/stat` `utime+stime` for every *active* worker every
**250 ms**, so RSS and CPU are kernel-reported, not self-reported. Inactive
workers sit blocked on stdin — memory held, no CPU. Steady-state RSS is the
median of the samples after the first second.

**Two throughput columns, and why.** An evaluation cannot be interrupted, so a
worker that has entered a big-policy case holds the step open past its
nominal end: at `c≥16` on `0.4.0` a 10 s step actually takes ~25 s.
`tput(wall)` divides completed evaluations by the step's **real** wall time —
the honest "drain a burst of N concurrent requests" figure. `tput(win)`
counts only evaluations that **completed inside the nominal 10 s** — the
honest steady-state "requests served per second" figure. Both are real; they
diverge exactly where that tail bites.

Likewise every step reports latency twice: over all evaluations, and over the
same window with testcases 062/063/064 removed (`L*` columns). Nothing is
excluded from the run — this is a second view, because a 10 s window at `c=1`
walks ~16 cheap cases and never reaches a heavy one, while a window at `c=28`
has several workers parked inside one for its whole duration. Without the
split, corpus composition is indistinguishable from contention.

### 0.4.0 — `results/load-040.json`

Medians across the 3 ramp repeats. Latencies in ms, RSS in GiB.

| c | tput(wall) | tput(win) | wall s | med | p95 | p99 | L-med | L-p95 | L-p99 | heavy% | RSS peak | RSS steady | cores | exc | rel.range |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 1 | 1.53 | 1.50 | 10.5 | 650 | 705 | 705 | 650 | 705 | 705 | 0.0 | 0.44 | 0.38 | 1.18 | 0 | 0.028 |
| 2 | 2.82 | 2.80 | 10.6 | 693 | 772 | 782 | 693 | 772 | 782 | 0.0 | 0.84 | 0.77 | 2.32 | 0 | 0.010 |
| 4 | 4.85 | 4.80 | 10.7 | 790 | 935 | 985 | 790 | 935 | 985 | 0.0 | 1.67 | 1.52 | 4.64 | 0 | 0.002 |
| 8 | **6.69** | 6.40 | 10.9 | 1097 | 1741 | 1843 | 1097 | 1741 | 1843 | 0.0 | 3.14 | 2.99 | 9.34 | 0 | 0.016 |
| 16 | 3.62 | **7.20** | 24.9 | 1879 | 2717 | 20597 | 1866 | 2288 | 2883 | 3.4 | 6.59 | 6.24 | 9.43 | 0 | 0.065 |
| 22 | 3.31 | 6.50 | 26.6 | 2579 | 17199 | 24111 | 2545 | 3235 | 3510 | 5.3 | 9.47 | 9.03 | 11.58 | 0 | 0.143 |
| 28 | 3.33 | 5.70 | 25.8 | 3276 | 24443 | 25820 | 3246 | 4675 | 5178 | 5.7 | 11.92 | 11.29 | 12.45 | 0 | 0.030 |

**`0.4.0` has a real degradation point.** End-to-end throughput peaks at
**c=8 (6.69 eval/s)** and then falls by half, to 3.33 at c=28; even the
window-only figure peaks at c=16 and declines after. Busy cores stop growing
at ~9.4 while 22 are available — the box is not saturated, the engine is.
Median latency rises 5.0× from c=1 to c=28.

### 0.6.0 — `results/load-060.json`

| c | tput(wall) | tput(win) | wall s | med | p95 | p99 | L-med | L-p95 | L-p99 | heavy% | RSS peak | RSS steady | cores | exc | rel.range |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 1 | 3.36 | 3.30 | 10.2 | 296 | 327 | 334 | 296 | 327 | 334 | 0.0 | 0.49 | 0.42 | 1.36 | 0 | 0.016 |
| 2 | 3.91 | 5.70 | 15.4 | 322 | 358 | 6682 | 321 | 356 | 367 | 1.7 | 1.15 | 0.90 | 2.12 | 0 | 0.032 |
| 4 | 5.50 | 7.70 | 14.9 | 359 | 426 | 7511 | 356 | 419 | 430 | 3.7 | 2.24 | 2.07 | 4.15 | 0 | 0.058 |
| 8 | 7.49 | 11.90 | 16.9 | 493 | 704 | 8740 | 488 | 684 | 717 | 3.4 | 4.47 | 4.19 | 7.59 | 0 | 0.064 |
| 16 | 7.89 | 11.10 | 16.6 | 798 | 11705 | 14432 | 793 | 1018 | 1235 | 6.5 | 9.03 | 8.61 | 13.94 | 0 | 0.119 |
| 22 | 8.12 | 11.40 | 17.1 | 1165 | 14759 | 16340 | 1152 | 1550 | 1714 | 5.7 | 11.78 | 11.42 | 16.11 | 0 | 0.047 |
| 28 | **9.02** | **12.70** | 17.9 | 1509 | 2479 | 16906 | 1495 | 2107 | 2330 | 4.4 | 14.78 | 14.28 | 15.86 | 0 | 0.056 |

**`0.6.0` does not collapse.** End-to-end throughput rises monotonically to
9.02 eval/s at c=28 and had not turned over at the ceiling this box can
safely hold; busy cores reach 16.1 of 22. Median latency rises 5.1× from c=1
to c=28 — the same relative degradation as `0.4.0`, from a 2.2× better
starting point and without the throughput cliff.

### Errors under load

**Zero exceptions**, at every level, in every repeat, on both versions —
42 timed steps, 0 thrown evaluations, error rate 0.000 throughout.

The separate `mismatch` counter (engine's decision ≠ fixture's expected one)
is *not* an under-load error rate, and the raw data proves it. Its baseline
is the conformance result itself: 5 of 68 at `0.4.0`, 1 of 68 at `0.6.0`.
Because a short window walks only part of the corpus, the observed mismatch
share moves around that baseline (it reaches 0.143 at `0.4.0` c=22, where
workers happen to be sampling the 51–68 region that contains all five known
failures). Checking every one of the 42 steps against the per-evaluation
testcase labels stored in `per_worker[]`: **each step's mismatch count is
exactly the number of known-conformance-failing fixtures it sampled**
(`{51,53,55,61,65}` at `0.4.0`, `{61}` at `0.6.0`), with no exceptions. Load
produced no new wrong answers on either version.

## 6. Outlier and stability gates

Three stated numeric rules. Nothing they catch is discarded; it is reported
and marked.

1. **Per-measurement, Tukey fence k = 1.5.** A single case-measurement is
   flagged when it falls outside `[Q1 − 1.5·IQR, Q3 + 1.5·IQR]` of that
   path's own pooled distribution.
2. **Per-case cross-repeat instability.** A case is flagged `unstable` when
   `(max − min) / median` over its 5 full-path measurements exceeds **0.25**.
3. **Per-level ramp instability.** A concurrency level is flagged
   `unstable_across_repeats` when its throughput `(max − min) / median`
   across the 3 ramp repeats exceeds **0.15**.

Results:

| gate | 0.4.0 | 0.6.0 |
|---|---|---|
| Tukey fence | `[556.7, 849.4]` ms; **15 of 340 flagged** | `[240.3, 354.2]` ms; **15 of 340 flagged** |
| what got flagged | all 15 are the 5 repeats × 3 big-policy cases; no light case ever crossed the fence | identical |
| unstable cases (rule 2) | **0 of 68** (worst light case 0.191; worst heavy case 0.054) | **0 of 68** (worst light 0.171, worst heavy 0.052) |
| unstable levels (rule 3) | **0 of 7** (worst 0.143 at c=22) | **0 of 7** (worst 0.119 at c=16) |

The honest reading: the Tukey fence fires 30 times across both versions and
every single firing is the *same three fixtures*, reproducibly, in both
directions of the version bump. These are flagged as statistical outliers and
they are **not** unstable — a big-policy case is a genuinely, repeatably
expensive evaluation, not a noisy one. No measurement in this pass had to be
reported as untrustworthy.

## What was not measured, and why

- **A server or request-rate axis.** There is no service to rate-limit
  against: this is a library. Concurrency is the only load axis that exists
  for it, so no requests/second ramp was attempted.
- **Concurrency above 28.** Refused on memory grounds on a shared,
  swapless box — see the ceiling justification above. `0.6.0` had not turned
  over there, so its true peak is above what this machine can safely show.
- **Whether the ~220 MiB in-process RSS growth is a leak.** Observed and
  reported; not diagnosed.
- **A cold `npm install` per version.** The cold-install figure
  (`npm-install-cold.time.txt`) was taken once, in a pristine clone with the
  npm cache cleared, at the suite's own `^0.4.0` pin. The `0.6.0` figure
  (9.62 s) is a warm re-pin of an existing tree, and is labelled as such.

## Files added by this pass

Scripts, in `bench/`:

- `perf-corpus.ts` — the conformance harness's corpus loading, URL rewriting
  and Allow/Deny reduction as a shared module, plus the percentile/IQR
  helpers. Nothing in it is new logic.
- `perf-bench.ts` — warmup, per-case latency over both paths, the in-process
  RSS series, and the intra-process concurrency probe. Single process.
- `load-worker.ts` — one load-generator process. Line-protocol over
  stdin/stdout; reports per-evaluation latency **with the testcase it
  belongs to**.
- `load-bench.ts` — the ramp host: spawns the pool, walks the levels, samples
  `/proc` for the active workers, applies the memory guard, aggregates. Every
  read from a worker is bounded by a timeout, so a wedged child fails the run
  with a diagnosis instead of hanging it.
- `run-perf.sh` — the driver that produced everything in `results/`, in order,
  including the pre-run environment capture and the post-run process sweep.

Raw results, in `results/` (nothing here overwrites the conformance
`allow-deny-*.json`):

| file | what |
|---|---|
| `environment-0*.txt` | pre-run `uptime`, `free -h`, `nproc`, versions, `ps` |
| `suite-commit.txt` | the suite commit measured (`7958238e`) |
| `clone.time.txt`, `npm-install-cold.time.txt`, `npm-install-0*.time.txt` | `/usr/bin/time -v` for setup |
| `installed-version-0*.txt`, `footprint-0*.txt` | pin actually loaded; `du -sh` breakdown |
| `verify-conformance-0*.json` | the conformance harness re-run on this checkout |
| `perf-0*.json`, `perf-0*.time.txt` | per-case latency + `time -v` resources |
| `load-0*.json`, `load-0*.time.txt` | the ramp, incl. raw per-worker latency/testcase arrays |

## Cleanup

`load-bench.ts` kills its pool after every ramp repeat and on `SIGINT`,
`SIGTERM` or an uncaught exception; `run-perf.sh` sweeps stragglers as its
last step. After the final run, `ps -eo pid,args | grep -c '[l]oad-worker.ts'`
returned **0** and no benchmark process of any kind was left behind.
