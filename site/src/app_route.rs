use yew_nested_router::Target;

/// This site's routing shell: the home page, Section 5's demonstrator UI,
/// the live compliance-suite run, the live ODRL 2.2 vocabulary coverage
/// run, the live full-spec compliance judgment, the per-release history
/// dashboard, and -- since the former README.md was split into
/// `docs/*.md` -- a docs index plus one route per doc, on the identical
/// `IndexRoute`/`Item { slug }` shape the ds42.org dataspace site's own
/// `AppRoute` already established for its ADR/worksheet/spike/case-study
/// routes (see that repo's `site/src/app_route.rs`). See `src/content.rs`
/// for the embedded `DocEntry` registry `DocIndex`/`Doc` render from.
///
/// Note that no route name here may also be the name of a directory Trunk
/// creates under `dist/` -- see `index.html`'s long comment on the
/// `/compliance` route-vs-directory collision, which is why every fetched
/// data artifact lands in `compliance-data/` instead. `coverage` is
/// likewise safe only because its catalog goes into that same directory
/// rather than a `dist/coverage/` of its own, and `history` is safe for
/// exactly the same reason: `release-history.json` is copied into
/// `compliance-data/` too, never into a `dist/history/`.
/// `full-compliance` fetches that same `compliance-data/latest-coverage.json`
/// and creates no directory of its own either. `docs` is safe on the same
/// footing: no `copy-file`/`copy-dir` directive in `index.html` targets a
/// `docs/` directory under `dist/`, so Trunk never creates one for this
/// route to collide with.
#[derive(Debug, Clone, PartialEq, Target, Eq)]
pub enum AppRoute {
    #[target(rename = "")]
    Home,
    #[target(rename = "demo")]
    Demo,
    #[target(rename = "compliance")]
    Compliance,
    #[target(rename = "coverage")]
    Coverage,
    #[target(rename = "full-compliance")]
    FullCompliance,
    #[target(rename = "history")]
    History,
    #[target(rename = "docs")]
    DocIndex,
    #[target(rename = "docs")]
    Doc { slug: String },
}
