#!/usr/bin/env python3
"""Feed every translated case to ODRL-PAP's own /validate endpoint and compare
the decision it produces with the SAME ground truth ground_truth.rs derives."""
import json
import sys
import urllib.request

PAP = "http://localhost:8091/validate"


def validate(vr):
    body = json.dumps(vr).encode()
    req = urllib.request.Request(
        PAP, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())


cases = json.load(open(sys.argv[1]))
results = []
for c in cases:
    if "skip" in c:
        results.append(dict(slug=c["slug"], title=c["title"],
                            expected=c["expected"], status="skipped",
                            reason=c["skip"]))
        continue
    try:
        resp = validate(c["validationRequest"])
        actual = "Allow" if resp.get("allow") else "Deny"
        status = "passed" if actual == c["expected"] else "failed"
        results.append(dict(slug=c["slug"], title=c["title"],
                            expected=c["expected"], actual=actual,
                            status=status, raw=resp))
    except Exception as e:
        detail = ""
        if hasattr(e, "read"):
            try:
                detail = e.read().decode()[:400]
            except Exception:
                pass
        results.append(dict(slug=c["slug"], title=c["title"],
                            expected=c["expected"], status="error",
                            reason="%s %s" % (e, detail)))
    print(results[-1]["slug"], results[-1]["status"],
          results[-1].get("actual", ""), flush=True)

json.dump(results, open(sys.argv[2], "w"), indent=1)
n = lambda s: sum(1 for r in results if r["status"] == s)
print("\ntotal %d | passed %d | failed %d | skipped %d | error %d"
      % (len(results), n("passed"), n("failed"), n("skipped"), n("error")))
