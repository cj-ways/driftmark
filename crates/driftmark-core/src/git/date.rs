//! Reading the date given to `--since`.

/// Reads a date as seconds since 1970-01-01 00:00 UTC, the way git stores
/// commit dates. Returns `None` for anything else.
///
/// Two forms are accepted:
///
/// - `2026-10-01`: midnight at the start of that day, in UTC.
/// - `2026-10-01T09:30:00+04:00` or `2026-10-01T05:30:00Z`: that moment, with
///   its offset from UTC.
///
/// A date alone means UTC, not the local time zone, because the engine reads
/// no clock and no time zone settings. Write the offset to mean local time.
/// Years run from 1970 to 9999.
#[must_use]
pub fn parse_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let day = parse_day(bytes.get(..10)?)?;
    let rest = &bytes[10..];
    let time = if rest.is_empty() {
        0
    } else {
        parse_time(rest)?
    };
    Some(day * 86_400 + time)
}

/// `YYYY-MM-DD` as days since 1970-01-01.
fn parse_day(b: &[u8]) -> Option<i64> {
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (year, month, day) = (number(&b[..4])?, number(&b[5..7])?, number(&b[8..])?);
    let month_ok = (1..=12).contains(&month);
    if year < 1970 || !month_ok || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_since_1970(year, month, day))
}

/// `THH:MM:SS` then `Z` or `+HH:MM` or `-HH:MM`, as seconds since midnight UTC.
fn parse_time(b: &[u8]) -> Option<i64> {
    if b.len() < 10 || b[0] != b'T' || b[3] != b':' || b[6] != b':' {
        return None;
    }
    let (hour, minute, second) = (number(&b[1..3])?, number(&b[4..6])?, number(&b[7..9])?);
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let offset = parse_offset(&b[9..])?;
    Some(hour * 3600 + minute * 60 + second - offset)
}

/// `Z`, `+HH:MM` or `-HH:MM` as seconds east of UTC.
fn parse_offset(b: &[u8]) -> Option<i64> {
    if b == b"Z" {
        return Some(0);
    }
    if b.len() != 6 || b[3] != b':' {
        return None;
    }
    let sign = match b[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes) = (number(&b[1..3])?, number(&b[4..])?);
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Digits only. `str::parse` would also take a sign, which no field allows.
fn number(digits: &[u8]) -> Option<i64> {
    digits.iter().try_fold(0, |value, &b| {
        b.is_ascii_digit().then(|| value * 10 + i64::from(b - b'0'))
    })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Days from 1970-01-01 to a date, by counting years from 1 March so the leap
/// day falls at the end. This is Howard Hinnant's `days_from_civil`, limited to
/// years from 1970, where every value it handles is positive.
fn days_since_1970(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_alone_are_midnight_utc() {
        let cases = [
            ("1970-01-01", 0),
            ("1970-01-02", 86_400),
            ("1970-03-01", 59 * 86_400),
            ("1972-02-29", 789 * 86_400),
            ("1972-03-01", 790 * 86_400),
            ("2000-01-01", 946_684_800),
            ("2000-02-29", 951_782_400),
            ("2000-03-01", 951_868_800),
            ("2000-12-31", 978_220_800),
            ("2026-10-18", 1_792_281_600),
            ("2100-03-01", 4_107_542_400),
            ("9999-12-31", 253_402_214_400),
        ];
        for (text, seconds) in cases {
            assert_eq!(parse_date(text), Some(seconds), "{text}");
        }
    }

    #[test]
    fn times_take_their_offset() {
        let midnight = 1_792_281_600;
        let cases = [
            ("2026-10-18T00:00:00Z", midnight),
            ("2026-10-18T09:30:15Z", midnight + 9 * 3600 + 30 * 60 + 15),
            ("2026-10-18T23:59:59Z", midnight + 86_399),
            ("2026-10-18T04:00:00+04:00", midnight),
            ("2026-10-18T00:00:00+23:59", midnight - 23 * 3600 - 59 * 60),
            ("2026-10-17T22:30:00-01:30", midnight),
            ("2026-10-18T00:00:00-00:00", midnight),
        ];
        for (text, seconds) in cases {
            assert_eq!(parse_date(text), Some(seconds), "{text}");
        }
    }

    #[test]
    fn every_month_has_its_length() {
        let lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for (index, length) in lengths.iter().enumerate() {
            let month = index + 1;
            assert!(
                parse_date(&format!("2025-{month:02}-{length:02}")).is_some(),
                "{month}"
            );
            let past = length + 1;
            assert_eq!(
                parse_date(&format!("2025-{month:02}-{past:02}")),
                None,
                "{month}"
            );
        }
        assert!(parse_date("2024-02-29").is_some());
        assert!(parse_date("2000-02-29").is_some());
        assert_eq!(parse_date("2100-02-29"), None);
        assert_eq!(parse_date("2023-02-29"), None);
    }

    #[test]
    fn anything_else_is_refused() {
        let bad = [
            "",
            "2026",
            "2026-10-1",
            "2026/10/18",
            "2026-10/18",
            "2026_10-18",
            "1969-12-31",
            "2026-00-10",
            "2026-13-10",
            "2026-10-00",
            "2026-10-32",
            "+026-10-18",
            "2026-+1-18",
            "2026-10-+8",
            "2026-1a-18",
            "2026-10-18 09:30:00Z",
            "2026-10-18T",
            "2026-10-18T09:30Z",
            "2026-10-18T09-30:00Z",
            "2026-10-18T09:30-00Z",
            "2026-10-18T24:00:00Z",
            "2026-10-18T23:60:00Z",
            "2026-10-18T23:59:60Z",
            "2026-10-18T+1:00:00Z",
            "2026-10-18T09:3a:00Z",
            "2026-10-18T09:30:0aZ",
            "2026-10-18T09:30:00",
            "2026-10-18T09:30:00z",
            "2026-10-18T09:30:00ZZ",
            "2026-10-18T09:30:00+04",
            "2026-10-18T09:30:00+0400",
            "2026-10-18T09:30:00+04:000",
            "2026-10-18T09:30:00*04:00",
            "2026-10-18T09:30:00+04-00",
            "2026-10-18T09:30:00+24:00",
            "2026-10-18T09:30:00+04:60",
            "2026-10-18T09:30:00+0a:00",
            "2026-10-18T09:30:00+04:0a",
            "2026-10-18Z",
            "2026-10-18x",
        ];
        for text in bad {
            assert_eq!(parse_date(text), None, "{text}");
        }
    }

    #[test]
    fn day_counting_matches_a_plain_count() {
        let mut expected = 0;
        for year in 1970..2030 {
            for month in 1..=12 {
                for day in 1..=days_in_month(year, month) {
                    assert_eq!(
                        days_since_1970(year, month, day),
                        expected,
                        "{year}-{month}-{day}"
                    );
                    expected += 1;
                }
            }
        }
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        for (year, leap) in [
            (2024, true),
            (2023, false),
            (2000, true),
            (1900, false),
            (2400, true),
        ] {
            assert_eq!(is_leap_year(year), leap, "{year}");
        }
    }
}
