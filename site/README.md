# ds-odrl-engine-rs-site

The documentation and demonstrator site for
[`ds-odrl-engine-rs`](..): a Yew + Trunk single-page app with a landing
page, an in-browser demonstrator over a real compiled `engine.wasm`, and
a Compliance Results page. Built with the same toolkit and visual
identity as the [ds42.org dataspace
study](https://github.com/Deepthought-Solutions/dataspace)'s own docs
site (`dataspace/site`) — [Yew](https://yew.rs/) +
[patternfly-yew](https://github.com/patternfly-yew/patternfly-yew) +
[yew-nested-router](https://github.com/ctron/yew-nested-router), built
with [Trunk](https://trunkrs.dev/), styled with
[PatternFly](https://www.patternfly.org/) 6.4.0 — re-themed with this
product's own teal brand ramp and a re-drawn mesh logo (same visual
language, honest `aria-label` for this site). Every page links back to
the case study this engine implements, filed at
`docs/case-studies/2026-08-30-attribute-based-odrl-policy-enforcement.md`
in that repository.

## Requirements

- [Rust](https://www.rust-lang.org/) — [install](https://rustup.rs/)
- [Trunk](https://trunkrs.dev/) **0.22.0-beta.2 or later**:
  `cargo install trunk@0.22.0-beta.2 --locked`
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- `npm`/`node` on `PATH` (Trunk shells out to `npm` to fetch the
  PatternFly and FontAwesome packages declared in `Trunk.toml`)

## Develop

```sh
trunk serve
```

Then open <http://localhost:8080>.

## Build

```sh
trunk build --release --public-url /ds-odrl-engine-rs/
```

(`--public-url /ds-odrl-engine-rs/` matches the GitHub Pages project
subpath this site deploys to; omit it, or pass `/`, for a build meant to
be served from a domain root.) Produces a fully static `dist/` —
deployable to any static host. No server-side component required.

## Why `engine` isn't a normal Rust dependency

`site/Cargo.toml` deliberately does **not** depend on the `engine` crate
(no `engine = { path = "../engine" }`, no other Rust-level link). The
engine's own design (Section 5 of the case study) is that a host talks
to `engine.wasm` as an **opaque compiled artifact** over its four-export
C ABI — `alloc(len) -> ptr`, `dealloc(ptr, len)`, `evaluate(req_ptr,
req_len) -> packed_ptr_len`, plus the toolchain's default `memory`
export (see `../engine/src/abi.rs`). A real host — a JS frontend, a JVM
host via Chicory — never links `engine`'s Rust source; it fetches the
compiled `.wasm` file and calls its raw exports across whatever FFI
boundary that host language has. If this site imported `engine` as a
normal Cargo dependency instead, its demonstrator would only be
proving that Rust can call Rust — not that the wire contract and ABI
actually work across a real host boundary, which is the entire point of
compiling to `wasm32-unknown-unknown` in the first place.

So this site treats `engine.wasm` as a black box, the same way any other
host would:

- **Build hook, not a dependency edge** — `Trunk.toml`'s `pre_build`
  hook runs `cargo build -p engine --target wasm32-unknown-unknown
  --release` against the sibling workspace crate and copies the result
  to `assets/engine.wasm` *before* Trunk's own asset pipeline (its
  `copy-file` directive in `index.html`) runs. Both the build and the
  copy live in one `sh -c` hook rather than two separate `[[hooks]]`
  entries, because Trunk runs every hook in a stage concurrently, not in
  listed order — two separate hooks raced in practice during
  development. `trunk serve`'s file watcher also excludes
  `assets/engine.wasm` (`[watch] ignore`), because the hook rewrites
  that file on every build, and without the exclusion the rewrite itself
  triggers another build, forever.
- **`engine_module.rs`** fetches the compiled artifact at runtime with a
  plain `fetch()` (relative to the page's `<base href>`, so it resolves
  correctly both under `trunk serve` and under the GitHub Pages project
  subpath) and reports its byte length — proving the fetch mechanism
  independent of the ABI.
- **`engine_bridge.rs`** is the actual ABI bridge: it instantiates the
  fetched bytes as a second, plain (non-wasm-bindgen) `WebAssembly`
  module via `js_sys::WebAssembly`, with an empty import object (the
  engine needs no host imports — no clock, no network), then drives the
  `alloc` / write-request-bytes / `evaluate` / read-response-bytes /
  `dealloc` round trip by hand against that instance's raw exports and
  linear memory. It re-reads `memory.buffer()` after every call that
  might grow (and so detach and reallocate) it, rather than holding a
  view across calls.

This is deliberately more code than `engine = { path = "../engine" }`
would be. That's the point: it's an honest demonstration of the same
wire contract and ABI a real independent host has to implement, not a
shortcut that only works because the demo and the engine happen to share
one Rust compilation.

## Compliance Results page

`compliance_page.rs` fetches `compliance-data/latest.json` at runtime
(copied from `../compliance/reports/latest.json` by `index.html`'s own
`copy-file` directive) rather than embedding it at this site's Rust
compile time, so redeploying after a future `compliance-runner` run
picks up new numbers with no code change — just re-copying that file and
rebuilding.

## Known limitations

- Cross-doc/page links are client-side SPA routes resolved against the
  page's `<base>` href, so a **direct** deep link (bookmark, shared URL)
  requires the static host to serve `index.html` for unknown paths.
  `trunk serve` does this automatically; GitHub Pages needs the
  `404.html`-redirect trick (or an equivalent) if deep-linking to a
  sub-route is required — not yet set up here since every current page
  is reachable from the Home nav.
- The Compliance Results page's asset is deliberately copied to
  `compliance-data/latest.json`, **not** `compliance/`, which would
  collide with this app's own `/compliance` SPA route the same way
  `dataspace/site` avoids colliding `authority-src/` with `/authority` —
  see `index.html`'s comment on that `copy-file` directive for the
  confirmed failure mode (a directory-redirect 404) this sidesteps.
