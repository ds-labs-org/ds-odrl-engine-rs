//! Wire request -> the three RDF inputs of ODRL-Evaluator (policy, request,
//! state of the world), serialized as one N3/Turtle document.
//!
//! Scope is the subset the Evaluator's rule set can decide. Everything
//! else is a named [`Unsupported`] so the caller can fall back to the
//! native engine instead of trusting a lossy translation.

use std::fmt::Write;

use engine::{ClaimValue, Request};
use serde_json::Value;

const NS_ODRL: &str = "http://www.w3.org/ns/odrl/2/";

/// ODRL leftOperands the Evaluator's `constraints.n3` accepts from a
/// request's `sotw:context` (its `list:in` allowlist), plus `dateTime`,
/// which it reads from `temp:currentTime` instead.
const CONTEXT_OPERANDS: &[&str] = &[
    "absolutePosition", "absoluteSize", "absoluteSpatialPosition", "absoluteTemporalPosition",
    "count", "delayPeriod", "deliveryChannel", "device", "elapsedTime", "event", "fileFormat",
    "industry", "language", "media", "meteredTime", "payAmount", "percentage", "product",
    "purpose", "recipient", "relativePosition", "relativeSize", "relativeSpatialPosition",
    "relativeTemporalPosition", "resolution", "spatial", "spatialCoordinates", "system",
    "systemDevice", "timeInterval", "unitOfCount", "version", "virtualLocation",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// `obligations`, per-rule duties/consequences/remedies: the Evaluator
    /// has no rules for them (its `ODRL-Support.md`: "No support (yet)").
    Duties,
    /// `odrl:and/or/xone/andSequence` nested constraints: not translated
    /// in this draft (the rules exist, the mapping does not).
    LogicalConstraint,
    /// `odrl:refinement` on an action: "No support yet" upstream.
    Refinement,
    ConflictStrategy,
    InheritFrom,
    AssetCollections,
    /// A constraint's leftOperand is not an ODRL context operand nor
    /// `dateTime` (the engine accepts arbitrary claim keys; RDF cannot).
    LeftOperand(String),
    /// `isAllOf` / `isPartOf`: upstream documents them as not meaningful.
    Operator(String),
    /// A rule uses `dateTime` but the request carries no `dateTime` claim.
    /// Silently substituting a clock would mis-evaluate; refuse instead.
    MissingTime,
    /// The rules count context operands in the request and only handle
    /// exactly one (`list:length 1`); two distinct referenced operands
    /// with claims present are not evaluable.
    MultipleContextOperands,
    /// A multi-valued claim referenced by a constraint.
    MultiValuedClaim(String),
    /// Some unmodelled key on a policy or rule.
    UnknownField(String),
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn iri_shaped(s: &str) -> bool {
    s.starts_with("urn:") || s.contains("://")
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// An opaque wire string (asset, party) as a named node. Deterministic, so
/// a policy's `odrl:target` and the request's compare equal exactly when
/// the engine's string comparison does.
fn node(kind: &str, s: &str) -> String {
    if iri_shaped(s) && !s.contains(['<', '>', ' ', '"']) {
        format!("<{s}>")
    } else {
        format!("<urn:n3e:{kind}:{}>", encode(s))
    }
}

fn action(s: &str) -> String {
    if iri_shaped(s) && !s.contains(['<', '>', ' ', '"']) {
        format!("<{s}>")
    } else {
        format!("<{NS_ODRL}{}>", encode(s))
    }
}

fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_date_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 19 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T'
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-'
}

/// Right operand as an RDF term. Temporal shapes are typed, integers are
/// `xsd:integer`, IRI-shaped strings are nodes, everything else a plain
/// string literal.
fn right_operand(s: &str) -> String {
    if is_date_time(s) {
        format!("{}^^<http://www.w3.org/2001/XMLSchema#dateTime>", lit(s))
    } else if is_date(s) {
        format!("{}^^<http://www.w3.org/2001/XMLSchema#date>", lit(s))
    } else if s.parse::<i64>().is_ok() {
        s.to_string()
    } else if iri_shaped(s) && !s.contains(['<', '>', ' ', '"']) {
        format!("<{s}>")
    } else {
        lit(s)
    }
}

/// The claim key that identifies the caller: the config's
/// `partyIdentityClaim`, else the `sub` claim the compliance adapter (and
/// OIDC) use. A constraint `<party claim> eq X` *is* ODRL's party
/// (`odrl:assignee`) premise, so it becomes an assignee triple, not a
/// generic constraint: the rules evaluate parties through `PartyReport`s
/// and would reject `sub` as a leftOperand.
fn is_party_constraint(c: &Value, party_key: &str) -> bool {
    c["left_operand"].as_str() == Some(party_key) && c["operator"].as_str() == Some("eq")
}

fn check_keys(v: &Value, allowed: &[&str]) -> Result<(), Unsupported> {
    if let Some(obj) = v.as_object() {
        for k in obj.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(match k.as_str() {
                    "odrl:refinement" => Unsupported::Refinement,
                    "odrl:conflict" => Unsupported::ConflictStrategy,
                    "inheritFrom" => Unsupported::InheritFrom,
                    "duties" | "duty" | "odrl:duty" | "consequence" | "odrl:consequence"
                    | "remedy" | "odrl:remedy" => Unsupported::Duties,
                    other => Unsupported::UnknownField(other.to_string()),
                });
            }
        }
    }
    Ok(())
}

pub fn to_n3(req: &Request) -> Result<String, Unsupported> {
    if !req.asset_collections.is_empty() {
        return Err(Unsupported::AssetCollections);
    }
    let cfg = serde_json::to_value(&req.config).expect("config serializes");
    let party_key = req.config.party_identity_claim.as_deref().unwrap_or("sub");
    let mut out = String::new();
    out.push_str(concat!(
        "@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n",
        "@prefix dct: <http://purl.org/dc/terms/> .\n",
        "@prefix sotw: <https://w3id.org/force/sotw#> .\n",
        "@prefix temp: <http://example.com/request/> .\n",
    ));

    // Declared action hierarchy: the engine's `includedIn` edges are the
    // request's config, so they travel with the request, not the rules.
    if let Some(actions) = cfg.get("odrl:action").and_then(Value::as_array) {
        for a in actions {
            let id = a.get("@id").and_then(Value::as_str);
            let parent = a
                .get("odrl:includedIn")
                .and_then(|p| p.get("@id"))
                .and_then(Value::as_str);
            if let (Some(id), Some(parent)) = (id, parent) {
                writeln!(out, "{} odrl:includedIn {} .", action(id), action(parent)).unwrap();
            }
        }
    }

    // Which operands do the rules reference, and is time among them?
    let mut policy_json = Vec::new();
    let mut operands: Vec<String> = Vec::new();
    for p in &req.policies {
        let pv = serde_json::to_value(p).expect("policy serializes");
        check_keys(
            &pv,
            &["id", "kind", "assigner", "assignee", "permissions", "prohibitions", "obligations"],
        )?;
        if pv["obligations"].as_array().is_some_and(|o| !o.is_empty()) {
            return Err(Unsupported::Duties);
        }
        for list in ["permissions", "prohibitions"] {
            for r in pv[list].as_array().into_iter().flatten() {
                check_keys(r, &["action", "odrl:target", "constraints"])?;
                for c in r["constraints"].as_array().into_iter().flatten() {
                    check_keys(c, &["left_operand", "operator", "right_operand"])
                        .map_err(|_| Unsupported::LogicalConstraint)?;
                    if is_party_constraint(c, party_key) {
                        continue;
                    }
                    let lo = c["left_operand"].as_str().unwrap_or_default().to_string();
                    if lo != "dateTime" && !CONTEXT_OPERANDS.contains(&lo.as_str()) {
                        return Err(Unsupported::LeftOperand(lo));
                    }
                    if matches!(c["operator"].as_str(), Some("isAllOf" | "isPartOf")) {
                        return Err(Unsupported::Operator(c["operator"].as_str().unwrap().into()));
                    }
                    if !operands.contains(&lo) {
                        operands.push(lo);
                    }
                }
            }
        }
        policy_json.push(pv);
    }

    // State of the world: `temp:currentTime` from the `dateTime` claim.
    let uses_time = operands.iter().any(|o| o == "dateTime");
    let time = match req.claims.get("dateTime") {
        Some(ClaimValue::Single(t)) => Some(t.clone()),
        Some(ClaimValue::Multi(_)) => return Err(Unsupported::MultiValuedClaim("dateTime".into())),
        None => None,
    };
    let time = match (uses_time, time) {
        (true, None) => return Err(Unsupported::MissingTime),
        // No rule reads the clock, but the report rules need *a* time to
        // stamp the PolicyReport; a fixed epoch is inert here.
        (false, None) => "1970-01-01T00:00:00Z".to_string(),
        (_, Some(t)) => t,
    };
    writeln!(
        out,
        "temp:currentTime dct:issued {}^^<http://www.w3.org/2001/XMLSchema#dateTime> .",
        lit(&time)
    )
    .unwrap();

    // Policies.
    for (pi, pv) in policy_json.iter().enumerate() {
        let pid = format!("<urn:n3e:policy:{pi}>");
        let kind = match pv["kind"].as_str() {
            Some("Offer") => "Offer",
            Some("Agreement") => "Agreement",
            _ => "Set",
        };
        writeln!(out, "{pid} a odrl:{kind} .").unwrap();
        let party_claim = req.config.party_identity_claim.as_deref();
        for (list, pred, class) in [
            ("permissions", "permission", "Permission"),
            ("prohibitions", "prohibition", "Prohibition"),
        ] {
            for (ri, r) in pv[list].as_array().into_iter().flatten().enumerate() {
                let rid = format!("<urn:n3e:policy:{pi}:{pred}:{ri}>");
                writeln!(out, "{pid} odrl:{pred} {rid} .\n{rid} a odrl:{class} ;").unwrap();
                writeln!(out, "  odrl:action {} ;", action(r["action"].as_str().unwrap_or(""))).unwrap();
                if let Some(t) = r["odrl:target"].as_str() {
                    writeln!(out, "  odrl:target {} ;", node("asset", t)).unwrap();
                }
                if let (Some(a), Some(_)) = (pv["assignee"].as_str(), party_claim) {
                    writeln!(out, "  odrl:assignee {} ;", node("party", a)).unwrap();
                }
                let parties: Vec<&str> = r["constraints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| is_party_constraint(c, party_key))
                    .filter_map(|c| c["right_operand"].as_str())
                    .collect();
                if parties.len() > 1 {
                    return Err(Unsupported::UnknownField("multiple party constraints on one rule".into()));
                }
                if let Some(p) = parties.first() {
                    writeln!(out, "  odrl:assignee {} ;", node("party", p)).unwrap();
                }
                let plain: Vec<&Value> = r["constraints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| !is_party_constraint(c, party_key))
                    .collect();
                for ci in 0..plain.len() {
                    writeln!(out, "  odrl:constraint <urn:n3e:policy:{pi}:{pred}:{ri}:c{ci}> ;").unwrap();
                }
                writeln!(out, "  .").unwrap();
                for (ci, c) in plain.iter().enumerate() {
                    let op = c["operator"].as_str().unwrap_or("eq");
                    let ro = c["right_operand"].as_str().unwrap_or("");
                    let rhs = if matches!(op, "isAnyOf" | "isNoneOf") {
                        let items: Vec<String> =
                            ro.split(',').map(|s| right_operand(s.trim())).collect();
                        format!("( {} )", items.join(" "))
                    } else {
                        right_operand(ro)
                    };
                    writeln!(
                        out,
                        "<urn:n3e:policy:{pi}:{pred}:{ri}:c{ci}> odrl:leftOperand odrl:{} ; odrl:operator odrl:{op} ; odrl:rightOperand {rhs} .",
                        c["left_operand"].as_str().unwrap_or("")
                    )
                    .unwrap();
                }
            }
        }
    }

    // The request: one `odrl:Request` with one permission for the action
    // and dataset, the caller's party (only when the config names the
    // identifying claim, like the engine), and at most one context operand.
    writeln!(out, "<urn:n3e:request> a odrl:Request ;\n  odrl:permission <urn:n3e:request:perm> .").unwrap();
    writeln!(
        out,
        "<urn:n3e:request:perm> a odrl:Permission ;\n  odrl:action {} ;\n  odrl:target {} .",
        action(&req.action),
        node("asset", &req.dataset_id)
    )
    .unwrap();
    if let Some(ClaimValue::Single(who)) = req.claims.get(party_key) {
        writeln!(out, "<urn:n3e:request:perm> odrl:assignee {} .", node("party", who)).unwrap();
    }
    let ctx: Vec<&String> = operands.iter().filter(|o| o.as_str() != "dateTime").collect();
    let ctx_with_claim: Vec<&&String> = ctx.iter().filter(|o| req.claims.contains_key(o.as_str())).collect();
    if ctx_with_claim.len() > 1 {
        return Err(Unsupported::MultipleContextOperands);
    }
    if let Some(o) = ctx_with_claim.first() {
        match &req.claims[o.as_str()] {
            ClaimValue::Single(v) => writeln!(
                out,
                "<urn:n3e:request:perm> sotw:context <urn:n3e:request:ctx> .\n<urn:n3e:request:ctx> odrl:leftOperand odrl:{o} ; odrl:rightOperand {} .",
                right_operand(v)
            )
            .unwrap(),
            ClaimValue::Multi(_) => return Err(Unsupported::MultiValuedClaim((**o).clone())),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(policy: serde_json::Value, claims: serde_json::Value) -> Request {
        serde_json::from_value(serde_json::json!({
            "dataset_id": "urn:d:1",
            "action": "read",
            "config": {"@type":"odrl:Profile","@id":"urn:p","odrl:action":[{"@id":"use"},{"@id":"read","odrl:includedIn":{"@id":"use"}}],"dutyMode":"advise"},
            "policies": [policy],
            "claims": claims,
        }))
        .unwrap()
    }

    fn policy(rule: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"id":"p1","kind":"Set","assigner":"a","assignee":null,
            "permissions":[rule],"prohibitions":[],"obligations":[]})
    }

    #[test]
    fn party_constraint_becomes_assignee_and_hierarchy_travels() {
        let r = req(
            policy(serde_json::json!({"action":"use","constraints":[{"left_operand":"sub","operator":"eq","right_operand":"alice"}]})),
            serde_json::json!({"sub":"alice"}),
        );
        let n3 = to_n3(&r).unwrap();
        assert!(n3.contains("odrl:assignee <urn:n3e:party:alice>"));
        assert!(n3.contains("<http://www.w3.org/ns/odrl/2/read> odrl:includedIn <http://www.w3.org/ns/odrl/2/use>"));
        assert!(!n3.contains("odrl:constraint"));
    }

    #[test]
    fn refuses_what_the_rules_cannot_express() {
        let c = |left: &str, op: &str| serde_json::json!({"action":"use","constraints":[{"left_operand":left,"operator":op,"right_operand":"x"}]});
        assert_eq!(to_n3(&req(policy(c("nationality", "eq")), serde_json::json!({}))), Err(Unsupported::LeftOperand("nationality".into())));
        assert_eq!(to_n3(&req(policy(c("purpose", "isPartOf")), serde_json::json!({}))), Err(Unsupported::Operator("isPartOf".into())));
        assert_eq!(to_n3(&req(policy(c("dateTime", "lt")), serde_json::json!({}))), Err(Unsupported::MissingTime));
        let mut p = policy(serde_json::json!({"action":"use","constraints":[]}));
        p["obligations"] = serde_json::json!([{"action":"notify","constraints":[]}]);
        assert_eq!(to_n3(&req(p, serde_json::json!({}))), Err(Unsupported::Duties));
        let mut p = policy(serde_json::json!({"action":"use","constraints":[]}));
        p["permissions"][0]["odrl:refinement"] = serde_json::json!({"left_operand":"purpose","operator":"eq","right_operand":"x"});
        assert_eq!(to_n3(&req(p, serde_json::json!({}))), Err(Unsupported::Refinement));
    }

    #[test]
    fn two_context_operands_with_claims_are_refused() {
        let r = req(
            policy(serde_json::json!({"action":"use","constraints":[
                {"left_operand":"purpose","operator":"eq","right_operand":"urn:x:p"},
                {"left_operand":"recipient","operator":"eq","right_operand":"urn:x:r"}]})),
            serde_json::json!({"purpose":"urn:x:p","recipient":"urn:x:r"}),
        );
        assert_eq!(to_n3(&r), Err(Unsupported::MultipleContextOperands));
    }
}
