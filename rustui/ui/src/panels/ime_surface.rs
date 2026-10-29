//! The apps' keyboard drawn (`panels/Keyboard.qml`): a sheet along the foot of the screen, full width for the letters and a narrow one for a
//! number pad, with the keys on it as plates, the one focus plate that moves, and the words and pictures of the keys over it. The look is the
//! one of LG's keys in the Lucent restyle: an opaque sheet of the material's glass, key plates one hairline wash above it, the function keys
//! one wash below, the focus a lit plate with a ring in the ink, the accent on the enter key only. Nothing typed is ever drawn.
//!
//! Cost: the plates and the pictures of the keys are one picture painted when the layout says something else (the letters, the symbols, the
//! enter key and the voice key change a few plates), the words of all the keys are one block whose lines change in place (Shift, the marks, the
//! words on the space bar), and the focus is three plates (a key, the accent key, the space bar) of which one is shown and glides. A layout of
//! another size (a number pad) makes the sheet again; a field that comes back to the same layout makes nothing.

use super::ime::{Kind, Layout, LayoutId, View};
use super::life::Phase;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, HIDDEN};
use crate::home::widgets::Canvas;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::picture::Picture;
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::edge;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Block, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::{Mode, PaletteRole};
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(209);
const REGION: Region = Region(2700);
const BASE: u32 = 0x2700_0000;
const SCREEN_W: f32 = 1920.0;
const SCREEN_H: f32 = 1080.0;
/// The air under the keys: Lucent's safe area at the foot.
const BOTTOM_PAD: f32 = 54.0;
/// The air of a function word to the key's sides (a larger type shrinks into the key, never to its edge).
const WORD_AIR: f32 = 8.0;

pub fn key_id(i: usize) -> Id {
    Id(BASE + i as u32)
}

pub fn key_of(id: Id) -> Option<usize> {
    let i = id.0.checked_sub(BASE)? as usize;
    (i < 64).then_some(i)
}

/// The colours of the sheet as the restyle of LG's keys computes them: opaque roles of the material.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tones {
    sheet: Rgba,
    tile: Rgba,
    tile_edge: Rgba,
    func: Rgba,
    /// The wash the focus plate lays over a key (premultiplied): the plate under it comes out as the lit role.
    lit: Rgba,
}

/// `a` with `wash` (white or black) laid over it at `alpha`, opaque.
fn over(a: Rgba, wash: u8, alpha: f32) -> Rgba {
    let m = |c: u8| (f32::from(wash) * alpha + f32::from(c) * (1.0 - alpha)).round() as u8;
    Rgba(m(a.0), m(a.1), m(a.2), 255)
}

fn tones(kit: &Kit) -> Tones {
    let g = kit.ui.look.colour(PaletteRole::Glass);
    // an opaque glass: the colour is the sheet's, whatever the material lets through
    let un = |c: u8| if g.3 == 0 || g.3 == 255 { c } else { (f32::from(c) * 255.0 / f32::from(g.3)).round().min(255.0) as u8 };
    let sheet = Rgba(un(g.0), un(g.1), un(g.2), 255);
    let light = kit.mode == Mode::Light;
    let wash = if light { 0 } else { 255 };
    let hair = if light { 0.12 } else { (f32::from(edge(kit).3) / 255.0).clamp(0.10, 0.22) };
    let tile = over(sheet, wash, hair);
    let lit = if light { 0.22 } else { 0.28 };
    // the plate under the focus is a tile: the wash that takes it to the lit colour
    let a = ((lit - hair) / (1.0 - hair)).clamp(0.0, 1.0);
    let k = (f32::from(wash) * a).round() as u8;
    Tones {
        sheet,
        tile,
        tile_edge: over(tile, wash, hair),
        func: mix_over(sheet, over(sheet, wash, 0.10), 0.35),
        lit: Rgba(k, k, k, (a * 255.0).round() as u8),
    }
}

/// `b` laid over `a` at `alpha`, both opaque.
fn mix_over(a: Rgba, b: Rgba, alpha: f32) -> Rgba {
    let m = |x: u8, y: u8| (f32::from(y) * alpha + f32::from(x) * (1.0 - alpha)).round() as u8;
    Rgba(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2), 255)
}

/// Where everything is.
#[derive(Clone, Debug, PartialEq)]
struct Geometry {
    padded: bool,
    scale: f32,
    unit: f32,
    key_h: f32,
    gap: f32,
    /// The sheet as it shows on the screen (the compositor is told this one).
    sheet: Frame,
    /// The sheet as it is drawn: its sides and its bottom corners are off the screen.
    face: Frame,
    grid: Frame,
    keys: Vec<Frame>,
}

fn geometry(kit: &Kit, layout: &Layout) -> Geometry {
    let padded = layout.pad;
    let scale = kit.text_scale.clamp(1.0, 1.5);
    let (unit, key_h, gap) = if padded {
        ((176.0 * scale).round().min(220.0), (84.0 * scale).round(), 12.0)
    } else {
        ((104.0 * scale).round().min(134.0), (68.0 * scale).round(), 10.0)
    };
    let (grid_w, grid_h) = (layout.cols as f32 * unit + (layout.cols - 1) as f32 * gap, layout.rows as f32 * key_h + (layout.rows - 1) as f32 * gap);
    let top = kit.space(5);
    let sheet_w = if padded { grid_w + 128.0 } else { SCREEN_W };
    let sheet_h = (top + grid_h + BOTTOM_PAD).ceil();
    let sheet = Frame::new(((SCREEN_W - sheet_w) / 2.0).round(), SCREEN_H - sheet_h, sheet_w, sheet_h);
    let radius = kit.tokens.radius(Radius::Sheet);
    let face =
        if padded { Frame::new(sheet.x, sheet.y, sheet.w, sheet.h + radius) } else { Frame::new(sheet.x - 4.0, sheet.y, sheet.w + 8.0, sheet.h + radius) };
    let grid = Frame::new(sheet.x + ((sheet.w - grid_w) / 2.0).round(), sheet.y + top, grid_w, grid_h);
    let keys = layout
        .keys
        .iter()
        .map(|k| {
            Frame::new(grid.x + k.col as f32 * (unit + gap), grid.y + k.row as f32 * (key_h + gap), k.span as f32 * unit + (k.span - 1) as f32 * gap, key_h)
        })
        .collect();
    Geometry { padded, scale, unit, key_h, gap, sheet, face, grid, keys }
}

/// How a key is lit: the class of its focus plate.
fn class_of(layout: &Layout, i: usize) -> usize {
    match layout.keys.get(i) {
        Some(k) if k.kind == Kind::Space => 2,
        Some(k) if k.accent => 1,
        _ => 0,
    }
}

struct Lit {
    shapes: [ShapeLayer; 3],
    at: Option<(usize, usize)>,
}

impl Lit {
    fn new(kit: &Kit, tones: &Tones, geo: &Geometry, layout: &Layout, z: i32) -> Self {
        let radius = kit.tokens.radius(Radius::Row);
        let ink = kit.ui.look.ink();
        let space = layout.keys.iter().position(|k| k.kind == Kind::Space).map_or((geo.unit, geo.key_h), |i| (geo.keys[i].w, geo.keys[i].h));
        let make = |fill: Rgba, w: f32, h: f32| {
            let s =
                plate::shape(&kit.ui, Frame::new(0.0, 0.0, w, h), z, Shape { corner_radius: radius, fill: Fill::Solid(fill), border_width: 3.0, border: ink });
            s.set(HIDDEN);
            s
        };
        // the accent key keeps its fill: only its ring is the focus
        Self { shapes: [make(tones.lit, geo.unit, geo.key_h), make(Rgba(0, 0, 0, 0), geo.unit, geo.key_h), make(tones.lit, space.0, space.1)], at: None }
    }

    /// The ring goes to key `i`: it glides from a key of its own class, and is there at once from another's.
    fn go(&mut self, kit: &Kit, geo: &Geometry, layout: &Layout, i: usize) {
        let class = class_of(layout, i);
        let to = Props::at(geo.keys[i].x, geo.keys[i].y);
        match self.at {
            Some((c, _)) if c == class => kit.glide(&self.shapes[class], to, Beat::Focus),
            _ => {
                for (c, s) in self.shapes.iter().enumerate() {
                    s.set(if c == class { to } else { HIDDEN });
                }
            }
        }
        self.at = Some((class, i));
    }
}

struct Built {
    sig: String,
    /// What the plates and pictures were painted for (the layout, the enter key, the voice key).
    paint: String,
    geo: Geometry,
    _face: ShapeLayer,
    art: ImageLayer,
    caps: Block,
    caps_now: Vec<(String, Rgba)>,
    /// Which keys' words are characters (the others are function words, a size smaller): a layout that changes it makes the words again.
    classes: Vec<bool>,
    lit: Lit,
    /// The dot and the bar of Shift's state, and the short line of the space bar.
    dot: ShapeLayer,
    bar: ShapeLayer,
    line: ShapeLayer,
    layout: Layout,
    /// Which keys take a press (the cue skips the ones that do not).
    enabled: Vec<bool>,
}

pub struct ImeSurface {
    stage: Stage,
    built: Option<Built>,
}

/// What the pointer did on the keys in one sample: the key a press began on, and the key a click ended on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointed {
    pub down: Option<usize>,
    pub click: Option<usize>,
}

impl ImeSurface {
    /// `z` is the keyboard's band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, layout: &Layout) -> String {
        // every full layout has the same grid; a pad has its own (the digits' pad has no corners)
        let shape = if layout.pad { layout.id.name() } else { "full" };
        format!("{}|{}|{shape}", kit.generation(), kit.text_scale)
    }

    fn paint_signature(v: &View) -> String {
        let enter = v.layout.keys.iter().find(|k| k.kind == Kind::Enter).map_or(String::new(), |k| format!("{}{}", k.icon.unwrap_or(""), k.label));
        let voice = v.layout.keys.iter().any(|k| k.kind == Kind::Voice);
        format!("{}|{enter}|{voice}", v.layout.id.name())
    }

    /// The text a key's block line holds: its character, its word, or what the microphone is doing on the space bar.
    fn cap_text(v: &View, i: usize) -> String {
        let k = &v.layout.keys[i];
        match k.kind {
            Kind::Space => v.note.clone(),
            _ if k.icon.is_some() => String::new(),
            _ => v.layout.label(i, v.shift).to_string(),
        }
    }

    fn cap_ink(kit: &Kit, v: &View, i: usize) -> Rgba {
        let k = &v.layout.keys[i];
        if k.accent {
            kit.ui.look.accent_ink()
        } else if k.needs_text && !v.has_text {
            crate::kit::button::faded(kit.ui.look.ink(), kit.tokens.states.disabled_opacity)
        } else {
            kit.ui.look.ink()
        }
    }

    /// The style of key `i`'s words: a character in the title's size (a pad's larger), a function word in the label's, shrunk until it fits
    /// the key with its air.
    fn cap_style(kit: &mut Kit, geo: &Geometry, k: &super::ime::Key, text: &str, ink: Rgba) -> Style {
        let role = if k.kind == Kind::Char { TypeRole::Title } else { TypeRole::Label };
        let mut style = role_style(kit, role, Some(Weight::Medium), ink, Align::Centre);
        if k.kind == Kind::Char {
            if geo.padded {
                style.ty.px = (style.ty.px as f32 * 1.2).round() as u32;
            }
        }
        // the line keeps its air above and below in the key: a large text size gives way before the key does
        let room_h = geo.key_h - 2.0 * kit.space(2);
        while style.ty.px > 12 && ui::components::line_height(style.ty) > room_h {
            style.ty.px -= 1;
        }
        if k.kind != Kind::Char {
            let span = k.span as f32;
            let room = span * geo.unit + (span - 1.0) * geo.gap - 2.0 * WORD_AIR;
            while style.ty.px > 12 && width_of(kit, style, text) > room {
                style.ty.px -= 1;
            }
        }
        style
    }

    /// The words of the keys as one block: each key's line in the style of its class, with what it says now.
    fn make_caps(kit: &mut Kit, z: i32, geo: &Geometry, v: &View) -> std::io::Result<(Block, Vec<(String, Rgba)>, Vec<bool>)> {
        let mut now = Vec::new();
        let mut lines = Vec::new();
        for (i, k) in v.layout.keys.iter().enumerate() {
            let text = Self::cap_text(v, i);
            let colour = Self::cap_ink(kit, v, i);
            let style = Self::cap_style(kit, geo, k, &text, colour);
            let f = geo.keys[i];
            let air = if k.kind == Kind::Char { 0.0 } else { WORD_AIR };
            lines.push((Frame::new(f.x - geo.grid.x + air, f.y - geo.grid.y, f.w - 2.0 * air, f.h), style, text.clone()));
            now.push((text, colour));
        }
        let classes = v.layout.keys.iter().map(|k| k.kind == Kind::Char).collect();
        Ok((Block::new(&mut kit.ui, geo.grid, z + 6, lines)?, now, classes))
    }

    fn paint_plates(kit: &mut Kit, art: &mut ImageLayer, t: &Tones, geo: &Geometry, v: &View) {
        let radius = kit.tokens.radius(Radius::Row);
        let (ink, accent, accent_ink) = (kit.ui.look.ink(), kit.ui.look.accent(), kit.ui.look.accent_ink());
        let side = (32.0 * geo.scale).round() as u32;
        let (w, h) = (art.width(), art.height());
        art.pixels_mut().fill(0);
        let mut canvas = Canvas { px: art.pixels_mut(), w, h };
        let mut pics: Vec<(&'static str, bool, Option<Picture>)> = Vec::new();
        for (i, k) in v.layout.keys.iter().enumerate() {
            let f = geo.keys[i];
            let local = Frame::new(f.x - geo.grid.x, f.y - geo.grid.y, f.w, f.h);
            let (fill, rim) = if k.accent {
                (accent, accent)
            } else if k.fn_key {
                (t.func, t.tile)
            } else {
                (t.tile, t.tile_edge)
            };
            canvas.rounded(local, radius, rim);
            canvas.rounded(Frame::new(local.x + 1.0, local.y + 1.0, local.w - 2.0, local.h - 2.0), (radius - 1.0).max(0.0), fill);
            if let Some(name) = k.icon {
                let on_accent = k.accent;
                if !pics.iter().any(|(n, a, _)| *n == name && *a == on_accent) {
                    let pic = kit.icons.draw(name, side, if on_accent { accent_ink } else { ink }).map(|rgba| Picture { width: side, height: side, rgba });
                    pics.push((name, on_accent, pic));
                }
                if let Some((_, _, Some(p))) = pics.iter().find(|(n, a, _)| *n == name && *a == on_accent) {
                    canvas.picture(
                        p,
                        (local.x + (local.w - side as f32) / 2.0).round() as i32,
                        (local.y + (local.h - side as f32) / 2.0).round() as i32,
                        side,
                        side,
                    );
                }
            }
        }
        art.damage(None);
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &ui::group::Group, z: i32, v: &View, geo: Geometry, sig: String) -> std::io::Result<Built> {
        let scene = kit.ui.scene.clone();
        let t = tones(kit);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        scene
            .adopting_view(root, || -> std::io::Result<Built> {
                let face_look = Shape { corner_radius: kit.tokens.radius(Radius::Sheet), fill: Fill::Solid(t.sheet), border_width: 1.0, border: edge(kit) };
                let face = plate::shape(&kit.ui, geo.face, z + 1, face_look);
                let mut art = kit.ui.scene.image(geo.grid.w as u32, geo.grid.h as u32, z + 2, Props::at(geo.grid.x, geo.grid.y))?;
                Self::paint_plates(kit, &mut art, &t, &geo, v);
                let lit = Lit::new(kit, &t, &geo, &v.layout, z + 4);
                let (caps, caps_now, classes) = Self::make_caps(kit, z, &geo, v)?;
                let mark = |w: f32, h: f32, colour: Rgba, r: f32| {
                    let s = plate::shape(&kit.ui, Frame::new(0.0, 0.0, w, h), z + 7, plate::solid(colour, r));
                    s.set(HIDDEN);
                    s
                };
                let dot = mark(10.0, 10.0, ink, 5.0);
                let bar = mark(36.0, 4.0, ink, 2.0);
                let line = mark(96.0, 4.0, ink2, 2.0);
                let enabled = (0..v.layout.keys.len()).map(|i| !v.layout.keys[i].needs_text || v.has_text).collect();
                Ok(Built {
                    sig,
                    paint: Self::paint_signature(v),
                    geo: geo.clone(),
                    _face: face,
                    art,
                    caps,
                    caps_now,
                    classes,
                    lit,
                    dot,
                    bar,
                    line,
                    layout: v.layout.clone(),
                    enabled,
                })
            })
            .map_err(|e| std::io::Error::other(format!("{e}")))
            .inspect(|b| {
                for (i, f) in b.geo.keys.iter().enumerate() {
                    engine.focus.register(key_id(i), *f, REGION, SCOPE);
                }
            })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for i in 0..b.geo.keys.len() {
                engine.focus.unregister(key_id(i));
            }
        }
    }

    /// Fits the sheet to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let sig = Self::signature(kit, &v.layout);
        let geo = geometry(kit, &v.layout);
        let arrive = Arrive::Bottom(geo.sheet.h + 64.0);
        if self.stage.needs_root(arrive) || self.built.as_ref().is_some_and(|b| b.sig != sig) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, arrive, false)?;
        if self.built.is_none() {
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), v, geo, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the keyboard could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if let Some(b) = self.built.as_mut() {
            Self::update(kit, self.stage.group(), self.stage.z(), b, v);
        }
        if self.stage.phase(kit, engine, phase) {
            self.stage.focus(engine, Some(key_id(v.focus)), now_ns);
        }
        Ok(true)
    }

    /// What changes in place: the plates of another layout, the words, the dots of Shift and the line of the space bar, and the ring.
    fn update(kit: &mut Kit, root: &ui::group::Group, z: i32, b: &mut Built, v: &View) {
        if v.layout != b.layout {
            let paint = Self::paint_signature(v);
            if paint != b.paint {
                let t = tones(kit);
                Self::paint_plates(kit, &mut b.art, &t, &b.geo, v);
                b.paint = paint;
            }
            b.layout = v.layout.clone();
            b.lit.at = None;
        }
        // a layout of the same grid has other words (the marks), other styles for them (a word is smaller than a character) and another focus
        let classes: Vec<bool> = v.layout.keys.iter().map(|k| k.kind == Kind::Char).collect();
        if classes != b.classes {
            let scene = kit.ui.scene.clone();
            let geo = b.geo.clone();
            if let Ok((caps, now, classes)) = scene.adopting_view(root, || Self::make_caps(kit, z, &geo, v)) {
                b.caps = caps;
                b.caps_now = now;
                b.classes = classes;
            }
        }
        let now: Vec<(String, Rgba)> = (0..v.layout.keys.len()).map(|i| (Self::cap_text(v, i), Self::cap_ink(kit, v, i))).collect();
        if now != b.caps_now {
            let changes: Vec<(usize, &str, Rgba)> = now.iter().enumerate().map(|(i, (t, c))| (i, t.as_str(), *c)).collect();
            b.caps.set_many(&mut kit.ui, &changes);
            b.caps_now = now;
        }
        b.enabled = v.layout.keys.iter().map(|k| !k.needs_text || v.has_text).collect();
        // Shift's state: a dot when one capital is waiting, a dot and a bar for caps lock (the ink, so it reads on every plate)
        let shift = if v.layout.shiftable { v.layout.index_of("shift") } else { None };
        match shift.map(|i| b.geo.keys[i]) {
            Some(r) if v.shift > 0 => {
                b.dot.set(Props::at(r.x + 12.0, r.y + 10.0));
                b.bar.set(if v.shift == 2 { Props::at(r.x + ((r.w - 36.0) / 2.0).round(), r.y + r.h - 16.0) } else { HIDDEN });
            }
            _ => {
                b.dot.set(HIDDEN);
                b.bar.set(HIDDEN);
            }
        }
        // the space bar: a short line, or what the microphone is doing (its words are the bar's cap)
        match v.layout.index_of("space").map(|i| b.geo.keys[i]) {
            Some(r) if v.note.is_empty() => b.line.set(Props::at(r.x + ((r.w - 96.0) / 2.0).round(), r.y + (r.h / 2.0).round() + 6.0)),
            _ => b.line.set(HIDDEN),
        }
        if b.lit.at.map(|(_, i)| i) != Some(v.focus) {
            b.lit.go(kit, &b.geo, &v.layout, v.focus.min(v.layout.keys.len() - 1));
        }
    }

    /// Where the sheet is on the screen, for the compositor (the rectangle it keeps the apps' fields clear of).
    pub fn sheet(&self) -> Option<(i32, i32, i32, i32)> {
        self.built.as_ref().map(|b| (b.geo.sheet.x.round() as i32, b.geo.sheet.y.round() as i32, b.geo.sheet.w.round() as i32, b.geo.sheet.h.round() as i32))
    }

    /// A key of the remote went down: the pointer's cue goes (and does not come back until the pointer moves).
    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it over the keys (not the ones that take no press), a press moves the focus and a click is
    /// the key it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Pointed {
        let radius = kit.tokens.radius(Radius::Row);
        let enabled = self.built.as_ref().map(|b| b.enabled.clone()).unwrap_or_default();
        let mut down = None;
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(key_of).filter(|i| enabled.get(*i).copied().unwrap_or(false)).map(|_| radius),
            Touch::Press { id, down: true } => {
                down = key_of(id);
                None
            }
            Touch::Press { .. } => None,
        });
        Pointed { down, click: click.and_then(key_of) }
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    /// How many layers the keyboard holds.
    pub fn layers(&self) -> usize {
        // the face, the plates' picture, the words, the three focus plates, the dot, the bar and the space bar's line
        self.built.as_ref().map_or(0, |_| 1 + 1 + 1 + 3 + 3)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        json!({
            "sheet": f(b.geo.sheet), "grid": f(b.geo.grid), "scale": b.geo.scale, "padded": b.geo.padded, "layout": b.layout.id.name(),
            "keys": b.geo.keys.iter().map(|k| f(*k)).collect::<Vec<_>>(),
            "caps": b.caps_now.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
            "focus": engine.focus.current().and_then(key_of),
            "layers": self.layers(), "scope": SCOPE.0,
        })
    }

    /// Describes what is on the screen to a lint: the sheet and every cap on its key.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let t = tones(kit);
        let sheet = lint.panel("keyboard sheet", b.geo.sheet, t.sheet, false);
        let (accent, accent_ink) = (kit.ui.look.accent(), kit.ui.look.accent_ink());
        for (i, k) in b.layout.keys.iter().enumerate() {
            let f = b.geo.keys[i];
            let paint = if k.accent {
                accent
            } else if k.fn_key {
                t.func
            } else {
                t.tile
            };
            let key = lint.panel_on(&format!("key {i}"), f, paint, false, sheet);
            let (text, colour) = &b.caps_now[i];
            // a key that takes no press is dimmed by design
            if !text.is_empty() && !(k.needs_text && !b.enabled[i]) {
                let style = Self::cap_style(kit, &b.geo, k, text, if k.accent { accent_ink } else { *colour });
                let air = if k.kind == Kind::Char { 0.0 } else { WORD_AIR };
                lint.word(kit, &format!("cap {i}"), text, Frame::new(f.x + air, f.y, f.w - 2.0 * air, f.h), style, Some(key));
            }
        }
    }

    /// The layout the keyboard is drawn in (the harness reads it).
    pub fn layout_id(&self) -> Option<LayoutId> {
        self.built.as_ref().map(|b| b.layout.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::ime::{ENTER_DONE, ENTER_SEARCH, Ime, PURPOSE_DIGITS, PURPOSE_EMAIL, PURPOSE_NORMAL, PURPOSE_NUMBER, PURPOSE_PHONE};

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: ImeSurface,
        ime: Ime,
        t: u64,
    }

    fn field(purpose: u32, enter: u32, text: bool) -> Value {
        json!({"active": true, "show": true, "model": 3, "purpose": purpose, "hint": 0, "enter": enter, "max": 0, "text": text})
    }

    impl Up {
        fn new(mode: Mode, scale: f32, purpose: u32, enter: u32, text: bool) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut ime = Ime::new();
            ime.field_is(&field(purpose, enter, text), 0);
            let mut me = Self { _h: h, kit, engine, s: ImeSurface::new(crate::kit::z::KEYBOARD), ime, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.ime.view();
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.ime.life.phase(), self.t * 1_000_000).expect("the keyboard is drawn");
        }

        fn key(&mut self, name: &str) {
            self.ime.imekey(name, true, false, self.t);
            self.t += 400;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }

        fn geo(&self) -> Value {
            self.s.geometry(&self.engine)
        }

        fn key_centre(&self, i: usize) -> (f32, f32) {
            let k = &self.geo()["keys"][i];
            (k["x"].as_f64().unwrap() as f32 + k["w"].as_f64().unwrap() as f32 / 2.0, k["y"].as_f64().unwrap() as f32 + k["h"].as_f64().unwrap() as f32 / 2.0)
        }
    }

    fn at(x: f32, y: f32, b: u32) -> Sample {
        Sample { x, y, buttons: b, visible: true }
    }

    #[test]
    fn the_letters_sit_along_the_foot_full_width_with_twelve_columns_and_the_focus_on_g() {
        let u = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, 0, false);
        let g = u.geo();
        assert_eq!((g["sheet"]["x"].as_f64(), g["sheet"]["w"].as_f64()), (Some(0.0), Some(1920.0)));
        assert_eq!(g["sheet"]["y"].as_f64().unwrap() + g["sheet"]["h"].as_f64().unwrap(), 1080.0, "to the foot of the screen");
        let keys = g["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 51);
        assert_eq!(g["layout"], "letters");
        assert_eq!(g["caps"][29], "g", "row 2, column 5");
        assert_eq!(g["focus"], 29);
        // the keys end 54 above the foot
        let last = keys.iter().map(|k| k["y"].as_f64().unwrap() + k["h"].as_f64().unwrap()).fold(0.0, f64::max);
        assert!(last <= 1080.0 - 54.0 + 0.5, "the safe area at the foot: {last}");
        assert_eq!(u.s.sheet(), Some((0, g["sheet"]["y"].as_f64().unwrap() as i32, 1920, g["sheet"]["h"].as_f64().unwrap() as i32)));
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
    }

    #[test]
    fn a_number_pad_is_a_narrow_sheet_in_the_middle_with_larger_keys() {
        let u = Up::new(Mode::Dark, 1.0, PURPOSE_NUMBER, 0, false);
        let g = u.geo();
        assert_eq!(g["padded"], true);
        let w = g["sheet"]["w"].as_f64().unwrap();
        assert!(w < 1000.0 && (g["sheet"]["x"].as_f64().unwrap() * 2.0 + w - 1920.0).abs() < 2.0, "centred: {}", g["sheet"]);
        assert_eq!(g["keys"].as_array().unwrap().len(), 15);
        assert_eq!(g["caps"][4], "5", "the 5 has the focus");
        assert_eq!(g["focus"], 4);
    }

    #[test]
    fn the_focus_glides_and_typing_the_shift_and_the_marks_make_and_drop_no_layer() {
        let mut u = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, 0, true);
        let layers = u.kit.room().layers;
        u.key("right");
        assert_eq!(u.geo()["focus"], 30);
        u.key("ok");
        u.key("7");
        u.key("down");
        u.key("down");
        assert_eq!(u.geo()["focus"], 49, "the space bar holds the column");
        // shift: the caps change in place
        let shift = u.ime.layout().index_of("shift").unwrap();
        u.ime.click(shift, u.t);
        u.apply();
        assert_eq!(u.geo()["caps"][13], "Q");
        assert_eq!(u.kit.room().layers, layers, "no layer is made or dropped");
        // the marks: the characters are marks, the keys that were words or pictures are others (the words are made again, the count of layers holds)
        let mode = u.ime.layout().index_of("mode").unwrap();
        u.ime.click(mode, u.t);
        u.apply();
        assert_eq!(u.geo()["caps"][13], "!");
        assert_eq!(u.geo()["caps"][mode], "ABC");
        assert_eq!(u.kit.room().layers, layers);
        u.ime.click(mode, u.t);
        u.apply();
        assert_eq!(u.geo()["caps"][mode], "?123");
        assert_eq!(u.kit.room().layers, layers);
    }

    #[test]
    fn the_enter_key_says_what_the_field_wants_and_the_space_bar_says_what_the_microphone_does() {
        let mut u = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, ENTER_SEARCH, true);
        let enter = u.ime.layout().index_of("enter").unwrap();
        assert_eq!(u.geo()["caps"][enter], "Search");
        let go = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, ENTER_DONE, true);
        assert_eq!(go.geo()["caps"][enter], "Done");
        let plain = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, 0, true);
        assert_eq!(plain.geo()["caps"][enter], "", "a return key is a picture");
        let space = u.ime.layout().index_of("space").unwrap();
        assert_eq!(u.geo()["caps"][space], "");
        u.ime.start();
        u.ime.luna(crate::panels::ime::tags::PROBE, &json!({"configs": {"system.supportVoiceRecognition": true}}), 0);
        u.apply();
        let voice = u.ime.layout().index_of("voice").expect("the voice key stands where the hide key did");
        u.ime.click(voice, u.t);
        u.apply();
        assert_eq!(u.geo()["caps"][space], "Listening");
    }

    #[test]
    fn the_pointer_shows_a_cue_over_a_key_a_press_moves_the_focus_and_a_click_names_the_key() {
        let mut u = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, 0, true);
        let a = u.ime.layout().keys.iter().position(|k| k.text == "a").unwrap();
        let (x, y) = u.key_centre(a);
        let t = |n: u64| (3000 + n) * 1_000_000;
        let p = u.s.pointer(&u.kit, &mut u.engine, t(0), at(x, y, 0));
        assert_eq!(p, Pointed::default(), "a hover moves nothing");
        let p = u.s.pointer(&u.kit, &mut u.engine, t(10), at(x, y, 1));
        assert_eq!(p.down, Some(a));
        let p = u.s.pointer(&u.kit, &mut u.engine, t(20), at(x, y, 0));
        assert_eq!(p.click, Some(a));
    }

    #[test]
    fn a_key_that_takes_no_press_gets_no_cue_and_a_field_with_text_dims_nothing() {
        let mut u = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, 0, false);
        let clear = u.ime.layout().index_of("clear").unwrap();
        let (x, y) = u.key_centre(clear);
        u.s.pointer(&u.kit, &mut u.engine, 1_000_000, at(x, y, 0));
        assert!(!u.s.stage.hover.is_shown(), "no cue over Clear while the field is empty");
        u.ime.field_is(&field(PURPOSE_NORMAL, 0, true), 5);
        u.apply();
        let (ox, oy) = u.key_centre(clear - 1);
        u.s.pointer(&u.kit, &mut u.engine, 2_000_000, at(ox, oy, 0));
        u.s.pointer(&u.kit, &mut u.engine, 3_000_000, at(x, y, 0));
        assert!(u.s.stage.hover.is_shown(), "and one once it has text");
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_in_every_layout() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for (name, purpose, enter, voice) in [
                    ("letters", PURPOSE_NORMAL, 0, false),
                    ("search", PURPOSE_NORMAL, ENTER_SEARCH, true),
                    ("email", PURPOSE_EMAIL, ENTER_DONE, false),
                    ("number", PURPOSE_NUMBER, 0, false),
                    ("phone", PURPOSE_PHONE, ENTER_DONE, false),
                    ("digits", PURPOSE_DIGITS, ENTER_SEARCH, false),
                ] {
                    let mut u = Up::new(mode, scale, purpose, enter, true);
                    if voice {
                        u.ime.start();
                        u.ime.luna(crate::panels::ime::tags::PROBE, &json!({"configs": {"system.supportVoiceRecognition": true}}), 0);
                        u.apply();
                    }
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} {mode:?} {scale}: {f:#?}");
                    let g = u.geo();
                    assert!(g["sheet"]["y"].as_f64().unwrap() >= 54.0, "{name} {scale}: the sheet is on the screen: {}", g["sheet"]);
                    if let Some(mode_key) = u.ime.layout().index_of("mode") {
                        u.ime.click(mode_key, u.t);
                        u.apply();
                        let f = u.lint();
                        assert!(f.is_empty(), "{name} marks {mode:?} {scale}: {f:#?}");
                    }
                }
            }
        }
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, PURPOSE_NORMAL, 0, false);
        u.ime.field_is(&json!({"active": true, "show": false, "model": 3}), 2000);
        u.t = 2000;
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE), "the keys stay while it slides out");
        u.ime.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.ime.tick(2300);
        u.t = 2300;
        u.apply();
        assert!(!u.s.is_up());
    }

    #[test]
    fn ids_name_the_keys() {
        for i in [0, 1, 50] {
            assert_eq!(key_of(key_id(i)), Some(i));
        }
        assert_eq!(key_of(Id(1)), None);
    }
}
