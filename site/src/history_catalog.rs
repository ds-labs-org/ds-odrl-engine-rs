//! Pure, browser-free logic for the Release History page: parsing
//! `compliance/reports/release-history.json` (see
//! `release-history/src/render.rs`), validating it, and deriving the few
//! per-release quantities the page draws that are not literally in the
//! file.
//!
//! Deliberately **not** `#[cfg(target_arch = "wasm32")]`-gated, for the
//! same reason `coverage_catalog.rs` and `compliance_cases.rs` aren't:
//! `cargo test --workspace` is a native build, so a gated module's unit
//! tests would silently never compile, let alone run.
//!
//! **This page is the one that does not recompute what it shows, and that
//! is a deliberate, stated exception.** The Compliance Results and
//! Capability Audit pages both re-execute their whole corpus against
//! `engine.wasm` in the visitor's browser, and say so. This one cannot:
//! its subject *is* nineteen different historical `engine.wasm` binaries,
//! 3.9 MB of them, which would have to be shipped and instantiated to
//! reproduce a number that only changes when someone cuts a new tag. So
//! the artifact carries results, the page renders them, and both the page
//! and this module are explicit that these are build-time figures with a
//! recorded provenance (`engine_wasm_sha256` per release, so a reader can
//! rebuild the tag and check the binary) rather than a live proof.

use serde::Deserialize;

/// The artifact, copied to `dist/compliance-data/` by `index.html`'s own
/// `copy-file` directive — same target directory, and same
/// route-collision reason, as the three artifacts beside it.
pub const HISTORY_URL: &str = "compliance-data/release-history.json";

/// Must equal `release-history/src/render.rs`'s own `SCHEMA`. Checked
/// rather than assumed: `copy-file` assets are not content-hashed, so a
/// returning visitor can be served a browser-cached artifact of an older
/// shape, which must fail loudly instead of half-parsing into a dashboard
/// with plausible-looking holes in it.
///
/// Bumped `@2` -> `@3` alongside [`Release::row_status`]'s retirement in
/// favour of [`Release::full_compliance`] -- see
/// `release-history/src/render.rs`'s own `SCHEMA` doc comment.
pub const HISTORY_SCHEMA: &str = "ds-odrl-engine-rs/release-history@3";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogInfo {
  pub generated_by: String,
  pub spec: String,
  pub source_analysis: String,
  pub rows: usize,
  pub probes: usize,
  pub implemented: usize,
  pub partial: usize,
  pub not_implemented: usize,
  pub out_of_scope: usize,
  /// Rows `/full-compliance` judges at all -- the catalog's rows minus the
  /// ones documented `OutOfScope`. A static catalog property, identical
  /// for every release, same reasoning as the four fields above.
  pub full_compliance_rows_in_scope: usize,
  pub full_compliance_rows_excluded: usize,
  /// Distinct probes named by at least one in-scope row.
  pub full_compliance_probes_judged: usize,
  pub full_compliance_probes_in_catalog: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ComplianceTally {
  pub total: u64,
  pub passed: u64,
  pub failed: u64,
  pub skipped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CoverageTally {
  pub probes_total: usize,
  pub agreed: usize,
  pub disagreed: usize,
  pub errored: usize,
  pub verified: usize,
  pub contradicted: usize,
  pub inconclusive: usize,
  pub documented: usize,
  /// Probes this release's own deserializer refused outright. A few of
  /// these is real signal (a release with no `isAllOf` operator refuses
  /// exactly the `isAllOf` probes); all of them means the release is not
  /// addressable by this catalog at all, in which case the generator
  /// records no tally and a reason instead — see [`Release::coverage`].
  pub envelope_rejected: usize,
}

/// One release's own reading against **full ODRL 2.2 compliance**, not
/// against this study's documentation -- genuinely varying release to
/// release, unlike [`CatalogInfo`]'s static, catalog-wide distribution
/// above. See `release-history/src/render.rs`'s own `FullComplianceTally`
/// doc comment for how this is derived (`full_compliance.rs`'s
/// `compile_full_compliance_report`, replayed against that release's own
/// probe outcomes -- the same function the live `/full-compliance` page
/// calls in the browser).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FullComplianceTally {
  pub rows_meets: usize,
  pub rows_falls_short: usize,
  pub rows_structural_gap: usize,
  pub rows_undetermined: usize,
  pub probes_meets: usize,
  pub probes_falls_short: usize,
  pub probes_undetermined: usize,
}

impl FullComplianceTally {
  pub fn rows_total(&self) -> usize {
    self.rows_meets + self.rows_falls_short + self.rows_structural_gap + self.rows_undetermined
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ContradictedRow {
  pub id: String,
  pub category: String,
  pub term: String,
  pub documented_status: String,
  pub probe_id: String,
  pub mismatch: String,
  pub engine_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
  pub tag: String,
  pub date: String,
  pub commit: String,
  pub summary: String,
  pub engine_wasm_bytes: u64,
  pub engine_wasm_sha256: String,
  pub compliance: Option<ComplianceTally>,
  /// `None` when the current probe catalog could not address this release
  /// at all; `coverage_error` then says why, in the historical engine's
  /// own words.
  pub coverage: Option<CoverageTally>,
  pub coverage_error: Option<String>,
  pub contradicted_rows: Vec<ContradictedRow>,
  /// This release's own full-ODRL-2.2-compliance tally -- `None` exactly
  /// when `coverage` is `None`. See [`FullComplianceTally`].
  pub full_compliance: Option<FullComplianceTally>,
}

impl Release {
  /// The date without its time-of-day, for a table cell. Every tag in
  /// this repo's history was cut on one long day, so the full ISO-8601
  /// timestamp is what actually distinguishes them — this is the short
  /// form, and the page shows the full one as a tooltip rather than
  /// throwing it away.
  pub fn day(&self) -> &str {
    self.date.split('T').next().unwrap_or(&self.date)
  }

  /// `HH:MM`, which is what actually separates one tag from the next
  /// here.
  pub fn time_of_day(&self) -> &str {
    match self.date.split_once('T') {
      Some((_, rest)) => rest.get(0..5).unwrap_or(rest),
      None => "",
    }
  }

  pub fn short_commit(&self) -> &str {
    self.commit.get(0..7).unwrap_or(&self.commit)
  }

  /// Passing fixtures as a fraction of the suite that release ran.
  pub fn compliance_fraction(&self) -> Option<f64> {
    let compliance = self.compliance.as_ref()?;
    if compliance.total == 0 {
      return None;
    }
    Some(compliance.passed as f64 / compliance.total as f64)
  }

  /// In-scope rows meeting full ODRL 2.2 compliance, as a fraction of
  /// every row `/full-compliance` judges, 0.0..=1.0. `None` when this
  /// release has no tally at all -- which the page must render as "not
  /// addressable", never as 0%.
  pub fn meets_full_spec_row_fraction(&self) -> Option<f64> {
    let full_compliance = self.full_compliance.as_ref()?;
    let total = full_compliance.rows_total();
    if total == 0 {
      return None;
    }
    Some(full_compliance.rows_meets as f64 / total as f64)
  }

  /// Meets-full-spec probes as a fraction of every probe `/full-compliance`
  /// judges, 0.0..=1.0. This is a strictly finer-grained axis than
  /// [`meets_full_spec_row_fraction`], for the identical reason
  /// `/coverage`'s own row-vs-probe pair was: that one counts a *row* as
  /// meeting the spec only when every one of its own judged probes does,
  /// so one falls-short probe among several sinks the whole row; this one
  /// counts every individual probe. `None` when this release has no tally
  /// at all, same reasoning as [`meets_full_spec_row_fraction`] -- rendered
  /// as "not addressable", never 0%.
  pub fn meets_full_spec_probe_fraction(&self) -> Option<f64> {
    let full_compliance = self.full_compliance.as_ref()?;
    let total = full_compliance.probes_meets + full_compliance.probes_falls_short + full_compliance.probes_undetermined;
    if total == 0 {
      return None;
    }
    Some(full_compliance.probes_meets as f64 / total as f64)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HistoryFile {
  pub schema: String,
  pub generated_by: String,
  pub note: String,
  pub method: String,
  pub catalog: CatalogInfo,
  pub releases: Vec<Release>,
}

impl HistoryFile {
  /// Releases the catalog could not address at all — reported as their
  /// own group on the page rather than mixed into the table as rows of
  /// zeroes, because a zero there would read as "this release supported
  /// nothing", which is a claim this data does not support.
  pub fn unaddressable(&self) -> Vec<&Release> {
    self.releases.iter().filter(|release| release.coverage.is_none()).collect()
  }

  /// The most recent release, i.e. the one whose numbers the live
  /// Coverage page should reproduce. `None` only for an empty file,
  /// which `parse_release_history` already rejects.
  pub fn latest(&self) -> Option<&Release> {
    self.releases.last()
  }

  /// Every row id that any release contradicted, with how many releases
  /// contradicted it — the "what took longest to land" view, which no
  /// single release's own column shows.
  pub fn contradiction_counts(&self) -> Vec<(String, String, usize)> {
    let mut counts: Vec<(String, String, usize)> = Vec::new();
    for release in &self.releases {
      for row in &release.contradicted_rows {
        match counts.iter_mut().find(|(id, _, _)| id == &row.id) {
          Some(entry) => entry.2 += 1,
          None => counts.push((row.id.clone(), row.term.clone(), 1)),
        }
      }
    }
    counts.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    counts
  }
}

/// Parses and validates the fetched artifact. Every rejection below is an
/// error rather than a degraded render: a dashboard that half-parsed
/// would show a release history with silent holes in it, which is worse
/// than showing none.
pub fn parse_release_history(text: &str) -> Result<HistoryFile, String> {
  let file: HistoryFile =
    serde_json::from_str(text).map_err(|err| format!("{HISTORY_URL} did not match the expected shape: {err}"))?;

  if file.schema != HISTORY_SCHEMA {
    return Err(format!("{HISTORY_URL} declares schema `{}`, this page speaks `{HISTORY_SCHEMA}`", file.schema));
  }
  if file.releases.is_empty() {
    return Err(format!("{HISTORY_URL} carries no releases at all"));
  }

  for release in &file.releases {
    if release.tag.is_empty() {
      return Err(format!("{HISTORY_URL}: a release carries no tag"));
    }
    // Exactly one of the two must be present. Both would mean the
    // generator reported a tally it also said it could not compute;
    // neither would leave the page with nothing to say about why a
    // release has no numbers.
    match (release.coverage.is_some(), release.coverage_error.is_some()) {
      (true, true) => {
        return Err(format!("{HISTORY_URL}: release `{}` carries both a coverage tally and an error", release.tag))
      }
      (false, false) => {
        return Err(format!(
          "{HISTORY_URL}: release `{}` carries neither a coverage tally nor a reason it has none",
          release.tag
        ))
      }
      _ => {}
    }
    if let Some(coverage) = &release.coverage {
      let judged = coverage.agreed + coverage.disagreed + coverage.errored;
      if judged != coverage.probes_total {
        return Err(format!(
          "{HISTORY_URL}: release `{}` tallies {judged} probe outcomes but claims {} probes",
          release.tag, coverage.probes_total
        ));
      }
      if coverage.contradicted != release.contradicted_rows.len() {
        return Err(format!(
          "{HISTORY_URL}: release `{}` counts {} contradicted rows but lists {}",
          release.tag,
          coverage.contradicted,
          release.contradicted_rows.len()
        ));
      }
    }

    // Same "exactly one of the two" reasoning as `coverage`/`coverage_error`
    // above, and for the same purpose: a release that is addressable at all
    // must carry a full-compliance tally to draw the charts from, and one
    // that isn't must not carry a tally a reader could mistake for a real
    // judgment on a release the catalog cannot address.
    if release.coverage.is_some() != release.full_compliance.is_some() {
      return Err(format!(
        "{HISTORY_URL}: release `{}` carries coverage: {} but full_compliance: {} -- these must agree",
        release.tag,
        release.coverage.is_some(),
        release.full_compliance.is_some()
      ));
    }
    if let Some(full_compliance) = &release.full_compliance {
      if full_compliance.rows_total() != file.catalog.full_compliance_rows_in_scope {
        return Err(format!(
          "{HISTORY_URL}: release `{}` full_compliance rows sum to {} but the catalog carries {} in-scope rows",
          release.tag,
          full_compliance.rows_total(),
          file.catalog.full_compliance_rows_in_scope
        ));
      }
    }
  }

  Ok(file)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The real, committed artifact this page fetches at runtime — embedded
  /// here **only** for these tests (`#[cfg(test)]`), never in the wasm
  /// build, which fetches it instead.
  const COMMITTED: &str = include_str!("../../compliance/reports/release-history.json");

  fn committed() -> HistoryFile {
    parse_release_history(COMMITTED).expect("the committed release-history.json parses")
  }

  #[test]
  fn the_committed_artifact_parses_and_covers_the_whole_tag_range() {
    let file = committed();
    assert_eq!(file.schema, HISTORY_SCHEMA);
    assert!(file.releases.len() >= 25, "expected every tag, found {}", file.releases.len());
    assert_eq!(file.releases.first().map(|r| r.tag.as_str()), Some("v0.1.0"));
    assert_eq!(file.catalog.rows, 52);
    assert_eq!(file.catalog.probes, 136);
  }

  /// The one release whose numbers are independently checkable: the
  /// newest tag is what the *live* Capability Audit page runs in the
  /// browser, so its historical row must agree with the current catalog's
  /// own totals.
  /// If a future engine change makes the live page report contradictions
  /// the newest staged tag doesn't, this fails and says the dashboard is
  /// stale.
  #[test]
  fn the_newest_release_agrees_with_the_current_catalog() {
    let file = committed();
    let latest = file.latest().expect("a newest release");
    let coverage = latest.coverage.as_ref().expect("the newest release is addressable by its own catalog");
    assert_eq!(coverage.probes_total, file.catalog.probes);
    assert_eq!(coverage.verified + coverage.contradicted + coverage.inconclusive + coverage.documented, file.catalog.rows);
    assert_eq!(coverage.contradicted, 0, "the newest tag must not contradict the catalog generated from it");
  }

  #[test]
  fn every_addressable_release_carries_a_full_compliance_tally_summing_to_the_catalogs_in_scope_rows() {
    let file = committed();
    for release in &file.releases {
      match (&release.coverage, &release.full_compliance) {
        (Some(_), Some(full_compliance)) => {
          assert_eq!(
            full_compliance.rows_total(),
            file.catalog.full_compliance_rows_in_scope,
            "{}: full_compliance must sum to every in-scope row",
            release.tag
          );
        }
        (None, None) => {}
        _ => panic!("{}: coverage and full_compliance must agree on whether this release is addressable", release.tag),
      }
    }
  }

  /// `rows_structural_gap` names a permanent wire-contract absence
  /// (`party.collections`: no request can even pose the question), not a
  /// probe outcome -- so it is identical for every addressable release,
  /// the same invariant `/coverage`'s own catalog-wide `out_of_scope`
  /// count holds for a structurally analogous reason.
  #[test]
  fn structural_gap_count_is_identical_across_every_addressable_release() {
    let file = committed();
    let addressable: Vec<&FullComplianceTally> =
      file.releases.iter().filter_map(|r| r.full_compliance.as_ref()).collect();
    assert!(addressable.len() >= 2, "need at least two addressable releases to compare");
    let first_structural_gap = addressable[0].rows_structural_gap;
    for (release, full_compliance) in file.releases.iter().filter(|r| r.full_compliance.is_some()).zip(addressable.iter()) {
      assert_eq!(full_compliance.rows_structural_gap, first_structural_gap, "{}: structural gap count moved", release.tag);
    }
  }

  #[test]
  fn releases_are_in_ascending_version_order() {
    let file = committed();
    let key = |tag: &str| -> Vec<u64> {
      tag.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect()
    };
    for pair in file.releases.windows(2) {
      assert!(
        key(&pair[0].tag) < key(&pair[1].tag),
        "{} must sort before {}",
        pair[0].tag,
        pair[1].tag
      );
    }
  }

  #[test]
  fn an_unaddressable_release_reports_no_fraction_rather_than_zero() {
    let file = committed();
    let unaddressable = file.unaddressable();
    assert!(!unaddressable.is_empty(), "this history contains a real pre-v0.6.0 wire break");
    for release in unaddressable {
      assert_eq!(release.meets_full_spec_row_fraction(), None);
      assert_eq!(release.meets_full_spec_probe_fraction(), None);
      assert!(release.coverage_error.as_ref().is_some_and(|e| !e.is_empty()));
      // It must still carry its own historical compliance number: the
      // wire break stops the *coverage* replay, not the suite run that
      // release actually performed.
      assert!(release.compliance.is_some());
    }
  }

  #[test]
  fn meets_full_spec_probe_fraction_is_a_finer_grain_than_the_row_fraction() {
    let file = committed();
    let addressable: Vec<&Release> = file.releases.iter().filter(|r| r.coverage.is_some()).collect();
    assert!(addressable.len() >= 2, "need at least two addressable releases to compare");
    for release in addressable {
      let full_compliance = release.full_compliance.as_ref().unwrap();
      let probe_fraction =
        release.meets_full_spec_probe_fraction().unwrap_or_else(|| panic!("{}: expected Some", release.tag));
      let probe_total = full_compliance.probes_meets + full_compliance.probes_falls_short + full_compliance.probes_undetermined;
      // Computed directly from the raw tally, not re-derived through any
      // other method -- this is the one thing this test actually pins.
      assert_eq!(probe_fraction, full_compliance.probes_meets as f64 / probe_total as f64, "{}", release.tag);
      assert!((0.0..=1.0).contains(&probe_fraction), "{}: {probe_fraction} out of range", release.tag);
      // The real, load-bearing claim in this metric's own doc comment: a
      // row only counts as meeting full spec when every one of its judged
      // probes does, so the row-level share can never exceed the
      // probe-level share for the same release -- one falls-short probe
      // cannot cost less than one row. Not asserting strict inequality: a
      // release with zero shortfall anywhere makes the two exactly equal,
      // which is a real, valid case, not a bug.
      if let Some(row_fraction) = release.meets_full_spec_row_fraction() {
        assert!(
          row_fraction <= probe_fraction + 1e-9,
          "{}: row-meets {row_fraction} exceeded probe-meets {probe_fraction}",
          release.tag
        );
      }
    }
  }

  #[test]
  fn schema_mismatch_is_rejected_rather_than_half_parsed() {
    let swapped = COMMITTED.replace(HISTORY_SCHEMA, "ds-odrl-engine-rs/release-history@99");
    let err = parse_release_history(&swapped).expect_err("a foreign schema must be rejected");
    assert!(err.contains("declares schema"), "{err}");
  }

  #[test]
  fn a_tally_that_does_not_add_up_is_rejected() {
    let file: serde_json::Value = serde_json::from_str(COMMITTED).unwrap();
    let mut file = file;
    file["releases"][18]["coverage"]["agreed"] = serde_json::json!(1);
    let err = parse_release_history(&file.to_string()).expect_err("a broken tally must be rejected");
    assert!(err.contains("probe outcomes"), "{err}");
  }

  #[test]
  fn a_release_claiming_both_a_tally_and_an_error_is_rejected() {
    let mut file: serde_json::Value = serde_json::from_str(COMMITTED).unwrap();
    file["releases"][18]["coverage_error"] = serde_json::json!("something");
    let err = parse_release_history(&file.to_string()).expect_err("both must be rejected");
    assert!(err.contains("both a coverage tally and an error"), "{err}");
  }

  #[test]
  fn contradiction_counts_rank_the_longest_standing_gaps_first() {
    let counts = committed().contradiction_counts();
    assert!(!counts.is_empty());
    for pair in counts.windows(2) {
      assert!(pair[0].2 >= pair[1].2, "counts must be descending");
    }
  }

  #[test]
  fn dates_split_into_a_day_and_a_time() {
    let file = committed();
    let first = &file.releases[0];
    assert_eq!(first.day().len(), 10, "an ISO-8601 date is YYYY-MM-DD");
    assert_eq!(first.time_of_day().len(), 5, "HH:MM");
    assert_eq!(first.short_commit().len(), 7);
  }
}
