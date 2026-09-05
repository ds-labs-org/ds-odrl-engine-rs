//! Parses `compliance/vendor/odrl-test-suite/data/index.ttl` — one
//! `urn:uuid` subject per test case, `dct:title` plus four custom
//! (non-standard, `http://example.org/...`) predicates each pointing at a
//! `raw.githubusercontent.com` URL for a file *inside this same vendored
//! submodule*. Per the task: rewritten to local paths under
//! `compliance/vendor/odrl-test-suite/data/...`, never fetched over the
//! network.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oxrdf::Term;

use crate::graph::Graph;

pub struct TestCaseEntry {
    pub id: String,
    pub title: String,
    pub policy_path: PathBuf,
    pub request_path: PathBuf,
    pub expected_report_path: PathBuf,
}

/// A raw-source URL always looks like
/// `https://raw.githubusercontent.com/SolidLabResearch/ODRL-Test-Suite/refs/heads/main/data/<rest>`
/// — this rewrites it to `<vendor_root>/data/<rest>`, i.e. the exact file
/// already vendored in this repository's own submodule checkout.
fn rewrite_to_local_path(vendor_root: &Path, url: &str) -> Result<PathBuf, String> {
    let marker = "/data/";
    let idx = url
        .find(marker)
        .ok_or_else(|| format!("source URL does not contain {marker:?}: {url}"))?;
    Ok(vendor_root.join(&url[idx + 1..]))
}

pub fn parse_index(vendor_root: &Path) -> Result<Vec<TestCaseEntry>, String> {
    let index_path = vendor_root.join("data/index.ttl");
    let g = Graph::parse(&index_path)?;

    let title_pred = "http://purl.org/dc/terms/title";
    let policy_pred = "http://example.org/policySource";
    let request_pred = "http://example.org/requestSource";
    let expected_pred = "http://example.org/expectedReportSource";

    let mut subjects: Vec<String> = Vec::new();
    for t in g.triples() {
        let s = match &t.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            oxrdf::NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
        };
        if !subjects.contains(&s) {
            subjects.push(s);
        }
    }

    let mut entries = Vec::new();
    for id in subjects {
        let title = g
            .object(&id, title_pred)
            .and_then(|t| match t {
                Term::Literal(l) => Some(l.value().to_string()),
                _ => None,
            })
            .ok_or_else(|| format!("{id}: no dct:title"))?;
        let policy_url = g
            .object_node(&id, policy_pred)
            .ok_or_else(|| format!("{id}: no policySource"))?;
        let request_url = g
            .object_node(&id, request_pred)
            .ok_or_else(|| format!("{id}: no requestSource"))?;
        let expected_url = g
            .object_node(&id, expected_pred)
            .ok_or_else(|| format!("{id}: no expectedReportSource"))?;

        entries.push(TestCaseEntry {
            id,
            title,
            policy_path: rewrite_to_local_path(vendor_root, &policy_url)?,
            request_path: rewrite_to_local_path(vendor_root, &request_url)?,
            expected_report_path: rewrite_to_local_path(vendor_root, &expected_url)?,
        });
    }

    // Stable, human-legible ordering: by the "testcase-NNN" sequence
    // number the upstream suite itself encodes in the expected-report
    // filename, not index.ttl's own (incidental) triple order.
    let mut numbered: BTreeMap<u32, TestCaseEntry> = BTreeMap::new();
    for entry in entries {
        let n = sequence_number(&entry.expected_report_path);
        numbered.insert(n, entry);
    }
    Ok(numbered.into_values().collect())
}

fn sequence_number(path: &Path) -> u32 {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    stem.split('-').nth(1).and_then(|n| n.parse().ok()).unwrap_or(0)
}

pub fn case_slug(entry: &TestCaseEntry) -> String {
    entry
        .expected_report_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&entry.id)
        .to_string()
}
