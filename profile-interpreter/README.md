# profile-interpreter

Turns a real ODRL Profile document (Turtle or JSON-LD) into the JSON this
engine's own wire contract actually consumes — an adapter, like
`compliance-runner`, not an engine or wire-contract change.

## Why this exists

Section 5.2's `config` field — the host's already-resolved union of every
ODRL Profile it has loaded, now shaped as real ODRL/JSON-LD
(`{"@type": "odrl:Profile", "odrl:action": [...], "dutyMode": ...}`, see
`engine`'s own `README.md`) — has to come from somewhere, but nothing in
this repo, before now, actually read a real ODRL Profile *document* to
produce it. A host had to hand-write the JSON. This tool reads the
document instead.

## What it does, and what it deliberately doesn't

Checked against the W3C ODRL Information Model's own Profile Mechanism
section (<https://www.w3.org/TR/odrl-model/#profile-mechanism>) rather
than assumed — see `src/interpret.rs`'s module doc for the full reasoning:

- A new Action is declared `ex:myAction a odrl:Action .` — every such
  subject becomes an `ActionDecl { id, included_in }` entry of
  `Profile.actions`.
- `odrl:includedIn` (a profile action naming a parent action) is captured
  as that `ActionDecl`'s own `included_in` field — real, usable data, not
  just a warning. `engine::ResolvedConfig::covers` walks exactly this
  declared edge to resolve action-taxonomy coverage (a permission for the
  parent now covers a request for the child), closing the gap Section 7 of
  the case study used to name as out of scope. This is still narrower than
  general inference: only *declared* edges are followed, and an action
  that is never itself typed `a odrl:Action` in the document contributes
  nothing even as someone else's `includedIn` target — declaring the edge
  is this tool's job, resolving it at evaluation time is the engine's.
- `duty_mode` is **never** read from the document — ODRL defines no
  property for a profile to declare its own enforcement behavior (that's
  this engine's own invention, Section 4.5), so it's always a
  caller-supplied flag, the same way it would be a real host's own
  deployment choice.
- `odrl:LeftOperand`/`odrl:Operator` extension declarations are noted
  (the former needs no action — this engine's leftOperand is already a
  free-form claims-map key; the latter genuinely can't be honored, since
  `engine::Operator` is a fixed enum) rather than silently ignored.

## Usage

```sh
# One profile document -> its own engine::Profile JSON (internal shape:
# id, actions: [{id, included_in}], duty_mode — not wire-shaped, since one
# profile alone is not a request config)
profile-interpreter interpret my-profile.ttl --duty-mode advise

# Multiple profile documents -> the merged Section 5.2 `config` field,
# printed as a wire-shaped engine::wire::RequestConfig
# (@type/@id/odrl:action/odrl:includedIn/dutyMode) — union of declared
# actions (and their includedIn edges), strictest duty_mode, the same
# merge rule engine::profile::resolve()'s own tests cover.
profile-interpreter resolve default-profile.ttl gaia-x-profile.jsonld --duty-mode deny
```

Format is inferred from each file's extension (`.ttl`/`.turtle`,
`.jsonld`/`.json`); override with `--format ttl|jsonld` if a file's
extension doesn't match its actual content. Warnings (an `includedIn`
relationship captured, an operator extension that can't be honored, a
missing `odrl:Profile`-typed subject falling back to a placeholder id)
print to stderr; the JSON output on stdout is always exactly what you'd
paste into a Section 5.2 request's `config` field (`resolve`) or a
`Profile` record (`interpret`).

## As a library

This CLI is a thin shell over `src/lib.rs` (`pub mod graph; pub mod
interpret;`) — any Rust caller can call `graph::Graph::from_turtle`/
`from_json_ld` and `interpret::interpret` directly, without shelling out
to a binary. `interpret::Interpreted` carries `declared_left_operands:
Vec<String>` (the raw `odrl:LeftOperand` local names, not just the
human-readable warning text) precisely so a caller building UI — the
`ds-odrl-engine-rs-site` Demonstrator page's "Load ODRL Profile" panel
is the motivating example — can populate a suggestion list without
re-parsing prose. `interpret::duty_mode_from_str` exists for the same
reason: a caller with no Rust-level dependency on the `engine` crate
itself can still produce a `DutyMode` value to pass to `interpret()`.
