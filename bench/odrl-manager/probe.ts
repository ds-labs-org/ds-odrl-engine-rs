/** Targeted vocabulary probes against odrl-manager develop HEAD. */
import { PolicyInstanciator } from 'PolicyInstanciator';
import { PolicyEvaluator } from 'PolicyEvaluator';
import { PolicyDataFetcher, Custom } from 'PolicyDataFetcher';
import { PolicyStateFetcher } from 'PolicyStateFetcher';
import { EntityRegistry } from 'EntityRegistry';

const NOW = '2024-02-12T11:20:10.999Z';

class Fetcher extends PolicyDataFetcher {
  @Custom()
  protected async getDateTime(): Promise<Date> {
    return new Date(NOW);
  }
  @Custom()
  protected async getPayAmount(): Promise<number> {
    return 5;
  }
  @Custom()
  protected async getPurpose(): Promise<string> {
    return 'research';
  }
  @Custom()
  protected async getIndustry(): Promise<string> {
    return 'health';
  }
}

class State extends PolicyStateFetcher {
  protected async getCompensate(): Promise<boolean> {
    return State.compensated;
  }
  static compensated = false;
}

async function run(label: string, json: any, action: string, target: string) {
  EntityRegistry.cleanReferences();
  const inst = new PolicyInstanciator();
  const policy = inst.genPolicyFrom(json);
  const ev = new PolicyEvaluator();
  if (!policy) {
    console.log(`${label.padEnd(64)} -> POLICY DID NOT PARSE (null)`);
    return;
  }
  ev.setPolicy(policy, new Fetcher(), new State());
  const r = await ev.isActionPerformable(action as any, target);
  console.log(`${label.padEnd(64)} -> ${r}`);
}

const base = (rules: any) => ({
  '@context': 'http://www.w3.org/ns/odrl/2/',
  '@type': 'Set',
  uid: 'urn:probe',
  ...rules,
});

(async () => {
  const T = 'http://example.org/x';

  console.log('\n--- A. action matching ---');
  await run('P1 permission action=read, request read', base({ permission: [{ target: T, action: 'read' }] }), 'read', T);
  await run('P2 permission action=use, request read', base({ permission: [{ target: T, action: 'use' }] }), 'read', T);
  await run('P3 permission action=use, request use', base({ permission: [{ target: T, action: 'use' }] }), 'use', T);
  await run('P4 permission action=use, request sell', base({ permission: [{ target: T, action: 'use' }] }), 'sell', T);
  await run('P5 permission action=transfer, request sell', base({ permission: [{ target: T, action: 'transfer' }] }), 'sell', T);
  await run('P6 permission action=write, request write', base({ permission: [{ target: T, action: 'write' }] }), 'write', T);

  console.log('\n--- B. prohibition / no-rule defaults ---');
  await run('P7 prohibition action=use (no constraint), request read', base({ prohibition: [{ target: T, action: 'use' }] }), 'read', T);
  await run('P8 empty policy (no rules), request read', base({}), 'read', T);
  await run('P9 permission on other target, request read on x', base({ permission: [{ target: 'http://example.org/y', action: 'use' }] }), 'read', T);
  await run(
    'P10 permission use + prohibition use, request read',
    base({ permission: [{ target: T, action: 'use' }], prohibition: [{ target: T, action: 'use' }] }),
    'read',
    T,
  );

  console.log('\n--- C. numeric comparison ---');
  await run(
    'P11 payAmount gt 3 (actual 5)',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'payAmount', operator: 'gt', rightOperand: 3 }] }] }),
    'read',
    T,
  );
  await run(
    'P12 payAmount gt 3 (rightOperand as STRING "3")',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'payAmount', operator: 'gt', rightOperand: '3' }] }] }),
    'read',
    T,
  );
  await run(
    'P13 payAmount lt 3 (actual 5)',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'payAmount', operator: 'lt', rightOperand: 3 }] }] }),
    'read',
    T,
  );
  await run(
    'P14 payAmount XSD-typed {"@value":"3","@type":"xsd:integer"}',
    base({
      permission: [
        { target: T, action: 'use', constraint: [{ leftOperand: 'payAmount', operator: 'gt', rightOperand: { '@value': '3', '@type': 'xsd:integer' } }] },
      ],
    }),
    'read',
    T,
  );

  console.log('\n--- D. dateTime comparison ---');
  for (const [op, rhs] of [
    ['eq', NOW],
    ['neq', NOW],
    ['lt', '2024-12-31T23:59:59Z'],
    ['gt', '2024-01-01T00:00:00Z'],
    ['lteq', NOW],
    ['gteq', NOW],
  ] as [string, string][]) {
    await run(
      `P dateTime ${op} ${rhs}`,
      base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'dateTime', operator: op, rightOperand: rhs }] }] }),
      'read',
      T,
    );
  }

  console.log('\n--- E. logical constraints ---');
  const and2 = { operator: 'and', constraint: [
    { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2024-01-01T00:00:00Z' },
    { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2024-12-31T23:59:59Z' },
  ] };
  await run("P15 native and-shape {operator:'and',constraint:[...]} (both true)", base({ permission: [{ target: T, action: 'use', constraint: [and2] }] }), 'read', T);
  await run(
    'P16 ODRL 2.2 shape {"and":[...]} (both true)',
    base({ permission: [{ target: T, action: 'use', constraint: [{ and: and2.constraint }] }] }),
    'read',
    T,
  );
  await run(
    'P17 native or-shape, one true one false',
    base({ permission: [{ target: T, action: 'use', constraint: [{ operator: 'or', constraint: [
      { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2030-01-01T00:00:00Z' },
      { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2024-12-31T23:59:59Z' },
    ] }] }] }),
    'read',
    T,
  );
  await run(
    'P18 NESTED: or of two ands (second and true)',
    base({ permission: [{ target: T, action: 'use', constraint: [{ operator: 'or', constraint: [
      { operator: 'and', constraint: [
        { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2030-01-01T00:00:00Z' },
        { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2031-01-01T00:00:00Z' },
      ] },
      { operator: 'and', constraint: [
        { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2024-01-01T00:00:00Z' },
        { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2024-12-31T23:59:59Z' },
      ] },
    ] }] }] }),
    'read',
    T,
  );
  await run(
    'P19 NESTED: or of two ands (both false)',
    base({ permission: [{ target: T, action: 'use', constraint: [{ operator: 'or', constraint: [
      { operator: 'and', constraint: [
        { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2030-01-01T00:00:00Z' },
        { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2031-01-01T00:00:00Z' },
      ] },
      { operator: 'and', constraint: [
        { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2040-01-01T00:00:00Z' },
        { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2041-01-01T00:00:00Z' },
      ] },
    ] }] }] }),
    'read',
    T,
  );
  await run(
    'P20 xone (declared operand, 1 of 2 true)',
    base({ permission: [{ target: T, action: 'use', constraint: [{ operator: 'xone', constraint: [
      { leftOperand: 'dateTime', operator: 'lt', rightOperand: '2024-12-31T23:59:59Z' },
      { leftOperand: 'dateTime', operator: 'gt', rightOperand: '2030-01-01T00:00:00Z' },
    ] }] }] }),
    'read',
    T,
  );

  console.log('\n--- F. set operators ---');
  await run(
    'P21 isAnyOf in a PERMISSION (purpose=research in [research,teaching])',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'purpose', operator: 'isAnyOf', rightOperand: ['research', 'teaching'] }] }] }),
    'read',
    T,
  );
  await run(
    'P22 isAnyOf in a PROHIBITION (purpose=research in [research,teaching])',
    base({ prohibition: [{ target: T, action: 'use', constraint: [{ leftOperand: 'purpose', operator: 'isAnyOf', rightOperand: ['research', 'teaching'] }] }] }),
    'read',
    T,
  );
  await run(
    'P23 isNoneOf in a PROHIBITION, control (purpose=research NOT in [a,b])',
    base({ prohibition: [{ target: T, action: 'use', constraint: [{ leftOperand: 'purpose', operator: 'isNoneOf', rightOperand: ['a', 'b'] }] }] }),
    'read',
    T,
  );
  await run(
    'P24 isAllOf in a PERMISSION',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'purpose', operator: 'isAllOf', rightOperand: ['research'] }] }] }),
    'read',
    T,
  );
  await run(
    'P25 isPartOf in a PERMISSION',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'industry', operator: 'isPartOf', rightOperand: ['health'] }] }] }),
    'read',
    T,
  );
  await run(
    'P26 hasPart in a PERMISSION',
    base({ permission: [{ target: T, action: 'use', constraint: [{ leftOperand: 'industry', operator: 'hasPart', rightOperand: ['health'] }] }] }),
    'read',
    T,
  );

  console.log('\n--- G. duty on a permission (state fetcher = compensate) ---');
  const withDuty = base({ permission: [{ target: T, action: 'use', duty: [{ action: 'compensate' }] }] });
  State.compensated = true;
  await run('P27 permission with duty compensate, state Performed', withDuty, 'read', T);
  State.compensated = false;
  await run('P28 permission with duty compensate, state NOT performed', withDuty, 'read', T);

  console.log('\n--- H. assignee ---');
  await run(
    'P29 permission assignee=alice, request (API has no party arg)',
    base({ permission: [{ target: T, action: 'use', assignee: 'http://example.org/alice' }] }),
    'read',
    T,
  );
  await run(
    'P30 permission assignee=bob, request (API has no party arg)',
    base({ permission: [{ target: T, action: 'use', assignee: 'http://example.org/bob' }] }),
    'read',
    T,
  );

  console.log('\n--- I. policy classes ---');
  for (const t of ['Set', 'Offer', 'Agreement', 'Request', 'Privacy', 'Ticket', 'Assertion', 'Policy']) {
    const json: any = { '@context': 'http://www.w3.org/ns/odrl/2/', '@type': t, uid: 'urn:probe', permission: [{ target: T, action: 'use' }] };
    await run(`P policy @type=${t}`, json, 'read', T);
  }
})();
