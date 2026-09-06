#!/usr/bin/env python3
"""Small, rough ODRL 2.2 vocabulary spot-check against ODRL-PAP's /validate.

Not a coverage audit - five targeted probes for constructs mapping.json
advertises (odrl:and/or/xone operands; isAnyOf/isNoneOf/isPartOf/isAllOf
operators; numeric comparisons) but that the 68-case corpus never exercises.
"""
import json
import urllib.request

PAP = "http://localhost:8091/validate"
CTX = {
    "odrl": {"@id": "http://www.w3.org/ns/odrl/2/", "@prefix": True},
    "json": {"@id": "https://odrl-pap.io/json#", "@prefix": True},
    "xsd": {"@id": "http://www.w3.org/2001/XMLSchema#", "@prefix": True},
}


def policy(pid, constraint):
    return {
        "@context": CTX,
        "@id": pid,
        "odrl:uid": pid,
        "@type": "odrl:Set",
        "odrl:permission": {
            "@type": "odrl:Permission",
            "json:target": "urn:probe:asset",
            "odrl:assignee": "json:any",
            "odrl:action": {"@id": "json:use"},
            "odrl:constraint": constraint,
        },
    }


def atom(path, op, right):
    # ODRL-PAP's own "domain-specific leftOperand" form: the namespaced
    # leftOperand IS the key, and its value is the method's parameter.
    return {
        "@type": "odrl:Constraint",
        "json:payloadValue": "$." + path,
        "odrl:operator": {"@id": "odrl:" + op},
        "odrl:rightOperand": right,
    }


def ask(pid, constraint, payload):
    vr = {"policy": policy(pid, constraint),
          "jsonInput": {"payload": dict(payload, action="use",
                                        target="urn:probe:asset")}}
    body = json.dumps(vr).encode()
    req = urllib.request.Request(PAP, data=body,
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read()).get("allow")
    except Exception as e:
        detail = ""
        if hasattr(e, "read"):
            try:
                detail = e.read().decode()[:300]
            except Exception:
                pass
        return "ERROR: %s %s" % (e, detail)


def show(name, results):
    print("\n### %s" % name)
    for label, value in results:
        print("   %-52s -> %s" % (label, value))


# P1 -- nested logical constraint: (a AND b) OR c, three levels deep
nested = {
    "@type": "odrl:LogicalConstraint",
    "odrl:or": [
        {"@type": "odrl:LogicalConstraint",
         "odrl:and": [atom("dept", "eq", "research"),
                      atom("clearance", "eq", "high")]},
        atom("role", "eq", "auditor"),
    ],
}
show("P1 nested odrl:or of (odrl:and, atomic)", [
    ("dept=research clearance=high role=none (left disjunct)",
     ask("urn:probe:1", nested, {"dept": "research", "clearance": "high",
                                 "role": "none"})),
    ("dept=sales clearance=low role=auditor (right disjunct)",
     ask("urn:probe:1", nested, {"dept": "sales", "clearance": "low",
                                 "role": "auditor"})),
    ("dept=research clearance=low role=none (neither)",
     ask("urn:probe:1", nested, {"dept": "research", "clearance": "low",
                                 "role": "none"})),
])

# P2 -- odrl:xone (exactly one)
xone = {
    "@type": "odrl:LogicalConstraint",
    "odrl:xone": [atom("a", "eq", "1"), atom("b", "eq", "1"),
                  atom("c", "eq", "1")],
}
show("P2 odrl:xone over three atomics", [
    ("exactly one true  (a=1,b=0,c=0)",
     ask("urn:probe:2", xone, {"a": "1", "b": "0", "c": "0"})),
    ("two true          (a=1,b=1,c=0)",
     ask("urn:probe:2", xone, {"a": "1", "b": "1", "c": "0"})),
    ("none true         (a=0,b=0,c=0)",
     ask("urn:probe:2", xone, {"a": "0", "b": "0", "c": "0"})),
])

# P3 -- odrl:isAnyOf / odrl:isNoneOf against a right-operand set
any_of = atom("role", "isAnyOf", ["admin", "auditor"])
none_of = atom("role", "isNoneOf", ["admin", "auditor"])
show("P3 odrl:isAnyOf / odrl:isNoneOf (scalar left, set right)", [
    ("isAnyOf  role=auditor in [admin,auditor]",
     ask("urn:probe:3a", any_of, {"role": "auditor"})),
    ("isAnyOf  role=intern  in [admin,auditor]",
     ask("urn:probe:3a", any_of, {"role": "intern"})),
    ("isNoneOf role=intern  in [admin,auditor]",
     ask("urn:probe:3b", none_of, {"role": "intern"})),
    ("isNoneOf role=admin   in [admin,auditor]",
     ask("urn:probe:3b", none_of, {"role": "admin"})),
])

# P4 -- odrl:isPartOf and odrl:isAllOf: ODRL 2.2 vs ODRL-PAP's rego
part_of_odrl22 = atom("role", "isPartOf", ["admin", "auditor"])   # left in right
part_of_pap = atom("roles", "isPartOf", "admin")                  # right in left
all_of = atom("roles", "isAllOf", ["admin", "auditor"])
show("P4 odrl:isPartOf / odrl:isAllOf", [
    ("isPartOf ODRL-2.2 reading: role=admin isPartOf [admin,auditor]",
     ask("urn:probe:4a", part_of_odrl22, {"role": "admin"})),
    ("isPartOf ODRL-PAP rego reading: roles=[admin,x] contains 'admin'",
     ask("urn:probe:4b", part_of_pap, {"roles": ["admin", "x"]})),
    ("isAllOf  roles=[admin,auditor] vs [admin,auditor] (equal sets)",
     ask("urn:probe:4c", all_of, {"roles": ["admin", "auditor"]})),
    ("isAllOf  roles=[admin,auditor,x] superset of [admin,auditor]",
     ask("urn:probe:4c", all_of, {"roles": ["admin", "auditor", "x"]})),
])

# P5 -- numeric leftOperand comparison
num = atom("count", "gt", {"@value": "5", "@type": "xsd:integer"})
show("P5 numeric leftOperand, odrl:gt, xsd:integer right operand", [
    ("count=10 gt 5", ask("urn:probe:5", num, {"count": 10})),
    ("count=3  gt 5", ask("urn:probe:5", num, {"count": 3})),
    ("count='10' (string) gt 5", ask("urn:probe:5", num, {"count": "10"})),
])
