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
//! case file must stringify identically, and two *different* resources
//! must not. The local name gives the first for free; the second holds
//! only as long as no two IRIs a case file compares share a local name
//! across namespaces -- and case files do mix namespaces (`odrl:print`,
//! `dpv:AcademicResearch`, `did:web:...`, and the file's own `:fragment`s
//! all appear in the worked example), so `:read` beside `odrl:read`, or
//! `:Research` beside `dpv:Research`, would silently unify. That is a
//! known, undetected collision risk this translator accepts for the
//! readability of a failing diff, stated here rather than as the
//! "every resource is a #fragment" invariant an earlier version of this
//! comment claimed (it never held). A translator over a corpus that
//! spread one request across several files/namespaces would need full
//! IRIs instead (`compliance-runner`'s own translator, over a very
//! different corpus shape, makes the same "local name is enough" call at
//! its own README's documented reasoning).
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

/// Every `odrl:action` value of a rule node: each is either a plain action
/// IRI, or the `[ rdf:value <action> ; odrl:refinement [...] ]` wrapper shape
/// the vocabulary spec's worked example uses for a refined action. Returns
/// one `(action, refinement)` per value; several refinements on one wrapper
/// are ANDed, as `dsp-odrl-adapter` does.
fn translate_actions(
    g: &Graph,
    rule_node: &str,
) -> Result<Vec<(String, Option<Constraint>)>, String> {
    let terms = g.objects(rule_node, &odrl_action());
    if terms.is_empty() {
        return Err(format!("{rule_node}: no odrl:action"));
    }
    let mut out = Vec::new();
    for action_term in terms {
        let action_id = match action_term {
            Term::Literal(_) => {
                return Err(format!("{rule_node}: odrl:action must be a resource"))
            }
            Term::NamedNode(n) => n.as_str().to_string(),
            Term::BlankNode(b) => format!("_:{}", b.as_str()),
        };
        if let Some(value_id) = g.object_id(&action_id, RDF_VALUE) {
            let mut refinement: Option<Constraint> = None;
            for r in g.object_ids(&action_id, &odrl_refinement()) {
                let c = translate_constraint(g, &r)?;
                refinement = Some(match refinement {
                    Some(prev) => Constraint::and(vec![prev, c]),
                    None => c,
                });
            }
            out.push((local_name(&value_id).to_string(), refinement));
        } else {
            out.push((local_name(&action_id).to_string(), None));
        }
    }
    Ok(out)
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
fn translate_rule(g: &Graph, node: &str) -> Result<Vec<(Rule, RuleIds)>, String> {
    let actions = translate_actions(g, node)?;
    // ODRL 2.2 IM §2.7: a rule naming several actions and/or targets is one
    // atomic rule per (action, target) pair. An absent dimension is one
    // `None` variant.
    let mut targets: Vec<Option<String>> = g
        .object_ids(node, &odrl_target())
        .iter()
        .map(|t| Some(local_name(t).to_string()))
        .collect();
    if targets.is_empty() {
        targets.push(None);
    }

    let constraints = g
        .object_ids(node, &odrl_constraint())
        .iter()
        .map(|c| translate_constraint(g, c))
        .collect::<Result<Vec<_>, _>>()?;

    let (duty, duty_ids) = translate_rules_at(g, node, &odrl_duty())?;
    let (remedy, remedy_ids) = translate_rules_at(g, node, &odrl_remedy())?;

    // `engine::Rule::consequence` holds a single successor, so a composite
    // consequence (several nodes, or one node expanding to several atomic
    // rules) has nowhere to go: fail loudly rather than keep the first.
    let consequence_nodes = g.object_ids(node, &odrl_consequence());
    let (consequence, consequence_ids) = match consequence_nodes.as_slice() {
        [] => (None, None),
        [c] => {
            let mut expanded = translate_rule(g, c)?;
            if expanded.len() != 1 {
                return Err(format!(
                    "{c}: composite odrl:consequence expands to {} atomic rules; \
                     engine::Rule::consequence models a single successor",
                    expanded.len()
                ));
            }
            let (rule, ids) = expanded.remove(0);
            (Some(Box::new(rule)), Some(Box::new(ids)))
        }
        _ => {
            return Err(format!(
                "{node}: {} odrl:consequence values; engine::Rule::consequence models a single successor",
                consequence_nodes.len()
            ))
        }
    };

    let mut out = Vec::new();
    for (action, action_refinement) in &actions {
        for target in &targets {
            let ids = RuleIds {
                rule_id: local_name(node).to_string(),
                action: action.clone(),
                duty: duty_ids.clone(),
                remedy: remedy_ids.clone(),
                consequence: consequence_ids.clone(),
            };
            let rule = Rule {
                action: action.clone(),
                target: target.clone(),
                constraints: constraints.clone(),
                action_refinement: action_refinement.clone(),
                duty: duty.clone(),
                remedy: remedy.clone(),
                consequence: consequence.clone(),
            };
            out.push((rule, ids));
        }
    }
    Ok(out)
}

/// Translates every rule node under `property`, flattening composite
/// expansions, and splits the result into the rules and their id shadow.
fn translate_rules_at(
    g: &Graph,
    node: &str,
    property: &str,
) -> Result<(Vec<Rule>, Vec<RuleIds>), String> {
    let mut rules = Vec::new();
    let mut ids = Vec::new();
    for child in g.object_ids(node, property) {
        for (r, i) in translate_rule(g, &child)? {
            rules.push(r);
            ids.push(i);
        }
    }
    Ok((rules, ids))
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
    let single_party = |property: &str| -> Result<Option<String>, String> {
        let parties = g.object_ids(node, property);
        if parties.len() > 1 {
            return Err(format!(
                "{node}: {} values for {property}; WirePolicy carries a single party",
                parties.len()
            ));
        }
        Ok(parties.first().map(|a| local_name(a).to_string()))
    };
    let assigner = single_party(&odrl_assigner())?.unwrap_or_default();
    let assignee = single_party(&odrl_assignee())?;

    let (permissions, permission_ids) = translate_rules_at(g, node, &odrl_permission())?;
    let (prohibitions, prohibition_ids) = translate_rules_at(g, node, &odrl_prohibition())?;
    let (obligations, obligation_ids) = translate_rules_at(g, node, &odrl_obligation())?;

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
/// (`engine/src/wire.rs`'s `ancestor_order`: this policy's own rules
/// first, then each *distinct* ancestor's own declared rules appended
/// exactly once, in depth-first preorder over the `dsc:inheritsFrom`
/// lists -- so a diamond's shared grandparent arrives once, not once per
/// path) over the id-only `PolicyIds` shape, so `compare.rs` can name
/// which RDF rule node a merged policy's `rule_index`-addressed
/// `DetailedRuleReport` is actually about. A second, independent
/// implementation of one merge algorithm is exactly the kind of drift
/// this workspace's own `conflicting_rules` doc comment warns about --
/// unavoidable here, since the engine's own `resolve_inherit_from` is a
/// private `wire.rs` function this crate has no access to -- so it is
/// kept to the one property that actually matters for correlation
/// (order) rather than re-deriving anything the engine itself decides
/// (conflict forcing, party-field replication), and pinned by
/// `merged_policy_ids_replicates_a_diamonds_shared_grandparent_exactly_once`
/// against the engine's own
/// `a_diamond_inherit_from_replicates_a_shared_grandparents_rules_exactly_once`.
pub fn merged_policy_ids(all: &[PolicyIds], id: &str) -> Result<PolicyIds, String> {
    let by_id: HashMap<&str, &PolicyIds> = all.iter().map(|p| (p.id.as_str(), p)).collect();
    let policy = *by_id
        .get(id)
        .ok_or_else(|| format!("no policy '{id}' in this request's dsc:policy list"))?;
    let mut merged = policy.clone();
    for ancestor_id in ancestor_order(id, &by_id, &mut Vec::new())? {
        let ancestor = by_id[ancestor_id.as_str()];
        merged
            .permissions
            .extend(ancestor.permissions.iter().cloned());
        merged
            .prohibitions
            .extend(ancestor.prohibitions.iter().cloned());
        merged
            .obligations
            .extend(ancestor.obligations.iter().cloned());
    }
    Ok(merged)
}

/// The distinct ancestor ids of `id` in depth-first preorder, first
/// occurrence kept -- the engine's `wire::ancestor_order`, over `PolicyIds`.
fn ancestor_order(
    id: &str,
    by_id: &HashMap<&str, &PolicyIds>,
    stack: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    if stack.contains(&id.to_string()) {
        return Err(format!(
            "circular dsc:inheritsFrom chain: {} -> {id}",
            stack.join(" -> ")
        ));
    }
    let policy = *by_id
        .get(id)
        .ok_or_else(|| format!("no policy '{id}' in this request's dsc:policy list"))?;
    let parent_ids: &[String] = match &policy.inherit_from {
        Some(ids) if !ids.is_empty() => ids,
        _ => return Ok(Vec::new()),
    };
    stack.push(id.to_string());
    let mut order: Vec<String> = Vec::new();
    for parent_id in parent_ids {
        let parent_ancestors = ancestor_order(parent_id, by_id, stack)?;
        for ancestor in std::iter::once(parent_id.clone()).chain(parent_ancestors) {
            if !order.contains(&ancestor) {
                order.push(ancestor);
            }
        }
    }
    stack.pop();
    Ok(order)
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
        // The spec's array-valued shape is ONE assertion with several
        // `rdf:value` triples (section 4.4, shape 5). Two assertions
        // sharing a key are unspecified, and silently keeping whichever
        // came last in file order would translate the request into
        // something its author never wrote -- and blame the engine.
        if claims.contains_key(&key) {
            return Err(format!(
                "{claim_id}: a second dsc:ClaimAssertion for dsc:key {key:?} -- a multi-valued \
                 claim is one assertion carrying several rdf:value triples, not several assertions"
            ));
        }
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

    // `wire::RequestConfig::duty_mode` is a required wire field, but most
    // fixtures in this corpus state no `dsc:dutyMode` (the vocabulary spec
    // names no default either). Rather than a constant this crate made
    // up, the fallback for both knobs is the engine's own answer to
    // "nothing loaded said anything about it" -- `engine::resolve(&[])`,
    // whose fold starts at the least strict value of each axis -- so the
    // corpus's absent-means-advise/open convention is the engine's, not
    // this translator's, and a change to either shows up in
    // `a_profile_that_omits_dsc_duty_mode_gets_the_engines_own_no_profile_default`.
    let no_profile_default = engine::resolve(&[]);
    let duty_mode: DutyMode = match g.object_id(profile_node, &dsc_dutyMode()) {
        Some(m) => from_local_name(&local_name(&m).to_lowercase())?,
        None => no_profile_default.duty_mode,
    };
    let behaviour: Behaviour = match g.object_id(profile_node, &dsc_behaviour()) {
        Some(b) => from_local_name(&local_name(&b).to_lowercase())?,
        None => no_profile_default.behaviour,
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
    let target_nodes = g.object_ids(request_node, &odrl_target());
    if target_nodes.len() > 1 {
        return Err(format!(
            "{request_node}: a request is one atomic target, found {} odrl:target values",
            target_nodes.len()
        ));
    }
    let target_node = target_nodes
        .into_iter()
        .next()
        .ok_or_else(|| format!("{request_node}: no odrl:target"))?;
    let dataset_id = local_name(&target_node).to_string();

    let action_ids = g.object_ids(request_node, &odrl_action());
    if action_ids.len() > 1 {
        return Err(format!(
            "{request_node}: a request is one atomic action, found {} odrl:action values",
            action_ids.len()
        ));
    }
    let action_id = action_ids
        .into_iter()
        .next()
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

#[cfg(test)]
mod tests {
    use super::*;

    const PREAMBLE: &str = r#"
@base <https://ds-labs-org.github.io/ds-odrl-compliance-rdf/cases/unit> .
@prefix :      <#> .
@prefix dsc:   <https://ds-labs-org.github.io/ds-odrl-compliance-rdf/ns#> .
@prefix odrl:  <http://www.w3.org/ns/odrl/2/> .
@prefix rdf:   <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:asset a odrl:Asset .
:perm a odrl:Permission ; odrl:target :asset ; odrl:action odrl:read .
:policy-a a odrl:Set ; odrl:permission :perm .
"#;

    fn graph(body: &str) -> Graph {
        Graph::from_turtle(format!("{PREAMBLE}\n{body}").as_bytes()).expect("valid turtle")
    }

    fn request_id() -> String {
        "https://ds-labs-org.github.io/ds-odrl-compliance-rdf/cases/unit#request".to_string()
    }

    #[test]
    fn a_profile_that_omits_dsc_duty_mode_gets_the_engines_own_no_profile_default() {
        // `wire::RequestConfig::duty_mode` is a required wire field, so
        // something has to supply one for a profile node that states
        // none -- and 7 of the 10 fixtures in the vendored corpus state
        // none, so "error out" is not available without breaking the
        // corpus this crate exists to run. The default must then be the
        // *engine's* own, not a constant this crate made up: `engine::
        // resolve(&[])` -- "nothing loaded said anything about it" -- is
        // the one place the engine already answers exactly this question,
        // and it answers `Advise`/`Open`. Pinned here so a future change
        // to that fold (or a silent crate-local override) shows up.
        let g = graph(
            r#"
:profile a odrl:Profile ; odrl:action odrl:read .
:request a dsc:Request ; odrl:target :asset ; odrl:action odrl:read ;
    dsc:profile :profile ; dsc:policy :policy-a .
"#,
        );
        let (request, _) = translate_request(&g, &request_id()).expect("translates");
        let engine_default = engine::resolve(&[]);
        assert_eq!(request.config.duty_mode, engine_default.duty_mode);
        assert_eq!(request.config.behaviour, engine_default.behaviour);
        assert_eq!(request.config.duty_mode, DutyMode::Advise);
    }

    #[test]
    fn two_claim_assertions_with_the_same_key_are_a_translation_error_not_last_wins() {
        // The spec's array-valued shape (section 4.4, shape 5) is ONE
        // `dsc:ClaimAssertion` with several `rdf:value` triples. Two
        // assertions sharing a `dsc:key` are unspecified, and the old
        // `HashMap::insert` silently kept whichever came last in file
        // order -- so an author who meant {PI, reviewer} got "reviewer"
        // alone, and a failing comparison blamed the engine.
        let g = graph(
            r#"
:profile a odrl:Profile ; odrl:action odrl:read ; dsc:dutyMode dsc:Advise .
:c1 a dsc:ClaimAssertion ; dsc:key "roles" ; rdf:value "PI" .
:c2 a dsc:ClaimAssertion ; dsc:key "roles" ; rdf:value "reviewer" .
:request a dsc:Request ; odrl:target :asset ; odrl:action odrl:read ;
    dsc:profile :profile ; dsc:policy :policy-a ; dsc:claim :c1, :c2 .
"#,
        );
        let err = translate_request(&g, &request_id()).expect_err("must not translate");
        assert!(err.contains("roles"), "{err}");
    }

    #[test]
    fn merged_policy_ids_replicates_a_diamonds_shared_grandparent_exactly_once() {
        // Mirrors `wire::resolve_inherit_from`'s (fixed) set semantics
        // over ancestors: the engine merges each distinct ancestor's own
        // rules once, in depth-first preorder, so this id shadow must
        // produce the same `rule_index` layout or every diamond fixture
        // would correlate rule reports to the wrong RDF nodes.
        let rule = |id: &str| RuleIds {
            rule_id: id.to_string(),
            action: "read".to_string(),
            duty: vec![],
            remedy: vec![],
            consequence: None,
        };
        let policy = |id: &str, perms: &[&str], parents: Option<&[&str]>| PolicyIds {
            id: id.to_string(),
            permissions: perms.iter().map(|p| rule(p)).collect(),
            prohibitions: vec![],
            obligations: vec![],
            inherit_from: parents.map(|p| p.iter().map(|s| s.to_string()).collect()),
        };
        let all = vec![
            policy("g", &["perm-g"], None),
            policy("p1", &["perm-p1"], Some(&["g"])),
            policy("p2", &["perm-p2"], Some(&["g"])),
            policy("c", &["perm-c"], Some(&["p1", "p2"])),
        ];
        let merged = merged_policy_ids(&all, "c").unwrap();
        let ids: Vec<&str> = merged
            .permissions
            .iter()
            .map(|r| r.rule_id.as_str())
            .collect();
        assert_eq!(ids, ["perm-c", "perm-p1", "perm-g", "perm-p2"]);
    }

    #[test]
    fn a_prohibition_naming_two_actions_translates_into_two_atomic_rules() {
        // ODRL 2.2 Information Model §2.7: a rule naming several actions is
        // one atomic rule per action -- what dsp-odrl-adapter's N5 pass
        // (`expand_one_rule`) already does. `translate_action` reads only
        // the first `odrl:action` value (`Graph::object`), and RDF objects
        // have no order, so one of these two prohibitions silently vanishes
        // and a request for it is allowed (#5). The id shadow must split in
        // step, or compare.rs cannot correlate the two rule reports.
        let g = graph(
            r#"
:proh a odrl:Prohibition ; odrl:target :asset ; odrl:action odrl:archive, odrl:index .
:policy-b a odrl:Set ; odrl:prohibition :proh .
:profile a odrl:Profile ; odrl:action odrl:archive, odrl:index .
:request a dsc:Request ; odrl:target :asset ; odrl:action odrl:index ;
    dsc:profile :profile ; dsc:policy :policy-b .
"#,
        );
        let (request, policy_ids) = translate_request(&g, &request_id()).expect("translates");
        let mut actions: Vec<&str> = request.policies[0]
            .prohibitions
            .iter()
            .map(|r| r.action.as_str())
            .collect();
        actions.sort();
        assert_eq!(actions, ["archive", "index"]);
        assert_eq!(policy_ids[0].prohibitions.len(), 2);
    }

    #[test]
    fn a_permission_naming_two_targets_translates_into_two_atomic_rules() {
        // Same composite-rule rule (§2.7) on the other axis: `translate_rule`
        // reads only the first `odrl:target` (`Graph::object_id`), so one
        // asset silently loses its permission (#5).
        let g = graph(
            r#"
:other a odrl:Asset .
:perm-two a odrl:Permission ; odrl:target :asset, :other ; odrl:action odrl:read .
:policy-b a odrl:Set ; odrl:permission :perm-two .
:profile a odrl:Profile ; odrl:action odrl:read .
:request a dsc:Request ; odrl:target :other ; odrl:action odrl:read ;
    dsc:profile :profile ; dsc:policy :policy-b .
"#,
        );
        let (request, policy_ids) = translate_request(&g, &request_id()).expect("translates");
        let mut targets: Vec<Option<&str>> = request.policies[0]
            .permissions
            .iter()
            .map(|r| r.target.as_deref())
            .collect();
        targets.sort();
        assert_eq!(targets, [Some("asset"), Some("other")]);
        assert_eq!(policy_ids[0].permissions.len(), 2);
    }

    #[test]
    fn several_refinements_on_one_action_are_anded_not_first_wins() {
        let g = graph(
            r#"
:perm-r a odrl:Permission ; odrl:target :asset ;
    odrl:action [ rdf:value odrl:read ;
        odrl:refinement [ odrl:leftOperand odrl:count ; odrl:operator odrl:lt ; odrl:rightOperand 5 ] ,
                        [ odrl:leftOperand odrl:count ; odrl:operator odrl:gt ; odrl:rightOperand 1 ] ] .
:policy-b a odrl:Set ; odrl:permission :perm-r .
:profile a odrl:Profile ; odrl:action odrl:read .
:request a dsc:Request ; odrl:target :asset ; odrl:action odrl:read ;
    dsc:profile :profile ; dsc:policy :policy-b .
"#,
        );
        let (request, _) = translate_request(&g, &request_id()).expect("translates");
        let refinement = format!("{:?}", request.policies[0].permissions[0].action_refinement);
        assert!(refinement.contains("\"5\"") && refinement.contains("\"1\""), "{refinement}");
    }

    #[test]
    fn a_request_naming_two_targets_or_two_actions_is_an_error() {
        let two_targets = graph(
            r#"
:other a odrl:Asset .
:profile a odrl:Profile ; odrl:action odrl:read .
:request a dsc:Request ; odrl:target :asset, :other ; odrl:action odrl:read ;
    dsc:profile :profile ; dsc:policy :policy-a .
"#,
        );
        assert!(translate_request(&two_targets, &request_id()).is_err());
        let two_actions = graph(
            r#"
:profile a odrl:Profile ; odrl:action odrl:read, odrl:index .
:request a dsc:Request ; odrl:target :asset ; odrl:action odrl:read, odrl:index ;
    dsc:profile :profile ; dsc:policy :policy-a .
"#,
        );
        assert!(translate_request(&two_actions, &request_id()).is_err());
    }

    #[test]
    fn a_policy_naming_two_assigners_or_two_assignees_is_an_error() {
        for prop in ["odrl:assigner", "odrl:assignee"] {
            let g = graph(&format!(
                r#"
:policy-b a odrl:Set ; odrl:permission :perm ; {prop} :alice, :bob .
:profile a odrl:Profile ; odrl:action odrl:read .
:request a dsc:Request ; odrl:target :asset ; odrl:action odrl:read ;
    dsc:profile :profile ; dsc:policy :policy-b .
"#
            ));
            assert!(translate_request(&g, &request_id()).is_err(), "{prop}");
        }
    }
}
