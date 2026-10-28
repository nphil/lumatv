//! Cameras and their event pop-ups through Home Assistant's **LumaShow integration** (`docs/research/ha-cameras-result.md`).
//!
//! The TV is one more LumaShow device to Home Assistant: the integration dials the TV (`ha-bridge`, the Show's own crate, TCP
//! `cameras.port`, announced as `_lumashow._tcp`), encrypted with a key derived from the TV's ESPHome pairing key, which is the
//! Bluetooth proxy's (`ble-link`; the TV holds no Home Assistant token and no address). Home Assistant pushes the cameras the owner
//! shared with this TV (`cameras.list`: names and signed picture addresses) and, with *Send camera events* on, what their detectors
//! see (`camera_events.event`: a person, a vehicle, an animal, a package, a doorbell ring, motion).
//!
//! What lives here:
//!
//! - the **looker** thread: with `cameras.enabled` and a pairing key the bridge listens and is announced; without either nothing
//!   listens, nothing is announced and nothing is fetched (the switch is off by default until the TV check passes);
//! - the **`camera_events` module** ([`ModuleHandler`]): the integration's events, through the pop-up policy ([`Popups`]: which kinds,
//!   a cooldown per camera and kind, a doorbell never held back), to the hub as the event `cameras.event` and the retained topic
//!   `cameras.popup` (what the shell shows now, with when it ends);
//! - the **picture route** `GET /v1/cameras/{id}/frame?w=&h=`: the camera's current picture, fetched from the signed address the
//!   integration gave (home network only, no token, as the Show fetches it), decoded by the hardware JPEG block through `images`
//!   when it is there, held `frameHoldMs` so a grid of tiles costs one fetch a camera. Nothing is fetched unless the shell asks, so a
//!   hidden pop-up or viewer costs nothing. No picture is written anywhere: the newest one of each camera is in memory until the next.
//!
//! The topic `cameras` says where it stands (enabled, paired, the link's state, the cameras by id and name with the kinds they can
//! announce) and never a picture address, a stream address or a key.

use crate::config::CamerasConfig;
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::images::{Images, wanted_size};
use crate::sys;
use crate::timeutil::now_ms;
use ha_bridge::modules::{ModuleHandler, ModuleOut};
use ha_bridge::wire::{Display, Has};
use ha_bridge::{Bridge, Config as BridgeConfig, DeviceInfo, KeySource};
use ha_link::picture::{Limits, fetch_signed};
use ha_link::{AddressPolicy, Notice, Sink, StateBatch, Status};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The retained topic: where the bridge stands and which cameras there are.
pub const TOPIC: &str = "cameras";
/// The retained topic: the pop-up the shell shows now (`null` when none).
pub const POPUP_TOPIC: &str = "cameras.popup";
/// The event (no retained value) raised for every accepted camera event.
pub const EVENT_TOPIC: &str = "cameras.event";
/// The retained topic: the live session on the video plane (`live::View`).
pub const LIVE_TOPIC: &str = "cameras.live";
/// The module of the protocol this listens to (`lumashow-ha` `docs/protocol.md` 10b).
pub const MODULE: &str = "camera_events";
/// The kinds an event may be, in the order a list shows them.
pub const KINDS: [&str; 6] = [
    "person", "vehicle", "animal", "package", "doorbell", "motion",
];
/// How long a fetch of one picture may take in all.
const FETCH_WITHIN: Duration = Duration::from_secs(4);
/// How often the looker looks at the switch, the key and the address when nothing happens.
const LOOK_EVERY: Duration = Duration::from_secs(10);

/// What the runner acts through; a test names the loopback and its own key.
pub struct Deps {
    /// The TV's pairing key now (`None`: not paired, nobody is let in).
    pub key: KeySource,
    /// Where to listen. `None` is the TV's private LAN address.
    pub bind: Option<Ipv4Addr>,
    /// Whether to announce the bridge with multicast DNS (a test does not).
    pub announce: bool,
    /// How picture addresses are judged (the home network only; a test allows the loopback).
    pub policy: AddressPolicy,
    pub tls: Option<fetch::TlsFetch>,
    /// What the TV says it is.
    pub device: DeviceInfo,
    /// The feeder for the live path; `None` is the config's choice (`cameras.live` with the staged feeder), a test names its own.
    pub feeder: Option<crate::live::FeederSpec>,
}

impl Deps {
    /// The real ones: the Bluetooth proxy's link holds the key, the LAN address is found, the announcement is made.
    pub fn real(ctx: &Ctx) -> Self {
        let data_dir = ctx.paths.data_dir.clone();
        let key: KeySource = Arc::new(move || {
            crate::ble_store::load(&data_dir)
                .ok()
                .flatten()
                .map(|link| link.key)
        });
        Self {
            key,
            bind: None,
            announce: true,
            policy: AddressPolicy::HOME,
            tls: crate::voice::tls_for(&ctx.config.fetch),
            device: device_info("LumaTV", "LG TV", env!("CARGO_PKG_VERSION")),
            feeder: None,
        }
    }
}

/// What the TV says it is to the integration (its device page there is made from it). The name is filled in with the MAC once the
/// address is known (`lumatv-<six hex digits>`, as the Show's `lumashow-<six>`).
pub fn device_info(friendly: &str, model: &str, version: &str) -> DeviceInfo {
    DeviceInfo {
        name: "lumatv".to_owned(),
        friendly_name: friendly.to_owned(),
        mac: String::new(),
        manufacturer: "LumaTV".to_owned(),
        model: model.to_owned(),
        sw_version: version.to_owned(),
        display: Display {
            width: 1920,
            height: 1080,
        },
        has: Has {
            camera: false,
            microphone: false,
            speaker: true,
        },
    }
}

// -- the pop-up policy ---------------------------------------------------------------------------------------------------------

/// One event as the integration said it.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraEvent {
    pub camera: String,
    pub kind: String,
    /// Seconds since the epoch of the detection.
    pub at: f64,
    pub source: String,
    pub label: Option<String>,
}

impl CameraEvent {
    /// Reads a `camera_events.event` body; `None` when it does not name a camera and a kind the TV knows.
    pub fn parse(body: &Map<String, Value>) -> Option<Self> {
        let camera = body.get("camera")?.as_str()?;
        let kind = body.get("kind")?.as_str()?;
        if !plain_id(camera) || !KINDS.contains(&kind) {
            return None;
        }
        Some(Self {
            camera: camera.to_owned(),
            kind: kind.to_owned(),
            at: body.get("at").and_then(Value::as_f64).unwrap_or(0.0),
            source: body
                .get("source")
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 200)
                .unwrap_or("")
                .to_owned(),
            label: body
                .get("label")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(64).collect()),
        })
    }
}

/// A camera's entity id as it goes into a path and a topic: `camera.` and lowercase letters, digits and underscores.
pub fn plain_id(id: &str) -> bool {
    id.len() <= 200
        && id.strip_prefix("camera.").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// Why an event made no pop-up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held {
    /// Its kind is not one the owner wants a pop-up for.
    Kind,
    /// The same camera and kind popped up a moment ago.
    Cooldown,
}

/// A pop-up the shell is to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub id: u64,
    pub event: CameraEvent,
    /// How long it stays without a key press.
    pub lasts: Duration,
}

/// The rules between an event and a pop-up: which kinds, and a cooldown per camera and kind. A doorbell ring is never held back and
/// stays half as long again: somebody is waiting at the door.
pub struct Popups {
    kinds: BTreeSet<String>,
    lasts: Duration,
    cooldown: Duration,
    last: HashMap<(String, String), Instant>,
    next_id: u64,
}

impl Popups {
    pub fn new(config: &CamerasConfig) -> Self {
        Self {
            kinds: config
                .popup_kinds
                .iter()
                .filter(|k| KINDS.contains(&k.as_str()))
                .cloned()
                .collect(),
            lasts: Duration::from_secs(u64::from(config.popup_seconds.clamp(3, 120))),
            cooldown: Duration::from_secs(u64::from(config.cooldown_seconds.min(3600))),
            last: HashMap::new(),
            next_id: 0,
        }
    }

    /// What the owner wants a pop-up for.
    pub fn kinds(&self) -> Vec<String> {
        KINDS
            .iter()
            .filter(|k| self.kinds.contains(**k))
            .map(|k| (*k).to_owned())
            .collect()
    }

    pub fn decide(&mut self, event: CameraEvent, now: Instant) -> Result<Popup, Held> {
        if !self.kinds.contains(&event.kind) {
            return Err(Held::Kind);
        }
        let doorbell = event.kind == "doorbell";
        let key = (event.camera.clone(), event.kind.clone());
        if !doorbell
            && let Some(last) = self.last.get(&key)
            && now.duration_since(*last) < self.cooldown
        {
            return Err(Held::Cooldown);
        }
        self.last.insert(key, now);
        self.next_id += 1;
        Ok(Popup {
            id: self.next_id,
            event,
            lasts: if doorbell {
                self.lasts + self.lasts / 2
            } else {
                self.lasts
            },
        })
    }
}

// -- shared state ----------------------------------------------------------------------------------------------------------------

/// What the integration said of the link.
#[derive(Clone, Debug, Default, PartialEq)]
struct LinkView {
    /// `off`, `unpaired`, `waiting`, `connected`, `problem`.
    state: &'static str,
    detail: String,
    entities: usize,
}

struct Frame {
    bytes: Vec<u8>,
    content_type: String,
    taken: Instant,
}

#[derive(Default)]
struct State {
    bridge: Option<Arc<Bridge>>,
    announced: Option<(mdns::Mdns, Ipv4Addr)>,
    link: LinkView,
    /// The kinds each camera can announce, as `camera_events.list` said.
    kinds: BTreeMap<String, Vec<String>>,
    problem: Option<String>,
    popup: Option<(Popup, Instant)>,
}

struct Inner {
    hub: Arc<Hub>,
    config: CamerasConfig,
    deps: Deps,
    images: Arc<Images>,
    state: Mutex<State>,
    popups: Mutex<Popups>,
    /// One slot a camera: a tile and the pop-up asking at once share one fetch.
    frames: Mutex<HashMap<String, Arc<Mutex<Option<Frame>>>>>,
    /// The live path (`live/`): the full view's stream on the video plane.
    live: crate::live::Live,
}

/// The handle the API and the looker share.
#[derive(Clone)]
pub struct Cameras {
    inner: Arc<Inner>,
}

impl Cameras {
    pub fn new(ctx: &Ctx, deps: Deps, images: Arc<Images>) -> Self {
        let config = ctx.config.cameras.clone();
        // the feeder: a test's own, else the staged one when the live switch is on
        let feeder = deps.feeder.clone().or_else(|| {
            (config.enabled && config.live).then(|| {
                let mut spec = crate::live::default_spec(
                    &ctx.paths.data_dir,
                    &config.feeder_window,
                    &config.feeder_library,
                );
                if !config.feeder.is_empty() {
                    spec.program = std::path::PathBuf::from(&config.feeder);
                }
                // the window is announced under an id of the brand's (as the saver's player is), never GeForce NOW's
                if let Some(app) = &ctx.config.brand_app_id {
                    spec.extra_args = vec!["-a".to_owned(), format!("{app}.cameras")];
                }
                spec
            })
        });
        let hub = ctx.hub.clone();
        let live = crate::live::Live::new(
            crate::live::Deps {
                feeder,
                policy: deps.policy,
            },
            Box::new(move |view| {
                hub.set(LIVE_TOPIC, view);
            }),
        );
        let inner = Arc::new(Inner {
            hub: ctx.hub.clone(),
            popups: Mutex::new(Popups::new(&config)),
            config,
            deps,
            images,
            state: Mutex::new(State::default()),
            frames: Mutex::new(HashMap::new()),
            live,
        });
        let cameras = Self { inner };
        cameras.inner.hub.set(POPUP_TOPIC, Value::Null);
        cameras.publish();
        cameras
    }

    /// Starts the looker: it keeps the bridge running while the switch is on and the TV is paired, and stops it when either goes.
    pub fn spawn(&self, ctx: &Ctx) -> io::Result<JoinHandle<()>> {
        let me = self.clone();
        let stop = ctx.stop.clone();
        thread::Builder::new()
            .name("cameras".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || {
                loop {
                    me.look();
                    if stop.sleep(LOOK_EVERY) {
                        break;
                    }
                }
                me.shut_down();
            })
    }

    /// One look: what should run runs, and the topic says where it stands.
    pub fn look(&self) {
        let inner = &self.inner;
        let key = (inner.deps.key)();
        let want = inner.config.enabled && key.is_some();
        {
            let mut state = inner.state.lock();
            if !want {
                if state.bridge.is_some() || state.announced.is_some() {
                    crate::log_info!("cameras: the bridge stops");
                }
                self.stop_bridge(&mut state);
                state.link = if !inner.config.enabled {
                    LinkView {
                        state: "off",
                        ..Default::default()
                    }
                } else {
                    LinkView {
                        state: "unpaired",
                        detail: "Pair the TV with Home Assistant first (ble-link)".to_owned(),
                        ..Default::default()
                    }
                };
                state.problem = None;
            } else if state.bridge.is_none() {
                match self.start_bridge() {
                    Ok(bridge) => {
                        crate::log_info!("cameras: the bridge listens at {}", bridge.local_addr());
                        state.bridge = Some(bridge);
                        state.problem = None;
                        state.link = LinkView {
                            state: "waiting",
                            detail: "Waiting for Home Assistant's LumaShow integration".to_owned(),
                            ..Default::default()
                        };
                    }
                    Err(why) => {
                        if state.problem.as_deref() != Some(why.as_str()) {
                            crate::log_warn!("cameras: {why}");
                        }
                        state.problem = Some(why);
                    }
                }
            }
            self.keep_announced(&mut state);
            self.expire_popup(&mut state);
        }
        self.publish();
    }

    fn start_bridge(&self) -> Result<Arc<Bridge>, String> {
        let inner = &self.inner;
        let ip = sys::lan_ipv4(inner.deps.bind)
            .ok_or_else(|| "the TV has no private network address".to_owned())?;
        let mut device = inner.deps.device.clone();
        if let Some(mac) = crate::ble::interface_mac(ip) {
            let six: String = mac
                .chars()
                .filter(char::is_ascii_hexdigit)
                .collect::<String>()
                .to_ascii_lowercase();
            device.name = format!("lumatv-{}", &six[six.len().saturating_sub(6)..]);
            device.mac = mac;
        }
        let hub = inner.hub.clone();
        let log = Arc::new(move |line: String| {
            crate::log_info!("cameras: {line}");
            let _ = &hub;
        });
        let config = BridgeConfig {
            bind: format!("{ip}:{}", inner.config.port),
            key: inner.deps.key.clone(),
            device,
            timing: ha_bridge::Timing::default(),
            log,
        };
        let sink = Box::new(LinkSink {
            cameras: self.clone(),
        });
        let bridge = Arc::new(Bridge::start(config, sink).map_err(|e| e.to_string())?);
        let handler: Arc<dyn ModuleHandler> = Arc::new(EventsModule {
            cameras: self.clone(),
            out: Mutex::new(None),
        });
        bridge
            .register_module(MODULE, handler)
            .map_err(|e| e.to_string())?;
        if inner.live.available() {
            let live: Arc<dyn ModuleHandler> = Arc::new(LiveModule {
                live: inner.live.clone(),
            });
            bridge
                .register_module(crate::live::MODULE, live)
                .map_err(|e| e.to_string())?;
        }
        Ok(bridge)
    }

    fn stop_bridge(&self, state: &mut State) {
        self.inner.live.stop("the bridge stopped");
        if let Some(bridge) = state.bridge.take() {
            bridge.stop();
        }
        if let Some((mdns, _)) = state.announced.take() {
            drop(mdns);
        }
        state.kinds.clear();
        self.inner.frames.lock().clear();
    }

    /// Announces the bridge at the TV's address while it listens (the address can move: a changed one is announced anew).
    fn keep_announced(&self, state: &mut State) {
        let inner = &self.inner;
        let Some(bridge) = state.bridge.clone() else {
            return;
        };
        if !inner.deps.announce {
            return;
        }
        let ip = match bridge.local_addr().ip() {
            std::net::IpAddr::V4(ip) => ip,
            std::net::IpAddr::V6(_) => return,
        };
        if state.announced.as_ref().is_some_and(|(_, at)| *at == ip) {
            return;
        }
        state.announced = None;
        let device = &inner.deps.device;
        let mac = crate::ble::interface_mac(ip).unwrap_or_default();
        let service = service_of(device, &mac, bridge.local_addr().port());
        match mdns::Mdns::start_udp(ip) {
            Ok(mdns) => {
                mdns.add(service);
                state.announced = Some((mdns, ip));
            }
            Err(e) => {
                let why = format!("mDNS: {e}");
                if state.problem.as_deref() != Some(why.as_str()) {
                    crate::log_warn!("cameras: {why}");
                }
                state.problem = Some(why);
            }
        }
    }

    fn shut_down(&self) {
        let mut state = self.inner.state.lock();
        self.stop_bridge(&mut state);
    }

    fn expire_popup(&self, state: &mut State) {
        if let Some((popup, since)) = &state.popup
            && since.elapsed() >= popup.lasts
        {
            state.popup = None;
            self.inner.hub.set(POPUP_TOPIC, Value::Null);
        }
    }

    /// `GET /v1/cameras`: the topic's value, now.
    pub fn status(&self) -> Value {
        self.view()
    }

    fn view(&self) -> Value {
        let inner = &self.inner;
        let state = inner.state.lock();
        let mut cameras: Vec<Value> = state
            .bridge
            .as_ref()
            .map(|b| b.cameras())
            .unwrap_or_default()
            .into_iter()
            .map(|rec| {
                json!({
                    "id": rec.id,
                    "name": rec.name,
                    "area": rec.area,
                    "picture": rec.snapshot.is_some(),
                    "stream": rec.streams.iter().any(|s| s.kind == "rtsp"),
                    "kinds": state.kinds.get(&rec.id).cloned().unwrap_or_default(),
                })
            })
            .collect();
        cameras.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        json!({
            "enabled": inner.config.enabled,
            "paired": (inner.deps.key)().is_some(),
            "port": state.bridge.as_ref().map(|b| b.local_addr().port()),
            "state": state.link.state,
            "detail": state.link.detail,
            "entities": state.link.entities,
            "problem": state.problem,
            "popupKinds": inner.popups.lock().kinds(),
            "popupSeconds": inner.config.popup_seconds,
            "live": inner.live.available(),
            "cameras": cameras,
        })
    }

    fn publish(&self) {
        let view = self.view();
        self.inner.hub.set(TOPIC, view);
    }

    fn set_link(&self, link: LinkView) {
        {
            let mut state = self.inner.state.lock();
            if state.link == link {
                return;
            }
            state.link = link;
        }
        self.publish();
    }

    /// An event from the integration (or a test): through the policy, then to the shell.
    pub fn event(&self, event: CameraEvent, test: bool) -> Result<Popup, Held> {
        let name = {
            let state = self.inner.state.lock();
            state
                .bridge
                .as_ref()
                .and_then(|b| b.cameras().into_iter().find(|c| c.id == event.camera))
                .map(|c| c.name)
        };
        let now = Instant::now();
        let popup = self.inner.popups.lock().decide(event, now)?;
        let until = now_ms() + popup.lasts.as_millis() as u64;
        let value = json!({
            "id": popup.id,
            "camera": popup.event.camera,
            "name": name.unwrap_or_else(|| popup.event.camera.clone()),
            "kind": popup.event.kind,
            "label": popup.event.label,
            "at": popup.event.at,
            "source": popup.event.source,
            "lastsMs": popup.lasts.as_millis() as u64,
            "untilMs": until,
            "test": test,
        });
        self.inner.state.lock().popup = Some((popup.clone(), now));
        self.inner.hub.emit(EVENT_TOPIC, "event", value.clone());
        self.inner.hub.set(POPUP_TOPIC, value);
        Ok(popup)
    }

    /// `POST /v1/cameras/popup/clear`: the shell dismissed or expanded it; other readers see it go.
    pub fn clear_popup(&self) {
        self.inner.state.lock().popup = None;
        self.inner.hub.set(POPUP_TOPIC, Value::Null);
    }

    /// `GET /v1/cameras/{id}/frame?w=&h=`
    pub fn frame(&self, request: &Request, id: &str) -> Response {
        if !plain_id(id) {
            return error_response(400, "bad_request", "a camera is camera.<name>");
        }
        let (want_w, want_h) = match wanted_size(request) {
            Ok(size) => size,
            Err(r) => return r,
        };
        let inner = &self.inner;
        let snapshot = {
            let state = inner.state.lock();
            match &state.bridge {
                None => {
                    return error_response(
                        503,
                        "not_connected",
                        "the camera bridge is not running",
                    );
                }
                Some(bridge) => bridge.snapshot_of(id),
            }
        };
        let Some(snapshot) = snapshot else {
            return error_response(
                404,
                "no_such_camera",
                "Home Assistant shares no such camera with this TV",
            );
        };
        let slot = inner
            .frames
            .lock()
            .entry(id.to_owned())
            .or_default()
            .clone();
        let mut slot = slot.lock();
        let hold = Duration::from_millis(u64::from(inner.config.frame_hold_ms));
        let fresh = slot.as_ref().is_some_and(|f| f.taken.elapsed() < hold);
        if !fresh {
            let fetched = fetch_signed(
                &snapshot.url,
                inner.deps.policy,
                ha_link::Timing::default(),
                inner.deps.tls.clone(),
                Limits {
                    total: FETCH_WITHIN,
                    max_bytes: inner.config.max_picture_bytes as usize,
                },
            );
            match fetched {
                Ok(picture) => {
                    *slot = Some(Frame {
                        bytes: picture.bytes,
                        content_type: picture.content_type,
                        taken: Instant::now(),
                    });
                }
                Err(why) => {
                    // An old picture is better than none while the camera is slow; none at all is said.
                    if slot.is_none() {
                        return error_response(502, "no_picture", &why);
                    }
                }
            }
        }
        let frame = slot.as_ref().expect("a frame is there");
        let age = frame.taken.elapsed().as_millis() as u64;
        let response = if frame.content_type == "image/jpeg" {
            inner.images.serve_jpeg(frame.bytes.clone(), want_w, want_h)
        } else {
            Response {
                status: 200,
                headers: vec![("Content-Type", frame.content_type.clone())],
                body: frame.bytes.clone(),
            }
        };
        response
            .with_header("Cache-Control", "no-store")
            .with_header("X-Luma-Camera-Age-Ms", age.to_string())
    }

    /// `POST /v1/cameras/live {camera}`: the full view asks for the camera's stream on the video plane.
    pub fn live_start(&self, body: &Value) -> Response {
        let camera = body.get("camera").and_then(Value::as_str).unwrap_or("");
        match self.inner.live.start(camera) {
            Ok(()) => Response::json(
                200,
                &json!({"ok": true, "live": self.inner.live.view().json()}),
            ),
            Err(why) => error_response(409, "not_now", &why),
        }
    }

    /// `POST /v1/cameras/live/stop`: the full view closed.
    pub fn live_stop(&self) -> Response {
        self.inner.live.stop("the viewer closed");
        Response::json(
            200,
            &json!({"ok": true, "live": self.inner.live.view().json()}),
        )
    }

    /// `POST /v1/cameras/event`: an event as the integration would send it, for the TV check and the shell's tests.
    pub fn test_event(&self, body: &Value) -> Response {
        let Some(object) = body.as_object() else {
            return error_response(400, "bad_request", "a JSON object");
        };
        let mut body = object.clone();
        body.entry("source".to_owned())
            .or_insert_with(|| Value::String("test".to_owned()));
        let Some(event) = CameraEvent::parse(&body) else {
            return error_response(
                400,
                "bad_request",
                "camera (camera.<name>) and kind (person, vehicle, animal, package, doorbell, motion)",
            );
        };
        match self.event(event, true) {
            Ok(popup) => Response::json(
                200,
                &json!({"ok": true, "popup": popup.id, "lastsMs": popup.lasts.as_millis() as u64}),
            ),
            Err(Held::Kind) => Response::json(200, &json!({"ok": false, "held": "kind"})),
            Err(Held::Cooldown) => Response::json(200, &json!({"ok": false, "held": "cooldown"})),
        }
    }
}

/// The multicast DNS service Home Assistant's discovery looks for, as the Show announces it.
fn service_of(device: &DeviceInfo, mac: &str, port: u16) -> mdns::Service {
    let six: String = mac
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase();
    let records = [
        ("mac", six.clone()),
        ("proto", ha_bridge::wire::PROTO.to_string()),
        ("friendly_name", device.friendly_name.clone()),
        ("project_name", format!("LumaShow.{}", device.model)),
        ("project_version", device.sw_version.clone()),
        (
            "modules",
            ha_bridge::wire::module::BUILT_IN
                .iter()
                .map(|(name, _)| *name)
                .chain([MODULE])
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "api_encryption",
            "Noise_NNpsk0_25519_ChaChaPoly_SHA256".to_owned(),
        ),
    ];
    mdns::Service {
        instance: if six.len() >= 6 {
            format!("lumatv-{}", &six[six.len() - 6..])
        } else {
            device.name.clone()
        },
        kind: ha_bridge::wire::SERVICE.into(),
        port,
        txt: records
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    }
}

// -- what the bridge tells ---------------------------------------------------------------------------------------------------------

/// The bridge's news: only where the link stands is kept (the entities the integration shares are the HA plugin's business, through
/// its own lease, not this module's).
struct LinkSink {
    cameras: Cameras,
}

impl Sink for LinkSink {
    fn status(&mut self, status: Status) {
        let link = match status {
            Status::Waiting | Status::Connecting => LinkView {
                state: "waiting",
                detail: "Waiting for Home Assistant's LumaShow integration".to_owned(),
                entities: 0,
            },
            Status::Bridged {
                entities,
                ha_version,
                ..
            } => LinkView {
                state: "connected",
                detail: format!("Connected via LumaShow integration (Home Assistant {ha_version})"),
                entities,
            },
            Status::Connected { version } => LinkView {
                state: "connected",
                detail: format!("Connected to Home Assistant {version}"),
                entities: 0,
            },
            Status::AuthInvalid => LinkView {
                state: "problem",
                detail: "Home Assistant's key is not this TV's: pair again".to_owned(),
                entities: 0,
            },
            Status::Offline { reason } => LinkView {
                state: "waiting",
                detail: reason,
                entities: 0,
            },
            Status::Stopped => LinkView {
                state: "off",
                ..Default::default()
            },
        };
        self.cameras.set_link(link);
    }

    fn states(&mut self, _batch: StateBatch) {
        // The camera list lives in the bridge's mirror; it is read when the topic is published.
        self.cameras.publish();
    }

    fn notices(&mut self, _notices: Vec<Notice>) {}
}

/// The `camera_live` module: the stream the integration gives for the full view (`live/`).
struct LiveModule {
    live: crate::live::Live,
}

impl ModuleHandler for LiveModule {
    fn on_session(&self, up: bool, out: Arc<dyn ModuleOut>) {
        self.live.module_session(up, out);
    }

    fn on_message(&self, kind: &str, body: &Map<String, Value>) {
        let op = body.get("op").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "started" => {
                let camera = body.get("camera").and_then(Value::as_str).unwrap_or("");
                let url = body.get("url").and_then(Value::as_str).unwrap_or("");
                let kind = body.get("kind").and_then(Value::as_str).unwrap_or("hls");
                if plain_id(camera) && !url.is_empty() {
                    self.live.started(op, camera, url, kind);
                }
            }
            "failed" => self.live.failed(
                op,
                body.get("code").and_then(Value::as_str).unwrap_or("failed"),
                body.get("message").and_then(Value::as_str).unwrap_or(""),
            ),
            _ => {}
        }
    }
}

/// The `camera_events` module: offered to the integration while the bridge runs; its events become pop-ups.
struct EventsModule {
    cameras: Cameras,
    out: Mutex<Option<Arc<dyn ModuleOut>>>,
}

impl ModuleHandler for EventsModule {
    fn on_session(&self, up: bool, out: Arc<dyn ModuleOut>) {
        if up {
            // The integration lists the cameras' kinds when the session begins; asking again costs nothing and covers a missed one.
            out.send("refresh", json!({}));
            *self.out.lock() = Some(out);
        } else {
            *self.out.lock() = None;
            self.cameras.inner.state.lock().kinds.clear();
            self.cameras.publish();
        }
    }

    fn on_message(&self, kind: &str, body: &Map<String, Value>) {
        match kind {
            "list" => {
                let mut kinds = BTreeMap::new();
                for camera in body
                    .get("cameras")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(id) = camera.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if !plain_id(id) {
                        continue;
                    }
                    let list: Vec<String> = camera
                        .get("kinds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|k| KINDS.contains(k))
                        .map(str::to_owned)
                        .collect();
                    kinds.insert(id.to_owned(), list);
                }
                self.cameras.inner.state.lock().kinds = kinds;
                self.cameras.publish();
            }
            "event" => {
                if let Some(event) = CameraEvent::parse(body) {
                    let _ = self.cameras.event(event, false);
                }
            }
            _ => {}
        }
    }
}

/// Deps for tests: no key (nothing listens) or a given key on the loopback, pictures allowed from the loopback.
pub mod testing {
    use super::*;

    /// Nothing is paired: the bridge never starts.
    pub fn inert_deps() -> Deps {
        Deps {
            key: Arc::new(|| None),
            bind: Some(Ipv4Addr::LOCALHOST),
            announce: false,
            policy: AddressPolicy::TESTING,
            tls: None,
            device: device_info("LumaTV", "test", "0.0.0"),
            feeder: None,
        }
    }

    /// Paired with `key`, listening on the loopback, not announced.
    pub fn keyed_deps(key: [u8; 32]) -> Deps {
        Deps {
            key: Arc::new(move || Some(key)),
            ..inert_deps()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(kinds: &[&str], cooldown: u32) -> CamerasConfig {
        CamerasConfig {
            popup_kinds: kinds.iter().map(|k| (*k).to_owned()).collect(),
            cooldown_seconds: cooldown,
            popup_seconds: 12,
            ..CamerasConfig::default()
        }
    }

    fn event(camera: &str, kind: &str) -> CameraEvent {
        CameraEvent {
            camera: camera.to_owned(),
            kind: kind.to_owned(),
            at: 1.0,
            source: "binary_sensor.x".to_owned(),
            label: None,
        }
    }

    #[test]
    fn a_kind_the_owner_does_not_want_makes_no_popup_and_a_doorbell_stays_longer() {
        let mut popups = Popups::new(&config(&["person", "doorbell"], 30));
        assert_eq!(popups.kinds(), ["person", "doorbell"]);
        let now = Instant::now();
        assert_eq!(
            popups.decide(event("camera.a", "vehicle"), now),
            Err(Held::Kind)
        );
        let person = popups.decide(event("camera.a", "person"), now).unwrap();
        assert_eq!((person.id, person.lasts), (1, Duration::from_secs(12)));
        let bell = popups.decide(event("camera.a", "doorbell"), now).unwrap();
        assert_eq!((bell.id, bell.lasts), (2, Duration::from_secs(18)));
    }

    #[test]
    fn the_same_camera_and_kind_are_held_for_the_cooldown_but_another_camera_or_a_doorbell_is_not()
    {
        let mut popups = Popups::new(&config(&["person", "vehicle", "doorbell"], 30));
        let t0 = Instant::now();
        assert!(popups.decide(event("camera.a", "person"), t0).is_ok());
        assert_eq!(
            popups.decide(event("camera.a", "person"), t0 + Duration::from_secs(5)),
            Err(Held::Cooldown)
        );
        assert!(
            popups
                .decide(event("camera.b", "person"), t0 + Duration::from_secs(5))
                .is_ok()
        );
        assert!(
            popups
                .decide(event("camera.a", "vehicle"), t0 + Duration::from_secs(5))
                .is_ok()
        );
        assert!(
            popups
                .decide(event("camera.a", "person"), t0 + Duration::from_secs(30))
                .is_ok()
        );
        assert!(popups.decide(event("camera.a", "doorbell"), t0).is_ok());
        assert!(
            popups
                .decide(event("camera.a", "doorbell"), t0 + Duration::from_secs(1))
                .is_ok()
        );
    }

    #[test]
    fn an_event_is_read_as_the_protocol_writes_it_and_a_strange_one_is_not() {
        let body: Map<String, Value> = serde_json::from_str(
            r#"{"camera":"camera.exterior_front_door_camera","kind":"animal","at":1793160000.5,"source":"event.kestrel_front_door_camera_animal","label":"Domestic Cat"}"#,
        )
        .unwrap();
        let event = CameraEvent::parse(&body).unwrap();
        assert_eq!(event.camera, "camera.exterior_front_door_camera");
        assert_eq!(
            (event.kind.as_str(), event.label.as_deref()),
            ("animal", Some("Domestic Cat"))
        );
        for bad in [
            r#"{"camera":"light.a","kind":"person"}"#,
            r#"{"camera":"camera.a","kind":"ghost"}"#,
            r#"{"camera":"camera.a/../x","kind":"person"}"#,
            r#"{"kind":"person"}"#,
        ] {
            let body: Map<String, Value> = serde_json::from_str(bad).unwrap();
            assert!(CameraEvent::parse(&body).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_announcement_names_the_tv_by_its_mac_and_lists_the_events_module() {
        let device = device_info("LumaTV", "LG C4", "0.4.0");
        let service = service_of(&device, "AA:BB:CC:DD:EE:FF", 6054);
        assert_eq!(service.instance, "lumatv-ddeeff");
        assert_eq!(service.kind, "_lumashow._tcp");
        let txt: BTreeMap<_, _> = service.txt.iter().cloned().collect();
        assert_eq!(txt["mac"], "aabbccddeeff");
        assert_eq!(
            txt["modules"],
            "states,actions,cameras,notify,camera_events"
        );
        assert_eq!(txt["project_name"], "LumaShow.LG C4");
    }
}
