# Bench: ODRL-PAP (`SEAMWARE/odrl-pap`)

FIWARE's own ODRL policy-administration component: it compiles ODRL policies
into Rego rules, and **Open Policy Agent (OPA)** is the actual runtime
decision point, fronted by APISIX as the enforcement point in a real
deployment. Benching it means benching ODRL→Rego translation plus OPA, not a
standalone ODRL evaluator library. Pinned commit:
`59e45474c910b97f537b8f39c68e2e17ec4243ef` = tag `1.7.0`.

## Real infrastructure requirement — read before attempting this one

Unlike the other three engines here, this is not a `git clone` + one runtime
away from working. A real reproduction needs, at minimum:

- **PostgreSQL** (ODRL-PAP's own persistence)
- **Open Policy Agent**, running and reachable
- **The Quarkus service itself**, built and running, exposing the
  `/validate` endpoint this harness calls (`http://localhost:8091/validate`
  by default — see `run_pap.py`/`probes.py`)

The conformance pass did not stand this up; it only checked the harness code
for path-independence, and `bench/README.md` says so. **The performance pass
did.** `run_perf.sh` brings the whole stack up from a bare clone, re-verifies
the conformance result against it, measures, and tears it back down — see
[Performance, resources and load](#performance-resources-and-load) below.

```sh
./run_perf.sh          # clone at the pin, build, stack up, measure, stack down
```

By hand it is four commands and one caveat:

```sh
git clone https://github.com/SEAMWARE/odrl-pap.git && cd odrl-pap
git checkout 59e45474c910b97f537b8f39c68e2e17ec4243ef
./mvnw -B -DskipTests package

docker run -d --name pap-postgres --network host \
  -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=pap \
  postgres:16-alpine
docker run -d --name pap-opa --network host -v "$PWD/opa.yaml:/opa.yaml" \
  openpolicyagent/opa:1.2.0 run --server -c /opa.yaml
java -Dquarkus.http.port=8091 -Dpaths.mapping=../pap-mapping.json \
  -jar target/quarkus-app/quarkus-run.jar
```

The caveat: OPA **polls its rego bundles from the PAP**, so `services[].url`
in `opa.yaml` must be the port the PAP actually listens on. The upstream
README's sample config says `8081`; this harness talks to `8091`, so the
config has to say `8091` too. Until OPA logs three `Bundle loaded and
activated successfully` lines, no `/validate` can resolve an `odrl_*` rego
method and every case fails. Two more things that cost time when they are not
written down: `quarkus.management.enabled=true` in the engine's own
`application.properties` moves `/q/health` onto port **9000**, not the REST
port; and the JVM needs `-Dpaths.mapping` pointed at `pap-mapping.json` or the
run silently becomes the low-scoring "stock" variant.

## Run

```sh
python3 translate_pap.py            # -> cases.json (translated + a 5-term mapping overlay,
                                     #    see the file's own header for exactly what it maps)
python3 run_pap.py cases.json results.json
```

`translate_pap.py` reads the vendored corpus directly from this repo's own
`compliance/vendor/odrl-test-suite` (resolved relative to this file's own
location — works from a fresh `ds-odrl-engine-rs` checkout without
modification).

## What's here

- `translate_pap.py` — translates the 68 vendored fixtures into ODRL-PAP's
  own validation-request JSON shape, with a documented mapping overlay
  (giving `odrl:dateTime` a fixture's own clock, since OPA's own
  `time.now_ns()` has no input channel otherwise) and an honest per-case
  `skip` reason for constructs this translator cannot represent at all
  (prohibitions — `OdrlMapper` reads only `odrl:permission` — and
  `PartyCollection`/`AssetCollection` membership).
- `run_pap.py` — posts each translated case to the running service's
  `/validate` endpoint and scores the response.
- `pap-mapping.json` — the mapping overlay `translate_pap.py`'s header
  describes, passed to the JVM as `-Dpaths.mapping`. It was referenced by name
  but **missing from the repo** until the performance pass; without it the run
  is the low-scoring stock variant, so it is reconstructed here from the
  engine's own `mapping.json` extension point (nothing is patched — every
  target is a rego method ODRL-PAP already ships). It rebinds six `odrl:`
  terms onto the generic-JSON rego methods: `read`/`write`/`use` onto
  `json_action.is_*(generic.payload)` instead of HTTP verbs, `assignee` onto
  `payload.assignee` instead of a JWT/VC issuer, `target`/`uid` onto
  `generic.target`, and `dateTime` onto
  `json_lo.payload_value(generic.payload, "$.currentTime")` so the fixture's
  own SotW clock is used instead of OPA's `time.now_ns()`.
- `probes.py` — vocabulary/capability probes against the running service.
- `cases.json` — the translated corpus, committed as a point-in-time record
  (663 KB; regenerate with `translate_pap.py` rather than hand-editing).
- `results/results.json` — **30 pass, 1 fail, 37 skip**, with the mapping
  overlay engaged.
- `results/results-builtin-datetime.json` — **20 pass, 11 fail, 37 skip**,
  stock (no overlay) — `odrl:dateTime` compared against OPA's own
  `time.now_ns()`, mismatched against the fixtures' fixed clock.

## Reproduction status

The conformance numbers above are **no longer carried forward**. The
performance pass built the pinned commit from scratch, stood up PostgreSQL +
OPA + Quarkus, and re-ran `run_pap.py` against that stack before timing
anything: **30 pass, 1 fail, 37 skip**, identical to `results/results.json`.
Raw output: `results/verify-conformance.json` and `.txt`.

The one remaining failure is `testcase-061-violated` (expected `Deny`, engine
answers `Allow`) — the same single case the original run reported, not a new
regression.

---

## Performance, resources and load

Everything below is a real measurement taken on this machine on 2026-09-06 by
`run_perf.sh`, against the stack that had just re-verified the conformance
result. No number here is estimated or carried over from the conformance run.

### Method

The call path is the same one `run_pap.py` scores with: an HTTP POST of a
case's `validationRequest` to `/validate`, decision read from `allow`.
`perf_corpus.py` holds that single call so the perf and load benches cannot
drift into measuring something else. The **31 cases the engine can actually
evaluate** are timed; the 37 `skip` cases are translation gaps, not slow
paths, and timing them would time the harness.

What one `/validate` actually does matters for reading any of this. Per
request, `ValidationResource.validatePolicy` performs a JSON-LD compaction, an
ODRL→Rego mapping, a **`PUT` of a freshly named temp policy module into OPA**,
an OPA data query, and a **`DELETE` of that module**. It is a
mutate-compile-query-mutate cycle against a shared OPA, not a stateless
evaluation — which is what every surprising number below comes from.

| Step | What was run |
| --- | --- |
| Warmup / smoke | 20 `/validate` calls cycling the first 10 cases, discarded. Aborts the run on any error. Needed here more than elsewhere in the bench: a JVM pays C1/C2 JIT on the first calls, and the JSON-LD handler caches `@context` documents on first use. |
| Performance | 5 full passes over the 31 cases, one request at a time, per-case latency. Every decision re-checked against the fixture, so a run that stopped agreeing with conformance cannot pass as a clean latency number. |
| Resources | `psutil` 5.9.8 sampler (`perf_corpus.Sampler`), 0.25 s interval, watching the Quarkus JVM, the OPA container and the Postgres container **from outside** for the whole timed section. `/usr/bin/time -v` cannot wrap the thing being measured — the server outlives every measurement — so it wraps the *client* instead (`results/*.time.txt`) and RSS/CPU of the servers come from the sampler. |
| Decomposition | 200 bare OPA `/v1/data` round trips with the PAP out of the path, to separate the OPA/HTTP floor from what the PAP adds. |
| Load | Two concurrency ramps, below. |

### Environment actually used

`nproc` **22**, `free -h` total **93 GiB** (24 GiB free, 71 GiB available),
Linux 6.8.0-138 x86_64. Load average before the run: **0.31 0.62 2.19** — the
box was quiet apart from a desktop Firefox and some idle nginx/prez containers
belonging to other work, which are listed in `results/environment.txt`.
Measurements were run strictly one at a time; nothing else in this bench ran
concurrently.

### System requirements

| | |
| --- | --- |
| Runtime | OpenJDK **21.0.12** (`21.0.12+8-1-24.04-Ubuntu`); the pom targets release 17 |
| Framework | Quarkus **3.30.6**, started in **1.46 s** |
| Build tool | `./mvnw` → Apache Maven **3.9.6** (downloaded by the wrapper) |
| Harness runtime | Python **3.12.3**, `psutil` **5.9.8** |
| Also required | Docker **29.1.3**, `postgres:16-alpine` (294 MB image), `openpolicyagent/opa:1.2.0` (80 MB image) |
| Clone | **2.46 s** |
| Build | **2 m 58.4 s** wall, 91.5 s user + 5.7 s sys, peak RSS 1.57 GB — cold, into a private `-Dmaven.repo.local` so this is genuine first-build cost |
| Dependency tree | **345 MB** (the private Maven repo) |
| Built artifact | **77 MB** (`target/quarkus-app/`), 85 MB for all of `target/` |
| Source checkout | 7.7 MB |

**On-disk cost to run this engine at all: ~800 MB** (345 MB deps + 85 MB build
+ 374 MB of container images). It is by a wide margin the heaviest engine in
this bench to get to a first decision.

### Latency, single client (concurrency 1)

5 repeats × 31 cases = 155 measurements. The distribution is sharply **bimodal
and case-determined**, so the pooled figures are close to meaningless on their
own and are given with the split:

| | mean | median | p95 | p99 | min | max | stddev |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Pooled (155)** | 766.8 | 34.2 | 7193.7 | 8891.4 | 24.3 | 8957.0 | 2258.5 |
| **28 ordinary cases (140)** | 34.2 | **33.2** | 46.9 | 81.2 | 24.3 | 91.2 | 9.7 |
| **3 `big-policy` cases (15)** | 7604.2 | **7200.7** | 8940.0 | 8957.0 | 6957.2 | 8957.0 | 807.5 |

All values in milliseconds. Raw: `results/perf.json`.

The split is not noise. Every one of the 15 slow measurements is one of
`testcase-062/063/064-big-policy`, in all 5 repeats, and no ordinary case ever
exceeded 91 ms. The cause is size: `testcase-062-big-policy` is **118,183
bytes** of policy JSON against **476 bytes** for an ordinary case — 248× the
input for **217×** the latency, i.e. this pipeline is roughly *linear in policy
size* with a very large constant.

Warmup behaved as expected for a JVM: first call **49.1 ms**, median of the
last five warmup calls **29.6 ms**.

### Where the time goes

| | |
| --- | --- |
| Bare OPA `/v1/data` round trip (n=200) | median **0.64 ms** (p95 1.05, max 1.83) |
| Three of those, i.e. the OPA/HTTP floor of one `/validate` | 1.91 ms — **5.6 %** of the 34 ms median |
| Quarkus CPU consumed over the timed section | **4.41 s** |
| OPA CPU consumed over the same section | **272.07 s** |

OPA burned **62×** the CPU the Java service did. Per request in the ordinary
band, OPA spends ~65 ms of CPU against a 32 ms wall latency (it parallelises
its recompile across roughly two cores), while the JVM spends ~3.6 ms. So
**~95 % of the cost of a `/validate` is OPA recompiling its policy set on the
`PUT`/`DELETE` of the temp module** — not the ODRL→Rego translation, and not
the policy evaluation, which is the 0.64 ms query.

### Memory

| Process | first | median | peak | growth |
| --- | --- | --- | --- | --- |
| Quarkus JVM | 494 MB | 851 MB | **860 MB** | +366 MB |
| OPA | 1865 MB | 675 MB | **2056 MB** | −3 MB |
| PostgreSQL (7 procs) | 77 MB | 77 MB | 77 MB | +1 MB |

OPA's 2 GB peak is entirely the `big-policy` fixtures: on the ramp that
excluded them, OPA's peak RSS stayed at **58 MB** across every concurrency
level. A 118 KB ODRL policy costs OPA ~35× its idle footprint to compile.
PostgreSQL is effectively idle throughout — `/validate` never persists
anything, it only calls the static `PolicyRepository.generatePolicyId()`. The
database is a startup requirement, not a request-path cost.

### Load / peak behaviour

**What "concurrency" means for this engine.** ODRL-PAP is the only engine in
this bench that is a real HTTP service, so concurrency here means N client
connections issuing `/validate` at once, with the server deciding for itself
how much of that to run in parallel (Quarkus's worker pool). It is *not* the
process-fanout stand-in the single-threaded interpreters in this bench need,
and it is not the client's own parallelism. **Do not read these numbers as
apples-to-apples against the other engines' load figures** — this one includes
HTTP, JSON, JSON-LD and two extra OPA hops that an in-process evaluator never
pays.

The client is a `ThreadPoolExecutor` of blocking `urllib` callers; the GIL is
released for socket I/O, so N threads really are N requests in flight. Client
CPU is reported per step and never exceeded 2.3 s per step, so the generator
was never the bottleneck.

Ceiling: 96 concurrent connections, or earlier on a stop rule (error rate
> 2 %, or median latency > 25× the level-1 median). 96 is ~4.4× this box's 22
cores, well past where the queue rather than the CPU sets latency.

**Ramp A — the full 31-case mix** (310 requests/step, 3 repeats,
`results/load.json`):

| conc | rps | median ms | p95 ms | p99 ms | error rate | OPA CPU s | OPA peak MB | flags |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1.34 | 32.4 | 7125 | 8677 | 0.0 | 542 | 2097 | rule 3 |
| 2 | 1.02 | 58.0 | 20088 | 20520 | 0.0 | 714 | 2097 | rule 3 |
| 4 | **0.66** | 119.3 | 39178 | 45271 | **0.029** | 1080 | 2097 | rule 3 |

Stop rule fired at concurrency 4: **error rate 0.0290 > 0.0200**. Throughput
*falls* monotonically as concurrency rises — 1.34 → 1.02 → 0.66 rps — which is
congestion collapse, not saturation. The errors are HTTP 500s, and the
service's own log names the cause exactly:

```
jakarta.ws.rs.ProcessingException: The timeout period of 30000ms has been
exceeded while executing PUT /v1/policies/<id> for server localhost:8181
```

Every failure is the PAP's REST client timing out after 30 s on the OPA
policy-module `PUT` or `DELETE`. Concurrent requests serialise on OPA's policy
store while it recompiles a 118 KB policy, so adding clients makes each one
slower than one client would have been.

**Ramp B — the same ramp over the other 28 cases** (`--exclude big-policy`,
560 requests/step, 3 repeats, `results/load-fastband.json`). Ramp A cannot say
anything about the ordinary path because it dies at concurrency 4 on three
outliers; ramp B is what the service does with the work it handles normally:

| conc | rps | median ms | p95 ms | p99 ms | error rate | JVM CPU s | OPA CPU s | JVM peak MB | flags |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 30.77 | 31.9 | 39.4 | 44.2 | 0.0 | 2.02 | 36.3 | 491 | — |
| 2 | **35.55** | 54.3 | 69.7 | 77.2 | 0.0 | 1.97 | 39.0 | 491 | — |
| 4 | 35.11 | 111.9 | 131.0 | 140.8 | 0.0 | 2.77 | 39.8 | 492 | — |
| 8 | 33.29 | 236.8 | 271.7 | 291.9 | 0.0 | 2.92 | 42.2 | 492 | — |
| 16 | 30.70 | 515.4 | 572.3 | 580.5 | 0.0 | 2.64 | 45.4 | 493 | — |
| 32 | 26.64 | 1203.4 | 1283.1 | 1292.8 | 0.0 | 1.85 | 52.6 | 495 | — |

Stop rule fired at concurrency 32: **median 1203.4 ms > 25 × the level-1
median of 34.6 ms**. Zero errors at every level, and neither stability gate
flagged anything — the three repeats agree closely.

Decision correctness is unaffected by concurrency, and the bench checks rather
than assumes it: every level returned exactly **60 wrong decisions out of
1680**, i.e. 1 in 28, which is precisely `testcase-061-violated` — the single
case that also fails at concurrency 1 — recurring once per pass over the mix.
Ramp A likewise: 30 of 930, exactly 1 in 31. Not one additional wrong answer
appeared under load, including in the steps where requests were timing out.

The shape is textbook closed-system queueing. **Throughput peaks at 35.6 rps
at concurrency 2** and then flattens and slowly decays, while median latency
doubles with every doubling of concurrency (54 → 112 → 237 → 515 → 1203 ms).
Past concurrency 2 the extra clients buy nothing and only wait. On a 22-core
box the service uses about two of them, because the serialising resource is
OPA's policy-store write lock, not CPU.

### Outlier and stability gates

Four numeric rules, all stated up front, all **flagging and never dropping**:

| Rule | Definition | Result |
| --- | --- | --- |
| 1 — measurement | Tukey fence, k=1.5, on the run's own pooled distribution | fence [12.1, 54.7] ms; **19 of 155 flagged**, 15 of them the `big-policy` measurements |
| 2 — case | `(max−min)/median > 0.50` across repeats | **14 of 31 cases flagged** |
| 3 — load step | `p99 > 4 × median` for the step | ramp A: levels **1, 2, 4** all flagged; ramp B: **none** |
| 4 — load repeat | throughput `(max−min)/median > 0.25` across repeats | **none** in either ramp |

Two of these need reading carefully rather than at face value.

Rule 1 is doing its job: the fence is derived from a pooled distribution that
is bimodal, so it correctly isolates the `big-policy` band. Those 15 are
flagged as outliers *of the pooled distribution* and are simultaneously the
most reproducible measurements in the run (per-case relative range 0.04–0.27,
none of them flagged by rule 2). They are outliers in the statistical sense
and facts in the engineering sense. Both are reported.

Rule 2's 14 flags are all in the **fast** band, and all of them are a small
absolute swing looking large in relative terms — the worst,
`testcase-026-alice-read-x`, is median 33.3 ms with min 27.2 and max 91.2, an
absolute range of 64 ms. At a 30 ms median, ordinary JVM and OS scheduling
jitter clears a 50 % relative threshold easily. The honest reading is that the
threshold is tight for this engine's scale, not that half the corpus is
unreliable; the flags stand as stated rather than being tuned away after the
fact.

### What this engine is, in one paragraph

`/validate` is not an evaluator call. It is a compile-and-load cycle against a
shared OPA: ~34 ms and ~35 rps ceiling for ordinary policies, ~7.2 s and a 2 GB
OPA for a 118 KB one, with ~95 % of the cost in OPA recompilation and 0.64 ms
in the actual decision. That is a property of the `/validate` *testing*
endpoint, which is what this bench measures because it is the only path that
takes an ODRL policy and returns a decision in one call. A production ODRL-PAP
deployment loads policies once through the bundle endpoint and lets OPA answer
from the compiled bundle, so its steady-state decision latency would be much
closer to the 0.64 ms OPA floor than to anything in these tables. **Read these
numbers as the cost of the ad-hoc validation path, not as ODRL-PAP's
enforcement-time performance.**

### Files

| File | Contents |
| --- | --- |
| `perf_corpus.py` | the shared `/validate` call, the corpus filter, the summariser, the sampler, the gate constants |
| `perf_bench.py` | warmup, 5 timed passes, resource sampling, OPA-floor decomposition |
| `load_bench.py` | the concurrency ramp, both mixes, stop rules and gates 3/4 |
| `run_perf.sh` | clone at the pin → cold build → stack up → conformance re-verify → measure → tear down (EXIT trap, so it cleans up on failure too) |
| `results/perf.json` | every per-case measurement, RSS/CPU series, gate output |
| `results/load.json` | ramp A, full mix |
| `results/load-fastband.json` | ramp B, `big-policy` excluded |
| `results/verify-conformance.json` | the 30/1/37 re-verification against the fresh stack |
| `results/environment.txt` | machine, versions, pre/post load average, the `/validate` error lines |
| `results/footprint.txt` | `du -sh` of deps, build, artifact, images |
| `results/build.time.txt` | `/usr/bin/time -v` of the cold build |
| `results/*.time.txt` | `/usr/bin/time -v` of each bench **client** |

Wall clock of the measured section: perf 2 m 0 s, ramp A 54 m 9 s, ramp B
5 m 36 s. Ramp A is slow for the reason it reports — most of its wall time is
requests waiting on OPA.
