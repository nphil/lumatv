//! Cameras through the LumaShow integration: a fake integration dials the helper's bridge, shares a camera and sends an event; the
//! pop-up reaches the hub within the budget, the picture route fetches the camera's picture once for a burst of askers, the cooldown
//! holds, a cleared pop-up is gone, and `bye` takes the cameras away. Off, nothing listens.

mod support;

use ha_bridge::fake::FakeIntegration;
use ha_bridge::wire::{CameraList, CameraRec, Message, SnapshotRec};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use support::*;

const KEY: [u8; 32] = [0x5a; 32];
const CAMERA: &str = "camera.front_door";
/// A 32x18 baseline JPEG.
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x0A, 0x07, 0x07, 0x08, 0x07, 0x06, 0x0A,
    0x08, 0x08, 0x08, 0x0B, 0x0A, 0x0A, 0x0B, 0x0E, 0x18, 0x10, 0x0E, 0x0D, 0x0D, 0x0E, 0x1D, 0x15,
    0x16, 0x11, 0x18, 0x23, 0x1F, 0x25, 0x24, 0x22, 0x1F, 0x22, 0x21, 0x26, 0x2B, 0x37, 0x2F, 0x26,
    0x29, 0x34, 0x29, 0x21, 0x22, 0x30, 0x41, 0x31, 0x34, 0x39, 0x3B, 0x3E, 0x3E, 0x3E, 0x25, 0x2E,
    0x44, 0x49, 0x43, 0x3C, 0x48, 0x37, 0x3D, 0x3E, 0x3B, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x0A, 0x0B,
    0x0B, 0x0E, 0x0D, 0x0E, 0x1C, 0x10, 0x10, 0x1C, 0x3B, 0x28, 0x22, 0x28, 0x3B, 0x3B, 0x3B, 0x3B,
    0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B,
    0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B,
    0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0x3B, 0xFF, 0xC0,
    0x00, 0x11, 0x08, 0x00, 0x12, 0x00, 0x20, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11,
    0x01, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05,
    0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
    0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23,
    0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
    0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A,
    0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A,
    0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
    0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
    0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5,
    0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1,
    0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xC4, 0x00, 0x1F, 0x01, 0x00, 0x03,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x11, 0x00,
    0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00,
    0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
    0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15,
    0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27,
    0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88,
    0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6,
    0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4,
    0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9,
    0xFA, 0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0xF3,
    0x5A, 0x28, 0xA2, 0xBA, 0x8A, 0x0A, 0x28, 0xA2, 0x80, 0x0A, 0x28, 0xA2, 0x80, 0x0A, 0x28, 0xA2,
    0x80, 0x3F, 0xFF, 0xD9,
];

/// Home Assistant's picture endpoint: answers the signed path with the JPEG and counts the fetches.
struct PictureServer {
    addr: SocketAddr,
    fetched: Arc<AtomicUsize>,
}

impl PictureServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let fetched = Arc::new(AtomicUsize::new(0));
        let count = fetched.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let count = count.clone();
                thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let mut head = Vec::new();
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        let Ok(n) = stream.read(&mut buf) else { return };
                        if n == 0 {
                            return;
                        }
                        head.extend_from_slice(&buf[..n]);
                    }
                    let text = String::from_utf8_lossy(&head);
                    let ok = text.starts_with(&format!(
                        "GET /api/camera_proxy/{CAMERA}?authSig=abc.def HTTP/1.1"
                    )) && !text.contains("Authorization");
                    let response = if ok {
                        count.fetch_add(1, Ordering::SeqCst);
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            JPEG.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(JPEG);
                        r
                    } else {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    };
                    let _ = stream.write_all(&response);
                });
            }
        });
        Self { addr, fetched }
    }

    fn snapshot_url(&self) -> String {
        format!(
            "http://{}/api/camera_proxy/{CAMERA}?authSig=abc.def",
            self.addr
        )
    }
}

fn camera_list(url: &str) -> Message {
    Message::CamerasList(CameraList {
        rev: 1,
        partial: false,
        cameras: vec![CameraRec {
            id: CAMERA.into(),
            name: "Front door".into(),
            area: Some("entry".into()),
            snapshot: Some(SnapshotRec {
                url: url.to_owned(),
                expires: 4_102_444_800,
            }),
            streams: Vec::new(),
        }],
    })
}

fn module_message(kind: &str, body: Value) -> Message {
    let Value::Object(body) = body else {
        panic!("an object")
    };
    Message::Other {
        module: "camera_events".into(),
        kind: kind.into(),
        body,
    }
}

fn start(enabled: bool, deps: lumatv_helper::cameras::Deps) -> Rig {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let config = test_config(
        json!({"cameras": {"enabled": enabled, "port": 0, "cooldownSeconds": 30, "popupSeconds": 12, "frameHoldMs": 400}}),
    );
    rig.start_cameras(config, Script::new(), deps);
    rig
}

#[test]
fn off_or_unpaired_nothing_listens_and_the_topic_says_so() {
    let rig = start(false, lumatv_helper::cameras::testing::keyed_deps(KEY));
    let topic = rig.topic("cameras").unwrap();
    assert_eq!(topic["state"], "off");
    assert_eq!(topic["enabled"], false);
    assert!(topic["port"].is_null(), "{topic}");
    drop(rig);
    let rig = start(true, lumatv_helper::cameras::testing::inert_deps());
    let topic = rig.topic("cameras").unwrap();
    assert_eq!(topic["state"], "unpaired");
    assert_eq!(topic["paired"], false);
    assert!(topic["port"].is_null(), "{topic}");
}

#[test]
fn an_event_from_the_integration_is_a_popup_within_the_budget_and_the_picture_is_fetched_once_for_a_burst()
 {
    let pictures = PictureServer::start();
    let rig = start(true, lumatv_helper::cameras::testing::keyed_deps(KEY));
    let topic = rig
        .wait_topic("cameras", Duration::from_secs(5), |t| {
            t["state"] == "waiting" && t["port"].is_u64()
        })
        .expect("the bridge listens");
    let addr: SocketAddr = format!("127.0.0.1:{}", topic["port"].as_u64().unwrap())
        .parse()
        .unwrap();

    let mut ha = FakeIntegration::connect(addr, &KEY).unwrap();
    let welcome = ha
        .hello(&[
            ("states", 1),
            ("actions", 1),
            ("cameras", 1),
            ("notify", 1),
            ("camera_events", 1),
        ])
        .unwrap();
    assert_eq!(
        welcome.modules.get("camera_events"),
        Some(&1),
        "{welcome:?}"
    );
    assert!(
        welcome.device.name.starts_with("lumatv"),
        "{:?}",
        welcome.device
    );
    // The TV asks for the kinds list when the module's session begins.
    let refresh = ha.recv_where(Duration::from_secs(3), |m| match m {
        Message::Other { module, kind, .. } if module == "camera_events" && kind == "refresh" => {
            Some(())
        }
        _ => None,
    });
    assert!(refresh.is_some(), "the TV asks for the list");

    ha.send(&camera_list(&pictures.snapshot_url())).unwrap();
    ha.send(&module_message(
        "list",
        json!({"cameras": [{"id": CAMERA, "kinds": ["person", "vehicle", "doorbell"]}]}),
    ))
    .unwrap();
    let topic = rig
        .wait_topic("cameras", Duration::from_secs(5), |t| {
            t["cameras"].as_array().is_some_and(|c| !c.is_empty())
        })
        .expect("the camera is listed");
    let camera = &topic["cameras"][0];
    assert_eq!(camera["id"], CAMERA);
    assert_eq!(camera["name"], "Front door");
    assert_eq!(camera["kinds"], json!(["person", "vehicle", "doorbell"]));
    assert_eq!(camera["picture"], true);
    assert!(
        topic.to_string().find("authSig").is_none(),
        "no picture address in a topic: {topic}"
    );

    // The event: a pop-up within a second of the integration's message.
    let sent = Instant::now();
    ha.send(&module_message(
        "event",
        json!({"camera": CAMERA, "kind": "person", "at": 1793160000.5, "source": "binary_sensor.front_door_person"}),
    ))
    .unwrap();
    let popup = rig
        .wait_topic("cameras.popup", Duration::from_secs(3), |p| {
            p["camera"] == CAMERA
        })
        .expect("a pop-up");
    let took = sent.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "event to pop-up took {took:?}"
    );
    assert_eq!(popup["kind"], "person");
    assert_eq!(popup["name"], "Front door");
    assert_eq!(popup["lastsMs"], 12_000);
    assert_eq!(popup["test"], false);
    assert!(popup["untilMs"].as_u64().unwrap() > popup["at"].as_f64().unwrap() as u64);

    // The picture: one fetch for a burst of askers (the pop-up and a grid tile), served as the file (no decoder on this host).
    let reply = get(rig.addr, &format!("/v1/cameras/{CAMERA}/frame?w=640&h=360"));
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    assert_eq!(reply.header("content-type"), Some("image/jpeg"));
    assert_eq!(reply.header("x-luma-decoder"), Some("file"));
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    assert_eq!(reply.body, JPEG);
    let again = get(rig.addr, &format!("/v1/cameras/{CAMERA}/frame"));
    assert_eq!(again.status, 200);
    assert_eq!(
        pictures.fetched.load(Ordering::SeqCst),
        1,
        "held for frameHoldMs"
    );
    thread::sleep(Duration::from_millis(450));
    let later = get(rig.addr, &format!("/v1/cameras/{CAMERA}/frame"));
    assert_eq!(later.status, 200);
    assert_eq!(
        pictures.fetched.load(Ordering::SeqCst),
        2,
        "fetched again after the hold"
    );
    let unknown = get(rig.addr, "/v1/cameras/camera.nowhere/frame");
    assert_eq!(unknown.status, 404);
    let bad = get(rig.addr, "/v1/cameras/light.x/frame");
    assert_eq!(bad.status, 400);

    // The cooldown: the same camera and kind again is held; a doorbell is not; another camera is not.
    let held = post(
        rig.addr,
        "/v1/cameras/event",
        &json!({"camera": CAMERA, "kind": "person"}).to_string(),
    );
    assert_eq!(held.json()["held"], "cooldown", "{}", held.json());
    let bell = post(
        rig.addr,
        "/v1/cameras/event",
        &json!({"camera": CAMERA, "kind": "doorbell"}).to_string(),
    );
    assert_eq!(bell.json()["ok"], true);
    assert_eq!(bell.json()["lastsMs"], 18_000);
    let popup = rig
        .wait_topic("cameras.popup", Duration::from_secs(3), |p| {
            p["kind"] == "doorbell"
        })
        .expect("the doorbell pop-up");
    assert_eq!(popup["test"], true);
    let not_wanted = post(
        rig.addr,
        "/v1/cameras/event",
        &json!({"camera": CAMERA, "kind": "motion"}).to_string(),
    );
    assert_eq!(not_wanted.json()["held"], "kind");

    // The shell cleared it: gone for every reader. The event stream saw both events.
    let cleared = post(rig.addr, "/v1/cameras/popup/clear", "{}");
    assert_eq!(cleared.status, 200);
    assert!(
        rig.wait_topic("cameras.popup", Duration::from_secs(2), Value::is_null)
            .is_some()
    );
    let events = get(rig.addr, "/v1/events?after=0&topics=cameras.event").json();
    let kinds: Vec<String> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["data"]["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["person", "doorbell"]);

    // Bye: the cameras go with the session; the bridge waits again.
    ha.bye("unloaded").unwrap();
    // the mirror empties and the link says waiting; the two are published one after the other, so both are waited for
    rig.wait_topic("cameras", Duration::from_secs(5), |t| {
        t["cameras"].as_array().is_some_and(Vec::is_empty) && t["state"] == "waiting"
    })
    .expect("the cameras are gone and the bridge waits again");
    let gone = get(rig.addr, &format!("/v1/cameras/{CAMERA}/frame"));
    assert_eq!(gone.status, 404);
    let status = get(rig.addr, "/v1/cameras").json();
    assert_eq!(
        status["cameras"]["popupKinds"],
        json!(["person", "vehicle", "animal", "doorbell"])
    );
}

#[test]
fn a_wrong_key_is_refused_and_a_strange_event_is_ignored() {
    let rig = start(true, lumatv_helper::cameras::testing::keyed_deps(KEY));
    let topic = rig
        .wait_topic("cameras", Duration::from_secs(5), |t| t["port"].is_u64())
        .expect("the bridge listens");
    let addr: SocketAddr = format!("127.0.0.1:{}", topic["port"].as_u64().unwrap())
        .parse()
        .unwrap();
    let mut wrong = FakeIntegration::connect(addr, &[0x11; 32]).unwrap();
    assert!(
        wrong.hello(&[("states", 1)]).is_err(),
        "a wrong key opens nothing"
    );

    let mut ha = FakeIntegration::connect(addr, &KEY).unwrap();
    ha.hello(&[("states", 1), ("camera_events", 1)]).unwrap();
    for body in [
        json!({"camera": "light.x", "kind": "person"}),
        json!({"camera": CAMERA, "kind": "ghost"}),
        json!({"kind": "person"}),
    ] {
        ha.send(&module_message("event", body)).unwrap();
    }
    thread::sleep(Duration::from_millis(300));
    assert!(rig.topic("cameras.popup").unwrap().is_null());
}
