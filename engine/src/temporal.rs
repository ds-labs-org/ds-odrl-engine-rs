//! Minimal UTC `xsd:dateTime` parsing and ordering, added so the `lt`/
//! `lteq`/`gt`/`gteq` operators (Section 4.2's Default Profile, extended)
//! can compare two ISO-8601 `...Z` timestamps chronologically rather than
//! lexically (a naive string comparison is wrong across a
//! fractional-seconds boundary: `"...:00Z"` sorts *after* `"...:00.5Z"` as
//! strings, even though 00.000 is chronologically earlier than 00.500).
//!
//! Deliberately hand-rolled rather than a new dependency: every timestamp
//! this engine's own fixtures and the vendored compliance suite use is
//! `YYYY-MM-DDTHH:MM:SS(.fff+)?Z` (UTC, no offset), so a fixed-width parser
//! plus the standard days-since-epoch civil calendar algorithm
//! (Howard Hinnant's `days_from_civil`) is enough, and keeps the `wasm32`
//! build free of a chrono-sized dependency.

/// Days since 1970-01-01 for a proleptic-Gregorian civil date. Standard
/// algorithm (Hinnant, "chrono-Compatible Low-Level Date Algorithms").
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The strictly-digits field parser. `str::parse::<i64>` alone is not
/// enough: it accepts a leading `+`/`-`, so `"+024"` or `"-1"` would slip
/// through a fixed-width field and silently misparse (e.g.
/// `"+024-01-01T..."` as year 24) instead of being rejected.
fn digits(s: &str, from: usize, to: usize) -> Option<i64> {
    let field = s.get(from..to)?;
    if !field.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// Parses `YYYY-MM-DDTHH:MM:SS(.fff+)?Z` into nanoseconds since the Unix
/// epoch. `None` for anything else (a non-UTC offset, a malformed string,
/// a non-numeric or out-of-range field — `2023-02-29`, month `13`, minute
/// `99`, ... are rejected, not rolled over into the next valid instant) —
/// a parse failure is a constraint *miss*, the same posture
/// `Constraint::evaluate` already takes for an absent claim. The one
/// deliberate leniency is XSD's own: `24:00:00` (with a zero fraction) is
/// a valid `xsd:dateTime` lexical form meaning the *following* midnight,
/// and the day-rollover arithmetic below already produces exactly that.
pub fn parse_utc_datetime_nanos(s: &str) -> Option<i128> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 || *bytes.last()? != b'Z' {
        return None;
    }
    let expect = |i: usize, c: u8| bytes.get(i) == Some(&c);
    if !(expect(4, b'-') && expect(7, b'-') && expect(10, b'T') && expect(13, b':') && expect(16, b':')) {
        return None;
    }

    let year = digits(s, 0, 4)?;
    let month = digits(s, 5, 7)?;
    let day = digits(s, 8, 10)?;
    let hour = digits(s, 11, 13)?;
    let minute = digits(s, 14, 16)?;
    let second = digits(s, 17, 19)?;

    let nanos = match bytes.get(19) {
        Some(b'Z') if s.len() == 20 => 0,
        Some(b'.') => {
            let frac = &s[20..s.len() - 1];
            if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut padded = frac.to_string();
            padded.truncate(9);
            while padded.len() < 9 {
                padded.push('0');
            }
            padded.parse::<i64>().ok()?
        }
        _ => return None,
    };

    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return None;
    }
    let ordinary_time = hour <= 23 && minute <= 59 && second <= 59;
    let xsd_end_of_day = hour == 24 && minute == 0 && second == 0 && nanos == 0;
    if !(ordinary_time || xsd_end_of_day) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(seconds as i128 * 1_000_000_000 + nanos as i128)
}

/// Parses `YYYY-MM-DDTHH:MM:SS(.fff+)?(+|-)HH:MM` — an `xsd:dateTime` with
/// an explicit numeric UTC offset rather than the literal `Z` designator —
/// into nanoseconds since the Unix epoch, converting to the equivalent UTC
/// instant (`UTC = local_time - offset`, XSD's own rule: `+02:00` means
/// local time is two hours *ahead* of UTC). A `Z`-suffixed value is not
/// this function's concern — `parse_utc_datetime_nanos` already handles
/// it, faster and unchanged — so this only ever matches the numeric-offset
/// form. Delegates the local-time-plus-calendar validation to
/// `parse_utc_datetime_nanos` itself (by substituting a literal `Z` for
/// the offset and re-parsing), so every existing field-width and
/// leading-sign guard there — including the fixed-width year field
/// rejecting a stray `+`/`-` — applies here too, unweakened.
fn parse_offset_datetime_nanos(s: &str) -> Option<i128> {
    let bytes = s.as_bytes();
    // Shortest valid form: "YYYY-MM-DDTHH:MM:SS+HH:MM" = 25 bytes.
    if bytes.len() < 25 {
        return None;
    }
    let offset_at = bytes.len() - 6;
    let sign: i64 = match bytes[offset_at] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    if bytes[bytes.len() - 3] != b':' {
        return None;
    }
    let offset_hour = digits(s, offset_at + 1, bytes.len() - 3)?;
    let offset_minute = digits(s, bytes.len() - 2, bytes.len())?;
    // XSD caps a legal timezone offset at +/-14:00.
    if offset_hour > 14 || offset_minute > 59 || (offset_hour == 14 && offset_minute != 0) {
        return None;
    }

    let local_part = s.get(..offset_at)?;
    let synthetic_utc = format!("{local_part}Z");
    let local_nanos = parse_utc_datetime_nanos(&synthetic_utc)?;
    let offset_nanos = sign as i128 * (offset_hour * 3_600 + offset_minute * 60) as i128 * 1_000_000_000;
    Some(local_nanos - offset_nanos)
}

/// Parses a bare `xsd:date` lexical form `YYYY-MM-DD` — no time component,
/// and, deliberately out of scope for this widening, none of `xsd:date`'s
/// own optional `zzzzzz` timezone suffix either — into nanoseconds since
/// the Unix epoch, treating the value as **midnight UTC of that calendar
/// date**. This is a comparison-purposes convention this module chooses,
/// not one XSD itself asserts: `xsd:date` denotes a whole day (an
/// interval), not an instant, so anchoring it to its first instant for
/// ordering against a `dateTime` is a deliberate simplification (the same
/// choice SPARQL's own date/dateTime comparison semantics make), not a
/// literal reading of the spec.
fn parse_date_only_nanos(s: &str) -> Option<i128> {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return None;
    }
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = digits(s, 0, 4)?;
    let month = digits(s, 5, 7)?;
    let day = digits(s, 8, 10)?;
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days as i128 * 86_400 * 1_000_000_000)
}

/// Consumes an optional `<digits>UNIT` (or, when `allow_fraction`,
/// `<digits>.<digits>UNIT`) prefix naming `unit` from the front of
/// `*cursor`, worth `nanos_per_unit` nanoseconds per whole unit. Three
/// outcomes: `Some(Some(nanos))` — present, matched, `*cursor` advanced
/// past it; `Some(None)` — this component is simply absent here (the
/// digit run, if any, is not immediately followed by `unit`), `*cursor`
/// untouched, so the caller tries the next unit in sequence; `None` — the
/// leading digits (or the fractional part, when present) are themselves
/// malformed. This function alone cannot detect ordering mistakes like
/// `"1M1Y"` (`M` before `Y`) — that falls out of `parse_xsd_duration_nanos`
/// itself later finding unconsumed characters once every unit in the
/// grammar's fixed order has had its turn.
fn eat_duration_component(cursor: &mut &str, unit: u8, nanos_per_unit: i128, allow_fraction: bool) -> Option<Option<i128>> {
    let bytes = cursor.as_bytes();
    let mut int_end = 0;
    while int_end < bytes.len() && bytes[int_end].is_ascii_digit() {
        int_end += 1;
    }
    if int_end == 0 {
        // No leading digits at all: this component is not here, not
        // malformed -- e.g. cursor is "8H" while looking for `Y`.
        return Some(None);
    }

    let mut unit_at = int_end;
    let mut frac_start = None;
    if allow_fraction && bytes.get(unit_at) == Some(&b'.') {
        let start = unit_at + 1;
        let mut frac_end = start;
        while frac_end < bytes.len() && bytes[frac_end].is_ascii_digit() {
            frac_end += 1;
        }
        if frac_end == start {
            return None; // a bare "." with no digits after it
        }
        frac_start = Some(start);
        unit_at = frac_end;
    }

    if bytes.get(unit_at) != Some(&unit) {
        // Digits are here, but they belong to a different unit letter
        // (or none at all) -- this component is absent, not malformed.
        return Some(None);
    }

    let int_part: i128 = cursor[..int_end].parse().ok()?;
    let mut nanos = int_part.checked_mul(nanos_per_unit)?;
    if let Some(start) = frac_start {
        // Only ever reached for seconds, where nanos_per_unit is exactly
        // 1_000_000_000 -- pad/truncate the fraction to nanosecond
        // precision the same way parse_utc_datetime_nanos's own fractional
        // field already does.
        let mut padded = cursor[start..unit_at].to_string();
        padded.truncate(9);
        while padded.len() < 9 {
            padded.push('0');
        }
        let frac_nanos: i128 = padded.parse().ok()?;
        nanos += frac_nanos;
    }

    *cursor = &cursor[unit_at + 1..];
    Some(Some(nanos))
}

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const NANOS_PER_MINUTE: i128 = 60 * NANOS_PER_SECOND;
const NANOS_PER_HOUR: i128 = 60 * NANOS_PER_MINUTE;
const NANOS_PER_DAY: i128 = 24 * NANOS_PER_HOUR;

/// Parses an ISO-8601 / `xsd:duration` lexical form
/// `[-]P[nY][nM][nD][T[nH][nM][nS]]` — the type ODRL 2.2 Vocabulary
/// Section 4.5 gives `odrl:delayPeriod`, `odrl:elapsedTime`,
/// `odrl:meteredTime` and `odrl:timeInterval` — into a **total, signed
/// magnitude in nanoseconds**, for `constraint.rs::ordering_matches`'
/// third fallback. At least one component must be present (`"P"` and
/// `"PT"` alone are both rejected), every present component's unit letter
/// must appear in the fixed `Y`, `M`, `D`, `T`, `H`, `M`, `S` order with no
/// repeats, and only the seconds component may carry a decimal fraction
/// (`"PT1.5S"`) — XSD's own duration grammar reserves fractional literals
/// to the smallest, right-most field.
///
/// **This is not `xsd:duration`'s own comparison semantics.** XSD defines
/// duration ordering as a *partial* order specifically because a calendar
/// `Y` or `M` component has no fixed length (February is not always 28
/// days) — some pairs of durations are, by XSD's own definition,
/// genuinely incomparable. This function instead converts `Y` to a fixed
/// 365 days and `M` to a fixed 30 days, so every two durations this
/// engine parses always compare as a *total* order. That is wrong for the
/// small minority of `Y`/`M`-bearing pairs XSD itself calls
/// indeterminate, and is a deliberate, comparison-purposes convention
/// this module chooses — the same posture `parse_date_only_nanos` above
/// already takes for anchoring a bare `xsd:date` to midnight UTC — rather
/// than a literal reading of the spec. It is exactly right for every
/// duration this engine's own four leftOperand terms are actually used
/// for in practice: `delayPeriod`, `elapsedTime`, `meteredTime` and
/// `timeInterval` are overwhelmingly expressed in `D`/`H`/`M`/`S`, not
/// calendar years or months.
pub fn parse_xsd_duration_nanos(s: &str) -> Option<i128> {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1i128, r),
        None => (1i128, s),
    };
    let rest = rest.strip_prefix('P')?;
    if rest.is_empty() {
        return None;
    }

    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };

    let mut total: i128 = 0;
    let mut any_component = false;

    let mut cursor = date_part;
    for (unit, nanos_per_unit) in [(b'Y', 365 * NANOS_PER_DAY), (b'M', 30 * NANOS_PER_DAY), (b'D', NANOS_PER_DAY)] {
        if let Some(nanos) = eat_duration_component(&mut cursor, unit, nanos_per_unit, false)? {
            total = total.checked_add(nanos)?;
            any_component = true;
        }
    }
    if !cursor.is_empty() {
        return None; // leftover: wrong order, an unknown unit, or a repeat
    }

    if let Some(t) = time_part {
        if t.is_empty() {
            return None; // "T" present with nothing following it
        }
        let mut cursor = t;
        for (unit, nanos_per_unit, allow_fraction) in
            [(b'H', NANOS_PER_HOUR, false), (b'M', NANOS_PER_MINUTE, false), (b'S', NANOS_PER_SECOND, true)]
        {
            if let Some(nanos) = eat_duration_component(&mut cursor, unit, nanos_per_unit, allow_fraction)? {
                total = total.checked_add(nanos)?;
                any_component = true;
            }
        }
        if !cursor.is_empty() {
            return None;
        }
    }

    if !any_component {
        return None; // "P" or "PT" alone name no actual duration
    }

    Some(sign * total)
}

/// The widened temporal parse the `lt`/`lteq`/`gt`/`gteq` operators
/// (`constraint.rs`) actually use: tries, in order, the original strict
/// UTC `...Z` form (`parse_utc_datetime_nanos`, entirely unchanged — every
/// existing caller and test of that function is untouched by this
/// widening), then a numeric-UTC-offset `dateTime`
/// (`parse_offset_datetime_nanos`), then a bare `xsd:date`
/// (`parse_date_only_nanos`, midnight UTC — see its own doc comment).
/// `None` if none of the three recognize `s` — a miss, not an error, the
/// same posture `parse_utc_datetime_nanos` already documents.
pub fn parse_xsd_temporal_nanos(s: &str) -> Option<i128> {
    parse_utc_datetime_nanos(s)
        .or_else(|| parse_offset_datetime_nanos(s))
        .or_else(|| parse_date_only_nanos(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_timestamp_without_fractional_seconds() {
        assert_eq!(
            parse_utc_datetime_nanos("2024-01-01T00:00:00Z"),
            parse_utc_datetime_nanos("2024-01-01T00:00:00.000Z")
        );
    }

    #[test]
    fn orders_chronologically_across_the_fractional_seconds_boundary() {
        // The exact case naive string comparison gets wrong.
        let no_fraction = parse_utc_datetime_nanos("2024-01-01T00:00:00Z").unwrap();
        let half_second = parse_utc_datetime_nanos("2024-01-01T00:00:00.500Z").unwrap();
        assert!(no_fraction < half_second);
    }

    #[test]
    fn orders_the_fixture_timestamps_this_compliance_suite_actually_uses() {
        let past = parse_utc_datetime_nanos("2017-02-12T11:20:10.999Z").unwrap();
        let jan_start = parse_utc_datetime_nanos("2024-01-01T00:00:00.000Z").unwrap();
        let now = parse_utc_datetime_nanos("2024-02-12T11:20:10.999Z").unwrap();
        let dec_end = parse_utc_datetime_nanos("2024-12-31T23:59:59.000Z").unwrap();
        let future = parse_utc_datetime_nanos("2025-02-12T11:20:10.999Z").unwrap();
        assert!(past < jan_start);
        assert!(jan_start < now);
        assert!(now < dec_end);
        assert!(dec_end < future);
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse_utc_datetime_nanos("not-a-date"), None);
        assert_eq!(parse_utc_datetime_nanos("2024-01-01T00:00:00+01:00"), None);
        assert_eq!(parse_utc_datetime_nanos(""), None);
    }

    #[test]
    fn rejects_wrong_separators_and_lowercase_designators() {
        assert_eq!(parse_utc_datetime_nanos("2024-01-01t00:00:00Z"), None);
        assert_eq!(parse_utc_datetime_nanos("2024-01-01T00:00:00z"), None);
        assert_eq!(parse_utc_datetime_nanos("2024-01-01 00:00:00Z"), None);
    }

    #[test]
    fn rejects_out_of_range_calendar_and_clock_fields() {
        // Each of these previously *misparsed* by rolling over into the
        // next valid instant (2023-02-29 became March 1st, month 13 became
        // January of the following year, June 31st became July 1st, ...) —
        // a silent wrong answer, not the documented miss.
        for s in [
            "2023-02-29T00:00:00Z", // 2023 is not a leap year
            "2024-13-01T00:00:00Z",
            "2024-00-10T00:00:00Z",
            "2024-06-31T00:00:00Z", // June has 30 days
            "2024-01-32T00:00:00Z",
            "2024-01-00T00:00:00Z",
            "2024-01-01T25:00:00Z",
            "2024-01-01T00:60:00Z",
            "2024-01-01T00:00:61Z",
            "2024-01-01T24:00:01Z", // 24: only as exactly 24:00:00
            "2024-01-01T24:00:00.500Z",
        ] {
            assert_eq!(parse_utc_datetime_nanos(s), None, "{s} must be rejected, not rolled over");
        }
        // ...while the genuine leap day stays accepted.
        assert!(parse_utc_datetime_nanos("2024-02-29T00:00:00Z").is_some());
    }

    #[test]
    fn rejects_signed_numeric_fields() {
        // `str::parse::<i64>` accepts a leading sign; the field parser
        // must not, or "+024" silently becomes year 24.
        assert_eq!(parse_utc_datetime_nanos("2024--1-01T00:00:00Z"), None);
        assert_eq!(parse_utc_datetime_nanos("+024-01-01T00:00:00Z"), None);
    }

    #[test]
    fn agrees_with_known_unix_epoch_values() {
        // Cross-checked against an independent implementation (Python's
        // `datetime`), pinning the civil-calendar arithmetic: epoch
        // itself, a leap day, both century rules (2000 divisible by 400
        // is a leap year, 1900 divisible by 100 is not), and both ends of
        // the four-digit-year range.
        let secs = |s: &str| parse_utc_datetime_nanos(s).unwrap() / 1_000_000_000;
        assert_eq!(secs("1970-01-01T00:00:00Z"), 0);
        assert_eq!(secs("2024-02-29T00:00:00Z"), 1_709_164_800);
        assert_eq!(secs("2000-02-29T00:00:00Z"), 951_782_400);
        assert_eq!(secs("1900-03-01T00:00:00Z"), -2_203_891_200);
        assert_eq!(secs("0001-01-01T00:00:00Z"), -62_135_596_800);
        assert_eq!(secs("9999-12-31T23:59:59Z"), 253_402_300_799);
    }

    #[test]
    fn hour_24_is_the_following_midnight_per_xsd() {
        assert_eq!(
            parse_utc_datetime_nanos("2024-01-01T24:00:00Z"),
            parse_utc_datetime_nanos("2024-01-02T00:00:00Z")
        );
    }

    #[test]
    fn truncates_fractional_digits_beyond_nanoseconds() {
        assert_eq!(
            parse_utc_datetime_nanos("2024-01-01T00:00:00.1234567891Z"),
            parse_utc_datetime_nanos("2024-01-01T00:00:00.123456789Z")
        );
    }

    // -- parse_xsd_temporal_nanos: numeric-offset dateTime ---------------

    #[test]
    fn offset_datetime_matching_the_zulu_instant_parses_to_the_same_nanos_as_z() {
        assert_eq!(
            parse_xsd_temporal_nanos("2024-01-01T02:00:00+02:00"),
            parse_xsd_temporal_nanos("2024-01-01T00:00:00Z")
        );
    }

    #[test]
    fn negative_offset_datetime_matching_the_zulu_instant_parses_to_the_same_nanos_as_z() {
        assert_eq!(
            parse_xsd_temporal_nanos("2023-12-31T19:00:00-05:00"),
            parse_xsd_temporal_nanos("2024-01-01T00:00:00Z")
        );
    }

    #[test]
    fn offset_datetime_with_fractional_seconds_converts_correctly() {
        assert_eq!(
            parse_xsd_temporal_nanos("2024-01-01T02:30:00.500+02:30"),
            parse_xsd_temporal_nanos("2024-01-01T00:00:00.500Z")
        );
    }

    #[test]
    fn positive_offset_datetime_crosses_back_over_a_utc_day_boundary() {
        // 01:00 local at +05:00 is still the *previous* UTC day.
        let crossed = parse_xsd_temporal_nanos("2024-01-01T01:00:00+05:00").unwrap();
        let expected = parse_xsd_temporal_nanos("2023-12-31T20:00:00Z").unwrap();
        assert_eq!(crossed, expected);
    }

    #[test]
    fn negative_offset_datetime_crosses_forward_over_a_utc_day_boundary() {
        // 23:00 local at -05:00 is already the *next* UTC day.
        let crossed = parse_xsd_temporal_nanos("2024-01-01T23:00:00-05:00").unwrap();
        let expected = parse_xsd_temporal_nanos("2024-01-02T04:00:00Z").unwrap();
        assert_eq!(crossed, expected);
    }

    #[test]
    fn half_hour_offset_converts_correctly() {
        // India Standard Time, +05:30.
        assert_eq!(
            parse_xsd_temporal_nanos("2024-01-01T05:30:00+05:30"),
            parse_xsd_temporal_nanos("2024-01-01T00:00:00Z")
        );
    }

    #[test]
    fn rejects_an_offset_datetime_whose_year_field_carries_a_stray_sign() {
        // The exact same fixed-width-year bug `rejects_signed_numeric_fields`
        // guards for the `Z` form must not reappear via the offset path,
        // which reuses `parse_utc_datetime_nanos` internally for this.
        assert_eq!(parse_xsd_temporal_nanos("+024-01-01T00:00:00+02:00"), None);
    }

    #[test]
    fn rejects_an_offset_datetime_with_an_out_of_range_offset_field() {
        for s in [
            "2024-01-01T00:00:00+15:00", // hour exceeds XSD's +/-14:00 cap
            "2024-01-01T00:00:00+14:01", // 14 hours only valid at :00 minutes
            "2024-01-01T00:00:00+02:60", // minute out of range
            "2024-01-01T00:00:00+0200",  // missing the ':' separator
        ] {
            assert_eq!(parse_xsd_temporal_nanos(s), None, "{s} must be rejected");
        }
    }

    #[test]
    fn does_not_double_parse_a_z_suffixed_value_through_the_offset_path() {
        // A `Z`-suffixed value must still resolve via parse_utc_datetime_nanos
        // itself, byte-identically to before this widening existed.
        assert_eq!(
            parse_xsd_temporal_nanos("2024-01-01T00:00:00Z"),
            parse_utc_datetime_nanos("2024-01-01T00:00:00Z")
        );
    }

    // -- parse_xsd_temporal_nanos: bare xsd:date --------------------------

    #[test]
    fn bare_date_parses_as_midnight_utc_of_that_calendar_date() {
        assert_eq!(
            parse_xsd_temporal_nanos("2024-01-01"),
            parse_xsd_temporal_nanos("2024-01-01T00:00:00Z")
        );
    }

    #[test]
    fn two_bare_dates_order_chronologically_against_each_other() {
        let earlier = parse_xsd_temporal_nanos("2024-01-01").unwrap();
        let later = parse_xsd_temporal_nanos("2024-06-15").unwrap();
        assert!(earlier < later);
    }

    #[test]
    fn a_bare_date_orders_correctly_against_a_full_datetime_on_the_same_day() {
        // Midnight of the date is earlier than any later instant that day.
        let date_only = parse_xsd_temporal_nanos("2024-01-01").unwrap();
        let same_day_later = parse_xsd_temporal_nanos("2024-01-01T11:20:10.999Z").unwrap();
        assert!(date_only < same_day_later);
    }

    #[test]
    fn rejects_a_bare_date_whose_year_field_carries_a_stray_sign() {
        // Same fixed-width-year guard as the dateTime parser, exercised on
        // the shorter date-only path.
        assert_eq!(parse_xsd_temporal_nanos("+024-01-01"), None);
        assert_eq!(parse_xsd_temporal_nanos("-024-01-01"), None);
    }

    #[test]
    fn rejects_an_out_of_range_bare_date() {
        for s in ["2023-02-29", "2024-13-01", "2024-01-32", "2024-00-10"] {
            assert_eq!(parse_xsd_temporal_nanos(s), None, "{s} must be rejected");
        }
        assert!(parse_xsd_temporal_nanos("2024-02-29").is_some());
    }

    #[test]
    fn rejects_a_bare_date_with_a_trailing_time_component_or_timezone() {
        // Deliberately out of scope: xsd:date's own optional zzzzzz
        // timezone suffix is not accepted by this widening (see
        // parse_date_only_nanos's doc comment).
        for s in ["2024-01-01Z", "2024-01-01T00:00:00", "2024-01-01+02:00"] {
            assert_eq!(parse_xsd_temporal_nanos(s), None, "{s} must be rejected");
        }
    }

    #[test]
    fn rejects_garbage_and_empty_input_for_the_widened_parser_too() {
        assert_eq!(parse_xsd_temporal_nanos("not-a-date"), None);
        assert_eq!(parse_xsd_temporal_nanos(""), None);
        assert_eq!(parse_xsd_temporal_nanos("42"), None);
    }

    // -- parse_xsd_duration_nanos: xsd:duration ---------------------------

    #[test]
    fn orders_two_simple_hour_durations_correctly() {
        let eight = parse_xsd_duration_nanos("PT8H").unwrap();
        let ten = parse_xsd_duration_nanos("PT10H").unwrap();
        assert!(eight < ten);
    }

    #[test]
    fn a_one_day_duration_equals_twenty_four_hours() {
        assert_eq!(parse_xsd_duration_nanos("P1D"), parse_xsd_duration_nanos("PT24H"));
    }

    #[test]
    fn combines_date_and_time_components_in_one_literal() {
        // 1 day, 2 hours, 3 minutes, 4 seconds.
        let combined = parse_xsd_duration_nanos("P1DT2H3M4S").unwrap();
        let expected = 86_400 + 2 * 3_600 + 3 * 60 + 4;
        assert_eq!(combined, expected as i128 * 1_000_000_000);
    }

    #[test]
    fn parses_fractional_seconds() {
        assert_eq!(
            parse_xsd_duration_nanos("PT1.5S"),
            Some(1_500_000_000)
        );
    }

    #[test]
    fn a_leading_minus_sign_negates_the_whole_duration() {
        let positive = parse_xsd_duration_nanos("P1D").unwrap();
        let negative = parse_xsd_duration_nanos("-P1D").unwrap();
        assert_eq!(negative, -positive);
    }

    #[test]
    fn zero_duration_parses_to_zero_not_none() {
        assert_eq!(parse_xsd_duration_nanos("PT0S"), Some(0));
    }

    #[test]
    fn rejects_malformed_duration_literals() {
        for s in [
            "",
            "P",           // no components at all
            "PT",          // T with nothing following
            "1Y",          // missing the leading P
            "P1D1Y",       // wrong order (D before Y)
            "P1Q",         // unknown unit
            "+P1D",        // XSD only allows a leading '-', never '+'
            "P1D2D",       // repeated unit
        ] {
            assert_eq!(parse_xsd_duration_nanos(s), None, "{s} must be rejected");
        }
    }

    #[test]
    fn a_plain_number_is_not_mistaken_for_a_duration() {
        assert_eq!(parse_xsd_duration_nanos("42"), None);
        assert_eq!(parse_xsd_duration_nanos("2024-01-01T00:00:00Z"), None);
    }
}
