//! Wall-clock time without a time library: Unix milliseconds and UTC stamps.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix epoch milliseconds now (0 if the clock is before 1970).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The calendar date of a day count since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// `2026-10-25T04:22:00Z` for Unix milliseconds. UTC, so a log line means the same thing wherever it is read.
pub fn utc_stamp(ms: u64) -> String {
    let secs = i64::try_from(ms / 1000).unwrap_or(i64::MAX / 2);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_match_known_instants() {
        assert_eq!(utc_stamp(0), "1970-01-01T00:00:00Z");
        // 1.7e9 s is Tue 2023-11-14 22:13:20 UTC.
        assert_eq!(utc_stamp(1_700_000_000_000), "2023-11-14T22:13:20Z");
        // The leap day, and the day after it.
        assert_eq!(utc_stamp(1_709_164_800_000), "2024-02-29T00:00:00Z");
        assert_eq!(utc_stamp(1_709_251_199_999), "2024-02-29T23:59:59Z");
        assert_eq!(utc_stamp(1_709_251_200_000), "2024-03-01T00:00:00Z");
        // 2026-10-25 04:22:00 UTC.
        assert_eq!(utc_stamp(1_792_902_120_000), "2026-10-25T04:22:00Z");
    }

    #[test]
    fn now_is_a_recent_unix_time_in_milliseconds() {
        assert!(now_ms() > 1_700_000_000_000);
    }
}
