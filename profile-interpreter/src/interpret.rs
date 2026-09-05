//! Reads one ODRL Profile document's RDF graph and produces `engine::Profile`
//! — this engine's own narrowed reading of the Profile Mechanism (Section
//! 4.4: `actions: Vec<ActionDecl>` and a `duty_mode`), not a general ODRL
//! profile processor.
//!
//! **What's read, and why, checked against the W3C ODRL Information
//! Model's own Profile Mechanism section
//! (<https://www.w3.org/TR/odrl-model/#profile-mechanism>) rather than
//! assumed:** a new Action is declared as an *instance*, not a subclass —
//! `ex:myAction a odrl:Action .`, optionally with `odrl:includedIn` naming
//! a parent action. This interpreter collects every `odrl:Action`-typed
//! subject into an `ActionDecl { id, included_in }`, capturing that
//! `odrl:includedIn` object as real data rather than discarding it —
//! `engine::ResolvedConfig::covers` (see its own doc comment) now walks
//! exactly this declared edge to resolve action-taxonomy coverage, closing
//! the gap Section 7 of the case study used to name as out of scope. What
//! remains genuinely unresolved, honestly: only *declared* edges are
//! followed (an action a document never types `a odrl:Action` contributes
//! nothing even as someone else's `includedIn` target — see
//! `does_not_reach_through_an_action_the_document_never_declares` below),
//! and this is still a per-request problem if a caller never loads a
//! profile that declares the edge in the first place — declaring it is
//! this tool's whole job, not a substitute for a caller actually calling
//! it.
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
//! **`behaviour` (the ODRL Community Group's own Formal Semantics axis,
//! Section 3.6) is, for exactly the same reason, also always
//! caller-supplied — not read from the document.** Unlike `duty_mode`,
//! this one *is* the standards body's own named concept, not this
//! engine's invention, but the Formal Semantics draft describes it as an
//! input to the evaluation process itself ("an optional parameter
//! specifying the Behaviour of the system"), not a property a Profile
//! document declares about itself. Defaults to `open`, `engine`'s own
//! historical, unconditional default.
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

use engine::profile::ActionDecl;
use engine::{Behaviour, DutyMode, Profile};

use crate::graph::{local_name, odrl, Graph};

/// Parses a caller-supplied duty-mode flag into `engine::DutyMode` --
/// exposed here, not just inlined in the CLI, so a consumer with no
/// Rust-level dependency on the `engine` crate itself (site/'s Demonstrator
/// page deliberately has none, see site/Cargo.toml's header comment) can
/// still produce a `DutyMode` value to pass to [`interpret`] without ever
/// needing to spell that type's name.
pub fn duty_mode_from_str(s: &str) -> Result<DutyMode, String> {
    match s {
        "advise" => Ok(DutyMode::Advise),
        "deny" => Ok(DutyMode::Deny),
        other => Err(format!("duty mode must be \"advise\" or \"deny\", got {other:?}")),
    }
}

/// Same reasoning as [`duty_mode_from_str`], for `behaviour` — see this
/// module's own doc comment for why it's caller-supplied, never read from
/// the document. Accepts the Formal Semantics draft's own `"default"` as
/// an alias for `"closed"`, exactly as `engine::Behaviour`'s own
/// `Deserialize` impl does.
pub fn behaviour_from_str(s: &str) -> Result<Behaviour, String> {
    match s {
        "open" => Ok(Behaviour::Open),
        "closed" | "default" => Ok(Behaviour::Closed),
        other => Err(format!("behaviour must be \"open\", \"closed\", or \"default\", got {other:?}")),
    }
}

/// `engine::Behaviour::default()` (`Open`), exposed so a caller with no
/// Rust-level dependency on `engine` itself can obtain one without
/// spelling either the type or a magic string — used where `interpret()`
/// needs *a* `Behaviour` value but nothing about the call site's actual
/// behavior selection, since `Profile.behaviour` is only ever stored, not
/// branched on, by this function's own logic (see [`interpret`]'s doc
/// comment on `duty_mode` for the same reasoning, which applies here too).
pub fn default_behaviour() -> Behaviour {
    Behaviour::default()
}

pub struct Interpreted {
    pub profile: Profile,
    pub warnings: Vec<String>,
    /// Local names of every `odrl:LeftOperand`-typed subject, as data --
    /// distinct from the human-readable warning text above, because a UI
    /// (site's Demonstrator page) needs this list to build an autocomplete/
    /// suggestion widget, and re-parsing a warning string for it would be
    /// fragile and wrong. Sorted and deduped, same convention as
    /// `Profile.actions`.
    pub declared_left_operands: Vec<String>,
}

/// `id_override` wins when given (a CLI `--id`, or a caller who already
/// knows the profile's canonical IRI); otherwise the first `odrl:Profile`-
/// typed subject in the graph; otherwise a placeholder, with a warning —
/// real profile *documents* are not required to self-declare as
/// `a odrl:Profile` (a Policy references a profile by IRI via
/// `odrl:profile`, the document itself is just vocabulary), so a missing
/// one is common, not necessarily a mistake.
pub fn interpret(graph: &Graph, id_override: Option<String>, duty_mode: DutyMode, behaviour: Behaviour) -> Interpreted {
    let mut warnings = Vec::new();

    let id = id_override
        .or_else(|| graph.subjects_with_type(&odrl("Profile")).into_iter().next())
        .unwrap_or_else(|| {
            warnings.push(
                "no odrl:Profile-typed subject in this document and no --id given; using a placeholder id — pass --id if this profile has a real, known IRI".to_string(),
            );
            "urn:uuid:unidentified-profile".to_string()
        });

    let declared_action_iris = graph.subjects_with_type(&odrl("Action"));
    let mut actions: Vec<ActionDecl> = declared_action_iris
        .iter()
        .map(|action| {
            let included_in = graph.object_node(action, &odrl("includedIn")).map(|parent| local_name(&parent).to_string());
            if let Some(parent) = &included_in {
                warnings.push(format!(
                    "{} declares odrl:includedIn {} — captured as a real ActionDecl edge, so a permission for {} now covers a request for {} via engine::ResolvedConfig::covers",
                    local_name(action),
                    parent,
                    parent,
                    local_name(action),
                ));
            }
            ActionDecl { id: local_name(action).to_string(), included_in }
        })
        .collect();
    actions.sort_by(|a, b| a.id.cmp(&b.id));
    actions.dedup_by(|a, b| a.id == b.id);

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

    Interpreted { profile: Profile { id, actions, duty_mode, behaviour }, warnings, declared_left_operands }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(ttl: &str) -> Graph {
        Graph::from_turtle(ttl.as_bytes()).unwrap()
    }

    fn action_ids(interpreted: &Interpreted) -> Vec<String> {
        interpreted.profile.actions.iter().map(|a| a.id.clone()).collect()
    }

    fn included_in_of<'a>(interpreted: &'a Interpreted, id: &str) -> Option<&'a str> {
        interpreted.profile.actions.iter().find(|a| a.id == id)?.included_in.as_deref()
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
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        assert_eq!(action_ids(&interpreted), vec!["archive", "redistribute"]);
    }

    #[test]
    fn captures_a_declared_includedin_edge_as_real_actiondecl_data() {
        // ex:redistribute odrl:includedIn odrl:distribute is now kept as
        // ActionDecl::included_in data, not just noted in a warning — this
        // is exactly what closes Section 7's old "action implication is
        // not evaluated" gap (see engine::ResolvedConfig::covers's doc
        // comment). "distribute" itself is not separately declared here,
        // so it does not appear as its own recognized action — capturing
        // an edge is not the same as recognizing its target.
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:redistribute a odrl:Action ;
    odrl:includedIn odrl:distribute ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        assert_eq!(action_ids(&interpreted), vec!["redistribute"]);
        assert_eq!(included_in_of(&interpreted, "redistribute"), Some("distribute"));
        assert!(interpreted.warnings.iter().any(|w| w.contains("redistribute") && w.contains("distribute") && w.contains("covers")));
    }

    #[test]
    fn interpret_then_resolve_lets_a_permission_for_the_parent_cover_a_request_for_the_child() {
        // The whole point of this tool, proven end to end (not just that
        // the JSON shape looks right): parse a document declaring
        // ex:sell includedIn odrl:transfer and odrl:transfer itself as an
        // action, interpret it, resolve it via engine::resolve, and
        // confirm engine's own coverage check actually resolves the edge.
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:sell a odrl:Action ;
    odrl:includedIn odrl:transfer .
odrl:transfer a odrl:Action ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        let resolved = engine::resolve(std::slice::from_ref(&interpreted.profile));
        assert!(resolved.covers("transfer", "sell"), "a permission for transfer must cover a request for sell");
        assert!(!resolved.covers("sell", "transfer"), "coverage does not run backwards");
    }

    #[test]
    fn does_not_reach_through_an_action_the_document_never_declares() {
        // ex:redistribute claims odrl:includedIn ex:distribute, but
        // ex:distribute is never itself typed odrl:Action in this
        // document — the chain must not silently keep walking past that
        // gap (engine::ResolvedConfig::covers's own documented limit).
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:use a odrl:Action .
ex:redistribute a odrl:Action ;
    odrl:includedIn ex:distribute ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        let resolved = engine::resolve(std::slice::from_ref(&interpreted.profile));
        assert!(!resolved.covers("use", "redistribute"));
    }

    #[test]
    fn uses_the_declared_odrl_profile_subject_as_id_when_present() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
<https://example.org/profiles/mine> a odrl:Profile ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        assert_eq!(interpreted.profile.id, "https://example.org/profiles/mine");
        assert!(interpreted.warnings.is_empty());
    }

    #[test]
    fn falls_back_to_a_placeholder_id_with_a_warning_when_neither_is_available() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        assert!(interpreted.warnings.iter().any(|w| w.contains("placeholder id")));
    }

    #[test]
    fn id_override_wins_over_a_declared_odrl_profile_subject() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
<https://example.org/profiles/mine> a odrl:Profile ."#,
        );
        let interpreted =
            interpret(&g, Some("https://example.org/profiles/override".to_string()), DutyMode::Advise, Behaviour::Open);
        assert_eq!(interpreted.profile.id, "https://example.org/profiles/override");
    }

    #[test]
    fn duty_mode_passes_through_untouched_by_anything_in_the_document() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        assert_eq!(interpret(&g, None, DutyMode::Deny, Behaviour::Open).profile.duty_mode, DutyMode::Deny);
        assert_eq!(interpret(&g, None, DutyMode::Advise, Behaviour::Open).profile.duty_mode, DutyMode::Advise);
    }

    #[test]
    fn behaviour_passes_through_untouched_by_anything_in_the_document() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        assert_eq!(interpret(&g, None, DutyMode::Advise, Behaviour::Closed).profile.behaviour, Behaviour::Closed);
        assert_eq!(interpret(&g, None, DutyMode::Advise, Behaviour::Open).profile.behaviour, Behaviour::Open);
    }

    #[test]
    fn behaviour_from_str_accepts_open_closed_and_the_default_alias() {
        assert_eq!(behaviour_from_str("open").unwrap(), Behaviour::Open);
        assert_eq!(behaviour_from_str("closed").unwrap(), Behaviour::Closed);
        assert_eq!(behaviour_from_str("default").unwrap(), Behaviour::Closed);
        assert!(behaviour_from_str("bogus").is_err());
    }

    #[test]
    fn warns_on_a_declared_operator_extension_it_cannot_honor() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:matches a odrl:Operator ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        assert!(interpreted.warnings.iter().any(|w| w.contains("odrl:Operator matches") && w.contains("cannot be honored")));
    }

    #[test]
    fn notes_a_declared_left_operand_extension_without_a_warning_of_incapability() {
        let g = graph(
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:riskScore a odrl:LeftOperand ."#,
        );
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
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
        let interpreted = interpret(&g, None, DutyMode::Advise, Behaviour::Open);
        assert_eq!(interpreted.declared_left_operands, vec!["riskScore", "tenure"]);
    }

    #[test]
    fn declared_left_operands_is_empty_when_none_are_declared() {
        let g = graph("@prefix odrl: <http://www.w3.org/ns/odrl/2/>.\n@prefix ex: <http://example.org/>.\nex:a a odrl:Action .");
        assert!(interpret(&g, None, DutyMode::Advise, Behaviour::Open).declared_left_operands.is_empty());
    }

    #[test]
    fn duty_mode_from_str_accepts_advise_and_deny_and_rejects_anything_else() {
        assert_eq!(duty_mode_from_str("advise"), Ok(DutyMode::Advise));
        assert_eq!(duty_mode_from_str("deny"), Ok(DutyMode::Deny));
        assert!(duty_mode_from_str("bogus").is_err());
    }
}
