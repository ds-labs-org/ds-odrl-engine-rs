//! The request/response wire contract (case study Section 5.1-5.2): the
//! JSON shape a host (or the WASM ABI of `crate::abi`) actually sends and
//! receives, plus `evaluate_request`, the pure function that drives it.
//!
//! `policies` mirrors `catalog_core::Policy`/`Rule`/`Constraint` field for
//! field (Section 5.2) — `WirePolicy` therefore carries `id`, `kind`,
//! `assigner` and `assignee` that `decision::Policy` deliberately drops
//! (that type keeps only what Section 4.3's algorithm consumes). This
//! module is where the two meet: it re-adds the identity fields a
//! multi-policy request needs to report *which* policy decided, on top of
//! the permission/prohibition/obligation lists `decision::Policy` already
//! knows how to evaluate.
//!
//! One of those identity fields is now read rather than merely carried.
//! When — and only when — the config names a `partyIdentityClaim`, a
//! policy's `odrl:assignee` is compared against the caller that claim
//! identifies, and a policy addressed to somebody else is dropped from
//! the set before `decide` ever sees it (`party_role_mismatch`,
//! `evaluate_request_for_action`). That check lives here rather than in
//! `decision` precisely because it is about a field only this layer has:
//! `decision::decide` still takes a party-less `decision::Policy` and is
//! untouched by the setting.
//!
//! `WirePolicy` also carries `odrl:inheritFrom` (Information Model §2.9),
//! the second field this layer reads for itself: `resolve_inherit_from`
//! walks each policy's declared parents — by `id`, within this same
//! request's `policies` list — and replicates their rules and party fields
//! into it, before party-role scoping or `decide` sees any policy at all.
//! See that function's doc comment for the exact MUST list this covers,
//! what this contract has no field for, and how a circular chain is
//! rejected.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::claims::Claims;
use crate::constraint::{Constraint, Operator, MAX_CONSTRAINT_DEPTH};
use crate::decision::{
    conflicting_rules, decide, ConflictStrategy, Decision, DecisionOutcome, DutyAttachment, Policy, Rule,
};
use crate::profile::{ActionDecl, Behaviour, DutyMode, ResolvedConfig};

/// A JSON-LD reference to another node by IRI — `{"@id": "..."}"`, ODRL's
/// own convention for "this property's value is another resource," used
/// here for `WireActionDecl`'s `odrl:includedIn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNodeRef {
    #[serde(rename = "@id")]
    pub id: String,
}

/// One entry of `RequestConfig`'s `odrl:action` list: real ODRL/JSON-LD
/// terms (`@id`, `odrl:includedIn`), not the bare-string shape this field
/// carried before this revision. Round-trips losslessly with
/// `profile::ActionDecl` (`From` impls below) — this type exists only
/// because the wire's field names are ODRL-shaped and `ActionDecl`'s
/// aren't (and shouldn't be: `ActionDecl` is an internal type with no
/// wire-format obligations of its own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireActionDecl {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "odrl:includedIn", default, skip_serializing_if = "Option::is_none")]
    pub included_in: Option<WireNodeRef>,
}

impl From<&ActionDecl> for WireActionDecl {
    fn from(a: &ActionDecl) -> Self {
        WireActionDecl { id: a.id.clone(), included_in: a.included_in.clone().map(|id| WireNodeRef { id }) }
    }
}

impl From<&WireActionDecl> for ActionDecl {
    fn from(a: &WireActionDecl) -> Self {
        ActionDecl { id: a.id.clone(), included_in: a.included_in.as_ref().map(|r| r.id.clone()) }
    }
}

/// Section 5.2's `config` object: the host's already-resolved union of its
/// loaded profiles (Section 4.4), travelling in the request itself so the
/// engine stays stateless. Unlike `profile::Profile`, this carries no
/// `id` — a resolved config is anonymous by the time it reaches the wire.
///
/// Reshaped, this revision, into real ODRL/JSON-LD vocabulary
/// (`@type`/`@id`/`odrl:action`/`odrl:includedIn`) rather than the bare
/// `{"recognized_actions": [...]}` shape earlier revisions used — the
/// underlying information (which actions are known, which broader action
/// each is `includedIn`, and the duty-handling knob) is unchanged; only
/// the wire's own field names now say what they mean in ODRL's own terms.
/// `duty_mode` stays `dutyMode`, not an `odrl:`-namespaced term: Section
/// 4.5's own doc comment already establishes ODRL defines no property for
/// a profile to declare its own enforcement behavior, and inventing one
/// here would misrepresent this engine's own invention as real ODRL
/// vocabulary. `@type` is carried for shape, not validated — a caller
/// naming anything other than `"odrl:Profile"` there is not rejected, the
/// field exists so the object reads as self-describing JSON-LD without
/// this engine taking on a JSON-LD processor's actual obligations.
///
/// `behaviour` (new this revision) is the ODRL Community Group's own
/// Formal Semantics draft term (Section 3.6) — unlike `dutyMode`, this
/// *is* the standards body's own named concept, so it keeps that name
/// rather than being invented here, though it stays outside the `odrl:`
/// namespace too since the draft does not clearly define a corresponding
/// RDF property for it, only an evaluator input parameter. `#[serde(default)]`
/// so a request built against an earlier revision of this wire contract
/// (before this field existed) still deserializes, defaulting to `Open`
/// — Section 4.3's own original, unconditional behavior, unchanged for
/// any caller that never sets this.
///
/// `partyIdentityClaim` (new this revision) is the same kind of knob and
/// obeys the same rule: absent means absent, and absent is what every
/// request built before it existed says. It names **which key of
/// `Request::claims` carries the caller's own identity**, and by naming it
/// switches on comparison of a policy's `odrl:assignee` against that
/// caller (`profile::ResolvedConfig::party_identity_claim` carries the full
/// rationale; `party_role_mismatch` below is the exact comparison).
/// Omitted, or `null`, and no policy's `assignee` is consulted at all —
/// which is what this engine did unconditionally until now. It stays
/// outside the `odrl:` namespace for the same reason `dutyMode` does: ODRL
/// defines no property by which a policy or a profile declares the shape of
/// somebody else's claims map. Skipped on serialization when unset, so a
/// config that never names one is byte-for-byte the object it always was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestConfig {
    #[serde(rename = "@type")]
    pub type_: String,
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "odrl:action")]
    pub actions: Vec<WireActionDecl>,
    #[serde(rename = "dutyMode")]
    pub duty_mode: DutyMode,
    #[serde(default)]
    pub behaviour: Behaviour,
    #[serde(rename = "partyIdentityClaim", default, skip_serializing_if = "Option::is_none")]
    pub party_identity_claim: Option<String>,
}

impl From<&RequestConfig> for ResolvedConfig {
    fn from(config: &RequestConfig) -> Self {
        let resolved = ResolvedConfig::new(
            config.actions.iter().map(ActionDecl::from).collect(),
            config.duty_mode,
            config.behaviour,
        );
        match &config.party_identity_claim {
            Some(claim) => resolved.with_party_identity_claim(claim.clone()),
            None => resolved,
        }
    }
}

/// One policy exactly as Section 5.2 documents it on the wire: the
/// identity fields (`id`, `kind`, `assigner`, `assignee`) that
/// `decision::Policy` has no use for, plus the same permission/
/// prohibition/obligation lists that type does consume.
///
/// `assignee` is the one of those four this module itself reads, and only
/// when the config asks it to — see `party_role_mismatch`. `kind` and
/// `assigner` remain carried and never evaluated: nothing here selects a
/// semantics from `kind` (an `Agreement` is evaluated exactly as a `Set`
/// is), and `assigner` names who granted the policy rather than who is
/// asking for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePolicy {
    pub id: String,
    pub kind: String,
    pub assigner: String,
    /// The party this policy is **addressed to** (ODRL's `odrl:assignee`).
    /// Read only when `ResolvedConfig::party_identity_claim` names the
    /// claim key that identifies the caller; `None` — the common case in
    /// the vendored corpus — means there is no party role to check and the
    /// policy applies to whoever asks, exactly as it always did.
    pub assignee: Option<String>,
    #[serde(default)]
    pub permissions: Vec<Rule>,
    #[serde(default)]
    pub prohibitions: Vec<Rule>,
    #[serde(default)]
    pub obligations: Vec<Rule>,
    /// `odrl:conflict` — this policy's own [`ConflictStrategy`], mirroring
    /// `decision::Policy::conflict` field for field, as every rule list
    /// above already does. Unlike `kind`/`assigner`, it is not an identity
    /// field this layer merely carries: it is read by `decide` and named by
    /// `describe_reason`. Absent from the JSON means ODRL's own default,
    /// `invalid`, and a policy meaning the default serializes without the
    /// key — so every request and every stored fixture built before this
    /// field existed parses and re-serializes byte-for-byte unchanged.
    #[serde(rename = "odrl:conflict", default, skip_serializing_if = "ConflictStrategy::is_default")]
    pub conflict: ConflictStrategy,
    /// `odrl:inheritFrom` — Information Model §2.9's Policy Inheritance:
    /// the `id`s of zero or more parent policies **elsewhere in this same
    /// request's `policies` list** whose rules this (child) policy
    /// replicates before `decide` ever sees it. `None` (the JSON key
    /// absent, the common case for every fixture in this workspace) is a
    /// policy with no parent, evaluated exactly as it always was — this is
    /// the second field, after `conflict`, that this layer reads for
    /// itself rather than merely carrying.
    ///
    /// **Resolved by `resolve_inherit_from`, once per `evaluate_request`
    /// call, ahead of party-role scoping** (so an inherited `assignee`
    /// participates in it) **and ahead of `decide`** (so inherited rules
    /// are just more rules by the time the deny-override algorithm runs —
    /// there is no separate "inherited" bit anywhere past this point).
    /// See that function's own doc comment for exactly what §2.9's MUST
    /// list this replicates, what it deliberately does not (this contract
    /// has no policy-level Asset or `odrl:profile` field to replicate),
    /// and how a cycle is rejected.
    #[serde(rename = "inheritFrom", default, skip_serializing_if = "Option::is_none")]
    pub inherit_from: Option<Vec<String>>,
}

impl WirePolicy {
    fn as_decision_policy(&self) -> Policy {
        Policy {
            permissions: self.permissions.clone(),
            prohibitions: self.prohibitions.clone(),
            obligations: self.obligations.clone(),
            conflict: self.conflict,
        }
    }
}

/// Section 5.2's request envelope.
///
/// `action` (new this revision) is the one action this whole request is
/// *about* — what a caller is asking to do, evaluated against every
/// policy's own permission/prohibition rules via
/// `ResolvedConfig::covers`. Earlier revisions had no such field: a host
/// was responsible for pre-filtering a policy's rules to the one action
/// under evaluation and rewriting every surviving `Rule.action` to equal
/// it, before this engine ever saw the request — real coverage matching
/// (a permission for `transfer` covering a request for `sell`) was
/// therefore entirely a host-side concern. It is now this engine's own.
///
/// **`dataset_id` is this request's `odrl:target`**, and not merely the
/// value echoed back in the response. Since `Rule::target` exists, each
/// rule may scope itself to one asset; the asset a rule is compared
/// against is this field, because it is the asset handle this contract
/// has always carried and a second, separate `target` field beside it
/// would be two sources of truth for one thing — with no rule for what a
/// host should do when they disagree. A rule naming no `odrl:target` is
/// about whatever this field names, which is exactly how every rule of
/// every policy behaved before per-rule targets existed.
///
/// `asset_collections` (new this revision) is a host-supplied fact
/// channel about `dataset_id` itself — the exact counterpart of `claims`
/// for the asset side of the request rather than the party side: every
/// `odrl:AssetCollection` (ODRL 2.2 Vocabulary §3.4.2) `dataset_id` is
/// asserted to be `odrl:partOf` (§3.8.1), so a rule's `odrl:target`
/// naming one of those collection IRIs applies to this request too, not
/// only to a request naming the collection itself. This engine resolves
/// no membership on its own — no graph, no transitive closure, no IRI
/// normalization — the same honest limit `Rule::target` already states;
/// the host resolves membership against its own catalog before building
/// the request, exactly as `compliance-runner`'s own `is_member_of`
/// adapter already does for the vendored compliance corpus (see this
/// crate's README's "Per-rule assets" section). `#[serde(default)]` and
/// skipped when empty, so a request built before this field existed — or
/// one naming an asset with no collection membership at all — parses and
/// re-serializes byte for byte as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub dataset_id: String,
    pub action: String,
    pub config: RequestConfig,
    #[serde(default)]
    pub policies: Vec<WirePolicy>,
    #[serde(default)]
    pub claims: Claims,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asset_collections: Vec<String>,
}

/// The wire form of `decision::Decision`: the three bare strings Section
/// 5.2 documents (`"Allow"`/`"Deny"`/`"Error"`), with the `Error` variant's
/// `UnrecognizedAction` payload folded into `Response::reason` instead of
/// serialized here — Section 5.2 is explicit that `reason` is where the
/// diagnostic detail lives, not a structured field a caller should parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireDecision {
    Allow,
    Deny,
    Error,
}

/// One entry of Section 5.2's `duties` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DutyEntry {
    pub policy_id: String,
    pub action: String,
    pub resolved: bool,
    /// Where in the policy this duty was attached, when that is anything
    /// other than a plain policy-level obligation —
    /// `permission[0].duty[0]`, `prohibition[0].remedy[0]`, or any of those
    /// with one `.consequence` segment per `odrl:consequence` hop walked
    /// (`decision::UnresolvedDuty::path`). A host that only ever sends
    /// policy-level obligations never sees this key: it is
    /// `#[serde(default)]` and skipped when `None`, so Section 5.2's
    /// original three-field entry is byte-for-byte what it was — which is
    /// what keeps this addition additive on the response side as well as
    /// on the request side.
    ///
    /// It is diagnostic provenance, on the same footing `reason` is: a
    /// host acting on the duty needs `action`, and this says which rule
    /// asked for it, so a caller holding several outstanding duties can
    /// tell them apart without re-deriving the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Section 5.2's response envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub dataset_id: String,
    pub decision: WireDecision,
    pub reason: String,
    pub duties: Vec<DutyEntry>,
}

/// Wire name for one `Operator`, shared by `describe_constraint` below —
/// factored out of that function rather than duplicated, since it's the
/// same "how should this operator print in a human trace" question.
fn operator_wire_name(operator: Operator) -> &'static str {
    match operator {
        Operator::Eq => "eq",
        Operator::Neq => "neq",
        Operator::IsAnyOf => "isAnyOf",
        Operator::IsAllOf => "isAllOf",
        Operator::IsNoneOf => "isNoneOf",
        Operator::IsPartOf => "isPartOf",
        Operator::Lt => "lt",
        Operator::Lteq => "lteq",
        Operator::Gt => "gt",
        Operator::Gteq => "gteq",
    }
}

/// Renders one `Constraint` — atomic or a nested `odrl:and`/`odrl:or`/
/// `odrl:xone` group — into the same short human-readable form
/// `describe_rule`'s `reason` trace already used for the flat case, now
/// recursing into nested children. Not exhaustive (a deep tree reads as
/// deeply parenthesized, not specially summarized), but never garbled or
/// panicking: recursion is bounded by the same `MAX_CONSTRAINT_DEPTH`
/// `Constraint::evaluate` itself is bounded by, past which this prints a
/// fixed placeholder instead of continuing to recurse — see that
/// constant's own doc comment in `constraint.rs`.
fn describe_constraint(constraint: &Constraint, depth: usize) -> String {
    if depth > MAX_CONSTRAINT_DEPTH {
        return "<constraint nested past MAX_CONSTRAINT_DEPTH>".to_string();
    }
    // Same xone > or > and > atomic precedence `Constraint::evaluate` uses.
    if let Some(xone) = &constraint.xone {
        let joined = xone.iter().map(|c| describe_constraint(c, depth + 1)).collect::<Vec<_>>().join(", ");
        return format!("xone({joined})");
    }
    if let Some(or) = &constraint.or {
        return join_children(or, " || ", depth);
    }
    if let Some(and) = &constraint.and {
        return join_children(and, " && ", depth);
    }
    format!("{} {} {}", constraint.left_operand, operator_wire_name(constraint.operator), constraint.right_operand)
}

fn join_children(children: &[Constraint], separator: &str, depth: usize) -> String {
    let joined = children.iter().map(|c| describe_constraint(c, depth + 1)).collect::<Vec<_>>().join(separator);
    format!("({joined})")
}

/// Renders a rule's `odrl:refinement`, if it has one, as the bracketed
/// suffix `describe_rule` appends to the action clause — `[copies lteq 2]`.
/// Brackets rather than the parentheses `describe_constraint` itself uses
/// for a logical group, so a logical refinement reads as
/// `[(a && b)]` rather than as an ambiguous doubled `((a && b))`.
fn describe_refinement(rule: &Rule) -> String {
    match &rule.action_refinement {
        Some(refinement) => format!(" refined by [{}]", describe_constraint(refinement, 0)),
        None => String::new(),
    }
}

/// Renders a rule's `odrl:target`, if it has one, as the clause
/// `describe_rule` appends after the action clause — `on target
/// 'urn:asset:A'`. A rule naming no target adds nothing at all, so every
/// trace this engine produced before per-rule targets existed is
/// byte-for-byte what it was.
fn describe_target(rule: &Rule) -> String {
    match &rule.target {
        Some(target) => format!(" on target '{target}'"),
        None => String::new(),
    }
}

/// Renders the status of a permission's own `odrl:duty` chains as the
/// clauses `describe_reason` appends after a permission that matched —
/// `; odrl:duty[0] 'compensate' satisfied`, or `; odrl:duty[0] 'compensate'
/// unresolved (advisory under duty_mode: advise)`. A permission carrying no
/// duty adds nothing at all, so every trace this engine produced before
/// per-permission duties existed is byte-for-byte what it was.
///
/// The unresolved case names the duty actually outstanding, which after an
/// `odrl:consequence` hop is the consequence rather than the duty it
/// replaced — and labels it as such, so a satisfied primary and a satisfied
/// consequence never read the same.
fn describe_permission_duties(rule: &Rule, claims: &Claims, duty_mode: DutyMode) -> String {
    let mut out = String::new();
    for (duty_index, duty) in rule.duty.iter().enumerate() {
        match crate::decision::outstanding_duty(duty, claims) {
            None => out.push_str(&format!("; odrl:duty[{duty_index}] '{}' satisfied", duty.action)),
            Some(outstanding) => out.push_str(&format!(
                "; odrl:duty[{duty_index}]{} '{}' unresolved (advisory under duty_mode: {})",
                ".consequence".repeat(outstanding.consequence_depth),
                outstanding.action,
                duty_mode_wire_name(duty_mode),
            )),
        }
    }
    out
}

/// Renders the status of a prohibition's `odrl:remedy` chains as the
/// clauses `describe_reason` appends after a prohibition that matched.
/// Both directions are printed — a *satisfied* remedy is stated too, and
/// stated as not lifting anything, because "the prohibition denied and the
/// remedy is done" is exactly the reading a caller might otherwise expect
/// to have produced an Allow. See `decision::decide`'s doc comment for why
/// it does not.
fn describe_remedies(rule: &Rule, claims: &Claims) -> String {
    let mut out = String::new();
    for (duty_index, remedy) in rule.remedy.iter().enumerate() {
        match crate::decision::outstanding_duty(remedy, claims) {
            None => out.push_str(&format!(
                "; its odrl:remedy[{duty_index}] '{}' is satisfied, which does not lift the prohibition",
                remedy.action
            )),
            Some(outstanding) => out.push_str(&format!(
                "; its odrl:remedy[{duty_index}]{} '{}' is unresolved and does not lift the prohibition",
                ".consequence".repeat(outstanding.consequence_depth),
                outstanding.action
            )),
        }
    }
    out
}

/// The wire spelling of a `DutyMode`, for the trace only — the same
/// question `operator_wire_name` above answers for an `Operator`.
fn duty_mode_wire_name(duty_mode: DutyMode) -> &'static str {
    match duty_mode {
        DutyMode::Advise => "advise",
        DutyMode::Deny => "deny",
    }
}

fn describe_rule(rule: &Rule, requested_action: &str) -> String {
    let action_clause = if rule.action == requested_action {
        format!("action '{}'{}{}", rule.action, describe_refinement(rule), describe_target(rule))
    } else {
        format!(
            "action '{}'{} covers requested '{requested_action}'{}",
            rule.action,
            describe_refinement(rule),
            describe_target(rule)
        )
    };
    if rule.constraints.is_empty() {
        return format!("{action_clause}, unconstrained");
    }
    let constraints = rule
        .constraints
        .iter()
        .map(|c| describe_constraint(c, 0))
        .collect::<Vec<_>>()
        .join(" && ");
    format!("{action_clause}: {constraints}")
}

/// Reconstructs a human-readable trace of *which* rule/constraint drove
/// one policy's outcome (Section 5.2's `reason` field), by re-running the
/// same match tests `decide` used internally — now including the same
/// whole action requirement (`Rule::action_applies`: coverage *and* the
/// action's own `odrl:refinement`), so a rule that didn't apply because
/// it names an uncovering action, or because its refined action isn't the
/// one requested, is not mistaken for one that applied but missed on
/// constraints. `decide` itself returns only the outcome, not
/// the trace, so this walks the policy a second time in the same
/// precedence order (prohibitions, then permissions, then duty-forcing)
/// rather than threading tracing state through the decision algorithm
/// itself.
fn describe_reason(
    policy: &WirePolicy,
    outcome: &DecisionOutcome,
    claims: &Claims,
    requested_action: &str,
    requested_target: &str,
    asset_collections: &[String],
    config: &ResolvedConfig,
) -> String {
    // `Rule::applies`, not `covers_action` alone: applicability is the
    // rule's target *and* its action requirement (coverage plus the
    // action's own `odrl:refinement`). Getting this wrong is not merely a
    // cosmetic trace defect — reconstructing the permission requirement
    // without the refinement made this function conclude the requirement
    // *was* met for a rule `decide` had correctly found inapplicable, and
    // fall through to "denied for a reason this trace could not
    // reconstruct" (the same failure shape the `beh-closed-empty` probe
    // found for `behaviour: "closed"`). A per-rule target left out here
    // would reproduce exactly that bug for exactly that input shape.
    let covers_and_matches = |rule: &Rule| {
        rule.applies(requested_action, requested_target, asset_collections, config, claims) && rule.matches(claims)
    };
    // A rule that would have applied in every other respect and is
    // inapplicable *purely* because it is about a different asset — the
    // one case where the target is the whole reason and deserves naming,
    // kept distinct from an action mismatch so a denied request's trace
    // says which of the two actually failed.
    let blocked_only_by_target = |rule: &Rule| {
        !rule.target_applies(requested_target, asset_collections)
            && rule.action_applies(requested_action, config, claims)
            && rule.matches(claims)
    };
    // A rule that covered the requested action and satisfied all its own
    // constraints, and was inapplicable *purely* because of its action
    // refinement — the one case where the refinement is the whole reason
    // and deserves to be named as such.
    let blocked_only_by_refinement = |rule: &Rule| {
        rule.target_applies(requested_target, asset_collections)
            && rule.covers_action(requested_action, config)
            && rule.matches(claims)
            && !rule.refinement_satisfied(claims)
    };

    // The `odrl:conflict` collision `decide` itself branched on, asked of
    // the same shared predicate rather than re-derived here — the whole
    // reason `decision::conflicting_rules` is `pub(crate)`. `None` for
    // every policy in which at most one of a permission and a prohibition
    // holds, which is every policy any fixture in this workspace has, so
    // none of the three clauses below can change a trace that existed
    // before this field did.
    let conflict = conflicting_rules(
        &policy.as_decision_policy(),
        claims,
        config,
        requested_action,
        requested_target,
        asset_collections,
    );

    match &outcome.decision {
        Decision::Error(unrecognized) => format!("policy '{}': {unrecognized}", policy.id),
        Decision::Deny => {
            // A void policy first, ahead of the prohibition branch: under
            // `odrl:conflict: invalid` the prohibition did not win, it
            // simply failed to be reconciled with a permission that holds
            // just as strongly, and reporting it as the deciding rule would
            // claim a resolution the policy explicitly refuses. This is the
            // only observable difference between `invalid` and `prohibit`
            // — both `Deny` — so it is load-bearing rather than cosmetic.
            if let Some(found) = conflict {
                if policy.conflict == ConflictStrategy::Invalid {
                    return format!(
                        "policy '{}' is void: permission[{}] and prohibition[{}] both matched requested \
                         action '{requested_action}', and the policy's odrl:conflict strategy is 'invalid' \
                         (ODRL's own default), which voids a conflicting policy rather than resolving it",
                        policy.id, found.permission_index, found.prohibition_index
                    );
                }
            }

            if let Some((index, rule)) = policy.prohibitions.iter().enumerate().find(|(_, rule)| covers_and_matches(rule))
            {
                // The conflict clause is appended only where a permission
                // really did hold too and `prohibit` really did settle it,
                // so "prohibition-first because this policy chose it" and
                // "prohibition-first because nothing contested it" are
                // never the same string.
                let resolved = match conflict {
                    Some(found) if policy.conflict == ConflictStrategy::Prohibit => format!(
                        "; odrl:conflict 'prohibit' resolves the conflict with permission[{}] in the \
                         prohibition's favour",
                        found.permission_index
                    ),
                    _ => String::new(),
                };
                return format!(
                    "prohibition[{index}] of policy '{}' matched: {}{}{resolved}",
                    policy.id,
                    describe_rule(rule, requested_action),
                    describe_remedies(rule, claims)
                );
            }

            // The same `match config.behaviour` arms `decide` itself uses
            // (decision.rs). Reconstructing this branch with `Open`'s rule
            // hardcoded — `permissions.is_empty() || ...`, as this line
            // read before — got the *decision* right and the *trace*
            // wrong for exactly one input: an empty `permissions` list
            // under `behaviour: "closed"`. `decide` correctly denies it;
            // this function then saw `permissions.is_empty()` as
            // satisfying the requirement, skipped the closed-default
            // branch, found no unresolved duty either, and fell through
            // to "denied for a reason this trace could not reconstruct" —
            // an unhelpful non-answer for a perfectly ordinary,
            // deliberately-configured decision. Found by a coverage probe
            // (`beh-closed-empty`) built to assert exactly this reason.
            //
            // `Rule::grants`, not `covers_and_matches`, for exactly the
            // same reason `decide` uses it: under `duty_mode: deny` a
            // permission with an outstanding `odrl:duty` does not grant,
            // and a trace reconstructing the requirement without that
            // gating would once again conclude the requirement was met for
            // a rule `decide` had correctly found not to grant.
            let any_permission_grants =
                policy.permissions.iter().any(|rule| rule.grants(requested_action, requested_target, asset_collections, config, claims));
            let permission_requirement_met = match config.behaviour {
                Behaviour::Open => policy.permissions.is_empty() || any_permission_grants,
                Behaviour::Closed => any_permission_grants,
            };
            if !permission_requirement_met {
                // A permission that applied and matched in every respect
                // and was stopped only by its own per-permission duty
                // under `duty_mode: deny`. Named ahead of the generic
                // closed-default line for the same reason the target and
                // refinement branches below are: "no permission covered
                // and matched" is flatly wrong here — one did, and a duty
                // it carries is the whole story.
                if let Some((index, rule)) =
                    policy.permissions.iter().enumerate().find(|(_, rule)| covers_and_matches(rule))
                {
                    if let Some(outstanding) = rule
                        .duty
                        .iter()
                        .enumerate()
                        .find_map(|(duty_index, duty)| {
                            crate::decision::outstanding_duty(duty, claims).map(|found| (duty_index, found))
                        })
                    {
                        let (duty_index, found) = outstanding;
                        return format!(
                            "permission[{index}] of policy '{}' matched, but its odrl:duty[{duty_index}]{} \
                             '{}' is unresolved under duty_mode: deny",
                            policy.id,
                            ".consequence".repeat(found.consequence_depth),
                            found.action
                        );
                    }
                }
                // Target first, then refinement: the two branches are
                // mutually exclusive by construction (this one requires the
                // action requirement to hold, which includes the
                // refinement), so the order is for reading, not for
                // correctness.
                if let Some((index, rule)) =
                    policy.permissions.iter().enumerate().find(|(_, rule)| blocked_only_by_target(rule))
                {
                    return format!(
                        "permission[{index}] of policy '{}' covers requested action \
                         '{requested_action}' but targets '{}', not the requested \
                         '{requested_target}'",
                        policy.id,
                        rule.target.as_deref().expect("blocked_only_by_target implies Some"),
                    );
                }
                if let Some((index, rule)) =
                    policy.permissions.iter().enumerate().find(|(_, rule)| blocked_only_by_refinement(rule))
                {
                    return format!(
                        "permission[{index}] of policy '{}' covers requested action \
                         '{requested_action}' but its action refinement was not satisfied: [{}]",
                        policy.id,
                        describe_constraint(
                            rule.action_refinement.as_ref().expect("blocked_only_by_refinement implies Some"),
                            0
                        )
                    );
                }
                return format!(
                    "no permission of policy '{}' covered and matched requested action '{requested_action}' (closed default)",
                    policy.id
                );
            }

            // Only a *policy-level* obligation can have forced this Deny
            // (`decide` keys its `duty_mode: deny` override off those
            // alone), so the search is for one of those rather than for
            // the first entry of a list that may now also carry
            // per-permission duties and remedies. `UnresolvedDuty::path`
            // renders `duty[0]` for the original shape and
            // `duty[0].consequence` once a consequence is what is actually
            // outstanding, so the message says which without a second
            // branch.
            match outcome.unresolved_duties.iter().find(|duty| duty.attachment == DutyAttachment::Obligation) {
                Some(duty) => format!(
                    "{} '{}' of policy '{}' is unresolved under duty_mode: deny",
                    duty.path(),
                    duty.action,
                    policy.id
                ),
                None => format!(
                    "policy '{}' denied for a reason this trace could not reconstruct",
                    policy.id
                ),
            }
        }
        Decision::Allow => {
            if policy.permissions.is_empty() {
                return format!("policy '{}' has no permissions (open default)", policy.id);
            }
            match policy
                .permissions
                .iter()
                .enumerate()
                .find(|(_, rule)| rule.grants(requested_action, requested_target, asset_collections, config, claims))
            {
                Some((index, rule)) => {
                    // `perm` is the only strategy that can reach an Allow
                    // over a matching prohibition, so this clause names the
                    // prohibition the policy chose to override — without
                    // it, an Allow issued in the teeth of a prohibition
                    // that really did match would read exactly like an
                    // Allow no prohibition contested.
                    let resolved = match conflict {
                        Some(found) if policy.conflict == ConflictStrategy::Perm => format!(
                            "; odrl:conflict 'perm' resolves the conflict with prohibition[{}] in the \
                             permission's favour",
                            found.prohibition_index
                        ),
                        _ => String::new(),
                    };
                    format!(
                        "permission[{index}] of policy '{}' matched: {}{}{resolved}",
                        policy.id,
                        describe_rule(rule, requested_action),
                        describe_permission_duties(rule, claims, config.duty_mode)
                    )
                }
                None => format!(
                    "policy '{}' allowed for a reason this trace could not reconstruct",
                    policy.id
                ),
            }
        }
    }
}

/// Why one policy is not being applied to this caller at all: it names an
/// `odrl:assignee`, party-role evaluation is switched on, and the caller
/// identified by `config.party_identity_claim` is somebody else.
///
/// Carries what the trace needs and nothing more. `observed` is the
/// caller's own value at that claim key, rendered as JSON — `None` when
/// the key is absent from the claims map entirely, which is a mismatch in
/// its own right (see `party_role_mismatch`).
struct PartyRoleMismatch<'a> {
    policy_id: &'a str,
    assignee: &'a str,
    claim_key: &'a str,
    observed: Option<String>,
}

impl PartyRoleMismatch<'_> {
    fn describe(&self) -> String {
        let observed = match &self.observed {
            Some(rendered) => format!("({rendered})"),
            None => "(absent from the claims map)".to_string(),
        };
        format!(
            "policy '{}' names odrl:assignee '{}', which does not match the caller's '{}' claim {observed}",
            self.policy_id, self.assignee, self.claim_key
        )
    }
}

/// Does `policy` fail to apply to the caller `claims` describes, on party
/// role — ODRL's `odrl:assignee`, the party a policy is *addressed to*.
/// `None` means the policy applies and is evaluated exactly as it always
/// has been.
///
/// **Three ways to get `None`, and only one to get `Some`.** The capability
/// is off unless `config.party_identity_claim` names a claim key (decision
/// 1: an existing host sees no change); a policy carrying no
/// `odrl:assignee` has no party role to check and is unaffected whether or
/// not the capability is on (decision 5, and the common case in the
/// vendored corpus); and a policy whose assignee the caller's identity
/// claim matches applies normally. Only a named assignee that the caller's
/// own identity does not match yields `Some`.
///
/// **The comparison is `ClaimValue::matches` — the engine's own `eq`
/// semantics**, not a bespoke string compare: opaque string equality for a
/// single-valued claim, and membership for a multi-valued one, so a caller
/// presenting several identifiers under one key (a `sub` list, a set of
/// DIDs) matches a policy naming any one of them. That reuse is the point:
/// party matching should not quietly be a *different* notion of equality
/// from the one every `eq` constraint in this engine already uses. There is
/// no IRI normalization and no `odrl:PartyCollection` membership here, on
/// exactly the footing `Rule::target` states for assets — "the same party"
/// means "the same characters".
///
/// **A claim key absent from the map is a mismatch, deliberately.** It is
/// the same direction `Constraint::evaluate` already takes for an absent
/// key (a miss, not an error), and it is the only safe one here: the
/// alternative — treating "I could not identify the caller" as "the caller
/// is whoever the policy names" — would make an unauthenticated request the
/// easiest way to collect a policy addressed to someone else.
///
/// **`assigner` is not evaluated, and that is a scope decision.** An
/// `odrl:assigner` identifies who *granted* the policy, not who is asking;
/// checking it against the caller's identity would be checking the wrong
/// party. Verifying that an assigner was entitled to grant what it granted
/// is a trust/provenance question about the policy's own issuance, which
/// this stateless engine has nothing to evaluate against.
fn party_role_mismatch<'a>(
    policy: &'a WirePolicy,
    claims: &Claims,
    config: &'a ResolvedConfig,
) -> Option<PartyRoleMismatch<'a>> {
    let claim_key = config.party_identity_claim.as_deref()?;
    let assignee = policy.assignee.as_deref()?;
    match claims.get(claim_key) {
        Some(value) if value.matches(assignee) => None,
        observed => Some(PartyRoleMismatch {
            policy_id: &policy.id,
            assignee,
            claim_key,
            observed: observed.map(|value| {
                serde_json::to_string(value).unwrap_or_else(|_| "unrenderable".to_string())
            }),
        }),
    }
}

/// `odrl:inheritFrom` resolution (Information Model §2.9), run once per
/// `evaluate_request` call over the whole request's `policies` list, before
/// party-role scoping or `decide` ever sees a policy.
///
/// **What §2.9 requires a child to replicate, and what this maps it to.**
/// The spec's own MUST list is "all policy-level Assets, Parties, Actions;
/// all profile identifiers; conflict properties; all Rules." This wire
/// contract has no policy-level Asset field (only `Rule::target`, per
/// rule, untouched here — a permission naming its own target keeps it after
/// inheriting) and no policy-level `odrl:profile` field at all (`config`
/// is a whole-request setting, not a per-policy one — see the
/// `other.profile-property` coverage row), so there is nothing to
/// replicate for either. What remains, and what this actually replicates
/// into the child:
///
/// - **Rules** — every entry of the parent's `permissions`, `prohibitions`
///   and `obligations` is appended to the child's own (child's rules
///   first, so a `reason` trace naming `permission[0]` still means "the
///   child's own first rule" for a child that declares any).
/// - **Parties** — `assigner` and `assignee` are the two this contract
///   carries. Each is replicated only when the child leaves it unset
///   (`assigner` the empty string, `assignee` `None`) — a child that
///   *does* name its own is not overridden by a parent's. This is the one
///   part of inheritance that can change a decision even for a child
///   declaring its own rules: a policy addressed to nobody in particular
///   picks up its parent's `odrl:assignee`, and so becomes subject to
///   party-role scoping (`party_role_mismatch`) it otherwise would not
///   have been.
///
/// `odrl:conflict` is deliberately **not** replicated: `ConflictStrategy`
/// has no wire representation for "unset" distinct from its own default
/// (`invalid`), the same ambiguity `#[serde(default)]` already accepts for
/// a policy that never declares the field at all, so there is no way to
/// tell "the child left this to inherit" apart from "the child declared
/// `invalid` itself" — inventing a rule for that ambiguity is a separate
/// decision from adding the field, not a side effect of this one. `kind`
/// is not replicated either: it is the child's own declared class, and
/// nothing here selects a semantics from it regardless.
///
/// **Multi-level and multi-parent, by construction.** A parent is itself
/// resolved (recursively, through this same function) before its rules are
/// copied into a child, so a grandparent's rules reach a grandchild
/// through its parent exactly once each — and a diamond (two parents
/// sharing a common ancestor) is resolved once and reused, not walked
/// twice, because a fully-resolved policy is cached by `id` the first time
/// any child reaches it.
///
/// **Circular inheritance MUST NOT occur, and is rejected, not looped.**
/// A parent chain that returns to a policy already being resolved fails
/// the whole request with `Decision::Error` (via a direct `Response`, the
/// same way an empty `policies` array or a party-role mismatch construct
/// their own outside `decide`) rather than silently truncating the chain
/// or overflowing the stack. Likewise, an `inheritFrom` naming an `id` that
/// is not any policy in this same request is an `Error`, not a `Deny`: in
/// both cases nothing about the *request being decided* is at fault — the
/// caller's own policy set is the thing that does not parse into a tree —
/// which is exactly the "configuration gap" distinction `Decision::Error`
/// exists to preserve elsewhere in this module.
fn resolve_inherit_from(policies: &[WirePolicy]) -> Result<Vec<WirePolicy>, String> {
    let by_id: HashMap<&str, &WirePolicy> = policies.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut resolved: HashMap<String, WirePolicy> = HashMap::with_capacity(policies.len());
    let mut stack: Vec<String> = Vec::new();

    for policy in policies {
        resolve_one(&policy.id, &by_id, &mut resolved, &mut stack)?;
    }

    Ok(policies
        .iter()
        .map(|p| resolved.get(&p.id).expect("resolved above").clone())
        .collect())
}

/// One policy's effective, post-inheritance form — see `resolve_inherit_from`
/// for what "effective" replicates. `resolved` is both the memo table (a
/// policy already fully resolved is cloned out, never recomputed, which is
/// what keeps a diamond of shared ancestors linear rather than exponential)
/// and, for a policy with no parents at all, exactly itself. `stack` is the
/// `id`s on the current recursion path, checked before `by_id` is even
/// consulted: an `id` already on it is a cycle, reported with the path that
/// found it rather than left to recurse until the real call stack overflows.
fn resolve_one(
    id: &str,
    by_id: &HashMap<&str, &WirePolicy>,
    resolved: &mut HashMap<String, WirePolicy>,
    stack: &mut Vec<String>,
) -> Result<WirePolicy, String> {
    if let Some(done) = resolved.get(id) {
        return Ok(done.clone());
    }
    if let Some(start) = stack.iter().position(|on_stack| on_stack == id) {
        let mut cycle = stack[start..].to_vec();
        cycle.push(id.to_string());
        return Err(format!(
            "circular odrl:inheritFrom chain, which Information Model \u{a7}2.9 requires be \
             rejected rather than resolved: {}",
            cycle.join(" -> ")
        ));
    }

    let policy = *by_id.get(id).ok_or_else(|| {
        format!(
            "a policy declares odrl:inheritFrom naming '{id}', which is not the id of any policy \
             in this same request's policies list"
        )
    })?;

    let parent_ids: &[String] = match &policy.inherit_from {
        Some(ids) if !ids.is_empty() => ids,
        _ => {
            resolved.insert(id.to_string(), policy.clone());
            return Ok(policy.clone());
        }
    };

    stack.push(id.to_string());
    let mut merged = policy.clone();
    for parent_id in parent_ids {
        let parent = resolve_one(parent_id, by_id, resolved, stack)?;
        merged.permissions.extend(parent.permissions.iter().cloned());
        merged.prohibitions.extend(parent.prohibitions.iter().cloned());
        merged.obligations.extend(parent.obligations.iter().cloned());
        if merged.assigner.is_empty() {
            merged.assigner = parent.assigner.clone();
        }
        if merged.assignee.is_none() {
            merged.assignee = parent.assignee.clone();
        }
    }
    stack.pop();

    resolved.insert(id.to_string(), merged.clone());
    Ok(merged)
}

struct Evaluation<'a> {
    policy: &'a WirePolicy,
    outcome: DecisionOutcome,
}

/// Section 5.2/7's multi-policy combining rule, chosen and documented here
/// since the case study leaves it formally undefined: **deny-override
/// across the whole policy set**, with an unrecognized action treated as
/// even stricter than an ordinary deny (Section 4.4's own fail-closed
/// posture for `Decision::Error` extended from one policy to the set) —
/// so precedence across `req.policies` is `Error` > `Deny` > `Allow`. The
/// first policy (in array order) carrying the overriding outcome is the
/// one `reason` reports on; this mirrors `decide`'s own within-policy
/// precedence (a matching prohibition beats a matching permission) at the
/// next level up, rather than inventing a different rule for policies than
/// for rules.
///
/// An **empty `policies` array is a default deny**, not the vacuous-Allow
/// exception Section 4.3 carves out for one policy's empty permissions
/// list: that exception is scoped to a policy which exists but grants
/// unconditionally, not to a request that names no policy at all — nothing
/// in the request authorizes anything, so this treats the empty set as
/// closed. It is the one case with no per-policy `reason` to surface, so
/// it constructs its own.
///
/// **`odrl:inheritFrom` resolves before any of that, even before
/// party-role scoping**: each policy declaring a parent (by `id`, within
/// this same request) replicates that parent's rules and unset party
/// fields into itself first, so party-role scoping and `decide` both see
/// the same, already-merged policy set `req.policies` describes — not the
/// pre-inheritance one. A parent naming no `inheritFrom` of its own is
/// unaffected either way. See `resolve_inherit_from` for exactly what is
/// and is not replicated, and how a circular chain (or a parent `id` this
/// request does not contain) fails the whole request as a `Decision::Error`
/// before a single policy is decided.
///
/// **Party-role scoping runs before everything past that**, when the
/// config asks for it (`ResolvedConfig::party_identity_claim`, off by
/// default): a policy whose `odrl:assignee` does not name the caller is
/// removed from the set entirely — not evaluated and found wanting,
/// *absent*. So it contributes neither a grant nor a deny nor a
/// `Decision::Error`, and the combining rule above simply never sees it.
/// When that leaves no policy at all, the answer is the same default deny
/// an empty `policies` array gets, under a `reason` that names the
/// mismatch rather than reporting a constraint miss that never happened.
/// See `party_role_mismatch`.
pub fn evaluate_request(req: &Request) -> Response {
    evaluate_request_for_action(req, &req.action)
}

/// `evaluate_request` with the requested action supplied separately rather
/// than read from `req.action`. Private, and introduced solely so
/// `performable_actions_for_request` below can ask the same question about
/// each declared action in turn **without cloning the request** once per
/// action (a policy set in this corpus can carry hundreds of rules) and,
/// more importantly, without a second copy of Section 5.2's multi-policy
/// combining rule existing anywhere. `evaluate_request` is this called with
/// `req.action`, and is otherwise unchanged in every observable respect.
fn evaluate_request_for_action(req: &Request, requested_action: &str) -> Response {
    let config = ResolvedConfig::from(&req.config);

    if req.policies.is_empty() {
        return Response {
            dataset_id: req.dataset_id.clone(),
            decision: WireDecision::Deny,
            reason: "no policies in the request: an empty policy set is a default deny, not the \
                     open exception Section 4.3 grants a single policy's empty permissions list"
                .to_string(),
            duties: Vec::new(),
        };
    }

    // `odrl:inheritFrom`, ahead of everything else, including party-role
    // scoping below: see this function's own doc comment and
    // `resolve_inherit_from`. A circular chain, or a parent `id` absent
    // from this same request, is a caller configuration gap rather than a
    // decidable request, so it fails the whole request here rather than
    // being reported as any one policy's own outcome.
    let policies: Vec<WirePolicy> = match resolve_inherit_from(&req.policies) {
        Ok(policies) => policies,
        Err(reason) => {
            return Response {
                dataset_id: req.dataset_id.clone(),
                decision: WireDecision::Error,
                reason,
                duties: Vec::new(),
            };
        }
    };

    // Party-role scoping, ahead of everything past inheritance: a policy
    // this caller is not the `odrl:assignee` of is **absent from the
    // request**, not a policy that happens to grant nothing. The
    // distinction is the whole decision (see this function's own doc
    // comment above) — it is why the prohibition of a policy addressed to
    // somebody else cannot deny this caller, why an unrecognized action
    // inside one is not this caller's configuration gap, and why
    // `behaviour: "open"` does not turn one into a vacuous Allow. Off
    // unless `config.party_identity_claim` names a claim key, so `skipped`
    // is empty for every request built before this existed and this whole
    // block is a no-op.
    let (applicable, skipped): (Vec<&WirePolicy>, Vec<PartyRoleMismatch>) = {
        let mut applicable = Vec::with_capacity(policies.len());
        let mut skipped = Vec::new();
        for policy in &policies {
            match party_role_mismatch(policy, &req.claims, &config) {
                Some(mismatch) => skipped.push(mismatch),
                None => applicable.push(policy),
            }
        }
        (applicable, skipped)
    };

    if applicable.is_empty() {
        // Every policy in the request is addressed to somebody else. The
        // set this caller is actually being evaluated against is empty,
        // which is the same default deny an empty `policies` array is —
        // but it needs its own trace, because "no policy applies to you" and
        // "no permission matched" are entirely different things for a host
        // to act on, and reporting the second for the first would send a
        // debugging host looking at its constraints and claims rather than
        // at who the policy names.
        return Response {
            dataset_id: req.dataset_id.clone(),
            decision: WireDecision::Deny,
            reason: format!(
                "no policy in the request applies to this caller: {}",
                skipped.iter().map(PartyRoleMismatch::describe).collect::<Vec<_>>().join("; ")
            ),
            duties: Vec::new(),
        };
    }

    let evaluations: Vec<Evaluation> = applicable
        .into_iter()
        .map(|policy| Evaluation {
            policy,
            // `req.dataset_id` is this request's `odrl:target` (see
            // `Request`'s own doc comment) — the asset each rule's own
            // `odrl:target`, if it has one, is compared against.
            // `req.asset_collections` names every `odrl:AssetCollection`
            // the host asserts `dataset_id` is `odrl:partOf`, so a rule
            // scoped to a collection IRI is in play for a member too.
            outcome: decide(
                &policy.as_decision_policy(),
                &req.claims,
                &config,
                requested_action,
                &req.dataset_id,
                &req.asset_collections,
            ),
        })
        .collect();

    let deciding = evaluations
        .iter()
        .find(|e| matches!(e.outcome.decision, Decision::Error(_)))
        .or_else(|| evaluations.iter().find(|e| e.outcome.decision == Decision::Deny))
        .unwrap_or(&evaluations[0]);

    let wire_decision = match deciding.outcome.decision {
        Decision::Allow => WireDecision::Allow,
        Decision::Deny => WireDecision::Deny,
        Decision::Error(_) => WireDecision::Error,
    };

    let reason = describe_reason(
        deciding.policy,
        &deciding.outcome,
        &req.claims,
        requested_action,
        &req.dataset_id,
        &req.asset_collections,
        &config,
    );

    // Under `duty_mode: deny`, a policy-level obligation's unresolved state
    // is exactly what the `Deny` decision already says, so listing it again
    // is noise — that suppression is Section 5.2's and is unchanged. The
    // reasoning does not transfer to the two narrower attachment points:
    // an unresolved per-permission duty removes one permission from
    // consideration (the request may still be allowed by another), and an
    // unresolved remedy never drove the decision at all, so in both cases
    // the response would otherwise carry no trace of an obligation the host
    // really does have. A policy set with neither — every fixture in this
    // workspace — emits exactly the empty list it always did.
    let duties = if matches!(deciding.outcome.decision, Decision::Error(_)) {
        Vec::new()
    } else {
        let suppress_obligations = config.duty_mode == DutyMode::Deny;
        evaluations
            .iter()
            .flat_map(|e| {
                e.outcome
                    .unresolved_duties
                    .iter()
                    .filter(move |duty| !(suppress_obligations && duty.is_plain_policy_obligation()))
                    .map(move |duty| DutyEntry {
                        policy_id: e.policy.id.clone(),
                        action: duty.action.clone(),
                        resolved: false,
                        // Omitted for Section 4.5's original shape — a
                        // policy-level obligation outstanding in its own
                        // right — so every entry this engine emitted
                        // before nested duties existed is exactly the
                        // three fields it always was.
                        source: (!duty.is_plain_policy_obligation()).then(|| duty.path()),
                    })
            })
            .collect()
    };

    Response {
        dataset_id: req.dataset_id.clone(),
        decision: wire_decision,
        reason,
        duties,
    }
}

/// Every claim-map key the policies in `req` could actually test, sorted
/// and deduplicated across the whole request —
/// `decision::referenced_left_operands` (see it for the exact semantics
/// and their limits) asked at the wire level, off the `Request` a host
/// already has in hand.
///
/// **Why this belongs here and not only on the engine crate's own types.**
/// A host does not hold a `decision::Policy`: that type is this engine's
/// internal reduction of a policy to what the decision algorithm consumes,
/// and `WirePolicy` — with its `id`/`kind`/`assigner`/`assignee` — is what
/// actually crosses the boundary. Making a host convert one to the other
/// just to ask which claims to gather would push this module's own
/// mapping job onto the caller. `evaluate_request` is the other pure
/// function over a `&Request` in this module, and this sits beside it
/// deliberately: same input type, no state, no side effects.
///
/// **Deliberately answered off `policies` alone, ignoring `claims`.** The
/// caller asking this question is, by construction, the one still deciding
/// what to put in `claims` — so a `Request` built for this call can carry
/// an empty claims map (the field is `#[serde(default)]`) and get the same
/// answer it would once populated. This also keeps the call honest about
/// what it is: a statement about the policies, not a diff against
/// whatever the host happens to have gathered so far.
///
/// **Not part of the JSON wire contract, and not a fifth WASM export.**
/// Section 5.2's request/response shapes are untouched by this: it is a
/// native Rust entry point over the same `Request` type, callable by
/// `compliance-runner`-style in-process hosts. A `wasm32` guest reaching
/// it would need a new `extern "C"` export alongside `evaluate` in
/// `crate::abi` — an additive but real change to the four-export ABI that
/// Section 5.1, this repo's README and `site/`'s own bridge each state as
/// a fixed contract, which is a separate decision rather than a side
/// effect of adding this function.
pub fn left_operands_for_request(req: &Request) -> Vec<String> {
    // Converts through `as_decision_policy` — the same conversion
    // `evaluate_request` above already performs per policy per request —
    // rather than re-walking `permissions`/`prohibitions`/`obligations`
    // here. Re-walking would duplicate, in a second place, the knowledge
    // of *which* rule lists a policy has, which is exactly the kind of
    // thing that silently drifts: a walk that missed one list would
    // under-report claim keys with no symptom at the call site.
    let policies: Vec<Policy> = req.policies.iter().map(WirePolicy::as_decision_policy).collect();
    crate::decision::referenced_left_operands(&policies)
}

/// Which of the actions `req.config` declares this caller could actually
/// perform against the request's **whole policy set** — sorted and
/// deduplicated. `decision::performable_actions` (see it for what the
/// answer does and does not mean) asked at the wire level, off the
/// `Request` a host already holds.
///
/// **Why this is a loop over `evaluate_request` and not over
/// `decision::performable_actions`.** A request carries several policies,
/// and combining their decisions is Section 5.2's own rule — deny-override
/// across the set, `Error` > `Deny` > `Allow` — chosen and documented in
/// this module because the case study leaves N-policy combining formally
/// undefined. Enumerating per policy and unioning the results would
/// contradict it in the one case that matters: an action one policy permits
/// and another prohibits would be reported as performable. So this asks the
/// same combined question `evaluate_request` answers, once per declared
/// action, and keeps the `Allow`s. There is exactly one combining rule in
/// this crate and this call does not add a second.
///
/// **`req.action` is ignored**, deliberately: that field is the one action
/// `evaluate_request` is about, and this call is about all of them. A
/// request built for this can leave it at anything — including an action
/// the config does not declare — and get the same answer, the same way
/// `left_operands_for_request` above ignores `claims`.
///
/// **Not part of the JSON wire contract, and not a fifth WASM export** —
/// the same boundary `left_operands_for_request` documents above, for the
/// same reason: Section 5.2's request/response shapes are untouched, and
/// the wasm32 guest still exposes exactly the four exports of `crate::abi`.
/// A guest-side caller would need a new `extern "C"` export, which is an
/// additive but real change to an ABI stated as fixed in three places, and
/// so is left as its own decision. The consequence worth stating plainly:
/// `site/`'s Demonstrator page cannot surface this today.
///
/// **Cost.** One full policy-set evaluation per declared action. That is
/// linear in the vocabulary, which for the W3C ODRL 2.2 Common Vocabulary
/// (`profile-interpreter/examples/odrl-2.2-common-actions.ttl`) is ~51
/// actions — cheap for one dataset, and worth a host's notice before
/// calling it once per dataset across a large catalog, since the natural
/// catalog-filtering use is exactly that loop.
pub fn performable_actions_for_request(req: &Request) -> Vec<String> {
    let mut allowed = std::collections::BTreeSet::new();
    for declared in &req.config.actions {
        if evaluate_request_for_action(req, &declared.id).decision == WireDecision::Allow {
            allowed.insert(declared.id.clone());
        }
    }
    allowed.into_iter().collect()
}

/// The response this engine returns when the request bytes handed to
/// `crate::abi::evaluate` (or any other host boundary) do not even parse
/// as `Request` JSON. Not part of Section 5.2's documented shape — that
/// section only specifies the response to a well-formed request — but the
/// four-export ABI (Section 5.1) has no separate error channel, so a
/// malformed request must still produce *a* JSON `Response` rather than a
/// guest trap. `dataset_id` is empty because a request that failed to
/// parse has no reliable `dataset_id` to echo back.
pub fn parse_error_response(err: &serde_json::Error) -> Response {
    Response {
        dataset_id: String::new(),
        decision: WireDecision::Error,
        reason: format!("request did not parse as the documented Section 5.2 JSON shape: {err}"),
        duties: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::ClaimValue;
    use crate::decision::MAX_CONSEQUENCE_DEPTH;

    const ALLOW_EXAMPLE: &str = r#"{
      "dataset_id": "urn:uuid:example-dataset-1",
      "action": "use",
      "config": {
        "@type": "odrl:Profile",
        "@id": "https://example.org/profiles/default",
        "odrl:action": [
          {"@id": "use"},
          {"@id": "distribute", "odrl:includedIn": {"@id": "use"}},
          {"@id": "notify"}
        ],
        "dutyMode": "advise"
      },
      "policies": [
        {
          "id": "policy-1",
          "kind": "Offer",
          "assigner": "did:web:provider.example",
          "assignee": null,
          "permissions": [
            {
              "action": "use",
              "constraints": [
                { "left_operand": "nationality", "operator": "eq", "right_operand": "DE" }
              ]
            }
          ],
          "prohibitions": [],
          "obligations": [
            { "action": "notify", "constraints": [] }
          ]
        }
      ],
      "claims": {
        "sub": "user-42",
        "nationality": "DE",
        "scope": ["catalog:read", "sparql:read"]
      }
    }"#;

    #[test]
    fn section_5_2_allow_example_deserializes_and_evaluates_exactly_as_documented() {
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        assert_eq!(req.dataset_id, "urn:uuid:example-dataset-1");
        assert_eq!(req.action, "use");
        assert_eq!(req.policies.len(), 1);
        assert_eq!(req.policies[0].assignee, None);
        assert_eq!(
            req.claims.get("nationality"),
            Some(&ClaimValue::Single("DE".to_string()))
        );

        let response = evaluate_request(&req);
        assert_eq!(response.dataset_id, "urn:uuid:example-dataset-1");
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-1' matched: action 'use': nationality eq DE"
        );
        assert_eq!(
            response.duties,
            vec![DutyEntry {
                policy_id: "policy-1".to_string(),
                action: "notify".to_string(),
                resolved: false,
                source: None,
            }]
        );
    }

    #[test]
    fn section_5_2_response_serializes_to_the_documented_shape() {
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        let response = evaluate_request(&req);
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["decision"], "Allow");
        assert_eq!(value["duties"][0]["policy_id"], "policy-1");
        assert_eq!(value["duties"][0]["resolved"], false);
    }

    #[test]
    fn config_serializes_to_the_documented_odrl_json_ld_shape() {
        let config = RequestConfig {
            type_: "odrl:Profile".to_string(),
            id: "https://example.org/profiles/default".to_string(),
            actions: vec![
                WireActionDecl { id: "use".to_string(), included_in: None },
                WireActionDecl {
                    id: "sell".to_string(),
                    included_in: Some(WireNodeRef { id: "transfer".to_string() }),
                },
            ],
            duty_mode: DutyMode::Advise,
            behaviour: Behaviour::Closed,
            party_identity_claim: None,
        };
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["@type"], "odrl:Profile");
        assert_eq!(value["@id"], "https://example.org/profiles/default");
        assert_eq!(value["odrl:action"][0]["@id"], "use");
        assert_eq!(value["odrl:action"][1]["odrl:includedIn"]["@id"], "transfer");
        assert_eq!(value["dutyMode"], "advise");
        assert_eq!(value["behaviour"], "closed");
        assert!(
            value["odrl:action"][0].get("odrl:includedIn").is_none(),
            "an action with no parent must not serialize a null odrl:includedIn"
        );
    }

    #[test]
    fn config_missing_behaviour_deserializes_defaulting_to_open() {
        let json = r#"{
            "@type": "odrl:Profile",
            "@id": "https://example.org/profiles/default",
            "odrl:action": [{"@id": "use"}],
            "dutyMode": "advise"
        }"#;
        let config: RequestConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.behaviour,
            Behaviour::Open,
            "a request built against an earlier revision of this wire contract, with no \
             behaviour field at all, must still deserialize and behave exactly as before"
        );
    }

    fn action(id: &str) -> WireActionDecl {
        WireActionDecl { id: id.to_string(), included_in: None }
    }

    fn deny_config(actions: &[&str]) -> RequestConfig {
        RequestConfig {
            type_: "odrl:Profile".to_string(),
            id: "https://example.org/profiles/test".to_string(),
            actions: actions.iter().map(|a| action(a)).collect(),
            duty_mode: DutyMode::Advise,
            behaviour: Behaviour::Open,
            party_identity_claim: None,
        }
    }

    #[test]
    fn a_matching_prohibition_denies_and_names_itself_in_the_reason() {
        // The policy declares `odrl:conflict: prohibit` because it really
        // does contain a conflict -- an unconstrained `use` permission and
        // a `use` prohibition this caller matches -- and this test is about
        // deny-overrides naming the prohibition, not about what an
        // unreconciled conflict means. Before `odrl:conflict` existed the
        // engine applied prohibition-first here unconditionally and this
        // fixture declared nothing; an undeclared strategy now means
        // ODRL's own default, `invalid`, which voids the policy instead
        // (`a_policy_declaring_no_conflict_strategy_is_void_...` below is
        // that case). Declaring the strategy keeps the fixture asking the
        // question it was written to ask.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use", "notify"]),
            policies: vec![WirePolicy {
                id: "policy-2".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![Rule::new("use", vec![])],
                prohibitions: vec![Rule::new(
                    "use",
                    vec![crate::constraint::Constraint::new(
                        "nationality",
                        Operator::Eq,
                        "US",
                    )],
                )],
                obligations: vec![],
                conflict: ConflictStrategy::Prohibit,
                inherit_from: None,
            }],
            claims: [("nationality".to_string(), ClaimValue::Single("US".to_string()))]
                .into_iter()
                .collect(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-2' matched: action 'use': nationality eq US; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the prohibition's favour"
        );
        assert!(response.duties.is_empty());
    }

    #[test]
    fn a_matching_prohibition_with_a_nested_and_constraint_renders_it_sensibly_in_the_reason() {
        // End-to-end proof that a native logical constraint (built here
        // with `Constraint::and`, not compliance-runner's host-side DNF
        // adapter) flows all the way through `evaluate_request` -- both
        // the decision itself and the human-readable `reason` trace, which
        // must render the nested shape legibly rather than garbled or
        // panicking (`describe_constraint` in this module).
        //
        // No permission: this fixture is about how a prohibition's
        // constraint renders, and an unconstrained `use` permission beside
        // it would make the policy a genuine `odrl:conflict` collision --
        // a different subject entirely, with its own tests below, and one
        // whose trace would bury the constraint this test exists to read.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![WirePolicy {
                id: "policy-nested".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![],
                prohibitions: vec![Rule::new(
                    "use",
                    vec![crate::constraint::Constraint::and(vec![
                        crate::constraint::Constraint::new("nationality", Operator::Eq, "US"),
                        crate::constraint::Constraint::new("scope", Operator::IsAnyOf, "embargoed"),
                    ])],
                )],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
                inherit_from: None,
            }],
            claims: [
                ("nationality".to_string(), ClaimValue::Single("US".to_string())),
                ("scope".to_string(), ClaimValue::Single("embargoed".to_string())),
            ]
            .into_iter()
            .collect(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-nested' matched: action 'use': \
             (nationality eq US && scope isAnyOf embargoed)"
        );
    }

    #[test]
    fn a_permission_for_a_broader_action_covers_the_requested_specific_one_and_says_so_in_the_reason() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "sell".to_string(),
            config: RequestConfig {
                type_: "odrl:Profile".to_string(),
                id: "https://example.org/profiles/test".to_string(),
                actions: vec![
                    action("transfer"),
                    WireActionDecl { id: "sell".to_string(), included_in: Some(WireNodeRef { id: "transfer".to_string() }) },
                ],
                duty_mode: DutyMode::Advise,
                behaviour: Behaviour::Open,
                party_identity_claim: None,
            },
            policies: vec![WirePolicy {
                id: "policy-transfer".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![Rule::new("transfer", vec![])],
                prohibitions: vec![],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
                inherit_from: None,
            }],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-transfer' matched: action 'transfer' covers requested 'sell', unconstrained"
        );
    }

    #[test]
    fn an_unrecognized_action_yields_error_and_is_not_downgraded_by_another_allowed_policy() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![
                WirePolicy {
                    id: "policy-ok".to_string(),
                    kind: "Offer".to_string(),
                    assigner: "did:web:provider.example".to_string(),
                    assignee: None,
                    permissions: vec![Rule::new("use", vec![])],
                    prohibitions: vec![],
                    obligations: vec![],
                    conflict: ConflictStrategy::default(),
                    inherit_from: None,
                },
                WirePolicy {
                    id: "policy-bad".to_string(),
                    kind: "Offer".to_string(),
                    assigner: "did:web:provider.example".to_string(),
                    assignee: None,
                    permissions: vec![Rule::new("anonymize", vec![])],
                    prohibitions: vec![],
                    obligations: vec![],
                    conflict: ConflictStrategy::default(),
                    inherit_from: None,
                },
            ],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Error,
            "Error out-ranks Deny and Allow across the whole policy set (Section 4.4's \
             fail-closed posture extended to multi-policy combining)"
        );
        assert!(response.reason.contains("policy-bad"));
        assert!(response.reason.contains("anonymize"));
        assert!(response.duties.is_empty());
    }

    #[test]
    fn empty_policy_set_is_a_default_deny_not_the_single_policy_open_exception() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use", "notify"]),
            policies: vec![],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert!(response.duties.is_empty());
    }

    #[test]
    fn duty_mode_deny_forces_deny_and_suppresses_the_duties_list() {
        let mut config = deny_config(&["use", "notify"]);
        config.duty_mode = DutyMode::Deny;
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config,
            policies: vec![WirePolicy {
                id: "policy-3".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![Rule::new("use", vec![])],
                prohibitions: vec![],
                obligations: vec![Rule::new("notify", vec![])],
                conflict: ConflictStrategy::default(),
                inherit_from: None,
            }],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert!(response.reason.contains("policy-3"));
        assert!(
            response.duties.is_empty(),
            "Section 5.2: duties is empty whenever duty_mode: deny already forced the decision, \
             the information is already carried by decision itself"
        );
    }

    #[test]
    fn an_empty_permissions_list_under_closed_behaviour_traces_the_closed_default_not_a_non_answer() {
        // The regression guard for `describe_reason`'s own copy of the
        // permission-requirement rule: `decide` branches on
        // `config.behaviour`, and this trace has to branch the same way or
        // it reports "denied for a reason this trace could not
        // reconstruct" for the one decision `behaviour: "closed"` exists
        // to produce. The decision itself was always right; only the
        // human-readable reason was wrong.
        let mut config = deny_config(&["use"]);
        config.behaviour = Behaviour::Closed;
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config,
            policies: vec![WirePolicy {
                id: "policy-empty".to_string(),
                kind: "Set".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![],
                prohibitions: vec![],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
                inherit_from: None,
            }],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "no permission of policy 'policy-empty' covered and matched requested action 'use' (closed default)"
        );
    }

    #[test]
    fn an_empty_permissions_list_under_open_behaviour_still_traces_the_open_default() {
        // The other side of the same branch, so the fix above cannot
        // silently swap which arm is hardcoded.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![WirePolicy {
                id: "policy-empty".to_string(),
                kind: "Set".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![],
                prohibitions: vec![],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
                inherit_from: None,
            }],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(response.reason, "policy 'policy-empty' has no permissions (open default)");
    }

    #[test]
    fn left_operands_for_request_unions_every_policys_claim_keys_including_nested_ones() {
        // The host-facing question this helper answers: "given the request
        // I am about to evaluate, which claim keys must I actually gather?"
        // Answered off `policies` alone -- `claims` is exactly what the
        // caller is still assembling when it asks.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use", "notify"]),
            policies: vec![
                WirePolicy {
                    id: "policy-a".to_string(),
                    kind: "Offer".to_string(),
                    assigner: "did:web:provider.example".to_string(),
                    assignee: None,
                    permissions: vec![Rule::new(
                        "use",
                        vec![crate::constraint::Constraint::or(vec![
                            crate::constraint::Constraint::new("nationality", Operator::Eq, "DE"),
                            crate::constraint::Constraint::new("sub", Operator::Eq, "alice"),
                        ])],
                    )],
                    prohibitions: vec![Rule::new(
                        "use",
                        vec![crate::constraint::Constraint::new("embargo", Operator::Eq, "true")],
                    )],
                    obligations: vec![],
                    conflict: ConflictStrategy::default(),
                    inherit_from: None,
                },
                WirePolicy {
                    id: "policy-b".to_string(),
                    kind: "Offer".to_string(),
                    assigner: "did:web:provider.example".to_string(),
                    assignee: None,
                    permissions: vec![],
                    prohibitions: vec![],
                    obligations: vec![Rule::new(
                        "notify",
                        vec![crate::constraint::Constraint::new("sub", Operator::Eq, "alice")],
                    )],
                    conflict: ConflictStrategy::default(),
                    inherit_from: None,
                },
            ],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        assert_eq!(
            left_operands_for_request(&req),
            vec!["embargo".to_string(), "nationality".to_string(), "sub".to_string()],
            "sorted, deduped, across every policy in the request and into nested logical \
             constraints"
        );
    }

    #[test]
    fn left_operands_for_request_is_empty_for_a_request_with_no_policies() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        assert!(left_operands_for_request(&req).is_empty());
    }

    #[test]
    fn the_section_5_2_allow_example_reports_the_one_claim_key_it_actually_reads() {
        // The documented worked example carries three claims (`sub`,
        // `nationality`, `scope`) but only ever reads one of them. This is
        // the whole point of the call: a host pushing all three was
        // pushing two it never needed to.
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        assert_eq!(left_operands_for_request(&req), vec!["nationality".to_string()]);
    }

    // -- odrl:refinement on an action --------------------------------------

    /// A whole request whose one permission carries an action refinement
    /// (`print`, refined to "at most 2 copies") — the ODRL 2.2 shape this
    /// engine had no representation for at all before this phase. Built as
    /// literal JSON rather than Rust values on purpose: the wire is where
    /// the additive-change claim actually has to hold, and a request
    /// carrying this key must not be a parse error for a host that sends
    /// it.
    fn refined_print_request(count_claim: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "print",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "print"}}],
                "dutyMode": "advise"
              }},
              "policies": [
                {{
                  "id": "policy-refined",
                  "kind": "Offer",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [
                    {{
                      "action": "print",
                      "constraints": [],
                      "odrl:refinement": {{
                        "left_operand": "copies",
                        "operator": "lteq",
                        "right_operand": "2"
                      }}
                    }}
                  ],
                  "prohibitions": [],
                  "obligations": []
                }}
              ],
              "claims": {{ "copies": "{count_claim}" }}
            }}"#
        )
    }

    #[test]
    fn an_unsatisfied_action_refinement_denies_even_though_the_bare_action_matches() {
        let req: Request = serde_json::from_str(&refined_print_request("5")).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "the permission's action is `print` refined to at most 2 copies; a request to \
             print 5 is not the action this permission grants, even though the bare action \
             string matches"
        );
    }

    #[test]
    fn a_satisfied_action_refinement_matches_normally() {
        let req: Request = serde_json::from_str(&refined_print_request("2")).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
    }

    #[test]
    fn a_matched_action_refinement_is_visible_in_the_reason_trace() {
        // A refinement that contributed to a match must show up in the
        // trace: "this permission matched" reads very differently when the
        // action it matched was a narrowed one.
        let req: Request = serde_json::from_str(&refined_print_request("2")).unwrap();
        assert_eq!(
            evaluate_request(&req).reason,
            "permission[0] of policy 'policy-refined' matched: action 'print' \
             refined by [copies lteq 2], unconstrained"
        );
    }

    #[test]
    fn a_permission_excluded_only_by_its_action_refinement_says_so_in_the_reason() {
        // The non-match direction, and the one that would otherwise be
        // silently absorbed: without this branch the trace reads "no
        // permission covered and matched", which is true but hides that
        // the permission covered the action perfectly and failed purely on
        // its refinement.
        let req: Request = serde_json::from_str(&refined_print_request("5")).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-refined' covers requested action 'print' but its \
             action refinement was not satisfied: [copies lteq 2]"
        );
    }

    #[test]
    fn an_existing_fixture_rule_without_a_refinement_key_round_trips_unchanged() {
        // Copied verbatim out of `compliance/reports/latest-cases.json` —
        // the exact serialized rule shape the vendored compliance corpus
        // produces today, with no `odrl:refinement` key anywhere. It must
        // parse to an unrefined rule and serialize back to precisely the
        // same JSON: this addition is additive or it is nothing.
        const FIXTURE_RULE: &str = r#"{
          "action": "use",
          "constraints": [
            {
              "left_operand": "sub",
              "operator": "eq",
              "right_operand": "alice"
            }
          ]
        }"#;
        let rule: Rule = serde_json::from_str(FIXTURE_RULE).unwrap();
        assert_eq!(rule.action_refinement, None);
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::from_str::<serde_json::Value>(FIXTURE_RULE).unwrap(),
            "a rule with no refinement must not gain an `odrl:refinement` key on the way out"
        );
    }

    #[test]
    fn a_refined_rule_round_trips_through_the_odrl_refinement_key() {
        let rule = Rule::refined(
            "print",
            vec![],
            crate::constraint::Constraint::new("copies", Operator::Lteq, "2"),
        );
        let value = serde_json::to_value(&rule).unwrap();
        assert_eq!(value["odrl:refinement"]["left_operand"], "copies");
        assert_eq!(value["odrl:refinement"]["operator"], "lteq");
        assert_eq!(value["odrl:refinement"]["right_operand"], "2");
        assert_eq!(serde_json::from_value::<Rule>(value).unwrap(), rule);
    }

    #[test]
    fn a_malformed_refinement_object_is_a_parse_error_not_a_silently_ignored_one() {
        // `Constraint`'s hand-written `Deserialize` carries its strictness
        // into this position too: a refinement that is neither a complete
        // atomic constraint nor a logical group fails the whole request,
        // rather than degrading into something inert. For a prohibition's
        // refinement, "inert" would mean the prohibition applies to the
        // unrefined action — the fail-open direction.
        let malformed = r#"{ "action": "print", "constraints": [], "odrl:refinement": {} }"#;
        assert!(
            serde_json::from_str::<Rule>(malformed).is_err(),
            "an empty refinement object must not parse"
        );
        let mis_prefixed = r#"{ "action": "print", "constraints": [],
                               "odrl:refinement": { "left_operand": "copies", "operator": "lteq" } }"#;
        assert!(
            serde_json::from_str::<Rule>(mis_prefixed).is_err(),
            "a refinement missing `right_operand` must not parse"
        );
    }

    #[test]
    fn a_permission_failing_on_its_own_constraints_too_keeps_the_ordinary_no_permission_trace() {
        // The refinement-specific branch above is deliberately narrow: it
        // fires only when the refinement is the *sole* reason the rule did
        // not apply. A rule whose own constraints also miss is an ordinary
        // non-match and must keep reading as one, so the trace never
        // credits the refinement with a decision it did not solely drive.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "print".to_string(),
            config: deny_config(&["print"]),
            policies: vec![WirePolicy {
                id: "policy-both-miss".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![Rule::refined(
                    "print",
                    vec![crate::constraint::Constraint::new("sub", Operator::Eq, "alice")],
                    crate::constraint::Constraint::new("copies", Operator::Lteq, "2"),
                )],
                prohibitions: vec![],
                obligations: vec![],
                conflict: ConflictStrategy::default(),
                inherit_from: None,
            }],
            claims: [
                ("sub".to_string(), ClaimValue::Single("bob".to_string())),
                ("copies".to_string(), ClaimValue::Single("5".to_string())),
            ]
            .into_iter()
            .collect(),
            asset_collections: Vec::new(),
        };
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "no permission of policy 'policy-both-miss' covered and matched requested action \
             'print' (closed default)"
        );
    }

    // -- per-rule odrl:target ----------------------------------------------

    /// One policy carrying a permission on one asset and a prohibition on
    /// another — the "permission on asset A, prohibition on asset B" shape
    /// a real ODRL policy expresses through each rule's own `odrl:target`,
    /// and which this wire contract could not represent at all before this
    /// phase. `requested` is what the request itself is about, which in
    /// this contract is its `dataset_id`.
    ///
    /// Literal JSON rather than Rust values on purpose, exactly as the
    /// refinement fixtures above are: the wire is where the additive-change
    /// claim has to hold, and a host sending this key must not get a parse
    /// error.
    fn two_asset_request(requested: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "{requested}",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "use"}}],
                "dutyMode": "advise"
              }},
              "policies": [
                {{
                  "id": "policy-two-assets",
                  "kind": "Set",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [
                    {{ "action": "use", "odrl:target": "urn:asset:A", "constraints": [] }}
                  ],
                  "prohibitions": [
                    {{ "action": "use", "odrl:target": "urn:asset:B", "constraints": [] }}
                  ],
                  "obligations": []
                }}
              ],
              "claims": {{}}
            }}"#
        )
    }

    #[test]
    fn a_permission_on_one_asset_and_a_prohibition_on_another_are_evaluated_per_rule() {
        let for_a: Request = serde_json::from_str(&two_asset_request("urn:asset:A")).unwrap();
        let response = evaluate_request(&for_a);
        assert_eq!(
            response.decision,
            WireDecision::Allow,
            "the prohibition is about asset B; a request for asset A must not be denied by it"
        );
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-two-assets' matched: action 'use' on target \
             'urn:asset:A', unconstrained"
        );

        let for_b: Request = serde_json::from_str(&two_asset_request("urn:asset:B")).unwrap();
        let response = evaluate_request(&for_b);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "the same policy denies a request for asset B, which is what its prohibition is about"
        );
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-two-assets' matched: action 'use' on target \
             'urn:asset:B', unconstrained"
        );
    }

    #[test]
    fn a_permission_excluded_only_by_its_target_says_so_in_the_reason() {
        // The non-match direction, kept distinct from an action mismatch:
        // this permission covers the requested action perfectly and has no
        // constraints to miss on — it is simply about another asset, and
        // the trace has to say which of the two failed.
        let json = r#"{
          "dataset_id": "urn:asset:A",
          "action": "use",
          "config": {
            "@type": "odrl:Profile",
            "@id": "https://example.org/profiles/default",
            "odrl:action": [{"@id": "use"}],
            "dutyMode": "advise"
          },
          "policies": [
            {
              "id": "policy-elsewhere",
              "kind": "Set",
              "assigner": "did:web:provider.example",
              "assignee": null,
              "permissions": [
                { "action": "use", "odrl:target": "urn:asset:B", "constraints": [] }
              ],
              "prohibitions": [],
              "obligations": []
            }
          ],
          "claims": {}
        }"#;
        let req: Request = serde_json::from_str(json).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "a permission on asset B must not cover a request for asset A, even though the \
             action matches exactly"
        );
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-elsewhere' covers requested action 'use' but \
             targets 'urn:asset:B', not the requested 'urn:asset:A'"
        );
    }

    #[test]
    fn an_existing_fixture_rule_without_a_target_key_round_trips_unchanged() {
        // Copied verbatim out of `compliance/reports/latest-cases.json` —
        // the exact serialized rule shape the vendored compliance corpus
        // produces today, with no `odrl:target` key anywhere. It must parse
        // to an untargeted rule and serialize back to precisely the same
        // JSON: this addition is additive or it is nothing.
        const FIXTURE_RULE: &str = r#"{
          "action": "read",
          "constraints": []
        }"#;
        let rule: Rule = serde_json::from_str(FIXTURE_RULE).unwrap();
        assert_eq!(rule.target, None);
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::from_str::<serde_json::Value>(FIXTURE_RULE).unwrap(),
            "a rule with no target must not gain an `odrl:target` key on the way out"
        );
    }

    #[test]
    fn a_targeted_rule_round_trips_through_the_odrl_target_key() {
        let rule = Rule::targeting("use", "urn:asset:A", vec![]);
        let value = serde_json::to_value(&rule).unwrap();
        assert_eq!(value["odrl:target"], "urn:asset:A");
        assert_eq!(serde_json::from_value::<Rule>(value).unwrap(), rule);
    }

    #[test]
    fn a_request_whose_rules_carry_no_target_evaluates_exactly_as_before() {
        // The backward-compatibility case at the wire level: the documented
        // Section 5.2 example names no target anywhere, so every rule falls
        // back to being about whatever the request itself is about — the
        // implicit behaviour every fixture in this workspace relies on.
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-1' matched: action 'use': nationality eq DE",
            "an untargeted rule's trace must not gain a target clause"
        );
    }

    // -- odrl:AssetCollection membership (odrl:partOf) -----------------------

    /// A prohibition scoped to `urn:asset:collection-X`, and a request for
    /// `dataset_id`, optionally asserting the collection membership the
    /// host's own catalog resolved out of band. The exact distinguishing
    /// example from the backlog item this closes.
    fn collection_prohibition_request(dataset_id: &str, asset_collections: &[&str]) -> String {
        let collections = asset_collections
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            r#"{{
              "dataset_id": "{dataset_id}",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "use"}}],
                "dutyMode": "advise"
              }},
              "policies": [
                {{
                  "id": "policy-collection",
                  "kind": "Set",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [],
                  "prohibitions": [
                    {{ "action": "use", "odrl:target": "urn:asset:collection-X", "constraints": [] }}
                  ],
                  "obligations": []
                }}
              ],
              "claims": {{}},
              "asset_collections": [{collections}]
            }}"#
        )
    }

    #[test]
    fn a_prohibition_on_a_collection_denies_a_request_for_an_asserted_member_at_the_wire_level() {
        // Request A: the collection IRI itself — already correctly denied
        // today by plain string equality, unaffected by this addition.
        let for_collection: Request =
            serde_json::from_str(&collection_prohibition_request("urn:asset:collection-X", &[])).unwrap();
        assert_eq!(evaluate_request(&for_collection).decision, WireDecision::Deny);

        // Request B: a member of the collection, with the host asserting the
        // membership via `asset_collections` — the fail-open gap this field
        // closes. Before it existed, `target_applies` did bare string
        // equality against `dataset_id` alone and this request evaded the
        // prohibition entirely.
        let for_member: Request = serde_json::from_str(&collection_prohibition_request(
            "urn:asset:member-1",
            &["urn:asset:collection-X"],
        ))
        .unwrap();
        let response = evaluate_request(&for_member);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "a member of a prohibited collection must be denied once the host asserts the \
             membership through `asset_collections`"
        );
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-collection' matched: action 'use' on target \
             'urn:asset:collection-X', unconstrained"
        );
    }

    #[test]
    fn a_request_for_a_collection_member_with_no_asserted_membership_evades_the_prohibition() {
        // The control this addition is measured against: with no
        // `asset_collections` entry naming the collection, a request for
        // the member is simply about a different asset from the one the
        // prohibition names — the same behaviour this contract always had,
        // and still correct absent the host's own fact.
        let req: Request =
            serde_json::from_str(&collection_prohibition_request("urn:asset:member-1", &[])).unwrap();
        assert_eq!(evaluate_request(&req).decision, WireDecision::Allow);
    }

    #[test]
    fn a_request_with_no_asset_collections_key_round_trips_unchanged() {
        // Wire-additive: a request built before this field existed — every
        // fixture in the vendored compliance corpus, and the documented
        // Section 5.2 example — carries no `asset_collections` key at all,
        // must still deserialize, and must not gain the key on the way
        // back out.
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        assert!(req.asset_collections.is_empty());
        let value = serde_json::to_value(&req).unwrap();
        assert!(
            value.get("asset_collections").is_none(),
            "a request with no asserted collection membership must not gain an \
             `asset_collections` key on the way out"
        );
    }

    // -- odrl:inheritFrom (Policy Inheritance, Information Model §2.9) ------

    fn inheriting_policy(id: &str, assignee: Option<&str>, inherit_from: Option<&[&str]>) -> WirePolicy {
        WirePolicy {
            id: id.to_string(),
            kind: "Set".to_string(),
            assigner: "did:web:provider.example".to_string(),
            assignee: assignee.map(|a| a.to_string()),
            permissions: vec![],
            prohibitions: vec![],
            obligations: vec![],
            conflict: ConflictStrategy::default(),
            inherit_from: inherit_from.map(|ids| ids.iter().map(|id| id.to_string()).collect()),
        }
    }

    #[test]
    fn resolve_inherit_from_replicates_rules_and_unset_party_fields_but_not_conflict_or_kind() {
        // A whitebox test of the merge itself (this module's own
        // `use super::*` reaches a private fn), so the exact replication
        // rule is pinned down independent of the wire-level combining
        // subtlety the next test's own comment explains.
        let mut parent = inheriting_policy("parent", None, None);
        parent.assigner = "urn:parent-assigner".to_string();
        parent.assignee = Some("urn:parent-assignee".to_string());
        parent.permissions = vec![Rule::new("use", vec![])];
        parent.prohibitions = vec![Rule::new("distribute", vec![])];
        parent.conflict = ConflictStrategy::Perm;

        let mut child = inheriting_policy("child", None, Some(&["parent"]));
        child.kind = "Offer".to_string();
        child.assigner = String::new(); // unset -- helper's own default is non-empty
        child.permissions = vec![Rule::new("print", vec![])];

        let resolved = resolve_inherit_from(&[parent, child]).unwrap();
        let child = resolved.iter().find(|p| p.id == "child").unwrap();

        assert_eq!(
            child.permissions,
            vec![Rule::new("print", vec![]), Rule::new("use", vec![])],
            "the child's own permission stays first, the parent's is appended"
        );
        assert_eq!(child.prohibitions, vec![Rule::new("distribute", vec![])]);
        assert_eq!(
            child.assigner, "urn:parent-assigner",
            "assigner is unset on the child (the empty string), so the parent's is replicated"
        );
        assert_eq!(
            child.assignee,
            Some("urn:parent-assignee".to_string()),
            "assignee is unset on the child (None), so the parent's is replicated"
        );
        assert_eq!(
            child.kind, "Offer",
            "kind is the child's own declared class -- not in §2.9's replicated list, and not \
             overwritten by the parent's"
        );
        assert_eq!(
            child.conflict,
            ConflictStrategy::default(),
            "odrl:conflict is deliberately not replicated -- see resolve_inherit_from's own doc \
             comment for why a wire-level 'unset' is indistinguishable from an explicit default"
        );
    }

    #[test]
    fn a_child_declaring_its_own_assignee_keeps_it_rather_than_the_parents() {
        let parent = inheriting_policy("parent", Some("urn:parent-assignee"), None);
        let child = inheriting_policy("child", Some("urn:child-assignee"), Some(&["parent"]));

        let resolved = resolve_inherit_from(&[parent, child]).unwrap();
        let child = resolved.iter().find(|p| p.id == "child").unwrap();
        assert_eq!(child.assignee, Some("urn:child-assignee".to_string()));
    }

    /// The exact distinguishing example this backlog item cites, adapted to
    /// actually isolate the gap this wire-level contract can observe.
    ///
    /// **Why the isolation matters.** `evaluate_request`'s own multi-policy
    /// rule is deny-override *across the whole `policies` array*: if
    /// `parent` were simply a second, unscoped sibling of `child` here, its
    /// own unconstrained prohibition would independently deny the request
    /// by itself, with or without inheritance ever being resolved -- the
    /// two decisions would coincide and the test would prove nothing (this
    /// was checked empirically against the pre-fix engine before writing
    /// this test: the naive two-sibling shape already denied, for exactly
    /// that reason, not because inheritance worked). What actually isolates
    /// "did the child inherit the prohibition" is party-role scoping
    /// (opt-in, and orthogonal to inheritance): `parent` is addressed to
    /// somebody else and so drops out of `applicable` entirely, on its own
    /// well-established semantics (`party_role_mismatch`) -- it contributes
    /// neither a grant nor a deny. `child` names the caller as its own
    /// `odrl:assignee` (so it is unaffected by party-role scoping either
    /// way, and does not itself inherit `parent`'s mismatched assignee --
    /// see the previous test), declares no rules of its own, and inherits
    /// `parent`'s. `child` alone is then what `evaluate_request` actually
    /// decides on.
    #[test]
    fn a_child_declaring_no_rules_of_its_own_inherits_its_parents_prohibition_and_denies() {
        let mut config = deny_config(&["use"]);
        config.behaviour = Behaviour::Open; // the engine's own documented default
        config.party_identity_claim = Some("sub".to_string());

        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config,
            policies: vec![
                {
                    let mut p = inheriting_policy("parent", Some("did:web:someone-else.example"), None);
                    p.prohibitions = vec![Rule::new("use", vec![])];
                    p
                },
                inheriting_policy("child", Some("did:web:alice.example"), Some(&["parent"])),
            ],
            claims: [("sub".to_string(), ClaimValue::Single("did:web:alice.example".to_string()))]
                .into_iter()
                .collect(),
            asset_collections: Vec::new(),
        };

        // The control this test is measured against: without the fix,
        // `child` carries no rules at all once `parent`'s mismatched
        // assignee excludes it, and `Behaviour::Open`'s vacuous-permission
        // path grants -- confirmed against the pre-fix engine, reason
        // `"policy 'child' has no permissions (open default)"`.
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "a child that inherits nothing of its own must still be bound by the parent's \
             prohibition it declared odrl:inheritFrom for"
        );
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'child' matched: action 'use', unconstrained"
        );
    }

    #[test]
    fn direct_circular_inherit_from_is_rejected_as_an_error_not_looped_forever() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![
                inheriting_policy("a", None, Some(&["b"])),
                inheriting_policy("b", None, Some(&["a"])),
            ],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Error);
        assert!(
            response.reason.contains("circular odrl:inheritFrom"),
            "reason was: {}",
            response.reason
        );
    }

    #[test]
    fn inherit_from_naming_an_id_absent_from_this_request_is_an_error() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![inheriting_policy("child", None, Some(&["no-such-parent"]))],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Error);
        assert!(
            response.reason.contains("no-such-parent") && response.reason.contains("not the id of any policy"),
            "reason was: {}",
            response.reason
        );
    }

    #[test]
    fn inheritance_is_transitive_across_more_than_one_level() {
        // grandparent -> parent -> child, neither parent nor child
        // declaring any rule of its own: the grandparent's prohibition
        // must still reach child.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![
                {
                    let mut p = inheriting_policy("grandparent", None, None);
                    p.prohibitions = vec![Rule::new("use", vec![])];
                    p
                },
                inheriting_policy("parent", None, Some(&["grandparent"])),
                inheriting_policy("child", None, Some(&["parent"])),
            ],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
    }

    #[test]
    fn a_policy_naming_no_inherit_from_is_unaffected_and_round_trips_without_the_key() {
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        assert_eq!(req.policies[0].inherit_from, None);
        let value = serde_json::to_value(&req).unwrap();
        assert!(
            value["policies"][0].get("inheritFrom").is_none(),
            "a policy with no odrl:inheritFrom must not gain the key on the way back out"
        );
    }

    // -- performable_actions_for_request -----------------------------------

    fn wire_policy(id: &str, permissions: Vec<Rule>, prohibitions: Vec<Rule>) -> WirePolicy {
        WirePolicy {
            id: id.to_string(),
            kind: "Offer".to_string(),
            assigner: "did:web:provider.example".to_string(),
            assignee: None,
            permissions,
            prohibitions,
            obligations: vec![],
            conflict: ConflictStrategy::default(),
            inherit_from: None,
        }
    }

    #[test]
    fn the_section_5_2_allow_example_is_performable_for_use_and_the_action_included_in_it() {
        // The documented worked example asks one yes/no question about
        // `use`. This asks the catalog-filtering question instead: of the
        // three actions its config declares, which could this caller
        // actually perform? `distribute` is declared `odrl:includedIn use`,
        // so the `use` permission covers it; nothing covers `notify`, which
        // is only ever this policy's obligation.
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        assert_eq!(
            performable_actions_for_request(&req),
            vec!["distribute".to_string(), "use".to_string()]
        );
    }

    #[test]
    fn performable_actions_for_request_does_not_read_the_requests_own_action_field() {
        // The request's `action` is the one action `evaluate_request` is
        // about; this call is about all of them, and must answer
        // identically no matter which one the caller happened to leave in
        // that field -- including one the config does not declare at all.
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        let baseline = performable_actions_for_request(&req);
        assert!(
            !baseline.is_empty(),
            "the control this test needs: an all-empty baseline would make the loop below \
             pass against an implementation that returned nothing at all"
        );
        for action in ["use", "distribute", "notify", "anonymize", ""] {
            let mut variant = req.clone();
            variant.action = action.to_string();
            assert_eq!(performable_actions_for_request(&variant), baseline, "action field {action:?}");
        }
    }

    #[test]
    fn performable_actions_for_request_applies_deny_override_across_the_whole_policy_set() {
        // Section 5.2's own multi-policy combining rule, inherited rather
        // than re-derived: policy-a permits `read`, policy-b prohibits it.
        // A per-policy enumeration unioned afterwards would report `read`
        // as performable; the request-level answer must not.
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "read".to_string(),
            config: deny_config(&["read", "write"]),
            policies: vec![
                wire_policy("policy-a", vec![Rule::new("read", vec![]), Rule::new("write", vec![])], vec![]),
                wire_policy("policy-b", vec![Rule::new("read", vec![]), Rule::new("write", vec![])], vec![Rule::new("read", vec![])]),
            ],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        assert_eq!(
            performable_actions_for_request(&req),
            vec!["write".to_string()],
            "policy-b's prohibition on `read` overrides policy-a's permission for it across the set"
        );
    }

    #[test]
    fn performable_actions_for_request_is_empty_for_a_request_with_no_policies() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use", "notify"]),
            policies: vec![],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        assert!(
            performable_actions_for_request(&req).is_empty(),
            "an empty policy set is `evaluate_request`'s own default deny, for every action"
        );
    }

    #[test]
    fn performable_actions_for_request_agrees_with_evaluate_request_on_every_declared_action() {
        // The wire-level consistency property, mirroring the decision-level
        // one in decision.rs: this is a loop over `evaluate_request`, so
        // membership must be exactly `evaluate_request(..with that action..)
        // == Allow`, fixture by fixture.
        let deny_over_allow = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "read".to_string(),
            config: deny_config(&["read", "write"]),
            policies: vec![
                wire_policy("policy-a", vec![Rule::new("read", vec![])], vec![]),
                wire_policy("policy-b", vec![], vec![Rule::new("read", vec![])]),
            ],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        let unrecognized = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "read".to_string(),
            config: deny_config(&["read"]),
            policies: vec![wire_policy("policy-bad", vec![Rule::new("anonymize", vec![])], vec![])],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        let no_policies = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "read".to_string(),
            config: deny_config(&["read", "write"]),
            policies: vec![],
            claims: Claims::new(),
            asset_collections: Vec::new(),
        };
        let allow_example: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();

        for (label, req) in [
            ("deny over allow", &deny_over_allow),
            ("unrecognized action", &unrecognized),
            ("no policies", &no_policies),
            ("the Section 5.2 allow example", &allow_example),
        ] {
            let performable = performable_actions_for_request(req);
            for declared in &req.config.actions {
                let mut probe = req.clone();
                probe.action = declared.id.clone();
                let allowed = evaluate_request(&probe).decision == WireDecision::Allow;
                assert_eq!(
                    performable.iter().any(|a| a == &declared.id),
                    allowed,
                    "fixture {label:?}: disagreement about {:?} (performable list was {performable:?})",
                    declared.id
                );
            }
        }
    }

    #[test]
    fn evaluate_request_is_byte_for_byte_unchanged_by_the_enumeration_refactor() {
        // `evaluate_request` now delegates to a private
        // `evaluate_request_for_action`; its own answer for the request's
        // own action must be exactly what it always was, reason string
        // included.
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-1' matched: action 'use': nationality eq DE"
        );
        assert_eq!(response.duties.len(), 1);
    }

    #[test]
    fn malformed_request_json_produces_a_response_not_a_panic() {
        let err = serde_json::from_str::<Request>("{ not json").unwrap_err();
        let response = parse_error_response(&err);
        assert_eq!(response.decision, WireDecision::Error);
        assert_eq!(response.dataset_id, "");
    }

    // -- per-permission odrl:duty ------------------------------------------
    //
    // Every request below is authored as literal JSON rather than as Rust
    // values, on the same reasoning `refined_print_request` states: the
    // additive-wire claim has to hold at the wire, and a host sending these
    // keys must not get a parse error.

    /// A policy whose one `use` permission carries `permission_extra` (the
    /// raw JSON fragment for whatever new key is under test) and whose
    /// obligations are `obligations_json`, evaluated under `duty_mode` with
    /// `claims_json`.
    fn duty_request(duty_mode: &str, permission_extra: &str, obligations_json: &str, claims_json: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "use"}}, {{"@id": "compensate"}}, {{"@id": "notify"}},
                                {{"@id": "anonymize"}}, {{"@id": "delete"}}],
                "dutyMode": "{duty_mode}",
                "behaviour": "closed"
              }},
              "policies": [
                {{
                  "id": "policy-duty",
                  "kind": "Offer",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [
                    {{ "action": "use", "constraints": []{permission_extra} }}
                  ],
                  "prohibitions": [],
                  "obligations": {obligations_json}
                }}
              ],
              "claims": {claims_json}
            }}"#
        )
    }

    /// The `odrl:duty` fragment used throughout: one duty whose own
    /// constraint is the ordinary claims-map lookup this engine already
    /// uses everywhere else — the host asserts "the compensate duty is
    /// fulfilled" by supplying that claim, exactly as `compliance-runner`
    /// asserts it today out of a `report:DutyReport` SOTW fact.
    const COMPENSATE_DUTY: &str = r#", "odrl:duty": [
        { "action": "compensate", "constraints": [
            { "left_operand": "duty:compensate", "operator": "eq", "right_operand": "fulfilled" }
        ] }
    ]"#;

    #[test]
    fn an_unresolved_per_permission_duty_under_duty_mode_deny_stops_that_permission_granting() {
        let req: Request =
            serde_json::from_str(&duty_request("deny", COMPENSATE_DUTY, "[]", "{}")).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "the only permission is conditioned on a duty the claims do not assert fulfilled"
        );
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-duty' matched, but its odrl:duty[0] 'compensate' is \
             unresolved under duty_mode: deny"
        );
    }

    #[test]
    fn an_unresolved_per_permission_duty_under_advise_still_grants_and_is_reported() {
        let req: Request =
            serde_json::from_str(&duty_request("advise", COMPENSATE_DUTY, "[]", "{}")).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Allow,
            "advise never blocks: the duty is advisory, exactly as a policy-level obligation is"
        );
        assert_eq!(
            response.duties,
            vec![DutyEntry {
                policy_id: "policy-duty".to_string(),
                action: "compensate".to_string(),
                resolved: false,
                source: Some("permission[0].duty[0]".to_string()),
            }],
            "and it is reported distinctly from a policy-level obligation, which carries no source"
        );
    }

    #[test]
    fn a_per_permission_duty_asserted_fulfilled_by_the_claims_resolves() {
        let req: Request = serde_json::from_str(&duty_request(
            "deny",
            COMPENSATE_DUTY,
            "[]",
            r#"{ "duty:compensate": "fulfilled" }"#,
        ))
        .unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert!(response.duties.is_empty(), "{:?}", response.duties);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-duty' matched: action 'use', unconstrained; \
             odrl:duty[0] 'compensate' satisfied"
        );
    }

    #[test]
    fn an_unresolved_per_permission_duty_is_scoped_to_its_own_permission_not_the_whole_policy() {
        // The whole point of the per-permission attachment point: under
        // duty_mode deny an unresolved *policy-level* obligation denies
        // everything, but an unresolved duty attached to permission[0] must
        // only stop permission[0] from granting — a sibling permission with
        // no duty of its own still grants.
        let two_permissions = format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "use"}}, {{"@id": "compensate"}}],
                "dutyMode": "deny",
                "behaviour": "closed"
              }},
              "policies": [
                {{
                  "id": "policy-two-permissions",
                  "kind": "Offer",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [
                    {{ "action": "use", "constraints": []{COMPENSATE_DUTY} }},
                    {{ "action": "use", "constraints": [] }}
                  ],
                  "prohibitions": [],
                  "obligations": []
                }}
              ],
              "claims": {{}}
            }}"#
        );
        let req: Request = serde_json::from_str(&two_permissions).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Allow,
            "permission[1] carries no duty and grants on its own; permission[0]'s unresolved duty \
             is scoped to permission[0], not to the policy"
        );
    }

    #[test]
    fn under_duty_mode_deny_a_scoped_duty_is_still_reported_while_an_obligation_is_not() {
        // Section 5.2 empties `duties` under `duty_mode: deny` because,
        // for a policy-level obligation, the information is already
        // carried by the decision itself: the request was denied, and an
        // unresolved obligation is why. That reasoning does not transfer
        // to the narrower attachment points. Here the decision is *Allow*
        // — permission[1] grants — and permission[0] still carries an
        // outstanding duty the response would otherwise say nothing about,
        // under the very mode that treats duties most strictly.
        let two_permissions = format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "use"}}, {{"@id": "compensate"}}, {{"@id": "notify"}}],
                "dutyMode": "deny",
                "behaviour": "closed"
              }},
              "policies": [
                {{
                  "id": "policy-mixed",
                  "kind": "Offer",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [
                    {{ "action": "use", "constraints": []{COMPENSATE_DUTY} }},
                    {{ "action": "use", "constraints": [] }}
                  ],
                  "prohibitions": [],
                  "obligations": []
                }}
              ],
              "claims": {{}}
            }}"#
        );
        let req: Request = serde_json::from_str(&two_permissions).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.duties,
            vec![DutyEntry {
                policy_id: "policy-mixed".to_string(),
                action: "compensate".to_string(),
                resolved: false,
                source: Some("permission[0].duty[0]".to_string()),
            }],
            "a per-permission duty is not suppressed by duty_mode: deny — the decision does not \
             carry its state"
        );
    }

    #[test]
    fn a_duty_on_a_permission_that_never_applies_is_not_reported_at_all() {
        // A duty is a pre-condition *of that permission*. A permission the
        // request never reaches (wrong action here) imposes nothing, so
        // reporting its duty would send a host chasing an obligation it
        // does not have.
        let unreachable = r#"{
          "dataset_id": "urn:uuid:ds",
          "action": "use",
          "config": {
            "@type": "odrl:Profile",
            "@id": "https://example.org/profiles/default",
            "odrl:action": [{"@id": "use"}, {"@id": "print"}, {"@id": "compensate"}],
            "dutyMode": "advise",
            "behaviour": "open"
          },
          "policies": [
            {
              "id": "policy-unreachable-duty",
              "kind": "Offer",
              "assigner": "did:web:provider.example",
              "assignee": null,
              "permissions": [
                { "action": "print", "constraints": [],
                  "odrl:duty": [{ "action": "compensate", "constraints": [] }] }
              ],
              "prohibitions": [],
              "obligations": []
            }
          ],
          "claims": {}
        }"#;
        let req: Request = serde_json::from_str(unreachable).unwrap();
        let response = evaluate_request(&req);
        assert!(
            response.duties.is_empty(),
            "the duty belongs to a `print` permission and this is a `use` request: {:?}",
            response.duties
        );
    }

    #[test]
    fn a_per_permission_duty_round_trips_through_the_odrl_duty_key() {
        let json = r#"{ "action": "use", "constraints": [],
                        "odrl:duty": [{ "action": "compensate", "constraints": [] }] }"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.duty.len(), 1);
        assert_eq!(rule.duty[0].action, "compensate");
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
    }

    // -- odrl:consequence ---------------------------------------------------

    /// A policy-level obligation `notify` (asserted fulfilled by the claim
    /// `duty:notify`) whose `odrl:consequence` is `compensate` (asserted by
    /// `duty:compensate`).
    const NOTIFY_WITH_CONSEQUENCE: &str = r#"[
        {
          "action": "notify",
          "constraints": [
            { "left_operand": "duty:notify", "operator": "eq", "right_operand": "fulfilled" }
          ],
          "odrl:consequence": {
            "action": "compensate",
            "constraints": [
              { "left_operand": "duty:compensate", "operator": "eq", "right_operand": "fulfilled" }
            ]
          }
        }
    ]"#;

    #[test]
    fn a_consequence_duty_resolves_where_the_primary_duty_did_not() {
        // The construct's whole point: the primary duty is not fulfilled,
        // so ODRL says the consequence duty is what now applies — and it
        // *is* fulfilled, so nothing is outstanding and duty_mode: deny has
        // nothing to act on.
        let req: Request = serde_json::from_str(&duty_request(
            "deny",
            "",
            NOTIFY_WITH_CONSEQUENCE,
            r#"{ "duty:compensate": "fulfilled" }"#,
        ))
        .unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Allow,
            "the unfulfilled `notify` duty falls through to its consequence, which is fulfilled"
        );
        assert!(response.duties.is_empty(), "{:?}", response.duties);
    }

    #[test]
    fn a_consequence_duty_that_is_itself_unresolved_leaves_duty_mode_governing() {
        let advise: Request =
            serde_json::from_str(&duty_request("advise", "", NOTIFY_WITH_CONSEQUENCE, "{}")).unwrap();
        let advised = evaluate_request(&advise);
        assert_eq!(advised.decision, WireDecision::Allow);
        assert_eq!(
            advised.duties,
            vec![DutyEntry {
                policy_id: "policy-duty".to_string(),
                action: "compensate".to_string(),
                resolved: false,
                source: Some("duty[0].consequence".to_string()),
            }],
            "the outstanding obligation is the consequence, not the primary duty it replaced"
        );

        let deny: Request =
            serde_json::from_str(&duty_request("deny", "", NOTIFY_WITH_CONSEQUENCE, "{}")).unwrap();
        let denied = evaluate_request(&deny);
        assert_eq!(denied.decision, WireDecision::Deny);
        assert_eq!(
            denied.reason,
            "duty[0].consequence 'compensate' of policy 'policy-duty' is unresolved under \
             duty_mode: deny",
            "and the trace says a consequence drove it, distinctly from a plain obligation"
        );
    }

    #[test]
    fn a_fulfilled_primary_duty_never_reaches_its_consequence() {
        let req: Request = serde_json::from_str(&duty_request(
            "deny",
            "",
            NOTIFY_WITH_CONSEQUENCE,
            r#"{ "duty:notify": "fulfilled" }"#,
        ))
        .unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert!(
            response.duties.is_empty(),
            "the consequence applies on non-fulfilment only: {:?}",
            response.duties
        );
    }

    #[test]
    fn a_consequence_attached_to_a_per_permission_duty_is_evaluated_the_same_way() {
        // The two constructs compose: `odrl:consequence` is a property of a
        // Duty, and a per-permission duty is a Duty.
        let permission_extra = r#", "odrl:duty": [
            { "action": "compensate", "constraints": [
                { "left_operand": "duty:compensate", "operator": "eq", "right_operand": "fulfilled" }
              ],
              "odrl:consequence": {
                "action": "delete",
                "constraints": [
                  { "left_operand": "duty:delete", "operator": "eq", "right_operand": "fulfilled" }
                ]
              }
            }
        ]"#;
        let req: Request = serde_json::from_str(&duty_request(
            "deny",
            permission_extra,
            "[]",
            r#"{ "duty:delete": "fulfilled" }"#,
        ))
        .unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Allow,
            "the permission's duty is unfulfilled but its consequence is, so the permission grants"
        );
    }

    /// A policy-level obligation whose `odrl:consequence` chain is `hops`
    /// links long, where **only the deepest link** is asserted fulfilled by
    /// the claims. Every earlier link is unfulfilled, so resolving requires
    /// walking the whole chain.
    fn consequence_chain_request(hops: usize) -> String {
        let mut duty = r#"{ "action": "delete", "constraints": [
            { "left_operand": "duty:deepest", "operator": "eq", "right_operand": "fulfilled" } ] }"#
            .to_string();
        for _ in 0..hops {
            duty = format!(
                r#"{{ "action": "notify", "constraints": [
                    {{ "left_operand": "duty:never", "operator": "eq", "right_operand": "fulfilled" }}
                  ], "odrl:consequence": {duty} }}"#
            );
        }
        duty_request("deny", "", &format!("[{duty}]"), r#"{ "duty:deepest": "fulfilled" }"#)
    }

    #[test]
    fn a_consequence_chain_is_followed_up_to_max_consequence_depth_and_bounded_past_it() {
        // `MAX_CONSEQUENCE_DEPTH` hops are walked; one more is not, and the
        // duty stays unresolved rather than the walk recursing without a
        // bound. Unresolved is the safe direction — a duty this engine
        // declines to walk to must never come back reported as done.
        let at_bound: Request =
            serde_json::from_str(&consequence_chain_request(MAX_CONSEQUENCE_DEPTH)).unwrap();
        assert_eq!(
            evaluate_request(&at_bound).decision,
            WireDecision::Allow,
            "a chain exactly MAX_CONSEQUENCE_DEPTH hops deep is still walked to its end"
        );

        let past_bound: Request =
            serde_json::from_str(&consequence_chain_request(MAX_CONSEQUENCE_DEPTH + 1)).unwrap();
        assert_eq!(
            evaluate_request(&past_bound).decision,
            WireDecision::Deny,
            "one hop further is not walked, so the duty stays unresolved under duty_mode: deny"
        );
    }

    #[test]
    fn a_consequence_round_trips_through_the_odrl_consequence_key() {
        let json = r#"{ "action": "notify", "constraints": [],
                        "odrl:consequence": { "action": "compensate", "constraints": [] } }"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.consequence.as_ref().map(|c| c.action.as_str()), Some("compensate"));
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
    }

    // -- odrl:remedy --------------------------------------------------------

    /// A policy whose `use` prohibition matches (`nationality eq US`
    /// against a US claim) and carries an `odrl:remedy` duty `anonymize`,
    /// asserted fulfilled by the claim `duty:anonymize`.
    ///
    /// It declares `odrl:conflict: prohibit` because the unconstrained
    /// `use` permission beside that prohibition makes it a genuine
    /// collision, and these tests are about the remedy never lifting a
    /// prohibition that won — not about what an unreconciled conflict
    /// means. Without the declaration the policy would be void under
    /// ODRL's own default and the prohibition would never be the deciding
    /// rule to hang a remedy clause off at all.
    fn remedy_request(duty_mode: &str, claims_json: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/default",
                "odrl:action": [{{"@id": "use"}}, {{"@id": "anonymize"}}],
                "dutyMode": "{duty_mode}",
                "behaviour": "closed"
              }},
              "policies": [
                {{
                  "id": "policy-remedy",
                  "kind": "Offer",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "odrl:conflict": "prohibit",
                  "permissions": [{{ "action": "use", "constraints": [] }}],
                  "prohibitions": [
                    {{
                      "action": "use",
                      "constraints": [
                        {{ "left_operand": "nationality", "operator": "eq", "right_operand": "US" }}
                      ],
                      "odrl:remedy": [
                        {{ "action": "anonymize", "constraints": [
                            {{ "left_operand": "duty:anonymize", "operator": "eq",
                               "right_operand": "fulfilled" }}
                        ] }}
                      ]
                    }}
                  ],
                  "obligations": []
                }}
              ],
              "claims": {claims_json}
            }}"#
        )
    }

    #[test]
    fn a_violated_remedy_does_not_drop_the_prohibition_and_leaves_a_trace() {
        // The specific fail-open hazard this repo's README already names
        // for `odrl:remedy` — "a violated duty attached to a prohibition
        // would drop the prohibition, fail-open". An unresolved remedy must
        // deny exactly as the bare prohibition would, and must say so.
        let req: Request =
            serde_json::from_str(&remedy_request("advise", r#"{ "nationality": "US" }"#)).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "an unresolved remedy must never turn a matching prohibition into an Allow"
        );
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-remedy' matched: action 'use': nationality eq US; \
             its odrl:remedy[0] 'anonymize' is unresolved and does not lift the prohibition; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the prohibition's favour",
            "and the remedy must not vanish silently from the trace"
        );
        assert_eq!(
            response.duties,
            vec![DutyEntry {
                policy_id: "policy-remedy".to_string(),
                action: "anonymize".to_string(),
                resolved: false,
                source: Some("prohibition[0].remedy[0]".to_string()),
            }],
        );
    }

    #[test]
    fn a_satisfied_remedy_still_denies_because_a_duty_never_loosens_a_decision_here() {
        // The documented sub-decision (see this crate's README, "Remedy"):
        // a remedy is reported, never enforced-away. Section 4.5's duties
        // only ever tighten a decision, and a claims-asserted remedy that
        // could erase a prohibition would be the first one in this engine
        // that loosens one.
        let req: Request = serde_json::from_str(&remedy_request(
            "advise",
            r#"{ "nationality": "US", "duty:anonymize": "fulfilled" }"#,
        ))
        .unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-remedy' matched: action 'use': nationality eq US; \
             its odrl:remedy[0] 'anonymize' is satisfied, which does not lift the prohibition; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the prohibition's favour"
        );
        assert!(
            response.duties.is_empty(),
            "a satisfied remedy is not outstanding: {:?}",
            response.duties
        );
    }

    #[test]
    fn a_remedy_on_a_prohibition_that_does_not_match_is_not_reported() {
        // A remedy is what must be done *on violation*. No violation, no
        // remedy — reporting it would invent an obligation.
        let req: Request =
            serde_json::from_str(&remedy_request("advise", r#"{ "nationality": "DE" }"#)).unwrap();
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Allow);
        assert!(response.duties.is_empty(), "{:?}", response.duties);
    }

    #[test]
    fn a_remedy_round_trips_through_the_odrl_remedy_key() {
        let json = r#"{ "action": "use", "constraints": [],
                        "odrl:remedy": [{ "action": "anonymize", "constraints": [] }] }"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.remedy.len(), 1);
        assert_eq!(rule.remedy[0].action, "anonymize");
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
    }

    // -- backward compatibility --------------------------------------------

    #[test]
    fn an_existing_policy_level_obligation_fixture_evaluates_byte_identically() {
        // The regression guard for this whole addition: a request carrying
        // no `odrl:duty`, no `odrl:consequence` and no `odrl:remedy`
        // anywhere — the shape every fixture in this workspace and in
        // `compliance/reports/latest-cases.json` actually has — must
        // produce exactly the response it produced before these three
        // fields existed, serialized JSON included (so a new response field
        // that failed to skip itself when absent would fail here too).
        let req: Request = serde_json::from_str(ALLOW_EXAMPLE).unwrap();
        assert_eq!(
            serde_json::to_string(&evaluate_request(&req)).unwrap(),
            r#"{"dataset_id":"urn:uuid:example-dataset-1","decision":"Allow","reason":"permission[0] of policy 'policy-1' matched: action 'use': nationality eq DE","duties":[{"policy_id":"policy-1","action":"notify","resolved":false}]}"#
        );
    }

    #[test]
    fn an_existing_fixture_rule_gains_no_duty_consequence_or_remedy_key() {
        // Copied verbatim out of `compliance/reports/latest-cases.json`, on
        // the same footing as the `odrl:refinement` and `odrl:target`
        // round-trip guards above.
        const FIXTURE_RULE: &str = r#"{
          "action": "use",
          "constraints": [
            {
              "left_operand": "sub",
              "operator": "eq",
              "right_operand": "alice"
            }
          ]
        }"#;
        let rule: Rule = serde_json::from_str(FIXTURE_RULE).unwrap();
        assert!(rule.duty.is_empty());
        assert!(rule.remedy.is_empty());
        assert_eq!(rule.consequence, None);
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::from_str::<serde_json::Value>(FIXTURE_RULE).unwrap(),
        );
        // And the constructor every existing call site uses builds the same
        // thing.
        let built = Rule::new("use", vec![crate::constraint::Constraint::new("sub", Operator::Eq, "alice")]);
        assert_eq!(built, rule);
    }

    // -----------------------------------------------------------------
    // Party-role (`odrl:assignee`) evaluation, opt-in via
    // `config.partyIdentityClaim`
    // -----------------------------------------------------------------

    /// One request whose only variables are the three this capability
    /// turns on: whether the config names an identity claim, what the
    /// policy's `odrl:assignee` says, and what the caller's claims map
    /// carries. Authored as JSON rather than as typed values so these
    /// tests state the *wire* contract — including that an engine which
    /// does not know `partyIdentityClaim` simply ignores the key.
    fn party_request(config_extra: &str, assignee_json: &str, claims_json: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/party",
                "odrl:action": [{{"@id": "use"}}],
                "dutyMode": "advise",
                "behaviour": "closed"{config_extra}
              }},
              "policies": [
                {{
                  "id": "policy-1",
                  "kind": "Agreement",
                  "assigner": "did:web:provider.example",
                  "assignee": {assignee_json},
                  "permissions": [{{"action": "use", "constraints": []}}],
                  "prohibitions": [],
                  "obligations": []
                }}
              ],
              "claims": {claims_json}
            }}"#
        )
    }

    const SUB_IS_THE_IDENTITY: &str = ",\n                \"partyIdentityClaim\": \"sub\"";
    const ALICE: &str = "\"did:web:alice.example\"";
    const MALLORY_CLAIMS: &str = "{\"sub\": \"did:web:mallory.example\"}";
    const ALICE_CLAIMS: &str = "{\"sub\": \"did:web:alice.example\"}";

    fn evaluate_text(text: &str) -> Response {
        evaluate_request(&serde_json::from_str::<Request>(text).unwrap())
    }

    #[test]
    fn party_role_evaluation_is_off_by_default_so_a_mismatched_assignee_still_grants() {
        // The regression guard for the whole capability: a request that
        // names no `partyIdentityClaim` — every request any existing host
        // sends, and every fixture in this workspace — evaluates exactly as
        // it did before party roles were evaluated at all, `odrl:assignee`
        // included.
        let response = evaluate_text(&party_request("", ALICE, MALLORY_CLAIMS));
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-1' matched: action 'use', unconstrained"
        );
        // And the answer is identical to the same request with no
        // `odrl:assignee` at all, which is what "the field is inert unless
        // you turn this on" actually means.
        let control = evaluate_text(&party_request("", "null", MALLORY_CLAIMS));
        assert_eq!(control.reason, response.reason);
        assert_eq!(control.decision, response.decision);
    }

    #[test]
    fn a_config_carrying_no_party_identity_claim_serializes_without_the_key() {
        let config: RequestConfig =
            serde_json::from_str::<Request>(ALLOW_EXAMPLE).unwrap().config;
        let value = serde_json::to_value(&config).unwrap();
        assert!(
            value.get("partyIdentityClaim").is_none(),
            "a config that never named an identity claim must not gain a null key on the wire"
        );
    }

    #[test]
    fn a_party_identity_claim_round_trips_through_the_wire_key() {
        let config: RequestConfig = serde_json::from_str::<Request>(&party_request(
            SUB_IS_THE_IDENTITY,
            ALICE,
            ALICE_CLAIMS,
        ))
        .unwrap()
        .config;
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(
            value.get("partyIdentityClaim").and_then(|v| v.as_str()),
            Some("sub"),
            "the configured identity claim must survive a parse/serialize round trip"
        );
    }

    #[test]
    fn a_configured_identity_claim_matching_the_assignee_leaves_the_policy_applying_normally() {
        let response = evaluate_text(&party_request(SUB_IS_THE_IDENTITY, ALICE, ALICE_CLAIMS));
        assert_eq!(response.decision, WireDecision::Allow);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-1' matched: action 'use', unconstrained",
            "a policy addressed to this very caller evaluates exactly as it would with the \
             capability switched off"
        );
    }

    #[test]
    fn a_policy_assigned_to_someone_else_grants_nothing_to_this_caller() {
        let response = evaluate_text(&party_request(SUB_IS_THE_IDENTITY, ALICE, MALLORY_CLAIMS));
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "no policy in the request applies to this caller: policy 'policy-1' names \
             odrl:assignee 'did:web:alice.example', which does not match the caller's 'sub' \
             claim (\"did:web:mallory.example\")"
        );
    }

    #[test]
    fn the_party_role_reason_is_distinct_from_an_ordinary_no_permission_trace() {
        let mismatch = evaluate_text(&party_request(SUB_IS_THE_IDENTITY, ALICE, MALLORY_CLAIMS));
        assert!(
            !mismatch.reason.contains("no permission of policy"),
            "a party-role skip must not be reported as an ordinary constraint miss: {}",
            mismatch.reason
        );
        assert!(mismatch.reason.contains("odrl:assignee"));
    }

    #[test]
    fn a_caller_whose_claims_lack_the_configured_identity_claim_never_matches_a_named_assignee() {
        // The defined behaviour for the missing-key case: a mismatch, not a
        // crash and not a silent bypass. `Constraint::evaluate`'s "an absent
        // claim key is a miss" rule, applied at the party position.
        let response = evaluate_text(&party_request(
            SUB_IS_THE_IDENTITY,
            ALICE,
            "{\"nationality\": \"DE\"}",
        ));
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "no policy in the request applies to this caller: policy 'policy-1' names \
             odrl:assignee 'did:web:alice.example', which does not match the caller's 'sub' \
             claim (absent from the claims map)"
        );
    }

    #[test]
    fn a_policy_naming_no_assignee_is_unaffected_by_party_role_evaluation() {
        // Decision 5: there is nothing to check a party role against, so the
        // common case in the vendored corpus behaves exactly as today whether
        // or not the capability is configured.
        let on = evaluate_text(&party_request(SUB_IS_THE_IDENTITY, "null", MALLORY_CLAIMS));
        let off = evaluate_text(&party_request("", "null", MALLORY_CLAIMS));
        assert_eq!(on.decision, WireDecision::Allow);
        assert_eq!(on.reason, off.reason);
    }

    #[test]
    fn a_multi_valued_identity_claim_matches_when_the_assignee_is_one_of_its_values() {
        let hit = evaluate_text(&party_request(
            SUB_IS_THE_IDENTITY,
            ALICE,
            "{\"sub\": [\"did:web:mallory.example\", \"did:web:alice.example\"]}",
        ));
        assert_eq!(hit.decision, WireDecision::Allow);

        let miss = evaluate_text(&party_request(
            SUB_IS_THE_IDENTITY,
            ALICE,
            "{\"sub\": [\"did:web:mallory.example\", \"did:web:bob.example\"]}",
        ));
        assert_eq!(miss.decision, WireDecision::Deny);
        assert!(
            miss.reason.contains("[\"did:web:mallory.example\",\"did:web:bob.example\"]"),
            "the trace must show what the caller actually presented: {}",
            miss.reason
        );
    }

    #[test]
    fn a_mismatched_policy_is_absent_from_the_request_not_merely_stripped_of_its_permissions() {
        // The interpretation decision, made observable. Were a non-matching
        // policy treated as "a policy with no rules", `behaviour: "open"`
        // would meet its permission requirement vacuously and *allow* a
        // caller the policy was never addressed to — the worst possible
        // reading. It is absent instead, and an empty policy set is a
        // default deny under either behaviour.
        let open = party_request(SUB_IS_THE_IDENTITY, ALICE, MALLORY_CLAIMS)
            .replace("\"behaviour\": \"closed\"", "\"behaviour\": \"open\"");
        let response = evaluate_text(&open);
        assert_eq!(response.decision, WireDecision::Deny);
        assert!(response.reason.contains("no policy in the request applies to this caller"));
    }

    /// Two policies, one addressed to a stranger: `p-forbid` prohibits
    /// `use` and is assigned to alice, `p-grant` permits `use` and is
    /// assigned to nobody. For a caller who is not alice the prohibition is
    /// not merely outvoted — it is not in the request at all, so the
    /// deny-override across the policy set never sees it.
    fn two_policy_request(config_extra: &str, claims_json: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/party",
                "odrl:action": [{{"@id": "use"}}],
                "dutyMode": "advise",
                "behaviour": "closed"{config_extra}
              }},
              "policies": [
                {{
                  "id": "p-forbid",
                  "kind": "Agreement",
                  "assigner": "did:web:provider.example",
                  "assignee": "did:web:alice.example",
                  "permissions": [],
                  "prohibitions": [{{"action": "use", "constraints": []}}],
                  "obligations": []
                }},
                {{
                  "id": "p-grant",
                  "kind": "Offer",
                  "assigner": "did:web:provider.example",
                  "assignee": null,
                  "permissions": [{{"action": "use", "constraints": []}}],
                  "prohibitions": [],
                  "obligations": []
                }}
              ],
              "claims": {claims_json}
            }}"#
        )
    }

    #[test]
    fn a_mismatched_policy_withholds_its_prohibition_as_well_as_its_permission() {
        let off = evaluate_text(&two_policy_request("", MALLORY_CLAIMS));
        assert_eq!(
            off.decision,
            WireDecision::Deny,
            "with the capability off, the stranger's prohibition still denies — today's behaviour"
        );

        let on = evaluate_text(&two_policy_request(SUB_IS_THE_IDENTITY, MALLORY_CLAIMS));
        assert_eq!(
            on.decision,
            WireDecision::Allow,
            "a policy that does not apply to this caller contributes nothing *either way*: its \
             prohibition is as absent as its permissions would have been"
        );
        assert_eq!(
            on.reason,
            "permission[0] of policy 'p-grant' matched: action 'use', unconstrained"
        );

        // And alice herself is still denied by the policy addressed to her.
        let alice = evaluate_text(&two_policy_request(SUB_IS_THE_IDENTITY, ALICE_CLAIMS));
        assert_eq!(alice.decision, WireDecision::Deny);
        assert_eq!(
            alice.reason,
            "prohibition[0] of policy 'p-forbid' matched: action 'use', unconstrained"
        );
    }

    #[test]
    fn a_mismatched_policys_unrecognized_action_is_not_this_callers_configuration_gap() {
        // A policy absent from the request cannot contribute a
        // `Decision::Error` either: Section 4.4's fail-closed posture is
        // about the policies actually being applied to this caller.
        let text = two_policy_request(SUB_IS_THE_IDENTITY, MALLORY_CLAIMS)
            .replace("\"prohibitions\": [{\"action\": \"use\"", "\"prohibitions\": [{\"action\": \"ex:undeclared\"");
        let response = evaluate_text(&text);
        assert_eq!(response.decision, WireDecision::Allow, "{}", response.reason);

        // The same request with the capability off is the control: there the
        // stranger's policy *is* in play, and its unrecognized action wins.
        let control_text = two_policy_request("", MALLORY_CLAIMS)
            .replace("\"prohibitions\": [{\"action\": \"use\"", "\"prohibitions\": [{\"action\": \"ex:undeclared\"");
        assert_eq!(evaluate_text(&control_text).decision, WireDecision::Error);
    }

    #[test]
    fn performable_actions_for_request_is_empty_for_a_caller_no_policy_applies_to() {
        let req: Request =
            serde_json::from_str(&party_request(SUB_IS_THE_IDENTITY, ALICE, MALLORY_CLAIMS)).unwrap();
        assert!(
            performable_actions_for_request(&req).is_empty(),
            "party-role scoping is inherited by the enumeration entry point, not re-implemented \
             or bypassed by it"
        );

        let addressed: Request =
            serde_json::from_str(&party_request(SUB_IS_THE_IDENTITY, ALICE, ALICE_CLAIMS)).unwrap();
        assert_eq!(performable_actions_for_request(&addressed), vec!["use".to_string()]);
    }

    #[test]
    fn left_operands_for_request_is_unchanged_by_party_role_scoping() {
        // Deliberate: this call is what a host uses to decide which claims
        // to gather in the first place, so it must not start depending on
        // the claims it is being asked about. A policy skipped for this
        // caller still reports the keys its rules read.
        let req: Request = serde_json::from_str(&two_policy_request(SUB_IS_THE_IDENTITY, MALLORY_CLAIMS)).unwrap();
        let with_constraint: Request = serde_json::from_str(
            &two_policy_request(SUB_IS_THE_IDENTITY, MALLORY_CLAIMS)
                .replace("\"prohibitions\": [{\"action\": \"use\", \"constraints\": []}]",
                         "\"prohibitions\": [{\"action\": \"use\", \"constraints\": [{\"left_operand\": \"nationality\", \"operator\": \"eq\", \"right_operand\": \"DE\"}]}]"),
        )
        .unwrap();
        assert!(left_operands_for_request(&req).is_empty());
        assert_eq!(left_operands_for_request(&with_constraint), vec!["nationality".to_string()]);
    }

    // -- odrl:conflict -----------------------------------------------------

    /// A request whose single policy carries a permission **and** a
    /// prohibition that both cover and match the *same* requested action on
    /// the *same* requested target — the only shape `odrl:conflict` has
    /// anything to say about, and (measured against
    /// `compliance/reports/latest-cases.json`) a shape no fixture of the
    /// vendored compliance corpus contains.
    ///
    /// `conflict_line` is spliced in as a raw JSON line so a test can send
    /// the key with any value at all, including one no `ConflictStrategy`
    /// variant spells, and the empty string for a policy that declares none.
    fn conflicting_request(conflict_line: &str) -> String {
        format!(
            r#"{{
              "dataset_id": "urn:uuid:ds",
              "action": "use",
              "config": {{
                "@type": "odrl:Profile",
                "@id": "https://example.org/profiles/test",
                "odrl:action": [{{"@id": "use"}}],
                "dutyMode": "advise"
              }},
              "policies": [{{
                "id": "policy-c",
                "kind": "Set",
                "assigner": "did:web:provider.example",
                "assignee": null,
                {conflict_line}
                "permissions": [{{"action": "use", "constraints": []}}],
                "prohibitions": [{{"action": "use", "constraints": []}}],
                "obligations": []
              }}],
              "claims": {{}}
            }}"#
        )
    }

    const VOID_REASON: &str = "policy 'policy-c' is void: permission[0] and prohibition[0] both matched \
                               requested action 'use', and the policy's odrl:conflict strategy is \
                               'invalid' (ODRL's own default), which voids a conflicting policy rather \
                               than resolving it";

    #[test]
    fn a_policy_declaring_no_conflict_strategy_is_void_when_a_permission_and_a_prohibition_collide() {
        // The deliberate behaviour change: before `odrl:conflict` existed
        // this engine resolved every collision prohibition-first,
        // unconditionally. ODRL's own default for a policy that declares no
        // conflict term is `invalid` -- the policy is void -- and that is
        // now what an undeclared strategy means here. Same `Deny`, entirely
        // different reason, and a different reason is the only observable
        // the wire contract has for it (there is deliberately no fourth
        // `WireDecision`).
        let response = evaluate_text(&conflicting_request(""));
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(response.reason, VOID_REASON);
    }

    #[test]
    fn an_explicitly_declared_invalid_conflict_strategy_voids_the_policy_identically() {
        let response = evaluate_text(&conflicting_request(r#""odrl:conflict": "invalid","#));
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason, VOID_REASON,
            "declaring the default explicitly must be indistinguishable from not declaring it"
        );
    }

    #[test]
    fn a_declared_perm_conflict_strategy_lets_the_permission_beat_the_prohibition() {
        // The one combining rule this engine has never had: a matching
        // permission wins over a matching prohibition, because the policy
        // says so.
        let response = evaluate_text(&conflicting_request(r#""odrl:conflict": "perm","#));
        assert_eq!(response.decision, WireDecision::Allow, "{}", response.reason);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'policy-c' matched: action 'use', unconstrained; \
             odrl:conflict 'perm' resolves the conflict with prohibition[0] in the permission's favour"
        );
    }

    #[test]
    fn a_declared_prohibit_conflict_strategy_keeps_the_prohibition_winning() {
        // What this engine did unconditionally before the field existed,
        // now reachable only by a policy that asks for it -- and saying so
        // in the trace, so "prohibition-first because the policy chose it"
        // and "prohibition-first because that is all this engine can do"
        // are never the same string again.
        let response = evaluate_text(&conflicting_request(r#""odrl:conflict": "prohibit","#));
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-c' matched: action 'use', unconstrained; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the prohibition's favour"
        );
    }

    #[test]
    fn a_conflict_term_outside_the_three_odrl_defines_is_a_parse_error() {
        // The `Behaviour`/`DutyMode` precedent, applied unchanged: an
        // enumerated wire term this engine does not know is a parse
        // failure, not a silently-substituted default. A host that
        // mistypes `prohibit`, or names a profile-declared strategy this
        // engine never implemented, must hear about it rather than be
        // handed some other strategy's answer.
        let err = serde_json::from_str::<Request>(&conflicting_request(r#""odrl:conflict": "ex:assigneeWins","#))
            .expect_err("an unknown odrl:conflict term must not parse");
        assert!(
            err.to_string().starts_with("unknown variant `ex:assigneeWins`, expected one of `perm`, `prohibit`, `invalid`"),
            "{err}"
        );
    }

    #[test]
    fn odrl_conflict_is_a_within_policy_rule_and_does_not_reach_across_the_policy_set() {
        // ODRL states `conflict` of one Policy, about that policy's own
        // permissions and prohibitions. A permission in policy A and a
        // prohibition in policy B are not a conflict in its sense, so
        // Section 5.2's own deny-override across the set decides them
        // exactly as it always did -- `perm` on A cannot promote A's
        // permission over somebody else's prohibition.
        let two_policies = conflicting_request("").replace(
            r#""prohibitions": [{"action": "use", "constraints": []}],
                "obligations": []
              }],"#,
            r#""prohibitions": [],
                "obligations": []
              },
              {
                "id": "policy-d",
                "kind": "Set",
                "assigner": "did:web:provider.example",
                "assignee": null,
                "permissions": [],
                "prohibitions": [{"action": "use", "constraints": []}],
                "obligations": []
              }],"#,
        );
        let with_perm = two_policies.replacen(
            r#""id": "policy-c","#,
            r#""id": "policy-c", "odrl:conflict": "perm","#,
            1,
        );

        for (label, text) in [("undeclared", two_policies.as_str()), ("perm on policy-c", with_perm.as_str())] {
            let response = evaluate_text(text);
            assert_eq!(response.decision, WireDecision::Deny, "{label}: {}", response.reason);
            assert_eq!(
                response.reason,
                "prohibition[0] of policy 'policy-d' matched: action 'use', unconstrained",
                "{label}: the set-level rule decides, and says so without any conflict clause"
            );
        }
    }

    #[test]
    fn a_declared_conflict_strategy_is_inert_for_a_request_with_no_genuine_collision() {
        // The regression guard at the wire level: `decision.rs` asserts it
        // per decision, this asserts the whole `Response` -- decision,
        // reason string and duty list -- is byte-identical across all three
        // strategies for request shapes that carry no collision, which is
        // every shape this workspace's own fixtures and the vendored
        // compliance corpus actually contain.
        let prohibition_only = conflicting_request("").replace(
            r#""permissions": [{"action": "use", "constraints": []}],"#,
            r#""permissions": [],"#,
        );
        let permission_misses = conflicting_request("").replace(
            r#""prohibitions": [{"action": "use", "constraints": []}]"#,
            r#""prohibitions": [{"action": "use", "constraints": [{"left_operand": "sub", "operator": "eq", "right_operand": "bob"}]}]"#,
        );

        for (label, text) in [
            ("the Section 5.2 worked example", ALLOW_EXAMPLE.to_string()),
            ("a prohibition with no permission beside it", prohibition_only),
            ("a permission whose prohibition misses on a constraint", permission_misses),
        ] {
            let baseline = evaluate_text(&text);
            for (term, strategy) in [
                ("perm", ConflictStrategy::Perm),
                ("prohibit", ConflictStrategy::Prohibit),
                ("invalid", ConflictStrategy::Invalid),
            ] {
                let declared = text.replacen(r#""kind":"#, &format!(r#""odrl:conflict": "{term}", "kind":"#), 1);
                let parsed: Request = serde_json::from_str(&declared).unwrap();
                assert_eq!(
                    parsed.policies[0].conflict, strategy,
                    "{label}: the control -- the injected key must actually reach the typed field, \
                     or the comparison below proves nothing"
                );
                assert_eq!(evaluate_request(&parsed), baseline, "{label} / {term}");
            }
        }
    }
}
