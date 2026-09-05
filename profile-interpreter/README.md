# profile-interpreter

Turns a real ODRL Profile document (Turtle or JSON-LD) into the JSON this
engine's own wire contract actually consumes — an adapter, like
`compliance-runner`, not an engine or wire-contract change.

## Why this exists

Section 5.2's `config` field (`{recognized_actions, duty_mode}`) is
described as "the host's already-resolved union of every ODRL Profile it
has loaded" — but nothing in this repo, before now, actually read a real
ODRL Profile *document* to produce that union. A host had to hand-write
the JSON. This tool reads the document instead.

## What it does, and what it deliberately doesn't

Checked against the W3C ODRL Information Model's own Profile Mechanism
section (<https://www.w3.org/TR/odrl-model/#profile-mechanism>) rather
than assumed — see `src/interpret.rs`'s module doc for the full reasoning:

- A new Action is declared `ex:myAction a odrl:Action .` — every such
  subject's local name becomes a `recognized_actions` entry.
- `odrl:includedIn` (a profile action naming a parent action) is noted as
  a warning, not followed transitively — chasing it would be exactly the
  general action-taxonomy-implication problem Section 7 of the case study
  names as out of scope for this engine.
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
# One profile document -> its own engine::Profile JSON
profile-interpreter interpret my-profile.ttl --duty-mode advise

# Multiple profile documents -> the merged Section 5.2 `config` field
# (union of recognized_actions, strictest duty_mode) — engine::resolve()
# under the hood, the exact function `engine::profile`'s own tests cover.
profile-interpreter resolve default-profile.ttl gaia-x-profile.jsonld --duty-mode deny
```

Format is inferred from each file's extension (`.ttl`/`.turtle`,
`.jsonld`/`.json`); override with `--format ttl|jsonld` if a file's
extension doesn't match its actual content. Warnings (an `includedIn`
relationship not followed, an operator extension that can't be honored,
a missing `odrl:Profile`-typed subject falling back to a placeholder id)
print to stderr; the JSON output on stdout is always exactly what you'd
paste into a Section 5.2 request's `config` field (`resolve`) or a
`Profile` record (`interpret`).
