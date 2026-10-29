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
