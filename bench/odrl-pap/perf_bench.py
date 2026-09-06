#!/usr/bin/env python3
"""Single-client performance + resource measurement for ODRL-PAP
(SEAMWARE/odrl-pap 1.7.0), over the same corpus and through the same
/validate call path run_pap.py scores 30/1/37 with.

Sequence:

  1. WARMUP: `--warmup` (default 20) /validate calls, cycling the first 10
     evaluable cases, discarded. This engine needs it more than any other in
     the bench -- it is a JVM, so the first calls pay C1/C2 JIT compilation,
     and on top of that the JSON-LD handler caches remote @context documents
     on first use. Warmup doubles as the smoke test: any error during warmup
     aborts the run rather than letting a half-up stack produce numbers.
  2. TIMED: `--repeats` (default 5) full passes over the 31 evaluable cases,
     per-case, in corpus order, one request at a time (concurrency 1). The
     decision is re-checked against the fixture's expectation on every call,
     so a run that silently stopped agreeing with the conformance result
     cannot pass itself off as a clean latency number.
  3. RESOURCES: a psutil sampler (perf_corpus.Sampler) watches the Quarkus
     JVM, the OPA container and the Postgres container from outside for the
     whole timed section. /usr/bin/time -v cannot be used on the thing being
     measured here -- the server outlives every measurement -- so RSS and CPU
     seconds are read from the live process trees instead.
  4. DECOMPOSITION: `--opa-probe` (default 200) bare OPA data-API round trips,
     no PAP in the path. One /validate is three OPA round trips (PUT module,
     query, DELETE module) plus the PAP's own JSON-LD + mapping work, so this
     says how much of the measured latency is unavoidable OPA/HTTP floor and
     how much the PAP adds.

Gates: perf_corpus.TUKEY_K (rule 1, per measurement) and
perf_corpus.CASE_INSTABILITY (rule 2, per case) are applied to the output and
flag measurements. Nothing is dropped.

Usage: perf_bench.py <out.json> [--repeats N] [--warmup N] [--opa-probe N]
"""
import json
import statistics
import sys
import time
from pathlib import Path

import perf_corpus as pc

OUT = Path(sys.argv[1]).resolve()
arg = lambda name, default: (int(sys.argv[sys.argv.index(name) + 1])
                             if name in sys.argv else default)
REPEATS = arg("--repeats", 5)
WARMUP = arg("--warmup", 20)
OPA_PROBE = arg("--opa-probe", 200)
PID_FILE = (sys.argv[sys.argv.index("--pid-file") + 1]
            if "--pid-file" in sys.argv else "run/pap.pid")

report = {
    "engine": "ODRL-PAP (SEAMWARE/odrl-pap)",
    "commit": "59e45474c910b97f537b8f39c68e2e17ec4243ef",
    "tag": "1.7.0",
    "endpoint": pc.PAP,
    "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    "python": sys.version.split()[0],
    "concurrency": 1,
    "repeats": REPEATS,
    "warmup_iterations": WARMUP,
}

cases = pc.load_cases()
report["cases_evaluable"] = len(cases)

# --- 1. warmup / smoke test -------------------------------------------------
warm = []
for i in range(WARMUP):
    c = cases[i % min(10, len(cases))]
    ms, dec, err = pc.validate(c)
    if err is not None:
        report["warmup_error"] = {"case": c["slug"], "error": err}
        OUT.write_text(json.dumps(report, indent=2))
        sys.exit("WARMUP FAILED on %s: %s -- refusing to report timings"
                 % (c["slug"], err))
    warm.append(round(ms, 3))
report["warmup_ms"] = warm
report["warmup_first_ms"] = warm[0]
report["warmup_last5_median_ms"] = round(statistics.median(warm[-5:]), 3)

# --- 2/3. timed passes, watched from outside --------------------------------
sampler = pc.Sampler(pc.pids_from_files(PID_FILE), interval=0.25).start()

samples = []
per_repeat = []
for rep in range(REPEATS):
    t_rep = time.perf_counter()
    mism = errs = 0
    for c in cases:
        ms, dec, err = pc.validate(c)
        if err is not None:
            errs += 1
        elif dec != c["expected"]:
            mism += 1
        samples.append({"repeat": rep, "slug": c["slug"], "ms": round(ms, 3),
                        "decision": dec, "expected": c["expected"], "error": err})
    per_repeat.append({
        "repeat": rep,
        "wall_ms": round((time.perf_counter() - t_rep) * 1000, 2),
        "mismatches": mism,
        "errors": errs,
    })

resources = sampler.stop()
report["per_repeat"] = per_repeat
report["samples"] = samples
report["resources"] = resources
report["resources"]["tool"] = (
    "psutil %s Sampler (perf_corpus.Sampler): RSS + cpu_times of each process "
    "tree, sampled every %ss for the whole timed section, from outside the "
    "server processes" % (__import__("psutil").__version__, resources["interval_s"])
)
report["resource_series"] = sampler.raw()

vals = [s["ms"] for s in samples if s["error"] is None]
report["latency_ms"] = pc.summarise(vals)

# per-case aggregation + stability gate (rule 2)
per_case = []
for c in cases:
    cv = [s["ms"] for s in samples if s["slug"] == c["slug"] and s["error"] is None]
    if not cv:
        continue
    med = statistics.median(cv)
    rel = (max(cv) - min(cv)) / med if med else 0.0
    per_case.append({
        "slug": c["slug"],
        "n": len(cv),
        "median_ms": round(med, 3),
        "min_ms": round(min(cv), 3),
        "max_ms": round(max(cv), 3),
        "rel_range": round(rel, 4),
        "unstable": rel > pc.CASE_INSTABILITY,
        "ms": [round(v, 3) for v in cv],
    })
report["per_case"] = per_case

# per-measurement gate (rule 1)
fence, flagged = pc.tukey_flags(samples, report["latency_ms"])
report["gates"] = {
    "rule_1_tukey": {
        "k": pc.TUKEY_K, "fence_ms": list(fence),
        "flagged": len(flagged), "of": len(vals),
        "measurements": [{"repeat": s["repeat"], "slug": s["slug"], "ms": s["ms"]}
                         for s in flagged],
    },
    "rule_2_case_instability": {
        "threshold_rel_range": pc.CASE_INSTABILITY,
        "unstable_cases": [c["slug"] for c in per_case if c["unstable"]],
        "worst_rel_range": max((c["rel_range"] for c in per_case), default=None),
    },
}

# --- 4. OPA floor decomposition ---------------------------------------------
opa_ms, opa_err = [], 0
for _ in range(OPA_PROBE):
    ms, err = pc.opa_roundtrip()
    if err:
        opa_err += 1
    else:
        opa_ms.append(ms)
opa_summ = pc.summarise(opa_ms)
med = report["latency_ms"]["median"]
report["opa_floor"] = {
    "n": OPA_PROBE,
    "errors": opa_err,
    "note": "bare OPA /v1/data round trip, PAP not in the path; one /validate "
            "makes three such round trips (PUT module, query, DELETE module)",
    "latency_ms": opa_summ,
    "three_roundtrips_median_ms": round(3 * opa_summ["median"], 3) if opa_summ else None,
    "share_of_validate_median": (round(3 * opa_summ["median"] / med, 4)
                                 if opa_summ and med else None),
}

report["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
OUT.write_text(json.dumps(report, indent=2))

s = report["latency_ms"]
print("cases=%d repeats=%d measurements=%d errors=%d mismatches=%d"
      % (len(cases), REPEATS, len(vals),
         sum(r["errors"] for r in per_repeat),
         sum(r["mismatches"] for r in per_repeat)))
print("  warmup: first=%.1fms last5_median=%.1fms (JIT + JSON-LD context cache)"
      % (report["warmup_first_ms"], report["warmup_last5_median_ms"]))
print("  latency ms: " + " ".join("%s=%s" % (k, s[k]) for k in
                                  ("mean", "median", "p95", "p99", "min", "max", "stddev")))
print("  tukey[%s] fence=[%s,%s] flagged=%d/%d"
      % (pc.TUKEY_K, fence[0], fence[1], len(flagged), len(vals)))
print("  unstable cases (rel_range>%s): %s"
      % (pc.CASE_INSTABILITY,
         [c["slug"] for c in per_case if c["unstable"]] or "none"))
for label, t in resources["trees"].items():
    print("  rss %-9s first=%dMB median=%dMB peak=%dMB cpu_consumed=%.2fs"
          % (label, t["rss_first_kb"] // 1024, t["rss_median_kb"] // 1024,
             t["rss_peak_kb"] // 1024, t["cpu_consumed_s"]))
print("  opa floor: median=%.2fms x3=%.2fms = %.1f%% of a /validate"
      % (opa_summ["median"], 3 * opa_summ["median"],
         100 * report["opa_floor"]["share_of_validate_median"]))
print("-> %s" % OUT)
