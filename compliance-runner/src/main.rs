mod graph;
mod ground_truth;
mod index;
mod odrl;
mod report;
mod translate;

use std::fs;
use std::path::PathBuf;

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

    for entry in &entries {
        let slug = index::case_slug(entry);
        let title = entry.title.clone();

        let outcome = (|| -> Result<CaseResult, String> {
            let policy_graph = Graph::parse(&entry.policy_path)?;
            let request_graph = Graph::parse(&entry.request_path)?;

            let policy = odrl::parse_policy(&policy_graph)?;
            let request = odrl::parse_request(&request_graph)?;

            match translate::translate(&policy, &request, &entry.id) {
                Translation::Skip(reason) => Ok(CaseResult::Skipped { slug: slug.clone(), title: title.clone(), reason }),
                Translation::Ready(wire_request) => {
                    let expected_graph = Graph::parse(&entry.expected_report_path)?;
                    let expected = ground_truth::expected_decision(&expected_graph);

                    let response = engine::evaluate_request(&wire_request);
                    let actual = response.decision;

                    if actual == expected {
                        Ok(CaseResult::Passed { slug: slug.clone(), title: title.clone(), decision: actual })
                    } else {
                        Ok(CaseResult::Failed {
                            slug: slug.clone(),
                            title: title.clone(),
                            expected,
                            actual,
                            reason: response.reason,
                        })
                    }
                }
            }
        })();

        match outcome {
            Ok(r) => results.push(r),
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

    let passed = results.iter().filter(|r| matches!(r, CaseResult::Passed { .. })).count();
    let failed = results.iter().filter(|r| matches!(r, CaseResult::Failed { .. })).count();
    let skipped = results.iter().filter(|r| matches!(r, CaseResult::Skipped { .. })).count();
    println!(
        "ODRL-Test-Suite compliance: {} total, {passed} passed, {failed} failed, {skipped} skipped -> {}",
        results.len(),
        reports_dir.join("latest.md").display()
    );
}
