use yew_nested_router::Target;

/// This site's routing shell: five flat top-level routes -- the home
/// page, Section 5's demonstrator UI, the live compliance-suite run, the
/// live ODRL 2.2 vocabulary coverage run, and the per-release history
/// dashboard. Kept flat and small on purpose -- no doc-tree/slug routes
/// like the ds42.org site's own `AppRoute`, since this site has no
/// embedded `docs/` corpus of its own to browse.
///
/// Note that no route name here may also be the name of a directory Trunk
/// creates under `dist/` -- see `index.html`'s long comment on the
/// `/compliance` route-vs-directory collision, which is why every fetched
/// data artifact lands in `compliance-data/` instead. `coverage` is
/// likewise safe only because its catalog goes into that same directory
/// rather than a `dist/coverage/` of its own, and `history` is safe for
/// exactly the same reason: `release-history.json` is copied into
/// `compliance-data/` too, never into a `dist/history/`.
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
  #[target(rename = "history")]
  History,
}
