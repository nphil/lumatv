//! The grid drawn (`docs/mocks/cameras-scrypted/grid*.jpg`, spec "The grid"): over a scrim that hides the page, the header ("Cameras", the
//! count, `Events` and `Settings`), a banner line while the link is not ready, the tiles (a still in the tile's radius, the name under it,
//! the state's words over a dimmed still), the foot (the range left below at the left, the keys as glyphs at the right), or the state's
//! words and one primary button where there is no tile.
//!
//! A tile is a focusable of the engine: its layers hang under a group the flowing lift moves (scale 1.03 about its centre, a lit edge of
//! 3 px that fades in with the lift and the name in the full ink); the content of the tiles hangs under one group that scrolls on the same
//! spring when the focus goes to a row that is not shown. Only the rows that show and the row on each side are built.

use super::common::{Ink, Parts, icon_side, scale, tile_grade};
use super::pic::{Fill, PicLayer};
use super::pill;
use super::{Cx, REGION_HEADER, REGION_TILES, SCOPE_BASE, depth, ids};
use crate::cameras::app::Target;
use crate::cameras::grid::{self as model, Focus as GFocus, GAP};
use crate::cameras::view::{Empty, GridView, HintView};
use crate::kit::button::{Button, Place, Spec, Tone, faded};
use crate::kit::flow::Lifted;
use crate::kit::focus::Id;
use crate::kit::icon;
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::wrap;
use compositor_protocol::{GroupPose, Props, Rect, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::scene::ShapeLayer;
use ui::text::Weight;

/// How far past the shown rows the clip reaches: the lift (1.03) and the halo of the focused tile, less than the gap between rows.
const CLIP_MARGIN: i32 = 20;

/// A word as drawn, for the lint.
pub struct WordRec {
    pub name: String,
    pub text: String,
    pub frame: Frame,
    pub style: Style,
}

fn hidden_props() -> Props {
    crate::system::parts::HIDDEN
}

/// The veil over a tile that is not fresh: the page's ink over the still.
fn veil_colour(kit: &crate::kit::Kit) -> Rgba {
    match kit.mode {
        ui::lucent::palette::Mode::Dark => Rgba(5, 4, 10, 158),
        ui::lucent::palette::Mode::Light => Rgba(175, 171, 164, 178),
    }
}

struct TileUi {
    index: usize,
    id: Id,
    camera: String,
    /// Where the tile is in the content (before the scroll).
    frame: Frame,
    pic_key: String,
    body: Group,
    pic: PicLayer,
    ring: ShapeLayer,
    ring_radius: f32,
    hovered: bool,
    veil: Option<ShapeLayer>,
    words: Vec<Text>,
    words_sig: (Vec<String>, bool),
    /// Kept alive: the layers under the lit group (the halo, the lit edge, the name in the full ink) and the name at rest.
    _static: Vec<Box<dyn std::any::Any>>,
    name_text: String,
    name_frame: Frame,
    name_style: Style,
}

pub struct GridUi {
    ink: Ink,
    sig: String,
    /// Clips the scrolled rows to the shown block (the rows beside it are built, but never seen).
    _viewport: Group,
    content: Group,
    /// The shown block of rows, with room for the lift and the halo (what a hit may land on).
    block: Frame,
    tiles: Vec<TileUi>,
    window: std::ops::Range<usize>,
    top: usize,
    pitch: f32,
    header: Parts,
    buttons: Vec<(Button, Target)>,
    header_words: Vec<WordRec>,
    banner: Option<(Parts, Frame, Option<WordRec>)>,
    banner_sig: Option<String>,
    foot: Parts,
    foot_sig: String,
    foot_left: Option<(Parts, WordRec)>,
    foot_left_sig: Option<String>,
    empty: Option<(Parts, Frame, Vec<WordRec>)>,
    /// Hides the page; held so the layer stays.
    _scrim: ShapeLayer,
    hint_rect: Option<Frame>,
}

/// What a build of the grid depends on: the layout, the names, the text size and the look.
fn signature(cx: &Cx, view: &GridView) -> String {
    let names: Vec<&str> = view.tiles.iter().map(|t| t.name.as_str()).collect();
    format!(
        "{:?}|{:?}|{names:?}|{}|{}|{}|{}",
        view.layout,
        view.empty.as_ref().map(|e| (e.icon, &e.title)),
        cx.kit.generation(),
        cx.kit.text_scale,
        view.count_words,
        view.banner.is_some()
    )
}

impl GridUi {
    pub fn build(cx: &mut Cx, view: &GridView) -> std::io::Result<Self> {
        let ink = Ink::of(cx.kit);
        let z = cx.z;
        let scrim_fill = cx.kit.tokens.extra(cx.kit.mode).scrim_hide;
        let scrim = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z + depth::BLACK, plate::solid(scrim_fill, 0.0));
        let scene = cx.kit.ui.scene.clone();
        let l = &view.layout;
        let block = Frame::new(
            (l.x0 - CLIP_MARGIN) as f32,
            (l.y0 - CLIP_MARGIN) as f32,
            (l.cols as i32 * l.tile_w + (l.cols as i32 - 1) * GAP + 2 * CLIP_MARGIN) as f32,
            (l.rows_shown as i32 * l.pitch() - GAP + 2 * CLIP_MARGIN) as f32,
        );
        let viewport = scene
            .group(None, GroupPose { clip: Some(Rect { x: block.x, y: block.y, w: block.w, h: block.h }), ..GroupPose::IDENTITY })
            .map_err(other)?;
        let content = scene.group(Some(viewport.id()), GroupPose::IDENTITY).map_err(other)?;
        let mut me = Self {
            ink,
            sig: signature(cx, view),
            _viewport: viewport,
            content,
            block,
            tiles: Vec::new(),
            window: 0..0,
            top: view.top,
            pitch: view.layout.pitch() as f32,
            header: Parts::new(),
            buttons: Vec::new(),
            header_words: Vec::new(),
            banner: None,
            banner_sig: None,
            foot: Parts::new(),
            foot_sig: String::new(),
            foot_left: None,
            foot_left_sig: None,
            empty: None,
            _scrim: scrim,
            hint_rect: None,
        };
        me.build_header(cx, view)?;
        me.sync_banner(cx, view)?;
        if let Some(empty) = &view.empty {
            me.build_empty(cx, empty)?;
        } else {
            let extent = (view.layout.rows_total.saturating_sub(view.layout.rows_shown)) as f32 * me.pitch;
            let scene = cx.kit.ui.scene.clone();
            cx.engine.flow.scroller(&scene, REGION_TILES, &me.content, GroupPose::IDENTITY, false, extent).map_err(other)?;
            let offset = view.top as f32 * me.pitch;
            cx.engine.flow.scroll_snap(REGION_TILES, offset).map_err(other)?;
        }
        me.sync(cx, view)?;
        if cx.kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the grid could not be drawn whole"));
        }
        Ok(me)
    }

    /// Whether the grid built for `view` is still the one to show (a layout, a name, the text size or the look changed: it is not).
    pub fn fits(&self, cx: &Cx, view: &GridView) -> bool {
        self.sig == signature(cx, view)
    }

    /// Gives the engine back what the grid registered; the layers go with the grid.
    pub fn retire(self, engine: &mut crate::kit::Engine) {
        for t in &self.tiles {
            engine.focus.unregister(t.id);
            engine.flow.drop_lift(t.id);
        }
        for (b, _) in &self.buttons {
            b.detach(engine);
        }
        engine.flow.drop_scroller(REGION_TILES);
    }

    #[allow(clippy::too_many_arguments)]
    fn button(cx: &mut Cx, id: Id, label: &str, icon: &'static str, tone: Tone, at: (f32, f32), right: bool, z: i32) -> std::io::Result<Button> {
        let w = Button::width_for(cx.kit, label, true);
        let h = Button::height(cx.kit);
        let radius = cx.kit.tokens.radius(Radius::Control) + 2.0;
        let x = if right { at.0 - w } else { at.0 };
        let place = Place { id, frame: Frame::new(x, at.1, w, h), region: REGION_HEADER, scope: SCOPE_BASE, z };
        let mut b = Button::new(cx.kit, cx.engine, place, Spec::new(label, tone).with_icon(icon).with_radius(radius))?;
        b.show(cx.kit, true);
        Ok(b)
    }

    fn build_header(&mut self, cx: &mut Cx, view: &GridView) -> std::io::Result<()> {
        let ts = scale(cx.kit);
        let header_h = model::header_height(ts) as f32;
        let cy = 54.0 + header_h / 2.0;
        let z = cx.at(depth::HEADER);
        let title_style = role_style(cx.kit, TypeRole::Title, Some(Weight::SemiBold), self.ink.ink, Align::Left);
        let title_w = width_of(cx.kit, title_style, "Cameras");
        let line = cx.kit.line(TypeRole::Title);
        let frame = Frame::new(96.0, (cy - line / 2.0).round(), title_w, line);
        let title = Text::new(cx.kit, frame, z, title_style, "Cameras")?;
        self.header.text(title);
        self.header_words.push(WordRec { name: "title".into(), text: "Cameras".into(), frame, style: title_style });
        if !view.count_words.is_empty() {
            let style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink2, Align::Left);
            let w = width_of(cx.kit, style, &view.count_words);
            let l = cx.kit.line(TypeRole::Label);
            let frame = Frame::new(96.0 + title_w + 20.0, (cy - l / 2.0).round(), w, l);
            let t = Text::new(cx.kit, frame, z, style, &view.count_words)?;
            self.header.text(t);
            self.header_words.push(WordRec { name: "count".into(), text: view.count_words.clone(), frame, style });
        }
        if view.empty.is_none() {
            let h = Button::height(cx.kit);
            let y = (cy - h / 2.0).round();
            let settings = Self::button(cx, ids::HEADER_SETTINGS, "Settings", "settings", Tone::Plain, (1824.0, y), true, z)?;
            let right = settings.frame().x - 20.0;
            let events = Self::button(cx, ids::HEADER_EVENTS, "Events", "list-video", Tone::Plain, (right, y), true, z)?;
            self.buttons.push((events, Target::HeaderEvents));
            self.buttons.push((settings, Target::HeaderSettings));
        }
        self.header.set_opacity(1.0);
        Ok(())
    }

    fn build_empty(&mut self, cx: &mut Cx, empty: &Empty) -> std::io::Result<()> {
        let z = cx.at(depth::CONTENT);
        let mut parts = Parts::new();
        let ts = scale(cx.kit);
        let side = (72.0 * (1.0 + (ts - 1.0) * 0.5)).round() as u32;
        let style = role_style(cx.kit, TypeRole::Title, Some(Weight::SemiBold), self.ink.ink, Align::Centre);
        let line = cx.kit.line(TypeRole::Title);
        let h = Button::height(cx.kit);
        let block = side as f32 + 16.0 + line + 28.0 + h;
        let top = ((1080.0 - block) / 2.0).round();
        if let Some(layer) = icon::layer(cx.kit, empty.icon, side, self.ink.ink2, z, hidden_props())? {
            parts.image(layer, (((1920 - side) / 2) as f32, top));
        }
        let frame = Frame::new(480.0, top + side as f32 + 16.0, 960.0, line);
        let t = Text::new(cx.kit, frame, z, style, &empty.title)?;
        parts.text(t);
        let mut words = vec![WordRec { name: "empty title".into(), text: empty.title.clone(), frame, style }];
        let by = top + side as f32 + 16.0 + line + 28.0;
        let bw = Button::width_for(cx.kit, "Settings", true);
        let b = Self::button(cx, ids::EMPTY_SETTINGS, "Settings", "settings", Tone::Primary, (((1920.0 - bw) / 2.0).round(), by), false, z)?;
        words.push(WordRec { name: "empty settings".into(), text: "Settings".into(), frame: b.frame(), style });
        self.buttons.push((b, Target::EmptySettings));
        parts.set_opacity(1.0);
        self.empty = Some((parts, Frame::new(480.0, top, 960.0, block), words));
        Ok(())
    }

    fn sync_banner(&mut self, cx: &mut Cx, view: &GridView) -> std::io::Result<()> {
        let sig = view.banner.clone();
        if sig == self.banner_sig {
            return Ok(());
        }
        self.banner = None;
        self.banner_sig = sig.clone();
        let Some(words) = sig else { return Ok(()) };
        let ts = scale(cx.kit);
        let z = cx.at(depth::HEADER);
        let header_h = model::header_height(ts) as f32;
        let style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink2, Align::Left);
        let line = cx.kit.line(TypeRole::Label);
        let h = 64.0f32.max(line + 16.0).round();
        let y = 54.0 + header_h + 16.0;
        let frame = Frame::new(96.0, y, 1728.0, h);
        let mut parts = Parts::new();
        let wash = faded(self.ink.ink, cx.kit.tokens.states.selected_wash.max(0.08));
        let panel = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, frame.w, frame.h), z, plate::solid(wash, 20.0));
        parts.shape(panel, (frame.x, frame.y));
        let side = icon_side(cx.kit, 30.0);
        if let Some(layer) = icon::layer(cx.kit, "triangle-alert", side, self.ink.warning, z + 1, hidden_props())? {
            parts.image(layer, (frame.x + 24.0, (frame.y + (h - side as f32) / 2.0).round()));
        }
        let tx = frame.x + 24.0 + side as f32 + 14.0;
        let tw = width_of(cx.kit, style, &words);
        let tf = Frame::new(tx, (frame.y + (h - line) / 2.0).round(), tw, line);
        let t = Text::new(cx.kit, tf, z + 1, style, &words)?;
        parts.text(t);
        parts.set_opacity(1.0);
        self.banner = Some((parts, frame, Some(WordRec { name: "banner".into(), text: words, frame: tf, style })));
        Ok(())
    }

    fn sync_foot(&mut self, cx: &mut Cx, view: &GridView) -> std::io::Result<()> {
        let hints_sig = view.hints.iter().map(|h| h.name.as_str()).collect::<Vec<_>>().join("|");
        if hints_sig != self.foot_sig {
            self.foot = Parts::new();
            self.foot_sig = hints_sig;
            let hints: Vec<HintView> = if view.empty.is_some() { Vec::new() } else { view.hints.clone() };
            if !hints.is_empty() {
                let w = pill::hints_width(cx.kit, &hints);
                let side = pill::hint_side(cx.kit) as f32;
                pill::draw_hints(cx.kit, &hints, 1824.0 - w, 1026.0 - side / 2.0, cx.at(depth::HEADER), &self.ink, &mut self.foot)?;
                self.foot.set_opacity(1.0);
                self.hint_rect = Some(Frame::new(1824.0 - w, 1026.0 - side, w, side));
            } else {
                self.hint_rect = None;
            }
        }
        let below = view.below.clone();
        if below != self.foot_left_sig {
            self.foot_left = None;
            self.foot_left_sig = below.clone();
            if let Some(words) = below {
                let z = cx.at(depth::HEADER);
                let mut parts = Parts::new();
                let style = role_style(cx.kit, TypeRole::Caption, None, self.ink.ink2, Align::Left);
                let line = cx.kit.line(TypeRole::Caption);
                let side = icon_side(cx.kit, 28.0);
                let cy = 1026.0 - (side as f32).max(line) / 2.0;
                if let Some(layer) = icon::layer(cx.kit, "chevron-down", side, self.ink.ink2, z, hidden_props())? {
                    parts.image(layer, (96.0, (cy - side as f32 / 2.0).round()));
                }
                let w = width_of(cx.kit, style, &words);
                let frame = Frame::new(96.0 + side as f32 + 10.0, (cy - line / 2.0).round(), w, line);
                let t = Text::new(cx.kit, frame, z, style, &words)?;
                parts.text(t);
                parts.set_opacity(1.0);
                self.foot_left = Some((parts, WordRec { name: "below".into(), text: words, frame, style }));
            }
        }
        Ok(())
    }

    /// Builds the tiles of the rows that are in the window and lets the others go.
    fn sync_tiles(&mut self, cx: &mut Cx, view: &GridView) -> std::io::Result<()> {
        if view.empty.is_some() {
            return Ok(());
        }
        let l = &view.layout;
        let first_row = view.top.saturating_sub(1);
        let last_row = (view.top + l.rows_shown + 1).min(l.rows_total);
        let want = (first_row * l.cols)..((last_row * l.cols).min(view.tiles.len()));
        let gone: Vec<usize> = self.tiles.iter().filter(|t| !want.contains(&t.index)).map(|t| t.index).collect();
        for i in gone {
            if let Some(pos) = self.tiles.iter().position(|t| t.index == i) {
                let t = self.tiles.remove(pos);
                cx.engine.focus.unregister(t.id);
                cx.engine.flow.drop_lift(t.id);
            }
        }
        for i in want.clone() {
            if self.tiles.iter().all(|t| t.index != i) {
                let t = self.build_tile(cx, view, i)?;
                self.tiles.push(t);
            }
        }
        self.window = want;
        Ok(())
    }

    /// Where a hit on a tile may land once the scroll is at `offset`: its box in the shown block, nothing where the clip hides it (a hidden row
    /// would otherwise answer under the header's buttons).
    fn hit_rect(&self, frame: Frame, offset: f32) -> Frame {
        Frame::new(frame.x, frame.y - offset, frame.w, frame.h).intersect(&self.block).unwrap_or(Frame::new(self.block.x, self.block.y, 0.0, 0.0))
    }

    fn build_tile(&mut self, cx: &mut Cx, view: &GridView, i: usize) -> std::io::Result<TileUi> {
        let tv = &view.tiles[i];
        let l = &view.layout;
        let (x, y) = l.tile_at(i);
        let (w, h) = (l.tile_w as f32, l.tile_h as f32);
        let total_h = h + l.name_h as f32;
        let radius = cx.kit.tokens.radius(Radius::Tile);
        let z = cx.at(depth::CONTENT);
        let scene = cx.kit.ui.scene.clone();
        let rest = GroupPose { x: x as f32, y: y as f32, pivot_x: w / 2.0, pivot_y: total_h / 2.0, ..GroupPose::IDENTITY };
        let lift = scene.group(Some(self.content.id()), rest).map_err(other)?;
        let lit = scene.group(Some(lift.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let body = scene.group(Some(lift.id()), GroupPose::IDENTITY).map_err(other)?;
        let name_style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink2, Align::Centre);
        let line = cx.kit.line(TypeRole::Label);
        let name_text = wrap::elide(cx.kit, name_style, &tv.name, w);
        let name_frame = Frame::new(0.0, h + 8.0, w, line);
        let lit_style = Style { colour: self.ink.ink, ..name_style };
        let ring_w = 3.0;
        let (ring, name, mut statics) = scene.adopting(&body, || -> std::io::Result<_> {
            let ring = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, h), z + 2, plate::ring(self.ink.edge, radius, ring_w));
            ring.set(Props { opacity: 0.6, ..Props::at(0.0, 0.0) });
            let name = Text::new(cx.kit, name_frame, z + 3, name_style, &name_text)?;
            Ok((ring, name, Vec::<Box<dyn std::any::Any>>::new()))
        })?;
        let (lit_ring, halo, name_lit) = scene.adopting(&lit, || -> std::io::Result<_> {
            let halo = plate::shape(
                &cx.kit.ui,
                Frame::new(-6.0, -6.0, w + 12.0, h + 12.0),
                z + 4,
                plate::ring(faded(self.ink.accent, 0.26), radius + 6.0, 6.0),
            );
            let lit_ring = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, h), z + 5, plate::ring(self.ink.edge_raised, radius, ring_w));
            let name_lit = Text::new(cx.kit, name_frame, z + 6, lit_style, &name_text)?;
            Ok((lit_ring, halo, name_lit))
        })?;
        statics.push(Box::new(name));
        statics.push(Box::new(lit_ring));
        statics.push(Box::new(halo));
        statics.push(Box::new(name_lit));
        let id = ids::tile(i);
        let frame = Frame::new(x as f32, y as f32, w, total_h);
        let offset = self.top as f32 * self.pitch;
        cx.engine.focus.register(id, self.hit_rect(frame, offset), REGION_TILES, SCOPE_BASE);
        cx.engine.flow.lift(id, Lifted { group: lift, rest, scale: cx.kit.tokens.focus.card_scale, rise: 0.0, lit: Some(lit) });
        let fill = if view.fill { Fill::Washed(tile_grade(cx.kit)) } else { Fill::Cover };
        let mut pic = PicLayer::new(&tv.pic, Frame::new(0.0, 0.0, w, h), radius, fill, z);
        pic.sync(cx.kit, Some(&body), cx.store)?;
        Ok(TileUi {
            index: i,
            id,
            camera: tv.id.clone(),
            frame,
            pic_key: tv.pic.clone(),
            body,
            pic,
            ring,
            ring_radius: radius,
            hovered: false,
            veil: None,
            words: Vec::new(),
            words_sig: (Vec::new(), false),
            _static: statics,
            name_text,
            name_frame,
            name_style,
        })
    }

    /// Follows the view: the pictures that arrived, the words of the tiles, the foot, the focus and the scroll.
    pub fn sync(&mut self, cx: &mut Cx, view: &GridView) -> std::io::Result<()> {
        self.sync_banner(cx, view)?;
        self.sync_foot(cx, view)?;
        if view.empty.is_some() {
            if let Some(b) = self.buttons.iter().find(|(_, t)| *t == Target::EmptySettings) {
                cx.follow(b.0.id);
            }
            return Ok(());
        }
        if view.top != self.top {
            self.top = view.top;
            let offset = view.top as f32 * self.pitch;
            cx.engine.flow.scroll_to(REGION_TILES, offset).map_err(other)?;
            self.sync_tiles(cx, view)?;
            for t in &self.tiles {
                cx.engine.focus.place(t.id, self.hit_rect(t.frame, offset));
            }
        } else {
            self.sync_tiles(cx, view)?;
        }
        let accent = self.ink;
        let veil = veil_colour(cx.kit);
        let radius = cx.kit.tokens.radius(Radius::Tile);
        let label_style = role_style(cx.kit, TypeRole::Label, None, accent.ink, Align::Centre);
        let line = cx.kit.line(TypeRole::Label);
        let z = cx.at(depth::CONTENT);
        let h = view.layout.tile_h as f32;
        let w = view.layout.tile_w as f32;
        for t in &mut self.tiles {
            let Some(tv) = view.tiles.get(t.index) else { continue };
            if t.pic.sync(cx.kit, Some(&t.body), cx.store)? {
                // a picture over a quiet tile comes in with the label beat; a refresh of the same picture does not
                t.pic.set_visible(true);
            }
            let sig = (tv.words.clone(), tv.dim);
            if sig != t.words_sig {
                t.words_sig = sig;
                t.words.clear();
                t.veil = None;
                let scene = cx.kit.ui.scene.clone();
                if tv.dim {
                    t.veil = Some(scene.adopting(&t.body, || plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, h), z + 1, plate::solid(veil, radius))));
                }
                let n = tv.words.len() as f32;
                let top = ((h - n * line) / 2.0).round();
                for (k, text) in tv.words.iter().enumerate() {
                    let f = Frame::new(12.0, top + k as f32 * line, w - 24.0, line);
                    let shown = wrap::elide(cx.kit, label_style, text, f.w);
                    let made = scene.adopting(&t.body, || Text::new(cx.kit, f, z + 7, label_style, &shown))?;
                    t.words.push(made);
                }
            }
        }
        let focus_id = match view.focus {
            GFocus::Tile(i) => ids::tile(i),
            GFocus::Events => ids::HEADER_EVENTS,
            GFocus::Settings => ids::HEADER_SETTINGS,
        };
        cx.follow(focus_id);
        Ok(())
    }

    /// The pointer's cue on or off for the focusable `id`.
    pub fn hover(&mut self, cx: &mut Cx, id: Id, on: bool) {
        for (b, _) in &mut self.buttons {
            if b.id == id {
                b.set_hover(cx.kit, on && cx.engine.focus.current() != Some(id));
            }
        }
        for t in &mut self.tiles {
            if t.id == id && t.hovered != on {
                t.hovered = on;
                let colour = if on { faded(self.ink.edge_raised, 0.6) } else { self.ink.edge };
                t.ring.reshape(plate::ring(colour, t.ring_radius, 3.0));
                t.ring.set(Props { opacity: if on { 1.0 } else { 0.6 }, ..Props::at(0.0, 0.0) });
            }
        }
    }

    /// The pointer is down on `id` (the button's pressed look) or let go.
    pub fn press(&mut self, cx: &mut Cx, id: Id, down: bool) {
        for (b, _) in &mut self.buttons {
            if b.id == id {
                b.set_pressed(cx.kit, down);
            }
        }
    }

    pub fn target(&self, id: Id) -> Option<Target> {
        if let Some((_, t)) = self.buttons.iter().find(|(b, _)| b.id == id) {
            return Some(*t);
        }
        self.tiles.iter().find(|t| t.id == id).map(|t| Target::Tile(t.index))
    }

    /// The pointer left: no tile or button wears the cue.
    pub fn no_cue(&mut self, cx: &mut Cx) {
        let ids: Vec<Id> = self.tiles.iter().map(|t| t.id).chain(self.buttons.iter().map(|(b, _)| b.id)).collect();
        for id in ids {
            self.hover(cx, id, false);
        }
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// The layers the grid holds, for the budget.
    pub fn layers(&self) -> usize {
        let tiles: usize = self
            .tiles
            .iter()
            .map(|t| 1 + t.pic.layers() + usize::from(t.veil.is_some()) + t.words.len() + t._static.len())
            .sum();
        1 + self.header.layers()
            + self.buttons.iter().map(|(b, _)| b.layers()).sum::<usize>()
            + self.banner.as_ref().map_or(0, |(p, _, _)| p.layers())
            + self.foot.layers()
            + self.foot_left.as_ref().map_or(0, |(p, _)| p.layers())
            + self.empty.as_ref().map_or(0, |(p, _, _)| p.layers())
            + tiles
    }

    /// The rectangles of the tiles on the screen (the harness reads them).
    pub fn tile_rects(&self) -> Vec<Frame> {
        let offset = self.top as f32 * self.pitch;
        self.tiles.iter().map(|t| Frame::new(t.frame.x, t.frame.y - offset, t.frame.w, t.frame.h)).collect()
    }

    /// The camera each built tile shows.
    pub fn tile_cameras(&self) -> Vec<&str> {
        self.tiles.iter().map(|t| t.camera.as_str()).collect()
    }

    /// Describes what is on the screen to a lint.
    pub fn feed_lint(&self, kit: &mut crate::kit::Kit, lint: &mut crate::kit::lint::Lint, over_scrim: usize) {
        for w in &self.header_words {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(over_scrim));
        }
        for (b, _) in &self.buttons {
            b.feed_lint(kit, lint, over_scrim);
        }
        if let Some((_, _, Some(w))) = &self.banner {
            // a row of an icon and words, left-aligned by design: read against the scrim it lies on
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(over_scrim));
        }
        if let Some((_, w)) = &self.foot_left {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(over_scrim));
        }
        if let Some((_, _, words)) = &self.empty {
            for w in words.iter().filter(|w| w.name != "empty settings") {
                lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(over_scrim));
            }
        }
        let offset = self.top as f32 * self.pitch;
        for t in &self.tiles {
            let screen = Frame::new(t.frame.x, t.frame.y - offset + t.name_frame.y, t.name_frame.w, t.name_frame.h);
            if screen.y >= 0.0 && screen.bottom() <= 1080.0 {
                lint.word(kit, &format!("name {}", t.name_text), &t.name_text, screen, t.name_style, Some(over_scrim));
            }
        }
    }

    /// The scrim's panel for a lint: the page hidden, so what is over it is read against it.
    pub fn scrim_paint(&self, kit: &crate::kit::Kit) -> Rgba {
        kit.tokens.extra(kit.mode).scrim_hide
    }

    pub fn pic_keys(&self) -> Vec<&str> {
        self.tiles.iter().map(|t| t.pic_key.as_str()).collect()
    }
}

pub fn other(e: ui::error::UiError) -> std::io::Error {
    std::io::Error::other(format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::app::{App, Config};
    use crate::cameras::surface::pic::PictureStore;
    use crate::cameras::view::Base;
    use crate::kit::lint::Lint;
    use crate::kit::screen::{Input, Model};
    use crate::kit::testkit;
    use serde_json::json;

    fn camera(id: &str, name: &str) -> serde_json::Value {
        json!({"id": id, "name": name, "online": true, "live": true, "recordings": true, "events": true, "clips": false, "prebuffered": true})
    }

    fn app(n: usize, text_scale: f32) -> App {
        let mut a = App::new(Config { text_scale, ..Config::default() });
        let list: Vec<_> = (0..n).map(|i| camera(&format!("{}", 300 + i), &format!("Camera {}", i + 1))).collect();
        a.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "ready", "cameras": list}) }, 1000);
        a
    }

    fn grid_view(a: &App) -> GridView {
        match a.view().base {
            Base::Grid(g) => g,
            other => panic!("not a grid: {other:?}"),
        }
    }

    fn rig(n: usize, text_scale: f32) -> (ui::testing::Harness, crate::kit::Kit, crate::kit::Engine, App, PictureStore) {
        let (h, kit) = testkit::kit(text_scale);
        let engine = crate::kit::Engine::new(&kit).expect("an engine");
        let mut a = app(n, text_scale);
        a.open_grid(1000);
        (h, kit, engine, a, PictureStore::new())
    }

    fn cx<'a>(kit: &'a mut crate::kit::Kit, engine: &'a mut crate::kit::Engine, store: &'a PictureStore) -> Cx<'a> {
        Cx { kit, engine, store, z: 2000, now_ms: 1000, now_ns: 1_000_000_000 }
    }

    #[test]
    fn eight_tiles_are_built_with_their_names_registered_with_the_engine_and_within_the_budget() {
        let (_h, mut kit, mut engine, a, store) = rig(8, 1.0);
        let view = grid_view(&a);
        let before = kit.room();
        let g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).expect("a grid");
        assert_eq!(g.tile_count(), 8);
        let rects = g.tile_rects();
        assert_eq!((rects[0].x, rects[0].y, rects[0].w, rects[0].h), (96.0, 270.0, 414.0, 232.0 + 44.0));
        assert_eq!(rects[1].x, 96.0 + 414.0 + 24.0);
        for i in 0..8 {
            assert!(engine.focus.is_live(ids::tile(i)), "tile {i} is a focusable");
        }
        assert!(engine.focus.hit(100.0, 300.0) == Some(ids::tile(0)));
        let used = before.layers - kit.room().layers;
        assert!(used < 140, "{used} layers for eight tiles and the header");
        assert!(kit.room().groups > 40, "{:?}", kit.room());
        g.retire(&mut engine);
        assert!(!engine.focus.is_live(ids::tile(0)));
    }

    #[test]
    fn the_focus_of_the_model_is_followed_with_the_lift() {
        let (_h, mut kit, mut engine, a, store) = rig(8, 1.0);
        let mut view = grid_view(&a);
        let mut g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(engine.focus.current(), Some(ids::tile(0)));
        view.focus = GFocus::Tile(5);
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(engine.focus.current(), Some(ids::tile(5)));
        view.focus = GFocus::Events;
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(engine.focus.current(), Some(ids::HEADER_EVENTS));
        assert_eq!(g.target(ids::HEADER_EVENTS), Some(Target::HeaderEvents));
        assert_eq!(g.target(ids::tile(3)), Some(Target::Tile(3)));
    }

    #[test]
    fn twelve_cameras_build_the_shown_rows_and_the_row_after_and_slide_the_window_with_the_scroll() {
        let (_h, mut kit, mut engine, mut a, store) = rig(12, 1.0);
        let view = grid_view(&a);
        let mut g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(g.tile_count(), 12, "rows 0, 1 and the one after: all three of this grid");
        a.update(&Input::Key { name: "down".into(), down: true, repeat: false }, 1100);
        a.update(&Input::Key { name: "down".into(), down: true, repeat: false }, 1200);
        let view = grid_view(&a);
        assert_eq!(view.top, 1);
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(g.tile_count(), 12);
        let rects = g.tile_rects();
        // the row above the shown ones is built but never answers a hit: its box lies under the header's buttons
        let above = rects[3];
        assert!(above.y < 0.0 + 150.0 && above.bottom() <= 300.0, "{above:?}");
        assert_ne!(engine.focus.hit(above.x + above.w / 2.0, above.y + above.h / 2.0), Some(ids::tile(3)));
        assert_eq!(engine.focus.current(), Some(ids::tile(8)));
    }

    #[test]
    fn a_picture_that_arrived_is_painted_over_a_tile_and_a_state_dims_it_with_words() {
        let (_h, mut kit, mut engine, mut a, mut store) = rig(4, 1.0);
        let mut view = grid_view(&a);
        let mut g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(g.tiles[0].words.len(), 1, "Connecting");
        store.put("tile:300", crate::kit::picture::Picture { width: 16, height: 9, rgba: vec![90; 16 * 9 * 4] });
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert!(g.tiles[0].pic.is_painted());
        assert!(!g.tiles[1].pic.is_painted(), "nothing is made up for a camera with no picture");
        view.tiles[0].words = vec!["4 min ago".into()];
        view.tiles[0].dim = true;
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert!(g.tiles[0].veil.is_some());
        assert_eq!(g.tiles[0].words.len(), 1);
        let _ = &mut a;
    }

    #[test]
    fn the_hover_cue_never_touches_the_focus_and_the_pointer_leaving_takes_it_away() {
        let (_h, mut kit, mut engine, a, store) = rig(4, 1.0);
        let view = grid_view(&a);
        let mut g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        g.hover(&mut cx(&mut kit, &mut engine, &store), ids::tile(2), true);
        assert!(g.tiles[2].hovered);
        assert_eq!(engine.focus.current(), Some(ids::tile(0)));
        g.no_cue(&mut cx(&mut kit, &mut engine, &store));
        assert!(!g.tiles[2].hovered);
    }

    #[test]
    fn no_tile_draws_the_states_words_and_one_primary_button() {
        let (_h, mut kit, mut engine, _a, store) = rig(1, 1.0);
        let mut empty = App::new(Config::default());
        empty.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "refused", "cameras": []}) }, 1000);
        empty.open_grid(1000);
        let view = grid_view(&empty);
        let mut g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(g.tile_count(), 0);
        assert_eq!(engine.focus.current(), Some(ids::EMPTY_SETTINGS));
        assert_eq!(g.target(ids::EMPTY_SETTINGS), Some(Target::EmptySettings));
        assert!(engine.focus.is_live(ids::EMPTY_SETTINGS) && !engine.focus.is_live(ids::HEADER_EVENTS), "the header has no buttons without cameras");
    }

    #[test]
    fn the_grid_reads_clean_to_the_lint_at_both_text_sizes_and_in_both_looks() {
        for (scale, n) in [(1.0f32, 8usize), (2.0, 8), (1.0, 3), (2.0, 4)] {
            for mode in [ui::lucent::palette::Mode::Dark, ui::lucent::palette::Mode::Light] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = crate::kit::Engine::new(&kit).unwrap();
                let mut a = app(n, scale);
                a.open_grid(1000);
                let mut view = grid_view(&a);
                view.banner = Some("Can\u{2019}t reach Scrypted".into());
                view.below = Some("9 to 12".into());
                let store = PictureStore::new();
                let mut g = GridUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
                g.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
                let mut lint = Lint::new();
                let scrim = lint.panel("scrim", Frame::new(0.0, 0.0, 1920.0, 1080.0), g.scrim_paint(&kit), false);
                g.feed_lint(&mut kit, &mut lint, scrim);
                let findings: Vec<String> = lint.findings().iter().map(|f| f.to_string()).collect();
                assert!(findings.is_empty(), "{n} cameras at {scale} in {mode:?}: {findings:?}");
            }
        }
    }
}
