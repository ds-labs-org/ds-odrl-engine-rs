/**
 * Engine-1 bench harness: SolidLab ODRL Evaluator (odrl-evaluator@0.4.0)
 * against the SAME 68 fixtures ds-odrl-engine-rs's compliance-runner uses,
 * reduced to the SAME Allow/Deny ground truth.
 *
 * Case selection mirrors compliance-runner/src/index.rs: subjects of
 * data/index.ttl, with raw.githubusercontent.com source URLs rewritten to
 * local paths under data/ (never fetched), ordered by the "testcase-NNN"
 * sequence number in the expected-report filename.
 *
 * Decision reduction mirrors compliance-runner/src/ground_truth.rs exactly:
 *   any report:ProhibitionReport with report:activationState report:Active -> Deny
 *   else any report:PermissionReport with report:activationState report:Active -> Allow
 *   else Deny  (ODRL Formal Semantics "closed" default Behaviour)
 * applied to BOTH the fixture's expected report (= ground truth) and the
 * report the SolidLab evaluator actually produces (= this engine's decision).
 * report:DutyReport nodes are ignored, as in ground_truth.rs.
 */
import * as fs from "fs";
import * as path from "path";
import { Parser, Store, Quad } from "n3";
import { ODRLEngineMultipleSteps, ODRLEvaluator } from "odrl-evaluator";

const REPORT = "https://w3id.org/force/compliance-report#";
const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const DATA = path.join(__dirname, "..", "data");

type Decision = "Allow" | "Deny";

function parseFile(p: string): Quad[] {
    return new Parser({ baseIRI: "http://example.org/" }).parse(fs.readFileSync(p, "utf-8"));
}

/** index.rs: rewrite ".../data/<rest>" -> "<data>/<rest>". */
function rewriteToLocal(url: string): string {
    const i = url.indexOf("/data/");
    if (i === -1) throw new Error(`source URL has no /data/: ${url}`);
    return path.join(DATA, url.slice(i + "/data/".length));
}

/** ground_truth.rs::expected_decision, on an arbitrary set of report quads. */
function reduceToDecision(quads: Quad[]): Decision {
    const store = new Store(quads);
    let prohibitionActive = false;
    let permissionActive = false;
    for (const q of store.getQuads(null, REPORT + "activationState", null, null)) {
        const isActive = q.object.value === REPORT + "Active";
        if (!isActive) continue;
        const types = store.getObjects(q.subject, RDF_TYPE, null);
        const t = types.length > 0 ? types[0].value : undefined; // object_node(): first match
        if (t === REPORT + "ProhibitionReport") prohibitionActive = true;
        else if (t === REPORT + "PermissionReport") permissionActive = true;
    }
    if (prohibitionActive) return "Deny";
    if (permissionActive) return "Allow";
    return "Deny";
}

async function main() {
    const index = new Store(parseFile(path.join(DATA, "index.ttl")));
    const ids = index.getSubjects(null, null, null)
        .map((s) => s.value)
        .filter((v, i, a) => a.indexOf(v) === i); // dedupe (suite tsconfig targets pre-ES2015: no Set spread)

    type Entry = { id: string; slug: string; title: string; policy: string; request: string; sotw: string; expected: string; seq: number };
    const entries: Entry[] = ids.map((id) => {
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
    console.error(`cases from index.ttl: ${entries.length}`);

    const evaluator = new ODRLEvaluator(new ODRLEngineMultipleSteps());
    const results: any[] = [];

    for (const e of entries) {
        const expectedDecision = reduceToDecision(parseFile(e.expected));
        let actual: Decision | null = null;
        let status = "ok";
        let error: string | undefined;
        let reportQuads: Quad[] = [];
        const t0 = performance.now();
        try {
            // @ts-ignore  (upstream typings, same @ts-ignore the suite's own TestCaseEvaluator uses)
            reportQuads = await evaluator.evaluate(parseFile(e.policy), parseFile(e.request), parseFile(e.sotw));
            actual = reduceToDecision(reportQuads);
        } catch (err) {
            status = "evaluation-error";
            error = err instanceof Error ? err.message : String(err);
        }
        const ms = performance.now() - t0;
        const outcome = status !== "ok" ? "ERROR" : actual === expectedDecision ? "PASS" : "FAIL";
        results.push({
            slug: e.slug, id: e.id, title: e.title, expected: expectedDecision, actual, outcome, status, error,
            ruleReports: reportQuads.filter((q) => q.predicate.value === REPORT + "ruleReport").length,
            ms: Math.round(ms),
        });
        console.error(`${e.slug}: expected=${expectedDecision} actual=${actual ?? "-"} ${outcome} (${Math.round(ms)}ms)`);
    }

    const pass = results.filter((r) => r.outcome === "PASS").length;
    const fail = results.filter((r) => r.outcome === "FAIL").length;
    const err = results.filter((r) => r.outcome === "ERROR").length;
    console.error(`TOTAL ${results.length}  PASS ${pass}  FAIL ${fail}  ERROR ${err}  SKIP 0`);
    fs.writeFileSync(process.env.OUT || path.join(__dirname, "..", "..", "allow-deny-results.json"),
        JSON.stringify({ engine: "SolidLabResearch/ODRL-Evaluator", version: require("odrl-evaluator/package.json").version, total: results.length, pass, fail, err, results }, null, 2));
}
main();
