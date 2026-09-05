#[cfg(target_arch = "wasm32")]
mod app_route;
#[cfg(target_arch = "wasm32")]
mod compliance_page;
#[cfg(target_arch = "wasm32")]
mod demo_form;
#[cfg(target_arch = "wasm32")]
mod demo_page;
#[cfg(target_arch = "wasm32")]
mod demo_widgets;
#[cfg(target_arch = "wasm32")]
mod engine_bridge;
#[cfg(target_arch = "wasm32")]
mod engine_module;
#[cfg(target_arch = "wasm32")]
mod main_view;
#[cfg(target_arch = "wasm32")]
mod pages;
#[cfg(target_arch = "wasm32")]
mod profile_load;
#[cfg(target_arch = "wasm32")]
mod profile_panel;
#[cfg(target_arch = "wasm32")]
mod switch_app_route;
#[cfg(target_arch = "wasm32")]
mod wire;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
  println!("ds-odrl-engine-rs-site only targets web (wasm32)");
}

#[cfg(target_arch = "wasm32")]
fn main() {
  use app_route::AppRoute;
  use main_view::MainView;
  use yew::prelude::*;
  use yew_nested_router::prelude::Router;

  std::panic::set_hook(Box::new(console_error_panic_hook::hook));

  fern::Dispatch::new().level(log::LevelFilter::Info).chain(fern::Output::call(console_log::log)).apply().unwrap();

  #[component]
  fn Root() -> Html {
    html!(<Router<AppRoute> default={AppRoute::Home}><MainView /></Router<AppRoute>>)
  }

  yew::Renderer::<Root>::new().render();
}
