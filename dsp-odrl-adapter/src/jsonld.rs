//! A deliberately partial, **offline** JSON-LD 1.1 *expansion* over the
//! subset of the algorithm a Dataspace Protocol contract policy actually
//! uses. See this crate's README for the itemized scope boundary; the
//! short version is that this implements context processing (inline
//! objects, arrays, string references to a bundled registry, `@vocab`,
//! `@import`, and type-scoped `@context` with `@propagate`), IRI
//! expansion, and `@id`/`@vocab` value coercion — and nothing else.
//!
//! **Why hand-rolled rather than the `json-ld` crate.** Two reasons, both
//! specific to what this adapter produces. First, a full processor's
//! natural output is an RDF dataset (that is also what `oxjsonld`, already
//! a dependency of `profile-interpreter`, would give): an RDF graph has no
//! array order, and `WirePolicy`'s `permissions[0]`/`prohibitions[1]`
//! indices — which the engine's own `reason` trace prints — are exactly
//! array order. Reconstructing it would mean re-deriving from
//! `rdf:List`/`@set` shapes what the JSON document already states plainly.
//! Second, `json-ld` is built around a document loader for remote contexts;
//! this adapter must be deterministic and offline (see `REGISTRY`), so most
//! of that machinery would be inert weight. The result is that this crate
//! adds **no third-party dependency at all** beyond `serde`/`serde_json`
//! and `engine` itself, which is the discipline the `engine` crate holds
//! itself to and there was no reason to break here.

use std::collections::{BTreeMap, BTreeSet};

/// The ODRL 2.2 namespace every term this adapter recognizes expands into.
pub const ODRL_NS: &str = "http://www.w3.org/ns/odrl/2/";

/// The context documents this adapter can resolve a `"@context": "<url>"`
/// string reference against. **Nothing is ever fetched over the network**:
/// a context that is not in here is a hard error (`JsonLdError::UnknownContext`),
/// never a silently-unresolvable document, because a document whose terms
/// all fail to expand yields an empty policy — and a policy that lost its
/// prohibitions is fail-open.
///
/// Each entry is a byte-for-byte copy of the published document, pinned in
/// `contexts/` with its source URL and fetch date recorded in this crate's
/// README. Both the `http` and `https` spellings of the W3C ODRL context
/// are listed because real documents use both and they are the same
/// document.
const REGISTRY: &[(&str, &str)] = &[
    ("http://www.w3.org/ns/odrl.jsonld", include_str!("../contexts/w3c-odrl-2.2.jsonld")),
    ("https://www.w3.org/ns/odrl.jsonld", include_str!("../contexts/w3c-odrl-2.2.jsonld")),
    ("https://w3id.org/dspace/2024/1/context.json", include_str!("../contexts/dsp-2024-1-context.json")),
    ("https://w3id.org/dspace/2025/1/context.jsonld", include_str!("../contexts/dsp-2025-1-dspace.jsonld")),
    (
        "https://w3id.org/dspace/2025/1/odrl-profile.jsonld",
        include_str!("../contexts/dsp-2025-1-odrl-profile.jsonld"),
    ),
];

/// The URLs `REGISTRY` above can resolve, in declaration order. Public so a
/// host can report what it is able to ingest without opening this file.
pub fn bundled_context_urls() -> Vec<&'static str> {
    REGISTRY.iter().map(|(url, _)| *url).collect()
}

/// One expanded value: a nested node object, an IRI (a node reference, or
/// a string value the active context type-coerced to `@id`/`@vocab`), or a
/// plain literal taken verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expanded {
    Node(Node),
    Iri(String),
    Literal(String),
}

/// An expanded node object.
///
/// `props` is a `Vec<(iri, values)>` rather than a map so that repeated
/// spellings of one property (`odrl:target` and `target` in the same
/// document, say) merge into one entry instead of one silently winning.
/// The *order of `props` itself carries no meaning* — `serde_json` without
/// the `preserve_order` feature already sorts an object's keys, and this
/// workspace deliberately does not enable it. What does carry meaning, and
/// is preserved exactly, is the order of values **within** a property: a
/// policy's `odrl:permission` array order is what `WirePolicy`'s
/// `permissions[0]` index — and therefore the engine's own `reason` trace
/// — is keyed on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Node {
    pub id: Option<String>,
    pub types: Vec<String>,
    pub props: Vec<(String, Vec<Expanded>)>,
}

impl Node {
    /// Every value this node carries for one **absolute** property IRI, in
    /// document order. An absent property is an empty slice, not an error:
    /// most of an ODRL policy's properties are optional.
    pub fn get(&self, iri: &str) -> &[Expanded] {
        self.props.iter().find(|(k, _)| k == iri).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    fn push(&mut self, iri: String, mut values: Vec<Expanded>) {
        if values.is_empty() {
            return;
        }
        match self.props.iter_mut().find(|(k, _)| *k == iri) {
            Some((_, existing)) => existing.append(&mut values),
            None => self.props.push((iri, values)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonLdError {
    /// A `"@context": "<url>"` naming a document not in `REGISTRY`.
    UnknownContext(String),
    /// A context that is not a string, an array, or an object — or a
    /// registry document that does not itself parse.
    MalformedContext(String),
    /// The document handed in is not a JSON object.
    NotANodeObject,
}

impl std::fmt::Display for JsonLdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonLdError::UnknownContext(url) => write!(
                f,
                "@context {url:?} is not one of the context documents bundled with this adapter, \
                 and no context is ever fetched over the network: {:?}",
                bundled_context_urls()
            ),
            JsonLdError::MalformedContext(what) => write!(f, "malformed @context: {what}"),
            JsonLdError::NotANodeObject => write!(f, "the document is not a JSON object"),
        }
    }
}

/// The result of expanding one document: the root node, plus everything
/// this expander had to take verbatim rather than resolve.
pub struct Expansion {
    pub node: Node,
    pub warnings: Vec<String>,
}

/// One term definition in an active context.
#[derive(Debug, Clone, Default)]
struct TermDef {
    /// The term's IRI mapping — or the literal `"@id"`/`"@type"` for a
    /// keyword alias (`"uid": "@id"` in the W3C ODRL context).
    iri: Option<String>,
    /// `@type` coercion for this term's *values*: `Some("@id")`,
    /// `Some("@vocab")`, or a datatype IRI (which this adapter ignores —
    /// see `expand_value`).
    type_mapping: Option<String>,
    /// A type-scoped local `@context`, applied when a node object declares
    /// this term as one of its `@type`s.
    scoped: Option<serde_json::Value>,
    /// Whether that scoped context survives into nested node objects.
    /// JSON-LD 1.1's default for a type-scoped context is `false`; DSP
    /// 2025/1 sets it `true` on every contract message, which is the only
    /// reason its `offer`/`permission`/`constraint` terms resolve at depth.
    propagate: bool,
}

/// An active context: term definitions plus `@vocab`.
#[derive(Debug, Clone, Default)]
struct Ctx {
    terms: BTreeMap<String, TermDef>,
    vocab: Option<String>,
}

/// A raw, not-yet-resolved term definition, as read straight off a context
/// object before prefixes are known. Resolution is deferred because a
/// context may define `"action": {"@id": "odrl:action"}` *before* it
/// defines the `odrl` prefix — `serde_json` sorts object keys, so relying
/// on declaration order would break on exactly the bundled contexts this
/// adapter has to read.
#[derive(Debug, Clone)]
struct RawDef {
    iri_value: Option<String>,
    type_mapping: Option<String>,
    scoped: Option<serde_json::Value>,
    propagate: bool,
}

/// How deep prefix-of-a-prefix resolution may chain before giving up. A
/// context defining `"a": "b:x"`, `"b": "c:y"`, ... is legal but finite in
/// practice; this bounds a malicious or accidental cycle.
const MAX_PREFIX_CHAIN: usize = 8;

fn registry_lookup(url: &str) -> Option<&'static str> {
    REGISTRY.iter().find(|(u, _)| *u == url).map(|(_, body)| *body)
}

/// Reads the `@context` value out of a bundled registry document.
fn registry_context(url: &str) -> Result<serde_json::Value, JsonLdError> {
    let body = registry_lookup(url).ok_or_else(|| JsonLdError::UnknownContext(url.to_string()))?;
    let doc: serde_json::Value =
        serde_json::from_str(body).map_err(|e| JsonLdError::MalformedContext(format!("{url}: {e}")))?;
    doc.get("@context")
        .cloned()
        .ok_or_else(|| JsonLdError::MalformedContext(format!("{url}: no @context member")))
}

/// Applies one local context (string reference, array, or object) on top of
/// `ctx`, in place.
fn process_context(ctx: &mut Ctx, local: &serde_json::Value) -> Result<(), JsonLdError> {
    match local {
        serde_json::Value::String(url) => {
            let imported = registry_context(url)?;
            process_context(ctx, &imported)
        }
        serde_json::Value::Array(items) => {
            for item in items {
                process_context(ctx, item)?;
            }
            Ok(())
        }
        serde_json::Value::Object(map) => process_context_object(ctx, map),
        serde_json::Value::Null => {
            // `"@context": null` resets the active context (JSON-LD 1.1).
            *ctx = Ctx::default();
            Ok(())
        }
        other => Err(JsonLdError::MalformedContext(format!("expected a string, array or object, got {other}"))),
    }
}

fn process_context_object(
    ctx: &mut Ctx,
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), JsonLdError> {
    // `@import` first: JSON-LD 1.1 merges the imported context *under* the
    // importing one, so the local definitions below may override it.
    if let Some(serde_json::Value::String(url)) = map.get("@import") {
        let imported = registry_context(url)?;
        process_context(ctx, &imported)?;
    }
    if let Some(serde_json::Value::String(vocab)) = map.get("@vocab") {
        ctx.vocab = Some(vocab.clone());
    }

    let mut raw: BTreeMap<String, RawDef> = BTreeMap::new();
    for (key, value) in map {
        // `@version`/`@protected`/`@propagate`/`@import`/`@vocab` are
        // handled above or deliberately inert — see the README's scope
        // list. `@base` is *not* honoured: this adapter never resolves a
        // relative reference (there is no reliable document base for a
        // policy that arrived over a wire protocol), so pretending to
        // would be worse than leaving relative IRIs verbatim.
        if key.starts_with('@') {
            continue;
        }
        let def = match value {
            serde_json::Value::String(s) => RawDef {
                iri_value: Some(s.clone()),
                type_mapping: None,
                scoped: None,
                propagate: false,
            },
            serde_json::Value::Object(def) => {
                let scoped = def.get("@context").cloned();
                let propagate = scoped
                    .as_ref()
                    .and_then(|c| c.get("@propagate"))
                    .and_then(|p| p.as_bool())
                    .unwrap_or(false);
                RawDef {
                    iri_value: def
                        .get("@id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or_else(|| Some(key.clone())),
                    type_mapping: def.get("@type").and_then(|v| v.as_str()).map(str::to_string),
                    scoped,
                    propagate,
                }
            }
            // `"term": null` removes a term (JSON-LD 1.1).
            serde_json::Value::Null => {
                ctx.terms.remove(key);
                continue;
            }
            other => {
                return Err(JsonLdError::MalformedContext(format!("term {key:?} is neither a string nor an object: {other}")))
            }
        };
        raw.insert(key.clone(), def);
    }

    let terms: Vec<String> = raw.keys().cloned().collect();
    for term in terms {
        let resolved = resolve_raw_term(ctx, &raw, &term, 0);
        let def = &raw[&term];
        ctx.terms.insert(
            term,
            TermDef {
                iri: resolved,
                type_mapping: def.type_mapping.clone(),
                scoped: def.scoped.clone(),
                propagate: def.propagate,
            },
        );
    }
    Ok(())
}

/// Resolves one raw term definition's IRI mapping, consulting other raw
/// definitions in the same context object (a prefix defined later in the
/// same object) and then the inherited active context.
fn resolve_raw_term(ctx: &Ctx, raw: &BTreeMap<String, RawDef>, term: &str, depth: usize) -> Option<String> {
    if depth > MAX_PREFIX_CHAIN {
        return None;
    }
    let value = raw.get(term)?.iri_value.as_ref()?;
    // Keyword aliases (`"uid": "@id"`, `"type": "@type"`) are kept as the
    // keyword itself; `expand_key` recognizes them.
    if value.starts_with('@') {
        return Some(value.clone());
    }
    if let Some((prefix, suffix)) = split_compact(value) {
        if let Some(base) = resolve_raw_term(ctx, raw, prefix, depth + 1).or_else(|| term_iri(ctx, prefix)) {
            return Some(format!("{base}{suffix}"));
        }
        return Some(value.clone());
    }
    // A bare value with no colon: `@vocab`-relative if a vocab is set,
    // otherwise taken as written (the term maps to itself).
    match &ctx.vocab {
        Some(vocab) => Some(format!("{vocab}{value}")),
        None => Some(value.clone()),
    }
}

fn term_iri(ctx: &Ctx, term: &str) -> Option<String> {
    ctx.terms.get(term).and_then(|d| d.iri.clone()).filter(|iri| !iri.starts_with('@'))
}

/// Splits `value` into `(prefix, suffix)` **only if** it is a compact IRI
/// candidate per JSON-LD's own IRI-expansion rule: it contains a colon, the
/// suffix does not begin with `//` (which makes it an absolute IRI such as
/// `https://…`), and the prefix is not the blank-node marker `_`.
fn split_compact(value: &str) -> Option<(&str, &str)> {
    let (prefix, suffix) = value.split_once(':')?;
    if suffix.starts_with("//") || prefix == "_" || prefix.is_empty() {
        return None;
    }
    Some((prefix, suffix))
}

/// True for the one shape `split_compact` excludes *because* it is already
/// absolute rather than because it isn't a colon-bearing string at all: a
/// `scheme://…` IRI (`https://…`, `http://…`). Mirrors `split_compact`'s
/// own `suffix.starts_with("//")` exclusion so the two stay in lockstep.
fn is_scheme_absolute(value: &str) -> bool {
    matches!(value.split_once(':'), Some((_, suffix)) if suffix.starts_with("//"))
}

/// JSON-LD IRI expansion. Returns the expanded value and whether it could
/// actually be resolved to something absolute; an unresolved value is
/// returned verbatim rather than guessed at, and the caller decides whether
/// that deserves a warning.
fn expand_iri(ctx: &Ctx, value: &str, vocab: bool) -> (String, bool) {
    if value.starts_with("_:") || value.starts_with('@') {
        return (value.to_string(), true);
    }
    if vocab {
        if let Some(iri) = term_iri(ctx, value) {
            return (iri, true);
        }
    }
    if let Some((prefix, suffix)) = split_compact(value) {
        if let Some(base) = term_iri(ctx, prefix) {
            return (format!("{base}{suffix}"), true);
        }
        // A colon, no matching prefix: an absolute IRI (`urn:uuid:…`,
        // `did:web:…`) is already what it needs to be.
        return (value.to_string(), true);
    }
    // A `scheme://…` string is excluded from `split_compact`'s compact-IRI
    // branch above only because a `//`-prefixed suffix needs this
    // different handling, not because it isn't already absolute — the
    // JSON-LD 1.1 IRI Expansion algorithm's own carve-out for exactly this
    // shape. Returning it verbatim here, *before* `@vocab` gets a turn,
    // matters two ways: as a KEY (`vocab: true`), the alternative is
    // `expand_key` falling through to "unresolved" and its caller
    // (`Key::Dropped`) silently discarding the whole property with no
    // warning; as a VALUE under a document-level `@vocab`, the alternative
    // is silently concatenating the vocab prefix onto an already-absolute
    // string (e.g. `https://example.org/ns#http://www.w3.org/ns/odrl/2/use`).
    if is_scheme_absolute(value) {
        return (value.to_string(), true);
    }
    if vocab {
        if let Some(v) = &ctx.vocab {
            return (format!("{v}{value}"), true);
        }
    }
    (value.to_string(), false)
}

enum Key {
    Id,
    Type,
    /// A keyword this adapter does not model, or a term that expanded to
    /// nothing absolute — dropped, exactly as a JSON-LD processor drops a
    /// key that expands to a relative IRI.
    Dropped,
    Property(String),
}

fn expand_key(ctx: &Ctx, key: &str) -> Key {
    let resolved = match ctx.terms.get(key).and_then(|d| d.iri.clone()) {
        Some(alias) if alias.starts_with('@') => alias,
        _ => key.to_string(),
    };
    match resolved.as_str() {
        "@id" => Key::Id,
        "@type" => Key::Type,
        k if k.starts_with('@') => Key::Dropped,
        _ => {
            let (iri, ok) = expand_iri(ctx, key, true);
            if ok {
                Key::Property(iri)
            } else {
                Key::Dropped
            }
        }
    }
}

/// Applies every type-scoped `@context` a node object's `@type`s bring in.
/// Returns `(effective, child_base)`: the context this node's own
/// properties are read in, and the context nested node objects start from
/// (which drops any non-propagating scoped context, per JSON-LD 1.1).
fn apply_type_scoped(
    base: &Ctx,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(Ctx, Ctx), JsonLdError> {
    let mut type_terms: BTreeSet<String> = BTreeSet::new();
    for (key, value) in obj {
        if !matches!(expand_key(base, key), Key::Type) {
            continue;
        }
        match value {
            serde_json::Value::String(s) => {
                type_terms.insert(s.clone());
            }
            serde_json::Value::Array(items) => {
                for item in items.iter().filter_map(|i| i.as_str()) {
                    type_terms.insert(item.to_string());
                }
            }
            _ => {}
        }
    }

    let mut effective = base.clone();
    let mut child_base = base.clone();
    // Lexicographic order, per the spec's own "sorted in lexicographical
    // order" wording for type-scoped contexts — `BTreeSet` gives that.
    for term in &type_terms {
        let Some(scoped) = base.terms.get(term).and_then(|d| d.scoped.clone()) else {
            continue;
        };
        let propagate = base.terms.get(term).map(|d| d.propagate).unwrap_or(false);
        process_context(&mut effective, &scoped)?;
        if propagate {
            process_context(&mut child_base, &scoped)?;
        }
    }
    Ok((effective, child_base))
}

fn scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Expands one property value (or each element of an array of them).
///
/// **A string in a position the active context does not coerce stays a
/// literal, byte for byte.** That single rule is the whole difference
/// between this and a recursive prefix-strip: whether `"odrl:something"` is
/// an IRI or a five-character-prefixed literal is decided by the context's
/// term definition, never by what the string happens to start with.
fn expand_value(
    child_base: &Ctx,
    ctx: &Ctx,
    type_mapping: Option<&str>,
    value: &serde_json::Value,
    warnings: &mut Vec<String>,
) -> Result<Vec<Expanded>, JsonLdError> {
    match value {
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                out.extend(expand_value(child_base, ctx, type_mapping, item, warnings)?);
            }
            Ok(out)
        }
        serde_json::Value::Object(map) => {
            if let Some(v) = map.get("@value") {
                // A value object: `@type`/`@language` are recorded nowhere,
                // because `engine::Constraint::right_operand` is a single
                // opaque `String` compared against a host claim — see the
                // README's "What is dropped".
                return Ok(scalar_to_string(v).map(Expanded::Literal).into_iter().collect());
            }
            // `{"@id": "..."}` and nothing else is a bare node reference,
            // which every ODRL `@type: @id` position uses.
            if map.len() == 1 {
                if let Some(serde_json::Value::String(id)) = map.get("@id") {
                    let (iri, _) = expand_iri(ctx, id, false);
                    return Ok(vec![Expanded::Iri(iri)]);
                }
            }
            Ok(vec![Expanded::Node(expand_node(child_base, map, warnings)?)])
        }
        serde_json::Value::Null => Ok(Vec::new()),
        scalar => {
            let Some(s) = scalar_to_string(scalar) else {
                return Ok(Vec::new());
            };
            match type_mapping {
                Some("@id") | Some("@vocab") => {
                    let vocab = type_mapping == Some("@vocab");
                    let (iri, resolved) = expand_iri(ctx, &s, vocab);
                    if !resolved {
                        warnings.push(format!(
                            "{s:?} sits in a position the active context coerces to \
                             {} but no term, prefix or @vocab resolves it — taken verbatim",
                            type_mapping.unwrap_or("@id")
                        ));
                    }
                    Ok(vec![Expanded::Iri(iri)])
                }
                // Includes a datatype coercion such as `xsd:dateTime`: the
                // datatype is dropped, the lexical form is kept.
                _ => Ok(vec![Expanded::Literal(s)]),
            }
        }
    }
}

fn expand_node(
    base: &Ctx,
    obj: &serde_json::Map<String, serde_json::Value>,
    warnings: &mut Vec<String>,
) -> Result<Node, JsonLdError> {
    let mut base = base.clone();
    if let Some(local) = obj.get("@context") {
        process_context(&mut base, local)?;
    }
    let (ctx, child_base) = apply_type_scoped(&base, obj)?;

    let mut node = Node::default();
    for (key, value) in obj {
        match expand_key(&ctx, key) {
            Key::Id => {
                if let Some(id) = value.as_str() {
                    node.id = Some(expand_iri(&ctx, id, false).0);
                }
            }
            Key::Type => {
                let raw: Vec<&str> = match value {
                    serde_json::Value::String(s) => vec![s.as_str()],
                    serde_json::Value::Array(items) => items.iter().filter_map(|i| i.as_str()).collect(),
                    _ => Vec::new(),
                };
                node.types.extend(raw.into_iter().map(|t| expand_iri(&ctx, t, true).0));
            }
            Key::Dropped => {}
            Key::Property(iri) => {
                let type_mapping = ctx.terms.get(key).and_then(|d| d.type_mapping.clone());
                let values = expand_value(&child_base, &ctx, type_mapping.as_deref(), value, warnings)?;
                node.push(iri, values);
            }
        }
    }
    Ok(node)
}

/// Expands one JSON-LD document into a node tree.
pub fn expand(doc: &serde_json::Value) -> Result<Expansion, JsonLdError> {
    let obj = doc.as_object().ok_or(JsonLdError::NotANodeObject)?;
    let mut warnings = Vec::new();
    let node = expand_node(&Ctx::default(), obj, &mut warnings)?;
    Ok(Expansion { node, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expanded(doc: serde_json::Value) -> Node {
        expand(&doc).expect("fixture must expand").node
    }

    #[test]
    fn a_compact_iri_expands_through_its_prefix_definition_not_by_string_surgery() {
        let node = expanded(json!({
            "@context": { "ex": "https://example.org/ns#", "target": { "@id": "ex:target", "@type": "@id" } },
            "@type": "ex:Thing",
            "ex:name": "plain",
            "target": "ex:asset"
        }));
        assert_eq!(node.types, vec!["https://example.org/ns#Thing".to_string()]);
        assert_eq!(node.get("https://example.org/ns#name"), [Expanded::Literal("plain".to_string())]);
        assert_eq!(
            node.get("https://example.org/ns#target"),
            [Expanded::Iri("https://example.org/ns#asset".to_string())],
            "a term declared `@type: @id` expands its string value as an IRI"
        );
    }

    #[test]
    fn a_value_in_a_position_with_no_type_coercion_stays_a_literal_even_when_it_looks_like_a_compact_iri() {
        // The whole difference between this and a recursive prefix-strip:
        // whether a string is an IRI is decided by the active context, not
        // by what the string happens to start with.
        let node = expanded(json!({
            "@context": { "ex": "https://example.org/ns#" },
            "ex:note": "ex:looks-like-a-curie"
        }));
        assert_eq!(
            node.get("https://example.org/ns#note"),
            [Expanded::Literal("ex:looks-like-a-curie".to_string())]
        );
    }

    #[test]
    fn a_suffix_beginning_with_a_double_slash_is_never_read_as_a_compact_iri() {
        // JSON-LD's own IRI-expansion rule, and not a hypothetical: a
        // context that happens to define a term named `https` must not
        // turn `https://example.org/x` into `<mapping>//example.org/x`.
        let node = expanded(json!({
            "@context": { "https": "https://wrong.example/", "ex": "https://example.org/ns#",
                          "ref": { "@id": "ex:ref", "@type": "@id" } },
            "ref": "https://example.org/x"
        }));
        assert_eq!(node.get("https://example.org/ns#ref"), [Expanded::Iri("https://example.org/x".to_string())]);
    }

    #[test]
    fn a_type_scoped_context_marked_propagate_applies_to_nested_node_objects_too() {
        // The JSON-LD 1.1 feature the whole DSP 2025/1 bare-term shape is
        // built on: the terms `offer`/`permission`/`action` exist only
        // inside the context scoped to the `ContractRequestMessage` type.
        let node = expanded(json!({
            "@context": {
                "ex": "https://example.org/ns#",
                "Envelope": { "@id": "ex:Envelope",
                              "@context": { "@propagate": true, "inner": { "@id": "ex:inner" }, "name": "ex:name" } }
            },
            "@type": "Envelope",
            "inner": { "name": "deep" }
        }));
        let Some(Expanded::Node(inner)) = node.get("https://example.org/ns#inner").first() else {
            panic!("`inner` must have resolved through the type-scoped context");
        };
        assert_eq!(inner.get("https://example.org/ns#name"), [Expanded::Literal("deep".to_string())]);
    }

    #[test]
    fn a_type_scoped_context_not_marked_propagate_stops_at_the_nested_node() {
        // The spec default for a type-scoped context is `@propagate: false`
        // — honoured, rather than quietly treated as always-true because
        // that is what DSP 2025/1 happens to set.
        let node = expanded(json!({
            "@context": {
                "ex": "https://example.org/ns#",
                "Envelope": { "@id": "ex:Envelope",
                              "@context": { "inner": { "@id": "ex:inner" }, "name": "ex:name" } }
            },
            "@type": "Envelope",
            "inner": { "name": "deep" }
        }));
        let Some(Expanded::Node(inner)) = node.get("https://example.org/ns#inner").first() else {
            panic!("`inner` itself is in scope");
        };
        assert!(
            inner.props.is_empty(),
            "`name` is out of scope inside the nested node, so it expands to nothing at all"
        );
    }

    #[test]
    fn an_already_absolute_iri_key_expands_verbatim_instead_of_being_dropped() {
        // The ABNF-legal `scheme "://" …` shape is excluded from
        // `split_compact`'s compact-IRI branch only because it needs
        // different handling, not because it isn't already absolute (see
        // the JSON-LD 1.1 IRI Expansion algorithm's own carve-out for a
        // `//`-prefixed suffix). Before this fix, falling out of that
        // branch with no `@vocab` set landed on `(value, false)`, and
        // `expand_key`'s caller treats an unresolved key as `Key::Dropped`
        // with no warning at all -- silently losing whatever the key named.
        let ctx = Ctx::default();
        assert_eq!(
            expand_iri(&ctx, "http://www.w3.org/ns/odrl/2/prohibition", true),
            ("http://www.w3.org/ns/odrl/2/prohibition".to_string(), true)
        );
    }

    #[test]
    fn an_already_absolute_iri_value_is_not_corrupted_by_a_document_level_vocab() {
        // The other half of the same bug: under a document-level `@vocab`,
        // an absolute IRI value must still be returned as-is rather than
        // routed through vocab concatenation, which would otherwise
        // silently produce the nonsensical
        // `https://example.org/ns#http://www.w3.org/ns/odrl/2/use`.
        let ctx = Ctx { vocab: Some("https://example.org/ns#".to_string()), ..Ctx::default() };
        assert_eq!(
            expand_iri(&ctx, "http://www.w3.org/ns/odrl/2/use", true),
            ("http://www.w3.org/ns/odrl/2/use".to_string(), true)
        );
    }

    #[test]
    fn a_full_odrl_iri_written_in_place_of_the_compact_term_is_not_silently_dropped() {
        // End-to-end version of the key-expansion test above: the ODRL 2.2
        // Vocabulary's own bundled context maps `prohibition` 1:1 onto
        // `http://www.w3.org/ns/odrl/2/prohibition`, so a document that
        // writes the absolute IRI directly in place of the compact term is
        // legal, RDF-equivalent JSON-LD -- not a malformed document.
        let node = expanded(json!({
            "@context": "http://www.w3.org/ns/odrl.jsonld",
            "@type": "Offer",
            "@id": "urn:uuid:t",
            "assigner": "did:web:provider.example",
            "target": "urn:asset:A",
            "permission": [{ "action": "use" }],
            "http://www.w3.org/ns/odrl/2/prohibition": [{ "action": "distribute" }]
        }));
        assert_eq!(
            node.get(&format!("{ODRL_NS}prohibition")).len(),
            1,
            "the full-IRI-keyed prohibition must expand exactly like its compact-term sibling, not vanish"
        );
    }

    #[test]
    fn every_bundled_context_url_resolves_to_a_pinned_document_that_parses() {
        let urls = bundled_context_urls();
        assert!(urls.contains(&"https://w3id.org/dspace/2024/1/context.json"));
        assert!(urls.contains(&"https://w3id.org/dspace/2025/1/context.jsonld"));
        assert!(urls.contains(&"https://w3id.org/dspace/2025/1/odrl-profile.jsonld"));
        assert!(urls.contains(&"http://www.w3.org/ns/odrl.jsonld"));
        for url in urls {
            let node = expanded(json!({ "@context": url }));
            assert!(node.props.is_empty(), "{url}: an empty document under a real context expands to nothing");
        }
    }
}
