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
  `EyeronLib` (in-process; eyeron is a git dependency pinned by `rev` in
  `Cargo.toml`). eyeron also parses the report output of any reasoner, so the
  crate has no RDF-library dependency and builds for `wasm32-unknown-unknown`
  as is: the ds42.org site embeds it (`/odrl-reasoner`).
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

`cargo test -p n3-enforcer` runs everything in-process, with no external reasoner, and is strict (any
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

## References and credits

Authors, editors and licences as each source states them (read from its manifest, licence file, BibTeX entry or specification front matter). The same list is rendered on the ds42.org page `/odrl-reasoner`, from `site/src/reasoner_demo.rs`.

- **Software:** [ODRL-Evaluator](https://github.com/SolidLabResearch/ODRL-Evaluator) — Wout Slabbinck (developer and maintainer), SolidLabResearch, IDLab, Ghent University – imec. MIT, © 2019–2025 imec. Its N3 rules (v0.6.0, commit 194894a) are used verbatim, with its licence text kept beside them. DOI 10.5281/zenodo.14265266.
- **Software:** [ODRL-Test-Suite](https://github.com/SolidLabResearch/ODRL-Test-Suite) — Wout Slabbinck, SolidLabResearch, IDLab, Ghent University – imec. MIT, © 2019–2025 imec. The 68 fixtures and expected compliance reports the differential test runs against. DOI 10.5281/zenodo.14290517.
- **Software:** [ODRL Compliance Report Model](https://github.com/SolidLabResearch/ODRL-Compliance-Report-Model) — SolidLabResearch. CC BY 4.0. The report: vocabulary the derived report is written in.
- **Software:** [eyeron](https://github.com/eyereasoner/eyeron) — Jos De Roo, KNoWS office of IDLab, Ghent University – imec. MIT. The pure-Rust N3 reasoner running in this page. The xsd:dateTime, log:collectAllIn and log:uuid fixes this path needs were developed in the ds-labs-org fork and merged as PR #9.
- **Software:** [EYE (Euler Yet another proof Engine)](https://github.com/eyereasoner/eye) — Jos De Roo, KNoWS office of IDLab, Ghent University – imec. MIT, © 2006–2026. The reference N3 reasoner eyeron follows and its test corpus.
- **Software:** [eye-js](https://github.com/eyereasoner/eye-js) — Jesse Wright (package author), eyereasoner. MIT. EYE compiled to WebAssembly; used only as an independent cross-check in the differential test, not in this page.
- **Paper:** [Interoperable Interpretation and Evaluation of ODRL Policies](https://doi.org/10.1007/978-3-031-94578-6_11) — Wout Slabbinck, Julián Rojas Meléndez, Beatriz Esteves, Pieter Colpaert and Ruben Verborgh. The Semantic Web (ESWC 2025), pp. 192–209. The paper behind the ODRL-Evaluator.
- **Paper:** [May the FORCE be with you? A Framework for ODRL Rule Compliance through Evaluation](https://ceur-ws.org/Vol-4064/NXDG25-paper6.pdf) — Wout Slabbinck, Julián Rojas Meléndez, Beatriz Esteves, Ruben Verborgh and Pieter Colpaert. NXDG 2025 at SEMANTiCS'25, CEUR-WS Vol-4064. Introduces the Compliance Report Model.
- **Paper:** [Automated Validation of ODRL Policies for Usage Control and Data Spaces](https://w3id.org/force/validator) — Elena Molino-Peña, Wout Slabbinck, José María García, Antonio Ruiz-Cortés and Beatriz Esteves. NXDG 2026 at SEMANTiCS 2026, Ghent. The validator/evaluator split that motivates the normalization work around this path.
- **Specification:** [ODRL Information Model 2.2](https://www.w3.org/TR/odrl-model/) — Renato Iannella and Serena Villata (editors), W3C Permissions & Obligations Expression Working Group. W3C Recommendation, 15 February 2018.
- **Specification:** [ODRL Vocabulary & Expression 2.2](https://www.w3.org/TR/odrl-vocab/) — Renato Iannella, Michael Steidl, Stuart Myles and Víctor Rodríguez-Doncel (editors). W3C Recommendation, 15 February 2018.
- **Specification:** [ODRL Formal Semantics](https://w3c.github.io/odrl/formal-semantics/) — Nicoletta Fornara, Víctor Rodríguez-Doncel, Beatriz Esteves, Simon Steyskal, Benedict Whittam Smith, Yassir Sellami and Andrea Cimmino Arriaga (editors), W3C ODRL Community Group. Community Group draft. The compliance-report semantics (Active/Inactive, premise reports) this path evaluates.
- **Specification:** [Notation3 Language](https://w3c-cg.github.io/N3/spec/) — William Van Woensel, Dörthe Arndt, Pierre-Antoine Champin, Dominik Tomaszuk and Gregg Kellogg (editors); Jos De Roo and Patrick Hochstenbach (authors), W3C N3 Community Group. Community Group draft. The rule language the ODRL-Evaluator is written in.
- **Specification:** [Notation3 Builtin Functions](https://w3c-cg.github.io/n3Builtins/) — William Van Woensel and Patrick Hochstenbach (editors), W3C N3 Community Group. Community Group final report. The log:, math:, list: and string: builtins the rules call.

The rule files in `rules/` are copied from the ODRL-Evaluator under its MIT licence, kept in `rules/LICENSE-ODRL-Evaluator.md`.
