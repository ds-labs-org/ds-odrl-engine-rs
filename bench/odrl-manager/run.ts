/**
 * Benches odrl-manager (develop HEAD) against the vendored
 * SolidLabResearch/ODRL-Test-Suite corpus.
 *
 * ---------------------------------------------------------------------------
 * ADAPTER CONVENTIONS (stated once, here)
 * ---------------------------------------------------------------------------
 * odrl-manager's decision surface is
 *     evaluator.isActionPerformable(actionType: string, target: string)
 * i.e. "given the loaded policy, may <action> be done to <target>?". It has no
 * requesting-party parameter, no state-of-the-world model, and no compliance
 * report -- it returns one boolean. Everything below is what it takes to put
 * one fixture triple (policy, request, sotw) in front of that call.
 *
 * MODE "native" -- structural translation only, no host-side decisions:
 *  - odrl:Set -> {'@context','@type':'Set',uid,permission[],prohibition[],obligation[]}
 *  - each rule keeps its own odrl:action (local name). A rule with NO action
 *    (policy-1 "everybody can do everything", policy-2 "nobody can do
 *    anything") gets the request's own action -- the same stand-in the
 *    ds-odrl-engine-rs adapter uses, because "any action" has no vocabulary
 *    term here either.
 *  - each rule keeps its own odrl:target. A rule with NO target gets the
 *    request's own target, for the same reason (odrl-manager only ever finds a
 *    rule by matching an Asset uid, so a rule with no Asset is unreachable).
 *  - odrl:assignee is emitted into the JSON verbatim. odrl-manager parses it
 *    into a Party and then never consults it (Rule.evaluate() looks only at
 *    constraints/duties) -- verified by probe P29/P30.
 *  - odrl:constraint: an atomic constraint becomes
 *    {leftOperand, operator, rightOperand}; an odrl:and / odrl:or logical
 *    constraint becomes odrl-manager's OWN logical shape
 *    {operator:'and'|'or', constraint:[...]}, recursively. ODRL 2.2's actual
 *    serialization {"and":[...]} is not accepted by this library (probe P16),
 *    so emitting it would only measure the parser, not the evaluator.
 *  - the SOTW's temp:currentTime dct:issued value is injected as the
 *    @Custom() getDateTime() of a PolicyDataFetcher subclass -- this is the
 *    library's own documented host-context extension point.
 *  - the SOTW's report:DutyReport report:performanceState is injected as the
 *    getCompensate() of a PolicyStateFetcher subclass: Performed -> true,
 *    anything else -> false. Again the library's own extension point.
 *
 * MODE "assisted" -- everything in "native", plus three host-side
 * pre-decisions that odrl-manager's API genuinely cannot express. Each is
 * decided by the adapter at translation time, not by the library:
 *  1. assignee scoping: a rule whose odrl:assignee is neither the request's
 *     party nor a collection the request's party is `odrl:partOf` in the SOTW
 *     is dropped from the translated policy.
 *  2. asset-collection targets: a rule whose odrl:target is an
 *     odrl:AssetCollection the request's target is `odrl:partOf` in the SOTW
 *     has its target rewritten to the request's target.
 *  3. duty deontic state: report:NonSet ("unknown") is treated as
 *     not-violated (getCompensate -> true), matching this corpus's own
 *     expected reports; only report:Violated leaves it false.
 * ---------------------------------------------------------------------------
 */
import * as fs from 'fs';
import { Store, DataFactory } from 'n3';
import { PolicyInstanciator } from 'PolicyInstanciator';
import { PolicyEvaluator } from 'PolicyEvaluator';
import { PolicyDataFetcher, Custom } from 'PolicyDataFetcher';
import { PolicyStateFetcher } from 'PolicyStateFetcher';
import { EntityRegistry } from 'EntityRegistry';
import { Action } from 'models/odrl/Action';
import { loadCases, loadStore, one, objs, typeOf, ODRL, EX, REPORT, DCT, RDF_TYPE, Case } from './suite';

const MODE = (process.argv[2] || 'native') as 'native' | 'assisted';
const VERBOSE = process.argv.includes('--verbose');

const local = (u: string) => u.replace(/^.*[/#]/, '');

// ---------- fixture reading ----------

interface ReqInfo {
  party?: string;
  action?: string;
  target?: string;
}

function readRequest(store: Store, requestUri: string): ReqInfo {
  const rule = objs(store, requestUri, ODRL + 'permission')[0];
  if (!rule) throw new Error(`request ${requestUri} has no odrl:permission`);
  return {
    party: one(store, rule, ODRL + 'assignee'),
    action: one(store, rule, ODRL + 'action'),
    target: one(store, rule, ODRL + 'target'),
  };
}

function sotwCurrentTime(store: Store): string | undefined {
  const q = store.getQuads(null, DataFactory.namedNode(DCT + 'issued'), null, null)[0];
  return q?.object.value;
}

/** report:performanceState / report:deonticState of the SOTW's DutyReport, if any. */
function sotwDutyState(store: Store): { performance?: string; deontic?: string } {
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

// ---------- constraint translation ----------

function translateConstraint(policy: Store, node: string): any {
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

// ---------- policy translation ----------

interface Translation {
  json: any;
  skipped?: string;
}

function translate(c: Case, policy: Store, sotw: Store, req: ReqInfo): Translation {
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

      if (MODE === 'assisted') {
        // 1. assignee scoping (individual or PartyCollection membership)
        if (assignee && req.party && assignee !== req.party && !isMemberOf(sotw, req.party, assignee)) continue;
        // 2. asset-collection target rewriting
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

// ---------- host context ----------

function makeFetchers(now: string, dutyPerformed: boolean) {
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

// ---------- main ----------

(async () => {
  const cases = loadCases();
  let pass = 0,
    fail = 0,
    skip = 0;
  const rows: string[] = [];
  const failures: string[] = [];
  const skips: string[] = [];
  const unreached: string[] = [];
  // odrl-manager's own hardcoded taxonomy, read back from the library itself.
  new PolicyInstanciator();
  const coveredBy = (ruleAction: string, requested: string): boolean =>
    ruleAction !== undefined && (Action as any).inclusions?.get(ruleAction)?.has(requested) === true;

  for (const c of cases) {
    const policy = loadStore([c.policyFile]);
    const sotw = loadStore([c.sotwFile]);
    const request = loadStore([c.requestFile]);
    const req = readRequest(request, c.requestUri);
    const now = sotwCurrentTime(sotw)!;
    const duty = sotwDutyState(sotw);
    const dutyPerformed =
      MODE === 'assisted'
        ? duty.deontic === undefined || local(duty.deontic) !== 'Violated'
        : local(duty.performance ?? '') === 'Performed';

    let t: Translation;
    try {
      t = translate(c, policy, sotw, req);
    } catch (e: any) {
      t = { json: null, skipped: e.message };
    }
    if (t.skipped) {
      skip++;
      skips.push(`${c.slug}: ${t.skipped}`);
      rows.push(`SKIP  ${c.slug.padEnd(34)} expected=${c.expected}`);
      continue;
    }

    EntityRegistry.cleanReferences();
    const inst = new PolicyInstanciator();
    const p = inst.genPolicyFrom(t.json);
    let got: 'Allow' | 'Deny';
    if (!p) {
      got = 'Deny';
    } else {
      const ev = new PolicyEvaluator();
      const { fetcher, state } = makeFetchers(now, dutyPerformed);
      ev.setPolicy(p, fetcher, state);
      const ok = await ev.isActionPerformable(local(req.action ?? 'use') as any, req.target ?? '');
      got = ok ? 'Allow' : 'Deny';
    }
    // Diagnostic: did any translated rule actually reach evaluation, or did
    // isActionPerformable find nothing and fall through to its closed default?
    const reqAction = local(req.action ?? 'use');
    const covering = (['permission', 'prohibition'] as const).flatMap((k) =>
      (t.json?.[k] ?? []).filter(
        (r: any) => r.target === req.target && (r.action === reqAction || coveredBy(r.action, reqAction)),
      ),
    );
    const reason = covering.length === 0 ? 'no-rule-reached-evaluation(closed default)' : `${covering.length} rule(s) evaluated`;

    const ok = got === c.expected;
    if (ok) pass++;
    else {
      fail++;
      failures.push(`${c.slug.padEnd(34)} expected=${c.expected} got=${got}   ${c.title}`);
    }
    if (covering.length === 0) unreached.push(`${c.slug.padEnd(34)} expected=${c.expected} got=${got}`);
    rows.push(`${ok ? 'PASS' : 'FAIL'}  ${c.slug.padEnd(34)} expected=${c.expected} got=${got}  [${reason}]`);
    if (VERBOSE) rows.push('      ' + JSON.stringify(t.json));
  }

  console.log(`\n===== odrl-manager @develop vs SolidLabResearch/ODRL-Test-Suite (mode: ${MODE}) =====`);
  rows.forEach((r) => console.log(r));
  console.log(`\ntotal ${cases.length} | passed ${pass} | failed ${fail} | skipped ${skip}`);
  if (failures.length) {
    console.log('\n--- failing cases ---');
    failures.forEach((f) => console.log('  ' + f));
  }
  if (skips.length) {
    console.log('\n--- skipped cases ---');
    skips.forEach((s) => console.log('  ' + s));
  }
  console.log(
    `\n--- cases where NO translated rule reached evaluation (verdict is isActionPerformable's closed default, not a decision): ${unreached.length} ---`,
  );
  unreached.forEach((u) => console.log('  ' + u));
  fs.writeFileSync(
    process.env.OUT || `odrl-manager-bench-${MODE}.txt`,
    rows.join('\n') + `\n\ntotal ${cases.length} | passed ${pass} | failed ${fail} | skipped ${skip}\n`,
  );
})();
