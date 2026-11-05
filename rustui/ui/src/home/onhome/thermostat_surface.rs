//! The thermostat sheet drawn: a pane over Home with the thermostat's name, on the left the room now (large, with what the thermostat is doing and
//! the humidity), on the right its set point with a step down and a step up (two of them, heat to and cool to, for a range), and under them the
//! modes as chips with the mode it is in filled.
//!
//! The pane has one picture and one layer of words. A change repaints the band it changes and tells the compositor that band only. The layout is a
//! pure function of the type's measures ([`layout`]), so that nothing in it overlaps at either text size is a test, and so that the pointer's place on
//! it is a pure function too ([`hit`]).

use super::layout::Metrics;
use super::thermostat::{Row, ThermostatSheet};
use super::{card, paint};
use crate::home::surface::glass_of;
use crate::home::widgets::{Canvas, Words};
use crate::kit::Kit;
use crate::kit::pane::PaneKind;
use crate::kit::text::role_style;
use compositor_protocol::{Props, Rect, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

const PAD: f32 = 32.0;
const GAP: f32 = 16.0;
/// The most chips the sheet draws (a thermostat lists a handful of modes: off, heat, cool, heat/cool, fan, dry).
const MAX_CHIPS: usize = 10;
/// The words ahead of the set points' and the chips': the name, the line under it, the room's label, the reading, what it is doing, the humidity.
const HEAD: usize = 6;

/// One set point: its band, the words at its left, and the step down, the value and the step up.
#[derive(Clone, Debug, PartialEq)]
pub struct SetAt {
    pub row: Option<Row>,
    pub band: Frame,
    pub label: Frame,
    pub minus: Frame,
    pub value: Frame,
    pub plus: Frame,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pane: Frame,
    pub name: Frame,
    pub sub: Frame,
    /// The label of the room's reading, the reading, what the thermostat is doing, the humidity.
    pub now: [Frame; 4],
    /// The set points shown (at least one place: the word that stands for none).
    pub sets: Vec<SetAt>,
    pub mode_band: Frame,
    pub mode_label: Frame,
    pub chips: Vec<Frame>,
}

/// The sheet's frames for a type of these measures: `shown` set points, and `chips` modes of at most `chars` characters each.
pub fn layout(m: &Metrics, shown: &[Row], chips: usize, chars: usize) -> Layout {
    let s = m.scale;
    let width = (1100.0 * s.min(1.4)).round().min(1700.0);
    let inner = width - 2.0 * PAD;
    let head_h = m.title + m.caption;
    let y0 = PAD + head_h + GAP;
    let value_line = if s <= 1.25 { m.display } else { m.title };
    // the room on the left
    let info_w = (inner * 0.38).floor();
    let info_h = m.label + value_line + 2.0 * m.caption;
    let now = [
        Frame::new(PAD, y0, info_w, m.label),
        Frame::new(PAD, y0 + m.label, info_w, value_line),
        Frame::new(PAD, y0 + m.label + value_line, info_w, m.caption),
        Frame::new(PAD, y0 + m.label + value_line + m.caption, info_w, m.caption),
    ];
    // the set points on the right, centred on the room's height
    let n = shown.len().max(1);
    let row_h = (m.title + 28.0).max(84.0).ceil();
    let gap = 8.0;
    let sets_h = n as f32 * row_h + (n as f32 - 1.0) * gap;
    let body_h = info_h.max(sets_h);
    let (rx, rw) = (PAD + info_w + GAP, inner - info_w - GAP);
    let top = y0 + ((body_h - sets_h) / 2.0).floor();
    let btn = (row_h * 0.62).floor();
    let label_w = (m.label * 3.6).round();
    let value_w = (m.title * 2.6).round();
    let sets = (0..n)
        .map(|i| {
            let band = Frame::new(rx, top + i as f32 * (row_h + gap), rw, row_h);
            let cy = band.y + ((row_h - btn) / 2.0).floor();
            let plus = Frame::new(band.right() - 16.0 - btn, cy, btn, btn);
            let value = Frame::new(plus.x - 12.0 - value_w, band.y + ((row_h - m.title) / 2.0).floor(), value_w, m.title);
            let minus = Frame::new(value.x - 12.0 - btn, cy, btn, btn);
            let label = Frame::new(band.x + 16.0, band.y + ((row_h - m.label) / 2.0).floor(), label_w, m.label);
            SetAt { row: shown.get(i).copied(), band, label, minus, value, plus }
        })
        .collect();
    // the modes under them: as many chips in a line as hold the longest name, and more lines when there is not room
    let y_mode = y0 + body_h + GAP;
    let chip_h = m.caption + 20.0;
    let chip_w = (m.caption * 0.42 * chars as f32).ceil() + 28.0;
    // (with no chips the label has no place, and no words: a frame inside the pane keeps its line where the layer is)
    let mode_label = if chips == 0 { Frame::new(PAD, PAD, 1.0, 1.0) } else { Frame::new(PAD + 16.0, y_mode + 8.0 + ((chip_h - m.label) / 2.0).floor(), (m.label * 3.0).round(), m.label) };
    let x0 = mode_label.right() + GAP;
    let avail = PAD + inner - x0 - 8.0;
    let per_line = (((avail + 8.0) / (chip_w + 8.0)).floor() as usize).max(1);
    let count = chips.min(MAX_CHIPS);
    let lines = count.div_ceil(per_line).max(1);
    let chip_frames = (0..count).map(|k| Frame::new(x0 + (k % per_line) as f32 * (chip_w + 8.0), y_mode + 8.0 + (k / per_line) as f32 * (chip_h + 8.0), chip_w, chip_h)).collect();
    let mode_h = if count == 0 { 0.0 } else { lines as f32 * chip_h + (lines as f32 - 1.0) * 8.0 + 16.0 };
    let max_h = 1080.0 - 2.0 * 54.0;
    let height = (if count == 0 { y0 + body_h + PAD } else { y_mode + mode_h + PAD }).min(max_h);
    let (px, py) = (((1920.0 - width) / 2.0).round(), (((1080.0 - height) / 2.0).round()).max(54.0));
    Layout {
        pane: Frame::new(px, py, width, height),
        name: Frame::new(PAD, PAD, inner, m.title),
        sub: Frame::new(PAD, PAD + m.title, inner, m.caption),
        now,
        sets,
        mode_band: Frame::new(PAD, y_mode, inner, mode_h),
        mode_label,
        chips: chip_frames,
    }
}

/// What the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The step down and the step up of a set point.
    Minus(Row),
    Plus(Row),
    /// A chip: an index into the modes.
    Chip(usize),
}

/// What is under the point (`x`, `y`) of the screen on a sheet laid out as `lay`: only what the keys reach (a step of a set point the TV may not set is
/// not a button).
pub fn hit(lay: &Layout, sheet: &ThermostatSheet, x: f32, y: f32) -> Option<Hit> {
    let (x, y) = (x - lay.pane.x, y - lay.pane.y);
    let near = |f: Frame| Frame::new(f.x - 8.0, f.y - 8.0, f.w + 16.0, f.h + 16.0).contains(x, y);
    for s in &lay.sets {
        if let Some(row) = s.row.filter(|r| sheet.rows.contains(r)) {
            if near(s.minus) {
                return Some(Hit::Minus(row));
            }
            if near(s.plus) {
                return Some(Hit::Plus(row));
            }
        }
    }
    if sheet.rows.contains(&Row::Mode) {
        return lay.chips.iter().position(|c| c.contains(x, y)).map(Hit::Chip);
    }
    None
}

// ---- what is painted

fn straight(rgb: [u8; 3]) -> Rgba {
    Rgba(rgb[0], rgb[1], rgb[2], 255)
}

/// A step's glyph: a bar for a step down, two for a step up, in the middle of its button.
fn glyph(canvas: &mut Canvas, f: Frame, plus: bool, colour: Rgba) {
    let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
    let (len, thick) = ((f.w * 0.42).round(), (f.w * 0.075).round().max(3.0));
    canvas.rounded(Frame::new(cx - len / 2.0, cy - thick / 2.0, len, thick), thick / 2.0, colour);
    if plus {
        canvas.rounded(Frame::new(cx - thick / 2.0, cy - len / 2.0, thick, len), thick / 2.0, colour);
    }
}

/// What a set point is called on its band.
fn label_of(row: Row) -> &'static str {
    match row {
        Row::Set => "Set to",
        Row::Low => "Heat to",
        Row::High => "Cool to",
        Row::Mode => "Mode",
    }
}

/// What the painted parts depend on: a change of it repaints them.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    at: Option<Row>,
    /// Each set point shown: its value (in hundredths) and whether its steps are held at an end of its range.
    sets: Vec<(Row, Option<i64>, bool, bool)>,
    mode: String,
    cursor: usize,
}

fn hundredths(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

fn look_of(s: &ThermostatSheet) -> Look {
    let sets = s
        .shown()
        .into_iter()
        .map(|r| {
            let v = s.value(r);
            (r, v.map(hundredths), v.is_some_and(|v| v <= s.min + 1e-9), v.is_some_and(|v| v >= s.max - 1e-9))
        })
        .collect();
    Look { at: s.row(), sets, mode: s.mode.clone(), cursor: s.cursor }
}

pub struct ThermostatSurface {
    _scrim: ShapeLayer,
    _pane: GlassLayer,
    art: ImageLayer,
    words: Words,
    pub layout: Layout,
    id: String,
    /// What the layout was made for: the set points shown, the modes' names.
    shape: (Vec<Row>, Vec<String>),
    drawn: Option<Look>,
    /// The line under the name as it was last said: the room, or what Home Assistant refused or why it may not be set.
    sub: String,
    ground: [u8; 3],
}

fn chars_of(modes: &[String]) -> usize {
    modes.iter().map(|m| ThermostatSheet::mode_word(m).chars().count()).max().unwrap_or(0)
}

impl ThermostatSurface {
    pub fn new(kit: &mut Kit, sheet: &ThermostatSheet, tile: &Value, z: i32) -> std::io::Result<Self> {
        let m = card::metrics_of(kit);
        let shown = sheet.shown();
        let lay = layout(&m, &shown, sheet.modes.len(), chars_of(&sheet.modes));
        let c = kit.tokens.extra(kit.mode).scrim_hide;
        let scrim = kit.ui.scene.shape(1920, 1080, z, plate::solid(c, 0.0), Props::at(0.0, 0.0));
        let pane = kit.ui.scene.glass(lay.pane.w as u32, lay.pane.h as u32, z + 1, glass_of(kit, PaneKind::Sheet, lay.pane.h, false, false), Props::at(lay.pane.x, lay.pane.y));
        let mut art = kit.ui.scene.image(lay.pane.w as u32, lay.pane.h as u32, z + 3, Props::at(lay.pane.x, lay.pane.y))?;
        art.clear();
        art.damage(None);
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let name = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let sub = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink2, Align::Left);
        let reading_role = if m.scale <= 1.25 { TypeRole::Display } else { TypeRole::Title };
        let reading = role_style(kit, reading_role, Some(Weight::SemiBold), ink, Align::Left);
        let value = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Centre);
        let chip = role_style(kit, TypeRole::Caption, None, ink, Align::Centre);
        let text = |k: &str| tile.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut lines = vec![
            (lay.name, name, text("name")),
            (lay.sub, sub, text("area")),
            (lay.now[0], label, "In the room".to_owned()),
            (lay.now[1], reading, String::new()),
            (lay.now[2], sub, String::new()),
            (lay.now[3], sub, String::new()),
        ];
        for s in &lay.sets {
            lines.push((s.label, label, String::new()));
            lines.push((s.value, value, String::new()));
        }
        lines.push((lay.mode_label, label, if lay.chips.is_empty() { String::new() } else { "Mode".to_owned() }));
        for f in &lay.chips {
            lines.push((Frame::new(f.x + 6.0, f.y + ((f.h - m.caption) / 2.0).floor(), f.w - 12.0, m.caption), chip, String::new()));
        }
        let mut words = Words::new(kit, Frame::new(0.0, 0.0, lay.pane.w, lay.pane.h), z + 4, lines)?;
        words.set_props(Props::at(lay.pane.x, lay.pane.y));
        let ground = [kit.ui.look.canvas().0, kit.ui.look.canvas().1, kit.ui.look.canvas().2];
        let mut me = Self { _scrim: scrim, _pane: pane, art, words, layout: lay, id: sheet.id.clone(), shape: (shown, sheet.modes.clone()), drawn: None, sub: text("area"), ground };
        me.update(kit, sheet, tile, None)?;
        Ok(me)
    }

    /// Whether this is the sheet of `id` with these set points and modes (a thermostat that is switched off while the sheet is up has no set point
    /// to show any more, and needs another).
    pub fn is_for(&self, id: &str, sheet: &ThermostatSheet) -> bool {
        self.id == id && self.shape.0 == sheet.shown() && self.shape.1 == sheet.modes
    }

    /// Paints what the sheet's state changed and tells the compositor those parts; `notice` is what Home Assistant refused, said in place of the
    /// room.
    pub fn update(&mut self, kit: &mut Kit, sheet: &ThermostatSheet, tile: &Value, notice: Option<&str>) -> std::io::Result<()> {
        let status = notice.map(str::to_owned).or_else(|| (!sheet.why.is_empty() && sheet.rows.is_empty()).then(|| sheet.why.clone())).unwrap_or_else(|| tile.get("area").and_then(Value::as_str).unwrap_or("").to_owned());
        if status != self.sub {
            let colour = if notice.is_some() { kit.ui.look.ink() } else { kit.ui.look.ink2() };
            self.words.set_many(kit, &[(1, status.as_str(), colour)]);
            self.sub = status;
        }
        let look = look_of(sheet);
        if self.drawn.as_ref() == Some(&look) {
            return Ok(());
        }
        let before = self.drawn.replace(look.clone());
        let first = before.is_none();
        let (ink, ink2, accent, on) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), kit.ui.look.accent_ink());
        // the canvas takes a colour straight and applies its alpha itself, so the ink goes in whole
        let wash = Rgba(ink.0, ink.1, ink.2, crate::kit::widgets::focus_wash(kit).3);
        let radius = kit.tokens.radius(Radius::Row);
        let ink_rgb = [ink.0, ink.1, ink.2];
        let (rest, dim) = (paint::mix(self.ground, ink_rgb, 0.14), paint::mix(self.ground, ink_rgb, 0.06));
        let lay = self.layout.clone();
        let (w, h) = (self.art.width(), self.art.height());
        let mut dirty: Vec<Rect> = Vec::new();
        let rect = |f: Frame| Rect { x: f.x.floor().max(0.0), y: f.y.floor().max(0.0), w: f.w.ceil() + 2.0, h: f.h.ceil() + 2.0 };
        let editable = |r: Row| sheet.rows.contains(&r);
        {
            let px = self.art.pixels_mut();
            for (i, s) in lay.sets.iter().enumerate() {
                let front_of = |l: &Look| s.row.is_some() && l.at == s.row;
                if !first && before.as_ref().is_some_and(|b| b.sets.get(i) == look.sets.get(i) && front_of(b) == front_of(&look)) {
                    continue;
                }
                clear(px, w, h, s.band);
                let mut canvas = Canvas { px: &mut *px, w, h };
                let front = front_of(&look);
                if front {
                    canvas.rounded(s.band, radius, wash);
                }
                if let (Some(_), Some((_, _, at_min, at_max))) = (s.row.filter(|r| editable(*r)), look.sets.get(i)) {
                    for (button, plus, held) in [(s.minus, false, *at_min), (s.plus, true, *at_max)] {
                        // the steps of the set point in front are the accent's; one held at an end of its range is dimmed and its glyph with it
                        let (fill, mark) = match (front, held) {
                            (_, true) => (straight(dim), Rgba(ink.0, ink.1, ink.2, 90)),
                            (true, false) => (accent, on),
                            (false, false) => (straight(rest), ink),
                        };
                        canvas.disc(button.x + button.w / 2.0, button.y + button.h / 2.0, button.w / 2.0, fill);
                        glyph(&mut canvas, button, plus, mark);
                    }
                }
                dirty.push(rect(s.band));
            }
            // the modes: the band washed when its row is in front, the chips on it
            if !lay.chips.is_empty() {
                let front = look.at == Some(Row::Mode);
                let moved = before.as_ref().is_none_or(|b| (&b.mode, b.cursor, b.at == Some(Row::Mode)) != (&look.mode, look.cursor, front));
                if moved {
                    clear(px, w, h, lay.mode_band);
                    let mut canvas = Canvas { px: &mut *px, w, h };
                    if front {
                        canvas.rounded(lay.mode_band, radius, wash);
                    }
                    for (k, c) in lay.chips.iter().enumerate() {
                        let current = sheet.modes.get(k).is_some_and(|m| *m == look.mode);
                        let r = c.h / 2.0;
                        if front && k == look.cursor {
                            canvas.rounded(Frame::new(c.x - 4.0, c.y - 4.0, c.w + 8.0, c.h + 8.0), r + 4.0, if current { accent } else { ink });
                            canvas.rounded(Frame::new(c.x - 1.5, c.y - 1.5, c.w + 3.0, c.h + 3.0), r + 1.5, straight(self.ground));
                        }
                        canvas.rounded(*c, r, if current { accent } else { straight(rest) });
                    }
                    dirty.push(rect(lay.mode_band));
                }
            }
        }
        for d in dirty {
            self.art.damage(Some(Rect { w: d.w.min(w as f32 - d.x), h: d.h.min(h as f32 - d.y), ..d }));
        }
        // the words
        let mut said: Vec<(usize, String, Rgba)> = vec![
            (3, sheet.current.map_or_else(|| "\u{2014}".to_owned(), ThermostatSheet::degrees), ink),
            (4, sheet.action_words().to_owned(), ink2),
            (5, sheet.humidity_words().unwrap_or_default(), ink2),
        ];
        let width = |f: Frame| f.w;
        for (i, s) in lay.sets.iter().enumerate() {
            let front = s.row.is_some() && look.at == s.row;
            match s.row {
                Some(row) => {
                    said.push((HEAD + 2 * i, label_of(row).to_owned(), if front { ink } else { ink2 }));
                    said.push((HEAD + 2 * i + 1, sheet.value(row).map_or_else(|| "\u{2014}".to_owned(), ThermostatSheet::degrees), ink));
                }
                // no set point to show (it is off, or fan only): the word for the mode in its place
                None => {
                    said.push((HEAD + 2 * i, String::new(), ink2));
                    said.push((HEAD + 2 * i + 1, ThermostatSheet::mode_word(&sheet.mode), ink));
                }
            }
        }
        let base = HEAD + 2 * lay.sets.len();
        if !lay.chips.is_empty() {
            said.push((base, "Mode".to_owned(), if look.at == Some(Row::Mode) { ink } else { ink2 }));
        }
        let caption = kit.ty(TypeRole::Caption);
        for (k, c) in lay.chips.iter().enumerate() {
            let name = sheet.modes.get(k).map(|m| ThermostatSheet::mode_word(m)).unwrap_or_default();
            let name = ui::components::text::ellipsise(&mut kit.ui.fonts, &name, caption, width(*c) - 12.0);
            let current = sheet.modes.get(k).is_some_and(|m| *m == look.mode);
            said.push((base + 1 + k, name, if current { on } else { ink }));
        }
        let refs: Vec<(usize, &str, Rgba)> = said.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.words.set_many(kit, &refs);
        Ok(())
    }

    /// The lint's view: the pane, and every line of words on it.
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, paint: Rgba) {
        let panel = lint.panel("thermostat sheet", self.layout.pane, paint, true);
        // a chip's words are read on the chip: the accent's ink for the mode it is in, the ink on a wash for the rest
        let (ink, accent) = (kit.ui.look.ink(), kit.ui.look.accent());
        let rest = straight(paint::mix(self.ground, [ink.0, ink.1, ink.2], 0.14));
        let current = self.drawn.as_ref().map(|l| l.mode.clone());
        let modes = &self.shape.1;
        let pane = self.layout.pane;
        let base = HEAD + 2 * self.layout.sets.len() + 1;
        let chips: Vec<usize> = self
            .layout
            .chips
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let on = current.as_ref().is_some_and(|m| modes.get(k) == Some(m));
                lint.panel_on("thermostat sheet chip", Frame::new(pane.x + c.x, pane.y + c.y, c.w, c.h), if on { accent } else { rest }, false, panel)
            })
            .collect();
        self.words.lint_by(kit, lint, "thermostat sheet", (0.0, 0.0), |i| Some(i.checked_sub(base).and_then(|k| chips.get(k).copied()).unwrap_or(panel)));
    }
}

/// Clears `f` of the picture (`w` by `h`) to nothing.
fn clear(px: &mut [u8], w: u32, h: u32, f: Frame) {
    let (x0, x1) = (f.x.floor().max(0.0) as u32, (f.right().ceil().max(0.0) as u32).min(w));
    let (y0, y1) = (f.y.floor().max(0.0) as u32, (f.bottom().ceil().max(0.0) as u32).min(h));
    for y in y0..y1 {
        let a = ((y * w + x0) * 4) as usize;
        let b = ((y * w + x1) * 4) as usize;
        px[a..b].fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: Frame, b: Frame) -> bool {
        a.w > 0.0 && a.h > 0.0 && b.w > 0.0 && b.h > 0.0 && a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    /// At either text size, for every set of set points and modes a thermostat has: the sheet is on the screen, nothing on it overlaps another part,
    /// and every button and chip has room to be pointed at.
    #[test]
    fn nothing_on_the_thermostat_sheet_overlaps_or_leaves_it_at_either_text_size() {
        for scale in [1.0f32, 1.5, 2.0] {
            let m = Metrics::nominal(scale);
            for shown in [&[][..], &[Row::Set], &[Row::Low, Row::High]] {
                for (chips, chars) in [(0usize, 0usize), (2, 3), (4, 9), (6, 9), (9, 9)] {
                    let l = layout(&m, shown, chips, chars);
                    let tag = format!("scale {scale} shown {shown:?} chips {chips}");
                    assert!(l.pane.x >= 0.0 && l.pane.y >= 54.0 - 0.01 && l.pane.right() <= 1920.0 && l.pane.bottom() <= 1080.0 - 54.0 + 0.01, "{tag}: {:?}", l.pane);
                    let inside = |f: Frame| f.x >= -0.01 && f.y >= -0.01 && f.right() <= l.pane.w + 0.01 && f.bottom() <= l.pane.h + 0.01;
                    assert!(inside(l.name) && inside(l.sub) && l.now.iter().all(|f| inside(*f)), "{tag}");
                    assert!(!overlaps(l.name, l.sub) && l.now.iter().all(|f| !overlaps(l.name, *f) && !overlaps(l.sub, *f)), "{tag}");
                    for (a, b) in l.now.iter().zip(l.now.iter().skip(1)) {
                        assert!(!overlaps(*a, *b), "{tag}: {a:?} {b:?}");
                    }
                    assert_eq!(l.sets.len(), shown.len().max(1), "{tag}");
                    for s in &l.sets {
                        assert!(inside(s.band) && inside(s.label) && inside(s.minus) && inside(s.value) && inside(s.plus), "{tag}: {s:?}");
                        assert!(!overlaps(s.label, s.minus) && !overlaps(s.minus, s.value) && !overlaps(s.value, s.plus), "{tag}: {s:?}");
                        assert!(s.minus.w >= 40.0 && s.plus.w >= 40.0, "{tag}: a button to point at");
                        // the room on the left is clear of the set points
                        assert!(l.now.iter().all(|f| !overlaps(*f, s.band)), "{tag}");
                    }
                    for (a, b) in l.sets.iter().zip(l.sets.iter().skip(1)) {
                        assert!(!overlaps(a.band, b.band), "{tag}");
                    }
                    assert_eq!(l.chips.len(), chips.min(MAX_CHIPS), "{tag}");
                    if chips > 0 {
                        assert!(inside(l.mode_band) && inside(l.mode_label), "{tag}");
                        assert!(l.sets.iter().all(|s| !overlaps(s.band, l.mode_band)) && l.now.iter().all(|f| !overlaps(*f, l.mode_band)), "{tag}");
                    }
                    for c in &l.chips {
                        assert!(inside(*c) && c.w >= 40.0 && c.h >= 30.0, "{tag}: {c:?}");
                        assert!(c.x >= l.mode_label.right() && c.y >= l.mode_band.y && c.bottom() <= l.mode_band.bottom(), "{tag}: {c:?} {:?}", l.mode_band);
                    }
                    for (i, a) in l.chips.iter().enumerate() {
                        assert!(l.chips.iter().skip(i + 1).all(|b| !overlaps(*a, *b)), "{tag}: {a:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_pointer_finds_a_step_and_a_chip_and_only_those_the_keys_reach() {
        use super::super::owner_tile;
        let m = Metrics::nominal(1.0);
        let tile = owner_tile("climate.living_room_thermostat");
        let sheet = ThermostatSheet::new("climate.living_room_thermostat", &tile);
        let l = layout(&m, &sheet.shown(), sheet.modes.len(), chars_of(&sheet.modes));
        let at = |f: Frame| (l.pane.x + f.x + f.w / 2.0, l.pane.y + f.y + f.h / 2.0);
        let (x, y) = at(l.sets[0].plus);
        assert_eq!(hit(&l, &sheet, x, y), Some(Hit::Plus(Row::Low)));
        let (x, y) = at(l.sets[1].minus);
        assert_eq!(hit(&l, &sheet, x, y), Some(Hit::Minus(Row::High)));
        let (x, y) = at(l.chips[2]);
        assert_eq!(hit(&l, &sheet, x, y), Some(Hit::Chip(2)));
        let (x, y) = at(l.now[1]);
        assert_eq!(hit(&l, &sheet, x, y), None, "the reading is not a button");
        // a thermostat Home Assistant lets the TV do nothing to has no buttons and no chips to press
        let mut shown = tile;
        shown["verbs"] = json!([]);
        let read_only = ThermostatSheet::new("climate.living_room_thermostat", &shown);
        let (x, y) = at(l.sets[0].plus);
        assert_eq!(hit(&l, &read_only, x, y), None);
        let (x, y) = at(l.chips[2]);
        assert_eq!(hit(&l, &read_only, x, y), None);
    }

    use serde_json::json;
}
