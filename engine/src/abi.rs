//! Section 5.1's four-export WebAssembly ABI: `alloc`/`dealloc`/`evaluate`
//! over the guest's own linear memory, exported under the toolchain's
//! default `memory` name. `#[cfg(target_arch = "wasm32")]`'d in full — a
//! native host never calls through this boundary, it calls
//! `crate::wire::evaluate_request` directly (the compliance runner does,
//! per this stage's task), so keeping these `extern "C"` exports out of
//! non-wasm32 builds costs nothing and avoids exporting raw-pointer ABI
//! surface a native binary has no business exposing.

#![cfg(target_arch = "wasm32")]

use std::alloc::{alloc as sys_alloc, dealloc as sys_dealloc, Layout};

use crate::wire::{evaluate_request, parse_error_response, Request};

fn layout_for(len: usize) -> Layout {
    Layout::from_size_align(len.max(1), 1).expect("byte-buffer layout with alignment 1 is always valid")
}

/// Guest allocates `len` bytes and returns the pointer, for a host to
/// write a request payload into ahead of `evaluate` (Section 5.1).
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    let len = len.max(0) as usize;
    unsafe { sys_alloc(layout_for(len)) as i32 }
}

/// Frees a region previously returned by `alloc` (this module's own, or
/// the one `evaluate` allocates for its response) — the host calls this
/// **twice** per round trip (Section 5.1) so the guest's allocator does
/// not leak across repeated invocations of one long-lived `Instance`.
#[no_mangle]
pub extern "C" fn dealloc(ptr: i32, len: i32) {
    let len = len.max(0) as usize;
    unsafe { sys_dealloc(ptr as *mut u8, layout_for(len)) };
}

/// Reads `req_len` bytes of UTF-8 JSON at `req_ptr`, evaluates it, writes
/// a UTF-8 JSON response into a freshly `alloc`'d region, and packs
/// `(out_ptr << 32) | (out_len & 0xFFFFFFFF)` into the returned `i64`
/// (Section 5.1) — the caller is responsible for unpacking it, reading
/// `out_len` bytes at `out_ptr`, then `dealloc`-ing both the request
/// buffer it wrote and this response buffer.
#[no_mangle]
pub extern "C" fn evaluate(req_ptr: i32, req_len: i32) -> i64 {
    let bytes = unsafe { std::slice::from_raw_parts(req_ptr as *const u8, req_len.max(0) as usize) };

    let response_json = match serde_json::from_slice::<Request>(bytes) {
        Ok(req) => serde_json::to_vec(&evaluate_request(&req)),
        Err(err) => serde_json::to_vec(&parse_error_response(&err)),
    }
    .expect("Response always serializes: no non-finite floats, no non-string map keys");

    let out_len = response_json.len();
    let out_ptr = unsafe { sys_alloc(layout_for(out_len)) };
    unsafe { std::ptr::copy_nonoverlapping(response_json.as_ptr(), out_ptr, out_len) };

    ((out_ptr as i64) << 32) | (out_len as i64 & 0xFFFF_FFFF)
}
