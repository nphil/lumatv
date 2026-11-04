//! A make-believe channel list and listings, for the host harness (`lumatv-ui --host --guide-fixture DIR`) and the surface's tests: the set's
//! 57 antenna channels and 459 internet ones, programmes that tile every channel's days, a fifth of the channels with no listings at all (as on
//! the TV, where most of the internet's have none), and after the 57 the cases a plain tiling does not make (a ticker of programmes of 5 to
//! 30 minutes, a long name, no logo; [`EDGES`]). Names, titles and logos are invented. It answers the three calls the guide makes of LG's
//! services (`iepg/getChannelList`, `iepg/getEventList`, `utp.channel/getLastChannelId`) in the shapes the TV gives them (`host/channels.rs`).

use crate::host::channels as ch;
use crate::host::clock;
use serde_json::{Value, json};
use std::path::Path;

const MIN: i64 = 60_000;
/// The first instant the listings tile from (2026-01-01 00:00 UTC): a channel's programmes are the same whatever window is asked for.
const ANCHOR: i64 = 1_767_225_600_000;

const TITLES: [&str; 28] = [
    "Evening News",
    "Morning Report",
    "Cooking With Fire",
    "Mountain Weather",
    "Quiet Hours",
    "Game Night",
    "Cartoon Hour",
    "The Harbour Lights",
    "Sports Roundup",
    "Garden Talk",
    "Cars And Engines",
    "Late Edition",
    "Kitchen Table",
    "The Long Walk Home",
    "Wild Rivers",
    "Money Matters",
    "Space Notes",
    "Street Music Live",
    "The Great Bake",
    "Detective Marlow",
    "Local Stories",
    "Science Hour",
    "Night Owls",
    "Family Quiz",
    "The Extraordinarily Long Documentary About The Rivers, Lakes And Mountains Of The Far North, Part Two",
    "Tonight At Nine",
    "Winter Cabin",
    "A",
];

const ABOUT: [&str; 4] = [
    "A look at the week's stories, with the people who made them and the places they happened.",
    "Two neighbours disagree about a fence, a tree and a very old recipe. Neither of them is right, and both of them are sure.",
    "The crew follows the river from the first snow to the last ice, and finds out what lives under it.",
    "An hour of music, interviews and one surprisingly good joke about the weather.",
];

const RATINGS: [&str; 4] = ["TV-G", "TV-PG", "TV-14", "TV-14"];
const DURATIONS: [i64; 9] = [30, 30, 30, 45, 60, 60, 60, 90, 120];
/// What a ticker channel runs: breaks of 5 to 30 minutes, the shortest most often.
const SHORT_DURATIONS: [i64; 11] = [5, 10, 15, 5, 5, 10, 20, 15, 30, 5, 10];
const SHORT_TITLES: [&str; 10] = ["Weather", "Headlines", "Traffic And Weather Together", "Markets", "News Brief", "Sports Flash", "Ad Break", "Lotto", "Late Edition", "A"];
/// The antenna channels that tile the days as the plain set does; the ones after them are [`EDGES`].
const ANTENNA: usize = 57;

/// What a channel's row holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Listings {
    /// Programmes of 30 to 120 minutes.
    Tiled,
    /// Programmes of 5 to 30 minutes.
    Short,
    /// No listings at all.
    None,
}

/// A channel the real list has and a plain tiling does not make: antenna channel `ANTENNA + i` is `EDGES[i]`.
struct Edge {
    number: &'static str,
    name: &'static str,
    logo: bool,
    listings: Listings,
}

const EDGES: [Edge; 5] = [
    Edge { number: "59-1", name: "KBRF-HD", logo: true, listings: Listings::Short },
    Edge { number: "60-1", name: "WSVP-HD Southern Valley Public Television", logo: true, listings: Listings::Tiled },
    Edge { number: "61-1", name: "KNLG-HD", logo: false, listings: Listings::Tiled },
    Edge { number: "62-1", name: "WWWWWWWWWWWW-HD", logo: false, listings: Listings::None },
    Edge { number: "199-99", name: "KMAX-HD", logo: false, listings: Listings::Short },
];

const SYLLABLES: [&str; 20] = ["AR", "BEN", "CO", "DAL", "EV", "FAR", "GIL", "HAL", "IRV", "JOR", "KEL", "LAN", "MOR", "NEL", "OTT", "PEN", "QUI", "ROS", "SAN", "TAL"];

pub struct Fixture {
    logos: Vec<String>,
    antenna: usize,
    internet: usize,
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn next(state: u64) -> u64 {
    // SplitMix64
    let z = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Fixture {
    /// The PNGs of `logo_dir` (when there is one) are the channels' logos, in turn.
    pub fn new(logo_dir: Option<&Path>) -> Self {
        let mut logos: Vec<String> = logo_dir
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "png"))
            .map(|p| p.display().to_string())
            .collect();
        logos.sort();
        Self { logos, antenna: ANTENNA + EDGES.len(), internet: 459 }
    }

    /// A set of `antenna` and `internet` channels (the tests make small ones).
    pub fn sized(antenna: usize, internet: usize) -> Self {
        Self { logos: Vec::new(), antenna, internet }
    }

    pub fn antenna_id(i: usize) -> String {
        format!("1_32_{}_1_0_0_0", 2 + i)
    }

    /// The edge case a channel id is, when it is one.
    fn edge(id: &str) -> Option<&'static Edge> {
        let at: usize = id.strip_prefix("1_32_")?.split('_').next()?.parse().ok()?;
        EDGES.get(at.checked_sub(2 + ANTENNA)?)
    }

    fn logo(&self, i: usize) -> &str {
        if self.logos.is_empty() { "" } else { &self.logos[i % self.logos.len()] }
    }

    fn channels(&self, internet: bool) -> Value {
        let list: Vec<Value> = if internet {
            (0..self.internet)
                .map(|i| json!({"channelId": format!("11_3_{}_65535_0_0_0", 100 + i), "channelNumber": (100 + i).to_string(), "channelName": format!("Channel {}", 100 + i),
                                "imgUrl": self.logo(i + 3), "channelMode": "Internet", "channelTypeId": 20, "skipped": false, "Invisible": false}))
                .collect()
        } else {
            (0..self.antenna)
                .map(|i| {
                    let edge = i.checked_sub(ANTENNA).and_then(|e| EDGES.get(e));
                    let name = edge.map_or_else(|| format!("{}{}-HD", if i % 2 == 0 { "K" } else { "W" }, SYLLABLES[i % SYLLABLES.len()]), |e| e.name.to_string());
                    let number = edge.map_or_else(|| format!("{}-1", 2 + i), |e| e.number.to_string());
                    let logo = if edge.is_some_and(|e| !e.logo) { "" } else { self.logo(i) };
                    json!({"channelId": Self::antenna_id(i), "channelNumber": number, "channelName": name, "imgUrl": logo,
                           "channelMode": "Terrestrial", "channelTypeId": 1, "skipped": false, "Invisible": false})
                })
                .collect()
        };
        json!({"returnValue": true, "channelListCount": list.len(), "channelList": list})
    }

    /// The programmes of `id` that meet `[from, to)`, as `getEventList` gives them, on a clock `offset_s` east of UTC.
    fn programmes(&self, id: &str, from: i64, to: i64, offset_s: i32) -> Vec<Value> {
        let h = fnv(id);
        let edge = Self::edge(id);
        if edge.map_or(h % 5 == 4, |e| e.listings == Listings::None) {
            return Vec::new();
        }
        let short = edge.is_some_and(|e| e.listings == Listings::Short);
        let (durations, titles): (&[i64], &[&str]) = if short { (&SHORT_DURATIONS, &SHORT_TITLES) } else { (&DURATIONS, &TITLES) };
        let mut out = Vec::new();
        let (mut t, mut rng) = (ANCHOR + (h >> 8) as i64 % 4 * 15 * MIN, h);
        while t < to {
            rng = next(rng);
            let end = t + durations[(rng >> 33) as usize % durations.len()] * MIN;
            if end > from {
                // the channel the guide opens on and the one under it carry the long title now and then, for the pictures of long titles
                let long = matches!(id, "1_32_7_1_0_0_0" | "1_32_8_1_0_0_0") && (rng >> 24) % 2 == 0;
                let title = if long { TITLES[24] } else { titles[(rng >> 20) as usize % titles.len()] };
                out.push(json!({
                    "channelId": id, "programId": format!("{id}@{t}"), "programName": title, "description": ABOUT[(rng >> 40) as usize % ABOUT.len()],
                    "startTime": ch::stamp(t, 0), "endTime": ch::stamp(end, 0), "localStartTime": ch::stamp(t, offset_s), "localEndTime": ch::stamp(end, offset_s),
                    "duration": (end - t) / 1000, "isSubtitle": (rng >> 50) % 3 != 0, "isAD": false,
                    "rating": [{"dimension": [{"dim": "TV", "dimRatingString": format!("TV {}", RATINGS[(rng >> 52) as usize % RATINGS.len()])}]}],
                }));
            }
            t = end;
        }
        out
    }

    /// The answer to a call of the guide, or none for a call the fixture does not stand in for.
    pub fn answer(&self, uri: &str, payload: &Value, now: u64) -> Option<Value> {
        if uri == ch::LIST_URI {
            let internet = payload.get("channelMode").and_then(Value::as_array).is_some_and(|m| m.iter().any(|v| v == "IP"));
            Some(self.channels(internet))
        } else if uri == ch::LAST_URI {
            Some(json!({"returnValue": true, "channelId": Self::antenna_id(5), "mode": "INPUTSOURCE_TERRESTRIAL_DIGITAL"}))
        } else if uri == ch::EVENTS_URI {
            let offset_s = clock::system_offset_s(now);
            let from = ch::parse_stamp(payload.get("localStartTime")?.as_str()?, offset_s)?;
            let to = ch::parse_stamp(payload.get("localEndTime")?.as_str()?, offset_s)?;
            let list: Vec<Value> = payload.get("channelId")?.as_array()?.iter().filter_map(Value::as_str).flat_map(|id| self.programmes(id, from, to, offset_s)).collect();
            Some(json!({"returnValue": true, "programListCount": list.len(), "programList": list}))
        } else {
            None
        }
    }

    /// Whether `uri` is one of the calls the fixture answers.
    pub fn stands_in_for(uri: &str) -> bool {
        [ch::LIST_URI, ch::LAST_URI, ch::EVENTS_URI].contains(&uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listings_are_the_same_whatever_window_they_are_asked_for_and_tile_the_time() {
        let f = Fixture::sized(8, 0);
        let id = Fixture::antenna_id(0);
        let day = 24 * 60 * MIN;
        let t0 = ANCHOR + 200 * day;
        let a = f.programmes(&id, t0, t0 + 6 * 60 * MIN, 0);
        let b = f.programmes(&id, t0 + 2 * 60 * MIN, t0 + 12 * 60 * MIN, 0);
        let overlap: Vec<_> = a.iter().filter(|p| b.contains(p)).collect();
        assert!(overlap.len() >= 2, "the same programmes on the same channel: {}", overlap.len());
        for pair in a.windows(2) {
            assert_eq!(pair[0]["endTime"], pair[1]["startTime"], "no gap and no overlap");
        }
    }

    #[test]
    fn the_answers_are_in_the_shapes_the_data_layer_reads() {
        let f = Fixture::sized(5, 7);
        let list = f.answer(ch::LIST_URI, &ch::list_request(ch::Source::Tuner), 0).unwrap();
        assert_eq!(ch::parse_channels(&list, ch::Source::Tuner).unwrap().len(), 5);
        let net = f.answer(ch::LIST_URI, &ch::list_request(ch::Source::Internet), 0).unwrap();
        assert_eq!(ch::parse_channels(&net, ch::Source::Internet).unwrap().len(), 7);
        assert_eq!(ch::parse_last(&f.answer(ch::LAST_URI, &json!({}), 0).unwrap()), Some(Fixture::antenna_id(5)));
        let from = ANCHOR + 300 * 24 * 60 * MIN;
        let to = from + 48 * 60 * MIN;
        let ids: Vec<String> = (0..5).map(Fixture::antenna_id).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let offset = clock::system_offset_s(0);
        let events = f.answer(ch::EVENTS_URI, &ch::events_request(&refs, from, to, offset), 0).unwrap();
        let programmes = ch::parse_programmes(&events, offset).unwrap();
        assert!(programmes.len() > 40 && programmes.iter().all(|p| p.end > from && p.start < to && p.title.is_ascii()));
        assert!(programmes.iter().any(|p| p.title.len() > 60), "a long title among them");
    }

    #[test]
    fn the_edge_channels_follow_the_plain_ones_with_a_ticker_a_long_name_and_no_logo_and_no_listings() {
        let f = Fixture::new(None);
        assert_eq!(f.antenna, ANTENNA + EDGES.len());
        let list = f.channels(false);
        let channels = list["channelList"].as_array().unwrap();
        assert_eq!(channels[ANTENNA + 1]["channelName"], EDGES[1].name);
        let t0 = ANCHOR + 200 * 24 * 60 * MIN;
        let breaks = f.programmes(&Fixture::antenna_id(ANTENNA), t0, t0 + 6 * 60 * MIN, 0);
        assert!(breaks.len() > 12 && breaks.iter().all(|p| p["duration"].as_i64().is_some_and(|s| (5 * 60..=30 * 60).contains(&s))), "a ticker of breaks");
        assert!(f.programmes(&Fixture::antenna_id(ANTENNA + 3), t0, t0 + 6 * 60 * MIN, 0).is_empty(), "a channel with no listings");
        let plain = f.programmes(&Fixture::antenna_id(1), t0, t0 + 6 * 60 * MIN, 0);
        assert!(plain.iter().all(|p| p["duration"].as_i64().is_some_and(|s| s >= 30 * 60)), "the plain channels keep their tiling");
        let mut logos = Fixture::new(None);
        logos.logos = vec!["/logos/one.png".into()];
        let with = logos.channels(false);
        let shown = |i: usize| with["channelList"][i]["imgUrl"].as_str().unwrap().to_string();
        assert_eq!((shown(ANTENNA), shown(ANTENNA + 2)), ("/logos/one.png".to_string(), String::new()), "a channel of the edge has no logo when it is meant to have none");
    }
}
