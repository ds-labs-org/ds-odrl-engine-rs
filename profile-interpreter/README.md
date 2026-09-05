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
- `behaviour` (open/closed, the empty-permissions-list evaluator setting
  from the ODRL Formal Semantics draft, engine Section 4.3) is likewise
  never read from the document — same reasoning as `duty_mode` — so it's
  a caller-supplied flag too, defaulting to `open` when omitted.
- `odrl:LeftOperand`/`odrl:Operator` extension declarations are noted
  (the former needs no action — this engine's leftOperand is already a
  free-form claims-map key; the latter genuinely can't be honored, since
  `engine::Operator` is a fixed enum) rather than silently ignored.
- `odrl:refinement` (the engine's `Rule.action_refinement`, root README's
  "Action refinement") is **not** a Profile-document construct and this
  tool never emits one: a refinement narrows an action inside a *policy*,
  whereas a Profile declares which actions exist at all. The connection
  is indirect and already covered — a refinement's `left_operand`s are
  ordinary claim-map keys, so a profile declaring them as
  `odrl:LeftOperand` extensions surfaces them through
  `declared_left_operands` exactly as it does for a rule's constraints.

## Usage

```sh
# One profile document -> its own engine::Profile JSON (internal shape:
# id, actions: [{id, included_in}], duty_mode, behaviour — not
# wire-shaped, since one profile alone is not a request config)
profile-interpreter interpret my-profile.ttl --duty-mode advise --behaviour closed

# Multiple profile documents -> the merged Section 5.2 `config` field,
# printed as a wire-shaped engine::wire::RequestConfig
# (@type/@id/odrl:action/odrl:includedIn/dutyMode/behaviour) — union of
# declared actions (and their includedIn edges), strictest duty_mode and
# behaviour, the same merge rule engine::profile::resolve()'s own tests
# cover.
profile-interpreter resolve default-profile.ttl gaia-x-profile.jsonld --duty-mode deny --behaviour open
```

Both `--duty-mode` and `--behaviour` default to their engine-side
defaults (`advise`, `open`) when omitted.

Format is inferred from each file's extension (`.ttl`/`.turtle`,
`.jsonld`/`.json`); override with `--format ttl|jsonld` if a file's
extension doesn't match its actual content. Warnings (an `includedIn`
relationship captured, an operator extension that can't be honored, a
missing `odrl:Profile`-typed subject falling back to a placeholder id)
print to stderr; the JSON output on stdout is always exactly what you'd
paste into a Section 5.2 request's `config` field (`resolve`) or a
`Profile` record (`interpret`).

## `examples/odrl-2.2-common-actions.ttl`

A loadable ODRL Profile document declaring the W3C ODRL 2.2 Vocabulary's
own Action taxonomy in full — the two Core Vocabulary roots (`use`,
`transfer`, Section 3.12) plus every one of the 49 Common Vocabulary
actions Section 4.4 individually defines (40 native `odrl:` terms and 9
Creative Commons terms ODRL adopts by reference), each with the
`odrl:includedIn` edge that section's own definition table states for
it. It was produced by fetching and reading
<https://www.w3.org/TR/odrl-vocab/> directly, term by term — every edge
in the file is transcribed from that page's own `Included In:` row, none
inferred or guessed; the file's own header comment says exactly which
section each group comes from, and flags a real spec quirk (Section
4.4.8's own published identifier for "Commercial Use" is the mis-spelled
`http://creativecommons.org/ns#CommericalUse`, transcribed here as
published rather than silently corrected). It exists as ready-to-use,
fully-sourced vocabulary data for a host that wants the *complete*
standard action taxonomy recognized (as opposed to, say,
`compliance-runner`'s own deliberately narrow, corpus-driven
`base_action_vocabulary`, which declares only the handful of actions
that vendored fixture corpus actually exercises). It parses with this
crate's existing `interpret`/`resolve` commands unmodified — no parser
change was needed to load it:

```sh
# Its own Profile-shaped JSON (51 actions; a placeholder id, since the
# document itself never types a subject `a odrl:Profile` -- pass --id
# if you want one):
profile-interpreter interpret examples/odrl-2.2-common-actions.ttl

# As one of several profiles feeding a Section 5.2 request `config`,
# alongside whatever profile-specific extensions a host also loads:
profile-interpreter resolve examples/odrl-2.2-common-actions.ttl my-extension-profile.ttl --duty-mode advise --behaviour open
```

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
`interpret::behaviour_from_str` (accepting `"open"`/`"closed"`/the ODRL
Formal Semantics draft's own `"default"` alias for `"closed"`) and
`interpret::default_behaviour` exist for the same reason on the
`Behaviour` side — the latter is what `ds-odrl-engine-rs-site`'s
"Load ODRL Profile" panel passes, since a profile document never
declares its own `behaviour` (see above) and that panel has no form
field for one; the Demonstrator's own `behaviour` selection reaches the
engine through a separate path, not through this crate.

Do not confuse `declared_left_operands` with the `engine` crate's own
`Policy::referenced_left_operands` / `left_operands_for_request` (root
README, "Asking which claims a set of policies actually reads"): this
list is what a **profile document** declares as vocabulary
(`odrl:LeftOperand`-typed subjects), that one is what **actual policies**
reference in their constraints. Neither constrains the other — the
engine's `left_operand` is a free-form claims-map key, never validated
against a profile's declared vocabulary, which is exactly what the
warning text emitted alongside this list already says. Both lists are
sorted and deduped by the same convention, so a caller wanting to compare
or merge them can.

The **action** half of a resolved config works the other way round, and is
worth knowing when writing a profile document: the actions `resolve` emits
into `config.odrl:action` are exactly the enumeration domain of the
engine's `performable_actions` / `performable_actions_for_request` (root
README, "Asking which actions a caller could actually perform"). An action
a profile document never declares as its own `odrl:Action` subject can
therefore never appear in that answer — not even when some other declared
action names it as an `odrl:includedIn` parent, which is the same rule
`engine::ResolvedConfig::recognizes` has always applied to a rule's own
action. A profile that under-declares its vocabulary quietly narrows what
a broker asking "what may this caller do?" is told, with no warning from
here, since nothing in this crate can tell an omission from a deliberately
narrow profile.

## The other half: ingesting a real contract policy

This crate reads a Profile document — the *vocabulary* declaration — and
produces a request's `config`. It deliberately does not read a **policy**:
a Profile says which actions exist, not what any particular offer permits.
That other half lives in [`dsp-odrl-adapter`](../dsp-odrl-adapter/) (root
README, "Ingesting a real DSP contract offer"), which turns a Dataspace
Protocol contract offer/agreement's ODRL JSON-LD into a Section 5.2
`WirePolicy`, and is opt-in behind a default-off Cargo feature.

The two compose, and are meant to: `dsp-odrl-adapter`'s own
`minimal_config` is a floor that declares only the actions the ingested
policy happens to name, with no `odrl:includedIn` edges at all — so a host
wanting real action-taxonomy coverage builds `config` here, from real
Profile documents, and passes it alongside the ingested policy rather than
taking that floor.
