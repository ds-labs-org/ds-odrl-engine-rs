#!/usr/bin/env bash
# Driver that produced every file in results/. Runs strictly sequentially and
# starts nothing in the background: performance, resource and load numbers
# taken while something else is competing for this machine are not
# measurements, they are noise.
#
#   SCRATCH=/some/isolated/dir \
#   RESULTS=<bench/odrl-manager>/results \
#   BENCH_SRC=<bench/odrl-manager> \
#   CORPUS=<ds-odrl-engine-rs>/compliance/vendor/odrl-test-suite/data \
#   bash run_perf.sh
#
# It clones odrl-manager fresh at the pinned commit, installs, copies this
# directory's harness into src/bench/ (odrl-manager's tsconfig baseUrl forces
# that -- see README), re-runs the CONFORMANCE harness first so the perf
# numbers are anchored to a reproduced 61/68, then runs perf_bench.ts and
# load_bench.ts under /usr/bin/time -v, then sweeps for stragglers.
set -euo pipefail

COMMIT=8842b6b9ff9fa580f9400f426a5f361f526dbd9b
SCRATCH=${SCRATCH:?set SCRATCH to an isolated scratch directory}
BENCH_SRC=${BENCH_SRC:?set BENCH_SRC to this bench/odrl-manager directory}
RESULTS=${RESULTS:?set RESULTS to where the raw JSON should land}
CORPUS=${CORPUS:?set CORPUS to the odrl-test-suite data directory}
LEVELS=${LEVELS:-1,2,4,8,16,22,32,44}
STEP_MS=${STEP_MS:-10000}
RAMPS=${RAMPS:-3}
PERF_REPEATS=${PERF_REPEATS:-20}
WARMUP=${WARMUP:-5}
PERF_PROCESS_REPEATS=${PERF_PROCESS_REPEATS:-3}

mkdir -p "$SCRATCH" "$RESULTS"
REPO="$SCRATCH/odrl-manager"
NODE_RUN=(npx ts-node -r tsconfig-paths/register)

# 0. Environment and baseline load, BEFORE anything heavy runs.
{
  echo "=== date ==="; date -Is
  echo "=== uptime (baseline load) ==="; uptime
  echo "=== nproc ==="; nproc
  echo "=== free -h ==="; free -h
  echo "=== MemAvailable ==="; grep MemAvailable /proc/meminfo
  echo "=== kernel ==="; uname -a
  echo "=== cpu ==="; lscpu | grep -E 'Model name|^CPU\(s\)|Thread|Core'
  echo "=== node ==="; node --version
  echo "=== npm ==="; npm --version
  echo "=== git ==="; git --version
  echo "=== ps (sorted by cpu) ==="; ps aux --sort=-%cpu | head -25
} > "$RESULTS/environment.txt" 2>&1

# 1. Fresh clone at the pinned commit.
rm -rf "$REPO"
/usr/bin/time -v git clone --quiet https://github.com/Prometheus-X-association/odrl-manager.git "$REPO" \
  2> "$RESULTS/clone.time.txt"
git -C "$REPO" checkout --quiet "$COMMIT"
{ git -C "$REPO" rev-parse HEAD; git -C "$REPO" log -1 --format='%H%n%ad%n%s'; } > "$RESULTS/pinned-commit.txt"

# 2. Install. Cold cache first (real from-nothing figure), then warm.
export NPM_CONFIG_CACHE="$SCRATCH/npm-cache-cold"
rm -rf "$NPM_CONFIG_CACHE"
cd "$REPO"
/usr/bin/time -v npm install --no-audit --no-fund \
  > "$RESULTS/npm-install-coldcache.log" 2> "$RESULTS/npm-install-coldcache.time.txt"
rm -rf node_modules
/usr/bin/time -v npm install --no-audit --no-fund \
  > "$RESULTS/npm-install-warmcache.log" 2> "$RESULTS/npm-install-warmcache.time.txt"
# n3/@types/n3 are this harness's dependency, not odrl-manager's.
/usr/bin/time -v npm install --no-save --no-audit --no-fund n3 @types/n3 \
  > "$RESULTS/npm-install-n3.log" 2> "$RESULTS/npm-install-n3.time.txt"

# 3. The library's own build. NOT on the measured path (everything below runs
#    from src/ through ts-node, exactly as the conformance harness does), but
#    it is what "build this engine" costs if a host wants dist/.
/usr/bin/time -v npm run build > "$RESULTS/build.log" 2> "$RESULTS/build.time.txt"

# 4. Harness into place.
mkdir -p "$REPO/src/bench"
cp "$BENCH_SRC"/*.ts "$REPO/src/bench/"

# 5. On-disk footprint.
{
  echo "=== du -sh checkout (incl node_modules) ==="; du -sh .
  echo "=== du -sh node_modules ==="; du -sh node_modules
  echo "=== du -sh src ==="; du -sh src
  echo "=== du -sh dist (built artifact) ==="; du -sh dist
  echo "=== du -sh .git ==="; du -sh .git
  echo "=== node_modules file count ==="; find node_modules -type f | wc -l
  echo "=== largest packages ==="; du -sh node_modules/* node_modules/@*/* 2>/dev/null | sort -rh | head -15
  # The scratch path is deliberately rewritten out: this file is a record of
  # sizes, not of where one person happened to run it.
  echo "=== npm ls --depth=0 ==="; npm ls --depth=0 2>&1 | head -40 | sed "s#$REPO#<checkout>#g"
} > "$RESULTS/footprint.txt" 2>&1

# 6. Conformance re-run FIRST: the perf numbers below are only worth anything
#    if this checkout still scores what the README says it scores.
for MODE in native assisted; do
  ODRL_TEST_SUITE_DATA="$CORPUS" OUT="$RESULTS/verify-conformance-$MODE.txt" \
    /usr/bin/time -v "${NODE_RUN[@]}" src/bench/run.ts "$MODE" \
    > "$RESULTS/verify-conformance-$MODE.stdout.txt" 2> "$RESULTS/verify-conformance-$MODE.time.txt"
done

# 7. Performance + resources. PERF_PROCESS_REPEATS separate processes, so
#    process-level variance is visible on top of the in-process repeats.
for i in $(seq 1 "$PERF_PROCESS_REPEATS"); do
  ODRL_TEST_SUITE_DATA="$CORPUS" OUT="$RESULTS/perf-native-p$i.json" \
    /usr/bin/time -v "${NODE_RUN[@]}" src/bench/perf_bench.ts native \
      --repeats "$PERF_REPEATS" --warmup "$WARMUP" \
    > "$RESULTS/perf-native-p$i.stdout.txt" 2> "$RESULTS/perf-native-p$i.time.txt"
done
# A long soak, to tell V8 heap growth apart from an actual leak: the short runs
# above show RSS rising monotonically, which on its own proves nothing.
ODRL_TEST_SUITE_DATA="$CORPUS" OUT="$RESULTS/perf-native-soak.json" \
  /usr/bin/time -v "${NODE_RUN[@]}" src/bench/perf_bench.ts native \
    --repeats "${SOAK_REPEATS:-400}" --warmup "$WARMUP" --no-raw-samples \
  > "$RESULTS/perf-native-soak.stdout.txt" 2> "$RESULTS/perf-native-soak.time.txt"

ODRL_TEST_SUITE_DATA="$CORPUS" OUT="$RESULTS/perf-assisted.json" \
  /usr/bin/time -v "${NODE_RUN[@]}" src/bench/perf_bench.ts assisted \
    --repeats "$PERF_REPEATS" --warmup "$WARMUP" \
  > "$RESULTS/perf-assisted.stdout.txt" 2> "$RESULTS/perf-assisted.time.txt"

# 8. Load ramp.
ODRL_TEST_SUITE_DATA="$CORPUS" OUT="$RESULTS/load.json" \
  /usr/bin/time -v "${NODE_RUN[@]}" src/bench/load_bench.ts native \
    --levels "$LEVELS" --step-ms "$STEP_MS" --ramps "$RAMPS" \
  > "$RESULTS/load.stdout.txt" 2> "$RESULTS/load.time.txt"

# 9. Straggler sweep. Match on the process's own comm being `node` so an
#    ancestor shell whose command line merely mentions these filenames is not
#    miscounted as a leftover worker.
{
  echo "=== date ==="; date -Is
  echo "=== straggler node processes from this bench ==="
  ps -eo pid,comm,args --no-headers | awk '$2=="node" && /load_worker|load_bench|perf_bench/' || true
  echo "=== count ==="
  ps -eo pid,comm,args --no-headers | awk '$2=="node" && /load_worker|load_bench|perf_bench/' | wc -l
  echo "=== uptime ==="; uptime
  echo "=== free -h ==="; free -h
} > "$RESULTS/sweep.txt" 2>&1

echo "done; results in $RESULTS"
cat "$RESULTS/sweep.txt"
