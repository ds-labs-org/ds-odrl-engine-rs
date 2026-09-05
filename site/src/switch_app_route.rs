use crate::app_route::AppRoute;
use crate::pages::{CompliancePage, DemoPage, HomePage};
use yew::{Html, html};

pub fn switch_app_route(target: AppRoute) -> Html {
  match target {
    AppRoute::Home => html! { <HomePage /> },
    AppRoute::Demo => html! { <DemoPage /> },
    AppRoute::Compliance => html! { <CompliancePage /> },
  }
}
