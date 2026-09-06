/**
 * Shared corpus loader + stats helpers for the three perf-instrumentation
 * scripts (perf-bench.ts, load-bench.ts, load-worker.ts).
 *
 * Case selection, source-URL rewriting and the Allow/Deny reduction are
 * byte-for-byte the same rules allow-deny-bench.ts uses, lifted into a module
 * so the perf scripts measure the *identical* invocation path the conformance
 * numbers were produced on (parse three Turtle files -> one
 * ODRLEvaluator.evaluate() call -> reduce the report), rather than a second,
 * subtly different way of calling the engine.
 *
 * The conformance harness itself is left untouched: it keeps its own inline
 * copy so `allow-deny-bench.ts` still runs standalone exactly as documented.
 */
import * as fs from "fs";
import * as path from "path";
import { Parser, Store, Quad } from "n3";

export const REPORT = "https://w3id.org/force/compliance-report#";
export const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
export const DATA = path.join(__dirname, "..", "data");

export type Decision = "Allow" | "Deny";

export interface Case {
    id: string;
    slug: string;
    title: string;
    policy: string;
    request: string;
    sotw: string;
    expected: string;
    seq: number;
}

export function parseFile(p: string): Quad[] {
    return new Parser({ baseIRI: "http://example.org/" }).parse(fs.readFileSync(p, "utf-8"));
}

/** index.rs: rewrite ".../data/<rest>" -> "<data>/<rest>". */
export function rewriteToLocal(url: string): string {
    const i = url.indexOf("/data/");
    if (i === -1) throw new Error(`source URL has no /data/: ${url}`);
    return path.join(DATA, url.slice(i + "/data/".length));
}

/** ground_truth.rs::expected_decision, on an arbitrary set of report quads. */
export function reduceToDecision(quads: Quad[]): Decision {
    const store = new Store(quads);
    let prohibitionActive = false;
    let permissionActive = false;
    for (const q of store.getQuads(null, REPORT + "activationState", null, null)) {
        if (q.object.value !== REPORT + "Active") continue;
        const types = store.getObjects(q.subject, RDF_TYPE, null);
        const t = types.length > 0 ? types[0].value : undefined; // object_node(): first match
        if (t === REPORT + "ProhibitionReport") prohibitionActive = true;
        else if (t === REPORT + "PermissionReport") permissionActive = true;
    }
    if (prohibitionActive) return "Deny";
    if (permissionActive) return "Allow";
    return "Deny";
}

/** Ground truth for one case: the fixture's own expected report, reduced. */
export function reduceToDecisionOfFile(p: string): Decision {
    return reduceToDecision(parseFile(p));
}

/** The same 68 cases, in the same order, allow-deny-bench.ts evaluates. */
export function loadCases(): Case[] {
    const index = new Store(parseFile(path.join(DATA, "index.ttl")));
    const ids = index.getSubjects(null, null, null)
        .map((s) => s.value)
        .filter((v, i, a) => a.indexOf(v) === i); // dedupe (suite tsconfig targets pre-ES2015: no Set spread)
    const entries: Case[] = ids.map((id) => {
        const one = (p: string) => index.getObjects(id, p, null)[0].value;
        const expectedUrl = one("http://example.org/expectedReportSource");
        const slug = path.basename(expectedUrl, ".ttl");
        return {
            id,
            slug,
            title: index.getObjects(id, "http://purl.org/dc/terms/title", null)[0].value,
            policy: rewriteToLocal(one("http://example.org/policySource")),
            request: rewriteToLocal(one("http://example.org/requestSource")),
            sotw: rewriteToLocal(one("http://example.org/sotwSource")),
            expected: rewriteToLocal(expectedUrl),
            seq: parseInt(slug.split("-")[1], 10),
        };
    });
    entries.sort((a, b) => a.seq - b.seq);
    return entries;
}

/**
 * Read every case's three input files ONCE into memory as parsed quads.
 * evaluate() mutates nothing on its inputs, but n3 Quad arrays are shared
 * objects, so each evaluation gets a fresh shallow copy of the array.
 *
 * Pre-parsing is deliberate: it takes Turtle parsing (which is not the engine)
 * out of the timed window, so the reported latency is the reasoner's own cost.
 * perf-bench.ts also measures the with-parsing path so both numbers exist.
 */
export interface LoadedCase extends Case {
    policyQuads: Quad[];
    requestQuads: Quad[];
    sotwQuads: Quad[];
    expectedDecision: Decision;
}

export function preload(cases: Case[]): LoadedCase[] {
    return cases.map((c) => ({
        ...c,
        policyQuads: parseFile(c.policy),
        requestQuads: parseFile(c.request),
        sotwQuads: parseFile(c.sotw),
        expectedDecision: reduceToDecision(parseFile(c.expected)),
    }));
}

// ---------------------------------------------------------------- statistics

export interface Stats {
    n: number;
    mean: number;
    median: number;
    p95: number;
    p99: number;
    min: number;
    max: number;
    q1: number;
    q3: number;
    iqr: number;
    stddev: number;
}

/** Nearest-rank percentile on an already-sorted ascending array. */
function pct(sorted: number[], p: number): number {
    if (sorted.length === 0) return NaN;
    const rank = Math.ceil((p / 100) * sorted.length);
    return sorted[Math.min(Math.max(rank, 1), sorted.length) - 1];
}

function median(sorted: number[]): number {
    if (sorted.length === 0) return NaN;
    const m = sorted.length >> 1;
    return sorted.length % 2 ? sorted[m] : (sorted[m - 1] + sorted[m]) / 2;
}

export function stats(xs: number[]): Stats {
    const s = xs.slice().sort((a, b) => a - b);
    const mean = s.reduce((a, b) => a + b, 0) / (s.length || 1);
    const variance = s.reduce((a, b) => a + (b - mean) * (b - mean), 0) / (s.length || 1);
    const q1 = median(s.slice(0, s.length >> 1));
    const q3 = median(s.slice((s.length + 1) >> 1));
    return {
        n: s.length,
        mean,
        median: median(s),
        p95: pct(s, 95),
        p99: pct(s, 99),
        min: s[0],
        max: s[s.length - 1],
        q1,
        q3,
        iqr: q3 - q1,
        stddev: Math.sqrt(variance),
    };
}

export const round3 = (x: number) => (Number.isFinite(x) ? Math.round(x * 1000) / 1000 : x);
