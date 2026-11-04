//! The sheet of a sensor's history drawn: a pane over Home with the name, the ranges as chips, the reading now and its low, average and high,
//! and under them what the readings were: a chart for a number, bands of on and off for a switch, the recent changes for words.
//!
//! The words are one layer; the chart is one picture painted once, when the answer comes, and revealed by a clip the compositor widens. The
//! layout is a pure function of the type's measures ([`layout`]), so that nothing in it overlaps at either text size is a test.

use super::chart::{self, Axis, Colours, Plot};
use super::layout::Metrics;
use super::sensor::{Doc, Load, RANGES, SensorSheet};
use super::{card, num_text};
use crate::home::clock;
use crate::home::surface::glass_of;
use crate::home::widgets::{Words, ago};
use crate::kit::Kit;
use crate::kit::button::faded;
use crate::kit::motion::Beat;
use crate::kit::pane::PaneKind;
use crate::kit::text::role_style;
use compositor_protocol::{Ease, Props, Rect, Rgba};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

/// What the sheet shows, by the kind of sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Number,
    Binary,
    Text,
}

pub fn shape_of(tile: &Value) -> Shape {
    match tile.pointer("/ctl/kind").and_then(Value::as_str) {
        Some("binary") => Shape::Binary,
        Some("text") => Shape::Text,
        _ => Shape::Number,
    }
}

/// Where the parts of the sheet are, in the pane's own space (the pane itself on the screen).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pane: Frame,
    pub name: Frame,
    pub sub: Frame,
    pub chips: Vec<Frame>,
    pub value: Frame,
    pub stats: Vec<Frame>,
    /// Under the stats: the chart's picture (a number), the bar of bands (a switch) or the list (words).
    pub plot: Frame,
    /// The room left of a chart for the words of its scale.
    pub gutter: f32,
    pub x_row: Frame,
    /// What the large reading is set in.
    pub value_line: f32,
}

const PAD: f32 = 32.0;
const GAP: f32 = 16.0;

/// The sheet's frames for a type of these measures: the name and its line, the chips, the reading and the three numbers beside it, and the
/// plot under them with the words of its time axis. A sheet no higher than the safe area allows: the plot gives way.
pub fn layout(m: &Metrics, shape: Shape) -> Layout {
    let s = m.scale;
    let width = (1100.0 * s.min(1.4)).round().min(1700.0);
    let inner = width - 2.0 * PAD;
    let head_h = m.title + m.caption;
    let chip_h = m.label + GAP;
    let chip_w = (m.label * 2.3).round();
    let value_line = if s <= 1.25 { m.display } else { m.title };
    let row_h = value_line.max(m.caption + m.label);
    // the time axis of a chart or a list has its own row under the plot; a switch's bands carry theirs inside it
    let x_h = if shape == Shape::Binary { 0.0 } else { m.caption + 8.0 };
    let (y_chips, y_value) = (PAD + head_h + GAP, PAD + head_h + GAP + chip_h + GAP);
    // a chart's unit stands over its scale, in the line the plot's top gives it
    let y_plot = y_value + row_h + if shape == Shape::Number { m.caption + 4.0 } else { GAP };
    let max_h = 1080.0 - 2.0 * 54.0;
    let want = match shape {
        Shape::Number => 340.0 * s.min(1.5),
        Shape::Binary => (m.label + 24.0).max(64.0) + 22.0 + 2.0 * m.caption,
        Shape::Text => (m.label + 14.0) * 6.0,
    };
    let plot_h = want.min(max_h - y_plot - x_h - PAD).max(120.0).floor();
    let height = (y_plot + plot_h + x_h + PAD).min(max_h);
    let (px, py) = (((1920.0 - width) / 2.0).round(), (((1080.0 - height) / 2.0).round()).max(54.0));
    let pane = Frame::new(px, py, width, height);
    // the chips are on their own row, from the left; the name has the whole width above them
    let chips: Vec<Frame> = (0..RANGES.len()).map(|i| Frame::new(PAD + i as f32 * (chip_w + 8.0), y_chips, chip_w, chip_h)).collect();
    let name = Frame::new(PAD, PAD, inner, m.title);
    let sub = Frame::new(PAD, PAD + m.title, inner, m.caption);
    // (the reading is a line shorter at large type: its share gives room to the three numbers, whose captions carry a time)
    let value_w = (inner * if s <= 1.25 { 0.42 } else { 0.30 }).floor();
    let value = Frame::new(PAD, y_value + ((row_h - value_line) / 2.0).floor(), value_w, value_line);
    let cell_w = ((inner - value_w - GAP) / 3.0).floor();
    let stats = (0..3).map(|i| Frame::new(PAD + value_w + GAP + i as f32 * cell_w, y_value + ((row_h - m.caption - m.label) / 2.0).floor(), cell_w, m.caption + m.label)).collect();
    let gutter = if shape == Shape::Number { (m.caption * 2.4).round() } else { 0.0 };
    let plot = Frame::new(PAD + gutter, y_plot, inner - gutter, plot_h);
    let x_row = if shape == Shape::Binary { Frame::new(PAD + gutter, y_plot + plot_h, inner - gutter, 0.0) } else { Frame::new(PAD + gutter, y_plot + plot_h + 8.0, inner - gutter, m.caption) };
    Layout { pane, name, sub, chips, value, stats, plot, gutter, x_row, value_line }
}

fn with_unit(value: &str, unit: &str) -> String {
    match unit {
        "" => value.to_owned(),
        u if u.starts_with('\u{b0}') || u == "%" => format!("{value}{u}"),
        u => format!("{value} {u}"),
    }
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A time of the range as a person says it: the clock for a day or less, the weekday and the clock beyond.
fn when(ms: f64, range: &str, twelve: bool) -> String {
    let c = clock::civil(ms as u64);
    let (t, ap) = c.time_words(twelve);
    let clock = if ap.is_empty() { t } else { format!("{t} {ap}") };
    if matches!(range, "7d" | "30d") { format!("{} {clock}", DAYS[c.weekday as usize % 7]) } else { clock }
}

/// The words of what a sensor of a kind was and is.
struct Body {
    words: Words,
    picture: Option<ImageLayer>,
    /// The legend's two dots (a switch's bands).
    _dots: Vec<ShapeLayer>,
}

pub struct SensorSurface {
    _scrim: ShapeLayer,
    _pane: GlassLayer,
    marker: ShapeLayer,
    head: Words,
    body: Option<Body>,
    pub layout: Layout,
    shape: Shape,
    z: i32,
    drawn: Option<(u64, usize)>,
    id: String,
}

impl SensorSurface {
    pub fn new(kit: &mut Kit, sheet: &SensorSheet, tile: &Value, z: i32) -> std::io::Result<Self> {
        let m = card::metrics_of(kit);
        let shape = shape_of(tile);
        let lay = layout(&m, shape);
        let c = kit.tokens.extra(kit.mode).scrim_hide;
        let scrim = kit.ui.scene.shape(1920, 1080, z, plate::solid(c, 0.0), Props::at(0.0, 0.0));
        let pane = kit.ui.scene.glass(lay.pane.w as u32, lay.pane.h as u32, z + 1, glass_of(kit, PaneKind::Sheet, lay.pane.h, false, false), Props::at(lay.pane.x, lay.pane.y));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let radius = (lay.chips[0].h / 2.0).floor();
        let at = Props::at(lay.pane.x + lay.chips[sheet.range].x, lay.pane.y + lay.chips[sheet.range].y);
        let marker = kit.ui.scene.shape(lay.chips[0].w as u32, lay.chips[0].h as u32, z + 2, plate::solid(kit.ui.look.accent(), radius), at);
        let name = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let sub = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let chip = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink2, Align::Centre);
        let mut lines = vec![(lay.name, name, tile.get("name").and_then(Value::as_str).unwrap_or("").to_owned()), (lay.sub, sub, sub_words(tile))];
        for (i, f) in lay.chips.iter().enumerate() {
            lines.push((Frame::new(f.x, f.y + ((f.h - m.label) / 2.0).floor(), f.w, m.label), chip, RANGES[i].1.to_owned()));
        }
        let head = Words::new(kit, Frame::new(0.0, 0.0, lay.pane.w, lay.pane.h), z + 4, lines)?;
        let mut me = Self { _scrim: scrim, _pane: pane, marker, head, body: None, layout: lay, shape, z, drawn: None, id: sheet.id.clone() };
        me.head.set_props(Props::at(me.layout.pane.x, me.layout.pane.y));
        me.chips(kit, sheet.range);
        Ok(me)
    }

    pub fn is_for(&self, id: &str, tile: &Value) -> bool {
        self.id == id && self.shape == shape_of(tile)
    }

    /// The chip of the range in front is the accent's, its word on it.
    fn chips(&mut self, kit: &mut Kit, range: usize) {
        let (ink2, on) = (kit.ui.look.ink2(), kit.ui.look.accent_ink());
        let changes: Vec<(usize, &str, Rgba)> = (0..RANGES.len()).map(|i| (2 + i, RANGES[i].1, if i == range { on } else { ink2 })).collect();
        self.head.set_many(kit, &changes);
    }

    pub fn update(&mut self, kit: &mut Kit, sheet: &SensorSheet, tile: &Value, twelve: bool) -> std::io::Result<()> {
        let to = Props::at(self.layout.pane.x + self.layout.chips[sheet.range].x, self.layout.pane.y + self.layout.chips[sheet.range].y);
        if self.drawn.is_none_or(|(_, r)| r != sheet.range) {
            let ms = kit.beat_ms(Beat::Label);
            if self.drawn.is_none() || ms == 0 { self.marker.set(to) } else { self.marker.animate(to, ms, Ease::Out) }
            self.chips(kit, sheet.range);
        }
        if self.drawn.map(|(rev, _)| rev) == Some(sheet.rev) && self.body.is_some() {
            self.drawn = Some((sheet.rev, sheet.range));
            return Ok(());
        }
        self.drawn = Some((sheet.rev, sheet.range));
        self.body = Some(self.make_body(kit, sheet, tile, twelve)?);
        Ok(())
    }

    fn make_body(&self, kit: &mut Kit, sheet: &SensorSheet, tile: &Value, twelve: bool) -> std::io::Result<Body> {
        let m = card::metrics_of(kit);
        let lay = &self.layout;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let value_role = if m.scale <= 1.25 { TypeRole::Display } else { TypeRole::Title };
        let value_style = role_style(kit, value_role, Some(Weight::SemiBold), ink, Align::Left);
        let cap = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let lab = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let mut lines: Vec<(Frame, ui::components::Style, String)> = Vec::new();
        let (mut picture, mut dots) = (None, Vec::new());
        let z = self.z + 5;
        let pane = lay.pane;
        let message = |kit: &mut Kit, lines: &mut Vec<(Frame, ui::components::Style, String)>, words: &str| {
            let style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink2(), Align::Centre);
            lines.push((Frame::new(lay.plot.x, lay.plot.y + ((lay.plot.h - m.label) / 2.0).floor(), lay.plot.w, m.label), style, words.to_owned()));
        };
        let range = sheet.range_id();
        let tile_words = tile.get("words").and_then(Value::as_str).unwrap_or("").to_owned();
        match (&sheet.load, &sheet.doc) {
            (Load::Asking, _) => message(kit, &mut lines, "Loading\u{2026}"),
            (Load::Failed(words), _) => message(kit, &mut lines, words),
            (Load::Ready, None) => message(kit, &mut lines, "No history for this range"),
            (Load::Ready, Some(Doc::Number { from, to, unit, class, points, stats, now })) => {
                lines.push((lay.value, value_style, now.map(|v| with_unit(&num_text(v), unit)).unwrap_or(tile_words)));
                let Some(st) = stats.as_ref().filter(|_| !points.is_empty()) else {
                    message(kit, &mut lines, "No readings in this range");
                    return self.finish(kit, lines, None, dots);
                };
                for (i, (label, v, at)) in [("Low", st.min, Some(st.min_at)), ("Average", st.avg, None), ("High", st.max, Some(st.max_at))].into_iter().enumerate() {
                    let f = lay.stats[i];
                    let caption = at.map_or_else(|| label.to_owned(), |t| format!("{label} \u{b7} {}", when(t, range, twelve)));
                    lines.push((Frame::new(f.x, f.y, f.w, m.caption), cap, caption));
                    lines.push((Frame::new(f.x, f.y + m.caption, f.w, m.label), lab, with_unit(&num_text(v), unit)));
                }
                // the scale: a share of a whole from 0 to 100, else the readings padded; as many ticks as the room has lines for
                let percent = chart::is_percent(class, unit, st.min, st.max);
                let most = (1 + (lay.plot.h / (m.caption * 1.3)).floor() as usize).clamp(2, 6);
                let axis: Axis = chart::axis(st.min, st.max, percent, most);
                let step = axis.ticks.windows(2).map(|w| w[1] - w[0]).next().unwrap_or(1.0);
                let right = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
                let pw = lay.plot.w as u32;
                let inset = 10.0;
                for t in &axis.ticks {
                    let y = lay.plot.y + inset + axis.row(*t, lay.plot.h - 1.0 - 2.0 * inset);
                    let y = (y.round() - (m.caption / 2.0).floor()).clamp(lay.plot.y - 2.0, lay.plot.y + lay.plot.h - m.caption + 2.0);
                    lines.push((Frame::new(PAD, y, lay.gutter - 12.0, m.caption), right, chart::tick_text(*t, step)));
                }
                // the unit stands over the scale
                if !unit.is_empty() {
                    lines.push((Frame::new(PAD, lay.plot.y - m.caption - 2.0, lay.gutter + 40.0, m.caption), cap, unit.clone()));
                }
                let centre = role_style(kit, TypeRole::Caption, None, ink2, Align::Centre);
                let ticks = chart::time_ticks(range, *from, *to, twelve, ((lay.plot.w / (m.caption * 4.6)).floor() as usize).clamp(2, 8));
                let span = (*to - *from).max(1.0);
                for t in &ticks {
                    let x = lay.plot.x + inset + ((t.at - *from) / span) as f32 * (lay.plot.w - 1.0 - 2.0 * inset);
                    let w = (m.caption * 4.4).round();
                    // a label may reach into the sheet's margin (and under the words of the scale, where the row has none), so that the first
                    // one stands at its time and is not pushed in
                    let x = (x - w / 2.0).round().clamp(PAD / 2.0, lay.plot.x + lay.plot.w - w + 8.0);
                    lines.push((Frame::new(x, lay.x_row.y, w, m.caption), centre, t.label.clone()));
                }
                // the picture: painted once, here, in the colours of the theme
                let ph = lay.plot.h as u32;
                let mut img = kit.ui.scene.image(pw, ph, z - 1, Props::at(pane.x + lay.plot.x, pane.y + lay.plot.y))?;
                img.clear();
                let accent = kit.ui.look.accent();
                let colours = Colours { line: accent, fill: faded(accent, 0.34), grid: faded(ink, 0.12), mark: accent, halo: kit.ui.look.colour(PaletteRole::Sheet) };
                let plot = Plot { w: pw, h: ph, inset, from: *from, to: *to, axis: &axis, points };
                let width = (3.0 * m.scale.min(1.5)).round().max(3.0);
                chart::paint(img.pixels_mut(), &plot, &colours, width, &[(st.min_at, st.min), (st.max_at, st.max)]);
                img.damage(None);
                reveal(kit, &img, Frame::new(pane.x + lay.plot.x, pane.y + lay.plot.y, lay.plot.w, lay.plot.h));
                picture = Some(img);
            }
            (Load::Ready, Some(Doc::Binary { from, to, class: _, bands, changes, now: _, last_change })) => {
                lines.push((lay.value, value_style, tile_words));
                let cells = [("Changes", changes.to_string()), ("Last changed", last_change.map_or_else(|| "\u{2014}".to_owned(), |t| when(t, range, twelve)))];
                for (i, (label, v)) in cells.into_iter().enumerate() {
                    let f = lay.stats[i];
                    lines.push((Frame::new(f.x, f.y, f.w, m.caption), cap, label.to_owned()));
                    lines.push((Frame::new(f.x, f.y + m.caption, f.w, m.label), lab, v));
                }
                if let Some(t) = last_change {
                    // how long ago it last changed, under the last change's own time
                    let f = lay.stats[2];
                    lines.push((Frame::new(f.x, f.y, f.w, m.caption), cap, "Ago".to_owned()));
                    lines.push((Frame::new(f.x, f.y + m.caption, f.w, m.label), lab, ago((clock_now_ms(*to) - *t).max(0.0) as u64)));
                }
                let bar_h = (m.label + 24.0).max(64.0).floor();
                let (pw, ph) = (lay.plot.w as u32, bar_h as u32);
                let mut img = kit.ui.scene.image(pw, ph, z - 1, Props::at(pane.x + lay.plot.x, pane.y + lay.plot.y))?;
                img.clear();
                let (on, off) = (kit.ui.look.accent(), kit.ui.look.theme.materials.track_off);
                let unknown = faded(kit.ui.look.ink3(), 0.5);
                let bands: Vec<(f64, f64, &str)> = bands.iter().map(|(a, b, s)| (*a, *b, s.as_str())).collect();
                chart::paint_bands(img.pixels_mut(), pw, ph, *from, *to, &bands, on, off, unknown);
                img.damage(None);
                reveal(kit, &img, Frame::new(pane.x + lay.plot.x, pane.y + lay.plot.y, lay.plot.w, lay.plot.h));
                picture = Some(img);
                // the legend: which colour is which
                let (yes, no) = (tile.pointer("/ctl/yes").and_then(Value::as_str).unwrap_or("On").to_owned(), tile.pointer("/ctl/no").and_then(Value::as_str).unwrap_or("Off").to_owned());
                let ly = lay.plot.y + bar_h + 14.0;
                let dot = (m.caption * 0.55).round();
                let mut x = lay.plot.x;
                for (word, colour) in [(yes, on), (no, off)] {
                    dots.push(kit.ui.scene.shape(dot as u32, dot as u32, z, plate::solid(colour, dot / 2.0), Props::at(pane.x + x, pane.y + ly + ((m.caption - dot) / 2.0).floor())));
                    let w = (m.caption * 0.6 * word.chars().count() as f32).ceil() + 8.0;
                    lines.push((Frame::new(x + dot + 8.0, ly, w, m.caption), cap, word));
                    x += dot + 8.0 + w + GAP * 2.0;
                }
                let ticks = chart::time_ticks(range, *from, *to, twelve, ((lay.plot.w / (m.caption * 4.6)).floor() as usize).clamp(2, 8));
                let centre = role_style(kit, TypeRole::Caption, None, ink2, Align::Centre);
                let span = (*to - *from).max(1.0);
                let ay = lay.plot.y + bar_h + 8.0 + m.caption + 14.0;
                for t in &ticks {
                    let w = (m.caption * 4.4).round();
                    let x = (lay.plot.x + ((t.at - *from) / span) as f32 * (lay.plot.w - 1.0) - w / 2.0).round().clamp(PAD / 2.0, lay.plot.x + lay.plot.w - w + 8.0);
                    lines.push((Frame::new(x, ay, w, m.caption), centre, t.label.clone()));
                }
            }
            (Load::Ready, Some(Doc::Text { changes, last_change })) => {
                let words = changes.first().map_or(tile_words.clone(), |c| c.1.clone());
                lines.push((lay.value, role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), words));
                if let Some(t) = last_change {
                    let f = lay.stats[0];
                    lines.push((Frame::new(f.x, f.y, f.w, m.caption), cap, "Last changed".to_owned()));
                    lines.push((Frame::new(f.x, f.y + m.caption, f.w, m.label), lab, when(*t, range, twelve)));
                }
                let row_h = m.label + 14.0;
                let rows = ((lay.plot.h + lay.x_row.h) / row_h).floor() as usize;
                for (i, (t, w)) in changes.iter().take(rows).enumerate() {
                    let y = lay.plot.y + i as f32 * row_h + 7.0;
                    lines.push((Frame::new(lay.plot.x, y, (m.caption * 9.0).round(), m.label), cap, when(*t, range, twelve)));
                    lines.push((Frame::new(lay.plot.x + (m.caption * 9.0).round() + GAP, y, lay.plot.w - (m.caption * 9.0).round() - GAP, m.label), role_style(kit, TypeRole::Label, None, ink, Align::Left), w.clone()));
                }
            }
        }
        self.finish(kit, lines, picture, dots)
    }

    fn finish(&self, kit: &mut Kit, lines: Vec<(Frame, ui::components::Style, String)>, picture: Option<ImageLayer>, dots: Vec<ShapeLayer>) -> std::io::Result<Body> {
        let pane = self.layout.pane;
        let mut words = Words::new(kit, Frame::new(0.0, 0.0, pane.w, pane.h), self.z + 5, lines)?;
        words.set_props(Props::at(pane.x, pane.y));
        Ok(Body { words, picture, _dots: dots })
    }

    /// The lint's view: the pane, and every line of words on it.
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, paint: Rgba) {
        let panel = lint.panel("sensor sheet", self.layout.pane, paint, true);
        // the range in front is read on its marker, the accent; the others on the sheet
        let front = self.layout.chips[self.drawn.map_or(0, |(_, r)| r)];
        let pane = self.layout.pane;
        let marker = lint.panel_on("sensor sheet range", Frame::new(pane.x + front.x, pane.y + front.y, front.w, front.h), kit.ui.look.accent(), false, panel);
        let chip = 2 + self.drawn.map_or(0, |(_, r)| r);
        self.head.lint_by(kit, lint, "sensor sheet", (0.0, 0.0), |i| Some(if i == chip { marker } else { panel }));
        if let Some(b) = &self.body {
            b.words.lint(kit, lint, "sensor sheet", (0.0, 0.0), Some(panel));
        }
    }

    /// Whether the chart's picture is on the sheet (for a test).
    pub fn has_picture(&self) -> bool {
        self.body.as_ref().is_some_and(|b| b.picture.is_some())
    }
}

/// The picture is revealed from its left edge: a clip that the compositor widens.
fn reveal(kit: &Kit, img: &ImageLayer, area: Frame) {
    let at = Props::at(area.x, area.y);
    let ms = kit.beat_ms(Beat::Layer) * 2;
    if ms == 0 {
        img.set(at);
        return;
    }
    let full = Rect { x: area.x, y: area.y, w: area.w, h: area.h };
    img.set(Props { clip: Some(Rect { w: 0.0, ..full }), ..at });
    img.animate(Props { clip: Some(full), ..at }, ms, Ease::Out);
}

fn sub_words(tile: &Value) -> String {
    let area = tile.get("area").and_then(Value::as_str).unwrap_or("");
    let class = tile.pointer("/ctl/class").and_then(Value::as_str).unwrap_or("").replace('_', " ");
    let class = {
        let mut c = class.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    [area, class.as_str()].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" \u{b7} ")
}

/// The moment the range ends at (the time the answer was made), as the clock of the page counts it.
fn clock_now_ms(to: f64) -> f64 {
    to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_is_inside_the_safe_area_and_its_parts_are_inside_it_and_apart_at_both_text_sizes() {
        for scale in [1.0f32, 1.5, 2.0] {
            let m = Metrics::nominal(scale);
            for shape in [Shape::Number, Shape::Binary, Shape::Text] {
                let l = layout(&m, shape);
                let tag = format!("{shape:?} at {scale}: {l:?}");
                assert!(l.pane.x >= 0.0 && l.pane.right() <= 1920.0 && l.pane.y >= 54.0 && l.pane.bottom() <= 1080.0 - 54.0 + 0.5, "{tag}");
                let pane = Frame::new(0.0, 0.0, l.pane.w, l.pane.h);
                let inside = |f: &Frame| f.x >= -9.0 && f.y >= 0.0 && f.right() <= pane.w + 9.0 && f.bottom() <= pane.h;
                let parts: Vec<&Frame> = [&l.name, &l.sub, &l.value, &l.plot, &l.x_row].into_iter().chain(l.chips.iter()).chain(l.stats.iter()).collect();
                assert!(parts.iter().all(|f| inside(f)), "{tag}");
                // the rows are in order from the top and none lies on another
                let order = [l.name.bottom(), l.sub.bottom(), l.chips[0].bottom(), l.value.bottom(), l.plot.y, l.x_row.y];
                assert!(order.windows(2).all(|w| w[1] >= w[0] - 0.5 || w[1] == l.plot.y), "{tag}: {order:?}");
                assert!(l.sub.bottom() <= l.chips[0].y && l.chips[0].bottom() <= l.value.y.min(l.stats[0].y) + 0.5, "{tag}");
                assert!(l.value.bottom() <= l.plot.y && l.stats.iter().all(|s| s.bottom() <= l.plot.y), "{tag}");
                assert!(l.plot.bottom() <= l.x_row.y && l.plot.h >= 120.0, "{tag}");
                assert!(l.chips.windows(2).all(|w| w[0].right() < w[1].x), "{tag}");
                assert!(l.value.right() <= l.stats[0].x && l.stats.windows(2).all(|w| w[0].right() <= w[1].x), "{tag}");
            }
        }
    }

    #[test]
    fn a_time_of_the_range_is_the_clock_for_a_day_and_the_weekday_beyond() {
        let t = 1_800_000_000_000.0;
        assert!(!when(t, "24h", false).contains(char::is_alphabetic));
        assert!(DAYS.iter().any(|d| when(t, "7d", true).starts_with(d)));
        assert_eq!(with_unit("21.4", "\u{b0}C"), "21.4\u{b0}C");
        assert_eq!(with_unit("55", "%"), "55%");
        assert_eq!(with_unit("1.2", "kWh"), "1.2 kWh");
        assert_eq!(with_unit("7", ""), "7");
    }
}
