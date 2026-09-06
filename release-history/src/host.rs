//! A minimal WebAssembly host for a *historical* `engine.wasm`.
//!
//! This drives Section 5.1's four-export ABI — `alloc`, `dealloc`,
//! `evaluate`, over the guest's own `memory` — exactly as
//! `site/src/engine_bridge.rs` drives it from JavaScript in a browser:
//! allocate a request buffer in the guest, write the UTF-8 JSON bytes into
//! it, call `evaluate`, unpack `(ptr << 32) | len` out of the returned
//! `i64`, read the response back out of linear memory, then `dealloc`
//! both regions.
//!
//! **Why an interpreter and not a Rust dependency.** The obvious
//! alternative — add each historical `engine` crate as a path dependency
//! and call `engine::wire::evaluate_request` natively — was tried in
//! outline and rejected: it would need each tag's *source* to compile
//! inside today's workspace, against today's `Cargo.toml`, today's
//! resolver and today's dependency versions, and every answer would come
//! from a fresh compilation rather than from the artifact that release
//! actually shipped. Loading the tagged, already-compiled `engine.wasm`
//! removes all of that. A historical release's numbers here are produced
//! by that release's own binary, byte-for-byte the file
//! `scripts/build-release-history.sh` built at its tag.
//!
//! It also happens to be the honest host shape: the site never links the
//! engine crate either (see `site/Cargo.toml`'s note), precisely so the
//! wire contract is exercised rather than short-circuited.

use wasmi::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

/// Fuel budget per `evaluate()` call.
///
/// Not a performance knob: an interpreter running a *historical* binary
/// has no upstream guarantee that some long-fixed bug cannot loop
/// forever on an input the current catalog feeds it, and a generator that
/// hangs on tag 3 of 19 is worse than one that reports a probe as
/// errored. 20 billion is roughly four orders of magnitude above the
/// measured cost of the heaviest probe in the current catalog (the
/// 51-action taxonomy ones), so it can only fire on a genuine runaway.
const FUEL_PER_CALL: u64 = 20_000_000_000;

/// One instantiated historical engine, kept alive across probes the way a
/// real host keeps one long-lived `Instance` (which is why `dealloc` is
/// called on both buffers: the guest allocator must not leak across 125
/// invocations).
pub struct HistoricalEngine {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    dealloc: TypedFunc<(i32, i32), ()>,
    evaluate: TypedFunc<(i32, i32), i64>,
}

impl HistoricalEngine {
    /// Instantiates one tagged `engine.wasm`.
    ///
    /// Every failure path is an `Err` a caller can attribute to a release
    /// rather than a panic that loses which release it was: a tag whose
    /// binary predates one of the four exports must show up on the
    /// dashboard as a release whose probes could not be run, not as an
    /// aborted generator.
    pub fn instantiate(wasm: &[u8]) -> Result<Self, String> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| format!("engine.wasm did not parse as a module: {e}"))?;
        let mut store = Store::new(&engine, ());

        // An empty linker on purpose: Section 5.1's ABI is self-contained,
        // and a historical binary that turned out to want a host import
        // (WASI, `env`, anything) is a finding to surface, not something
        // to satisfy with a stub.
        let linker = Linker::<()>::new(&engine);
        // `instantiate_and_start` rather than the two-step
        // instantiate/start: a cdylib built by rustc has no `start`
        // section to run separately, and folding both into one call keeps
        // a trap in either half attributable to the same release.
        let instance: Instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("engine.wasm could not be instantiated with no host imports: {e}"))?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| "engine.wasm exports no `memory`".to_string())?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, "alloc")
            .map_err(|e| format!("engine.wasm exports no `alloc(i32) -> i32`: {e}"))?;
        let dealloc = instance
            .get_typed_func::<(i32, i32), ()>(&store, "dealloc")
            .map_err(|e| format!("engine.wasm exports no `dealloc(i32, i32)`: {e}"))?;
        let evaluate = instance
            .get_typed_func::<(i32, i32), i64>(&store, "evaluate")
            .map_err(|e| format!("engine.wasm exports no `evaluate(i32, i32) -> i64`: {e}"))?;

        Ok(Self { store, memory, alloc, dealloc, evaluate })
    }

    /// One round trip: request JSON in, response JSON out.
    pub fn evaluate(&mut self, request_json: &str) -> Result<String, String> {
        let bytes = request_json.as_bytes();
        let len = i32::try_from(bytes.len()).map_err(|_| "request larger than 2 GiB".to_string())?;

        self.store.set_fuel(FUEL_PER_CALL).map_err(|e| format!("could not set fuel: {e}"))?;

        let ptr = self.alloc.call(&mut self.store, len).map_err(|e| format!("alloc trapped: {e}"))?;
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .map_err(|e| format!("writing the request into guest memory failed: {e}"))?;

        let packed = match self.evaluate.call(&mut self.store, (ptr, len)) {
            Ok(packed) => packed,
            Err(e) => {
                // Still hand the request buffer back, so one trapping
                // probe cannot starve the 124 that follow it.
                let _ = self.dealloc.call(&mut self.store, (ptr, len));
                return Err(format!("evaluate trapped: {e}"));
            }
        };

        let out_ptr = (packed >> 32) as i32;
        let out_len = (packed & 0xFFFF_FFFF) as i32;
        let mut out = vec![0u8; out_len.max(0) as usize];
        let read = self.memory.read(&self.store, out_ptr as usize, &mut out);

        self.dealloc.call(&mut self.store, (ptr, len)).map_err(|e| format!("dealloc(request) trapped: {e}"))?;
        self.dealloc
            .call(&mut self.store, (out_ptr, out_len))
            .map_err(|e| format!("dealloc(response) trapped: {e}"))?;

        read.map_err(|e| format!("reading the response out of guest memory failed: {e}"))?;
        String::from_utf8(out).map_err(|e| format!("engine.wasm returned non-UTF-8 bytes: {e}"))
    }
}
