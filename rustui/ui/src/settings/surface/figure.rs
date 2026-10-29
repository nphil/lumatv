//! The picture over a page's rows (During the screensaver): the dim saver in miniature, with what the focused row would put on it, and
//! beside it the key to the answers' glyphs with the one shown lit. It follows the focus and the picker's highlight, so an answer is seen
//! before it is chosen. The picture holds no words (a clock, two bars standing for text, glyphs); the words are the key's.
//!
//! The four answers look like what they are: *over* is a card above the dim saver, *inside* a quiet chip in the saver's own area, *wake*
//! the screen lit with the card on it, *hide* nothing but the saver. Each is a group whose opacity springs between the states.

use super::layout::Sizes;
use super::paint::Paint;
use super::rows::{other, pill};
use crate::kit::button::faded;
use crate::kit::motion::Beat;
use crate::kit::{Kit, icon};
use crate::settings::view::Figure;
use compositor_protocol::{Ease, GroupPose, PoseBinding, PoseProperties, Props, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::Align;
use ui::components::plate;
use ui::components::text::{Run, line_height};
use ui::components::text_layer::{Item, TextLayer};
use ui::geometry::Frame;
use ui::group::Group;
use ui::motion::{Motion, MotionBounds, MotionToken, Settle, Units};
use ui::scene::{ImageLayer, ShapeLayer};

const CLEAR: Rgba = Rgba(0, 0, 0, 0);
const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
/// The panel's black and what sits on it: the saver is dark in both looks, it is a picture of the screen.
const PANEL: Rgba = Rgba(9, 8, 13, 255);
const CARD: Rgba = Rgba(40, 36, 56, 255);
const CARD_EDGE: Rgba = Rgba(86, 80, 112, 255);
const BAR: Rgba = Rgba(150, 144, 172, 255);
const BAR_DIM: Rgba = Rgba(92, 88, 112, 255);
const GLYPH: Rgba = Rgba(226, 222, 240, 255);

/// Where the picture and its key are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureGeo {
    pub screen: Frame,
    pub key: Frame,
    pub height: f32,
    pub columns: usize,
    pub line: f32,
    /// At a large text size the key is the one answer shown, beside the picture, not the four.
    pub single: bool,
}

impl FigureGeo {
    /// Under the page's head, `w` wide from `x`: the screen at the left, 16:9, and the key's lines beside it. At a large text size the key
    /// has two columns when they fit (the picture never takes the room of the rows) and each column is as wide as its longest word.
    pub fn of(kit: &mut Kit, sz: &Sizes, x: f32, y: f32, w: f32, words: &[&str]) -> Self {
        let single = kit.text_scale >= 1.5;
        let sw: f32 = if single { 256.0 } else { 320.0 };
        let sh = (sw * 9.0 / 16.0).round();
        let ty = kit.ty(TypeRole::Label);
        let widest = words.iter().map(|t| kit.ui.fonts.measure(t, ty).width).fold(0.0, f32::max);
        let col_w = (sz.s3 + sz.chevron() + sz.s3 + widest + sz.s4).ceil();
        let kx = x + sw + sz.s7;
        let avail = (x + w - kx).max(0.0);
        let line = (kit.line(TypeRole::Label) + sz.s3).max((sz.target * 0.6).round()).ceil();
        let lines = if single { 1 } else { words.len() };
        let key_h = lines as f32 * line;
        let height = sh.max(key_h);
        let screen = Frame::new(x, y + ((height - sh) / 2.0).round(), sw, sh);
        let key = Frame::new(kx, y + ((height - key_h) / 2.0).round(), col_w.min(avail.max(col_w)), key_h);
        FigureGeo { screen, key, height, columns: 1, line, single }
    }
}

/// A group whose opacity springs between hidden and shown.
struct Fader {
    group: Group,
    motion: Motion,
    shown: bool,
}

impl Fader {
    fn new(kit: &Kit, parent: &Group) -> std::io::Result<Self> {
        let scene = &kit.ui.scene;
        let rest = GroupPose { opacity: 0.0, ..GroupPose::IDENTITY };
        let group = scene.group(Some(parent.id()), rest).map_err(other)?;
        let motion = scene.motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress)).map_err(other)?;
        group.bind(&motion, PoseBinding { values: [0.0, 1.0], from: rest, to: GroupPose::IDENTITY, properties: PoseProperties::OPACITY }).map_err(other)?;
        Ok(Fader { group, motion, shown: false })
    }

    fn set(&mut self, kit: &Kit, on: bool) {
        if self.shown == on {
            return;
        }
        self.shown = on;
        let ms = kit.beat_ms(Beat::Card);
        let _ = self.motion.settle(Some(if on { 1.0 } else { 0.0 }), if ms == 0 { Some(if on { 1.0 } else { 0.0 }) } else { None }, Settle::Timed(MotionToken { duration_ms: ms, ease: Ease::Out }));
    }
}

/// A card or a chip: a plate, a glyph the class changes, and bars that stand for its words.
struct Mark {
    fade: Fader,
    glyph: Option<ImageLayer>,
    _parts: Vec<ShapeLayer>,
}

pub struct FigureArt {
    sig: String,
    pub geo: FigureGeo,
    _frame: ShapeLayer,
    _face: Vec<ShapeLayer>,
    _clock: Option<ImageLayer>,
    wake: Fader,
    _wake_plate: ShapeLayer,
    over: Mark,
    inside: Mark,
    hide: Fader,
    _hide_glyph: Option<ImageLayer>,
    key: Vec<KeyArt>,
    key_plate: ShapeLayer,
    key_text: TextLayer,
    icon_now: String,
    lit_now: Option<String>,
}

struct KeyRun {
    text: String,
    ty: ui::text::Type,
    colour: Rgba,
    x: f32,
    y: f32,
    w: f32,
}

impl KeyRun {
    fn item(&self) -> Item<'_> {
        Item::Text(Run { text: &self.text, ty: self.ty, colour: self.colour, x: self.x, y: self.y, w: self.w, align: Align::Left })
    }
}

struct KeyArt {
    id: String,
    icon: String,
    glyph: Option<ImageLayer>,
    at: Props,
    frame: Frame,
}

/// `a` over `b` at `k` of `a`, opaque.
fn mix(a: Rgba, b: Rgba, k: f32) -> Rgba {
    let m = |x: u8, y: u8| (f32::from(x) * k + f32::from(y) * (1.0 - k)).round().clamp(0.0, 255.0) as u8;
    Rgba(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2), 255)
}

impl FigureArt {
    pub fn signature(kit: &Kit, fig: &Figure, geo: &FigureGeo) -> String {
        format!("{}|{}|{}|{}|{:?}|{}", fig.face, fig.dim, kit.mode as u8, kit.text_scale.to_bits(), geo.screen, fig.legend.iter().map(|l| format!("{}:{}:{}", l.id, l.icon, l.word)).collect::<Vec<_>>().join(","))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(kit: &mut Kit, paint: &Paint, sz: &Sizes, parent: &Group, geo: FigureGeo, fig: &Figure, z: i32, sig: String) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let s = geo.screen;
        let radius = (kit.tokens.radius(Radius::Card) * 0.5).round();
        let frame = plate::shape(&kit.ui, s, z, pill(PANEL, paint.edge_raised, radius, 1.0));
        // the saver's face, as bright as its dimming leaves it
        let k = if fig.dim { 0.5 } else { 0.9 };
        let mut face = Vec::new();
        let mut clock = None;
        let accent = paint.accent;
        match fig.face.as_str() {
            "art" => {
                let inner = Frame::new(s.x + (s.w * 0.14).round(), s.y + (s.h * 0.16).round(), (s.w * 0.72).round(), (s.h * 0.66).round());
                face.push(plate::shape(&kit.ui, inner, z + 1, pill(mix(accent, PANEL, 0.22 * k * 2.0), CLEAR, 4.0, 0.0)));
                let hill = Frame::new(inner.x, inner.y + (inner.h * 0.58).round(), inner.w, (inner.h * 0.42).round());
                face.push(plate::shape(&kit.ui, hill, z + 2, pill(mix(accent, PANEL, 0.12 * k * 2.0), CLEAR, 4.0, 0.0)));
                let d = (inner.h * 0.2).round();
                face.push(plate::shape(&kit.ui, Frame::new(inner.x + (inner.w * 0.68).round(), inner.y + (inner.h * 0.14).round(), d, d), z + 2, pill(mix(GLYPH, PANEL, 0.4 * k), CLEAR, d / 2.0, 0.0)));
            }
            "off" => {}
            _ => {
                let ty = ui::text::Type { px: (s.h * 0.24).round() as u32, weight: ui::text::Weight::Regular };
                let text = "10:08";
                let ext = kit.ui.fonts.measure(text, ty);
                let (w, h) = (ext.width.ceil() as u32 + 4, ext.height().ceil() as u32 + 4);
                let at = Props::at((s.x + (s.w - w as f32) / 2.0).round(), (s.y + s.h * 0.33).round());
                if let Ok(mut layer) = scene.image(w, h, z + 1, at) {
                    let mut px = vec![0u8; (w * h * 4) as usize];
                    let ink = mix(GLYPH, PANEL, 0.62 * k);
                    kit.ui.fonts.draw(&mut ui::text::Canvas { pixels: &mut px, width: w as usize }, text, ty, (2.0, 2.0 + ext.ascent), ink);
                    layer.pixels_mut().copy_from_slice(&px);
                    layer.damage(None);
                    clock = Some(layer);
                }
                // two small bars under it: the saver's own items
                let bw = (s.w * 0.22).round();
                let bh = (s.h * 0.035).round().max(3.0);
                let by = (s.y + s.h * 0.62).round();
                face.push(plate::shape(&kit.ui, Frame::new(s.x + ((s.w - bw) / 2.0).round(), by, bw, bh), z + 1, pill(mix(BAR, PANEL, 0.3 * k), CLEAR, bh / 2.0, 0.0)));
            }
        }
        // the lit screen: what Wake does to the dimmed saver
        let wake = Fader::new(kit, parent)?;
        let wake_plate = scene.adopting(&wake.group, || plate::shape(&kit.ui, s, z + 3, pill(faded(Rgba(255, 255, 255, 255), 0.16), CLEAR, radius, 0.0)));
        // over: a card at the corner pop-ups come to
        let card_w = (s.w * 0.4).round();
        let card_h = (s.h * 0.27).round();
        let card_at = Frame::new((s.right() - card_w - s.w * 0.05).round(), (s.y + s.h * 0.06).round(), card_w, card_h);
        let over = Self::mark(kit, parent, card_at, z + 5, true, (CARD, CARD_EDGE))?;
        // inside: a quiet chip under the clock, no brighter than the saver
        let chip_w = (s.w * 0.44).round();
        let chip_h = (s.h * 0.17).round();
        let chip_at = Frame::new((s.x + (s.w - chip_w) / 2.0).round(), (s.y + s.h * 0.74).round(), chip_w, chip_h);
        let inside = Self::mark(kit, parent, chip_at, z + 4, false, (mix(CARD, PANEL, 0.45), mix(CARD_EDGE, PANEL, 0.35)))?;
        // hide: the saver alone, and a glyph that says nothing else comes
        let hide = Fader::new(kit, parent)?;
        let hs = (s.h * 0.16).round() as u32;
        let hide_glyph = scene.adopting(&hide.group, || icon::layer(kit, "eye-off", hs, mix(BAR_DIM, PANEL, 0.9), z + 4, Props::at((s.x + (s.w - hs as f32) / 2.0).round(), (s.y + s.h * 0.75).round())))?;
        // the key
        let (key, key_plate, key_text) = Self::key(kit, paint, sz, &geo, fig, z + 1)?;
        let mut art = FigureArt {
            sig,
            geo,
            _frame: frame,
            _face: face,
            _clock: clock,
            wake,
            _wake_plate: wake_plate,
            over,
            inside,
            hide,
            _hide_glyph: hide_glyph,
            key,
            key_plate,
            key_text,
            icon_now: String::new(),
            lit_now: None,
        };
        // the card and the chip hold a glyph of the row's: a neutral one until a row is focused
        art.paint_glyphs(kit, "bell");
        Ok(art)
    }

    fn mark(kit: &mut Kit, parent: &Group, at: Frame, z: i32, card: bool, (fill, edge): (Rgba, Rgba)) -> std::io::Result<Mark> {
        let fade = Fader::new(kit, parent)?;
        let scene = kit.ui.scene.clone();
        let radius = (at.h * if card { 0.22 } else { 0.5 }).round();
        let mut parts = Vec::new();
        let mut glyph = None;
        scene.adopting(&fade.group, || -> std::io::Result<()> {
            parts.push(plate::shape(&kit.ui, at, z, pill(fill, edge, radius, 1.0)));
            let side = (at.h * if card { 0.5 } else { 0.56 }).round();
            let gx = at.x + if card { (at.h * 0.22).round() } else { (at.h * 0.3).round() };
            glyph = icon::layer(kit, "bell", side as u32, GLYPH, z + 1, Props::at(gx, (at.y + (at.h - side) / 2.0).round()))?;
            let bx = gx + side + (at.h * 0.18).round();
            let bw = at.right() - bx - (at.h * if card { 0.22 } else { 0.4 }).round();
            let bh = (at.h * if card { 0.1 } else { 0.16 }).round().max(3.0);
            if card {
                parts.push(plate::shape(&kit.ui, Frame::new(bx, (at.y + at.h * 0.3).round(), bw, bh), z + 1, pill(BAR, CLEAR, bh / 2.0, 0.0)));
                parts.push(plate::shape(&kit.ui, Frame::new(bx, (at.y + at.h * 0.56).round(), (bw * 0.62).round(), bh), z + 1, pill(BAR_DIM, CLEAR, bh / 2.0, 0.0)));
            } else {
                parts.push(plate::shape(&kit.ui, Frame::new(bx, (at.y + (at.h - bh) / 2.0).round(), bw, bh), z + 1, pill(BAR_DIM, CLEAR, bh / 2.0, 0.0)));
            }
            Ok(())
        })?;
        Ok(Mark { fade, glyph, _parts: parts })
    }

    fn key(kit: &mut Kit, paint: &Paint, sz: &Sizes, geo: &FigureGeo, fig: &Figure, z: i32) -> std::io::Result<(Vec<KeyArt>, ShapeLayer, TextLayer)> {
        let side = sz.chevron().round() as u32;
        let per = fig.legend.len().max(1);
        let col_w = geo.key.w.floor();
        let radius = kit.tokens.radius(Radius::Control).round();
        let plate_shape = plate::shape(&kit.ui, Frame::new(0.0, 0.0, col_w, geo.line), z, pill(paint.wash_selected, CLEAR, radius, 0.0));
        plate_shape.set(HIDDEN);
        let mut entries = Vec::new();
        for (i, l) in fig.legend.iter().enumerate() {
            let (col, row) = (i / per, if geo.single { 0 } else { i % per });
            let frame = Frame::new(geo.key.x + col as f32 * col_w, geo.key.y + row as f32 * geo.line, col_w, geo.line);
            let at = Props::at(frame.x + sz.s3, (frame.y + (frame.h - side as f32) / 2.0).round());
            let g = icon::layer(kit, &l.icon, side, paint.ink3, z + 2, if geo.single { HIDDEN } else { at })?;
            entries.push(KeyArt { id: l.id.clone(), icon: l.icon.clone(), glyph: g, at, frame });
        }
        let runs = Self::key_runs(kit, paint, sz, geo, &entries, fig, None);
        let layer = TextLayer::new(&mut kit.ui, geo.key, z + 2, &runs.iter().map(KeyRun::item).collect::<Vec<_>>())?;
        Ok((entries, plate_shape, layer))
    }

    /// The key's words, each beside its glyph, the lit one in the full ink.
    fn key_runs(kit: &mut Kit, paint: &Paint, sz: &Sizes, geo: &FigureGeo, entries: &[KeyArt], fig: &Figure, lit: Option<&str>) -> Vec<KeyRun> {
        let ty = kit.ty(TypeRole::Label);
        let ll = line_height(ty);
        let before = sz.s3 + sz.chevron() + sz.s3;
        entries
            .iter()
            .zip(&fig.legend)
            .filter(|(e, _)| !geo.single || lit == Some(e.id.as_str()))
            .map(|(e, l)| KeyRun {
                text: l.word.clone(),
                ty,
                colour: if lit == Some(e.id.as_str()) { paint.ink } else { paint.ink2 },
                x: e.frame.x - geo.key.x + before,
                y: e.frame.y - geo.key.y + ((e.frame.h - ll) / 2.0).round(),
                w: (e.frame.w - before - sz.s3).max(1.0),
            })
            .collect()
    }

    fn paint_glyphs(&mut self, kit: &mut Kit, name: &str) {
        if self.icon_now == name {
            return;
        }
        self.icon_now = name.to_string();
        for m in [&mut self.over, &mut self.inside] {
            if let Some(g) = &mut m.glyph {
                icon::repaint(kit, g, name, GLYPH);
            }
        }
    }

    /// The picture for `fig`: the answer the focused row would give (or none), and the key lit to match.
    pub fn update(&mut self, kit: &mut Kit, paint: &Paint, sz: &Sizes, fig: &Figure) {
        let (name, how) = match &fig.shows {
            Some((icon, how)) => (Some(icon.as_str()), Some(how.as_str())),
            None => (None, None),
        };
        if let Some(n) = name {
            self.paint_glyphs(kit, n);
        }
        let wake = how == Some("wake");
        let inside = how == Some("inside");
        let hide = how == Some("hide");
        // an answer this page does not know is a card, as a pop-up is
        let over = how.is_some() && !inside && !hide;
        self.wake.set(kit, wake);
        self.over.fade.set(kit, over);
        self.inside.fade.set(kit, inside);
        self.hide.set(kit, hide);
        if self.lit_now.as_deref() != how {
            self.lit_now = how.map(str::to_string);
            let mut at = None;
            for (e, l) in self.key.iter_mut().zip(&fig.legend) {
                let on = how == Some(l.id.as_str());
                if on {
                    at = Some(e.frame);
                }
                if let Some(g) = &mut e.glyph {
                    icon::repaint(kit, g, &e.icon, if on { paint.accent } else { paint.ink3 });
                    if self.geo.single {
                        g.set(if on { e.at } else { HIDDEN });
                    }
                }
            }
            match at {
                Some(f) if !self.geo.single => self.key_plate.set(Props::at(f.x, f.y)),
                Some(_) => {}
                None => self.key_plate.set(HIDDEN),
            }
            let runs = Self::key_runs(kit, paint, sz, &self.geo, &self.key, fig, how);
            self.key_text.draw(&mut kit.ui, &runs.iter().map(KeyRun::item).collect::<Vec<_>>());
        }
    }

    /// The frames of the picture and of the key's entries, for the layout lint.
    pub fn frames(&self) -> Vec<(String, Frame)> {
        let mut v = vec![("saver".to_string(), self.geo.screen)];
        v.extend(self.key.iter().map(|k| (format!("key:{}", k.id), k.frame)));
        v
    }

    pub fn sig(&self) -> &str {
        &self.sig
    }
}
