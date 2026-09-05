//! A minimal RDF graph over `oxrdf` triples, parsed by `oxttl` (Turtle) or
//! `oxjsonld` (JSON-LD) — the same oxrdf-family stack `compliance-runner`
//! already standardizes this organization on (see that crate's own
//! `graph.rs` doc comment), not a new RDF library for one more crate.
//!
//! Deliberately a separate, smaller copy rather than a shared library with
//! `compliance-runner`'s own `Graph`: the two have different query shapes
//! (this one only ever needs "every subject typed as X", not the
//! by-local-name reverse lookups `compliance-runner` needs for SOTW
//! membership/duty facts), and extracting a shared crate for ~30 lines
//! used two places is more abstraction than either currently earns.

use std::io::Read;

use oxrdf::{NamedOrBlankNode, Term, Triple};

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

pub fn odrl(local: &str) -> String {
    format!("http://www.w3.org/ns/odrl/2/{local}")
}

fn subject_id(s: &NamedOrBlankNode) -> String {
    match s {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

fn term_id(t: &Term) -> Option<String> {
    match t {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        Term::BlankNode(b) => Some(format!("_:{}", b.as_str())),
        Term::Literal(_) => None,
    }
}

/// The local-name half of an IRI (after the last `/` or `#`) — used only
/// for display (a `Profile.recognized_actions` entry, a warning message),
/// mirroring `compliance-runner`'s own convention rather than exposing
/// full IRIs to a human reading this tool's output.
pub fn local_name(iri: &str) -> &str {
    iri.rsplit(['/', '#']).next().unwrap_or(iri)
}

pub struct Graph {
    triples: Vec<Triple>,
}

impl Graph {
    pub fn from_turtle(bytes: &[u8]) -> Result<Self, String> {
        let mut triples = Vec::new();
        for triple in oxttl::TurtleParser::new().for_reader(bytes) {
            triples.push(triple.map_err(|e| format!("Turtle parse error: {e}"))?);
        }
        Ok(Self { triples })
    }

    pub fn from_json_ld(bytes: &[u8]) -> Result<Self, String> {
        let mut triples = Vec::new();
        for quad in oxjsonld::JsonLdParser::new().for_reader(bytes) {
            let quad = quad.map_err(|e| format!("JSON-LD parse error: {e}"))?;
            triples.push(Triple::new(quad.subject, quad.predicate, quad.object));
        }
        Ok(Self { triples })
    }

    /// Every subject with `rdf:type` exactly `type_iri`, in file order.
    pub fn subjects_with_type(&self, type_iri: &str) -> Vec<String> {
        self.triples
            .iter()
            .filter(|t| t.predicate.as_str() == RDF_TYPE)
            .filter(|t| term_id(&t.object).as_deref() == Some(type_iri))
            .map(|t| subject_id(&t.subject))
            .collect()
    }

    /// The first object of `subject`/`predicate` that is itself a node
    /// (IRI or blank node) — `None` for a missing triple or a
    /// literal-valued one.
    pub fn object_node(&self, subject: &str, predicate: &str) -> Option<String> {
        self.triples
            .iter()
            .find(|t| subject_id(&t.subject) == subject && t.predicate.as_str() == predicate)
            .and_then(|t| term_id(&t.object))
    }
}

/// Reads `path`'s bytes, dispatching to Turtle or JSON-LD by extension —
/// `.ttl`/`.turtle` for the former, `.jsonld`/`.json` for the latter.
/// `None` for any other extension (including none at all): this tool
/// does not guess a format from content sniffing, only from the name the
/// caller gave the file.
pub fn parse_by_extension(path: &std::path::Path) -> Result<Graph, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match ext.as_str() {
        "ttl" | "turtle" => Graph::from_turtle(&bytes),
        "jsonld" | "json" => Graph::from_json_ld(&bytes),
        other => Err(format!(
            "{}: unrecognized extension {other:?} — expected .ttl/.turtle or .jsonld/.json (pass --format to override)",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_name_splits_on_last_slash_or_hash() {
        assert_eq!(local_name("http://example.org/myAction"), "myAction");
        assert_eq!(local_name("http://www.w3.org/ns/odrl/2/Action"), "Action");
    }

    #[test]
    fn parses_turtle_and_finds_typed_subjects() {
        let g = Graph::from_turtle(
            br#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:myAction a odrl:Action ;
    odrl:includedIn odrl:use ."#,
        )
        .unwrap();
        let actions = g.subjects_with_type(&odrl("Action"));
        assert_eq!(actions, vec!["http://example.org/myAction".to_string()]);
        assert_eq!(
            g.object_node("http://example.org/myAction", &odrl("includedIn")).as_deref(),
            Some(odrl("use").as_str())
        );
    }

    #[test]
    fn parses_json_ld_and_finds_typed_subjects() {
        let json_ld = br#"{
            "@context": { "odrl": "http://www.w3.org/ns/odrl/2/", "ex": "http://example.org/" },
            "@id": "ex:myAction",
            "@type": "odrl:Action"
        }"#;
        let g = Graph::from_json_ld(json_ld).unwrap();
        let actions = g.subjects_with_type(&odrl("Action"));
        assert_eq!(actions, vec!["http://example.org/myAction".to_string()]);
    }
}
