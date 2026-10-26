//! The GeForce NOW app's screens on the toolkit, in LumaTV's look (`look`: the glass of LumaHome on the shipped wallpaper, in the dark or
//! the light material) and Lucent's TV rules: little copy (a row is a name and a value, what is not obvious sits behind a help mark), one
//! button size per screen, chrome over the stream that leaves after inactivity and never blocks the way out, an opaque backdrop behind
//! an explanation or the question of leaving a game. `layout` decides where everything goes, for the rules' state and the text scale;
//! this module makes the layers once and puts a plan on them, sending only what differs from the last plan.
//!
//! Over the stream the wallpaper and every card are hidden, so the window is transparent and the video plane shows through
//! (`docs/research/gfn-app-result.md`, "The app"); the status line and the overlay are the only pixels.
//!
//! The library's tiles show box art (`art`): each tile is a glass layer (its shadow and focus), a picture layer (the art, or a
//! placeholder of the same shape until it is there, with the title strip and the corners baked in) and, for the focused tile, a ring.
//! The art is decoded on a worker thread from the file the core cached; nothing here waits for it.
use crate::gfn::app::{App, Art, GRID_ROWS, LIBRARY_COLUMNS, Screen};
use crate::gfn::art::{self, Loader, Spec, Thumb};
use crate::gfn::layout::{self, Fill, Ink, Lay, Metrics, Plan, Surf, Txt, H, TILE_H, TILE_W, W};
use crate::gfn::prefs::Row;
use crate::look::{self, Mode, Palette};
use compositor_protocol::{Backdrop, Glass, Props, Rgba, Wallpaper, WallpaperKind};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::Path;
use ui::components::{Form, Icon, Item, Mark, Run, TextLayer};
use ui::geometry::Frame;
use ui::{GlassLayer, ImageLayer, Ui, WallpaperLayer};

const Z_WALL: i32 = 0;
const Z_PANEL: i32 = 10;
const Z_TILE: i32 = 11;
const Z_ART: i32 = 12;
const Z_ROW: i32 = 12;
const Z_RING: i32 = 13;
const Z_TEXT: i32 = 14;
const Z_QR: i32 = 15;
const Z_BUTTON: i32 = 16;
const Z_TILE_TEXT: i32 = 17;
const Z_BUTTON_TEXT: i32 = 18;
const Z_STATUS: i32 = 30;
const Z_STATUS_TEXT: i32 = 32;
const Z_OVERLAY: i32 = 40;
const Z_OVERLAY_ROW: i32 = 42;
const Z_OVERLAY_TEXT: i32 = 44;
const Z_HELP: i32 = 50;
const Z_HELP_TEXT: i32 = 52;

/// Decoded pictures kept in memory, by source address; the oldest go first. The visible tiles and a row either side are 36.
const THUMBS_KEPT: usize = 48;
/// The strip under a tile's title: dark in both modes, so the title's ink is light in both.
const STRIP: [u8; 3] = [0x12, 0x0E, 0x1A];
const STRIP_ALPHA: u8 = 224;
const STRIP_FADE: u32 = 40;

fn ink_colour(p: &Palette, ink: Ink) -> Rgba {
    match ink {
        Ink::Ink => p.ink,
        Ink::Ink2 => p.ink2,
        Ink::OnArt => p.on_art,
        Ink::OnAccent => p.ink_on_accent,
        Ink::Danger => p.second,
        Ink::Rule => p.rule,
        Ink::Wash => p.wash,
        Ink::Accent => p.accent,
    }
}

/// The status line's chip: dark in both modes, since it sits over a game's picture.
fn status_glass() -> Glass {
    look::pill()
}

/// The overlay, the explanation card: an opaque backdrop (`scrimHide`, at least .97) with no blur under it.
fn opaque_glass(p: &Palette, radius: f32) -> Glass {
    let mut g = look::card(p, radius, false, true);
    g.tint = p.scrim_hide;
    g.backdrop = Backdrop::None;
    g
}

/// The focused tile's outline: glass with no body and an accent edge.
fn ring_glass(p: &Palette) -> Glass {
    Glass {
        corner_radius: look::R_TILE,
        tint: Rgba(0, 0, 0, 0),
        edge: p.accent,
        edge_width: 4.0,
        highlight: Rgba(0, 0, 0, 0),
        sheen: None,
        shadows: [None, None],
        depth: None,
        backdrop: Backdrop::None,
        saturation: 1.0,
        refraction: 0.0,
    }
}

fn hidden() -> Props {
    Props { visible: false, ..Props::at(0.0, 0.0) }
}

fn at(f: Frame) -> Props {
    Props::at(f.x, f.y)
}

/// What was drawn last, to send only the difference.
struct Shown {
    plan: Plan,
    streaming: bool,
    /// What each tile slot's picture layer holds: the address of the picture, or `None` for the placeholder.
    art: Vec<Option<String>>,
    qr: Vec<String>,
}

pub struct GfnUi {
    pub ui: Ui,
    p: Palette,
    mode: Mode,
    m: Metrics,
    wall: WallpaperLayer,
    panel: GlassLayer,
    head: TextLayer,
    body: TextLayer,
    qr: ImageLayer,
    button: GlassLayer,
    button_text: TextLayer,
    header_button: GlassLayer,
    settings_rows: Vec<GlassLayer>,
    tiles: Vec<GlassLayer>,
    tile_art: Vec<ImageLayer>,
    ring: GlassLayer,
    tile_text: TextLayer,
    status: GlassLayer,
    status_text: TextLayer,
    overlay: GlassLayer,
    overlay_rows: Vec<GlassLayer>,
    overlay_text: TextLayer,
    help: Vec<GlassLayer>,
    help_text: TextLayer,
    // box art
    spec: Spec,
    placeholder: Thumb,
    loader: Loader,
    thumbs: HashMap<String, Thumb>,
    thumb_order: VecDeque<String>,
    requested: HashSet<String>,
    failed: HashSet<String>,
    art_version: u64,
    shown_art_version: u64,
    shown: Option<Shown>,
}

impl GfnUi {
    /// `scale` is the text scale (1.0, or 2.0 for Lucent's 200 % check); `mode` forces a material, else the wallpaper's brightness
    /// chooses as LumaHome's does; `wall` replaces the packaged wallpaper (the renders' light one).
    pub fn build(mut ui: Ui, assets: &Path, scale: f32, mode: Option<Mode>, wall: Option<Vec<u8>>, land: &mut dyn FnMut(&mut Ui) -> io::Result<()>) -> io::Result<Self> {
        let m = Metrics::new(scale);
        let pixels = match wall {
            Some(p) => p,
            None => crate::home::wallpaper_pixels(&assets.join("wallpaper/wall_1080.png"), W as u32, H as u32)?,
        };
        let mode = mode.unwrap_or_else(|| Mode::of_wallpaper(&pixels));
        let p = look::palette_for(mode);
        ui.warm(&m.types());
        land(&mut ui)?;
        let forms = forms_to_warm(&m, &mut ui.fonts);
        ui.warm_marks(&forms);
        land(&mut ui)?;

        let wall = {
            let mut layer = ui.scene.wallpaper(W as u32, H as u32, Z_WALL, Wallpaper { kind: WallpaperKind::Silk, speed: 0.0, intensity: 0.0, fps: 0 }, Props::at(0.0, 0.0))?;
            layer.pixels_mut().ok_or_else(|| io::Error::other("the wallpaper has no picture"))?.copy_from_slice(&pixels);
            layer.damage(None);
            layer
        };
        let panel_f = layout::panel_frame(&m);
        let panel = ui.scene.glass(panel_f.w as u32, panel_f.h as u32, Z_PANEL, look::card(&p, look::R_CARD, false, false), Props { visible: false, ..at(panel_f) });
        let head = TextLayer::new(&mut ui, Frame::new(0.0, 0.0, W, layout::grid_top(&m)), Z_TEXT, &[])?;
        let body = TextLayer::new(&mut ui, panel_f, Z_TEXT + 1, &[])?;
        let qr_side = m.qr_side();
        let qr = ui.scene.image(qr_side, qr_side, Z_QR, hidden())?;
        let (bw, bh) = layout::button_size(&mut ui.fonts, &m);
        let button = ui.scene.glass(bw as u32, bh as u32, Z_BUTTON, look::card(&p, bh / 2.0, true, false), hidden());
        let button_text = TextLayer::new(&mut ui, Frame::new(0.0, 0.0, bw, bh), Z_BUTTON_TEXT, &[])?;
        let hs = m.head_side();
        let header_button = ui.scene.glass(hs as u32, hs as u32, Z_ROW, look::card(&p, hs / 2.0, false, false), hidden());
        let row_f = layout::settings_row_frame(&m, layout::settings_body_y(&m), 0);
        let mut settings_rows = Vec::new();
        for _ in Row::ALL {
            settings_rows.push(ui.scene.glass(row_f.w as u32, row_f.h as u32, Z_ROW, look::card(&p, 20.0, false, false), hidden()));
        }
        let (mut tiles, mut tile_art) = (Vec::new(), Vec::new());
        for _ in 0..LIBRARY_COLUMNS * GRID_ROWS {
            tiles.push(ui.scene.glass(TILE_W as u32, TILE_H as u32, Z_TILE, look::card(&p, look::R_TILE, false, false), hidden()));
            tile_art.push(ui.scene.image(TILE_W as u32, TILE_H as u32, Z_ART, hidden())?);
        }
        let ring = ui.scene.glass(TILE_W as u32, TILE_H as u32, Z_RING, ring_glass(&p), hidden());
        let tile_text = TextLayer::new(&mut ui, Frame::new(0.0, layout::grid_top(&m), W, H - layout::grid_top(&m)), Z_TILE_TEXT, &[])?;
        let status_f = layout::status_frame(&mut ui.fonts, &m);
        let status = ui.scene.glass(status_f.w as u32, status_f.h as u32, Z_STATUS, status_glass(), hidden());
        let status_text = TextLayer::new(&mut ui, status_f, Z_STATUS_TEXT, &[])?;
        let overlay_f = layout::overlay_frame(&mut ui.fonts, &m);
        let overlay = ui.scene.glass(overlay_f.w as u32, overlay_f.h as u32, Z_OVERLAY, opaque_glass(&p, look::R_CARD), hidden());
        let mut overlay_rows = Vec::new();
        for i in 0..crate::gfn::app::OVERLAY_ROWS.len() {
            let f = layout::overlay_row_frame(&mut ui.fonts, &m, i);
            overlay_rows.push(ui.scene.glass(f.w as u32, f.h as u32, Z_OVERLAY_ROW, look::card(&p, 20.0, false, false), hidden()));
        }
        let overlay_text = TextLayer::new(&mut ui, overlay_f, Z_OVERLAY_TEXT, &[])?;
        let mut help = Vec::new();
        for i in 0..Row::ALL.len() {
            let (f, _) = layout::help_card(&mut ui.fonts, &m, i);
            help.push(ui.scene.glass(f.w as u32, f.h as u32, Z_HELP, opaque_glass(&p, look::R_CARD), hidden()));
        }
        let help_text = TextLayer::new(&mut ui, panel_f, Z_HELP_TEXT, &[])?;
        land(&mut ui)?;

        let spec = Spec {
            w: TILE_W as u32,
            h: TILE_H as u32,
            radius: look::R_TILE as u32,
            strip: layout::strip_height(&m) as u32,
            fade: STRIP_FADE,
            scrim: STRIP,
            scrim_alpha: STRIP_ALPHA,
        };
        let (top, bottom) = match mode {
            Mode::Dark => ([0x2C, 0x26, 0x3A], [0x1F, 0x1D, 0x2E]),
            Mode::Light => ([0xE6, 0xDE, 0xF0], [0xCB, 0xC0, 0xDE]),
        };
        let placeholder = art::placeholder(&spec, top, bottom);
        Ok(Self {
            ui,
            p,
            mode,
            m,
            wall,
            panel,
            head,
            body,
            qr,
            button,
            button_text,
            header_button,
            settings_rows,
            tiles,
            tile_art,
            ring,
            tile_text,
            status,
            status_text,
            overlay,
            overlay_rows,
            overlay_text,
            help,
            help_text,
            spec,
            placeholder,
            loader: Loader::new(std::thread::available_parallelism().map_or(1, |n| n.get().min(2))),
            thumbs: HashMap::new(),
            thumb_order: VecDeque::new(),
            requested: HashSet::new(),
            failed: HashSet::new(),
            art_version: 0,
            shown_art_version: 0,
            shown: None,
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn metrics(&self) -> &Metrics {
        &self.m
    }

    /// Pictures are still being decoded: the loop should come back soon to take them in.
    pub fn art_busy(&self) -> bool {
        self.loader.inflight() > 0
    }

    pub fn thumbs_loaded(&self) -> usize {
        self.thumbs.len()
    }

    /// Takes in the pictures the worker has finished.
    fn take_art(&mut self) {
        for done in self.loader.poll() {
            match done.result {
                Ok(t) => {
                    self.thumbs.insert(done.key.clone(), t);
                    self.thumb_order.retain(|k| *k != done.key);
                    self.thumb_order.push_back(done.key);
                    while self.thumb_order.len() > THUMBS_KEPT {
                        if let Some(old) = self.thumb_order.pop_front() {
                            self.thumbs.remove(&old);
                            self.requested.remove(&old);
                        }
                    }
                    self.art_version += 1;
                }
                Err(e) => {
                    eprintln!("gfn: box art {}: {e}", done.key);
                    self.failed.insert(done.key);
                }
            }
        }
    }

    /// The address of the picture each visible tile should show, if the core has it in its cache.
    fn wanted_art(&self, app: &App) -> Vec<Option<String>> {
        app.games
            .iter()
            .skip(app.first_row * LIBRARY_COLUMNS)
            .take(LIBRARY_COLUMNS * GRID_ROWS)
            .map(|g| match app.art.get(&g.image_url) {
                Some(Art::File(_)) => Some(g.image_url.clone()),
                _ => None,
            })
            .collect()
    }

    /// Sends the decoder the pictures of visible tiles that are in the core's cache and not yet decoded.
    fn request_art(&mut self, app: &App) {
        if app.screen != Screen::Library {
            return;
        }
        for g in app.games.iter().skip(app.first_row * LIBRARY_COLUMNS).take(LIBRARY_COLUMNS * GRID_ROWS) {
            if let Some(Art::File(path)) = app.art.get(&g.image_url) {
                if !self.thumbs.contains_key(&g.image_url) && !self.requested.contains(&g.image_url) && !self.failed.contains(&g.image_url) {
                    self.requested.insert(g.image_url.clone());
                    self.loader.request(art::Job { key: g.image_url.clone(), path: path.into(), spec: self.spec });
                }
            }
        }
    }

    /// Puts the rules' state on the layers.
    pub fn show(&mut self, app: &App) {
        self.take_art();
        self.request_art(app);
        let plan = layout::plan(app, &self.m, &mut self.ui.fonts);
        let wanted = if app.screen == Screen::Library { self.wanted_art(app) } else { Vec::new() };
        let art_now: Vec<Option<String>> = wanted.into_iter().map(|k| k.filter(|k| self.thumbs.contains_key(k))).collect();
        let next = Shown { plan, streaming: app.streaming(), art: art_now, qr: if app.screen == Screen::SignIn { app.sign_in.qr_rows.clone() } else { Vec::new() } };
        if let Some(prev) = &self.shown {
            if prev.plan == next.plan && prev.streaming == next.streaming && prev.art == next.art && prev.qr == next.qr && self.shown_art_version == self.art_version {
                return;
            }
        }
        let prev = self.shown.take();
        self.apply(prev.as_ref(), &next, app);
        self.shown_art_version = self.art_version;
        self.shown = Some(next);
    }

    fn apply(&mut self, prev: Option<&Shown>, next: &Shown, app: &App) {
        if prev.map(|s| s.streaming) != Some(next.streaming) {
            self.wall.set(Props { visible: !next.streaming, ..Props::at(0.0, 0.0) });
        }
        self.apply_surfaces(prev.map(|s| &s.plan), &next.plan);
        self.apply_text(prev.map(|s| &s.plan), &next.plan);
        if prev.map(|s| &s.qr) != Some(&next.qr) {
            self.draw_qr(&next.qr, &next.plan);
        }
        self.apply_art(prev, next, app);
    }

    /// One glass layer per surface of the plan; a layer the plan does not name is hidden, one it names is moved and restyled only if
    /// what the previous plan said of it differs.
    fn apply_surfaces(&mut self, prev: Option<&Plan>, next: &Plan) {
        let find = |plan: Option<&Plan>, fill: Fill, slot: usize| -> Option<Surf> { plan.and_then(|p| p.surfs.iter().find(|s| s.fill == fill && s.slot == slot).cloned()) };
        let pal = &self.p;
        let sync = |layer: &GlassLayer, fill: Fill, slot: usize, style: &dyn Fn(&Surf) -> Glass| {
            let (was, now) = (find(prev, fill, slot), find(Some(next), fill, slot));
            if was == now && prev.is_some() {
                return;
            }
            match now {
                Some(s) => {
                    if was.as_ref().map(|w| (w.focus, w.rect.w, w.rect.h)) != Some((s.focus, s.rect.w, s.rect.h)) || prev.is_none() {
                        layer.restyle(style(&s));
                    }
                    layer.set(at(s.rect));
                }
                None => {
                    if was.is_some() || prev.is_none() {
                        layer.set(hidden());
                    }
                }
            }
        };
        sync(&self.panel, Fill::Panel, 0, &|_| look::card(pal, look::R_CARD, false, false));
        sync(&self.header_button, Fill::HeaderButton, 0, &|s| look::card(pal, s.rect.h / 2.0, s.focus, s.focus));
        sync(&self.button, Fill::Button, 0, &|s| look::card(pal, s.rect.h / 2.0, true, false));
        sync(&self.ring, Fill::Ring, 0, &|_| ring_glass(pal));
        sync(&self.status, Fill::Status, 0, &|_| status_glass());
        sync(&self.overlay, Fill::Overlay, 0, &|_| opaque_glass(pal, look::R_CARD));
        for (i, l) in self.settings_rows.iter().enumerate() {
            sync(l, Fill::SettingsRow, i, &|s| look::card(pal, 20.0, s.focus, false));
        }
        for (i, l) in self.overlay_rows.iter().enumerate() {
            sync(l, Fill::OverlayRow, i, &|s| look::card(pal, 20.0, s.focus, false));
        }
        for (i, l) in self.tiles.iter().enumerate() {
            sync(l, Fill::Tile, i, &|s| look::card(pal, look::R_TILE, s.focus, s.focus));
        }
        for (i, l) in self.help.iter().enumerate() {
            sync(l, Fill::Help, i, &|_| opaque_glass(pal, look::R_CARD));
        }
    }

    /// The text layers: each is redrawn when what the plan puts on it changed.
    fn apply_text(&mut self, prev: Option<&Plan>, next: &Plan) {
        let button = next.surface(Fill::Button).next().map(|s| s.rect);
        let layers: [(Lay, Frame); 7] = [
            (Lay::Head, Frame::new(0.0, 0.0, W, layout::grid_top(&self.m))),
            (Lay::Body, layout::panel_frame(&self.m)),
            (Lay::Button, button.unwrap_or(Frame::new(0.0, 0.0, 0.0, 0.0))),
            (Lay::Tiles, Frame::new(0.0, layout::grid_top(&self.m), W, H - layout::grid_top(&self.m))),
            (Lay::Overlay, layout::overlay_frame(&mut self.ui.fonts, &self.m)),
            (Lay::Status, layout::status_frame(&mut self.ui.fonts, &self.m)),
            (Lay::Help, layout::panel_frame(&self.m)),
        ];
        for (lay, origin) in layers {
            let pick = |plan: Option<&Plan>| -> (Vec<Txt>, Vec<layout::MarkBox>, Option<Frame>) {
                match plan {
                    Some(p) => (p.texts.iter().filter(|t| t.lay == lay).cloned().collect(), p.marks.iter().filter(|m| m.lay == lay).cloned().collect(), p.surface(Fill::Button).next().map(|s| s.rect)),
                    None => (Vec::new(), Vec::new(), None),
                }
            };
            let (texts, marks, bframe) = pick(Some(next));
            let (ptexts, pmarks, pframe) = pick(prev);
            let same = prev.is_some() && texts == ptexts && marks == pmarks && (lay != Lay::Button || bframe == pframe);
            if same {
                continue;
            }
            let pal = &self.p;
            let mut items: Vec<Item> = Vec::new();
            for mk in &marks {
                items.push(Item::Mark(Mark { form: mk.form, x: (mk.x - origin.x).round() as i32, y: (mk.y - origin.y).round() as i32, colour: ink_colour(pal, mk.ink) }));
            }
            for t in &texts {
                let mut run = Run::left(&t.text, t.ty, ink_colour(pal, t.ink), t.x - origin.x, t.y - origin.y, t.w);
                run.align = t.align;
                items.push(Item::Text(run));
            }
            match lay {
                Lay::Head => self.head.draw(&mut self.ui, &items),
                Lay::Body => self.body.draw(&mut self.ui, &items),
                Lay::Button => {
                    if let Some(b) = bframe {
                        self.button_text.set_props(Props::at(b.x, b.y));
                    }
                    self.button_text.draw(&mut self.ui, &items);
                }
                Lay::Tiles => self.tile_text.draw(&mut self.ui, &items),
                Lay::Overlay => self.overlay_text.draw(&mut self.ui, &items),
                Lay::Status => self.status_text.draw(&mut self.ui, &items),
                Lay::Help => self.help_text.draw(&mut self.ui, &items),
            }
        }
    }

    fn draw_qr(&mut self, qr: &[String], plan: &Plan) {
        let Some(frame) = plan.surface(Fill::Qr).next().map(|s| s.rect).filter(|_| !qr.is_empty()) else {
            self.qr.set(hidden());
            return;
        };
        let n = qr.len() as u32;
        let quiet = 2;
        let side = self.qr.width();
        let scale = (side / (n + 2 * quiet)).max(1);
        let side = side as usize;
        let px = self.qr.pixels_mut();
        px.fill(255);                                   /* white, opaque: the QR needs its quiet zone */
        for (y, row) in qr.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                if c != b'1' {
                    continue;
                }
                let x0 = ((x as u32 + quiet) * scale) as usize;
                let y0 = ((y as u32 + quiet) * scale) as usize;
                for yy in y0..(y0 + scale as usize).min(side) {
                    for xx in x0..(x0 + scale as usize).min(side) {
                        let at = (yy * side + xx) * 4;
                        px[at] = 0x17;
                        px[at + 1] = 0x13;
                        px[at + 2] = 0x1F;
                    }
                }
            }
        }
        self.qr.damage(None);
        self.qr.set(Props { visible: true, ..Props::at(frame.x, frame.y) });
    }

    /// Each visible tile's picture layer holds its art, or the placeholder until the art is decoded.
    fn apply_art(&mut self, prev: Option<&Shown>, next: &Shown, app: &App) {
        let slots = self.tile_art.len();
        let shown_art = |s: Option<&Shown>, i: usize| -> Option<Option<String>> { s.and_then(|s| s.art.get(i).cloned()) };
        for slot in 0..slots {
            let tile = next.plan.surfaces_of(Fill::Tile, slot);
            let Some(tile) = tile else {
                if prev.is_none_or(|p| p.plan.surfaces_of(Fill::Tile, slot).is_some()) {
                    self.tile_art[slot].set(hidden());
                }
                continue;
            };
            let want = next.art.get(slot).cloned().unwrap_or(None);
            let was_there = prev.and_then(|p| p.plan.surfaces_of(Fill::Tile, slot)).is_some();
            let same_picture = was_there && shown_art(prev, slot) == Some(want.clone()) && self.shown_art_version == self.art_version;
            if !same_picture || prev.is_none() {
                let thumb = want.as_ref().and_then(|k| self.thumbs.get(k)).unwrap_or(&self.placeholder);
                let layer = &mut self.tile_art[slot];
                layer.pixels_mut().copy_from_slice(&thumb.rgba);
                layer.damage(None);
            }
            self.tile_art[slot].set(at(tile.rect));
        }
        let _ = app;
    }
}

impl Plan {
    fn surfaces_of(&self, fill: Fill, slot: usize) -> Option<&Surf> {
        self.surfs.iter().find(|s| s.fill == fill && s.slot == slot)
    }
}

/// Every mark a screen can put in a text layer, for the atlas to hold before the first draw: found by planning each screen once.
fn forms_to_warm(m: &Metrics, fonts: &mut ui::text::Fonts) -> Vec<Form> {
    let mut forms: Vec<Form> = Vec::new();
    let mut note = |p: &Plan| {
        for mk in &p.marks {
            if !forms.contains(&mk.form) {
                forms.push(mk.form);
            }
        }
    };
    let mut app = App::new();
    app.signed_in = true;
    app.user_name = "Name".into();
    app.screen = Screen::Library;
    note(&layout::plan(&app, m, fonts));
    app.screen = Screen::SignIn;
    app.sign_in.user_code = "CODE".into();
    note(&layout::plan(&app, m, fonts));
    app.screen = Screen::Detail;
    note(&layout::plan(&app, m, fonts));
    app.screen = Screen::Settings;
    for i in 0..Row::ALL.len() {
        app.settings_row = i;
        note(&layout::plan(&app, m, fonts));
    }
    app.help_open = true;
    note(&layout::plan(&app, m, fonts));
    forms.push(Form::Icon(Icon::Info, m.icon(28.0)));
    forms
}
