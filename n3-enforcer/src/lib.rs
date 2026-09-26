//! **DRAFT.** A reasoner-backed ODRL enforcer for `ds-odrl-engine-rs`.
//!
//! The native `engine` crate is a pure, stateless `(policy, claims) ->
//! decision` evaluator with no RDF layer and no reasoner. This crate is a
//! *second implementation of the same wire contract*: it translates an
//! `engine::Request` into RDF (`rdf.rs`), runs SolidLabResearch's
//! ODRL-Evaluator N3 rule set on an N3 reasoner in the same two rounds
//! `ODRLEngineMultipleSteps` uses, and reduces the resulting compliance
//! report to Allow/Deny (`reduce.rs`) by the rule
//! `compliance-runner/src/ground_truth.rs` already applies to the suite's
//! expected reports.
//!
//! It is deliberately **not** on the hot path and does not replace
//! `engine::evaluate_request`. Its jobs: (1) an independent second opinion
//! for differential testing (`tests/differential.rs`); (2) a way to run
//! the Evaluator's declarative semantics inside a host that already has an
//! N3 reasoner; (3) a place to measure what a reasoner-based path costs.
//!
//! Anything the rule set cannot express is reported as
//! [`EnforcerError::Unsupported`], never guessed: a wrong Allow from a
//! translation gap is worse than a refusal.

mod rdf;
mod reduce;
mod rules;

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use engine::{Request, Response};

pub use rdf::{to_n3, Unsupported};
pub use reduce::{reduce, to_response, ReportSummary};

/// An N3 reasoner: takes one N3 document (rules and data concatenated) and
/// returns **only the newly derived triples** as N3/Turtle text — the
/// contract of `eye --quiet --nope --pass-only-new` and of
/// `eyeron::reason`, and exactly what `ODRL-Evaluator`'s `Reasoner.run`
/// relies on.
pub trait Reasoner {
    fn derive(&self, n3: &str) -> Result<String, ReasonerError>;
}

#[derive(Debug)]
pub struct ReasonerError(pub String);

impl std::fmt::Display for ReasonerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "reasoner failed: {}", self.0)
    }
}

impl std::error::Error for ReasonerError {}

/// Runs an external reasoner binary on a temp file, the way
/// `ODRL-Evaluator`'s `EyeReasoner` does. Works with `eye`
/// (`["--quiet", "--nope", "--pass-only-new"]`) and with the `eyeron`
/// binary (no arguments).
pub struct CommandReasoner {
    program: PathBuf,
    args: Vec<String>,
}

impl CommandReasoner {
    pub fn new(program: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    pub fn eye(program: impl Into<PathBuf>) -> Self {
        Self::new(program, &["--quiet", "--nope", "--pass-only-new"])
    }

    pub fn eyeron(program: impl Into<PathBuf>) -> Self {
        Self::new(program, &[])
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl Reasoner for CommandReasoner {
    fn derive(&self, n3: &str) -> Result<String, ReasonerError> {
        let path = std::env::temp_dir().join(format!(
            "n3-enforcer-{}-{}.n3",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, n3).map_err(|e| ReasonerError(e.to_string()))?;
        let out = Command::new(&self.program).args(&self.args).arg(&path).output();
        let _ = std::fs::remove_file(&path);
        let out = out.map_err(|e| ReasonerError(format!("{}: {e}", self.program.display())))?;
        if !out.status.success() {
            return Err(ReasonerError(String::from_utf8_lossy(&out.stderr).into_owned()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// In-process eyeron (`--features eyeron`).
pub struct EyeronLib;

impl Reasoner for EyeronLib {
    fn derive(&self, n3: &str) -> Result<String, ReasonerError> {
        eyeron::reason(n3).map_err(|e| ReasonerError(e.to_string()))
    }
}

#[derive(Debug)]
pub enum EnforcerError {
    /// The request uses something the N3 rule set (or this translation)
    /// cannot express. The caller should fall back to the native engine.
    Unsupported(Unsupported),
    Reasoner(ReasonerError),
    /// The reasoner's output was not parseable Turtle/N3.
    BadOutput(String),
}

impl std::fmt::Display for EnforcerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(u) => write!(f, "unsupported: {u}"),
            Self::Reasoner(e) => e.fmt(f),
            Self::BadOutput(e) => write!(f, "unparseable reasoner output: {e}"),
        }
    }
}

impl std::error::Error for EnforcerError {}

/// Which implementation produced a decision from [`Enforcer::enforce`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Path {
    /// Decided by the N3 rules on the reasoner.
    Reasoner,
    /// Decided by `engine::evaluate_request`; the string says why the
    /// reasoner path did not decide (`Unsupported`, a contradictory report,
    /// or a reasoner failure).
    Native(String),
}

/// The integration point: enforce a request with the reasoner where the
/// Evaluator's rules can express it, and with the native engine otherwise.
/// A rule reported both `Active` and `Inactive` is treated as a failed
/// reasoner run, never as an answer.
pub struct Enforcer<R: Reasoner> {
    n3: N3Enforcer<R>,
}

impl<R: Reasoner> Enforcer<R> {
    pub fn new(reasoner: R) -> Self {
        Self { n3: N3Enforcer::new(reasoner) }
    }

    pub fn enforce(&self, req: &Request) -> (Response, Path) {
        match self.n3.report(req) {
            Ok((_, summary)) if summary.contradictory.is_empty() => {
                (to_response(&req.dataset_id, &summary), Path::Reasoner)
            }
            Ok(_) => (engine::evaluate_request(req), Path::Native("contradictory report".into())),
            Err(e) => (engine::evaluate_request(req), Path::Native(e.to_string())),
        }
    }
}

impl Enforcer<EyeronLib> {
    /// 100% Rust: the in-process eyeron reasoner, no subprocess, no Node.
    pub fn eyeron() -> Self {
        Self::new(EyeronLib)
    }
}

pub struct N3Enforcer<R: Reasoner> {
    reasoner: R,
}

impl<R: Reasoner> N3Enforcer<R> {
    pub fn new(reasoner: R) -> Self {
        Self { reasoner }
    }

    /// The reasoner-backed counterpart of `engine::evaluate_request`.
    pub fn evaluate_request(&self, req: &Request) -> Result<Response, EnforcerError> {
        let (report, summary) = self.report(req)?;
        let _ = report;
        Ok(to_response(&req.dataset_id, &summary))
    }

    /// Runs both rounds and returns the raw derived triples (Turtle) plus
    /// the parsed summary. Round 2 sees round 1's output as data, mirroring
    /// `ODRLEngineMultipleSteps`, because activation is non-monotonic
    /// (`log:collectAllIn` counts) and must run after every premise report
    /// is complete.
    pub fn report(&self, req: &Request) -> Result<(String, ReportSummary), EnforcerError> {
        let e = self.explain(req)?;
        Ok((e.report_turtle, e.summary))
    }

    /// Like [`report`](Self::report), but also returns the N3 generated from
    /// the request, for display and debugging.
    pub fn explain(&self, req: &Request) -> Result<Explanation, EnforcerError> {
        let n3_input = rdf::to_n3(req).map_err(EnforcerError::Unsupported)?;
        let round1 = self
            .reasoner
            .derive(&format!("{n3_input}\n{}", rules::ROUND1))
            .map_err(EnforcerError::Reasoner)?;
        let round2 = self
            .reasoner
            .derive(&format!("{n3_input}\n{round1}\n{}", rules::ROUND2))
            .map_err(EnforcerError::Reasoner)?;
        let report_turtle = format!("{round1}\n{round2}");
        let summary = reduce(&report_turtle).map_err(EnforcerError::BadOutput)?;
        Ok(Explanation { n3_input, report_turtle, summary })
    }
}

/// Everything one reasoner run produced for a request.
#[derive(Debug)]
pub struct Explanation {
    /// The policy, request and state of the world as N3 (the reasoner's data).
    pub n3_input: String,
    /// The newly derived compliance-report triples of both rounds (Turtle).
    pub report_turtle: String,
    pub summary: ReportSummary,
}
