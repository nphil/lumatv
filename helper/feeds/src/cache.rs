//! The saved-first cache: the last good document on disk, how old it may be, and when to try again.
//!
//! Home shows what is saved while a refresh runs, so a cold start draws at once. Every function takes the time as an
//! argument; nothing here reads a clock.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::news::{NewsConfig, NewsDoc};
use crate::weather::{WeatherConfig, WeatherDoc};
use crate::{EXPIRE_MS, REFRESH_MS, RETRY_MS, SCHEMA};

/// The largest document file that is read; real ones are a few KiB, so a bigger file is damaged or foreign.
const MAX_DOC_BYTES: u64 = 1024 * 1024;

/// How far a document's `fetched_at` may lie ahead of the clock before it is distrusted: the clock was wrong (an unset RTC)
/// when the file was written.
const FUTURE_SKEW_MS: u64 = 300_000;

/// QML shows Home again and refetches when a reading is within this much of the refresh interval.
const REFETCH_EARLY_MS: u64 = 5_000;

/// How usable a saved reading is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Younger than [`REFRESH_MS`].
    Fresh,
    /// Older than [`REFRESH_MS`] but younger than [`EXPIRE_MS`]: shown while a refetch runs.
    Stale,
    /// Never shown: older than [`EXPIRE_MS`], or stamped more than 5 minutes in the future.
    Expired,
}

/// The freshness of a reading fetched at `fetched_at`, at `now_ms`. A stamp up to 5 minutes ahead counts as age 0
/// (ordinary clock skew); further ahead it is expired.
pub fn freshness(fetched_at: u64, now_ms: u64) -> Freshness {
    let age = match now_ms.checked_sub(fetched_at) {
        Some(age) => age,
        None if fetched_at - now_ms > FUTURE_SKEW_MS => return Freshness::Expired,
        None => 0,
    };
    if age < REFRESH_MS {
        Freshness::Fresh
    } else if age < EXPIRE_MS {
        Freshness::Stale
    } else {
        Freshness::Expired
    }
}

/// The wait after `consecutive_failures` failed refreshes in a row: none for 0, then [`RETRY_MS`], the last entry for 3 or
/// more.
pub fn retry_delay_ms(consecutive_failures: u32) -> u64 {
    match consecutive_failures {
        0 => 0,
        n => RETRY_MS[(n as usize - 1).min(RETRY_MS.len() - 1)],
    }
}

/// When the next fetch is due, in epoch milliseconds. After failures: the last attempt plus the retry delay. Otherwise the
/// reading's age reaching the refresh interval minus 5 s (the QML's rule); no reading at all is due at 0, i.e. now.
pub fn next_attempt_ms(fetched_at: Option<u64>, consecutive_failures: u32, last_attempt_ms: u64) -> u64 {
    if consecutive_failures > 0 {
        return last_attempt_ms.saturating_add(retry_delay_ms(consecutive_failures));
    }
    fetched_at.map_or(0, |at| at.saturating_add(REFRESH_MS - REFETCH_EARLY_MS))
}

/// Writes `bytes` to `path` so a reader sees the old file or the new one, never a partial one: the parent directory is
/// created, the data goes to `<dir>/.<file name>.tmp`, is flushed and synced, and is renamed over `path`. The file is mode
/// 0644 whatever the umask, so the compositor's QML (another user) can read it. No temp file is left behind on error.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    fs::create_dir_all(dir)?;
    let mut tmp_name = OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    let result = write_synced(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(path)?;
    // `mode` is filtered by the umask and ignored for a leftover file; this makes the result certain.
    file.set_permissions(Permissions::from_mode(0o644))?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

/// Where the documents live and how they are read back.
#[derive(Clone, Debug)]
pub struct DocCache {
    dir: PathBuf,
}

impl DocCache {
    /// A cache in `dir`; the directory is created on the first store.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<dir>/weather.json`.
    pub fn weather_path(&self) -> PathBuf {
        self.dir.join("weather.json")
    }

    /// `<dir>/news.json`.
    pub fn news_path(&self) -> PathBuf {
        self.dir.join("news.json")
    }

    /// The saved weather document if it can be shown: readable, at most 1 MiB, valid, of this [`SCHEMA`], made for this
    /// zip and unit, and not expired. A stale one is returned (the runner refetches in the background).
    pub fn load_weather(&self, cfg: &WeatherConfig, now_ms: u64) -> Option<WeatherDoc> {
        let doc: WeatherDoc = read_json(&self.weather_path())?;
        let usable = doc.schema == SCHEMA
            && doc.zip == cfg.zip
            && doc.units == cfg.units
            && freshness(doc.fetched_at, now_ms) != Freshness::Expired;
        usable.then_some(doc)
    }

    /// The saved news document under the same rules, made for the same primary feed URLs in the same order.
    pub fn load_news(&self, cfg: &NewsConfig, now_ms: u64) -> Option<NewsDoc> {
        let doc: NewsDoc = read_json(&self.news_path())?;
        let usable = doc.schema == SCHEMA
            && doc.feeds.iter().map(String::as_str).eq(cfg.feeds.iter().map(|f| f.url.as_str()))
            && freshness(doc.fetched_at, now_ms) != Freshness::Expired;
        usable.then_some(doc)
    }

    /// Saves the weather document atomically as compact JSON.
    pub fn store_weather(&self, doc: &WeatherDoc) -> io::Result<()> {
        atomic_write(&self.weather_path(), &serde_json::to_vec(doc).map_err(io::Error::other)?)
    }

    /// Saves the news document atomically as compact JSON.
    pub fn store_news(&self, doc: &NewsDoc) -> io::Result<()> {
        atomic_write(&self.news_path(), &serde_json::to_vec(doc).map_err(io::Error::other)?)
    }
}

/// A JSON file of at most [`MAX_DOC_BYTES`] as `T`; `None` for every kind of failure.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let mut bytes = Vec::new();
    File::open(path).ok()?.take(MAX_DOC_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_DOC_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
