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
}
