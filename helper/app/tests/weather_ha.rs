//! Home's weather from a Home Assistant weather entity, end to end: the real service, the saved Home Assistant link and a stub Home
//! Assistant that speaks the few WebSocket commands the source uses over real TCP.
//!
//! What these hold the source to: the entity's documents are served in place of the built-in ones and come back with the setting, and
//! with the Home Assistant plugin's switch; switched off, the built-in source's are served whatever the setting says; a failed read keeps
//! the last good document, stale, with the error, and never reaches for the built-in source; the Settings picker's list is the instance's
//! weather entities by name, and is refused while the plugin is off.

mod support;

use ha_link::ws;
use lumatv_feeds::cache::DocCache;
use lumatv_feeds::weather::{Units, WeatherConfig};
use lumatv_feeds::weather_ha;
use lumatv_helper::ha_plugin::OPT_IN;
use lumatv_helper::timeutil::now_ms;
use lumatv_helper::voice_store;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);
const MINUTE: u64 = 60_000;
const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.test-payload.test-signature";
const ENTITY: &str = "weather.pirate_weather";

// ------------------------------------------------------------------------------------------------------------- stub Home Assistant

struct Shared {
    token: String,
    /// What `get_states` lists and what `subscribe_entities` finds, by entity id: `{"s": state, "a": attributes}`.
    entities: Mutex<serde_json::Map<String, Value>>,
    /// Closes every connection as it is made: Home Assistant is down.
    down: AtomicBool,
    stop: AtomicBool,
    reads: AtomicUsize,
    /// The `service_data.type` of each forecast asked for.
    forecasts: Mutex<Vec<String>>,
}

struct StubHa {
    addr: SocketAddr,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

fn pirate_weather() -> Value {
    json!({"s": "rainy", "a": {"friendly_name": "Pirate Weather", "temperature": 16.0, "temperature_unit": "\u{b0}C", "apparent_temperature": 14.0, "humidity": 80,
        "wind_speed": 18.0, "wind_speed_unit": "km/h", "pressure": 1012.0, "pressure_unit": "hPa", "precipitation_unit": "mm", "supported_features": 3}})
}

fn forecast(kind: &str) -> Value {
    match kind {
        "daily" => json!([{"datetime": "2099-01-01T05:00:00+00:00", "condition": "rainy", "temperature": 18.0, "templow": 11.0, "precipitation_probability": 80},
                          {"datetime": "2099-01-02T05:00:00+00:00", "condition": "sunny", "temperature": 20.0, "templow": 9.0}]),
        _ => json!([{"datetime": "2099-01-01T12:00:00+00:00", "condition": "rainy", "temperature": 15.0, "precipitation_probability": 70, "wind_speed": 20.0}]),
    }
}

impl StubHa {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let mut entities = serde_json::Map::new();
        entities.insert(ENTITY.into(), pirate_weather());
        let shared = Arc::new(Shared {
            token: TOKEN.to_owned(),
            entities: Mutex::new(entities),
            down: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
            forecasts: Mutex::new(Vec::new()),
        });
        let accept = {
            let shared = shared.clone();
            thread::spawn(move || {
                let mut children = Vec::new();
                while !shared.stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) if shared.down.load(Ordering::SeqCst) => drop(stream),
                        Ok((stream, _)) => {
                            let shared = shared.clone();
                            children.push(thread::spawn(move || serve(stream, &shared)));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(5)),
                    }
                }
                for child in children {
                    let _ = child.join();
                }
            })
        };
        Self { addr, shared, thread: Some(accept) }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for StubHa {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn text_frame(value: &Value) -> Vec<u8> {
    let payload = value.to_string().into_bytes();
    let mut out = vec![0x81];
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= usize::from(u16::MAX) => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(&payload);
    out
}

/// One masked client frame from `buf`, removed from it.
fn take_frame(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = buf[0] & 0x0f;
    let (len, at) = match buf[1] & 0x7f {
        126 if buf.len() >= 4 => (usize::from(u16::from_be_bytes([buf[2], buf[3]])), 4),
        126 => return None,
        127 => panic!("a command this size is not expected"),
        n => (usize::from(n), 2),
    };
    if buf.len() < at + 4 + len {
        return None;
    }
    let mask: [u8; 4] = buf[at..at + 4].try_into().unwrap();
    let payload: Vec<u8> = buf[at + 4..at + 4 + len].iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect();
    buf.drain(..at + 4 + len);
    Some((opcode, payload))
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let mut buf = Vec::new();
    let key = loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let mut chunk = [0u8; 2048];
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            buf.drain(..end + 4);
            break head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).expect("a key").to_owned();
        }
    };
    let accept = ws::accept_key(&key);
    stream
        .write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes())
        .unwrap();
    let send = |stream: &mut TcpStream, value: Value| {
        let _ = stream.write_all(&text_frame(&value));
    };
    send(&mut stream, json!({"type": "auth_required", "ha_version": "2026.11.0"}));
    let mut authed = false;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        while let Some((opcode, payload)) = take_frame(&mut buf) {
            if opcode == 0x8 {
                return;
            }
            if opcode != 0x1 {
                continue;
            }
            let message: Value = serde_json::from_slice(&payload).unwrap();
            if !authed {
                if message["type"] == "auth" && message["access_token"] == json!(shared.token) {
                    authed = true;
                    send(&mut stream, json!({"type": "auth_ok", "ha_version": "2026.11.0"}));
                } else {
                    send(&mut stream, json!({"type": "auth_invalid", "message": "Invalid access token or password"}));
                    return;
                }
                continue;
            }
            let id = message["id"].clone();
            let result = |value: Value| json!({"id": id, "type": "result", "success": true, "result": value});
            let entities = shared.entities.lock().clone();
            match message["type"].as_str().unwrap_or("") {
                "subscribe_entities" => {
                    shared.reads.fetch_add(1, Ordering::SeqCst);
                    send(&mut stream, result(Value::Null));
                    let wanted: Vec<&str> = message["entity_ids"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
                    let found: serde_json::Map<String, Value> = entities.iter().filter(|(k, _)| wanted.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
                    send(&mut stream, json!({"id": id, "type": "event", "event": {"a": found}}));
                }
                "get_config" => send(&mut stream, result(json!({"latitude": 33.75, "longitude": -84.39, "time_zone": "America/New_York"}))),
                "call_service" => {
                    let kind = message["service_data"]["type"].as_str().unwrap_or("").to_owned();
                    assert_eq!((message["domain"].as_str(), message["service"].as_str(), message["return_response"].as_bool()), (Some("weather"), Some("get_forecasts"), Some(true)));
                    let entity = message["target"]["entity_id"].as_str().unwrap().to_owned();
                    shared.forecasts.lock().push(kind.clone());
                    send(&mut stream, result(json!({"context": {}, "response": {entity: {"forecast": forecast(&kind)}}})));
                }
                "get_states" => {
                    let list: Vec<Value> = entities.iter().map(|(k, v)| json!({"entity_id": k, "state": v["s"], "attributes": v["a"]})).collect();
                    send(&mut stream, result(Value::Array(list)));
                }
                other => send(&mut stream, json!({"id": id, "type": "result", "success": false, "error": {"code": "unknown_command", "message": format!("Unknown command {other}.")}})),
            }
        }
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
    }
}

// ------------------------------------------------------------------------------------------------------------------------- the rig

/// A service whose weather source is the stub's `ENTITY`, the Home Assistant link saved to `url`. It comes up with the Home Assistant
/// plugin switched on, as a TV whose weather came from Home Assistant before it was a plugin does.
fn rig(url: &str, before_start: impl FnOnce(&Rig)) -> Rig {
    let mut rig = Rig::new();
    seed_settings(
        &rig.paths,
        "base.weather",
        json!({"source": {"type": "choice", "value": "homeassistant"}, "entity": {"type": "text", "value": ENTITY}}),
    );
    voice_store::save(&rig.paths.data_dir, url, TOKEN).unwrap();
    before_start(&rig);
    rig.start_voice(test_config(json!({})), Script::internet());
    rig
}

fn has_doc(v: &Value) -> bool {
    !v["doc"].is_null()
}

/// A port nothing listens on.
fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// Switches the Home Assistant plugin as the TV's Settings does.
fn switch(rig: &Rig, on: bool) -> Reply {
    let body = if on { json!({"on": true, "approve": OPT_IN.approval()}) } else { json!({"on": false}) };
    http(
        rig.addr,
        "POST",
        "/v1/plugins/home-assistant/switch",
        &[("X-Luma-Client", "lumatv-ui"), ("Content-Type", "application/json")],
        Some(&body.to_string()),
    )
}

// -------------------------------------------------------------------------------------------------------------------------- tests

#[test]
fn the_entitys_documents_are_served_for_the_card_and_the_view_and_the_built_in_source_comes_back_with_the_setting() {
    let ha = StubHa::start();
    let rig = rig(&ha.url(), |_| {});
    let weather = rig.wait_topic("weather", WAIT, has_doc).expect("the entity's weather");
    assert_eq!(weather["doc"]["zip"], "ha:weather.pirate_weather");
    assert_eq!(weather["doc"]["display"]["place"], "Pirate Weather");
    assert_eq!(weather["doc"]["display"]["temp"], "61\u{b0}", "16 \u{b0}C in the card's Fahrenheit");
    assert_eq!(weather["doc"]["display"]["cond"], "Rain");
    assert_eq!(weather["doc"]["display"]["days"].as_array().unwrap().len(), 2);
    assert_eq!((weather["doc"]["lat"].as_f64(), weather["doc"]["lon"].as_f64()), (Some(33.75), Some(-84.39)), "the radar centres on home");
    assert_eq!(weather["state"], "fresh");
    // the view's details come from the same conversation
    let more = rig.wait_topic("weather.more", WAIT, |v| v["doc"]["hourly"].as_array().is_some_and(|h| !h.is_empty())).expect("the details");
    assert_eq!(more["doc"]["fetchedAt"], weather["doc"]["fetchedAt"]);
    assert_eq!(more["doc"]["hourly"][0]["rainPct"], 70.0);
    assert_eq!(more["doc"]["daily"][0]["rainPct"], 80.0);
    assert_eq!(ha.shared.reads.load(Ordering::SeqCst), 1, "one conversation made both documents");
    assert_eq!(*ha.shared.forecasts.lock(), ["daily", "hourly"]);
    let on_disk: Value = serde_json::from_slice(&std::fs::read(rig.paths.cache_dir().join("weather.json")).unwrap()).unwrap();
    assert_eq!(on_disk["zip"], "ha:weather.pirate_weather");

    let builtin = r#"{"changes":[{"key":"source","value":{"type":"choice","value":"builtin"}}]}"#;
    assert_eq!(post(rig.addr, "/v1/settings/base.weather", builtin).status, 200);
    let back = rig.wait_topic("weather", WAIT, |v| v["doc"]["zip"] == "30311").expect("the built-in reading");
    assert_eq!(back["doc"]["place"], "Atlanta");
}

#[test]
fn a_failed_read_keeps_the_last_good_document_stale_with_the_error_and_does_not_fall_back() {
    let ha = StubHa::start();
    let rig = rig(&ha.url(), |_| {});
    let good = rig.wait_topic("weather", WAIT, has_doc).expect("the entity's weather");
    ha.shared.down.store(true, Ordering::SeqCst);
    assert_eq!(post(rig.addr, "/v1/refresh", "{}").status, 200);
    let failed = rig.wait_topic("weather", WAIT, |v| v["status"]["failures"].as_u64().unwrap_or(0) >= 1).expect("the failure");
    assert_eq!(failed["doc"], good["doc"], "the last good reading, whole");
    assert_eq!(failed["status"]["error"]["code"], "fetch");
    assert!(!failed["status"]["error"]["message"].as_str().unwrap().is_empty());
    assert_eq!(failed["doc"]["zip"], "ha:weather.pirate_weather", "not the built-in source's");
}

#[test]
fn switched_off_the_built_in_source_serves_the_card_and_the_entity_comes_back_with_the_switch() {
    let ha = StubHa::start();
    let rig = rig(&ha.url(), |_| {});
    let entity = |v: &Value| v["doc"]["zip"] == "ha:weather.pirate_weather";
    rig.wait_topic("weather", WAIT, entity).expect("the entity's weather");
    assert_eq!(switch(&rig, false).status, 200);
    let builtin = rig.wait_topic("weather", WAIT, |v| v["doc"]["zip"] == "30311").expect("the built-in reading");
    assert_eq!(builtin["doc"]["place"], "Atlanta");
    assert_eq!(switch(&rig, true).status, 200);
    rig.wait_topic("weather", WAIT, entity).expect("the entity's weather again");
}

#[test]
fn a_saved_reading_is_served_stale_while_home_assistant_cannot_be_reached() {
    let now = now_ms();
    let rig = rig(&dead_url(), |rig| {
        let state = pirate_weather();
        let snapshot = json!({"entity": ENTITY, "available": true, "name": "Pirate Weather", "condition": state["s"], "reading": {"temperature": 16.0, "temperature_unit": "\u{b0}C"}, "hourly": [], "daily": []});
        let cfg = WeatherConfig { zip: format!("ha:{ENTITY}"), units: Units::Fahrenheit };
        let (doc, _) = weather_ha::documents(&snapshot, &cfg, now - 20 * MINUTE).unwrap();
        DocCache::new(rig.paths.cache_dir()).store_weather(&doc).unwrap();
    });
    let saved = rig.wait_topic("weather", WAIT, has_doc).expect("the saved reading");
    assert_eq!(saved["state"], "stale", "20 minutes old: shown, marked stale");
    assert_eq!(saved["doc"]["display"]["temp"], "61\u{b0}");
    let failed = rig.wait_topic("weather", WAIT, |v| v["status"]["failures"].as_u64().unwrap_or(0) >= 1).expect("the attempt that failed");
    assert_eq!(failed["state"], "stale");
    assert_eq!(failed["doc"]["fetchedAt"], saved["doc"]["fetchedAt"], "nothing replaced it");
    assert_eq!(failed["doc"]["place"], "Pirate Weather");
}

#[test]
fn the_picker_lists_the_weather_entities_by_name_and_says_why_it_cannot() {
    let ha = StubHa::start();
    {
        let mut entities = ha.shared.entities.lock();
        entities.insert("sensor.outside".into(), json!({"s": "3", "a": {"friendly_name": "Outside"}}));
        entities.insert("weather.home".into(), json!({"s": "sunny", "a": {"friendly_name": "Forecast Home"}}));
    }
    let mut rig = Rig::new();
    rig.start_voice(test_config(json!({})), Script::new());
    let off = get(rig.addr, "/v1/weather/entities");
    assert_eq!((off.status, off.json()["error"]["code"].clone()), (409, json!("off")));
    assert_eq!(switch(&rig, true).status, 200);
    let unlinked = get(rig.addr, "/v1/weather/entities");
    assert_eq!(unlinked.status, 409);
    assert_eq!(unlinked.json()["error"]["code"], "no_link");
    voice_store::save(&rig.paths.data_dir, &ha.url(), "not-the-token").unwrap();
    let refused = get(rig.addr, "/v1/weather/entities");
    assert_eq!((refused.status, refused.json()["error"]["code"].clone()), (502, json!("auth")));
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    let listed = get(rig.addr, "/v1/weather/entities");
    assert_eq!(listed.status, 200);
    assert_eq!(
        listed.json()["entities"],
        json!([{"id": "weather.home", "name": "Forecast Home"}, {"id": ENTITY, "name": "Pirate Weather"}])
    );
}
