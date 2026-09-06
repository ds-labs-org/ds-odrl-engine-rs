#!/usr/bin/env python3
"""Re-derives the vendored ODRL-Test-Suite's per-case Allow/Deny ground truth,
mirroring ds-odrl-engine-rs's compliance-runner/src/index.rs (case enumeration)
and ground_truth.rs (FORCE report -> Allow/Deny reduction) exactly.

index.rs:  read data/index.ttl, one urn:uuid subject per case; dct:title plus
           ex:policySource / ex:requestSource / ex:sotwSource /
           ex:expectedReportSource raw.githubusercontent URLs rewritten to local
           vendored paths; ordered by the testcase-NNN number in the expected
           report's filename; slug = that filename's stem.

ground_truth.rs: over the expected report graph --
           any report:ProhibitionReport with report:activationState report:Active
             -> Deny (deny-overrides)
           else any report:PermissionReport Active -> Allow
           else -> Deny (ODRL Formal Semantics 'closed' default)
           report:DutyReport rule-reports are ignored.
"""
import json
import sys
from pathlib import Path

from rdflib import Graph, Namespace, RDF, URIRef
from rdflib.namespace import DCTERMS

VENDOR = Path(__file__).resolve().parent.parent.parent / "compliance" / "vendor" / "odrl-test-suite"
EX = Namespace("http://example.org/")
REPORT = Namespace("https://w3id.org/force/compliance-report#")


def rewrite(url: str) -> Path:
    """index.rs::rewrite_to_local_path -- everything from '/data/' onward."""
    marker = "/data/"
    i = url.find(marker)
    if i < 0:
        raise ValueError(f"source URL does not contain {marker!r}: {url}")
    return VENDOR / url[i + 1 :]


def expected_decision(report_path: Path) -> str:
    """ground_truth.rs::expected_decision, verbatim in its logic."""
    g = Graph()
    g.parse(report_path, format="turtle")
    any_prohibition_active = False
    any_permission_active = False
    for subj, obj in g.subject_objects(REPORT.activationState):
        is_active = obj == REPORT.Active
        if not is_active:
            continue
        types = set(g.objects(subj, RDF.type))
        if REPORT.ProhibitionReport in types:
            any_prohibition_active = True
        elif REPORT.PermissionReport in types:
            any_permission_active = True
    if any_prohibition_active:
        return "Deny"
    if any_permission_active:
        return "Allow"
    return "Deny"


def main() -> None:
    idx = Graph()
    idx.parse(VENDOR / "data/index.ttl", format="turtle")

    cases = []
    for subj in set(idx.subjects()):
        title = idx.value(subj, DCTERMS.title)
        pol = idx.value(subj, EX.policySource)
        req = idx.value(subj, EX.requestSource)
        sotw = idx.value(subj, EX.sotwSource)
        exp = idx.value(subj, EX.expectedReportSource)
        if not all(x is not None for x in (title, pol, req, sotw, exp)):
            raise SystemExit(f"{subj}: incomplete index entry")
        exp_path = rewrite(str(exp))
        slug = exp_path.stem
        seq = int(slug.split("-")[1])
        cases.append(
            {
                "seq": seq,
                "id": str(subj),
                "slug": slug,
                "title": str(title),
                "policy": str(rewrite(str(pol))),
                "request": str(rewrite(str(req))),
                "sotw": str(rewrite(str(sotw))),
                "expected_report": str(exp_path),
                "expected_decision": expected_decision(exp_path),
            }
        )

    cases.sort(key=lambda c: c["seq"])
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("ground_truth.json")
    out.write_text(json.dumps(cases, indent=2))
    allow = sum(1 for c in cases if c["expected_decision"] == "Allow")
    print(f"{len(cases)} cases -> {out}  (Allow={allow}, Deny={len(cases)-allow})")


if __name__ == "__main__":
    main()
