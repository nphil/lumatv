//! On Home: Home Assistant's shared things as tiles on a card, and the sheet that opens over Home for one of them.
//!
//! The helper's `onhome` topic says everything in words (a name, a line of status, a glyph, a tone, what OK does, the numbers a sheet moves;
//! `helper/app/src/onhome/tile.rs`): this module draws it and turns keys into the helper's `POST /v1/onhome/act` bodies. It knows nothing of
//! Home Assistant, its attributes or its services, and holds no address or token: the TV is one more LumaShow device to Home Assistant's
//! integration, which pushes what the owner shared and decides what the TV may do.
//!
//! **The card.** A heading, what the link says at its right, and the tiles two to a row on a card wide enough for it (one on a narrow one),
//! each a glass plate with a tinted disc and a glyph, the name, and a line of status. With no tile the card says why in one chip (not
//! switched on, not paired, waiting for Home Assistant, nothing shared yet) and OK opens Settings at the page. With tiles OK on the card
//! enters it: the D-pad moves over the tiles, OK does the tile's one action (switch it, run it; a thermostat, a cover or a thing that is
//! only shown opens its sheet), a held OK or the Menu key opens the sheet, Back leaves.
//!
//! **The sheet** ([`Sheet`], drawn by [`SheetSurface`]): the controls of one tile as rows (a switch, a slider, a mode, an action) with
//! Left and Right changing the value of the row in front. A slider is sent once the key rests (a held key is one call, not forty), and
//! what the person set is shown until Home Assistant's own answer is. A refusal comes back in words and is said for a few seconds.

use super::surface::{HIDDEN, faded, glass_of};
use super::widgets::{Canvas, Pictures, Words};
use crate::kit::Kit;
use crate::kit::pane::PaneKind;
use crate::kit::picture::{Fit, Picture};
use crate::kit::text::role_style;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use std::collections::HashMap;
use ui::components::text::ellipsise;
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const TOPIC: &str = "onhome";
/// The card's own padding is the surface's (`PAD_SIDE`, `PAD_TOP`); a card this wide or wider has two tiles to a row.
const TWO_COLUMNS_FROM: f32 = 640.0;
const GAP: f32 = 12.0;
const MAX_VISIBLE: usize = 24;

/// Tiles to a row on a card whose content box is `w` wide.
pub fn cols(w: f32) -> usize {
    if w >= TWO_COLUMNS_FROM { 2 } else { 1 }
}

/// The tiles of the topic, as they are listed.
pub fn tiles(topic: &Value) -> Vec<Value> {
    topic.get("tiles").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The title of a kind of tile, as the helper's `group` names it.
pub fn group_title(group: &str) -> &'static str {
    match group {
        "lights" => "Lights",
        "switches" => "Switches & fans",
        "scenes" => "Scenes & scripts",
        "climate" => "Climate",
        "covers" => "Covers",
        "sensors" => "Sensors",
        "locks" => "Locks",
        "media" => "Media players",
        _ => "Other",
    }
}

/// One word for where the link stands, for the card's heading when the person is not doing anything with a tile.
pub fn heading_words(topic: &Value) -> String {
    match topic.get("state").and_then(Value::as_str) {
        Some("connected") => topic.get("words").and_then(Value::as_str).unwrap_or("").to_owned(),
        Some("waiting") => "Waiting for Home Assistant".into(),
        _ => String::new(),
    }
}

/// What the card says with no tile: a title, a line under it and the tone of its glyph (the helper's own words).
pub fn note_of(topic: &Value) -> (String, String, &'static str) {
    let text = |k: &str| topic.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match topic.get("state").and_then(Value::as_str) {
        None => ("Starting".into(), "Waiting for the helper".into(), "info"),
        Some("off") => ("Not switched on".into(), "Settings, Home & look, Home Assistant, On Home".into(), "info"),
        Some("unpaired") => ("Not paired".into(), "Pair the TV with Home Assistant first".into(), "info"),
        Some("problem") => ("Can't listen".into(), text("detail"), "warning"),
        Some("waiting") => ("Waiting for Home Assistant".into(), "The LumaShow integration has not connected".into(), "warning"),
        Some("empty") if topic.get("shared").and_then(Value::as_u64).unwrap_or(0) == 0 => ("Nothing shared yet".into(), "Share some with the TV in Home Assistant".into(), "info"),
        Some("empty") => ("Nothing on Home".into(), "Choose what to show in Settings".into(), "info"),
        Some(_) => (text("words"), text("detail"), "info"),
    }
}

fn tone_colour(kit: &Kit, tone: &str) -> Rgba {
    use ui::lucent::palette::PaletteRole as R;
    let look = &kit.ui.look;
    match tone {
        "on" => look.accent(),
        "warn" => look.colour(R::Warning),
        "alert" => look.colour(R::Danger),
        "ok" => look.colour(R::Positive),
        "info" | "warning" => look.colour(R::Info),
        _ => look.ink2(),
    }
}

fn glyph(kit: &mut Kit, pictures: &mut Pictures, name: &str, side: u32, colour: Rgba) -> Option<Vec<u8>> {
    kit.icons.draw(name, side, colour).or_else(|| pictures.get(&format!("base/{name}")).map(|p| p.fitted(side, side, Fit::Contain).rgba))
}

// ---- the card

pub struct OnHome {
    pub block: Words,
    art: ImageLayer,
    w: f32,
    cols: usize,
    tile_w: f32,
    tile_h: f32,
    row_top: f32,
    rows_in_view: usize,
    show_title: bool,
    /// The disc behind a glyph, the glyph, where the words start in a tile and how wide they may be.
    disc: f32,
    icon_px: u32,
    text_x: f32,
    text_w: f32,
    label: Style,
    cap: Style,
    first_row: usize,
    /// Per slot in view: the lines of its name and of its status.
    slots: Vec<(usize, usize)>,
    heading: (usize, usize),
    note: (usize, usize),
    at: Props,
    shown: Option<String>,
}

impl OnHome {
    pub fn new(kit: &mut Kit, w: f32, h: f32, z: i32, entry: &Value) -> std::io::Result<Self> {
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let cap = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let cap_r = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
        let lh = kit.line(TypeRole::Label);
        let ch = kit.line(TypeRole::Caption);
        let cols = cols(w);
        let tile_w = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        let tile_h = (lh + ch + 16.0).max(64.0 * kit.text_scale).ceil();
        // a card too low for a heading and a row of tiles gives the room to the tiles (as the QML's lane does)
        let show_title = h >= lh + 14.0 + tile_h;
        let row_top = if show_title { lh + 14.0 } else { 0.0 };
        let rows_in_view = (((h - row_top + GAP) / (tile_h + GAP)).floor().max(0.0) as usize).min(MAX_VISIBLE / cols);
        // the disc and its glyph grow with the tile, which grows with the text size
        let disc = (tile_h * 0.62).round().min(64.0);
        let icon_px = ((disc * 0.6).round() as u32).max(16);
        let text_x = 14.0 + disc + 12.0;
        let text_w = (tile_w - text_x - 12.0).max(10.0);
        let title = entry.get("title").and_then(Value::as_str).unwrap_or("Smart home").to_owned();
        let mut lines: Vec<(Frame, ui::components::Style, String)> = vec![(Frame::new(0.0, 0.0, w * 0.45, lh), label, title), (Frame::new(w * 0.45, 0.0, w * 0.55, lh), cap_r, String::new())];
        let mut slots = Vec::new();
        for s in 0..rows_in_view * cols {
            let (c, r) = (s % cols, s / cols);
            let (x, y) = (c as f32 * (tile_w + GAP), row_top + r as f32 * (tile_h + GAP));
            let top = y + ((tile_h - lh - ch) / 2.0).floor();
            let a = lines.len();
            lines.push((Frame::new(x + text_x, top, text_w, lh), label, String::new()));
            let b = lines.len();
            lines.push((Frame::new(x + text_x, top + lh, text_w, ch), cap, String::new()));
            slots.push((a, b));
        }
        let note_a = lines.len();
        lines.push((Frame::new(text_x, row_top + ((tile_h - lh - ch) / 2.0).floor(), w - text_x - 40.0, lh), label, String::new()));
        let note_b = lines.len();
        lines.push((Frame::new(text_x, row_top + ((tile_h - lh - ch) / 2.0).floor() + lh, w - text_x - 40.0, ch), cap, String::new()));
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        let mut art = kit.ui.scene.image(w.max(1.0).round() as u32, h.max(1.0).round() as u32, z + 1, HIDDEN)?;
        art.clear();
        art.damage(None);
        Ok(Self { block, art, w, cols, tile_w, tile_h, row_top, rows_in_view, show_title, disc, icon_px, text_x, text_w, label, cap, first_row: 0, slots, heading: (0, 1), note: (note_a, note_b), at: HIDDEN, shown: None })
    }

    pub fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        self.art.set(at);
    }

    /// How many tiles are in view.
    pub fn visible(&self) -> usize {
        self.slots.len()
    }

    /// The tile `index` of the list is drawn where, when it is in view.
    fn slot_of(&self, index: usize) -> Option<usize> {
        let (row, col) = (index / self.cols, index % self.cols);
        (row >= self.first_row && row < self.first_row + self.rows_in_view).then(|| (row - self.first_row) * self.cols + col)
    }

    /// The tiles of the topic (`inner` the one the inner focus is on), with the notice the last control left, if one is being said.
    pub fn set(&mut self, kit: &mut Kit, topic: &Value, inner: Option<usize>, notice: Option<&str>, pictures: &mut Pictures) {
        let list = tiles(topic);
        if let Some(i) = inner.filter(|i| *i < list.len() && self.rows_in_view > 0) {
            let row = i / self.cols;
            if row < self.first_row {
                self.first_row = row;
            } else if row >= self.first_row + self.rows_in_view {
                self.first_row = row + 1 - self.rows_in_view;
            }
        }
        let rows = list.len().div_ceil(self.cols);
        self.first_row = self.first_row.min(rows.saturating_sub(self.rows_in_view));
        let key = format!("{topic}|{inner:?}|{notice:?}|{}", self.first_row);
        if self.shown.as_deref() == Some(key.as_str()) {
            return;
        }
        self.shown = Some(key);
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let mut changes: Vec<(usize, String, Rgba)> = Vec::new();
        let heading = match (notice, inner.is_some() && !list.is_empty()) {
            (Some(n), _) => n.to_owned(),
            (None, true) => "Hold OK for controls".to_owned(),
            _ => heading_words(topic),
        };
        if self.show_title {
            changes.push((self.heading.1, heading, ink2));
        } else {
            changes.push((self.heading.0, String::new(), ink));
            changes.push((self.heading.1, String::new(), ink2));
        }
        let radius = kit.tokens.radius(Radius::Control);
        let (w, h) = (self.art.width(), self.art.height());
        let mut drawn: Vec<(f32, f32, bool, Rgba, Option<Vec<u8>>, bool)> = Vec::new();
        let mut note_icon: Option<(Rgba, Option<Vec<u8>>)> = None;
        if list.is_empty() {
            let (title, sub, tone) = note_of(topic);
            let colour = tone_colour(kit, tone);
            let wide = self.w - self.text_x - 40.0;
            changes.push((self.note.0, ellipsise(&mut kit.ui.fonts, &title, self.label.ty, wide), ink));
            changes.push((self.note.1, ellipsise(&mut kit.ui.fonts, &sub, self.cap.ty, wide), ink2));
            note_icon = Some((colour, glyph(kit, pictures, "house-wifi", self.icon_px, colour)));
            for (a, b) in &self.slots {
                changes.push((*a, String::new(), ink));
                changes.push((*b, String::new(), ink2));
            }
        } else {
            changes.push((self.note.0, String::new(), ink));
            changes.push((self.note.1, String::new(), ink2));
            for (s, (a, b)) in self.slots.iter().enumerate() {
                let index = self.first_row * self.cols + s;
                let Some(t) = list.get(index) else {
                    changes.push((*a, String::new(), ink));
                    changes.push((*b, String::new(), ink2));
                    continue;
                };
                let text = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("");
                let quiet = matches!(text("tone"), "off" | "muted");
                changes.push((*a, ellipsise(&mut kit.ui.fonts, text("name"), self.label.ty, self.text_w), ink));
                changes.push((*b, ellipsise(&mut kit.ui.fonts, text("words"), self.cap.ty, self.text_w), if quiet { ink2 } else { ink }));
                let colour = tone_colour(kit, text("tone"));
                let (c, r) = (s % self.cols, s / self.cols);
                let px = glyph(kit, pictures, text("icon"), self.icon_px, colour);
                drawn.push((c as f32 * (self.tile_w + GAP), self.row_top + r as f32 * (self.tile_h + GAP), inner == Some(index), colour, px, t.get("available").and_then(Value::as_bool) == Some(false)));
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.block.set_many(kit, &refs);
        let chevron = if list.is_empty() { kit.icons.draw("chevron-right", 22, ink2) } else { None };
        let plate_colour = faded(ink, 0.10);
        let lit = faded(ink, 0.26);
        self.art.clear();
        {
            let mut canvas = Canvas { px: self.art.pixels_mut(), w, h };
            let (tw, th) = (self.tile_w, self.tile_h);
            for (x, y, focused, colour, px, gone) in &drawn {
                canvas.rounded(Frame::new(*x, *y, tw, th), radius, if *focused { lit } else { plate_colour });
                canvas.disc(x + 14.0 + self.disc / 2.0, y + th / 2.0, self.disc / 2.0, faded(*colour, if *gone { 0.12 } else { 0.22 }));
                if let Some(px) = px {
                    let side = self.icon_px;
                    let pic = Picture { width: side, height: side, rgba: px.clone() };
                    canvas.picture(&pic, (x + 14.0 + self.disc / 2.0 - side as f32 / 2.0).round() as i32, (y + th / 2.0 - side as f32 / 2.0).round() as i32, side, side);
                }
            }
            if let Some((colour, px)) = &note_icon {
                let y = self.row_top;
                canvas.rounded(Frame::new(0.0, y, self.w, self.tile_h), radius, plate_colour);
                canvas.disc(14.0 + self.disc / 2.0, y + self.tile_h / 2.0, self.disc / 2.0, faded(*colour, 0.22));
                if let Some(px) = px {
                    let side = self.icon_px;
                    let pic = Picture { width: side, height: side, rgba: px.clone() };
                    canvas.picture(&pic, (14.0 + self.disc / 2.0 - side as f32 / 2.0).round() as i32, (y + self.tile_h / 2.0 - side as f32 / 2.0).round() as i32, side, side);
                }
                if let Some(c) = &chevron {
                    let pic = Picture { width: 22, height: 22, rgba: c.clone() };
                    canvas.picture(&pic, self.w as i32 - 22 - 14, (y + (self.tile_h - 22.0) / 2.0).round() as i32, 22, 22);
                }
            }
        }
        self.art.damage(None);
        let _ = self.slot_of(0);
    }
}

// ---- the sheet

/// What a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A line of words, never focused.
    Info,
    Toggle,
    Slider { min: f64, max: f64, step: f64, unit: String },
    /// One of a list, Left and Right walk it.
    Cycle(Vec<String>),
    Action,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: &'static str,
    pub label: String,
    pub kind: Kind,
    /// The value as the row says it.
    pub value: String,
    /// The number a slider is at, `None` when it has none yet.
    pub num: Option<f64>,
    /// An answer that cannot be taken back (unlocking): drawn in the danger colour.
    pub danger: bool,
}

/// A control to send: the helper's `POST /v1/onhome/act` body.
#[derive(Clone, Debug, PartialEq)]
pub struct Act {
    pub id: String,
    pub verb: &'static str,
    pub value: Value,
}

fn has(verbs: &[&str], v: &str) -> bool {
    verbs.contains(&v)
}

fn num_text(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r:.1}") }
}

fn mode_word(mode: &str) -> String {
    match mode {
        "heat_cool" => "Heat/Cool".into(),
        "fan_only" => "Fan".into(),
        other => {
            let t = other.replace('_', " ");
            let mut c = t.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

/// The rows of a tile's sheet, from the tile the helper made.
pub fn rows_of(tile: &Value) -> Vec<Row> {
    let id = tile.get("id").and_then(Value::as_str).unwrap_or("");
    let group = tile.get("group").and_then(Value::as_str).unwrap_or("");
    let ctl = tile.get("ctl").cloned().unwrap_or(Value::Null);
    let verbs: Vec<&str> = tile.get("verbs").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let can: Vec<&str> = tile.get("can").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let available = tile.get("available").and_then(Value::as_bool) != Some(false);
    let f = |k: &str| ctl.get(k).and_then(Value::as_f64);
    let on = ctl.get("on").and_then(Value::as_bool).unwrap_or(false);
    let mut rows: Vec<Row> = Vec::new();
    let row = |id: &'static str, label: &str, kind: Kind, value: String, num: Option<f64>| Row { id, label: label.to_owned(), kind, value, num, danger: false };
    let domain = id.split('.').next().unwrap_or("");
    let toggle = can.contains(&"toggle");
    if available {
        match group {
            "lights" => {
                if toggle {
                    rows.push(row("power", "Power", Kind::Toggle, if on { "On" } else { "Off" }.into(), None));
                }
                if has(&verbs, "turn_on") && ctl.get("dimmable").and_then(Value::as_bool) == Some(true) {
                    let b = f("brightness");
                    rows.push(row("brightness", "Brightness", Kind::Slider { min: 0.0, max: 100.0, step: 10.0, unit: "%".into() }, b.map_or("\u{2014}".into(), |b| format!("{}%", num_text(b))), b));
                }
                if has(&verbs, "turn_on")
                    && let Some(k) = ctl.get("kelvin").filter(|k| !k.is_null())
                {
                    let (lo, hi, now) = (k["min"].as_f64().unwrap_or(2000.0), k["max"].as_f64().unwrap_or(6500.0), k["now"].as_f64());
                    rows.push(row("kelvin", "Colour temperature", Kind::Slider { min: lo, max: hi, step: 250.0, unit: " K".into() }, now.map_or("\u{2014}".into(), |k| format!("{} K", num_text(k))), now));
                }
            }
            "switches" => {
                if toggle {
                    rows.push(row("power", "Power", Kind::Toggle, if on { "On" } else { "Off" }.into(), None));
                }
                if domain == "fan" && has(&verbs, "set_percentage") {
                    let p = f("percentage");
                    let step = f("step").filter(|s| *s >= 1.0).unwrap_or(10.0);
                    rows.push(row("percentage", "Speed", Kind::Slider { min: 0.0, max: 100.0, step, unit: "%".into() }, p.map_or("\u{2014}".into(), |p| format!("{}%", num_text(p))), p));
                }
            }
            "scenes" => {
                if can.contains(&"run") {
                    rows.push(row("run", if domain == "scene" { "Activate" } else { "Run" }, Kind::Action, String::new(), None));
                }
            }
            "climate" => {
                if let Some(c) = f("current") {
                    let action = ctl.get("action").and_then(Value::as_str).unwrap_or("");
                    let unit = ctl.get("unit").and_then(Value::as_str).unwrap_or("\u{b0}");
                    let said = match action {
                        "heating" => " \u{b7} Heating",
                        "cooling" => " \u{b7} Cooling",
                        _ => "",
                    };
                    rows.push(row("now", "In the room", Kind::Info, format!("{}{}{said}", num_text(c), unit.chars().next().filter(|c| *c == '\u{b0}').map_or(String::new(), |c| c.to_string())), None));
                }
                let modes: Vec<String> = ctl.get("modes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                if has(&verbs, "set_hvac_mode") && !modes.is_empty() {
                    let now = ctl.get("mode").and_then(Value::as_str).unwrap_or("");
                    rows.push(row("mode", "Mode", Kind::Cycle(modes), mode_word(now), None));
                }
                if has(&verbs, "set_temperature") {
                    let (min, max, step) = (f("min").unwrap_or(50.0), f("max").unwrap_or(90.0), f("step").unwrap_or(1.0));
                    let t = |v: Option<f64>| v.map_or("\u{2014}".into(), |v| format!("{}\u{b0}", num_text(v)));
                    let off = ctl.get("mode").and_then(Value::as_str) == Some("off");
                    if let Some(target) = f("target") {
                        if !off {
                            rows.push(row("target", "Set to", Kind::Slider { min, max, step, unit: "\u{b0}".into() }, t(Some(target)), Some(target)));
                        }
                    } else if let (Some(low), Some(high)) = (f("low"), f("high"))
                        && !off
                    {
                        rows.push(row("low", "Heat to", Kind::Slider { min, max, step, unit: "\u{b0}".into() }, t(Some(low)), Some(low)));
                        rows.push(row("high", "Cool to", Kind::Slider { min, max, step, unit: "\u{b0}".into() }, t(Some(high)), Some(high)));
                    }
                }
            }
            "covers" => {
                if has(&verbs, "open_cover") {
                    rows.push(row("open", "Open", Kind::Action, String::new(), None));
                }
                if has(&verbs, "close_cover") {
                    rows.push(row("close", "Close", Kind::Action, String::new(), None));
                }
                if has(&verbs, "stop_cover") {
                    rows.push(row("stop", "Stop", Kind::Action, String::new(), None));
                }
                if has(&verbs, "set_cover_position")
                    && let Some(p) = f("position")
                {
                    rows.push(row("position", "Position", Kind::Slider { min: 0.0, max: 100.0, step: 10.0, unit: "%".into() }, format!("{}%", num_text(p)), Some(p)));
                }
            }
            "locks" => {
                let state = ctl.get("state").and_then(Value::as_str).unwrap_or("");
                let sensitive: Vec<&str> = tile.get("sensitive").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                if has(&verbs, "lock") && state != "locked" && state != "locking" {
                    rows.push(row("lock", "Lock", Kind::Action, String::new(), None));
                }
                // Unlocking asks first, and is the person's alone: a row only for a lock Home Assistant offers it for
                if state == "locked" && sensitive.contains(&"unlock") {
                    let mut r = row("unlock", "Unlock", Kind::Action, String::new(), None);
                    r.danger = true;
                    rows.push(r);
                } else if state == "locked" && has(&verbs, "lock") {
                    rows.push(row("unlockinfo", "Unlock", Kind::Info, "Not offered by Home Assistant".into(), None));
                }
            }
            "media" => {
                for (k, label) in [("artist", "Artist"), ("app", "App"), ("source", "Source")] {
                    if let Some(v) = ctl.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()) {
                        rows.push(row("info", label, Kind::Info, v.to_owned(), None));
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(area) = tile.get("area").and_then(Value::as_str).filter(|a| !a.is_empty()) {
        rows.push(row("room", "Room", Kind::Info, area.to_owned(), None));
    }
    rows
}

/// What a Left or Right on a slider comes to: one step on the row's grid, inside its range.
pub fn stepped(row: &Row, dir: i32) -> Option<f64> {
    let Kind::Slider { min, max, step, .. } = &row.kind else { return None };
    let now = row.num.unwrap_or(*min);
    // from nothing (a light that is off), the first press lands on the step above the lowest
    let v = ((now + f64::from(dir) * step) / step).round() * step;
    Some(((v.clamp(*min, *max)) * 100.0).round() / 100.0)
}

fn value_text(row: &Row, v: f64) -> String {
    match &row.kind {
        Kind::Slider { unit, .. } => format!("{}{unit}", num_text(v)),
        _ => num_text(v),
    }
}

/// How long a slider rests before it is sent, and how long what the person set is shown in place of what Home Assistant says.
const REST_MS: u64 = 300;
const HOLD_SHOWN_MS: u64 = 2500;
const NOTICE_MS: u64 = 4000;

/// The sheet of one tile.
#[derive(Clone, Debug)]
pub struct Sheet {
    pub id: String,
    /// The row in front (an index into the focusable rows).
    pub index: usize,
    pending: Option<(Act, u64)>,
    /// What the person set, by row id: the text, the number, until when.
    shown: HashMap<&'static str, (String, f64, u64)>,
    pub notice: Option<(String, u64)>,
    /// The question "Unlock <name>?" is up: its two answers are the rows, Cancel in front.
    pub asking: bool,
}

/// What a key did.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Nothing,
    Close,
    /// Send this now.
    Send(Act),
    /// The person pressed Unlock in the question, with the remote: unlock this lock. Only [`Sheet::key`] makes this, from the key `ok` on that
    /// answer, and the model sends it to the route only the shell may call; it is never an [`Act`].
    Unlock(String),
}

impl Sheet {
    pub fn new(id: &str) -> Self {
        Self { id: id.to_owned(), index: 0, pending: None, shown: HashMap::new(), notice: None, asking: false }
    }

    /// A sheet that opens on the question "Unlock <name>?" (OK on a locked lock's tile).
    pub fn asking(id: &str) -> Self {
        Self { asking: true, ..Self::new(id) }
    }

    /// The question, while it is up.
    pub fn ask(&self, tile: &Value) -> Option<String> {
        self.asking.then(|| format!("Unlock {}?", tile.get("name").and_then(Value::as_str).unwrap_or("this lock")))
    }

    fn focusable(rows: &[Row]) -> Vec<usize> {
        rows.iter().enumerate().filter(|(_, r)| r.kind != Kind::Info).map(|(i, _)| i).collect()
    }

    /// The rows with what the person set put over what the tile says, while it is shown.
    pub fn rows(&self, tile: &Value, now: u64) -> Vec<Row> {
        if self.asking {
            // the safe answer first and in front
            let mut yes = Row { id: "unlock_yes", label: "Unlock".into(), kind: Kind::Action, value: String::new(), num: None, danger: true };
            yes.danger = true;
            return vec![Row { id: "cancel", label: "Cancel".into(), kind: Kind::Action, value: String::new(), num: None, danger: false }, yes];
        }
        let mut rows = rows_of(tile);
        for r in &mut rows {
            if let Some((text, num, until)) = self.shown.get(r.id)
                && now < *until
            {
                r.value = text.clone();
                if matches!(r.kind, Kind::Slider { .. }) {
                    r.num = Some(*num);
                }
            }
        }
        rows
    }

    /// The index in `rows` of the row in front (`None` when nothing can be focused).
    pub fn front(&self, rows: &[Row]) -> Option<usize> {
        let f = Self::focusable(rows);
        f.get(self.index.min(f.len().saturating_sub(1))).copied()
    }

    /// A key. `tile` is the tile as the topic has it now.
    pub fn key(&mut self, name: &str, tile: &Value, now: u64) -> Done {
        let rows = self.rows(tile, now);
        let focusable = Self::focusable(&rows);
        match name {
            // Back takes the question away before it takes the sheet away
            "back" if self.asking => {
                self.asking = false;
                self.index = 0;
                return Done::Nothing;
            }
            "back" => return Done::Close,
            "up" | "down" => {
                let n = focusable.len();
                if n > 0 {
                    let i = self.index.min(n - 1);
                    self.index = if name == "up" { i.saturating_sub(1) } else { (i + 1).min(n - 1) };
                }
                return Done::Nothing;
            }
            _ => {}
        }
        let Some(at) = self.front(&rows) else {
            return if name == "ok" { Done::Close } else { Done::Nothing };
        };
        let row = &rows[at];
        let act = |verb: &'static str, value: Value| Act { id: self.id.clone(), verb, value };
        match (&row.kind, name) {
            (Kind::Toggle, "ok" | "left" | "right") => Done::Send(act("toggle", Value::Null)),
            (Kind::Action, "ok") if row.id == "cancel" => {
                self.asking = false;
                self.index = 0;
                Done::Nothing
            }
            (Kind::Action, "ok") if row.id == "unlock" => {
                self.asking = true;
                self.index = 0;
                Done::Nothing
            }
            (Kind::Action, "ok") if row.id == "unlock_yes" => {
                self.asking = false;
                Done::Unlock(self.id.clone())
            }
            (Kind::Action, "ok") => {
                let verb = match row.id {
                    "lock" => "lock",
                    "open" => "open",
                    "close" => "close",
                    "stop" => "stop",
                    _ => "run",
                };
                Done::Send(act(verb, Value::Null))
            }
            (Kind::Slider { .. }, "left" | "right") => {
                let Some(v) = stepped(row, if name == "left" { -1 } else { 1 }) else { return Done::Nothing };
                let (verb, value) = match row.id {
                    "brightness" => ("brightness", json!(v)),
                    "kelvin" => ("kelvin", json!(v)),
                    "percentage" => ("percentage", json!(v)),
                    "position" => ("position", json!(v)),
                    "target" => ("temperature", json!(v)),
                    "low" | "high" => {
                        let other = |id: &str| rows.iter().find(|r| r.id == id).and_then(|r| r.num).unwrap_or(v);
                        let (low, high) = if row.id == "low" { (v, other("high")) } else { (other("low"), v) };
                        ("range", json!({"low": low, "high": high}))
                    }
                    _ => return Done::Nothing,
                };
                self.shown.insert(row.id, (value_text(row, v), v, now + HOLD_SHOWN_MS));
                self.pending = Some((act(verb, value), now + REST_MS));
                Done::Nothing
            }
            (Kind::Cycle(options), "left" | "right" | "ok") => {
                let current = tile.pointer("/ctl/mode").and_then(Value::as_str).unwrap_or("");
                let i = options.iter().position(|o| o == current).unwrap_or(0) as i64;
                let step = if name == "left" { -1 } else { 1 };
                let next = &options[(i + step).rem_euclid(options.len() as i64) as usize];
                self.shown.insert(row.id, (mode_word(next), 0.0, now + HOLD_SHOWN_MS));
                Done::Send(act("hvac_mode", json!(next)))
            }
            _ => Done::Nothing,
        }
    }

    /// A slider that has rested long enough: the control to send.
    pub fn due(&mut self, now: u64) -> Option<Act> {
        match &self.pending {
            Some((_, at)) if now >= *at => self.pending.take().map(|(a, _)| a),
            _ => None,
        }
    }

    pub fn waiting(&self) -> Option<u64> {
        self.pending.as_ref().map(|(_, at)| *at)
    }

    pub fn say(&mut self, words: &str, now: u64) {
        self.notice = Some((words.to_owned(), now + NOTICE_MS));
    }

    pub fn tick(&mut self, now: u64) -> bool {
        let mut changed = false;
        if self.notice.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.notice = None;
            changed = true;
        }
        let before = self.shown.len();
        self.shown.retain(|_, (_, _, until)| now < *until);
        changed || self.shown.len() != before
    }
}

// ---- where the sheet is

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pane: Frame,
    pub head: Frame,
    pub sub: Frame,
    pub rows: Vec<Frame>,
}

pub const PAD: f32 = 24.0;
const ROW_GAP: f32 = 6.0;

/// The sheet's frames for `n` rows at a text size: a pane in the middle of the screen, the name and its line above the rows.
pub fn layout(n: usize, text_scale: f32) -> Layout {
    let s = text_scale.max(1.0);
    let line = (26.0 * 1.3 * s).ceil();
    let row_h = 48.0f32.max(line + 12.0);
    let head_h = line + 4.0;
    let sub_h = (22.0 * 1.3 * s).ceil();
    let width = (640.0 * s).round().min(1600.0);
    let body = n as f32 * (row_h + ROW_GAP) - if n > 0 { ROW_GAP } else { 0.0 };
    let height = (2.0 * PAD + head_h + sub_h + 12.0 + body).min(900.0);
    let (x, y) = (((1920.0 - width) / 2.0).round(), (((1080.0 - height) / 2.0).round()).max(40.0));
    let pane = Frame::new(x, y, width, height);
    let head = Frame::new(x + PAD, y + PAD, width - 2.0 * PAD, head_h);
    let sub = Frame::new(x + PAD, y + PAD + head_h, width - 2.0 * PAD, sub_h);
    let top = y + PAD + head_h + sub_h + 12.0;
    let rows = (0..n).map(|i| Frame::new(x + PAD, top + i as f32 * (row_h + ROW_GAP), width - 2.0 * PAD, row_h)).collect();
    Layout { pane, head, sub, rows }
}

// ---- drawing the sheet

pub struct SheetSurface {
    _pane: GlassLayer,
    _scrim: ShapeLayer,
    art: ImageLayer,
    words: Words,
    layout: Layout,
    /// The structure it was made for: the id and the rows' ids.
    structure: String,
    key: String,
    scale: f32,
}

fn structure_of(id: &str, rows: &[Row]) -> String {
    format!("{id}|{}", rows.iter().map(|r| r.id).collect::<Vec<_>>().join(","))
}

impl SheetSurface {
    pub fn new(kit: &mut Kit, id: &str, rows: &[Row], z: i32) -> std::io::Result<Self> {
        let lay = layout(rows.len(), kit.text_scale);
        let c = kit.tokens.extra(kit.mode).scrim_hide;
        let scrim = kit.ui.scene.shape(1920, 1080, z, plate::solid(c, 0.0), Props::at(0.0, 0.0));
        let pane = kit.ui.scene.glass(lay.pane.w as u32, lay.pane.h as u32, z + 1, glass_of(kit, PaneKind::Card, lay.pane.h, false, false), Props::at(lay.pane.x, lay.pane.y));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let head = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let sub = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink2, Align::Left);
        let value = role_style(kit, TypeRole::Label, None, ink, Align::Right);
        let line = kit.line(TypeRole::Label);
        let whole = lay.pane;
        let rel = |f: Frame| Frame::new(f.x - whole.x, f.y - whole.y, f.w, f.h);
        let mut lines = vec![(rel(lay.head), head, String::new()), (rel(lay.sub), sub, String::new())];
        for f in &lay.rows {
            let at = Frame::new(f.x - whole.x + 16.0, f.y - whole.y + ((f.h - line) / 2.0).round(), f.w - 32.0, line);
            lines.push((at, label, String::new()));
            lines.push((at, value, String::new()));
        }
        let words = Words::new(kit, Frame::new(whole.x, whole.y, whole.w, whole.h), z + 4, lines)?;
        let mut art = kit.ui.scene.image(whole.w as u32, whole.h as u32, z + 3, Props::at(whole.x, whole.y))?;
        art.clear();
        art.damage(None);
        let mut me = Self { _pane: pane, _scrim: scrim, art, words, layout: lay, structure: structure_of(id, rows), key: String::new(), scale: kit.text_scale.max(1.0) };
        me.words.set_props(Props::at(whole.x, whole.y));
        Ok(me)
    }

    pub fn is_for(&self, id: &str, rows: &[Row]) -> bool {
        self.structure == structure_of(id, rows)
    }

    pub fn update(&mut self, kit: &mut Kit, tile: &Value, rows: &[Row], front: Option<usize>, notice: Option<&str>) {
        let key = format!("{tile}|{rows:?}|{front:?}|{notice:?}");
        if key == self.key {
            return;
        }
        self.key = key;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let danger = kit.ui.look.colour(ui::lucent::palette::PaletteRole::Danger);
        let text = |k: &str| tile.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut changes: Vec<(usize, String, Rgba)> = vec![(0, text("name"), ink)];
        let sub = match notice {
            Some(n) => n.to_owned(),
            None => [text("words")].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" \u{b7} "),
        };
        changes.push((1, sub, if notice.is_some() { ink } else { ink2 }));
        for (i, r) in rows.iter().enumerate() {
            let on = front == Some(i);
            let value = match (&r.kind, on) {
                (Kind::Slider { .. } | Kind::Cycle(_), true) => format!("\u{2039} {} \u{203a}", r.value),
                (Kind::Action, _) => "\u{203a}".into(),
                _ => r.value.clone(),
            };
            changes.push((2 + 2 * i, r.label.clone(), if r.danger { danger } else if on { ink } else { ink2 }));
            changes.push((3 + 2 * i, value, ink));
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.words.set_many(kit, &refs);
        let radius = kit.tokens.radius(Radius::Row);
        let (w, h) = (self.art.width(), self.art.height());
        let wash = faded(ink, kit.tokens.states.focus_wash[0]);
        let track = faded(ink, 0.30);
        let fill = kit.ui.look.accent();
        self.art.clear();
        {
            let origin = self.layout.pane;
            let mut canvas = Canvas { px: self.art.pixels_mut(), w, h };
            for (i, r) in rows.iter().enumerate() {
                let f = self.layout.rows[i];
                let rel = Frame::new(f.x - origin.x, f.y - origin.y, f.w, f.h);
                if front == Some(i) {
                    canvas.rounded(rel, radius, wash);
                }
                if let (Kind::Slider { min, max, .. }, Some(n)) = (&r.kind, r.num) {
                    // the track under the value, at the middle of the row's right half
                    let (tw, th) = (rel.w * 0.28, (6.0 * self.scale).round());
                    let x = rel.x + rel.w * 0.40;
                    let y = rel.y + (rel.h - th) / 2.0;
                    let share = ((n - min) / (max - min).max(1e-9)).clamp(0.0, 1.0) as f32;
                    canvas.rounded(Frame::new(x, y, tw, th), th / 2.0, track);
                    if share > 0.0 {
                        canvas.rounded(Frame::new(x, y, (tw * share).max(th), th), th / 2.0, fill);
                    }
                }
            }
        }
        self.art.damage(None);
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, paint: Rgba) {
        let panel = lint.panel("onhome sheet", self.layout.pane, paint, true);
        self.words.lint(kit, lint, "onhome sheet", (0.0, 0.0), Some(panel));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(on: bool, brightness: Option<i64>) -> Value {
        json!({"id": "light.lamp", "name": "Lamp", "area": "Kitchen", "group": "lights", "words": "On", "can": ["toggle"], "available": true,
               "verbs": ["turn_on", "turn_off", "toggle"], "ctl": {"on": on, "dimmable": true, "brightness": brightness, "kelvin": {"now": 3000, "min": 2000, "max": 6500}}})
    }

    fn thermostat() -> Value {
        json!({"id": "climate.t", "name": "Thermostat", "group": "climate", "words": "68\u{b0}", "can": ["sheet"], "available": true,
               "verbs": ["set_temperature", "set_hvac_mode"], "ctl": {"mode": "heat", "modes": ["off", "heat", "cool"], "current": 68.0, "target": 70.0, "min": 50.0, "max": 90.0, "step": 1.0, "unit": "\u{b0}F", "action": "heating"}})
    }

    #[test]
    fn a_card_wide_enough_has_two_tiles_to_a_row_and_a_narrow_one_has_one() {
        assert_eq!((cols(1000.0), cols(640.0), cols(639.0), cols(300.0)), (2, 2, 1, 1));
    }

    #[test]
    fn a_light_has_power_brightness_and_colour_temperature_and_a_room_that_is_only_read() {
        let rows = rows_of(&light(true, Some(50)));
        assert_eq!(rows.iter().map(|r| (r.id, r.value.as_str())).collect::<Vec<_>>(), [("power", "On"), ("brightness", "50%"), ("kelvin", "3000 K"), ("room", "Kitchen")]);
        // off, it has no level to say
        let off = rows_of(&light(false, None));
        assert_eq!((off[0].value.as_str(), off[1].value.as_str()), ("Off", "\u{2014}"));
        // one that is only switched has only power
        let mut plain = light(true, Some(50));
        plain["ctl"]["dimmable"] = json!(false);
        plain["ctl"]["kelvin"] = Value::Null;
        assert_eq!(rows_of(&plain).iter().map(|r| r.id).collect::<Vec<_>>(), ["power", "room"]);
        // and one the integration shares for looking has none of the controls
        let mut shown = light(true, Some(50));
        shown["can"] = json!([]);
        shown["verbs"] = json!([]);
        assert_eq!(rows_of(&shown).iter().map(|r| r.id).collect::<Vec<_>>(), ["room"]);
    }

    #[test]
    fn a_slider_steps_on_its_grid_and_is_sent_once_it_rests() {
        let tile = light(true, Some(50));
        let mut s = Sheet::new("light.lamp");
        assert_eq!(s.key("down", &tile, 1000), Done::Nothing);
        assert_eq!(s.key("right", &tile, 1000), Done::Nothing, "a slider is not sent at once");
        assert_eq!(s.key("right", &tile, 1100), Done::Nothing);
        assert_eq!(s.due(1200), None, "a held key keeps it waiting");
        assert_eq!(s.due(1500), Some(Act { id: "light.lamp".into(), verb: "brightness", value: json!(70.0) }), "two presses, two steps of ten, one call");
        // what was set is shown until Home Assistant's answer, then the tile's own value
        assert_eq!(s.rows(&tile, 1600)[1].value, "70%");
        assert_eq!(s.rows(&tile, 1100 + HOLD_SHOWN_MS)[1].value, "50%");
        // never beyond its range
        let top = light(true, Some(100));
        assert_eq!(stepped(&rows_of(&top)[1], 1), Some(100.0));
        assert_eq!(stepped(&rows_of(&light(true, Some(0)))[1], -1), Some(0.0));
    }

    #[test]
    fn the_power_row_toggles_on_ok_left_and_right_and_back_closes() {
        let tile = light(true, Some(50));
        let mut s = Sheet::new("light.lamp");
        for k in ["ok", "left", "right"] {
            assert_eq!(s.key(k, &tile, 0), Done::Send(Act { id: "light.lamp".into(), verb: "toggle", value: Value::Null }), "{k}");
        }
        assert_eq!(s.key("back", &tile, 0), Done::Close);
    }

    #[test]
    fn a_thermostat_walks_its_modes_and_steps_its_set_point() {
        let tile = thermostat();
        let rows = rows_of(&tile);
        assert_eq!(rows.iter().map(|r| (r.id, r.value.as_str())).collect::<Vec<_>>(), [("now", "68\u{b0} \u{b7} Heating"), ("mode", "Heat"), ("target", "70\u{b0}")]);
        let mut s = Sheet::new("climate.t");
        assert_eq!(s.key("right", &tile, 0), Done::Send(Act { id: "climate.t".into(), verb: "hvac_mode", value: json!("cool") }));
        assert_eq!(s.key("left", &tile, 0), Done::Send(Act { id: "climate.t".into(), verb: "hvac_mode", value: json!("off") }));
        s.key("down", &tile, 10);
        s.key("right", &tile, 10);
        assert_eq!(s.due(400), Some(Act { id: "climate.t".into(), verb: "temperature", value: json!(71.0) }));
        // a heat/cool thermostat has two set points, each sent with the other as it is
        let mut range = thermostat();
        range["ctl"]["target"] = Value::Null;
        range["ctl"]["low"] = json!(66.0);
        range["ctl"]["high"] = json!(74.0);
        let mut s = Sheet::new("climate.t");
        s.key("down", &range, 0);
        s.key("right", &range, 0);
        s.key("right", &range, 0);
        assert_eq!(s.due(1000), Some(Act { id: "climate.t".into(), verb: "range", value: json!({"low": 68.0, "high": 74.0}) }));
        // off, it has no set point to set
        let mut off = thermostat();
        off["ctl"]["mode"] = json!("off");
        assert_eq!(rows_of(&off).iter().map(|r| r.id).collect::<Vec<_>>(), ["now", "mode"]);
    }

    #[test]
    fn a_cover_has_open_close_stop_and_a_position() {
        let tile = json!({"id": "cover.b", "name": "Blinds", "group": "covers", "can": ["sheet"], "available": true, "verbs": ["open_cover", "close_cover", "stop_cover", "set_cover_position"], "ctl": {"state": "open", "position": 60}});
        let rows = rows_of(&tile);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), ["open", "close", "stop", "position"]);
        let mut s = Sheet::new("cover.b");
        s.key("down", &tile, 0);
        assert_eq!(s.key("ok", &tile, 0), Done::Send(Act { id: "cover.b".into(), verb: "close", value: Value::Null }));
        // a cover that is a garage door is made with no verbs: nothing to press, and OK closes the sheet
        let mut barred = tile.clone();
        barred["verbs"] = json!([]);
        barred["can"] = json!([]);
        assert_eq!(Sheet::new("cover.b").key("ok", &barred, 0), Done::Close);
    }

    #[test]
    fn a_thing_that_is_only_shown_has_a_sheet_of_words_and_nothing_to_press() {
        let lock = json!({"id": "lock.front", "name": "Front door", "area": "Hall", "group": "locks", "words": "Locked", "can": [], "verbs": [], "available": true, "ctl": {}});
        let rows = rows_of(&lock);
        assert_eq!(rows.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), [Kind::Info]);
        let mut s = Sheet::new("lock.front");
        assert_eq!(s.front(&rows), None);
        assert_eq!(s.key("left", &lock, 0), Done::Nothing);
        assert_eq!(s.key("ok", &lock, 0), Done::Close);
        let gone = json!({"id": "light.x", "name": "X", "group": "lights", "can": [], "available": false, "ctl": {}});
        assert!(rows_of(&gone).is_empty(), "an unavailable thing offers nothing");
    }

    fn lock(state: &str, sensitive: &[&str]) -> Value {
        json!({"id": "lock.front", "name": "Front door", "area": "Hall", "group": "locks", "words": "Locked", "can": [], "available": true, "verbs": ["lock"], "sensitive": sensitive, "ctl": {"state": state}})
    }

    #[test]
    fn a_locked_lock_has_an_unlock_row_that_asks_first_and_an_open_one_has_a_lock_row() {
        let rows = rows_of(&lock("locked", &["unlock"]));
        assert_eq!(rows.iter().map(|r| (r.id, r.danger)).collect::<Vec<_>>(), [("unlock", true), ("room", false)]);
        assert_eq!(rows_of(&lock("unlocked", &["unlock"])).iter().map(|r| r.id).collect::<Vec<_>>(), ["lock", "room"], "an open lock is locked, not unlocked");
        // Home Assistant does not offer unlocking: the sheet says so and has no row to press
        let rows = rows_of(&lock("locked", &[]));
        assert_eq!(rows.iter().map(|r| (r.id, r.kind.clone())).collect::<Vec<_>>(), [("unlockinfo", Kind::Info), ("room", Kind::Info)]);
        // a lock that is only shown has no controls at all
        let mut shown = lock("locked", &["unlock"]);
        shown["verbs"] = json!([]);
        shown["sensitive"] = json!([]);
        assert_eq!(rows_of(&shown).iter().map(|r| r.id).collect::<Vec<_>>(), ["room"]);
    }

    #[test]
    fn the_unlock_question_has_cancel_in_front_and_only_the_second_answer_unlocks() {
        let tile = lock("locked", &["unlock"]);
        let mut s = Sheet::new("lock.front");
        assert_eq!(s.key("ok", &tile, 0), Done::Nothing, "OK on the Unlock row asks, it does not unlock");
        assert_eq!(s.ask(&tile).as_deref(), Some("Unlock Front door?"));
        let rows = s.rows(&tile, 0);
        assert_eq!(rows.iter().map(|r| (r.id, r.danger)).collect::<Vec<_>>(), [("cancel", false), ("unlock_yes", true)]);
        assert_eq!(s.front(&rows), Some(0), "the safe answer is in front");
        // left and right do nothing here, Back and Cancel take the question away and keep the sheet
        for k in ["left", "right"] {
            assert_eq!(s.key(k, &tile, 0), Done::Nothing);
        }
        assert_eq!(s.key("back", &tile, 0), Done::Nothing);
        assert!(!s.asking);
        s.key("ok", &tile, 0);
        assert_eq!(s.key("ok", &tile, 0), Done::Nothing, "OK on Cancel");
        assert!(!s.asking);
        // down and OK: the one way to Unlock
        s.key("ok", &tile, 0);
        s.key("down", &tile, 0);
        assert_eq!(s.key("ok", &tile, 0), Done::Unlock("lock.front".into()));
        assert!(!s.asking, "asked once, answered once");
        assert_eq!(s.key("back", &tile, 0), Done::Close);
        // a sheet that opens on the question (OK on a locked lock's tile)
        let q = Sheet::asking("lock.front");
        assert_eq!(q.rows(&tile, 0).len(), 2);
    }

    #[test]
    fn no_key_on_any_sheet_makes_an_act_that_unlocks_or_unlatches() {
        let tiles = [lock("locked", &["unlock"]), lock("unlocked", &["unlock"]), lock("locked", &[]), light(true, Some(50)), thermostat()];
        for tile in tiles {
            let id = tile["id"].as_str().unwrap().to_owned();
            for asking in [false, true] {
                let mut s = if asking { Sheet::asking(&id) } else { Sheet::new(&id) };
                for step in 0..12 {
                    let key = ["ok", "down", "left", "right", "ok", "up", "ok"][step % 7];
                    if let Done::Send(act) = s.key(key, &tile, step as u64) {
                        assert!(!matches!(act.verb, "unlock" | "open") || id.starts_with("cover"), "{id} {key}: {act:?}");
                    }
                    if let Some(act) = s.due(10_000) {
                        assert!(!matches!(act.verb, "unlock" | "open"));
                    }
                }
            }
        }
    }

    #[test]
    fn the_card_says_why_it_has_no_tile_in_the_helpers_words() {
        assert_eq!(note_of(&json!({"state": "off"})).0, "Not switched on");
        assert_eq!(note_of(&json!({"state": "unpaired"})).0, "Not paired");
        assert_eq!(note_of(&json!({"state": "waiting"})).0, "Waiting for Home Assistant");
        assert_eq!(note_of(&json!({"state": "empty", "shared": 0})).0, "Nothing shared yet");
        assert_eq!(note_of(&json!({"state": "empty", "shared": 4})).0, "Nothing on Home");
        assert_eq!(note_of(&Value::Null).0, "Starting");
        assert_eq!(heading_words(&json!({"state": "connected", "words": "6 things"})), "6 things");
    }

    #[test]
    fn the_sheet_is_in_the_middle_of_the_screen_and_its_rows_are_inside_its_pane() {
        for scale in [1.0, 2.0] {
            let l = layout(4, scale);
            assert!(l.pane.x > 0.0 && l.pane.right() < 1920.0 && l.pane.y >= 40.0 && l.pane.bottom() <= 1080.0, "{scale}: {:?}", l.pane);
            assert!(l.rows.iter().all(|r| r.x >= l.pane.x && r.right() <= l.pane.right() && r.bottom() <= l.pane.bottom()), "{scale}");
        }
        assert!(layout(4, 2.0).pane.w > layout(4, 1.0).pane.w);
    }
}
