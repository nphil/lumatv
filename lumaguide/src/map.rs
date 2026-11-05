//! Which station of a source is which broadcast channel: by virtual channel first, then by call sign, then by RF channel.
//!
//! The virtual channel (`2.1`) is what the tuner and the listings both print and is right almost always, so it decides alone, even
//! where the broadcast's own name disagrees (30311's 11.11 calls itself `WXIA-TV` and is listed as `WATLDT4`: the listing is the one
//! that is right). A channel left over is matched by its station's call sign and subchannel (a translator on another number), and
//! then by RF channel and subchannel (a station that moved its number), each only to a station no other channel took and only when
//! exactly one fits.

use crate::model::Channel;
use crate::provider::Station;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// How a channel was matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    Number,
    CallSign,
    Rf,
}

/// One channel matched to one station.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The channel's id.
    pub channel: String,
    /// The station's key.
    pub station: String,
    pub how: How,
}

/// The channels of the tuner against the stations of a lineup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelMap {
    pub matched: Vec<Match>,
    /// The ids of the channels no station fits.
    pub unmatched: Vec<String>,
}

impl ChannelMap {
    /// The station keys some channel takes, once each.
    pub fn stations(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.matched.iter().map(|m| m.station.as_str()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The call sign of a station without the suffixes of its service and its subchannel: `WSB-HD`, `WSBDT` and `WSBDT2` are all `WSB`.
/// Anything that is not a US or Canadian call sign is `None` (`BOUNCE`, `Dabl`).
pub fn base_callsign(text: &str) -> Option<String> {
    let mut s: String = text
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase();
    while s.ends_with(|c: char| c.is_ascii_digit()) {
        s.pop();
    }
    for suffix in ["DT", "HD", "TV", "CD", "LD", "LP", "CA", "SD"] {
        if s.len() > 4 && s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    let callsign = (3..=4).contains(&s.len())
        && s.starts_with(['W', 'K', 'C'])
        && s.chars().all(|c| c.is_ascii_uppercase());
    callsign.then_some(s)
}

/// The subchannel a call sign names: `WSBDT2` is 2, `WSBDT` is 1.
fn callsign_minor(text: &str) -> u16 {
    let digits: String = text
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().unwrap_or(1)
}

fn minor_of(s: &Station) -> u16 {
    s.number
        .map_or_else(|| callsign_minor(&s.callsign), |(_, minor)| minor)
}

pub fn match_channels(channels: &[Channel], stations: &[Station]) -> ChannelMap {
    let mut by_number: HashMap<(u16, u16), &Station> = HashMap::new();
    for s in stations {
        if let Some(n) = s.number {
            by_number.entry(n).or_insert(s);
        }
    }
    let mut found: HashMap<&str, (&str, How)> = HashMap::new();
    let mut taken: HashSet<&str> = HashSet::new();
    for c in channels {
        if let Some(s) = by_number.get(&(c.major, c.minor)) {
            found.insert(&c.id, (&s.key, How::Number));
            taken.insert(&s.key);
        }
    }
    let one = |fits: &dyn Fn(&Station) -> bool, taken: &HashSet<&str>| -> Option<&str> {
        let mut candidates = stations
            .iter()
            .filter(|s| !taken.contains(s.key.as_str()) && fits(s));
        let first = candidates.next()?;
        candidates
            .all(|s| s.key == first.key)
            .then_some(first.key.as_str())
    };
    let left: Vec<&Channel> = channels
        .iter()
        .filter(|c| !found.contains_key(c.id.as_str()))
        .collect();
    for c in left {
        let by_callsign = base_callsign(&c.name).and_then(|base| {
            one(
                &|s: &Station| {
                    base_callsign(&s.callsign).as_deref() == Some(base.as_str())
                        && minor_of(s) == c.minor
                },
                &taken,
            )
            .map(|k| (k, How::CallSign))
        });
        let by_rf = || {
            c.rf.and_then(|rf| {
                one(
                    &|s: &Station| s.rf == Some(rf) && minor_of(s) == c.minor,
                    &taken,
                )
                .map(|k| (k, How::Rf))
            })
        };
        if let Some((key, how)) = by_callsign.or_else(by_rf) {
            found.insert(&c.id, (key, how));
            taken.insert(key);
        }
    }
    let mut map = ChannelMap::default();
    for c in channels {
        match found.get(c.id.as_str()) {
            Some((station, how)) => map.matched.push(Match {
                channel: c.id.clone(),
                station: (*station).to_owned(),
                how: *how,
            }),
            None => map.unmatched.push(c.id.clone()),
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, major: u16, minor: u16, name: &str, rf: Option<u16>) -> Channel {
        Channel {
            id: id.into(),
            major,
            minor,
            name: name.into(),
            rf,
        }
    }

    fn station(key: &str, number: Option<(u16, u16)>, callsign: &str, rf: Option<u16>) -> Station {
        Station {
            key: key.into(),
            number,
            callsign: callsign.into(),
            name: String::new(),
            rf,
        }
    }

    #[test]
    fn call_signs_lose_their_service_and_subchannel() {
        assert_eq!(base_callsign("WSB-HD").as_deref(), Some("WSB"));
        assert_eq!(base_callsign("WSBDT2").as_deref(), Some("WSB"));
        assert_eq!(base_callsign("WXIA-TV").as_deref(), Some("WXIA"));
        assert_eq!(base_callsign("WYGA-CD").as_deref(), Some("WYGA"));
        assert_eq!(base_callsign("KCBS"), Some("KCBS".into()));
        assert_eq!(base_callsign("BOUNCE"), None);
        assert_eq!(base_callsign("Dabl"), None);
    }

    #[test]
    fn the_number_decides_then_the_call_sign_then_the_rf_channel() {
        let channels = [
            channel("a", 2, 1, "WSB-HD", Some(39)),
            // the broadcast names the multiplex, the listing the station: the number wins
            channel("b", 11, 11, "WXIA-TV", Some(10)),
            // a translator on another number: its call sign and subchannel find it
            channel("c", 66, 1, "WMYA-CD", Some(30)),
            // a station that moved its number: the RF channel and subchannel find it
            channel("d", 41, 2, "MOVED", Some(22)),
            // nothing fits
            channel("e", 35, 2, "ESPANOL", Some(21)),
        ];
        let stations = [
            station("19586", Some((2, 1)), "WSBDT", Some(39)),
            station("112400", Some((11, 11)), "WATLDT4", None),
            station("70001", Some((45, 1)), "WMYACD", None),
            station("70002", Some((40, 2)), "WXYZDT2", Some(22)),
        ];
        let map = match_channels(&channels, &stations);
        let got: Vec<(&str, &str, How)> = map
            .matched
            .iter()
            .map(|m| (m.channel.as_str(), m.station.as_str(), m.how))
            .collect();
        assert_eq!(
            got,
            [
                ("a", "19586", How::Number),
                ("b", "112400", How::Number),
                ("c", "70001", How::CallSign),
                ("d", "70002", How::Rf)
            ]
        );
        assert_eq!(map.unmatched, ["e"]);
    }

    #[test]
    fn a_station_another_channel_took_by_number_is_not_matched_again_by_call_sign() {
        // 30311's 66.1 calls itself WYGA-CD, as 16.1 does: 16.1 has the station
        let channels = [
            channel("a", 16, 1, "WYGA-CD", None),
            channel("b", 66, 1, "WYGA-CD", None),
        ];
        let stations = [station("68571/16.1", Some((16, 1)), "WYGACD", None)];
        let map = match_channels(&channels, &stations);
        assert_eq!(map.matched.len(), 1);
        assert_eq!(map.unmatched, ["b"]);
    }

    #[test]
    fn two_stations_that_fit_equally_match_neither() {
        let channels = [channel("a", 50, 1, "Mystery", Some(30))];
        let stations = [
            station("1", Some((51, 1)), "WAAA", Some(30)),
            station("2", Some((52, 1)), "WBBB", Some(30)),
        ];
        assert_eq!(match_channels(&channels, &stations).unmatched, ["a"]);
    }
}
