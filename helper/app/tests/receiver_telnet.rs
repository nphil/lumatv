//! The receiver's telnet link end to end: a fake receiver that speaks the S760H's telnet protocol on the loopback (queries
//! answered, changes pushed, connections counted), the fake HTTP receiver beside it for the fallback, a fake `luna-send`, and
//! the real service in between.

mod support;

use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use support::*;

const WAIT: Duration = Duration::from_secs(10);

// ----------------------------------------------------------------------------------------------- fake telnet receiver

#[derive(Clone)]
struct Values {
    power: String,
    /// `MV` digits as the receiver writes them: `355` is 35.5.
    volume: String,
    mute: String,
    input: String,
}

struct Inner {
    values: Mutex<Values>,
    connections: Mutex<Vec<TcpStream>>,
    received: Mutex<Vec<String>>,
    live: AtomicUsize,
    max_live: AtomicUsize,
    total: AtomicUsize,
    /// Read what the client sends but answer nothing and push nothing.
    silent: AtomicBool,
    listening: AtomicBool,
    stopping: AtomicBool,
}

struct FakeTelnet {
    port: u16,
    inner: Arc<Inner>,
    thread: Option<JoinHandle<()>>,
}

impl FakeTelnet {
    fn start() -> Self {
        Self::start_on(true)
    }

    /// A port nothing listens on yet: `listen()` opens it.
    fn start_closed() -> Self {
        Self::start_on(false)
    }

    fn start_on(listening: bool) -> Self {
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = first.local_addr().unwrap().port();
        let inner = Arc::new(Inner {
            values: Mutex::new(Values {
                power: "ON".into(),
                volume: "355".into(),
                mute: "OFF".into(),
                input: "TV".into(),
            }),
            connections: Mutex::new(Vec::new()),
            received: Mutex::new(Vec::new()),
            live: AtomicUsize::new(0),
            max_live: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            silent: AtomicBool::new(false),
            listening: AtomicBool::new(listening),
            stopping: AtomicBool::new(false),
        });
        let thread = {
            let inner = inner.clone();
            thread::spawn(move || accept_loop(&inner, first, port))
        };
        Self {
            port,
            inner,
            thread: Some(thread),
        }
    }

    /// Opens the port (after `start_closed` or `unlisten`).
    fn listen(&self) {
        self.inner.listening.store(true, Ordering::SeqCst);
    }

    /// Closes the port and every connection: new attempts are refused.
    fn unlisten(&self) {
        self.inner.listening.store(false, Ordering::SeqCst);
        self.drop_connections();
    }

    fn drop_connections(&self) {
        for stream in self.inner.connections.lock().iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn set_silent(&self, silent: bool) {
        self.inner.silent.store(silent, Ordering::SeqCst);
    }

    /// The receiver changes (a knob, a remote, Home Assistant) and tells every client.
    fn change(&self, edit: impl FnOnce(&mut Values), push: &[&str]) {
        edit(&mut self.inner.values.lock());
        for line in push {
            self.push(line);
        }
    }

    fn push(&self, line: &str) {
        for mut stream in self.inner.connections.lock().iter() {
            let _ = stream.write_all(format!("{line}\r").as_bytes());
        }
    }

    fn live(&self) -> usize {
        self.inner.live.load(Ordering::SeqCst)
    }

    fn max_live(&self) -> usize {
        self.inner.max_live.load(Ordering::SeqCst)
    }

    fn total(&self) -> usize {
        self.inner.total.load(Ordering::SeqCst)
    }

    fn received(&self) -> Vec<String> {
        self.inner.received.lock().clone()
    }
}

impl Drop for FakeTelnet {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.drop_connections();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept_loop(inner: &Arc<Inner>, first: TcpListener, port: u16) {
    let mut listener = Some(first);
    let mut handlers: Vec<JoinHandle<()>> = Vec::new();
    while !inner.stopping.load(Ordering::SeqCst) {
        let want = inner.listening.load(Ordering::SeqCst);
        if !want {
            listener = None;
        } else if listener.is_none() {
            listener = TcpListener::bind(("127.0.0.1", port)).ok();
        }
        if let Some(l) = &listener {
            l.set_nonblocking(true).unwrap();
            if let Ok((stream, _)) = l.accept() {
                stream.set_nonblocking(false).unwrap();
                let now = inner.live.fetch_add(1, Ordering::SeqCst) + 1;
                inner.max_live.fetch_max(now, Ordering::SeqCst);
                inner.total.fetch_add(1, Ordering::SeqCst);
                inner.connections.lock().push(stream.try_clone().unwrap());
                let inner = inner.clone();
                handlers.push(thread::spawn(move || serve(&inner, stream)));
                continue;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    drop(listener);
    for handler in handlers {
        let _ = handler.join();
    }
}

fn serve(inner: &Arc<Inner>, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
    let mut pending = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        if inner.stopping.load(Ordering::SeqCst) {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => pending.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        while let Some(end) = pending.iter().position(|b| *b == b'\r') {
            let line = String::from_utf8_lossy(&pending[..end]).into_owned();
            pending.drain(..=end);
            inner.received.lock().push(line.clone());
            if inner.silent.load(Ordering::SeqCst) {
                continue;
            }
            let v = inner.values.lock().clone();
            let reply = match line.as_str() {
                "PW?" => format!("PW{}\r", if v.power == "ON" { "ON" } else { "STANDBY" }),
                "MV?" => format!("MV{}\rMVMAX 665\r", v.volume),
                "MU?" => format!("MU{}\r", v.mute),
                "SI?" => format!("SI{}\rSVOFF\r", v.input),
                _ => String::new(),
            };
            if stream.write_all(reply.as_bytes()).is_err() {
                break;
            }
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    inner.live.fetch_sub(1, Ordering::SeqCst);
}

// ----------------------------------------------------------------------------------------------- rig

fn fresh(v: &Value) -> bool {
    v["fresh"] == true
}

/// A service whose screen follows a fake luna-send, whose HTTP receiver is `http` and whose telnet port is `telnet`'s.
fn rig_with(http: &FakeAvr, telnet: &FakeTelnet, luna_state: &str, tune: Value) -> (Rig, FakeLuna) {
    let mut rig = Rig::new();
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::Subscribes, luna_state);
    luna.answer_sound_output();
    receiver_at(&rig.paths, http.port);
    let mut receiver = json!({"telnetPort": telnet.port});
    if let (Value::Object(base), Value::Object(extra)) = (&mut receiver, tune) {
        base.extend(extra);
    }
    let config = test_config(json!({
        "screen": {"source": "luna", "pollMs": 40},
        "luna": {"send": luna.program.display().to_string()},
        "receiver": receiver,
    }));
    rig.start(config, Script::new());
    (rig, luna)
}

/// A rhythm where a heartbeat is slow beside a push and a reading stays fresh between heartbeats.
fn slow() -> Value {
    json!({"pollMs": 400, "hideAfterMs": 1500, "maxBackoffMs": 400})
}

fn wait_for(what: &str, check: impl FnMut() -> bool) {
    assert!(wait_until(WAIT, check), "timed out waiting for {what}");
}

// ----------------------------------------------------------------------------------------------- tests

#[test]
fn changes_reach_the_topic_as_they_are_pushed_and_nothing_is_polled() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(&http, &telnet, "Active", slow());

    let live = rig
        .wait_topic("receiver", WAIT, fresh)
        .expect("a first reading");
    assert_eq!(live["reachable"], true);
    assert_eq!(live["power"], "on");
    assert_eq!(live["input"], "TV");
    assert_eq!(live["volumeDb"], -44.5);
    assert_eq!(live["volumeAbs"], 35.5);
    assert_eq!(
        live["volumeMax"], 66.5,
        "the receiver's own limit, from its MVMAX"
    );
    assert_eq!(live["mute"], false);
    // The panel's scale is not on the telnet port: one HTTP read learns it.
    let shown = rig
        .wait_topic("receiver", WAIT, |v| v["volumeDisplay"] == "absolute")
        .expect("the scale learned over HTTP");
    assert_eq!(shown["volumeText"], "35.5");
    let http_reads = http.hits();
    assert_eq!(http_reads, 1, "one HTTP read for the scale, no polling");

    // Each change is on the topic well before the next heartbeat (400 ms) could have found it.
    let seq = get(rig.addr, "/v1/state").json()["seq"].as_u64().unwrap();
    let started = Instant::now();
    telnet.change(|v| v.volume = "36".into(), &["MV36"]);
    rig.wait_topic("receiver", Duration::from_millis(300), |v| {
        v["volumeDb"] == -44.0
    })
    .expect("the volume within 300 ms of the push");
    assert!(started.elapsed() < Duration::from_millis(300));
    let moved = rig.topic("receiver").unwrap();
    assert_eq!(moved["volumeText"], "36", "the scale survives the push");

    telnet.change(|v| v.mute = "ON".into(), &["MUON"]);
    rig.wait_topic("receiver", Duration::from_millis(300), |v| {
        v["mute"] == true
    })
    .expect("mute");
    telnet.change(|v| v.input = "GAME".into(), &["SIGAME", "SVOFF"]);
    rig.wait_topic("receiver", Duration::from_millis(300), |v| {
        v["input"] == "GAME"
    })
    .expect("input");
    telnet.change(|v| v.power = "STANDBY".into(), &["PWSTANDBY"]);
    let off = rig
        .wait_topic("receiver", Duration::from_millis(300), |v| {
            v["power"] == "standby"
        })
        .expect("power");
    assert_eq!(off["volumeDb"], -44.0);
    assert_eq!(off["mute"], true);
    assert_eq!(off["input"], "GAME");

    // One event per change, in order; lines that are not state (`MVMAX`, `SVOFF`) raised none.
    let events = get(
        rig.addr,
        &format!("/v1/events?after={seq}&topics=receiver&timeout=0.2"),
    )
    .json();
    let seen: Vec<_> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let d = &e["data"];
            (
                d["volumeDb"].clone(),
                d["mute"].clone(),
                d["input"].clone(),
                d["power"].clone(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (json!(-44.0), json!(false), json!("TV"), json!("on")),
            (json!(-44.0), json!(true), json!("TV"), json!("on")),
            (json!(-44.0), json!(true), json!("GAME"), json!("on")),
            (json!(-44.0), json!(true), json!("GAME"), json!("standby")),
        ],
        "{events}"
    );

    assert_eq!(
        http.hits(),
        http_reads,
        "HTTP was not read while the link held"
    );
    assert_eq!(telnet.total(), 1, "one connection");
    assert_eq!(telnet.max_live(), 1);
}

#[test]
fn the_link_only_asks_questions() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(&http, &telnet, "Active", json!({"pollMs": 100}));
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    thread::sleep(Duration::from_millis(500));
    let received = telnet.received();
    assert!(received.len() >= 8, "heartbeats: {received:?}");
    // The receiver's read form ends in `?`; anything else would change its state.
    for line in &received {
        assert!(line.ends_with('?'), "sent {line:?}");
    }
}

#[test]
fn settings_queries_stop_after_page_interest_expires() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(&http, &telnet, "Active", slow());
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    assert_eq!(get(rig.addr, "/v1/receiver/settings").status, 200);
    wait_for("a settings query", || {
        telnet.received().iter().any(|q| q == "PSTONE CTRL ?")
    });
    thread::sleep(Duration::from_secs(6));
    let count = telnet
        .received()
        .iter()
        .filter(|q| *q == "PSTONE CTRL ?")
        .count();
    thread::sleep(Duration::from_secs(2));
    assert_eq!(
        telnet
            .received()
            .iter()
            .filter(|q| *q == "PSTONE CTRL ?")
            .count(),
        count
    );
}

#[test]
fn a_quiet_receiver_keeps_a_fresh_stamp_without_raising_events() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    // The window (300 ms) is shorter than the time watched: only the heartbeat's answers keep the reading shown.
    let (rig, _luna) = rig_with(
        &http,
        &telnet,
        "Active",
        json!({"pollMs": 100, "hideAfterMs": 300}),
    );
    // Settled: the reading is there and the scale has been learned (that is one event, before the quiet).
    rig.wait_topic("receiver", WAIT, |v| {
        fresh(v) && v["volumeDisplay"] == "absolute"
    })
    .expect("a reading with its scale");
    let seq = get(rig.addr, "/v1/state").json()["seq"].as_u64().unwrap();
    let stamp = rig.topic("receiver").unwrap()["updatedAt"]
        .as_u64()
        .unwrap();
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(100));
        assert!(
            fresh(&rig.topic("receiver").unwrap()),
            "hidden while the link was fine"
        );
    }
    let later = rig.topic("receiver").unwrap()["updatedAt"]
        .as_u64()
        .unwrap();
    assert!(later >= stamp + 500, "the stamp moved {stamp} -> {later}");
    let events = get(
        rig.addr,
        &format!("/v1/events?after={seq}&topics=receiver&timeout=0.1"),
    )
    .json();
    assert_eq!(events["events"].as_array().unwrap().len(), 0, "{events}");
}

#[test]
fn a_dropped_connection_is_reopened_and_pushes_continue() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(
        &http,
        &telnet,
        "Active",
        json!({"pollMs": 100, "maxBackoffMs": 300}),
    );
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    assert_eq!(telnet.total(), 1);

    telnet.drop_connections();
    wait_for("a second connection", || {
        telnet.total() == 2 && telnet.live() == 1
    });
    // The new connection works: a push arrives and the reading is fresh again.
    telnet.change(|v| v.volume = "40".into(), &["MV40"]);
    rig.wait_topic("receiver", WAIT, |v| v["volumeDb"] == -40.0 && fresh(v))
        .expect("a push over the new connection");
    assert_eq!(telnet.max_live(), 1, "never two connections at once");
}

#[test]
fn http_covers_while_telnet_is_down_and_stops_when_it_is_back() {
    let http = FakeAvr::start();
    http.set(|s| s.volume = "-40.0".into());
    let telnet = FakeTelnet::start_closed();
    let (rig, _luna) = rig_with(
        &http,
        &telnet,
        "Active",
        json!({"pollMs": 50, "hideAfterMs": 400, "maxBackoffMs": 200}),
    );
    // Refused: the reading comes over HTTP.
    let covered = rig
        .wait_topic("receiver", WAIT, |v| fresh(v) && v["volumeDb"] == -40.0)
        .expect("a reading over HTTP");
    assert_eq!(covered["reachable"], true);
    assert_eq!(telnet.total(), 0);
    let before = http.hits();
    thread::sleep(Duration::from_millis(300));
    assert!(http.hits() > before, "the fallback keeps reading");

    // The receiver's telnet port opens: the link takes over (its value, not the stale HTTP one) and HTTP goes quiet.
    telnet.listen();
    rig.wait_topic("receiver", WAIT, |v| fresh(v) && v["volumeDb"] == -44.5)
        .expect("the telnet reading");
    wait_for("the link", || telnet.live() == 1);
    thread::sleep(Duration::from_millis(100));
    let settled = http.hits();
    thread::sleep(Duration::from_millis(500));
    assert!(
        http.hits() <= settled + 1,
        "HTTP read {} more times with the link up",
        http.hits() - settled
    );
    telnet.change(|v| v.volume = "36".into(), &["MV36"]);
    rig.wait_topic("receiver", Duration::from_millis(300), |v| {
        v["volumeDb"] == -44.0
    })
    .expect("a push");
    assert_eq!(telnet.max_live(), 1);
}

#[test]
fn a_receiver_that_stops_answering_is_dropped_and_read_over_http() {
    let http = FakeAvr::start();
    http.set(|s| s.volume = "-40.0".into());
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(
        &http,
        &telnet,
        "Active",
        json!({"pollMs": 100, "hideAfterMs": 600, "maxBackoffMs": 300}),
    );
    rig.wait_topic("receiver", WAIT, |v| fresh(v) && v["volumeDb"] == -44.5)
        .expect("a telnet reading");
    let before = http.hits();

    // The socket stays open but nothing is answered: the link gives up (after pollMs + requestTimeoutMs) and HTTP reads.
    telnet.set_silent(true);
    rig.wait_topic("receiver", WAIT, |v| fresh(v) && v["volumeDb"] == -40.0)
        .expect("HTTP took over");
    assert!(http.hits() > before);
    assert_eq!(
        telnet.max_live(),
        1,
        "the dead connection closed before another opened"
    );

    // It answers again: the link comes back and HTTP stops.
    telnet.set_silent(false);
    rig.wait_topic("receiver", WAIT, |v| fresh(v) && v["volumeDb"] == -44.5)
        .expect("the link is back");
    thread::sleep(Duration::from_millis(200));
    let settled = http.hits();
    thread::sleep(Duration::from_millis(500));
    assert!(http.hits() <= settled + 1);
    assert_eq!(telnet.max_live(), 1);
}

#[test]
fn a_dead_receiver_is_hidden_after_the_window_not_shown_stale() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(
        &http,
        &telnet,
        "Active",
        json!({"pollMs": 50, "hideAfterMs": 300, "maxBackoffMs": 200}),
    );
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    drop(http);
    telnet.unlisten();
    let hidden = rig
        .wait_topic("receiver", WAIT, |v| {
            v["fresh"] == false && v["polling"] == true
        })
        .expect("hidden after the window");
    assert_eq!(hidden["volumeDb"], Value::Null);
    assert_eq!(hidden["mute"], Value::Null);
    assert_eq!(hidden["reachable"], false);
    assert_eq!(hidden["error"]["code"], "unreachable");
    assert!(hidden["updatedAt"].is_u64());
}

#[test]
fn the_connection_exists_only_while_the_screen_is_on() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, luna) = rig_with(&http, &telnet, "Active Standby", json!({"pollMs": 100}));
    rig.wait_topic("receiver", WAIT, |v| v["polling"] == false)
        .expect("idle");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(telnet.total(), 0, "connected with the screen off");
    assert_eq!(http.hits(), 0);

    luna.set_state("Active");
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    assert_eq!(telnet.live(), 1);

    luna.set_state("Active Standby");
    wait_for("the connection to close", || telnet.live() == 0);
    let off = rig
        .wait_topic("receiver", WAIT, |v| v["polling"] == false)
        .expect("idle again");
    assert_eq!(off["fresh"], false);
    assert_eq!(off["volumeDb"], Value::Null);
    let total = telnet.total();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(telnet.total(), total, "reconnected with the screen off");

    luna.set_state("Active");
    rig.wait_topic("receiver", WAIT, fresh)
        .expect("a reading again");
    assert_eq!(telnet.live(), 1);
    assert_eq!(telnet.max_live(), 1);
}

#[test]
fn the_connection_exists_only_while_the_plugin_is_on() {
    let http = FakeAvr::start();
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(&http, &telnet, "Active", json!({"pollMs": 100}));
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    assert_eq!(telnet.live(), 1);

    assert_eq!(switch_receiver(rig.addr, false).status, 200);
    wait_for("the connection to close", || telnet.live() == 0);
    let (total, hits) = (telnet.total(), http.hits());
    thread::sleep(Duration::from_millis(400));
    assert_eq!((telnet.total(), http.hits()), (total, hits), "reached a receiver whose plugin is off");

    assert_eq!(switch_receiver(rig.addr, true).status, 200);
    rig.wait_topic("receiver", WAIT, |v| v["enabled"] == true && fresh(v))
        .expect("a reading again");
    assert_eq!(telnet.live(), 1);
}

#[test]
fn a_scale_that_could_not_be_read_is_asked_for_again() {
    let http = FakeAvr::start();
    http.set(|s| s.forbidden = true);
    let telnet = FakeTelnet::start();
    let (rig, _luna) = rig_with(
        &http,
        &telnet,
        "Active",
        json!({"pollMs": 100, "hideAfterMs": 1000, "maxBackoffMs": 200}),
    );
    let first = rig
        .wait_topic("receiver", WAIT, fresh)
        .expect("a reading without the scale");
    assert_eq!(first["volumeDisplay"], "unknown");
    assert_eq!(first["volumeText"], "-44.5 dB");

    http.set(|s| s.forbidden = false);
    let learned = rig
        .wait_topic("receiver", WAIT, |v| v["volumeDisplay"] == "absolute")
        .expect("the scale, learned on a later try");
    assert_eq!(learned["volumeText"], "35.5");
    let settled = http.hits();
    thread::sleep(Duration::from_millis(600));
    assert_eq!(
        http.hits(),
        settled,
        "no more reads once the scale is known"
    );
}

#[test]
fn a_new_address_in_the_settings_moves_the_connection() {
    let http = FakeAvr::start();
    let first = FakeTelnet::start();
    let (rig, _luna) = rig_with(&http, &first, "Active", json!({"pollMs": 100}));
    rig.wait_topic("receiver", WAIT, fresh).expect("a reading");
    assert_eq!(first.live(), 1);

    // `127.0.0.2` is loopback too, but nothing listens there: the old connection must close and none may be added.
    let body =
        json!({"changes": [{"key": "host", "value": {"type": "text", "value": "127.0.0.2"}}]})
            .to_string();
    assert_eq!(
        post(rig.addr, "/v1/settings/denon-receiver", &body).status,
        200
    );
    wait_for("the old connection to close", || first.live() == 0);
    let moved = rig
        .wait_topic("receiver", WAIT, |v| {
            v["host"] == "127.0.0.2" && v["reachable"] == false
        })
        .expect("the new address, nothing read from it");
    assert_eq!(moved["reachable"], false);
    assert_eq!(first.total(), 1);
    assert_eq!(first.max_live(), 1);
}
