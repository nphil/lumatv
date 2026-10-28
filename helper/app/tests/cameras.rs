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

// ---- the live path: a fake HLS source, the real host feeder, the fake NDL ----------------------------------------------------------

/// Home Assistant's stream integration, as far as the live path sees it: a master playlist, a media playlist that grows a segment at a
/// time (then ends, or stays live), the initialisation segment and the segments of the fixture.
struct HlsServer {
    addr: SocketAddr,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

impl HlsServer {
    fn start(ends: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = requests.clone();
        let started = Instant::now();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let log = log.clone();
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
                    let text = String::from_utf8_lossy(&head).into_owned();
                    let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
                    log.lock().unwrap().push(path.clone());
                    let (kind, body): (&str, Vec<u8>) = match path.split('?').next().unwrap_or("") {
                        "/api/hls/tok/master_playlist.m3u8" => (
                            "application/vnd.apple.mpegurl",
                            b"#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-STREAM-INF:BANDWIDTH=500000,CODECS=\"avc1.4d401e\"\nplaylist.m3u8\n".to_vec(),
                        ),
                        "/api/hls/tok/playlist.m3u8" => {
                            // a segment more every 400 ms, three in all
                            let have = ((started.elapsed().as_millis() / 400) as usize + 1).min(3);
                            let mut p = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n");
                            for i in 0..have {
                                p.push_str(&format!("#EXTINF:1.000000,\nsegment/{i}.m4s\n"));
                            }
                            if ends && have == 3 {
                                p.push_str("#EXT-X-ENDLIST\n");
                            }
                            ("application/vnd.apple.mpegurl", p.into_bytes())
                        }
                        "/api/hls/tok/init.mp4" => ("video/mp4", include_bytes!("fixtures/hls/init.mp4").to_vec()),
                        "/api/hls/tok/segment/0.m4s" => ("video/iso.segment", include_bytes!("fixtures/hls/seg0.m4s").to_vec()),
                        "/api/hls/tok/segment/1.m4s" => ("video/iso.segment", include_bytes!("fixtures/hls/seg1.m4s").to_vec()),
                        "/api/hls/tok/segment/2.m4s" => ("video/iso.segment", include_bytes!("fixtures/hls/seg2.m4s").to_vec()),
                        _ => ("text/plain", Vec::new()),
                    };
                    let status = if body.is_empty() {
                        "404 Not Found"
                    } else {
                        "200 OK"
                    };
                    let mut r = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&body);
                    let _ = stream.write_all(&r);
                });
            }
        });
        Self { addr, requests }
    }

    fn url(&self) -> String {
        format!("http://{}/api/hls/tok/master_playlist.m3u8", self.addr)
    }
}

fn host_feeder() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gfn/feeder/build/host");
    let program = root.join("lumatv-gfn-feeder");
    let library = root.join("libNDL_directmedia.so.1");
    (program.is_file() && library.is_file()).then_some((program, library))
}

/// Answers the integration's part of the live module: a `start` gets a `started` with the fake server's address.
fn answer_start(ha: &mut FakeIntegration, url: &str) -> (u64, String) {
    let (op, camera) = ha
        .recv_where(Duration::from_secs(5), |m| match m {
            Message::Other { module, kind, body } if module == "camera_live" && kind == "start" => {
                Some((
                    body.get("op").and_then(Value::as_u64).unwrap_or(0),
                    body.get("camera")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                ))
            }
            _ => None,
        })
        .expect("the TV asks for the stream");
    ha.send(&module_message_of(
        "camera_live",
        "started",
        json!({"op": op, "camera": camera, "kind": "hls", "url": url}),
    ))
    .unwrap();
    (op, camera)
}

fn module_message_of(module: &str, kind: &str, body: Value) -> Message {
    let Value::Object(body) = body else {
        panic!("an object")
    };
    Message::Other {
        module: module.into(),
        kind: kind.into(),
        body,
    }
}

fn ndl_log_lines(path: &std::path::Path, what: &str) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(what))
        .count()
}

#[test]
fn the_full_view_plays_the_stream_on_the_video_plane_through_the_feeder_and_stops_clean() {
    let Some((program, library)) = host_feeder() else {
        eprintln!("skipped: the host feeder is not built (sh gfn/feeder/build.sh)");
        return;
    };
    let run_dir = tempfile::tempdir().unwrap();
    let ndl_log = run_dir.path().join("ndl.log");
    let spec = lumatv_helper::live::FeederSpec {
        program,
        window: "card".into(),
        library: library.display().to_string(),
        extra_args: vec!["--no-window".into()],
        env: vec![
            ("FAKE_NDL_LOG".into(), ndl_log.display().to_string()),
            (
                "LD_LIBRARY_PATH".into(),
                library.parent().unwrap().display().to_string(),
            ),
        ],
        run_dir: run_dir.path().to_path_buf(),
    };
    let mut deps = lumatv_helper::cameras::testing::keyed_deps(KEY);
    deps.feeder = Some(spec);
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let config = test_config(json!({"cameras": {"enabled": true, "live": true, "port": 0}}));
    rig.start_cameras(config, Script::new(), deps);
    let topic = rig
        .wait_topic("cameras", Duration::from_secs(5), |t| t["port"].is_u64())
        .expect("the bridge listens");
    assert_eq!(topic["live"], true, "{topic}");
    let live = rig.topic("cameras.live").unwrap();
    assert_eq!(live["state"], "idle");

    // not before the integration is there
    let refused = post(
        rig.addr,
        "/v1/cameras/live",
        &json!({"camera": CAMERA}).to_string(),
    );
    assert_eq!(refused.status, 409);

    let addr: SocketAddr = format!("127.0.0.1:{}", topic["port"].as_u64().unwrap())
        .parse()
        .unwrap();
    let mut ha = FakeIntegration::connect(addr, &KEY).unwrap();
    let welcome = ha
        .hello(&[
            ("states", 1),
            ("cameras", 1),
            ("camera_events", 1),
            ("camera_live", 1),
        ])
        .unwrap();
    assert_eq!(welcome.modules.get("camera_live"), Some(&1), "{welcome:?}");
    ha.send(&camera_list(
        "http://127.0.0.1:1/api/camera_proxy/camera.front_door?authSig=x",
    ))
    .unwrap();

    // a stream that ends after three segments: the whole of it reaches the feeder, then the session is over by itself
    let server = HlsServer::start(true);
    let started = Instant::now();
    let asked = post(
        rig.addr,
        "/v1/cameras/live",
        &json!({"camera": CAMERA}).to_string(),
    );
    assert_eq!(
        asked.status,
        200,
        "{}",
        String::from_utf8_lossy(&asked.body)
    );
    let (_op, camera) = answer_start(&mut ha, &server.url());
    assert_eq!(camera, CAMERA);
    let playing = rig
        .wait_topic("cameras.live", Duration::from_secs(10), |l| {
            l["state"] == "playing"
        })
        .unwrap_or_else(|| {
            panic!(
                "the stream plays: {:?} / ndl: {}",
                rig.topic("cameras.live"),
                std::fs::read_to_string(&ndl_log).unwrap_or_default()
            )
        });
    let to_first_picture = started.elapsed();
    assert_eq!(playing["camera"], CAMERA);
    assert_eq!(playing["window"], "_Window_Id_none");
    assert_eq!(playing["pieceMs"], 1000);
    let ended = rig
        .wait_topic("cameras.live", Duration::from_secs(15), |l| {
            l["state"] == "stopped"
        })
        .expect("the stream ends by itself");
    assert_eq!(ended["reason"], "the stream ended");
    assert_eq!(
        (ended["fed"].as_u64(), ended["dropped"].as_u64()),
        (Some(30), Some(0)),
        "{ended}"
    );
    // the fake library's own count when the feeder quit: every access unit reached it, in an order it accepts (rising timestamps,
    // the parameter sets with the key picture), and it was unloaded on the way out
    assert!(
        wait_until(Duration::from_secs(5), || ndl_log_lines(
            &ndl_log,
            "Quit played=30 shown=30 dropped=0"
        ) == 1),
        "every access unit reached the (fake) decoder: {}",
        std::fs::read_to_string(&ndl_log).unwrap_or_default()
    );
    assert!(
        ndl_log_lines(&ndl_log, "Unload") >= 1,
        "unloaded on the way out"
    );
    assert_eq!(
        ndl_log_lines(&ndl_log, "VIOLATION"),
        0,
        "the demands the real library makes: {}",
        std::fs::read_to_string(&ndl_log).unwrap_or_default()
    );
    eprintln!("live: start to playing {to_first_picture:?} on this host");
    let asked_for: Vec<String> = server.requests.lock().unwrap().clone();
    assert!(asked_for.iter().any(|p| p.ends_with("init.mp4")));
    assert!(
        asked_for.iter().filter(|p| p.contains("segment/")).count() == 3,
        "{asked_for:?}"
    );
    assert!(
        asked_for.iter().all(|p| !p.contains("authSig")),
        "no picture address is fetched by the live path"
    );

    // a live stream that does not end: the viewer closes, the session stops, the feeder is gone, the integration is told
    let live_server = HlsServer::start(false);
    assert_eq!(
        post(
            rig.addr,
            "/v1/cameras/live",
            &json!({"camera": CAMERA}).to_string()
        )
        .status,
        200
    );
    answer_start(&mut ha, &live_server.url());
    rig.wait_topic("cameras.live", Duration::from_secs(10), |l| {
        l["state"] == "playing"
    })
    .expect("the second stream plays");
    assert!(wait_until(Duration::from_secs(5), || rig
        .topic("cameras.live")
        .is_some_and(|l| l["fed"].as_u64().unwrap_or(0) >= 10)));
    let stopped = post(rig.addr, "/v1/cameras/live/stop", "{}");
    assert_eq!(stopped.status, 200);
    let after = rig
        .wait_topic("cameras.live", Duration::from_secs(5), |l| {
            l["state"] == "stopped"
        })
        .expect("stopped");
    assert_eq!(after["reason"], "the viewer closed");
    let told = ha.recv_where(Duration::from_secs(5), |m| match m {
        Message::Other { module, kind, .. } if module == "camera_live" && kind == "stop" => {
            Some(())
        }
        _ => None,
    });
    assert!(
        told.is_some(),
        "the integration is told the stream is no longer wanted"
    );
    assert!(
        wait_until(Duration::from_secs(5), || ndl_log_lines(
            &ndl_log,
            "Quit played="
        ) == 2),
        "the second feeder quit too"
    );
    assert!(ndl_log_lines(&ndl_log, "Unload") >= 2);
    assert_eq!(ndl_log_lines(&ndl_log, "VIOLATION"), 0);
    assert!(
        !std::fs::read_dir(run_dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".sock")),
        "the socket is gone"
    );

    // the integration cannot give the stream: said in words, nothing started
    assert_eq!(
        post(
            rig.addr,
            "/v1/cameras/live",
            &json!({"camera": CAMERA}).to_string()
        )
        .status,
        200
    );
    let (op, _) = ha
        .recv_where(Duration::from_secs(5), |m| match m {
            Message::Other { module, kind, body } if module == "camera_live" && kind == "start" => {
                Some((body.get("op").and_then(Value::as_u64).unwrap_or(0), ()))
            }
            _ => None,
        })
        .unwrap();
    ha.send(&module_message_of(
        "camera_live",
        "failed",
        json!({"op": op, "code": "no_stream", "message": "the camera has no stream"}),
    ))
    .unwrap();
    let failed = rig
        .wait_topic("cameras.live", Duration::from_secs(5), |l| {
            l["state"] == "failed"
        })
        .expect("failed in words");
    assert_eq!(failed["reason"], "no_stream: the camera has no stream");
}

/// The milliseconds of the clock the fake NDL library stamps its log with.
fn monotonic_ms() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a plain clock read into a timespec this function owns.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000
}

/// `(when in ms, played number)` of every `Play` line of the fake library's log.
fn ndl_plays(path: &std::path::Path) -> Vec<(i64, u64)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            (words.next()? == "T").then_some(())?;
            let when: i64 = words.next()?.parse().ok()?;
            (words.next()? == "Play").then_some(())?;
            let number: u64 = words.next()?.trim_start_matches('#').parse().ok()?;
            Some((when, number))
        })
        .collect()
}

/// The fixture's 30 access units, as the integration's relay would send them (Annex B, the parameter sets before the key pictures).
fn fixture_units() -> (
    lumatv_helper::live::fmp4::Init,
    Vec<lumatv_helper::live::fmp4::AccessUnit>,
) {
    use lumatv_helper::live::fmp4;
    let init = fmp4::parse_init(include_bytes!("fixtures/hls/init.mp4")).unwrap();
    let mut units = Vec::new();
    for segment in [
        &include_bytes!("fixtures/hls/seg0.m4s")[..],
        &include_bytes!("fixtures/hls/seg1.m4s")[..],
        &include_bytes!("fixtures/hls/seg2.m4s")[..],
    ] {
        units.extend(fmp4::parse_segment(&init, segment).unwrap());
    }
    (init, units)
}

#[test]
fn the_relay_feeds_each_picture_the_moment_it_arrives_and_a_lost_chunk_loses_one_picture() {
    use lumatv_helper::live::relay;
    let Some((program, library)) = host_feeder() else {
        eprintln!("skipped: the host feeder is not built (sh gfn/feeder/build.sh)");
        return;
    };
    let run_dir = tempfile::tempdir().unwrap();
    let ndl_log = run_dir.path().join("ndl.log");
    let spec = lumatv_helper::live::FeederSpec {
        program,
        window: "card".into(),
        library: library.display().to_string(),
        extra_args: vec!["--no-window".into()],
        env: vec![
            ("FAKE_NDL_LOG".into(), ndl_log.display().to_string()),
            ("FAKE_NDL_VERBOSE".into(), "1".into()),
            (
                "LD_LIBRARY_PATH".into(),
                library.parent().unwrap().display().to_string(),
            ),
        ],
        run_dir: run_dir.path().to_path_buf(),
    };
    let mut deps = lumatv_helper::cameras::testing::keyed_deps(KEY);
    deps.feeder = Some(spec);
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let config = test_config(json!({"cameras": {"enabled": true, "live": true, "port": 0}}));
    rig.start_cameras(config, Script::new(), deps);
    let topic = rig
        .wait_topic("cameras", Duration::from_secs(5), |t| t["port"].is_u64())
        .expect("the bridge listens");
    let addr: SocketAddr = format!("127.0.0.1:{}", topic["port"].as_u64().unwrap())
        .parse()
        .unwrap();
    let mut ha = FakeIntegration::connect(addr, &KEY).unwrap();
    ha.hello(&[
        ("states", 1),
        ("cameras", 1),
        ("camera_events", 1),
        ("camera_live", 1),
    ])
    .unwrap();
    ha.send(&camera_list(
        "http://127.0.0.1:1/api/camera_proxy/camera.front_door?authSig=x",
    ))
    .unwrap();

    assert_eq!(
        post(
            rig.addr,
            "/v1/cameras/live",
            &json!({"camera": CAMERA}).to_string()
        )
        .status,
        200
    );
    // the TV asks for the relay first, HLS after
    let (op, formats) = ha
        .recv_where(Duration::from_secs(5), |m| match m {
            Message::Other { module, kind, body } if module == "camera_live" && kind == "start" => {
                Some((
                    body.get("op").and_then(Value::as_u64).unwrap_or(0),
                    body.get("formats").cloned().unwrap_or(Value::Null),
                ))
            }
            _ => None,
        })
        .expect("the TV asks for the stream");
    assert_eq!(formats, json!(["relay", "hls"]));
    let (init, units) = fixture_units();
    assert_eq!(units.len(), 30);
    let sets = esphome_api::base64::encode(&init.parameter_sets);
    ha.send(&module_message_of(
        "camera_live",
        "started",
        json!({"op": op, "camera": CAMERA, "kind": "relay", "stream": 5, "codec": "h264", "clock": 90000, "parameter_sets": sets}),
    ))
    .unwrap();
    rig.wait_topic("cameras.live", Duration::from_secs(10), |l| {
        l["state"] == "starting" && l["kind"] == "relay"
    })
    .expect("the feeder is started before the first picture");

    // the pictures at 30 a second, as the camera would send them: each is stamped when it leaves
    let mut seq = 0u32;
    let mut left_at: Vec<i64> = Vec::new();
    let frame_time = Duration::from_millis(33);
    let started = Instant::now();
    for (i, unit) in units.iter().enumerate() {
        let timestamp = (unit.dts * 90_000 / i64::from(init.timescale)) as u32;
        let frames = relay::frames(5, &mut seq, timestamp, unit.key, &unit.data, 60_000);
        left_at.push(monotonic_ms());
        for frame in &frames {
            ha.send_frame(relay::FRAME_TYPE, frame).unwrap();
        }
        let due = started + frame_time * (i as u32 + 1);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
    let playing = rig
        .wait_topic("cameras.live", Duration::from_secs(10), |l| {
            l["state"] == "playing"
        })
        .unwrap_or_else(|| {
            panic!(
                "the relay plays: {:?} / ndl: {}",
                rig.topic("cameras.live"),
                std::fs::read_to_string(&ndl_log).unwrap_or_default()
            )
        });
    assert_eq!(playing["kind"], "relay");
    assert_eq!(playing["pieceMs"], 0);
    assert!(
        wait_until(Duration::from_secs(5), || ndl_plays(&ndl_log).len() == 30),
        "every picture reached the (fake) decoder: {}",
        std::fs::read_to_string(&ndl_log).unwrap_or_default()
    );
    // the delay from a picture leaving the integration to the decoder's Play: no segment, no playlist, no wait for the next picture
    let plays = ndl_plays(&ndl_log);
    let mut delays: Vec<i64> = plays
        .iter()
        .zip(&left_at)
        .map(|((when, _), left)| when - left)
        .collect();
    delays.sort_unstable();
    let (median, worst) = (delays[delays.len() / 2], *delays.last().unwrap());
    eprintln!(
        "relay: integration to decoder on this host: median {median} ms, worst {worst} ms, all {delays:?}"
    );
    // the first picture carries the pipeline's load; a loaded host adds spikes, so the worst is held loosely and the median tightly
    assert!(median <= 20, "median {median} ms: {delays:?}");
    assert!(worst <= 500, "worst {worst} ms: {delays:?}");
    assert_eq!(
        ndl_log_lines(&ndl_log, "VIOLATION"),
        0,
        "{}",
        std::fs::read_to_string(&ndl_log).unwrap_or_default()
    );

    // a chunk lost on the way: the picture it belonged to is dropped, the next one plays, and the loss is counted
    let big: Vec<u8> = {
        let mut d = units[0].data.clone();
        d.resize(130_000, 0);
        d
    };
    let mut broken = relay::frames(5, &mut seq, 90_000 * 2, false, &big, 60_000);
    assert_eq!(broken.len(), 3);
    broken.remove(1);
    for frame in &broken {
        ha.send_frame(relay::FRAME_TYPE, frame).unwrap();
    }
    for (i, unit) in units.iter().enumerate().skip(1).take(5) {
        let timestamp = (90_000 * 2 + i as i64 * 3000) as u32;
        for frame in relay::frames(5, &mut seq, timestamp, unit.key, &unit.data, 60_000) {
            ha.send_frame(relay::FRAME_TYPE, &frame).unwrap();
        }
    }
    assert!(
        wait_until(Duration::from_secs(5), || ndl_plays(&ndl_log).len() == 35),
        "the pictures after the broken one play: {} plays",
        ndl_plays(&ndl_log).len()
    );
    assert!(
        wait_until(Duration::from_secs(3), || rig
            .topic("cameras.live")
            .is_some_and(|l| l["lost"] == 1)),
        "the loss is counted: {:?}",
        rig.topic("cameras.live")
    );
    assert_eq!(ndl_log_lines(&ndl_log, "VIOLATION"), 0);

    // the restream ends on Home Assistant's side: the TV hears it, the feeder is gone, nothing is left running
    ha.send(&module_message_of(
        "camera_live",
        "ended",
        json!({"camera": CAMERA, "stream": 5, "reason": "the stream ended"}),
    ))
    .unwrap();
    let ended = rig
        .wait_topic("cameras.live", Duration::from_secs(5), |l| {
            l["state"] == "stopped"
        })
        .expect("stopped");
    assert_eq!(ended["reason"], "the stream ended: the stream ended");
    assert!(
        wait_until(Duration::from_secs(5), || ndl_log_lines(
            &ndl_log,
            "Quit played=35"
        ) == 1),
        "the feeder quit: {}",
        std::fs::read_to_string(&ndl_log).unwrap_or_default()
    );
    assert!(
        !std::fs::read_dir(run_dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".sock")),
        "the socket is gone"
    );
    // a frame of a stream that is over is nothing
    ha.send_frame(
        relay::FRAME_TYPE,
        &relay::frames(5, &mut seq, 1, true, b"x", 60_000)[0],
    )
    .unwrap();
    assert_eq!(rig.topic("cameras.live").unwrap()["state"], "stopped");
}

#[test]
fn with_the_relay_off_the_tv_asks_for_hls_only() {
    let mut deps = lumatv_helper::cameras::testing::keyed_deps(KEY);
    deps.feeder = Some(lumatv_helper::live::FeederSpec {
        program: "/nonexistent/feeder".into(),
        window: "card".into(),
        library: String::new(),
        extra_args: Vec::new(),
        env: Vec::new(),
        run_dir: std::env::temp_dir(),
    });
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let config =
        test_config(json!({"cameras": {"enabled": true, "live": true, "relay": false, "port": 0}}));
    rig.start_cameras(config, Script::new(), deps);
    let topic = rig
        .wait_topic("cameras", Duration::from_secs(5), |t| t["port"].is_u64())
        .expect("the bridge listens");
    let addr: SocketAddr = format!("127.0.0.1:{}", topic["port"].as_u64().unwrap())
        .parse()
        .unwrap();
    let mut ha = FakeIntegration::connect(addr, &KEY).unwrap();
    ha.hello(&[("states", 1), ("cameras", 1), ("camera_live", 1)])
        .unwrap();
    ha.send(&camera_list(
        "http://127.0.0.1:1/api/camera_proxy/camera.front_door?authSig=x",
    ))
    .unwrap();
    assert_eq!(
        post(
            rig.addr,
            "/v1/cameras/live",
            &json!({"camera": CAMERA}).to_string()
        )
        .status,
        200
    );
    let asked = ha
        .recv_where(Duration::from_secs(5), |m| match m {
            Message::Other { module, kind, body } if module == "camera_live" && kind == "start" => {
                Some(Value::Object(body.clone()))
            }
            _ => None,
        })
        .expect("the TV asks for the stream");
    assert_eq!(asked["formats"], json!(["hls"]));
    assert_eq!(asked["format"], "hls");
}
