# Bench: `ds-odrl-engine-rs` (this repo — the comparison's baseline)

The fifth row of the five-engine comparison in [`../README.md`](../README.md),
and the one the other four are measured against.

Unlike the other four subdirectories, this one is **not** a conformance
harness. This engine's conformance number is already produced by
`cargo run -p compliance-runner --release` in this same repo (68 total, 68
passed, 0 failed, 0 skipped) and needs no separate rig. What this directory
adds is the dimension `../README.md` explicitly scoped out of that pass:
**performance, system requirements, resource consumption and load/peak
behavior**, measured over the identical 68-fixture corpus.

## Pinned version

This engine is the repo you are reading, so there is nothing to clone from a
third party — but the numbers below were still produced from a **fresh,
isolated clone at a pinned commit**, not from a working tree, on the same
discipline the other four subdirectories apply to their targets:

```sh
git clone https://github.com/ds-labs-org/ds-odrl-engine-rs.git
cd ds-odrl-engine-rs
git checkout --detach 7068ebae83557a2cfda4d55fc1f2d585f5fc724b
```

`7068eba` is the commit that added `bench/` — the same tree the other four
harnesses' reproductions were run against. The `compliance/vendor/odrl-test-suite`
submodule is **not** needed for this bench (see "The corpus" below), so
`git submodule update --init` can be skipped.

## Run

```sh
bench/ds-odrl-engine-rs/run.sh            # writes results/ (~3 min)
```

`run.sh` builds `engine.wasm`, builds the harness, and runs four phases,
each as its own process under `/usr/bin/time -v`. It must run on a **quiet
machine**: two of the four phases saturate every core.

Individual phases, if you want them one at a time:

```sh
cd bench/ds-odrl-engine-rs/perf
CARGO_TARGET_DIR=/tmp/perf-bench cargo build --release
/tmp/perf-bench/release/perf-bench native      out.json
/tmp/perf-bench/release/perf-bench wasm        out.json
/tmp/perf-bench/release/perf-bench load-native out.json
/tmp/perf-bench/release/perf-bench load-wasm   out.json
```

`perf/` is deliberately **its own cargo workspace** (an empty `[workspace]`
table in its `Cargo.toml`), so `wasmi` and a bench-only binary never enter
`cargo test --workspace` / `cargo clippy --workspace --all-targets` or any
other gate this repo already runs. The root `Cargo.toml` is untouched.

## What's here

- `perf/src/main.rs` — the harness. Every knob (warmup passes, repetition
  counts, ramp steps, sampling rule) is a documented constant at the top.
- `perf/src/host.rs` — the WASM-ABI driver. A near-verbatim copy of
  `release-history/src/host.rs`, with one intended change: fuel metering
  becomes a constructor argument so both the metered and unmetered costs can
  be reported. Copying rather than reimplementing is the point — a
  differently-written ABI driver would not be measuring the thing the rest of
  this repo executes.
- `perf/src/stats.rs` — summary statistics and the three numeric stability
  gates, defined in one place so this README and the result JSON quote the
  same constants the code applies.
- `run.sh` — the driver described above.
- `results/` — the raw artifacts. `native.json`, `wasm.json`,
  `load-native.json`, `load-wasm.json`, one `*.time.txt` per phase and per
  build (verbatim `/usr/bin/time -v` output, the evidence behind every
  resource figure below), plus `environment.txt` and `footprint.txt`.

---

# Performance, resources and load

Everything below is a real number from a real execution in this environment
on **2026-09-06T08:45:06+02:00**, traceable to a named file in `results/`.
Nothing is estimated, and nothing is carried over from the conformance run.

## The corpus, and why it is the same corpus

Both invocation paths are driven from
[`compliance/reports/latest-cases.json`](../../compliance/reports/latest-cases.json)
— the per-case `engine::wire::Request` values `compliance-runner` actually
fed `engine::evaluate_request`, exported by `compliance-runner/src/cases.rs`
for exactly this kind of re-execution. Using that artifact rather than
re-deriving requests from the RDF fixtures guarantees the requests being
timed are byte-identical to the ones the published conformance tally was
computed over.

All **68 of 68** fixtures are evaluable, so this is the whole corpus, not a
subset. Each phase re-checks its own decisions against each fixture's
`expected_decision` and reports the agreement: **68/68 on all three timed
paths**. That is not a second conformance run; it is a guard that the thing
being timed is still the thing that was scored.

## Environment, and the baseline load

From `results/environment.txt`:

| | |
|---|---|
| CPU | Intel Core Ultra 9 185H — 16 physical cores (6 performance + 8 efficient + 2 low-power efficient), **`nproc` = 22** logical |
| Memory | 93 GiB total, 71 GiB available |
| Kernel | Linux 6.8.0-138-generic x86_64 |
| Toolchain | `rustc 1.96.1 (31fca3adb 2026-06-26)`, `cargo 1.96.1 (356927216 2026-06-26)`, plus the `wasm32-unknown-unknown` std component |
| Load average at phase start | **0.50** (1 min) |

The machine was checked with `ps aux` / `uptime` / `nproc` before the first
timed measurement: nothing heavy was running — only an interactive desktop
session (a Firefox process tree, single-digit percent CPU), no other engine's
bench, no stray build. This is the first phase of the sequential measurement
pass, so no other engine's harness had run yet.

**A caveat stated rather than hidden:** this is a shared interactive desktop,
not a dedicated benchmark host, and the CPU is a **heterogeneous P-core /
E-core** part. The consequences are visible in the numbers and are called out
where they appear (they are the reason parallel scaling is 8.3x rather than
16x at 16 threads). The three-repeat stability gate below exists precisely
because of this, and every step passed it.

## The two invocation paths

This is the only engine in the comparison with two genuinely different
invocation paths, and they are reported separately because they measure
different products.

| Label | What it is | Who experiences it |
|---|---|---|
| **(a) NATIVE** | `engine::evaluate_request(&Request)` — one Rust library call. No serialization, no ABI, no interpreter. | The entry point `compliance-runner` itself uses. A genuine lower bound. |
| **(b) WASM-ABI** | The same requests through the real `engine.wasm` via `alloc` / `evaluate` / `dealloc` over guest linear memory, on a `wasmi` interpreter. | Any real host: a browser, a JVM via Chicory, another process embedding the artifact rather than linking the crate. |

Path (b) is reported at **two fuel settings** because this repo really runs
both: `release-history/src/host.rs` meters fuel (a runaway guard when
executing historical binaries), and `site/src/engine_bridge.rs` in a browser
has no fuel accounting at all.

The timed region on path (b) is exactly `WasmEngine::evaluate(&str)` —
`alloc` + `memory.write` + `evaluate` + `memory.read` + two `dealloc`s.
Request-to-JSON serialization is host-side work outside that boundary and is
**not** included.

## 1. Warmup and smoke test

**3 full passes over all 68 fixtures, discarded**, before any timer starts, on
every phase. Rust has no JIT, but three things genuinely are cold on a first
pass and would otherwise be charged to fixture #1: the page cache and
resident pages for the freshly-read 650 KB corpus file, first-touch growth of
the allocator's arenas, and — on the wasm path — `wasmi`'s lazy per-function
translation plus the guest allocator's and linear memory's first growth.

The warmup doubles as the smoke test: it asserts a decision comes back for
every fixture and panics with the fixture's slug otherwise. **All warmups
passed cleanly on every phase**, on both paths.

## 2. Per-case latency, post-warmup

Full 68-fixture corpus. `NATIVE_REPS = 501` timed repetitions per case on
path (a), `WASM_REPS = 51` on path (b) (odd counts, so every median is a real
observed sample rather than an interpolation). Timing is
`std::time::Instant`, `std::hint::black_box` on both the request and the
response.

**Headline distribution — one number per fixture** (each fixture's own
median, so per-case noise is already removed; 68 samples). Source:
`results/native.json`, `results/wasm.json`.

| Path | mean | median | p95 | p99 | min | max |
|---|---:|---:|---:|---:|---:|---:|
| (a) native | 19.16 µs | **0.78 µs** | 2.23 µs | 527.21 µs | 0.17 µs | 527.21 µs |
| (b) wasm-ABI, fuel-metered | 369.26 µs | **62.76 µs** | 96.62 µs | 8 818.75 µs | 34.82 µs | 8 818.75 µs |
| (b) wasm-ABI, unmetered | 349.39 µs | **60.48 µs** | 94.46 µs | 8 198.92 µs | 33.71 µs | 8 198.92 µs |

**Every individual timed repetition** (34 068 samples native, 3 468 each wasm
— wider tails by construction, since every scheduler hiccup is in there):

| Path | n | mean | median | p95 | p99 | min | max |
|---|---:|---:|---:|---:|---:|---:|---:|
| (a) native | 34 068 | 19.71 µs | 0.80 µs | 2.26 µs | 526.69 µs | 0.17 µs | 1 493.73 µs |
| (b) wasm, metered | 3 468 | 372.47 µs | 63.37 µs | 97.91 µs | 8 801.20 µs | 33.43 µs | 9 783.63 µs |
| (b) wasm, unmetered | 3 468 | 351.42 µs | 60.87 µs | 96.73 µs | 8 204.63 µs | 32.33 µs | 10 134.60 µs |

Reading these:

- **The ABI costs ~80x.** Median 62.76 µs through `wasmi` against 0.78 µs
  native. That is the price of an *interpreter*, not of the wasm format —
  a JIT-backed host (Wasmtime, or a browser's own engine) would land
  somewhere between the two, and this bench does not measure that.
- **Fuel metering costs 3.8%** at the median (62.76 vs 60.48 µs). Small
  enough that a host wanting a runaway guard has no real reason to skip it.
- **Mean far exceeds median on every path.** That is not noise; it is three
  genuinely heavy fixtures, correctly flagged by the outlier gate below.

**Wasm cold start**, measured separately (`Module::new` + `Store::new` +
`instantiate_and_start` + the four export lookups, 21 repetitions):
**1.023 ms** median metered, **1.034 ms** median unmetered. That is a
per-instance cost a host pays once, not per request.

## 3. System requirements

What this engine needed to build and run at all, here.

**Runtime dependencies: none.** No JVM, no Node, no Python, no database, no
policy sidecar. `engine.wasm` is a **278 660 byte (272 KiB)** self-contained module that
imports nothing from its host — the harness instantiates it with a
deliberately *empty* `Linker`, and it instantiates cleanly. The native path
is a plain Rust `rlib`.

**Build toolchain:** `rustc`/`cargo` 1.96.1 plus the `wasm32-unknown-unknown`
std component. Nothing else.

**Real build times**, `/usr/bin/time -v`, from a cleaned `target/` with the
crates.io registry already populated (so this is a from-scratch *compile* of
the whole dependency graph, not a network fetch):

| Build | wall | CPU (user+sys) | peak RSS | evidence |
|---|---:|---:|---:|---|
| `cargo build -p engine --target wasm32-unknown-unknown --release` | **3.22 s** | 9.13 s | 287 MB | `results/build-engine-wasm.time.txt` |
| `cargo build -p compliance-runner --release` (produces the conformance number) | **4.68 s** | 27.75 s | 381 MB | `results/build-compliance-runner.time.txt` |
| `cargo build --release` for this bench harness (adds `wasmi`) | **10.20 s** | 64.38 s | 809 MB | `results/build-perf-bench.time.txt` |

**On-disk footprint** (`du -sh`, `results/footprint.txt`):

| | |
|---|---:|
| `engine.wasm` — the deployable artifact | **278 660 B (272 KiB)** |
| `target/wasm32-unknown-unknown/release/` | 19 MB |
| `target/release/` (native, incl. `compliance-runner`) | 108 MB |
| `target/` total | 126 MB |
| Bench harness build tree (`wasmi` and friends) | 131 MB |
| Source clone, excluding `target/` and `.git/` | 7.7 MB |
| Rust toolchain (`rustup` sysroot) | 1.5 GB |
| Shared crates.io registry cache | 1.5 GB |

The 272 KiB module is the number that matters for an integrator; the gigabytes
are build-time only and are the toolchain's, not this engine's.

## 4. Resource consumption during the performance run

Tool: **`/usr/bin/time -v`**, invoked as
`/usr/bin/time -v -o results/<phase>.time.txt <bin> <phase> results/<phase>.json`.
`-o FILE` rather than a stderr redirect, because the harness writes its own
progress to stderr and interleaving the two would make the raw evidence
unparseable. Each phase is its own process precisely so "Maximum resident set
size" is attributable to one phase rather than a mixture. The harness
*also* samples `/proc/self/status` (`VmRSS`) and `/proc/self/stat` (utime,
stime) at phase boundaries, recorded in each result JSON as
`resources_at_start` / `resources_after`.

| Phase | wall | user CPU | sys CPU | %CPU | **peak RSS** | steady-state RSS at end |
|---|---:|---:|---:|---:|---:|---:|
| `native` (68 x 501 evals) | 0.68 s | 0.68 s | 0.00 s | 99% | **8 272 KB (8.1 MiB)** | 8.0 MiB |
| `wasm` (68 x 51 x 2 settings, + 42 instantiations) | 2.71 s | 2.71 s | 0.00 s | 99% | **11 780 KB (11.5 MiB)** | 10.7 MiB |
| `load-native` (whole ramp) | 72.64 s | 857.91 s | 1.47 s | 1182% | **251 952 KB (246 MiB)** | 58.4 MiB |
| `load-wasm` (whole ramp) | 55.05 s | 475.84 s | 0.85 s | 865% | **186 904 KB (182.5 MiB)** | 13.3 MiB |

A single-threaded pass over the entire corpus — evaluating every fixture 501
times — costs **8.1 MiB** resident and 0.68 s of CPU. Adding the whole wasm
interpreter plus a live instance takes that to **11.5 MiB**.

The two ramp figures are dominated by the *harness*, not the engine: each
ramp thread parses its own private copy of the 68-case corpus (see "shared
nothing" below), so `load-native`'s 246 MiB peak is 44 threads' worth of
`Request` objects, roughly 5 MiB each. That is stated rather than quietly
attributed to the engine.

**One found-the-hard-way detail, kept because it affects how these numbers
should be read.** The in-process `VmHWM` checkpoints are *not* a substitute
for `/usr/bin/time -v`'s peak, and this bench does not treat them as one:
`/proc`'s `VmHWM` is `max(latched hiwater_rss, current_rss)`, and the latch
only updates at certain kernel events. In this run's own output the wasm
phase's mid-run checkpoint reads 12 068 KB while the process's true maximum
was 11 780 KB, and a *later* checkpoint therefore reads *lower* — something a
real monotonic peak counter could never do. Every **end-of-phase** reading
did match `/usr/bin/time -v` exactly. **Rule applied throughout this README:
the peak quoted for a phase is always `/usr/bin/time -v`'s.**

## 5. Load and peak behavior

### What "concurrency" means for this engine specifically

**Do not read this axis as apples-to-apples against the other four engines.**
It is not, and the difference is structural rather than a matter of degree.

`engine::evaluate_request` is a **pure, stateless function** of (policies,
request, claims). No global state, no interior mutability, no lock, no
connection pool, no interpreter instance, no session. So on path (a),
"concurrency = N" means **N real OS threads doing genuinely independent,
shared-nothing evaluations simultaneously, in one process**. Each thread even
parses its own private copy of the corpus before the clock starts, so not
even a read-only cache line is shared. That is a property of the engine's
design, not a tuning result — and it is the one thing in this whole
comparison that no other engine here can do in-process. A single-threaded
interpreter needs multiple OS processes to get real parallelism at all; a
service-shaped engine gets it, but pays a request boundary for it.

On path (b) concurrency is **not** free, and that is worth being just as
plain about: a `wasmi` `Store` is not `Sync`, so each thread owns a whole
independent `Engine` + `Module` + `Store` + `engine.wasm` instance, including
its own guest linear memory. That cost is reported per step rather than
elided.

Method: `RAMP_STEP_SECONDS = 3` at full tilt per step, all threads released
from a `std::sync::Barrier` so private setup is never inside the clock. The
**whole ramp is repeated 3 times** (`RAMP_REPEATS = 3`); the throughput below
is the median of the three, with all three shown in the JSON. Latency
sampling records every evaluation of every 8th *whole corpus pass* per
thread, capped at 65 536 samples per thread. "Errors" means a decision that
disagreed with the single-threaded baseline computed before the ramp, or an
ABI failure — not merely a `Deny`.

### (a) Native — real OS threads, `results/load-native.json`

| threads | throughput (evals/s) | rel. range over 3 repeats | p50 | p95 | p99 | max | errors | cores busy |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 46 529 | 0.026 | 1.06 µs | 9.36 µs | 570.9 µs | 1.41 ms | 0 | 1.0 |
| 2 | 88 874 | 0.003 | 1.10 µs | 9.65 µs | 591.1 µs | 1.01 ms | 0 | 2.0 |
| 4 | 169 603 | 0.078 | 1.18 µs | 10.16 µs | 627.7 µs | 1.30 ms | 0 | 4.0 |
| 8 | 276 423 | 0.036 | 1.41 µs | 12.88 µs | 805.2 µs | 2.83 ms | 0 | 8.0 |
| **16** | **387 093** | 0.048 | 1.84 µs | 48.28 µs | 1.19 ms | 2.05 ms | 0 | 16.0 |
| 22 (= `nproc`) | 358 101 | 0.109 | 3.32 µs | 53.05 µs | 1.65 ms | 36.25 ms | 0 | 21.3 |
| 32 | 285 439 | 0.020 | 4.49 µs | 47.74 µs | 3.02 ms | 80.61 ms | 0 | 21.3 |
| 44 (2x `nproc`) | 260 743 | 0.065 | 4.32 µs | 47.28 µs | 5.38 ms | 59.73 ms | 0 | 20.4 |

- **Peak throughput 387 093 evals/s at 16 threads**, then a real, repeatable
  decline. This is the observed degradation point, not an arbitrary ceiling:
  the ramp was deliberately pushed to 2x `nproc` to find it.
- **Scaling is 8.3x on 16 threads, not 16x.** The honest explanation is the
  hardware: this CPU has 6 performance cores (12 SMT threads) and 8+2
  efficiency cores. Threads past the P-cores land on E-cores and on SMT
  siblings, which do less work each. `cores_busy` confirms the threads really
  were scheduled (16.0 of 22), so this is throughput per core falling, not
  threads idling.
- **Zero errors at every level, including 2x oversubscription.** Every one of the
  roughly 16.9 million evaluations in this ramp (8 steps x 3 s x 3 repeats)
  agreed with the single-threaded baseline. For a stateless function that is the expected result — it is
  reported because "expected" is not the same as "checked".
- Tail latency degrades gracefully: p50 rises 1.06 → 4.32 µs across a 44x
  concurrency increase. The multi-millisecond `max` values at high
  concurrency are scheduler preemption of a sampled evaluation, not engine
  behavior.

### (b) WASM-ABI — one full instance per thread, `results/load-wasm.json`

Fuel-metered, matching how `release-history` drives it.

| threads | throughput (evals/s) | rel. range | p50 | p95 | p99 | errors | cores busy | guest linear memory (total) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 2 348 | 0.069 | 72.0 µs | 179.1 µs | 10.02 ms | 0 | 1.0 | 1.4 MiB |
| 2 | 4 759 | 0.005 | 71.1 µs | 120.5 µs | 9.89 ms | 0 | 2.0 | 2.9 MiB |
| 4 | 8 765 | 0.013 | 77.6 µs | 140.0 µs | 11.22 ms | 0 | 4.0 | 5.8 MiB |
| 8 | 13 804 | 0.096 | 93.9 µs | 217.3 µs | 12.89 ms | 0 | 8.0 | 11.5 MiB |
| **16** | **18 556** | 0.029 | 136.7 µs | 401.3 µs | 23.18 ms | 0 | 15.8 | 23.0 MiB |
| 22 (= `nproc`) | 15 977 | 0.019 | 173.3 µs | 713.6 µs | 39.32 ms | 0 | 20.9 | 31.6 MiB |

- Same shape, same peak position (16 threads), **21x lower throughput** — the
  interpreter cost from section 2, unchanged under load.
- **Guest memory grows exactly linearly at ~1.44 MiB per instance.** This is
  the real cost of concurrency on this path and the reason the ramp stops at
  `nproc` rather than oversubscribing: past the core count you are buying
  memory, not throughput.
- **Zero errors at every level**, across roughly 578 000 ABI round trips. No
  trap, no allocator leak across hundreds of thousands of
  `alloc`/`evaluate`/`dealloc` cycles through one long-lived instance.

## 6. Outlier and stability gates

Three gates, each a stated numeric rule applied by `perf/src/stats.rs`, not a
judgement call. **A flagged measurement is reported, marked, and kept — never
discarded.**

| Gate | Rule | Result on this run |
|---|---|---|
| **Within a case** | a case is `unstable_within_case` when its own repeats' `IQR / median > 0.25` | **0 of 68** flagged on all three timed paths |
| **Across cases** | a case is `outlier_across_cases` when its median falls outside the Tukey fence `[Q1 − 1.5·IQR, Q3 + 1.5·IQR]` over all 68 case medians | **3 of 68** flagged on every path |
| **Across ramp repeats** | a step is `unstable_across_repeats` when `(max − min) / median > 0.20` over its 3 whole-ramp repeats | **0 of 14** steps flagged; worst observed 0.109 (native, 22 threads) |

The robust `IQR/median` was chosen over `stdev/mean` deliberately: a single
scheduler preemption in one of 501 repetitions destroys a standard deviation
while leaving the interquartile spread alone, and this gate is meant to fire
on a case that is *consistently* jittery, not one that was interrupted once.

### The three flagged outliers, and what they actually are

| Fixture | native median | wasm metered median | wasm unmetered median |
|---|---:|---:|---:|
| `testcase-062-big-policy` | 207.0 µs | 3 786.5 µs | 3 533.5 µs |
| `testcase-063-big-policy-OoO` | 513.3 µs | 8 453.6 µs | 8 103.3 µs |
| `testcase-064-big-policy-past` | 527.2 µs | 8 818.8 µs | 8 198.9 µs |

These are **not noise** — all three pass the within-case gate cleanly, on all
three paths, and reproduce across independent runs. They are the corpus's own
`big-policy` fixtures: genuinely much larger policies, 250-680x the median
fixture's cost. They are the single reason the mean in section 2 sits 25x
above the median, and they are exactly what an outlier gate should surface
rather than silently average away. They are the honest answer to "what does a
large policy cost?" and are left in every reported distribution.

Two runs earlier in this session (identical method, same machine, same day)
put the native per-case median at 821 ns and 822 ns (780 ns here) and the
16-thread native peak at 374 388 and 386 824 evals/s (387 093 here) —
cross-run agreement well inside the gates above.

### A measurement bug this gate actually caught

Worth recording, because it is the kind of thing that silently produces a
publishable-looking wrong number. The first version of the load ramp sampled
every 8th *evaluation*, and reported a p99 of 9.7 µs at concurrency 1 — while
the quiet per-case run had just put three fixtures at 200-527 µs. The two
were irreconcilable, which is what exposed it: a running counter mod 8
against a 68-case corpus cycles with period `lcm(68, 8) = 136`, so only case
indices divisible by 4 are ever sampled — and the three `big-policy`
fixtures sit at indices 61, 62 and 63. The entire tail of the distribution
was structurally invisible.

The harness now samples **whole corpus passes**, which makes the recorded
distribution the corpus's own mixture by construction for any corpus length,
with no coprimality coincidence to rely on. All numbers above are from after
the fix; `RAMP_SAMPLE_PASS_STRIDE`'s comment in `perf/src/main.rs` records the
reasoning.

## What was not measured, and why

- **No JIT-backed wasm host.** Path (b) is `wasmi`, a pure-Rust *interpreter*,
  chosen because it is what this repo already uses in
  `release-history/src/host.rs` and because reusing that exact ABI driver is
  what makes these numbers comparable to the rest of the repo. A real browser
  or a Wasmtime embedding would be substantially faster than 62.76 µs; that
  number is an interpreted upper bound on the wasm path, not the wasm
  format's cost.
- **No cross-engine claim on the concurrency axis.** See section 5 — the word
  "concurrency" means something structurally different for each engine in
  this comparison.
- **No network/service dimension.** This engine has no service to stand up.
  That is a genuine finding about its deployment shape, not an omission.
