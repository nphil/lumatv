//! Typing a channel number on Live TV, in place of LG's number entry (`inputcommon`'s `SimilarListView`): the digits typed and the channels
//! they match, with what is on each, tuned after a pause or on OK.
//!
//! The rules are LG's where LG has one (`SimilarListView.qml`, `SimilarListInterfaces.qml`): a leading 0 is not typed, the dash comes
//! before a fourth digit of the major number by itself, a minor number has three digits at most, the entry goes 2.5 s after the last key (1.5 s
//! when the one match is the number typed) and tunes the focused match only when it is the number typed or was picked with Up or Down, Back
//! takes the last character away and ends the entry when none is left. One rule is LumaTV's own: digits typed without the dash also match a
//! sub-channel they spell, so `52` finds 5-2 (LG's own entry finds only 52-x for it). The order is the major number typed itself, then the
//! sub-channels it spells, then the majors it begins.
//!
//! The channels are the tuner's (`getChannelList`, without the ones LG hides), asked once for each visit of Live TV; internet channels are
//! reached through the guide.

use super::banner::{Row, Spec};
use crate::host::channels::{self, Channel, Source};
use crate::host::clock;
use crate::kit::screen::Effect;
use serde_json::Value;
use std::collections::HashMap;

/// The tag of the ask of the tuner's channels.
pub const LIST_TAG: u64 = 0x4A00_0015;
/// The tag of the ask of what is on the matches shown.
pub const ON_NOW_TAG: u64 = 0x4A00_0016;
/// The tag of the launch that tunes the chosen channel.
pub const TUNE_TAG: u64 = 0x4A00_0017;
/// LG's `hideTimer`: the entry tunes or goes this long after the last key, sooner when the one match is the number typed.
pub const IDLE_MS: u64 = 2500;
pub const ONE_MS: u64 = 1500;
/// The matches shown at once.
pub const ROWS: usize = 4;
const MAJOR_DIGITS: usize = 3;
const MINOR_DIGITS: usize = 3;
/// What is on a match is asked again after this long without an answer.
const RE_ASK_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum List {
    #[default]
    Unasked,
    Asked,
    Heard,
    Failed,
}

/// What a key or the time did to the entry.
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    pub effects: Vec<Effect>,
    /// The entry ended (tuned or not).
    pub closed: bool,
    /// The channel it tuned.
    pub tuned: Option<String>,
}

#[derive(Debug, Default)]
pub struct Entry {
    typed: String,
    list: List,
    channels: Vec<Channel>,
    /// What is on, by channel id: its title and when it ends.
    on_now: HashMap<String, (String, i64)>,
    asked_at: HashMap<String, u64>,
    matches: Vec<usize>,
    /// How many of the first matches are the number typed (or a sub-channel it spells): the ones the time may tune.
    exact: usize,
    index: usize,
    /// Up or Down chose the focused match.
    moved: bool,
    until: Option<u64>,
}

/// The number's major and minor parts as LG writes them (`2-1`, `100`).
fn parts(number: &str) -> (&str, &str) {
    number.split_once(['-', '.']).unwrap_or((number, ""))
}

/// The channels `typed` matches, best first, and how many of the first are the number typed.
fn matches(typed: &str, channels: &[Channel]) -> (Vec<usize>, usize) {
    let by_number = |v: &mut Vec<usize>| v.sort_by_key(|&i| channels[i].number_key());
    let all = 0..channels.len();
    if let Some((major, minor)) = typed.split_once('-') {
        let (mut same, mut begun): (Vec<usize>, Vec<usize>) =
            all.filter(|&i| parts(&channels[i].number).0 == major && parts(&channels[i].number).1.starts_with(minor)).partition(|&i| !minor.is_empty() && parts(&channels[i].number).1 == minor);
        by_number(&mut same);
        by_number(&mut begun);
        let exact = same.len() + if minor.is_empty() { begun.len() } else { 0 };
        same.extend(begun);
        return (same, exact);
    }
    let (mut major, mut spelt, mut begun) = (Vec::new(), Vec::new(), Vec::new());
    for i in all {
        let (ma, mi) = parts(&channels[i].number);
        if ma == typed {
            major.push(i);
        } else if !mi.is_empty() && (1..typed.len()).any(|at| typed[..at] == *ma && typed[at..] == *mi) {
            spelt.push(i);
        } else if ma.starts_with(typed) {
            begun.push(i);
        }
    }
    by_number(&mut major);
    by_number(&mut spelt);
    by_number(&mut begun);
    let exact = major.len() + spelt.len();
    major.extend(spelt);
    major.extend(begun);
    (major, exact)
}

impl Entry {
    pub fn open(&self) -> bool {
        self.until.is_some()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.until
    }

    /// Live TV left: the channels are asked again on its next visit.
    pub fn reset(&mut self) {
        *self = Entry::default();
    }

    /// The entry ends without tuning: another banner took its place, or what was in front went.
    pub fn cancel(&mut self) {
        let _ = self.close();
    }

    fn close(&mut self) -> Step {
        let was = self.open();
        self.typed.clear();
        self.matches.clear();
        self.exact = 0;
        self.index = 0;
        self.moved = false;
        self.until = None;
        Step { closed: was, ..Step::default() }
    }

    fn tune(&mut self) -> Step {
        let Some(c) = self.matches.get(self.index).map(|&i| self.channels[i].clone()) else { return self.close() };
        let mut step = self.close();
        step.effects.push(Effect::Log(format!("number entry: tunes {}", c.number)));
        step.effects.push(Effect::Luna { tag: TUNE_TAG, uri: channels::LAUNCH_URI.into(), payload: channels::tune_request(&c.id), subscribe: false });
        step.tuned = Some(c.id);
        step
    }

    /// A key while Live TV is in front (the first digit opens the entry); a key that is none of the entry's ends it.
    pub fn key(&mut self, name: &str, repeat: bool, now: u64) -> Step {
        let mut effects = Vec::new();
        let edited = match name {
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
                if repeat || (self.typed.is_empty() && d == "0") {
                    return Step::default();
                }
                match self.typed.split_once('-') {
                    Some((_, minor)) if minor.len() >= MINOR_DIGITS => {}
                    Some(_) => self.typed.push_str(d),
                    None if self.typed.len() >= MAJOR_DIGITS => {
                        self.typed.push('-');
                        self.typed.push_str(d);
                    }
                    None => self.typed.push_str(d),
                }
                if matches!(self.list, List::Unasked | List::Failed) {
                    self.list = List::Asked;
                    effects.push(Effect::Luna { tag: LIST_TAG, uri: channels::LIST_URI.into(), payload: channels::list_request(Source::Tuner), subscribe: false });
                }
                true
            }
            _ if !self.open() => return Step::default(),
            "dash" if !repeat => {
                if self.typed.contains('-') {
                    false
                } else {
                    self.typed.push('-');
                    true
                }
            }
            "back" | "left" if !repeat => {
                self.typed.pop();
                if self.typed.is_empty() {
                    return self.close();
                }
                true
            }
            "up" | "down" => {
                let last = self.matches.len().saturating_sub(1);
                self.index = if name == "up" { self.index.saturating_sub(1) } else { (self.index + 1).min(last) };
                self.moved = true;
                false
            }
            "ok" if !repeat => return self.tune(),
            _ if repeat => return Step::default(),
            _ => return self.close(),
        };
        if edited {
            self.rematch();
        }
        effects.extend(self.ask_on_now(now));
        self.until = Some(now + self.idle());
        Step { effects, ..Step::default() }
    }

    fn idle(&self) -> u64 {
        let one = self.matches.len() == 1 && self.exact == 1 && self.channels[self.matches[0]].number.replace('.', "-") == self.typed;
        if one { ONE_MS } else { IDLE_MS }
    }

    fn rematch(&mut self) {
        let (m, exact) = matches(&self.typed, &self.channels);
        self.matches = m;
        self.exact = exact;
        self.index = 0;
        self.moved = false;
    }

    /// The time: the entry tunes the focused match when it is the number typed or was chosen, and goes either way.
    pub fn tick(&mut self, now: u64) -> Step {
        if !self.until.is_some_and(|t| now >= t) {
            return Step::default();
        }
        if !self.matches.is_empty() && (self.moved || self.index < self.exact) { self.tune() } else { self.close() }
    }

    /// An answer of the list or of what is on; true when what is shown changed.
    pub fn answer(&mut self, tag: u64, payload: &Value, now: u64) -> (bool, Vec<Effect>) {
        match tag {
            LIST_TAG => {
                match channels::parse_channels(payload, Source::Tuner) {
                    Ok(list) => {
                        self.channels = list;
                        self.list = List::Heard;
                    }
                    Err(_) => self.list = List::Failed,
                }
                if !self.open() {
                    return (false, Vec::new());
                }
                if !self.moved {
                    self.rematch();
                }
                (true, self.ask_on_now(now))
            }
            ON_NOW_TAG => {
                let offset = channels::offset_of(payload).unwrap_or_else(|| clock::system_offset_s(now));
                let Ok(list) = channels::parse_programmes(payload, offset) else { return (false, Vec::new()) };
                let at = now as i64;
                let mut changed = false;
                for p in list.into_iter().filter(|p| p.start <= at && at < p.end) {
                    changed |= self.on_now.insert(p.channel.clone(), (p.title.clone(), p.end)) != Some((p.title, p.end));
                }
                (changed && self.open(), Vec::new())
            }
            _ => (false, Vec::new()),
        }
    }

    /// The first of the rows shown: the focused match stays in view.
    fn first_row(&self) -> usize {
        (self.index + 1).saturating_sub(ROWS)
    }

    /// What is on the matches in view, for the ones that do not have it.
    fn ask_on_now(&mut self, now: u64) -> Vec<Effect> {
        let first = self.first_row();
        let at = now as i64;
        let ids: Vec<String> = self.matches[first..(first + ROWS).min(self.matches.len())]
            .iter()
            .map(|&i| self.channels[i].id.clone())
            .filter(|id| !self.on_now.get(id).is_some_and(|(_, end)| *end > at) && !self.asked_at.get(id).is_some_and(|t| now.saturating_sub(*t) < RE_ASK_MS))
            .collect();
        if ids.is_empty() {
            return Vec::new();
        }
        for id in &ids {
            self.asked_at.insert(id.clone(), now);
        }
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        vec![Effect::Luna { tag: ON_NOW_TAG, uri: channels::NOW_NEXT_URI.into(), payload: channels::now_next_request(&refs), subscribe: false }]
    }

    /// The strip: the number typed, the matches in view with the focused one marked, or a word on why there are none.
    pub fn spec(&self, now: u64) -> Spec {
        let mut s = Spec::number();
        s.number = self.typed.clone();
        let first = self.first_row();
        let at = now as i64;
        s.rows = self.matches[first..(first + ROWS).min(self.matches.len())]
            .iter()
            .map(|&i| {
                let c = &self.channels[i];
                let title = self.on_now.get(&c.id).filter(|(_, end)| *end > at).map(|(t, _)| t.clone()).unwrap_or_default();
                Row { number: c.number.clone(), name: c.name.clone(), title }
            })
            .collect();
        s.row = self.index - first;
        if s.rows.is_empty() {
            s.title = match self.list {
                List::Unasked | List::Asked => "Finding channels".into(),
                List::Failed => "The channel list is not available".into(),
                List::Heard => format!("No channel {}", self.typed),
            };
        }
        s
    }

    pub fn typed(&self) -> &str {
        &self.typed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::banner::Kind;
    use serde_json::json;

    const T0: u64 = 1_800_000_000_000;

    fn list(numbers: &[&str]) -> Value {
        let channels: Vec<Value> = numbers.iter().map(|n| json!({"channelId": format!("id{n}"), "channelNumber": n, "channelName": format!("Ch {n}")})).collect();
        json!({"returnValue": true, "channelList": channels})
    }

    fn numbers(e: &Entry) -> Vec<String> {
        e.matches.iter().map(|&i| e.channels[i].number.clone()).collect()
    }

    /// An entry that has heard `numbers` and has `typed` typed.
    fn typed(numbers: &[&str], keys: &[&str]) -> Entry {
        let mut e = Entry::default();
        for (i, k) in keys.iter().enumerate() {
            e.key(k, false, T0 + i as u64);
        }
        e.answer(LIST_TAG, &list(numbers), T0 + 50);
        e
    }

    fn tuned(step: &Step) -> Option<&str> {
        step.tuned.as_deref()
    }

    #[test]
    fn digits_match_the_major_then_the_sub_channel_they_spell_then_the_majors_they_begin_and_the_dash_narrows_to_one_major() {
        let all = ["2-1", "5-1", "5-2", "5-10", "52-1", "11-2", "520"];
        assert_eq!(numbers(&typed(&all, &["5"])), ["5-1", "5-2", "5-10", "52-1", "520"]);
        let e = typed(&all, &["5", "2"]);
        assert_eq!((numbers(&e), e.exact), (vec!["52-1".to_string(), "5-2".into(), "520".into()], 2), "52 is 52-x first, then 5-2, then what it begins");
        assert_eq!(numbers(&typed(&all, &["5", "dash"])), ["5-1", "5-2", "5-10"]);
        let e = typed(&all, &["5", "dash", "1"]);
        assert_eq!((numbers(&e), e.exact), (vec!["5-1".to_string(), "5-10".into()], 1), "the minor typed itself first");
        assert_eq!(numbers(&typed(&all, &["1"])), ["11-2"], "a major it only begins");
    }

    #[test]
    fn a_leading_zero_is_not_typed_the_dash_comes_before_a_fourth_digit_and_back_takes_away_then_ends() {
        let mut e = Entry::default();
        assert_eq!(e.key("0", false, T0), Step::default());
        assert!(!e.open(), "0 alone opens nothing");
        let first = e.key("1", false, T0);
        assert!(matches!(&first.effects[..], [Effect::Luna { tag: LIST_TAG, .. }]), "the first digit asks for the channels: {first:?}");
        for d in ["2", "3", "4", "5", "6", "7", "8"] {
            assert!(e.key(d, false, T0 + 10).effects.is_empty(), "the list is asked once");
        }
        assert_eq!(e.typed(), "123-456", "a fourth major digit is a minor one, and a minor has three");
        assert!(!e.key("dash", false, T0 + 20).closed && e.typed() == "123-456", "one dash");
        for _ in 0..6 {
            e.key("back", false, T0 + 30);
        }
        assert_eq!(e.typed(), "1");
        assert!(e.key("left", false, T0 + 40).closed && !e.open(), "Left takes the last away, and the entry ends with it");
        assert!(e.key("dash", false, T0 + 50) == Step::default() && !e.open(), "the dash opens nothing");
    }

    #[test]
    fn the_time_tunes_the_number_typed_or_a_match_chosen_and_ok_tunes_the_focus_through_the_launch_that_keeps_lgs_banner_away() {
        let all = ["5-1", "5-2", "11-2", "11-11"];
        let mut e = typed(&all, &["5"]);
        assert_eq!(e.due_at(), Some(T0 + IDLE_MS));
        assert!(e.tick(T0 + IDLE_MS - 1) == Step::default() && e.open());
        let step = e.tick(T0 + IDLE_MS);
        assert_eq!(tuned(&step), Some("id5-1"));
        assert!(step.closed && !e.open());
        assert!(step.effects.iter().any(|f| matches!(f, Effect::Luna { tag: TUNE_TAG, uri, payload, .. } if uri == channels::LAUNCH_URI && payload["params"]["type"] == "channels" && payload["params"]["channelId"] == "id5-1")), "{step:?}");
        // 1 matches only majors it begins: the time tunes nothing, unless Down chose one
        let mut e = typed(&all, &["1"]);
        let step = e.tick(T0 + IDLE_MS);
        assert!(step.closed && step.tuned.is_none());
        let mut e = typed(&all, &["1"]);
        e.key("down", false, T0 + 100);
        e.key("up", false, T0 + 200);
        assert_eq!(e.index, 0);
        assert_eq!(tuned(&e.tick(T0 + 200 + IDLE_MS)), Some("id11-2"), "chosen with the arrows");
        let mut e = typed(&all, &["1"]);
        e.key("down", false, T0 + 100);
        assert_eq!(tuned(&e.key("ok", false, T0 + 200)), Some("id11-11"), "OK tunes the focus at once");
        // the one match that is the number typed goes sooner, as LG's does
        let mut e = typed(&all, &["5", "dash", "2"]);
        e.key("dash", false, T0 + 100);
        assert_eq!(e.due_at(), Some(T0 + 100 + ONE_MS));
        // a key that is none of the entry's ends it
        let mut e = typed(&all, &["5"]);
        assert!(e.key("chup", false, T0 + 100).closed);
    }

    #[test]
    fn the_strip_shows_four_matches_with_what_is_on_and_says_why_there_are_none() {
        let mut e = Entry::default();
        e.key("5", false, T0);
        assert_eq!(e.spec(T0).title, "Finding channels");
        let (changed, fx) = e.answer(LIST_TAG, &list(&["5-1", "5-2", "5-3", "5-4", "5-5"]), T0 + 10);
        assert!(changed);
        let ask = fx.iter().find_map(|f| match f {
            Effect::Luna { tag: ON_NOW_TAG, payload, .. } => Some(payload["channelId"].clone()),
            _ => None,
        });
        assert_eq!(ask, Some(json!(["id5-1", "id5-2", "id5-3", "id5-4"])), "what is on the rows in view");
        let stamp = |min: i64| channels::stamp(T0 as i64 + min * 60_000, 0);
        let on = json!({"returnValue": true, "programList": [{"channelId": "id5-2", "programId": "p", "programName": "News", "startTime": stamp(-5), "endTime": stamp(25)}]});
        assert!(e.answer(ON_NOW_TAG, &on, T0 + 20).0);
        let s = e.spec(T0 + 30);
        assert_eq!((s.kind, s.number.as_str(), s.rows.len(), s.row), (Kind::Number, "5", ROWS, 0));
        assert_eq!(s.rows[1], Row { number: "5-2".into(), name: "Ch 5-2".into(), title: "News".into() });
        for _ in 0..4 {
            e.key("down", false, T0 + 40);
        }
        let s = e.spec(T0 + 40);
        assert_eq!((s.rows[0].number.as_str(), s.row), ("5-2", 3), "the focus stays in view");
        e.key("9", false, T0 + 50);
        assert_eq!(e.spec(T0 + 50).title, "No channel 59");
    }
}
