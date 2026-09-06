#!/usr/bin/env python3
"""Small, rough vocabulary spot-check against OVAL -- NOT part of the 68-case
tally, and not a rigorous coverage audit.

Each probe is a minimal ODRL 2.2 policy in OVAL's own native input shape
(policy Turtle + State-of-the-World CSV), fed through the same public
entrypoint the bench uses:

    ODRL_Evaluator.evaluate_ODRL_from_files(policy.ttl, sotw.csv, normalise=False)

P1 reuses the vendored suite's own testcase-049 policy verbatim (an odrl:and of
a dateTime gt/lt pair) rather than reinventing one, varying only the SotW time.
P2-P5 are hand-written because the corpus contains no odrl:xone, no set
operator, and no numeric leftOperand to reuse.

"expected" below is what ODRL 2.2 semantics require, stated per-probe; it is my
reading of the spec for this spot-check, not the vendored suite's ground truth.
"""
import json
import os
import sys
from pathlib import Path

BASE = Path(__file__).resolve().parent
REPO = BASE / "odrl-Engine"
WORK = BASE / "probes"
WORK.mkdir(exist_ok=True)

sys.path.insert(0, str(REPO))
os.chdir(REPO)  # ODRL/ODRL22.ttl is resolved cwd-relative -- see bench.py note 1
import ODRL_Evaluator  # noqa: E402

PREFIX = """@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
@prefix xsd: <http://www.w3.org/2001/XMLSchema#>.
"""

DT = "http://www.w3.org/ns/odrl/2/dateTime"
PARTY = "http://www.w3.org/ns/odrl/2/Party"
ACTION = "http://www.w3.org/ns/odrl/2/Action"
ASSET = "http://www.w3.org/ns/odrl/2/Asset"


def run(name, policy_ttl, columns, row, expected, note):
    p = WORK / f"{name}.ttl"
    c = WORK / f"{name}.csv"
    p.write_text(policy_ttl)
    c.write_text(",".join(columns) + "\n" + ",".join(row) + "\n")
    try:
        r = ODRL_Evaluator.evaluate_ODRL_from_files(
            str(p), str(c), normalise=False, ontology_files=[], ontology_graphs=[]
        )
        actual = "Allow" if r[1] == 1 else "Deny"
        err = None
    except Exception as e:  # noqa: BLE001
        actual, err = "ERROR", f"{type(e).__name__}: {e}"
    verdict = "as ODRL 2.2 requires" if actual == expected else ">>> DIVERGES <<<"
    print(f"{name:26} expected={expected:5} actual={actual:5}  {verdict}")
    print(f"{'':26} {note}")
    if err:
        print(f"{'':26} error: {err}")
    return actual == expected


# --- P1: nested odrl:and (reuses the vendored suite's own testcase-049 policy) ---
AND_POLICY = (REPO / "test_cases/evaluation/force/extracted_testcase-049-alice-read-x-past.ttl").read_text()
BASE_COLS = [DT, PARTY, ACTION, ASSET]
BASE_ROW = ["{t}", "http://example.org/alice", "http://www.w3.org/ns/odrl/2/read", "http://example.org/x"]


def and_row(t):
    return [t if x == "{t}" else x for x in BASE_ROW]


print("=" * 100)
print("P1  odrl:and LogicalConstraint  (policy: Alice may read X between 2024-01-01 and 2024-12-31)")
print("=" * 100)
run("P1a_and_inside", AND_POLICY, BASE_COLS, and_row("2024-06-01T00:00:00+00:00"), "Allow",
    "time inside the AND'd window -> both conjuncts true")
run("P1b_and_before", AND_POLICY, BASE_COLS, and_row("2017-02-12T11:20:10+00:00"), "Deny",
    "time BEFORE the window -> the gt conjunct is false, so the AND is false")
run("P1c_and_after", AND_POLICY, BASE_COLS, and_row("2030-02-12T11:20:10+00:00"), "Deny",
    "time AFTER the window -> the lt conjunct is false, so the AND is false")

# --- P2: odrl:or ---
OR_POLICY = PREFIX + """
ex:p2 a odrl:Set;
    odrl:permission ex:perm2.
ex:perm2 a odrl:Permission;
    odrl:assignee ex:alice;
    odrl:action odrl:read;
    odrl:target ex:x;
    odrl:constraint ex:lc2.
ex:lc2 a odrl:LogicalConstraint;
    odrl:or ex:c2a, ex:c2b.
ex:c2a a odrl:Constraint;
    odrl:leftOperand odrl:dateTime;
    odrl:operator odrl:lt;
    odrl:rightOperand "2020-01-01T00:00:00Z"^^xsd:dateTime.
ex:c2b a odrl:Constraint;
    odrl:leftOperand odrl:dateTime;
    odrl:operator odrl:gt;
    odrl:rightOperand "2030-01-01T00:00:00Z"^^xsd:dateTime.
"""
print()
print("=" * 100)
print("P2  odrl:or LogicalConstraint  (permit if dateTime < 2020 OR dateTime > 2030)")
print("=" * 100)
run("P2a_or_first", OR_POLICY, BASE_COLS, and_row("2015-06-01T00:00:00+00:00"), "Allow",
    "satisfies the FIRST disjunct only")
run("P2b_or_second", OR_POLICY, BASE_COLS, and_row("2035-06-01T00:00:00+00:00"), "Allow",
    "satisfies the SECOND disjunct only")
run("P2c_or_neither", OR_POLICY, BASE_COLS, and_row("2025-06-01T00:00:00+00:00"), "Deny",
    "satisfies NEITHER disjunct")

# --- P3: odrl:xone ---
XONE_POLICY = PREFIX + """
ex:p3 a odrl:Set;
    odrl:permission ex:perm3.
ex:perm3 a odrl:Permission;
    odrl:assignee ex:alice;
    odrl:action odrl:read;
    odrl:target ex:x;
    odrl:constraint ex:lc3.
ex:lc3 a odrl:LogicalConstraint;
    odrl:xone ex:c3a, ex:c3b.
ex:c3a a odrl:Constraint;
    odrl:leftOperand ex:flagA;
    odrl:operator odrl:eq;
    odrl:rightOperand "yes".
ex:c3b a odrl:Constraint;
    odrl:leftOperand ex:flagB;
    odrl:operator odrl:eq;
    odrl:rightOperand "yes".
"""
XCOLS = BASE_COLS + ["http://example.org/flagA", "http://example.org/flagB"]
print()
print("=" * 100)
print("P3  odrl:xone LogicalConstraint  (permit iff EXACTLY ONE of flagA/flagB == yes)")
print("=" * 100)
run("P3a_xone_one_true", XONE_POLICY, XCOLS, and_row("2024-06-01T00:00:00+00:00") + ["yes", "no"], "Allow",
    "exactly one branch true -> xone satisfied")
run("P3b_xone_both_true", XONE_POLICY, XCOLS, and_row("2024-06-01T00:00:00+00:00") + ["yes", "yes"], "Deny",
    "BOTH branches true -> xone's exclusivity must reject this (the discriminating case)")
run("P3c_xone_none_true", XONE_POLICY, XCOLS, and_row("2024-06-01T00:00:00+00:00") + ["no", "no"], "Deny",
    "neither branch true -> xone unsatisfied")

# --- P4: set operators isAnyOf / isNoneOf / isPartOf ---
def setop_policy(op):
    return PREFIX + f"""
ex:p4 a odrl:Set;
    odrl:permission ex:perm4.
ex:perm4 a odrl:Permission;
    odrl:assignee ex:alice;
    odrl:action odrl:read;
    odrl:target ex:x;
    odrl:constraint ex:c4.
ex:c4 a odrl:Constraint;
    odrl:leftOperand ex:tier;
    odrl:operator odrl:{op};
    odrl:rightOperand "gold", "silver".
"""

TCOLS = BASE_COLS + ["http://example.org/tier"]
print()
print("=" * 100)
print("P4  set operators  (leftOperand ex:tier vs the set {gold, silver})")
print("=" * 100)
run("P4a_isAnyOf_member", setop_policy("isAnyOf"), TCOLS, and_row("2024-06-01T00:00:00+00:00") + ["gold"], "Allow",
    "isAnyOf: tier=gold IS in the set")
run("P4b_isAnyOf_nonmember", setop_policy("isAnyOf"), TCOLS, and_row("2024-06-01T00:00:00+00:00") + ["bronze"], "Deny",
    "isAnyOf: tier=bronze is NOT in the set")
run("P4c_isNoneOf_nonmember", setop_policy("isNoneOf"), TCOLS, and_row("2024-06-01T00:00:00+00:00") + ["bronze"], "Allow",
    "isNoneOf: tier=bronze is correctly absent from the set")
run("P4d_isNoneOf_member", setop_policy("isNoneOf"), TCOLS, and_row("2024-06-01T00:00:00+00:00") + ["gold"], "Deny",
    "isNoneOf: tier=gold IS in the set, so the constraint fails")

# --- P5: numeric leftOperand ---
def count_policy(op, val):
    return PREFIX + f"""
ex:p5 a odrl:Set;
    odrl:permission ex:perm5.
ex:perm5 a odrl:Permission;
    odrl:assignee ex:alice;
    odrl:action odrl:read;
    odrl:target ex:x;
    odrl:constraint ex:c5.
ex:c5 a odrl:Constraint;
    odrl:leftOperand odrl:count;
    odrl:operator odrl:{op};
    odrl:rightOperand "{val}"^^xsd:integer.
"""

CCOLS = BASE_COLS + ["http://www.w3.org/ns/odrl/2/count"]
print()
print("=" * 100)
print("P5  numeric leftOperand  (odrl:count compared with lteq / gt)")
print("=" * 100)
run("P5a_count_lteq_under", count_policy("lteq", 5), CCOLS, and_row("2024-06-01T00:00:00+00:00") + ["3"], "Allow",
    "count=3 lteq 5 -> true")
run("P5b_count_lteq_over", count_policy("lteq", 5), CCOLS, and_row("2024-06-01T00:00:00+00:00") + ["9"], "Deny",
    "count=9 lteq 5 -> false (a real numeric comparison; string compare would say '9' > '5' too, so...)")
run("P5c_count_gt_numeric", count_policy("gt", 5), CCOLS, and_row("2024-06-01T00:00:00+00:00") + ["10"], "Allow",
    "count=10 gt 5 -> true NUMERICALLY, but false as a STRING compare ('10' < '5'): discriminates the two")
