/**
 * The suite's OWN full-FORCE-report comparison, all three implemented
 * comparators, evaluating each test case once. Same code paths as
 * demo/test-suite.ts (which hardcodes only the `simple` comparator).
 */
import * as path from "path";
import { ODRLEngineMultipleSteps, ODRLEvaluator } from "odrl-evaluator";
import { ComplianceReportComparator, loadTestSuite, TestCase, TestCaseEvaluator } from "../src";

async function main() {
    const testCaseMap = await loadTestSuite(path.join(__dirname, "..", "data"));
    const testCases: TestCase[] = [];
    testCaseMap.forEach((tc) => testCases.push(tc));

    const evaluator = new ODRLEvaluator(new ODRLEngineMultipleSteps());
    const runner = new TestCaseEvaluator(evaluator, ComplianceReportComparator.isomorphism);

    const tally: Record<string, number> = {};
    let evaluated = 0;
    for (const tc of testCases) {
        const evaluation = await runner.evaluateAndCompare(tc); // evaluates once (isomorphism)
        if (!evaluation.evaluationStatus) continue;
        evaluated++;
        for (const c of [ComplianceReportComparator.isomorphism, ComplianceReportComparator.activation, ComplianceReportComparator.simple]) {
            const cmp = await new TestCaseEvaluator(evaluator, c).compare(tc, evaluation.evaluation);
            tally[c] = (tally[c] ?? 0) + (cmp.comparisonStatus ? 1 : 0);
        }
    }
    console.log(`odrl-evaluator ${require("odrl-evaluator/package.json").version}`);
    console.log(`test cases: ${testCases.length}, evaluations succeeded: ${evaluated}`);
    for (const [k, v] of Object.entries(tally)) console.log(`  ${k}: ${v}/${testCases.length}`);
}
main();
