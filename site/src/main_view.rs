use crate::app_route::AppRoute;
use crate::switch_app_route::switch_app_route;
use patternfly_yew::prelude::*;
use yew::prelude::*;
use yew_nested_router::prelude::Switch as RouterSwitch;

/// The page shell: masthead brand, sidebar nav, and a router `Switch`
/// plugging in whichever of the four routes (Home/Demo/Compliance/
/// Coverage) is current. Structurally mirrors the ds42.org dataspace
/// study's own `MainView` (Masthead/Page/Nav via patternfly-yew), not its
/// nav items -- this product has its own four-page shape.
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
          <NavRouterItem<AppRoute> to={AppRoute::Coverage}>{ "ODRL 2.2 Coverage" }</NavRouterItem<AppRoute>>
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
