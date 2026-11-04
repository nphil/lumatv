//! The chart of a sensor's readings: the scales (a range that is padded, or fixed where the thing has one: a humidity is 0 to 100), the ticks
//! on them, the smooth curve through the points, and the picture of it painted on the CPU, once, when the readings arrive.
//!
//! The picture has a soft fill under the curve (the accent at its strongest at the line, nothing at the foot), gridlines one pixel wide on
//! whole pixels, a line that is anti-aliased and three pixels wide at the least, and a small mark where the readings were lowest and highest.
//! Nothing here reads a clock or a font: the axis words are the surface's, set as text.

use super::super::clock::{self, Civil};
use compositor_protocol::Rgba;

/// A scale of values: its ends and where its ticks are.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub lo: f64,
    pub hi: f64,
    pub ticks: Vec<f64>,
}

impl Axis {
    /// How far up a plot of `h` pixels the value `v` is, as a row from the top (`h` is the foot).
    pub fn row(&self, v: f64, h: f32) -> f32 {
        let t = ((v - self.lo) / (self.hi - self.lo).max(1e-9)).clamp(0.0, 1.0) as f32;
        (1.0 - t) * h
    }
}

fn nice_step(raw: f64) -> f64 {
    let raw = raw.max(1e-9);
    let e = raw.log10().floor();
    let (base, frac) = (10f64.powf(e), raw / 10f64.powf(e));
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

/// Whether a reading is a share of a whole: a thing that is read from none to a hundred.
pub fn is_percent(class: &str, unit: &str, min: f64, max: f64) -> bool {
    matches!(class, "humidity" | "moisture" | "battery") || (unit == "%" && min >= 0.0 && max <= 100.0)
}

/// The scale for readings between `min` and `max`: padded by a tenth each way and ticked at a round step, so that a line that hardly moves is
/// not stretched across the whole plot; a share of a whole is from 0 to 100 whatever the readings were. No more than `most` ticks.
pub fn axis(min: f64, max: f64, percent: bool, most: usize) -> Axis {
    let most = most.max(2);
    if percent {
        let ticks: Vec<f64> = if most >= 5 { vec![0.0, 25.0, 50.0, 75.0, 100.0] } else if most >= 3 { vec![0.0, 50.0, 100.0] } else { vec![0.0, 100.0] };
        return Axis { lo: 0.0, hi: 100.0, ticks };
    }
    let span = max - min;
    let pad = if span > 1e-9 { span * 0.1 } else { max.abs().max(1.0) * 0.1 };
    let (lo, hi) = (min - pad, max + pad);
    if most == 2 {
        // two ticks, at the ends: the readings padded out to a round step of their own range, not stretched to a round step of a coarser one
        let fine = nice_step((hi - lo) / 4.0);
        let (a, b) = ((lo / fine).floor() * fine, (hi / fine).ceil() * fine);
        return Axis { lo: a, hi: b, ticks: vec![a, b] };
    }
    let mut step = nice_step((hi - lo) / (most as f64 - 1.0));
    loop {
        let (a, b) = ((lo / step).floor() * step, (hi / step).ceil() * step);
        let n = ((b - a) / step).round() as usize + 1;
        if n <= most {
            let ticks = (0..n).map(|i| ((a + i as f64 * step) / step).round() * step).collect();
            return Axis { lo: a, hi: b, ticks };
        }
        step = nice_step(step * 1.5);
    }
}

/// A tick as it is written: whole when the step is, else to the tenth (or hundredth for a fine step).
pub fn tick_text(v: f64, step: f64) -> String {
    if step >= 1.0 - 1e-9 && (step - step.round()).abs() < 1e-9 && (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else if step >= 0.1 - 1e-9 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// A tick of the time axis: where, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeTick {
    pub at: f64,
    pub label: String,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn clock_words(c: &Civil, twelve: bool) -> String {
    if twelve {
        let h = if c.hour % 12 == 0 { 12 } else { c.hour % 12 };
        format!("{h}{}", if c.hour < 12 { " AM" } else { " PM" })
    } else {
        format!("{:02}:00", c.hour)
    }
}

/// The ticks of the time axis of a range (`1h`, `6h`, `24h`, `7d`, `30d`) that is `from` to `to` (milliseconds): on the hour, or the day, in
/// local time, and no more than `most` of them.
pub fn time_ticks(range: &str, from: f64, to: f64, twelve: bool, most: usize) -> Vec<TimeTick> {
    // (step in minutes, what the label says)
    let (step_min, kind) = match range {
        "1h" => (15u64, 'm'),
        "6h" => (60, 'h'),
        "24h" => (360, 'h'),
        "7d" => (1440, 'd'),
        _ => (7 * 1440, 'n'),
    };
    // local minutes of the day at `from`, to land on the round times of local time
    let c = clock::civil(from as u64);
    let into_day = f64::from(c.hour * 60 + c.minute) * 60_000.0 + f64::from(c.second) * 1000.0;
    let day = 86_400_000.0;
    let step = step_min as f64 * 60_000.0;
    let origin = from - into_day;
    let mut ticks = Vec::new();
    let first = ((from - origin) / step).ceil();
    let mut at = origin + first * step;
    while at <= to + 1.0 {
        let c = clock::civil(at as u64);
        let label = match kind {
            'm' => format!("{}:{:02}", if twelve { if c.hour % 12 == 0 { 12 } else { c.hour % 12 } } else { c.hour }, c.minute),
            'h' => clock_words(&c, twelve),
            'd' => DAYS[c.weekday as usize % 7].to_owned(),
            _ => format!("{} {}", c.day, MONTHS[(c.month as usize + 11) % 12]),
        };
        if at >= from - 1.0 {
            ticks.push(TimeTick { at, label });
        }
        at += if kind == 'd' || kind == 'n' { (step / day).round().max(1.0) * day } else { step };
    }
    while ticks.len() > most.max(2) {
        // every other one
        ticks = ticks.into_iter().enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, t)| t).collect();
    }
    ticks
}

/// The colours the picture is painted in (premultiplied, as the compositor takes them).
#[derive(Clone, Copy, Debug)]
pub struct Colours {
    pub line: Rgba,
    pub fill: Rgba,
    pub grid: Rgba,
    pub mark: Rgba,
    pub halo: Rgba,
}

/// What is plotted: the readings as (time, value) with the times in milliseconds, in a picture `w` by `h` pixels over the range `from` to `to`.
/// The readings are kept `inset` pixels from the picture's edges (room for the marks at the extremes); the grid goes across all of it.
pub struct Plot<'a> {
    pub w: u32,
    pub h: u32,
    pub inset: f32,
    pub from: f64,
    pub to: f64,
    pub axis: &'a Axis,
    pub points: &'a [(f64, f64)],
}

impl Plot<'_> {
    pub fn x(&self, t: f64) -> f32 {
        self.inset + (((t - self.from) / (self.to - self.from).max(1.0)).clamp(0.0, 1.0) as f32) * (self.w as f32 - 1.0 - 2.0 * self.inset)
    }

    pub fn y(&self, v: f64) -> f32 {
        self.inset + self.axis.row(v, self.h as f32 - 1.0 - 2.0 * self.inset)
    }

    /// The curve through the points, a smooth one that never goes above the highest reading or below the lowest between two (a monotone
    /// cubic), at every column the points cover: (column, row).
    pub fn curve(&self) -> Vec<(f32, f32)> {
        let pts: Vec<(f32, f32)> = self.points.iter().map(|&(t, v)| (self.x(t), self.y(v))).collect();
        let Some(&(first, _)) = pts.first() else { return Vec::new() };
        let last = pts.last().map_or(first, |p| p.0);
        if pts.len() == 1 {
            // one reading holds from when it was
            return (first.floor() as u32..self.w).map(|c| (c as f32, pts[0].1)).collect();
        }
        // Fritsch-Carlson slopes
        let n = pts.len();
        let d: Vec<f32> = pts.windows(2).map(|w| if w[1].0 > w[0].0 { (w[1].1 - w[0].1) / (w[1].0 - w[0].0) } else { 0.0 }).collect();
        let mut m = vec![0.0f32; n];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            m[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) / 2.0 };
        }
        for i in 0..n - 1 {
            if d[i].abs() < 1e-9 {
                m[i] = 0.0;
                m[i + 1] = 0.0;
                continue;
            }
            let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * d[i];
                m[i + 1] = t * b * d[i];
            }
        }
        let mut out = Vec::new();
        let mut k = 0usize;
        for c in first.ceil() as u32..=(last.floor() as u32).min(self.w - 1) {
            let x = c as f32;
            while k + 2 < n && pts[k + 1].0 < x {
                k += 1;
            }
            let (x0, y0, x1, y1) = (pts[k].0, pts[k].1, pts[k + 1].0, pts[k + 1].1);
            let hh = (x1 - x0).max(1e-6);
            let t = ((x - x0) / hh).clamp(0.0, 1.0);
            let (t2, t3) = (t * t, t * t * t);
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * hh * m[k] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * hh * m[k + 1];
            out.push((x, y.clamp(self.inset, self.h as f32 - 1.0 - self.inset)));
        }
        out
    }
}

fn blend(px: &mut [u8], w: u32, x: i32, y: i32, c: Rgba, cover: f32) {
    if x < 0 || y < 0 || x >= w as i32 || cover <= 0.0 {
        return;
    }
    let i = ((y as u32 * w + x as u32) * 4) as usize;
    if i + 3 >= px.len() {
        return;
    }
    let a = cover.clamp(0.0, 1.0);
    let src = [f32::from(c.0) * a, f32::from(c.1) * a, f32::from(c.2) * a, f32::from(c.3) * a];
    let inv = 1.0 - src[3] / 255.0;
    for k in 0..4 {
        px[i + k] = (src[k] + f32::from(px[i + k]) * inv).round().min(255.0) as u8;
    }
}

/// Paints the plot: the grid, the fill under the curve, the line, the marks at `marks` (time, value). `line` is the line's width in pixels.
pub fn paint(px: &mut [u8], plot: &Plot, c: &Colours, line: f32, marks: &[(f64, f64)]) {
    let (w, h) = (plot.w, plot.h);
    // gridlines: one pixel, on a whole row
    for &t in &plot.axis.ticks {
        let y = plot.y(t).round() as i32;
        for x in 0..w as i32 {
            blend(px, w, x, y, c.grid, 1.0);
        }
    }
    let curve = plot.curve();
    if curve.is_empty() {
        return;
    }
    // the fill: the accent at the line, none at the foot; the top pixel of each column is covered by how much of it the curve leaves. The
    // fade starts from a moving average of the line, so neighbouring columns share one gradient and a wiggling curve leaves no comb under it
    let reach = (curve.len() / 24).clamp(4, 24);
    let mut sums = vec![0.0f32; curve.len() + 1];
    for (i, &(_, y)) in curve.iter().enumerate() {
        sums[i + 1] = sums[i] + y;
    }
    for (i, &(x, y)) in curve.iter().enumerate() {
        let (col, top) = (x as i32, y);
        let (lo, hi) = (i.saturating_sub(reach), (i + reach + 1).min(curve.len()));
        let anchor = (sums[hi] - sums[lo]) / (hi - lo) as f32;
        let start = top.floor() as i32;
        let depth = (h as f32 - anchor).max(1.0);
        for row in start..h as i32 {
            let cover = if row == start { 1.0 - (top - start as f32) } else { 1.0 };
            let fade = (1.0 - (row as f32 - anchor) / depth).clamp(0.0, 1.0);
            blend(px, w, col, row, c.fill, cover * fade * fade);
        }
    }
    // the line: coverage by the distance of a pixel from the polyline, the most of the segments that reach it
    let half = line / 2.0;
    let mut cover = vec![0.0f32; (w * h) as usize];
    for seg in curve.windows(2) {
        let ((x0, y0), (x1, y1)) = (seg[0], seg[1]);
        let (min_x, max_x) = ((x0.min(x1) - half - 1.0).floor().max(0.0) as i32, (x0.max(x1) + half + 1.0).ceil().min(w as f32 - 1.0) as i32);
        let (min_y, max_y) = ((y0.min(y1) - half - 1.0).floor().max(0.0) as i32, (y0.max(y1) + half + 1.0).ceil().min(h as f32 - 1.0) as i32);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = (dx * dx + dy * dy).max(1e-9);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((fx - x0) * dx + (fy - y0) * dy) / len2).clamp(0.0, 1.0);
                let (px_, py_) = (x0 + t * dx, y0 + t * dy);
                let d = ((fx - px_).powi(2) + (fy - py_).powi(2)).sqrt();
                let a = (half + 0.5 - d).clamp(0.0, 1.0);
                let i = (y as u32 * w + x as u32) as usize;
                if a > cover[i] {
                    cover[i] = a;
                }
            }
        }
    }
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let a = cover[(y as u32 * w + x as u32) as usize];
            if a > 0.0 {
                blend(px, w, x, y, c.line, a);
            }
        }
    }
    // the marks: a dot in a ring of the plot's own ground, so that it stands out of the line it is on
    for &(t, v) in marks {
        let (cx, cy) = (plot.x(t) + 0.5, plot.y(v) + 0.5);
        disc(px, w, h, cx, cy, half + 4.5, c.halo);
        disc(px, w, h, cx, cy, half + 2.5, c.mark);
    }
}

fn disc(px: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, r: f32, c: Rgba) {
    for y in (cy - r - 1.0).floor().max(0.0) as i32..=(cy + r + 1.0).ceil().min(h as f32 - 1.0) as i32 {
        for x in (cx - r - 1.0).floor().max(0.0) as i32..=(cx + r + 1.0).ceil().min(w as f32 - 1.0) as i32 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            blend(px, w, x, y, c, (r + 0.5 - d).clamp(0.0, 1.0));
        }
    }
}

/// The bands of an on-off sensor painted along a bar `w` by `h` pixels with round ends: `on` where it was on, `off` where it was off, `unknown`
/// where nothing was known, over the range `from` to `to`.
#[allow(clippy::too_many_arguments)]
pub fn paint_bands(px: &mut [u8], w: u32, h: u32, from: f64, to: f64, bands: &[(f64, f64, &str)], on: Rgba, off: Rgba, unknown: Rgba) {
    let span = (to - from).max(1.0);
    let r = h as f32 / 2.0;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let t = from + (f64::from(x) + 0.5) / f64::from(w) * span;
            let Some(&(_, _, state)) = bands.iter().find(|b| t >= b.0 && t < b.1) else { continue };
            let c = match state {
                "on" => on,
                "off" => off,
                _ => unknown,
            };
            // the bar's ends are round: the distance to the straight part between the end circles' centres
            let cx = (x as f32 + 0.5).clamp(r, w as f32 - r);
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - r);
            let inside = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            blend(px, w, x, y, c, inside);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_of_a_whole_is_read_from_0_to_100_and_a_temperature_is_padded_and_ticked_round() {
        assert!(is_percent("humidity", "%", 41.0, 55.0));
        assert!(is_percent("", "%", 3.0, 90.0) && !is_percent("", "%", 3.0, 190.0) && !is_percent("temperature", "\u{b0}C", 18.0, 24.0));
        let a = axis(41.0, 55.0, true, 5);
        assert_eq!((a.lo, a.hi, a.ticks.clone()), (0.0, 100.0, vec![0.0, 25.0, 50.0, 75.0, 100.0]));
        assert_eq!(axis(41.0, 55.0, true, 3).ticks, vec![0.0, 50.0, 100.0]);
        let t = axis(19.2, 23.8, false, 5);
        assert!(t.lo <= 19.2 - 0.4 && t.hi >= 23.8 + 0.4, "padded: {t:?}");
        assert!(t.ticks.len() <= 5 && t.ticks.windows(2).all(|w| (w[1] - w[0] - (t.ticks[1] - t.ticks[0])).abs() < 1e-9), "{t:?}");
        assert_eq!(t.ticks.first().copied(), Some(t.lo));
        assert_eq!(t.ticks.last().copied(), Some(t.hi));
        // a line that does not move is not stretched over the plot
        let flat = axis(21.0, 21.0, false, 5);
        assert!(flat.lo < 21.0 && flat.hi > 21.0 && flat.hi - flat.lo >= 1.0, "{flat:?}");
        assert_eq!(tick_text(20.0, 2.0), "20");
        assert_eq!(tick_text(20.5, 0.5), "20.5");
        assert_eq!(tick_text(0.25, 0.05), "0.25");
        // room for two ticks only: the readings padded to a round step of their own range, not the 0 to 50 a coarse step would give
        let two = axis(18.1, 22.9, false, 2);
        assert_eq!((two.lo, two.hi, two.ticks.clone()), (16.0, 24.0, vec![16.0, 24.0]));
        assert_eq!(tick_text(0.5, 1.0), "0.5");
    }

    #[test]
    fn the_time_axis_is_on_the_hour_or_the_day_and_never_has_more_ticks_than_there_is_room_for() {
        let to = 1_800_000_000_000.0;
        for (range, hours) in [("1h", 1.0), ("6h", 6.0), ("24h", 24.0), ("7d", 168.0), ("30d", 720.0)] {
            let from = to - hours * 3_600_000.0;
            for most in [3usize, 5, 8] {
                let t = time_ticks(range, from, to, false, most);
                assert!(!t.is_empty() && t.len() <= most, "{range} {most}: {t:?}");
                assert!(t.iter().all(|k| k.at >= from - 1.0 && k.at <= to + 1.0 && !k.label.is_empty()), "{range}: {t:?}");
                assert!(t.windows(2).all(|w| w[1].at > w[0].at));
            }
        }
        let day = time_ticks("24h", to - 86_400_000.0, to, true, 8);
        assert!(day.iter().all(|k| k.label.ends_with("AM") || k.label.ends_with("PM")), "{day:?}");
        let week = time_ticks("7d", to - 7.0 * 86_400_000.0, to, false, 8);
        assert!(week.iter().all(|k| DAYS.contains(&k.label.as_str())), "{week:?}");
    }

    #[test]
    fn the_curve_is_smooth_never_beyond_the_readings_and_one_reading_holds_from_when_it_was() {
        let axis = Axis { lo: 0.0, hi: 10.0, ticks: vec![0.0, 5.0, 10.0] };
        let pts = [(0.0, 2.0), (100.0, 8.0), (200.0, 8.0), (300.0, 3.0)];
        let plot = Plot { w: 301, h: 101, inset: 0.0, from: 0.0, to: 300.0, axis: &axis, points: &pts };
        let c = plot.curve();
        assert_eq!(c.len(), 301);
        let (lo, hi) = (plot.y(8.0), plot.y(2.0));
        assert!(c.iter().all(|p| p.1 >= lo - 0.01 && p.1 <= hi + 0.01), "no overshoot");
        assert!((c[100].1 - plot.y(8.0)).abs() < 0.01 && (c[0].1 - plot.y(2.0)).abs() < 0.01);
        let one = Plot { points: &[(150.0, 5.0)], ..plot };
        let held = one.curve();
        assert_eq!((held.first().map(|p| p.0), held.last().map(|p| p.0)), (Some(150.0), Some(300.0)));
        assert!(Plot { points: &[], ..one }.curve().is_empty());
    }

    #[test]
    fn the_picture_has_its_grid_on_whole_rows_a_line_that_is_anti_aliased_and_thick_and_marks_where_the_extremes_were() {
        let axis = Axis { lo: 0.0, hi: 10.0, ticks: vec![0.0, 5.0, 10.0] };
        let pts: Vec<(f64, f64)> = (0..=300).map(|t| (f64::from(t), 5.0 + 3.0 * (f64::from(t) / 40.0).sin())).collect();
        let (w, h) = (301u32, 121u32);
        let plot = Plot { w, h, inset: 0.0, from: 0.0, to: 300.0, axis: &axis, points: &pts };
        let c = Colours { line: Rgba(255, 255, 255, 255), fill: Rgba(60, 60, 120, 120), grid: Rgba(20, 20, 20, 40), mark: Rgba(255, 255, 255, 255), halo: Rgba(0, 0, 0, 255) };
        let mut px = vec![0u8; (w * h * 4) as usize];
        paint(&mut px, &plot, &c, 3.0, &[(280.0, plot.axis.hi)]);
        let alpha = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
        // a gridline is a whole row of the same alpha, and not on the rows beside it, where nothing else is
        let row = plot.y(10.0).round() as u32;
        assert!((0..w).all(|x| alpha(x, row) > 0) && (alpha(0, row + 1) == 0 || alpha(0, row + 1) < alpha(0, row)));
        // the line: a column holds fully covered pixels and partly covered ones (anti-aliased), about three pixels deep in all
        let col = 60u32;
        let ys: Vec<u8> = (0..h).map(|y| px[((y * w + col) * 4) as usize]).collect();
        assert!(ys.contains(&255), "a full pixel");
        let line_y = plot.y(5.0 + 3.0 * (60.0f64 / 40.0).sin()).round() as usize;
        let around: f32 = (line_y.saturating_sub(4)..line_y + 4).map(|y| f32::from(ys[y]) / 255.0).sum();
        assert!(around >= 2.5, "at least about three pixels of line: {around}");
        // the fill is under the curve, stronger near it than at the foot
        let near = alpha(col, (line_y + 5) as u32);
        let foot = alpha(col, h - 1);
        assert!(near > foot, "{near} {foot}");
        assert_eq!(alpha(col, 5), 0, "nothing above the line, off the grid's top row");
        // premultiplied: no channel above the alpha
        assert!(px.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }

    #[test]
    fn the_bands_of_an_on_off_sensor_are_painted_where_it_was_on_and_where_it_was_off() {
        let (w, h) = (200u32, 20u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let bands = [(0.0, 100.0, "off"), (100.0, 150.0, "on"), (150.0, 200.0, "off")];
        paint_bands(&mut px, w, h, 0.0, 200.0, &bands, Rgba(255, 0, 0, 255), Rgba(0, 0, 255, 255), Rgba(80, 80, 80, 255));
        let at = |x: u32| &px[(((h / 2) * w + x) * 4) as usize..(((h / 2) * w + x) * 4 + 4) as usize];
        assert_eq!(at(50), &[0, 0, 255, 255]);
        assert_eq!(at(125), &[255, 0, 0, 255]);
        assert_eq!(at(175), &[0, 0, 255, 255]);
        assert_eq!(px[3], 0, "the corner of a bar with round ends");
    }
}
