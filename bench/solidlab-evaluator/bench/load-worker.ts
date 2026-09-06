/**
 * One load-generator worker for load-bench.ts. Not run by hand.
 *
 * Protocol over stdin/stdout, one JSON object per line:
 *   worker -> {"ready":true,"pid":N,"warmupMs":[...]}      after its own warmup
 *   host   -> {"cmd":"go","step":K}                         start a timed window
 *   worker -> {"started":true,"step":K}
 *   host   -> {"cmd":"stop"}                                end the window
 *   worker -> {"step":K,"evals":N,"exceptions":N,"mismatches":N,"latencies":[ms,...],
 *              "seqs":[testcase number per latency, same order],...}
 *   host   -> {"cmd":"exit"}
 *
 * A worker sits blocked on stdin between windows, so an idle worker in a
 * ramp costs memory but effectively no CPU.
 *
 * The evaluation it loops over is the SAME "full" path perf-bench.ts and
 * allow-deny-bench.ts time: parse three Turtle files, one evaluate(), reduce
 * to Allow/Deny, compared against the fixture's expected decision. Two
 * failure counters are kept apart on purpose: `exceptions` (evaluate() threw)
 * is the real under-load error rate, while `mismatches` (engine disagreed
 * with the fixture) has a nonzero *baseline* equal to the conformance run's
 * own failures -- 5/68 at 0.4.0, 1/68 at 0.6.0 -- so it is only interesting
 * if it rises above that baseline under load.
 * Cases are walked round-robin from a per-worker offset so N workers are not
 * all hammering the identical case at the identical moment.
 *
 * Every latency is reported WITH the testcase number it belongs to (`seqs`).
 * That is not bookkeeping for its own sake: three fixtures (062/063/064,
 * "big-policy") take ~13.5 s each while the other 65 take ~0.7 s, so a short
 * timed window at low concurrency may never reach one while a window at high
 * concurrency has several workers sitting inside one for its whole duration.
 * Without the case labels, that corpus-composition effect is indistinguishable
 * from contention. load-bench.ts uses them to report both.
 */
import * as readline from "readline";
import { Quad } from "n3";
import { ODRLEngineMultipleSteps, ODRLEvaluator } from "odrl-evaluator";
import { Case, Decision, loadCases, parseFile, reduceToDecision, reduceToDecisionOfFile } from "./perf-corpus";

const OFFSET = parseInt(process.env.WORKER_OFFSET || "0", 10);
const WARMUP = parseInt(process.env.WORKER_WARMUP || "5", 10);

const evaluator = new ODRLEvaluator(new ODRLEngineMultipleSteps());

/** Returns true when the engine's decision matches the fixture's expected one. */
async function evaluateCase(c: Case, expected: Decision): Promise<boolean> {
    // @ts-ignore  (upstream typings, as in allow-deny-bench.ts)
    const quads: Quad[] = await evaluator.evaluate(parseFile(c.policy), parseFile(c.request), parseFile(c.sotw));
    return reduceToDecision(quads) === expected;
}

async function main() {
    const cases = loadCases();
    // Ground truth read once, outside every timed window.
    const expected = new Map<string, Decision>();
    for (const c of cases) expected.set(c.slug, reduceToDecisionOfFile(c.expected));

    const warmupMs: number[] = [];
    for (let i = 0; i < WARMUP; i++) {
        const c = cases[(OFFSET + i) % cases.length];
        const t0 = performance.now();
        await evaluateCase(c, expected.get(c.slug)!);
        warmupMs.push(Math.round(performance.now() - t0));
    }

    const rl = readline.createInterface({ input: process.stdin });
    const queue: string[] = [];
    let waiter: ((l: string) => void) | null = null;
    rl.on("line", (l) => { if (waiter) { const w = waiter; waiter = null; w(l); } else queue.push(l); });
    const nextLine = () => new Promise<string>((res) => { const q = queue.shift(); if (q !== undefined) res(q); else waiter = res; });

    process.stdout.write(JSON.stringify({ ready: true, pid: process.pid, warmupMs }) + "\n");

    let cursor = OFFSET;
    for (;;) {
        const msg = JSON.parse(await nextLine());
        if (msg.cmd === "exit") break;
        if (msg.cmd !== "go") continue;

        let stop = false;
        // Watch for the stop line without blocking the eval loop.
        const stopSeen = nextLine().then((l) => { if (JSON.parse(l).cmd === "stop") stop = true; });
        const cpu0 = process.cpuUsage();
        const t0 = performance.now();
        const latencies: number[] = [];
        const seqs: number[] = [];
        let exceptions = 0;
        let mismatches = 0;
        process.stdout.write(JSON.stringify({ started: true, step: msg.step }) + "\n");
        while (!stop) {
            const c = cases[cursor++ % cases.length];
            const s0 = performance.now();
            try {
                if (!(await evaluateCase(c, expected.get(c.slug)!))) mismatches++;
            } catch { exceptions++; }
            latencies.push(Math.round((performance.now() - s0) * 1000) / 1000);
            seqs.push(c.seq);
        }
        await stopSeen;
        const cpu = process.cpuUsage(cpu0);
        process.stdout.write(JSON.stringify({
            step: msg.step,
            evals: latencies.length,
            exceptions,
            mismatches,
            wall_ms: Math.round((performance.now() - t0) * 1000) / 1000,
            cpu_user_s: Math.round(cpu.user / 1000) / 1000,
            cpu_system_s: Math.round(cpu.system / 1000) / 1000,
            rss_bytes: process.memoryUsage().rss,
            latencies,
            seqs,
        }) + "\n");
    }
    process.exit(0);
}
main();
