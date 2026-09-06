#!/usr/bin/env python3
"""Concurrency ramp against ODRL-PAP's /validate.

What "concurrency" means for THIS engine
----------------------------------------
ODRL-PAP is the only engine in this bench that is a real HTTP service, so
concurrency here means what it normally means for a service: N client
connections issuing /validate requests at the same time, and the server
deciding for itself how much of that it runs in parallel. It is NOT a
process-fanout trick standing in for parallelism (which is what the
single-threaded interpreters in this bench need), and it is NOT the client's
own parallelism being measured.

Server side, one /validate is handled on a Quarkus worker thread and does, per
request: a JSON-LD compaction, an ODRL->Rego mapping, a PUT of a temp policy
module into OPA, an OPA data query, and a DELETE of that module. So a level-N
step puts N of those pipelines in flight against one JVM and one OPA, and the
first thing to saturate is whichever of those two runs out of worker threads
or CPU first -- that is exactly what the ramp is here to find.

Client side, the generator is a thread pool of blocking urllib callers. Python
threads are the right tool precisely because the work is socket I/O: the GIL
is released for the whole request, so N threads really do keep N requests in
flight. The client's own CPU cost is reported per step so a reader can see the
generator was not itself the bottleneck.

Do not read this as apples-to-apples against the other engines' load numbers.
The others measure an in-process or per-process evaluation; this one measures a
network service including HTTP, JSON, JSON-LD and two extra OPA hops. It is a
different quantity, and the README says so.

Method per step
---------------
For each concurrency level: a short open-loop warm-in (`--per-step-warmup`
requests, discarded, so a step is not charged for the pool's own first-touch),
then `--requests` requests dispatched across the pool, cycling the 31 evaluable
cases so every step evaluates the same mix of work. Latency is measured per
request client-side; throughput is completed requests over the step's own wall
clock; error rate counts anything that did not come back as a scored decision;
decision correctness is re-checked per request so a step that degrades into
returning wrong answers cannot look like a fast step.

`--repeats` (default 3) whole ramps are run back to back. A step's headline
numbers are pooled over its repeats, and rule 4 below is computed across them.

Ceiling
-------
The ramp stops at `--max-concurrency` (default 96) or earlier if a stop rule
fires: error rate over `--error-stop` (default 0.02), or median latency over
`--latency-stop-factor` (default 25) times the level-1 median. 96 is the
justified ceiling for this sandbox: the box has 22 cores and the server-side
pipeline is a JVM plus OPA, so 96 in-flight requests is already >4x the core
count and well past the point where the queue, not the CPU, sets latency. The
run reports nproc/free so the ceiling can be judged.

Gates: rule 3 (perf_corpus.STEP_INSTABILITY) flags a step whose p99 exceeds
that multiple of its own median; rule 4 flags a step whose per-repeat median
throughput spread (max-min)/median exceeds `--repeat-spread` (default 0.25).
Both flag, neither drops.

Usage:
  load_bench.py <out.json> [--levels 1,2,4,...] [--requests N] [--repeats N]
                [--per-step-warmup N] [--max-concurrency N] [--pid-file P]
"""
import json
import statistics
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import perf_corpus as pc

OUT = Path(sys.argv[1]).resolve()
opt = lambda name, default: (sys.argv[sys.argv.index(name) + 1]
                             if name in sys.argv else default)
LEVELS = [int(x) for x in opt("--levels", "1,2,4,8,16,32,64,96").split(",")]
REQUESTS = int(opt("--requests", "310"))
REPEATS = int(opt("--repeats", "3"))
STEP_WARMUP = int(opt("--per-step-warmup", "31"))
MAX_CONC = int(opt("--max-concurrency", "96"))
ERROR_STOP = float(opt("--error-stop", "0.02"))
LAT_STOP = float(opt("--latency-stop-factor", "25"))
REPEAT_SPREAD = float(opt("--repeat-spread", "0.25"))
PID_FILE = opt("--pid-file", "run/pap.pid")
# Substring filter over slugs. perf_bench.py found the three `big-policy`
# fixtures cost ~7-9s each against a ~33ms median for every other case, so a
# ramp over the full mix is really a ramp over those three. `--exclude
# big-policy` runs the same ramp over the other 28, which is the only way to
# see what concurrency does to the ordinary path rather than to the outlier.
EXCLUDE = [x for x in opt("--exclude", "").split(",") if x]

LEVELS = [n for n in LEVELS if n <= MAX_CONC]
cases = [c for c in pc.load_cases()
         if not any(x in c["slug"] for x in EXCLUDE)]

report = {
    "engine": "ODRL-PAP (SEAMWARE/odrl-pap)",
    "commit": "59e45474c910b97f537b8f39c68e2e17ec4243ef",
    "tag": "1.7.0",
    "endpoint": pc.PAP,
    "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    "concurrency_means": "N simultaneous HTTP client connections issuing "
                         "/validate; server-side parallelism is Quarkus's own "
                         "worker pool, not something this harness sets",
    "client": "ThreadPoolExecutor of blocking urllib callers (GIL released "
              "during socket I/O, so N threads = N requests genuinely in flight)",
    "cases_in_mix": len(cases),
    "excluded_slug_substrings": EXCLUDE,
    "levels": LEVELS,
    "requests_per_step": REQUESTS,
    "repeats": REPEATS,
    "per_step_warmup": STEP_WARMUP,
    "ceiling": {"max_concurrency": MAX_CONC,
                "error_stop_rate": ERROR_STOP,
                "latency_stop_factor": LAT_STOP},
}


def fire(n_conc, n_req):
    """Dispatch n_req /validate calls across n_conc threads. Returns rows +
    wall clock + the client's own CPU seconds for the step."""
    import os
    work = [cases[i % len(cases)] for i in range(n_req)]

    def one(case):
        ms, dec, err = pc.validate(case)
        return {"slug": case["slug"], "ms": round(ms, 3), "error": err,
                "wrong": (err is None and dec != case["expected"])}

    c0 = os.times()
    t0 = time.perf_counter()
    with ThreadPoolExecutor(max_workers=n_conc) as ex:
        rows = list(ex.map(one, work))
    wall = time.perf_counter() - t0
    c1 = os.times()
    client_cpu = (c1.user - c0.user) + (c1.system - c0.system)
    return rows, wall, client_cpu


sampler = pc.Sampler(pc.pids_from_files(PID_FILE), interval=0.25).start()

steps = {}          # level -> list of per-repeat step records
stopped_at = None
stop_reason = None
level1_median = None

for rep in range(REPEATS):
    for n in LEVELS:
        if stopped_at is not None and n > stopped_at:
            continue
        fire(n, STEP_WARMUP)                      # warm-in, discarded
        rss_before = sampler.report()["trees"]
        rows, wall, client_cpu = fire(n, REQUESTS)
        rss_after = sampler.report()["trees"]

        ok = [r["ms"] for r in rows if r["error"] is None]
        errs = [r for r in rows if r["error"] is not None]
        wrong = [r for r in rows if r["wrong"]]
        summ = pc.summarise(ok)
        rec = {
            "concurrency": n,
            "repeat": rep,
            "requests": len(rows),
            "completed": len(ok),
            "errors": len(errs),
            "error_rate": round(len(errs) / len(rows), 5),
            "wrong_decisions": len(wrong),
            "wall_s": round(wall, 4),
            "throughput_rps": round(len(ok) / wall, 2) if wall else None,
            "latency_ms": summ,
            "client_cpu_s": round(client_cpu, 3),
            "server_cpu_s": {
                lbl: round(rss_after[lbl]["cpu_end_s"] - rss_before[lbl]["cpu_end_s"], 3)
                for lbl in rss_after if lbl in rss_before
            },
            "server_rss_peak_mb": {
                lbl: rss_after[lbl]["rss_peak_kb"] // 1024 for lbl in rss_after
            },
            "error_samples": [e["error"][:200] for e in errs[:5]],
        }
        steps.setdefault(n, []).append(rec)

        if n == 1 and level1_median is None and summ:
            level1_median = summ["median"]
        print("  rep%d conc=%-3d rps=%-8s median=%-8s p95=%-8s p99=%-8s err=%s wrong=%d"
              % (rep, n, rec["throughput_rps"], summ["median"] if summ else "-",
                 summ["p95"] if summ else "-", summ["p99"] if summ else "-",
                 rec["error_rate"], len(wrong)), flush=True)

        fired = None
        if rec["error_rate"] > ERROR_STOP:
            fired = ("error rate %.4f > %.4f at concurrency %d"
                     % (rec["error_rate"], ERROR_STOP, n))
        elif (summ and level1_median
              and summ["median"] > LAT_STOP * level1_median):
            fired = ("median %.1fms > %sx level-1 median %.1fms at concurrency %d"
                     % (summ["median"], LAT_STOP, level1_median, n))
        if fired and stopped_at is None:
            stopped_at, stop_reason = n, fired
            print("  STOP RULE FIRED: %s -- ceiling for the remaining repeats"
                  % fired, flush=True)

resources = resources_after = sampler.stop()
report["resources_whole_ramp"] = resources
report["resources_whole_ramp"]["tool"] = (
    "psutil %s Sampler (perf_corpus.Sampler), 0.25s interval, watching the "
    "Quarkus JVM / OPA / Postgres trees from outside for the whole ramp"
    % __import__("psutil").__version__)

# --- pool each level over its repeats, apply gates 3 and 4 ------------------
table = []
for n in LEVELS:
    recs = steps.get(n)
    if not recs:
        continue
    pooled_lat = []
    for r in recs:
        pooled_lat.append(r["latency_ms"])
    rps = [r["throughput_rps"] for r in recs if r["throughput_rps"]]
    med = [p["median"] for p in pooled_lat if p]
    p95 = [p["p95"] for p in pooled_lat if p]
    p99 = [p["p99"] for p in pooled_lat if p]
    med_med = statistics.median(med) if med else None
    p99_med = statistics.median(p99) if p99 else None
    rps_med = statistics.median(rps) if rps else None
    rps_spread = ((max(rps) - min(rps)) / rps_med) if rps and rps_med else 0.0
    row = {
        "concurrency": n,
        "repeats": len(recs),
        "throughput_rps_median": round(rps_med, 2) if rps_med else None,
        "throughput_rps_per_repeat": rps,
        "latency_median_ms": round(med_med, 3) if med_med else None,
        "latency_p95_ms": round(statistics.median(p95), 3) if p95 else None,
        "latency_p99_ms": round(p99_med, 3) if p99_med else None,
        "latency_max_ms": max((p["max"] for p in pooled_lat if p), default=None),
        "error_rate": round(statistics.median(r["error_rate"] for r in recs), 5),
        "wrong_decisions": sum(r["wrong_decisions"] for r in recs),
        "client_cpu_s_median": round(statistics.median(
            r["client_cpu_s"] for r in recs), 3),
        "server_cpu_s_median": {
            lbl: round(statistics.median(r["server_cpu_s"].get(lbl, 0)
                                         for r in recs), 3)
            for lbl in recs[0]["server_cpu_s"]
        },
        "server_rss_peak_mb": {
            lbl: max(r["server_rss_peak_mb"].get(lbl, 0) for r in recs)
            for lbl in recs[0]["server_rss_peak_mb"]
        },
        "rule_3_p99_over_median": round(p99_med / med_med, 3) if med_med else None,
        "rule_3_unstable": bool(med_med and p99_med
                                and p99_med / med_med > pc.STEP_INSTABILITY),
        "rule_4_rps_repeat_spread": round(rps_spread, 4),
        "rule_4_unstable": rps_spread > REPEAT_SPREAD,
    }
    table.append(row)

report["steps_raw"] = [r for n in LEVELS for r in steps.get(n, [])]
report["table"] = table
report["stopped_at"] = stopped_at
report["stop_reason"] = stop_reason
best = max((r for r in table if r["throughput_rps_median"]),
           key=lambda r: r["throughput_rps_median"], default=None)
report["peak"] = (
    {"concurrency": best["concurrency"],
     "throughput_rps": best["throughput_rps_median"],
     "latency_median_ms": best["latency_median_ms"],
     "latency_p99_ms": best["latency_p99_ms"]} if best else None)
report["gates"] = {
    "rule_3_step_instability": {
        "threshold_p99_over_median": pc.STEP_INSTABILITY,
        "unstable_levels": [r["concurrency"] for r in table if r["rule_3_unstable"]],
    },
    "rule_4_repeat_spread": {
        "threshold_rel_range_of_rps": REPEAT_SPREAD,
        "unstable_levels": [r["concurrency"] for r in table if r["rule_4_unstable"]],
    },
}
report["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
OUT.write_text(json.dumps(report, indent=2))

print("\n conc |    rps | median |    p95 |    p99 |  err | jvm cpu s | jvm rss MB | flags")
for r in table:
    flags = ",".join(f for f, on in (("p99", r["rule_3_unstable"]),
                                     ("rps-spread", r["rule_4_unstable"])) if on)
    print(" %4d | %6s | %6s | %6s | %6s | %4s | %9s | %10s | %s"
          % (r["concurrency"], r["throughput_rps_median"], r["latency_median_ms"],
             r["latency_p95_ms"], r["latency_p99_ms"], r["error_rate"],
             r["server_cpu_s_median"].get("quarkus"),
             r["server_rss_peak_mb"].get("quarkus"), flags or "-"))
if report["peak"]:
    print("\npeak throughput: %(throughput_rps)s rps at concurrency "
          "%(concurrency)s (median %(latency_median_ms)sms, p99 %(latency_p99_ms)sms)"
          % report["peak"])
print("stop rule: %s" % (stop_reason or "none fired; ran to the stated ceiling"))
print("-> %s" % OUT)
