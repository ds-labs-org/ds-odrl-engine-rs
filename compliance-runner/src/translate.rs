//! Adapts one (policy, request, state-of-the-world) fixture triple into
//! Section 5.2's JSON request contract, or determines that the fixture
//! needs a capability this engine genuinely lacks and returns a one-line,
//! cited reason instead.
//!
//! **The adapter's own translation convention, stated once here because
//! nothing in Section 5.2 specifies it:** `Request` now carries its own
//! top-level `action` — the one action the whole request is about — and
//! `engine::decide` itself only considers a permission/prohibition rule
//! when `ResolvedConfig::covers(rule.action, requested_action)` holds
//! (exact match, or a declared `odrl:includedIn` chain; see
//! `engine::profile::ResolvedConfig::covers`'s own doc comment). This
//! runner therefore no longer pre-filters a policy's rules to the
//! request's action, nor rewrites a surviving rule's `action` to equal
//! it: each translated `Rule` keeps its own action exactly as
//! `odrl.rs::parse_rule` read it off the policy document. The one
//! exception is a rule that declares **no** `odrl:action` at all (this
//! corpus has two: policy-1's "everybody can do everything" and policy-2's
//! "nobody can do anything") — ODRL's own Rule class does not make action
//! optional, so an absent one is this corpus's way of saying "any action,"
//! which this engine's wire contract has no vocabulary term for; the only
//! faithful translation is to set that one rule's `action` to the
//! request's own, so it trivially covers whatever is asked. This is a
//! stand-in for genuinely missing source data, not a re-introduction of
//! the removed action-implication special case below.
//!
//! What DOES still need translate-time work is scoping by
//! `odrl:assignee`/`odrl:target` (when present) against the request's own —
//! an `odrl:assignee` becomes a `sub eq <name>` constraint (Section 4.1's
//! `sub` claim), since `WirePolicy.assignee` itself is purely descriptive
//! and has no effect on `decide` (`as_decision_policy` drops it).
//!
//! **`odrl:target` scoping stays here even though `engine::Rule` now
//! carries its own `odrl:target`.** That field (see its doc comment in
//! `engine/src/decision.rs`) covers the individual-asset case this adapter
//! resolves in `target_matches`'s `TargetRef::Individual` arm — but not
//! its `TargetRef::Collection` arm, which asks the fixture's
//! state-of-the-world graph for `<member> odrl:partOf <collection>` facts
//! the engine never receives and has no vocabulary to express. Emitting
//! the individual half as a wire target while keeping the collection half
//! here would split one scoping rule across two layers for no gain, and
//! would rewrite every request exported to
//! `compliance/reports/latest-cases.json` — the corpus an independent host
//! re-runs. So this adapter is deliberately unchanged by that engine
//! addition, exactly as it is by the native logical constraints
//! (`Constraint::and`/`or`/`xone`) it also does not use: migrating onto
//! either is its own decision, not a side effect of the capability
//! existing.
//!
//! **Action-taxonomy coverage is now the engine's own general mechanism,
//! not a host-side special case.** Earlier revisions of this adapter
//! hand-coded one exception to exact-string action matching (a
//! `TRANSFER_CATEGORY_ACTIONS` constant and an `action_is_generic_use`
//! check) because `engine::decide` had no requested-action parameter at
//! all. That workaround is gone — `ResolvedConfig::covers` (see its doc
//! comment) resolves any *declared* `odrl:includedIn` chain, closing
//! Section 7's "action implication is not evaluated" gap for real. The
//! underlying vocabulary facts that workaround encoded are still real and
//! still worth declaring, just as actual `ActionDecl` data in
//! [`base_action_vocabulary`] rather than adapter logic: per the W3C ODRL
//! Vocabulary & Expression (<https://www.w3.org/TR/vocab-odrl/>),
//! `odrl:read` and `odrl:distribute` are formally declared "Included In:
//! use," and this fixture corpus's own expected-report ground truth
//! confirms the same holds empirically for `write` (Active/Satisfied
//! whenever a `use` rule meets a `read`/`write` request) — so
//! `base_action_vocabulary` declares `write includedIn use` on that same
//! empirical basis, not by guessing. `odrl:sell` and `odrl:give` are
//! declared "Included In: transfer," a sibling category, not a child of
//! `use` — the ground truth agrees (a `use` rule against a `sell` request
//! reports Inactive/Unsatisfied). Any other includedIn/implies chain (a
//! *profile's own* declared extensions, per Section 3.5's Profile
//! Mechanism) remains unevaluated by this fixed base vocabulary — only
//! what a real profile document declares would extend it further.
//!
//! **State-of-the-world facts become claims or membership checks, not
//! wire-contract changes.** Three more capabilities, added after the
//! above, all resolved by reading the fixture's own SOTW graph rather than
//! by extending `engine`'s Section 5.2 schema:
//!
//! - `odrl:dateTime` constraints: `engine::Operator` gained `lt`/`lteq`/
//!   `gt`/`gteq` (a real, additive Default Profile extension — see
//!   `engine::constraint`'s doc comment). This adapter resolves what "the
//!   current time" is from the SOTW's own `temp:currentTime dct:issued
//!   "..."` fact and injects it as an ordinary `dateTime` claim — the
//!   engine still just does claims-map lookups, unaware this one happens
//!   to mean "now" rather than an identity attribute.
//! - `odrl:and`/`odrl:or` logical constraints: expanded into disjunctive
//!   normal form (`to_dnf`) — an `and` becomes multiple constraints on the
//!   SAME rule (the engine's own `Rule::matches` already ANDs its
//!   `constraints` list), an `or` becomes MULTIPLE sibling rules, one per
//!   disjunct (`decide` already ORs a policy's permissions: "any one
//!   matching permission is enough, not all"). No engine change needed for
//!   either. `odrl:xone` is NOT modeled — nothing in this corpus uses it,
//!   and this engine can express "at least one" but not xone's "exactly
//!   one, not more" exclusivity, so it stays a genuine, cited skip rather
//!   than a silent (and wrong) approximation as `or`.
//! - `odrl:PartyCollection`/`odrl:AssetCollection` membership: resolved
//!   directly against the SOTW graph's own `<member> odrl:partOf
//!   <collection>` facts (`is_member_of`), the same way an
//!   `odrl:assignee`/`odrl:target` match already was for an individual —
//!   still no claim or wire-contract change, since membership is decided
//!   once, at translate time, not evaluated inside the engine.
//! - A per-permission `odrl:duty`: `engine::Rule` now models one
//!   (`Rule::duty`, resolved from the claims map like every other duty
//!   there), but this adapter deliberately does **not** go through it —
//!   what it does is narrower and stays translate-time: it reads the
//!   SOTW's own `report:DutyReport` fact for that duty node (this vendored
//!   corpus's own way of stating "here is what actually happened" for a
//!   duty) and excludes the rule entirely when `report:deonticState` is
//!   `report:Violated`. A `Fulfilled` or `NonSet` (unknown) state leaves
//!   the rule in play, matching this corpus's own expected reports. This
//!   resolves the three fixture cases that exercise it. Migrating it onto
//!   the engine's own field — which would mean minting a claim per duty
//!   node and rewriting every exported request in
//!   `compliance/reports/latest-cases.json` — is a separate deliberate
//!   decision, exactly as it is for the native logical constraints and
//!   per-rule `odrl:target` above.
//!
//! If **no** rule of a policy survives assignee/target scoping or duty
//! exclusion, the policy has nothing to say about the request under
//! evaluation and is omitted from `policies` entirely — deliberately not
//! included as an empty-rules shell, which would instead trigger
//! `decide`'s own *open* exception for a policy with zero permissions
//! (Section 4.3) and silently invert the intended "this policy doesn't
//! apply here" outcome into an unconditional Allow. Action coverage no
//! longer participates in this omission decision at all — a rule whose
//! action doesn't cover the request's still survives translation, and is
//! instead excluded by `engine::decide`'s own coverage check at
//! evaluation time.

use engine::profile::ActionDecl;
use engine::wire::WireActionDecl;
use engine::{
    ClaimValue, Claims, ConflictStrategy, Constraint, DutyMode, Operator, Request, RequestConfig, Rule, WirePolicy,
};

use crate::graph::{dct, local_name, odrl, report_ns, Graph};
use crate::odrl::{ConstraintForm, PartyRef, PolicyInfo, RequestInfo, RuleKind, TargetRef};

pub enum Translation {
    /// Boxed for the same reason `cases.rs`'s `FixtureData::Ready` already
    /// boxes its own `Request`: `Request` grew past clippy's
    /// `large_enum_variant` threshold once it gained `asset_collections`,
    /// and `Skip`'s lone `String` is by far the smaller of the two shapes.
    Ready(Box<Request>),
    Skip(String),
}

/// The `@id` this adapter's own resolved config travels under — never
/// validated by the engine (`RequestConfig`'s own doc comment: "carried
/// for shape, not validated"), just a stable, self-describing label for
/// this runner's fixed vocabulary.
const CONFIG_ID: &str = "https://ds42.org/profiles/compliance-runner";

/// This adapter's fixed base vocabulary: every action this vendored corpus
/// actually uses (`compensate`, `read`, `sell`, `use`, `write` — confirmed
/// by grepping `data/policies/*.ttl`/`data/requests/*.ttl` for every
/// `odrl:action`, not guessed) plus the `odrl:includedIn` edges this
/// module's doc comment cites and justifies. Declaring `distribute`,
/// `transfer`, and `give` too costs nothing (this corpus never uses them
/// as a rule or request action) but keeps the vocabulary fact intact and
/// citable exactly as the W3C ODRL Vocabulary states it, not narrowed to
/// only the pairs this corpus happens to exercise.
fn base_action_vocabulary() -> Vec<ActionDecl> {
    vec![
        ActionDecl::new("use"),
        ActionDecl::included_in("read", "use"),
        ActionDecl::included_in("write", "use"),
        ActionDecl::included_in("distribute", "use"),
        ActionDecl::new("transfer"),
        ActionDecl::included_in("sell", "transfer"),
        ActionDecl::included_in("give", "transfer"),
        ActionDecl::new("compensate"),
    ]
}

fn base_request_config() -> RequestConfig {
    RequestConfig {
        type_: "odrl:Profile".to_string(),
        id: CONFIG_ID.to_string(),
        actions: base_action_vocabulary().iter().map(WireActionDecl::from).collect(),
        duty_mode: DutyMode::Advise,
        // `ground_truth.rs`'s own doc comment already established this
        // suite is built against the ODRL Formal Semantics draft's closed
        // default (Section 3.6), not `engine`'s own historical Open
        // default (Section 4.3) — confirmed against exactly the fixture
        // (`testcase-014-alice-sell`) this now fixes for real. Setting
        // `Behaviour::Closed` here makes that alignment an actual engine
        // parameter instead of relying on an empty-`policies`-array side
        // effect of rule pre-filtering, which no longer exists.
        behaviour: engine::profile::Behaviour::Closed,
        // Left off deliberately, and this adapter is a large part of why
        // the capability had to be opt-in. This translation already
        // resolves `odrl:assignee` itself — per *rule*, against the SOTW
        // graph's `odrl:partOf` collection membership, mirrored into a
        // `sub` constraint (see this module's header). Switching the
        // engine's own party-role scoping on as well would layer a second,
        // coarser, policy-level assignee check on top of the one this
        // corpus's ground truth is actually stated in terms of.
        party_identity_claim: None,
    }
}

fn unsupported_operator(op: &str) -> String {
    format!(
        "constraint operator odrl:{op} has no equivalent in the Default Profile's eq/neq/isAnyOf/lt/lteq/gt/gteq set; Section 7: \"...isPartOf/range-membership beyond isAnyOf remain unimplemented in the Default Profile (Section 4.2)\"."
    )
}

fn xone_unsupported(branch_count: usize) -> String {
    format!(
        "constraint is an odrl:LogicalConstraint using odrl:xone over {branch_count} branch(es); this engine can express \"at least one\" via sibling permission rules (odrl:or) but not xone's \"exactly one, not more\" exclusivity, and nothing in this vendored corpus exercises it, so it is left genuinely unsupported rather than silently approximated as odrl:or."
    )
}

fn resolve_operator(local: &str) -> Result<Operator, String> {
    match local {
        "eq" => Ok(Operator::Eq),
        "neq" => Ok(Operator::Neq),
        "lt" => Ok(Operator::Lt),
        "lteq" => Ok(Operator::Lteq),
        "gt" => Ok(Operator::Gt),
        "gteq" => Ok(Operator::Gteq),
        other => Err(unsupported_operator(other)),
    }
}

/// Expands a (possibly Boolean-combined) constraint into disjunctive
/// normal form: an OR of AND-conjunctions, each a flat `Vec<Constraint>`
/// ready to attach to one engine `Rule`. See this module's doc comment for
/// why `and`/`or` need no engine change, and why `xone` is a hard error
/// here rather than an approximation.
fn to_dnf(form: &ConstraintForm) -> Result<Vec<Vec<Constraint>>, String> {
    match form {
        ConstraintForm::Atomic { left_operand, operator, right_operand } => {
            let op = resolve_operator(operator)?;
            Ok(vec![vec![Constraint::new(left_operand.clone(), op, right_operand.clone())]])
        }
        ConstraintForm::And(children) => {
            let mut conjunctions: Vec<Vec<Constraint>> = vec![Vec::new()];
            for child in children {
                let child_dnf = to_dnf(child)?;
                let mut expanded = Vec::with_capacity(conjunctions.len() * child_dnf.len());
                for prefix in &conjunctions {
                    for disjunct in &child_dnf {
                        let mut combined = prefix.clone();
                        combined.extend(disjunct.iter().cloned());
                        expanded.push(combined);
                    }
                }
                conjunctions = expanded;
            }
            Ok(conjunctions)
        }
        ConstraintForm::Or(children) => {
            let mut disjuncts = Vec::new();
            for child in children {
                disjuncts.extend(to_dnf(child)?);
            }
            Ok(disjuncts)
        }
        ConstraintForm::Xone(children) => Err(xone_unsupported(children.len())),
    }
}

/// Does the SOTW graph assert `<member> odrl:partOf <collection>` (by
/// local name, matching the rest of this adapter's convention)?
fn is_member_of(sotw: &Graph, member_local: &str, collection_local: &str) -> bool {
    sotw.objects_by_subject_local_name(member_local, &odrl("partOf"))
        .iter()
        .any(|object| local_name(object) == collection_local)
}

/// `true` only if the SOTW graph carries a `report:DutyReport` for this
/// duty node whose `report:deonticState` is explicitly `report:Violated`.
/// No matching report, or any other state (`Fulfilled`, `NonSet`), is
/// "not violated" — the same not-proven-guilty default this corpus's own
/// fixtures expect (Section 7's per-permission-duty limitation is
/// otherwise still real; see this module's doc comment).
fn duty_is_violated(sotw: &Graph, duty_id: &str) -> bool {
    let duty_local = local_name(duty_id);
    sotw.subjects_by_object_local_name(&report_ns("rule"), duty_local).iter().any(|report_node| {
        sotw.type_of(report_node).as_deref() == Some(report_ns("DutyReport").as_str())
            && sotw.object_node(report_node, &report_ns("deonticState")).as_deref().map(local_name)
                == Some("Violated")
    })
}

fn current_time(sotw: &Graph) -> Option<String> {
    sotw.first_literal_for_predicate(&dct("issued"))
}

pub fn translate(policy: &PolicyInfo, req: &RequestInfo, sotw: &Graph, dataset_id: &str) -> Translation {
    let mut permissions = Vec::new();
    let mut prohibitions = Vec::new();

    for rule in &policy.rules {
        // No `odrl:action` at all is this corpus's way of saying "any
        // action" (policy-1/-2's unconditional grant/deny) — see this
        // module's doc comment for why the request's own action is the
        // only faithful stand-in for genuinely missing source data, not a
        // rewrite of a rule that already names one.
        let action = rule.action.clone().unwrap_or_else(|| req.action.clone());

        let assignee_matches = match &rule.assignee {
            None => true,
            Some(PartyRef::Individual(name)) => name == &req.assignee,
            Some(PartyRef::Collection(collection)) => is_member_of(sotw, &req.assignee, collection),
        };
        let target_matches = match &rule.target {
            None => true,
            Some(TargetRef::Individual(name)) => Some(name) == req.target.as_ref(),
            Some(TargetRef::Collection(collection)) => {
                req.target.as_deref().is_some_and(|t| is_member_of(sotw, t, collection))
            }
        };
        if !(assignee_matches && target_matches) {
            continue;
        }

        if let Some(duty_id) = &rule.nested_duty {
            if duty_is_violated(sotw, duty_id) {
                continue;
            }
        }

        let mut base_constraints = Vec::new();
        match &rule.assignee {
            Some(PartyRef::Individual(name)) => {
                base_constraints.push(Constraint::new("sub", Operator::Eq, name.clone()));
            }
            Some(PartyRef::Collection(_)) => {
                base_constraints.push(Constraint::new("sub", Operator::Eq, req.assignee.clone()));
            }
            None => {}
        }

        let disjuncts = match &rule.constraint {
            None => vec![Vec::new()],
            Some(form) => match to_dnf(form) {
                Ok(d) => d,
                Err(reason) => return Translation::Skip(reason),
            },
        };

        for disjunct in disjuncts {
            let mut constraints = base_constraints.clone();
            constraints.extend(disjunct);
            let engine_rule = Rule::new(action.clone(), constraints);
            match rule.kind {
                RuleKind::Permission => permissions.push(engine_rule),
                RuleKind::Prohibition => prohibitions.push(engine_rule),
            }
        }
    }

    let policies = if permissions.is_empty() && prohibitions.is_empty() {
        Vec::new()
    } else {
        vec![WirePolicy {
            id: policy.id.clone(),
            kind: "Set".to_string(),
            assigner: "urn:uuid:unspecified-assigner".to_string(),
            assignee: None,
            permissions,
            prohibitions,
            obligations: Vec::new(),
            // The engine's `odrl:conflict` default, `invalid`, and
            // deliberately not overridden: no Turtle document in the
            // vendored suite declares `odrl:conflict` at all, and no
            // fixture policy carries a permission and a prohibition at
            // once, so no case in this corpus can reach a conflict under
            // any strategy. Declaring one here would invent a term the
            // source document does not have.
            conflict: ConflictStrategy::default(),
        }]
    };

    let mut claims: Claims = Claims::new();
    claims.insert("sub".to_string(), ClaimValue::from(req.assignee.clone()));
    if let Some(now) = current_time(sotw) {
        claims.insert("dateTime".to_string(), ClaimValue::from(now));
    }

    Translation::Ready(Box::new(Request {
        dataset_id: dataset_id.to_string(),
        action: req.action.clone(),
        config: base_request_config(),
        policies,
        claims,
        // This adapter resolves `odrl:AssetCollection` membership itself,
        // ahead of ever building this request (`is_member_of` below), by
        // rewriting a targeted rule's own scope rather than by asserting a
        // fact for `engine` to read — so this stays empty rather than
        // duplicating that resolution through the new wire channel. See
        // this crate's own README and `ds-odrl-engine-rs`'s "Per-rule
        // assets" section for why the two are not redundant.
        asset_collections: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::odrl::RuleInfo;

    fn req(assignee: &str, action: &str, target: Option<&str>) -> RequestInfo {
        RequestInfo { assignee: assignee.to_string(), action: action.to_string(), target: target.map(String::from) }
    }

    fn policy(id: &str, rules: Vec<RuleInfo>) -> PolicyInfo {
        PolicyInfo { id: id.to_string(), rules }
    }

    fn unconstrained(kind: RuleKind) -> RuleInfo {
        RuleInfo { kind, assignee: None, action: None, target: None, constraint: None, nested_duty: None }
    }

    fn allow(policy: &PolicyInfo, req: &RequestInfo, sotw: &Graph) -> engine::WireDecision {
        match translate(policy, req, sotw, "ds1") {
            Translation::Ready(wire) => engine::evaluate_request(&wire).decision,
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn unconstrained_permission_allows_any_assignee_and_action() {
        let p = policy("p1", vec![unconstrained(RuleKind::Permission)]);
        let r = req("alice", "read", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Allow);
    }

    #[test]
    fn unconstrained_prohibition_denies_any_assignee_and_action() {
        let p = policy("p2", vec![unconstrained(RuleKind::Prohibition)]);
        let r = req("bob", "sell", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Deny);
    }

    #[test]
    fn assignee_scoped_permission_denies_a_non_matching_caller_by_omitting_the_policy() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p3", vec![rule]);
        let r = req("bob", "read", None);
        match translate(&p, &r, &Graph::empty(), "ds1") {
            Translation::Ready(wire) => {
                assert!(
                    wire.policies.is_empty(),
                    "a rule scoped to a different assignee must not survive translation as an \
                     empty-permissions shell, which would trigger decide()'s own open exception"
                );
                assert_eq!(engine::evaluate_request(&wire).decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn target_scoped_permission_denies_a_different_target() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: Some(TargetRef::Individual("x".to_string())),
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p4", vec![rule]);
        let r = req("alice", "read", Some("y"));
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Deny);
    }

    #[test]
    fn use_permission_covers_a_read_request_via_the_base_vocabularys_declared_includedin() {
        // Previously this was `action_is_generic_use`, a host-side special
        // case; now it's `engine::decide`'s own coverage check against
        // `base_action_vocabulary`'s declared `read includedIn use` edge —
        // same outcome, different (general, not one-off) mechanism.
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p7", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Allow);
    }

    #[test]
    fn use_prohibition_covers_a_write_request_via_the_base_vocabularys_declared_includedin() {
        let rule = RuleInfo {
            kind: RuleKind::Prohibition,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p8", vec![rule]);
        let r = req("alice", "write", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Deny);
    }

    #[test]
    fn use_rule_still_respects_assignee_scoping_regardless_of_action_coverage() {
        let rule = RuleInfo {
            kind: RuleKind::Prohibition,
            assignee: Some(PartyRef::Individual("bob".to_string())),
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p9", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &Graph::empty()),
            engine::WireDecision::Deny,
            "assignee mismatch (bob vs alice) excludes the rule regardless of the generic 'use' action matching 'read'"
        );
    }

    #[test]
    fn use_permission_does_not_cover_a_transfer_category_action_via_engine_coverage_not_translate_time_filtering() {
        // Mirrors the vendored fixture testcase-010-alice-sell: policy-3
        // ("everybody can do use") against a `sell` request. The upstream
        // expected report is Inactive/Unsatisfied, not Active — `sell` is
        // "Included In: transfer" per the W3C ODRL Vocabulary, a sibling
        // category to `use`, not a child of it. Unlike before this
        // revision, the rule now survives translation unchanged (it is no
        // longer excluded by a translate-time action filter) and is denied
        // by `engine::decide`'s own coverage check instead.
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("use".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p10", vec![rule]);
        let r = req("alice", "sell", None);
        match translate(&p, &r, &Graph::empty(), "ds1") {
            Translation::Ready(wire) => {
                assert!(
                    !wire.policies.is_empty(),
                    "the rule keeps its own declared action ('use') and survives translation; \
                     it is engine::decide's coverage check, not translate-time filtering, that \
                     denies this request"
                );
                assert_eq!(engine::evaluate_request(&wire).decision, engine::WireDecision::Deny);
            }
            Translation::Skip(reason) => panic!("expected a translated request, got skip: {reason}"),
        }
    }

    #[test]
    fn transfer_permission_covers_a_sell_request_via_the_base_vocabularys_declared_includedin() {
        // The Section 3.5 worked example ("transfer implies give and
        // sell"), through this runner's own translation.
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("transfer".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p20", vec![rule]);
        let r = req("alice", "sell", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Allow);
    }

    #[test]
    fn a_rule_with_no_declared_action_covers_whatever_the_request_asks_for() {
        // policy-1/-2's own shape: "everybody can do everything" /
        // "nobody can do anything" declare no odrl:action on their rule at
        // all — the one case this adapter still fills in with the
        // request's own action, since the wire contract has no "any
        // action" vocabulary term.
        let p = policy("p21", vec![unconstrained(RuleKind::Permission)]);
        let r = req("alice", "compensate", None);
        assert_eq!(allow(&p, &r, &Graph::empty()), engine::WireDecision::Allow);
    }

    fn sotw_with_current_time(iso: &str) -> Graph {
        Graph::parse_str(&format!(
            r#"@prefix dct: <http://purl.org/dc/terms/>.
@prefix temp: <http://example.com/request/>.
@prefix xsd: <http://www.w3.org/2001/XMLSchema#>.
temp:currentTime dct:issued "{iso}"^^xsd:dateTime."#
        ))
        .unwrap()
    }

    #[test]
    fn datetime_atomic_constraint_is_evaluated_not_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Atomic {
                left_operand: "dateTime".to_string(),
                operator: "gt".to_string(),
                right_operand: "2024-01-01T00:00:00Z".to_string(),
            }),
            nested_duty: None,
        };
        let p = policy("p11", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-06-01T00:00:00Z")),
            engine::WireDecision::Allow
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2017-01-01T00:00:00Z")),
            engine::WireDecision::Deny
        );
    }

    #[test]
    fn and_logical_constraint_becomes_multiple_constraints_on_one_rule() {
        // "between 2024-01-01 and 2024-12-31" — mirrors policy-15/-21.
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::And(vec![
                ConstraintForm::Atomic {
                    left_operand: "dateTime".to_string(),
                    operator: "gt".to_string(),
                    right_operand: "2024-01-01T00:00:00Z".to_string(),
                },
                ConstraintForm::Atomic {
                    left_operand: "dateTime".to_string(),
                    operator: "lt".to_string(),
                    right_operand: "2024-12-31T23:59:59Z".to_string(),
                },
            ])),
            nested_duty: None,
        };
        let p = policy("p12", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(allow(&p, &r, &sotw_with_current_time("2024-06-01T00:00:00Z")), engine::WireDecision::Allow);
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2025-06-01T00:00:00Z")),
            engine::WireDecision::Deny,
            "outside the AND'd window on the far side"
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2017-06-01T00:00:00Z")),
            engine::WireDecision::Deny,
            "outside the AND'd window on the near side"
        );
    }

    #[test]
    fn or_logical_constraint_becomes_sibling_permission_rules() {
        // "9-17 on day 1, OR 9-17 on day 2" — the same shape as policy-20's
        // 262-branch "business hours in 2024", shrunk to two branches.
        let branch = |start: &str, end: &str| ConstraintForm::And(vec![
            ConstraintForm::Atomic { left_operand: "dateTime".into(), operator: "gt".into(), right_operand: start.into() },
            ConstraintForm::Atomic { left_operand: "dateTime".into(), operator: "lt".into(), right_operand: end.into() },
        ]);
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Or(vec![
                branch("2024-01-01T09:00:00Z", "2024-01-01T17:00:00Z"),
                branch("2024-01-02T09:00:00Z", "2024-01-02T17:00:00Z"),
            ])),
            nested_duty: None,
        };
        let p = policy("p13", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-01-01T11:00:00Z")),
            engine::WireDecision::Allow,
            "inside the first disjunct"
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-01-02T11:00:00Z")),
            engine::WireDecision::Allow,
            "inside the second disjunct"
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_current_time("2024-01-01T20:00:00Z")),
            engine::WireDecision::Deny,
            "inside neither disjunct"
        );
    }

    #[test]
    fn xone_logical_constraint_is_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Xone(vec![
                ConstraintForm::Atomic { left_operand: "dateTime".into(), operator: "gt".into(), right_operand: "2024-01-01T00:00:00Z".into() },
            ])),
            nested_duty: None,
        };
        let p = policy("p14", vec![rule]);
        let r = req("alice", "read", None);
        assert!(matches!(translate(&p, &r, &Graph::empty(), "ds1"), Translation::Skip(_)));
    }

    #[test]
    fn unsupported_operator_is_skipped() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: None,
            action: Some("read".to_string()),
            target: None,
            constraint: Some(ConstraintForm::Atomic {
                left_operand: "count".into(),
                operator: "isPartOf".into(),
                right_operand: "5".into(),
            }),
            nested_duty: None,
        };
        let p = policy("p15", vec![rule]);
        let r = req("alice", "read", None);
        assert!(matches!(translate(&p, &r, &Graph::empty(), "ds1"), Translation::Skip(_)));
    }

    fn sotw_with_membership(member: &str, collection: &str) -> Graph {
        Graph::parse_str(&format!(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:{member} odrl:partOf ex:{collection}."#
        ))
        .unwrap()
    }

    #[test]
    fn party_collection_assignee_resolves_via_sotw_membership() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Collection("partyCollection".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p16", vec![rule]);
        let member = req("alice", "read", None);
        assert_eq!(
            allow(&p, &member, &sotw_with_membership("alice", "partyCollection")),
            engine::WireDecision::Allow
        );
        let non_member = req("mallory", "read", None);
        assert_eq!(
            allow(&p, &non_member, &sotw_with_membership("alice", "partyCollection")),
            engine::WireDecision::Deny,
            "the SOTW graph asserts alice's membership, not mallory's"
        );
    }

    #[test]
    fn asset_collection_target_resolves_via_sotw_membership() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: Some(TargetRef::Collection("assetCollection".to_string())),
            constraint: None,
            nested_duty: None,
        };
        let p = policy("p17", vec![rule]);
        let in_collection = req("alice", "read", Some("x"));
        assert_eq!(
            allow(&p, &in_collection, &sotw_with_membership("x", "assetCollection")),
            engine::WireDecision::Allow
        );
        let outside_collection = req("alice", "read", Some("y"));
        assert_eq!(
            allow(&p, &outside_collection, &sotw_with_membership("x", "assetCollection")),
            engine::WireDecision::Deny
        );
        let no_target_named = req("alice", "read", None);
        assert_eq!(
            allow(&p, &no_target_named, &sotw_with_membership("x", "assetCollection")),
            engine::WireDecision::Deny,
            "membership cannot be checked without a target to check it for"
        );
    }

    fn sotw_with_duty_state(duty_id: &str, state: &str) -> Graph {
        Graph::parse_str(&format!(
            r#"@prefix report: <https://w3id.org/force/compliance-report#>.
@prefix ex: <http://example.org/>.
ex:report1 a report:DutyReport;
    report:rule <{duty_id}>;
    report:deonticState report:{state}."#
        ))
        .unwrap()
    }

    #[test]
    fn permission_with_a_violated_nested_duty_is_excluded() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: Some("urn:uuid:duty-1".to_string()),
        };
        let p = policy("p18", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_duty_state("urn:uuid:duty-1", "Violated")),
            engine::WireDecision::Deny
        );
    }

    fn atom(name: &str) -> ConstraintForm {
        ConstraintForm::Atomic { left_operand: name.into(), operator: "eq".into(), right_operand: "v".into() }
    }

    fn dnf_shape(form: &ConstraintForm) -> Vec<Vec<String>> {
        to_dnf(form)
            .unwrap()
            .iter()
            .map(|conjunct| conjunct.iter().map(|c| c.left_operand.clone()).collect())
            .collect()
    }

    #[test]
    fn to_dnf_distributes_an_and_over_nested_ors() {
        // (a OR b) AND (c OR d) — a shape this corpus never produces (its
        // fixtures only nest the other way around, OR-of-ANDs) but a valid
        // ODRL LogicalConstraint tree all the same. Correct DNF is the
        // full 2x2 cartesian product; dropping or duplicating a disjunct
        // here would be a silent wrong answer for any policy shaped this
        // way.
        let form = ConstraintForm::And(vec![
            ConstraintForm::Or(vec![atom("a"), atom("b")]),
            ConstraintForm::Or(vec![atom("c"), atom("d")]),
        ]);
        assert_eq!(
            dnf_shape(&form),
            vec![vec!["a", "c"], vec!["a", "d"], vec!["b", "c"], vec!["b", "d"]]
        );
    }

    #[test]
    fn to_dnf_handles_three_levels_of_alternating_nesting() {
        // (a AND (b OR c)) OR d — one level deeper than the corpus's own
        // deepest shape (policy-20's OR of two-constraint ANDs): the inner
        // OR must distribute over its sibling atom inside the AND, and the
        // outer OR must keep the lone `d` disjunct intact alongside.
        let form = ConstraintForm::Or(vec![
            ConstraintForm::And(vec![atom("a"), ConstraintForm::Or(vec![atom("b"), atom("c")])]),
            atom("d"),
        ]);
        assert_eq!(dnf_shape(&form), vec![vec!["a", "b"], vec!["a", "c"], vec!["d"]]);
    }

    #[test]
    fn to_dnf_of_an_and_of_ands_stays_one_flat_conjunction() {
        // Pure conjunctive nesting must not multiply disjuncts: exactly
        // one engine Rule should come out, carrying all four constraints.
        let form = ConstraintForm::And(vec![
            ConstraintForm::And(vec![atom("a"), atom("b")]),
            ConstraintForm::And(vec![atom("c"), atom("d")]),
        ]);
        assert_eq!(dnf_shape(&form), vec![vec!["a", "b", "c", "d"]]);
    }

    #[test]
    fn to_dnf_propagates_an_unsupported_operator_from_any_depth() {
        // The loud-skip posture must survive nesting: an unsupported
        // operator buried two levels down is still a translation error,
        // never silently dropped from the expansion.
        let form = ConstraintForm::Or(vec![
            ConstraintForm::And(vec![
                atom("a"),
                ConstraintForm::Atomic {
                    left_operand: "resource".into(),
                    operator: "isPartOf".into(),
                    right_operand: "set".into(),
                },
            ]),
            atom("d"),
        ]);
        assert!(to_dnf(&form).is_err());
    }

    #[test]
    fn permission_with_a_fulfilled_or_nonset_nested_duty_still_grants() {
        let rule = RuleInfo {
            kind: RuleKind::Permission,
            assignee: Some(PartyRef::Individual("alice".to_string())),
            action: Some("read".to_string()),
            target: None,
            constraint: None,
            nested_duty: Some("urn:uuid:duty-1".to_string()),
        };
        let p = policy("p19", vec![rule]);
        let r = req("alice", "read", None);
        assert_eq!(
            allow(&p, &r, &sotw_with_duty_state("urn:uuid:duty-1", "Fulfilled")),
            engine::WireDecision::Allow
        );
        assert_eq!(
            allow(&p, &r, &sotw_with_duty_state("urn:uuid:duty-1", "NonSet")),
            engine::WireDecision::Allow
        );
        assert_eq!(
            allow(&p, &r, &Graph::empty()),
            engine::WireDecision::Allow,
            "no DutyReport at all is not evidence of violation"
        );
    }
}
