//! Home's weather and news: saved first, fetched later, only while the screen is on.
//!
//! At start the last good documents are read from `<data>/cache/` and put on the hub at once, whatever the screen is doing, so
//! Home draws them on a cold start without waiting for the network. Then, while the screen is on, each feed is refreshed when it
//! is due: 15 minutes after its last fetch (the news interval is a setting), earlier than that never, and after a failure on the
//! plan's schedule (2, 10, then every 30 minutes) while the saved document keeps being served until it is 6 hours old. A
//! reading older than that is not shown (`expiresAt`); a changed setting (another ZIP, another source) drops the document for
//! the old one at once.
//!
//! The topics `weather` and `news` hold `{"doc": <document|null>, "status": {"failures", "error", "lastAttemptAt"}}`. The
//! documents are the files in the cache directory, byte for byte the same JSON, so QML can read `cache/weather.json` directly
//! before the helper is up and get the same thing the API serves.

use crate::ctx::Ctx;
use crate::screen;
use crate::timeutil::now_ms;
use fetch::Fetch;
use lumatv_feeds::cache::{self, DocCache, FUTURE_SKEW_MS};
use lumatv_feeds::news::{self, NewsConfig, NewsDoc};
use lumatv_feeds::weather::{self, Units, WeatherConfig, WeatherDoc};
use lumatv_feeds::weather_more::{self, MoreDoc};
use lumatv_feeds::{FeedError, REFRESH_MS};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The least time between two fetches of one feed unless a refresh is asked for (guards against a clock that keeps jumping).
const MIN_GAP_MS: u64 = 60_000;

pub const WEATHER_TOPIC: &str = "weather";
pub const NEWS_TOPIC: &str = "news";
/// The full weather view's hours, details and alerts: refreshed with the weather, so that opening the view finds them.
pub const WEATHER_MORE_TOPIC: &str = "weather.more";

/// What wakes the feeds thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// The screen or a setting may have changed.
    Wake,
    /// Fetch both feeds now, even with the screen off and inside a retry delay.
    Refresh,
    Stop,
}

/// A feed's configuration: what identifies its document (`key`) and how often it is refreshed.
struct Cfg<K> {
    key: K,
    interval_ms: u64,
}

/// One kind of feed.
trait Feed {
    type Key: Clone + PartialEq;
    type Doc: Clone + Serialize;
    const TOPIC: &'static str;
    const NAME: &'static str;
    fn cfg(ctx: &Ctx) -> Cfg<Self::Key>;
    fn load(cache: &DocCache, key: &Self::Key, now: u64) -> Option<Self::Doc>;
    fn refresh(
        fetch: &dyn Fetch,
        key: &Self::Key,
        previous: Option<&Self::Doc>,
        now: u64,
    ) -> Result<Self::Doc, FeedError>;
    fn store(cache: &DocCache, doc: &Self::Doc) -> std::io::Result<()>;
    fn fetched_at(doc: &Self::Doc) -> u64;
    /// Puts the document's `staleAt` on the interval this feed is configured with.
    fn set_interval(doc: &mut Self::Doc, interval_ms: u64);
}

struct WeatherFeed;

impl Feed for WeatherFeed {
    type Key = WeatherConfig;
    type Doc = WeatherDoc;
    const TOPIC: &'static str = WEATHER_TOPIC;
    const NAME: &'static str = "weather";

    fn cfg(ctx: &Ctx) -> Cfg<WeatherConfig> {
        let s = ctx.settings.weather();
        Cfg {
            key: WeatherConfig {
                zip: s.zip,
                units: Units::parse(&s.units).unwrap_or(Units::Fahrenheit),
            },
            interval_ms: REFRESH_MS,
        }
    }

    fn load(cache: &DocCache, key: &WeatherConfig, now: u64) -> Option<WeatherDoc> {
        cache.load_weather(key, now)
    }

    fn refresh(
        fetch: &dyn Fetch,
        key: &WeatherConfig,
        previous: Option<&WeatherDoc>,
        now: u64,
    ) -> Result<WeatherDoc, FeedError> {
        weather::refresh(fetch, key, previous, now)
    }

    fn store(cache: &DocCache, doc: &WeatherDoc) -> std::io::Result<()> {
        cache.store_weather(doc)
    }

    fn fetched_at(doc: &WeatherDoc) -> u64 {
        doc.fetched_at
    }

    fn set_interval(doc: &mut WeatherDoc, interval_ms: u64) {
        doc.stale_at = doc.fetched_at.saturating_add(interval_ms);
    }
}

struct WeatherMoreFeed;

impl Feed for WeatherMoreFeed {
    type Key = WeatherConfig;
    type Doc = MoreDoc;
    const TOPIC: &'static str = WEATHER_MORE_TOPIC;
    const NAME: &'static str = "weather details";

    fn cfg(ctx: &Ctx) -> Cfg<WeatherConfig> {
        WeatherFeed::cfg(ctx)
    }

    fn load(cache: &DocCache, key: &WeatherConfig, now: u64) -> Option<MoreDoc> {
        cache.load_weather_more(key, now)
    }

    fn refresh(fetch: &dyn Fetch, key: &WeatherConfig, previous: Option<&MoreDoc>, now: u64) -> Result<MoreDoc, FeedError> {
        weather_more::refresh(fetch, key, previous, now)
    }

    fn store(cache: &DocCache, doc: &MoreDoc) -> std::io::Result<()> {
        cache.store_weather_more(doc)
    }

    fn fetched_at(doc: &MoreDoc) -> u64 {
        doc.fetched_at
    }

    fn set_interval(doc: &mut MoreDoc, interval_ms: u64) {
        doc.stale_at = doc.fetched_at.saturating_add(interval_ms);
    }
}

struct NewsFeed;

impl Feed for NewsFeed {
    type Key = NewsConfig;
    type Doc = NewsDoc;
    const TOPIC: &'static str = NEWS_TOPIC;
    const NAME: &'static str = "news";

    fn cfg(ctx: &Ctx) -> Cfg<NewsConfig> {
        let s = ctx.settings.news();
        let mut key = NewsConfig::standard();
        if s.edition == "nyt" {
            // The other source is the fallback when the chosen one gives nothing.
            std::mem::swap(&mut key.feeds, &mut key.alt);
        }
        Cfg {
            key,
            interval_ms: s.refresh_minutes.saturating_mul(60_000),
        }
    }

    fn load(cache: &DocCache, key: &NewsConfig, now: u64) -> Option<NewsDoc> {
        cache.load_news(key, now)
    }

    fn refresh(
        fetch: &dyn Fetch,
        key: &NewsConfig,
        _previous: Option<&NewsDoc>,
        now: u64,
    ) -> Result<NewsDoc, FeedError> {
        news::refresh(fetch, key, now)
    }

    fn store(cache: &DocCache, doc: &NewsDoc) -> std::io::Result<()> {
        cache.store_news(doc)
    }

    fn fetched_at(doc: &NewsDoc) -> u64 {
        doc.fetched_at
    }

    fn set_interval(doc: &mut NewsDoc, interval_ms: u64) {
        doc.stale_at = doc.fetched_at.saturating_add(interval_ms);
    }
}

/// One feed's state in the thread.
struct Lane<F: Feed> {
    cfg: Option<Cfg<F::Key>>,
    doc: Option<F::Doc>,
    failures: u32,
    last_attempt: u64,
    error: Option<(&'static str, String)>,
    /// A refresh was asked for: due now, screen or not, backoff or not.
    force: bool,
}

impl<F: Feed> Lane<F> {
    fn new() -> Self {
        Self {
            cfg: None,
            doc: None,
            failures: 0,
            last_attempt: 0,
            error: None,
            force: false,
        }
    }

    /// Re-reads the settings. When what identifies the document changed (or at the first call) the saved document for the new
    /// key is taken instead, or none. Returns whether the topic needs publishing.
    fn sync(&mut self, ctx: &Ctx, cache: &DocCache) -> bool {
        let cfg = F::cfg(ctx);
        let changed = self.cfg.as_ref().map(|c| &c.key) != Some(&cfg.key);
        if changed {
            self.doc = F::load(cache, &cfg.key, now_ms());
            self.failures = 0;
            self.error = None;
            self.last_attempt = 0;
            if self.cfg.is_some() {
                crate::log_info!("{}: settings changed; fetching for the new ones", F::NAME);
            }
        }
        // A document that was just loaded carries the interval it was saved with; one in hand follows a changed interval.
        let interval_changed = self
            .cfg
            .as_ref()
            .is_some_and(|c| c.interval_ms != cfg.interval_ms);
        if (changed || interval_changed)
            && let Some(doc) = self.doc.as_mut()
        {
            F::set_interval(doc, cfg.interval_ms);
        }
        self.cfg = Some(cfg);
        changed || interval_changed
    }

    /// When the next fetch is due, Unix milliseconds (0: now).
    fn due_at(&self) -> u64 {
        if self.force {
            return 0;
        }
        let interval = self.cfg.as_ref().map_or(REFRESH_MS, |c| c.interval_ms);
        if self.failures > 0 {
            return cache::next_attempt_ms(None, self.failures, self.last_attempt);
        }
        // The cache's rule, on this feed's own interval: due 5 s before the reading is `interval` old. A reading stamped in the
        // future (the clock was wrong when it was fetched, or is now) has no usable age: it is due at once.
        self.doc.as_ref().map_or(0, |d| {
            let fetched = F::fetched_at(d);
            if fetched > now_ms().saturating_add(FUTURE_SKEW_MS) {
                // But not faster than once a minute, however the clock jumps about.
                return self.last_attempt.saturating_add(MIN_GAP_MS);
            }
            fetched.saturating_add(interval.saturating_sub(5_000))
        })
    }

    fn attempt(&mut self, fetch: &dyn Fetch, cache: &DocCache) {
        let Some(cfg) = &self.cfg else {
            return;
        };
        let now = now_ms();
        self.force = false;
        self.last_attempt = now;
        match F::refresh(fetch, &cfg.key, self.doc.as_ref(), now) {
            Ok(mut doc) => {
                F::set_interval(&mut doc, cfg.interval_ms);
                if let Err(e) = F::store(cache, &doc) {
                    crate::log_warn!(
                        "{}: could not save the reading ({e}); it is served from memory",
                        F::NAME
                    );
                }
                if self.failures > 0 || self.doc.is_none() {
                    crate::log_info!("{}: fetched", F::NAME);
                }
                self.doc = Some(doc);
                self.failures = 0;
                self.error = None;
            }
            Err(e) => {
                let code = e.code();
                if self.error.as_ref().map(|(c, _)| *c) != Some(code) {
                    crate::log_warn!("{}: {e}", F::NAME);
                }
                self.failures = self.failures.saturating_add(1);
                self.error = Some((code, e.to_string()));
            }
        }
    }

    fn publish(&self, ctx: &Ctx) {
        let doc: Value = serde_json::to_value(&self.doc).unwrap_or(Value::Null);
        let error = self.error.as_ref().map_or(
            Value::Null,
            |(code, message)| json!({"code": code, "message": message}),
        );
        let last_attempt = (self.last_attempt > 0).then_some(self.last_attempt);
        ctx.hub.set(
            F::TOPIC,
            json!({"doc": doc, "status": {"failures": self.failures, "error": error, "lastAttemptAt": last_attempt}}),
        );
    }
}

/// Starts the feeds thread.
pub fn spawn(
    ctx: &Ctx,
    rx: Receiver<Cmd>,
    fetch: Arc<dyn Fetch>,
) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("feeds".to_owned())
        .stack_size(512 * 1024)
        .spawn(move || run(&ctx, &rx, fetch.as_ref()))
}

fn run(ctx: &Ctx, rx: &Receiver<Cmd>, fetch: &dyn Fetch) {
    let cache = DocCache::new(ctx.paths.cache_dir());
    let mut weather = Lane::<WeatherFeed>::new();
    let mut news = Lane::<NewsFeed>::new();
    let mut more = Lane::<WeatherMoreFeed>::new();
    // Saved first: what is on disk goes out now, before the screen is known and before any network.
    weather.sync(ctx, &cache);
    news.sync(ctx, &cache);
    more.sync(ctx, &cache);
    weather.publish(ctx);
    news.publish(ctx);
    more.publish(ctx);
    loop {
        if ctx.stop.is_set() {
            return;
        }
        if weather.sync(ctx, &cache) {
            weather.publish(ctx);
        }
        if news.sync(ctx, &cache) {
            news.publish(ctx);
        }
        if more.sync(ctx, &cache) {
            more.publish(ctx);
        }
        let on = screen::is_on(&ctx.hub);
        let mut waiting_for: Option<u64> = None;
        {
            let now = now_ms();
            if (on || weather.force) && now >= weather.due_at() {
                weather.attempt(fetch, &cache);
                weather.publish(ctx);
            }
            if (on || news.force) && now_ms() >= news.due_at() {
                news.attempt(fetch, &cache);
                news.publish(ctx);
            }
            // after the weather, so the two are for the same quarter hour (and a weather that failed has not left the details to waste a request)
            if more.doc.is_none()
                && let Some(w) = &weather.doc
            {
                more.doc = Some(MoreDoc::seed(w));
            }
            if (on || more.force) && now_ms() >= more.due_at() && weather.failures == 0 && weather.doc.is_some() {
                more.attempt(fetch, &cache);
                more.publish(ctx);
            }
        }
        if on {
            waiting_for = Some(weather.due_at().min(news.due_at()).min(more.due_at()));
        }
        let command = match waiting_for {
            Some(due) => {
                let wait = due.saturating_sub(now_ms()).max(1);
                rx.recv_timeout(Duration::from_millis(wait))
            }
            // Screen off: no timer. Anything that matters arrives as a command.
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Cmd::Refresh) => {
                weather.force = true;
                news.force = true;
                more.force = true;
            }
            Ok(Cmd::Wake) | Err(RecvTimeoutError::Timeout) => {}
        }
    }
}
