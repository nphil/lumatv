//! What the status bar's free side shows with Full width on: a glyph for each source that has unread notifications in the bell, and a line
//! with the newest unread title that gives way to the next every [`ROTATE_MS`] by a crossfade (the surface draws it; nothing here moves).
//! The data is the bell's own: the TV's log and the plugins' notifications that came after the drawer was last open.

use serde_json::{Value, json};

/// How many sources get a glyph of their own; the rest are "+N".
pub const MAX_SOURCES: usize = 6;
/// How many titles the line takes turns with.
pub const MAX_TITLES: usize = 8;
/// How long a title is on the line before the next one comes.
pub const ROTATE_MS: u64 = 6000;
/// The ticker's narrowest and widest pill at 100 % text. A free side with less room than the narrowest shows the glyphs alone.
pub const MIN_TICKER: f32 = 240.0;
pub const MAX_TICKER: f32 = 640.0;

/// The `type` of the virtual status items the strip adds after the layout's own.
pub const SOURCE: &str = "note-source";
pub const MORE: &str = "note-more";
pub const TICKER: &str = "ticker";

/// An unread notification as the bell holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub source: String,
    pub icon: String,
    pub title: String,
    /// Milliseconds since the epoch.
    pub time: u64,
}

/// A title on the line, and the source it is of (what OK on the line opens the bell at).
#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    pub text: String,
    pub source: String,
}

/// A source with unread notifications: the glyph of its newest, and how many it has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    pub name: String,
    pub icon: String,
    pub count: usize,
}

/// What the bell holds unread: the badge's count, the sources newest first and the titles newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unread {
    pub count: usize,
    pub sources: Vec<Source>,
    pub titles: Vec<Title>,
}

impl Unread {
    pub fn of(mut notes: Vec<Note>, count: usize) -> Self {
        // a stable sort: what came at the same time keeps the order it was given in
        notes.sort_by_key(|n| std::cmp::Reverse(n.time));
        let mut sources: Vec<Source> = Vec::new();
        let mut titles: Vec<Title> = Vec::new();
        for n in notes {
            if titles.len() < MAX_TITLES && !n.title.is_empty() && !titles.iter().any(|t| t.text == n.title) {
                titles.push(Title { text: n.title.clone(), source: n.source.clone() });
            }
            match sources.iter_mut().find(|s| s.name == n.source) {
                Some(s) => s.count += 1,
                None => sources.push(Source { name: n.source, icon: n.icon, count: 1 }),
            }
        }
        Self { count, sources, titles }
    }
}

/// The strip's items and where each is, `(x, y, w, h)` in the order they are visited: the sources' glyphs, "+N", the ticker.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    pub items: Vec<Value>,
    pub frames: Vec<(f32, f32, f32, f32)>,
}

/// The strip laid out in `room` (its left edge and its width), one button's height `h` tall at `y`, `gap` between its parts. Glyphs are a
/// button each, at most [`MAX_SOURCES`] and fewer when `room` has not the width (then "+N" says how many sources are left); the ticker is
/// what is left of the room, `min` to `max` wide, and is left out when there is less than `min` or when `ticker` is off.
pub fn place(unread: &Unread, ticker: bool, room: (f32, f32), (y, h): (f32, f32), gap: f32, (min, max): (f32, f32)) -> Placed {
    let n = unread.sources.len();
    if n == 0 {
        return Placed::default();
    }
    let run = |shown: usize| -> f32 {
        let glyphs = shown as f32 * h + shown.saturating_sub(1) as f32 * gap;
        match (shown, n > shown) {
            (0, true) => h,
            (_, true) => glyphs + gap + h,
            _ => glyphs,
        }
    };
    let fit = |avail: f32| (0..=n.min(MAX_SOURCES)).rev().find(|s| run(*s) <= avail);
    let with_line = ticker && !unread.titles.is_empty();
    let (shown, line) = match fit(room.1 - if with_line { min + gap } else { 0.0 }) {
        Some(s) if with_line => (s, true),
        _ => match fit(room.1) {
            Some(s) => (s, false),
            None => return Placed::default(),
        },
    };
    let mut out = Placed::default();
    let mut x = room.0;
    for s in &unread.sources[..shown] {
        out.items.push(json!({"type": SOURCE, "source": s.name, "icon": s.icon, "count": s.count}));
        out.frames.push((x, y, h, h));
        x += h + gap;
    }
    if n > shown {
        out.items.push(json!({"type": MORE, "n": n - shown}));
        out.frames.push((x, y, h, h));
        x += h + gap;
    }
    if line {
        let w = (room.0 + room.1 - x).min(max);
        out.items.push(json!({"type": TICKER}));
        out.frames.push((x, y, w, h));
    }
    out
}

/// Which title is on the line. A title that is no longer among the unread gives way at once, and with several the next comes every
/// [`ROTATE_MS`], round to the first again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rotor {
    shown: Option<Title>,
    since: u64,
}

impl Rotor {
    pub fn shown(&self) -> Option<&Title> {
        self.shown.as_ref()
    }

    /// Takes the titles as they are at `now`; true when the title on the line changed.
    pub fn sync(&mut self, titles: &[Title], now: u64) -> bool {
        let next = match self.shown.as_ref().and_then(|s| titles.iter().position(|t| t.text == s.text)) {
            _ if titles.is_empty() => None,
            None => Some(titles[0].clone()),
            Some(at) if titles.len() > 1 && now >= self.since + ROTATE_MS => Some(titles[(at + 1) % titles.len()].clone()),
            Some(_) => return false,
        };
        self.since = now;
        let changed = next != self.shown;
        self.shown = next;
        changed
    }

    /// When the next title is due: only with more than one.
    pub fn due_at(&self, titles: usize) -> Option<u64> {
        (titles > 1 && self.shown.is_some()).then_some(self.since + ROTATE_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(source: &str, title: &str, time: u64) -> Note {
        Note { source: source.into(), icon: format!("{source}-icon"), title: title.into(), time }
    }

    fn unread(sources: usize) -> Unread {
        Unread::of((0..sources).map(|i| note(&format!("s{i}"), &format!("t{i}"), 100 - i as u64)).collect(), sources)
    }

    #[test]
    fn a_source_is_one_glyph_with_how_many_it_has_and_the_newest_first_in_sources_and_titles() {
        let u = Unread::of(vec![note("Doorbell", "Motion", 10), note("TV", "Update", 30), note("Doorbell", "Person", 20), note("Doorbell", "Motion", 5)], 4);
        assert_eq!(u.sources.iter().map(|s| (s.name.as_str(), s.count)).collect::<Vec<_>>(), [("TV", 1), ("Doorbell", 3)]);
        assert_eq!(u.sources[1].icon, "Doorbell-icon", "the glyph of its newest");
        assert_eq!(u.titles.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["Update", "Person", "Motion"], "newest first, what said the same once");
        assert_eq!(u.titles[1].source, "Doorbell");
        assert_eq!(Unread::of(Vec::new(), 0), Unread::default());
    }

    #[test]
    fn no_more_than_six_titles_are_taken_turns_with_and_none_that_are_blank() {
        let u = Unread::of((0..20).map(|i| note("p", &format!("t{i}"), i)).chain([note("p", "", 99)]).collect(), 21);
        assert_eq!(u.titles.len(), MAX_TITLES);
        assert_eq!(u.titles[0].text, "t19");
    }

    #[test]
    fn nothing_unread_places_nothing_and_a_few_sources_get_a_glyph_each_with_the_ticker_after() {
        assert_eq!(place(&Unread::default(), true, (100.0, 900.0), (6.0, 48.0), 8.0, (240.0, 640.0)), Placed::default());
        let p = place(&unread(3), true, (100.0, 900.0), (6.0, 48.0), 8.0, (240.0, 640.0));
        let kinds: Vec<&str> = p.items.iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, [SOURCE, SOURCE, SOURCE, TICKER]);
        assert_eq!(p.frames[0], (100.0, 6.0, 48.0, 48.0));
        assert_eq!(p.frames[1].0, 156.0, "a button and a gap on");
        let t = p.frames[3];
        assert_eq!((t.0, t.2), (100.0 + 3.0 * 56.0, 640.0), "the ticker from after the glyphs, no wider than its most");
    }

    #[test]
    fn past_six_sources_the_rest_are_a_plus_chip_and_it_is_the_chip_that_gives_way_when_room_is_short() {
        let p = place(&unread(9), true, (0.0, 1000.0), (0.0, 48.0), 8.0, (240.0, 640.0));
        let kinds: Vec<&str> = p.items.iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, [SOURCE, SOURCE, SOURCE, SOURCE, SOURCE, SOURCE, MORE, TICKER]);
        assert_eq!(p.items[6]["n"], 3, "nine sources, six shown");
        // the room left for the glyphs is what the ticker's narrowest leaves: three buttons and the chip
        let p = place(&unread(9), true, (0.0, 240.0 + 8.0 + 3.0 * 48.0 + 2.0 * 8.0 + 8.0 + 48.0), (0.0, 48.0), 8.0, (240.0, 640.0));
        let shown = p.items.iter().filter(|i| i["type"] == SOURCE).count();
        assert_eq!(shown, 3);
        assert_eq!(p.items.iter().find(|i| i["type"] == MORE).unwrap()["n"], 6);
        assert!(p.items.iter().any(|i| i["type"] == TICKER), "the ticker keeps its narrowest");
    }

    #[test]
    fn with_the_ticker_off_or_no_room_for_it_the_glyphs_are_alone_and_with_no_room_for_one_nothing_is_drawn() {
        let off = place(&unread(3), false, (0.0, 900.0), (0.0, 48.0), 8.0, (240.0, 640.0));
        assert_eq!(off.items.len(), 3);
        assert!(off.items.iter().all(|i| i["type"] == SOURCE));
        // 200 px: the glyphs take what there is and the ticker has none
        let tight = place(&unread(3), true, (0.0, 200.0), (0.0, 48.0), 8.0, (240.0, 640.0));
        assert!(tight.items.iter().all(|i| i["type"] != TICKER) && !tight.items.is_empty());
        assert_eq!(place(&unread(3), true, (0.0, 40.0), (0.0, 48.0), 8.0, (240.0, 640.0)), Placed::default());
        // no titles, no ticker
        let mut mute = unread(2);
        mute.titles.clear();
        assert!(place(&mute, true, (0.0, 900.0), (0.0, 48.0), 8.0, (240.0, 640.0)).items.iter().all(|i| i["type"] != TICKER));
    }

    #[test]
    fn the_line_takes_the_newest_title_and_turns_to_the_next_every_six_seconds_round_to_the_first() {
        let t = |v: &[&str]| v.iter().map(|s| Title { text: s.to_string(), source: "p".into() }).collect::<Vec<_>>();
        let titles = t(&["a", "b", "c"]);
        let mut r = Rotor::default();
        assert!(r.sync(&titles, 1000) && r.shown().map(|t| t.text.as_str()) == Some("a"));
        assert_eq!(r.due_at(3), Some(1000 + ROTATE_MS));
        assert!(!r.sync(&titles, 1000 + ROTATE_MS - 1), "not yet");
        assert!(r.sync(&titles, 1000 + ROTATE_MS) && r.shown().map(|t| t.text.as_str()) == Some("b"));
        assert!(r.sync(&titles, 1000 + 2 * ROTATE_MS) && r.shown().map(|t| t.text.as_str()) == Some("c"));
        assert!(r.sync(&titles, 1000 + 3 * ROTATE_MS) && r.shown().map(|t| t.text.as_str()) == Some("a"), "round");
        // a newer notification does not cut the turn short while the title on the line is still unread
        assert!(!r.sync(&t(&["z", "a", "b"]), 1000 + 3 * ROTATE_MS + 10));
        // a title that was read gives way at once, to the newest
        assert!(r.sync(&t(&["z", "b"]), 1000 + 3 * ROTATE_MS + 20) && r.shown().map(|t| t.text.as_str()) == Some("z"));
        // one title alone stays, and nothing unread empties the line
        assert!(!r.sync(&t(&["z"]), 1_000_000) && r.due_at(1).is_none());
        assert!(r.sync(&[], 1_000_001) && r.shown().is_none() && !r.sync(&[], 1_000_002));
    }
}
