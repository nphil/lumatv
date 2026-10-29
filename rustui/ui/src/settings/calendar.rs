//! The TV's clock as its Time & Date page sets it (`settings/PageClock.qml`): local time from seconds and back, and one press on a field.
//! The conversions are the C library's, so the zone is the one the TV runs in.

/// A moment as a person reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 0 for January.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Hour,
    Minute,
    Year,
    Month,
    Day,
}

pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

pub fn local(secs: i64) -> Local {
    let t = secs as libc::time_t;
    // SAFETY: `tm` is plain data, `localtime_r` fills it for the time given and keeps nothing.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    Local { year: tm.tm_year + 1900, month: tm.tm_mon as u32, day: tm.tm_mday as u32, hour: tm.tm_hour as u32, minute: tm.tm_min as u32 }
}

/// Seconds since the epoch of a local time (this UI is 64-bit only: `time_t` is an `i64`).
pub fn utc(l: &Local) -> i64 {
    // SAFETY: `tm` is plain data; `mktime` reads it and normalises it, keeping nothing.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = l.year - 1900;
        tm.tm_mon = l.month as i32;
        tm.tm_mday = l.day as i32;
        tm.tm_hour = l.hour as i32;
        tm.tm_min = l.minute as i32;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    }
}

pub fn days_in(year: i32, month: u32) -> u32 {
    match month {
        1 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        1 => 28,
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    }
}

/// One press on a field: the field moves a step, a month or a day wrapping within its own range, a year held to 2020..=2037 (what LG's
/// clock takes), the day kept within the month it lands in; seconds are zero.
pub fn step(secs: i64, field: Field, d: i32) -> i64 {
    let mut l = local(secs);
    let wrap = |v: u32, d: i32, n: u32| ((i64::from(v) + i64::from(d)).rem_euclid(i64::from(n))) as u32;
    match field {
        Field::Hour => l.hour = wrap(l.hour, d, 24),
        Field::Minute => l.minute = wrap(l.minute, d, 60),
        Field::Year => {
            l.year = (l.year + d).clamp(2020, 2037);
            l.day = l.day.min(days_in(l.year, l.month));
        }
        Field::Month => {
            l.month = wrap(l.month, d, 12);
            l.day = l.day.min(days_in(l.year, l.month));
        }
        Field::Day => {
            let n = days_in(l.year, l.month);
            l.day = wrap(l.day - 1, d, n) + 1;
        }
    }
    utc(&l)
}

/// "3 PM" for an hour of the day.
pub fn hour_text(h: u32) -> String {
    format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
}

/// "3:05 PM".
pub fn clock_text(h: u32, m: u32) -> String {
    format!("{}:{m:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_wraps_within_its_own_range_and_keeps_the_day_inside_the_month() {
        let at = |y, mo, d, h, mi| utc(&Local { year: y, month: mo, day: d, hour: h, minute: mi });
        let jan31 = at(2026, 0, 31, 23, 59);
        assert_eq!(local(step(jan31, Field::Hour, 1)).hour, 0, "an hour wraps, the day does not move");
        assert_eq!(local(step(jan31, Field::Hour, 1)).day, 31);
        assert_eq!(local(step(jan31, Field::Minute, 1)).minute, 0);
        let feb = local(step(jan31, Field::Month, 1));
        assert_eq!((feb.month, feb.day), (1, 28), "the 31st of January into February is the 28th");
        assert_eq!(local(step(at(2024, 0, 31, 1, 0), Field::Month, 1)).day, 29, "a leap year");
        assert_eq!(local(step(at(2026, 11, 15, 1, 0), Field::Month, 1)).month, 0, "December wraps to January");
        assert_eq!(local(step(at(2026, 5, 30, 1, 0), Field::Day, 1)).day, 1, "the 30th of June wraps to the 1st");
        assert_eq!(local(step(at(2026, 5, 1, 1, 0), Field::Day, -1)).day, 30);
        assert_eq!(local(step(at(2037, 5, 1, 1, 0), Field::Year, 1)).year, 2037, "LG's clock stops at 2037");
        assert_eq!(local(step(at(2020, 5, 1, 1, 0), Field::Year, -1)).year, 2020);
        assert_eq!(local(step(at(2024, 1, 29, 1, 0), Field::Year, 1)).day, 28, "the 29th of February into a year without one");
    }

    #[test]
    fn the_words_for_a_time_are_twelve_hour() {
        assert_eq!(hour_text(0), "12 AM");
        assert_eq!(hour_text(15), "3 PM");
        assert_eq!(clock_text(0, 5), "12:05 AM");
        assert_eq!(clock_text(12, 30), "12:30 PM");
    }
}
