//! Shared by the integration tests: the fixtures and a scripted `Fetch`.
#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use fetch::{Fetch, FetchError, FetchLimits};
use lumatv_feeds::news::{self, NewsConfig, NewsDoc};
use lumatv_feeds::weather::{self, Units, WeatherConfig, WeatherDoc};

pub const OPEN_METEO: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/open-meteo.json"
));
pub const ZIPPOPOTAM: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/zippopotam-30311.json"
));
pub const BBC_WORLD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bbc-world.xml"
));
pub const BBC_US: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bbc-us.xml"
));
pub const NYT_WORLD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/nyt-world.xml"
));
pub const NYT_US: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/nyt-us.xml"
));

pub const BBC_WORLD_URL: &str = "https://feeds.bbci.co.uk/news/world/rss.xml";
pub const BBC_US_URL: &str = "https://feeds.bbci.co.uk/news/world/us_and_canada/rss.xml";
pub const NYT_WORLD_URL: &str = "https://rss.nytimes.com/services/xml/rss/nyt/World.xml";
pub const NYT_US_URL: &str = "https://rss.nytimes.com/services/xml/rss/nyt/US.xml";

type Reply = (String, Result<Vec<u8>, FetchError>);

/// A `Fetch` that answers by exact URL from a script and records every URL asked for, in order.
pub struct Script {
    replies: Mutex<Vec<Reply>>,
    log: Mutex<Vec<String>>,
}

/// The guard of a lock whether or not a failed test thread poisoned it.
fn held<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Script {
    pub fn new(replies: Vec<(&str, Result<Vec<u8>, FetchError>)>) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|(u, r)| (u.to_owned(), r))
                    .collect(),
            ),
            log: Mutex::new(Vec::new()),
        }
    }

    /// The URLs requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        held(&self.log).clone()
    }
}

impl Fetch for Script {
    fn get(&self, url: &str, _limits: FetchLimits) -> Result<Vec<u8>, FetchError> {
        held(&self.log).push(url.to_owned());
        held(&self.replies)
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| Err(FetchError::Failed(format!("no script for {url}"))))
    }
}

pub fn ok(body: &str) -> Result<Vec<u8>, FetchError> {
    Ok(body.as_bytes().to_vec())
}

pub fn atlanta() -> WeatherConfig {
    WeatherConfig {
        zip: "30311".to_owned(),
        units: Units::Fahrenheit,
    }
}

/// The fixture weather document, fetched at `now_ms`.
pub fn weather_doc(now_ms: u64) -> WeatherDoc {
    let geo = weather::parse_geo(ZIPPOPOTAM.as_bytes()).unwrap();
    weather::parse_forecast(OPEN_METEO.as_bytes(), &geo, &atlanta(), now_ms).unwrap()
}

/// The fixture news document (BBC feeds), fetched at `now_ms`.
pub fn news_doc(now_ms: u64) -> NewsDoc {
    let script = Script::new(vec![
        (BBC_WORLD_URL, ok(BBC_WORLD)),
        (BBC_US_URL, ok(BBC_US)),
    ]);
    news::refresh(&script, &NewsConfig::standard(), now_ms).unwrap()
}
