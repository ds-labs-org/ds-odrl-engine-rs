//! Summary statistics and the two explicitly-numeric stability gates this
//! bench applies. Nothing here ever drops a sample: a measurement that
//! trips a gate is reported with a flag beside it, never removed.

use serde::Serialize;

/// Nearest-rank percentile over an already-sorted slice.
///
/// Nearest-rank rather than linear interpolation on purpose: with 68 case
/// medians, an interpolated p99 is a weighted blend of the two slowest
/// cases and corresponds to no measurement that was ever taken. Every
/// percentile printed here is a real observed value.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[(rank - 1).min(sorted.len() - 1)]
}

pub fn median(sorted: &[f64]) -> f64 {
    percentile(sorted, 50.0)
}

pub fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

pub fn sorted(xs: &[f64]) -> Vec<f64> {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    v
}

/// The six-number summary every latency distribution in this bench is
/// reported as.
#[derive(Serialize, Clone)]
pub struct Summary {
    pub n: usize,
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    pub min: f64,
    pub max: f64,
    pub q1: f64,
    pub q3: f64,
    pub iqr: f64,
}

impl Summary {
    pub fn of(xs: &[f64]) -> Self {
        let s = sorted(xs);
        let q1 = percentile(&s, 25.0);
        let q3 = percentile(&s, 75.0);
        Summary {
            n: s.len(),
            mean: mean(&s),
            median: median(&s),
            p95: percentile(&s, 95.0),
            p99: percentile(&s, 99.0),
            min: *s.first().unwrap_or(&f64::NAN),
            max: *s.last().unwrap_or(&f64::NAN),
            q1,
            q3,
            iqr: q3 - q1,
        }
    }
}

// ---------------------------------------------------------------------
// The stability gates. Both are stated as numeric rules here, in one
// place, so the README and the result JSON quote the same constants the
// code applies rather than a prose paraphrase of them.
// ---------------------------------------------------------------------

/// Gate 1, WITHIN a case: a case whose own repeat set is noisy.
///
/// Rule: `IQR / median > 0.25`. That is a robust coefficient of variation
/// — the middle half of the repeats spreading wider than a quarter of the
/// median. Chosen over stdev/mean because a single scheduler preemption
/// in one of several hundred repeats blows up a standard deviation while
/// leaving the interquartile spread alone, and this gate is meant to fire
/// on a case that is *consistently* jittery, not on a case that was
/// interrupted once.
pub const WITHIN_CASE_RELATIVE_IQR_MAX: f64 = 0.25;

/// Gate 2, ACROSS cases: a case that is an outlier in the corpus.
///
/// Rule: Tukey's fence on the 68 per-case medians — flag any case whose
/// median falls outside `[Q1 - 1.5*IQR, Q3 + 1.5*IQR]`. This is the
/// standard boxplot fence, not a hand-picked cutoff, and it fires on
/// genuinely heavier fixtures as much as on noise; both are worth
/// surfacing, and neither is removed from the reported distribution.
pub const ACROSS_CASE_TUKEY_K: f64 = 1.5;

/// Gate 3, ACROSS ramp repeats: a load step that did not reproduce.
///
/// Rule: `(max - min) / median > 0.20` over the step's throughput across
/// the whole-ramp repeats. A step that trips it is still reported with
/// its median throughput, marked unstable.
pub const RAMP_STEP_RELATIVE_RANGE_MAX: f64 = 0.20;

pub fn within_case_unstable(s: &Summary) -> bool {
    s.median > 0.0 && s.iqr / s.median > WITHIN_CASE_RELATIVE_IQR_MAX
}

/// Returns `(low_fence, high_fence)` for the across-case gate.
pub fn tukey_fence(medians: &[f64]) -> (f64, f64) {
    let s = sorted(medians);
    let q1 = percentile(&s, 25.0);
    let q3 = percentile(&s, 75.0);
    let iqr = q3 - q1;
    (q1 - ACROSS_CASE_TUKEY_K * iqr, q3 + ACROSS_CASE_TUKEY_K * iqr)
}

pub fn relative_range(xs: &[f64]) -> f64 {
    let s = sorted(xs);
    let m = median(&s);
    if m <= 0.0 || s.is_empty() {
        return f64::NAN;
    }
    (s[s.len() - 1] - s[0]) / m
}
