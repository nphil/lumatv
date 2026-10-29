//! Local time for the panels' words: the quick panel's clock ("9:52 PM"), the sleep timer's "off at", the notification drawer's "Yesterday"
//! and "Oct 22". The zone is the TV's (`localtime_r`); the formatting is pure so a test fixes the time.

/// A broken-down local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 0 to 11.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The local time at `ms` (milliseconds since the epoch).
pub fn local(ms: u64) -> Local {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: `tm` is plain data that `localtime_r` fills in; `secs` is a valid time_t on its own stack.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return Local { year: 1970, month: 0, day: 1, hour: 0, minute: 0 };
        }
        tm
    };
    Local { year: tm.tm_year + 1900, month: tm.tm_mon.clamp(0, 11) as u32, day: tm.tm_mday as u32, hour: tm.tm_hour as u32, minute: tm.tm_min as u32 }
}

/// `h:mm AP` as Qt writes it: "9:52 PM", "12:05 AM".
pub fn h_mm_ap(t: Local) -> String {
    let h12 = match t.hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{}:{:02} {}", h12, t.minute, if t.hour < 12 { "AM" } else { "PM" })
}

/// `MMM d`: "Oct 22".
pub fn month_day(t: Local) -> String {
    format!("{} {}", MONTHS[t.month.min(11) as usize], t.day)
}

pub fn same_day(a: Local, b: Local) -> bool {
    (a.year, a.month, a.day) == (b.year, b.month, b.day)
}

/// The time of day `ms` as "9:52 PM".
pub fn time_of(ms: u64) -> String {
    h_mm_ap(local(ms))
}

/// What the notification drawer says of when something happened: the time today, "Yesterday" within two days, else the date.
pub fn when(at_ms: u64, now_ms: u64) -> String {
    if at_ms == 0 {
        return String::new();
    }
    let (a, n) = (local(at_ms), local(now_ms));
    if same_day(a, n) {
        h_mm_ap(a)
    } else if now_ms.saturating_sub(at_ms) < 172_800_000 {
        "Yesterday".into()
    } else {
        month_day(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(hour: u32, minute: u32) -> Local {
        Local { year: 2026, month: 9, day: 22, hour, minute }
    }

    #[test]
    fn the_hour_is_on_a_twelve_hour_clock_with_am_and_pm() {
        assert_eq!(h_mm_ap(t(21, 52)), "9:52 PM");
        assert_eq!(h_mm_ap(t(0, 5)), "12:05 AM");
        assert_eq!(h_mm_ap(t(12, 0)), "12:00 PM");
        assert_eq!(h_mm_ap(t(9, 7)), "9:07 AM");
    }

    #[test]
    fn a_date_is_a_month_and_a_day() {
        assert_eq!(month_day(t(1, 1)), "Oct 22");
        assert!(same_day(t(1, 1), t(23, 59)));
    }

    #[test]
    fn local_time_is_the_zones_and_when_says_today_yesterday_or_the_date() {
        let now = 1_761_000_000_000u64;
        let l = local(now);
        assert!((1..=31).contains(&l.day) && l.hour < 24 && l.minute < 60);
        assert_eq!(when(0, now), "");
        assert_eq!(when(now - 60_000, now), when(now - 60_000, now));
        assert_eq!(when(now - 10 * 86_400_000, now), month_day(local(now - 10 * 86_400_000)));
    }
}
