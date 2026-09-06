#!/usr/bin/env python3
"""Concurrency ramp for OVAL (DIPS-Tools/odrl-Engine).

WHAT "CONCURRENCY" MEANS FOR THIS ENGINE -- do not read it as apples-to-apples
against another engine's number. OVAL is a Python library, not a service: there
is no port, no request rate, nothing to stand up. Its evaluation is CPU-bound
RDF reasoning inside one interpreter, so Python's GIL makes in-process threading
not parallelism at all (perf_bench.py's `gil_probe` measures exactly that rather
than asserting it). Therefore:

    ONE UNIT OF CONCURRENCY = ONE OS PROCESS.

Specifically `multiprocessing` with the **"spawn"** start method, not fork and
not threads: each worker is a fresh CPython interpreter that does its own
`import ODRL_Evaluator`, its own ontology parse and its own warmup, so nothing
is shared copy-on-write and no worker's rdflib state can influence another's.
The alternative (`subprocess` invocations of bench.py) would have re-paid the
~1 s import cost on every evaluation and measured process startup instead of the
engine; the persistent-worker shape is the one that isolates the engine.

Method: the pool is spawned ONCE per ramp repeat at the maximum level; at each
level only the first `c` workers are told to "go" and the rest block in recv()
-- memory resident, zero CPU. Per step the host samples /proc/<pid>/status
VmRSS and /proc/<pid>/stat utime+stime for the ACTIVE workers only, every
SAMPLE_MS, so RSS and CPU are kernel-reported rather than self-reported.

A memory guard reads MemAvailable from /proc/meminfo before each pool and each
step and skips the step (recording why) below MIN_FREE_MB, because this is a
shared, swapless box.

Stability gate (rule 3, perf_corpus.LEVEL_INSTABILITY): a level whose
throughput varies by more than the threshold across ramp repeats is flagged
`unstable_across_repeats`. Flagged, never dropped.

Usage: load_bench.py <out.json> [--levels 1,2,4,...] [--step-s 10]
                                [--repeats 3] [--no-isolate]
"""
import json
import multiprocessing as mp
import signal
import statistics
import sys
import time
from pathlib import Path

import perf_corpus as pc
import load_worker


def arg(name, default, cast=str):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


# NOTE: with the "spawn" start method multiprocessing re-imports this module in
# every child (as __mp_main__) after restoring the parent's sys.argv, so this
# module-level parsing runs there too. It is kept argv-tolerant for that reason,
# and everything with a side effect (signal handlers, the pool) lives in main().
OUT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 and not sys.argv[1].startswith("-") \
    else Path("load.json").resolve()
LEVELS = [int(x) for x in arg("--levels", "1,2,4,8,16,22,32").split(",")]
STEP_S = arg("--step-s", 10.0, float)
RAMP_REPEATS = arg("--repeats", 3, int)
ISOLATE = "--no-isolate" not in sys.argv
SAMPLE_MS = 250
MIN_FREE_MB = 6144
READY_TIMEOUT_S = 180
STEP_SLACK_S = 120

MAXC = max(LEVELS)
_pool = []


def kill_pool():
    for proc, conn in _pool:
        try:
            conn.close()
        except Exception:  # noqa: BLE001
            pass
        if proc.is_alive():
            proc.terminate()
    for proc, _ in _pool:
        proc.join(timeout=10)
        if proc.is_alive():
            proc.kill()
    _pool.clear()


def _bail(signum, _frame):
    kill_pool()
    sys.exit(f"load_bench: caught signal {signum}, pool killed")


def spawn_pool(ctx, n):
    for i in range(n):
        host_conn, child_conn = ctx.Pipe(duplex=True)
        proc = ctx.Process(target=load_worker.worker_main,
                           args=(child_conn, i * 7, ISOLATE), daemon=True)
        proc.start()
        child_conn.close()
        _pool.append((proc, host_conn))
        time.sleep(0.15)  # stagger, so N interpreters do not all import at once
    for proc, conn in _pool:
        if not conn.poll(READY_TIMEOUT_S):
            kill_pool()
            sys.exit(f"worker pid={proc.pid} never reported ready in {READY_TIMEOUT_S}s")
        msg = conn.recv()
        if not msg.get("ready"):
            kill_pool()
            sys.exit(f"worker pid={proc.pid} sent {msg!r} instead of ready")


def run_step(c):
    """Tell the first c workers to go for STEP_S, sampling /proc meanwhile."""
    active = _pool[:c]
    cpu0 = {p.pid: pc.cpu_seconds(p.pid) for p, _ in active}
    t0 = time.perf_counter()
    for _, conn in active:
        conn.send({"cmd": "go", "duration": STEP_S})

    rss_samples, n_samples = [], 0
    deadline = t0 + STEP_S + STEP_SLACK_S
    pending = list(active)
    while pending and time.perf_counter() < deadline:
        total = 0
        for p, _ in active:
            v = pc.rss_kb(p.pid)
            if v:
                total += v
        if total:
            rss_samples.append(total)
            n_samples += 1
        time.sleep(SAMPLE_MS / 1000.0)
        pending = [(p, cn) for p, cn in pending if not cn.poll()]

    records = []
    for p, conn in active:
        if not conn.poll(STEP_SLACK_S):
            kill_pool()
            sys.exit(f"worker pid={p.pid} did not return records for c={c}")
        records.append(conn.recv()["records"])
    wall = time.perf_counter() - t0
    cpu = sum((pc.cpu_seconds(p.pid) or 0) - (cpu0[p.pid] or 0) for p, _ in active)

    flat = [r for w in records for r in w]
    lat = [r[1] for r in flat]
    inwin = [r for r in flat if r[2] <= STEP_S]
    heavy = [r for r in flat if r[0] in pc.HEAVY]
    light = [r[1] for r in flat if r[0] not in pc.HEAVY]
    errs = [r for r in flat if r[4] is not None]
    mism = [r for r in flat if r[4] is None and not r[3]]
    # steady state = samples after the first second of the step
    steady = rss_samples[int(1000 / SAMPLE_MS):] or rss_samples
    return {
        "concurrency": c,
        "wall_s": round(wall, 3),
        "evaluations": len(flat),
        "evaluations_in_window": len(inwin),
        "throughput_wall": round(len(flat) / wall, 3) if wall else None,
        "throughput_window": round(len(inwin) / STEP_S, 3),
        "latency_ms": pc.summarise(lat),
        "latency_light_ms": pc.summarise(light),
        "heavy_share": round(len(heavy) / len(flat), 4) if flat else None,
        "errors": len(errs),
        "error_rate": round(len(errs) / len(flat), 6) if flat else None,
        "mismatches": len(mism),
        "mismatch_slugs": sorted({r[0] for r in mism}),
        "rss_peak_kb": max(rss_samples) if rss_samples else None,
        "rss_steady_kb": int(statistics.median(steady)) if steady else None,
        "rss_samples": n_samples,
        "cpu_s": round(cpu, 3),
        "busy_cores": round(cpu / wall, 3) if wall else None,
        "mem_available_mb_before": None,  # filled by caller
        "per_worker_records": records,
    }


def main():
    signal.signal(signal.SIGINT, _bail)
    signal.signal(signal.SIGTERM, _bail)
    ctx = mp.get_context("spawn")
    report = {
        "engine": "OVAL (DIPS-Tools/odrl-Engine)",
        "commit": "a427e71b50bdd14027f2d5552d6ce03d089487f3",
        "concurrency_unit": "one OS process (multiprocessing, spawn start method)",
        "levels": LEVELS,
        "step_s": STEP_S,
        "ramp_repeats": RAMP_REPEATS,
        "worker_warmup_evaluations": load_worker.WARMUP,
        "sample_ms": SAMPLE_MS,
        "min_free_mb": MIN_FREE_MB,
        "isolate": ISOLATE,
        "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "ramps": [],
    }
    for rep in range(RAMP_REPEATS):
        avail = pc.mem_available_mb()
        if avail < MIN_FREE_MB:
            report["ramps"].append({"repeat": rep, "skipped": f"MemAvailable {avail:.0f}MB < {MIN_FREE_MB}MB"})
            continue
        t_pool = time.perf_counter()
        spawn_pool(ctx, MAXC)
        pool_ms = (time.perf_counter() - t_pool) * 1000
        steps = []
        for c in LEVELS:
            before = pc.mem_available_mb()
            if before < MIN_FREE_MB:
                steps.append({"concurrency": c, "skipped": f"MemAvailable {before:.0f}MB < {MIN_FREE_MB}MB"})
                continue
            st = run_step(c)
            st["mem_available_mb_before"] = round(before, 1)
            steps.append(st)
            print(f"  rep{rep} c={c:2d} tput_wall={st['throughput_wall']:7.2f} "
                  f"tput_win={st['throughput_window']:7.2f} "
                  f"med={st['latency_ms']['median']:8.2f} p99={st['latency_ms']['p99']:9.2f} "
                  f"rss_peak={st['rss_peak_kb']/1048576:5.2f}GiB cores={st['busy_cores']:6.2f} "
                  f"err={st['errors']}")
        for _, conn in _pool:
            conn.send({"cmd": "stop"})
        kill_pool()
        report["ramps"].append({
            "repeat": rep,
            "pool_spawn_ms": round(pool_ms, 1),
            "mem_available_mb_at_pool": round(avail, 1),
            "steps": steps,
        })

    # aggregate + stability gate (rule 3)
    agg = []
    for c in LEVELS:
        per = [s for r in report["ramps"] for s in r.get("steps", [])
               if s.get("concurrency") == c and "skipped" not in s]
        if not per:
            continue
        tp = [s["throughput_wall"] for s in per]
        med_tp = statistics.median(tp)
        rel = (max(tp) - min(tp)) / med_tp if med_tp else 0.0
        agg.append({
            "concurrency": c,
            "repeats": len(per),
            "throughput_wall": round(med_tp, 3),
            "throughput_window": round(statistics.median(s["throughput_window"] for s in per), 3),
            "wall_s": round(statistics.median(s["wall_s"] for s in per), 2),
            "median_ms": round(statistics.median(s["latency_ms"]["median"] for s in per), 2),
            "p95_ms": round(statistics.median(s["latency_ms"]["p95"] for s in per), 2),
            "p99_ms": round(statistics.median(s["latency_ms"]["p99"] for s in per), 2),
            "light_median_ms": round(statistics.median(s["latency_light_ms"]["median"] for s in per), 2),
            "light_p95_ms": round(statistics.median(s["latency_light_ms"]["p95"] for s in per), 2),
            "light_p99_ms": round(statistics.median(s["latency_light_ms"]["p99"] for s in per), 2),
            "heavy_share": round(statistics.median(s["heavy_share"] for s in per), 4),
            "rss_peak_gib": round(statistics.median(s["rss_peak_kb"] for s in per) / 1048576, 3),
            "rss_steady_gib": round(statistics.median(s["rss_steady_kb"] for s in per) / 1048576, 3),
            "busy_cores": round(statistics.median(s["busy_cores"] for s in per), 2),
            "errors_total": sum(s["errors"] for s in per),
            "error_rate": round(sum(s["errors"] for s in per) / sum(s["evaluations"] for s in per), 6),
            "mismatches_total": sum(s["mismatches"] for s in per),
            "throughput_rel_range": round(rel, 4),
            "unstable_across_repeats": rel > pc.LEVEL_INSTABILITY,
        })
    report["by_level"] = agg
    report["gates"] = {"rule_3_level_instability": {
        "threshold_rel_range": pc.LEVEL_INSTABILITY,
        "unstable_levels": [a["concurrency"] for a in agg if a["unstable_across_repeats"]],
        "worst_rel_range": max((a["throughput_rel_range"] for a in agg), default=None),
    }}
    report["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    OUT.write_text(json.dumps(report, indent=2))
    print(f"-> {OUT}")


if __name__ == "__main__":
    try:
        main()
    finally:
        kill_pool()
