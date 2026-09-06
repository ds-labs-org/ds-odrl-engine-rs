#!/usr/bin/env python3
"""Single-process performance + in-process resource measurement for OVAL
(DIPS-Tools/odrl-Engine), over the same 68-fixture corpus and through the same
entrypoint bench.py scores 59/68 with.

Sequence, in one process:

  0. interpreter/import cost -- `import ODRL_Evaluator` pulls rdflib + pyshacl +
     owlrl; for anything invoked per-request as a CLI that is paid every time,
     so it is timed separately rather than hidden inside the first evaluation.
  1. WARMUP: `--warmup` (default 10) evaluations, cycling testcases 001-010,
     discarded. Doubles as the smoke test: any error during warmup aborts the
     run instead of letting a broken engine produce numbers.
  2. TIMED: `--repeats` (default 5) full passes over the corpus, per-case, in
     ground_truth.json order. `--isolate/--no-isolate` picks which of the two
     paths bench.py documents is being measured; both are measured by the
     driver, in separate processes, because the non-isolated one deliberately
     accumulates state and must not contaminate the clean numbers.
  3. RSS series: /proc/self/status VmRSS sampled after EVERY timed evaluation,
     so steady-state and growth are visible, not just the peak `time -v` reports
     from outside.
  4. GIL probe: N evaluations sequentially vs the same N through a
     ThreadPoolExecutor. This is the evidence for the load bench's choice of OS
     processes as its unit of concurrency -- it is measured here, not assumed.

`--gc-off` calls gc.disable() after warmup and before the timed passes. It
exists because the default run's own output raised a question it could answer:
per-case latency is bimodal in a strictly periodic way (roughly every 4th-5th
evaluation costs ~1.65x the rest), which is the signature of CPython's
generational collector walking the object graph rdflib leaves behind rather
than anything the reasoner does. Running the identical corpus with the cyclic
collector off is the controlled test of that, so the effect can be reported as
diagnosed instead of merely observed. gc.get_stats() is recorded either way.

Outlier/stability gates (perf_corpus.TUKEY_K, CASE_INSTABILITY) are applied to
the output and flag measurements; nothing is dropped.

Usage: perf_bench.py <out.json> [--repeats N] [--warmup N] [--no-isolate] [--gc-off]
"""
import gc
import json
import statistics
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import perf_corpus as pc

OUT = Path(sys.argv[1]).resolve()
REPEATS = int(sys.argv[sys.argv.index("--repeats") + 1]) if "--repeats" in sys.argv else 5
WARMUP = int(sys.argv[sys.argv.index("--warmup") + 1]) if "--warmup" in sys.argv else 10
ISOLATE = "--no-isolate" not in sys.argv
GC_OFF = "--gc-off" in sys.argv
GIL_N = 4

report = {
    "engine": "OVAL (DIPS-Tools/odrl-Engine)",
    "commit": "a427e71b50bdd14027f2d5552d6ce03d089487f3",
    "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    "python": sys.version,
    "isolate": ISOLATE,
    "gc_disabled": GC_OFF,
    "gc_thresholds": gc.get_threshold(),
    "repeats": REPEATS,
    "warmup_iterations": WARMUP,
}

cases = pc.load_cases()
report["cases"] = len(cases)

# --- 0. import cost ---------------------------------------------------------
t0 = time.perf_counter()
engine = pc.attach()
report["import_ms"] = round((time.perf_counter() - t0) * 1000, 3)
report["rss_after_import_kb"] = pc.rss_kb()

# --- 1. warmup / smoke test -------------------------------------------------
warm = []
for i in range(WARMUP):
    c = cases[i % min(10, len(cases))]
    ms, dec, err = pc.evaluate(engine, c, isolate=ISOLATE)
    if err is not None:
        report["warmup_error"] = {"case": c["slug"], "error": err}
        OUT.write_text(json.dumps(report, indent=2))
        sys.exit(f"WARMUP FAILED on {c['slug']}: {err} -- refusing to report timings")
    warm.append(round(ms, 3))
report["warmup_ms"] = warm
report["rss_after_warmup_kb"] = pc.rss_kb()

# --- 2/3. timed passes + RSS series ----------------------------------------
if GC_OFF:
    gc.collect()
    gc.disable()
gc_before = gc.get_stats()  # after the settling collect, so the delta is the run's own
samples = []     # every timed measurement
rss_series = []  # one VmRSS reading per timed measurement
per_repeat = []
for rep in range(REPEATS):
    t_rep = time.perf_counter()
    mism = errs = 0
    for c in cases:
        ms, dec, err = pc.evaluate(engine, c, isolate=ISOLATE)
        rss_series.append(pc.rss_kb())
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
        "rss_end_kb": pc.rss_kb(),
    })
report["per_repeat"] = per_repeat
report["samples"] = samples
gc_after = gc.get_stats()
report["gc"] = {
    "disabled_for_timed_passes": GC_OFF,
    "enabled_at_end": gc.isenabled(),
    "collections_during_timed_passes": [
        a["collections"] - b["collections"] for a, b in zip(gc_after, gc_before)
    ],
    "collected_during_timed_passes": [
        a["collected"] - b["collected"] for a, b in zip(gc_after, gc_before)
    ],
}

vals = [s["ms"] for s in samples if s["error"] is None]
report["latency_ms"] = pc.summarise(vals)
report["memory_in_process"] = {
    "tool": "/proc/self/status VmRSS, read after every timed evaluation",
    "samples": len(rss_series),
    "first_kb": rss_series[0] if rss_series else None,
    "median_kb": int(statistics.median(rss_series)) if rss_series else None,
    "max_kb": max(rss_series) if rss_series else None,
    "last_kb": rss_series[-1] if rss_series else None,
    "growth_kb": (rss_series[-1] - rss_series[0]) if rss_series else None,
    "series_kb": rss_series,
}

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
        "heavy": c["slug"] in pc.HEAVY,
        "ms": [round(v, 3) for v in cv],
    })
report["per_case"] = per_case

# Bimodality of the light (non-big-policy) measurements. Stated rule: a light
# measurement is "slow band" when it exceeds 1.4x the light median. `gaps` is
# the spacing, in evaluations, between consecutive slow-band measurements --
# a tight gap distribution means the spike is periodic (a runtime event) rather
# than case-specific (a property of a fixture).
light = [(i, s) for i, s in enumerate(samples)
         if s["error"] is None and s["slug"] not in pc.HEAVY]
if light:
    lmed = statistics.median(s["ms"] for _, s in light)
    cut = 1.4 * lmed
    slow_idx = [i for i, s in light if s["ms"] > cut]
    fast = [s["ms"] for _, s in light if s["ms"] <= cut]
    slow = [s["ms"] for _, s in light if s["ms"] > cut]
    gaps = [b - a for a, b in zip(slow_idx, slow_idx[1:])]
    report["bimodality"] = {
        "rule": "light measurement is slow-band when ms > 1.4 * light median",
        "light_n": len(light),
        "light_median_ms": round(lmed, 3),
        "cut_ms": round(cut, 3),
        "slow_band_n": len(slow),
        "slow_band_share": round(len(slow) / len(light), 4),
        "fast_band_median_ms": round(statistics.median(fast), 3) if fast else None,
        "slow_band_median_ms": round(statistics.median(slow), 3) if slow else None,
        "ratio": round(statistics.median(slow) / statistics.median(fast), 3) if slow and fast else None,
        "gap_median": statistics.median(gaps) if gaps else None,
        "gap_min": min(gaps) if gaps else None,
        "gap_max": max(gaps) if gaps else None,
    }

# per-measurement gate (rule 1): Tukey fence on the pooled distribution
summ = report["latency_ms"]
lo = summ["q1"] - pc.TUKEY_K * summ["iqr"]
hi = summ["q3"] + pc.TUKEY_K * summ["iqr"]
flagged = [{"repeat": s["repeat"], "slug": s["slug"], "ms": s["ms"]}
           for s in samples if s["error"] is None and not (lo <= s["ms"] <= hi)]
report["gates"] = {
    "rule_1_tukey": {"k": pc.TUKEY_K, "fence_ms": [round(lo, 3), round(hi, 3)],
                     "flagged": len(flagged), "of": len(vals), "measurements": flagged},
    "rule_2_case_instability": {
        "threshold_rel_range": pc.CASE_INSTABILITY,
        "unstable_cases": [c["slug"] for c in per_case if c["unstable"]],
        "worst_rel_range": max((c["rel_range"] for c in per_case), default=None),
    },
}

# --- 4. GIL probe -----------------------------------------------------------
# Same N evaluations, sequential vs ThreadPoolExecutor. A speedup near 1.0 means
# in-process threading buys nothing and real concurrency needs OS processes.
probe_cases = cases[:GIL_N]
t = time.perf_counter()
for c in probe_cases:
    pc.evaluate(engine, c, isolate=ISOLATE)
seq_ms = (time.perf_counter() - t) * 1000
t = time.perf_counter()
with ThreadPoolExecutor(max_workers=GIL_N) as ex:
    list(ex.map(lambda c: pc.evaluate(engine, c, isolate=ISOLATE), probe_cases))
thr_ms = (time.perf_counter() - t) * 1000
report["gil_probe"] = {
    "n": GIL_N,
    "sequential_ms": round(seq_ms, 2),
    "threadpool_ms": round(thr_ms, 2),
    "speedup": round(seq_ms / thr_ms, 3) if thr_ms else None,
    "note": "isolate=%s; a speedup near 1.0 is the GIL, not a measurement error" % ISOLATE,
}

report["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
OUT.write_text(json.dumps(report, indent=2))

bim = report.get("bimodality") or {}
print(f"isolate={ISOLATE} gc_off={GC_OFF} cases={len(cases)} repeats={REPEATS} "
      f"measurements={len(vals)}")
print(f"  bimodality: slow band {bim.get('slow_band_n')}/{bim.get('light_n')} "
      f"({bim.get('slow_band_share')}) fast={bim.get('fast_band_median_ms')}ms "
      f"slow={bim.get('slow_band_median_ms')}ms ratio={bim.get('ratio')} "
      f"gap median/min/max={bim.get('gap_median')}/{bim.get('gap_min')}/{bim.get('gap_max')}")
print(f"  gc collections during timed passes: {report['gc']['collections_during_timed_passes']}")
print("  latency ms: " + " ".join(f"{k}={summ[k]}" for k in
                                  ("mean", "median", "p95", "p99", "min", "max", "stddev")))
print(f"  tukey[{pc.TUKEY_K}] fence=[{lo:.1f},{hi:.1f}] flagged={len(flagged)}/{len(vals)}")
print(f"  unstable cases (rel_range>{pc.CASE_INSTABILITY}): "
      f"{[c['slug'] for c in per_case if c['unstable']] or 'none'}")
print(f"  rss kb first={report['memory_in_process']['first_kb']} "
      f"median={report['memory_in_process']['median_kb']} "
      f"last={report['memory_in_process']['last_kb']}")
print(f"  gil probe {GIL_N} seq={seq_ms:.1f}ms threads={thr_ms:.1f}ms "
      f"speedup={report['gil_probe']['speedup']}")
print(f"  mismatches/repeat={[r['mismatches'] for r in per_repeat]} "
      f"errors/repeat={[r['errors'] for r in per_repeat]}")
print(f"-> {OUT}")
