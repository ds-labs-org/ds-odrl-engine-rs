//! Translates one `dsc:Request` node (per `docs/vocabulary-spec.md` in the
//! vendored `ds-odrl-compliance-rdf` corpus) into the engine's real
//! `wire::Request` -- general Turtle-to-wire-JSON translation for the
//! `dsc:` vocabulary, the first time this has been built (earlier
//! verification in this project did this by hand, per-file, transcribing
//! each case's wire JSON equivalent -- this crate automates exactly that
//! transcription and then runs it for real, rather than trusting the
//! hand-transcription was faithful).
//!
//! **Why every string below goes through `graph::local_name` and nothing
//! richer.** The engine compares every one of `Rule::action`,
//! `Rule::target`, `WirePolicy::assignee`, a claim's key/value, and an
//! `odrl:rightOperand` purely as opaque strings -- it has no IRI type, no
//! namespace awareness, nothing. Correctness therefore only requires one
//! property: two occurrences of *the same* RDF resource anywhere in one
//! case file must stringify identically. Since every resource in a
//! `dsc:` case file is a `#fragment` of that one file's own `@base` (the
//! vocabulary spec's own file-shape convention, section 4.2), the
//! fragment-only local name already satisfies that uniquely within one
//! file, and a shorter string makes a failing comparison's diff far more
//! readable than a full IRI would. This is a deliberate simplification
//! specific to this one-file-per-case corpus shape; a translator over a
//! corpus that spread one request across several files/namespaces would
//! need full IRIs instead (`compliance-runner`'s own translator, over a
//! very different corpus shape, makes the same "local name is enough"
//! call at its own README's documented reasoning).
//!
//! **Explicitly not supported: by-reference `dsc:profile`.** The
//! vocabulary spec (section 4.6) allows a `dsc:profile` value that is an
//! IRI into a separate document under `profiles/`, rooted at a fixed
//! `#profile` fragment there. Resolving that would mean fetching and
//! parsing a second file this translator was not handed, which is a real
//! capability gap, not a corner worth guessing at -- so a profile IRI
//! that this file's own graph does not itself assert `a odrl:Profile` for
//! is a translation error, not a silent wrong answer. No fixture in this
//! corpus uses by-reference profiles today (confirmed by grep before
//! writing this).

use std::collections::HashMap;

use oxrdf::Term;

use engine::wire::{Request, RequestConfig, WireActionDecl, WireNodeRef, WirePolicy};
use engine::{
    Behaviour, ClaimValue, Claims, ConflictStrategy, Constraint, DutyMode, Operator, Rule,
};

use crate::graph::{local_name, odrl, Graph};
use crate::vocab::*;

/// Deserializes a small enum (`Operator`/`DutyMode`/`Behaviour`/
/// `ConflictStrategy`) from its RDF local name via the exact same
/// `#[serde(rename = "...")]` table each enum's own `Deserialize` impl
/// already carries, instead of hand-duplicating that string-to-variant
/// mapping a second time in this crate (and risking it silently drifting
/// from the engine's own the next time a variant is renamed or added).
fn from_local_name<T: serde::de::DeserializeOwned>(local: &str) -> Result<T, String> {
    serde_json::from_str(&format!("{local:?}"))
        .map_err(|e| format!("{local:?} is not a recognized value: {e}"))
}

/// A single string this translator can compare, stringified per this
/// module's own doc comment: a resource by its local name, a literal by
/// its lexical form.
fn term_string(term: &Term) -> String {
    match term {
        Term::NamedNode(n) => local_name(n.as_str()).to_string(),
        Term::BlankNode(b) => local_name(&format!("_:{}", b.as_str())).to_string(),
        Term::Literal(l) => l.value().to_string(),
    }
}

/// One `odrl:leftOperand` resolved per `docs/vocabulary-spec.md` section
/// 4.5: explicitly via a `dsc:ClaimKey` blank/named node's own `dsc:key`
/// literal, or implicitly via a native ODRL term's own local name.
fn resolve_left_operand(g: &Graph, node: &Term) -> Result<String, String> {
    let id = match node {
        Term::Literal(_) => {
            return Err("odrl:leftOperand must be a resource, not a literal".to_string())
        }
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
    };
    if g.has_type(&id, &dsc_ClaimKey()) {
        g.literal(&id, &dsc_key())
            .ok_or_else(|| format!("{id}: dsc:ClaimKey with no dsc:key"))
    } else {
        Ok(local_name(&id).to_string())
    }
}

/// One `odrl:Constraint` or `odrl:LogicalConstraint` node, recursive.
/// Logical children are read via `Graph::rdf_list` (real Turtle `( ... )`
/// list syntax, per the vocabulary spec's own `odrl:and`/`or`/`xone`
/// examples) -- never plain repeated triples, which this corpus never uses
/// for these four properties.
type ConstraintBuilder = fn(Vec<Constraint>) -> Constraint;

fn translate_constraint(g: &Graph, node: &str) -> Result<Constraint, String> {
    let logical_predicates: [(String, ConstraintBuilder); 4] = [
        (odrl_and(), Constraint::and as ConstraintBuilder),
        (odrl_or(), Constraint::or),
        (odrl_xone(), Constraint::xone),
        (odrl_andSequence(), Constraint::and_sequence),
    ];
    for (predicate, build) in logical_predicates {
        let children_ids = g.rdf_list_ids(node, &predicate);
        if !children_ids.is_empty() {
            let children = children_ids
                .iter()
                .map(|c| translate_constraint(g, c))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(build(children));
        }
    }

    let left_term = g
        .object(node, &odrl_leftOperand())
        .ok_or_else(|| format!("{node}: constraint has no odrl:leftOperand"))?;
    let left_operand = resolve_left_operand(g, left_term)?;

    let operator_id = g
        .object_id(node, &odrl_operator())
        .ok_or_else(|| format!("{node}: constraint has no odrl:operator"))?;
    let operator: Operator = from_local_name(local_name(&operator_id))?;

    let right_operand = g
        .objects(node, &odrl_rightOperand())
        .into_iter()
        .map(term_string)
        .collect::<Vec<_>>()
        .join(",");
    if right_operand.is_empty() {
        return Err(format!("{node}: constraint has no odrl:rightOperand"));
    }

    Ok(Constraint::new(left_operand, operator, right_operand))
}

/// One `odrl:action` value: either a plain action IRI, or the
/// `[ rdf:value <action> ; odrl:refinement [...] ]` wrapper shape the
/// vocabulary spec's worked example uses for a refined action. Returns
/// `(action, refinement)`.
fn translate_action(g: &Graph, rule_node: &str) -> Result<(String, Option<Constraint>), String> {
    let action_term = g
        .object(rule_node, &odrl_action())
        .ok_or_else(|| format!("{rule_node}: no odrl:action"))?;
    let action_id = match action_term {
        Term::Literal(_) => return Err(format!("{rule_node}: odrl:action must be a resource")),
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
    };
    if let Some(value_id) = g.object_id(&action_id, RDF_VALUE) {
        let refinement = g
            .object_id(&action_id, &odrl_refinement())
            .map(|r| translate_constraint(g, &r))
            .transpose()?;
        Ok((local_name(&value_id).to_string(), refinement))
    } else {
        Ok((local_name(&action_id).to_string(), None))
    }
}

/// The RDF-id shadow of one translated `Rule`, built in lockstep with it
/// by `translate_rule` below -- carries nothing `compare.rs` needs to
/// *evaluate*, only what it needs to name: the original `#fragment` id of
/// this rule and, recursively, of every nested `odrl:duty`/`odrl:remedy`/
/// `odrl:consequence` at the identical tree position the engine's own
/// `DetailedRuleReport`/`DutyAttachment` addresses by index. Kept as a
/// wholly separate tree rather than a field bolted onto `engine::Rule`
/// itself, since that type is the engine's own wire contract and has no
/// business carrying a diagnostic id this crate invented.
#[derive(Debug, Clone)]
pub struct RuleIds {
    pub rule_id: String,
    /// This rule's own translated `odrl:action` string -- carried here
    /// (rather than re-derived by a second graph lookup) so `compare.rs`
    /// can name the exact action string `wire::DutyEntry::action` will
    /// carry for this node without needing the `Graph` at compare time.
    pub action: String,
    pub duty: Vec<RuleIds>,
    pub remedy: Vec<RuleIds>,
    pub consequence: Option<Box<RuleIds>>,
}

/// One `odrl:Permission`/`odrl:Prohibition`/`odrl:Duty` node, recursively:
/// action (+ refinement), `odrl:target`, every `odrl:constraint`,
/// `odrl:duty` (Permission position), `odrl:remedy` (Prohibition
/// position), `odrl:consequence` (Duty position). Reading all three
/// regardless of which position `node` is actually used in is harmless --
/// the engine's own `decision::Rule` always carries all three fields and
/// only ever consults the one appropriate to where the `Rule` sits
/// (`Policy::permissions[].duty`, `Policy::prohibitions[].remedy`,
/// a duty's own `.consequence`) -- and keeps this one function the single
/// place a rule-shaped node is translated, rather than three near-copies.
///
/// Returns `(Rule, RuleIds)` together, built from the very same recursive
/// walk, so the id shadow can never drift out of step with the `Rule`
/// tree it names -- two separate walks over the same RDF would risk
/// silently visiting `odrl:duty`/`odrl:remedy` members in different
/// orders on some future, more adversarial fixture.
fn translate_rule(g: &Graph, node: &str) -> Result<(Rule, RuleIds), String> {
    let (action, action_refinement) = translate_action(g, node)?;
    let target = g
        .object_id(node, &odrl_target())
        .map(|t| local_name(&t).to_string());

    let constraints = g
        .object_ids(node, &odrl_constraint())
        .iter()
        .map(|c| translate_constraint(g, c))
        .collect::<Result<Vec<_>, _>>()?;

    let (duty, duty_ids): (Vec<Rule>, Vec<RuleIds>) = g
        .object_ids(node, &odrl_duty())
        .iter()
        .map(|d| translate_rule(g, d))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .unzip();
    let (remedy, remedy_ids): (Vec<Rule>, Vec<RuleIds>) = g
        .object_ids(node, &odrl_remedy())
        .iter()
        .map(|r| translate_rule(g, r))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .unzip();
    let (consequence, consequence_ids) = match g.object_id(node, &odrl_consequence()) {
        Some(c) => {
            let (rule, ids) = translate_rule(g, &c)?;
            (Some(Box::new(rule)), Some(Box::new(ids)))
        }
        None => (None, None),
    };

    let ids = RuleIds {
        rule_id: local_name(node).to_string(),
        action: action.clone(),
        duty: duty_ids,
        remedy: remedy_ids,
        consequence: consequence_ids,
    };
    let rule = Rule {
        action,
        target,
        constraints,
        action_refinement,
        duty,
        remedy,
        consequence,
    };
    Ok((rule, ids))
}

/// The seven native ODRL 2.2 Policy subclasses this corpus's
/// `dsc:policyKind` escape hatch never shadows -- checked in this fixed
/// order (irrelevant here since a policy node carries at most one, per
/// the vocabulary spec's own "never asserted alongside a native rdf:type"
/// rule, but a `Vec` keeps this a plain, greppable list rather than seven
/// near-identical `if` arms).
const NATIVE_POLICY_KINDS: &[&str] = &[
    "Set",
    "Offer",
    "Agreement",
    "Privacy",
    "Request",
    "Ticket",
    "Assertion",
];

fn translate_policy_kind(g: &Graph, node: &str) -> String {
    for kind in NATIVE_POLICY_KINDS {
        if g.has_type(node, &odrl(kind)) {
            return kind.to_string();
        }
    }
    g.literal(node, &dsc_policyKind())
        .unwrap_or_else(|| "Set".to_string())
}

/// The RDF-id shadow of one translated `WirePolicy`, mirroring `RuleIds`'
/// own reason for existing: `id` names the policy for correlation with
/// `report:policy`, and `permissions`/`prohibitions`/`obligations` are
/// this policy's own **directly-declared** rules only -- never the
/// `dsc:inheritsFrom`-merged set. `merged_policy_ids` below replicates
/// `wire::resolve_inherit_from`'s own merge order over exactly this
/// unmerged shape, the same way the engine merges the `WirePolicy` values
/// themselves before `decide` ever sees them.
#[derive(Debug, Clone)]
pub struct PolicyIds {
    pub id: String,
    pub permissions: Vec<RuleIds>,
    pub prohibitions: Vec<RuleIds>,
    pub obligations: Vec<RuleIds>,
    pub inherit_from: Option<Vec<String>>,
}

fn translate_policy(g: &Graph, node: &str) -> Result<(WirePolicy, PolicyIds), String> {
    let id = local_name(node).to_string();
    let kind = translate_policy_kind(g, node);
    let assigner = g
        .object_id(node, &odrl_assigner())
        .map(|a| local_name(&a).to_string())
        .unwrap_or_default();
    let assignee = g
        .object_id(node, &odrl_assignee())
        .map(|a| local_name(&a).to_string());

    let (permissions, permission_ids): (Vec<Rule>, Vec<RuleIds>) = g
        .object_ids(node, &odrl_permission())
        .iter()
        .map(|p| translate_rule(g, p))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .unzip();
    let (prohibitions, prohibition_ids): (Vec<Rule>, Vec<RuleIds>) = g
        .object_ids(node, &odrl_prohibition())
        .iter()
        .map(|p| translate_rule(g, p))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .unzip();
    let (obligations, obligation_ids): (Vec<Rule>, Vec<RuleIds>) = g
        .object_ids(node, &odrl_obligation())
        .iter()
        .map(|o| translate_rule(g, o))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .unzip();

    let conflict: ConflictStrategy = match g.object_id(node, &odrl_conflict()) {
        Some(c) => from_local_name(local_name(&c))?,
        None => ConflictStrategy::default(),
    };

    let parent_ids = g.rdf_list_ids(node, &dsc_inheritsFrom());
    let inherit_from: Option<Vec<String>> = if parent_ids.is_empty() {
        None
    } else {
        Some(
            parent_ids
                .iter()
                .map(|p| local_name(p).to_string())
                .collect(),
        )
    };

    let policy = WirePolicy {
        id: id.clone(),
        kind,
        assigner,
        assignee,
        permissions,
        prohibitions,
        obligations,
        conflict,
        inherit_from: inherit_from.clone(),
    };
    let ids = PolicyIds {
        id,
        permissions: permission_ids,
        prohibitions: prohibition_ids,
        obligations: obligation_ids,
        inherit_from,
    };
    Ok((policy, ids))
}

/// Replicates `wire::resolve_inherit_from`'s own merge order
/// (`engine/src/wire.rs`'s `resolve_one`: this policy's own rules first,
/// then each parent's own already-merged rules appended, in
/// `dsc:inheritsFrom` list order) over the id-only `PolicyIds` shape, so
/// `compare.rs` can name which RDF rule node a merged policy's
/// `rule_index`-addressed `DetailedRuleReport` is actually about. A
/// second, independent implementation of one merge algorithm is exactly
/// the kind of drift this workspace's own `conflicting_rules` doc comment
/// warns about -- unavoidable here, since the engine's own
/// `resolve_inherit_from` is a private `wire.rs` function this crate has
/// no access to -- so it is kept to the one property that actually
/// matters for correlation (order) rather than re-deriving anything the
/// engine itself decides (conflict forcing, party-field replication).
pub fn merged_policy_ids(all: &[PolicyIds], id: &str) -> Result<PolicyIds, String> {
    let by_id: HashMap<&str, &PolicyIds> = all.iter().map(|p| (p.id.as_str(), p)).collect();
    merge_one(id, &by_id, &mut Vec::new())
}

fn merge_one(
    id: &str,
    by_id: &HashMap<&str, &PolicyIds>,
    stack: &mut Vec<String>,
) -> Result<PolicyIds, String> {
    let policy = *by_id
        .get(id)
        .ok_or_else(|| format!("no policy '{id}' in this request's dsc:policy list"))?;
    let parent_ids: &[String] = match &policy.inherit_from {
        Some(ids) if !ids.is_empty() => ids,
        _ => return Ok(policy.clone()),
    };
    if stack.contains(&id.to_string()) {
        return Err(format!(
            "circular dsc:inheritsFrom chain: {} -> {id}",
            stack.join(" -> ")
        ));
    }
    stack.push(id.to_string());
    let mut merged = policy.clone();
    for parent_id in parent_ids {
        let parent = merge_one(parent_id, by_id, stack)?;
        merged.permissions.extend(parent.permissions);
        merged.prohibitions.extend(parent.prohibitions);
        merged.obligations.extend(parent.obligations);
    }
    stack.pop();
    Ok(merged)
}

/// `dsc:claim`: every `dsc:ClaimAssertion` the request's `dsc:claim` list
/// names, each contributing one `(key, value)` entry to the flat claims
/// map. All five shapes `docs/vocabulary-spec.md` section 4.4 documents
/// collapse to the same reading here -- identity, VC-backed, bare fact,
/// per-party fact, and array-valued -- because `dsc:aboutParty` and a
/// linking `cred:VerifiableCredential` are both documentary only (the
/// spec says so explicitly): this translator needs nothing beyond a
/// claim's own `dsc:key` and its one-or-more `rdf:value` triples.
fn translate_claims(g: &Graph, request_node: &str) -> Result<Claims, String> {
    let mut claims = HashMap::new();
    for claim_id in g.object_ids(request_node, &dsc_claim()) {
        let key = g
            .literal(&claim_id, &dsc_key())
            .ok_or_else(|| format!("{claim_id}: dsc:ClaimAssertion has no dsc:key"))?;
        let values: Vec<String> = g
            .objects(&claim_id, RDF_VALUE)
            .into_iter()
            .map(term_string)
            .collect();
        let value = match values.len() {
            0 => return Err(format!("{claim_id}: dsc:ClaimAssertion has no rdf:value")),
            1 => ClaimValue::Single(values.into_iter().next().unwrap()),
            _ => ClaimValue::Multi(values),
        };
        claims.insert(key, value);
    }
    Ok(claims)
}

/// Every action IRI referenced anywhere in the request's inline profile
/// (`odrl:action` list) plus, for each, an `odrl:includedIn` edge if one
/// exists anywhere in this same file's graph (the worked example declares
/// these as free-standing triples, e.g. `odrl:print a odrl:Action;
/// odrl:includedIn odrl:use.`, not nested under the profile node).
fn translate_config(g: &Graph, profile_node: &str) -> Result<RequestConfig, String> {
    let actions: Vec<WireActionDecl> = g
        .object_ids(profile_node, &odrl_action())
        .iter()
        .map(|a| WireActionDecl {
            id: local_name(a).to_string(),
            included_in: g.object_id(a, &odrl_includedIn()).map(|inc| WireNodeRef {
                id: local_name(&inc).to_string(),
            }),
        })
        .collect();

    let duty_mode: DutyMode = match g.object_id(profile_node, &dsc_dutyMode()) {
        Some(m) => from_local_name(&local_name(&m).to_lowercase())?,
        None => DutyMode::Advise,
    };
    let behaviour: Behaviour = match g.object_id(profile_node, &dsc_behaviour()) {
        Some(b) => from_local_name(&local_name(&b).to_lowercase())?,
        None => Behaviour::default(),
    };
    let party_identity_claim = g.literal(profile_node, &dsc_partyIdentityClaim());
    let agreement_assignee_claim = g.literal(profile_node, &dsc_agreementAssigneeClaim());

    Ok(RequestConfig {
        type_: "odrl:Profile".to_string(),
        id: format!("#{}", local_name(profile_node)),
        actions,
        duty_mode,
        behaviour,
        party_identity_claim,
        agreement_assignee_claim,
    })
}

/// Translates one `dsc:Request` node (`docs/vocabulary-spec.md` section
/// 2, 4.3) into the engine's real `wire::Request`, plus the per-policy
/// `PolicyIds` shadow `compare.rs` needs to correlate the engine's own
/// index-addressed `DetailedRuleReport`s back to the RDF rule nodes the
/// expected `report:` tree names.
pub fn translate_request(
    g: &Graph,
    request_node: &str,
) -> Result<(Request, Vec<PolicyIds>), String> {
    let target_node = g
        .object_id(request_node, &odrl_target())
        .ok_or_else(|| format!("{request_node}: no odrl:target"))?;
    let dataset_id = local_name(&target_node).to_string();

    let action_id = g
        .object_id(request_node, &odrl_action())
        .ok_or_else(|| format!("{request_node}: no odrl:action"))?;
    let action = local_name(&action_id).to_string();

    let profile_id = g
        .object_id(request_node, &dsc_profile())
        .ok_or_else(|| format!("{request_node}: no dsc:profile"))?;
    if !g.has_type(&profile_id, &odrl_Profile()) {
        return Err(format!(
            "{request_node}: dsc:profile {profile_id} is not an inline odrl:Profile in this file -- \
             by-reference profiles (docs/vocabulary-spec.md section 4.6) are not supported by this runner"
        ));
    }
    let config = translate_config(g, &profile_id)?;

    let (policies, policy_ids): (Vec<WirePolicy>, Vec<PolicyIds>) = g
        .object_ids(request_node, &dsc_policy())
        .iter()
        .map(|p| translate_policy(g, p))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .unzip();

    let claims = translate_claims(g, request_node)?;

    let asset_collections: Vec<String> = g
        .object_ids(&target_node, &odrl_partOf())
        .iter()
        .map(|c| local_name(c).to_string())
        .collect();

    Ok((
        Request {
            dataset_id,
            action,
            config,
            policies,
            claims,
            asset_collections,
        },
        policy_ids,
    ))
}
