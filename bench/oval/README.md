# Bench: OVAL (`DIPS-Tools/odrl-Engine`)

Python, RDF-reasoning-based ODRL evaluator. Pinned commit:
`a427e71b50bdd14027f2d5552d6ce03d089487f3`.

## Setup

```sh
git clone https://github.com/DIPS-Tools/odrl-Engine.git odrl-Engine
cd odrl-Engine
git checkout a427e71b50bdd14027f2d5552d6ce03d089487f3
python3 -m venv venv
./venv/bin/pip install -r requirements.txt
cd ..
cp <this-directory>/*.py .        # ground_truth.py, bench.py, probes*.py, retranslate.py --
                                   # placed BESIDE the odrl-Engine/ clone, not inside it
```

## Run

```sh
# 1. Generate the shared ground truth this bench and ds-odrl-engine-rs's own
#    compliance-runner are checked against, re-derived independently from the
#    fixtures' own report:* graphs (not copied from anywhere).
python3 ground_truth.py
#   -> ground_truth.json (68 cases, Allow=27, Deny=41)

# 2. Run the bench against OVAL's own upstream-committed, pre-translated
#    corpus (extracted_<slug>.ttl / .csv pairs already shipped in the clone).
./venv/bin/python bench.py odrl-Engine/test_cases/evaluation/force results.json --isolate
```

`corpus_dir` must point at a directory of `extracted_<slug>.ttl`/`.csv` pairs
in OVAL's own pre-translated format — **not** the raw ODRL-Test-Suite
fixtures. `odrl-Engine/test_cases/evaluation/force` (upstream's own committed
corpus) is the simplest choice; `retranslate.py` (below) produces an
independent one from the local vendored suite instead, to prove the result
isn't an artifact of OVAL's own possibly-stale committed translation.

```sh
# Optional: retranslate the LOCAL vendored corpus independently, to prove
# the result above isn't just OVAL's own committed corpus agreeing with itself.
git clone https://github.com/SolidLabResearch/ODRL-Test-Suite.git   # beside odrl-Engine/
python3 retranslate.py
./venv/bin/python bench.py retranslated/test_cases/evaluation/force results_retranslated.json --isolate
```

`--isolate` passes fresh `ontology_files`/`ontology_graphs` lists per case;
omit it to reproduce upstream's own `test_on_force.py` conditions verbatim
(see `bench.py`'s own header comment for the real, found-the-hard-way reason
this flag exists — a mutable-default-argument bug that leaks state across a
loop in the same process).

## What's here

- `ground_truth.py` — independent Allow/Deny re-derivation from the vendored
  suite's own `report:*` graphs, matching `ground_truth.rs` exactly.
- `bench.py` — the harness. Its own header comment documents two real,
  found-the-hard-way environment facts about upstream `ODRL_Evaluator.py`:
  a cwd-relative ontology path (`ODRL/ODRL22.ttl`) that silently breaks
  action-taxonomy reasoning from the wrong working directory, and the
  mutable-default-argument state leak `--isolate` exists to defeat.
- `probes.py`, `probes2.py` — vocabulary/capability probes.
- `retranslate.py` — re-translates the LOCAL vendored corpus into OVAL's own
  input format independently of its committed one, calling upstream's real
  `parse_test_cases_from_md` directly (its own header documents a genuine
  upstream bug: `FORCE_translator.py`'s `__main__` block calls the
  *reverse*-direction function and dies on the suite's own documentation
  files if invoked as its own README suggests).
- `results/results_A_upstream.json`, `results/results_B_isolated.json` — **59
  pass** out of 68, both with and without `--isolate` engaged.

The perf-instrumentation files added later (`perf_corpus.py`, `perf_bench.py`,
`load_bench.py`, `load_worker.py`, `run_perf.sh`, and everything in `results/`
whose name is not `results_[AB]_*.json`) are described in their own section
below.

## Reproduced

Verified with a fresh `git clone` + venv install in an isolated scratch
location, run against upstream's own committed corpus: **59/68**, exact match
to the committed result.

Re-verified again on 2026-09-06 as step 5 of the perf pass below, from a fresh
clone at `a427e71` in an isolated scratch directory: `results/verify-conformance-A.json`
and `results/verify-conformance-B.json` are both **59 PASS / 9 FAIL / 0 ERROR**,
per-case identical (slug, status, verdict, raw validity) to the committed
`results_A_upstream.json` / `results_B_isolated.json`. Every timed run below
also re-checked each evaluation against the fixture's expected decision and got
exactly 9 mismatches in every one of its five repeats, and the load ramp got
zero mismatches outside the same nine fixtures across 19,017 evaluations — so
the latency numbers were produced by an engine still answering the way the
conformance table says it does.

---

# Performance, resources and load

Added 2026-09-06. Every number in this section is a real measurement taken on
this machine in one sequential pass, with no other engine's benchmark running
alongside, and each points at a file in `results/`. Nothing is carried over
from the conformance-only run above.

## What this engine physically is

A Python library. No service to start, no port, no database, no compile step —
`import ODRL_Evaluator` and call a function. The evaluation itself is RDF work:
`rdflib` parsing plus `pyshacl`/`owlrl` reasoning over the policy graph, the
State-of-the-World, and the ODRL 2.2 ontology, per call. Everything below
follows from that: a ~117 MiB resident interpreter, exactly one core per
process, and a GIL that makes threads useless for it.

Upstream does ship `api/main.py` (FastAPI) and a `Dockerfile`, so an HTTP axis
exists in principle. It was deliberately **not** measured: the 59/68 conformance
number came from the library entrypoint, and timing a different code path would
not be comparable to it.

## Environment, and the baseline load

Captured by `run_perf.sh` step 0 *before* anything heavy ran →
`results/environment.txt`.

| | |
|---|---|
| Kernel | Linux 6.8.0-138-generic x86_64 |
| CPU | Intel Core Ultra 9 185H — 16 physical cores / 22 threads, hybrid (4 threads @ 5.1 GHz, 8 @ 4.8, 8 E-cores @ 3.8, 2 LP-E @ 2.5) |
| `nproc` | 22 |
| `free -h` total / available | 93 GiB / 71 GiB, **no swap configured** |
| `MemAvailable` at start | 75,094,624 kB (71.6 GiB) |
| Python actually used | `python3 -VV` → **3.12.3 (main, Jul 15 2026) [GCC 13.3.0]** |
| git | 2.43.0 |

Baseline at 10:54:10 local: `load average: 1.37, 6.14, 5.45`. The 5- and
15-minute figures are **residual decay from this same pass's own earlier,
discarded driver run, which ended at 10:51:09** — not competing work. The `ps`
listing captured in the same file at the same instant contains no benchmark
process of any kind, only idle desktop processes (`firefox-bin`, `Xorg`,
`cinnamon`) and two `claude` CLIs. The three-repeat spread reported under the
stability gates is the check that matters, and no level moved more than 3.5%
across repeats.

## The invocation path, and why it is the conformance harness's

`perf_corpus.py` is `bench.py`'s corpus resolution and engine call lifted
verbatim into functions. Every timed evaluation below is the same single call
`bench.py` puts its own `ms` around:

```
ODRL_Evaluator.evaluate_ODRL_from_files(extracted_<slug>.ttl,
                                        extracted_<slug>.csv,
                                        normalise=False,
                                        [ontology_files=[], ontology_graphs=[]])
```

from the same working directory (the repo root, so `ODRL/ODRL22.ttl` resolves —
see `bench.py` note 1), on the same 68 cases, in the same order, reduced by the
same `r[1] == 1 → Allow` rule. `bench.py` itself was not modified and still runs
standalone as documented above. Upstream exposes no pre-parsed entrypoint, so
unlike a library with a separate parse step there is no second, "engine-only"
path to time here — Turtle parsing is inside the measurement because it is
inside the engine's only public call.

**Both paths `bench.py` documents were measured, in separate processes**:
`--isolate` (fresh `ontology_files`/`ontology_graphs` per call) is the primary
one; the default, non-isolated one reproduces upstream's own `test_on_force.py`
conditions and turns out to be a performance story of its own (below).

### Reproducing the whole pass

```sh
SCRATCH=/some/isolated/dir \
RESULTS=<this-directory>/results \
BENCH_SRC=<this-directory> \
COLD_PIP=1 \
bash run_perf.sh
```

`run_perf.sh` records the environment, clones at the pinned commit, builds the
venv, measures the footprint, re-runs the **conformance** harness first, then
runs `perf_bench.py` (three configurations) and `load_bench.py` under
`/usr/bin/time -v`, and sweeps stragglers. It runs nothing in the background;
whole pass, 8m13s wall.

## 1. Warmup and smoke test

**10 discarded iterations**, cycling testcases 001–010, before any timed
measurement, in every one of the three perf configurations and in every load
worker (5 there). It doubles as the smoke test: a warmup error aborts the run
and refuses to print timings rather than letting a broken engine produce
numbers. All runs completed warmup cleanly. Raw: `warmup_ms` in
`results/perf-*.json`.

| run | warmup latencies, ms (in order) |
|---|---|
| isolated | 58.4, 50.3, 65.8, 48.8, 50.4, 51.7, 79.2, 49.5, 48.4, 48.1 |
| non-isolated | 56.7, 48.5, 68.1, 47.7, 48.8, 50.1, 80.6, 48.2, 48.4, 48.0 |
| isolated, GC off | 56.5, 48.4, 67.1, 47.5, 48.3, 49.5, 73.4, 47.9, 48.9, 48.1 |

There is **no meaningful cold-start cliff inside the evaluator** — the first
call is 1.19× the steady value, and the 65–80 ms entries at positions 3 and 7
are the GC spike described below, not warmup. What *is* a real one-off is the
import: `import ODRL_Evaluator` (pulling `rdflib`, `pyshacl`, `owlrl`, `pandas`)
costs **265 ms** and brings the process to **116.7 MiB** resident before a
single evaluation. For a per-request CLI invocation that dwarfs the ~49 ms
evaluation; it is timed separately (`import_ms`) rather than hidden in the
first measurement.

## 2. Per-case latency, post-warmup

Full 68-case corpus (all 68 evaluate; nothing is skipped), **5 repeats** =
340 measurements per configuration, per-case, not just totals. Every
measurement is in `samples[]` and every case's spread in `per_case[]` of
`results/perf-*.json`.

| ms | isolated (primary) | non-isolated (upstream conditions) | isolated, GC off |
|---|---:|---:|---:|
| n | 340 | 340 | 340 |
| mean | 59.91 | 173.63 | 52.33 |
| **median** | **49.12** | **184.28** | **49.11** |
| p95 | 87.25 | 324.43 | 51.80 |
| p99 | 152.13 | 370.62 | 120.99 |
| min | 47.32 | 47.28 | 47.15 |
| max | 164.09 | 435.39 | 123.76 |
| stddev | 21.80 | 84.95 | 14.66 |
| IQR | 28.25 | 146.48 | **0.86** |
| wall per 68-case pass | 4.02–4.12 s | 4.46 → 19.04 s | 3.55–3.58 s |

A full corpus pass costs **4.0 s in one process — about 16.8 evaluations/s**,
and the load ramp's own `c=1` step independently measured 16.18/s.

Per-case, the corpus is mildly heterogeneous, not bimodal: 65 "light" cases have
per-case medians in a **48.1–54.8 ms** band, and the three big-policy fixtures
sit at 150.4 (`062`), 144.6 (`063`) and 120.1 ms (`064`) — **2.45–3.06× the
median**, a far gentler heavy tail than an engine like the SolidLab evaluator
shows on the same three fixtures. Those three account for **11.5%** of the sum
of all 68 per-case medians.

## 3. A periodic 1.65× latency spike, and what causes it

The isolated run's mean (59.9) sits well above its median (49.1) and its p95
(87.3) is 1.78× the median, even though no *case* is intrinsically slow enough
to explain it. The raw samples say why: **23.1% of light measurements (75 of
325) land in a slow band at a median of 80.5 ms against a fast band of 48.8 ms,
a ratio of 1.649** — and the spacing between consecutive slow measurements has
median 4, min 4, max 9. It is periodic, not case-specific: the same fixture is
fast on one repeat and slow on the next.

That periodicity is the signature of CPython's generational collector, and
`perf_bench.py --gc-off` is the controlled test. Running the identical corpus
with `gc.disable()`:

| | GC on (default) | GC off |
|---|---:|---:|
| slow-band measurements | 75 / 325 (23.1%) | **0 / 325** |
| GC collections during the timed passes (gen0/1/2) | 10,142 / 922 / 84 | 0 / 0 / 0 |
| objects collected in gen2 | 7,703,172 | 0 |
| median ms | 49.12 | 49.11 |
| p95 ms | 87.25 | **51.80** |
| IQR ms | 28.25 | **0.86** |
| wall per pass | 4.02–4.12 s | 3.55–3.58 s (**−12%**) |
| peak RSS (`time -v`) | 159.6 MiB | **1.873 GiB** |
| in-process RSS, first → last | 147.1 → 156.1 MiB | 145.0 MiB → **1.835 GiB** |

So this is diagnosed, not merely observed: **each evaluation leaves behind
enough reference cycles that gen-2 collection fires roughly every fourth call**,
and the collector is buying a flat ~155 MiB working set at the price of a
periodic pause worth 12% of throughput and 1.7× the p95. Turning it off makes
latency almost perfectly flat and memory unbounded — 1.7 GiB of growth over 340
evaluations, with no sign of levelling. Neither setting is free; a deployment
that cares about tail latency has a real, measured knob here, and the number to
weigh it against is in this table. What was *not* established is whether a
middle setting (`gc.freeze()` after import, or raised gen-2 thresholds) gets
most of the flatness at bounded cost.

## 4. The mutable-default state leak, priced

`bench.py`'s note 2 records that `evaluate_ODRL_from_files` carries mutable
default `ontology_files=[], ontology_graphs=[]` and appends to them on every
call, so in a loop each earlier policy graph is fed to the reasoner again. That
was known as a *correctness* hazard. Measured, it is also a performance one:

| repeat (68 cases each) | 1 | 2 | 3 | 4 | 5 |
|---|---:|---:|---:|---:|---:|
| wall, s | 4.46 | 8.22 | 11.71 | 15.63 | 19.04 |
| median latency, ms | 50.5 | 99.0 | 146.4 | 202.6 | 247.0 |

Growth is linear at **+3.7 s per additional pass**; the first 20 evaluations of
the process have a median of 49.6 ms and the last 20 a median of **288.6 ms, a
5.8× degradation**, with in-process RSS climbing 146.8 → 218.4 MiB. Verdicts
never change (9 mismatches in every repeat, same nine fixtures), so on this
corpus the leak costs time and memory, not answers — but it is unbounded in a
long-lived process, and any deployment that keeps the interpreter alive across
requests without passing fresh lists will slow down forever. Raw:
`results/perf-upstream.json`.

## 5. System requirements

No JVM, no database, no container, no service, and **no build step of any kind**
— nothing is compiled, the "build" is entirely dependency resolution. Raw:
`results/clone.time.txt`, `venv-create.time.txt`, `pip-install-*.log`,
`footprint.txt`, `pinned-commit.txt`.

| | measured |
|---|---|
| Runtime actually used | `python3 -VV` → **3.12.3 (main, Jul 15 2026) [GCC 13.3.0]**; `git --version` → 2.43.0 |
| `git clone` | **5.48 s** wall, 34.0 MiB peak RSS |
| `python3 -m venv venv` | **1.56 s** wall, 73.4 MiB peak RSS |
| `pip install -r requirements.txt` (warm pip cache) | **14.70 s** wall — 11.99 s user + 0.70 s sys, 86% CPU, 206.3 MiB peak RSS, **59 distributions** |
| same, `--no-cache-dir` (network-cold) | **1 m 23.04 s** wall — 14.45 s user + 1.73 s sys, 19% CPU, 206.8 MiB peak RSS |
| Total from bare machine to first evaluation | ~21 s warm, ~90 s cold |

**`requirements.txt` is completely unpinned upstream** (nine bare package
names), so a reproducer months from now will not get this dependency set. The
versions actually measured are recorded verbatim in `footprint.txt`'s
`pip freeze`; the ones on the evaluation path are **rdflib 7.6.0, pyshacl
0.40.1, owlrl 7.6.2, pandas 3.0.5** (and `numpy 2.5.2` / `pyarrow 25.0.1`,
which pandas 3.0 requires).

On-disk footprint (`du -sh`):

| | size |
|---|---:|
| whole checkout, incl. `.git` + `venv` | **610 MB** |
| `venv` | 576 MB |
| `venv/lib/python3.12/site-packages` | 554 MB |
| upstream source tree only (no `venv`, no `.git`) | 20 MB |
| `.git` | 14 MB |
| upstream's pre-translated corpus (`test_cases/evaluation/force`) | 1.2 MB |
| `ODRL_Evaluator.py` + `rdf_utils.py` — the engine itself | 81.6 KB |

The five largest installed packages are `pyarrow` 156M, `pandas` 73M, `numpy`
43M, `streamlit` 35M, `matplotlib` 35M. Roughly **150 MB of the 554 MB is the
Streamlit/matplotlib UI stack** (`streamlit`, `altair`, `pydeck`, `matplotlib`,
`fontTools`, `pillow`) plus `fastapi`/`uvicorn`, none of which
`ODRL_Evaluator.py` or `rdf_utils.py` imports — they are in `requirements.txt`
for the project's apps, not its evaluator. The evaluator you are benchmarking is
82 KB of Python on top of a half-gigabyte dependency tree it uses about
three-quarters of.

## 6. Resource consumption during the performance run

Tool: **`/usr/bin/time -v`** wrapped around each whole `perf_bench.py` process
by `run_perf.sh` — it reports `Maximum resident set size` and CPU seconds
directly from `wait4()`. Raw: `results/perf-*.time.txt`. In addition
`perf_bench.py` reads `/proc/self/status` `VmRSS` after **every** timed
evaluation (`memory_in_process` in `results/perf-*.json`), giving steady state
and growth rather than only a peak. `psutil` was deliberately not used: it is
not in OVAL's `requirements.txt`, and installing it would have changed the
dependency tree this same pass reports the footprint of.

| | isolated | non-isolated | isolated, GC off |
|---|---:|---:|---:|
| Elapsed | 0:21.80 | 1:02.90 | 0:20.76 |
| User CPU | 23.51 s | 64.54 s | 21.82 s |
| System CPU | 0.11 s | 0.18 s | 0.76 s |
| Percent of CPU | 108% | 102% | 108% |
| **Peak RSS (`time -v`)** | **159.6 MiB** | 231.5 MiB | **1.873 GiB** |
| Major page faults / swaps | 0 / 0 | 0 / 0 | 0 / 0 |
| Minor page faults | 31,390 | 75,136 | 478,283 |
| In-process RSS, first sample | 147.1 MiB | 146.8 MiB | 145.0 MiB |
| In-process RSS, steady (median) | 155.3 MiB | 180.0 MiB | 994.4 MiB |
| In-process RSS, last sample | 156.1 MiB | 218.4 MiB | 1.835 GiB |
| Growth over 340 evaluations | **+9.0 MiB** | +71.6 MiB | +1.693 GiB |

The primary path is the well-behaved one: **+9 MiB over 340 evaluations**, which
is flat for practical purposes. "Percent of CPU" just above 100% is close enough
to single-threaded that the honest evidence for single-threadedness is the load
bench's kernel measurement instead — at `c=1` it reads **exactly 1.00 busy
cores** from `/proc/<pid>/stat`. The residual ~8% is not evaluation parallelism
(the GIL probe below rules that out) and is not diagnosed further here.

## 7. Load and peak behaviour

### What "concurrency" means for THIS engine — not apples-to-apples

**One unit of concurrency = one OS process**, specifically `multiprocessing`
with the **`spawn`** start method (not `fork`, not threads). Each worker is a
fresh CPython interpreter that does its own `import ODRL_Evaluator`, its own
ODRL 2.2 ontology parse and its own 5-evaluation warmup, so nothing is shared
copy-on-write and no worker's rdflib state can reach another's.

That choice was **measured, not assumed**. `perf_bench.py` runs a `gil_probe`:
4 evaluations sequentially against the same 4 through a `ThreadPoolExecutor`.

| run | 4 sequential | 4 via ThreadPoolExecutor | speedup |
|---|---:|---:|---:|
| isolated | 226.35 ms | 260.96 ms | **0.867×** |
| non-isolated | 1248.43 ms | 1391.18 ms | 0.897× |
| isolated, GC off | 195.30 ms | 213.76 ms | 0.914× |

Threads are consistently **slower than sequential**: the work is CPU-bound
Python holding the GIL, and the thread pool only adds switching. In-process
concurrency for this engine does not exist. A reader comparing this axis to a
thread-based or async-runtime figure from another engine is comparing different
things — a process here carries a whole CPython runtime plus its own rdflib
heap (~154 MiB), which a native thread does not. The other plausible unit,
one `subprocess` per evaluation, was rejected because it would have re-paid the
265 ms import on every call and measured process startup rather than the engine.

### Method

Levels **1, 2, 4, 8, 16, 22, 32, 44**; **10 s** nominal step; **3 full ramp
repeats**; 5 warmup evaluations per worker before the host will time it; spawns
staggered 150 ms (pool of 44 ready in 7.2–7.4 s). The pool is spawned once per
ramp repeat at the maximum level; at each level only the first `c` workers are
told to go and the rest block in `recv()` — memory resident, zero CPU. Per step
the host samples `/proc/<pid>/status` `VmRSS` and `/proc/<pid>/stat`
`utime+stime` for the **active** workers every **250 ms**, so RSS and CPU are
kernel-reported, not self-reported. Steady-state RSS is the median of samples
after the first second. A memory guard skips any step with `MemAvailable` below
`MIN_FREE_MB=6144`; **no step was skipped** (`MemAvailable` at each pool:
73.3, 73.8, 73.7 GB). `tput(wall)` divides completed evaluations by the step's
real wall time; `tput(win)` counts only evaluations that finished inside the
nominal 10 s. `L-*` columns repeat the latency stats with the three big-policy
fixtures removed — a second view, not an exclusion, so corpus composition stays
distinguishable from contention.

### Results — `results/load.json`

Medians across the 3 ramp repeats. Latency ms, RSS GiB.

| c | tput(wall) | tput(win) | per-worker | wall s | med | p95 | p99 | L-med | L-p95 | L-p99 | heavy% | RSS peak | RSS steady | cores | err | rel.range |
|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| 1 | 16.18 | 16.10 | 16.18 | 10.01 | 51 | 86 | 156 | 51 | 84 | 86 | 3.7% | 0.15 | 0.15 | 1.00 | 0 | 0.012 |
| 2 | 30.01 | 30.60 | 15.01 | 10.26 | 55 | 97 | 162 | 55 | 92 | 97 | 3.9% | 0.31 | 0.30 | 1.96 | 0 | 0.023 |
| 4 | 55.13 | 56.20 | 13.78 | 10.27 | 59 | 121 | 183 | 58 | 104 | 119 | 4.2% | 0.61 | 0.61 | 3.91 | 0 | 0.029 |
| 8 | 80.89 | 82.30 | 10.11 | 10.27 | 76 | 198 | 250 | 75 | 143 | 216 | 5.5% | 1.22 | 1.21 | 7.81 | 0 | 0.023 |
| **16** | **110.47** | **112.10** | 6.90 | 10.29 | 126 | 281 | 398 | 125 | 270 | 355 | 3.7% | 2.42 | 2.40 | 15.65 | 0 | 0.012 |
| 22 | 108.62 | 110.60 | 4.94 | 10.38 | 173 | 364 | 495 | 169 | 302 | 400 | 3.9% | 3.32 | 3.31 | 20.70 | 0 | 0.010 |
| 32 | 105.17 | 106.70 | 3.29 | 10.45 | 267 | 627 | 866 | 255 | 548 | 681 | 5.2% | 4.83 | 4.81 | 20.82 | 0 | 0.018 |
| 44 | 105.71 | 106.80 | 2.40 | 10.49 | 356 | 810 | 1127 | 347 | 729 | 987 | 3.6% | 6.60 | 6.59 | 20.89 | 0 | 0.035 |

**There is a real, clean degradation point at c=16.** Throughput peaks at
**110.47 eval/s**, then falls and flattens at ~105–109 for every level beyond,
while median latency keeps climbing — 51 ms at c=1 to 356 ms at c=44, a **7.0×
rise for zero additional throughput**. Busy cores saturate at **20.7–20.9 of 22**
from c=22 onward, which is the binding constraint; memory never is (6.60 GiB at
c=44 against 73 GiB available, a flat **~154 MiB per worker** at every level).

Scaling is sublinear well before saturation — per-worker throughput is already
down to 10.11/s at c=8 with only 7.81 cores busy. That is the machine's hybrid
topology, not the engine: 6 performance cores at 4.8–5.1 GHz, then 8 E-cores at
3.8 and 2 LP-E at 2.5, so the ninth-and-later worker is on slower silicon.

### The ceiling, and why it is 44

44 = **2× `nproc`**, chosen because it is comfortably past the observed
degradation point rather than in place of finding one: throughput had already
turned over at c=16 and been flat for three levels, CPU was pinned at ~95% of
22 threads, and memory was nowhere near binding. Going further would only have
deepened the queue, not found a new regime.

### Errors under load

**Zero exceptions**, at every level, in every repeat: **19,017 evaluations
across 24 timed steps, error rate 0.000000 throughout**. The separate `mismatch`
counter is not an error rate — its baseline is the conformance result itself (9
of 68), and checking every step's `mismatch_slugs` against the nine known
conformance failures gives **no mismatch slug outside that set, anywhere in the
ramp**. Load produced no new wrong answers.

## 8. Outlier and stability gates

Three stated numeric rules (constants live in `perf_corpus.py`). Nothing they
catch is discarded; it is reported and marked.

1. **Per-measurement, Tukey fence k = 1.5.** A measurement is flagged when it
   falls outside `[Q1 − 1.5·IQR, Q3 + 1.5·IQR]` of that run's own pooled
   distribution.
2. **Per-case cross-repeat instability.** A case is flagged `unstable` when
   `(max − min) / median` over its 5 measurements exceeds **0.25**.
3. **Per-level ramp instability.** A concurrency level is flagged
   `unstable_across_repeats` when its `tput(wall)` `(max − min) / median` across
   the 3 ramp repeats exceeds **0.15**.

| gate | isolated | non-isolated | isolated, GC off |
|---|---|---|---|
| rule 1 fence | `[6.16, 119.16]` ms | `[−123.49, 462.43]` ms | `[47.44, 50.90]` ms |
| rule 1 flagged | **12 / 340** | **0 / 340** | **30 / 340** |
| what got flagged | only big-policy fixtures (062×5, 063×4, 064×3) | nothing | the 15 big-policy measurements + 15 light ones |
| rule 2 unstable cases | **64 / 68** (worst 0.961) | **68 / 68** (worst 2.026) | **0 / 68** (worst 0.135) |
| rule 3 unstable levels | **0 / 8** (worst 0.035 at c=44) | — | — |

The honest reading of each row:

- The isolated run's **64-of-68 rule-2 flags are the GC spike, not measurement
  noise**, and the GC-off run proves it: the identical corpus on the identical
  machine drops to 0 of 68 with the collector disabled. They are reported as
  flagged. The aggregate numbers they sit inside are nonetheless highly
  repeatable — the five per-pass walls span 4.02–4.12 s, a 2.5% range.
- The non-isolated run flags **every** case (worst 2.03) and **no** measurement:
  a distribution stretched by a monotonic 5.8× drift has an IQR so wide the
  Tukey fence catches nothing. That is a case where rule 1 is uninformative and
  rule 2 is the one doing the work; both are reported rather than one being
  quietly preferred.
- The GC-off run's 30 rule-1 flags are an artifact of the opposite condition: its
  IQR collapses to **0.86 ms**, so the fence is only 3.5 ms wide and any
  millisecond of jitter crosses it. Flagged, and explained.
- The load ramp is the most stable measurement in the pass: **worst throughput
  relative range 0.035** across three full repeats of eight levels.

## What was not measured, and why

- **The FastAPI/Docker service.** It exists upstream, but the conformance number
  this pass is anchored to came from the library entrypoint; timing the HTTP
  wrapper would measure a different thing.
- **Concurrency above 44.** Refused as uninformative: throughput turned over at
  16 and CPU was pinned from 22 on.
- **A middle GC setting.** `gc.freeze()` or raised gen-2 thresholds might get
  most of the flat-latency benefit at bounded memory cost. Not attempted.
- **The `retranslate.py` corpus.** All perf runs used upstream's own committed
  pre-translated corpus, the same one the 59/68 conformance number used.

## Files added by this pass

Scripts, beside the existing harness:

- `perf_corpus.py` — `bench.py`'s corpus resolution, engine call and cwd/isolate
  handling as a shared module, plus `/proc` readers and the percentile/IQR
  helpers and gate constants. No new evaluation logic.
- `perf_bench.py` — import cost, warmup/smoke test, per-case latency over 5
  repeats, the in-process RSS series, the GC accounting and `--gc-off` control,
  the bimodality analysis, and the GIL probe. Single process.
- `load_worker.py` — one load-generator process; line protocol over a duplex
  `Pipe`, reports per-evaluation latency with the fixture it belongs to.
- `load_bench.py` — the ramp host: spawns the pool, walks the levels, samples
  `/proc` for active workers, enforces the memory guard, aggregates, and kills
  its pool on every exit path including `SIGINT`/`SIGTERM`.
- `run_perf.sh` — the driver that produced everything below, in order.

Raw results, in `results/` (nothing overwrites `results_[AB]_*.json`):

| file | what |
|---|---|
| `environment.txt` | pre-run `uptime`, `free -h`, `nproc`, `lscpu`, versions, `ps` |
| `pinned-commit.txt` | the commit actually measured (`a427e71…`) |
| `clone.time.txt`, `venv-create.time.txt`, `pip-install-warmcache.log`, `pip-install-nocache.log` | `/usr/bin/time -v` for every setup step |
| `footprint.txt` | `du -sh` breakdown, largest packages, full `pip freeze`, venv Python |
| `verify-conformance-A.json/.txt`, `verify-conformance-B.json/.txt` | the conformance harness re-run on this checkout, both paths |
| `perf-isolated.json/.time.txt` | primary per-case latency + resources |
| `perf-upstream.json/.time.txt` | the non-isolated path, incl. the state-leak drift |
| `perf-isolated-nogc.json/.time.txt` | the GC control run |
| `load.json` (3.2 MB), `load.time.txt` | the ramp, incl. every per-worker `[slug, ms, end_offset, correct, error]` record for all 19,017 evaluations |
| `sweep.txt` | post-run straggler count and machine state |

## Cleanup

`load_bench.py` kills its pool after every ramp repeat, in a `finally`, and on
`SIGINT`/`SIGTERM`; `run_perf.sh` sweeps as its last step and records the
result. After the final run `sweep.txt` reports **0** stragglers, and a
`ps -eo args | grep -cE '[l]oad_worker|[l]oad_bench|[p]erf_bench'` afterwards
returned 0 as well. No process, server or load generator was left running.
