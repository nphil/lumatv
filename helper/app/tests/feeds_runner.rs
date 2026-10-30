//! Home's weather and news end to end: saved first, fetched while the screen is on, kept through failures, and following the
//! settings. A scripted fetcher stands in for the internet.

mod support;

use fetch::FetchError;
use lumatv_feeds::cache::DocCache;
use lumatv_feeds::news::{self, NewsConfig, NewsDoc};
use lumatv_feeds::weather::{self, Units, WeatherConfig, WeatherDoc};
use lumatv_helper::timeutil::now_ms;
use serde_json::{Value, json};
use std::thread;
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);
const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;

fn weather_doc(fetched_at: u64) -> WeatherDoc {
    let geo = weather::parse_geo(ZIPPOPOTAM.as_bytes()).unwrap();
    let cfg = WeatherConfig {
        zip: "30311".into(),
        units: Units::Fahrenheit,
    };
    weather::parse_forecast(OPEN_METEO.as_bytes(), &geo, &cfg, fetched_at).unwrap()
}

fn news_doc(fetched_at: u64) -> NewsDoc {
    news::refresh(
        Script::internet().as_ref(),
        &NewsConfig::standard(),
        fetched_at,
    )
    .unwrap()
}

/// A rig with the receiver off and the screen following a fake luna-send in `luna_state`.
fn rig(
    luna_state: &str,
    script: &std::sync::Arc<Script>,
    before_start: impl FnOnce(&Rig),
) -> (Rig, FakeLuna) {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::Subscribes, luna_state);
    before_start(&rig);
    rig.start(
        test_config(json!({
            "screen": {"source": "luna", "pollMs": 40},
            "luna": {"send": luna.program.display().to_string()},
        })),
        script.clone(),
    );
    (rig, luna)
}

fn has_doc(v: &Value) -> bool {
    !v["doc"].is_null()
}

#[test]
fn saved_documents_are_served_at_once_and_before_any_fetch_even_with_the_screen_off() {
    let script = Script::new();
    let now = now_ms();
    let (rig, _luna) = rig("Active Standby", &script, |rig| {
        let cache = DocCache::new(rig.paths.cache_dir());
        cache
            .store_weather(&weather_doc(now - 20 * MINUTE))
            .unwrap();
        cache.store_news(&news_doc(now - 20 * MINUTE)).unwrap();
    });
    let weather = rig
        .wait_topic("weather", WAIT, has_doc)
        .expect("the saved weather");
    // 20 minutes old: past the 15-minute refresh, well inside the 6 hours: shown, marked stale.
    assert_eq!(weather["state"], "stale");
    assert_eq!(weather["doc"]["place"], "Atlanta");
    assert_eq!(weather["doc"]["display"]["temp"], "54\u{b0}");
    assert_eq!(weather["doc"]["display"]["nowKind"], "moon");
    assert!(weather["ageMs"].as_u64().unwrap() >= 20 * MINUTE);
    let news = rig
        .wait_topic("news", WAIT, has_doc)
        .expect("the saved news");
    assert_eq!(news["state"], "stale");
    assert_eq!(news["doc"]["items"].as_array().unwrap().len(), 8);
    assert_eq!(news["doc"]["items"][0]["src"], "BBC News");
    // The screen is off: not one request.
    thread::sleep(Duration::from_millis(300));
    assert!(script.requests().is_empty(), "{:?}", script.requests());
    // And the state document carries them for a client that reads everything once.
    let state = get(rig.addr, "/v1/state").json();
    assert!(state["topics"]["weather"]["doc"]["display"].is_object());
}

#[test]
fn a_reading_older_than_six_hours_is_not_shown() {
    let script = Script::new();
    let now = now_ms();
    let (rig, _luna) = rig("Active Standby", &script, |rig| {
        let cache = DocCache::new(rig.paths.cache_dir());
        cache.store_weather(&weather_doc(now - 7 * HOUR)).unwrap();
        cache.store_news(&news_doc(now - 7 * HOUR)).unwrap();
    });
    let weather = rig
        .wait_topic("weather", WAIT, |v| v.get("doc").is_some())
        .expect("a weather topic");
    assert_eq!(weather["state"], "none");
    assert!(weather["doc"].is_null());
    let news = rig
        .wait_topic("news", WAIT, |v| v.get("doc").is_some())
        .expect("a news topic");
    assert!(news["doc"].is_null());
}

#[test]
fn a_saved_document_carries_the_time_it_stops_being_shown() {
    let script = Script::new();
    let now = now_ms();
    let (rig, _luna) = rig("Active Standby", &script, |rig| {
        // Saved 5 h 59 min ago: still shown now; the stored document carries when it stops being shown.
        let cache = DocCache::new(rig.paths.cache_dir());
        cache
            .store_weather(&weather_doc(now - 6 * HOUR + MINUTE))
            .unwrap();
    });
    let weather = rig.wait_topic("weather", WAIT, has_doc).expect("shown");
    assert_eq!(weather["state"], "stale");
    let expires = weather["doc"]["expiresAt"].as_u64().unwrap();
    assert!(
        expires > now && expires <= now + 2 * MINUTE,
        "{expires} vs {now}"
    );
}

#[test]
fn with_the_screen_on_it_fetches_saves_and_announces() {
    let script = Script::internet();
    let (rig, _luna) = rig("Active", &script, |_| {});
    let weather = rig
        .wait_topic("weather", WAIT, |v| has_doc(v) && v["state"] == "fresh")
        .expect("a fetched weather");
    assert_eq!(weather["doc"]["zip"], "30311");
    assert_eq!(
        weather["doc"]["display"]["stats"],
        "Feels like 50\u{b0} \u{b7} 72% humidity \u{b7} 8 mph"
    );
    assert_eq!(weather["status"]["failures"], 0);
    let news = rig
        .wait_topic("news", WAIT, |v| has_doc(v) && v["state"] == "fresh")
        .expect("fetched news");
    assert_eq!(news["doc"]["feeds"], json!([BBC_WORLD_URL, BBC_US_URL]));
    assert_eq!(news["doc"]["items"].as_array().unwrap().len(), 8);

    // Geocoding, then the forecast, then the two feeds, then the full weather view's details and alerts for the same place (no second lookup).
    let requests = script.requests();
    assert_eq!(requests[..4], ["https://api.zippopotam.us/us/30311", FORECAST_ATLANTA, BBC_WORLD_URL, BBC_US_URL]);
    let mut more: Vec<&str> = requests[4..].iter().map(String::as_str).collect();
    more.sort_unstable();
    assert_eq!(more.len(), 2, "{more:?}");
    assert!(more[0].starts_with("https://api.open-meteo.com/v1/forecast?latitude=33.723&longitude=-84.4702&current=uv_index,"), "{more:?}");
    assert_eq!(more[1], "https://api.weather.gov/alerts/active?point=33.7230,-84.4702");

    // The files are the documents, byte for byte the same JSON.
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(rig.paths.cache_dir().join("weather.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk, weather["doc"]);
    let news_disk: Value =
        serde_json::from_slice(&std::fs::read(rig.paths.cache_dir().join("news.json")).unwrap())
            .unwrap();
    assert_eq!(news_disk, news["doc"]);

    // Nothing is fetched again until the next refresh is due.
    thread::sleep(Duration::from_millis(300));
    assert_eq!(script.requests().len(), 6, "the four, and the details and the alerts (which this script has no answer for)");

    // A restart on the same data dir serves the saved reading before it fetches.
    let mut rig = rig;
    rig.stop();
    let offline = Script::new();
    let (again, _luna2) = {
        let luna = FakeLuna::new(rig.dir.path(), LunaMode::Subscribes, "Active Standby");
        let mut second = Rig::new();
        // Share the data dir by pointing the new rig's paths at the old one.
        second.paths = rig.paths.clone();
        receiver_off(&second.paths);
        second.start(
            test_config(json!({"screen": {"source": "luna", "pollMs": 40}, "luna": {"send": luna.program.display().to_string()}})),
            offline.clone(),
        );
        (second, luna)
    };
    let cached = again
        .wait_topic("weather", WAIT, has_doc)
        .expect("saved weather after a restart");
    assert_eq!(cached["doc"]["place"], "Atlanta");
    assert_eq!(cached["state"], "fresh");
    assert!(offline.requests().is_empty());
}

#[test]
fn a_failed_fetch_keeps_the_saved_reading_and_counts_the_failure() {
    let script = Script::internet();
    script.everything_fails(FetchError::Offline);
    let now = now_ms();
    let (rig, _luna) = rig("Active", &script, |rig| {
        let cache = DocCache::new(rig.paths.cache_dir());
        cache
            .store_weather(&weather_doc(now - 20 * MINUTE))
            .unwrap();
        cache.store_news(&news_doc(now - 20 * MINUTE)).unwrap();
    });
    let weather = rig
        .wait_topic("weather", WAIT, |v| v["status"]["failures"] == 1)
        .expect("one failure counted");
    assert_eq!(weather["status"]["error"]["code"], "offline");
    assert!(weather["status"]["lastAttemptAt"].as_u64().unwrap() >= now);
    // The saved reading is still shown, as stale.
    assert_eq!(weather["state"], "stale");
    assert_eq!(weather["doc"]["place"], "Atlanta");
    let news = rig
        .wait_topic("news", WAIT, |v| v["status"]["failures"] == 1)
        .expect("a news failure");
    assert_eq!(news["doc"]["items"].as_array().unwrap().len(), 8);
    // The saved place was reused, so the weather asked once (the forecast); the feeds asked for both feeds and their fallbacks.
    let requests = script.requests();
    assert_eq!(
        requests.iter().filter(|u| u.contains("open-meteo")).count(),
        1,
        "{requests:?}"
    );
    assert_eq!(
        requests.iter().filter(|u| u.contains("zippopotam")).count(),
        0,
        "{requests:?}"
    );
    // The retry is minutes away: nothing more within the test.
    thread::sleep(Duration::from_millis(300));
    assert_eq!(script.requests().len(), requests.len());
}

#[test]
fn a_refresh_request_fetches_even_with_the_screen_off() {
    let script = Script::internet();
    let (rig, _luna) = rig("Active Standby", &script, |_| {});
    thread::sleep(Duration::from_millis(200));
    assert!(script.requests().is_empty());
    assert_eq!(post(rig.addr, "/v1/refresh", "").status, 200);
    let weather = rig
        .wait_topic("weather", WAIT, has_doc)
        .expect("weather after a refresh");
    assert_eq!(weather["state"], "fresh");
    rig.wait_topic("news", WAIT, has_doc)
        .expect("news after a refresh");
}

#[test]
fn turning_the_screen_on_fetches_a_stale_reading_at_once() {
    let script = Script::internet();
    let now = now_ms();
    let (rig, luna) = rig("Active Standby", &script, |rig| {
        let cache = DocCache::new(rig.paths.cache_dir());
        cache
            .store_weather(&weather_doc(now - 20 * MINUTE))
            .unwrap();
    });
    rig.wait_topic("weather", WAIT, has_doc)
        .expect("saved weather");
    thread::sleep(Duration::from_millis(200));
    assert!(script.requests().is_empty());
    luna.set_state("Active");
    let fresh = rig
        .wait_topic("weather", WAIT, |v| v["state"] == "fresh")
        .expect("fetched when the screen came on");
    assert!(fresh["doc"]["fetchedAt"].as_u64().unwrap() > now);
}

#[test]
fn a_new_zip_drops_the_old_reading_and_fetches_for_the_new_place() {
    let script = Script::internet();
    script.answer(
        "https://api.zippopotam.us/us/10001",
        Ok(ZIPPOPOTAM.replace("Atlanta", "New York").into_bytes()),
    );
    let (rig, _luna) = rig("Active", &script, |_| {});
    rig.wait_topic("weather", WAIT, |v| v["doc"]["zip"] == "30311")
        .expect("the first reading");
    let change = r#"{"changes":[{"key":"zip","value":{"type":"text","value":"10001"}}]}"#;
    assert_eq!(
        post(rig.addr, "/v1/settings/base.weather", change).status,
        200
    );
    let moved = rig
        .wait_topic("weather", WAIT, |v| v["doc"]["zip"] == "10001")
        .expect("a reading for the new zip");
    assert_eq!(moved["doc"]["place"], "New York");
    assert!(script.requests().iter().any(|u| u.ends_with("/us/10001")));
    // Both readings are never shown as one: the file on disk is the new place's.
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(rig.paths.cache_dir().join("weather.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk["zip"], "10001");
}

#[test]
fn the_news_source_setting_switches_the_feeds_and_the_interval_moves_the_stale_time() {
    let script = Script::internet();
    let (rig, _luna) = rig("Active", &script, |_| {});
    let bbc = rig
        .wait_topic("news", WAIT, |v| {
            v["doc"]["feeds"] == json!([BBC_WORLD_URL, BBC_US_URL])
        })
        .expect("bbc");
    assert_eq!(
        bbc["doc"]["staleAt"].as_u64().unwrap() - bbc["doc"]["fetchedAt"].as_u64().unwrap(),
        15 * MINUTE
    );

    let nyt = r#"{"changes":[{"key":"edition","value":{"type":"choice","value":"nyt"}}]}"#;
    assert_eq!(post(rig.addr, "/v1/settings/base.news", nyt).status, 200);
    let moved = rig
        .wait_topic("news", WAIT, |v| {
            v["doc"]["feeds"] == json!([NYT_WORLD_URL, NYT_US_URL])
        })
        .expect("the NYT edition");
    assert_eq!(moved["doc"]["items"][0]["src"], "The New York Times");

    let hour = r#"{"changes":[{"key":"refresh_minutes","value":{"type":"integer","value":60}}]}"#;
    assert_eq!(post(rig.addr, "/v1/settings/base.news", hour).status, 200);
    let slower = rig
        .wait_topic("news", WAIT, |v| {
            v["doc"]["staleAt"]
                .as_u64()
                .zip(v["doc"]["fetchedAt"].as_u64())
                .is_some_and(|(s, f)| s - f == 60 * MINUTE)
        })
        .expect("a 60-minute interval");
    assert_eq!(slower["state"], "fresh");
}

#[test]
fn a_reading_from_a_clock_that_was_ahead_is_shown_as_stale_and_fetched_again_at_once() {
    let script = Script::internet();
    let now = now_ms();
    // Saved with a clock that was an hour fast: in the future now. It must not vanish, and it must not be trusted as fresh.
    let (rig, luna) = rig("Active Standby", &script, |rig| {
        let cache = DocCache::new(rig.paths.cache_dir());
        cache.store_weather(&weather_doc(now + HOUR)).unwrap();
    });
    let shown = rig.wait_topic("weather", WAIT, has_doc).expect("shown");
    assert_eq!(shown["state"], "stale");
    assert_eq!(shown["doc"]["place"], "Atlanta");
    thread::sleep(Duration::from_millis(150));
    assert!(script.requests().is_empty(), "fetched with the screen off");
    luna.set_state("Active");
    let fresh = rig
        .wait_topic("weather", WAIT, |v| v["state"] == "fresh")
        .expect("fetched when the screen came on");
    assert!(fresh["doc"]["fetchedAt"].as_u64().unwrap() < now + HOUR);
}

#[test]
fn a_saved_news_document_follows_the_configured_interval_from_the_first_look() {
    let script = Script::new();
    let now = now_ms();
    let (rig, _luna) = rig("Active Standby", &script, |rig| {
        let cache = DocCache::new(rig.paths.cache_dir());
        cache.store_news(&news_doc(now - MINUTE)).unwrap();
        seed_settings(
            &rig.paths,
            "base.news",
            json!({"refresh_minutes": {"type": "integer", "value": 60}}),
        );
    });
    let news = rig
        .wait_topic("news", WAIT, has_doc)
        .expect("the saved news");
    let doc = &news["doc"];
    assert_eq!(
        doc["staleAt"].as_u64().unwrap() - doc["fetchedAt"].as_u64().unwrap(),
        60 * MINUTE
    );
    assert_eq!(news["state"], "fresh");
}
