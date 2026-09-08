use crate::app_route::AppRoute;
use crate::switch_app_route::switch_app_route;
use patternfly_yew::prelude::*;
use yew::prelude::*;
use yew_nested_router::prelude::Switch as RouterSwitch;

/// The page shell: masthead brand, sidebar nav, and a router `Switch`
/// plugging in whichever of the six routes (Home/Demo/Compliance/
/// Capability Audit/ODRL 2.2 Full Compliance/History) is current.
/// Structurally mirrors the ds42.org dataspace study's own `MainView`
/// (Masthead/Page/Nav via patternfly-yew), not its nav items -- this
/// product has its own six-page shape.
///
/// "ODRL 2.2 Full Compliance" sits directly under "Capability Audit"
/// deliberately: the two pages drive the same engine over the same
/// probes and differ only in what they judge the answers against, so a
/// reader who has just read one should find the other without hunting
/// for it. ("Capability Audit" was named "ODRL 2.2 Coverage" through
/// v0.20.3 -- see `docs/journey/00-walkthrough.md` in the dataspace
/// repository's mirror for why: once `/full-compliance` existed to judge
/// against the spec ideal, "Coverage" read as though this page did that
/// too, when it actually judges live behaviour against this study's own
/// documentation instead.)
#[component]
pub fn MainView() -> Html {
  let brand = html!(
    <>
      <img src="brand/logo.svg" alt="ds-odrl-engine-rs" style="height: 32px !important; margin-right: 10px;" />
      <Title level={Level::H3} size={Size::XLarge}>{ "ds-odrl-engine-rs" }</Title>
    </>
  );

  let sidebar = html_nested!(
    <PageSidebar>
      <Nav>
        <NavList>
          <NavRouterItem<AppRoute> to={AppRoute::Home}>{ "Home" }</NavRouterItem<AppRoute>>
          <NavRouterItem<AppRoute> to={AppRoute::Demo}>{ "Demonstrator" }</NavRouterItem<AppRoute>>
          <NavRouterItem<AppRoute> to={AppRoute::Compliance}>{ "Compliance Results" }</NavRouterItem<AppRoute>>
          <NavRouterItem<AppRoute> to={AppRoute::Coverage}>{ "Capability Audit" }</NavRouterItem<AppRoute>>
          <NavRouterItem<AppRoute> to={AppRoute::FullCompliance}>{ "ODRL 2.2 Full Compliance" }</NavRouterItem<AppRoute>>
          <NavRouterItem<AppRoute> to={AppRoute::History}>{ "Release History" }</NavRouterItem<AppRoute>>
        </NavList>
      </Nav>
    </PageSidebar>
  );

  html!(
    <Page {brand} {sidebar} full_height=true>
      <PageSection>
        <RouterSwitch<AppRoute> render={switch_app_route} />
      </PageSection>
    </Page>
  )
}
