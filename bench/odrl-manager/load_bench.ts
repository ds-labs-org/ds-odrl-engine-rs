/**
 * Concurrency ramp for odrl-manager.
 *
 * WHAT "CONCURRENCY" MEANS HERE, CONCRETELY: one unit of concurrency is one
 * OS process -- a forked Node process running `load_worker.ts`, each with its
 * own module import, its own translated corpus and its own warmup. It is NOT
 * `Promise.all` over N evaluations in one process. That alternative was the
 * obvious one to try given odrl-manager is Promise-based throughout, and
 * `perf_bench.ts`'s `asyncProbe` measured it: it adds no throughput (the work
 * between awaits is synchronous CPU-bound JS on a single event loop) and it
 * returns WRONG ANSWERS, because `EntityRegistry`'s state is `private static`
 * -- one table per process, cleared before every `genPolicyFrom` -- so
 * interleaved evaluations overwrite each other's entity graph.
 *
 * A reader comparing this axis to another engine's must not assume
 * apples-to-apples: a process here carries a whole V8 heap plus ts-node's
 * TypeScript compiler, which is harness weight, not engine weight. The
 * per-worker RSS reported below is measured, and the engine's own share of it
 * is the much smaller in-process figure `perf_bench.ts` reports.
 *
 * Method: spawn the pool once per ramp repeat at the maximum level; at each
 * level only the first `c` workers are told to go, the rest sit idle in their
 * IPC wait (resident, zero CPU). Per step the host samples
 * /proc/<pid>/status VmRSS and /proc/<pid>/stat utime+stime for the ACTIVE
 * workers every SAMPLE_MS, so RSS and CPU are kernel-reported.
 *
 * Usage:
 *   ODRL_TEST_SUITE_DATA=<corpus> OUT=<file.json> \
 *     npx ts-node -r tsconfig-paths/register src/bench/load_bench.ts \
 *       [native|assisted] [--levels 1,2,4,...] [--step-ms 10000] [--ramps 3]
 */
import * as fs from 'fs';
import * as path from 'path';
import { fork, ChildProcess } from 'child_process';
import {
  Mode,
  stats,
  percentile,
  r3,
  vmRssKb,
  cpuSeconds,
  memAvailableKb,
  nowMs,
  LEVEL_INSTABILITY,
} from './perf_corpus';

const MODE = ((process.argv[2] && !process.argv[2].startsWith('--') ? process.argv[2] : 'native') as Mode);
const arg = (flag: string, dflt: string) => {
  const i = process.argv.indexOf(flag);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : dflt;
};
const LEVELS = arg('--levels', '1,2,4,8,16,22,32,44').split(',').map((s) => parseInt(s, 10));
const STEP_MS = parseInt(arg('--step-ms', '10000'), 10);
const RAMPS = parseInt(arg('--ramps', '3'), 10);
const SAMPLE_MS = parseInt(arg('--sample-ms', '250'), 10);
const SPAWN_STAGGER_MS = parseInt(arg('--stagger-ms', '150'), 10);
/** Refuse a step that would leave the box short of memory. */
const MIN_FREE_MB = parseInt(arg('--min-free-mb', '8192'), 10);
const OUT = process.env.OUT || `load-${MODE}.json`;

const MAX_LEVEL = Math.max(...LEVELS);
/** The three fixtures whose policies are an order of magnitude larger than the
 *  rest of the corpus. They are 3/68 of every round-robin cycle and they alone
 *  set this engine's p99 -- so every latency figure is reported twice, with and
 *  without them, rather than letting corpus composition masquerade as load. */
const HEAVY_SLUGS = ['062-big-policy', '063-big-policy', '064-big-policy'];
const sleep = (ms: number) => new Promise((res) => setTimeout(res, ms));

interface Worker {
  cp: ChildProcess;
  pid: number;
  ready: any;
  resolveDone?: (m: any) => void;
}

let pool: Worker[] = [];

function killPool() {
  for (const w of pool) {
    try {
      if (!w.cp.killed) w.cp.kill('SIGKILL');
    } catch {
      /* already gone */
    }
  }
  pool = [];
}
// Every exit path kills the pool. A load generator left running would poison
// whatever measurement runs next on this machine.
process.on('exit', killPool);
process.on('SIGINT', () => {
  killPool();
  process.exit(130);
});
process.on('SIGTERM', () => {
  killPool();
  process.exit(143);
});
process.on('uncaughtException', (e) => {
  console.error('uncaught, killing pool:', e);
  killPool();
  process.exit(1);
});

function spawnWorker(): Promise<Worker> {
  return new Promise((resolve, reject) => {
    const cp = fork(path.join(__dirname, 'load_worker.ts'), [], {
      execArgv: ['-r', 'ts-node/register', '-r', 'tsconfig-paths/register'],
      env: { ...process.env, BENCH_MODE: MODE },
      stdio: ['ignore', 'ignore', 'inherit', 'ipc'],
    });
    const w: Worker = { cp, pid: cp.pid!, ready: null };
    cp.on('message', (m: any) => {
      if (m.t === 'ready') {
        w.ready = m;
        resolve(w);
      } else if (m.t === 'warmup-failed') {
        reject(new Error(`worker ${m.pid} failed warmup: ${m.error}`));
      } else if (m.t === 'done' && w.resolveDone) {
        const r = w.resolveDone;
        w.resolveDone = undefined;
        r(m);
      }
    });
    cp.on('error', reject);
    cp.on('exit', (code) => {
      if (!w.ready) reject(new Error(`worker exited ${code} before ready`));
    });
  });
}

async function spawnPool(n: number): Promise<{ workers: Worker[]; spawn_ms: number }> {
  const t0 = nowMs();
  const pending: Promise<Worker>[] = [];
  for (let i = 0; i < n; i++) {
    pending.push(spawnWorker());
    await sleep(SPAWN_STAGGER_MS);
  }
  const workers = await Promise.all(pending);
  return { workers, spawn_ms: r3(nowMs() - t0) };
}

async function runLevel(workers: Worker[], level: number, repeat: number) {
  const active = workers.slice(0, level);
  const samples: { at_ms: number; rss_kb: number; cpu_s: number }[] = [];
  const cpu0 = active.map((w) => cpuSeconds(w.pid));
  const t0 = nowMs();

  const dones = active.map(
    (w) =>
      new Promise<any>((res) => {
        w.resolveDone = res;
        w.cp.send({ t: 'go', level, repeat, duration_ms: STEP_MS });
      }),
  );

  let sampling = true;
  const sampler = (async () => {
    while (sampling) {
      const rss = active.reduce((a, w) => a + Math.max(0, vmRssKb(w.pid)), 0);
      const cpu = active.reduce((a, w, i) => a + Math.max(0, cpuSeconds(w.pid) - cpu0[i]), 0);
      samples.push({ at_ms: r3(nowMs() - t0), rss_kb: rss, cpu_s: r3(cpu) });
      await sleep(SAMPLE_MS);
    }
  })();

  const results = await Promise.all(dones);
  sampling = false;
  await sampler;
  const wall_ms = r3(nowMs() - t0);

  const cpu_total_s = active.reduce((a, w, i) => a + Math.max(0, cpuSeconds(w.pid) - cpu0[i]), 0);
  const count = results.reduce((a, r) => a + r.count, 0);
  const errors = results.reduce((a, r) => a + r.errors, 0);
  const mismatches = results.reduce((a, r) => a + r.mismatches, 0);
  const mismatchSlugs = new Set<string>();
  for (const r of results) for (const s of r.mismatch_slugs) mismatchSlugs.add(s);

  // Pooled latency distribution from every worker's uniform random subsample
  // of (slug, ms) pairs. `light` repeats it with the three big-policy fixtures
  // removed -- a second view of the same data, not an exclusion, so corpus
  // composition stays distinguishable from contention.
  const pooled: number[] = [];
  const pooledLight: number[] = [];
  for (const r of results) {
    for (const [slug, ms] of r.subsample as [string, number][]) {
      pooled.push(ms);
      if (!HEAVY_SLUGS.some((h) => slug.includes(h))) pooledLight.push(ms);
    }
  }
  const pooledStats = stats(pooled);
  // Cross-check: medians of each worker's EXACT stats over all its samples.
  // If this disagrees with the pooled subsample the sampling is suspect.
  const exactMedians = results.map((r) => r.stats.median).sort((a, b) => a - b);
  const exactP99s = results.map((r) => r.stats.p99).sort((a, b) => a - b);

  // Steady-state RSS: median of samples taken after the first second.
  const settled = samples.filter((s) => s.at_ms > 1000).map((s) => s.rss_kb).sort((a, b) => a - b);
  const rss_steady_kb = settled.length ? Math.round(percentile(settled, 0.5)) : 0;
  const rss_peak_kb = Math.max(...samples.map((s) => s.rss_kb), 0);

  return {
    level,
    repeat,
    wall_ms,
    evaluations: count,
    errors,
    error_rate: count ? r3(errors / count) : 0,
    mismatches,
    mismatch_slugs: [...mismatchSlugs].sort(),
    throughput_wall: r3(count / (wall_ms / 1000)),
    throughput_per_worker: r3(count / (wall_ms / 1000) / level),
    latency_ms: pooledStats,
    latency_light_ms: stats(pooledLight),
    latency_pooled_n: pooled.length,
    latency_exact_worker_median_of_medians: r3(percentile(exactMedians, 0.5)),
    latency_exact_worker_median_of_p99: r3(percentile(exactP99s, 0.5)),
    per_worker_latency_ms: results.map((r) => ({ pid: r.pid, median: r.stats.median, p99: r.stats.p99, n: r.count })),
    heavy_fixture_latency_ms: HEAVY_SLUGS.map((h) => {
      const rows = results.flatMap((r: any) => r.per_slug.filter((s: any) => s.slug.includes(h)));
      const meds = rows.map((s: any) => s.median).sort((a: number, b: number) => a - b);
      return { fixture: h, worker_median_of_medians: r3(percentile(meds, 0.5)), workers: rows.length };
    }),
    cpu_seconds: r3(cpu_total_s),
    busy_cores: r3(cpu_total_s / (wall_ms / 1000)),
    rss_peak_kb,
    rss_steady_kb,
    rss_per_worker_steady_kb: Math.round(rss_steady_kb / level),
    mem_available_kb: memAvailableKb(),
    rss_samples: samples.length,
  };
}

(async () => {
  const started = new Date().toISOString();
  const allSteps: any[] = [];
  const poolInfo: any[] = [];

  for (let ramp = 0; ramp < RAMPS; ramp++) {
    const avail = memAvailableKb();
    console.log(`\n=== ramp repeat ${ramp}: spawning ${MAX_LEVEL} workers (MemAvailable ${Math.round(avail / 1024)} MB) ===`);
    const { workers, spawn_ms } = await spawnPool(MAX_LEVEL);
    pool = workers;
    poolInfo.push({
      ramp,
      spawn_ms,
      workers: MAX_LEVEL,
      mem_available_kb_before: avail,
      worker_prepare_ms: workers.map((w) => w.ready.prepare_ms),
      worker_warmup_ms: workers.map((w) => w.ready.warmup_ms),
      worker_rss_after_warmup_kb: workers.map((w) => w.ready.rss_kb),
      host_rss_kb: vmRssKb(),
    });
    console.log(`    pool ready in ${spawn_ms} ms; worker RSS after warmup ~${workers[0].ready.rss_kb} kB`);

    for (const level of LEVELS) {
      const freeMb = memAvailableKb() / 1024;
      if (freeMb < MIN_FREE_MB) {
        console.log(`  c=${level}: SKIPPED, MemAvailable ${Math.round(freeMb)} MB < MIN_FREE_MB ${MIN_FREE_MB}`);
        allSteps.push({ level, repeat: ramp, skipped: `MemAvailable ${Math.round(freeMb)} MB below guard` });
        continue;
      }
      const s = await runLevel(workers, level, ramp);
      allSteps.push(s);
      console.log(
        `  c=${String(level).padStart(2)}  tput ${String(s.throughput_wall).padStart(9)}/s  med ${s.latency_ms.median} ms  p95 ${s.latency_ms.p95}  p99 ${s.latency_ms.p99}  [light med ${s.latency_light_ms.median} p99 ${s.latency_light_ms.p99}]  [exact med ${s.latency_exact_worker_median_of_medians} p99 ${s.latency_exact_worker_median_of_p99}]  cores ${s.busy_cores}  RSS ${(s.rss_steady_kb / 1048576).toFixed(2)} GiB  err ${s.errors}`,
      );
    }

    for (const w of workers) {
      try {
        w.cp.send({ t: 'stop' });
      } catch {
        /* gone */
      }
    }
    await sleep(500);
    killPool();
    await sleep(500);
  }

  // ---- aggregate across ramp repeats, and apply gate rule 3 ----
  const byLevel = new Map<number, any[]>();
  for (const s of allSteps) {
    if (s.skipped) continue;
    if (!byLevel.has(s.level)) byLevel.set(s.level, []);
    byLevel.get(s.level)!.push(s);
  }
  const summary = [...byLevel.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([level, steps]) => {
      const med = (f: (s: any) => number) => r3(percentile(steps.map(f).sort((a, b) => a - b), 0.5));
      const tputs = steps.map((s) => s.throughput_wall).sort((a, b) => a - b);
      const tputMed = percentile(tputs, 0.5);
      const rel = tputMed > 0 ? (tputs[tputs.length - 1] - tputs[0]) / tputMed : 0;
      return {
        level,
        ramps: steps.length,
        throughput_wall: med((s) => s.throughput_wall),
        throughput_per_worker: med((s) => s.throughput_per_worker),
        latency_median_ms: med((s) => s.latency_ms.median),
        latency_mean_ms: med((s) => s.latency_ms.mean),
        latency_p95_ms: med((s) => s.latency_ms.p95),
        latency_p99_ms: med((s) => s.latency_ms.p99),
        latency_max_ms: med((s) => s.latency_ms.max),
        latency_light_median_ms: med((s) => s.latency_light_ms.median),
        latency_light_p95_ms: med((s) => s.latency_light_ms.p95),
        latency_light_p99_ms: med((s) => s.latency_light_ms.p99),
        latency_exact_worker_median_of_medians: med((s) => s.latency_exact_worker_median_of_medians),
        latency_exact_worker_median_of_p99: med((s) => s.latency_exact_worker_median_of_p99),
        busy_cores: med((s) => s.busy_cores),
        cpu_seconds: med((s) => s.cpu_seconds),
        rss_peak_kb: Math.round(med((s) => s.rss_peak_kb)),
        rss_steady_kb: Math.round(med((s) => s.rss_steady_kb)),
        rss_per_worker_steady_kb: Math.round(med((s) => s.rss_per_worker_steady_kb)),
        evaluations: steps.reduce((a, s) => a + s.evaluations, 0),
        errors: steps.reduce((a, s) => a + s.errors, 0),
        error_rate: r3(steps.reduce((a, s) => a + s.errors, 0) / Math.max(1, steps.reduce((a, s) => a + s.evaluations, 0))),
        mismatch_slugs: [...new Set(steps.flatMap((s) => s.mismatch_slugs))].sort(),
        throughput_rel_range: r3(rel),
        unstable_across_repeats: rel > LEVEL_INSTABILITY,
      };
    });

  const out = {
    engine: 'odrl-manager (Prometheus-X-association/odrl-manager, develop)',
    commit: '8842b6b9ff9fa580f9400f426a5f361f526dbd9b',
    mode: MODE,
    started,
    node: process.version,
    concurrency_unit: 'one OS process (forked Node), not an async task -- see header',
    levels: LEVELS,
    step_ms: STEP_MS,
    ramps: RAMPS,
    sample_ms: SAMPLE_MS,
    spawn_stagger_ms: SPAWN_STAGGER_MS,
    min_free_mb: MIN_FREE_MB,
    rule3_threshold: LEVEL_INSTABILITY,
    nproc: require('os').cpus().length,
    pools: poolInfo,
    summary,
    steps: allSteps,
    finished: new Date().toISOString(),
  };
  fs.writeFileSync(OUT, JSON.stringify(out, null, 1));

  console.log('\n===== ramp summary (medians across ramp repeats) =====');
  console.log('  c |    tput/s | per-wkr |   med |   p95 |    p99 | L-med | L-p95 | L-p99 | cores | RSS GiB | err | rel.range');
  for (const s of summary) {
    const c = (x: number, w: number) => String(x).padStart(w);
    console.log(
      `${c(s.level, 3)} | ${c(s.throughput_wall, 9)} | ${c(s.throughput_per_worker, 7)} | ${c(s.latency_median_ms, 5)} | ${c(s.latency_p95_ms, 5)} | ${c(s.latency_p99_ms, 6)} | ${c(s.latency_light_median_ms, 5)} | ${c(s.latency_light_p95_ms, 5)} | ${c(s.latency_light_p99_ms, 5)} | ${c(s.busy_cores, 5)} | ${(s.rss_steady_kb / 1048576).toFixed(2).padStart(7)} | ${c(s.errors, 3)} | ${s.throughput_rel_range}${s.unstable_across_repeats ? ' UNSTABLE' : ''}`,
    );
  }
  console.log(`wrote ${OUT}`);
  killPool();
  process.exit(0);
})();
