/**
 * Rough vocabulary spot-check (NOT a coverage audit): 6 hand-written
 * ODRL 2.2 probes fed straight to the SolidLab evaluator, each reduced to
 * Allow/Deny by the same ground_truth.rs rule the fixture bench uses.
 * The request and sotw are the vendored fixtures' own shapes (request-1 /
 * temporal sotw), only the policy varies, except where a probe needs a
 * request-side sotw:context left-operand value.
 */
import { Parser, Store, Quad } from "n3";
import { ODRLEngineMultipleSteps, ODRLEvaluator } from "odrl-evaluator";

const REPORT = "https://w3id.org/force/compliance-report#";
const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PFX = `@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
@prefix temp: <http://example.com/request/>.
@prefix dct: <http://purl.org/dc/terms/>.
@prefix xsd: <http://www.w3.org/2001/XMLSchema#>.
@prefix sotw: <https://w3id.org/force/sotw#>.
`;
const parse = (ttl: string) => new Parser({ baseIRI: "http://example.org/" }).parse(PFX + ttl);

/** Vendored request-1.ttl, verbatim (Alice requests to read x). */
const REQUEST = `
<urn:uuid:req> a odrl:Request; odrl:uid <urn:uuid:req>;
    odrl:permission <urn:uuid:reqperm>.
<urn:uuid:reqperm> a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x.`;

/** Vendored temporal.ttl, verbatim: current time 2024-02-12T11:20:10.999Z. */
const SOTW = `
<urn:uuid:sotw> a ex:Sotw; ex:includes temp:currentTime.
temp:currentTime dct:issued "2024-02-12T11:20:10.999Z"^^xsd:dateTime.`;

/** Wrap a rule body in the vendored policy-N.ttl Set shape. */
const policy = (body: string) => `
<urn:uuid:pol> a odrl:Set; odrl:uid <urn:uuid:pol>;
    odrl:permission <urn:uuid:perm>.
<urn:uuid:perm> a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
${body}`;

/** ground_truth.rs reduction, identical to allow-deny-bench.ts. */
function reduce(quads: Quad[]): "Allow" | "Deny" {
    const s = new Store(quads);
    let proh = false, perm = false;
    for (const q of s.getQuads(null, REPORT + "activationState", null, null)) {
        if (q.object.value !== REPORT + "Active") continue;
        const t = s.getObjects(q.subject, RDF_TYPE, null)[0]?.value;
        if (t === REPORT + "ProhibitionReport") proh = true;
        else if (t === REPORT + "PermissionReport") perm = true;
    }
    return proh ? "Deny" : perm ? "Allow" : "Deny";
}

type Probe = { name: string; policy: string; request?: string; expect: "Allow" | "Deny"; why: string };

const probes: Probe[] = [
    {
        name: "P1a nested and/or (depth 2), satisfied branch",
        why: "or{ and{dateTime gt 09:00, dateTime lt 17:00}, dateTime eq 1999 }; now=11:20 -> and-branch true -> Allow",
        expect: "Allow",
        policy: policy(`    odrl:constraint <urn:uuid:c-or>.
<urn:uuid:c-or> a odrl:LogicalConstraint; odrl:or <urn:uuid:c-and>, <urn:uuid:c-far>.
<urn:uuid:c-and> a odrl:LogicalConstraint; odrl:and <urn:uuid:c1>, <urn:uuid:c2>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:gt;
    odrl:rightOperand "2024-02-12T09:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c2> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:lt;
    odrl:rightOperand "2024-02-12T17:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c-far> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:eq;
    odrl:rightOperand "1999-01-01T00:00:00.000Z"^^xsd:dateTime.`),
    },
    {
        name: "P1b nested and/or (depth 2), unsatisfied branch",
        why: "same shape, window moved to 13:00-17:00; now=11:20 -> both branches false -> Deny",
        expect: "Deny",
        policy: policy(`    odrl:constraint <urn:uuid:c-or>.
<urn:uuid:c-or> a odrl:LogicalConstraint; odrl:or <urn:uuid:c-and>, <urn:uuid:c-far>.
<urn:uuid:c-and> a odrl:LogicalConstraint; odrl:and <urn:uuid:c1>, <urn:uuid:c2>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:gt;
    odrl:rightOperand "2024-02-12T13:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c2> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:lt;
    odrl:rightOperand "2024-02-12T17:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c-far> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:eq;
    odrl:rightOperand "1999-01-01T00:00:00.000Z"^^xsd:dateTime.`),
    },
    {
        name: "P2a xone, exactly one true",
        why: "xone{ dateTime gt 09:00 (true), dateTime eq 1999 (false) } -> Allow",
        expect: "Allow",
        policy: policy(`    odrl:constraint <urn:uuid:c-x>.
<urn:uuid:c-x> a odrl:LogicalConstraint; odrl:xone <urn:uuid:c1>, <urn:uuid:c2>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:gt;
    odrl:rightOperand "2024-02-12T09:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c2> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:eq;
    odrl:rightOperand "1999-01-01T00:00:00.000Z"^^xsd:dateTime.`),
    },
    {
        name: "P2b xone, both true (must NOT activate)",
        why: "xone{ dateTime gt 09:00 (true), dateTime lt 17:00 (true) } -> exclusive-or fails -> Deny",
        expect: "Deny",
        policy: policy(`    odrl:constraint <urn:uuid:c-x>.
<urn:uuid:c-x> a odrl:LogicalConstraint; odrl:xone <urn:uuid:c1>, <urn:uuid:c2>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:gt;
    odrl:rightOperand "2024-02-12T09:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c2> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:lt;
    odrl:rightOperand "2024-02-12T17:00:00.000Z"^^xsd:dateTime.`),
    },
    {
        name: "P3 odrl:isAnyOf on odrl:purpose",
        why: "purpose isAnyOf (ex:research ex:teaching); request context purpose=ex:research -> Allow",
        expect: "Allow",
        request: `
<urn:uuid:req> a odrl:Request; odrl:uid <urn:uuid:req>; odrl:permission <urn:uuid:reqperm>.
<urn:uuid:reqperm> a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    sotw:context <urn:uuid:ctx>.
<urn:uuid:ctx> odrl:leftOperand odrl:purpose; odrl:rightOperand ex:research.`,
        policy: policy(`    odrl:constraint <urn:uuid:c1>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:purpose; odrl:operator odrl:isAnyOf;
    odrl:rightOperand ex:research, ex:teaching.`),
    },
    {
        name: "P4 odrl:isNoneOf on odrl:purpose",
        why: "purpose isNoneOf (ex:marketing ex:resale); request context purpose=ex:research -> Allow",
        expect: "Allow",
        request: `
<urn:uuid:req> a odrl:Request; odrl:uid <urn:uuid:req>; odrl:permission <urn:uuid:reqperm>.
<urn:uuid:reqperm> a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    sotw:context <urn:uuid:ctx>.
<urn:uuid:ctx> odrl:leftOperand odrl:purpose; odrl:rightOperand ex:research.`,
        policy: policy(`    odrl:constraint <urn:uuid:c1>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:purpose; odrl:operator odrl:isNoneOf;
    odrl:rightOperand ex:marketing, ex:resale.`),
    },
    {
        name: "P5a numeric leftOperand odrl:count gt (true)",
        why: "count gt 5; request context count=10 -> Allow",
        expect: "Allow",
        request: `
<urn:uuid:req> a odrl:Request; odrl:uid <urn:uuid:req>; odrl:permission <urn:uuid:reqperm>.
<urn:uuid:reqperm> a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    sotw:context <urn:uuid:ctx>.
<urn:uuid:ctx> odrl:leftOperand odrl:count; odrl:rightOperand "10"^^xsd:integer.`,
        policy: policy(`    odrl:constraint <urn:uuid:c1>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:count; odrl:operator odrl:gt;
    odrl:rightOperand "5"^^xsd:integer.`),
    },
    {
        name: "P5b numeric leftOperand odrl:count gt (false)",
        why: "count gt 5; request context count=3 -> Deny",
        expect: "Deny",
        request: `
<urn:uuid:req> a odrl:Request; odrl:uid <urn:uuid:req>; odrl:permission <urn:uuid:reqperm>.
<urn:uuid:reqperm> a odrl:Permission;
    odrl:assignee ex:alice; odrl:action odrl:read; odrl:target ex:x;
    sotw:context <urn:uuid:ctx>.
<urn:uuid:ctx> odrl:leftOperand odrl:count; odrl:rightOperand "3"^^xsd:integer.`,
        policy: policy(`    odrl:constraint <urn:uuid:c1>.
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:count; odrl:operator odrl:gt;
    odrl:rightOperand "5"^^xsd:integer.`),
    },
    {
        name: "P6 ODRL 2.2 spec shape: odrl:and pointing at an rdf:List",
        why: "ODRL 2.2 IM 2.6 says a LogicalConstraint operand's value is an rdf:List of Constraints. Same two satisfied dateTime constraints as P1a's and-branch, expressed as a list.",
        expect: "Allow",
        policy: policy(`    odrl:constraint <urn:uuid:c-and>.
<urn:uuid:c-and> a odrl:LogicalConstraint; odrl:and ( <urn:uuid:c1> <urn:uuid:c2> ).
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:gt;
    odrl:rightOperand "2024-02-12T09:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c2> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:lt;
    odrl:rightOperand "2024-02-12T17:00:00.000Z"^^xsd:dateTime.`),
    },
    {
        name: "P6b rdf:List odrl:and where BOTH constraints are FALSE",
        why: "Control for P6: if the list form were really evaluated this must Deny. If it still Allows, the list is being ignored and the LogicalConstraint is vacuously satisfied.",
        expect: "Deny",
        policy: policy(`    odrl:constraint <urn:uuid:c-and>.
<urn:uuid:c-and> a odrl:LogicalConstraint; odrl:and ( <urn:uuid:c1> <urn:uuid:c2> ).
<urn:uuid:c1> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:gt;
    odrl:rightOperand "2099-01-01T00:00:00.000Z"^^xsd:dateTime.
<urn:uuid:c2> a odrl:Constraint; odrl:leftOperand odrl:dateTime; odrl:operator odrl:lt;
    odrl:rightOperand "1999-01-01T00:00:00.000Z"^^xsd:dateTime.`),
    },
];

async function main() {
    const evaluator = new ODRLEvaluator(new ODRLEngineMultipleSteps());
    console.log(`odrl-evaluator ${require("odrl-evaluator/package.json").version}\n`);
    for (const p of probes) {
        let got: string, note = "";
        try {
            // @ts-ignore
            const quads: Quad[] = await evaluator.evaluate(parse(p.policy), parse(p.request ?? REQUEST), parse(SOTW));
            got = reduce(quads);
            const s = new Store(quads);
            const sat = s.getQuads(null, REPORT + "satisfactionState", null, null)
                .map((q) => `${s.getObjects(q.subject, RDF_TYPE, null)[0]?.value.replace(REPORT, "") ?? "?"}=${q.object.value.replace(REPORT, "")}`);
            note = sat.join(" ");
        } catch (e) {
            got = "ERROR";
            note = e instanceof Error ? e.message : String(e);
        }
        console.log(`${got === p.expect ? "OK  " : "DIFF"} ${p.name}`);
        console.log(`       ${p.why}`);
        console.log(`       expect=${p.expect} got=${got}   premises: ${note}\n`);
    }
}
main();
