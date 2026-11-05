//! The sources of listings. Each finds the antenna lineup of a zip code, lists its stations and gives the airings of some of them for
//! a span of time; the guide does the rest the same way for all three.

pub mod gracenote;
pub mod plex;
pub mod sd;

use crate::model::Programme;
use crate::net::{Http, NetError, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

/// Which source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// The guide of the owner's Plex server (Gracenote's data, through Plex's DVR).
    Plex,
    /// Schedules Direct's JSON service: an account, official and paid.
    SchedulesDirect,
    /// The grid behind tvlistings.gracenote.com: free, unofficial.
    Gracenote,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Plex, Source::SchedulesDirect, Source::Gracenote];

    /// The id a setting and a store key use.
    pub fn id(self) -> &'static str {
        match self {
            Source::Plex => "plex",
            Source::SchedulesDirect => "schedules-direct",
            Source::Gracenote => "gracenote",
        }
    }

    pub fn from_id(id: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.id() == id)
    }

    /// What the owner reads.
    pub fn label(self) -> &'static str {
        match self {
            Source::Plex => "Plex",
            Source::SchedulesDirect => "Schedules Direct",
            Source::Gracenote => "Free listings (Gracenote)",
        }
    }
}

/// A lineup of a zip code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lineup {
    /// The source's id of it.
    pub id: String,
    pub name: String,
    /// An antenna lineup, the one a tuner wants.
    pub antenna: bool,
}

/// One station of a lineup: what the guide matches a broadcast channel against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    /// The source's id of it: what its airings name.
    pub key: String,
    /// The virtual channel, when the source gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<(u16, u16)>,
    /// `WSBDT`.
    #[serde(default)]
    pub callsign: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rf: Option<u16>,
}

/// Why a source gave nothing, in words for the owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Net(NetError),
    /// The source turned the credentials down.
    Credentials(String),
    /// The source answered no, in its words (or with a status).
    Refused(String),
    /// The source answered something the guide does not read.
    Shape(String),
    /// The zip code has nothing this source can give.
    Nothing(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Net(e) => e.fmt(f),
            Error::Credentials(why)
            | Error::Refused(why)
            | Error::Shape(why)
            | Error::Nothing(why) => f.write_str(why),
        }
    }
}

impl From<NetError> for Error {
    fn from(e: NetError) -> Self {
        Error::Net(e)
    }
}

/// What a source works with: the host's network and the time, epoch milliseconds.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub net: &'a dyn Http,
    pub now: i64,
}

pub trait Provider: Send {
    fn source(&self) -> Source;

    /// The lineups of `zip`; the guide takes the first antenna one.
    fn lineups(&mut self, cx: Ctx<'_>, zip: &str) -> Result<Vec<Lineup>, Error>;

    /// The stations of `lineup`.
    fn stations(&mut self, cx: Ctx<'_>, lineup: &Lineup) -> Result<Vec<Station>, Error>;

    /// The airings of `stations` (by [`Station::key`]) that start in `[from, to)`, epoch milliseconds, as programmes whose `channel` is
    /// the station's key.
    fn airings(
        &mut self,
        cx: Ctx<'_>,
        lineup: &Lineup,
        stations: &[&Station],
        from: i64,
        to: i64,
    ) -> Result<Vec<Programme>, Error>;
}

/// The answer's body as JSON when the status is a success; otherwise the status, with what the body says when it is short text.
pub(crate) fn json_of(answer: Response, what: &str) -> Result<Value, Error> {
    if !(200..300).contains(&answer.status) {
        return Err(Error::Refused(format!(
            "{what} answered HTTP {}",
            answer.status
        )));
    }
    serde_json::from_slice(&answer.body)
        .map_err(|e| Error::Shape(format!("{what} did not answer JSON ({e})")))
}

pub(crate) fn send(net: &dyn Http, request: &Request<'_>) -> Result<Response, Error> {
    net.send(request).map_err(Error::Net)
}

pub(crate) fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).map_or("", str::trim)
}

/// The airings kept: those of the wanted stations that start in `[from, to)`, by station and start, one per start.
pub(crate) fn keep(
    mut list: Vec<Programme>,
    stations: &[&Station],
    from: i64,
    to: i64,
) -> Vec<Programme> {
    let wanted: HashSet<&str> = stations.iter().map(|s| s.key.as_str()).collect();
    list.retain(|p| {
        (from..to).contains(&p.start)
            && p.end > p.start
            && !p.title.is_empty()
            && wanted.contains(p.channel.as_str())
    });
    list.sort_by(|a, b| (a.channel.as_str(), a.start).cmp(&(b.channel.as_str(), b.start)));
    list.dedup_by(|b, a| a.channel == b.channel && a.start == b.start);
    list
}

#[cfg(test)]
pub(crate) mod fake {
    //! A network of recorded answers: the first rule whose words are all in the URL answers.
    use super::Ctx;
    use crate::net::{Http, Method, NetError, Request, Response};
    use parking_lot::Mutex;

    /// The clock of the recorded answers: 2026-11-05 17:00 UTC.
    pub const NOW: i64 = 1_793_898_000_000;

    pub fn cx(net: &Net) -> Ctx<'_> {
        Ctx { net, now: NOW }
    }

    pub struct Seen {
        pub method: Method,
        pub url: String,
        pub headers: Vec<(String, String)>,
        pub body: Option<String>,
    }

    #[derive(Default)]
    pub struct Net {
        rules: Vec<(Vec<&'static str>, u16, String)>,
        pub seen: Mutex<Vec<Seen>>,
    }

    impl Net {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn on(mut self, words: &[&'static str], status: u16, body: &str) -> Self {
            self.rules.push((words.to_vec(), status, body.to_owned()));
            self
        }

        pub fn urls(&self) -> Vec<String> {
            self.seen
                .lock()
                .iter()
                .map(|s| format!("{} {}", s.method.as_str(), s.url))
                .collect()
        }
    }

    impl Http for Net {
        fn send(&self, r: &Request<'_>) -> Result<Response, NetError> {
            self.seen.lock().push(Seen {
                method: r.method,
                url: r.url.to_owned(),
                headers: r
                    .headers
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                body: r.body.map(|b| String::from_utf8_lossy(b).into_owned()),
            });
            let target = format!("{} {}", r.method.as_str(), r.url);
            self.rules
                .iter()
                .find(|(words, _, _)| words.iter().all(|w| target.contains(w)))
                .map(|(_, status, body)| Response {
                    status: *status,
                    body: body.clone().into_bytes(),
                })
                .ok_or(NetError::Offline)
        }
    }
}
