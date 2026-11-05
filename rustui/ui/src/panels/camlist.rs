//! The camera list: the sheet that grows out of Home's camera button, a row for each camera with a small picture of what it sees and its name
//! (`docs/research/camera-list-result.md`). Up and Down walk the rows, OK opens the camera in the Cameras viewer, Back closes the sheet into
//! the button; a click does the same.
//!
//! The pictures are stills of the helper's snapshot route (`/v1/cameras/{id}/frame`) at the size of the picture box, not video: the TV has two
//! hardware video windows and a window over Home retimes the panel to 100 Hz, so no row plays video. The focused row is asked for about eight
//! pictures a second, the other rows in view three, the row above and the row below one; nothing is asked for while the sheet is not up, except
//! the first rows while the focus rests on the camera button, so the sheet opens on pictures that are already there.

use super::life::{LINGER_MS, Life, SPRING_MS};
use super::{K, Out};
use crate::cameras::data::{Cameras, LinkState};
use crate::cameras::grid;
use crate::cameras::pics::{Planner, Want};
use crate::cameras::sheet::encode;
use crate::cameras::words;
use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// The tags of the asks (the Cameras app's are `0x0CA0_0000` and up, Home's cards' `0x0CB0_0000`).
pub const TAG_FIRST: u64 = 0x0CC0_0000;
pub const TAG_LAST: u64 = 0x0CCF_FFFF;
/// The picture box of a row: 16:9 and even, and the size each still is asked for (the helper decodes a camera's 320 px stream at half size).
pub const PICTURE: (u32, u32) = (160, 90);
/// The rows that show whole at most.
pub const ROWS_SHOWN: usize = 5;
/// The pause between a picture arriving and the next ask of it: the focused row, the other rows in view, the rows beside the view.
pub const FOCUS_MS: u64 = 125;
pub const SHOWN_MS: u64 = 333;
pub const NEAR_MS: u64 = 1000;
/// The most stills asked for at once: the helper serves at most eight cameras' streams, and a ninth is refused as busy.
pub const MAX_ASKED: usize = 8;
/// The focus has rested on the camera button this long before its first rows are asked for, and they are asked for this long at most.
pub const DWELL_MS: u64 = 350;
pub const PREFETCH_MS: u64 = 10_000;
pub const PREFETCH_ROWS: usize = 4;
/// A picture this young is "Live".
pub const LIVE_MS: u64 = 3_000;

/// What one row says.
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    pub id: String,
    pub name: String,
    /// How many pictures of it have arrived: the surface paints the row again when it changes.
    pub rev: u64,
    /// What the row says under its name, and whether that is "Live" (the live dot) and whether the picture is dimmed.
    pub words: String,
    pub live: bool,
    pub dim: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub rows: Vec<RowView>,
    pub index: usize,
    /// The head's words at the right: how many cameras, or what is wrong with the link.
    pub note: String,
    /// With no row: the icon, what to say and why.
    pub empty: Option<(String, String, String)>,
    pub over_video: bool,
}

struct Held {
    picture: Picture,
    rev: u64,
}

pub struct CameraList {
    pub life: Life,
    topic: Cameras,
    index: usize,
    /// The first row that shows, and how many show whole (the surface says).
    first: usize,
    fit: usize,
    size: (u32, u32),
    planner: Planner,
    wants: Vec<Want>,
    held: HashMap<String, Held>,
    /// Whether the focus is on the camera button, and since when it has rested there (none once the list opened from it).
    button: bool,
    since: Option<u64>,
    clock: u64,
    /// When the list opened, and how long each row's first fresh picture took after that.
    opened: Option<u64>,
    first_ms: HashMap<String, u64>,
}

impl Default for CameraList {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraList {
    pub fn new() -> Self {
        Self {
            life: Life::new(SPRING_MS, SPRING_MS, LINGER_MS),
            topic: Cameras::default(),
            index: 0,
            first: 0,
            fit: ROWS_SHOWN,
            size: PICTURE,
            planner: Planner::with_tags(TAG_FIRST, TAG_LAST),
            wants: Vec::new(),
            held: HashMap::new(),
            button: false,
            since: None,
            clock: 0,
            opened: None,
            first_ms: HashMap::new(),
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// The newest picture of a camera, whatever its age.
    pub fn picture(&self, id: &str) -> Option<&Picture> {
        self.held.get(id).map(|h| &h.picture)
    }

    /// The surface says how many rows show whole and how big its picture box is.
    pub fn set_shape(&mut self, fit: usize, size: (u32, u32)) {
        let fit = fit.max(1);
        if (self.fit, self.size) != (fit, size) {
            self.fit = fit;
            self.size = size;
            self.follow();
        }
    }

    /// The focus is on the camera button or has left it: resting there, the first rows' pictures are asked for.
    pub fn button_focus(&mut self, on: bool, now: u64) {
        if on != self.button {
            self.button = on;
            self.clock = self.clock.max(now);
            self.since = on.then_some(now);
        }
    }

    /// The first row that shows keeps the focused row in view.
    fn follow(&mut self) {
        let n = self.topic.list.len();
        self.index = self.index.min(n.saturating_sub(1));
        self.first = self.first.min(n.saturating_sub(self.fit));
        if self.index < self.first {
            self.first = self.index;
        } else if self.index >= self.first + self.fit {
            self.first = self.index + 1 - self.fit;
        }
    }

    /// The list is wanted: the focus on the first camera, the pictures asked for at once.
    pub fn open(&mut self, now: u64) -> Out {
        self.index = 0;
        self.first = 0;
        self.since = None;
        self.opened = Some(now);
        // a picture that is there and young is the row's first
        self.first_ms = self.topic.list.iter().filter(|c| self.planner.pic(&c.id).last_at.is_some_and(|t| now.saturating_sub(t) < LIVE_MS)).map(|c| (c.id.clone(), 0)).collect();
        self.follow();
        Out::effects(self.plan(now))
    }

    /// The list goes: nothing is asked for any more, and what is still on its way is not wanted.
    pub fn close(&mut self) -> Out {
        self.opened = None;
        self.wants.clear();
        self.planner.quiesce();
        Out::default()
    }

    fn prefetching(&self, now: u64) -> bool {
        self.since.is_some_and(|s| now >= s + DWELL_MS && now < s + DWELL_MS + PREFETCH_MS)
    }

    fn asking(&self, now: u64) -> bool {
        self.life.presented() || self.prefetching(now)
    }

    /// The stills wanted now, the focused row first and the rest by how near they are to it.
    fn want_list(&self, now: u64) -> Vec<Want> {
        let list = &self.topic.list;
        if self.topic.state != LinkState::Ready || !self.asking(now) {
            return Vec::new();
        }
        let (w, h) = self.size;
        let path = |id: &str, pace: &str| format!("/v1/cameras/{}/frame?w={w}&h={h}{pace}", encode(id));
        let mut out = Vec::new();
        if self.life.presented() {
            let lo = self.first.saturating_sub(1);
            let hi = (self.first + self.fit).min(list.len().saturating_sub(1));
            let mut rows: Vec<usize> = (lo..=hi).filter(|i| *i < list.len()).collect();
            rows.sort_by_key(|i| i.abs_diff(self.index));
            for i in rows {
                let c = &list[i];
                if !c.online {
                    continue;
                }
                let (every, pace) = if i == self.index {
                    (FOCUS_MS, "&fps=8")
                } else if (self.first..self.first + self.fit).contains(&i) {
                    (SHOWN_MS, "&fps=4")
                } else {
                    (NEAR_MS, "")
                };
                out.push(Want::repeating(&c.id, &path(&c.id, pace), every));
            }
        } else {
            for c in list.iter().filter(|c| c.online).take(PREFETCH_ROWS) {
                out.push(Want::repeating(&c.id, &path(&c.id, ""), NEAR_MS));
            }
        }
        out.truncate(MAX_ASKED);
        out
    }

    fn plan(&mut self, now: u64) -> Vec<Effect> {
        self.clock = now;
        self.wants = self.want_list(now);
        if self.wants.is_empty() {
            // what is on its way is not wanted: its answer is dropped
            self.planner.quiesce();
            return Vec::new();
        }
        self.planner.plan(&self.wants, now, MAX_ASKED)
    }

    /// An input of the host: the `cameras` topic and the answers to the stills. Returns what to ask for and whether the view changed.
    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let mut changed = false;
        match input {
            Input::Helper { topic, data } if topic == "cameras" => {
                let next = Cameras::parse(data);
                if next != self.topic {
                    self.topic = next;
                    let ids: Vec<&str> = self.topic.list.iter().map(|c| c.id.as_str()).collect();
                    self.held.retain(|id, _| ids.contains(&id.as_str()));
                    self.follow();
                    changed = true;
                }
            }
            Input::Bytes { tag, status, bytes } if (TAG_FIRST..=TAG_LAST).contains(tag) => {
                // an answer that is not in flight any more (the list closed, the row left the view) is dropped undecoded
                if let Some(key) = self.planner.key_of(*tag).map(str::to_owned) {
                    let picture = (*status == 200).then(|| picture::decode(bytes).ok()).flatten();
                    let ok = picture.is_some();
                    if let Some(picture) = picture {
                        let rev = self.held.get(&key).map_or(1, |h| h.rev + 1);
                        self.held.insert(key.clone(), Held { picture, rev });
                        if let Some(opened) = self.opened {
                            self.first_ms.entry(key).or_insert(now.saturating_sub(opened));
                        }
                        changed = true;
                    }
                    self.planner.arrived(*tag, *status, ok, now);
                }
            }
            Input::Fetched { tag, status, .. } if (TAG_FIRST..=TAG_LAST).contains(tag) => {
                self.planner.arrived(*tag, *status, false, now);
            }
            _ => {}
        }
        (self.plan(now), changed)
    }

    /// Time passed: what is due is asked for. True when what a row says of its picture's age has changed.
    pub fn tick(&mut self, now: u64) -> (Vec<Effect>, bool) {
        let before = self.clock;
        let effects = self.plan(now);
        let aged = self.life.presented() && self.caption_at(before).is_some_and(|t| now >= t);
        (effects, aged)
    }

    /// When a row's words next change by themselves: its picture stops being "Live", and then its age's words move on.
    fn caption_at(&self, now: u64) -> Option<u64> {
        self.topic
            .list
            .iter()
            .filter_map(|c| {
                let at = self.planner.pic(&c.id).last_at?;
                let age = now.saturating_sub(at);
                Some(if age < LIVE_MS {
                    at + LIVE_MS
                } else if age < 60_000 {
                    at + (age / 1000 + 1) * 1000
                } else {
                    at + (age / 60_000 + 1) * 60_000
                })
            })
            .min()
    }

    pub fn due_at(&self) -> Option<u64> {
        let mut due = self.planner.due_at(&self.wants);
        if let Some(s) = self.since {
            for t in [s + DWELL_MS, s + DWELL_MS + PREFETCH_MS] {
                if t > self.clock {
                    due = Some(due.map_or(t, |d| d.min(t)));
                }
            }
        }
        if self.life.presented()
            && let Some(t) = self.caption_at(self.clock)
        {
            due = Some(due.map_or(t, |d| d.min(t)));
        }
        due
    }

    /// A key of the remote: Up and Down walk the rows without wrapping, OK opens the camera, Back closes the list.
    pub fn key(&mut self, k: K, now: u64) -> Out {
        let n = self.topic.list.len();
        match k {
            K::Back => return Out::close(),
            K::Ok => return self.open_camera(self.index),
            K::Up => self.index = self.index.saturating_sub(1),
            K::Down => self.index = (self.index + 1).min(n.saturating_sub(1)),
            _ => return Out::default(),
        }
        self.follow();
        Out::effects(self.plan(now))
    }

    /// A click on a row opens its camera.
    pub fn click(&mut self, i: usize) -> Out {
        self.index = i.min(self.topic.list.len().saturating_sub(1));
        self.follow();
        self.open_camera(self.index)
    }

    /// The viewer is to open on the camera, and the list goes (the shell does the opening: it is the Cameras app's).
    fn open_camera(&self, i: usize) -> Out {
        match self.topic.list.get(i) {
            Some(c) => Out { events: vec![("cameralist.open".into(), json!({"camera": c.id}))], close: true, ..Out::default() },
            None => Out::default(),
        }
    }

    /// What a row says of its picture: "Live" while pictures arrive, else the Cameras grid's words for a tile.
    fn say(&self, id: &str, online: bool, now: u64) -> (String, bool, bool) {
        let pic = self.planner.pic(id);
        let tile = grid::tile_words(online, self.topic.state, pic, now);
        if !tile.lines.is_empty() {
            return (tile.lines.join("  \u{b7}  "), false, tile.dim);
        }
        let age = pic.last_at.map_or(0, |t| now.saturating_sub(t));
        if age < LIVE_MS { (words::LIVE.to_string(), true, false) } else { (words::age(age as i64), false, false) }
    }

    pub fn view(&self, over_video: bool, now: u64) -> View {
        let rows: Vec<RowView> = self
            .topic
            .list
            .iter()
            .map(|c| {
                let (words, live, dim) = self.say(&c.id, c.online, now);
                RowView { id: c.id.clone(), name: c.name.clone(), rev: self.held.get(&c.id).map_or(0, |h| h.rev), words, live, dim }
            })
            .collect();
        let link = words::link_words(self.topic.state.name());
        let empty = if !rows.is_empty() {
            None
        } else if self.topic.state == LinkState::Ready {
            Some(("video".to_string(), words::NO_CAMERAS.to_string(), "Cameras shared with this TV show here.".to_string()))
        } else {
            Some(("unplug".to_string(), link.to_string(), "Cameras show here once the link is up.".to_string()))
        };
        let note = if !link.is_empty() { link.to_string() } else if rows.is_empty() { String::new() } else { words::count(rows.len()) };
        View { up: self.life.presented(), rows, index: self.index, note, empty, over_video }
    }

    /// What the host's state shows of the list: the harness reads the rows asked for and the time each first picture took. Ages are as of the
    /// last time the model looked.
    pub fn report(&self) -> Value {
        let now = self.clock;
        let pictures: Map<String, Value> = self
            .topic
            .list
            .iter()
            .filter_map(|c| {
                let held = self.held.get(&c.id)?;
                Some((c.id.clone(), json!({"rev": held.rev, "ageMs": self.planner.pic(&c.id).last_at.map(|t| now.saturating_sub(t))})))
            })
            .collect();
        let firsts: Map<String, Value> = self.first_ms.iter().map(|(id, ms)| (id.clone(), json!(ms))).collect();
        json!({
            "rows": self.topic.list.len(),
            "index": self.index,
            "first": self.first,
            "fit": self.fit,
            "asking": self.wants.iter().map(|w| w.key.clone()).collect::<Vec<_>>(),
            "inFlight": self.planner.in_flight(),
            "pictures": pictures,
            "firstMs": firsts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(n: usize) -> Value {
        json!({"state": "ready", "cameras": (0..n).map(|i| json!({"id": format!("c{i}"), "name": format!("Camera {i}"), "online": true})).collect::<Vec<_>>()})
    }

    fn list(n: usize) -> CameraList {
        let mut l = CameraList::new();
        l.update(&Input::Helper { topic: "cameras".into(), data: topic(n) }, 1000);
        l
    }

    fn open(l: &mut CameraList, now: u64) -> Vec<Effect> {
        l.life.show(now);
        l.open(now).effects
    }

    fn fetches(fx: &[Effect]) -> Vec<(u64, String)> {
        fx.iter().filter_map(|e| if let Effect::Fetch { tag, path } = e { Some((*tag, path.clone())) } else { None }).collect()
    }

    fn picture_bytes() -> Vec<u8> {
        include_bytes!("../../../../assets/icon160.png").to_vec()
    }

    fn answer(l: &mut CameraList, tag: u64, now: u64) -> Vec<Effect> {
        l.update(&Input::Bytes { tag, status: 200, bytes: picture_bytes() }, now).0
    }

    #[test]
    fn opening_asks_for_the_rows_in_view_and_one_beside_them_the_focused_row_fastest_and_first() {
        let mut l = list(8);
        let asked = fetches(&open(&mut l, 2000));
        assert_eq!(asked.len(), 6, "five in view and the one under them: {asked:?}");
        assert_eq!(asked[0].1, "/v1/cameras/c0/frame?w=160&h=90&fps=8", "the focused row, at its quick pace");
        assert_eq!(asked[1].1, "/v1/cameras/c1/frame?w=160&h=90&fps=4");
        assert_eq!(asked[5].1, "/v1/cameras/c5/frame?w=160&h=90", "the row beside the view is asked for at the helper's own pace");
        assert!(asked.iter().all(|(t, _)| (TAG_FIRST..=TAG_LAST).contains(t)));
    }

    #[test]
    fn up_and_down_walk_the_rows_without_wrapping_and_the_view_follows_the_focus() {
        let mut l = list(8);
        open(&mut l, 2000);
        l.key(K::Up, 2100);
        assert_eq!(l.index(), 0, "the top holds");
        for _ in 0..6 {
            l.key(K::Down, 2200);
        }
        assert_eq!((l.index(), l.first), (6, 2), "rows 2 to 6 show");
        for _ in 0..4 {
            l.key(K::Down, 2300);
        }
        assert_eq!(l.index(), 7, "the bottom holds");
        for _ in 0..8 {
            l.key(K::Up, 2400);
        }
        assert_eq!((l.index(), l.first), (0, 0));
    }

    #[test]
    fn the_rows_asked_for_follow_the_view_and_the_focus_takes_the_quick_pace() {
        let mut l = list(8);
        open(&mut l, 2000);
        let fx = l.key(K::Down, 2100).effects;
        // c0 is no longer the focus (its path changes: asked afresh), c1 is
        let paths: Vec<String> = fetches(&fx).into_iter().map(|f| f.1).collect();
        assert!(paths.contains(&"/v1/cameras/c1/frame?w=160&h=90&fps=8".to_string()), "{paths:?}");
        assert!(paths.contains(&"/v1/cameras/c0/frame?w=160&h=90&fps=4".to_string()), "{paths:?}");
        assert_eq!(l.report()["asking"][0], "c1", "the focused row is asked first");
    }

    #[test]
    fn ok_opens_the_focused_camera_and_closes_the_list_and_back_only_closes_it() {
        let mut l = list(3);
        open(&mut l, 2000);
        l.key(K::Down, 2100);
        let out = l.key(K::Ok, 2200);
        assert_eq!(out.events, vec![("cameralist.open".to_string(), json!({"camera": "c1"}))]);
        assert!(out.close);
        let out = l.key(K::Back, 2300);
        assert!(out.close && out.events.is_empty());
        let out = l.click(2);
        assert_eq!((l.index(), out.close, out.events[0].1["camera"].clone()), (2, true, json!("c2")), "a click opens the row it is on");
    }

    #[test]
    fn a_picture_is_kept_for_its_row_and_its_next_ask_waits_for_the_quick_pace() {
        let mut l = list(2);
        let asked = fetches(&open(&mut l, 2000));
        assert_eq!(l.view(false, 2000).rows[0].words, "Connecting");
        let fx = answer(&mut l, asked[0].0, 2150);
        assert!(l.picture("c0").is_some());
        assert!(fx.is_empty(), "the next ask of the row waits {FOCUS_MS} ms");
        let v = l.view(false, 2150);
        assert_eq!((v.rows[0].rev, v.rows[0].words.as_str(), v.rows[0].live), (1, "Live", true));
        assert_eq!(l.report()["firstMs"]["c0"], 150, "the first picture's time from the opening");
        assert_eq!(l.due_at(), Some(2150 + FOCUS_MS));
        let again = fetches(&l.tick(2150 + FOCUS_MS).0);
        assert_eq!(again.len(), 1, "{again:?}");
        assert!(again[0].1.starts_with("/v1/cameras/c0/frame"));
    }

    #[test]
    fn closing_pauses_everything_and_a_late_answer_is_dropped_undecoded() {
        let mut l = list(3);
        let asked = fetches(&open(&mut l, 2000));
        l.life.hide(2100);
        l.close();
        assert_eq!(l.due_at(), None);
        assert!(fetches(&l.tick(5000).0).is_empty());
        let (fx, changed) = l.update(&Input::Bytes { tag: asked[0].0, status: 200, bytes: picture_bytes() }, 2200);
        assert!(fx.is_empty() && !changed && l.picture("c0").is_none());
        assert_eq!(l.report()["asking"], json!([]));
    }

    #[test]
    fn the_pictures_stay_for_the_next_opening_and_are_its_first() {
        let mut l = list(3);
        let asked = fetches(&open(&mut l, 2000));
        for (tag, _) in &asked {
            answer(&mut l, *tag, 2100);
        }
        l.life.hide(2200);
        l.close();
        let again: Vec<String> = fetches(&open(&mut l, 2300)).into_iter().map(|f| f.1).collect();
        assert_eq!(again, ["/v1/cameras/c0/frame?w=160&h=90&fps=8"], "only the focused row is due again: the others' pace has not run out");
        assert_eq!(l.report()["firstMs"], json!({"c0": 0, "c1": 0, "c2": 0}), "young pictures are there at the opening");
        assert!(l.view(false, 2300).rows.iter().all(|r| r.rev == 1 && r.live));
    }

    #[test]
    fn resting_on_the_button_fetches_the_first_rows_early_and_leaving_it_stops() {
        let mut l = list(8);
        l.button_focus(true, 5000);
        assert!(fetches(&l.tick(5349).0).is_empty(), "not before the focus has rested");
        assert_eq!(l.due_at(), Some(5000 + DWELL_MS));
        let asked = fetches(&l.tick(5350).0);
        assert_eq!(asked.len(), PREFETCH_ROWS);
        assert_eq!(asked[0].1, "/v1/cameras/c0/frame?w=160&h=90", "the helper's own pace: the list is not up");
        answer(&mut l, asked[0].0, 5400);
        l.button_focus(false, 5500);
        assert!(fetches(&l.tick(6600).0).is_empty());
        assert_eq!(l.due_at(), None, "nothing is wanted once the focus is gone");
        // opening ends the resting: closing again does not start the asks over
        l.button_focus(true, 7000);
        l.tick(7400);
        open(&mut l, 7500);
        l.life.hide(7600);
        l.close();
        l.button_focus(true, 7700);
        assert!(fetches(&l.tick(9000).0).is_empty(), "the focus never left the button: the list's closing is not a new rest");
    }

    #[test]
    fn a_rest_on_the_button_asks_for_ten_seconds_at_most() {
        let mut l = list(2);
        l.button_focus(true, 5000);
        assert!(!fetches(&l.tick(5400).0).is_empty());
        assert!(fetches(&l.tick(5000 + DWELL_MS + PREFETCH_MS).0).is_empty());
    }

    #[test]
    fn at_most_eight_stills_are_asked_for_however_many_rows_show() {
        let mut l = list(20);
        l.set_shape(12, PICTURE);
        assert_eq!(fetches(&open(&mut l, 2000)).len(), MAX_ASKED);
    }

    #[test]
    fn an_offline_camera_is_not_asked_for_and_says_so_and_a_stale_picture_says_its_age_dimmed() {
        let mut l = CameraList::new();
        let data = json!({"state": "ready", "cameras": [{"id": "a", "name": "Front", "online": true}, {"id": "b", "name": "Garage", "online": false}]});
        l.update(&Input::Helper { topic: "cameras".into(), data }, 1000);
        let asked = fetches(&open(&mut l, 2000));
        assert_eq!(asked.len(), 1, "{asked:?}");
        answer(&mut l, asked[0].0, 2100);
        let words = |l: &CameraList, at: u64| l.view(false, at).rows.iter().map(|r| (r.words.clone(), r.dim)).collect::<Vec<_>>();
        assert_eq!(words(&l, 2100), [("Live".to_string(), false), ("Offline".to_string(), false)]);
        assert_eq!(words(&l, 2100 + 8_000)[0], ("8 s ago".to_string(), false));
        assert_eq!(words(&l, 2100 + 13_000)[0], ("13 s ago".to_string(), true), "past twelve seconds the picture is dimmed");
    }

    #[test]
    fn with_no_row_the_sheet_says_why() {
        let mut l = CameraList::new();
        l.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "unlinked", "cameras": []}) }, 1000);
        assert_eq!(l.view(false, 1000).empty.map(|e| e.1), Some("Scrypted isn\u{2019}t linked".to_string()));
        assert!(open(&mut l, 2000).is_empty(), "nothing is asked of a link that is not up");
        l.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "ready", "cameras": []}) }, 1100);
        assert_eq!(l.view(false, 1100).empty.map(|e| e.1), Some("No cameras shared".to_string()));
        assert_eq!(l.key(K::Ok, 2100), Out::default(), "OK on nothing opens nothing");
    }

    #[test]
    fn a_camera_that_leaves_the_topic_takes_its_picture_and_the_focus_stays_on_a_row() {
        let mut l = list(3);
        let asked = fetches(&open(&mut l, 2000));
        answer(&mut l, asked[2].0, 2100);
        l.key(K::Down, 2200);
        l.key(K::Down, 2210);
        assert_eq!(l.index(), 2);
        l.update(&Input::Helper { topic: "cameras".into(), data: topic(2) }, 2300);
        assert!(l.picture("c2").is_none());
        assert_eq!(l.index(), 1);
    }
}
