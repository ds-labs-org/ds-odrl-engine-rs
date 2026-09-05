//! Fetches the compiled `engine.wasm` artifact (see Trunk.toml's pre_build
//! hook) exactly as a real JS/JVM host would: as an opaque byte blob over
//! `fetch()`, not by linking the `engine` crate (see site/Cargo.toml's own
//! header comment). This stage only proves the fetch and reports the byte
//! count on the Home page; instantiating the module and driving its
//! `alloc`/`dealloc`/`evaluate` C ABI (engine/src/abi.rs) is the
//! Demonstrator page's job in a later stage.
//!
//! The request path below, `"engine.wasm"`, is deliberately relative with
//! no leading slash. index.html stamps a `<base href>` from Trunk's own
//! `--public-url` (via `<base data-trunk-public-url />`), so this resolves
//! against that base -- "/ds-odrl-engine-rs/engine.wasm" once deployed to
//! GitHub Pages' project subpath, "/engine.wasm" under a local `trunk
//! serve` -- rather than against whatever route the SPA router currently
//! shows in the address bar (which would break this fetch under e.g.
//! "/ds-odrl-engine-rs/demo"). This is the mechanism, exercised for real
//! every time the Home page mounts.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

/// Fetches `engine.wasm` relative to the document's own `<base href>` and
/// returns its raw byte length on success.
pub async fn fetch_engine_wasm_len() -> Result<usize, String> {
  let window = web_sys::window().ok_or_else(|| "no `window` (not running in a browser)".to_string())?;

  let response: Response = JsFuture::from(window.fetch_with_str("engine.wasm"))
    .await
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "fetch() did not resolve to a Response".to_string())?;

  if !response.ok() {
    return Err(format!("engine.wasm fetch returned HTTP {}", response.status()));
  }

  let buffer = JsFuture::from(response.array_buffer().map_err(describe_js_error)?)
    .await
    .map_err(describe_js_error)?;
  let bytes = js_sys::Uint8Array::new(&buffer);
  Ok(bytes.length() as usize)
}

fn describe_js_error(err: JsValue) -> String {
  err.as_string().unwrap_or_else(|| format!("{err:?}"))
}
