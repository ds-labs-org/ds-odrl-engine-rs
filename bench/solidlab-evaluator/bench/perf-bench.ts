/**
 * Perf instrumentation for the SolidLab ODRL Evaluator, single process.
 *
 * Measures the SAME thing allow-deny-bench.ts measures, on the SAME 68 cases,
 * through the SAME call: one `ODRLEvaluator(new ODRLEngineMultipleSteps())`
 * reused across cases, `evaluate(policyQuads, requestQuads, sotwQuads)` per
 * case, report reduced to Allow/Deny by ground_truth.rs's rule. The decision
 * of every timed evaluation is still checked against the fixture's expected
 * decision, so a perf run that silently stopped agreeing with the conformance
 * run cannot pass unnoticed -- pass/fail counts are in the output.
 *
 * Two paths are timed:
 *   "full"   -- Turtle parse + evaluate + reduce, i.e. byte-for-byte the
 *               region allow-deny-bench.ts puts its own `ms` around. This is
 *               the headline latency; it is comparable to results.json's `ms`.
 *   "engine" -- evaluate + reduce only, on quads parsed once up front. The
 *               difference between the two is the n3 Turtle parsing cost,
 *               which is not the reasoner.
 *
 * Also runs an intra-process concurrency probe (Promise.all over N evaluate()
 * calls vs the same N sequentially) whose only purpose is to justify, with a
 * real number, why load-bench.ts defines concurrency as OS processes rather
 * than as concurrent async evaluations. See load-bench.ts's header.
 *
 * Invocation (see README):
 *   OUT=results/perf-040.json npx ts-node bench/perf-bench.ts
 * Environment knobs, all with the defaults the committed results used:
 *   WARMUP=10  REPEATS=5  ENGINE_REPEATS=2  CONC_PROBE=4
 *
 * Peak RSS and CPU seconds for this process are NOT self-reported: run it
 * under `/usr/bin/time -v`, which is what run-perf.sh does. What is
 * self-reported here is the per-case RSS series (sampled after every
 * evaluation), for steady-state and growth.
 */
import * as fs from "fs";
import * as os from "os";
import { Quad } from "n3";
import { ODRLEngineMultipleSteps, ODRLEvaluator } from "odrl-evaluator";
import { Decision, LoadedCase, loadCases, parseFile, preload, reduceToDecision, round3, stats } from "./perf-corpus";

const WARMUP = parseInt(process.env.WARMUP || "10", 10);
const REPEATS = parseInt(process.env.REPEATS || "5", 10);
const ENGINE_REPEATS = parseInt(process.env.ENGINE_REPEATS || "2", 10);
const CONC_PROBE = parseInt(process.env.CONC_PROBE || "4", 10);

/**
 * Outlier / stability gates. Both are stated numeric rules, applied to the
 * run's own distribution; a flagged measurement is reported, never dropped.
 *
 *  TUKEY_K   -- a single case-measurement is flagged `outlier` when it falls
 *               outside [Q1 - k*IQR, Q3 + k*IQR] of the pooled distribution
 *               of that path's own measurements. k = 1.5 is the standard
 *               Tukey fence.
 *  UNSTABLE_RELATIVE_RANGE -- a *case* (not a measurement) is flagged
 *               `unstable` when (max - min) / median across its REPEATS
 *               measurements exceeds this. 0.25 = the slowest repeat is more
 *               than a quarter of the median away from the fastest.
 */
const TUKEY_K = 1.5;
const UNSTABLE_RELATIVE_RANGE = 0.25;

type Path = "full" | "engine";

interface Sample {
    slug: string;
    seq: number;
    repeat: number;
    path: Path;
    ms: number;
    outcome: "PASS" | "FAIL" | "ERROR";
    rssBytes: number;
    outlier?: "high" | "low";
}

const evaluator = new ODRLEvaluator(new ODRLEngineMultipleSteps());

/** The one call under test. Same @ts-ignore the suite's own TestCaseEvaluator uses. */
async function evaluateQuads(p: Quad[], r: Quad[], s: Quad[]): Promise<Quad[]> {
    // @ts-ignore  (upstream typings)
    return await evaluator.evaluate(p, r, s);
}

async function timeOne(c: LoadedCase, path: Path): Promise<{ ms: number; outcome: "PASS" | "FAIL" | "ERROR"; error?: string }> {
    const t0 = performance.now();
    let actual: Decision | null = null;
    let error: string | undefined;
    try {
        const quads = path === "full"
            ? await evaluateQuads(parseFile(c.policy), parseFile(c.request), parseFile(c.sotw))
            : await evaluateQuads(c.policyQuads.slice(), c.requestQuads.slice(), c.sotwQuads.slice());
        actual = reduceToDecision(quads);
    } catch (err) {
        error = err instanceof Error ? err.message : String(err);
    }
    const ms = performance.now() - t0;
    const outcome = error ? "ERROR" : actual === c.expectedDecision ? "PASS" : "FAIL";
    return { ms, outcome, error };
}

/** Apply TUKEY_K to one path's pooled measurements, in place. */
function flagOutliers(samples: Sample[]): { fenceLow: number; fenceHigh: number; flagged: number } {
    const st = stats(samples.map((s) => s.ms));
    const fenceLow = st.q1 - TUKEY_K * st.iqr;
    const fenceHigh = st.q3 + TUKEY_K * st.iqr;
    let flagged = 0;
    for (const s of samples) {
        if (s.ms > fenceHigh) { s.outlier = "high"; flagged++; }
        else if (s.ms < fenceLow) { s.outlier = "low"; flagged++; }
    }
    return { fenceLow, fenceHigh, flagged };
}

function perCase(samples: Sample[]) {
    const bySlug = new Map<string, Sample[]>();
    for (const s of samples) {
        if (!bySlug.has(s.slug)) bySlug.set(s.slug, []);
        bySlug.get(s.slug)!.push(s);
    }
    return Array.from(bySlug.entries()).map(([slug, ss]) => {
        const st = stats(ss.map((s) => s.ms));
        const relRange = st.median > 0 ? (st.max - st.min) / st.median : 0;
        return {
            slug,
            seq: ss[0].seq,
            n: st.n,
            mean_ms: round3(st.mean),
            median_ms: round3(st.median),
            min_ms: round3(st.min),
            max_ms: round3(st.max),
            stddev_ms: round3(st.stddev),
            relative_range: round3(relRange),
            unstable: relRange > UNSTABLE_RELATIVE_RANGE,
            outlier_measurements: ss.filter((s) => s.outlier).length,
            outcomes: ss.map((s) => s.outcome).filter((v, i, a) => a.indexOf(v) === i),
        };
    }).sort((a, b) => a.seq - b.seq);
}

function summarize(samples: Sample[]) {
    const st = stats(samples.map((s) => s.ms));
    return {
        n: st.n,
        mean_ms: round3(st.mean),
        median_ms: round3(st.median),
        p95_ms: round3(st.p95),
        p99_ms: round3(st.p99),
        min_ms: round3(st.min),
        max_ms: round3(st.max),
        stddev_ms: round3(st.stddev),
        q1_ms: round3(st.q1),
        q3_ms: round3(st.q3),
        iqr_ms: round3(st.iqr),
    };
}

async function main() {
    const t_start = Date.now();
    const cases = preload(loadCases());
    console.error(`cases from index.ttl: ${cases.length}`);

    // ---- 1. warmup / smoke test. Discarded, but recorded so cold start is visible.
    const warmupMs: number[] = [];
    for (let i = 0; i < WARMUP; i++) {
        const c = cases[i % cases.length];
        const r = await timeOne(c, "full");
        warmupMs.push(round3(r.ms));
        if (r.outcome === "ERROR") throw new Error(`warmup failed on ${c.slug}: ${r.error}`);
        console.error(`warmup ${i + 1}/${WARMUP} ${c.slug} ${Math.round(r.ms)}ms ${r.outcome}`);
    }
    const cpuAfterWarmup = process.cpuUsage();

    // ---- 2. timed sweeps.
    const samples: Sample[] = [];
    const perRepeat: { path: Path; repeat: number; wall_ms: number; median_ms: number; pass: number; fail: number; err: number }[] = [];

    const sweep = async (path: Path, repeats: number) => {
        for (let rep = 0; rep < repeats; rep++) {
            const t0 = performance.now();
            const mine: Sample[] = [];
            for (const c of cases) {
                const r = await timeOne(c, path);
                const s: Sample = {
                    slug: c.slug, seq: c.seq, repeat: rep, path,
                    ms: round3(r.ms), outcome: r.outcome, rssBytes: process.memoryUsage().rss,
                };
                mine.push(s);
                samples.push(s);
            }
            const wall = performance.now() - t0;
            perRepeat.push({
                path, repeat: rep,
                wall_ms: round3(wall),
                median_ms: round3(stats(mine.map((s) => s.ms)).median),
                pass: mine.filter((s) => s.outcome === "PASS").length,
                fail: mine.filter((s) => s.outcome === "FAIL").length,
                err: mine.filter((s) => s.outcome === "ERROR").length,
            });
            console.error(`${path} repeat ${rep + 1}/${repeats}: ${Math.round(wall)}ms wall, median ${Math.round(stats(mine.map((s) => s.ms)).median)}ms`);
        }
    };

    await sweep("full", REPEATS);
    await sweep("engine", ENGINE_REPEATS);

    const full = samples.filter((s) => s.path === "full");
    const engine = samples.filter((s) => s.path === "engine");
    const fullFence = flagOutliers(full);
    const engineFence = flagOutliers(engine);

    // ---- 3. intra-process concurrency probe (justifies load-bench.ts's definition).
    const probeCase = cases[0];
    const one = () => evaluateQuads(probeCase.policyQuads.slice(), probeCase.requestQuads.slice(), probeCase.sotwQuads.slice());
    let t0 = performance.now();
    for (let i = 0; i < CONC_PROBE; i++) await one();
    const seqMs = performance.now() - t0;
    t0 = performance.now();
    await Promise.all(Array.from({ length: CONC_PROBE }, () => one()));
    const parMs = performance.now() - t0;

    const rssSeries = samples.map((s) => s.rssBytes);
    const cpu = process.cpuUsage();

    const out = {
        schema: "solidlab-evaluator/perf-bench@1",
        phase: "perf",
        engine: "SolidLabResearch/ODRL-Evaluator",
        version: require("odrl-evaluator/package.json").version,
        node_version: process.version,
        corpus_cases: cases.length,
        started_utc: new Date(t_start).toISOString(),
        wall_seconds: round3((Date.now() - t_start) / 1000),
        host: { nproc: os.cpus().length, totalmem_bytes: os.totalmem(), loadavg_at_start: os.loadavg() },
        method: {
            warmup_iterations: WARMUP,
            warmup_note: "cycled over the first WARMUP cases of the same corpus, results discarded; a warmup ERROR aborts the run",
            repeats_full: REPEATS,
            repeats_engine: ENGINE_REPEATS,
            paths: {
                full: "parseFile(policy)+parseFile(request)+parseFile(sotw) -> evaluate() -> reduceToDecision(); identical to allow-deny-bench.ts's timed region",
                engine: "evaluate() -> reduceToDecision() on quads parsed once up front; excludes n3 Turtle parsing",
            },
            evaluator: "one ODRLEvaluator(new ODRLEngineMultipleSteps()) reused for the whole process, as in allow-deny-bench.ts",
            outlier_rule: `Tukey fence k=${TUKEY_K} on the pooled per-path measurements: flagged when outside [Q1-${TUKEY_K}*IQR, Q3+${TUKEY_K}*IQR]. Flagged, never dropped.`,
            stability_rule: `a case is 'unstable' when (max-min)/median over its ${REPEATS} full-path measurements > ${UNSTABLE_RELATIVE_RANGE}`,
        },
        warmup_ms: warmupMs,
        per_repeat: perRepeat,
        latency: {
            full: summarize(full),
            engine: summarize(engine),
            parse_overhead_ms_median: round3(summarize(full).median_ms - summarize(engine).median_ms),
        },
        outliers: {
            full: { fence_low_ms: round3(fullFence.fenceLow), fence_high_ms: round3(fullFence.fenceHigh), flagged: fullFence.flagged, of: full.length },
            engine: { fence_low_ms: round3(engineFence.fenceLow), fence_high_ms: round3(engineFence.fenceHigh), flagged: engineFence.flagged, of: engine.length },
        },
        conformance_check: {
            full: { pass: full.filter((s) => s.outcome === "PASS").length / REPEATS, fail: full.filter((s) => s.outcome === "FAIL").length / REPEATS, err: full.filter((s) => s.outcome === "ERROR").length / REPEATS },
        },
        memory_in_process: {
            note: "process.memoryUsage().rss sampled after every timed evaluation; authoritative peak RSS comes from /usr/bin/time -v around this process",
            first_rss_bytes: rssSeries[0],
            last_rss_bytes: rssSeries[rssSeries.length - 1],
            max_rss_bytes: Math.max(...rssSeries),
            median_rss_bytes: stats(rssSeries).median,
            growth_bytes: rssSeries[rssSeries.length - 1] - rssSeries[0],
        },
        cpu_self_reported: {
            note: "process.cpuUsage(), microseconds, whole process incl. startup",
            after_warmup_user_s: round3(cpuAfterWarmup.user / 1e6),
            after_warmup_system_s: round3(cpuAfterWarmup.system / 1e6),
            total_user_s: round3(cpu.user / 1e6),
            total_system_s: round3(cpu.system / 1e6),
        },
        intra_process_concurrency_probe: {
            n: CONC_PROBE,
            sequential_ms: round3(seqMs),
            promise_all_ms: round3(parMs),
            speedup: round3(seqMs / parMs),
            verdict: seqMs / parMs < 1.2
                ? "no intra-process parallelism: Promise.all over evaluate() is ~as slow as running them sequentially, so concurrency for this engine has to mean OS processes"
                : "Promise.all showed real overlap; revisit load-bench.ts's process-based definition",
        },
        per_case_full: perCase(full),
        per_case_engine: perCase(engine),
        samples,
    };

    const dest = process.env.OUT || "perf.json";
    fs.writeFileSync(dest, JSON.stringify(out, null, 2));
    console.error(`wrote ${dest}`);
    console.error(`full: median ${out.latency.full.median_ms}ms p95 ${out.latency.full.p95_ms}ms p99 ${out.latency.full.p99_ms}ms  outliers ${fullFence.flagged}/${full.length}`);
    console.error(`unstable cases: ${out.per_case_full.filter((c) => c.unstable).length}/${cases.length}`);
    console.error(`concurrency probe speedup: ${out.intra_process_concurrency_probe.speedup}x`);
}
main();
