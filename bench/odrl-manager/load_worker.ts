/**
 * One load-generator process for `load_bench.ts`.
 *
 * Concurrency for odrl-manager is OS processes, not async tasks -- see the
 * `asyncProbe` in `perf_bench.ts` and the README's load section: interleaving
 * evaluations inside one Node process is both no faster (the work between
 * awaits is synchronous CPU-bound JS on one event loop) and *wrong* (the
 * engine's `EntityRegistry` state is `private static`, i.e. process-global,
 * and gets clobbered mid-evaluation).
 *
 * So each worker is a whole fresh Node process that does its own module
 * import, its own corpus read/translate, and its own warmup before the host is
 * allowed to time it. Nothing is shared with any other worker.
 *
 * Protocol, over Node's `child_process.fork` IPC channel:
 *   worker -> host  {t:'ready', pid, prepare_ms, warmup_ms, rss_kb}
 *   host   -> worker {t:'go', level, repeat, duration_ms}
 *   worker -> host  {t:'done', level, repeat, count, errors, mismatches,
 *                    mismatch_slugs, wall_ms, stats, subsample, rss_kb, cpu_s}
 *   host   -> worker {t:'stop'}   (worker exits 0)
 *
 * The timed unit is the ENGINE-ONLY path (`evaluateOnce`), the same one
 * `perf_bench.ts` reports as the primary latency, so a c=1 load figure is
 * directly comparable to the single-process median.
 */
import {
  Mode,
  Prepared,
  prepareCorpus,
  evaluateOnce,
  stats,
  vmRssKb,
  cpuSeconds,
  nowMs,
  r3,
} from './perf_corpus';

const MODE = (process.env.BENCH_MODE || 'native') as Mode;
const WARMUP_PASSES = parseInt(process.env.WARMUP_PASSES || '2', 10);
/** How many (slug, ms) pairs to ship back per step. A worker can do >30k
 *  evaluations in a 10 s step; sending them all would be tens of MB per step
 *  over IPC. Each worker computes its OWN exact percentiles over every sample
 *  it took, and additionally ships a UNIFORM RANDOM subsample so the host can
 *  pool a distribution across workers. Nothing is silently dropped: `count` is
 *  the true evaluation count and the per-worker stats are exact.
 *
 *  It has to be random, not every-k-th. The corpus is cycled round-robin with
 *  period 68, so a systematic stride k aliases: it only ever lands on case
 *  indices congruent mod gcd(k, 68). A first version of this file used
 *  stride = ceil(n/2000) ~= 17, gcd(17,68) = 17, i.e. it sampled 4 of the 68
 *  fixtures and never once sampled `testcase-062/063/064-big-policy` -- which
 *  are precisely the tail of this corpus's latency distribution. The pooled
 *  p95/p99 that came out were wrong by two orders of magnitude and disagreed
 *  with perf_bench.ts's single-process p99 on the identical corpus. Keep the
 *  sampling random. */
const SUBSAMPLE = parseInt(process.env.SUBSAMPLE || '4000', 10);

const send = (m: any) => process.send && process.send(m);

let prepared: Prepared[] = [];

async function warmup(): Promise<number> {
  const t0 = nowMs();
  for (let w = 0; w < WARMUP_PASSES; w++) {
    for (const p of prepared) await evaluateOnce(p);
  }
  return r3(nowMs() - t0);
}

async function runStep(level: number, repeat: number, durationMs: number) {
  const latencies: number[] = [];
  const slugs: string[] = [];
  const mismatchSlugs = new Set<string>();
  let errors = 0;
  let mismatches = 0;
  const t0 = nowMs();
  const deadline = t0 + durationMs;
  let i = 0;
  while (nowMs() < deadline) {
    const p = prepared[i % prepared.length];
    i++;
    const t = nowMs();
    try {
      const d = await evaluateOnce(p);
      latencies.push(nowMs() - t);
      slugs.push(p.slug);
      if (d !== p.expected) {
        mismatches++;
        mismatchSlugs.add(p.slug);
      }
    } catch (e: any) {
      errors++;
      latencies.push(nowMs() - t);
      slugs.push(p.slug);
    }
  }
  const wall = r3(nowMs() - t0);

  // Uniform random subsample of (slug, ms) pairs, without replacement.
  const n = latencies.length;
  const take = Math.min(SUBSAMPLE, n);
  const idx = new Set<number>();
  while (idx.size < take) idx.add(Math.floor(Math.random() * n));
  const subsample = [...idx].map((k) => [slugs[k], r3(latencies[k])] as [string, number]);

  // Exact per-fixture stats over every sample this worker took.
  const bySlug = new Map<string, number[]>();
  for (let k = 0; k < n; k++) {
    if (!bySlug.has(slugs[k])) bySlug.set(slugs[k], []);
    bySlug.get(slugs[k])!.push(latencies[k]);
  }
  const per_slug = [...bySlug.entries()].map(([slug, v]) => ({ slug, ...stats(v) }));

  send({
    t: 'done',
    pid: process.pid,
    level,
    repeat,
    count: n,
    errors,
    mismatches,
    mismatch_slugs: [...mismatchSlugs],
    wall_ms: wall,
    stats: stats(latencies),
    per_slug,
    subsample,
    subsample_n: take,
    rss_kb: vmRssKb(),
    cpu_s: cpuSeconds(),
  });
}

(async () => {
  const tPrep = nowMs();
  prepared = prepareCorpus(MODE);
  const prepare_ms = r3(nowMs() - tPrep);
  let warmup_ms = -1;
  try {
    warmup_ms = await warmup();
  } catch (e: any) {
    send({ t: 'warmup-failed', pid: process.pid, error: e.message });
    process.exit(2);
  }
  send({ t: 'ready', pid: process.pid, prepare_ms, warmup_ms, rss_kb: vmRssKb(), cases: prepared.length });

  process.on('message', async (m: any) => {
    if (m.t === 'go') await runStep(m.level, m.repeat, m.duration_ms);
    else if (m.t === 'stop') process.exit(0);
  });
})();

// A worker must never outlive its host. If the IPC channel goes away (host
// killed, pipe broken) the worker exits rather than spinning forever.
process.on('disconnect', () => process.exit(0));
