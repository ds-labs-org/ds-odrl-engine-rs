//! Browser plumbing shared by the two live in-page runners (the
//! Compliance Results page's corpus run and the ODRL 2.2 Coverage page's
//! probe run).
//!
//! Deliberately only the *result-shape-independent* half. Each page keeps
//! its own `Stage` list and `RunState`: the two runs differ in stage
//! labels, in what their progress counters count (passed/failed/skipped
//! vs. agreed/disagreed/errored) and in their terminal payload, so a
//! generic `RunState<P, R>` plus a stages trait would be more code than
//! the ~60 lines it replaced and would couple two pages that should stay
//! free to grow different stage lists. The *idiom* is copied; the
//! plumbing is shared.
//!
//! `fetch_text` moved here from `compliance_run.rs`, where it carried its
//! own local `describe_js_error` — `err.as_string().unwrap_or_else(|| format!("{err:?}"))`,
//! precisely the weaker version an adversarial review had already removed
//! one file over. A failed `fetch()` rejects with a `TypeError`, not a
//! string, so that local copy *always* fell through to the raw
//! `JsValue(TypeError: ...)` debug dump — the exact leak that review
//! fixed. This module calls `engine_bridge::describe_js_error` instead, so
//! there is one error formatter in the crate rather than a good one and a
//! bad one.

use crate::engine_bridge::describe_js_error;

/// One frame at 60 Hz: the shortest interval at which yielding could let
/// anything new actually appear on screen.
pub const FRAME_MS: f64 = 16.0;

/// Hands control back to the browser's event loop so a pending Yew render
/// can actually paint. A resolved-`Promise` await would only reach the
/// *microtask* queue, which drains before the browser paints -- so this
/// goes through a real macrotask (`setTimeout(0)`) instead.
///
/// Two distinct jobs, both load-bearing:
///
/// * **Between the last two state transitions of a run.** Setting two Yew
///   `UseStateHandle`s back to back with no await in between coalesces
///   into a single render, so the first `set` is never painted at all.
///   Found by an adversarial review driving exactly that code path under
///   CPU throttling with a per-millisecond DOM sampler: the "Compiling
///   result report" stage existed in the state machine and never once
///   appeared on screen.
/// * **Inside a long loop**, at most once per [`FRAME_MS`] and never once
///   per item: on real hardware a whole run finishes in a few tens of
///   milliseconds, and a timeout per item would be manufacturing hundreds
///   of milliseconds of delay purely to make a fast thing look busy.
///   Neither runner contains artificial delay of any other kind either --
///   the elapsed time a finished report prints is real work, measured.
pub async fn yield_for_paint() {
  let promise = js_sys::Promise::new(&mut |resolve, _reject| {
    match web_sys::window() {
      Some(window) => {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
      }
      // No `window` never happens in a browser, but a never-resolving
      // promise here would hang the whole run rather than degrade it --
      // so resolve immediately instead.
      None => {
        let _ = resolve.call0(&wasm_bindgen::JsValue::undefined());
      }
    }
  });
  let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// `fetch(url)` as text, with every failure path rendered as a message a
/// visitor can actually read (see this module's header on the duplicate
/// formatter this replaced).
pub async fn fetch_text(url: &str) -> Result<String, String> {
  use wasm_bindgen::JsCast;

  let window = web_sys::window().ok_or_else(|| "no `window` (not running in a browser)".to_string())?;

  let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
    .await
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "fetch() did not resolve to a Response".to_string())?;

  if !response.ok() {
    return Err(format!("{url} fetch returned HTTP {}", response.status()));
  }

  wasm_bindgen_futures::JsFuture::from(response.text().map_err(describe_js_error)?)
    .await
    .map_err(describe_js_error)?
    .as_string()
    .ok_or_else(|| format!("{url}: response.text() did not resolve to a string"))
}
