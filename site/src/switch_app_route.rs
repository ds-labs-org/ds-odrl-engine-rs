use crate::app_route::AppRoute;
use crate::compliance_page::CompliancePage;
use crate::coverage_page::CoveragePage;
use crate::demo_page::DemoPage;
use crate::pages::HomePage;
use yew::{Html, html};

pub fn switch_app_route(target: AppRoute) -> Html {
  match target {
    AppRoute::Home => html! { <HomePage /> },
    AppRoute::Demo => html! { <DemoPage /> },
    AppRoute::Compliance => html! { <CompliancePage /> },
    AppRoute::Coverage => html! { <CoveragePage /> },
  }
}
