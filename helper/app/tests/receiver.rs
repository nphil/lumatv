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
    let idle = rig
        .wait_topic("receiver", WAIT, |v| v["polling"] == false)
        .expect("an idle document");
    assert_eq!(idle["fresh"], false);
    assert_eq!(idle["reachable"], Value::Null);
    assert_eq!(idle["volumeDb"], Value::Null);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(avr.hits(), 0, "polled a receiver with the screen off");

    // The screen comes on: polling starts and the first reading is shown.
    luna.set_state("Active");
    let live = rig
        .wait_topic("receiver", WAIT, fresh)
        .expect("a live reading");
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
    rig.wait_topic("receiver", WAIT, |v| v["polling"] == false)
        .expect("polling stopped");
    thread::sleep(Duration::from_millis(100));
    let hits = avr.hits();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(avr.hits(), hits, "kept polling with the screen off");
    let after = rig.topic("receiver").unwrap();
    assert_eq!(after["fresh"], false);
    assert_eq!(
        after["volumeDb"],
        Value::Null,
        "a reading is hidden once polling stops"
    );
}

#[test]
fn a_change_at_the_receiver_is_an_event() {
    let avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({}));
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("first reading");
    let seq = get(rig.addr, "/v1/state").json()["seq"].as_u64().unwrap();

    avr.set(|s| s.volume = "-40.0".into());
    let moved = rig
        .wait_topic("receiver", WAIT, |v| v["volumeDb"] == -40.0)
        .expect("the new volume");
    assert_eq!(moved["volumeAbs"], 40.0);
    assert_eq!(moved["volumeText"], "40");

    avr.set(|s| s.mute = "on".into());
    rig.wait_topic("receiver", WAIT, |v| v["mute"] == true)
        .expect("mute");
    avr.set(|s| {
        s.input = "GAME".into();
        s.power = "OFF".into();
    });
    let changed = rig
        .wait_topic("receiver", WAIT, |v| {
            v["input"] == "GAME" && v["power"] == "standby"
        })
        .expect("input and power");
    assert_eq!(changed["mute"], true);

    // Each change was one event on the topic; unchanged polls raised none.
    let events = get(
        rig.addr,
        &format!("/v1/events?after={seq}&topics=receiver&timeout=0.2"),
    )
    .json();
    let volumes: Vec<_> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["data"]["volumeDb"].clone(),
                e["data"]["mute"].clone(),
                e["data"]["input"].clone(),
            )
        })
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
fn polls_that_change_nothing_move_the_stamp_in_the_snapshot_and_raise_no_event() {
    let avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({"pollMs": 40}));
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("first reading");
    let before = get(rig.addr, "/v1/state/receiver").json();
    let seq = before["seq"].as_u64().unwrap();
    let at = before["value"]["updatedAt"].as_u64().unwrap();
    // Several polls later the snapshot carries a newer stamp (a surface that asks the helper learns the receiver
    // is still answering) while the event log has nothing new: no long poll was woken for a stamp.
    assert!(
        wait_until(WAIT, || {
            get(rig.addr, "/v1/state/receiver").json()["value"]["updatedAt"]
                .as_u64()
                .is_some_and(|now| now > at + 100)
        }),
        "the stamp never moved"
    );
    let events = get(
        rig.addr,
        &format!("/v1/events?after={seq}&topics=receiver&timeout=0.2"),
    )
    .json();
    assert_eq!(events["events"].as_array().unwrap().len(), 0, "{events}");
}

#[test]
fn a_receiver_that_goes_quiet_is_hidden_after_the_window_not_shown_stale() {
    let mut avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({"hideAfterMs": 400}));
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("first reading");
    avr.shut_down();
    // Within the window the last reading is still shown; after it, nothing is.
    let hidden = rig
        .wait_topic("receiver", WAIT, |v| {
            v["fresh"] == false && v["polling"] == true
        })
        .expect("hidden after the window");
    assert_eq!(hidden["reachable"], false);
    assert_eq!(hidden["volumeDb"], Value::Null);
    assert_eq!(hidden["mute"], Value::Null);
    assert_eq!(hidden["input"], Value::Null);
    assert_eq!(hidden["error"]["code"], "unreachable");
    assert!(
        hidden["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not reachable")
    );
    assert!(
        hidden["updatedAt"].is_u64(),
        "the time of the last good reading is kept"
    );
}

#[test]
fn a_forbidden_page_is_an_error_not_a_reading() {
    let avr = FakeAvr::start();
    avr.set(|s| s.forbidden = true);
    let (rig, _luna) = rig_with(&avr, "Active", json!({}));
    let doc = rig
        .wait_topic("receiver", WAIT, |v| {
            v["polling"] == true && v["error"]["code"] == "http"
        })
        .expect("an http error");
    assert_eq!(doc["fresh"], false);
    assert_eq!(doc["reachable"], false);
    assert!(doc["error"]["message"].as_str().unwrap().contains("403"));
}

#[test]
fn a_nudge_reads_now_instead_of_at_the_next_poll() {
    let avr = FakeAvr::start();
    // A poll period far longer than the test: only a nudge can make the change show. The reading is kept for as long as the test takes: a
    // test thread starved for seconds by a busy host must not find it hidden (it is hidden 6 s after it was read).
    let (rig, _luna) = rig_with(
        &avr,
        "Active",
        json!({"pollMs": 20_000, "fastPollMs": 15, "burstMs": 400, "hideAfterMs": 600_000}),
    );
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("first reading");
    let hits = avr.hits();
    avr.set(|s| s.volume = "-30.5".into());
    thread::sleep(Duration::from_millis(200));
    assert_eq!(avr.hits(), hits, "polled between periods");
    assert_eq!(rig.topic("receiver").unwrap()["volumeDb"], -44.5);

    assert_eq!(post(rig.addr, "/v1/receiver/nudge", "").status, 200);
    let moved = rig.wait_topic("receiver", Duration::from_secs(3), |v| {
        v["volumeDb"] == -30.5
    });
    assert!(
        moved.is_some(),
        "the nudge did not make the receiver be read"
    );
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
    rig.wait_topic("receiver", WAIT, |v| v["polling"] == false)
        .expect("idle");
    assert_eq!(post(rig.addr, "/v1/receiver/nudge", "").status, 200);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(avr.hits(), 0);
}

#[test]
fn switching_the_receiver_off_in_the_settings_stops_the_polling() {
    let avr = FakeAvr::start();
    let (rig, _luna) = rig_with(&avr, "Active", json!({}));
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("first reading");
    let off = r#"{"changes":[{"key":"enabled","value":{"type":"bool","value":false}}]}"#;
    assert_eq!(
        post(rig.addr, "/v1/settings/base.receiver", off).status,
        200
    );
    let doc = rig
        .wait_topic("receiver", WAIT, |v| v["enabled"] == false)
        .expect("disabled");
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
    let seen = rig
        .wait_topic("receiver", WAIT, fresh)
        .expect("first receiver");
    assert_eq!(seen["volumeDb"], -44.5);
    let body =
        json!({"changes": [{"key": "port", "value": {"type": "integer", "value": second.port}}]})
            .to_string();
    assert_eq!(
        post(rig.addr, "/v1/settings/base.receiver", &body).status,
        200
    );
    let moved = rig
        .wait_topic("receiver", WAIT, |v| v["volumeDb"] == -12.0)
        .expect("the second receiver");
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
    assert!(
        (1..=6).contains(&asked),
        "asked {asked} times in {:?}",
        started.elapsed()
    );
    // The receiver comes back: the next retry (within the cap) shows it.
    avr.set(|s| s.forbidden = false);
    let back = rig
        .wait_topic("receiver", Duration::from_secs(4), fresh)
        .expect("recovered");
    assert_eq!(back["reachable"], true);
    assert_eq!(back["error"], Value::Null);
    // Back to the normal period: about ten asks a second, not one.
    let before = avr.hits();
    thread::sleep(Duration::from_millis(600));
    assert!(
        avr.hits() - before >= 3,
        "still backed off: {}",
        avr.hits() - before
    );
}

#[test]
fn a_held_volume_key_drives_the_receiver_to_the_displays_level_and_it_ends_there() {
    // The set's Denon takes one CEC step in 150 ms; a held key makes 25 a second. The display's level climbs half a step every
    // 80 ms and is posted on every key repeat: the receiver is sent at most one level in 60 ms and ends on the display's level,
    // although a CEC step that was still in flight lands after the last command.
    let avr = FakeAvr::start();
    avr.set(|s| s.volume = "-44.5".into());
    let (rig, _luna) = rig_with(
        &avr,
        "Active",
        json!({"volumeGapMs": 60, "volumeSettleMs": 150}),
    );
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("first reading");
    assert_eq!(avr.level(), 35.5);
    let mut taken = 0;
    let mut level = 35.5;
    for repeat in 0..50 {
        if avr.cec_step(true) {
            taken += 1;
        }
        if repeat % 2 == 0 {
            level += 0.5;
        }
        let body = format!(r#"{{"level":{level}}}"#);
        assert_eq!(post(rig.addr, "/v1/receiver/volume", &body).status, 200);
        thread::sleep(Duration::from_millis(40));
    }
    avr.cec_step_after(true, Duration::from_millis(60));
    assert!(
        taken < 16,
        "the fake took {taken} CEC steps of 50 keys: it is not slow"
    );
    assert!(
        wait_until(Duration::from_secs(3), || avr.level() == level),
        "the receiver stopped at {} and the display at {level}",
        avr.level()
    );
    thread::sleep(Duration::from_millis(400));
    assert_eq!(avr.level(), level, "the receiver moved after it arrived");
    let commands = avr.commands();
    assert!(
        commands.len() <= 2_000 / 60 + 4,
        "{} commands in two seconds",
        commands.len()
    );
    for pair in commands.windows(2) {
        assert!(
            pair[1].0.duration_since(pair[0].0) >= Duration::from_millis(55),
            "two commands {:?} apart",
            pair[1].0.duration_since(pair[0].0)
        );
    }
    assert_eq!(
        commands.last().unwrap().1,
        "MV48",
        "the last one sent is the display's level"
    );
    let doc = rig.topic("receiver").unwrap();
    assert!(doc["volumeAbs"].is_number());
}

#[test]
fn a_volume_level_is_not_sent_while_the_screen_is_off_or_the_receiver_switched_off() {
    let avr = FakeAvr::start();
    let (rig, luna) = rig_with(&avr, "Active Standby", json!({"volumeGapMs": 20}));
    rig.wait_topic("receiver", WAIT, |v| v["polling"] == false)
        .expect("idle");
    assert_eq!(
        post(rig.addr, "/v1/receiver/volume", r#"{"level":40}"#).status,
        200
    );
    thread::sleep(Duration::from_millis(300));
    assert!(avr.commands().is_empty(), "sent with the screen off");

    luna.set_state("Active");
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    let off = r#"{"changes":[{"key":"enabled","value":{"type":"bool","value":false}}]}"#;
    assert_eq!(
        post(rig.addr, "/v1/settings/base.receiver", off).status,
        200
    );
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        post(rig.addr, "/v1/receiver/volume", r#"{"level":40}"#).status,
        200
    );
    thread::sleep(Duration::from_millis(300));
    assert!(avr.commands().is_empty(), "sent to a receiver switched off");
}
