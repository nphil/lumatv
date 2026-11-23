//! The basic task manager: what the compositor draws by itself when the UI process cannot. It shows the running apps as cards, the
//! stats, and the recovery actions (close every app, restart LumaTV, LG's Home, restart the TV), from the palette the UI last sent.
//!
//! This module only decides and draws. [`Screen::key`] is a state machine with no clock; [`Screen::render`] paints the parts of the screen
//! that changed into premultiplied RGBA [`Band`]s; the compositor places the bands, draws a flat dim of the palette's panel colour behind
//! them and draws the cards' pictures itself in the rectangles [`Screen::slots`] gives (a hung UI cannot hand it any other way to a
//! window's pixels).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::font::{Canvas, Rgb, Typeface, mix, opaque, with_alpha};
use super::{Action, Palette, Stats};
use crate::Rect;
use crate::switcher::Picture;

const SCREEN_W: i32 = 1920;
const SCREEN_H: i32 = 1080;
/// Lucent's TV safe area: nothing important outside x 96, y 54.
const SAFE_X: i32 = 96;
const SAFE_Y: i32 = 54;
const SAFE_W: i32 = SCREEN_W - 2 * SAFE_X;
/// Five cards and their gaps span exactly this width; the title, cards and gauges share its left edge so that the
/// focus ring of the outer cards still lies inside the safe area.
const CONTENT_X: i32 = SAFE_X + 20;
const CONTENT_W: i32 = SAFE_W - 40;

const TITLE_PX: u32 = 40;
const BODY_PX: u32 = 28;
const LABEL_PX: u32 = 26;
const CAPTION_PX: u32 = 24;

const CARD_W: i32 = 312;
const CARD_H: i32 = 175;
const CARD_GAP: i32 = 32;
const MAX_SLOTS: usize = 5;
/// The focus ring around a card, outside its rectangle.
const RING: i32 = 5;
/// Corner radius of the cards; the compositor rounds the pictures it draws in the card slots by this much.
pub const CARD_RADIUS: f32 = 20.0;
const CONTROL_RADIUS: f32 = 14.0;
const SHEET_RADIUS: f32 = 28.0;

const WHY_Y: i32 = 112;
const SLOT_Y: i32 = 172;
const LABEL_Y: i32 = SLOT_Y + CARD_H + 16;
const STATE_Y: i32 = LABEL_Y + 36;
const PAGE_Y: i32 = STATE_Y + 42;
const HINT_Y: i32 = PAGE_Y + 52;
const NOTICE_Y: i32 = HINT_Y + 38;
/// Title, cards, labels, hint and notice; the margin on both sides holds the focus ring.
const MAIN_BAND: Rect = Rect::new(CONTENT_X - RING, SAFE_Y, CONTENT_W + 2 * RING, NOTICE_Y + 36 - SAFE_Y);

const STATS_Y: i32 = 632;
const GAUGE_GAP: i32 = 24;
const BAR_H: i32 = 12;
const STATS_BAND: Rect = Rect::new(CONTENT_X, STATS_Y, CONTENT_W, 100);

const ACTIONS_Y: i32 = 850;
const BUTTON_H: i32 = 72;
const BUTTON_GAP: i32 = 24;
const ACTIONS_H: i32 = BUTTON_H + 8 + 33;

const SHEET_W: i32 = 900;
const SHEET_H: i32 = 420;
const WORKING_H: i32 = 240;
const SHEET_PAD: i32 = 56;
/// Room round the sheet for its shadow.
const SHADOW: i32 = 48;

const CAPTION_GAP: i32 = 8;
const HINT_KEY_GAP: i32 = 12;
const HINT_GAP: i32 = 44;
const BODY_LINE: i32 = 40;
const SHEET_BUTTON_W: i32 = 300;

const REFRESHER_CAPTION: &str = "Refresher running";
const REFRESHER_NOTICE: &str = "Restart TV waits for the panel's pixel refresher to finish.";

/// A running app, as the compositor lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub picture: Picture,
    pub mark: Mark,
    /// The process does not answer.
    pub hung: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Nothing,
    /// Asked to quit and not gone yet.
    Quitting,
    /// Did not quit in time.
    Stuck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
    Back,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Nothing,
    /// Something to draw: call [`Screen::render`].
    Redraw,
    Close,
    Activate(String),
    Quit { id: String, force: bool },
    QuitAll,
    System(Action),
}

/// What [`Screen::render`] draws from.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub items: &'a [Item],
    pub stats: Option<&'a Stats>,
    pub palette: &'a Palette,
    /// Why the basic screen is up, in a sentence.
    pub why: &'a str,
    /// A line from the compositor about what just failed, in the danger colour.
    pub notice: Option<&'a str>,
    /// The panel's pixel refresher runs: Restart TV is not offered.
    pub refresher: bool,
    /// An action that was confirmed and is under way.
    pub working: Option<Action>,
}

/// A part of the screen, premultiplied RGBA8, rows top first, `rect.w * rect.h * 4` bytes. A band with an empty rectangle takes that
/// band off the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Band {
    /// 0: title, cards, labels, hint, notice and the confirm sheet. 1: the stats. 2: the actions.
    pub key: u8,
    pub rect: Rect,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    Cards,
    Actions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    CloseAll,
    System(Action),
}

impl Button {
    /// The buttons in the row, left to right; "Close all apps" only when there is an app to close.
    fn count(has_items: bool) -> usize {
        Action::ALL.len() + usize::from(has_items)
    }

    fn at(has_items: bool, i: usize) -> Option<Button> {
        let i = if has_items { i } else { i + 1 };
        match i {
            0 => Some(Button::CloseAll),
            _ => Action::ALL.get(i - 1).map(|a| Button::System(*a)),
        }
    }

    fn position(self, has_items: bool) -> usize {
        match self {
            Button::CloseAll => 0,
            Button::System(a) => Action::ALL.iter().position(|b| *b == a).unwrap_or(0) + usize::from(has_items),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Button::CloseAll => "Close all apps",
            Button::System(a) => a.label(),
        }
    }

    fn tag(self) -> usize {
        match self {
            Button::CloseAll => 0,
            Button::System(a) => 1 + Action::ALL.iter().position(|b| *b == a).unwrap_or(0),
        }
    }
}

/// The confirmation of a system action; `confirm` is on the action's own button rather than on Cancel.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Sheet {
    action: Action,
    confirm: bool,
}

pub struct Screen {
    face: Typeface,
    zone: Zone,
    /// The focused card.
    card: usize,
    /// The first card of the visible window.
    first: usize,
    button: Button,
    sheet: Option<Sheet>,
    notice: Option<&'static str>,
    working: Option<Action>,
    /// What each band was last drawn from, as a hash; `None` while the band is not on the screen.
    drawn: [Option<u64>; 3],
}

impl Default for Screen {
    fn default() -> Screen {
        Screen::new()
    }
}

/// The visible cards: the first of them and how many. The window moves only as far as keeps the focus in it.
fn window(n: usize, focus: usize, first: usize) -> (usize, usize) {
    let count = n.min(MAX_SLOTS);
    if count == 0 {
        return (0, 0);
    }
    let focus = focus.min(n - 1);
    let first = first.min(n - count);
    if focus < first {
        (focus, count)
    } else if focus >= first + count {
        (focus + 1 - count, count)
    } else {
        (first, count)
    }
}

/// Where `count` cards sit: one row, centred in the content width.
fn slot_rects(count: usize) -> impl Iterator<Item = Rect> {
    let count = count as i32;
    let row = count * CARD_W + (count - 1).max(0) * CARD_GAP;
    let x0 = CONTENT_X + (CONTENT_W - row) / 2;
    (0..count).map(move |k| Rect::new(x0 + k * (CARD_W + CARD_GAP), SLOT_Y, CARD_W, CARD_H))
}

fn grow(r: Rect, by: i32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.w + 2 * by, r.h + 2 * by)
}

fn sheet_rect(h: i32) -> Rect {
    Rect::new((SCREEN_W - SHEET_W) / 2, (SCREEN_H - h) / 2, SHEET_W, h)
}

fn sheet_band(h: i32) -> Rect {
    grow(sheet_rect(h), SHADOW)
}

fn fingerprint(f: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut h = DefaultHasher::new();
    f(&mut h);
    h.finish()
}

fn hash_palette(h: &mut DefaultHasher, p: &Palette) {
    [p.panel, p.text, p.muted, p.accent, p.danger].hash(h);
}

/// What each key does, for the line under the cards.
fn hint(zone: Zone, force: bool, has_items: bool) -> Vec<(&'static str, &'static str)> {
    match zone {
        Zone::Cards => vec![("Left/Right", "choose"), ("OK", "open"), ("Up", if force { "force quit" } else { "close" }), ("Down", "more"), ("Back", "leave")],
        Zone::Actions if has_items => vec![("Left/Right", "choose"), ("OK", "select"), ("Up", "apps"), ("Back", "leave")],
        Zone::Actions => vec![("Left/Right", "choose"), ("OK", "select"), ("Back", "leave")],
    }
}

/// The screen's colours, all blended from the five of the palette.
#[derive(Clone, Copy)]
struct Tones {
    panel: Rgb,
    text: Rgb,
    muted: Rgb,
    accent: Rgb,
    danger: Rgb,
    tile: Rgb,
    tile_dim: Rgb,
    button: Rgb,
    sheet: Rgb,
    sheet_button: Rgb,
}

fn luminance(c: Rgb) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

impl Tones {
    fn new(p: &Palette) -> Tones {
        let tile = mix(p.panel, p.text, 0.10);
        Tones {
            panel: p.panel,
            text: p.text,
            muted: p.muted,
            accent: p.accent,
            danger: p.danger,
            tile,
            tile_dim: mix(p.panel, tile, 0.4),
            button: mix(p.panel, p.text, 0.12),
            sheet: mix(p.panel, p.text, 0.07),
            sheet_button: mix(p.panel, p.text, 0.17),
        }
    }

    /// The label colour for a button filled with `bg`: the panel's or the text's, whichever reads better on it.
    fn ink_on(&self, bg: Rgb) -> Rgb {
        if contrast(bg, self.panel) >= contrast(bg, self.text) { self.panel } else { self.text }
    }
}

/// A canvas, the face to draw text with and the colours.
struct Paint<'a> {
    cv: Canvas,
    face: &'a mut Typeface,
    t: Tones,
}

impl<'a> Paint<'a> {
    fn new(rect: Rect, face: &'a mut Typeface, palette: &Palette) -> Paint<'a> {
        Paint { cv: Canvas::new(rect), face, t: Tones::new(palette) }
    }

    fn text(&mut self, s: &str, x: i32, y: i32, px: u32, c: Rgb, max: Option<i32>) -> i32 {
        self.cv.text(self.face, s, (x, y), px, opaque(c), max)
    }

    /// A line centred on `cx`.
    fn centered(&mut self, s: &str, cx: i32, y: i32, px: u32, c: Rgb, max: i32) {
        let shown = self.face.fit(s, px, max);
        let w = self.face.measure(&shown, px);
        self.cv.text(self.face, &shown, (cx - w / 2, y), px, opaque(c), None);
    }

    /// A line centred in `r`, both ways.
    fn label(&mut self, s: &str, r: Rect, px: u32, c: Rgb) {
        let y = r.y + (r.h - self.face.line_height(px)) / 2;
        self.centered(s, r.x + r.w / 2, y, px, c, r.w - 16);
    }

    fn into_band(self, key: u8) -> Band {
        Band { key, rect: self.cv.rect(), rgba: self.cv.into_rgba() }
    }
}

/// One of the stats row's columns: a label, a bar when the number is a share of something, and the value in words.
struct Gauge {
    label: String,
    value: String,
    share: Option<f32>,
    /// Its width, relative to the other columns'.
    weight: f32,
}

fn percent(label: String, pct: Option<u8>) -> Gauge {
    Gauge { label, value: pct.map_or("--".into(), |p| format!("{p}%")), share: Some(f32::from(pct.unwrap_or(0)) / 100.0), weight: 1.0 }
}

/// The columns for a reading; `None`, or a reading with no per-core loads yet, gives dashes. `degree`: the face has the degree sign.
fn gauges(stats: Option<&Stats>, degree: bool) -> Vec<Gauge> {
    let mut g = Vec::new();
    match stats.filter(|s| !s.cpu.is_empty()) {
        Some(s) => g.extend(s.cpu.iter().enumerate().map(|(i, c)| percent(format!("CPU {}", i + 1), Some(*c)))),
        None => g.push(percent("CPU".into(), None)),
    }
    let gb = |kb: u64| kb as f64 / (1024.0 * 1024.0);
    g.push(match stats.filter(|s| s.total_kb > 0) {
        Some(s) => {
            let used = s.total_kb.saturating_sub(s.avail_kb);
            Gauge { label: "Memory".into(), value: format!("{:.1} of {:.1} GB", gb(used), gb(s.total_kb)), share: Some(used as f32 / s.total_kb as f32), weight: 1.5 }
        }
        None => Gauge { label: "Memory".into(), value: "--".into(), share: Some(0.0), weight: 1.5 },
    });
    g.push(Gauge { weight: 0.9, ..percent("GPU".into(), stats.and_then(|s| s.gpu)) });
    let unit = if degree { "\u{b0}C" } else { "C" };
    g.push(Gauge {
        label: "Temperature".into(),
        value: stats.and_then(|s| s.temp_c).map_or("--".into(), |t| format!("{t} {unit}")),
        share: None,
        weight: 1.0,
    });
    g.push(Gauge { label: "Clock".into(), value: stats.and_then(|s| s.mhz).map_or("--".into(), |m| format!("{m} MHz")), share: None, weight: 1.1 });
    g
}

/// Where the action buttons sit (centred in the safe width) and the band that holds them with the caption under the last.
fn actions_layout(face: &mut Typeface, has_items: bool) -> (Vec<(Button, Rect)>, Rect) {
    let widths: Vec<(Button, i32)> =
        (0..Button::count(has_items)).filter_map(|i| Button::at(has_items, i)).map(|b| (b, (face.measure(b.label(), BODY_PX) + 72).max(168))).collect();
    let row = widths.iter().map(|(_, w)| w).sum::<i32>() + BUTTON_GAP * (widths.len() as i32 - 1);
    let mut x = (SCREEN_W - row) / 2;
    let buttons: Vec<(Button, Rect)> = widths
        .into_iter()
        .map(|(b, w)| {
            let r = Rect::new(x, ACTIONS_Y, w, BUTTON_H);
            x += w + BUTTON_GAP;
            (b, r)
        })
        .collect();
    let mut band = Rect::new((SCREEN_W - row) / 2, ACTIONS_Y, row, ACTIONS_H);
    if let Some((_, r)) = buttons.last() {
        let cap = face.measure(REFRESHER_CAPTION, CAPTION_PX);
        band = band.union(&Rect::new(r.x + (r.w - cap) / 2, r.bottom() + CAPTION_GAP, cap, ACTIONS_H - BUTTON_H - CAPTION_GAP));
    }
    (buttons, band)
}

fn working_text(a: Action) -> &'static str {
    match a {
        Action::RestartLumaTv => "Restarting LumaTV...",
        Action::LgHome => "Switching to LG's Home...",
        Action::RestartTv => "Restarting the TV...",
    }
}

/// Greedy word wrap by measured width.
fn wrap(face: &mut Typeface, text: &str, px: u32, width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
            continue;
        }
        let longer = format!("{line} {word}");
        if face.measure(&longer, px) <= width {
            line = longer;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// What stands in a card when the compositor has no picture of the app to draw there.
fn initial(item: &Item) -> char {
    item.title.chars().chain(item.id.chars()).find(|c| c.is_alphanumeric()).map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
}

impl Screen {
    pub fn new() -> Screen {
        Screen {
            face: Typeface::new(),
            zone: Zone::Cards,
            card: 0,
            first: 0,
            button: Button::CloseAll,
            sheet: None,
            notice: None,
            working: None,
            drawn: [None; 3],
        }
    }

    /// Each opening: focus on the first card (on the actions once the list turns out empty), no sheet, no notice, every band dirty.
    pub fn reset(&mut self) {
        self.zone = Zone::Cards;
        self.card = 0;
        self.first = 0;
        self.button = Button::CloseAll;
        self.sheet = None;
        self.notice = None;
        self.working = None;
        self.drawn = [None; 3];
    }

    /// Keeps the focus on something that exists when the list changed underneath it.
    fn settle(&mut self, n: usize) {
        if n == 0 {
            self.zone = Zone::Actions;
            self.card = 0;
            self.first = 0;
            if self.button == Button::CloseAll {
                self.button = Button::at(false, 0).unwrap_or(self.button);
            }
        } else {
            self.card = self.card.min(n - 1);
            self.first = window(n, self.card, self.first).0;
        }
    }

    /// `refresher`: the panel's pixel refresher runs, so Restart TV is refused here.
    pub fn key(&mut self, key: Key, items: &[Item], refresher: bool) -> Event {
        if self.working.is_some() {
            return Event::Nothing;
        }
        let notice_gone = self.notice.take().is_some();
        self.settle(items.len());
        let event = match (self.sheet, self.zone) {
            (Some(sheet), _) => self.sheet_key(sheet, key),
            (None, Zone::Cards) => self.cards_key(key, items),
            (None, Zone::Actions) => self.actions_key(key, !items.is_empty(), refresher),
        };
        if notice_gone && event == Event::Nothing { Event::Redraw } else { event }
    }

    fn cards_key(&mut self, key: Key, items: &[Item]) -> Event {
        let Some(item) = items.get(self.card) else { return Event::Nothing };
        match key {
            Key::Left | Key::Right => {
                let to = if key == Key::Left { self.card.checked_sub(1) } else { Some(self.card + 1).filter(|c| *c < items.len()) };
                let Some(to) = to else { return Event::Nothing };
                self.card = to;
                self.first = window(items.len(), to, self.first).0;
                Event::Redraw
            }
            Key::Ok => Event::Activate(item.id.clone()),
            Key::Up if item.mark == Mark::Quitting && !item.hung => Event::Nothing,
            Key::Up => Event::Quit { id: item.id.clone(), force: item.hung || item.mark == Mark::Stuck },
            Key::Down => {
                self.zone = Zone::Actions;
                self.button = Button::at(true, 0).unwrap_or(self.button);
                Event::Redraw
            }
            Key::Back => Event::Close,
        }
    }

    fn actions_key(&mut self, key: Key, has_items: bool, refresher: bool) -> Event {
        match key {
            Key::Left | Key::Right => {
                let at = self.button.position(has_items);
                let to = if key == Key::Left { at.checked_sub(1) } else { Some(at + 1) };
                match to.and_then(|i| Button::at(has_items, i)) {
                    Some(b) => {
                        self.button = b;
                        Event::Redraw
                    }
                    None => Event::Nothing,
                }
            }
            Key::Ok => match self.button {
                Button::CloseAll => Event::QuitAll,
                Button::System(Action::RestartTv) if refresher => {
                    self.notice = Some(REFRESHER_NOTICE);
                    Event::Redraw
                }
                Button::System(action) => {
                    self.sheet = Some(Sheet { action, confirm: false });
                    Event::Redraw
                }
            },
            Key::Up if has_items => {
                self.zone = Zone::Cards;
                Event::Redraw
            }
            Key::Up | Key::Down => Event::Nothing,
            Key::Back => Event::Close,
        }
    }

    fn sheet_key(&mut self, sheet: Sheet, key: Key) -> Event {
        match key {
            Key::Left | Key::Right => {
                let confirm = key == Key::Right;
                if confirm == sheet.confirm {
                    return Event::Nothing;
                }
                self.sheet = Some(Sheet { confirm, ..sheet });
                Event::Redraw
            }
            Key::Ok if sheet.confirm => Event::System(sheet.action),
            Key::Ok | Key::Back => {
                self.sheet = None;
                Event::Redraw
            }
            Key::Up | Key::Down => Event::Nothing,
        }
    }

    /// Where the compositor draws the cards' pictures: the visible cards, in a window that follows the focus. None while a sheet is up or
    /// an action is under way.
    pub fn slots(&self, items: &[Item]) -> Vec<(usize, Rect)> {
        if self.sheet.is_some() || self.working.is_some() {
            return Vec::new();
        }
        let (first, count) = window(items.len(), self.card, self.first);
        slot_rects(count).enumerate().map(|(k, r)| (first + k, r)).collect()
    }

    /// The bands that changed since the last call: all of them after [`Screen::reset`] or a new palette, none when nothing changed. Under
    /// a sheet or a working message only band 0 is drawn (the sheet, alone), and bands 1 and 2 are taken off the screen.
    pub fn render(&mut self, v: &View) -> Vec<Band> {
        self.working = v.working;
        self.settle(v.items.len());
        let mut out = Vec::new();
        if self.working.is_some() || self.sheet.is_some() {
            let hash = fingerprint(|h| {
                1u8.hash(h);
                hash_palette(h, v.palette);
                self.working.map(Action::word).hash(h);
                self.sheet.map(|s| (s.action.word(), s.confirm)).hash(h);
            });
            if self.drawn[0] != Some(hash) {
                self.drawn[0] = Some(hash);
                out.push(self.draw_sheet(v));
            }
            for key in 1..3u8 {
                if self.drawn[key as usize].take().is_some() {
                    out.push(Band { key, rect: Rect::default(), rgba: Vec::new() });
                }
            }
            return out;
        }
        let n = v.items.len();
        let (first, count) = window(n, self.card, self.first);
        let notice = v.notice.or(self.notice);
        let force = v.items.get(self.card).is_some_and(|i| i.hung || i.mark == Mark::Stuck);
        let hashes = [
            fingerprint(|h| {
                0u8.hash(h);
                hash_palette(h, v.palette);
                (v.why, notice, n, first, self.card, self.zone == Zone::Cards, force).hash(h);
                for i in &v.items[first..first + count] {
                    (&i.title, i.picture as u8, i.mark as u8, i.hung).hash(h);
                }
            }),
            fingerprint(|h| {
                hash_palette(h, v.palette);
                v.stats.map(|s| (&s.cpu, s.total_kb, s.avail_kb, s.gpu, s.temp_c, s.mhz)).hash(h);
            }),
            fingerprint(|h| {
                hash_palette(h, v.palette);
                (n > 0, v.refresher, (self.zone == Zone::Actions).then(|| self.button.tag())).hash(h);
            }),
        ];
        for (key, hash) in hashes.into_iter().enumerate() {
            if self.drawn[key] == Some(hash) {
                continue;
            }
            self.drawn[key] = Some(hash);
            out.push(match key {
                0 => self.draw_main(v),
                1 => self.draw_stats(v),
                _ => self.draw_actions(v),
            });
        }
        out
    }

    fn draw_main(&mut self, v: &View) -> Band {
        let (zone, card) = (self.zone, self.card);
        let n = v.items.len();
        let (first, count) = window(n, card, self.first);
        let notice = v.notice.or(self.notice);
        let force = v.items.get(card).is_some_and(|i| i.hung || i.mark == Mark::Stuck);
        let mut p = Paint::new(MAIN_BAND, &mut self.face, v.palette);
        let t = p.t;
        p.text("Task manager", CONTENT_X, SAFE_Y, TITLE_PX, t.text, None);
        p.text(v.why, CONTENT_X, WHY_Y, CAPTION_PX, t.muted, Some(CONTENT_W));
        if n == 0 {
            let y = SLOT_Y + (CARD_H - p.face.line_height(BODY_PX)) / 2;
            p.centered("No apps are open.", SCREEN_W / 2, y, BODY_PX, t.muted, CONTENT_W);
        }
        for (k, (item, slot)) in v.items[first..first + count].iter().zip(slot_rects(count)).enumerate() {
            let quitting = item.mark == Mark::Quitting;
            if zone == Zone::Cards && first + k == card {
                p.cv.rounded_rect(grow(slot, RING), CARD_RADIUS + RING as f32, opaque(t.accent));
            }
            let tile = if quitting { t.tile_dim } else { t.tile };
            p.cv.rounded_rect(slot, CARD_RADIUS, opaque(tile));
            if matches!(item.picture, Picture::Video | Picture::None) {
                let y = slot.y + (slot.h - p.face.line_height(96)) / 2;
                p.centered(&initial(item).to_string(), slot.x + slot.w / 2, y, 96, mix(tile, t.text, if quitting { 0.3 } else { 0.6 }), slot.w);
            }
            p.text(&item.title, slot.x, LABEL_Y, LABEL_PX, if quitting { t.muted } else { t.text }, Some(CARD_W));
            let state = if item.hung {
                Some(("Not responding", t.danger))
            } else {
                match item.mark {
                    Mark::Quitting => Some(("Closing...", t.muted)),
                    Mark::Stuck => Some(("Did not close", t.danger)),
                    Mark::Nothing => None,
                }
            };
            if let Some((s, c)) = state {
                p.text(s, slot.x, STATE_Y, CAPTION_PX, c, Some(CARD_W));
            }
        }
        if n > MAX_SLOTS {
            let label = format!("{} of {n}", card + 1);
            let half = p.face.measure(&label, CAPTION_PX) / 2;
            p.centered(&label, SCREEN_W / 2, PAGE_Y, CAPTION_PX, t.muted, CONTENT_W);
            let cy = (PAGE_Y + p.face.line_height(CAPTION_PX) / 2) as f32;
            // Fixed positions, dimmed when nothing lies that way, so the row does not jump.
            for (more, dir) in [(first > 0, -1.0f32), (first + count < n, 1.0)] {
                let tip = (SCREEN_W / 2) as f32 + dir * (half as f32 + 28.0);
                let base = tip - dir * 11.0;
                let c = with_alpha(t.muted, if more { 1.0 } else { 0.25 });
                p.cv.triangle([(tip, cy), (base, cy - 11.0), (base, cy + 11.0)], c);
            }
        }
        let pairs = hint(zone, force, n > 0);
        let measured: Vec<(i32, i32)> = pairs.iter().map(|(k, w)| (p.face.measure(k, CAPTION_PX), p.face.measure(w, CAPTION_PX))).collect();
        let total = measured.iter().map(|(k, w)| k + HINT_KEY_GAP + w).sum::<i32>() + HINT_GAP * (pairs.len() as i32 - 1);
        let mut x = (SCREEN_W - total) / 2;
        for ((key, word), (kw, ww)) in pairs.iter().zip(measured) {
            p.text(key, x, HINT_Y, CAPTION_PX, t.text, None);
            p.text(word, x + kw + HINT_KEY_GAP, HINT_Y, CAPTION_PX, t.muted, None);
            x += kw + HINT_KEY_GAP + ww + HINT_GAP;
        }
        if let Some(s) = notice {
            p.centered(s, SCREEN_W / 2, NOTICE_Y, CAPTION_PX, t.danger, CONTENT_W);
        }
        p.into_band(0)
    }

    fn draw_stats(&mut self, v: &View) -> Band {
        let columns = gauges(v.stats, self.face.has('\u{b0}'));
        let mut p = Paint::new(STATS_BAND, &mut self.face, v.palette);
        let t = p.t;
        let total: f32 = columns.iter().map(|g| g.weight).sum();
        let room = (CONTENT_W - GAUGE_GAP * (columns.len() as i32 - 1)) as f32;
        let edge = |weight: f32| CONTENT_X + (room * weight / total).round() as i32;
        let mut before = 0.0;
        for (i, g) in columns.iter().enumerate() {
            let x = edge(before) + i as i32 * GAUGE_GAP;
            before += g.weight;
            let w = edge(before) + i as i32 * GAUGE_GAP - x;
            p.text(&g.label, x, STATS_Y, CAPTION_PX, t.muted, Some(w));
            let bar = Rect::new(x, STATS_Y + 41, w, BAR_H);
            if let Some(share) = g.share {
                p.cv.rounded_rect(bar, BAR_H as f32 / 2.0, with_alpha(t.muted, 0.28));
                let fill = ((w as f32 * share.clamp(0.0, 1.0)).round() as i32).max(if share > 0.0 { 4 } else { 0 });
                let c = if share >= 0.9 { t.danger } else { t.accent };
                p.cv.rounded_rect(Rect::new(x, bar.y, fill, BAR_H), (fill as f32 / 2.0).min(BAR_H as f32 / 2.0), opaque(c));
            }
            p.text(&g.value, x, STATS_Y + 61, BODY_PX, t.text, Some(w));
        }
        p.into_band(1)
    }

    fn draw_actions(&mut self, v: &View) -> Band {
        let (buttons, band) = actions_layout(&mut self.face, !v.items.is_empty());
        let focus = (self.zone == Zone::Actions).then_some(self.button);
        let mut p = Paint::new(band, &mut self.face, v.palette);
        let t = p.t;
        for (b, r) in buttons {
            let refused = v.refresher && b == Button::System(Action::RestartTv);
            let focused = focus == Some(b);
            let lit = focused && !refused;
            let (fill, ink) = if lit {
                (t.accent, t.ink_on(t.accent))
            } else if refused {
                (t.button, t.muted)
            } else {
                (t.button, t.text)
            };
            p.cv.rounded_rect(r, CONTROL_RADIUS, opaque(fill));
            p.label(b.label(), r, BODY_PX, ink);
            if refused {
                if focused {
                    p.cv.ring(r, CONTROL_RADIUS, 4, opaque(t.accent));
                }
                p.centered(REFRESHER_CAPTION, r.x + r.w / 2, r.bottom() + CAPTION_GAP, CAPTION_PX, t.muted, CONTENT_W);
            }
        }
        p.into_band(2)
    }

    fn draw_sheet(&mut self, v: &View) -> Band {
        let h = if self.working.is_some() { WORKING_H } else { SHEET_H };
        let sheet = sheet_rect(h);
        let (working, confirm) = (self.working, self.sheet);
        let mut p = Paint::new(sheet_band(h), &mut self.face, v.palette);
        let t = p.t;
        for k in (1..=8).rev() {
            p.cv.rounded_rect(grow(sheet, k * 4).translated(0, 10), SHEET_RADIUS + (k * 4) as f32, [0, 0, 0, 6]);
        }
        p.cv.rounded_rect(sheet, SHEET_RADIUS, opaque(t.sheet));
        let inner = sheet.w - 2 * SHEET_PAD;
        if let Some(action) = working {
            p.label(working_text(action), sheet, TITLE_PX, t.text);
        } else if let Some(Sheet { action, confirm }) = confirm {
            p.text(action.ask(), sheet.x + SHEET_PAD, sheet.y + 48, TITLE_PX, t.text, Some(inner));
            let mut y = sheet.y + 48 + p.face.line_height(TITLE_PX) + 16;
            for line in wrap(p.face, action.detail(), BODY_PX, inner) {
                p.text(&line, sheet.x + SHEET_PAD, y, BODY_PX, t.text, Some(inner));
                y += BODY_LINE;
            }
            let top = sheet.bottom() - 48 - BUTTON_H;
            let ok_x = sheet.right() - SHEET_PAD - SHEET_BUTTON_W;
            let cancel = Rect::new(ok_x - BUTTON_GAP - SHEET_BUTTON_W, top, SHEET_BUTTON_W, BUTTON_H);
            let ok = Rect::new(ok_x, top, SHEET_BUTTON_W, BUTTON_H);
            let ok_fill = if action == Action::LgHome { t.accent } else { t.danger };
            for (r, fill, label, focused) in [(cancel, t.sheet_button, "Cancel", !confirm), (ok, ok_fill, action.label(), confirm)] {
                p.cv.rounded_rect(r, CONTROL_RADIUS, opaque(fill));
                p.label(label, r, BODY_PX, t.ink_on(fill));
                if focused {
                    // An accent ring would vanish round an accent button.
                    let c = if fill == t.accent { t.text } else { t.accent };
                    p.cv.ring(grow(r, 7), CONTROL_RADIUS + 7.0, 4, opaque(c));
                }
            }
        }
        p.into_band(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: usize) -> Item {
        Item { id: format!("app.{n}"), title: format!("App {n}"), picture: Picture::Live, mark: Mark::Nothing, hung: false }
    }

    fn items(n: usize) -> Vec<Item> {
        (0..n).map(item).collect()
    }

    fn reading() -> Stats {
        Stats { cpu: vec![12, 47, 91], total_kb: 2_030_392, avail_kb: 564_304, cached_kb: 268_104, gpu: Some(23), temp_c: Some(53), mhz: Some(1400), refresher: false }
    }

    fn view<'a>(items: &'a [Item], stats: Option<&'a Stats>, palette: &'a Palette) -> View<'a> {
        View { items, stats, palette, why: "LumaTV's own screen is not answering. This is the basic screen.", notice: None, refresher: false, working: None }
    }

    fn press(s: &mut Screen, items: &[Item], keys: &[Key]) -> Event {
        keys.iter().map(|k| s.key(*k, items, false)).last().unwrap_or(Event::Nothing)
    }

    fn keys_of(bands: &[Band]) -> Vec<u8> {
        bands.iter().map(|b| b.key).collect()
    }

    #[test]
    fn the_slots_sit_inside_the_safe_area_apart_and_centred() {
        let pal = Palette::default();
        let _ = &pal;
        for n in [0, 1, 3, 5, 9] {
            let list = items(n);
            let slots = Screen::new().slots(&list);
            assert_eq!(slots.len(), n.min(5), "{n} items");
            for (k, (i, r)) in slots.iter().enumerate() {
                assert_eq!(*i, k);
                assert_eq!((r.w, r.h), (312, 175));
                let ring = grow(*r, RING);
                assert!(ring.x >= SAFE_X && ring.right() <= SCREEN_W - SAFE_X && ring.y >= SAFE_Y && ring.bottom() <= SCREEN_H - SAFE_Y, "{ring:?}");
                for (_, o) in &slots[k + 1..] {
                    assert!(r.intersect(&grow(*o, RING)).is_empty(), "{r:?} meets {o:?}");
                }
            }
            if let (Some((_, a)), Some((_, b))) = (slots.first(), slots.last()) {
                assert!((a.x - SAFE_X - (SCREEN_W - SAFE_X - b.right())).abs() <= 1, "the row is centred");
            }
        }
    }

    #[test]
    fn the_window_follows_the_focus_and_only_as_far_as_it_must() {
        let list = items(9);
        let mut s = Screen::new();
        let shown = |s: &Screen| s.slots(&list).iter().map(|(i, _)| *i).collect::<Vec<_>>();
        assert_eq!(shown(&s), [0, 1, 2, 3, 4]);
        press(&mut s, &list, &[Key::Right; 4]);
        assert_eq!(shown(&s), [0, 1, 2, 3, 4]);
        press(&mut s, &list, &[Key::Right; 2]);
        assert_eq!(shown(&s), [2, 3, 4, 5, 6], "the 7th is at the right edge");
        press(&mut s, &list, &[Key::Right; 5]);
        assert_eq!(shown(&s), [4, 5, 6, 7, 8]);
        press(&mut s, &list, &[Key::Left; 4]);
        assert_eq!(shown(&s), [4, 5, 6, 7, 8], "moving back inside the window leaves it");
        press(&mut s, &list, &[Key::Left]);
        assert_eq!(shown(&s), [3, 4, 5, 6, 7]);
    }

    #[test]
    fn the_cards_keys() {
        let mut list = items(3);
        list[1].hung = true;
        list[2].mark = Mark::Stuck;
        let mut s = Screen::new();
        assert_eq!(s.key(Key::Left, &list, false), Event::Nothing);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Activate("app.0".into()));
        assert_eq!(s.key(Key::Up, &list, false), Event::Quit { id: "app.0".into(), force: false });
        assert_eq!(s.key(Key::Right, &list, false), Event::Redraw);
        assert_eq!(s.key(Key::Up, &list, false), Event::Quit { id: "app.1".into(), force: true }, "a hung app is killed");
        assert_eq!(s.key(Key::Right, &list, false), Event::Redraw);
        assert_eq!(s.key(Key::Up, &list, false), Event::Quit { id: "app.2".into(), force: true }, "a stuck app is forced");
        assert_eq!(s.key(Key::Right, &list, false), Event::Nothing, "no wrap past the last card");
        assert_eq!(s.key(Key::Back, &list, false), Event::Close);
        list[2].mark = Mark::Quitting;
        assert_eq!(s.key(Key::Up, &list, false), Event::Nothing, "already closing");
        list[2].hung = true;
        assert_eq!(s.key(Key::Up, &list, false), Event::Quit { id: "app.2".into(), force: true }, "closing but hung");
        assert_eq!(s.key(Key::Down, &list, false), Event::Redraw);
    }

    #[test]
    fn the_actions_keys() {
        let list = items(3);
        let mut s = Screen::new();
        press(&mut s, &list, &[Key::Right, Key::Right, Key::Down]);
        assert_eq!(s.key(Key::Left, &list, false), Event::Nothing, "Close all apps is first");
        assert_eq!(s.key(Key::Ok, &list, false), Event::QuitAll);
        assert_eq!(press(&mut s, &list, &[Key::Right; 3]), Event::Redraw);
        assert_eq!(s.key(Key::Right, &list, false), Event::Nothing, "no wrap past Restart TV");
        assert_eq!(s.key(Key::Down, &list, false), Event::Nothing);
        assert_eq!(s.key(Key::Back, &list, false), Event::Close);
        assert_eq!(s.key(Key::Up, &list, false), Event::Redraw);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Activate("app.2".into()), "focus stays on the card it left");
    }

    #[test]
    fn with_no_apps_the_actions_are_all_there_is() {
        let mut s = Screen::new();
        assert_eq!(s.key(Key::Up, &[], false), Event::Nothing);
        assert_eq!(s.key(Key::Left, &[], false), Event::Nothing, "no Close all apps to go left to");
        assert_eq!(s.key(Key::Down, &[], false), Event::Nothing);
        assert_eq!(s.key(Key::Ok, &[], false), Event::Redraw, "the first button is Restart LumaTV");
        assert_eq!(s.key(Key::Back, &[], false), Event::Redraw, "Back closes the sheet");
        assert_eq!(s.key(Key::Back, &[], false), Event::Close);
        assert!(s.slots(&[]).is_empty());
    }

    #[test]
    fn the_sheet_opens_on_cancel_and_acts_only_on_the_action() {
        let list = items(2);
        let mut s = Screen::new();
        press(&mut s, &list, &[Key::Down, Key::Right]);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Redraw);
        assert!(s.slots(&list).is_empty(), "the cards are hidden behind the sheet");
        assert_eq!(s.key(Key::Left, &list, false), Event::Nothing, "Cancel is already focused");
        assert_eq!(s.key(Key::Up, &list, false), Event::Nothing);
        assert_eq!(s.key(Key::Down, &list, false), Event::Nothing);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Redraw, "OK on Cancel closes it");
        assert_eq!(s.slots(&list).len(), 2);
        s.key(Key::Ok, &list, false);
        assert_eq!(s.key(Key::Back, &list, false), Event::Redraw);
        assert_eq!(s.slots(&list).len(), 2);
        s.key(Key::Ok, &list, false);
        assert_eq!(s.key(Key::Right, &list, false), Event::Redraw);
        assert_eq!(s.key(Key::Right, &list, false), Event::Nothing);
        assert_eq!(s.key(Key::Ok, &list, false), Event::System(Action::RestartLumaTv));
        assert!(s.slots(&list).is_empty(), "the sheet stays until reset");
        s.reset();
        assert_eq!(s.slots(&list).len(), 2);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Activate("app.0".into()));
    }

    #[test]
    fn every_system_action_is_reached_through_its_own_sheet() {
        let list = items(1);
        for (presses, action) in [(1, Action::RestartLumaTv), (2, Action::LgHome), (3, Action::RestartTv)] {
            let mut s = Screen::new();
            press(&mut s, &list, &[Key::Down]);
            press(&mut s, &list, &vec![Key::Right; presses]);
            s.key(Key::Ok, &list, false);
            assert_eq!(press(&mut s, &list, &[Key::Right, Key::Ok]), Event::System(action));
        }
    }

    #[test]
    fn a_restart_tv_during_the_refresher_is_refused_with_a_notice() {
        let list = items(1);
        let pal = Palette::default();
        let mut s = Screen::new();
        press(&mut s, &list, &[Key::Down, Key::Right, Key::Right, Key::Right]);
        let mut v = view(&list, None, &pal);
        s.render(&v);
        assert_eq!(s.key(Key::Ok, &list, true), Event::Redraw);
        assert_eq!(s.slots(&list).len(), 1, "no sheet");
        v.refresher = true;
        let bands = s.render(&v);
        assert_eq!(keys_of(&bands), [0, 2], "the notice and the muted button");
        assert_eq!(s.key(Key::Right, &list, true), Event::Redraw, "the notice goes with the next key");
        assert_eq!(keys_of(&s.render(&v)), [0]);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Redraw);
        assert!(s.slots(&list).is_empty(), "with the refresher over, a sheet opens");
    }

    #[test]
    fn nothing_is_accepted_while_an_action_is_under_way() {
        let list = items(2);
        let pal = Palette::default();
        let mut s = Screen::new();
        let mut v = view(&list, None, &pal);
        v.working = Some(Action::RestartLumaTv);
        s.render(&v);
        for k in [Key::Left, Key::Right, Key::Up, Key::Down, Key::Ok, Key::Back] {
            assert_eq!(s.key(k, &list, false), Event::Nothing);
        }
        assert!(s.slots(&list).is_empty());
        v.working = None;
        s.render(&v);
        assert_eq!(s.key(Key::Ok, &list, false), Event::Activate("app.0".into()));
    }

    #[test]
    fn the_focus_follows_a_list_that_shrinks() {
        let list = items(5);
        let pal = Palette::default();
        let mut s = Screen::new();
        press(&mut s, &list, &[Key::Right; 4]);
        let two = items(2);
        assert_eq!(s.key(Key::Left, &two, false), Event::Redraw, "clamped to the 2nd card, then moved to the 1st");
        assert_eq!(s.key(Key::Ok, &two, false), Event::Activate("app.0".into()));

        let mut s = Screen::new();
        press(&mut s, &list, &[Key::Right; 4]);
        s.render(&view(&two, None, &pal));
        assert_eq!(s.slots(&two).iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1]);

        let mut s = Screen::new();
        press(&mut s, &list, &[Key::Right; 2]);
        s.render(&view(&[], None, &pal));
        assert!(s.slots(&[]).is_empty());
        assert_eq!(s.key(Key::Up, &[], false), Event::Nothing, "with no apps the zone is the actions");
        assert_eq!(s.key(Key::Ok, &[], false), Event::Redraw);
    }

    #[test]
    fn the_hints_say_what_the_keys_do_now() {
        let joined = |pairs: Vec<(&str, &str)>| pairs.iter().map(|(k, w)| format!("{k} {w}")).collect::<Vec<_>>().join("   ");
        assert_eq!(joined(hint(Zone::Cards, false, true)), "Left/Right choose   OK open   Up close   Down more   Back leave");
        assert_eq!(joined(hint(Zone::Cards, true, true)), "Left/Right choose   OK open   Up force quit   Down more   Back leave");
        assert_eq!(joined(hint(Zone::Actions, false, true)), "Left/Right choose   OK select   Up apps   Back leave");
        assert_eq!(joined(hint(Zone::Actions, false, false)), "Left/Right choose   OK select   Back leave", "there is no list to go up to");
    }

    #[test]
    fn dirty_bands_are_only_what_changed() {
        let list = items(5);
        let pal = Palette::default();
        let stats = reading();
        let mut s = Screen::new();
        let v = view(&list, Some(&stats), &pal);
        assert_eq!(keys_of(&s.render(&v)), [0, 1, 2]);
        assert!(s.render(&v).is_empty(), "the same view twice draws nothing");

        let busier = Stats { cpu: vec![12, 47, 92], ..reading() };
        assert_eq!(keys_of(&s.render(&view(&list, Some(&busier), &pal))), [1]);
        assert_eq!(keys_of(&s.render(&view(&list, None, &pal))), [1]);
        s.render(&v);

        s.key(Key::Right, &list, false);
        assert_eq!(keys_of(&s.render(&v)), [0], "a card focus move");
        s.key(Key::Down, &list, false);
        assert_eq!(keys_of(&s.render(&v)), [0, 2], "the ring leaves the card and the button lights");
        s.key(Key::Right, &list, false);
        assert_eq!(keys_of(&s.render(&v)), [2], "a button focus move");

        s.reset();
        assert_eq!(keys_of(&s.render(&v)), [0, 1, 2]);
        let other = Palette { accent: [1, 200, 3], ..Palette::default() };
        assert_eq!(keys_of(&s.render(&view(&list, Some(&stats), &other))), [0, 1, 2]);

        let mut refresher = view(&list, Some(&stats), &other);
        refresher.refresher = true;
        assert_eq!(keys_of(&s.render(&refresher)), [2]);
        refresher.notice = Some("Something failed.");
        assert_eq!(keys_of(&s.render(&refresher)), [0]);
        refresher.why = "Another reason.";
        assert_eq!(keys_of(&s.render(&refresher)), [0]);
        let fewer = items(4);
        assert_eq!(keys_of(&s.render(&View { items: &fewer, ..refresher })), [0], "the page count changed");
        assert_eq!(keys_of(&s.render(&View { items: &[], ..refresher })), [0, 2], "no apps: no Close all apps");
    }

    #[test]
    fn a_hung_state_change_redraws_the_cards_only() {
        let mut list = items(3);
        let pal = Palette::default();
        let mut s = Screen::new();
        s.render(&view(&list, None, &pal));
        list[1].hung = true;
        assert_eq!(keys_of(&s.render(&view(&list, None, &pal))), [0]);
        list[1].title = "Renamed".into();
        assert_eq!(keys_of(&s.render(&view(&list, None, &pal))), [0]);
    }

    #[test]
    fn a_sheet_replaces_band_zero_and_takes_the_others_off_the_screen() {
        let list = items(3);
        let pal = Palette::default();
        let stats = reading();
        let mut s = Screen::new();
        let v = view(&list, Some(&stats), &pal);
        s.render(&v);
        press(&mut s, &list, &[Key::Down, Key::Right]);
        s.key(Key::Ok, &list, false);
        let bands = s.render(&v);
        assert_eq!(keys_of(&bands), [0, 1, 2]);
        assert_eq!(bands[0].rect, sheet_band(SHEET_H));
        assert!(bands[1].rect.is_empty() && bands[1].rgba.is_empty() && bands[2].rect.is_empty());
        assert!(s.render(&v).is_empty());
        s.key(Key::Right, &list, false);
        assert_eq!(keys_of(&s.render(&v)), [0], "the focus moved to the action");
        s.key(Key::Back, &list, false);
        assert_eq!(keys_of(&s.render(&v)), [0, 1, 2], "back to the main screen");
        let mut working = v;
        working.working = Some(Action::LgHome);
        let bands = s.render(&working);
        assert_eq!(bands[0].rect, sheet_band(WORKING_H));
        assert_eq!(keys_of(&bands), [0, 1, 2]);
    }

    #[test]
    fn the_bands_do_not_overlap_and_stay_under_the_pixel_budget() {
        let pal = Palette::default();
        let stats = reading();
        let mut worst = 0;
        for n in [0, 9] {
            let list = items(n);
            {
                let mut s = Screen::new();
                let mut v = view(&list, Some(&stats), &pal);
                v.refresher = true;
                v.notice = Some("A line from the compositor.");
                let mut states = vec![s.render(&v)];
                press(&mut s, &list, &[Key::Down, Key::Right]);
                s.key(Key::Ok, &list, false);
                states.push(s.render(&v));
                v.working = Some(Action::RestartTv);
                states.push(s.render(&v));
                for bands in &states {
                    let mut pixels = 0;
                    for (i, a) in bands.iter().enumerate() {
                        assert_eq!(a.rgba.len(), (a.rect.w * a.rect.h * 4) as usize);
                        assert!(a.rect.is_empty() || Rect::new(0, 0, SCREEN_W, SCREEN_H).intersect(&a.rect) == a.rect, "{:?} leaves the screen", a.rect);
                        pixels += a.rect.w * a.rect.h;
                        for b in &bands[i + 1..] {
                            assert!(a.rect.intersect(&b.rect).is_empty(), "{:?} meets {:?}", a.rect, b.rect);
                        }
                    }
                    worst = worst.max(pixels);
                }
            }
        }
        assert!(worst < SCREEN_W * SCREEN_H * 6 / 10, "{worst} pixels");
    }

    #[test]
    fn a_band_is_transparent_where_nothing_is_drawn() {
        let pal = Palette::default();
        let stats = reading();
        let list = items(5);
        let bands = Screen::new().render(&view(&list, Some(&stats), &pal));
        for b in &bands {
            assert_eq!(&b.rgba[b.rgba.len() - 4..], [0; 4], "band {} is flat where it has no element", b.key);
        }
        let main = &bands[0];
        assert!(main.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count() > 100_000, "the tiles are opaque");
    }

    #[test]
    fn what_a_card_says_about_its_app() {
        let mut list = vec![item(0), item(1), item(2), item(3)];
        list[0].picture = Picture::Video;
        list[1].picture = Picture::None;
        list[1].hung = true;
        list[2].mark = Mark::Quitting;
        list[3].mark = Mark::Stuck;
        let pal = Palette::default();
        let plain = Screen::new().render(&view(&items(4), None, &pal));
        let marked = Screen::new().render(&view(&list, None, &pal));
        assert_ne!(plain[0].rgba, marked[0].rgba);
        assert_eq!(initial(&Item { title: "plex".into(), ..item(0) }), 'P');
        assert_eq!(initial(&Item { title: "  ".into(), id: "com.x".into(), ..item(0) }), 'C');
        assert_eq!(initial(&Item { title: "".into(), id: "".into(), ..item(0) }), '?');
    }

    #[test]
    fn the_stats_row_reads_each_number_in_its_own_words() {
        let g = gauges(Some(&reading()), true);
        let words: Vec<(&str, &str)> = g.iter().map(|g| (g.label.as_str(), g.value.as_str())).collect();
        assert_eq!(
            words,
            [("CPU 1", "12%"), ("CPU 2", "47%"), ("CPU 3", "91%"), ("Memory", "1.4 of 1.9 GB"), ("GPU", "23%"), ("Temperature", "53 \u{b0}C"), ("Clock", "1400 MHz")]
        );
        assert!(g[2].share >= Some(0.9) && g[0].share < Some(0.9));
        assert!(gauges(Some(&reading()), false).iter().any(|g| g.value == "53 C"));
        let none = gauges(None, true);
        assert_eq!(none.iter().map(|g| g.value.as_str()).collect::<Vec<_>>(), ["--", "--", "--", "--", "--"]);
        let early = Stats { cpu: vec![], gpu: None, temp_c: None, mhz: None, ..reading() };
        assert_eq!(gauges(Some(&early), true)[0].label, "CPU");
        assert_eq!(gauges(Some(&early), true)[2].value, "--");
        assert!(Typeface::new().has('\u{b0}'), "Manrope has the degree sign");
    }

    #[test]
    fn the_buttons_are_big_enough_to_hit() {
        let mut face = Typeface::new();
        for has_items in [true, false] {
            let (buttons, band) = actions_layout(&mut face, has_items);
            assert_eq!(buttons.len(), if has_items { 4 } else { 3 });
            let mut right = i32::MIN;
            for (_, r) in &buttons {
                assert!(r.h >= 64 && r.w >= 64);
                assert!(r.x > right, "buttons do not touch");
                right = r.right();
                assert!(band.intersect(r) == *r);
            }
            assert!(buttons[0].1.x >= SAFE_X && right <= SCREEN_W - SAFE_X);
        }
        assert_eq!(Button::at(true, 0), Some(Button::CloseAll));
        assert_eq!(Button::at(false, 0), Some(Button::System(Action::RestartLumaTv)));
        assert_eq!(Button::at(false, 3), None);
        assert_eq!(Button::count(true), 4);
    }

    #[test]
    fn long_text_wraps_by_measure() {
        let mut face = Typeface::new();
        for a in Action::ALL {
            let lines = wrap(&mut face, a.detail(), BODY_PX, 788);
            assert!(!lines.is_empty() && lines.len() <= 3, "{lines:?}");
            assert!(lines.iter().all(|l| face.measure(l, BODY_PX) <= 788));
            assert_eq!(lines.join(" "), a.detail());
        }
        assert_eq!(wrap(&mut face, "", BODY_PX, 100), Vec::<String>::new());
        assert_eq!(wrap(&mut face, "Unbreakable", BODY_PX, 10), ["Unbreakable"]);
    }

    #[test]
    fn button_labels_stay_readable_on_the_palettes_own_colours() {
        let t = Tones::new(&Palette::default());
        for fill in [t.accent, t.danger] {
            assert!(contrast(fill, t.ink_on(fill)) >= 4.5);
        }
        let light = Palette { panel: [250, 244, 237], text: [87, 82, 121], muted: [121, 117, 147], accent: [144, 122, 169], danger: [180, 99, 122] };
        let t = Tones::new(&light);
        assert!(contrast(t.panel, t.text) >= 4.5);
        assert!(contrast(t.accent, t.ink_on(t.accent)) >= 3.0);
    }

    fn compose(s: &mut Screen, v: &View) -> Canvas {
        let panel = v.palette.panel;
        let mut screen = Canvas::new(Rect::new(0, 0, SCREEN_W, SCREEN_H));
        // A stand-in for whatever is behind the dim.
        screen.fill_rect(Rect::new(0, 0, SCREEN_W, SCREEN_H), opaque([70, 80, 110]));
        screen.fill_rect(Rect::new(0, 0, SCREEN_W, SCREEN_H), with_alpha(panel, 0.96));
        for b in s.render(v) {
            if !b.rect.is_empty() {
                screen.blit(b.rect, &b.rgba);
            }
        }
        let fills = [[200, 90, 60], [60, 140, 200], [90, 170, 90], [190, 160, 60], [150, 90, 190]];
        for (i, r) in s.slots(v.items) {
            let it = &v.items[i];
            if matches!(it.picture, Picture::Live | Picture::Thumb) {
                screen.rounded_rect(r, CARD_RADIUS, opaque(fills[i % fills.len()]));
                let word = if it.picture == Picture::Live { "live" } else { "thumb" };
                screen.text(&mut s.face, word, (r.x + 16, r.y + 12), 24, opaque([255, 255, 255]), None);
            }
        }
        screen
    }

    fn write_png(dir: &str, name: &str, screen: Canvas) {
        let file = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/{name}.png")).unwrap());
        let mut enc = png::Encoder::new(file, SCREEN_W as u32, SCREEN_H as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&screen.into_rgba()).unwrap();
    }

    /// Writes the screens to the directory named by `TASKMGR_SHOTS`, for looking at; does nothing without it.
    #[test]
    fn screenshots() {
        let Ok(dir) = std::env::var("TASKMGR_SHOTS") else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let pal = Palette::default();
        let stats = reading();
        let titles = ["Netflix", "YouTube", "Plex", "LumaGame: The Legend of a Very Long Adventure Title", "Disney+", "Prime Video", "Spotify", "Kodi", "Steam Link"];
        let mut apps: Vec<Item> = titles
            .iter()
            .enumerate()
            .map(|(i, t)| Item { id: format!("com.example.{i}"), title: (*t).into(), picture: [Picture::Live, Picture::Thumb, Picture::Video, Picture::None][i % 4], mark: Mark::Nothing, hung: false })
            .collect();

        let five = apps[..5].to_vec();
        let mut s = Screen::new();
        s.key(Key::Right, &five, false);
        write_png(&dir, "a-five-apps", compose(&mut s, &view(&five, Some(&stats), &pal)));

        let mut s = Screen::new();
        write_png(&dir, "b-no-apps", compose(&mut s, &view(&[], None, &pal)));

        apps[3].hung = true;
        apps[5].mark = Mark::Quitting;
        apps[6].mark = Mark::Stuck;
        let mut s = Screen::new();
        press(&mut s, &apps, &[Key::Right; 6]);
        write_png(&dir, "c-nine-apps-seventh", compose(&mut s, &view(&apps, Some(&stats), &pal)));

        let mut s = Screen::new();
        press(&mut s, &apps[..3], &[Key::Down, Key::Right, Key::Ok]);
        write_png(&dir, "d-restart-lumatv-sheet", compose(&mut s, &view(&apps[..3], Some(&stats), &pal)));

        let mut s = Screen::new();
        press(&mut s, &apps[..3], &[Key::Down, Key::Right, Key::Right, Key::Ok, Key::Right]);
        write_png(&dir, "d2-lg-home-sheet-confirm-focused", compose(&mut s, &view(&apps[..3], Some(&stats), &pal)));

        let mut s = Screen::new();
        let mut v = view(&apps[..3], Some(&stats), &pal);
        v.refresher = true;
        for k in [Key::Down, Key::Right, Key::Right, Key::Right] {
            s.key(k, &apps[..3], true);
        }
        assert_eq!(s.key(Key::Ok, &apps[..3], true), Event::Redraw);
        write_png(&dir, "e-restart-tv-refresher", compose(&mut s, &v));

        let mut s = Screen::new();
        let mut v = view(&apps[..3], Some(&stats), &pal);
        v.working = Some(Action::RestartLumaTv);
        write_png(&dir, "f-working", compose(&mut s, &v));

        let light = Palette { panel: [250, 244, 237], text: [87, 82, 121], muted: [121, 117, 147], accent: [144, 122, 169], danger: [180, 99, 122] };
        let mut s = Screen::new();
        s.key(Key::Right, &five, false);
        write_png(&dir, "g-light-palette", compose(&mut s, &view(&five, Some(&stats), &light)));
    }
}
