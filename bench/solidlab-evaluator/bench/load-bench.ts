/**
 * Load / peak-behaviour ramp for the SolidLab ODRL Evaluator.
 *
 * WHAT "CONCURRENCY" MEANS HERE, AND WHY
 * --------------------------------------
 * One concurrency unit = one OS process (`ts-node bench/load-worker.ts`), each
 * with its own ODRLEvaluator and its own WASM-compiled EYE reasoner instance,
 * each looping over the same 68-case corpus from a different offset.
 *
 * It is NOT "N concurrent async evaluations in one process". That was measured,
 * not assumed: perf-bench.ts's intra_process_concurrency_probe runs N
 * evaluate() calls under Promise.all against the same N run sequentially, and
 * the speedup is ~1.0x. `evaluate()` is an async signature over a synchronous
 * WASM reasoner run, so it occupies Node's single thread for its whole
 * duration; awaiting several at once interleaves nothing. Real parallelism for
 * this engine therefore has to come from OS processes.
 *
 * This is NOT comparable to a threaded/in-process concurrency number from
 * another engine in this comparison: a process here carries a whole Node
 * runtime and its own reasoner heap (~0.5 GB), so the memory cost per unit of
 * concurrency is enormous compared to a native thread, and it is the memory,
 * not the CPU, that sets the ceiling on this box.
 *
 * METHOD
 * ------
 * For each ramp repeat: spawn MAX_CONCURRENCY workers once, wait for every one
 * to finish its own warmup and report ready, then walk the ramp by activating
 * only the first K of them per step. Idle workers stay blocked on stdin, so
 * they hold memory but burn no CPU -- which is itself the honest condition,
 * since the memory of the whole pool is what a K-process deployment would need
 * to have provisioned. Per step the host samples /proc/<pid>/status VmRSS and
 * /proc/<pid>/stat utime+stime for every ACTIVE worker every SAMPLE_MS, so CPU
 * and RSS are measured by the kernel, not self-reported.
 *
 * Invocation (see README):
 *   OUT=results/load-040.json npx ts-node bench/load-bench.ts
 * Environment knobs, all with the defaults the committed results used:
 *   LEVELS=1,2,4,8,16,22,28  STEP_SECONDS=10  RAMP_REPEATS=3
 *   WORKER_WARMUP=5  SAMPLE_MS=250  MIN_FREE_MB=6144
 *   SPAWN_STAGGER_MS=150  READY_TIMEOUT_MS=300000  REPLY_TIMEOUT_MS=120000
 *   HEAVY_SEQS=62,63,64
 *
 * MIN_FREE_MB is a real abort guard, not decoration: the host reads
 * MemAvailable from /proc/meminfo before spawning each pool and before each
 * step, and stops the ramp (recording why) rather than pushing a shared
 * workstation into swap. Any step it refuses is reported as skipped with the
 * reason, never silently omitted.
 */
import { ChildProcessWithoutNullStreams, spawn } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as readline from "readline";
import { round3, stats } from "./perf-corpus";

const LEVELS = (process.env.LEVELS || "1,2,4,8,16,22,28").split(",").map((s) => parseInt(s.trim(), 10));
const STEP_SECONDS = parseFloat(process.env.STEP_SECONDS || "10");
const RAMP_REPEATS = parseInt(process.env.RAMP_REPEATS || "3", 10);
const WORKER_WARMUP = process.env.WORKER_WARMUP || "5";
const SAMPLE_MS = parseInt(process.env.SAMPLE_MS || "250", 10);
const MIN_FREE_MB = parseInt(process.env.MIN_FREE_MB || "6144", 10);
const SPAWN_STAGGER_MS = parseInt(process.env.SPAWN_STAGGER_MS || "150", 10);
const READY_TIMEOUT_MS = parseInt(process.env.READY_TIMEOUT_MS || "300000", 10);
/** A single evaluation can take ~14 s (the big-policy cases), so a step's own
 *  reply budget is the step length plus generous slack for one of those. */
const REPLY_TIMEOUT_MS = parseInt(process.env.REPLY_TIMEOUT_MS || "120000", 10);
/**
 * The testcase numbers perf-bench.ts measured at ~13.5 s each while all other
 * 65 cases sit at ~0.7 s (062/063/064, the "big-policy" fixtures). They are NOT
 * excluded from the run -- every worker still evaluates them, and they are in
 * every headline number. They are labelled so the ramp can report a second,
 * heavy-case-free view of latency, because a 10 s window at concurrency 1 walks
 * ~15 cheap cases and never reaches one, while a window at concurrency 28 has
 * several workers parked inside one for its whole duration. Without that split,
 * corpus composition and contention are impossible to tell apart.
 */
const HEAVY_SEQS = new Set((process.env.HEAVY_SEQS || "62,63,64").split(",").map((x) => parseInt(x.trim(), 10)));
const MAX_CONCURRENCY = Math.max(...LEVELS);

/**
 * Stability gate for the ramp, stated as a numeric rule: a concurrency level
 * whose throughput relative range across the RAMP_REPEATS repeats --
 * (max - min) / median -- exceeds this is flagged `unstable_across_repeats`.
 * Flagged, still reported.
 */
const UNSTABLE_THROUGHPUT_RELATIVE_RANGE = 0.15;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function memAvailableMb(): number {
    const m = /MemAvailable:\s+(\d+) kB/.exec(fs.readFileSync("/proc/meminfo", "utf-8"));
    return m ? parseInt(m[1], 10) / 1024 : NaN;
}

function procRssBytes(pid: number): number {
    try {
        const m = /VmRSS:\s+(\d+) kB/.exec(fs.readFileSync(`/proc/${pid}/status`, "utf-8"));
        return m ? parseInt(m[1], 10) * 1024 : 0;
    } catch { return 0; }
}

/** utime+stime of pid, in seconds (fields 14,15 of /proc/pid/stat, after the comm field). */
function procCpuSeconds(pid: number): number {
    try {
        const raw = fs.readFileSync(`/proc/${pid}/stat`, "utf-8");
        const rest = raw.slice(raw.lastIndexOf(")") + 2).split(" ");
        const hz = 100; // USER_HZ, fixed at 100 on Linux
        return (parseInt(rest[11], 10) + parseInt(rest[12], 10)) / hz;
    } catch { return 0; }
}

interface Worker {
    proc: ChildProcessWithoutNullStreams;
    index: number;
    pid: number;
    lines: string[];
    waiter: ((l: string) => void) | null;
    warmupMs: number[];
    dead: string | null;
}

function attach(proc: ChildProcessWithoutNullStreams, index: number): Worker {
    const w: Worker = { proc, index, pid: proc.pid!, lines: [], waiter: null, warmupMs: [], dead: null };
    readline.createInterface({ input: proc.stdout }).on("line", (l) => {
        if (w.waiter) { const f = w.waiter; w.waiter = null; f(l); } else w.lines.push(l);
    });
    proc.stderr.on("data", (d) => process.stderr.write(`[w${w.index}/${w.pid}] ${d}`));
    proc.on("exit", (code, sig) => {
        w.dead = `exited code=${code} signal=${sig}`;
        if (w.waiter) { const f = w.waiter; w.waiter = null; f(""); } // unblock, `next` turns "" into a throw
    });
    proc.on("error", (e) => { w.dead = `spawn error: ${e.message}`; });
    return w;
}

/**
 * Await one line from a worker. Every worker read in this file goes through
 * here, and every one is bounded: an unbounded await on a child that died or
 * wedged is the difference between a failed run that says why and a ramp that
 * hangs forever holding 28 processes (which is exactly what the first
 * unguarded version of this script did).
 */
function next(w: Worker, what: string, timeoutMs: number): Promise<any> {
    if (w.waiter) return Promise.reject(new Error(`w${w.index}: two concurrent reads (${what})`));
    return new Promise((res, rej) => {
        let done = false;
        const finish = (l: string) => {
            if (done) return;
            done = true;
            clearTimeout(timer);
            if (!l) rej(new Error(`w${w.index}/${w.pid}: ${w.dead || "empty line"} while awaiting ${what}`));
            else { try { res(JSON.parse(l)); } catch (e) { rej(new Error(`w${w.index}: unparseable line awaiting ${what}: ${l.slice(0, 200)}`)); } }
        };
        const timer = setTimeout(() => {
            if (done) return;
            done = true;
            w.waiter = null;
            rej(new Error(`w${w.index}/${w.pid}: timed out after ${timeoutMs} ms awaiting ${what}${w.dead ? ` (${w.dead})` : ""}`));
        }, timeoutMs);
        const q = w.lines.shift();
        if (q !== undefined) finish(q);
        else w.waiter = finish;
    });
}

const send = (w: Worker, o: any) => w.proc.stdin.write(JSON.stringify(o) + "\n");

async function spawnPool(n: number): Promise<Worker[]> {
    const script = path.join(__dirname, "load-worker.ts");
    const root = path.join(__dirname, "..");
    // Spawn `node node_modules/.bin/ts-node <worker>` directly rather than via
    // `npx ts-node`: npx is a wrapper process, and its pid is NOT the pid that
    // holds the reasoner, so /proc sampling on it reports the wrapper's ~90 MB
    // and ~0 CPU instead of the worker's. It is the same binary npx resolves to.
    const tsNode = path.join(root, "node_modules", ".bin", "ts-node");
    const workers: Worker[] = [];
    const t0 = Date.now();
    for (let i = 0; i < n; i++) {
        const proc = spawn(process.execPath, [tsNode, script], {
            cwd: root,
            env: { ...process.env, WORKER_OFFSET: String((i * 7) % 68), WORKER_WARMUP },
            stdio: ["pipe", "pipe", "pipe"],
        }) as ChildProcessWithoutNullStreams;
        workers.push(attach(proc, i));
        if (SPAWN_STAGGER_MS > 0) await sleep(SPAWN_STAGGER_MS);
    }
    let ready = 0;
    await Promise.all(workers.map(async (w) => {
        w.warmupMs = (await next(w, "ready", READY_TIMEOUT_MS)).warmupMs;
        ready++;
        if (ready % 4 === 0 || ready === n) console.error(`  ready ${ready}/${n} at ${((Date.now() - t0) / 1000).toFixed(1)}s`);
    }));
    console.error(`pool of ${n} ready in ${((Date.now() - t0) / 1000).toFixed(1)}s`);
    return workers;
}

function killPool(workers: Worker[]) {
    for (const w of workers) { try { send(w, { cmd: "exit" }); } catch { /* already gone */ } }
    for (const w of workers) { try { w.proc.kill("SIGKILL"); } catch { /* already gone */ } }
}

/** Never leave workers behind, whatever happens to this process. */
let livePools: Worker[][] = [];
for (const sig of ["SIGINT", "SIGTERM", "uncaughtException"] as const) {
    process.on(sig as any, (e: any) => {
        for (const p of livePools) killPool(p);
        if (sig === "uncaughtException") console.error(e);
        process.exit(1);
    });
}

interface StepResult {
    concurrency: number;
    repeat: number;
    skipped?: string;
    wall_seconds: number;
    evaluations: number;
    exceptions: number;
    mismatches: number;
    exception_rate: number;
    mismatch_rate: number;
    throughput_evals_per_sec: number;
    /**
     * Evaluations that COMPLETED inside the nominal STEP_SECONDS window,
     * divided by STEP_SECONDS. `throughput_evals_per_sec` above divides by the
     * step's real wall time, which at high concurrency is ~2.5x the nominal
     * window: an evaluation cannot be interrupted, so a worker that has entered
     * a ~13.5 s big-policy case keeps the whole step open until it finishes.
     * Both are reported. The wall-clock one is the honest end-to-end figure for
     * "drain a burst of N concurrent requests"; this one is the honest
     * steady-state figure for "requests served per second".
     */
    throughput_in_window_evals_per_sec: number;
    evaluations_in_window: number;
    latency_ms: ReturnType<typeof stats>;
    latency_ms_light: ReturnType<typeof stats>;   // excluding HEAVY_SEQS
    heavy_evaluations: number;
    heavy_share: number;
    /** Raw per-worker latency/testcase arrays, so any of the above can be recomputed. */
    per_worker: { index: number; latencies: number[]; seqs: number[] }[];
    kernel: {
        samples: number;
        active_rss_peak_bytes: number;
        active_rss_steady_bytes: number;
        pool_rss_peak_bytes: number;
        cpu_seconds_active: number;
        cores_busy: number;
    };
    host_loadavg_1m: number;
    mem_available_mb_before: number;
}

async function runStep(pool: Worker[], k: number, repeat: number): Promise<StepResult> {
    const active = pool.slice(0, k);
    const avail = memAvailableMb();
    const blank = {
        concurrency: k, repeat, wall_seconds: 0, evaluations: 0, exceptions: 0, mismatches: 0,
        exception_rate: 0, mismatch_rate: 0, throughput_evals_per_sec: 0,
        throughput_in_window_evals_per_sec: 0, evaluations_in_window: 0, per_worker: [],
        latency_ms: stats([]), latency_ms_light: stats([]), heavy_evaluations: 0, heavy_share: 0,
        host_loadavg_1m: os.loadavg()[0], mem_available_mb_before: round3(avail),
        kernel: { samples: 0, active_rss_peak_bytes: 0, active_rss_steady_bytes: 0, pool_rss_peak_bytes: 0, cpu_seconds_active: 0, cores_busy: 0 },
    };
    if (avail < MIN_FREE_MB) {
        return { ...blank, skipped: `MemAvailable ${Math.round(avail)} MB < MIN_FREE_MB ${MIN_FREE_MB} MB` };
    }

    const cpu0 = new Map(active.map((w) => [w.pid, procCpuSeconds(w.pid)]));
    const activeRss: number[] = [];
    const poolRss: number[] = [];
    let samples = 0;
    const sampler = setInterval(() => {
        activeRss.push(active.reduce((a, w) => a + procRssBytes(w.pid), 0));
        poolRss.push(pool.reduce((a, w) => a + procRssBytes(w.pid), 0));
        samples++;
    }, SAMPLE_MS);

    const t0 = performance.now();
    for (const w of active) send(w, { cmd: "go", step: k });
    await Promise.all(active.map((w) => next(w, "started", REPLY_TIMEOUT_MS)));
    await sleep(STEP_SECONDS * 1000);
    for (const w of active) send(w, { cmd: "stop" });
    const reports = await Promise.all(active.map((w) => next(w, "step report", REPLY_TIMEOUT_MS + STEP_SECONDS * 1000)));
    const wall = (performance.now() - t0) / 1000;
    clearInterval(sampler);

    const cpuSeconds = active.reduce((a, w) => a + (procCpuSeconds(w.pid) - (cpu0.get(w.pid) || 0)), 0);
    const latencies: number[] = [];
    const lightLatencies: number[] = [];
    const perWorker: { index: number; latencies: number[]; seqs: number[] }[] = [];
    let heavy = 0;
    let inWindow = 0;
    for (let j = 0; j < reports.length; j++) {
        const r = reports[j];
        const ls = r.latencies as number[];
        const ss = r.seqs as number[];
        perWorker.push({ index: active[j].index, latencies: ls, seqs: ss });
        let elapsed = 0;
        for (let i = 0; i < ls.length; i++) {
            latencies.push(ls[i]);
            if (HEAVY_SEQS.has(ss[i])) heavy++; else lightLatencies.push(ls[i]);
            // Worker loops back-to-back, so cumulative latency is its completion clock.
            elapsed += ls[i];
            if (elapsed <= STEP_SECONDS * 1000) inWindow++;
        }
    }
    const evals = reports.reduce((a, r) => a + r.evals, 0);
    const exceptions = reports.reduce((a, r) => a + r.exceptions, 0);
    const mismatches = reports.reduce((a, r) => a + r.mismatches, 0);
    // Steady state = median of the samples after the first second, to drop the ramp-in.
    const steadyFrom = Math.min(Math.ceil(1000 / SAMPLE_MS), Math.max(activeRss.length - 1, 0));
    const steady = activeRss.slice(steadyFrom);

    return {
        concurrency: k,
        repeat,
        wall_seconds: round3(wall),
        evaluations: evals,
        exceptions,
        mismatches,
        exception_rate: round3(evals ? exceptions / evals : 0),
        mismatch_rate: round3(evals ? mismatches / evals : 0),
        throughput_evals_per_sec: round3(evals / wall),
        throughput_in_window_evals_per_sec: round3(inWindow / STEP_SECONDS),
        evaluations_in_window: inWindow,
        per_worker: perWorker,
        latency_ms: stats(latencies),
        latency_ms_light: stats(lightLatencies),
        heavy_evaluations: heavy,
        heavy_share: round3(latencies.length ? heavy / latencies.length : 0),
        kernel: {
            samples,
            active_rss_peak_bytes: activeRss.length ? Math.max(...activeRss) : 0,
            active_rss_steady_bytes: steady.length ? Math.round(stats(steady).median) : 0,
            pool_rss_peak_bytes: poolRss.length ? Math.max(...poolRss) : 0,
            cpu_seconds_active: round3(cpuSeconds),
            cores_busy: round3(cpuSeconds / wall),
        },
        host_loadavg_1m: os.loadavg()[0],
        mem_available_mb_before: round3(avail),
    };
}

async function main() {
    const t_start = Date.now();
    const version = require("odrl-evaluator/package.json").version;
    console.error(`load-bench: odrl-evaluator ${version}, levels ${LEVELS.join(",")}, ${STEP_SECONDS}s steps, ${RAMP_REPEATS} repeats`);

    const all: StepResult[] = [];
    const poolWarmup: number[][] = [];
    for (let rep = 0; rep < RAMP_REPEATS; rep++) {
        if (memAvailableMb() < MIN_FREE_MB) {
            console.error(`ramp repeat ${rep}: refusing to spawn pool, MemAvailable ${Math.round(memAvailableMb())} MB`);
            break;
        }
        const pool = await spawnPool(MAX_CONCURRENCY);
        livePools = [pool];
        poolWarmup.push(pool.map((w) => stats(w.warmupMs).median));
        try {
            for (const k of LEVELS) {
                const r = await runStep(pool, k, rep);
                all.push(r);
                if (r.skipped) console.error(`  c=${k} rep=${rep} SKIPPED: ${r.skipped}`);
                else console.error(`  c=${k} rep=${rep}: ${r.throughput_evals_per_sec.toFixed(2)} eval/s, median ${Math.round(r.latency_ms.median)}ms, p99 ${Math.round(r.latency_ms.p99)}ms, rss ${(r.kernel.active_rss_peak_bytes / 2 ** 30).toFixed(2)} GiB, cores ${r.kernel.cores_busy.toFixed(1)}, exc ${r.exceptions}`);
            }
        } finally {
            killPool(pool);
            livePools = [];
            await sleep(1500); // let the kernel reap before the next pool
        }
    }

    // Aggregate per level across repeats, with the stability gate applied.
    const levels = LEVELS.map((k) => {
        const rs = all.filter((r) => r.concurrency === k && !r.skipped);
        const skipped = all.filter((r) => r.concurrency === k && r.skipped);
        if (rs.length === 0) return { concurrency: k, repeats: 0, skipped: skipped.map((s) => s.skipped) };
        const tp = rs.map((r) => r.throughput_evals_per_sec);
        const tpStats = stats(tp);
        const relRange = tpStats.median > 0 ? (tpStats.max - tpStats.min) / tpStats.median : 0;
        return {
            concurrency: k,
            repeats: rs.length,
            throughput_evals_per_sec_median: round3(tpStats.median),
            throughput_in_window_evals_per_sec_median: round3(stats(rs.map((r) => r.throughput_in_window_evals_per_sec)).median),
            wall_seconds_median: round3(stats(rs.map((r) => r.wall_seconds)).median),
            throughput_per_repeat: tp,
            throughput_relative_range: round3(relRange),
            unstable_across_repeats: relRange > UNSTABLE_THROUGHPUT_RELATIVE_RANGE,
            latency_ms_median_of_repeats: round3(stats(rs.map((r) => r.latency_ms.median)).median),
            latency_light_ms_median_of_repeats: round3(stats(rs.map((r) => r.latency_ms_light.median)).median),
            latency_light_ms_p95_median_of_repeats: round3(stats(rs.map((r) => r.latency_ms_light.p95)).median),
            latency_light_ms_p99_median_of_repeats: round3(stats(rs.map((r) => r.latency_ms_light.p99)).median),
            heavy_evaluations_total: rs.reduce((a, r) => a + r.heavy_evaluations, 0),
            heavy_share: round3(rs.reduce((a, r) => a + r.heavy_evaluations, 0) / Math.max(rs.reduce((a, r) => a + r.evaluations, 0), 1)),
            latency_ms_p95_median_of_repeats: round3(stats(rs.map((r) => r.latency_ms.p95)).median),
            latency_ms_p99_median_of_repeats: round3(stats(rs.map((r) => r.latency_ms.p99)).median),
            latency_ms_max: round3(Math.max(...rs.map((r) => r.latency_ms.max))),
            evaluations_total: rs.reduce((a, r) => a + r.evaluations, 0),
            exceptions_total: rs.reduce((a, r) => a + r.exceptions, 0),
            exception_rate: round3(rs.reduce((a, r) => a + r.exceptions, 0) / Math.max(rs.reduce((a, r) => a + r.evaluations, 0), 1)),
            mismatch_rate: round3(rs.reduce((a, r) => a + r.mismatches, 0) / Math.max(rs.reduce((a, r) => a + r.evaluations, 0), 1)),
            active_rss_peak_bytes_max: Math.max(...rs.map((r) => r.kernel.active_rss_peak_bytes)),
            active_rss_steady_bytes_median: Math.round(stats(rs.map((r) => r.kernel.active_rss_steady_bytes)).median),
            pool_rss_peak_bytes_max: Math.max(...rs.map((r) => r.kernel.pool_rss_peak_bytes)),
            cores_busy_median: round3(stats(rs.map((r) => r.kernel.cores_busy)).median),
            skipped: skipped.map((s) => s.skipped),
        };
    });

    const out = {
        schema: "solidlab-evaluator/perf-bench@1",
        phase: "load",
        engine: "SolidLabResearch/ODRL-Evaluator",
        version,
        node_version: process.version,
        started_utc: new Date(t_start).toISOString(),
        wall_seconds: round3((Date.now() - t_start) / 1000),
        host: { nproc: os.cpus().length, totalmem_bytes: os.totalmem(), loadavg_at_start: os.loadavg() },
        method: {
            concurrency_means: "one OS process per unit of concurrency: `npx ts-node bench/load-worker.ts`, each with its own ODRLEvaluator and its own WASM EYE reasoner, looping the same 68-case corpus from a different offset. NOT in-process async concurrency -- see perf-bench.ts's intra_process_concurrency_probe, which measures ~1.0x speedup for Promise.all over evaluate(). Not comparable to a thread-based concurrency figure from another engine.",
            levels: LEVELS,
            step_seconds: STEP_SECONDS,
            throughput_note: "throughput_evals_per_sec divides by the step's REAL wall time; throughput_in_window_evals_per_sec counts only evaluations that completed within the nominal STEP_SECONDS. They diverge above concurrency 8 because an evaluation cannot be interrupted: a worker that has entered a ~13.5 s big-policy case holds the step open past its nominal end, so wall_seconds_median rises to ~25 s for a 10 s step.",
            ramp_repeats: RAMP_REPEATS,
            worker_warmup_evaluations: parseInt(WORKER_WARMUP, 10),
            pool: `MAX_CONCURRENCY=${MAX_CONCURRENCY} workers spawned once per ramp repeat; a step activates the first K and leaves the rest blocked on stdin (memory held, no CPU)`,
            sampling: `/proc/<pid>/status VmRSS and /proc/<pid>/stat utime+stime for every active worker every ${SAMPLE_MS} ms; CPU and RSS are kernel-reported, not self-reported`,
            steady_state_rule: "median of the aggregate active-RSS samples taken after the first second of the step",
            heavy_case_split: `testcases ${Array.from(HEAVY_SEQS).join("/")} take ~13.5 s each against ~0.7 s for the other 65, so every step also reports latency_ms_light (the same window with those cases removed) and heavy_share (their fraction of the window's evaluations). Nothing is excluded from the run or from the headline latency_ms -- this is a second view, so that a reader can tell contention from corpus composition.`,
            ceiling_justification: `ceiling is set by memory, not cores: each worker holds ~0.5 GiB RSS, so ${MAX_CONCURRENCY} workers is already ~${(MAX_CONCURRENCY * 0.5).toFixed(0)} GiB on a ${os.cpus().length}-core box. MIN_FREE_MB=${MIN_FREE_MB} aborts a step rather than swapping a shared workstation.`,
            stability_rule: `a level is 'unstable_across_repeats' when its throughput (max-min)/median across repeats > ${UNSTABLE_THROUGHPUT_RELATIVE_RANGE}. Flagged, never dropped.`,
        },
        worker_warmup_median_ms_per_pool: poolWarmup.map((p) => round3(stats(p).median)),
        levels,
        steps: all,
    };

    const dest = process.env.OUT || "load.json";
    fs.writeFileSync(dest, JSON.stringify(out, null, 2));
    console.error(`wrote ${dest}`);
}
main().catch((e) => { console.error(e); process.exit(1); });
