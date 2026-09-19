//! A minimal, generic RDF graph built directly on `oxrdf`/`oxttl` (the same
//! stack `compliance-runner/src/graph.rs` already standardizes on for this
//! workspace, and `dataspace/site`'s own `build.rs`). Not `compliance-runner`'s
//! own `Graph` reused verbatim: that one indexes lookups by a subject's
//! *local name* only, which is the right shape for the SolidLabResearch
//! corpus it reads (subjects scattered across several files with unrelated
//! base IRIs) but the wrong one here -- every `dsc:` case file is one
//! self-contained document under its own `@base`, so every local resource
//! is a `#fragment` of that one file's own canonical URL and there is
//! nothing to disambiguate by matching on the full, absolute IRI instead.
//!
//! Two axes of lookup this module provides that `compliance-runner`'s own
//! `Graph` does not need for its corpus:
//!
//! - **`rdf_list`**, an actual `rdf:first`/`rdf:rest`/`rdf:nil` walk. The
//!   `dsc:` vocabulary's own file-shape conventions
//!   (`docs/vocabulary-spec.md`) use real Turtle list syntax (`( ... )`)
//!   for exactly two properties -- `odrl:and`/`odrl:or`/`odrl:xone`/
//!   `odrl:andSequence` and `dsc:inheritsFrom` -- and plain repeated
//!   triples (`odrl:permission :a, :b`) for every other multi-valued
//!   property in the corpus (confirmed by reading every `cases/*.ttl` file
//!   before writing this translator). `objects`/`object_ids` below read
//!   the latter; `rdf_list` reads the former, and only the former.
//! - **Multi-typed nodes**: `:alice a odrl:Party, foaf:Person` needs
//!   `type_ids` returning every `rdf:type`, not just the first.

use std::fs;
use std::path::Path;

use oxrdf::{NamedOrBlankNode, Term, Triple};
use oxttl::TurtleParser;

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
pub const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
pub const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";

pub fn odrl(local: &str) -> String {
    format!("http://www.w3.org/ns/odrl/2/{local}")
}

pub fn dsc(local: &str) -> String {
    format!("https://ds-labs-org.github.io/ds-odrl-compliance-rdf/ns#{local}")
}

pub fn report_ns(local: &str) -> String {
    format!("https://w3id.org/force/compliance-report#{local}")
}

/// The `local-name` half of an IRI (after the last `/` or `#`), or the
/// whole string when it has neither separator (e.g. a bare `did:web:...`
/// URI, carried as `WirePolicy::assigner` and never compared against
/// anything). Every string this translator hands to the engine --
/// actions, targets, assignee/assigner, claim values that are resources,
/// `odrl:rightOperand` resources, policy ids -- goes through this one
/// function, so two occurrences of the same IRI anywhere in one case file
/// always stringify identically, which is all correctness here actually
/// requires (see `translate.rs`'s own doc comment for why a single,
/// uniform scheme is sufficient and deliberately not a full IRI).
pub fn local_name(iri: &str) -> &str {
    iri.rsplit(['/', '#']).next().unwrap_or(iri)
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

pub struct Graph {
    triples: Vec<Triple>,
}

impl Graph {
    pub fn parse(path: &Path) -> Result<Self, String> {
        let content = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_turtle(&content).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The same parse over in-memory Turtle bytes -- what `parse` delegates
    /// to, and what this crate's own unit tests feed synthetic case
    /// fragments through without touching the filesystem.
    pub fn from_turtle(content: &[u8]) -> Result<Self, String> {
        let mut triples = Vec::new();
        for triple in TurtleParser::new().for_reader(content) {
            triples.push(triple.map_err(|e| e.to_string())?);
        }
        Ok(Self { triples })
    }

    /// Every object of `subject`/`predicate`, in file order -- the plain,
    /// repeated-triple multi-value shape (`odrl:permission :a, :b`), never
    /// an `rdf:List`.
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

    /// The first node-valued (non-literal) object of `subject`/`predicate`,
    /// as a graph-indexable id.
    pub fn object_id(&self, subject: &str, predicate: &str) -> Option<String> {
        self.object(subject, predicate).and_then(term_id)
    }

    /// Every node-valued object of `subject`/`predicate`.
    pub fn object_ids(&self, subject: &str, predicate: &str) -> Vec<String> {
        self.objects(subject, predicate)
            .into_iter()
            .filter_map(term_id)
            .collect()
    }

    /// The first literal's lexical value of `subject`/`predicate` --
    /// datatype and language tag both ignored, since every literal this
    /// translator reads (a `dsc:key`, an `rdf:value`, an
    /// `odrl:rightOperand`) is compared purely by lexical form per
    /// `docs/vocabulary-spec.md` section 4.4's comparison convention.
    pub fn literal(&self, subject: &str, predicate: &str) -> Option<String> {
        self.objects(subject, predicate)
            .into_iter()
            .find_map(|t| match t {
                Term::Literal(l) => Some(l.value().to_string()),
                _ => None,
            })
    }

    /// Every `rdf:type` of `subject` -- plural, because a party node in
    /// this corpus is routinely `a odrl:Party, foaf:Person` and this
    /// translator only ever needs to ask "is one of these a given type",
    /// never "the" type.
    pub fn type_ids(&self, subject: &str) -> Vec<String> {
        self.object_ids(subject, RDF_TYPE)
    }

    pub fn has_type(&self, subject: &str, type_iri: &str) -> bool {
        self.type_ids(subject).iter().any(|t| t == type_iri)
    }

    /// The subject of the (expected to be unique) triple `?s a <type_iri>`.
    pub fn subject_with_type(&self, type_iri: &str) -> Option<String> {
        self.triples
            .iter()
            .find(|t| {
                t.predicate.as_str() == RDF_TYPE && term_id(&t.object).as_deref() == Some(type_iri)
            })
            .map(|t| subject_id(&t.subject))
    }

    /// Walks a genuine `rdf:List` (Turtle's `( ... )` syntax) from its head
    /// node, in order, to `rdf:nil`. `head` is `None` when `subject`
    /// carries no `predicate` triple at all (an absent optional list, e.g.
    /// no `dsc:inheritsFrom`); `Some(&[])` never occurs for a real list --
    /// an explicit `()` would resolve to `rdf:nil` directly and this walk
    /// stops immediately, returning an empty `Vec`, which callers treat
    /// identically to "absent" since both mean "no children".
    pub fn rdf_list(&self, subject: &str, predicate: &str) -> Vec<Term> {
        let Some(head) = self.object(subject, predicate) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        let mut current = term_id(head);
        while let Some(node) = current {
            if node == RDF_NIL {
                break;
            }
            match self.object(&node, RDF_FIRST) {
                Some(first) => items.push(first.clone()),
                None => break, // malformed list -- stop rather than loop forever
            }
            current = self.object_id(&node, RDF_REST);
        }
        items
    }

    /// Node ids from an `rdf_list` call, dropping any literal member --
    /// every list this translator reads (`odrl:and`/`or`/`xone`/
    /// `andSequence` children, `dsc:inheritsFrom` parents) is a list of
    /// resources, never literals.
    pub fn rdf_list_ids(&self, subject: &str, predicate: &str) -> Vec<String> {
        self.rdf_list(subject, predicate)
            .iter()
            .filter_map(term_id)
            .collect()
    }
}
