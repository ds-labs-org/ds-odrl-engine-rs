#!/usr/bin/env python3
"""Shared plumbing for the perf/resource/load instrumentation (perf_bench.py,
load_bench.py, load_worker.py).

Nothing here is new evaluation logic. `load_cases()` and `evaluate()` are
bench.py's own corpus resolution and engine call lifted verbatim into functions
so the timed measurements go through the identical path the 59/68 conformance
number came from -- same entrypoint, same `normalise=False`, same
`r[1] == 1 -> Allow` reduction, same cwd, same `--isolate` keyword trick.

Both environment facts bench.py's header documents apply here unchanged and are
enforced by `attach()`:

1. ODRL_Evaluator.py:793 appends the *cwd-relative* path "ODRL/ODRL22.ttl". Run
   from anywhere else, the ontology is silently absent and the action-taxonomy
   fixtures flip. We chdir to the repo root and assert the file resolves.
2. `evaluate_ODRL_from_files` carries MUTABLE DEFAULT arguments
   `ontology_files=[], ontology_graphs=[]` and appends to them on every call, so
   in a loop each earlier policy graph leaks into every later evaluation.
   `isolate=True` passes fresh lists per call; `isolate=False` reproduces
   upstream's own test_on_force.py conditions verbatim. The perf pass measures
   BOTH, because that leak is a performance fact, not only a correctness one.

RSS is read from /proc/self/status rather than psutil on purpose: psutil is not
in OVAL's requirements.txt, and installing it would change the dependency tree
this same pass reports the on-disk footprint of.
"""
import json
import os
import sys
import time
from pathlib import Path

BASE = Path(__file__).resolve().parent
REPO = BASE / "odrl-Engine"
CORPUS = REPO / "test_cases" / "evaluation" / "force"

# The three fixtures the conformance run already showed to be the expensive
# ones. Named here only so the load ramp can report a "composition" column --
# nothing is ever excluded from a measurement on account of being in this list.
HEAVY = ("testcase-062-big-policy", "testcase-063-big-policy-OoO", "testcase-064-big-policy-past")


def load_cases(corpus=CORPUS):
    """bench.py's case list: ground_truth.json order, restricted to slugs that
    actually have an extracted_<slug>.ttl/.csv pair in the corpus."""
    cases = []
    for c in json.loads((BASE / "ground_truth.json").read_text()):
        ttl = Path(corpus) / f"extracted_{c['slug']}.ttl"
        csvf = Path(corpus) / f"extracted_{c['slug']}.csv"
        if not ttl.exists() or not csvf.exists():
            continue
        cases.append(
            {
                "slug": c["slug"],
                "expected": c["expected_decision"],
                "ttl": str(ttl.resolve()),
                "csv": str(csvf.resolve()),
            }
        )
    return cases


def attach():
    """chdir into the repo root, import upstream, return the module. See note 1."""
    sys.path.insert(0, str(REPO))
    os.chdir(REPO)
    import ODRL_Evaluator  # noqa: E402

    assert Path("ODRL/ODRL22.ttl").exists(), "ODRL 2.2 ontology not resolvable from cwd"
    return ODRL_Evaluator


def evaluate(engine, case, isolate=True):
    """One timed evaluation. Returns (elapsed_ms, decision_or_None, error_or_None).

    The timed region is exactly bench.py's: the single upstream call. Turtle
    parsing is inside it -- upstream exposes no pre-parsed entrypoint, so unlike
    a library with a separate parse step there is no second path to time here.
    """
    kw = {"ontology_files": [], "ontology_graphs": []} if isolate else {}
    t0 = time.perf_counter()
    try:
        r = engine.evaluate_ODRL_from_files(case["ttl"], case["csv"], normalise=False, **kw)
    except Exception as e:  # noqa: BLE001 - an engine error is a real result, not a skip
        return (time.perf_counter() - t0) * 1000.0, None, f"{type(e).__name__}: {e}"
    return (time.perf_counter() - t0) * 1000.0, ("Allow" if r[1] == 1 else "Deny"), None


def rss_kb(pid="self"):
    """Kernel-reported resident set size, kB. None if the pid is gone."""
    try:
        with open(f"/proc/{pid}/status") as fh:
            for line in fh:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
    except (OSError, ValueError):
        return None
    return None


def cpu_seconds(pid):
    """utime+stime of a pid in seconds, from /proc/<pid>/stat. None if gone."""
    try:
        with open(f"/proc/{pid}/stat") as fh:
            fields = fh.read().rsplit(") ", 1)[1].split()
    except (OSError, IndexError):
        return None
    ticks = os.sysconf("SC_CLK_TCK")
    return (int(fields[11]) + int(fields[12])) / ticks


def mem_available_mb():
    with open("/proc/meminfo") as fh:
        for line in fh:
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) / 1024.0
    return None


# --- statistics -------------------------------------------------------------

def pct(sorted_vals, q):
    """Nearest-rank percentile over an already-sorted list."""
    if not sorted_vals:
        return None
    i = max(0, min(len(sorted_vals) - 1, int(round(q * (len(sorted_vals) - 1)))))
    return sorted_vals[i]


def summarise(vals):
    """mean/median/p95/p99/min/max/stddev/IQR over a list of numbers."""
    if not vals:
        return None
    s = sorted(vals)
    n = len(s)
    mean = sum(s) / n
    var = sum((v - mean) ** 2 for v in s) / n if n > 1 else 0.0
    q1, q3 = pct(s, 0.25), pct(s, 0.75)
    return {
        "n": n,
        "mean": round(mean, 3),
        "median": round(pct(s, 0.5), 3),
        "p95": round(pct(s, 0.95), 3),
        "p99": round(pct(s, 0.99), 3),
        "min": round(s[0], 3),
        "max": round(s[-1], 3),
        "stddev": round(var ** 0.5, 3),
        "q1": round(q1, 3),
        "q3": round(q3, 3),
        "iqr": round(q3 - q1, 3),
    }


# Outlier / stability gate constants -- stated numeric rules, applied by
# perf_bench.py and load_bench.py, and never used to discard a measurement.
TUKEY_K = 1.5           # rule 1: per-measurement fence on the pooled distribution
CASE_INSTABILITY = 0.25  # rule 2: per-case (max-min)/median across repeats
LEVEL_INSTABILITY = 0.15  # rule 3: per-level throughput (max-min)/median across ramp repeats
