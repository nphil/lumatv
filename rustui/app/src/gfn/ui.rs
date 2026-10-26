//! The GeForce NOW app's screens on the toolkit, in LumaTV's look (`look`: the glass of LumaHome on the shipped wallpaper) and
//! Lucent's TV rules: little copy (a row is a name and a value), one button size per screen, chrome over the stream that leaves
//! after inactivity and never blocks the way out, an opaque backdrop behind the question of leaving a game. The layers are made
//! once and restyled or redrawn when the rules' state changes; [`GfnUi::show`] sends only what differs from the last call.
//!
//! Over the stream the wallpaper and every card are hidden, so the window is transparent and the video plane shows through
//! (`docs/research/gfn-app-result.md`, "The app"); the status line and the overlay are the only pixels.
use crate::gfn::app::{App, LIBRARY_COLUMNS, OVERLAY_ROWS, Screen};
use crate::look::{self, MED, Palette, SEMI};
use compositor_protocol::{Props, Wallpaper, WallpaperKind};
use std::io;
use std::path::Path;
use ui::components::{Form, Icon, Item, Mark, Run, TextLayer, line_height};
use ui::geometry::Frame;
use ui::text::Type;
use ui::{GlassLayer, ImageLayer, Ui, WallpaperLayer};

pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;

const SAFE_X: f32 = 64.0;
const SAFE_Y: f32 = 54.0;
const PAD: f32 = 48.0;

/// The one card the sign-in, detail, launching and error screens share, centred.
pub const PANEL: Frame = Frame::new(360.0, 150.0, 1200.0, 780.0);
/// The overlay over the stream: smaller, centred.
pub const OVERLAY: Frame = Frame::new(660.0, 280.0, 600.0, 520.0);
const OVERLAY_ROW_TOP: f32 = 120.0;
const OVERLAY_ROW_H: f32 = 84.0;
const OVERLAY_ROW_GAP: f32 = 12.0;
/// The status line over the stream, bottom left.
pub const STATUS: Frame = Frame::new(SAFE_X, H - SAFE_Y - 56.0, 760.0, 56.0);

/// The library grid: six tiles a row under the title, 2:3 posters.
const GRID_TOP: f32 = 160.0;
const TILE_W: f32 = 272.0;
const TILE_H: f32 = 256.0;
const TILE_GAP: f32 = 32.0;
pub const GRID_ROWS: usize = 3;
const _: () = assert!(SAFE_X + LIBRARY_COLUMNS as f32 * TILE_W + (LIBRARY_COLUMNS as f32 - 1.0) * TILE_GAP <= W - SAFE_X);
const _: () = assert!(GRID_TOP + GRID_ROWS as f32 * TILE_H + (GRID_ROWS as f32 - 1.0) * TILE_GAP <= H - SAFE_Y);

const QR_SIDE: u32 = 348;
const BUTTON_H: f32 = 68.0;
const BUTTON_W: f32 = 260.0;

const Z_WALL: i32 = 0;
const Z_PANEL: i32 = 10;
const Z_TILE: i32 = 11;
const Z_TEXT: i32 = 14;
const Z_QR: i32 = 15;
const Z_BUTTON: i32 = 16;
const Z_BUTTON_TEXT: i32 = 18;
const Z_STATUS: i32 = 30;
const Z_STATUS_TEXT: i32 = 32;
const Z_OVERLAY: i32 = 40;
const Z_OVERLAY_ROW: i32 = 42;
const Z_OVERLAY_TEXT: i32 = 44;

const DISPLAY: Type = look::ty(56, SEMI);
const TITLE: Type = look::ty(44, SEMI);
const CAPTION: Type = look::ty(26, MED);
const LABEL: Type = look::ty(28, MED);
const VALUE: Type = look::ty(28, SEMI);
const TILE_TEXT: Type = look::ty(24, SEMI);
const CODE: Type = look::ty(64, SEMI);
const SMALL: Type = look::ty(22, MED);

/// The words of each screen, as rows a test can read without a GPU.
pub fn rows(app: &App) -> Vec<(String, String)> {
    match app.screen {
        Screen::Starting => vec![("GeForce NOW".into(), "Starting".into())],
        Screen::SignIn => {
            let s = &app.sign_in;
            if s.user_code.is_empty() {
                vec![("Sign in".into(), "Asking NVIDIA for a code".into())]
            } else {
                vec![("Code".into(), s.user_code.clone()), ("Where".into(), s.url.clone()), ("Status".into(), if s.status.is_empty() { "waiting".into() } else { s.status.clone() })]
            }
        }
        Screen::Library => vec![("Library".into(), format!("{} games", app.games.len())), ("Account".into(), format!("{} · {}", app.user_name, app.tier))],
        Screen::Detail => {
            let g = app.detail.clone().unwrap_or_default();
            let mut r = vec![("Game".into(), g.title), ("Store".into(), g.store), ("Status".into(), decision_text(&app.decision, &app.decision_message))];
            if app.runtime_capabilities["nativeHdrSupported"] == true {
                r.push(("Stream".into(), "4K · 120 fps · HDR10".into()));
            }
            r
        }
        Screen::Launching => {
            let s = app.session.as_ref();
            vec![
                ("Game".into(), app.detail.as_ref().map(|g| g.title.clone()).unwrap_or_default()),
                ("Status".into(), s.map(|s| phase_text(&s.phase, s.queue_position)).unwrap_or_else(|| "Asking for a seat".into())),
            ]
        }
        Screen::Stream | Screen::Overlay => vec![("Stream".into(), stats_line(app))],
        Screen::Error => vec![("Problem".into(), app.error.clone())],
    }
}

fn decision_text(status: &str, message: &str) -> String {
    match status {
        "" => "Checking".into(),
        "ready" => "Ready".into(),
        "ownership_required" => "Needs the game in a store library".into(),
        "subscription_required" => "Needs a membership".into(),
        "patching" => "Being updated".into(),
        "maintenance" => "In maintenance".into(),
        other => {
            if message.is_empty() {
                other.replace('_', " ")
            } else {
                message.to_owned()
            }
        }
    }
}

fn phase_text(phase: &str, queue: i64) -> String {
    match phase {
        "queued" if queue > 0 => format!("In the queue, place {queue}"),
        "queued" => "In the queue".into(),
        "setup" | "starting" => "Setting up the seat".into(),
        "ready" => "Connecting".into(),
        "resuming" => "Resuming".into(),
        "" => "Asking for a seat".into(),
        other => other.replace('_', " "),
    }
}

/// The status line's words: fps, bitrate and what the engine says, short.
pub fn stats_line(app: &App) -> String {
    let s = &app.stats;
    let mut parts = Vec::new();
    if s.fps > 0.0 {
        parts.push(format!("{:.0} fps", s.fps));
    }
    if s.bitrate_mbps > 0.0 {
        parts.push(format!("{:.0} Mbit/s", s.bitrate_mbps));
    }
    if s.queue_drops > 0 {
        parts.push(format!("{} dropped", s.queue_drops));
    }
    if !s.input_ready && s.status == "streaming" {
        parts.push("no input yet".into());
    }
    if parts.is_empty() {
        if s.message.is_empty() { s.status.clone() } else { s.message.clone() }
    } else {
        parts.join(" · ")
    }
}

pub fn hint(screen: Screen) -> &'static str {
    match screen {
        Screen::Starting => "",
        Screen::SignIn => "Open the address on a phone and enter the code  ·  Back leaves",
        Screen::Library => "OK opens a game  ·  Back leaves",
        Screen::Detail => "OK plays  ·  Back returns to the library",
        Screen::Launching => "Back gives the seat up",
        Screen::Stream => "",
        Screen::Overlay => "OK chooses  ·  Back returns to the game",
        Screen::Error => "OK returns",
    }
}

/// The tile's frame in the grid, for the index as shown (the grid scrolls by whole rows).
pub fn tile_frame(index: usize, first_row: usize) -> Option<Frame> {
    let row = index / LIBRARY_COLUMNS;
    if row < first_row || row >= first_row + GRID_ROWS {
        return None;
    }
    let col = index % LIBRARY_COLUMNS;
    Some(Frame::new(SAFE_X + col as f32 * (TILE_W + TILE_GAP), GRID_TOP + (row - first_row) as f32 * (TILE_H + TILE_GAP), TILE_W, TILE_H))
}

/// The first row shown so that the focused tile is on the screen.
pub fn first_row_for(focus: usize, current: usize) -> usize {
    let row = focus / LIBRARY_COLUMNS;
    if row < current {
        row
    } else if row >= current + GRID_ROWS {
        row + 1 - GRID_ROWS
    } else {
        current
    }
}

#[derive(Clone, PartialEq)]
struct Shown {
    screen: Screen,
    rows: Vec<(String, String)>,
    focus: usize,
    first_row: usize,
    tiles: Vec<String>,
    overlay_row: usize,
    chrome: bool,
    show_stats: bool,
    qr: Vec<String>,
}

pub struct GfnUi {
    pub ui: Ui,
    p: Palette,
    wall: WallpaperLayer,
    panel: GlassLayer,
    head: TextLayer,
    body: TextLayer,
    qr: ImageLayer,
    button: GlassLayer,
    button_text: TextLayer,
    tiles: Vec<GlassLayer>,
    tile_text: TextLayer,
    status: GlassLayer,
    status_text: TextLayer,
    overlay: GlassLayer,
    overlay_rows: Vec<GlassLayer>,
    overlay_text: TextLayer,
    shown: Option<Shown>,
    first_row: usize,
}

const BUTTON: Frame = Frame::new(PANEL.x + PAD, PANEL.y + PANEL.h - PAD - 48.0 - BUTTON_H, BUTTON_W, BUTTON_H);
const QR_AT: (f32, f32) = (PANEL.x + PANEL.w - PAD - QR_SIDE as f32, PANEL.y + 150.0);

impl GfnUi {
    pub fn build(mut ui: Ui, assets: &Path, land: &mut dyn FnMut(&mut Ui) -> io::Result<()>) -> io::Result<Self> {
        let p = look::palette();
        ui.warm(&[DISPLAY, TITLE, CAPTION, LABEL, VALUE, TILE_TEXT, CODE, SMALL]);
        land(&mut ui)?;
        let inner_w = PANEL.w - 2.0 * PAD;
        ui.warm_marks(&[Form::Disc(96), Form::Icon(Icon::Play, 44), Form::Rule(inner_w as u32)]);
        land(&mut ui)?;

        let wall = {
            let mut layer = ui.scene.wallpaper(W as u32, H as u32, Z_WALL, Wallpaper { kind: WallpaperKind::Silk, speed: 0.0, intensity: 0.0, fps: 0 }, Props::at(0.0, 0.0))?;
            let pixels = crate::home::wallpaper_pixels(&assets.join("wallpaper/wall_1080.png"), W as u32, H as u32)?;
            layer.pixels_mut().ok_or_else(|| io::Error::other("the wallpaper has no picture"))?.copy_from_slice(&pixels);
            layer.damage(None);
            layer
        };
        let panel = ui.scene.glass(PANEL.w as u32, PANEL.h as u32, Z_PANEL, look::card(&p, look::R_CARD, false, false), Props::at(PANEL.x, PANEL.y));
        let head = TextLayer::new(&mut ui, Frame::new(0.0, 0.0, W, GRID_TOP), Z_TEXT, &[])?;
        let body = TextLayer::new(&mut ui, PANEL, Z_TEXT + 1, &[])?;
        let qr = ui.scene.image(QR_SIDE, QR_SIDE, Z_QR, Props { visible: false, ..Props::at(QR_AT.0, QR_AT.1) })?;
        let button = ui.scene.glass(BUTTON.w as u32, BUTTON.h as u32, Z_BUTTON, look::card(&p, BUTTON_H / 2.0, true, false), Props { visible: false, ..Props::at(BUTTON.x, BUTTON.y) });
        let button_text = TextLayer::new(&mut ui, BUTTON, Z_BUTTON_TEXT, &[])?;
        let mut tiles = Vec::with_capacity(LIBRARY_COLUMNS * GRID_ROWS);
        for i in 0..LIBRARY_COLUMNS * GRID_ROWS {
            let f = tile_frame(i, 0).expect("a grid slot");
            tiles.push(ui.scene.glass(TILE_W as u32, TILE_H as u32, Z_TILE, look::card(&p, look::R_TILE, false, false), Props { visible: false, ..Props::at(f.x, f.y) }));
        }
        let tile_text = TextLayer::new(&mut ui, Frame::new(0.0, GRID_TOP, W, H - GRID_TOP), Z_TEXT + 2, &[])?;
        let status = ui.scene.glass(STATUS.w as u32, STATUS.h as u32, Z_STATUS, look::pill(), Props { visible: false, ..Props::at(STATUS.x, STATUS.y) });
        let status_text = TextLayer::new(&mut ui, STATUS, Z_STATUS_TEXT, &[])?;
        let overlay = ui.scene.glass(OVERLAY.w as u32, OVERLAY.h as u32, Z_OVERLAY, overlay_glass(&p), Props { visible: false, ..Props::at(OVERLAY.x, OVERLAY.y) });
        let mut overlay_rows = Vec::new();
        for i in 0..OVERLAY_ROWS.len() {
            let f = overlay_row_frame(i);
            overlay_rows.push(ui.scene.glass(f.w as u32, f.h as u32, Z_OVERLAY_ROW, look::card(&p, 20.0, false, false), Props { visible: false, ..Props::at(f.x, f.y) }));
        }
        let overlay_text = TextLayer::new(&mut ui, OVERLAY, Z_OVERLAY_TEXT, &[])?;
        land(&mut ui)?;
        Ok(Self {
            ui,
            p,
            wall,
            panel,
            head,
            body,
            qr,
            button,
            button_text,
            tiles,
            tile_text,
            status,
            status_text,
            overlay,
            overlay_rows,
            overlay_text,
            shown: None,
            first_row: 0,
        })
    }

    /// Puts the rules' state on the layers.
    pub fn show(&mut self, app: &App) {
        if app.screen == Screen::Library {
            self.first_row = first_row_for(app.focus, self.first_row);
        }
        let next = Shown {
            screen: app.screen,
            rows: rows(app),
            focus: app.focus,
            first_row: self.first_row,
            tiles: if app.screen == Screen::Library {
                app.games.iter().skip(self.first_row * LIBRARY_COLUMNS).take(LIBRARY_COLUMNS * GRID_ROWS).map(|g| g.title.clone()).collect()
            } else {
                Vec::new()
            },
            overlay_row: app.overlay_row,
            chrome: app.chrome_visible(),
            show_stats: app.show_stats,
            qr: if app.screen == Screen::SignIn { app.sign_in.qr_rows.clone() } else { Vec::new() },
        };
        if self.shown.as_ref() == Some(&next) {
            return;
        }
        let prev = self.shown.take();
        let screen_changed = prev.as_ref().map(|s| s.screen) != Some(next.screen);
        let streaming = app.streaming();
        if screen_changed {
            self.wall.set(Props { visible: !streaming, ..Props::at(0.0, 0.0) });
            let panel_screens = matches!(next.screen, Screen::Starting | Screen::SignIn | Screen::Detail | Screen::Launching | Screen::Error);
            self.panel.set(Props { visible: panel_screens, ..Props::at(PANEL.x, PANEL.y) });
            let button_on = matches!(next.screen, Screen::Detail | Screen::Error);
            self.button.set(Props { visible: button_on, ..Props::at(BUTTON.x, BUTTON.y) });
            self.button_text.draw(&mut self.ui, &if button_on { button_items(&self.p, next.screen) } else { Vec::new() });
            self.overlay.set(Props { visible: next.screen == Screen::Overlay, ..Props::at(OVERLAY.x, OVERLAY.y) });
            for (i, row) in self.overlay_rows.iter().enumerate() {
                let f = overlay_row_frame(i);
                row.set(Props { visible: next.screen == Screen::Overlay, ..Props::at(f.x, f.y) });
            }
            self.draw_head(&next);
        }
        if screen_changed || prev.as_ref().map(|s| &s.rows) != Some(&next.rows) || (next.screen == Screen::Detail && prev.as_ref().map(|s| s.screen) != Some(Screen::Detail)) {
            self.draw_body(&next, app);
        }
        if prev.as_ref().map(|s| &s.qr) != Some(&next.qr) {
            self.draw_qr(&next.qr);
        }
        if next.screen == Screen::Library {
            if screen_changed || prev.as_ref().map(|s| (&s.tiles, s.first_row, s.focus)) != Some((&next.tiles, next.first_row, next.focus)) {
                self.draw_tiles(&next);
            }
        } else if prev.as_ref().is_some_and(|s| s.screen == Screen::Library) {
            for t in &self.tiles {
                t.set(Props { visible: false, ..Props::at(0.0, 0.0) });
            }
            self.tile_text.draw(&mut self.ui, &[]);
        }
        if next.screen == Screen::Overlay && (screen_changed || prev.as_ref().map(|s| s.overlay_row) != Some(next.overlay_row)) {
            self.draw_overlay(&next);
        } else if next.screen != Screen::Overlay && prev.as_ref().is_some_and(|s| s.screen == Screen::Overlay) {
            self.overlay_text.draw(&mut self.ui, &[]);
        }
        let status_on = next.screen == Screen::Stream && (next.chrome || next.show_stats);
        if screen_changed || prev.as_ref().map(|s| (s.chrome, s.show_stats, &s.rows)) != Some((next.chrome, next.show_stats, &next.rows)) {
            self.status.set(Props { visible: status_on, ..Props::at(STATUS.x, STATUS.y) });
            let items = if status_on {
                let lh = line_height(VALUE);
                vec![Item::Text(Run::left(&next.rows[0].1, VALUE, self.p.ink, 24.0, ((STATUS.h - lh) / 2.0).round(), STATUS.w - 48.0))]
            } else {
                Vec::new()
            };
            self.status_text.draw(&mut self.ui, &items);
        }
        self.shown = Some(next);
    }

    fn draw_head(&mut self, s: &Shown) {
        let p = &self.p;
        let mut items: Vec<Item> = Vec::new();
        if s.screen == Screen::Library {
            items.push(Item::Mark(Mark::disc(SAFE_X as i32, SAFE_Y as i32, 72, p.accent)));
            items.push(Item::Mark(Mark::icon(Icon::Play, 36, SAFE_X as i32 + 18, SAFE_Y as i32 + 18, p.ink_on_accent)));
            items.push(Item::Text(Run::left("GeForce NOW", TITLE, p.ink, SAFE_X + 96.0, SAFE_Y + 6.0, 700.0)));
            if let Some((_, account)) = s.rows.get(1) {
                items.push(Item::Text(Run::right(account, CAPTION, p.ink2, W - SAFE_X - 700.0, SAFE_Y + 20.0, 700.0)));
            }
        }
        self.head.draw(&mut self.ui, &items);
    }

    fn draw_body(&mut self, s: &Shown, app: &App) {
        let p = &self.p;
        let inner_w = PANEL.w - 2.0 * PAD;
        let lh = line_height(VALUE);
        let mut items: Vec<Item> = Vec::new();
        let title = match s.screen {
            Screen::Starting => "GeForce NOW",
            Screen::SignIn => "Sign in",
            Screen::Detail => app.detail.as_ref().map(|g| g.title.as_str()).unwrap_or("Game"),
            Screen::Launching => "Starting",
            Screen::Error => "Something went wrong",
            _ => "",
        };
        if !title.is_empty() {
            items.push(Item::Text(Run::left(title, DISPLAY, p.ink, PAD, 44.0, inner_w)));
            items.push(Item::Mark(Mark::rule(PAD as i32, 130, inner_w as u32, look::white(0.10))));
        }
        match s.screen {
            Screen::SignIn if !app.sign_in.user_code.is_empty() => {
                items.push(Item::Text(Run::left(&app.sign_in.user_code, CODE, p.ink, PAD, 176.0, inner_w - QR_SIDE as f32 - 40.0)));
                items.push(Item::Text(Run::left(&app.sign_in.url, LABEL, p.ink2, PAD, 176.0 + line_height(CODE) + 16.0, inner_w - QR_SIDE as f32 - 40.0)));
                let status = if app.sign_in.status == "pending" || app.sign_in.status.is_empty() { "Waiting for the sign-in" } else { app.sign_in.status.as_str() };
                items.push(Item::Text(Run::left(status, CAPTION, p.ink3, PAD, 176.0 + line_height(CODE) + 16.0 + line_height(LABEL) + 24.0, inner_w - QR_SIDE as f32 - 40.0)));
            }
            Screen::Starting | Screen::SignIn | Screen::Detail | Screen::Launching | Screen::Error => {
                let label_w = 220.0;
                let rows = if s.screen == Screen::Detail { &s.rows[1..] } else { &s.rows[..] };
                for (i, (name, text)) in rows.iter().enumerate() {
                    let y = 176.0 + i as f32 * 64.0 + ((64.0 - lh) / 2.0).round();
                    items.push(Item::Text(Run::left(name, LABEL, p.ink2, PAD, y, label_w)));
                    items.push(Item::Text(Run::left(text, VALUE, p.ink, PAD + label_w, y, inner_w - label_w)));
                }
            }
            _ => {}
        }
        let hint = hint(s.screen);
        if !hint.is_empty() && s.screen != Screen::Library {
            items.push(Item::Text(Run::left(hint, SMALL, p.ink3, PAD, PANEL.h - PAD - line_height(SMALL), inner_w)));
        }
        self.body.draw(&mut self.ui, &items);
    }

    fn draw_qr(&mut self, qr: &[String]) {
        if qr.is_empty() {
            self.qr.set(Props { visible: false, ..Props::at(QR_AT.0, QR_AT.1) });
            return;
        }
        let n = qr.len() as u32;
        let quiet = 2;
        let scale = (QR_SIDE / (n + 2 * quiet)).max(1);
        let side = QR_SIDE as usize;
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
        self.qr.set(Props { visible: true, ..Props::at(QR_AT.0, QR_AT.1) });
    }

    fn draw_tiles(&mut self, s: &Shown) {
        let p = &self.p;
        let mut items: Vec<Item> = Vec::new();
        let lh = line_height(TILE_TEXT);
        for (slot, tile) in self.tiles.iter().enumerate() {
            let index = s.first_row * LIBRARY_COLUMNS + slot;
            let Some(title) = s.tiles.get(slot) else {
                tile.set(Props { visible: false, ..Props::at(0.0, 0.0) });
                continue;
            };
            let f = tile_frame(index, s.first_row).expect("a shown slot");
            let focus = index == s.focus;
            tile.set(Props { visible: true, ..Props::at(f.x, f.y) });
            tile.restyle(look::card(p, look::R_TILE, focus, focus));
            items.push(Item::Text(Run::centre(title, TILE_TEXT, p.ink, f.x, f.y - GRID_TOP + TILE_H - 24.0 - 2.0 * lh, f.w)));
        }
        self.tile_text.draw(&mut self.ui, &items);
    }

    fn draw_overlay(&mut self, s: &Shown) {
        let p = &self.p;
        let mut items: Vec<Item> = Vec::new();
        items.push(Item::Text(Run::left("Game", TITLE, p.ink, PAD, 36.0, OVERLAY.w - 2.0 * PAD)));
        let lh = line_height(VALUE);
        for (i, (row, name)) in self.overlay_rows.iter().zip(OVERLAY_ROWS).enumerate() {
            let f = overlay_row_frame(i);
            let focus = i == s.overlay_row;
            row.restyle(look::card(p, 20.0, focus, false));
            let colour = if i == OVERLAY_ROWS.len() - 1 { p.second } else { p.ink };
            items.push(Item::Text(Run::left(name, VALUE, colour, f.x - OVERLAY.x + 28.0, f.y - OVERLAY.y + ((f.h - lh) / 2.0).round(), f.w - 56.0)));
        }
        items.push(Item::Text(Run::left(hint(Screen::Overlay), SMALL, p.ink3, PAD, OVERLAY.h - PAD - line_height(SMALL), OVERLAY.w - 2.0 * PAD)));
        self.overlay_text.draw(&mut self.ui, &items);
    }
}

fn button_items(p: &Palette, screen: Screen) -> Vec<Item<'static>> {
    let text = match screen {
        Screen::Detail => "Play",
        Screen::Error => "OK",
        _ => "",
    };
    let lh = line_height(VALUE);
    vec![Item::Text(Run::centre(text, VALUE, p.ink, 0.0, ((BUTTON_H - lh) / 2.0).round(), BUTTON.w))]
}

fn overlay_row_frame(i: usize) -> Frame {
    Frame::new(OVERLAY.x + PAD, OVERLAY.y + OVERLAY_ROW_TOP + i as f32 * (OVERLAY_ROW_H + OVERLAY_ROW_GAP), OVERLAY.w - 2.0 * PAD, OVERLAY_ROW_H)
}

/// The overlay's backdrop is opaque enough to read on and to carry the question of leaving a game (Lucent: an opaque backdrop
/// behind a confirmation).
fn overlay_glass(p: &Palette) -> compositor_protocol::Glass {
    let mut g = look::card(p, look::R_CARD, false, true);
    g.tint = look::hex("#F0120E1A");
    g.backdrop = compositor_protocol::Backdrop::None;
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfn::app::{Game, Stats};

    #[test]
    fn the_grid_scrolls_by_rows_and_keeps_the_focus_on_the_screen() {
        assert_eq!(first_row_for(0, 0), 0);
        assert_eq!(first_row_for(17, 0), 0);
        assert_eq!(first_row_for(18, 0), 1);
        assert_eq!(first_row_for(30, 1), 3);
        assert_eq!(first_row_for(3, 3), 0);
        assert!(tile_frame(0, 0).is_some());
        assert!(tile_frame(18, 0).is_none());
        let f = tile_frame(7, 0).unwrap();
        assert_eq!((f.x, f.y), (SAFE_X + TILE_W + TILE_GAP, GRID_TOP + TILE_H + TILE_GAP));
        let last = tile_frame(17, 0).unwrap();
        assert!(last.x + last.w <= W - SAFE_X && last.y + last.h <= H - SAFE_Y);
    }

    #[test]
    fn the_rows_say_little_and_the_status_line_is_short() {
        let mut app = App::new();
        app.screen = Screen::Detail;
        app.detail = Some(Game { title: "Portal 2".into(), store: "STEAM".into(), ..Default::default() });
        app.decision = "ownership_required".into();
        let r = rows(&app);
        assert_eq!(r[0], ("Game".to_string(), "Portal 2".to_string()));
        assert_eq!(r[2].1, "Needs the game in a store library");
        app.stats = Stats { status: "streaming".into(), fps: 119.6, bitrate_mbps: 78.4, input_ready: true, ..Default::default() };
        assert_eq!(stats_line(&app), "120 fps · 78 Mbit/s");
        app.stats.queue_drops = 3;
        app.stats.input_ready = false;
        assert_eq!(stats_line(&app), "120 fps · 78 Mbit/s · 3 dropped · no input yet");
        app.stats = Stats { status: "starting".into(), message: "Launching the native NVST media runtime…".into(), ..Default::default() };
        assert!(stats_line(&app).starts_with("Launching"));
        assert_eq!(phase_text("queued", 7), "In the queue, place 7");
    }

    /// Lucent: nothing over anything. The card's rows, button and hint, the overlay's rows and hint, the status line, all apart and inside.
    #[test]
    fn nothing_overlaps() {
        let rows_end = 176.0 + 5.0 * 64.0;
        assert!(rows_end + 16.0 <= BUTTON.y - PANEL.y, "five rows end at {rows_end}, the button starts at {}", BUTTON.y - PANEL.y);
        assert!(BUTTON.y + BUTTON.h <= PANEL.y + PANEL.h - PAD - line_height(SMALL) - 16.0);
        assert!(QR_AT.0 + QR_SIDE as f32 <= PANEL.x + PANEL.w - PAD && QR_AT.1 + QR_SIDE as f32 <= BUTTON.y - 16.0);
        let code_end = 176.0 + line_height(CODE) + 16.0 + line_height(LABEL) + 24.0 + line_height(CAPTION);
        assert!(code_end + 16.0 <= PANEL.h - PAD - line_height(SMALL));
        let last = overlay_row_frame(OVERLAY_ROWS.len() - 1);
        assert!(last.y + last.h + 16.0 <= OVERLAY.y + OVERLAY.h - PAD - line_height(SMALL));
        assert!(OVERLAY_ROW_TOP >= 36.0 + line_height(TITLE) + 16.0);
        assert!(STATUS.y + STATUS.h <= H - SAFE_Y && STATUS.h >= line_height(VALUE));
    }
}
