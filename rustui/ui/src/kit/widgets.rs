//! The TV's small controls, built from the kit's panes, text, icons and the flowing focus (`kit/LuTile.qml`, `LuChip.qml`, `LuToggle.qml`,
//! `LuSlider.qml` and the plate of a focused row of the QML shell): what a list of rows is made of. A control is a few layers made once
//! where it sits and changed in place (its state is a property of the layers it holds); the ones that are only a picture of a shape and an
//! icon are drawn on the CPU into one image layer, so a row costs the compositor few layers (the scene's ceiling is 256 for every kind).
//!
//! - [`Px`]: a premultiplied canvas that draws rounded shapes with anti-aliased edges, icons and other pictures over it;
//! - [`tile`]: the rounded square an icon sits in (a quiet wash at rest, the accent when it says what is on);
//! - [`Plate`]: what a focusable is behind its words: the lit wash and edge, lifted by the flowing focus like every focusable;
//! - [`Toggle`], [`Chip`], [`Stepper`], [`Slider`]: a switch, a state tag, the value of a stepper with its chevrons, a track with a thumb.

use super::flow::Lifted;
use super::focus::{Id, Region, Scope};
use super::text::{Text, role_style, width_of};
use super::{Engine, Kit};
use compositor_protocol::{Ease, Fill, GroupPose, Props, Rect, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::{Mode, PaletteRole};
use ui::scene::{ImageLayer, ShapeLayer};

/// A premultiplied canvas: rounded shapes with a one-pixel anti-aliased edge, and pictures laid over.
#[derive(Clone, Debug, PartialEq)]
pub struct Px {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

fn over(dst: &mut [u8], src: Rgba, cover: f32) {
    let k = cover.clamp(0.0, 1.0);
    if k <= 0.0 {
        return;
    }
    let s = |v: u8| f32::from(v) * k;
    let sa = s(src.3);
    let keep = 1.0 - sa / 255.0;
    dst[0] = (s(src.0) + f32::from(dst[0]) * keep).round().min(255.0) as u8;
    dst[1] = (s(src.1) + f32::from(dst[1]) * keep).round().min(255.0) as u8;
    dst[2] = (s(src.2) + f32::from(dst[2]) * keep).round().min(255.0) as u8;
    dst[3] = (sa + f32::from(dst[3]) * keep).round().min(255.0) as u8;
}

impl Px {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, data: vec![0; (w * h * 4) as usize] }
    }

    pub fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.w + x) * 4) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// A rounded rectangle at (`x`, `y`) of `w` by `h` with corner `radius`, filled with `fill` and (if `edge` has a width) a border of
    /// that width inside its edge, over what is there.
    pub fn round_rect(&mut self, at: Frame, radius: f32, fill: Rgba, edge: Option<(Rgba, f32)>) {
        let Frame { x, y, w, h } = at;
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);
        let x0 = (x.floor() as i32 - 1).max(0);
        let x1 = ((x + w).ceil() as i32 + 1).min(self.w as i32);
        let y0 = (y.floor() as i32 - 1).max(0);
        let y1 = ((y + h).ceil() as i32 + 1).min(self.h as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let (qx, qy) = ((px as f32 + 0.5 - cx).abs() - hx, (py as f32 + 0.5 - cy).abs() - hy);
                // the signed distance to the edge: negative inside
                let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                let inside = (0.5 - d).clamp(0.0, 1.0);
                if inside <= 0.0 {
                    continue;
                }
                let i = ((py as u32 * self.w + px as u32) * 4) as usize;
                over(&mut self.data[i..i + 4], fill, inside);
                if let Some((ec, ew)) = edge
                    && ew > 0.0
                {
                    let ring = inside * (d + ew + 0.5).clamp(0.0, 1.0);
                    over(&mut self.data[i..i + 4], ec, ring);
                }
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, fill: Rgba, edge: Option<(Rgba, f32)>) {
        self.round_rect(Frame::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0), radius, fill, edge);
    }

    /// A premultiplied picture `src` of `sw` by `sh` laid over at (`dx`, `dy`).
    pub fn lay(&mut self, src: &[u8], sw: u32, sh: u32, dx: i32, dy: i32) {
        for y in 0..sh as i32 {
            for x in 0..sw as i32 {
                let (tx, ty) = (x + dx, y + dy);
                if tx < 0 || ty < 0 || tx >= self.w as i32 || ty >= self.h as i32 {
                    continue;
                }
                let s = ((y as u32 * sw + x as u32) * 4) as usize;
                let px = Rgba(src[s], src[s + 1], src[s + 2], src[s + 3]);
                if px.3 == 0 {
                    continue;
                }
                let i = ((ty as u32 * self.w + tx as u32) * 4) as usize;
                over(&mut self.data[i..i + 4], px, 1.0);
            }
        }
    }

    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Writes the canvas into an image layer of the same size.
    pub fn paint(&self, layer: &mut ImageLayer) {
        if layer.width() == self.w && layer.height() == self.h {
            layer.pixels_mut().copy_from_slice(&self.data);
            layer.damage(None);
        }
    }

    pub fn layer(&self, kit: &Kit, z: i32, at: Props) -> std::io::Result<ImageLayer> {
        let mut layer = kit.ui.scene.image(self.w, self.h, z, at)?;
        self.paint(&mut layer);
        Ok(layer)
    }
}

/// A square picture turned a quarter clockwise (the chevrons of the tile editor's move marks point up and down).
pub fn rotate_cw(src: &[u8], side: u32) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    for y in 0..side {
        for x in 0..side {
            let from = ((y * side + x) * 4) as usize;
            let (nx, ny) = (side - 1 - y, x);
            let to = ((ny * side + nx) * 4) as usize;
            out[to..to + 4].copy_from_slice(&src[from..from + 4]);
        }
    }
    out
}

/// The ink at `a` of its alpha: a wash.
pub fn ink_at(kit: &Kit, a: f32) -> Rgba {
    super::button::faded(kit.ui.look.ink(), a)
}

/// The focus wash (a little ink over the glass), as the mode has it.
pub fn focus_wash(kit: &Kit) -> Rgba {
    let a = kit.tokens.states.focus_wash[usize::from(kit.mode == Mode::Light)];
    ink_at(kit, a)
}

pub fn selected_wash(kit: &Kit) -> Rgba {
    ink_at(kit, kit.tokens.states.selected_wash)
}

pub fn hover_wash(kit: &Kit) -> Rgba {
    ink_at(kit, kit.tokens.states.hover_wash)
}

/// On the dark glass the focus wash lifts the background toward the white label (4.5:1 on the brightest wallpaper, 4.5 needed): a shade
/// under it keeps the label's ground where it was. The plate's fill is that shade with the wash over it.
pub fn lit_fill(kit: &Kit) -> Rgba {
    let wash = focus_wash(kit);
    if kit.mode == Mode::Light {
        return wash;
    }
    let mut px = [0u8; 4];
    over(&mut px, Rgba(0, 0, 0, 41), 1.0);
    over(&mut px, wash, 1.0);
    Rgba(px[0], px[1], px[2], px[3])
}

/// The lit edge of a focused plate.
pub fn lit_edge(kit: &Kit) -> Rgba {
    kit.ui.look.colour(PaletteRole::EdgeRaised)
}

pub fn edge(kit: &Kit) -> Rgba {
    kit.ui.look.colour(PaletteRole::Edge)
}

/// The side of the icon in a tile of `side`.
pub fn glyph_of(side: u32) -> u32 {
    (side as f32 * 0.55).round() as u32
}

/// The rounded square an icon sits in: a quiet wash at rest, the accent when `accent`, the glyph in ink (or the ink that reads on the accent).
pub fn tile(kit: &mut Kit, side: u32, icon_name: &str, accent: bool) -> Px {
    let mut px = Px::new(side, side);
    let radius = (side as f32 * 0.32).round();
    let (fill, ink) = if accent { (kit.ui.look.accent(), kit.ui.look.accent_ink()) } else { (selected_wash(kit), kit.ui.look.ink()) };
    let edge = if accent { None } else { Some((edge(kit), 1.0)) };
    px.round_rect(Frame::new(0.0, 0.0, side as f32, side as f32), radius, fill, edge);
    let g = glyph_of(side);
    if let Some(icon) = kit.icons.draw(icon_name, g, ink) {
        let o = ((side - g) / 2) as i32;
        px.lay(&icon, g, g, o, o);
    }
    px
}

pub const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

/// How a focusable of a kind lifts: its scale, and its rise in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lift {
    Row,
    Card,
    Tile,
    Button,
}

impl Lift {
    pub fn scale(self, kit: &Kit) -> f32 {
        let f = &kit.tokens.focus;
        match self {
            Lift::Row => f.row_scale,
            Lift::Card => f.card_scale,
            Lift::Tile => f.tile_scale,
            Lift::Button => f.button_scale,
        }
    }

    pub fn rise(self, kit: &Kit) -> f32 {
        if self == Lift::Card { kit.tokens.focus.card_lift } else { 0.0 }
    }
}

/// Where a focusable is: its name, its frame on the screen (what the pointer hits), the same frame in the space of the group it hangs
/// under (they differ for a list that scrolls), its region and scope and its depth.
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    pub id: Id,
    pub screen: Frame,
    pub local: Frame,
    pub region: Region,
    pub scope: Scope,
    pub z: i32,
}

/// What a focusable is behind its words: the lit wash with its edge, hidden at rest and carried by the flowing focus (the lift's scale and
/// the lit glass's opacity, one value on a track of the engine's pool). The words and the controls on it do not move.
pub struct Plate {
    pub id: Id,
    shape: ShapeLayer,
    radius: f32,
}

impl Plate {
    /// Makes the plate under `parent`, registers it with the engine and gives it its lift.
    pub fn new(kit: &mut Kit, engine: &mut Engine, parent: &Group, spot: Spot, lift: Lift, radius: f32) -> std::io::Result<Self> {
        let (w, h) = (spot.local.w, spot.local.h);
        let rest = GroupPose { x: spot.local.x, y: spot.local.y, pivot_x: w / 2.0, pivot_y: h / 2.0, ..GroupPose::IDENTITY };
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let group = kit.ui.scene.group(Some(parent.id()), rest).map_err(other)?;
        let lit = kit.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let scene = kit.ui.scene.clone();
        let shape = Shape { corner_radius: radius, fill: Fill::Solid(lit_fill(kit)), border_width: 1.0, border: lit_edge(kit) };
        let layer = scene.adopting(&lit, || kit.ui.scene.shape(w.round() as u32, h.round() as u32, spot.z, shape, Props::at(0.0, 0.0)));
        engine.focus.register(spot.id, spot.screen, spot.region, spot.scope);
        // The plate grows about its own centre and does not rise: what is on it does not move, so a rise would take the plate off the
        // tile it is behind (4 px higher at the top, level at the bottom).
        engine.flow.lift(spot.id, Lifted { group, rest, scale: lift.scale(kit), rise: 0.0, lit: Some(lit) });
        Ok(Self { id: spot.id, shape: layer, radius })
    }

    /// Gives the plate back: it leaves the registry and loses its lift (its layer goes with the handle).
    pub fn destroy(self, engine: &mut Engine) {
        engine.focus.unregister(self.id);
        engine.flow.drop_lift(self.id);
    }

    /// The same plate in another look.
    pub fn restyle(&mut self, kit: &Kit) {
        self.shape.reshape(Shape { corner_radius: self.radius, fill: Fill::Solid(lit_fill(kit)), border_width: 1.0, border: lit_edge(kit) });
    }

    pub fn layers(&self) -> usize {
        1
    }
}

/// The hover cue: the hover wash and a rim lifted from the glass edge, nothing else (no lift, no scale: those are focus). One layer that
/// moves to the control under the pointer; a control of another size gets a layer of its size (the cue is the pointer's, a rare thing).
pub struct Hover {
    z: i32,
    shape: Option<ShapeLayer>,
    at: Option<(Frame, f32)>,
}

impl Hover {
    pub fn new(z: i32) -> Self {
        Self { z, shape: None, at: None }
    }

    fn look(kit: &Kit, radius: f32) -> Shape {
        Shape { corner_radius: radius, fill: Fill::Solid(hover_wash(kit)), border_width: 1.0, border: super::button::faded(lit_edge(kit), 0.6) }
    }

    /// The cue over `frame` (in the space of `parent`) with `radius`, or gone.
    pub fn show(&mut self, kit: &Kit, parent: &Group, target: Option<(Frame, f32)>) {
        let Some((f, radius)) = target else {
            if let Some(s) = &self.shape {
                s.set(HIDDEN);
            }
            self.at = None;
            return;
        };
        if self.at.map(|(a, r)| (a.w.round(), a.h.round(), r)) != Some((f.w.round(), f.h.round(), radius)) || self.shape.is_none() {
            let scene = kit.ui.scene.clone();
            self.shape = Some(scene.adopting(parent, || kit.ui.scene.shape(f.w.round() as u32, f.h.round() as u32, self.z, Self::look(kit, radius), HIDDEN)));
        }
        self.at = Some((f, radius));
        if let Some(s) = &self.shape {
            s.set(Props::at(f.x, f.y));
        }
    }

    /// The cue is gone (the pointer left, a key hid it, the surface goes).
    pub fn hide(&mut self) {
        if let Some(s) = &self.shape {
            s.set(HIDDEN);
        }
        self.at = None;
    }

    pub fn is_shown(&self) -> bool {
        self.at.is_some()
    }

    pub fn layers(&self) -> usize {
        usize::from(self.shape.is_some())
    }
}

/// A switch: a pill track and a thumb that slides. On is the accent track with the thumb in the ink that reads on it; off is the quiet
/// track. The position of the thumb says the state, not the colour.
pub struct Toggle {
    track: ShapeLayer,
    thumb: ShapeLayer,
    on: bool,
    frame: Frame,
}

impl Toggle {
    pub fn size(kit: &Kit) -> (f32, f32) {
        let h = (kit.tokens.target * 0.75).round();
        ((h * 1.75).round(), h)
    }

    fn pad(kit: &Kit) -> f32 {
        kit.space(1)
    }

    fn looks(kit: &Kit, on: bool, h: f32) -> (Shape, Shape) {
        let look = &kit.ui.look;
        let track = if on { look.accent() } else { kit.ui.look.theme.materials.track_off };
        let thumb_fill = if on { look.accent_ink() } else { look.ink() };
        (
            plate::solid(track, h / 2.0),
            Shape { corner_radius: (h - 2.0 * Self::pad(kit)) / 2.0, fill: Fill::Solid(thumb_fill), border_width: 1.0, border: lit_edge(kit) },
        )
    }

    fn thumb_x(&self, kit: &Kit, on: bool) -> f32 {
        let pad = Self::pad(kit);
        if on { self.frame.x + self.frame.w - (self.frame.h - 2.0 * pad) - pad } else { self.frame.x + pad }
    }

    pub fn new(kit: &Kit, frame: Frame, z: i32, on: bool) -> Self {
        let (track_shape, thumb_shape) = Self::looks(kit, on, frame.h);
        let track = kit.ui.scene.shape(frame.w as u32, frame.h as u32, z, track_shape, Props::at(frame.x, frame.y));
        let d = (frame.h - 2.0 * Self::pad(kit)).round();
        let me = Self { track, thumb: kit.ui.scene.shape(d as u32, d as u32, z + 1, thumb_shape, Props::at(0.0, 0.0)), on, frame };
        let x = me.thumb_x(kit, on);
        me.thumb.set(Props::at(x, frame.y + Self::pad(kit)));
        me
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The state changes: the thumb slides over the label beat (at once under reduced motion), the track takes its colour.
    pub fn set(&mut self, kit: &Kit, on: bool) {
        if self.on == on {
            return;
        }
        self.on = on;
        let (track, thumb) = Self::looks(kit, on, self.frame.h);
        self.track.reshape(track);
        self.thumb.reshape(thumb);
        let to = Props::at(self.thumb_x(kit, on), self.frame.y + Self::pad(kit));
        let ms = kit.beat_ms(super::motion::Beat::Label);
        if ms == 0 { self.thumb.set(to) } else { self.thumb.animate(to, ms, Ease::Out) }
    }

    pub fn layers(&self) -> usize {
        2
    }
}

/// A short state tag: text, with a dot in a semantic colour when there is a tone. The meaning is in the text; the dot only helps.
pub struct Chip {
    /// The pill and its dot, and the words on it: held so they stay on the screen as long as the chip does.
    _pill: ImageLayer,
    _words: Text,
    frame: Frame,
}

impl Chip {
    pub fn tone_colour(kit: &Kit, tone: &str) -> Option<Rgba> {
        let look = &kit.ui.look;
        Some(match tone {
            "positive" => look.positive(),
            "warning" => look.warning(),
            "danger" => look.danger(),
            "info" => look.colour(PaletteRole::Info),
            "live" => look.colour(PaletteRole::Live),
            _ => return None,
        })
    }

    /// The size of a chip saying `text` (with a dot when it has a tone).
    pub fn size(kit: &mut Kit, text: &str, tone: &str) -> (f32, f32) {
        let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink(), Align::Left);
        let dot = if tone.is_empty() { 0.0 } else { kit.space(2) + kit.space(2) };
        ((2.0 * kit.space(4) + dot + width_of(kit, style, text)).ceil(), (kit.line(TypeRole::Caption) + 2.0 * kit.space(2)).ceil())
    }

    /// A chip with its top left at (`x`, `y`).
    pub fn new(kit: &mut Kit, x: f32, y: f32, z: i32, text: &str, tone: &str) -> std::io::Result<Self> {
        let (w, h) = Self::size(kit, text, tone);
        let mut px = Px::new(w as u32, h as u32);
        px.round_rect(Frame::new(0.0, 0.0, w, h), kit.tokens.radius(Radius::Pill).min(h / 2.0), selected_wash(kit), Some((edge(kit), 1.0)));
        let mut text_x = kit.space(4);
        if let Some(c) = Self::tone_colour(kit, tone) {
            let d = kit.space(2);
            px.circle(text_x + d / 2.0, h / 2.0, d / 2.0, c, None);
            text_x += d + kit.space(2);
        }
        let img = px.layer(kit, z, Props::at(x, y))?;
        let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink(), Align::Left);
        let tw = width_of(kit, style, text);
        let text = Text::line_at(kit, x + text_x, y + h / 2.0, tw, z + 1, style, text)?;
        Ok(Self { _pill: img, _words: text, frame: Frame::new(x, y, w, h) })
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn layers(&self) -> usize {
        2
    }

    /// Describes the words of the chip to a lint: the chip is a panel of its own, over `parent` (the sheet it sits on), for the words in it.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut super::lint::Lint, name: &str, shift: (f32, f32), parent: usize) {
        let pill = lint.panel_on(&format!("{name} chip"), Frame::new(self.frame.x + shift.0, self.frame.y + shift.1, self.frame.w, self.frame.h), selected_wash(kit), false, parent);
        let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink(), Align::Left);
        let f = self._words.frame();
        lint.word(kit, name, self._words.text(), Frame::new(f.x + shift.0, f.y + shift.1, f.w, f.h), style, Some(pill));
    }
}

/// The value of a stepper or a nudge on a line of its own or beside the words: a chevron at each end and the value between them, in a
/// recessed track. Left and Right change it, which is why the chevrons are there; they brighten with the row's focus.
pub struct Stepper {
    track: ShapeLayer,
    arrows: ImageLayer,
    text: Text,
    frame: Frame,
    focused: bool,
}

impl Stepper {
    pub fn arrow(kit: &Kit) -> u32 {
        (kit.tokens.target * 0.5).round() as u32
    }

    /// The width a stepper needs on one line: a chevron at each end and the longer of the value and `widest`.
    pub fn need(kit: &mut Kit, value: &str, widest: &str) -> f32 {
        let style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Centre);
        2.0 * (kit.space(3) * 2.0 + Self::arrow(kit) as f32) + width_of(kit, style, value).max(width_of(kit, style, widest))
    }

    pub fn height(kit: &Kit) -> f32 {
        (kit.tokens.target * 0.75).round().max(kit.line(TypeRole::Label) + 2.0 * kit.space(2))
    }

    fn track_shape(kit: &Kit, focused: bool) -> Shape {
        // a shade on dark: the track's own ink wash read the value 3.7:1 over a bright wallpaper
        let fill = if kit.mode == Mode::Light { kit.ui.look.theme.materials.track_off } else { Rgba(0, 0, 0, 56) };
        Shape {
            corner_radius: kit.tokens.radius(Radius::Control),
            fill: Fill::Solid(fill),
            border_width: 1.0,
            border: if focused { lit_edge(kit) } else { edge(kit) },
        }
    }

    fn paint_arrows(kit: &mut Kit, px: &mut Px, focused: bool) {
        px.clear();
        let side = Self::arrow(kit);
        let ink = if focused { kit.ui.look.ink() } else { kit.ui.look.ink2() };
        let y = ((px.h - side) / 2) as i32;
        if let Some(left) = kit.icons.draw("chevron-left", side, ink) {
            px.lay(&left, side, side, kit.space(3) as i32, y);
        }
        if let Some(right) = kit.icons.draw("chevron-right", side, ink) {
            px.lay(&right, side, side, px.w as i32 - kit.space(3) as i32 - side as i32, y);
        }
    }

    pub fn new(kit: &mut Kit, frame: Frame, z: i32, value: &str, focused: bool) -> std::io::Result<Self> {
        let track = kit.ui.scene.shape(frame.w.round() as u32, frame.h.round() as u32, z, Self::track_shape(kit, focused), Props::at(frame.x, frame.y));
        let mut px = Px::new(frame.w.round() as u32, frame.h.round() as u32);
        Self::paint_arrows(kit, &mut px, focused);
        let arrows = px.layer(kit, z + 1, Props::at(frame.x, frame.y))?;
        let inset = kit.space(3) * 2.0 + Self::arrow(kit) as f32;
        let style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Centre);
        let shown = ui::components::ellipsise(&mut kit.ui.fonts, value, style.ty, frame.w - 2.0 * inset);
        let text = Text::line_at(kit, frame.x + inset, frame.y + frame.h / 2.0, frame.w - 2.0 * inset, z + 2, style, &shown)?;
        Ok(Self { track, arrows, text, frame, focused })
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn set_value(&mut self, kit: &mut Kit, value: &str) {
        let inset = kit.space(3) * 2.0 + Self::arrow(kit) as f32;
        let ty = kit.ty(TypeRole::Label);
        let shown = ui::components::ellipsise(&mut kit.ui.fonts, value, ty, self.frame.w - 2.0 * inset);
        self.text.set(kit, &shown);
    }

    pub fn set_focused(&mut self, kit: &mut Kit, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        self.track.reshape(Self::track_shape(kit, focused));
        let mut px = Px::new(self.arrows.width(), self.arrows.height());
        Self::paint_arrows(kit, &mut px, focused);
        px.paint(&mut self.arrows);
    }

    pub fn layers(&self) -> usize {
        3
    }

    /// Describes the value's words to a lint, `panel` being what they sit on and `shift` where a scrolled list has moved them.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut super::lint::Lint, name: &str, panel: Option<usize>, shift: (f32, f32)) {
        let style = role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Centre);
        let f = self.text.frame();
        lint.word(kit, name, self.text.text(), Frame::new(f.x + shift.0, f.y + shift.1, f.w, f.h), style, panel);
    }
}

/// A slider: a quiet track, the accent fill to the value and an isolated thumb. The control is as tall as a target, the track slim.
pub struct Slider {
    _track: ShapeLayer,
    fill: ShapeLayer,
    thumb: ShapeLayer,
    frame: Frame,
    from: f64,
    to: f64,
}

impl Slider {
    pub fn thumb_size(kit: &Kit) -> f32 {
        kit.space(7)
    }

    pub fn new(kit: &Kit, frame: Frame, z: i32, from: f64, to: f64, value: f64) -> Self {
        let look = &kit.ui.look;
        let t = Self::thumb_size(kit);
        let track_h = kit.space(3);
        let tx = frame.x + t / 2.0;
        let tw = frame.w - t;
        let ty = frame.y + (frame.h - track_h) / 2.0;
        let track = kit.ui.scene.shape(tw as u32, track_h as u32, z, plate::solid(look.theme.materials.track_off, track_h / 2.0), Props::at(tx, ty));
        let fill = kit.ui.scene.shape(tw as u32, track_h as u32, z + 1, plate::solid(look.accent(), track_h / 2.0), Props::at(tx, ty));
        let thumb = kit.ui.scene.shape(
            t as u32,
            t as u32,
            z + 2,
            Shape { corner_radius: t / 2.0, fill: Fill::Solid(look.ink()), border_width: 1.0, border: lit_edge(kit) },
            Props::at(0.0, 0.0),
        );
        let mut s = Self { _track: track, fill, thumb, frame, from, to };
        s.set(kit, value);
        s
    }

    fn fraction(&self, value: f64) -> f32 {
        if self.to > self.from { ((value - self.from) / (self.to - self.from)).clamp(0.0, 1.0) as f32 } else { 0.0 }
    }

    pub fn set(&mut self, kit: &Kit, value: f64) {
        let f = self.fraction(value);
        let t = Self::thumb_size(kit);
        let track_h = kit.space(3);
        let tx = self.frame.x + t / 2.0;
        let tw = self.frame.w - t;
        let ty = self.frame.y + (self.frame.h - track_h) / 2.0;
        let clip = Rect { x: tx, y: ty, w: (tw * f).round(), h: track_h };
        self.fill.set(Props { clip: Some(clip), ..Props::at(tx, ty) });
        self.thumb.set(Props::at(tx + tw * f - t / 2.0, self.frame.y + (self.frame.h - t) / 2.0));
    }

    pub fn layers(&self) -> usize {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    #[test]
    fn a_rounded_rectangle_is_filled_inside_clear_at_the_corners_and_edged_inside_its_edge() {
        let mut px = Px::new(40, 20);
        px.round_rect(Frame::new(0.0, 0.0, 40.0, 20.0), 8.0, Rgba(0, 0, 100, 255), Some((Rgba(255, 255, 255, 255), 2.0)));
        assert_eq!(px.at(0, 0)[3], 0, "the corner is outside the round");
        assert_eq!(px.at(20, 10), [0, 0, 100, 255], "the middle is the fill");
        assert!(px.at(20, 0)[0] > 200 && px.at(20, 1)[0] > 200, "the edge is inside the outline, two pixels wide");
        assert!(px.at(20, 3)[0] < 10, "and no wider");
        assert!(px.at(1, 10)[3] > 200);
    }

    #[test]
    fn a_picture_laid_over_keeps_what_is_under_where_it_is_clear_and_a_quarter_turn_turns_it() {
        let mut px = Px::new(4, 4);
        px.round_rect(Frame::new(0.0, 0.0, 4.0, 4.0), 0.0, Rgba(10, 20, 30, 255), None);
        let mut icon = vec![0u8; 2 * 2 * 4];
        icon[0..4].copy_from_slice(&[255, 255, 255, 255]);
        px.lay(&icon, 2, 2, 1, 1);
        assert_eq!(px.at(1, 1), [255, 255, 255, 255]);
        assert_eq!(px.at(2, 1), [10, 20, 30, 255]);
        let turned = rotate_cw(&icon, 2);
        assert_eq!(&turned[4..8], &[255, 255, 255, 255], "the top left went to the top right");
        assert_eq!(&turned[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_tile_is_a_wash_with_the_glyph_in_ink_and_the_accent_one_is_the_accents() {
        let (_h, mut kit) = testkit::kit(1.0);
        let t = tile(&mut kit, 48, "wifi", false);
        assert_eq!(t.at(0, 0)[3], 0, "round corners");
        let inked = (8..40).flat_map(|y| (8..40).map(move |x| (x, y))).filter(|(x, y)| t.at(*x, *y)[3] > 150).count();
        assert!(inked > 20, "the glyph is drawn in the middle: {inked} pixels");
        let on = tile(&mut kit, 48, "wifi", true);
        let a = kit.ui.look.accent();
        assert_eq!(&on.at(5, 24)[..3], &[a.0, a.1, a.2], "the accent tile is the accent's colour");
        assert_eq!(glyph_of(48), 26);
    }

    #[test]
    fn a_plate_registers_lifts_and_hands_everything_back() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let scene = kit.ui.scene.clone();
        let root = scene.group(None, GroupPose::IDENTITY).expect("a group");
        let before = kit.room();
        let spot = Spot {
            id: Id(9),
            screen: Frame::new(100.0, 100.0, 300.0, 80.0),
            local: Frame::new(100.0, 100.0, 300.0, 80.0),
            region: Region(0),
            scope: Scope::BASE,
            z: 5,
        };
        let plate = Plate::new(&mut kit, &mut engine, &root, spot, Lift::Row, 22.0).expect("a plate");
        let after = kit.room();
        assert_eq!(before.layers - after.layers, plate.layers());
        assert_eq!(before.groups - after.groups, 2, "the lift group and the lit group");
        assert!(engine.focus.focus(Id(9)));
        assert_eq!(engine.flow.in_flight(), 0);
        engine.flow.go(None, Some(Id(9)), 0).expect("a lift");
        assert_eq!(engine.flow.in_flight(), 1, "a track of the pool is lifting it");
        plate.destroy(&mut engine);
        assert!(engine.focus.frame(Id(9)).is_none());
    }

    #[test]
    fn a_toggle_a_chip_a_stepper_and_a_slider_hold_the_layers_they_say() {
        let (_h, mut kit) = testkit::kit(1.0);
        let b = kit.room();
        let (w, h) = Toggle::size(&kit);
        assert_eq!((w, h), (84.0, 48.0));
        let mut toggle = Toggle::new(&kit, Frame::new(10.0, 10.0, w, h), 3, false);
        assert_eq!(b.layers - kit.room().layers, toggle.layers());
        toggle.set(&kit, true);
        assert!(toggle.is_on());
        let b = kit.room();
        let chip = Chip::new(&mut kit, 10.0, 80.0, 3, "On", "positive").expect("a chip");
        assert_eq!(b.layers - kit.room().layers, chip.layers());
        assert!(chip.frame().w > 60.0 && chip.frame().h > 40.0);
        let b = kit.room();
        let need = Stepper::need(&mut kit, "Cinema", "Filmmaker");
        assert!(need > 112.0 + 100.0, "{need}");
        let height = Stepper::height(&kit);
        let mut stepper = Stepper::new(&mut kit, Frame::new(10.0, 150.0, need, height), 3, "Cinema", false).expect("a stepper");
        assert_eq!(b.layers - kit.room().layers, stepper.layers());
        stepper.set_value(&mut kit, "Vivid");
        stepper.set_focused(&mut kit, true);
        let b = kit.room();
        let mut slider = Slider::new(&kit, Frame::new(10.0, 220.0, 400.0, 64.0), 3, 0.0, 100.0, 40.0);
        assert_eq!(b.layers - kit.room().layers, slider.layers());
        slider.set(&kit, 100.0);
    }

    #[test]
    fn the_lit_fill_has_a_shade_under_the_wash_on_the_dark_glass_only() {
        let (_h, kit) = testkit::kit_in(Mode::Dark, 1.0);
        let dark = lit_fill(&kit);
        assert!(dark.3 > focus_wash(&kit).3, "the shade adds to the wash");
        let (_h2, light) = testkit::kit_in(Mode::Light, 1.0);
        assert_eq!(lit_fill(&light), focus_wash(&light));
    }
}
