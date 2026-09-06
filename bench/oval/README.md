# Bench: OVAL (`DIPS-Tools/odrl-Engine`)

Python, RDF-reasoning-based ODRL evaluator. Pinned commit:
`a427e71b50bdd14027f2d5552d6ce03d089487f3`.

## Setup

```sh
git clone https://github.com/DIPS-Tools/odrl-Engine.git odrl-Engine
cd odrl-Engine
git checkout a427e71b50bdd14027f2d5552d6ce03d089487f3
python3 -m venv venv
./venv/bin/pip install -r requirements.txt
cd ..
cp <this-directory>/*.py .        # ground_truth.py, bench.py, probes*.py, retranslate.py --
                                   # placed BESIDE the odrl-Engine/ clone, not inside it
```

## Run

```sh
# 1. Generate the shared ground truth this bench and ds-odrl-engine-rs's own
#    compliance-runner are checked against, re-derived independently from the
#    fixtures' own report:* graphs (not copied from anywhere).
python3 ground_truth.py
#   -> ground_truth.json (68 cases, Allow=27, Deny=41)

# 2. Run the bench against OVAL's own upstream-committed, pre-translated
#    corpus (extracted_<slug>.ttl / .csv pairs already shipped in the clone).
./venv/bin/python bench.py odrl-Engine/test_cases/evaluation/force results.json --isolate
```

`corpus_dir` must point at a directory of `extracted_<slug>.ttl`/`.csv` pairs
in OVAL's own pre-translated format — **not** the raw ODRL-Test-Suite
fixtures. `odrl-Engine/test_cases/evaluation/force` (upstream's own committed
corpus) is the simplest choice; `retranslate.py` (below) produces an
independent one from the local vendored suite instead, to prove the result
isn't an artifact of OVAL's own possibly-stale committed translation.

```sh
# Optional: retranslate the LOCAL vendored corpus independently, to prove
# the result above isn't just OVAL's own committed corpus agreeing with itself.
git clone https://github.com/SolidLabResearch/ODRL-Test-Suite.git   # beside odrl-Engine/
python3 retranslate.py
./venv/bin/python bench.py retranslated/test_cases/evaluation/force results_retranslated.json --isolate
```

`--isolate` passes fresh `ontology_files`/`ontology_graphs` lists per case;
omit it to reproduce upstream's own `test_on_force.py` conditions verbatim
(see `bench.py`'s own header comment for the real, found-the-hard-way reason
this flag exists — a mutable-default-argument bug that leaks state across a
loop in the same process).

## What's here

- `ground_truth.py` — independent Allow/Deny re-derivation from the vendored
  suite's own `report:*` graphs, matching `ground_truth.rs` exactly.
- `bench.py` — the harness. Its own header comment documents two real,
  found-the-hard-way environment facts about upstream `ODRL_Evaluator.py`:
  a cwd-relative ontology path (`ODRL/ODRL22.ttl`) that silently breaks
  action-taxonomy reasoning from the wrong working directory, and the
  mutable-default-argument state leak `--isolate` exists to defeat.
- `probes.py`, `probes2.py` — vocabulary/capability probes.
- `retranslate.py` — re-translates the LOCAL vendored corpus into OVAL's own
  input format independently of its committed one, calling upstream's real
  `parse_test_cases_from_md` directly (its own header documents a genuine
  upstream bug: `FORCE_translator.py`'s `__main__` block calls the
  *reverse*-direction function and dies on the suite's own documentation
  files if invoked as its own README suggests).
- `results/results_A_upstream.json`, `results/results_B_isolated.json` — **59
  pass** out of 68, both with and without `--isolate` engaged.

## Reproduced

Verified with a fresh `git clone` + venv install in an isolated scratch
location, run against upstream's own committed corpus: **59/68**, exact match
to the committed result.
