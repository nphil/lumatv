//! Home drawn (`LumaHome.qml`, `StatusBar.qml`, `Widget.qml`, `Dock.qml`, `Tile.qml`): stack 0 of the engine, under the compositor's
//! windows. The wallpaper and its scrim at the bottom; the cards as glass panes of the Lucent card material over the wallpaper's cached
//! blur with their contents (`widgets.rs`); the dock as a card with its tiles (a plate in the icon's colour, the icon, the name in a pill
//! under the focused tile), the accent glow behind the focused tile and the fade over the rows that overflow, the rows scrolled on the
//! focus spring; the status bar as a pill with its buttons and the profile chip (the avatar as a rounded square and the name). Every
//! focusable has its lift from the shared [`Engine`]'s flow (the card scale 1.03 with its 4 px rise, the tile 1.12, the status button
//! 1.04), and the lit glass that comes up with it; a hover cue is one ring moved to what the pointer is over. The layers over Home
//! (`layers.rs`) are made when they open and dropped when they close.

use super::layers::{AppsSurface, EditorSurface, MenuSurface, PickerSurface, SearchSurface};
use super::model::{self, Home, Layer, STATUS_ID, WIDGET_ID};
use super::wallpaper::{self, WallpaperSurface};
use super::widgets::{Content, PAD_SIDE, PAD_TOP, Pictures};
use crate::kit::flow::Lifted;
use crate::kit::pane::{self, PaneKind};
use crate::kit::picture::{self, Fit};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Engine, Id, Kit};
use compositor_protocol::{Backdrop, Glass, GroupPose, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use ui::lucent::palette::{Mode, PaletteRole};
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
/// The depths of stack 0: the wallpaper at the bottom, the layers over Home near the top (the system layer begins at `system_z`).
pub const Z_WALL: i32 = 0;
pub const Z_CARDS: i32 = 10;
pub const Z_DOCK: i32 = 20;
pub const Z_STATUS: i32 = 40;
pub const Z_HOVER: i32 = 60;
pub const Z_LAYERS: i32 = 100;
pub const Z_VEIL: i32 = 900;

pub use crate::kit::button::faded;

/// The Lucent glass of a card or a control over the wallpaper: the material's recipe with the wallpaper's cached blur behind it (stack
/// 0 is a layer stack of the engine, so the backdrop is real), the tint at the theme's `glass`; `lit` is the focused recipe.
pub fn glass_of(kit: &Kit, kind: PaneKind, h: f32, lit: bool, over_video: bool) -> Glass {
    let role = match kind {
        PaneKind::Sheet => SmartMaterial::Overlay,
        PaneKind::Card | PaneKind::Pill | PaneKind::Strip => SmartMaterial::Card,
        PaneKind::Control => SmartMaterial::Button,
    };
    let state = ControlState { focus_visible: lit, ..ControlState::default() };
    let pair = kit.ui.look.material(role, &state, MaterialContext::Independent);
    let base = pane::glass(kit, kind, h, over_video);
    let backdrop = if over_video || kit.tier == ui::lucent::material::MaterialTier::T0 { Backdrop::None } else { Backdrop::Cached { radius: kit.tokens.blur } };
    let tint = if lit { kit.ui.look.colour(PaletteRole::GlassRaised) } else { kit.ui.look.colour(PaletteRole::Glass) };
    let edge = if lit { kit.tokens.extra(kit.mode).glass_focus_edge } else { kit.ui.look.colour(PaletteRole::Edge) };
    Glass { corner_radius: base.corner_radius, tint: if over_video { base.tint } else { tint }, edge, backdrop, saturation: kit.tokens.saturation, ..pair.rest }
}

/// A focusable glass pane with its lift: the group at its rest pose (the pivot at its centre), the pane and, under a group whose
/// opacity the lift drives, the lit glass. The contents hang under the group too, so they lift with it.
pub struct Lift {
    pub id: Id,
    pub pane: GlassLayer,
    pub lit: GlassLayer,
    pub frame: Frame,
    pub rest: GroupPose,
}

impl Lift {
    #[allow(clippy::too_many_arguments)]
    pub fn new(kit: &mut Kit, engine: &mut Engine, id: Id, frame: Frame, z: i32, kind: PaneKind, scale: f32, rise: f32, over_video: bool) -> std::io::Result<Self> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let rest = GroupPose { x: frame.x, y: frame.y, pivot_x: frame.w / 2.0, pivot_y: frame.h / 2.0, ..GroupPose::IDENTITY };
        let group = kit.ui.scene.group(None, rest).map_err(other)?;
        let lit_group = kit.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let (w, h) = (frame.w.round() as u32, frame.h.round() as u32);
        let scene = kit.ui.scene.clone();
        let pane = scene.adopting(&group, || kit.ui.scene.glass(w, h, z, glass_of(kit, kind, frame.h, false, over_video), Props::at(0.0, 0.0)));
        let lit = scene.adopting(&lit_group, || kit.ui.scene.glass(w, h, z + 1, glass_of(kit, kind, frame.h, true, over_video), Props::at(0.0, 0.0)));
        engine.flow.lift(id, Lifted { group, rest, scale, rise, lit: Some(lit_group) });
        Ok(Self { id, pane, lit, frame, rest })
    }

    /// Runs `build` with every layer it makes hanging under the lift's group (`scene` is the kit's scene, cloned so `build` may use the kit).
    pub fn adopting<R>(&self, engine: &Engine, scene: &ui::scene::Scene, build: impl FnOnce() -> R) -> Option<R> {
        let group = engine.flow.group(self.id)?;
        Some(scene.adopting(group, build))
    }

    pub fn set_over_video(&self, kit: &Kit, kind: PaneKind, over_video: bool) {
        self.pane.restyle(glass_of(kit, kind, self.frame.h, false, over_video));
        self.lit.restyle(glass_of(kit, kind, self.frame.h, true, over_video));
    }

    /// Poses what the lift does not drive: the opacity (a card dimmed beside the focused one) and whether it shows.
    pub fn pose(&self, engine: &Engine, opacity: f32, visible: bool) {
        if let Some(g) = engine.flow.group(self.id) {
            g.set_pose(GroupPose { opacity, visible, ..self.rest });
        }
    }
}

/// A status bar button: a lifted control with its glyph, and for the profile chip the avatar and the name (the handles keep the layers).
#[allow(dead_code)]
struct StatusItem {
    id: Id,
    kind: String,
    lift: Lift,
    icon: Option<ImageLayer>,
    avatar: Option<ImageLayer>,
    name: Option<Text>,
    badge: Option<ShapeLayer>,
}

struct StatusBar {
    pane: GlassLayer,
    frame: Frame,
    items: Vec<StatusItem>,
    profile_key: String,
}

/// A card: its lift and its content.
#[allow(dead_code)]
struct Card {
    id: String,
    kind: String,
    focus_id: Id,
    lift: Lift,
    content: Content,
    frame: Frame,
    inner_rows: usize,
}

/// A dock tile (the handles keep the layers).
#[allow(dead_code)]
struct Tile {
    id: Id,
    plate: ShapeLayer,
    icon: Option<ImageLayer>,
    app: String,
    slot: (i32, i32),
}

#[allow(dead_code)]
struct DockSurface {
    lift: Lift,
    frame: Frame,
    viewport: Group,
    grid: Group,
    halo: ImageLayer,
    fade: Option<ShapeLayer>,
    tiles: Vec<Tile>,
    pill: GlassLayer,
    pill_text: Text,
    pill_for: Option<Id>,
    cols: usize,
    scroll: f32,
    key: String,
    halo_accent: Option<Rgba>,
    halo_at: Option<(f32, f32)>,
}

pub struct HomeSurface {
    module: Option<PathBuf>,
    data: Option<PathBuf>,
    pictures: Pictures,
    wall: WallpaperSurface,
    status: Option<StatusBar>,
    cards: Vec<Card>,
    dock: Option<DockSurface>,
    hover: Option<ShapeLayer>,
    apps: Option<AppsSurface>,
    search: Option<SearchSurface>,
    editor: Option<EditorSurface>,
    picker: Option<PickerSurface>,
    menu: Option<MenuSurface>,
    veil: Option<ShapeLayer>,
    generation: u64,
    text_scale: f32,
    last_focus: Option<Id>,
    last_hover: Option<Id>,
    layout_key: String,
    theme_key: String,
    over_video: bool,
    live: bool,
    news_at: u64,
    news_index: usize,
    last_clock_s: u64,
    wall_mode: Option<Mode>,
    page_hidden: bool,
    pub glyph_runs: bool,
}

impl HomeSurface {
    pub fn new(_system_z: i32) -> Self {
        Self {
            module: None,
            data: None,
            pictures: Pictures::default(),
            wall: WallpaperSurface::new(Z_WALL),
            status: None,
            cards: Vec::new(),
            dock: None,
            hover: None,
            apps: None,
            search: None,
            editor: None,
            picker: None,
            menu: None,
            veil: None,
            generation: 0,
            text_scale: 0.0,
            last_focus: None,
            last_hover: None,
            layout_key: String::new(),
            theme_key: String::new(),
            over_video: false,
            live: false,
            news_at: 0,
            news_index: 0,
            last_clock_s: 0,
            wall_mode: None,
            page_hidden: false,
            glyph_runs: true,
        }
    }

    pub fn dirs(&mut self, module: &Path, data: &Path) {
        self.module = Some(module.to_path_buf());
        self.data = Some(data.to_path_buf());
        self.pictures = Pictures::new(module);
    }

    pub fn restyle(&mut self) {
        self.generation = 0;
    }

    /// The surface's own timers: the news slide, the clock's second (or minute).
    pub fn tick(&mut self, now: u64) -> bool {
        now / 1000 != self.last_clock_s || (self.news_at > 0 && now >= self.news_at)
    }

    pub fn due_at(&self) -> Option<u64> {
        if self.cards.iter().any(|c| c.kind == "news") && self.news_at > 0 { Some(self.news_at) } else { None }
    }

    fn now_ns() -> u64 {
        ui::scene::monotonic_ns()
    }

    /// Draws Home as the model has it. Returns whether something of Home is on screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, home: &mut Home, now: u64) -> std::io::Result<bool> {
        let now_ns = Self::now_ns();
        let rebuild = self.generation != kit.generation() || self.text_scale != kit.text_scale;
        home.set_text_scale(kit.text_scale);
        if rebuild {
            self.teardown(engine);
            self.generation = kit.generation();
            self.text_scale = kit.text_scale;
        }
        // the wallpaper: loaded when the theme names it, sampled for the material and the accent
        if let Some(s) = self.wall.load(kit, &home.theme, self.data.as_deref(), self.module.as_deref())? {
            let material = home.theme.get("material").and_then(Value::as_str).unwrap_or("auto");
            if material == "auto" && s.luma >= 0.0 {
                let threshold = home.theme.get("materialThreshold").and_then(Value::as_f64).unwrap_or(0.6) as f32;
                let mode = if s.luma > threshold { Mode::Light } else { Mode::Dark };
                if mode != kit.mode {
                    let tokens = kit.tokens.clone();
                    kit.retheme(tokens, mode, kit.text_scale);
                    self.wall_mode = Some(mode);
                    return self.apply(kit, engine, home, now);
                }
            }
            if let Some(t) = s.tint {
                home.tint = wallpaper::tint_file(&self.wall.source, &s);
                let _ = t;
            }
        }
        self.wall.restyle(kit, &home.theme);
        let hole = home.wall_hole;
        self.wall.set_hole(kit, hole);
        if hole != self.over_video {
            self.over_video = hole;
            for c in &self.cards {
                c.lift.set_over_video(kit, PaneKind::Card, hole);
            }
            if let Some(d) = &self.dock {
                d.lift.set_over_video(kit, PaneKind::Card, hole);
            }
        }
        // the chrome, made again when the layout or the theme's own keys change
        let layout_key = home.layout.to_string();
        let theme_key = format!("{}|{}", home.theme.get("clock").map(Value::to_string).unwrap_or_default(), home.theme.get("tiles").map(Value::to_string).unwrap_or_default());
        if rebuild || layout_key != self.layout_key || theme_key != self.theme_key {
            self.layout_key = layout_key;
            self.theme_key = theme_key;
            self.build_chrome(kit, engine, home)?;
        }
        self.build_status(kit, engine, home)?;
        self.build_dock_tiles(kit, engine, home)?;
        self.update_contents(kit, home, now);
        self.update_dock(kit, engine, home, now_ns);
        self.update_focus(kit, engine, home, now_ns);
        self.update_layers(kit, engine, home, now)?;
        self.update_veil(kit, home, now);
        self.live = home.active;
        Ok(home.active)
    }

    fn teardown(&mut self, engine: &mut Engine) {
        for c in self.cards.drain(..) {
            engine.flow.drop_lift(c.focus_id);
        }
        if let Some(s) = self.status.take() {
            for it in s.items {
                engine.flow.drop_lift(it.id);
            }
        }
        if let Some(d) = self.dock.take() {
            engine.flow.drop_lift(Id(model::DOCK_ID + model::DOCK_MAX));
            for t in d.tiles {
                engine.flow.drop_lift(t.id);
            }
        }
        if let Some(a) = self.apps.take() {
            a.destroy(engine);
        }
        if let Some(s) = self.search.take() {
            s.destroy(engine);
        }
        if let Some(e) = self.editor.take() {
            e.destroy(engine);
        }
        if let Some(p) = self.picker.take() {
            p.destroy(engine);
        }
        self.menu = None;
        self.hover = None;
        self.veil = None;
        self.last_focus = None;
        self.last_hover = None;
        // the wallpaper's picture is the same in every look: its scrim is restyled, the picture is not decoded again
        self.layout_key.clear();
    }

    // ---- the cards and the dock's card
    fn build_chrome(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home) -> std::io::Result<()> {
        // cards that are gone go; cards that stay keep their layers when their box is the same
        let cards = home.cards();
        let keep: Vec<String> = cards.iter().map(|e| e.id.clone()).collect();
        let mut old: Vec<Card> = std::mem::take(&mut self.cards);
        let mut next = Vec::new();
        for e in &cards {
            let r = home.rect(e);
            let frame = Frame::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
            let focus_id = home.widget_id(&e.id).unwrap_or(Id(WIDGET_ID + 99));
            if let Some(i) = old.iter().position(|c| c.id == e.id && c.kind == e.kind && c.frame == frame) {
                next.push(old.remove(i));
                continue;
            }
            if let Some(i) = old.iter().position(|c| c.id == e.id) {
                let c = old.remove(i);
                engine.flow.drop_lift(c.focus_id);
            }
            let entry = super::grid::widgets(&home.layout).iter().find(|w| w.get("id").and_then(Value::as_str) == Some(&e.id)).cloned().unwrap_or(Value::Null);
            let lift = Lift::new(kit, engine, focus_id, frame, Z_CARDS, PaneKind::Card, kit.tokens.focus.card_scale, kit.tokens.focus.card_lift, self.over_video)?;
            let (cw, ch) = ((frame.w - 2.0 * PAD_SIDE).max(10.0), (frame.h - 2.0 * PAD_TOP).max(10.0));
            let scene = kit.ui.scene.clone();
            let mut content = lift.adopting(engine, &scene, || Content::new(kit, &e.kind, cw, ch, Z_CARDS + 2, &home.theme, &entry)).ok_or_else(|| std::io::Error::other("the card's group"))??;
            content.show(Props::at(PAD_SIDE, PAD_TOP));
            let inner_rows = match &content {
                Content::AtHome(a) => a.rows(),
                Content::Cameras(c) => c.rows(),
                Content::News(_) => 1,
                _ => 0,
            };
            next.push(Card { id: e.id.clone(), kind: e.kind.clone(), focus_id, lift, content, frame, inner_rows });
        }
        for c in old {
            if !keep.contains(&c.id) {
                engine.flow.drop_lift(c.focus_id);
            }
        }
        self.cards = next;
        self.glyph_runs = self.cards.iter().all(|c| c.content.is_glyph_run());
        // the dock's card
        let dock_rect = home.dock_rect();
        if let Some(r) = dock_rect {
            let frame = Frame::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
            if self.dock.as_ref().is_none_or(|d| d.frame != frame) {
                if let Some(d) = self.dock.take() {
                    for t in d.tiles {
                        engine.flow.drop_lift(t.id);
                    }
                }
                self.dock = Some(self.make_dock(kit, engine, home, frame)?);
            }
        } else if let Some(d) = self.dock.take() {
            for t in d.tiles {
                engine.flow.drop_lift(t.id);
            }
        }
        if self.hover.is_none() {
            let ink = kit.ui.look.ink();
            let ring = plate::ring(faded(ink, 0.55), kit.tokens.radius(Radius::Card) + 4.0, 3.0);
            self.hover = Some(kit.ui.scene.shape(8, 8, Z_HOVER, ring, HIDDEN));
        }
        Ok(())
    }

    fn make_dock(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, frame: Frame) -> std::io::Result<DockSurface> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let lift = Lift::new(kit, engine, Id(model::DOCK_ID + model::DOCK_MAX), frame, Z_DOCK, PaneKind::Card, 1.0, 0.0, self.over_video)?;
        let radius = kit.tokens.radius(Radius::Card);
        let parent = engine.flow.group(lift.id).map(|g| g.id());
        let viewport = kit.ui.scene.group(parent, GroupPose { clip: Some(compositor_protocol::Rect { x: 0.0, y: 0.0, w: frame.w, h: frame.h }), ..GroupPose::IDENTITY }).map_err(other)?;
        let grid = kit.ui.scene.group(Some(viewport.id()), GroupPose::IDENTITY).map_err(other)?;
        let scene = kit.ui.scene.clone();
        let (halo, fade, pill, pill_text) = scene.adopting(&viewport, || -> std::io::Result<_> {
            let halo = kit.ui.scene.image(440, 230, Z_DOCK + 2, HIDDEN)?;
            let fade = if home.dock.scrollable() {
                let base = if kit.mode == Mode::Light { Rgba(244, 242, 248, 240) } else { Rgba(18, 14, 26, 230) };
                let s = Shape { corner_radius: 0.0, fill: compositor_protocol::Fill::Linear { from: Rgba(0, 0, 0, 0), to: base, angle_deg: 90.0 }, border_width: 0.0, border: Rgba(0, 0, 0, 0) };
                Some(kit.ui.scene.shape(frame.w as u32, home.dock.fade.max(1) as u32, Z_DOCK + 8, s, Props::at(0.0, frame.h - home.dock.fade as f32)))
            } else {
                None
            };
            let pill = kit.ui.scene.glass(10, 43, Z_DOCK + 9, glass_of(kit, PaneKind::Control, 43.0, false, false), HIDDEN);
            let ink = kit.ui.look.ink();
            let mut pill_text = Text::new(kit, Frame::new(0.0, 0.0, 10.0, 43.0), Z_DOCK + 10, role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre), "")?;
            pill_text.set_props(HIDDEN);
            Ok((halo, fade, pill, pill_text))
        })?;
        let _ = radius;
        engine.flow.scroller(&kit.ui.scene, model::DOCK, &grid, GroupPose::IDENTITY, false, 4000.0).map_err(other)?;
        Ok(DockSurface { lift, frame, viewport, grid, halo, fade, tiles: Vec::new(), pill, pill_text, pill_for: None, cols: home.dock.cols(), scroll: 0.0, key: String::new(), halo_accent: None, halo_at: None })
    }

    /// The tiles, made again when the dock's list changes.
    fn build_dock_tiles(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home) -> std::io::Result<()> {
        let Some(d) = &mut self.dock else { return Ok(()) };
        let key = format!("{}|{}", home.dock.ids().join(","), home.dock.cols());
        if key == d.key {
            return Ok(());
        }
        d.key = key;
        d.cols = home.dock.cols();
        let mut old: Vec<Option<Tile>> = std::mem::take(&mut d.tiles).into_iter().map(Some).collect();
        let tile = home.dock.tile as f32;
        let radius = kit.tokens.radius(Radius::Tile);
        let inset = home.theme.pointer("/tiles/inset").and_then(Value::as_f64).unwrap_or(19.0) as f32;
        for (i, p) in home.dock.points.iter().enumerate() {
            let id = Id(model::DOCK_ID + i as u32);
            let (sx, sy) = home.dock.slot(i);
            let rest = GroupPose { x: sx as f32, y: sy as f32, pivot_x: tile / 2.0, pivot_y: tile / 2.0, ..GroupPose::IDENTITY };
            // a tile that was already made for this app moves to its slot
            if let Some(j) = old.iter().position(|t| t.as_ref().is_some_and(|t| t.app == p.id)) {
                let mut t = old[j].take().expect("found");
                if t.id != id {
                    // the lift keeps the app's group: it is given the tile's new name
                    if let Some(l) = engine.flow.take_lift(t.id) {
                        engine.flow.lift(id, l);
                    }
                    t.id = id;
                }
                t.slot = (sx, sy);
                let _ = engine.flow.place(id, rest);
                d.tiles.push(t);
                continue;
            }
            let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
            let group = kit.ui.scene.group(Some(d.grid.id()), rest).map_err(other)?;
            let plate_colour = lumatv_tokens::colour(&p.plate).map(|c| c.premultiply(0.0)).unwrap_or(Rgba(42, 40, 51, 255));
            let scene = kit.ui.scene.clone();
            let (plate_layer, icon) = scene.adopting(&group, || -> std::io::Result<_> {
                let shape = Shape { corner_radius: radius, fill: compositor_protocol::Fill::Solid(plate_colour), border_width: 1.0, border: faded(Rgba(255, 255, 255, 255), 0.13) };
                let plate_layer = kit.ui.scene.shape(tile as u32, tile as u32, Z_DOCK + 3, shape, Props::at(0.0, 0.0));
                let icon = Self::tile_icon(kit, &p.icon, tile as u32, inset as u32, radius, Z_DOCK + 4)?;
                Ok((plate_layer, icon))
            })?;
            engine.flow.lift(id, Lifted { group, rest, scale: kit.tokens.focus.tile_scale, rise: 6.0, lit: None });
            d.tiles.push(Tile { id, plate: plate_layer, icon, app: p.id.clone(), slot: (sx, sy) });
        }
        for t in old.into_iter().flatten() {
            engine.flow.drop_lift(t.id);
        }
        Ok(())
    }

    /// The icon of a tile: edge to edge when the picture fills its canvas (the plate then is its own colour), inset on the plate when
    /// its edges are clear.
    fn tile_icon(kit: &mut Kit, path: &str, tile: u32, inset: u32, radius: f32, z: i32) -> std::io::Result<Option<ImageLayer>> {
        if path.is_empty() {
            return Ok(None);
        }
        let Ok(bytes) = std::fs::read(path) else { return Ok(None) };
        let Ok(pic) = picture::decode(&bytes) else { return Ok(None) };
        let s = wallpaper::sample(&pic.rgba);
        // the edge alpha: the mean alpha of the outer ring of pixels
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in 0..pic.height {
            for x in 0..pic.width {
                if x == 0 || y == 0 || x + 1 == pic.width || y + 1 == pic.height {
                    sum += pic.rgba[((y * pic.width + x) * 4 + 3) as usize] as u64;
                    n += 1;
                }
            }
        }
        let full = n > 0 && sum as f32 / n as f32 / 255.0 >= 0.9;
        let _ = s;
        let side = if full { tile } else { tile - 2 * inset };
        let mut fitted = pic.fitted(side, side, Fit::Contain);
        if full {
            fitted.round_corners(radius);
        }
        let mut layer = kit.ui.scene.image(side, side, z, Props::at(if full { 0.0 } else { inset as f32 }, if full { 0.0 } else { inset as f32 }))?;
        fitted.paint(&mut layer);
        layer.damage(None);
        Ok(Some(layer))
    }

    // ---- the status bar
    fn build_status(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home) -> std::io::Result<()> {
        let (sx, sy) = home.safe();
        let frame = Frame::new(sx, sy, 1920.0 - 2.0 * sx, home.status_height());
        let profile = home.profile().cloned().unwrap_or(Value::Null);
        let profile_key = format!("{}|{}", profile, home.status_items().len());
        if self.status.as_ref().is_some_and(|s| s.profile_key == profile_key && s.frame == frame) {
            return Ok(());
        }
        if let Some(s) = self.status.take() {
            for it in s.items {
                engine.flow.drop_lift(it.id);
            }
        }
        let pane = kit.ui.scene.glass(frame.w as u32, frame.h as u32, Z_STATUS, glass_of(kit, PaneKind::Pill, frame.h, false, false), Props::at(frame.x, frame.y));
        let mut items = Vec::new();
        let ink = kit.ui.look.ink();
        let frames = home.status_frames();
        for (i, it) in home.status_items().iter().enumerate() {
            let Some(&(x, y, w, h)) = frames.get(i) else { break };
            let id = Id(STATUS_ID + i as u32);
            let kind = it.get("type").and_then(Value::as_str).unwrap_or("").to_owned();
            let lift = Lift::new(kit, engine, id, Frame::new(x, y, w, h), Z_STATUS + 1, PaneKind::Pill, kit.tokens.focus.button_scale, 0.0, false)?;
            let scene = kit.ui.scene.clone();
            let (icon, avatar, name, badge) = lift.adopting(engine, &scene, || -> std::io::Result<_> {
                let mut icon = None;
                let mut avatar = None;
                let mut name = None;
                let badge = None;
                if kind == "profile" {
                    let side = (h - 8.0) as u32;
                    let mut av = kit.ui.scene.image(side, side, Z_STATUS + 3, Props::at(4.0, 4.0))?;
                    Self::paint_avatar(kit, &mut av, &profile, side);
                    avatar = Some(av);
                    let n = home.profile_name();
                    if !n.is_empty() {
                        let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
                        let tw = width_of(kit, style, &n);
                        name = Some(Text::line_at(kit, h + 8.0, h / 2.0, tw.max(10.0), Z_STATUS + 3, style, &n)?);
                    }
                } else {
                    let glyph = match kind.as_str() {
                        "notifications" => "bell",
                        "settings" => "settings",
                        "search" => "search",
                        other => it.get("icon").and_then(Value::as_str).unwrap_or(other),
                    };
                    let side = 28u32;
                    icon = crate::kit::icon::layer(kit, glyph, side, ink, Z_STATUS + 3, Props::at(((w - side as f32) / 2.0).round(), ((h - side as f32) / 2.0).round()))?;
                }
                Ok((icon, avatar, name, badge))
            }).ok_or_else(|| std::io::Error::other("the item's group"))??;
            items.push(StatusItem { id, kind, lift, icon, avatar, name, badge });
        }
        self.status = Some(StatusBar { pane, frame, items, profile_key });
        Ok(())
    }

    /// The avatar as a rounded square: the person's picture, or their colour and letter.
    pub fn paint_avatar(kit: &mut Kit, layer: &mut ImageLayer, profile: &Value, side: u32) {
        let colour = profile.get("color").and_then(Value::as_str).and_then(lumatv_tokens::colour).map(|c| c.premultiply(0.0)).unwrap_or(Rgba(115, 96, 231, 255));
        let radius = side as f32 / 5.0;
        let file = profile.get("avatarFile").and_then(Value::as_str).unwrap_or("");
        let painted = !file.is_empty()
            && std::fs::read(file).ok().and_then(|b| picture::decode(&b).ok()).map(|p| {
                let mut f = p.fitted(side, side, Fit::Cover);
                f.round_corners(radius);
                f.paint(layer)
            }) == Some(true);
        if !painted {
            layer.clear();
            let (w, h) = (layer.width(), layer.height());
            let mut canvas = super::widgets::Canvas { px: layer.pixels_mut(), w, h };
            canvas.rounded(Frame::new(0.0, 0.0, side as f32, side as f32), radius, colour);
            let initial = profile.get("initial").and_then(Value::as_str).unwrap_or("?");
            let ink = if kit.mode == Mode::Light { Rgba(255, 255, 255, 255) } else { Rgba(23, 19, 31, 255) };
            let mut ty = kit.ty(TypeRole::Label);
            ty.px = (side as f32 * 0.5).round() as u32;
            ty.weight = Weight::SemiBold;
            let extent = kit.ui.fonts.measure(initial, ty);
            let x = ((side as f32 - extent.width) / 2.0).round();
            let baseline = ((side as f32 - extent.height()) / 2.0 + extent.ascent).round();
            let mut canvas2 = ui::text::Canvas { pixels: layer.pixels_mut(), width: side as usize };
            kit.ui.fonts.draw(&mut canvas2, initial, ty, (x, baseline), ink);
        }
        layer.damage(None);
    }

    // ---- the contents
    fn update_contents(&mut self, kit: &mut Kit, home: &Home, now: u64) {
        let t = super::clock::civil(now);
        let inner = home.inner.as_deref();
        let inner_index = home.inner_index;
        let slide_ms = home.theme.pointer("/news/slideMs").and_then(Value::as_u64).unwrap_or(10_000).max(1000);
        if self.news_at == 0 {
            self.news_at = now + slide_ms;
        } else if now >= self.news_at && inner.is_none() {
            self.news_at = now + slide_ms;
            self.news_index += 1;
        }
        for c in &mut self.cards {
            let mine = inner == Some(c.id.as_str());
            let row = if mine { inner_index } else { None };
            match &mut c.content {
                Content::Clock(cl) => cl.tick(kit, &t),
                Content::Weather(w) => w.set(kit, &mut self.pictures, &home.weather),
                Content::News(n) => n.set(kit, &home.news, if mine { inner_index.unwrap_or(0) } else { self.news_index }, now),
                Content::AtHome(a) => a.set(kit, &home.ha_cards, row, &mut self.pictures),
                Content::Cameras(cm) => cm.set(kit, &home.cameras, row, now),
                Content::Unknown(_) => {}
            }
        }
        self.last_clock_s = now / 1000;
    }

    // ---- the dock's rows, halo and pill
    fn update_dock(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now_ns: u64) {
        let Some(d) = &mut self.dock else { return };
        let scroll = home.dock.scroll_px();
        if scroll != d.scroll {
            d.scroll = scroll;
            let _ = engine.flow.scroll_to(model::DOCK, scroll);
        }
        let _ = now_ns;
        for (i, t) in d.tiles.iter_mut().enumerate() {
            let (sx, sy) = home.dock.slot(i);
            if t.slot != (sx, sy) {
                t.slot = (sx, sy);
                let rest = GroupPose { x: sx as f32, y: sy as f32, pivot_x: home.dock.tile as f32 / 2.0, pivot_y: home.dock.tile as f32 / 2.0, ..GroupPose::IDENTITY };
                let _ = engine.flow.place(t.id, rest);
            }
        }
        // the glow behind the focused tile and the name under it
        let focused = home.focus.current().and_then(|id| id.0.checked_sub(model::DOCK_ID).filter(|i| *i < model::DOCK_MAX).map(|i| i as usize));
        let dock_focused = focused.is_some() && home.layer == Layer::None;
        let accent_hex = home.dock.accent_for(home.dock.focused(), &home.theme, &home.tint.pointer("/tint").map(|t| {
            let c = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0) as u8;
            wallpaper::hex(wallpaper::accent_from((c("r"), c("g"), c("b")), kit.mode == Mode::Light))
        }).unwrap_or_else(|| "#9a8cf0".into()));
        let accent = lumatv_tokens::colour(&accent_hex).map(|c| c.premultiply(0.0)).unwrap_or(kit.ui.look.accent());
        match (dock_focused, focused) {
            (true, Some(i)) => {
                let (sx, sy) = home.dock.slot(i);
                let cx = sx as f32 + home.dock.tile as f32 / 2.0;
                let cy = sy as f32 + home.dock.tile as f32 / 2.0 - scroll;
                if d.halo_accent != Some(accent) {
                    d.halo_accent = Some(accent);
                    Self::paint_halo(&mut d.halo, accent);
                }
                let at = ((cx - 220.0).round(), (cy - 115.0).round());
                if d.halo_at != Some(at) {
                    d.halo_at = Some(at);
                    d.halo.set(Props { opacity: 0.75, ..Props::at(at.0, at.1) });
                }
                let title = home.dock.points.get(i).map(|p| p.title.clone()).unwrap_or_default();
                let id = Id(model::DOCK_ID + i as u32);
                if d.pill_for != Some(id) {
                    d.pill_for = Some(id);
                    let ink = kit.ui.look.ink();
                    let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
                    let w = (width_of(kit, style, &title) + 26.0).round().max(40.0);
                    let above = home.dock.row_of(i) == home.dock.scroll_row && home.dock.scrollable();
                    let lift = home.dock.tile as f32 * (kit.tokens.focus.tile_scale - 1.0) / 2.0;
                    let y = if above { sy as f32 - lift - 4.0 - 10.0 - 43.0 } else { sy as f32 + home.dock.tile as f32 + lift - 4.0 + 10.0 } - scroll;
                    let x = (cx - w / 2.0).round();
                    let scene = kit.ui.scene.clone();
                    let (pill, text) = scene.adopting(&d.viewport, || {
                        let pill = kit.ui.scene.glass(w as u32, 43, Z_DOCK + 9, glass_of(kit, PaneKind::Control, 43.0, false, false), Props::at(x, y));
                        let text = Text::new(kit, Frame::new(x, y, w, 43.0), Z_DOCK + 10, style, &title).map(|mut t| {
                            t.set_props(Props::at(x, y));
                            t
                        });
                        (pill, text)
                    });
                    d.pill = pill;
                    if let Ok(t) = text {
                        d.pill_text = t;
                    }
                }
            }
            _ => {
                if d.halo_at.take().is_some() {
                    d.halo.set(HIDDEN);
                }
                if d.pill_for.take().is_some() {
                    d.pill.set(HIDDEN);
                    d.pill_text.set_props(HIDDEN);
                }
            }
        }
    }

    /// The accent glow: a radial fall-off from the centre, alpha .5 at the centre to 0 at 70 %.
    fn paint_halo(layer: &mut ImageLayer, accent: Rgba) {
        let (w, h) = (layer.width(), layer.height());
        let px = layer.pixels_mut();
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = ((x as f32 + 0.5) / w as f32 - 0.5, (y as f32 + 0.5) / h as f32 - 0.5);
                let r = (dx * dx + dy * dy).sqrt() * 2.0;
                let a = (0.5 * (1.0 - r / 0.7)).clamp(0.0, 0.5);
                let i = ((y * w + x) * 4) as usize;
                px[i] = (accent.0 as f32 * a) as u8;
                px[i + 1] = (accent.1 as f32 * a) as u8;
                px[i + 2] = (accent.2 as f32 * a) as u8;
                px[i + 3] = (255.0 * a) as u8;
            }
        }
        layer.damage(None);
    }

    // ---- the focus
    fn update_focus(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now_ns: u64) {
        let current = if home.layer == Layer::None || home.layer == Layer::Edit { home.focus.current() } else { None };
        let shown = match home.layer {
            Layer::None => current,
            _ => home.focus.current(),
        };
        if shown != self.last_focus {
            let _ = engine.flow.go(self.last_focus, shown, now_ns);
            self.last_focus = shown;
        }
        // the cards not focused dim while one is
        let on_cards = current.is_some_and(|id| home.widget_of(id).is_some());
        for c in &self.cards {
            let dim = on_cards && Some(c.focus_id) != current;
            c.lift.pose(engine, if dim { 0.86 } else { 1.0 }, true);
        }
        if let Some(d) = &self.dock {
            let on_dock = current.is_some_and(|id| id.0 >= model::DOCK_ID && id.0 < model::DOCK_ID + model::DOCK_MAX);
            d.lift.pose(engine, if on_cards && !on_dock { 0.92 } else { 1.0 }, true);
            d.lift.pane.restyle(glass_of(kit, PaneKind::Card, d.frame.h, on_dock, self.over_video));
        }
        // the hover cue: a ring on what the pointer is over
        let hover = home.focus.hovered();
        if hover != self.last_hover {
            self.last_hover = hover;
            if let Some(h) = &self.hover {
                match hover.and_then(|id| home.focus.frame(id)) {
                    Some(f) => {
                        let ink = kit.ui.look.ink();
                        let r = if hover.is_some_and(|id| id.0 >= model::DOCK_ID) { kit.tokens.radius(Radius::Tile) } else { kit.tokens.radius(Radius::Card) };
                        let ring = plate::ring(faded(ink, 0.5), r + 4.0, 3.0);
                        let l = kit.ui.scene.shape((f.w + 8.0) as u32, (f.h + 8.0) as u32, Z_HOVER, ring, Props::at(f.x - 4.0, f.y - 4.0));
                        let _ = h;
                        self.hover = Some(l);
                    }
                    None => h.set(HIDDEN),
                }
            }
        }
    }

    // ---- the layers over Home
    fn update_layers(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now: u64) -> std::io::Result<()> {
        let now_ns = Self::now_ns();
        // the library
        let apps_up = home.layer == Layer::Apps || (home.layer == Layer::Search && home.search_over_apps());
        match (apps_up, &mut self.apps) {
            (true, None) => self.apps = Some(AppsSurface::new(kit, engine, home, Z_LAYERS)?),
            (false, Some(_)) => {
                if let Some(a) = self.apps.take() {
                    a.destroy(engine);
                }
            }
            _ => {}
        }
        if let Some(a) = &mut self.apps {
            a.update(kit, engine, home, now_ns)?;
        }
        let search_up = home.layer == Layer::Search;
        match (search_up, &mut self.search) {
            (true, None) => self.search = Some(SearchSurface::new(kit, engine, home, Z_LAYERS + 40)?),
            (false, Some(_)) => {
                if let Some(s) = self.search.take() {
                    s.destroy(engine);
                }
            }
            _ => {}
        }
        if let Some(s) = &mut self.search {
            s.update(kit, engine, home, now)?;
        }
        let edit_up = home.layer == Layer::Edit && home.editor.is_some();
        match (edit_up, &mut self.editor) {
            (true, None) => self.editor = Some(EditorSurface::new(kit, engine, home, Z_LAYERS + 60)?),
            (false, Some(_)) => {
                if let Some(e) = self.editor.take() {
                    e.destroy(engine);
                }
            }
            _ => {}
        }
        if let Some(e) = &mut self.editor {
            e.update(kit, engine, home, now)?;
        }
        let picker_up = home.layer == Layer::Picker;
        match (picker_up, &mut self.picker) {
            (true, None) => self.picker = Some(PickerSurface::new(kit, engine, home, Z_LAYERS + 80)?),
            (false, Some(_)) => {
                if let Some(p) = self.picker.take() {
                    p.destroy(engine);
                }
            }
            _ => {}
        }
        if let Some(p) = &mut self.picker {
            p.update(kit, engine, home, now)?;
        }
        let menu_up = home.layer == Layer::Menu;
        match (menu_up, &mut self.menu) {
            (true, None) => self.menu = Some(MenuSurface::new(kit, home, Z_LAYERS + 90)?),
            (false, Some(_)) => self.menu = None,
            _ => {}
        }
        if let Some(m) = &mut self.menu {
            m.update(kit, home)?;
        }
        // while a layer that hides the page is up, the cards and the dock step back
        let page_hidden = matches!(home.layer, Layer::Apps | Layer::Picker) || (home.layer == Layer::Search && home.search_over_apps());
        if page_hidden != self.page_hidden {
            self.page_hidden = page_hidden;
            for c in &self.cards {
                c.lift.pose(engine, 1.0, !page_hidden);
            }
            if let Some(d) = &self.dock {
                d.lift.pose(engine, 1.0, !page_hidden);
            }
        }
        if let Some(s) = &self.status {
            let hidden = page_hidden || home.layer == Layer::Edit || home.layer == Layer::Search;
            let at = Props { opacity: if hidden { 0.0 } else { 1.0 }, visible: !hidden, ..Props::at(s.frame.x, s.frame.y) };
            s.pane.set(at);
            for it in &s.items {
                it.lift.pose(engine, 1.0, !hidden);
            }
        }
        Ok(())
    }

    fn update_veil(&mut self, kit: &mut Kit, home: &Home, now: u64) {
        let a = home.picker.veil_alpha(now);
        if a <= 0.0 {
            if let Some(v) = &self.veil {
                v.set(HIDDEN);
            }
            return;
        }
        let canvas = kit.ui.look.canvas();
        let veil = self.veil.get_or_insert_with(|| kit.ui.scene.shape(1920, 1080, Z_VEIL, plate::solid(Rgba(canvas.0, canvas.1, canvas.2, 255), 0.0), HIDDEN));
        veil.set(Props { opacity: a, ..Props::at(0.0, 0.0) });
    }

    /// What is drawn, as the lint takes it: the panes as panels, every card's and the status bar's words in their boxes.
    pub fn lint(&self, kit: &mut Kit, home: &Home) -> crate::kit::lint::Lint {
        use crate::kit::lint::Lint;
        let mut lint = Lint::new();
        // A pane's paint as the lint composites it: the glass tint over the mode's scrim over the canvas. The material is chosen by
        // the wallpaper's luma (`material: auto`, the threshold 0.6), so under the dark glass the picture is never lighter than mid
        // grey, which the canvas stands for; a lighter picture gets the light material and its dark ink.
        let paint = {
            let canvas = kit.ui.look.canvas();
            let scrim = wallpaper::scrim_of(&home.theme, kit.mode)[1];
            let over = |under: Rgba, top: Rgba| {
                let k = 1.0 - top.3 as f32 / 255.0;
                Rgba((top.0 as f32 + under.0 as f32 * k) as u8, (top.1 as f32 + under.1 as f32 * k) as u8, (top.2 as f32 + under.2 as f32 * k) as u8, 255)
            };
            let dimmed = over(Rgba(canvas.0, canvas.1, canvas.2, 255), Rgba(0, 0, 0, (scrim * 255.0) as u8));
            over(dimmed, kit.ui.look.colour(PaletteRole::Glass))
        };
        if let Some(s) = &self.status {
            lint.panel("status", s.frame, paint, true);
            for it in &s.items {
                lint.panel(&format!("status.{}", it.kind), it.lift.frame, paint, true);
                if let Some(n) = &it.name {
                    // the chip holds the avatar and the name: the name's own box is the chip right of the avatar
                    let f = n.frame();
                    let name_box = Frame::new(it.lift.frame.x + f.x - 8.0, it.lift.frame.y, f.w + 16.0, it.lift.frame.h);
                    let p = lint.panel("status.profile.name-box", name_box, paint, true);
                    let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Left);
                    lint.word(kit, "status.profile.name", n.text(), Frame::new(it.lift.frame.x + f.x, it.lift.frame.y + f.y, f.w, f.h), style, Some(p));
                }
            }
        }
        for c in &self.cards {
            let p = lint.panel(&c.id, c.frame, paint, true);
            c.content.words().lint(kit, &mut lint, &c.id, (c.frame.x, c.frame.y), Some(p));
        }
        if let Some(d) = &self.dock {
            let p = lint.panel("dock", d.frame, paint, true);
            if d.pill_for.is_some() {
                let f = d.pill_text.frame();
                let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Centre);
                lint.word(kit, "dock.pill", d.pill_text.text(), Frame::new(d.frame.x + f.x, d.frame.y + f.y, f.w, f.h), style, Some(p));
            }
        }
        let _ = home;
        lint
    }

    /// Where things are, for the harness and the tests.
    pub fn geometry(&self, home: &Home) -> Value {
        json!({
            "cards": self.cards.iter().map(|c| json!({"id": c.id, "x": c.frame.x, "y": c.frame.y, "w": c.frame.w, "h": c.frame.h})).collect::<Vec<_>>(),
            "dock": self.dock.as_ref().map(|d| json!({"x": d.frame.x, "y": d.frame.y, "w": d.frame.w, "h": d.frame.h, "tiles": d.tiles.len(), "cols": d.cols})),
            "status": self.status.as_ref().map(|s| s.items.iter().map(|i| json!({"kind": i.kind, "x": i.lift.frame.x, "w": i.lift.frame.w})).collect::<Vec<_>>()),
            "wallpaper": self.wall.is_up(),
            "focus": home.focus.current().map(|i| i.0),
            "glyphRuns": self.glyph_runs,
            "apps": self.apps.is_some(), "search": self.search.is_some(), "editor": self.editor.is_some(), "picker": self.picker.is_some(), "menu": self.menu.is_some(),
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use serde_json::json;

    fn fixtures(h: &mut Home) {
        h.start(1000);
        h.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1000);
        let mut lps = vec![json!({"id": "com.webos.app.discovery", "title": "Apps", "unmovable": true, "icon": ""})];
        for (id, t) in [("netflix", "Netflix"), ("youtube", "YouTube"), ("plex", "Plex"), ("browser", "Web Browser"), ("hue", "Hue Sync")] {
            lps.push(json!({"id": id, "title": t, "icon": "", "iconColor": "#101010"}));
        }
        h.update(&Input::Luna { tag: model::LP_TAG, payload: json!({"launchPoints": lps}) }, 1001);
        let days: Vec<Value> = [("Today", "sun", 71, 51), ("Mon", "cloud", 73, 51), ("Tue", "rain", 62, 48), ("Wed", "partly", 66, 56), ("Thu", "drizzle", 59, 44)]
            .iter()
            .map(|(d, k, hi, lo)| json!({"day": d, "kind": k, "hi": format!("{hi}\u{b0}"), "lo": format!("{lo}\u{b0}")}))
            .collect();
        let weather = json!({"doc": {"fetchedAt": 5, "display": {"place": "Atlanta", "temp": "61\u{b0}", "cond": "Clear", "stats": "Feels like 57\u{b0} \u{b7} 53% humidity \u{b7} 6 mph", "nowKind": "moon", "days": days}}});
        h.update(&Input::Helper { topic: "weather".into(), data: weather }, 1002);
        let news = json!({"doc": {"fetchedAt": 5, "items": [{"tag": "WORLD", "src": "BBC News", "title": "Denmark says Russian warship fired flares at military helicopter",
            "summary": "The Danish prime minister has accused Russia of reckless behaviour, while the Kremlin is yet to comment. The helicopter was on a routine patrol over the Baltic when the flares were fired.", "pubMs": 0}]}});
        h.update(&Input::Helper { topic: "news".into(), data: news }, 1003);
        let cards = json!({"items": [{"id": "living", "revision": 1, "card": {"content": {"title": "Living room", "icon": "base.icons/lightbulb", "role": "warning", "facts": [{"label": "Lights", "value": "3 on"}]}}},
            {"id": "door", "revision": 1, "card": {"content": {"title": "Front door", "icon": "base.icons/lock", "role": "positive", "facts": [{"label": "State", "value": "Locked"}]}}}]});
        h.update(&Input::Helper { topic: "plugin.homeassistant.cards".into(), data: cards }, 1004);
        let profiles = json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "switchedAt": 1, "profiles": [
            {"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false}, {"id": "isabel", "name": "Isabel", "initial": "I", "color": "#EB6F92", "active": false, "locked": false}]});
        h.update(&Input::Helper { topic: "profiles".into(), data: profiles }, 1005);
    }

    fn press(h: &mut Home, name: &str, at: u64) {
        h.update(&Input::Key { name: name.into(), down: true, repeat: false }, at);
        h.update(&Input::Key { name: name.into(), down: false, repeat: false }, at + 30);
    }

    fn lint_home(mode: Mode, scale: f32, layout: Value) -> (Vec<crate::kit::lint::Finding>, usize) {
        let (h, mut kit) = testkit::kit_in(mode, scale);
        let mut engine = Engine::new(&kit).expect("an engine");
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let mut home = Home::new(theme, super::super::grid::shipped(), layout);
        fixtures(&mut home);
        press(&mut home, "down", 2000);
        press(&mut home, "down", 2100);
        press(&mut home, "down", 2200);
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
        let lint = s.lint(&mut kit, &home);
        let findings = lint.findings();
        let words = lint.words().len();
        std::mem::forget(h);
        (findings, words)
    }

    #[test]
    fn the_shipped_home_passes_the_layout_and_contrast_lint_at_both_text_sizes_in_both_materials() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (findings, words) = lint_home(mode, scale, json!({}));
                assert!(words >= 12, "{mode:?} {scale}: only {words} words were drawn");
                let bad: Vec<String> = findings.iter().map(|f| format!("{f:?}")).collect();
                assert!(bad.is_empty(), "{mode:?} at {scale}: {}", bad.join("\n"));
            }
        }
    }

    #[test]
    fn the_halves_and_the_thirds_pass_the_lint() {
        let halves = json!({"grid": {"rows": 16}, "widgets": [
            {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 6, "rows": 4}, {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 6, "rows": 5, "title": "At home"},
            {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9}, {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        for scale in [1.0, 2.0] {
            let (findings, _) = lint_home(Mode::Dark, scale, halves.clone());
            let bad: Vec<String> = findings.iter().map(|f| format!("{f:?}")).collect();
            assert!(bad.is_empty(), "halves at {scale}: {}", bad.join("\n"));
        }
    }
}
