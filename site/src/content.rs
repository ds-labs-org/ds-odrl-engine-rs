use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag};

/// One rendered documentation page, embedded at compile time from this
/// repo's own `docs/` tree -- mirrors the ds42.org dataspace study's own
/// `site/src/content.rs::DocEntry` (see that repo's README.md, "content
/// here is hand-maintained, not auto-discovered"). Every entry here is a
/// former README.md section, relocated rather than summarized: see the
/// top-level README's own "Documentation" table for the map from doc to
/// original section.
#[derive(PartialEq)]
pub struct DocEntry {
    pub title: &'static str,
    pub slug: &'static str,
    /// Directory the source markdown lives in, repo-root-relative, with a
    /// trailing slash -- used to resolve the doc's own relative links
    /// (e.g. `../compliance/reports/latest.md`) the same way the
    /// dataspace site's `resolve_link` does.
    pub base_dir: &'static str,
    /// Repo-root-relative path to the raw markdown -- used for "view raw
    /// source".
    pub raw_href: &'static str,
    pub content: &'static str,
}

pub static DOCS: &[DocEntry] = &[
    DocEntry {
        title: "What this is not, and the design rationale",
        slug: "design-rationale",
        base_dir: "docs/",
        raw_href: "docs/design-rationale.md",
        content: include_str!("../../docs/design-rationale.md"),
    },
    DocEntry {
        title: "Wire contract, logical constraints, action refinement, per-rule targets",
        slug: "wire-contract",
        base_dir: "docs/",
        raw_href: "docs/wire-contract.md",
        content: include_str!("../../docs/wire-contract.md"),
    },
    DocEntry {
        title: "Per-permission duties, consequences and remedies",
        slug: "duties-consequences-remedies",
        base_dir: "docs/",
        raw_href: "docs/duties-consequences-remedies.md",
        content: include_str!("../../docs/duties-consequences-remedies.md"),
    },
    DocEntry {
        title: "Party-role evaluation (odrl:assignee, opt-in)",
        slug: "party-role-evaluation",
        base_dir: "docs/",
        raw_href: "docs/party-role-evaluation.md",
        content: include_str!("../../docs/party-role-evaluation.md"),
    },
    DocEntry {
        title: "Conflict strategy and policy inheritance",
        slug: "conflict-and-inheritance",
        base_dir: "docs/",
        raw_href: "docs/conflict-and-inheritance.md",
        content: include_str!("../../docs/conflict-and-inheritance.md"),
    },
    DocEntry {
        title: "Detailed evaluation (evaluate_request_detailed)",
        slug: "detailed-evaluation",
        base_dir: "docs/",
        raw_href: "docs/detailed-evaluation.md",
        content: include_str!("../../docs/detailed-evaluation.md"),
    },
    DocEntry {
        title: "Introspection: claims and actions",
        slug: "introspection",
        base_dir: "docs/",
        raw_href: "docs/introspection.md",
        content: include_str!("../../docs/introspection.md"),
    },
    DocEntry {
        title: "Profile interpretation and DSP ingestion",
        slug: "profiles-and-ingestion",
        base_dir: "docs/",
        raw_href: "docs/profiles-and-ingestion.md",
        content: include_str!("../../docs/profiles-and-ingestion.md"),
    },
    DocEntry {
        title: "Compliance methodology and suite attribution",
        slug: "compliance-methodology",
        base_dir: "docs/",
        raw_href: "docs/compliance-methodology.md",
        content: include_str!("../../docs/compliance-methodology.md"),
    },
    DocEntry {
        title: "Release history dashboard methodology",
        slug: "release-history-methodology",
        base_dir: "docs/",
        raw_href: "docs/release-history-methodology.md",
        content: include_str!("../../docs/release-history-methodology.md"),
    },
    DocEntry {
        title: "References",
        slug: "references",
        base_dir: "docs/references/",
        raw_href: "docs/references/README.md",
        content: include_str!("../../docs/references/README.md"),
    },
];

pub fn find_doc(slug: &str) -> Option<&'static DocEntry> {
    DOCS.iter().find(|doc| doc.slug == slug)
}

/// Joins `target` against `base_dir`, resolving `.`/`..` segments --
/// identical logic to the dataspace site's own `resolve_path`, since a
/// doc's relative link (e.g. `../compliance/reports/latest.md`, written
/// against this doc's location on disk) needs the same repo-root-relative
/// resolution there as here.
fn resolve_path(base_dir: &str, target: &str) -> String {
    let mut segments: Vec<&str> = base_dir
        .trim_end_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            seg => segments.push(seg),
        }
    }
    segments.join("/")
}

/// Rewrites a doc-relative link/image target. An absolute URL, an
/// in-page anchor, or a `mailto:` link passes through unchanged; anything
/// else resolves against `base_dir` into a repo-root-relative path (e.g.
/// `docs/wire-contract.md` -> another doc route, `compliance/reports/
/// latest.md` -> a path this site does not serve, left as a relative
/// reference a reader resolves against the repository). No `ROUTES` table
/// here, unlike the dataspace site's own `resolve_link`: every route this
/// site serves under `/docs/<slug>` already matches a doc's own repo path
/// one-for-one (`docs/<slug>.md`), so a same-directory doc-to-doc link
/// resolves to a real page without a lookup table.
fn resolve_link(base_dir: &str, target: &str) -> String {
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with('#')
        || target.starts_with("mailto:")
    {
        return target.to_string();
    }
    let joined = resolve_path(base_dir, target);
    if let Some(slug) = joined
        .strip_prefix("docs/")
        .and_then(|rest| rest.strip_suffix(".md"))
    {
        return format!("docs/{slug}");
    }
    joined
}

/// Renders a doc's markdown to HTML, rewriting relative links/images so
/// they resolve against the repo root (see `resolve_link`) -- same
/// `pulldown-cmark` call shape as the dataspace site's own
/// `content::render`.
pub fn render(doc: &DocEntry) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(doc.content, options).map(|event| match event {
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: CowStr::from(resolve_link(doc.base_dir, &dest_url)),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: CowStr::from(resolve_link(doc.base_dir, &dest_url)),
            title,
            id,
        }),
        other => other,
    });

    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}
