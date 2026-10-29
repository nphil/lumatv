//! A row of Settings drawn (`kit/LuRow.qml` and the controls it holds): the plate that lifts with the flowing focus (the lit glass is its
//! lit group's), a tile with the icon, the words as one run of glyph quads, the info mark, and the control at the right (a switch, a
//! segmented tray, a slider, a chevron, a chip, a stepper, a row of cards). A row is made once for its words and kept; what moves
//! (a switch's thumb, the chosen segment, the slider's fill, the lit state, the hover cue) is set in place.
//!
//! Layers hang under the list's scroll group, so a scroll is one track and a row costs nothing while it scrolls; the plate's group is
//! the one the focus lifts (the tile, the words and the control stay where they are, as in the QML row: the lift grows the plate only).

use super::layout::{RowGeom, SegGeom, Sizes};
use super::paint::{Paint, tile_pixels};
use crate::kit::button::faded;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::{Engine, Kit, Lifted, icon};
use crate::settings::row::{Face, Kind, Row};
use compositor_protocol::{Fill, GroupPose, Props, Rect, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::Align;
use ui::components::text::{Run, line_height};
use ui::components::text_layer::{Item, TextLayer};
use ui::geometry::Frame;
use ui::group::Group;
use ui::scene::{ImageLayer, ShapeLayer};

const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
const CLEAR: Rgba = Rgba(0, 0, 0, 0);

/// What a row is made in: the kit and the engine, the colours and the sizes, the group the list scrolls and where its layers go.
pub struct Env<'a> {
    pub kit: &'a mut Kit,
    pub engine: &'a mut Engine,
    pub paint: Paint,
    pub sz: Sizes,
    pub scroll: &'a Group,
    pub z: i32,
    pub region: Region,
    pub scope: Scope,
    /// Where the scroll group's origin is on the screen (the engine's frames are the screen's).
    pub origin: (f32, f32),
}

pub fn other(e: ui::error::UiError) -> std::io::Error {
    std::io::Error::other(format!("{e}"))
}

pub fn pill(fill: Rgba, border: Rgba, radius: f32, bw: f32) -> Shape {
    Shape { corner_radius: radius, fill: Fill::Solid(fill), border_width: bw, border }
}

/// What a row is in the moment: the focus, the pointer's cue, the inner focus of a segmented row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moment {
    pub focused: bool,
    pub hovered: bool,
    /// The inner focus of a segmented row (only while the row is focused).
    pub seg: Option<usize>,
}

/// One card of a row of cards: it is a focusable of its own, so the lift hands over from card to card and from card to row.
pub struct CardArt {
    pub id: Id,
    pub frame: Frame,
    icon: String,
    /// The glyph is an icon (it takes the accent when chosen), not a face.
    face_free: bool,
    glyph_at: Props,
    hover: ShapeLayer,
    base: ShapeLayer,
    _lit: ShapeLayer,
    glyph: Option<ImageLayer>,
    check: Option<ImageLayer>,
    selected: Option<bool>,
    _text: TextLayer,
}

/// A control with parts that move.
enum Trailing {
    None,
    Toggle { track: ShapeLayer, thumb: ShapeLayer, tick: Option<ImageLayer>, at: (f32, f32), on: Option<bool> },
    Seg { chosen: ShapeLayer, mark: ShapeLayer, focus: ShapeLayer, geom: SegGeom, at: (f32, f32), index: Option<usize>, cursor: Option<usize>, first: usize },
    Slider { fill: ShapeLayer, thumb: Option<ShapeLayer>, at: (f32, f32), w: f32, thumb_d: f32, shown: Option<f32> },
    Cards(Vec<CardArt>),
}

/// One row, built.
pub struct RowArt {
    pub id: Id,
    pub row_id: String,
    pub kind: Kind,
    pub geom: RowGeom,
    /// The row's rectangle in the scroll group's space.
    pub rect: Frame,
    pub sig: String,
    focusable: bool,
    last: Row,
    _lit: Option<ShapeLayer>,
    selected_wash: Option<ShapeLayer>,
    hover_wash: Option<ShapeLayer>,
    tile: Option<ImageLayer>,
    _words: Option<TextLayer>,
    trail_text: Option<TextLayer>,
    _chevron: Option<ImageLayer>,
    _plate: Option<ShapeLayer>,
    _arrows: Vec<ImageLayer>,
    mark: Option<ImageLayer>,
    trailing: Trailing,
    moment: Moment,
    /// Where the control and the info mark are, in the row's own coordinates, for the pointer.
    pub control: Option<Frame>,
    pub info: Option<Frame>,
    /// The cards' ids and frames, when the row is a row of cards.
    pub cards: Vec<(Id, Frame)>,
}

/// What of a row decides how it is made: a change in any of it makes the row again; the rest (the switch, the chosen segment, the
/// number, the focus) is set in place.
pub fn signature(row: &Row) -> String {
    let opts: String = row.options.join("\u{1}");
    let cards: String = row.cards.iter().map(|c| format!("{}\u{2}{}\u{2}{}", c.title, c.icon, c.sub)).collect::<Vec<_>>().join("\u{1}");
    let value = if row.kind == Kind::Slider { String::new() } else { row.value.clone() };
    let own = if row.kind == Kind::Value { format!("{}", row.selected) } else { String::new() };
    format!(
        "{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        row.kind,
        row.icon,
        row.label,
        row.detail,
        value,
        opts,
        cards,
        row.help.is_empty(),
        row.value_icon,
        row.value_min,
        row.chevron,
        row.button,
        row.enabled,
        row.danger,
        row.info,
        row.text,
        row.strip,
        row.unit,
        own
    )
}

fn caption_ink(paint: &Paint) -> Rgba {
    if paint.light { paint.ink2 } else { paint.ink3 }
}

fn mid(rect: &Frame, h: f32) -> f32 {
    rect.y + ((rect.h - h) / 2.0).round()
}

/// The plate of a focusable: the group the focus lifts and, in it, the lit glass. The focusable is registered by [`RowArt::sync_focus`]
/// (a row takes the pointer and the focus only where it is whole in view); the lift is the engine's from here.
fn make_plate(env: &mut Env, id: Id, rect: Frame, lift: bool, scale: f32, rise: f32, radius: f32) -> std::io::Result<ShapeLayer> {
    let (w, h) = (rect.w, rect.h);
    let rest = GroupPose { x: rect.x, y: rect.y, pivot_x: w / 2.0, pivot_y: h / 2.0, ..GroupPose::IDENTITY };
    let scene = env.kit.ui.scene.clone();
    let group = scene.group(Some(env.scroll.id()), rest).map_err(other)?;
    let lit_group = scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
    let shape = pill(env.paint.wash_focus, env.paint.edge_raised, radius, 1.0);
    let z = env.z;
    let lit = scene.adopting(&lit_group, || scene.shape(w.round() as u32, h.round() as u32, z + 1, shape, Props::at(0.0, 0.0)));
    if lift {
        env.engine.flow.lift(id, Lifted { group, rest, scale, rise, lit: Some(lit_group) });
    }
    Ok(lit)
}

fn make_wash(env: &mut Env, rect: Frame, hover: bool) -> ShapeLayer {
    let radius = env.kit.tokens.radius(Radius::Row);
    let colour = if hover { env.paint.wash_hover } else { env.paint.wash_selected };
    let edge = if hover { faded(env.paint.edge_raised, 0.6) } else { CLEAR };
    let scene = env.kit.ui.scene.clone();
    let (scroll, z) = (env.scroll, env.z);
    let at = Props { visible: false, opacity: 0.0, ..Props::at(rect.x, rect.y) };
    let shape = pill(colour, edge, radius, if hover { 1.0 } else { 0.0 });
    scene.adopting(scroll, || scene.shape(rect.w.round() as u32, rect.h.round() as u32, z, shape, at))
}

/// The pixels of a tile (a plate with a glyph in it): the plate's colours follow the accent and the row's state.
fn tile_px(env: &mut Env, icon_name: &str, enabled: bool, accent: bool, side: u32, glyph_side: u32) -> Vec<u8> {
    let paint = env.paint;
    let (fill, border, ink) = if accent { (paint.accent, CLEAR, paint.accent_ink) } else { (paint.wash_selected, paint.edge, paint.ink) };
    let dim = |c: Rgba| paint.dim(c, enabled);
    let g = env.kit.icons.draw(icon_name, glyph_side, dim(ink));
    tile_pixels(side, (side as f32 * 0.32).round(), dim(fill), dim(border), g.as_deref().map(|g| (g, glyph_side)))
}

/// The pixels of a person's face on a tile of `side`: the picture of the file (a bundled avatar or a photo) as a rounded square (a fifth
/// of the side, Lucent's avatar), else the letter on the colour.
pub fn face_px(env: &mut Env, face: &Face, side: u32, enabled: bool) -> Vec<u8> {
    let radius = (side as f32 * 0.2).round();
    let paint = env.paint;
    let mut px = match face.file.as_deref().and_then(|f| std::fs::read(f).ok()).and_then(|b| crate::kit::picture::decode(&b).ok()) {
        Some(pic) => {
            let mut p = pic.fitted(side, side, crate::kit::picture::Fit::Cover);
            p.round_corners(radius);
            p.rgba
        }
        None => {
            let straight = lumatv_tokens::colour(&face.colour);
            let colour = straight.map_or(paint.accent, |c| c.premultiply(0.0));
            let mut px = tile_pixels(side, radius, colour, CLEAR, None);
            if !face.letter.is_empty() {
                let luma = f32::from(colour.0) * 0.2126 + f32::from(colour.1) * 0.7152 + f32::from(colour.2) * 0.0722;
                let ink = if luma > 140.0 { Rgba(24, 22, 34, 255) } else { Rgba(250, 248, 255, 255) };
                let ty = ui::text::Type { px: (side as f32 * 0.5).round() as u32, weight: ui::text::Weight::SemiBold };
                let ext = env.kit.ui.fonts.measure(&face.letter, ty);
                let x = ((side as f32 - ext.width) / 2.0).round();
                let baseline = ((side as f32 - ext.height()) / 2.0 + ext.ascent).round();
                env.kit.ui.fonts.draw(&mut ui::text::Canvas { pixels: &mut px, width: side as usize }, &face.letter, ty, (x, baseline), ink);
            }
            px
        }
    };
    if !enabled {
        for v in px.iter_mut() {
            *v = (f32::from(*v) * paint.disabled) as u8;
        }
    }
    px
}

/// A square of `side` holding a tile (a plate with a glyph in it) or a glyph, or a face.
fn make_tile(env: &mut Env, (icon_name, face): (&str, Option<&Face>), (enabled, accent): (bool, bool), at: Props, side: u32, glyph_side: u32) -> std::io::Result<Option<ImageLayer>> {
    if icon_name.is_empty() && face.is_none() {
        return Ok(None);
    }
    let px = match face {
        Some(f) => face_px(env, f, side, enabled),
        None => tile_px(env, icon_name, enabled, accent, side, glyph_side),
    };
    let scene = env.kit.ui.scene.clone();
    let (scroll, z) = (env.scroll, env.z);
    let mut layer = scene.adopting(scroll, || scene.image(side, side, z + 3, at))?;
    layer.pixels_mut().copy_from_slice(&px);
    layer.damage(None);
    Ok(Some(layer))
}

fn make_text(env: &mut Env, frame: Frame, z: i32, items: &[Item]) -> std::io::Result<TextLayer> {
    let scene = env.kit.ui.scene.clone();
    let scroll = env.scroll;
    scene.adopting(scroll, || TextLayer::new(&mut env.kit.ui, frame, z, items))
}

fn make_icon(env: &mut Env, name: &str, side: u32, ink: Rgba, z: i32, at: Props) -> std::io::Result<Option<ImageLayer>> {
    let scene = env.kit.ui.scene.clone();
    let scroll = env.scroll;
    scene.adopting(scroll, || icon::layer(env.kit, name, side, ink, z, at))
}

fn make_shape(env: &mut Env, w: f32, h: f32, z: i32, shape: Shape, at: Props) -> ShapeLayer {
    let scene = env.kit.ui.scene.clone();
    let scroll = env.scroll;
    scene.adopting(scroll, || scene.shape(w.round().max(1.0) as u32, h.round().max(1.0) as u32, z, shape, at))
}

impl RowArt {
    /// Builds `row` at `top` (the scroll group's space), `w` wide, as `geom` lays it out. `focusable` registers it with the engine.
    pub fn build(env: &mut Env, row: &Row, geom: RowGeom, top: f32, w: f32, id: Id, focusable: bool) -> std::io::Result<RowArt> {
        let rect = Frame::new(0.0, top, w, geom.h);
        let mut art = RowArt {
            id,
            row_id: row.id.clone(),
            kind: row.kind,
            geom,
            rect,
            sig: signature(row),
            focusable,
            last: row.clone(),
            _lit: None,
            selected_wash: None,
            hover_wash: None,
            tile: None,
            _words: None,
            trail_text: None,
            _chevron: None,
            _plate: None,
            _arrows: Vec::new(),
            mark: None,
            trailing: Trailing::None,
            moment: Moment::default(),
            control: None,
            info: None,
            cards: Vec::new(),
        };
        match row.kind {
            Kind::Head => art.build_head(env, row)?,
            Kind::Note => art.build_note(env, row)?,
            Kind::Empty => art.build_empty(env, row)?,
            Kind::Figure => {}
            Kind::Cards => art.build_cards(env, row)?,
            _ => art.build_row(env, row)?,
        }
        Ok(art)
    }

    /// Describes the row's words to the layout lint, as they are placed: `at` is the row's frame on the screen, `panel` the pane under it.
    pub fn feed_lint(&self, kit: &mut Kit, paint: &Paint, sz: &Sizes, row: &Row, at: Frame, lint: &mut crate::kit::lint::Lint, panel: usize) {
        if matches!(self.kind, Kind::Head | Kind::Note | Kind::Empty | Kind::Figure | Kind::Cards) {
            return;
        }
        let g = &self.geom;
        let (tl, tc) = (kit.ty(TypeRole::Label), kit.ty(TypeRole::Caption));
        let (ll, lc) = (line_height(tl), line_height(tc));
        let block = g.label.len().max(1) as f32 * ll + g.detail.len() as f32 * lc;
        let mut y = at.y + ((at.h - block) / 2.0).round();
        let ink = paint.dim(if row.danger { paint.danger_ink } else { paint.ink }, row.enabled || row.info);
        let muted = !(row.enabled || row.info);
        let mut run = |kit: &mut Kit, name: String, text: &str, ty: ui::text::Type, x: f32, y: f32, room: f32, line: f32, ink: Rgba, muted: bool, right: bool| {
            let w = kit.ui.fonts.measure(text, ty).width.ceil().min(room);
            let x = if right { x + room - w } else { x };
            lint.word_at(&name, Frame::new(x, y, w, line), ink, ty.px, muted, Some(panel));
        };
        for (i, l) in g.label.iter().enumerate() {
            run(kit, format!("{} label {i}", row.id), l, tl, at.x + g.text_x, y, g.text_w, ll, ink, muted, false);
            y += ll;
        }
        for (i, l) in g.detail.iter().enumerate() {
            run(kit, format!("{} detail {i}", row.id), l, tc, at.x + g.text_x, y, g.text_w, lc, paint.ink2, true, false);
            y += lc;
        }
        let local_right = at.right() - sz.s6;
        let value_y = at.y + ((at.h - ll) / 2.0).round();
        match row.kind {
            Kind::Value if !g.value.is_empty() => {
                if g.chip_w > 0.0 {
                    run(kit, format!("{} chip", row.id), &g.value, tl, local_right - g.chip_w + sz.s3 + sz.chevron() + sz.s2, value_y, g.value_w, ll, ink, muted, false);
                } else {
                    let room = g.value_w;
                    let x = local_right - g.trail_w - if g.trail_w > 0.0 { sz.s4 } else { 0.0 } - room;
                    run(kit, format!("{} value", row.id), &g.value, tl, x, value_y, room, ll, ink, muted, true);
                }
            }
            Kind::Slider => {
                let text = row.value_text.clone().unwrap_or_else(|| format!("{}{}", row.num.round() as i64, row.unit));
                run(kit, format!("{} number", row.id), &text, tl, local_right - g.value_w, value_y, g.value_w, ll, ink, muted, true);
            }
            _ => {}
        }
    }

    pub fn frame_on_screen(&self, env: &Env) -> Frame {
        Frame::new(env.origin.0 + self.rect.x, env.origin.1 + self.rect.y, self.rect.w, self.rect.h)
    }

    fn build_head(&mut self, env: &mut Env, row: &Row) -> std::io::Result<()> {
        let ty = env.kit.ty(TypeRole::Caption);
        let line = line_height(ty);
        let w = self.rect.w;
        let items = [Item::Text(Run { text: &row.text, ty, colour: caption_ink(&env.paint), x: 0.0, y: (self.rect.h - env.sz.s2 - line).max(0.0), w: w - env.sz.s2, align: Align::Left })];
        let frame = Frame::new(self.rect.x + env.sz.s2, self.rect.y, w - env.sz.s2, self.rect.h);
        self._words = Some(make_text(env, frame, env.z + 4, &items)?);
        Ok(())
    }

    fn build_note(&mut self, env: &mut Env, row: &Row) -> std::io::Result<()> {
        let side = (env.sz.target * 0.36).round() as u32;
        let ink3 = env.paint.ink3;
        self._chevron = make_icon(env, "info", side, ink3, env.z + 3, Props::at(self.rect.x + env.sz.s2, self.rect.y + env.sz.s1))?;
        let ty = env.kit.ty(TypeRole::Caption);
        let line = line_height(ty);
        let items: Vec<Item> = self.geom.label.iter().enumerate().map(|(i, l)| Item::Text(Run { text: l, ty, colour: env.paint.ink2, x: 0.0, y: i as f32 * line, w: self.geom.text_w, align: Align::Left })).collect();
        let _ = row;
        let frame = Frame::new(self.rect.x + self.geom.text_x, self.rect.y, self.geom.text_w, self.rect.h);
        self._words = Some(make_text(env, frame, env.z + 4, &items)?);
        Ok(())
    }

    fn build_empty(&mut self, env: &mut Env, row: &Row) -> std::io::Result<()> {
        let side = 64u32;
        let at = Props::at(self.rect.x + ((self.rect.w - side as f32) / 2.0).round(), self.rect.y + env.sz.s5);
        let icon = if row.icon.is_empty() { "info" } else { &row.icon };
        self.tile = make_tile(env, (icon, None), (true, false), at, side, (side as f32 * 0.55).round() as u32)?;
        let (tl, tc) = (env.kit.ty(TypeRole::Label), env.kit.ty(TypeRole::Caption));
        let (ll, lc) = (line_height(tl), line_height(tc));
        let mut y = env.sz.s5 + side as f32 + env.sz.s3;
        let mut items = Vec::new();
        for l in &self.geom.label {
            items.push(Item::Text(Run { text: l, ty: tl, colour: env.paint.ink, x: 0.0, y, w: self.rect.w, align: Align::Centre }));
            y += ll;
        }
        y += env.sz.s3;
        for l in &self.geom.detail {
            items.push(Item::Text(Run { text: l, ty: tc, colour: env.paint.ink2, x: env.sz.s6, y, w: self.rect.w - 2.0 * env.sz.s6, align: Align::Centre }));
            y += lc;
        }
        self._words = Some(make_text(env, Frame::new(self.rect.x, self.rect.y, self.rect.w, self.rect.h), env.z + 4, &items)?);
        Ok(())
    }

    fn build_row(&mut self, env: &mut Env, row: &Row) -> std::io::Result<()> {
        let radius = env.kit.tokens.radius(Radius::Row);
        let scale = env.kit.tokens.focus.row_scale;
        if self.focusable {
            self._lit = Some(make_plate(env, self.id, self.rect, true, scale, 0.0, radius)?);
        }
        let sz = env.sz;
        // the tile
        let side = sz.tile.round() as u32;
        let at = Props::at(self.rect.x + sz.s6, mid(&self.rect, side as f32));
        self.tile = make_tile(env, (&row.icon, row.face.as_ref()), (row.enabled, row.accent), at, side, sz.glyph)?;
        // the words
        let geom = self.geom.clone();
        let (tl, tc) = (env.kit.ty(TypeRole::Label), env.kit.ty(TypeRole::Caption));
        let (ll, lc) = (line_height(tl), line_height(tc));
        let block = geom.label.len().max(1) as f32 * ll + geom.detail.len() as f32 * lc;
        let top = ((self.rect.h - block) / 2.0).round();
        let paint = env.paint;
        let label_ink = paint.dim(if row.danger { paint.danger_ink } else { paint.ink }, row.enabled || row.info);
        let detail_ink = if row.enabled { paint.ink2 } else { paint.dim(paint.ink, false) };
        let mut items = Vec::new();
        let mut y = top;
        for l in &geom.label {
            items.push(Item::Text(Run { text: l, ty: tl, colour: label_ink, x: 0.0, y, w: geom.text_w, align: Align::Left }));
            y += ll;
        }
        for l in &geom.detail {
            items.push(Item::Text(Run { text: l, ty: tc, colour: detail_ink, x: 0.0, y, w: geom.text_w, align: Align::Left }));
            y += lc;
        }
        if !items.is_empty() {
            let frame = Frame::new(self.rect.x + geom.text_x, self.rect.y, geom.text_w, self.rect.h);
            self._words = Some(make_text(env, frame, env.z + 4, &items)?);
        }
        // the info mark after the last line of the label
        if !row.help.is_empty() {
            let side = (sz.target * 0.4).round() as u32;
            let last_w = geom.label.last().map_or(0.0, |l| env.kit.ui.fonts.measure(l, tl).width.ceil());
            let n = geom.label.len().max(1) as f32;
            let mx = self.rect.x + geom.text_x + last_w + sz.s3;
            let my = self.rect.y + top + (n - 1.0) * ll + ((ll - side as f32) / 2.0).round() - 1.0;
            let pill_w = side + sz.s2 as u32 * 3 + side;
            let scene = env.kit.ui.scene.clone();
            let scroll = env.scroll;
            let z = env.z;
            self.mark = Some(scene.adopting(scroll, || scene.image(pill_w, side + 2, z + 6, Props::at(mx, my)))?);
            self.info = Some(Frame::new(mx - self.rect.x - 8.0, my - self.rect.y - 8.0, side as f32 + 16.0, side as f32 + 18.0));
            self.paint_mark(env, false);
        }
        self.build_trailing(env, row)?;
        self.moment = Moment::default();
        Ok(())
    }

    /// The info mark: the glyph alone, and, while its row has the focus, the glyph and the key that opens it (hold OK).
    fn paint_mark(&mut self, env: &mut Env, focused: bool) {
        let ink2 = env.paint.dim(env.paint.ink2, self.last.enabled || self.last.info);
        let Some(layer) = &mut self.mark else { return };
        let (w, h) = (layer.width() as usize, layer.height() as usize);
        let side = h.saturating_sub(2) as u32;
        let mut canvas = vec![0u8; w * h * 4];
        fn blit(canvas: &mut [u8], w: usize, src: &[u8], ss: u32, x0: usize, y0: usize) {
            for yy in 0..ss as usize {
                for xx in 0..ss as usize {
                    let j = (yy * ss as usize + xx) * 4;
                    let i = ((yy + y0) * w + xx + x0) * 4;
                    if i + 4 <= canvas.len() && src[j + 3] > 0 {
                        canvas[i..i + 4].copy_from_slice(&src[j..j + 4]);
                    }
                }
            }
        }
        if focused {
            let r = h as f32 / 2.0;
            let wash = env.paint.wash_selected;
            let edge = env.paint.edge_raised;
            for yy in 0..h {
                for xx in 0..w {
                    let dx = (xx as f32 + 0.5 - w as f32 / 2.0).abs() - (w as f32 / 2.0 - r);
                    let dy = (yy as f32 + 0.5 - r).abs();
                    let d = (dx.max(0.0).powi(2) + dy.powi(2)).sqrt() - r;
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    let rim = (1.0 - (-d).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                    let i = (yy * w + xx) * 4;
                    for (k, (a, b)) in [(wash.0, edge.0), (wash.1, edge.1), (wash.2, edge.2), (wash.3, edge.3)].into_iter().enumerate() {
                        canvas[i + k] = ((f32::from(a) * (1.0 - rim * 0.5) + f32::from(b) * rim * 0.5) * cov).min(255.0) as u8;
                    }
                }
            }
            let gs = side.saturating_sub(4);
            if let Some(g) = env.kit.icons.draw("info", gs, env.paint.ink) {
                blit(&mut canvas, w, &g, gs, 5, 3);
            }
            let mut key = vec![0u8; (gs * gs * 4) as usize];
            glyph::draw("key-ok-hold", gs, env.paint.ink, &mut key);
            blit(&mut canvas, w, &key, gs, 5 + gs as usize + 8, 3);
        } else if let Some(g) = env.kit.icons.draw("info", side, ink2) {
            blit(&mut canvas, w, &g, side, 0, 1);
        }
        layer.pixels_mut().copy_from_slice(&canvas);
        layer.damage(None);
    }

    fn build_trailing(&mut self, env: &mut Env, row: &Row) -> std::io::Result<()> {
        let sz = env.sz;
        let z = env.z;
        let paint = env.paint;
        let geom = self.geom.clone();
        let rect = self.rect;
        let right = rect.x + rect.w - sz.s6;
        let local_right = rect.w - sz.s6;
        let ink = paint.dim(paint.ink, row.enabled || row.info);
        let (tl, tc) = (env.kit.ty(TypeRole::Label), env.kit.ty(TypeRole::Caption));
        let (ll, lc) = (line_height(tl), line_height(tc));
        let label_y = ((rect.h - ll) / 2.0).round();
        match row.kind {
            Kind::Toggle => {
                let (tw, th) = sz.toggle();
                let at = (right - tw, mid(&rect, th));
                let track = make_shape(env, tw, th, z + 3, pill(paint.track_off, CLEAR, th / 2.0, 0.0), Props::at(at.0, at.1));
                let d = th - 2.0 * sz.s1;
                let thumb = make_shape(env, d, d, z + 4, pill(paint.ink, paint.edge_raised, d / 2.0, 1.0), Props::at(at.0 + sz.s1, at.1 + sz.s1));
                // the tick on the track while it is on: the state is told by the thumb's place and by this mark, not by colour alone
                let tick_side = (th * 0.46).round() as u32;
                let tick = make_icon(env, "check", tick_side, paint.accent_ink, z + 4, Props { visible: false, opacity: 0.0, ..Props::at(at.0 + sz.s3, at.1 + ((th - tick_side as f32) / 2.0).round()) })?;
                self.control = Some(Frame::new(at.0 - rect.x, at.1 - rect.y, tw, th));
                self.trailing = Trailing::Toggle { track, thumb, tick, at, on: None };
            }
            Kind::Seg => {
                let s = geom.seg.expect("a segmented row has its segments measured");
                let total = s.total();
                let at = (right - total, mid(&rect, s.h));
                let radius = env.kit.tokens.radius(Radius::Control);
                let tray = make_shape(env, total, s.h, z + 3, pill(paint.tray(), CLEAR, radius, 0.0), Props::at(at.0, at.1));
                let (sw, sh) = (s.seg_w - 2.0 * sz.s1, s.h - 2.0 * sz.s1);
                let r2 = radius - sz.s1;
                let chosen = make_shape(env, sw, sh, z + 4, pill(paint.seg_selected(), CLEAR, r2, 0.0), HIDDEN);
                let focus = make_shape(env, sw, sh, z + 4, pill(paint.seg_focus(), paint.edge_raised, r2, 1.0), HIDDEN);
                let mark = make_shape(env, sz.s7, sz.s1, z + 6, pill(paint.accent, CLEAR, sz.s1 / 2.0, 0.0), HIDDEN);
                self.control = Some(Frame::new(at.0 - rect.x, at.1 - rect.y, total, s.h));
                self._plate = Some(tray);
                self.trailing = Trailing::Seg { chosen, mark, focus, geom: s, at, index: None, cursor: None, first: 0 };
                self.draw_seg_labels(env, row, 0)?;
            }
            Kind::Slider => {
                let w = sz.target * 5.0;
                let at = (right - geom.value_w - sz.s4 - w, mid(&rect, sz.target));
                let thumb_d = sz.s7;
                let th = sz.s3;
                let track_w = w - thumb_d;
                let track_at = (at.0 + thumb_d / 2.0, at.1 + ((sz.target - th) / 2.0).round());
                let track = make_shape(env, track_w, th, z + 3, pill(paint.track_off, CLEAR, th / 2.0, 0.0), Props::at(track_at.0, track_at.1));
                let fill = make_shape(env, track_w, th, z + 4, pill(paint.accent, CLEAR, th / 2.0, 0.0), Props::at(track_at.0, track_at.1));
                let thumb = (!row.info).then(|| {
                    let dy = at.1 + ((sz.target - thumb_d) / 2.0).round();
                    make_shape(env, thumb_d, thumb_d, z + 5, pill(paint.ink, paint.edge_raised, thumb_d / 2.0, 1.0), Props::at(at.0, dy))
                });
                self.control = Some(Frame::new(at.0 - rect.x, at.1 - rect.y, w, sz.target));
                self._plate = Some(track);
                self.trailing = Trailing::Slider { fill, thumb, at: track_at, w: track_w, thumb_d, shown: None };
                self.draw_trail_text(env, row, &geom)?;
                let _ = (label_y, local_right);
            }
            Kind::Value => {
                let show = row.chevron || row.selected;
                if geom.chip_w > 0.0 {
                    // the answer as a chip: a pill, its glyph, the word (drawn with the row's trailing text)
                    let h = ll + 2.0 * sz.s2;
                    let x = right - geom.chip_w;
                    self._plate = Some(make_shape(env, geom.chip_w, h, z + 3, pill(paint.dim(paint.wash_selected, row.enabled), paint.dim(paint.edge, row.enabled), (h / 2.0).floor(), 1.0), Props::at(x, mid(&rect, h))));
                    let side = sz.chevron().round() as u32;
                    self._chevron = make_icon(env, &row.value_icon, side, paint.dim(paint.ink2, row.enabled), z + 4, Props::at(x + sz.s3, mid(&rect, side as f32)))?;
                } else if show {
                    let side = sz.chevron().round() as u32;
                    let (name, c) = if row.selected { ("check", paint.accent) } else { ("chevron-right", paint.ink2) };
                    self._chevron = make_icon(env, name, side, paint.dim(c, row.enabled), z + 3, Props::at(right - side as f32, mid(&rect, side as f32)))?;
                }
                self.draw_trail_text(env, row, &geom)?;
            }
            Kind::Action => {
                let h = lc + 2.0 * sz.s2;
                let (fill, edge) = if row.danger { (faded(paint.danger, 0.16), faded(paint.danger, 0.5)) } else { (paint.wash_selected, paint.edge) };
                let at = (right - geom.chip_w, mid(&rect, h));
                self._plate = Some(make_shape(env, geom.chip_w, h, z + 3, pill(paint.dim(fill, row.enabled), paint.dim(edge, row.enabled), (h / 2.0).floor(), 1.0), Props::at(at.0, at.1)));
                self.draw_trail_text(env, row, &geom)?;
            }
            Kind::Stepper | Kind::Nudge => {
                let side = if row.kind == Kind::Stepper { sz.chevron() } else { (sz.target * 0.45).round() }.round() as u32;
                let ink2 = paint.dim(paint.ink2, row.enabled);
                let l = make_icon(env, "chevron-left", side, ink2, z + 3, Props::at(right - geom.trail_w, mid(&rect, side as f32)))?;
                let r = make_icon(env, "chevron-right", side, ink2, z + 3, Props::at(right - side as f32, mid(&rect, side as f32)))?;
                self._arrows = [l, r].into_iter().flatten().collect();
                self.draw_trail_text(env, row, &geom)?;
            }
            _ => {}
        }
        let _ = ink;
        Ok(())
    }

    /// The words at the right (a value, a chip's button, the stepper's value, the slider's number).
    fn draw_trail_text(&mut self, env: &mut Env, row: &Row, geom: &RowGeom) -> std::io::Result<()> {
        let sz = env.sz;
        let paint = env.paint;
        let (tl, tc) = (env.kit.ty(TypeRole::Label), env.kit.ty(TypeRole::Caption));
        let (ll, lc) = (line_height(tl), line_height(tc));
        let local_right = self.rect.w - sz.s6;
        let ink = paint.dim(paint.ink, row.enabled || row.info);
        let label_y = ((self.rect.h - ll) / 2.0).round();
        let text: String;
        let item = match row.kind {
            Kind::Value if !geom.value.is_empty() => {
                text = geom.value.clone();
                if geom.chip_w > 0.0 {
                    // a chip's word follows its glyph, left-aligned so the words of the chips form a column
                    let x = local_right - geom.chip_w + sz.s3 + sz.chevron() + sz.s2;
                    Run { text: &text, ty: tl, colour: ink, x, y: label_y, w: geom.value_w, align: Align::Left }
                } else {
                    let x = local_right - geom.trail_w - if geom.trail_w > 0.0 { sz.s4 } else { 0.0 } - geom.value_w;
                    Run { text: &text, ty: tl, colour: ink, x, y: label_y, w: geom.value_w, align: Align::Right }
                }
            }
            Kind::Slider => {
                text = row.value_text.clone().unwrap_or_else(|| format!("{}{}", row.num.round() as i64, row.unit));
                Run { text: &text, ty: tl, colour: ink, x: local_right - geom.value_w, y: label_y, w: geom.value_w, align: Align::Right }
            }
            Kind::Action => {
                let h = lc + 2.0 * sz.s2;
                text = row.button.clone();
                let colour = paint.dim(if row.danger { paint.danger_ink } else { paint.ink }, row.enabled);
                Run { text: &text, ty: tc, colour, x: local_right - geom.chip_w, y: ((self.rect.h - h) / 2.0).round() + sz.s2, w: geom.chip_w, align: Align::Centre }
            }
            Kind::Stepper | Kind::Nudge => {
                let side = if row.kind == Kind::Stepper { sz.chevron() } else { (sz.target * 0.45).round() };
                text = if row.kind == Kind::Stepper { row.index.and_then(|i| row.options.get(i)).cloned().unwrap_or_default() } else { row.value.clone() };
                Run { text: &text, ty: tl, colour: ink, x: local_right - geom.trail_w + side + sz.s3, y: label_y, w: geom.trail_w - 2.0 * side - 2.0 * sz.s3, align: Align::Centre }
            }
            _ => return Ok(()),
        };
        let items = [Item::Text(item)];
        match &mut self.trail_text {
            Some(layer) => layer.draw(&mut env.kit.ui, &items),
            None => {
                let frame = Frame::new(self.rect.x, self.rect.y, self.rect.w, self.rect.h);
                self.trail_text = Some(make_text(env, frame, env.z + 5, &items)?);
            }
        }
        Ok(())
    }

    /// The labels of a segmented tray from the first segment in view.
    fn draw_seg_labels(&mut self, env: &mut Env, row: &Row, first: usize) -> std::io::Result<()> {
        let Trailing::Seg { geom, .. } = &self.trailing else { return Ok(()) };
        let s = *geom;
        let tc = env.kit.ty(TypeRole::Caption);
        let lc = line_height(tc);
        let top = ((self.rect.h - lc) / 2.0).round();
        let ink = env.paint.dim(env.paint.ink, row.enabled || row.info);
        let items: Vec<Item> = row.options.iter().enumerate().skip(first).take(s.in_view).map(|(i, o)| Item::Text(Run { text: o, ty: tc, colour: ink, x: (i - first) as f32 * s.seg_w, y: top, w: s.seg_w, align: Align::Centre })).collect();
        let left = self.rect.w - env.sz.s6 - s.total();
        match &mut self.trail_text {
            Some(layer) => layer.draw(&mut env.kit.ui, &items),
            None => {
                let frame = Frame::new(self.rect.x + left, self.rect.y, s.total(), self.rect.h);
                self.trail_text = Some(make_text(env, frame, env.z + 5, &items)?);
            }
        }
        Ok(())
    }

    fn build_cards(&mut self, env: &mut Env, row: &Row) -> std::io::Result<()> {
        let g = self.geom.cards.expect("a row of cards is measured");
        let sz = env.sz;
        let paint = env.paint;
        let radius = env.kit.tokens.radius(Radius::Control);
        let scale = env.kit.tokens.focus.button_scale;
        let (tl, tc) = (env.kit.ty(TypeRole::Label), env.kit.ty(TypeRole::Caption));
        let (ll, lc) = (line_height(tl), line_height(tc));
        let side = (sz.target * 0.5).round() as u32;
        let check_side = (sz.target * 0.34).round() as u32;
        let mut cards = Vec::new();
        for (i, c) in row.cards.iter().enumerate() {
            let col = if row.strip { i } else { i % row.cols.max(1) };
            let line = if row.strip { 0 } else { i / row.cols.max(1) };
            let x = self.rect.x + g.edge + col as f32 * (g.card_w + g.gap);
            let y = self.rect.y + line as f32 * (g.card_h + g.gap);
            let rect = Frame::new(x, y, g.card_w, g.card_h);
            let id = Id(self.id.0 + 1 + i as u32);
            let lit = make_plate(env, id, rect, self.focusable, scale, env.kit.tokens.focus.card_lift, radius)?;
            let base = make_shape(env, rect.w, rect.h, env.z, pill(paint.wash_hover, paint.edge, radius, 1.0), Props::at(rect.x, rect.y));
            let hover = make_shape(env, rect.w, rect.h, env.z + 2, pill(paint.wash_hover, faded(paint.edge_raised, 0.6), radius, 1.0), HIDDEN);
            let glyph_at = Props::at(rect.x + sz.s5, rect.y + ((rect.h - side as f32) / 2.0).round());
            let glyph = match &c.face {
                Some(f) => {
                    let px = face_px(env, f, side, row.enabled);
                    let scene = env.kit.ui.scene.clone();
                    let (scroll, z) = (env.scroll, env.z);
                    let mut layer = scene.adopting(scroll, || scene.image(side, side, z + 3, glyph_at))?;
                    layer.pixels_mut().copy_from_slice(&px);
                    layer.damage(None);
                    Some(layer)
                }
                None if c.icon.is_empty() => None,
                None => make_icon(env, &c.icon, side, paint.dim(paint.ink2, row.enabled), env.z + 3, glyph_at)?,
            };
            let check = make_icon(env, "check", check_side, paint.accent, env.z + 3, Props { visible: false, opacity: 0.0, ..Props::at(rect.x + rect.w - check_side as f32 - sz.s4, rect.y + sz.s3) })?;
            let tx = sz.s5 + if glyph.is_some() { side as f32 + sz.s4 } else { 0.0 };
            let tw = (g.card_w - tx - sz.s5 - check_side as f32 - sz.s2).max(40.0);
            let ellipsised = |kit: &mut Kit, text: &str, role: TypeRole| {
                let ty = kit.ty(role);
                ui::components::text::ellipsise(&mut kit.ui.fonts, text, ty, tw)
            };
            let title = ellipsised(env.kit, &c.title, TypeRole::Label);
            let sub = if c.sub.is_empty() { String::new() } else { ellipsised(env.kit, &c.sub, TypeRole::Caption) };
            let block = ll + if sub.is_empty() { 0.0 } else { lc };
            let top = ((rect.h - block) / 2.0).round();
            let mut items = vec![Item::Text(Run { text: &title, ty: tl, colour: paint.dim(paint.ink, row.enabled), x: 0.0, y: top, w: tw, align: Align::Left })];
            if !sub.is_empty() {
                items.push(Item::Text(Run { text: &sub, ty: tc, colour: paint.dim(paint.ink2, row.enabled), x: 0.0, y: top + ll, w: tw, align: Align::Left }));
            }
            let text = make_text(env, Frame::new(rect.x + tx, rect.y, tw, rect.h), env.z + 4, &items)?;
            self.cards.push((id, rect));
            cards.push(CardArt { id, frame: rect, icon: c.icon.clone(), face_free: c.face.is_none(), glyph_at, hover, base, _lit: lit, glyph, check, selected: None, _text: text });
        }
        self.trailing = Trailing::Cards(cards);
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------------- in place

    /// Sets what moves: the switch's thumb, the chosen segment, the slider's fill, the chosen card, the tile's accent, the pointer's cue,
    /// the info mark's pill, the wash of a chosen row.
    pub fn update(&mut self, env: &mut Env, row: &Row, moment: Moment, at_once: bool) {
        let sz = env.sz;
        let paint = env.paint;
        let beat = |kit: &Kit, layer: &ui::scene::Layer, to: Props| {
            if at_once { layer.set(to) } else { kit.glide(layer, to, Beat::Label) }
        };
        let hidden_at = |r: &Frame| Props { opacity: 0.0, visible: false, ..Props::at(r.x, r.y) };
        // the tile's colour follows the accent (the rail's current section)
        if self.last.accent != row.accent
            && row.face.is_none()
            && let Some(layer) = &mut self.tile
        {
            let side = layer.width();
            let px = tile_px(env, &row.icon, row.enabled, row.accent, side, sz.glyph);
            let layer = self.tile.as_mut().expect("checked");
            layer.pixels_mut().copy_from_slice(&px);
            layer.damage(None);
        }
        // the washes: a chosen row, the pointer's cue; both give way to the focus
        let want_sel = (row.selected || row.wash) && !moment.focused && !matches!(row.kind, Kind::Head | Kind::Note | Kind::Empty | Kind::Cards);
        if want_sel && self.selected_wash.is_none() {
            let rect = self.rect;
            self.selected_wash = Some(make_wash(env, rect, false));
        }
        if let Some(w) = &self.selected_wash {
            w.set(if want_sel { Props::at(self.rect.x, self.rect.y) } else { hidden_at(&self.rect) });
        }
        let want_hover = moment.hovered && !moment.focused && self.focusable && !matches!(row.kind, Kind::Cards);
        if want_hover && self.hover_wash.is_none() {
            let rect = self.rect;
            self.hover_wash = Some(make_wash(env, rect, true));
        }
        if let Some(w) = &self.hover_wash {
            w.set(if want_hover { Props::at(self.rect.x, self.rect.y) } else { hidden_at(&self.rect) });
        }
        if self.moment.focused != moment.focused && self.mark.is_some() {
            self.paint_mark(env, moment.focused);
        }
        let rect = self.rect;
        let mut relabel: Option<usize> = None;
        match &mut self.trailing {
            Trailing::Toggle { track, thumb, tick, at, on } => {
                let want = row.on;
                if *on != Some(want) {
                    let (tw, th) = sz.toggle();
                    let d = th - 2.0 * sz.s1;
                    if let Some(t) = tick {
                        let side = t.width() as f32;
                        t.set(if want { Props::at(at.0 + sz.s3, at.1 + ((th - side) / 2.0).round()) } else { Props { visible: false, opacity: 0.0, ..Props::at(0.0, 0.0) } });
                    }
                    track.reshape(pill(paint.dim(if want { paint.accent } else { paint.track_off }, row.enabled || row.info), CLEAR, th / 2.0, 0.0));
                    thumb.reshape(pill(paint.dim(if want { paint.accent_ink } else { paint.ink }, row.enabled || row.info), paint.edge_raised, d / 2.0, 1.0));
                    let x = if want { at.0 + tw - d - sz.s1 } else { at.0 + sz.s1 };
                    beat(env.kit, thumb, Props::at(x, at.1 + sz.s1));
                    *on = Some(want);
                }
            }
            Trailing::Seg { chosen, mark, focus, geom, at, index, cursor, first } => {
                let want_idx = row.index;
                let want_cursor = if moment.focused { moment.seg } else { None };
                let anchor = want_cursor.or(want_idx).unwrap_or(0);
                let mut new_first = *first;
                if anchor < new_first {
                    new_first = anchor;
                } else if anchor >= new_first + geom.in_view {
                    new_first = anchor + 1 - geom.in_view;
                }
                let in_view = |i: usize| i >= new_first && i < new_first + geom.in_view;
                let place = |i: usize| Props::at(at.0 + (i - new_first) as f32 * geom.seg_w + sz.s1, at.1 + sz.s1);
                match want_idx.filter(|i| in_view(*i)) {
                    Some(i) => {
                        beat(env.kit, chosen, place(i));
                        let m = place(i);
                        beat(env.kit, mark, Props::at(m.x + ((geom.seg_w - 2.0 * sz.s1 - sz.s7) / 2.0).round(), at.1 + geom.h - sz.s1 - sz.s2));
                    }
                    None => {
                        chosen.set(HIDDEN);
                        mark.set(HIDDEN);
                    }
                }
                match want_cursor.filter(|i| in_view(*i)) {
                    Some(i) => beat(env.kit, focus, place(i)),
                    None => focus.set(HIDDEN),
                }
                if new_first != *first {
                    relabel = Some(new_first);
                }
                *index = want_idx;
                *cursor = want_cursor;
                *first = new_first;
            }
            Trailing::Slider { fill, thumb, at, w, thumb_d, shown } => {
                let frac = if row.max > row.min { (((row.num - row.min) / (row.max - row.min)) as f32).clamp(0.0, 1.0) } else { 0.0 };
                let first_time = shown.is_none();
                let changed = *shown != Some(frac);
                let scale = if moment.focused { env.kit.tokens.focus.button_scale } else { 1.0 };
                if changed {
                    let fw = (*w * frac).round();
                    fill.set(Props { clip: Some(Rect { x: at.0, y: at.1, w: fw, h: sz.s3 }), ..Props::at(at.0, at.1) });
                    *shown = Some(frac);
                }
                if let Some(t) = thumb {
                    let tx = at.0 + *w * frac - *thumb_d / 2.0;
                    let ty = rect.y + ((rect.h - *thumb_d) / 2.0).round();
                    let to = Props { scale, ..Props::at(tx, ty) };
                    if at_once || first_time { t.set(to) } else { env.kit.glide(t, to, Beat::Focus) }
                }
                if changed {
                    let geom = self.geom.clone();
                    let _ = self.draw_trail_text(env, row, &geom);
                }
            }
            Trailing::Cards(cards) => {
                for c in cards.iter_mut() {
                    let i = (c.id.0 - self.id.0 - 1) as usize;
                    let sel = row.index == Some(i);
                    if c.selected != Some(sel) {
                        let radius = env.kit.tokens.radius(Radius::Control);
                        c.base.reshape(pill(if sel { paint.wash_selected } else { paint.wash_hover }, paint.edge, radius, 1.0));
                        if let Some(ck) = &c.check {
                            ck.set(if sel { Props::at(c.frame.x + c.frame.w - ck.width() as f32 - sz.s4, c.frame.y + sz.s3) } else { Props { visible: false, opacity: 0.0, ..Props::at(0.0, 0.0) } });
                        }
                        if c.face_free
                            && let Some(g) = &mut c.glyph
                        {
                            icon::repaint(env.kit, g, &c.icon, paint.dim(if sel { paint.accent } else { paint.ink2 }, row.enabled));
                        }
                        c.selected = Some(sel);
                    }
                    let _ = c.glyph_at;
                }
            }
            Trailing::None => {}
        }
        if let Some(first) = relabel {
            let _ = self.draw_seg_labels(env, row, first);
        }
        self.moment = moment;
        self.last = row.clone();
    }

    /// The hover cue of one card of a row of cards.
    pub fn set_card_hover(&mut self, id: Id, on: bool) {
        if let Trailing::Cards(cards) = &self.trailing {
            for c in cards {
                let me = on && c.id == id;
                c.hover.set(Props { opacity: if me { 1.0 } else { 0.0 }, visible: me, ..Props::at(c.frame.x, c.frame.y) });
            }
        }
    }

    /// Registers the row with the focus engine where it is (`row_frame` is its rectangle on the screen) when it is whole in view, takes
    /// it out when it is not. A row of cards registers its cards, each with its own place.
    pub fn sync_focus(&self, engine: &mut Engine, row_frame: Frame, region: Region, scope: Scope, live: bool) {
        if !self.focusable {
            return;
        }
        if self.kind == Kind::Cards {
            for (id, rect) in &self.cards {
                if live {
                    let f = Frame::new(row_frame.x + rect.x - self.rect.x, row_frame.y + rect.y - self.rect.y, rect.w, rect.h);
                    engine.focus.register(*id, f, region, scope);
                } else {
                    engine.focus.unregister(*id);
                }
            }
        } else if live {
            engine.focus.register(self.id, row_frame, region, scope);
        } else {
            engine.focus.unregister(self.id);
        }
    }

    /// Gives the row back: it leaves the engine's registry and, unless it is about to be made again (`keep_lift`: the lift state of a
    /// row that is rebuilt where it was carries on in the new layers), loses its lift; its layers go with it.
    pub fn release(&self, engine: &mut Engine, keep_lift: bool) {
        engine.focus.unregister(self.id);
        if !keep_lift {
            engine.flow.drop_lift(self.id);
        }
        for (id, _) in &self.cards {
            engine.focus.unregister(*id);
            if !keep_lift {
                engine.flow.drop_lift(*id);
            }
        }
    }
}
