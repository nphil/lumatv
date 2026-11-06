//! The helper's own plugins end to end: Home's weather, news and clock, and the ambient light tile, speaking the plugin API through
//! the broker's sessions in a real helper with scripted feeds. What is held to: they are listed as connected first-party plugins,
//! what they publish is what the feeds and the settings say, a reading the feeds no longer show is no card, and an action the UI
//! invokes through the same route as for any plugin reaches them and is answered.

mod support;

use lumatv_feeds::cache::DocCache;
use lumatv_feeds::news::{self, NewsConfig, NewsDoc};
use lumatv_feeds::weather::{self, Units, WeatherConfig, WeatherDoc};
use lumatv_helper::hub::Event;
use lumatv_helper::timeutil::now_ms;
use serde_json::{Value, json};
use std::net::UdpSocket;
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

fn items(v: &Value) -> usize {
    v["items"].as_array().map_or(0, Vec::len)
}

/// The `plugins` topic's first-party entries, as `id: state`.
fn first_party(rig: &Rig) -> Vec<(String, String)> {
    rig.topic("plugins")
        .and_then(|t| {
            t["firstParty"].as_array().map(|a| {
                a.iter()
                    .map(|p| {
                        (
                            p["id"].as_str().unwrap_or("").to_owned(),
                            p["state"].as_str().unwrap_or("").to_owned(),
                        )
                    })
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Events of the plugin's own topic (`action_result` is published there) since the start.
fn action_results(rig: &Rig, plugin: &str) -> Vec<Event> {
    let topics = [format!("plugin.{plugin}")];
    rig.service()
        .hub()
        .events_after(Some(0), None, Some(&topics))
        .events
        .into_iter()
        .filter(|e| e.kind == "action_result")
        .collect()
}

#[test]
fn weather_news_and_the_clock_are_plugins_of_the_helper_and_follow_the_feeds() {
    let script = Script::internet();
    let mut rig = Rig::new();
    rig.start(test_config(json!({})), script.clone());

    // Listed apart from the plugins the owner switched on, all connected through the broker's sessions; the Guide data plugin, Home
    // Assistant, the Live TV Tuner, the Bluetooth proxy, the cameras, the TV sensors and the Denon receiver, which ask first, listed off.
    assert!(
        wait_until(WAIT, || {
            let mut have = first_party(&rig);
            have.sort();
            have == [
                ("ambient".to_owned(), "connected".to_owned()),
                ("ble-proxy".to_owned(), "off".to_owned()),
                ("clock".to_owned(), "connected".to_owned()),
                ("denon-receiver".to_owned(), "off".to_owned()),
                ("guide-data".to_owned(), "off".to_owned()),
                ("home-assistant".to_owned(), "off".to_owned()),
                ("live-tv-tuner".to_owned(), "off".to_owned()),
                ("news".to_owned(), "connected".to_owned()),
                ("scrypted-cameras".to_owned(), "off".to_owned()),
                ("tv-sensors".to_owned(), "off".to_owned()),
                ("weather".to_owned(), "connected".to_owned()),
            ]
        }),
        "{:?}",
        rig.topic("plugins")
    );
    assert_eq!(rig.topic("plugins").unwrap()["plugins"], json!([]));
    assert_eq!(
        get(rig.addr, "/v1/plugins").json()["firstParty"][0]["firstParty"],
        true
    );

    // The weather: what the feeds fetched, as the card the contract allows.
    let weather = rig
        .wait_topic("weather", WAIT, |v| !v["doc"].is_null())
        .expect("the weather feed");
    let display = &weather["doc"]["display"];
    let cards = rig
        .wait_topic("plugin.weather.cards", WAIT, |v| items(v) == 1)
        .expect("the weather card");
    let content = &cards["items"][0]["card"]["content"];
    assert_eq!(content["type"], "facts");
    assert_eq!(
        content["title"],
        format!("Weather \u{b7} {}", display["place"].as_str().unwrap())
    );
    assert_eq!(
        content["facts"][0],
        json!({"label": "Now", "value": display["temp"]})
    );
    assert_eq!(cards["items"][0]["card"]["actions"][0]["action"], "refresh");
    // The reading lives six hours, and so does the card.
    let expires = cards["items"][0]["expiresAt"].as_i64().unwrap();
    assert_eq!(
        expires,
        (weather["doc"]["expiresAt"].as_u64().unwrap() / 1000) as i64
    );

    // The news: the top story.
    let news = rig
        .wait_topic("news", WAIT, |v| !v["doc"].is_null())
        .expect("the news feed");
    let top = rig
        .wait_topic("plugin.news.cards", WAIT, |v| items(v) == 1)
        .expect("the news card");
    let story = &news["doc"]["items"][0];
    let shown = top["items"][0]["card"]["content"]["title"]
        .as_str()
        .unwrap();
    assert!(
        story["title"]
            .as_str()
            .unwrap()
            .starts_with(shown.trim_end_matches('\u{2026}')),
        "{shown} / {}",
        story["title"]
    );
    assert_eq!(top["items"][0]["card"]["content"]["detail"], story["src"]);

    // The clock: the time and the date, while the screen is on.
    let clock = rig
        .wait_topic("plugin.clock.cards", WAIT, |v| items(v) == 1)
        .expect("the clock card");
    assert_eq!(
        clock["items"][0]["card"]["content"]["facts"][0]["label"],
        "Time"
    );
    assert_eq!(
        clock["items"][0]["card"]["content"]["facts"][1]["label"],
        "Date"
    );

    // No strip is named, so there is no ambient tile to offer.
    assert_eq!(items(&rig.topic("plugin.ambient.tiles").unwrap()), 0);

    // A reading the feeds drop (another ZIP, nothing scripted for it) is no card either.
    let changed = post(
        rig.addr,
        "/v1/settings/base.weather",
        r#"{"changes":[{"key":"zip","value":{"type":"text","value":"90210"}}]}"#,
    );
    assert_eq!(
        changed.status,
        200,
        "{}",
        String::from_utf8_lossy(&changed.body)
    );
    assert!(
        rig.wait_topic("plugin.weather.cards", WAIT, |v| items(v) == 0)
            .is_some(),
        "the card outlived its reading"
    );
}

#[test]
fn refresh_is_an_action_of_the_plugin_like_any_other_and_asks_the_feeds_to_fetch() {
    let script = Script::internet();
    let mut rig = Rig::new();
    rig.start(test_config(json!({})), script.clone());
    rig.wait_topic("plugin.weather.cards", WAIT, |v| items(v) == 1)
        .expect("the weather card");
    let before = script.requests().len();

    let invoked = post(rig.addr, "/v1/plugins/weather/actions/refresh", "{}");
    assert_eq!(
        invoked.status,
        202,
        "{}",
        String::from_utf8_lossy(&invoked.body)
    );
    let op = invoked.json()["op"].as_u64().unwrap();
    assert!(
        wait_until(WAIT, || action_results(&rig, "weather")
            .iter()
            .any(|e| e.data["op"] == op && e.data["outcome"]["type"] == "ok")),
        "no answer: {:?}",
        action_results(&rig, "weather")
    );
    // The feeds fetch when told to, even if they were fresh a moment ago.
    assert!(
        wait_until(WAIT, || script.requests().len() > before),
        "nothing was fetched after the refresh"
    );
    // An action the plugin did not declare is refused by the broker before the plugin hears of it.
    let nothing = post(rig.addr, "/v1/plugins/weather/actions/explode", "{}");
    assert_eq!(
        nothing.status,
        404,
        "{}",
        String::from_utf8_lossy(&nothing.body)
    );
}

#[test]
fn the_ambient_tile_follows_the_settings_and_a_tap_switches_the_light() {
    let strip = UdpSocket::bind("127.0.0.1:0").unwrap();
    strip.set_read_timeout(Some(WAIT)).unwrap();
    let addr = strip.local_addr().unwrap();
    let mut rig = Rig::new();
    seed_settings(
        &rig.paths,
        "base.ambient",
        json!({"mode": {"type": "choice", "value": "colour"}, "brightness": {"type": "integer", "value": 100}}),
    );
    rig.start(
        test_config(
            json!({"ambient": {"wled": {"host": addr.ip().to_string(), "port": addr.port()}}}),
        ),
        Script::new(),
    );

    // A strip is named, so the tile is there, off.
    let tiles = rig
        .wait_topic("plugin.ambient.tiles", WAIT, |v| items(v) == 1)
        .expect("the ambient tile");
    let tile = &tiles["items"][0];
    assert_eq!(tile["label"], "Ambient light");
    assert_eq!(tile["state"], json!({"type": "off"}));
    assert_eq!(tile["hint"], "Off");
    assert_eq!(tile["action"], "toggle");
    assert_eq!(rig.topic("ambient").unwrap()["available"], true);

    // A tap: the UI invokes the tile's action through the plugins' route.
    let invoked = post(rig.addr, "/v1/plugins/ambient/actions/toggle", "{}");
    assert_eq!(
        invoked.status,
        202,
        "{}",
        String::from_utf8_lossy(&invoked.body)
    );
    let op = invoked.json()["op"].as_u64().unwrap();
    let on = rig
        .wait_topic("plugin.ambient.tiles", WAIT, |v| {
            v["items"][0]["state"]["type"] == "on"
        })
        .expect("the tile never turned on");
    assert_eq!(on["items"][0]["hint"], "Still colour \u{b7} 100%");
    assert!(wait_until(WAIT, || action_results(&rig, "ambient")
        .iter()
        .any(
            |e| e.data["op"] == op && e.data["outcome"]["type"] == "ok"
        )));
    // It is the Settings page's own switch that moved.
    let settings = get(rig.addr, "/v1/settings/base.ambient").json();
    assert_eq!(settings["effective"]["enabled"]["value"], true);
    // ...and the strip is lit: the still colour, at full brightness, on every section.
    let mut packet = [0u8; 2048];
    let n = strip.recv(&mut packet).unwrap();
    let ddp = lumatv_ambient::ddp::parse(&packet[..n]).unwrap();
    assert_eq!(ddp.data.len(), 58 * 3);
    assert_eq!(
        &ddp.data[..3],
        &lumatv_ambient::plan::Still::Warm.rgb().to_u8()
    );

    // Tapped again: off, and the strip is told black.
    post(rig.addr, "/v1/plugins/ambient/actions/toggle", "{}");
    rig.wait_topic("plugin.ambient.tiles", WAIT, |v| {
        v["items"][0]["state"]["type"] == "off"
    })
    .expect("the tile never turned off");
    let mut black = false;
    for _ in 0..20 {
        let Ok(n) = strip.recv(&mut packet) else {
            break;
        };
        if lumatv_ambient::ddp::parse(&packet[..n])
            .unwrap()
            .data
            .iter()
            .all(|b| *b == 0)
        {
            black = true;
            break;
        }
    }
    assert!(black, "the strip was never told to go dark");
}

#[test]
fn the_test_pattern_runs_on_the_strip_when_the_settings_page_asks_and_stops_when_told() {
    let strip = UdpSocket::bind("127.0.0.1:0").unwrap();
    strip.set_read_timeout(Some(WAIT)).unwrap();
    let addr = strip.local_addr().unwrap();
    let mut rig = Rig::new();
    rig.start(
        test_config(
            json!({"ambient": {"wled": {"host": addr.ip().to_string(), "port": addr.port()}}}),
        ),
        Script::new(),
    );
    let started = post(rig.addr, "/v1/ambient/test", r#"{"run":true}"#);
    assert_eq!(
        started.status,
        200,
        "{}",
        String::from_utf8_lossy(&started.body)
    );
    assert_eq!(started.json()["ambient"]["state"], "testing");
    // One section at a time lights, and the lit one moves on.
    let mut packet = [0u8; 2048];
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..40 {
        let n = strip.recv(&mut packet).unwrap();
        let data = lumatv_ambient::ddp::parse(&packet[..n]).unwrap().data;
        let lit: Vec<usize> = data
            .chunks(3)
            .enumerate()
            .filter(|(_, p)| p.iter().any(|b| *b != 0))
            .map(|(i, _)| i)
            .collect();
        assert!(lit.len() <= 1, "{lit:?}");
        seen.extend(lit);
        if seen.len() >= 3 {
            break;
        }
    }
    assert!(seen.len() >= 3, "the pattern did not move: {seen:?}");
    assert_eq!(rig.topic("ambient").unwrap()["state"], "testing");
    let stopped = post(rig.addr, "/v1/ambient/test", r#"{"run":false}"#);
    assert_eq!(stopped.status, 200);
    assert!(
        rig.wait_topic("ambient", WAIT, |v| v["state"] == "off")
            .is_some(),
        "{:?}",
        rig.topic("ambient")
    );
}

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

#[test]
fn the_cards_are_there_from_what_was_saved_before_any_fetch_and_the_clock_waits_for_the_screen() {
    const MINUTE: u64 = 60_000;
    let now = now_ms();
    let mut rig = Rig::new();
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::Subscribes, "Active Standby");
    let cache = DocCache::new(rig.paths.cache_dir());
    cache
        .store_weather(&weather_doc(now - 20 * MINUTE))
        .unwrap();
    cache.store_news(&news_doc(now - 20 * MINUTE)).unwrap();
    // The screen is off and nothing can be fetched: what was saved is what the cards say.
    let script = Script::new();
    rig.start(
        test_config(json!({
            "screen": {"source": "luna", "pollMs": 40},
            "luna": {"send": luna.program.display().to_string()},
        })),
        script.clone(),
    );
    let weather = rig
        .wait_topic("plugin.weather.cards", WAIT, |v| items(v) == 1)
        .expect("the saved weather as a card");
    assert_eq!(
        weather["items"][0]["card"]["content"]["facts"][0]["value"],
        "54\u{b0}"
    );
    assert_eq!(
        weather["items"][0]["card"]["content"]["title"],
        "Weather \u{b7} Atlanta"
    );
    assert!(
        rig.wait_topic("plugin.news.cards", WAIT, |v| items(v) == 1)
            .is_some(),
        "the saved news as a card"
    );
    assert!(
        script.requests().is_empty(),
        "nothing was fetched: the screen is off"
    );
    // No clock while the panel is dark, and none appears by itself.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(items(&rig.topic("plugin.clock.cards").unwrap()), 0);
    // The panel comes on: the clock card follows.
    luna.set_state("Active");
    assert!(
        rig.wait_topic("plugin.clock.cards", WAIT, |v| items(v) == 1)
            .is_some(),
        "the clock never appeared with the screen on"
    );
}
