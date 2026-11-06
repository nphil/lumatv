//! TV sensors: the TV's own state as sensors of its ESPHome device in Home Assistant (the device the Bluetooth proxy is, `ble`), a
//! plugin of the helper's own that asks first ([`OPT_IN`], `plugins/local.rs`).
//!
//! **Read only, and only while someone listens.** Nothing is read until the owner has switched the plugin on and Home Assistant has
//! subscribed to the device's states ([`Board::wanted`], which the device sets). Then:
//! - the power state and whether the screen is lit: the `screen` topic, at its change;
//! - the app in front, and the input it is: LG's application manager through a subscription, at its change; an input's name is the
//!   one LG's input list gives it (`eim/getAllInputStatus`), read when an input's app comes to the front;
//! - the picture mode: LG's settings service through a subscription, at its change;
//! - the Live TV Tuner's status at its change (its plugin's facts), and its signal while it streams (the demodulator's report);
//! - the SoC's temperature (`/proc/lg/pm/temperature`), the load (`/proc/loadavg`) and the memory available (`/proc/meminfo`), every
//!   30 s while the screen is on and every 60 s while it is off; the tuner's signal with them.
//!
//! A reading goes on the [`Board`]; the device sends Home Assistant a state only when it changed.

use crate::ble_proxy::{Entity, Kind, Reading};
use crate::ctx::Ctx;
use crate::plugins::local::{OptIn, Spec};
use crate::plugins::session::Shared;
use crate::screen;
use lumatuner::{Hardware, Status, Tuner};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const ID: &str = "tv-sensors";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "TV sensors",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Sends this TV's state to Home Assistant as sensors of its ESPHome device: power and screen, the app in front and its \
              input, the picture mode, the Live TV Tuner and its signal, the chip's temperature, the load and the free memory. Read \
              only: nothing on the TV can be changed through it. It uses the Bluetooth proxy's link (ble-link).",
};

const TEMPERATURE: u32 = 1;
const POWER: u32 = 2;
const SCREEN: u32 = 3;
const APP: u32 = 4;
const INPUT: u32 = 5;
const PICTURE: u32 = 6;
const LOAD: u32 = 7;
const MEMORY: u32 = 8;
const TUNER: u32 = 9;
const SIGNAL: u32 = 10;

/// What the device lists while the plugin is on.
pub fn entities() -> Vec<Entity> {
    let sensor = |key, object_id, name, icon, unit, device_class, decimals, diagnostic| Entity {
        key,
        object_id,
        name,
        icon,
        kind: Kind::Sensor {
            unit,
            device_class,
            decimals,
        },
        diagnostic,
    };
    let text = |key, object_id, name, icon| Entity {
        key,
        object_id,
        name,
        icon,
        kind: Kind::Text,
        diagnostic: false,
    };
    vec![
        sensor(
            TEMPERATURE,
            "soc_temperature",
            "SoC temperature",
            "",
            "°C",
            "temperature",
            0,
            false,
        ),
        text(POWER, "power_state", "Power state", "mdi:power"),
        Entity {
            key: SCREEN,
            object_id: "screen",
            name: "Screen",
            icon: "mdi:television",
            kind: Kind::BinarySensor { device_class: "" },
            diagnostic: false,
        },
        text(APP, "foreground_app", "Foreground app", "mdi:apps"),
        text(INPUT, "input", "Input", "mdi:video-input-hdmi"),
        text(PICTURE, "picture_mode", "Picture mode", "mdi:palette"),
        sensor(
            LOAD,
            "cpu_load",
            "CPU load",
            "mdi:cpu-64-bit",
            "",
            "",
            2,
            true,
        ),
        sensor(
            MEMORY,
            "free_memory",
            "Free memory",
            "mdi:memory",
            "MB",
            "data_size",
            0,
            true,
        ),
        text(TUNER, "live_tv_tuner", "Live TV Tuner", "mdi:antenna"),
        sensor(
            SIGNAL,
            "tuner_signal",
            "Tuner signal",
            "mdi:signal",
            "%",
            "",
            0,
            false,
        ),
    ]
}

/// The readings, put by the sampler and the subscriptions' readers and taken by the device, and whether the device has a listener.
#[derive(Default)]
pub struct Board {
    inner: Mutex<Inner>,
    wanted: AtomicBool,
}

#[derive(Default)]
struct Inner {
    readings: BTreeMap<u32, Reading>,
    changed: BTreeSet<u32>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Board {
    /// How the device is woken when a reading changed.
    pub fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.inner.lock().wake = Some(wake);
    }

    pub fn put(&self, key: u32, reading: Reading) {
        let wake = {
            let mut inner = self.inner.lock();
            if inner.readings.get(&key) == Some(&reading) {
                return;
            }
            inner.readings.insert(key, reading);
            inner.changed.insert(key);
            inner.wake.clone()
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    /// The readings that changed since the last take.
    pub fn take_changed(&self) -> Vec<(u32, Reading)> {
        let mut inner = self.inner.lock();
        let keys = std::mem::take(&mut inner.changed);
        keys.into_iter()
            .filter_map(|k| inner.readings.get(&k).map(|r| (k, r.clone())))
            .collect()
    }

    /// Every reading there is (a device that starts listing the sensors starts from them).
    pub fn all(&self) -> Vec<(u32, Reading)> {
        let inner = self.inner.lock();
        inner
            .readings
            .iter()
            .map(|(k, r)| (*k, r.clone()))
            .collect()
    }

    /// Readings that are no longer kept current are forgotten, so the next ones count as changes.
    fn forget(&self) {
        let mut inner = self.inner.lock();
        inner.readings.clear();
        inner.changed.clear();
    }

    /// Whether the device has a connection subscribed to its states (what the device says).
    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.store(wanted, Ordering::Relaxed);
    }

    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Relaxed)
    }
}

/// How often the temperature, the load, the memory and the tuner's signal are read.
const EVERY_ON: Duration = Duration::from_secs(30);
const EVERY_OFF: Duration = Duration::from_secs(60);
/// How long before a subscription that ended is started again.
const FOLLOW_RETRY: Duration = Duration::from_secs(60);
/// The longest the thread sleeps with nothing to do, so that a stop is noticed even without the hub.
const LONGEST_SLEEP: Duration = Duration::from_secs(3600);
const TUNER_FACTS: &str = "plugin.live-tv-tuner.facts";
const FOREGROUND_URI: &str = "luna://com.webos.applicationManager/getForegroundAppInfo";
const PICTURE_URI: &str = "luna://com.webos.settingsservice/getSystemSettings";
const PICTURE_PAYLOAD: &str = r#"{"category":"picture","keys":["pictureMode"],"subscribe":true}"#;
const INPUTS_URI: &str = "luna://com.webos.service.eim/getAllInputStatus";
const LIVE_TV: &str = "com.webos.app.livetv";

/// Starts the sampler. With the plugin off, or nobody listening, it is one thread waiting on the hub.
pub fn spawn(
    ctx: &Ctx,
    shared: Arc<Shared>,
    board: Arc<Board>,
    tuner: Arc<Tuner>,
    hw: Arc<dyn Hardware>,
) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("tvsensors".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            Sampler {
                luna: ctx.config.luna.send.clone(),
                ctx,
                shared,
                board,
                tuner,
                hw,
                follows: Vec::new(),
                next_sample: None,
                follow_retry: None,
            }
            .run();
        })
}

struct Sampler {
    ctx: Ctx,
    luna: String,
    shared: Arc<Shared>,
    board: Arc<Board>,
    tuner: Arc<Tuner>,
    hw: Arc<dyn Hardware>,
    follows: Vec<Follow>,
    next_sample: Option<Instant>,
    follow_retry: Option<Instant>,
}

impl Sampler {
    fn run(&mut self) {
        let hub = self.ctx.hub.clone();
        let topics: Vec<String> = [screen::TOPIC, crate::ble::TOPIC, TUNER_FACTS, "plugins"]
            .iter()
            .map(|t| (*t).to_owned())
            .collect();
        let mut after = hub.events_after(None, None, None).seq;
        while !self.ctx.stop.is_set() {
            let now = Instant::now();
            let wait = if self.shared.switched_on(ID) && self.board.wanted() {
                self.turn(now)
            } else {
                self.rest();
                LONGEST_SLEEP
            };
            after = hub
                .wait_events(
                    Some(after),
                    None,
                    Some(&topics),
                    wait.max(Duration::from_millis(50)),
                )
                .seq;
        }
        self.rest();
    }

    /// Nothing is read: the subscriptions end and what was read is forgotten.
    fn rest(&mut self) {
        if !self.follows.is_empty() || self.next_sample.is_some() {
            self.follows.clear();
            self.next_sample = None;
            self.follow_retry = None;
            self.board.forget();
        }
    }

    /// Reads what is due; answers how long until the next reading is.
    fn turn(&mut self, now: Instant) -> Duration {
        self.follow(now);
        let screen = self.ctx.hub.get(screen::TOPIC);
        let lit = screen::is_on(&self.ctx.hub);
        let state = screen
            .as_ref()
            .and_then(|s| s.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.board.put(POWER, text_or_missing(state));
        self.board.put(SCREEN, Reading::Flag(lit));
        let status = self.tuner.status();
        self.board.put(TUNER, Reading::Text(status.to_string()));
        if self.next_sample.is_none_or(|t| now >= t) {
            self.next_sample = Some(now + if lit { EVERY_ON } else { EVERY_OFF });
            self.board.put(TEMPERATURE, number(read_temperature()));
            self.board.put(LOAD, number(read_load()));
            self.board.put(MEMORY, number(read_available_mb()));
            let signal = match status {
                Status::Streaming { .. } => {
                    self.hw.signal().map(|s| (s.strength() * 100.0).round())
                }
                _ => None,
            };
            self.board.put(SIGNAL, number(signal));
        }
        let mut next = self
            .next_sample
            .map_or(EVERY_ON, |t| t.saturating_duration_since(now));
        if let Some(retry) = self.follow_retry {
            next = next.min(retry.saturating_duration_since(now));
        }
        next
    }

    /// Keeps the two subscriptions running, starting again one that ended (after [`FOLLOW_RETRY`]).
    fn follow(&mut self, now: Instant) {
        if self.follows.len() == 2 && self.follows.iter().all(Follow::alive) {
            return;
        }
        if self.follow_retry.is_some_and(|t| now < t) {
            return;
        }
        self.follows.clear();
        let board = self.board.clone();
        let inputs = Inputs::new(&self.luna);
        let foreground = Follow::start(
            &self.luna,
            FOREGROUND_URI,
            r#"{"subscribe":true}"#,
            Box::new(move |reply| {
                if let Some(app) = reply.get("appId").and_then(Value::as_str) {
                    board.put(
                        APP,
                        Reading::Text(if app.is_empty() { "none" } else { app }.to_owned()),
                    );
                    board.put(INPUT, Reading::Text(inputs.name_of(app)));
                }
            }),
        );
        let board = self.board.clone();
        let picture = Follow::start(
            &self.luna,
            PICTURE_URI,
            PICTURE_PAYLOAD,
            Box::new(move |reply| {
                if let Some(mode) = reply
                    .pointer("/settings/pictureMode")
                    .and_then(Value::as_str)
                {
                    board.put(PICTURE, Reading::Text(mode.to_owned()));
                }
            }),
        );
        match (foreground, picture) {
            (Ok(a), Ok(b)) => {
                self.follows = vec![a, b];
                self.follow_retry = None;
            }
            (a, b) => {
                for e in [a.err(), b.err()].into_iter().flatten() {
                    crate::log_warn!("tvsensors: a subscription did not start: {e}");
                }
                self.follow_retry = Some(now + FOLLOW_RETRY);
            }
        }
    }
}

fn number(value: Option<f32>) -> Reading {
    value.map_or(Reading::Missing, Reading::Number)
}

fn text_or_missing(text: &str) -> Reading {
    if text.is_empty() {
        Reading::Missing
    } else {
        Reading::Text(text.to_owned())
    }
}

/// The SoC's temperature in °C, as LG's power manager reports it (`61`).
fn read_temperature() -> Option<f32> {
    parse_number(&std::fs::read_to_string("/proc/lg/pm/temperature").ok()?)
}

fn parse_number(text: &str) -> Option<f32> {
    text.split_whitespace().next()?.parse().ok()
}

/// The load of the last minute, to two places.
fn read_load() -> Option<f32> {
    let load = parse_number(&std::fs::read_to_string("/proc/loadavg").ok()?)?;
    Some((load * 100.0).round() / 100.0)
}

fn read_available_mb() -> Option<f32> {
    available_mb(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// `MemAvailable` of `/proc/meminfo`, in whole MB.
fn available_mb(meminfo: &str) -> Option<f32> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: f32 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some((kb / 1024.0).round())
}

/// The names of the TV's inputs by the app that shows each, read from LG's input list when an input's app is first seen in front.
struct Inputs {
    luna: String,
    names: Mutex<HashMap<String, String>>,
}

impl Inputs {
    fn new(luna: &str) -> Self {
        Self {
            luna: luna.to_owned(),
            names: Mutex::new(HashMap::new()),
        }
    }

    /// The input `app` is: its name in LG's list, Live TV, or `None` for an app that is not an input.
    fn name_of(&self, app: &str) -> String {
        if app == LIVE_TV {
            return "Live TV".to_owned();
        }
        if !is_input_app(app) {
            return "None".to_owned();
        }
        if let Some(name) = self.names.lock().get(app) {
            return name.clone();
        }
        if let Some(reply) = luna_once(&self.luna, INPUTS_URI, "{}") {
            *self.names.lock() = input_names(&reply);
        }
        self.names
            .lock()
            .get(app)
            .cloned()
            .unwrap_or_else(|| app.trim_start_matches("com.webos.app.").to_uppercase())
    }
}

/// LG's apps that show an external input (`com.webos.app.hdmi2`, `com.webos.app.externalinput.component`).
fn is_input_app(app: &str) -> bool {
    [
        "com.webos.app.hdmi",
        "com.webos.app.externalinput",
        "com.webos.app.composite",
        "com.webos.app.component",
    ]
    .iter()
    .any(|p| app.starts_with(p))
}

/// `getAllInputStatus`'s inputs: the app that shows each, and the name the owner sees.
fn input_names(reply: &Value) -> HashMap<String, String> {
    reply
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let app = d.get("appId")?.as_str()?;
            let label = d
                .get("label")
                .and_then(Value::as_str)
                .filter(|l| !l.is_empty());
            let name = label.or_else(|| d.get("id")?.as_str())?;
            Some((app.to_owned(), name.to_owned()))
        })
        .collect()
}

/// One answer of a luna call (`luna-send -n 1`), or `None` when it fails or takes longer than 4 s.
fn luna_once(luna: &str, uri: &str, payload: &str) -> Option<Value> {
    let mut child = Command::new(luna)
        .args(["-n", "1", uri, payload])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = out.by_ref().take(1024 * 1024).read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    let text = reader.join().ok()?;
    serde_json::Deserializer::from_str(&text)
        .into_iter::<Value>()
        .next()?
        .ok()
}

/// A luna subscription (`luna-send -n 1000000 ... {"subscribe":true}`): every reply goes to the callback, from a reader thread, until
/// the subscription ends or is dropped.
struct Follow {
    child: Child,
    reader: Option<JoinHandle<()>>,
    alive: Arc<AtomicBool>,
}

type OnReply = Box<dyn Fn(&Value) + Send>;

impl Follow {
    fn start(luna: &str, uri: &str, payload: &str, on_reply: OnReply) -> std::io::Result<Self> {
        let mut child = Command::new(luna)
            .args(["-n", "1000000", uri, payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no subscription output"));
        };
        let alive = Arc::new(AtomicBool::new(true));
        let flag = alive.clone();
        let reader = thread::Builder::new()
            .name("tvsensors-follow".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                read_replies(BufReader::new(out), &on_reply);
                flag.store(false, Ordering::Relaxed);
            });
        match reader {
            Ok(reader) => Ok(Self {
                child,
                reader: Some(reader),
                alive,
            }),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The replies of a subscription, one JSON object each (compact, or spread over lines), until the output ends or stops being JSON.
fn read_replies(out: impl BufRead, on_reply: &dyn Fn(&Value)) {
    let mut pending = String::new();
    for line in out.lines() {
        let Ok(line) = line else { break };
        pending.push_str(&line);
        pending.push('\n');
        match serde_json::from_str::<Value>(&pending) {
            Ok(reply) => {
                pending.clear();
                if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
                    break;
                }
                on_reply(&reply);
            }
            Err(e) if e.is_eof() && pending.len() <= 64 * 1024 => {}
            Err(_) => break,
        }
    }
}

/// What the TV sensors plugin's page says.
pub fn facts(linked: bool, listening: bool) -> Vec<(&'static str, String)> {
    let status = match (linked, listening) {
        (false, _) => "Needs the Bluetooth proxy's link: run ble-link on the TV".to_owned(),
        (true, false) => "Waiting for Home Assistant".to_owned(),
        (true, true) => format!("Sending {} sensors to Home Assistant", entities().len()),
    };
    vec![("status", status)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_object_ids_are_unique_and_keys_are_never_zero() {
        let all = entities();
        let keys: BTreeSet<u32> = all.iter().map(|e| e.key).collect();
        let ids: BTreeSet<&str> = all.iter().map(|e| e.object_id).collect();
        assert_eq!((keys.len(), ids.len()), (all.len(), all.len()));
        assert!(!keys.contains(&0));
    }

    #[test]
    fn the_board_hands_over_each_change_once_and_wakes_the_device() {
        let board = Board::default();
        let woken = Arc::new(AtomicBool::new(false));
        let flag = woken.clone();
        board.set_wake(Arc::new(move || flag.store(true, Ordering::Relaxed)));
        board.put(TEMPERATURE, Reading::Number(51.0));
        board.put(TEMPERATURE, Reading::Number(51.0));
        board.put(APP, Reading::Text("netflix".into()));
        assert!(woken.swap(false, Ordering::Relaxed));
        assert_eq!(
            board.take_changed(),
            vec![
                (TEMPERATURE, Reading::Number(51.0)),
                (APP, Reading::Text("netflix".into()))
            ]
        );
        board.put(TEMPERATURE, Reading::Number(51.0));
        assert!(
            board.take_changed().is_empty() && !woken.load(Ordering::Relaxed),
            "an unchanged reading is no change"
        );
        board.forget();
        board.put(TEMPERATURE, Reading::Number(51.0));
        assert_eq!(
            board.take_changed().len(),
            1,
            "after a rest every reading is new"
        );
    }

    #[test]
    fn the_proc_files_read_as_lg_and_linux_write_them() {
        assert_eq!(parse_number("61\n"), Some(61.0));
        assert_eq!(
            parse_number("15.66 14.65 14.64 2/1133 13124\n"),
            Some(15.66)
        );
        assert_eq!(
            available_mb("MemTotal:  2000000 kB\nMemFree:  170340 kB\nMemAvailable:  866372 kB\n"),
            Some(846.0)
        );
        assert_eq!(available_mb("MemFree: 1 kB\n"), None);
    }

    #[test]
    fn an_input_is_named_as_lg_names_it_and_an_app_is_no_input() {
        let reply: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/luna/getAllInputStatus.json"
        ))
        .unwrap();
        let names = input_names(&reply);
        assert_eq!(
            names.get("com.webos.app.hdmi2").map(String::as_str),
            Some("AVR-S760H")
        );
        assert_eq!(
            names.get("com.webos.app.hdmi1").map(String::as_str),
            Some("HDMI 1")
        );
        let inputs = Inputs::new("/bin/false");
        *inputs.names.lock() = names;
        assert_eq!(inputs.name_of("com.webos.app.hdmi2"), "AVR-S760H");
        assert_eq!(inputs.name_of(LIVE_TV), "Live TV");
        assert_eq!(inputs.name_of("netflix"), "None");
    }

    #[test]
    fn subscription_replies_reach_the_callback_compact_or_spread_until_one_fails() {
        let out = "{\"subscribed\":true,\"returnValue\":true,\"appId\":\"netflix\"}\n{\n  \"returnValue\": true,\n  \"appId\": \"com.webos.app.home\"\n}\n{\"returnValue\":false}\n{\"appId\":\"never\"}\n";
        let got = Mutex::new(Vec::new());
        read_replies(out.as_bytes(), &|reply: &Value| {
            got.lock()
                .push(reply["appId"].as_str().unwrap_or_default().to_owned())
        });
        assert_eq!(*got.lock(), vec!["netflix", "com.webos.app.home"]);
    }
}
