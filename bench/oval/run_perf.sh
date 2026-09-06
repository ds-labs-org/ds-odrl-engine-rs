#!/usr/bin/env bash
# Driver that produced everything in results/ whose name is not results_[AB]_*.json.
# Runs strictly sequentially and starts nothing in the background: performance,
# resource and load numbers taken while something else is competing for this
# machine are not measurements, they are noise.
#
#   SCRATCH=/some/isolated/dir \
#   RESULTS=<bench/oval>/results \
#   BENCH_SRC=<bench/oval> \
#   bash run_perf.sh
#
# Optional: COLD_PIP=1 additionally times a --no-cache-dir install in a
# throwaway venv, for a network-cold dependency-resolution figure.
set -euo pipefail

COMMIT=a427e71b50bdd14027f2d5552d6ce03d089487f3
SCRATCH=${SCRATCH:?set SCRATCH to an isolated scratch directory}
BENCH_SRC=${BENCH_SRC:?set BENCH_SRC to this bench/oval directory}
RESULTS=${RESULTS:?set RESULTS to where the raw JSON should land}
LEVELS=${LEVELS:-1,2,4,8,16,22,32,44}
STEP_S=${STEP_S:-10}
RAMP_REPEATS=${RAMP_REPEATS:-3}
PERF_REPEATS=${PERF_REPEATS:-5}
WARMUP=${WARMUP:-10}

mkdir -p "$SCRATCH" "$RESULTS"
cd "$SCRATCH"
PY="$SCRATCH/odrl-Engine/venv/bin/python"

# 0. Environment and baseline load, BEFORE anything heavy runs.
{
  echo "=== date ==="; date -Is
  echo "=== uptime (baseline load) ==="; uptime
  echo "=== nproc ==="; nproc
  echo "=== free -h ==="; free -h
  echo "=== MemAvailable ==="; grep MemAvailable /proc/meminfo
  echo "=== kernel ==="; uname -a
  echo "=== cpu ==="; lscpu | grep -E 'Model name|^CPU\(s\)|Thread|Core'
  echo "=== python3 ==="; python3 -VV
  echo "=== git ==="; git --version
  echo "=== ps, top 15 by cpu ==="; ps aux --sort=-%cpu | head -16
} > "$RESULTS/environment.txt" 2>&1

# 1. Fresh clone at the pinned commit -- never reuse a stale scratch checkout.
rm -rf "$SCRATCH/odrl-Engine"
/usr/bin/time -v git clone https://github.com/DIPS-Tools/odrl-Engine.git odrl-Engine \
  > "$RESULTS/clone.time.txt" 2>&1
git -C odrl-Engine checkout "$COMMIT" >/dev/null 2>&1
git -C odrl-Engine rev-parse HEAD > "$RESULTS/pinned-commit.txt"

# 2. venv + dependency install, both timed. requirements.txt is UNPINNED
#    upstream, so pip freeze is recorded as the version set actually measured.
cd "$SCRATCH/odrl-Engine"
/usr/bin/time -v python3 -m venv venv > "$RESULTS/venv-create.time.txt" 2>&1
/usr/bin/time -v ./venv/bin/pip install -r requirements.txt \
  > "$RESULTS/pip-install-warmcache.log" 2>&1
if [ "${COLD_PIP:-0}" = 1 ]; then
  cd "$SCRATCH"
  python3 -m venv coldvenv
  /usr/bin/time -v ./coldvenv/bin/pip install --no-cache-dir -r odrl-Engine/requirements.txt \
    > "$RESULTS/pip-install-nocache.log" 2>&1
  rm -rf "$SCRATCH/coldvenv"
fi
cd "$SCRATCH"

# 3. Harness in place BESIDE the clone, exactly as bench/oval/README.md says.
cp "$BENCH_SRC"/*.py "$SCRATCH/"
"$PY" "$BENCH_SRC/ground_truth.py" "$SCRATCH/ground_truth.json"

# 4. On-disk footprint of what step 2 produced.
{
  echo "=== whole checkout, incl .git + venv ==="; du -sh "$SCRATCH/odrl-Engine"
  echo "=== venv ==="; du -sh "$SCRATCH/odrl-Engine/venv"
  echo "=== site-packages ==="; du -sh "$SCRATCH"/odrl-Engine/venv/lib/python*/site-packages
  echo "=== source tree only (no venv, no .git) ==="
  du -sh --exclude=venv --exclude=.git "$SCRATCH/odrl-Engine"
  echo "=== .git ==="; du -sh "$SCRATCH/odrl-Engine/.git"
  echo "=== 15 largest installed packages ==="
  # `| head` makes sort die of SIGPIPE, which under `set -e` kills this whole
  # script -- found the hard way on the first run of this driver.
  du -sh "$SCRATCH"/odrl-Engine/venv/lib/python*/site-packages/* | sort -rh | head -15 || true
  echo "=== distributions installed ==="
  ls -d "$SCRATCH"/odrl-Engine/venv/lib/python*/site-packages/*.dist-info | wc -l
  echo "=== upstream corpus ==="; du -sh "$SCRATCH/odrl-Engine/test_cases/evaluation/force"
  echo "=== pip freeze ==="; "$SCRATCH/odrl-Engine/venv/bin/pip" freeze
  echo "=== venv python ==="; "$PY" -VV
} > "$RESULTS/footprint.txt" 2>&1

# 5. CONFORMANCE FIRST. The perf numbers are only worth anything if this
#    checkout still answers the way the 59/68 tally says it does.
"$PY" bench.py odrl-Engine/test_cases/evaluation/force \
  "$RESULTS/verify-conformance-A.json" > "$RESULTS/verify-conformance-A.txt" 2>&1 || true
"$PY" bench.py odrl-Engine/test_cases/evaluation/force \
  "$RESULTS/verify-conformance-B.json" --isolate > "$RESULTS/verify-conformance-B.txt" 2>&1 || true

# 6. Performance + in-process resources, both evaluation paths, one process each.
/usr/bin/time -v "$PY" perf_bench.py "$RESULTS/perf-isolated.json" \
  --repeats "$PERF_REPEATS" --warmup "$WARMUP" > "$RESULTS/perf-isolated.time.txt" 2>&1
/usr/bin/time -v "$PY" perf_bench.py "$RESULTS/perf-upstream.json" \
  --repeats "$PERF_REPEATS" --warmup "$WARMUP" --no-isolate \
  > "$RESULTS/perf-upstream.time.txt" 2>&1
# Controlled test of the periodic latency spike the first two runs show; see
# perf_bench.py's own note on --gc-off.
/usr/bin/time -v "$PY" perf_bench.py "$RESULTS/perf-isolated-nogc.json" \
  --repeats "$PERF_REPEATS" --warmup "$WARMUP" --gc-off \
  > "$RESULTS/perf-isolated-nogc.time.txt" 2>&1

# 7. Load ramp. One unit of concurrency = one OS process; see load_bench.py.
/usr/bin/time -v "$PY" load_bench.py "$RESULTS/load.json" \
  --levels "$LEVELS" --step-s "$STEP_S" --repeats "$RAMP_REPEATS" \
  > "$RESULTS/load.time.txt" 2>&1

# 8. Sweep. Nothing this script starts may outlive it.
pkill -f 'load_worker|load_bench|perf_bench' 2>/dev/null || true
sleep 1
{
  echo "=== date ==="; date -Is
  echo "=== stragglers (expect 0) ==="
  ps -eo args | grep -cE '[l]oad_worker|[l]oad_bench|[p]erf_bench' || true
  echo "=== uptime after ==="; uptime
  echo "=== free -h after ==="; free -h
} > "$RESULTS/sweep.txt" 2>&1
cat "$RESULTS/sweep.txt"
