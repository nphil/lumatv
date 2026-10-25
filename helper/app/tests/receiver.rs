//! The receiver client end to end: a fake receiver on the loopback, a fake `luna-send` that says whether the screen is on, and
//! the real service in between.

mod support;

use serde_json::{Value, json};
use std::thread;
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

fn fresh(v: &Value) -> bool {
    v["fresh"] == true
}

/// A service whose screen follows a fake luna-send and whose receiver client points at `avr`.
fn rig_with(avr: &FakeAvr, luna_state: &str, tune: Value) -> (Rig, FakeLuna) {
    let mut rig = Rig::new();
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::Subscribes, luna_state);
    receiver_at(&rig.paths, avr.port);
    let config = test_config(json!({
        "screen": {"source": "luna", "pollMs": 40},
        "luna": {"send": luna.program.display().to_string()},
        "receiver": tune,
    }));
    rig.start(config, Script::new());
    (rig, luna)
}

#[test]
fn nothing_is_asked_of_the_receiver_while_the_screen_is_off() {
    let avr = FakeAvr::start();
    let (rig, luna) = rig_with(&avr, "Active Standby", json!({}));
    // The screen is off: the document says nothing is live, and the receiver is never touched.
    let idle = rig.wait_topic("receiver", WAIT, |v| v["polling"] == false).expect("an idle document");
    assert_eq!(idle["fresh"], false);
    assert_eq!(idle["reachable"], Value::Null);
    assert_eq!(idle["volumeDb"], Value::Null);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(avr.hits(), 0, "polled a receiver with the screen off");

    // The screen comes on: polling starts and the first reading is shown.
    luna.set_state("Active");
    let live = rig.wait_topic("receiver", WAIT, fresh).expect("a live reading");
    assert_eq!(live["polling"], true);
    assert_eq!(live["reachable"], true);
    assert_eq!(live["power"], "on");
    assert_eq!(live["input"], "TV");
    assert_eq!(live["volumeDb"], -44.5);
    assert_eq!(live["volumeAbs"], 35.5);
    assert_eq!(live["volumeDisplay"], "absolute");
    assert_eq!(live["volumeText"], "35.5");
    assert_eq!(live["mute"], false);
    assert_eq!(live["host"], "127.0.0.1");
    assert!(live["updatedAt"].as_u64().unwrap() > 1_700_000_000_000);

    // The screen goes off again: polling stops within a moment and stays stopped.
    luna.set_state("Active Standby");
    rig.wait_topic("receiver", WAIT, |v| v["polling"] == false).expect("polling stopped");
    thread::sleep(Duration::from_millis(100));
    let hits = avr.hits();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(avr.hits(), hits, "kept polling with the screen off");
    let after = rig.topic("receiver").unwrap();
    assert_eq!(after["fresh"], false);
    assert_eq!(after["volumeDb"], Value::Null, "a reading is hidden once polling stops");
}

#[test]
fn a_change_at_the_receiver_is_an_event() {
    let avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({}));
    rig.wait_topic("receiver", WAIT, fresh).expect("first reading");
    let seq = get(rig.addr, "/v1/state").json()["seq"].as_u64().unwrap();

    avr.set(|s| s.volume = "-40.0".into());
    let moved = rig.wait_topic("receiver", WAIT, |v| v["volumeDb"] == -40.0).expect("the new volume");
    assert_eq!(moved["volumeAbs"], 40.0);
    assert_eq!(moved["volumeText"], "40");

    avr.set(|s| s.mute = "on".into());
    rig.wait_topic("receiver", WAIT, |v| v["mute"] == true).expect("mute");
    avr.set(|s| {
        s.input = "GAME".into();
        s.power = "OFF".into();
    });
    let changed = rig
        .wait_topic("receiver", WAIT, |v| v["input"] == "GAME" && v["power"] == "standby")
        .expect("input and power");
    assert_eq!(changed["mute"], true);

    // Each change was one event on the topic; unchanged polls raised none.
    let events = get(rig.addr, &format!("/v1/events?after={seq}&topics=receiver&timeout=0.2")).json();
    let volumes: Vec<_> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["data"]["volumeDb"].clone(), e["data"]["mute"].clone(), e["data"]["input"].clone()))
        .collect();
    assert_eq!(
        volumes,
        [
            (json!(-40.0), json!(false), json!("TV")),
            (json!(-40.0), json!(true), json!("TV")),
            (json!(-40.0), json!(true), json!("GAME")),
        ],
        "{events}"
    );
}

#[test]
fn a_receiver_that_goes_quiet_is_hidden_after_the_window_not_shown_stale() {
    let mut avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({"hideAfterMs": 400}));
    rig.wait_topic("receiver", WAIT, fresh).expect("first reading");
    avr.shut_down();
    // Within the window the last reading is still shown; after it, nothing is.
    let hidden = rig
        .wait_topic("receiver", WAIT, |v| v["fresh"] == false && v["polling"] == true)
        .expect("hidden after the window");
    assert_eq!(hidden["reachable"], false);
    assert_eq!(hidden["volumeDb"], Value::Null);
    assert_eq!(hidden["mute"], Value::Null);
    assert_eq!(hidden["input"], Value::Null);
    assert_eq!(hidden["error"]["code"], "unreachable");
    assert!(hidden["error"]["message"].as_str().unwrap().contains("not reachable"));
    assert!(hidden["updatedAt"].is_u64(), "the time of the last good reading is kept");
}

#[test]
fn a_forbidden_page_is_an_error_not_a_reading() {
    let avr = FakeAvr::start();
    avr.set(|s| s.forbidden = true);
    let (rig, _luna) = rig_with(&avr, "Active", json!({}));
    let doc = rig
        .wait_topic("receiver", WAIT, |v| v["polling"] == true && v["error"]["code"] == "http")
        .expect("an http error");
    assert_eq!(doc["fresh"], false);
    assert_eq!(doc["reachable"], false);
    assert!(doc["error"]["message"].as_str().unwrap().contains("403"));
}

#[test]
fn a_nudge_reads_now_instead_of_at_the_next_poll() {
    let avr = FakeAvr::start();
    // A poll period far longer than the test: only a nudge can make the change show.
    let (rig, _luna) = rig_with(&avr, "Active", json!({"pollMs": 20_000, "fastPollMs": 15, "burstMs": 400}));
    rig.wait_topic("receiver", WAIT, fresh).expect("first reading");
    let hits = avr.hits();
    avr.set(|s| s.volume = "-30.5".into());
    thread::sleep(Duration::from_millis(200));
    assert_eq!(avr.hits(), hits, "polled between periods");
    assert_eq!(rig.topic("receiver").unwrap()["volumeDb"], -44.5);

    assert_eq!(post(rig.addr, "/v1/receiver/nudge", "").status, 200);
    let moved = rig.wait_topic("receiver", Duration::from_secs(3), |v| v["volumeDb"] == -30.5);
    assert!(moved.is_some(), "the nudge did not make the receiver be read");
    // The burst reads fast for a short while, then goes back to the long period.
    wait_until(Duration::from_secs(3), || avr.hits() >= hits + 3);
    thread::sleep(Duration::from_millis(700));
    let settled = avr.hits();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(avr.hits(), settled, "the burst did not end");
}

#[test]
fn a_nudge_with_the_screen_off_does_nothing() {
    let avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active Standby", json!({}));
    rig.wait_topic("receiver", WAIT, |v| v["polling"] == false).expect("idle");
    assert_eq!(post(rig.addr, "/v1/receiver/nudge", "").status, 200);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(avr.hits(), 0);
}

#[test]
fn switching_the_receiver_off_in_the_settings_stops_the_polling() {
    let avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({}));
    rig.wait_topic("receiver", WAIT, fresh).expect("first reading");
    let off = r#"{"changes":[{"key":"enabled","value":{"type":"bool","value":false}}]}"#;
    assert_eq!(post(rig.addr, "/v1/settings/base.receiver", off).status, 200);
    let doc = rig.wait_topic("receiver", WAIT, |v| v["enabled"] == false).expect("disabled");
    assert_eq!(doc["polling"], false);
    assert_eq!(doc["fresh"], false);
    thread::sleep(Duration::from_millis(100));
    let hits = avr.hits();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(avr.hits(), hits);
}

#[test]
fn a_new_address_in_the_settings_repoints_the_client() {
    let first = FakeAvr::start();
    let second = FakeAvr::start();
    second.set(|s| s.volume = "-12.0".into());
    let (rig, _luna) = rig_with(&first, "Active", json!({}));
    let seen = rig.wait_topic("receiver", WAIT, fresh).expect("first receiver");
    assert_eq!(seen["volumeDb"], -44.5);
    let body = json!({"changes": [{"key": "port", "value": {"type": "integer", "value": second.port}}]}).to_string();
    assert_eq!(post(rig.addr, "/v1/settings/base.receiver", &body).status, 200);
    let moved = rig.wait_topic("receiver", WAIT, |v| v["volumeDb"] == -12.0).expect("the second receiver");
    assert_eq!(moved["host"], "127.0.0.1");
    let hits = first.hits();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(first.hits(), hits, "the old receiver is still being read");
}

#[test]
fn a_receiver_that_keeps_failing_is_asked_less_and_less_often_and_found_again_when_it_recovers() {
    let avr = FakeAvr::start();
    avr.set(|s| s.forbidden = true);
    let (rig, _luna) = rig_with(&avr, "Active", json!({"pollMs": 100, "maxBackoffMs": 1000}));
    assert!(wait_until(WAIT, || avr.hits() >= 1), "never asked");
    let started = std::time::Instant::now();
    let first = avr.hits();
    thread::sleep(Duration::from_millis(1500));
    // Without a backoff that is about 15 more asks; with 200, 400, 800, ... ms it is about 3.
    let asked = avr.hits() - first;
    assert!((1..=6).contains(&asked), "asked {asked} times in {:?}", started.elapsed());
    // The receiver comes back: the next retry (within the cap) shows it.
    avr.set(|s| s.forbidden = false);
    let back = rig.wait_topic("receiver", Duration::from_secs(4), fresh).expect("recovered");
    assert_eq!(back["reachable"], true);
    assert_eq!(back["error"], Value::Null);
    // Back to the normal period: about ten asks a second, not one.
    let before = avr.hits();
    thread::sleep(Duration::from_millis(600));
    assert!(avr.hits() - before >= 3, "still backed off: {}", avr.hits() - before);
}
