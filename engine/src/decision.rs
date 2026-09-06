//! Enforcement decision semantics — this proposal's choice (case study
//! Section 4.3), extended with Section 4.4's profile-driven action check,
//! Section 4.5's policy-level duty evaluation, and real `odrl:includedIn`
//! action-taxonomy coverage (`ResolvedConfig::covers`, `profile.rs`).
//!
//! Section 4.5's duty evaluation now runs at four ODRL attachment points
//! rather than one — the policy's own `obligations`, a permission's
//! `odrl:duty`, a duty's `odrl:consequence`, a prohibition's
//! `odrl:remedy` — all through the same mechanism: a duty is satisfied
//! when its own constraints match the claims map, and `dutyMode` governs
//! an unresolved one. Nothing here observes execution state; see `decide`'s
//! own doc comment for each position's scope and for the one place a
//! choice had to be made (a satisfied `odrl:remedy` does not lift its
//! prohibition).
//!
//! Stated per `(Policy, claims, config, requested_action)`. A permission
//! or prohibition rule now matters only if it *covers* `requested_action`
//! — an exact match, or a declared `includedIn` chain from the requested
//! action up to the rule's own — checked ahead of (and independently of)
//! that rule's `constraints`; Section 4.4's unrecognized-action check
//! (a rule naming an action no loaded profile knows about *at all*) still
//! runs first, unaffected by coverage. A policy-level duty's own `action`
//! (what the caller must *do*, e.g. `notify`) is a different thing
//! entirely from `requested_action` (what the caller is asking *for*) —
//! duty satisfaction never involves coverage matching.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::claims::Claims;
use crate::constraint::Constraint;
use crate::profile::{Behaviour, DutyMode, ResolvedConfig};

/// One permission or prohibition rule: an action plus the constraints that
/// must all be satisfied for the rule to "match" a claims set (Section
/// 4.3). An empty `constraints` list matches vacuously — an unconstrained
/// rule always applies.
///
/// `action_refinement` (W3C ODRL 2.2's `odrl:refinement`) is a later,
/// purely additive addition: a `Constraint` narrowing *the action itself*
/// rather than the circumstances under which the rule applies. ODRL 2.2's
/// own canonical example is "print, at most 2 copies" — `print` is still
/// the action, but the permission is for a narrower action than bare
/// `print`, and a request to print five copies is simply not the action
/// this rule is about. See the field's own doc comment below for the
/// scope decision (Action only, not Party or Asset) and for why this is a
/// separate field rather than one more entry in `constraints`.
///
/// `target` (W3C ODRL 2.2's `odrl:target`) is a later, equally additive
/// addition: **which asset this one rule is about**, defaulting to
/// whatever the decision is being taken about when the rule names none.
/// See that field's own doc comment for the default-fallback convention
/// and for what "the same asset" does and does not mean here.
///
/// `duty`, `remedy` and `consequence` are the three later, equally
/// additive nested-duty fields — ODRL's `odrl:duty` on a Permission,
/// `odrl:remedy` on a Prohibition, `odrl:consequence` on a Duty. Each
/// holds `Rule`s of this same type, because a Duty *is* an action plus
/// constraints here; each is read only in the rule position it belongs to,
/// stated on the field itself; and a rule carrying none of them — every
/// rule this workspace's fixtures build — is byte-for-byte what it was
/// before they existed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    /// `odrl:target` on this rule: **which asset this one rule is about**,
    /// per the ODRL 2.2 Information Model (§2.3, `odrl:target`), where
    /// every Rule carries its own target rather than inheriting one from
    /// the Policy. A later, purely additive addition, and the exact
    /// counterpart of what `Request::action` did for actions: until it
    /// existed, `Request::dataset_id` was the only asset handle anywhere on
    /// the wire, so *every* rule of *every* policy was implicitly about
    /// that one asset and a policy saying "permission on asset A,
    /// prohibition on asset B" — an ordinary thing for one ODRL policy to
    /// say — could not be represented at all.
    ///
    /// **`None` means "whatever the request is about", not "no asset".**
    /// A rule that names no target matches whichever target the decision is
    /// being taken for, which is precisely the implicit behaviour every
    /// existing fixture in this workspace relies on: nothing carrying no
    /// target changes meaning, in either direction, because of this field's
    /// existence. `Some(t)` is the narrowing case: the rule is about `t`
    /// and no other asset, so it simply does not participate in a decision
    /// about a different one — a permission for asset B does not permit
    /// asset A, and (the direction that matters more) a prohibition on
    /// asset B does not deny asset A.
    ///
    /// **Compared as an opaque string, exactly as `dataset_id` always
    /// was** — and, since `target_applies` below, also against every
    /// string in the request's `asset_collections`, the host-supplied
    /// `odrl:partOf` fact channel: this field can name an
    /// `odrl:AssetCollection` IRI and be satisfied by a request for one of
    /// its declared members, but there is still no IRI normalization, no
    /// relative-reference resolution, and no transitive closure computed
    /// by this engine itself — "the same asset" and "a member of this
    /// collection" both mean "the same characters", the same honest limit
    /// this field always had. A host resolves membership against its own
    /// graph before the request is built, exactly as `compliance-runner`'s
    /// own `is_member_of` already does (see this crate's README's
    /// "Per-rule assets" section) — this field's own comparison is still
    /// unaware that "asset" and "collection" are different kinds of thing;
    /// it is `target_applies` that reads the fact the host hands it.
    ///
    /// Wire-additive on the same convention `action_refinement` below
    /// already set: `#[serde(default)]` plus
    /// `skip_serializing_if = "Option::is_none"`, at the `odrl:`-namespaced
    /// key `odrl:target` (real ODRL vocabulary, unlike the contract's
    /// original bare-named `action`/`constraints`). A rule that carries no
    /// target — every fixture in the vendored compliance corpus, and
    /// everything `Rule::new` builds — is byte-for-byte what it was before
    /// this field existed.
    #[serde(rename = "odrl:target", default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// `odrl:refinement` on this rule's **action**, per the ODRL 2.2
    /// Information Model (§2.5, `odrl:refinement`): a `Constraint` that
    /// narrows the action itself, so the rule is about the refined action
    /// and no other. Reuses `Constraint` verbatim — including its nested
    /// `odrl:and`/`odrl:or`/`odrl:xone` groupings — so an action narrowed
    /// on several axes at once is one logical refinement, exactly as
    /// ODRL's own `odrl:LogicalConstraint` allows in this position.
    ///
    /// **Scope: Action only, deliberately.** The ODRL Information Model
    /// also permits `odrl:refinement` on a Party and on an Asset (a
    /// party *collection* narrowed to its members in a given role, an
    /// asset collection narrowed to a subset). Neither is implemented
    /// here, and neither is represented anywhere in this engine's wire
    /// contract: `decision::Policy` carries no party or asset at all
    /// (`wire::WirePolicy`'s `assigner`/`assignee` are opaque strings, and
    /// the one place `assignee` is now read — `wire::party_role_mismatch`,
    /// opt-in and above this layer — compares it by bare equality against
    /// one claim key rather than resolving it into a collection with
    /// members), so there is no node for such a refinement to attach to
    /// without first modelling parties and assets as evaluable
    /// structures. That is a much larger change than this
    /// one and is stated here as a scope decision, not an oversight.
    ///
    /// **Why not just another entry in `constraints`?** Because the two
    /// answer different questions and this engine reports on them
    /// separately. `constraints` say under what circumstances the rule
    /// applies; a refinement says *which action* the rule is about, and is
    /// therefore evaluated alongside `covers_action` as part of the action
    /// requirement (`action_applies` below) rather than as one more
    /// condition of a rule whose action already matched. Folding it into
    /// `constraints` would produce the same allow/deny answer for a
    /// permission or prohibition, but would flatten the distinction the
    /// `reason` trace exists to expose (`wire::describe_rule`), and would
    /// *not* produce the same answer for a duty: `duty_satisfied` requires
    /// at least one constraint, so a duty carrying only a refinement would
    /// silently become resolvable rather than staying unresolved.
    ///
    /// Wire-additive: `#[serde(default)]` plus
    /// `skip_serializing_if = "Option::is_none"`, at the `odrl:`-namespaced
    /// key `odrl:refinement` (the same convention `Constraint`'s own
    /// `odrl:and`/`odrl:or`/`odrl:xone` fields already set for ODRL
    /// vocabulary added after this wire contract's original bare-named
    /// `action`/`constraints`). A rule that carries no refinement — every
    /// fixture in the vendored compliance corpus, and everything
    /// `Rule::new` builds — is byte-for-byte what it was before this
    /// field existed.
    #[serde(rename = "odrl:refinement", default, skip_serializing_if = "Option::is_none")]
    pub action_refinement: Option<Constraint>,
    /// `odrl:duty` on this **permission**, per the ODRL 2.2 Information
    /// Model (§2.6.1): the duties that are a *pre-condition* of receiving
    /// this one permission, as opposed to `Policy::obligations`, which are
    /// the whole policy's (Section 4.5). Read only when this rule sits in
    /// `Policy::permissions`; a `duty` on a prohibition or on a duty is
    /// carried and never consulted (`odrl:remedy` and `odrl:consequence`
    /// below are those two positions' own properties), which is the
    /// fail-*closed* direction in both cases — an ignored pre-condition on
    /// a prohibition cannot weaken it.
    ///
    /// **Resolved exactly as a policy-level obligation is** — there is
    /// deliberately no second claims-lookup mechanism here. A duty is
    /// satisfied when its own `constraints` all match the claims map
    /// (`Rule::duty_satisfied`), which is how a host asserts "this duty is
    /// fulfilled" as an ordinary claim: a duty constrained
    /// `duty:compensate eq fulfilled` resolves precisely when the host
    /// supplies that claim. That generalizes what `compliance-runner`'s
    /// adapter already does for this one vendored corpus, where the same
    /// fact arrives as a `report:DutyReport`/`report:deonticState` triple
    /// in the fixture's state-of-the-world graph and is resolved at
    /// translate time. This engine is stateless and still cannot observe
    /// whether anything was performed; that boundary is unchanged.
    ///
    /// **Scoped to this one permission, not to the policy.** An unresolved
    /// duty here is governed by the same `dutyMode` axis Section 4.5
    /// already defines, applied at this narrower attachment point: under
    /// `DutyMode::Advise` the permission still grants and the duty is
    /// reported advisory; under `DutyMode::Deny` *this permission* does not
    /// grant, while a sibling permission with no outstanding duty still
    /// does. Denying the whole policy would be the policy-level
    /// obligation's behaviour, not this one's.
    #[serde(rename = "odrl:duty", default, skip_serializing_if = "Vec::is_empty")]
    pub duty: Vec<Rule>,
    /// `odrl:remedy` on this **prohibition**, per the ODRL 2.2 Information
    /// Model (§2.6.3): the duties that must be performed once the
    /// prohibition has been violated. Read only when this rule sits in
    /// `Policy::prohibitions`.
    ///
    /// **A remedy never lifts the prohibition** — see `decide`'s own doc
    /// comment for the full reasoning behind that sub-decision. It is
    /// resolved from claims exactly as any other duty here, and its only
    /// effect is to appear (when unresolved) in `DecisionOutcome::
    /// unresolved_duties` and in the `reason` trace.
    #[serde(rename = "odrl:remedy", default, skip_serializing_if = "Vec::is_empty")]
    pub remedy: Vec<Rule>,
    /// `odrl:consequence` on this **duty**, per the ODRL 2.2 Information
    /// Model (§2.6.2): the duty that applies when *this* duty is not
    /// fulfilled. Read wherever this rule is being evaluated in duty
    /// position — a policy-level obligation, a permission's `duty`, a
    /// prohibition's `remedy`, or another duty's own consequence.
    ///
    /// When the duty it hangs off is unresolved, the consequence becomes
    /// the duty actually evaluated (`outstanding_duty`); if the consequence
    /// resolves, nothing is outstanding and `dutyMode` has nothing to act
    /// on. If it does not, `dutyMode` governs exactly as it does for the
    /// duty it replaced.
    ///
    /// **One successor, chained, bounded** by
    /// [`MAX_CONSEQUENCE_DEPTH`]. ODRL permits a Duty to carry several
    /// consequences; this models one, because "the duty that becomes the
    /// one actually evaluated" needs a single successor to be well defined
    /// and the decided semantics state no rule for combining several. The
    /// successor may carry its own consequence, so this is a chain rather
    /// than a single level — bounded for the same reason
    /// `MAX_CONSTRAINT_DEPTH` bounds a constraint tree.
    #[serde(rename = "odrl:consequence", default, skip_serializing_if = "Option::is_none")]
    pub consequence: Option<Box<Rule>>,
}

/// The maximum number of `odrl:consequence` hops [`outstanding_duty`] will
/// follow from an attached duty before treating the chain as unresolved
/// instead of walking further. A duty itself is depth 0, so a duty plus
/// `MAX_CONSEQUENCE_DEPTH` consequences is the longest chain evaluated.
///
/// Mirrors `constraint::MAX_CONSTRAINT_DEPTH` in intent and, deliberately,
/// not in value. That bound is about a `Constraint` tree, which real ODRL
/// policies genuinely nest a few levels deep and which costs nothing to
/// allow generously; a consequence chain is a *deontic* escalation ("if you
/// do not notify, you must compensate; if you do not compensate, ..."), and
/// no policy in the corpora this workspace tracks — nor in ODRL 2.2's own
/// examples — chains more than one. 4 is chosen as small, obviously
/// sufficient, and still a chain rather than a single hardcoded level, so
/// the bound is a stated limit rather than an accidental one.
///
/// **Past the bound a duty stays unresolved**, never resolved: the
/// unwalked tail cannot report itself done. That is the same safe direction
/// `duty_satisfied` already takes for an unconditional duty, and the
/// opposite of `MAX_CONSTRAINT_DEPTH`'s deterministic non-*match* only in
/// wording — both refuse to credit input the evaluator declined to read.
/// See `wire.rs`'s
/// `a_consequence_chain_is_followed_up_to_max_consequence_depth_and_bounded_past_it`.
pub const MAX_CONSEQUENCE_DEPTH: usize = 4;

impl Rule {
    /// Builds a rule with no action refinement and no `odrl:target` —
    /// unchanged in meaning and call shape from before either field
    /// existed. An untargeted rule is about whatever the decision is being
    /// taken about, which is what every rule in this workspace's fixtures
    /// already implicitly was.
    pub fn new(action: impl Into<String>, constraints: Vec<Constraint>) -> Self {
        Self {
            action: action.into(),
            target: None,
            constraints,
            action_refinement: None,
            duty: Vec::new(),
            remedy: Vec::new(),
            consequence: None,
        }
    }

    /// Builds a duty carrying an `odrl:consequence` — the duty that applies
    /// when this one is not fulfilled. Separate from `new` for the same
    /// reason `refined` and `targeting` are: every existing call site keeps
    /// compiling, and a duty has no consequence unless someone says so.
    pub fn with_consequence(
        action: impl Into<String>,
        constraints: Vec<Constraint>,
        consequence: Rule,
    ) -> Self {
        Self {
            consequence: Some(Box::new(consequence)),
            ..Self::new(action, constraints)
        }
    }

    /// Builds a rule whose action carries an `odrl:refinement`. Separate
    /// from `new` rather than an extra parameter on it so that every
    /// existing call site in this workspace keeps compiling untouched.
    pub fn refined(action: impl Into<String>, constraints: Vec<Constraint>, refinement: Constraint) -> Self {
        Self {
            action_refinement: Some(refinement),
            ..Self::new(action, constraints)
        }
    }

    /// Builds a rule scoped to one asset by `odrl:target` — separate from
    /// `new` for exactly the reason `refined` is: every existing call site
    /// keeps compiling, and a rule is untargeted unless someone says
    /// otherwise. Compose with `refined` by field update
    /// (`Rule { target: Some(..), ..Rule::refined(..) }`) rather than by a
    /// four-parameter constructor.
    pub fn targeting(action: impl Into<String>, target: impl Into<String>, constraints: Vec<Constraint>) -> Self {
        Self {
            target: Some(target.into()),
            ..Self::new(action, constraints)
        }
    }

    /// `pub(crate)`, not private: Section 5.2's `wire` module re-derives
    /// *which* rule/constraint drove a decision for its human-readable
    /// `reason` trace, which needs the same match test `decide` uses.
    pub(crate) fn matches(&self, claims: &Claims) -> bool {
        self.constraints.iter().all(|c| c.evaluate(claims))
    }

    /// Does this rule's own declared action cover `requested_action` —
    /// `ResolvedConfig::covers`'s doc comment has the real semantics. A
    /// permission/prohibition rule is only ever considered for
    /// `matches()`/duty purposes once this also holds; duty satisfaction
    /// never calls this at all (a duty's action is what must be *done*,
    /// not what's being requested).
    ///
    /// This is the *bare action string* half of the action requirement
    /// only. It takes no claims and knows nothing about
    /// `action_refinement` — `action_applies` below is the whole question.
    pub(crate) fn covers_action(&self, requested_action: &str, config: &ResolvedConfig) -> bool {
        config.covers(&self.action, requested_action)
    }

    /// Is this rule's `odrl:refinement` (if any) satisfied by `claims`?
    /// A rule with no refinement is trivially satisfied — that is the
    /// today-shape case, and it must stay exactly as permissive as it was
    /// before this field existed.
    pub(crate) fn refinement_satisfied(&self, claims: &Claims) -> bool {
        self.action_refinement.as_ref().is_none_or(|c| c.evaluate(claims))
    }

    /// The full action requirement for a permission or prohibition: the
    /// rule's declared action covers `requested_action` **and** the
    /// action's own `odrl:refinement` holds. Both halves are about *which
    /// action this rule is about*, which is why they sit together here
    /// rather than the refinement being tacked onto `matches` — see
    /// `action_refinement`'s own doc comment.
    pub(crate) fn action_applies(&self, requested_action: &str, config: &ResolvedConfig, claims: &Claims) -> bool {
        self.covers_action(requested_action, config) && self.refinement_satisfied(claims)
    }

    /// Is this rule about `requested_target` — the asset the decision is
    /// being taken about? A rule naming no `odrl:target` is about whatever
    /// is being asked about, so it always applies; a rule naming one
    /// applies to that exact asset identifier, **or to any asset collection
    /// `asset_collections` asserts `requested_target` is a member of**
    /// (ODRL 2.2 Vocabulary §3.8.1's `odrl:partOf`, §3.4.2's
    /// `odrl:AssetCollection`) — a rule's `target` matches when it equals
    /// `requested_target` itself or appears anywhere in that list.
    ///
    /// **`asset_collections` is a host-supplied fact, not something this
    /// engine derives.** It names, from the wire contract's own
    /// `Request::asset_collections`, every collection the host already
    /// knows `requested_target` belongs to — resolved against the host's
    /// own catalog/graph before the request is built, exactly as
    /// `compliance-runner`'s own `is_member_of` adapter already does for
    /// the vendored compliance corpus (see this crate's README). This
    /// engine performs no graph traversal and no transitive closure: a
    /// two-level `odrl:partOf` chain (`member partOf sub-collection partOf
    /// collection`) matches a rule scoped to `collection` only if the host
    /// flattens that chain into `asset_collections` itself, one entry per
    /// ancestor, before calling this engine.
    ///
    /// Deliberately a plain string comparison — no IRI normalization — on
    /// both the direct-equality and the membership reading: "the same
    /// asset" and "a member of this collection" both mean "the same
    /// characters" here, the same honest limit `target` itself already
    /// documents.
    pub(crate) fn target_applies(&self, requested_target: &str, asset_collections: &[String]) -> bool {
        self.target
            .as_deref()
            .is_none_or(|t| t == requested_target || asset_collections.iter().any(|c| c == t))
    }

    /// The whole applicability question for a permission or prohibition:
    /// is this rule about the asset being asked about (`target_applies`)
    /// **and** about the action being asked about (`action_applies`,
    /// coverage plus the action's own `odrl:refinement`)? A rule that
    /// applies is then subject to its own `constraints` (`matches`), which
    /// is a separate condition on top and always was.
    ///
    /// The two halves are kept separate rather than merged because the
    /// `reason` trace has to be able to say *which* of them failed — a
    /// permission that misses on the asset and one that misses on the
    /// action are different mistakes for a policy author to have made
    /// (`wire::describe_reason`).
    pub(crate) fn applies(
        &self,
        requested_action: &str,
        requested_target: &str,
        asset_collections: &[String],
        config: &ResolvedConfig,
        claims: &Claims,
    ) -> bool {
        self.target_applies(requested_target, asset_collections) && self.action_applies(requested_action, config, claims)
    }

    /// Section 4.5's duty-satisfaction check — deliberately *not* the same
    /// as `matches`. An unconstrained permission/prohibition matches
    /// vacuously (an unconditional grant always applies), but an
    /// unconstrained duty is the opposite case: an unconditional "must do
    /// Y" this engine has no claims-based way to verify, so it is
    /// unresolved, not satisfied. Satisfied therefore requires at least
    /// one constraint, all of which match.
    ///
    /// An `odrl:refinement` on the duty's action is an *additional*
    /// requirement here, never a substitute for that at-least-one-
    /// constraint rule: a duty's action refinement narrows what would
    /// count as having done the duty (`notify`, refined to `by email`), so
    /// a refinement this engine cannot confirm from claims leaves the duty
    /// unresolved. That direction is the safe one — a refinement can only
    /// ever move a duty from resolved to unresolved, never the reverse —
    /// and it is why a duty carrying *only* a refinement and no
    /// constraints stays unresolved rather than becoming confirmable.
    pub(crate) fn duty_satisfied(&self, claims: &Claims) -> bool {
        !self.constraints.is_empty() && self.matches(claims) && self.refinement_satisfied(claims)
    }

    /// Are every one of this **permission**'s own `odrl:duty` chains
    /// resolved? Vacuously true for a permission carrying none, which is
    /// every rule this workspace's fixtures build.
    pub(crate) fn duties_resolved(&self, claims: &Claims) -> bool {
        self.duty.iter().all(|duty| outstanding_duty(duty, claims).is_none())
    }

    /// Does this permission actually grant — is it applicable
    /// (`applies`), does it match its own `constraints` (`matches`), and,
    /// under `DutyMode::Deny` only, are its per-permission `odrl:duty`
    /// chains all resolved?
    ///
    /// `pub(crate)` and factored out rather than inlined into `decide`
    /// because `wire::describe_reason` has to reconstruct exactly this
    /// question to trace *why* a request was denied. Two copies of the
    /// permission requirement is precisely the drift that produced the
    /// `beh-closed-empty` "denied for a reason this trace could not
    /// reconstruct" bug once already (see that function's own comment).
    pub(crate) fn grants(
        &self,
        requested_action: &str,
        requested_target: &str,
        asset_collections: &[String],
        config: &ResolvedConfig,
        claims: &Claims,
    ) -> bool {
        self.applies(requested_action, requested_target, asset_collections, config, claims)
            && self.matches(claims)
            && (config.duty_mode != DutyMode::Deny || self.duties_resolved(claims))
    }
}

/// The duty that is actually outstanding once `odrl:consequence` has been
/// followed from `duty`, or `None` when `duty` — or some duty in its
/// consequence chain — is satisfied from the claims.
///
/// This is the whole of the `odrl:consequence` semantics: a duty that is
/// not fulfilled does **not** immediately fall through to `dutyMode`
/// (Section 4.5's original behaviour, still exactly what happens for a duty
/// carrying no consequence). Its consequence becomes the duty actually
/// evaluated, recursively, up to [`MAX_CONSEQUENCE_DEPTH`] hops; only when
/// the chain runs out — or is cut off by the bound — is anything reported
/// outstanding, and `dutyMode` then governs that as it always did.
///
/// The outstanding duty reported is the **last one evaluated**, not the
/// first: the consequence is what the policy now requires, so a host acting
/// on this list needs the consequence's action, not the action it replaced.
/// `consequence_depth` says how many hops in that is, so a caller can tell
/// the two apart.
pub(crate) fn outstanding_duty(duty: &Rule, claims: &Claims) -> Option<OutstandingDuty> {
    outstanding_duty_at(duty, claims, 0)
}

fn outstanding_duty_at(duty: &Rule, claims: &Claims, depth: usize) -> Option<OutstandingDuty> {
    if duty.duty_satisfied(claims) {
        return None;
    }
    match &duty.consequence {
        // The bound is checked against the depth the *successor* would sit
        // at, so `MAX_CONSEQUENCE_DEPTH` hops are walked and the next is
        // not. An unwalked tail leaves this duty unresolved rather than
        // resolved — the safe direction, since nothing this evaluator
        // declined to read may report itself done.
        Some(next) if depth < MAX_CONSEQUENCE_DEPTH => outstanding_duty_at(next, claims, depth + 1),
        _ => Some(OutstandingDuty {
            action: duty.action.clone(),
            consequence_depth: depth,
        }),
    }
}

/// What [`outstanding_duty`] reports: which duty of a consequence chain is
/// still outstanding, and how far down the chain it sits (0 being the
/// attached duty itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutstandingDuty {
    pub action: String,
    pub consequence_depth: usize,
}

/// W3C ODRL 2.2's `odrl:conflict` (Information Model §2.10, the
/// `odrl:ConflictTerm` vocabulary): **what a policy means when one of its
/// own permissions and one of its own prohibitions both hold for the same
/// request**. Three values, exactly the three ODRL defines, spelled on the
/// wire exactly as ODRL spells them — the same enum-with-`serde(rename)`
/// shape [`Behaviour`] and [`DutyMode`] already use, and, like those two,
/// a term outside the enumeration is a parse failure rather than a
/// silently substituted default.
///
/// Consulted **only** for a genuine collision — see [`conflicting_rules`]
/// for the exact test and [`decide`] for where it branches. A policy in
/// which at most one of the two ever holds for a given request reaches the
/// same decision under all three values, which is every policy shape this
/// workspace's fixtures and the vendored compliance corpus contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictStrategy {
    /// The permission wins: a matching permission grants even though a
    /// matching prohibition also applies. The one combining rule this
    /// engine had no way to express at all before this field existed.
    #[serde(rename = "perm")]
    Perm,
    /// The prohibition wins — deny-overrides, which is exactly what this
    /// engine did unconditionally before this field existed, now a value a
    /// policy has to actually ask for.
    #[serde(rename = "prohibit")]
    Prohibit,
    /// The conflicting policy is **void**: neither rule resolves the
    /// other, so the policy authorizes nothing. Surfaced as `Decision::
    /// Deny` under a distinct `reason` (`wire::describe_reason`) rather
    /// than as a fourth `Decision` variant — see [`Policy::conflict`] for
    /// why a void policy is not a `Decision::Error`.
    #[serde(rename = "invalid")]
    Invalid,
}

impl Default for ConflictStrategy {
    /// `Invalid` — **ODRL's own stated default** for a policy that
    /// declares no `odrl:conflict` term, and a deliberate departure from
    /// this engine's own prior implicit behaviour (an unconditional,
    /// unnamed `Prohibit`).
    ///
    /// The departure is deliberate on a measured basis, and the measurement
    /// is the whole argument: **zero of the 68 fixtures in the vendored
    /// compliance corpus contain a policy carrying both a permission and a
    /// prohibition**, so no fixture is affected in either direction and
    /// there is nothing here to be compatible *with*. That is precisely the
    /// situation [`Behaviour`] is not in — an `Offer` with an empty
    /// `permissions` list is common real input, so `Behaviour::Open` keeps
    /// diverging from the Formal Semantics draft's `closed` default for an
    /// operational reason. There is no equivalent reason here, so this
    /// follows the spec.
    fn default() -> Self {
        ConflictStrategy::Invalid
    }
}

impl ConflictStrategy {
    /// `skip_serializing_if` for [`Policy::conflict`] and
    /// `wire::WirePolicy::conflict`: a policy meaning the default emits no
    /// key at all, so every document this engine produced before the field
    /// existed is byte-for-byte what it was.
    pub(crate) fn is_default(&self) -> bool {
        *self == ConflictStrategy::default()
    }
}

/// An ODRL Policy reduced to what Section 4.3's decision algorithm needs
/// — its permission and prohibition rules — plus Section 4.5's
/// policy-level duties. The profile/action machinery of Section 4.4
/// applies uniformly across all three lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub permissions: Vec<Rule>,
    #[serde(default)]
    pub prohibitions: Vec<Rule>,
    #[serde(default)]
    pub obligations: Vec<Rule>,
    /// `odrl:conflict` on this policy — the [`ConflictStrategy`] to apply
    /// when one of its permissions and one of its prohibitions both hold
    /// for the same request. A later, purely additive addition on the
    /// convention `Rule::target` and `Rule::action_refinement` already set:
    /// `#[serde(default)]` plus a `skip_serializing_if` on the default, at
    /// the `odrl:`-namespaced key `odrl:conflict` (real ODRL vocabulary,
    /// unlike this contract's original bare-named `permissions`/
    /// `prohibitions`).
    ///
    /// **Per policy, not per host.** ODRL puts `conflict` on the Policy,
    /// and so does this: the strategy travels with the document that
    /// contains the conflicting rules, rather than being one more knob in
    /// `ResolvedConfig` that would let a host silently reinterpret
    /// somebody else's policy. A host that controls the policies it sends
    /// (`compliance-runner`, `dsp-odrl-adapter`, any broker assembling a
    /// request) configures this by setting it on the policies it builds.
    ///
    /// **`Invalid` is a `Deny`, not a `Decision::Error`.** The distinction
    /// `Decision::Error` exists to preserve is "this is a *configuration
    /// gap* — load a profile that recognizes this action", which a caller
    /// fixes by changing its own setup. A void policy is not that: the
    /// policy parsed, every action in it was recognized, and the policy
    /// itself says the two rules cannot be reconciled. Nothing about the
    /// caller's configuration is wrong, and the enforcement answer is the
    /// same one an unsatisfied permission gets. What it does need is to be
    /// *distinguishable*, which is what the `reason` trace is for.
    #[serde(rename = "odrl:conflict", default, skip_serializing_if = "ConflictStrategy::is_default")]
    pub conflict: ConflictStrategy,
}

impl Policy {
    /// Every claim-map key (`Constraint::left_operand`) this policy's
    /// rules could actually test — across permissions, prohibitions **and**
    /// obligations alike, recursing into nested `odrl:and`/`odrl:or`/
    /// `odrl:xone` groupings at any depth. Sorted and deduplicated; see
    /// `Constraint::referenced_left_operands` for the walk's exact rules
    /// (logical nodes contribute nothing of their own, the
    /// `MAX_CONSTRAINT_DEPTH` bound applies here too) and for why sorted.
    ///
    /// **What this exists for.** This engine's whole claims model is that
    /// the host pushes a flat map it assembled from identity it already
    /// trusts, and a `left_operand` absent from that map is a *miss, not
    /// an error* (`Constraint::evaluate`). That posture is deliberate and
    /// stays — but it leaves a host with no way to know which claims a
    /// given set of policies actually wants, so it must push everything it
    /// has or guess. Guessing low is the dangerous direction: an
    /// unsupplied claim key silently turns a prohibition into a non-match,
    /// which is fail-*open* for exactly the rule kind where that direction
    /// of mistake matters most (the same asymmetry
    /// `first_unrecognized_action` above exists to protect). This call is
    /// how a host stops guessing.
    ///
    /// **What it is not.** It is a *reachability* answer, not a
    /// requirement: it says which keys could be consulted, not which must
    /// be present for any particular outcome. Nothing here reports that a
    /// claim is missing, mandatory, or sufficient — a rule the requested
    /// action never covers still contributes its operands (coverage
    /// depends on `requested_action` and the resolved config, neither of
    /// which this call takes), and `isNoneOf` is satisfied precisely *by*
    /// an absent key. A host wanting "which of these am I not carrying?"
    /// diffs this list against its own claims map; that diff is the
    /// host's policy call to make, not this engine's.
    pub fn referenced_left_operands(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        self.collect_left_operands(&mut names);
        names.into_iter().collect()
    }

    fn collect_left_operands(&self, out: &mut BTreeSet<String>) {
        // Each rule's own claim keys, then whichever nested duties this
        // engine actually *reads* in that rule's position — a permission's
        // `odrl:duty`, a prohibition's `odrl:remedy`, an obligation as a
        // duty in its own right — each followed down its `odrl:consequence`
        // chain. Walking only the positions evaluation reads keeps this a
        // reachability answer rather than an inventory of every key present
        // in the document.
        for rule in &self.permissions {
            collect_rule_left_operands(rule, out);
            for duty in &rule.duty {
                collect_duty_chain_left_operands(duty, 0, out);
            }
        }
        for rule in &self.prohibitions {
            collect_rule_left_operands(rule, out);
            for remedy in &rule.remedy {
                collect_duty_chain_left_operands(remedy, 0, out);
            }
        }
        for duty in &self.obligations {
            collect_duty_chain_left_operands(duty, 0, out);
        }
    }
}

/// One rule's own claim keys: its `constraints`, plus its
/// `odrl:refinement`, which is a `Constraint` this engine really evaluates
/// against the claims map. Omitting the refinement would tell a host to
/// gather less than the engine reads — the fail-open direction
/// `referenced_left_operands` exists to close.
fn collect_rule_left_operands(rule: &Rule, out: &mut BTreeSet<String>) {
    for constraint in &rule.constraints {
        constraint.collect_left_operands(0, out);
    }
    if let Some(refinement) = &rule.action_refinement {
        refinement.collect_left_operands(0, out);
    }
}

/// A duty's claim keys and those of every duty in its `odrl:consequence`
/// chain, stopping at the same [`MAX_CONSEQUENCE_DEPTH`] bound evaluation
/// stops at: a duty past the bound is never evaluated, so naming its claim
/// key would send a host to gather a claim that provably cannot change any
/// decision (the rule `Constraint::collect_left_operands` already applies
/// to its own depth bound).
fn collect_duty_chain_left_operands(duty: &Rule, depth: usize, out: &mut BTreeSet<String>) {
    collect_rule_left_operands(duty, out);
    if depth >= MAX_CONSEQUENCE_DEPTH {
        return;
    }
    if let Some(consequence) = &duty.consequence {
        collect_duty_chain_left_operands(consequence, depth + 1, out);
    }
}

/// [`Policy::referenced_left_operands`] across a whole policy set, unioned
/// — the shape a host actually holds, since one request carries several
/// policies (`wire::Request::policies`) and the claims map it assembles
/// has to serve all of them at once. Sorted and deduplicated across the
/// set, not merely per policy. An empty set references nothing.
///
/// A free function rather than a method because `Policy` is one policy and
/// this is a question about several; `wire::left_operands_for_request` is
/// the same question asked one level up, off a `Request`'s own
/// `WirePolicy` list.
pub fn referenced_left_operands(policies: &[Policy]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for policy in policies {
        policy.collect_left_operands(&mut names);
    }
    names.into_iter().collect()
}

/// Which of a `Policy`'s rule lists an `UnrecognizedAction` (or, for
/// `Duty`, an `UnresolvedDuty`) came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Permission,
    Prohibition,
    Duty,
}

impl fmt::Display for RuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleKind::Permission => write!(f, "permission"),
            RuleKind::Prohibition => write!(f, "prohibition"),
            RuleKind::Duty => write!(f, "duty"),
        }
    }
}

/// Section 4.4's unrecognized-action outcome: naming both the action and
/// which rule of the policy raised it, since `catalog_core::Policy` has
/// no policy-level identifier of its own for this evaluation to cite
/// (Section 4.4's decision is per-`(Policy, claims, config)`, one policy
/// at a time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecognizedAction {
    pub action: String,
    pub rule_kind: RuleKind,
    pub rule_index: usize,
    /// Set only for an action found on a **nested** duty — a permission's
    /// `odrl:duty`, a prohibition's `odrl:remedy`, or anything down an
    /// `odrl:consequence` chain — carrying that duty's full path
    /// ([`duty_path`]), since `rule_index` alone cannot say which rule's
    /// duty list it indexes. `None` for every rule of the three lists
    /// `Policy` itself carries, which is every case that existed before
    /// nested duties did, and which therefore prints exactly the message it
    /// always printed.
    pub duty_path: Option<String>,
}

impl fmt::Display for UnrecognizedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.duty_path {
            None => write!(
                f,
                "unrecognized action \"{}\" in {} rule at index {}: no loaded profile's \
                 recognized_actions includes it",
                self.action, self.rule_kind, self.rule_index
            ),
            Some(path) => write!(
                f,
                "unrecognized action \"{}\" in the duty at {path}: no loaded profile's \
                 recognized_actions includes it",
                self.action
            ),
        }
    }
}

/// The result of Section 4.3's algorithm, extended by Section 4.4's
/// unrecognized-action check.
///
/// `Error` is deliberately its own outcome, not folded into `Deny`: per
/// Section 4.4, a broker consuming it **must** treat it as fail-closed at
/// its own boundary, but it is a configuration gap ("load a profile that
/// recognizes this action"), not a policy decision, and collapsing it
/// into `Deny` would make that distinction unrecoverable downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Error(UnrecognizedAction),
}

/// Section 4.5's outcome for one policy-level duty that did not resolve
/// (its constraints did not all match, or it is unconditional): named and
/// indexed the same way `UnrecognizedAction` is, since `catalog_core::Rule`
/// carries no duty-level identifier of its own either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedDuty {
    pub action: String,
    pub duty_index: usize,
    /// Which of the three attachment points this duty hangs off. A later,
    /// additive field: `DutyAttachment::Obligation` is Section 4.5's
    /// original policy-level duty and the only value any fixture in this
    /// workspace produces.
    pub attachment: DutyAttachment,
    /// How many `odrl:consequence` hops from the attached duty this
    /// outstanding one sits — 0 being the attached duty itself, which is
    /// every case before `odrl:consequence` existed.
    pub consequence_depth: usize,
}

/// Where an unresolved duty was attached — the three ODRL positions a Duty
/// can occupy in this engine's model, distinguished so a caller (and the
/// `reason` trace) can tell a policy-wide obligation from a pre-condition
/// on one permission from a prohibition's remedy. They behave differently
/// enough that collapsing them would misreport what a host has to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DutyAttachment {
    /// `Policy::obligations[duty_index]` — Section 4.5's policy-level duty.
    Obligation,
    /// `Policy::permissions[rule_index].duty[duty_index]`.
    PermissionDuty { rule_index: usize },
    /// `Policy::prohibitions[rule_index].remedy[duty_index]`.
    ProhibitionRemedy { rule_index: usize },
}

/// Renders one duty's position as the short path both the `reason` trace
/// and Section 5.2's `duties` entries use — `duty[0]`,
/// `permission[1].duty[0]`, `prohibition[0].remedy[0]`, each with one
/// `.consequence` segment per hop walked. Shared by both so the two can
/// never disagree about what to call the same duty.
pub(crate) fn duty_path(attachment: DutyAttachment, duty_index: usize, consequence_depth: usize) -> String {
    let mut path = match attachment {
        DutyAttachment::Obligation => format!("duty[{duty_index}]"),
        DutyAttachment::PermissionDuty { rule_index } => format!("permission[{rule_index}].duty[{duty_index}]"),
        DutyAttachment::ProhibitionRemedy { rule_index } => {
            format!("prohibition[{rule_index}].remedy[{duty_index}]")
        }
    };
    for _ in 0..consequence_depth {
        path.push_str(".consequence");
    }
    path
}

impl UnresolvedDuty {
    /// This duty's position, per [`duty_path`].
    pub fn path(&self) -> String {
        duty_path(self.attachment, self.duty_index, self.consequence_depth)
    }

    /// `true` for the shape that existed before per-permission duties,
    /// consequences and remedies did: a policy-level obligation, itself
    /// outstanding rather than some consequence of it. Section 5.2's
    /// `duties` entries omit their `source` field for exactly these, so
    /// every response this engine produced before this addition is
    /// byte-for-byte what it was.
    pub(crate) fn is_plain_policy_obligation(&self) -> bool {
        self.attachment == DutyAttachment::Obligation && self.consequence_depth == 0
    }
}

/// `decide`'s full result: Section 4.3/4.4's `Decision`, plus Section
/// 4.5's list of duties this engine could not confirm. The list is
/// populated independently of `duty_mode` — it is the caller's advisory
/// record of every unresolved duty, whether or not that duty also forced
/// `decision` to `Deny` under `duty_mode: "deny"`. Section 5.2's `duties`
/// response field, in the next revision, filters this by `duty_mode`
/// before it reaches a caller; this type is the pre-filter source of
/// truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionOutcome {
    pub decision: Decision,
    pub unresolved_duties: Vec<UnresolvedDuty>,
}

/// Section 4.4's unrecognized-action check, ahead of Section 4.3's
/// algorithm: prohibitions first, then permissions, then Section 4.5's
/// obligations, so the earliest rule (in the same precedence order
/// `decide` itself evaluates) that names an action outside
/// `config.recognized_actions` is the one reported. A duty's `action` is
/// checked on the same footing as a permission's or prohibition's — an
/// obligation this engine cannot even identify is no safer to guess about
/// than a permission or prohibition it cannot identify (Section 4.4).
///
/// This checks *every* rule's action regardless of whether its
/// constraints would otherwise match — Section 4.4 is explicit that an
/// unrecognized action is unsafe to treat as an ordinary non-match in
/// either direction (silent fail-open inside a `Prohibition`, silent
/// fail-closed indistinguishable from an intended `Deny` inside a
/// `Permission`), so it cannot be left to `Rule::matches` to notice only
/// incidentally.
fn first_unrecognized_action(policy: &Policy, config: &ResolvedConfig) -> Option<UnrecognizedAction> {
    for (rule_index, rule) in policy.prohibitions.iter().enumerate() {
        if !config.recognizes(&rule.action) {
            return Some(UnrecognizedAction {
                action: rule.action.clone(),
                rule_kind: RuleKind::Prohibition,
                rule_index,
                duty_path: None,
            });
        }
    }

    for (rule_index, rule) in policy.permissions.iter().enumerate() {
        if !config.recognizes(&rule.action) {
            return Some(UnrecognizedAction {
                action: rule.action.clone(),
                rule_kind: RuleKind::Permission,
                rule_index,
                duty_path: None,
            });
        }
    }

    for (rule_index, rule) in policy.obligations.iter().enumerate() {
        if !config.recognizes(&rule.action) {
            return Some(UnrecognizedAction {
                action: rule.action.clone(),
                rule_kind: RuleKind::Duty,
                rule_index,
                duty_path: None,
            });
        }
    }

    first_unrecognized_nested_duty_action(policy, config)
}

/// Section 4.4's same check, extended to the duties that hang off a rule
/// rather than off the policy: a permission's `odrl:duty`, a prohibition's
/// `odrl:remedy`, and every `odrl:consequence` reachable from any of those
/// or from a policy-level obligation.
///
/// **Why these are checked at all.** A policy-level obligation naming an
/// action outside every loaded profile's vocabulary is already a
/// `Decision::Error` (the loop above), on Section 4.4's reasoning that an
/// obligation this engine cannot even identify is no safer to guess about
/// than a permission it cannot identify. An identical duty attached to a
/// permission instead of to the policy is the same configuration gap, and
/// leaving it unchecked would make the outcome depend on where the policy
/// author put the duty rather than on what it says.
///
/// **Why it runs as a separate pass, after all three of the original
/// loops.** So that every policy shape that existed before nested duties
/// did reports exactly the rule it always reported: this pass can only
/// change the answer for a policy that actually carries a nested duty.
fn first_unrecognized_nested_duty_action(policy: &Policy, config: &ResolvedConfig) -> Option<UnrecognizedAction> {
    // Prohibitions, then permissions, then obligations — the same
    // precedence order the three loops above already use.
    for (rule_index, rule) in policy.prohibitions.iter().enumerate() {
        for (duty_index, remedy) in rule.remedy.iter().enumerate() {
            let attachment = DutyAttachment::ProhibitionRemedy { rule_index };
            if let Some(found) = first_unrecognized_in_duty_chain(remedy, config, attachment, duty_index, 0) {
                return Some(found);
            }
        }
    }
    for (rule_index, rule) in policy.permissions.iter().enumerate() {
        for (duty_index, duty) in rule.duty.iter().enumerate() {
            let attachment = DutyAttachment::PermissionDuty { rule_index };
            if let Some(found) = first_unrecognized_in_duty_chain(duty, config, attachment, duty_index, 0) {
                return Some(found);
            }
        }
    }
    for (duty_index, obligation) in policy.obligations.iter().enumerate() {
        // The obligation's own action was checked by the loop above; only
        // its consequence chain is new here.
        if let Some(consequence) = &obligation.consequence {
            let attachment = DutyAttachment::Obligation;
            if let Some(found) = first_unrecognized_in_duty_chain(consequence, config, attachment, duty_index, 1) {
                return Some(found);
            }
        }
    }
    None
}

/// Walks one duty and its `odrl:consequence` chain — bounded by
/// [`MAX_CONSEQUENCE_DEPTH`], the same bound evaluation itself stops at, so
/// this never reports a duty that could not have influenced a decision
/// anyway — returning the first action no loaded profile declares.
fn first_unrecognized_in_duty_chain(
    duty: &Rule,
    config: &ResolvedConfig,
    attachment: DutyAttachment,
    duty_index: usize,
    depth: usize,
) -> Option<UnrecognizedAction> {
    if !config.recognizes(&duty.action) {
        return Some(UnrecognizedAction {
            action: duty.action.clone(),
            rule_kind: RuleKind::Duty,
            rule_index: duty_index,
            duty_path: Some(duty_path(attachment, duty_index, depth)),
        });
    }
    if depth >= MAX_CONSEQUENCE_DEPTH {
        return None;
    }
    duty.consequence
        .as_ref()
        .and_then(|next| first_unrecognized_in_duty_chain(next, config, attachment, duty_index, depth + 1))
}

/// Section 4.5's duty evaluation: every policy-level duty whose
/// constraints do not all match (or which has none at all — an
/// unconditional duty, Section 4.5) is unresolved.
///
/// Takes neither the requested action nor the requested target, and that
/// is the same decision twice: a duty's `action` is what must be *done*,
/// not what is being asked for, and a duty's `odrl:target` (if it carries
/// one) is the asset that duty is to be performed on — an audit log to
/// write, a copy to delete — which need not be, and often is not, the
/// asset under request. Scoping duties by the requested target would
/// silently drop obligations a policy really does attach to the very
/// permission being exercised, so this engine leaves a duty's own target
/// as descriptive data it carries and does not evaluate.
///
/// Each obligation is resolved through `outstanding_duty`, so a duty
/// carrying an `odrl:consequence` falls through to it on non-fulfilment
/// rather than straight to `dutyMode`. A duty with no consequence behaves
/// exactly as it always did.
fn unresolved_obligations(policy: &Policy, claims: &Claims) -> Vec<UnresolvedDuty> {
    policy
        .obligations
        .iter()
        .enumerate()
        .filter_map(|(duty_index, duty)| {
            outstanding_duty(duty, claims).map(|outstanding| UnresolvedDuty {
                action: outstanding.action,
                duty_index,
                attachment: DutyAttachment::Obligation,
                consequence_depth: outstanding.consequence_depth,
            })
        })
        .collect()
}

/// The per-permission `odrl:duty` chains that are outstanding, across every
/// permission that is actually **in play** for this decision (applicable
/// and matching its own constraints).
///
/// Scoping to permissions in play is the whole difference from
/// `unresolved_obligations` above: a policy-level obligation is the
/// policy's, unconditionally, but a duty attached to a permission is a
/// pre-condition *of that permission*. A permission this request never
/// reaches — wrong action, wrong asset, constraints missed — imposes
/// nothing, so reporting its duty would send a host chasing an obligation
/// it does not have.
fn unresolved_permission_duties(
    policy: &Policy,
    claims: &Claims,
    config: &ResolvedConfig,
    requested_action: &str,
    requested_target: &str,
    asset_collections: &[String],
) -> Vec<UnresolvedDuty> {
    let mut outstanding = Vec::new();
    for (rule_index, rule) in policy.permissions.iter().enumerate() {
        if !(rule.applies(requested_action, requested_target, asset_collections, config, claims) && rule.matches(claims))
        {
            continue;
        }
        for (duty_index, duty) in rule.duty.iter().enumerate() {
            if let Some(found) = outstanding_duty(duty, claims) {
                outstanding.push(UnresolvedDuty {
                    action: found.action,
                    duty_index,
                    attachment: DutyAttachment::PermissionDuty { rule_index },
                    consequence_depth: found.consequence_depth,
                });
            }
        }
    }
    outstanding
}

/// The `odrl:remedy` chains that are outstanding, across every prohibition
/// that actually **fired** — applied, matched, and so denied.
///
/// A remedy is what must be done *once the prohibition has been violated*,
/// so a prohibition that did not fire imposes no remedy: reporting one
/// would invent an obligation out of a rule that had nothing to say. This
/// mirrors the permission-duty scoping above, at the opposite polarity.
fn unresolved_remedies(
    policy: &Policy,
    claims: &Claims,
    config: &ResolvedConfig,
    requested_action: &str,
    requested_target: &str,
    asset_collections: &[String],
) -> Vec<UnresolvedDuty> {
    let mut outstanding = Vec::new();
    for (rule_index, rule) in policy.prohibitions.iter().enumerate() {
        if !(rule.applies(requested_action, requested_target, asset_collections, config, claims) && rule.matches(claims))
        {
            continue;
        }
        for (duty_index, remedy) in rule.remedy.iter().enumerate() {
            if let Some(found) = outstanding_duty(remedy, claims) {
                outstanding.push(UnresolvedDuty {
                    action: found.action,
                    duty_index,
                    attachment: DutyAttachment::ProhibitionRemedy { rule_index },
                    consequence_depth: found.consequence_depth,
                });
            }
        }
    }
    outstanding
}

/// The two rules of a genuine `odrl:conflict` collision, when there is
/// one: the index of the first permission that actually **grants** and the
/// index of the first prohibition that actually **denies**, for this exact
/// `requested_action`/`requested_target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConflictingRules {
    pub(crate) permission_index: usize,
    pub(crate) prohibition_index: usize,
}

/// **What "conflict" means here, precisely.** A permission and a
/// prohibition of the same policy collide when both would independently
/// have decided this request: a prohibition that applies
/// (`Rule::applies` — right asset, right action, refinement satisfied) and
/// matches its own constraints, *and* a permission that `Rule::grants`,
/// for the identical `requested_action` and `requested_target`. Only then
/// is [`Policy::conflict`] consulted at all; when at most one of the two
/// holds, the decision is whatever that one rule already made it, under
/// every strategy alike.
///
/// **`Rule::grants`, not `applies() && matches()`, on the permission
/// side** — the same predicate `decide` uses for the permission
/// requirement. A permission whose own `odrl:duty` is outstanding under
/// `DutyMode::Deny` is not in force, so it is not a party to a conflict
/// either, and `perm` cannot promote it over a prohibition. Using a looser
/// test here would make `perm` able to grant through a duty gate that
/// `decide` itself treats as closed.
///
/// **Deliberately *not* keyed off `permission_requirement_met`.** Under
/// `Behaviour::Open` a policy with an empty `permissions` list meets the
/// permission requirement vacuously — but there is no permission there to
/// win anything, so a matching prohibition denies under every strategy,
/// `perm` included. A conflict needs two actual rules.
///
/// `pub(crate)` and shared rather than re-derived: `wire::describe_reason`
/// has to name both rules of a collision in its trace, and two copies of
/// this test drifting apart is exactly the failure mode that produced the
/// `beh-closed-empty` "denied for a reason this trace could not
/// reconstruct" bug once already.
pub(crate) fn conflicting_rules(
    policy: &Policy,
    claims: &Claims,
    config: &ResolvedConfig,
    requested_action: &str,
    requested_target: &str,
    asset_collections: &[String],
) -> Option<ConflictingRules> {
    let prohibition_index = policy.prohibitions.iter().position(|rule| {
        rule.applies(requested_action, requested_target, asset_collections, config, claims) && rule.matches(claims)
    })?;
    let permission_index = policy
        .permissions
        .iter()
        .position(|rule| rule.grants(requested_action, requested_target, asset_collections, config, claims))?;
    Some(ConflictingRules { permission_index, prohibition_index })
}

/// Section 4.3's decision algorithm: deny-overrides, then a permission
/// requirement — gated by Section 4.4's unrecognized-action check, and
/// followed by Section 4.5's duty evaluation.
///
/// **The ODRL Community Group's `Behaviour` axis (Section 3.6) is now a
/// real, host-configurable parameter (`config.behaviour`), not a fixed
/// choice baked into this function.** Under `Behaviour::Open` — Section
/// 4.3's own original, unconditional default — a policy with an empty
/// `permissions` list has its permission requirement met vacuously
/// (because an empty-`permissions` `Offer` is the common harvested-data
/// case, not the exception); under `Behaviour::Closed`, it is not: the
/// permission requirement is met only by an actual covering, matching
/// permission, same as a non-empty list under either setting. This
/// governs *only* that one degenerate case — a matching prohibition
/// still denies, and a non-empty `permissions` list that never matches
/// still denies, under either setting.
///
/// **Deny-overrides is no longer unconditional either.** When a granting
/// permission and a denying prohibition both hold for this exact request —
/// and only then ([`conflicting_rules`]) — the policy's own
/// `odrl:conflict` term ([`Policy::conflict`]) decides: `perm` lets the
/// permission win, `prohibit` is the deny-overrides this function applied
/// unconditionally before the term existed, and `invalid` — **ODRL's own
/// default, and therefore this engine's for a policy that declares
/// nothing** — voids the policy, which surfaces as `Deny` under a distinct
/// `reason` rather than as a fourth `Decision`. A policy in which at most
/// one of the two ever holds is entirely unaffected, under every strategy.
///
/// Duty evaluation (Section 4.5) runs after the permission/prohibition
/// decision is reached, and can only ever *tighten* it: under
/// `duty_mode: "deny"`, an unresolved duty overrides an otherwise-`Allow`
/// decision to `Deny` (a decision already `Deny` via a matching
/// prohibition is unaffected — it cannot be denied twice). Under
/// `duty_mode: "advise"`, unresolved duties never change `decision`,
/// only the returned `unresolved_duties` list.
///
/// `requested_action` is the one action this whole decision is *about* —
/// a permission or prohibition rule is only in play if it covers this
/// action (`Rule::covers_action`, exact match or a declared `includedIn`
/// chain) **and** its action's own `odrl:refinement`, if it carries one,
/// is satisfied by `claims` (`Rule::action_applies`). The two together
/// are the action requirement; the rule's own `constraints` are then a
/// separate condition on top, as they always were.
///
/// `requested_target` is the one asset this decision is about, and is the
/// exact counterpart of `requested_action` for `Rule::target`: a rule
/// naming an `odrl:target` is in play only for that asset, and a rule
/// naming none is in play for whatever asset is asked about
/// (`Rule::target_applies`). At the wire level this is `Request::
/// dataset_id` — the asset handle that contract already carried — rather
/// than a second, separate field; see `wire::evaluate_request_for_action`.
/// **A caller must pass the asset it is actually deciding about**: passing
/// something else silently makes every explicitly-targeted rule
/// inapplicable, which for a prohibition is the fail-open direction. That
/// is the reason this is a required parameter rather than an
/// `Option<&str>` defaulting to "matches everything".
///
/// `asset_collections` is a later, purely additive widening of
/// `requested_target` itself: the `odrl:AssetCollection`s (ODRL 2.2
/// Vocabulary §3.4.2) that `requested_target` is asserted to be
/// `odrl:partOf` (§3.8.1), so a rule scoped to a collection IRI is in
/// play for a request naming any asset the host asserts is a member of
/// it, not only the collection IRI itself (`Rule::target_applies`). An
/// empty slice — every call site before this parameter existed — is
/// exactly the prior behaviour: a targeted rule matches only the literal
/// `requested_target` string. This engine resolves no membership itself
/// (no graph, no transitive closure); the host supplies the list already
/// flattened, the same division of labour `requested_target` itself
/// already has with the profile-declared action taxonomy.
///
/// **Targets are never a `Decision::Error`, unlike actions.** Section
/// 4.4's unrecognized-action check exists because a profile declares the
/// action vocabulary, so an action outside it is a demonstrable
/// configuration gap. Nothing declares an asset vocabulary anywhere in
/// this engine, so a rule naming a target no one has heard of is not
/// distinguishable from a rule about an asset this request is simply not
/// about — an ordinary non-match, and the only honest reading.
///
/// **A duty's own target is not checked here**, on exactly the footing
/// `requested_action` is already not checked against a duty's action: a
/// policy-level duty says what must be *done* (`notify`, perhaps about
/// some audit log), not what is being asked for, so scoping it by the
/// requested asset would silently drop duties a policy really does attach.
/// See `unresolved_obligations` below.
///
/// # Duties beyond the policy level
///
/// Section 4.5 attached duties to the policy. Three more ODRL attachment
/// points are evaluated here, all of them the *same* mechanism at a
/// different position — a claims-precondition read by `dutyMode`, never
/// an observation that anything was performed:
///
/// - **A permission's `odrl:duty`** (`Rule::duty`) is a pre-condition of
///   that one permission. Under `DutyMode::Advise` the permission still
///   grants and the duty is reported; under `DutyMode::Deny` that
///   permission does not grant, while any sibling permission with nothing
///   outstanding still does. Only permissions actually in play contribute.
/// - **A duty's `odrl:consequence`** (`Rule::consequence`) is the duty that
///   applies when the duty it hangs off is not fulfilled. It replaces that
///   duty as the one evaluated, chained up to `MAX_CONSEQUENCE_DEPTH`; when
///   the chain also fails to resolve, `dutyMode` governs exactly as before.
///   See `outstanding_duty`.
/// - **A prohibition's `odrl:remedy`** (`Rule::remedy`) is what must be done
///   once the prohibition has been violated. Only prohibitions that
///   actually fired contribute one.
///
/// ## The remedy sub-decision, stated plainly
///
/// **A satisfied remedy does not lift the prohibition.** A prohibition
/// that applies and matches denies, and its remedy — resolved or not —
/// only ever adds an entry to `unresolved_duties` and a clause to the
/// `reason` trace. The alternative reading (ODRL's "the remedy substitutes
/// for the violation", turning a would-be `Deny` into an
/// `Allow`-with-a-duty) was considered and rejected on three grounds:
///
/// 1. **Duties in this engine only ever tighten a decision.** Section
///    4.5's duty step can move `Allow` to `Deny` under `DutyMode::Deny` and
///    can never move `Deny` to `Allow`. A remedy that flipped a denial
///    would be the first duty here that loosens one, which contradicts the
///    very pattern this addition was asked to extend rather than redesign.
/// 2. **"Satisfied" here is a host-supplied claim, not an observation.**
///    This engine cannot see that a remedy was performed; it sees that the
///    claims map says so. Letting one such claim erase a prohibition would
///    make the engine's most consequential rule the easiest thing in the
///    contract to switch off — a strictly worse fail-open than the adapter
///    bug this repo's README already records for exactly this construct.
/// 3. **ODRL's remedy is consequent on the violation, not a licence for
///    it.** The prohibited act still happened; the policy's response is to
///    demand something further, which is what an outstanding duty entry and
///    a named clause in the trace say.
///
/// An **unresolved** remedy therefore behaves exactly analogously to an
/// unresolved obligation under the current `dutyMode` semantics — reported,
/// and unable to make the decision any more permissive than it already is.
pub fn decide(
    policy: &Policy,
    claims: &Claims,
    config: &ResolvedConfig,
    requested_action: &str,
    requested_target: &str,
    asset_collections: &[String],
) -> DecisionOutcome {
    if let Some(unrecognized) = first_unrecognized_action(policy, config) {
        return DecisionOutcome {
            decision: Decision::Error(unrecognized),
            unresolved_duties: Vec::new(),
        };
    }

    let denied_by_prohibition = policy.prohibitions.iter().any(|rule| {
        rule.applies(requested_action, requested_target, asset_collections, config, claims) && rule.matches(claims)
    });

    // `Rule::grants`, not `applies() && matches()`: under
    // `DutyMode::Deny` a permission whose own `odrl:duty` chain is
    // outstanding does not grant. That gating is scoped to the one
    // permission — a sibling permission with nothing outstanding still
    // grants, which is exactly what makes a per-permission duty different
    // from a policy-level obligation below.
    let any_permission_grants = policy
        .permissions
        .iter()
        .any(|rule| rule.grants(requested_action, requested_target, asset_collections, config, claims));
    let permission_requirement_met = match config.behaviour {
        Behaviour::Open => policy.permissions.is_empty() || any_permission_grants,
        Behaviour::Closed => any_permission_grants,
    };

    // ODRL's `odrl:conflict`, and the *only* place it is consulted: both a
    // granting permission and a denying prohibition hold for this exact
    // request, so the policy's own strategy decides which reading wins —
    // or, under the default `invalid`, that neither does and the policy is
    // void. When at most one of the two holds there is nothing to
    // reconcile, and the three arms below are unreachable: the ordinary
    // deny-overrides/permission-requirement branches decide exactly as they
    // always have, under every strategy alike.
    let mut decision = if denied_by_prohibition && any_permission_grants {
        match policy.conflict {
            ConflictStrategy::Perm => Decision::Allow,
            // Same decision, different reasons — the trace tells a policy
            // that chose prohibition-first apart from a policy that refused
            // to resolve the conflict at all.
            ConflictStrategy::Prohibit | ConflictStrategy::Invalid => Decision::Deny,
        }
    } else if denied_by_prohibition {
        Decision::Deny
    } else if permission_requirement_met {
        Decision::Allow
    } else {
        Decision::Deny
    };

    // Policy-level obligations first, then the two narrower attachment
    // points — the order Section 5.2's `duties` list reports them in, and
    // the order that keeps every response this engine produced before
    // nested duties existed byte-for-byte what it was (a policy carrying
    // neither a per-permission duty nor a remedy contributes nothing to
    // either of the two tails).
    let mut unresolved_duties = unresolved_obligations(policy, claims);
    let obligation_outstanding = !unresolved_duties.is_empty();
    unresolved_duties.extend(unresolved_permission_duties(
        policy,
        claims,
        config,
        requested_action,
        requested_target,
        asset_collections,
    ));
    unresolved_duties.extend(unresolved_remedies(
        policy,
        claims,
        config,
        requested_action,
        requested_target,
        asset_collections,
    ));

    // Deliberately keyed off the *policy-level* obligations alone, not off
    // the whole list. An unresolved obligation is the policy's and denies
    // the request outright, exactly as Section 4.5 has always had it; an
    // unresolved per-permission duty has already had its effect, scoped to
    // its own permission, in `Rule::grants` above, and denying here as well
    // would silently un-scope it (a sibling permission that does grant
    // would stop mattering). An unresolved remedy never denies anything
    // either, because its prohibition already did.
    if config.duty_mode == DutyMode::Deny && obligation_outstanding {
        decision = Decision::Deny;
    }

    DecisionOutcome {
        decision,
        unresolved_duties,
    }
}

/// Which of the actions `config` declares this caller could actually
/// perform against `policy`, **on the asset `requested_target` names** —
/// `decide` asked once per declared action, keeping the ones that answer
/// `Decision::Allow`. The target is per-call rather than per-action for
/// the same reason it is a parameter of `decide` itself: a policy may
/// scope individual rules to individual assets (`Rule::target`), so "what
/// may I do" is only ever answerable about one asset at a time. Sorted and deduplicated
/// (the same stable-ordering convention `referenced_left_operands` above
/// and `profile-interpreter`'s `declared_left_operands` already set: a set
/// of action names has no meaningful intrinsic order, and a stable one is
/// diffable and safe to print).
///
/// **What this exists for.** Every other entry point here answers one
/// yes/no question about one action the caller already had in mind. A
/// broker rendering a catalog has the opposite question: not "may this
/// caller `use` dataset 7", asked 400 times, but "which of the actions my
/// vocabulary declares could this caller perform at all", so it can filter
/// or grey out what it shows. Nothing stopped a host from writing this
/// loop itself — but writing it correctly means knowing that the
/// enumeration domain is `ResolvedConfig`'s declared actions and not, say,
/// the actions the policy's own rules happen to name (which would miss
/// every action reachable only through an `odrl:includedIn` edge from a
/// broader rule, exactly the coverage `decide` exists to resolve).
///
/// **A thin wrapper over `decide`, deliberately — no second decision
/// algorithm.** Every semantic below is inherited, not restated here:
///
/// - **`Decision::Error` yields an empty list**, because it yields `Error`
///   for *every* action: a rule naming an action outside the declared
///   vocabulary is a configuration gap the caller must treat as
///   fail-closed (Section 4.4), and reporting the remaining actions as
///   performable would launder that `Error` into a partial allow-list.
/// - **`Behaviour` is honoured as-is.** Under `Behaviour::Open`, a policy
///   with an empty `permissions` list is vacuously met, so *every* declared
///   action comes back — the honest answer for that configuration, and a
///   caller that finds it surprising wants `Behaviour::Closed`, which is
///   the parameter for exactly that.
/// - **`DutyMode::Deny` is honoured as-is**: an unresolved duty denies
///   every action, so the list is empty. An action in the list may still
///   carry unresolved *advisory* duties — this call reports only which
///   actions allow, never which duties came with them; a caller that needs
///   the duties calls `decide` for the specific action it is proceeding
///   with.
///
/// **One policy, mirroring `decide` itself.** There is deliberately no
/// `&[Policy]` form here, unlike `referenced_left_operands` above: unioning
/// claim keys across policies is well-defined at this layer, but combining
/// *decisions* across policies is not — that rule (deny-override across the
/// set, `Error` > `Deny` > `Allow`) lives in `wire::evaluate_request` and
/// is this implementation's own choice, which the case study leaves
/// formally undefined. A policy-set enumeration that unioned per-policy
/// answers here would silently contradict it, reporting as performable an
/// action some other policy in the same request prohibits.
/// `wire::performable_actions_for_request` is the policy-set form, and it
/// gets the combining right by going through `evaluate_request` rather than
/// through this function.
pub fn performable_actions(
    policy: &Policy,
    claims: &Claims,
    config: &ResolvedConfig,
    requested_target: &str,
    asset_collections: &[String],
) -> Vec<String> {
    let mut allowed = BTreeSet::new();
    for action in config.declared_actions() {
        if decide(policy, claims, config, action, requested_target, asset_collections).decision == Decision::Allow {
            allowed.insert(action.to_string());
        }
    }
    allowed.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::ClaimValue;
    use crate::constraint::Operator;
    use crate::profile::{ActionDecl, DutyMode, Profile};

    /// The asset every fixture below is about unless it says otherwise —
    /// the requested target `decide` now takes beside the requested action.
    /// Almost every rule in these fixtures is untargeted, so for those this
    /// value is deliberately arbitrary: an untargeted rule is about
    /// whatever is being asked about, which is the whole
    /// backward-compatibility claim of the `odrl:target` field.
    const ASSET: &str = "urn:uuid:test-asset";

    fn claims_with(pairs: &[(&str, ClaimValue)]) -> Claims {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn flat(names: &[&str]) -> Vec<ActionDecl> {
        names.iter().map(|n| ActionDecl::new(*n)).collect()
    }

    fn config_recognizing(actions: &[&str]) -> ResolvedConfig {
        config_with_duty_mode(actions, DutyMode::Advise)
    }

    fn config_with_duty_mode(actions: &[&str], duty_mode: DutyMode) -> ResolvedConfig {
        config_with(actions, duty_mode, Behaviour::Open)
    }

    fn config_with(actions: &[&str], duty_mode: DutyMode, behaviour: Behaviour) -> ResolvedConfig {
        crate::profile::resolve(&[Profile {
            id: "https://example.org/profiles/test".to_string(),
            actions: flat(actions),
            duty_mode,
            behaviour,
        }])
    }

    fn all_actions_config() -> ResolvedConfig {
        config_recognizing(&["read", "write"])
    }

    #[test]
    fn permission_only_unconstrained_allows() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision, Decision::Allow);
    }

    #[test]
    fn permission_only_satisfied_constraint_allows() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision, Decision::Allow);
    }

    #[test]
    fn prohibition_overrides_a_matching_permission() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision,
            Decision::Deny,
            "deny-overrides: a matching prohibition wins even though a permission also matches"
        );
    }

    #[test]
    fn unsatisfied_prohibition_does_not_deny() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "bob")],
            )],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision, Decision::Allow);
    }

    #[test]
    fn no_matching_permission_denies_closed_default() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "bob")],
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision,
            Decision::Deny,
            "closed default: a non-empty permissions list that never matches must deny"
        );
    }

    #[test]
    fn empty_permissions_list_is_the_open_exception() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision,
            Decision::Allow,
            "Section 4.3's named departure: no permission rules at all is treated as open"
        );
    }

    #[test]
    fn empty_permissions_still_yields_to_a_matching_prohibition() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new("read", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert_eq!(decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision, Decision::Deny);
    }

    #[test]
    fn behaviour_closed_denies_an_empty_permissions_list_instead_of_the_open_exception() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with(&["read"], DutyMode::Advise, Behaviour::Closed);
        assert_eq!(
            decide(&policy, &claims, &config, "read", ASSET, &[]).decision,
            Decision::Deny,
            "Behaviour::Closed: no permission rules at all denies, the Formal Semantics draft's \
             own closed default, not Section 4.3's Open exception"
        );
    }

    #[test]
    fn behaviour_closed_still_evaluates_a_non_empty_permissions_list_normally() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with(&["read"], DutyMode::Advise, Behaviour::Closed);
        assert_eq!(
            decide(&policy, &claims, &config, "read", ASSET, &[]).decision,
            Decision::Allow,
            "Behaviour only changes the EMPTY-list case; an actual covering, matching \
             permission still allows under Closed exactly as it does under Open"
        );
    }

    #[test]
    fn behaviour_closed_denies_past_an_unrelated_non_covering_prohibition() {
        // The exact shape of the vendored ODRL-Test-Suite regression this
        // parameter exists to let a host correct: a policy's only rule is
        // a prohibition that does not cover the requested action, leaving
        // `permissions` empty. Behaviour::Open (the engine's own default)
        // allows here; Behaviour::Closed — matching that suite's own
        // closed-world ground truth — denies, without weakening Section
        // 4.3's own Open default for hosts that still want it.
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new("use", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config_open = config_with(&["use", "sell"], DutyMode::Advise, Behaviour::Open);
        assert_eq!(
            decide(&policy, &claims, &config_open, "sell", ASSET, &[]).decision,
            Decision::Allow,
            "Open: the empty permissions list is still vacuously met even though the lone \
             prohibition (use) does not cover the request (sell)"
        );

        let config_closed = config_with(&["use", "sell"], DutyMode::Advise, Behaviour::Closed);
        assert_eq!(
            decide(&policy, &claims, &config_closed, "sell", ASSET, &[]).decision,
            Decision::Deny,
            "Closed: nothing actively permits sell, so it denies regardless of the unrelated \
             non-covering prohibition"
        );
    }

    #[test]
    fn any_one_matching_permission_is_enough_not_all() {
        let policy = Policy {
            permissions: vec![
                Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")]),
                Rule::new("read", vec![]),
            ],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision,
            Decision::Allow,
            "multiple permission rules are alternative grants, not a conjunction"
        );
    }

    #[test]
    fn any_one_matching_prohibition_is_enough() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![
                Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")]),
                Rule::new("read", vec![]),
            ],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &claims, &all_actions_config(), "read", ASSET, &[]).decision, Decision::Deny);
    }

    #[test]
    fn deserializes_from_the_documented_json_shape() {
        let json = r#"{
            "permissions": [{"action": "read", "constraints": []}],
            "prohibitions": [],
            "obligations": [{"action": "notify", "constraints": []}]
        }"#;
        let policy: Policy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.permissions.len(), 1);
        assert_eq!(policy.permissions[0].action, "read");
        assert_eq!(policy.obligations.len(), 1);
        assert_eq!(policy.obligations[0].action, "notify");
    }

    #[test]
    fn missing_permissions_and_prohibitions_fields_default_to_empty() {
        let policy: Policy = serde_json::from_str("{}").unwrap();
        assert!(policy.permissions.is_empty());
        assert!(policy.prohibitions.is_empty());
        assert!(policy.obligations.is_empty());
    }

    #[test]
    fn recognized_action_is_evaluated_normally() {
        let policy = Policy {
            permissions: vec![Rule::new("distribute", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["distribute"]);
        assert_eq!(
            decide(&policy, &claims, &config, "distribute", ASSET, &[]).decision,
            Decision::Allow,
            "an action present in the resolved config's declared actions is evaluated by \
             Section 4.3's algorithm as usual"
        );
    }

    #[test]
    fn unrecognized_action_in_a_permission_yields_error_naming_the_action_and_rule() {
        let policy = Policy {
            permissions: vec![Rule::new("anonymize", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read", "write"]);
        match decide(&policy, &claims, &config, "read", ASSET, &[]).decision {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Permission);
                assert_eq!(unrecognized.rule_index, 0);
                let message = unrecognized.to_string();
                assert!(
                    message.contains("anonymize"),
                    "message should name the unrecognized action: {message}"
                );
                assert!(
                    message.contains("permission"),
                    "message should name which rule list raised it: {message}"
                );
            }
            other => panic!("expected Decision::Error, got {other:?}"),
        }
    }

    #[test]
    fn unrecognized_action_in_a_prohibition_yields_error_and_is_not_silently_permissive() {
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new("anonymize", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read", "write"]);
        match decide(&policy, &claims, &config, "read", ASSET, &[]).decision {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Prohibition);
                assert_eq!(unrecognized.rule_index, 0);
            }
            other => panic!(
                "expected Decision::Error — a Prohibition with an unrecognized action must \
                 never be silently treated as a non-match (that would fail open), got {other:?}"
            ),
        }
    }

    #[test]
    fn unrecognized_action_is_reported_even_when_its_constraints_would_not_have_matched() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "anonymize",
                vec![Constraint::new("sub", Operator::Eq, "nobody")],
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_recognizing(&["read"]);
        assert!(
            matches!(decide(&policy, &claims, &config, "read", ASSET, &[]).decision, Decision::Error(_)),
            "the action check must not be skipped just because the rule would have missed anyway"
        );
    }

    #[test]
    fn unrecognized_action_is_reported_even_when_it_would_not_have_covered_the_request() {
        // Section 4.4's Error check runs on every rule's own declared
        // action, ahead of and independent from coverage matching — a
        // rule naming a vocabulary-unknown action is a configuration gap
        // regardless of what's actually being requested.
        let policy = Policy {
            permissions: vec![Rule::new("anonymize", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read"]);
        assert!(matches!(decide(&policy, &claims, &config, "write", ASSET, &[]).decision, Decision::Error(_)));
    }

    #[test]
    fn profile_union_recognizes_actions_from_either_loaded_profile() {
        let policy = Policy {
            permissions: vec![Rule::new("modify", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = crate::profile::resolve(&[
            Profile {
                id: "https://example.org/profiles/a".to_string(),
                actions: flat(&["read"]),
                duty_mode: DutyMode::Advise,
                behaviour: Behaviour::Open,
            },
            Profile {
                id: "https://example.org/profiles/b".to_string(),
                actions: flat(&["modify"]),
                duty_mode: DutyMode::Deny,
                behaviour: Behaviour::Open,
            },
        ]);
        assert_eq!(
            decide(&policy, &claims, &config, "modify", ASSET, &[]).decision,
            Decision::Allow,
            "an action recognized by only one of two loaded profiles is still recognized by \
             the union (Section 4.4's named fail-open choice, implemented as specified)"
        );
        assert_eq!(
            config.duty_mode,
            DutyMode::Deny,
            "the resolved config's duty_mode is the strictest across loaded profiles"
        );
    }

    #[test]
    fn a_permission_for_a_broader_action_covers_a_request_for_an_includedin_specific_one() {
        // The Section 3.5 worked example, end to end through decide(, &[]):
        // a permission for the broad "transfer" action covers a request
        // for the specific "sell" action, with no host-side pre-filtering
        // or rewriting of Rule::action needed.
        let policy = Policy {
            permissions: vec![Rule::new("transfer", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = crate::profile::resolve(&[Profile {
            id: "https://example.org/profiles/test".to_string(),
            actions: vec![ActionDecl::new("transfer"), ActionDecl::included_in("sell", "transfer")],
            duty_mode: DutyMode::Advise,
            behaviour: Behaviour::Open,
        }]);
        assert_eq!(decide(&policy, &claims, &config, "sell", ASSET, &[]).decision, Decision::Allow);
    }

    #[test]
    fn a_permission_that_does_not_cover_the_requested_action_denies_not_errors() {
        // "sell" and "give" are siblings (both includedIn "transfer");
        // a permission naming only "sell" does not cover a "give" request
        // — an ordinary closed-default Deny, not a configuration Error,
        // since "give" is still a recognized (just uncovered) action.
        let policy = Policy {
            permissions: vec![Rule::new("sell", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = crate::profile::resolve(&[Profile {
            id: "https://example.org/profiles/test".to_string(),
            actions: vec![
                ActionDecl::new("transfer"),
                ActionDecl::included_in("sell", "transfer"),
                ActionDecl::included_in("give", "transfer"),
            ],
            duty_mode: DutyMode::Advise,
            behaviour: Behaviour::Open,
        }]);
        assert_eq!(decide(&policy, &claims, &config, "give", ASSET, &[]).decision, Decision::Deny);
    }

    #[test]
    fn already_satisfied_duty_produces_no_unresolved_entry_and_does_not_affect_decision() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new(
                "notify",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Deny);
        let outcome = decide(&policy, &claims, &config, "read", ASSET, &[]);
        assert_eq!(
            outcome.decision,
            Decision::Allow,
            "a duty whose constraints all match is already satisfied, even under duty_mode: deny"
        );
        assert!(
            outcome.unresolved_duties.is_empty(),
            "a satisfied duty must not appear in the unresolved list: {:?}",
            outcome.unresolved_duties
        );
    }

    #[test]
    fn duty_mode_deny_forces_an_otherwise_allow_decision_to_deny_on_an_unresolved_duty() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("notify", vec![])],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Deny);
        let outcome = decide(&policy, &claims, &config, "read", ASSET, &[]);
        assert_eq!(
            outcome.decision,
            Decision::Deny,
            "duty_mode: deny must override an otherwise-Allow decision when a duty is unresolved"
        );
        assert_eq!(
            outcome.unresolved_duties,
            vec![UnresolvedDuty {
                action: "notify".to_string(),
                duty_index: 0,
                attachment: DutyAttachment::Obligation,
                consequence_depth: 0,
            }]
        );
    }

    #[test]
    fn duty_mode_deny_with_an_unconditional_duty_is_unresolved_not_vacuously_satisfied() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("delete-after-30-days", vec![])],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "delete-after-30-days"], DutyMode::Deny);
        let outcome = decide(&policy, &claims, &config, "read", ASSET, &[]);
        assert_eq!(
            outcome.decision,
            Decision::Deny,
            "Section 4.5: an unconstrained duty has no claims-based way to be verified, so it \
             is unresolved (not vacuously satisfied the way an unconstrained permission is)"
        );
        assert_eq!(outcome.unresolved_duties.len(), 1);
    }

    #[test]
    fn duty_mode_advise_does_not_change_the_decision_but_surfaces_the_unresolved_duty() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("notify", vec![])],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Advise);
        let outcome = decide(&policy, &claims, &config, "read", ASSET, &[]);
        assert_eq!(
            outcome.decision,
            Decision::Allow,
            "duty_mode: advise must never block an otherwise-Allow decision"
        );
        assert_eq!(
            outcome.unresolved_duties,
            vec![UnresolvedDuty {
                action: "notify".to_string(),
                duty_index: 0,
                attachment: DutyAttachment::Obligation,
                consequence_depth: 0,
            }],
            "the unresolved duty must still be surfaced for the caller to act on"
        );
    }

    #[test]
    fn duty_mode_advise_does_not_rescue_a_decision_already_denied_by_a_prohibition() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new("read", vec![])],
            obligations: vec![Rule::new("notify", vec![])],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Advise);
        let outcome = decide(&policy, &claims, &config, "read", ASSET, &[]);
        assert_eq!(outcome.decision, Decision::Deny);
        assert_eq!(outcome.unresolved_duties.len(), 1);
    }

    #[test]
    fn multiple_unresolved_duties_are_all_reported_with_their_own_indices() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![
                Rule::new("notify", vec![]),
                Rule::new(
                    "delete-after-30-days",
                    vec![Constraint::new("sub", Operator::Eq, "nobody")],
                ),
            ],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let config = config_with_duty_mode(&["read", "notify", "delete-after-30-days"], DutyMode::Advise);
        let outcome = decide(&policy, &claims, &config, "read", ASSET, &[]);
        assert_eq!(outcome.decision, Decision::Allow);
        assert_eq!(
            outcome.unresolved_duties,
            vec![
                UnresolvedDuty {
                    action: "notify".to_string(),
                    duty_index: 0,
                    attachment: DutyAttachment::Obligation,
                    consequence_depth: 0,
                },
                UnresolvedDuty {
                    action: "delete-after-30-days".to_string(),
                    duty_index: 1,
                    attachment: DutyAttachment::Obligation,
                    consequence_depth: 0,
                },
            ]
        );
    }

    #[test]
    fn a_permission_gated_by_a_native_nested_or_constraint_allows_when_either_branch_matches() {
        // Proof that `decide` itself (not just `Constraint::evaluate` in
        // isolation) treats a single `Rule::constraints` entry that is a
        // logical grouping the same way it always treated a flat one: it
        // is one constraint the rule's own `Rule::matches` -- `.all()`
        // over `constraints` -- evaluates via `Constraint::evaluate`,
        // which now recurses into the nested `odrl:or`.
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::or(vec![
                    Constraint::new("sub", Operator::Eq, "alice"),
                    Constraint::new("sub", Operator::Eq, "bob"),
                ])],
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = all_actions_config();

        let alice = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(decide(&policy, &alice, &config, "read", ASSET, &[]).decision, Decision::Allow);

        let carol = claims_with(&[("sub", ClaimValue::Single("carol".into()))]);
        assert_eq!(decide(&policy, &carol, &config, "read", ASSET, &[]).decision, Decision::Deny);
    }

    #[test]
    fn a_prohibition_gated_by_a_native_xone_constraint_denies_on_exactly_one_match_only() {
        // The genuinely new capability (Section 7 / this repo's README's
        // own "no way to express exactly one" limitation), wired all the
        // way through `decide`: a prohibition naming an `odrl:xone` over
        // two mutually-exclusive-in-intent scopes must deny when exactly
        // one holds, but not when neither or both do.
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new(
                "read",
                vec![Constraint::xone(vec![
                    Constraint::new("scope", Operator::IsAnyOf, "internal-only"),
                    Constraint::new("scope", Operator::IsAnyOf, "embargoed"),
                ])],
            )],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = all_actions_config();

        let neither = claims_with(&[("scope", ClaimValue::Single("public".into()))]);
        assert_eq!(
            decide(&policy, &neither, &config, "read", ASSET, &[]).decision,
            Decision::Allow,
            "0 of 2 xone branches matching must not trigger the prohibition"
        );

        let exactly_one = claims_with(&[("scope", ClaimValue::Single("embargoed".into()))]);
        assert_eq!(
            decide(&policy, &exactly_one, &config, "read", ASSET, &[]).decision,
            Decision::Deny,
            "exactly 1 of 2 xone branches matching must trigger the prohibition"
        );

        let both = claims_with(&[(
            "scope",
            ClaimValue::Multi(vec!["internal-only".into(), "embargoed".into()]),
        )]);
        assert_eq!(
            decide(&policy, &both, &config, "read", ASSET, &[]).decision,
            Decision::Allow,
            "2 of 2 xone branches matching must not trigger the prohibition either -- exactly \
             one, not one-or-more"
        );
    }

    // -- referenced left operands ------------------------------------------

    #[test]
    fn a_policy_with_no_constraints_references_no_left_operands() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("notify", vec![])],
            conflict: ConflictStrategy::default(),
        };
        assert!(
            policy.referenced_left_operands().is_empty(),
            "an unconstrained policy needs no claims gathered for it at all"
        );
        assert!(Policy::default().referenced_left_operands().is_empty());
    }

    #[test]
    fn a_policy_references_a_single_atomic_constraints_left_operand() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "alice")])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(policy.referenced_left_operands(), vec!["sub".to_string()]);
    }

    #[test]
    fn a_policy_unions_left_operands_across_permissions_prohibitions_and_obligations() {
        // All three rule lists count: a host that gathered only the claims
        // a permission names would silently fail-open the prohibition
        // (a missing claim key is a miss, so an unfed prohibition simply
        // never fires) and silently under-resolve the duty.
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::new("sub", Operator::Eq, "alice"), Constraint::new("scope", Operator::IsAnyOf, "read")],
            )],
            prohibitions: vec![Rule::new("read", vec![Constraint::new("embargo", Operator::Eq, "true")])],
            obligations: vec![Rule::new("notify", vec![Constraint::new("sub", Operator::Eq, "alice")])],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(
            policy.referenced_left_operands(),
            vec!["embargo".to_string(), "scope".to_string(), "sub".to_string()],
            "sorted, deduped across every rule list"
        );
    }

    #[test]
    fn a_policys_nested_logical_constraints_are_walked_to_any_depth() {
        let policy = Policy {
            permissions: vec![Rule::new(
                "read",
                vec![Constraint::or(vec![
                    Constraint::new("sub", Operator::Eq, "root"),
                    Constraint::and(vec![
                        Constraint::new("nationality", Operator::IsAnyOf, "FR,DE"),
                        Constraint::xone(vec![
                            Constraint::new("clearance", Operator::Eq, "high"),
                            Constraint::new("sub", Operator::Eq, "alice"),
                        ]),
                    ]),
                ])],
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(
            policy.referenced_left_operands(),
            vec!["clearance".to_string(), "nationality".to_string(), "sub".to_string()],
            "a walk that only read each rule's top-level constraints would report nothing at \
             all here -- every operand this policy uses lives inside a logical grouping"
        );
    }

    #[test]
    fn a_policy_set_unions_every_policys_referenced_left_operands() {
        let policies = vec![
            Policy {
                permissions: vec![Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "alice")])],
                prohibitions: vec![],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
            },
            Policy {
                permissions: vec![],
                prohibitions: vec![Rule::new(
                    "read",
                    vec![Constraint::and(vec![
                        Constraint::new("embargo", Operator::Eq, "true"),
                        Constraint::new("sub", Operator::Neq, "root"),
                    ])],
                )],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
            },
        ];
        assert_eq!(
            referenced_left_operands(&policies),
            vec!["embargo".to_string(), "sub".to_string()]
        );
        assert!(
            referenced_left_operands(&[]).is_empty(),
            "an empty policy set references nothing"
        );
    }

    // -- odrl:refinement on an action --------------------------------------

    #[test]
    fn an_unsatisfied_action_refinement_makes_a_covering_permission_inapplicable() {
        // "print, at most 2 copies" — ODRL 2.2's own worked example of a
        // refinement. The bare action string matches the request exactly;
        // the refined action does not, so the permission never applies and
        // the permission requirement goes unmet.
        let policy = Policy {
            permissions: vec![Rule::refined(
                "print",
                vec![],
                Constraint::new("copies", Operator::Lteq, "2"),
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("copies", ClaimValue::Single("5".into()))]);
        assert_eq!(
            decide(&policy, &claims, &config_recognizing(&["print"]), "print", ASSET, &[]).decision,
            Decision::Deny,
            "an action refinement the claims do not satisfy must make the rule inapplicable, \
             not be silently ignored because the bare action string matched"
        );
    }

    #[test]
    fn a_satisfied_action_refinement_leaves_the_permission_applying_normally() {
        let policy = Policy {
            permissions: vec![Rule::refined(
                "print",
                vec![],
                Constraint::new("copies", Operator::Lteq, "2"),
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("copies", ClaimValue::Single("2".into()))]);
        assert_eq!(
            decide(&policy, &claims, &config_recognizing(&["print"]), "print", ASSET, &[]).decision,
            Decision::Allow
        );
    }

    #[test]
    fn a_rule_built_without_a_refinement_carries_none_and_decides_exactly_as_before() {
        // The backward-compatibility case at the Rust-value level (the
        // wire-level one is `wire.rs`'s own
        // `an_existing_fixture_rule_without_a_refinement_key_round_trips_unchanged`):
        // `Rule::new` is the constructor every existing call site in this
        // workspace uses, and it must keep producing an unrefined rule.
        let rule = Rule::new("print", vec![Constraint::new("sub", Operator::Eq, "alice")]);
        assert_eq!(rule.action_refinement, None);
        let policy = Policy {
            permissions: vec![rule],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert_eq!(
            decide(&policy, &claims, &config_recognizing(&["print"]), "print", ASSET, &[]).decision,
            Decision::Allow
        );
    }

    #[test]
    fn an_action_refinement_is_a_separate_condition_from_the_rules_own_constraints() {
        // Both must hold: the refinement narrows *which action* the rule is
        // about, the constraints say *under what circumstances* the rule
        // applies. Neither substitutes for the other, so each failing alone
        // denies.
        let refined = || {
            Rule::refined(
                "print",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
                Constraint::new("copies", Operator::Lteq, "2"),
            )
        };
        let policy = Policy {
            permissions: vec![refined()],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["print"]);

        let constraints_only = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("copies", ClaimValue::Single("5".into())),
        ]);
        assert_eq!(
            decide(&policy, &constraints_only, &config, "print", ASSET, &[]).decision,
            Decision::Deny,
            "rule constraints satisfied but the action refinement is not"
        );

        let refinement_only = claims_with(&[
            ("sub", ClaimValue::Single("bob".into())),
            ("copies", ClaimValue::Single("1".into())),
        ]);
        assert_eq!(
            decide(&policy, &refinement_only, &config, "print", ASSET, &[]).decision,
            Decision::Deny,
            "action refinement satisfied but the rule's own constraints are not"
        );

        let both = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("copies", ClaimValue::Single("1".into())),
        ]);
        assert_eq!(decide(&policy, &both, &config, "print", ASSET, &[]).decision, Decision::Allow);
    }

    #[test]
    fn an_unsatisfied_action_refinement_stops_a_prohibition_from_denying() {
        // The fail-open direction, and the reason a refinement cannot be
        // treated as decoration a host may drop: a prohibition on `print,
        // more than 2 copies` must not deny a request that prints one —
        // and must still deny one that prints three.
        let policy = Policy {
            permissions: vec![Rule::new("print", vec![])],
            prohibitions: vec![Rule::refined(
                "print",
                vec![],
                Constraint::new("copies", Operator::Gt, "2"),
            )],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["print"]);

        let within = claims_with(&[("copies", ClaimValue::Single("1".into()))]);
        assert_eq!(
            decide(&policy, &within, &config, "print", ASSET, &[]).decision,
            Decision::Allow,
            "the prohibition is about a refined action this request is not performing"
        );

        let beyond = claims_with(&[("copies", ClaimValue::Single("3".into()))]);
        assert_eq!(
            decide(&policy, &beyond, &config, "print", ASSET, &[]).decision,
            Decision::Deny,
            "the same prohibition, now applicable to the refined action, denies"
        );
    }

    #[test]
    fn an_action_refinement_may_itself_be_a_logical_group() {
        // `Constraint` is reused verbatim for the refinement, so an
        // `odrl:and`/`odrl:or`/`odrl:xone` group is a refinement too — the
        // ODRL shape for an action narrowed on more than one axis at once.
        let policy = Policy {
            permissions: vec![Rule::refined(
                "print",
                vec![],
                Constraint::and(vec![
                    Constraint::new("copies", Operator::Lteq, "2"),
                    Constraint::new("resolution", Operator::Eq, "draft"),
                ]),
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["print"]);

        let both = claims_with(&[
            ("copies", ClaimValue::Single("1".into())),
            ("resolution", ClaimValue::Single("draft".into())),
        ]);
        assert_eq!(decide(&policy, &both, &config, "print", ASSET, &[]).decision, Decision::Allow);

        let one = claims_with(&[
            ("copies", ClaimValue::Single("1".into())),
            ("resolution", ClaimValue::Single("high".into())),
        ]);
        assert_eq!(
            decide(&policy, &one, &config, "print", ASSET, &[]).decision,
            Decision::Deny,
            "an `odrl:and` refinement narrows on every axis at once, not any one of them"
        );
    }

    #[test]
    fn a_duty_whose_action_refinement_is_unsatisfied_stays_unresolved() {
        // A duty's action is what must be *done*; refining it narrows what
        // would count as having done it. This engine can only ever confirm
        // a duty from claims, so a refinement it cannot confirm leaves the
        // duty unresolved — the safe direction, and the same direction an
        // unsatisfied duty constraint already goes.
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::refined(
                "notify",
                vec![Constraint::new("notified", Operator::Eq, "true")],
                Constraint::new("notify_channel", Operator::Eq, "email"),
            )],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["read", "notify"]);

        let wrong_channel = claims_with(&[
            ("notified", ClaimValue::Single("true".into())),
            ("notify_channel", ClaimValue::Single("sms".into())),
        ]);
        let outcome = decide(&policy, &wrong_channel, &config, "read", ASSET, &[]);
        assert_eq!(outcome.unresolved_duties.len(), 1);
        assert_eq!(outcome.unresolved_duties[0].action, "notify");

        let right_channel = claims_with(&[
            ("notified", ClaimValue::Single("true".into())),
            ("notify_channel", ClaimValue::Single("email".into())),
        ]);
        assert!(
            decide(&policy, &right_channel, &config, "read", ASSET, &[]).unresolved_duties.is_empty(),
            "the duty's constraints and its action refinement both hold: resolved"
        );
    }

    #[test]
    fn an_action_refinement_contributes_its_left_operands_to_the_claims_a_host_must_gather() {
        // A host that gathered only the constraints' claim keys would feed
        // the refinement nothing — and an unfed refinement on a prohibition
        // silently never applies, which is fail-open exactly where
        // `referenced_left_operands`' own doc comment says that direction
        // of mistake matters most.
        let policy = Policy {
            permissions: vec![Rule::refined(
                "print",
                vec![Constraint::new("sub", Operator::Eq, "alice")],
                Constraint::and(vec![
                    Constraint::new("copies", Operator::Lteq, "2"),
                    Constraint::new("resolution", Operator::Eq, "draft"),
                ]),
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(
            policy.referenced_left_operands(),
            vec!["copies".to_string(), "resolution".to_string(), "sub".to_string()],
            "an action refinement's own claim keys are read by this engine, so they belong in \
             the set a host is told to gather — nested ones included"
        );
    }

    #[test]
    fn unrecognized_action_in_an_obligation_yields_error() {
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("anonymize", vec![])],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["read"]);
        match decide(&policy, &claims, &config, "read", ASSET, &[]).decision {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Duty);
                assert_eq!(unrecognized.rule_index, 0);
            }
            other => panic!(
                "an obligation naming an action outside the declared vocabulary must yield Error \
                 exactly as an unrecognized permission/prohibition action would, got {other:?}"
            ),
        }
    }

    // -- per-rule odrl:target ----------------------------------------------

    const ASSET_A: &str = "urn:asset:A";
    const ASSET_B: &str = "urn:asset:B";

    #[test]
    fn a_permission_targeting_another_asset_does_not_cover_a_request_for_this_one() {
        // The bare action matches exactly; only the asset differs. Before
        // per-rule targets existed this permission covered every request,
        // because the only asset handle anywhere was the request's own.
        let policy = Policy {
            permissions: vec![Rule::targeting("read", ASSET_B, vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET_A, &[]).decision,
            Decision::Deny,
            "a permission on asset B must not permit a request for asset A"
        );
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET_B, &[]).decision,
            Decision::Allow,
            "the control: the same permission does permit the asset it is actually about"
        );
    }

    #[test]
    fn a_prohibition_targeting_another_asset_does_not_deny_this_one() {
        // The fail-open direction, and why a dropped target is a real
        // defect rather than a missing nicety: "permission on asset A,
        // prohibition on asset B" is one ordinary ODRL policy, and reading
        // it without targets denies A as well.
        let policy = Policy {
            permissions: vec![Rule::targeting("read", ASSET_A, vec![])],
            prohibitions: vec![Rule::targeting("read", ASSET_B, vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET_A, &[]).decision,
            Decision::Allow,
            "the prohibition is about asset B and has nothing to say about a request for A"
        );
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET_B, &[]).decision,
            Decision::Deny,
            "the same policy denies asset B, where the prohibition applies and the permission \
             does not"
        );
    }

    #[test]
    fn a_rule_with_no_target_is_about_whatever_the_request_is_about() {
        // The backward-compatibility case at the Rust-value level: every
        // rule in every fixture of this workspace is untargeted, and must
        // keep applying to whatever target the decision is taken for.
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        for target in [ASSET_A, ASSET_B, "", "anything at all"] {
            assert_eq!(
                decide(&policy, &claims, &all_actions_config(), "read", target, &[]).decision,
                Decision::Allow,
                "untargeted rule, target {target:?}"
            );
        }
        assert_eq!(Rule::new("read", vec![]).target, None);
    }

    #[test]
    fn a_target_and_an_action_are_separate_requirements_neither_substituting_for_the_other() {
        // Both halves of `Rule::applies`, each failing alone.
        let policy = Policy {
            permissions: vec![Rule::targeting("read", ASSET_A, vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = all_actions_config();
        assert_eq!(
            decide(&policy, &claims, &config, "read", ASSET_A, &[]).decision,
            Decision::Allow,
            "right action, right asset"
        );
        assert_eq!(
            decide(&policy, &claims, &config, "write", ASSET_A, &[]).decision,
            Decision::Deny,
            "right asset, wrong action"
        );
        assert_eq!(
            decide(&policy, &claims, &config, "read", ASSET_B, &[]).decision,
            Decision::Deny,
            "right action, wrong asset"
        );
    }

    #[test]
    fn a_targeted_rule_is_matched_by_exact_string_not_by_any_asset_taxonomy() {
        // Asserted rather than merely documented, because this limitation
        // is easy to mistake for a bug: there is no IRI normalization, and
        // (with `asset_collections` empty, as every one of these calls
        // leaves it) no collection membership either — "the same asset"
        // means "the same characters". See the `odrl:partOf` tests below
        // for the one case this is deliberately *not* true of anymore.
        let policy = Policy {
            permissions: vec![Rule::targeting("read", "urn:asset:A", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = all_actions_config();
        for near_miss in ["urn:asset:a", "urn:asset:A ", "asset:A", "urn:asset:A#frag"] {
            assert_eq!(
                decide(&policy, &claims, &config, "read", near_miss, &[]).decision,
                Decision::Deny,
                "target {near_miss:?} is not the same string as the rule's own"
            );
        }
    }

    #[test]
    fn an_unrecognized_target_is_an_ordinary_non_match_not_a_configuration_error() {
        // Unlike an action outside the declared vocabulary (Section 4.4's
        // `Decision::Error`), a target no one declared cannot be a
        // configuration gap: nothing declares an asset vocabulary anywhere,
        // so a rule about an asset this request is not about is simply a
        // rule that does not apply.
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::targeting("read", "urn:asset:never-heard-of", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert_eq!(
            decide(&policy, &claims, &all_actions_config(), "read", ASSET_A, &[]).decision,
            Decision::Allow
        );
    }

    // -- odrl:AssetCollection membership (odrl:partOf) ----------------------

    const COLLECTION_X: &str = "urn:asset:collection-X";
    const MEMBER_1: &str = "urn:asset:member-1";

    #[test]
    fn target_applies_is_satisfied_by_a_collection_the_requested_target_is_asserted_to_be_part_of() {
        // `Rule::target_applies` directly, the narrowest unit here: a rule
        // scoped to a collection IRI is about a request for one of that
        // collection's declared members too, once the host asserts the
        // membership via `asset_collections` — not only about a request
        // naming the collection IRI itself.
        let rule = Rule::targeting("use", COLLECTION_X, vec![]);
        assert!(
            rule.target_applies(MEMBER_1, &[COLLECTION_X.to_string()]),
            "the host asserts member-1 is part of collection-X, so a rule targeting the \
             collection is in play for member-1 too"
        );
        assert!(
            !rule.target_applies(MEMBER_1, &[]),
            "control: with no asserted membership, member-1 is just a different string from \
             collection-X and the rule does not apply"
        );
        assert!(
            !rule.target_applies(MEMBER_1, &["urn:asset:some-other-collection".to_string()]),
            "membership in an unrelated collection does not make member-1 the same asset as \
             collection-X"
        );
        assert!(
            rule.target_applies(COLLECTION_X, &[]),
            "the exact-match reading this field always had is unaffected by this addition"
        );
    }

    #[test]
    fn a_prohibition_on_a_collection_denies_a_request_for_an_asserted_member() {
        // The exact backlog example: a prohibition scoped to a collection
        // IRI, and a request for one of that collection's members, with the
        // host asserting the membership out of band via `asset_collections`
        // — the fail-open gap this addition closes. Before this field
        // existed, `target_applies` did bare string equality and this
        // request slipped past the prohibition entirely.
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::targeting("use", COLLECTION_X, vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_recognizing(&["use"]);

        // Request A: the collection IRI itself — already correctly denied
        // before this addition, by plain string equality.
        assert_eq!(
            decide(&policy, &claims, &config, "use", COLLECTION_X, &[]).decision,
            Decision::Deny,
            "a request for the collection IRI itself was already denied by exact match"
        );

        // Request B: a member of the collection, with the membership
        // asserted. This is the case that used to fail open.
        assert_eq!(
            decide(&policy, &claims, &config, "use", MEMBER_1, &[COLLECTION_X.to_string()]).decision,
            Decision::Deny,
            "a member of a prohibited collection must be denied once the host asserts the \
             membership, not silently allowed to evade the prohibition"
        );

        // Control: the same member request, with no membership asserted —
        // this is the prior (and still correct, absent the fact) behaviour.
        assert_eq!(
            decide(&policy, &claims, &config, "use", MEMBER_1, &[]).decision,
            Decision::Allow,
            "control: with no asserted collection membership, member-1 is simply not the asset \
             the prohibition names"
        );
    }

    #[test]
    fn a_duty_is_not_scoped_by_the_requested_target() {
        // A duty's action is what must be *done* and its target is the
        // asset to do it to (write this audit log, delete that copy) —
        // neither is what the request asks for, so scoping duties by the
        // requested target would silently drop obligations a policy really
        // does attach. The same decision `requested_action` already made
        // for a duty's action.
        let policy = Policy {
            permissions: vec![Rule::targeting("read", ASSET_A, vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::targeting(
                "notify",
                "urn:asset:audit-log",
                vec![Constraint::new("notified", Operator::Eq, "true")],
            )],
            conflict: ConflictStrategy::default(),
        };
        let config = config_with_duty_mode(&["read", "notify"], DutyMode::Deny);

        let unnotified = claims_with(&[]);
        let outcome = decide(&policy, &unnotified, &config, "read", ASSET_A, &[]);
        assert_eq!(
            outcome.decision,
            Decision::Deny,
            "the duty is about another asset, but it is still this policy's duty and still \
             unresolved under duty_mode: deny"
        );
        assert_eq!(outcome.unresolved_duties.len(), 1);

        let notified = claims_with(&[("notified", ClaimValue::Single("true".into()))]);
        assert!(
            decide(&policy, &notified, &config, "read", ASSET_A, &[]).unresolved_duties.is_empty(),
            "and it resolves from claims exactly as an untargeted duty would"
        );
    }

    #[test]
    fn performable_actions_answers_per_asset() {
        // Once rules can be scoped to assets, "what may I do" is only
        // answerable about one asset at a time: the same policy and the
        // same claims give different answers for two different datasets.
        let policy = Policy {
            permissions: vec![
                Rule::targeting("read", ASSET_A, vec![]),
                Rule::targeting("write", ASSET_B, vec![]),
            ],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        let config = config_with(&["read", "write"], DutyMode::Advise, Behaviour::Closed);
        assert_eq!(performable_actions(&policy, &claims, &config, ASSET_A, &[]), vec!["read".to_string()]);
        assert_eq!(performable_actions(&policy, &claims, &config, ASSET_B, &[]), vec!["write".to_string()]);
        assert!(performable_actions(&policy, &claims, &config, "urn:asset:C", &[]).is_empty());
    }

    #[test]
    fn a_targeted_rule_contributes_its_left_operands_exactly_as_an_untargeted_one_does() {
        // A target narrows *which* decisions a rule participates in, never
        // which claims it reads — so the reachability answer a host uses to
        // decide what to gather must not shrink because a rule named an
        // asset. Reporting per-target would be wrong in the fail-open
        // direction for any host that asks once and reuses the answer.
        let policy = Policy {
            permissions: vec![Rule::targeting(
                "read",
                ASSET_B,
                vec![Constraint::new("sub", Operator::Eq, "alice")],
            )],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(policy.referenced_left_operands(), vec!["sub".to_string()]);
    }

    // -- performable_actions: "what may I do?" ------------------------------

    fn taxonomy_config(behaviour: Behaviour, duty_mode: DutyMode) -> ResolvedConfig {
        crate::profile::resolve(&[Profile {
            id: "https://example.org/profiles/taxonomy".to_string(),
            actions: vec![
                ActionDecl::new("use"),
                ActionDecl::included_in("read", "use"),
                ActionDecl::included_in("write", "use"),
                ActionDecl::new("print"),
                ActionDecl::new("notify"),
            ],
            duty_mode,
            behaviour,
        }])
    }

    /// A permission for the broad `use`, a prohibition carving `write` back
    /// out of it, and a second permission for `print` whose constraint the
    /// claims miss.
    fn mixed_policy() -> Policy {
        Policy {
            permissions: vec![
                Rule::new("use", vec![Constraint::new("nationality", Operator::Eq, "DE")]),
                Rule::new("print", vec![Constraint::new("sub", Operator::Eq, "bob")]),
            ],
            prohibitions: vec![Rule::new("write", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        }
    }

    fn de_alice() -> Claims {
        claims_with(&[
            ("nationality", ClaimValue::Single("DE".into())),
            ("sub", ClaimValue::Single("alice".into())),
        ])
    }

    #[test]
    fn performable_actions_returns_exactly_the_declared_actions_that_allow() {
        let config = taxonomy_config(Behaviour::Closed, DutyMode::Advise);
        assert_eq!(
            performable_actions(&mixed_policy(), &de_alice(), &config, ASSET, &[]),
            vec!["read".to_string(), "use".to_string()],
            "`use` matches outright; `read` inherits it through a declared includedIn edge; \
             `write` inherits it too but a prohibition carves it back out; `print`'s own \
             permission misses on its constraint; nothing covers `notify` at all"
        );
    }

    #[test]
    fn performable_actions_agrees_with_decide_on_every_declared_action() {
        // The consistency property, not a hand-picked example: this is a
        // thin wrapper over `decide`, so for every action the resolved
        // config declares, membership in the returned list must be exactly
        // `decide(..., action, &[]) == Allow` — on each of a handful of
        // deliberately different fixtures.
        let fixtures: Vec<(&str, Policy, Claims, ResolvedConfig)> = vec![
            (
                "mixed permissions/prohibition, closed",
                mixed_policy(),
                de_alice(),
                taxonomy_config(Behaviour::Closed, DutyMode::Advise),
            ),
            (
                "mixed permissions/prohibition, open",
                mixed_policy(),
                de_alice(),
                taxonomy_config(Behaviour::Open, DutyMode::Advise),
            ),
            (
                "claims that satisfy nothing",
                mixed_policy(),
                claims_with(&[]),
                taxonomy_config(Behaviour::Closed, DutyMode::Advise),
            ),
            (
                "empty policy under the open default",
                Policy::default(),
                claims_with(&[]),
                taxonomy_config(Behaviour::Open, DutyMode::Advise),
            ),
            (
                "empty policy under closed",
                Policy::default(),
                claims_with(&[]),
                taxonomy_config(Behaviour::Closed, DutyMode::Advise),
            ),
            (
                "an unresolved duty under duty_mode deny",
                Policy {
                    permissions: vec![Rule::new("use", vec![])],
                    prohibitions: vec![],
                    obligations: vec![Rule::new("notify", vec![])],
                    conflict: ConflictStrategy::default(),
                },
                claims_with(&[]),
                taxonomy_config(Behaviour::Closed, DutyMode::Deny),
            ),
            (
                "a rule naming an action outside the declared vocabulary",
                Policy {
                    permissions: vec![Rule::new("anonymize", vec![])],
                    prohibitions: vec![],
                    obligations: vec![],
                    conflict: ConflictStrategy::default(),
                },
                claims_with(&[]),
                taxonomy_config(Behaviour::Closed, DutyMode::Advise),
            ),
            (
                "an action refinement narrowing what the permission is about",
                Policy {
                    permissions: vec![Rule::refined(
                        "print",
                        vec![],
                        Constraint::new("copies", Operator::Lteq, "2"),
                    )],
                    prohibitions: vec![],
                    obligations: vec![],
                    conflict: ConflictStrategy::default(),
                },
                claims_with(&[("copies", ClaimValue::Single("5".into()))]),
                taxonomy_config(Behaviour::Closed, DutyMode::Advise),
            ),
        ];

        for (label, policy, claims, config) in &fixtures {
            let performable = performable_actions(policy, claims, config, ASSET, &[]);
            for action in config.declared_actions() {
                let allowed = decide(policy, claims, config, action, ASSET, &[]).decision == Decision::Allow;
                assert_eq!(
                    performable.iter().any(|a| a == action),
                    allowed,
                    "fixture {label:?}: performable_actions and decide disagree about {action:?} \
                     (performable list was {performable:?})"
                );
            }
        }
    }

    #[test]
    fn performable_actions_is_empty_when_any_rule_names_an_unrecognized_action() {
        // `decide` answers Error, not Deny, for every action in that case —
        // a configuration gap a caller must treat as fail-closed. An
        // enumeration that quietly reported the other actions as performable
        // would launder that Error into a partial allow-list.
        let config = taxonomy_config(Behaviour::Open, DutyMode::Advise);
        let claims = claims_with(&[]);
        let policy = Policy {
            permissions: vec![Rule::new("use", vec![]), Rule::new("anonymize", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        assert!(performable_actions(&policy, &claims, &config, ASSET, &[]).is_empty());

        // The control this negative needs to mean anything: the same
        // policy with the offending rule dropped is performable for
        // plenty. Without it, an implementation that returned nothing at
        // all would sail through the assertion above.
        let control = Policy {
            permissions: vec![Rule::new("use", vec![])],
            ..policy
        };
        assert_eq!(
            performable_actions(&control, &claims, &config, ASSET, &[]),
            vec!["read".to_string(), "use".to_string(), "write".to_string()]
        );
    }

    #[test]
    fn an_empty_permissions_policy_is_performable_for_everything_under_open_and_nothing_under_closed() {
        let policy = Policy::default();
        let claims = claims_with(&[]);
        assert_eq!(
            performable_actions(&policy, &claims, &taxonomy_config(Behaviour::Open, DutyMode::Advise), ASSET, &[]),
            vec![
                "notify".to_string(),
                "print".to_string(),
                "read".to_string(),
                "use".to_string(),
                "write".to_string()
            ],
            "Section 4.3's open default makes an empty permissions list vacuously met for every \
             declared action — the honest answer, and the reason this call takes a config"
        );
        assert!(
            performable_actions(&policy, &claims, &taxonomy_config(Behaviour::Closed, DutyMode::Advise), ASSET, &[]).is_empty(),
            "under closed the same policy permits nothing at all"
        );
    }

    #[test]
    fn performable_actions_is_sorted_and_deduplicated_even_when_the_config_declares_a_duplicate() {
        let config = ResolvedConfig::new(
            vec![ActionDecl::new("write"), ActionDecl::new("read"), ActionDecl::new("read")],
            DutyMode::Advise,
            Behaviour::Open,
        );
        assert_eq!(
            performable_actions(&Policy::default(), &claims_with(&[]), &config, ASSET, &[]),
            vec!["read".to_string(), "write".to_string()]
        );
    }

    // -- nested duties: odrl:duty, odrl:consequence, odrl:remedy -----------

    /// A duty resolved the way every duty here is: by an ordinary
    /// claims-map lookup a host satisfies with a claim asserting the duty
    /// fulfilled. There is deliberately no second mechanism.
    fn asserted_duty(action: &str) -> Rule {
        Rule::new(action, vec![Constraint::new(format!("duty:{action}"), Operator::Eq, "fulfilled")])
    }

    fn fulfilled(actions: &[&str]) -> Claims {
        actions
            .iter()
            .map(|a| (format!("duty:{a}"), ClaimValue::Single("fulfilled".into())))
            .collect()
    }

    #[test]
    fn a_permission_duty_naming_an_action_outside_the_vocabulary_is_a_configuration_error() {
        // Section 4.4's own posture, at the new attachment point: a
        // policy-level obligation naming an unknown action is already an
        // Error, and an identical duty attached to a permission is the same
        // configuration gap. Leaving it unchecked would make the outcome
        // depend on where the policy author put the duty.
        let policy = Policy {
            permissions: vec![Rule {
                duty: vec![asserted_duty("anonymize")],
                ..Rule::new("read", vec![])
            }],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        match decide(&policy, &claims, &config_recognizing(&["read"]), "read", ASSET, &[]).decision {
            Decision::Error(unrecognized) => {
                assert_eq!(unrecognized.action, "anonymize");
                assert_eq!(unrecognized.rule_kind, RuleKind::Duty);
                assert_eq!(unrecognized.duty_path.as_deref(), Some("permission[0].duty[0]"));
                assert!(
                    unrecognized.to_string().contains("permission[0].duty[0]"),
                    "the message must say which rule's duty list it indexes: {unrecognized}"
                );
            }
            other => panic!("expected Decision::Error, got {other:?}"),
        }
    }

    #[test]
    fn a_consequence_or_remedy_naming_an_unknown_action_is_an_error_too() {
        let claims = claims_with(&[]);
        let with_consequence = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::with_consequence(
                "notify",
                vec![Constraint::new("duty:notify", Operator::Eq, "fulfilled")],
                asserted_duty("anonymize"),
            )],
            conflict: ConflictStrategy::default(),
        };
        match decide(&with_consequence, &claims, &config_recognizing(&["read", "notify"]), "read", ASSET, &[]).decision {
            Decision::Error(u) => assert_eq!(u.duty_path.as_deref(), Some("duty[0].consequence")),
            other => panic!("expected Decision::Error for an unknown consequence action, got {other:?}"),
        }

        let with_remedy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule {
                remedy: vec![asserted_duty("anonymize")],
                ..Rule::new("read", vec![])
            }],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        match decide(&with_remedy, &claims, &config_recognizing(&["read"]), "read", ASSET, &[]).decision {
            Decision::Error(u) => assert_eq!(u.duty_path.as_deref(), Some("prohibition[0].remedy[0]")),
            other => panic!("expected Decision::Error for an unknown remedy action, got {other:?}"),
        }
    }

    #[test]
    fn an_ordinary_policy_still_reports_the_unrecognized_action_message_it_always_did() {
        // The nested pass runs after all three original loops precisely so
        // that no policy without a nested duty can change its answer, this
        // message text included — it reaches a caller inside `reason`.
        let policy = Policy {
            permissions: vec![Rule::new("anonymize", vec![])],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        match decide(&policy, &claims_with(&[]), &config_recognizing(&["read"]), "read", ASSET, &[]).decision {
            Decision::Error(u) => {
                assert_eq!(u.duty_path, None);
                assert_eq!(
                    u.to_string(),
                    "unrecognized action \"anonymize\" in permission rule at index 0: no loaded \
                     profile's recognized_actions includes it"
                );
            }
            other => panic!("expected Decision::Error, got {other:?}"),
        }
    }

    #[test]
    fn nested_duties_contribute_their_left_operands_to_the_claims_a_host_must_gather() {
        // The same fail-open reasoning `referenced_left_operands` already
        // applies to an action refinement: a host told to gather less than
        // the engine reads leaves the duty unfeedable, and a duty that can
        // never resolve is a permission that can never grant under
        // duty_mode: deny — or, for a remedy, an obligation the host is
        // never told it has.
        let policy = Policy {
            permissions: vec![Rule {
                duty: vec![Rule::with_consequence(
                    "notify",
                    vec![Constraint::new("duty:notify", Operator::Eq, "fulfilled")],
                    asserted_duty("compensate"),
                )],
                ..Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "alice")])
            }],
            prohibitions: vec![Rule {
                remedy: vec![asserted_duty("anonymize")],
                ..Rule::new("read", vec![])
            }],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(
            policy.referenced_left_operands(),
            vec![
                "duty:anonymize".to_string(),
                "duty:compensate".to_string(),
                "duty:notify".to_string(),
                "sub".to_string(),
            ],
            "a permission's duty, that duty's consequence, and a prohibition's remedy are all \
             read by this engine, so all three belong in the set a host is told to gather"
        );
    }

    #[test]
    fn the_left_operand_walk_stops_at_the_same_consequence_bound_evaluation_stops_at() {
        // A duty past the bound is never evaluated, so naming its claim key
        // would send a host to gather a claim that provably cannot change
        // any decision — the rule `Constraint::collect_left_operands`
        // already applies to its own depth bound.
        let mut duty = asserted_duty("past-the-bound");
        for _ in 0..=MAX_CONSEQUENCE_DEPTH {
            duty = Rule::with_consequence(
                "notify",
                vec![Constraint::new("duty:notify", Operator::Eq, "fulfilled")],
                duty,
            );
        }
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![],
            obligations: vec![duty],
            conflict: ConflictStrategy::default(),
        };
        assert_eq!(
            policy.referenced_left_operands(),
            vec!["duty:notify".to_string()],
            "the deepest link's own claim key sits past MAX_CONSEQUENCE_DEPTH and is not reported"
        );
    }

    #[test]
    fn a_per_permission_duty_gates_only_its_own_permission_under_duty_mode_deny() {
        // The decision-layer statement of the scoping rule, beside
        // `wire.rs`'s wire-level one: unlike a policy-level obligation,
        // which denies the request outright, an unresolved per-permission
        // duty removes exactly one permission from consideration.
        let policy = Policy {
            permissions: vec![
                Rule {
                    duty: vec![asserted_duty("compensate")],
                    ..Rule::new("read", vec![])
                },
                Rule::new("read", vec![]),
            ],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_with(&["read", "compensate"], DutyMode::Deny, Behaviour::Closed);

        let outcome = decide(&policy, &claims_with(&[]), &config, "read", ASSET, &[]);
        assert_eq!(
            outcome.decision,
            Decision::Allow,
            "permission[1] grants on its own; permission[0]'s outstanding duty is not the policy's"
        );
        assert_eq!(outcome.unresolved_duties.len(), 1);
        assert_eq!(outcome.unresolved_duties[0].path(), "permission[0].duty[0]");

        // With permission[1] gone, that same outstanding duty is the whole
        // reason nothing grants — and the control shows it really is the duty.
        let alone = Policy {
            permissions: vec![policy.permissions[0].clone()],
            ..policy
        };
        assert_eq!(decide(&alone, &claims_with(&[]), &config, "read", ASSET, &[]).decision, Decision::Deny);
        assert_eq!(
            decide(&alone, &fulfilled(&["compensate"]), &config, "read", ASSET, &[]).decision,
            Decision::Allow
        );
    }

    #[test]
    fn a_remedy_never_lifts_a_prohibition_in_either_direction() {
        // The documented sub-decision, asserted at the decision layer: a
        // remedy is reported, never enforced away. Both resolutions give
        // the same Deny; only the duty list differs.
        let policy = Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule {
                remedy: vec![asserted_duty("anonymize")],
                ..Rule::new("read", vec![])
            }],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["read", "anonymize"]);

        let violated = decide(&policy, &claims_with(&[]), &config, "read", ASSET, &[]);
        assert_eq!(violated.decision, Decision::Deny);
        assert_eq!(violated.unresolved_duties.len(), 1);
        assert_eq!(violated.unresolved_duties[0].path(), "prohibition[0].remedy[0]");

        let remedied = decide(&policy, &fulfilled(&["anonymize"]), &config, "read", ASSET, &[]);
        assert_eq!(
            remedied.decision,
            Decision::Deny,
            "a satisfied remedy is still a Deny: duties in this engine only ever tighten a \
             decision, and a claims-asserted remedy able to erase a prohibition would be the \
             first one that loosens one"
        );
        assert!(remedied.unresolved_duties.is_empty());
    }

    #[test]
    fn performable_actions_inherits_the_per_permission_duty_gating_rather_than_re_deciding_it() {
        let policy = Policy {
            permissions: vec![
                Rule {
                    duty: vec![asserted_duty("compensate")],
                    ..Rule::new("read", vec![])
                },
                Rule::new("write", vec![]),
            ],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_with(&["read", "write", "compensate"], DutyMode::Deny, Behaviour::Closed);
        assert_eq!(
            performable_actions(&policy, &claims_with(&[]), &config, ASSET, &[]),
            vec!["write".to_string()],
            "`read` is gated by its own permission's outstanding duty; `write` is not"
        );
        assert_eq!(
            performable_actions(&policy, &fulfilled(&["compensate"]), &config, ASSET, &[]),
            vec!["read".to_string(), "write".to_string()]
        );
    }

    #[test]
    fn an_unresolved_duty_under_duty_mode_deny_makes_nothing_performable() {
        let policy = Policy {
            permissions: vec![Rule::new("use", vec![])],
            prohibitions: vec![],
            obligations: vec![Rule::new("notify", vec![])],
            conflict: ConflictStrategy::default(),
        };
        let claims = claims_with(&[]);
        assert!(
            performable_actions(&policy, &claims, &taxonomy_config(Behaviour::Open, DutyMode::Deny), ASSET, &[]).is_empty(),
            "an unconditional duty this engine cannot confirm denies every action under \
             duty_mode: deny, and this enumeration inherits that from `decide` rather than \
             re-deciding it"
        );
        // The control: the identical policy under `advise` is performable
        // for everything the `use` permission covers, unresolved duty and
        // all -- so the empty list above really is duty_mode's doing.
        assert_eq!(
            performable_actions(&policy, &claims, &taxonomy_config(Behaviour::Open, DutyMode::Advise), ASSET, &[]),
            vec!["read".to_string(), "use".to_string(), "write".to_string()]
        );
    }

    // -- odrl:conflict -----------------------------------------------------

    const EVERY_STRATEGY: [ConflictStrategy; 3] =
        [ConflictStrategy::Perm, ConflictStrategy::Prohibit, ConflictStrategy::Invalid];

    /// The one shape `odrl:conflict` is about: a permission and a
    /// prohibition that both cover and match the *same* requested action on
    /// the *same* requested target.
    fn colliding_policy(conflict: ConflictStrategy) -> Policy {
        Policy {
            permissions: vec![Rule::new("read", vec![])],
            prohibitions: vec![Rule::new("read", vec![])],
            obligations: vec![],
            conflict,
        }
    }

    #[test]
    fn a_policy_that_declares_no_conflict_strategy_defaults_to_invalid() {
        // The deliberate departure from this engine's own prior implicit
        // behaviour, stated as its own assertion rather than left to be
        // inferred from a decision that happens to be `Deny` either way.
        // ODRL's stated default for a policy carrying no conflict term is
        // `invalid`; the engine's was an unconditional, unnamed `prohibit`.
        assert_eq!(Policy::default().conflict, ConflictStrategy::Invalid);
        assert_eq!(
            serde_json::from_str::<Policy>(r#"{"permissions": [], "prohibitions": []}"#).unwrap().conflict,
            ConflictStrategy::Invalid,
            "a policy document from before this field existed reads as ODRL's own default"
        );
        assert_eq!(
            serde_json::from_str::<Policy>(r#"{"odrl:conflict": "perm"}"#).unwrap().conflict,
            ConflictStrategy::Perm,
            "the control: the key is really read, so the assertion above is about a default \
             and not about an ignored key"
        );
    }

    #[test]
    fn a_policy_carrying_the_default_conflict_strategy_serializes_exactly_as_it_did_before() {
        // Wire-additive on the convention `odrl:target` and
        // `odrl:refinement` already set: the key is emitted only by a
        // policy that means something other than the default, so every
        // document this engine produced before the field existed is
        // byte-for-byte what it was.
        assert_eq!(
            serde_json::to_string(&Policy::default()).unwrap(),
            r#"{"permissions":[],"prohibitions":[],"obligations":[]}"#
        );
        assert_eq!(
            serde_json::to_string(&colliding_policy(ConflictStrategy::Perm)).unwrap(),
            r#"{"permissions":[{"action":"read","constraints":[]}],"prohibitions":[{"action":"read","constraints":[]}],"obligations":[],"odrl:conflict":"perm"}"#
        );
    }

    #[test]
    fn conflict_perm_lets_a_matching_permission_beat_a_matching_prohibition() {
        // The one combining rule this engine has never had at all.
        assert_eq!(
            decide(&colliding_policy(ConflictStrategy::Perm), &claims_with(&[]), &all_actions_config(), "read", ASSET, &[])
                .decision,
            Decision::Allow
        );
    }

    #[test]
    fn conflict_prohibit_and_conflict_invalid_both_deny_a_genuine_collision() {
        // Same decision, different reasons -- `prohibit` resolves the
        // policy prohibition-first, `invalid` refuses to resolve it at all.
        // The two are told apart by the wire trace (`wire.rs`), which is
        // deliberately the only place they differ: there is no fourth
        // `Decision` variant for a void policy.
        for conflict in [ConflictStrategy::Prohibit, ConflictStrategy::Invalid] {
            assert_eq!(
                decide(&colliding_policy(conflict), &claims_with(&[]), &all_actions_config(), "read", ASSET, &[]).decision,
                Decision::Deny,
                "{conflict:?}"
            );
        }
    }

    #[test]
    fn a_conflict_strategy_changes_nothing_for_a_policy_with_no_genuine_collision() {
        // The regression guard for the whole addition: `odrl:conflict` is
        // consulted only where a permission and a prohibition both hold for
        // the same request. Every other shape -- which is every shape any
        // fixture in this workspace or in the vendored compliance corpus
        // actually has -- must answer identically under all three
        // strategies, including the new default.
        let alice = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let fixtures: Vec<(&str, Policy, Claims, ResolvedConfig, &str)> = vec![
            (
                "permission only, matching",
                Policy { permissions: vec![Rule::new("read", vec![])], ..Policy::default() },
                claims_with(&[]),
                all_actions_config(),
                ASSET,
            ),
            (
                "prohibition only, matching",
                Policy { prohibitions: vec![Rule::new("read", vec![])], ..Policy::default() },
                claims_with(&[]),
                all_actions_config(),
                ASSET,
            ),
            (
                "permission matches, prohibition misses on its constraint",
                Policy {
                    permissions: vec![Rule::new("read", vec![])],
                    prohibitions: vec![Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")])],
                    ..Policy::default()
                },
                alice.clone(),
                all_actions_config(),
                ASSET,
            ),
            (
                "prohibition matches, permission misses on its constraint",
                Policy {
                    permissions: vec![Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")])],
                    prohibitions: vec![Rule::new("read", vec![])],
                    ..Policy::default()
                },
                alice.clone(),
                all_actions_config(),
                ASSET,
            ),
            (
                "both rules present but about different assets, so only one applies",
                Policy {
                    permissions: vec![Rule::targeting("read", ASSET, vec![])],
                    prohibitions: vec![Rule::targeting("read", "urn:uuid:other-asset", vec![])],
                    ..Policy::default()
                },
                claims_with(&[]),
                all_actions_config(),
                ASSET,
            ),
            (
                "both rules present but about different actions",
                Policy {
                    permissions: vec![Rule::new("read", vec![])],
                    prohibitions: vec![Rule::new("write", vec![])],
                    ..Policy::default()
                },
                claims_with(&[]),
                all_actions_config(),
                ASSET,
            ),
            (
                "empty permissions under behaviour: open, with a matching prohibition",
                Policy { prohibitions: vec![Rule::new("read", vec![])], ..Policy::default() },
                claims_with(&[]),
                config_with(&["read", "write"], DutyMode::Advise, Behaviour::Open),
                ASSET,
            ),
            (
                "nothing matches at all under behaviour: closed",
                Policy {
                    permissions: vec![Rule::new("read", vec![Constraint::new("sub", Operator::Eq, "bob")])],
                    ..Policy::default()
                },
                alice.clone(),
                config_with(&["read", "write"], DutyMode::Advise, Behaviour::Closed),
                ASSET,
            ),
            (
                "an unresolved policy-level obligation under duty_mode: deny",
                Policy {
                    permissions: vec![Rule::new("read", vec![])],
                    obligations: vec![Rule::new("write", vec![])],
                    ..Policy::default()
                },
                claims_with(&[]),
                config_with(&["read", "write"], DutyMode::Deny, Behaviour::Open),
                ASSET,
            ),
        ];

        for (label, policy, claims, config, target) in fixtures {
            let baseline = decide(&policy, &claims, &config, "read", target, &[]);
            for conflict in EVERY_STRATEGY {
                let declared = Policy { conflict, ..policy.clone() };
                assert_eq!(
                    decide(&declared, &claims, &config, "read", target, &[]),
                    baseline,
                    "{label}: declaring odrl:conflict {conflict:?} changed a decision with no \
                     genuine collision in it"
                );
            }
        }
    }

    #[test]
    fn an_empty_permissions_list_is_never_a_conflict_however_open_the_behaviour_is() {
        // `Behaviour::Open` meets the permission *requirement* vacuously
        // for a policy with no permissions at all -- but there is no
        // permission to win a conflict, so `perm` has nothing to promote
        // and the prohibition denies under every strategy. Keyed off an
        // actual granting permission rather than off
        // `permission_requirement_met` for exactly this reason.
        let policy = Policy { prohibitions: vec![Rule::new("read", vec![])], ..Policy::default() };
        let config = config_with(&["read"], DutyMode::Advise, Behaviour::Open);
        for conflict in EVERY_STRATEGY {
            assert_eq!(
                decide(&Policy { conflict, ..policy.clone() }, &claims_with(&[]), &config, "read", ASSET, &[]).decision,
                Decision::Deny,
                "{conflict:?}"
            );
        }
    }

    #[test]
    fn conflict_resolution_counts_only_permissions_that_actually_grant() {
        // A permission whose own `odrl:duty` is outstanding under
        // `duty_mode: deny` does not grant (`Rule::grants`), so it is not a
        // party to a conflict either: `perm` cannot promote a permission
        // that is not in force. The control below -- the identical policy
        // with the duty's claim supplied -- is what makes this an
        // assertion about the duty rather than about `perm` being inert.
        let policy = Policy {
            permissions: vec![Rule {
                duty: vec![Rule::new(
                    "compensate",
                    vec![Constraint::new("duty:compensate", Operator::Eq, "fulfilled")],
                )],
                ..Rule::new("read", vec![])
            }],
            prohibitions: vec![Rule::new("read", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::Perm,
        };
        let config = config_with(&["read", "compensate"], DutyMode::Deny, Behaviour::Open);
        assert_eq!(
            decide(&policy, &claims_with(&[]), &config, "read", ASSET, &[]).decision,
            Decision::Deny,
            "an outstanding per-permission duty keeps the permission out of the conflict"
        );
        assert_eq!(
            decide(&policy, &fulfilled(&["compensate"]), &config, "read", ASSET, &[]).decision,
            Decision::Allow,
            "the control: with the duty resolved the permission grants, the conflict is genuine, \
             and `perm` resolves it"
        );
    }

    #[test]
    fn performable_actions_inherits_the_conflict_strategy_rather_than_re_deciding_it() {
        // The enumeration entry point is a thin wrapper over `decide`, and
        // that has to hold for this axis too: under `perm` every action the
        // colliding pair covers comes back, under the `invalid` default
        // none of them does.
        let policy = Policy {
            permissions: vec![Rule::new("use", vec![])],
            prohibitions: vec![Rule::new("use", vec![])],
            obligations: vec![],
            conflict: ConflictStrategy::Perm,
        };
        let config = taxonomy_config(Behaviour::Open, DutyMode::Advise);
        assert_eq!(
            performable_actions(&policy, &claims_with(&[]), &config, ASSET, &[]),
            vec!["read".to_string(), "use".to_string(), "write".to_string()]
        );
        assert!(
            performable_actions(
                &Policy { conflict: ConflictStrategy::Invalid, ..policy },
                &claims_with(&[]),
                &config,
                ASSET
            , &[])
            .is_empty(),
            "a void policy performs nothing"
        );
    }

    // -- xsd:duration comparison (lt/lteq/gt/gteq) --------------------------

    #[test]
    fn a_prohibition_keyed_on_a_duration_left_operand_denies_once_duration_literals_compare() {
        // The exact distinguishing example from the backlog item: an
        // odrl:meteredTime prohibition, both sides genuine xsd:duration
        // literals (the type ODRL 2.2 Vocabulary Section 4.5 actually
        // prescribes for delayPeriod/elapsedTime/meteredTime/timeInterval),
        // under the engine's own default Behaviour::Open with an empty
        // permissions list. Before a duration parser existed, PT10H and
        // PT8H matched neither the temporal (dateTime/date) reading nor the
        // plain-f64 reading, so `ordering_matches` returned `false`
        // unconditionally, the prohibition never fired, and the empty
        // permissions list under Open resolved to a silent Allow -- wrong,
        // since PT10H is genuinely greater than PT8H.
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new(
                "use",
                vec![Constraint::new("meteredTime", Operator::Gt, "PT8H")],
            )],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["use"]);
        let claims = claims_with(&[("meteredTime", ClaimValue::Single("PT10H".into()))]);

        assert_eq!(
            decide(&policy, &claims, &config, "use", ASSET, &[]).decision,
            Decision::Deny,
            "PT10H > PT8H: the prohibition must fire rather than silently missing"
        );
    }

    #[test]
    fn the_same_prohibition_does_not_fire_when_the_duration_claim_is_not_actually_greater() {
        // The control for the test above: same policy, same operator, a
        // claim that genuinely does not exceed the threshold.
        let policy = Policy {
            permissions: vec![],
            prohibitions: vec![Rule::new(
                "use",
                vec![Constraint::new("meteredTime", Operator::Gt, "PT8H")],
            )],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
        };
        let config = config_recognizing(&["use"]);
        let claims = claims_with(&[("meteredTime", ClaimValue::Single("PT8H".into()))]);

        assert_eq!(
            decide(&policy, &claims, &config, "use", ASSET, &[]).decision,
            Decision::Allow,
            "PT8H is not > PT8H: the prohibition's own constraint correctly misses, \
             and the empty permissions list under Open allows"
        );
    }
}
