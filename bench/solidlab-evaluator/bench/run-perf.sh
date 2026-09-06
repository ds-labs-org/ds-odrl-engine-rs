#!/usr/bin/env bash
# Drive the whole perf/resource/load pass for one pinned odrl-evaluator
# version, from a fresh checkout of the suite, and write every raw artefact
# into RESULTS.
#
#   VERSION=0.4.0 SCRATCH=/tmp/.../solidlab-evaluator \
#   RESULTS=/path/to/bench/solidlab-evaluator/results \
#   BENCH_SRC=/path/to/bench/solidlab-evaluator/bench \
#   bash run-perf.sh
#
# Steps, in order:
#   0. record the environment and the baseline load BEFORE anything heavy runs
#   1. clone the suite (once per SCRATCH) and `npm install odrl-evaluator@VERSION`,
#      both timed with /usr/bin/time -v
#   2. re-run the CONFORMANCE harness unchanged, to prove this checkout still
#      produces the committed 63/68 (0.4.0) or 67/68 (0.6.0) before any perf
#      number from it is trusted
#   3. perf-bench.ts under /usr/bin/time -v  -> peak RSS + CPU seconds
#   4. load-bench.ts  (its own kernel-level sampling; also under time -v)
#   5. on-disk footprint of node_modules and of the odrl-evaluator package
#
# Nothing here runs in the background and nothing survives the script: the
# load ramp's worker processes are killed by load-bench.ts itself, and this
# script sweeps any stragglers at the end.
set -euo pipefail

VERSION="${VERSION:?set VERSION=0.4.0 or 0.6.0}"
SCRATCH="${SCRATCH:?set SCRATCH to an isolated working directory}"
RESULTS="${RESULTS:?set RESULTS to bench/solidlab-evaluator/results}"
BENCH_SRC="${BENCH_SRC:?set BENCH_SRC to bench/solidlab-evaluator/bench}"
SUITE="$SCRATCH/ODRL-Test-Suite"
TAG="${VERSION//./}"   # 0.4.0 -> 040

mkdir -p "$RESULTS" "$SCRATCH"

say() { printf '\n=== %s ===\n' "$1" >&2; }

# ---- 0. environment + baseline load, before anything heavy ------------------
say "environment (baseline, version $VERSION)"
{
    echo "date_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "uname: $(uname -srmo)"
    echo "nproc: $(nproc)"
    echo "node: $(node --version)"
    echo "npm: $(npm --version)"
    echo "target odrl-evaluator: $VERSION"
    echo "--- uptime ---"; uptime
    echo "--- free -h ---"; free -h
    echo "--- top CPU consumers before the run ---"
    ps -eo pid,ppid,user,pcpu,pmem,rss,etime,comm --sort=-pcpu | head -15
} > "$RESULTS/environment-$TAG.txt" 2>&1
cat "$RESULTS/environment-$TAG.txt" >&2

# ---- 1. clone + install -----------------------------------------------------
if [ ! -d "$SUITE/.git" ]; then
    say "clone"
    /usr/bin/time -v git clone https://github.com/SolidLabResearch/ODRL-Test-Suite.git "$SUITE" \
        2> "$RESULTS/clone.time.txt"
fi
git -C "$SUITE" log -1 --format='%H %ci %s' > "$RESULTS/suite-commit.txt"

mkdir -p "$SUITE/bench"
cp "$BENCH_SRC"/*.ts "$SUITE/bench/"

say "npm install odrl-evaluator@$VERSION"
( cd "$SUITE" && /usr/bin/time -v npm install --no-audit --no-fund "odrl-evaluator@$VERSION" \
    > "$RESULTS/npm-install-$TAG.log" 2> "$RESULTS/npm-install-$TAG.time.txt" )
( cd "$SUITE" && node -e "console.log(require('odrl-evaluator/package.json').version)" ) \
    > "$RESULTS/installed-version-$TAG.txt"

# ---- 2. conformance re-check on THIS checkout -------------------------------
say "conformance re-check"
( cd "$SUITE" && OUT="$RESULTS/verify-conformance-$TAG.json" npx ts-node bench/allow-deny-bench.ts ) \
    2>&1 | tail -3 >&2

# ---- 3. per-case latency + resource consumption -----------------------------
say "perf-bench (under /usr/bin/time -v)"
( cd "$SUITE" && OUT="$RESULTS/perf-$TAG.json" \
    /usr/bin/time -v npx ts-node bench/perf-bench.ts 2> "$RESULTS/perf-$TAG.time.txt" ) || true
tail -25 "$RESULTS/perf-$TAG.time.txt" >&2

# ---- 4. load ramp -----------------------------------------------------------
say "load-bench"
( cd "$SUITE" && OUT="$RESULTS/load-$TAG.json" \
    /usr/bin/time -v npx ts-node bench/load-bench.ts 2> "$RESULTS/load-$TAG.time.txt" ) || true
tail -25 "$RESULTS/load-$TAG.time.txt" >&2

# ---- 5. footprint -----------------------------------------------------------
say "footprint"
{
    echo "suite checkout (incl. .git, node_modules):"; du -sh "$SUITE"
    echo "node_modules total:";                        du -sh "$SUITE/node_modules"
    echo "odrl-evaluator package:";                    du -sh "$SUITE/node_modules/odrl-evaluator"
    echo "odrl-evaluator dist:";                       du -sh "$SUITE/node_modules/odrl-evaluator/dist" 2>/dev/null || true
    echo "eye WASM (the reasoner):"
    find "$SUITE/node_modules" -iname '*.wasm' -printf '%10s  %p\n' 2>/dev/null | sort -rn | head -10
    echo "installed package count:"; ls "$SUITE/node_modules" | wc -l
} > "$RESULTS/footprint-$TAG.txt" 2>&1
cat "$RESULTS/footprint-$TAG.txt" >&2

# ---- 6. sweep ---------------------------------------------------------------
say "cleanup"
pkill -f 'load-worker\.ts' 2>/dev/null || true
sleep 1
pgrep -af 'load-worker\.ts|perf-bench\.ts|load-bench\.ts' >&2 || echo "no stragglers" >&2
