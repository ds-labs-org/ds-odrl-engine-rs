//! Structured views over one `request.ttl` and one `policy.ttl` fixture,
//! read directly off the parsed `oxrdf` triples (see `graph.rs`) — no
//! intermediate string-templating, per the task's own instruction to work
//! from `oxrdf`/`oxttl` types throughout rather than converting to
//! strings early.

use oxrdf::Term;

use crate::graph::{local_name, odrl, Graph};

pub struct RequestInfo {
    pub assignee: String,
    pub action: String,
    pub target: Option<String>,
}

pub fn parse_request(g: &Graph) -> Result<RequestInfo, String> {
    let req_node = g
        .subject_with_any_type(&[odrl("Request")])
        .ok_or("no odrl:Request node")?;
    let permission = g
        .object_node(&req_node, &odrl("permission"))
        .ok_or("odrl:Request has no odrl:permission")?;
    let assignee = g
        .object_node(&permission, &odrl("assignee"))
        .map(|id| local_name(&id).to_string())
        .ok_or("request's odrl:permission has no odrl:assignee")?;
    let action = g
        .object_node(&permission, &odrl("action"))
        .map(|id| local_name(&id).to_string())
        .ok_or("request's odrl:permission has no odrl:action")?;
    let target = g
        .object_node(&permission, &odrl("target"))
        .map(|id| local_name(&id).to_string());
    Ok(RequestInfo { assignee, action, target })
}

#[derive(Clone, Debug)]
pub enum PartyRef {
    Individual(String),
    /// `odrl:PartyCollection` — membership is a SOTW-asserted
    /// `odrl:partOf` graph fact, not a flat claim (Section 4.1).
    Collection,
}

#[derive(Clone, Debug)]
pub enum TargetRef {
    Individual(String),
    /// `odrl:AssetCollection` — same representability problem as
    /// `PartyRef::Collection`, one level over on the resource side.
    Collection,
}

#[derive(Clone, Debug)]
pub enum ConstraintForm {
    Atomic {
        left_operand: String,
        operator: String,
        right_operand: String,
    },
    /// `odrl:LogicalConstraint` (`odrl:and`/`odrl:or`/`odrl:xone`).
    Logical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Permission,
    Prohibition,
}

pub struct RuleInfo {
    pub kind: RuleKind,
    pub assignee: Option<PartyRef>,
    pub action: Option<String>,
    pub target: Option<TargetRef>,
    pub constraint: Option<ConstraintForm>,
    /// A `odrl:duty` attached directly to *this* permission — ODRL's
    /// finer pre/post-condition form, distinct from the policy-level
    /// `odrl:duty` on the Policy node itself (Section 4.5's obligations).
    pub has_nested_duty: bool,
}

pub struct PolicyInfo {
    pub id: String,
    pub rules: Vec<RuleInfo>,
}

fn literal_value(term: &Term) -> Option<String> {
    match term {
        Term::Literal(l) => Some(l.value().to_string()),
        _ => None,
    }
}

fn parse_rule(g: &Graph, rule_node: &str, kind: RuleKind) -> RuleInfo {
    let assignee = g.object_node(rule_node, &odrl("assignee")).map(|id| {
        if g.type_of(&id).as_deref() == Some(odrl("PartyCollection").as_str()) {
            PartyRef::Collection
        } else {
            PartyRef::Individual(local_name(&id).to_string())
        }
    });

    let action = g
        .object_node(rule_node, &odrl("action"))
        .map(|id| local_name(&id).to_string());

    let target = g.object_node(rule_node, &odrl("target")).map(|id| {
        if g.type_of(&id).as_deref() == Some(odrl("AssetCollection").as_str()) {
            TargetRef::Collection
        } else {
            TargetRef::Individual(local_name(&id).to_string())
        }
    });

    let has_nested_duty = g.object_node(rule_node, &odrl("duty")).is_some();

    let constraint = g.object_node(rule_node, &odrl("constraint")).map(|cnode| {
        if g.type_of(&cnode).as_deref() == Some(odrl("LogicalConstraint").as_str()) {
            ConstraintForm::Logical
        } else {
            let left_operand = g
                .object_node(&cnode, &odrl("leftOperand"))
                .map(|id| local_name(&id).to_string())
                .unwrap_or_default();
            let operator = g
                .object_node(&cnode, &odrl("operator"))
                .map(|id| local_name(&id).to_string())
                .unwrap_or_default();
            let right_operand = g
                .object(&cnode, &odrl("rightOperand"))
                .and_then(literal_value)
                .unwrap_or_default();
            ConstraintForm::Atomic { left_operand, operator, right_operand }
        }
    });

    RuleInfo { kind, assignee, action, target, constraint, has_nested_duty }
}

pub fn parse_policy(g: &Graph) -> Result<PolicyInfo, String> {
    let policy_node = g
        .subject_with_any_type(&[odrl("Set"), odrl("Offer"), odrl("Agreement")])
        .ok_or("no odrl:Set/Offer/Agreement node")?;

    let mut rules = Vec::new();
    for permission_node in g.object_nodes(&policy_node, &odrl("permission")) {
        rules.push(parse_rule(g, &permission_node, RuleKind::Permission));
    }
    for prohibition_node in g.object_nodes(&policy_node, &odrl("prohibition")) {
        rules.push(parse_rule(g, &prohibition_node, RuleKind::Prohibition));
    }

    Ok(PolicyInfo { id: policy_node, rules })
}
