//! The WASM-ABI invocation path (b), driven exactly as
//! `release-history/src/host.rs` in this same repo drives a historical
//! `engine.wasm`.
//!
//! This file is a deliberate, near-verbatim copy of that module rather
//! than a fresh implementation. The instruction that produced this bench
//! was explicit about it, and the reason is measurement validity: if this
//! harness invented its own way of packing the request into guest memory,
//! or skipped one of the two `dealloc` calls, or instantiated without
//! fuel where the dashboard instantiates with it, then the number
//! reported here would not be the cost of the thing the rest of the repo
//! actually executes. The only intended divergence is the one this bench
//! needs and `release-history` does not:
//!
//!   * `instantiate_with(wasm, fuel)` — fuel metering becomes a
//!     constructor argument instead of an unconditional `true`, so the
//!     bench can report BOTH the metered cost (what
//!     `release-history` really pays, and what any host that wants a
//!     runaway guard pays) and the unmetered cost (what
//!     `site/src/engine_bridge.rs` pays in a browser, where the JS engine
//!     provides no fuel accounting at all). Those are genuinely different
//!     numbers and collapsing them into one would be the misleading
//!     choice.
//!
//! Everything else — the empty `Linker`, `instantiate_and_start`, the
//! four-export lookup, the `(ptr << 32) | len` unpack, the
//! dealloc-on-trap path, the `FUEL_PER_CALL` constant — is the same code
//! and the same reasoning. See `release-history/src/host.rs` for the
//! full rationale on each.

use wasmi::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

/// Fuel budget per `evaluate()` call, identical to
/// `release-history/src/host.rs`'s. Not a performance knob there; here it
/// is also the metered/unmetered switch's budget, chosen the same way so
/// that the metered path measured is the metered path that repo code
/// actually runs.
const FUEL_PER_CALL: u64 = 20_000_000_000;

/// One instantiated `engine.wasm`, kept alive across cases the way a real
/// host keeps one long-lived `Instance` (hence the two `dealloc`s per
/// round trip: the guest allocator must not leak across a 68-case pass,
/// let alone across the hundreds of thousands of passes the load ramp
/// drives through a single instance).
pub struct WasmEngine {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    dealloc: TypedFunc<(i32, i32), ()>,
    evaluate: TypedFunc<(i32, i32), i64>,
    fuel: bool,
}

impl WasmEngine {
    /// Instantiates `engine.wasm`, with fuel metering on or off.
    pub fn instantiate_with(wasm: &[u8], fuel: bool) -> Result<Self, String> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(fuel);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| format!("engine.wasm did not parse as a module: {e}"))?;
        let mut store = Store::new(&engine, ());

        // An empty linker on purpose: Section 5.1's ABI is self-contained,
        // and a binary that turned out to want a host import (WASI, `env`,
        // anything) is a finding to surface, not something to satisfy with
        // a stub.
        let linker = Linker::<()>::new(&engine);
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

        Ok(Self { store, memory, alloc, dealloc, evaluate, fuel })
    }

    /// One round trip: request JSON in, response JSON out.
    pub fn evaluate(&mut self, request_json: &str) -> Result<String, String> {
        let bytes = request_json.as_bytes();
        let len = i32::try_from(bytes.len()).map_err(|_| "request larger than 2 GiB".to_string())?;

        if self.fuel {
            self.store.set_fuel(FUEL_PER_CALL).map_err(|e| format!("could not set fuel: {e}"))?;
        }

        let ptr = self.alloc.call(&mut self.store, len).map_err(|e| format!("alloc trapped: {e}"))?;
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .map_err(|e| format!("writing the request into guest memory failed: {e}"))?;

        let packed = match self.evaluate.call(&mut self.store, (ptr, len)) {
            Ok(packed) => packed,
            Err(e) => {
                // Still hand the request buffer back, so one trapping case
                // cannot starve the 67 that follow it.
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

    /// Current size of the guest's linear memory, in bytes.
    ///
    /// Reported per concurrency step by the load ramp: every thread in the
    /// wasm ramp owns a whole independent `Store`, so this is the part of
    /// the per-thread footprint that is the *guest's*, distinguishable
    /// from the interpreter's own host-side structures.
    pub fn guest_memory_bytes(&self) -> usize {
        self.memory.size(&self.store) as usize * 65536
    }
}
