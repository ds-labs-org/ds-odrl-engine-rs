//! Pure, browser-free logic for the `/full-compliance` page: deciding, per
//! probe and then per vocabulary row, whether this engine's real live
//! behaviour meets **full ODRL 2.2** — not whether it matches what this
//! study documents about itself.
//!
//! That distinction is the entire reason this module exists beside
//! `coverage_catalog.rs` rather than inside it. `/coverage` judges a live
//! response against `Expectation`, which records what this engine *does*,
//! honestly-documented gaps included; a probe failing there means the
//! **documentation** is wrong. This module judges against
//! [`crate::coverage_catalog::Ideal`], which records what the **spec**
//! requires; a probe falling short here means the engine is right about
//! itself and still short of ODRL 2.2. The two axes can and do disagree,
//! and neither is a defect in the other.
//!
//! Consequently **nothing in this module reuses `/coverage`'s
//! Agreed/Disagreed/Verified/Contradicted vocabulary.** Those four words
//! already mean matches-the-documentation, and reusing them for a
//! different question is exactly the confusion this page was created to
//! avoid. The words here are *meets full spec*, *falls short*, and
//! *structural gap*.
//!
//! **`Ideal::is_some()` is the falls-short judgment, and not a decision
//! comparison.** That is load-bearing rather than a stylistic preference:
//! `duty-consequence-itself-unresolved`'s ideal decision *equals* its
//! current one (both `Allow`) and what falls short there is the reported
//! `duties` list — a comparison of decisions would score that probe fully
//! compliant. See `coverage_catalog::Ideal`'s own doc comment.
//!
//! Ungated for the same reason `coverage_catalog.rs` and
//! `coverage_state.rs` are: `cargo test --workspace` is a native build, so
//! behind a `#[cfg(target_arch = "wasm32")]` gate this module's tests —
//! which are the only thing standing between a scope-rule regression and a
//! page that renders "everything meets ODRL 2.2 in full" — would silently
//! never compile, let alone run.

use crate::coverage_catalog::{CatalogRow, Category, CoverageFile, Ideal, ProbeFixture, ProbeOutcome, ProbeStatus};

/// A row this page renders at all. The seven rows documented `OutOfScope`
/// are excluded outright — they name profile-extension points and
/// `odrl:hasPolicy`, which sit outside the wire contract entirely. Being
/// outside the contract is not the same thing as falling short of the
/// spec, and scoring them either way would be a category error.
pub fn is_in_scope(row: &CatalogRow) -> bool {
  row.status != "OutOfScope"
}

/// What this browser made of one probe, measured against full ODRL 2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecOutcome {
  /// The engine's live answer is the answer a fully spec-compliant engine
  /// would give. True of every probe on every `Implemented` row, and of
  /// most probes on most `Partial` rows: a row is partial for one specific
  /// reason, and its other probes usually demonstrate exactly right
  /// behaviour.
  MeetsFullSpec,
  /// The catalog records a researched [`Ideal`] for this probe, so a fully
  /// spec-compliant engine would answer differently from what this one
  /// does — and does correctly, by its own documentation.
  FallsShort,
  /// This page cannot say. Either the probe could not be judged at all
  /// (an ABI failure, a response outside Section 5.2's envelope), or the
  /// engine departed from its own documented behaviour, which invalidates
  /// the premise the researched ideal was written against. Never counted
  /// as meeting the spec *or* as falling short of it.
  Undetermined,
}

impl SpecOutcome {
  pub fn label(self) -> &'static str {
    match self {
      SpecOutcome::MeetsFullSpec => "meets full spec",
      SpecOutcome::FallsShort => "falls short",
      SpecOutcome::Undetermined => "undetermined",
    }
  }
}

/// What this run made of one vocabulary row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSpecVerdict {
  /// Every probe on the row reached the spec-ideal answer.
  MeetsFullSpec,
  /// At least one probe demonstrates a shortfall, with a live answer to
  /// point at and an ODRL 2.2 clause that settles it.
  FallsShort,
  /// The row falls short *by construction*: the wire contract cannot even
  /// pose the question, so there is no probe and no wrong answer — only a
  /// missing capability. Exactly one row (`party.collections`) is in this
  /// state, and it carries the wire-contract addition full support needs
  /// instead of a probe.
  StructuralGap,
  /// No shortfall demonstrated, but at least one probe could not be
  /// judged. Deliberately out-ranked by `FallsShort`, mirroring
  /// `/coverage`'s own Contradicted-over-Inconclusive precedence: a real
  /// finding must not be buried under an unjudgeable probe beside it.
  Undetermined,
}

impl RowSpecVerdict {
  pub fn label(self) -> &'static str {
    match self {
      RowSpecVerdict::MeetsFullSpec => "Meets full spec",
      RowSpecVerdict::FallsShort => "Falls short",
      RowSpecVerdict::StructuralGap => "Falls short (structural)",
      RowSpecVerdict::Undetermined => "Undetermined",
    }
  }

  /// Whether this verdict counts against full ODRL 2.2 compliance. Both
  /// shortfall shapes do; `Undetermined` deliberately does not, because a
  /// probe this browser could not judge must not be able to turn a row
  /// green *or* score it as a finding.
  pub fn is_short_of_spec(self) -> bool {
    matches!(self, RowSpecVerdict::FallsShort | RowSpecVerdict::StructuralGap)
  }
}

/// One probe, judged on this page's axis. Carries the engine's live answer
/// beside the spec-ideal one so a reader can see both without expanding
/// anything on the other page.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeJudgment {
  pub id: String,
  pub title: String,
  pub kind: String,
  pub asserts: String,
  pub outcome: SpecOutcome,
  /// What this study documents the engine as answering — `/coverage`'s
  /// target, shown here as the thing the ideal departs from.
  pub documented_decision: String,
  /// The researched full-compliance target, present exactly when this
  /// probe falls short.
  pub ideal: Option<Ideal>,
  /// What `engine.wasm` answered in this browser, just now.
  pub observed_decision: Option<String>,
  pub observed_reason: Option<String>,
  /// Why this page declined to judge, when it did.
  pub undetermined_because: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowJudgment {
  pub row: CatalogRow,
  pub verdict: RowSpecVerdict,
  pub probes: Vec<ProbeJudgment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FullComplianceReport {
  pub generated_by: String,
  pub spec: String,
  pub source_analysis: String,
  pub categories: Vec<Category>,
  /// In-scope rows only, in the catalog's own order.
  pub rows: Vec<RowJudgment>,

  // --- row axis -----------------------------------------------------
  /// Rows this page judges: the catalog's 52 minus the `OutOfScope` ones.
  pub rows_in_scope: u64,
  pub rows_excluded: u64,
  pub rows_meets: u64,
  /// Rows with at least one falls-short probe. Does **not** include the
  /// structural gap — see [`Self::rows_short_of_spec`].
  pub rows_falls_short: u64,
  pub rows_structural_gap: u64,
  pub rows_undetermined: u64,
  /// The documented statuses of the in-scope rows, for the "already
  /// documented as such" framing.
  pub implemented_rows: u64,
  pub partial_rows: u64,
  pub not_implemented_rows: u64,

  // --- probe axis ---------------------------------------------------
  /// Distinct probes named by at least one in-scope row: what this page
  /// judges. Smaller than the catalog's total, because some probes are
  /// named only by rows this page excludes.
  pub probes_judged: u64,
  pub probes_in_catalog: u64,
  pub probes_meets: u64,
  /// The probes that actually distinguish this page's question from
  /// `/coverage`'s. Every other judged probe reaches the spec-ideal answer
  /// already, so quoting a bare `N/136` would imply a change of meaning
  /// that most of the catalog did not undergo.
  pub probes_falls_short: u64,
  pub probes_undetermined: u64,

  /// Wall-clock milliseconds the replay actually took in this browser.
  pub elapsed_ms: f64,
  pub engine_bytes: usize,
}

impl FullComplianceReport {
  pub fn rows_short_of_spec(&self) -> u64 {
    self.rows_falls_short + self.rows_structural_gap
  }

  pub fn short_rows(&self) -> Vec<&RowJudgment> {
    self.rows.iter().filter(|row| row.verdict.is_short_of_spec()).collect()
  }
}

/// One judgment on this page that the spec genuinely admits more than one
/// reading of — named, with the reading that shipped and the one that was
/// rejected, so a reader can find them without archaeology.
///
/// This exists because the shortfalls this page reports are *judgments*,
/// not measurements. The live half is measured: the engine really is
/// driven and its real answer really is shown. The ideal half is a reading
/// of two W3C documents, and on the entries below that reading is
/// contested rather than obvious. A page that presented all fifteen
/// shortfalls with equal confidence would be overclaiming on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContestedReading {
  pub row: &'static str,
  pub probe: &'static str,
  /// The reading this page ships, and why.
  pub shipped: &'static str,
  /// The reading it rejects, stated in its own strongest form — including,
  /// where it applies, what the row's verdict would become under it.
  pub alternative: &'static str,
}

/// The sign-off list from the research pass, carried onto the page
/// verbatim in substance.
///
/// Fifteen entries over fourteen distinct probes: `pc-kind-nonsense` is
/// listed under both in-scope rows that name it (deliberately judged
/// identically in both — a shared probe must not get two different
/// ideals), and `other.uid` contributes two probes turning on one shared
/// scope question.
///
/// Note that a contested entry is **not** the same thing as a falls-short
/// entry. Five of these fourteen probes carry an `Ideal` and so fall short;
/// the other nine were judged ideal == current, meaning what is contested
/// there is whether they should have fallen short at all.
pub const CONTESTED_READINGS: &[ContestedReading] = &[
  ContestedReading {
    row: "actions.included-in-transitive",
    probe: "act-includedin-undeclared-gap",
    shipped: "Ideal Allow, so the row falls short. The bare tokens use/play/display are the odrl: terms \
              (that is what the JSON-LD context makes them), and odrl:play odrl:includedIn odrl:use is a \
              normative Vocabulary assertion, so the chain closes through the undeclared intermediate.",
    alternative: "Read the profile as closed -- IM Terminology calls the Common Vocabulary a set of terms \
                  that \"MAY be re-used by ODRL Profiles\" -- and the tokens are opaque, nothing external \
                  closes the chain, and the ideal is the current Deny. Under that reading this row meets \
                  full spec today. It is the single highest-leverage judgment on the page.",
  },
  ContestedReading {
    row: "left-operands.spatial",
    probe: "lo-spatial-no-containment",
    shipped: "Ideal Deny, equal to current, so this probe demonstrates no shortfall: odrl:eq is literal \
              equality of two named areas, and Berlin is not Germany.",
    alternative: "The widespread deployment reading treats `spatial eq <region IRI>` as \"the location of \
                  exercise falls within that area\", under which the ideal is Allow and this probe does \
                  demonstrate the row's containment shortfall. Picking it commits the page to asserting \
                  that eq is not equality.",
  },
  ContestedReading {
    row: "left-operands.opaque",
    probe: "lo-language-no-bcp47",
    shipped: "Ideal Deny, equal to current: Vocabulary 4.5.13 cites BCP 47 in a non-normative Note to fix \
              the lexical form of values, while matching is the operator's job -- and eq is equality, so \
              en-GB is not en.",
    alternative: "Naming BCP 47 pulls in RFC 4647 basic filtering, under which the range \"en\" matches the \
                  tag \"en-GB\" and the ideal is Allow. The /coverage row's own documented premise asserts \
                  this second reading, so either this page contradicts that row or the row is re-scoped.",
  },
  ContestedReading {
    row: "left-operands.coordinates",
    probe: "lo-absoluteposition-no-ordering",
    shipped: "Ideal Deny, equal to current: ODRL defines no ordering over a 2-D coordinate tuple, so \
              `absolutePosition lt 49,3` is an undefined comparison and IM 2.5.1's binary model leaves the \
              constraint unsatisfied.",
    alternative: "A geometry-aware engine implements the componentwise product order -- 48.8566 < 49 and \
                  2.3522 < 3 both hold -- making the ideal Allow. Worth recording separately: the narrower \
                  term absoluteTemporalPosition (4.5.3) genuinely IS orderable, a real gap this 2-D probe \
                  does not reach.",
  },
  ContestedReading {
    row: "operators.ordering",
    probe: "op-lt-mixed-type-miss",
    shipped: "Ideal Deny, equal to current: a rightOperand violating its leftOperand's declared datatype \
              leaves the constraint unsatisfiable, and IM 1.3 assigns datatype checking to a separate \
              \"ODRL Validator\" conformance class, distinct from the Evaluator this engine is.",
    alternative: "Vocabulary 4.5.6's xsd:date/xsd:dateTime clause is a hard MUST, so an engine could refuse \
                  the policy outright with Error -- demonstrably this engine's posture elsewhere, since an \
                  out-of-enum operator token is a hard parse Error. Under either reading the refusal to \
                  coerce a timestamp into a number is itself correct; only the outcome's shape is at issue.",
  },
  ContestedReading {
    row: "operators.isa-haspart",
    probe: "op-haspart-unparseable",
    shipped: "That the current Error is wrong is settled, not contested: hasPart is one of Vocabulary \
              3.14.4's twelve core Operator instances, so a compliant engine must parse and evaluate it. \
              What is contested is only the non-Error decision it should reach -- Deny ships, reading \
              3.16.8's \"contains\" as a set relation a bare scalar cannot satisfy.",
    alternative: "Standard mereology makes parthood reflexive, so every value contains itself and the ideal \
                  would be Allow with the reason 'purpose hasPart odrl:Purpose'. ODRL 2.2 says nothing \
                  about reflexivity either way. Either branch leaves this row falling short.",
  },
  ContestedReading {
    row: "policy-classes.discrimination",
    probe: "pc-kind-offer-assignee-inert-even-on-a-match",
    shipped: "Ideal Allow, equal to current: Vocabulary 3.2.2's \"MUST not grant any privileges to that \
              Party\" makes only the Offer's named assignee inert, leaving the rule evaluable for whoever \
              asks.",
    alternative: "Read IM 2.1.2's \"An Offer ... does not grant any Rules\" literally and an Offer confers \
                  access to nobody, so a closed-world access engine must never return Allow from an Offer \
                  alone. That is what most dataspace connectors do in practice: an Offer is contracted into \
                  an Agreement first.",
  },
  ContestedReading {
    row: "policy-classes.discrimination",
    probe: "pc-kind-offer-assignee-inert-even-on-a-mismatch",
    shipped: "Ideal Allow, equal to current -- the same reading as its sibling probe.",
    alternative: "The same Offer-confers-nothing reading, which flips both Offer probes identically. \
                  Whichever branch is chosen must be applied to both, since the pair exists precisely to \
                  show the answer does not depend on the assignee's direction.",
  },
  ContestedReading {
    row: "policy-classes.discrimination",
    probe: "pc-kind-ticket-with-assignee",
    shipped: "Ideal Allow, equal to current: Vocabulary 4.1.4's \"MUST NOT contain an Assignee\" sits in an \
              explicitly non-normative section, and IM 1.3 makes conformance checking the ODRL Validator's \
              job rather than the Evaluator's.",
    alternative: "Error, refusing the structurally invalid Ticket -- an internal-consistency argument, since \
                  this engine already Errors on an undeclared action. And 4.1.4 grants \"to the holder of \
                  that Ticket\", evidence the wire carries no channel for at all, so a third ideal (Deny \
                  for want of holder evidence) is arguable.",
  },
  ContestedReading {
    row: "policy-classes.discrimination",
    probe: "pc-kind-nonsense",
    shipped: "Ideal Error, so the row falls short: IM 2.1 requires a processor to understand a declared \
              subclass's additional constraints, and 2.1.1 licenses the Set default only \"if none is \
              specified\" -- here one IS specified, just unrecognisably. Error ships over Deny for \
              consistency with this engine's existing act-unrecognized-error behaviour.",
    alternative: "Refusing an undeclared Policy subclass may be an ODRL Validator's duty (IM 1.3) and so \
                  outside an Evaluator's remit, leaving the current Allow defensible. The spec prescribes \
                  no output form, so Error vs Deny vs evaluate-as-Set is genuinely undetermined.",
  },
  ContestedReading {
    row: "policy-classes.set-default",
    probe: "pc-kind-nonsense",
    shipped: "The same probe, judged identically in both rows it appears on -- deliberately, because a \
              shared probe must not carry two different ideals. Read for this row the finding is sharper: \
              the row's claim is that identical evaluation of any kind IS always-Set semantics, and this is \
              the one place where falling back to Set is an extrapolation the spec does not grant.",
    alternative: "Under the Allow branch this row would have no falls-short probe at all, and its shortfall \
                  would need a new probe of its own.",
  },
  ContestedReading {
    row: "party.assigner-assignee",
    probe: "pf-assignee-scoped-miss",
    shipped: "Ideal Allow, so the row falls short -- and it is the one place on this page where the engine \
              is short of the spec by being STRICTER than it, not laxer. Vocabulary 3.2.3 says of a Set \
              that \"No privileges are granted to any Party (if defined)\", and IM 2.6.1 enumerates a \
              Permission's satisfaction condition with no party term at all, so partyIdentityClaim's \
              narrowing Deny goes beyond ODRL rather than approximating it.",
    alternative: "3.2.3's clause is about granting, and the spec never says an evaluator MUST NOT narrow -- \
                  under which the current Deny is fine and this row meets full spec. This is the judgment \
                  in the whole set most likely to be contested.",
  },
  ContestedReading {
    row: "other.uid",
    probe: "uid-rule-index-not-uid",
    shipped: "Ideal equal to current: the spec is silent on an evaluator's output, and IM 2.6's rule uid is \
              a MAY used only so a rule may be referenced by other rules -- which this request does not \
              exercise.",
    alternative: "IM 2.6's stated purpose makes an author-assigned rule identity load-bearing, so an engine \
                  that silently discards it falls short. The real question is scope-level and is answered \
                  once, here: this page judges DECISIONS, not diagnostics.",
  },
  ContestedReading {
    row: "other.uid",
    probe: "uid-constraint-no-uid",
    shipped: "The same decisions-only scope answer as its sibling.",
    alternative: "This one has a latent decision-level bite the current probe does not reach: IM 2.5.2's \
                  Logical Constraint operands are references to existing Constraint instances, so a policy \
                  whose xone/and operands are IRI references to uid-bearing constraints would be \
                  unevaluable in an engine that drops constraint uids. The wire contract has no \
                  reference-by-uid operand form today.",
  },
  ContestedReading {
    row: "other.right-operand-reference",
    probe: "ror-reference-key-ignored",
    shipped: "Ideal Error, so the row falls short: the constraint declares both odrl:rightOperand and \
              odrl:rightOperandReference, and IM 2.5.1 permits only one, so a compliant engine refuses the \
              expression rather than silently deciding it by the literal operand.",
    alternative: "Honour the reference charitably -- dereference it as 2.5.1 intends and compare -- which \
                  still fails, so the decision stays the current Deny and only the reason changes. Either \
                  way this probe does not exhibit the row's ACTUAL shortfall, which needs a new probe \
                  carrying rightOperandReference alone under a set-based operator. Closely related to \
                  op-lt-mixed-type-miss's Deny-vs-Error fork, where the opposite branch shipped.",
  },
];

/// For each implementable row that comes back **meeting full spec**, what
/// its documented `Partial`/`NotImplemented` status names that none of its
/// current probes reaches.
///
/// This is the honest other half of the page. A row here is not a row this
/// study now claims is fully compliant: it is a row whose *current probes*
/// all reach the spec-ideal answer, while the narrowing its own
/// documentation describes is demonstrated by nothing. Two different
/// remedies apply and the text says which — a new probe, or a re-scoped
/// documented status where the source gap analysis turned out to be wrong
/// about the engine rather than about the spec.
///
/// Nineteen of these came from the research pass's own needs-a-new-probe
/// backlog; `logical.and-sequence` did not, and was found by the catalog's
/// invariant test instead.
pub const UNREACHED_NARROWING: &[(&str, &str)] = &[
  (
    "actions.implies",
    "Both probes reach the spec-ideal answer: dropping odrl:implies happens to be right here because \
     implies is a prohibition/conflict-detection relation (IM 2.4), never a permission-coverage one. The \
     real gap needs a probe where a permitted action implies a PROHIBITED one over the same target -- the \
     conflict the spec's own share/distribute example describes.",
  ),
  (
    "actions.spec-taxonomy",
    "All three chains (display->play->use, extract->reproduce->use, sell->transfer) resolve exactly as the \
     Vocabulary asserts, so whatever narrowing this row's Partial status names is reached by none of them.",
  ),
  (
    "left-operands.numeric",
    "lo-count-absent-not-stateful denies for the same reason a compliant engine would: there is no value to \
     compare. The real shortfall needs a probe injecting the spec's own odrl:status key alongside `count \
     lteq 10`, which an ideal engine must use and this engine's three-field Constraint ignores.",
  ),
  (
    "left-operands.spatial",
    "Under the shipped reading of odrl:eq (see the contested judgments above) the containment shortfall \
     lives entirely under isPartOf, and needs a probe posing `spatial isPartOf <Germany>` against a Berlin \
     claim -- which the operators.is-part-of row now has, and this one does not.",
  ),
  (
    "left-operands.opaque",
    "Neither probe falls short under the shipped readings: eq is equality, and IM 2.5.1 forbids \
     dereferencing a plain rightOperand, so no period is supplied for either bare event IRI. Demonstrating \
     either premise needs probes that actually supply the language ranges or event periods the row assumes.",
  ),
  (
    "left-operands.durations",
    "lo-duration-malformed-miss denies for the spec's own reason -- an invalid xsd:duration literal can \
     never match. The documented narrowing is the fixed 365/30-day nominal conversion of Y/M components, \
     which needs a yearMonth duration where XSD's partial order and that conversion diverge.",
  ),
  (
    "left-operands.coordinates",
    "lo-coordinates-no-geometry compares two genuinely distinct points about 10 m apart, which eq correctly \
     rejects -- ODRL defines no tolerance or precision anywhere on Constraint. A demonstrating probe needs \
     two lexically different values denoting the SAME geometry under 4.5.27's defaults, or a containment \
     question posed with isPartOf against a bordered area.",
  ),
  (
    "left-operands.unit-of-count",
    "Neither probe puts odrl:unit or a sibling unitOfCount Constraint on the POLICY -- the key appears only \
     in claims the policy never references -- so a compliant engine ignores it too. The shortfall needs IM \
     2.5.5 Example 16's shape, which the three-field Constraint wire type cannot currently carry.",
  ),
  (
    "operators.neq",
    "Both probes reach the spec-ideal answer: the engine's absent-value miss is exactly IM 2.5.1's binary \
     \"otherwise it is not satisfied\". The source gap analysis this row rests on was wrong about the \
     engine rather than about the spec, so the row may deserve re-scoping rather than a new probe.",
  ),
  (
    "operators.ordering",
    "All six probes reach the spec-ideal answer -- chronological ordering, offset normalization, \
     date-vs-dateTime, strict and non-strict boundaries, and no cross-datatype coercion. Any residual \
     ordering gap would need a probe naming it.",
  ),
  (
    "logical.and-sequence",
    "Found by the catalog's own invariant test rather than handed to it by the research pass. With two \
     stateless attribute predicates, \"satisfied in sequence\" degenerates to \"satisfied\", so Vocabulary \
     3.17.4's \"in the order specified\" -- precisely the narrowing this row's own caveat names -- is \
     reached by none of its three probes.",
  ),
  (
    "party.inverse-properties",
    "pf-assignerof-inert injects assignerOf/assigneeOf ON the policy with asset URNs as values, so IM \
     2.3.3's inference (Domain: Party, Range: Policy) is never licensed and a compliant engine infers \
     nothing either. A demonstrating probe needs them asserted in the spec's own direction, from an \
     external Party metadata expression onto the Policy.",
  ),
  (
    "party.common-functions",
    "Vocabulary 4.3.1-4.3.12 define all twelve functions as ancillary roles hanging off specific DUTY \
     actions, none of which appears in IM 2.6.1's permission condition -- so dropping them is spec-correct \
     for this request. A probe would have to attach them to the duty actions they qualify.",
  ),
  (
    "duty.obligation",
    "All three probes reach the spec-ideal answer. The real narrowing is that Rule::duty_satisfied treats \
     ANY satisfied constraint as proof the action was exercised, contra IM 2.6.4's conjunction -- needing \
     an obligation whose constraint is unrelated to performance (e.g. `dateTime lt 2030`), which the engine \
     would report fulfilled on the constraint alone.",
  ),
  (
    "duty.remedy",
    "All three probes reach the spec-ideal answer, and the finding here is sharper than a missing probe: IM \
     2.6.2/2.6.7 do not support this row's own stated premise that a performed remedy substitutes for the \
     violation. A fulfilled remedy changes the Prohibition's INFRINGEMENT state, not what it permits, so \
     the documentation overstates the gap and may deserve re-scoping.",
  ),
  (
    "assets.collections",
    "All three probes reach the spec-ideal answer: IM 2.2.2 makes odrl:partOf an asserted fact, never \
     inferred, which is exactly the host-supplied, non-transitive model the engine implements. Any real \
     shortfall -- transitive closure over nested collections -- needs a probe asserting a chain of \
     memberships.",
  ),
  (
    "assets.target",
    "Both probes reach the spec-ideal answer: per-rule odrl:target scoping and IM 2.10's same-target \
     restriction on conflict are implemented correctly.",
  ),
  (
    "assets.output",
    "asset-output-ignored reaches the spec-ideal answer because odrl:output states no precondition on \
     whether a permission grants (Vocabulary 4.2.1; IM 2.2.1 makes target the only relation sub-property a \
     validator MUST support). This row's shortfall -- the produced-Asset obligation never carried or \
     reported back -- is not decision-observable, and cannot be probed without a wire-contract addition \
     echoing odrl:output in the response.",
  ),
  (
    "other.uid",
    "Under the decisions-only scope answered in the contested judgments above, both uid probes reach the \
     spec-ideal answer. The latent decision-level bite is IM 2.5.2's operands-by-reference, for which the \
     wire contract has no form today.",
  ),
  (
    "other.inherit-from",
    "All six probes reach the spec-ideal answer, including the conflict-divergence void. Whatever this \
     row's Partial status names -- transitive multi-level chains, IM 2.9's policy-level asset/action \
     replication -- would need new probes to reach.",
  ),
];

/// The unreached-narrowing note for one row, if it has one.
pub fn unreached_narrowing(row_id: &str) -> Option<&'static str> {
  UNREACHED_NARROWING.iter().find(|(id, _)| *id == row_id).map(|(_, note)| *note)
}

/// Every contested judgment recorded against one row.
pub fn contested_readings_for(row_id: &str) -> Vec<&'static ContestedReading> {
  CONTESTED_READINGS.iter().filter(|entry| entry.row == row_id).collect()
}

/// Judges one probe against full ODRL 2.2.
///
/// The order of the arms is the whole contract:
///
/// 1. A probe this browser could not judge is [`SpecOutcome::Undetermined`],
///    never either verdict.
/// 2. A probe whose live answer departed from its *documented* one is also
///    undetermined here, and says so pointing at `/coverage`. The ideal
///    recorded in the catalog was researched against the documented
///    behaviour; once the engine stops producing that behaviour, the
///    research no longer describes what is running, and quietly reporting
///    the old judgment would be asserting something this run did not
///    observe.
/// 3. Otherwise the presence of an [`Ideal`] *is* the judgment. Not a
///    comparison of decisions — one probe's ideal decision equals its
///    current one and what falls short is the duties list.
pub fn judge_probe(fixture: &ProbeFixture, outcome: &ProbeOutcome) -> (SpecOutcome, Option<String>) {
  match outcome.status {
    ProbeStatus::Errored => (
      SpecOutcome::Undetermined,
      Some(format!(
        "this browser could not judge the probe at all: {}",
        outcome.reason.clone().unwrap_or_else(|| "no detail".to_string())
      )),
    ),
    ProbeStatus::Disagreed => (
      SpecOutcome::Undetermined,
      Some(format!(
        "the engine departed from its own documented behaviour here ({}), which the Coverage page reports \
         as a contradiction. The full-spec judgment recorded for this probe was researched against the \
         documented behaviour, so it does not describe what just ran.",
        outcome.mismatch.clone().unwrap_or_else(|| "no detail".to_string())
      )),
    ),
    ProbeStatus::Agreed => {
      if fixture.ideal.is_some() {
        (SpecOutcome::FallsShort, None)
      } else {
        (SpecOutcome::MeetsFullSpec, None)
      }
    }
  }
}

/// One row's verdict over its own probes.
///
/// `StructuralGap` wins outright: a row whose gap no request can pose is
/// short of the spec regardless of what any probe did, and it has no
/// probes anyway. After that, `FallsShort` out-ranks `Undetermined` for
/// the same reason `/coverage`'s `Contradicted` out-ranks its
/// `Inconclusive`.
pub fn derive_row_verdict(row: &CatalogRow, probes: &[&ProbeJudgment]) -> RowSpecVerdict {
  if row.full_compliance_gap.is_some() {
    return RowSpecVerdict::StructuralGap;
  }
  // Not reachable from a live run — `parse_coverage_catalog` rejects a row
  // naming an unknown probe, and every catalog probe produces exactly one
  // outcome — but guarded explicitly rather than left to fall through to a
  // vacuous `MeetsFullSpec`, which is the one silent-green result this
  // module must never produce. The same guard, for the same reason, that
  // `coverage_catalog::derive_verdict` carries.
  if probes.is_empty() {
    return RowSpecVerdict::Undetermined;
  }
  if probes.iter().any(|p| p.outcome == SpecOutcome::FallsShort) {
    return RowSpecVerdict::FallsShort;
  }
  if probes.iter().any(|p| p.outcome == SpecOutcome::Undetermined) {
    return RowSpecVerdict::Undetermined;
  }
  RowSpecVerdict::MeetsFullSpec
}

/// Attaches every probe outcome to the in-scope rows that name it, derives
/// a verdict per row, and tallies both axes. Infallible by construction: a
/// pure function over owned data.
pub fn compile_full_compliance_report(
  catalog: &CoverageFile,
  outcomes: Vec<ProbeOutcome>,
  elapsed_ms: f64,
  engine_bytes: usize,
) -> FullComplianceReport {
  let judgments: Vec<ProbeJudgment> = catalog
    .probes
    .iter()
    .filter_map(|fixture| {
      let outcome = outcomes.iter().find(|o| o.id == fixture.id)?;
      let (spec_outcome, because) = judge_probe(fixture, outcome);
      Some(ProbeJudgment {
        id: fixture.id.clone(),
        title: fixture.title.clone(),
        kind: fixture.kind.clone(),
        asserts: fixture.asserts.clone(),
        outcome: spec_outcome,
        documented_decision: fixture.expect.decision.clone(),
        ideal: fixture.ideal.clone(),
        observed_decision: outcome.decision.clone(),
        observed_reason: outcome.reason.clone(),
        undetermined_because: because,
      })
    })
    .collect();

  let find = |id: &str| judgments.iter().find(|judgment| judgment.id == id);

  let rows: Vec<RowJudgment> = catalog
    .rows
    .iter()
    .filter(|row| is_in_scope(row))
    .map(|row| {
      let probes: Vec<&ProbeJudgment> = row.probe_ids.iter().filter_map(|id| find(id)).collect();
      let verdict = derive_row_verdict(row, &probes);
      RowJudgment { row: row.clone(), verdict, probes: probes.into_iter().cloned().collect() }
    })
    .collect();

  // The probe axis counts *distinct* probes this page renders, not
  // probe-appearances: three rows name `pc-kind-nonsense`, and counting it
  // three times would inflate both the numerator and the denominator of a
  // number a reader will read as "probes".
  let mut judged: Vec<&ProbeJudgment> = Vec::new();
  for row in &rows {
    for probe in &row.probes {
      if !judged.iter().any(|seen| seen.id == probe.id) {
        judged.push(probe);
      }
    }
  }

  let count_verdict = |verdict: RowSpecVerdict| rows.iter().filter(|r| r.verdict == verdict).count() as u64;
  let count_status = |status: &str| rows.iter().filter(|r| r.row.status == status).count() as u64;
  let count_outcome = |outcome: SpecOutcome| judged.iter().filter(|p| p.outcome == outcome).count() as u64;

  FullComplianceReport {
    generated_by: catalog.generated_by.clone(),
    spec: catalog.spec.clone(),
    source_analysis: catalog.source_analysis.clone(),
    categories: catalog.categories.clone(),
    rows_in_scope: rows.len() as u64,
    rows_excluded: (catalog.rows.len() - rows.len()) as u64,
    rows_meets: count_verdict(RowSpecVerdict::MeetsFullSpec),
    rows_falls_short: count_verdict(RowSpecVerdict::FallsShort),
    rows_structural_gap: count_verdict(RowSpecVerdict::StructuralGap),
    rows_undetermined: count_verdict(RowSpecVerdict::Undetermined),
    implemented_rows: count_status("Implemented"),
    partial_rows: count_status("Partial"),
    not_implemented_rows: count_status("NotImplemented"),
    probes_judged: judged.len() as u64,
    probes_in_catalog: catalog.probes.len() as u64,
    probes_meets: count_outcome(SpecOutcome::MeetsFullSpec),
    probes_falls_short: count_outcome(SpecOutcome::FallsShort),
    probes_undetermined: count_outcome(SpecOutcome::Undetermined),
    rows,
    elapsed_ms,
    engine_bytes,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::coverage_catalog::parse_coverage_catalog;

  const LATEST_COVERAGE_JSON: &str = include_str!("../../compliance/reports/latest-coverage.json");

  fn catalog() -> CoverageFile {
    parse_coverage_catalog(LATEST_COVERAGE_JSON).expect("the committed catalog parses")
  }

  /// Every probe agreeing with its documented expectation: the shape a
  /// healthy run produces today, and the one the compliance suite's
  /// 68/68/0/0 plus `/coverage`'s own zero-contradiction run stand behind.
  fn healthy_outcomes(catalog: &CoverageFile) -> Vec<ProbeOutcome> {
    catalog
      .probes
      .iter()
      .map(|probe| ProbeOutcome {
        id: probe.id.clone(),
        title: probe.title.clone(),
        kind: probe.kind.clone(),
        asserts: probe.asserts.clone(),
        falsified_by: probe.falsified_by.clone(),
        expected_decision: probe.expect.decision.clone(),
        status: ProbeStatus::Agreed,
        decision: Some(probe.expect.decision.clone()),
        reason: Some("synthetic".to_string()),
        mismatch: None,
      })
      .collect()
  }

  fn report() -> FullComplianceReport {
    let catalog = catalog();
    let outcomes = healthy_outcomes(&catalog);
    compile_full_compliance_report(&catalog, outcomes, 41.0, 232_881)
  }

  #[test]
  fn the_seven_out_of_scope_rows_are_excluded_and_nothing_else_is() {
    let catalog = catalog();
    let report = report();

    assert_eq!(report.rows_excluded, 7, "the profile-extension and odrl:hasPolicy rows");
    assert_eq!(report.rows_in_scope, 45);
    assert_eq!(report.rows_in_scope + report.rows_excluded, catalog.rows.len() as u64);
    for row in &report.rows {
      assert_ne!(row.row.status, "OutOfScope", "row {} should have been excluded", row.row.id);
    }
    assert_eq!(
      report.implemented_rows + report.partial_rows + report.not_implemented_rows,
      report.rows_in_scope,
      "every in-scope row falls in one of the three remaining documented statuses"
    );
    assert_eq!((report.implemented_rows, report.partial_rows, report.not_implemented_rows), (11, 23, 11));
  }

  /// The headline numbers this page prints, pinned against the committed
  /// artifact. A change to the catalog's falls-short evidence must move
  /// these deliberately rather than silently.
  #[test]
  fn the_row_axis_tallies_to_the_numbers_this_page_states() {
    let report = report();

    assert_eq!(report.rows_meets, 31);
    assert_eq!(report.rows_falls_short, 13, "thirteen rows have a probe demonstrating a shortfall");
    assert_eq!(report.rows_structural_gap, 1, "party.collections, which no request can pose");
    assert_eq!(report.rows_undetermined, 0);
    assert_eq!(report.rows_short_of_spec(), 14);
    assert_eq!(
      report.rows_meets + report.rows_short_of_spec() + report.rows_undetermined,
      report.rows_in_scope,
      "the three buckets partition the in-scope rows"
    );
  }

  /// The honesty check the page's own copy turns on: most judged probes
  /// already produce the spec-ideal answer, so only a small minority
  /// actually distinguish full compliance from documented compliance.
  /// Quoting a bare `N/136` would imply otherwise.
  #[test]
  fn the_probe_axis_counts_distinct_probes_and_only_a_minority_distinguish_the_two_questions() {
    let catalog = catalog();
    let report = report();

    assert_eq!(report.probes_in_catalog, 136);
    assert_eq!(report.probes_judged, 130, "six probes are named only by rows this page excludes");
    assert_eq!(report.probes_meets, 115);
    assert_eq!(report.probes_falls_short, 15);
    assert_eq!(report.probes_undetermined, 0);
    assert_eq!(
      report.probes_meets + report.probes_falls_short + report.probes_undetermined,
      report.probes_judged,
      "the three outcomes partition the judged probes"
    );

    // `pc-kind-nonsense` is named by three rows (two in scope, one out of
    // it). If the tally counted appearances rather than distinct probes,
    // this assertion is what would catch it.
    let appearances: usize = report.rows.iter().map(|row| row.probes.len()).sum();
    assert!(
      appearances > report.probes_judged as usize,
      "at least one probe is shared between in-scope rows, so appearances must exceed distinct probes"
    );
    assert_eq!(
      catalog.probes.iter().filter(|p| p.ideal.is_some()).count() as u64,
      report.probes_falls_short,
      "every probe carrying an ideal is named by at least one in-scope row, so none is lost by the filter"
    );
  }

  /// The load-bearing rule, stated as a test rather than as a comment:
  /// the judgment is the *presence* of an ideal, never a comparison of
  /// decisions. `duty-consequence-itself-unresolved` is the probe that
  /// proves it — its ideal decision equals its current one, and what falls
  /// short is the reported duties list.
  #[test]
  fn a_probe_whose_ideal_decision_equals_its_current_one_still_falls_short() {
    let catalog = catalog();
    let report = report();

    let fixture = catalog
      .probes
      .iter()
      .find(|p| p.id == "duty-consequence-itself-unresolved")
      .expect("the catalog carries this probe");
    let ideal = fixture.ideal.as_ref().expect("it carries an ideal");
    assert_eq!(ideal.decision, fixture.expect.decision, "the premise of this test: the decisions are equal");

    let judgment = report
      .rows
      .iter()
      .flat_map(|row| row.probes.iter())
      .find(|p| p.id == "duty-consequence-itself-unresolved")
      .expect("an in-scope row names it");
    assert_eq!(judgment.outcome, SpecOutcome::FallsShort);
  }

  #[test]
  fn every_implemented_row_meets_full_spec_and_carries_no_ideal_anywhere() {
    let report = report();
    for row in report.rows.iter().filter(|r| r.row.status == "Implemented") {
      assert_eq!(row.verdict, RowSpecVerdict::MeetsFullSpec, "row {}", row.row.id);
      for probe in &row.probes {
        assert!(probe.ideal.is_none(), "Implemented row {} names falls-short probe {}", row.row.id, probe.id);
      }
    }
  }

  /// Not every `Partial`/`NotImplemented` row falls short: a row is
  /// partial for one specific reason, and twenty of the thirty-four
  /// implementable rows reach the spec-ideal answer on every probe they
  /// currently carry. Saying so is the point of the honest half of this
  /// page.
  #[test]
  fn twenty_implementable_rows_meet_full_spec_on_the_probes_they_carry() {
    let report = report();
    let implementable: Vec<&RowJudgment> = report.rows.iter().filter(|r| r.row.is_implementable()).collect();
    assert_eq!(implementable.len(), 34);

    let meeting: Vec<&str> = implementable
      .iter()
      .filter(|r| r.verdict == RowSpecVerdict::MeetsFullSpec)
      .map(|r| r.row.id.as_str())
      .collect();
    assert_eq!(meeting.len(), 20, "{meeting:?}");
    assert!(meeting.contains(&"logical.and-sequence"));
    assert!(meeting.contains(&"operators.ordering"));
  }

  #[test]
  fn exactly_one_row_is_a_structural_gap_and_it_is_the_one_with_no_probes() {
    let report = report();
    let gaps: Vec<&RowJudgment> =
      report.rows.iter().filter(|r| r.verdict == RowSpecVerdict::StructuralGap).collect();

    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].row.id, "party.collections");
    assert!(gaps[0].probes.is_empty(), "a structural gap has no probe to expand");
    assert!(
      gaps[0].row.full_compliance_gap.as_ref().is_some_and(|gap| gap.contains("party_collections")),
      "the gap text must name the wire-contract addition, not merely assert one is needed"
    );
    assert!(gaps[0].verdict.is_short_of_spec());
  }

  /// The famous one, kept as its own test because it is the example the
  /// page leads with and a reader will look for it by name.
  #[test]
  fn the_ispartof_row_falls_short_with_berlin_and_germany_named_in_its_citation() {
    let report = report();
    let row = report.rows.iter().find(|r| r.row.id == "operators.is-part-of").expect("the row exists");

    assert_eq!(row.verdict, RowSpecVerdict::FallsShort);
    let probe = row.probes.iter().find(|p| p.id == "op-ispartof-no-hierarchy").expect("the probe exists");
    assert_eq!(probe.outcome, SpecOutcome::FallsShort);
    assert_eq!(probe.documented_decision, "Deny");
    let ideal = probe.ideal.as_ref().expect("it falls short, so it carries an ideal");
    assert_eq!(ideal.decision, "Allow", "the engine denies where a compliant one allows");
    assert!(ideal.spec_citation.contains("3.16.9"));
    assert!(ideal.spec_citation.contains("Germany"));
  }

  /// An unjudgeable probe must not be able to turn a row green or score a
  /// finding, and a probe whose engine answer departed from its own
  /// documentation must invalidate the researched ideal rather than be
  /// reported under it.
  #[test]
  fn an_errored_or_contradicting_probe_is_undetermined_and_never_either_verdict() {
    let catalog = catalog();
    let mut outcomes = healthy_outcomes(&catalog);

    // A probe that carries an ideal, perturbed to have errored: it must
    // stop being reported as a researched shortfall.
    for outcome in &mut outcomes {
      if outcome.id == "op-ispartof-no-hierarchy" {
        outcome.status = ProbeStatus::Errored;
        outcome.reason = Some("evaluate() did not return a BigInt (i64)".to_string());
      }
      // A probe that carries no ideal, perturbed to disagree with its own
      // documentation: it must stop being reported as meeting full spec.
      if outcome.id == "act-includedin-1hop" {
        outcome.status = ProbeStatus::Disagreed;
        outcome.mismatch = Some("expected Allow, observed Deny".to_string());
      }
    }
    let report = compile_full_compliance_report(&catalog, outcomes, 1.0, 1);

    let judgment = |id: &str| {
      report
        .rows
        .iter()
        .flat_map(|row| row.probes.iter())
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no judgment for {id}"))
        .clone()
    };

    let errored = judgment("op-ispartof-no-hierarchy");
    assert_eq!(errored.outcome, SpecOutcome::Undetermined);
    assert!(errored.undetermined_because.unwrap().contains("could not judge"));

    let contradicting = judgment("act-includedin-1hop");
    assert_eq!(contradicting.outcome, SpecOutcome::Undetermined);
    assert!(contradicting.undetermined_because.unwrap().contains("departed from its own documented behaviour"));

    // The isPartOf row loses its only shortfall evidence, so it is now
    // undetermined -- emphatically not "meets full spec".
    let row = report.rows.iter().find(|r| r.row.id == "operators.is-part-of").expect("the row exists");
    assert_eq!(row.verdict, RowSpecVerdict::Undetermined);
    assert!(!row.verdict.is_short_of_spec());
    assert_eq!(report.rows_short_of_spec(), 13, "the isPartOf row left the shortfall bucket");

    // `act-includedin-1hop` is named by two in-scope rows -- one
    // Implemented (actions.open-vocabulary) and one NotImplemented
    // (actions.implies) -- and both leave the meets bucket for
    // Undetermined, not for a shortfall. That an Implemented row can go
    // undetermined is the point: this page never asserts full compliance
    // over an answer it did not observe.
    assert_eq!(report.rows_meets, 29);
    assert_eq!(report.rows_undetermined, 3);
    for id in ["actions.open-vocabulary", "actions.implies"] {
      let row = report.rows.iter().find(|r| r.row.id == id).expect("the row exists");
      assert_eq!(row.verdict, RowSpecVerdict::Undetermined, "row {id}");
    }
  }

  #[test]
  fn falls_short_out_ranks_undetermined_within_one_row() {
    let catalog = catalog();
    let mut outcomes = healthy_outcomes(&catalog);
    // operators.isa-haspart carries two falls-short probes; erroring one
    // must leave the row a shortfall on the strength of the other.
    for outcome in &mut outcomes {
      if outcome.id == "op-isa-unparseable" {
        outcome.status = ProbeStatus::Errored;
      }
    }
    let report = compile_full_compliance_report(&catalog, outcomes, 1.0, 1);
    let row = report.rows.iter().find(|r| r.row.id == "operators.isa-haspart").expect("the row exists");
    assert_eq!(row.verdict, RowSpecVerdict::FallsShort);
  }

  #[test]
  fn a_row_with_no_probes_and_no_structural_gap_is_undetermined_not_silently_compliant() {
    let mut row = CatalogRow {
      id: "synthetic".to_string(),
      category: "actions".to_string(),
      term: "t".to_string(),
      status: "Partial".to_string(),
      why: "w".to_string(),
      evidence: "e".to_string(),
      asserts: "a".to_string(),
      probe_ids: vec!["a".to_string()],
      documented_because: None,
      caveat: None,
      full_compliance_gap: None,
    };
    assert_eq!(derive_row_verdict(&row, &[]), RowSpecVerdict::Undetermined);

    row.full_compliance_gap = Some("a wire-contract addition".to_string());
    assert_eq!(derive_row_verdict(&row, &[]), RowSpecVerdict::StructuralGap);
  }

  #[test]
  fn every_falls_short_probe_names_a_contract_decision_and_quotes_the_spec() {
    let report = report();
    for probe in report.rows.iter().flat_map(|row| row.probes.iter()) {
      let Some(ideal) = &probe.ideal else { continue };
      assert!(matches!(ideal.decision.as_str(), "Allow" | "Deny" | "Error"), "{}", probe.id);
      assert!(!ideal.reason.is_empty(), "{}", probe.id);
      assert!(!ideal.spec_citation.is_empty(), "{}", probe.id);
      assert_eq!(probe.outcome, SpecOutcome::FallsShort, "{}", probe.id);
    }
  }

  /// The sign-off list must name things that exist, on rows this page
  /// actually renders — otherwise the caveat is decoration.
  #[test]
  fn every_contested_reading_names_a_real_in_scope_row_and_a_probe_that_row_names() {
    let catalog = catalog();
    for entry in CONTESTED_READINGS {
      let row = catalog
        .rows
        .iter()
        .find(|row| row.id == entry.row)
        .unwrap_or_else(|| panic!("contested reading names unknown row {}", entry.row));
      assert!(is_in_scope(row), "contested reading names excluded row {}", entry.row);
      assert!(
        row.probe_ids.iter().any(|id| id == entry.probe),
        "row {} does not name probe {}",
        entry.row,
        entry.probe
      );
      assert!(!entry.shipped.is_empty() && !entry.alternative.is_empty(), "{}", entry.probe);
    }

    let mut probes: Vec<&str> = CONTESTED_READINGS.iter().map(|e| e.probe).collect();
    probes.sort_unstable();
    probes.dedup();
    assert_eq!(CONTESTED_READINGS.len(), 15);
    assert_eq!(probes.len(), 14, "pc-kind-nonsense is listed under both in-scope rows that name it");
    assert_eq!(contested_readings_for("policy-classes.discrimination").len(), 4);
  }

  /// A contested judgment is not the same thing as a shortfall, and the
  /// page says so. Five of the fourteen contested probes carry an ideal;
  /// the other nine were judged ideal == current, so what is contested
  /// there is whether they should have fallen short at all.
  #[test]
  fn five_of_the_contested_probes_fall_short_and_the_rest_were_judged_to_meet_the_spec() {
    let catalog = catalog();
    let mut falling: Vec<&str> = CONTESTED_READINGS
      .iter()
      .filter(|entry| {
        catalog.probes.iter().find(|p| p.id == entry.probe).is_some_and(|p| p.ideal.is_some())
      })
      .map(|entry| entry.probe)
      .collect();
    falling.sort_unstable();
    falling.dedup();
    assert_eq!(
      falling,
      [
        "act-includedin-undeclared-gap",
        "op-haspart-unparseable",
        "pc-kind-nonsense",
        "pf-assignee-scoped-miss",
        "ror-reference-key-ignored",
      ]
    );
    assert_eq!(
      CONTESTED_READINGS
        .iter()
        .filter(|entry| falling.contains(&entry.probe))
        .count(),
      6,
      "five distinct probes, six of the fifteen listed row/probe entries -- pc-kind-nonsense twice"
    );
  }

  /// The strongest guard on the page's copy: the unreached-narrowing notes
  /// must describe **exactly** the implementable rows that come back
  /// meeting full spec — no more, no fewer. An exact set comparison, so a
  /// row silently gaining shortfall evidence (and keeping a now-false
  /// "nothing reaches this row's narrowing" note) fails just as loudly as
  /// one losing it.
  #[test]
  fn the_unreached_narrowing_notes_cover_exactly_the_implementable_rows_that_meet_full_spec() {
    let report = report();

    let mut meeting: Vec<&str> = report
      .rows
      .iter()
      .filter(|row| row.row.is_implementable() && row.verdict == RowSpecVerdict::MeetsFullSpec)
      .map(|row| row.row.id.as_str())
      .collect();
    meeting.sort_unstable();

    let mut noted: Vec<&str> = UNREACHED_NARROWING.iter().map(|(id, _)| *id).collect();
    noted.sort_unstable();

    assert_eq!(noted, meeting, "the notes and the live verdicts must describe the same set of rows");
    assert_eq!(noted.len(), 20);
    for (id, note) in UNREACHED_NARROWING {
      assert!(!note.is_empty(), "{id} has an empty note");
      assert!(unreached_narrowing(id).is_some());
    }
    // An Implemented row meets full spec trivially and has no narrowing to
    // be unreached, so none may carry a note.
    for row in report.rows.iter().filter(|r| r.row.status == "Implemented") {
      assert_eq!(unreached_narrowing(&row.row.id), None, "row {}", row.row.id);
    }
  }

  #[test]
  fn the_two_axes_use_none_of_the_coverage_pages_four_words() {
    // A guard on this module's vocabulary, which the project owner decided
    // explicitly: /coverage's Agreed/Disagreed/Verified/Contradicted mean
    // matches-the-documentation, and this page asks a different question.
    let labels: Vec<&str> = [
      SpecOutcome::MeetsFullSpec.label(),
      SpecOutcome::FallsShort.label(),
      SpecOutcome::Undetermined.label(),
      RowSpecVerdict::MeetsFullSpec.label(),
      RowSpecVerdict::FallsShort.label(),
      RowSpecVerdict::StructuralGap.label(),
      RowSpecVerdict::Undetermined.label(),
    ]
    .to_vec();
    for label in labels {
      for forbidden in ["greed", "erified", "ontradicted"] {
        assert!(!label.contains(forbidden), "label {label:?} reuses /coverage's vocabulary");
      }
    }
  }
}
