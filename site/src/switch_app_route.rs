use crate::app_route::AppRoute;
use crate::compliance_page::CompliancePage;
use crate::content::find_doc;
use crate::coverage_page::CoveragePage;
use crate::demo_page::DemoPage;
use crate::full_compliance_page::FullCompliancePage;
use crate::history_page::HistoryPage;
use crate::pages::{DocIndexPage, DocPage, HomePage, NotFoundPage};
use yew::{html, Html};

pub fn switch_app_route(target: AppRoute) -> Html {
    match target {
        AppRoute::Home => html! { <HomePage /> },
        AppRoute::Demo => html! { <DemoPage /> },
        AppRoute::Compliance => html! { <CompliancePage /> },
        AppRoute::Coverage => html! { <CoveragePage /> },
        AppRoute::FullCompliance => html! { <FullCompliancePage /> },
        AppRoute::History => html! { <HistoryPage /> },
        AppRoute::DocIndex => html! { <DocIndexPage /> },
        AppRoute::Doc { slug } => match find_doc(&slug) {
            Some(doc) => html! { <DocPage {doc} /> },
            None => html! { <NotFoundPage /> },
        },
    }
}
