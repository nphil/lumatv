//! Where everything on the GeForce NOW app's screens goes, computed and not drawn: for the rules' state and a text scale, a [`Plan`] of
//! surfaces (glass), texts and marks, in screen coordinates. `ui.rs` draws a plan; the tests here read the same plan for the
//! checks Lucent asks of a screen, at 100 % and at 200 % text (`skill://design-language`, "No overlapping text, checked automatically").
//!
//! Text scale moves sizes, not designs: a larger type makes rows taller, wraps what does not fit and widens the cards, and a screen
//! that cannot hold its content at 200 % is a failing test, not a clipped label. Chrome (the header's disc and button) grows half as
//! fast as the text. At 100 % the numbers are the ones the screens always had.
use crate::gfn::app::{App, GRID_ROWS, LIBRARY_COLUMNS, OVERLAY_ROWS, Screen};
use crate::gfn::prefs::Row;
use crate::look::{self, MED, SEMI};
use ui::components::{Align, Form, Icon, ellipsise, line_height, wrap, wrap_to};
use ui::geometry::Frame;
use ui::text::{Fonts, Type};

pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;
pub const SAFE_X: f32 = 64.0;
pub const SAFE_Y: f32 = 54.0;
pub const PAD: f32 = 48.0;

pub const TILE_W: f32 = 272.0;
/// Box art is 3:4, a little shorter: two rows of these fit under the header at 200 % text.
pub const TILE_H: f32 = 360.0;
pub const TILE_GAP: f32 = 32.0;
/// Text inside a tile keeps this far from its sides.
pub const TILE_PAD: f32 = 16.0;
const _: () = assert!(SAFE_X + LIBRARY_COLUMNS as f32 * TILE_W + (LIBRARY_COLUMNS as f32 - 1.0) * TILE_GAP <= W - SAFE_X);

/// The text sizes of the screens at a scale (rounded to whole pixels, as the font cache keys them).
#[derive(Clone, Copy, Debug)]
pub struct Types {
    pub display: Type,
    pub title: Type,
    pub caption: Type,
    pub label: Type,
    pub value: Type,
    pub tile: Type,
    pub code: Type,
    pub small: Type,
}

#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Text scale: 1.0 is the design, 2.0 is Lucent's 200 % check.
    pub s: f32,
    /// The scale of chrome (icons, the header's disc): half of the text's growth.
    pub chrome: f32,
    pub ty: Types,
}

impl Metrics {
    pub fn new(scale: f32) -> Self {
        let s = scale.clamp(1.0, 2.0);
        let px = |base: f32| (base * s).round() as u32;
        Self {
            s,
            chrome: 1.0 + (s - 1.0) * 0.5,
            ty: Types {
                display: look::ty(px(56.0), SEMI),
                title: look::ty(px(44.0), SEMI),
                caption: look::ty(px(26.0), MED),
                label: look::ty(px(28.0), MED),
                value: look::ty(px(28.0), SEMI),
                tile: look::ty(px(24.0), SEMI),
                code: look::ty(px(64.0), SEMI),
                small: look::ty(px(22.0), MED),
            },
        }
    }

    /// Every type a screen draws in, for the atlas to be warmed with.
    pub fn types(&self) -> [Type; 8] {
        let t = &self.ty;
        [t.display, t.title, t.caption, t.label, t.value, t.tile, t.code, t.small]
    }

    pub fn qr_side(&self) -> u32 {
        (348.0 * self.chrome).round() as u32
    }

    pub fn head_side(&self) -> f32 {
        (72.0 * self.chrome).round()
    }

    pub fn icon(&self, base: f32) -> u32 {
        (base * self.chrome).round() as u32
    }
}

// ---- the plan ---------------------------------------------------------------------------------------------------------------------

/// A colour role; `ui.rs` maps it to the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ink {
    Ink,
    Ink2,
    OnArt,
    OnAccent,
    Danger,
    Rule,
    Wash,
    Accent,
}

/// Which text layer carries a piece of text or a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lay {
    Head,
    Body,
    Button,
    Tiles,
    Overlay,
    Status,
    Help,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Txt {
    pub lay: Lay,
    pub text: String,
    pub ty: Type,
    pub ink: Ink,
    /// The box the line is aligned in, and the top of its line box.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub align: Align,
    /// The surface it is drawn on, for the checks.
    pub on: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkBox {
    pub lay: Lay,
    pub form: Form,
    pub x: f32,
    pub y: f32,
    pub ink: Ink,
    pub on: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fill {
    Panel,
    /// The sign-in's QR picture (an image, not glass).
    Qr,
    SettingsRow,
    OverlayRow,
    Tile,
    Button,
    Status,
    Overlay,
    Help,
    HeaderButton,
    Ring,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surf {
    pub fill: Fill,
    /// Which of the layers of this kind (the nth tile, the nth row).
    pub slot: usize,
    pub rect: Frame,
    pub focus: bool,
    pub parent: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub surfs: Vec<Surf>,
    pub texts: Vec<Txt>,
    pub marks: Vec<MarkBox>,
}

impl Plan {
    fn surf(&mut self, fill: Fill, slot: usize, rect: Frame, focus: bool, parent: Option<usize>) -> usize {
        self.surfs.push(Surf { fill, slot, rect, focus, parent });
        self.surfs.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, lay: Lay, text: impl Into<String>, ty: Type, ink: Ink, x: f32, y: f32, w: f32, align: Align, on: Option<usize>) {
        self.texts.push(Txt { lay, text: text.into(), ty, ink, x, y, w, align, on });
    }

    fn mark(&mut self, lay: Lay, form: Form, x: f32, y: f32, ink: Ink, on: Option<usize>) {
        self.marks.push(MarkBox { lay, form, x, y, ink, on });
    }

    pub fn surface(&self, fill: Fill) -> impl Iterator<Item = &Surf> {
        self.surfs.iter().filter(move |s| s.fill == fill)
    }
}

// ---- helpers ----------------------------------------------------------------------------------------------------------------------

/// `text` as at most `max` lines of at most `w` pixels: broken at spaces, the last line ending in an ellipsis when something had to go,
/// and a single word wider than a line cut as well.
pub fn lines(f: &mut Fonts, text: &str, ty: Type, w: f32, max: usize) -> Vec<String> {
    let broken = if max == 0 { wrap(f, text, ty, w) } else { wrap_to(f, text, ty, w, max) };
    broken.into_iter().map(|l| if f.measure(&l, ty).width > w { ellipsise(f, &l, ty, w) } else { l }).collect()
}

fn width(f: &mut Fonts, text: &str, ty: Type) -> f32 {
    f.measure(text, ty).width
}

/// Where a line of `w` wide starts in a box at `x` of `box_w`.
pub fn start_x(x: f32, box_w: f32, w: f32, align: Align) -> f32 {
    x + match align {
        Align::Left => 0.0,
        Align::Centre => ((box_w - w) / 2.0).max(0.0),
        Align::Right => (box_w - w).max(0.0),
    }
}

/// The centre card of the sign-in, detail, launching, error and settings screens.
pub fn panel_frame(m: &Metrics) -> Frame {
    let w = (1200.0 * (1.0 + (m.s - 1.0) * 0.2)).round();
    let h = (780.0 + (m.s - 1.0) * 192.0).min(H - 2.0 * SAFE_Y).round();
    Frame::new(((W - w) / 2.0).round(), ((H - h) / 2.0).round(), w, h)
}

pub fn inner_w(m: &Metrics) -> f32 {
    panel_frame(m).w - 2.0 * PAD
}

/// The one button of a screen: as wide as its label asks and never under the TV's 64 target.
pub fn button_size(f: &mut Fonts, m: &Metrics) -> (f32, f32) {
    let lh = line_height(m.ty.value);
    let label = width(f, "Play", m.ty.value).max(width(f, "OK", m.ty.value));
    ((260.0f32).max((label + 96.0).ceil()), (68.0f32).max(lh + 24.0))
}

// ---- panel screens ---------------------------------------------------------------------------------------------------------------

/// The title in the card's head: the display size if it fits one line, else the title size over at most two. Returns where the
/// body starts, panel-relative.
fn panel_head(p: &mut Plan, f: &mut Fonts, m: &Metrics, panel: Frame, title: &str, panel_surf: usize) -> f32 {
    let iw = inner_w(m);
    let (ty, ls) = if width(f, title, m.ty.display) <= iw { (m.ty.display, vec![title.to_owned()]) } else { (m.ty.title, lines(f, title, m.ty.title, iw, 2)) };
    let mut y = 44.0;
    for l in ls {
        p.text(Lay::Body, l, ty, Ink::Ink, panel.x + PAD, panel.y + y, iw, Align::Left, Some(panel_surf));
        y += line_height(ty);
    }
    p.mark(Lay::Body, Form::Rule(iw as u32), panel.x + PAD, panel.y + y + 8.0, Ink::Rule, Some(panel_surf));
    y + 8.0 + 46.0
}

/// The hint line at the card's foot, wrapped to the card; returns its top, panel-relative.
fn panel_hint(p: &mut Plan, f: &mut Fonts, m: &Metrics, panel: Frame, hint: &str, panel_surf: usize) -> f32 {
    let iw = inner_w(m);
    let ls = lines(f, hint, m.ty.small, iw, 0);
    let lh = line_height(m.ty.small);
    let top = panel.h - PAD - ls.len() as f32 * lh;
    for (i, l) in ls.into_iter().enumerate() {
        p.text(Lay::Body, l, m.ty.small, Ink::Ink2, panel.x + PAD, panel.y + top + i as f32 * lh, iw, Align::Left, Some(panel_surf));
    }
    top
}

/// Name and value rows from `y0` (panel-relative): the name in a column as wide as the longest asks, the value wrapped to the rest.
/// Returns where the rows end.
fn panel_rows(p: &mut Plan, f: &mut Fonts, m: &Metrics, panel: Frame, y0: f32, rows: &[(String, String)], max_lines: usize, panel_surf: usize) -> f32 {
    let t = &m.ty;
    let (lhv, lhl) = (line_height(t.value), line_height(t.label));
    let iw = inner_w(m);
    let label_w = rows.iter().map(|(n, _)| width(f, n, t.label)).fold(0.0f32, f32::max).ceil() + 40.0;
    let label_w = label_w.max(220.0);
    let vw = iw - label_w;
    let mut y = y0;
    for (name, value) in rows {
        let ls = lines(f, value, t.value, vw, max_lines);
        let h = (ls.len() as f32 * lhv + 24.0).max(lhv + 24.0);
        p.text(Lay::Body, name.clone(), t.label, Ink::Ink2, panel.x + PAD, panel.y + y + ((lhv + 24.0 - lhl) / 2.0).round(), label_w - 8.0, Align::Left, Some(panel_surf));
        for (i, l) in ls.into_iter().enumerate() {
            p.text(Lay::Body, l, t.value, Ink::Ink, panel.x + PAD + label_w, panel.y + y + 12.0 + i as f32 * lhv, vw, Align::Left, Some(panel_surf));
        }
        y += h;
    }
    y
}

/// The words of each screen as name and value rows, so that a test can read them without drawing.
pub fn rows(app: &App) -> Vec<(String, String)> {
    match app.screen {
        Screen::Starting => vec![("Status".into(), "Starting".into())],
        Screen::SignIn => {
            let s = &app.sign_in;
            if s.user_code.is_empty() {
                vec![("Status".into(), "Asking NVIDIA for a code".into())]
            } else {
                vec![("Code".into(), s.user_code.clone()), ("Where".into(), s.url.clone()), ("Status".into(), if s.status.is_empty() { "waiting".into() } else { s.status.clone() })]
            }
        }
        Screen::Library => vec![("Library".into(), format!("{} games", app.games.len())), ("Account".into(), account_text(app))],
        Screen::Settings => Row::ALL.iter().map(|r| (r.name().to_owned(), r.value(&app.prefs))).collect(),
        Screen::Detail => {
            let g = app.detail.clone().unwrap_or_default();
            let mut r = vec![("Game".into(), g.title), ("Store".into(), g.store), ("Status".into(), decision_text(&app.decision, &app.decision_message))];
            if app.runtime_capabilities["nativeHdrSupported"] == true {
                r.push(("Stream".into(), app.prefs.summary()));
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

pub fn account_text(app: &App) -> String {
    match (app.user_name.is_empty(), app.tier.is_empty()) {
        (true, true) => String::new(),
        (false, true) => app.user_name.clone(),
        (true, false) => app.tier.clone(),
        (false, false) => format!("{} · {}", app.user_name, app.tier),
    }
}

pub fn decision_text(status: &str, message: &str) -> String {
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

pub fn phase_text(phase: &str, queue: i64) -> String {
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
        Screen::Settings => "Left and Right change  ·  Hold OK explains  ·  Back returns",
        Screen::Detail => "OK plays  ·  Back returns to the library",
        Screen::Launching => "Back gives the seat up",
        Screen::Stream => "",
        Screen::Overlay => "OK chooses  ·  Back returns to the game",
        Screen::Error => "OK returns",
    }
}

fn panel_title(app: &App) -> String {
    match app.screen {
        Screen::Starting => "GeForce NOW".into(),
        Screen::SignIn => "Sign in".into(),
        Screen::Settings => "Settings".into(),
        Screen::Detail => app.detail.as_ref().map(|g| g.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| "Game".into()),
        Screen::Launching => "Starting".into(),
        Screen::Error => "Something went wrong".into(),
        _ => String::new(),
    }
}

fn plan_panel(app: &App, m: &Metrics, f: &mut Fonts) -> Plan {
    let mut p = Plan::default();
    let panel = panel_frame(m);
    let ps = p.surf(Fill::Panel, 0, panel, false, None);
    let body_y = panel_head(&mut p, f, m, panel, &panel_title(app), ps);
    let t = &m.ty;
    let iw = inner_w(m);
    let all = rows(app);
    let hint_top = panel_hint(&mut p, f, m, panel, hint(app.screen), ps);
    let _ = hint_top;
    match app.screen {
        Screen::SignIn if !app.sign_in.user_code.is_empty() => {
            let side = m.qr_side() as f32;
            let col = iw - side - 40.0;
            let qr = Frame::new(panel.x + panel.w - PAD - side, panel.y + body_y - 46.0 + 20.0, side, side);
            p.surf(Fill::Qr, 0, qr, false, Some(ps));
            let mut y = body_y;
            p.text(Lay::Body, app.sign_in.user_code.clone(), t.code, Ink::Ink, panel.x + PAD, panel.y + y, col, Align::Left, Some(ps));
            y += line_height(t.code) + 16.0;
            for l in lines(f, &app.sign_in.url, t.label, col, 2) {
                p.text(Lay::Body, l, t.label, Ink::Ink2, panel.x + PAD, panel.y + y, col, Align::Left, Some(ps));
                y += line_height(t.label);
            }
            y += 24.0 - 0.0;
            let status = if app.sign_in.status == "pending" || app.sign_in.status.is_empty() { "Waiting for the sign-in".to_owned() } else { app.sign_in.status.clone() };
            for l in lines(f, &status, t.caption, col, 2) {
                p.text(Lay::Body, l, t.caption, Ink::Ink2, panel.x + PAD, panel.y + y, col, Align::Left, Some(ps));
                y += line_height(t.caption);
            }
        }
        Screen::Detail => {
            let end = panel_rows(&mut p, f, m, panel, body_y, &all[1..], 3, ps);
            let _ = end;
            if app.decision == "ready" {
                let (bw, bh) = button_size(f, m);
                let hint_h = hint_height(f, m, app.screen);
                let by = panel.h - PAD - hint_h - 17.0 - bh;
                let b = p.surf(Fill::Button, 0, Frame::new(panel.x + PAD, panel.y + by, bw, bh), true, Some(ps));
                let lh = line_height(t.value);
                p.text(Lay::Button, "Play", t.value, Ink::Ink, panel.x + PAD, panel.y + by + ((bh - lh) / 2.0).round(), bw, Align::Centre, Some(b));
            }
        }
        Screen::Error => {
            panel_rows(&mut p, f, m, panel, body_y, &all, 4, ps);
            let (bw, bh) = button_size(f, m);
            let hint_h = hint_height(f, m, app.screen);
            let by = panel.h - PAD - hint_h - 17.0 - bh;
            let b = p.surf(Fill::Button, 0, Frame::new(panel.x + PAD, panel.y + by, bw, bh), true, Some(ps));
            let lh = line_height(t.value);
            p.text(Lay::Button, "OK", t.value, Ink::Ink, panel.x + PAD, panel.y + by + ((bh - lh) / 2.0).round(), bw, Align::Centre, Some(b));
        }
        _ => {
            panel_rows(&mut p, f, m, panel, body_y, &all, 3, ps);
        }
    }
    p
}

fn hint_height(f: &mut Fonts, m: &Metrics, screen: Screen) -> f32 {
    lines(f, hint(screen), m.ty.small, inner_w(m), 0).len() as f32 * line_height(m.ty.small)
}

// ---- settings --------------------------------------------------------------------------------------------------------------------

pub fn settings_row_h(m: &Metrics) -> f32 {
    line_height(m.ty.value) + 24.0
}

/// A settings row's frame; `body_y` is where the rows start in the card.
pub fn settings_row_frame(m: &Metrics, body_y: f32, i: usize) -> Frame {
    let panel = panel_frame(m);
    let h = settings_row_h(m);
    Frame::new(panel.x + PAD, panel.y + body_y + i as f32 * (h + 8.0), inner_w(m), h)
}

/// Where the card's body starts: the head's height, which depends on the type size alone ("Settings" is one line).
pub fn settings_body_y(m: &Metrics) -> f32 {
    44.0 + line_height(m.ty.display) + 8.0 + 46.0
}

const ROW_INSET: f32 = 28.0;

/// The help card of row `i`: its frame and its lines (the card's width is the card's inner width at most 840, widening with the text).
pub fn help_card(f: &mut Fonts, m: &Metrics, i: usize) -> (Frame, Vec<String>) {
    let panel = panel_frame(m);
    let t = &m.ty;
    let cw = ((840.0 * (1.0 + (m.s - 1.0) * 0.6)).round()).min(inner_w(m));
    let body = lines(f, Row::ALL[i].help(), t.label, cw - 2.0 * ROW_INSET, 0);
    let ch = ROW_INSET + line_height(t.label) + 10.0 + body.len() as f32 * line_height(t.label) + ROW_INSET;
    let row = settings_row_frame(m, settings_body_y(m), i);
    let below = row.bottom() + 12.0;
    let y = if below + ch <= panel.bottom() - PAD + 8.0 { below } else { (row.y - 12.0 - ch).max(panel.y + settings_body_y(m) - 46.0 + 8.0) };
    (Frame::new(panel.x + PAD, y.round(), cw, ch), body)
}

fn plan_settings(app: &App, m: &Metrics, f: &mut Fonts) -> Plan {
    let mut p = Plan::default();
    let panel = panel_frame(m);
    let ps = p.surf(Fill::Panel, 0, panel, false, None);
    let body_y = panel_head(&mut p, f, m, panel, "Settings", ps);
    debug_assert!((body_y - settings_body_y(m)).abs() < 0.5);
    panel_hint(&mut p, f, m, panel, hint(Screen::Settings), ps);
    let t = &m.ty;
    let (lhv, lhl, lhc) = (line_height(t.value), line_height(t.label), line_height(t.caption));
    let mark = m.icon(28.0);
    let chev = m.icon(28.0);
    for (i, row) in Row::ALL.iter().enumerate() {
        let focus = i == app.settings_row;
        let rect = settings_row_frame(m, body_y, i);
        let rs = p.surf(Fill::SettingsRow, i, rect, focus, Some(ps));
        let name_w = width(f, row.name(), t.label);
        let nx = rect.x + ROW_INSET;
        p.text(Lay::Body, row.name(), t.label, Ink::Ink, nx, rect.y + ((lhv + 24.0 - lhl) / 2.0).round(), name_w + 4.0, Align::Left, Some(rs));
        let after = nx + name_w + 14.0;
        if focus {
            let hint_w = width(f, "Hold OK", t.caption);
            let pad = 14.0;
            let ph = (lhc + 12.0).max(mark as f32 + 12.0);
            let pw = pad + mark as f32 + 8.0 + hint_w + pad;
            let py = rect.y + ((rect.h - ph) / 2.0).round();
            p.mark(Lay::Body, Form::Round { w: pw as u32, h: ph as u32, radius: (ph / 2.0) as u32 }, after, py, Ink::Wash, Some(rs));
            p.mark(Lay::Body, Form::Icon(Icon::Info, mark), after + pad, py + ((ph - mark as f32) / 2.0).round(), Ink::Ink, Some(rs));
            p.text(Lay::Body, "Hold OK", t.caption, Ink::Ink, after + pad + mark as f32 + 8.0, py + ((ph - lhc) / 2.0).round(), hint_w + 4.0, Align::Left, Some(rs));
        } else {
            p.mark(Lay::Body, Form::Icon(Icon::Info, mark), after, rect.y + ((rect.h - mark as f32) / 2.0).round(), Ink::Ink2, Some(rs));
        }
        let value = row.value(&app.prefs);
        let right = rect.right() - ROW_INSET - chev as f32 - 10.0;
        let vw = width(f, &value, t.value);
        p.text(Lay::Body, value, t.value, Ink::Ink, right - vw - 4.0, rect.y + ((rect.h - lhv) / 2.0).round(), vw + 4.0, Align::Right, Some(rs));
        if focus {
            let cy = rect.y + ((rect.h - chev as f32) / 2.0).round();
            p.mark(Lay::Body, Form::Icon(Icon::Back, chev), right - vw - 10.0 - chev as f32, cy, Ink::Ink, Some(rs));
            p.mark(Lay::Body, Form::Icon(Icon::ChevronRight, chev), rect.right() - ROW_INSET - chev as f32, cy, Ink::Ink, Some(rs));
        }
    }
    if app.help_open {
        let (card, body) = help_card(f, m, app.settings_row);
        let cs = p.surf(Fill::Help, app.settings_row, card, false, Some(ps));
        let side = m.icon(28.0);
        p.mark(Lay::Help, Form::Icon(Icon::Info, side), card.x + ROW_INSET, card.y + ROW_INSET + ((lhl - side as f32) / 2.0).round(), Ink::Ink2, Some(cs));
        p.text(Lay::Help, Row::ALL[app.settings_row].name(), t.label, Ink::Ink, card.x + ROW_INSET + side as f32 + 12.0, card.y + ROW_INSET, card.w - 2.0 * ROW_INSET - side as f32 - 12.0, Align::Left, Some(cs));
        for (i, l) in body.into_iter().enumerate() {
            p.text(Lay::Help, l, t.label, Ink::Ink, card.x + ROW_INSET, card.y + ROW_INSET + lhl + 10.0 + i as f32 * lhl, card.w - 2.0 * ROW_INSET, Align::Left, Some(cs));
        }
    }
    p
}

// ---- library ---------------------------------------------------------------------------------------------------------------------

pub fn grid_top(m: &Metrics) -> f32 {
    SAFE_Y + m.head_side() + 34.0
}

/// The tile's frame in the grid, for the index as shown (the grid scrolls by whole rows).
pub fn tile_frame(m: &Metrics, index: usize, first_row: usize) -> Option<Frame> {
    let row = index / LIBRARY_COLUMNS;
    if row < first_row || row >= first_row + GRID_ROWS {
        return None;
    }
    let col = index % LIBRARY_COLUMNS;
    Some(Frame::new(SAFE_X + col as f32 * (TILE_W + TILE_GAP), grid_top(m) + (row - first_row) as f32 * (TILE_H + TILE_GAP), TILE_W, TILE_H))
}

/// The height of a tile's title strip: two lines of the tile type and the room around them.
pub fn strip_height(m: &Metrics) -> f32 {
    2.0 * line_height(m.ty.tile) + 24.0
}

pub fn header_button(m: &Metrics) -> Frame {
    let s = m.head_side();
    Frame::new(W - SAFE_X - s, SAFE_Y, s, s)
}

fn plan_library(app: &App, m: &Metrics, f: &mut Fonts) -> Plan {
    let mut p = Plan::default();
    let t = &m.ty;
    let side = m.head_side();
    let lht = line_height(t.title);
    // the header: the logo, the name, the account, and the settings button the focus can reach by going up from the first row
    p.mark(Lay::Head, Form::Disc(side as u32), SAFE_X, SAFE_Y, Ink::Accent, None);
    let ic = m.icon(36.0);
    p.mark(Lay::Head, Form::Icon(Icon::Play, ic), SAFE_X + ((side - ic as f32) / 2.0).round(), SAFE_Y + ((side - ic as f32) / 2.0).round(), Ink::OnAccent, None);
    p.text(Lay::Head, "GeForce NOW", t.title, Ink::Ink, SAFE_X + side + 24.0, SAFE_Y + ((side - lht) / 2.0).round(), 700.0 * m.chrome, Align::Left, None);
    let hb = header_button(m);
    let hs = p.surf(Fill::HeaderButton, 0, hb, app.header_focus, None);
    let sl = m.icon(36.0);
    p.mark(Lay::Head, Form::Icon(Icon::Sliders, sl), hb.x + ((hb.w - sl as f32) / 2.0).round(), hb.y + ((hb.h - sl as f32) / 2.0).round(), Ink::Ink, Some(hs));
    let account = account_text(app);
    if !account.is_empty() {
        let aw = 700.0 * m.chrome;
        let lha = line_height(t.caption);
        let text = ellipsise(f, &account, t.caption, aw);
        p.text(Lay::Head, text, t.caption, Ink::Ink2, hb.x - 24.0 - aw, SAFE_Y + ((side - lha) / 2.0).round(), aw, Align::Right, None);
    }
    // the tiles
    let lht2 = line_height(t.tile);
    let strip = strip_height(m);
    let first = app.first_row;
    let mut shown = 0;
    for (slot, g) in app.games.iter().skip(first * LIBRARY_COLUMNS).take(LIBRARY_COLUMNS * GRID_ROWS).enumerate() {
        let index = first * LIBRARY_COLUMNS + slot;
        let Some(fr) = tile_frame(m, index, first) else { continue };
        shown += 1;
        let focus = index == app.focus && !app.header_focus;
        let ts = p.surf(Fill::Tile, slot, fr, focus, None);
        let ls = lines(f, &g.title, t.tile, TILE_W - 2.0 * TILE_PAD, 2);
        let block = ls.len() as f32 * lht2;
        let top = fr.y + fr.h - strip + ((strip - block) / 2.0).round();
        for (i, l) in ls.into_iter().enumerate() {
            p.text(Lay::Tiles, l, t.tile, Ink::OnArt, fr.x + TILE_PAD, top + i as f32 * lht2, TILE_W - 2.0 * TILE_PAD, Align::Centre, Some(ts));
        }
        if focus {
            p.surf(Fill::Ring, 0, fr, true, None);
        }
    }
    if shown == 0 {
        let words = if app.loading_library() { "Loading your library" } else { "Nothing in your library yet" };
        let lh = line_height(t.value);
        p.text(Lay::Tiles, words, t.value, Ink::Ink2, SAFE_X, grid_top(m) + 120.0, W - 2.0 * SAFE_X, Align::Centre, None);
        let _ = lh;
    }
    p
}

// ---- over the stream -------------------------------------------------------------------------------------------------------------

pub fn overlay_row_h(m: &Metrics) -> f32 {
    (84.0f32).max(line_height(m.ty.value) + 28.0)
}

/// The overlay's card: centred, as high as its rows and hint ask.
pub fn overlay_frame(f: &mut Fonts, m: &Metrics) -> Frame {
    let w = (600.0 * (1.0 + (m.s - 1.0) * 0.6)).round();
    let rows_top = 36.0 + line_height(m.ty.title) + 22.0;
    let n = OVERLAY_ROWS.len() as f32;
    let rows_end = rows_top + n * overlay_row_h(m) + (n - 1.0) * 12.0;
    let hint_h = lines(f, hint(Screen::Overlay), m.ty.small, w - 2.0 * PAD, 0).len() as f32 * line_height(m.ty.small);
    let h = (rows_end + 28.0 + hint_h + 36.0).round();
    Frame::new(((W - w) / 2.0).round(), ((H - h) / 2.0).round(), w, h)
}

pub fn overlay_row_frame(f: &mut Fonts, m: &Metrics, i: usize) -> Frame {
    let o = overlay_frame(f, m);
    let top = 36.0 + line_height(m.ty.title) + 22.0;
    Frame::new(o.x + PAD, o.y + top + i as f32 * (overlay_row_h(m) + 12.0), o.w - 2.0 * PAD, overlay_row_h(m))
}

fn plan_overlay(app: &App, m: &Metrics, f: &mut Fonts) -> Plan {
    let mut p = Plan::default();
    let t = &m.ty;
    let o = overlay_frame(f, m);
    let os = p.surf(Fill::Overlay, 0, o, false, None);
    p.text(Lay::Overlay, "Game", t.title, Ink::Ink, o.x + PAD, o.y + 36.0, o.w - 2.0 * PAD, Align::Left, Some(os));
    let lhv = line_height(t.value);
    for (i, name) in OVERLAY_ROWS.iter().enumerate() {
        let r = overlay_row_frame(f, m, i);
        let rs = p.surf(Fill::OverlayRow, i, r, i == app.overlay_row, Some(os));
        let ink = if i == OVERLAY_ROWS.len() - 1 { Ink::Danger } else { Ink::Ink };
        p.text(Lay::Overlay, *name, t.value, ink, r.x + ROW_INSET, r.y + ((r.h - lhv) / 2.0).round(), r.w - 2.0 * ROW_INSET, Align::Left, Some(rs));
        if *name == "Stats" {
            p.text(Lay::Overlay, if app.prefs.show_stats { "On" } else { "Off" }, t.value, Ink::Ink2, r.x + ROW_INSET, r.y + ((r.h - lhv) / 2.0).round(), r.w - 2.0 * ROW_INSET, Align::Right, Some(rs));
        }
    }
    let ls = lines(f, hint(Screen::Overlay), t.small, o.w - 2.0 * PAD, 0);
    let lhs = line_height(t.small);
    let top = o.bottom() - 36.0 - ls.len() as f32 * lhs;
    for (i, l) in ls.into_iter().enumerate() {
        p.text(Lay::Overlay, l, t.small, Ink::Ink2, o.x + PAD, top + i as f32 * lhs, o.w - 2.0 * PAD, Align::Left, Some(os));
    }
    p
}

/// The status line's pill: bottom left, wide enough for the longest line the stream puts in it.
pub fn status_frame(f: &mut Fonts, m: &Metrics) -> Frame {
    let w = (width(f, "120 fps · 120 Mbit/s · 99999 dropped · no input yet", m.ty.value) + 48.0).ceil().max(760.0).min(W - 2.0 * SAFE_X);
    let h = line_height(m.ty.value) + 16.0;
    Frame::new(SAFE_X, H - SAFE_Y - h, w, h)
}

fn plan_status(app: &App, m: &Metrics, f: &mut Fonts) -> Plan {
    let mut p = Plan::default();
    let fr = status_frame(f, m);
    let on = app.screen == Screen::Stream && (app.chrome_visible() || app.prefs.show_stats);
    if on {
        let ss = p.surf(Fill::Status, 0, fr, false, None);
        let text = ellipsise(f, &stats_line(app), m.ty.value, fr.w - 48.0);
        let lh = line_height(m.ty.value);
        p.text(Lay::Status, text, m.ty.value, Ink::OnArt, fr.x + 24.0, fr.y + ((fr.h - lh) / 2.0).round(), fr.w - 48.0, Align::Left, Some(ss));
    }
    p
}

/// Everything the current screen puts up.
pub fn plan(app: &App, m: &Metrics, f: &mut Fonts) -> Plan {
    match app.screen {
        Screen::Library => plan_library(app, m, f),
        Screen::Settings => plan_settings(app, m, f),
        Screen::Stream => plan_status(app, m, f),
        Screen::Overlay => plan_overlay(app, m, f),
        _ => plan_panel(app, m, f),
    }
}

// ---- the checks ------------------------------------------------------------------------------------------------------------------

/// A text's ink box: its line's width and the font's size, in the line box.
fn ink_box(f: &mut Fonts, t: &Txt) -> Frame {
    let w = width(f, &t.text, t.ty);
    let lh = line_height(t.ty);
    Frame::new(start_x(t.x, t.w, w, t.align), t.y + ((lh - t.ty.px as f32) / 2.0).round(), w, t.ty.px as f32)
}

fn grown(r: Frame, by: f32) -> Frame {
    Frame::new(r.x - by, r.y - by, r.w + 2.0 * by, r.h + 2.0 * by)
}

fn meets(a: Frame, b: Frame) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

fn contains(outer: Frame, inner: Frame) -> bool {
    inner.x >= outer.x - 0.01 && inner.y >= outer.y - 0.01 && inner.right() <= outer.right() + 0.01 && inner.bottom() <= outer.bottom() + 0.01
}

/// What is wrong with a plan, by Lucent's rules for a 1920x1080 screen (the rest-unit thresholds of the checker doubled): two texts
/// closer than 6, a text within 12 of the screen's edge or within 16 by 6 of the plate it is on or outside it, a text touching a mark
/// (within 4), two surfaces that overlap without one holding the other. Empty means clean.
pub fn violations(plan: &Plan, f: &mut Fonts) -> Vec<String> {
    let mut out = Vec::new();
    let boxes: Vec<Frame> = plan.texts.iter().map(|t| ink_box(f, t)).collect();
    // an explanation card is opaque: what it covers is not on the screen, and is not compared with what is written on the card
    let cards: Vec<Frame> = plan.surface(Fill::Help).map(|s| s.rect).collect();
    let on_card = |on: Option<usize>| on.is_some_and(|s| plan.surfs[s].fill == Fill::Help);
    let hidden = |on: Option<usize>, r: Frame| !on_card(on) && cards.iter().any(|c| meets(*c, r));
    let apart = |a: (Option<usize>, Frame), b: (Option<usize>, Frame)| (on_card(a.0) && hidden(b.0, b.1)) || (on_card(b.0) && hidden(a.0, a.1));
    for (i, (t, b)) in plan.texts.iter().zip(&boxes).enumerate() {
        if !contains(Frame::new(12.0, 12.0, W - 24.0, H - 24.0), *b) {
            out.push(format!("text {:?} is within 12 of the screen's edge: {b:?}", t.text));
        }
        if let Some(s) = t.on {
            let plate = plan.surfs[s].rect;
            let inner = Frame::new(plate.x + 16.0, plate.y + 6.0, plate.w - 32.0, plate.h - 12.0);
            if !contains(inner, *b) {
                out.push(format!("text {:?} {b:?} is not 16x6 inside its {:?} {plate:?}", t.text, plan.surfs[s].fill));
            }
        }
        for (j, (u, c)) in plan.texts.iter().zip(&boxes).enumerate() {
            if j > i && meets(grown(*b, 3.0), *c) && !apart((t.on, *b), (u.on, *c)) {
                out.push(format!("texts {:?} and {:?} are closer than 6: {b:?} {c:?}", t.text, u.text));
            }
        }
        for mk in &plan.marks {
            if let Form::Icon(_, side) = mk.form {
                let mb = Frame::new(mk.x, mk.y, side as f32, side as f32);
                if meets(grown(*b, 2.0), mb) && !apart((t.on, *b), (mk.on, mb)) {
                    out.push(format!("text {:?} {b:?} touches an icon at {mb:?}", t.text));
                }
            }
        }
    }
    for (i, a) in plan.surfs.iter().enumerate() {
        for (j, b) in plan.surfs.iter().enumerate() {
            if j <= i || !meets(a.rect, b.rect) {
                continue;
            }
            let held = contains(a.rect, b.rect) || contains(b.rect, a.rect);
            // the ring is the tile's own outline and the help card is an overlay over the rows it explains: both are meant to cover
            let meant = matches!((a.fill, b.fill), (Fill::Ring, _) | (_, Fill::Ring) | (Fill::Help, _) | (_, Fill::Help));
            if !held && !meant {
                out.push(format!("surfaces {:?} {:?} and {:?} {:?} overlap without one holding the other", a.fill, a.rect, b.fill, b.rect));
            }
        }
    }
    for s in &plan.surfs {
        if s.rect.x < 0.0 || s.rect.y < 0.0 || s.rect.right() > W || s.rect.bottom() > H {
            out.push(format!("surface {:?} {:?} leaves the screen", s.fill, s.rect));
        }
        if let Some(parent) = s.parent {
            if !contains(plan.surfs[parent].rect, s.rect) {
                out.push(format!("surface {:?} {:?} is not inside its parent {:?}", s.fill, s.rect, plan.surfs[parent].rect));
            }
        }
    }
    out
}

/// WCAG relative luminance of straight sRGB bytes.
pub fn luminance(c: [f32; 3]) -> f32 {
    let lin = |v: f32| {
        let v = v / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

pub fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// A premultiplied colour over an opaque colour, straight RGB out.
pub fn over(fg: compositor_protocol::Rgba, bg: [f32; 3]) -> [f32; 3] {
    let a = f32::from(fg.3) / 255.0;
    [f32::from(fg.0) + bg[0] * (1.0 - a), f32::from(fg.1) + bg[1] * (1.0 - a), f32::from(fg.2) + bg[2] * (1.0 - a)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfn::app::{Game, Stats};
    use crate::look::{Mode, palette_for};
    use serde_json::json;

    fn games(n: usize) -> Vec<Game> {
        (0..n)
            .map(|i| Game {
                id: format!("g{i}"),
                variant_id: format!("v{i}"),
                title: ["Portal 2", "The Witcher 3: Wild Hunt - Complete Edition", "Counter-Strike 2", "Cyberpunk 2077", "Supercalifragilisticexpialidocious Adventures", "Hades"][i % 6].into(),
                image_url: format!("https://x/{i}.jpg"),
                store: "STEAM".into(),
                in_library: true,
            })
            .collect()
    }

    /// Every screen the app has, in a state that fills it: the longest words the rules produce.
    pub fn every_screen() -> Vec<(&'static str, App)> {
        let mut out = Vec::new();
        let base = || {
            let mut a = App::new();
            a.signed_in = true;
            a.user_name = "Nitin Phil".into();
            a.tier = "ULTIMATE".into();
            a.games = games(40);
            a.runtime_capabilities = json!({"nativeHdrSupported": true});
            a
        };
        let mut a = App::new();
        a.screen = Screen::Starting;
        out.push(("starting", a));
        let mut a = base();
        a.screen = Screen::SignIn;
        a.sign_in.user_code = "ABCD-EFGH".into();
        a.sign_in.url = "https://www.nvidia.com/en-us/geforce-now/activate".into();
        a.sign_in.status = "pending".into();
        out.push(("signin", a));
        let mut a = base();
        a.screen = Screen::Library;
        out.push(("library", a));
        let mut a = base();
        a.screen = Screen::Library;
        a.focus = 17;
        a.first_row = 2;
        out.push(("library-scrolled", a));
        let mut a = base();
        a.screen = Screen::Library;
        a.header_focus = true;
        out.push(("library-header", a));
        let mut a = base();
        a.screen = Screen::Library;
        a.games.clear();
        out.push(("library-empty", a));
        for (i, name) in ["settings", "settings-hdr", "settings-bitrate", "settings-deadzone", "settings-stats"].into_iter().enumerate() {
            let mut a = base();
            a.screen = Screen::Settings;
            a.settings_row = i;
            out.push((name, a));
        }
        for i in [0usize, 2, 4] {
            let mut a = base();
            a.screen = Screen::Settings;
            a.settings_row = i;
            a.help_open = true;
            out.push((["settings-help-stream", "", "settings-help-bitrate", "", "settings-help-stats"][i], a));
        }
        let mut a = base();
        a.screen = Screen::Detail;
        a.detail = Some(games(2)[1].clone());
        a.decision = "ownership_required".into();
        out.push(("detail-blocked", a));
        let mut a = base();
        a.screen = Screen::Detail;
        a.detail = Some(games(2)[1].clone());
        a.decision = "ready".into();
        out.push(("detail-ready", a));
        let mut a = base();
        a.screen = Screen::Launching;
        a.detail = Some(games(2)[1].clone());
        out.push(("launching", a));
        let mut a = base();
        a.screen = Screen::Stream;
        a.now_ms = 1000;
        a.chrome_until_ms = 5000;
        a.stats = Stats { status: "streaming".into(), fps: 119.6, bitrate_mbps: 78.4, input_ready: false, queue_drops: 12345, ..Default::default() };
        out.push(("stream", a));
        let mut a = base();
        a.screen = Screen::Overlay;
        a.prefs.show_stats = true;
        out.push(("overlay", a));
        let mut a = base();
        a.screen = Screen::Error;
        a.error = "The session could not be started because the service says there is no seat for this game right now (upstream)".into();
        out.push(("error", a));
        out
    }

    #[test]
    fn at_100_percent_the_numbers_are_the_ones_the_screens_always_had() {
        let m = Metrics::new(1.0);
        assert_eq!((m.ty.display.px, m.ty.value.px, m.ty.tile.px, m.ty.small.px), (56, 28, 24, 22));
        assert_eq!(panel_frame(&m), Frame::new(360.0, 150.0, 1200.0, 780.0));
        assert_eq!(settings_body_y(&m), 177.0, "the body starts 46 under the rule, which is 8 under the 79-high display line");
        assert_eq!(settings_row_h(&m), 64.0);
        let mut f = Fonts::new();
        assert_eq!(button_size(&mut f, &m), (260.0, 68.0));
        assert_eq!(grid_top(&m), 160.0);
        let t = tile_frame(&m, 7, 0).unwrap();
        assert_eq!((t.x, t.y), (SAFE_X + TILE_W + TILE_GAP, 160.0 + TILE_H + TILE_GAP));
        assert!(tile_frame(&m, 12, 0).is_none());
    }

    #[test]
    fn the_grid_fits_under_the_header_at_every_scale() {
        for s in [1.0, 1.5, 2.0] {
            let m = Metrics::new(s);
            let last = tile_frame(&m, LIBRARY_COLUMNS * GRID_ROWS - 1, 0).unwrap();
            assert!(last.bottom() <= H - SAFE_Y && last.right() <= W - SAFE_X, "{s}: {last:?}");
            assert!(grid_top(&m) >= SAFE_Y + m.head_side() + 24.0);
        }
    }

    #[test]
    fn a_title_that_cannot_hold_the_display_size_wraps_at_the_title_size() {
        let mut f = Fonts::new();
        let m = Metrics::new(1.0);
        let mut p = Plan::default();
        let panel = panel_frame(&m);
        let ps = p.surf(Fill::Panel, 0, panel, false, None);
        panel_head(&mut p, &mut f, &m, panel, "Portal 2", ps);
        assert_eq!(p.texts.len(), 1);
        assert_eq!(p.texts[0].ty, m.ty.display);
        let mut q = Plan::default();
        let qs = q.surf(Fill::Panel, 0, panel, false, None);
        panel_head(&mut q, &mut f, &m, panel, "The Witcher 3: Wild Hunt - Complete Edition - Game of the Year - The Witcher 3: Wild Hunt - Complete Edition - Game of the Year", qs);
        assert_eq!(q.texts.len(), 2);
        assert_eq!(q.texts[0].ty, m.ty.title);
        assert!(q.texts[1].text.ends_with('…'));
    }

    /// Lucent: no overlapping text, checked at 100 % and 200 % text. Every screen, in the state that fills it most.
    #[test]
    fn no_screen_has_overlapping_or_clipped_text_at_100_and_200_percent() {
        let mut f = Fonts::new();
        let mut checked = 0;
        for s in [1.0, 2.0] {
            let m = Metrics::new(s);
            for (name, app) in every_screen() {
                let p = plan(&app, &m, &mut f);
                let v = violations(&p, &mut f);
                assert!(v.is_empty(), "{name} at {}%: {:#?}", s * 100.0, v);
                checked += 1;
            }
        }
        assert!(checked >= 40);
    }

    #[test]
    fn the_checker_catches_what_it_is_for() {
        let mut f = Fonts::new();
        let m = Metrics::new(1.0);
        let mut p = Plan::default();
        let s = p.surf(Fill::Panel, 0, Frame::new(100.0, 100.0, 400.0, 200.0), false, None);
        p.text(Lay::Body, "Touching", m.ty.value, Ink::Ink, 110.0, 120.0, 300.0, Align::Left, Some(s));
        assert!(!violations(&p, &mut f).is_empty(), "10 from the plate's edge is too close");
        let mut p = Plan::default();
        let s = p.surf(Fill::Panel, 0, Frame::new(100.0, 100.0, 400.0, 200.0), false, None);
        p.text(Lay::Body, "One", m.ty.value, Ink::Ink, 130.0, 120.0, 300.0, Align::Left, Some(s));
        p.text(Lay::Body, "Two", m.ty.value, Ink::Ink, 130.0, 130.0, 300.0, Align::Left, Some(s));
        assert!(violations(&p, &mut f).iter().any(|v| v.contains("closer than 6")));
        let mut p = Plan::default();
        p.surf(Fill::Tile, 0, Frame::new(0.0, 0.0, 100.0, 100.0), false, None);
        p.surf(Fill::Tile, 1, Frame::new(50.0, 50.0, 100.0, 100.0), false, None);
        assert!(violations(&p, &mut f).iter().any(|v| v.contains("overlap")));
        let mut p = Plan::default();
        p.text(Lay::Body, "Edge", m.ty.value, Ink::Ink, 2.0, 500.0, 200.0, Align::Left, None);
        assert!(violations(&p, &mut f).iter().any(|v| v.contains("edge")));
    }

    fn straight(c: compositor_protocol::Rgba) -> compositor_protocol::Rgba {
        c
    }

    fn rgb(c: compositor_protocol::Rgba) -> [f32; 3] {
        let a = f32::from(c.3).max(1.0) / 255.0;
        [f32::from(c.0) / a, f32::from(c.1) / a, f32::from(c.2) / a]
    }

    /// The colour an ink resolves to in a mode, as a premultiplied colour.
    fn ink_colour(p: &crate::look::Palette, ink: Ink) -> compositor_protocol::Rgba {
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

    /// Text is read at 4.5:1 on what it is really drawn on (3:1 from 48 px, which is Lucent's 24 units at 1080p): the plate's tint over
    /// the wallpaper at its darkest and at its brightest, the title strip over a white picture.
    #[test]
    fn every_text_reads_at_4_5_to_1_on_its_plate_in_both_modes_over_the_worst_wallpaper() {
        let mut f = Fonts::new();
        for (mode, probes) in [(Mode::Dark, [[10.0f32, 8.0, 20.0], [70.0, 60.0, 110.0]]), (Mode::Light, [[200.0, 190.0, 215.0], [255.0, 250.0, 246.0]])] {
            let pal = palette_for(mode);
            for s in [1.0, 2.0] {
                let m = Metrics::new(s);
                for (name, app) in every_screen() {
                    let plan = plan(&app, &m, &mut f);
                    for t in &plan.texts {
                        let fill = t.on.map(|i| plan.surfs[i].fill);
                        let need = if t.ty.px >= 48 { 3.0 } else { 4.5 };
                        for wall in probes {
                            let bg = match fill {
                                Some(Fill::Tile) => over(look::black(0.0), [255.0, 255.0, 255.0]),
                                _ => wall,
                            };
                            let plate = match fill {
                                None => bg,
                                Some(Fill::OverlayRow) => over(if plan.surfs[t.on.unwrap()].focus { pal.glass_focus } else { pal.glass }, over(pal.scrim_hide, bg)),
                                Some(Fill::Panel) | Some(Fill::SettingsRow) | Some(Fill::HeaderButton) | Some(Fill::Button) => {
                                    let focus = plan.surfs[t.on.unwrap()].focus || fill == Some(Fill::Button);
                                    let mut base = over(if focus { pal.glass_focus } else { pal.glass }, bg);
                                    // a row or a button stands on the panel's glass, which stands on the wallpaper
                                    if matches!(fill, Some(Fill::SettingsRow) | Some(Fill::Button)) {
                                        base = over(if focus { pal.glass_focus } else { pal.glass }, over(pal.glass, bg));
                                    }
                                    base
                                }
                                // the chip over a picture as bright as can be
                                Some(Fill::Status) => over(look::hex("#db120e1a"), [255.0, 255.0, 255.0]),
                                Some(Fill::Overlay) | Some(Fill::Help) => over(pal.scrim_hide, bg),
                                Some(Fill::Tile) => {
                                    // the strip: its colour at .88 over a picture as bright as can be
                                    over(look::hex("#E0120E1A"), [255.0, 255.0, 255.0])
                                }
                                other => panic!("no plate for {other:?}"),
                            };
                            let fg = over(straight(ink_colour(&pal, t.ink)), plate);
                            let ratio = contrast(fg, plate);
                            assert!(ratio >= need, "{name} {mode:?} {}%: {:?} ({:?}) is {ratio:.2}:1 on {fill:?} over {wall:?}, wants {need}", s * 100.0, t.text, t.ink);
                        }
                    }
                }
            }
        }
        let _ = rgb;
    }

    #[test]
    fn the_scrims_that_hide_the_page_are_at_least_97_percent() {
        for mode in [Mode::Dark, Mode::Light] {
            assert!(palette_for(mode).scrim_hide.3 >= 247, "{mode:?}");
        }
    }

    #[test]
    fn the_settings_rows_carry_their_name_a_help_mark_and_their_value_and_the_focused_one_says_how_to_open_it() {
        let mut f = Fonts::new();
        let m = Metrics::new(1.0);
        let mut app = App::new();
        app.screen = Screen::Settings;
        app.settings_row = 2;
        let p = plan(&app, &m, &mut f);
        let names: Vec<&str> = p.texts.iter().filter(|t| matches!(t.ink, Ink::Ink) && t.ty == m.ty.label).map(|t| t.text.as_str()).collect();
        assert_eq!(names, ["Stream", "HDR", "Bitrate cap", "Stick dead zone", "Show stats"]);
        let values: Vec<&str> = p.texts.iter().filter(|t| t.ty == m.ty.value && t.align == Align::Right).map(|t| t.text.as_str()).collect();
        assert_eq!(values, ["4K 120", "On", "80 Mbit/s", "8%", "Off"]);
        assert_eq!(p.texts.iter().filter(|t| t.text == "Hold OK").count(), 1, "only the focused row says how to open the help");
        let infos = p.marks.iter().filter(|mk| matches!(mk.form, Form::Icon(Icon::Info, _))).count();
        assert_eq!(infos, 5, "every row has a mark, the focused one's inside its pill");
        let chev = p.marks.iter().filter(|mk| matches!(mk.form, Form::Icon(Icon::Back | Icon::ChevronRight, _))).count();
        assert_eq!(chev, 2, "the focused row shows that Left and Right change it");
        assert_eq!(p.surface(Fill::SettingsRow).filter(|s| s.focus).count(), 1);
        assert!(p.surface(Fill::Help).next().is_none());
        app.help_open = true;
        let p = plan(&app, &m, &mut f);
        let card = p.surface(Fill::Help).next().unwrap();
        let row = settings_row_frame(&m, settings_body_y(&m), 2);
        assert!(!meets(card.rect, row), "the explanation never covers the row it explains: {:?} {:?}", card.rect, row);
    }

    #[test]
    fn the_library_says_what_it_is_waiting_for() {
        let mut f = Fonts::new();
        let m = Metrics::new(1.0);
        let mut app = App::new();
        app.screen = Screen::Library;
        let p = plan(&app, &m, &mut f);
        assert!(p.texts.iter().any(|t| t.text == "Nothing in your library yet"));
        app.sent("1", "catalog.library.list");
        let p = plan(&app, &m, &mut f);
        assert!(p.texts.iter().any(|t| t.text == "Loading your library"));
    }

    #[test]
    fn the_words_of_a_detail_card_say_little_and_the_status_line_is_short() {
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
}
