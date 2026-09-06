/**
 * Shared corpus/engine-call module for the performance instrumentation.
 *
 * This is `run.ts`'s fixture reading, constraint/policy translation, host
 * fetchers and engine call lifted VERBATIM into exported functions, with the
 * one change that `run.ts`'s module-level `MODE` const becomes a parameter.
 * `run.ts` itself is untouched and still produces the conformance tally
 * exactly as its own README documents.
 *
 * The point of lifting rather than rewriting: every timed evaluation in
 * `perf_bench.ts` / `load_worker.ts` is the same sequence of library calls
 * `run.ts` puts its PASS/FAIL around, on the same 68 cases, in the same order,
 * with the same translated JSON — so a latency number here is a latency for
 * the thing the 61/68 conformance number actually measured.
 *
 * ---------------------------------------------------------------------------
 * TWO TIMED PATHS, AND WHY BOTH
 * ---------------------------------------------------------------------------
 * odrl-manager's own public surface is JSON-in / boolean-out:
 *     PolicyInstanciator.genPolicyFrom(json)  ->  PolicyEvaluator
 *     evaluator.isActionPerformable(action, target)  ->  Promise<boolean>
 * It never sees Turtle. The RDF parse (n3) and the ODRL-JSON translation are
 * this harness's own adapter, not the engine. So:
 *
 *  - `evaluateOnce(prep)` -- ENGINE-ONLY. Instantiate + evaluate a
 *    pre-translated policy object. This is odrl-manager's own cost and the
 *    primary latency figure.
 *  - `pipelineOnce(c, mode)` -- END-TO-END. Re-reads the three fixture files,
 *    re-parses them with n3, re-translates, then does the same engine call.
 *    This is the whole per-case body of `run.ts`'s loop, i.e. what a host
 *    starting from RDF on disk actually pays.
 *
 * Both are reported. Neither is presented as the other.
 *
 * ---------------------------------------------------------------------------
 * A GLOBAL THE ENGINE KEEPS, AND WHAT IT COSTS CONCURRENCY
 * ---------------------------------------------------------------------------
 * `EntityRegistry` holds its state in `private static` fields
 * (`parentRelations`, `entityReferences`, failed-evaluation list), i.e. one
 * mutable table per Node process, not per policy. `run.ts` calls
 * `EntityRegistry.cleanReferences()` before each `genPolicyFrom` for exactly
 * that reason. Two evaluations interleaved in one process therefore share and
 * clobber that table -- see `perf_bench.ts`'s `asyncProbe`, which measures it
 * rather than assuming it.
 */
import * as fs from 'fs';
import { Store, DataFactory } from 'n3';
import { PolicyInstanciator } from 'PolicyInstanciator';
import { PolicyEvaluator } from 'PolicyEvaluator';
import { PolicyDataFetcher, Custom } from 'PolicyDataFetcher';
import { PolicyStateFetcher } from 'PolicyStateFetcher';
import { EntityRegistry } from 'EntityRegistry';
import { loadCases, loadStore, one, objs, typeOf, ODRL, REPORT, DCT, RDF_TYPE, Case } from './suite';

export type Mode = 'native' | 'assisted';
export type Decision = 'Allow' | 'Deny';

export const local = (u: string) => u.replace(/^.*[/#]/, '');

// ---------------------------------------------------------------------------
// Stated numeric gates. Nothing these catch is discarded; it is reported and
// marked. See the README's "Outlier and stability gates" section.
// ---------------------------------------------------------------------------

/** Rule 1: Tukey fence multiplier on the run's own pooled per-case latencies. */
export const TUKEY_K = 1.5;
/** Rule 2: a case is `unstable` when (max-min)/median over its repeats exceeds this. */
export const CASE_INSTABILITY = 0.25;
/** Rule 3: a concurrency level is `unstable_across_repeats` when its
 *  throughput (max-min)/median across ramp repeats exceeds this. */
export const LEVEL_INSTABILITY = 0.15;

// ---------- fixture reading (verbatim from run.ts) ----------

export interface ReqInfo {
  party?: string;
  action?: string;
  target?: string;
}

export function readRequest(store: Store, requestUri: string): ReqInfo {
  const rule = objs(store, requestUri, ODRL + 'permission')[0];
  if (!rule) throw new Error(`request ${requestUri} has no odrl:permission`);
  return {
    party: one(store, rule, ODRL + 'assignee'),
    action: one(store, rule, ODRL + 'action'),
    target: one(store, rule, ODRL + 'target'),
  };
}

export function sotwCurrentTime(store: Store): string | undefined {
  const q = store.getQuads(null, DataFactory.namedNode(DCT + 'issued'), null, null)[0];
  return q?.object.value;
}

export function sotwDutyState(store: Store): { performance?: string; deontic?: string } {
  for (const q of store.getQuads(null, DataFactory.namedNode(RDF_TYPE), DataFactory.namedNode(REPORT + 'DutyReport'), null)) {
    return {
      performance: one(store, q.subject.value, REPORT + 'performanceState'),
      deontic: one(store, q.subject.value, REPORT + 'deonticState'),
    };
  }
  return {};
}

const isMemberOf = (sotw: Store, member: string, collection: string) =>
  objs(sotw, member, ODRL + 'partOf').includes(collection);

// ---------- constraint translation (verbatim from run.ts) ----------

export function translateConstraint(policy: Store, node: string): any {
  for (const operand of ['and', 'or', 'xone', 'andSequence']) {
    const children = objs(policy, node, ODRL + operand);
    if (children.length) {
      return { operator: operand, constraint: children.map((c) => translateConstraint(policy, c)) };
    }
  }
  const left = one(policy, node, ODRL + 'leftOperand');
  const op = one(policy, node, ODRL + 'operator');
  const rightQ = policy.getQuads(DataFactory.namedNode(node), DataFactory.namedNode(ODRL + 'rightOperand'), null, null)[0];
  if (left && op && rightQ) {
    return { leftOperand: local(left), operator: local(op), rightOperand: rightQ.object.value };
  }
  throw new Error(`unrecognised constraint node ${node}`);
}

// ---------- policy translation (verbatim from run.ts, MODE now a parameter) ----------

export interface Translation {
  json: any;
  skipped?: string;
}

export function translate(c: Case, policy: Store, sotw: Store, req: ReqInfo, mode: Mode): Translation {
  const kind = local(typeOf(policy, c.policyUri) ?? '');
  if (!['Set', 'Offer', 'Agreement'].includes(kind)) {
    return { json: null, skipped: `policy class odrl:${kind} is not instanciable (selectPolicyType throws)` };
  }
  const json: any = {
    '@context': 'http://www.w3.org/ns/odrl/2/',
    '@type': kind,
    uid: c.policyUri,
    permission: [],
    prohibition: [],
    obligation: [],
  };

  const kinds: [string, string][] = [
    [ODRL + 'permission', 'permission'],
    [ODRL + 'prohibition', 'prohibition'],
    [ODRL + 'obligation', 'obligation'],
  ];

  for (const [pred, key] of kinds) {
    for (const ruleNode of objs(policy, c.policyUri, pred)) {
      const assignee = one(policy, ruleNode, ODRL + 'assignee');
      let target = one(policy, ruleNode, ODRL + 'target');
      const action = one(policy, ruleNode, ODRL + 'action');

      if (mode === 'assisted') {
        if (assignee && req.party && assignee !== req.party && !isMemberOf(sotw, req.party, assignee)) continue;
        if (target && req.target && target !== req.target && isMemberOf(sotw, req.target, target)) {
          target = req.target;
        }
      }

      const rule: any = {
        target: target ?? req.target,
        action: action ? local(action) : req.action ? local(req.action) : undefined,
      };
      if (assignee) rule.assignee = assignee;

      const constraints = objs(policy, ruleNode, ODRL + 'constraint');
      if (constraints.length) rule.constraint = constraints.map((n) => translateConstraint(policy, n));

      const duties = objs(policy, ruleNode, ODRL + 'duty');
      if (duties.length) {
        rule.duty = duties.map((d) => {
          const da = one(policy, d, ODRL + 'action');
          const duty: any = { action: da ? local(da) : undefined };
          const dc = objs(policy, d, ODRL + 'constraint');
          if (dc.length) duty.constraint = dc.map((n) => translateConstraint(policy, n));
          return duty;
        });
      }
      json[key].push(rule);
    }
  }
  return { json };
}

// ---------- host context (verbatim from run.ts) ----------

export function makeFetchers(now: string, dutyPerformed: boolean) {
  class Fetcher extends PolicyDataFetcher {
    @Custom()
    protected async getDateTime(): Promise<Date> {
      return new Date(now);
    }
  }
  class State extends PolicyStateFetcher {
    protected async getCompensate(): Promise<boolean> {
      return dutyPerformed;
    }
  }
  return { fetcher: new Fetcher(), state: new State() };
}

// ---------- the two timed paths ----------

/** One fixture, fully read and translated: everything the engine call needs. */
export interface Prepared {
  slug: string;
  expected: Decision;
  json: any;
  skipped?: string;
  now: string;
  dutyPerformed: boolean;
  action: string;
  target: string;
}

function prepareOne(c: Case, mode: Mode): Prepared {
  const policy = loadStore([c.policyFile]);
  const sotw = loadStore([c.sotwFile]);
  const request = loadStore([c.requestFile]);
  const req = readRequest(request, c.requestUri);
  const now = sotwCurrentTime(sotw)!;
  const duty = sotwDutyState(sotw);
  const dutyPerformed =
    mode === 'assisted'
      ? duty.deontic === undefined || local(duty.deontic) !== 'Violated'
      : local(duty.performance ?? '') === 'Performed';
  let t: Translation;
  try {
    t = translate(c, policy, sotw, req, mode);
  } catch (e: any) {
    t = { json: null, skipped: e.message };
  }
  return {
    slug: c.slug,
    expected: c.expected,
    json: t.json,
    skipped: t.skipped,
    now,
    dutyPerformed,
    action: local(req.action ?? 'use'),
    target: req.target ?? '',
  };
}

/** Read + translate the whole corpus once, so the engine path can be timed alone. */
export function prepareCorpus(mode: Mode): Prepared[] {
  return loadCases().map((c) => prepareOne(c, mode));
}

export const listCases = (): Case[] => loadCases();

/**
 * ENGINE-ONLY timed unit: odrl-manager instantiating and evaluating one
 * already-translated policy. Identical call sequence to run.ts's loop body
 * after `translate` returns.
 */
export async function evaluateOnce(p: Prepared): Promise<Decision> {
  if (p.skipped) return 'Deny';
  EntityRegistry.cleanReferences();
  const inst = new PolicyInstanciator();
  const policy = inst.genPolicyFrom(p.json);
  if (!policy) return 'Deny';
  const ev = new PolicyEvaluator();
  const { fetcher, state } = makeFetchers(p.now, p.dutyPerformed);
  ev.setPolicy(policy, fetcher, state);
  const ok = await ev.isActionPerformable(p.action as any, p.target);
  return ok ? 'Allow' : 'Deny';
}

/** END-TO-END timed unit: run.ts's entire per-case body, RDF parse included. */
export async function pipelineOnce(c: Case, mode: Mode): Promise<Decision> {
  return evaluateOnce(prepareOne(c, mode));
}

// ---------- stats ----------

export function percentile(sorted: number[], q: number): number {
  if (!sorted.length) return NaN;
  if (sorted.length === 1) return sorted[0];
  const pos = (sorted.length - 1) * q;
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  return lo === hi ? sorted[lo] : sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo);
}

export interface Stats {
  n: number;
  mean: number;
  median: number;
  p95: number;
  p99: number;
  min: number;
  max: number;
  stdev: number;
  q1: number;
  q3: number;
  iqr: number;
}

export function stats(values: number[]): Stats {
  const s = [...values].sort((a, b) => a - b);
  const n = s.length;
  const mean = s.reduce((a, b) => a + b, 0) / n;
  const q1 = percentile(s, 0.25);
  const q3 = percentile(s, 0.75);
  const variance = s.reduce((a, b) => a + (b - mean) ** 2, 0) / n;
  return {
    n,
    mean: r3(mean),
    median: r3(percentile(s, 0.5)),
    p95: r3(percentile(s, 0.95)),
    p99: r3(percentile(s, 0.99)),
    min: r3(s[0]),
    max: r3(s[n - 1]),
    stdev: r3(Math.sqrt(variance)),
    q1: r3(q1),
    q3: r3(q3),
    iqr: r3(q3 - q1),
  };
}

export const r3 = (x: number) => Math.round(x * 1000) / 1000;

/** Rule-1 Tukey fence over a pooled sample. */
export function tukeyFence(values: number[]): { lo: number; hi: number } {
  const s = stats(values);
  return { lo: r3(s.q1 - TUKEY_K * s.iqr), hi: r3(s.q3 + TUKEY_K * s.iqr) };
}

// ---------- /proc readers (kernel-reported, not self-reported) ----------

export function vmRssKb(pid: number | 'self' = 'self'): number {
  try {
    const m = /VmRSS:\s+(\d+) kB/.exec(fs.readFileSync(`/proc/${pid}/status`, 'utf8'));
    return m ? parseInt(m[1], 10) : -1;
  } catch {
    return -1;
  }
}

/** (utime + stime) of a pid, in seconds. */
export function cpuSeconds(pid: number | 'self' = 'self'): number {
  try {
    const raw = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
    const fields = raw.slice(raw.lastIndexOf(')') + 2).split(' ');
    const utime = parseInt(fields[11], 10);
    const stime = parseInt(fields[12], 10);
    const hz = 100; // USER_HZ on this kernel
    return (utime + stime) / hz;
  } catch {
    return -1;
  }
}

export function memAvailableKb(): number {
  const m = /MemAvailable:\s+(\d+) kB/.exec(fs.readFileSync('/proc/meminfo', 'utf8'));
  return m ? parseInt(m[1], 10) : -1;
}

export const nowMs = () => Number(process.hrtime.bigint()) / 1e6;
