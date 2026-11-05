//! The free listings: the grid behind tvlistings.gracenote.com (TV listings sites and zap2xml-style tools read the same).
//!
//! Unofficial and fragile: no account, no published terms for this use, and the site answers a program that does not look like a
//! browser with a human-verification page (HTTP 403 or 405), so the requests carry a browser's `User-Agent`. Shapes read on
//! 2026-11-05 for 30311:
//!
//! - `GET /gapzap_webapi/api/Providers/getPostalCodeProviders/USA/<zip>/gapzap/en-us` answers `{Providers: [{type: "OTA", lineupId:
//!   "USA-lineupId-DEFAULT", headendId: "lineupId", device: "", name: "Local Over the Air Broadcast"}, ...]}`.
//! - `GET /api/grid?lineupId=..&headendId=..&device=-&postalCode=<zip>&country=USA&timespan=6&time=<s>&...` answers `{channels:
//!   [{channelId: "19586", channelNo: "2.1", callSign: "WSBDT", events: [{startTime: "2026-11-05T18:00:00Z", endTime, rating,
//!   tags: ["Stereo", "CC"], program: {title, episodeTitle, shortDesc, tmsId}}]}]}`: every station of the zip code's antenna lineup
//!   (156 for 30311), with the events on the air at some time in the six hours after `time` (six is the most it gives; 12 is a 400).
//!   Six hours of the whole lineup is about 700 KB.

use super::{Ctx, Error, Lineup, Provider, Source, Station, json_of, keep, send, text};
use crate::model::{Programme, parse_number};
use crate::net::{Http, Request, encode};
use crate::time::{HOUR_MS, parse_utc};
use serde_json::Value;

const SITE: &str = "https://tvlistings.gracenote.com";
/// The site turns away anything that does not say it is a browser.
const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
/// The longest span one grid request covers.
const SPAN: i64 = 6 * HOUR_MS;
const MAX_GRID_BYTES: usize = 4 << 20;

pub struct Gracenote;

impl Gracenote {
    fn get(&self, net: &dyn Http, url: &str, max_bytes: usize) -> Result<Value, Error> {
        let mut request = Request::get(url)
            .header("User-Agent", BROWSER)
            .header("Accept", "application/json");
        request.max_bytes = max_bytes;
        let answer = send(net, &request)?;
        if matches!(answer.status, 403 | 405) {
            return Err(Error::Refused(format!(
                "the free listings site turned the request away (HTTP {})",
                answer.status
            )));
        }
        json_of(answer, "the free listings site")
    }
}

/// `lineupId|headendId|device|zip`, the words a grid request repeats: what the guide keeps as the lineup's id.
fn lineup_id(p: &Value, zip: &str) -> String {
    let device = match text(p, "device") {
        "" => "-",
        d => d,
    };
    format!(
        "{}|{}|{device}|{zip}",
        text(p, "lineupId"),
        text(p, "headendId")
    )
}

/// A station's key: one station can be on two numbers, so both.
fn key_of(channel: &Value) -> String {
    format!(
        "{}/{}",
        text(channel, "channelId"),
        text(channel, "channelNo")
    )
}

fn parse_grid(grid: &Value) -> Vec<Programme> {
    let mut out = Vec::new();
    for channel in grid["channels"].as_array().into_iter().flatten() {
        let key = key_of(channel);
        for e in channel["events"].as_array().into_iter().flatten() {
            let (Some(start), Some(end)) = (
                parse_utc(text(e, "startTime")),
                parse_utc(text(e, "endTime")),
            ) else {
                continue;
            };
            let program = &e["program"];
            let tags: Vec<&str> = e["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let id = match text(program, "tmsId") {
                "" => text(program, "id"),
                id => id,
            };
            out.push(Programme {
                id: id.to_owned(),
                channel: key.clone(),
                title: text(program, "title").to_owned(),
                // "null" is how the site writes an absent episode title in places
                subtitle: Some(text(program, "episodeTitle"))
                    .filter(|t| *t != "null")
                    .unwrap_or("")
                    .to_owned(),
                text: text(program, "shortDesc").to_owned(),
                start,
                end,
                rating: text(e, "rating").to_owned(),
                captions: tags.contains(&"CC"),
                described: tags.contains(&"DVS"),
            });
        }
    }
    out
}

impl Provider for Gracenote {
    fn source(&self) -> Source {
        Source::Gracenote
    }

    fn lineups(&mut self, cx: Ctx<'_>, zip: &str) -> Result<Vec<Lineup>, Error> {
        let answer = self.get(
            cx.net,
            &format!(
                "{SITE}/gapzap_webapi/api/Providers/getPostalCodeProviders/USA/{}/gapzap/en-us",
                encode(zip)
            ),
            1 << 20,
        )?;
        Ok(answer["Providers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Lineup {
                id: lineup_id(p, zip),
                name: text(p, "name").to_owned(),
                antenna: text(p, "type") == "OTA",
            })
            .collect())
    }

    fn stations(&mut self, cx: Ctx<'_>, lineup: &Lineup) -> Result<Vec<Station>, Error> {
        // The grid is the only list of the lineup's stations: the six hours from now of it.
        let grid = self.grid(cx.net, lineup, cx.now)?;
        Ok(grid["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| Station {
                key: key_of(c),
                number: parse_number(text(c, "channelNo")),
                callsign: text(c, "callSign").to_owned(),
                name: text(c, "affiliateName").to_owned(),
                rf: None,
            })
            .collect())
    }

    fn airings(
        &mut self,
        cx: Ctx<'_>,
        lineup: &Lineup,
        stations: &[&Station],
        from: i64,
        to: i64,
    ) -> Result<Vec<Programme>, Error> {
        let mut out = Vec::new();
        let mut at = from;
        while at < to {
            out.extend(parse_grid(&self.grid(cx.net, lineup, at)?));
            at += SPAN;
        }
        Ok(keep(out, stations, from, to))
    }
}

impl Gracenote {
    fn grid(&self, net: &dyn Http, lineup: &Lineup, at: i64) -> Result<Value, Error> {
        let mut parts = lineup.id.split('|');
        let mut part = || parts.next().unwrap_or("");
        let (lineup_id, headend, device, zip) = (part(), part(), part(), part());
        let url = format!(
            "{SITE}/api/grid?lineupId={}&timespan=6&headendId={}&country=USA&timezone=&device={}&postalCode={}&isOverride=true&time={}&pref=16%2C128&userId=-&aid=orbebb&languagecode=en-us",
            encode(lineup_id),
            encode(headend),
            encode(device),
            encode(zip),
            at / 1000
        );
        self.get(net, &url, MAX_GRID_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::fake::{Net, cx};

    const PROVIDERS: &str = include_str!("../../fixtures/gracenote/providers.json");
    const GRID: &str = include_str!("../../fixtures/gracenote/grid.json");

    fn site() -> Net {
        Net::new()
            .on(&["/getPostalCodeProviders/USA/30311/"], 200, PROVIDERS)
            .on(&["/api/grid?"], 200, GRID)
    }

    #[test]
    fn the_antenna_lineup_is_the_over_the_air_provider_and_the_requests_look_like_a_browser() {
        let net = site();
        let mut g = Gracenote;
        let lineups = g.lineups(cx(&net), "30311").unwrap();
        let antenna = lineups.iter().find(|l| l.antenna).unwrap();
        assert_eq!(antenna.id, "USA-lineupId-DEFAULT|lineupId|-|30311");
        assert_eq!(lineups.iter().filter(|l| l.antenna).count(), 1);
        let seen = net.seen.lock();
        assert!(
            seen[0]
                .headers
                .iter()
                .any(|(k, v)| k == "User-Agent" && v.starts_with("Mozilla/5.0"))
        );
    }

    #[test]
    fn a_day_is_four_grids_of_six_hours_and_only_the_wanted_stations_are_kept() {
        let net = site();
        let mut g = Gracenote;
        let lineup = g
            .lineups(cx(&net), "30311")
            .unwrap()
            .into_iter()
            .find(|l| l.antenna)
            .unwrap();
        let stations = g.stations(cx(&net), &lineup).unwrap();
        assert_eq!(stations.len(), 5);
        let wsb = stations.iter().find(|s| s.callsign == "WSBDT").unwrap();
        assert_eq!((wsb.key.as_str(), wsb.number), ("19586/2.1", Some((2, 1))));

        let from = parse_utc("2026-11-05T00:00:00Z").unwrap();
        let airings = g
            .airings(cx(&net), &lineup, &[wsb], from, from + 24 * HOUR_MS)
            .unwrap();
        let grids: Vec<String> = net
            .urls()
            .into_iter()
            .filter(|u| u.contains("/api/grid?"))
            .skip(1)
            .collect();
        assert_eq!(grids.len(), 4);
        assert!(grids[0].contains("lineupId=USA-lineupId-DEFAULT&timespan=6&headendId=lineupId&country=USA&timezone=&device=-&postalCode=30311"), "{}", grids[0]);
        assert!(grids[1].contains(&format!("time={}", (from + SPAN) / 1000)));
        // four answers of the same events are one list
        assert_eq!(airings.len(), 3);
        let gma = &airings[0];
        assert_eq!(
            (
                gma.id.as_str(),
                gma.title.as_str(),
                gma.rating.as_str(),
                gma.captions
            ),
            ("EP059182660362", "GMA3", "TV-PG", true)
        );
        assert!(gma.text.starts_with("Actress Lacey Chabert"));
    }

    #[test]
    fn an_episode_title_is_the_subtitle_and_a_turned_away_request_says_so() {
        let all = parse_grid(&serde_json::from_str(GRID).unwrap());
        let heat = all
            .iter()
            .find(|p| p.title == "In the Heat of the Night")
            .unwrap();
        assert_eq!(heat.subtitle, "Citizen Trundel");
        let net = Net::new().on(
            &["/getPostalCodeProviders/"],
            405,
            "<!DOCTYPE html><title>Human Verification</title>",
        );
        let err = Gracenote.lineups(cx(&net), "30311").unwrap_err();
        assert!(err.to_string().contains("turned the request away"), "{err}");
    }
}
