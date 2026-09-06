//! ODRL Profiles: the action/vocabulary extension mechanism this engine
//! actually implements (case study Section 4.4, extended to resolve real
//! `odrl:includedIn` action-taxonomy coverage rather than exact-string
//! matching only — see `ResolvedConfig::covers`'s doc comment for what
//! that does and does not mean).
//!
//! A `Profile` is this engine's own deliberately narrowed reading of W3C
//! ODRL's Profile Mechanism (Section 3.5) — "additional Actions"
//! (`actions`, each optionally naming a broader action it's
//! `odrl:includedIn`) plus a `duty_mode` setting that Section 4.5's duty
//! evaluation consumes. Nothing here claims the rest of that mechanism
//! (additional Policy subclasses, Party functional roles, Logical
//! Constraint operands, Rule classes, profile-declared conflict
//! strategies).
//!
//! That last one is a narrower exclusion than it used to be. ODRL's own
//! three `odrl:ConflictTerm`s are now really evaluated, per policy
//! (`decision::ConflictStrategy`) — what stays out of scope is a *profile*
//! declaring a strategy of its own (`ex:assigneeWins`), which this engine
//! cannot select because the enum is closed at its compile time over those
//! three.

use serde::{Deserialize, Serialize};

/// How this engine's Section 4.5 duty evaluation should treat an
/// unresolved duty. `Deny` is the conservative setting (XACML's own
/// stated principle: an unenforceable Obligation forces Deny); `Advise`
/// surfaces it to the caller instead of blocking Allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DutyMode {
    #[serde(rename = "advise")]
    Advise,
    #[serde(rename = "deny")]
    Deny,
}

/// The ODRL Community Group's own named axis (case study Section 3.6,
/// its Formal Semantics draft report): what a policy with no matching
/// permission at all should mean. `Open` ("anything not permitted is
/// unaffected") is Section 4.3's own, deliberate departure from a fully
/// closed-world reading — the engine's long-standing default, because an
/// `Offer` with an empty `permissions` list is the common harvested-data
/// case, not the exception. `Closed` ("anything not explicitly permitted
/// is denied") is the Formal Semantics draft's own stated default, and
/// exactly what a caller wanting XACML's `deny-unless-permit` posture, or
/// matching an external ODRL evaluator's own closed-world ground truth
/// (as this engine's own compliance suite does), should choose instead.
///
/// This governs *only* the empty-`permissions`-list case — an explicit,
/// covering, but unsatisfied permission still denies under either
/// setting; a matching prohibition still denies under either setting.
/// Named `Behaviour` (not, say, `EmptyPermissionsMode`) deliberately
/// matching the Formal Semantics draft's own vocabulary — Section 3.6
/// already discusses it under that name, and this is that same knob,
/// finally a real, host-configurable parameter rather than a fixed
/// choice baked into the algorithm.
///
/// Deserializes the draft's own third value, `"default"`, as `Closed` —
/// the draft states plainly that `default` *is* `closed`, so this is not
/// a third behavior, just the spec's own synonym; serializing never
/// re-emits it, since "default" names a resolved value, not a distinct
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Behaviour {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "closed", alias = "default")]
    Closed,
}

impl Default for Behaviour {
    /// `Open` — Section 4.3's own original, unconditional choice, before
    /// this became a real parameter. Preserved as the default so an
    /// existing caller that never sets this sees no behavior change.
    fn default() -> Self {
        Behaviour::Open
    }
}

/// One action a profile declares — `ex:myAction a odrl:Action`, optionally
/// with `odrl:includedIn` naming a broader parent action, exactly the W3C
/// ODRL Information Model's own Profile Mechanism pattern
/// (<https://www.w3.org/TR/odrl-model/#profile-mechanism>): "Create an
/// instance of an Action and define its includedIn parent Action."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDecl {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub included_in: Option<String>,
}

impl ActionDecl {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), included_in: None }
    }

    pub fn included_in(id: impl Into<String>, parent: impl Into<String>) -> Self {
        Self { id: id.into(), included_in: Some(parent.into()) }
    }
}

/// One loaded ODRL Profile (Section 4.4's JSON shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub actions: Vec<ActionDecl>,
    pub duty_mode: DutyMode,
    #[serde(default)]
    pub behaviour: Behaviour,
}

/// The merged configuration a broker resolves **once, at its own
/// startup** from every profile it loads, and includes in every request
/// (Section 4.4). Resolving per-request would work identically here —
/// this type carries no state of its own — but the case study is explicit
/// that the *intended* lifecycle is host-startup-time, not per-request,
/// even though the engine itself stays stateless either way.
///
/// `actions` is deliberately not `pub`: every real use is either "is this
/// action known at all" (`recognizes`) or "does this rule's action cover
/// that requested one" (`covers`), and exposing the raw list would invite
/// a caller to reimplement one of those two queries slightly differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    actions: Vec<ActionDecl>,
    pub duty_mode: DutyMode,
    pub behaviour: Behaviour,
    /// **Which key of the caller's claims map carries the caller's own
    /// identity** — and, by naming it, the switch that turns party-role
    /// evaluation on at all. `None` (the default from every construction
    /// path here, `resolve` included) means party roles are not evaluated:
    /// a policy's `odrl:assignee` is carried on the wire and consulted by
    /// nothing, exactly as it was before this field existed.
    ///
    /// `Some(key)` means a policy naming an `odrl:assignee` applies only to
    /// a caller whose `claims[key]` matches it; see
    /// `wire::party_role_mismatch` for the comparison's exact semantics and
    /// `wire::evaluate_request` for what a mismatch does to the decision.
    /// **Only the wire layer reads this**, because only `wire::WirePolicy`
    /// carries a party at all — `decision::Policy` deliberately does not,
    /// and `decision::decide` is therefore unaffected by this setting.
    ///
    /// **Deliberately not configurable, and not resolvable, from a
    /// `Profile`**, unlike `duty_mode` and `behaviour` beside it. Those two
    /// are statements about how policies should be *evaluated*, which is
    /// what a profile document is for. This is a statement about the shape
    /// of the host's own claims map — which key its identity plumbing puts
    /// the caller's identifier under — and that is host deployment
    /// configuration, not something a published, shareable ODRL profile can
    /// or should assert about someone else's identity provider. A host names
    /// it here (`with_party_identity_claim`) or on the wire
    /// (`wire::RequestConfig::party_identity_claim`), and nowhere else.
    pub party_identity_claim: Option<String>,
    /// **Which key of the caller's claims map must match an
    /// `odrl:Agreement` policy's own `odrl:assignee`** — a second, fully
    /// independent on/off switch from `party_identity_claim` above, scoped
    /// to `kind == "Agreement"` only.
    ///
    /// `party_identity_claim` is opt-in for every `kind` alike, which is
    /// correct only once a host has asked for it generally. ODRL 2.2
    /// Vocabulary & Expression §3.2.1 states the Agreement's own MUST
    /// unconditionally ("The Agreement Policy will grant the terms of the
    /// Policy from the Assigner to the Assignee") — a host that has
    /// configured nothing still leaves that MUST unenforced today, and
    /// turning on `party_identity_claim` to close it would also start
    /// scoping every other `kind`, which nothing in the spec asks for. This
    /// field closes exactly the Agreement gap and nothing wider: `None` (the
    /// default from every construction path here) means unaffected, exactly
    /// as `party_identity_claim` unset does; `Some(key)` means a policy with
    /// `kind == "Agreement"` naming an `odrl:assignee` applies only to a
    /// caller whose `claims[key]` matches it, using the identical
    /// `ClaimValue::matches` comparison `party_identity_claim` already uses
    /// — see `wire::party_role_mismatch`.
    ///
    /// Independent of `party_identity_claim` deliberately: a host may
    /// configure either, both, or neither. Both configured and both
    /// applying to the same mismatched Agreement simply both exclude it —
    /// there is no merged reason to construct, and no precedence to define,
    /// because either check alone is already sufficient to exclude the
    /// policy.
    ///
    /// Same non-`Profile` scoping as `party_identity_claim`, for the same
    /// reason: this names a shape of the host's own claims map, not
    /// something a published ODRL profile can assert.
    pub agreement_assignee_claim: Option<String>,
}

impl ResolvedConfig {
    pub fn new(actions: Vec<ActionDecl>, duty_mode: DutyMode, behaviour: Behaviour) -> Self {
        Self { actions, duty_mode, behaviour, party_identity_claim: None, agreement_assignee_claim: None }
    }

    /// Turns party-role evaluation on, naming the claim key that carries
    /// the caller's identity (see `party_identity_claim`).
    ///
    /// A consuming builder rather than a fourth parameter of `new`
    /// deliberately: `new` is public and already called from outside this
    /// module, and this capability is opt-in — a host that never calls this
    /// gets the behaviour it has always had, and does not have to be
    /// recompiled against a wider signature to keep it.
    pub fn with_party_identity_claim(mut self, claim: impl Into<String>) -> Self {
        self.party_identity_claim = Some(claim.into());
        self
    }

    /// Turns Agreement-assignee enforcement on, naming the claim key an
    /// `odrl:Agreement`'s `odrl:assignee` must match (see
    /// `agreement_assignee_claim`). Independent of, and composable with,
    /// `with_party_identity_claim` — a consuming builder for the same
    /// reason that one is: opt-in, and not a wider constructor signature
    /// every existing caller of `new` would have to grow to keep.
    pub fn with_agreement_assignee_claim(mut self, claim: impl Into<String>) -> Self {
        self.agreement_assignee_claim = Some(claim.into());
        self
    }

    /// Is `action` declared at all, by any loaded profile — Section 4.4's
    /// original vocabulary-validity check, unchanged in meaning: a rule
    /// naming an action no loaded profile even knows about is a
    /// configuration gap (`Decision::Error`), not an ordinary non-match.
    pub fn recognizes(&self, action: &str) -> bool {
        self.actions.iter().any(|a| a.id == action)
    }

    /// Every action id some loaded profile actually declared, in
    /// declaration order — exactly the set `recognizes` above answers
    /// `true` for, and nothing more.
    ///
    /// **A third query, deliberately, not a leak of `actions`.** This type's
    /// own doc comment says `actions` stays private because every real use
    /// is either "is this action known at all" (`recognizes`) or "does this
    /// rule's action cover that requested one" (`covers`), and handing out
    /// the raw `Vec<ActionDecl>` would invite a caller to reimplement one of
    /// those two slightly differently. That reasoning is intact:
    /// `decision::performable_actions` needs a genuinely third query —
    /// *enumerate* the vocabulary, in order to ask `decide` about each of
    /// its members — which neither of the other two can express. What this
    /// returns is bare `&str` ids, **not** `ActionDecl`s: the
    /// `odrl:includedIn` edges stay behind `covers`, so no caller can walk
    /// the taxonomy itself and arrive at a second, divergent notion of
    /// coverage.
    ///
    /// Returned in declaration order and **not** deduplicated, because this
    /// is a report of what was declared: `resolve` already dedupes by id
    /// when it builds a config from profiles, but `ResolvedConfig::new` is
    /// public and does not, so a hand-built config with a repeated id
    /// reports that repeat. A caller wanting a set says so for itself —
    /// `performable_actions` does exactly that.
    ///
    /// An action named only as some other action's `odrl:includedIn` parent,
    /// and never declared as an `ActionDecl` of its own, is not here: it is
    /// not `recognizes`d either, and a rule naming it is `Decision::Error`.
    pub fn declared_actions(&self) -> Vec<&str> {
        self.actions.iter().map(|a| a.id.as_str()).collect()
    }

    /// Does a rule declaring `rule_action` cover a request for
    /// `requested_action` — either the same action, or `requested_action`
    /// reaching `rule_action` through a chain of `odrl:includedIn` edges
    /// declared across every loaded profile (e.g. a permission for the
    /// broad `odrl:transfer` covers a request for `odrl:sell`, since
    /// `odrl:sell` is declared `odrl:includedIn odrl:transfer` — the same
    /// worked example Section 3.5 itself gives: "`transfer` implies `give`
    /// and `sell`").
    ///
    /// This is the general mechanism the case study's Section 7 named as
    /// missing ("Action implication is not evaluated") and
    /// `compliance-runner`'s own adapter previously special-cased for
    /// exactly one vocabulary fact (`odrl:use` vs. the transfer category)
    /// as a host-side workaround. That workaround is now redundant, not
    /// merely superseded — the engine resolves any *declared*
    /// `includedIn` chain, not only that one pair.
    ///
    /// Deliberately narrower than full inference: an action not
    /// separately declared as its own `ActionDecl` contributes nothing to
    /// this closure even if some other declared action's `includedIn`
    /// would seem to reach it — every step of the chain must be a real,
    /// declared edge, so a typo'd or unlisted action can never silently
    /// become "covered." A cycle in declared edges (which nothing here
    /// prevents a profile from asserting) terminates the walk as
    /// non-covering rather than looping forever.
    pub fn covers(&self, rule_action: &str, requested_action: &str) -> bool {
        if rule_action == requested_action {
            return true;
        }
        let mut current = requested_action.to_string();
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return false;
            }
            match self.actions.iter().find(|a| a.id == current) {
                Some(ActionDecl { included_in: Some(parent), .. }) => {
                    if parent == rule_action {
                        return true;
                    }
                    current = parent.clone();
                }
                _ => return false,
            }
        }
    }
}

/// Resolves multiple loaded profiles into one `ResolvedConfig`: the
/// **union** of every profile's declared `actions` (deduplicated by
/// `id` — the first profile to declare a given action's `included_in`
/// wins if two profiles disagree, an edge case nothing in this corpus
/// exercises and not worth a more elaborate merge rule for), and the
/// **strictest** (`deny` beats `advise`) of every profile's `duty_mode`
/// (Section 4.4), and — the same "strictest wins" rule — the **strictest**
/// (`closed` beats `open`) of every profile's `behaviour`.
///
/// The union choice is a *named* fail-open limitation (Section 7): this
/// engine has no way to scope a request to only the profile(s) a specific
/// Policy actually declares via ODRL's `profile` property, so every
/// request is evaluated against the union of *all* loaded profiles. A
/// superset can only recognize/cover more actions than a correctly-scoped
/// per-policy selection would, never fewer — safe in the sense that it
/// never manufactures a spurious `Error`, but coarser than real per-policy
/// profile scoping. This function implements that choice as specified,
/// not a narrower alternative.
///
/// Zero loaded profiles resolves to no declared actions (every action
/// becomes unrecognized) and the least-strict `duty_mode` (`Advise`) —
/// there is nothing to be strict *about* when nothing was loaded.
pub fn resolve(profiles: &[Profile]) -> ResolvedConfig {
    let mut actions: Vec<ActionDecl> = Vec::new();
    let mut duty_mode = DutyMode::Advise;
    let mut behaviour = Behaviour::Open;

    for profile in profiles {
        for action in &profile.actions {
            if !actions.iter().any(|a| a.id == action.id) {
                actions.push(action.clone());
            }
        }
        if profile.duty_mode == DutyMode::Deny {
            duty_mode = DutyMode::Deny;
        }
        if profile.behaviour == Behaviour::Closed {
            behaviour = Behaviour::Closed;
        }
    }

    // `party_identity_claim` and `agreement_assignee_claim` are both
    // deliberately absent from this merge: no ODRL Profile document
    // declares which claim key carries the caller's identity (see each
    // field's own doc comment), so resolving profiles can never switch
    // either check on by itself. A host that wants one or both chains
    // `ResolvedConfig::with_party_identity_claim` and/or
    // `with_agreement_assignee_claim` onto this call, or sets
    // `partyIdentityClaim`/`agreementAssigneeClaim` on the wire.
    ResolvedConfig { actions, duty_mode, behaviour, party_identity_claim: None, agreement_assignee_claim: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, actions: &[ActionDecl], duty_mode: DutyMode) -> Profile {
        Profile { id: id.to_string(), actions: actions.to_vec(), duty_mode, behaviour: Behaviour::Open }
    }

    fn profile_with_behaviour(id: &str, actions: &[ActionDecl], behaviour: Behaviour) -> Profile {
        Profile { id: id.to_string(), actions: actions.to_vec(), duty_mode: DutyMode::Advise, behaviour }
    }

    fn flat(names: &[&str]) -> Vec<ActionDecl> {
        names.iter().map(|n| ActionDecl::new(*n)).collect()
    }

    #[test]
    fn resolves_the_union_of_recognized_actions_across_profiles() {
        let profiles = vec![
            profile("p1", &flat(&["use", "distribute"]), DutyMode::Advise),
            profile("p2", &flat(&["distribute", "modify"]), DutyMode::Advise),
        ];
        let resolved = resolve(&profiles);
        assert!(resolved.recognizes("use"));
        assert!(resolved.recognizes("distribute"));
        assert!(resolved.recognizes("modify"));
        assert!(!resolved.recognizes("delete"));
    }

    #[test]
    fn resolves_the_strictest_duty_mode_deny_beats_advise_either_order() {
        let advise_then_deny = vec![
            profile("p1", &flat(&["use"]), DutyMode::Advise),
            profile("p2", &flat(&["distribute"]), DutyMode::Deny),
        ];
        assert_eq!(resolve(&advise_then_deny).duty_mode, DutyMode::Deny);

        let deny_then_advise = vec![
            profile("p1", &flat(&["use"]), DutyMode::Deny),
            profile("p2", &flat(&["distribute"]), DutyMode::Advise),
        ];
        assert_eq!(resolve(&deny_then_advise).duty_mode, DutyMode::Deny);
    }

    #[test]
    fn all_advise_profiles_resolve_to_advise() {
        let profiles = vec![
            profile("p1", &flat(&["use"]), DutyMode::Advise),
            profile("p2", &flat(&["distribute"]), DutyMode::Advise),
        ];
        assert_eq!(resolve(&profiles).duty_mode, DutyMode::Advise);
    }

    #[test]
    fn no_profiles_recognizes_nothing() {
        let resolved = resolve(&[]);
        assert!(!resolved.recognizes("use"));
        assert_eq!(resolved.duty_mode, DutyMode::Advise);
    }

    #[test]
    fn deserializes_from_the_documented_json_shape() {
        let json = r#"{
            "id": "https://example.org/profiles/default",
            "actions": [
                {"id": "use"},
                {"id": "distribute", "included_in": "use"},
                {"id": "modify"},
                {"id": "notify"}
            ],
            "duty_mode": "advise"
        }"#;
        let profile: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.id, "https://example.org/profiles/default");
        assert_eq!(profile.actions.len(), 4);
        assert_eq!(profile.actions[1].included_in.as_deref(), Some("use"));
        assert_eq!(profile.duty_mode, DutyMode::Advise);
        assert_eq!(
            profile.behaviour,
            Behaviour::Open,
            "a profile document from before this parameter existed must still deserialize, defaulting to Open"
        );
    }

    #[test]
    fn covers_exact_match_with_no_included_in_declared_at_all() {
        let config = resolve(&[profile("p1", &flat(&["use"]), DutyMode::Advise)]);
        assert!(config.covers("use", "use"));
        assert!(!config.covers("use", "distribute"));
    }

    #[test]
    fn covers_the_section_3_5_worked_example_transfer_implies_give_and_sell() {
        let config = resolve(&[profile(
            "p1",
            &[
                ActionDecl::new("transfer"),
                ActionDecl::included_in("sell", "transfer"),
                ActionDecl::included_in("give", "transfer"),
            ],
            DutyMode::Advise,
        )]);
        assert!(config.covers("transfer", "sell"), "a permission for transfer must cover a request for sell");
        assert!(config.covers("transfer", "give"), "a permission for transfer must cover a request for give");
        assert!(!config.covers("sell", "give"), "sell and give are siblings, neither covers the other");
        assert!(!config.covers("transfer", "use"), "an unrelated action is not covered just because something else is");
    }

    #[test]
    fn covers_transitively_across_more_than_one_included_in_hop() {
        let config = resolve(&[profile(
            "p1",
            &[
                ActionDecl::new("use"),
                ActionDecl::included_in("distribute", "use"),
                ActionDecl::included_in("redistribute", "distribute"),
            ],
            DutyMode::Advise,
        )]);
        assert!(
            config.covers("use", "redistribute"),
            "use -> distribute -> redistribute: a permission for the top of the chain covers the bottom"
        );
        assert!(!config.covers("distribute", "use"), "coverage does not run backwards up the chain");
    }

    #[test]
    fn covers_resolves_one_hop_of_a_two_node_cycle_directly() {
        // "a includedIn b" and "b includedIn a" together still let a
        // single hop resolve correctly in either direction — this is not
        // yet the runaway case, just confirming a cycle's edges are each
        // individually honored rather than rejected outright.
        let config = resolve(&[profile(
            "p1",
            &[ActionDecl::included_in("a", "b"), ActionDecl::included_in("b", "a")],
            DutyMode::Advise,
        )]);
        assert!(config.covers("a", "b"), "b includedIn a is a real, single-hop declared edge");
        assert!(config.covers("b", "a"), "a includedIn b is a real, single-hop declared edge");
    }

    #[test]
    fn covers_returns_false_rather_than_looping_forever_when_a_cycle_never_reaches_the_query() {
        // Nothing in `resolve`/the interpreter prevents a profile from
        // asserting a cycle; walking from "z" (not part of the a/b cycle
        // at all) must terminate as non-covering, not hang, once the walk
        // revisits a node it has already seen.
        let config = resolve(&[profile(
            "p1",
            &[ActionDecl::included_in("a", "b"), ActionDecl::included_in("b", "a")],
            DutyMode::Advise,
        )]);
        assert!(!config.covers("z", "a"));
        assert!(!config.covers("z", "b"));
    }

    #[test]
    fn covers_does_not_reach_through_an_undeclared_intermediate_action() {
        // "redistribute" claims to be includedIn "distribute", but nothing
        // in this loaded set declares "distribute" itself as an action —
        // the chain must not silently keep walking past a gap.
        let config = resolve(&[profile(
            "p1",
            &[ActionDecl::new("use"), ActionDecl::included_in("redistribute", "distribute")],
            DutyMode::Advise,
        )]);
        assert!(!config.covers("use", "redistribute"));
    }

    #[test]
    fn resolve_keeps_the_first_profiles_edge_when_two_profiles_disagree() {
        let profiles = vec![
            profile("p1", &[ActionDecl::included_in("sell", "transfer")], DutyMode::Advise),
            profile("p2", &[ActionDecl::new("sell")], DutyMode::Advise),
        ];
        let config = resolve(&profiles);
        assert!(config.covers("transfer", "sell"), "p1's edge (declared first) wins over p2's bare re-declaration");
    }

    #[test]
    fn declared_actions_lists_exactly_the_ids_recognizes_accepts_in_declaration_order() {
        let config = resolve(&[profile(
            "p1",
            &[
                ActionDecl::new("use"),
                ActionDecl::included_in("distribute", "use"),
                ActionDecl::included_in("sell", "transfer"),
            ],
            DutyMode::Advise,
        )]);
        assert_eq!(
            config.declared_actions(),
            vec!["use", "distribute", "sell"],
            "every declared ActionDecl's id, in declaration order"
        );
        assert!(
            !config.declared_actions().contains(&"transfer"),
            "an action named only as someone else's includedIn target is never separately \
             declared, so recognizes() rejects it and this must not list it either"
        );
        for action in config.declared_actions() {
            assert!(config.recognizes(action), "declared_actions and recognizes must agree");
        }
    }

    #[test]
    fn declared_actions_is_empty_when_no_profile_was_loaded() {
        assert!(resolve(&[]).declared_actions().is_empty());
    }

    #[test]
    fn declared_actions_reports_a_duplicate_id_a_caller_built_by_hand() {
        // `resolve` dedupes, but `ResolvedConfig::new` is public and does
        // not: this accessor reports the raw declaration list, and callers
        // that need a set (`performable_actions`) dedupe for themselves.
        let config = ResolvedConfig::new(
            vec![ActionDecl::new("use"), ActionDecl::new("use")],
            DutyMode::Advise,
            Behaviour::Open,
        );
        assert_eq!(config.declared_actions(), vec!["use", "use"]);
    }

    #[test]
    fn no_profiles_resolves_to_open_the_original_unconditional_default() {
        assert_eq!(resolve(&[]).behaviour, Behaviour::Open);
    }

    #[test]
    fn resolves_the_strictest_behaviour_closed_beats_open_either_order() {
        let open_then_closed =
            vec![profile_with_behaviour("p1", &flat(&["use"]), Behaviour::Open), profile_with_behaviour("p2", &flat(&["distribute"]), Behaviour::Closed)];
        assert_eq!(resolve(&open_then_closed).behaviour, Behaviour::Closed);

        let closed_then_open =
            vec![profile_with_behaviour("p1", &flat(&["use"]), Behaviour::Closed), profile_with_behaviour("p2", &flat(&["distribute"]), Behaviour::Open)];
        assert_eq!(resolve(&closed_then_open).behaviour, Behaviour::Closed);
    }

    #[test]
    fn all_open_profiles_resolve_to_open() {
        let profiles =
            vec![profile_with_behaviour("p1", &flat(&["use"]), Behaviour::Open), profile_with_behaviour("p2", &flat(&["distribute"]), Behaviour::Open)];
        assert_eq!(resolve(&profiles).behaviour, Behaviour::Open);
    }

    #[test]
    fn behaviour_deserializes_open_and_closed() {
        assert_eq!(serde_json::from_str::<Behaviour>("\"open\"").unwrap(), Behaviour::Open);
        assert_eq!(serde_json::from_str::<Behaviour>("\"closed\"").unwrap(), Behaviour::Closed);
    }

    #[test]
    fn behaviour_deserializes_the_formal_semantics_drafts_own_default_alias_as_closed() {
        assert_eq!(
            serde_json::from_str::<Behaviour>("\"default\"").unwrap(),
            Behaviour::Closed,
            "the Formal Semantics draft states plainly that its own \"default\" value is closed"
        );
    }

    #[test]
    fn behaviour_never_serializes_the_default_alias_back_out() {
        assert_eq!(serde_json::to_string(&Behaviour::Closed).unwrap(), "\"closed\"");
        assert_eq!(serde_json::to_string(&Behaviour::Open).unwrap(), "\"open\"");
    }

    #[test]
    fn a_resolved_config_names_no_party_identity_claim_unless_a_host_asks_for_one() {
        // Decision 1: off by default, from every construction path there is.
        assert_eq!(ResolvedConfig::new(vec![], DutyMode::Advise, Behaviour::Open).party_identity_claim, None);
        assert_eq!(resolve(&[]).party_identity_claim, None);
        assert_eq!(
            resolve(&[profile("p1", &flat(&["use"]), DutyMode::Advise)]).party_identity_claim,
            None,
            "no ODRL Profile document declares which claim carries the caller's identity, so \
             resolving profiles can never switch party-role evaluation on by itself"
        );
    }

    #[test]
    fn with_party_identity_claim_names_the_claim_and_changes_nothing_else() {
        let base = ResolvedConfig::new(vec![ActionDecl::new("use")], DutyMode::Deny, Behaviour::Closed);
        let scoped = base.clone().with_party_identity_claim("sub");
        assert_eq!(scoped.party_identity_claim.as_deref(), Some("sub"));
        assert_eq!(scoped.duty_mode, base.duty_mode);
        assert_eq!(scoped.behaviour, base.behaviour);
        assert_eq!(scoped.declared_actions(), base.declared_actions());
    }
}
