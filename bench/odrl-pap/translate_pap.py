#!/usr/bin/env python3
"""
Adapter: SolidLabResearch/ODRL-Test-Suite Turtle fixtures -> ODRL-PAP
(SEAMWARE/odrl-pap) native input, plus the SAME Allow/Deny ground truth
ds-odrl-engine-rs's compliance-runner/src/ground_truth.rs derives.

Translation conventions (stated loudly, in the spirit of translate.rs):

  * ODRL-PAP consumes ODRL as *JSON-LD*, the fixtures are Turtle. The policy
    document is rebuilt as a nested JSON-LD tree with the fixture's own
    odrl: vocabulary UNCHANGED - same rule type, same odrl:action IRI, same
    odrl:assignee/odrl:target IRIs, same odrl:leftOperand/odrl:operator/
    odrl:rightOperand, same odrl:and / odrl:or / odrl:xone nesting. Nothing
    in the policy is renamed or re-namespaced.

  * ODRL-PAP has no odrl:Request concept: the request reaches it as an OPA
    input document. We use the PAP's own *generic JSON* evaluation context
    (utils/generic.rego, the json: mappings in mapping.json), so the
    fixture's (assignee, action, target) triple lands as
        input.request.payload = {action, target, assignee, currentTime}
    and the SotW's temp:currentTime dct:issued value lands as
    payload.currentTime - the same move translate.rs makes when it injects
    the SotW clock as a `dateTime` claim.

  * Because the PAP's *built-in* odrl: bindings resolve odrl:read/odrl:use to
    HTTP verbs (odrl_action.is_read(helper.http_part) == "method == GET") and
    odrl:assignee to a JWT/VC issuer, we supply the documented
    `paths.mapping` overlay (pap-mapping.json) that rebinds exactly five
    odrl: terms - read, write, use, assignee, target, dateTime - onto the
    PAP's own generic-JSON rego methods. That is ODRL-PAP's own advertised
    extension point ("The mapping.json can be extended via a mapping file,
    configured at paths.mapping"), not a patch to the engine.

  * dateTime literals: ODRL-PAP converts a right operand only when it is
    typed xsd:date, and it parses it with SimpleDateFormat("yyyy-MM-dd"),
    which silently drops the time of day; an xsd:dateTime literal is passed
    through as a *string*. So both sides are compared as strings. To make
    Rego's lexicographic string order agree with chronological order we
    canonicalise both the policy literal and the SotW clock to
    YYYY-MM-DDTHH:MM:SS.mmmZ (UTC). The instant is preserved exactly; only
    the spelling is normalised.

Honest skips - the fixture needs a construct ODRL-PAP's input format cannot
represent at all (each skip carries the engine's own reason):

  S1 odrl:prohibition          - OdrlMapper reads only odrl:permission; a
                                 policy without one is rejected outright
                                 ("The policy has no permission").
  S2 permission with no action - "The permission does not contain an action."
  S3 permission with no target - "The permission does not contain a target."
  S4 PartyCollection /         - OdrlMapper routes a collection through
     AssetCollection             mapRefinementCollection, which REQUIRES an
                                 odrl:refinement; these fixtures identify
                                 members via odrl:source + a SotW
                                 odrl:partOf graph, which has no home.
"""

import json
import re
import sys
from pathlib import Path

from rdflib import Graph, Namespace, URIRef, Literal
from rdflib.namespace import RDF, DCTERMS

ODRL = Namespace("http://www.w3.org/ns/odrl/2/")
REPORT = Namespace("https://w3id.org/force/compliance-report#")
EX_SRC = Namespace("http://example.org/")

SUITE = Path(__file__).resolve().parent.parent.parent / "compliance" / "vendor" / "odrl-test-suite"

CONTEXT = {
    "odrl": {"@id": "http://www.w3.org/ns/odrl/2/", "@prefix": True},
    "xsd": {"@id": "http://www.w3.org/2001/XMLSchema#", "@prefix": True},
}


# --------------------------------------------------------------------------
# index.ttl -> the 68 cases, in the suite's own testcase-NNN order
# --------------------------------------------------------------------------
def parse_index():
    g = Graph()
    g.parse(SUITE / "data/index.ttl", format="turtle")
    entries = []
    for s in set(g.subjects()):
        def one(p):
            return next(g.objects(s, p), None)

        def local(u):
            return SUITE / str(u).split("/data/", 1)[1].join(["data/", ""])

        pol = str(one(EX_SRC.policySource))
        req = str(one(EX_SRC.requestSource))
        sotw = str(one(EX_SRC.sotwSource))
        exp = str(one(EX_SRC.expectedReportSource))
        rel = lambda u: SUITE / ("data/" + u.split("/data/", 1)[1])
        slug = Path(exp).stem
        entries.append(
            {
                "id": str(s),
                "slug": slug,
                "n": int(slug.split("-")[1]),
                "title": str(one(DCTERMS.title)),
                "policy": rel(pol),
                "request": rel(req),
                "sotw": rel(sotw),
                "expected": rel(exp),
            }
        )
    entries.sort(key=lambda e: e["n"])
    return entries


# --------------------------------------------------------------------------
# ground truth - a transliteration of compliance-runner/src/ground_truth.rs
# --------------------------------------------------------------------------
def expected_decision(path):
    g = Graph()
    g.parse(path, format="turtle")
    any_prohibition_active = False
    any_permission_active = False
    for s, o in g.subject_objects(REPORT.activationState):
        active = o == REPORT.Active
        types = set(g.objects(s, RDF.type))
        if REPORT.ProhibitionReport in types and active:
            any_prohibition_active = True
        elif REPORT.PermissionReport in types and active:
            any_permission_active = True
    if any_prohibition_active:
        return "Deny"
    if any_permission_active:
        return "Allow"
    return "Deny"


# --------------------------------------------------------------------------
# dateTime canonicalisation (see module docstring)
# --------------------------------------------------------------------------
_DT = re.compile(
    r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?"
    r"(Z|[+-]\d{2}:\d{2})?$"
)


def canonical_dt(value):
    m = _DT.match(value)
    if not m:
        raise ValueError("not an ISO dateTime this adapter canonicalises: %r" % value)
    y, mo, d, h, mi, s, ms, tz = m.groups()
    if tz not in (None, "Z", "+00:00", "-00:00"):
        raise ValueError("non-UTC offset, not handled: %r" % value)
    ms = (ms or "").ljust(3, "0")
    if len(ms) > 3:
        if ms[3:].strip("0"):
            raise ValueError("sub-millisecond precision, not handled: %r" % value)
        ms = ms[:3]
    return "%s-%s-%sT%s:%s:%s.%sZ" % (y, mo, d, h, mi, s, ms)


class Unsupported(Exception):
    pass


# --------------------------------------------------------------------------
# policy Turtle -> ODRL JSON-LD in ODRL-PAP's shape
# --------------------------------------------------------------------------
def node_id(g, node):
    """A collection (PartyCollection / AssetCollection) has no representation."""
    types = set(g.objects(node, RDF.type))
    if ODRL.PartyCollection in types or ODRL.AssetCollection in types:
        raise Unsupported(
            "S4: <%s> is an odrl:%s identified by odrl:source with membership "
            "asserted in the state-of-the-world graph (odrl:partOf). ODRL-PAP "
            "maps a collection only through OdrlMapper.mapRefinementCollection, "
            "which fails with \"No refinement contained in the collection.\" "
            "unless the collection carries an odrl:refinement; it has no "
            "channel for externally asserted membership."
            % (node, "PartyCollection" if ODRL.PartyCollection in types else "AssetCollection")
        )
    return str(node)


def constraint_json(g, node):
    # logical constraint?
    for op in ("and", "andSequence", "or", "xone"):
        members = list(g.objects(node, ODRL[op]))
        if members:
            return {
                "@type": "odrl:LogicalConstraint",
                "odrl:%s" % op: [constraint_json(g, m) for m in members],
            }
    left = next(g.objects(node, ODRL.leftOperand), None)
    oper = next(g.objects(node, ODRL.operator), None)
    right = next(g.objects(node, ODRL.rightOperand), None)
    if left is None or oper is None or right is None:
        raise Unsupported("constraint <%s> is neither atomic nor logical" % node)

    def qname(term):
        u = str(term)
        if u.startswith(str(ODRL)):
            return "odrl:" + u[len(str(ODRL)) :]
        raise Unsupported("non-odrl term %s" % u)

    out = {
        "@type": "odrl:Constraint",
        "odrl:leftOperand": {"@id": qname(left)},
        "odrl:operator": {"@id": qname(oper)},
    }
    if isinstance(right, Literal):
        dt = str(right.datatype) if right.datatype else None
        if dt == "http://www.w3.org/2001/XMLSchema#dateTime":
            out["odrl:rightOperand"] = {
                "@value": canonical_dt(str(right)),
                "@type": "xsd:dateTime",
            }
        elif dt:
            out["odrl:rightOperand"] = {
                "@value": str(right),
                "@type": "xsd:" + dt.rsplit("#", 1)[1],
            }
        else:
            out["odrl:rightOperand"] = {"@value": str(right)}
    else:
        out["odrl:rightOperand"] = {"@id": str(right)}
    return out


def policy_json(path):
    g = Graph()
    g.parse(path, format="turtle")
    sets = [s for s in g.subjects(RDF.type, ODRL.Set)]
    if len(sets) != 1:
        raise Unsupported("expected exactly one odrl:Set, found %d" % len(sets))
    pol = sets[0]

    prohibitions = list(g.objects(pol, ODRL.prohibition))
    permissions = list(g.objects(pol, ODRL.permission))
    if prohibitions:
        raise Unsupported(
            "S1: the policy carries %d odrl:prohibition rule(s). ODRL-PAP's "
            "OdrlMapper reads only odrl:permission (there is no occurrence of "
            "\"prohibition\" anywhere in src/main/java); a policy with no "
            "permission is rejected with \"The policy has no permission.\", "
            "and a prohibition alongside a permission would be silently dropped."
            % len(prohibitions)
        )
    if not permissions:
        raise Unsupported("S1: the policy has no odrl:permission at all")

    perms = []
    for p in permissions:
        action = next(g.objects(p, ODRL.action), None)
        target = next(g.objects(p, ODRL.target), None)
        assignee = next(g.objects(p, ODRL.assignee), None)
        if action is None:
            raise Unsupported(
                "S2: the permission declares no odrl:action; ODRL-PAP rejects it "
                "with \"The permission does not contain an action.\" "
                "(OdrlMapper.mapPermission)"
            )
        if target is None:
            raise Unsupported(
                "S3: the permission declares no odrl:target; ODRL-PAP rejects it "
                "with \"The permission does not contain a target.\" "
                "(OdrlMapper.mapPermission)"
            )
        pj = {
            "@type": "odrl:Permission",
            "odrl:action": {"@id": "odrl:" + str(action)[len(str(ODRL)) :]},
            "odrl:target": {"@id": node_id(g, target)},
        }
        if assignee is not None:
            pj["odrl:assignee"] = {"@id": node_id(g, assignee)}
        else:
            raise Unsupported(
                "the permission declares no odrl:assignee; ODRL-PAP rejects it "
                "with \"The permission does not contain an assignee.\""
            )
        constraint = next(g.objects(p, ODRL.constraint), None)
        if constraint is not None:
            pj["odrl:constraint"] = constraint_json(g, constraint)
        duty = next(g.objects(p, ODRL.duty), None)
        if duty is not None:
            # Passed through verbatim. ODRL-PAP has no duty concept and will
            # silently ignore the key - that silence is itself a result.
            da = next(g.objects(duty, ODRL.action), None)
            pj["odrl:duty"] = {
                "@type": "odrl:Duty",
                "odrl:action": {"@id": "odrl:" + str(da)[len(str(ODRL)) :]},
            }
        perms.append(pj)

    return {
        "@context": CONTEXT,
        "@id": str(pol),
        "odrl:uid": str(pol),
        "@type": "odrl:Set",
        "odrl:permission": perms if len(perms) > 1 else perms[0],
    }


# --------------------------------------------------------------------------
# request + sotw -> the PAP's generic-JSON input document
# --------------------------------------------------------------------------
def request_info(path):
    g = Graph()
    g.parse(path, format="turtle")
    req = next(g.subjects(RDF.type, ODRL.Request))
    perm = next(g.objects(req, ODRL.permission))
    action = next(g.objects(perm, ODRL.action))
    return {
        "assignee": str(next(g.objects(perm, ODRL.assignee))),
        "action": str(action)[len(str(ODRL)) :],
        "target": str(next(g.objects(perm, ODRL.target))),
    }


def sotw_current_time(path):
    g = Graph()
    g.parse(path, format="turtle")
    for o in g.objects(None, DCTERMS.issued):
        return canonical_dt(str(o))
    return None


def build_case(entry):
    gt = expected_decision(entry["expected"])
    try:
        policy = policy_json(entry["policy"])
    except Unsupported as e:
        return {"slug": entry["slug"], "title": entry["title"], "expected": gt,
                "skip": str(e)}
    ri = request_info(entry["request"])
    payload = {"action": ri["action"], "target": ri["target"],
               "assignee": ri["assignee"]}
    ct = sotw_current_time(entry["sotw"])
    if ct:
        payload["currentTime"] = ct
    return {
        "slug": entry["slug"],
        "title": entry["title"],
        "expected": gt,
        "validationRequest": {"policy": policy, "jsonInput": {"payload": payload}},
    }


if __name__ == "__main__":
    out = [build_case(e) for e in parse_index()]
    Path(sys.argv[1]).write_text(json.dumps(out, indent=1))
    print("cases: %d, translated: %d, skipped: %d"
          % (len(out), sum(1 for c in out if "skip" not in c),
             sum(1 for c in out if "skip" in c)))
