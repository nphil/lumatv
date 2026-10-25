mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::*;
use lumatv_feeds::cache::*;
use lumatv_feeds::news::{FeedSpec, NewsConfig};
use lumatv_feeds::weather::{Units, WeatherConfig};
use lumatv_feeds::{EXPIRE_MS, REFRESH_MS, RETRY_MS};

const T0: u64 = 1_792_900_000_000;

#[test]
fn freshness_boundaries() {
    // 14:59.999 is fresh, 15:00 is stale
    assert_eq!(freshness(T0, T0), Freshness::Fresh);
    assert_eq!(freshness(T0, T0 + 14 * 60_000 + 59_000), Freshness::Fresh);
    assert_eq!(freshness(T0, T0 + 899_999), Freshness::Fresh);
    assert_eq!(freshness(T0, T0 + 900_000), Freshness::Stale);
    assert_eq!(freshness(T0, T0 + 900_000), freshness(T0, T0 + REFRESH_MS));
    // 5:59:59 is stale, 6:00:00 is expired
    assert_eq!(
        freshness(T0, T0 + 5 * 3_600_000 + 59 * 60_000 + 59_000),
        Freshness::Stale
    );
    assert_eq!(freshness(T0, T0 + 21_599_999), Freshness::Stale);
    assert_eq!(freshness(T0, T0 + 21_600_000), Freshness::Expired);
    assert_eq!(freshness(T0, T0 + EXPIRE_MS), Freshness::Expired);
    assert_eq!(freshness(T0, u64::MAX), Freshness::Expired);
}

#[test]
fn freshness_with_clock_skew() {
    // up to 5 minutes ahead is age 0
    assert_eq!(freshness(T0 + 1, T0), Freshness::Fresh);
    assert_eq!(freshness(T0 + 300_000, T0), Freshness::Fresh);
    // further ahead: the clock was wrong when it was written
    assert_eq!(freshness(T0 + 300_001, T0), Freshness::Expired);
    assert_eq!(freshness(u64::MAX, 0), Freshness::Expired);
    assert_eq!(freshness(0, 0), Freshness::Fresh);
}

#[test]
fn retry_delays() {
    assert_eq!(RETRY_MS, [120_000, 600_000, 1_800_000]);
    assert_eq!(retry_delay_ms(0), 0);
    assert_eq!(retry_delay_ms(1), 120_000);
    assert_eq!(retry_delay_ms(2), 600_000);
    assert_eq!(retry_delay_ms(3), 1_800_000);
    assert_eq!(retry_delay_ms(4), 1_800_000);
    assert_eq!(retry_delay_ms(u32::MAX), 1_800_000);
}

#[test]
fn next_attempts() {
    // no failures: the reading's age reaches 15 min minus 5 s
    assert_eq!(next_attempt_ms(Some(T0), 0, T0 + 1), T0 + 895_000);
    // nothing saved: due now
    assert_eq!(next_attempt_ms(None, 0, T0), 0);
    // failures count from the last attempt, saved reading or not
    assert_eq!(next_attempt_ms(Some(T0), 1, T0 + 50), T0 + 50 + 120_000);
    assert_eq!(next_attempt_ms(Some(T0), 2, T0 + 50), T0 + 50 + 600_000);
    assert_eq!(next_attempt_ms(None, 3, T0 + 50), T0 + 50 + 1_800_000);
    assert_eq!(next_attempt_ms(None, 9, 7), 7 + 1_800_000);
}

#[test]
fn paths() {
    let cache = DocCache::new("/some/dir");
    assert_eq!(
        cache.weather_path().to_str(),
        Some("/some/dir/weather.json")
    );
    assert_eq!(cache.news_path().to_str(), Some("/some/dir/news.json"));
}

#[test]
fn round_trip_both_documents() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path().join("cache"));
    let w = weather_doc(T0);
    let n = news_doc(T0);
    cache.store_weather(&w).unwrap();
    cache.store_news(&n).unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0 + 1_000), Some(w.clone()));
    assert_eq!(
        cache.load_news(&NewsConfig::standard(), T0 + 1_000),
        Some(n.clone())
    );

    // the file itself is compact JSON that parses back to an equal document
    let text = fs::read_to_string(cache.weather_path()).unwrap();
    assert!(!text.contains('\n'));
    assert_eq!(
        serde_json::from_str::<lumatv_feeds::weather::WeatherDoc>(&text).unwrap(),
        w
    );
    let text = fs::read_to_string(cache.news_path()).unwrap();
    assert_eq!(
        serde_json::from_str::<lumatv_feeds::news::NewsDoc>(&text).unwrap(),
        n
    );
}

#[test]
fn stored_weather_file_has_the_keys_qml_reads() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    cache.store_weather(&weather_doc(T0)).unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(cache.weather_path()).unwrap()).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["fetchedAt"], T0);
    assert_eq!(v["units"], "fahrenheit");
    assert_eq!(v["display"]["temp"], "54\u{b0}");
    assert_eq!(v["display"]["days"][0]["day"], "Today");
}

#[test]
fn missing_or_damaged_files_load_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
    assert_eq!(cache.load_news(&NewsConfig::standard(), T0), None);

    cache.store_weather(&weather_doc(T0)).unwrap();
    cache.store_news(&news_doc(T0)).unwrap();
    let w = fs::read(cache.weather_path()).unwrap();
    let n = fs::read(cache.news_path()).unwrap();
    fs::write(cache.weather_path(), &w[..w.len() / 2]).unwrap();
    fs::write(cache.news_path(), &n[..n.len() / 2]).unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
    assert_eq!(cache.load_news(&NewsConfig::standard(), T0), None);

    fs::write(cache.weather_path(), b"").unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
    fs::write(cache.weather_path(), b"{\"schema\":1}").unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
    fs::write(cache.weather_path(), [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);

    // a directory where the file should be is unreadable, not a panic
    fs::remove_file(cache.news_path()).unwrap();
    fs::create_dir(cache.news_path()).unwrap();
    assert_eq!(cache.load_news(&NewsConfig::standard(), T0), None);
}

#[test]
fn another_schema_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    let mut w = weather_doc(T0);
    w.schema = 2;
    cache.store_weather(&w).unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
    let mut n = news_doc(T0);
    n.schema = 0;
    cache.store_news(&n).unwrap();
    assert_eq!(cache.load_news(&NewsConfig::standard(), T0), None);
}

#[test]
fn another_key_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    cache.store_weather(&weather_doc(T0)).unwrap();
    cache.store_news(&news_doc(T0)).unwrap();

    let other_zip = WeatherConfig {
        zip: "10001".to_owned(),
        units: Units::Fahrenheit,
    };
    let other_units = WeatherConfig {
        zip: "30311".to_owned(),
        units: Units::Celsius,
    };
    assert_eq!(cache.load_weather(&other_zip, T0), None);
    assert_eq!(cache.load_weather(&other_units, T0), None);
    assert!(cache.load_weather(&atlanta(), T0).is_some());

    let mut reordered = NewsConfig::standard();
    reordered.feeds.reverse();
    assert_eq!(cache.load_news(&reordered, T0), None);
    let mut shorter = NewsConfig::standard();
    shorter.feeds.truncate(1);
    assert_eq!(cache.load_news(&shorter, T0), None);
    let mut other = NewsConfig::standard();
    other.feeds[1] = FeedSpec {
        tag: "U.S.".to_owned(),
        src: "BBC News".to_owned(),
        url: "https://feeds.bbci.co.uk/news/rss.xml".to_owned(),
    };
    assert_eq!(cache.load_news(&other, T0), None);
    // the alternative feeds and per_feed are not part of the key
    let mut tuned = NewsConfig::standard();
    tuned.per_feed = 9;
    tuned.alt.clear();
    assert!(cache.load_news(&tuned, T0).is_some());
}

#[test]
fn stale_is_returned_and_expired_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    cache.store_weather(&weather_doc(T0)).unwrap();
    cache.store_news(&news_doc(T0)).unwrap();
    let cfg = NewsConfig::standard();

    assert!(cache.load_weather(&atlanta(), T0 + 899_999).is_some());
    assert!(
        cache.load_weather(&atlanta(), T0 + 900_000).is_some(),
        "stale is still shown"
    );
    assert!(cache.load_news(&cfg, T0 + 21_599_999).is_some());
    assert_eq!(cache.load_weather(&atlanta(), T0 + 21_600_000), None);
    assert_eq!(cache.load_news(&cfg, T0 + 21_600_000), None);
}

#[test]
fn future_dated_beyond_five_minutes_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    let cfg = NewsConfig::standard();
    cache.store_weather(&weather_doc(T0 + 300_000)).unwrap();
    cache.store_news(&news_doc(T0 + 300_000)).unwrap();
    assert!(cache.load_weather(&atlanta(), T0).is_some());
    assert!(cache.load_news(&cfg, T0).is_some());
    cache.store_weather(&weather_doc(T0 + 300_001)).unwrap();
    cache.store_news(&news_doc(T0 + 300_001)).unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
    assert_eq!(cache.load_news(&cfg, T0), None);
}

#[test]
fn oversized_files_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DocCache::new(dir.path());
    cache.store_weather(&weather_doc(T0)).unwrap();
    let mut bytes = fs::read(cache.weather_path()).unwrap();

    // exactly 1 MiB of valid JSON (trailing whitespace) still loads
    bytes.resize(1024 * 1024, b' ');
    fs::write(cache.weather_path(), &bytes).unwrap();
    assert!(cache.load_weather(&atlanta(), T0).is_some());
    // one byte more does not
    bytes.push(b' ');
    fs::write(cache.weather_path(), &bytes).unwrap();
    assert_eq!(cache.load_weather(&atlanta(), T0), None);
}

#[test]
fn atomic_write_creates_directories_and_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("file.json");
    atomic_write(&path, b"first").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"first");
    atomic_write(&path, b"second, longer").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"second, longer");
    atomic_write(&path, b"x").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"x");

    let names: Vec<String> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["file.json"]);
}

#[test]
fn atomic_write_is_world_readable_whatever_the_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    atomic_write(&path, b"{}").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );

    // a stale temp file with restrictive permissions from a crashed run does not leak its mode
    let tmp = dir.path().join(".doc.json.tmp");
    fs::write(&tmp, b"junk").unwrap();
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).unwrap();
    atomic_write(&path, b"{\"n\":1}").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"{\"n\":1}");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert!(!tmp.exists());
}

#[test]
fn atomic_write_leaves_no_temp_file_on_error() {
    let dir = tempfile::tempdir().unwrap();
    // renaming a file over a directory fails
    let path = dir.path().join("target");
    fs::create_dir(&path).unwrap();
    assert!(atomic_write(&path, b"data").is_err());
    assert!(!dir.path().join(".target.tmp").exists());
    assert!(path.is_dir());

    // a parent that is a file cannot be created
    let file = dir.path().join("plain");
    fs::write(&file, b"x").unwrap();
    assert!(atomic_write(&file.join("child.json"), b"data").is_err());

    // no file name at all
    assert!(atomic_write(std::path::Path::new("/"), b"data").is_err());
}

#[test]
fn store_failure_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, b"x").unwrap();
    let cache = DocCache::new(blocker.join("cache"));
    assert!(cache.store_weather(&weather_doc(T0)).is_err());
    assert!(cache.store_news(&news_doc(T0)).is_err());
}
