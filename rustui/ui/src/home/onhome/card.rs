//! The card drawn: a heading, and under it the items as raised glass mini cards in the grid the card's room makes ([`super::layout`]).
//!
//! **Each item** is a layer of glass at rest with the glyph, the words and, for a switch, the switch hung beside it. The item in front (and
//! those still settling back) has a [`Lift`] in place of its glass at rest: the glass at rest and the lit glass that comes up with the focus,
//! both on groups that the flowing focus scales and fades. A lift is two of the scene's 96 groups, and Home's dock and bars take most of
//! them, so lifts are made when an item comes to the front and go once it has settled back, never more than [`MAX_LIFTS`]. The glass grows
//! with the focus, the type does not, so every word stays on whole pixels at rest and lifted. The whole card's content hangs under a group
//! that is not scaled with the card's own lift, for the same reason.
//!
//! **What is not drawn.** Home at rest draws nothing: every layer here is retained, every motion is the compositor's (the lift's spring,
//! the scroll's track, a layer's `animate`), and a frame costs this process nothing until a key or a topic changes what is shown. Items
//! are made only for the strides that are in view or about to be.

use super::items::{self, Item, Kind};
use super::layout::{Axis, Grid, Metrics};
use super::paint::{self, Keep};
use super::parts::{self, Look, Sizes};
use super::smart::{STRIP, Smart, Strip};
use crate::home::stills::Stills;
use crate::home::surface::{HIDDEN, Lift, Z_CARDS};
use crate::kit::pane::glass_of;
use crate::home::widgets::{Canvas, PAD_SIDE, PAD_TOP, Pictures, Words};
use crate::kit::focus::{Id, Region};
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::pane::{self, PaneKind};
use crate::kit::picture::{Fit, Picture};
use crate::kit::text::role_style;
use crate::kit::widgets::{Hover, Toggle};
use crate::kit::{Engine, Kit};
use compositor_protocol::{Backdrop, Ease, Edges, Glass, GroupPose, Props, Rect, Rgba, Shadow, Sheen, TintGradient};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::text::ellipsise;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::{Mode, PaletteRole};
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

/// The first focus id of the items: one for each, in the order of the list.
pub const ITEM_ID: u32 = 20_000;
/// The scroller's region.
pub const REGION: Region = Region(ITEM_ID);
/// The most items made at once: the window of the view and, while the view moves, the one it came from.
const MAX_BUILT: usize = 24;
/// How long the items of the window the view has left stay after it moved: the spring settles in about 300 ms.
const LINGER_MS: u64 = 500;

const Z_PANE: i32 = Z_CARDS + 3;
const Z_ART: i32 = Z_CARDS + 6;
const Z_SWITCH: i32 = Z_CARDS + 8;
const Z_WORDS: i32 = Z_CARDS + 10;
const Z_CUE: i32 = Z_CARDS + 12;
const Z_STRIP: i32 = Z_CARDS + 20;

/// How far the glass of the item in front grows past its cell, at each side of its longer side, and how far it rises: together they stay
/// inside the gutter, so that a lifted item never covers its neighbour and the clip at the end of a row never cuts it.
const LIFT_GROW: f32 = 9.0;
const LIFT_RISE: f32 = 4.0;

/// The scale the glass of an item in a cell of `w` by `h` lifts to: [`LIFT_GROW`] on its longer side (a small item grows by a tenth, a large one
/// by less, as a card's lift is less than a tile's).
fn lift_scale(w: f32, h: f32) -> f32 {
    1.0 + 2.0 * LIFT_GROW / w.max(h).max(1.0)
}

/// The look of the glass of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glassy {
    Rest,
    /// In front: raised.
    Lit,
    /// In front and under the finger: it sinks back a little.
    Pressed,
}

fn white(alpha: f32) -> Rgba {
    let v = (255.0 * alpha).round() as u8;
    Rgba(v, v, v, v)
}

fn black(alpha: f32) -> Rgba {
    Rgba(0, 0, 0, (255.0 * alpha).round() as u8)
}

/// The lit glass of an item that is in front, which is drawn over its glass at rest: the focus raises the item a clear step, so that it reads
/// from across a room. It adds light, an edge and a lip that catch it, a stronger sheen and a shadow that says the item floats. Under the
/// finger the shadow draws in and the light dims.
///
/// The light theme cannot lift by brightness alone: its card is already a pale veil, so a white wash is a slab (host render: a lit item of 241
/// over items of 226 and a card of 210). There the item is glass lit from above, white at its top and almost clear at its foot, with a rim
/// and a short shadow; the items at rest are thinner ([`LIGHT_REST_VEIL`]) so that the lit one has something to be brighter than.
fn raised(kit: &Kit, rest: Glass, pressed: bool) -> Glass {
    let dark = kit.mode == Mode::Dark;
    let (wash, top, bottom, edge, lip, sheen, far, near) = if dark {
        (0.15, 0.15, 0.15, white(0.46), white(0.32), Sheen { peak: 0.26, floor: 0.07, start: 0.30, end: 0.62 }, Shadow { offset_y: 20.0, blur: 44.0, colour: black(0.55) }, Shadow { offset_y: 4.0, blur: 10.0, colour: black(0.40) })
    } else {
        (0.20, 0.46, 0.04, white(0.98), white(0.98), Sheen { peak: 0.22, floor: 0.0, start: 0.18, end: 0.58 }, Shadow { offset_y: 12.0, blur: 28.0, colour: black(0.20) }, Shadow { offset_y: 3.0, blur: 8.0, colour: black(0.16) })
    };
    let dim = if pressed { 0.6 } else { 1.0 };
    let tint = white(wash * dim);
    let tint_gradient = (!dark).then(|| TintGradient { top: white(top * dim), bottom: white(bottom * dim) });
    let shadows = if pressed {
        [Some(Shadow { offset_y: 6.0, blur: 14.0, colour: black(if dark { 0.34 } else { 0.18 }) }), None]
    } else {
        [Some(far), Some(near)]
    };
    Glass { tint, tint_gradient, edge, edge_width: 1.5, highlight: lip, sheen: Some(sheen), shadows, ..rest }
}

/// How much of the card glass's veil an item keeps at rest in the light theme: the same veil on the card's own would read as a lighter card in
/// a card. At 0.6 the items sit a step above the card (host render: 214 on 210) and the lit item clears them by about 20.
const LIGHT_REST_VEIL: f32 = 0.6;

fn thinned(c: Rgba, k: f32) -> Rgba {
    let by = |v: u8| (v as f32 * k).round() as u8;
    Rgba(by(c.0), by(c.1), by(c.2), by(c.3))
}

/// The glass of an item: a mini card, nested on the card's own glass (it has no backdrop of its own: the card's is behind it), in the
/// Lucent card material at the tile radius; raised when it is in front ([`raised`]).
pub fn item_glass(kit: &Kit, h: f32, look: Glassy, gone: bool) -> Glass {
    let mut g = glass_of(kit, PaneKind::Card, h, false, false);
    g.corner_radius = kit.tokens.radius(Radius::Tile).min(h / 2.0);
    g.backdrop = Backdrop::None;
    if kit.mode == Mode::Light {
        g.tint = thinned(g.tint, LIGHT_REST_VEIL);
        if let Some(t) = &mut g.tint_gradient {
            *t = TintGradient { top: thinned(t.top, LIGHT_REST_VEIL), bottom: thinned(t.bottom, LIGHT_REST_VEIL) };
        }
    }
    if look != Glassy::Rest {
        g = pane::decorate(kit, PaneKind::Card, raised(kit, g, look == Glassy::Pressed), false);
    }
    if gone {
        // a thing that cannot be reached lies flatter: no shadow
        g.shadows = [None, None];
    }
    g
}

fn rgb_of(c: Rgba) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// What the kit measures of the type: the card's metrics.
pub fn metrics_of(kit: &Kit) -> Metrics {
    let lines = [kit.line(TypeRole::Label), kit.line(TypeRole::Caption), kit.line(TypeRole::Title), kit.line(TypeRole::Body), kit.line(TypeRole::Display)];
    Metrics::new(kit.space(4), kit.space(4), kit.space(3), lines, kit.text_scale)
}

fn tone_colour(kit: &Kit, tone: &str) -> Rgba {
    let look = &kit.ui.look;
    match tone {
        "on" => look.accent(),
        "warn" => look.colour(PaletteRole::Warning),
        "alert" => look.colour(PaletteRole::Danger),
        "ok" => look.colour(PaletteRole::Positive),
        "info" | "warning" => look.colour(PaletteRole::Info),
        _ => look.ink2(),
    }
}

// ---- the parts of one item

/// What is drawn of an item, to tell what a change of the topic changes.
#[derive(Clone, Debug, PartialEq)]
struct Drawn {
    look: Look,
    w: u32,
    h: u32,
    name: String,
    words: String,
    icon: String,
    gone: bool,
    on: bool,
    tone: String,
    still: u64,
    lamp: Option<([u8; 3], u8)>,
    /// A notice said on the item itself, when the card has no heading to say it in.
    said: Option<String>,
}

/// One colour of a lit lamp: the picture, and where it rests when it is fully in.
struct Lit {
    layer: ImageLayer,
    at: Props,
}

struct ItemView {
    id: Id,
    key: String,
    /// The glass at rest: one layer and no group, so that an item costs the scene its layers and none of its groups (it holds 96, and
    /// Home's dock and bars take most of them). The lift stands in its place while the item is in front and until it has settled back.
    flat: GlassLayer,
    lift: Option<Lift>,
    /// When the item stopped being in front: its lift goes once it has settled.
    left_ms: Option<u64>,
    /// The cell, in the scroller's space, the view at its first stride.
    rect: Frame,
    parts: parts::Parts,
    drawn: Option<Drawn>,
    glyph: Option<ImageLayer>,
    still: Option<ImageLayer>,
    lit: Vec<Lit>,
    /// Layers that were crossfaded out: dropped once they are gone.
    retired: Vec<(ImageLayer, u64)>,
    words: Words,
    toggle: Option<Toggle>,
    pressed: bool,
    cue: Option<ShapeLayer>,
}

struct Built {
    _viewport: Group,
    scroller: Group,
    hover: Hover,
    views: Vec<ItemView>,
    /// The structure these were made for: the grid and the items' ids and sizes.
    shape: String,
    /// The first stride of the view the scroller was last sent to.
    off: usize,
    moved_ms: u64,
    /// The previous offset, whose items are kept while the view settles.
    from: usize,
    focus: Option<Id>,
    pressed: Option<usize>,
    hovered: Option<usize>,
}

/// The note a card with no item makes: why there is none.
struct NoteView {
    _pane: GlassLayer,
    _art: ImageLayer,
    words: Words,
    key: String,
}

/// The item menu: a strip of buttons over the item.
struct StripView {
    _pane: GlassLayer,
    marker: ShapeLayer,
    art: ImageLayer,
    frame: Frame,
    at: Option<Strip>,
    key: String,
}

/// The one quiet glyph of the heading: the key that opens the menu of the item in front.
struct Quiet {
    layer: ImageLayer,
    at: Props,
}

pub struct Card {
    title: String,
    pub heading: Words,
    menu_hint: Option<Quiet>,
    w: f32,
    h: f32,
    at: Props,
    built: Option<Built>,
    note: Option<NoteView>,
    strip: Option<StripView>,
    /// What the heading said last, to say it again only when it changes: the notice, whether there is room, whether the menu glyph shows.
    said: (String, bool, bool),
}

/// What the card is told each time it is drawn.
pub struct View<'a> {
    pub smart: &'a Smart,
    pub topic: &'a Value,
    pub entered: bool,
    pub notice: Option<&'a str>,
    pub hold_ms: u64,
}

pub struct Cx<'a> {
    pub kit: &'a mut Kit,
    pub engine: &'a mut Engine,
    pub lift: &'a Lift,
    pub pictures: &'a mut Pictures,
    pub stills: &'a Stills,
    pub now_ms: u64,
    pub now_ns: u64,
}

fn other(e: ui::error::UiError) -> std::io::Error {
    std::io::Error::other(format!("{e}"))
}

impl Card {
    pub fn new(kit: &mut Kit, w: f32, h: f32, z: i32, entry: &Value) -> std::io::Result<Self> {
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let cap_r = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
        let lh = kit.line(TypeRole::Label);
        let title = entry.get("title").and_then(Value::as_str).unwrap_or("Smart home").to_owned();
        let lines = vec![(Frame::new(0.0, 0.0, (w * 0.45).floor(), lh), label, title.clone()), (Frame::new((w * 0.45).floor(), 0.0, w - (w * 0.45).floor(), lh), cap_r, String::new())];
        let heading = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 2, lines)?;
        Ok(Self { title, heading, menu_hint: None, w, h, at: HIDDEN, built: None, note: None, strip: None, said: (String::new(), true, false) })
    }

    pub fn show(&mut self, at: Props) {
        self.at = at;
        self.heading.set_props(at);
    }

    pub fn is_glyph_run(&self) -> bool {
        self.heading.is_glyph_run()
    }

    /// How many items are in view.
    pub fn visible(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.views.len())
    }

    /// The cameras the card draws, with the size of the still each needs: `(id, w, h, online)`, for the stills the model asks for.
    pub fn camera_rows(&self, smart: &Smart) -> Vec<(String, u32, u32, bool)> {
        let plan = smart.plan();
        let (cell_w, cell_h) = plan.grid.cell;
        smart
            .items()
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind == Kind::Camera)
            .filter_map(|(n, i)| {
                let size = plan.placed.get(n)?.size;
                let (cols, rows) = size.cells();
                let (w, h) = (cols as f32 * cell_w + (cols as f32 - 1.0) * plan.grid.gap, rows as f32 * cell_h + (rows as f32 - 1.0) * plan.grid.gap);
                Some((items::camera_of(&i.id)?.to_owned(), w as u32, h as u32, !i.gone))
            })
            .collect()
    }

    // ---- drawing

    /// Draws what the model says: the heading, the items in view and the one in front, the item menu, or the note when there is no item.
    pub fn sync(&mut self, cx: &mut Cx, v: &View) -> std::io::Result<()> {
        let items = v.smart.items();
        let plan = v.smart.plan();
        if items.is_empty() || plan.grid.cols == 0 {
            self.drop_built(cx);
            self.strip = None;
            self.draw_note(cx, v)?;
            self.heading_words(cx, v, false);
            return Ok(());
        }
        self.note = None;
        let shape = shape_of(v.smart);
        if self.built.as_ref().is_none_or(|b| b.shape != shape) {
            self.rebuild(cx, v, shape)?;
        }
        self.items(cx, v)?;
        self.focus(cx, v)?;
        self.menu(cx, v)?;
        self.heading_words(cx, v, plan.grid.heading.is_some());
        Ok(())
    }

    fn drop_built(&mut self, cx: &mut Cx) {
        if let Some(b) = self.built.take() {
            for view in b.views {
                cx.engine.flow.drop_lift(view.id);
            }
            cx.engine.flow.drop_scroller(REGION);
        }
    }

    /// The items' lifts go at once (two groups each), but the one under a finger: Home is hidden or a panel over it needs the room. The item in
    /// front lifts again when the card is next drawn.
    pub fn shed(&mut self, engine: &mut Engine) {
        let Some(b) = self.built.as_mut() else { return };
        for view in b.views.iter_mut().filter(|v| !v.pressed) {
            lower(engine, view);
        }
        b.focus = None;
    }

    /// The structure: the viewport that clips, the scroller that moves under it, the track that moves it. The views are made by [`Self::items`].
    fn rebuild(&mut self, cx: &mut Cx, v: &View, shape: String) -> std::io::Result<()> {
        self.drop_built(cx);
        let grid = v.smart.plan().grid;
        let kept = cx.engine.flow.kept(cx.lift.id).ok_or_else(|| std::io::Error::other("the card's kept group"))?.id();
        let origin = GroupPose::at(PAD_SIDE, PAD_TOP);
        let viewport = cx.kit.ui.scene.group(Some(kept), origin).map_err(other)?;
        let (clip, feather) = viewport_clip(&grid, v.smart.plan().peek(), self.w);
        viewport.set_clip_feathered(Some(clip), feather);
        let scroller = cx.kit.ui.scene.group(Some(viewport.id()), GroupPose::IDENTITY).map_err(other)?;
        let (sx, sy) = grid.stride();
        let extent = v.smart.plan().max_offset() as f32 * if grid.axis == Axis::Horizontal { sx } else { sy };
        cx.engine.flow.scroller(&cx.kit.ui.scene, REGION, &scroller, GroupPose::IDENTITY, grid.axis == Axis::Horizontal, extent).map_err(other)?;
        let off = v.smart.offset();
        let _ = cx.engine.flow.scroll_snap(REGION, off as f32 * if grid.axis == Axis::Horizontal { sx } else { sy });
        self.built = Some(Built { _viewport: viewport, scroller, hover: Hover::new(Z_CUE), views: Vec::new(), shape, off, moved_ms: 0, from: off, focus: None, pressed: None, hovered: None });
        Ok(())
    }

    /// The views of the items in the window, made when they come into it and dropped when they have gone from it, and what they say.
    fn items(&mut self, cx: &mut Cx, v: &View) -> std::io::Result<()> {
        let plan = v.smart.plan();
        let grid = plan.grid;
        let Some(built) = self.built.as_mut() else { return Ok(()) };
        let off = v.smart.offset();
        if off != built.off {
            built.from = built.off;
            built.off = off;
            built.moved_ms = cx.now_ms;
            let stride = if grid.axis == Axis::Horizontal { grid.stride().0 } else { grid.stride().1 };
            let _ = cx.engine.flow.scroll_to(REGION, off as f32 * stride);
        }
        let mut want = plan.window(off);
        if cx.now_ms < built.moved_ms + LINGER_MS {
            for i in plan.window(built.from) {
                if !want.contains(&i) {
                    want.push(i);
                }
            }
        }
        want.truncate(MAX_BUILT);
        let ids: Vec<String> = want.iter().map(|&i| v.smart.items()[i].id.clone()).collect();
        // views for items that are not wanted go
        let (keep, gone): (Vec<ItemView>, Vec<ItemView>) = std::mem::take(&mut built.views).into_iter().partition(|view| ids.contains(&view.key));
        for view in gone {
            cx.engine.flow.drop_lift(view.id);
        }
        built.views = keep;
        let scene = cx.kit.ui.scene.clone();
        let Built { scroller, views, hover, hovered: was_hovered, pressed: was_pressed, focus: front, .. } = built;
        for &i in &want {
            let item = &v.smart.items()[i];
            if views.iter().any(|view| view.key == item.id) {
                continue;
            }
            let Some(rect) = plan.rect(i, 0) else { continue };
            let look = parts::look_of(plan.placed[i].size, grid.style);
            let view = make_view(cx, scroller, &scene, item, i, rect, look)?;
            views.push(view);
        }
        for view in views.iter_mut() {
            let Some(i) = v.smart.items().iter().position(|it| it.id == view.key) else { continue };
            let item = v.smart.items()[i].clone();
            let look = parts::look_of(plan.placed[i].size, grid.style);
            let Some(rect) = plan.rect(i, 0) else { continue };
            // a cell that moved or changed its size (the grid was made again for the same items): its glass goes there
            if view.rect != rect {
                view.rect = rect;
                let rest = GroupPose { x: rect.x, y: rect.y, pivot_x: rect.w / 2.0, pivot_y: rect.h / 2.0, ..GroupPose::IDENTITY };
                let _ = cx.engine.flow.place(view.id, rest);
                if view.lift.is_none() {
                    view.flat.set(Props::at(rect.x, rect.y));
                }
            }
            update_view(cx, scroller, view, &item, look, v)?;
        }
        // the lift of an item that has settled back goes: the glass at rest looks the same
        for view in views.iter_mut() {
            if view.lift.is_some() && Some(view.id) != *front && !view.pressed && view.left_ms.is_some_and(|at| cx.now_ms >= at + SETTLE_MS) {
                lower(cx.engine, view);
            }
        }
        // the pointer's cue and the press
        let hovered = v.smart.hovered();
        if *was_hovered != hovered {
            *was_hovered = hovered;
            let target = hovered.and_then(|h| plan.rect(h, 0)).map(|r| (r, cx.kit.tokens.radius(Radius::Tile).min(r.h / 2.0)));
            hover.show(cx.kit, scroller, target);
        }
        let pressed = v.smart.pressed().map(|(i, _)| i);
        if *was_pressed != pressed {
            for view in views.iter_mut() {
                let is = pressed.is_some_and(|p| v.smart.items().get(p).is_some_and(|it| it.id == view.key));
                if is != view.pressed {
                    let holds = v.smart.items().iter().find(|it| it.id == view.key).is_some_and(|it| it.hold != items::Hold::Nothing);
                    press_view(cx, scroller, view, is, holds, v.hold_ms)?;
                }
            }
            *was_pressed = pressed;
        }
        Ok(())
    }

    /// The item in front lifts, the one that was lifts back: the flowing focus, from wherever each is. The lift of the one that comes to the
    /// front is made now, and the one that was in front keeps its own until it has settled back.
    fn focus(&mut self, cx: &mut Cx, v: &View) -> std::io::Result<()> {
        let Some(built) = self.built.as_mut() else { return Ok(()) };
        let want = v.entered.then(|| v.smart.focused()).flatten().and_then(|i| built.views.iter().find(|view| view.key == v.smart.items()[i].id).map(|view| view.id));
        if want != built.focus {
            let Built { scroller, views, focus, .. } = built;
            if let Some(view) = want.and_then(|id| views.iter_mut().find(|view| view.id == id)) {
                raise(cx, scroller, view)?;
            }
            if let Some(view) = focus.and_then(|id| views.iter_mut().find(|view| view.id == id)) {
                view.left_ms = Some(cx.now_ms);
            }
            trim(cx, views, want);
            let _ = cx.engine.flow.go(*focus, want, cx.now_ns);
            *focus = want;
        }
        Ok(())
    }

    // ---- the heading

    /// The glyph of the Menu key, small and quiet, with its drawing ending where the card's content does and centred on the title's line.
    fn menu_glyph(&mut self, cx: &mut Cx) -> std::io::Result<()> {
        if self.menu_hint.is_some() {
            return Ok(());
        }
        let kit = &mut *cx.kit;
        let kept = cx.engine.flow.kept(cx.lift.id).ok_or_else(|| std::io::Error::other("the card's kept group"))?;
        let scene = kit.ui.scene.clone();
        let line = kit.line(TypeRole::Label);
        let side = (line * 0.8).round();
        let (x, y) = (PAD_SIDE + self.w - side + parts::optical(side), PAD_TOP + ((line - side) / 2.0).round());
        let at = Props::at(x, y);
        let colour = kit.ui.look.ink3();
        let mut layer = scene.adopting(kept, || kit.ui.scene.image(side as u32, side as u32, Z_CARDS + 14, Props { visible: false, opacity: 0.0, ..at }))?;
        let mut px = vec![0u8; (side * side * 4.0) as usize];
        glyph::draw("key-menu", side as u32, colour, &mut px);
        layer.pixels_mut().copy_from_slice(&px);
        layer.damage(None);
        self.menu_hint = Some(Quiet { layer, at });
        Ok(())
    }

    /// The heading: the card's title; at its right the notice, or while an item that has a menu is in front the one glyph of the key that opens
    /// it. Nothing when the card has no room for a heading.
    fn heading_words(&mut self, cx: &mut Cx, v: &View, room: bool) {
        let (ink, ink2) = (cx.kit.ui.look.ink(), cx.kit.ui.look.ink2());
        let notice = v.notice.filter(|_| room).unwrap_or("");
        let in_front = v.smart.focused().and_then(|i| v.smart.items().get(i));
        let menu = room && v.entered && notice.is_empty() && v.smart.arrange.is_none() && in_front.is_some_and(|i| i.kind != Kind::Camera);
        if menu && self.menu_hint.is_none() {
            let _ = self.menu_glyph(cx);
        }
        let key = (notice.to_owned(), room, menu);
        if key == self.said {
            return;
        }
        self.said = key;
        let title = if room { self.title.clone() } else { String::new() };
        let caption = cx.kit.ty(TypeRole::Caption);
        let right = if room { ellipsise(&mut cx.kit.ui.fonts, notice, caption, self.w * 0.55) } else { String::new() };
        self.heading.set_many(cx.kit, &[(0, title.as_str(), ink), (1, right.as_str(), if notice.is_empty() { ink2 } else { ink })]);
        if let Some(q) = &self.menu_hint {
            q.layer.set(Props { visible: menu, opacity: if menu { 1.0 } else { 0.0 }, ..q.at });
        }
    }

    // ---- the note

    /// The card with no item: one plate that says why, with its glyph (the helper's own words).
    fn draw_note(&mut self, cx: &mut Cx, v: &View) -> std::io::Result<()> {
        let (title, sub, tone) = super::note_of(v.topic);
        let key = format!("{title}|{sub}|{tone}");
        if self.note.as_ref().is_some_and(|n| n.key == key) {
            return Ok(());
        }
        let kit = &mut *cx.kit;
        let m = metrics_of(kit);
        let room = (self.h - m.heading_h()).max(m.mini_h());
        let height = room.min((m.compact_h() * 1.0).max(m.mini_h())).floor();
        let top = (m.heading_h() + ((self.h - m.heading_h() - height) / 2.0).max(0.0)).floor();
        let frame = Frame::new(0.0, top, self.w.floor(), height);
        let colour = tone_colour(kit, tone);
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let kept = cx.engine.flow.kept(cx.lift.id).ok_or_else(|| std::io::Error::other("the card's kept group"))?;
        let scene = kit.ui.scene.clone();
        let pane = scene.adopting(kept, || kit.ui.scene.glass(frame.w as u32, frame.h as u32, Z_PANE, item_glass(kit, frame.h, Glassy::Rest, false), Props::at(PAD_SIDE + frame.x, PAD_TOP + frame.y)));
        let g = m.glyph.min(frame.h - 2.0 * m.pad_y);
        let glyph_x = m.pad_x;
        let mut art = scene.adopting(kept, || kit.ui.scene.image(frame.w as u32, frame.h as u32, Z_ART, Props::at(PAD_SIDE + frame.x, PAD_TOP + frame.y)))?;
        art.clear();
        let (aw, ah) = (art.width(), art.height());
        {
            let chevron_side = (m.caption * 0.8).round() as u32;
            let mut canvas = Canvas { px: art.pixels_mut(), w: aw, h: ah };
            if let Some(px) = kit.icons.draw("house-wifi", g as u32, colour).or_else(|| cx.pictures.get("base/house-wifi").map(|p| p.fitted(g as u32, g as u32, Fit::Contain).rgba)) {
                let pic = Picture { width: g as u32, height: g as u32, rgba: px };
                canvas.picture(&pic, glyph_x as i32, ((frame.h - g) / 2.0).round() as i32, g as u32, g as u32);
            }
            if let Some(px) = kit.icons.draw("chevron-right", chevron_side, ink2) {
                let pic = Picture { width: chevron_side, height: chevron_side, rgba: px };
                canvas.picture(&pic, (frame.w - m.pad_x) as i32 - chevron_side as i32, ((frame.h - chevron_side as f32) / 2.0).round() as i32, chevron_side, chevron_side);
            }
        }
        art.damage(None);
        let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let caption = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let x = glyph_x + g + 12.0;
        let tw = (frame.w - x - m.pad_x - 28.0).max(20.0);
        let y = ((frame.h - m.label - m.caption) / 2.0).floor();
        let lines = vec![(Frame::new(x, y, tw, m.label), label, ellipsise(&mut kit.ui.fonts, &title, label.ty, tw)), (Frame::new(x, y + m.label, tw, m.caption), caption, ellipsise(&mut kit.ui.fonts, &sub, caption.ty, tw))];
        let mut words = scene.adopting(kept, || Words::new(kit, Frame::new(0.0, 0.0, frame.w, frame.h), Z_WORDS, lines))?;
        words.set_props(Props::at(PAD_SIDE + frame.x, PAD_TOP + frame.y));
        self.note = Some(NoteView { _pane: pane, _art: art, words, key });
        Ok(())
    }

    // ---- the item menu

    fn menu(&mut self, cx: &mut Cx, v: &View) -> std::io::Result<()> {
        let Some(a) = v.smart.arrange.as_ref().filter(|a| !a.moving) else {
            self.strip = None;
            return Ok(());
        };
        let Some(i) = v.smart.items().iter().position(|it| it.id == a.id) else {
            self.strip = None;
            return Ok(());
        };
        let key = format!("{}|{:?}", a.id, a.at);
        if self.strip.as_ref().is_some_and(|s| s.key == key) {
            return Ok(());
        }
        let plan = v.smart.plan();
        let Some(cell) = plan.rect(i, v.smart.offset()) else { return Ok(()) };
        let kit = &mut *cx.kit;
        let m = metrics_of(kit);
        let b = (48.0 * kit.text_scale.clamp(1.0, 1.5)).round();
        let (gap, pad) = (kit.space(2), kit.space(2));
        let wide = kit.space(4);
        let width = 5.0 * b + 3.0 * gap + wide + 2.0 * pad;
        let height = b + 2.0 * pad;
        // above the item, centred on it, inside the card's content; below it when there is no room above
        let x = (cell.x + (cell.w - width) / 2.0).round().clamp(-PAD_SIDE + 8.0, self.w + PAD_SIDE - 8.0 - width);
        let above = cell.y - height - 8.0;
        let y = if above >= -PAD_TOP + 8.0 { above } else { cell.bottom() + 8.0 }.round();
        let frame = Frame::new(x, y, width, height);
        let reuse = self.strip.as_ref().is_some_and(|s| s.frame == frame);
        let kept = cx.engine.flow.kept(cx.lift.id).ok_or_else(|| std::io::Error::other("the card's kept group"))?;
        let scene = kit.ui.scene.clone();
        let (ink, accent_ink) = (kit.ui.look.ink(), kit.ui.look.accent_ink());
        let slot = |n: usize| pad + n as f32 * (b + gap) + if n >= 3 { wide - gap } else { 0.0 };
        let at = STRIP.iter().position(|s| *s == a.at).unwrap_or(0);
        if !reuse {
            let pane = scene.adopting(kept, || kit.ui.scene.glass(width as u32, height as u32, Z_STRIP, pane::glass(kit, PaneKind::Pill, height, false), Props::at(PAD_SIDE + x, PAD_TOP + y)));
            let marker = scene.adopting(kept, || kit.ui.scene.shape(b as u32, b as u32, Z_STRIP + 1, plate::solid(kit.ui.look.accent(), b / 2.0), Props::at(PAD_SIDE + x + slot(at), PAD_TOP + y + pad)));
            let art = scene.adopting(kept, || kit.ui.scene.image(width as u32, height as u32, Z_STRIP + 2, Props::at(PAD_SIDE + x, PAD_TOP + y)))?;
            self.strip = Some(StripView { _pane: pane, marker, art, frame, at: None, key: String::new() });
        }
        let Some(s) = self.strip.as_mut() else { return Ok(()) };
        let to = Props::at(PAD_SIDE + x + slot(at), PAD_TOP + y + pad);
        let ms = kit.beat_ms(Beat::Label);
        if reuse && s.at.is_some() && ms > 0 {
            s.marker.animate(to, ms, Ease::Out);
        } else {
            s.marker.set(to);
        }
        s.at = Some(a.at);
        s.art.clear();
        let (aw, ah) = (s.art.width(), s.art.height());
        {
            let mut canvas = Canvas { px: s.art.pixels_mut(), w: aw, h: ah };
            let side = (b * 0.6).round() as u32;
            for (n, strip) in STRIP.iter().enumerate() {
                let colour = if n == at { accent_ink } else { ink };
                if let Some(px) = kit.icons.draw(strip.glyph(), side, colour) {
                    let pic = Picture { width: side, height: side, rgba: px };
                    canvas.picture(&pic, (slot(n) + (b - side as f32) / 2.0).round() as i32, (pad + (b - side as f32) / 2.0).round() as i32, side, side);
                }
            }
        }
        s.art.damage(None);
        s.key = key;
        let _ = m;
        Ok(())
    }

    /// The lint's view of the card: the heading, and the words of each item in view, in its cell.
    #[allow(clippy::too_many_arguments)]
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, name: &str, card: Frame, off: (f32, f32), bounds: Frame, paint: Rgba) {
        let origin = (card.x + PAD_SIDE, card.y + PAD_TOP);
        for view in self.built.iter().flat_map(|b| &b.views) {
            let cell = Frame::new(origin.0 + view.rect.x - off.0, origin.1 + view.rect.y - off.1, view.rect.w, view.rect.h);
            // a neighbour that peeks in past the last whole stride, faded, is not read, and one scrolled out is not drawn
            if cell.x < origin.0 + bounds.x - 1.0 || cell.y < origin.1 + bounds.y - 1.0 || cell.right() > origin.0 + bounds.right() + 1.0 || cell.bottom() > origin.1 + bounds.bottom() + 1.0 {
                continue;
            }
            // a camera's picture is under its words: they are read on the plate, whatever the picture is
            let shown = if view.still.is_some() && view.drawn.as_ref().is_some_and(|d| d.still != 0) { plate_for(kit) } else { paint };
            let panel = lint.panel(&format!("{name}.{}", view.key), cell, shown, true);
            let g = view.parts.glyph;
            lint.word_at(&format!("{name}.{}.glyph", view.key), Frame::new(cell.x + g.x, cell.y + g.y, g.w, g.h), kit.ui.look.ink2(), g.h as u32, true, Some(panel));
            view.words.lint(kit, lint, &format!("{name}.{}", view.key), (origin.0 - off.0, origin.1 - off.1), Some(panel));
        }
        if let Some(n) = &self.note {
            n.words.lint(kit, lint, &format!("{name}.note"), (card.x, card.y), None);
        }
    }

    /// The items' lifted plates, for the lint's check that no clip squares one off.
    pub fn plates(&self, engine: &Engine, at_ns: u64) -> Vec<(String, Frame, Option<Frame>)> {
        let Some(b) = &self.built else { return Vec::new() };
        b.views.iter().filter_map(|view| engine.flow.lifted_plate(view.id, at_ns).map(|(f, c)| (view.key.clone(), f, c))).collect()
    }

    pub fn view_count(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.views.len())
    }
}

/// The clip of the viewport the items scroll under, and how it fades. Across the scroll it leaves a margin for a lifted item's growth and its
/// shadow. Along it the stride before the view ends a gutter short of the clip, so that none of it ever shows (a sliver of a tile is
/// worse than none); at the far end the clip reaches `peek` past the next gutter and fades over all of it, so that the next stride shows as
/// a tile that is cut off, which reads as more to come. Without a peek that end is as tight as the near one.
fn viewport_clip(grid: &Grid, peek: f32, w: f32) -> (Rect, Edges) {
    let (mx, my) = (PAD_SIDE - 8.0, PAD_TOP - 8.0);
    let b = grid.bounds();
    let near = grid.gap - 1.0;
    let (far, fade) = if peek > 0.0 { (grid.gap + peek, peek) } else { (near, near) };
    match grid.axis {
        Axis::Horizontal => {
            // above the first row there is the heading, a gutter clear of it
            let top = b.y - if grid.heading.is_some() { near } else { my };
            (Rect { x: b.x - near, y: top, w: b.w + near + far, h: b.bottom() + my - top }, Edges::new(near, fade, 0.0, my - 8.0))
        }
        Axis::Vertical => (Rect { x: -mx, y: b.y - near, w: w + 2.0 * mx, h: b.h + near + far }, Edges::new(0.0, 0.0, near, fade)),
    }
}

/// The structure the views are made for: the grid, and every item's id and size.
fn shape_of(smart: &Smart) -> String {
    let plan = smart.plan();
    let g = &plan.grid;
    let placed: Vec<String> = smart.items().iter().zip(&plan.placed).map(|(i, p)| format!("{}:{}@{},{}", i.id, p.size.id(), p.col, p.row)).collect();
    format!("{}x{} {:?}/{:?} {:?} {:?}|{}", g.cols, g.rows, g.cell, g.origin, g.style, g.heading, placed.join(","))
}

// ---- one item

fn make_view(cx: &mut Cx, scroller: &Group, scene: &ui::scene::Scene, item: &Item, index: usize, rect: Frame, look: Look) -> std::io::Result<ItemView> {
    let id = Id(ITEM_ID + index as u32);
    let kit = &mut *cx.kit;
    let flat = scene.adopting(scroller, || kit.ui.scene.glass(rect.w as u32, rect.h as u32, Z_PANE, item_glass(kit, rect.h, Glassy::Rest, item.gone), Props::at(rect.x, rect.y)));
    let m = metrics_of(kit);
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let (first, second) = parts::lines_of(look, item.headline(), rect.h, &m);
    let switch = item.has_switch().then(|| Toggle::size(kit));
    let sizes = Sizes { main: first.height(&m), second: second.height(&m), switch, headline: item.headline() };
    let p = parts::parts(look, rect.w, rect.h, &sizes, &m);
    let lines = vec![(p.main, text_style(kit, first, ink, true), String::new()), (p.second, text_style(kit, second, ink2, false), String::new())];
    let mut words = scene.adopting(scroller, || Words::new(kit, Frame::new(0.0, 0.0, rect.w, rect.h), Z_WORDS, lines))?;
    words.set_props(Props::at(rect.x, rect.y));
    let toggle = p.switch.map(|f| {
        let at = Frame::new(rect.x + f.x, rect.y + f.y, f.w, f.h);
        scene.adopting(scroller, || Toggle::new(kit, at, Z_SWITCH, item.on))
    });
    Ok(ItemView { id, key: item.id.clone(), flat, lift: None, left_ms: None, rect, parts: p, drawn: None, glyph: None, still: None, lit: Vec::new(), retired: Vec::new(), words, toggle, pressed: false, cue: None })
}

/// How long after an item left the front its lift is kept: the spring that puts it back settles in about 300 ms.
const SETTLE_MS: u64 = 600;
/// The most lifts an item card keeps at once (each is two groups of the scene's 96): the item in front, those settling back, and one more.
const MAX_LIFTS: usize = 5;

/// The lift of an item that is to come to the front (or is pressed): made now, its glass standing in the place of the glass at rest.
fn raise(cx: &mut Cx, scroller: &Group, view: &mut ItemView) -> std::io::Result<()> {
    view.left_ms = None;
    if view.lift.is_some() {
        return Ok(());
    }
    let kit = &mut *cx.kit;
    let gone = view.drawn.as_ref().is_some_and(|d| d.gone);
    let lift = Lift::nested(kit, cx.engine, scroller, view.id, view.rect, Z_PANE, PaneKind::Card, lift_scale(view.rect.w, view.rect.h), LIFT_RISE)?;
    lift.pane.restyle(item_glass(kit, view.rect.h, Glassy::Rest, gone));
    lift.lit.restyle(item_glass(kit, view.rect.h, if view.pressed { Glassy::Pressed } else { Glassy::Lit }, gone));
    view.flat.set(HIDDEN);
    view.lift = Some(lift);
    Ok(())
}

/// The lift of an item that is at rest goes, and the glass at rest is shown again where it was: the two look the same at rest.
fn lower(engine: &mut Engine, view: &mut ItemView) {
    if view.lift.take().is_some() {
        engine.flow.drop_lift(view.id);
    }
    view.left_ms = None;
    view.flat.set(Props::at(view.rect.x, view.rect.y));
}

/// Keeps the lifts to [`MAX_LIFTS`]: the one that left the front longest ago goes first, never the one in front or under the finger.
fn trim(cx: &mut Cx, views: &mut [ItemView], front: Option<Id>) {
    while views.iter().filter(|v| v.lift.is_some()).count() > MAX_LIFTS {
        let oldest = views.iter().enumerate().filter(|(_, v)| v.lift.is_some() && Some(v.id) != front && !v.pressed).min_by_key(|(_, v)| v.left_ms.unwrap_or(u64::MAX)).map(|(i, _)| i);
        match oldest {
            Some(i) => lower(cx.engine, &mut views[i]),
            None => break,
        }
    }

}

fn text_style(kit: &Kit, line: parts::Line, colour: Rgba, strong: bool) -> ui::components::Style {
    let (role, weight) = match line {
        parts::Line::Display => (TypeRole::Display, None),
        parts::Line::Title => (TypeRole::Title, Some(Weight::SemiBold)),
        parts::Line::Body => (TypeRole::Body, None),
        parts::Line::Label => (TypeRole::Label, strong.then_some(Weight::SemiBold)),
        parts::Line::Caption => (TypeRole::Caption, None),
    };
    role_style(kit, role, weight, colour, Align::Left)
}

/// A lamp that is on: its glow and its glyph in the colour it shows, one picture the size of what the glow reaches inside the cell.
struct LitImage {
    px: Vec<u8>,
    w: u32,
    h: u32,
    /// Where it starts in the cell.
    x: f32,
    y: f32,
}

#[allow(clippy::too_many_arguments)]
fn lit_image(kit: &mut Kit, icon: &str, glyph: Frame, cell: (f32, f32), g: u32, rgb: [u8; 3], level: f32, radius: f32) -> Option<LitImage> {
    let ground = rgb_of(kit.ui.look.canvas());
    // the glyph is the lamp's colour made as light as it needs to be to read on the glass; the glow is the colour itself
    let ink = paint::legible(rgb, ground, 3.0);
    let glyph_px = kit.icons.draw(icon, g, Rgba(ink[0], ink[1], ink[2], 255))?;
    let r = g as f32 * 1.7;
    let (cx, cy) = (glyph.x + glyph.w / 2.0, glyph.y + glyph.h / 2.0);
    let (x0, y0) = ((cx - r).floor().max(0.0), (cy - r).floor().max(0.0));
    let (x1, y1) = ((cx + r).ceil().min(cell.0), (cy + r).ceil().min(cell.1));
    let (w, h) = ((x1 - x0).max(1.0) as u32, (y1 - y0).max(1.0) as u32);
    let mut px = vec![0u8; (w * h * 4) as usize];
    // the glow follows the theme: a light on a dark glass is brighter than a light on a pale one
    let peak = if kit.mode == Mode::Dark { 0.55 } else { 0.38 };
    let strength = peak * (0.35 + 0.65 * level.clamp(0.0, 1.0));
    // the glow dies out before the edge of the cell: the glass under a lifted item grows past it, and a glow that ended at the old edge would show it
    paint::glow(&mut px, w, h, cx - x0, cy - y0, r, rgb, strength, Keep { x: -x0, y: -y0, w: cell.0, h: cell.1, radius, soft: 16.0 });
    paint::lay(&mut px, w, h, &glyph_px, g, g, (glyph.x - x0) as i32, (glyph.y - y0) as i32);
    Some(LitImage { px, w, h, x: x0, y: y0 })
}

/// The alpha of the theme's canvas colour that keeps both lines of a label readable over any picture (the lint's worst of black, white and
/// mid grey): the least step that passes, so a dark picture in the light look is not veiled more than it needs.
fn plate_alpha(kit: &Kit) -> f32 {
    use crate::kit::lint::{CONTRAST_MUTED, CONTRAST_TEXT, worst_contrast};
    let c = kit.ui.look.canvas();
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    (25..=48)
        .map(|n| n as f32 * 0.02)
        .find(|&a| {
            let paint = plate_paint(c, a);
            worst_contrast(ink, paint) >= CONTRAST_TEXT && worst_contrast(ink2, paint) >= CONTRAST_MUTED
        })
        .unwrap_or(0.96)
}

/// The canvas colour at `a`, premultiplied as the lint composites it.
fn plate_paint(c: Rgba, a: f32) -> Rgba {
    let m = |v: u8| (f32::from(v) * a).round() as u8;
    Rgba(m(c.0), m(c.1), m(c.2), (a * 255.0).round() as u8)
}

/// What the words of a camera item with a still are read against, for the lint: the plate of the theme's canvas colour over the picture.
fn plate_for(kit: &Kit) -> Rgba {
    plate_paint(kit.ui.look.canvas(), plate_alpha(kit))
}

/// How far above the first line the plate fades in.
const PLATE_FEATHER: f32 = 28.0;

/// A camera's still across the cell, with a plate of the theme's canvas colour under the words (from `label_top`, the top of the first line,
/// down) and a scrim that deepens to the foot; the plate is as strong as the lint needs the words to be over any picture.
fn paint_still(kit: &Kit, layer: &mut ImageLayer, pic: &Picture, radius: f32, label_top: f32) {
    let (w, h) = (layer.width(), layer.height());
    let mut fitted = pic.fitted(w, h, Fit::Cover);
    fitted.round_corners(radius);
    // laid over a cleared layer, a premultiplied picture of the layer's own size is the layer's pixels
    layer.pixels_mut().copy_from_slice(&fitted.rgba);
    let scrim = kit.ui.look.canvas();
    let rows = (h as f32 * 0.55) as u32;
    let plate = plate_alpha(kit);
    let start = (label_top - PLATE_FEATHER).max(0.0);
    let px = layer.pixels_mut();
    for y in (h - rows).min(start as u32)..h {
        let t = if y >= h - rows { (y - (h - rows)) as f32 / rows as f32 } else { 0.0 };
        let ramp = ((y as f32 - start) / PLATE_FEATHER).clamp(0.0, 1.0);
        let a = (0.86 * t * t).max(plate * ramp * ramp * (3.0 - 2.0 * ramp));
        if a <= 0.0 {
            continue;
        }
        let (keep, add) = (1.0 - a, [f32::from(scrim.0) * a, f32::from(scrim.1) * a, f32::from(scrim.2) * a]);
        // (v + 0.5) cut to a byte is v.round() for these never-negative sums, without a libm call for each channel
        for p in px[(y * w * 4) as usize..((y + 1) * w * 4) as usize].chunks_exact_mut(4) {
            let cover = f32::from(p[3]) / 255.0;
            p[0] = (f32::from(p[0]) * keep + add[0] * cover + 0.5) as u8;
            p[1] = (f32::from(p[1]) * keep + add[1] * cover + 0.5) as u8;
            p[2] = (f32::from(p[2]) * keep + add[2] * cover + 0.5) as u8;
        }
    }
    layer.damage(None);
}

fn paint_glyph(layer: &mut ImageLayer, px: &[u8]) {
    layer.clear();
    let n = layer.pixels_mut().len().min(px.len());
    layer.pixels_mut()[..n].copy_from_slice(&px[..n]);
    layer.damage(None);
}

fn update_view(cx: &mut Cx, scroller: &Group, view: &mut ItemView, item: &Item, look: Look, v: &View) -> std::io::Result<()> {
    let now = cx.now_ms;
    view.retired.retain(|(_, until)| now < *until);
    let (w, h) = (view.rect.w as u32, view.rect.h as u32);
    let level = (item.level * 100.0).round() as u8;
    let camera = items::camera_of(&item.id);
    // (borrowed: a copy of the picture, some 0.7 MB for a camera, made for every apply that found the item as it was, was most of its time)
    let still = camera.and_then(|c| cx.stills.picture(c));
    let said = v.notice.filter(|_| v.smart.focus_id() == Some(item.id.as_str()) && v.smart.plan().grid.heading.is_none()).map(str::to_owned);
    let lamp = (item.is_lamp() && item.on && !item.gone).then_some(item.rgb).flatten().map(|c| (c, level));
    let drawn = Drawn { look, w, h, name: item.name.clone(), words: item.words.clone(), icon: item.icon.clone(), gone: item.gone, on: item.on, tone: item.tone.clone(), still: camera.filter(|_| still.is_some()).map_or(0, |c| cx.stills.revision_of(c)), lamp, said: said.clone() };
    if view.drawn.as_ref() == Some(&drawn) {
        return Ok(());
    }
    let before = view.drawn.take();
    let first = before.is_none();
    let scene = cx.kit.ui.scene.clone();
    let kit = &mut *cx.kit;
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let m = metrics_of(kit);
    let radius = kit.tokens.radius(Radius::Tile).min(view.rect.h / 2.0);
    // the words: the name over the status, or the reading over the name; one line in a cell that has room for one
    let said_or = |words: &str| said.clone().unwrap_or_else(|| words.to_owned());
    if before.as_ref().is_none_or(|b| (&b.name, &b.words, b.gone, &b.said) != (&drawn.name, &drawn.words, drawn.gone, &drawn.said)) {
        let (first_line, second_line) = parts::lines_of(look, item.headline(), view.rect.h, &m);
        let (s1, s2) = (text_style(kit, first_line, ink, true), text_style(kit, second_line, ink2, false));
        let (main, second) = if item.headline() { (said_or(&item.words), item.name.clone()) } else { (item.name.clone(), said_or(&item.words)) };
        let a = ellipsise(&mut kit.ui.fonts, &main, s1.ty, view.parts.main.w);
        let b = if look == Look::Mini { String::new() } else { ellipsise(&mut kit.ui.fonts, &second, s2.ty, view.parts.second.w) };
        view.words.set_many(kit, &[(0, a.as_str(), ink), (1, b.as_str(), ink2)]);
    }
    // the glass of a thing that cannot be reached lies flat
    if before.as_ref().is_some_and(|b| b.gone != drawn.gone) {
        view.flat.restyle(item_glass(kit, view.rect.h, Glassy::Rest, drawn.gone));
        if let Some(lift) = &view.lift {
            lift.pane.restyle(item_glass(kit, view.rect.h, Glassy::Rest, drawn.gone));
            lift.lit.restyle(item_glass(kit, view.rect.h, Glassy::Lit, drawn.gone));
        }
    }
    // the glyph: dim and neutral for a lamp that is off (the lit one is laid over it), the tone's colour for what is on, dimmer when gone
    let g = parts::glyph_side(look, item.headline(), &m) as u32;
    let glyph_at = Props::at(view.rect.x + view.parts.glyph.x, view.rect.y + view.parts.glyph.y);
    let with_still = camera.is_some() && still.is_some();
    if before.as_ref().is_none_or(|b| (&b.icon, b.gone, b.on, &b.tone) != (&drawn.icon, drawn.gone, drawn.on, &drawn.tone)) || view.glyph.is_none() {
        let colour = if item.gone {
            kit.ui.look.ink3()
        } else if item.is_lamp() {
            kit.ui.look.ink2()
        } else {
            tone_colour(kit, &item.tone)
        };
        let px = kit.icons.draw(&item.icon, g, colour).or_else(|| cx.pictures.get(&format!("base/{}", item.icon)).map(|p| p.fitted(g, g, Fit::Contain).rgba));
        if let Some(px) = px {
            if view.glyph.is_none() {
                view.glyph = Some(scene.adopting(scroller, || kit.ui.scene.image(g, g, Z_ART, glyph_at))?);
            }
            if let Some(layer) = view.glyph.as_mut() {
                paint_glyph(layer, &px);
            }
        }
    }
    if let Some(layer) = &view.glyph {
        layer.set(if with_still { HIDDEN } else { glyph_at });
    }
    // the camera's still
    if let (Some(pic), true) = (&still, camera.is_some()) {
        if view.still.is_none() {
            view.still = Some(scene.adopting(scroller, || kit.ui.scene.image(w, h, Z_ART - 1, Props::at(view.rect.x, view.rect.y)))?);
        }
        if let Some(layer) = view.still.as_mut() {
            paint_still(kit, layer, pic, radius, view.parts.main.y.min(view.parts.second.y));
        }
    }
    // the lamp: lit in its own colour, with a glow that follows its level; the change is a fade, and a change of colour goes by the way the
    // eye mixes (a midpoint in OKLab under the new colour), never through a muddy sRGB middle
    let prev = before.as_ref().and_then(|b| b.lamp);
    if first || prev != lamp {
        set_lit(cx, scroller, &scene, view, item, lamp, prev, first, g, radius)?;
    }
    if let Some(t) = view.toggle.as_mut() {
        t.set(cx.kit, item.on);
    }
    view.drawn = Some(drawn);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn set_lit(cx: &mut Cx, scroller: &Group, scene: &ui::scene::Scene, view: &mut ItemView, item: &Item, lamp: Option<([u8; 3], u8)>, prev: Option<([u8; 3], u8)>, first: bool, g: u32, radius: f32) -> std::io::Result<()> {
    let kit = &mut *cx.kit;
    let now = cx.now_ms;
    let (fade_on, fade_colour) = (kit.beat_ms(Beat::Label), kit.beat_ms(Beat::Card));
    let cell = (view.rect.w, view.rect.h);
    let origin = (view.rect.x, view.rect.y);
    let glyph = view.parts.glyph;
    // the lit layers fade out and are dropped once they are gone
    let retire = |view: &mut ItemView, ms: u32| {
        for l in std::mem::take(&mut view.lit) {
            if ms == 0 {
                l.layer.set(HIDDEN);
            } else {
                l.layer.animate(Props { opacity: 0.0, ..l.at }, ms, Ease::Out);
            }
            view.retired.push((l.layer, now + u64::from(ms) + 120));
        }
    };
    let Some((rgb, level)) = lamp else {
        retire(view, fade_on);
        return Ok(());
    };
    let make = |kit: &mut Kit, view: &mut ItemView, colour: [u8; 3], opaque: bool| -> std::io::Result<()> {
        let Some(img) = lit_image(kit, &item.icon, glyph, cell, g, colour, f32::from(level) / 100.0, radius) else { return Ok(()) };
        let at = Props::at(origin.0 + img.x, origin.1 + img.y);
        let mut layer = scene.adopting(scroller, || kit.ui.scene.image(img.w, img.h, Z_ART + 1, if opaque { at } else { Props { opacity: 0.0, ..at } }))?;
        paint_glyph(&mut layer, &img.px);
        view.lit.push(Lit { layer, at });
        Ok(())
    };
    if first || prev.is_none() {
        // from nothing: off to on is one fade in (none at all for a card that is just drawn)
        make(kit, view, rgb, first)?;
        if !first && let Some(l) = view.lit.last() {
            if fade_on == 0 { l.layer.set(l.at) } else { l.layer.animate(l.at, fade_on, Ease::Out) }
        }
        return Ok(());
    }
    // from one colour to another: the old one fades out as the new one comes in; a far one goes by a midpoint
    let from = prev.map_or(rgb, |p| p.0);
    let far = paint::distance(from, rgb) > 0.12;
    let old = std::mem::take(&mut view.lit);
    if far {
        make(kit, view, paint::mix(from, rgb, 0.5), false)?;
    }
    make(kit, view, rgb, false)?;
    let n = view.lit.len();
    for (i, l) in view.lit.iter().enumerate() {
        // the midpoint is in for the first half, the new colour for the whole of it, over the old
        let ms = if far && i + 1 < n { fade_colour / 2 } else { fade_colour };
        if ms == 0 { l.layer.set(l.at) } else { l.layer.animate(l.at, ms, Ease::Out) }
    }
    let half = if far { fade_colour / 2 } else { fade_colour };
    for l in old {
        if half == 0 {
            l.layer.set(HIDDEN);
        } else {
            l.layer.animate(Props { opacity: 0.0, ..l.at }, half, Ease::Out);
        }
        view.retired.push((l.layer, now + u64::from(fade_colour) + 120));
    }
    Ok(())
}

/// The press: the glass shrinks a little under the finger and takes the pressed material, the type stays; a held press that opens a sheet
/// shows a bar filling along the foot of the cell over the hold's time, which the compositor runs.
fn press_view(cx: &mut Cx, scroller: &Group, view: &mut ItemView, pressed: bool, holds: bool, hold_ms: u64) -> std::io::Result<()> {
    view.pressed = pressed;
    // the item under the finger has its lift, in front or not (a click lands before the focus is drawn there)
    if pressed {
        raise(cx, scroller, view)?;
    }
    let kit = &mut *cx.kit;
    let gone = view.drawn.as_ref().is_some_and(|d| d.gone);
    let ms = kit.beat_ms(Beat::Press);
    let to = if pressed { Props { scale: kit.tokens.focus.press_scale, ..Props::at(0.0, 0.0) } } else { Props::at(0.0, 0.0) };
    if let Some(lift) = &view.lift {
        lift.pane.animate(to, ms, Ease::Out);
        lift.lit.animate(to, ms, Ease::Out);
        // the lit glass is over the glass at rest while the item is in front: it is the one that sinks back, and it goes back
        lift.lit.restyle(item_glass(kit, view.rect.h, if pressed { Glassy::Pressed } else { Glassy::Lit }, gone));
    }
    if !pressed || !holds {
        view.cue = None;
        return Ok(());
    }
    let (pad, bar_h) = (kit.space(4), 4.0);
    let (x, y) = (view.rect.x + pad, view.rect.y + view.rect.h - kit.space(3) - bar_h);
    let w = view.rect.w - 2.0 * pad;
    let full = Rect { x, y, w, h: bar_h };
    let start = Rect { w: 0.0, ..full };
    let scene = kit.ui.scene.clone();
    let accent = kit.ui.look.accent();
    let bar = scene.adopting(scroller, || kit.ui.scene.shape(w as u32, bar_h as u32, Z_CUE, plate::solid(accent, bar_h / 2.0), Props { clip: Some(start), ..Props::at(x, y) }));
    bar.animate(Props { clip: Some(full), ..Props::at(x, y) }, hold_ms as u32, Ease::Linear);
    view.cue = Some(bar);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::lint::{CONTRAST_MUTED, CONTRAST_TEXT, worst_contrast};
    use crate::kit::testkit;

    #[test]
    fn a_camera_label_reads_on_its_plate_over_any_picture_in_both_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            let (h, kit) = testkit::kit_in(mode, 1.0);
            let plate = plate_for(&kit);
            assert!(worst_contrast(kit.ui.look.ink(), plate) >= CONTRAST_TEXT, "{mode:?} name");
            assert!(worst_contrast(kit.ui.look.ink2(), plate) >= CONTRAST_MUTED, "{mode:?} status");
            std::mem::forget(h);
        }
    }
}
