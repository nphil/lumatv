//! The task manager (`docs/research/task-manager.md`): the screen a long press of Back opens, which began as the app switcher. It lists what is
//! open, how the TV is doing and the few things a person does about either. The compositor owns the opening and the closing and draws each
//! app's picture where [`Switcher::layout`] says (`src/switcher.rs` of the compositor); this model decides the cards, the focus and the
//! dialogs, and `switcher_surface.rs` draws everything else over the pictures.
//!
//! Keys: Left and Right move between the cards, OK opens one, Up quits it (the card flies up and its neighbours close the gap). Down goes to
//! Close all and System below the cards and Up comes back. Back goes up one step: a confirmation, then the System sheet, then the screen
//! itself (the compositor's Home closes it as well). A pointer clicks the same things, and a card dragged up is quit. Close all and every
//! System choice ask first.
//!
//! The compositor's topics: `apps` (what is open: `{id, title, picture: live|thumb|video|none, state, hung, fg}`), `switcher` (`{open, from, n,
//! mode}`: `fallback` is the compositor's own basic screen, which this neither draws nor takes keys for) and `stats` (once a second while open;
//! each is answered by a `beat`, or the compositor takes its basic screen over). 1920x1080 TV pixels.

use super::{K, Out};
use crate::host::apps::Apps;
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

const PATH: &str = "/v1/switcher";

/// The cards' places. A card in focus fills its footprint, 400 by 225 (16:9); the others are that footprint at [`geo::REST`] about its centre.
/// Four fit side by side with 32 between and start at x 112 (fewer are centred); more are a window of four that moves by a card when the
/// focus leaves it. How high the row stands depends on the size of the text (the lower area grows with it), so the surface says: `top`.
pub mod geo {
    pub const W: f32 = 400.0;
    pub const H: f32 = 225.0;
    pub const GAP: f32 = 32.0;
    pub const PITCH: f32 = W + GAP;
    pub const RADIUS: f32 = 24.0;
    /// A card out of focus: 336 by 189, whole pixels both ways.
    pub const REST: f32 = 0.84;
    /// How far a card that is sent away flies up.
    pub const FLY: f32 = 120.0;
    pub const VISIBLE: usize = 4;
    pub const SCREEN: f32 = 1920.0;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Rect {
        pub x: f32,
        pub y: f32,
        pub w: f32,
        pub h: f32,
        pub r: f32,
    }

    pub fn shown(n: usize) -> usize {
        n.min(VISIBLE)
    }

    pub fn x0(n: usize) -> f32 {
        let v = shown(n) as f32;
        ((SCREEN - (v * W + (v - 1.0).max(-1.0) * GAP)) / 2.0).round()
    }

    /// The first card of the window, moved only as far as keeps `index` inside it.
    pub fn window_start(n: usize, index: usize, first: usize) -> usize {
        let last = n.saturating_sub(VISIBLE);
        let first = if index < first {
            index
        } else if index > first + VISIBLE - 1 {
            index + 1 - VISIBLE
        } else {
            first
        };
        first.min(last)
    }

    pub fn in_window(i: usize, first: usize) -> bool {
        i >= first && i < first + VISIBLE
    }

    /// Card `i`'s footprint when the window starts at `first` and the row stands at `top`: cards past either end carry on along the same
    /// row, out of sight.
    pub fn card(i: usize, n: usize, first: usize, top: f32) -> Rect {
        Rect { x: x0(n) + (i as f32 - first as f32) * PITCH, y: top, w: W, h: H, r: RADIUS }
    }

    /// The footprint as a card out of focus has it.
    pub fn rest(f: Rect) -> Rect {
        let (w, h) = ((f.w * REST).round(), (f.h * REST).round());
        Rect { x: f.x + ((f.w - w) / 2.0).round(), y: f.y + ((f.h - h) / 2.0).round(), w, h, r: (f.r * REST).round() }
    }

    /// Where a card is: its footprint in focus, else resting.
    pub fn pose(f: Rect, focused: bool) -> Rect {
        if focused { f } else { rest(f) }
    }

    pub fn flown(r: Rect) -> Rect {
        Rect { y: r.y - FLY, ..r }
    }
}

/// Who draws an app's picture: the compositor (`live`, and the still of `thumb`), or nobody (`video` is on the video plane and `none` has
/// none): then the card is a tile with the app's icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Shown,
    Tile,
}

/// How an app stands, which says what Up does to its card and what the card says: a hung app is not responding (it is quit at once), one
/// that was asked to quit is closing, and one that did not quit in time stays until it is forced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Fine,
    Hung,
    Closing,
    Stuck,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    /// The app's icon file ("" for none).
    pub icon: String,
    pub picture: Picture,
    pub standing: Standing,
}

/// One app of the compositor's `apps` topic, as it sent it.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    id: String,
    title: String,
    picture: Picture,
    standing: Standing,
    /// The app that was in front when the screen opened.
    front: bool,
}

fn parse_app(a: &Value) -> Option<Listed> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("");
    let flag = |k: &str| a.get(k).and_then(Value::as_bool) == Some(true);
    let id = s("id");
    if id.is_empty() {
        return None;
    }
    let standing = match (s("state"), flag("hung")) {
        ("quitting", _) => Standing::Closing,
        ("stuck", _) => Standing::Stuck,
        (_, true) => Standing::Hung,
        _ => Standing::Fine,
    };
    let picture = if matches!(s("picture"), "live" | "thumb") { Picture::Shown } else { Picture::Tile };
    Some(Listed { id: id.to_string(), title: s("title").to_string(), picture, standing, front: flag("fg") })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mem {
    pub total_kb: u64,
    pub avail_kb: u64,
}

impl Mem {
    pub fn used_kb(self) -> u64 {
        self.total_kb.saturating_sub(self.avail_kb)
    }
}

/// A sample of the compositor's `stats`: what is sent is shown and nothing is made up for what is not (a missing reading is `None`).
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    /// Each core's load in percent.
    pub cpu: Vec<f32>,
    pub mem: Option<Mem>,
    pub gpu: Option<f32>,
    pub temp_c: Option<f32>,
    /// The panel refresher is running: the TV must not be restarted now.
    pub refresher: bool,
}

impl Stats {
    fn parse(data: &Value) -> Self {
        let percent = |v: &Value| v.as_f64().map(|p| p.clamp(0.0, 100.0) as f32);
        let kb = |k: &str| data.pointer(&format!("/mem/{k}")).and_then(Value::as_u64);
        Self {
            cpu: data.get("cpu").and_then(Value::as_array).map(|a| a.iter().filter_map(percent).collect()).unwrap_or_default(),
            mem: kb("totalKb").zip(kb("availKb")).filter(|(total, _)| *total > 0).map(|(total_kb, avail_kb)| Mem { total_kb, avail_kb }),
            gpu: data.get("gpu").and_then(percent),
            temp_c: data.get("tempC").and_then(Value::as_f64).map(|t| t as f32),
            refresher: data.get("refresher").and_then(Value::as_bool) == Some(true),
        }
    }
}

/// Where the focus is on the screen itself: on the cards (the one at [`View::index`]), or on one of the two buttons below them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Cards,
    Close,
    System,
}

/// What the System sheet offers, in the order of its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RestartLumatv,
    LgHome,
    Reboot,
}

pub const ACTIONS: [Action; 3] = [Action::RestartLumatv, Action::LgHome, Action::Reboot];

impl Action {
    /// The compositor's name for it.
    pub fn wire(self) -> &'static str {
        match self {
            Action::RestartLumatv => "restart-lumatv",
            Action::LgHome => "lg-home",
            Action::Reboot => "reboot",
        }
    }
}

/// What is being asked before it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Every open app (how many).
    CloseAll(usize),
    System(Action),
}

/// Why the compositor would not do a system action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Refresher,
    Busy,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskState {
    Asking,
    /// The compositor took the action: nothing more can be done until the TV or the interface goes down.
    Working,
    Refused(Reason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AskView {
    pub kind: Confirm,
    /// The button in focus: 0 Cancel, 1 the action.
    pub choice: usize,
    pub state: AskState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// The screen is up (open, and not the compositor's basic one).
    pub up: bool,
    pub entries: Vec<Entry>,
    pub first: usize,
    pub index: usize,
    pub focus: Focus,
    /// `None` until the first sample.
    pub stats: Option<Stats>,
    /// The System sheet is open, on this row.
    pub sheet: Option<usize>,
    pub ask: Option<AskView>,
    /// The cards that were sent away and have not left the list yet: they fly up instead of staying where they were.
    pub sent: Vec<String>,
}

impl View {
    /// Whether a System choice is off: the TV is not restarted while the panel refresher runs.
    pub fn blocked(&self, action: Action) -> bool {
        action == Action::Reboot && self.stats.as_ref().is_some_and(|s| s.refresher)
    }
}

/// The tag of the apps' subscription (the icons and names).
pub const APPS_TAG: u64 = 0x5100_0200;
/// The tag of the compositor's answer to a system action.
pub const SYSTEM_TAG: u64 = 0x5100_0300;
/// A surface that takes the keys and gets none for this long closes: a remote cannot stay behind it for ever (and a still picture of an
/// app is not left on a panel that is burning in).
pub const IDLE_MS: u64 = 45_000;
/// How long the picture of a card sent away stays placed at its flown rectangle before it is left out and fades where it is.
pub const FLY_MS: u64 = 220;
/// A card that was sent away and is still in the list after this long is back (the app did not go).
const DEPART_MS: u64 = 6_000;
/// A system action the compositor took ends the interface or the TV; when neither has happened by then, it did not start.
const SYSTEM_MS: u64 = 15_000;

/// Where a card was when it was sent away.
#[derive(Clone, Copy, Debug)]
struct Place {
    index: usize,
    n: usize,
    first: usize,
    focused: bool,
    /// The compositor has a picture of it in the window to fly.
    drawn: bool,
}

struct Departing {
    id: String,
    at: Place,
    since: u64,
}

struct Asked {
    kind: Confirm,
    choice: usize,
    state: AskState,
    /// When `Working` ends if nothing did.
    until: u64,
}

fn post(body: Value) -> Effect {
    Effect::Channel { path: PATH.into(), body }
}

pub struct Switcher {
    pub apps: Apps,
    list: Vec<Listed>,
    open: bool,
    /// The compositor's own basic screen is in charge (`mode: fallback`).
    fallback: bool,
    /// The opening's number: a new one is a new layout.
    seen_opening: i64,
    /// Nothing has been pressed since it opened: a list that arrives late still starts on the app that was in front.
    fresh: bool,
    focus: Focus,
    /// The card in focus, or the one the focus returns to.
    index: usize,
    first: usize,
    /// The System sheet's row.
    sheet: Option<usize>,
    ask: Option<Asked>,
    stats: Option<Stats>,
    departing: Vec<Departing>,
    idle_due: Option<u64>,
    /// The latest time any call was told.
    clock: u64,
    last_layout: String,
}

impl Default for Switcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Switcher {
    pub fn new() -> Self {
        Self {
            apps: Apps::new(APPS_TAG),
            list: Vec::new(),
            open: false,
            fallback: false,
            seen_opening: -1,
            fresh: false,
            focus: Focus::Cards,
            index: 0,
            first: 0,
            sheet: None,
            ask: None,
            stats: None,
            departing: Vec::new(),
            idle_due: None,
            clock: 0,
            last_layout: String::new(),
        }
    }

    pub fn subscribe(&self) -> Vec<Effect> {
        self.apps.subscribe()
    }

    /// Open, and this is the one drawing it: the compositor's basic screen draws itself and takes its own keys.
    fn shown(&self) -> bool {
        self.open && !self.fallback
    }

    /// The apps that are on the screen: those the compositor lists, less the cards sent away.
    fn listed(&self) -> impl Iterator<Item = &Listed> + '_ {
        self.list.iter().filter(|a| !self.departing.iter().any(|d| d.id == a.id))
    }

    /// The cards: each running app with the name and icon the launch points give it, else the compositor's own title.
    pub fn entries(&self) -> Vec<Entry> {
        self.listed()
            .map(|a| {
                let known = self.apps.by_id(&a.id);
                let title = known.map(|k| k.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| if a.title.is_empty() { a.id.clone() } else { a.title.clone() });
                Entry { id: a.id.clone(), title, icon: known.map(|k| k.icon.clone()).unwrap_or_default(), picture: a.picture, standing: a.standing }
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.listed().count()
    }

    pub fn is_open(&self) -> bool {
        self.shown()
    }

    /// Whether it takes the keys.
    pub fn wants(&self) -> bool {
        self.shown()
    }

    fn touch(&mut self, now: u64) {
        self.clock = now;
        self.idle_due = Some(now + IDLE_MS);
    }

    /// The id of the card the focus is on or returns to.
    fn focused_id(&self) -> Option<String> {
        self.listed().nth(self.index).map(|a| a.id.clone())
    }

    /// The list or the cards changed: the focus stays on the app it was on (else on the card that took its place), inside the list, and
    /// the window follows it. A list that arrives before anything is pressed starts again on the app that was in front.
    fn settle(&mut self, was: Option<String>) {
        let n = self.count();
        self.index = if self.fresh && self.shown() {
            self.listed().position(|a| a.front).unwrap_or(0)
        } else if let Some(i) = was.and_then(|id| self.listed().position(|a| a.id == id)) {
            i
        } else {
            self.index.min(n.saturating_sub(1))
        };
        self.first = geo::window_start(n, self.index, self.first);
        if n == 0 && self.focus != Focus::System {
            self.focus = Focus::System;
        }
        if self.fresh && self.shown() {
            self.focus = if n == 0 { Focus::System } else { Focus::Cards };
        }
        match self.ask.as_mut() {
            Some(Asked { kind: Confirm::CloseAll(_), .. }) if n == 0 => self.ask = None,
            Some(Asked { kind: Confirm::CloseAll(count), .. }) => *count = n,
            _ => {}
        }
    }

    /// The screen has just come up.
    fn begin(&mut self, now: u64) {
        self.fresh = true;
        self.last_layout.clear();
        self.stats = None;
        self.sheet = None;
        self.ask = None;
        self.departing.clear();
        self.index = 0;
        self.first = 0;
        self.settle(None);
        self.touch(now);
    }

    fn end(&mut self) {
        self.idle_due = None;
        self.fresh = false;
        self.sheet = None;
        self.ask = None;
        self.departing.clear();
    }

    /// The System sheet's rows that can be chosen now: the restart of the TV waits for the panel refresher.
    fn row_enabled(&self, row: usize) -> bool {
        !(ACTIONS[row] == Action::Reboot && self.stats.as_ref().is_some_and(|s| s.refresher))
    }

    /// The row `dir` rows from `row` that can be chosen, or `row` when there is none.
    fn row_from(&self, row: usize, dir: isize) -> usize {
        let mut at = row;
        loop {
            let next = at as isize + dir;
            if !(0..ACTIONS.len() as isize).contains(&next) {
                return row;
            }
            at = next as usize;
            if self.row_enabled(at) {
                return at;
            }
        }
    }

    /// A topic of the compositor: the list, whether it is open, and how the TV is doing. Returns whether the view changed.
    pub fn topic(&mut self, topic: &str, data: &Value, now: u64) -> bool {
        match topic {
            "apps" => {
                let list: Vec<Listed> = data.as_array().map(|a| a.iter().filter_map(parse_app).collect()).unwrap_or_default();
                if list == self.list {
                    return false;
                }
                let was = self.focused_id();
                self.list = list;
                // a card that left the list, or that did not quit in time, is no longer on its way out
                let list = &self.list;
                self.departing.retain(|d| list.iter().any(|a| a.id == d.id && a.standing != Standing::Stuck));
                self.settle(was);
                true
            }
            "switcher" => {
                let was = self.shown();
                self.open = data.get("open").and_then(Value::as_bool) == Some(true);
                self.fallback = data.get("mode").and_then(Value::as_str) == Some("fallback");
                let n = data.get("n").and_then(Value::as_i64).unwrap_or(0);
                match (was, self.shown()) {
                    (false, true) => self.begin(now),
                    (true, false) => self.end(),
                    _ => {}
                }
                if self.shown() && n != self.seen_opening {
                    self.last_layout.clear();
                }
                self.seen_opening = n;
                was || self.shown()
            }
            "stats" => {
                let stats = Stats::parse(data);
                if self.stats.as_ref() == Some(&stats) {
                    return false;
                }
                self.stats = Some(stats);
                if let Some(row) = self.sheet.filter(|r| !self.row_enabled(*r)) {
                    self.sheet = Some(self.row_from(row, -1));
                }
                true
            }
            _ => false,
        }
    }

    /// The places of the cards as the compositor takes them, when they changed since the last sent (one message for the changes of one
    /// turn). `top` is where the row stands, `scrim` the colour behind the pictures as `{r, g, b, a}` in 0 to 1, `ms` how long the cards
    /// take to move (and `reduced` says the motion is a fade), `palette` the theme's colours for the compositor's basic screen. A card
    /// that was sent away is named at its flown rectangle until [`FLY_MS`] is up, then left out: the compositor fades it where it is, which
    /// is all that reduced motion does, so it is left out at once.
    pub fn layout(&mut self, now: u64, top: f32, scrim: Value, reduced: bool, ms: u32, palette: Value) -> Option<Effect> {
        if !self.shown() {
            return None;
        }
        let doc = json!({"op": "layout", "cards": self.placed(now, top, reduced), "scrim": scrim, "ms": ms, "reduced": reduced, "palette": palette});
        let text = doc.to_string();
        if text == self.last_layout {
            return None;
        }
        self.last_layout = text;
        Some(post(doc))
    }

    /// The compositor's half of a layout: the cards in the window whose picture it draws.
    fn placed(&self, now: u64, top: f32, reduced: bool) -> Vec<Value> {
        let entries = self.entries();
        let n = entries.len();
        let card = |id: &str, r: geo::Rect| json!({"id": id, "x": r.x, "y": r.y, "w": r.w, "h": r.h, "r": r.r});
        let mut cards: Vec<Value> = entries
            .iter()
            .enumerate()
            .filter(|(i, e)| e.picture == Picture::Shown && geo::in_window(*i, self.first))
            .map(|(i, e)| card(&e.id, geo::pose(geo::card(i, n, self.first, top), self.focus == Focus::Cards && i == self.index)))
            .collect();
        for d in self.departing.iter().filter(|d| !reduced && d.at.drawn && now < d.since + FLY_MS) {
            let at = geo::card(d.at.index, d.at.n, d.at.first, top);
            cards.push(card(&d.id, geo::flown(geo::pose(at, d.at.focused))));
        }
        cards
    }

    fn activate(&self, i: usize) -> Vec<Effect> {
        match self.listed().nth(i) {
            Some(a) if a.standing != Standing::Closing => vec![post(json!({"op": "activate", "id": a.id}))],
            _ => Vec::new(),
        }
    }

    /// Up on a card. An app that is fine or hung is quit and its card sent away at once; one that did not quit stays and is forced; one that
    /// is closing is left to it.
    fn quit(&mut self, i: usize, now: u64) -> Vec<Effect> {
        let n = self.count();
        let Some(a) = self.listed().nth(i).cloned() else { return Vec::new() };
        match a.standing {
            Standing::Closing => Vec::new(),
            Standing::Stuck => vec![post(json!({"op": "quit", "id": a.id, "force": true}))],
            Standing::Fine | Standing::Hung => {
                let at = Place { index: i, n, first: self.first, focused: self.focus == Focus::Cards && i == self.index, drawn: a.picture == Picture::Shown && geo::in_window(i, self.first) };
                self.departing.push(Departing { id: a.id.clone(), at, since: now });
                self.settle(None);
                vec![post(json!({"op": "quit", "id": a.id}))]
            }
        }
    }

    pub fn close_effect(&self) -> Effect {
        post(json!({"op": "close"}))
    }

    /// Asks the compositor to open it (it answers by opening, and the `switcher` topic says so).
    pub fn open_effect(&self) -> Effect {
        post(json!({"op": "open"}))
    }

    fn ask(&mut self, kind: Confirm) {
        self.ask = Some(Asked { kind, choice: 0, state: AskState::Asking, until: 0 });
    }

    /// OK on the question's second button.
    fn confirm(&mut self, now: u64) -> Vec<Effect> {
        let Some(a) = self.ask.as_mut() else { return Vec::new() };
        match a.kind {
            Confirm::CloseAll(_) => {
                self.ask = None;
                self.focus = Focus::Close;
                vec![post(json!({"op": "quitall"}))]
            }
            Confirm::System(action) => {
                a.state = AskState::Working;
                a.until = now + SYSTEM_MS;
                vec![Effect::ChannelRequest { tag: SYSTEM_TAG, path: PATH.into(), body: json!({"op": "system", "action": action.wire()}) }]
            }
        }
    }

    /// The compositor's answer to a system action: `{ok: true}` is taken (the screen waits for the end of the interface or the TV), a refusal
    /// is said and the choice is the person's again.
    fn answered(&mut self, data: &Value) -> bool {
        let Some(a) = self.ask.as_mut().filter(|a| a.state == AskState::Working) else { return false };
        if data.get("ok").and_then(Value::as_bool) == Some(true) {
            return false;
        }
        a.state = AskState::Refused(match data.get("error").and_then(Value::as_str) {
            Some("refresher") => Reason::Refresher,
            Some("busy") => Reason::Busy,
            _ => Reason::Other,
        });
        a.choice = 0;
        true
    }

    /// A key of the remote while it is up.
    pub fn key(&mut self, k: K, now: u64) -> Out {
        if !self.shown() {
            return Out::default();
        }
        self.touch(now);
        self.fresh = false;
        if self.ask.is_some() {
            return Out::effects(self.key_ask(k, now));
        }
        if let Some(row) = self.sheet {
            self.key_sheet(k, row);
            return Out::default();
        }
        self.key_screen(k, now)
    }

    fn key_ask(&mut self, k: K, now: u64) -> Vec<Effect> {
        let Some(a) = self.ask.as_mut().filter(|a| a.state != AskState::Working) else { return Vec::new() };
        match k {
            K::Left => a.choice = 0,
            K::Right => a.choice = 1,
            K::Back => self.ask = None,
            K::Ok if a.choice == 0 => self.ask = None,
            K::Ok => return self.confirm(now),
            _ => {}
        }
        Vec::new()
    }

    fn key_sheet(&mut self, k: K, row: usize) {
        match k {
            K::Up => self.sheet = Some(self.row_from(row, -1)),
            K::Down => self.sheet = Some(self.row_from(row, 1)),
            K::Back => self.sheet = None,
            K::Ok if self.row_enabled(row) => self.ask(Confirm::System(ACTIONS[row])),
            _ => {}
        }
    }

    fn key_screen(&mut self, k: K, now: u64) -> Out {
        if k == K::Back {
            return Out::effects(vec![self.close_effect()]);
        }
        let n = self.count();
        match (self.focus, k) {
            (Focus::Cards, K::Left | K::Right) => {
                self.index = if k == K::Left { self.index.saturating_sub(1) } else { (self.index + 1).min(n.saturating_sub(1)) };
                self.first = geo::window_start(n, self.index, self.first);
            }
            (Focus::Cards, K::Ok) => return Out::effects(self.activate(self.index)),
            (Focus::Cards, K::Up) => return Out::effects(self.quit(self.index, now)),
            (Focus::Cards, K::Down) => self.focus = Focus::Close,
            (Focus::Close, K::Right) => self.focus = Focus::System,
            (Focus::Close, K::Ok) => self.ask(Confirm::CloseAll(n)),
            (Focus::System, K::Left) if n > 0 => self.focus = Focus::Close,
            (Focus::System, K::Ok) => self.sheet = Some(0),
            (Focus::Close | Focus::System, K::Up) if n > 0 => self.focus = Focus::Cards,
            _ => {}
        }
        Out::default()
    }

    /// Whether a click or a drag lands on the screen itself (the sheet and the questions take them while they are up).
    fn pointed(&mut self, now: u64) -> bool {
        if !self.shown() {
            return false;
        }
        self.touch(now);
        self.fresh = false;
        self.sheet.is_none() && self.ask.is_none()
    }

    /// A click on a card: it takes the focus and is opened.
    pub fn click_card(&mut self, i: usize, now: u64) -> Out {
        if !self.pointed(now) || i >= self.count() {
            return Out::default();
        }
        self.focus = Focus::Cards;
        self.index = i;
        self.first = geo::window_start(self.count(), i, self.first);
        Out::effects(self.activate(i))
    }

    /// A card dragged up past the threshold is quit, as Up quits it.
    pub fn drag_quit(&mut self, i: usize, now: u64) -> Out {
        if !self.pointed(now) || i >= self.count() {
            return Out::default();
        }
        self.focus = Focus::Cards;
        self.index = i;
        Out::effects(self.quit(i, now))
    }

    /// A click on Close all.
    pub fn click_close(&mut self, now: u64) -> Out {
        if self.pointed(now) && self.count() > 0 {
            self.focus = Focus::Close;
            self.ask(Confirm::CloseAll(self.count()));
        }
        Out::default()
    }

    /// A click on System.
    pub fn click_system(&mut self, now: u64) -> Out {
        if self.pointed(now) {
            self.focus = Focus::System;
            self.sheet = Some(0);
        }
        Out::default()
    }

    /// A click on a row of the sheet.
    pub fn click_row(&mut self, row: usize, now: u64) -> Out {
        if !self.shown() || self.ask.is_some() || self.sheet.is_none() || row >= ACTIONS.len() {
            return Out::default();
        }
        self.touch(now);
        self.sheet = Some(row);
        self.key(K::Ok, now)
    }

    /// A click on a question's button: 0 Cancel, 1 the action.
    pub fn click_choice(&mut self, choice: usize, now: u64) -> Out {
        if !self.shown() || self.ask.is_none() {
            return Out::default();
        }
        if let Some(a) = self.ask.as_mut() {
            a.choice = choice.min(1);
        }
        self.key(K::Ok, now)
    }

    /// A click outside what is in front is Back.
    pub fn click_outside(&mut self, now: u64) -> Out {
        self.key(K::Back, now)
    }

    pub fn tick(&mut self, now: u64) -> Out {
        self.clock = now;
        let before = self.departing.len();
        self.departing.retain(|d| now < d.since + DEPART_MS);
        if self.departing.len() != before {
            self.settle(None);
        }
        if let Some(a) = self.ask.as_mut().filter(|a| a.state == AskState::Working && now >= a.until) {
            a.state = AskState::Refused(Reason::Other);
            a.choice = 0;
        }
        if self.idle_due.is_some_and(|d| now >= d) {
            self.idle_due = None;
            if self.shown() {
                return Out::effects(vec![self.close_effect()]);
            }
        }
        Out::default()
    }

    /// The next time something of its own happens: the idle close, a card's flight ending, a card that did not go coming back, a system action
    /// that did not start.
    pub fn due_at(&self) -> Option<u64> {
        let mut due = self.idle_due;
        let later = self.departing.iter().flat_map(|d| [d.since + FLY_MS, d.since + DEPART_MS]).chain(self.ask.iter().filter(|a| a.state == AskState::Working).map(|a| a.until));
        for t in later.filter(|t| *t > self.clock) {
            due = Some(due.map_or(t, |d| d.min(t)));
        }
        due
    }

    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let (mut effects, mut changed) = self.apps.update(input);
        match input {
            Input::Topic { topic, data } => {
                changed |= self.topic(topic, data, now);
                // every sample is answered at once, from the loop that draws: a compositor that hears nothing for 2.5 s takes its own screen
                if topic == "stats" {
                    if let Some(n) = data.get("n").and_then(Value::as_u64) {
                        effects.push(post(json!({"op": "beat", "n": n})));
                    }
                }
            }
            Input::Fetched { tag: SYSTEM_TAG, data, .. } => changed |= self.answered(data),
            _ => {}
        }
        (effects, changed)
    }

    pub fn view(&self) -> View {
        View {
            up: self.shown(),
            entries: self.entries(),
            first: self.first,
            index: self.index,
            focus: self.focus,
            stats: self.stats.clone(),
            sheet: self.sheet,
            ask: self.ask.as_ref().map(|a| AskView { kind: a.kind, choice: a.choice, state: a.state }),
            sent: self.departing.iter().map(|d| d.id.clone()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP: f32 = 200.0;

    fn app(id: &str, picture: &str, state: &str) -> Value {
        json!({"id": id, "title": id.to_uppercase(), "picture": picture, "state": state, "hung": false, "fg": false})
    }

    fn five() -> Value {
        json!([app("a", "live", ""), app("b", "live", ""), app("c", "none", ""), app("d", "live", ""), app("e", "video", "")])
    }

    fn up(list: Value) -> Switcher {
        let mut s = Switcher::new();
        s.topic("apps", &list, 0);
        s.topic("switcher", &json!({"open": true, "from": "home", "n": 1, "mode": "ui"}), 0);
        s
    }

    fn ops(out: &Out) -> Vec<Value> {
        out.effects
            .iter()
            .filter_map(|e| match e {
                Effect::Channel { path, body } | Effect::ChannelRequest { path, body, .. } if path == PATH => Some(body.clone()),
                _ => None,
            })
            .collect()
    }

    fn press(s: &mut Switcher, keys: &[K]) -> Vec<Value> {
        keys.iter().flat_map(|k| ops(&s.key(*k, 10))).collect()
    }

    fn layout(s: &mut Switcher, now: u64) -> Option<Value> {
        match s.layout(now, TOP, json!({}), false, 180, json!({})) {
            Some(Effect::Channel { body, .. }) => Some(body),
            _ => None,
        }
    }

    fn ids(v: &Value) -> Vec<String> {
        v["cards"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn it_opens_on_the_app_that_was_in_front_even_when_the_list_comes_after_the_opening() {
        let mut s = Switcher::new();
        s.topic("switcher", &json!({"open": true, "n": 1}), 0);
        let front = json!([app("a", "live", ""), {"id": "b", "picture": "live", "fg": true}]);
        s.topic("apps", &front, 5);
        assert_eq!(s.view().index, 1, "the list that came after the opening is read as the opening's");
        s.key(K::Left, 10);
        s.topic("apps", &json!([app("a", "live", ""), app("b", "live", ""), {"id": "c", "picture": "live", "fg": true}]), 20);
        assert_eq!(s.view().index, 0, "a list that arrives once a key was pressed leaves the focus where it was put, whatever is in front");
    }

    #[test]
    fn left_and_right_move_ok_opens_and_the_window_follows_the_focus() {
        let mut s = up(five());
        assert_eq!(press(&mut s, &[K::Right, K::Right, K::Ok]), vec![json!({"op": "activate", "id": "c"})]);
        press(&mut s, &[K::Right, K::Right]);
        let v = s.view();
        assert_eq!((v.index, v.first), (4, 1), "the fifth card moves the window by one");
        assert_eq!(press(&mut s, &[K::Right, K::Right]), Vec::<Value>::new());
        assert_eq!(s.view().index, 4, "and the last card is the end");
        press(&mut s, &[K::Left, K::Left, K::Left, K::Left, K::Left]);
        assert_eq!((s.view().index, s.view().first), (0, 0));
    }

    #[test]
    fn up_quits_the_card_which_flies_up_while_its_neighbours_close_the_gap_and_the_focus_takes_a_neighbour() {
        let mut s = up(five());
        layout(&mut s, 0).expect("the first layout");
        press(&mut s, &[K::Right]);
        let out = s.key(K::Up, 100);
        assert_eq!(ops(&out), vec![json!({"op": "quit", "id": "b"})]);
        let v = s.view();
        assert_eq!(v.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "c", "d", "e"]);
        assert_eq!((v.index, v.focus, v.sent.clone()), (1, Focus::Cards, vec!["b".to_string()]), "the focus stays at the place, which is the next card now");
        let flying = layout(&mut s, 100).expect("the cards move at once");
        assert_eq!(ids(&flying), ["a", "d", "b"], "the pictures of the others close the gap and the one sent away is named where it flew to");
        let flown = &flying["cards"][2];
        assert_eq!((flown["x"].as_f64(), flown["y"].as_f64()), (Some(f64::from(geo::x0(5) + geo::PITCH)), Some(f64::from(TOP - geo::FLY))), "up from where it was, in focus");
        assert_eq!(layout(&mut s, 100 + FLY_MS - 1), None, "while it flies the places stand");
        let after = layout(&mut s, 100 + FLY_MS).expect("then it is left out and the compositor fades it");
        assert_eq!(ids(&after), ["a", "d"]);
        s.topic("apps", &json!([app("a", "live", ""), app("c", "none", ""), app("d", "live", ""), app("e", "video", "")]), 300);
        assert!(s.view().sent.is_empty(), "gone from the list is gone");
    }

    #[test]
    fn the_last_card_quit_leaves_the_empty_screen_with_the_focus_on_system() {
        let mut s = up(json!([app("a", "live", "")]));
        assert_eq!(press(&mut s, &[K::Up]), vec![json!({"op": "quit", "id": "a"})]);
        let v = s.view();
        assert_eq!((v.entries.len(), v.focus), (0, Focus::System));
        assert_eq!(press(&mut s, &[K::Up, K::Left, K::Down]), Vec::<Value>::new());
        assert_eq!(s.view().focus, Focus::System, "there is nothing else to go to");
    }

    #[test]
    fn a_hung_app_is_quit_like_any_a_stuck_one_is_forced_and_a_closing_one_is_left_to_it() {
        let hung = json!({"id": "h", "title": "H", "picture": "live", "state": "", "hung": true});
        let mut s = up(json!([hung, app("s", "live", "stuck"), app("q", "live", "quitting")]));
        assert_eq!(s.view().entries.iter().map(|e| e.standing).collect::<Vec<_>>(), [Standing::Hung, Standing::Stuck, Standing::Closing]);
        press(&mut s, &[K::Right]);
        assert_eq!(press(&mut s, &[K::Up]), vec![json!({"op": "quit", "id": "s", "force": true})]);
        assert_eq!(s.count(), 3, "a card that did not quit stays: it is forced and says so");
        press(&mut s, &[K::Right]);
        assert_eq!(press(&mut s, &[K::Up, K::Ok]), Vec::<Value>::new(), "a closing app is neither quit again nor opened");
        press(&mut s, &[K::Left, K::Left]);
        assert_eq!(press(&mut s, &[K::Up]), vec![json!({"op": "quit", "id": "h"})], "a hung app is quit at once, with the same op");
        assert_eq!(s.count(), 2);
    }

    #[test]
    fn a_card_sent_away_that_will_not_go_comes_back_stuck_or_after_a_while() {
        let mut s = up(json!([app("a", "live", ""), app("b", "live", "")]));
        press(&mut s, &[K::Up]);
        assert_eq!(s.count(), 1);
        s.topic("apps", &json!([app("a", "live", "stuck"), app("b", "live", "")]), 4_000);
        assert_eq!(s.count(), 2, "the card the compositor calls stuck is on the screen again, to be forced");
        press(&mut s, &[K::Right, K::Up]);
        assert_eq!(s.count(), 1);
        s.tick(10 + DEPART_MS);
        assert_eq!(s.count(), 2, "and one that is still listed when it should long be gone comes back");
    }

    #[test]
    fn down_goes_below_the_cards_up_comes_back_and_the_lower_buttons_are_two() {
        let mut s = up(five());
        press(&mut s, &[K::Right, K::Right, K::Down]);
        assert_eq!(s.view().focus, Focus::Close);
        press(&mut s, &[K::Left, K::Down]);
        assert_eq!(s.view().focus, Focus::Close, "nothing is left of Close all or below it");
        press(&mut s, &[K::Right]);
        assert_eq!(s.view().focus, Focus::System);
        press(&mut s, &[K::Right]);
        assert_eq!(s.view().focus, Focus::System);
        press(&mut s, &[K::Up]);
        let v = s.view();
        assert_eq!((v.focus, v.index), (Focus::Cards, 2), "back to the card it left");
    }

    #[test]
    fn with_nothing_open_the_screen_is_still_there_for_the_stats_and_system_and_has_no_close_all() {
        let mut s = up(json!([]));
        let v = s.view();
        assert_eq!((v.up, v.entries.len(), v.focus), (true, 0, Focus::System));
        assert_eq!(layout(&mut s, 0).map(|l| l["cards"].as_array().unwrap().len()), Some(0), "the compositor still gets its layout");
        press(&mut s, &[K::Left, K::Up]);
        assert_eq!(s.view().focus, Focus::System);
        press(&mut s, &[K::Ok]);
        assert_eq!(s.view().sheet, Some(0));
        s.topic("apps", &json!([app("a", "live", "")]), 20);
        assert_eq!(s.view().focus, Focus::System, "an app that starts under it does not take the focus from the person");
    }

    #[test]
    fn back_goes_up_one_step_and_closes_the_screen_from_the_top() {
        let mut s = up(five());
        press(&mut s, &[K::Down, K::Ok]);
        assert!(s.view().ask.is_some());
        assert_eq!(press(&mut s, &[K::Back]), Vec::<Value>::new());
        assert!(s.view().ask.is_none(), "Back cancels the question");
        press(&mut s, &[K::Right, K::Ok, K::Ok]);
        assert!(s.view().ask.is_some());
        press(&mut s, &[K::Back]);
        assert!(s.view().ask.is_none() && s.view().sheet == Some(0), "from a question Back is to the sheet");
        press(&mut s, &[K::Back]);
        assert!(s.view().sheet.is_none() && s.view().up, "and from the sheet to the screen");
        assert_eq!(press(&mut s, &[K::Back]), vec![json!({"op": "close"})]);
    }

    #[test]
    fn close_all_asks_with_cancel_in_focus_and_does_nothing_until_it_is_chosen() {
        let mut s = up(five());
        assert_eq!(press(&mut s, &[K::Down, K::Ok]), Vec::<Value>::new());
        let ask = s.view().ask.expect("a question");
        assert_eq!((ask.kind, ask.choice, ask.state), (Confirm::CloseAll(5), 0, AskState::Asking));
        assert_eq!(press(&mut s, &[K::Ok]), Vec::<Value>::new(), "OK on Cancel cancels");
        assert!(s.view().ask.is_none());
        assert_eq!(press(&mut s, &[K::Ok, K::Right, K::Ok]), vec![json!({"op": "quitall"})]);
        assert!(s.view().ask.is_none() && s.view().focus == Focus::Close, "it asked, it did, and the screen stays");
    }

    #[test]
    fn the_system_sheet_has_three_rows_and_each_asks_with_cancel_in_focus_before_it_posts_its_action() {
        for (rows, action) in [(0, "restart-lumatv"), (1, "lg-home"), (2, "reboot")] {
            let mut s = up(five());
            press(&mut s, &[K::Down, K::Right, K::Ok]);
            assert_eq!(s.view().sheet, Some(0));
            for _ in 0..rows {
                press(&mut s, &[K::Down]);
            }
            assert_eq!(press(&mut s, &[K::Ok]), Vec::<Value>::new(), "choosing a row only asks");
            assert_eq!(s.view().ask.expect("a question").choice, 0, "the safe button has the focus");
            let sent = s.key(K::Right, 20);
            assert!(ops(&sent).is_empty());
            let sent = s.key(K::Ok, 30);
            assert_eq!(ops(&sent), vec![json!({"op": "system", "action": action})]);
            assert!(matches!(&sent.effects[..], [Effect::ChannelRequest { tag: SYSTEM_TAG, .. }]), "its answer is asked for");
            assert_eq!(s.view().ask.expect("still up").state, AskState::Working);
            assert_eq!(press(&mut s, &[K::Back, K::Left, K::Ok]), Vec::<Value>::new(), "nothing is taken back once it is asked");
        }
    }

    #[test]
    fn the_restart_of_the_tv_is_off_while_the_refresher_runs_and_a_refusal_says_why() {
        let mut s = up(five());
        s.topic("stats", &json!({"n": 1, "cpu": [1.0], "mem": {"totalKb": 100, "availKb": 50}, "gpu": null, "tempC": null, "refresher": true}), 5);
        press(&mut s, &[K::Down, K::Right, K::Ok, K::Down, K::Down, K::Down]);
        assert_eq!(s.view().sheet, Some(1), "the row is passed over");
        assert!(s.view().blocked(Action::Reboot) && !s.view().blocked(Action::LgHome));
        s.topic("stats", &json!({"n": 2, "cpu": [1.0], "mem": {"totalKb": 100, "availKb": 50}, "refresher": false}), 1_000);
        press(&mut s, &[K::Down, K::Ok, K::Right, K::Ok]);
        let refusal = |s: &mut Switcher, body: Value, status: u16| s.update(&Input::Fetched { tag: SYSTEM_TAG, status, data: body }, 2_000);
        let (_, changed) = refusal(&mut s, json!({"ok": true}), 200);
        assert!(!changed && s.view().ask.unwrap().state == AskState::Working, "taken: it waits for the end");
        refusal(&mut s, json!({"ok": false, "error": "refresher"}), 200);
        let ask = s.view().ask.unwrap();
        assert_eq!((ask.state, ask.choice), (AskState::Refused(Reason::Refresher), 0), "the reason, and the choice is back with the safe button");
        refusal(&mut s, json!({"ok": false, "error": "busy"}), 200);
        assert_eq!(s.view().ask.unwrap().state, AskState::Refused(Reason::Refresher), "an answer to nothing asked changes nothing");
        press(&mut s, &[K::Right, K::Ok]);
        refusal(&mut s, Value::Null, 0);
        assert_eq!(s.view().ask.unwrap().state, AskState::Refused(Reason::Other), "no answer is a refusal too");
        press(&mut s, &[K::Right, K::Ok]);
        refusal(&mut s, json!({"ok": false, "error": "busy"}), 200);
        assert_eq!(s.view().ask.unwrap().state, AskState::Refused(Reason::Busy));
    }

    #[test]
    fn a_system_action_that_was_taken_and_did_not_happen_says_so_after_a_while() {
        let mut s = up(five());
        press(&mut s, &[K::Down, K::Right, K::Ok, K::Ok, K::Right, K::Ok]);
        assert_eq!(s.view().ask.unwrap().state, AskState::Working);
        assert_eq!(s.due_at(), Some(10 + SYSTEM_MS), "the guard is the next thing to happen");
        s.tick(10 + SYSTEM_MS);
        assert_eq!(s.view().ask.unwrap().state, AskState::Refused(Reason::Other));
    }

    #[test]
    fn every_stats_push_is_answered_with_its_number_at_once_whatever_the_mode() {
        let mut s = up(five());
        let beat = |s: &mut Switcher, n: u64| s.update(&Input::Topic { topic: "stats".into(), data: json!({"n": n, "cpu": [10.0, 20.0, 30.0], "mem": {"totalKb": 1000, "availKb": 250, "cachedKb": 1}, "gpu": 40, "tempC": 55, "mhz": 1000, "refresher": false}) }, 50);
        let (fx, changed) = beat(&mut s, 7);
        assert!(changed && fx.iter().any(|e| matches!(e, Effect::Channel { body, .. } if *body == json!({"op": "beat", "n": 7}))));
        let (fx, changed) = beat(&mut s, 8);
        assert!(!changed && fx.iter().any(|e| matches!(e, Effect::Channel { body, .. } if *body == json!({"op": "beat", "n": 8}))), "the same readings change nothing and are still answered");
        let stats = s.view().stats.expect("a sample");
        assert_eq!((stats.cpu.clone(), stats.mem.map(Mem::used_kb), stats.gpu, stats.temp_c), (vec![10.0, 20.0, 30.0], Some(750), Some(40.0), Some(55.0)));
        s.topic("switcher", &json!({"open": true, "n": 1, "mode": "fallback"}), 60);
        assert!(beat(&mut s, 9).0.iter().any(|e| matches!(e, Effect::Channel { body, .. } if body["n"] == 9)), "the basic screen's samples are answered as well");
    }

    #[test]
    fn readings_that_are_not_sent_are_not_made_up_and_the_first_sample_is_awaited() {
        let mut s = up(five());
        assert_eq!(s.view().stats, None, "placeholders until it comes");
        s.topic("stats", &json!({"n": 1, "cpu": [150.0, -3.0, "x"], "mem": {"totalKb": 0, "availKb": 0}, "gpu": null, "tempC": null, "mhz": null}), 5);
        let st = s.view().stats.unwrap();
        assert_eq!((st.cpu.clone(), st.mem, st.gpu, st.temp_c, st.refresher), (vec![100.0, 0.0], None, None, None, false));
        s.topic("switcher", &json!({"open": false, "n": 1}), 6);
        s.topic("switcher", &json!({"open": true, "n": 2}), 7);
        assert_eq!(s.view().stats, None, "the next time it opens starts from none");
    }

    #[test]
    fn in_the_compositors_basic_screen_nothing_is_drawn_no_key_is_taken_and_no_layout_is_sent() {
        let mut s = up(five());
        s.topic("switcher", &json!({"open": true, "from": "home", "n": 1, "mode": "fallback"}), 20);
        assert!(!s.is_open() && !s.wants() && !s.view().up);
        assert_eq!(s.key(K::Down, 30), Out::default());
        assert_eq!(layout(&mut s, 30), None);
        s.topic("switcher", &json!({"open": true, "n": 1, "mode": "ui"}), 40);
        assert!(s.wants() && layout(&mut s, 40).is_some(), "when the interface is in charge again the layout goes out again");
    }

    #[test]
    fn the_layout_carries_the_pictures_the_motion_the_palette_and_is_sent_when_it_changed_or_a_new_opening_began() {
        let mut s = up(five());
        let go = |s: &mut Switcher, now: u64| s.layout(now, TOP, json!({"r": 0, "g": 0, "b": 0, "a": 0.97}), true, 120, json!({"panel": [1, 2, 3]}));
        let Some(Effect::Channel { path, body }) = go(&mut s, 0) else { panic!("a layout") };
        assert_eq!(path, "/v1/switcher");
        assert_eq!((body["op"].as_str(), body["ms"].as_u64(), body["reduced"].as_bool(), body["palette"]["panel"].clone()), (Some("layout"), Some(120), Some(true), json!([1, 2, 3])));
        assert_eq!(body["cards"].as_array().map(Vec::len), Some(3), "the pictures the compositor draws: the tile and the video plane have none");
        let infocus = body["cards"][0].clone();
        assert_eq!((infocus["x"].as_f64(), infocus["w"].as_f64(), infocus["r"].as_f64()), (Some(112.0), Some(400.0), Some(24.0)), "the card in focus fills its footprint");
        let rest = body["cards"][1].clone();
        assert_eq!((rest["x"].as_f64(), rest["y"].as_f64(), rest["w"].as_f64(), rest["h"].as_f64(), rest["r"].as_f64()), (Some(112.0 + 432.0 + 32.0), Some(f64::from(TOP) + 18.0), Some(336.0), Some(189.0), Some(20.0)), "the others rest");
        assert!(go(&mut s, 0).is_none(), "the same places are not sent again");
        press(&mut s, &[K::Down]);
        let Some(Effect::Channel { body: lower, .. }) = go(&mut s, 0) else { panic!("the focus left the cards") };
        assert!(lower["cards"].as_array().unwrap().iter().all(|c| c["w"] == 336.0), "none is in focus below");
        assert!(go(&mut s, 0).is_none());
        s.topic("switcher", &json!({"open": true, "n": 2}), 5);
        assert!(go(&mut s, 5).is_some(), "a new opening is a new layout");
    }

    #[test]
    fn a_click_is_what_its_key_is_and_a_card_dragged_up_is_quit_while_the_sheet_and_questions_take_the_clicks() {
        let mut s = up(five());
        assert_eq!(ops(&s.click_card(3, 10)), vec![json!({"op": "activate", "id": "d"})]);
        assert_eq!(s.view().index, 3, "the card clicked has the focus");
        assert_eq!(ops(&s.drag_quit(1, 20)), vec![json!({"op": "quit", "id": "b"})]);
        s.click_close(30);
        assert_eq!(s.view().ask.map(|a| a.kind), Some(Confirm::CloseAll(4)));
        assert!(s.click_card(0, 40).effects.is_empty() && s.drag_quit(0, 40).effects.is_empty(), "the question has the clicks");
        assert_eq!(ops(&s.click_choice(1, 50)), vec![json!({"op": "quitall"})]);
        s.click_system(60);
        assert_eq!(s.view().sheet, Some(0));
        s.click_row(1, 70);
        assert_eq!(s.view().ask.map(|a| a.kind), Some(Confirm::System(Action::LgHome)));
        s.click_outside(80);
        assert!(s.view().ask.is_none() && s.view().sheet == Some(1), "outside is Back, one step");
        s.click_outside(90);
        assert!(s.view().sheet.is_none());
        assert_eq!(ops(&s.click_outside(100)), vec![json!({"op": "close"})]);
    }

    #[test]
    fn the_focus_stays_on_its_app_when_the_list_changes_around_it() {
        let mut s = up(five());
        press(&mut s, &[K::Right, K::Right]);
        s.topic("apps", &json!([app("b", "live", ""), app("c", "none", ""), app("d", "live", "")]), 20);
        assert_eq!(s.view().index, 1, "c is now the second card and has kept the focus");
        s.topic("apps", &json!([app("b", "live", ""), app("d", "live", "")]), 30);
        assert_eq!(s.view().index, 1, "its place goes to the next one");
        s.topic("apps", &json!([app("b", "live", "")]), 40);
        assert_eq!(s.view().index, 0);
    }

    #[test]
    fn it_closes_itself_after_45_seconds_without_a_key_and_a_key_starts_the_wait_again() {
        let mut s = up(five());
        assert_eq!(s.due_at(), Some(IDLE_MS));
        s.key(K::Right, 40_000);
        assert_eq!(s.due_at(), Some(40_000 + IDLE_MS));
        assert_eq!(s.tick(IDLE_MS), Out::default());
        let out = s.tick(40_000 + IDLE_MS);
        assert_eq!(ops(&out), vec![json!({"op": "close"})]);
        s.topic("switcher", &json!({"open": false, "n": 1}), 90_000);
        assert_eq!(s.due_at(), None);
    }
}
