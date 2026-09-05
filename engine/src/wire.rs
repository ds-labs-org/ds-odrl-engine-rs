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

use serde::{Deserialize, Serialize};

use crate::claims::Claims;
use crate::constraint::{Constraint, Operator, MAX_CONSTRAINT_DEPTH};
use crate::decision::{decide, Decision, DecisionOutcome, Policy, Rule};
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
}

impl From<&RequestConfig> for ResolvedConfig {
    fn from(config: &RequestConfig) -> Self {
        ResolvedConfig::new(
            config.actions.iter().map(ActionDecl::from).collect(),
            config.duty_mode,
            config.behaviour,
        )
    }
}

/// One policy exactly as Section 5.2 documents it on the wire: the
/// identity fields (`id`, `kind`, `assigner`, `assignee`) that
/// `decision::Policy` has no use for, plus the same permission/
/// prohibition/obligation lists that type does consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePolicy {
    pub id: String,
    pub kind: String,
    pub assigner: String,
    pub assignee: Option<String>,
    #[serde(default)]
    pub permissions: Vec<Rule>,
    #[serde(default)]
    pub prohibitions: Vec<Rule>,
    #[serde(default)]
    pub obligations: Vec<Rule>,
}

impl WirePolicy {
    fn as_decision_policy(&self) -> Policy {
        Policy {
            permissions: self.permissions.clone(),
            prohibitions: self.prohibitions.clone(),
            obligations: self.obligations.clone(),
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub dataset_id: String,
    pub action: String,
    pub config: RequestConfig,
    #[serde(default)]
    pub policies: Vec<WirePolicy>,
    #[serde(default)]
    pub claims: Claims,
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
    let covers_and_matches =
        |rule: &Rule| rule.applies(requested_action, requested_target, config, claims) && rule.matches(claims);
    // A rule that would have applied in every other respect and is
    // inapplicable *purely* because it is about a different asset — the
    // one case where the target is the whole reason and deserves naming,
    // kept distinct from an action mismatch so a denied request's trace
    // says which of the two actually failed.
    let blocked_only_by_target = |rule: &Rule| {
        !rule.target_applies(requested_target)
            && rule.action_applies(requested_action, config, claims)
            && rule.matches(claims)
    };
    // A rule that covered the requested action and satisfied all its own
    // constraints, and was inapplicable *purely* because of its action
    // refinement — the one case where the refinement is the whole reason
    // and deserves to be named as such.
    let blocked_only_by_refinement = |rule: &Rule| {
        rule.target_applies(requested_target)
            && rule.covers_action(requested_action, config)
            && rule.matches(claims)
            && !rule.refinement_satisfied(claims)
    };

    match &outcome.decision {
        Decision::Error(unrecognized) => format!("policy '{}': {unrecognized}", policy.id),
        Decision::Deny => {
            if let Some((index, rule)) = policy.prohibitions.iter().enumerate().find(|(_, rule)| covers_and_matches(rule))
            {
                return format!(
                    "prohibition[{index}] of policy '{}' matched: {}",
                    policy.id,
                    describe_rule(rule, requested_action)
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
            let any_permission_covers_and_matches = policy.permissions.iter().any(covers_and_matches);
            let permission_requirement_met = match config.behaviour {
                Behaviour::Open => policy.permissions.is_empty() || any_permission_covers_and_matches,
                Behaviour::Closed => any_permission_covers_and_matches,
            };
            if !permission_requirement_met {
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

            match outcome.unresolved_duties.first() {
                Some(duty) => format!(
                    "duty[{}] '{}' of policy '{}' is unresolved under duty_mode: deny",
                    duty.duty_index, duty.action, policy.id
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
            match policy.permissions.iter().enumerate().find(|(_, rule)| covers_and_matches(rule)) {
                Some((index, rule)) => format!(
                    "permission[{index}] of policy '{}' matched: {}",
                    policy.id,
                    describe_rule(rule, requested_action)
                ),
                None => format!(
                    "policy '{}' allowed for a reason this trace could not reconstruct",
                    policy.id
                ),
            }
        }
    }
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

    let evaluations: Vec<Evaluation> = req
        .policies
        .iter()
        .map(|policy| Evaluation {
            policy,
            // `req.dataset_id` is this request's `odrl:target` (see
            // `Request`'s own doc comment) — the asset each rule's own
            // `odrl:target`, if it has one, is compared against.
            outcome: decide(
                &policy.as_decision_policy(),
                &req.claims,
                &config,
                requested_action,
                &req.dataset_id,
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
        &config,
    );

    let duties = if matches!(deciding.outcome.decision, Decision::Error(_)) || config.duty_mode == DutyMode::Deny {
        Vec::new()
    } else {
        evaluations
            .iter()
            .flat_map(|e| {
                e.outcome.unresolved_duties.iter().map(move |duty| DutyEntry {
                    policy_id: e.policy.id.clone(),
                    action: duty.action.clone(),
                    resolved: false,
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
        }
    }

    #[test]
    fn a_matching_prohibition_denies_and_names_itself_in_the_reason() {
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
            }],
            claims: [("nationality".to_string(), ClaimValue::Single("US".to_string()))]
                .into_iter()
                .collect(),
        };

        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert_eq!(
            response.reason,
            "prohibition[0] of policy 'policy-2' matched: action 'use': nationality eq US"
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
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "use".to_string(),
            config: deny_config(&["use"]),
            policies: vec![WirePolicy {
                id: "policy-nested".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![Rule::new("use", vec![])],
                prohibitions: vec![Rule::new(
                    "use",
                    vec![crate::constraint::Constraint::and(vec![
                        crate::constraint::Constraint::new("nationality", Operator::Eq, "US"),
                        crate::constraint::Constraint::new("scope", Operator::IsAnyOf, "embargoed"),
                    ])],
                )],
                obligations: vec![],
            }],
            claims: [
                ("nationality".to_string(), ClaimValue::Single("US".to_string())),
                ("scope".to_string(), ClaimValue::Single("embargoed".to_string())),
            ]
            .into_iter()
            .collect(),
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
            },
            policies: vec![WirePolicy {
                id: "policy-transfer".to_string(),
                kind: "Offer".to_string(),
                assigner: "did:web:provider.example".to_string(),
                assignee: None,
                permissions: vec![Rule::new("transfer", vec![])],
                prohibitions: vec![],
                obligations: vec![],
            }],
            claims: Claims::new(),
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
                },
                WirePolicy {
                    id: "policy-bad".to_string(),
                    kind: "Offer".to_string(),
                    assigner: "did:web:provider.example".to_string(),
                    assignee: None,
                    permissions: vec![Rule::new("anonymize", vec![])],
                    prohibitions: vec![],
                    obligations: vec![],
                },
            ],
            claims: Claims::new(),
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
            }],
            claims: Claims::new(),
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
            }],
            claims: Claims::new(),
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
            }],
            claims: Claims::new(),
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
                },
            ],
            claims: Claims::new(),
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
            }],
            claims: [
                ("sub".to_string(), ClaimValue::Single("bob".to_string())),
                ("copies".to_string(), ClaimValue::Single("5".to_string())),
            ]
            .into_iter()
            .collect(),
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
        };
        let unrecognized = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "read".to_string(),
            config: deny_config(&["read"]),
            policies: vec![wire_policy("policy-bad", vec![Rule::new("anonymize", vec![])], vec![])],
            claims: Claims::new(),
        };
        let no_policies = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            action: "read".to_string(),
            config: deny_config(&["read", "write"]),
            policies: vec![],
            claims: Claims::new(),
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
}
