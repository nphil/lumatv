//! Hue bulbs through Zigbee2MQTT straight from the TV, end to end against a fake MQTT broker on the loopback (in this file: it
//! takes CONNECT, SUBSCRIBE, PUBLISH, PINGREQ and DISCONNECT, serves retained messages, and can refuse a connection, push a new
//! message or drop every client). A real helper runs in a temporary data dir with the real screen watcher (a fake `luna-send`) and
//! the capture socket. Nothing here goes beyond 127.0.0.1; no broker, Zigbee2MQTT or lamp of anyone's is touched.
//!
//! What is held to: the broker is connected to with the saved user name and password, which appear in no output; the device list
//! and the lights' states are subscribed to; the start sequence is Bifrost's (light balance, then command 3 with the counter, to
//! each bulb); the frames are byte for byte what Bifrost packs for the same colours, to the first bulb's topic, with every bulb's
//! record and the counter running on; the picture's colours reach each bulb by where it sits; the screen going off sends black,
//! command 3 and each bulb's earlier state back; a bulb Zigbee2MQTT does not know is reported in words and streamed to once the
//! list names it; a bulb's new address is used; a refused password and a dropped connection are reported and retried; and
//! `z2m-link` checks the lights against the device list before saving, privately.

mod support;

use lumatv_ambient::plan::Still;
use lumatv_ambient::zones::{MAX_ZONES, Preset, ZoneFrame};
use lumatv_ambient::{Rgb, wire};
use lumatv_helper::ambient_z2m_frame::{self as fr, Mode};
use lumatv_helper::ambient_z2m_store::{self, Z2mLight, Z2mLink};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use support::*;

const BIN: &str = env!("CARGO_BIN_EXE_lumatv-helper");
const WAIT: Duration = Duration::from_secs(60);
const USER: &str = "lumatv";
const PASSWORD: &str = "hunter2-very-secret-words";
const LEFT: &str = "TV left";
const RIGHT: &str = "TV right";
const LEFT_ADDR: u16 = 0xD297;
const RIGHT_ADDR: u16 = 0xD298;

// ------------------------------------------------------------------------------------------------- the fake broker

#[derive(Clone, Debug)]
struct Published {
    topic: String,
    payload: Value,
    /// When the broker took it in: a lower bound on a wait holds on a busy machine, a count in a wall-clock window does not.
    at: Instant,
}

#[derive(Default)]
struct State {
    retained: HashMap<String, Vec<u8>>,
    /// The CONNACK code to answer with instead of 0.
    refuse: Option<u8>,
    /// (client id, user name, password) of every CONNECT.
    connects: Vec<(String, Option<String>, Option<String>)>,
    subscribed: Vec<String>,
    published: Vec<Published>,
    /// Live clients: a writer and the topics each subscribed to.
    clients: Vec<ClientHandle>,
}

type ClientHandle = (Arc<Mutex<TcpStream>>, Arc<Mutex<Vec<String>>>);

struct Broker {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

/// MQTT's topic filter matching: `+` is one level, `#` the rest.
fn filter_matches(filter: &str, topic: &str) -> bool {
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn mqtt_str(s: &str) -> Vec<u8> {
    let mut v = (u16::try_from(s.len()).unwrap()).to_be_bytes().to_vec();
    v.extend_from_slice(s.as_bytes());
    v
}

fn mqtt_packet(first: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    let mut n = body.len();
    loop {
        let mut b = u8::try_from(n % 128).unwrap();
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}

fn take_str(body: &[u8], at: &mut usize) -> String {
    let len = usize::from(u16::from_be_bytes([body[*at], body[*at + 1]]));
    let s = String::from_utf8_lossy(&body[*at + 2..*at + 2 + len]).into_owned();
    *at += 2 + len;
    s
}

/// Reads one packet (blocking, with the stream's timeout); `None` when the connection ended.
fn read_packet(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut first = [0u8; 1];
    match stream.read(&mut first) {
        Ok(0) => return None,
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Some((0, Vec::new()));
        }
        Err(_) => return None,
    }
    let mut len = 0usize;
    let mut shift = 0;
    loop {
        let mut b = [0u8; 1];
        read_rest(stream, &mut b)?;
        len |= usize::from(b[0] & 0x7F) << shift;
        shift += 7;
        if b[0] & 0x80 == 0 {
            break;
        }
    }
    let mut body = vec![0u8; len];
    read_rest(stream, &mut body)?;
    Some((first[0], body))
}

/// The rest of a packet whose first byte has come. The socket's short read timeout is for waiting between packets: a client that a busy
/// machine kept from finishing one for longer than that is not gone.
fn read_rest(stream: &mut TcpStream, buf: &mut [u8]) -> Option<()> {
    let deadline = Instant::now() + WAIT;
    let mut at = 0;
    while at < buf.len() {
        match stream.read(&mut buf[at..]) {
            Ok(0) => return None,
            Ok(n) => at += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && Instant::now() < deadline => {}
            Err(_) => return None,
        }
    }
    Some(())
}

impl Broker {
    fn start() -> Broker {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        thread::spawn(move || {
            while !sp.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (st, sp) = (st.clone(), sp.clone());
                        thread::spawn(move || serve(stream, &st, &sp));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Broker { addr, state, stop }
    }

    fn retain(&self, topic: &str, payload: &Value) {
        let bytes = payload.to_string().into_bytes();
        self.state
            .lock()
            .retained
            .insert(topic.to_owned(), bytes.clone());
        self.deliver(topic, &bytes, 0x31);
    }

    /// A message that is not retained, as Zigbee2MQTT publishes its logs and a device's state.
    fn push(&self, topic: &str, payload: &Value) {
        self.deliver(topic, payload.to_string().as_bytes(), 0x30);
    }

    /// Every subscriber whose filter matches gets it now.
    fn deliver(&self, topic: &str, payload: &[u8], first: u8) {
        let mut body = mqtt_str(topic);
        body.extend_from_slice(payload);
        let packet = mqtt_packet(first, &body);
        for (writer, subs) in &self.state.lock().clients {
            if subs.lock().iter().any(|s| filter_matches(s, topic)) {
                let _ = writer.lock().write_all(&packet);
            }
        }
    }

    fn refuse(&self, code: u8) {
        self.state.lock().refuse = Some(code);
    }

    /// Drops every connection, as a broker restart would.
    fn kill(&self) {
        let st = self.state.lock();
        for (writer, _) in &st.clients {
            let _ = writer.lock().shutdown(Shutdown::Both);
        }
    }

    fn published(&self) -> Vec<Published> {
        self.state.lock().published.clone()
    }

    fn sets(&self, name: &str) -> Vec<Value> {
        let topic = format!("zigbee2mqtt/{name}/set");
        self.published()
            .into_iter()
            .filter(|p| p.topic == topic)
            .map(|p| p.payload)
            .collect()
    }

    fn connects(&self) -> Vec<(String, Option<String>, Option<String>)> {
        self.state.lock().connects.clone()
    }

    fn subscribed(&self) -> Vec<String> {
        self.state.lock().subscribed.clone()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.kill();
    }
}

fn serve(mut stream: TcpStream, state: &Arc<Mutex<State>>, stop: &Arc<AtomicBool>) {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let writer = Arc::new(Mutex::new(stream.try_clone().unwrap()));
    let subs = Arc::new(Mutex::new(Vec::<String>::new()));
    state.lock().clients.push((writer.clone(), subs.clone()));
    let send = |bytes: &[u8]| {
        let _ = writer.lock().write_all(bytes);
    };
    while !stop.load(Ordering::Relaxed) {
        let Some((first, body)) = read_packet(&mut stream) else {
            break;
        };
        match first & 0xF0 {
            0x00 => {}
            0x10 => {
                let mut at = 0;
                let _protocol = take_str(&body, &mut at);
                let _level = body[at];
                let flags = body[at + 1];
                at += 4;
                let id = take_str(&body, &mut at);
                if flags & 0x04 != 0 {
                    take_str(&body, &mut at);
                    take_str(&body, &mut at);
                }
                let user = (flags & 0x80 != 0).then(|| take_str(&body, &mut at));
                let pass = (flags & 0x40 != 0).then(|| take_str(&body, &mut at));
                let code = {
                    let mut st = state.lock();
                    st.connects.push((id, user, pass));
                    st.refuse.unwrap_or(0)
                };
                send(&[0x20, 0x02, 0x00, code]);
                if code != 0 {
                    break;
                }
            }
            0x30 => {
                let mut at = 0;
                let topic = take_str(&body, &mut at);
                if (first >> 1) & 0x03 > 0 {
                    at += 2;
                }
                let payload = serde_json::from_slice(&body[at..]).unwrap_or_else(|_| {
                    Value::String(String::from_utf8_lossy(&body[at..]).into_owned())
                });
                state.lock().published.push(Published {
                    topic,
                    payload,
                    at: Instant::now(),
                });
            }
            0x80 => {
                let id = [body[0], body[1]];
                let mut at = 2;
                let mut codes = Vec::new();
                let mut topics = Vec::new();
                while at < body.len() {
                    let t = take_str(&body, &mut at);
                    at += 1;
                    codes.push(0u8);
                    topics.push(t);
                }
                let mut ack = vec![id[0], id[1]];
                ack.extend_from_slice(&codes);
                send(&mqtt_packet(0x90, &ack));
                let retained: Vec<(String, Vec<u8>)> = {
                    let mut st = state.lock();
                    st.subscribed.extend(topics.iter().cloned());
                    st.retained
                        .iter()
                        .filter(|(t, _)| topics.iter().any(|f| filter_matches(f, t)))
                        .map(|(t, p)| (t.clone(), p.clone()))
                        .collect()
                };
                subs.lock().extend(topics);
                for (t, p) in retained {
                    let mut b = mqtt_str(&t);
                    b.extend_from_slice(&p);
                    send(&mqtt_packet(0x31, &b));
                }
            }
            0xC0 => send(&[0xD0, 0x00]),
            0xE0 => break,
            _ => {}
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

// ------------------------------------------------------------------------------------------------- the setup

fn devices(entries: &[(&str, u16)]) -> Value {
    let mut list = vec![
        json!({"friendly_name": "Coordinator", "type": "Coordinator", "network_address": 0, "ieee_address": "0x00"}),
    ];
    for (name, addr) in entries {
        list.push(json!({
            "friendly_name": name, "type": "Router", "network_address": addr, "ieee_address": format!("0x{addr:016x}"),
            "definition": {"vendor": "Philips", "model": "9290024688", "description": "Hue white and color ambiance E26/E27"},
            "endpoints": {"11": {"clusters": {"input": ["genBasic", "genOnOff", "genLevelCtrl", "lightingColorCtrl", "manuSpecificPhilips2", "manuSpecificPhilips3"]}}},
        }));
    }
    list.push(json!({"friendly_name": "plug", "type": "Router", "network_address": 0x1111, "definition": {"vendor": "IKEA", "model": "E1603"}}));
    Value::Array(list)
}

fn two_bulbs(broker: &Broker) {
    broker.retain(
        "zigbee2mqtt/bridge/devices",
        &devices(&[(LEFT, LEFT_ADDR), (RIGHT, RIGHT_ADDR)]),
    );
    broker.retain(
        &format!("zigbee2mqtt/{LEFT}"),
        &json!({"state": "OFF", "brightness": 254, "linkquality": 120}),
    );
    broker.retain(
        &format!("zigbee2mqtt/{RIGHT}"),
        &json!({"state": "ON", "brightness": 120, "color_mode": "color_temp", "color_temp": 366, "color": {"x": 0.4, "y": 0.4}}),
    );
}

fn lights() -> Vec<Z2mLight> {
    vec![
        Z2mLight::parse(&format!("{LEFT}=left")).unwrap(),
        Z2mLight::parse(&format!("{RIGHT}=right")).unwrap(),
    ]
}

fn link(data_dir: &Path, broker: &Broker) {
    let link = Z2mLink::new(
        "127.0.0.1",
        broker.addr.port(),
        Some(USER),
        Some(PASSWORD),
        "zigbee2mqtt",
        20,
        lights(),
    )
    .unwrap();
    ambient_z2m_store::save(data_dir, &link).unwrap();
}

struct Setup {
    rig: Rig,
    luna: FakeLuna,
}

fn setup(broker: &Broker, mode: &str) -> Setup {
    setup_timed(broker, mode, &json!({}))
}

/// Short waits instead of the real ones (30 s settle, 5 min stale, 60 s pause, 2 s cut gap, 20 s recovery).
fn quick(recover_ms: u64) -> Value {
    json!({"ambient": {"z2m": {
        "otaSettleMs": 700, "otaStaleMs": 600_000, "busyPauseMs": 1500, "busyCutGapMs": 150, "recoverMs": recover_ms,
    }}})
}

/// What the topic `ambient` says is wrong, or nothing.
fn problem(rig: &Rig) -> String {
    rig.topic("ambient")
        .and_then(|v| v["problem"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn problem_has(rig: &Rig, words: &str) -> bool {
    rig.wait_topic("ambient", WAIT, |v| {
        v["problem"].as_str().is_some_and(|p| p.contains(words))
    })
    .is_some()
}

fn setup_timed(broker: &Broker, mode: &str, extra: &Value) -> Setup {
    let mut rig = Rig::new();
    seed_settings(
        &rig.paths,
        "base.ambient",
        json!({
            "enabled": {"type": "bool", "value": true},
            "mode": {"type": "choice", "value": mode},
            "brightness": {"type": "integer", "value": 100},
            "delay_ms": {"type": "integer", "value": 0},
        }),
    );
    link(&rig.paths.data_dir, broker);
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::PollOnly, "Active");
    let mut config = json!({
        "screen": {"source": "luna", "pollMs": 40},
        "luna": {"send": luna.program},
    });
    config
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    rig.start(test_config(config), Script::new());
    Setup { rig, luna }
}

fn z2m_state(rig: &Rig, state: &str) -> bool {
    rig.wait_topic("ambient", WAIT, |v| v["z2mState"] == state)
        .is_some()
}

/// A frame as published: (counter, smoothing, records as 7-byte chunks), from a `zclcommand` command 1 payload.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SentFrame {
    counter: u32,
    smoothing: u16,
    records: Vec<Vec<u8>>,
}

fn zcl_data(p: &Value, command: u64) -> Option<Vec<u8>> {
    let z = p.get("zclcommand")?;
    if z["command"] != command {
        return None;
    }
    Some(
        z["payload"]["data"]
            .as_array()?
            .iter()
            .map(|b| u8::try_from(b.as_u64().unwrap()).unwrap())
            .collect(),
    )
}

fn frames_to(broker: &Broker, name: &str) -> Vec<SentFrame> {
    broker
        .sets(name)
        .iter()
        .filter_map(|p| zcl_data(p, 1))
        .map(|d| SentFrame {
            counter: u32::from_le_bytes([d[0], d[1], d[2], d[3]]),
            smoothing: u16::from_le_bytes([d[4], d[5]]),
            records: d[6..].chunks(7).map(<[u8]>::to_vec).collect(),
        })
        .collect()
}

fn resets_to(broker: &Broker, name: &str) -> Vec<u32> {
    broker
        .sets(name)
        .iter()
        .filter_map(|p| zcl_data(p, 3))
        .map(|d| u32::from_le_bytes([d[2], d[3], d[4], d[5]]))
        .collect()
}

fn record_of(addr: u16, rgb: [u16; 3]) -> Vec<u8> {
    fr::record(addr, rgb, Mode::Device).pack().to_vec()
}

/// The x of a packed record's colour (the inverse of the 12-bit quantisation).
fn x_of(record: &[u8]) -> f64 {
    let x12 = u16::from(record[4]) | (u16::from(record[5] & 0x0F) << 8);
    f64::from(x12) * 0.7347 / 4095.0
}

fn nothing_secret(text: &str) {
    assert!(!text.contains(PASSWORD), "the password was printed: {text}");
}

fn log_text(_rig: &Rig) -> String {
    lumatv_helper::logging::tail("lumatv-helper", 100).join("\n")
}

// ------------------------------------------------------------------------------------------------- the tests

#[test]
fn the_still_colour_starts_the_bulbs_with_bifrosts_frames_and_the_screen_going_off_puts_them_back()
{
    let broker = Broker::start();
    two_bulbs(&broker);
    let s = setup(&broker, "colour");
    let rig = &s.rig;

    assert!(z2m_state(rig, "streaming"), "{:?}", rig.topic("ambient"));
    let status = rig.topic("ambient").unwrap();
    assert_eq!(
        (
            status["z2m"].clone(),
            status["hue"].clone(),
            status["available"].clone(),
            status["hueState"].clone()
        ),
        (json!(true), json!(false), json!(true), json!("off"))
    );
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 3));

    // The connection (a look at the lights before the stream may have had one of its own, the same way): the saved credentials, a
    // clean client id, the device list and both lights' states subscribed to.
    let connects = broker.connects();
    for (id, user, password) in &connects {
        assert_eq!(
            (user.as_deref(), password.as_deref()),
            (Some(USER), Some(PASSWORD))
        );
        assert!(id.starts_with("lumatv-"), "{connects:?}");
    }
    let subscribed = broker.subscribed();
    for t in [
        "zigbee2mqtt/bridge/devices",
        "zigbee2mqtt/TV left",
        "zigbee2mqtt/TV right",
    ] {
        assert!(subscribed.iter().any(|s| s == t), "{subscribed:?}");
    }

    // The start: light balance to each, then command 3 with counter 0 to each, then frames to the first light only.
    let left = broker.sets(LEFT);
    let right = broker.sets(RIGHT);
    assert_eq!(left[0], fr::light_balance(0xFE), "{left:?}");
    assert_eq!(right[0], fr::light_balance(0xFE), "{right:?}");
    assert_eq!(left[1], fr::zclcommand(3, &fr::reset(0)));
    assert_eq!(right[1], fr::zclcommand(3, &fr::reset(0)));
    assert!(
        right.iter().all(|p| zcl_data(p, 1).is_none()),
        "frames went to the second bulb: {right:?}"
    );
    assert!(zcl_data(&left[2], 1).is_some(), "{left:?}");
    // Nothing else went anywhere: no plain light command, no other topic.
    for p in broker.published() {
        assert!(
            p.topic.ends_with("/set") && (p.topic.contains(LEFT) || p.topic.contains(RIGHT)),
            "{p:?}"
        );
    }

    // The frames: Bifrost's bytes for the warm still colour on both bulbs, the smoothing of 20 fps, counters running from 0.
    let still = Still::Warm.rgb().to_u16();
    let frames = frames_to(&broker, LEFT);
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.counter, u32::try_from(i).unwrap(), "{frames:?}");
        assert_eq!(f.smoothing, 1280);
        assert_eq!(
            f.records,
            vec![record_of(LEFT_ADDR, still), record_of(RIGHT_ADDR, still)],
            "{f:?}"
        );
    }
    // The whole payload, not just the data: Bifrost's zclcommand shape.
    let expected = fr::zclcommand(
        1,
        &fr::frame(
            0,
            1280,
            &[
                fr::record(LEFT_ADDR, still, Mode::Device),
                fr::record(RIGHT_ADDR, still, Mode::Device),
            ],
        ),
    );
    assert_eq!(left[2], expected);
    // A still colour is sent again about once a second (the keepalive), not at the frame rate and not twice a second.
    let before = frames_to(&broker, LEFT).len();
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= before + 2));
    let gap = frame_gap(&broker, LEFT, before);
    assert!(
        gap >= Duration::from_millis(700),
        "{gap:?} between two frames of a still colour"
    );

    // Screen off: black twice, command 3 to each, then each bulb back to how it was.
    s.luna.set_state("Active Standby");
    assert!(
        wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 2),
        "{:?}",
        broker.sets(RIGHT)
    );
    assert!(z2m_state(rig, "idle"), "{:?}", rig.topic("ambient"));
    let frames = frames_to(&broker, LEFT);
    let black = [record_of(LEFT_ADDR, [0; 3]), record_of(RIGHT_ADDR, [0; 3])];
    assert_eq!(frames[frames.len() - 1].records, black, "{frames:?}");
    assert_eq!(frames[frames.len() - 2].records, black, "{frames:?}");
    let sent = u32::try_from(frames.len()).unwrap();
    assert_eq!(resets_to(&broker, LEFT), vec![0, sent]);
    let left = broker.sets(LEFT);
    let right = broker.sets(RIGHT);
    assert_eq!(left.last(), Some(&json!({"state": "OFF"})), "{left:?}");
    assert_eq!(
        right.last(),
        Some(&json!({"state": "ON", "brightness": 120, "color_temp": 366})),
        "{right:?}"
    );
    // The restore comes after the command 3 of the stop.
    let reset_at = left.iter().rposition(|p| zcl_data(p, 3).is_some()).unwrap();
    assert_eq!(reset_at, left.len() - 2, "{left:?}");

    // Screen on again: the start sequence again, the counter carried on.
    s.luna.set_state("Active");
    assert!(
        wait_until(WAIT, || resets_to(&broker, LEFT).len() == 3),
        "{:?}",
        broker.sets(LEFT)
    );
    assert!(z2m_state(rig, "streaming"));
    assert_eq!(resets_to(&broker, LEFT)[2], sent);
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len()
        > usize::try_from(sent).unwrap()));
    assert_eq!(
        frames_to(&broker, LEFT)[usize::try_from(sent).unwrap()].counter,
        sent
    );

    // The password is in no topic and no log line.
    nothing_secret(&rig.topic("ambient").unwrap().to_string());
    nothing_secret(&log_text(rig));

    // The helper ending ends the stream too: command 3 once more and the bulbs put back.
    let mut s = s;
    s.rig.stop();
    // (the helper has written them by now; the broker's own thread reads them when it gets to it)
    assert!(
        wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 4),
        "{:?}",
        broker.sets(RIGHT)
    );
    assert!(
        wait_until(WAIT, || broker.sets(RIGHT).last()
            == Some(
                &json!({"state": "ON", "brightness": 120, "color_temp": 366})
            )),
        "{:?}",
        broker.sets(RIGHT)
    );
}

/// The capture helper's side of the socket: frames of one colour on the left of the picture and another on the right.
/// How long after frame `n` to `name` the next frame came in.
fn frame_gap(broker: &Broker, name: &str, n: usize) -> Duration {
    let topic = format!("zigbee2mqtt/{name}/set");
    let frames: Vec<Instant> = broker
        .published()
        .into_iter()
        .filter(|p| p.topic == topic && zcl_data(&p.payload, 1).is_some())
        .map(|p| p.at)
        .collect();
    frames[n + 1].duration_since(frames[n])
}

fn left_red_right_blue() -> ZoneFrame {
    let mut f = ZoneFrame::blank(Preset::Plain);
    for (i, rect) in Preset::Plain.rects().iter().enumerate().take(MAX_ZONES) {
        f.color[i] = if (rect.x0 + rect.x1) / 2.0 < 0.5 {
            Rgb::new(0.9, 0.05, 0.05)
        } else {
            Rgb::new(0.05, 0.05, 0.9)
        };
        f.count[i] = 1000;
    }
    f
}

#[test]
fn the_pictures_colours_reach_each_bulb_by_where_it_sits_at_the_frame_rate() {
    let broker = Broker::start();
    two_bulbs(&broker);
    let s = setup(&broker, "picture");
    let socket = s.rig.paths.data_dir.join("ambient.sock");
    assert!(wait_until(WAIT, || socket.exists()), "no capture socket");
    let mut capture = UnixStream::connect(&socket).unwrap();
    let bytes = wire::encode(&left_red_right_blue(), 1000);

    let deadline = Instant::now() + WAIT;
    let mut seen = None;
    let started = Instant::now();
    let mut pushed = 0;
    while Instant::now() < deadline && seen.is_none() {
        capture.write_all(&bytes).unwrap();
        pushed += 1;
        thread::sleep(Duration::from_millis(30));
        seen = frames_to(&broker, LEFT).into_iter().rev().find(|f| {
            f.records.len() == 2 && (x_of(&f.records[0]) > 0.5 || x_of(&f.records[1]) > 0.5)
        });
    }
    let f = seen.unwrap_or_else(|| {
        panic!(
            "no picture on the bulbs after {pushed} frames: {:?}",
            frames_to(&broker, LEFT).last()
        )
    });
    let (left, right) = (x_of(&f.records[0]), x_of(&f.records[1]));
    assert!(left > 0.5, "the left bulb is not red: x {left} {right}");
    assert!(right < 0.3, "the right bulb is not blue: x {left} {right}");
    assert_eq!(&f.records[0][..2], &LEFT_ADDR.to_le_bytes());
    assert_eq!(&f.records[1][..2], &RIGHT_ADDR.to_le_bytes());
    assert!(z2m_state(&s.rig, "streaming"));

    // The pace is the link's 20 a second, not the engine's 30: keep feeding until twenty more frames have come and read their pace off the
    // broker's clock. A slow machine only lowers it, so there is an upper bound and no lower one.
    let before = frames_to(&broker, LEFT).len();
    let topic = format!("zigbee2mqtt/{LEFT}/set");
    let arrivals = || -> Vec<Instant> {
        broker
            .published()
            .into_iter()
            .filter(|p| p.topic == topic && zcl_data(&p.payload, 1).is_some())
            .map(|p| p.at)
            .skip(before)
            .collect()
    };
    assert!(
        wait_until(WAIT, || {
            let _ = capture.write_all(&bytes);
            thread::sleep(Duration::from_millis(30));
            arrivals().len() >= 20
        }),
        "{} frames after the picture arrived (started {:?} ago)",
        arrivals().len(),
        started.elapsed()
    );
    let arrivals = arrivals();
    let per_second = 19.0 / arrivals[19].duration_since(arrivals[0]).as_secs_f64();
    assert!(
        per_second <= 26.0,
        "{per_second} frames a second at the link's 20"
    );

    // The picture goes: the lights fade, black is sent, the stream ends with command 3 and the restore.
    assert!(
        wait_until(Duration::from_secs(30), || resets_to(&broker, RIGHT).len()
            == 2),
        "{:?}",
        broker.sets(RIGHT)
    );
    assert!(z2m_state(&s.rig, "idle"));
    let frames = frames_to(&broker, LEFT);
    assert_eq!(
        frames.last().unwrap().records,
        [record_of(LEFT_ADDR, [0; 3]), record_of(RIGHT_ADDR, [0; 3])]
    );
}

#[test]
fn a_bulb_zigbee2mqtt_does_not_know_is_an_error_until_the_list_names_it() {
    let broker = Broker::start();
    broker.retain("zigbee2mqtt/bridge/devices", &devices(&[(LEFT, LEFT_ADDR)]));
    let s = setup(&broker, "colour");
    let status = s
        .rig
        .wait_topic("ambient", WAIT, |v| v["z2mState"] == "error")
        .expect("no complaint");
    let problem = status["problem"].as_str().unwrap();
    assert!(
        problem.starts_with("Zigbee2MQTT:")
            && problem.contains("no device named \"TV right\"")
            && problem.contains("its Hue lights are: TV left"),
        "{status}"
    );
    // Nothing was sent to any light.
    thread::sleep(Duration::from_millis(300));
    assert!(broker.published().is_empty(), "{:?}", broker.published());
    nothing_secret(&status.to_string());

    // The bulb joins: the next try (the pause is 5 s) reads the new list and streams.
    two_bulbs(&broker);
    assert!(
        z2m_state(&s.rig, "streaming"),
        "{:?}",
        s.rig.topic("ambient")
    );
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 2));
    assert_eq!(
        &frames_to(&broker, LEFT)[0].records[1][..2],
        &RIGHT_ADDR.to_le_bytes()
    );
}

#[test]
fn a_bulb_that_rejoined_with_a_new_address_gets_it_in_the_next_frames() {
    let broker = Broker::start();
    two_bulbs(&broker);
    let s = setup(&broker, "colour");
    assert!(z2m_state(&s.rig, "streaming"));
    // a look at the lights before the stream may have had a connection of its own: the stream's is counted from here
    let connects = broker.connects().len();
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 2));
    broker.retain(
        "zigbee2mqtt/bridge/devices",
        &devices(&[(LEFT, LEFT_ADDR), (RIGHT, 0xA001)]),
    );
    assert!(
        wait_until(Duration::from_secs(5), || frames_to(&broker, LEFT)
            .last()
            .is_some_and(|f| f.records[1][..2] == 0xA001u16.to_le_bytes())),
        "{:?}",
        frames_to(&broker, LEFT).last()
    );
    // The same connection, no new start.
    assert_eq!(broker.connects().len(), connects);
    assert_eq!(resets_to(&broker, LEFT).len(), 1);
    assert!(z2m_state(&s.rig, "streaming"));
}

#[test]
fn a_broker_that_refuses_the_password_is_reported_in_words_that_do_not_carry_it() {
    let broker = Broker::start();
    two_bulbs(&broker);
    broker.refuse(4);
    let s = setup(&broker, "colour");
    let status = s
        .rig
        .wait_topic("ambient", WAIT, |v| v["z2mState"] == "error")
        .expect("no complaint");
    let problem = status["problem"].as_str().unwrap();
    assert!(problem.contains("bad user name or password"), "{status}");
    nothing_secret(&status.to_string());
    nothing_secret(&log_text(&s.rig));
    assert!(broker.published().is_empty());
    assert!(broker.subscribed().is_empty());
}

#[test]
fn a_dropped_connection_is_reported_and_the_stream_started_again() {
    let broker = Broker::start();
    two_bulbs(&broker);
    let s = setup(&broker, "colour");
    assert!(z2m_state(&s.rig, "streaming"));
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 2));
    broker.kill();
    let status = s
        .rig
        .wait_topic("ambient", WAIT, |v| v["z2mState"] == "error")
        .expect("the drop was not noticed");
    assert!(
        status["problem"]
            .as_str()
            .unwrap()
            .contains("the stream ended"),
        "{status}"
    );
    // After the pause: a new connection, the start sequence again, the counter carried on.
    assert!(
        z2m_state(&s.rig, "streaming"),
        "{:?}",
        s.rig.topic("ambient")
    );
    assert!(wait_until(WAIT, || broker.connects().len() == 2
        && resets_to(&broker, LEFT).len() == 2));
    // The counters only ever go up (a frame written into the dying connection may be lost with its number), and the second
    // start told the bulbs the number its first frame then carried.
    let restart = resets_to(&broker, LEFT)[1];
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT)
        .iter()
        .any(|f| f.counter == restart)));
    let frames = frames_to(&broker, LEFT);
    let counters: Vec<u32> = frames.iter().map(|f| f.counter).collect();
    assert!(counters.windows(2).all(|w| w[1] > w[0]), "{counters:?}");
    assert!(
        counters.iter().filter(|c| **c < restart).count() >= 2,
        "{counters:?} restart {restart}"
    );
}

// ------------------------------------------------------------------------------------------------- the guard

const GONE: &str = "zigbee2mqtt/leak";

fn updating(progress: f64) -> Value {
    json!({"battery": 90, "update": {"state": "updating", "progress": progress}})
}

#[test]
fn an_update_running_before_the_start_holds_every_frame_back_until_it_has_ended_and_settled() {
    let broker = Broker::start();
    two_bulbs(&broker);
    broker.retain(GONE, &updating(54.0));
    let s = setup_timed(&broker, "colour", &quick(20_000));

    let held = s
        .rig
        .wait_topic("ambient", WAIT, |v| {
            v["problem"]
                .as_str()
                .is_some_and(|p| p.contains("paused: Zigbee2MQTT is updating leak"))
        })
        .expect("no word of the update");
    let connects = broker.connects().len();
    assert_eq!(held["z2mState"], "idle");
    assert!(
        held["problem"]
            .as_str()
            .unwrap()
            .starts_with("Zigbee2MQTT: "),
        "{held}"
    );
    thread::sleep(Duration::from_millis(1500));
    assert!(
        broker.published().is_empty(),
        "sent during an update: {:?}",
        broker.published()
    );

    // The update ends: still nothing for the settle time, then the start, on the same connection.
    let ended = Instant::now();
    broker.push(GONE, &json!({"update": {"state": "idle"}}));
    assert!(
        z2m_state(&s.rig, "streaming"),
        "{:?}",
        s.rig.topic("ambient")
    );
    // the status says streaming as the first command goes out; the broker has it a moment later
    assert!(wait_until(WAIT, || !broker.published().is_empty()));
    let first = broker.published()[0].at;
    assert!(
        first.duration_since(ended) >= Duration::from_millis(700),
        "sent {:?} after the update ended: the settle time is 700 ms",
        first.duration_since(ended)
    );
    assert_eq!(problem(&s.rig), "");
    assert_eq!(broker.connects().len(), connects);
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 2));
    let left = broker.sets(LEFT);
    assert_eq!(left[0], fr::light_balance(0xFE), "{left:?}");
}

#[test]
fn an_update_starting_mid_stream_ends_it_and_the_lights_follow_again_when_it_is_over() {
    let broker = Broker::start();
    two_bulbs(&broker);
    let s = setup_timed(&broker, "colour", &quick(20_000));
    assert!(z2m_state(&s.rig, "streaming"));
    let connects = broker.connects().len();
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 2));
    let restored = json!({"state": "ON", "brightness": 120, "color_temp": 366});

    // Zigbee2MQTT starts updating a leak sensor: the stream ends the normal way and the words say why.
    broker.push(GONE, &updating(1.0));
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 2));
    assert!(problem_has(&s.rig, "paused: Zigbee2MQTT is updating leak"));
    assert_eq!(s.rig.topic("ambient").unwrap()["z2mState"], "idle");
    assert!(wait_until(WAIT, || broker.sets(RIGHT).last() == Some(&restored)));
    assert!(wait_until(WAIT, || broker.sets(LEFT).last()
        == Some(&json!({"state": "OFF"}))));
    let sent = broker.published().len();
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(broker.published().len(), sent, "frames during an update");
    // Progress messages keep it going; a different device's plain state changes nothing.
    broker.push(GONE, &updating(30.0));
    broker.push("zigbee2mqtt/plug", &json!({"state": "ON"}));

    // It ends: after the settle time the start sequence again on the same connection, the counter carried on.
    broker.push(GONE, &json!({"update": {"state": "idle"}}));
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 3));
    assert!(z2m_state(&s.rig, "streaming"));
    assert_eq!(problem(&s.rig), "");
    assert_eq!(broker.connects().len(), connects);
    let restart = resets_to(&broker, LEFT)[2];
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT)
        .iter()
        .any(|f| f.counter == restart)));

    // The same by Zigbee2MQTT's request and response topics.
    broker.push(
        "zigbee2mqtt/bridge/request/device/ota_update/update",
        &json!({"id": "leak"}),
    );
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 4));
    assert!(problem_has(&s.rig, "paused: Zigbee2MQTT is updating leak"));
    broker.push(
        "zigbee2mqtt/bridge/response/device/ota_update/update",
        &json!({"data": {"id": "leak", "from": {}, "to": {}}, "status": "ok"}),
    );
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 5));
    assert!(z2m_state(&s.rig, "streaming"));

    // And by its log, which also carries an update's progress.
    broker.push(
        "zigbee2mqtt/bridge/logging",
        &json!({"level": "info", "message": "Update of 'leak' at 12.50%, ≈ 9 minutes remaining"}),
    );
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 6));
    assert!(problem_has(&s.rig, "paused: Zigbee2MQTT is updating leak"));
    broker.push(
        "zigbee2mqtt/bridge/logging",
        &json!({"level": "info", "message": "Finished update of 'leak'"}),
    );
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 7));
    assert!(z2m_state(&s.rig, "streaming"));
    assert_eq!(broker.connects().len(), connects);
}

#[test]
fn an_update_that_never_ends_stops_holding_the_lights_back_after_the_stale_time() {
    let broker = Broker::start();
    two_bulbs(&broker);
    broker.retain(GONE, &updating(10.0));
    let mut timing = quick(20_000);
    timing["ambient"]["z2m"]["otaStaleMs"] = json!(4000);
    let s = setup_timed(&broker, "colour", &timing);
    assert!(problem_has(&s.rig, "paused: Zigbee2MQTT is updating leak"));
    assert!(broker.published().is_empty());
    // Not heard of again: taken as over after 4 s, then the settle time, then the lights.
    assert!(z2m_state(&s.rig, "streaming"));
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 2));
}

/// Pictures on the capture socket until it is dropped: the stream's pace is then the frame rate and not the keepalive.
struct Feeder {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Feeder {
    fn start(socket: &Path) -> Feeder {
        assert!(wait_until(WAIT, || socket.exists()), "no capture socket");
        let mut capture = UnixStream::connect(socket).unwrap();
        let bytes = wire::encode(&left_red_right_blue(), 1000);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                if capture.write_all(&bytes).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(30));
            }
        });
        Feeder {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Feeder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn log_line(level: &str, message: &str) -> Value {
    json!({"level": level, "message": message})
}

/// What Zigbee2MQTT logs when the coordinator did not answer a frame in time.
fn late(name: &str) -> Value {
    log_line(
        "error",
        &format!(
            "Publish 'set' 'zclcommand' to '{name}' failed: 'Error: SRSP - AF - dataRequest after 6000ms'"
        ),
    )
}

#[test]
fn late_answers_halve_the_rate_then_pause_the_stream_and_the_pause_ends_on_its_own() {
    let broker = Broker::start();
    two_bulbs(&broker);
    // No recovery inside the test: what each step reads is what the last late answer left, however long the machine takes between them.
    let s = setup_timed(&broker, "picture", &quick(600_000));
    let _feed = Feeder::start(&s.rig.paths.data_dir.join("ambient.sock"));
    assert!(z2m_state(&s.rig, "streaming"));
    let connects = broker.connects().len();
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 5));
    assert_eq!(problem(&s.rig), "");
    let log = "zigbee2mqtt/bridge/logging";

    // A late answer: half the rate, shown in words, and the fade of a frame follows the interval. (Late answers in one moment are
    // one event: the worker's cut gap, `a_late_answer_within_the_cut_gap_is_the_same_event`.)
    broker.push(log, &late(LEFT));
    assert!(problem_has(&s.rig, "10 frames a second instead of 20"));
    assert_eq!(s.rig.topic("ambient").unwrap()["z2mState"], "streaming");
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT)
        .last()
        .is_some_and(
            |f| f.smoothing == fr::smoothing_for(Duration::from_millis(100))
        )));

    // Another one, past the cut gap: the lowest rate, whose frames then fade over its 200 ms interval.
    thread::sleep(Duration::from_millis(300));
    broker.push(log, &late(LEFT));
    assert!(problem_has(&s.rig, "5 frames a second instead of 20"));
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT)
        .last()
        .is_some_and(
            |f| f.smoothing == fr::smoothing_for(Duration::from_millis(200))
        )));

    // Not the lights' trouble, or not an error: no change.
    broker.push(
        log,
        &log_line(
            "error",
            "Publish 'set' 'state' to 'plug' failed: 'Error: no ack'",
        ),
    );
    broker.push(
        log,
        &log_line("info", &format!("Publish 'set' to '{LEFT}' failed")),
    );
    thread::sleep(Duration::from_millis(300));
    assert!(
        problem(&s.rig).contains("5 frames a second"),
        "{}",
        problem(&s.rig)
    );
    assert_eq!(resets_to(&broker, RIGHT).len(), 1);

    // Still late at the lowest rate: the stream ends the normal way and stays off for a while.
    broker.push(log, &late(LEFT));
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 2));
    assert!(problem_has(
        &s.rig,
        "paused: the Zigbee network was too busy"
    ));
    assert_eq!(s.rig.topic("ambient").unwrap()["z2mState"], "idle");
    assert_eq!(
        broker.sets(RIGHT).last(),
        Some(&json!({"state": "ON", "brightness": 120, "color_temp": 366}))
    );

    // Then it starts again, at the lower rate, on the same connection, and no frame went out between the stop and the start.
    assert!(wait_until(WAIT, || resets_to(&broker, RIGHT).len() == 3));
    assert!(z2m_state(&s.rig, "streaming"));
    assert!(
        problem(&s.rig).contains("5 frames a second instead of 20"),
        "{}",
        problem(&s.rig)
    );
    assert_eq!(broker.connects().len(), connects);
    let left = broker.sets(LEFT);
    let commands: Vec<usize> = (0..left.len())
        .filter(|&i| zcl_data(&left[i], 3).is_some())
        .collect();
    assert!(
        left[commands[1]..commands[2]]
            .iter()
            .all(|p| zcl_data(p, 1).is_none()),
        "frames during the pause: {left:?}"
    );
}

#[test]
fn a_quiet_stream_recovers_its_rate_by_half_again_each_time() {
    let broker = Broker::start();
    two_bulbs(&broker);
    let s = setup_timed(&broker, "picture", &quick(4000));
    let _feed = Feeder::start(&s.rig.paths.data_dir.join("ambient.sock"));
    assert!(z2m_state(&s.rig, "streaming"));
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT).len() >= 5));
    let log = "zigbee2mqtt/bridge/logging";
    broker.push(log, &late(LEFT));
    assert!(problem_has(&s.rig, "10 frames a second instead of 20"));
    thread::sleep(Duration::from_millis(300));
    broker.push(log, &late(LEFT));
    assert!(problem_has(&s.rig, "5 frames a second instead of 20"));

    // Quiet: the rate climbs back by half again each time, to the link's 20, and the status goes clean.
    assert!(
        s.rig
            .wait_topic("ambient", Duration::from_secs(40), |v| v["problem"]
                .is_null())
            .is_some(),
        "{}",
        problem(&s.rig)
    );
    assert!(wait_until(WAIT, || frames_to(&broker, LEFT)
        .last()
        .is_some_and(|f| f.smoothing == 1280)));
}

// ------------------------------------------------------------------------------------------------- the command line

fn cli(dir: &Path, args: &[&str], input: &str) -> (i32, Value, String) {
    let mut child = Command::new(BIN)
        .args(args)
        .arg("--data-dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    (out.status.code().unwrap(), value, text)
}

#[test]
fn z2m_link_checks_the_lights_against_the_device_list_and_saves_the_password_privately() {
    let broker = Broker::start();
    two_bulbs(&broker);
    let dir = tempfile::tempdir().unwrap();
    let port = broker.addr.port().to_string();
    let left = format!("{LEFT}=left");
    let right = format!("{RIGHT}=right");

    // A light Zigbee2MQTT does not have: refused with the Hue lights it does have, nothing saved.
    let (code, said, text) = cli(
        dir.path(),
        &[
            "z2m-link",
            "--host",
            "127.0.0.1",
            "--port",
            &port,
            "--user",
            USER,
            "--light",
            &left,
            "--light",
            "TV back=behind",
        ],
        PASSWORD,
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        said["error"]
            .as_str()
            .unwrap()
            .contains("its Hue lights are: TV left, TV right"),
        "{said}"
    );
    nothing_secret(&text);
    assert!(ambient_z2m_store::load(dir.path()).unwrap().is_none());

    // The right lights: looked up (the credentials used), saved with mode 0600, nothing sent to a light.
    let (code, said, text) = cli(
        dir.path(),
        &[
            "z2m-link",
            "--host",
            "127.0.0.1",
            "--port",
            &port,
            "--user",
            USER,
            "--fps",
            "10",
            "--light",
            &left,
            "--light",
            &right,
        ],
        &format!("{PASSWORD}\n"),
    );
    assert_eq!(code, 0, "{text}");
    nothing_secret(&text);
    assert_eq!(said["linked"], true);
    assert_eq!(said["lights"][0]["name"], LEFT);
    assert_eq!(said["lights"][0]["networkAddress"], "0xd297");
    assert_eq!(said["lights"][1]["networkAddress"], "0xd298");
    assert_eq!(said["framesTo"], LEFT);
    assert_eq!(said["fps"], 10);
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(ambient_z2m_store::file(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode, 0o600);
    let saved = ambient_z2m_store::load(dir.path()).unwrap().unwrap();
    assert_eq!(saved.password.as_ref().map(|p| p.expose()), Some(PASSWORD));
    assert_eq!(
        (saved.username.as_deref(), saved.fps, saved.lights.len()),
        (Some(USER), 10, 2)
    );
    let connects = broker.connects();
    assert_eq!(connects.len(), 2);
    assert!(
        connects
            .iter()
            .all(|c| c.1.as_deref() == Some(USER) && c.2.as_deref() == Some(PASSWORD))
    );
    assert!(broker.published().is_empty(), "{:?}", broker.published());

    // A wrong password is the broker's words, not ours, and no file.
    let other = tempfile::tempdir().unwrap();
    broker.refuse(5);
    let (code, said, text) = cli(
        other.path(),
        &[
            "z2m-link",
            "--host",
            "127.0.0.1",
            "--port",
            &port,
            "--user",
            USER,
            "--light",
            &left,
        ],
        "wrong",
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        said["error"].as_str().unwrap().contains("not authorised"),
        "{said}"
    );
    assert!(ambient_z2m_store::load(other.path()).unwrap().is_none());

    // Unlink takes it away.
    let (code, said, _) = cli(dir.path(), &["z2m-unlink"], "");
    assert_eq!((code, said["removed"].clone()), (0, json!(true)));
    assert!(ambient_z2m_store::load(dir.path()).unwrap().is_none());

    // The usage errors: no light, a bad place, a public address.
    let (code, _, _) = cli(
        dir.path(),
        &["z2m-link", "--host", "127.0.0.1", "--port", &port],
        "",
    );
    assert_eq!(code, 2);
    let (code, said, _) = cli(
        dir.path(),
        &[
            "z2m-link",
            "--host",
            "127.0.0.1",
            "--port",
            &port,
            "--light",
            "x=nowhere",
        ],
        "",
    );
    assert_eq!(code, 2);
    assert!(
        said["error"].as_str().unwrap().contains("left, right, top"),
        "{said}"
    );
    let (code, _, _) = cli(
        dir.path(),
        &["z2m-link", "--host", "8.8.8.8", "--light", &left],
        "",
    );
    assert_eq!(code, 2);
}
