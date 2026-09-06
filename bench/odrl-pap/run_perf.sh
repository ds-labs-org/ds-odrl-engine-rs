#!/usr/bin/env bash
# Stand up the real ODRL-PAP stack (PostgreSQL + OPA + Quarkus), verify it
# reproduces the conformance result, then run the performance and load benches
# against it -- and tear the whole stack back down.
#
# This is the script that produced results/perf.json, results/load.json and the
# results/*.txt environment records. It is written to be run from a bare clone:
# it clones odrl-pap itself, at the pinned commit, into $WORK.
#
# Everything heavy it starts (two containers, one JVM) is torn down by the EXIT
# trap, including on failure, because this bench runs in a shared sandbox and
# the next measurement needs a quiet machine.
#
# Usage: ./run_perf.sh [work-dir]        (default: ./.perf-work)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${1:-$HERE/.perf-work}"
RESULTS="$HERE/results"
RUN="$WORK/run"
PIN=59e45474c910b97f537b8f39c68e2e17ec4243ef      # = tag 1.7.0
PAP_PORT=8091
OPA_PORT=8181
MGMT_PORT=9000

mkdir -p "$WORK" "$RUN" "$RESULTS"

cleanup() {
  echo "=== teardown ==="
  [ -f "$RUN/pap.pid" ] && kill "$(cat "$RUN/pap.pid")" 2>/dev/null || true
  sleep 2
  [ -f "$RUN/pap.pid" ] && kill -9 "$(cat "$RUN/pap.pid")" 2>/dev/null || true
  docker rm -f pap-postgres pap-opa >/dev/null 2>&1 || true
  rm -f "$RUN/pap.pid"
  echo "teardown done: $(docker ps --filter name=pap- --format '{{.Names}}' | wc -l) pap containers left"
}
trap cleanup EXIT

# --------------------------------------------------------------------------
# 0. Environment of record -- what this ran on, before anything heavy starts
# --------------------------------------------------------------------------
{
  echo "date:            $(date -Is)"
  echo "uname:           $(uname -srm)"
  echo "nproc:           $(nproc)"
  echo "loadavg (pre):   $(cut -d' ' -f1-3 /proc/loadavg)"
  echo
  echo "--- free -h ---";        free -h
  echo "--- java -version ---";  java -version 2>&1
  echo "--- python3 ---";        python3 --version
  echo "--- psutil ---";         python3 -c 'import psutil;print(psutil.__version__)'
  echo "--- docker ---";         docker --version
  echo "--- images ---";         docker image inspect -f '{{index .RepoTags 0}} {{.Id}}' postgres:16-alpine openpolicyagent/opa:1.2.0 2>/dev/null
  echo "--- other load on the box (top 8 by cpu) ---"
  ps -eo pcpu,rss,comm --sort=-pcpu | head -9
} > "$RESULTS/environment.txt"
cat "$RESULTS/environment.txt"

# --------------------------------------------------------------------------
# 1. Clone at the pinned commit, cold isolated build
# --------------------------------------------------------------------------
SRC="$WORK/odrl-pap"
if [ ! -d "$SRC/.git" ]; then
  /usr/bin/time -f "clone wall=%e s" -o "$RESULTS/clone.time.txt" \
    git clone https://github.com/SEAMWARE/odrl-pap.git "$SRC"
fi
git -C "$SRC" checkout -q "$PIN"
git -C "$SRC" rev-parse HEAD > "$RESULTS/pinned-commit.txt"
git -C "$SRC" describe --tags >> "$RESULTS/pinned-commit.txt"

# A private local repository, so the build time recorded is a genuinely cold
# dependency resolution and `du -sh` on it is this engine's real dependency
# footprint, not a share of whatever else is in ~/.m2.
M2="$WORK/m2"
mkdir -p "$M2"
if [ ! -f "$SRC/target/quarkus-app/quarkus-run.jar" ]; then
  ( cd "$SRC" && /usr/bin/time -v -o "$RESULTS/build.time.txt" \
      ./mvnw -B -q -DskipTests -Dmaven.repo.local="$M2" package \
      > "$RUN/build.log" 2>&1 )
fi

{
  echo "--- maven local repository (dependency tree) ---"; du -sh "$M2"
  echo "--- build output (target/) ---";                   du -sh "$SRC/target"
  echo "--- runnable artifact (target/quarkus-app/) ---";  du -sh "$SRC/target/quarkus-app"
  echo "--- source checkout, excluding target/ ---"
  du -sh --exclude=target "$SRC"
  echo "--- container images ---"
  docker image inspect -f '{{index .RepoTags 0}} {{.Size}} bytes' \
    postgres:16-alpine openpolicyagent/opa:1.2.0
} > "$RESULTS/footprint.txt"

# --------------------------------------------------------------------------
# 2. Stack: PostgreSQL, OPA, and the Quarkus service on the harness's own port
# --------------------------------------------------------------------------
# OPA polls its rego bundles from the PAP, so its bundle-server URL has to be
# the port the PAP actually listens on (8091, the port run_pap.py already
# targets) rather than the 8081 the upstream README's sample config assumes.
cat > "$RUN/opa.yaml" <<EOF
services:
  - name: bundle-server
    url: http://localhost:$PAP_PORT/bundles/service/v1
bundles:
  policies:
      service: bundle-server
      resource: policies.tar.gz
      polling: {min_delay_seconds: 2, max_delay_seconds: 4}
  methods:
      service: bundle-server
      resource: methods.tar.gz
      polling: {min_delay_seconds: 1, max_delay_seconds: 3}
  data:
      service: bundle-server
      resource: data.tar.gz
      polling: {min_delay_seconds: 1, max_delay_seconds: 15}
default_decision: /policy/main/allow
EOF

docker rm -f pap-postgres pap-opa >/dev/null 2>&1 || true
docker run -d --name pap-postgres --network host \
  -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=pap \
  postgres:16-alpine >/dev/null
docker run -d --name pap-opa --network host \
  -v "$RUN/opa.yaml:/opa.yaml" openpolicyagent/opa:1.2.0 run --server -c /opa.yaml >/dev/null

for i in $(seq 60); do
  curl -sf "http://localhost:$OPA_PORT/health" >/dev/null && break; sleep 1
done

# `paths.mapping` is ODRL-PAP's own documented extension point. pap-mapping.json
# is the 5-term overlay translate_pap.py's header describes; without it the run
# is the "stock" variant (odrl:read/use resolve to HTTP verbs and odrl:dateTime
# to OPA's own wall clock), which scores far lower.
nohup java -Dquarkus.http.port=$PAP_PORT \
           -Dpaths.mapping="$HERE/pap-mapping.json" \
           -jar "$SRC/target/quarkus-app/quarkus-run.jar" > "$RUN/pap.log" 2>&1 &
echo $! > "$RUN/pap.pid"

# quarkus.management.enabled=true in the engine's own application.properties
# moves /q/health onto the management port (9000), not the REST port.
for i in $(seq 90); do
  curl -sf "http://localhost:$MGMT_PORT/q/health" >/dev/null && break; sleep 1
done
# OPA has to have pulled and activated the methods/policies/data bundles before
# any /validate can resolve an odrl_* rego method.
for i in $(seq 60); do
  [ "$(docker logs pap-opa 2>&1 | grep -c 'Bundle loaded and activated')" -ge 3 ] && break
  sleep 1
done
docker logs pap-opa 2>&1 | grep -c 'Bundle loaded and activated' \
  | sed 's/^/opa bundle activations: /'

# --------------------------------------------------------------------------
# 3. Conformance re-verification against THIS stack, before any timing
# --------------------------------------------------------------------------
# The point of the perf pass is that its latency numbers describe the same work
# the conformance numbers describe. That only holds if this freshly built stack
# still scores what results/results.json says, so it is checked, not assumed.
( cd "$HERE" && python3 run_pap.py cases.json "$RESULTS/verify-conformance.json" ) \
  | tee "$RESULTS/verify-conformance.txt"

# --------------------------------------------------------------------------
# 4. Measurements
# --------------------------------------------------------------------------
cd "$HERE"
/usr/bin/time -v -o "$RESULTS/perf.time.txt" \
  python3 perf_bench.py "$RESULTS/perf.json" \
    --repeats 5 --warmup 20 --opa-probe 200 --pid-file "$RUN/pap.pid" \
  | tee "$RESULTS/perf.txt"

# Ramp A -- the full evaluable mix. Three of its 31 cases (the `big-policy`
# fixtures) cost ~7-9s each, so this ramp is dominated by them and dies early.
/usr/bin/time -v -o "$RESULTS/load.time.txt" \
  python3 load_bench.py "$RESULTS/load.json" \
    --levels 1,2,4,8,16,32,64,96 --requests 310 --repeats 3 \
    --pid-file "$RUN/pap.pid" \
  | tee "$RESULTS/load.txt"

# Ramp B -- the same ramp over the other 28 cases. Ramp A answers "what does
# this service do under load with the corpus as given"; ramp B answers "what
# does the ordinary path do under load", which ramp A cannot reach because it
# hits the ceiling at concurrency 4 on the outliers alone.
/usr/bin/time -v -o "$RESULTS/load-fastband.time.txt" \
  python3 load_bench.py "$RESULTS/load-fastband.json" \
    --levels 1,2,4,8,16,32,64,96 --requests 560 --repeats 3 \
    --exclude big-policy --pid-file "$RUN/pap.pid" \
  | tee "$RESULTS/load-fastband.txt"

{
  echo "loadavg (post):  $(cut -d' ' -f1-3 /proc/loadavg)"
  echo "--- pap log: errors/warnings during the measured run ---"
  grep -cE ' (ERROR|WARN) ' "$RUN/pap.log" | sed 's/^/count: /'
  grep -E ' ERROR ' "$RUN/pap.log" | tail -20 || true
} >> "$RESULTS/environment.txt"

echo "=== done; results in $RESULTS ==="
