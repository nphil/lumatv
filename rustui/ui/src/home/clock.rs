//! Civil time for the clock card and the status bar: the TV's local time (the system's zone, through `localtime_r`, which is what
//! the QML's `Date` read), the words of the clock widget (`ClockWidget.qml`: the greeting by the hour, "Saturday", "October 25", the
//! week's initials with today marked, the months of a calendar), and the analogue face rastered on the CPU (the dial once, the hands
//! per second) into premultiplied pixels.

use compositor_protocol::Rgba;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 0 is Sunday.
    pub weekday: u32,
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const INITIALS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

/// The day a week starts on (`clock.weekStart`, a profile's own key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeekStart {
    Monday,
    Sunday,
}

/// What a cell of the month grid is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Day {
    /// A day of the month before or after the shown one.
    Other,
    This,
    Today,
}

/// The regions whose weeks start on a Sunday (CLDR's `firstDay`); everywhere else it is a Monday.
const SUNDAY_REGIONS: &[&str] = &[
    "AG", "AR", "AS", "AU", "BD", "BR", "BS", "BT", "BW", "BZ", "CA", "CO", "DM", "DO", "ET", "GT", "GU", "HK", "HN", "ID", "IL", "IN", "JM", "JP", "KE", "KH", "KR", "LA", "MH", "MM", "MO", "MT", "MX", "MZ", "NI", "NP", "PA", "PE",
    "PH", "PK", "PR", "PT", "PY", "SA", "SG", "SV", "TH", "TT", "TW", "UM", "US", "VE", "VI", "WS", "YE", "ZA", "ZW",
];

impl WeekStart {
    /// What the setting says (`monday` or `sunday`); with anything else, what the TV's locale uses.
    pub fn of(setting: Option<&str>) -> Self {
        match setting {
            Some("monday") => Self::Monday,
            Some("sunday") => Self::Sunday,
            _ => Self::of_locale(["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).as_deref()),
        }
    }

    /// The week of a locale name (`en_US.UTF-8`): a Sunday in the regions that begin it so, a Monday otherwise and where there is no region.
    pub fn of_locale(locale: Option<&str>) -> Self {
        let region = locale.and_then(|l| l.split(['.', '@']).next()).and_then(|l| l.split(['_', '-']).nth(1));
        if region.is_some_and(|r| SUNDAY_REGIONS.contains(&r.to_ascii_uppercase().as_str())) { Self::Sunday } else { Self::Monday }
    }

    /// Where `weekday` (0 is Sunday) falls in a week that starts here.
    pub fn column(self, weekday: u32) -> u32 {
        match self {
            Self::Sunday => weekday % 7,
            Self::Monday => (weekday + 6) % 7,
        }
    }

    /// The initials of the seven days, the first of them the day a week starts on.
    pub fn initials(self) -> [&'static str; 7] {
        std::array::from_fn(|i| INITIALS[(i + usize::from(self == Self::Monday)) % 7])
    }
}

/// The local civil time of `ms` since the epoch.
pub fn civil(ms: u64) -> Civil {
    let t: libc::time_t = (ms / 1000) as libc::time_t;
    // SAFETY: a zeroed tm is a valid out-parameter for localtime_r, which fills every field it reads back.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live stack values for the call.
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if !ok {
        return Civil { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, weekday: 4 };
    }
    Civil {
        year: tm.tm_year + 1900,
        month: tm.tm_mon as u32 + 1,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
        weekday: tm.tm_wday as u32,
    }
}

impl Civil {
    /// "2:31" and "PM" for a twelve-hour clock; "14:31" and "" otherwise.
    pub fn time_words(&self, twelve: bool) -> (String, &'static str) {
        if twelve {
            let h = match self.hour % 12 {
                0 => 12,
                h => h,
            };
            (format!("{h}:{:02}", self.minute), if self.hour < 12 { "AM" } else { "PM" })
        } else {
            (format!("{}:{:02}", self.hour, self.minute), "")
        }
    }

    pub fn weekday_name(&self) -> &'static str {
        DAYS[(self.weekday % 7) as usize]
    }

    /// "October 25".
    pub fn date_line(&self) -> String {
        format!("{} {}", MONTHS[(self.month as usize - 1).min(11)], self.day)
    }

    /// Good morning until noon, afternoon until six, evening after.
    pub fn greeting(&self) -> &'static str {
        match self.hour {
            0..=11 => "Good morning",
            12..=17 => "Good afternoon",
            _ => "Good evening",
        }
    }

    /// The week around today: the day of the month, its initial, whether it is today.
    pub fn week(&self, start: WeekStart) -> [(u32, &'static str, bool); 7] {
        let mut out = [(0, "", false); 7];
        let dim = days_in_month(self.year, self.month);
        let (py, pm) = if self.month == 1 { (self.year - 1, 12) } else { (self.year, self.month - 1) };
        let pdim = days_in_month(py, pm);
        for (i, slot) in out.iter_mut().enumerate() {
            let offset = i as i32 - start.column(self.weekday) as i32;
            let mut d = self.day as i32 + offset;
            if d < 1 {
                d += pdim as i32;
            } else if d > dim as i32 {
                d -= dim as i32;
            }
            *slot = (d as u32, INITIALS[(i + usize::from(start == WeekStart::Monday)) % 7], offset == 0);
        }
        out
    }

    /// "November 2026".
    pub fn month_title(&self) -> String {
        format!("{} {}", MONTHS[(self.month as usize - 1).min(11)], self.year)
    }

    /// The month's six weeks as seven columns by six rows, the first column the `start` day: every cell is a day of the month (marked if it is today) or a
    /// day of the month before or after, which fills the weeks out. Six rows hold every month whichever day the week starts on, so the grid never changes height.
    pub fn month(&self, start: WeekStart) -> [(u32, Day); 42] {
        let first = start.column(weekday_of(self.year, self.month, 1)) as i32;
        let dim = days_in_month(self.year, self.month) as i32;
        let (py, pm) = if self.month == 1 { (self.year - 1, 12) } else { (self.year, self.month - 1) };
        let pdim = days_in_month(py, pm) as i32;
        let mut cells = [(0, Day::Other); 42];
        for (i, cell) in cells.iter_mut().enumerate() {
            let d = i as i32 - first + 1;
            *cell = if d < 1 {
                ((pdim + d) as u32, Day::Other)
            } else if d > dim {
                ((d - dim) as u32, Day::Other)
            } else {
                (d as u32, if d as u32 == self.day { Day::Today } else { Day::This })
            };
        }
        cells
    }
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// Zeller's rule: 0 is Sunday.
pub fn weekday_of(year: i32, month: u32, day: u32) -> u32 {
    let (y, m) = if month < 3 { (year - 1, month + 12) } else { (year, month) };
    let k = y.rem_euclid(100);
    let j = y.div_euclid(100);
    let h = (day as i32 + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    ((h + 6) % 7) as u32
}

// ---- the analogue face
fn blend(px: &mut [u8], colour: Rgba, a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let src = [colour.0 as f32 * a * colour.3 as f32 / 255.0, colour.1 as f32 * a * colour.3 as f32 / 255.0, colour.2 as f32 * a * colour.3 as f32 / 255.0, colour.3 as f32 * a];
    for i in 0..4 {
        let d = px[i] as f32;
        px[i] = (src[i] + d * (1.0 - src[3] / 255.0)).round().min(255.0) as u8;
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 <= 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// A round-capped stroke from `a` to `b`, `w` wide, into `pixels` (`side` square, premultiplied).
pub fn stroke(pixels: &mut [u8], side: u32, a: (f32, f32), b: (f32, f32), w: f32, colour: Rgba) {
    let r = w / 2.0;
    let (minx, maxx) = ((a.0.min(b.0) - r - 1.0).floor().max(0.0) as u32, (a.0.max(b.0) + r + 1.0).ceil().min(side as f32) as u32);
    let (miny, maxy) = ((a.1.min(b.1) - r - 1.0).floor().max(0.0) as u32, (a.1.max(b.1) + r + 1.0).ceil().min(side as f32) as u32);
    for y in miny..maxy {
        for x in minx..maxx {
            let d = seg_dist((x as f32 + 0.5, y as f32 + 0.5), a, b);
            let cov = (r - d + 0.5).clamp(0.0, 1.0);
            let i = ((y * side + x) * 4) as usize;
            blend(&mut pixels[i..i + 4], colour, cov);
        }
    }
}

/// A disc at `c` of radius `r`.
pub fn disc(pixels: &mut [u8], side: u32, c: (f32, f32), r: f32, colour: Rgba) {
    stroke(pixels, side, c, c, 2.0 * r, colour);
}

/// A ring at `c`: radius `r`, `w` wide.
pub fn ring(pixels: &mut [u8], side: u32, c: (f32, f32), r: f32, w: f32, colour: Rgba) {
    let (minx, maxx) = ((c.0 - r - w).floor().max(0.0) as u32, (c.0 + r + w).ceil().min(side as f32) as u32);
    let (miny, maxy) = ((c.1 - r - w).floor().max(0.0) as u32, (c.1 + r + w).ceil().min(side as f32) as u32);
    for y in miny..maxy {
        for x in minx..maxx {
            let d = (((x as f32 + 0.5) - c.0).powi(2) + ((y as f32 + 0.5) - c.1).powi(2)).sqrt();
            let cov = (w / 2.0 - (d - r).abs() + 0.5).clamp(0.0, 1.0);
            let i = ((y * side + x) * 4) as usize;
            blend(&mut pixels[i..i + 4], colour, cov);
        }
    }
}

/// The dial: a faint ring, twelve ticks (the quarters longer and in the full ink).
pub fn draw_dial(pixels: &mut [u8], side: u32, ink: Rgba, ink3: Rgba) {
    pixels.fill(0);
    let c = (side as f32 / 2.0, side as f32 / 2.0);
    let r = side as f32 / 2.0 - 2.0;
    let k = side as f32 / 166.0;
    ring(pixels, side, c, r, 1.5 * k, ink3);
    for i in 0..12 {
        let a = i as f32 * std::f32::consts::PI / 6.0;
        let quarter = i % 3 == 0;
        let (len, w, col) = if quarter { (10.0 * k, 3.0 * k, ink) } else { (6.0 * k, 2.0 * k, ink3) };
        let (sx, sy) = (c.0 + (r - 6.0 * k) * a.sin(), c.1 - (r - 6.0 * k) * a.cos());
        let (ex, ey) = (c.0 + (r - 6.0 * k - len) * a.sin(), c.1 - (r - 6.0 * k - len) * a.cos());
        stroke(pixels, side, (sx, sy), (ex, ey), w, col);
    }
}

/// The hands at `civil`'s time: the hour and the minute in the ink, the second in its colour, a hub at the centre.
pub fn draw_hands(pixels: &mut [u8], side: u32, t: &Civil, ink: Rgba, second: Option<Rgba>) {
    pixels.fill(0);
    let c = (side as f32 / 2.0, side as f32 / 2.0);
    let r = side as f32 / 2.0 - 2.0;
    let k = side as f32 / 166.0;
    let hand = |px: &mut [u8], angle: f32, len: f32, back: f32, w: f32, col: Rgba| {
        let (s, co) = (angle.sin(), angle.cos());
        stroke(px, side, (c.0 - back * s, c.1 + back * co), (c.0 + len * s, c.1 - len * co), w, col);
    };
    let pi2 = 2.0 * std::f32::consts::PI;
    let ha = ((t.hour % 12) as f32 + t.minute as f32 / 60.0) / 12.0 * pi2;
    let ma = (t.minute as f32 + t.second as f32 / 60.0) / 60.0 * pi2;
    hand(pixels, ha, r * 0.52, 0.0, 6.0 * k, ink);
    hand(pixels, ma, r * 0.78, 0.0, 4.0 * k, ink);
    if let Some(sc) = second {
        let sa = t.second as f32 / 60.0 * pi2;
        hand(pixels, sa, r * 0.86, r * 0.18, 1.8 * k, sc);
        disc(pixels, side, c, 4.5 * k, sc);
    } else {
        disc(pixels, side, c, 4.5 * k, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_follow_the_hour_and_the_calendar_is_right() {
        let t = Civil { year: 2026, month: 10, day: 25, hour: 14, minute: 5, second: 0, weekday: 0 };
        assert_eq!(t.time_words(true), ("2:05".into(), "PM"));
        assert_eq!(t.time_words(false), ("14:05".into(), ""));
        assert_eq!(t.greeting(), "Good afternoon");
        assert_eq!(t.date_line(), "October 25");
        assert_eq!(t.weekday_name(), "Sunday");
        assert_eq!(weekday_of(2026, 10, 25), 0);
        assert_eq!(weekday_of(2026, 10, 1), 4);
        let w = t.week(WeekStart::Sunday);
        assert_eq!(w[0], (25, "S", true));
        assert_eq!(w[6], (31, "S", false));
        assert_eq!(t.month_title(), "October 2026");
        let cells = t.month(WeekStart::Sunday);
        assert_eq!(cells[4], (1, Day::This));
        assert_eq!(cells[28], (25, Day::Today));
        assert_eq!(days_in_month(2028, 2), 29);
        let m = Civil { year: 2026, month: 1, day: 1, hour: 7, minute: 0, second: 0, weekday: 4 };
        assert_eq!(m.greeting(), "Good morning");
        assert_eq!(m.week(WeekStart::Sunday)[0].0, 28, "the week reaches into December");
        assert!(matches!(civil(0).year, 1969 | 1970));
    }

    #[test]
    fn a_week_that_starts_on_monday_ends_on_the_sunday_and_the_month_leans_to_match() {
        // Sunday 25 October 2026: the last day of its week when the week starts on Monday
        let t = Civil { year: 2026, month: 10, day: 25, hour: 14, minute: 5, second: 0, weekday: 0 };
        let w = t.week(WeekStart::Monday);
        assert_eq!((w[0], w[6]), ((19, "M", false), (25, "S", true)));
        assert_eq!(w.map(|d| d.1).concat(), "MTWTFSS");
        // 1 October 2026 is a Thursday: the fourth cell of a Monday week, the fifth of a Sunday one
        let monday = t.month(WeekStart::Monday);
        assert_eq!((monday[3], monday[27]), ((1, Day::This), (25, Day::Today)));
        assert_eq!(monday[0], (28, Day::Other), "the days before the first are September's");
        assert_eq!(monday[41], (8, Day::Other), "and those after the last are November's");
        // a week across a month's end, started on a Monday
        let m = Civil { year: 2026, month: 1, day: 1, hour: 7, minute: 0, second: 0, weekday: 4 };
        assert_eq!(m.week(WeekStart::Monday).map(|d| d.0), [29, 30, 31, 1, 2, 3, 4]);
    }

    #[test]
    fn every_month_fills_six_rows_with_its_own_days_in_order_whichever_day_the_week_starts_on() {
        // 28, 29, 30 and 31 days; Feb 2026 starts on a Sunday, Feb 2027 on a Monday, Aug 2026 on a Saturday (it needs six weeks)
        for (year, month, dim) in [(2026, 2, 28), (2027, 2, 28), (2028, 2, 29), (2026, 4, 30), (2026, 8, 31), (2026, 12, 31), (2027, 1, 31)] {
            for start in [WeekStart::Monday, WeekStart::Sunday] {
                let t = Civil { year, month, day: 15, hour: 9, minute: 0, second: 0, weekday: weekday_of(year, month, 15) };
                let cells = t.month(start);
                let own: Vec<u32> = cells.iter().filter(|c| c.1 != Day::Other).map(|c| c.0).collect();
                assert_eq!(own, (1..=dim).collect::<Vec<u32>>(), "{year}-{month} {start:?}");
                assert_eq!(cells.iter().filter(|c| c.1 == Day::Today).collect::<Vec<_>>(), [&(15, Day::Today)]);
                let first = start.column(weekday_of(year, month, 1)) as usize;
                assert_eq!(cells[first], (1, Day::This));
                assert!(cells[..first].iter().chain(&cells[first + dim as usize..]).all(|c| c.1 == Day::Other));
            }
        }
        let aug = Civil { year: 2026, month: 8, day: 31, hour: 9, minute: 0, second: 0, weekday: 1 };
        assert_eq!(aug.month(WeekStart::Sunday)[36], (31, Day::Today), "a month of six weeks reaches the sixth row");
        assert_eq!(WeekStart::Monday.initials().concat(), "MTWTFSS");
        assert_eq!(WeekStart::Sunday.initials().concat(), "SMTWTFS");
    }

    #[test]
    fn the_week_starts_as_the_setting_says_and_else_as_the_locale_does() {
        assert_eq!(WeekStart::of(Some("monday")), WeekStart::Monday);
        assert_eq!(WeekStart::of(Some("sunday")), WeekStart::Sunday);
        assert_eq!(WeekStart::of_locale(Some("en_US.UTF-8")), WeekStart::Sunday);
        assert_eq!(WeekStart::of_locale(Some("en-GB")), WeekStart::Monday);
        assert_eq!(WeekStart::of_locale(Some("de_DE.UTF-8@euro")), WeekStart::Monday);
        assert_eq!(WeekStart::of_locale(Some("C")), WeekStart::Monday);
        assert_eq!(WeekStart::of_locale(None), WeekStart::Monday);
    }

    #[test]
    fn the_face_and_the_hands_draw_inside_the_square() {
        let side = 166;
        let mut px = vec![0u8; (side * side * 4) as usize];
        draw_dial(&mut px, side, Rgba(255, 255, 255, 255), Rgba(255, 255, 255, 110));
        assert!(px.chunks_exact(4).any(|p| p[3] > 0));
        let t = Civil { year: 2026, month: 10, day: 25, hour: 3, minute: 0, second: 15, weekday: 0 };
        draw_hands(&mut px, side, &t, Rgba(255, 255, 255, 255), Some(Rgba(255, 59, 92, 255)));
        // at three o'clock the hour hand points right of the centre
        let at = |x: u32, y: u32| px[((y * side + x) * 4 + 3) as usize];
        assert!(at(110, 83) > 0);
        assert!(at(83, 20) > 0, "the minute hand points up");
        assert_eq!(at(0, 0), 0);
    }
}
