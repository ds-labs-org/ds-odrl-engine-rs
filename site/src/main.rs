#[cfg(target_arch = "wasm32")]
mod app_route;
// Deliberately NOT wasm32-gated like every other module here: `cargo test
// --workspace` is a native build, so this module's unit tests -- the
// artifact parsing, the expected-vs-actual comparison, the tally, and the
// cross-check against the native run's baseline -- would silently never
// compile, let alone run, behind that gate. Only its callers
// (`compliance_run`, `compliance_page`) touch the browser. The
// `allow(dead_code)` applies on the native side only, where nothing but
// those tests uses it, and keeps `cargo clippy --workspace --all-targets`
// clean without hiding real dead code from the wasm build.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod compliance_cases;
#[cfg(target_arch = "wasm32")]
mod compliance_page;
#[cfg(target_arch = "wasm32")]
mod compliance_run;
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
