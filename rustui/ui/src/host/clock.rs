//! The wall clock of the person's room: epoch milliseconds as the local civil time (year, month, day, weekday, hour, minute, second). The
//! offset is the system's (`localtime_r`, the TV's time zone as LG's settings put it in `/etc/localtime`); the calendar arithmetic is plain,
//! so a model that is handed an offset runs under a test with no zone at all. English names only (plan section 2.9: the locale table is the
//! extension point).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    /// 0 is Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millis: u16,
}

/// Days since 1970-01-01 as (year, month, day) in the proleptic Gregorian calendar (Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

/// Days since 1970-01-01 of a calendar date (the inverse of [`civil_from_days`]).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The epoch milliseconds of a local civil time `offset_s` seconds east of UTC.
pub fn epoch_of(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8, offset_s: i32) -> u64 {
    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    let secs = days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second) - i64::from(offset_s);
    (secs * 1000).max(0) as u64
}

/// `2026-10-29T10:08:35` (or with a space) as epoch milliseconds in the system's zone: what the harness's `--clock` takes.
pub fn parse_local(text: &str) -> Option<u64> {
    let (date, time) = text.trim().split_once(['T', ' '])?;
    let mut d = date.split('-').map(|p| p.parse::<i32>().ok());
    let (year, month, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<u8>().ok());
    let (hour, minute) = (t.next()??, t.next()??);
    let second = t.next().flatten().unwrap_or(0);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let guess = epoch_of(year, month as u8, day as u8, hour, minute, second, 0);
    let offset = system_offset_s(guess);
    Some(epoch_of(year, month as u8, day as u8, hour, minute, second, offset))
}

/// The civil time at `epoch_ms` in a zone `offset_s` seconds east of UTC.
pub fn civil_at(epoch_ms: u64, offset_s: i32) -> Civil {
    let local_ms = epoch_ms as i64 + i64::from(offset_s) * 1000;
    let secs = local_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let in_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Civil {
        year,
        month,
        day,
        weekday: (days + 4).rem_euclid(7) as u8,
        hour: (in_day / 3600) as u8,
        minute: (in_day % 3600 / 60) as u8,
        second: (in_day % 60) as u8,
        millis: local_ms.rem_euclid(1000) as u16,
    }
}

/// The system's offset from UTC at `epoch_ms`, in seconds east (the TV's zone, DST as it is then).
pub fn system_offset_s(epoch_ms: u64) -> i32 {
    let t = (epoch_ms / 1000) as libc::time_t;
    // SAFETY: `tm` is plain data that `localtime_r` fills; `t` outlives the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i32
    }
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

impl Civil {
    /// "Thursday, October 29".
    pub fn date_line(&self) -> String {
        format!("{}, {} {}", WEEKDAYS[usize::from(self.weekday) % 7], MONTHS[usize::from(self.month.clamp(1, 12)) - 1], self.day)
    }

    /// Seconds into the day, with the fraction: what the hands are drawn from.
    pub fn day_seconds(&self) -> f64 {
        f64::from(self.hour) * 3600.0 + f64::from(self.minute) * 60.0 + f64::from(self.second) + f64::from(self.millis) / 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_milliseconds_become_the_calendar_in_any_zone() {
        // 2026-10-29 02:35:12.250 UTC was a Thursday
        let ms = 1_793_241_312_250u64;
        let utc = civil_at(ms, 0);
        assert_eq!((utc.year, utc.month, utc.day, utc.weekday), (2026, 10, 29, 4));
        assert_eq!((utc.hour, utc.minute, utc.second, utc.millis), (2, 35, 12, 250));
        let eastern = civil_at(ms, -4 * 3600);
        assert_eq!((eastern.day, eastern.hour), (28, 22), "four hours west: the evening before");
        assert_eq!(eastern.weekday, 3);
        assert_eq!(eastern.date_line(), "Wednesday, October 28");
        assert_eq!(utc.date_line(), "Thursday, October 29");
        let epoch = civil_at(0, 0);
        assert_eq!((epoch.year, epoch.month, epoch.day, epoch.weekday), (1970, 1, 1, 4));
        let leap = civil_at(951_782_400_000, 0);
        assert_eq!((leap.year, leap.month, leap.day), (2000, 2, 29), "2000 was a leap year");
        let before = civil_at(0, -3600);
        assert_eq!((before.year, before.month, before.day, before.hour), (1969, 12, 31, 23));
    }

    #[test]
    fn a_local_time_is_read_back_as_the_same_calendar_and_clock() {
        assert_eq!(epoch_of(1970, 1, 1, 0, 0, 0, 0), 0);
        assert_eq!(epoch_of(2026, 10, 29, 2, 35, 12, 0), 1_793_241_312_000);
        assert_eq!(epoch_of(2000, 2, 29, 0, 0, 0, 0), 951_782_400_000, "a leap day");
        let ms = parse_local("2026-10-29T10:08:35").expect("a time");
        let c = civil_at(ms, system_offset_s(ms));
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute, c.second), (2026, 10, 29, 10, 8, 35));
        assert_eq!(parse_local("2026-10-29 07:30").map(|m| civil_at(m, system_offset_s(m)).minute), Some(30), "a space for the T, no seconds");
        assert!(parse_local("2026-13-01T00:00:00").is_none() && parse_local("nonsense").is_none());
    }

    #[test]
    fn day_seconds_run_from_midnight_with_the_fraction() {
        let c = civil_at(1_793_241_312_250, 0);
        assert!((c.day_seconds() - (2.0 * 3600.0 + 35.0 * 60.0 + 12.25)).abs() < 1e-9);
    }

    #[test]
    fn the_system_offset_is_a_whole_number_of_minutes() {
        let off = system_offset_s(1_793_241_312_250);
        assert_eq!(off % 60, 0);
        assert!(off.abs() <= 14 * 3600);
    }
}
