//! The events and clips sheet (`docs/research/cameras-ui-spec.md`, "The events and clips sheet"): a lower pane over the picture (the
//! full view) or over the grid (`Events`, every camera) with a row of cards, 352 by 198 pictures with the kind's glyph at the corner and,
//! for a clip, its length; under the picture the title (two lines at most) and "2:31 PM \u{b7} 6 min ago". Four cards show and a
//! `chevron-right` says there are more; Left and Right move, Up to the tabs, Down returns, OK plays, Back closes.
//!
//! OK on a clip plays it (`POST playback {camera, clip}`); on an event with a recording, three seconds before it (`{camera, at}`); on an event
//! without one it only turns to the camera. Events are the last 24 h of the camera (up to 30, newest first), clips its own, the recent list
//! every camera's `cameras.events`. A card's picture is asked for only when it shows; one that is gone or never came leaves the card as the
//! kind's glyph on a quiet tile.

use super::data::{Clip, Pushed, Tick};
use super::strip::LEAD_MS;
use super::view::{CardView, SheetFocus, SheetView, Tab};
use super::words::{self, Clock};

/// How many cards show at once.
pub const SHOWN: usize = 4;
/// The card's picture.
pub const PIC_W: u32 = 352;
pub const PIC_H: u32 = 198;
/// The most events a sheet lists.
pub const MAX_EVENTS: usize = 30;

/// What OK on a card does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Play a recording from this moment (an event with a recording: three seconds before it).
    Play { camera: String, at: i64 },
    /// Play a clip.
    Clip { camera: String, clip: String },
    /// An event without a recording: only turn to its camera.
    Turn { camera: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub kind: String,
    pub title: String,
    /// The moment, for the card's second line.
    pub at: i64,
    pub length: Option<i64>,
    /// Where its picture is, if the card has a source for one.
    pub pic_path: Option<String>,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Camera { id: String, name: String },
    Recent,
}

/// What a key did to the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Nothing,
    Redraw,
    Close,
    /// The tab changed: the list it shows may need reading.
    Tab(Tab),
    Do(Action),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
}

/// `%`-encodes what is not unreserved, for an id in a query (a clip's id is percent-encoded).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct Sheet {
    pub source: Source,
    pub tab: Tab,
    events: Vec<Card>,
    clips: Vec<Card>,
    events_read: bool,
    clips_read: bool,
    pub focus: SheetFocus,
    first: usize,
}

impl Sheet {
    /// The sheet of one camera, over the full view.
    pub fn camera(id: &str, name: &str) -> Self {
        Self {
            source: Source::Camera { id: id.to_string(), name: name.to_string() },
            tab: Tab::Events,
            events: Vec::new(),
            clips: Vec::new(),
            events_read: false,
            clips_read: false,
            focus: SheetFocus::Card(0),
            first: 0,
        }
    }

    /// The recent events of every camera, over the grid: no tabs, the list is already there.
    pub fn recent(list: &[Pushed]) -> Self {
        let mut s = Self { source: Source::Recent, ..Self::camera("", "") };
        s.set_recent(list);
        s
    }

    pub fn title(&self) -> String {
        match &self.source {
            Source::Camera { name, .. } => name.clone(),
            Source::Recent => "Recent events".to_string(),
        }
    }

    pub fn camera_id(&self) -> Option<&str> {
        match &self.source {
            Source::Camera { id, .. } => Some(id),
            Source::Recent => None,
        }
    }

    pub fn has_tabs(&self) -> bool {
        matches!(self.source, Source::Camera { .. })
    }

    pub fn cards(&self) -> &[Card] {
        match self.tab {
            Tab::Events => &self.events,
            Tab::Clips => &self.clips,
        }
    }

    /// Whether the list of the shown tab has been read.
    pub fn read(&self) -> bool {
        if matches!(self.source, Source::Recent) {
            return true;
        }
        match self.tab {
            Tab::Events => self.events_read,
            Tab::Clips => self.clips_read,
        }
    }

    /// The camera's events of the last 24 h, as the recorder has them (oldest first): shown newest first, up to 30.
    pub fn set_events(&mut self, ticks: &[Tick]) {
        let Some(camera) = self.camera_id().map(str::to_string) else { return };
        let mut sorted: Vec<&Tick> = ticks.iter().collect();
        sorted.sort_by_key(|t| std::cmp::Reverse(t.at));
        self.events = sorted
            .into_iter()
            .take(MAX_EVENTS)
            .map(|t| Card {
                kind: t.kind.clone(),
                title: words::what(&t.kind, &t.label),
                at: t.at,
                length: None,
                pic_path: Some(format!("/v1/cameras/{camera}/thumbnail?at={}&w={PIC_W}&h={PIC_H}", t.at)),
                action: Action::Play { camera: camera.clone(), at: (t.at - LEAD_MS).max(0) },
            })
            .collect();
        self.events_read = true;
        self.clamp();
    }

    /// The camera's clips, newest first as the helper gives them.
    pub fn set_clips(&mut self, clips: &[Clip]) {
        let Some(camera) = self.camera_id().map(str::to_string) else { return };
        self.clips = clips
            .iter()
            .take(MAX_EVENTS)
            .filter(|c| c.has_video)
            .map(|c| Card {
                kind: c.kind.clone(),
                title: if c.description.trim().is_empty() { words::what(&c.kind, &c.label) } else { c.description.trim().to_string() },
                at: c.at,
                length: Some(c.duration),
                pic_path: c.has_thumbnail.then(|| format!("/v1/cameras/{camera}/clip-thumbnail?clip={}&w={PIC_W}&h={PIC_H}", encode(&c.id))),
                action: Action::Clip { camera: camera.clone(), clip: c.id.clone() },
            })
            .collect();
        self.clips_read = true;
        self.clamp();
    }

    /// The recent list of every camera: one card per event, newest first as `cameras.events` says.
    pub fn set_recent(&mut self, list: &[Pushed]) {
        self.events = list
            .iter()
            .take(MAX_EVENTS)
            .map(|e| Card {
                kind: e.kind.clone(),
                title: words::event_title(Some(&e.name), &e.kind, &e.label),
                at: e.at,
                length: None,
                pic_path: e.thumbnail.then(|| format!("/v1/cameras/events/{}/thumbnail?w={PIC_W}&h={PIC_H}", encode(&e.id))),
                action: if e.recording { Action::Play { camera: e.camera.clone(), at: (e.at - LEAD_MS).max(0) } } else { Action::Turn { camera: e.camera.clone() } },
            })
            .collect();
        self.events_read = true;
        self.clamp();
    }

    fn clamp(&mut self) {
        let n = self.cards().len();
        if let SheetFocus::Card(i) = self.focus {
            let i = if n == 0 { 0 } else { i.min(n - 1) };
            self.focus = SheetFocus::Card(i);
            self.reveal(i);
        }
    }

    /// Keeps card `i` among the four shown, scrolling one card at a time.
    fn reveal(&mut self, i: usize) {
        if i < self.first {
            self.first = i;
        } else if i >= self.first + SHOWN {
            self.first = i + 1 - SHOWN;
        }
        let n = self.cards().len();
        self.first = self.first.min(n.saturating_sub(SHOWN));
    }

    /// The cards that show, by index.
    pub fn visible(&self) -> std::ops::Range<usize> {
        let n = self.cards().len();
        self.first.min(n)..(self.first + SHOWN).min(n)
    }

    pub fn more(&self) -> bool {
        self.first + SHOWN < self.cards().len()
    }

    pub fn key(&mut self, key: Key) -> Out {
        match (self.focus, key) {
            (SheetFocus::Card(i), Key::Left) if i > 0 => {
                self.focus = SheetFocus::Card(i - 1);
                self.reveal(i - 1);
                Out::Redraw
            }
            (SheetFocus::Card(i), Key::Right) if i + 1 < self.cards().len() => {
                self.focus = SheetFocus::Card(i + 1);
                self.reveal(i + 1);
                Out::Redraw
            }
            (SheetFocus::Card(_), Key::Up) if self.has_tabs() => {
                self.focus = SheetFocus::Tab(self.tab);
                Out::Redraw
            }
            (SheetFocus::Card(i), Key::Ok) => match self.cards().get(i) {
                Some(c) => Out::Do(c.action.clone()),
                None => Out::Nothing,
            },
            (SheetFocus::Tab(_), Key::Down) => {
                self.focus = SheetFocus::Card(0);
                self.first = 0;
                Out::Redraw
            }
            (SheetFocus::Tab(Tab::Clips), Key::Left) => self.select(Tab::Events),
            (SheetFocus::Tab(Tab::Events), Key::Right) => self.select(Tab::Clips),
            (SheetFocus::Tab(t), Key::Ok) if t != self.tab => self.select(t),
            _ => Out::Nothing,
        }
    }

    /// Selects a tab (a click on it, or Left and Right on the tabs).
    pub fn select(&mut self, tab: Tab) -> Out {
        if tab == self.tab || !self.has_tabs() {
            self.focus = SheetFocus::Tab(tab);
            return Out::Redraw;
        }
        self.tab = tab;
        self.focus = SheetFocus::Tab(tab);
        self.first = 0;
        Out::Tab(tab)
    }

    /// Focuses card `i` (a click on it), revealing it.
    pub fn focus_card(&mut self, i: usize) {
        if i < self.cards().len() {
            self.focus = SheetFocus::Card(i);
            self.reveal(i);
        }
    }

    /// The pictures to ask for: the shown cards'.
    pub fn wanted(&self) -> Vec<(usize, &str)> {
        self.visible().filter_map(|i| self.cards()[i].pic_path.as_deref().map(|p| (i, p))).collect()
    }

    /// The picture key of card `i` of the shown tab.
    pub fn pic_key(&self, i: usize) -> String {
        let tab = match self.tab {
            Tab::Events => "e",
            Tab::Clips => "c",
        };
        format!("card:{}:{tab}{i}", self.camera_id().unwrap_or("*"))
    }

    pub fn view(&self, clock: &Clock, now: i64, has_pic: &dyn Fn(&str) -> bool) -> SheetView {
        let cards: Vec<CardView> = self
            .cards()
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let key = self.pic_key(i);
                CardView {
                    title: c.title.clone(),
                    line2: clock.card_line(c.at, now),
                    icon: words::kind_icon(&c.kind),
                    length: c.length.map(words::length),
                    pic: (c.pic_path.is_some() && has_pic(&key)).then_some(key),
                }
            })
            .collect();
        let empty = (self.read() && cards.is_empty()).then(|| match self.tab {
            Tab::Events => "No events".to_string(),
            Tab::Clips => "No clips".to_string(),
        });
        SheetView { title: self.title(), tabs: self.has_tabs().then_some(self.tab), cards, first: self.first, more: self.more(), focus: self.focus, empty }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::data::ClipRef;
    use crate::cameras::words::Zone;

    const MIN: i64 = 60_000;
    const NOW: i64 = (20_755 * 86_400 + 14 * 3600 + 37 * 60) * 1000;

    fn clock() -> Clock {
        Clock { twelve_hour: true, zone: Zone::UTC }
    }

    fn tick(at: i64, kind: &str, label: &str) -> Tick {
        Tick { at, kind: kind.into(), label: label.into(), clip: None::<ClipRef> }
    }

    fn five() -> Vec<Tick> {
        vec![
            tick(NOW - 5 * 60 * MIN, "package", ""),
            tick(NOW - 3 * 60 * MIN - 53 * MIN, "vehicle", ""),
            tick(NOW - 2 * 60 * MIN - 25 * MIN, "animal", "Domestic Cat"),
            tick(NOW - 39 * MIN, "doorbell", "doorbell"),
            tick(NOW - 6 * MIN, "person", "person"),
        ]
    }

    #[test]
    fn events_are_listed_newest_first_with_their_words_and_a_picture_source() {
        let mut s = Sheet::camera("273", "Front Door");
        assert!(!s.read());
        s.set_events(&five());
        assert!(s.read());
        let titles: Vec<&str> = s.cards().iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Person", "Doorbell rang", "Animal \u{b7} Domestic Cat", "Vehicle", "Package"]);
        assert_eq!(s.cards()[0].action, Action::Play { camera: "273".into(), at: NOW - 6 * MIN - LEAD_MS }, "three seconds before");
        assert_eq!(s.cards()[0].pic_path.as_deref(), Some(format!("/v1/cameras/273/thumbnail?at={}&w=352&h=198", NOW - 6 * MIN).as_str()));
        let v = s.view(&clock(), NOW, &|_| false);
        assert_eq!(v.title, "Front Door");
        assert_eq!(v.cards[0].line2, "2:31 PM \u{b7} 6 min ago");
        assert_eq!(v.cards[1].line2, "1:58 PM \u{b7} 39 min ago");
        assert_eq!(v.cards[0].icon, "user");
        assert_eq!(v.tabs, Some(Tab::Events));
        assert!(v.more, "five cards, four show: a chevron says there are more");
    }

    #[test]
    fn four_cards_show_and_the_row_scrolls_one_card_at_a_time_with_the_focus() {
        let mut s = Sheet::camera("273", "A");
        s.set_events(&five());
        assert_eq!(s.visible(), 0..4);
        for _ in 0..3 {
            s.key(Key::Right);
        }
        assert_eq!((s.focus, s.visible()), (SheetFocus::Card(3), 0..4));
        s.key(Key::Right);
        assert_eq!((s.focus, s.visible()), (SheetFocus::Card(4), 1..5), "the fifth card scrolls one");
        assert!(!s.more());
        assert_eq!(s.key(Key::Right), Out::Nothing, "the end holds");
        s.key(Key::Left);
        s.key(Key::Left);
        s.key(Key::Left);
        s.key(Key::Left);
        assert_eq!((s.focus, s.visible()), (SheetFocus::Card(0), 0..4));
        assert_eq!(s.key(Key::Left), Out::Nothing);
    }

    #[test]
    fn up_goes_to_the_tabs_left_and_right_choose_one_and_down_returns() {
        let mut s = Sheet::camera("273", "A");
        s.set_events(&five());
        assert_eq!(s.key(Key::Up), Out::Redraw);
        assert_eq!(s.focus, SheetFocus::Tab(Tab::Events));
        assert_eq!(s.key(Key::Right), Out::Tab(Tab::Clips));
        assert_eq!((s.tab, s.focus), (Tab::Clips, SheetFocus::Tab(Tab::Clips)));
        assert!(!s.read(), "the clips are not read yet");
        assert_eq!(s.key(Key::Left), Out::Tab(Tab::Events));
        assert_eq!(s.key(Key::Down), Out::Redraw);
        assert_eq!(s.focus, SheetFocus::Card(0));
    }

    #[test]
    fn a_clip_has_its_description_a_length_a_percent_encoded_picture_and_plays_by_its_id() {
        let mut s = Sheet::camera("273", "A");
        s.tab = Tab::Clips;
        s.set_clips(&[
            Clip { id: "c1.WyIx/2".into(), at: NOW - 6 * MIN, duration: 30_000, kind: "person".into(), label: "person".into(), description: "Person at the front door".into(), has_thumbnail: true, has_video: true },
            Clip { id: "c2".into(), at: NOW - 40 * MIN, duration: 42_000, kind: "doorbell".into(), label: "".into(), description: "".into(), has_thumbnail: false, has_video: true },
            Clip { id: "c3".into(), at: NOW - 41 * MIN, duration: 1000, kind: "person".into(), label: "".into(), description: "".into(), has_thumbnail: true, has_video: false },
        ]);
        assert_eq!(s.cards().len(), 2, "a clip with no video cannot be played and is not listed");
        assert_eq!(s.cards()[0].pic_path.as_deref(), Some("/v1/cameras/273/clip-thumbnail?clip=c1.WyIx%2F2&w=352&h=198"));
        assert_eq!(s.cards()[1].pic_path, None, "no thumbnail: a glyph on a quiet tile");
        assert_eq!(s.cards()[1].title, "Doorbell rang");
        let v = s.view(&clock(), NOW, &|_| false);
        assert_eq!(v.cards[0].length.as_deref(), Some("0:30"));
        assert_eq!(v.cards[0].title, "Person at the front door");
        assert_eq!(s.key(Key::Ok), Out::Do(Action::Clip { camera: "273".into(), clip: "c1.WyIx/2".into() }));
        assert_eq!(s.pic_key(0), "card:273:c0");
    }

    #[test]
    fn the_recent_list_names_the_camera_and_an_event_without_a_recording_only_turns_to_it() {
        let list = vec![
            Pushed { id: "e2".into(), camera: "274".into(), name: "Backyard".into(), kind: "animal".into(), label: "animal".into(), at: NOW - 11 * MIN, phase: crate::cameras::data::Phase::New, thumbnail: true, recording: false },
            Pushed { id: "e1".into(), camera: "273".into(), name: "Front Door".into(), kind: "person".into(), label: "".into(), at: NOW - 6 * MIN, phase: crate::cameras::data::Phase::New, thumbnail: false, recording: true },
        ];
        let mut s = Sheet::recent(&list);
        assert!(s.read() && !s.has_tabs());
        assert_eq!(s.title(), "Recent events");
        let v = s.view(&clock(), NOW, &|_| false);
        assert_eq!(v.cards[0].title, "Backyard \u{b7} Animal");
        assert_eq!(v.tabs, None);
        assert_eq!(s.cards()[0].pic_path.as_deref(), Some("/v1/cameras/events/e2/thumbnail?w=352&h=198"));
        assert_eq!(s.cards()[1].pic_path, None, "no thumbnail: no ask");
        assert_eq!(s.key(Key::Ok), Out::Do(Action::Turn { camera: "274".into() }));
        s.key(Key::Right);
        assert_eq!(s.key(Key::Ok), Out::Do(Action::Play { camera: "273".into(), at: NOW - 6 * MIN - LEAD_MS }));
        assert_eq!(s.key(Key::Up), Out::Nothing, "no tabs to go up to");
    }

    #[test]
    fn only_the_shown_cards_ask_for_pictures_and_a_card_with_its_picture_in_the_store_shows_it() {
        let mut s = Sheet::camera("273", "A");
        s.set_events(&five());
        assert_eq!(s.wanted().len(), 4);
        s.focus_card(4);
        let wanted: Vec<usize> = s.wanted().into_iter().map(|(i, _)| i).collect();
        assert_eq!(wanted, [1, 2, 3, 4]);
        let v = s.view(&clock(), NOW, &|k| k == "card:273:e2");
        assert_eq!(v.cards[2].pic.as_deref(), Some("card:273:e2"));
        assert_eq!(v.cards[1].pic, None, "no picture arrived: the glyph on a quiet tile");
    }

    #[test]
    fn an_empty_list_that_was_read_says_so() {
        let mut s = Sheet::camera("273", "A");
        assert_eq!(s.view(&clock(), NOW, &|_| false).empty, None, "not read yet: nothing is claimed");
        s.set_events(&[]);
        assert_eq!(s.view(&clock(), NOW, &|_| false).empty.as_deref(), Some("No events"));
        assert_eq!(s.key(Key::Ok), Out::Nothing);
    }

    #[test]
    fn percent_encoding_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(encode("c1.Wy-_~Ab9"), "c1.Wy-_~Ab9");
        assert_eq!(encode("a b/c+d=="), "a%20b%2Fc%2Bd%3D%3D");
    }
}
