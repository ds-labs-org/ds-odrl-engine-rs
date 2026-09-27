//! `N3Enforcer::explain` calls the reasoner twice per request (round 1 and
//! round 2 of `ODRLEngineMultipleSteps`'s semantics), and both calls send a
//! large *static* rule file (`rules::ROUND1`/`ROUND2`, 2,694/89 lines)
//! concatenated with the small, per-request N3 data as one opaque string to
//! `Reasoner::derive`. For `EyeronLib` that means `eyeron::reason(&str)`
//! re-parses the same unchanging rule text, and rebuilds the reasoner's
//! rule-agenda index from scratch, on every single enforcement call -- the
//! same class of bug as eyeron's own `--stream-messages` re-parsing a static
//! rule program per message (fixed upstream in
//! ds-labs-org/eyeron#5/eyereasoner/eyeron#22).
//!
//! `Reasoner::derive_with_rules(data, rules)` lets an implementation that can
//! exploit the split (`EyeronLib`) cache a prepared reasoner per distinct
//! `rules` text, while implementations that can't (`CommandReasoner`, which
//! has to write one file per call regardless) get the exact old behavior via
//! the trait's default method.
//!
//! Timing is compared within the same run, not against an absolute bound:
//! calling `derive_with_rules` `CALLS` times with the *same* `rules::ROUND1`
//! text should be substantially cheaper than `CALLS` calls where the rules
//! text differs every time (a harmless per-call comment appended, so the
//! rule *semantics* -- and therefore the derived output -- stay identical,
//! only the text differs, forcing a cache miss every time).
//!
//! The two conditions are interleaved call-by-call, not run as two separate
//! blocks. `cargo test` runs each integration test file as its own process
//! and can run several of those processes concurrently (this crate's own
//! `differential.rs` is one), so a contention burst can land entirely inside
//! one block and invert the comparison even though neither side is actually
//! slower -- caught for real running this suite as `cargo test -p
//! n3-enforcer` (368ms "same" vs 271ms "distinct", the wrong way round) right
//! after the same test passed reliably run alone. Interleaving means any
//! contention burst, wherever it lands in time, touches both conditions in
//! roughly the same proportion, so the *difference* stays meaningful even
//! when neither absolute number does.

use n3_enforcer::rules;
use n3_enforcer::{EyeronLib, Reasoner};

const DATA: &str = "@prefix : <http://example.org/> .\n:alice :read :x .\n";

#[test]
fn eyeron_lib_reuses_a_parsed_rule_set_across_calls_with_the_same_rules_text() {
    const CALLS: usize = 30;
    let lib = EyeronLib;

    // Warm up: make sure both conditions below pay for process/allocator
    // warmup equally, not as part of the measured difference.
    lib.derive_with_rules(DATA, rules::ROUND1).expect("warmup call");

    // Same rule *set*, different rule *text* every call (a per-call unique
    // trailing comment), so a text-keyed cache cannot help here: this is the
    // "no caching possible" baseline, not a strawman -- it exercises the
    // exact same parse+reason work the fix is supposed to amortize away.
    let distinct_rules: Vec<String> =
        (0..CALLS).map(|i| format!("{}\n# cache-buster {i}\n", rules::ROUND1)).collect();

    let mut same_rules_elapsed = std::time::Duration::ZERO;
    let mut distinct_rules_elapsed = std::time::Duration::ZERO;
    let mut same_output = None;
    let mut distinct_output = None;
    for r in &distinct_rules {
        let started = std::time::Instant::now();
        let out = lib.derive_with_rules(DATA, rules::ROUND1).expect("same-rules call");
        same_rules_elapsed += started.elapsed();
        same_output.get_or_insert(out);

        let started = std::time::Instant::now();
        let out = lib.derive_with_rules(DATA, r).expect("distinct-rules call");
        distinct_rules_elapsed += started.elapsed();
        distinct_output.get_or_insert(out);
    }

    // Caching must not change what gets derived.
    assert_eq!(same_output, distinct_output, "a cache-buster comment must not change the derived triples");

    // Measured repeatedly on this rule set: caching consistently saves
    // ~25-30% (e.g. 234ms vs 311ms for 30 calls), not the ~2x eyeron's own
    // library-level PreparedReasoner microbenchmark showed. That is real,
    // not a weaker fix: `rules::ROUND1` has ~40 rules, several backward, and
    // even a single trivial data triple pays real search cost against them
    // on every call regardless of caching -- unlike that microbenchmark's
    // 2,000 rules that never matched anything, which isolated pure clone
    // cost with ~zero reasoning work. Parsing+indexing a 2,694-line rule
    // file is a real fraction of the per-call cost here, not almost all of
    // it. A 10% bound is comfortably inside the ~25-30% actually observed,
    // leaving headroom for noise while still catching a regression back to
    // "no caching at all" (which measures ~0% difference, not ~25-30%).
    assert!(
        same_rules_elapsed.as_nanos().saturating_mul(10) < distinct_rules_elapsed.as_nanos().saturating_mul(9),
        "{CALLS} calls with the same rules text took {same_rules_elapsed:?}, \
         {CALLS} calls with distinct (but semantically identical) rules text took \
         {distinct_rules_elapsed:?}: EyeronLib should reuse a prepared reasoner for \
         repeated identical rules text, not re-parse/re-index it every call"
    );
}

#[test]
fn command_reasoner_style_default_still_concatenates_data_and_rules() {
    // A `Reasoner` that only implements `derive` (the trait's original,
    // single method -- what `CommandReasoner` does) must see `derive_with_rules`
    // behave exactly like the old call sites' `derive(&format!("{data}\n{rules}"))`,
    // unchanged, via the trait's default method.
    struct EchoReasoner;
    impl Reasoner for EchoReasoner {
        fn derive(&self, n3: &str) -> Result<String, n3_enforcer::ReasonerError> {
            Ok(n3.to_string())
        }
    }
    let echo = EchoReasoner;
    let out = echo.derive_with_rules("DATA", "RULES").unwrap();
    assert_eq!(out, "DATA\nRULES");
}
