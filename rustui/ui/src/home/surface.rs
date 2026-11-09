//! Home drawn (`LumaHome.qml`, `StatusBar.qml`, `Widget.qml`, `Dock.qml`, `Tile.qml`): stack 0 of the engine, under the compositor's
//! windows. The wallpaper and its scrim at the bottom; the cards as glass panes of the Lucent card material over the wallpaper's cached
//! blur with their contents (`widgets.rs`); the dock as a card with its tiles (a plate in the icon's colour, the icon, the name in a pill
//! under the focused tile), the accent glow behind the focused tile and the fade over the rows that overflow, the rows scrolled on the
//! focus spring; the status bar as a pill with its buttons and the profile chip (the avatar as a disc in the pill's left cap, and the name). Every
//! focusable has its lift from the shared [`Engine`]'s flow (the card scale 1.03 with its 4 px rise, the tile 1.12, the status button
//! 1.04), and the lit glass that comes up with it; a hover cue is one ring moved to what the pointer is over. The layers over Home
//! (`layers.rs`) are made when they open and dropped when they close.

use super::layers::{AppsSurface, ConfirmSurface, EditorSurface, LibraryIcons, MenuSurface, PickerSurface, PopoverSurface, SearchSurface};
use super::model::{self, Home, Layer, STATUS_ID, WIDGET_ID};
use super::wallpaper::{self, WallpaperSurface};
use super::widgets::{Content, PAD_SIDE, PAD_TOP, Pictures};
use crate::kit::flow::Lifted;
use crate::kit::mend::Mend;
use crate::kit::pane::{PaneKind, glass_of};
use crate::kit::picture::{self, Fit, Picture};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Engine, Id, Kit};
use compositor_protocol::{Glass, GroupPose, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
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
/// How often the cards' contents are brought up to date while a full-screen sheet covers Home (`HomeSurface::contents_due`).
const COVERED_CONTENTS_MS: u64 = 1000;

pub use crate::kit::button::faded;

/// The glass of the status bar's pill and of the buttons on it: the material's own, without the light and shade copies of its depth pair (the soft glow
/// beside every edge, which reads as a second, blurred edge at the size of a button). A button at rest has no shadow of its own either (the pill casts the
/// one); the focused button keeps the material's, which is part of its lift.
pub fn status_glass(glass: Glass, shadow: bool) -> Glass {
    Glass { depth: None, shadows: if shadow { glass.shadows } else { [None, None] }, ..glass }
}

/// The scale a dock tile lifts to: `scale`, nudged so the lifted tile is an even number of pixels wider than at rest. Its edges, which
/// grow about the tile's centre, then fall on whole pixels (120 px at 1.12 is 134.4 px, edges at -7.2; 134 px has them at -7).
pub fn lift_scale(tile: f32, scale: f32) -> f32 {
    if tile < 1.0 || scale <= 1.0 {
        return scale.max(1.0);
    }
    let grow = (tile * (scale - 1.0) / 2.0).round() * 2.0;
    (tile + grow) / tile
}

/// The side of the second picture a tile's icon has for the lift: the box at full lift, an even number of pixels (the lifted icon is
/// centred on the tile's centre, so an even side puts its edges on whole pixels).
pub fn lifted_icon_side(box_side: u32, scale: f32) -> u32 {
    ((box_side as f32 * scale / 2.0).round() as u32 * 2).max(box_side)
}

/// A focusable glass pane with its lift: the group at its rest pose (the pivot at its centre), the pane and, under a group whose
/// opacity the lift drives, the lit glass. The contents hang under the group too, so they lift with it.
pub struct Lift {
    pub id: Id,
    pub pane: GlassLayer,
    pub lit: GlassLayer,
    pub frame: Frame,
}

impl Lift {
    #[allow(clippy::too_many_arguments)]
    pub fn new(kit: &mut Kit, engine: &mut Engine, id: Id, frame: Frame, z: i32, kind: PaneKind, scale: f32, rise: f32, over_video: bool) -> std::io::Result<Self> {
        Self::make(kit, engine, None, id, frame, z, kind, scale, rise, over_video)
    }

    /// A lift whose group hangs under `parent` (a list that scrolls), at `frame` in the parent's space.
    #[allow(clippy::too_many_arguments)]
    pub fn nested(kit: &mut Kit, engine: &mut Engine, parent: &Group, id: Id, frame: Frame, z: i32, kind: PaneKind, scale: f32, rise: f32) -> std::io::Result<Self> {
        Self::make(kit, engine, Some(parent.id()), id, frame, z, kind, scale, rise, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn make(kit: &mut Kit, engine: &mut Engine, parent: Option<ui::group::GroupId>, id: Id, frame: Frame, z: i32, kind: PaneKind, scale: f32, rise: f32, over_video: bool) -> std::io::Result<Self> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let rest = GroupPose { x: frame.x, y: frame.y, pivot_x: frame.w / 2.0, pivot_y: frame.h / 2.0, ..GroupPose::IDENTITY };
        let group = kit.ui.scene.group(parent, rest).map_err(other)?;
        let lit_group = kit.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let (w, h) = (frame.w.round() as u32, frame.h.round() as u32);
        let scene = kit.ui.scene.clone();
        let pane = scene.adopting(&group, || kit.ui.scene.glass(w, h, z, glass_of(kit, kind, frame.h, false, over_video), Props::at(0.0, 0.0)));
        let lit = scene.adopting(&lit_group, || kit.ui.scene.glass(w, h, z + 1, glass_of(kit, kind, frame.h, true, over_video), Props::at(0.0, 0.0)));
        engine.flow.lift(id, Lifted { group, rest, scale, rise, lit: Some(lit_group) });
        Ok(Self { id, pane, lit, frame })
    }

    /// Gives the lift a group that does not scale or rise with it: what is made under [`Lift::adopting`] from now on is the card's text, on
    /// whole pixels at rest and at full lift, while the glass behind it grows.
    pub fn keep(&self, kit: &Kit, engine: &mut Engine) -> std::io::Result<()> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let Some(group) = engine.flow.group(self.id) else { return Ok(()) };
        let rest = GroupPose { pivot_x: self.frame.w / 2.0, pivot_y: self.frame.h / 2.0, ..GroupPose::IDENTITY };
        let kept = kit.ui.scene.group(Some(group.id()), rest).map_err(other)?;
        engine.flow.keep(self.id, kept, rest);
        Ok(())
    }

    /// Runs `build` with every layer it makes hanging under the lift's group, or under its kept group when it has one (`scene` is the kit's
    /// scene, cloned so `build` may use the kit).
    pub fn adopting<R>(&self, engine: &Engine, scene: &ui::scene::Scene, build: impl FnOnce() -> R) -> Option<R> {
        let group = engine.flow.kept(self.id).or_else(|| engine.flow.group(self.id))?;
        Some(scene.adopting(group, build))
    }

    pub fn set_over_video(&self, kit: &Kit, kind: PaneKind, over_video: bool) {
        self.pane.restyle(glass_of(kit, kind, self.frame.h, false, over_video));
        self.lit.restyle(glass_of(kit, kind, self.frame.h, true, over_video));
    }

    /// Poses what the lift does not drive: its opacity and whether it shows.
    pub fn pose(&self, engine: &mut Engine, opacity: f32, visible: bool) {
        engine.flow.pose(self.id, opacity, visible);
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
    /// The bell's count: a pill at the button's top right corner, painted when the count changes (`None` for the other buttons).
    badge: Option<ImageLayer>,
    /// The count painted in `badge` now.
    badge_n: Option<usize>,
}

struct StatusBar {
    pane: GlassLayer,
    /// The glass, and the box the buttons are in: with Full width on the buttons move inside a glass that stays where it is.
    frame: Frame,
    group: Frame,
    items: Vec<StatusItem>,
    profile_key: String,
    mark: Option<Mark>,
    /// Where the pane was last put (`update_layers`): it is put again only when that changes.
    at: Option<Props>,
    /// Whether the bar came through whole: it is made again when not, since nothing else would (the chip is made last).
    mend: Mend,
}

/// Do Not Disturb's mark before the buttons: its glyph, and the time left when it is timed. Nothing on it is focusable.
struct Mark {
    icon: Option<ImageLayer>,
    icon_at: Props,
    words: Option<Text>,
    /// Whether the bar was hidden when the layers were last put (`update_layers`).
    hidden: Option<bool>,
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

/// A dock tile's icon, resampled to the pixels it is drawn in: edge to edge when the picture fills its canvas (the plate then is its own
/// colour), inset on the plate when its edges are clear. Two pictures, each made once from the file by one Lanczos pass: the box's size,
/// drawn 1:1 at whole pixels while the tile rests, and the lifted box's size, drawn 1:1 when the tile is at full lift and crossfaded in
/// by the lift. A single picture shown at both sizes was drawn through a second, bilinear resample at one of them
/// (`docs/research/ui-sharpness-result.md`, `icon-sharpness-result.md`).
struct IconArt {
    rest: Picture,
    lifted: Picture,
    /// Where the box's corner is on the tile, and the lift's scale (the lifted picture is shown at its inverse, so the group's lift
    /// draws it 1:1).
    off: f32,
    lift: f32,
    key: ArtKey,
}

/// What a tile's pictures are made from: the icon's file, the tile's side, the inset, the corner's radius and the lift's scale.
type ArtKey = (String, u32, u32, u32, u32);

impl IconArt {
    fn key_of(path: &str, tile: u32, inset: u32, radius: f32, lift: f32) -> ArtKey {
        (path.to_owned(), tile, inset, radius.to_bits(), lift.to_bits())
    }

    fn of(path: &str, data: Option<&Path>, tile: u32, inset: u32, radius: f32, lift: f32) -> Option<Self> {
        let pic = picture::load_icon(path, data)?;
        // the edge alpha: the mean alpha of the outer ring of pixels
        let (mut sum, mut n) = (0u64, 0u64);
        for y in 0..pic.height {
            for x in 0..pic.width {
                if x == 0 || y == 0 || x + 1 == pic.width || y + 1 == pic.height {
                    sum += u64::from(pic.rgba[((y * pic.width + x) * 4 + 3) as usize]);
                    n += 1;
                }
            }
        }
        let full = n > 0 && sum as f32 / n as f32 / 255.0 >= 0.9;
        let side = if full { tile } else { tile - 2 * inset };
        let lifted_side = lifted_icon_side(side, lift);
        let make = |s: u32| {
            let mut p = pic.fitted_sharp(s, s);
            if full {
                p.round_corners(radius * s as f32 / side as f32);
            }
            p
        };
        Some(Self { rest: make(side), lifted: make(lifted_side), off: if full { 0.0 } else { inset as f32 }, lift, key: Self::key_of(path, tile, inset, radius, lift) })
    }

    /// The layer of one of the two pictures, in the group it is made under. The lifted one is centred where the rest one is and shown
    /// at `1 / lift`, so the lifted tile's group scale makes it as large as its pixels.
    fn layer(&self, kit: &mut Kit, z: i32, lifted: bool) -> std::io::Result<ImageLayer> {
        let (pic, scale) = if lifted { (&self.lifted, 1.0 / self.lift) } else { (&self.rest, 1.0) };
        let side = self.rest.width as f32;
        let at = self.off + (side - pic.width as f32) / 2.0;
        let mut layer = kit.ui.scene.image(pic.width, pic.height, z, Props { scale, ..Props::at(at, at) })?;
        pic.paint(&mut layer);
        layer.damage(None);
        Ok(layer)
    }
}

/// A dock tile (the handles keep the layers).
#[allow(dead_code)]
struct Tile {
    id: Id,
    plate: ShapeLayer,
    icon: Option<ImageLayer>,
    /// The icon again at the size the lifted tile draws it, crossfaded in by the lift (`tile_icon`), in a lit group of the tile's lift. Both are
    /// made only while the tile is focused or on its way back to rest: a lit group for each of 16 tiles held a sixth of the 96 groups the scene had.
    lifted_icon: Option<ImageLayer>,
    art: Option<IconArt>,
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
    /// [`Dock::rev`] and the columns the tiles were made for.
    key: Option<(u64, usize)>,
    /// The pictures of the tiles let go while Home was hidden or squeezed, for the tiles made again when it shows.
    arts: Vec<IconArt>,
    halo_accent: Option<Rgba>,
    halo_at: Option<(f32, f32)>,
    /// The glass the dock's pane was last given (`update_focus`); `None` once something else restyled it.
    pane_glass: Option<Glass>,
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
    library_icons: LibraryIcons,
    search: Option<SearchSurface>,
    editor: Option<EditorSurface>,
    picker: Option<PickerSurface>,
    menu: Option<MenuSurface>,
    confirm: Option<ConfirmSurface>,
    popover: Option<PopoverSurface>,
    weather_view: Option<super::weather_view::WeatherSurface>,
    onhome_sheet: Option<super::onhome::SheetSurface>,
    history_sheet: Option<super::onhome::sensor_surface::SensorSurface>,
    colour_sheet: Option<super::onhome::colour_surface::ColourSurface>,
    thermostat_sheet: Option<super::onhome::thermostat_surface::ThermostatSurface>,
    veil: Option<ShapeLayer>,
    generation: u64,
    text_scale: f32,
    /// The wallpaper's prepared backdrop the glass was made with.
    backdrop: Option<compositor_protocol::BackdropId>,
    last_focus: Option<Id>,
    last_hover: Option<Id>,
    /// [`Home::chrome_rev`] as the chrome was last made.
    chrome_rev: Option<(u64, u64)>,
    /// Whether the chrome (the cards and the dock with their focus plates) came through whole.
    chrome_mend: Mend,
    over_video: bool,
    live: bool,
    news_at: u64,
    /// When the weather picture's next paths are due (while it moves).
    art_at: Option<u64>,
    /// The same for the full weather view's picture.
    view_art_at: Option<u64>,
    news_index: usize,
    last_clock_s: u64,
    /// When the cards' contents were last brought up to date, and whether a full-screen sheet covered Home at the last apply (`contents_due`).
    contents_at: u64,
    covered: bool,
    /// Whether the dock's tiles are let go: Home is hidden under a full kind, or a panel over it needs their groups ([`HomeSurface::set_shed`]).
    shed: bool,
    page_hidden: bool,
    pub glyph_runs: bool,
}

impl HomeSurface {
    pub fn silk_report(&self) -> &Value { self.wall.report() }

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
            library_icons: LibraryIcons::default(),
            search: None,
            editor: None,
            picker: None,
            menu: None,
            confirm: None,
            popover: None,
            weather_view: None,
            onhome_sheet: None,
            history_sheet: None,
            colour_sheet: None,
            thermostat_sheet: None,
            veil: None,
            generation: 0,
            text_scale: 0.0,
            backdrop: None,
            last_focus: None,
            last_hover: None,
            chrome_rev: None,
            chrome_mend: Mend::default(),
            over_video: false,
            live: false,
            news_at: 0,
            art_at: None,
            view_art_at: None,
            news_index: 0,
            last_clock_s: 0,
            contents_at: 0,
            covered: false,
            shed: false,
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

    /// Lets the dock's tiles and the smart-home items' lifts go at once (`true`), a group or two each, keeping the tiles' pictures; `false` has
    /// them made again by the next apply. The shell sheds them under a full kind and for a panel or Settings that found no room in the
    /// scene's groups.
    pub fn set_shed(&mut self, engine: &mut Engine, shed: bool) {
        self.shed = shed;
        if !shed {
            return;
        }
        for c in &mut self.cards {
            if let Content::OnHome(card) = &mut c.content {
                card.shed(engine);
            }
        }
        let Some(d) = self.dock.as_mut() else { return };
        d.key = None;
        for mut t in d.tiles.drain(..) {
            engine.flow.drop_lift(t.id);
            d.arts.extend(t.art.take());
        }
    }

    pub fn is_shed(&self) -> bool {
        self.shed
    }

    /// The surface's own timers that ask for Home to be drawn again: the news slide, the clock's second (or minute), and a sky that is not made yet or
    /// waits for the atlas (the apply makes it). A sky that is made restates its paths without it ([`HomeSurface::art_due`]).
    pub fn tick(&mut self, now: u64) -> bool {
        (now / 1000 != self.last_clock_s && self.contents_due(now))
            || (self.news_at > 0 && now >= self.news_at)
            || (self.art_due(now) && !self.sky_steady())
            || self.view_art_at.is_some_and(|t| now >= t)
    }

    /// The weather card's sky is due to restate its paths (`Live::step`, every 0.4 s): the compositor moves its sprites along them.
    pub fn art_due(&self, now: u64) -> bool {
        self.art_at.is_some_and(|t| now >= t)
    }

    /// Every weather card's sky is made and the atlas is not holding it back.
    fn sky_steady(&self) -> bool {
        self.cards.iter().all(|c| match &c.content {
            Content::Weather(w) => w.art_steady(),
            _ => true,
        })
    }

    /// Restates the sky's paths without drawing Home again: the apply that did it every 0.4 s took 0.6 ms of the host's thread (a tenth of the shell's
    /// time at rest) to leave the cards, the glass and the dock as they were. Only for a steady sky ([`HomeSurface::tick`] asks the apply for the rest).
    pub fn step_art(&mut self, kit: &Kit, home: &Home, now: u64) {
        self.drive_weather(kit, home, now);
    }

    pub fn due_at(&self) -> Option<u64> {
        let news = if self.cards.iter().any(|c| c.kind == "news") && self.news_at > 0 { Some(self.news_at) } else { None };
        [news, self.art_at, self.view_art_at].into_iter().flatten().min()
    }

    /// Whether the cards' contents are to be brought up to date now: at every apply while Home is seen, and under a full-screen sheet (Settings, the
    /// display lab, the guide: Home shows dimmed through its glass) once a second and at the news slide. Each change of the sheet drew them again, and
    /// a page being made changes it at every answer of the helper: 0.7-1.3 ms of the shell's thread apiece on the host, most of it the Smart home card.
    fn contents_due(&self, now: u64) -> bool {
        !self.covered || now >= self.contents_at + COVERED_CONTENTS_MS || (self.news_at > 0 && now >= self.news_at)
    }

    /// The weather card's picture moves only while the card is seen: Home in front with nothing over the card, the screen on, no screensaver,
    /// and no reduced motion; otherwise its sprites rest on whole pixels and nothing is asked of the engine (`art_at` is `None`, so no wake).
    fn drive_weather(&mut self, kit: &Kit, home: &Home, now: u64) {
        let run = home.weather_art_runs() && !kit.ui.look.reduced_motion;
        let mut next: Option<u64> = None;
        for c in &mut self.cards {
            if let Content::Weather(w) = &mut c.content {
                // a picture that waits for the atlas to settle looks again in 100 ms
                let t = w.animate(now, run).or_else(|| (w.art_waits() && home.active).then_some(now + 100));
                if let Some(t) = t {
                    next = Some(next.map_or(t, |n| n.min(t)));
                }
            }
        }
        self.art_at = next;
    }

    fn now_ns() -> u64 {
        ui::scene::monotonic_ns()
    }

    /// Draws Home as the model has it. Returns whether something of Home is on screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, home: &mut Home, now: u64) -> std::io::Result<bool> {
        let now_ns = Self::now_ns();
        self.wall.set_held(home.wallpaper_held());
        if let Some(s) = self.wall.load(kit, &home.theme, self.data.as_deref(), self.module.as_deref())? {
            if s.tint.is_some() {
                home.tint = wallpaper::tint_file(&self.wall.source, &s);
            }
        }
        if let Some(s) = self.wall.sample.filter(|s| s.luma >= 0.0)
            && home.theme.get("material").and_then(Value::as_str).unwrap_or("auto") == "auto"
        {
            let threshold = home.theme.get("materialThreshold").and_then(Value::as_f64).unwrap_or(0.6) as f32;
            let mode = if s.luma > threshold { Mode::Light } else { Mode::Dark };
            if mode != kit.mode {
                kit.retheme(kit.tokens.clone(), mode, kit.text_scale);
                // Regrade the same original for the resolved mode, without resampling it.
                self.wall.load(kit, &home.theme, self.data.as_deref(), self.module.as_deref())?;
            }
        }
        self.wall.sync_backdrop(kit, &home.theme);
        let rebuild = self.generation != kit.generation() || self.text_scale != kit.text_scale || self.backdrop != kit.wall_backdrop;
        home.set_text_scale(kit.text_scale);
        if let Some(name) = home.name_to_measure() {
            let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Left);
            let width = width_of(kit, style, &name);
            home.set_name_width(name, width);
        }
        if let Some(shape) = home.mark_to_measure() {
            let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Left);
            let width = width_of(kit, style, &shape);
            home.set_mark_width(shape, width);
        }
        if rebuild {
            self.teardown(engine);
            self.generation = kit.generation();
            self.text_scale = kit.text_scale;
            self.backdrop = kit.wall_backdrop;
        }
        self.wall.restyle(kit, &home.theme);
        let hole = home.wall_hole;
        self.wall.set_hole(kit, hole);
        if hole != self.over_video {
            self.over_video = hole;
            for c in &self.cards {
                c.lift.set_over_video(kit, PaneKind::Card, hole);
            }
            if let Some(d) = &mut self.dock {
                d.lift.set_over_video(kit, PaneKind::Card, hole);
                d.pane_glass = None;
            }
        }
        // the chrome, made again when the layout or the theme's own keys change, or when it did not come through whole
        if rebuild || self.chrome_rev != Some(home.chrome_rev()) || self.chrome_mend.due(&kit.ui.scene, now) {
            self.chrome_rev = Some(home.chrome_rev());
            // the cards made now have no words yet
            self.contents_at = 0;
            self.chrome_mend.watch(&kit.ui.scene);
            self.build_chrome(kit, engine, home)?;
        }
        self.build_status(kit, engine, home, now)?;
        self.build_dock_tiles(kit, engine, home)?;
        // the card of smart-home items is laid out by the type the kit measures; the stills it needs are asked for at the size of their cells
        home.set_onhome_metrics(super::onhome::card::metrics_of(kit));
        let camera_rows = self
            .cards
            .iter()
            .flat_map(|c| match &c.content {
                Content::Cameras(camera) => camera.still_rows(&home.cameras),
                Content::OnHome(card) => home.smart_of(&c.id).map(|s| card.camera_rows(s)).unwrap_or_default(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        home.stills.configure(&camera_rows);
        self.update_badge(kit, home);
        self.update_mark(kit, home);
        // The clock's hands, the news slide and the cards' words are kept up to date while Home is in front, and brought up to date by the apply
        // that finds it in front again: a covered Home that went on doing it sent the engine a damaged picture every second, a composed frame under
        // a playing video for nothing.
        self.covered = home.covered();
        if home.active && self.contents_due(now) {
            self.contents_at = now;
            self.update_contents(kit, engine, home, now);
        }
        self.drive_weather(kit, home, now);
        self.update_dock(kit, engine, home, now_ns);
        self.update_focus(kit, engine, home, now_ns);
        home.set_motion_reduced(kit.ui.look.reduced_motion);
        self.update_layers(kit, engine, home, now)?;
        // the question's answers are focusables of Home's model once it is drawn
        if let Some(c) = self.confirm.as_mut().filter(|c| !c.reported) {
            home.set_confirm_frames(&c.frames);
            c.reported = true;
        }
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
        if let Some(c) = self.confirm.take() {
            c.destroy(engine);
        }
        self.popover = None;
        self.weather_view = None;
        self.onhome_sheet = None;
        self.history_sheet = None;
        self.colour_sheet = None;
        self.thermostat_sheet = None;
        self.hover = None;
        self.veil = None;
        self.last_focus = None;
        self.last_hover = None;
        // the wallpaper's picture is the same in every look: its scrim is restyled, the picture is not decoded again
        self.chrome_rev = None;
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
            // the items' words must stay on whole pixels when the card lifts: they hang under a group that does not scale with it
            if e.kind == "onhome" {
                lift.keep(kit, engine)?;
            }
            let (cw, ch) = ((frame.w - 2.0 * PAD_SIDE).max(10.0), (frame.h - 2.0 * PAD_TOP).max(10.0));
            let scene = kit.ui.scene.clone();
            let mut content = lift.adopting(engine, &scene, || Content::new(kit, &e.kind, cw, ch, Z_CARDS + 2, &home.theme, &entry)).ok_or_else(|| std::io::Error::other("the card's group"))??;
            content.show(Props::at(PAD_SIDE, PAD_TOP));
            let inner_rows = match &content {
                Content::AtHome(a) => a.rows(),
                Content::Cameras(c) => c.rows(),
                Content::OnHome(c) => c.visible(),
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
                let base = kit.ui.look.colour(PaletteRole::Glass);
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
        Ok(DockSurface { lift, frame, viewport, grid, halo, fade, tiles: Vec::new(), pill, pill_text, pill_for: None, cols: home.dock.cols(), scroll: 0.0, key: None, arts: Vec::new(), halo_accent: None, halo_at: None, pane_glass: None })
    }

    /// The tiles, made again when the dock's list changes, and after a shed ([`HomeSurface::set_shed`]) from the pictures kept for them: a
    /// picture is a decode and two resamples.
    fn build_dock_tiles(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home) -> std::io::Result<()> {
        if self.shed {
            return Ok(());
        }
        let Some(d) = &mut self.dock else { return Ok(()) };
        let key = Some((home.dock.rev, home.dock.cols()));
        if key == d.key {
            return Ok(());
        }
        // the tiles made again after a shed lift the focus again (`update_focus`)
        if d.key.is_none() && self.last_focus.is_some_and(|f| f.0 >= model::DOCK_ID && f.0 < model::DOCK_ID + model::DOCK_MAX) {
            self.last_focus = None;
        }
        d.cols = home.dock.cols();
        let mut old: Vec<Option<Tile>> = std::mem::take(&mut d.tiles).into_iter().map(Some).collect();
        let tile = home.dock.tile as f32;
        let lift = lift_scale(tile, kit.tokens.focus.tile_scale);
        let radius = kit.tokens.radius(Radius::Tile);
        let inset = home.theme.pointer("/tiles/inset").and_then(Value::as_f64).unwrap_or(19.0) as f32;
        let mut failed = None;
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
            let group = match kit.ui.scene.group(Some(d.grid.id()), rest) {
                Ok(g) => g,
                Err(e) => {
                    failed = Some(other(e));
                    break;
                }
            };
            let plate_colour = lumatv_tokens::colour(&p.plate).map(|c| c.premultiply(0.0)).unwrap_or(Rgba(42, 40, 51, 255));
            let scene = kit.ui.scene.clone();
            let made = IconArt::key_of(&p.icon, tile as u32, inset as u32, radius, lift);
            let art = match d.arts.iter().position(|a| a.key == made) {
                Some(j) => Some(d.arts.swap_remove(j)),
                None => IconArt::of(&p.icon, self.data.as_deref(), tile as u32, inset as u32, radius, lift),
            };
            let layers = scene.adopting(&group, || -> std::io::Result<_> {
                let shape = Shape { corner_radius: radius, fill: compositor_protocol::Fill::Solid(plate_colour), border_width: 1.0, border: faded(Rgba(255, 255, 255, 255), 0.13) };
                let plate_layer = kit.ui.scene.shape(tile as u32, tile as u32, Z_DOCK + 3, shape, Props::at(0.0, 0.0));
                let icon = match &art {
                    Some(a) => Some(a.layer(kit, Z_DOCK + 4, false)?),
                    None => None,
                };
                Ok((plate_layer, icon))
            });
            let (plate_layer, icon) = match layers {
                Ok(l) => l,
                Err(e) => {
                    d.arts.extend(art);
                    failed = Some(e);
                    break;
                }
            };
            engine.flow.lift(id, Lifted { group, rest, scale: lift, rise: 6.0, lit: None });
            d.tiles.push(Tile { id, plate: plate_layer, icon, lifted_icon: None, art, app: p.id.clone(), slot: (sx, sy) });
        }
        for mut t in old.into_iter().flatten() {
            engine.flow.drop_lift(t.id);
            d.arts.extend(t.art.take());
        }
        // a dock the scene had no room for keeps no key and the pictures of the tiles it lacks: the next apply makes the rest
        if let Some(e) = failed {
            return Err(e);
        }
        d.arts.clear();
        d.key = key;
        Ok(())
    }

    // ---- the status bar
    fn build_status(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now: u64) -> std::io::Result<()> {
        let pill = home.status_pill();
        let frame = Frame::new(pill.x, pill.y, pill.w, pill.h);
        let group = home.status_group();
        let profile = home.profile().cloned().unwrap_or(Value::Null);
        let profile_key = format!("{}|{}|{:?}|{:?}|{:?}", profile, home.status_items().len(), home.chip(), home.mark_frame(), home.status_mark().map(|m| m.0));
        let mend = self.status.as_mut().is_some_and(|s| s.mend.due(&kit.ui.scene, now));
        if !mend && self.status.as_ref().is_some_and(|s| s.profile_key == profile_key && s.frame == frame && s.group == group) {
            return Ok(());
        }
        let mut mend = self.status.as_ref().map(|s| s.mend).unwrap_or_default();
        mend.watch(&kit.ui.scene);
        if let Some(s) = self.status.take() {
            for it in s.items {
                engine.flow.drop_lift(it.id);
            }
        }
        let pane = kit.ui.scene.glass(frame.w as u32, frame.h as u32, Z_STATUS, status_glass(glass_of(kit, PaneKind::Pill, frame.h, false, false), true), Props::at(frame.x, frame.y));
        let mut items = Vec::new();
        let ink = kit.ui.look.ink();
        let frames = home.status_frames();
        for (i, it) in home.status_items().iter().enumerate() {
            let Some(&(x, y, w, h)) = frames.get(i) else { break };
            let id = Id(STATUS_ID + i as u32);
            let kind = it.get("type").and_then(Value::as_str).unwrap_or("").to_owned();
            // the glass lifts by a whole number of pixels either side and what is on it does not lift at all, so no glyph, letter or avatar is ever sampled between pixels
            let lift = Lift::new(kit, engine, id, Frame::new(x, y, w, h), Z_STATUS + 1, PaneKind::Pill, lift_scale(h, kit.tokens.focus.button_scale), 0.0, false)?;
            lift.keep(kit, engine)?;
            lift.pane.restyle(status_glass(glass_of(kit, PaneKind::Pill, h, false, false), false));
            lift.lit.restyle(status_glass(glass_of(kit, PaneKind::Pill, h, true, false), true));
            let scene = kit.ui.scene.clone();
            let (icon, avatar, name, badge) = lift.adopting(engine, &scene, || -> std::io::Result<_> {
                let mut icon = None;
                let mut avatar = None;
                let mut name = None;
                let mut badge = None;
                if kind == "notifications" {
                    let (bw, bh, _) = Self::badge_metrics(kit, "99+");
                    badge = Some(kit.ui.scene.image(bw as u32, bh as u32, Z_STATUS + 4, Props { opacity: 0.0, visible: false, ..Props::at(w - bw, 2.0) })?);
                }
                if kind == "profile" {
                    let chip = home.chip();
                    let side = chip.avatar as u32;
                    let mut av = kit.ui.scene.image(side, side, Z_STATUS + 3, Props::at(chip.inset, chip.inset))?;
                    Self::paint_avatar(kit, &mut av, &profile, side, side as f32 / 2.0);
                    avatar = Some(av);
                    let n = home.profile_name();
                    if !n.is_empty() {
                        let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
                        let tw = width_of(kit, style, &n);
                        name = Some(Text::line_at(kit, chip.name_x, h / 2.0, tw.max(10.0), Z_STATUS + 3, style, &n)?);
                    }
                } else {
                    let glyph = match kind.as_str() {
                        "notifications" => "bell",
                        "cameras" => "camera",
                        "settings" => "settings",
                        "search" => "search",
                        other => it.get("icon").and_then(Value::as_str).unwrap_or(other),
                    };
                    // drawn from its vector source on a whole multiple of Lucide's grid and centred on whole pixels; a glyph the source does not have is the PNG's
                    icon = match crate::kit::icon::crisp_layer(kit, glyph, ink, Z_STATUS + 3, (w / 2.0, h / 2.0))? {
                        Some(layer) => Some(layer),
                        None => {
                            let side = 28u32;
                            crate::kit::icon::layer(kit, glyph, side, ink, Z_STATUS + 3, Props::at(((w - side as f32) / 2.0).round(), ((h - side as f32) / 2.0).round()))?
                        }
                    };
                }
                Ok((icon, avatar, name, badge))
            }).ok_or_else(|| std::io::Error::other("the item's group"))??;
            items.push(StatusItem { id, kind, lift, icon, avatar, name, badge, badge_n: None });
        }
        let mark = Self::build_mark(kit, home)?;
        self.status = Some(StatusBar { pane, frame, group, items, profile_key, mark, at: None, mend });
        Ok(())
    }

    /// The mark, absolutely placed on the bar: the moon centred in a button's square, a snooze alone as its smaller glyph, and the time left after it.
    fn build_mark(kit: &mut Kit, home: &Home) -> std::io::Result<Option<Mark>> {
        let (Some((glyph, words)), Some((x, y, w, h))) = (home.status_mark(), home.mark_frame()) else { return Ok(None) };
        let ink = kit.ui.look.ink();
        let grid = crate::kit::vector_icon::GRID * crate::kit::vector_icon::multiple(kit.text_scale);
        let (icon, icon_at) = if *glyph == crate::dnd::GLYPH {
            let centre = (x + h / 2.0, y + h / 2.0);
            let shift = kit.icons.crisp(glyph, grid).map_or((0, 0), |(_, s)| *s);
            let (ix, iy) = crate::kit::icon::crisp_origin(centre, grid, shift);
            (crate::kit::icon::crisp_layer(kit, glyph, ink, Z_STATUS + 3, centre)?, Props::at(ix, iy))
        } else {
            let side = grid * 5 / 6;
            let at = Props::at((x + (h - side as f32) / 2.0).round(), (y + (h - side as f32) / 2.0).round());
            (crate::kit::icon::layer(kit, glyph, side, ink, Z_STATUS + 3, at)?, at)
        };
        let words = if words.is_empty() {
            None
        } else {
            let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
            let tx = home.mark_text_x();
            Some(Text::line_at(kit, x + tx, y + h / 2.0, w - tx, Z_STATUS + 3, style, words)?)
        };
        Ok(Some(Mark { icon, icon_at, words, hidden: None }))
    }

    /// The mark's time left, written again when the minute changes (the bar is laid out again only when its width does).
    fn update_mark(&mut self, kit: &mut Kit, home: &Home) {
        let Some(words) = self.status.as_mut().and_then(|s| s.mark.as_mut()).and_then(|m| m.words.as_mut()) else { return };
        if let Some((_, now)) = home.status_mark()
            && words.text() != now
        {
            words.set(kit, now);
        }
    }

    /// The pill of the bell's count for `text`: its width, its height and the type of the number. The height and the type grow with the
    /// text size, the width with the number.
    fn badge_metrics(kit: &mut Kit, text: &str) -> (f32, f32, ui::text::Type) {
        let scale = kit.text_scale.max(1.0);
        let mut ty = kit.ty(TypeRole::Label);
        ty.px = (15.0 * scale).round() as u32;
        ty.weight = Weight::SemiBold;
        let h = (22.0 * scale).round();
        let w = h.max(kit.ui.fonts.measure(text, ty).width.ceil() + 12.0 * scale);
        (w, h, ty)
    }

    /// The bell's badge for `n` unread: hidden at none, otherwise the count (99+ past a hundred) in the danger colour, with a light ring
    /// on the light material.
    fn update_badge(&mut self, kit: &mut Kit, home: &Home) {
        let n = home.unread;
        let Some(item) = self.status.as_mut().and_then(|s| s.items.iter_mut().find(|i| i.badge.is_some())) else { return };
        if item.badge_n == Some(n) {
            return;
        }
        item.badge_n = Some(n);
        let Some(layer) = item.badge.as_mut() else { return };
        let (w, h) = (layer.width(), layer.height());
        let at = Props::at(item.lift.frame.w - w as f32, 2.0);
        if n == 0 {
            layer.set(Props { opacity: 0.0, visible: false, ..at });
            return;
        }
        let text = if n > 99 { "99+".to_owned() } else { n.to_string() };
        let (pw, ph, ty) = Self::badge_metrics(kit, &text);
        let fill = kit.ui.look.colour(PaletteRole::Danger);
        layer.clear();
        {
            let mut canvas = super::widgets::Canvas { px: layer.pixels_mut(), w, h };
            let pill = Frame::new(w as f32 - pw, 0.0, pw, ph);
            if kit.mode == Mode::Light {
                canvas.rounded(pill, ph / 2.0, Rgba(255, 255, 255, 230));
                canvas.rounded(Frame::new(pill.x + 2.0, 2.0, pw - 4.0, ph - 4.0), (ph - 4.0) / 2.0, fill);
            } else {
                canvas.rounded(pill, ph / 2.0, fill);
            }
        }
        let extent = kit.ui.fonts.measure(&text, ty);
        let x = (w as f32 - pw + (pw - extent.width) / 2.0).round();
        let baseline = ((ph - extent.height()) / 2.0 + extent.ascent).round();
        let mut canvas = ui::text::Canvas { pixels: layer.pixels_mut(), width: w as usize };
        kit.ui.fonts.draw(&mut canvas, &text, ty, (x, baseline), Rgba(255, 255, 255, 255));
        layer.damage(None);
        layer.set(Props { opacity: 1.0, visible: true, ..at });
    }

    /// The avatar with corners of `radius` (the picker's tile a fifth of the side, the status chip half of it, a circle): the person's
    /// picture, or their colour and letter in the ink that reads on it, centred by the letter's own ink.
    pub fn paint_avatar(kit: &mut Kit, layer: &mut ImageLayer, profile: &Value, side: u32, radius: f32) {
        let colour = profile.get("color").and_then(Value::as_str).and_then(lumatv_tokens::colour).map(|c| c.premultiply(0.0)).unwrap_or(Rgba(115, 96, 231, 255));
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
            let mut ty = kit.ty(TypeRole::Label);
            ty.px = (side as f32 * 0.5).round() as u32;
            ty.weight = Weight::SemiBold;
            let extent = kit.ui.fonts.measure(initial, ty);
            let (x, baseline) = (((side as f32 - extent.width) / 2.0).round(), ((side as f32 - extent.height()) / 2.0 + extent.ascent).round());
            // the letter drawn once on nothing to find where its ink is, then again moved so that the ink's centre is the disc's
            let n = side as usize;
            let mut probe = vec![0u8; n * n * 4];
            kit.ui.fonts.draw(&mut ui::text::Canvas { pixels: &mut probe, width: n }, initial, ty, (x, baseline), Rgba(255, 255, 255, 255));
            let (mut lo, mut hi) = ((n, n), (0usize, 0usize));
            for (i, px) in probe.chunks_exact(4).enumerate() {
                if px[3] > 0 {
                    let (px_x, px_y) = (i % n, i / n);
                    lo = (lo.0.min(px_x), lo.1.min(px_y));
                    hi = (hi.0.max(px_x + 1), hi.1.max(px_y + 1));
                }
            }
            let (dx, dy) = if hi.0 > lo.0 { (((side as f32 - (lo.0 + hi.0) as f32) / 2.0).round(), ((side as f32 - (lo.1 + hi.1) as f32) / 2.0).round()) } else { (0.0, 0.0) };
            let mut canvas2 = ui::text::Canvas { pixels: layer.pixels_mut(), width: n };
            kit.ui.fonts.draw(&mut canvas2, initial, ty, (x + dx, baseline + dy), crate::kit::avatar::ink_on(colour));
        }
        layer.damage(None);
    }

    // ---- the contents
    fn update_contents(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now: u64) {
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
                Content::Weather(w) => {
                    // the words and the picture of the card, and the sky's sprites, are made inside the card's group, which a lift of its own owns
                    let twelve = home.theme.pointer("/clock/twelveHour").and_then(Value::as_bool) != Some(false);
                    let scene = kit.ui.scene.clone();
                    let pictures = &mut self.pictures;
                    let _ = c.lift.adopting(engine, &scene, || {
                        w.set(kit, pictures, &home.weather, &home.weather_more, twelve);
                        w.set_art(kit, &home.weather);
                    });
                }
                Content::News(n) => n.set(kit, &home.news, if mine { inner_index.unwrap_or(0) } else { self.news_index }, now),
                Content::AtHome(a) => a.set(kit, &home.ha_items(), &home.ha_lane(), row, &mut self.pictures),
                Content::Cameras(cm) => cm.set(kit, &home.cameras, &home.stills, row, now),
                Content::OnHome(oh) => {
                    let Some(smart) = home.smart_of(&c.id) else { continue };
                    let mut cx = super::onhome::card::Cx { kit, engine, lift: &c.lift, pictures: &mut self.pictures, stills: &home.stills, now_ms: now, now_ns: Self::now_ns() };
                    // a shed card lifts no item (`set_shed`): its contents still change under the kind that covers it
                    let entered = ((mine && home.layer == Layer::None) || home.items_editing(&c.id)) && !self.shed;
                    let view = super::onhome::card::View { smart, topic: &home.onhome, entered, notice: home.onhome_notice(), hold_ms: home.hold_ms() };
                    let _ = oh.sync(&mut cx, &view);
                }
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
        let focused = home.focus.current().and_then(|id| id.0.checked_sub(model::DOCK_ID).filter(|i| *i < model::DOCK_MAX).map(|i| i as usize));
        let scene = kit.ui.scene.clone();
        for (i, t) in d.tiles.iter_mut().enumerate() {
            let (sx, sy) = home.dock.slot(i);
            if t.slot != (sx, sy) {
                t.slot = (sx, sy);
                let rest = GroupPose { x: sx as f32, y: sy as f32, pivot_x: home.dock.tile as f32 / 2.0, pivot_y: home.dock.tile as f32 / 2.0, ..GroupPose::IDENTITY };
                let _ = engine.flow.place(t.id, rest);
            }
            // the lifted icon and the lit group its lift fades in: made for the tile the focus is on (before `update_focus` lifts it) and kept
            // until the tile is back at rest
            let lit = t.art.is_some() && (focused == Some(i) || !engine.flow.resting(t.id, now_ns));
            if lit == t.lifted_icon.is_some() {
                continue;
            }
            if !lit {
                t.lifted_icon = None;
                drop(engine.flow.set_lit(t.id, None));
                continue;
            }
            let (Some(a), Some(parent)) = (&t.art, engine.flow.group(t.id).map(|g| g.id())) else { continue };
            let Ok(group) = scene.group(Some(parent), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }) else { continue };
            if let Ok(layer) = scene.adopting(&group, || a.layer(kit, Z_DOCK + 5, true)) {
                t.lifted_icon = Some(layer);
                engine.flow.set_lit(t.id, Some(group));
            }
        }
        // the glow behind the focused tile and the name under it
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
                // The name comes only once the focus has rested (`Tile.qml`: the name pill comes after `rested`, so a glide across the dock makes none): a
                // pill made for every tile the focus crosses is a pane and a line of text made and dropped at each step.
                if !home.focus_view().rested {
                    if d.pill_for.take().is_some() {
                        d.pill.set(HIDDEN);
                        d.pill_text.set_props(HIDDEN);
                    }
                } else if d.pill_for != Some(id) {
                    d.pill_for = Some(id);
                    let ink = kit.ui.look.ink();
                    let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
                    // the name stays on the dock's card: a long one is cut to its width and a pill under a tile at the end of a row is moved in
                    const EDGE: f32 = 8.0;
                    let w = (width_of(kit, style, &title) + 26.0).round().max(40.0).min(d.frame.w - 2.0 * EDGE);
                    let lift = home.dock.tile as f32 * (kit.tokens.focus.tile_scale - 1.0) / 2.0;
                    // the pill grows with the type (a 43 pill cut a 200 % name) and goes where the card has room for it
                    let ph = (kit.line(TypeRole::Label) + kit.space(1)).max(43.0).ceil();
                    let y = home.dock.label_top(i, ph, lift, 2.0);
                    let x = (cx - w / 2.0).round().clamp(EDGE, (d.frame.w - EDGE - w).max(EDGE));
                    let scene = kit.ui.scene.clone();
                    let (pill, text) = scene.adopting(&d.viewport, || {
                        let pill = kit.ui.scene.glass(w as u32, ph as u32, Z_DOCK + 9, glass_of(kit, PaneKind::Control, ph, false, false), Props::at(x, y));
                        let text = Text::new(kit, Frame::new(x, y, w, ph), Z_DOCK + 10, style, &title).map(|mut t| {
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
        let is_dock = |id: Id| id.0 >= model::DOCK_ID && id.0 < model::DOCK_ID + model::DOCK_MAX;
        let moved = shown != self.last_focus;
        if moved {
            let _ = engine.flow.go(self.last_focus, shown, now_ns);
            self.last_focus = shown;
        }
        let on_dock = current.is_some_and(is_dock);
        if let Some(d) = &mut self.dock {
            // the dock's glass is lit while the focus is on it (the cards' own lifts light theirs); said at every apply (a dock made again, or
            // restyled for a video behind, has it back), restyled only when that changes: the same glass at every apply made a commit, and a
            // frame, of every apply that changed nothing else
            let glass = glass_of(kit, PaneKind::Card, d.frame.h, on_dock, self.over_video);
            if d.pane_glass != Some(glass) {
                d.pane_glass = Some(glass);
                d.lift.pane.restyle(glass);
            }
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
    fn update_layers(&mut self, kit: &mut Kit, engine: &mut Engine, home: &mut Home, now: u64) -> std::io::Result<()> {
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
        self.library_icons.prepare(home.apps.all(), kit.tokens.radius(Radius::Tile));
        if let Some(a) = &mut self.apps {
            a.update(kit, engine, home, &self.library_icons, now_ns)?;
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
        let confirm_up = home.layer == Layer::Confirm && home.confirm.is_some();
        match (confirm_up, &mut self.confirm) {
            (true, None) => self.confirm = Some(ConfirmSurface::new(kit, engine, home, Z_LAYERS + 95)?),
            (false, Some(_)) => {
                if let Some(c) = self.confirm.take() {
                    c.destroy(engine);
                }
            }
            _ => {}
        }
        if let Some(c) = &mut self.confirm {
            c.update(kit, home);
        }
        // the status bar's settings popover: made when it opens and when its rows scroll; the wallpaper behind is the preview, so the page steps back
        // the full weather view
        match (home.layer == Layer::Weather, &mut self.weather_view) {
            (true, None) => self.weather_view = Some(super::weather_view::WeatherSurface::new(kit, home, Z_LAYERS + 30)?),
            (false, Some(_)) => self.weather_view = None,
            _ => {}
        }
        if let Some(w) = &mut self.weather_view {
            let run = home.weather_view_runs() && !kit.ui.look.reduced_motion;
            w.update(kit, home, &mut self.pictures, now, run)?;
            self.view_art_at = w.due_at(now, run);
        } else {
            self.view_art_at = None;
        }
        // one smart-home tile's controls: made when it opens and again when its rows change shape (a light that goes off loses its level)
        match (home.layer == Layer::OnHome, home.sheet.as_ref().zip(home.sheet_tile())) {
            (true, Some((sheet, tile))) => {
                let rows = sheet.rows(&tile, now);
                let id = sheet.id.clone();
                if self.onhome_sheet.as_ref().is_none_or(|s| !s.is_for(&id, &rows)) {
                    self.onhome_sheet = Some(super::onhome::SheetSurface::new(kit, &id, &rows, Z_LAYERS + 40)?);
                }
                if let Some(s) = &mut self.onhome_sheet {
                    let ask = sheet.ask(&tile);
                    let notice = sheet.notice.as_ref().map(|(w, _)| w.as_str()).or(ask.as_deref());
                    s.update(kit, &tile, &rows, sheet.front(&rows), notice);
                }
            }
            _ => self.onhome_sheet = None,
        }
        // a sensor's history: made when it opens (another sensor, or another kind of one, is another sheet) and filled when the helper answers
        match (home.layer == Layer::OnHome, home.history.as_ref().zip(home.sheet_tile())) {
            (true, Some((sheet, tile))) => {
                if self.history_sheet.as_ref().is_none_or(|s| !s.is_for(&sheet.id, &tile)) {
                    self.history_sheet = Some(super::onhome::sensor_surface::SensorSurface::new(kit, sheet, &tile, Z_LAYERS + 40)?);
                }
                if let Some(s) = &mut self.history_sheet {
                    let twelve = home.theme.pointer("/clock/twelveHour").and_then(Value::as_bool) != Some(false);
                    s.update(kit, sheet, &tile, twelve)?;
                }
            }
            _ => self.history_sheet = None,
        }
        // a lamp's colours: made when it opens and again when its rows change (a lamp that loses its colour)
        match (home.layer == Layer::OnHome, home.colour.as_ref().zip(home.sheet_tile())) {
            (true, Some((sheet, tile))) => {
                if self.colour_sheet.as_ref().is_none_or(|s| !s.is_for(&sheet.id, sheet)) {
                    self.colour_sheet = Some(super::onhome::colour_surface::ColourSurface::new(kit, sheet, &tile, Z_LAYERS + 40)?);
                }
                if let Some(s) = &mut self.colour_sheet {
                    s.update(kit, sheet, &tile, home.onhome_notice())?;
                }
            }
            _ => self.colour_sheet = None,
        }
        // a thermostat's set point and modes: made when it opens and again when its shape changes (one that is switched off loses its set point)
        match (home.layer == Layer::OnHome, home.thermostat.as_ref().zip(home.sheet_tile())) {
            (true, Some((sheet, tile))) => {
                if self.thermostat_sheet.as_ref().is_none_or(|s| !s.is_for(&sheet.id, sheet)) {
                    self.thermostat_sheet = Some(super::onhome::thermostat_surface::ThermostatSurface::new(kit, sheet, &tile, Z_LAYERS + 40)?);
                }
                if let Some(s) = &mut self.thermostat_sheet {
                    s.update(kit, sheet, &tile, home.onhome_notice())?;
                }
            }
            _ => self.thermostat_sheet = None,
        }
        let popover_up = home.layer == Layer::Popover && home.popover_lay.is_some();
        if self.popover.as_ref().is_some_and(|p| !popover_up || !p.is_for(home)) {
            self.popover = None;
        }
        if popover_up && self.popover.is_none() {
            self.popover = Some(PopoverSurface::new(kit, home, Z_LAYERS + 100)?);
        }
        if let Some(p) = &mut self.popover {
            p.update(kit, home)?;
        }
        // while a layer that hides the page is up, the cards and the dock step back
        let page_hidden = matches!(home.layer, Layer::Apps | Layer::Picker | Layer::Popover | Layer::OnHome) || (home.layer == Layer::Search && home.search_over_apps());
        if page_hidden != self.page_hidden {
            self.page_hidden = page_hidden;
            for c in &self.cards {
                c.lift.pose(engine, 1.0, !page_hidden);
            }
            if let Some(d) = &self.dock {
                d.lift.pose(engine, 1.0, !page_hidden);
            }
        }
        if let Some(s) = &mut self.status {
            // the popover hangs under the bar, which stays
            let hidden = (page_hidden && home.layer != Layer::Popover) || home.layer == Layer::Edit || home.layer == Layer::Search;
            let at = Props { opacity: if hidden { 0.0 } else { 1.0 }, visible: !hidden, ..Props::at(s.frame.x, s.frame.y) };
            if s.at != Some(at) {
                s.at = Some(at);
                s.pane.set(at);
            }
            for it in &s.items {
                it.lift.pose(engine, 1.0, !hidden);
            }
            if let Some(m) = &mut s.mark
                && m.hidden != Some(hidden)
            {
                m.hidden = Some(hidden);
                let shown = |at: Props| Props { opacity: if hidden { 0.0 } else { 1.0 }, visible: !hidden, ..at };
                if let Some(icon) = &m.icon {
                    icon.set(shown(m.icon_at));
                }
                if let Some(words) = &mut m.words {
                    let at = words.at();
                    words.set_props(shown(at));
                }
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
        if home.layer == Layer::Apps {
            if let Some(apps) = &self.apps { apps.lint(kit, &mut lint); }
            return lint;
        }
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
            // The `clear` glass is the picture's own blur under the tint, with no scrim over it (as the QML Home's): a pane's colour is the tint over the
            // picture, which is its mean grey here; `frosted` is the tint over the scrimmed canvas.
            let under = match self.wall.sample.filter(|s| s.luma >= 0.0 && wallpaper::clear_glass(&home.theme)) {
                Some(s) => {
                    let g = (s.luma * 255.0).round() as u8;
                    Rgba(g, g, g, 255)
                }
                None => over(Rgba(canvas.0, canvas.1, canvas.2, 255), Rgba(0, 0, 0, (scrim * 255.0) as u8)),
            };
            over(under, kit.ui.look.colour(PaletteRole::Glass))
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
        // the page is stepped back behind the popover: its words are not on the screen
        for c in self.cards.iter().filter(|_| !matches!(home.layer, Layer::Popover | Layer::OnHome)) {
            let p = lint.panel(&c.id, c.frame, paint, true);
            c.content.words().lint(kit, &mut lint, &c.id, (c.frame.x, c.frame.y), Some(p));
            if let (Content::OnHome(oh), Some(s)) = (&c.content, home.smart_of(&c.id)) {
                let (g, o) = (&s.plan().grid, s.offset() as f32);
                let (sx, sy) = g.stride();
                let off = if g.axis == super::onhome::layout::Axis::Horizontal { (o * sx, 0.0) } else { (0.0, o * sy) };
                oh.lint(kit, &mut lint, &c.id, c.frame, off, g.bounds(), paint);
            }
        }
        if let Some(d) = self.dock.as_ref().filter(|_| !matches!(home.layer, Layer::Popover | Layer::OnHome)) {
            let p = lint.panel("dock", d.frame, paint, true);
            if d.pill_for.is_some() {
                let f = d.pill_text.frame();
                let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Centre);
                lint.word(kit, "dock.pill", d.pill_text.text(), Frame::new(d.frame.x + f.x, d.frame.y + f.y, f.w, f.h), style, Some(p));
            }
        }
        if let Some(p) = &self.picker {
            p.lint(kit, &mut lint, home);
        }
        if let Some(apps) = &self.apps {
            apps.lint(kit, &mut lint);
        }
        if let Some(s) = &self.onhome_sheet {
            s.lint(kit, &mut lint, paint);
        }
        if let Some(s) = &self.history_sheet {
            s.lint(kit, &mut lint, paint);
        }
        if let Some(s) = &self.colour_sheet {
            s.lint(kit, &mut lint, paint);
        }
        if let Some(s) = &self.thermostat_sheet {
            s.lint(kit, &mut lint, paint);
        }
        if let Some(p) = &self.popover {
            p.lint(kit, &mut lint, paint, home);
        }
        lint
    }

    /// Where things are, for the harness and the tests.
    pub fn geometry(&self, home: &Home) -> Value {
        json!({
            "cards": self.cards.iter().map(|c| json!({"id": c.id, "x": c.frame.x, "y": c.frame.y, "w": c.frame.w, "h": c.frame.h})).collect::<Vec<_>>(),
            "dock": self.dock.as_ref().map(|d| json!({"x": d.frame.x, "y": d.frame.y, "w": d.frame.w, "h": d.frame.h, "tiles": d.tiles.len(), "cols": d.cols})),
            "status": self.status.as_ref().map(|s| s.items.iter().map(|i| json!({"kind": i.kind, "x": i.lift.frame.x, "w": i.lift.frame.w})).collect::<Vec<_>>()),
            "pickerOrigin": self.picker.as_ref().map(|p| p.origin()),
            "wallpaper": self.wall.is_up(),
            "focus": home.focus.current().map(|i| i.0),
            "glyphRuns": self.glyph_runs,
            "apps": self.apps.is_some(), "search": self.search.is_some(), "editor": self.editor.is_some(), "picker": self.picker.is_some(), "menu": self.menu.is_some(),
            "popover": self.popover.as_ref().map(PopoverSurface::geometry),
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use super::super::weather_view::Page;
    use serde_json::json;

    #[test]
    fn auto_returns_to_cached_source_mode_after_preview_and_threshold_change() {
        for (pixel, expected, preview) in [(32u8, Mode::Dark, Mode::Light), (224u8, Mode::Light, Mode::Dark)] {
            let file = std::env::temp_dir().join(format!("lumatv-auto-{}-{pixel}.png", std::process::id()));
            let mut enc = png::Encoder::new(std::fs::File::create(&file).unwrap(), 1, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&[pixel, pixel, pixel, 255]).unwrap();
            let (h, mut kit) = testkit::kit_in(expected, 1.0);
            let mut engine = Engine::new(&kit).unwrap();
            let raw = json!({"material": "auto", "materialThreshold": 0.6, "wallpaper": file.to_str().unwrap(),
                "wallpaperPalette": {"dark": ["#000000", "#111111", "#222222"], "light": ["#aaaaaa", "#bbbbbb", "#ffffff"]}});
            let auto = lumatv_tokens::Tokens::from_json(&raw);
            kit.retheme(auto.clone(), expected, 1.0);
            let mut home = Home::new(auto.json().clone(), super::super::grid::shipped(), json!({}));
            let mut surface = HomeSurface::new(1000);
            surface.apply(&mut kit, &mut engine, &mut home, 1).unwrap();
            let sample = surface.wall.sample.unwrap();
            let tint = home.tint.clone();
            let mut manual = raw.clone();
            manual["material"] = json!(if preview == Mode::Light { "light" } else { "dark" });
            let tokens = lumatv_tokens::Tokens::from_json(&manual);
            kit.retheme(tokens.clone(), preview, 1.0);
            home.set_theme(tokens.json().clone());
            surface.apply(&mut kit, &mut engine, &mut home, 2).unwrap();
            assert_eq!(kit.mode, preview);
            kit.retheme(auto.clone(), preview, 1.0);
            home.set_theme(auto.json().clone());
            surface.apply(&mut kit, &mut engine, &mut home, 3).unwrap();
            assert_eq!(kit.mode, expected);
            let key = if kit.mode == Mode::Dark { "dark" } else { "light" };
            assert_eq!(kit.tokens.json()["wallpaperPalette"][key], raw["wallpaperPalette"][key]);
            assert_eq!(surface.wall.sample, Some(sample));
            assert_eq!(home.tint, tint);
            let generation = kit.generation();
            surface.apply(&mut kit, &mut engine, &mut home, 4).unwrap();
            assert_eq!(kit.generation(), generation);
            let mut changed = raw.clone();
            changed["materialThreshold"] = json!(if expected == Mode::Dark { 0.0 } else { 1.0 });
            let tokens = lumatv_tokens::Tokens::from_json(&changed);
            kit.retheme(tokens.clone(), expected, 1.0);
            home.set_theme(tokens.json().clone());
            surface.apply(&mut kit, &mut engine, &mut home, 5).unwrap();
            assert_eq!(kit.mode, preview);
            std::fs::remove_file(file).unwrap();
            std::mem::forget(h);
        }
    }

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
        h.update(&Input::Helper { topic: "plugins".into(), data: json!({"plugins": [{"id": "home-assistant", "state": "connected"}], "firstParty": []}) }, 1003);
        let cards = json!({"items": [{"id": "living", "revision": 1, "card": {"content": {"title": "Living room", "icon": "base.icons/lightbulb", "role": "warning", "facts": [{"label": "Lights", "value": "3 on"}]}}},
            {"id": "door", "revision": 1, "card": {"content": {"title": "Front door", "icon": "base.icons/lock", "role": "positive", "facts": [{"label": "State", "value": "Locked"}]}}}]});
        h.update(&Input::Helper { topic: "plugin.home-assistant.cards".into(), data: cards }, 1004);
        let profiles = json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "switchedAt": 1, "profiles": [
            {"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false}, {"id": "isabel", "name": "Isabel", "initial": "I", "color": "#EB6F92", "active": false, "locked": false}]});
        h.update(&Input::Helper { topic: "profiles".into(), data: profiles }, 1005);
    }

    #[test]
    fn a_lifted_tile_has_whole_pixel_edges_and_its_icon_is_a_picture_of_the_lifted_size() {
        let lift = lift_scale(120.0, 1.12);
        assert!((120.0 * lift - 134.0).abs() < 1e-3, "120 px at 1.12 is 134.4: 134 keeps its edges on whole pixels, got {}", 120.0 * lift);
        assert_eq!(lift_scale(120.0, 1.0), 1.0, "no lift stays none");
        assert_eq!(lift_scale(0.0, 1.12), 1.12);
        assert_eq!(lifted_icon_side(120, lift), 134, "a full-bleed icon is drawn 1:1 at full lift");
        assert_eq!(lifted_icon_side(82, lift), 92, "an inset icon's side is even, so its edges are whole about the tile's centre");
        assert_eq!(lifted_icon_side(120, 1.0), 120, "no lift: the box");
    }

    #[test]
    fn a_tile_icon_is_made_at_the_box_and_at_the_lifted_box_from_the_files_own_pixels() {
        let dir = std::env::temp_dir().join(format!("iconart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("icon.png");
        let mut enc = png::Encoder::new(std::fs::File::create(&file).expect("a file"), 192, 192);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..192 * 192).flat_map(|i| if (i % 192) < 96 { [255, 255, 255, 255] } else { [0, 0, 0, 255] }).collect();
        enc.write_header().expect("header").write_image_data(&pixels).expect("pixels");
        let art = IconArt::of(file.to_str().expect("utf-8"), None, 120, 19, 24.0, lift_scale(120.0, 1.12)).expect("an icon");
        assert_eq!((art.rest.width, art.lifted.width), (120, 134), "full bleed: the box and the lifted box");
        assert_eq!(art.off, 0.0);
        let row = |p: &Picture| (0..p.width).map(|x| p.at(x, p.height / 2)[0]).collect::<Vec<_>>();
        for (name, p) in [("rest", &art.rest), ("lifted", &art.lifted)] {
            let r = row(p);
            let soft = r[10..r.len() - 10].iter().filter(|&&v| v > 20 && v < 235).count();
            assert!(soft <= 2, "{name}: the edge is a pixel or two wide, not a ramp: {soft}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    fn press(h: &mut Home, name: &str, at: u64) {
        h.update(&Input::Key { name: name.into(), down: true, repeat: false }, at);
        h.update(&Input::Key { name: name.into(), down: false, repeat: false }, at + 30);
    }

    fn lint_home(mode: Mode, scale: f32, layout: Value) -> (Vec<crate::kit::lint::Finding>, usize) {
        lint_home_with(mode, scale, layout, false)
    }

    /// The reading of the weather with its hours (the card then has the next thing and the hours, not the week).
    fn weather_hours(h: &mut Home, alert: bool) {
        let hours: Vec<Value> = (0..12).map(|i| json!({"time": format!("2026-10-30T{:02}:00", (15 + i) % 24), "temp": 61.0 - f64::from(i), "rainPct": if i > 2 { 80.0 } else { 10.0 }, "code": if i > 2 { 61 } else { 3 }, "isDay": true})).collect();
        let alerts = if alert { json!([{"event": "Flood Watch", "severity": "Moderate", "headline": "Flood Watch", "ends": "2026-10-31T08:00:00-04:00"}]) } else { json!([]) };
        h.update(&Input::Helper { topic: "weather.more".into(), data: json!({"doc": {"fetchedAt": 6, "hourly": hours, "daily": [], "alerts": alerts}}) }, 1002);
    }

    /// A whole reading for the view's pages (the wind makes it feel colder: the longest of the Details words).
    fn weather_page(h: &mut Home) {
        let days: Vec<Value> = [("Today", "rain", 71, 51), ("Sat", "cloud", 73, 51), ("Sun", "rain", 62, 48), ("Mon", "partly", 66, 56), ("Tue", "drizzle", 59, 44)]
            .iter()
            .map(|(d, k, hi, lo)| json!({"day": d, "kind": k, "hi": format!("{hi}\u{b0}"), "lo": format!("{lo}\u{b0}")}))
            .collect();
        let doc = json!({"doc": {"fetchedAt": 7, "lat": 33.75, "lon": -84.39, "place": "Atlanta", "current": {"time": "2026-10-30T15:00", "temp": 61.0, "feelsLike": 57.0, "code": 63, "isDay": true, "humidity": 53.0, "windMph": 12.0},
            "display": {"place": "Atlanta", "temp": "61\u{b0}", "cond": "Rain", "stats": "Feels like 57\u{b0} \u{b7} 53% humidity \u{b7} 12 mph", "nowKind": "rain", "days": days}}, "state": "fresh"});
        h.update(&Input::Helper { topic: "weather".into(), data: doc }, 1006);
        let hours: Vec<Value> = (0..24).map(|i| json!({"time": format!("2026-10-30T{:02}:00", (15 + i) % 24), "temp": 61.0 - f64::from(i % 9), "rainPct": if i < 8 { 70.0 } else { 10.0 }, "code": 63, "isDay": true, "windMph": 12.0})).collect();
        let daily: Vec<Value> = (0..10).map(|i| json!({"date": format!("2026-11-{:02}", i + 1), "sunrise": "2026-10-30T07:41", "sunset": "2026-10-30T18:25", "uvMax": 4.0, "rainPct": 40.0, "rainIn": 0.2, "code": 61, "hi": 70.0, "lo": 50.0})).collect();
        let more = json!({"doc": {"fetchedAt": 7, "now": {"uvIndex": 4.0, "gustMph": 24.0, "windDeg": 225.0, "pressureHpa": 1013.0, "cloudPct": 40.0, "visibilityM": 24000.0, "rainIn": 0.0}, "hourly": hours, "daily": daily,
            "alerts": [{"event": "Flood Watch", "severity": "Moderate", "headline": "Flood Watch until Friday morning", "ends": "2026-10-31T08:00:00-04:00"}]}});
        h.update(&Input::Helper { topic: "weather.more".into(), data: more }, 1007);
    }

    #[test]
    fn the_weather_view_cuts_no_words_at_either_text_size_in_either_look() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let theme = lumatv_tokens::Tokens::shipped().json().clone();
                let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                weather_page(&mut home);
                for (i, key) in ["down", "right", "right", "ok"].into_iter().enumerate() {
                    press(&mut home, key, 2000 + 100 * i as u64);
                }
                assert_eq!(home.layer, Layer::Weather, "{mode:?} at {scale}");
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                for page in 0..3u64 {
                    if page > 0 {
                        press(&mut home, "right", 2500 + 100 * page);
                    }
                    s.apply(&mut kit, &mut engine, &mut home, 2600 + 100 * page).expect("the view drawn");
                    let cut = s.weather_view.as_ref().expect("the view is up").cut_words(&mut kit);
                    assert!(cut.is_empty(), "{mode:?} at {scale}, page {page}: {cut:?}");
                }
                std::mem::forget(h);
            }
        }
    }

    #[test]
    fn a_page_slides_in_from_the_side_the_key_went_and_the_one_it_replaces_is_let_go_when_it_is_out_of_sight() {
        let (h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
        fixtures(&mut home);
        weather_page(&mut home);
        for (i, key) in ["down", "right", "right", "ok"].into_iter().enumerate() {
            press(&mut home, key, 2000 + 100 * i as u64);
        }
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        let paging = |s: &HomeSurface, at: u64| s.weather_view.as_ref().and_then(|w| w.paging(at)).expect("the view is up");
        s.apply(&mut kit, &mut engine, &mut home, 2600).expect("the view drawn");
        assert_eq!(paging(&s, 0), (Page::Now, 0.0, vec![]), "opened in place, with no slide");
        // the offset a page comes in from is read at the moment before the key was applied, when the spring has not yet moved it
        let before = ui::scene::monotonic_ns();
        press(&mut home, "right", 3000);
        s.apply(&mut kit, &mut engine, &mut home, 3010).expect("the view drawn");
        let (page, from, out) = paging(&s, before);
        assert_eq!((page, out, from), (Page::Radar, vec![Page::Now], 1920.0), "Right brings the next page in from the right");
        let before = ui::scene::monotonic_ns();
        press(&mut home, "left", 3100);
        s.apply(&mut kit, &mut engine, &mut home, 3110).expect("the view drawn");
        let (page, from, out) = paging(&s, before);
        assert_eq!((page, out, from), (Page::Now, vec![Page::Now, Page::Radar], -1920.0), "Left brings the one before in from the left");
        s.apply(&mut kit, &mut engine, &mut home, 3900).expect("the view drawn");
        assert_eq!(paging(&s, 0).2, vec![], "what slid out is let go");
        std::mem::forget(h);
    }

    #[test]
    fn the_map_takes_up_the_zoom_the_model_asks_for_and_a_zoom_about_the_pointer_moves_the_picture_to_keep_the_place_under_it() {
        use super::super::radar_zoom::{Z_HOME, Z_MAX, scale_of};
        let (h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
        fixtures(&mut home);
        weather_page(&mut home);
        for (i, key) in ["down", "right", "right", "ok", "right"].into_iter().enumerate() {
            press(&mut home, key, 2000 + 100 * i as u64);
        }
        assert!(home.radar.open, "the map page is up");
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        let taken = |s: &HomeSurface| s.weather_view.as_ref().and_then(|w| w.zoom_taken());
        s.apply(&mut kit, &mut engine, &mut home, 2600).expect("the view drawn");
        assert_eq!(taken(&s).map(|t| t.0), Some(Z_HOME), "taken up at the opening view");
        // a notch about the middle: the target is taken up, and the picture keeps to the middle
        home.update(&Input::Key { name: "wheelup".into(), down: true, repeat: false }, 2700);
        s.apply(&mut kit, &mut engine, &mut home, 2710).expect("the view drawn");
        let (target, pan) = taken(&s).expect("taken up");
        assert_eq!(target, Z_HOME + 0.25);
        assert_eq!(pan.at(scale_of(target)), (0.0, 0.0));
        // a notch with the pointer shown: about it, so the picture is moved the other way from where the pointer is
        home.update(&Input::Pointer { x: 1500.0, y: 300.0, buttons: 0, visible: true }, 2800);
        for i in 0..4 {
            home.update(&Input::Key { name: "wheelup".into(), down: true, repeat: false }, 2900 + i * 10);
        }
        s.apply(&mut kit, &mut engine, &mut home, 2950).expect("the view drawn");
        let (target, pan) = taken(&s).expect("taken up");
        assert_eq!(target, Z_MAX.min(Z_HOME + 0.25 + 1.0));
        let (ox, oy) = pan.at(scale_of(target));
        assert!(ox < 0.0 && oy > 0.0, "the pointer is up and to the right of the middle, so the picture moves down and to the left: {ox} {oy}");
        // the model is told the view the zoom comes to, and the world under the pointer is the one that was under it at the opening view
        let view = home.radar.view().expect("the surface told the model where the zoom puts the view");
        let (lon, lat) = (-84.39_f64, 33.75_f64);
        let place = ((lon + 180.0) / 360.0, 0.5 - ((std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln()) / (2.0 * std::f64::consts::PI));
        let under = |cx: f64, cy: f64, zoom: f64| (cx + (1500.0 - 960.0) / (256.0 * 2f64.powf(zoom)), cy + (300.0 - 540.0) / (256.0 * 2f64.powf(zoom)));
        let (was, is) = (under(place.0, place.1, f64::from(Z_HOME)), under(view.cx, view.cy, view.zoom));
        let pixel = 1.0 / (256.0 * 2f64.powf(view.zoom));
        assert!(view.zoom == f64::from(target) && (was.0 - is.0).abs() < 3.0 * pixel && (was.1 - is.1).abs() < 3.0 * pixel, "{was:?} {is:?} {view:?}");
        std::mem::forget(h);
    }

    fn lint_home_with(mode: Mode, scale: f32, layout: Value, hours: bool) -> (Vec<crate::kit::lint::Finding>, usize) {
        let (h, mut kit) = testkit::kit_in(mode, scale);
        let mut engine = Engine::new(&kit).expect("an engine");
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let mut home = Home::new(theme, super::super::grid::shipped(), layout);
        fixtures(&mut home);
        if hours {
            weather_hours(&mut home, scale < 1.5);
        }
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
    fn under_a_full_screen_sheet_the_contents_are_brought_up_to_date_once_a_second_and_at_once_when_the_cards_are_made_again_or_the_sheet_goes() {
        let (h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut home = Home::new(lumatv_tokens::Tokens::shipped().json().clone(), super::super::grid::shipped(), json!({}));
        fixtures(&mut home);
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        let mut apply = |s: &mut HomeSurface, home: &mut Home, at: u64| s.apply(&mut kit, &mut engine, home, at).expect("Home drawn");
        apply(&mut s, &mut home, 2300);
        apply(&mut s, &mut home, 2310);
        assert_eq!(s.contents_at, 2310, "seen: at every apply");
        home.set_covered(true);
        apply(&mut s, &mut home, 2320);
        assert_eq!(s.contents_at, 2310, "covered: not at every apply");
        // (the sky's own timer asks for frames of its own: what is under test is the clock's second)
        s.art_at = None;
        s.view_art_at = None;
        assert!(!s.contents_due(3050) && !s.tick(3050), "and the clock's second asks for no drawing of Home");
        assert!(s.contents_due(3320) && s.tick(3320), "until the second is up");
        apply(&mut s, &mut home, 3320);
        assert_eq!(s.contents_at, 3320);
        s.restyle();
        apply(&mut s, &mut home, 3330);
        assert_eq!(s.contents_at, 3330, "cards made again have their words at once");
        apply(&mut s, &mut home, 3340);
        assert_eq!(s.contents_at, 3330);
        home.set_covered(false);
        apply(&mut s, &mut home, 3350);
        assert_eq!(s.contents_at, 3350, "and when the sheet goes");
        std::mem::forget(h);
    }

    #[test]
    fn the_weather_card_with_its_hours_and_its_next_thing_overlaps_nothing_at_either_text_size_in_either_look() {
        let halves = json!({"grid": {"rows": 16}, "widgets": [
            {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 6, "rows": 4}, {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 6, "rows": 5, "title": "At home"},
            {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9}, {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for layout in [json!({}), halves.clone()] {
                    let (findings, words) = lint_home_with(mode, scale, layout, true);
                    let bad: Vec<String> = findings.iter().filter(|f| format!("{f:?}").contains("weather")).map(|f| format!("{f:?}")).collect();
                    assert!(bad.is_empty(), "{mode:?} at {scale}: {bad:?}");
                    assert!(words > 0);
                }
            }
        }
    }

    #[test]
    fn the_text_on_the_cards_passes_the_contrast_lint_with_the_clear_and_the_frosted_glass_over_the_darkest_and_the_lightest_pictures_each_material_takes() {
        // (mode, the picture's mean luma): the clear glass keeps the accent words (the date, the news tag) at 3:1 up to a mean luma of 0.5; at 0.55 they read 2.85:1, which a text plate under those two words would mend (not built)
        for (mode, luma) in [(Mode::Dark, 0.05), (Mode::Dark, 0.5), (Mode::Light, 0.61), (Mode::Light, 0.65)] {
            for glass in ["clear", "frosted"] {
                for scale in [1.0, 2.0] {
                    let (h, mut kit) = testkit::kit_in(mode, scale);
                    let mut engine = Engine::new(&kit).expect("an engine");
                    let mut theme = lumatv_tokens::Tokens::shipped().json().clone();
                    lumatv_tokens::merge(&mut theme, &json!({"cardGlass": glass}));
                    let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
                    fixtures(&mut home);
                    let mut s = HomeSurface::new(1000);
                    s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                    s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
                    s.wall.sample = Some(wallpaper::Sample { luma, tint: None });
                    let lint = s.lint(&mut kit, &home);
                    // the two accent words (today's date, the news tag) read under 3:1 on the clear glass over a mid-light picture (2.5 to 2.9): a text plate under just
                    // those words is the mend, not built; every other word of Home passes
                    let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject != "clock.9" && f.subject != "news.0").map(|f| format!("{f:?}")).collect();
                    assert!(bad.is_empty(), "{mode:?} luma {luma} {glass} at {scale}: {}", bad.join("\n"));
                    std::mem::forget(h);
                }
            }
        }
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
    fn the_uninstall_question_is_drawn_on_the_screen_with_both_answers_in_the_card_and_keep_it_in_front_at_both_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut home = Home::new(lumatv_tokens::Tokens::shipped().json().clone(), super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                let lp = json!({"id": "org.webosbrew.hyperhdr.loader", "title": "HyperHDR with a rather long name", "icon": "", "iconColor": "#101010", "removable": true});
                home.update(&Input::Luna { tag: model::LP_TAG, payload: json!({"launchPoints": [lp]}) }, 1500);
                for (i, k) in ["down", "down", "down"].into_iter().enumerate() {
                    press(&mut home, k, 1600 + i as u64 * 100);
                }
                home.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 2000);
                home.update(&Input::Tick, 2900);
                home.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 2950);
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                s.apply(&mut kit, &mut engine, &mut home, 2400).expect("Home drawn");
                assert_eq!(home.layer, Layer::Menu);
                while home.menu[home.menu_index].id != "uninstall" {
                    press(&mut home, "down", 2500);
                }
                press(&mut home, "ok", 2600);
                assert_eq!(home.layer, Layer::Confirm, "{mode:?} {scale}");
                s.apply(&mut kit, &mut engine, &mut home, 2700).expect("the question drawn");
                let c = s.confirm.as_ref().expect("the question");
                assert_eq!(c.frames.len(), 2);
                assert!(c.card.x >= 0.0 && c.card.y >= 0.0 && c.card.right() <= 1920.0 && c.card.bottom() <= 1080.0, "{mode:?} {scale}: {:?}", c.card);
                for f in &c.frames {
                    assert!(f.x >= c.card.x && f.right() <= c.card.right() && f.y >= c.card.y && f.bottom() <= c.card.bottom(), "{mode:?} {scale}: answer {f:?} in {:?}", c.card);
                }
                assert!(c.frames[0].right() <= c.frames[1].x, "the answers do not overlap");
                assert_eq!(home.focus.current(), Some(Id(model::CONFIRM_ID)), "Keep it has the focus");
                // away from the question, the layer and its layers go
                press(&mut home, "back", 2800);
                s.apply(&mut kit, &mut engine, &mut home, 2900).expect("Home drawn");
                assert!(s.confirm.is_none());
                std::mem::forget(h);
            }
        }
    }

    fn smart_tiles(n: usize) -> Value {
        let kinds = [
            json!({"id": "light.lamp", "name": "Kitchen main light", "area": "Kitchen", "group": "lights", "words": "On \u{b7} 50%", "tone": "on", "icon": "lightbulb", "available": true, "can": ["toggle"], "verbs": ["turn_on", "turn_off", "toggle"],
                   "ctl": {"on": true, "dimmable": true, "brightness": 50, "kelvin": {"now": 3000, "min": 2000, "max": 6500}}}),
            json!({"id": "light.desk", "name": "Desk", "area": "Office", "group": "lights", "words": "Off", "tone": "off", "icon": "lightbulb", "available": true, "can": ["toggle"], "verbs": ["toggle"], "ctl": {"on": false, "dimmable": false}}),
            json!({"id": "scene.movie", "name": "Movie night", "group": "scenes", "words": "Scene", "tone": "muted", "icon": "sparkles", "available": true, "can": ["run"], "verbs": ["turn_on"], "ctl": {}}),
            json!({"id": "climate.t", "name": "Living room thermostat", "area": "Living room", "group": "climate", "words": "68\u{b0} \u{b7} Heating to 70\u{b0}", "tone": "warn", "icon": "thermometer", "available": true, "can": ["sheet"],
                   "verbs": ["set_temperature", "set_hvac_mode"], "ctl": {"mode": "heat", "modes": ["off", "heat", "cool"], "current": 68.0, "target": 70.0, "min": 50.0, "max": 90.0, "step": 1.0, "unit": "\u{b0}F", "action": "heating"}}),
            json!({"id": "cover.b", "name": "Master bedroom blinds", "group": "covers", "words": "Open \u{b7} 60%", "tone": "on", "icon": "blinds", "available": true, "can": ["sheet"], "verbs": ["open_cover", "close_cover", "stop_cover", "set_cover_position"], "ctl": {"state": "open", "position": 60}}),
            json!({"id": "sensor.t", "name": "Hall temperature", "area": "Hall", "group": "sensors", "words": "21.4\u{b0}C", "tone": "info", "icon": "thermometer", "available": true, "can": [], "verbs": [], "ctl": {}}),
            json!({"id": "binary_sensor.d", "name": "Front door", "group": "sensors", "words": "Closed", "tone": "off", "icon": "door-closed", "available": true, "can": [], "verbs": [], "ctl": {}}),
            json!({"id": "lock.f", "name": "Front door lock", "group": "locks", "words": "Locked", "tone": "ok", "icon": "lock", "available": true, "can": [], "verbs": [], "ctl": {}}),
            json!({"id": "light.gone", "name": "Garage lights", "group": "lights", "words": "Unavailable", "tone": "muted", "icon": "lightbulb", "available": false, "can": [], "verbs": [], "ctl": {}}),
        ];
        Value::Array((0..n).map(|i| kinds[i % kinds.len()].clone()).collect())
    }

    /// Home with a smart-home card beside the clock, fed `topic` (the helper's `onhome` document).
    fn smart_home(topic: Value) -> Home {
        // the weather card beside it keeps its sky still: a tick of the sky's sprites is requests of its own, which these tests count the card's by
        let mut theme = lumatv_tokens::Tokens::shipped().json().clone();
        lumatv_tokens::merge(&mut theme, &json!({"weather": {"animated": false}}));
        let layout = json!({"grid": {"rows": 16}, "widgets": [{"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 6, "rows": 4}, {"id": "smart", "type": "onhome", "col": 0, "row": 4, "span": 6, "rows": 5, "title": "Smart home"},
                                         {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9},
                                         {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        let mut home = Home::new(theme, super::super::grid::shipped(), layout);
        fixtures(&mut home);
        home.update(&Input::Helper { topic: "onhome".into(), data: topic }, 1010);
        home
    }

    fn smart_findings(mode: Mode, scale: f32, topic: Value) -> (Vec<String>, usize) {
        let (h, mut kit) = testkit::kit_in(mode, scale);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut home = smart_home(topic);
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
        let lint = s.lint(&mut kit, &home);
        let words = lint.words().iter().filter(|w| w.name.starts_with("smart")).count();
        let bad = lint.findings().iter().filter(|f| f.subject.starts_with("smart")).map(|f| format!("{f:?}")).collect();
        std::mem::forget(h);
        (bad, words)
    }

    #[test]
    fn the_smart_home_card_reads_clean_with_tiles_and_with_each_reason_for_none_at_both_text_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let with = json!({"state": "connected", "words": "9 things", "tiles": smart_tiles(9)});
                let (bad, words) = smart_findings(mode, scale, with);
                assert!(bad.is_empty(), "{mode:?} at {scale}: {}", bad.join("\n"));
                assert!(words >= 3, "{mode:?} {scale}: the title, the heading and some tiles are drawn, not {words}");
                for state in ["off", "unpaired", "waiting", "empty"] {
                    let (bad, words) = smart_findings(mode, scale, json!({"state": state, "shared": 0, "detail": "why", "tiles": []}));
                    assert!(bad.is_empty(), "{state} {mode:?} at {scale}: {}", bad.join("\n"));
                    assert!(words >= 2, "{state}: the title and the reason");
                }
            }
        }
    }

    /// A tall smart-home card has made a good many items when the layout editor opens over it, and the scene may hold only so many groups: a
    /// scene that is over them draws nothing at all, so that Home stayed as it was with the editor open and unseen.
    #[test]
    fn the_layout_editor_is_drawn_over_a_tall_smart_home_card() {
        for mode in [Mode::Dark, Mode::Light] {
            let (h, mut kit) = testkit::kit_in(mode, 1.0);
            let mut engine = Engine::new(&kit).expect("an engine");
            let theme = lumatv_tokens::Tokens::shipped().json().clone();
            let layout = json!({"grid": {"rows": 16}, "widgets": [{"id": "smart", "type": "onhome", "col": 0, "row": 0, "span": 6, "rows": 9, "title": "Smart home"},
                                         {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9},
                                         {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
            let mut home = Home::new(theme, super::super::grid::shipped(), layout);
            fixtures(&mut home);
            // the dock of a real TV: two dozen tiles, and the cameras ahead of the things
            let lps: Vec<Value> = std::iter::once(json!({"id": "com.webos.app.discovery", "title": "Apps", "unmovable": true, "icon": ""}))
                .chain((0..21).map(|i| json!({"id": format!("app{i}"), "title": format!("App {i}"), "icon": "", "iconColor": "#101010"})))
                .collect();
            home.update(&Input::Luna { tag: model::LP_TAG, payload: json!({"launchPoints": lps}) }, 1011);
            home.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "connected", "cameras": [{"id": "front", "name": "Front door", "state": "live"}, {"id": "yard", "name": "Back yard", "state": "offline"}]}) }, 1012);
            home.update(&Input::Helper { topic: "onhome".into(), data: json!({"state": "connected", "words": "30 things", "tiles": smart_tiles(30)}) }, 1010);
            let mut s = HomeSurface::new(1000);
            s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
            s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
            // however many items it holds, the card leaves the groups a layer over it needs (the editor takes about twenty)
            assert!(kit.room().groups >= 24, "{mode:?}: {} groups left with the card drawn", kit.room().groups);
            let mut out = Vec::new();
            home.edit_home("smart", &mut out);
            s.apply(&mut kit, &mut engine, &mut home, 2400).unwrap_or_else(|e| panic!("{mode:?}: the editor over the card: {e}"));
            assert!(s.editor.is_some(), "{mode:?}: the editor is drawn");
            std::mem::forget(h);
        }
    }

    #[test]
    fn the_sheet_is_drawn_over_a_stepped_back_page_inside_the_screen_and_goes_with_back_at_both_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut home = smart_home(json!({"state": "connected", "words": "9 things", "tiles": smart_tiles(9)}));
                home.walk_to_card("smart", 1100);
                press(&mut home, "ok", 1900);
                // the blinds are the fifth tile: the arrows take them to the front, whatever the layout, and OK opens their sheet (a thermostat has its own)
                let want = home.smart_of("smart").and_then(|s| s.items().iter().position(|i| i.id == "cover.b")).expect("the blinds");
                for i in 0..30u64 {
                    let s = home.smart_of("smart").expect("the card's state");
                    let cur = s.focused().expect("the card is entered");
                    if cur == want {
                        break;
                    }
                    let (a, b) = ((s.plan().placed[cur].row, s.plan().placed[cur].col), (s.plan().placed[want].row, s.plan().placed[want].col));
                    let k = if b.0 > a.0 { "down" } else if b.0 < a.0 { "up" } else if b.1 > a.1 { "right" } else { "left" };
                    press(&mut home, k, 2000 + i * 100);
                }
                press(&mut home, "ok", 5200);
                assert_eq!(home.layer, Layer::OnHome, "{mode:?} {scale}");
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                s.apply(&mut kit, &mut engine, &mut home, 2500).expect("the sheet drawn");
                let sheet = s.onhome_sheet.as_ref().expect("the sheet");
                let pane = sheet.layout().pane;
                assert!(pane.x >= 0.0 && pane.y >= 0.0 && pane.right() <= 1920.0 && pane.bottom() <= 1080.0, "{mode:?} {scale}: {pane:?}");
                let lint = s.lint(&mut kit, &home);
                let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("onhome sheet")).map(|f| format!("{f:?}")).collect();
                assert!(bad.is_empty(), "{mode:?} at {scale}: {}", bad.join("\n"));
                assert!(lint.words().iter().any(|w| w.name.starts_with("onhome sheet")), "the sheet's words were linted");
                // a value changed in the sheet is said at once, from the tile the helper last made
                press(&mut home, "back", 2600);
                s.apply(&mut kit, &mut engine, &mut home, 2700).expect("Home drawn");
                assert!(s.onhome_sheet.is_none(), "Back takes the sheet away");
                std::mem::forget(h);
            }
        }
    }

    /// The card entered and the item `id` in front, whatever the layout is: the time after the last key.
    fn enter_smart_card_at(home: &mut Home, id: &str, at: u64) -> u64 {
        home.walk_to_card("smart", 1100);
        press(home, "ok", at);
        walk_smart_card_to(home, id, at + 100)
    }

    fn colour_lamp_tile() -> Value {
        json!({"id": "light.strip", "name": "TV strip", "area": "Living room", "group": "lights", "words": "On \u{b7} 60%", "tone": "on", "icon": "lightbulb", "available": true, "can": ["toggle"],
               "verbs": ["turn_on", "turn_off", "toggle", "colour", "brightness", "effect"],
               "ctl": {"on": true, "dimmable": true, "brightness": 60, "colour": true, "rgb": [255, 0, 128], "kelvin": null, "effect": "Rainbow", "effects": ["None", "Rainbow", "Candle", "Pulse", "Party"]}})
    }

    /// A sensor's history and a lamp's colours are sheets over a stepped back page: inside the screen, the chart and the colours painted, every
    /// word readable, and Back takes them away, in either look and at either text size.
    #[test]
    fn the_history_and_the_colour_sheets_are_drawn_inside_the_screen_and_read_clean_at_both_sizes_and_looks() {
        let history = json!({"history": {"id": "sensor.t", "kind": "number", "range": "24h", "from": 1_000_000.0, "to": 87_400_000.0, "unit": "\u{b0}C", "class": "temperature", "source": "history",
                                         "points": [[1_000_000.0, 20.0], [20_000_000.0, 22.5], [40_000_000.0, 21.0], [87_400_000.0, 22.0]],
                                         "stats": {"min": 20.0, "max": 22.5, "avg": 21.4, "min_at": 1_000_000.0, "max_at": 20_000_000.0, "now": 22.0, "now_at": 87_400_000.0}}});
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                // the sensor: OK opens the history, which says it is loading until the helper answers
                let mut home = smart_home(json!({"state": "connected", "words": "9 things", "tiles": smart_tiles(9)}));
                let at = enter_smart_card_at(&mut home, "sensor.t", 1900);
                press(&mut home, "ok", at);
                assert!(home.layer == Layer::OnHome && home.history.is_some() && home.sheet.is_none(), "{mode:?} {scale}");
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                s.apply(&mut kit, &mut engine, &mut home, 2500).expect("the sheet drawn");
                assert!(s.history_sheet.as_ref().is_some_and(|s| !s.has_picture()), "asking: no chart yet");
                home.update(&Input::Fetched { tag: super::super::model::HISTORY_TAG, status: 200, data: history.clone() }, 2600);
                s.apply(&mut kit, &mut engine, &mut home, 2700).expect("the sheet drawn");
                let sheet = s.history_sheet.as_ref().expect("the history");
                assert!(sheet.has_picture(), "{mode:?} {scale}: the answer is the chart");
                let pane = sheet.layout.pane;
                assert!(pane.x >= 0.0 && pane.y >= 0.0 && pane.right() <= 1920.0 && pane.bottom() <= 1080.0, "{mode:?} {scale}: {pane:?}");
                let lint = s.lint(&mut kit, &home);
                let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("sensor sheet")).map(|f| format!("{f:?}")).collect();
                assert!(bad.is_empty(), "history {mode:?} at {scale}: {}", bad.join("\n"));
                assert!(lint.words().iter().any(|w| w.name.starts_with("sensor sheet")), "the sheet's words were linted");
                press(&mut home, "back", 2800);
                s.apply(&mut kit, &mut engine, &mut home, 2900).expect("Home drawn");
                assert!(s.history_sheet.is_none(), "Back takes it away");
                // a lamp with a white (temperature only), and one with colours and effects: a held OK opens the colour sheet
                for tiles in [smart_tiles(9), json!([colour_lamp_tile(), smart_tiles(1)[0].clone()])] {
                    let id = tiles[0]["id"].as_str().unwrap().to_owned();
                    let mut home = smart_home(json!({"state": "connected", "words": "9 things", "tiles": tiles}));
                    let at = enter_smart_card_at(&mut home, &id, 1900);
                    home.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, at);
                    home.update(&Input::Tick, at + 800);
                    home.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, at + 850);
                    assert!(home.layer == Layer::OnHome && home.colour.is_some() && home.sheet.is_none(), "{mode:?} {scale} {id}");
                    s.apply(&mut kit, &mut engine, &mut home, at + 900).expect("the sheet drawn");
                    let sheet = s.colour_sheet.as_ref().expect("the colours");
                    let pane = sheet.layout.pane;
                    assert!(pane.x >= 0.0 && pane.y >= 0.0 && pane.right() <= 1920.0 && pane.bottom() <= 1080.0, "{mode:?} {scale} {id}: {pane:?}");
                    assert_eq!(sheet.layout.ring.is_some(), id == "light.strip", "a ring of hues only for a lamp that has colours");
                    let lint = s.lint(&mut kit, &home);
                    let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("colour sheet")).map(|f| format!("{f:?}")).collect();
                    assert!(bad.is_empty(), "colours {mode:?} at {scale} {id}: {}", bad.join("\n"));
                    assert!(lint.words().iter().any(|w| w.name.starts_with("colour sheet")), "the sheet's words were linted");
                    // turned: the sheet is painted again for what changed (a hue, then the row below it) and reads clean still
                    for k in ["right", "right", "down", "right", "down", "down", "right"] {
                        press(&mut home, k, at + 1000);
                        s.apply(&mut kit, &mut engine, &mut home, at + 1100).expect("the sheet drawn");
                    }
                    let lint = s.lint(&mut kit, &home);
                    let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("colour sheet")).map(|f| format!("{f:?}")).collect();
                    assert!(bad.is_empty(), "turned: colours {mode:?} at {scale} {id}: {}", bad.join("\n"));
                    press(&mut home, "back", at + 1200);
                    s.apply(&mut kit, &mut engine, &mut home, at + 1300).expect("Home drawn");
                    assert!(s.colour_sheet.is_none(), "Back takes it away");
                }
                std::mem::forget(h);
            }
        }
    }

    /// The sheets that open for the owner's own bulbs and thermostats (the tiles the helper makes of what Home Assistant reported of them) are
    /// drawn inside the screen and every word on them is readable, in either look and at either text size, and after the keys have turned them.
    #[test]
    fn the_sheets_of_the_owners_own_bulbs_and_thermostats_are_drawn_inside_the_screen_and_read_clean_at_both_sizes_and_looks() {
        use super::super::onhome::owner_tile;
        let ids = ["light.master_bedroom_corner_light", "light.living_room_floor_lamp", "climate.living_room_thermostat", "climate.master_bedroom_bedjet"];
        // (the sheet to open, the text sizes to see it at)
        let cases: [(&str, &[f32]); 4] = [(ids[0], &[1.0, 2.0]), (ids[1], &[1.0]), (ids[2], &[1.0, 2.0]), (ids[3], &[1.0])];
        for mode in [Mode::Dark, Mode::Light] {
            for (id, scales) in cases {
                for &scale in scales {
                    let (h, mut kit) = testkit::kit_in(mode, scale);
                    let mut engine = Engine::new(&kit).expect("an engine");
                    let tiles: Vec<Value> = ids
                        .iter()
                        .map(|i| {
                            let mut t = owner_tile(i);
                            // the BedJet as it is while it heats: a set point, and the modes
                            if *i == "climate.master_bedroom_bedjet" {
                                t["ctl"]["mode"] = json!("heat");
                            }
                            t
                        })
                        .collect();
                    let mut home = smart_home(json!({"state": "connected", "words": "4 things", "tiles": tiles}));
                    let at = enter_smart_card_at(&mut home, id, 1900);
                    let lamp = id.starts_with("light.");
                    if lamp {
                        home.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, at);
                        home.update(&Input::Tick, at + 800);
                        home.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, at + 850);
                    } else {
                        press(&mut home, "ok", at);
                    }
                    let subject = if lamp { "colour sheet" } else { "thermostat sheet" };
                    assert!(home.layer == Layer::OnHome && home.sheet.is_none() && (home.colour.is_some() == lamp) && (home.thermostat.is_some() != lamp), "{mode:?} {scale} {id}");
                    let mut s = HomeSurface::new(1000);
                    s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                    s.apply(&mut kit, &mut engine, &mut home, at + 900).expect("the sheet drawn");
                    let pane = if lamp { s.colour_sheet.as_ref().expect("the colours").layout.pane } else { s.thermostat_sheet.as_ref().expect("the thermostat").layout.pane };
                    assert!(pane.x >= 0.0 && pane.y >= 0.0 && pane.right() <= 1920.0 && pane.bottom() <= 1080.0, "{mode:?} {scale} {id}: {pane:?}");
                    for turned in [false, true] {
                        if turned {
                            // turned: the sheet is painted again for what changed (a value, a row, a chip) and reads clean still
                            for k in ["right", "down", "right", "up", "left", "down", "down", "right"] {
                                press(&mut home, k, at + 1000);
                                s.apply(&mut kit, &mut engine, &mut home, at + 1100).expect("the sheet drawn");
                            }
                        }
                        let lint = s.lint(&mut kit, &home);
                        let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with(subject)).map(|f| format!("{f:?}")).collect();
                        assert!(bad.is_empty(), "{subject} {mode:?} at {scale} {id} turned {turned}: {}", bad.join("\n"));
                        assert!(lint.words().iter().any(|w| w.name.starts_with(subject)), "the sheet's words were linted");
                    }
                    press(&mut home, "back", at + 1200);
                    s.apply(&mut kit, &mut engine, &mut home, at + 1300).expect("Home drawn");
                    assert!(s.colour_sheet.is_none() && s.thermostat_sheet.is_none(), "Back takes it away");
                    std::mem::forget(h);
                }
            }
        }
    }

    /// What an `apply` sent to the compositor, without the commit that ends it.
    fn sent(h: &mut ui::testing::Harness) -> Vec<compositor_protocol::Request> {
        h.commit().into_iter().filter(|r| !matches!(r, compositor_protocol::Request::Commit { .. })).collect()
    }

    /// A switch and a lamp are toggled in the step of the key: what changes is sent once, no glass or group of the card is made again (nothing
    /// is laid out again), and the frames the compositor then runs (the thumb's travel, the lamp's fade, the lift) ask this process for nothing.
    #[test]
    fn a_toggle_sends_its_change_once_without_laying_the_card_out_again_and_its_animation_costs_the_client_nothing() {
        use compositor_protocol::{Content as Made, Request};
        let (mut h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let fan = json!({"id": "switch.fan", "name": "Fan", "area": "Bedroom", "group": "switches", "words": "On", "tone": "on", "icon": "fan", "available": true, "can": ["toggle"], "verbs": ["toggle"], "ctl": {"on": true}});
        let mut home = smart_home(json!({"state": "connected", "words": "3 things", "tiles": [fan, smart_tiles(1)[0].clone(), smart_tiles(2)[1].clone()]}));
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
        h.settle_atlas();
        s.apply(&mut kit, &mut engine, &mut home, 2350).expect("Home drawn");
        let at = enter_smart_card_at(&mut home, "switch.fan", 2400);
        s.apply(&mut kit, &mut engine, &mut home, at).expect("the card entered");
        h.settle_atlas();
        s.apply(&mut kit, &mut engine, &mut home, at + 10).expect("the card entered");
        sent(&mut h);
        let views = |s: &HomeSurface| s.cards.iter().map(|c| if let Content::OnHome(oh) = &c.content { oh.view_count() } else { 0 }).sum::<usize>();
        let mut when = at + 1000;
        for id in ["switch.fan", "light.lamp"] {
            when = walk_smart_card_to(&mut home, id, when) + 100;
            // the arrows are drawn as they come: the lift of the item that came to the front is made then, not by the press
            s.apply(&mut kit, &mut engine, &mut home, when - 50).expect("the focus drawn");
            h.settle_atlas();
            sent(&mut h);
            let before = views(&s);
            press(&mut home, "ok", when);
            s.apply(&mut kit, &mut engine, &mut home, when + 40).expect("the toggle drawn");
            let first = sent(&mut h);
            let glass = ui::testing::count_requests(&first, |r| matches!(r, Request::Create { content: Made::Glass(_), .. }));
            let deleted = ui::testing::count_requests(&first, |r| matches!(r, Request::Delete { .. }));
            assert_eq!((glass, deleted, views(&s)), (0, 0, before), "{id}: the card was not laid out again ({} requests)", first.len());
            assert!(!first.is_empty(), "{id}: the change is sent");
            assert!(first.len() < 40, "{id}: only what changed: {} requests", first.len());
            // the thumb's travel, the lamp's fade and the lift are the compositor's: this process is not woken for a frame of them
            let due = home.smart_of("smart").and_then(|s| s.due_at());
            assert!(due.is_none_or(|d| d >= when + 250), "{id}: the card wakes the client at {due:?} for a toggle at {when} ms");
            assert_eq!(ui::testing::pictures_made(&first), 0, "{id}: no picture is painted again for a toggle");
            when += 6000;
        }
    }

    /// The arrows from where the entered card has the focus to the item `id`: the time after the last key.
    fn walk_smart_card_to(home: &mut Home, id: &str, at: u64) -> u64 {
        let want = home.smart_of("smart").and_then(|s| s.items().iter().position(|i| i.id == id)).expect("the item");
        for i in 0..30u64 {
            let s = home.smart_of("smart").expect("the card's state");
            let cur = s.focused().expect("the card is entered");
            if cur == want {
                return at + i * 100;
            }
            let (a, b) = ((s.plan().placed[cur].row, s.plan().placed[cur].col), (s.plan().placed[want].row, s.plan().placed[want].col));
            let k = if b.0 > a.0 { "down" } else if b.0 < a.0 { "up" } else if b.1 > a.1 { "right" } else { "left" };
            press(home, k, at + i * 100);
        }
        panic!("the arrows did not reach {id}");
    }

    /// A switch of the theme (dark to light and back) makes the card again in the new ink, the glass and the one quiet glyph of the heading
    /// included, leaks no layer, group or track, and reads clean in both.
    #[test]
    fn switching_the_theme_makes_the_smart_home_card_again_in_the_new_look_and_leaks_nothing() {
        let (h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut home = smart_home(json!({"state": "connected", "words": "9 things", "tiles": smart_tiles(9)}));
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
        let at = enter_smart_card_at(&mut home, "light.lamp", 2400);
        s.apply(&mut kit, &mut engine, &mut home, at).expect("the card entered");
        let room = |kit: &Kit| (kit.room().layers, kit.room().groups, kit.room().tracks);
        let dark = room(&kit);
        for (i, mode) in [Mode::Light, Mode::Dark, Mode::Light].into_iter().enumerate() {
            kit.retheme(lumatv_tokens::Tokens::shipped(), mode, 1.0);
            s.apply(&mut kit, &mut engine, &mut home, at + 100 + i as u64 * 100).expect("Home drawn in the new look");
            let lint = s.lint(&mut kit, &home);
            let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("smart")).map(|f| format!("{f:?}")).collect();
            assert!(bad.is_empty(), "{mode:?}: {}", bad.join("\n"));
            assert!(lint.words().iter().any(|w| w.name.starts_with("smart")), "{mode:?}: the card is drawn");
            let now = room(&kit);
            assert!(now.0 + 2 >= dark.0 && now.1 + 2 >= dark.1 && now.2 + 2 >= dark.2, "{mode:?}: layers, groups and tracks left {now:?}, were {dark:?}: something leaked");
        }
        std::mem::forget(h);
    }

    #[test]
    fn the_unlock_question_is_drawn_with_its_answers_in_the_sheet_and_reads_clean_at_both_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let lock = json!({"id": "lock.front", "name": "Front door", "area": "Hall", "group": "locks", "words": "Locked", "tone": "ok", "icon": "lock", "available": true, "can": ["unlock"], "verbs": ["lock"], "sensitive": ["unlock"], "ctl": {"state": "locked"}});
                let mut home = smart_home(json!({"state": "connected", "words": "1 thing", "tiles": [lock]}));
                home.walk_to_card("smart", 1100);
                press(&mut home, "ok", 1900);
                press(&mut home, "ok", 2000);
                assert_eq!((home.layer, home.sheet.as_ref().map(|s| s.asking)), (Layer::OnHome, Some(true)), "{mode:?} {scale}");
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                s.apply(&mut kit, &mut engine, &mut home, 2300).expect("the question drawn");
                let sheet = s.onhome_sheet.as_ref().expect("the sheet");
                let pane = sheet.layout().pane;
                assert_eq!(sheet.layout().rows.len(), 2, "Cancel and Unlock");
                assert!(pane.x >= 0.0 && pane.right() <= 1920.0 && pane.bottom() <= 1080.0, "{mode:?} {scale}: {pane:?}");
                let lint = s.lint(&mut kit, &home);
                let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("onhome sheet")).map(|f| format!("{f:?}")).collect();
                assert!(bad.is_empty(), "{mode:?} at {scale}: {}", bad.join("\n"));
                assert!(lint.words().iter().any(|w| w.name.starts_with("onhome sheet")));
                std::mem::forget(h);
            }
        }
    }

    #[test]
    fn whos_watching_passes_the_lint_at_both_text_sizes() {
        for scale in [1.0, 2.0] {
            let (h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let mut engine = Engine::new(&kit).expect("an engine");
            let theme = lumatv_tokens::Tokens::shipped().json().clone();
            let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
            fixtures(&mut home);
            for i in 0..home.status_index("profile").expect("the profile chip") as u64 {
                press(&mut home, "right", 1500 + i * 100);
            }
            press(&mut home, "ok", 2000);
            assert_eq!(home.layer, Layer::Picker);
            let mut s = HomeSurface::new(1000);
            s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
            s.apply(&mut kit, &mut engine, &mut home, 2300).expect("the picker drawn");
            let lint = s.lint(&mut kit, &home);
            let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.starts_with("picker")).map(|f| format!("{f:?}")).collect();
            assert!(bad.is_empty(), "the picker at {scale}: {}", bad.join("\n"));
            assert!(lint.words().iter().any(|w| w.name == "picker.name.1"), "the names were linted");
            std::mem::forget(h);
        }
    }

    #[test]
    fn the_library_rail_passes_lint_when_scrolled_at_both_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).unwrap();
                let theme = lumatv_tokens::Tokens::shipped().json().clone();
                let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                home.set_text_scale(scale);
                home.apps.open();
                home.layer = Layer::Apps;
                let mut surface = HomeSurface::new(1000);
                surface.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                for i in (0..home.apps.rail().len()).chain((0..home.apps.rail().len()).rev()) {
                    home.apps.set_section(i);
                    surface.apply(&mut kit, &mut engine, &mut home, 2300).unwrap();
                    let lint = surface.lint(&mut kit, &home);
                    let bad: Vec<_> = lint.findings().into_iter().filter(|f| f.subject.starts_with("apps.")).collect();
                    assert!(bad.is_empty(), "{mode:?} {scale} rail {i}: {bad:?}");
                    assert!(lint.words().iter().any(|w| w.name.starts_with("apps.rail")));
                }
                std::mem::forget(h);
            }
        }
    }

    /// The focused tile's name against its box (in the dock's coordinates): `Some(true)` above it, `Some(false)` below it.
    fn pill_side(s: &HomeSurface, home: &Home) -> Option<bool> {
        let d = s.dock.as_ref()?;
        d.pill_for?;
        let (_, top) = home.dock.slot(home.dock.index);
        let (top, tile) = (top as f32 - d.scroll, home.dock.tile as f32);
        let f = d.pill_text.frame();
        if f.y + f.h <= top {
            Some(true)
        } else if f.y >= top + tile {
            Some(false)
        } else {
            None
        }
    }

    #[test]
    fn a_long_name_under_the_first_or_last_tile_of_a_row_stays_whole_on_the_docks_card_above_its_tile_at_both_text_sizes_and_looks() {
        let long = "Homebrew Channel Updater and Friends";
        let list: Vec<Value> = std::iter::once(json!({"id": "com.webos.app.discovery", "title": long, "unmovable": true, "icon": ""}))
            .chain((1..13).map(|n| json!({"id": format!("app{n}"), "title": if n == 11 { long.to_owned() } else { format!("App {n}") }, "icon": "", "iconColor": "#101010"})))
            .collect();
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for (last, key) in [(false, "left"), (true, "right")] {
                    let (h, mut kit) = testkit::kit_in(mode, scale);
                    let mut engine = Engine::new(&kit).expect("an engine");
                    let mut home = Home::new(lumatv_tokens::Tokens::shipped().json().clone(), super::super::grid::shipped(), json!({}));
                    fixtures(&mut home);
                    home.update(&Input::Luna { tag: model::LP_TAG, payload: json!({"launchPoints": list}) }, 1500);
                    for (i, k) in ["down", "down", "down"].into_iter().enumerate() {
                        press(&mut home, k, 2000 + i as u64 * 100);
                    }
                    // a tile short of the row's end: one press further would roll over to the next row
                    let cols = home.dock.cols();
                    let want = if last { cols - 1 } else { 0 };
                    // Down lands on the last tile of a shorter row, which can lie past `want`
                    let key = if home.dock.index > want { "left" } else { key };
                    for n in 0..home.dock.index.abs_diff(want) {
                        press(&mut home, key, 2400 + n as u64 * 100);
                    }
                    assert_eq!(home.dock.index, want, "{mode:?} {scale}: the end of the row");
                    home.update(&Input::Tick, 4000);
                    let mut s = HomeSurface::new(1000);
                    s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                    s.apply(&mut kit, &mut engine, &mut home, 4000).expect("Home drawn");
                    let d = s.dock.as_ref().unwrap();
                    assert!(d.pill_for.is_some(), "{mode:?} {scale}: the name is up");
                    let f = d.pill_text.frame();
                    assert!(f.x >= 0.0 && f.x + f.w <= d.frame.w, "{mode:?} {scale} last={last}: {f:?} on a card {} wide", d.frame.w);
                    assert!(pill_side(&s, &home).is_some(), "{mode:?} {scale} last={last}: above or below the top row's tile, not over it");
                    assert!(f.y >= 0.0 && f.y + f.h <= d.frame.h, "{mode:?} {scale} last={last}: {f:?} on a card {} high", d.frame.h);
                    // the words are linted too: the layout lint finds a clipped or crowded name
                    let lint = s.lint(&mut kit, &home);
                    let bad: Vec<String> = lint.findings().iter().filter(|f| f.subject.contains("dock.pill")).map(|f| format!("{f:?}")).collect();
                    assert!(bad.is_empty(), "{mode:?} {scale} last={last}: {}", bad.join("\n"));
                    std::mem::forget(h);
                }
            }
        }
    }

    #[test]
    fn a_focused_tiles_name_stands_above_it_on_the_docks_top_row_and_below_it_on_the_others_at_both_text_sizes() {
        let many: Vec<Value> = std::iter::once(json!({"id": "com.webos.app.discovery", "title": "Apps", "unmovable": true, "icon": ""}))
            .chain((1..20).map(|n| json!({"id": format!("app{n}"), "title": format!("App {n}"), "icon": "", "iconColor": "#101010"})))
            .collect();
        for scale in [1.0, 2.0] {
            // one row of six apps, then two rows of them
            for (count, second_row) in [(6, false), (20, true)] {
                let (h, mut kit) = testkit::kit_in(Mode::Dark, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut home = Home::new(lumatv_tokens::Tokens::shipped().json().clone(), super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                home.update(&Input::Luna { tag: model::LP_TAG, payload: json!({"launchPoints": many[..count]}) }, 1500);
                for (i, k) in ["down", "left", "down", "right"].into_iter().enumerate() {
                    press(&mut home, k, 2000 + i as u64 * 100);
                }
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                // a glide: no name yet; once the focus has rested for restMs (150), the name
                s.apply(&mut kit, &mut engine, &mut home, 2400).expect("Home drawn");
                assert_eq!(pill_side(&s, &home), None, "{scale}: no name while the focus is still moving");
                assert!(s.dock.as_ref().unwrap().pill_for.is_none());
                let due = home.due_at().expect("a tick is due when the focus rests");
                assert!(due <= 2300 + 150 + 5, "{due}");
                let out = home.update(&Input::Tick, due);
                assert!(out.contains(&crate::kit::screen::Effect::Redraw), "the rest is drawn");
                s.apply(&mut kit, &mut engine, &mut home, due).expect("Home drawn");
                assert_eq!(home.dock.row_of(home.dock.index), 0, "{scale}: the dock's top row has the focus");
                if scale == 1.0 {
                    assert_eq!(pill_side(&s, &home), Some(true), "{scale}, {count} apps: the top row's name is above its tile");
                }
                // the name stays on the dock's card
                let d = s.dock.as_ref().unwrap();
                let f = d.pill_text.frame();
                assert!(f.y >= 0.0 && f.y + f.h <= d.frame.h, "{scale}, {count} apps: {f:?} on a card {} high", d.frame.h);
                if second_row {
                    assert_eq!(home.dock.total_rows(), 2);
                    press(&mut home, "down", 3000);
                    s.apply(&mut kit, &mut engine, &mut home, 3040).expect("Home drawn");
                    assert!(s.dock.as_ref().unwrap().pill_for.is_none(), "{scale}: the name goes the moment the focus moves on");
                    home.update(&Input::Tick, 3300);
                    s.apply(&mut kit, &mut engine, &mut home, 3300).expect("Home drawn");
                    assert_eq!(home.dock.row_of(home.dock.index), 1, "{scale}");
                    let d = s.dock.as_ref().unwrap();
                    let f = d.pill_text.frame();
                    assert!(f.y >= 0.0 && f.y + f.h <= d.frame.h, "{scale}: the second row's name {f:?} on a card {} high", d.frame.h);
                    if scale == 1.0 {
                        assert_eq!(pill_side(&s, &home), Some(false), "{scale}: the second row's name is below its tile");
                    }
                }
                std::mem::forget(h);
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

    #[test]
    fn the_settings_popover_hangs_under_the_gear_with_the_page_stepped_back_and_reads_clean_at_both_sizes_in_both_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).unwrap();
                let theme = lumatv_tokens::Tokens::shipped().json().clone();
                let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                home.set_text_scale(scale);
                home.edit_wallpaper(&mut Vec::new());
                assert_eq!(home.layer, Layer::Popover);
                let mut surface = HomeSurface::new(1000);
                surface.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                // every row has been in front once, so the scrolled panel is read at each of its places
                for i in 0..16 {
                    surface.apply(&mut kit, &mut engine, &mut home, 2300 + i * 10).expect("the popover is drawn");
                    let lint = surface.lint(&mut kit, &home);
                    let bad: Vec<String> = lint.findings().iter().map(|f| format!("{f:?}")).collect();
                    assert!(bad.is_empty(), "{mode:?} at {scale}, row {}: {}", home.popover.as_ref().unwrap().index, bad.join("\n"));
                    assert!(lint.words().iter().any(|w| w.name == "popover head"));
                    press(&mut home, "down", 2400 + i * 100);
                }
                let g = surface.geometry(&home)["popover"].clone();
                let pane = &g["pane"];
                assert!(pane["y"].as_f64().unwrap() > 54.0 + 60.0 && pane["y"].as_f64().unwrap() + pane["h"].as_f64().unwrap() <= 1080.0 - 54.0 + 0.5, "under the bar, inside the safe area: {pane}");
                assert!(pane["x"].as_f64().unwrap() + pane["w"].as_f64().unwrap() <= 1856.0, "{pane}");
                if scale == 1.0 {
                    assert_eq!(g["rows"].as_array().map(Vec::len), Some(super::super::popover::ROWS.len()), "all the rows at 100 %");
                    assert_eq!(g["hints"].as_array().map(Vec::len), Some(2), "the keys of the panel as glyphs");
                }
                assert_eq!(g["head"], "Wallpaper");
                // the page is stepped back for the preview, the bar stays
                assert!(surface.geometry(&home)["status"].is_array());
                std::mem::forget(h);
            }
        }
    }

    #[test]
    fn the_profile_chip_is_one_pill_with_a_round_avatar_in_its_left_cap_and_its_name_in_the_middle_of_the_rest_at_both_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let theme = lumatv_tokens::Tokens::shipped().json().clone();
                let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
                let chip = home.chip();
                let item = s.status.as_ref().unwrap().items.iter().find(|i| i.kind == "profile").expect("the chip");
                let (cw, ch) = (item.lift.frame.w, item.lift.frame.h);
                assert_eq!(cw, chip.width, "the chip is as wide as its parts: {mode:?} at {scale}");
                // the avatar is a disc, concentric with the pill's left cap: the same gap above, below and at the left
                let av = item.avatar.as_ref().expect("the avatar");
                let side = av.width() as f32;
                assert_eq!(side, ch - 2.0 * chip.inset, "{mode:?} at {scale}");
                let at = |x: u32, y: u32| av.pixels()[((y * av.width() + x) * 4 + 3) as usize];
                let mid = av.width() / 2;
                assert_eq!(at(0, 0), 0, "a disc has no corner");
                assert_eq!(at(av.width() - 1, av.height() - 1), 0);
                assert_eq!(at(mid, 0).max(at(mid, 1)), 255, "and its top is whole");
                assert_eq!(at(0, mid).max(at(1, mid)), 255, "and its left");
                let cap = (ch / 2.0).floor();
                assert_eq!(chip.inset + side / 2.0, ch / 2.0, "the disc's centre is the cap's");
                assert!((chip.inset + side / 2.0 - cap).abs() <= 0.5);
                // the name: centred on the pill's height, the avatar's gap before it and the cap's padding after it
                let name = item.name.as_ref().expect("the name").frame();
                assert_eq!(name.x, chip.name_x);
                let gap = chip.name_x - (chip.inset + side);
                let trailing = cw - (name.x + name.w);
                assert!(trailing >= gap && trailing <= 2.0 * gap, "padding after the name {trailing} against the gap before it {gap}: {mode:?} at {scale}");
                assert!((name.y + name.h / 2.0 - ch / 2.0).abs() <= 0.5, "the name is on the pill's middle: {name:?}");
                std::mem::forget(h);
            }
        }
    }

    #[test]
    fn a_profile_chip_the_compositor_refused_is_made_again_on_the_next_frame_and_one_it_took_is_kept() {
        use compositor_protocol::{Event, Request};
        let tag = |sent: &[Request]| sent.iter().find_map(|r| match r { Request::Commit { tag } => Some(*tag), _ => None }).expect("a commit");
        let (mut h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut home = Home::new(lumatv_tokens::Tokens::shipped().json().clone(), super::super::grid::shipped(), json!({}));
        fixtures(&mut home);
        let mut s = HomeSurface::new(1000);
        s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
        let chip = |s: &HomeSurface| {
            let item = s.status.as_ref().unwrap().items.iter().find(|i| i.kind == "profile").expect("the chip");
            [item.avatar.as_ref().expect("the avatar").id(), item.name.as_ref().expect("the name").layer().id()]
        };
        s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
        let first = chip(&s);
        let sent = h.commit();
        // the compositor refuses the chip (the profile item is made last), which it says before it shows the commit
        kit.ui.scene.observe(&Event::Error { message: "more than 256 layers".into() });
        kit.ui.scene.shown(tag(&sent));
        s.apply(&mut kit, &mut engine, &mut home, 2316).expect("Home drawn");
        let second = chip(&s);
        let sent = h.commit();
        for (old, new) in first.into_iter().zip(second) {
            assert!(new != old && sent.iter().any(|r| matches!(r, Request::Create { id, .. } if *id == new)), "made again and sent");
        }
        kit.ui.scene.shown(tag(&sent));
        s.apply(&mut kit, &mut engine, &mut home, 2332).expect("Home drawn");
        assert_eq!(chip(&s), second, "it came through: kept");
        std::mem::forget(h);
    }

    /// The share of a glyph's ink that is an edge: the pixels that are neither empty nor full, over all the pixels with ink in them.
    fn edge_share(rgba: &[u8]) -> f32 {
        let ink = rgba.chunks_exact(4).filter(|p| p[3] > 0).count().max(1);
        rgba.chunks_exact(4).filter(|p| p[3] > 0 && p[3] < 255).count() as f32 / ink as f32
    }

    #[test]
    fn the_status_glyphs_come_from_vectors_on_whole_strokes_and_the_bar_stands_on_whole_pixels_at_both_sizes_and_looks() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let theme = lumatv_tokens::Tokens::shipped().json().clone();
                let mut home = Home::new(theme, super::super::grid::shipped(), json!({}));
                fixtures(&mut home);
                let mut s = HomeSurface::new(1000);
                s.dirs(&testkit::module_dir(), std::path::Path::new("/nonexistent"));
                s.apply(&mut kit, &mut engine, &mut home, 2300).expect("Home drawn");
                let k = crate::kit::vector_icon::multiple(scale);
                let bar = s.status.as_ref().unwrap();
                // the pill and every button are on whole pixels
                let whole = |v: f32| (v - v.round()).abs() < 1e-4;
                assert!([bar.frame.x, bar.frame.y, bar.frame.w, bar.frame.h].into_iter().all(whole), "{:?}", bar.frame);
                let ink = kit.ui.look.ink();
                for item in &bar.items {
                    let f = item.lift.frame;
                    assert!([f.x, f.y, f.w, f.h].into_iter().all(whole), "{}: {f:?}", item.kind);
                    if item.kind == "profile" {
                        continue;
                    }
                    let name = match item.kind.as_str() {
                        "notifications" => "bell",
                        "cameras" => "camera",
                        other => other,
                    };
                    let icon = item.icon.as_ref().unwrap_or_else(|| panic!("{} has a glyph", item.kind));
                    assert_eq!((icon.width(), icon.height()), (24 * k, 24 * k), "{}: a whole multiple of Lucide's grid at {scale}", item.kind);
                    // against the PNG's glyph taken down to the same box by the area mean: the strokes are steps, not ramps
                    let soft = kit.icons.draw(name, 24 * k, ink).expect("the png");
                    let (vector, old) = (edge_share(icon.pixels()), edge_share(&soft));
                    assert!(vector + 0.15 <= old, "{}: {vector:.2} of the vector glyph's ink is an edge, {old:.2} of the scaled bitmap's ({mode:?} at {scale})", item.kind);
                }
                std::mem::forget(h);
            }
        }
    }
}
