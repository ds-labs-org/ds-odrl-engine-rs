//! Reads one ODRL Profile document's RDF graph and produces `engine::Profile`
//! — this engine's own narrowed reading of the Profile Mechanism (Section
//! 4.4: just `recognized_actions` and a `duty_mode`), not a general ODRL
//! profile processor.
//!
//! **What's read, and why, checked against the W3C ODRL Information
//! Model's own Profile Mechanism section
//! (<https://www.w3.org/TR/odrl-model/#profile-mechanism>) rather than
//! assumed:** a new Action is declared as an *instance*, not a subclass —
//! `ex:myAction a odrl:Action .`, optionally with `odrl:includedIn` naming
//! a parent action. This interpreter collects every `odrl:Action`-typed
//! subject's local name into `recognized_actions`. It does **not** follow
//! `includedIn` transitively (if `ex:myAction odrl:includedIn odrl:use`,
//! recognizing `ex:myAction` does not also imply recognizing actions
//! `odrl:use` itself includes) — that is exactly the general
//! action-taxonomy-implication problem Section 7 of the case study names
//! as out of scope, and this tool does not quietly resolve it as a side
//! effect of an unrelated feature.
//!
//! **`duty_mode` is deliberately NOT read from the profile document at
//! all.** The ODRL specification defines no property for a profile to
//! declare its own enforcement/duty behavior (confirmed against the same
//! spec section — profiles are additive vocabulary, not policy-behavior
//! declarations), and `duty_mode` is this engine's own invention (Section
//! 4.5), not an ODRL concept a real profile document would ever carry.
//! Inventing a private RDF property for it here would manufacture a
//! pseudo-standard nothing else could ever produce or consume. `duty_mode`
//! is therefore always a caller-supplied parameter (a CLI flag, defaulting
//! to `advise`), same as a host's own deployment choice would be.
//!
//! **`odrl:LeftOperand`/`odrl:Operator` extensions are recognized but not
//! actionable**, and said so out loud rather than silently ignored: this
//! engine's `leftOperand` is already a free-form claims-map key (Section
//! 4.2), so a profile-declared `LeftOperand` needs no registration; but
//! `engine::Operator` is a fixed, non-extensible enum (`eq`/`neq`/
//! `isAnyOf`/`lt`/`lteq`/`gt`/`gteq`), so a profile-declared `Operator`
//! genuinely cannot be honored without an engine change, and this
//! interpreter surfaces that as a warning rather than pretending the
//! extension took effect.

use engine::{DutyMode, Profile};

use crate::graph::{local_name, odrl, Graph};

pub struct Interpreted {
    pub profile: Profile,
    pub warnings: Vec<String>,
    /// Local names of every `odrl:LeftOperand`-typed subject, as data --
    /// distinct from the human-readable warning text above, because a UI
    /// (site's Demonstrator page) needs this list to build an autocomplete/
    /// suggestion widget, and re-parsing a warning string for it would be
    /// fragile and wrong. Sorted and deduped, same convention as
    /// `recognized_actions`.
    pub declared_left_operands: Vec<String>,
}

/// `id_override` wins when given (a CLI `--id`, or a caller who already
/// knows the profile's canonical IRI); otherwise the first `odrl:Profile`-
/// typed subject in the graph; otherwise a placeholder, with a warning —
/// real profile *documents* are not required to self-declare as
/// `a odrl:Profile` (a Policy references a profile by IRI via
/// `odrl:profile`, the document itself is just vocabulary), so a missing
/// one is common, not necessarily a mistake.
pub fn interpret(graph: &Graph, id_override: Option<String>, duty_mode: DutyMode) -> Interpreted {
    let mut warnings = Vec::new();

    let id = id_override
        .or_else(|| graph.subjects_with_type(&odrl("Profile")).into_iter().next())
        .unwrap_or_else(|| {
            warnings.push(
                "no odrl:Profile-typed subject in this document and no --id given; using a placeholder id — pass --id if this profile has a real, known IRI".to_string(),
            );
            "urn:uuid:unidentified-profile".to_string()
        });

    let declared_actions = graph.subjects_with_type(&odrl("Action"));
    for action in &declared_actions {
        if let Some(parent) = graph.object_node(action, &odrl("includedIn")) {
            warnings.push(format!(
                "{} declares odrl:includedIn {} — recognized as its own action only; this tool does not transitively recognize {} (or anything else {} includes) as a consequence, per Section 7's action-implication limitation",
                local_name(action),
                local_name(&parent),
                local_name(&parent),
                local_name(action),
            ));
        }
    }
    let mut recognized_actions: Vec<String> = declared_actions.iter().map(|iri| local_name(iri).to_string()).collect();
    recognized_actions.sort();
    recognized_actions.dedup();

    let declared_left_operand_iris = graph.subjects_with_type(&odrl("LeftOperand"));
    for left_operand in &declared_left_operand_iris {
        warnings.push(format!(
            "profile declares odrl:LeftOperand {} — no action needed, this engine's leftOperand is already a free-form claims-map key (Section 4.2)",
            local_name(left_operand)
        ));
    }
    let mut declared_left_operands: Vec<String> =
        declared_left_operand_iris.iter().map(|iri| local_name(iri).to_string()).collect();
    declared_left_operands.sort();
    declared_left_operands.dedup();

    for operator in graph.subjects_with_type(&odrl("Operator")) {
        warnings.push(format!(
            "profile declares odrl:Operator {} — this engine's Operator enum is fixed (eq/neq/isAnyOf/lt/lteq/gt/gteq); a profile-declared operator cannot be honored without an engine change",
            local_name(&operator)
        ));
    }

    Interpreted { profile: Profile { id, recognized_actions, duty_mode }, warnings, declared_left_operands }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(ttl: &str) -> Graph {
        Graph::from_turtle(ttl.as_bytes()).unwrap()
    }

    #[test]
    fn collects_declared_actions_by_local_name() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:redistribute a odrl:Action ;
    odrl:includedIn odrl:distribute .
ex:archive a odrl:Action ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert_eq!(interpreted.profile.recognized_actions, vec!["archive", "redistribute"]);
    }

    #[test]
    fn does_not_transitively_recognize_includedin_parents() {
        // ex:redistribute includedIn odrl:distribute does NOT mean this
        // interpreter also recognizes "distribute" — that would be the
        // general action-implication inference Section 7 names as out of
        // scope, not this tool's job to quietly resolve.
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:redistribute a odrl:Action ;
    odrl:includedIn odrl:distribute ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert_eq!(interpreted.profile.recognized_actions, vec!["redistribute"]);
        assert!(!interpreted.profile.recognized_actions.contains(&"distribute".to_string()));
        assert!(interpreted.warnings.iter().any(|w| w.contains("redistribute") && w.contains("does not transitively recognize")));
    }

    #[test]
    fn uses_the_declared_odrl_profile_subject_as_id_when_present() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
<https://example.org/profiles/mine> a odrl:Profile ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert_eq!(interpreted.profile.id, "https://example.org/profiles/mine");
        assert!(interpreted.warnings.is_empty());
    }

    #[test]
    fn falls_back_to_a_placeholder_id_with_a_warning_when_neither_is_available() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert!(interpreted.warnings.iter().any(|w| w.contains("placeholder id")));
    }

    #[test]
    fn id_override_wins_over_a_declared_odrl_profile_subject() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
<https://example.org/profiles/mine> a odrl:Profile ."#,
        );
        let interpreted = interpret(&g, Some("https://example.org/profiles/override".to_string()), DutyMode::Advise);
        assert_eq!(interpreted.profile.id, "https://example.org/profiles/override");
    }

    #[test]
    fn duty_mode_passes_through_untouched_by_anything_in_the_document() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        assert_eq!(interpret(&g, None, DutyMode::Deny).profile.duty_mode, DutyMode::Deny);
        assert_eq!(interpret(&g, None, DutyMode::Advise).profile.duty_mode, DutyMode::Advise);
    }

    #[test]
    fn warns_on_a_declared_operator_extension_it_cannot_honor() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:matches a odrl:Operator ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert!(interpreted.warnings.iter().any(|w| w.contains("odrl:Operator matches") && w.contains("cannot be honored")));
    }

    #[test]
    fn notes_a_declared_left_operand_extension_without_a_warning_of_incapability() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:riskScore a odrl:LeftOperand ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert!(interpreted.warnings.iter().any(|w| w.contains("odrl:LeftOperand riskScore") && w.contains("no action needed")));
        assert_eq!(interpreted.declared_left_operands, vec!["riskScore"]);
    }

    #[test]
    fn declared_left_operands_are_sorted_and_deduped_independent_of_the_warning_text() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:riskScore a odrl:LeftOperand .
ex:tenure a odrl:LeftOperand .
ex:riskScore a odrl:LeftOperand ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise);
        assert_eq!(interpreted.declared_left_operands, vec!["riskScore", "tenure"]);
    }

    #[test]
    fn declared_left_operands_is_empty_when_none_are_declared() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        assert!(interpret(&g, None, DutyMode::Advise).declared_left_operands.is_empty());
    }
}
