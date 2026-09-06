//! The catalog: 52 vocabulary rows from the source gap analysis, and the
//! ~125 `evaluate()` calls that put 49 of them to the test in a browser.
//!
//! Two design rules every probe here obeys, because without them a probe
//! proves nothing:
//!
//! 1. **No positive probe stands alone.** Every "the feature works" probe
//!    is paired with a same-shape "and here is the input that makes it
//!    *not* fire" probe. Without the miss, an engine that ignored
//!    constraints entirely would sail through the hit.
//! 2. **No unknown-field probe stands alone.** Every "this ODRL property
//!    is inert" probe names a control — usually a byte-identical request
//!    without the injected key, or better, a baseline the property would
//!    have changed had it been honoured. The row's `asserts` states the
//!    pair; where only the weak form is available, the row says so.
//!
//! Requests are authored as typed `engine::Request` values (so the
//! supported half of every request is exactly Section 5.2's shape, checked
//! by the compiler) and then patched — see `patch.rs` for why unknown keys
//! can only enter that one way.

use serde_json::json;

use engine::claims::ClaimValue;
use engine::constraint::{Constraint, Operator};
use engine::profile::{Behaviour, DutyMode};
use engine::wire::{WireActionDecl, WireNodeRef};
use engine::{Claims, ConflictStrategy, Request, RequestConfig, Rule, WirePolicy};

use crate::patch::{apply_patches, Patch};
use crate::render::{Category, DutyExpect, Expect, Probe, Row};
use crate::taxonomy::taxonomy_config;

const DATASET_ID: &str = "urn:uuid:coverage-probe";
const PROFILE_ID: &str = "https://ds42.org/profiles/coverage-probe";
const ASSIGNER: &str = "did:web:provider.example";
const POSITIVE: &str = "positive";
const NEGATIVE: &str = "negative";

// ---------------------------------------------------------------------
// Request construction primitives
// ---------------------------------------------------------------------

pub fn action(id: &str) -> WireActionDecl {
    WireActionDecl { id: id.to_string(), included_in: None }
}

pub fn action_in(id: &str, parent: &str) -> WireActionDecl {
    WireActionDecl { id: id.to_string(), included_in: Some(WireNodeRef { id: parent.to_string() }) }
}

/// The shared `config`: `dutyMode: advise`, and `behaviour: closed` so a
/// constraint miss produces the unambiguous
/// `... (closed default)` reason rather than the open default's vacuous
/// Allow.
pub fn config(actions: Vec<WireActionDecl>) -> RequestConfig {
    RequestConfig {
        type_: "odrl:Profile".to_string(),
        id: PROFILE_ID.to_string(),
        actions,
        duty_mode: DutyMode::Advise,
        behaviour: Behaviour::Closed,
        party_identity_claim: None,
    }
}

pub fn flat_config(ids: &[&str]) -> RequestConfig {
    config(ids.iter().map(|id| action(id)).collect())
}

/// Builds an unrefined rule — `Rule::new`'s own shape. No probe in this
/// catalog exercises `odrl:refinement`; adding one would rewrite
/// `compliance/reports/latest-coverage.json`, which is a separate,
/// deliberate regeneration rather than a side effect of adding the
/// capability to the engine.
pub fn rule(action: &str, constraints: Vec<Constraint>) -> Rule {
    Rule::new(action, constraints)
}

/// An unconstrained rule scoped to one asset by `odrl:target` — a real,
/// modelled field on `engine::Rule`, so these probes build it as a typed
/// value like every other supported part of a request, rather than
/// injecting it as an unknown key through `patch.rs`.
pub fn targeted_rule(action: &str, target: &str) -> Rule {
    Rule::targeting(action, target, vec![])
}

pub fn policy(id: &str, permissions: Vec<Rule>) -> WirePolicy {
    WirePolicy {
        id: id.to_string(),
        kind: "Set".to_string(),
        assigner: ASSIGNER.to_string(),
        assignee: None,
        permissions,
        prohibitions: Vec::new(),
        obligations: Vec::new(),
        // Probes that need a strategy set it explicitly (category 9);
        // every other probe stays on ODRL's own default, `invalid`, which
        // is unreachable for a policy that carries no prohibition at all.
        conflict: ConflictStrategy::default(),
        // Probes that exercise odrl:inheritFrom set it explicitly (below);
        // every other probe has no parent.
        inherit_from: None,
    }
}

/// The shared base every probe varies: one `Set` policy granting `use`,
/// unconstrained, under a closed-world config declaring only `use`.
pub fn base_request() -> Request {
    Request {
        dataset_id: DATASET_ID.to_string(),
        action: "use".to_string(),
        config: flat_config(&["use"]),
        policies: vec![policy("probe", vec![rule("use", vec![])])],
        claims: Claims::new(),
        asset_collections: Vec::new(),
    }
}

/// `base_request()` with the single `use` permission carrying exactly
/// `constraints`, and the given claims.
fn constrained(constraints: Vec<Constraint>, claims: Claims) -> Request {
    let mut request = base_request();
    request.policies[0].permissions = vec![rule("use", constraints)];
    request.claims = claims;
    request
}

fn one(constraint: Constraint, claims: Claims) -> Request {
    constrained(vec![constraint], claims)
}

fn c(left: &str, operator: Operator, right: &str) -> Constraint {
    Constraint::new(left, operator, right)
}

fn s(value: &str) -> ClaimValue {
    ClaimValue::Single(value.to_string())
}

fn m(values: &[&str]) -> ClaimValue {
    ClaimValue::Multi(values.iter().map(|v| v.to_string()).collect())
}

fn claims(pairs: &[(&str, ClaimValue)]) -> Claims {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn no_claims() -> Claims {
    Claims::new()
}

// ---------------------------------------------------------------------
// Expectation construction
// ---------------------------------------------------------------------

fn expectation(decision: &'static str, contains: &[String]) -> Expect {
    Expect {
        decision,
        reason_contains: contains.to_vec(),
        reason_excludes: Vec::new(),
        duties: None,
        dataset_id: None,
    }
}

fn allow(reason: &str) -> Expect {
    expectation("Allow", &[reason.to_string()])
}

fn deny(reason: &str) -> Expect {
    expectation("Deny", &[reason.to_string()])
}

fn error(reasons: &[&str]) -> Expect {
    expectation("Error", &reasons.iter().map(|r| r.to_string()).collect::<Vec<_>>())
}

impl Expect {
    fn excluding(mut self, excludes: &[&str]) -> Self {
        self.reason_excludes = excludes.iter().map(|e| e.to_string()).collect();
        self
    }

    fn with_duties(mut self, duties: Vec<DutyExpect>) -> Self {
        self.duties = Some(duties);
        self
    }

    fn with_dataset_id(mut self, dataset_id: &str) -> Self {
        self.dataset_id = Some(dataset_id.to_string());
        self
    }
}

fn duty(action: &str) -> DutyExpect {
    DutyExpect { policy_id: "probe".to_string(), action: action.to_string(), resolved: false, source: None }
}

/// A duty entry carrying provenance — a per-permission `odrl:duty`, a
/// prohibition's `odrl:remedy`, or a consequence of either — which a plain
/// policy-level obligation never does.
fn attached_duty(action: &str, source: &str) -> DutyExpect {
    DutyExpect {
        policy_id: "probe".to_string(),
        action: action.to_string(),
        resolved: false,
        source: Some(source.to_string()),
    }
}

/// `permission[0] of policy 'probe' matched: action 'use', unconstrained`
/// — the reason a great many "this property changed nothing" probes are
/// read against.
fn base_allow_reason() -> String {
    "permission[0] of policy 'probe' matched: action 'use', unconstrained".to_string()
}

/// The unambiguous closed-world miss, for policy `probe`.
fn closed_deny(requested_action: &str) -> String {
    format!(
        "no permission of policy 'probe' covered and matched requested action '{requested_action}' (closed default)"
    )
}

/// Allow, where the reason must render the permission's own constraint —
/// the only way a probe can tell "the constraint was satisfied" apart from
/// "the constraint was never looked at".
fn allow_constrained(rendered_constraint: &str) -> Expect {
    allow(&format!("permission[0] of policy 'probe' matched: action 'use': {rendered_constraint}"))
}

// ---------------------------------------------------------------------
// Probe assembly
// ---------------------------------------------------------------------

struct Spec {
    id: &'static str,
    kind: &'static str,
    title: &'static str,
    asserts: &'static str,
    falsified_by: &'static str,
    request: Request,
    patches: Vec<Patch>,
    expect: Expect,
}

/// Serializes one probe's typed request, applies its patches, and panics
/// — loudly, naming the probe — if any patch does not land. A probe whose
/// patch silently missed would still reach its expected decision while
/// having injected nothing at all.
fn build(spec: Spec) -> Probe {
    let mut request = serde_json::to_value(&spec.request).expect("engine::Request always serializes");
    if let Err(err) = apply_patches(&mut request, &spec.patches) {
        panic!("probe `{}`: {err}", spec.id);
    }
    Probe {
        id: spec.id,
        kind: spec.kind,
        title: spec.title.to_string(),
        asserts: spec.asserts.to_string(),
        falsified_by: spec.falsified_by.to_string(),
        request,
        expect: spec.expect,
    }
}

// ---------------------------------------------------------------------
// 1 — Actions
// ---------------------------------------------------------------------

fn action_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(10);

    probes.push(build(Spec {
        id: "act-base-exact",
        kind: POSITIVE,
        title: "a permission naming exactly the requested action matches",
        asserts: "The baseline every other probe in this catalog is read against: one Set policy, one \
                  unconstrained permission for the requested action, closed-world behaviour.",
        falsified_by: "anything but Allow -- which would mean the baseline itself is broken and no other \
                       probe's reading can be trusted",
        request: base_request(),
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "act-includedin-1hop",
        kind: POSITIVE,
        title: "a permission for the broader action covers a request for its declared child",
        asserts: "The Information Model's own worked example (sell includedIn transfer): a permission for \
                  transfer covers a request for sell, and the reason says which edge it walked.",
        falsified_by: "Deny -- which would mean coverage is exact-string matching, not taxonomy resolution",
        request: Request {
            action: "sell".to_string(),
            config: config(vec![action("use"), action("transfer"), action_in("sell", "transfer")]),
            policies: vec![policy("probe", vec![rule("transfer", vec![])])],
            ..base_request()
        },
        patches: vec![],
        expect: allow("permission[0] of policy 'probe' matched: action 'transfer' covers requested 'sell', unconstrained"),
    }));

    probes.push(build(Spec {
        id: "act-includedin-2hop",
        kind: POSITIVE,
        title: "an includedIn chain resolves through more than one declared hop",
        asserts: "display includedIn play includedIn use: a permission for the top of a two-hop chain \
                  covers a request for the bottom.",
        falsified_by: "Deny -- which would mean coverage resolves one hop only",
        request: Request {
            action: "display".to_string(),
            config: config(vec![action("use"), action_in("play", "use"), action_in("display", "play")]),
            policies: vec![policy("probe", vec![rule("use", vec![])])],
            ..base_request()
        },
        patches: vec![],
        expect: allow("permission[0] of policy 'probe' matched: action 'use' covers requested 'display', unconstrained"),
    }));

    probes.push(build(Spec {
        id: "act-includedin-undeclared-gap",
        kind: NEGATIVE,
        title: "an includedIn chain through an undeclared intermediate action does not cover",
        asserts: "The same two-hop chain as act-includedin-2hop with its intermediate action (`play`) left \
                  undeclared as its own ActionDecl. Coverage traverses only edges some loaded profile \
                  declares -- a typo'd or unlisted action can never silently become covered.",
        falsified_by: "Allow -- which would mean covers() reaches through actions no profile declared",
        request: Request {
            action: "display".to_string(),
            config: config(vec![action("use"), action_in("display", "play")]),
            policies: vec![policy("probe", vec![rule("use", vec![])])],
            ..base_request()
        },
        patches: vec![],
        expect: deny(&closed_deny("display")),
    }));

    probes.push(build(Spec {
        id: "act-implies-ignored",
        kind: NEGATIVE,
        title: "odrl:implies on an action declaration is dropped",
        asserts: "Byte-identical to act-includedin-1hop except that sell's relationship to transfer is \
                  declared with `odrl:implies` instead of `odrl:includedIn`. That one is honoured (Allow), \
                  this one is not (Deny) -- the engine resolves includedIn and nothing else.",
        falsified_by: "Allow -- which would mean odrl:implies is being resolved as a coverage edge",
        request: Request {
            action: "sell".to_string(),
            config: config(vec![action("use"), action("transfer"), action("sell")]),
            policies: vec![policy("probe", vec![rule("transfer", vec![])])],
            ..base_request()
        },
        patches: vec![Patch::set("/config/odrl:action/2", "odrl:implies", json!({"@id": "transfer"}))],
        expect: deny(&closed_deny("sell")),
    }));

    let taxonomy_probe = |id: &'static str,
                          title: &'static str,
                          asserts: &'static str,
                          requested: &str,
                          permitted: &str,
                          reason: String| {
        build(Spec {
            id,
            kind: POSITIVE,
            title,
            asserts,
            falsified_by: "Deny -- which would mean the engine cannot resolve the spec's own taxonomy",
            request: Request {
                action: requested.to_string(),
                config: taxonomy_config(),
                policies: vec![policy("probe", vec![rule(permitted, vec![])])],
                ..base_request()
            },
            patches: vec![],
            expect: allow(&reason),
        })
    };

    probes.push(taxonomy_probe(
        "act-taxonomy-display-play-use",
        "the shipped 51-term ODRL 2.2 taxonomy resolves display -> play -> use",
        "One of the two chains in the whole ODRL 2.2 Common Vocabulary whose root is neither `use` nor \
         `transfer` directly, and one no hand-written config anywhere in this workspace declares. The 51 \
         action declarations in this probe's config came out of profile-interpreter parsing the spec's own \
         taxonomy; what runs here is the engine's resolution over them.",
        "display",
        "use",
        "permission[0] of policy 'probe' matched: action 'use' covers requested 'display', unconstrained".to_string(),
    ));

    probes.push(taxonomy_probe(
        "act-taxonomy-extract-reproduce-use",
        "the shipped taxonomy resolves extract -> reproduce",
        "The second non-`use`-rooted chain (odrl-vocab 4.4.20). A permission for reproduce covers a \
         request for extract.",
        "extract",
        "reproduce",
        "permission[0] of policy 'probe' matched: action 'reproduce' covers requested 'extract', unconstrained"
            .to_string(),
    ));

    probes.push(taxonomy_probe(
        "act-taxonomy-sell-transfer",
        "the shipped taxonomy resolves sell -> transfer",
        "The vocabulary's second root: a permission for transfer covers a request for sell, over the real \
         51-term taxonomy rather than a three-action config hand-written for the occasion.",
        "sell",
        "transfer",
        "permission[0] of policy 'probe' matched: action 'transfer' covers requested 'sell', unconstrained"
            .to_string(),
    ));

    probes.push(build(Spec {
        id: "act-unrecognized-error",
        kind: POSITIVE,
        title: "a rule naming an action no loaded profile declares is an Error, not a non-match",
        asserts: "The engine ships zero actions of its own: `anonymize` is a real ODRL 2.2 Common \
                  Vocabulary term, and a config that does not declare it makes any rule naming it a \
                  configuration gap rather than an ordinary miss.",
        falsified_by: "Allow or Deny -- either would mean unknown vocabulary is tolerated silently",
        request: Request {
            policies: vec![policy("probe", vec![rule("anonymize", vec![])])],
            ..base_request()
        },
        patches: vec![],
        expect: error(&["unrecognized action \"anonymize\" in permission rule at index 0"]),
    }));

    probes.push(build(Spec {
        id: "act-unrecognized-outranks-allow",
        kind: POSITIVE,
        title: "an unrecognized action in one policy out-ranks another policy's Allow",
        asserts: "Two policies: the first grants the requested action outright, the second names an \
                  undeclared action. Error beats Allow across the whole policy set, and the reason names \
                  the offending policy.",
        falsified_by: "Allow or Deny -- Error must out-rank both, or a misconfigured policy could be \
                       masked by a well-formed one beside it",
        request: Request {
            policies: vec![
                policy("probe-ok", vec![rule("use", vec![])]),
                policy("probe-bad", vec![rule("anonymize", vec![])]),
            ],
            ..base_request()
        },
        patches: vec![],
        expect: error(&["policy 'probe-bad'", "unrecognized action \"anonymize\""]),
    }));

    probes
}

// ---------------------------------------------------------------------
// 2 — Left operands
// ---------------------------------------------------------------------

fn left_operand_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(26);

    let tier = "https://example.org/ns#loyaltyTier";

    probes.push(build(Spec {
        id: "lo-extension-hit",
        kind: POSITIVE,
        title: "a profile-invented left operand IRI works as an ordinary claims key",
        asserts: "leftOperand is a free-form key into the claims map, so an extension operand needs no \
                  registration at all -- the one place this design is strictly better off than a closed \
                  enum. The reason renders the full IRI, so nothing was normalized away.",
        falsified_by: "Deny -- which would mean unknown left operands are rejected rather than looked up",
        request: one(c(tier, Operator::Eq, "gold"), claims(&[(tier, s("gold"))])),
        patches: vec![],
        expect: allow_constrained(&format!("{tier} eq gold")),
    }));

    probes.push(build(Spec {
        id: "lo-extension-miss",
        kind: NEGATIVE,
        title: "the same extension operand misses when the claim does not match",
        asserts: "The pair's other half: without it, an engine that ignored constraints entirely would \
                  pass lo-extension-hit.",
        falsified_by: "Allow",
        request: one(c(tier, Operator::Eq, "gold"), claims(&[(tier, s("silver"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-datetime-hit",
        kind: POSITIVE,
        title: "dateTime compares chronologically when the host injects the claim",
        asserts: "The engine parses both sides as xsd:dateTime and orders them.",
        falsified_by: "Deny",
        request: one(
            c("dateTime", Operator::Lt, "2027-01-01T00:00:00Z"),
            claims(&[("dateTime", s("2026-09-05T12:00:00Z"))]),
        ),
        patches: vec![],
        expect: allow_constrained("dateTime lt 2027-01-01T00:00:00Z"),
    }));

    probes.push(build(Spec {
        id: "lo-datetime-miss",
        kind: NEGATIVE,
        title: "dateTime misses on the far side of the boundary",
        asserts: "Same constraint, a claim past the bound.",
        falsified_by: "Allow",
        request: one(
            c("dateTime", Operator::Lt, "2027-01-01T00:00:00Z"),
            claims(&[("dateTime", s("2027-06-01T00:00:00Z"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-datetime-absent-no-clock",
        kind: NEGATIVE,
        title: "with no dateTime claim there is no clock to fall back on",
        asserts: "ODRL's dateTime means \"the current moment\"; this engine has no idea what that is. \
                  engine.wasm is instantiated with an EMPTY import object (engine_bridge::\
                  load_engine_instance), so this is a structural property of the artifact this page \
                  loaded, not a policy choice: there is no host function it could call for the time.",
        falsified_by: "Allow -- which would mean the guest synthesised a clock from somewhere",
        request: one(c("dateTime", Operator::Lt, "2027-01-01T00:00:00Z"), no_claims()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-count-hit",
        kind: POSITIVE,
        title: "count compares numerically against a host-supplied claim",
        asserts: "`count lteq 10` is expressible and evaluated as a number, not a string.",
        falsified_by: "Deny",
        request: one(c("count", Operator::Lteq, "10"), claims(&[("count", s("7"))])),
        patches: vec![],
        expect: allow_constrained("count lteq 10"),
    }));

    probes.push(build(Spec {
        id: "lo-count-miss",
        kind: NEGATIVE,
        title: "count misses above the bound",
        asserts: "11 is not lteq 10 -- and \"11\" < \"10\" lexically, so an Allow here would also be \
                  evidence of string comparison rather than numeric.",
        falsified_by: "Allow",
        request: one(c("count", Operator::Lteq, "10"), claims(&[("count", s("11"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-count-absent-not-stateful",
        kind: NEGATIVE,
        title: "with no count claim the engine counts nothing itself",
        asserts: "ODRL's count is a *stateful execution count*. A stateless engine keeps no history; \
                  count only ever means what a host put in the claims map.",
        falsified_by: "Allow -- which would mean the engine invented an execution count",
        request: one(c("count", Operator::Lteq, "10"), no_claims()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-count-nonnumeric-miss",
        kind: NEGATIVE,
        title: "a non-numeric count claim is a silent miss, not an error",
        asserts: "\"seven\" parses as neither a temporal value nor a number, so the ordering operators \
                  miss -- the same posture an absent key already has, not a Decision::Error.",
        falsified_by: "Allow, or Error",
        request: one(c("count", Operator::Lteq, "10"), claims(&[("count", s("seven"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-count-infinity-rejected",
        kind: NEGATIVE,
        title: "a claim of literally \"inf\" does not vacuously satisfy gt",
        asserts: "Rust's str::parse::<f64> accepts \"inf\"; without the engine's own is_finite() guard \
                  this claim would make `gt`/`gteq` match every finite bound, silently.",
        falsified_by: "Allow -- which is exactly the fail-open the is_finite() guard exists to stop",
        request: one(c("count", Operator::Gt, "0"), claims(&[("count", s("inf"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-payamount-decimal",
        kind: POSITIVE,
        title: "payAmount compares as a decimal number",
        asserts: "The numeric path is not integer-only: 5.5 gteq 5.00 holds, and lexically \"5.5\" > \
                  \"5.00\" would too -- so this is read together with lo-count-miss, whose lexical and \
                  numeric answers differ.",
        falsified_by: "Deny",
        request: one(c("payAmount", Operator::Gteq, "5.00"), claims(&[("payAmount", s("5.5"))])),
        patches: vec![],
        expect: allow_constrained("payAmount gteq 5.00"),
    }));

    probes.push(build(Spec {
        id: "lo-spatial-flat-hit",
        kind: POSITIVE,
        title: "spatial matches a flat region IRI by string equality",
        asserts: "The degenerate half of spatial support: an exact region code or IRI works.",
        falsified_by: "Deny",
        request: one(
            c("spatial", Operator::Eq, "https://www.geonames.org/2921044"),
            claims(&[("spatial", s("https://www.geonames.org/2921044"))]),
        ),
        patches: vec![],
        expect: allow_constrained("spatial eq https://www.geonames.org/2921044"),
    }));

    probes.push(build(Spec {
        id: "lo-spatial-no-containment",
        kind: NEGATIVE,
        title: "spatial has no region containment: Berlin does not match Germany",
        asserts: "geonames 2950159 (Berlin) is genuinely inside 2921044 (Germany). The engine compares \
                  opaque strings, so a claim one level down the hierarchy never matches.",
        falsified_by: "Allow -- which would mean a region hierarchy exists somewhere in the engine",
        request: one(
            c("spatial", Operator::Eq, "https://www.geonames.org/2921044"),
            claims(&[("spatial", s("https://www.geonames.org/2950159"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-purpose-opaque-hit",
        kind: POSITIVE,
        title: "purpose works losslessly as an opaque IRI-valued claim",
        asserts: "The left operands whose spec semantics are plain identity (purpose, industry, media, \
                  product, fileFormat, ...) lose nothing under opaque-string matching.",
        falsified_by: "Deny",
        request: one(
            c("purpose", Operator::Eq, "http://example.com/Purpose:research"),
            claims(&[("purpose", s("http://example.com/Purpose:research"))]),
        ),
        patches: vec![],
        expect: allow_constrained("purpose eq http://example.com/Purpose:research"),
    }));

    probes.push(build(Spec {
        id: "lo-language-no-bcp47",
        kind: NEGATIVE,
        title: "language has no BCP-47 range matching: en-GB does not match en",
        asserts: "Per BCP-47 basic filtering, the range `en` matches the tag `en-GB`. This engine \
                  compares strings, so it does not.",
        falsified_by: "Allow -- which would mean language-range handling exists",
        request: one(c("language", Operator::Eq, "en"), claims(&[("language", s("en-GB"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-event-no-period-ordering",
        kind: NEGATIVE,
        title: "event has no period ordering: lt over two event IRIs is a miss",
        asserts: "The spec reads `event lt <IRI>` as \"before that event's period\". Neither side parses \
                  as a temporal value or a number here, so the ordering operator misses rather than \
                  dereferencing anything.",
        falsified_by: "Allow -- which would require modelling event periods",
        request: one(
            c("event", Operator::Lt, "https://example.org/events/conf2"),
            claims(&[("event", s("https://example.org/events/conf"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-duration-metering-example",
        kind: POSITIVE,
        title: "xsd:duration is parsed by magnitude: PT30M lteq PT60M allows",
        asserts: "The spec's own metering example (ODRL 2.2 Vocabulary Section 4.5). Read against \
                  lo-duration-malformed-miss, which is the same left operand and operator with a \
                  malformed duration and Denies -- so this is genuine ISO-8601 duration parsing, not \
                  a lexical accident.",
        falsified_by: "Deny -- which would mean the duration parser regressed",
        request: one(c("meteredTime", Operator::Lteq, "PT60M"), claims(&[("meteredTime", s("PT30M"))])),
        patches: vec![],
        expect: allow_constrained("meteredTime lteq PT60M"),
    }));

    probes.push(build(Spec {
        id: "lo-duration-malformed-miss",
        kind: NEGATIVE,
        title: "a malformed duration still misses rather than silently matching",
        asserts: "The control for lo-duration-metering-example: same left operand and operator, but the \
                  claim is not a well-formed ISO-8601 duration (no unit letters at all), so it satisfies \
                  neither the temporal, duration, nor numeric fallback and correctly misses.",
        falsified_by: "Allow -- which would mean an unparseable value is silently accepted somewhere",
        request: one(c("meteredTime", Operator::Lteq, "PT60M"), claims(&[("meteredTime", s("thirty minutes"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-coordinates-string-eq",
        kind: POSITIVE,
        title: "spatialCoordinates matches only as an exact string",
        asserts: "A coordinate pair is carried faithfully and compared as text.",
        falsified_by: "Deny",
        request: one(
            c("spatialCoordinates", Operator::Eq, "48.8566,2.3522"),
            claims(&[("spatialCoordinates", s("48.8566,2.3522"))]),
        ),
        patches: vec![],
        expect: allow_constrained("spatialCoordinates eq 48.8566,2.3522"),
    }));

    probes.push(build(Spec {
        id: "lo-coordinates-no-geometry",
        kind: NEGATIVE,
        title: "coordinates about ten metres apart do not match",
        asserts: "48.8567,2.3523 is roughly 10 m from 48.8566,2.3522 -- the same place by any geometric \
                  reading. There is no geometry math anywhere in the engine.",
        falsified_by: "Allow -- which would require a distance or containment computation",
        request: one(
            c("spatialCoordinates", Operator::Eq, "48.8566,2.3522"),
            claims(&[("spatialCoordinates", s("48.8567,2.3523"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-absoluteposition-no-ordering",
        kind: NEGATIVE,
        title: "absolutePosition has no ordering: lt over coordinate pairs misses",
        asserts: "\"48.8566,2.3522\" parses as neither a temporal value nor a number, so an ordering \
                  operator over positions is a silent miss.",
        falsified_by: "Allow",
        request: one(
            c("absolutePosition", Operator::Lt, "49,3"),
            claims(&[("absolutePosition", s("48.8566,2.3522"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lo-unitofcount-page",
        kind: NEGATIVE,
        title: "unitOfCount: page does not qualify the count constraint",
        asserts: "Read against lo-unitofcount-volume, which is the same request with a different \
                  unitOfCount claim and reaches the identical decision and the identical reason -- so the \
                  unit never enters the evaluation at all.",
        falsified_by: "a decision or reason differing from lo-unitofcount-volume's",
        request: one(c("count", Operator::Lteq, "10"), claims(&[("count", s("7")), ("unitOfCount", s("page"))])),
        patches: vec![],
        expect: allow_constrained("count lteq 10"),
    }));

    probes.push(build(Spec {
        id: "lo-unitofcount-volume",
        kind: NEGATIVE,
        title: "unitOfCount: volume reaches exactly the same decision and reason",
        asserts: "The other half of the pair: two mutually exclusive units, one indistinguishable answer.",
        falsified_by: "a decision or reason differing from lo-unitofcount-page's",
        request: one(c("count", Operator::Lteq, "10"), claims(&[("count", s("7")), ("unitOfCount", s("volume"))])),
        patches: vec![],
        expect: allow_constrained("count lteq 10"),
    }));

    probes.push(build(Spec {
        id: "lo-unitofcount-as-plain-key",
        kind: POSITIVE,
        title: "unitOfCount is only ever an ordinary opaque claims key",
        asserts: "The one thing it can do: be constrained directly, like any other free-form key. It is \
                  never a qualifier on another constraint.",
        falsified_by: "Deny",
        request: one(c("unitOfCount", Operator::Eq, "page"), claims(&[("unitOfCount", s("page"))])),
        patches: vec![],
        expect: allow_constrained("unitOfCount eq page"),
    }));

    probes.push(build(Spec {
        id: "lo-policyusage-literal",
        kind: NEGATIVE,
        title: "odrl:policyUsage is compared as a bare string, not as execution history",
        asserts: "The reserved right operand means \"the moment this policy was used\". Here it matches \
                  only because a host put the literal string in the claims map.",
        falsified_by: "Deny -- which would only mean this catalog mis-stated the string comparison",
        request: one(c("event", Operator::Eq, "odrl:policyUsage"), claims(&[("event", s("odrl:policyUsage"))])),
        patches: vec![],
        expect: allow_constrained("event eq odrl:policyUsage"),
    }));

    probes.push(build(Spec {
        id: "lo-policyusage-absent",
        kind: NEGATIVE,
        title: "with no host-injected claim, odrl:policyUsage means nothing",
        asserts: "The pair's point: the engine keeps no execution history, so the reserved right operand \
                  carries no meaning of its own.",
        falsified_by: "Allow -- which would mean the engine tracked its own past usage",
        request: one(c("event", Operator::Eq, "odrl:policyUsage"), no_claims()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes
}

// ---------------------------------------------------------------------
// 3 — Operators
// ---------------------------------------------------------------------

fn operator_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(27);

    probes.push(build(Spec {
        id: "op-eq-single",
        kind: POSITIVE,
        title: "eq over a single-valued claim is plain equality",
        asserts: "The uncontroversial half of eq.",
        falsified_by: "Deny",
        request: one(c("nationality", Operator::Eq, "DE"), claims(&[("nationality", s("DE"))])),
        patches: vec![],
        expect: allow_constrained("nationality eq DE"),
    }));

    probes.push(build(Spec {
        id: "op-eq-multi-membership",
        kind: NEGATIVE,
        title: "eq over a multi-valued claim is membership, not identity",
        asserts: "The documented divergence from strict ODRL equality: the claim is the two-element list \
                  [FR, DE], which under spec equality is not equal to DE. This engine reads it as \
                  membership and Allows.",
        falsified_by: "Deny -- which would mean the documented adaptation is not actually in effect",
        request: one(c("nationality", Operator::Eq, "DE"), claims(&[("nationality", m(&["FR", "DE"]))])),
        patches: vec![],
        expect: allow_constrained("nationality eq DE"),
    }));

    probes.push(build(Spec {
        id: "op-eq-no-concat",
        kind: NEGATIVE,
        title: "the membership reading of eq is bounded: it does not compare joined lists",
        asserts: "Bounds op-eq-multi-membership. `eq FR,DE` against the claim [FR, DE] misses -- eq never \
                  splits or joins its right operand, so the divergence is membership and nothing wider.",
        falsified_by: "Allow -- which would mean eq quietly acquired list semantics of its own",
        request: one(c("nationality", Operator::Eq, "FR,DE"), claims(&[("nationality", m(&["FR", "DE"]))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-neq-hit",
        kind: POSITIVE,
        title: "neq is satisfied by a present, differing claim",
        asserts: "The ordinary case.",
        falsified_by: "Deny",
        request: one(c("nationality", Operator::Neq, "DE"), claims(&[("nationality", s("US"))])),
        patches: vec![],
        expect: allow_constrained("nationality neq DE"),
    }));

    probes.push(build(Spec {
        id: "op-neq-absent-miss",
        kind: NEGATIVE,
        title: "an absent claim key is a MISS for neq, not a satisfaction",
        asserts: "This one is a live catch of stale documentation: the source gap analysis records the \
                  opposite (\"an absent claim key counts as satisfying neq\"). It does not. Read against \
                  op-isnoneof-absent-satisfies, which is the one operator that genuinely does treat an \
                  absent key as satisfied.",
        falsified_by: "Allow -- which would make the absent-key posture non-uniform in a second place",
        request: one(c("nationality", Operator::Neq, "DE"), no_claims()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-isanyof-hit",
        kind: POSITIVE,
        title: "isAnyOf matches a member of its comma-delimited right operand",
        asserts: "The set operator's ordinary case.",
        falsified_by: "Deny",
        request: one(c("scope", Operator::IsAnyOf, "read,write,delete"), claims(&[("scope", s("write"))])),
        patches: vec![],
        expect: allow_constrained("scope isAnyOf read,write,delete"),
    }));

    probes.push(build(Spec {
        id: "op-isanyof-miss",
        kind: NEGATIVE,
        title: "isAnyOf misses a value outside the list",
        asserts: "The pair's other half.",
        falsified_by: "Allow",
        request: one(c("scope", Operator::IsAnyOf, "read,write,delete"), claims(&[("scope", s("admin"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-isanyof-comma-unescapable",
        kind: NEGATIVE,
        title: "a value containing a comma is inexpressible in a set operator's right operand",
        asserts: "ODRL's rightOperand for a set operator is a JSON-LD list; this engine's is a single \
                  string split on commas, with no escaping convention. The one-element list \
                  [\"research,teaching\"] therefore cannot be written at all -- the claim that IS that \
                  literal string misses.",
        falsified_by: "Allow -- which would mean an escaping convention exists after all",
        request: one(
            c("purpose", Operator::IsAnyOf, "research,teaching"),
            claims(&[("purpose", s("research,teaching"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-isanyof-comma-control",
        kind: POSITIVE,
        title: "the same constraint matches a claim with no comma in it",
        asserts: "Isolates op-isanyof-comma-unescapable's failure to the comma: identical constraint, \
                  comma-free claim, Allow.",
        falsified_by: "Deny",
        request: one(c("purpose", Operator::IsAnyOf, "research,teaching"), claims(&[("purpose", s("research"))])),
        patches: vec![],
        expect: allow_constrained("purpose isAnyOf research,teaching"),
    }));

    probes.push(build(Spec {
        id: "op-isallof-hit",
        kind: POSITIVE,
        title: "isAllOf requires every element of the right operand among the claim's values",
        asserts: "The claim [read, write, delete] covers the required set [read, write].",
        falsified_by: "Deny",
        request: one(
            c("scope", Operator::IsAllOf, "read,write"),
            claims(&[("scope", m(&["read", "write", "delete"]))]),
        ),
        patches: vec![],
        expect: allow_constrained("scope isAllOf read,write"),
    }));

    probes.push(build(Spec {
        id: "op-isallof-miss",
        kind: NEGATIVE,
        title: "isAllOf misses when one required element is absent",
        asserts: "The claim [read] does not cover [read, write].",
        falsified_by: "Allow -- which would collapse isAllOf into isAnyOf",
        request: one(c("scope", Operator::IsAllOf, "read,write"), claims(&[("scope", m(&["read"]))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-isnoneof-hit",
        kind: POSITIVE,
        title: "isNoneOf is satisfied by a value outside the excluded set",
        asserts: "The exclusion holds for FR against [US, CN].",
        falsified_by: "Deny",
        request: one(c("nationality", Operator::IsNoneOf, "US,CN"), claims(&[("nationality", s("FR"))])),
        patches: vec![],
        expect: allow_constrained("nationality isNoneOf US,CN"),
    }));

    probes.push(build(Spec {
        id: "op-isnoneof-miss",
        kind: NEGATIVE,
        title: "isNoneOf misses on an excluded value",
        asserts: "The pair's other half.",
        falsified_by: "Allow -- which would make the exclusion inert",
        request: one(c("nationality", Operator::IsNoneOf, "US,CN"), claims(&[("nationality", s("US"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-isnoneof-absent-satisfies",
        kind: POSITIVE,
        title: "an absent claim key SATISFIES isNoneOf -- the one deliberate exception",
        asserts: "A claim that is not present cannot carry a forbidden value, so there is nothing left to \
                  violate the exclusion. Read against op-neq-absent-miss, the other negation-shaped \
                  operator, which does not behave this way: this exception is narrow and separately \
                  justified, not a general rule about negations.",
        falsified_by: "Deny -- which would make the documented exception not real",
        request: one(c("nationality", Operator::IsNoneOf, "US,CN"), no_claims()),
        patches: vec![],
        expect: allow_constrained("nationality isNoneOf US,CN"),
    }));

    probes.push(build(Spec {
        id: "op-ispartof-hit",
        kind: POSITIVE,
        title: "isPartOf matches flat, enumerated membership",
        asserts: "What this engine's isPartOf actually does: enumerated membership in a comma list.",
        falsified_by: "Deny",
        request: one(c("spatial", Operator::IsPartOf, "DE,FR,IT"), claims(&[("spatial", s("DE"))])),
        patches: vec![],
        expect: allow_constrained("spatial isPartOf DE,FR,IT"),
    }));

    probes.push(build(Spec {
        id: "op-ispartof-no-hierarchy",
        kind: NEGATIVE,
        title: "isPartOf is not hierarchy membership: Berlin is not part of Germany here",
        asserts: "Berlin genuinely IS part of Germany, which is exactly what ODRL's isPartOf means. This \
                  engine's version cannot see it -- the operator name is honest about intent, not about \
                  capability.",
        falsified_by: "Allow -- which would require a containment graph",
        request: one(
            c("spatial", Operator::IsPartOf, "https://www.geonames.org/2921044"),
            claims(&[("spatial", s("https://www.geonames.org/2950159"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-ispartof-mirrors-isanyof",
        kind: NEGATIVE,
        title: "isPartOf is observationally a degenerate alias for isAnyOf",
        asserts: "This request differs from op-isanyof-hit by the operator token alone, and reaches the \
                  same decision -- the two operators run the identical test.",
        falsified_by: "Deny -- which would mean isPartOf is doing something isAnyOf does not",
        request: one(c("scope", Operator::IsPartOf, "read,write,delete"), claims(&[("scope", s("write"))])),
        patches: vec![],
        expect: allow_constrained("scope isPartOf read,write,delete"),
    }));

    probes.push(build(Spec {
        id: "op-lt-fractional-chronological",
        kind: POSITIVE,
        title: "lt orders timestamps chronologically, not lexically",
        asserts: "Lexically \"2024-02-12T11:20:00Z\" sorts AFTER \"2024-02-12T11:20:00.5Z\" (Z > .), so a \
                  string comparison would miss. Chronologically it is half a second earlier, so an Allow \
                  here is proof the comparison is temporal.",
        falsified_by: "Deny -- which is exactly what a lexical comparison would produce",
        request: one(
            c("dateTime", Operator::Lt, "2024-02-12T11:20:00.5Z"),
            claims(&[("dateTime", s("2024-02-12T11:20:00Z"))]),
        ),
        patches: vec![],
        expect: allow_constrained("dateTime lt 2024-02-12T11:20:00.5Z"),
    }));

    probes.push(build(Spec {
        id: "op-lteq-xsd-date",
        kind: POSITIVE,
        title: "a bare xsd:date claim is read as midnight UTC",
        asserts: "\"2024-02-12\" equals \"2024-02-12T00:00:00Z\" under this engine's stated convention, so \
                  lteq holds at the boundary.",
        falsified_by: "Deny -- which would mean only the full dateTime lexical form is accepted",
        request: one(
            c("dateTime", Operator::Lteq, "2024-02-12T00:00:00Z"),
            claims(&[("dateTime", s("2024-02-12"))]),
        ),
        patches: vec![],
        expect: allow_constrained("dateTime lteq 2024-02-12T00:00:00Z"),
    }));

    probes.push(build(Spec {
        id: "op-lt-offset-datetime",
        kind: POSITIVE,
        title: "a numeric UTC offset is normalized before comparison",
        asserts: "13:20+02:00 is 11:20Z, which is before 12:00Z. A UTC-only reader comparing the hour \
                  field would miss this.",
        falsified_by: "Deny",
        request: one(
            c("dateTime", Operator::Lt, "2024-02-12T12:00:00Z"),
            claims(&[("dateTime", s("2024-02-12T13:20:00+02:00"))]),
        ),
        patches: vec![],
        expect: allow_constrained("dateTime lt 2024-02-12T12:00:00Z"),
    }));

    probes.push(build(Spec {
        id: "op-gteq-numeric-boundary",
        kind: POSITIVE,
        title: "gteq includes its boundary",
        asserts: "10 gteq 10 holds.",
        falsified_by: "Deny",
        request: one(c("count", Operator::Gteq, "10"), claims(&[("count", s("10"))])),
        patches: vec![],
        expect: allow_constrained("count gteq 10"),
    }));

    probes.push(build(Spec {
        id: "op-gt-numeric-boundary",
        kind: NEGATIVE,
        title: "gt excludes its boundary",
        asserts: "The strict/non-strict pair over the identical claim: 10 is not gt 10.",
        falsified_by: "Allow -- which would make gt and gteq indistinguishable",
        request: one(c("count", Operator::Gt, "10"), claims(&[("count", s("10"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "op-lt-mixed-type-miss",
        kind: NEGATIVE,
        title: "comparing a timestamp against a number is a silent miss, not a coercion",
        asserts: "Neither the temporal pairing nor the numeric fallback succeeds when the two sides are \
                  different kinds of value. The engine misses rather than coercing one into the other.",
        falsified_by: "Allow -- which would mean cross-type coercion happens somewhere",
        request: one(c("dateTime", Operator::Lt, "99999"), claims(&[("dateTime", s("2024-02-12T11:20:00Z"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    // The three operator-token probes below are literally one policy
    // structure varying only the `operator` string.
    let operator_token_request =
        || one(c("purpose", Operator::Eq, "odrl:Purpose"), claims(&[("purpose", s("odrl:Purpose"))]));

    probes.push(build(Spec {
        id: "op-isa-unparseable",
        kind: NEGATIVE,
        title: "isA is not in the operator enum, so the whole request fails to parse",
        asserts: "The engine's Operator is a closed Rust enum. An out-of-enum token is not an unknown \
                  field to be ignored -- it fails deserialization, and the ABI answers Error with the \
                  serde message. Read against op-isa-control-eq, the byte-identical request with a \
                  supported token.",
        falsified_by: "Allow or Deny -- either would mean the token was tolerated somehow",
        request: operator_token_request(),
        patches: vec![Patch::set("/policies/0/permissions/0/constraints/0", "operator", json!("isA"))],
        expect: error(&["request did not parse as the documented Section 5.2 JSON shape", "isA"]),
    }));

    probes.push(build(Spec {
        id: "op-haspart-unparseable",
        kind: NEGATIVE,
        title: "hasPart is not in the operator enum either",
        asserts: "The same policy structure as op-isa-unparseable, varying only the operator token: the \
                  enum is closed at the engine's own compile time, not per-token.",
        falsified_by: "Allow or Deny",
        request: operator_token_request(),
        patches: vec![Patch::set("/policies/0/permissions/0/constraints/0", "operator", json!("hasPart"))],
        expect: error(&["request did not parse as the documented Section 5.2 JSON shape", "hasPart"]),
    }));

    probes.push(build(Spec {
        id: "op-isa-control-eq",
        kind: POSITIVE,
        title: "the same request with a supported operator parses and matches",
        asserts: "The control for both unparseable-operator probes: identical policy, identical claim, \
                  operator `eq` -- so the Error above is the operator token and nothing else about the \
                  request.",
        falsified_by: "anything but Allow -- which would mean the pair proves nothing about operators",
        request: operator_token_request(),
        patches: vec![],
        expect: allow_constrained("purpose eq odrl:Purpose"),
    }));

    probes.push(build(Spec {
        id: "op-profile-operator-unparseable",
        kind: NEGATIVE,
        title: "declaring an operator extension in the profile changes nothing",
        asserts: "The config declares `ex:withinRadius` as an odrl:Operator -- the real ODRL profile \
                  extension point -- and the constraint then uses it. The request still fails to parse. \
                  The enum is fixed at the engine's compile time; no wire-level declaration can widen it.",
        falsified_by: "Allow or Deny -- either would mean profile-declared operators take effect",
        request: operator_token_request(),
        patches: vec![
            Patch::set("/config", "odrl:operator", json!([{"@id": "ex:withinRadius"}])),
            Patch::set("/policies/0/permissions/0/constraints/0", "operator", json!("ex:withinRadius")),
        ],
        expect: error(&["request did not parse as the documented Section 5.2 JSON shape", "unknown variant"]),
    }));

    probes
}

// ---------------------------------------------------------------------
// 4 — Logical constraints
// ---------------------------------------------------------------------

fn logical_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(13);

    let de_read = || claims(&[("nationality", s("DE")), ("scope", s("read"))]);
    let de_admin = || claims(&[("nationality", s("DE")), ("scope", s("admin"))]);
    let and_children = || vec![c("nationality", Operator::Eq, "DE"), c("scope", Operator::Eq, "read")];
    let xone_children = || vec![c("nationality", Operator::Eq, "DE"), c("scope", Operator::Eq, "admin")];

    probes.push(build(Spec {
        id: "lc-and-both",
        kind: POSITIVE,
        title: "odrl:and is satisfied when every child is",
        asserts: "A real nested logical constraint inside one rule's constraint list, rendered as a group \
                  in the reason rather than flattened.",
        falsified_by: "Deny",
        request: one(Constraint::and(and_children()), de_read()),
        patches: vec![],
        expect: allow_constrained("(nationality eq DE && scope eq read)"),
    }));

    probes.push(build(Spec {
        id: "lc-and-one-false",
        kind: NEGATIVE,
        title: "odrl:and misses when one child does not match",
        asserts: "The pair's other half.",
        falsified_by: "Allow",
        request: one(Constraint::and(and_children()), de_admin()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lc-and-empty-vacuous",
        kind: POSITIVE,
        title: "an empty odrl:and is vacuously satisfied",
        asserts: "The same convention an empty constraint list already uses. Contrast lc-or-empty-never, \
                  whose empty list is never satisfied.",
        falsified_by: "Deny",
        request: one(Constraint::and(vec![]), no_claims()),
        patches: vec![],
        expect: allow_constrained("()"),
    }));

    probes.push(build(Spec {
        id: "lc-or-second",
        kind: POSITIVE,
        title: "odrl:or is satisfied by its second child",
        asserts: "Disjunction is real nesting inside the engine, not sibling rules expanded by a host-side \
                  adapter.",
        falsified_by: "Deny",
        request: one(
            Constraint::or(vec![c("nationality", Operator::Eq, "FR"), c("nationality", Operator::Eq, "DE")]),
            claims(&[("nationality", s("DE"))]),
        ),
        patches: vec![],
        expect: allow_constrained("(nationality eq FR || nationality eq DE)"),
    }));

    probes.push(build(Spec {
        id: "lc-or-none",
        kind: NEGATIVE,
        title: "odrl:or misses when no child matches",
        asserts: "The pair's other half.",
        falsified_by: "Allow",
        request: one(
            Constraint::or(vec![c("nationality", Operator::Eq, "FR"), c("nationality", Operator::Eq, "DE")]),
            claims(&[("nationality", s("US"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lc-or-empty-never",
        kind: NEGATIVE,
        title: "an empty odrl:or is never satisfied",
        asserts: "There is nothing to witness the \"at least one\". Read against lc-and-empty-vacuous: the \
                  two empty lists deliberately mean opposite things.",
        falsified_by: "Allow",
        request: one(Constraint::or(vec![]), no_claims()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lc-xone-exactly-one",
        kind: POSITIVE,
        title: "odrl:xone is satisfied by exactly one matching child",
        asserts: "The capability a disjunctive-normal-form expansion cannot express at all.",
        falsified_by: "Deny",
        request: one(Constraint::xone(xone_children()), claims(&[("nationality", s("DE")), ("scope", s("user"))])),
        patches: vec![],
        expect: allow_constrained("xone(nationality eq DE, scope eq admin)"),
    }));

    probes.push(build(Spec {
        id: "lc-xone-zero",
        kind: NEGATIVE,
        title: "odrl:xone misses when no child matches",
        asserts: "The lower boundary, shared with odrl:or.",
        falsified_by: "Allow",
        request: one(Constraint::xone(xone_children()), claims(&[("nationality", s("US")), ("scope", s("user"))])),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lc-xone-two-denies",
        kind: NEGATIVE,
        title: "odrl:xone misses when TWO children match",
        asserts: "The upper boundary, and the whole reason xone is not or. Read against \
                  lc-or-two-allows-control, whose children and claims are identical and which Allows.",
        falsified_by: "Allow -- which would make xone an or",
        request: one(Constraint::xone(xone_children()), de_admin()),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lc-or-two-allows-control",
        kind: POSITIVE,
        title: "the same two matching children under odrl:or do Allow",
        asserts: "The control that makes lc-xone-two-denies mean something: same children, same claims, \
                  one key name apart, opposite decisions.",
        falsified_by: "Deny",
        request: one(Constraint::or(xone_children()), de_admin()),
        patches: vec![],
        expect: allow_constrained("(nationality eq DE || scope eq admin)"),
    }));

    // The strongest negative pair in the catalog: two requests one key
    // name apart.
    let andsequence_children = json!([
        {"left_operand": "nationality", "operator": "eq", "right_operand": "DE"},
        {"left_operand": "scope", "operator": "eq", "right_operand": "read"}
    ]);

    probes.push(build(Spec {
        id: "lc-andsequence-ignored",
        kind: NEGATIVE,
        title: "odrl:andSequence is dropped; the constraint's own atomic fields decide",
        asserts: "The engine's Constraint type has and/or/xone and nothing else. Read against \
                  lc-and-control-honored, whose request differs from this one by the single key name \
                  `odrl:and` vs `odrl:andSequence`: that one is honoured (Allow), this one is dropped and \
                  the false atomic fields decide (Deny).",
        falsified_by: "Allow -- which would mean odrl:andSequence is being evaluated",
        request: one(c("nationality", Operator::Eq, "US"), de_read()),
        patches: vec![Patch::set(
            "/policies/0/permissions/0/constraints/0",
            "odrl:andSequence",
            andsequence_children.clone(),
        )],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "lc-and-control-honored",
        kind: POSITIVE,
        title: "the byte-identical request with the key renamed odrl:and IS honoured",
        asserts: "One key name apart from lc-andsequence-ignored. The nested children are evaluated, the \
                  atomic fields beside them are ignored, and the reason renders the group.",
        falsified_by: "Deny -- which would mean the pair isolates nothing",
        request: one(c("nationality", Operator::Eq, "US"), de_read()),
        patches: vec![Patch::set("/policies/0/permissions/0/constraints/0", "odrl:and", andsequence_children)],
        expect: allow_constrained("(nationality eq DE && scope eq read)"),
    }));

    probes.push(build(Spec {
        id: "lc-custom-logical-ignored",
        kind: NEGATIVE,
        title: "a profile-declared logical operand is dropped like any other unknown key",
        asserts: "The config declares `ex:majorityOf` and the constraint uses it; the engine's three \
                  logical fields are fixed, so the declaration changes nothing and the false atomic fields \
                  decide. Controlled by lc-and-control-honored, which is the same shape under a key the \
                  engine does know.",
        falsified_by: "Allow -- which would mean profile-declared logical operands take effect",
        request: one(c("nationality", Operator::Eq, "US"), de_read()),
        patches: vec![
            Patch::set("/config", "odrl:logicalConstraint", json!([{"@id": "ex:majorityOf"}])),
            Patch::set(
                "/policies/0/permissions/0/constraints/0",
                "ex:majorityOf",
                json!([
                    {"left_operand": "nationality", "operator": "eq", "right_operand": "DE"},
                    {"left_operand": "scope", "operator": "eq", "right_operand": "read"}
                ]),
            ),
        ],
        expect: deny(&closed_deny("use")),
    }));

    probes
}

// ---------------------------------------------------------------------
// 5 — Policy classes
// ---------------------------------------------------------------------

fn policy_class_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(5);

    let stranger = || claims(&[("sub", s("did:web:mallory.example"))]);
    let named_to_alice = |kind: &str| {
        let mut request = base_request();
        request.policies[0].kind = kind.to_string();
        request.policies[0].assignee = Some("did:web:alice.example".to_string());
        request.claims = stranger();
        request
    };

    probes.push(build(Spec {
        id: "pc-kind-set",
        kind: NEGATIVE,
        title: "kind: Set grants to a caller who is not the named assignee",
        asserts: "The reference point for the whole category: whatever `kind` says, evaluation is the \
                  same. This is the one class for which that is also the correct answer.",
        falsified_by: "Deny",
        request: named_to_alice("Set"),
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pc-kind-agreement-ignores-assignee",
        kind: NEGATIVE,
        title: "kind: Agreement grants to a stranger, exactly as Set does",
        asserts: "An ODRL Agreement MUST grant only to its own named assignee. This request names \
                  did:web:alice.example and the caller presents did:web:mallory.example -- and the \
                  decision AND the reason are byte-identical to pc-kind-set's. No class-level MUST is \
                  validated anywhere: `kind` never selects a semantics. Assignee scoping is available \
                  (pf-assignee-scoped-miss) but is switched on by the config's partyIdentityClaim, not \
                  by a policy calling itself an Agreement, and this request does not switch it on.",
        falsified_by: "Deny -- which would mean assignee scoping is enforced for Agreements",
        request: named_to_alice("Agreement"),
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pc-kind-ticket-with-assignee",
        kind: NEGATIVE,
        title: "kind: Ticket carrying an assignee is accepted, though the spec forbids one",
        asserts: "The MUST NOT direction of the same absence: a structurally invalid Ticket evaluates \
                  normally rather than being rejected.",
        falsified_by: "Deny or Error",
        request: named_to_alice("Ticket"),
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pc-kind-nonsense",
        kind: NEGATIVE,
        title: "a kind that is not an ODRL policy class at all is accepted",
        asserts: "`NotAnOdrlPolicyClass` reaches the identical decision and reason as Set, Agreement and \
                  Ticket. Nothing validates `kind`; carrying it on the wire is documentation, not \
                  semantics.",
        falsified_by: "Error -- which would mean `kind` is validated",
        request: named_to_alice("NotAnOdrlPolicyClass"),
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pc-kind-profile-subclass",
        kind: NEGATIVE,
        title: "a profile-declared Policy subclass sits in the same position as any other unknown kind",
        asserts: "The config declares `ex:ResearchLicence` as a Policy subclass and the policy uses it. \
                  Same decision, same reason as pc-kind-nonsense -- which is the honest reading: a \
                  declared subclass is observationally indistinguishable from an unrecognised string.",
        falsified_by: "a decision or reason differing from pc-kind-nonsense's",
        request: named_to_alice("ex:ResearchLicence"),
        patches: vec![Patch::set("/config", "odrl:policyClass", json!([{"@id": "ex:ResearchLicence"}]))],
        expect: allow(&base_allow_reason()),
    }));

    probes
}

// ---------------------------------------------------------------------
// 6 — Party functions
// ---------------------------------------------------------------------

fn party_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(7);

    let stranger = || claims(&[("sub", s("did:web:mallory.example"))]);

    // The request the two `pf-assignee-scoped-*` probes share, differing
    // only in whose identity the caller presents: one policy assigned to
    // alice, under a config naming `sub` as the claim that identifies the
    // caller. `party_identity_claim` is a real, modelled field on
    // `engine::RequestConfig`, so these are typed values like every other
    // supported part of a request rather than keys injected through
    // `patch.rs`.
    let scoped_to_alice = |caller_claims: Claims| {
        let mut request = Request {
            policies: vec![WirePolicy {
                assignee: Some("did:web:alice.example".to_string()),
                ..policy("probe", vec![rule("use", vec![])])
            }],
            claims: caller_claims,
            ..base_request()
        };
        request.config.party_identity_claim = Some("sub".to_string());
        request
    };

    probes.push(build(Spec {
        id: "pf-assignee-mismatch",
        kind: NEGATIVE,
        title: "with no partyIdentityClaim configured, a policy's assignee is never compared against \
                the caller",
        asserts: "The policy names did:web:alice.example; the caller's `sub` claim is \
                  did:web:mallory.example; the config names no identity claim. The permission still \
                  applies. This is the default, and what every request built before \
                  partyIdentityClaim existed gets.",
        falsified_by: "Deny -- which would mean assignee is evaluated as a party scope unasked",
        request: Request {
            policies: vec![WirePolicy {
                assignee: Some("did:web:alice.example".to_string()),
                ..policy("probe", vec![rule("use", vec![])])
            }],
            claims: stranger(),
            ..base_request()
        },
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pf-assignee-null-control",
        kind: POSITIVE,
        title: "the same request with no assignee at all reaches the identical answer",
        asserts: "The control for pf-assignee-mismatch and for every inert-party-property probe below: \
                  the field is carried faithfully and, with no partyIdentityClaim configured, consumed \
                  by nothing.",
        falsified_by: "a decision or reason differing from pf-assignee-mismatch's",
        request: Request { claims: stranger(), ..base_request() },
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pf-assignee-scoped-hit",
        kind: POSITIVE,
        title: "with partyIdentityClaim configured, a policy still applies to the assignee it names",
        asserts: "The opt-in half of the pair: the config names `sub` as the caller's identity claim, \
                  the policy names did:web:alice.example, and the caller presents exactly that. The \
                  answer is byte-identical to pf-assignee-mismatch's -- switching party-role scoping \
                  on must change nothing for the party a policy is addressed to.",
        falsified_by: "Deny -- which would mean the check rejects even the correct assignee",
        request: scoped_to_alice(claims(&[("sub", s("did:web:alice.example"))])),
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pf-assignee-scoped-miss",
        kind: NEGATIVE,
        title: "with partyIdentityClaim configured, a policy assigned to someone else grants nothing",
        asserts: "Identical to pf-assignee-scoped-hit but for the caller's own `sub`. The policy is \
                  treated as absent from the request rather than as a policy that grants nothing, so \
                  the answer is a deny whose reason names the assignee mismatch -- explicitly not the \
                  closed-default 'no permission ... covered and matched' line.",
        falsified_by: "Allow, or a Deny reported as an ordinary constraint miss",
        request: scoped_to_alice(stranger()),
        patches: vec![],
        expect: deny(
            "no policy in the request applies to this caller: policy 'probe' names odrl:assignee \
             'did:web:alice.example', which does not match the caller's 'sub' claim \
             (\"did:web:mallory.example\")",
        )
        .excluding(&["no permission of policy"]),
    }));

    probes.push(build(Spec {
        id: "pf-assignee-as-claim",
        kind: POSITIVE,
        title: "party scoping is also reachable by mirroring the party into the claims map",
        asserts: "The other route to party-scoped evaluation, and the only one before \
                  partyIdentityClaim existed: put the party in the claims map and constrain on it as \
                  an ordinary opaque key. Note the key is a claim named `assignee`, not the policy's \
                  own assignee field, and that this scopes one *rule* where partyIdentityClaim scopes \
                  the whole policy.",
        falsified_by: "Deny",
        request: one(
            c("assignee", Operator::Eq, "did:web:alice.example"),
            claims(&[("assignee", s("did:web:alice.example"))]),
        ),
        patches: vec![],
        expect: allow_constrained("assignee eq did:web:alice.example"),
    }));

    probes.push(build(Spec {
        id: "pf-assignerof-inert",
        kind: NEGATIVE,
        title: "assignerOf and assigneeOf are dropped",
        asserts: "ODRL's two inverse party properties, injected on the policy, reaching the identical \
                  decision and reason as pf-assignee-null-control's request without them.",
        falsified_by: "any decision or reason differing from pf-assignee-null-control's",
        request: Request { claims: stranger(), ..base_request() },
        patches: vec![
            Patch::set("/policies/0", "assignerOf", json!("urn:asset:1")),
            Patch::set("/policies/0", "assigneeOf", json!("urn:asset:2")),
        ],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "pf-common-functions-inert",
        kind: NEGATIVE,
        title: "all twelve common party functions, injected at once, change nothing",
        asserts: "attributedParty/attributingParty, compensatedParty/compensatingParty, \
                  consentingParty/consentedParty, contractingParty/contractedParty, \
                  informedParty/informingParty, trackingParty/trackedParty -- every one of the Common \
                  Vocabulary's paired party roles on one policy, reaching the identical decision and \
                  reason as pf-assignee-null-control.",
        falsified_by: "any decision or reason differing from pf-assignee-null-control's",
        request: Request { claims: stranger(), ..base_request() },
        patches: vec![
            Patch::set("/policies/0", "attributedParty", json!("did:web:a.example")),
            Patch::set("/policies/0", "attributingParty", json!("did:web:b.example")),
            Patch::set("/policies/0", "compensatedParty", json!("did:web:c.example")),
            Patch::set("/policies/0", "compensatingParty", json!("did:web:d.example")),
            Patch::set("/policies/0", "consentingParty", json!("did:web:e.example")),
            Patch::set("/policies/0", "consentedParty", json!("did:web:f.example")),
            Patch::set("/policies/0", "contractingParty", json!("did:web:g.example")),
            Patch::set("/policies/0", "contractedParty", json!("did:web:h.example")),
            Patch::set("/policies/0", "informedParty", json!("did:web:i.example")),
            Patch::set("/policies/0", "informingParty", json!("did:web:j.example")),
            Patch::set("/policies/0", "trackingParty", json!("did:web:k.example")),
            Patch::set("/policies/0", "trackedParty", json!("did:web:l.example")),
        ],
        expect: allow(&base_allow_reason()),
    }));

    probes
}

// ---------------------------------------------------------------------
// 7 — Duty
// ---------------------------------------------------------------------

fn duty_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(7);

    let obligation_request = |duty_mode: DutyMode, obligations: Vec<Rule>, request_claims: Claims| {
        let mut request = base_request();
        request.config = flat_config(&["use", "notify"]);
        request.config.duty_mode = duty_mode;
        request.policies[0].obligations = obligations;
        request.claims = request_claims;
        request
    };

    probes.push(build(Spec {
        id: "duty-obligation-unresolved-advise",
        kind: POSITIVE,
        title: "an unconditional policy-level obligation is reported unresolved, not satisfied",
        asserts: "\"Satisfied\" here means a claims precondition the engine can check. An obligation with \
                  no constraints is an unconditional \"you must notify\" it has no way to verify, so it \
                  surfaces in `duties` rather than being assumed done.",
        falsified_by: "an empty duties list -- which would mean an unverifiable duty is treated as met",
        request: obligation_request(DutyMode::Advise, vec![rule("notify", vec![])], no_claims()),
        patches: vec![],
        expect: allow(&base_allow_reason()).with_duties(vec![duty("notify")]),
    }));

    probes.push(build(Spec {
        id: "duty-obligation-satisfied-by-claims",
        kind: POSITIVE,
        title: "a constrained obligation whose constraints match resolves and leaves duties empty",
        asserts: "The other half: duty satisfaction is purely a claims precondition, never an observation \
                  that the action was performed.",
        falsified_by: "a non-empty duties list",
        request: obligation_request(
            DutyMode::Advise,
            vec![rule("notify", vec![c("notified", Operator::Eq, "true")])],
            claims(&[("notified", s("true"))]),
        ),
        patches: vec![],
        expect: allow(&base_allow_reason()).with_duties(vec![]),
    }));

    probes.push(build(Spec {
        id: "duty-obligation-deny-mode",
        kind: POSITIVE,
        title: "under dutyMode: deny an unresolved obligation overrides the Allow",
        asserts: "The same request as duty-obligation-unresolved-advise with one knob moved. duties is \
                  emptied because the information is already carried by the decision itself.",
        falsified_by: "Allow -- which would make dutyMode inert",
        request: obligation_request(DutyMode::Deny, vec![rule("notify", vec![])], no_claims()),
        patches: vec![],
        expect: deny("duty[0] 'notify' of policy 'probe' is unresolved under duty_mode: deny").with_duties(vec![]),
    }));

    // A duty asserted through the claims map, which is how every duty in
    // this engine resolves: `duty:<action> eq fulfilled`. There is no
    // second lookup mechanism -- a host states duty state as an ordinary
    // claim, exactly as `compliance-runner` derives it from a
    // `report:DutyReport` fact before building its request.
    fn asserted_duty(action: &str) -> Rule {
        rule(action, vec![c(&format!("duty:{action}"), Operator::Eq, "fulfilled")])
    }

    fn fulfilled(action: &str) -> Claims {
        claims(&[(&format!("duty:{action}"), s("fulfilled"))])
    }

    // `base_request()` with the single `use` permission carrying one
    // per-permission `odrl:duty`, under `duty_mode`.
    let permission_duty_request = |duty_mode: DutyMode, duty: Rule, request_claims: Claims| {
        let mut request = base_request();
        request.config = flat_config(&["use", "compensate", "notify"]);
        request.config.duty_mode = duty_mode;
        request.policies[0].permissions = vec![Rule { duty: vec![duty], ..rule("use", vec![]) }];
        request.claims = request_claims;
        request
    };

    probes.push(build(Spec {
        id: "duty-per-permission-unresolved-deny",
        kind: POSITIVE,
        title: "under dutyMode: deny an unresolved per-permission odrl:duty stops that permission granting",
        asserts: "ODRL's `duty` on a Permission is a PRE-CONDITION for receiving that permission. It is                   resolved exactly as a policy-level obligation is -- its own constraints against the                   claims -- but the effect is scoped to the one permission it hangs off, so the reason                   names the permission rather than the policy.",
        falsified_by: "Allow, which would mean the pre-condition was discarded -- the fail-open gap this                        row recorded before the field existed",
        request: permission_duty_request(DutyMode::Deny, asserted_duty("compensate"), no_claims()),
        patches: vec![],
        expect: deny(
            "permission[0] of policy 'probe' matched, but its odrl:duty[0] 'compensate' is \
             unresolved under duty_mode: deny",
        )
        .with_duties(vec![attached_duty("compensate", "permission[0].duty[0]")]),
    }));

    probes.push(build(Spec {
        id: "duty-per-permission-satisfied",
        kind: POSITIVE,
        title: "the same permission grants once the claims assert its duty fulfilled",
        asserts: "The paired hit for the probe above: one claim differs. Without it, an engine that                   denied every duty-bearing permission outright would pass the deny probe.",
        falsified_by: "Deny, or a non-empty duties list",
        request: permission_duty_request(DutyMode::Deny, asserted_duty("compensate"), fulfilled("compensate")),
        patches: vec![],
        expect: allow(
            "permission[0] of policy 'probe' matched: action 'use', unconstrained; odrl:duty[0] \
             'compensate' satisfied",
        )
        .with_duties(vec![]),
    }));

    probes.push(build(Spec {
        id: "duty-per-permission-advisory",
        kind: POSITIVE,
        title: "under dutyMode: advise the same unresolved duty is advisory, and carries its attachment point",
        asserts: "The duty surfaces in `duties` with a `source` naming where it hangs -- the field a                   policy-level obligation never carries, which is how a caller tells the two apart.",
        falsified_by: "Deny, or a duties entry with no source",
        request: permission_duty_request(DutyMode::Advise, asserted_duty("compensate"), no_claims()),
        patches: vec![],
        expect: allow("permission[0] of policy 'probe' matched: action 'use', unconstrained")
            .with_duties(vec![attached_duty("compensate", "permission[0].duty[0]")]),
    }));

    probes.push(build(Spec {
        id: "duty-per-permission-scoped-to-its-own-permission",
        kind: POSITIVE,
        title: "an unresolved per-permission duty does not deny a policy whose other permission grants",
        asserts: "The difference from a policy-level obligation, stated as an observation: under the same                   dutyMode: deny, an unresolved *obligation* denies outright (duty-obligation-deny-mode),                   while an unresolved duty on permission[0] leaves permission[1] free to grant.",
        falsified_by: "Deny -- which would mean the duty was applied policy-wide, not to its own permission",
        request: {
            let mut request =
                permission_duty_request(DutyMode::Deny, asserted_duty("compensate"), no_claims());
            request.policies[0].permissions.push(rule("use", vec![]));
            request
        },
        patches: vec![],
        expect: allow("permission[1] of policy 'probe' matched: action 'use', unconstrained")
            .with_duties(vec![attached_duty("compensate", "permission[0].duty[0]")]),
    }));

    /// A policy-level obligation `notify` carrying an `odrl:consequence`
    /// duty `compensate`, both resolved from the claims map.
    fn notify_with_consequence() -> Rule {
        Rule::with_consequence(
            "notify",
            vec![c("duty:notify", Operator::Eq, "fulfilled")],
            asserted_duty("compensate"),
        )
    }

    probes.push(build(Spec {
        id: "duty-consequence-resolves-where-the-primary-did-not",
        kind: POSITIVE,
        title: "an unfulfilled duty falls through to its odrl:consequence, which resolves",
        asserts: "`notify` is not fulfilled, so ODRL says the consequence duty is what now applies -- and                   the claims assert *it* fulfilled, so nothing is outstanding and dutyMode: deny has                   nothing to act on.",
        falsified_by: "Deny, or a duties entry -- either would mean the consequence was never consulted",
        request: {
            let mut request = base_request();
            request.config = flat_config(&["use", "notify", "compensate"]);
            request.config.duty_mode = DutyMode::Deny;
            request.policies[0].obligations = vec![notify_with_consequence()];
            request.claims = fulfilled("compensate");
            request
        },
        patches: vec![],
        expect: allow(&base_allow_reason()).with_duties(vec![]),
    }));

    probes.push(build(Spec {
        id: "duty-consequence-itself-unresolved",
        kind: POSITIVE,
        title: "a consequence that is itself unresolved leaves dutyMode governing, and is named as a consequence",
        asserts: "The paired miss: the same obligation with no claims at all. The outstanding duty reported                   is the consequence -- what the policy now requires -- not the `notify` it replaced, and                   its source says `.consequence`.",
        falsified_by: "a duties entry naming notify, or one with no source",
        request: {
            let mut request = base_request();
            request.config = flat_config(&["use", "notify", "compensate"]);
            request.policies[0].obligations = vec![notify_with_consequence()];
            request
        },
        patches: vec![],
        expect: allow(&base_allow_reason())
            .with_duties(vec![attached_duty("compensate", "duty[0].consequence")]),
    }));

    /// The prohibition every remedy probe below varies: `use` prohibited
    /// for a US claim, carrying an `odrl:remedy` duty `anonymize`.
    fn remedy_request(request_claims: Claims) -> Request {
        let mut request = base_request();
        request.config = flat_config(&["use", "anonymize"]);
        request.policies[0].prohibitions = vec![Rule {
            remedy: vec![asserted_duty("anonymize")],
            ..rule("use", vec![c("nationality", Operator::Eq, "US")])
        }];
        // `base_request`'s unconstrained `use` permission plus this
        // prohibition is a genuine odrl:conflict collision for a US claim.
        // These probes are about a remedy never lifting a prohibition that
        // won, so the policy declares the strategy under which one does --
        // otherwise it would be void under ODRL's default and there would
        // be no deciding prohibition to hang a remedy clause off at all
        // (category 9 covers that case on its own).
        request.policies[0].conflict = ConflictStrategy::Prohibit;
        request.claims = request_claims;
        request
    }

    probes.push(build(Spec {
        id: "duty-remedy-unresolved-does-not-drop-the-prohibition",
        kind: POSITIVE,
        title: "a violated prohibition's unresolved odrl:remedy denies and leaves a trace",
        asserts: "The specific fail-open hazard this engine's README names for remedy: a violated duty                   attached to a prohibition must not drop the prohibition. It denies exactly as the bare                   prohibition would, the remedy is named in the reason, and it surfaces in duties with its                   attachment point.",
        falsified_by: "Allow, or an empty duties list, or a reason that does not mention the remedy",
        request: remedy_request(claims(&[("nationality", s("US"))])),
        patches: vec![],
        expect: deny(
            "prohibition[0] of policy 'probe' matched: action 'use': nationality eq US; its \
             odrl:remedy[0] 'anonymize' is unresolved and does not lift the prohibition; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the prohibition's favour",
        )
        .with_duties(vec![attached_duty("anonymize", "prohibition[0].remedy[0]")]),
    }));

    probes.push(build(Spec {
        id: "duty-remedy-satisfied-still-denies",
        kind: POSITIVE,
        title: "a satisfied remedy still denies -- a duty never loosens a decision in this engine",
        asserts: "The documented sub-decision, as an observation rather than a claim: one extra claim                   resolves the remedy, and the decision is the same Deny. Duties here only ever tighten a                   decision (dutyMode: deny turns Allow into Deny; nothing turns Deny into Allow), so the                   ODRL reading where a remedy substitutes for the violation is deliberately not                   implemented -- see engine/src/decision.rs::decide.",
        falsified_by: "Allow -- which would make a single host-supplied claim able to erase a prohibition",
        request: remedy_request(claims(&[("nationality", s("US")), ("duty:anonymize", s("fulfilled"))])),
        patches: vec![],
        expect: deny(
            "prohibition[0] of policy 'probe' matched: action 'use': nationality eq US; its \
             odrl:remedy[0] 'anonymize' is satisfied, which does not lift the prohibition; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the prohibition's favour",
        )
        .with_duties(vec![]),
    }));

    probes.push(build(Spec {
        id: "duty-remedy-not-reported-when-the-prohibition-does-not-fire",
        kind: NEGATIVE,
        title: "a remedy on a prohibition that never applies is not reported",
        asserts: "A remedy is what must be done *on violation*. The same policy with a DE claim leaves the                   prohibition inapplicable, and reporting its remedy would invent an obligation out of a                   rule that had nothing to say.",
        falsified_by: "a duties entry naming anonymize",
        request: remedy_request(claims(&[("nationality", s("DE"))])),
        patches: vec![],
        expect: allow(&base_allow_reason()).with_duties(vec![]),
    }));

    probes.push(build(Spec {
        id: "duty-profile-rule-class-inert",
        kind: NEGATIVE,
        title: "a profile-declared Rule class on a policy is dropped",
        asserts: "`ex:auditObligations` as a fourth rule list beside permissions/prohibitions/obligations. \
                  Identical decision, identical reason and an empty duties list, against act-base-exact's \
                  byte-identical request without it.",
        falsified_by: "any decision, reason or duties list differing from act-base-exact's",
        request: base_request(),
        patches: vec![Patch::set(
            "/policies/0",
            "ex:auditObligations",
            json!([{"action": "notify", "constraints": []}]),
        )],
        expect: allow(&base_allow_reason()).with_duties(vec![]),
    }));

    probes
}

// ---------------------------------------------------------------------
// 8 — Asset relations
// ---------------------------------------------------------------------

fn asset_probes() -> Vec<Probe> {
    /// One policy carrying a permission on `urn:asset:A` and a prohibition
    /// on `urn:asset:B`, requested for whichever of the two `requested`
    /// names — the "permission on A, prohibition on B" shape one ODRL
    /// policy expresses through each rule's own `odrl:target`. The hit/miss
    /// pair below differs in nothing but that one field.
    fn two_asset_request(requested: &str) -> Request {
        Request {
            dataset_id: requested.to_string(),
            policies: vec![WirePolicy {
                prohibitions: vec![targeted_rule("use", "urn:asset:B")],
                ..policy("probe", vec![targeted_rule("use", "urn:asset:A")])
            }],
            ..base_request()
        }
    }

    // The one category short enough to read as a literal list; the others
    // interleave shared bindings between their probes and stay pushes.
    vec![
        build(Spec {
            id: "asset-per-rule-target-hit",
            kind: POSITIVE,
            title: "a per-rule odrl:target is evaluated: a prohibition scoped to another asset does not deny",
            asserts: "One policy, permission on urn:asset:A and prohibition on urn:asset:B, requested for \
                      urn:asset:A. Allow, and the reason names the permission's own target -- the \
                      prohibition is about another asset and does not participate. Paired with \
                      asset-per-rule-target-miss, the same policy requested for urn:asset:B.",
            falsified_by: "Deny -- which would mean the prohibition's target is dropped and every rule is \
                           implicitly about dataset_id, as every rule was before per-rule targets existed",
            request: two_asset_request("urn:asset:A"),
            patches: vec![],
            expect: allow(
                "permission[0] of policy 'probe' matched: action 'use' on target 'urn:asset:A', unconstrained",
            )
            .with_dataset_id("urn:asset:A"),
        }),
        build(Spec {
            id: "asset-per-rule-target-miss",
            kind: NEGATIVE,
            title: "the same policy, requested for the prohibited asset, denies",
            asserts: "Byte-identical to asset-per-rule-target-hit except for dataset_id (the request's own \
                      target). Deny, naming the prohibition and its target: the permission on urn:asset:A \
                      no longer applies and the prohibition on urn:asset:B does.",
            falsified_by: "Allow -- which would mean the permission's own target is dropped",
            request: two_asset_request("urn:asset:B"),
            patches: vec![],
            expect: deny(
                "prohibition[0] of policy 'probe' matched: action 'use' on target 'urn:asset:B', unconstrained",
            )
            .with_dataset_id("urn:asset:B"),
        }),
        build(Spec {
            id: "asset-target-not-a-collection",
            kind: NEGATIVE,
            title: "a per-rule odrl:target is matched as an opaque string, absent an asserted odrl:partOf",
            asserts: "The permission targets urn:asset:collection and the request is for urn:asset:A, a \
                      member of it in any real catalog, but the request asserts no odrl:partOf membership \
                      via asset_collections. Deny: with no host-supplied fact to read, IRI-level \
                      inclusion is not inferred -- exactly as it was before asset_collections existed. \
                      Paired with asset-collection-membership-hit, the same shape with the membership \
                      asserted.",
            falsified_by: "Allow -- which would mean collection membership is inferred from the IRIs alone, \
                           with no host-supplied fact",
            request: Request {
                dataset_id: "urn:asset:A".to_string(),
                policies: vec![policy("probe", vec![targeted_rule("use", "urn:asset:collection")])],
                ..base_request()
            },
            patches: vec![],
            expect: deny(
                "permission[0] of policy 'probe' covers requested action 'use' but targets \
                 'urn:asset:collection', not the requested 'urn:asset:A'",
            )
            .with_dataset_id("urn:asset:A"),
        }),
        build(Spec {
            id: "asset-collection-membership-hit",
            kind: POSITIVE,
            title: "odrl:AssetCollection membership (odrl:partOf) is evaluated when the host asserts it",
            asserts: "Byte-identical to asset-target-not-a-collection except that the request's own \
                      asset_collections names urn:asset:collection as one urn:asset:A is odrl:partOf. \
                      Allow: a rule scoped to a collection IRI now covers a request for an asserted \
                      member of it, not only the collection IRI itself.",
            falsified_by: "Deny -- which would mean asset_collections is ignored and target_applies still \
                           does bare string equality against dataset_id alone",
            request: Request {
                dataset_id: "urn:asset:A".to_string(),
                policies: vec![policy("probe", vec![targeted_rule("use", "urn:asset:collection")])],
                asset_collections: vec!["urn:asset:collection".to_string()],
                ..base_request()
            },
            patches: vec![],
            expect: allow(
                "permission[0] of policy 'probe' matched: action 'use' on target 'urn:asset:collection', \
                 unconstrained",
            )
            .with_dataset_id("urn:asset:A"),
        }),
        build(Spec {
            id: "asset-collection-membership-wrong-collection-miss",
            kind: NEGATIVE,
            title: "membership in an unrelated collection does not satisfy a rule scoped to a different one",
            asserts: "Same shape as asset-collection-membership-hit, but asset_collections names a \
                      collection the rule is not scoped to. Deny: asserting *some* membership is not the \
                      same as asserting membership in the collection this rule actually names.",
            falsified_by: "Allow -- which would mean any non-empty asset_collections list satisfies any \
                           targeted rule, regardless of which collection it names",
            request: Request {
                dataset_id: "urn:asset:A".to_string(),
                policies: vec![policy("probe", vec![targeted_rule("use", "urn:asset:collection")])],
                asset_collections: vec!["urn:asset:some-other-collection".to_string()],
                ..base_request()
            },
            patches: vec![],
            expect: deny(
                "permission[0] of policy 'probe' covers requested action 'use' but targets \
                 'urn:asset:collection', not the requested 'urn:asset:A'",
            )
            .with_dataset_id("urn:asset:A"),
        }),
        build(Spec {
            id: "asset-output-ignored",
            kind: NEGATIVE,
            title: "odrl:output on a permission is dropped",
            asserts: "The asset a permitted action produces has no representation on the wire. Identical \
                      decision and reason to act-base-exact's byte-identical request without the key.",
            falsified_by: "any decision or reason differing from act-base-exact's",
            request: base_request(),
            patches: vec![Patch::set("/policies/0/permissions/0", "output", json!("urn:asset:derived"))],
            expect: allow(&base_allow_reason()),
        }),
    ]
}

// ---------------------------------------------------------------------
// 9 — Conflict strategy
// ---------------------------------------------------------------------

fn conflict_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(5);

    /// The one shape `odrl:conflict` has anything to say about: a
    /// permission and a prohibition of the same policy that both cover and
    /// match the same requested action on the same requested target.
    /// `conflict` is a real, modelled field on `engine::WirePolicy` now, so
    /// these probes set it as a typed value rather than injecting it as an
    /// unknown key through `patch.rs`.
    fn conflicting(conflict: ConflictStrategy) -> Request {
        Request {
            policies: vec![WirePolicy {
                prohibitions: vec![rule("use", vec![])],
                conflict,
                ..policy("probe", vec![rule("use", vec![])])
            }],
            ..base_request()
        }
    }
    let void_reason = || {
        "policy 'probe' is void: permission[0] and prohibition[0] both matched requested action \
         'use', and the policy's odrl:conflict strategy is 'invalid' (ODRL's own default), which \
         voids a conflicting policy rather than resolving it"
    };

    probes.push(build(Spec {
        id: "conflict-default-invalid-voids",
        kind: POSITIVE,
        title: "a policy declaring no odrl:conflict is void when a permission and a prohibition collide",
        asserts: "ODRL's own default for an undeclared conflict term is `invalid` -- void the policy -- \
                  and that is now what this engine does, rather than the unconditional \
                  prohibition-overrides earlier revisions applied. Same Deny either way, so the reason \
                  string is the whole observable: it must name the policy as void and name both \
                  colliding rules.",
        falsified_by: "Allow, or a reason reporting the prohibition as the deciding rule",
        request: conflicting(ConflictStrategy::default()),
        patches: vec![],
        expect: deny(void_reason()).excluding(&["prohibition[0] of policy 'probe' matched"]),
    }));

    probes.push(build(Spec {
        id: "conflict-invalid-declared-explicitly",
        kind: NEGATIVE,
        title: "declaring invalid explicitly is indistinguishable from declaring nothing",
        asserts: "The control for conflict-default-invalid-voids: the same request with the term written \
                  out on the wire reaches a byte-identical decision and reason, which is what makes the \
                  undeclared case a real default rather than a separate code path. The key is injected \
                  as a patch rather than set on the typed request precisely because a policy meaning the \
                  default serializes without it -- setting the field would produce the same bytes as the \
                  probe this one controls, and prove nothing.",
        falsified_by: "any decision or reason differing from conflict-default-invalid-voids's",
        request: conflicting(ConflictStrategy::Invalid),
        patches: vec![Patch::set("/policies/0", "odrl:conflict", json!("invalid"))],
        expect: deny(void_reason()).excluding(&["prohibition[0] of policy 'probe' matched"]),
    }));

    probes.push(build(Spec {
        id: "conflict-perm-allows",
        kind: POSITIVE,
        title: "odrl:conflict: perm lets the permission beat the matching prohibition",
        asserts: "The one ODRL combining rule this engine had no way to express at all until this \
                  revision: the identical policy that is void under the default Allows when it asks for \
                  permission-first resolution, and the trace names the prohibition it overrode. Read \
                  against conflict-default-invalid-voids, the byte-identical request minus the term.",
        falsified_by: "Deny -- which would mean odrl:conflict is still inert",
        request: conflicting(ConflictStrategy::Perm),
        patches: vec![],
        expect: allow(
            "permission[0] of policy 'probe' matched: action 'use', unconstrained; odrl:conflict 'perm' \
             resolves the conflict with prohibition[0] in the permission's favour",
        ),
    }));

    probes.push(build(Spec {
        id: "conflict-prohibit-denies",
        kind: POSITIVE,
        title: "odrl:conflict: prohibit is deny-overrides, now as a value a policy has to ask for",
        asserts: "What this engine did unconditionally before the term existed, reachable only by \
                  declaring it -- and saying so in the trace, so \"prohibition-first because the policy \
                  chose it\" and \"void because nothing reconciled the two\" are never the same string.",
        falsified_by: "Allow, or the void reason",
        request: conflicting(ConflictStrategy::Prohibit),
        patches: vec![],
        expect: deny(
            "prohibition[0] of policy 'probe' matched: action 'use', unconstrained; odrl:conflict \
             'prohibit' resolves the conflict with permission[0] in the prohibition's favour",
        )
        .excluding(&["is void"]),
    }));

    probes.push(build(Spec {
        id: "conflict-no-collision-inert",
        kind: NEGATIVE,
        title: "a declared strategy changes nothing for a policy with no genuine collision",
        asserts: "The scope limit of the whole feature, and the reason no vendored compliance fixture \
                  moved: `perm` on a policy carrying no prohibition at all is byte-identical to the \
                  bare baseline. Controlled by act-base-exact.",
        falsified_by: "any decision or reason differing from act-base-exact's",
        request: Request {
            policies: vec![WirePolicy { conflict: ConflictStrategy::Perm, ..policy("probe", vec![rule("use", vec![])]) }],
            ..base_request()
        },
        patches: vec![],
        expect: allow(&base_allow_reason()),
    }));

    probes.push(build(Spec {
        id: "conflict-profile-strategy-unparseable",
        kind: NEGATIVE,
        title: "a profile-declared conflict strategy cannot be selected: the request fails to parse",
        asserts: "The config declares `ex:assigneeWins` and the policy selects it. ConflictStrategy is a \
                  closed Rust enum spelling ODRL's three ConflictTerms and nothing else, so an \
                  out-of-enum token is a deserialization failure rather than a silently substituted \
                  default -- the same closed-enum posture op-isa-unparseable establishes for operators.",
        falsified_by: "Allow or Deny -- either would mean the token was tolerated and some other strategy applied",
        request: conflicting(ConflictStrategy::Prohibit),
        patches: vec![
            Patch::set("/config", "odrl:conflict", json!([{"@id": "ex:assigneeWins"}])),
            Patch::set("/policies/0", "odrl:conflict", json!("ex:assigneeWins")),
        ],
        expect: error(&[
            "request did not parse as the documented Section 5.2 JSON shape",
            "unknown variant `ex:assigneeWins`, expected one of `perm`, `prohibit`, `invalid`",
        ]),
    }));

    probes
}

// ---------------------------------------------------------------------
// 10 — Other
// ---------------------------------------------------------------------

fn other_probes() -> Vec<Probe> {
    // Capacity is this category's own probe count, asserted by
    // `the_catalog_has_fifty_two_rows_across_ten_categories`'s sibling
    // checks rather than left as a guess.
    let mut probes = Vec::with_capacity(16);

    let empty_permissions = |behaviour: Behaviour| {
        let mut request = base_request();
        request.config.behaviour = behaviour;
        request.policies[0].permissions = Vec::new();
        request
    };

    probes.push(build(Spec {
        id: "beh-open-empty",
        kind: POSITIVE,
        title: "behaviour: open makes an empty permissions list a vacuous Allow",
        asserts: "The engine's own historical, unconditional reading: an Offer with no permissions is the \
                  common harvested-data case, not the exception.",
        falsified_by: "Deny",
        request: empty_permissions(Behaviour::Open),
        patches: vec![],
        expect: allow("policy 'probe' has no permissions (open default)"),
    }));

    probes.push(build(Spec {
        id: "beh-closed-empty",
        kind: POSITIVE,
        title: "behaviour: closed denies the same empty permissions list",
        asserts: "The Community Group Formal Semantics draft's own default, and the whole point of the \
                  parameter: the identical policy reaches the opposite decision.",
        falsified_by: "Allow -- which would mean the parameter is inert",
        request: empty_permissions(Behaviour::Closed),
        patches: vec![],
        expect: deny(&closed_deny("use")).excluding(&["open default"]),
    }));

    probes.push(build(Spec {
        id: "beh-default-alias",
        kind: POSITIVE,
        title: "the draft's own \"default\" value resolves to closed",
        asserts: "The Formal Semantics draft states plainly that its `default` behaviour IS `closed`, so \
                  this is a synonym rather than a third behaviour -- and it reaches closed's decision.",
        falsified_by: "Allow, or Error",
        request: empty_permissions(Behaviour::Closed),
        patches: vec![Patch::set("/config", "behaviour", json!("default"))],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "beh-absent-defaults-open",
        kind: POSITIVE,
        title: "a request with no behaviour field at all still parses, defaulting to open",
        asserts: "Backwards compatibility with a caller built against the wire contract from before this \
                  parameter existed: the field is REMOVED from the config here, not set.",
        falsified_by: "Error (a required-field parse failure), or Deny (a changed default)",
        request: empty_permissions(Behaviour::Closed),
        patches: vec![Patch::remove("/config", "behaviour")],
        expect: allow("policy 'probe' has no permissions (open default)"),
    }));

    probes.push(build(Spec {
        id: "beh-closed-with-matching-permission",
        kind: NEGATIVE,
        title: "behaviour: closed changes nothing when a permission actually matches",
        asserts: "Bounds the knob to the one degenerate case it governs: with a real, covering, satisfied \
                  permission, closed and open agree.",
        falsified_by: "Deny -- which would mean the parameter reaches beyond the empty-list case",
        request: one(c("nationality", Operator::Eq, "DE"), claims(&[("nationality", s("DE"))])),
        patches: vec![],
        expect: allow_constrained("nationality eq DE"),
    }));

    let uid_request = || {
        let mut request = one(c("nationality", Operator::Eq, "DE"), claims(&[("nationality", s("DE"))]));
        request.policies[0].id = "https://example.org/policies/p-42".to_string();
        request
    };
    let uid_reason = "permission[0] of policy 'https://example.org/policies/p-42' matched: action 'use': nationality eq DE";

    probes.push(build(Spec {
        id: "uid-policy-in-reason",
        kind: POSITIVE,
        title: "a policy's own uid is carried and named in the reason",
        asserts: "The one identifier the engine reports back verbatim.",
        falsified_by: "a reason not naming the policy IRI",
        request: uid_request(),
        patches: vec![],
        expect: allow(uid_reason),
    }));

    probes.push(build(Spec {
        id: "uid-rule-index-not-uid",
        kind: NEGATIVE,
        title: "a rule's own uid is dropped; the reason cites a list index instead",
        asserts: "The permission carries uid urn:rule:r-7 and the reason says `permission[0]`. The \
                  substitution is deliberate, but it does mean a rule identifier a policy author chose \
                  never reaches the caller.",
        falsified_by: "a reason containing r-7",
        request: uid_request(),
        patches: vec![Patch::set("/policies/0/permissions/0", "uid", json!("urn:rule:r-7"))],
        expect: allow(uid_reason).excluding(&["r-7"]),
    }));

    probes.push(build(Spec {
        id: "uid-constraint-no-uid",
        kind: NEGATIVE,
        title: "a constraint's own uid is dropped too",
        asserts: "The constraint carries uid urn:constraint:c-9 and the reason renders the constraint by \
                  its operands instead.",
        falsified_by: "a reason containing c-9",
        request: uid_request(),
        patches: vec![Patch::set(
            "/policies/0/permissions/0/constraints/0",
            "uid",
            json!("urn:constraint:c-9"),
        )],
        expect: allow(uid_reason).excluding(&["c-9"]),
    }));

    probes.push(build(Spec {
        id: "profile-union-not-per-policy",
        kind: NEGATIVE,
        title: "a policy may use vocabulary its own declared profile never defines",
        asserts: "The config is the union of two loaded profiles (it declares both `use` and `anonymize`); \
                  the policy declares odrl:profile A, which defines only `use`; and the policy then uses \
                  `anonymize` successfully. Per-policy profile scoping would produce an Error here \
                  instead. A superset can only ever recognize more, never fewer -- a named fail-open.",
        falsified_by: "Error -- which is what correctly-scoped per-policy profile selection would give",
        request: Request {
            action: "anonymize".to_string(),
            config: flat_config(&["use", "anonymize"]),
            policies: vec![policy("probe", vec![rule("anonymize", vec![])])],
            ..base_request()
        },
        patches: vec![Patch::set("/policies/0", "profile", json!("https://example.org/profiles/A"))],
        expect: allow("permission[0] of policy 'probe' matched: action 'anonymize', unconstrained"),
    }));

    // `odrl:inheritFrom` is now a real, modelled field
    // (`engine::wire::WirePolicy::inherit_from`), so these four build it
    // as a typed value like every other supported part of a request,
    // rather than injecting it through `patch.rs`.
    //
    // **Why `parent` is scoped to a different `odrl:assignee` in all four.**
    // `evaluate_request`'s own multi-policy rule is deny-override *across
    // the whole policy set*: if `parent` were simply a second, unscoped
    // sibling of `child`, its own rule would independently decide the
    // request by itself, with or without `inheritFrom` ever being
    // resolved, for the fail-open pair especially (an unconstrained
    // prohibition on `parent` alone already denies), and the probe would
    // demonstrate nothing (confirmed empirically against the pre-fix
    // engine while writing these: the naive two-sibling shape already
    // reached the fixed pair's Deny, for that reason, not because
    // inheritance worked). Scoping `parent` to `did:web:mallory.example`
    // under a configured `partyIdentityClaim` removes it from
    // `applicable` entirely (`party_role_mismatch`) while leaving it in
    // `policies` for `inheritFrom` to still find by `id` -- so what
    // `evaluate_request` actually decides on is `child` alone.
    let isolated_parent = |rule_kind_permissions: Vec<Rule>, rule_kind_prohibitions: Vec<Rule>| WirePolicy {
        assignee: Some("did:web:mallory.example".to_string()),
        permissions: rule_kind_permissions,
        prohibitions: rule_kind_prohibitions,
        ..policy("parent", vec![])
    };
    let child_addressed_to_caller = |inherit: bool, permissions: Vec<Rule>, prohibitions: Vec<Rule>| WirePolicy {
        assignee: Some("did:web:alice.example".to_string()),
        inherit_from: inherit.then(|| vec!["parent".to_string()]),
        permissions,
        prohibitions,
        ..policy("child", vec![])
    };
    let isolated_config = |actions: &[&str], behaviour: Behaviour| {
        let mut config = flat_config(actions);
        config.behaviour = behaviour;
        config.party_identity_claim = Some("sub".to_string());
        config
    };
    let caller_is_alice = || claims(&[("sub", s("did:web:alice.example"))]);

    probes.push(build(Spec {
        id: "inheritfrom-safe-direction-hit",
        kind: POSITIVE,
        title: "under a closed default, a child inherits a parent's permission it lacks itself",
        asserts: "`parent` (party-scoped away from this caller, so only reachable through \
                  `inheritFrom`) permits `use`; `child` (addressed to this caller) permits only \
                  `notify` of its own and declares `inheritFrom: [\"parent\"]`. Real inheritance \
                  makes `child` grant `use` too.",
        falsified_by: "Deny -- which would mean inheritFrom is not resolved",
        request: Request {
            config: isolated_config(&["use", "notify"], Behaviour::Closed),
            policies: vec![
                isolated_parent(vec![rule("use", vec![])], vec![]),
                child_addressed_to_caller(true, vec![rule("notify", vec![])], vec![]),
            ],
            claims: caller_is_alice(),
            ..base_request()
        },
        patches: vec![],
        expect: allow("permission[1] of policy 'child' matched: action 'use', unconstrained"),
    }));

    probes.push(build(Spec {
        id: "inheritfrom-safe-direction-control",
        kind: NEGATIVE,
        title: "the same policies, minus inheritFrom, miss exactly as a closed default always has",
        asserts: "The control for inheritfrom-safe-direction-hit: identical policies, `child` \
                  declaring no `inheritFrom` at all. `child`'s own `notify` permission does not \
                  cover `use`, and a closed default never grants vacuously -- the ordinary miss \
                  this contract always had, proving the Allow above comes from real inheritance \
                  rather than from the closed default itself.",
        falsified_by: "Allow",
        request: Request {
            config: isolated_config(&["use", "notify"], Behaviour::Closed),
            policies: vec![
                isolated_parent(vec![rule("use", vec![])], vec![]),
                child_addressed_to_caller(false, vec![rule("notify", vec![])], vec![]),
            ],
            claims: caller_is_alice(),
            ..base_request()
        },
        patches: vec![],
        expect: deny("no permission of policy 'child' covered and matched requested action 'use' (closed default)"),
    }));

    // The fail-open direction specifically: `child` declares no rule of
    // its own at all, under `behaviour: open`'s documented vacuous-grant
    // reading for an *empty* permissions list. This is the exact shape a
    // real caller most naturally writes for "child adds nothing, inherits
    // everything" -- and, unresolved, it used to make this engine grant an
    // action `parent` explicitly prohibited.
    probes.push(build(Spec {
        id: "inheritfrom-fail-open-hit",
        kind: POSITIVE,
        title: "under the open default, a child with no rules of its own still inherits a prohibition",
        asserts: "`parent` (party-scoped away, reachable only through inheritFrom) prohibits `use`; \
                  `child` (addressed to this caller) declares neither permissions nor prohibitions \
                  of its own, only `inheritFrom: [\"parent\"]`, under `behaviour: open` -- the \
                  engine's own documented default. Before this addition, an empty child evaded an \
                  inherited prohibition entirely; see inheritfrom-fail-open-control for the exact \
                  vacuous Allow this closes.",
        falsified_by: "Allow -- which is the fail-open gap this closes",
        request: Request {
            config: isolated_config(&["use"], Behaviour::Open),
            policies: vec![
                isolated_parent(vec![], vec![rule("use", vec![])]),
                child_addressed_to_caller(true, vec![], vec![]),
            ],
            claims: caller_is_alice(),
            ..base_request()
        },
        patches: vec![],
        expect: deny("prohibition[0] of policy 'child' matched: action 'use', unconstrained"),
    }));

    probes.push(build(Spec {
        id: "inheritfrom-fail-open-control",
        kind: NEGATIVE,
        title: "the same child with no inheritFrom is the vacuous Allow this addition closes",
        asserts: "The control for inheritfrom-fail-open-hit: identical policies, `child` declaring \
                  no `inheritFrom`. With no rule of its own and nothing inherited, `behaviour: \
                  open`'s vacuous-permission reading grants -- exactly the fail-open outcome the hit \
                  probe shows this addition now prevents for a child that names the prohibiting \
                  policy as its parent.",
        falsified_by: "Deny",
        request: Request {
            config: isolated_config(&["use"], Behaviour::Open),
            policies: vec![
                isolated_parent(vec![], vec![rule("use", vec![])]),
                child_addressed_to_caller(false, vec![], vec![]),
            ],
            claims: caller_is_alice(),
            ..base_request()
        },
        patches: vec![],
        expect: allow("policy 'child' has no permissions (open default)"),
    }));

    // Information Model §2.10, validation rule 4: once inheritance itself
    // produces a merge, a parent and child that declared *differing*
    // `odrl:conflict` values over a genuine collision must void the whole
    // policy, not resolve it by either value alone. `parent` is isolated
    // exactly as the two pairs above -- reachable only through
    // `inheritFrom` -- so what `evaluate_request` decides on is `child`
    // alone, and the reason names it.
    probes.push(build(Spec {
        id: "inheritfrom-conflict-divergence-hit",
        kind: POSITIVE,
        title: "a parent and child declaring differing odrl:conflict values void the merged policy",
        asserts: "`parent` (reachable only through inheritFrom) declares `odrl:conflict: perm` \
                  alongside a permission and a prohibition that both cover `use`; `child` inherits \
                  both rules, declaring no rules of its own and its own `odrl:conflict: prohibit`. \
                  The merge now carries two distinct values over a genuine collision, which \
                  validation rule 4 requires be void -- not resolved by `child`'s own `prohibit` \
                  (which would coincidentally also deny, for the wrong reason) or `parent`'s `perm`.",
        falsified_by: "any Deny whose reason names a resolved strategy rather than voiding the \
                       policy, or an Allow",
        request: Request {
            config: isolated_config(&["use"], Behaviour::Open),
            policies: vec![
                WirePolicy {
                    conflict: ConflictStrategy::Perm,
                    ..isolated_parent(vec![rule("use", vec![])], vec![rule("use", vec![])])
                },
                WirePolicy {
                    conflict: ConflictStrategy::Prohibit,
                    ..child_addressed_to_caller(true, vec![], vec![])
                },
            ],
            claims: caller_is_alice(),
            ..base_request()
        },
        patches: vec![],
        expect: deny(
            "policy 'child' is void: permission[0] and prohibition[0] both matched requested \
             action 'use', and odrl:inheritFrom merged more than one distinct odrl:conflict \
             value into this policy (its own declared value is 'prohibit') — Information Model \
             §2.10's validation rule 4 requires the entire policy be void when a merge carries \
             differing conflict values over a genuine collision, rather than resolved by any \
             one of them",
        ),
    }));

    probes.push(build(Spec {
        id: "inheritfrom-conflict-divergence-control",
        kind: NEGATIVE,
        title: "the same shape with matching odrl:conflict on both ends is not a divergence",
        asserts: "The control for inheritfrom-conflict-divergence-hit: identical policies except \
                  `parent` also declares `odrl:conflict: prohibit`, matching `child`'s own. With \
                  nothing to disagree about, the merge resolves as an ordinary single-value \
                  `prohibit` collision always has -- proving the void reason above comes from the \
                  actual divergence, not from inheritance carrying a collision at all.",
        falsified_by: "the void reason above",
        request: Request {
            config: isolated_config(&["use"], Behaviour::Open),
            policies: vec![
                WirePolicy {
                    conflict: ConflictStrategy::Prohibit,
                    ..isolated_parent(vec![rule("use", vec![])], vec![rule("use", vec![])])
                },
                WirePolicy {
                    conflict: ConflictStrategy::Prohibit,
                    ..child_addressed_to_caller(true, vec![], vec![])
                },
            ],
            claims: caller_is_alice(),
            ..base_request()
        },
        patches: vec![],
        expect: deny(
            "prohibition[0] of policy 'child' matched: action 'use', unconstrained; \
             odrl:conflict 'prohibit' resolves the conflict with permission[0] in the \
             prohibition's favour",
        ),
    }));

    probes.push(build(Spec {
        id: "ror-literal-eq",
        kind: POSITIVE,
        title: "a right operand IRI is compared as a literal string",
        asserts: "It works exactly when the claim IS that IRI.",
        falsified_by: "Deny",
        request: one(
            c("spatial", Operator::Eq, "https://example.org/regions/eu"),
            claims(&[("spatial", s("https://example.org/regions/eu"))]),
        ),
        patches: vec![],
        expect: allow_constrained("spatial eq https://example.org/regions/eu"),
    }));

    probes.push(build(Spec {
        id: "ror-not-dereferenced",
        kind: NEGATIVE,
        title: "a right operand IRI is never dereferenced to its member values",
        asserts: "DE is a member of the EU region the right operand names. Resolving that would require \
                  fetching the IRI -- and engine.wasm is instantiated with an EMPTY import object \
                  (engine_bridge::load_engine_instance), so this is structurally impossible for the \
                  artifact this page just loaded, not merely unimplemented.",
        falsified_by: "Allow -- which would require network access the guest does not have",
        request: one(
            c("spatial", Operator::Eq, "https://example.org/regions/eu"),
            claims(&[("spatial", s("DE"))]),
        ),
        patches: vec![],
        expect: deny(&closed_deny("use")),
    }));

    probes.push(build(Spec {
        id: "ror-reference-key-ignored",
        kind: NEGATIVE,
        title: "odrl:rightOperandReference is dropped entirely",
        asserts: "The spec's own indirect form for exactly this case, injected on the constraint. The \
                  constraint's literal right operand `XX` decides instead, and misses.",
        falsified_by: "Allow -- which would mean the reference form is read",
        request: one(c("spatial", Operator::Eq, "XX"), claims(&[("spatial", s("DE"))])),
        patches: vec![Patch::set(
            "/policies/0/permissions/0/constraints/0",
            "rightOperandReference",
            json!({"@id": "https://example.org/regions/eu"}),
        )],
        expect: deny(&closed_deny("use")),
    }));

    probes
}

// ---------------------------------------------------------------------
// The assembled catalog
// ---------------------------------------------------------------------

pub fn categories() -> Vec<Category> {
    vec![
        Category { id: "actions", number: 1, title: "Actions", spec_ref: "odrl-vocab 3.12, 4.4" },
        Category { id: "left-operands", number: 2, title: "Left operands", spec_ref: "odrl-vocab 4.5" },
        Category { id: "operators", number: 3, title: "Operators", spec_ref: "odrl-vocab 2.9.4-2.9.15" },
        Category { id: "logical", number: 4, title: "Logical constraints", spec_ref: "odrl-vocab 2.10" },
        Category { id: "policy-classes", number: 5, title: "Policy classes", spec_ref: "odrl-model 2.4-2.6" },
        Category { id: "party", number: 6, title: "Party functions", spec_ref: "odrl-vocab 2.6, 4.2" },
        Category { id: "duty", number: 7, title: "Duty relations", spec_ref: "odrl-model 2.8, odrl-vocab 2.7" },
        Category { id: "assets", number: 8, title: "Asset relations", spec_ref: "odrl-model 2.3, odrl-vocab 2.5" },
        Category { id: "conflict", number: 9, title: "Conflict strategy", spec_ref: "odrl-model 2.10, odrl-vocab 2.11" },
        Category { id: "other", number: 10, title: "Other spec material", spec_ref: "odrl-model 2.2, 2.7, odrl-vocab 2.3" },
    ]
}

pub fn probes() -> Vec<Probe> {
    let mut probes = Vec::new();
    probes.extend(action_probes());
    probes.extend(left_operand_probes());
    probes.extend(operator_probes());
    probes.extend(logical_probes());
    probes.extend(policy_class_probes());
    probes.extend(party_probes());
    probes.extend(duty_probes());
    probes.extend(asset_probes());
    probes.extend(conflict_probes());
    probes.extend(other_probes());
    probes
}

struct RowSpec {
    id: &'static str,
    category: &'static str,
    term: &'static str,
    status: &'static str,
    why: &'static str,
    evidence: &'static str,
    asserts: &'static str,
    probe_ids: &'static [&'static str],
    documented_because: Option<&'static str>,
    caveat: Option<&'static str>,
}

fn row(spec: RowSpec) -> Row {
    Row {
        id: spec.id.to_string(),
        category: spec.category,
        term: spec.term.to_string(),
        status: spec.status,
        why: spec.why.to_string(),
        evidence: spec.evidence.to_string(),
        asserts: spec.asserts.to_string(),
        probe_ids: spec.probe_ids.iter().map(|id| id.to_string()).collect(),
        documented_because: spec.documented_because.map(str::to_string),
        caveat: spec.caveat.map(str::to_string),
    }
}

const IMPLEMENTED: &str = "Implemented";
const PARTIAL: &str = "Partial";
const NOT_IMPLEMENTED: &str = "NotImplemented";
const OUT_OF_SCOPE: &str = "OutOfScope";

/// The caveat every "profile-declared X extension" row carries: the probe
/// verifies the observable half only.
const EXTENSION_CAVEAT: &str = "The probe verifies the observable half -- declaring the extension changes \
                                nothing the engine does. That this is a deliberate design boundary rather \
                                than an oversight is a documented claim (engine/src/profile.rs's module \
                                doc), not something any probe can establish.";

pub fn rows() -> Vec<Row> {
    vec![
        // --- 1 Actions -------------------------------------------------
        row(RowSpec {
            id: "actions.open-vocabulary",
            category: "actions",
            term: "Action as open vocabulary (profile-declared instances)",
            status: IMPLEMENTED,
            why: "The engine ships no actions of its own; a host declares each one as an ActionDecl, \
                  exactly the Information Model's own Profile Mechanism pattern.",
            evidence: "engine/src/profile.rs::ActionDecl, profile-interpreter/src/interpret.rs",
            asserts: "A declared action matches exactly, and a declared includedIn edge is walked.",
            probe_ids: &["act-base-exact", "act-includedin-1hop"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "actions.included-in-transitive",
            category: "actions",
            term: "odrl:includedIn (transitive, declared-only)",
            status: PARTIAL,
            why: "ResolvedConfig::covers walks any declared includedIn chain, cycle-safe -- but only \
                  through edges a loaded profile actually declares.",
            evidence: "engine/src/profile.rs::ResolvedConfig::covers",
            asserts: "The pair shows both halves: a two-hop declared chain resolves; the same chain with \
                      its intermediate action left undeclared does not.",
            probe_ids: &["act-includedin-2hop", "act-includedin-undeclared-gap"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "actions.implies",
            category: "actions",
            term: "odrl:implies",
            status: NOT_IMPLEMENTED,
            why: "Only includedIn is read off an action declaration. odrl:implies is an unknown JSON key \
                  and is discarded at deserialization.",
            evidence: "engine/src/wire.rs::WireActionDecl",
            asserts: "The same taxonomy relationship expressed with odrl:implies does not cover, while \
                      the byte-neighbouring request expressing it with odrl:includedIn does.",
            probe_ids: &["act-implies-ignored", "act-includedin-1hop"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "actions.spec-taxonomy",
            category: "actions",
            term: "The vocabulary's own 51-action taxonomy",
            status: PARTIAL,
            why: "Shipped as a loadable Turtle profile document rather than baked into the engine: a host \
                  that never loads it recognizes nothing, and one that does gets every declared edge, \
                  including the two chains rooted below `use`.",
            evidence: "profile-interpreter/examples/odrl-2.2-common-actions.ttl",
            asserts: "Three chains no hand-written config in this workspace declares -- display -> play -> \
                      use, extract -> reproduce, sell -> transfer -- resolve over the real 51-term \
                      taxonomy.",
            probe_ids: &[
                "act-taxonomy-display-play-use",
                "act-taxonomy-extract-reproduce-use",
                "act-taxonomy-sell-transfer",
            ],
            documented_because: None,
            caveat: Some(
                "The 51 action declarations in these probes' config were produced by profile-interpreter \
                 parsing profile-interpreter/examples/odrl-2.2-common-actions.ttl natively at \
                 catalog-generation time; this run verifies the engine's chain resolution over them, not \
                 the Turtle parse.",
            ),
        }),
        row(RowSpec {
            id: "actions.unrecognized-is-error",
            category: "actions",
            term: "An action no loaded profile declares",
            status: IMPLEMENTED,
            why: "A rule naming an undeclared action is a configuration gap (Decision::Error), never an \
                  ordinary non-match -- deliberately stricter than the spec, which would police unknown \
                  vocabulary at validation time instead.",
            evidence: "engine/src/decision.rs::first_unrecognized_action",
            asserts: "An undeclared action errors, and that Error out-ranks a sibling policy's Allow.",
            probe_ids: &["act-unrecognized-error", "act-unrecognized-outranks-allow"],
            documented_because: None,
            caveat: None,
        }),
        // --- 2 Left operands -------------------------------------------
        row(RowSpec {
            id: "left-operands.extension",
            category: "left-operands",
            term: "Profile-declared LeftOperand extension",
            status: IMPLEMENTED,
            why: "leftOperand is a free-form key into the claims map, so an extension operand needs no \
                  registration at all.",
            evidence: "engine/src/constraint.rs::Constraint::evaluate, engine/src/claims.rs",
            asserts: "An invented operand IRI matches on a matching claim and misses on a differing one, \
                      with the full IRI rendered in the reason.",
            probe_ids: &["lo-extension-hit", "lo-extension-miss"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.datetime",
            category: "left-operands",
            term: "dateTime",
            status: PARTIAL,
            why: "Real chronological comparison, but only over a claim the host injects: the engine has \
                  no clock and cannot know what \"now\" is.",
            evidence: "engine/src/temporal.rs, engine/src/constraint.rs::ordering_matches",
            asserts: "Hit and miss on either side of the bound, and -- with no claim at all -- a miss, \
                      because there is no clock to fall back on.",
            probe_ids: &["lo-datetime-hit", "lo-datetime-miss", "lo-datetime-absent-no-clock"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.numeric",
            category: "left-operands",
            term: "count, percentage, payAmount, absoluteSize, relativeSize, resolution",
            status: PARTIAL,
            why: "Numeric comparison is real, including decimals and a rejection of non-finite lexical \
                  forms -- but count's own spec meaning is a stateful execution count, which a stateless \
                  engine cannot supply.",
            evidence: "engine/src/constraint.rs::ordering_matches",
            asserts: "Hit, miss, absent-key miss, non-numeric miss, an \"inf\" claim rejected rather than \
                      vacuously matching, and a decimal comparison.",
            probe_ids: &[
                "lo-count-hit",
                "lo-count-miss",
                "lo-count-absent-not-stateful",
                "lo-count-nonnumeric-miss",
                "lo-count-infinity-rejected",
                "lo-payamount-decimal",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.spatial",
            category: "left-operands",
            term: "spatial",
            status: PARTIAL,
            why: "Flat code/IRI equality works; there is no region containment anywhere.",
            evidence: "engine/src/claims.rs::ClaimValue::matches",
            asserts: "An exact region IRI matches, and a region genuinely contained by it does not.",
            probe_ids: &["lo-spatial-flat-hit", "lo-spatial-no-containment"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.opaque",
            category: "left-operands",
            term: "purpose, recipient, industry, media, product, event, deliveryChannel, fileFormat, language, systemDevice, virtualLocation, version",
            status: PARTIAL,
            why: "Closest to lossless under an opaque-key design -- but language needs BCP-47 range \
                  handling and event needs period ordering, neither of which exists.",
            evidence: "engine/src/constraint.rs::Constraint::evaluate",
            asserts: "purpose is lossless; language does not do range matching (en-GB vs en); event does \
                      not order periods.",
            probe_ids: &["lo-purpose-opaque-hit", "lo-language-no-bcp47", "lo-event-no-period-ordering"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.durations",
            category: "left-operands",
            term: "elapsedTime, meteredTime, delayPeriod, timeInterval",
            status: PARTIAL,
            why: "xsd:duration is now parsed to a total magnitude and ordered by lt/lteq/gt/gteq -- but \
                  Y/M components are converted at a fixed nominal length (365/30 days) rather than XSD's \
                  own genuinely partial-order semantics for calendar durations.",
            evidence: "engine/src/temporal.rs::parse_xsd_duration_nanos, engine/src/constraint.rs::ordering_matches",
            asserts: "The spec's own metering example allows; a malformed duration still misses rather \
                      than being silently accepted.",
            probe_ids: &["lo-duration-metering-example", "lo-duration-malformed-miss"],
            documented_because: None,
            caveat: Some(
                "Y/M components use a fixed nominal length (365/30 days), not XSD's own partial-order \
                 duration comparison -- see parse_xsd_duration_nanos's own doc comment.",
            ),
        }),
        row(RowSpec {
            id: "left-operands.coordinates",
            category: "left-operands",
            term: "spatialCoordinates, absolutePosition, absoluteSpatialPosition, absoluteTemporalPosition, relativePosition, relativeSpatialPosition, relativeTemporalPosition",
            status: NOT_IMPLEMENTED,
            why: "No coordinate or geometry math of any kind; a coordinate is an opaque string.",
            evidence: "engine/src/constraint.rs",
            asserts: "An exact coordinate string matches; coordinates ten metres apart do not; and \
                      ordering over positions is a miss.",
            probe_ids: &[
                "lo-coordinates-string-eq",
                "lo-coordinates-no-geometry",
                "lo-absoluteposition-no-ordering",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.unit-of-count",
            category: "left-operands",
            term: "unitOfCount",
            status: NOT_IMPLEMENTED,
            why: "There is no qualifier mechanism on a constraint: unitOfCount can only ever be another \
                  opaque claims key.",
            evidence: "engine/src/constraint.rs::Constraint (three atomic fields, no qualifier)",
            asserts: "Two mutually exclusive units over the identical count constraint reach the identical \
                      decision and reason -- and the key does work as an ordinary operand on its own.",
            probe_ids: &["lo-unitofcount-page", "lo-unitofcount-volume", "lo-unitofcount-as-plain-key"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "left-operands.policy-usage",
            category: "left-operands",
            term: "Right operand odrl:policyUsage",
            status: NOT_IMPLEMENTED,
            why: "The reserved right operand names the moment the policy was used -- execution history a \
                  stateless engine keeps none of.",
            evidence: "engine/src/constraint.rs (right_operand is a plain string)",
            asserts: "It is compared as a bare string when a host injects the claim, and means nothing at \
                      all when nobody does.",
            probe_ids: &["lo-policyusage-literal", "lo-policyusage-absent"],
            documented_because: None,
            caveat: None,
        }),
        // --- 3 Operators ------------------------------------------------
        row(RowSpec {
            id: "operators.eq",
            category: "operators",
            term: "eq",
            status: PARTIAL,
            why: "Against a multi-valued claim, eq is membership rather than identity -- a deliberate, \
                  documented adaptation, and a real divergence from spec equality.",
            evidence: "engine/src/claims.rs::ClaimValue::matches",
            asserts: "Single-valued equality; the multi-valued membership divergence; and the bound on \
                      it -- eq never splits or joins its right operand.",
            probe_ids: &["op-eq-single", "op-eq-multi-membership", "op-eq-no-concat"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "operators.neq",
            category: "operators",
            term: "neq",
            status: PARTIAL,
            why: "The same multi-value adaptation, negated. An absent claim key is a miss, not a \
                  satisfaction.",
            evidence: "engine/src/constraint.rs::Constraint::evaluate",
            asserts: "A present, differing claim satisfies it -- and an absent claim does NOT, which is \
                      the opposite of what the source gap analysis records.",
            probe_ids: &["op-neq-hit", "op-neq-absent-miss"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "operators.set-operators",
            category: "operators",
            term: "isAnyOf, isAllOf, isNoneOf",
            status: IMPLEMENTED,
            why: "All three exist and carry the documented comma-delimited right-operand adaptation, \
                  including isNoneOf's own deliberate absent-key exception.",
            evidence: "engine/src/claims.rs::matches_any/matches_all, engine/src/constraint.rs",
            asserts: "Hit and miss for each, the inexpressible comma-containing value with its own \
                      control, and isNoneOf's absent-key exception.",
            probe_ids: &[
                "op-isanyof-hit",
                "op-isanyof-miss",
                "op-isanyof-comma-unescapable",
                "op-isanyof-comma-control",
                "op-isallof-hit",
                "op-isallof-miss",
                "op-isnoneof-hit",
                "op-isnoneof-miss",
                "op-isnoneof-absent-satisfies",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "operators.is-part-of",
            category: "operators",
            term: "isPartOf",
            status: PARTIAL,
            why: "Honestly labelled rather than honestly implemented: it runs isAnyOf's exact test, so it \
                  expresses enumerated membership, never containment.",
            evidence: "engine/src/constraint.rs::Operator::IsPartOf",
            asserts: "Flat membership works; genuine hierarchy membership does not; and the operator is \
                      observationally identical to isAnyOf.",
            probe_ids: &["op-ispartof-hit", "op-ispartof-no-hierarchy", "op-ispartof-mirrors-isanyof"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "operators.ordering",
            category: "operators",
            term: "lt, lteq, gt, gteq",
            status: PARTIAL,
            why: "Real chronological ordering over xsd:dateTime and xsd:date (offsets included), an \
                  xsd:duration reading (see left-operands.durations for its own caveat), and a numeric \
                  fallback -- but no coercion between any of the three kinds.",
            evidence: "engine/src/temporal.rs, engine/src/constraint.rs::ordering_matches",
            asserts: "Fractional seconds order chronologically rather than lexically; a bare date and a \
                      numeric offset are both accepted; the strict/non-strict boundary pair differ; and a \
                      mixed-type comparison misses rather than coercing.",
            probe_ids: &[
                "op-lt-fractional-chronological",
                "op-lteq-xsd-date",
                "op-lt-offset-datetime",
                "op-gteq-numeric-boundary",
                "op-gt-numeric-boundary",
                "op-lt-mixed-type-miss",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "operators.isa-haspart",
            category: "operators",
            term: "isA, hasPart",
            status: NOT_IMPLEMENTED,
            why: "Neither is in the Operator enum. Because that enum is closed, an out-of-enum token is \
                  not silently ignored -- the request fails to parse and the engine answers Error.",
            evidence: "engine/src/constraint.rs::Operator",
            asserts: "Both tokens fail to parse, while the byte-identical request with a supported \
                      operator succeeds -- so the Error is the token, not the request.",
            probe_ids: &["op-isa-unparseable", "op-haspart-unparseable", "op-isa-control-eq"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "operators.profile-extension",
            category: "operators",
            term: "Profile-declared Operator extension",
            status: OUT_OF_SCOPE,
            why: "The wire contract has no per-profile operator registration mechanism; the enum is fixed \
                  at the engine's own compile time, and profile-interpreter warns about a declared \
                  operator rather than pretending it took effect.",
            evidence: "profile-interpreter/src/interpret.rs (the odrl:Operator warning)",
            asserts: "Declaring the extension in the config and then using it still fails to parse.",
            probe_ids: &["op-profile-operator-unparseable"],
            documented_because: None,
            caveat: Some(EXTENSION_CAVEAT),
        }),
        // --- 4 Logical constraints --------------------------------------
        row(RowSpec {
            id: "logical.and",
            category: "logical",
            term: "odrl:and",
            status: IMPLEMENTED,
            why: "A real nested field on the engine's own Constraint type, not a host-side expansion.",
            evidence: "engine/src/constraint.rs::Constraint::and",
            asserts: "Both children matching Allows, one child failing Denies, and an empty list is \
                      vacuously satisfied.",
            probe_ids: &["lc-and-both", "lc-and-one-false", "lc-and-empty-vacuous"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "logical.or",
            category: "logical",
            term: "odrl:or",
            status: IMPLEMENTED,
            why: "Likewise a real nested field, with the opposite empty-list convention to odrl:and.",
            evidence: "engine/src/constraint.rs::Constraint::or",
            asserts: "The second child alone satisfies it, no child does not, and an empty list is never \
                      satisfied.",
            probe_ids: &["lc-or-second", "lc-or-none", "lc-or-empty-never"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "logical.xone",
            category: "logical",
            term: "odrl:xone",
            status: IMPLEMENTED,
            why: "Exactly-one semantics, both boundaries -- the thing a disjunctive-normal-form expansion \
                  provably cannot express.",
            evidence: "engine/src/constraint.rs::Constraint::xone",
            asserts: "One matching child Allows, zero Denies, and TWO Denies -- against a control with \
                      the identical children and claims under odrl:or, which Allows.",
            probe_ids: &["lc-xone-exactly-one", "lc-xone-zero", "lc-xone-two-denies", "lc-or-two-allows-control"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "logical.and-sequence",
            category: "logical",
            term: "odrl:andSequence",
            status: NOT_IMPLEMENTED,
            why: "The Constraint type has and/or/xone and nothing else; andSequence is an unknown key and \
                  is discarded.",
            evidence: "engine/src/constraint.rs::RawConstraint",
            asserts: "Two requests one key name apart: odrl:and is honoured, odrl:andSequence is dropped \
                      and the constraint's own false atomic fields decide.",
            probe_ids: &["lc-andsequence-ignored", "lc-and-control-honored"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "logical.profile-extension",
            category: "logical",
            term: "Profile-declared Logical Constraint operand",
            status: OUT_OF_SCOPE,
            why: "The three logical fields are fixed on the type; a profile cannot add a fourth.",
            evidence: "engine/src/profile.rs (module doc), engine/src/constraint.rs::Constraint",
            asserts: "A declared ex:majorityOf operand, used on a constraint, is dropped like any other \
                      unknown key.",
            probe_ids: &["lc-custom-logical-ignored"],
            documented_because: None,
            caveat: Some(EXTENSION_CAVEAT),
        }),
        // --- 5 Policy classes -------------------------------------------
        row(RowSpec {
            id: "policy-classes.discrimination",
            category: "policy-classes",
            term: "Policy class discrimination (Set, Offer, Agreement, Assertion, Privacy, Request, Ticket)",
            status: NOT_IMPLEMENTED,
            why: "The wire carries `kind` as a string and nothing ever reads it; none of the \
                  class-specific MUSTs is checked.",
            evidence: "engine/src/wire.rs::WirePolicy::as_decision_policy (kind is dropped)",
            asserts: "An Agreement grants to a stranger, a Ticket carrying a forbidden assignee is \
                      accepted, and a kind that is not an ODRL class at all evaluates identically.",
            probe_ids: &[
                "pc-kind-agreement-ignores-assignee",
                "pc-kind-ticket-with-assignee",
                "pc-kind-nonsense",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "policy-classes.set-default",
            category: "policy-classes",
            term: "Set semantics as the effective default",
            status: PARTIAL,
            why: "Evaluating every policy as bare rules IS applying Set semantics to all seven classes -- \
                  a side effect worth stating outright rather than leaving implicit.",
            evidence: "engine/src/decision.rs::decide",
            asserts: "A declared Set and an unrecognisable kind reach the identical decision and reason: \
                      identical evaluation of any kind is always-Set semantics.",
            probe_ids: &["pc-kind-set", "pc-kind-nonsense"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "policy-classes.profile-subclass",
            category: "policy-classes",
            term: "Profile-declared additional Policy subclasses",
            status: OUT_OF_SCOPE,
            why: "Named directly in the engine's own scope-narrowing: profiles extend actions and left \
                  operands here, not policy classes.",
            evidence: "engine/src/profile.rs (module doc)",
            asserts: "A declared ex:ResearchLicence subclass is observationally in the same position as \
                      any other unrecognised kind -- which is the honest grouping, not a stronger claim.",
            probe_ids: &["pc-kind-profile-subclass", "pc-kind-nonsense"],
            documented_because: None,
            caveat: Some(EXTENSION_CAVEAT),
        }),
        // --- 6 Party functions ------------------------------------------
        row(RowSpec {
            id: "party.assigner-assignee",
            category: "party",
            term: "assigner, assignee",
            status: PARTIAL,
            why: "assignee is evaluated as a party scope, but only on request: a config naming \
                  partyIdentityClaim says which claim identifies the caller, and a policy whose \
                  assignee that caller does not match is then treated as absent from the request. \
                  Unset -- the default, and every request built before it existed -- both party fields \
                  are carried on the wire and dropped before the decision algorithm runs. assigner is \
                  never evaluated at all: it names who granted the policy, not who is asking.",
            evidence: "engine/src/wire.rs::party_role_mismatch, engine/src/profile.rs::ResolvedConfig::\
                       party_identity_claim",
            asserts: "With no partyIdentityClaim, a named assignee does not scope the grant and removing \
                      the field changes nothing; with one, the named assignee still gets the same answer \
                      while a stranger gets a deny that names the mismatch; and party scoping is also \
                      reachable per-rule by mirroring the party into the claims map.",
            probe_ids: &[
                "pf-assignee-mismatch",
                "pf-assignee-null-control",
                "pf-assignee-scoped-hit",
                "pf-assignee-scoped-miss",
                "pf-assignee-as-claim",
            ],
            documented_because: None,
            caveat: Some(
                "Opt-in and assignee-only. Nothing here validates an assigner, resolves an \
                 odrl:PartyCollection, or normalizes an IRI: the comparison is the engine's own `eq` \
                 semantics against one claim key -- string equality, or membership for a multi-valued \
                 claim.",
            ),
        }),
        row(RowSpec {
            id: "party.collections",
            category: "party",
            term: "Party / PartyCollection / membership",
            status: NOT_IMPLEMENTED,
            why: "Collection membership is resolved by compliance-runner against the vendored suite's \
                  state-of-the-world graph, natively, before any Request exists.",
            evidence: "compliance-runner/src/translate.rs (odrl:partOf lookups)",
            asserts: "",
            probe_ids: &[],
            documented_because: Some(
                "There is no odrl:partOf, no state-of-the-world graph and no collection concept anywhere \
                 on the wire -- nothing for evaluate() to observe, so no request can put this claim to \
                 the test. It is a native-tooling claim, verified by that crate's own tests instead.",
            ),
            caveat: None,
        }),
        row(RowSpec {
            id: "party.inverse-properties",
            category: "party",
            term: "assignerOf, assigneeOf",
            status: NOT_IMPLEMENTED,
            why: "No occurrence anywhere in the engine; unknown keys on a policy are discarded.",
            evidence: "engine/src/wire.rs::WirePolicy",
            asserts: "Both properties injected on a policy leave the decision and reason identical to the \
                      control request without them.",
            probe_ids: &["pf-assignerof-inert", "pf-assignee-null-control"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "party.common-functions",
            category: "party",
            term: "The twelve common party functions",
            status: NOT_IMPLEMENTED,
            why: "Same absence: most would only matter alongside duty-execution semantics the engine also \
                  does not have.",
            evidence: "engine/src/wire.rs::WirePolicy",
            asserts: "All twelve on one policy at once leave the decision and reason identical to the \
                      control.",
            probe_ids: &["pf-common-functions-inert", "pf-assignee-null-control"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "party.profile-roles",
            category: "party",
            term: "Profile-declared Party functional roles",
            status: OUT_OF_SCOPE,
            why: "Named in the engine's own scope-narrowing sentence.",
            evidence: "engine/src/profile.rs (module doc)",
            asserts: "The twelve-property probe is the observable half: a declared role would sit in \
                      exactly the same position -- an unknown key on a policy.",
            probe_ids: &["pf-common-functions-inert", "pf-assignee-null-control"],
            documented_because: None,
            caveat: Some(EXTENSION_CAVEAT),
        }),
        // --- 7 Duty ------------------------------------------------------
        row(RowSpec {
            id: "duty.obligation",
            category: "duty",
            term: "odrl:obligation (policy-level)",
            status: PARTIAL,
            why: "\"Satisfied\" means the duty's own constraints match the claims -- a precondition \
                  reading, since the engine cannot observe whether the action was performed. dutyMode \
                  itself is an engine invention with no ODRL counterpart.",
            evidence: "engine/src/decision.rs::unresolved_duties, Rule::duty_satisfied",
            asserts: "An unconditional duty is unresolved rather than assumed met; a claims-satisfied one \
                      resolves; and dutyMode: deny turns an unresolved duty into a Deny.",
            probe_ids: &[
                "duty-obligation-unresolved-advise",
                "duty-obligation-satisfied-by-claims",
                "duty-obligation-deny-mode",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "duty.per-permission",
            category: "duty",
            term: "odrl:duty (per-permission pre-condition)",
            status: PARTIAL,
            why: "Rule now carries odrl:duty, resolved exactly as a policy-level obligation is -- its own \
                  constraints against the claims map -- but scoped to the one permission it hangs off: \
                  under dutyMode: deny that permission does not grant, while a sibling permission still \
                  can. Partial, not Implemented, for the same reason the obligation row is: \"satisfied\" \
                  is a claims precondition, never an observation that the duty was performed.",
            evidence: "engine/src/decision.rs::Rule::duty, Rule::grants, unresolved_permission_duties",
            asserts: "Unresolved under deny stops that permission and says so in the reason; one claim \
                      resolves it; under advise it is advisory and carries its attachment point in the \
                      duties entry's source; and a sibling permission still grants, which a policy-level \
                      obligation would not have allowed.",
            probe_ids: &[
                "duty-per-permission-unresolved-deny",
                "duty-per-permission-satisfied",
                "duty-per-permission-advisory",
                "duty-per-permission-scoped-to-its-own-permission",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "duty.consequence",
            category: "duty",
            term: "odrl:consequence",
            status: PARTIAL,
            why: "A Duty's on-non-fulfilment Duty is now evaluated: an unresolved duty falls through to \
                  its consequence rather than straight to dutyMode, chained up to MAX_CONSEQUENCE_DEPTH \
                  (4) hops and treated as unresolved past that bound. Partial: ODRL permits a Duty to \
                  carry several consequences and this models one successor, because the decided semantics \
                  state no rule for combining several.",
            evidence: "engine/src/decision.rs::Rule::consequence, outstanding_duty, MAX_CONSEQUENCE_DEPTH",
            asserts: "An unfulfilled duty whose consequence the claims satisfy leaves nothing outstanding; \
                      the same duty with no claims reports the consequence -- not the duty it replaced -- \
                      with a source ending .consequence.",
            probe_ids: &["duty-consequence-resolves-where-the-primary-did-not", "duty-consequence-itself-unresolved"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "duty.remedy",
            category: "duty",
            term: "odrl:remedy",
            status: PARTIAL,
            why: "A prohibition's remedy duty is evaluated and reported, and never lifts the prohibition. \
                  Partial, and deliberately so: ODRL's reading where a performed remedy substitutes for \
                  the violation is NOT implemented, because duties in this engine only ever tighten a \
                  decision and a claims-asserted remedy able to erase a prohibition would be the first \
                  one that loosens one.",
            evidence: "engine/src/decision.rs::Rule::remedy, unresolved_remedies, decide (doc comment)",
            asserts: "An unresolved remedy denies exactly as the bare prohibition would, names itself in \
                      the reason and appears in duties with its attachment point; a satisfied one gives \
                      the same Deny with an empty duties list; and a prohibition that never fires reports \
                      no remedy at all.",
            probe_ids: &[
                "duty-remedy-unresolved-does-not-drop-the-prohibition",
                "duty-remedy-satisfied-still-denies",
                "duty-remedy-not-reported-when-the-prohibition-does-not-fire",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "duty.profile-rule-classes",
            category: "duty",
            term: "Profile-declared Rule classes",
            status: OUT_OF_SCOPE,
            why: "Three rule lists, fixed on the type; a profile cannot add a fourth.",
            evidence: "engine/src/profile.rs (module doc), engine/src/wire.rs::WirePolicy",
            asserts: "A fourth rule list injected on a policy is dropped, leaving act-base-exact's answer \
                      unchanged.",
            probe_ids: &["duty-profile-rule-class-inert", "act-base-exact"],
            documented_because: None,
            caveat: Some(EXTENSION_CAVEAT),
        }),
        // --- 8 Asset relations -------------------------------------------
        row(RowSpec {
            id: "assets.collections",
            category: "assets",
            term: "AssetCollection, odrl:partOf, odrl:source",
            status: PARTIAL,
            why: "odrl:partOf/AssetCollection membership is now a real, opt-in wire fact: the request \
                  carries its own asset_collections, naming every collection dataset_id is asserted to be \
                  odrl:partOf, and a rule's odrl:target matches a collection it names or any collection \
                  in that list (Rule::target_applies). Partial, not Implemented: this engine resolves no \
                  membership itself -- no graph, no transitive closure, no IRI normalization -- the host \
                  supplies the (already-flattened) fact, exactly as compliance-runner's own is_member_of \
                  adapter already does natively. odrl:source (an asset's own derivation provenance) is \
                  untouched by this and remains unaddressed.",
            evidence: "engine/src/wire.rs::Request::asset_collections, engine/src/decision.rs::Rule::target_applies",
            asserts: "A prohibition scoped to a collection IRI denies a request for an asserted member of \
                      it (asset-collection-membership-hit), but not a request whose asset_collections \
                      names a different collection (asset-collection-membership-wrong-collection-miss) or \
                      names none at all (asset-target-not-a-collection).",
            probe_ids: &[
                "asset-collection-membership-hit",
                "asset-collection-membership-wrong-collection-miss",
                "asset-target-not-a-collection",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "assets.target",
            category: "assets",
            term: "odrl:target (per-rule)",
            status: PARTIAL,
            why: "Each rule may carry its own odrl:target, matched against the request's own asset \
                  (Request.dataset_id); a rule naming none is about whatever is requested. Partial, not \
                  Implemented: the match is opaque string equality against dataset_id, with no IRI \
                  normalization -- and, absent an asserted odrl:partOf fact, a rule scoped to a collection \
                  IRI does not cover a member of it either (see AssetCollection, odrl:partOf above for the \
                  opt-in case where the host does assert one).",
            evidence: "engine/src/decision.rs::Rule::target, Rule::target_applies",
            asserts: "One policy, permission on urn:asset:A and prohibition on urn:asset:B: the same \
                      policy allows a request for A and denies one for B, each reason naming the target \
                      that decided it.",
            probe_ids: &["asset-per-rule-target-hit", "asset-per-rule-target-miss"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "assets.output",
            category: "assets",
            term: "odrl:output",
            status: NOT_IMPLEMENTED,
            why: "No occurrence anywhere; the asset a permitted action produces is not modelled.",
            evidence: "engine/src/decision.rs::Rule",
            asserts: "The injected key leaves act-base-exact's decision and reason unchanged.",
            probe_ids: &["asset-output-ignored", "act-base-exact"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "assets.has-policy",
            category: "assets",
            term: "odrl:hasPolicy",
            status: OUT_OF_SCOPE,
            why: "Policy-to-asset association is deliberately a host/catalog concern under this study's \
                  own harvested-catalog scope boundary.",
            evidence: "engine/src/wire.rs::Request (policies arrive already selected)",
            asserts: "",
            probe_ids: &[],
            documented_because: Some(
                "Association happens when a host SELECTS which policies to put in the request. evaluate() \
                 receives an already-selected policy set, so there is no observable difference between \
                 \"the host used hasPolicy\" and \"the host used anything else\" -- a pre-request \
                 concern no request can encode.",
            ),
            caveat: None,
        }),
        // --- 9 Conflict ---------------------------------------------------
        row(RowSpec {
            id: "conflict.property",
            category: "conflict",
            term: "odrl:conflict (perm, prohibit, invalid)",
            status: IMPLEMENTED,
            why: "A real per-policy field on the wire policy shape, read by decide(): a genuine collision \
                  -- a permission that grants and a prohibition that denies, for the same requested \
                  action and target -- is resolved by the policy's own term. All three ConflictTerms are \
                  evaluated, including `perm`, which was previously inexpressible.",
            evidence: "engine/src/decision.rs::ConflictStrategy, conflicting_rules, decide; \
                       engine/src/wire.rs::WirePolicy",
            asserts: "The same colliding policy Denies as void under the default, Allows under `perm`, \
                      and Denies naming the prohibition under `prohibit`.",
            probe_ids: &["conflict-default-invalid-voids", "conflict-perm-allows", "conflict-prohibit-denies"],
            documented_because: None,
            caveat: Some(
                "`invalid` surfaces as Deny under a distinct reason rather than as a fourth wire \
                 decision -- a void policy is a policy decision, not the configuration gap \
                 Decision::Error exists to flag. The probes read the reason string for it.",
            ),
        }),
        row(RowSpec {
            id: "conflict.fixed-strategy",
            category: "conflict",
            term: "Default for a policy that declares no odrl:conflict",
            status: IMPLEMENTED,
            why: "ODRL's own stated default, `invalid`, is the default here too. Earlier revisions applied \
                  an unconditional, unnamed prohibition-overrides instead; that was a divergence, and \
                  closing it was safe to do because no fixture in the vendored compliance corpus carries \
                  a policy with both a permission and a prohibition, so nothing in it moved.",
            evidence: "engine/src/decision.rs::<ConflictStrategy as Default>::default",
            asserts: "A conflicting policy declaring no strategy is void, and declaring `invalid` \
                      explicitly reaches a byte-identical decision and reason.",
            probe_ids: &["conflict-default-invalid-voids", "conflict-invalid-declared-explicitly"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "conflict.profile-strategies",
            category: "conflict",
            term: "Profile-declared conflict strategies",
            status: OUT_OF_SCOPE,
            why: "Named in the engine's own scope-narrowing sentence. ConflictStrategy is closed at the \
                  engine's compile time over ODRL's three ConflictTerms, so a profile-declared strategy \
                  is not silently ignored either -- it fails deserialization.",
            evidence: "engine/src/profile.rs (module doc), engine/src/decision.rs::ConflictStrategy",
            asserts: "A declared ex:assigneeWins strategy, selected by the policy, makes the request fail \
                      to parse rather than resolving under some substituted strategy.",
            probe_ids: &["conflict-profile-strategy-unparseable", "conflict-no-collision-inert"],
            documented_because: None,
            caveat: Some(EXTENSION_CAVEAT),
        }),
        // --- 10 Other ------------------------------------------------------
        row(RowSpec {
            id: "other.uid",
            category: "other",
            term: "odrl:uid",
            status: PARTIAL,
            why: "Policies only. Rules and constraints have no uid; the engine substitutes list indexes \
                  in its diagnostic output by explicit design.",
            evidence: "engine/src/wire.rs::describe_reason",
            asserts: "The policy's own uid is named in the reason, while a rule's and a constraint's are \
                      dropped in favour of an index and a rendering of the operands.",
            probe_ids: &["uid-policy-in-reason", "uid-rule-index-not-uid", "uid-constraint-no-uid"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "other.profile-property",
            category: "other",
            term: "odrl:profile (a Policy declares its conforming profile)",
            status: NOT_IMPLEMENTED,
            why: "Every request is evaluated against the union of all loaded profiles, never the specific \
                  one a policy names -- a named fail-open: a superset recognizes more, never fewer.",
            evidence: "engine/src/profile.rs::resolve (union), engine/src/wire.rs::RequestConfig",
            asserts: "A policy declaring a profile that defines only `use` successfully uses `anonymize` \
                      from the union. Per-policy scoping would produce Error here.",
            probe_ids: &["profile-union-not-per-policy"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "other.inherit-from",
            category: "other",
            term: "odrl:inheritFrom (Policy inheritance)",
            status: PARTIAL,
            why: "A child now replicates a named parent's permissions, prohibitions, obligations, \
                  and its own unset assigner/assignee, resolved by id within the same request's \
                  policies list (WirePolicy::inherit_from), with circular chains and an absent \
                  parent id rejected as Decision::Error rather than looped or silently fail-open. \
                  The one thing odrl:conflict IS read for across the chain: whether a parent's and \
                  a child's own declared value actually disagree over a genuine merged collision \
                  (Information Model 2.10 validation rule 4) -- when they do, the entire policy is \
                  void, reusing the same ConflictStrategy::Invalid machinery a single policy's own \
                  undeclared strategy already uses, rather than resolved by either value alone. \
                  Still partial, not full Section 2.9: the term itself is not replicated onto the \
                  child (no wire representation for 'unset' distinct from its own default), and \
                  this contract has no policy-level Asset or odrl:profile field to replicate in the \
                  first place.",
            evidence: "engine/src/wire.rs::resolve_inherit_from, resolve_one, WirePolicy::inherit_from",
            asserts: "Three hit/control pairs. Under a closed default, a child with only an unrelated \
                      permission of its own still grants the action its inheritFrom parent permits \
                      (inheritfrom-safe-direction-hit/-control). Under the open default, a child \
                      declaring no rule of its own at all -- the single most natural real-world \
                      inheritFrom shape -- still denies an action its parent prohibits, rather than \
                      the vacuous Allow an empty permissions list would otherwise grant \
                      (inheritfrom-fail-open-hit/-control, the direction this addition actually \
                      closes). And a parent/child pair declaring differing odrl:conflict values \
                      over a genuine collision voids the policy rather than resolving it by either \
                      value (inheritfrom-conflict-divergence-hit/-control).",
            probe_ids: &[
                "inheritfrom-safe-direction-hit",
                "inheritfrom-safe-direction-control",
                "inheritfrom-fail-open-hit",
                "inheritfrom-fail-open-control",
                "inheritfrom-conflict-divergence-hit",
                "inheritfrom-conflict-divergence-control",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "other.right-operand-reference",
            category: "other",
            term: "odrl:rightOperandReference",
            status: NOT_IMPLEMENTED,
            why: "Structurally impossible rather than merely unbuilt: a no-network wasm32 guest cannot \
                  dereference a remote IRI, and this one is instantiated with an empty import object.",
            evidence: "engine/src/constraint.rs, site/src/engine_bridge.rs::load_engine_instance",
            asserts: "An IRI right operand matches only as a literal; a value genuinely inside the \
                      referenced collection does not; and the odrl:rightOperandReference form itself is \
                      discarded.",
            probe_ids: &["ror-literal-eq", "ror-not-dereferenced", "ror-reference-key-ignored"],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "other.behaviour",
            category: "other",
            term: "Community Group Formal Semantics `Behaviour`",
            status: IMPLEMENTED,
            why: "A real, host-configurable parameter, kept outside the odrl: namespace because the draft \
                  describes it as an evaluator input rather than an RDF property. Not an ODRL 2.2 REC \
                  term.",
            evidence: "engine/src/profile.rs::Behaviour, engine/src/decision.rs::decide",
            asserts: "open and closed reach opposite decisions on the identical empty-permissions policy; \
                      the draft's own \"default\" alias resolves to closed; an absent field still parses \
                      as open; and the knob does not reach past the degenerate case.",
            probe_ids: &[
                "beh-open-empty",
                "beh-closed-empty",
                "beh-default-alias",
                "beh-absent-defaults-open",
                "beh-closed-with-matching-permission",
            ],
            documented_because: None,
            caveat: None,
        }),
        row(RowSpec {
            id: "other.error-on-unrecognized",
            category: "other",
            term: "Decision::Error for an unrecognized action (engine invention)",
            status: IMPLEMENTED,
            why: "No direct ODRL analog: a spec-conformant evaluator would police unknown vocabulary \
                  through validation (SHACL shapes), not through evaluation-time refusal. Coherent as a \
                  choice, but stricter than the spec.",
            evidence: "engine/src/decision.rs::Decision::Error, engine/src/wire.rs::evaluate_request",
            asserts: "The same two observations as the actions row, read for a different claim: the \
                      refusal happens at evaluation time, and it out-ranks every other outcome in the \
                      policy set.",
            probe_ids: &["act-unrecognized-error", "act-unrecognized-outranks-allow"],
            documented_because: None,
            caveat: None,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{evaluate_request, DutyEntry, Request as EngineRequest, WireDecision};
    use std::collections::BTreeSet;

    fn decision_str(decision: WireDecision) -> &'static str {
        match decision {
            WireDecision::Allow => "Allow",
            WireDecision::Deny => "Deny",
            WireDecision::Error => "Error",
        }
    }

    #[test]
    fn the_catalog_has_fifty_two_rows_across_ten_categories() {
        assert_eq!(rows().len(), 52);
        assert_eq!(categories().len(), 10);
    }

    #[test]
    fn every_row_id_and_probe_id_is_unique() {
        let mut row_ids = BTreeSet::new();
        for row in rows() {
            assert!(row_ids.insert(row.id.clone()), "duplicate row id {}", row.id);
        }
        let mut probe_ids = BTreeSet::new();
        for probe in probes() {
            assert!(probe_ids.insert(probe.id), "duplicate probe id {}", probe.id);
        }
    }

    #[test]
    fn every_row_names_a_real_category_and_every_category_is_used() {
        let category_ids: BTreeSet<&str> = categories().iter().map(|c| c.id).collect();
        let mut referenced: BTreeSet<&str> = BTreeSet::new();
        for row in rows() {
            assert!(category_ids.contains(row.category), "row {} names unknown category {}", row.id, row.category);
            referenced.insert(row.category);
        }
        assert_eq!(referenced, category_ids, "every category must carry at least one row");
    }

    #[test]
    fn referential_integrity_holds_in_both_directions() {
        let probe_ids: BTreeSet<&str> = probes().iter().map(|p| p.id).collect();
        let mut referenced: BTreeSet<String> = BTreeSet::new();

        for row in rows() {
            for id in &row.probe_ids {
                assert!(probe_ids.contains(id.as_str()), "row {} references unknown probe {id}", row.id);
                referenced.insert(id.clone());
            }
        }
        for id in &probe_ids {
            assert!(referenced.contains(*id), "probe {id} is referenced by no row");
        }
    }

    #[test]
    fn a_row_is_either_probed_or_documented_never_both_and_never_neither() {
        let mut documented = 0;
        for row in rows() {
            match (row.probe_ids.is_empty(), &row.documented_because) {
                (true, Some(_)) => documented += 1,
                (false, None) => {}
                _ => panic!("row {} must be exactly one of probed or documented-only", row.id),
            }
        }
        assert_eq!(
            documented, 2,
            "exactly two rows are documented-only: party collections, hasPolicy -- asset collections \
             moved to probed once Request::asset_collections gave it a real wire fact to evaluate()"
        );
    }

    #[test]
    fn every_row_carries_a_status_from_the_documented_four() {
        for row in rows() {
            assert!(
                matches!(row.status, IMPLEMENTED | PARTIAL | NOT_IMPLEMENTED | OUT_OF_SCOPE),
                "row {} has status {}",
                row.id,
                row.status
            );
        }
    }

    #[test]
    fn every_probe_carries_a_kind_a_title_and_a_falsifier() {
        for probe in probes() {
            assert!(matches!(probe.kind, "positive" | "negative"), "{}: kind {}", probe.id, probe.kind);
            assert!(!probe.title.is_empty(), "{}: empty title", probe.id);
            assert!(!probe.asserts.is_empty(), "{}: empty asserts", probe.id);
            assert!(!probe.falsified_by.is_empty(), "{}: empty falsified_by", probe.id);
            assert!(probe.request.is_object(), "{}: request is not a JSON object", probe.id);
        }
    }

    #[test]
    fn every_probes_request_is_a_complete_section_5_2_envelope() {
        for probe in probes() {
            for key in ["dataset_id", "action", "config", "policies", "claims"] {
                assert!(probe.request.get(key).is_some(), "{}: request is missing `{key}`", probe.id);
            }
        }
    }

    /// The guard the committed artifact alone cannot provide.
    ///
    /// The artifact records requests and expectations, never decisions --
    /// so `git diff --exit-code compliance/reports/` catches a *catalog*
    /// change but would NOT catch an engine change that alters a decision
    /// or a `reason` string. This test does: it drives every probe through
    /// the engine natively and checks the expectation the browser will
    /// check, so such a change fails `cargo test --workspace` here rather
    /// than reaching a visitor as a red CONTRADICTED row on the live page.
    ///
    /// Deliberately reports every divergence at once rather than the first:
    /// an engine change usually moves several probes together, and seeing
    /// them one per `cargo test` run would be miserable.
    #[test]
    fn every_probe_expectation_holds_against_the_native_engine() {
        let mut failures: Vec<String> = Vec::new();

        for probe in probes() {
            let text = serde_json::to_string(&probe.request).expect("a probe request serializes");
            let response = match serde_json::from_str::<EngineRequest>(&text) {
                Ok(request) => evaluate_request(&request),
                Err(err) => engine::parse_error_response(&err),
            };

            let observed = decision_str(response.decision);
            if observed != probe.expect.decision {
                failures.push(format!(
                    "{}: expected {}, observed {observed} -- {}",
                    probe.id, probe.expect.decision, response.reason
                ));
                continue;
            }
            for needle in &probe.expect.reason_contains {
                if !response.reason.contains(needle.as_str()) {
                    failures.push(format!("{}: reason missing `{needle}` -- got `{}`", probe.id, response.reason));
                }
            }
            for needle in &probe.expect.reason_excludes {
                if response.reason.contains(needle.as_str()) {
                    failures.push(format!("{}: reason contained excluded `{needle}`", probe.id));
                }
            }
            if let Some(expected_duties) = &probe.expect.duties {
                let observed_duties: Vec<DutyExpect> = response
                    .duties
                    .iter()
                    .map(|DutyEntry { policy_id, action, resolved, source }| DutyExpect {
                        policy_id: policy_id.clone(),
                        action: action.clone(),
                        resolved: *resolved,
                        source: source.clone(),
                    })
                    .collect();
                if &observed_duties != expected_duties {
                    failures.push(format!(
                        "{}: duties differed -- expected {expected_duties:?}, observed {observed_duties:?}",
                        probe.id
                    ));
                }
            }
            if let Some(expected_dataset_id) = &probe.expect.dataset_id {
                if &response.dataset_id != expected_dataset_id {
                    failures.push(format!(
                        "{}: dataset_id expected `{expected_dataset_id}`, observed `{}`",
                        probe.id, response.dataset_id
                    ));
                }
            }
        }

        assert!(failures.is_empty(), "{} probe expectation(s) do not hold:\n{}", failures.len(), failures.join("\n"));
    }

    /// Every positive/negative pair the catalog leans on must actually
    /// disagree. A pair whose two halves reach the same decision proves
    /// nothing, and would be very easy to introduce by a copy-paste slip.
    #[test]
    fn each_named_hit_miss_pair_reaches_opposite_decisions() {
        let all = probes();
        let decision_of = |id: &str| {
            all.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("no probe {id}")).expect.decision
        };

        for (hit, miss) in [
            ("lo-extension-hit", "lo-extension-miss"),
            ("lo-datetime-hit", "lo-datetime-miss"),
            ("lo-count-hit", "lo-count-miss"),
            ("lo-duration-metering-example", "lo-duration-malformed-miss"),
            ("lo-coordinates-string-eq", "lo-coordinates-no-geometry"),
            ("lo-spatial-flat-hit", "lo-spatial-no-containment"),
            ("op-eq-multi-membership", "op-eq-no-concat"),
            ("op-neq-hit", "op-neq-absent-miss"),
            ("op-isanyof-hit", "op-isanyof-miss"),
            ("op-isanyof-comma-control", "op-isanyof-comma-unescapable"),
            ("op-isallof-hit", "op-isallof-miss"),
            ("op-isnoneof-hit", "op-isnoneof-miss"),
            ("op-isnoneof-absent-satisfies", "op-neq-absent-miss"),
            ("op-ispartof-hit", "op-ispartof-no-hierarchy"),
            ("op-gteq-numeric-boundary", "op-gt-numeric-boundary"),
            ("op-isa-control-eq", "op-isa-unparseable"),
            ("lc-and-both", "lc-and-one-false"),
            ("lc-and-empty-vacuous", "lc-or-empty-never"),
            ("lc-or-second", "lc-or-none"),
            ("lc-xone-exactly-one", "lc-xone-zero"),
            ("lc-or-two-allows-control", "lc-xone-two-denies"),
            ("lc-and-control-honored", "lc-andsequence-ignored"),
            ("act-includedin-1hop", "act-implies-ignored"),
            ("act-includedin-2hop", "act-includedin-undeclared-gap"),
            ("beh-open-empty", "beh-closed-empty"),
            ("inheritfrom-safe-direction-hit", "inheritfrom-safe-direction-control"),
            ("inheritfrom-fail-open-control", "inheritfrom-fail-open-hit"),
            ("ror-literal-eq", "ror-not-dereferenced"),
            ("asset-per-rule-target-hit", "asset-per-rule-target-miss"),
            ("asset-collection-membership-hit", "asset-target-not-a-collection"),
            ("asset-collection-membership-hit", "asset-collection-membership-wrong-collection-miss"),
            ("pf-assignee-scoped-hit", "pf-assignee-scoped-miss"),
            ("conflict-perm-allows", "conflict-default-invalid-voids"),
            ("conflict-perm-allows", "conflict-prohibit-denies"),
        ] {
            assert_ne!(
                decision_of(hit),
                decision_of(miss),
                "the pair {hit} / {miss} must reach opposite decisions, or it proves nothing"
            );
        }
    }

    /// The inert-property probes are only meaningful against a control
    /// that reaches the identical answer. Asserted here rather than left
    /// to a reader comparing two table rows by eye.
    #[test]
    fn each_named_inert_property_probe_matches_its_control_exactly() {
        let all = probes();
        let expect_of = |id: &str| {
            let probe = all.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("no probe {id}"));
            (probe.expect.decision, probe.expect.reason_contains.clone())
        };

        for (injected, control) in [
            ("duty-profile-rule-class-inert", "act-base-exact"),
            ("asset-output-ignored", "act-base-exact"),
            ("pf-assignerof-inert", "pf-assignee-null-control"),
            ("pf-common-functions-inert", "pf-assignee-null-control"),
            ("pf-assignee-mismatch", "pf-assignee-null-control"),
            ("pc-kind-agreement-ignores-assignee", "pc-kind-set"),
            ("pc-kind-nonsense", "pc-kind-set"),
            ("pc-kind-profile-subclass", "pc-kind-nonsense"),
            ("conflict-invalid-declared-explicitly", "conflict-default-invalid-voids"),
            ("conflict-no-collision-inert", "act-base-exact"),
            ("lo-unitofcount-volume", "lo-unitofcount-page"),
        ] {
            assert_eq!(
                expect_of(injected),
                expect_of(control),
                "{injected} must expect exactly what its control {control} expects, or the pair shows \
                 nothing about the injected property"
            );
        }
    }

    /// The determinism guard asserted at the point that actually provides
    /// it: `build`'s own `serde_json::to_value(&request)`.
    ///
    /// Measured, not assumed — a throwaway binary against this
    /// workspace's `engine`, serializing one two-claim `Request` per
    /// process across eight processes, emitted the claims keys in a
    /// different order in three of them; through `to_value`, never. Two
    /// `HashMap::new()` calls get independently randomized `RandomState`s,
    /// so the two maps below are a real instance of that same
    /// non-determinism rather than a staged one.
    #[test]
    fn two_independently_randomized_claims_maps_build_to_identical_probe_json() {
        let build_with = |claims: Claims| {
            build(Spec {
                id: "determinism",
                kind: POSITIVE,
                title: "t",
                asserts: "a",
                falsified_by: "f",
                request: Request { claims, ..base_request() },
                patches: vec![],
                expect: allow("x"),
            })
            .request
            .to_string()
        };

        let mut a: Claims = std::collections::HashMap::new();
        a.insert("sub".to_string(), s("alice"));
        a.insert("dateTime".to_string(), s("2026-09-05T12:00:00Z"));
        a.insert("nationality".to_string(), s("DE"));
        a.insert("scope".to_string(), m(&["read", "write"]));

        let mut b: Claims = std::collections::HashMap::new();
        b.insert("scope".to_string(), m(&["read", "write"]));
        b.insert("nationality".to_string(), s("DE"));
        b.insert("dateTime".to_string(), s("2026-09-05T12:00:00Z"));
        b.insert("sub".to_string(), s("alice"));

        assert_eq!(
            build_with(a),
            build_with(b),
            "two logically identical claims maps, built through independently-randomized HashMap \
             instances, must produce byte-identical probe JSON"
        );
    }

    /// Belt and braces on the same property, across the whole committed
    /// catalog rather than one synthetic probe: generating it twice in one
    /// process must produce identical bytes. (The five-run,
    /// five-*process* checksum comparison is in this phase's verification
    /// notes; this is the part that can live in CI.)
    #[test]
    fn generating_the_whole_catalog_twice_produces_identical_bytes() {
        let first = probes();
        let second = probes();
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.request.to_string(), b.request.to_string(), "probe {} is not stable", a.id);
        }
    }

    #[test]
    fn a_patch_whose_pointer_does_not_resolve_panics_naming_the_probe() {
        let result = std::panic::catch_unwind(|| {
            build(Spec {
                id: "deliberately-broken",
                kind: NEGATIVE,
                title: "t",
                asserts: "a",
                falsified_by: "f",
                request: base_request(),
                patches: vec![Patch::set("/policies/9", "conflict", json!("perm"))],
                expect: allow("x"),
            })
        });
        assert!(result.is_err(), "a patch that does not land must fail generation, never pass silently");
    }
}
