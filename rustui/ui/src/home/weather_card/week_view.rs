//! The week on the weather card and the day that opens out of it: the words and pictures of each, the plate under the focused day, and the motion between
//! them. The geometry is [`super::super::weather_week`]'s.
//!
//! Everything that moves is a layer the compositor moves: the week (words and picture), the plate and the opened day (words and picture) are separate
//! layers, so a focus move or an open costs the app a few requests and no painting. The day grows out of its row (its layers are clipped to the row and the
//! clip springs out to the room the week had) while the week fades; closing is the same run back. A request starts from wherever the layer is, so a day that is
//! closed while it is still opening turns round without a jump. Under reduced motion the day does not grow, and the two crossfade.

use super::{HIDDEN, Ink, Look, Slot, slot, straight};
use crate::home::weather_view::{hour_label, kind_of, legible};
use crate::home::weather_week::{Panel, Week, day_hours};
use crate::home::widgets::{Canvas, Pictures, Words, art, warm};
use crate::kit::Kit;
use crate::kit::button::faded;
use crate::kit::motion::Beat;
use crate::kit::picture::Picture;
use compositor_protocol::{Ease, Props, Rect, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, Layer, ShapeLayer};
use ui::text::Weight;

/// How long the day takes to grow out of its row and back: the spring's own time.
const MORPH_MS: u32 = 320;

/// What the card has been told, for the week and the day.
pub struct Feed<'a> {
    pub doc: &'a Value,
    pub hours: &'a [Value],
    pub days: &'a [Value],
    pub daily: &'a [Value],
    pub twelve: bool,
}

/// One row of the week: its day's name, the chance of rain, the low and the high.
struct DayLine {
    name: usize,
    rain: Option<usize>,
    lo: usize,
    hi: usize,
}

/// The opened day: its name and range, and per hour's column its time, temperature, chance of rain and wind.
struct Head {
    name: usize,
    range: usize,
    cols: Vec<(usize, usize, usize, Option<usize>)>,
}

struct Open {
    words: Words,
    rows: ImageLayer,
    head: Head,
    /// The row the day's closed pose is clipped to.
    row: Option<usize>,
}

pub struct WeekView {
    week: Week,
    panel: Option<Panel>,
    k: f32,
    /// The card's content box, in pixels.
    px: (f32, f32),
    words: Words,
    rows: ImageLayer,
    lines: Vec<DayLine>,
    plate: ShapeLayer,
    head_plate: ShapeLayer,
    head_on: bool,
    open: Option<Open>,
    at: Props,
    look: Look,
    stamp: String,
    painted_day: Option<usize>,
    /// Reduced motion as the card last saw it: the day crossfades instead of growing.
    reduced: bool,
}

fn go(layer: &Layer, to: Props, ms: u32, ease: Ease) {
    if ms == 0 {
        layer.set(to);
    } else {
        layer.animate(to, ms, ease);
    }
}

fn style_of(kit: &Kit, k: f32, s: &Slot, ink: Rgba, types: &mut Vec<ui::text::Type>) -> (Frame, Style) {
    let mut ty = kit.ty(TypeRole::Label);
    ty.px = (s.px * k).round().max(8.0) as u32;
    ty.weight = s.weight;
    types.push(ty);
    (Frame::new((s.f.x * k).round(), (s.f.y * k).round(), (s.f.w * k).round(), (s.f.h * k).round()), ui::components::style(ty, ink, s.align))
}

fn inks(kit: &Kit) -> impl Fn(Ink) -> Rgba + use<> {
    let (ink, ink2, accent, warn) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), kit.ui.look.warning());
    let light = kit.mode == ui::lucent::palette::Mode::Light;
    move |i| match i {
        Ink::Primary => ink,
        Ink::Secondary => ink2,
        Ink::Accent => legible(accent, ink, light),
        Ink::Warning => legible(warn, ink, light),
    }
}

/// Between two colours, opaque.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t.clamp(0.0, 1.0)).round() as u8;
    Rgba(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2), 255)
}

/// A bar of `h` thick from `a` to `b` along the x axis (`at` is its y) whose colour follows the way along it: `colour` is given 0 at `a`, 1 at `b`.
fn gradient_bar(canvas: &mut Canvas, a: f32, b: f32, at: f32, h: f32, colour: &dyn Fn(f32) -> Rgba) {
    let (lo, hi) = (a.min(b), a.max(b));
    let r = h / 2.0;
    let f = |p: f32| ((p - a) / (b - a)).clamp(0.0, 1.0);
    canvas.disc(lo + r, at + r, r, colour(f(lo + r)));
    canvas.disc(hi - r, at + r, r, colour(f(hi - r)));
    // whole pixels, so that the strips of colour meet without a seam
    let (mut p, end) = ((lo + r).floor(), hi - r);
    while p < end {
        let len = 2.0f32.min(end - p).max(1.0);
        canvas.rounded(Frame::new(p, at, len, h), 0.0, colour(f(p + len / 2.0)));
        p += 2.0;
    }
}

fn rain_of(doc: &Value, daily: &[Value], i: usize) -> Option<i64> {
    let date = doc.pointer(&format!("/daily/{i}/date")).and_then(Value::as_str)?;
    daily.iter().find(|d| d.get("date").and_then(Value::as_str) == Some(date)).and_then(|d| d.get("rainPct")).and_then(Value::as_f64).map(|p| p.round() as i64)
}

fn num(v: Option<&Value>, key: &str) -> Option<f64> {
    v.and_then(|h| h.get(key)).and_then(Value::as_f64)
}

/// How much of the ink the plate under the focused day lays over the card: raised glass, quiet, with no ring.
const PLATE: f32 = 0.12;

impl WeekView {
    /// The layers of a week (and of the day that opens out of it where the room holds one) for a content box of `size` pixels.
    pub fn new(kit: &mut Kit, week: Week, panel: Option<Panel>, head_h: f32, k: f32, size: (f32, f32), z: i32) -> std::io::Result<Self> {
        let (w, h) = size;
        let ink = kit.ui.look.ink();
        let mut types = Vec::new();
        let mut lines = Vec::new();
        let mut spec: Vec<(Frame, Style, String)> = Vec::new();
        let mut put = |kit: &Kit, s: Slot, spec: &mut Vec<(Frame, Style, String)>| -> usize {
            let (f, st) = style_of(kit, k, &s, ink, &mut types);
            spec.push((f, st, String::new()));
            spec.len() - 1
        };
        for i in 0..week.n {
            let (x, y) = (week.frame.x, week.row_y(i));
            let name = put(kit, slot(x, y, week.name_w, week.step, 24.0, Weight::Medium, Align::Left), &mut spec);
            let rain = (week.precip_w > 0.0).then(|| put(kit, slot(week.precip_x(), y, week.precip_w, week.step, 20.0, Weight::Medium, Align::Left), &mut spec));
            let lo = put(kit, slot(week.lo_x(), y, week.num_w, week.step, 24.0, Weight::Medium, Align::Right), &mut spec);
            let hi = put(kit, slot(week.frame.right() - week.num_w, y, week.num_w, week.step, 24.0, Weight::Medium, Align::Right), &mut spec);
            lines.push(DayLine { name, rain, lo, hi });
        }
        let day = panel.map(|pn| {
            let mut spec: Vec<(Frame, Style, String)> = Vec::new();
            let name = put(kit, slot(pn.frame.x, pn.frame.y + 4.0, pn.frame.w * 0.5, 38.0, 28.0, Weight::SemiBold, Align::Left), &mut spec);
            let range = put(kit, slot(pn.frame.right() - 190.0, pn.frame.y + 4.0, 190.0, 38.0, 24.0, Weight::Medium, Align::Right), &mut spec);
            let mut cols = Vec::new();
            for c in 0..pn.cols {
                let (x, cx) = (pn.col_x(c), pn.col_x(c) + pn.col_w / 2.0);
                let time = put(kit, slot(x, pn.time_y, pn.col_w, 26.0, 20.0, Weight::Medium, Align::Centre), &mut spec);
                let temp = put(kit, slot(x, pn.temp_y, pn.col_w, 34.0, pn.temp_px, Weight::SemiBold, Align::Centre), &mut spec);
                let rain = put(kit, slot(cx - 6.0, pn.rain_y, 38.0, 24.0, 20.0, Weight::Medium, Align::Left), &mut spec);
                let wind = pn.wind_y.map(|y| put(kit, slot(cx - 6.0, y, 38.0, 24.0, 20.0, Weight::Medium, Align::Left), &mut spec));
                cols.push((time, temp, rain, wind));
            }
            (spec, Head { name, range, cols })
        });
        warm(kit, std::mem::take(&mut types));
        let words = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 3, spec)?;
        let rows = art(kit, Frame::new(0.0, 0.0, w, h), z + 2)?;
        let open = match day {
            Some((spec, head)) => Some(Open { words: Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 3, spec)?, rows: art(kit, Frame::new(0.0, 0.0, w, h), z + 2)?, head, row: None }),
            None => None,
        };
        let radius = kit.tokens.radius(Radius::Control);
        let glass = plate::solid(ink, radius);
        let plate_h = ((week.step - 4.0) * k).round().max(1.0);
        let plate = kit.ui.scene.shape(w.round().max(1.0) as u32, plate_h as u32, z + 1, glass, HIDDEN);
        let head_plate = kit.ui.scene.shape(w.round().max(1.0) as u32, (head_h * k).round().max(1.0) as u32, z + 1, glass, HIDDEN);
        Ok(Self { week, panel, k, px: (w, h), words, rows, lines, plate, head_plate, head_on: false, open, at: HIDDEN, look: Look::default(), stamp: String::new(), painted_day: None, reduced: kit.ui.look.reduced_motion })
    }

    /// Layers the week holds on the screen: the words and the picture, a plate while the reading or a day is focused, and the day's two while one is open.
    #[cfg(test)]
    pub fn visible_layers(&self) -> usize {
        if self.opened() { 2 } else { 2 + usize::from(self.focus().is_some() || self.head_on) }
    }

    fn opened(&self) -> bool {
        self.open.is_some() && self.look.open.is_some_and(|d| d < self.week.n)
    }

    fn focus(&self) -> Option<usize> {
        self.look.focus.filter(|f| *f < self.week.n)
    }

    fn row_rect(&self, i: usize) -> Rect {
        let k = self.k;
        Rect { x: self.at.x, y: self.at.y + (self.week.row_y(i) + 2.0) * k, w: self.px.0, h: (self.week.step - 4.0) * k }
    }

    fn day_rect(&self) -> Rect {
        let f = self.panel.map_or(self.week.frame, |p| p.frame);
        Rect { x: self.at.x, y: self.at.y + f.y * self.k, w: self.px.0, h: f.h * self.k }
    }

    /// The plate under the reading while the full-screen weather is what OK would open.
    fn head_pose(&self) -> Props {
        let at = self.at;
        if self.head_on { Props { opacity: at.opacity * PLATE, ..at } } else { Props { opacity: 0.0, visible: false, ..at } }
    }

    /// The poses of the week's layers, the plate's and the day's, for the day closed or open; `reduced` keeps the day's clip at its own room.
    fn poses(&self, open: bool, reduced: bool) -> (Props, Props, Props) {
        let at = self.at;
        let gone = Props { opacity: 0.0, visible: false, ..at };
        let row = self.focus();
        let plate = match (open, row) {
            (false, Some(i)) => {
                let r = self.row_rect(i);
                Props { x: r.x, y: r.y, opacity: at.opacity * PLATE, ..at }
            }
            (_, i) => {
                let r = self.row_rect(i.unwrap_or(0));
                Props { x: r.x, y: r.y, ..gone }
            }
        };
        let closed_clip = match (reduced, row) {
            (false, Some(i)) => self.row_rect(i),
            _ => self.day_rect(),
        };
        if open {
            (gone, plate, Props { clip: Some(self.day_rect()), ..at })
        } else {
            (at, plate, Props { clip: Some(closed_clip), ..gone })
        }
    }

    /// The layers are put where the card is. Nothing moves while the card is where it was, so an open or a close in flight is left alone.
    pub fn show(&mut self, at: Props) {
        if self.at == at {
            return;
        }
        self.at = at;
        let (week, plate, day) = self.poses(self.opened(), self.reduced);
        self.words.set_props(week);
        self.rows.set(week);
        self.plate.set(plate);
        self.head_plate.set(self.head_pose());
        if let Some(o) = &mut self.open {
            o.words.set_props(day);
            o.rows.set(day);
            o.row = self.look.focus;
        }
    }

    /// The week and the day as `feed` and `look` have them. `stamp` says whether `feed` is what it was.
    pub fn set(&mut self, kit: &mut Kit, pictures: &mut Pictures, feed: &Feed, stamp: &str, look: Look) {
        let fresh = self.stamp != stamp;
        let day = look.open.filter(|d| self.open.is_some() && *d < self.week.n);
        if fresh {
            self.stamp = stamp.to_owned();
            self.words_of_week(kit, feed);
            self.paint_week(kit, pictures, feed);
        }
        if let Some(d) = day.filter(|d| fresh || self.painted_day != Some(*d)) {
            self.words_of_day(kit, feed, d);
            self.paint_day(kit, pictures, feed, d);
            self.painted_day = Some(d);
        }
        let (was_open, was_focus) = (self.opened(), self.focus());
        self.look = look;
        self.reduced = kit.ui.look.reduced_motion;
        self.glide(kit, was_open, was_focus);
    }

    /// Takes the layers to where the new look puts them: the plate to its day, the day out of its row or back into it.
    fn glide(&mut self, kit: &Kit, was_open: bool, was_focus: Option<usize>) {
        if self.at == HIDDEN {
            return;
        }
        let (open, focus) = (self.opened(), self.focus());
        let reduced = kit.ui.look.reduced_motion;
        let (week, plate, day) = self.poses(open, reduced);
        let fade = kit.beat_ms(Beat::Layer);
        let (morph, curve) = if reduced { (kit.beat_ms(Beat::Label), Ease::Linear) } else { (MORPH_MS, Ease::Spring) };
        let head = self.look.head && !open;
        if head != self.head_on {
            self.head_on = head;
            go(&self.head_plate, self.head_pose(), kit.beat_ms(Beat::Card), Ease::Out);
        }
        if open != was_open {
            if open {
                // the day starts as the row it grows out of, unless it is still on its way back into that row
                let from = focus.map(|i| Props { clip: Some(if reduced { self.day_rect() } else { self.row_rect(i) }), opacity: 0.0, visible: false, ..self.at });
                if let (Some(o), Some(i), Some(closed)) = (&mut self.open, focus, from) {
                    if o.row != Some(i) {
                        o.words.set_props(closed);
                        o.rows.set(closed);
                        o.row = Some(i);
                    }
                }
            }
            if let Some(o) = &mut self.open {
                o.words.animate_props(day, morph, curve);
                go(&o.rows, day, morph, curve);
                o.row = focus.or(o.row);
            }
            self.words.animate_props(week, fade, Ease::Out);
            go(&self.rows, week, fade, Ease::Out);
            go(&self.plate, plate, fade, Ease::Out);
            return;
        }
        if open {
            // a day opened next to this one: the plate waits, hidden, at its new row
            self.plate.set(plate);
            return;
        }
        match (was_focus, focus) {
            (None, Some(_)) => {
                self.plate.set(Props { opacity: 0.0, ..plate });
                go(&self.plate, plate, fade, Ease::Out);
            }
            (_, None) => go(&self.plate, plate, fade, Ease::Out),
            (Some(a), Some(b)) if a != b => go(&self.plate, plate, kit.beat_ms(Beat::Card).max(morph / 2), curve),
            _ => {}
        }
    }

    fn words_of_week(&mut self, kit: &mut Kit, feed: &Feed) {
        let colour = inks(kit);
        let mut changes: Vec<(usize, String, Ink)> = Vec::new();
        for (i, ln) in self.lines.iter().enumerate() {
            let dv = feed.days.get(i);
            let ds = |key: &str| dv.and_then(|v| v.get(key)).and_then(Value::as_str).unwrap_or("").to_owned();
            changes.push((ln.name, ds("day"), Ink::Primary));
            if let Some(r) = ln.rain {
                let p = rain_of(feed.doc, feed.daily, i).filter(|p| *p >= 10);
                changes.push((r, p.map(|p| format!("{p}%")).unwrap_or_default(), if p.is_some_and(|p| p >= 40) { Ink::Accent } else { Ink::Secondary }));
            }
            changes.push((ln.lo, ds("lo"), Ink::Secondary));
            changes.push((ln.hi, ds("hi"), Ink::Primary));
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, t, c)| (*i, t.as_str(), colour(*c))).collect();
        self.words.set_many(kit, &refs);
    }

    fn words_of_day(&mut self, kit: &mut Kit, feed: &Feed, day: usize) {
        let colour = inks(kit);
        let Some(o) = &mut self.open else { return };
        let dv = feed.days.get(day);
        let ds = |key: &str| dv.and_then(|v| v.get(key)).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut changes: Vec<(usize, String, Ink)> = vec![(o.head.name, ds("day"), Ink::Primary), (o.head.range, format!("H {}  L {}", ds("hi"), ds("lo")), Ink::Secondary)];
        let date = feed.doc.pointer(&format!("/daily/{day}/date")).and_then(Value::as_str).unwrap_or("");
        let hours = day_hours(feed.hours, date, o.head.cols.len());
        for (c, (t, tp, r, wd)) in o.head.cols.iter().enumerate() {
            let hv = hours.get(c).copied();
            changes.push((*t, hv.map(|h| hour_label(h.get("time").and_then(Value::as_str).unwrap_or(""), feed.twelve)).unwrap_or_default(), Ink::Secondary));
            changes.push((*tp, num(hv, "temp").map(|t| format!("{}\u{b0}", t.round() as i64)).unwrap_or_default(), Ink::Primary));
            changes.push((*r, num(hv, "rainPct").map(|p| format!("{}", p.round() as i64)).unwrap_or_default(), if num(hv, "rainPct").is_some_and(|p| p >= 40.0) { Ink::Accent } else { Ink::Secondary }));
            if let Some(wd) = wd {
                changes.push((*wd, num(hv, "windMph").map(|m| format!("{}", m.round() as i64)).unwrap_or_default(), Ink::Secondary));
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, t, c)| (*i, t.as_str(), colour(*c))).collect();
        o.words.set_many(kit, &refs);
    }

    /// The week's pictures: each day's weather and its range on the week's one scale.
    fn paint_week(&mut self, kit: &mut Kit, pictures: &mut Pictures, feed: &Feed) {
        let (k, wk) = (self.k, self.week);
        let (ink, accent, warn) = (kit.ui.look.ink(), kit.ui.look.accent(), kit.ui.look.warning());
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        let wash = |a: f32| straight(faded(ink, a));
        let deg = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).and_then(|t| t.trim_end_matches('\u{b0}').parse::<f32>().ok());
        let (w, h) = (self.px.0 as u32, self.px.1 as u32);
        self.rows.clear();
        let mut canvas = Canvas { px: self.rows.pixels_mut(), w, h };
        let (mut lo_min, mut hi_max) = (f32::MAX, f32::MIN);
        for dv in feed.days.iter().take(wk.n) {
            if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi")) {
                lo_min = lo_min.min(lo);
                hi_max = hi_max.max(hi);
            }
        }
        let span = (hi_max - lo_min).max(1.0);
        let tone = |t: f32| mix(accent, warn, (t - lo_min) / span);
        let now = feed.doc.pointer("/current/temp").and_then(Value::as_f64).map(|t| t as f32);
        for (i, dv) in feed.days.iter().enumerate().take(wk.n) {
            let kind = dv.get("kind").and_then(Value::as_str).unwrap_or("");
            let (y, rh) = (wk.row_y(i) * k, wk.step * k);
            let ic = (wk.icon * k).round() as u32;
            if let Some(pic) = pictures.get_look(&format!("wxbig-{kind}"), light) {
                canvas.picture(pic, ((wk.frame.x + wk.name_w) * k).round() as i32, (y + (rh - ic as f32) / 2.0).round() as i32, ic, ic);
            }
            let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi")) else { continue };
            if !wk.bar || lo_min > hi_max {
                continue;
            }
            let (b0, b1) = wk.bar_span();
            let (bx, bw, bh) = (b0 * k, (b1 - b0) * k, 8.0 * k);
            let by = y + (rh - bh) / 2.0;
            canvas.rounded(Frame::new(bx, by, bw, bh), bh / 2.0, wash(0.12));
            let (x0, x1) = (bx + bw * (lo - lo_min) / span, (bx + bw * (hi - lo_min) / span).max(bx + bw * (lo - lo_min) / span + bh));
            gradient_bar(&mut canvas, x0, x1, by, bh, &|f| tone(lo + (hi - lo) * f));
            if let (0, Some(t)) = (i, now) {
                let tx = (bx + bw * (t - lo_min) / span).clamp(x0 + bh / 2.0, x1 - bh / 2.0);
                canvas.disc(tx, by + bh / 2.0, bh * 0.95, ink);
                canvas.disc(tx, by + bh / 2.0, bh * 0.5, tone(t));
            }
        }
        self.rows.damage(None);
    }

    /// The opened day's pictures: its weather, and per hour's column its weather, the bar of its rain and the glyphs for rain and wind.
    fn paint_day(&mut self, kit: &mut Kit, pictures: &mut Pictures, feed: &Feed, day: usize) {
        let (k, Some(pn)) = (self.k, self.panel) else { return };
        let Some(o) = &mut self.open else { return };
        let (ink, ink2, accent) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent());
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        let wash = |a: f32| straight(faded(ink, a));
        let (w, h) = (self.px.0 as u32, self.px.1 as u32);
        o.rows.clear();
        let mut canvas = Canvas { px: o.rows.pixels_mut(), w, h };
        let glyph = |kit: &mut Kit, canvas: &mut Canvas, name: &str, side: f32, x: f32, y: f32, c: Rgba| {
            let s = side.round() as u32;
            if let Some(rgba) = kit.icons.draw(name, s, c) {
                canvas.picture(&Picture { width: s, height: s, rgba }, x.round() as i32, y.round() as i32, s, s);
            }
        };
        // the horizon under the reading is the card's, drawn once for the week and the day alike
        let f = pn.frame;
        canvas.rounded(Frame::new(f.x * k, (f.y + pn.head_h - 2.0) * k, f.w * k, k.max(1.0)), 0.0, wash(0.10));
        if let Some(kind) = feed.days.get(day).and_then(|dv| dv.get("kind")).and_then(Value::as_str) {
            let side = (34.0 * k).round() as u32;
            if let Some(pic) = pictures.get_look(&format!("wxbig-{kind}"), light) {
                canvas.picture(pic, ((f.right() - 190.0 - 46.0) * k).round() as i32, ((f.y + 6.0) * k).round() as i32, side, side);
            }
        }
        let date = feed.doc.pointer(&format!("/daily/{day}/date")).and_then(Value::as_str).unwrap_or("");
        for (c, hv) in day_hours(feed.hours, date, pn.cols).into_iter().enumerate() {
            let cx = (pn.col_x(c) + pn.col_w / 2.0) * k;
            let kind = kind_of(hv.get("code").and_then(Value::as_i64).unwrap_or(0), hv.get("isDay").and_then(Value::as_bool).unwrap_or(true));
            let side = (pn.icon * k).round() as u32;
            if let Some(pic) = pictures.get_look(&format!("wxbig-{kind}"), light) {
                canvas.picture(pic, (cx - side as f32 / 2.0).round() as i32, (pn.icon_y * k).round() as i32, side, side);
            }
            let prob = (hv.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
            if let Some((top, bh)) = pn.bar {
                let bw = 18.0 * k;
                canvas.rounded(Frame::new(cx - bw / 2.0, top * k, bw, bh * k), bw / 2.0, wash(0.10));
                if prob >= 0.04 {
                    let fh = (bh * k * prob).max(bw * 0.5);
                    canvas.rounded(Frame::new(cx - bw / 2.0, (top + bh) * k - fh, bw, fh), bw / 2.0, accent);
                }
            }
            glyph(kit, &mut canvas, "droplets", 16.0 * k, cx - 28.0 * k, (pn.rain_y + 4.0) * k, if prob >= 0.4 { accent } else { ink2 });
            if let Some(y) = pn.wind_y {
                glyph(kit, &mut canvas, "wind", 16.0 * k, cx - 28.0 * k, (y + 4.0) * k, ink2);
            }
        }
        o.rows.damage(None);
    }

    /// The words that are on the screen, for the lint: the week's, or the opened day's while one is open (the other is faded out and its words share the room).
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, name: &str, origin: (f32, f32), panel: Option<usize>) {
        match (&self.open, self.opened()) {
            (Some(o), true) => o.words.lint(kit, lint, &format!("{name}.day"), origin, panel),
            _ => self.words.lint(kit, lint, &format!("{name}.week"), origin, panel),
        }
    }
}
