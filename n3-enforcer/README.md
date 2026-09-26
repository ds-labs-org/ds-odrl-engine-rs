# n3-enforcer (DRAFT)

A second implementation of `engine`'s wire contract, decided by a pure-Rust
**N3 reasoner** (`eyereasoner/eyeron`, >= 0.6.8) running SolidLabResearch's ODRL-Evaluator rules instead of
native Rust. Not on the hot path (see cost below); `Enforcer::enforce` falls back to
`engine::evaluate_request` for anything the rules cannot express. See ADR-0025 in the ds42.org hub repository
(`docs/adr/0025-reasoner-backed-second-enforcement-path.md`) for why, and
`docs/spikes/2026-09-26-odrl-evaluator-reasoner-integration-vs-enforcer-gap.md`
there for the study this draws on.

```
engine::Request ──rdf.rs──► N3 (policy + odrl:Request + state of the world)
      │
      ▼   round 1: constraints, premise reports, rule reports   (Reasoner::derive)
      ▼   round 2: activation state, given round 1's output     (Reasoner::derive)
report triples ──reduce.rs──► Allow / Deny   (prohibition > permission > closed default)
```

- `Reasoner` is one method: N3 document in, **newly derived triples** out
  (the `eye --pass-only-new` contract). Implementations: `CommandReasoner`
  (any binary: `eye`, the `eyeron` binary, `node eye-shim.mjs`) and
  `EyeronLib` (default feature `eyeron`; in-process; git-only, pinned by
  `rev` in `Cargo.toml`).
- `rules/round1.n3`, `rules/round2.n3` are the upstream rule strings
  copied verbatim (MIT; see `rules/NOTICE`). Draft-only: replace with a
  pinned submodule plus a regeneration step.
- **Anything the rules cannot express is refused, never guessed**
  (`Unsupported`): duties/obligations, refinements, nested logical
  constraints, conflict strategies, `inheritFrom`, asset collections,
  claim keys that are not ODRL leftOperands, `isAllOf`/`isPartOf`, a
  temporal constraint with no `dateTime` claim, and more than one context
  operand. A caller falls back to the native engine on `Unsupported`.
- A rule reported both `Active` and `Inactive` is surfaced as a
  contradiction in `ReportSummary`/the response reason, not resolved: it is
  the signature of a reasoner mishandling the non-monotonic
  `log:collectAllIn` counts.
- A constraint `sub eq X` (or `partyIdentityClaim eq X`) is translated to
  `odrl:assignee`, which is how the compliance adapter encodes ODRL party.

## Enforcing, and the differential test

```rust
let enforcer = n3_enforcer::Enforcer::eyeron();          // 100% Rust, in-process
let (response, path) = enforcer.enforce(&request);
// path == Path::Reasoner            decided by the N3 rules on eyeron
// path == Path::Native(reason)      the rules could not express it (Unsupported),
//                                   the report was contradictory, or the reasoner
//                                   failed: decided by engine::evaluate_request
```

`cargo test -p n3-enforcer` runs everything in-process on the default
`eyeron` feature, with no external reasoner, and is strict (any
disagreement, contradiction or error fails). Cases with more than 100
constraints are skipped unless `N3E_ALL=1` (see cost below).

To cross-check against another reasoner instead, set `N3E_REASONER`:

```sh
# real EYE via eye-js (npm i eyereasoner; shim mimics `eye --pass-only-new`)
N3E_REASONER=node N3E_KIND=args N3E_ARGS=/path/to/eye-shim.mjs \
  cargo test -p n3-enforcer -- --nocapture
N3E_REASONER=/usr/local/bin/eye N3E_KIND=eye cargo test -p n3-enforcer -- --nocapture
N3E_REASONER=/path/to/eyeron N3E_KIND=eyeron cargo test -p n3-enforcer -- --nocapture
```

`eye-shim.mjs`:

```js
import { n3reasoner } from 'eyereasoner';
import fs from 'node:fs';
const input = fs.readFileSync(process.argv.at(-1), 'utf8');
process.stdout.write(await n3reasoner(input, undefined, { output: 'derivations' }));
```

## Results on the 68 exported wire cases (2026-09-26)

| Reasoner | agree | n3 wrong | contradictory | unsupported | error |
|---|---|---|---|---|---|
| eye-js 21.1.24 (EYE on swipl-wasm) | **68** | 0 | 0 | 0 | 0 |
| **eyereasoner/eyeron `1c01133`** (0.6.8, in-process, pure Rust) | **68** | 0 | 0 | 0 | 0 |
| upstream eyereasoner/eyeron `1ad0386` | 41 | 11 | 16 | 0 | 0 |

"agree" means n3 = native engine = the suite's expected decision. The
upstream eyeron failures were three reasoner defects, fixed test-first in
`ds-labs-org/eyeron` and merged upstream as PR #9: `xsd:dateTime`/`xsd:date` ordering in
`math:` comparisons (11 wrong Denies), `log:collectAllIn` returning extra
singleton lists for blank-node templates (16 contradictions), and the
missing `log:uuid`.

**Cost.** In-process release build: median 57 ms per decision (two
reasoner rounds, including re-parsing the ~1,800-line ODRL vocabulary),
against 63 µs for the native engine through wasm. The three `big-policy`
fixtures (about 800 constraints each) take about 18 s each, because the
rules recount premise reports with `log:collectAllIn`; that is why this
path is an oracle and a fallback-guarded enforcer, not the hot path.

## Limits of this draft

- The 68 cases exercise only party (`sub`) and `dateTime` constraints, so
  agreement says little about context operands (`purpose`, ...), set
  operators, prohibitions with constraints, or `odrl:target`: `rdf.rs`
  translates them but nothing here tests them against the reasoner.
- The compared output is Allow/Deny. The engine's `DetailedEvaluation`
  already mirrors the `report:` vocabulary; comparing per-rule activation
  and per-premise satisfaction would be a much stronger check.
- The vocabulary is re-parsed on every call; a prepared reasoner
  (`eyeron::PreparedReasoner`) could cache it.
- Large policies are slow on the reasoner path (see cost).
