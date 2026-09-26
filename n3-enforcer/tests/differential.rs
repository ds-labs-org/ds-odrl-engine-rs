//! Differential test: the reasoner-backed enforcer against the native
//! engine on the wire cases `compliance-runner` exports
//! (`compliance/reports/latest-cases.json`, 68 cases with the suite's
//! expected decision).
//!
//! With the default `eyeron` feature this runs in-process, in pure Rust, and
//! is strict: `cargo test -p n3-enforcer`. To cross-check against another
//! reasoner, set `N3E_REASONER`:
//!
//! ```sh
//! N3E_REASONER=/path/to/eyeron   N3E_KIND=eyeron cargo test -p n3-enforcer -- --nocapture
//! N3E_REASONER=/usr/local/bin/eye N3E_KIND=eye    cargo test -p n3-enforcer -- --nocapture
//! N3E_REASONER=node N3E_KIND=args N3E_ARGS=eye-shim.mjs cargo test ...   # eye-js (EYE on WASM)
//! N3E_STRICT=1 ...   # fail on any disagreement or contradictory report
//! ```
//!
//! Cases with more than 100 constraints are skipped unless `N3E_ALL=1`.
//!
//! Categories per case: `agree` (n3 == native == expected), `unsupported`
//! (translation refused, by design), `n3-wrong` (n3 differs from the
//! suite's expected decision), `contradictory` (a rule reported both
//! Active and Inactive), `error` (reasoner failed).

use engine::{evaluate_request, Request, WireDecision};
use n3_enforcer::{CommandReasoner, EnforcerError, N3Enforcer};

fn cases() -> Vec<(String, Request, String)> {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../compliance/reports/latest-cases.json"))
        .expect("latest-cases.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["slug"].as_str().unwrap().to_string(),
                serde_json::from_value(c["request"].clone()).unwrap(),
                c["expected_decision"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn differential_against_native_engine() {
        if std::env::var("N3E_REASONER").is_err() {
        run(N3Enforcer::new(n3_enforcer::EyeronLib));
        return;
    }
    let Ok(bin) = std::env::var("N3E_REASONER") else {
        eprintln!("N3E_REASONER not set and no in-process reasoner built in: differential test skipped");
        return;
    };
    let reasoner = match std::env::var("N3E_KIND").as_deref() {
        // Any command taking the N3 file path as its last argument, e.g.
        // N3E_REASONER=node N3E_KIND=args N3E_ARGS=/path/eye-shim.mjs
        Ok("args") => {
            let args = std::env::var("N3E_ARGS").unwrap_or_default();
            let args: Vec<&str> = args.split_whitespace().collect();
            CommandReasoner::new(bin, &args)
        }
        Ok("eye") => CommandReasoner::eye(bin),
        _ => CommandReasoner::eyeron(bin),
    };
    run(N3Enforcer::new(reasoner));
}

fn run<R: n3_enforcer::Reasoner>(enforcer: N3Enforcer<R>) {
    let (mut agree, mut unsupported, mut wrong, mut contradictory, mut errors) = (0, 0, 0, 0, 0);
    let mut native_wrong = 0;
    let mut timings: Vec<(std::time::Duration, String)> = Vec::new();
    let mut unsupported_reasons: std::collections::BTreeMap<String, usize> = Default::default();
    let mut skipped_large = Vec::new();
    for (slug, req, expected) in cases() {
        // The 3 `big-policy` fixtures carry ~800 constraints each and cost
        // ~18 s apiece on the reasoner (native: microseconds). Skipped unless
        // N3E_ALL is set, so a plain `cargo test` stays quick.
        let constraints: usize = req.policies.iter().flat_map(|p| p.permissions.iter().chain(&p.prohibitions)).map(|r| r.constraints.len()).sum();
        if constraints > 100 && std::env::var("N3E_ALL").is_err() {
            skipped_large.push(slug);
            continue;
        }
        let native = evaluate_request(&req).decision;
        if format!("{native:?}") != expected {
            native_wrong += 1;
        }
        let t0 = std::time::Instant::now();
        let outcome = enforcer.report(&req);
        timings.push((t0.elapsed(), slug.clone()));
        match outcome {
            Err(EnforcerError::Unsupported(u)) => {
                unsupported += 1;
                *unsupported_reasons.entry(format!("{u}")).or_default() += 1;
            }
            Err(e) => {
                errors += 1;
                eprintln!("ERROR    {slug}: {e}");
            }
            Ok((_, summary)) => {
                let resp = n3_enforcer::to_response(&req.dataset_id, &summary);
                if !summary.contradictory.is_empty() {
                    contradictory += 1;
                    eprintln!("CONTRADICTORY {slug}");
                } else if format!("{:?}", resp.decision) == expected && resp.decision == native {
                    agree += 1;
                } else {
                    wrong += 1;
                    let d: WireDecision = resp.decision;
                    eprintln!("N3-WRONG {slug}: n3={d:?} native={native:?} expected={expected}");
                }
            }
        }
    }
    timings.sort();
    let median = timings[timings.len() / 2].0;
    eprintln!("per-case enforcement: median {median:?}, slowest {:?}", &timings[timings.len().saturating_sub(3)..]);
    eprintln!("skipped as large (set N3E_ALL=1 to include): {skipped_large:?}");
    eprintln!(
        "agree={agree} unsupported={unsupported} n3-wrong={wrong} contradictory={contradictory} error={errors} (native vs expected mismatches: {native_wrong})"
    );
    for (r, n) in &unsupported_reasons {
        eprintln!("  unsupported x{n}: {r}");
    }
    assert_eq!(native_wrong, 0, "native engine must match the suite");
    // The in-process reasoner is the enforcer's own dependency, so it must be
    // exact; external reasoners only fail the test when N3E_STRICT is set.
    if std::env::var("N3E_STRICT").is_ok() || std::env::var("N3E_REASONER").is_err() {
        assert_eq!(wrong + contradictory + errors, 0);
    }
}

#[test]
fn enforce_uses_the_reasoner_and_falls_back_for_what_it_cannot_express() {
    use n3_enforcer::{Enforcer, Path};
    let enforcer = Enforcer::eyeron();
    let all = cases();

    // A supported case is decided by the reasoner, and matches the suite.
    let (slug, req, expected) = all.iter().find(|(s, _, _)| s == "testcase-034-alice-read-x-past").unwrap();
    let (resp, path) = enforcer.enforce(req);
    assert_eq!(path, Path::Reasoner, "{slug}");
    assert_eq!(format!("{:?}", resp.decision), *expected);

    // An obligation is outside the rules: refused by the reasoner path and
    // decided natively instead of being guessed.
    let mut with_duty = req.clone();
    with_duty.policies[0].obligations.push(serde_json::from_value(serde_json::json!({"action": "notify", "constraints": []})).unwrap());
    let (resp, path) = enforcer.enforce(&with_duty);
    assert!(matches!(path, Path::Native(ref why) if why.contains("Duties")), "{path:?}");
    assert_eq!(resp, engine::evaluate_request(&with_duty));
}
