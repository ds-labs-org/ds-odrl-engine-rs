//! Maps an expanded ODRL policy node (`jsonld::expand`) onto `engine`'s
//! Section 5.2 wire shape. See this crate's README for the scope boundary.
//!
//! Two naming conventions this module applies, both deliberate and both
//! visible in every test below:
//!
//! - **Vocabulary terms are compacted out of the ODRL namespace.** An
//!   action, a policy class and a `leftOperand` are *vocabulary*, so
//!   `http://www.w3.org/ns/odrl/2/use` becomes `use` and
//!   `http://www.w3.org/ns/odrl/2/dateTime` becomes `dateTime`. That is
//!   what makes an ingested policy line up with the vocabulary the rest of
//!   this workspace already uses (`engine`'s own Section 5.2 example,
//!   `compliance-runner`'s `base_action_vocabulary`,
//!   `profile-interpreter/examples/odrl-2.2-common-actions.ttl`), and with
//!   the flat claim-map keys `engine::Claims` is built from. An IRI outside
//!   the ODRL namespace is left exactly as written.
//! - **A `rightOperand` is never compacted.** It is *data* — the value a
//!   host claim is compared against — not vocabulary, so it is carried
//!   byte for byte, including a literal that happens to begin with
//!   `odrl:`.

use std::collections::BTreeSet;

use engine::claims::Claims;
use engine::constraint::{Constraint, Operator, MAX_CONSTRAINT_DEPTH};
use engine::decision::{ConflictStrategy, Rule, MAX_CONSEQUENCE_DEPTH};
use engine::profile::{Behaviour, DutyMode};
use engine::wire::{Request, RequestConfig, WireActionDecl, WirePolicy};

use crate::jsonld::{expand, Expanded, JsonLdError, Node, ODRL_NS};

/// `rdf:value`, which ODRL 2.2 uses to name the action inside an Action
/// node that also carries an `odrl:refinement`.
const RDF_VALUE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#value";

/// The ODRL classes this adapter recognizes as "this node is the policy".
/// `Policy` itself is included because a document may state only the
/// abstract class; it maps to `Set`, ODRL's own unrestricted subclass.
const POLICY_CLASSES: &[&str] = &["Offer", "Agreement", "Set", "Policy", "Request", "Ticket", "Assertion", "Privacy"];

/// One ingested DSP contract policy, plus everything this adapter had to
/// take verbatim, guess, or drop on the way — the same `{ value, warnings }`
/// shape `profile-interpreter::interpret` already returns, for the same
/// reason: an adapter that silently discards what it could not map is an
/// adapter you cannot audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingested {
    pub policy: WirePolicy,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    Json(String),
    UnknownContext(String),
    MalformedContext(String),
    NotANodeObject,
    NoPolicyNode,
    SeveralPolicyNodes(Vec<String>),
    RuleWithoutAction(String),
    /// A rule stated as a bare `{"@id": …}` reference to a body defined in
    /// a document this adapter does not resolve. An error rather than a
    /// skip: silently dropping a prohibition is fail-open.
    RuleIsABareReference(String, String),
    ConstraintWithoutOperator,
    ConstraintWithoutLeftOperand,
    ConstraintWithoutRightOperand,
    UnsupportedOperator(String),
    /// A logical constraint tree nested past `engine::MAX_CONSTRAINT_DEPTH`
    /// — the same bound the evaluator itself stops recursing at.
    ConstraintNestedTooDeep(usize),
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for IngestError {}

impl From<JsonLdError> for IngestError {
    fn from(e: JsonLdError) -> Self {
        match e {
            JsonLdError::UnknownContext(url) => IngestError::UnknownContext(url),
            JsonLdError::MalformedContext(what) => IngestError::MalformedContext(what),
            JsonLdError::NotANodeObject => IngestError::NotANodeObject,
        }
    }
}

/// Ingests the ODRL policy carried by one DSP contract offer/agreement
/// document (or by a bare ODRL policy document) into `engine`'s Section 5.2
/// `WirePolicy`.
pub fn ingest_policy(json: &str) -> Result<Ingested, IngestError> {
    let doc: serde_json::Value = serde_json::from_str(json).map_err(|e| IngestError::Json(e.to_string()))?;
    ingest_policy_value(&doc)
}

/// `ingest_policy` for a caller that already parsed the document.
pub fn ingest_policy_value(doc: &serde_json::Value) -> Result<Ingested, IngestError> {
    let expansion = expand(doc)?;
    let mut warnings = expansion.warnings;

    let mut found: Vec<&Node> = Vec::new();
    collect_policy_nodes(&expansion.node, 0, &mut found);
    let node = match found.as_slice() {
        [] => return Err(IngestError::NoPolicyNode),
        [one] => *one,
        several => {
            return Err(IngestError::SeveralPolicyNodes(
                several.iter().map(|n| n.id.clone().unwrap_or_else(|| "<no @id>".to_string())).collect(),
            ))
        }
    };

    let policy = policy_from(node, &mut warnings)?;
    Ok(Ingested { policy, warnings })
}

/// A floor `config` for the ingested policy: every action its own rules
/// name, declared so `engine` does not answer `Decision::Error` for a
/// vocabulary gap (Section 4.4) before it ever evaluates a rule.
///
/// **Not a substitute for a real ODRL Profile.** It declares no
/// `odrl:includedIn` edge, because a contract policy declares no action
/// taxonomy — a permission for `transfer` ingested through here will *not*
/// cover a request for `sell`. A host that wants real coverage builds its
/// `config` with `profile-interpreter` from actual Profile documents and
/// uses this only as a fallback.
///
/// **Includes every nested duty/remedy/consequence action, not only each
/// rule's own.** Section 4.4's vocabulary check applies uniformly to a
/// duty's `action` too (`decision::grants`'s own "unrecognized action ...
/// in the duty at ..." error), so a permission's `odrl:duty` naming
/// `compensate` needs `compensate` declared exactly as `use` itself does —
/// otherwise ingesting a real duty would trade one silent-Allow bug for a
/// noisy `Decision::Error` on every offer that carries one.
pub fn minimal_config(policy: &WirePolicy, duty_mode: DutyMode, behaviour: Behaviour) -> RequestConfig {
    let mut actions: BTreeSet<String> = BTreeSet::new();
    for rule in policy.permissions.iter().chain(&policy.prohibitions).chain(&policy.obligations) {
        collect_actions(rule, &mut actions);
    }
    RequestConfig {
        type_: "odrl:Profile".to_string(),
        id: "urn:dsp-odrl-adapter:minimal-config".to_string(),
        actions: actions.into_iter().map(|id| WireActionDecl { id, included_in: None }).collect(),
        duty_mode,
        behaviour,
        // Party-role scoping stays off for an ingested DSP offer: the
        // adapter has no way to know which claim key the host's own
        // identity plumbing puts the caller's identifier under, and
        // guessing `sub` would silently switch on a decision-semantics
        // change the host never asked for. A host that wants it sets
        // `partyIdentityClaim` on the config this returns.
        party_identity_claim: None,
    }
}

/// Inserts `rule`'s own action, then recurses into its `odrl:duty`,
/// `odrl:remedy`, and `odrl:consequence` — the three positions a `Rule`
/// can carry another `Rule` in. Terminates: `duty`/`remedy` are only ever
/// populated one level deep by this adapter (a nested duty carries no
/// `odrl:duty`/`odrl:remedy` of its own, per their Vocabulary domains —
/// see `duty_from`'s own `warn_wrong_domain` calls), and `consequence`'s
/// own chain is bounded at ingestion by `MAX_CONSEQUENCE_DEPTH`
/// (`consequence_from`), well short of anything requiring a depth guard
/// here as well.
fn collect_actions(rule: &Rule, actions: &mut BTreeSet<String>) {
    actions.insert(rule.action.clone());
    for duty in &rule.duty {
        collect_actions(duty, actions);
    }
    for remedy in &rule.remedy {
        collect_actions(remedy, actions);
    }
    if let Some(consequence) = &rule.consequence {
        collect_actions(consequence, actions);
    }
}

/// A complete Section 5.2 `Request` around one ingested policy, using
/// `minimal_config` above. `dataset_id` is the request's `odrl:target`, so
/// it is what each rule's own `odrl:target` is matched against.
pub fn request_for(
    policy: &WirePolicy,
    dataset_id: &str,
    action: &str,
    claims: Claims,
    duty_mode: DutyMode,
    behaviour: Behaviour,
) -> Request {
    Request {
        dataset_id: dataset_id.to_string(),
        action: action.to_string(),
        config: minimal_config(policy, duty_mode, behaviour),
        policies: vec![policy.clone()],
        claims,
        // This adapter resolves no `odrl:AssetCollection` membership of
        // its own — it has no state-of-the-world graph to resolve it
        // against — so this stays empty exactly as every other field this
        // ingestion path does not populate does.
        asset_collections: Vec::new(),
    }
}

// -- node walking ---------------------------------------------------------

/// Every node in the tree typed as an ODRL policy class. Depth-first from
/// the root, so a bare policy document finds itself, and a DSP negotiation
/// message finds the one nested under `dspace:offer`/`dspace:agreement` —
/// **matched by ODRL `@type`, not by the DSP property IRI**, which differs
/// between DSP 2024/1 (`https://w3id.org/dspace/2024/1/offer`) and 2025/1
/// (`…/2025/1/offer`) and would otherwise need one hardcoded case per DSP
/// revision.
fn collect_policy_nodes<'a>(node: &'a Node, depth: usize, out: &mut Vec<&'a Node>) {
    if depth > 8 {
        return;
    }
    if node.types.iter().any(|t| policy_kind(t).is_some()) {
        out.push(node);
        return;
    }
    for (_, values) in &node.props {
        for value in values {
            if let Expanded::Node(child) = value {
                collect_policy_nodes(child, depth + 1, out);
            }
        }
    }
}

/// `Some(kind)` when `iri` names an ODRL policy class — the string that
/// becomes `WirePolicy::kind`. `odrl:Policy` maps to `Set`, ODRL's own
/// unrestricted subclass, since `kind` is a concrete label.
fn policy_kind(iri: &str) -> Option<String> {
    let local = iri.strip_prefix(ODRL_NS)?;
    if !POLICY_CLASSES.contains(&local) {
        return None;
    }
    Some(if local == "Policy" { "Set".to_string() } else { local.to_string() })
}

/// This adapter's compaction convention for a *vocabulary* IRI — see this
/// module's own doc comment. Never applied to a `rightOperand`.
fn compact(iri: &str) -> String {
    iri.strip_prefix(ODRL_NS).unwrap_or(iri).to_string()
}

fn odrl(node: &Node, local: &str) -> Vec<Expanded> {
    node.get(&format!("{ODRL_NS}{local}")).to_vec()
}

/// The string an expanded value carries, whichever form it took: an IRI, a
/// literal, or a node reference that is nothing but an `@id`.
fn as_string(value: &Expanded) -> Option<String> {
    match value {
        Expanded::Iri(iri) => Some(iri.clone()),
        Expanded::Literal(lit) => Some(lit.clone()),
        Expanded::Node(n) if n.props.is_empty() => n.id.clone(),
        Expanded::Node(_) => None,
    }
}

fn first_string(node: &Node, local: &str) -> Option<String> {
    odrl(node, local).first().and_then(as_string)
}

// -- policy ---------------------------------------------------------------

fn policy_from(node: &Node, warnings: &mut Vec<String>) -> Result<WirePolicy, IngestError> {
    let kind = node.types.iter().find_map(|t| policy_kind(t)).expect("collect_policy_nodes only yields policy nodes");

    let id = node.id.clone().unwrap_or_else(|| {
        warnings.push("the policy node carries no @id; WirePolicy.id is the empty string".to_string());
        String::new()
    });
    let assigner = first_string(node, "assigner").unwrap_or_else(|| {
        warnings.push("the policy names no odrl:assigner; WirePolicy.assigner is the empty string".to_string());
        String::new()
    });

    // ODRL scopes a Policy-level odrl:target to every rule the policy
    // carries; `engine::Rule` has a per-rule target and no policy-level
    // one, so the faithful mapping is to push it down onto each rule that
    // does not name its own.
    let policy_target = first_string(node, "target");

    // Information Model §2.7.1 "Compact Policy": shared properties MAY
    // include "One or many `action` properties" declared once at the
    // Policy level, with the normative processing step "Replicate these
    // properties in the Rule" for every Rule lacking its own -- the exact
    // same shorthand `policy_target` above already implements for
    // `odrl:target`. Read with the same compaction convention `action_from`
    // applies to a rule's own `odrl:action` (an IRI or literal is stripped
    // of the ODRL namespace; an Action node's `rdf:value` is used, ignoring
    // any policy-level `odrl:refinement`, which a rule pushdown target
    // never carried either).
    let policy_action = policy_action_from(node, warnings);

    if !odrl(node, "profile").is_empty() {
        warnings.push(
            "the policy declares an odrl:profile; this adapter does not load it, so any term it \
             defines is ingested as an ordinary opaque string"
                .to_string(),
        );
    }
    if !odrl(node, "inheritFrom").is_empty() {
        warnings.push(
            "the policy declares odrl:inheritFrom; this adapter does not map it, so \
             WirePolicy.inherit_from is left empty here -- engine::wire itself now resolves the \
             field when a caller populates it directly, but this ingestion path still does not"
                .to_string(),
        );
    }
    // `engine::Policy` really evaluates `odrl:conflict` now, and this
    // adapter maps no conflict term onto it: ingesting one means deciding
    // what an IRI-or-literal `odrl:perm`/`odrl:prohibit`/`odrl:invalid`
    // expands to and what an unrecognized term should do, which is its own
    // decision rather than a side effect of the engine gaining the field.
    // Until then the engine's default (`invalid` -- void a conflicting
    // policy) applies, so an offer asking for `perm` would get the opposite
    // answer, and that has to be said out loud.
    if !odrl(node, "conflict").is_empty() {
        warnings.push(
            "the policy declares odrl:conflict; this adapter ingests no conflict strategy, so the \
             engine's own default applies (invalid: a policy whose permission and prohibition both \
             match is void)"
                .to_string(),
        );
    }

    Ok(WirePolicy {
        id,
        kind,
        assigner,
        assignee: first_string(node, "assignee"),
        permissions: rules_from(node, "permission", policy_target.as_deref(), policy_action.as_deref(), warnings)?,
        prohibitions: rules_from(node, "prohibition", policy_target.as_deref(), policy_action.as_deref(), warnings)?,
        obligations: rules_from(node, "obligation", policy_target.as_deref(), policy_action.as_deref(), warnings)?,
        // Never ingested from the document -- see the warning above.
        conflict: ConflictStrategy::default(),
        // Never ingested from the document either -- see the odrl:inheritFrom warning above.
        inherit_from: None,
    })
}

fn rules_from(
    policy: &Node,
    local: &str,
    policy_target: Option<&str>,
    policy_action: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<Vec<Rule>, IngestError> {
    let mut rules = Vec::new();
    for value in odrl(policy, local) {
        let Expanded::Node(node) = value else {
            return Err(IngestError::RuleIsABareReference(
                local.to_string(),
                as_string(&value).unwrap_or_default(),
            ));
        };
        rules.push(rule_from(&node, local, policy_target, policy_action, warnings)?);
    }
    Ok(rules)
}

fn rule_from(
    node: &Node,
    local: &str,
    policy_target: Option<&str>,
    policy_action: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<Rule, IngestError> {
    let (action, action_refinement) = action_from(node, local, policy_action, warnings)?;

    let mut constraints = Vec::new();
    for value in odrl(node, "constraint") {
        constraints.push(constraint_from(&value, 0)?);
    }

    let target = first_string(node, "target").or_else(|| policy_target.map(str::to_string));
    let mut rule = Rule { target, action_refinement, ..Rule::new(action, constraints) };

    // `odrl:duty` (Vocabulary: domain Permission, also read at Policy
    // level per the Information Model) gates *this* permission: it "must
    // be satisfied for the Permission to be exercised". Only a permission
    // reads it here; `Policy::obligations` is where the Policy-level
    // reading already lands, one rule per declared `odrl:obligation`, so
    // there is no second place in this per-rule function that means that.
    if local == "permission" {
        rule.duty = duties_from(node, "duty", warnings)?;
    } else {
        warn_wrong_domain(node, "duty", local, "Permission", warnings);
    }

    // `odrl:remedy` (Vocabulary: domain Prohibition) — Model §2.6.7's
    // Duty that "MUST be fulfilled in case that a Prohibition has been
    // infringed". Never lifts the prohibition itself; `engine` already
    // encodes that.
    if local == "prohibition" {
        rule.remedy = duties_from(node, "remedy", warnings)?;
    } else {
        warn_wrong_domain(node, "remedy", local, "Prohibition", warnings);
    }

    // `odrl:consequence` (Vocabulary: domain Duty) — an `odrl:obligation`
    // rule *is itself* the Policy-level Duty (there is no separate nested
    // "duty" property to unwrap for it, unlike a permission's `odrl:duty`
    // above), so its own `odrl:consequence` is read right here, the same
    // way a nested duty's is in `duty_from` below.
    if local == "obligation" {
        rule.consequence = consequence_from(node, 0, warnings)?;
    } else {
        warn_wrong_domain(node, "consequence", local, "Duty", warnings);
    }

    Ok(rule)
}

/// Reads every nested Duty under `odrl:{property}` (`"duty"` on a
/// permission, `"remedy"` on a prohibition) into the `Rule`s
/// `engine::Rule::duty`/`::remedy` hold directly — each one built the same
/// way a top-level rule is (action, constraints, own optional target),
/// plus its own `odrl:consequence` chain, since a Duty's domain includes
/// that property too.
fn duties_from(node: &Node, property: &str, warnings: &mut Vec<String>) -> Result<Vec<Rule>, IngestError> {
    let mut duties = Vec::new();
    for value in odrl(node, property) {
        let Expanded::Node(child) = value else {
            // A bare `{"@id": …}` reference to a duty defined elsewhere:
            // silently dropping it would leave the permission it was meant
            // to gate unconditional, or the prohibition's remedy
            // unenforced — the same fail-open `rules_from` above already
            // refuses for a top-level rule.
            return Err(IngestError::RuleIsABareReference(
                property.to_string(),
                as_string(&value).unwrap_or_default(),
            ));
        };
        // A permission's `odrl:duty` or a prohibition's `odrl:remedy` is a
        // fresh duty chain, not a continuation of some enclosing
        // `odrl:consequence` walk, so it starts its own depth count at 0 —
        // exactly as `engine::decision::outstanding_duty` starts the same
        // duty at depth 0 (see its own doc comment).
        duties.push(duty_from(&child, property, 0, warnings)?);
    }
    Ok(duties)
}

/// One Duty node reached in duty position (a permission's `odrl:duty`, a
/// prohibition's `odrl:remedy`, or an `odrl:consequence` successor) —
/// action, constraints, an optional own target (not inherited from the
/// policy: a duty's target, when it states one, is the duty's own
/// deliverable, not the asset the permission/prohibition it hangs off is
/// about), and its own `odrl:consequence`, if any.
///
/// `depth` is *this* duty's own position in a consequence chain (0 for a
/// permission's `odrl:duty` / a prohibition's `odrl:remedy`, or whatever
/// depth [`consequence_from`] reached to build this node as a successor);
/// it is what gets offered to this duty's own `odrl:consequence` lookup
/// below, one hop deeper, mirroring
/// `engine::decision::outstanding_duty_at`'s own `depth + 1` recursion.
fn duty_from(node: &Node, local: &str, depth: usize, warnings: &mut Vec<String>) -> Result<Rule, IngestError> {
    // A Duty node never inherits the enclosing Policy's `odrl:action` --
    // exactly like a Duty's own `target` above, which is never inherited
    // from the policy either (see this function's own doc comment): the
    // §2.7.1 Compact Policy pushdown replicates onto "the Rule" the Policy
    // itself carries (permission/prohibition/obligation), not onto a nested
    // duty/remedy/consequence reached through one of those.
    let (action, action_refinement) = action_from(node, local, None, warnings)?;

    let mut constraints = Vec::new();
    for value in odrl(node, "constraint") {
        constraints.push(constraint_from(&value, 0)?);
    }

    let mut rule =
        Rule { target: first_string(node, "target"), action_refinement, ..Rule::new(action, constraints) };
    rule.consequence = consequence_from(node, depth + 1, warnings)?;

    // A Duty's own domain carries neither `odrl:duty` (Permission's) nor
    // `odrl:remedy` (Prohibition's); either appearing here is an
    // unmappable shape, not silently droppable — dropping a duty's own
    // gating condition would be exactly the fail-open direction this item
    // exists to close.
    warn_wrong_domain(node, "duty", local, "Permission", warnings);
    warn_wrong_domain(node, "remedy", local, "Prohibition", warnings);

    Ok(rule)
}

/// `odrl:consequence` on a Duty node (Vocabulary: domain Duty) — "the
/// consequent Duty" `engine::Rule::consequence` chains to. Bounded at
/// [`MAX_CONSEQUENCE_DEPTH`], the same bound the engine itself stops
/// walking a chain at, so a chain deeper than that is dropped (warned,
/// never silently) rather than built into a `Rule` the engine would never
/// fully evaluate anyway.
fn consequence_from(node: &Node, depth: usize, warnings: &mut Vec<String>) -> Result<Option<Box<Rule>>, IngestError> {
    let values = odrl(node, "consequence");
    let Some(first) = values.first() else {
        return Ok(None);
    };
    if values.len() > 1 {
        warnings.push(format!(
            "this odrl:consequence carries {} duties; engine::Rule::consequence models a single \
             successor per duty (see its own doc comment on why), so only the first is ingested",
            values.len()
        ));
    }
    if depth >= MAX_CONSEQUENCE_DEPTH {
        warnings.push(format!(
            "an odrl:consequence chain nested past MAX_CONSEQUENCE_DEPTH ({MAX_CONSEQUENCE_DEPTH}) \
             is dropped rather than ingested past the depth the engine's own outstanding_duty \
             would ever walk"
        ));
        return Ok(None);
    }
    let Expanded::Node(child) = first else {
        return Err(IngestError::RuleIsABareReference(
            "consequence".to_string(),
            as_string(first).unwrap_or_default(),
        ));
    };
    // `child` is the successor duty this call was asked to build at `depth`
    // (the check just above already refused to build it past the bound);
    // `duty_from` uses that same `depth` to place *its own* consequence one
    // hop deeper.
    Ok(Some(Box::new(duty_from(child, "consequence", depth, warnings)?)))
}

/// Warns when `node` (a rule of kind `local`) carries `odrl:{property}`,
/// whose Vocabulary domain is `expected_domain`, not `local` — a shape
/// this adapter cannot map without guessing which `Rule` field the author
/// actually meant, so it is named and dropped rather than silently
/// ignored or misfiled.
fn warn_wrong_domain(node: &Node, property: &str, local: &str, expected_domain: &str, warnings: &mut Vec<String>) {
    if !odrl(node, property).is_empty() {
        warnings.push(format!(
            "the odrl:{local} rule carries odrl:{property}, whose domain is odrl:{expected_domain}, \
             not odrl:{local}; dropped rather than mapped to the wrong engine::Rule field"
        ));
    }
}

/// Reads the Policy-level `odrl:action` a Compact Policy (Information Model
/// §2.7.1) may declare once for the whole policy, for `policy_from` to offer
/// to [`action_from`] as the fallback a Rule naming none of its own falls
/// back to -- the same role `policy_target` plays for `odrl:target` above.
/// Applies this adapter's ordinary vocabulary-term compaction to whichever
/// of the two `odrl:action` shapes is present (a plain term, or an Action
/// node's `rdf:value`); a policy-level `odrl:refinement` is not read here,
/// matching the target pushdown, which likewise carries no per-rule detail
/// of its own.
fn policy_action_from(node: &Node, warnings: &mut Vec<String>) -> Option<String> {
    let values = odrl(node, "action");
    if values.len() > 1 {
        warnings.push(format!(
            "the policy names {} actions at its own level; this adapter's odrl:action pushdown \
             offers one default to a rule naming none of its own, so only the first is offered",
            values.len()
        ));
    }
    match values.first()? {
        Expanded::Iri(iri) => Some(compact(iri)),
        Expanded::Literal(lit) => Some(compact(lit)),
        Expanded::Node(action) => {
            action.get(RDF_VALUE).first().and_then(as_string).or_else(|| action.id.clone()).map(|s| compact(&s))
        }
    }
}

/// Reads a rule's `odrl:action`, in either of the two shapes ODRL 2.2
/// allows: the plain action term, or an Action node carrying `rdf:value`
/// plus one or more `odrl:refinement`s (the Information Model's own "print,
/// at most 2 copies" shape, which `engine::Rule::action_refinement` models
/// directly). When the rule names none of its own, falls back to
/// `policy_action` -- the Information Model §2.7.1 Compact Policy pushdown,
/// mirroring `policy_target`'s existing fallback for `odrl:target`.
fn action_from(
    node: &Node,
    local: &str,
    policy_action: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<(String, Option<Constraint>), IngestError> {
    let values = odrl(node, "action");
    if values.len() > 1 {
        warnings.push(format!(
            "the odrl:{local} rule names {} actions; this contract carries one action per rule, so \
             only the first is ingested",
            values.len()
        ));
    }
    let Some(value) = values.first() else {
        return policy_action
            .map(|action| (action.to_string(), None))
            .ok_or_else(|| IngestError::RuleWithoutAction(local.to_string()));
    };

    match value {
        Expanded::Iri(iri) => Ok((compact(iri), None)),
        Expanded::Literal(lit) => Ok((compact(lit), None)),
        Expanded::Node(action) => {
            let named = action
                .get(RDF_VALUE)
                .first()
                .and_then(as_string)
                .or_else(|| action.id.clone())
                .ok_or_else(|| IngestError::RuleWithoutAction(local.to_string()))?;

            let refinements: Vec<Constraint> = odrl(action, "refinement")
                .iter()
                .map(|r| constraint_from(r, 0))
                .collect::<Result<_, _>>()?;
            // Several refinements on one action are an implicit conjunction
            // (ODRL 2.2 §2.5); `engine::Rule` holds exactly one, and
            // `Constraint` is the type that already expresses "all of
            // these" natively.
            let refinement = match refinements.len() {
                0 => None,
                1 => refinements.into_iter().next(),
                _ => Some(Constraint::and(refinements)),
            };
            Ok((compact(&named), refinement))
        }
    }
}

// -- constraints ----------------------------------------------------------

fn constraint_from(value: &Expanded, depth: usize) -> Result<Constraint, IngestError> {
    let Expanded::Node(node) = value else {
        // A constraint given as a bare IRI reference is a pointer to a
        // constraint defined elsewhere in a document this adapter does not
        // resolve. Treating it as absent would silently widen a permission.
        return Err(IngestError::ConstraintWithoutOperator);
    };
    if depth > MAX_CONSTRAINT_DEPTH {
        // The same bound `engine::Constraint::evaluate` stops at: a tree
        // deeper than this could never change a decision anyway, and
        // building it would only grow the evaluator's stack.
        return Err(IngestError::ConstraintNestedTooDeep(depth));
    }

    // `xone`, then `or`, then `and` — **the same fixed precedence
    // `engine::Constraint::evaluate` applies** when a hand-written object
    // sets more than one of them (see `Constraint`'s own doc comment).
    // Choosing a different order here would make an ingested policy decide
    // differently from the identical policy written straight into Section
    // 5.2 JSON, for exactly the input the engine documents a rule for.
    for (local, build) in [
        ("xone", Constraint::xone as fn(Vec<Constraint>) -> Constraint),
        ("or", Constraint::or),
        ("and", Constraint::and),
    ] {
        // An `"odrl:and": []` is indistinguishable from an absent one after
        // expansion (an empty array contributes no values), so it falls
        // through to the atomic path below and surfaces as
        // `ConstraintWithoutLeftOperand` — still an error, which is the
        // part that matters: an empty `odrl:and` is *vacuously satisfied*
        // in this engine, so accepting one would silently make a permission
        // unconditional.
        let children = odrl(node, local);
        if children.is_empty() {
            continue;
        }
        let mapped: Vec<Constraint> =
            children.iter().map(|c| constraint_from(c, depth + 1)).collect::<Result<_, _>>()?;
        return Ok(build(mapped));
    }

    let left = first_string(node, "leftOperand").ok_or(IngestError::ConstraintWithoutLeftOperand)?;
    let operator_iri = first_string(node, "operator").ok_or(IngestError::ConstraintWithoutOperator)?;
    let operator = operator_from(&operator_iri)?;

    let right_values = odrl(node, "rightOperand");
    if right_values.is_empty() {
        return Err(IngestError::ConstraintWithoutRightOperand);
    }
    // A multi-valued ODRL rightOperand (what `isAnyOf` and friends really
    // take) becomes this engine's own comma-delimited string, which is the
    // convention `engine::Operator` already documents — including its lack
    // of any escaping convention for a value that itself contains a comma.
    let right: Vec<String> = right_values.iter().filter_map(as_string).collect();
    if right.is_empty() {
        return Err(IngestError::ConstraintWithoutRightOperand);
    }

    Ok(Constraint::new(compact(&left), operator, right.join(",")))
}

fn operator_from(iri: &str) -> Result<Operator, IngestError> {
    match iri.strip_prefix(ODRL_NS).unwrap_or(iri) {
        "eq" => Ok(Operator::Eq),
        "neq" => Ok(Operator::Neq),
        "lt" => Ok(Operator::Lt),
        "lteq" => Ok(Operator::Lteq),
        "gt" => Ok(Operator::Gt),
        "gteq" => Ok(Operator::Gteq),
        "isAnyOf" => Ok(Operator::IsAnyOf),
        "isAllOf" => Ok(Operator::IsAllOf),
        "isNoneOf" => Ok(Operator::IsNoneOf),
        "isPartOf" => Ok(Operator::IsPartOf),
        // Real ODRL 2.2 operators this engine has no evaluation for
        // (`odrl:isA`, `odrl:hasPart`), and anything a profile invented.
        // Dropping the constraint would leave the rule unconditional,
        // which for a permission is fail-open, so the ingest fails instead.
        _ => Err(IngestError::UnsupportedOperator(iri.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::wire::{evaluate_request, WireDecision};

    const DSP_2024_1: &str = include_str!("../examples/dsp-2024-1-contract-request.jsonld");
    const DSP_2025_1: &str = include_str!("../examples/dsp-2025-1-contract-request.jsonld");

    /// The offer's own `odrl:target`, which ODRL scopes to the whole policy
    /// and this adapter therefore pushes down onto every rule that names
    /// none of its own (`engine::Rule` has a per-rule target and no
    /// policy-level one).
    const OFFER_TARGET: &str = "urn:uuid:3dd1add8-4d2d-569e-d634-8394a8836a88";
    const PROHIBITION_TARGET: &str = "urn:uuid:0f2b6dcb-5f0c-4d1e-9a5f-1c0e6f5f0aa1";

    /// Exactly the `engine::wire::WirePolicy` both fixture documents must
    /// ingest to — written out field for field rather than asserted
    /// piecemeal, so a mapping change anywhere shows up as a diff here.
    fn expected_policy() -> WirePolicy {
        WirePolicy {
            id: "urn:uuid:d526561f-528e-4d5a-ae12-9a9dd9b7a815".to_string(),
            kind: "Offer".to_string(),
            assigner: "urn:tsdshhs636378".to_string(),
            assignee: Some("urn:jashd766".to_string()),
            permissions: vec![
                Rule::targeting(
                    "use",
                    OFFER_TARGET,
                    vec![Constraint::and(vec![
                        Constraint::new("dateTime", Operator::Lteq, "2026-12-31T06:00:00Z"),
                        Constraint::new("purpose", Operator::Eq, "odrl:internal-use-only"),
                    ])],
                ),
                Rule {
                    target: Some(OFFER_TARGET.to_string()),
                    ..Rule::refined("print", vec![], Constraint::new("resolution", Operator::Lteq, "1200"))
                },
            ],
            prohibitions: vec![Rule::targeting(
                "distribute",
                PROHIBITION_TARGET,
                vec![Constraint::new(
                    "https://example.org/claims/region",
                    Operator::IsAnyOf,
                    "US,CN",
                )],
            )],
            obligations: vec![Rule::targeting("inform", OFFER_TARGET, vec![])],
            // Neither fixture declares `odrl:conflict`, and this adapter
            // would not ingest one if it did: ODRL's own default.
            conflict: ConflictStrategy::default(),
            // Neither fixture declares `odrl:inheritFrom` either, and this
            // adapter would not map it if it did -- see the warning in
            // `policy_from`.
            inherit_from: None,
        }
    }

    #[test]
    fn a_dsp_2024_1_contract_request_ingests_field_for_field_into_the_wire_policy() {
        let ingested = ingest_policy(DSP_2024_1).expect("the 2024/1 fixture must ingest");
        assert_eq!(ingested.policy, expected_policy());
    }

    #[test]
    fn the_same_policy_in_the_dsp_2025_1_bare_term_shape_ingests_to_an_identical_wire_policy() {
        // The headline property, and the reason this adapter expands
        // against the document's declared @context instead of stripping a
        // literal `odrl:` prefix off every key: the two fixtures share not
        // one property key spelling (`odrl:permission` vs `permission`,
        // `odrl:leftOperand` vs `leftOperand`) and must still produce one
        // identical policy. A prefix-stripper produces nothing at all from
        // the 2025/1 document, whose keys carry no prefix to strip.
        let ingested = ingest_policy(DSP_2025_1).expect("the 2025/1 fixture must ingest");
        assert_eq!(ingested.policy, expected_policy());
    }

    #[test]
    fn a_right_operand_literal_that_starts_with_odrl_is_kept_verbatim() {
        // The exact input Prometheus-X odrl-manager's
        // `policy-helper/idsa.parser.json.ts` corrupts: it recursively
        // strips the literal string "odrl:" from every key *and every
        // string value*, so this right operand would arrive as
        // "internal-use-only" and silently compare unequal to the claim the
        // provider actually wrote. Nothing here strips anything from a
        // value: `odrl:rightOperand` carries no `@type` coercion in any of
        // the four bundled contexts, so its value is a plain literal and a
        // plain literal is taken exactly as written.
        for (label, fixture) in [("2024/1", DSP_2024_1), ("2025/1", DSP_2025_1)] {
            let ingested = ingest_policy(fixture).expect("fixture must ingest");
            let Some(children) = ingested.policy.permissions[0].constraints[0].and.as_ref() else {
                panic!("{label}: permission[0]'s constraint must be an odrl:and group");
            };
            assert_eq!(
                children[1].right_operand, "odrl:internal-use-only",
                "{label}: a right-operand literal beginning with `odrl:` must survive verbatim"
            );
        }
    }

    #[test]
    fn a_bare_odrl_policy_document_with_no_dsp_envelope_ingests_too() {
        // A host that has already unwrapped the negotiation message hands
        // the policy object itself; the policy node is found by its ODRL
        // `@type`, not by a hardcoded `dspace:offer` property IRI (which
        // differs between DSP 2024/1 and 2025/1 anyway).
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:bare-offer",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "permission": [{ "action": "use" }]
        }"#;
        let ingested = ingest_policy(doc).expect("a bare ODRL offer must ingest");
        assert_eq!(ingested.policy.id, "urn:uuid:bare-offer");
        assert_eq!(ingested.policy.kind, "Offer");
        assert_eq!(ingested.policy.assignee, None);
        assert_eq!(ingested.policy.permissions, vec![Rule::targeting("use", "urn:asset:A", vec![])]);
    }

    #[test]
    fn a_prohibition_keyed_by_its_full_odrl_iri_instead_of_the_compact_term_still_ingests() {
        // `http://www.w3.org/ns/odrl.jsonld` maps `prohibition` 1:1 onto
        // `http://www.w3.org/ns/odrl/2/prohibition`; writing that absolute
        // IRI directly in place of the compact term is legal,
        // RDF-equivalent JSON-LD, not a malformed document. Before the
        // `expand_iri` fix this key expanded to nothing and was dropped
        // with zero warning, so the whole prohibition vanished -- the
        // fail-open direction this crate's README calls "the worst answer
        // available".
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:t",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "permission": [{ "action": "use" }],
          "http://www.w3.org/ns/odrl/2/prohibition": [{ "action": "distribute" }]
        }"#;
        let ingested = ingest_policy(doc).expect("a full-IRI-keyed prohibition must still ingest");
        assert_eq!(
            ingested.policy.prohibitions,
            vec![Rule::targeting("distribute", "urn:asset:A", vec![])],
            "warnings: {:?}",
            ingested.warnings
        );
        assert!(ingested.warnings.is_empty(), "warnings: {:?}", ingested.warnings);
    }

    #[test]
    fn a_declared_odrl_conflict_term_is_warned_about_rather_than_silently_dropped() {
        // `engine::Policy` now really evaluates `odrl:conflict`, and this
        // adapter does not ingest one -- it maps no conflict term onto
        // `WirePolicy::conflict`, so an offer asking for `perm` would be
        // evaluated under ODRL's default `invalid` instead, which is the
        // opposite answer for a policy that actually conflicts. Silently
        // substituting one strategy for another is exactly the class of
        // loss every other warning here exists for.
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:conflicting-offer",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "conflict": "perm",
          "permission": [{ "action": "use" }],
          "prohibition": [{ "action": "use" }]
        }"#;
        let ingested = ingest_policy(doc).expect("a policy declaring odrl:conflict still ingests");
        assert_eq!(
            ingested.policy.conflict,
            ConflictStrategy::default(),
            "the strategy really is dropped -- which is what makes the warning load-bearing"
        );
        assert!(
            ingested.warnings.iter().any(|w| w.contains("odrl:conflict")),
            "warnings: {:?}",
            ingested.warnings
        );

        // The control: an otherwise identical offer that declares no
        // conflict term warns about nothing of the sort, so the assertion
        // above is about the declaration and not about a warning this
        // adapter emits for every policy.
        let quiet = ingest_policy(&doc.replace("\"conflict\": \"perm\",", "")).expect("must ingest");
        assert!(
            !quiet.warnings.iter().any(|w| w.contains("odrl:conflict")),
            "warnings: {:?}",
            quiet.warnings
        );
    }

    #[test]
    fn an_unknown_context_url_is_a_named_error_not_a_silently_empty_policy() {
        // This adapter never fetches a context over the network, so an
        // unbundled one cannot be resolved. Ignoring it would leave every
        // term unexpandable and yield an empty policy — which for a
        // document whose prohibitions all vanished is fail-open.
        let doc = r#"{
          "@context": "https://example.org/some/other/context.jsonld",
          "@type": "odrl:Offer",
          "odrl:permission": [{ "odrl:action": "odrl:use" }]
        }"#;
        assert_eq!(
            ingest_policy(doc),
            Err(IngestError::UnknownContext("https://example.org/some/other/context.jsonld".to_string()))
        );
    }

    #[test]
    fn a_constraint_written_with_the_idsa_examples_odrl_operand_misspelling_is_a_named_error() {
        // The IDSA specification's own contract-request-message.json
        // example writes `odrl:operand`; ODRL 2.2 names the property
        // `odrl:operator`. Quietly accepting the misspelling would make
        // this adapter's behaviour depend on a typo rather than on the
        // vocabulary, so a constraint with no `odrl:operator` is rejected
        // by name.
        let doc = r#"{
          "@context": "https://w3id.org/dspace/2024/1/context.json",
          "@type": "odrl:Offer",
          "odrl:permission": [{
            "odrl:action": "odrl:use",
            "odrl:constraint": [{
              "odrl:leftOperand": "odrl:dateTime",
              "odrl:operand": "odrl:lteq",
              "odrl:rightOperand": { "@value": "2023-12-31T06:00Z", "@type": "xsd:dateTime" }
            }]
          }]
        }"#;
        assert_eq!(ingest_policy(doc), Err(IngestError::ConstraintWithoutOperator));
    }

    #[test]
    fn an_operator_outside_this_engines_ten_is_a_named_error_not_a_dropped_constraint() {
        // `odrl:hasPart` is real ODRL 2.2 and this engine has no operator
        // for it. Dropping the constraint would leave a permission granted
        // unconditionally — the fail-open direction — so the whole ingest
        // fails, citing the operator IRI.
        let doc = r#"{
          "@context": "https://w3id.org/dspace/2024/1/context.json",
          "@type": "odrl:Offer",
          "odrl:permission": [{
            "odrl:action": "odrl:use",
            "odrl:constraint": [{
              "odrl:leftOperand": "odrl:spatial",
              "odrl:operator": "odrl:hasPart",
              "odrl:rightOperand": "EU"
            }]
          }]
        }"#;
        assert_eq!(
            ingest_policy(doc),
            Err(IngestError::UnsupportedOperator("http://www.w3.org/ns/odrl/2/hasPart".to_string()))
        );
    }

    #[test]
    fn a_constraint_setting_several_logical_keys_resolves_by_the_engines_own_precedence() {
        // A hand-written document *can* set more than one of
        // `odrl:and`/`odrl:or`/`odrl:xone` on one constraint node, and
        // `engine::Constraint::evaluate` resolves that by a fixed
        // `xone > or > and` precedence (constraint.rs's own doc comment).
        // This adapter has to pick the same one, or an ingested policy
        // would decide differently from the identical policy written
        // straight into Section 5.2 JSON.
        let doc = r#"{
          "@context": "https://w3id.org/dspace/2024/1/context.json",
          "@type": "odrl:Offer",
          "odrl:permission": [{
            "odrl:action": "odrl:use",
            "odrl:constraint": [{
              "odrl:and": [
                { "odrl:leftOperand": "odrl:purpose", "odrl:operator": "odrl:eq", "odrl:rightOperand": "a" }
              ],
              "odrl:xone": [
                { "odrl:leftOperand": "odrl:purpose", "odrl:operator": "odrl:eq", "odrl:rightOperand": "b" }
              ]
            }]
          }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        let constraint = &ingested.policy.permissions[0].constraints[0];
        assert!(constraint.and.is_none(), "`odrl:and` must lose to `odrl:xone`, as it does in the engine");
        assert_eq!(
            constraint.xone.as_ref().map(|c| c[0].right_operand.clone()),
            Some("b".to_string())
        );
    }

    #[test]
    fn a_rule_given_as_a_bare_reference_is_an_error_not_a_silently_skipped_rule() {
        // A rule stated as `{"@id": …}` points at a rule body defined in
        // some document this adapter does not resolve. Skipping it drops a
        // whole prohibition on the floor, which is the fail-open direction
        // and precisely the class of silent loss every other unmappable
        // construct here is an error for.
        let doc = r#"{
          "@context": "https://w3id.org/dspace/2024/1/context.json",
          "@type": "odrl:Offer",
          "odrl:permission": [{ "odrl:action": "odrl:use" }],
          "odrl:prohibition": [{ "@id": "urn:uuid:rule-defined-elsewhere" }]
        }"#;
        assert_eq!(
            ingest_policy(doc),
            Err(IngestError::RuleIsABareReference(
                "prohibition".to_string(),
                "urn:uuid:rule-defined-elsewhere".to_string()
            ))
        );
    }

    #[test]
    fn several_policy_nodes_in_one_document_is_a_named_error_listing_them() {
        // A whole catalog carries many offers. Picking the first would be a
        // guess at "which offer applies", which is a separate question with
        // its own answer, so this crate declines rather than guessing.
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Asset",
          "hasPolicy": [
            { "@id": "urn:uuid:offer-a", "@type": "Offer", "permission": [{ "action": "use" }] },
            { "@id": "urn:uuid:offer-b", "@type": "Offer", "permission": [{ "action": "print" }] }
          ]
        }"#;
        assert_eq!(
            ingest_policy(doc),
            Err(IngestError::SeveralPolicyNodes(vec![
                "urn:uuid:offer-a".to_string(),
                "urn:uuid:offer-b".to_string()
            ]))
        );
    }

    #[test]
    fn minimal_config_declares_every_action_the_ingested_policy_names() {
        // Without this the engine answers `Error` for the first rule naming
        // an action nothing declared (Section 4.4). It is a floor, not a
        // profile: no `odrl:includedIn` edges, because a contract policy
        // declares no action taxonomy.
        let ingested = ingest_policy(DSP_2024_1).unwrap();
        let config = minimal_config(&ingested.policy, DutyMode::Advise, Behaviour::Closed);
        let declared: Vec<&str> = config.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(declared, vec!["distribute", "inform", "print", "use"]);
        assert!(
            config.actions.iter().all(|a| a.included_in.is_none()),
            "a contract policy declares no action taxonomy, so no includedIn edge may be invented"
        );
        assert_eq!(config.duty_mode, DutyMode::Advise);
        assert_eq!(config.behaviour, Behaviour::Closed);
    }

    #[test]
    fn the_ingested_policy_decides_end_to_end_through_the_engine() {
        let ingested = ingest_policy(DSP_2024_1).unwrap();
        let claims: Claims = [
            ("dateTime".to_string(), "2026-01-01T00:00:00Z".into()),
            ("purpose".to_string(), "odrl:internal-use-only".into()),
        ]
        .into_iter()
        .collect();

        let allow = request_for(
            &ingested.policy,
            OFFER_TARGET,
            "use",
            claims.clone(),
            DutyMode::Advise,
            Behaviour::Closed,
        );
        let response = evaluate_request(&allow);
        assert_eq!(response.decision, WireDecision::Allow, "reason: {}", response.reason);
        assert_eq!(
            response.reason,
            "permission[0] of policy 'urn:uuid:d526561f-528e-4d5a-ae12-9a9dd9b7a815' matched: \
             action 'use' on target 'urn:uuid:3dd1add8-4d2d-569e-d634-8394a8836a88': \
             (dateTime lteq 2026-12-31T06:00:00Z && purpose eq odrl:internal-use-only)"
        );

        // The prohibition targets a different asset, so the same claims
        // asking to distribute *that* asset are denied — the per-rule
        // `odrl:target` the offer really carries, evaluated as written.
        let deny = request_for(
            &ingested.policy,
            PROHIBITION_TARGET,
            "distribute",
            [("https://example.org/claims/region".to_string(), "US".into())]
                .into_iter()
                .collect(),
            DutyMode::Advise,
            Behaviour::Closed,
        );
        assert_eq!(evaluate_request(&deny).decision, WireDecision::Deny);
    }

    #[test]
    fn a_request_built_from_an_ingested_policy_serializes_deterministically() {
        // `engine::Claims` is `HashMap`-backed, so a `Request` printed by
        // this crate's CLI must go through `serde_json::to_value` (this
        // workspace's `serde_json::Value::Object` is a `BTreeMap`; no crate
        // here enables `preserve_order`). A previously-shipped bug in this
        // workspace was exactly a non-deterministic export from skipping
        // that step, so this rebuilds the claims map from scratch each time
        // — a fresh `HashMap` gets a fresh `RandomState`, so a
        // direct-serialization regression would show up here.
        let ingested = ingest_policy(DSP_2024_1).unwrap();
        let render = || {
            let claims: Claims = [
                ("dateTime".to_string(), "2026-01-01T00:00:00Z".into()),
                ("purpose".to_string(), "odrl:internal-use-only".into()),
                ("https://example.org/claims/region".to_string(), "DE".into()),
                ("sub".to_string(), "alice".into()),
                ("scope".to_string(), "catalog:read".into()),
            ]
            .into_iter()
            .collect();
            let req = request_for(&ingested.policy, OFFER_TARGET, "use", claims, DutyMode::Advise, Behaviour::Closed);
            serde_json::to_string_pretty(&serde_json::to_value(&req).unwrap()).unwrap()
        };
        let first = render();
        for _ in 0..4 {
            assert_eq!(render(), first, "canonicalized Request JSON must be byte-identical every time");
        }
    }

    // -- odrl:duty / odrl:consequence / odrl:remedy ingestion -------------

    #[test]
    fn a_permission_naming_odrl_duty_ingests_it_instead_of_being_dropped() {
        // The task's own distinguishing example: a permission genuinely
        // gated on a duty to compensate.
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:duty-offer",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "permission": [{
            "action": "use",
            "duty": [{ "action": "compensate" }]
          }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        assert_eq!(
            ingested.policy.permissions[0].duty,
            vec![Rule::new("compensate", vec![])],
            "odrl:duty must be ingested into engine::Rule::duty, not dropped"
        );
        assert!(
            ingested.warnings.iter().all(|w| !w.contains("dropped")),
            "a genuinely mappable odrl:duty must not warn that it was dropped: {:?}",
            ingested.warnings
        );

        // Before this fix, `Rule::duty` was always empty regardless of the
        // document, so `DutyMode::Deny` had nothing to gate on and this
        // Allowed unconditionally -- exactly as if the duty had never
        // existed.
        let req = request_for(&ingested.policy, "urn:asset:A", "use", Claims::new(), DutyMode::Deny, Behaviour::Open);
        let response = evaluate_request(&req);
        assert_eq!(
            response.decision,
            WireDecision::Deny,
            "an unconstrained odrl:duty this engine has no claims-based way to confirm must stay \
             unresolved, and DutyMode::Deny must withhold the grant it gates -- reason: {}",
            response.reason
        );
        assert!(
            response.duties.iter().any(|d| d.action == "compensate" && !d.resolved),
            "the outstanding duty must be reported in the response: {:?}",
            response.duties
        );
    }

    #[test]
    fn a_permission_gated_on_a_constrained_duty_denies_unresolved_and_allows_once_satisfied() {
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:duty-gate",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "permission": [{
            "action": "use",
            "duty": [{
              "action": "compensate",
              "constraint": [{ "leftOperand": "payment", "operator": "eq", "rightOperand": "received" }]
            }]
          }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        assert_eq!(ingested.policy.permissions[0].duty.len(), 1, "the duty must be ingested");
        assert_eq!(ingested.policy.permissions[0].duty[0].constraints.len(), 1);

        let unresolved =
            request_for(&ingested.policy, "urn:asset:A", "use", Claims::new(), DutyMode::Deny, Behaviour::Open);
        assert_eq!(evaluate_request(&unresolved).decision, WireDecision::Deny);

        let satisfied: Claims = [("payment".to_string(), "received".into())].into_iter().collect();
        let resolved = request_for(&ingested.policy, "urn:asset:A", "use", satisfied, DutyMode::Deny, Behaviour::Open);
        assert_eq!(
            evaluate_request(&resolved).decision,
            WireDecision::Allow,
            "the same duty, satisfied from claims, must let the permission it gates grant"
        );
    }

    #[test]
    fn a_prohibition_naming_odrl_remedy_ingests_it_instead_of_being_silently_ignored() {
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:remedy-offer",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "prohibition": [{
            "action": "distribute",
            "remedy": [{ "action": "notify" }]
          }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        assert_eq!(
            ingested.policy.prohibitions[0].remedy,
            vec![Rule::new("notify", vec![])],
            "odrl:remedy must be ingested into engine::Rule::remedy, not dropped"
        );

        let req =
            request_for(&ingested.policy, "urn:asset:A", "distribute", Claims::new(), DutyMode::Advise, Behaviour::Open);
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny, "the prohibition itself still denies");
        assert!(
            response.duties.iter().any(|d| d.action == "notify" && !d.resolved),
            "the unresolved remedy must be reported: {:?}",
            response.duties
        );
    }

    #[test]
    fn an_obligation_naming_odrl_consequence_ingests_it_instead_of_being_dropped() {
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:consequence-offer",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "permission": [{ "action": "use" }],
          "obligation": [{
            "action": "notify",
            "consequence": [{ "action": "compensate" }]
          }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        assert_eq!(
            ingested.policy.obligations[0].consequence,
            Some(Box::new(Rule::new("compensate", vec![]))),
            "odrl:consequence must be ingested into engine::Rule::consequence, not dropped"
        );

        // The obligation's own action ("notify") is unconstrained, so it
        // never resolves either; its consequence is what `outstanding_duty`
        // reports once it doesn't, and a policy-level obligation denies
        // outright under DutyMode::Deny regardless of which permission was
        // requested.
        let req = request_for(&ingested.policy, "urn:asset:A", "use", Claims::new(), DutyMode::Deny, Behaviour::Open);
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert!(
            response.duties.iter().any(|d| d.action == "compensate" && !d.resolved),
            "the consequence successor must be the duty actually reported once the obligation \
             itself is unresolved: {:?}",
            response.duties
        );
    }

    #[test]
    fn an_odrl_consequence_chain_deeper_than_max_consequence_depth_is_bounded_at_ingestion() {
        // A chain 6 links deep, one unique action per link, so the exact
        // point of truncation is unambiguous. `rule_from`'s own call into
        // `consequence_from` starts the chain at depth 0, so — mirroring
        // `engine::decision::outstanding_duty_at`'s own depth walk, which
        // this ingestion-time bound exists to match — links c1..c4 (depths
        // 0..3) must be ingested and c5/c6 (depths 4 and 5) must not.
        let mut duty = r#"{ "action": "c6" }"#.to_string();
        for i in (1..=5).rev() {
            duty = format!(r#"{{ "action": "c{i}", "consequence": [{duty}] }}"#);
        }
        let doc = format!(
            r#"{{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:deep-consequence-offer",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "permission": [{{ "action": "use" }}],
          "obligation": [{{
            "action": "c0",
            "consequence": [{duty}]
          }}]
        }}"#
        );
        let ingested = ingest_policy(&doc).expect("must ingest");

        let mut actions = Vec::new();
        let mut next = ingested.policy.obligations[0].consequence.as_deref();
        while let Some(rule) = next {
            actions.push(rule.action.clone());
            next = rule.consequence.as_deref();
        }
        assert_eq!(
            actions,
            vec!["c1", "c2", "c3", "c4"],
            "MAX_CONSEQUENCE_DEPTH ({MAX_CONSEQUENCE_DEPTH}) links must be ingested and no \
             more — c5/c6 must be dropped, not built into a Rule engine's own \
             outstanding_duty_at would never fully walk anyway: {actions:?}"
        );
        assert!(
            ingested
                .warnings
                .iter()
                .any(|w| w.contains("MAX_CONSEQUENCE_DEPTH") && w.contains("dropped")),
            "dropping a chain past MAX_CONSEQUENCE_DEPTH must be warned, not silent: {:?}",
            ingested.warnings
        );
    }

    #[test]
    fn odrl_duty_odrl_remedy_odrl_consequence_in_the_wrong_domain_are_dropped_with_a_named_warning() {
        // odrl:duty's domain is Permission, not Prohibition; putting one on
        // a prohibition is an unmappable shape, not silently ignorable.
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:wrong-domain",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "prohibition": [{
            "action": "distribute",
            "duty": [{ "action": "compensate" }]
          }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        assert!(ingested.policy.prohibitions[0].duty.is_empty());
        assert!(
            ingested
                .warnings
                .iter()
                .any(|w| w.contains("odrl:duty") && w.contains("odrl:Permission")),
            "a wrong-domain odrl:duty must be named in a warning, not silently dropped: {:?}",
            ingested.warnings
        );
    }

    #[test]
    fn a_policy_level_odrl_action_is_pushed_down_onto_a_rule_naming_none_of_its_own() {
        // Information Model §2.7.1 "Compact Policy": shared properties MAY
        // include "One or many `action` properties" declared once at the
        // Policy level, with the normative processing step "Replicate these
        // properties in the Rule" for every Rule lacking its own -- the same
        // pushdown `policy_target` above already gets, now given to
        // `odrl:action` too.
        //
        // This is the spec's own Example 28, verbatim (`target`, `assigner`
        // and `action` stated once at the Policy level; each `permission`
        // entry carries only its own `assignee`). Before this fix,
        // `ingest_policy` failed this exact document outright with
        // `IngestError::RuleWithoutAction("permission")`.
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Policy",
          "uid": "http://example.com/policy:8888",
          "target": "http://example.com/music/1999.mp3",
          "assigner": "http://example.com/org/sony-music",
          "action": "play",
          "permission": [
            { "assignee": "http://example.com/people/billie" },
            { "assignee": "http://example.com/people/murphy" }
          ]
        }"#;
        let ingested = ingest_policy(doc).expect("Example 28 must ingest, not fail with RuleWithoutAction");
        assert_eq!(ingested.policy.permissions.len(), 2);
        for permission in &ingested.policy.permissions {
            assert_eq!(permission.action, "play");
            assert_eq!(permission.target.as_deref(), Some("http://example.com/music/1999.mp3"));
        }
    }

    #[test]
    fn a_rules_own_odrl_action_wins_over_a_differing_policy_level_default() {
        // The pushdown only fills in for a Rule naming none of its own --
        // exactly `policy_target`'s existing precedence, mirrored for
        // `odrl:action`.
        let doc = r#"{
          "@context": "http://www.w3.org/ns/odrl.jsonld",
          "@type": "Offer",
          "@id": "urn:uuid:action-precedence",
          "assigner": "did:web:provider.example",
          "target": "urn:asset:A",
          "action": "play",
          "permission": [{ "action": "use" }]
        }"#;
        let ingested = ingest_policy(doc).expect("must ingest");
        assert_eq!(ingested.policy.permissions[0].action, "use");
    }
}
