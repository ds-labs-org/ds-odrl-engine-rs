//! Performance, resource and load instrumentation for `ds-odrl-engine-rs`
//! itself — the baseline row of the five-engine comparison in
//! `bench/README.md`.
//!
//! # What this measures, and why there are two of everything
//!
//! Every other engine in that comparison has exactly one way to be
//! called. This one has two that matter, and they are not the same
//! product:
//!
//! * **(a) NATIVE** — `engine::evaluate_request(&Request)`, a plain Rust
//!   library call. This is the entry point `compliance-runner/src/main.rs`
//!   uses for every one of the 68 fixtures, so it is the invocation path
//!   the conformance number was produced through. It is also a genuine
//!   *lower bound*: no serialization, no ABI, no host boundary, no
//!   interpreter — just the decision procedure.
//!
//! * **(b) WASM-ABI** — the same requests driven through the real
//!   `engine.wasm` (`cargo build -p engine --target wasm32-unknown-unknown
//!   --release`) via its `alloc`/`evaluate`/`dealloc` C ABI, over a
//!   `wasmi` interpreter, using the same `host.rs` code path
//!   `release-history` already uses for historical releases. This is what
//!   a *host* experiences — a browser, a JVM via Chicory, another Rust
//!   process embedding the artifact rather than linking the crate. It is
//!   reported at two fuel settings because the repo really runs both:
//!   metered (as `release-history` does) and unmetered (as the browser
//!   bridge does).
//!
//! Reporting only (a) would flatter the engine with a number no
//! integrator can reach. Reporting only (b) would charge the engine for
//! an interpreter it does not require. Both are here, labelled, and the
//! README states plainly which one a given comparison should use.
//!
//! # The corpus
//!
//! `compliance/reports/latest-cases.json` — the exact per-case
//! `engine::wire::Request` values `compliance-runner` fed
//! `engine::evaluate_request` on its own run, exported by
//! `compliance-runner/src/cases.rs` for precisely this kind of
//! re-execution. Using that artifact instead of re-deriving requests from
//! the RDF fixtures is deliberate: it guarantees the requests timed here
//! are byte-identical to the ones the conformance tally was computed
//! over, so a latency reported per case attaches to a case whose verdict
//! is already published. All 68 are evaluable (0 skipped), so this bench
//! covers the entire corpus rather than a subset.
//!
//! Each phase re-checks its decisions against the fixture's own
//! `expected_decision` and reports the agreement count. That is not a
//! second conformance run — it is a guard that the thing being timed is
//! still the thing that was scored.
//!
//! # Phases
//!
//! ```text
//! perf-bench native      OUT.json     # (a), per-case latency
//! perf-bench wasm        OUT.json     # (b), per-case latency, metered + unmetered
//! perf-bench load-native OUT.json     # (a) under a real OS-thread ramp
//! perf-bench load-wasm   OUT.json     # (b) under a real OS-thread ramp
//! ```
//!
//! Each is a separate process so that `/usr/bin/time -v`'s "Maximum
//! resident set size" and CPU seconds are attributable to one phase
//! rather than to a mixture. `run.sh` drives all four that way.

mod host;
mod stats;

use std::collections::HashMap;
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use engine::{Request, WireDecision};
use serde::Serialize;
use stats::Summary;

use host::WasmEngine;

// ---------------------------------------------------------------------
// Knobs. Every one of these is quoted in the README next to the numbers
// it produced, so a reader never has to guess how many times something
// ran.
// ---------------------------------------------------------------------

/// Full corpus passes executed and **discarded** before any timer starts.
///
/// Rust has no JIT to warm, but three things here genuinely are cold on
/// the first pass and would otherwise be charged to fixture #1: the OS
/// page cache and the process's own resident pages for the freshly-read
/// 650 KB cases file, the allocator's first-touch growth of its arenas,
/// and — on the wasm path — `wasmi`'s lazy per-function translation, its
/// guest allocator's first `alloc` of each size class, and the guest's
/// linear-memory growth. Three passes is enough for all three to settle
/// and doubles as the smoke test: a corpus that cannot get through warmup
/// cleanly is reported as a failure before any latency is believed.
const WARMUP_PASSES: usize = 3;

/// Timed repetitions per case, native path. 501 (odd, so the median is a
/// real observed sample) — a native evaluation is single-digit
/// microseconds, so 501 * 68 is still well under a second and buys a
/// per-case distribution dense enough for the within-case IQR gate to
/// mean something.
const NATIVE_REPS: usize = 501;

/// Timed repetitions per case, wasm path. Lower than the native count
/// only because an interpreted round trip is two to three orders of
/// magnitude more expensive; 51 is still odd and still enough for a
/// quartile.
const WASM_REPS: usize = 51;

/// Repetitions of the whole `Module::new` + `instantiate_and_start`
/// sequence, measured separately as the host's cold-start cost.
const INSTANTIATE_REPS: usize = 21;

/// Seconds each concurrency step of the load ramp runs at full tilt.
const RAMP_STEP_SECONDS: u64 = 3;

/// How many times the WHOLE ramp is repeated. The per-step throughput
/// reported is the median of these, and a step whose spread across them
/// trips `RAMP_STEP_RELATIVE_RANGE_MAX` is flagged unstable.
const RAMP_REPEATS: usize = 3;

/// Native ramp steps. `nproc` on this machine is 22; the ramp runs past
/// it on purpose (32, 44 = 2x cores) so the report shows what happens
/// when threads genuinely oversubscribe cores rather than stopping at the
/// point where the curve is still flattering.
const NATIVE_RAMP: &[usize] = &[1, 2, 4, 8, 16, 22, 32, 44];

/// Wasm ramp steps, stopping at core count. Each wasm thread owns a whole
/// independent `wasmi` `Store` (a `Store` is not `Sync`, and that is not
/// a harness limitation — it is the execution model), so oversubscribing
/// costs real memory per thread, not just scheduler time. The ramp goes
/// to 22 and the memory cost per step is reported rather than pushed
/// past.
const WASM_RAMP: &[usize] = &[1, 2, 4, 8, 16, 22];

/// Latency sampling under load: record every evaluation of every Nth
/// *whole corpus pass*, per thread, capped. A full trace at the top of
/// the ramp (44 threads at ~265k evals/s for 3 s) would be tens of
/// millions of samples whose allocation would itself distort the RSS
/// number this same run reports, so some subsampling is necessary.
///
/// **Whole passes, not every Nth evaluation** — and that distinction is
/// not cosmetic. The first version of this harness sampled every 8th
/// *evaluation*, and its ramp reported a p99 of 9.7 us at concurrency 1
/// while the quiet per-case run put three fixtures at 200-520 us. The
/// cause: a running counter taken mod 8 against a corpus of 68 cases
/// cycles with period lcm(68, 8) = 136, so only case indices divisible by
/// 4 are ever sampled — and `testcase-062/063/064-big-policy`, the three
/// heaviest fixtures in the corpus, sit at indices 61, 62 and 63. The
/// whole tail of the distribution was structurally invisible. Sampling
/// whole passes makes the recorded distribution the corpus's own mixture
/// by construction, for any corpus length, with no coprimality
/// coincidence to rely on.
const RAMP_SAMPLE_PASS_STRIDE: u64 = 8;
const RAMP_SAMPLE_CAP: usize = 65_536;

// ---------------------------------------------------------------------
// Corpus loading
// ---------------------------------------------------------------------

struct Case {
    slug: String,
    title: String,
    request: Request,
    request_json: String,
    expected: WireDecision,
}

fn repo_root() -> PathBuf {
    // Three parents up from this crate's own manifest directory:
    // bench/ds-odrl-engine-rs/perf -> bench/ds-odrl-engine-rs -> bench ->
    // repo root. Resolved from `CARGO_MANIFEST_DIR` rather than from the
    // process's cwd, and overridable, for the same reason the rescue of
    // the other four harnesses had to fix exactly this bug in three of
    // them: a hardcoded absolute path makes a bench reproducible only in
    // the directory it was written in.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("perf/ has three ancestors up to the repo root")
        .to_path_buf()
}

fn cases_path() -> PathBuf {
    match std::env::var("CASES_JSON") {
        Ok(p) => PathBuf::from(p),
        Err(_) => repo_root().join("compliance/reports/latest-cases.json"),
    }
}

fn wasm_path() -> PathBuf {
    match std::env::var("ENGINE_WASM") {
        Ok(p) => PathBuf::from(p),
        Err(_) => repo_root().join("target/wasm32-unknown-unknown/release/engine.wasm"),
    }
}

fn decision_str(d: WireDecision) -> &'static str {
    match d {
        WireDecision::Allow => "Allow",
        WireDecision::Deny => "Deny",
        WireDecision::Error => "Error",
    }
}

fn load_cases() -> Vec<Case> {
    let path = cases_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read the case corpus at {}: {e}", path.display()));
    let file: serde_json::Value = serde_json::from_str(&raw).expect("latest-cases.json parses as JSON");

    let schema = file["schema"].as_str().unwrap_or("");
    assert_eq!(
        schema, "ds-odrl-engine-rs/compliance-cases@1",
        "case corpus schema changed; this bench pins the shape it knows"
    );

    let mut cases = Vec::new();
    for c in file["cases"].as_array().expect("`cases` is an array") {
        // A skipped case carries no request and cannot be timed. As of the
        // pinned commit there are none; the branch exists so that a future
        // corpus with skips reports a smaller `n` honestly instead of
        // panicking or silently timing 68 things when it evaluated 60.
        let Some(req_value) = c.get("request") else { continue };
        let request: Request = serde_json::from_value(req_value.clone()).expect("a fixture request deserializes");
        let expected = match c["expected_decision"].as_str() {
            Some("Allow") => WireDecision::Allow,
            Some("Deny") => WireDecision::Deny,
            Some("Error") => WireDecision::Error,
            other => panic!("unexpected expected_decision {other:?}"),
        };
        cases.push(Case {
            slug: c["slug"].as_str().unwrap_or("?").to_string(),
            title: c["title"].as_str().unwrap_or("").to_string(),
            request_json: serde_json::to_string(&request).expect("a Request serializes"),
            request,
            expected,
        });
    }
    assert!(!cases.is_empty(), "the corpus contains no evaluable case");
    cases
}

// ---------------------------------------------------------------------
// Process resource readings, straight out of /proc
// ---------------------------------------------------------------------

/// USER_HZ. 100 on every Linux/x86_64 this could plausibly run on;
/// `/usr/bin/time -v` wraps the whole process independently, so if this
/// constant were ever wrong the two numbers would disagree visibly rather
/// than silently.
const USER_HZ: f64 = 100.0;

fn proc_status_kb(field: &str) -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Current RSS in bytes (steady state, sampled at phase checkpoints).
fn rss_bytes() -> u64 {
    proc_status_kb("VmRSS:").unwrap_or(0) * 1024
}

/// `VmHWM` from `/proc/self/status`, read in-process at a phase boundary.
///
/// **Not a substitute for `/usr/bin/time -v`'s "Maximum resident set
/// size", and this bench does not treat it as one.** `/proc`'s `VmHWM` is
/// `max(mm->hiwater_rss, current_rss)`, and `mm->hiwater_rss` is only
/// *latched* at certain kernel events (unmap, exit, and friends) rather
/// than on every page fault. Two consequences were observed in this
/// bench's own output rather than assumed:
///
///  1. a mid-run read can come back *higher* than the peak the process
///     ends up reporting, because it is really reporting current RSS
///     when current exceeds the last latched high-water mark — the wasm
///     phase's fuel-metered checkpoint reads 12,056 KB while the whole
///     process's true maximum was 11,396 KB;
///  2. so a later read can be *lower* than an earlier one, which a real
///     monotonic peak counter could never be.
///
/// Every end-of-phase reading here did match `/usr/bin/time -v` exactly
/// (8,448 KB native, 11,396 KB wasm, 251,892 KB load-native, 186,964 KB
/// load-wasm). The rule this directory's README follows: **the peak
/// figure quoted for a phase is always `/usr/bin/time -v`'s**, and these
/// in-process numbers are steady-state checkpoints.
fn peak_rss_bytes() -> u64 {
    proc_status_kb("VmHWM:").unwrap_or(0) * 1024
}

/// (user, system) CPU seconds consumed by this process so far, from
/// `/proc/self/stat` fields 14 and 15.
fn cpu_seconds() -> (f64, f64) {
    let Ok(s) = std::fs::read_to_string("/proc/self/stat") else { return (0.0, 0.0) };
    // The comm field can contain spaces and parentheses; everything after
    // the last ')' is positionally stable.
    let Some((_, tail)) = s.rsplit_once(')') else { return (0.0, 0.0) };
    let f: Vec<&str> = tail.split_whitespace().collect();
    // After the ')' the next field is `state` (field 3), so utime (14) is
    // index 11 and stime (15) is index 12.
    let get = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (get(11) / USER_HZ, get(12) / USER_HZ)
}

#[derive(Serialize, Clone)]
struct ResourceSnapshot {
    rss_bytes: u64,
    peak_rss_bytes: u64,
    cpu_user_seconds: f64,
    cpu_system_seconds: f64,
}

fn snapshot() -> ResourceSnapshot {
    let (u, s) = cpu_seconds();
    ResourceSnapshot {
        rss_bytes: rss_bytes(),
        peak_rss_bytes: peak_rss_bytes(),
        cpu_user_seconds: u,
        cpu_system_seconds: s,
    }
}

// ---------------------------------------------------------------------
// Per-case latency phases
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct CaseLatency {
    slug: String,
    title: String,
    reps: usize,
    /// Nanoseconds, over this case's own repeat set.
    summary: Summary,
    /// Gate 1: this case's own repeats were jittery
    /// (IQR/median > `WITHIN_CASE_RELATIVE_IQR_MAX`).
    unstable_within_case: bool,
    /// Gate 2: this case's median sits outside the Tukey fence computed
    /// over all cases' medians. Filled in after all cases are timed.
    outlier_across_cases: bool,
    decision: &'static str,
    expected_decision: &'static str,
    agrees_with_expected: bool,
}

#[derive(Serialize)]
struct LatencyPhase {
    path: &'static str,
    what_is_timed: &'static str,
    reps_per_case: usize,
    warmup_passes: usize,
    /// Distribution of the 68 per-case MEDIAN latencies, in nanoseconds.
    /// This is the headline "per-case latency" distribution: one number
    /// per fixture, each already robust to its own repeat noise.
    per_case_median_ns: Summary,
    /// Distribution over every individual timed repetition (68 * reps
    /// samples), in nanoseconds. Wider tails by construction — it
    /// includes every scheduler hiccup — and reported next to the other
    /// so neither can be mistaken for the other.
    all_repetitions_ns: Summary,
    tukey_fence_low_ns: f64,
    tukey_fence_high_ns: f64,
    cases_flagged_unstable: usize,
    cases_flagged_outlier: usize,
    agreement_with_expected: String,
    cases: Vec<CaseLatency>,
    resources_after: ResourceSnapshot,
}

fn finish_phase(
    path: &'static str,
    what_is_timed: &'static str,
    reps: usize,
    mut cases: Vec<CaseLatency>,
    all_reps: Vec<f64>,
    agreed: usize,
) -> LatencyPhase {
    let medians: Vec<f64> = cases.iter().map(|c| c.summary.median).collect();
    let (low, high) = stats::tukey_fence(&medians);
    for c in &mut cases {
        c.outlier_across_cases = c.summary.median < low || c.summary.median > high;
    }
    let n = cases.len();
    LatencyPhase {
        path,
        what_is_timed,
        reps_per_case: reps,
        warmup_passes: WARMUP_PASSES,
        per_case_median_ns: Summary::of(&medians),
        all_repetitions_ns: Summary::of(&all_reps),
        tukey_fence_low_ns: low,
        tukey_fence_high_ns: high,
        cases_flagged_unstable: cases.iter().filter(|c| c.unstable_within_case).count(),
        cases_flagged_outlier: cases.iter().filter(|c| c.outlier_across_cases).count(),
        agreement_with_expected: format!("{agreed}/{n}"),
        cases,
        resources_after: snapshot(),
    }
}

fn phase_native(cases: &[Case]) -> LatencyPhase {
    // Warmup / smoke test.
    for pass in 0..WARMUP_PASSES {
        for c in cases {
            let r = engine::evaluate_request(black_box(&c.request));
            assert!(
                matches!(r.decision, WireDecision::Allow | WireDecision::Deny | WireDecision::Error),
                "warmup pass {pass} produced no decision for {}",
                c.slug
            );
            black_box(&r);
        }
    }

    let mut out = Vec::with_capacity(cases.len());
    let mut all = Vec::with_capacity(cases.len() * NATIVE_REPS);
    let mut agreed = 0usize;

    for c in cases {
        let mut reps = Vec::with_capacity(NATIVE_REPS);
        let mut decision = WireDecision::Error;
        for _ in 0..NATIVE_REPS {
            let t0 = Instant::now();
            let r = engine::evaluate_request(black_box(&c.request));
            let dt = t0.elapsed();
            decision = r.decision;
            black_box(&r);
            reps.push(dt.as_nanos() as f64);
        }
        all.extend_from_slice(&reps);
        let summary = Summary::of(&reps);
        let agrees = decision == c.expected;
        if agrees {
            agreed += 1;
        }
        out.push(CaseLatency {
            slug: c.slug.clone(),
            title: c.title.clone(),
            reps: NATIVE_REPS,
            unstable_within_case: stats::within_case_unstable(&summary),
            summary,
            outlier_across_cases: false,
            decision: decision_str(decision),
            expected_decision: decision_str(c.expected),
            agrees_with_expected: agrees,
        });
    }

    finish_phase(
        "native",
        "engine::evaluate_request(&Request) -- one Rust library call, no serialization, no ABI",
        NATIVE_REPS,
        out,
        all,
        agreed,
    )
}

#[derive(Serialize)]
struct InstantiationCost {
    fuel_metered: bool,
    reps: usize,
    /// `Module::new` + `Store::new` + `instantiate_and_start` + the four
    /// export lookups, in nanoseconds: a host's cold start per instance.
    ns: Summary,
}

fn phase_wasm(cases: &[Case], wasm: &[u8], fuel: bool) -> (LatencyPhase, InstantiationCost) {
    let label: &'static str = if fuel { "wasm-abi-fuel-metered" } else { "wasm-abi-unmetered" };
    let what: &'static str = if fuel {
        "one alloc + memory.write + evaluate + memory.read + 2x dealloc round trip through \
         engine.wasm on wasmi, fuel metering ON (as release-history/src/host.rs drives it). \
         Request->JSON serialization is NOT in the timed region."
    } else {
        "one alloc + memory.write + evaluate + memory.read + 2x dealloc round trip through \
         engine.wasm on wasmi, fuel metering OFF (as a browser host has no fuel accounting). \
         Request->JSON serialization is NOT in the timed region."
    };

    // Cold-start cost, measured before anything is warm.
    let mut inst = Vec::with_capacity(INSTANTIATE_REPS);
    for _ in 0..INSTANTIATE_REPS {
        let t0 = Instant::now();
        let e = WasmEngine::instantiate_with(wasm, fuel).expect("engine.wasm instantiates");
        inst.push(t0.elapsed().as_nanos() as f64);
        black_box(&e.guest_memory_bytes());
    }

    let mut vm = WasmEngine::instantiate_with(wasm, fuel).expect("engine.wasm instantiates");

    for pass in 0..WARMUP_PASSES {
        for c in cases {
            let r = vm
                .evaluate(&c.request_json)
                .unwrap_or_else(|e| panic!("warmup pass {pass} failed on {}: {e}", c.slug));
            black_box(&r);
        }
    }

    let mut out = Vec::with_capacity(cases.len());
    let mut all = Vec::with_capacity(cases.len() * WASM_REPS);
    let mut agreed = 0usize;

    for c in cases {
        let mut reps = Vec::with_capacity(WASM_REPS);
        let mut decision = WireDecision::Error;
        for _ in 0..WASM_REPS {
            let t0 = Instant::now();
            let r = vm.evaluate(&c.request_json).unwrap_or_else(|e| panic!("{}: {e}", c.slug));
            let dt = t0.elapsed();
            reps.push(dt.as_nanos() as f64);
            decision = parse_decision(&r);
            black_box(&r);
        }
        all.extend_from_slice(&reps);
        let summary = Summary::of(&reps);
        let agrees = decision == c.expected;
        if agrees {
            agreed += 1;
        }
        out.push(CaseLatency {
            slug: c.slug.clone(),
            title: c.title.clone(),
            reps: WASM_REPS,
            unstable_within_case: stats::within_case_unstable(&summary),
            summary,
            outlier_across_cases: false,
            decision: decision_str(decision),
            expected_decision: decision_str(c.expected),
            agrees_with_expected: agrees,
        });
    }

    (
        finish_phase(label, what, WASM_REPS, out, all, agreed),
        InstantiationCost { fuel_metered: fuel, reps: INSTANTIATE_REPS, ns: Summary::of(&inst) },
    )
}

/// Reads the `decision` field back out of the guest's JSON response.
///
/// Deliberately a string match on the wire form rather than a
/// `serde_json::from_str::<engine::Response>`: the point of the wasm path
/// is that the host is not linked against the engine crate, so decoding
/// the answer with the engine's own types would quietly re-introduce the
/// coupling the path exists to avoid.
fn parse_decision(json: &str) -> WireDecision {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return WireDecision::Error,
    };
    match v.get("decision").and_then(|d| d.as_str()) {
        Some("Allow") => WireDecision::Allow,
        Some("Deny") => WireDecision::Deny,
        _ => WireDecision::Error,
    }
}

// ---------------------------------------------------------------------
// Load ramp
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct RampStep {
    concurrency: usize,
    /// Median over `RAMP_REPEATS` whole-ramp repeats.
    throughput_evals_per_sec: f64,
    throughput_per_repeat: Vec<f64>,
    /// Gate 3: `(max-min)/median` over `throughput_per_repeat`.
    throughput_relative_range: f64,
    unstable_across_repeats: bool,
    /// From the repeat whose throughput is the median one.
    latency_ns: Summary,
    latency_samples: usize,
    evaluations: u64,
    errors: u64,
    error_rate: f64,
    wall_seconds: f64,
    cpu_seconds: f64,
    /// `cpu_seconds / wall_seconds` — how many cores the step actually
    /// kept busy, against a ceiling of `nproc`.
    cores_busy: f64,
    rss_bytes_after: u64,
    peak_rss_bytes_after: u64,
    /// Wasm ramp only: total guest linear memory across all instances.
    #[serde(skip_serializing_if = "Option::is_none")]
    guest_memory_bytes_total: Option<usize>,
}

struct StepRun {
    throughput: f64,
    latencies: Vec<f64>,
    evaluations: u64,
    errors: u64,
    wall: f64,
    cpu: f64,
    guest_memory: Option<usize>,
}

/// Runs one concurrency step. `worker` is handed (thread index, stop
/// flag, barrier) and returns (evaluations, errors, latency samples,
/// optional guest-memory bytes for this worker).
fn run_step<W>(threads: usize, worker: W) -> StepRun
where
    W: Fn(usize, &AtomicBool, &Barrier) -> (u64, u64, Vec<f64>, Option<usize>) + Send + Sync + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    // `threads + 1`: the main thread waits on the same barrier, so the
    // clock starts only once every worker has finished its own private
    // setup (deserializing its own corpus copy, or instantiating its own
    // wasm Store) and is standing at the line.
    let barrier = Arc::new(Barrier::new(threads + 1));
    let worker = Arc::new(worker);

    let mut handles = Vec::with_capacity(threads);
    for t in 0..threads {
        let stop = Arc::clone(&stop);
        let barrier = Arc::clone(&barrier);
        let worker = Arc::clone(&worker);
        handles.push(std::thread::spawn(move || worker(t, &stop, &barrier)));
    }

    barrier.wait();
    let (cu0, cs0) = cpu_seconds();
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(RAMP_STEP_SECONDS));
    stop.store(true, Ordering::Relaxed);

    let mut evaluations = 0u64;
    let mut errors = 0u64;
    let mut latencies = Vec::new();
    let mut guest_memory: Option<usize> = None;
    for h in handles {
        let (n, e, mut l, g) = h.join().expect("a ramp worker panicked");
        evaluations += n;
        errors += e;
        latencies.append(&mut l);
        if let Some(g) = g {
            *guest_memory.get_or_insert(0) += g;
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    let (cu1, cs1) = cpu_seconds();

    StepRun {
        throughput: evaluations as f64 / wall,
        latencies,
        evaluations,
        errors,
        wall,
        cpu: (cu1 - cu0) + (cs1 - cs0),
        guest_memory,
    }
}

#[derive(Serialize)]
struct LoadPhase {
    path: &'static str,
    concurrency_means: &'static str,
    step_seconds: u64,
    ramp_repeats: usize,
    latency_sampling: String,
    nproc: usize,
    steps: Vec<RampStep>,
    resources_after: ResourceSnapshot,
}

fn assemble_step(concurrency: usize, runs: Vec<StepRun>) -> RampStep {
    let tputs: Vec<f64> = runs.iter().map(|r| r.throughput).collect();
    let sorted = stats::sorted(&tputs);
    let med = stats::median(&sorted);
    // The representative repeat is the one whose throughput IS the median
    // — so the latency distribution reported beside a throughput is the
    // one that actually accompanied it, not a blend of three runs.
    let rep = runs
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (a.throughput - med).abs().partial_cmp(&(b.throughput - med).abs()).expect("no NaN throughput")
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let r = &runs[rep];
    let rel = stats::relative_range(&tputs);

    RampStep {
        concurrency,
        throughput_evals_per_sec: med,
        throughput_per_repeat: tputs,
        throughput_relative_range: rel,
        unstable_across_repeats: rel > stats::RAMP_STEP_RELATIVE_RANGE_MAX,
        latency_ns: Summary::of(&r.latencies),
        latency_samples: r.latencies.len(),
        evaluations: r.evaluations,
        errors: r.errors,
        error_rate: if r.evaluations > 0 { r.errors as f64 / r.evaluations as f64 } else { 0.0 },
        wall_seconds: r.wall,
        cpu_seconds: r.cpu,
        cores_busy: if r.wall > 0.0 { r.cpu / r.wall } else { 0.0 },
        rss_bytes_after: rss_bytes(),
        peak_rss_bytes_after: peak_rss_bytes(),
        guest_memory_bytes_total: r.guest_memory,
    }
}

/// Baseline decision per case, computed single-threaded before any ramp,
/// so "error" under load means "this concurrent run disagreed with the
/// quiet run", not merely "returned Deny".
fn baseline(cases: &[Case]) -> HashMap<String, WireDecision> {
    cases.iter().map(|c| (c.slug.clone(), engine::evaluate_request(&c.request).decision)).collect()
}

fn phase_load_native(cases_json: Arc<String>, base: Arc<HashMap<String, WireDecision>>) -> LoadPhase {
    let mut steps = Vec::new();
    for &n in NATIVE_RAMP {
        let mut runs = Vec::with_capacity(RAMP_REPEATS);
        for _ in 0..RAMP_REPEATS {
            let cases_json = Arc::clone(&cases_json);
            let base = Arc::clone(&base);
            runs.push(run_step(n, move |_t, stop, barrier| {
                // Shared-nothing on purpose: each thread parses its OWN
                // copy of the corpus before the barrier. The engine is a
                // pure function of (policies, request, claims) with no
                // interior mutability and no global state, so sharing one
                // `Arc<Vec<Request>>` would also have been correct — this
                // just removes even the read-only cache line sharing, so
                // the scaling curve is the engine's, not an artifact of
                // 44 threads hammering one cache line.
                let mine = parse_cases_from(&cases_json);
                let mut n = 0u64;
                let mut pass = 0u64;
                let mut errs = 0u64;
                let mut lat = Vec::with_capacity(RAMP_SAMPLE_CAP);
                barrier.wait();
                while !stop.load(Ordering::Relaxed) {
                    let sample = pass.is_multiple_of(RAMP_SAMPLE_PASS_STRIDE) && lat.len() < RAMP_SAMPLE_CAP;
                    for c in &mine {
                        let t0 = Instant::now();
                        let r = engine::evaluate_request(black_box(&c.request));
                        let dt = t0.elapsed();
                        if base.get(&c.slug) != Some(&r.decision) {
                            errs += 1;
                        }
                        if sample {
                            lat.push(dt.as_nanos() as f64);
                        }
                        n += 1;
                    }
                    pass += 1;
                }
                (n, errs, lat, None)
            }));
        }
        let step = assemble_step(n, runs);
        eprintln!(
            "   native x{n}: {:.0} evals/s, p95 {:.0} ns, {} errors, {:.1} cores busy{}",
            step.throughput_evals_per_sec,
            step.latency_ns.p95,
            step.errors,
            step.cores_busy,
            if step.unstable_across_repeats { "  [UNSTABLE]" } else { "" }
        );
        steps.push(step);
    }

    LoadPhase {
        path: "native",
        concurrency_means: "N real OS threads, each evaluating its own private copy of the 68-case \
                            corpus in a loop. The engine is a pure, stateless function -- no shared \
                            mutable state, no lock, no connection pool, no interpreter instance -- so \
                            these are genuinely independent evaluations running in parallel, not \
                            interleaved work on one runtime. This is a structural property of the \
                            engine, not a tuning result: no other engine in this comparison can be \
                            driven this way in-process.",
        step_seconds: RAMP_STEP_SECONDS,
        ramp_repeats: RAMP_REPEATS,
        latency_sampling: format!(
            "every evaluation of every {RAMP_SAMPLE_PASS_STRIDE}th whole 68-case pass, per thread, capped at \
             {RAMP_SAMPLE_CAP} samples per thread (whole passes, not every Nth evaluation -- see \
             RAMP_SAMPLE_PASS_STRIDE's comment for the aliasing bug that rule replaces)"
        ),
        nproc: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        steps,
        resources_after: snapshot(),
    }
}

fn phase_load_wasm(
    cases_json: Arc<String>,
    wasm: Arc<Vec<u8>>,
    base: Arc<HashMap<String, WireDecision>>,
) -> LoadPhase {
    let mut steps = Vec::new();
    for &n in WASM_RAMP {
        let mut runs = Vec::with_capacity(RAMP_REPEATS);
        for _ in 0..RAMP_REPEATS {
            let cases_json = Arc::clone(&cases_json);
            let wasm = Arc::clone(&wasm);
            let base = Arc::clone(&base);
            runs.push(run_step(n, move |_t, stop, barrier| {
                let mine = parse_cases_from(&cases_json);
                // One whole independent instance per thread. `wasmi`'s
                // `Store` is not `Sync` and this is not a harness
                // workaround: a wasm instance owns a linear memory and a
                // guest heap, so concurrency on this path costs an
                // instance's worth of memory per thread. The step's
                // `guest_memory_bytes_total` is exactly that cost, made
                // visible.
                let mut vm = WasmEngine::instantiate_with(&wasm, true).expect("engine.wasm instantiates");
                let mut n = 0u64;
                let mut pass = 0u64;
                let mut errs = 0u64;
                let mut lat = Vec::with_capacity(RAMP_SAMPLE_CAP);
                barrier.wait();
                while !stop.load(Ordering::Relaxed) {
                    let sample = pass.is_multiple_of(RAMP_SAMPLE_PASS_STRIDE) && lat.len() < RAMP_SAMPLE_CAP;
                    for c in &mine {
                        let t0 = Instant::now();
                        let r = vm.evaluate(&c.request_json);
                        let dt = t0.elapsed();
                        match r {
                            Ok(body) if base.get(&c.slug) == Some(&parse_decision(&body)) => {}
                            _ => errs += 1,
                        }
                        if sample {
                            lat.push(dt.as_nanos() as f64);
                        }
                        n += 1;
                    }
                    pass += 1;
                }
                let mem = vm.guest_memory_bytes();
                (n, errs, lat, Some(mem))
            }));
        }
        let step = assemble_step(n, runs);
        eprintln!(
            "   wasm x{n}: {:.0} evals/s, p95 {:.0} ns, {} errors, {:.1} cores busy, guest mem {:.1} MiB{}",
            step.throughput_evals_per_sec,
            step.latency_ns.p95,
            step.errors,
            step.cores_busy,
            step.guest_memory_bytes_total.unwrap_or(0) as f64 / 1_048_576.0,
            if step.unstable_across_repeats { "  [UNSTABLE]" } else { "" }
        );
        steps.push(step);
    }

    LoadPhase {
        path: "wasm-abi-fuel-metered",
        concurrency_means: "N real OS threads, each owning its OWN wasmi Engine + Module + Store + \
                            engine.wasm instance and driving its own private copy of the 68-case \
                            corpus through the alloc/evaluate/dealloc ABI. A wasmi Store is not Sync, \
                            so unlike the native path this concurrency is not free: each thread pays a \
                            full instantiation and a full guest linear memory, both reported per step.",
        step_seconds: RAMP_STEP_SECONDS,
        ramp_repeats: RAMP_REPEATS,
        latency_sampling: format!(
            "every evaluation of every {RAMP_SAMPLE_PASS_STRIDE}th whole 68-case pass, per thread, capped at \
             {RAMP_SAMPLE_CAP} samples per thread (whole passes, not every Nth evaluation -- see \
             RAMP_SAMPLE_PASS_STRIDE's comment for the aliasing bug that rule replaces)"
        ),
        nproc: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        steps,
        resources_after: snapshot(),
    }
}

/// Re-parses the corpus from its raw JSON. Used by ramp workers so every
/// thread owns its own request objects.
fn parse_cases_from(raw: &str) -> Vec<Case> {
    let file: serde_json::Value = serde_json::from_str(raw).expect("latest-cases.json parses as JSON");
    let mut cases = Vec::new();
    for c in file["cases"].as_array().expect("`cases` is an array") {
        let Some(req_value) = c.get("request") else { continue };
        let request: Request = serde_json::from_value(req_value.clone()).expect("a fixture request deserializes");
        cases.push(Case {
            slug: c["slug"].as_str().unwrap_or("?").to_string(),
            title: String::new(),
            request_json: serde_json::to_string(&request).expect("a Request serializes"),
            request,
            expected: WireDecision::Error,
        });
    }
    cases
}

// ---------------------------------------------------------------------

#[derive(Serialize)]
struct Envelope<T> {
    schema: &'static str,
    phase: &'static str,
    corpus: String,
    corpus_cases: usize,
    engine_commit: String,
    resources_at_start: ResourceSnapshot,
    result: T,
}

fn engine_commit() -> String {
    std::env::var("ENGINE_COMMIT").unwrap_or_else(|_| "unknown".to_string())
}

fn write<T: Serialize>(out: &str, phase: &'static str, corpus_cases: usize, start: ResourceSnapshot, result: T) {
    let env = Envelope {
        schema: "ds-odrl-engine-rs/perf-bench@1",
        phase,
        corpus: cases_path().display().to_string(),
        corpus_cases,
        engine_commit: engine_commit(),
        resources_at_start: start,
        result,
    };
    std::fs::write(out, serde_json::to_string_pretty(&env).expect("envelope serializes"))
        .unwrap_or_else(|e| panic!("could not write {out}: {e}"));
    eprintln!("-> {out}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let phase = args.get(1).map(String::as_str).unwrap_or("");
    let out = args.get(2).cloned().unwrap_or_else(|| format!("{phase}.json"));

    let start = snapshot();
    let cases = load_cases();
    eprintln!("== phase `{phase}`: {} evaluable cases from {}", cases.len(), cases_path().display());

    match phase {
        "native" => {
            let p = phase_native(&cases);
            eprintln!(
                "   per-case median latency: median {:.0} ns, p95 {:.0} ns, max {:.0} ns; {} agree",
                p.per_case_median_ns.median, p.per_case_median_ns.p95, p.per_case_median_ns.max, p.agreement_with_expected
            );
            write(&out, "native", cases.len(), start, p);
        }
        "wasm" => {
            let wasm = std::fs::read(wasm_path())
                .unwrap_or_else(|e| panic!("could not read {}: {e}", wasm_path().display()));
            eprintln!("   engine.wasm: {} bytes ({})", wasm.len(), wasm_path().display());
            let (metered, inst_metered) = phase_wasm(&cases, &wasm, true);
            let (unmetered, inst_unmetered) = phase_wasm(&cases, &wasm, false);
            for p in [&metered, &unmetered] {
                eprintln!(
                    "   {}: per-case median {:.0} ns, p95 {:.0} ns; {} agree",
                    p.path, p.per_case_median_ns.median, p.per_case_median_ns.p95, p.agreement_with_expected
                );
            }
            #[derive(Serialize)]
            struct WasmResult {
                engine_wasm_bytes: usize,
                engine_wasm_path: String,
                instantiation: Vec<InstantiationCost>,
                fuel_metered: LatencyPhase,
                unmetered: LatencyPhase,
            }
            write(
                &out,
                "wasm",
                cases.len(),
                start,
                WasmResult {
                    engine_wasm_bytes: wasm.len(),
                    engine_wasm_path: wasm_path().display().to_string(),
                    instantiation: vec![inst_metered, inst_unmetered],
                    fuel_metered: metered,
                    unmetered,
                },
            );
        }
        "load-native" => {
            let raw = Arc::new(std::fs::read_to_string(cases_path()).expect("corpus readable"));
            let base = Arc::new(baseline(&cases));
            let p = phase_load_native(raw, base);
            write(&out, "load-native", cases.len(), start, p);
        }
        "load-wasm" => {
            let raw = Arc::new(std::fs::read_to_string(cases_path()).expect("corpus readable"));
            let wasm = Arc::new(
                std::fs::read(wasm_path()).unwrap_or_else(|e| panic!("could not read {}: {e}", wasm_path().display())),
            );
            let base = Arc::new(baseline(&cases));
            let p = phase_load_wasm(raw, wasm, base);
            write(&out, "load-wasm", cases.len(), start, p);
        }
        other => {
            eprintln!("unknown phase {other:?}");
            eprintln!("usage: perf-bench <native|wasm|load-native|load-wasm> <out.json>");
            std::process::exit(2);
        }
    }
}
