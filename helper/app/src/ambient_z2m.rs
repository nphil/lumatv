//! The Zigbee2MQTT output of ambient light: Hue bulbs in Zigbee entertainment mode, driven straight from the TV over MQTT, with
//! no Hue bridge and no Bifrost. One thread that owns the connection to the broker.
//!
//! ```text
//!   ambient (30 Hz)  --set(demand, frame)-->  this thread  --MQTT-->  broker  -->  Zigbee2MQTT  -->  coordinator  -->  the first bulb,
//!                                                                                                        which rebroadcasts to the others
//! ```
//!
//! It does what Bifrost's z2m backend does for a HueStream client (`docs/research/z2m-direct-result.md`), and sends the same bytes:
//!
//! - **Off**: nothing is open. Going to Off ends a stream that was running: the bulbs are sent black twice, told the stream is over
//!   (command 3), and put back to the state Zigbee2MQTT last reported for each before the stream (`{"state": "OFF"}`, or on with
//!   its brightness and colour), so the room is as it was. This is what happens when the screen goes off.
//! - **Discover**: the broker is connected to and the device list read, to check that every named light exists; nothing is sent to
//!   a light. The lights' positions come from the saved link, so ambient light is told them as soon as the thread starts and asks
//!   for this only when the list was never read.
//! - **Stream**: connect, subscribe to `<base>/bridge/devices` (retained: the list arrives at once, and again whenever a bulb's
//!   network address changes) and to each light's own state topic, resolve the names to network addresses, send the start sequence
//!   (light balance to full, a segment map for a strip, then command 3 with the frame counter to each bulb), then the latest frame
//!   at most `fps` times a second and again after a second of silence, to the first light's topic, with the fade of each frame set
//!   to one interval.
//!
//! A lost connection, a refused publish or a bulb Zigbee2MQTT does not know waits 5 s, then 10, up to 60, and the words of it are
//! the output's [`Status`], which ambient light shows as the `problem`. The password is used only in CONNECT, to the address it was
//! saved with; nothing here logs, prints or returns it.
//!
//! The output never adds to a busy Zigbee network (one stream once saturated the coordinator and aborted an OTA update of a leak
//! sensor, `docs/research/z2m-direct-result.md`). It listens to what Zigbee2MQTT says about updates: a device's `update.state` on
//! its state topic (and in the device list), the log lines of an update, and the requests and responses of `ota_update`. While
//! any device is updating, and for a settle time after the last one, no start sequence and no frame is sent; a stream that is
//! live is ended the normal way and the status says `paused: Zigbee2MQTT is updating <device>`; it starts again by itself. Late
//! answers from the coordinator (a failed publish to a light, a dataRequest timeout in Zigbee2MQTT's log) halve the frame rate
//! down to 5 a second, a further one pauses the stream for a while, and the rate comes back by half again for each quiet period.

use crate::ambient_z2m_frame::{self as fr, Mode, Record};
use crate::ambient_z2m_mqtt::{Client, Error as MqttError, Message, Options};
use crate::ambient_z2m_store::Z2mLink;
use crate::config::Z2mConfig;
pub use crate::hue::{Demand, Event, Frame, Status};
use parking_lot::{Condvar, Mutex};
use serde_json::{Value as Json, json};
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A frame is sent again after this long without a new one (the bulbs stay in step, and the path stays warm).
const KEEPALIVE: Duration = Duration::from_secs(1);
/// How often the connection is read while streaming (device list changes, the broker's pings).
const POLL: Duration = Duration::from_millis(250);
/// MQTT's keep-alive, and how long a connect or an answer may take.
const MQTT_KEEP_ALIVE: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(5);
/// How long the retained device list, and then the lights' states, are waited for after subscribing.
const DEVICES_WAIT: Duration = Duration::from_secs(5);
const STATES_WAIT: Duration = Duration::from_millis(400);
/// The pause after a failure, which doubles up to the longest.
const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_LONGEST: Duration = Duration::from_secs(60);
/// A stop on the way out must not hold the helper up.
const STOP_WITHIN: Duration = Duration::from_secs(4);
/// The slowest the frames go before a late answer pauses the stream instead.
const FLOOR_FPS: u8 = 5;

struct Slot {
    demand: Demand,
    frame: Option<Frame>,
    fresh: bool,
    stop: bool,
    status: Status,
}

struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}

/// The thread, and the way to talk to it. Dropping it ends the stream (see [`Output::shutdown`]).
pub struct Output {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Output {
    /// Starts the thread for `link`. It tells ambient light the lights at once and does nothing else until it is given a demand.
    pub fn spawn(
        link: Z2mLink,
        pacing: Z2mConfig,
        events: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                demand: Demand::Off,
                frame: None,
                fresh: false,
                stop: false,
                status: Status {
                    state: "idle",
                    problem: None,
                },
            }),
            wake: Condvar::new(),
        });
        let fps = link.fps;
        let worker = Worker {
            shared: shared.clone(),
            events,
            link,
            client: None,
            devices: None,
            states: HashMap::new(),
            live: None,
            counter: 0,
            last_frame: None,
            seen: Demand::Off,
            retry_at: None,
            backoff: RETRY_FIRST,
            last_problem: String::new(),
            pending: false,
            rate: fps,
            pacing,
            ota: HashMap::new(),
            ota_settle_until: None,
            ota_last: String::new(),
            last_cut: None,
            recover_at: None,
            pause_until: None,
            held: false,
            polled_at: Instant::now(),
            subscribed: HashSet::new(),
        };
        let thread = thread::Builder::new()
            .name("z2m".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || worker.run())?;
        Ok(Self {
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// What is wanted now, and the latest frame (only meaningful for [`Demand::Stream`]). Never blocks on the network.
    pub fn set(&self, demand: Demand, frame: Option<Frame>) {
        let mut slot = self.shared.slot.lock();
        let mut changed = slot.demand != demand;
        slot.demand = demand;
        if demand == Demand::Stream {
            if let Some(frame) = frame
                && !frame.is_empty()
            {
                slot.frame = Some(frame);
                slot.fresh = true;
                changed = true;
            }
        } else {
            slot.frame = None;
            slot.fresh = false;
        }
        drop(slot);
        if changed {
            self.shared.wake.notify_all();
        }
    }

    pub fn status(&self) -> Status {
        self.shared.slot.lock().status.clone()
    }

    /// Ends a running stream (black, command 3, the bulbs' states back) and the thread. Waits for it.
    pub fn shutdown(&self) {
        self.shared.slot.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A device of Zigbee2MQTT's list that matters here.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Device {
    network_address: u16,
    vendor: String,
    model: String,
    /// Zigbee2MQTT's own word that an update is running (`update.state`), if the list carries it.
    updating: bool,
}

/// A light of the link with its address: `addr + i` is segment `i`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Resolved {
    name: String,
    addr: u16,
    segments: u8,
    /// The first engine channel of this light.
    channel: u8,
}

struct Live {
    lights: Vec<Resolved>,
    /// The states the lights had before the stream, by name, for the restore at the end.
    before: HashMap<String, Json>,
    sent_at: Option<Instant>,
    polled_at: Instant,
    smoothing: u16,
}

struct Worker {
    shared: Arc<Shared>,
    events: Arc<dyn Fn(Event) + Send + Sync>,
    link: Z2mLink,
    client: Option<Client>,
    /// Zigbee2MQTT's devices by friendly name, from the last `bridge/devices`; `None` until one was read.
    devices: Option<HashMap<String, Device>>,
    /// The last state Zigbee2MQTT published for each light, by name.
    states: HashMap<String, Json>,
    live: Option<Live>,
    /// The frame counter, continued across streams as Bifrost does.
    counter: u32,
    last_frame: Option<Frame>,
    /// The demand the loop last acted on.
    seen: Demand,
    retry_at: Option<Instant>,
    backoff: Duration,
    last_problem: String,
    /// A frame has come that has not been sent yet.
    pending: bool,
    /// The frames a second now: the link's, less what the coordinator's late answers took off. Set when the thread starts.
    rate: u8,
    pacing: Z2mConfig,
    /// Devices Zigbee2MQTT is updating over the air, by friendly name.
    ota: HashMap<String, Updating>,
    /// No frames before this: an update ended (or went quiet) a moment ago.
    ota_settle_until: Option<Instant>,
    /// The last device that was updating, for the words of the settle.
    ota_last: String,
    /// When the rate was last cut, and when it next steps back up.
    last_cut: Option<Instant>,
    recover_at: Option<Instant>,
    /// No stream before this: the coordinator answered late too often.
    pause_until: Option<Instant>,
    /// Connected, no stream open, waiting for a pause to end.
    held: bool,
    polled_at: Instant,
    /// Topics subscribed to beyond the first ones: a device whose name has a slash is not under `<base>/+`.
    subscribed: HashSet<String>,
}

/// What one pass of the loop is told.
struct Pass {
    demand: Demand,
    frame: Option<Frame>,
    stop: bool,
}

/// The device list's entries that are lights, by friendly name.
fn parse_devices(payload: &[u8]) -> Option<HashMap<String, Device>> {
    let list: Vec<Json> = serde_json::from_slice(payload).ok()?;
    let mut out = HashMap::new();
    for d in &list {
        let (Some(name), Some(addr)) = (d["friendly_name"].as_str(), d["network_address"].as_u64())
        else {
            continue;
        };
        if d["type"].as_str() == Some("Coordinator") {
            continue;
        }
        out.insert(
            name.to_owned(),
            Device {
                network_address: u16::try_from(addr).unwrap_or(0),
                vendor: d["definition"]["vendor"].as_str().unwrap_or("").to_owned(),
                model: d["definition"]["model"].as_str().unwrap_or("").to_owned(),
                updating: d["update"]["state"].as_str() == Some("updating"),
            },
        );
    }
    Some(out)
}

/// The `/set` payload that puts a light back to a state Zigbee2MQTT reported: off, or on with its brightness and the colour of
/// the mode it was in. `None` when the state says nothing usable.
fn restore_payload(state: &Json) -> Option<Json> {
    match state["state"].as_str()? {
        "OFF" => Some(json!({"state": "OFF"})),
        "ON" => {
            let mut set = json!({"state": "ON"});
            if let Some(b) = state["brightness"].as_u64() {
                set["brightness"] = json!(b.clamp(1, 254));
            }
            let c = &state["color"];
            match state["color_mode"].as_str() {
                Some("color_temp") => {
                    if let Some(ct) = state["color_temp"].as_u64() {
                        set["color_temp"] = json!(ct);
                    }
                }
                Some("xy" | "hs") if c["x"].is_number() && c["y"].is_number() => {
                    set["color"] = json!({"x": c["x"], "y": c["y"]});
                }
                Some("xy" | "hs") if c["hue"].is_number() && c["saturation"].is_number() => {
                    set["color"] = json!({"hue": c["hue"], "saturation": c["saturation"]});
                }
                _ => {}
            }
            Some(set)
        }
        _ => None,
    }
}

/// Connects to the link's broker and subscribes to the device list and to each light's state topic.
fn open_client(link: &Z2mLink) -> Result<Client, String> {
    let id = format!(
        "lumatv-{}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    );
    let options = Options {
        client_id: &id,
        username: link.username.as_deref(),
        password: link.password.as_ref().map(|p| p.expose()),
        keep_alive: MQTT_KEEP_ALIVE,
        timeout: TIMEOUT,
    };
    let addr = SocketAddr::from((link.host, link.port));
    let mut client = Client::connect(addr, &options).map_err(|e| format!("{addr}: {e}"))?;
    let base = &link.base_topic;
    // The update topics first: their retained messages then arrive before the lights' states, which the start waits for.
    let mut topics = vec![
        format!("{base}/bridge/devices"),
        // Every device's state, where an update shows as `update.state`.
        format!("{base}/+"),
        format!("{base}/bridge/logging"),
        format!("{base}/bridge/request/device/ota_update/#"),
        format!("{base}/bridge/response/device/ota_update/#"),
    ];
    topics.extend(link.lights.iter().map(|l| format!("{base}/{}", l.name)));
    for topic in &topics {
        client.subscribe(topic).map_err(|e| e.to_string())?;
    }
    Ok(client)
}

/// The link's lights with their network addresses from the device list, or the words of what is missing.
fn resolve(
    link: &Z2mLink,
    devices: Option<&HashMap<String, Device>>,
) -> Result<Vec<Resolved>, String> {
    let Some(devices) = devices else {
        return Err(format!(
            "Zigbee2MQTT has not published its device list on {}/bridge/devices (is it running, and is the base topic right?)",
            link.base_topic
        ));
    };
    let mut out = Vec::new();
    let mut channel: u8 = 0;
    for l in &link.lights {
        let Some(d) = devices.get(&l.name) else {
            let mut hue: Vec<&str> = devices
                .iter()
                .filter(|(_, d)| {
                    d.vendor.eq_ignore_ascii_case("Philips")
                        || d.vendor.eq_ignore_ascii_case("Signify")
                })
                .map(|(n, _)| n.as_str())
                .collect();
            hue.sort_unstable();
            return Err(format!(
                "Zigbee2MQTT has no device named {:?}; its Hue lights are: {}",
                l.name,
                if hue.is_empty() {
                    "none".to_owned()
                } else {
                    hue.join(", ")
                }
            ));
        };
        out.push(Resolved {
            name: l.name.clone(),
            addr: d.network_address,
            segments: l.segments.max(1),
            channel,
        });
        channel = channel.saturating_add(l.segments.max(1));
    }
    Ok(out)
}

/// A light of the link as Zigbee2MQTT knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub network_address: u16,
    pub model: String,
    pub segments: u8,
}

/// Connects once, reads the device list and resolves the link's lights; nothing is sent to any light. What `z2m-link` does to
/// check a link before saving it.
pub fn probe(link: &Z2mLink) -> Result<Vec<Found>, String> {
    let mut client = open_client(link)?;
    let devices_topic = format!("{}/bridge/devices", link.base_topic);
    let deadline = Instant::now() + DEVICES_WAIT;
    let mut devices = None;
    while devices.is_none() && Instant::now() < deadline {
        match client.poll(deadline.saturating_duration_since(Instant::now())) {
            Ok(Some(m)) if m.topic == devices_topic => devices = parse_devices(&m.payload),
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => {
                client.disconnect();
                return Err(e.to_string());
            }
        }
    }
    let resolved = resolve(link, devices.as_ref());
    let found = resolved.map(|lights| {
        lights
            .iter()
            .map(|l| Found {
                name: l.name.clone(),
                network_address: l.addr,
                model: devices
                    .as_ref()
                    .and_then(|d| d.get(&l.name))
                    .map(|d| format!("{} {}", d.vendor, d.model).trim().to_owned())
                    .unwrap_or_default(),
                segments: l.segments,
            })
            .collect()
    });
    client.disconnect();
    found
}

/// A device Zigbee2MQTT is updating over the air.
struct Updating {
    /// Taken as over after this, unless it is heard of again.
    until: Instant,
    /// Known only from a request, not yet from the device's own state.
    by_request: bool,
}

/// What opening a stream came to.
enum Opened {
    Live(Live),
    /// Not wanted any more by the time it was ready.
    Unwanted,
    /// Not now: the words say why.
    Held(String),
}

/// The text between the first pair of quotes: Zigbee2MQTT quotes friendly names in its log.
fn quoted(s: &str) -> Option<&str> {
    let rest = s.split_once('\'')?.1;
    Some(rest.split_once('\'')?.0)
}

/// A device and whether it is updating, from a log line of Zigbee2MQTT's update extension.
fn ota_log(message: &str) -> Option<(&str, bool)> {
    if message.starts_with("Update of '") {
        return Some((quoted(message)?, !message.contains("failed")));
    }
    if message.starts_with("Updating '") {
        return Some((quoted(message)?, true));
    }
    if message.starts_with("Finished update of '") {
        return Some((quoted(message)?, false));
    }
    None
}

/// Whether a log line of Zigbee2MQTT says the coordinator answered late or not at all: a failure that names one of the lights, or a
/// dataRequest timeout (the coordinator's own, for whichever device).
fn late_answer<'a>(message: &str, mut lights: impl Iterator<Item = &'a str>) -> bool {
    let lower = message.to_lowercase();
    let failed = ["failed", "timeout", "timed out", "no ack"]
        .iter()
        .any(|w| lower.contains(w));
    lower.contains("datarequest") || (failed && lights.any(|n| message.contains(n)))
}

impl Worker {
    /// The least time between two frames.
    fn interval(&self) -> Duration {
        Duration::from_millis(1000 / u64::from(self.rate.max(1)))
    }

    fn topic(&self, name: &str, suffix: &str) -> String {
        format!("{}/{name}{suffix}", self.link.base_topic)
    }

    fn run(mut self) {
        (self.events)(Event::Lights(self.link.engine_lights()));
        loop {
            let pass = self.wait();
            if pass.stop {
                self.end_stream();
                self.close();
                self.set_status("idle", None);
                return;
            }
            self.seen = pass.demand;
            if let Some(frame) = pass.frame {
                // The same frame again (a still colour) is not news: the keepalive carries it once a second.
                self.pending = self.pending || self.last_frame.as_ref() != Some(&frame);
                self.last_frame = Some(frame);
            }
            match pass.demand {
                Demand::Off => {
                    self.end_stream();
                    self.close();
                    self.retry_at = None;
                    self.set_status("idle", None);
                }
                Demand::Discover => {
                    self.end_stream();
                    if self.devices.is_none() {
                        self.discover();
                    } else {
                        self.set_status("idle", None);
                    }
                    self.close();
                }
                Demand::Stream => self.stream_pass(),
            }
        }
    }

    fn retry_ready(&self, now: Instant) -> bool {
        self.retry_at.is_none_or(|t| now >= t)
    }

    /// Whether the thread has something to do, and when it next will have.
    fn due(&self, slot: &Slot, now: Instant) -> (bool, Option<Instant>) {
        if slot.stop || slot.demand != self.seen {
            return (true, None);
        }
        match (slot.demand, &self.live) {
            (Demand::Off, None) => (false, None),
            (Demand::Off | Demand::Discover, Some(_)) => (true, None),
            (Demand::Discover, None) => {
                if self.devices.is_some() {
                    (false, None)
                } else if self.retry_ready(now) {
                    (true, None)
                } else {
                    (false, self.retry_at)
                }
            }
            (Demand::Stream, None) => {
                if self.held {
                    let next = self.polled_at + POLL;
                    (now >= next, Some(next))
                } else if self.retry_ready(now) {
                    (true, None)
                } else {
                    (false, self.retry_at)
                }
            }
            (Demand::Stream, Some(live)) => {
                let from_sent = |gap: Duration| live.sent_at.map_or(now, |t| t + gap);
                let mut next = live.polled_at + POLL;
                if self.last_frame.is_some() {
                    next = next.min(from_sent(KEEPALIVE));
                    if slot.fresh || self.pending {
                        next = next.min(from_sent(self.interval()));
                    }
                } else if slot.fresh {
                    next = now;
                }
                (now >= next, Some(next))
            }
        }
    }

    /// Blocks until there is something to do.
    fn wait(&mut self) -> Pass {
        let mut slot = self.shared.slot.lock();
        loop {
            let now = Instant::now();
            let (due, wake_at) = self.due(&slot, now);
            if due {
                let frame = if slot.fresh {
                    slot.fresh = false;
                    slot.frame.clone()
                } else {
                    None
                };
                return Pass {
                    demand: slot.demand,
                    frame,
                    stop: slot.stop,
                };
            }
            match wake_at {
                Some(at) => {
                    let _ = self.shared.wake.wait_until(&mut slot, at);
                }
                None => self.shared.wake.wait(&mut slot),
            }
        }
    }

    fn set_status(&mut self, state: &'static str, problem: Option<String>) {
        let status = Status { state, problem };
        {
            let mut slot = self.shared.slot.lock();
            if slot.status == status {
                return;
            }
            slot.status = status;
        }
        (self.events)(Event::Status);
    }

    fn fail(&mut self, words: String) {
        if self.last_problem != words {
            crate::log_warn!("z2m: {words}");
            self.last_problem.clone_from(&words);
        }
        self.retry_at = Some(Instant::now() + self.backoff);
        self.backoff = (self.backoff * 2).min(RETRY_LONGEST);
        self.set_status("error", Some(words));
    }

    fn succeeded(&mut self) {
        self.retry_at = None;
        self.backoff = RETRY_FIRST;
        self.last_problem.clear();
    }

    /// Opens the connection if it is not open: CONNECT, then the device list and each light's state subscribed to. A new
    /// connection starts with no device list: the retained one arrives afresh, so a bulb that has joined since is seen.
    fn connect(&mut self) -> Result<(), String> {
        if self.client.is_none() {
            self.client = Some(open_client(&self.link)?);
            self.devices = None;
            self.states.clear();
            self.subscribed = self
                .link
                .lights
                .iter()
                .map(|l| self.topic(&l.name, ""))
                .collect();
        }
        Ok(())
    }

    fn close(&mut self) {
        self.held = false;
        if let Some(client) = self.client.take() {
            client.disconnect();
        }
    }

    /// Takes in a message from the broker: a new device list, a light's state, or what it says about updates and late answers.
    fn take(&mut self, m: Message) {
        let now = Instant::now();
        let Some(rest) = m
            .topic
            .strip_prefix(self.link.base_topic.as_str())
            .and_then(|r| r.strip_prefix('/'))
        else {
            return;
        };
        if let Some(bridge) = rest.strip_prefix("bridge/") {
            match bridge {
                "devices" => {
                    if let Some(devices) = parse_devices(&m.payload) {
                        for (name, d) in &devices {
                            if d.updating {
                                self.ota_note(name, true, false, now);
                            }
                        }
                        self.watch_slashed(&devices);
                        self.devices = Some(devices);
                    }
                }
                "logging" => self.take_log(&m.payload, now),
                t if t.starts_with("request/device/ota_update") && !t.contains("/check") => {
                    if let Ok(v) = serde_json::from_slice::<Json>(&m.payload)
                        && let Some(id) = v["id"].as_str()
                    {
                        self.ota_note(id, true, true, now);
                    }
                }
                t if t.starts_with("response/device/ota_update") && !t.contains("/check") => {
                    let v = serde_json::from_slice::<Json>(&m.payload).unwrap_or(Json::Null);
                    self.ota_answered(v["data"]["id"].as_str(), now);
                }
                _ => {}
            }
            return;
        }
        let is_light = self.link.lights.iter().any(|l| l.name == rest);
        if !is_light && !m.payload.windows(8).any(|w| w == b"\"update\"") {
            return;
        }
        let Ok(state) = serde_json::from_slice::<Json>(&m.payload) else {
            return;
        };
        if !state.is_object() {
            return;
        }
        if let Some(s) = state["update"]["state"].as_str() {
            self.ota_note(rest, s == "updating", false, now);
        }
        if is_light {
            self.states.insert(rest.to_owned(), state);
        }
    }

    /// Reads what the broker has sent, for up to `wait` (a zero wait takes what has arrived), or until `until` says enough.
    fn drain(
        &mut self,
        wait: Duration,
        mut until: impl FnMut(&Self) -> bool,
    ) -> Result<(), MqttError> {
        let deadline = Instant::now() + wait;
        loop {
            if until(self) {
                return Ok(());
            }
            let Some(client) = self.client.as_mut() else {
                return Ok(());
            };
            match client.poll(deadline.saturating_duration_since(Instant::now()))? {
                Some(m) => self.take(m),
                None => return Ok(()),
            }
        }
    }

    /// The link's lights with their network addresses from the device list, or the words of what is missing.
    fn resolve(&self) -> Result<Vec<Resolved>, String> {
        resolve(&self.link, self.devices.as_ref())
    }

    fn discover(&mut self) {
        self.set_status("connecting", None);
        let found = self.connect().and_then(|()| {
            let _ = self.drain(DEVICES_WAIT, |w| w.devices.is_some());
            self.resolve()
        });
        match found {
            Ok(lights) => {
                crate::log_info!(
                    "z2m: {} light(s) found: {}",
                    lights.len(),
                    lights
                        .iter()
                        .map(|l| format!("{} 0x{:04x}", l.name, l.addr))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                self.succeeded();
                self.set_status("idle", None);
            }
            Err(words) => {
                self.devices = None;
                self.fail(words);
            }
        }
    }

    fn publish(&mut self, topic: &str, payload: &Json) -> Result<(), MqttError> {
        let Some(client) = self.client.as_mut() else {
            return Err(MqttError::Io(io::Error::new(
                io::ErrorKind::NotConnected,
                "not connected",
            )));
        };
        client.publish(topic, payload.to_string().as_bytes())
    }

    fn stream_pass(&mut self) {
        if self.live.is_none() {
            if self.held {
                self.hold_pass();
            } else {
                self.begin();
            }
            return;
        }
        let now = Instant::now();
        if self
            .live
            .as_ref()
            .is_some_and(|l| now >= l.polled_at + POLL)
        {
            if let Some(l) = self.live.as_mut() {
                l.polled_at = now;
            }
            if let Err(e) = self.drain(Duration::ZERO, |_| false) {
                return self.lost(&e.to_string());
            }
            // A bulb that rejoined has a new address: the next frame carries it.
            if let Ok(lights) = self.resolve()
                && let Some(l) = self.live.as_mut()
                && l.lights != lights
            {
                crate::log_info!("z2m: the lights' addresses changed; frames follow");
                l.lights = lights;
            }
            self.tick_rate(now);
            if let Some(words) = self.hold_reason(now) {
                return self.hold(words, now);
            }
        }
        let interval = self.interval();
        let Some(live) = self.live.as_ref() else {
            return;
        };
        let since = live
            .sent_at
            .map_or(Duration::MAX, |t| now.saturating_duration_since(t));
        // A new frame goes out as soon as the pace allows; the last one goes again when nothing new has come for a second.
        let send = (since >= interval && self.pending) || since >= KEEPALIVE;
        let Some(frame) = self.last_frame.clone().filter(|_| send) else {
            return;
        };
        match self.send_frame(&frame) {
            Ok(()) => {
                if let Some(l) = self.live.as_mut() {
                    l.sent_at = Some(now);
                }
                self.pending = false;
            }
            Err(e) => self.lost(&e.to_string()),
        }
    }

    /// The records of a frame for the live lights: a channel the frame does not name is black.
    fn records(lights: &[Resolved], frame: &Frame) -> Vec<Record> {
        let mut out = Vec::new();
        for l in lights {
            let mode = if l.segments > 1 {
                Mode::Segment
            } else {
                Mode::Device
            };
            for i in 0..l.segments {
                let channel = l.channel.saturating_add(i);
                let rgb = frame
                    .iter()
                    .find(|(c, _)| *c == channel)
                    .map_or([0u16; 3], |(_, rgb)| *rgb);
                out.push(fr::record(l.addr.wrapping_add(u16::from(i)), rgb, mode));
            }
        }
        out.truncate(fr::MAX_RECORDS);
        out
    }

    /// One frame to the first light, with every light's record, and the counter moved on.
    fn send_frame(&mut self, frame: &Frame) -> Result<(), MqttError> {
        let Some(live) = self.live.as_ref() else {
            return Ok(());
        };
        let records = Self::records(&live.lights, frame);
        let data = fr::frame(self.counter, live.smoothing, &records);
        let topic = self.topic(&live.lights[0].name, "/set");
        self.publish(&topic, &fr::zclcommand(fr::CMD_FRAME, &data))?;
        self.counter = self.counter.wrapping_add(1);
        Ok(())
    }

    /// The stream is gone without being asked: the broker closed it, or the network did.
    fn lost(&mut self, why: &str) {
        self.live = None;
        self.close();
        self.fail(format!("the stream ended ({why})"));
    }

    fn begin(&mut self) {
        self.set_status("connecting", None);
        match self.open() {
            Ok(Opened::Live(live)) => {
                crate::log_info!(
                    "z2m: streaming to {} at {} a second, frames to {}",
                    live.lights
                        .iter()
                        .map(|l| format!("{} 0x{:04x}", l.name, l.addr))
                        .collect::<Vec<_>>()
                        .join(", "),
                    self.rate,
                    live.lights[0].name
                );
                self.live = Some(live);
                self.held = false;
                self.pending = self.last_frame.is_some();
                if self.rate < self.link.fps {
                    self.recover_at = Some(Instant::now() + self.recover());
                }
                self.succeeded();
                self.live_status();
            }
            Ok(Opened::Held(words)) => {
                crate::log_info!("z2m: {words}");
                self.held = true;
                self.polled_at = Instant::now();
                self.set_status("idle", Some(words));
            }
            Ok(Opened::Unwanted) => {
                self.close();
                self.set_status("idle", None);
            }
            Err(words) => {
                self.close();
                self.fail(words);
            }
        }
    }

    /// Connects, resolves the lights and sends the start sequence, unless an update or a pause says not now.
    fn open(&mut self) -> Result<Opened, String> {
        self.connect()?;
        self.drain(DEVICES_WAIT, |w| w.devices.is_some())
            .map_err(|e| e.to_string())?;
        let lights = self.resolve()?;
        // The lights' retained states follow the device list at once; a moment is enough, and none is not an error.
        let names: Vec<String> = lights.iter().map(|l| l.name.clone()).collect();
        self.drain(STATES_WAIT, |w| {
            names.iter().all(|n| w.states.contains_key(n))
        })
        .map_err(|e| e.to_string())?;
        let wanted = {
            let slot = self.shared.slot.lock();
            slot.demand == Demand::Stream && !slot.stop
        };
        if !wanted {
            return Ok(Opened::Unwanted);
        }
        // What the retained messages said about updates has arrived by now: take it in before anything is sent.
        self.drain(Duration::ZERO, |_| false)
            .map_err(|e| e.to_string())?;
        if let Some(words) = self.hold_reason(Instant::now()) {
            return Ok(Opened::Held(words));
        }
        let before: HashMap<String, Json> = names
            .iter()
            .filter_map(|n| self.states.get(n).map(|s| (n.clone(), s.clone())))
            .collect();
        // Bifrost's start: light balance (and the segment map of a strip) to each, then command 3 with the counter to each.
        for l in &lights {
            let set = self.topic(&l.name, "/set");
            self.publish(&set, &fr::light_balance(fr::BALANCE_FULL))
                .map_err(|e| e.to_string())?;
            if l.segments > 1 {
                let addrs: Vec<u16> = (0..l.segments)
                    .map(|i| l.addr.wrapping_add(u16::from(i)))
                    .collect();
                self.publish(
                    &set,
                    &fr::zclcommand(fr::CMD_SEGMENT_MAP, &fr::segment_map(&addrs)),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let reset = fr::zclcommand(fr::CMD_RESET, &fr::reset(self.counter));
        for l in &lights {
            let set = self.topic(&l.name, "/set");
            self.publish(&set, &reset).map_err(|e| e.to_string())?;
        }
        Ok(Opened::Live(Live {
            lights,
            before,
            sent_at: None,
            polled_at: Instant::now(),
            smoothing: fr::smoothing_for(self.interval()),
        }))
    }

    /// Ends the stream the way a person would want the room left: black, the stream closed, each bulb as it was.
    fn end_stream(&mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        let started = Instant::now();
        let black: Frame = live
            .lights
            .iter()
            .flat_map(|l| (0..l.segments).map(move |i| (l.channel.saturating_add(i), [0u16; 3])))
            .collect();
        self.live = Some(live);
        for _ in 0..2 {
            if self.send_frame(&black).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        let Some(live) = self.live.take() else {
            return;
        };
        self.last_frame = None;
        let reset = fr::zclcommand(fr::CMD_RESET, &fr::reset(self.counter));
        for l in &live.lights {
            if started.elapsed() > STOP_WITHIN {
                break;
            }
            let set = self.topic(&l.name, "/set");
            if let Err(e) = self.publish(&set, &reset) {
                crate::log_warn!("z2m: ending the stream: {e}");
                return;
            }
            if let Some(payload) = live.before.get(&l.name).and_then(restore_payload)
                && let Err(e) = self.publish(&set, &payload)
            {
                crate::log_warn!("z2m: putting {} back: {e}", l.name);
                return;
            }
        }
    }

    fn recover(&self) -> Duration {
        Duration::from_millis(self.pacing.recover_ms)
    }

    /// Notes that a device is, or is no longer, being updated. An update that ends starts the settle time.
    fn ota_note(&mut self, name: &str, updating: bool, by_request: bool, now: Instant) {
        if updating {
            let until = now + Duration::from_millis(self.pacing.ota_stale_ms);
            if let Some(m) = self.ota.get_mut(name) {
                m.until = until;
                m.by_request &= by_request;
            } else {
                crate::log_info!("z2m: {name} is being updated; no frames until it is done");
                self.ota
                    .insert(name.to_owned(), Updating { until, by_request });
            }
        } else if self.ota.remove(name).is_some() {
            self.ota_ended(name, now);
        }
    }

    fn ota_ended(&mut self, name: &str, now: Instant) {
        name.clone_into(&mut self.ota_last);
        let until = now + Duration::from_millis(self.pacing.ota_settle_ms);
        self.ota_settle_until = self.ota_settle_until.max(Some(until));
    }

    /// A response to `ota_update`: the update it was for is over, done or failed. One that does not say which (an error) ends every
    /// update known only from a request.
    fn ota_answered(&mut self, id: Option<&str>, now: Instant) {
        match id {
            Some(id) => self.ota_note(id, false, false, now),
            None => {
                let names: Vec<String> = self
                    .ota
                    .iter()
                    .filter(|(_, m)| m.by_request)
                    .map(|(n, _)| n.clone())
                    .collect();
                for n in names {
                    self.ota_note(&n, false, false, now);
                }
            }
        }
    }

    /// Reads a line of Zigbee2MQTT's log: an update's progress or end, or a late answer from the coordinator.
    fn take_log(&mut self, payload: &[u8], now: Instant) {
        let Ok(v) = serde_json::from_slice::<Json>(payload) else {
            return;
        };
        let (Some(level), Some(message)) = (v["level"].as_str(), v["message"].as_str()) else {
            return;
        };
        if let Some((name, updating)) = ota_log(message) {
            self.ota_note(name, updating, false, now);
        } else if matches!(level, "error" | "warning")
            && late_answer(message, self.link.lights.iter().map(|l| l.name.as_str()))
        {
            self.busy(now);
        }
    }

    /// Subscribes to the state topic of each device whose name has a slash (`<base>/+` does not reach it).
    fn watch_slashed(&mut self, devices: &HashMap<String, Device>) {
        for name in devices.keys().filter(|n| n.contains('/')) {
            let topic = self.topic(name, "");
            if self.subscribed.insert(topic.clone())
                && let Some(client) = self.client.as_mut()
            {
                let _ = client.subscribe(&topic);
            }
        }
    }

    /// Why no frame may be sent now, in words, or `None`: a device is updating, one finished a moment ago, or the stream is paused
    /// after late answers.
    fn hold_reason(&mut self, now: Instant) -> Option<String> {
        let settle = Duration::from_millis(self.pacing.ota_settle_ms);
        let gone: Vec<String> = self
            .ota
            .iter()
            .filter(|(_, m)| m.until <= now)
            .map(|(n, _)| n.clone())
            .collect();
        for name in gone {
            if let Some(m) = self.ota.remove(&name) {
                self.ota_last = name;
                self.ota_settle_until = self.ota_settle_until.max(Some(m.until + settle));
            }
        }
        if !self.ota.is_empty() {
            let mut names: Vec<&str> = self.ota.keys().map(String::as_str).collect();
            names.sort_unstable();
            return Some(format!(
                "paused: Zigbee2MQTT is updating {}",
                names.join(", ")
            ));
        }
        if self.ota_settle_until.is_some_and(|t| now < t) {
            return Some(format!(
                "paused: Zigbee2MQTT has just finished updating {}; the lights follow again in a moment",
                self.ota_last
            ));
        }
        if let Some(t) = self.pause_until {
            if now < t {
                return Some(format!(
                    "paused: the Zigbee network was too busy for the lights; trying again shortly at {} frames a second",
                    self.rate
                ));
            }
            self.pause_until = None;
        }
        None
    }

    /// Ends the live stream for a pause and keeps the connection, so the end of the update is heard.
    fn hold(&mut self, words: String, now: Instant) {
        crate::log_info!("z2m: {words}");
        self.end_stream();
        self.held = true;
        self.polled_at = now;
        self.set_status("idle", Some(words));
    }

    /// One look at the connection while paused: starts again when nothing holds the stream any more.
    fn hold_pass(&mut self) {
        let now = Instant::now();
        if now < self.polled_at + POLL {
            return;
        }
        self.polled_at = now;
        if let Err(e) = self.drain(Duration::ZERO, |_| false) {
            return self.lost(&e.to_string());
        }
        match self.hold_reason(now) {
            Some(words) => self.set_status("idle", Some(words)),
            None => {
                self.held = false;
                self.begin();
            }
        }
    }

    /// The status of a live stream: it says so when the rate is below the link's.
    fn live_status(&mut self) {
        let problem = (self.rate < self.link.fps).then(|| {
            format!(
                "the Zigbee network is busy: {} frames a second instead of {}",
                self.rate, self.link.fps
            )
        });
        self.set_status("streaming", problem);
    }

    /// The pace changed: the fade of a frame is one interval, and the status shows the rate.
    fn rate_changed(&mut self) {
        let smoothing = fr::smoothing_for(self.interval());
        if let Some(live) = self.live.as_mut() {
            live.smoothing = smoothing;
        }
        self.live_status();
    }

    /// The coordinator answered late. Late answers within the cut gap are one event; an event halves the rate, and one at the lowest
    /// rate pauses the stream.
    fn busy(&mut self, now: Instant) {
        if self.live.is_none()
            || self
                .last_cut
                .is_some_and(|t| now < t + Duration::from_millis(self.pacing.busy_cut_gap_ms))
        {
            return;
        }
        self.last_cut = Some(now);
        self.recover_at = Some(now + self.recover());
        let floor = self.link.fps.min(FLOOR_FPS);
        if self.rate <= floor {
            self.pause_until = Some(now + Duration::from_millis(self.pacing.busy_pause_ms));
            crate::log_warn!(
                "z2m: the coordinator answers late even at {} frames a second; pausing",
                self.rate
            );
        } else {
            self.rate = (self.rate / 2).max(floor);
            crate::log_warn!(
                "z2m: the coordinator answers late; {} frames a second now",
                self.rate
            );
            self.rate_changed();
        }
    }

    /// After a quiet period the rate goes up by half again, to the link's.
    fn tick_rate(&mut self, now: Instant) {
        if self.rate >= self.link.fps || self.recover_at.is_none_or(|t| now < t) {
            return;
        }
        let up = u16::from(self.rate) + u16::from((self.rate / 2).max(1));
        self.rate = u8::try_from(up).map_or(self.link.fps, |r| r.min(self.link.fps));
        self.recover_at = Some(now + self.recover());
        self.rate_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_list_gives_lights_by_name_without_the_coordinator() {
        let list = json!([
            {"friendly_name": "Coordinator", "type": "Coordinator", "network_address": 0},
            {"friendly_name": "TV left", "type": "Router", "network_address": 0xD297, "definition": {"vendor": "Philips", "model": "9290024688"}},
            {"friendly_name": "plug", "type": "Router", "network_address": 0x1234, "definition": {"vendor": "IKEA", "model": "E1603"}},
            {"friendly_name": "half", "type": "EndDevice"},
        ]);
        let d = parse_devices(list.to_string().as_bytes()).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d["TV left"].network_address, 0xD297);
        assert_eq!(d["TV left"].vendor, "Philips");
        assert!(parse_devices(b"not json").is_none());
    }

    #[test]
    fn a_restore_is_off_or_on_with_the_colour_of_the_mode() {
        assert_eq!(
            restore_payload(&json!({"state": "OFF", "brightness": 100})),
            Some(json!({"state": "OFF"}))
        );
        assert_eq!(
            restore_payload(
                &json!({"state": "ON", "brightness": 200, "color_mode": "xy", "color": {"x": 0.4, "y": 0.4, "hue": 1}, "color_temp": 300})
            ),
            Some(json!({"state": "ON", "brightness": 200, "color": {"x": 0.4, "y": 0.4}}))
        );
        assert_eq!(
            restore_payload(
                &json!({"state": "ON", "brightness": 0, "color_mode": "color_temp", "color_temp": 366})
            ),
            Some(json!({"state": "ON", "brightness": 1, "color_temp": 366}))
        );
        assert_eq!(
            restore_payload(&json!({"state": "ON"})),
            Some(json!({"state": "ON"}))
        );
        assert_eq!(restore_payload(&json!({"linkquality": 3})), None);
    }

    #[test]
    fn records_follow_the_lights_order_and_segments_count_as_their_own_addresses() {
        let lights = vec![
            Resolved {
                name: "a".into(),
                addr: 0x1000,
                segments: 1,
                channel: 0,
            },
            Resolved {
                name: "s".into(),
                addr: 0x2000,
                segments: 3,
                channel: 1,
            },
        ];
        let frame: Frame = vec![(0, [65535, 0, 0]), (2, [0, 65535, 0]), (3, [0, 0, 65535])];
        let r = Worker::records(&lights, &frame);
        assert_eq!(r.len(), 4);
        assert_eq!((r[0].addr, r[0].mode), (0x1000, Mode::Device));
        assert_eq!(
            (r[1].addr, r[1].mode, r[1].brightness),
            (0x2000, Mode::Segment, 1)
        );
        assert_eq!((r[2].addr, r[3].addr), (0x2001, 0x2002));
        assert_eq!(r[0], fr::record(0x1000, [65535, 0, 0], Mode::Device));
        assert_eq!(r[3], fr::record(0x2002, [0, 0, 65535], Mode::Segment));
    }

    #[test]
    fn update_lines_of_the_log_name_the_device_and_whether_it_is_updating() {
        assert_eq!(
            ota_log("Update of 'leak' at 54.30%, ≈ 4 minutes remaining"),
            Some(("leak", true))
        );
        assert_eq!(
            ota_log("Updating 'leak' to latest firmware"),
            Some(("leak", true))
        );
        assert_eq!(ota_log("Finished update of 'leak'"), Some(("leak", false)));
        assert_eq!(
            ota_log("Update of 'leak' failed (ABORT)"),
            Some(("leak", false))
        );
        assert_eq!(ota_log("MQTT publish: topic 'zigbee2mqtt/leak'"), None);
    }

    #[test]
    fn a_late_answer_names_a_light_or_is_the_coordinators_own_timeout() {
        let lights = ["TV left", "TV right"];
        let late = |m: &str| late_answer(m, lights.iter().copied());
        assert!(late(
            "Publish 'set' 'zclcommand' to 'TV left' failed: 'Error: Timeout - 1 - 11 - 100ms'"
        ));
        assert!(late(
            "Failed to write: SRSP - AF - dataRequest after 6000ms"
        ));
        assert!(!late(
            "Publish 'set' 'state' to 'plug' failed: 'Error: no ack'"
        ));
        assert!(!late("Update of 'TV left' at 5%"));
    }
}
