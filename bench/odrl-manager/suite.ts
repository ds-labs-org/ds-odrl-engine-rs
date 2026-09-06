/**
 * Independent re-derivation of the SolidLabResearch/ODRL-Test-Suite corpus:
 * loads every fixture (test case, policy, request, state-of-the-world) as RDF
 * and derives the expected Allow/Deny verdict from the fixture's own
 * report:* expected report, with no reference to any other engine's output.
 *
 * Reduction rule (re-derived here from the fixtures themselves, and stated to
 * match ds-odrl-engine-rs/compliance-runner/src/ground_truth.rs):
 *   any report:ProhibitionReport with report:activationState report:Active
 *     -> Deny  (deny-overrides)
 *   else any report:PermissionReport with report:activationState report:Active
 *     -> Allow
 *   else -> Deny  (closed-world default)
 * report:DutyReport rule-reports are not part of the reduction.
 */
import * as fs from 'fs';
import * as path from 'path';
import { Parser, Store, DataFactory } from 'n3';

const ODRL = 'http://www.w3.org/ns/odrl/2/';
const EX = 'http://example.org/';
const REPORT = 'https://w3id.org/force/compliance-report#';
const DCT = 'http://purl.org/dc/terms/';
const RDF_TYPE = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type';

// odrl-manager's own module resolution (tsconfig `baseUrl: "./src"`) means
// this file has to physically live inside a checkout of odrl-manager itself
// (src/bench/) to import PolicyEvaluator etc. at all -- so it can never sit
// at a fixed relative distance from ds-odrl-engine-rs's own vendored corpus
// the way a script that stays put could. ODRL_TEST_SUITE_DATA lets a
// reproducer point this at wherever they put the corpus (a checkout of
// SolidLabResearch/ODRL-Test-Suite, or ds-odrl-engine-rs's own
// compliance/vendor/odrl-test-suite/data); the fallback assumes the common
// case of running this straight from inside a ds-odrl-engine-rs checkout's
// own bench/odrl-manager/ (its original, non-copied location).
export const SUITE_ROOT =
  process.env.ODRL_TEST_SUITE_DATA ||
  path.join(__dirname, '..', '..', 'compliance', 'vendor', 'odrl-test-suite', 'data');

export function loadStore(files: string[]): Store {
  const store = new Store();
  const parser = new Parser({ format: 'text/turtle' });
  for (const f of files) {
    store.addQuads(parser.parse(fs.readFileSync(f, 'utf8')));
  }
  return store;
}

export function objs(store: Store, s: string, p: string): string[] {
  return store
    .getQuads(DataFactory.namedNode(s), DataFactory.namedNode(p), null, null)
    .map((q) => q.object.value);
}

export function objTerms(store: Store, s: string, p: string) {
  return store.getQuads(DataFactory.namedNode(s), DataFactory.namedNode(p), null, null).map((q) => q.object);
}

export function one(store: Store, s: string, p: string): string | undefined {
  return objs(store, s, p)[0];
}

export function typeOf(store: Store, s: string): string | undefined {
  return one(store, s, RDF_TYPE);
}

/** All files of one kind, indexed by every subject URI they define. */
function indexBySubject(dir: string): Map<string, string> {
  const idx = new Map<string, string>();
  for (const name of fs.readdirSync(dir)) {
    if (!name.endsWith('.ttl')) continue;
    const file = path.join(dir, name);
    const store = loadStore([file]);
    for (const q of store.getQuads(null, null, null, null)) {
      if (q.subject.termType === 'NamedNode') idx.set(q.subject.value, file);
    }
  }
  return idx;
}

export interface Case {
  slug: string;
  title: string;
  caseFile: string;
  policyFile: string;
  requestFile: string;
  sotwFile: string;
  policyUri: string;
  requestUri: string;
  sotwUri: string;
  expected: 'Allow' | 'Deny';
}

export function expectedDecision(store: Store): 'Allow' | 'Deny' {
  let prohibitionActive = false;
  let permissionActive = false;
  for (const q of store.getQuads(null, DataFactory.namedNode(REPORT + 'activationState'), null, null)) {
    const isActive = q.object.value === REPORT + 'Active';
    if (!isActive) continue;
    const t = typeOf(store, q.subject.value);
    if (t === REPORT + 'ProhibitionReport') prohibitionActive = true;
    else if (t === REPORT + 'PermissionReport') permissionActive = true;
  }
  if (prohibitionActive) return 'Deny';
  if (permissionActive) return 'Allow';
  return 'Deny';
}

export function loadCases(): Case[] {
  const policyIdx = indexBySubject(path.join(SUITE_ROOT, 'policies'));
  const requestIdx = indexBySubject(path.join(SUITE_ROOT, 'requests'));
  const sotwIdx = indexBySubject(path.join(SUITE_ROOT, 'sotw'));
  const caseDir = path.join(SUITE_ROOT, 'test_cases');
  const cases: Case[] = [];
  for (const name of fs.readdirSync(caseDir).sort()) {
    if (!name.endsWith('.ttl')) continue;
    const caseFile = path.join(caseDir, name);
    const store = loadStore([caseFile]);
    const tc = store
      .getQuads(null, DataFactory.namedNode(RDF_TYPE), DataFactory.namedNode(EX + 'TestCase'), null)
      .map((q) => q.subject.value)[0];
    if (!tc) throw new Error(`no ex:TestCase in ${name}`);
    const policyUri = one(store, tc, EX + 'policy')!;
    const requestUri = one(store, tc, EX + 'request')!;
    const sotwUri = one(store, tc, EX + 'sotw')!;
    cases.push({
      slug: name.replace(/\.ttl$/, ''),
      title: one(store, tc, DCT + 'title') ?? '',
      caseFile,
      policyFile: policyIdx.get(policyUri)!,
      requestFile: requestIdx.get(requestUri)!,
      sotwFile: sotwIdx.get(sotwUri)!,
      policyUri,
      requestUri,
      sotwUri,
      expected: expectedDecision(store),
    });
  }
  return cases;
}

export { ODRL, EX, REPORT, DCT, RDF_TYPE };
