#!/usr/bin/env bash
#
# Drives the whole performance / resource / load pass for
# `ds-odrl-engine-rs` itself and writes the raw artifacts into
# `results/`.
#
# Each phase runs as its OWN process under `/usr/bin/time -v`, on purpose:
# "Maximum resident set size" and the CPU-seconds counters that tool
# reports are per-process, so folding four phases into one binary
# invocation would make every memory number the maximum of a mixture and
# attributable to nothing. The per-phase `.time.txt` files beside each
# `.json` are that tool's verbatim output, kept as the raw evidence for
# every resource figure quoted in this directory's README.
#
# Usage:
#
#   bench/ds-odrl-engine-rs/run.sh [RESULTS_DIR]
#
# Environment:
#   CARGO_TARGET_DIR  where to build (default: a scratch dir, NOT the
#                     repo's own target/, so this bench never disturbs the
#                     workspace build the compliance gates use)
#   CASES_JSON        override the corpus (default:
#                     compliance/reports/latest-cases.json)
#   ENGINE_WASM       override the wasm artifact
#
# RUN THIS ON A QUIET MACHINE. Steps 4 and 5 saturate every core; a
# concurrent build or another engine's bench running at the same time
# invalidates the throughput curve and the latency tails alike.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
results="${1:-$here/results}"
mkdir -p "$results"

# Two build trees, deliberately kept apart:
#   - engine.wasm goes to the repo's own `target/`, where every other
#     consumer of it (site, release-history, the default ENGINE_WASM path
#     below) already looks;
#   - the bench harness goes to its own tree, so `wasmi` and a bench-only
#     binary never enter the workspace build the compliance gates use.
bench_target="${CARGO_TARGET_DIR:-$here/perf/target}"
export ENGINE_COMMIT="$(git -C "$repo_root" rev-parse HEAD)"

echo "== environment"
{
  echo "date:      $(date -Is)"
  echo "host:      $(uname -srmo)"
  echo "nproc:     $(nproc)"
  echo "uptime:    $(uptime)"
  echo "cargo:     $(cargo --version)"
  echo "rustc:     $(rustc --version)"
  echo "commit:    $ENGINE_COMMIT"
  echo
  free -h
  echo
  lscpu | sed -n '1,20p'
} | tee "$results/environment.txt"

echo
echo "== build: engine.wasm (the (b) WASM-ABI artifact under test)"
CARGO_TARGET_DIR="$repo_root/target" /usr/bin/time -v -o "$results/build-engine-wasm.time.txt" \
  cargo build --manifest-path "$repo_root/Cargo.toml" -p engine \
  --target wasm32-unknown-unknown --release > "$results/build-engine-wasm.log" 2>&1

echo "== build: perf-bench harness"
CARGO_TARGET_DIR="$bench_target" /usr/bin/time -v -o "$results/build-perf-bench.time.txt" \
  cargo build --manifest-path "$here/perf/Cargo.toml" --release \
  > "$results/build-perf-bench.log" 2>&1

bin="$bench_target/release/perf-bench"
ls -l "$bin"

run_phase() {
  local phase="$1"
  echo
  echo "== phase: $phase"
  # `-o FILE` rather than a stderr redirect: the harness writes its own
  # progress to stderr, and interleaving the two would make the raw
  # resource evidence unparseable.
  /usr/bin/time -v -o "$results/$phase.time.txt" "$bin" "$phase" "$results/$phase.json"
}

run_phase native
run_phase wasm
run_phase load-native
run_phase load-wasm

echo
echo "== disk footprint"
{
  echo "engine.wasm:                 $(du -h "$repo_root/target/wasm32-unknown-unknown/release/engine.wasm" | cut -f1)"
  echo "wasm32 release build tree:   $(du -sh "$repo_root/target/wasm32-unknown-unknown/release" | cut -f1)"
  echo "perf-bench build tree:       $(du -sh "$bench_target" | cut -f1)"
  echo "cargo registry (deps cache): $(du -sh "${CARGO_HOME:-$HOME/.cargo}/registry" 2>/dev/null | cut -f1)"
} | tee "$results/footprint.txt"

echo
echo "done -> $results"
