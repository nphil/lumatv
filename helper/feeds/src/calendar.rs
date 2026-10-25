//! Proleptic Gregorian date arithmetic, hand-written because the crate has no date dependency.

/// Whether `year` is a leap year.
pub(crate) fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` (1..=12) of `year`; 0 for a month outside that range.
pub(crate) fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 of a valid calendar date (Howard Hinnant's `days_from_civil`).
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(month) + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Weekday of a day count from [`days_from_civil`]: 0 = Sunday .. 6 = Saturday (1970-01-01 was a Thursday).
pub(crate) fn weekday(days: i64) -> usize {
    (days + 4).rem_euclid(7) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        // 2026-10-25 00:00:00 UTC is 1_792_886_400 s.
        assert_eq!(days_from_civil(2026, 10, 25) * 86_400, 1_792_886_400);
    }

    #[test]
    fn weekdays() {
        assert_eq!(weekday(days_from_civil(1970, 1, 1)), 4);
        assert_eq!(weekday(days_from_civil(2000, 1, 1)), 6);
        assert_eq!(weekday(days_from_civil(2026, 10, 25)), 0);
    }

    #[test]
    fn leap_rules() {
        assert!(is_leap(2000));
        assert!(is_leap(2028));
        assert!(!is_leap(2100));
        assert!(!is_leap(2026));
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 13), 0);
    }
}
