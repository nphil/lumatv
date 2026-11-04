//! What the channel banner of live TV says, and when: the channel the tuner is on and what is on it, read from LG's own services
//! (`host::channels`), raised as a [`Spec`] of the banner when the channel changes, when Live TV comes to the front and when INFO is pressed.
//!
//! The tuner's channel is followed with `utp.channel/getLastChannelId` (a subscription that needs no pipeline; LG's own channel change
//! follows the same call), and for a channel that differs from the last one the channel's own record (`iepg/getChannelInfo`: number, name,
//! logo) and what is on it and next (`iepg/getCurrentNextEventList`) are asked once. The banner waits for both for [`WAIT_MS`] and then
//! says what it has: the number from the id, "No information" for a programme it was not told of; an answer that comes later refreshes it
//! without holding it longer. An answer about another channel than the tuner's is dropped.
//!
//! The timing is LG's: a change or a launch is shown for five seconds (`BannerInterfaces.qml`: `hideTimer`, zapping and launching). LG's
//! INFO card is open for 30 s and is closed with Back; this banner takes no arrow and no pointer, so INFO holds it for eight seconds, and
//! INFO or Back while it is up ends it.

use super::banner::{HOLD_CHANNEL_MS, HOLD_INFO_MS, Spec};
use crate::host::channels::{self, Channel, Programme, Source};
use crate::host::clock;
use crate::kit::screen::{Effect, Input};
use crate::panels::clock::time_of;
use serde_json::Value;

/// The subscription to the channel the tuner is on.
pub const LAST_TAG: u64 = 0x4A00_0010;
/// The one-shot ask for a channel's record.
pub const INFO_TAG: u64 = 0x4A00_0011;
/// The one-shot ask for what is on a channel and what is next.
pub const NOW_TAG: u64 = 0x4A00_0012;

/// How long the banner waits for the channel's record and its programmes before it says what it has.
pub const WAIT_MS: u64 = 700;
/// How long Live TV in front waits for the tuner to name its channel before the plain input pill is shown instead.
pub const NAME_WAIT_MS: u64 = 1500;
/// A programme list this old, or one that does not reach the present, is asked for again by INFO.
const STALE_MS: u64 = 30_000;
/// What LG's banner says of a programme it has no entry for.
pub const NO_INFORMATION: &str = "No information";

/// A banner that is owed: the words it waits for and until when.
#[derive(Clone, Copy, Debug)]
struct Owed {
    info_banner: bool,
    until: u64,
    channel: bool,
    programmes: bool,
}

/// What the feed asks of the banner.
#[derive(Debug, PartialEq)]
pub enum Raise {
    /// A banner to show, held from now.
    Show(Spec),
    /// The words of the banner that is up, with more in hand: it keeps its hold.
    Refresh(Spec),
}

#[derive(Debug, Default)]
pub struct Out {
    pub effects: Vec<Effect>,
    pub raise: Option<Raise>,
    /// The tuner named a channel other than the last: LG draws its own banner for it about now.
    pub changed: bool,
    /// The tuner named no channel in time: the plain input pill is the answer.
    pub pill: bool,
}

impl Out {
    fn effects(effects: Vec<Effect>) -> Out {
        Out { effects, ..Out::default() }
    }
}

#[derive(Debug, Default)]
pub struct Feed {
    live: bool,
    /// The channel id the tuner last named.
    current: Option<String>,
    channel: Option<Channel>,
    programmes: Vec<Programme>,
    /// When the programmes in hand were asked for.
    asked_at: u64,
    owed: Option<Owed>,
    /// The banner of the last raise is up, and whether it is the INFO one: a later answer refreshes it.
    up: Option<bool>,
    /// Live TV came to the front at this time and the tuner has not named a channel yet.
    nameless_since: Option<u64>,
}

/// The number of a channel id when its record is not in hand: `1_32_2_1_0_0_0` is 2-1, an internet channel's `11_3_100_...` is 100 (LG's own
/// fallback in `ChannelBannerController.setAtscBanenrInfoByChannelId`).
pub fn number_of(id: &str) -> String {
    let parts: Vec<&str> = id.split('_').collect();
    match (parts.first(), parts.get(2), parts.get(3)) {
        (Some(&"11"), Some(n), _) => (*n).to_string(),
        (_, Some(major), Some(minor)) => format!("{major}-{minor}"),
        (_, Some(major), None) => (*major).to_string(),
        _ => String::new(),
    }
}

impl Feed {
    pub fn live(&self) -> bool {
        self.live
    }

    /// The INFO banner is up: INFO and Back end it.
    pub fn info_up(&self) -> bool {
        self.up == Some(true)
    }

    /// Live TV is in front (with the owner's banners on) or it is not: the tuner's channel is followed while it is.
    pub fn set_live(&mut self, live: bool, now: u64) -> Vec<Effect> {
        if live == self.live {
            return Vec::new();
        }
        *self = Feed { live, nameless_since: live.then_some(now), ..Feed::default() };
        if live {
            vec![Effect::Luna { tag: LAST_TAG, uri: channels::LAST_URI.into(), payload: channels::last_request(true), subscribe: true }]
        } else {
            vec![Effect::Cancel { tag: LAST_TAG }]
        }
    }

    /// The banner went (its time ran out, a key ended it): a later answer does not bring it back.
    pub fn gone(&mut self) {
        self.up = None;
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.owed.map(|o| o.until), self.nameless_since.map(|t| t + NAME_WAIT_MS)].into_iter().flatten().min()
    }

    fn ask(&mut self, id: &str, now: u64) -> Vec<Effect> {
        self.asked_at = now;
        vec![
            Effect::Luna { tag: INFO_TAG, uri: channels::INFO_URI.into(), payload: channels::info_request(id), subscribe: false },
            Effect::Luna { tag: NOW_TAG, uri: channels::NOW_NEXT_URI.into(), payload: channels::now_next_request(&[id]), subscribe: false },
        ]
    }

    /// The words of the banner as the answers in hand give them.
    fn spec(&self, info_banner: bool, now: u64) -> Option<Spec> {
        let id = self.current.as_deref()?;
        let now_ms = now as i64;
        let (airing, next) = channels::now_next(&self.programmes, now_ms);
        let mut s = Spec::channel();
        s.info = info_banner;
        s.hold_ms = if info_banner { HOLD_INFO_MS } else { HOLD_CHANNEL_MS };
        match &self.channel {
            Some(c) => {
                s.number = c.number.clone();
                s.call = c.name.clone();
                s.logo = c.logo.clone().unwrap_or_default();
            }
            None => s.number = number_of(id),
        }
        match airing {
            Some(p) => {
                s.title = p.title.clone();
                s.sub = format!("{} \u{2013} {}", time_of(p.start as u64), time_of(p.end as u64));
                let left = ((p.end - now_ms).max(0) + 59_999) / 60_000;
                if left > 0 {
                    s.sub.push_str(&format!(" \u{b7} {left} min left"));
                }
                s.progress = p.progress(now_ms);
                if info_banner {
                    s.detail = p.text.trim().to_string();
                }
                s.chips.extend((!p.rating.is_empty()).then(|| p.rating.clone()));
                s.chips.extend(p.captions.then(|| "CC".to_string()));
                s.chips.extend(p.described.then(|| "AD".to_string()));
            }
            None => s.title = NO_INFORMATION.into(),
        }
        s.next = next.map(|n| n.title.clone()).unwrap_or_default();
        if self.channel.as_ref().is_some_and(|c| c.source == Source::Internet) {
            s.chips.push("Internet".into());
        }
        Some(s)
    }

    /// Raises the banner owed once the words it waits for are in or the time to wait is over; with none owed, an answer refreshes the banner up.
    fn ready(&mut self, now: u64, due: bool) -> Out {
        let Some(o) = self.owed else {
            let Some(info_banner) = self.up else { return Out::default() };
            return Out { raise: self.spec(info_banner, now).map(Raise::Refresh), ..Out::default() };
        };
        if !(o.channel && o.programmes) && !due {
            return Out::default();
        }
        self.owed = None;
        self.up = Some(o.info_banner);
        Out { raise: self.spec(o.info_banner, now).map(Raise::Show), ..Out::default() }
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Out {
        if !self.live {
            return Out::default();
        }
        match input {
            Input::Luna { tag, payload } if *tag == LAST_TAG => self.named(payload, now),
            Input::Luna { tag, payload } if *tag == INFO_TAG => {
                let Some(c) = channels::parse_channel_info(payload).filter(|c| self.current.as_deref() == Some(c.id.as_str())) else { return Out::default() };
                self.channel = Some(c);
                if let Some(o) = self.owed.as_mut() {
                    o.channel = true;
                }
                self.ready(now, false)
            }
            Input::Luna { tag, payload } if *tag == NOW_TAG => {
                let offset = channels::offset_of(payload).unwrap_or_else(|| clock::system_offset_s(now));
                let Ok(list) = channels::parse_programmes(payload, offset) else { return Out::default() };
                let id = self.current.as_deref().unwrap_or_default();
                let mine: Vec<Programme> = list.into_iter().filter(|p| p.channel == id).collect();
                if mine.is_empty() && self.owed.is_none() {
                    return Out::default();
                }
                self.programmes = mine;
                if let Some(o) = self.owed.as_mut() {
                    o.programmes = true;
                }
                self.ready(now, false)
            }
            Input::Tick => {
                let mut out = Out::default();
                if self.owed.is_some_and(|o| now >= o.until) {
                    out = self.ready(now, true);
                }
                if self.nameless_since.is_some_and(|t| now >= t + NAME_WAIT_MS) {
                    self.nameless_since = None;
                    out.pill = true;
                }
                out
            }
            _ => Out::default(),
        }
    }

    /// The tuner named a channel: a banner for it when it is another than the last, its record and its programmes asked for.
    fn named(&mut self, payload: &Value, now: u64) -> Out {
        let Some(id) = channels::parse_last(payload) else { return Out::default() };
        self.nameless_since = None;
        if self.current.as_deref() == Some(id.as_str()) {
            return Out::default();
        }
        self.current = Some(id.clone());
        self.channel = None;
        self.programmes.clear();
        self.owed = Some(Owed { info_banner: false, until: now + WAIT_MS, channel: false, programmes: false });
        Out { effects: self.ask(&id, now), changed: true, ..Out::default() }
    }

    /// INFO: the banner again with what the programme is about, the record and the programmes asked for again when they are old.
    pub fn info_pressed(&mut self, now: u64) -> Out {
        let Some(id) = self.current.clone() else { return Out::default() };
        let stale = now.saturating_sub(self.asked_at) > STALE_MS || channels::now_next(&self.programmes, now as i64).0.is_none();
        self.owed = Some(Owed { info_banner: true, until: now + WAIT_MS, channel: self.channel.is_some(), programmes: !stale });
        if stale {
            return Out::effects(self.ask(&id, now));
        }
        self.ready(now, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000_000;
    const MIN: i64 = 60_000;
    const ID: &str = "1_32_2_1_0_0_0";
    const OTHER: &str = "1_32_9_1_0_0_0";
    const LOGO: &str = "/mnt/lg/cmn_data/channel_logo/major_logo_img/19586_h356x200.png";

    fn last(id: &str) -> Input {
        Input::Luna { tag: LAST_TAG, payload: json!({"returnValue": true, "channelId": id, "mode": "Terrestrial"}) }
    }

    fn record(id: &str, number: &str, name: &str, logo: &str) -> Input {
        Input::Luna {
            tag: INFO_TAG,
            payload: json!({"returnValue": true, "channelList": [{"channelId": id, "channelNumber": number, "channelName": name, "imgUrl": logo, "channelTypeId": 1}]}),
        }
    }

    fn programme(id: &str, title: &str, from: i64, to: i64, text: &str) -> Value {
        json!({"channelId": id, "programId": format!("{id}-{from}"), "programName": title, "description": text,
               "startTime": channels::stamp(NOW as i64 + from * MIN, 0), "endTime": channels::stamp(NOW as i64 + to * MIN, 0),
               "rating": [{"dimension": [{"dim": 0, "dimRatingString": "TV PG"}]}], "isSubtitle": true})
    }

    fn programmes(id: &str) -> Input {
        Input::Luna {
            tag: NOW_TAG,
            payload: json!({"returnValue": true, "programList": [
                programme(id, "Evening News", -10, 20, "Local headlines, weather and sport from the newsroom."),
                programme(id, "Jeopardy!", 20, 50, "")]}),
        }
    }

    fn live() -> Feed {
        let mut f = Feed::default();
        f.set_live(true, NOW);
        f
    }

    fn shown(out: Out) -> Spec {
        match out.raise {
            Some(Raise::Show(s)) => s,
            other => panic!("a banner was to be shown: {other:?}"),
        }
    }

    #[test]
    fn live_tv_in_front_follows_the_tuner_and_the_channel_it_names_raises_the_banner_when_its_words_are_in() {
        let mut f = Feed::default();
        let fx = f.set_live(true, NOW);
        assert!(matches!(&fx[..], [Effect::Luna { tag: LAST_TAG, uri, subscribe: true, .. }] if uri == channels::LAST_URI), "{fx:?}");
        assert!(f.set_live(true, NOW).is_empty(), "once");
        let out = f.update(&last(ID), NOW + 50);
        assert!(out.raise.is_none(), "the words are asked for first");
        let asks: Vec<(u64, &str)> = out.effects.iter().filter_map(|e| if let Effect::Luna { tag, uri, subscribe: false, .. } = e { Some((*tag, uri.as_str())) } else { None }).collect();
        assert_eq!(asks, vec![(INFO_TAG, channels::INFO_URI), (NOW_TAG, channels::NOW_NEXT_URI)]);
        assert!(f.update(&record(ID, "2-1", "WSB-HD", LOGO), NOW + 80).raise.is_none(), "the programmes are still to come");
        let s = shown(f.update(&programmes(ID), NOW + 120));
        assert_eq!((s.kind, s.number.as_str(), s.call.as_str(), s.logo.as_str(), s.title.as_str(), s.next.as_str()), (super::super::banner::Kind::Channel, "2-1", "WSB-HD", LOGO, "Evening News", "Jeopardy!"));
        assert_eq!(s.hold_ms, HOLD_CHANNEL_MS, "LG's five seconds");
        assert!(!s.info && s.detail.is_empty());
        assert!(s.sub.starts_with(&time_of(NOW - 10 * 60_000)) && s.sub.ends_with("20 min left"), "{}", s.sub);
        assert!((s.progress - 10.0 / 30.0).abs() < 0.01, "{}", s.progress);
        assert!(s.chips.contains(&"CC".to_string()) && s.chips.len() == 2, "the rating and the captions: {:?}", s.chips);
        assert_eq!(f.due_at(), None, "nothing is owed once it is shown");
    }

    #[test]
    fn the_channel_the_tuner_is_on_again_is_no_change_and_another_one_is() {
        let mut f = live();
        assert!(f.update(&last(ID), NOW).changed, "the first channel the tuner names is a change: LG draws its banner for it");
        f.update(&record(ID, "2-1", "WSB-HD", ""), NOW);
        shown(f.update(&programmes(ID), NOW));
        assert!(!f.update(&last(ID), NOW + 9000).changed && f.update(&last(ID), NOW + 9000).raise.is_none() && f.update(&last(ID), NOW + 9000).effects.is_empty(), "a push that says the same is nothing");
        let out = f.update(&last(OTHER), NOW + 10_000);
        assert!(out.changed && out.effects.len() == 2, "the new channel is a change, and its record and programmes are asked for");
        f.update(&record(OTHER, "9-1", "WXIA", ""), NOW + 10_050);
        let s = shown(f.update(&programmes(OTHER), NOW + 10_100));
        assert_eq!((s.number.as_str(), s.call.as_str(), s.logo.as_str()), ("9-1", "WXIA", ""), "no logo: no plate, and nothing of the last channel's is left");
    }

    #[test]
    fn what_has_not_come_by_the_wait_is_said_in_plain_words_and_a_late_answer_refreshes_the_banner_without_a_new_hold() {
        let mut f = live();
        f.update(&last(ID), NOW);
        assert_eq!(f.due_at(), Some(NOW + WAIT_MS));
        assert!(f.update(&Input::Tick, NOW + WAIT_MS - 1).raise.is_none(), "not before the wait is over");
        let s = shown(f.update(&Input::Tick, NOW + WAIT_MS));
        assert_eq!((s.number.as_str(), s.call.as_str(), s.title.as_str(), s.progress), ("2-1", "", NO_INFORMATION, 0.0), "the number from the id, no programme");
        match f.update(&record(ID, "2-1", "WSB-HD", LOGO), NOW + 900).raise {
            Some(Raise::Refresh(s)) => assert_eq!((s.call.as_str(), s.logo.as_str()), ("WSB-HD", LOGO)),
            other => panic!("the banner that is up takes the late word: {other:?}"),
        }
        match f.update(&programmes(ID), NOW + 950).raise {
            Some(Raise::Refresh(s)) => assert_eq!(s.title, "Evening News"),
            other => panic!("{other:?}"),
        }
        f.gone();
        assert!(f.update(&record(ID, "2-1", "WSB-HD", LOGO), NOW + 9000).raise.is_none(), "a banner that is gone is not brought back by an answer");
    }

    #[test]
    fn answers_about_another_channel_than_the_tuners_are_dropped() {
        let mut f = live();
        f.update(&last(OTHER), NOW);
        f.update(&last(ID), NOW + 100);
        assert!(f.update(&record(OTHER, "9-1", "WXIA", ""), NOW + 150).raise.is_none());
        assert!(f.update(&programmes(OTHER), NOW + 160).raise.is_none(), "late answers of the channel that was left");
        let s = shown(f.update(&Input::Tick, NOW + 100 + WAIT_MS));
        assert_eq!((s.number.as_str(), s.title.as_str()), ("2-1", NO_INFORMATION), "what is shown is the tuner's, from the id");
    }

    #[test]
    fn info_shows_what_the_programme_is_about_asks_again_when_the_programmes_are_old_and_holds_eight_seconds() {
        let mut f = live();
        f.update(&last(ID), NOW);
        f.update(&record(ID, "2-1", "WSB-HD", LOGO), NOW);
        shown(f.update(&programmes(ID), NOW));
        let s = shown(f.info_pressed(NOW + 6000));
        assert!(s.info && s.hold_ms == HOLD_INFO_MS && s.detail.starts_with("Local headlines"), "{s:?}");
        assert!(f.info_up());
        // a minute later the list is old: it is asked for again, and the banner follows the answer
        let out = f.info_pressed(NOW + STALE_MS + 1);
        assert_eq!(out.effects.iter().filter(|e| matches!(e, Effect::Luna { tag: NOW_TAG, .. })).count(), 1);
        assert!(out.raise.is_none());
        let s = shown(f.update(&programmes(ID), NOW + STALE_MS + 100));
        assert!(s.info && !s.detail.is_empty());
        // nothing is known of a channel before the tuner names one
        assert!(live().info_pressed(NOW).raise.is_none());
    }

    #[test]
    fn leaving_live_tv_lets_go_of_the_tuner_and_coming_back_is_a_launch_again() {
        let mut f = live();
        f.update(&last(ID), NOW);
        assert_eq!(f.set_live(false, NOW + 100), vec![Effect::Cancel { tag: LAST_TAG }]);
        assert!(!f.live() && f.update(&last(OTHER), NOW + 200).effects.is_empty(), "nothing is followed while Live TV is not in front");
        f.set_live(true, NOW + 5000);
        f.update(&last(ID), NOW + 5100);
        f.update(&record(ID, "2-1", "WSB-HD", ""), NOW + 5100);
        assert_eq!(shown(f.update(&programmes(ID), NOW + 5100)).number, "2-1", "the same channel as before still shows when Live TV comes back");
    }

    #[test]
    fn a_tuner_that_names_no_channel_in_time_is_the_plain_pills_turn() {
        let mut f = live();
        assert_eq!(f.due_at(), Some(NOW + NAME_WAIT_MS));
        assert!(!f.update(&Input::Tick, NOW + NAME_WAIT_MS - 1).pill);
        assert!(f.update(&Input::Tick, NOW + NAME_WAIT_MS).pill);
        assert!(!f.update(&Input::Tick, NOW + NAME_WAIT_MS + 1).pill, "once");
        let mut f = live();
        f.update(&Input::Luna { tag: LAST_TAG, payload: json!({"returnValue": false, "errorCode": -106}) }, NOW + 10);
        assert!(f.update(&Input::Tick, NOW + NAME_WAIT_MS).pill, "an answer that is an error names nothing");
        let mut f = live();
        f.update(&last(ID), NOW + 10);
        assert!(!f.update(&Input::Tick, NOW + NAME_WAIT_MS).pill, "a named channel needs no pill");
    }

    #[test]
    fn the_number_of_an_id_is_what_lg_reads_off_it() {
        assert_eq!(number_of("1_32_2_1_0_0_0"), "2-1");
        assert_eq!(number_of("1_32_11_2_0_0_0"), "11-2");
        assert_eq!(number_of("11_3_100_65535_0_0_0"), "100", "an internet channel has no minor number");
        assert_eq!(number_of("garbage"), "");
    }
}
