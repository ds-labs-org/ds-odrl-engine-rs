#!/usr/bin/env python3
"""Shared plumbing for the ODRL-PAP performance/load benches: the corpus, the
one call path both benches use, the process sampler, and the stability gates.

The call path is deliberately IDENTICAL to run_pap.py's: an HTTP POST of
`case["validationRequest"]` to the service's own /validate endpoint, scored by
`resp["allow"]`. Nothing here reaches around the service to talk to OPA
directly for the timed numbers, so a latency reported here is the same work
run_pap.py's conformance numbers were produced by -- one /validate is one
JSON-LD compaction, one ODRL->Rego mapping, one PUT of a temp policy module
into OPA, one OPA data query, and one DELETE of the module (see the engine's
own ValidationResource.validatePolicy).

The 37 cases run_pap.py marks `skip` are skipped here too. A skip is a
translation gap, not a slow path, and timing a case the engine never sees
would be timing this harness. So the timed corpus is the 31 cases the engine
actually evaluates.

Gates (rule numbers are quoted by both benches' output):

  TUKEY_K          rule 1 -- a single measurement is flagged when it falls
                   outside [q1 - k*iqr, q3 + k*iqr] of its own run's pooled
                   distribution. Flagged, never dropped.
  CASE_INSTABILITY rule 2 -- a case is flagged unstable when, across repeats,
                   (max-min)/median exceeds this. Flagged, never dropped.
  STEP_INSTABILITY rule 3 -- a load step is flagged unstable when its p99 is
                   more than this multiple of its own median.
"""
import json
import statistics
import time
import urllib.error
import urllib.request
from pathlib import Path

PAP = "http://localhost:8091/validate"
OPA = "http://localhost:8181"

TUKEY_K = 1.5
CASE_INSTABILITY = 0.50
STEP_INSTABILITY = 4.0

HERE = Path(__file__).resolve().parent


def load_cases(path=None):
    """The 31 evaluable cases, in the corpus's own testcase-NNN order."""
    cases = json.loads((Path(path) if path else HERE / "cases.json").read_text())
    return [c for c in cases if "skip" not in c]


def validate(case, timeout=120):
    """One /validate call. Returns (elapsed_ms, decision, error).

    Same request shape, same endpoint, same scoring rule as run_pap.py.
    """
    body = json.dumps(case["validationRequest"]).encode()
    req = urllib.request.Request(
        PAP, data=body, headers={"Content-Type": "application/json"}
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            payload = json.loads(r.read())
        ms = (time.perf_counter() - t0) * 1000
        return ms, ("Allow" if payload.get("allow") else "Deny"), None
    except Exception as e:
        ms = (time.perf_counter() - t0) * 1000
        detail = ""
        if hasattr(e, "read"):
            try:
                detail = e.read().decode()[:200]
            except Exception:
                pass
        return ms, None, "%s %s" % (e, detail)


def opa_roundtrip(timeout=30):
    """One bare OPA data-API round trip, no PAP in the path.

    The decomposition baseline: /validate costs three of these plus the
    JSON-LD + mapping work the PAP does itself, so this says how much of a
    /validate is unavoidable OPA/HTTP floor.
    """
    body = json.dumps({"input": {}}).encode()
    req = urllib.request.Request(
        OPA + "/v1/data", data=body, headers={"Content-Type": "application/json"}
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            r.read()
        return (time.perf_counter() - t0) * 1000, None
    except Exception as e:
        return (time.perf_counter() - t0) * 1000, str(e)


def summarise(values):
    """mean/median/p95/p99/min/max/stddev + the quartiles the Tukey gate needs."""
    if not values:
        return None
    s = sorted(values)
    pick = lambda p: s[min(len(s) - 1, max(0, int(round(p * (len(s) - 1)))))]
    q1, q3 = pick(0.25), pick(0.75)
    return {
        "n": len(s),
        "mean": round(statistics.fmean(s), 3),
        "median": round(statistics.median(s), 3),
        "p95": round(pick(0.95), 3),
        "p99": round(pick(0.99), 3),
        "min": round(s[0], 3),
        "max": round(s[-1], 3),
        "stddev": round(statistics.stdev(s), 3) if len(s) > 1 else 0.0,
        "q1": round(q1, 3),
        "q3": round(q3, 3),
        "iqr": round(q3 - q1, 3),
    }


def tukey_flags(samples, summ, key="ms"):
    """Rule 1 applied to a list of sample dicts. Returns (fence, flagged list)."""
    lo = summ["q1"] - TUKEY_K * summ["iqr"]
    hi = summ["q3"] + TUKEY_K * summ["iqr"]
    flagged = [s for s in samples
               if s.get("error") is None and not (lo <= s[key] <= hi)]
    return (round(lo, 3), round(hi, 3)), flagged


# ---------------------------------------------------------------------------
# Process sampling
# ---------------------------------------------------------------------------
# ODRL-PAP is not a CLI, so /usr/bin/time -v cannot wrap the thing being
# measured -- the JVM outlives every individual measurement. Resource numbers
# therefore come from a psutil sampler that watches the already-running
# process trees from outside, in a background thread, while the bench drives
# load through them.

class Sampler:
    """Samples RSS and CPU time of the named process trees on an interval.

    `targets` is {label: pid}. Each pid is sampled together with its children
    (Postgres forks a backend per connection; the JVM and OPA do not, but the
    tree walk costs nothing and keeps all three symmetric).
    """

    def __init__(self, targets, interval=0.25):
        import psutil
        self.psutil = psutil
        self.interval = interval
        self.procs = {}
        self.missing = []
        for label, pid in targets.items():
            try:
                self.procs[label] = psutil.Process(pid)
            except Exception as e:
                self.missing.append({"label": label, "pid": pid, "error": str(e)})
        self.series = {label: [] for label in self.procs}
        self._stop = False
        self._thread = None

    def _tree(self, proc):
        try:
            members = [proc] + proc.children(recursive=True)
        except Exception:
            return None
        rss = cpu = 0.0
        alive = 0
        for p in members:
            try:
                rss += p.memory_info().rss
                t = p.cpu_times()
                cpu += t.user + t.system
                alive += 1
            except Exception:
                continue
        return {"rss_kb": int(rss / 1024), "cpu_s": round(cpu, 3), "procs": alive}

    def sample_once(self):
        stamp = time.perf_counter()
        for label, proc in self.procs.items():
            row = self._tree(proc)
            if row is not None:
                row["t"] = round(stamp, 4)
                self.series[label].append(row)

    def _loop(self):
        while not self._stop:
            self.sample_once()
            time.sleep(self.interval)

    def start(self):
        import threading
        self.sample_once()
        self._stop = False
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()
        return self

    def stop(self):
        self._stop = True
        if self._thread:
            self._thread.join(timeout=5)
        self.sample_once()
        return self.report()

    def report(self):
        out = {"interval_s": self.interval, "missing": self.missing, "trees": {}}
        for label, rows in self.series.items():
            if not rows:
                continue
            rss = [r["rss_kb"] for r in rows]
            cpu = [r["cpu_s"] for r in rows]
            out["trees"][label] = {
                "samples": len(rows),
                "rss_first_kb": rss[0],
                "rss_median_kb": int(statistics.median(rss)),
                "rss_peak_kb": max(rss),
                "rss_last_kb": rss[-1],
                "rss_growth_kb": rss[-1] - rss[0],
                "cpu_start_s": cpu[0],
                "cpu_end_s": cpu[-1],
                "cpu_consumed_s": round(cpu[-1] - cpu[0], 3),
                "max_procs_in_tree": max(r["procs"] for r in rows),
            }
        return out

    def raw(self):
        return self.series


def pids_from_files(pid_file, opa_name="pap-opa", pg_name="pap-postgres"):
    """The three tree roots: the Quarkus JVM (pid file written by run_perf.sh)
    and the two containers (host-network, so their pids are host pids)."""
    import subprocess
    targets = {}
    try:
        targets["quarkus"] = int(Path(pid_file).read_text().strip())
    except Exception:
        pass
    for label, name in (("opa", opa_name), ("postgres", pg_name)):
        try:
            out = subprocess.run(["docker", "inspect", "-f", "{{.State.Pid}}", name],
                                 capture_output=True, text=True, timeout=20)
            pid = int(out.stdout.strip())
            if pid > 0:
                targets[label] = pid
        except Exception:
            pass
    return targets
