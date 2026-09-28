//! Spike harness: measure the real cost/shape of turning on
//! `ReasonerOptions.proof` for `n3-enforcer`'s two-round ODRL enforcement,
//! against the same 65 real cases `tests/differential.rs` uses.
//!
//! Run twice, as two separate processes, to get an honest peak-memory
//! reading for each mode (VmHWM only grows within a process, so comparing
//! the two in one run would silently penalize whichever runs second):
//!
//!   cargo run --release --example proof_spike -- --no-proof
//!   cargo run --release --example proof_spike -- --proof
//!
//! Each run reports: total/median/slowest wall time across both rounds of
//! all cases, peak resident memory, and -- `--proof` only -- total proof
//! text size and how many cases' proofs actually check out via eyeron's
//! own independent checker (`proof_check_n3::check_proof_document`), not
//! just "were produced".

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use engine::Request;
use eyeron::{parse_n3, result_to_string, PreparedReasoner, ReasonerOptions};
use n3_enforcer::{rules, to_n3};

#[cfg(target_os = "linux")]
fn peak_resident_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kilobytes: usize = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kilobytes * 1024)
}

#[cfg(not(target_os = "linux"))]
fn peak_resident_bytes() -> Option<usize> {
    None
}

fn cases() -> Vec<(String, Request)> {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../compliance/reports/latest-cases.json"
    ))
    .expect("latest-cases.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| {
            let req: Request = serde_json::from_value(c["request"].clone()).unwrap();
            let constraints: usize = req
                .policies
                .iter()
                .flat_map(|p| p.permissions.iter().chain(&p.prohibitions))
                .map(|r| r.constraints.len())
                .sum();
            // Same skip as tests/differential.rs's default (no N3E_ALL):
            // the 3 big-policy fixtures cost ~18s apiece on the reasoner.
            if constraints > 100 {
                return None;
            }
            Some((c["slug"].as_str().unwrap().to_string(), req))
        })
        .collect()
}

struct RulesCache(Mutex<HashMap<String, PreparedReasoner>>);

impl RulesCache {
    fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }

    /// Reason `data` against `rules`, options included, timing only the
    /// reasoning call itself (not the one-time rules parse this cache
    /// amortizes away, mirroring EyeronLib's own cache in src/lib.rs).
    fn reason(
        &self,
        data: &str,
        rules_text: &str,
        options: &ReasonerOptions,
    ) -> (String, Duration, Option<String>) {
        let data_doc = parse_n3(data, None).expect("data parses");
        let mut guard = self.0.lock().unwrap();
        if !guard.contains_key(rules_text) {
            let rules_doc = parse_n3(rules_text, None).expect("rules parse");
            guard.insert(rules_text.to_string(), PreparedReasoner::new(rules_doc));
        }
        let prepared = guard.get(rules_text).unwrap();
        let started = Instant::now();
        let result = prepared.reason(&data_doc, options);
        let elapsed = started.elapsed();
        drop(guard);
        let derived = result_to_string(&data_doc.prefixes, &result.derived);
        let proof_n3 = if options.proof {
            Some(eyeron::proof_to_n3(&data_doc.prefixes, &result))
        } else {
            None
        };
        (derived, elapsed, proof_n3)
    }
}

fn check_one_proof(rules_text: &str, data: &str, proof_n3: &str) -> bool {
    if proof_n3.is_empty() {
        // No new facts derived this round -- nothing to prove, not a
        // failure (several real cases derive nothing in round 2).
        return true;
    }
    let merged = format!("{data}\n{rules_text}");
    let Ok(source_doc) = parse_n3(&merged, None) else {
        return false;
    };
    match eyeron::proof_check_n3::check_proof_document(&source_doc, proof_n3) {
        Ok(report) => report.valid(),
        Err(_) => false,
    }
}

fn main() {
    let with_proof = std::env::args().any(|a| a == "--proof");
    let options = ReasonerOptions {
        proof: with_proof,
        ..ReasonerOptions::default()
    };

    let round1_cache = RulesCache::new();
    let round2_cache = RulesCache::new();

    let mut timings: Vec<(Duration, String)> = Vec::new();
    let mut total_proof_bytes = 0usize;
    let mut proofs_checked = 0usize;
    let mut proofs_valid = 0usize;
    let mut per_case_proof_bytes: Vec<(usize, String)> = Vec::new();

    for (slug, req) in cases() {
        let n3_input = match to_n3(&req) {
            Ok(n3) => n3,
            Err(_) => continue, // Unsupported: same skip as the differential test's Err(Unsupported) branch.
        };

        let (round1_out, t1, proof1) = round1_cache.reason(&n3_input, rules::ROUND1, &options);
        let round2_data = format!("{n3_input}\n{round1_out}");
        let (_round2_out, t2, proof2) = round2_cache.reason(&round2_data, rules::ROUND2, &options);

        timings.push((t1 + t2, slug.clone()));

        let mut case_bytes = 0usize;
        if let Some(p1) = &proof1 {
            total_proof_bytes += p1.len();
            case_bytes += p1.len();
            proofs_checked += 1;
            if check_one_proof(rules::ROUND1, &n3_input, p1) {
                proofs_valid += 1;
            }
        }
        if let Some(p2) = &proof2 {
            total_proof_bytes += p2.len();
            case_bytes += p2.len();
            proofs_checked += 1;
            if check_one_proof(rules::ROUND2, &round2_data, p2) {
                proofs_valid += 1;
            }
        }
        if with_proof {
            per_case_proof_bytes.push((case_bytes, slug.clone()));
        }
    }

    timings.sort();
    let median = timings[timings.len() / 2].0;
    let total: Duration = timings.iter().map(|(d, _)| *d).sum();
    let slowest = &timings[timings.len().saturating_sub(3)..];

    println!("mode: {}", if with_proof { "proof" } else { "no-proof" });
    println!("cases: {}", timings.len());
    println!("total wall time (both rounds, all cases): {total:?}");
    println!("median per-case (both rounds): {median:?}");
    println!("slowest 3: {slowest:?}");
    if let Some(peak) = peak_resident_bytes() {
        println!("peak resident: {} MB", peak / (1024 * 1024));
    }
    if with_proof {
        println!(
            "total proof text: {} bytes ({} proofs)",
            total_proof_bytes, proofs_checked
        );
        println!("proofs independently valid: {proofs_valid}/{proofs_checked}");
        per_case_proof_bytes.sort();
        let n = per_case_proof_bytes.len();
        println!(
            "per-case proof bytes (both rounds combined): min {:?}, median {:?}, max {:?}",
            per_case_proof_bytes.first(),
            per_case_proof_bytes[n / 2],
            per_case_proof_bytes.last(),
        );
    }
}
