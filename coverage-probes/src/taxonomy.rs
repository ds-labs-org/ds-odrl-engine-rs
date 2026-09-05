//! The shipped W3C ODRL 2.2 action taxonomy, as a `RequestConfig` three
//! probes evaluate against.
//!
//! `profile-interpreter/examples/odrl-2.2-common-actions.ttl` is a real
//! Turtle profile document transcribed from the vocabulary's own §3.12 and
//! §4.4 definition tables. It is parsed **here, natively, at
//! catalog-generation time**, and the resulting `ActionDecl`s are baked
//! into those probes' `config` — so what the browser then verifies is the
//! *engine's* chain resolution over 51 declared actions (including the two
//! non-`use` two-hop chains, `display -> play -> use` and
//! `extract -> reproduce -> use`, that no hand-written config anywhere in
//! this workspace declares), not the Turtle parse. The three rows that use
//! it carry exactly that caveat, stated on the page.
//!
//! Re-parsing the Turtle in-browser is possible in principle — the site
//! does link `profile-interpreter` and already parses Turtle in wasm for
//! its Demonstrator page — and was considered and rejected for this
//! phase: the `.ttl` lives outside `pages.yml`'s deploy-path filter, and
//! it would introduce a second, structurally different probe kind into an
//! otherwise perfectly uniform "one probe = one `evaluate()` call"
//! catalog. Noted as a clean follow-up, not done here.

use engine::profile::{Behaviour, DutyMode};
use engine::wire::{WireActionDecl, WireNodeRef};
use engine::RequestConfig;

use profile_interpreter::graph::Graph;
use profile_interpreter::interpret::interpret;

/// The vocabulary document, embedded at *this generator's* compile time
/// (never in the wasm build — the site fetches the finished catalog).
const TAXONOMY_TTL: &str = include_str!("../../profile-interpreter/examples/odrl-2.2-common-actions.ttl");

/// The number of `odrl:Action`-typed subjects that document declares: 2
/// core roots (`use`, `transfer`), 40 native common actions, 9 Creative
/// Commons terms ODRL 2.2 adopts by reference. Asserted rather than
/// trusted, so an edit to the `.ttl` that silently drops or duplicates a
/// term fails the build instead of quietly shrinking three probes.
pub const TAXONOMY_ACTION_COUNT: usize = 51;

/// `profile-interpreter`'s own `interpret()` over the shipped taxonomy,
/// converted to the wire's `odrl:action` list. Panics rather than degrades:
/// a catalog generated against a taxonomy that failed to parse would be a
/// silently weaker catalog.
pub fn taxonomy_actions() -> Vec<WireActionDecl> {
    let graph = Graph::from_turtle(TAXONOMY_TTL.as_bytes()).expect("the shipped ODRL 2.2 taxonomy parses as Turtle");
    let interpreted = interpret(&graph, None, DutyMode::Advise, Behaviour::Closed);

    let actions: Vec<WireActionDecl> = interpreted
        .profile
        .actions
        .iter()
        .map(|action| WireActionDecl {
            id: action.id.clone(),
            included_in: action.included_in.clone().map(|id| WireNodeRef { id }),
        })
        .collect();

    assert_eq!(
        actions.len(),
        TAXONOMY_ACTION_COUNT,
        "the shipped ODRL 2.2 taxonomy must declare exactly {TAXONOMY_ACTION_COUNT} actions"
    );
    actions
}

/// The `config` object the three `act-taxonomy-*` probes carry.
pub fn taxonomy_config() -> RequestConfig {
    RequestConfig {
        type_: "odrl:Profile".to_string(),
        id: "https://www.w3.org/TR/odrl-vocab/".to_string(),
        actions: taxonomy_actions(),
        duty_mode: DutyMode::Advise,
        behaviour: Behaviour::Closed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent_of(actions: &[WireActionDecl], id: &str) -> Option<String> {
        actions.iter().find(|a| a.id == id)?.included_in.as_ref().map(|r| r.id.clone())
    }

    #[test]
    fn the_shipped_taxonomy_parses_to_fifty_one_declared_actions() {
        assert_eq!(taxonomy_actions().len(), TAXONOMY_ACTION_COUNT);
    }

    #[test]
    fn both_two_hop_chains_the_probes_depend_on_are_present_as_declared_edges() {
        let actions = taxonomy_actions();

        // display -> play -> use: every hop must be its own declared
        // ActionDecl, or `ResolvedConfig::covers` stops at the gap.
        assert_eq!(parent_of(&actions, "display").as_deref(), Some("play"));
        assert_eq!(parent_of(&actions, "play").as_deref(), Some("use"));

        // extract -> reproduce -> use.
        assert_eq!(parent_of(&actions, "extract").as_deref(), Some("reproduce"));
        assert_eq!(parent_of(&actions, "reproduce").as_deref(), Some("use"));

        // sell -> transfer, the Information Model's own worked example.
        assert_eq!(parent_of(&actions, "sell").as_deref(), Some("transfer"));

        // The two roots declare no parent at all.
        assert_eq!(parent_of(&actions, "use"), None);
        assert_eq!(parent_of(&actions, "transfer"), None);
    }

    #[test]
    fn the_taxonomy_config_carries_every_action_and_a_closed_behaviour() {
        let config = taxonomy_config();
        assert_eq!(config.actions.len(), TAXONOMY_ACTION_COUNT);
        assert_eq!(config.behaviour, Behaviour::Closed);
    }
}
