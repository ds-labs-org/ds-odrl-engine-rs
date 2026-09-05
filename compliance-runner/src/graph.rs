//! A minimal, generic RDF graph built directly on `oxrdf`/`oxttl` types
//! (Oxigraph's own crates — see `../README.md` for why this compliance
//! runner standardizes on that stack rather than `sophia`/`rio_turtle` or
//! a hand-rolled Turtle parser).
//!
//! This is deliberately not `oxigraph::store::Store`: every fixture file
//! in `compliance/vendor/odrl-test-suite/data/` is a handful of triples,
//! so plain linear iteration over a `Vec<Triple>` is simpler than standing
//! up an in-memory SPARQL-capable store for lookups no more elaborate than
//! "objects of this subject/predicate".

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use oxrdf::{NamedOrBlankNode, Term, Triple};
use oxttl::TurtleParser;

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

pub fn odrl(local: &str) -> String {
    format!("http://www.w3.org/ns/odrl/2/{local}")
}

pub fn report_ns(local: &str) -> String {
    format!("https://w3id.org/force/compliance-report#{local}")
}

/// The `local-name` half of an IRI (after the last `/` or `#`) — this
/// vendored suite's own convention for turning e.g.
/// `http://example.org/alice` into `alice`, or `odrl:read` into `read`,
/// which is all the exact-string matching this translation needs.
pub fn local_name(iri: &str) -> &str {
    iri.rsplit(['/', '#']).next().unwrap_or(iri)
}

fn subject_id(s: &NamedOrBlankNode) -> String {
    match s {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

/// `None` for a `Literal` object — a literal is a leaf value, never
/// something this graph indexes lookups by.
fn term_id(t: &Term) -> Option<String> {
    match t {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        Term::BlankNode(b) => Some(format!("_:{}", b.as_str())),
        Term::Literal(_) => None,
    }
}

pub struct Graph {
    triples: Vec<Triple>,
}

impl Graph {
    pub fn parse(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut triples = Vec::new();
        for triple in TurtleParser::new().for_reader(BufReader::new(file)) {
            triples.push(triple.map_err(|e| format!("{}: {e}", path.display()))?);
        }
        Ok(Self { triples })
    }

    pub fn triples(&self) -> &[Triple] {
        &self.triples
    }

    /// Every object of `subject`/`predicate`, in file order.
    pub fn objects<'a>(&'a self, subject: &str, predicate: &str) -> Vec<&'a Term> {
        self.triples
            .iter()
            .filter(|t| subject_id(&t.subject) == subject && t.predicate.as_str() == predicate)
            .map(|t| &t.object)
            .collect()
    }

    pub fn object(&self, subject: &str, predicate: &str) -> Option<&Term> {
        self.objects(subject, predicate).into_iter().next()
    }

    /// The first object of `subject`/`predicate` that is itself a node
    /// (IRI or blank node), as a graph-indexable id — `None` for a
    /// missing triple or a literal-valued one.
    pub fn object_node(&self, subject: &str, predicate: &str) -> Option<String> {
        self.object(subject, predicate).and_then(term_id)
    }

    /// Every node-valued (non-literal) object of `subject`/`predicate`.
    pub fn object_nodes(&self, subject: &str, predicate: &str) -> Vec<String> {
        self.objects(subject, predicate)
            .into_iter()
            .filter_map(term_id)
            .collect()
    }

    pub fn type_of(&self, subject: &str) -> Option<String> {
        self.object_node(subject, RDF_TYPE)
    }

    /// The first subject found with the given `rdf:type`, trying each
    /// candidate type in order — used where a fixture's own top-level
    /// Policy node could in principle be an `odrl:Offer`/`odrl:Agreement`
    /// as well as the `odrl:Set` every vendored fixture actually uses
    /// (confirmed by grep across `data/policies/*.ttl` before writing
    /// this).
    pub fn subject_with_any_type(&self, type_iris: &[String]) -> Option<String> {
        self.triples
            .iter()
            .find(|t| t.predicate.as_str() == RDF_TYPE && term_id(&t.object).is_some_and(|id| type_iris.contains(&id)))
            .map(|t| subject_id(&t.subject))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_name_splits_on_last_slash_or_hash() {
        assert_eq!(local_name("http://example.org/alice"), "alice");
        assert_eq!(local_name("http://www.w3.org/ns/odrl/2/read"), "read");
        assert_eq!(local_name("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"), "type");
        assert_eq!(local_name("no-separator"), "no-separator");
    }

    #[test]
    fn parses_a_minimal_turtle_fixture_into_queryable_triples() {
        let dir = std::env::temp_dir().join(format!("compliance-runner-graph-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.ttl");
        std::fs::write(
            &path,
            r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/>.
@prefix ex: <http://example.org/>.
ex:alice a odrl:Permission;
    odrl:action odrl:read."#,
        )
        .unwrap();

        let g = Graph::parse(&path).unwrap();
        assert_eq!(g.type_of("http://example.org/alice").as_deref(), Some(odrl("Permission").as_str()));
        assert_eq!(
            g.object_node("http://example.org/alice", &odrl("action")).as_deref(),
            Some(odrl("read").as_str())
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
