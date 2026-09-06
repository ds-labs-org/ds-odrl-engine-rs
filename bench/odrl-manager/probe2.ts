/** Focused probe: odrl-manager's built-in action taxonomy, exactly as it
 *  affects testcase-009 (a `use` rule against a `write` request). */
import { PolicyInstanciator } from 'PolicyInstanciator';
import { PolicyEvaluator } from 'PolicyEvaluator';
import { EntityRegistry } from 'EntityRegistry';
import { Action } from 'models/odrl/Action';

const T = 'http://example.org/y';

async function run(ruleAction: string, requested: string) {
  EntityRegistry.cleanReferences();
  const inst = new PolicyInstanciator();
  const p = inst.genPolicyFrom({
    '@context': 'http://www.w3.org/ns/odrl/2/',
    '@type': 'Set',
    uid: 'urn:probe',
    permission: [{ target: T, action: ruleAction }],
  });
  const ev = new PolicyEvaluator();
  ev.setPolicy(p!);
  const r = await ev.isActionPerformable(requested as any, T);
  console.log(`  rule action=${ruleAction.padEnd(9)} request=${requested.padEnd(9)} -> ${r}`);
}

(async () => {
  console.log('isActionPerformable, action coverage:');
  for (const [ra, rq] of [
    ['use', 'read'],
    ['use', 'write'],
    ['use', 'distribute'],
    ['use', 'use'],
    ['use', 'sell'],
    ['transfer', 'sell'],
    ['transfer', 'give'],
    ['read', 'read'],
    ['write', 'write'],
  ] as [string, string][]) {
    await run(ra, rq);
  }
  new PolicyInstanciator(); // ensure the static inclusion map is populated
  const inc = await Action.getIncluded(['use'] as any);
  console.log(`\nAction.getIncluded(['use']).length = ${inc.length}`);
  console.log(`  includes 'read'?       ${inc.includes('read' as any)}`);
  console.log(`  includes 'write'?      ${inc.includes('write' as any)}`);
  console.log(`  includes 'distribute'? ${inc.includes('distribute' as any)}`);
  console.log(`  includes 'display'?    ${inc.includes('display' as any)} (ODRL: display includedIn play includedIn use)`);
})();
