mod cases;
mod graph;
mod ground_truth;
mod index;
mod odrl;
mod report;
mod translate;

use std::fs;
use std::path::PathBuf;

use cases::FixtureData;
use graph::Graph;
use report::CaseResult;
use translate::Translation;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().expect("compliance-runner has a parent directory");
    let vendor_root = repo_root.join("compliance/vendor/odrl-test-suite");

    let entries = index::parse_index(&vendor_root).unwrap_or_else(|e| {
        eprintln!("failed to parse {}: {e}", vendor_root.join("data/index.ttl").display());
        std::process::exit(1);
    });

    let mut results = Vec::with_capacity(entries.len());
    // Built in the same per-case closure as `results`, from the very same
    // bindings, so `latest-cases.json`'s request for a case cannot drift
    // from the one `latest.json`'s verdict was actually computed over.
    let mut fixtures: Vec<(String, String, FixtureData)> = Vec::with_capacity(entries.len());

    for entry in &entries {
        let slug = index::case_slug(entry);
        let title = entry.title.clone();

        let outcome = (|| -> Result<(CaseResult, FixtureData), String> {
            let policy_graph = Graph::parse(&entry.policy_path)?;
            let request_graph = Graph::parse(&entry.request_path)?;
            let sotw_graph = Graph::parse(&entry.sotw_path)?;

            let policy = odrl::parse_policy(&policy_graph)?;
            let request = odrl::parse_request(&request_graph)?;

            match translate::translate(&policy, &request, &sotw_graph, &entry.id) {
                Translation::Skip(reason) => Ok((
                    CaseResult::Skipped { slug: slug.clone(), title: title.clone(), reason: reason.clone() },
                    FixtureData::Skipped { reason },
                )),
                Translation::Ready(wire_request) => {
                    let expected_graph = Graph::parse(&entry.expected_report_path)?;
                    let expected = ground_truth::expected_decision(&expected_graph);

                    let response = engine::evaluate_request(&wire_request);
                    let actual = response.decision;

                    let result = if actual == expected {
                        CaseResult::Passed { slug: slug.clone(), title: title.clone(), decision: actual }
                    } else {
                        CaseResult::Failed {
                            slug: slug.clone(),
                            title: title.clone(),
                            expected,
                            actual,
                            reason: response.reason,
                        }
                    };

                    Ok((result, FixtureData::Ready { request: wire_request, expected }))
                }
            }
        })();

        match outcome {
            Ok((r, fixture)) => {
                results.push(r);
                fixtures.push((slug, title, fixture));
            }
            Err(e) => {
                eprintln!("{slug}: {e}");
                std::process::exit(1);
            }
        }
    }

    let (md, json) = report::render(&results);

    let reports_dir = repo_root.join("compliance/reports");
    fs::create_dir_all(&reports_dir).expect("create compliance/reports");
    fs::write(reports_dir.join("latest.md"), &md).expect("write latest.md");
    fs::write(reports_dir.join("latest.json"), &json).expect("write latest.json");

    // The third artifact, written from the same run and in the same order:
    // the per-case fixtures the site re-executes against `engine.wasm` in a
    // visitor's browser (see cases.rs). Nothing above it changes — the two
    // reports are byte-for-byte what they were before this existed.
    let fixture_views: Vec<_> =
        fixtures.iter().map(|(slug, title, data)| cases::fixture_view(slug, title, data)).collect();
    fs::write(reports_dir.join("latest-cases.json"), cases::render(&fixture_views)).expect("write latest-cases.json");

    let passed = results.iter().filter(|r| matches!(r, CaseResult::Passed { .. })).count();
    let failed = results.iter().filter(|r| matches!(r, CaseResult::Failed { .. })).count();
    let skipped = results.iter().filter(|r| matches!(r, CaseResult::Skipped { .. })).count();
    println!(
        "ODRL-Test-Suite compliance: {} total, {passed} passed, {failed} failed, {skipped} skipped -> {}",
        results.len(),
        reports_dir.join("latest.md").display()
    );
}
