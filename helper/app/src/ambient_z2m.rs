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

use crate::ambient_z2m_frame::{self as fr, Mode, Record};
use crate::ambient_z2m_mqtt::{Client, Error as MqttError, Message, Options};
use crate::ambient_z2m_store::Z2mLink;
pub use crate::hue::{Demand, Event, Frame, Status};
use parking_lot::{Condvar, Mutex};
use serde_json::{Value as Json, json};
use std::collections::HashMap;
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
    pub fn spawn(link: Z2mLink, events: Arc<dyn Fn(Event) + Send + Sync>) -> io::Result<Self> {
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
    client
        .subscribe(&format!("{}/bridge/devices", link.base_topic))
        .map_err(|e| e.to_string())?;
    for l in &link.lights {
        client
            .subscribe(&format!("{}/{}", link.base_topic, l.name))
            .map_err(|e| e.to_string())?;
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

impl Worker {
    /// The least time between two frames.
    fn interval(&self) -> Duration {
        Duration::from_millis(1000 / u64::from(self.link.fps.max(1)))
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
                if self.retry_ready(now) {
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
        }
        Ok(())
    }

    fn close(&mut self) {
        if let Some(client) = self.client.take() {
            client.disconnect();
        }
    }

    /// Takes in a message from the broker: a new device list, or a light's state.
    fn take(&mut self, m: Message) {
        if m.topic == self.topic("bridge", "/devices") {
            if let Some(devices) = parse_devices(&m.payload) {
                self.devices = Some(devices);
            }
            return;
        }
        for l in &self.link.lights {
            if m.topic == self.topic(&l.name, "") {
                if let Ok(state) = serde_json::from_slice::<Json>(&m.payload)
                    && state.is_object()
                {
                    self.states.insert(l.name.clone(), state);
                }
                return;
            }
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
            self.begin();
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
            Ok(Some(live)) => {
                crate::log_info!(
                    "z2m: streaming to {} at {} a second, frames to {}",
                    live.lights
                        .iter()
                        .map(|l| format!("{} 0x{:04x}", l.name, l.addr))
                        .collect::<Vec<_>>()
                        .join(", "),
                    self.link.fps,
                    live.lights[0].name
                );
                self.live = Some(live);
                self.succeeded();
                self.set_status("streaming", None);
            }
            Ok(None) => {
                self.close();
                self.set_status("idle", None);
            }
            Err(words) => {
                self.close();
                self.fail(words);
            }
        }
    }

    /// Connects, resolves the lights and sends the start sequence. `Ok(None)`: no longer wanted by the time it was ready.
    fn open(&mut self) -> Result<Option<Live>, String> {
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
            return Ok(None);
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
        Ok(Some(Live {
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
}
