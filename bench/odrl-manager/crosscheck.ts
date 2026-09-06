/** Cross-checks this bench's independently re-derived ground truth against
 *  ds-odrl-engine-rs's compliance/reports/latest-cases.json. */
import * as fs from 'fs';
import * as path from 'path';
import { loadCases } from './suite';

// See suite.ts's own comment on ODRL_TEST_SUITE_DATA -- same reasoning:
// this file must live inside a copy of odrl-manager itself, so it can't
// assume a fixed relative distance from ds-odrl-engine-rs's own repo.
const REF =
  process.env.LATEST_CASES_JSON ||
  path.join(__dirname, '..', '..', 'compliance', 'reports', 'latest-cases.json');

const cases = loadCases();
const ref = JSON.parse(fs.readFileSync(REF, 'utf8'));
const refMap = new Map<string, string>(ref.cases.map((c: any) => [c.slug, c.expected_decision]));

let mismatches = 0;
let missing = 0;
for (const c of cases) {
  const r = refMap.get(c.slug);
  if (r === undefined) {
    missing++;
    console.log(`MISSING IN REF  ${c.slug}`);
  } else if (r !== c.expected) {
    mismatches++;
    console.log(`MISMATCH        ${c.slug}: mine=${c.expected} ref=${r}`);
  }
}
for (const slug of refMap.keys()) {
  if (!cases.find((c) => c.slug === slug)) console.log(`MISSING LOCALLY ${slug}`);
}
const allow = cases.filter((c) => c.expected === 'Allow').length;
console.log(
  `cases re-derived locally: ${cases.length} (Allow ${allow}, Deny ${cases.length - allow}) | ` +
    `cases in latest-cases.json: ${refMap.size} | mismatches: ${mismatches} | missing: ${missing}`,
);
