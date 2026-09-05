//! ODRL Profiles: the action/vocabulary extension mechanism this engine
//! actually implements (case study Section 4.4).
//!
//! A `Profile` is this engine's own deliberately narrowed reading of W3C
//! ODRL's Profile Mechanism (Section 3.5) — just "additional Actions"
//! (`recognized_actions`) plus a `duty_mode` setting that Section 4.5's
//! duty evaluation consumes. Nothing here claims the rest of that
//! mechanism (additional Policy subclasses, Party functional roles,
//! Logical Constraint operands, Rule classes, conflict strategies).

use std::collections::HashSet;

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

/// One loaded ODRL Profile (Section 4.4's JSON shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub recognized_actions: Vec<String>,
    pub duty_mode: DutyMode,
}

/// The merged configuration a broker resolves **once, at its own
/// startup** from every profile it loads, and includes in every request
/// (Section 4.4). Resolving per-request would work identically here —
/// this type carries no state of its own — but the case study is explicit
/// that the *intended* lifecycle is host-startup-time, not per-request,
/// even though the engine itself stays stateless either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub recognized_actions: HashSet<String>,
    pub duty_mode: DutyMode,
}

impl ResolvedConfig {
    pub fn recognizes(&self, action: &str) -> bool {
        self.recognized_actions.contains(action)
    }
}

/// Resolves multiple loaded profiles into one `ResolvedConfig`: the
/// **union** of every profile's `recognized_actions`, and the
/// **strictest** (`deny` beats `advise`) of every profile's `duty_mode`
/// (Section 4.4).
///
/// The union choice is a *named* fail-open limitation (Section 7): this
/// engine has no way to scope a request to only the profile(s) a specific
/// Policy actually declares via ODRL's `profile` property, so every
/// request is evaluated against the union of *all* loaded profiles. A
/// superset can only recognize more actions than a correctly-scoped
/// per-policy selection would, never fewer — safe in the sense that it
/// never manufactures a spurious `Error`, but coarser than real per-policy
/// profile scoping. This function implements that choice as specified,
/// not a narrower alternative.
///
/// Zero loaded profiles resolves to an empty `recognized_actions` set
/// (every action becomes unrecognized) and the least-strict `duty_mode`
/// (`Advise`) — there is nothing to be strict *about* when nothing was
/// loaded.
pub fn resolve(profiles: &[Profile]) -> ResolvedConfig {
    let mut recognized_actions = HashSet::new();
    let mut duty_mode = DutyMode::Advise;

    for profile in profiles {
        recognized_actions.extend(profile.recognized_actions.iter().cloned());
        if profile.duty_mode == DutyMode::Deny {
            duty_mode = DutyMode::Deny;
        }
    }

    ResolvedConfig {
        recognized_actions,
        duty_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, actions: &[&str], duty_mode: DutyMode) -> Profile {
        Profile {
            id: id.to_string(),
            recognized_actions: actions.iter().map(|a| a.to_string()).collect(),
            duty_mode,
        }
    }

    #[test]
    fn resolves_the_union_of_recognized_actions_across_profiles() {
        let profiles = vec![
            profile("p1", &["use", "distribute"], DutyMode::Advise),
            profile("p2", &["distribute", "modify"], DutyMode::Advise),
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
            profile("p1", &["use"], DutyMode::Advise),
            profile("p2", &["distribute"], DutyMode::Deny),
        ];
        assert_eq!(resolve(&advise_then_deny).duty_mode, DutyMode::Deny);

        let deny_then_advise = vec![
            profile("p1", &["use"], DutyMode::Deny),
            profile("p2", &["distribute"], DutyMode::Advise),
        ];
        assert_eq!(resolve(&deny_then_advise).duty_mode, DutyMode::Deny);
    }

    #[test]
    fn all_advise_profiles_resolve_to_advise() {
        let profiles = vec![
            profile("p1", &["use"], DutyMode::Advise),
            profile("p2", &["distribute"], DutyMode::Advise),
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
            "recognized_actions": ["use", "distribute", "modify", "notify"],
            "duty_mode": "advise"
        }"#;
        let profile: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.id, "https://example.org/profiles/default");
        assert_eq!(profile.recognized_actions.len(), 4);
        assert_eq!(profile.duty_mode, DutyMode::Advise);
    }
}
