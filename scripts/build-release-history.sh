#!/usr/bin/env bash
#
# Stage 1 of 2 of the Release History dashboard's data pipeline.
#
# Re-executes this repo's own machinery against every tagged release, and
# leaves the raw results in a staging directory for `release-history` (the
# native generator crate, stage 2) to analyse and turn into
# `compliance/reports/release-history.json`.
#
# For each tag reachable from `git tag -l`, this script:
#
#   1. checks the tag out in a *detached, isolated git worktree* (never in
#      your own working tree -- the whole point is that this can run while
#      you have uncommitted work in progress, and that a historical
#      `cargo run -p compliance-runner` cannot overwrite your
#      `compliance/reports/`);
#   2. builds `engine.wasm` for `wasm32-unknown-unknown --release` at that
#      tag, and copies the binary into the staging directory;
#   3. builds and runs `compliance-runner --release` at that tag, against
#      the ODRL-Test-Suite revision *that tag itself pinned*, and copies
#      that run's own `compliance/reports/latest.json` into the staging
#      directory. This is the release's real historical pass rate, not a
#      number re-derived today.
#
# Nothing here judges anything: it produces inputs. Stage 2 drives each of
# these historical `engine.wasm` binaries through its own four-export C
# ABI with a wasm interpreter, using the *current* probe catalog -- see
# `release-history/src/main.rs`.
#
# Usage:
#
#   scripts/build-release-history.sh [STAGE_DIR] [WORKTREE_DIR]
#
# Both default under `target/` (git-ignored). The worktree is removed on
# success unless KEEP_WORKTREE=1. Re-running is cheap: a tag whose staging
# directory already holds both artifacts is skipped, so an interrupted run
# resumes. Set FORCE=1 to rebuild everything.
#
# The build target directory is shared across all tags on purpose
# (CARGO_TARGET_DIR): the dependency graph barely moves between releases,
# so the second tag onward mostly relinks.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stage_dir="${1:-$repo_root/target/release-history/stage}"
worktree_dir="${2:-$repo_root/target/release-history/worktree}"
cargo_target_dir="${CARGO_TARGET_DIR:-$repo_root/target/release-history/build}"

mkdir -p "$stage_dir" "$(dirname "$worktree_dir")" "$cargo_target_dir"
stage_dir="$(cd "$stage_dir" && pwd)"
cargo_target_dir="$(cd "$cargo_target_dir" && pwd)"

tags="$(git -C "$repo_root" tag -l | sort -V)"
if [ -z "$tags" ]; then
  echo "no tags in $repo_root -- nothing to do" >&2
  exit 1
fi

if [ ! -d "$worktree_dir" ]; then
  echo "== creating isolated worktree at $worktree_dir"
  git -C "$repo_root" worktree add --detach "$worktree_dir" HEAD >/dev/null
fi

for tag in $tags; do
  out="$stage_dir/$tag"
  if [ "${FORCE:-0}" != "1" ] && [ -f "$out/engine.wasm" ] && [ -f "$out/meta.json" ]; then
    echo "== $tag: already staged, skipping"
    continue
  fi
  echo "== $tag"
  mkdir -p "$out"

  # A previous tag's `compliance-runner` run leaves modified files under
  # compliance/reports/; `--force` plus a clean is what makes the next
  # checkout deterministic rather than a merge attempt.
  git -C "$worktree_dir" checkout --detach --force "$tag" >/dev/null 2>&1
  git -C "$worktree_dir" reset --hard "$tag" >/dev/null
  git -C "$worktree_dir" clean -fdq -e /target
  # The vendored ODRL-Test-Suite is a submodule, and its pin is part of
  # the tag: re-syncing per tag is what makes the pass rate historical.
  git -C "$worktree_dir" submodule update --init --recursive --force >/dev/null

  commit="$(git -C "$repo_root" rev-list -n1 "$tag")"
  date="$(git -C "$repo_root" log -1 --format=%cI "$commit")"
  subject="$(git -C "$repo_root" log -1 --format=%s "$commit")"

  (
    cd "$worktree_dir"
    export CARGO_TARGET_DIR="$cargo_target_dir"
    cargo build -p engine --target wasm32-unknown-unknown --release >/dev/null
    cp "$cargo_target_dir/wasm32-unknown-unknown/release/engine.wasm" "$out/engine.wasm"

    # `|| true`: a tag whose compliance run genuinely fails (or whose
    # runner does not build) must still contribute its engine.wasm and its
    # real, failed status to the dashboard rather than aborting the sweep.
    if cargo run -p compliance-runner --release >"$out/compliance-stdout.txt" 2>"$out/compliance-stderr.txt"; then
      cp compliance/reports/latest.json "$out/compliance.json"
    else
      echo "   !! compliance-runner failed at $tag (see $out/compliance-stderr.txt)"
      rm -f "$out/compliance.json"
    fi
  )

  python3 - "$out/meta.json" "$tag" "$commit" "$date" "$subject" <<'PY'
import json, sys
path, tag, commit, date, subject = sys.argv[1:6]
json.dump({"tag": tag, "commit": commit, "date": date, "subject": subject},
          open(path, "w"), indent=2, sort_keys=True)
PY
done

if [ "${KEEP_WORKTREE:-0}" != "1" ]; then
  echo "== removing worktree $worktree_dir"
  git -C "$repo_root" worktree remove --force "$worktree_dir"
fi

echo
echo "staged $(echo "$tags" | wc -w) tags under $stage_dir"
echo "now run: cargo run -p release-history --release -- $stage_dir"
