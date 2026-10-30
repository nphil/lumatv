//! The weather card (`widgets/WeatherWidget.qml`, redesigned for the room it has): the sky of the day drawn behind the reading, and under it what
//! to know from the couch at a glance: the place, the temperature large, the condition, how it feels with the day's high and low, one line that
//! says what is next (rain from four, a watch in force, the sunset), the next hours with their weather and their chance of rain, and, where the
//! card is tall or wide enough, the week as rows with their range bars.
//!
//! The sky is the card's picture ([`super::weather_art`]): a tint over the glass from the top, the sprites the compositor moves, and a hairline
//! under the reading where the rain lands. The card's words are one block (one layer, glyph quads) and its pictures one image layer painted on
//! the CPU (icons, bars), so a card costs the compositor a handful of layers besides the sky's.
//!
//! The plan ([`plan`]) is pure: from the box it has (in the text size's reduction, as the clock does it) and from what data there is (the hours or
//! not), it decides which of the parts there is room for and where each goes.

use super::weather_art::{Cond, Live, Stage};
use super::weather_view::{SmartTone, hour_label, kind_of, legible, smart_line};
use super::widgets::{Canvas, PAD_SIDE, PAD_TOP, Pictures, Words, art, warm};
use crate::kit::Kit;
use crate::kit::button::faded;
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::text::ellipsise;
use ui::components::{Align, Style};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

/// What data the card has to show, which decides its plan (the card is planned again when it changes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Data {
    /// The next hours are there.
    pub hours: bool,
    /// Some of the first of them have a chance of rain worth a bar.
    pub bars: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    Primary,
    Secondary,
    Accent,
    Warning,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    f: Frame,
    px: f32,
    weight: Weight,
    align: Align,
}

fn slot(x: f32, y: f32, w: f32, h: f32, px: f32, weight: Weight, align: Align) -> Slot {
    Slot { f: Frame::new(x, y, w, h), px, weight, align }
}

#[derive(Clone, Copy, Debug)]
struct Hours {
    x: f32,
    y: f32,
    col_w: f32,
    n: usize,
    icon: f32,
    /// The rows of a column, from `y`: the icon, the temperature and (when there are bars) the bar's top and its height; the size of the temperature.
    icon_y: f32,
    temp_y: f32,
    temp_px: f32,
    bar: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug)]
struct Days {
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    rows: usize,
    day_w: f32,
    icon: f32,
    num_w: f32,
}

/// Where everything goes, in the content box at the text size's reduction (the caller scales it up by the text size).
#[derive(Clone, Debug)]
struct Plan {
    w: f32,
    wide: bool,
    hero_w: f32,
    hero_bottom: f32,
    place: Option<Slot>,
    temp: Slot,
    cond: Slot,
    feels: Option<Slot>,
    smart: Option<Slot>,
    hours: Option<Hours>,
    days: Option<Days>,
}

const SMART_H: f32 = 34.0;
const HOURS_H: f32 = 106.0;
const HOURS_BARS_H: f32 = 146.0;
const DAY_H: f32 = 46.0;

/// Which parts there is room for in a box of `w` by `h` (logical pixels) and where each goes.
fn plan(w: f32, h: f32, data: Data) -> Plan {
    let wide = w >= 620.0;
    let hero_w = if wide { (w * 0.42).round().clamp(320.0, 430.0) } else { w };
    // the reading, as large as the height gives and the width lets a "-12°" be
    let (temp_px, place_on, feels_on) = if h >= 392.0 {
        (112.0, true, true)
    } else if h >= 318.0 {
        (96.0, true, true)
    } else if h >= 250.0 {
        (84.0, true, false)
    } else if h >= 190.0 {
        (72.0, true, false)
    } else if h >= 150.0 {
        (72.0, false, false)
    } else {
        (((h - 30.0) * 0.62).clamp(36.0, 64.0), false, false)
    };
    let temp_px = temp_px.min(((hero_w - 8.0) / 2.1).floor()).max(36.0);
    let mut y = 0.0;
    let place = place_on.then(|| {
        let s = slot(0.0, y, hero_w, 30.0, 24.0, Weight::SemiBold, Align::Left);
        y += 28.0;
        s
    });
    let temp = slot(0.0, y, hero_w, temp_px, temp_px, Weight::SemiBold, Align::Left);
    y += temp_px + 2.0;
    let cond = slot(0.0, y, hero_w, 36.0, if temp_px >= 96.0 { 30.0 } else { 26.0 }, Weight::Medium, Align::Left);
    y += 36.0;
    let feels = feels_on.then(|| {
        let s = slot(0.0, y, hero_w, 30.0, 22.0, Weight::Medium, Align::Left);
        y += 30.0;
        s
    });
    let hero_bottom = y;
    // under the reading (or beside it, on a wide card): the next thing, the hours and the week, as many as there is room for
    let (x0, y0, bw, bh) = if wide { (hero_w + 44.0, 0.0, w - hero_w - 44.0, h) } else { (0.0, hero_bottom + 14.0, w, h - hero_bottom - 14.0) };
    let mut y = y0;
    let mut left = bh;
    let mut smart = None;
    if left >= SMART_H {
        // a card as narrow as the largest text size makes the default one says it a little smaller
        smart = Some(slot(x0, y, bw, SMART_H, if bw < 230.0 { 21.0 } else { 26.0 }, Weight::SemiBold, Align::Left));
        y += SMART_H + 10.0;
        left -= SMART_H + 10.0;
    }
    let mut hours = None;
    let n = ((bw / 70.0).floor() as usize).min(8);
    if data.hours && n >= 3 && smart.is_some() && left >= HOURS_H {
        let col_w = (bw / n as f32).floor();
        // with the room for them, the bars of the rain; where there is no rain to show, the same room for larger pictures; tight, the plain strip
        let (hh, h) = if left >= HOURS_BARS_H {
            let h = if data.bars {
                Hours { x: x0, y, col_w, n, icon: 40.0, icon_y: y + 30.0, temp_y: y + 72.0, temp_px: 26.0, bar: Some((y + 112.0, 34.0)) }
            } else {
                Hours { x: x0, y, col_w, n, icon: 48.0, icon_y: y + 34.0, temp_y: y + 90.0, temp_px: 30.0, bar: None }
            };
            (HOURS_BARS_H, h)
        } else {
            (HOURS_H, Hours { x: x0, y, col_w, n, icon: 40.0, icon_y: y + 30.0, temp_y: y + 72.0, temp_px: 26.0, bar: None })
        };
        hours = Some(h);
        y += hh + 14.0;
        left -= hh + 14.0;
    }
    let rows = ((left / DAY_H).floor() as usize).min(7);
    let days = (rows >= 2 && smart.is_some()).then(|| Days { x: x0, y, w: bw, row_h: DAY_H, rows, day_w: 70.0, icon: 32.0, num_w: 52.0 });
    Plan { w, wide, hero_w, hero_bottom, place, temp, cond, feels, smart, hours, days }
}

/// The text of a line, and whose ink it is in.
type Change = (usize, String, Ink);

/// Which lines of the block are which.
#[derive(Default)]
struct Lines {
    place: Option<usize>,
    temp: usize,
    cond: usize,
    feels: Option<usize>,
    smart: Option<usize>,
    /// Per hour: its time and its temperature.
    hours: Vec<(usize, usize)>,
    /// Per day: its name, its low and its high.
    days: Vec<(usize, usize, usize)>,
    styles: Vec<Style>,
}

pub struct Weather {
    pub block: Words,
    rows: ImageLayer,
    /// The content box, in pixels, and the text size.
    w: f32,
    h: f32,
    k: f32,
    plan: Plan,
    data: Data,
    lines: Lines,
    at: Props,
    shown: Option<String>,
    /// The sky: the picture of the condition (the sprites in the card's group), the tint behind it and the stage they are made for (box and hero, in pixels).
    art: Option<Live>,
    wash: Option<(ShapeLayer, Cond, bool)>,
    stage: Stage,
    art_waits: bool,
    z: i32,
}

/// The card's box in the content box's coordinates: the padding is the card's own, so the sky reaches the card's edges.
fn sky_box(plan: &Plan, k: f32, w_px: f32) -> Stage {
    let card_w = w_px + 2.0 * PAD_SIDE;
    // a wide card has the sky over the reading only; the week beside it is on the plain glass
    let sky_w = if plan.wide { (PAD_SIDE + plan.hero_w * k + 24.0).min(card_w) } else { card_w };
    let sky_h = (PAD_TOP + plan.hero_bottom * k + 12.0).round();
    let size = (sky_h * 0.84).min(sky_w * 0.47).clamp(72.0, 260.0).round();
    let hero = ((sky_w - size * 0.5 - 8.0).round(), (sky_h * 0.45).round());
    Stage { w: sky_w, h: sky_h, hero, size, ground: sky_h - 5.0, fade: [14.0, 14.0, 10.0, 26.0], shy: 0.5 }
}

impl Weather {
    pub fn new(kit: &mut Kit, w: f32, h: f32, z: i32, _theme: &Value) -> std::io::Result<Self> {
        let k = kit.text_scale.max(1.0);
        let data = Data::default();
        let p = plan(w / k, h / k, data);
        let (block, rows, lines) = Self::make(kit, &p, k, w, h, z)?;
        let stage = sky_box(&p, k, w);
        Ok(Self { block, rows, w, h, k, plan: p, data, lines, at: HIDDEN, shown: None, art: None, wash: None, stage, art_waits: false, z })
    }

    /// The words and the picture of a plan.
    fn make(kit: &mut Kit, p: &Plan, k: f32, w: f32, h: f32, z: i32) -> std::io::Result<(Words, ImageLayer, Lines)> {
        let ink = kit.ui.look.ink();
        let types: std::cell::RefCell<Vec<ui::text::Type>> = std::cell::RefCell::new(Vec::new());
        let sized = |kit: &Kit, s: &Slot| -> Style {
            let mut ty = kit.ty(TypeRole::Label);
            ty.px = (s.px * k).round().max(8.0) as u32;
            ty.weight = s.weight;
            types.borrow_mut().push(ty);
            ui::components::style(ty, ink, s.align)
        };
        let mut lines: Vec<(Frame, Style, String)> = Vec::new();
        let mut map = Lines::default();
        let push = |kit: &Kit, s: &Slot, lines: &mut Vec<(Frame, Style, String)>| -> usize {
            let st = sized(kit, s);
            lines.push((Frame::new(s.f.x * k, s.f.y * k, s.f.w * k, s.f.h * k), st, String::new()));
            lines.len() - 1
        };
        if let Some(s) = &p.place {
            map.place = Some(push(kit, s, &mut lines));
        }
        map.temp = push(kit, &p.temp, &mut lines);
        map.cond = push(kit, &p.cond, &mut lines);
        if let Some(s) = &p.feels {
            map.feels = Some(push(kit, s, &mut lines));
        }
        if let Some(s) = &p.smart {
            map.smart = Some(push(kit, s, &mut lines));
        }
        if let Some(hr) = &p.hours {
            for i in 0..hr.n {
                let x = hr.x + i as f32 * hr.col_w;
                let t = push(kit, &slot(x, hr.y, hr.col_w, 26.0, 22.0, Weight::Medium, Align::Centre), &mut lines);
                let v = push(kit, &slot(x, hr.temp_y, hr.col_w, hr.temp_px * 1.25, hr.temp_px, Weight::SemiBold, Align::Centre), &mut lines);
                map.hours.push((t, v));
            }
        }
        if let Some(d) = &p.days {
            for i in 0..d.rows {
                let y = d.y + i as f32 * d.row_h;
                let name = push(kit, &slot(d.x, y, d.day_w, d.row_h, 23.0, Weight::Medium, Align::Left), &mut lines);
                let lo = push(kit, &slot(d.x + d.day_w + d.icon + 10.0, y, d.num_w, d.row_h, 23.0, Weight::Medium, Align::Right), &mut lines);
                let hi = push(kit, &slot(d.x + d.w - d.num_w, y, d.num_w, d.row_h, 23.0, Weight::Medium, Align::Right), &mut lines);
                map.days.push((name, lo, hi));
            }
        }
        warm(kit, types.into_inner());
        map.styles = lines.iter().map(|l| l.1).collect();
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 3, lines)?;
        let rows = art(kit, Frame::new(0.0, 0.0, w, h), z + 2)?;
        Ok((block, rows, map))
    }

    /// The card is planned again for what data there is: its words and picture are made anew (the caller is inside the card's `adopting`).
    fn replan(&mut self, kit: &mut Kit, data: Data) -> std::io::Result<()> {
        let p = plan(self.w / self.k, self.h / self.k, data);
        let (block, rows, lines) = Self::make(kit, &p, self.k, self.w, self.h, self.z)?;
        self.block = block;
        self.rows = rows;
        self.lines = lines;
        self.stage = sky_box(&p, self.k, self.w);
        self.plan = p;
        self.data = data;
        self.shown = None;
        // the sky is made for the box the plan gives it: again
        self.art = None;
        self.wash = None;
        self.block.set_props(self.at);
        self.rows.set(self.at);
        Ok(())
    }

    pub fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        self.rows.set(at);
        if let Some((w, _, _)) = &self.wash {
            w.set(Props { x: at.x - PAD_SIDE, y: at.y - PAD_TOP, ..at });
        }
        if let Some(a) = &mut self.art {
            a.place((at.x - PAD_SIDE, at.y - PAD_TOP));
        }
    }

    pub fn art_waits(&self) -> bool {
        self.art_waits
    }

    /// Keeps the picture's motion going (`run`) or at rest, for the tick `now`; the tick the next paths are due at, while it moves.
    pub fn animate(&mut self, now: u64, run: bool) -> Option<u64> {
        let a = self.art.as_mut()?;
        if !run {
            if a.running() {
                a.rest();
            }
            return None;
        }
        Some(a.step(now))
    }

    /// The sky: its sprites and the tint behind them are made when the condition (or the look, or the box) changes. The caller is inside the card's
    /// `adopting`, so that the layers hang under the card's group.
    pub fn set_art(&mut self, kit: &mut Kit, topic: &Value) {
        // the words first: the sprites' pixels are a few hundred KiB the engine takes in a frame of their own, and the card's glyphs are
        // not to wait behind them (the picture comes on the first apply after the atlas has settled)
        if kit.ui.text_atlas().is_some_and(|a| !a.is_settled()) {
            self.art_waits = true;
            return;
        }
        self.art_waits = false;
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        match Cond::from_doc(topic.pointer("/doc").unwrap_or(&Value::Null)) {
            Some(cond) if !self.art.as_ref().is_some_and(|a| a.is(cond, self.stage, light)) => {
                let origin = (self.at.x - PAD_SIDE, self.at.y - PAD_TOP);
                self.art = Live::new(kit, cond, self.stage, light, origin, self.z + 1).ok();
                // the tint behind the sprites: the card's top corners, from the colour of the sky to nothing at the foot of the picture
                if !self.wash.as_ref().is_some_and(|(_, c, l)| *c == cond && *l == light) {
                    let (rgb, a) = cond.wash(light);
                    let top = Rgba((rgb[0] as f32 * a).round() as u8, (rgb[1] as f32 * a).round() as u8, (rgb[2] as f32 * a).round() as u8, (a * 255.0).round() as u8);
                    let shape = Shape { corner_radius: kit.tokens.radius(Radius::Card), fill: Fill::Linear { from: top, to: Rgba(0, 0, 0, 0), angle_deg: 90.0 }, border_width: 0.0, border: Rgba(0, 0, 0, 0) };
                    let props = Props { x: self.at.x - PAD_SIDE, y: self.at.y - PAD_TOP, ..self.at };
                    self.wash = Some((kit.ui.scene.shape(self.stage.w as u32, (self.stage.h + 20.0) as u32, self.z, shape, props), cond, light));
                }
            }
            None => {
                self.art = None;
                self.wash = None;
            }
            Some(_) => {}
        }
    }

    /// The helper's reading (`topic.doc`) and the details (`more.doc`, the hours and the alerts), drawn when either changed. The caller is inside the
    /// card's `adopting` (a change of what data there is plans the card again).
    pub fn set(&mut self, kit: &mut Kit, pictures: &mut Pictures, topic: &Value, more: &Value, twelve: bool) {
        let key = format!("{}|{}|{twelve}|{}", topic.pointer("/doc/fetchedAt").map(Value::to_string).unwrap_or_default(), more.pointer("/doc/fetchedAt").map(Value::to_string).unwrap_or_default(), self.k);
        if self.shown.as_deref() == Some(key.as_str()) {
            return;
        }
        let hours: Vec<Value> = more.pointer("/doc/hourly").and_then(Value::as_array).cloned().unwrap_or_default();
        let data = Data { hours: hours.len() >= 4, bars: hours.iter().take(8).any(|h| h.get("rainPct").and_then(Value::as_f64).is_some_and(|p| p >= 15.0)) };
        if data != self.data && self.replan(kit, data).is_err() {
            return;
        }
        self.shown = Some(key);
        let doc = topic.pointer("/doc").unwrap_or(&Value::Null);
        let d = doc.get("display").cloned().unwrap_or(Value::Null);
        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let k = self.k;
        let l = &self.lines;
        let mut changes: Vec<Change> = Vec::new();
        // the reading: no reading yet means the helper is starting again or not up, which the card says instead of staying empty
        let place = if d.is_null() { "Reconnecting\u{2026}".to_owned() } else { s("place") };
        if let Some(i) = l.place {
            changes.push((i, ellipsise(&mut kit.ui.fonts, &place, l.styles[i].ty, self.plan.hero_w * k), Ink::Primary));
        }
        changes.push((l.temp, s("temp"), Ink::Primary));
        changes.push((l.cond, ellipsise(&mut kit.ui.fonts, &s("cond"), l.styles[l.cond].ty, self.plan.hero_w * k), Ink::Primary));
        let days: Vec<Value> = d.get("days").and_then(Value::as_array).cloned().unwrap_or_default();
        if let Some(i) = l.feels {
            let feels = doc.pointer("/current/feelsLike").and_then(Value::as_f64).map(|f| format!("Feels like {}\u{b0}", f.round() as i64));
            let hl = days.first().map(|d0| format!("H {}  L {}", d0.get("hi").and_then(Value::as_str).unwrap_or(""), d0.get("lo").and_then(Value::as_str).unwrap_or("")));
            let text = match (feels, hl) {
                (Some(f), Some(h)) => format!("{f}  \u{b7}  {h}"),
                (Some(t), None) | (None, Some(t)) => t,
                (None, None) => s("stats").split(" \u{b7} ").next().unwrap_or("").to_owned(),
            };
            changes.push((i, ellipsise(&mut kit.ui.fonts, &text, l.styles[i].ty, self.plan.hero_w * k), Ink::Secondary));
        }
        if let Some(i) = l.smart {
            let (full, short, tone) = smart_line(doc, more.pointer("/doc").unwrap_or(&Value::Null), twelve);
            let (room, ty) = (self.plan.smart.map_or(0.0, |s| s.f.w) * k, l.styles[i].ty);
            // the whole of it when it fits, else the shorter way of saying it, else as much as there is room for
            let text = if kit.ui.fonts.measure(&full, ty).width <= room { full } else { ellipsise(&mut kit.ui.fonts, &short, ty, room) };
            changes.push((i, text, if tone == SmartTone::Warning { Ink::Warning } else { Ink::Accent }));
        }
        if let Some(hr) = &self.plan.hours {
            for (i, (t, v)) in l.hours.iter().enumerate() {
                let (time, temp) = if i == 0 {
                    ("Now".to_owned(), s("temp"))
                } else {
                    let h = hours.get(i);
                    (hour_label(h.and_then(|h| h.get("time")).and_then(Value::as_str).unwrap_or(""), twelve), format!("{}\u{b0}", h.and_then(|h| h.get("temp")).and_then(Value::as_f64).unwrap_or(0.0).round() as i64))
                };
                changes.push((*t, time, Ink::Secondary));
                changes.push((*v, temp, Ink::Primary));
            }
            let _ = hr;
        }
        for (i, (name, lo, hi)) in l.days.iter().enumerate() {
            match days.get(i) {
                Some(dv) => {
                    let ds = |k: &str| dv.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                    changes.push((*name, ds("day"), Ink::Primary));
                    changes.push((*lo, ds("lo"), Ink::Secondary));
                    changes.push((*hi, ds("hi"), Ink::Primary));
                }
                None => {
                    changes.push((*name, String::new(), Ink::Primary));
                    changes.push((*lo, String::new(), Ink::Primary));
                    changes.push((*hi, String::new(), Ink::Primary));
                }
            }
        }
        let (ink, ink2, accent, warn) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), kit.ui.look.warning());
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        let colour = |i: Ink| match i {
            Ink::Primary => ink,
            Ink::Secondary => ink2,
            Ink::Accent => legible(accent, ink, light),
            Ink::Warning => legible(warn, ink, light),
        };
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, t, c)| (*i, t.as_str(), colour(*c))).collect();
        self.block.set_many(kit, &refs);
        self.paint(kit, pictures, doc, &hours, &days);
    }

    /// The card's picture: the hairline under the reading, the hours' icons and bars, the week's icons and range bars.
    fn paint(&mut self, kit: &mut Kit, pictures: &mut Pictures, doc: &Value, hours: &[Value], days: &[Value]) {
        let k = self.k;
        let (w, h) = (self.w as u32, self.h as u32);
        let (ink, accent) = (kit.ui.look.ink(), kit.ui.look.accent());
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        self.rows.clear();
        let p = &self.plan;
        let mut canvas = Canvas { px: self.rows.pixels_mut(), w, h };
        // the horizon the rain lands on
        let line_w = if p.wide { p.hero_w } else { p.w };
        canvas.rounded(Frame::new(0.0, ((p.hero_bottom + 6.0) * k).round(), line_w * k, 1.0), 0.0, faded(ink, 0.14));
        if let Some(hr) = &p.hours {
            let now_kind = doc.pointer("/current/code").and_then(Value::as_i64).zip(doc.pointer("/current/isDay").and_then(Value::as_bool)).map(|(c, d)| kind_of(c, d));
            for i in 0..hr.n {
                let Some(hv) = hours.get(i) else { break };
                let cx = (hr.x + (i as f32 + 0.5) * hr.col_w) * k;
                let kind = if i == 0 { now_kind } else { None }.unwrap_or_else(|| kind_of(hv.get("code").and_then(Value::as_i64).unwrap_or(0), hv.get("isDay").and_then(Value::as_bool).unwrap_or(true)));
                let side = (hr.icon * k).round() as u32;
                if let Some(pic) = pictures.get(&format!("wxbig-{kind}")) {
                    canvas.picture(pic, (cx - side as f32 / 2.0).round() as i32, (hr.icon_y * k).round() as i32, side, side);
                }
                if let Some((top, bh)) = hr.bar {
                    let prob = (hv.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
                    let bw = 18.0 * k;
                    canvas.rounded(Frame::new(cx - bw / 2.0, top * k, bw, bh * k), bw / 2.0, faded(ink, 0.10));
                    if prob >= 0.04 {
                        let fh = (bh * k * prob).max(bw * 0.5);
                        canvas.rounded(Frame::new(cx - bw / 2.0, (top + bh) * k - fh, bw, fh), bw / 2.0, if light { faded(accent, 0.9) } else { accent });
                    }
                }
            }
        }
        if let Some(d) = &p.days {
            let deg = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).and_then(|t| t.trim_end_matches('\u{b0}').parse::<f32>().ok());
            let (mut lo_min, mut hi_max) = (f32::MAX, f32::MIN);
            for dv in days {
                if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi")) {
                    lo_min = lo_min.min(lo);
                    hi_max = hi_max.max(hi);
                }
            }
            let track = faded(ink, 0.12);
            for (i, dv) in days.iter().enumerate().take(d.rows) {
                let kind = dv.get("kind").and_then(Value::as_str).unwrap_or("");
                let y = (d.y + i as f32 * d.row_h) * k;
                let icon = (d.icon * k).round() as u32;
                if let Some(pic) = pictures.get(&format!("wxbig-{kind}")) {
                    canvas.picture(pic, ((d.x + d.day_w) * k).round() as i32, (y + (d.row_h * k - icon as f32) / 2.0).round() as i32, icon, icon);
                }
                // the range bar: the day's span of the week's
                let bx = (d.x + d.day_w + d.icon + 10.0 + d.num_w + 14.0) * k;
                let bw = ((d.x + d.w - d.num_w - 14.0) * k - bx).max(0.0);
                let by = y + (d.row_h * k - 8.0 * k) / 2.0;
                canvas.rounded(Frame::new(bx, by, bw, 8.0 * k), 4.0 * k, track);
                if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi"))
                    && hi_max > lo_min
                {
                    let (f0, f1) = ((lo - lo_min) / (hi_max - lo_min), (hi - lo_min) / (hi_max - lo_min));
                    canvas.rounded(Frame::new(bx + bw * f0, by, (bw * (f1 - f0)).max(8.0 * k), 8.0 * k), 4.0 * k, accent);
                }
            }
        }
        self.rows.damage(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(p: &Plan, h: f32) {
        let all: Vec<(&str, Slot)> = [("place", p.place), ("temp", Some(p.temp)), ("cond", Some(p.cond)), ("feels", p.feels), ("smart", p.smart)].into_iter().filter_map(|(n, s)| s.map(|s| (n, s))).collect();
        for (n, s) in all {
            assert!(s.f.x >= 0.0 && s.f.y >= 0.0 && s.f.right() <= p.w + 0.5 && s.f.bottom() <= h + 0.5, "{n} {:?} outside {}x{}", s.f, p.w, h);
        }
        if let Some(hr) = &p.hours {
            let hh = if hr.bar.is_some() || hr.icon > 40.0 { HOURS_BARS_H } else { HOURS_H };
            assert!(hr.x >= 0.0 && hr.x + hr.n as f32 * hr.col_w <= p.w + 0.5 && hr.y + hh <= h + 0.5, "hours {hr:?} outside {}x{}", p.w, h);
        }
        if let Some(d) = &p.days {
            assert!(d.x >= 0.0 && d.x + d.w <= p.w + 0.5 && d.y + d.rows as f32 * d.row_h <= h + 0.5, "days {d:?} outside {}x{}", p.w, h);
        }
    }

    #[test]
    fn every_part_there_is_room_for_is_inside_the_box_whatever_the_box() {
        for w in [160.0, 184.0, 246.0, 300.0, 369.0, 500.0, 640.0, 822.0, 1200.0, 1792.0] {
            for h in [90.0, 130.0, 160.0, 212.0, 260.0, 300.0, 330.0, 424.0, 560.0, 640.0] {
                for data in [Data::default(), Data { hours: true, bars: false }, Data { hours: true, bars: true }] {
                    inside(&plan(w, h, data), h);
                }
            }
        }
    }

    #[test]
    fn the_default_card_has_the_reading_the_next_thing_and_the_hours_with_their_rain() {
        let p = plan(369.0, 424.0, Data { hours: true, bars: true });
        assert!(p.place.is_some() && p.feels.is_some() && p.smart.is_some(), "the whole reading and the line that says what is next");
        let hr = p.hours.expect("the hours");
        assert_eq!(hr.n, 5);
        assert!(hr.bar.is_some(), "with the bars of the rain");
        let dry = plan(369.0, 424.0, Data { hours: true, bars: false }).hours.expect("the hours");
        assert!(dry.bar.is_none() && dry.icon > hr.icon, "with no rain to show, the room is the pictures'");
        assert!(hr.y >= p.hero_bottom, "under the reading");
        assert!(p.days.is_none(), "no room for the week as well: that is the view's");
        // without the hours the week fills the room
        let p = plan(369.0, 424.0, Data::default());
        assert!(p.hours.is_none() && p.days.is_some_and(|d| d.rows >= 3));
    }

    #[test]
    fn a_wide_card_has_the_week_beside_the_reading_and_a_small_one_only_the_reading() {
        let p = plan(822.0, 424.0, Data { hours: true, bars: true });
        assert!(p.wide && p.hours.is_some_and(|h| h.n >= 5) && p.days.is_some_and(|d| d.rows >= 3));
        assert!(p.days.is_some_and(|d| d.x >= p.hero_w), "beside the reading, not under it");
        let p = plan(184.0, 212.0, Data { hours: true, bars: true });
        assert!(p.hours.is_none() && p.days.is_none(), "a card as small as the largest text size makes the default one has the reading");
        let p = plan(300.0, 120.0, Data::default());
        assert!(p.place.is_none() && p.feels.is_none() && p.smart.is_none());
    }
}
