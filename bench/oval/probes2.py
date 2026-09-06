#!/usr/bin/env python3
"""Follow-up probes, correcting one mis-designed probe and adding the decisive
discriminators for odrl:and / odrl:or.

Why a second round:
 * P5 in probes.py used odrl:count as its "numeric leftOperand". That was my
   error, not OVAL's: ODRL_Evaluator.eval_count treats odrl:count as an
   AGGREGATE over matching State-of-the-World events (how many times the action
   was performed), not as a per-row literal to read out of the CSV. All three P5
   outcomes are exactly what "count = 1 matching row" predicts, so P5 measured
   OVAL's count aggregation, not its numeric comparison. P5' below re-tests
   numeric comparison with a plain, non-reserved left operand, and P5'' checks
   the count-as-aggregate reading directly.
 * P1/P2 showed odrl:and behaving permissively and odrl:or restrictively. P1d
   (both conjuncts FALSE) and P2d (both disjuncts TRUE) are the two cases that
   discriminate "and/or are swapped" from "and/or are merely broken".
"""
import os
import sys
from pathlib import Path

BASE = Path(__file__).resolve().parent
REPO = BASE / "odrl-Engine"
WORK = BASE / "probes"
WORK.mkdir(exist_ok=True)
sys.path.insert(0, str(REPO))
os.chdir(REPO)
import ODRL_Evaluator  # noqa: E402

PREFIX = """@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
@prefix xsd: <http://www.w3.org/2001/XMLSchema#>.
"""
DT = "http://www.w3.org/ns/odrl/2/dateTime"
COLS = [DT, "http://www.w3.org/ns/odrl/2/Party",
        "http://www.w3.org/ns/odrl/2/Action", "http://www.w3.org/ns/odrl/2/Asset"]


def row(t, *extra):
    return [t, "http://example.org/alice", "http://www.w3.org/ns/odrl/2/read",
            "http://example.org/x", *extra]


def run(name, policy, cols, rows, expected, note):
    p, c = WORK / f"{name}.ttl", WORK / f"{name}.csv"
    p.write_text(policy)
    c.write_text(",".join(cols) + "\n" + "".join(",".join(r) + "\n" for r in rows))
    try:
        r = ODRL_Evaluator.evaluate_ODRL_from_files(
            str(p), str(c), normalise=False, ontology_files=[], ontology_graphs=[])
        actual = "Allow" if r[1] == 1 else "Deny"
    except Exception as e:  # noqa: BLE001
        actual = f"ERROR {type(e).__name__}: {e}"
    mark = "as ODRL 2.2 requires" if actual == expected else ">>> DIVERGES <<<"
    print(f"{name:26} expected={expected:5} actual={actual:5}  {mark}")
    print(f"{'':26} {note}")


AND_POLICY = (REPO / "test_cases/evaluation/force/extracted_testcase-049-alice-read-x-past.ttl").read_text()
print("=" * 100)
print("P1d  odrl:and, BOTH conjuncts false  (discriminates 'and is ignored' from 'and behaves as or')")
print("=" * 100)
# window is 2024-01-01 .. 2024-12-31; no single instant makes BOTH gt and lt false,
# so use a purpose-built policy with two independently falsifiable conjuncts.
AND2 = PREFIX + """
ex:p a odrl:Set; odrl:permission ex:perm.
ex:perm a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    odrl:constraint ex:lc.
ex:lc a odrl:LogicalConstraint; odrl:and ex:ca, ex:cb.
ex:ca a odrl:Constraint; odrl:leftOperand ex:fa; odrl:operator odrl:eq; odrl:rightOperand "yes".
ex:cb a odrl:Constraint; odrl:leftOperand ex:fb; odrl:operator odrl:eq; odrl:rightOperand "yes".
"""
FCOLS = COLS + ["http://example.org/fa", "http://example.org/fb"]
T = "2024-06-01T00:00:00+00:00"
run("P1d_and_both_true", AND2, FCOLS, [row(T, "yes", "yes")], "Allow", "both conjuncts true -> AND true")
run("P1e_and_one_true", AND2, FCOLS, [row(T, "yes", "no")], "Deny", "one conjunct FALSE -> AND must be false")
run("P1f_and_both_false", AND2, FCOLS, [row(T, "no", "no")], "Deny", "both conjuncts false -> AND false")

print()
print("=" * 100)
print("P2d  odrl:or, BOTH disjuncts true")
print("=" * 100)
OR2 = AND2.replace("odrl:and ex:ca, ex:cb", "odrl:or ex:ca, ex:cb")
run("P2d_or_both_true", OR2, FCOLS, [row(T, "yes", "yes")], "Allow", "both disjuncts true -> OR true")
run("P2e_or_one_true", OR2, FCOLS, [row(T, "yes", "no")], "Allow", "one disjunct true -> OR must be TRUE")
run("P2f_or_both_false", OR2, FCOLS, [row(T, "no", "no")], "Deny", "both disjuncts false -> OR false")

print()
print("=" * 100)
print("P5'  numeric leftOperand, plain (non-reserved) operand ex:size")
print("=" * 100)


def size_policy(op, val):
    return PREFIX + f"""
ex:p a odrl:Set; odrl:permission ex:perm.
ex:perm a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    odrl:constraint ex:c.
ex:c a odrl:Constraint; odrl:leftOperand ex:size;
    odrl:operator odrl:{op}; odrl:rightOperand "{val}"^^xsd:integer.
"""


SCOLS = COLS + ["http://example.org/size"]
run("P5'a_gt_numeric", size_policy("gt", 5), SCOLS, [row(T, "10")], "Allow",
    "size=10 gt 5 is TRUE numerically but FALSE as a string compare ('10' < '5') -- discriminates the two")
run("P5'b_lteq_over", size_policy("lteq", 5), SCOLS, [row(T, "9")], "Deny", "size=9 lteq 5 -> false")
run("P5'c_lteq_under", size_policy("lteq", 5), SCOLS, [row(T, "3")], "Allow", "size=3 lteq 5 -> true")

print()
print("=" * 100)
print("P5'' odrl:count read as an AGGREGATE over SotW events (checks the probes.py P5 explanation)")
print("=" * 100)


def count_policy(op, val):
    return PREFIX + f"""
ex:p a odrl:Set; odrl:permission ex:perm.
ex:perm a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    odrl:constraint ex:c.
ex:c a odrl:Constraint; odrl:leftOperand odrl:count;
    odrl:operator odrl:{op}; odrl:rightOperand "{val}"^^xsd:integer.
"""


times = [f"2024-06-0{i}T00:00:00+00:00" for i in range(1, 8)]  # 7 matching read events
run("P5''a_count_gt5_7events", count_policy("gt", 5), COLS, [row(t) for t in times], "Allow",
    "7 matching read events in the SotW; count gt 5 -> true IF count aggregates events")
run("P5''b_count_gt5_1event", count_policy("gt", 5), COLS, [row(T)], "Deny",
    "1 matching event; count gt 5 -> false. Together these confirm count is an event aggregate.")
