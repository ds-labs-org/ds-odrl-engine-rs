#!/usr/bin/env python3
"""Route 2 of the plan: re-translate the LOCAL vendored ODRL-Test-Suite into
OVAL's native (policy.ttl + SotW.csv) input format, to prove independence from
OVAL's committed, pre-translated corpus.

DEVIATION from the plan's `bash translate_force.sh`, documented because it is
real: at current upstream HEAD, `FORCE_translator.py`'s `__main__` block calls
`translate_csv_to_solid_syntax(args[0])` -- the *reverse* direction (CSV -> Solid
Turtle). translate_force.sh feeds it `data/documentation/*.md`, so it runs
`pd.read_csv()` over markdown and dies inside the Turtle serializer:

    Exception: "Any request results into yes (Alice Request)." does not look
    like a valid URI, I cannot serialize this as N3/Turtle.

The function translate_force.sh plainly intends is `parse_test_cases_from_md`,
which is present and unmodified in the same module. This driver calls exactly
that upstream function -- no reimplementation of its logic -- once per case,
restricted to the 68 slugs ds-odrl-engine-rs's compliance-runner enumerates from
data/index.ttl, so the corpus is case-for-case identical to the Rust runner's.

Output lands in a scratch cwd so OVAL's committed corpus stays intact for the
provenance diff.
"""
import json
import os
import sys
from pathlib import Path

BASE = Path(__file__).resolve().parent
REPO = BASE / "odrl-Engine"
SUITE = BASE / "ODRL-Test-Suite"
OUT = BASE / "retranslated"

sys.path.insert(0, str(REPO))

# parse_test_cases_from_md hardcodes its destination as the relative path
# "test_cases/evaluation/force/extracted_<name>.csv", so we chdir into a scratch
# root that provides that subtree.
(OUT / "test_cases/evaluation/force").mkdir(parents=True, exist_ok=True)
os.chdir(OUT)

import FORCE_translator  # noqa: E402  (import after sys.path/cwd setup)

cases = json.loads((BASE / "ground_truth.json").read_text())

ok, failed = 0, []
for c in cases:
    md = SUITE / "data/documentation" / f"{c['slug']}.md"
    if not md.exists():
        failed.append((c["slug"], "no documentation/*.md for this slug"))
        continue
    try:
        FORCE_translator.parse_test_cases_from_md(str(md))
        ok += 1
    except Exception as e:  # noqa: BLE001 - report, never hide
        failed.append((c["slug"], f"{type(e).__name__}: {e}"))

print(f"translated {ok}/{len(cases)}")
for slug, why in failed:
    print(f"  FAILED {slug}: {why}")
