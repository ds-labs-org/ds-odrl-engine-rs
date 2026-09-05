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
use crate::constraint::Operator;
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

fn describe_rule(rule: &Rule, requested_action: &str) -> String {
    let action_clause = if rule.action == requested_action {
        format!("action '{}'", rule.action)
    } else {
        format!("action '{}' covers requested '{requested_action}'", rule.action)
    };
    if rule.constraints.is_empty() {
        return format!("{action_clause}, unconstrained");
    }
    let constraints = rule
        .constraints
        .iter()
        .map(|c| {
            let op = match c.operator {
                Operator::Eq => "eq",
                Operator::Neq => "neq",
                Operator::IsAnyOf => "isAnyOf",
                Operator::Lt => "lt",
                Operator::Lteq => "lteq",
                Operator::Gt => "gt",
                Operator::Gteq => "gteq",
            };
            format!("{} {} {}", c.left_operand, op, c.right_operand)
        })
        .collect::<Vec<_>>()
        .join(" && ");
    format!("{action_clause}: {constraints}")
}

/// Reconstructs a human-readable trace of *which* rule/constraint drove
/// one policy's outcome (Section 5.2's `reason` field), by re-running the
/// same match tests `decide` used internally — now including the same
/// `covers_action` coverage check, so a rule that didn't apply because it
/// names an uncovering action is not mistaken for one that applied but
/// missed on constraints. `decide` itself returns only the outcome, not
/// the trace, so this walks the policy a second time in the same
/// precedence order (prohibitions, then permissions, then duty-forcing)
/// rather than threading tracing state through the decision algorithm
/// itself.
fn describe_reason(policy: &WirePolicy, outcome: &DecisionOutcome, claims: &Claims, requested_action: &str, config: &ResolvedConfig) -> String {
    let covers_and_matches = |rule: &Rule| rule.covers_action(requested_action, config) && rule.matches(claims);

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

            let permission_requirement_met =
                policy.permissions.is_empty() || policy.permissions.iter().any(covers_and_matches);
            if !permission_requirement_met {
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
            outcome: decide(&policy.as_decision_policy(), &req.claims, &config, &req.action),
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

    let reason = describe_reason(deciding.policy, &deciding.outcome, &req.claims, &req.action, &config);

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
    fn malformed_request_json_produces_a_response_not_a_panic() {
        let err = serde_json::from_str::<Request>("{ not json").unwrap_err();
        let response = parse_error_response(&err);
        assert_eq!(response.decision, WireDecision::Error);
        assert_eq!(response.dataset_id, "");
    }
}
