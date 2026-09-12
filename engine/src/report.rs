//! The "detailed evaluation" API: a second, richer answer over the exact
//! same `(Policy, Claims, ResolvedConfig, requested_action, requested_target,
//! asset_collections)` input `decision::decide` already consumes, shaped to
//! mirror the `report:` vocabulary (`ComplianceReportModel.ttl`'s
//! `PermissionReport`/`ProhibitionReport`/`DutyReport`/`ConstraintReport`
//! family) rather than the coarse `Allow`/`Deny`/`Error` the wire contract's
//! `Response` reports.
//!
//! Pure data. No behavior lives here — derivation logic lives in
//! `decision.rs` (`derive_detailed_rule_reports`) and `constraint.rs`
//! (`Constraint::evaluate_report`), precisely because that is where every
//! predicate it needs is already `pub(crate)`. Every type here is
//! `Debug + Clone + PartialEq + Eq`; none of them cross the JSON wire
//! contract (`wire::Request`/`wire::Response` are untouched by this
//! module), so none derive `Serialize`/`Deserialize` — the same convention
//! this crate already uses for its other purely-diagnostic, non-wire output
//! types (`decision::DecisionOutcome`, `decision::UnresolvedDuty`,
//! `decision::UnrecognizedAction`).

use crate::constraint::Operator;
use crate::decision::{ConflictStrategy, DutyAttachment, UnrecognizedAction};

/// `report:activationState` — `report:Active` | `report:Inactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    Active,
    Inactive,
}

/// `report:attemptState` — `report:Attempted` | `report:NotAttempted`.
/// The real vocabulary's declared domain for this property is
/// `PermissionReport` and `ProhibitionReport` only — `DetailedDutyReport`
/// structurally carries no field of this type at all, so misuse on a duty
/// is a compile error rather than a runtime convention to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptState {
    Attempted,
    NotAttempted,
}

/// `report:performanceState` — `report:Performed` | `report:Unperformed` |
/// `report:Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceState {
    Performed,
    Unperformed,
    Unknown,
}

/// `report:deonticState` — `report:NonSet` | `report:Violated` |
/// `report:Fulfilled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeonticState {
    NonSet,
    Violated,
    Fulfilled,
}

/// `report:satisfactionState` — `report:Satisfied` | `report:Unsatisfied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatisfactionState {
    Satisfied,
    Unsatisfied,
}

impl SatisfactionState {
    pub(crate) fn of(satisfied: bool) -> Self {
        if satisfied {
            SatisfactionState::Satisfied
        } else {
            SatisfactionState::Unsatisfied
        }
    }
}

/// One node of a `Constraint` tree. Mirrors `Constraint`'s own recursive
/// shape 1:1 (atomic vs. one of the four logical kinds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintNode {
    Atomic { left_operand: String, operator: Operator, right_operand: String },
    And,
    Or,
    Xone,
    AndSequence,
}

/// One `report:ConstraintReport`-shaped node, including internal logical
/// nodes. `children` is empty for `ConstraintNode::Atomic`; for a logical
/// node it holds one `DetailedConstraintReport` per child of the source
/// `Constraint`, in source order.
///
/// **Disclosed, not a confirmed vocabulary edge**: whether
/// `ComplianceReportModel.ttl` actually defines a `ConstraintReport ->
/// ConstraintReport` nesting edge is unverified by this implementation;
/// this tree is an information-preserving Rust-side convenience a caller
/// may flatten to leaves-only `report:premiseReport` entries when emitting
/// RDF, or keep as a project-local extension property. Verify against the
/// live vocabulary before an RDF-emitting side is built; the Rust shape
/// here does not need to change either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedConstraintReport {
    pub node: ConstraintNode,
    pub satisfaction_state: SatisfactionState,
    pub children: Vec<DetailedConstraintReport>,
}

/// `report:ActionReport`. Never emitted for a `DetailedDutyReport` (see
/// that type's own doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedActionReport {
    pub satisfaction_state: SatisfactionState,
    pub covers: bool,
    pub refinement: Option<DetailedConstraintReport>,
}

/// `report:TargetReport`. Never emitted for a `DetailedDutyReport` — a
/// duty's own `odrl:target` is carried descriptively and never evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedTargetReport {
    pub satisfaction_state: SatisfactionState,
}

/// One `report:PremiseReport`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailedPremiseReport {
    Action(DetailedActionReport),
    Target(DetailedTargetReport),
    Constraint(DetailedConstraintReport),
}

/// `report:PermissionReport`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedPermissionReport {
    pub rule_index: usize,
    /// **Hard rule, structurally guaranteed by construction**: whenever
    /// `condition_report` is `Some(d)` with `d.deontic_state ==
    /// DeonticState::Violated`, `activation_state` here is always
    /// `Inactive`. Both fields are derived in `decision::
    /// derive_detailed_rule_reports` from the same `duty_gate_violated`
    /// boolean, computed exactly once — this cannot diverge by
    /// construction. See that function's own doc comment for the one
    /// deliberate, disclosed place where this makes `activation_state ==
    /// Inactive` while the *policy's* overall `Response.decision` is still
    /// `Allow` (`DutyMode::Advise`).
    pub activation_state: ActivationState,
    pub attempt_state: AttemptState,
    pub performance_state: PerformanceState,
    pub deontic_state: DeonticState,
    pub premise_reports: Vec<DetailedPremiseReport>,
    /// `report:conditionReport` — present iff `Rule::duty` is non-empty.
    /// Only the first `odrl:duty` entry is linked (the real vocabulary's
    /// range for this property is a single `RuleReport`, not a list) —
    /// every `duty[j]` for `j >= 1` still gets its own sibling
    /// `DetailedRuleReport::Duty` in the owning policy's `rule_reports`,
    /// simply not cross-linked here. Boxed because a duty's own
    /// consequence chain can, in principle, recurse.
    pub condition_report: Option<Box<DetailedDutyReport>>,
}

/// `report:ProhibitionReport`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedProhibitionReport {
    pub rule_index: usize,
    pub activation_state: ActivationState,
    pub attempt_state: AttemptState,
    pub performance_state: PerformanceState,
    pub deontic_state: DeonticState,
    pub premise_reports: Vec<DetailedPremiseReport>,
}

/// `report:DutyReport`. Deliberately carries no `attempt_state` field — the
/// real vocabulary's declared domain for `report:attemptState` is
/// `PermissionReport`/`ProhibitionReport` only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedDutyReport {
    /// Which of the three ODRL attachment points this duty hangs off.
    pub attachment: DutyAttachment,
    pub duty_index: usize,
    /// 0 for the attached duty itself; N for the Nth `odrl:consequence`
    /// hop. A duty reached via a consequence hop is reported as its own
    /// independent sibling entry in `DetailedPolicyReport::rule_reports` —
    /// never nested inside the duty it superseded (neither
    /// `report:conditionReport` nor an equivalent link exists for
    /// duty-of-a-duty or prohibition-remedy nesting in the real
    /// vocabulary).
    pub consequence_depth: usize,
    pub activation_state: ActivationState,
    pub performance_state: PerformanceState,
    pub deontic_state: DeonticState,
    pub premise_reports: Vec<DetailedPremiseReport>,
}

/// `report:RuleReport` — the sum type over the three real subclasses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailedRuleReport {
    Permission(DetailedPermissionReport),
    Prohibition(DetailedProhibitionReport),
    Duty(DetailedDutyReport),
}

/// `report:policyRequest`'s target: a tiny `odrl:Request`-typed policy
/// mirroring what was asked. Named distinctly from `wire::Request` (the
/// wire envelope) so the two are never confused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedPolicyRequest {
    pub requested_action: String,
    pub requested_target: String,
}

/// `report:PolicyReport`. `policy_id` correlates back to the caller's own
/// `wire::WirePolicy::id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedPolicyReport {
    pub policy_id: String,
    pub policy_request: DetailedPolicyRequest,
    pub rule_reports: Vec<DetailedRuleReport>,
    /// **Disclosed gap, not a vocabulary field.** Populated instead of
    /// `rule_reports` (left empty) when this policy's own `decide` call hit
    /// the unrecognized-action short-circuit (`Decision::Error`) before any
    /// rule was evaluated. There is no real `report:` class for "we refused
    /// to evaluate this policy at all"; fabricating per-rule states for
    /// rules never reached would be worse than leaving this honestly empty.
    pub evaluation_error: Option<UnrecognizedAction>,
    /// This policy's own declared `odrl:conflict` value, exactly as merged
    /// by `wire::resolve_inherit_from`, *before* any forcing.
    pub declared_conflict: ConflictStrategy,
    /// `true` iff `wire::resolve_inherit_from` forced this policy's
    /// effective conflict strategy to `Invalid` because an inherited chain
    /// carried more than one distinct declared `odrl:conflict` value —
    /// distinct from the policy itself having declared `invalid` (in which
    /// case `declared_conflict == ConflictStrategy::Invalid` and this field
    /// is `false`). Every per-rule state in `rule_reports` was derived
    /// using the *effective* (possibly-forced) strategy, which equals
    /// `declared_conflict` unless this field is `true`, in which case the
    /// effective strategy used was `ConflictStrategy::Invalid` regardless
    /// of `declared_conflict`.
    pub conflict_forced_invalid: bool,
}

/// One policy the request named that produced no `DetailedPolicyReport` at
/// all, because party-role scoping (`wire::party_role_mismatch`) removed it
/// from the applicable set before `decide` ever ran — the same treatment
/// the coarse path already gives it. Deliberately not `report:`-shaped:
/// there is no `report:PolicyReport` for a policy addressed to somebody
/// else. Exists purely as a diagnostic for a form-based editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPolicyReport {
    pub policy_id: String,
    pub reason: String,
}

/// The full detailed evaluation for one `wire::Request`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedEvaluation {
    pub dataset_id: String,
    pub requested_action: String,
    pub policy_reports: Vec<DetailedPolicyReport>,
    pub skipped_policies: Vec<SkippedPolicyReport>,
}
