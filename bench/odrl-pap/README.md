# Bench: ODRL-PAP (`SEAMWARE/odrl-pap`)

FIWARE's own ODRL policy-administration component: it compiles ODRL policies
into Rego rules, and **Open Policy Agent (OPA)** is the actual runtime
decision point, fronted by APISIX as the enforcement point in a real
deployment. Benching it means benching ODRL→Rego translation plus OPA, not a
standalone ODRL evaluator library. Pinned commit:
`59e45474c910b97f537b8f39c68e2e17ec4243ef` = tag `1.7.0`.

## Real infrastructure requirement — read before attempting this one

Unlike the other three engines here, this is not a `git clone` + one runtime
away from working. A real reproduction needs, at minimum:

- **PostgreSQL** (ODRL-PAP's own persistence)
- **Open Policy Agent**, running and reachable
- **The Quarkus service itself**, built and running, exposing the
  `/validate` endpoint this harness calls (`http://localhost:8091/validate`
  by default — see `run_pap.py`/`probes.py`)

Standing this up is a real cost, not a quick check — this repository's own
`bench/README.md` says plainly that this engine was **not** given a full
clean-infrastructure reproduction in this pass, only a path-independence
check on the harness code itself.

```sh
git clone https://github.com/SEAMWARE/odrl-pap.git
cd odrl-pap
git checkout 59e45474c910b97f537b8f39c68e2e17ec4243ef
# Follow this project's own README/docker-compose for standing up
# PostgreSQL + OPA + the Quarkus service. Confirm /validate answers before
# running the harness below.
```

## Run

```sh
python3 translate_pap.py            # -> cases.json (translated + a 5-term mapping overlay,
                                     #    see the file's own header for exactly what it maps)
python3 run_pap.py cases.json results.json
```

`translate_pap.py` reads the vendored corpus directly from this repo's own
`compliance/vendor/odrl-test-suite` (resolved relative to this file's own
location — works from a fresh `ds-odrl-engine-rs` checkout without
modification).

## What's here

- `translate_pap.py` — translates the 68 vendored fixtures into ODRL-PAP's
  own validation-request JSON shape, with a documented mapping overlay
  (giving `odrl:dateTime` a fixture's own clock, since OPA's own
  `time.now_ns()` has no input channel otherwise) and an honest per-case
  `skip` reason for constructs this translator cannot represent at all
  (prohibitions — `OdrlMapper` reads only `odrl:permission` — and
  `PartyCollection`/`AssetCollection` membership).
- `run_pap.py` — posts each translated case to the running service's
  `/validate` endpoint and scores the response.
- `probes.py` — vocabulary/capability probes against the running service.
- `cases.json` — the translated corpus, committed as a point-in-time record
  (663 KB; regenerate with `translate_pap.py` rather than hand-editing).
- `results/results.json` — **30 pass, 1 fail, 37 skip**, with the mapping
  overlay engaged.
- `results/results-builtin-datetime.json` — **20 pass, 11 fail, 37 skip**,
  stock (no overlay) — `odrl:dateTime` compared against OPA's own
  `time.now_ns()`, mismatched against the fixtures' fixed clock.

## Not reproduced end to end in this pass

The harness's own file paths were checked for portability (no hardcoded
absolute path survived the rescue), but no full clean install of PostgreSQL +
OPA + Quarkus was performed to confirm these numbers against a fresh
environment. Treat them as carried forward from the original run.
