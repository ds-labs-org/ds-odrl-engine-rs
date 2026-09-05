use yew_nested_router::Target;

/// This stage's routing shell: three top-level routes for the pages a
/// later stage fills in with real content (Section 5's demonstrator UI and
/// the compliance-runner results view). Kept flat and small on purpose --
/// no doc-tree/slug routes like the ds42.org site's own `AppRoute`, since
/// this site has no embedded `docs/` corpus of its own to browse.
#[derive(Debug, Clone, PartialEq, Target, Eq)]
pub enum AppRoute {
  #[target(rename = "")]
  Home,
  #[target(rename = "demo")]
  Demo,
  #[target(rename = "compliance")]
  Compliance,
}
