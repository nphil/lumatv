//! The cards' contents (`widgets/*.qml`), each in its card's content box (`padSide` 32, `padTop` 28): the clock (digital or analogue,
//! `ClockWidget.qml`'s plan for the room it has: tiny, column, month, zoned or regular), the weather (the place, the reading, the week as
//! rows with the range bars, or columns on a wide card), the news (one story at a time, slid every `news.slideMs`), at home (the Home
//! Assistant plugin's cards as rows) and the cameras (the Scrypted topic's cameras as rows with the link's state in words). Every card's
//! words are one [`Block`] (one layer, glyph quads) and its pictures one or two image layers painted on the CPU, so a card costs the
//! compositor three or four layers; the pane and its lit glass are the surface's.

use super::clock::{self, Civil};
use crate::kit::Kit;
use crate::kit::picture::{self, Fit, Picture};
use crate::kit::text::role_style;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use ui::components::text::{ellipsise, wrap};
use ui::components::{Align, Block, Style};
use ui::geometry::Frame;
use ui::scene::ImageLayer;
use ui::text::Weight;

pub const PAD_SIDE: f32 = 32.0;
pub const PAD_TOP: f32 = 28.0;
const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

/// A line of a block: its frame in the content box, its style and its text.
type Line = (Frame, Style, String);

/// A card's words: one block (one layer) and what it says, kept beside it so the lint can read every line's box, style and text.
pub struct Words {
    block: Block,
    lines: Vec<(Frame, Style)>,
    texts: Vec<String>,
    at: Props,
}

impl Words {
    pub fn new(kit: &mut Kit, at: Frame, z: i32, lines: Vec<Line>) -> std::io::Result<Self> {
        let spec = lines.iter().map(|l| (l.0, l.1)).collect();
        let texts = lines.iter().map(|l| l.2.clone()).collect();
        Ok(Self { block: Block::new(&mut kit.ui, at, z, lines)?, lines: spec, texts, at: HIDDEN })
    }

    pub fn set_many(&mut self, kit: &mut Kit, changes: &[(usize, &str, Rgba)]) {
        for &(i, text, colour) in changes {
            if let Some(t) = self.texts.get_mut(i) {
                *t = text.to_owned();
            }
            if let Some(l) = self.lines.get_mut(i) {
                l.1.colour = colour;
            }
        }
        self.block.set_many(&mut kit.ui, changes);
    }

    pub fn set_props(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
    }

    pub fn is_glyph_run(&self) -> bool {
        self.block.is_glyph_run()
    }

    /// Every line that says something, as the lint takes it: its box on the screen (`origin` is where the block's own origin is).
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, name: &str, origin: (f32, f32), panel: Option<usize>) {
        for (i, ((frame, style), text)) in self.lines.iter().zip(self.texts.iter()).enumerate() {
            if text.is_empty() {
                continue;
            }
            let f = Frame::new(origin.0 + self.at.x + frame.x, origin.1 + self.at.y + frame.y, frame.w, frame.h);
            lint.word(kit, &format!("{name}.{i}"), text, f, *style, panel);
        }
    }
}

/// Pictures the cards paint: the module's weather icons and the plugin's base icons, decoded once.
#[derive(Default)]
pub struct Pictures {
    module: Option<PathBuf>,
    kept: HashMap<String, Option<Picture>>,
}

impl Pictures {
    pub fn new(module: &Path) -> Self {
        Self { module: Some(module.to_path_buf()), kept: HashMap::new() }
    }

    /// `<module>/icons/<name>.png`, or an absolute path.
    pub fn get(&mut self, name: &str) -> Option<&Picture> {
        if !self.kept.contains_key(name) {
            let path = if name.starts_with('/') { PathBuf::from(name) } else { self.module.as_ref()?.join("icons").join(format!("{name}.png")) };
            let pic = std::fs::read(&path).ok().and_then(|b| picture::decode(&b).ok());
            self.kept.insert(name.to_owned(), pic);
        }
        self.kept.get(name).and_then(Option::as_ref)
    }
}

// ---- CPU painting into an art layer (premultiplied RGBA)
pub struct Canvas<'a> {
    pub px: &'a mut [u8],
    pub w: u32,
    pub h: u32,
}

impl Canvas<'_> {
    fn over(&mut self, x: i32, y: i32, src: &[u8; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || src[3] == 0 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let a = 255 - src[3] as u32;
        for c in 0..4 {
            self.px[i + c] = (src[c] as u32 + self.px[i + c] as u32 * a / 255).min(255) as u8;
        }
    }

    /// Paints `pic` scaled to `side` with its top left at (`x`, `y`).
    pub fn picture(&mut self, pic: &Picture, x: i32, y: i32, w: u32, h: u32) {
        let fitted = if pic.width == w && pic.height == h { None } else { Some(pic.fitted(w, h, Fit::Contain)) };
        let p = fitted.as_ref().unwrap_or(pic);
        for yy in 0..p.height {
            for xx in 0..p.width {
                let i = ((yy * p.width + xx) * 4) as usize;
                let src = [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]];
                self.over(x + xx as i32, y + yy as i32, &src);
            }
        }
    }

    /// A rounded rectangle of one colour.
    pub fn rounded(&mut self, f: Frame, radius: f32, colour: Rgba) {
        let r = radius.min(f.w / 2.0).min(f.h / 2.0);
        let (x0, y0, x1, y1) = (f.x.floor().max(0.0) as i32, f.y.floor().max(0.0) as i32, f.right().ceil().min(self.w as f32) as i32, f.bottom().ceil().min(self.h as f32) as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let dx = (f.x + r - px).max(px - (f.right() - r)).max(0.0);
                let dy = (f.y + r - py).max(py - (f.bottom() - r)).max(0.0);
                let d = (dx * dx + dy * dy).sqrt() - r;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = colour.3 as f32 * cov / 255.0;
                    let src = [(colour.0 as f32 * a) as u8, (colour.1 as f32 * a) as u8, (colour.2 as f32 * a) as u8, (255.0 * a) as u8];
                    self.over(x, y, &src);
                }
            }
        }
    }

    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, colour: Rgba) {
        self.rounded(Frame::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, colour);
    }
}

/// Rasters the characters of these types into the atlas now, so a card's words (and the digits and degrees they change to) never
/// wait for a chunk on a frame.
fn warm(kit: &mut Kit, mut types: Vec<ui::text::Type>) {
    types.sort_by_key(|t| (t.px, t.weight as u8));
    types.dedup();
    if !types.is_empty() {
        kit.ui.warm(&types);
        let sets: Vec<(ui::text::Type, &str)> = types.iter().map(|t| (*t, "0123456789:\u{b0}\u{b7}%\u{2019}\u{201c}\u{201d}")).collect();
        kit.ui.warm_chars(&sets);
    }
}

use crate::kit::button::faded;

fn art(kit: &mut Kit, f: Frame, z: i32) -> std::io::Result<ImageLayer> {
    let mut l = kit.ui.scene.image(f.w.max(1.0).round() as u32, f.h.max(1.0).round() as u32, z, HIDDEN)?;
    l.clear();
    l.damage(None);
    Ok(l)
}

/// The kind of content a card has.
pub enum Content {
    Clock(Clock),
    Weather(Weather),
    News(News),
    AtHome(AtHome),
    Cameras(Cameras),
    Unknown(Words),
}

impl Content {
    /// Makes the content of `kind` for a box `w` by `h`, inside the card's group (the caller adopts the layers).
    pub fn new(kit: &mut Kit, kind: &str, w: f32, h: f32, z: i32, theme: &Value, entry: &Value) -> std::io::Result<Self> {
        Ok(match kind {
            "clock" => Content::Clock(Clock::new(kit, w, h, z, theme, entry)?),
            "weather" => Content::Weather(Weather::new(kit, w, h, z, theme)?),
            "news" => Content::News(News::new(kit, w, h, z)?),
            "homeassistant" => Content::AtHome(AtHome::new(kit, w, h, z, entry)?),
            "cameras" => Content::Cameras(Cameras::new(kit, w, h, z)?),
            other => {
                let ink2 = kit.ui.look.ink2();
                let s = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
                let lines = vec![(Frame::new(0.0, 0.0, w, kit.line(TypeRole::Caption)), s, format!("No card of type \u{201c}{other}\u{201d}"))];
                Content::Unknown(Words::new(kit, Frame::new(0.0, 0.0, w, h), z, lines)?)
            }
        })
    }

    pub fn show(&mut self, at: Props) {
        match self {
            Content::Clock(c) => c.show(at),
            Content::Weather(c) => c.show(at),
            Content::News(c) => c.show(at),
            Content::AtHome(c) => c.show(at),
            Content::Cameras(c) => c.show(at),
            Content::Unknown(b) => b.set_props(at),
        }
    }

    pub fn is_glyph_run(&self) -> bool {
        match self {
            Content::Clock(c) => c.block.is_glyph_run(),
            Content::Weather(c) => c.block.is_glyph_run(),
            Content::News(c) => c.block.is_glyph_run(),
            Content::AtHome(c) => c.block.is_glyph_run(),
            Content::Cameras(c) => c.block.is_glyph_run(),
            Content::Unknown(b) => b.is_glyph_run(),
        }
    }

    pub fn words(&self) -> &Words {
        match self {
            Content::Clock(c) => &c.block,
            Content::Weather(c) => &c.block,
            Content::News(c) => &c.block,
            Content::AtHome(c) => &c.block,
            Content::Cameras(c) => &c.block,
            Content::Unknown(b) => b,
        }
    }
}

// ---- the clock
#[derive(Clone, Copy, Debug, PartialEq)]
struct Box {
    x: f32,
    y: f32,
    w: f32,
}

/// `ClockWidget.qml`'s plan: where the face (the time or the dial), the info (weekday and date), the week strip and the months go.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub mode: &'static str,
    pub dia: f32,
    pub ts: f32,
    pub greet: bool,
    pub date_on: bool,
    face: Box,
    face_centred: bool,
    info: Option<(Box, bool)>,
    strip: Option<(Box, f32)>,
    months: Option<(Box, f32, usize)>,
}

fn time_w(ts: f32, ampm: bool) -> f32 {
    (ts * 2.63 + if ampm { ts * 0.5 + 10.0 } else { 0.0 }).round()
}

fn fit_time(token: f32, max_w: f32, max_h: f32, ampm: bool, extra: f32) -> f32 {
    [token, 80.0, 68.0, 56.0, 48.0, 40.0].into_iter().find(|&s| s <= token && time_w(s, ampm) <= max_w && s + extra <= max_h).unwrap_or(40.0)
}

fn clampi(v: f32, a: f32, b: f32) -> f32 {
    v.round().clamp(a, b)
}

pub fn plan(w: f32, h: f32, analog: bool, greeting: bool, token: f32, ampm: bool) -> Plan {
    let mut p = Plan { mode: "regular", dia: 0.0, ts: 0.0, greet: false, date_on: false, face: Box { x: 0.0, y: 0.0, w }, face_centred: false, info: None, strip: None, months: None };
    let gap = 36.0;
    if w < 260.0 || h < 120.0 {
        p.mode = "tiny";
        if analog {
            p.dia = clampi(h.min(w).min(120.0), 40.0, 120.0);
            p.face = Box { x: 0.0, y: ((h - p.dia) / 2.0).round(), w: p.dia };
            if w - p.dia >= 150.0 && h >= 64.0 {
                p.info = Some((Box { x: p.dia + 24.0, y: ((h - 64.0) / 2.0).round(), w: w - p.dia - 24.0 }, false));
            } else {
                p.face.x = ((w - p.dia) / 2.0).round();
            }
        } else {
            p.ts = fit_time(token.min(64.0), w, h, ampm, 0.0);
            p.date_on = h - p.ts >= 34.0;
            p.face = Box { x: 0.0, y: ((h - p.ts - if p.date_on { 34.0 } else { 0.0 }) / 2.0).round(), w };
        }
        return p;
    }
    if w < 560.0 && h >= 300.0 {
        p.mode = "column";
        if analog {
            p.dia = clampi((h - 166.0).min(200.0).min(w), 96.0, 200.0);
            let ch = p.dia + 16.0 + 64.0 + 16.0 + 70.0;
            let y0 = ((h - ch) / 2.0).round();
            p.face = Box { x: ((w - p.dia) / 2.0).round(), y: y0, w: p.dia };
            p.info = Some((Box { x: 0.0, y: y0 + p.dia + 16.0, w }, true));
            let cell = clampi((w / 7.0).min(56.0), 28.0, 56.0);
            p.strip = Some((Box { x: ((w - 7.0 * cell) / 2.0).round(), y: y0 + p.dia + 16.0 + 64.0 + 16.0, w: 7.0 * cell }, cell));
        } else {
            p.ts = fit_time(token, w, h, ampm, 0.0);
            p.greet = greeting;
            p.date_on = true;
            let fh = if p.greet { 43.0 } else { 0.0 } + p.ts + 8.0 + 34.0;
            let cell = clampi((w / 7.0).min(56.0), 28.0, 56.0);
            let y1 = ((h - (fh + 24.0 + 70.0)) / 2.0).round();
            p.face = Box { x: 0.0, y: y1, w };
            p.face_centred = true;
            p.strip = Some((Box { x: ((w - 7.0 * cell) / 2.0).round(), y: y1 + fh + 24.0, w: 7.0 * cell }, cell));
        }
        return p;
    }
    if w >= 560.0 && h >= 230.0 {
        p.mode = "month";
        let col_w;
        if analog {
            p.dia = clampi((h - 84.0).min(220.0).min(w * 0.32), 96.0, 220.0);
            col_w = p.dia.max(190.0);
            let y2 = ((h - (p.dia + 80.0)) / 2.0).round();
            p.face = Box { x: ((col_w - p.dia) / 2.0).round(), y: y2, w: p.dia };
            p.info = Some((Box { x: 0.0, y: y2 + p.dia + 14.0, w: col_w }, true));
        } else {
            p.greet = greeting;
            p.ts = fit_time(token, (w * 0.45).round(), h, ampm, 0.0);
            p.date_on = true;
            col_w = time_w(p.ts, ampm);
            let bh = if p.greet { 43.0 } else { 0.0 } + p.ts + 8.0 + 34.0;
            p.face = Box { x: 0.0, y: ((h - bh) / 2.0).round(), w: col_w };
        }
        let rx = col_w + 48.0;
        let rw = w - rx;
        let cell = clampi((rw / 7.0).min(64.0).min((h - 44.0) / 7.0), 24.0, 64.0);
        let count = clampi(((rw + 48.0) / (7.0 * cell + 48.0)).floor(), 1.0, 3.0) as usize;
        let mw = count as f32 * 7.0 * cell + (count as f32 - 1.0) * 48.0;
        p.months = Some((Box { x: rx + ((rw - mw) / 2.0).round(), y: 0.0, w: mw }, cell, count));
        return p;
    }
    if w >= 700.0 {
        p.mode = "zoned";
        let (face_w, mut info_w);
        if analog {
            p.dia = clampi(h.min(166.0), 96.0, 166.0);
            face_w = p.dia;
            info_w = 230.0;
        } else {
            p.greet = greeting && h >= 150.0;
            let spread = w >= 1100.0;
            p.date_on = !spread;
            p.ts = fit_time(token, (w * 0.5).round(), h - if p.greet { 43.0 } else { 0.0 } - if p.date_on { 34.0 } else { 0.0 }, ampm, 0.0);
            face_w = time_w(p.ts, ampm);
            info_w = if spread { 230.0 } else { 0.0 };
        }
        let mut cell = clampi(((w - face_w - info_w - 2.0 * gap) / 7.0).min(64.0), 36.0, 64.0);
        let mut strip_w = 7.0 * cell;
        let mut spare = w - face_w - info_w - strip_w;
        if spare < 48.0 && cell > 36.0 {
            cell = 36.0;
            strip_w = 252.0;
            spare = w - face_w - info_w - strip_w;
        }
        let g = (spare / if info_w > 0.0 { 2.0 } else { 1.0 }).round().max(24.0);
        if analog {
            p.face = Box { x: 0.0, y: ((h - p.dia) / 2.0).round(), w: p.dia };
            p.info = Some((Box { x: face_w + g, y: ((h - 70.0) / 2.0).round(), w: info_w }, false));
            p.strip = Some((Box { x: face_w + g + info_w + g, y: ((h - 70.0) / 2.0).round(), w: strip_w }, cell));
        } else {
            let fh2 = if p.greet { 43.0 } else { 0.0 } + p.ts + if p.date_on { 42.0 } else { 0.0 };
            p.face = Box { x: 0.0, y: ((h - fh2) / 2.0).round(), w: face_w };
            if info_w > 0.0 {
                p.info = Some((Box { x: face_w + g, y: ((h - 70.0) / 2.0).round(), w: info_w }, false));
            }
            p.strip = Some((Box { x: face_w + g + info_w + if info_w > 0.0 { g } else { 0.0 }, y: ((h - 70.0) / 2.0).round(), w: strip_w }, cell));
        }
        if info_w > 0.0 {
            info_w = 0.0;
        }
        let _ = info_w;
        return p;
    }
    if analog {
        p.dia = clampi(h.min(166.0), 96.0, 166.0);
        let sx = p.dia + gap;
        let sw = w - sx;
        let cell = clampi((sw / 7.0).min(56.0), 0.0, 56.0);
        let k = (cell / 48.0).clamp(0.72, 1.17);
        let show_strip = cell >= 30.0 && h - 70.0 * k >= 80.0;
        p.face = Box { x: 0.0, y: ((h - p.dia) / 2.0).round(), w: p.dia };
        p.info = Some((Box { x: sx, y: if show_strip { 0.0 } else { ((h - 70.0) / 2.0).round() }, w: sw }, false));
        if show_strip {
            p.strip = Some((Box { x: sx, y: (h - 70.0 * k).round(), w: 7.0 * cell }, cell));
        }
    } else {
        p.greet = greeting && h >= 150.0;
        let gh = if p.greet { 43.0 } else { 0.0 };
        p.date_on = h >= gh + 52.0 + 41.0;
        p.ts = fit_time(token, w, h + 10.0, ampm, gh + if p.date_on { 41.0 } else { 0.0 });
        p.date_on = p.date_on && h + 10.0 >= gh + p.ts + 41.0;
        p.face = Box { x: 0.0, y: 0.0, w };
    }
    p
}

pub struct Clock {
    pub block: Words,
    dial: Option<ImageLayer>,
    hands: Option<ImageLayer>,
    plan: Plan,
    analog: bool,
    twelve: bool,
    second: Option<Rgba>,
    /// Which block lines are what: the greeting, the time, the ampm, the weekday, the date, the strip's initials and numbers, the months.
    lines: ClockLines,
    at: Props,
    last: Option<(u32, u32, u32, u32)>,
}

#[derive(Default)]
struct ClockLines {
    greet: Option<usize>,
    time: Option<usize>,
    ampm: Option<usize>,
    weekday: Option<usize>,
    date: Option<usize>,
    /// (initial line, number line) per day of the strip.
    strip: Vec<(usize, usize)>,
    /// Per month: the title line, then 42 cell lines.
    months: Vec<(usize, Vec<usize>)>,
}

impl Clock {
    fn new(kit: &mut Kit, w: f32, h: f32, z: i32, theme: &Value, entry: &Value) -> std::io::Result<Self> {
        let style = entry.pointer("/settings/style").or_else(|| theme.pointer("/clock/style")).and_then(Value::as_str).unwrap_or("digital");
        let analog = style == "analog";
        let twelve = theme.pointer("/clock/twelveHour").and_then(Value::as_bool) != Some(false);
        let greeting = theme.pointer("/clock/greeting").and_then(Value::as_bool) != Some(false);
        let token = theme.pointer("/type/clock").and_then(Value::as_f64).unwrap_or(92.0) as f32;
        let second = theme.pointer("/clock/secondHand").and_then(Value::as_str).and_then(lumatv_tokens::colour).map(|c| c.premultiply(0.0));
        // At a larger text size the card's room is planned as if it were that much smaller and the plan scaled up with the type:
        // every box grows with its words, and a card that is too small for a part at that size loses the part, as a smaller card does.
        let k = kit.text_scale.max(1.0);
        let p = plan(w / k, h / k, analog, greeting, token, twelve);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let ink3 = kit.ui.look.ink3();
        let accent = kit.ui.look.accent();
        let mut lines: Vec<Line> = Vec::new();
        let mut map = ClockLines::default();
        let types: std::cell::RefCell<Vec<ui::text::Type>> = std::cell::RefCell::new(Vec::new());
        let sized = |kit: &Kit, px: f32, weight: Weight, colour: Rgba, align: Align| -> Style {
            let mut ty = kit.ty(TypeRole::Label);
            ty.px = (px * k).round().max(8.0) as u32;
            ty.weight = weight;
            types.borrow_mut().push(ty);
            ui::components::style(ty, colour, align)
        };
        let align = if p.face_centred { Align::Centre } else { Align::Left };
        if !analog {
            let mut y = p.face.y;
            if p.greet {
                map.greet = Some(lines.len());
                lines.push((Frame::new(p.face.x, y, p.face.w, 34.0), sized(kit, 26.0, Weight::Medium, ink2, align), String::new()));
                y += 43.0;
            }
            let tw = time_w(p.ts, twelve);
            let tx = if p.face_centred { p.face.x + ((p.face.w - tw) / 2.0).round() } else { p.face.x };
            map.time = Some(lines.len());
            lines.push((Frame::new(tx, y, p.ts * 2.63, p.ts * 1.1), sized(kit, p.ts, Weight::SemiBold, ink, Align::Left), String::new()));
            if twelve {
                map.ampm = Some(lines.len());
                lines.push((Frame::new(tx + p.ts * 2.63 + 10.0, y + p.ts * 0.5, p.ts * 0.6, p.ts * 0.5), sized(kit, (p.ts * 0.35).max(18.0), Weight::SemiBold, ink2, Align::Left), String::new()));
            }
            y += p.ts + 8.0;
            if p.date_on {
                map.date = Some(lines.len());
                lines.push((Frame::new(p.face.x, y, p.face.w, 34.0), sized(kit, 24.0, Weight::Medium, ink2, align), String::new()));
            }
        }
        if let Some((b, centred)) = p.info {
            let a = if centred { Align::Centre } else { Align::Left };
            let wd = if p.mode == "tiny" { 24.0 } else { 32.0 };
            map.weekday = Some(lines.len());
            lines.push((Frame::new(b.x, b.y, b.w, wd * 1.3), sized(kit, wd, Weight::SemiBold, ink, a), String::new()));
            map.date = Some(lines.len());
            lines.push((Frame::new(b.x, b.y + wd * 1.3 + 2.0, b.w, 24.0 * 1.3), sized(kit, 24.0, Weight::Medium, ink2, a), String::new()));
        }
        if let Some((b, cell)) = p.strip {
            let kc = (cell / 48.0).clamp(0.72, 1.17);
            for i in 0..7 {
                let x = b.x + i as f32 * cell;
                let ii = lines.len();
                lines.push((Frame::new(x, b.y, cell, 24.0 * kc), sized(kit, (17.0 * kc).max(14.0), Weight::Medium, ink3, Align::Centre), String::new()));
                let ni = lines.len();
                lines.push((Frame::new(x, b.y + 28.0 * kc, cell, 36.0 * kc), sized(kit, (22.0 * kc).max(16.0), Weight::SemiBold, ink, Align::Centre), String::new()));
                map.strip.push((ii, ni));
            }
        }
        if let Some((b, cell, count)) = p.months {
            let fs = (cell * 0.42).clamp(15.0, 26.0);
            for m in 0..count {
                let mx = b.x + m as f32 * (7.0 * cell + 48.0);
                let ti = lines.len();
                lines.push((Frame::new(mx, 0.0, 7.0 * cell, 30.0), sized(kit, 22.0, Weight::SemiBold, ink, Align::Left), String::new()));
                let mut cells = Vec::new();
                for c in 0..42 {
                    let (col, row) = (c % 7, c / 7);
                    cells.push(lines.len());
                    lines.push((Frame::new(mx + col as f32 * cell, 44.0 + row as f32 * cell, cell, cell), sized(kit, fs, Weight::Medium, ink2, Align::Centre), String::new()));
                }
                map.months.push((ti, cells));
            }
        }
        let _ = accent;
        warm(kit, types.into_inner());
        for l in &mut lines {
            l.0 = Frame::new(l.0.x * k, l.0.y * k, l.0.w * k, l.0.h * k);
        }
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        let p = Plan { dia: p.dia * k, face: Box { x: p.face.x * k, y: p.face.y * k, w: p.face.w * k }, ..p };
        let (dial, hands) = if analog {
            let side = p.dia.max(1.0) as u32;
            let mut d = kit.ui.scene.image(side, side, z + 1, HIDDEN)?;
            clock::draw_dial(d.pixels_mut(), side, ink, ink3);
            d.damage(None);
            let hd = kit.ui.scene.image(side, side, z + 2, HIDDEN)?;
            (Some(d), Some(hd))
        } else {
            (None, None)
        };
        Ok(Self { block, dial, hands, plan: p, analog, twelve, second, lines: map, at: HIDDEN, last: None })
    }

    fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        let fx = self.plan.face.x;
        let fy = self.plan.face.y;
        if let Some(d) = &self.dial {
            d.set(Props { x: at.x + fx, y: at.y + fy, ..at });
        }
        if let Some(h) = &self.hands {
            h.set(Props { x: at.x + fx, y: at.y + fy, ..at });
        }
    }

    /// The time now; drawn again only when a shown part changed (the minute, or the second when the hand shows).
    pub fn tick(&mut self, kit: &mut Kit, t: &Civil) {
        let key = (t.day, t.hour, t.minute, if self.analog && self.second.is_some() { t.second } else { 0 });
        if self.last == Some(key) {
            return;
        }
        let day_changed = self.last.is_none_or(|l| l.0 != t.day);
        self.last = Some(key);
        let mut changes: Vec<(usize, String, Rgba)> = Vec::new();
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let ink3 = kit.ui.look.ink3();
        let accent = kit.ui.look.accent();
        let (time, ampm) = t.time_words(self.twelve);
        if let Some(i) = self.lines.greet {
            changes.push((i, t.greeting().to_owned(), ink2));
        }
        if let Some(i) = self.lines.time {
            changes.push((i, time, ink));
        }
        if let Some(i) = self.lines.ampm {
            changes.push((i, ampm.to_owned(), ink2));
        }
        if day_changed {
            if let Some(i) = self.lines.weekday {
                changes.push((i, t.weekday_name().to_owned(), ink));
            }
            if let Some(i) = self.lines.date {
                changes.push((i, if self.lines.weekday.is_some() { t.date_line() } else { format!("{}, {}", t.weekday_name(), t.date_line()) }, ink2));
            }
            for (i, (ii, ni)) in self.lines.strip.iter().enumerate() {
                let (n, initial, today) = t.week()[i];
                changes.push((*ii, initial.to_owned(), ink3));
                changes.push((*ni, n.to_string(), if today { accent } else { ink }));
            }
            for (m, (ti, cells)) in self.lines.months.iter().enumerate() {
                let (title, days) = t.month(m as u32);
                changes.push((*ti, title, ink));
                for (c, li) in cells.iter().enumerate() {
                    match days.get(c).copied().flatten() {
                        Some((n, today)) => changes.push((*li, n.to_string(), if today { accent } else { ink2 })),
                        None => changes.push((*li, String::new(), ink2)),
                    }
                }
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.block.set_many(kit, &refs);
        if let Some(h) = &mut self.hands {
            let side = h.width();
            clock::draw_hands(h.pixels_mut(), side, t, ink, self.second);
            h.damage(None);
        }
    }

    pub fn second_hand(&self) -> bool {
        self.analog && self.second.is_some()
    }
}

// ---- the weather
pub struct Weather {
    pub block: Words,
    now_icon: ImageLayer,
    rows_art: ImageLayer,
    w: f32,
    h: f32,
    mode: &'static str,
    lines: WeatherLines,
    at: Props,
    shown: Option<String>,
    now_side: u32,
    now_at: (f32, f32),
    rows_at: (f32, f32),
}

#[derive(Default)]
struct WeatherLines {
    place: usize,
    temp: usize,
    cond: usize,
    stats: usize,
    stats_shown: bool,
    stats_style: Option<Style>,
    stats_w: f32,
    /// Per day: day, lo, hi (rows) or day, hi, lo (columns).
    days: Vec<(usize, usize, usize)>,
    row_top: f32,
    row_h: f32,
    day_w: f32,
    num_w: f32,
    icon: f32,
    col_w: f32,
    col_x: f32,
}

impl Weather {
    fn new(kit: &mut Kit, w: f32, h: f32, z: i32, theme: &Value) -> std::io::Result<Self> {
        // planned at the text size's reduction and scaled up with the type, as the clock is
        let scale = kit.text_scale.max(1.0);
        let (w, h) = (w / scale, h / scale);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let types: std::cell::RefCell<Vec<ui::text::Type>> = std::cell::RefCell::new(Vec::new());
        let sized = |kit: &Kit, px: f32, weight: Weight, colour: Rgba, align: Align| -> Style {
            let mut ty = kit.ty(TypeRole::Label);
            ty.px = (px * scale).round().max(8.0) as u32;
            ty.weight = weight;
            types.borrow_mut().push(ty);
            ui::components::style(ty, colour, align)
        };
        let token = theme.pointer("/type/temp").and_then(Value::as_f64).unwrap_or(84.0) as f32;
        let wide = w >= 620.0;
        let mode = if !wide {
            "stack"
        } else if w < 1100.0 && h >= 240.0 {
            "list"
        } else {
            "columns"
        };
        let compact = h < 130.0;
        let now_icon = if compact {
            (h - 8.0).clamp(40.0, 64.0)
        } else if mode == "stack" {
            96.0
        } else if h >= 300.0 {
            112.0
        } else if h >= 200.0 {
            96.0
        } else {
            72.0
        };
        let now_w = match mode {
            "stack" => w,
            "list" => (w * 0.4).round().clamp(300.0, 400.0),
            _ => (w * 0.3).round().clamp(300.0, 440.0),
        };
        let show_place = !(compact || h < 110.0);
        let cond_h = if now_icon >= 96.0 { 36.0 } else { 30.0 };
        let now_temp = if compact {
            (now_icon * 0.875).round()
        } else if mode == "stack" {
            // the token's size, but no larger than the room right of the icon (a "-12°" is 2 em)
            token.min(((now_w - now_icon - (now_icon * 18.0 / 96.0).round() - 4.0) / 2.0).floor()).max(40.0)
        } else {
            (now_icon * token / 96.0).round().min(((now_w - now_icon - (now_icon * 18.0 / 96.0).round() - 4.0) / 2.0).floor()).min(h - if show_place { 40.0 } else { 0.0 } - cond_h - 4.0).max(40.0)
        };
        let core_h = if compact { now_icon } else { now_icon.max(now_temp + 4.0 + cond_h) };
        let place_h: f32 = if show_place { 40.0 } else { 0.0 };
        let now_top = if mode == "stack" && !compact { 47.0 } else { place_h + ((h - place_h - core_h) / 2.0).round().max(0.0) };
        let temp_top = now_top + if compact { ((now_icon - now_temp) / 2.0).round() } else { 0.0 };
        let stats_y = now_top + core_h + 8.0;
        let mut lines: Vec<Line> = Vec::new();
        let mut map = WeatherLines { place: 0, ..WeatherLines::default() };
        lines.push((Frame::new(0.0, 0.0, now_w, 34.0), sized(kit, 24.0, Weight::SemiBold, ink, Align::Left), String::new()));
        let temp_x = now_icon + (now_icon * 18.0 / 96.0).round();
        map.temp = lines.len();
        lines.push((Frame::new(temp_x, temp_top, now_w - temp_x, now_temp), sized(kit, now_temp, Weight::SemiBold, ink, Align::Left), String::new()));
        map.cond = lines.len();
        lines.push((Frame::new(temp_x, now_top + now_temp + 4.0, now_w - temp_x, cond_h), sized(kit, if now_icon >= 96.0 { 26.0 } else { 22.0 }, Weight::Medium, ink, Align::Left), String::new()));
        map.stats = lines.len();
        // the stats line only where it fits under the reading
        let stats_room = if stats_y + 30.0 <= h { w } else { 0.0 };
        let stats_style = sized(kit, 20.0, Weight::Medium, ink2, Align::Left);
        lines.push((Frame::new(0.0, stats_y, stats_room, 30.0), stats_style, String::new()));
        map.stats_shown = stats_room > 0.0;
        map.stats_style = Some(stats_style);
        map.stats_w = stats_room;
        // the week
        map.icon = if mode == "stack" { 26.0 } else { 34.0 };
        map.day_w = 70.0;
        map.num_w = 52.0;
        if mode == "columns" {
            map.col_x = now_w + 48.0;
            map.col_w = ((w - map.col_x) / 7.0).floor();
            for i in 0..7 {
                let x = map.col_x + i as f32 * map.col_w;
                let d = lines.len();
                lines.push((Frame::new(x, 0.0, map.col_w, 30.0), sized(kit, 22.0, Weight::Medium, ink2, Align::Centre), String::new()));
                let hi = lines.len();
                lines.push((Frame::new(x, 34.0 + 64.0 + 4.0, map.col_w, 34.0), sized(kit, 26.0, Weight::SemiBold, ink, Align::Centre), String::new()));
                let lo = lines.len();
                lines.push((Frame::new(x, 34.0 + 64.0 + 4.0 + 34.0, map.col_w, 30.0), sized(kit, 24.0, Weight::Medium, ink2, Align::Centre), String::new()));
                map.days.push((d, hi, lo));
            }
        } else {
            let (top, row_h, x0, rows) = if mode == "stack" {
                let top = stats_y + 46.0;
                (top, 46.0, 0.0, ((h - top) / 46.0).floor().clamp(0.0, 7.0) as usize)
            } else {
                let row_h = 64.0f32.min(h / 5.0).max(40.0);
                let rows = (h / row_h).floor().clamp(1.0, 7.0) as usize;
                (((h - rows as f32 * row_h) / 2.0).round(), row_h, now_w + 48.0, rows)
            };
            map.row_top = top;
            map.row_h = row_h;
            for i in 0..rows {
                let y = top + i as f32 * row_h;
                let d = lines.len();
                lines.push((Frame::new(x0, y, map.day_w, row_h), sized(kit, 23.0, Weight::Medium, ink, Align::Left), String::new()));
                let lo = lines.len();
                lines.push((Frame::new(x0 + map.day_w + map.icon + 10.0, y, map.num_w, row_h), sized(kit, 23.0, Weight::Medium, ink2, Align::Right), String::new()));
                let hi = lines.len();
                lines.push((Frame::new(w - map.num_w, y, map.num_w, row_h), sized(kit, 23.0, Weight::Medium, ink, Align::Right), String::new()));
                map.days.push((d, lo, hi));
            }
            map.col_x = x0;
        }
        warm(kit, types.into_inner());
        let k = scale;
        for l in &mut lines {
            l.0 = Frame::new(l.0.x * k, l.0.y * k, l.0.w * k, l.0.h * k);
        }
        let (w, h) = (w * k, h * k);
        map.row_top *= k;
        map.row_h *= k;
        map.day_w *= k;
        map.num_w *= k;
        map.icon *= k;
        map.col_w *= k;
        map.col_x *= k;
        map.stats_w *= k;
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        let now_side = (now_icon * k) as u32;
        let now_icon_layer = kit.ui.scene.image(now_side, now_side, z + 1, HIDDEN)?;
        let rows_art = art(kit, Frame::new(0.0, 0.0, w, h), z + 1)?;
        Ok(Self { block, now_icon: now_icon_layer, rows_art, w, h, mode, lines: map, at: HIDDEN, shown: None, now_side, now_at: (0.0, now_top * k), rows_at: (0.0, 0.0) })
    }

    fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        self.now_icon.set(Props { x: at.x + self.now_at.0, y: at.y + self.now_at.1, ..at });
        self.rows_art.set(Props { x: at.x + self.rows_at.0, y: at.y + self.rows_at.1, ..at });
    }

    /// The helper's reading (`topic.doc.display`), drawn when it changed.
    pub fn set(&mut self, kit: &mut Kit, pictures: &mut Pictures, topic: &Value) {
        let key = topic.pointer("/doc/fetchedAt").map(Value::to_string).unwrap_or_default();
        if self.shown.as_deref() == Some(key.as_str()) {
            return;
        }
        self.shown = Some(key);
        let d = topic.pointer("/doc/display").cloned().unwrap_or(Value::Null);
        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let mut changes: Vec<(usize, String, Rgba)> = vec![
            (self.lines.place, s("place"), ink),
            (self.lines.temp, s("temp"), ink),
            (self.lines.cond, s("cond"), ink),
            (self.lines.stats, String::new(), ink2),
        ];
        if self.lines.stats_shown
            && let Some(st) = self.lines.stats_style
        {
            let stats = if self.mode == "stack" { s("stats") } else { s("stats").replace(" \u{b7} ", "\n").lines().take(1).collect() };
            changes[3].1 = ellipsise(&mut kit.ui.fonts, &stats, st.ty, self.lines.stats_w);
        }
        let days: Vec<Value> = d.get("days").and_then(Value::as_array).cloned().unwrap_or_default();
        let deg = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).and_then(|t| t.trim_end_matches('°').parse::<f32>().ok());
        let (mut lo_min, mut hi_max) = (f32::MAX, f32::MIN);
        for dv in &days {
            if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi")) {
                lo_min = lo_min.min(lo);
                hi_max = hi_max.max(hi);
            }
        }
        for (i, (di, a, b)) in self.lines.days.iter().enumerate() {
            match days.get(i) {
                Some(dv) => {
                    let ds = |k: &str| dv.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                    changes.push((*di, ds("day"), if self.mode == "columns" { ink2 } else { ink }));
                    if self.mode == "columns" {
                        changes.push((*a, ds("hi"), ink));
                        changes.push((*b, ds("lo"), ink2));
                    } else {
                        changes.push((*a, ds("lo"), ink2));
                        changes.push((*b, ds("hi"), ink));
                    }
                }
                None => {
                    changes.push((*di, String::new(), ink));
                    changes.push((*a, String::new(), ink));
                    changes.push((*b, String::new(), ink));
                }
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.block.set_many(kit, &refs);
        // the pictures: the now icon, and the rows' icons and range bars in one art layer
        let kind = s("nowKind");
        self.now_icon.clear();
        if !kind.is_empty()
            && let Some(p) = pictures.get(&format!("wxbig-{kind}"))
        {
            let side = self.now_side;
            let fitted = p.fitted(side, side, Fit::Contain);
            fitted.paint(&mut self.now_icon);
        }
        self.now_icon.damage(None);
        let (w, h) = (self.w as u32, self.h as u32);
        self.rows_art.clear();
        {
            let light = kit.mode == ui::lucent::palette::Mode::Light;
            let accent = kit.ui.look.accent();
            let track = faded(ink, 0.12);
            let mut canvas = Canvas { px: self.rows_art.pixels_mut(), w, h };
            if self.mode == "stack" && !days.is_empty() {
                canvas.rounded(Frame::new(0.0, self.lines.row_top - 5.0, self.w, 1.0), 0.0, faded(ink, 0.16));
            }
            for (i, dv) in days.iter().enumerate() {
                let k = dv.get("kind").and_then(Value::as_str).unwrap_or("");
                if self.mode == "columns" {
                    if i >= 7 {
                        break;
                    }
                    let x = self.lines.col_x + i as f32 * self.lines.col_w;
                    let side = (64.0 * kit.text_scale.max(1.0)) as u32;
                    if let Some(p) = pictures.get(&format!("wxbig-{k}")) {
                        canvas.picture(p, (x + (self.lines.col_w - side as f32) / 2.0).round() as i32, (34.0 * kit.text_scale.max(1.0)) as i32, side, side);
                    }
                } else {
                    if i >= self.lines.days.len() {
                        break;
                    }
                    let y = self.lines.row_top + i as f32 * self.lines.row_h;
                    let icon = self.lines.icon as u32;
                    let name = if self.mode == "stack" { format!("wx-{k}_{}", if light { "l" } else { "d" }) } else { format!("wxbig-{k}") };
                    if let Some(p) = pictures.get(&name) {
                        canvas.picture(p, (self.lines.col_x + self.lines.day_w) as i32, (y + (self.lines.row_h - icon as f32) / 2.0).round() as i32, icon, icon);
                    }
                    // the range bar: the day's span of the week's
                    let bx = self.lines.col_x + self.lines.day_w + self.lines.icon + 10.0 + self.lines.num_w + 14.0;
                    let bw = (self.w - bx - self.lines.num_w - 14.0).max(0.0);
                    let by = y + (self.lines.row_h - 8.0) / 2.0;
                    canvas.rounded(Frame::new(bx, by, bw, 8.0), 4.0, track);
                    if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi"))
                        && hi_max > lo_min
                    {
                        let f0 = (lo - lo_min) / (hi_max - lo_min);
                        let f1 = (hi - lo_min) / (hi_max - lo_min);
                        canvas.rounded(Frame::new(bx + bw * f0, by, (bw * (f1 - f0)).max(8.0), 8.0), 4.0, accent);
                    }
                }
            }
        }
        self.rows_art.damage(None);
    }
}

// ---- the news
pub struct News {
    pub block: Words,
    w: f32,
    lines: NewsLines,
    at: Props,
    pub index: usize,
    shown: Option<(String, usize)>,
    title_style: Style,
    summary_style: Style,
    src_style: Style,
    src_w: f32,
}

struct NewsLines {
    tag: usize,
    src: usize,
    title: Vec<usize>,
    summary: Vec<usize>,
    counter: usize,
}

impl News {
    fn new(kit: &mut Kit, w: f32, h: f32, z: i32) -> std::io::Result<Self> {
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let accent = kit.ui.look.accent();
        let tag_s = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), accent, Align::Left);
        let src_s = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let title_s = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
        let summary_s = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let cap = kit.line(TypeRole::Caption);
        let body = kit.line(TypeRole::Body);
        let mut lines: Vec<Line> = Vec::new();
        let tag_w = (110.0 * kit.text_scale).round();
        let tag = lines.len();
        lines.push((Frame::new(0.0, 0.0, tag_w, cap), tag_s, String::new()));
        let src = lines.len();
        lines.push((Frame::new(tag_w, 0.0, w - tag_w, cap), src_s, String::new()));
        let mut y = cap + 12.0;
        let title_rows = if h >= 400.0 { 3 } else if h >= 200.0 { 2 } else { 1 };
        let mut title = Vec::new();
        for _ in 0..title_rows {
            title.push(lines.len());
            lines.push((Frame::new(0.0, y, w, body), title_s, String::new()));
            y += body;
        }
        y += 10.0;
        let mut summary = Vec::new();
        let rows = (((h - y - cap) / cap).floor().max(0.0) as usize).min(8);
        for _ in 0..rows {
            summary.push(lines.len());
            lines.push((Frame::new(0.0, y, w, cap), summary_s, String::new()));
            y += cap;
        }
        let counter = lines.len();
        lines.push((Frame::new(0.0, h - cap, w, cap), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), String::new()));
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        Ok(Self { block, w, lines: NewsLines { tag, src, title, summary, counter }, at: HIDDEN, index: 0, shown: None, title_style: title_s, summary_style: summary_s, src_style: src_s, src_w: w - tag_w })
    }

    fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
    }

    /// Story `index` of the topic.
    pub fn set(&mut self, kit: &mut Kit, topic: &Value, index: usize, now_ms: u64) {
        let items: Vec<Value> = topic.pointer("/doc/items").and_then(Value::as_array).cloned().unwrap_or_default();
        let index = if items.is_empty() { 0 } else { index % items.len() };
        let key = (topic.pointer("/doc/fetchedAt").map(Value::to_string).unwrap_or_default(), index);
        if self.shown.as_ref() == Some(&key) {
            return;
        }
        self.shown = Some(key);
        self.index = index;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let accent = kit.ui.look.accent();
        let mut changes: Vec<(usize, String, Rgba)> = Vec::new();
        match items.get(index) {
            Some(it) => {
                let s = |k: &str| it.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                changes.push((self.lines.tag, s("tag"), accent));
                let age = it.get("pubMs").and_then(Value::as_u64).map(|p| ago(now_ms.saturating_sub(p))).unwrap_or_default();
                let src = if age.is_empty() { s("src") } else { format!("{} \u{b7} {age}", s("src")) };
                changes.push((self.lines.src, ellipsise(&mut kit.ui.fonts, &src, self.src_style.ty, self.src_w), ink2));
                let title = wrap(&mut kit.ui.fonts, &s("title"), self.title_style.ty, self.w);
                for (i, li) in self.lines.title.iter().enumerate() {
                    let last = i + 1 == self.lines.title.len();
                    let text = match title.get(i) {
                        Some(_) if last && title.len() > self.lines.title.len() => ellipsise(&mut kit.ui.fonts, &title[i..].join(" "), self.title_style.ty, self.w),
                        Some(t) => t.clone(),
                        None => String::new(),
                    };
                    changes.push((*li, text, ink));
                }
                let summary = wrap(&mut kit.ui.fonts, &s("summary"), self.summary_style.ty, self.w);
                for (i, li) in self.lines.summary.iter().enumerate() {
                    let last = i + 1 == self.lines.summary.len();
                    let text = match summary.get(i) {
                        Some(_) if last && summary.len() > self.lines.summary.len() => ellipsise(&mut kit.ui.fonts, &summary[i..].join(" "), self.summary_style.ty, self.w),
                        Some(t) => t.clone(),
                        None => String::new(),
                    };
                    changes.push((*li, text, ink2));
                }
                changes.push((self.lines.counter, if items.len() > 1 { format!("{} of {}", index + 1, items.len()) } else { String::new() }, ink2));
            }
            None => {
                changes.push((self.lines.tag, "NEWS".into(), accent));
                changes.push((self.lines.src, "Nothing yet".into(), ink2));
                for li in self.lines.title.iter().chain(self.lines.summary.iter()).chain(std::iter::once(&self.lines.counter)) {
                    changes.push((*li, String::new(), ink));
                }
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.block.set_many(kit, &refs);
    }
}

/// "Just now", "12 min ago", "3 h ago", "Yesterday", "4 days ago".
pub fn ago(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        "Just now".into()
    } else if s < 90 * 60 {
        format!("{} min ago", s / 60)
    } else if s < 24 * 3600 {
        format!("{} h ago", s / 3600)
    } else if s < 48 * 3600 {
        "Yesterday".into()
    } else {
        format!("{} days ago", s / 86400)
    }
}

// ---- at home: the Home Assistant plugin's cards as rows
pub struct AtHome {
    pub block: Words,
    art: ImageLayer,
    w: f32,
    rows: Vec<(usize, usize)>,
    title: usize,
    row_h: f32,
    row_top: f32,
    at: Props,
    shown: Option<(String, Option<usize>)>,
}

const ROW_H: f32 = 76.0;

impl AtHome {
    fn new(kit: &mut Kit, w: f32, h: f32, z: i32, entry: &Value) -> std::io::Result<Self> {
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let cap = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let mut lines: Vec<Line> = Vec::new();
        let title = lines.len();
        let lh = kit.line(TypeRole::Label);
        lines.push((Frame::new(0.0, 0.0, w, lh), label, entry.get("title").and_then(Value::as_str).unwrap_or("At home").to_owned()));
        let row_top = lh + 12.0;
        let row_h = ROW_H.max(kit.line(TypeRole::Label) + kit.line(TypeRole::Caption) + 12.0);
        let n = (((h - row_top) / row_h).floor().max(0.0) as usize).min(8);
        let mut rows = Vec::new();
        for i in 0..n {
            let y = row_top + i as f32 * row_h;
            let a = lines.len();
            lines.push((Frame::new(64.0, y + 6.0, w - 64.0, kit.line(TypeRole::Label)), label, String::new()));
            let b = lines.len();
            lines.push((Frame::new(64.0, y + 6.0 + kit.line(TypeRole::Label), w - 64.0, kit.line(TypeRole::Caption)), cap, String::new()));
            rows.push((a, b));
        }
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        let art = art(kit, Frame::new(0.0, 0.0, w, h), z + 1)?;
        Ok(Self { block, art, w, rows, title, row_h, row_top, at: HIDDEN, shown: None })
    }

    fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        self.art.set(at);
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The plugin's cards, `inner` the row the inner focus is on.
    pub fn set(&mut self, kit: &mut Kit, topic: &Value, inner: Option<usize>, pictures: &mut Pictures) {
        let items: Vec<Value> = topic.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        let key = (items.iter().map(|i| format!("{}:{}", i.get("id").and_then(Value::as_str).unwrap_or(""), i.get("revision").and_then(Value::as_u64).unwrap_or(0))).collect::<Vec<_>>().join("|"), inner);
        if self.shown.as_ref() == Some(&key) {
            return;
        }
        self.shown = Some(key);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let look = &kit.ui.look;
        let role_colour = |r: &str| match r {
            "warning" => look.colour(ui::lucent::palette::PaletteRole::Warning),
            "danger" => look.colour(ui::lucent::palette::PaletteRole::Danger),
            "positive" => look.colour(ui::lucent::palette::PaletteRole::Positive),
            "info" => look.colour(ui::lucent::palette::PaletteRole::Info),
            _ => look.accent(),
        };
        let mut changes: Vec<(usize, String, Rgba)> = Vec::new();
        let mut tiles: Vec<(f32, Rgba, String)> = Vec::new();
        for (i, (a, b)) in self.rows.iter().enumerate() {
            match items.get(i) {
                Some(it) => {
                    let c = it.get("card").and_then(|c| c.get("content")).cloned().unwrap_or(Value::Null);
                    let title = c.get("title").and_then(Value::as_str).unwrap_or("").to_owned();
                    let fact = c.pointer("/facts/0").map(|f| format!("{} {}", f.get("label").and_then(Value::as_str).unwrap_or(""), f.get("value").and_then(Value::as_str).unwrap_or(""))).unwrap_or_default();
                    let focused = inner == Some(i);
                    changes.push((*a, title, ink));
                    changes.push((*b, fact.trim().to_owned(), if focused { ink } else { ink2 }));
                    let role = c.get("role").and_then(Value::as_str).unwrap_or("accent");
                    let icon = c.get("icon").and_then(Value::as_str).unwrap_or("").rsplit('/').next().unwrap_or("").to_owned();
                    tiles.push((self.row_top + i as f32 * self.row_h, role_colour(role), icon));
                }
                None => {
                    changes.push((*a, String::new(), ink));
                    changes.push((*b, String::new(), ink2));
                }
            }
        }
        if items.is_empty() {
            if let Some((a, b)) = self.rows.first() {
                changes.push((*a, "Nothing to show".into(), ink2));
                changes.push((*b, "The Home Assistant plugin has no cards".into(), ink2));
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.block.set_many(kit, &refs);
        let _ = self.title;
        let (w, h) = (self.art.width(), self.art.height());
        let radius = kit.tokens.radius(lumatv_tokens::Radius::Control);
        let focus_ring = faded(ink, 0.14);
        let mut icons: Vec<(f32, Rgba, Option<Vec<u8>>)> = Vec::new();
        for (y, colour, icon) in &tiles {
            let px = if icon.is_empty() { None } else { kit.icons.draw(icon, 28, *colour).or_else(|| pictures.get(&format!("base/{icon}")).map(|p| p.fitted(28, 28, Fit::Contain).rgba)) };
            icons.push((*y, *colour, px));
        }
        self.art.clear();
        {
            let mut canvas = Canvas { px: self.art.pixels_mut(), w, h };
            for (i, (y, colour, px)) in icons.iter().enumerate() {
                if inner == Some(i) {
                    canvas.rounded(Frame::new(-8.0, *y, self.w + 16.0, self.row_h - 4.0), radius, focus_ring);
                }
                canvas.rounded(Frame::new(0.0, y + (self.row_h - 4.0 - 48.0) / 2.0, 48.0, 48.0), 14.0, faded(*colour, 0.22));
                if let Some(px) = px {
                    let pic = Picture { width: 28, height: 28, rgba: px.clone() };
                    canvas.picture(&pic, 10, (y + (self.row_h - 4.0 - 28.0) / 2.0).round() as i32, 28, 28);
                } else {
                    canvas.disc(24.0, y + (self.row_h - 4.0) / 2.0, 8.0, *colour);
                }
            }
        }
        self.art.damage(None);
    }
}

// ---- the cameras
pub struct Cameras {
    pub block: Words,
    art: ImageLayer,
    w: f32,
    rows: Vec<(usize, usize)>,
    head: (usize, usize),
    row_h: f32,
    row_top: f32,
    at: Props,
    shown: Option<String>,
}

impl Cameras {
    fn new(kit: &mut Kit, w: f32, h: f32, z: i32) -> std::io::Result<Self> {
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let cap = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let cap_r = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
        let lh = kit.line(TypeRole::Label);
        let mut lines: Vec<Line> = vec![(Frame::new(0.0, 0.0, w * 0.5, lh), label, "Cameras".into()), (Frame::new(w * 0.5, 0.0, w * 0.5, lh), cap_r, String::new())];
        let row_top = lh + 12.0;
        let row_h = (64.0 * kit.text_scale).max(kit.line(TypeRole::Label) + 8.0);
        let n = (((h - row_top) / row_h).floor().max(0.0) as usize).min(8);
        let mut rows = Vec::new();
        for i in 0..n {
            let y = row_top + i as f32 * row_h;
            let a = lines.len();
            lines.push((Frame::new(28.0, y, w * 0.6 - 28.0, row_h), label, String::new()));
            let b = lines.len();
            lines.push((Frame::new(w * 0.6, y, w * 0.4, row_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), String::new()));
            rows.push((a, b));
        }
        let _ = cap;
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        let art = art(kit, Frame::new(0.0, 0.0, w, h), z + 1)?;
        Ok(Self { block, art, w, rows, head: (0, 1), row_h, row_top, at: HIDDEN, shown: None })
    }

    fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        self.art.set(at);
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The link's state in words (`cameras-ui-spec.md`), never an address.
    pub fn state_words(state: &str, n: usize) -> String {
        match state {
            "unlinked" => "Scrypted isn't linked".into(),
            "connecting" => "Connecting to Scrypted".into(),
            "refused" => "Scrypted refused this TV".into(),
            "untrusted" => "Can't verify Scrypted".into(),
            "offline" => "Can't reach Scrypted".into(),
            _ if n == 0 => "No cameras shared".into(),
            _ => format!("{n} camera{}", if n == 1 { "" } else { "s" }),
        }
    }

    pub fn set(&mut self, kit: &mut Kit, topic: &Value, inner: Option<usize>, now_ms: u64) {
        let cams: Vec<Value> = topic.get("cameras").and_then(Value::as_array).cloned().unwrap_or_default();
        let state = topic.get("state").and_then(Value::as_str).unwrap_or("unlinked").to_owned();
        let key = format!("{state}|{:?}|{}", inner, cams.iter().map(|c| format!("{}:{}", c.get("id").and_then(Value::as_str).unwrap_or(""), c.get("online").and_then(Value::as_bool).unwrap_or(false))).collect::<Vec<_>>().join(","));
        if self.shown.as_deref() == Some(key.as_str()) {
            return;
        }
        self.shown = Some(key);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let mut changes: Vec<(usize, String, Rgba)> = vec![(self.head.1, Self::state_words(&state, cams.len()), ink2)];
        let mut dots: Vec<(f32, bool)> = Vec::new();
        for (i, (a, b)) in self.rows.iter().enumerate() {
            match cams.get(i) {
                Some(c) => {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
                    let online = c.get("online").and_then(Value::as_bool).unwrap_or(false);
                    let since = c.get("since").and_then(Value::as_u64).map(|s| ago(now_ms.saturating_sub(s))).unwrap_or_default();
                    let words = if online { c.get("room").and_then(Value::as_str).unwrap_or("").to_owned() } else if since.is_empty() { "Offline".into() } else { format!("Offline \u{b7} {since}") };
                    changes.push((*a, name, ink));
                    changes.push((*b, words, ink2));
                    dots.push((self.row_top + i as f32 * self.row_h, online));
                }
                None => {
                    changes.push((*a, String::new(), ink));
                    changes.push((*b, String::new(), ink2));
                }
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.block.set_many(kit, &refs);
        let live = kit.ui.look.colour(ui::lucent::palette::PaletteRole::Live);
        let positive = kit.ui.look.colour(ui::lucent::palette::PaletteRole::Positive);
        let radius = kit.tokens.radius(lumatv_tokens::Radius::Control);
        let ring = faded(ink, 0.14);
        let (w, h) = (self.art.width(), self.art.height());
        self.art.clear();
        {
            let mut canvas = Canvas { px: self.art.pixels_mut(), w, h };
            for (i, (y, online)) in dots.iter().enumerate() {
                if inner == Some(i) {
                    canvas.rounded(Frame::new(-8.0, *y, self.w + 16.0, self.row_h - 4.0), radius, ring);
                }
                canvas.disc(8.0, y + (self.row_h - 4.0) / 2.0, 6.0, if *online { positive } else { faded(live, 0.7) });
            }
        }
        self.art.damage(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_plans_the_mocks_boxes_and_the_words_age() {
        let p = plan(520.0, 166.0, true, true, 92.0, true);
        assert_eq!((p.mode, p.dia), ("regular", 166.0));
        assert!(p.strip.is_some() && p.info.is_some());
        let d = plan(520.0, 166.0, false, true, 92.0, true);
        assert_eq!((d.mode, d.ts, d.greet), ("regular", 92.0, true));
        assert_eq!(plan(200.0, 100.0, false, true, 92.0, true).mode, "tiny");
        assert_eq!(plan(400.0, 400.0, false, true, 92.0, true).mode, "column");
        assert_eq!(plan(900.0, 300.0, true, true, 92.0, true).mode, "month");
        assert_eq!(plan(1200.0, 180.0, false, true, 92.0, true).mode, "zoned");
        assert_eq!(ago(30_000), "Just now");
        assert_eq!(ago(12 * 60_000), "12 min ago");
        assert_eq!(ago(3 * 3_600_000), "3 h ago");
        assert_eq!(ago(30 * 3_600_000), "Yesterday");
        assert_eq!(ago(4 * 86_400_000), "4 days ago");
        assert_eq!(Cameras::state_words("ready", 2), "2 cameras");
        assert_eq!(Cameras::state_words("offline", 2), "Can't reach Scrypted");
    }

    #[test]
    fn the_canvas_paints_inside_its_bounds() {
        let mut px = vec![0u8; 40 * 20 * 4];
        let mut c = Canvas { px: &mut px, w: 40, h: 20 };
        c.rounded(Frame::new(-10.0, 5.0, 30.0, 10.0), 4.0, Rgba(255, 0, 0, 255));
        c.disc(38.0, 18.0, 5.0, Rgba(0, 255, 0, 255));
        assert!(px[(10 * 40 + 5) * 4] > 200);
        assert_eq!(px[(2 * 40 + 5) * 4], 0);
        assert!(px[(18 * 40 + 38) * 4 + 1] > 200);
    }
}
