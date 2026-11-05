//! Schedules Direct: the official listings service for open-source DVRs (an account, about $35 a year), JSON API 20141201
//! (`github.com/SchedulesDirect/JSON-Service/wiki/API-20141201`).
//!
//! The client's part of the bargain, as the API asks it: log in once (`POST /token` with the SHA-1 of the password, in hex) and keep
//! the token until it expires (24 hours; a hundred logins a day is the limit), look at `/status` and stop while the service says it is
//! offline, add the zip code's antenna lineup to the account only when it is not there yet (six additions a day), ask for schedules a
//! day at a time and for the details of a programme only when its MD5 changed. Every answer carries a `code` (0 is good); an error is
//! never retried at once.
//!
//! Built against the documentation with answers written from it (`fixtures/sd`, the 30311 stations as the Gracenote grid gave them),
//! not against the live service.

use super::{Ctx, Error, Lineup, Provider, Source, Station, keep, send, text};
use crate::model::{Programme, tidy_rating};
use crate::net::{Http, Method, Request, Response, encode};
use crate::time::{DAY_MS, HOUR_MS, date, day_start, parse_utc};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::collections::HashMap;

pub(crate) const HOST: &str = "json.schedulesdirect.org";
const API: &str = "https://json.schedulesdirect.org/20141201";
/// The service asks for the client's name and version in the user agent.
const AGENT: &str = concat!("lumaguide/", env!("CARGO_PKG_VERSION"));
/// What one request may ask for, at most (the service's limit).
const BATCH: usize = 5000;
/// Programme details kept between refreshes; beyond this the cache starts again.
const KEPT: usize = 20_000;
/// A token is used until this close to its end.
const TOKEN_MARGIN_MS: i64 = HOUR_MS;

/// What the guide keeps of a programme.
#[derive(Clone, Debug, Default)]
struct Detail {
    md5: String,
    title: String,
    subtitle: String,
    text: String,
    rating: String,
}

pub struct SchedulesDirect {
    username: String,
    /// The SHA-1 of the password, in lowercase hex: all the service is ever sent, and all that is kept.
    password: String,
    /// The token and when it ends, epoch milliseconds.
    token: Option<(String, i64)>,
    details: HashMap<String, Detail>,
}

/// Why a call failed: the token ran out (it is renewed once), or anything else.
enum Fail {
    Expired,
    Other(Error),
}

impl From<Error> for Fail {
    fn from(e: Error) -> Self {
        Fail::Other(e)
    }
}

impl SchedulesDirect {
    pub fn new(username: &str, password: &str) -> Self {
        let hash = Sha1::digest(password.as_bytes());
        Self {
            username: username.trim().to_owned(),
            password: hash.iter().map(|b| format!("{b:02x}")).collect(),
            token: None,
            details: HashMap::new(),
        }
    }

    /// One request: the service's JSON whatever the HTTP status (an error says its `code` in the body), the token in a header.
    fn call(
        &self,
        net: &dyn Http,
        method: Method,
        path: &str,
        body: Option<&Value>,
        token: Option<&str>,
    ) -> Result<Value, Fail> {
        let url = format!("{API}{path}");
        let body = body.map(|b| b.to_string());
        let mut request = Request::get(&url)
            .header("User-Agent", AGENT)
            .header("Accept", "application/json");
        request.method = method;
        request.max_bytes = 16 << 20;
        if let Some(body) = &body {
            request.body = Some(body.as_bytes());
            request = request.header("Content-Type", "application/json");
        }
        if let Some(token) = token {
            request = request.header("token", token);
        }
        let answer: Response = send(net, &request)?;
        let value: Value = serde_json::from_slice(&answer.body).map_err(|_| {
            Error::Refused(format!(
                "Schedules Direct answered HTTP {} with no JSON",
                answer.status
            ))
        })?;
        match value.get("code").and_then(Value::as_i64) {
            Some(0) | None if (200..300).contains(&answer.status) => Ok(value),
            Some(TOKEN_EXPIRED) => Err(Fail::Expired),
            Some(code) => Err(refusal(code, text(&value, "message")).into()),
            None => Err(Error::Refused(format!(
                "Schedules Direct answered HTTP {}",
                answer.status
            ))
            .into()),
        }
    }

    /// The token, from before while it lasts.
    fn token(&mut self, cx: Ctx<'_>) -> Result<String, Error> {
        if let Some((token, until)) = &self.token
            && *until > cx.now + TOKEN_MARGIN_MS
        {
            return Ok(token.clone());
        }
        let body = json!({"username": self.username, "password": self.password});
        let answer = match self.call(cx.net, Method::Post, "/token", Some(&body), None) {
            Ok(answer) => answer,
            Err(Fail::Expired) => return Err(refusal(TOKEN_EXPIRED, "")),
            Err(Fail::Other(e)) => return Err(e),
        };
        let token = text(&answer, "token");
        if token.is_empty() {
            return Err(Error::Shape("Schedules Direct gave no token".into()));
        }
        let until = answer["tokenExpires"]
            .as_i64()
            .map_or(cx.now + 23 * HOUR_MS, |s| s * 1000);
        self.token = Some((token.to_owned(), until));
        Ok(token.to_owned())
    }

    /// A call with the token; one that ran out is renewed once.
    fn authed(
        &mut self,
        cx: Ctx<'_>,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, Error> {
        let token = self.token(cx)?;
        let first = self.call(cx.net, method, path, body, Some(&token));
        let again = match first {
            Ok(answer) => return Ok(answer),
            Err(Fail::Other(e)) => return Err(e),
            Err(Fail::Expired) => {
                self.token = None;
                let token = self.token(cx)?;
                self.call(cx.net, method, path, body, Some(&token))
            }
        };
        match again {
            Ok(answer) => Ok(answer),
            Err(Fail::Expired) => Err(refusal(TOKEN_EXPIRED, "")),
            Err(Fail::Other(e)) => Err(e),
        }
    }

    /// The details of the programmes whose MD5 is not the one kept, asked for in batches.
    fn fetch_details(&mut self, cx: Ctx<'_>, wanted: &[(String, String)]) -> Result<(), Error> {
        let mut missing: Vec<&str> = wanted
            .iter()
            .filter(|(id, md5)| self.details.get(id).is_none_or(|d| &d.md5 != md5))
            .map(|(id, _)| id.as_str())
            .collect();
        missing.sort_unstable();
        missing.dedup();
        if self.details.len() + missing.len() > KEPT {
            self.details.clear();
        }
        for chunk in missing.chunks(BATCH) {
            let answer = self.authed(cx, Method::Post, "/programs", Some(&json!(chunk)))?;
            for p in answer.as_array().into_iter().flatten() {
                // a programme being made (6001) or gone (6000) is left without details this time
                if p.get("code")
                    .and_then(Value::as_i64)
                    .is_some_and(|c| c != 0)
                {
                    continue;
                }
                let id = text(p, "programID");
                if !id.is_empty() {
                    self.details.insert(id.to_owned(), detail_of(p));
                }
            }
        }
        Ok(())
    }
}

const TOKEN_EXPIRED: i64 = 4006;

/// An error code in words for the owner.
fn refusal(code: i64, message: &str) -> Error {
    let said = |what: &str| {
        if message.is_empty() {
            what.to_owned()
        } else {
            format!("{what} ({message})")
        }
    };
    match code {
        4001 => Error::Credentials(said("the Schedules Direct account expired")),
        4002 | 4003 => Error::Credentials(said(
            "Schedules Direct turned the user name or password down",
        )),
        4004 => Error::Credentials(said("the Schedules Direct account is locked")),
        4005 | 4007 | 4008 => {
            Error::Credentials(said("the Schedules Direct account cannot use the service"))
        }
        TOKEN_EXPIRED => Error::Refused(said("the Schedules Direct login ran out")),
        4009 => Error::Refused(said("too many Schedules Direct logins today")),
        3000 => Error::Refused(said("Schedules Direct is offline for maintenance")),
        4100..=4102 => Error::Refused(said(
            "the lineup cannot be added to the Schedules Direct account",
        )),
        _ => Error::Refused(said(&format!("Schedules Direct said no (code {code})"))),
    }
}

/// The first `description` of a description list.
fn description(p: &Value, size: &str) -> String {
    p["descriptions"][size]
        .as_array()
        .and_then(|l| l.first())
        .map_or("", |d| text(d, "description"))
        .to_owned()
}

fn us_rating(list: &Value) -> String {
    list.as_array()
        .into_iter()
        .flatten()
        .find(|r| text(r, "body") == "USA Parental Rating")
        .map_or_else(String::new, |r| tidy_rating(text(r, "code")))
}

fn detail_of(p: &Value) -> Detail {
    let long = description(p, "description1000");
    Detail {
        md5: text(p, "md5").to_owned(),
        title: p["titles"]
            .as_array()
            .and_then(|t| t.first())
            .map_or("", |t| text(t, "title120"))
            .to_owned(),
        subtitle: text(p, "episodeTitle150").to_owned(),
        text: if long.is_empty() {
            description(p, "description100")
        } else {
            long
        },
        rating: us_rating(&p["contentRating"]),
    }
}

/// One airing of a schedule, before its programme's details are joined to it.
struct Slot {
    station: String,
    id: String,
    md5: String,
    start: i64,
    end: i64,
    rating: String,
    captions: bool,
    described: bool,
}

fn slot_of(station: &str, slot: &Value) -> Option<Slot> {
    let start = parse_utc(text(slot, "airDateTime"))?;
    let seconds = slot["duration"].as_i64()?;
    let audio: Vec<&str> = slot["audioProperties"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Some(Slot {
        station: station.to_owned(),
        id: text(slot, "programID").to_owned(),
        md5: text(slot, "md5").to_owned(),
        start,
        end: start + seconds * 1000,
        rating: us_rating(&slot["ratings"]),
        captions: audio.contains(&"cc"),
        described: audio.contains(&"dvs"),
    })
}

impl Provider for SchedulesDirect {
    fn source(&self) -> Source {
        Source::SchedulesDirect
    }

    fn lineups(&mut self, cx: Ctx<'_>, zip: &str) -> Result<Vec<Lineup>, Error> {
        let answer = self.authed(
            cx,
            Method::Get,
            &format!("/headends?country=USA&postalcode={}", encode(zip)),
            None,
        )?;
        let mut out = Vec::new();
        for headend in answer.as_array().into_iter().flatten() {
            let antenna = text(headend, "transport") == "Antenna";
            for l in headend["lineups"].as_array().into_iter().flatten() {
                out.push(Lineup {
                    id: text(l, "lineup").to_owned(),
                    name: text(l, "name").to_owned(),
                    antenna,
                });
            }
        }
        Ok(out)
    }

    fn stations(&mut self, cx: Ctx<'_>, lineup: &Lineup) -> Result<Vec<Station>, Error> {
        if lineup.id.is_empty()
            || !lineup
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(Error::Shape(format!(
                "not a Schedules Direct lineup: {}",
                lineup.id
            )));
        }
        let status = self.authed(cx, Method::Get, "/status", None)?;
        if status["systemStatus"]
            .as_array()
            .and_then(|s| s.first())
            .is_some_and(|s| text(s, "status") == "Offline")
        {
            return Err(Error::Refused("Schedules Direct is offline".into()));
        }
        let listed = status["lineups"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|l| text(l, "lineup") == lineup.id && l["isDeleted"].as_bool() != Some(true));
        if !listed {
            self.authed(cx, Method::Put, &format!("/lineups/{}", lineup.id), None)?;
        }
        let answer = self.authed(cx, Method::Get, &format!("/lineups/{}", lineup.id), None)?;
        let names: HashMap<&str, &Value> = answer["stations"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| (text(s, "stationID"), s))
            .collect();
        Ok(answer["map"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = text(m, "stationID");
                let station = names.get(id)?;
                let number = match (m["atscMajor"].as_u64(), m["atscMinor"].as_u64()) {
                    (Some(major), Some(minor)) => {
                        Some((u16::try_from(major).ok()?, u16::try_from(minor).ok()?))
                    }
                    _ => None,
                };
                Some(Station {
                    key: id.to_owned(),
                    number,
                    callsign: text(station, "callsign").to_owned(),
                    name: text(station, "name").to_owned(),
                    rf: m["uhfVhf"].as_u64().and_then(|rf| u16::try_from(rf).ok()),
                })
            })
            .collect())
    }

    fn airings(
        &mut self,
        cx: Ctx<'_>,
        _lineup: &Lineup,
        stations: &[&Station],
        from: i64,
        to: i64,
    ) -> Result<Vec<Programme>, Error> {
        let mut days = Vec::new();
        let mut day = day_start(from);
        while day < to {
            days.push(date(day));
            day += DAY_MS;
        }
        let mut ids: Vec<&str> = stations.iter().map(|s| s.key.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut slots: Vec<Slot> = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let ask: Vec<Value> = chunk
                .iter()
                .map(|id| json!({"stationID": id, "date": days}))
                .collect();
            let answer = self.authed(cx, Method::Post, "/schedules", Some(&Value::Array(ask)))?;
            for schedule in answer.as_array().into_iter().flatten() {
                // a station gone (2201), a schedule still being made (7100) or out of range (7020): nothing of it this time
                if schedule
                    .get("code")
                    .and_then(Value::as_i64)
                    .is_some_and(|c| c != 0)
                {
                    continue;
                }
                let station = text(schedule, "stationID");
                slots.extend(
                    schedule["programs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| slot_of(station, s)),
                );
            }
        }
        let wanted: Vec<(String, String)> = slots
            .iter()
            .map(|s| (s.id.clone(), s.md5.clone()))
            .collect();
        self.fetch_details(cx, &wanted)?;
        let out = slots
            .into_iter()
            .filter_map(|s| {
                let detail = self.details.get(&s.id)?;
                Some(Programme {
                    title: detail.title.clone(),
                    subtitle: detail.subtitle.clone(),
                    text: detail.text.clone(),
                    rating: if s.rating.is_empty() {
                        detail.rating.clone()
                    } else {
                        s.rating
                    },
                    id: s.id,
                    channel: s.station,
                    start: s.start,
                    end: s.end,
                    captions: s.captions,
                    described: s.described,
                })
            })
            .collect();
        Ok(keep(out, stations, from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::fake::{Net, cx};

    const TOKEN: &str = include_str!("../../fixtures/sd/token.json");
    const BAD_USER: &str = include_str!("../../fixtures/sd/token-invalid-user.json");
    const STATUS: &str = include_str!("../../fixtures/sd/status.json");
    const HEADENDS: &str = include_str!("../../fixtures/sd/headends.json");
    const ADDED: &str = include_str!("../../fixtures/sd/add-lineup.json");
    const LINEUP: &str = include_str!("../../fixtures/sd/lineup.json");
    const SCHEDULES: &str = include_str!("../../fixtures/sd/schedules.json");
    const PROGRAMS: &str = include_str!("../../fixtures/sd/programs.json");

    fn service() -> Net {
        Net::new()
            .on(&["POST ", "/token"], 200, TOKEN)
            .on(&["/headends?country=USA&postalcode=30311"], 200, HEADENDS)
            .on(&["/status"], 200, STATUS)
            .on(&["PUT ", "/lineups/USA-OTA-30311"], 200, ADDED)
            .on(&["GET ", "/lineups/USA-OTA-30311"], 200, LINEUP)
            .on(&["/schedules"], 200, SCHEDULES)
            .on(&["/programs"], 200, PROGRAMS)
    }

    #[test]
    fn it_logs_in_once_with_the_hash_of_the_password_and_finds_the_antenna_lineup() {
        let net = service();
        let mut sd = SchedulesDirect::new("nitin", "password");
        let lineups = sd.lineups(cx(&net), "30311").unwrap();
        let antenna: Vec<&Lineup> = lineups.iter().filter(|l| l.antenna).collect();
        assert_eq!(antenna.len(), 1);
        assert_eq!(antenna[0].id, "USA-OTA-30311");
        sd.lineups(cx(&net), "30311").unwrap();
        let seen = net.seen.lock();
        let logins: Vec<_> = seen.iter().filter(|s| s.url.ends_with("/token")).collect();
        assert_eq!(logins.len(), 1);
        // sha1("password")
        assert_eq!(
            logins[0].body.as_deref(),
            Some(r#"{"password":"5baa61e4c9b93f3f0682250b6cf8331b7ee68fd8","username":"nitin"}"#)
        );
        assert!(!seen.iter().any(|s| {
            s.body
                .as_deref()
                .is_some_and(|b| b.contains("\"password\":\"password\""))
        }));
        let headends = seen.iter().find(|s| s.url.contains("/headends")).unwrap();
        let origin = crate::net::Origin::of(&headends.url).unwrap();
        assert_eq!(
            (origin.https, origin.host.as_str()),
            (true, Source::SchedulesDirect.host().unwrap())
        );
        assert!(
            headends
                .headers
                .contains(&("token".into(), "f3c0ffee0000000000000000000000aa".into()))
        );
        assert!(
            headends
                .headers
                .iter()
                .any(|(k, v)| k == "User-Agent" && v.starts_with("lumaguide/"))
        );
    }

    #[test]
    fn a_lineup_not_on_the_account_is_added_once_and_its_map_gives_number_rf_and_call_sign() {
        let net = service();
        let mut sd = SchedulesDirect::new("nitin", "password");
        let lineup = Lineup {
            id: "USA-OTA-30311".into(),
            name: "Antenna".into(),
            antenna: true,
        };
        let stations = sd.stations(cx(&net), &lineup).unwrap();
        assert_eq!(stations.len(), 6);
        assert_eq!(
            stations[0],
            Station {
                key: "19586".into(),
                number: Some((2, 1)),
                callsign: "WSBDT".into(),
                name: "WSBDT (WSB-DT)".into(),
                rf: Some(39)
            }
        );
        assert_eq!(
            net.urls().iter().filter(|u| u.starts_with("PUT ")).count(),
            1
        );
    }

    #[test]
    fn a_day_of_schedules_takes_its_details_once_and_skips_a_deleted_station() {
        let net = service();
        let mut sd = SchedulesDirect::new("nitin", "password");
        let lineup = Lineup {
            id: "USA-OTA-30311".into(),
            name: "Antenna".into(),
            antenna: true,
        };
        let stations: Vec<Station> = ["19586", "59472", "91234"]
            .iter()
            .map(|k| Station {
                key: (*k).into(),
                number: None,
                callsign: String::new(),
                name: String::new(),
                rf: None,
            })
            .collect();
        let wanted: Vec<&Station> = stations.iter().collect();
        let day = parse_utc("2026-11-05T00:00:00Z").unwrap();
        let airings = sd
            .airings(cx(&net), &lineup, &wanted, day, day + DAY_MS)
            .unwrap();
        assert_eq!(airings.len(), 4);
        let gh = airings.iter().find(|p| p.id == "EP000018078775").unwrap();
        assert_eq!(
            (gh.title.as_str(), gh.subtitle.as_str(), gh.rating.as_str()),
            ("General Hospital", "Tracy Places Her Trust", "TV-14")
        );
        assert!(gh.captions && gh.described);
        assert_eq!(gh.end - gh.start, HOUR_MS);
        let gma = airings.iter().find(|p| p.id == "EP059182660362").unwrap();
        assert!(
            gma.text
                .starts_with("Actress Lacey Chabert; Good Housekeeping"),
            "the long description: {}",
            gma.text
        );
        let schedules = net
            .seen
            .lock()
            .iter()
            .find(|s| s.url.ends_with("/schedules"))
            .map(|s| s.body.clone().unwrap())
            .unwrap();
        assert_eq!(
            schedules,
            r#"[{"date":["2026-11-05"],"stationID":"19586"},{"date":["2026-11-05"],"stationID":"59472"},{"date":["2026-11-05"],"stationID":"91234"}]"#
        );

        // the same day again: the schedules are asked for, the unchanged details are not
        sd.airings(cx(&net), &lineup, &wanted, day, day + DAY_MS)
            .unwrap();
        assert_eq!(
            net.urls()
                .iter()
                .filter(|u| u.ends_with("/programs"))
                .count(),
            1
        );
    }

    #[test]
    fn a_refused_login_is_the_credentials_and_is_not_retried() {
        let net = Net::new().on(&["/token"], 400, BAD_USER);
        let mut sd = SchedulesDirect::new("nitin", "wrong");
        let err = sd.lineups(cx(&net), "30311").unwrap_err();
        assert!(matches!(err, Error::Credentials(_)), "{err:?}");
        assert!(err.to_string().contains("user name or password"), "{err}");
        assert_eq!(net.urls().len(), 1);
    }
}
