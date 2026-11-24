//! The week on the weather card, and the day that opens out of it: the geometry only (the card draws and words it, [`super::weather_card`]).
//!
//! The week is a list, a row a day: the day, its picture, the chance of rain, the low, the range bar on the week's one scale, the high. A day that is
//! opened takes the room the week had and shows its hours, every third one, as columns of time, picture, temperature, rain and wind.

use ui::geometry::Frame;

/// The room of one row of the list at the least, and of a column's bar.
pub const ROW_MIN: f32 = 38.0;
pub const ROW_MAX: f32 = 48.0;

#[derive(Clone, Copy, Debug)]
pub struct Week {
    pub frame: Frame,
    /// Days shown.
    pub n: usize,
    /// A row's height (list) or a column's width (columns).
    pub step: f32,
    pub name_w: f32,
    pub icon: f32,
    /// The chance of rain's width; none where the week is narrow.
    pub precip_w: f32,
    pub num_w: f32,
    pub bar: bool,
}

impl Week {
    /// The list in `frame`: as many rows of 38 to 48 pixels as it holds (seven at most), with the parts its width has room for.
    pub fn list(frame: Frame) -> Option<Week> {
        let n = ((frame.h / ROW_MIN).floor() as usize).min(7);
        if n < 3 || frame.w < 206.0 {
            return None;
        }
        let step = (frame.h / n as f32).floor().min(ROW_MAX);
        let w = frame.w;
        let (name_w, icon, precip_w, num_w, bar) = if w >= 470.0 {
            (104.0, 34.0, 56.0, 52.0, true)
        } else if w >= 330.0 {
            (92.0, 32.0, 0.0, 52.0, true)
        } else {
            (80.0, 30.0, 0.0, 46.0, false)
        };
        Some(Week { frame, n, step, name_w, icon, precip_w, num_w, bar })
    }
    // ---- the list
    pub fn row_y(&self, i: usize) -> f32 {
        self.frame.y + i as f32 * self.step
    }

    pub fn lo_x(&self) -> f32 {
        if self.bar { self.frame.x + self.name_w + self.icon + 8.0 + self.precip_w + if self.precip_w > 0.0 { 8.0 } else { 0.0 } } else { self.frame.right() - 2.0 * self.num_w }
    }

    pub fn precip_x(&self) -> f32 {
        self.frame.x + self.name_w + self.icon + 8.0
    }

    /// Where the range bar of a row runs, from and to.
    pub fn bar_span(&self) -> (f32, f32) {
        (self.lo_x() + self.num_w + 14.0, self.frame.right() - self.num_w - 14.0)
    }
}

/// The day opened out over the room the week had: a head (the day, its range) and the hours' columns.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub frame: Frame,
    pub head_h: f32,
    /// Columns of hours, each 3 hours apart.
    pub cols: usize,
    pub col_w: f32,
    /// The rows of a column, from the body's top (the frame's top and the head under it): the time, the picture, the temperature, the bar of the rain and its number, the wind.
    pub time_y: f32,
    pub icon_y: f32,
    pub icon: f32,
    pub temp_y: f32,
    pub temp_px: f32,
    pub bar: Option<(f32, f32)>,
    pub rain_y: f32,
    pub wind_y: Option<f32>,
}

pub const PANEL_HEAD: f32 = 46.0;

impl Panel {
    pub fn new(frame: Frame) -> Option<Panel> {
        let body = frame.h - PANEL_HEAD;
        if body < 118.0 || frame.w < 240.0 {
            return None;
        }
        let cols = ((frame.w / 76.0).floor() as usize).clamp(3, 8);
        let col_w = (frame.w / cols as f32).floor();
        let y0 = frame.y + PANEL_HEAD;
        // the richest of the three that the room holds
        let (icon, temp_off, bar, rain_off, wind_off) = if body >= 214.0 {
            (38.0, 74.0, Some((112.0, 44.0)), 160.0, Some(188.0))
        } else if body >= 168.0 {
            (36.0, 70.0, None, 108.0, Some(138.0))
        } else {
            (30.0, 60.0, None, 94.0, None)
        };
        // the rows spread over a taller body, by a quarter at most
        let s = (body / (wind_off.unwrap_or(rain_off) + 32.0)).clamp(1.0, 1.25);
        Some(Panel {
            frame,
            head_h: PANEL_HEAD,
            cols,
            col_w,
            time_y: y0,
            icon_y: y0 + 28.0,
            icon,
            temp_y: y0 + temp_off * s,
            temp_px: 28.0,
            bar: bar.map(|(t, h)| (y0 + t * s, h * s)),
            rain_y: y0 + rain_off * s,
            wind_y: wind_off.map(|o| y0 + o * s),
        })
    }

    pub fn col_x(&self, i: usize) -> f32 {
        self.frame.x + i as f32 * self.col_w
    }
}

/// The hours of the day `date` (`YYYY-MM-DD`) at the `cols` of every third one from midnight: those the hours say, as many as there are.
/// A day the hours begin in the middle of (today, from the current hour on) starts at its first hour instead, and takes every hour while
/// the rest of the day fits the columns: at 22:00 the third-hour grid has no hour left.
pub fn day_hours<'a>(hours: &'a [serde_json::Value], date: &str, cols: usize) -> Vec<&'a serde_json::Value> {
    let step = if cols >= 8 { 3 } else { 24 / cols.max(1) };
    let of_day: Vec<(&serde_json::Value, usize)> = hours
        .iter()
        .filter_map(|h| {
            let t = h.get("time").and_then(serde_json::Value::as_str).filter(|t| t.starts_with(date))?;
            Some((h, t.get(11..13)?.parse().ok()?))
        })
        .collect();
    match of_day.first() {
        Some(&(_, first)) if first > 0 => {
            let stride = if of_day.len() <= cols { 1 } else { step };
            of_day.into_iter().step_by(stride).take(cols).map(|(h, _)| h).collect()
        }
        _ => of_day.into_iter().filter(|(_, h)| h % step == 0).map(|(v, _)| v).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_seven_days_at_the_current_size_and_fewer_in_a_smaller_card() {
        let w = Week::list(Frame::new(0.0, 108.0, 671.0, 316.0)).expect("a list");
        assert_eq!((w.n, w.step), (7, 45.0));
        assert!(w.bar && w.precip_w > 0.0);
        assert!(w.bar_span().1 - w.bar_span().0 >= 200.0, "a bar worth reading");
        assert_eq!(Week::list(Frame::new(0.0, 0.0, 369.0, 170.0)).map(|w| w.n), Some(4), "a small card: today and the next three");
        assert!(Week::list(Frame::new(0.0, 0.0, 369.0, 100.0)).is_none() && Week::list(Frame::new(0.0, 0.0, 150.0, 400.0)).is_none());
    }

    #[test]
    fn the_opened_day_stays_in_its_frame() {
        for h in [316.0, 242.0, 190.0, 130.0] {
            let Some(p) = Panel::new(Frame::new(0.0, 100.0, 671.0, h)) else { continue };
            let bottom = p.wind_y.map(|y| y + 24.0).unwrap_or(p.rain_y + 24.0);
            assert!(bottom <= 100.0 + h, "{h}: the last row is inside");
            assert!(p.col_w * p.cols as f32 <= 671.0);
        }
        assert!(Panel::new(Frame::new(0.0, 0.0, 671.0, 100.0)).is_none());
    }

    #[test]
    fn a_day_has_every_third_hour() {
        let hours: Vec<serde_json::Value> = (0..48).map(|i| serde_json::json!({"time": format!("2026-11-{:02}T{:02}:00", 23 + i / 24, i % 24)})).collect();
        assert_eq!(day_hours(&hours, "2026-11-24", 8).len(), 8);
        assert_eq!(day_hours(&hours, "2026-11-23", 8)[1]["time"], "2026-11-23T03:00");
        assert_eq!(day_hours(&hours, "2026-11-25", 8).len(), 0);
    }

    #[test]
    fn today_shows_the_hours_from_now_on_and_a_late_evening_is_not_empty() {
        let from = |first: usize| -> Vec<serde_json::Value> { (first..24).map(|h| serde_json::json!({"time": format!("2026-11-24T{h:02}:00")})).collect() };
        let times = |h: &[serde_json::Value]| day_hours(h, "2026-11-24", 8).iter().map(|v| v["time"].as_str().unwrap()[11..13].to_owned()).collect::<Vec<_>>();
        assert_eq!(times(&from(22)), ["22", "23"], "no third hour is left, every hour is");
        assert_eq!(times(&from(16)), ["16", "17", "18", "19", "20", "21", "22", "23"]);
        assert_eq!(times(&from(8)), ["08", "11", "14", "17", "20", "23"], "more than the columns hold: every third hour from the first");
        assert_eq!(times(&from(0)).len(), 8, "a whole day keeps its grid");
    }
}
