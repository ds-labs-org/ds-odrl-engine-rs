/**
 * Single-process performance / resource instrumentation for odrl-manager
 * (`develop` @ 8842b6b), on the same 68-fixture corpus and through the same
 * call sequence `run.ts` scores 61/68 native with.
 *
 * Everything it measures:
 *   0. startup      -- ts-node + module import cost, before any evaluation.
 *   1. warmup       -- N discarded full corpus passes (V8 JIT / lazy init /
 *                      page cache), each one's wall recorded so the warm-up
 *                      curve is visible instead of asserted. Doubles as the
 *                      smoke test: any warmup error aborts before timing.
 *   2. latency      -- per-case ms over R repeats, on BOTH paths defined in
 *                      perf_corpus.ts (engine-only, and end-to-end incl. the
 *                      n3 RDF parse). mean/median/p95/p99/min/max for each.
 *   4. resources    -- in-process RSS series from /proc/self/status and CPU
 *                      from process.cpuUsage(). The authoritative peak-RSS and
 *                      CPU-seconds figures come from /usr/bin/time -v around
 *                      this whole process (run_perf.sh); these are the shape.
 *   6. gates        -- Tukey fence (rule 1) and per-case cross-repeat
 *                      instability (rule 2), both from perf_corpus.ts's stated
 *                      constants. Flagged, never dropped.
 *   +  asyncProbe   -- does in-process async concurrency buy anything, and is
 *                      it even correct? Measured, not assumed (see below).
 *
 * Usage:
 *   ODRL_TEST_SUITE_DATA=<corpus> OUT=<file.json> \
 *     npx ts-node -r tsconfig-paths/register src/bench/perf_bench.ts \
 *       [native|assisted] [--repeats N] [--warmup N]
 */
import * as fs from 'fs';
import {
  Mode,
  Decision,
  Prepared,
  prepareCorpus,
  evaluateOnce,
  pipelineOnce,
  listCases,
  stats,
  tukeyFence,
  percentile,
  r3,
  vmRssKb,
  memAvailableKb,
  nowMs,
  TUKEY_K,
  CASE_INSTABILITY,
} from './perf_corpus';

// performance.now()'s time origin IS process start in Node (unlike
// hrtime.bigint(), whose origin is arbitrary), so this is real
// elapsed-since-exec at the moment the last import above finished resolving:
// ts-node's transpile of this file and the whole odrl-manager + n3 import
// graph, before a single evaluation has happened.
const T_STARTUP_MS = performance.now();

const MODE = ((process.argv[2] && !process.argv[2].startsWith('--') ? process.argv[2] : 'native') as Mode);
const argNum = (flag: string, dflt: number) => {
  const i = process.argv.indexOf(flag);
  return i >= 0 && process.argv[i + 1] ? parseInt(process.argv[i + 1], 10) : dflt;
};
const REPEATS = argNum('--repeats', 5);
const WARMUP_PASSES = argNum('--warmup', 5);
const OUT = process.env.OUT || `perf-${MODE}.json`;
/** Drop the individual `engine_samples`/`pipeline_samples` records from the
 *  written JSON. Everything derived from them -- the aggregate stats, the
 *  per-case table, both gates, the per-repeat rows -- is still computed from
 *  the full set and still written; only the raw per-evaluation list is
 *  omitted. Used for the long soak run, where the point is the RSS trajectory
 *  and 54,400 sample records would be 8.6 MB of JSON to make it. The 20-repeat
 *  runs keep their raw samples. */
const NO_RAW = process.argv.includes('--no-raw-samples');

interface Sample {
  repeat: number;
  slug: string;
  ms: number;
  decision: Decision;
  expected: Decision;
  error: string | null;
}

const cpuMs = () => {
  const c = process.cpuUsage();
  return { user: r3(c.user / 1000), system: r3(c.system / 1000) };
};

(async () => {
  const startup_ms = r3(T_STARTUP_MS);
  const rss_after_import_kb = vmRssKb();

  // ---- corpus preparation (adapter work, timed separately from both paths) ----
  const tPrep = nowMs();
  const prepared: Prepared[] = prepareCorpus(MODE);
  const prepare_corpus_ms = r3(nowMs() - tPrep);
  const cases = listCases();
  const rss_after_corpus_kb = vmRssKb();

  // ---- 1. warmup / smoke test -------------------------------------------
  const warmup_pass_ms: number[] = [];
  const warmup_first_call_ms: number[] = [];
  for (let w = 0; w < WARMUP_PASSES; w++) {
    const t0 = nowMs();
    let first = 0;
    for (let i = 0; i < prepared.length; i++) {
      const t = nowMs();
      try {
        await evaluateOnce(prepared[i]);
      } catch (e: any) {
        console.error(`WARMUP FAILED on ${prepared[i].slug}: ${e.message}`);
        console.error('refusing to print timings for an engine that cannot complete warmup');
        process.exit(2);
      }
      if (i === 0) first = nowMs() - t;
    }
    warmup_pass_ms.push(r3(nowMs() - t0));
    warmup_first_call_ms.push(r3(first));
  }
  const rss_after_warmup_kb = vmRssKb();
  const cpu_after_warmup = cpuMs();

  // ---- 2. engine-only per-case latency ----------------------------------
  const engineSamples: Sample[] = [];
  const enginePerRepeat: any[] = [];
  const rssSeries: { at_ms: number; rss_kb: number; phase: string }[] = [];
  const tEngine0 = nowMs();
  for (let rep = 0; rep < REPEATS; rep++) {
    const t0 = nowMs();
    let mismatches = 0;
    let errors = 0;
    for (const p of prepared) {
      const t = nowMs();
      let decision: Decision = 'Deny';
      let error: string | null = null;
      try {
        decision = await evaluateOnce(p);
      } catch (e: any) {
        error = e.message;
        errors++;
      }
      const ms = r3(nowMs() - t);
      if (!error && decision !== p.expected) mismatches++;
      engineSamples.push({ repeat: rep, slug: p.slug, ms, decision, expected: p.expected, error });
    }
    rssSeries.push({ at_ms: r3(nowMs() - tEngine0), rss_kb: vmRssKb(), phase: `engine-repeat-${rep}` });
    enginePerRepeat.push({
      repeat: rep,
      wall_ms: r3(nowMs() - t0),
      mismatches,
      errors,
      rss_end_kb: vmRssKb(),
    });
  }
  const cpu_after_engine = cpuMs();

  // ---- 2b. end-to-end per-case latency (RDF parse + translate + engine) --
  const pipelineSamples: Sample[] = [];
  const pipelinePerRepeat: any[] = [];
  for (let rep = 0; rep < REPEATS; rep++) {
    const t0 = nowMs();
    let mismatches = 0;
    let errors = 0;
    for (const c of cases) {
      const t = nowMs();
      let decision: Decision = 'Deny';
      let error: string | null = null;
      try {
        decision = await pipelineOnce(c, MODE);
      } catch (e: any) {
        error = e.message;
        errors++;
      }
      const ms = r3(nowMs() - t);
      if (!error && decision !== c.expected) mismatches++;
      pipelineSamples.push({ repeat: rep, slug: c.slug, ms, decision, expected: c.expected, error });
    }
    rssSeries.push({ at_ms: r3(nowMs() - tEngine0), rss_kb: vmRssKb(), phase: `pipeline-repeat-${rep}` });
    pipelinePerRepeat.push({
      repeat: rep,
      wall_ms: r3(nowMs() - t0),
      mismatches,
      errors,
      rss_end_kb: vmRssKb(),
    });
  }
  const cpu_after_pipeline = cpuMs();
  const rss_peak_in_process_kb = Math.max(...rssSeries.map((s) => s.rss_kb), rss_after_warmup_kb);

  // ---- asyncProbe: is in-process concurrency faster, and is it correct? --
  //
  // odrl-manager is Promise-based end to end, so `Promise.all` over N
  // evaluations LOOKS like concurrency. Two things are actually true and both
  // are measured here rather than argued:
  //   (a) the work between awaits is synchronous CPU-bound JS on one event
  //       loop, so interleaving cannot add throughput;
  //   (b) EntityRegistry's state is `private static` -- process-global -- and
  //       run.ts clears it before every genPolicyFrom. Interleaved evaluations
  //       therefore share one table. The probe re-checks every decision against
  //       the sequential answer for the same fixture and counts divergences.
  //
  // The probe is fenced with markers on stderr because odrl-manager prints its
  // own `Error in "isActionPerformable"` lines from an internal catch when the
  // shared registry is torn out from under an in-flight evaluation. Everything
  // between the markers in the captured stderr belongs to the probe; anything
  // outside them would be a real fault in the sequential timed runs.
  const probeWidths = [2, 4, 8, 16, 68];
  const asyncProbe: any[] = [];
  const sequentialTruth = new Map<string, Decision>();
  for (const p of prepared) sequentialTruth.set(p.slug, await evaluateOnce(p));
  console.error('--- ASYNC PROBE BEGIN (engine stderr below is the probe, by design) ---');
  for (const width of probeWidths) {
    const batch = prepared.slice(0, width);
    const tSeq = nowMs();
    for (const p of batch) await evaluateOnce(p);
    const seq_ms = r3(nowMs() - tSeq);

    const tPar = nowMs();
    const results = await Promise.all(batch.map((p) => evaluateOnce(p).catch((e) => `ERR:${e.message}` as any)));
    const par_ms = r3(nowMs() - tPar);

    let corrupted = 0;
    let errored = 0;
    const corruptedSlugs: string[] = [];
    results.forEach((got, i) => {
      const slug = batch[i].slug;
      if (typeof got === 'string' && got.startsWith('ERR:')) {
        errored++;
        corruptedSlugs.push(`${slug}(error)`);
      } else if (got !== sequentialTruth.get(slug)) {
        corrupted++;
        corruptedSlugs.push(slug);
      }
    });
    asyncProbe.push({
      width,
      sequential_ms: seq_ms,
      promise_all_ms: par_ms,
      speedup: r3(seq_ms / par_ms),
      wrong_answers_vs_sequential: corrupted,
      errors: errored,
      diverging_slugs: corruptedSlugs.slice(0, 20),
    });
  }
  console.error('--- ASYNC PROBE END ---');

  // ---- 6. gates ----------------------------------------------------------
  const gateFor = (samples: Sample[]) => {
    const values = samples.filter((s) => !s.error).map((s) => s.ms);
    const fence = tukeyFence(values);
    const flagged = samples.filter((s) => !s.error && (s.ms < fence.lo || s.ms > fence.hi));
    const bySlug = new Map<string, number[]>();
    for (const s of samples) {
      if (s.error) continue;
      if (!bySlug.has(s.slug)) bySlug.set(s.slug, []);
      bySlug.get(s.slug)!.push(s.ms);
    }
    const unstable: any[] = [];
    for (const [slug, v] of bySlug) {
      const sorted = [...v].sort((a, b) => a - b);
      const med = percentile(sorted, 0.5);
      const rel = med > 0 ? (sorted[sorted.length - 1] - sorted[0]) / med : 0;
      if (rel > CASE_INSTABILITY) unstable.push({ slug, rel_range: r3(rel), values: v });
    }
    unstable.sort((a, b) => b.rel_range - a.rel_range);
    return {
      rule1_tukey_k: TUKEY_K,
      rule1_fence_ms: fence,
      rule1_flagged_count: flagged.length,
      rule1_flagged_total: values.length,
      rule1_flagged: flagged
        .sort((a, b) => b.ms - a.ms)
        .slice(0, 40)
        .map((s) => ({ repeat: s.repeat, slug: s.slug, ms: s.ms })),
      rule2_threshold: CASE_INSTABILITY,
      rule2_unstable_count: unstable.length,
      rule2_cases_total: bySlug.size,
      rule2_worst: unstable.slice(0, 15),
    };
  };

  const perCase = (samples: Sample[]) => {
    const bySlug = new Map<string, number[]>();
    for (const s of samples) {
      if (s.error) continue;
      if (!bySlug.has(s.slug)) bySlug.set(s.slug, []);
      bySlug.get(s.slug)!.push(s.ms);
    }
    return [...bySlug.entries()]
      .map(([slug, v]) => ({ slug, ...stats(v), values: v }))
      .sort((a, b) => b.median - a.median);
  };

  const out = {
    engine: 'odrl-manager (Prometheus-X-association/odrl-manager, develop)',
    commit: '8842b6b9ff9fa580f9400f426a5f361f526dbd9b',
    mode: MODE,
    started: new Date().toISOString(),
    node: process.version,
    v8: process.versions.v8,
    corpus_root: process.env.ODRL_TEST_SUITE_DATA,
    cases: prepared.length,
    repeats: REPEATS,
    warmup_passes: WARMUP_PASSES,
    warmup_evaluations: WARMUP_PASSES * prepared.length,
    startup_ms,
    prepare_corpus_ms,
    rss_after_import_kb,
    rss_after_corpus_kb,
    rss_after_warmup_kb,
    rss_peak_in_process_kb,
    mem_available_kb_at_start: memAvailableKb(),
    warmup_pass_ms,
    warmup_first_call_ms,
    cpu_ms: {
      after_warmup: cpu_after_warmup,
      after_engine_run: cpu_after_engine,
      after_pipeline_run: cpu_after_pipeline,
    },
    engine_only: {
      what: 'genPolicyFrom + setPolicy + isActionPerformable on a pre-translated policy',
      per_repeat: enginePerRepeat,
      latency_ms: stats(engineSamples.filter((s) => !s.error).map((s) => s.ms)),
      per_case: perCase(engineSamples),
      gates: gateFor(engineSamples),
    },
    end_to_end: {
      what: "run.ts's whole per-case body: n3 parse of 3 fixture files + translate + engine call",
      per_repeat: pipelinePerRepeat,
      latency_ms: stats(pipelineSamples.filter((s) => !s.error).map((s) => s.ms)),
      per_case: perCase(pipelineSamples),
      gates: gateFor(pipelineSamples),
    },
    async_probe: asyncProbe,
    rss_series: rssSeries,
    raw_samples_omitted: NO_RAW,
    engine_samples: NO_RAW ? undefined : engineSamples,
    pipeline_samples: NO_RAW ? undefined : pipelineSamples,
    finished: new Date().toISOString(),
  };

  fs.writeFileSync(OUT, JSON.stringify(out, null, 1));

  const e = out.engine_only.latency_ms;
  const p = out.end_to_end.latency_ms;
  console.log(`\n===== odrl-manager perf (${MODE}) =====`);
  console.log(`node ${process.version}  cases ${prepared.length}  repeats ${REPEATS}  warmup ${WARMUP_PASSES} passes`);
  console.log(`startup(import) ${startup_ms} ms   corpus prepare ${prepare_corpus_ms} ms`);
  console.log(`warmup pass walls (ms): ${warmup_pass_ms.join(', ')}`);
  console.log(
    `engine-only  mean ${e.mean}  median ${e.median}  p95 ${e.p95}  p99 ${e.p99}  min ${e.min}  max ${e.max}  (n=${e.n})`,
  );
  console.log(
    `end-to-end   mean ${p.mean}  median ${p.median}  p95 ${p.p95}  p99 ${p.p99}  min ${p.min}  max ${p.max}  (n=${p.n})`,
  );
  console.log(
    `RSS import ${rss_after_import_kb} -> warmup ${rss_after_warmup_kb} -> peak(in-process) ${rss_peak_in_process_kb} kB`,
  );
  console.log('async probe (in-process Promise.all vs sequential):');
  for (const a of asyncProbe) {
    console.log(
      `  width ${String(a.width).padStart(2)}  seq ${a.sequential_ms} ms  all ${a.promise_all_ms} ms  speedup ${a.speedup}x  wrong-answers ${a.wrong_answers_vs_sequential}  errors ${a.errors}`,
    );
  }
  console.log(
    `gates: engine rule1 ${out.engine_only.gates.rule1_flagged_count}/${out.engine_only.gates.rule1_flagged_total} flagged, rule2 ${out.engine_only.gates.rule2_unstable_count}/${out.engine_only.gates.rule2_cases_total} unstable`,
  );
  console.log(`wrote ${OUT}`);
})();
