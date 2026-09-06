#!/usr/bin/env python3
"""Runs OVAL (DIPS-Tools/odrl-Engine) over the 68 vendored ODRL-Test-Suite
fixtures and scores it against the SAME ground truth ds-odrl-engine-rs's
compliance-runner is checked against (ground_truth.py, which reproduces
ground_truth.rs's report:* -> Allow/Deny reduction exactly).

The engine call is upstream's own public entrypoint, unmodified:

    ODRL_Evaluator.evaluate_ODRL_from_files(policy_ttl, sotw_csv, normalise=False)

exactly as upstream's own test_on_force.py invokes it. Its return tuple's
element [1] is `temporary_validity`: 1 = allowed, 0 = not allowed -- upstream's
own harness uses the identical `result[1] == 1` test, so no interpretation is
added here.

TWO ENVIRONMENT FACTS this harness must control for, both found the hard way:

1. ODRL_Evaluator.py:793 does
       ontology_files.append(os.path.join("ODRL", "ODRL22.ttl"))
   -- a *cwd-relative* path to the ODRL 2.2 ontology that drives its
   action-implication reasoning (odrl:read/write includedIn odrl:use). Run from
   any directory other than the repo root, the file is absent, the failure is
   silent, and the "use covers read" fixtures flip to Deny. So we chdir to the
   repo root and pass absolute corpus paths.

2. The same signature carries MUTABLE DEFAULT arguments
       ontology_files=[], ontology_graphs=[]
   and line 952 does `ontology_graphs.append(graph)` on every call. Across a
   loop in one process, every previously-evaluated policy graph accumulates in
   that shared list and is fed to the reasoner for later cases. `isolate=True`
   passes fresh lists per case to defeat this; `isolate=False` reproduces
   upstream's own test_on_force.py conditions verbatim. We run both and report
   any case whose verdict depends on it.

Usage: bench.py <corpus_dir> <out.json> [--isolate]
"""
import json
import os
import sys
import time
from pathlib import Path

BASE = Path(__file__).resolve().parent
REPO = BASE / "odrl-Engine"
corpus = Path(sys.argv[1]).resolve()
outp = (BASE / sys.argv[2]).resolve()
isolate = "--isolate" in sys.argv

cases = json.loads((BASE / "ground_truth.json").read_text())

sys.path.insert(0, str(REPO))
os.chdir(REPO)  # so "ODRL/ODRL22.ttl" resolves -- see note 1 above
import ODRL_Evaluator  # noqa: E402

assert Path("ODRL/ODRL22.ttl").exists(), "ODRL 2.2 ontology not resolvable from cwd"

results = []
for c in cases:
    slug = c["slug"]
    ttl = corpus / f"extracted_{slug}.ttl"
    csvf = corpus / f"extracted_{slug}.csv"
    rec = {"slug": slug, "title": c["title"], "expected": c["expected_decision"]}
    if not ttl.exists() or not csvf.exists():
        rec.update(status="SKIP", actual=None, reason="no translated corpus files for this slug")
        results.append(rec)
        continue
    kw = {"ontology_files": [], "ontology_graphs": []} if isolate else {}
    t0 = time.perf_counter()
    try:
        r = ODRL_Evaluator.evaluate_ODRL_from_files(str(ttl), str(csvf), normalise=False, **kw)
        actual = "Allow" if r[1] == 1 else "Deny"
        rec.update(
            status="PASS" if actual == c["expected_decision"] else "FAIL",
            actual=actual,
            raw_validity=r[1],
            ms=round((time.perf_counter() - t0) * 1000, 2),
        )
    except Exception as e:  # noqa: BLE001 - an engine error is a real result, not a skip
        rec.update(status="ERROR", actual=None, reason=f"{type(e).__name__}: {e}")
    results.append(rec)

outp.write_text(json.dumps(results, indent=2))

tally = {}
for r in results:
    tally[r["status"]] = tally.get(r["status"], 0) + 1
print(f"corpus={corpus.name}  isolate={isolate}")
print(f"total={len(results)} " + " ".join(f"{k}={v}" for k, v in sorted(tally.items())))
for r in results:
    if r["status"] != "PASS":
        print(f"  {r['status']:5} {r['slug']:38} expected={r['expected']:5} actual={r.get('actual')}")
        print(f"        {r['title']}")
