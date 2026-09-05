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

fn digits(s: &str, from: usize, to: usize) -> Option<i64> {
    s.get(from..to)?.parse().ok()
}

/// Parses `YYYY-MM-DDTHH:MM:SS(.fff+)?Z` into nanoseconds since the Unix
/// epoch. `None` for anything else (a non-UTC offset, a malformed string,
/// a non-numeric field) — a parse failure is a constraint *miss*, the same
/// posture `Constraint::evaluate` already takes for an absent claim.
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
}
