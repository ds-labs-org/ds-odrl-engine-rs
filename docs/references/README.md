# References

Academic papers this project's design has been checked against, mirrored
locally (both CC BY 4.0, redistribution with attribution permitted —
confirmed from each PDF's own copyright line before copying) so this
repository stays self-contained and citable from a bare clone, without
depending on a third-party host staying up.

- **Molino-Peña, E., Slabbinck, W., García, J. M., Ruiz-Cortés, A. and
  Esteves, B.** — ["Automated Validation of ODRL Policies for Usage Control
  and Data Spaces."](2026-09-automated-validation-of-odrl-policies.pdf)
  3rd NeXt-generation Data Governance workshop (NXDG 2026), co-located with
  the 22nd SEMANTiCS conference, 15 September 2026, Ghent, Belgium. Canonical
  source: [w3id.org/force/validator](https://w3id.org/force/validator).
  Defines a three-stage ODRL *validator* — normalization (into an atomic
  form), SHACL model-conformance checking, and N3-rule inconsistency
  detection — distinct from an *evaluator* like this engine. Drove the
  `dsp-odrl-adapter` N1/N4/N5 normalization work and its
  `detect_inconsistencies` function (tags `v0.24.0`–`v0.24.1`); see
  [Design rationale](../design-rationale.md)'s "Known adapter fragility"
  section for what shipped and what's still out of scope (the
  52-requirement SHACL conformance catalogue).

- **Slabbinck, W., Rojas Meléndez, J., Esteves, B., Verborgh, R. and
  Colpaert, P.** — ["May the FORCE be with you? A Framework for ODRL Rule
  Compliance through Evaluation."](2025-09-force-odrl-rule-compliance-through-evaluation.pdf)
  NeXt-generation Data Governance workshop (NXDG 2025), co-located with
  SEMANTiCS'25, 3–5 September 2025, Vienna, Austria. CEUR Workshop
  Proceedings, Vol-4064. Canonical source:
  [ceur-ws.org/Vol-4064/NXDG25-paper6.pdf](https://ceur-ws.org/Vol-4064/NXDG25-paper6.pdf).
  Introduces the **Compliance Report Model** — the `report:` vocabulary
  (`https://w3id.org/force/compliance-report#`) reused verbatim, not
  reinvented, by the sibling `ds-odrl-compliance-rdf` project for every
  test case's `dsc:expectedOutcome` tree. This engine's own
  `evaluate_request_detailed` (added `v0.22.0`, three correctness bugs
  found and fixed against this exact vocabulary's semantics across
  `v0.22.1`–`v0.23.2`) is this project's own independent implementation of
  the same report shape this paper defines — not derived from FORCE's own
  ODRL Evaluator/N3-rules implementation.

## Candidates found, not yet imported

Found via the same NXDG/OPAL/SolidLab research lineage (and one general
search) while adding the two papers above — listed here rather than
silently dropped, for a future pass to pick up:

- **Salas, J. O., Pareti, P., Aslam, A., Maidens, C. and Konstantinidis,
  G.** — "A Formally Grounded ODRL Evaluator: Implementation and
  Comparison." arXiv:2607.15987 (July 2026). Describes **OVAL**, one of
  the three external engines this project's own comparative
  `docs/compliance-methodology.md` benchmarks against — a first-party
  formal-semantics account of an engine already in this study's own
  numbers, not yet cited as its own paper.
- **Slabbinck, W., Rojas Meléndez, J., Esteves, B. and Verborgh, R.** —
  "Interoperable Interpretation and Evaluation of ODRL Policies." ESWC
  2025. Already cited in the ds42.org case study's own bibliography
  (`^odrl-evaluator-eswc2025`) — the paper behind the reference
  `SolidLabResearch/ODRL-Evaluator` implementation this study already
  compares against by name.
- Rodríguez-Doncel, V. and Roman, N. — "Towards Conformance in ODRL 3.0."
  OPAL 2025 (also already cited in the case study, `^odrl-conformance-2025`).
- "ODRL Policy Comparison Through Normalisation." arXiv:2603.12926 (2026) —
  directly adjacent to this engine's own N1/N4/N5 normalization work; not
  yet read in enough depth to say more.
- Mustafa, D. M. et al. — "What Does ODRL Mean? A Cross-Level..."
  arXiv:2606.24344 (2026).

None of these have been read closely enough yet to state what, if
anything, they'd change here — recorded so a future pass doesn't have to
rediscover the search.
