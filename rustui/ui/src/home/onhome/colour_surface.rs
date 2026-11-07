//! The colour sheet of a lamp drawn: a pane over Home with the lamp's name, on the left what the lamp shows (a disc of its colour at its
//! brightness, with the ring of hues round it for a lamp that has colours) and on the right one row for each thing it can change: a track each
//! for hue, saturation, brightness and warmth, a row of colours worth having, and the effects it has.
//!
//! The pane has one picture and one layer of words. The ring is rastered once; a change repaints what it changes (the disc, the tracks, the
//! row in front) and tells the compositor those parts only. The layout is a pure function of the type's measures ([`layout`]), so that nothing
//! in it overlaps at either text size is a test, and so that the pointer's place on it is a pure function too ([`hit`]).

use super::colour::{ColourSheet, Mode, Preset, Row, hsv_to_rgb};
use super::layout::Metrics;
use super::{card, paint};
use crate::kit::pane::glass_of;
use crate::home::widgets::{Canvas, Words};
use crate::kit::Kit;
use crate::kit::button::faded;
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
/// The effects shown at once, as chips: the one in front and its neighbours.
const CHIPS: usize = 3;
/// The colours worth having: three whites and five colours (`colour.rs`).
const SWATCHES: usize = 8;
/// How much of the ink an effect's chip is washed with when it is not the one in front.
const CHIP_REST: f32 = 0.10;

/// The ring of hues round the lamp's disc: its centre in the pane, and the radii of its two edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub cx: f32,
    pub cy: f32,
    pub outer: f32,
    pub inner: f32,
}

/// Where one row of the sheet is, in the pane's own space.
#[derive(Clone, Debug, PartialEq)]
pub struct RowAt {
    pub row: Row,
    /// The row's whole box, which the focus washes.
    pub band: Frame,
    pub label: Frame,
    pub value: Frame,
    /// A track's line (its ends are the ends of its range), or the room the swatches or the chips are in.
    pub track: Frame,
    /// The swatches or the chips, left to right.
    pub cells: Vec<Frame>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pane: Frame,
    pub name: Frame,
    pub sub: Frame,
    pub ring: Option<Ring>,
    /// The disc of the lamp's colour: its centre and radius.
    pub disc: (f32, f32, f32),
    pub rows: Vec<RowAt>,
}

/// The sheet's frames for a type of these measures and the rows a lamp has: `chips` is how many effects there are to show.
pub fn layout(m: &Metrics, rows: &[Row], chips: usize) -> Layout {
    let s = m.scale;
    let width = (1200.0 * s.min(1.4)).round().min(1700.0);
    let inner = width - 2.0 * PAD;
    let head_h = m.title + m.caption;
    let has_ring = rows.contains(&Row::Hue);
    let d = ((if has_ring { 300.0 } else { 220.0 }) * s.min(1.3)).round();
    let row_h = (m.label + 24.0).max(56.0);
    let gap = 8.0;
    let body = rows.len() as f32 * row_h + (rows.len().max(1) as f32 - 1.0) * gap;
    let y0 = PAD + head_h + GAP;
    let max_h = 1080.0 - 2.0 * 54.0;
    let height = (y0 + body.max(d) + PAD).min(max_h);
    let (px, py) = (((1920.0 - width) / 2.0).round(), ((1080.0 - height) / 2.0).round().max(54.0));
    let pane = Frame::new(px, py, width, height);
    let area_h = height - y0 - PAD;
    let (cx, cy) = (PAD + d / 2.0, y0 + (area_h / 2.0).floor());
    let ring = has_ring.then(|| Ring { cx, cy, outer: d / 2.0, inner: d / 2.0 - (d * 0.115).round() });
    let disc_r = ring.map_or(d / 2.0 - 8.0, |r| r.inner - 14.0).floor();
    let right_x = PAD + d + 2.0 * GAP;
    let right_w = inner - d - 2.0 * GAP;
    let label_w = (m.label * 4.2).round();
    // (the longest of the whites' names, "Neutral white", stands in it: a value starts where the track ends)
    let value_w = (m.caption * 5.4).round();
    let top = y0 + ((area_h - body) / 2.0).floor().max(0.0);
    let rows = rows
        .iter()
        .enumerate()
        .map(|(i, &row)| {
            let y = top + i as f32 * (row_h + gap);
            let band = Frame::new(right_x, y, right_w, row_h);
            let label = Frame::new(band.x + 16.0, y + ((row_h - m.label) / 2.0).floor(), label_w, m.label);
            let value = Frame::new(band.right() - 16.0 - value_w, y + ((row_h - m.caption) / 2.0).floor(), value_w, m.caption);
            let track_x = label.right() + GAP;
            let track_w = (value.x - GAP - track_x).max(40.0);
            let track = match row {
                Row::Hue | Row::Saturation | Row::Brightness | Row::Temperature => Frame::new(track_x, y + ((row_h - 12.0) / 2.0).floor(), track_w, 12.0),
                Row::Preset | Row::Effect => Frame::new(track_x, y + 8.0, track_w, row_h - 16.0),
            };
            let cells = match row {
                Row::Preset => {
                    let dia = ((track_w - (SWATCHES as f32 - 1.0) * 8.0) / SWATCHES as f32).floor().min(40.0 * s.min(1.3)).max(16.0);
                    let x = track.x + ((track_w - SWATCHES as f32 * dia - (SWATCHES as f32 - 1.0) * 8.0) / 2.0).floor();
                    (0..SWATCHES).map(|k| Frame::new(x + k as f32 * (dia + 8.0), track.y + ((track.h - dia) / 2.0).floor(), dia, dia)).collect()
                }
                Row::Effect => {
                    // as many chips as hold a name ("Rainbow") whole, the effect in front among them
                    let fit = (((track_w + 8.0) / ((m.caption * 3.1 + 8.0).round() + 8.0)).floor() as usize).max(1);
                    let n = chips.min(CHIPS).min(fit);
                    let w = if n == 0 { 0.0 } else { ((track_w - (n as f32 - 1.0) * 8.0) / n as f32).floor() };
                    let h = (m.caption + 16.0).min(track.h);
                    (0..n).map(|k| Frame::new(track.x + k as f32 * (w + 8.0), track.y + ((track.h - h) / 2.0).floor(), w, h)).collect()
                }
                _ => Vec::new(),
            };
            RowAt { row, band, label, value, track, cells }
        })
        .collect();
    Layout { pane, name: Frame::new(PAD, PAD, inner, m.title), sub: Frame::new(PAD, PAD + m.title, inner, m.caption), ring, disc: (cx, cy, disc_r), rows }
}

/// Which of the effects the chips begin at: the one in front is the middle chip, as near the ends as there is room (`shown` chips of `total`).
pub fn chip_window(selected: Option<usize>, total: usize, shown: usize) -> usize {
    let shown = total.min(shown);
    selected.map_or(0, |e| e.saturating_sub(shown / 2)).min(total - shown)
}

/// What the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// The ring: the hue at that angle, 0 to 360 clockwise from the top.
    Ring(f32),
    /// A track: how far along, 0 to 1.
    Track(Row, f32),
    /// A swatch, or a chip (an index into the effects): by place.
    Swatch(usize),
    Chip(usize),
}

/// What is under the point (`x`, `y`) of the screen on a sheet laid out as `lay`; `window` is where the chips begin ([`chip_window`]).
pub fn hit(lay: &Layout, window: usize, x: f32, y: f32) -> Option<Hit> {
    let (x, y) = (x - lay.pane.x, y - lay.pane.y);
    if let Some(r) = lay.ring {
        let (dx, dy) = (x - r.cx, y - r.cy);
        let d = (dx * dx + dy * dy).sqrt();
        // the ring, with a little room round it for a thumb that is not exact
        if d >= r.inner - 4.0 && d <= r.outer + 8.0 {
            return Some(Hit::Ring(dx.atan2(-dy).to_degrees().rem_euclid(360.0)));
        }
    }
    for r in &lay.rows {
        match r.row {
            Row::Preset => {
                if let Some(k) = r.cells.iter().position(|c| c.contains(x, y)) {
                    return Some(Hit::Swatch(k));
                }
            }
            Row::Effect => {
                if let Some(k) = r.cells.iter().position(|c| c.contains(x, y)) {
                    return Some(Hit::Chip(window + k));
                }
            }
            row => {
                // the track and the room round it: a row's whole band is where its thumb is taken
                let t = Frame::new(r.track.x - 12.0, r.band.y, r.track.w + 24.0, r.band.h);
                if t.contains(x, y) {
                    return Some(Hit::Track(row, ((x - r.track.x) / r.track.w).clamp(0.0, 1.0)));
                }
            }
        }
    }
    None
}

/// The pointer's value on a track once it is held (the thumb is followed outside the track too): how far along, 0 to 1.
pub fn along(lay: &Layout, row: Row, x: f32) -> f32 {
    lay.rows.iter().find(|r| r.row == row).map_or(0.0, |r| ((x - lay.pane.x - r.track.x) / r.track.w).clamp(0.0, 1.0))
}

/// The hue of the pointer's place on the ring, held (the thumb goes round wherever the pointer is).
pub fn round(lay: &Layout, x: f32, y: f32) -> Option<f32> {
    let r = lay.ring?;
    Some((x - lay.pane.x - r.cx).atan2(-(y - lay.pane.y - r.cy)).to_degrees().rem_euclid(360.0))
}

// ---- what is painted

/// The ring of hues as premultiplied pixels, `side` by `side`: pure colours all the way round, clockwise from the top, an anti-aliased edge each side.
fn ring_pixels(side: u32, outer: f32, inner: f32) -> Vec<u8> {
    let mut px = vec![0u8; (side * side * 4) as usize];
    let c = side as f32 / 2.0;
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let cover = (outer - d + 0.5).clamp(0.0, 1.0).min((d - inner + 0.5).clamp(0.0, 1.0));
            if cover <= 0.0 {
                continue;
            }
            let rgb = hsv_to_rgb(dx.atan2(-dy).to_degrees().rem_euclid(360.0), 1.0, 1.0);
            let i = ((y * side + x) * 4) as usize;
            for k in 0..3 {
                px[i + k] = (f32::from(rgb[k]) * cover).round() as u8;
            }
            px[i + 3] = (255.0 * cover).round() as u8;
        }
    }
    px
}

/// A horizontal gradient of colours in a rounded track of `track` (in the picture's pixels): `at` gives the colour a share of the way along.
/// An end too near the sheet's `ground` to see (white on the light sheet, the dark end of a brightness track on the dark one) puts a hairline of
/// `rim` round the whole track, so that its extent reads.
fn gradient_track(canvas: &mut Canvas, track: Frame, at: &dyn Fn(f32) -> [u8; 3], ground: [u8; 3], rim: Rgba) {
    let r = track.h / 2.0;
    if paint::contrast(at(0.0), ground) < 3.0 || paint::contrast(at(1.0), ground) < 3.0 {
        canvas.rounded(Frame::new(track.x - 1.0, track.y - 1.0, track.w + 2.0, track.h + 2.0), r + 1.0, rim);
    }
    let (x0, x1) = (track.x.floor() as i32, track.right().ceil() as i32);
    for x in x0..x1 {
        let t = ((x as f32 + 0.5 - track.x) / track.w).clamp(0.0, 1.0);
        let rgb = at(t);
        // one column of the track: its rounded ends are the disc's
        let col = Frame::new(x as f32, track.y, 1.0, track.h);
        let end = r - (x as f32 + 0.5 - track.x).min(track.right() - (x as f32 + 0.5)).clamp(0.0, r);
        let inset = r - (r * r - end * end).max(0.0).sqrt();
        let col = Frame::new(col.x, col.y + inset, 1.0, (col.h - 2.0 * inset).max(0.0));
        if col.h > 0.0 {
            // (a radius of half the column's width: with none, `rounded` covers each pixel by half)
            canvas.rounded(col, 0.5, Rgba(rgb[0], rgb[1], rgb[2], 255));
        }
    }
}

fn straight(rgb: [u8; 3]) -> Rgba {
    Rgba(rgb[0], rgb[1], rgb[2], 255)
}

/// How the lamp is shown on the disc: its colour at its brightness over the glass, never quite dark.
fn shown(rgb: [u8; 3], level: f32, ground: [u8; 3]) -> [u8; 3] {
    paint::mix(ground, rgb, 0.12 + 0.88 * level.clamp(0.0, 1.0))
}

/// The name of a colour worth having.
pub fn preset_name(p: Preset) -> &'static str {
    match p {
        Preset::White(k) if k <= 3200 => "Warm white",
        Preset::White(k) if k <= 4800 => "Neutral white",
        Preset::White(_) => "Cool white",
        Preset::Colour(c) => match c {
            [255, 60, 48] => "Red",
            [255, 159, 10] => "Orange",
            [48, 209, 88] => "Green",
            [10, 132, 255] => "Blue",
            [191, 90, 242] => "Purple",
            _ => "Colour",
        },
    }
}

/// What a row says at its right: its value in words.
pub fn value_of(sheet: &ColourSheet, row: Row) -> String {
    match row {
        Row::Hue => format!("{}\u{b0}", sheet.hue.round() as i32),
        Row::Saturation => format!("{}%", (sheet.sat * 100.0).round() as i32),
        Row::Brightness => format!("{}%", (sheet.level * 100.0).round() as i32),
        Row::Temperature => format!("{} K", sheet.kelvin.round() as i32),
        Row::Preset => sheet.presets.get(sheet.preset).map_or(String::new(), |p| preset_name(*p).to_owned()),
        Row::Effect => sheet.effect.and_then(|e| sheet.effects.get(e)).cloned().unwrap_or_default(),
    }
}

fn label_of(row: Row) -> &'static str {
    match row {
        Row::Hue => "Colour",
        Row::Saturation => "Saturation",
        Row::Brightness => "Brightness",
        Row::Temperature => "Warmth",
        Row::Preset => "Presets",
        Row::Effect => "Effect",
    }
}

/// What the painted parts depend on: a change of it repaints them.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    hue: i32,
    sat: i32,
    level: i32,
    kelvin: i32,
    colour: bool,
    at: usize,
    preset: usize,
    effect: Option<usize>,
}

fn look_of(s: &ColourSheet) -> Look {
    Look { hue: (s.hue * 2.0).round() as i32, sat: (s.sat * 100.0).round() as i32, level: (s.level * 100.0).round() as i32, kelvin: (s.kelvin / 10.0).round() as i32, colour: s.mode == Mode::Colour, at: s.at, preset: s.preset, effect: s.effect }
}

pub struct ColourSurface {
    _scrim: ShapeLayer,
    _pane: GlassLayer,
    art: ImageLayer,
    words: Words,
    pub layout: Layout,
    id: String,
    rows: Vec<Row>,
    ring: Vec<u8>,
    ring_side: u32,
    drawn: Option<Look>,
    window: usize,
    /// The line under the name as it was last said: the lamp's status, or what Home Assistant refused.
    sub: String,
    /// The paint of the pane under the glass, for the disc to lie on.
    ground: [u8; 3],
}

impl ColourSurface {
    pub fn new(kit: &mut Kit, sheet: &ColourSheet, tile: &Value, z: i32) -> std::io::Result<Self> {
        let m = card::metrics_of(kit);
        let lay = layout(&m, &sheet.rows, sheet.effects.len());
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
        let value = role_style(kit, TypeRole::Caption, None, ink, Align::Left);
        let chip = role_style(kit, TypeRole::Caption, None, ink, Align::Centre);
        let text = |k: &str| tile.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut lines = vec![(lay.name, name, text("name")), (lay.sub, sub, text("words"))];
        // for each row its label and value; the effect row's chips after them
        for r in &lay.rows {
            lines.push((r.label, label, label_of(r.row).to_owned()));
            lines.push((r.value, value, String::new()));
        }
        let chips = lay.rows.iter().find(|r| r.row == Row::Effect).map(|r| r.cells.clone()).unwrap_or_default();
        for f in &chips {
            lines.push((Frame::new(f.x + 6.0, f.y + ((f.h - m.caption) / 2.0).floor(), f.w - 12.0, m.caption), chip, String::new()));
        }
        let mut words = Words::new(kit, Frame::new(0.0, 0.0, lay.pane.w, lay.pane.h), z + 4, lines)?;
        words.set_props(Props::at(lay.pane.x, lay.pane.y));
        let ground = [kit.ui.look.canvas().0, kit.ui.look.canvas().1, kit.ui.look.canvas().2];
        let ring_side = lay.ring.map_or(0, |r| (r.outer * 2.0).ceil() as u32);
        let ring = lay.ring.map_or_else(Vec::new, |r| ring_pixels(ring_side, r.outer, r.inner));
        let mut me = Self { _scrim: scrim, _pane: pane, art, words, layout: lay, id: sheet.id.clone(), rows: sheet.rows.clone(), ring, ring_side, drawn: None, window: 0, sub: text("words"), ground };
        me.update(kit, sheet, tile, None)?;
        Ok(me)
    }

    /// Whether this is the sheet of `id` with these rows (a lamp that loses its colour while the sheet is up needs another).
    pub fn is_for(&self, id: &str, sheet: &ColourSheet) -> bool {
        self.id == id && self.rows == sheet.rows
    }

    pub fn window(&self) -> usize {
        self.window
    }

    /// Paints what the sheet's state changed and tells the compositor those parts; `notice` is what Home Assistant refused, said in place of the
    /// lamp's status.
    pub fn update(&mut self, kit: &mut Kit, sheet: &ColourSheet, tile: &Value, notice: Option<&str>) -> std::io::Result<()> {
        let sub = notice.map_or_else(|| tile.get("words").and_then(Value::as_str).unwrap_or("").to_owned(), str::to_owned);
        if sub != self.sub {
            let colour = if notice.is_some() { kit.ui.look.ink() } else { kit.ui.look.ink2() };
            self.words.set_many(kit, &[(1, sub.as_str(), colour)]);
            self.sub = sub;
        }
        let look = look_of(sheet);
        if self.drawn.as_ref() == Some(&look) {
            return Ok(());
        }
        let before = self.drawn.replace(look.clone());
        let first = before.is_none();
        let colour_moved = before.as_ref().is_none_or(|b| (b.hue, b.sat, b.colour, b.kelvin, b.level) != (look.hue, look.sat, look.colour, look.kelvin, look.level));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        // the canvas takes a colour straight and applies its alpha itself, so the ink goes in whole: `faded` (premultiplied) would be shaded
        // twice, and the focus wash of the dark theme (light ink) came out black and invisible
        let wash = Rgba(ink.0, ink.1, ink.2, crate::kit::widgets::focus_wash(kit).3);
        let track = Rgba(ink.0, ink.1, ink.2, (255.0 * CHIP_REST).round() as u8);
        // half the ink: 3:1 against the sheet in both themes
        let rim = Rgba(ink.0, ink.1, ink.2, 128);
        let radius = kit.tokens.radius(Radius::Row);
        let lay = self.layout.clone();
        let (w, h) = (self.art.width(), self.art.height());
        let rgb = sheet.rgb();
        let mut dirty: Vec<Rect> = Vec::new();
        let rect = |f: Frame| Rect { x: f.x.floor().max(0.0), y: f.y.floor().max(0.0), w: f.w.ceil() + 2.0, h: f.h.ceil() + 2.0 };
        {
            let canvas_px = self.art.pixels_mut();
            // the lamp: the ring and a disc of what it shows, with its light round it
            if colour_moved {
                let (cx, cy, r) = lay.disc;
                let side = (r * 2.0 + 2.0 * (r * 0.9)).ceil();
                let area = Frame::new((cx - side / 2.0).floor(), (cy - side / 2.0).floor(), side, side);
                let left = lay.ring.map_or(area, |g| Frame::new(g.cx - g.outer - 2.0, g.cy - g.outer - 2.0, g.outer * 2.0 + 4.0, g.outer * 2.0 + 4.0));
                clear(canvas_px, w, h, left);
                if let Some(g) = lay.ring {
                    let x = (g.cx - g.outer).round() as i32;
                    let y = (g.cy - g.outer).round() as i32;
                    paint::lay(canvas_px, w, h, &self.ring, self.ring_side, self.ring_side, x, y);
                }
                let lamp = shown(rgb, sheet.level, self.ground);
                // its light: a glow that reaches past the disc, held inside the room the disc has
                {
                    let keep = paint::Keep { x: left.x, y: left.y, w: left.w, h: left.h, radius: left.w / 2.0, soft: 0.0 };
                    let strength = 0.20 + 0.40 * sheet.level.clamp(0.0, 1.0);
                    paint::glow(canvas_px, w, h, cx, cy, r * 1.0 + (r * 0.5), rgb, strength, keep);
                }
                let mut canvas = Canvas { px: canvas_px, w, h };
                canvas.disc(cx, cy, r, straight(lamp));
                // a thumb on the ring at the hue
                if let (Some(g), true) = (lay.ring, look.colour || sheet.row() == Some(Row::Hue)) {
                    let mid = (g.outer + g.inner) / 2.0;
                    let a = sheet.hue.to_radians();
                    let (tx, ty) = (g.cx + mid * a.sin(), g.cy - mid * a.cos());
                    let tr = (g.outer - g.inner) / 2.0 + 2.0;
                    canvas.disc(tx, ty, tr + 2.0, Rgba(0, 0, 0, 80));
                    canvas.disc(tx, ty, tr, straight([255, 255, 255]));
                    canvas.disc(tx, ty, tr - 4.0, straight(hsv_to_rgb(sheet.hue, 1.0, 1.0)));
                }
                dirty.push(rect(left));
            }
            // the rows: the one in front on a wash, the track or the swatches or the chips on it
            for (i, r) in lay.rows.iter().enumerate() {
                let dep = |l: &Look| -> (i32, i32, i32, i32, bool, bool, usize, Option<usize>) {
                    let front = l.at == i;
                    match r.row {
                        Row::Hue => (l.hue, 0, 0, 0, l.colour, front, 0, None),
                        Row::Saturation => (l.hue, l.sat, 0, 0, l.colour, front, 0, None),
                        Row::Brightness => (l.hue, l.sat, l.level, l.kelvin, l.colour, front, 0, None),
                        Row::Temperature => (0, 0, 0, l.kelvin, l.colour, front, 0, None),
                        Row::Preset => (0, 0, 0, 0, false, front, l.preset, None),
                        Row::Effect => (0, 0, 0, 0, false, front, 0, l.effect),
                    }
                };
                if !first && before.as_ref().is_some_and(|b| dep(b) == dep(&look)) {
                    continue;
                }
                let band = r.band;
                clear(canvas_px, w, h, band);
                let mut canvas = Canvas { px: canvas_px, w, h };
                if look.at == i {
                    canvas.rounded(band, radius, wash);
                }
                let tr = r.track;
                match r.row {
                    Row::Hue => gradient_track(&mut canvas, tr, &|t| hsv_to_rgb(t * 360.0, 1.0, 1.0), self.ground, rim),
                    Row::Saturation => gradient_track(&mut canvas, tr, &|t| hsv_to_rgb(sheet.hue, t, 1.0), self.ground, rim),
                    Row::Brightness => gradient_track(&mut canvas, tr, &|t| shown(rgb, t.max(0.01), self.ground), self.ground, rim),
                    Row::Temperature => {
                        let (lo, hi) = sheet.kelvin_range;
                        gradient_track(&mut canvas, tr, &|t| paint::kelvin_rgb(lo + t * (hi - lo)), self.ground, rim);
                    }
                    Row::Preset => {
                        for (k, c) in r.cells.iter().enumerate() {
                            let Some(p) = sheet.presets.get(k) else { continue };
                            let (cx, cy, rr) = (c.x + c.w / 2.0, c.y + c.h / 2.0, c.w / 2.0);
                            if k == sheet.preset {
                                canvas.disc(cx, cy, rr + 4.0, if look.at == i { accent } else { ink });
                                canvas.disc(cx, cy, rr + 1.5, straight(self.ground));
                            }
                            // a colour too near the sheet's ground to see (the white on the light sheet) is ringed with ink, so that its edge reads
                            if paint::contrast(p.rgb(), self.ground) < 3.0 {
                                canvas.disc(cx, cy, rr, Rgba(ink.0, ink.1, ink.2, 140));
                            }
                            canvas.disc(cx, cy, rr - 1.0, straight(p.rgb()));
                        }
                    }
                    Row::Effect => {
                        let first = chip_window(sheet.effect, sheet.effects.len(), r.cells.len());
                        for (k, c) in r.cells.iter().enumerate() {
                            let on = sheet.effect == Some(first + k);
                            canvas.rounded(*c, c.h / 2.0, if on { accent } else { track });
                        }
                    }
                }
                if matches!(r.row, Row::Hue | Row::Saturation | Row::Brightness | Row::Temperature) {
                    let share = match r.row {
                        Row::Hue => sheet.hue / 360.0,
                        Row::Saturation => sheet.sat,
                        Row::Brightness => sheet.level,
                        _ => (sheet.kelvin - sheet.kelvin_range.0) / (sheet.kelvin_range.1 - sheet.kelvin_range.0).max(1.0),
                    }
                    .clamp(0.0, 1.0);
                    let (tx, ty) = (tr.x + share * tr.w, tr.y + tr.h / 2.0);
                    let big = if look.at == i { 13.0 } else { 10.0 };
                    canvas.disc(tx, ty, big + 2.0, Rgba(0, 0, 0, 70));
                    canvas.disc(tx, ty, big, straight([255, 255, 255]));
                }
                dirty.push(rect(band));
            }
        }
        for d in dirty {
            self.art.damage(Some(Rect { w: d.w.min(w as f32 - d.x), h: d.h.min(h as f32 - d.y), ..d }));
        }
        // the words: the values and the chips
        let mut said: Vec<(usize, String, Rgba)> = Vec::new();
        for (i, r) in lay.rows.iter().enumerate() {
            let front = look.at == i;
            said.push((2 + 2 * i, label_of(r.row).to_owned(), if front { ink } else { ink2 }));
            said.push((3 + 2 * i, value_of(sheet, r.row), if front { ink } else { ink2 }));
        }
        let base = 2 + 2 * lay.rows.len();
        let on = kit.ui.look.accent_ink();
        let cells = lay.rows.iter().find(|r| r.row == Row::Effect).map_or(0, |r| r.cells.len());
        let first_chip = chip_window(sheet.effect, sheet.effects.len(), cells);
        self.window = first_chip;
        for k in 0..cells {
            let name = sheet.effects.get(first_chip + k).cloned().unwrap_or_default();
            let width = lay.rows.iter().find(|r| r.row == Row::Effect).map_or(0.0, |r| r.cells[k].w - 12.0);
            let caption = kit.ty(TypeRole::Caption);
            let name = ui::components::text::ellipsise(&mut kit.ui.fonts, &name, caption, width);
            said.push((base + k, name, if sheet.effect == Some(first_chip + k) { on } else { ink }));
        }
        let refs: Vec<(usize, &str, Rgba)> = said.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.words.set_many(kit, &refs);
        Ok(())
    }

    /// The lint's view: the pane, and every line of words on it.
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, paint: Rgba) {
        let panel = lint.panel("colour sheet", self.layout.pane, paint, true);
        // a chip's words are read on the chip: the accent for the effect in front, a wash of the ink for the rest
        let (ink, accent) = (kit.ui.look.ink(), kit.ui.look.accent());
        let front = self.drawn.as_ref().and_then(|l| l.effect);
        let base = 2 + 2 * self.layout.rows.len();
        let pane = self.layout.pane;
        let cells = self.layout.rows.iter().find(|r| r.row == Row::Effect).map_or(&[][..], |r| &r.cells[..]);
        let chips: Vec<usize> = cells
            .iter()
            .enumerate()
            .map(|(k, c)| lint.panel_on("colour sheet chip", Frame::new(pane.x + c.x, pane.y + c.y, c.w, c.h), if front == Some(self.window + k) { accent } else { faded(ink, CHIP_REST) }, false, panel))
            .collect();
        self.words.lint_by(kit, lint, "colour sheet", (0.0, 0.0), |i| Some(i.checked_sub(base).and_then(|k| chips.get(k).copied()).unwrap_or(panel)));
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

    const ALL: [Row; 6] = [Row::Hue, Row::Saturation, Row::Brightness, Row::Temperature, Row::Preset, Row::Effect];

    /// At either text size, for every set of rows a lamp has: the sheet is on the screen, nothing in a row overlaps another part of it, the
    /// rows do not overlap each other and every track has room to be pointed at.
    #[test]
    fn nothing_on_the_colour_sheet_overlaps_or_leaves_it_at_either_text_size() {
        for scale in [1.0f32, 1.5, 2.0] {
            let m = Metrics::nominal(scale);
            for rows in [&ALL[..], &[Row::Brightness], &[Row::Hue, Row::Saturation, Row::Brightness, Row::Preset], &[Row::Brightness, Row::Temperature, Row::Preset], &[Row::Brightness, Row::Effect]] {
                for chips in [0usize, 2, 9] {
                    let l = layout(&m, rows, chips);
                    let tag = format!("scale {scale} rows {rows:?} chips {chips}");
                    let screen = Frame::new(0.0, 0.0, 1920.0, 1080.0);
                    assert!(l.pane.x >= 0.0 && l.pane.y >= 54.0 - 0.01 && l.pane.right() <= screen.right() && l.pane.bottom() <= 1080.0 - 54.0 + 0.01, "{tag}: {:?}", l.pane);
                    let inside = |f: Frame| f.x >= -0.01 && f.y >= -0.01 && f.right() <= l.pane.w + 0.01 && f.bottom() <= l.pane.h + 0.01;
                    for r in &l.rows {
                        assert!(inside(r.band) && inside(r.label) && inside(r.value) && inside(r.track), "{tag}: {r:?}");
                        assert!(!overlaps(r.label, r.value) && !overlaps(r.label, r.track) && !overlaps(r.value, r.track), "{tag}: {r:?}");
                        assert!(r.track.w >= 40.0, "{tag}: a track to point at {r:?}");
                        for c in &r.cells {
                            assert!(inside(*c) && c.w >= 16.0 && c.h >= 16.0, "{tag}: {c:?}");
                        }
                        for (a, b) in r.cells.iter().zip(r.cells.iter().skip(1)) {
                            assert!(!overlaps(*a, *b), "{tag}: {a:?} {b:?}");
                        }
                    }
                    for (a, b) in l.rows.iter().zip(l.rows.iter().skip(1)) {
                        assert!(!overlaps(a.band, b.band), "{tag}");
                    }
                    // the lamp on the left clear of the rows
                    let (cx, cy, r) = l.disc;
                    let lamp = l.ring.map_or(Frame::new(cx - r, cy - r, 2.0 * r, 2.0 * r), |g| Frame::new(g.cx - g.outer, g.cy - g.outer, 2.0 * g.outer, 2.0 * g.outer));
                    assert!(inside(lamp), "{tag}: {lamp:?}");
                    assert!(l.rows.iter().all(|r| !overlaps(lamp, r.band)), "{tag}");
                    assert!(!overlaps(l.name, lamp) || l.rows.is_empty(), "{tag}: {:?} {lamp:?}", l.name);
                }
            }
        }
    }

    #[test]
    fn the_pointer_finds_the_ring_a_track_a_swatch_and_a_chip() {
        let m = Metrics::nominal(1.0);
        let l = layout(&m, &ALL, 5);
        let at = |f: Frame, fx: f32, fy: f32| (l.pane.x + f.x + f.w * fx, l.pane.y + f.y + f.h * fy);
        let g = l.ring.unwrap();
        // the ring: straight up is hue 0, to the right 90
        assert_eq!(hit(&l, 0, l.pane.x + g.cx, l.pane.y + g.cy - (g.outer + g.inner) / 2.0), Some(Hit::Ring(0.0)));
        let Some(Hit::Ring(h)) = hit(&l, 0, l.pane.x + g.cx + (g.outer + g.inner) / 2.0, l.pane.y + g.cy) else { panic!("the ring") };
        assert!((h - 90.0).abs() < 0.01, "{h}");
        // the middle of the disc is not on the ring
        assert_eq!(hit(&l, 0, l.pane.x + g.cx, l.pane.y + g.cy), None);
        let sat = l.rows.iter().find(|r| r.row == Row::Saturation).unwrap();
        let (x, y) = at(sat.track, 0.25, 0.5);
        assert_eq!(hit(&l, 0, x, y), Some(Hit::Track(Row::Saturation, 0.25)));
        let (x, y) = at(sat.track, 1.0, 0.5);
        assert_eq!(hit(&l, 0, x, y), Some(Hit::Track(Row::Saturation, 1.0)));
        let preset = l.rows.iter().find(|r| r.row == Row::Preset).unwrap();
        let (x, y) = at(preset.cells[3], 0.5, 0.5);
        assert_eq!(hit(&l, 0, x, y), Some(Hit::Swatch(3)));
        let effect = l.rows.iter().find(|r| r.row == Row::Effect).unwrap();
        let (x, y) = at(effect.cells[1], 0.5, 0.5);
        assert_eq!(hit(&l, 2, x, y), Some(Hit::Chip(3)), "the chips begin at the third effect");
        // held, a thumb follows the pointer past the end of its track
        assert_eq!(along(&l, Row::Saturation, l.pane.x + sat.track.x - 90.0), 0.0);
        assert_eq!(along(&l, Row::Saturation, l.pane.x + sat.track.right() + 400.0), 1.0);
        let h = round(&l, l.pane.x + g.cx - 300.0, l.pane.y + g.cy).unwrap();
        assert!((h - 270.0).abs() < 0.01, "{h}");
    }

    #[test]
    fn the_chips_begin_so_that_the_effect_in_front_is_shown_and_never_beyond_the_ends() {
        assert_eq!(chip_window(None, 9, 3), 0);
        assert_eq!(chip_window(Some(0), 9, 3), 0);
        assert_eq!(chip_window(Some(4), 9, 3), 3);
        assert_eq!(chip_window(Some(8), 9, 3), 6);
        assert_eq!(chip_window(Some(1), 2, 3), 0);
        assert_eq!(chip_window(Some(1), 1, 3), 0);
        assert_eq!(chip_window(Some(4), 9, 2), 3, "the one in front is the second of two");
        assert_eq!(chip_window(Some(8), 9, 1), 8);
    }

    #[test]
    fn a_hue_ring_is_pure_colour_with_soft_edges_and_clear_inside_and_out() {
        let px = ring_pixels(100, 50.0, 38.0);
        let at = |x: u32, y: u32| &px[((y * 100 + x) * 4) as usize..((y * 100 + x) * 4 + 4) as usize];
        // straight up is red (to a pixel's width of hue), a quarter round is the hue 90 (a yellowish green)
        let top = at(50, 5);
        assert!(top[0] == 255 && top[1] < 8 && top[2] == 0 && top[3] == 255, "{top:?}");
        let right = at(94, 50);
        assert!(right[1] > 200 && right[0] > 100 && right[0] < 160 && right[2] < 10 && right[3] == 255, "{right:?}");
        assert_eq!(at(50, 50)[3], 0, "the middle is clear");
        assert_eq!(at(0, 0)[3], 0, "so is outside");
    }

    #[test]
    fn a_colour_worth_having_has_a_name() {
        assert_eq!(preset_name(Preset::White(2700)), "Warm white");
        assert_eq!(preset_name(Preset::White(4000)), "Neutral white");
        assert_eq!(preset_name(Preset::White(6500)), "Cool white");
        assert_eq!(preset_name(Preset::Colour([10, 132, 255])), "Blue");
    }
}
