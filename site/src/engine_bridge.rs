//! Drives `engine.wasm`'s own four-export C ABI (`memory`/`alloc`/
//! `dealloc`/`evaluate`, see `engine/src/abi.rs`) exactly as a real JS or
//! JVM host would: this module is itself compiled to wasm via
//! wasm-bindgen, and separately instantiates a SECOND, plain (non
//! wasm-bindgen) wasm module fetched at runtime, then calls its raw
//! exports by hand across the `js_sys::WebAssembly` boundary. There is no
//! Rust-level link to the `engine` crate (see site/Cargo.toml's header
//! comment) -- everything below only knows about `engine.wasm` as an
//! opaque binary and its documented ABI.
//!
//! The instantiated `WebAssembly::Instance` is cached in a thread-local
//! (wasm32-in-a-browser is single-threaded, so this is the idiomatic
//! wasm-bindgen equivalent of a lazily-initialized global) so repeat calls
//! to [`evaluate`] reuse the same module instead of re-fetching and
//! re-instantiating `engine.wasm` every time.

use std::cell::RefCell;

use js_sys::{BigInt, Function, Object, Reflect, Uint8Array, WebAssembly};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

thread_local! {
  static ENGINE: RefCell<Option<EngineInstance>> = const { RefCell::new(None) };
}

/// The instantiated `engine.wasm` module's memory and the three exported
/// functions this bridge calls (Section 5.1's ABI), plus the fetched
/// artifact's own byte length -- kept so a caller can state exactly which
/// compiled binary it drove (`engine_module::fetch_engine_wasm_len`
/// reports the same number for the Home page, from its own separate
/// fetch).
struct EngineInstance {
  memory: WebAssembly::Memory,
  alloc: Function,
  dealloc: Function,
  evaluate: Function,
  byte_len: usize,
}

/// Fetches and instantiates `engine.wasm` if this page hasn't already,
/// and returns the compiled artifact's byte length. Idempotent: a later
/// call returns the cached instance's length without re-fetching.
///
/// Split out of [`evaluate`] so a caller that wants "loading the module"
/// to be its own observable stage -- the Compliance Results page's live
/// runner, whose first progress step is exactly this -- can await it
/// separately, instead of having it silently fold into the first case's
/// `evaluate()` call and make that one case look inexplicably slow.
pub async fn ensure_loaded() -> Result<usize, String> {
  let cached = ENGINE.with(|cell| cell.borrow().as_ref().map(|engine| engine.byte_len));
  if let Some(byte_len) = cached {
    return Ok(byte_len);
  }

  let instance = load_engine_instance().await?;
  let byte_len = instance.byte_len;
  ENGINE.with(|cell| *cell.borrow_mut() = Some(instance));
  Ok(byte_len)
}

/// Evaluates `request_json` (Section 5.2's request shape) against
/// `engine.wasm`'s `evaluate` export, returning the raw response JSON
/// string on success. Fetches and instantiates `engine.wasm` on the first
/// call only; later calls reuse the cached instance.
pub async fn evaluate(request_json: &str) -> Result<String, String> {
  ensure_loaded().await?;

  ENGINE.with(|cell| {
    let borrowed = cell.borrow();
    let engine = borrowed.as_ref().expect("just loaded above, or already present");
    engine.evaluate_request(request_json)
  })
}

/// Fetches `engine.wasm` relative to this page's own `<base href>` (same
/// mechanism as `engine_module::fetch_engine_wasm_len`) and instantiates
/// it with an **empty** import object -- Section 5's own design constraint
/// is that the engine needs no host imports at all (no clock, no network).
async fn load_engine_instance() -> Result<EngineInstance, String> {
  let window = web_sys::window().ok_or_else(|| "no `window` (not running in a browser)".to_string())?;

  let response: Response = JsFuture::from(window.fetch_with_str("engine.wasm"))
    .await
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "fetch() did not resolve to a Response".to_string())?;
  if !response.ok() {
    return Err(format!("engine.wasm fetch returned HTTP {}", response.status()));
  }

  let array_buffer = JsFuture::from(response.array_buffer().map_err(describe_js_error)?)
    .await
    .map_err(describe_js_error)?;
  let module_bytes = Uint8Array::new(&array_buffer).to_vec();

  // No imports: engine.wasm declares none (Section 5's no-host-dependency
  // design), so an empty Object satisfies WebAssembly.instantiate's
  // required second argument.
  let imports = Object::new();

  // `WebAssembly::instantiate_buffer` resolves to a `WebAssembly.Result`
  // object shaped `{ module, instance }`, not an `Instance` directly (the
  // `js_sys_unstable_apis` cfg that would give us a typed `Promise<Instance>`
  // isn't enabled in this build) -- pull `instance` out by hand via
  // `Reflect`.
  let result_object = JsFuture::from(WebAssembly::instantiate_buffer(&module_bytes, &imports))
    .await
    .map_err(describe_js_error)?;
  let instance: WebAssembly::Instance = Reflect::get(&result_object, &JsValue::from_str("instance"))
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "WebAssembly.instantiate(...) result had no `instance` property".to_string())?;

  let exports = instance.exports();
  let memory: WebAssembly::Memory = Reflect::get(&exports, &JsValue::from_str("memory"))
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| "engine.wasm exports no `memory`".to_string())?;
  let alloc = get_exported_function(&exports, "alloc")?;
  let dealloc = get_exported_function(&exports, "dealloc")?;
  let evaluate = get_exported_function(&exports, "evaluate")?;

  Ok(EngineInstance { memory, alloc, dealloc, evaluate, byte_len: module_bytes.len() })
}

fn get_exported_function(exports: &Object, name: &str) -> Result<Function, String> {
  Reflect::get(exports, &JsValue::from_str(name))
    .map_err(describe_js_error)?
    .dyn_into()
    .map_err(|_| format!("engine.wasm exports no `{name}` function"))
}

impl EngineInstance {
  /// The `alloc`/write/`evaluate`/read/`dealloc`x2 round trip from
  /// Section 5.1's ABI spec, taking a *fresh* `Uint8Array` view of
  /// `memory.buffer()` after every call that might grow (and so detach and
  /// reallocate) the underlying `ArrayBuffer` -- an already-held view would
  /// silently read/write the wrong (stale, detached) buffer otherwise.
  fn evaluate_request(&self, request_json: &str) -> Result<String, String> {
    let request_bytes = request_json.as_bytes();
    let request_len = request_bytes.len() as u32;

    // 1. alloc(request_len) -> req_ptr.
    let req_ptr = call_returning_u32(&self.alloc, &JsValue::from_f64(request_len as f64))?;

    // 2. Write the request bytes at req_ptr, via a fresh view taken *after*
    //    the alloc call above (alloc's own allocator growth, if any, may
    //    have moved the buffer).
    {
      let memory_view = Uint8Array::new(&self.memory.buffer());
      memory_view.subarray(req_ptr, req_ptr + request_len).copy_from(request_bytes);
    }

    // 3. evaluate(req_ptr, req_len) -> packed i64, surfaced to JS as a
    //    BigInt (every current browser represents a wasm i64 return value
    //    this way when called directly, not through wasm-bindgen glue).
    let packed = self
      .evaluate
      .call2(&JsValue::undefined(), &JsValue::from_f64(req_ptr as f64), &JsValue::from_f64(request_len as f64))
      .map_err(describe_js_error)?;
    let packed_bigint: BigInt = packed.dyn_into().map_err(|_| "evaluate() did not return a BigInt (i64)".to_string())?;
    let packed_u64: u64 = u64::try_from(packed_bigint).map_err(|_| "evaluate()'s packed i64 did not fit a u64".to_string())?;
    let out_ptr = (packed_u64 >> 32) as u32;
    let out_len = (packed_u64 & 0xFFFF_FFFF) as u32;

    // 4. Read out_len bytes at out_ptr, via a FRESH view taken after the
    //    evaluate() call (evaluate allocates its own response buffer
    //    internally, which may have grown/moved memory again).
    let response_bytes = {
      let memory_view = Uint8Array::new(&self.memory.buffer());
      memory_view.subarray(out_ptr, out_ptr + out_len).to_vec()
    };

    // 5. Free both buffers -- the request buffer this call wrote, and the
    //    response buffer evaluate() allocated -- so the guest's allocator
    //    doesn't leak across repeated calls against this one long-lived
    //    Instance.
    call_ignoring_result(&self.dealloc, req_ptr, request_len)?;
    call_ignoring_result(&self.dealloc, out_ptr, out_len)?;

    String::from_utf8(response_bytes).map_err(|err| format!("engine.wasm response was not valid UTF-8: {err}"))
  }
}

fn call_returning_u32(f: &Function, arg: &JsValue) -> Result<u32, String> {
  let result = f.call1(&JsValue::undefined(), arg).map_err(describe_js_error)?;
  result.as_f64().map(|v| v as u32).ok_or_else(|| "expected a numeric return value".to_string())
}

fn call_ignoring_result(f: &Function, a: u32, b: u32) -> Result<(), String> {
  f.call2(&JsValue::undefined(), &JsValue::from_f64(a as f64), &JsValue::from_f64(b as f64))
    .map(|_| ())
    .map_err(describe_js_error)
}

/// Renders a caught `JsValue` as a short, user-presentable message,
/// preferring the most specific form available: a plain JS string, then a
/// real JS `Error`'s own `.message` (this is the common case for a failed
/// `fetch()` -- it rejects with a `TypeError`, not a string, so
/// `as_string()` alone always misses it), and only as a last resort the
/// full `{err:?}` debug dump wasm-bindgen produces (a multi-line stack
/// trace with mangled wasm symbol names) -- found leaking into a
/// user-facing `Alert` by an adversarial review of the first feature to
/// put this fallback somewhere prominent enough to notice.
fn describe_js_error(err: JsValue) -> String {
  if let Some(s) = err.as_string() {
    return s;
  }
  if let Some(message) = err.dyn_ref::<js_sys::Error>().map(|e| e.message()) {
    return message.into();
  }
  format!("{err:?}")
}
