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

## Loading a real ODRL Profile document

The Demonstrator page has a "Load ODRL Profile" panel above the form:
paste a real Turtle or JSON-LD ODRL Profile document, pick its format
(a paste carries no filename to infer a format from, unlike
`profile-interpreter`'s own CLI, which infers from extension — so this
panel asks explicitly, via the same `FormSelect` component the rest of
the form already uses), and click "Load profile". On success it shows
the profile's own id, how many actions and left operands it declares,
and any interpreter warnings (an `odrl:includedIn` relationship not
followed transitively, an extension it can't represent); on failure it
shows the parser's actual error message.

A loaded profile then configures the form:

- every action field — the new top-level "Requested Action" field
  (Section 5.2's `Request.action`), the `odrl:action` (`recognized_actions`)
  field, and each permission/prohibition/obligation's own action — gets an
  "insert from profile" picker offering the loaded profile's declared
  actions; the underlying field stays a free-form text input throughout, so
  a value typed by hand or set before a profile was loaded is never
  overwritten or restricted;
- an action field whose current value isn't among the loaded profile's
  declared actions gets an inline warning cue (a UI hint only, not a
  gate — the engine's own decision at evaluation time is still the
  authority: `Error` if this is a rule's own unrecognized action, `Deny`
  if this is the requested action and nothing declared covers it);
- the loaded profile's own declared actions **and their `odrl:includedIn`
  edges** flow directly into the constructed request's
  `config.odrl:action` list, not just into the suggestion pickers above —
  a permission for a profile-declared parent action genuinely covers a
  request for a profile-declared child action, exercised end to end
  against the real compiled `engine.wasm`, not just displayed in the UI;
- every `left_operand` constraint field gets an HTML `<datalist>` of
  suggestions — the loaded profile's declared `odrl:LeftOperand` names,
  plus `sub`/`nationality`/`scope`/`dateTime` (this repo's own README
  Section 5.2 worked example) seeded in even with no profile loaded.
  `leftOperand` stays deliberately free-form (Section 4.2 of the case
  study) — a `<datalist>` suggests, it does not restrict, which is why
  it's the right primitive here rather than a closed `<select>`.

This is a Rust-level dependency on `profile-interpreter` (and, through
it, `oxrdf`/`oxttl`/`oxjsonld`) — unlike the `engine`/`engine.wasm`
relationship described above, this is not a wire-contract shortcut:
`profile-interpreter` is a parsing *adapter* the Demonstrator now runs
client-side, and it never touches how `evaluate()` itself is called.
Getting that RDF stack to compile for `wasm32-unknown-unknown` needed
one extra piece: `getrandom` (pulled in transitively via `oxiri`) only
builds for this target with its `wasm_js` backend enabled, which needs
both the `getrandom = { version = "0.3", features = ["wasm_js"] }`
dependency *and* the `--cfg getrandom_backend="wasm_js"` rustflag in
`site/.cargo/config.toml` — scoped to `site/` (Cargo resolves
`.cargo/config.toml` by walking up from the invoking directory, and
Trunk always invokes from `site/`), not the repository root, so it has
no effect on `engine`'s own zero-dependency `wasm32-unknown-unknown`
build.

## Compliance Results page

This page does not display a compliance verdict somebody else committed.
It **computes one, in your browser, on load**, by running the whole
vendored ODRL Test Suite corpus against the real compiled `engine.wasm`
over the same raw `alloc`/`evaluate`/`dealloc` ABI described above.

Four stages, shown as a PatternFly `ProgressStepper` while they run:

1. **Loading engine.wasm** — `engine_bridge::ensure_loaded()`, split out
   of `evaluate()` so this is its own observable step rather than
   silently folding into the first case's evaluation. Returns the
   fetched artifact's byte length, which the finished report prints.
2. **Loading test cases** — fetches `compliance-data/latest-cases.json`,
   the corpus `compliance-runner` exports (see
   `../compliance-runner/src/cases.rs`): for every case, the *exact*
   `engine::wire::Request` the native run evaluated, plus the vendored
   suite's own expected decision. It deliberately carries **no** tally
   and **no** decision the native engine produced — nothing in that file
   may pre-decide what this page is supposed to compute. A wrong
   `schema`, an empty `cases` array or a truncated body fails this stage
   loudly. `compliance-data/latest.json` is fetched here too, but only
   as the native run's *baseline* (see below).
3. **Performing tests** — every case's request bytes go to
   `engine_bridge::evaluate()` **verbatim** (via
   `serde_json::value::RawValue`, never re-serialized through
   `src/wire.rs`, which models only flat constraints and would silently
   drop a nested `odrl:and`/`or`/`xone` one), and each answer is compared
   against the suite's expected decision. A case whose response isn't
   Section 5.2's envelope, or whose decision isn't one of
   `Allow`/`Deny`/`Error`, is counted **errored** — never quietly passed
   — and the loop continues, so one bad case can't hide the other 67.
4. **Compiling result report** — tally, then cross-check against the
   baseline.

The run makes **no artificial delay of any kind**. It yields to the
event loop (a real `setTimeout(0)` macrotask — a resolved-`Promise`
microtask drains *before* paint and would leave the counter jumping 0 to
68 in one frame) at most once per ~16 ms, so the progress count is a real
count rather than theatre, and the finished report prints the run's
measured wall-clock milliseconds.

**`compliance-data/latest.json` kept its `copy-file`, in a better job.**
It is no longer this page's source of truth; it is the *native*
`compliance-runner` run's recorded verdicts, and stage 4 compares the
live per-case results against it. Same corpus, same engine source, one
run through `engine::evaluate_request` natively and one through the
compiled `engine.wasm` ABI in a browser — so agreement is a real
native-vs-wasm cross-host check, and a divergence is either a genuine
finding or the signal that someone regenerated one artifact and not the
other. A failed baseline fetch is deliberately **not** fatal: the live
run is the authority and the page just says the cross-check was
unavailable. (Both artifacts come from one `cargo run -p
compliance-runner --release` invocation, so they agree by construction;
`src/compliance_cases.rs`'s own tests also assert at `cargo test` time
that the two committed files list the same slugs in the same order.)

**What the live run does and does not prove.** It proves that the real
compiled `engine.wasm`, instantiated in the visitor's own browser and
driven over its four-export C ABI, returns the vendored suite's expected
decision for every translated request, computed on their machine, now. It
does **not** re-derive the Turtle → `Request` translation
(`../compliance-runner/src/translate.rs`, with its own documented
fragilities) or the `report:*` → Allow/Deny ground truth
(`ground_truth.rs`) in-browser: both were computed natively and travel
inside the fetched artifact. Making those live too would mean shipping
the whole vendored corpus to `dist/` and lifting `translate.rs` into a
library this site depends on — deliberately out of scope. The page says
this in its own prose, not just here.

Code layout, and one deliberate exception to this crate's `#[cfg]`
convention: `src/compliance_cases.rs` (artifact parsing, the
expected-vs-actual comparison, the tally, the baseline cross-check) is
**not** `#[cfg(target_arch = "wasm32")]`-gated like every other module
here, because `cargo test --workspace` is a native build and a gated
module's unit tests would silently never compile, let alone run.
`src/compliance_run.rs` (the async driver and the state machine) and
`src/compliance_page.rs` (rendering) stay gated, since every line of them
touches the browser.

Regenerate both artifacts with:

```sh
cargo run -p compliance-runner --release
```

`Trunk.toml` deliberately does *not* run that as a build hook: it would
make a site build mutate tracked files outside `site/`, add a native
`oxrdf`/`oxttl` compile to every `trunk serve` rebuild, and undercut
`pages.yml`'s trigger design, which keys on `compliance/reports/**` being
*committed*. Trunk's `copy-file` does fail the build if either artifact
is missing, which is the right failure mode — this site must not deploy
without the corpus its Compliance page executes.

## Known limitations

- Cross-doc/page links are client-side SPA routes resolved against the
  page's `<base>` href, so a **direct** deep link (bookmark, shared URL,
  or a reload on a sub-route) requires the static host to serve
  `index.html` for unknown paths. `trunk serve` does this automatically.
  GitHub Pages does not, so it needs the `404.html`-redirect trick — now
  wired in: `site/404.html` (produced by Trunk's own `copy-file`, not
  hand-maintained) redirects a 404'd GitHub Pages request back to
  `index.html` with the original path folded into a query string, and a
  matching head script in `site/index.html` decodes it back with
  `history.replaceState` before the Yew app boots, so a direct hit or
  reload on `/demo`, `/compliance`, or `/coverage` lands on the right
  page instead of a 404. Confirmed against the live deployment.
- `copy-file` assets are not content-hashed (true of `engine.wasm` today
  too), so a returning visitor's browser may serve a cached
  `latest-cases.json`. Not a regression, and not silent: the finished run
  prints the artifact's own declared `suite` line and case count, so a
  stale fetch is visible rather than invisible, and a corpus whose
  `schema` doesn't match fails stage 2 outright.
- The Compliance Results page's assets are deliberately copied to
  `compliance-data/`, **not** `compliance/`, which would
  collide with this app's own `/compliance` SPA route the same way
  `dataspace/site` avoids colliding `authority-src/` with `/authority` —
  see `index.html`'s comment on that `copy-file` directive for the
  confirmed failure mode (a directory-redirect 404) this sidesteps.
