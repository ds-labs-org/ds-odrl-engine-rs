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
//! Constraint operands, Rule classes, conflict strategies).

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
}

impl ResolvedConfig {
    pub fn new(actions: Vec<ActionDecl>, duty_mode: DutyMode, behaviour: Behaviour) -> Self {
        Self { actions, duty_mode, behaviour }
    }

    /// Is `action` declared at all, by any loaded profile — Section 4.4's
    /// original vocabulary-validity check, unchanged in meaning: a rule
    /// naming an action no loaded profile even knows about is a
    /// configuration gap (`Decision::Error`), not an ordinary non-match.
    pub fn recognizes(&self, action: &str) -> bool {
        self.actions.iter().any(|a| a.id == action)
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

    ResolvedConfig { actions, duty_mode, behaviour }
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
}
