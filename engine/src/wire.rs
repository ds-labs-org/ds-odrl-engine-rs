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

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::claims::Claims;
use crate::constraint::Operator;
use crate::decision::{decide, Decision, DecisionOutcome, Policy, Rule};
use crate::profile::{DutyMode, ResolvedConfig};

/// Section 5.2's `config` object: the host's already-resolved union of its
/// loaded profiles (Section 4.4), travelling in the request itself so the
/// engine stays stateless. Unlike `profile::Profile`, this carries no
/// `id` — a resolved config is anonymous by the time it reaches the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestConfig {
    pub recognized_actions: Vec<String>,
    pub duty_mode: DutyMode,
}

impl From<&RequestConfig> for ResolvedConfig {
    fn from(config: &RequestConfig) -> Self {
        ResolvedConfig {
            recognized_actions: config.recognized_actions.iter().cloned().collect::<HashSet<_>>(),
            duty_mode: config.duty_mode,
        }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub dataset_id: String,
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

fn describe_rule(rule: &Rule) -> String {
    if rule.constraints.is_empty() {
        return "unconstrained".to_string();
    }
    rule.constraints
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
        .join(" && ")
}

/// Reconstructs a human-readable trace of *which* rule/constraint drove
/// one policy's outcome (Section 5.2's `reason` field), by re-running the
/// same match tests `decide` used internally. `decide` itself returns only
/// the outcome, not the trace, so this walks the policy a second time in
/// the same precedence order (prohibitions, then permissions, then
/// duty-forcing) rather than threading tracing state through the decision
/// algorithm itself.
fn describe_reason(policy: &WirePolicy, outcome: &DecisionOutcome, claims: &Claims) -> String {
    match &outcome.decision {
        Decision::Error(unrecognized) => format!("policy '{}': {unrecognized}", policy.id),
        Decision::Deny => {
            if let Some((index, rule)) = policy
                .prohibitions
                .iter()
                .enumerate()
                .find(|(_, rule)| rule.matches(claims))
            {
                return format!(
                    "prohibition[{index}] of policy '{}' matched: {}",
                    policy.id,
                    describe_rule(rule)
                );
            }

            let permission_requirement_met =
                policy.permissions.is_empty() || policy.permissions.iter().any(|rule| rule.matches(claims));
            if !permission_requirement_met {
                return format!(
                    "no permission of policy '{}' matched (closed default)",
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
            match policy.permissions.iter().enumerate().find(|(_, rule)| rule.matches(claims)) {
                Some((index, rule)) => format!(
                    "permission[{index}] of policy '{}' matched: {}",
                    policy.id,
                    describe_rule(rule)
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
            outcome: decide(&policy.as_decision_policy(), &req.claims, &config),
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

    let reason = describe_reason(deciding.policy, &deciding.outcome, &req.claims);

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
      "config": {
        "recognized_actions": ["use", "distribute", "notify"],
        "duty_mode": "advise"
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
            "permission[0] of policy 'policy-1' matched: nationality eq DE"
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

    fn deny_config() -> RequestConfig {
        RequestConfig {
            recognized_actions: vec!["use".to_string(), "notify".to_string()],
            duty_mode: DutyMode::Advise,
        }
    }

    #[test]
    fn a_matching_prohibition_denies_and_names_itself_in_the_reason() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            config: deny_config(),
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
            "prohibition[0] of policy 'policy-2' matched: nationality eq US"
        );
        assert!(response.duties.is_empty());
    }

    #[test]
    fn an_unrecognized_action_yields_error_and_is_not_downgraded_by_another_allowed_policy() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            config: RequestConfig {
                recognized_actions: vec!["use".to_string()],
                duty_mode: DutyMode::Advise,
            },
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
            config: deny_config(),
            policies: vec![],
            claims: Claims::new(),
        };
        let response = evaluate_request(&req);
        assert_eq!(response.decision, WireDecision::Deny);
        assert!(response.duties.is_empty());
    }

    #[test]
    fn duty_mode_deny_forces_deny_and_suppresses_the_duties_list() {
        let req = Request {
            dataset_id: "urn:uuid:ds".to_string(),
            config: RequestConfig {
                recognized_actions: vec!["use".to_string(), "notify".to_string()],
                duty_mode: DutyMode::Deny,
            },
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
