/** Dump the SolidLab evaluator's actual compliance report for one index.ttl case slug. */
import * as fs from "fs";
import * as path from "path";
import { Parser, Store, Writer, Quad } from "n3";
import { ODRLEngineMultipleSteps, ODRLEvaluator } from "odrl-evaluator";

const DATA = path.join(__dirname, "..", "data");
const parseFile = (p: string) => new Parser({ baseIRI: "http://example.org/" }).parse(fs.readFileSync(p, "utf-8"));
const local = (u: string) => path.join(DATA, u.slice(u.indexOf("/data/") + 6));

async function main() {
    const want = process.argv[2];
    const index = new Store(parseFile(path.join(DATA, "index.ttl")));
    const ids = index.getSubjects(null, null, null).map((s) => s.value).filter((v, i, a) => a.indexOf(v) === i);
    for (const id of ids) {
        const one = (p: string) => index.getObjects(id, p, null)[0].value;
        const exp = one("http://example.org/expectedReportSource");
        if (path.basename(exp, ".ttl") !== want) continue;
        console.log(`### case ${want}  ${index.getObjects(id, "http://purl.org/dc/terms/title", null)[0].value}`);
        console.log(`# policy  : ${path.basename(local(one("http://example.org/policySource")))}`);
        console.log(`# request : ${path.basename(local(one("http://example.org/requestSource")))}`);
        console.log(`# sotw    : ${path.basename(local(one("http://example.org/sotwSource")))}`);
        const evaluator = new ODRLEvaluator(new ODRLEngineMultipleSteps());
        // @ts-ignore
        const quads: Quad[] = await evaluator.evaluate(
            parseFile(local(one("http://example.org/policySource"))),
            parseFile(local(one("http://example.org/requestSource"))),
            parseFile(local(one("http://example.org/sotwSource"))));
        console.log("# ---- ACTUAL report produced by odrl-evaluator:");
        console.log(new Writer({ format: "N-Triples" }).quadsToString(quads));
        return;
    }
    console.error("no such case");
}
main();
