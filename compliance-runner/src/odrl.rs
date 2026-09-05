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
    /// `odrl:PartyCollection` — the collection node's own local name.
    /// Membership is a SOTW-asserted `odrl:partOf` graph fact (the
    /// collection's `odrl:source` is not needed: the vendored suite's own
    /// `odrl:partOf` facts point directly at this same node), not a flat
    /// claim (Section 4.1) — `translate.rs` resolves it against the SOTW
    /// graph rather than the request's `claims`.
    Collection(String),
}

#[derive(Clone, Debug)]
pub enum TargetRef {
    Individual(String),
    /// `odrl:AssetCollection` — same shape as `PartyRef::Collection`, one
    /// level over on the resource side.
    Collection(String),
}

/// One atomic ODRL constraint, or a Boolean combination of them
/// (`odrl:and`/`odrl:or`) parsed into a tree. `odrl:xone` is not modeled —
/// nothing in this vendored corpus uses it (confirmed by grep across
/// `data/policies/*.ttl`), and this engine has no way to enforce the
/// "exactly one, not more" half of its semantics, only "at least one" —
/// see `Xone` below, kept as an explicit unsupported marker rather than
/// silently mis-evaluated as `Or`.
#[derive(Clone, Debug)]
pub enum ConstraintForm {
    Atomic { left_operand: String, operator: String, right_operand: String },
    And(Vec<ConstraintForm>),
    Or(Vec<ConstraintForm>),
    Xone(Vec<ConstraintForm>),
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
    /// The node id of a `odrl:duty` attached directly to *this*
    /// permission — ODRL's finer pre/post-condition form, distinct from
    /// the policy-level `odrl:duty` on the Policy node itself (Section
    /// 4.5's obligations). Carried as an id (not a bool) so `translate.rs`
    /// can look its performance state up in the SOTW graph's
    /// `report:DutyReport` fact for this same duty node.
    pub nested_duty: Option<String>,
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

/// Parses one `odrl:Constraint` or `odrl:LogicalConstraint` node,
/// recursively — a `LogicalConstraint`'s `odrl:and`/`odrl:or`/`odrl:xone`
/// children are each themselves either atomic or logical (confirmed
/// against the vendored suite's own "during business hours" fixture,
/// `policy-20.ttl`: a top-level `odrl:or` of 262 `odrl:and` groups, each
/// an `odrl:and` of two atomic `dateTime` constraints — two levels of
/// nesting in practice, but nothing here assumes a depth limit).
fn parse_constraint(g: &Graph, cnode: &str) -> ConstraintForm {
    let ty = g.type_of(cnode);
    if ty.as_deref() == Some(odrl("LogicalConstraint").as_str()) {
        let children_of = |pred: &str| -> Vec<ConstraintForm> {
            g.object_nodes(cnode, pred).iter().map(|child| parse_constraint(g, child)).collect()
        };
        let and = children_of(&odrl("and"));
        if !and.is_empty() {
            return ConstraintForm::And(and);
        }
        let or = children_of(&odrl("or"));
        if !or.is_empty() {
            return ConstraintForm::Or(or);
        }
        let xone = children_of(&odrl("xone"));
        return ConstraintForm::Xone(xone);
    }

    let left_operand =
        g.object_node(cnode, &odrl("leftOperand")).map(|id| local_name(&id).to_string()).unwrap_or_default();
    let operator = g.object_node(cnode, &odrl("operator")).map(|id| local_name(&id).to_string()).unwrap_or_default();
    let right_operand = g.object(cnode, &odrl("rightOperand")).and_then(literal_value).unwrap_or_default();
    ConstraintForm::Atomic { left_operand, operator, right_operand }
}

fn parse_rule(g: &Graph, rule_node: &str, kind: RuleKind) -> RuleInfo {
    let assignee = g.object_node(rule_node, &odrl("assignee")).map(|id| {
        if g.type_of(&id).as_deref() == Some(odrl("PartyCollection").as_str()) {
            PartyRef::Collection(local_name(&id).to_string())
        } else {
            PartyRef::Individual(local_name(&id).to_string())
        }
    });

    let action = g
        .object_node(rule_node, &odrl("action"))
        .map(|id| local_name(&id).to_string());

    let target = g.object_node(rule_node, &odrl("target")).map(|id| {
        if g.type_of(&id).as_deref() == Some(odrl("AssetCollection").as_str()) {
            TargetRef::Collection(local_name(&id).to_string())
        } else {
            TargetRef::Individual(local_name(&id).to_string())
        }
    });

    let nested_duty = g.object_node(rule_node, &odrl("duty"));

    let constraint = g.object_node(rule_node, &odrl("constraint")).map(|cnode| parse_constraint(g, &cnode));

    RuleInfo { kind, assignee, action, target, constraint, nested_duty }
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
