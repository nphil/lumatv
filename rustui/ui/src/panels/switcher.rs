//! The app switcher (`docs/mocks/switcher`, `panels/Switcher.qml` and `SwitcherLayout.js` of the QML shell), the one list of what is open with
//! two doors: a second press of Home (the compositor opens it) and Back from the top of Home (Home asks). Each running app is a live
//! picture of its own window, which the compositor draws under the shell's layers where [`placed`] says (`src/switcher.rs` of the
//! compositor); this surface draws the rest: the names, the focus, Quit, Quit all, and for an app whose picture is on the video plane (no
//! picture is taken of it) its icon on a tile. Nothing here covers a picture but the focus ring and the dim of the cards that are not in
//! focus, so the pictures show through a window that is otherwise clear.
//!
//! Keys: Left and Right move between the cards, Down goes to a card's own Quit, Up to the header's Quit all; OK opens the card, quits, or
//! asks; Back (and the compositor's Home) closes. A pointer acts by clicking; outside the cards a click is Back. Quit all asks first.
//!
//! The compositor's `apps` topic is the list ([{id, title, picture: live|video|none, w, h, state}]) and its `switcher` topic says when it
//! is open ({open, from, n}); the cards' places are sent whenever the list or the window of cards changes, as one message
//! (`POST /v1/switcher {op: "layout", ...}`) the compositor moves its pictures to. 1920x1080 TV pixels.

use super::{K, Out};
use crate::host::apps::Apps;
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// A picture is 400 by 225 (16:9) with corners of 22; four fit side by side with 32 between and start at x 112 (fewer are centred);
/// pictures sit at y 340. More than four are a window of four that moves by a card when the focus leaves it.
pub mod geo {
    pub const W: f32 = 400.0;
    pub const H: f32 = 225.0;
    pub const GAP: f32 = 32.0;
    pub const TOP: f32 = 340.0;
    pub const RADIUS: f32 = 22.0;
    pub const VISIBLE: usize = 4;
    pub const SCREEN: f32 = 1920.0;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Level {
        Card,
        Quit,
        All,
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

    /// Card `i`'s rectangle (x, y, w, h, r) when the window starts at `first`: cards past either end carry on along the same row, out of sight.
    pub fn card(i: usize, n: usize, first: usize) -> (f32, f32, f32, f32, f32) {
        (x0(n) + (i as f32 - first as f32) * (W + GAP), TOP, W, H, RADIUS)
    }

    /// The focus's neighbours, as the key moves it: the answer is the same place when the key leads nowhere. Up goes toward the header
    /// (Quit all), down toward the card's own Quit.
    pub fn step(level: Level, index: usize, dir: super::K, n: usize) -> (Level, usize) {
        use super::K;
        if n == 0 {
            return (level, index);
        }
        let i = index.min(n - 1);
        match dir {
            K::Left if level != Level::All => (level, i.saturating_sub(1)),
            K::Right if level != Level::All => (level, (i + 1).min(n - 1)),
            K::Down => match level {
                Level::Card => (Level::Quit, i),
                Level::All => (Level::Card, i),
                Level::Quit => (level, i),
            },
            K::Up => match level {
                Level::Quit => (Level::Card, i),
                Level::Card => (Level::All, i),
                Level::All => (level, i),
            },
            _ => (level, i),
        }
    }
}

pub use geo::Level;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    /// The app's icon file ("" for none).
    pub icon: String,
    /// `live`, `video`, `none`.
    pub picture: String,
    /// `""`, `quitting`, `stuck`.
    pub state: String,
}

/// The compositor's half of a layout: the cards in the window whose picture it draws (an app whose picture is on the video plane has none).
pub fn placed(entries: &[Entry], first: usize) -> Vec<Value> {
    let n = entries.len();
    entries
        .iter()
        .enumerate()
        .filter(|(i, e)| geo::in_window(*i, first) && e.picture == "live")
        .map(|(i, e)| {
            let (x, y, w, h, r) = geo::card(i, n, first);
            json!({"id": e.id, "x": x, "y": y, "w": w, "h": h, "r": r})
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub entries: Vec<Entry>,
    pub first: usize,
    pub index: usize,
    pub level: Level,
    pub asking: bool,
    pub ask_index: usize,
    /// The row is on the move: the ring waits for it.
    pub moving: bool,
}

/// The tag of the apps' subscription (the icons and names).
pub const APPS_TAG: u64 = 0x5100_0200;
/// A surface that takes the keys and gets none for this long closes: a remote cannot stay behind it for ever (and a still picture of an
/// app is not left on a panel that is burning in).
pub const IDLE_MS: u64 = 45_000;
/// How long the row is on the move after the list or the window changed.
pub const SLIDE_MS: u64 = 200;

pub struct Switcher {
    pub apps: Apps,
    list: Vec<Value>,
    open: bool,
    /// The opening's number: a new one is a new layout.
    seen_opening: i64,
    level: Level,
    index: usize,
    first: usize,
    asking: bool,
    ask_index: usize,
    idle_due: Option<u64>,
    slide_until: u64,
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
            seen_opening: -1,
            level: Level::Card,
            index: 0,
            first: 0,
            asking: false,
            ask_index: 0,
            idle_due: None,
            slide_until: 0,
            last_layout: String::new(),
        }
    }

    pub fn subscribe(&self) -> Vec<Effect> {
        self.apps.subscribe()
    }

    /// The list: each running app with the name and icon the launch points give it, else the compositor's own title.
    pub fn entries(&self) -> Vec<Entry> {
        self.list
            .iter()
            .map(|a| {
                let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let id = s("id");
                let known = self.apps.by_id(&id);
                let title = known.map(|k| k.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| if s("title").is_empty() { id.clone() } else { s("title") });
                let picture = if s("picture").is_empty() { "none".to_string() } else { s("picture") };
                Entry { icon: known.map(|k| k.icon.clone()).unwrap_or_default(), title, picture, state: s("state"), id }
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether it takes the keys.
    pub fn wants(&self) -> bool {
        self.open
    }

    fn entry_state(&self, i: usize) -> String {
        self.list.get(i).and_then(|a| a.get("state")).and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn post(&self, body: Value) -> Effect {
        Effect::Channel { path: "/v1/switcher".into(), body }
    }

    fn touch(&mut self, now: u64) {
        self.idle_due = Some(now + IDLE_MS);
    }

    /// The count changed: the focus stays inside the list, the window follows it.
    fn recount(&mut self, now: u64) {
        let n = self.count();
        if self.index >= n {
            self.index = n.saturating_sub(1);
        }
        self.first = geo::window_start(n, self.index, self.first);
        if n == 0 {
            self.level = Level::Card;
            self.asking = false;
        }
        if self.open {
            self.slide_until = now + SLIDE_MS;
        }
    }

    /// A topic of the compositor: the list and whether it is open. Returns whether the view changed.
    pub fn topic(&mut self, topic: &str, data: &Value, now: u64) -> bool {
        match topic {
            "apps" => {
                let list = data.as_array().cloned().unwrap_or_default();
                if list == self.list {
                    return false;
                }
                let before = self.count();
                self.list = list;
                if self.count() != before {
                    self.recount(now);
                }
                true
            }
            "switcher" => {
                let open = data.get("open").and_then(Value::as_bool) == Some(true);
                let n = data.get("n").and_then(Value::as_i64).unwrap_or(0);
                let mut changed = false;
                if open && (!self.open || n != self.seen_opening) {
                    self.seen_opening = n;
                    self.last_layout.clear();
                }
                if open && !self.open {
                    self.index = 0;
                    self.first = 0;
                    self.level = Level::Card;
                    self.asking = false;
                    self.ask_index = 0;
                    self.slide_until = now + SLIDE_MS;
                    self.touch(now);
                    changed = true;
                }
                if !open && self.open {
                    self.idle_due = None;
                    changed = true;
                }
                self.open = open;
                changed
            }
            _ => false,
        }
    }

    /// The places of the cards as the compositor takes them, when they changed since the last sent (one message for the changes of one
    /// turn). `scrim` is the colour behind them, as `{r, g, b, a}` in 0 to 1; `reduced` says the motion is a fade.
    pub fn layout(&mut self, scrim: Value, reduced: bool) -> Option<Effect> {
        if !self.open {
            return None;
        }
        let doc = json!({"op": "layout", "cards": placed(&self.entries(), self.first), "scrim": scrim, "ms": if reduced { 120 } else { 200 }, "reduced": reduced});
        let text = doc.to_string();
        if text == self.last_layout {
            return None;
        }
        self.last_layout = text;
        Some(self.post(doc))
    }

    fn activate(&self, i: usize) -> Vec<Effect> {
        match self.list.get(i) {
            Some(a) if self.entry_state(i) != "quitting" => vec![self.post(json!({"op": "activate", "id": a.get("id").cloned().unwrap_or(Value::Null)}))],
            _ => Vec::new(),
        }
    }

    fn quit(&mut self, i: usize) -> Vec<Effect> {
        let out = match self.list.get(i) {
            Some(a) if self.entry_state(i) != "quitting" => vec![self.post(json!({"op": "quit", "id": a.get("id").cloned().unwrap_or(Value::Null)}))],
            _ => return Vec::new(),
        };
        self.level = Level::Card;
        out
    }

    fn quit_all(&mut self) -> Vec<Effect> {
        self.asking = false;
        self.level = Level::Card;
        vec![self.post(json!({"op": "quitall"}))]
    }

    pub fn close_effect(&self) -> Effect {
        self.post(json!({"op": "close"}))
    }

    /// A key of the remote while it is up.
    pub fn key(&mut self, k: K, now: u64) -> Out {
        self.touch(now);
        if self.asking {
            match k {
                K::Left | K::Right => self.ask_index = 1 - self.ask_index,
                K::Back => self.asking = false,
                K::Ok => {
                    if self.ask_index == 1 {
                        return Out::effects(self.quit_all());
                    }
                    self.asking = false;
                }
                _ => {}
            }
            return Out::default();
        }
        if k == K::Back {
            return Out::effects(vec![self.close_effect()]);
        }
        let n = self.count();
        if n == 0 {
            return if k == K::Ok { Out::effects(vec![self.close_effect()]) } else { Out::default() };
        }
        if k == K::Ok {
            match self.level {
                Level::All => {
                    self.asking = true;
                    self.ask_index = 0;
                    Out::default()
                }
                Level::Quit => Out::effects(self.quit(self.index)),
                Level::Card => Out::effects(self.activate(self.index)),
            }
        } else {
            let before = self.first;
            let (level, index) = geo::step(self.level, self.index, k, n);
            self.level = level;
            self.index = index;
            self.first = geo::window_start(n, self.index, self.first);
            if self.first != before {
                self.slide_until = now + SLIDE_MS;
            }
            Out::default()
        }
    }

    /// A click on a card's picture: it takes the focus and is opened. `quit`: on its Quit button instead.
    pub fn click_card(&mut self, i: usize, quit: bool, now: u64) -> Out {
        if self.asking {
            return Out::default();
        }
        self.touch(now);
        self.level = if quit { Level::Quit } else { Level::Card };
        self.index = i.min(self.count().saturating_sub(1));
        self.key(K::Ok, now)
    }

    /// A click on the header's Quit all.
    pub fn click_all(&mut self, now: u64) -> Out {
        if self.asking {
            return Out::default();
        }
        self.level = Level::All;
        self.key(K::Ok, now)
    }

    /// A click on the question's answers: 0 Cancel, 1 Quit all.
    pub fn click_ask(&mut self, answer: usize, now: u64) -> Out {
        self.ask_index = answer.min(1);
        self.key(K::Ok, now)
    }

    /// A click outside the cards is Back.
    pub fn click_outside(&mut self, now: u64) -> Out {
        self.key(K::Back, now)
    }

    pub fn tick(&mut self, now: u64) -> Out {
        if self.idle_due.is_some_and(|d| now >= d) {
            self.idle_due = None;
            if self.open {
                return Out::effects(vec![self.close_effect()]);
            }
        }
        Out::default()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.idle_due
    }

    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let (effects, changed) = self.apps.update(input);
        let mut changed = changed;
        if let Input::Topic { topic, data } = input {
            changed |= self.topic(topic, data, now);
        }
        (effects, changed)
    }

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn first(&self) -> usize {
        self.first
    }

    pub fn asking(&self) -> bool {
        self.asking
    }

    pub fn view(&self, now: u64) -> View {
        View { up: self.open, entries: self.entries(), first: self.first, index: self.index, level: self.level, asking: self.asking, ask_index: self.ask_index, moving: now < self.slide_until }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Value {
        json!([
            {"id": "plex", "title": "Plex", "picture": "live", "w": 1920, "h": 1080, "state": ""},
            {"id": "youtube.leanback.v4", "title": "YouTube", "picture": "live", "w": 1920, "h": 1080, "state": ""},
            {"id": "com.webos.app.hdmi2", "title": "HDMI 2", "picture": "video", "w": 1920, "h": 1080, "state": ""},
            {"id": "netflix", "title": "Netflix", "picture": "live", "w": 1920, "h": 1080, "state": ""}
        ])
    }

    fn opened(list: Value) -> Switcher {
        let mut s = Switcher::new();
        s.topic("apps", &list, 0);
        s.topic("switcher", &json!({"open": true, "from": "", "n": 1}), 0);
        s
    }

    fn scrim() -> Value {
        json!({"r": 0.07, "g": 0.055, "b": 0.1, "a": 0.973})
    }

    fn posts(out: &Out) -> Vec<Value> {
        out.effects.iter().filter_map(|e| if let Effect::Channel { body, .. } = e { Some(body.clone()) } else { None }).collect()
    }

    #[test]
    fn it_takes_the_keys_only_while_the_compositor_says_it_is_open() {
        let mut s = Switcher::new();
        s.topic("apps", &apps(), 0);
        assert!(!s.wants() && s.layout(scrim(), false).is_none(), "closed it takes no key and sends no place");
        s.topic("switcher", &json!({"open": true, "from": "", "n": 1}), 10);
        assert!(s.wants());
        assert_eq!((s.level(), s.index(), s.first(), s.asking()), (Level::Card, 0, 0, false), "its first card in focus");
    }

    #[test]
    fn the_places_of_the_cards_are_sent_once_only_the_apps_with_a_live_picture_have_one_and_a_move_inside_the_window_sends_nothing() {
        let mut s = opened(apps());
        let Some(Effect::Channel { path, body }) = s.layout(scrim(), false) else { panic!("a layout") };
        assert_eq!(path, "/v1/switcher");
        let cards: Vec<(String, i64, i64, i64, i64, i64)> = body["cards"].as_array().unwrap().iter().map(|c| (c["id"].as_str().unwrap().into(), c["x"].as_f64().unwrap() as i64, c["y"].as_f64().unwrap() as i64, c["w"].as_f64().unwrap() as i64, c["h"].as_f64().unwrap() as i64, c["r"].as_f64().unwrap() as i64)).collect();
        assert_eq!(cards, vec![("plex".into(), 112, 340, 400, 225, 22), ("youtube.leanback.v4".into(), 544, 340, 400, 225, 22), ("netflix".into(), 1408, 340, 400, 225, 22)], "the one on the video plane has a tile of its own");
        assert_eq!((body["ms"].clone(), body["reduced"].clone()), (json!(200), json!(false)));
        assert_eq!(body["scrim"]["a"], json!(0.973));
        assert!(s.layout(scrim(), false).is_none(), "the same places are not sent twice");
        s.key(K::Right, 100);
        assert_eq!((s.level(), s.index()), (Level::Card, 1));
        assert!(s.layout(scrim(), false).is_none(), "moving the focus inside the window changes no place: nothing is sent");
        // a new opening is a new layout
        s.topic("switcher", &json!({"open": false, "from": "", "n": 1}), 200);
        s.topic("switcher", &json!({"open": true, "from": "", "n": 2}), 300);
        assert!(s.layout(scrim(), false).is_some());
    }

    #[test]
    fn three_cards_are_centred_and_the_row_of_four_starts_at_x_112() {
        let three = json!([
            {"id": "plex", "picture": "live"}, {"id": "netflix", "picture": "live"}, {"id": "youtube.leanback.v4", "picture": "live"}
        ]);
        let mut s = opened(three);
        let Some(Effect::Channel { body, .. }) = s.layout(scrim(), false) else { panic!() };
        let xs: Vec<i64> = body["cards"].as_array().unwrap().iter().map(|c| c["x"].as_f64().unwrap() as i64).collect();
        assert_eq!(xs, vec![328, 760, 1192], "three cards, centred");
        assert_eq!(geo::x0(4), 112.0);
        assert_eq!(geo::x0(1), 760.0);
    }

    #[test]
    fn the_dpad_walks_the_cards_down_to_a_quit_and_up_to_quit_all() {
        let mut s = opened(apps());
        s.key(K::Right, 10);
        assert_eq!((s.level(), s.index()), (Level::Card, 1));
        s.key(K::Down, 20);
        assert_eq!((s.level(), s.index()), (Level::Quit, 1), "Down goes to the card's own Quit");
        s.key(K::Up, 30);
        s.key(K::Up, 40);
        assert_eq!((s.level(), s.index()), (Level::All, 1), "and Up from a card goes to the header's Quit all");
        s.key(K::Left, 45);
        assert_eq!(s.index(), 1, "Left and Right mean nothing on Quit all");
        s.key(K::Down, 50);
        assert_eq!((s.level(), s.index()), (Level::Card, 1));
    }

    #[test]
    fn ok_opens_the_card_quit_quits_it_and_back_closes() {
        let mut s = opened(apps());
        s.key(K::Right, 10);
        assert_eq!(posts(&s.key(K::Ok, 20)), vec![json!({"op": "activate", "id": "youtube.leanback.v4"})]);
        s.key(K::Down, 30);
        assert_eq!(posts(&s.key(K::Ok, 40)), vec![json!({"op": "quit", "id": "youtube.leanback.v4"})]);
        assert_eq!((s.level(), s.index()), (Level::Card, 1), "after a Quit the focus is on the card again");
        assert_eq!(posts(&s.key(K::Back, 50)), vec![json!({"op": "close"})]);
    }

    #[test]
    fn quit_all_asks_first_and_cancel_is_where_the_focus_starts() {
        let mut s = opened(apps());
        s.key(K::Up, 10);
        let out = s.key(K::Ok, 20);
        assert!(posts(&out).is_empty(), "nothing is quit by asking");
        assert!(s.asking());
        assert_eq!(s.view(30).ask_index, 0);
        s.key(K::Ok, 40);
        assert!(!s.asking(), "OK on Cancel leaves it as it was");
        assert_eq!((s.level(), s.index()), (Level::All, 0));
        s.key(K::Ok, 50);
        s.key(K::Right, 60);
        assert_eq!(posts(&s.key(K::Ok, 70)), vec![json!({"op": "quitall"})], "only the second yes quits them all");
        assert_eq!(s.level(), Level::Card);
        s.key(K::Up, 80);
        s.key(K::Ok, 90);
        let out = s.key(K::Back, 100);
        assert!(posts(&out).is_empty() && !s.asking(), "Back on the question is Cancel, not the switcher's own Back");
        assert_eq!(s.level(), Level::All);
    }

    #[test]
    fn more_than_four_apps_are_a_window_of_four_that_moves_a_card_at_a_time() {
        let many: Vec<Value> = (0..7).map(|i| json!({"id": format!("app{i}"), "title": format!("App {i}"), "picture": "live", "state": ""})).collect();
        let mut s = opened(Value::Array(many));
        let Some(Effect::Channel { body, .. }) = s.layout(scrim(), false) else { panic!() };
        let ids = |b: &Value| b["cards"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&body), vec!["app0", "app1", "app2", "app3"]);
        for t in 0..4 {
            s.key(K::Right, 100 + t * 20);
        }
        assert_eq!((s.level(), s.index(), s.first()), (Level::Card, 4, 1), "the fifth card is in focus and the window moved by one");
        let Some(Effect::Channel { body, .. }) = s.layout(scrim(), false) else { panic!("the places changed") };
        assert_eq!(ids(&body), vec!["app1", "app2", "app3", "app4"]);
        let xs: Vec<i64> = body["cards"].as_array().unwrap().iter().map(|c| c["x"].as_f64().unwrap() as i64).collect();
        assert_eq!(xs, vec![112, 544, 976, 1408], "the row moved a card to the left");
        assert!(s.view(150).moving && !s.view(1000).moving, "the row is on the move for a moment");
    }

    #[test]
    fn the_quit_of_a_card_that_is_closing_is_not_asked_again_and_a_stuck_one_can_be_tried_again() {
        let closing = json!([{"id": "plex", "title": "Plex", "picture": "live", "state": "quitting"}, {"id": "youtube.leanback.v4", "title": "YouTube", "picture": "live", "state": ""}]);
        let mut s = opened(closing);
        s.key(K::Down, 10);
        assert!(posts(&s.key(K::Ok, 20)).is_empty(), "an app that is closing is not asked to close again");
        assert_eq!(s.level(), Level::Quit);
        let stuck = json!([{"id": "plex", "title": "Plex", "picture": "live", "state": "stuck"}, {"id": "youtube.leanback.v4", "title": "YouTube", "picture": "live", "state": ""}]);
        s.topic("apps", &stuck, 30);
        assert_eq!(posts(&s.key(K::Ok, 40)), vec![json!({"op": "quit", "id": "plex"})], "one that did not close can be asked again");
    }

    #[test]
    fn reduced_motion_is_said_to_the_compositor() {
        let mut s = opened(apps());
        let Some(Effect::Channel { body, .. }) = s.layout(scrim(), true) else { panic!() };
        assert_eq!((body["reduced"].clone(), body["ms"].clone()), (json!(true), json!(120)));
    }

    #[test]
    fn nothing_open_says_so_ok_closes_it_and_the_focus_stays_inside_a_list_that_shrinks() {
        let mut s = opened(json!([]));
        assert_eq!(s.count(), 0);
        assert_eq!(posts(&s.key(K::Ok, 10)), vec![json!({"op": "close"})]);
        assert!(posts(&s.key(K::Right, 20)).is_empty());
        let mut s = opened(apps());
        s.key(K::Right, 10);
        s.key(K::Right, 20);
        s.key(K::Right, 30);
        assert_eq!(s.index(), 3);
        s.topic("apps", &json!([{"id": "plex", "picture": "live"}]), 40);
        assert_eq!((s.index(), s.first()), (0, 0), "the focus is clamped to the list");
        s.topic("apps", &json!([]), 50);
        assert_eq!((s.level(), s.asking()), (Level::Card, false));
    }

    #[test]
    fn an_idle_switcher_closes_itself_after_forty_five_seconds_and_a_key_starts_the_time_over() {
        let mut s = opened(apps());
        assert_eq!(s.due_at(), Some(45_000));
        s.key(K::Right, 20_000);
        assert_eq!(s.due_at(), Some(65_000));
        assert!(posts(&s.tick(64_999)).is_empty());
        assert_eq!(posts(&s.tick(65_000)), vec![json!({"op": "close"})]);
        s.topic("switcher", &json!({"open": false, "from": "", "n": 1}), 66_000);
        assert_eq!(s.due_at(), None);
    }

    #[test]
    fn the_name_and_icon_are_the_launch_points_and_a_click_acts_like_ok_on_what_it_is_on() {
        let mut s = opened(apps());
        s.apps.update(&Input::Luna { tag: APPS_TAG, payload: json!({"launchPoints": [{"id": "plex", "title": "Plex Media", "icon": "/media/plex.png"}]}) });
        let e = s.entries();
        assert_eq!((e[0].title.as_str(), e[0].icon.as_str()), ("Plex Media", "/media/plex.png"));
        assert_eq!((e[1].title.as_str(), e[1].icon.as_str()), ("YouTube", ""), "an app the catalogue does not know keeps the compositor's title");
        assert_eq!(posts(&s.click_card(1, false, 10)), vec![json!({"op": "activate", "id": "youtube.leanback.v4"})]);
        assert_eq!(posts(&s.click_card(3, true, 20)), vec![json!({"op": "quit", "id": "netflix"})]);
        assert!(posts(&s.click_all(30)).is_empty() && s.asking());
        assert!(posts(&s.click_card(0, false, 40)).is_empty(), "while asked, the cards take no click");
        assert_eq!(posts(&s.click_ask(1, 50)), vec![json!({"op": "quitall"})]);
        assert_eq!(posts(&s.click_outside(60)), vec![json!({"op": "close"})]);
    }

    #[test]
    fn the_window_never_goes_past_the_last_card_and_follows_the_focus_back() {
        assert_eq!(geo::window_start(7, 6, 0), 3);
        assert_eq!(geo::window_start(7, 2, 3), 2);
        assert_eq!(geo::window_start(3, 2, 0), 0);
        assert_eq!(geo::window_start(0, 0, 0), 0);
        assert!(geo::in_window(3, 0) && !geo::in_window(4, 0));
        assert_eq!(geo::card(5, 7, 3), (112.0 + 2.0 * 432.0, 340.0, 400.0, 225.0, 22.0));
    }
}
