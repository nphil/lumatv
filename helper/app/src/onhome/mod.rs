//! On Home: the lights, switches, scenes, sensors, thermostats, covers and players of the owner's Home Assistant on LumaTV's Home, live, and
//! the OK key to run them.
//!
//! **The TV holds nothing of Home Assistant's.** It is one more LumaShow device to Home Assistant's `lumashow` integration (the TV's entry,
//! *LumaTV*): the integration dials the TV (TCP `onhome.port`, 6054, announced as `_lumashow._tcp`), encrypted with a key derived from the
//! TV's pairing key (`ble-link`: the Bluetooth proxy's), and pushes **what the owner shared with this TV** in Home Assistant (the entry's
//! *Configure*: pick by area, device, type or entity) and, for the entities marked controllable there, accepts the TV's calls. The TV has
//! no address, no token, no way to ask Home Assistant for anything that was not shared, and no way to widen what is. Which entities are shared,
//! and which of them the TV may call, are decided in Home Assistant; this module narrows further (nothing is called on a lock, an
//! alarm, a siren, a camera or a valve, a garage door or a gate, and a switch, script or scene that unlocks or disarms is refused: `tile::plan`).
//! Cameras are not carried (the `cameras` module is not offered): the TV's cameras come through Scrypted.
//!
//! What the TV shows is the owner's choice among what is shared: all of it (until the owner chooses) or a list kept in
//! `<data dir>/onhome/picks.json`, written by Settings › Home & look › Home Assistant › On Home.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/onhome` | the same document as the `onhome` topic |
//! | `GET /v1/onhome/shared?q=&area=&group=&offset=&limit=` | what Home Assistant shares, for the picker: rows, the groups and areas with their counts |
//! | `POST /v1/onhome/picks` | the owner's list: `{"show": id}`, `{"hide": id}`, `{"up": id}`, `{"down": id}`, `{"all": true}` (owner only) |
//! | `POST /v1/onhome/act` | `{"id", "verb", "value"?}`: one control of one tile (`tile::plan` says which are possible) |
//!
//! It costs nothing until the Home Assistant plugin is switched on (`ha_plugin`) and a pairing key is there: no listener, no socket, no
//! announcement, and the routes but the topic's refuse. Off, the looker sleeps until the switch wakes it.

mod machine;
pub mod history;
pub mod mirror;
pub mod tile;

use crate::API_VERSION;
use crate::config::OnHomeConfig;
use crate::ctx::Ctx;
use crate::ha_plugin;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::plugins::session::Shared;
use crate::sys;
use crate::timeutil::now_ms;
use ha_bridge::wire::{DeviceInfo, Display, Has};
use machine::{Bridge, Call, Config as BridgeConfig, KeySource};
use mirror::Mirror;
use parking_lot::{Condvar, Mutex};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::thread::JoinHandle;
use std::time::Duration;
use tile::{GROUPS, Group, MAX_PICKS, Reach, Refused};

/// The retained topic.
pub const TOPIC: &str = "onhome";

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/onhome", "On Home: where the link to Home Assistant stands and the tiles, as the topic says"),
    ("GET", "/v1/onhome/shared?q=&area=&group=&offset=&limit=", "what Home Assistant shares with the TV, for the picker"),
    ("GET", "/v1/onhome/history?id=&range=", "the history of one shown sensor from Home Assistant's recorder (range 1h, 6h, 24h, 7d or 30d): a curve with its extremes and average, on/off bands, or the recent changes"),
    ("POST", "/v1/onhome/picks", "what Home shows and how: show, hide, move, size, up, down or all (the owner's profile only)"),
    ("POST", "/v1/onhome/act", "one control of one tile: {id, verb, value?}; 409 says why not in words; a lock only locks here, never unlocks"),
    ("POST", "/v1/onhome/unlock", "unlock one lock: the TV's own shell only ({id, token}: the token is in a file only the shell reads, and the peer must be the loopback)"),
];

/// How often the looker looks at the switch, the key and the address when nothing happens.
const LOOK_EVERY: Duration = Duration::from_secs(10);
/// How long a history that was asked for is served again without asking Home Assistant.
const HISTORY_KEPT: Duration = Duration::from_secs(60);
/// How long a control may take in all (the integration answers within 15 s).
const ACT_WITHIN: Duration = Duration::from_secs(17);

/// What the runner acts through; a test names the loopback and its own key.
pub struct Deps {
    pub key: KeySource,
    /// Where to listen. `None` is the TV's private LAN address.
    pub bind: Option<Ipv4Addr>,
    /// Whether to announce the listener with multicast DNS (a test does not).
    pub announce: bool,
    pub device: DeviceInfo,
    /// Where a sensor's history is asked: the saved Home Assistant link, the addresses it may be dialled at and the TLS helper.
    pub history: history::Source,
}

impl Deps {
    pub fn real(ctx: &Ctx) -> Self {
        let data_dir = ctx.paths.data_dir.clone();
        // The pairing key is only read: the BLE proxy and the TV sensors own it and run on their own switches, whatever this plugin's.
        let key: KeySource = Arc::new(move || crate::ble_store::load(&data_dir).ok().flatten().map(|link| link.key));
        Self { key, bind: None, announce: true, device: device_info("LumaTV", "LG TV", env!("CARGO_PKG_VERSION")), history: history::Source::real(ctx) }
    }
}

/// What the TV says it is to the integration (its device page there is made from it).
pub fn device_info(friendly: &str, model: &str, version: &str) -> DeviceInfo {
    DeviceInfo {
        name: "lumatv".to_owned(),
        friendly_name: friendly.to_owned(),
        mac: String::new(),
        manufacturer: "LumaTV".to_owned(),
        model: model.to_owned(),
        sw_version: version.to_owned(),
        display: Display { width: 1920, height: 1080 },
        has: Has { camera: false, microphone: false, speaker: true },
    }
}

/// The announcement: the TV by its MAC, listing the two modules it speaks.
fn service_of(device: &DeviceInfo, mac: &str, port: u16) -> mdns::Service {
    let six: String = mac.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_ascii_lowercase();
    let records = [
        ("mac", six.clone()),
        ("proto", ha_bridge::wire::PROTO.to_string()),
        ("friendly_name", device.friendly_name.clone()),
        ("project_name", format!("LumaShow.{}", device.model)),
        ("project_version", device.sw_version.clone()),
        ("modules", [ha_bridge::wire::module::STATES, ha_bridge::wire::module::ACTIONS].join(",")),
        ("api_encryption", "Noise_NNpsk0_25519_ChaChaPoly_SHA256".to_owned()),
    ];
    mdns::Service {
        instance: if six.len() >= 6 { format!("lumatv-{}", &six[six.len() - 6..]) } else { device.name.clone() },
        kind: ha_bridge::wire::SERVICE.into(),
        port,
        txt: records.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    }
}

// -- the owner's list ------------------------------------------------------------------------------------------------------------

/// The sizes an item of the card can be given, as the owner's list keeps them.
pub const SIZES: [&str; 3] = ["small", "wide", "large"];

/// What the owner chose for Home: which of the shared entities it shows and in what order (`None` is all of them, in the order of their
/// kinds; what is shared is already the owner's choice) and the size each item has on the card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picks {
    pub list: Option<Vec<String>>,
    /// `small`, `wide` or `large` by entity; an entity with none has the size its kind has.
    pub sizes: BTreeMap<String, String>,
}

fn picks_file(dir: &std::path::Path) -> PathBuf {
    dir.join("onhome").join("picks.json")
}

impl Picks {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn load(dir: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(picks_file(dir)) else { return Self::all() };
        let doc = serde_json::from_str::<Value>(&text).ok();
        let list = doc.as_ref().and_then(|v| v.get("picks").and_then(Value::as_array).cloned());
        let sizes = doc
            .as_ref()
            .and_then(|v| v.get("sizes").and_then(Value::as_object))
            .map(|m| m.iter().filter(|(id, _)| tile::domain_of(id).is_some()).filter_map(|(id, s)| s.as_str().filter(|s| SIZES.contains(s)).map(|s| (id.clone(), s.to_owned()))).take(2 * MAX_PICKS).collect())
            .unwrap_or_default();
        Self {
            list: list.map(|l| {
                let mut out: Vec<String> = Vec::new();
                for id in l.iter().filter_map(Value::as_str) {
                    if tile::domain_of(id).is_some() && !out.iter().any(|o| o == id) {
                        out.push(id.to_owned());
                    }
                }
                out.truncate(MAX_PICKS);
                out
            }),
            sizes,
        }
    }

    fn save(&self, dir: &std::path::Path) -> io::Result<()> {
        let file = picks_file(dir);
        if self.list.is_none() && self.sizes.is_empty() {
            return match std::fs::remove_file(&file) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut doc = json!({"sizes": self.sizes});
        if let (Some(list), Some(m)) = (&self.list, doc.as_object_mut()) {
            m.insert("picks".into(), json!(list));
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, doc.to_string())?;
        std::fs::rename(tmp, file)
    }

    /// The ids Home shows, in the order Home lists them: the owner's own order when there is a list, else by kind and inside a kind by area
    /// and name.
    pub fn shown(&self, mirror: &Mirror) -> Vec<String> {
        let group_of = |id: &str| tile::domain_of(id).and_then(Group::of_domain).unwrap_or(Group::Sensors);
        match &self.list {
            Some(list) => list.iter().take(MAX_PICKS).cloned().collect(),
            None => {
                let mut ids: Vec<(Group, String, String, String)> = mirror
                    .items()
                    .filter(|(_, item)| !item.minor)
                    .map(|(id, item)| (group_of(id), item.area.as_deref().and_then(|a| mirror.area_name(a)).unwrap_or("\u{10ffff}").to_lowercase(), item.name.to_lowercase(), id.clone()))
                    .collect();
                ids.sort();
                ids.into_iter().map(|(_, _, _, id)| id).take(MAX_PICKS).collect()
            }
        }
    }

    pub fn is_shown(&self, mirror: &Mirror, id: &str) -> bool {
        match &self.list {
            Some(list) => list.iter().any(|p| p == id),
            None => mirror.get(id).is_some_and(|i| !i.minor),
        }
    }

    /// The list made explicit (what is shown now), to be changed.
    fn explicit(&self, mirror: &Mirror) -> Vec<String> {
        match &self.list {
            Some(list) => list.clone(),
            None => self.shown(mirror),
        }
    }
}

// -- the state -----------------------------------------------------------------------------------------------------------------

struct State {
    bridge: Option<Arc<Bridge>>,
    announced: Option<(mdns::Mdns, Ipv4Addr)>,
    problem: Option<String>,
    picks: Picks,
    /// The names, areas and kinds of what was shared while the link was up, to say "not reachable" in the tiles' own names when it is not.
    known: BTreeMap<String, (String, Option<String>)>,
}

struct Inner {
    hub: Arc<Hub>,
    config: OnHomeConfig,
    deps: Deps,
    data_dir: PathBuf,
    /// What the shell shows to unlock (`<data dir>/onhome/shell.json`, root-only, new at every start of the helper). A route, an assistant
    /// or a plugin has no way to read it, so none can unlock.
    unlock_token: String,
    state: Mutex<State>,
    /// The histories asked lately: (entity and range, when, what came back). A sheet that is flipped between ranges costs one dial a range a minute.
    histories: Mutex<Vec<(String, std::time::Instant, Value)>>,
    /// The plugin's switch.
    shared: Arc<Shared>,
    /// Set by [`OnHome::wake`]: the switch moved or the helper stops.
    woken: Mutex<bool>,
    wake: Condvar,
}

/// The handle the API and the looker share.
#[derive(Clone)]
pub struct OnHome {
    inner: Arc<Inner>,
}

impl OnHome {
    pub fn new(ctx: &Ctx, deps: Deps, shared: &Arc<Shared>) -> Self {
        let data_dir = ctx.paths.data_dir.clone();
        let unlock_token = new_token();
        if let Err(e) = write_shell_file(&data_dir, &unlock_token) {
            // without the file the shell cannot unlock: locks are shown and locked, and not unlocked
            crate::log_warn!("onhome: the shell's unlock token could not be written: {e}");
        }
        let inner = Arc::new(Inner {
            hub: ctx.hub.clone(),
            config: ctx.config.onhome.clone(),
            deps,
            unlock_token,
            state: Mutex::new(State { bridge: None, announced: None, problem: None, picks: Picks::load(&data_dir), known: BTreeMap::new() }),
            histories: Mutex::new(Vec::new()),
            shared: shared.clone(),
            woken: Mutex::new(false),
            wake: Condvar::new(),
            data_dir,
        });
        let me = Self { inner };
        me.publish();
        me
    }

    /// Starts the looker, guarded as the Home Assistant plugin's: after a panic it looks again.
    pub fn spawn(&self, ctx: &Ctx) -> io::Result<JoinHandle<()>> {
        let me = self.clone();
        let stop = ctx.stop.clone();
        stop.on_stop(self.waker());
        crate::plugins::guard::spawn(
            "onhome",
            256 * 1024,
            self.inner.shared.clone(),
            &[crate::ha_plugin::ID],
            ctx.stop.clone(),
            move |_| {
                loop {
                    me.look();
                    if me.nap(&stop) {
                        break;
                    }
                }
                me.shut_down();
            },
        )
    }

    /// Has the looker look now: the plugin was switched, or the helper stops.
    fn wake(&self) {
        *self.inner.woken.lock() = true;
        self.inner.wake.notify_all();
    }

    /// What the plugin's switch wakes the looker with. It holds On Home weakly: the switch is kept by `Shared`, which On Home holds.
    pub fn waker(&self) -> impl Fn() + Send + Sync + 'static {
        let weak = Arc::downgrade(&self.inner);
        move || {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.wake();
            }
        }
    }

    /// Sleeps until the next look: `LOOK_EVERY`, or, switched off, until the switch or the stop wakes it. True once the stop is asked.
    fn nap(&self, stop: &crate::control::Stop) -> bool {
        let on = self.on();
        let mut woken = self.inner.woken.lock();
        if !*woken && !stop.is_set() {
            if on {
                self.inner.wake.wait_for(&mut woken, LOOK_EVERY);
            } else {
                self.inner.wake.wait(&mut woken);
            }
        }
        *woken = false;
        stop.is_set()
    }

    /// Whether the Home Assistant plugin is switched on.
    fn on(&self) -> bool {
        self.inner.shared.switched_on(ha_plugin::ID)
    }

    /// One look: what should run runs, and the topic says where it stands.
    pub fn look(&self) {
        let inner = &self.inner;
        let key = (inner.deps.key)();
        let want = self.on() && key.is_some();
        // a listener that stops is told so after the lock is let go: its last word is a change of the topic, which reads the state
        let mut stopped = None;
        {
            let mut state = inner.state.lock();
            if !want {
                if state.bridge.is_some() || state.announced.is_some() {
                    crate::log_info!("onhome: the listener stops");
                }
                stopped = self.stop_bridge(&mut state);
                state.problem = None;
            } else if state.bridge.is_none() {
                match self.start_bridge() {
                    Ok(bridge) => {
                        crate::log_info!("onhome: listening at {}", bridge.local_addr());
                        state.bridge = Some(bridge);
                        state.problem = None;
                    }
                    Err(why) => {
                        if state.problem.as_deref() != Some(why.as_str()) {
                            crate::log_warn!("onhome: {why}");
                        }
                        state.problem = Some(why);
                    }
                }
            }
            if let Some(bridge) = &state.bridge {
                // a session made with a key the TV no longer holds ends here, not at the listener's own next check
                bridge.recheck_key();
            }
            self.keep_announced(&mut state);
        }
        if let Some(bridge) = stopped {
            bridge.stop_and_wait();
        }
        self.publish();
    }

    fn start_bridge(&self) -> Result<Arc<Bridge>, String> {
        let inner = &self.inner;
        let ip = sys::lan_ipv4(inner.deps.bind).ok_or_else(|| "the TV has no private network address".to_owned())?;
        let mut device = inner.deps.device.clone();
        if let Some(mac) = crate::ble::interface_mac(ip) {
            let six: String = mac.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_ascii_lowercase();
            device.name = format!("lumatv-{}", &six[six.len().saturating_sub(6)..]);
            device.mac = mac;
        }
        let me = self.clone();
        let config = BridgeConfig {
            bind: format!("{ip}:{}", inner.config.port),
            key: inner.deps.key.clone(),
            device,
            timing: ha_bridge::Timing::default(),
            log: Arc::new(|line: String| crate::log_info!("onhome: {line}")),
            changed: Arc::new(move || me.publish()),
        };
        Bridge::start(config).map(Arc::new).map_err(|e| e.to_string())
    }

    /// Takes the listener out of the state; the caller stops it with the lock let go.
    fn stop_bridge(&self, state: &mut State) -> Option<Arc<Bridge>> {
        state.announced = None;
        state.bridge.take()
    }

    /// Announces the listener at the TV's address while it listens (the address can move: a changed one is announced anew).
    fn keep_announced(&self, state: &mut State) {
        let inner = &self.inner;
        let Some(bridge) = state.bridge.clone() else { return };
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
        let mac = crate::ble::interface_mac(ip).unwrap_or_default();
        let service = service_of(&inner.deps.device, &mac, bridge.local_addr().port());
        match mdns::Mdns::start_udp(ip) {
            Ok(mdns) => {
                mdns.add(service);
                state.announced = Some((mdns, ip));
            }
            Err(e) => {
                let why = format!("mDNS: {e}");
                if state.problem.as_deref() != Some(why.as_str()) {
                    crate::log_warn!("onhome: {why}");
                }
                state.problem = Some(why);
            }
        }
    }

    pub fn shut_down(&self) {
        let stopped = self.stop_bridge(&mut self.inner.state.lock());
        if let Some(bridge) = stopped {
            bridge.stop_and_wait();
        }
    }

    /// The listener's address, while it listens.
    pub fn local_addr(&self) -> Option<std::net::SocketAddr> {
        self.inner.state.lock().bridge.as_ref().map(|b| b.local_addr())
    }

    // -- what is said ----------------------------------------------------------------------------------------------------------

    /// The document of the `onhome` topic and of `GET /v1/onhome`.
    pub fn status(&self) -> Value {
        let inner = &self.inner;
        let key = (inner.deps.key)();
        let mut state = inner.state.lock();
        let bridge = state.bridge.clone();
        let picks = state.picks.clone();
        let problem = state.problem.clone();
        let (view, known) = view(self.on(), key.is_some(), bridge.as_deref(), &picks, &state.known, problem.as_deref());
        if let Some(known) = known {
            state.known = known;
        }
        view
    }

    fn publish(&self) {
        let doc = self.status();
        self.inner.hub.set(TOPIC, doc);
    }

    /// What the picker lists: the shared entities, filtered, by kind and name.
    pub fn shared(&self, request: &Request) -> Value {
        let state = self.inner.state.lock();
        let Some(bridge) = state.bridge.clone() else {
            return json!({"state": "unavailable", "rows": [], "more": false, "total": 0, "groups": [], "areas": []});
        };
        let picks = state.picks.clone();
        drop(state);
        bridge.with(|sh| search(&sh.mirror, &picks, request))
    }

    /// Changes the owner's list. Returns the new status.
    pub fn pick(&self, body: &Value) -> Result<Value, String> {
        let inner = &self.inner;
        let bridge = inner.state.lock().bridge.clone();
        let mut state = inner.state.lock();
        let picks = state.picks.clone();
        let next = bridge
            .as_deref()
            .map_or_else(|| apply_pick(&picks, &Mirror::default(), body), |b| b.with(|sh| apply_pick(&picks, &sh.mirror, body)))?;
        if next != picks {
            next.save(&inner.data_dir).map_err(|e| format!("the list could not be saved: {e}"))?;
            state.picks = next;
        }
        drop(state);
        self.publish();
        Ok(self.status())
    }

    /// The history of one sensor that is shared and shown: the kind and the words of its tile decide what is asked, never the caller.
    pub fn history(&self, id: &str, range: &str) -> Result<Value, Failure> {
        let inner = &self.inner;
        let (bridge, picks) = {
            let state = inner.state.lock();
            (state.bridge.clone(), state.picks.clone())
        };
        let Some(bridge) = bridge else { return Err(Failure::unreachable()) };
        let found = bridge.with(|sh| sh.mirror.get(id).filter(|_| picks.is_shown(&sh.mirror, id)).and_then(|item| tile::history_of(id, &item.ent)));
        let Some((kind, unit, class)) = found else { return Err(Failure::from(Refused::NotShown)) };
        let kind = history::Kind::from_id(kind).ok_or_else(|| Failure::from(Refused::NotShown))?;
        let key = format!("{id}|{range}");
        if let Some((_, _, doc)) = inner.histories.lock().iter().find(|(k, at, _)| *k == key && at.elapsed() < HISTORY_KEPT) {
            return Ok(doc.clone());
        }
        let doc = inner.deps.history.ask(id, kind, range, &unit, &class, now_ms()).map_err(|w| Failure { code: w.code, words: w.words })?;
        let mut kept = inner.histories.lock();
        kept.retain(|(k, at, _)| *k != key && at.elapsed() < HISTORY_KEPT);
        kept.push((key, std::time::Instant::now(), doc.clone()));
        if kept.len() > 16 {
            kept.remove(0);
        }
        Ok(doc)
    }

    /// One control of one tile. Never an unlock: a lock only locks here (`tile::plan`), and a try to unlock or unlatch is refused and written down.
    pub fn act(&self, id: &str, verb: &str, value: &Value) -> Result<(), Failure> {
        let unit = |m: &Mirror| m.temp_unit();
        let result = self.carry_out(id, false, |item, mirror| tile::plan(id, &item.ent, &item.ctl, unit(mirror), verb, value));
        if id.starts_with("lock.") && matches!(verb, "unlock" | "open") {
            self.journal("refused", id, "the API asked for it; only the TV's own screen unlocks", false);
        }
        result
    }

    /// Unlocks one lock, for the TV's own shell: the person pressed Unlock in the shell's question with the remote, and the shell shows the
    /// token that only it can read. Every try is written down, done or refused. Home Assistant decides last (its own PIN and confirmation rules).
    pub fn unlock(&self, id: &str, token: &str) -> Result<(), Failure> {
        if !same(token.as_bytes(), self.inner.unlock_token.as_bytes()) {
            self.journal("refused", id, "not from the TV's shell (wrong token)", true);
            return Err(Failure::from(Refused::NeedsShell));
        }
        let result = self.carry_out(id, true, |item, _| tile::plan_unlock(id, &item.ent, &item.ctl, &item.ctl_pin));
        match &result {
            Ok(()) => self.journal("done", id, "unlocked from the TV's remote", true),
            Err(f) => self.journal("failed", id, &f.words, true),
        }
        // Home Assistant's own words about its PIN say where the setting is
        result.map_err(|f| if f.words.contains("PIN") { Failure { code: f.code, words: "Home Assistant wants its PIN for this. Allow unlocking from the TV with a confirmation in its LumaTV entry".into() } } else { f })
    }

    /// The common path of a control: the thing must be shared and on Home, Home Assistant connected, and the plan the table makes.
    fn carry_out(&self, id: &str, touch: bool, plan: impl FnOnce(&mirror::Item, &Mirror) -> Result<tile::Call, Refused>) -> Result<(), Failure> {
        let inner = &self.inner;
        let (bridge, picks) = {
            let state = inner.state.lock();
            (state.bridge.clone(), state.picks.clone())
        };
        let Some(bridge) = bridge else { return Err(Failure::unreachable()) };
        let planned = bridge.with(|sh| {
            if sh.client.is_none() {
                return Err(Refused::Unavailable);
            }
            let Some(item) = sh.mirror.get(id).filter(|_| picks.is_shown(&sh.mirror, id)) else {
                return Err(Refused::NotShown);
            };
            plan(item, &sh.mirror)
        });
        let call = planned.map_err(Failure::from)?;
        let (tx, rx) = channel();
        bridge.call(
            Call { entity_ids: vec![id.to_owned()], verb: call.service.to_owned(), args: call.data, touch },
            Box::new(move |result| {
                let _ = tx.send(result);
            }),
        );
        match rx.recv_timeout(ACT_WITHIN) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(why)) => Err(Failure { code: "failed", words: why }),
            Err(_) => Err(Failure { code: "timeout", words: "Home Assistant did not answer".into() }),
        }
    }

    /// A line of the unlock journal (`<data dir>/onhome/journal.jsonl`, and the helper's log): when, which lock, how it ended and who asked.
    /// The file keeps its last 256 KiB.
    fn journal(&self, result: &str, id: &str, why: &str, from_shell: bool) {
        let name = {
            let state = self.inner.state.lock();
            state.known.get(id).map(|k| k.0.clone())
        };
        let line = json!({"at": now_ms(), "what": "unlock", "id": id, "name": name, "result": result, "why": why, "viaShell": from_shell});
        crate::log_info!("onhome: unlock {result}: {id} ({why})");
        let dir = self.inner.data_dir.join("onhome");
        let file = dir.join("journal.jsonl");
        let _ = std::fs::create_dir_all(&dir);
        let mut text = std::fs::read_to_string(&file).unwrap_or_default();
        text.push_str(&line.to_string());
        text.push('\n');
        if text.len() > JOURNAL_KEEPS {
            let from = text.len() - JOURNAL_KEEPS / 2;
            text = text[from..].split_once('\n').map_or_else(String::new, |(_, rest)| rest.to_owned());
        }
        let _ = std::fs::write(&file, text);
    }
}

/// How much of the journal is kept.
const JOURNAL_KEEPS: usize = 256 * 1024;

/// A new random token, as hex.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut f, &mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `<data dir>/onhome/shell.json` for the shell to read (`Effect::Read`), mode 0600 in a directory of 0700.
fn write_shell_file(data_dir: &std::path::Path, token: &str) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = data_dir.join("onhome");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    let tmp = dir.join("shell.json.tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    io::Write::write_all(&mut file, json!({"token": token}).to_string().as_bytes())?;
    std::fs::rename(tmp, dir.join("shell.json"))
}

/// Equal, without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// A refusal with its code and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub words: String,
}

impl Failure {
    fn unreachable() -> Self {
        Self { code: "unavailable", words: "Home Assistant is not connected".into() }
    }
}

impl From<Refused> for Failure {
    fn from(r: Refused) -> Self {
        Self { code: r.code(), words: r.words() }
    }
}

/// Applies one change to the owner's list.
fn apply_pick(picks: &Picks, mirror: &Mirror, body: &Value) -> Result<Picks, String> {
    let id_of = |key: &str| body.get(key).and_then(Value::as_str);
    if body.get("all").and_then(Value::as_bool) == Some(true) {
        return Ok(Picks { list: None, sizes: picks.sizes.clone() });
    }
    // the size of one item: kept beside the list and not part of it, so it holds while the item is hidden and shown again
    if let Some(id) = id_of("size") {
        if tile::domain_of(id).is_none() {
            return Err("that is not something Home can show".into());
        }
        let mut sizes = picks.sizes.clone();
        match body.get("to") {
            Some(Value::Null) | None => {
                sizes.remove(id);
            }
            Some(Value::String(s)) if SIZES.contains(&s.as_str()) => {
                if sizes.len() >= 2 * MAX_PICKS && !sizes.contains_key(id) {
                    return Err("too many sizes are kept".into());
                }
                sizes.insert(id.to_owned(), s.clone());
            }
            _ => return Err("a size is small, wide or large".into()),
        }
        return Ok(Picks { list: picks.list.clone(), sizes });
    }
    // with nothing shared (Home Assistant away) there is no "all of it" to take a thing out of: the list would come out empty
    if picks.list.is_none() && mirror.is_empty() {
        return Err("Home Assistant is not connected, so there is nothing to choose from".into());
    }
    let mut list = picks.explicit(mirror);
    if let Some(id) = id_of("show") {
        if tile::domain_of(id).is_none() {
            return Err("that is not something Home can show".into());
        }
        if !list.iter().any(|p| p == id) {
            if list.len() >= MAX_PICKS {
                return Err(format!("Home shows at most {MAX_PICKS}"));
            }
            // after the last of its own kind when the list has one (it keeps its company), else at the end
            let group = |i: &str| tile::domain_of(i).and_then(Group::of_domain);
            let after = list.iter().rposition(|p| group(p) == group(id)).map_or(list.len(), |i| i + 1);
            list.insert(after, id.to_owned());
        }
    } else if let Some(id) = id_of("hide") {
        list.retain(|p| p != id);
    } else if let Some(id) = id_of("move") {
        // to a place of the list: the card's own order, which crosses kinds
        let Some(at) = list.iter().position(|p| p == id) else { return Err("that is not on Home".into()) };
        let to = body.get("to").and_then(Value::as_u64).ok_or("move needs the place it goes to")? as usize;
        let moved = list.remove(at);
        list.insert(to.min(list.len()), moved);
    } else if let Some((id, down)) = id_of("up").map(|i| (i, false)).or_else(|| id_of("down").map(|i| (i, true))) {
        // the order is among the same kind: a tile does not leave its group
        let group = |i: &str| tile::domain_of(i).and_then(Group::of_domain);
        let Some(at) = list.iter().position(|p| p == id) else { return Err("that is not on Home".into()) };
        let other = if down { list.iter().enumerate().skip(at + 1).find(|(_, p)| group(p) == group(id)) } else { list.iter().enumerate().take(at).rev().find(|(_, p)| group(p) == group(id)) };
        if let Some((to, _)) = other {
            list.swap(at, to);
        }
    } else {
        return Err("say show, hide, move, size, up, down or all".into());
    }
    Ok(Picks { list: Some(list), sizes: picks.sizes.clone() })
}

type Known = BTreeMap<String, (String, Option<String>)>;

/// The document: pure over what the listener holds, so a test reads it without a socket. The second answer is the names to remember.
fn view(on: bool, paired: bool, bridge: Option<&Bridge>, picks: &Picks, known: &Known, problem: Option<&str>) -> (Value, Option<Known>) {
    let doc = |state: &str, words: &str, detail: &str, extra: Value| {
        let mut d = json!({"state": state, "words": words, "detail": detail, "at": now_ms(), "tiles": [], "shared": 0, "controllable": 0, "paused": false, "picked": null});
        if let (Some(m), Some(e)) = (d.as_object_mut(), extra.as_object()) {
            m.extend(e.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        d
    };
    if !on {
        return (doc("off", "Not switched on", ha_plugin::off_words(), json!({})), None);
    }
    if !paired {
        return (doc("unpaired", "Not paired", "Pair the TV with Home Assistant first (lumatv-helper ble-link)", json!({})), None);
    }
    let Some(bridge) = bridge else {
        let why = problem.unwrap_or("Starting");
        return (doc("problem", "Can't listen", why, json!({})), None);
    };
    bridge.with(|sh| {
        let mirror = &sh.mirror;
        let picked = picks.list.clone().map(|l| json!(l));
        // the size the owner gave an item on the card, with the tile it belongs to
        let sized = |mut t: Value| {
            let id = t["id"].as_str().map(str::to_owned);
            if let (Some(m), Some(id)) = (t.as_object_mut(), id) {
                m.insert("size".into(), picks.sizes.get(&id).map_or(Value::Null, |s| json!(s)));
            }
            t
        };
        if sh.client.is_none() {
            // Home Assistant is not connected: the tiles that were there say so, in their own names (the owner's list, else all that was shared)
            let reach = Reach::Waiting("Not reachable");
            let group_of = |id: &str| tile::domain_of(id).and_then(Group::of_domain).unwrap_or(Group::Sensors);
            let ids: Vec<&String> = match &picks.list {
                Some(list) => list.iter().collect(),
                None => {
                    let mut all: Vec<(Group, &String)> = known.keys().map(|id| (group_of(id), id)).collect();
                    all.sort();
                    all.into_iter().map(|(_, id)| id).collect()
                }
            };
            let tiles: Vec<Value> = ids
                .into_iter()
                .take(MAX_PICKS)
                .filter_map(|id| known.get(id).map(|(name, area)| sized(tile::tile(id, Some(name), area.as_deref(), None, &[], &[], "", reach))))
                .collect();
            let d = doc("waiting", "Waiting for Home Assistant", "Home Assistant's LumaShow integration has not connected", json!({"tiles": tiles, "picked": picked}));
            return (d, None);
        }
        let unit = mirror.temp_unit();
        let ids = picks.shown(mirror);
        let mut remember = Known::new();
        let mut tiles: Vec<Value> = Vec::with_capacity(ids.len());
        for id in &ids {
            let item = mirror.get(id);
            let area = item.and_then(|i| i.area.as_deref()).and_then(|a| mirror.area_name(a)).map(str::to_owned);
            let name = item.map(|i| i.name.clone());
            if let (Some(n), true) = (&name, item.is_some()) {
                remember.insert(id.clone(), (n.clone(), area.clone()));
            }
            let ctl: &[String] = if mirror.paused() { &[] } else { item.map_or(&[], |i| &i.ctl) };
            let ctl_pin: &[String] = if mirror.paused() { &[] } else { item.map_or(&[], |i| &i.ctl_pin) };
            tiles.push(sized(tile::tile(id, name.as_deref().or_else(|| known.get(id).map(|k| k.0.as_str())), area.as_deref(), item.map(|i| &i.ent), ctl, ctl_pin, unit, Reach::Live)));
        }
        let shared = mirror.len();
        let (state, words) = match (shared, tiles.len()) {
            (0, _) => ("empty", "Nothing shared yet".to_owned()),
            (_, 0) => ("empty", "Nothing on Home".to_owned()),
            (_, n) => ("connected", if n == 1 { "1 thing".to_owned() } else { format!("{n} things") }),
        };
        let detail = match state {
            "empty" if shared == 0 => "Share some in Home Assistant: Settings, Devices & services, LumaShow, LumaTV, Configure",
            "empty" => "Choose what Home shows in Settings, Home & look, Home Assistant, On Home",
            _ if mirror.paused() => "Control is paused in Home Assistant",
            _ => "",
        };
        let d = doc(state, &words, detail, json!({"tiles": tiles, "shared": shared, "controllable": mirror.controllable(), "paused": mirror.paused(), "picked": picked, "revision": mirror.rev()}));
        (d, Some(remember))
    })
}

/// The picker: the shared entities that match, by kind and name.
fn search(mirror: &Mirror, picks: &Picks, request: &Request) -> Value {
    let q = request.param("q").unwrap_or("").trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let area = request.param("area").filter(|a| !a.is_empty());
    let group = request.param("group").and_then(Group::of_id);
    let offset: usize = request.param("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let limit: usize = request.param("limit").and_then(|v| v.parse().ok()).unwrap_or(30).clamp(1, 64);
    let unit = mirror.temp_unit();
    let shown: std::collections::BTreeSet<String> = picks.shown(mirror).into_iter().collect();
    let listed = |picks: &Picks, id: &str| match &picks.list {
        Some(l) => l.iter().any(|p| p == id),
        None => shown.contains(id),
    };
    // (group, area name, name, id) of what matches the words and the filters of the other axis
    struct Hit<'a> {
        id: &'a str,
        group: Group,
        area_id: Option<&'a str>,
        area: &'a str,
        item: &'a mirror::Item,
    }
    let mut hits: Vec<Hit> = Vec::new();
    for (id, item) in mirror.items() {
        let Some(g) = tile::domain_of(id).and_then(Group::of_domain) else { continue };
        let area_name = item.area.as_deref().and_then(|a| mirror.area_name(a)).unwrap_or("");
        if item.minor && words.is_empty() {
            continue;
        }
        let hay = format!("{} {} {}", item.name.to_lowercase(), id, area_name.to_lowercase());
        if !words.iter().all(|w| hay.contains(w)) {
            continue;
        }
        hits.push(Hit { id, group: g, area_id: item.area.as_deref(), area: area_name, item });
    }
    let count_by = |f: &dyn Fn(&Hit) -> bool| hits.iter().filter(|h| f(h)).count();
    let groups: Vec<Value> = GROUPS
        .iter()
        .map(|g| json!({"id": g.id(), "title": g.title(), "count": count_by(&|h| h.group == *g && area.is_none_or(|a| h.area_id == Some(a)))}))
        .filter(|g| g["count"].as_u64().unwrap_or(0) > 0)
        .collect();
    let areas: Vec<Value> = mirror
        .areas()
        .into_iter()
        .map(|(id, name)| {
            let n = count_by(&|h| h.area_id == Some(id.as_str()) && group.is_none_or(|g| h.group == g));
            json!({"id": id, "name": name, "count": n})
        })
        .filter(|a| a["count"].as_u64().unwrap_or(0) > 0)
        .collect();
    let mut matched: Vec<&Hit> = hits.iter().filter(|h| group.is_none_or(|g| h.group == g) && area.is_none_or(|a| h.area_id == Some(a))).collect();
    matched.sort_by(|a, b| (a.group, a.area.to_lowercase(), a.item.name.to_lowercase(), a.id).cmp(&(b.group, b.area.to_lowercase(), b.item.name.to_lowercase(), b.id)));
    let total = matched.len();
    let rows: Vec<Value> = matched
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|h| {
            json!({"id": h.id, "name": h.item.name, "area": if h.area.is_empty() { Value::Null } else { json!(h.area) }, "group": h.group.id(),
                   "words": tile::glance(h.id, &h.item.ent, unit), "icon": tile::icon_of(h.id, &h.item.ent, unit),
                   "controllable": !h.item.ctl.is_empty(), "shown": listed(picks, h.id)})
        })
        .collect();
    json!({"state": "ready", "rows": rows, "more": total > offset + limit, "total": total, "groups": groups, "areas": areas, "limit": MAX_PICKS})
}

// -- the routes ------------------------------------------------------------------------------------------------------------------

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

pub fn handle(onhome: &OnHome, request: &Request, rest: &[&str]) -> Response {
    let get = request.method == "GET";
    let post = request.method == "POST";
    let body = || serde_json::from_slice::<Value>(&request.body).ok().filter(Value::is_object);
    if !rest.is_empty() && !onhome.on() {
        return error_response(409, "off", "the Home Assistant plugin is switched off");
    }
    match rest {
        [] if get => ok(json!({"onhome": onhome.status()})),
        ["shared"] if get => ok(onhome.shared(request)),
        ["history"] if get => {
            let (Some(id), range) = (request.param("id"), request.param("range").unwrap_or("24h")) else {
                return error_response(400, "bad_request", "id is needed");
            };
            match onhome.history(id, range) {
                Ok(doc) => ok(json!({"history": doc})),
                Err(r) => error_response(409, r.code, &r.words),
            }
        }
        ["picks"] if post => match body() {
            None => error_response(400, "bad_request", "the body is not a JSON object"),
            Some(b) => match onhome.pick(&b) {
                Ok(status) => ok(json!({"onhome": status})),
                Err(why) => error_response(422, "invalid", &why),
            },
        },
        ["act"] if post => {
            let Some(b) = body() else { return error_response(400, "bad_request", "the body is not a JSON object") };
            let (Some(id), Some(verb)) = (b.get("id").and_then(Value::as_str), b.get("verb").and_then(Value::as_str)) else {
                return error_response(400, "bad_request", "id and verb are needed");
            };
            match onhome.act(id, verb, b.get("value").unwrap_or(&Value::Null)) {
                Ok(()) => ok(json!({"done": true})),
                Err(r) => error_response(409, r.code, &r.words),
            }
        }
        ["unlock"] if post => {
            // Only the TV's own shell, on the loopback, with the token it alone can read. A request from another machine never gets as far as the token.
            if !request.peer.is_loopback() {
                return error_response(403, "needs_shell", &Refused::NeedsShell.words());
            }
            let Some(b) = body() else { return error_response(400, "bad_request", "the body is not a JSON object") };
            let (Some(id), Some(token)) = (b.get("id").and_then(Value::as_str), b.get("token").and_then(Value::as_str)) else {
                return error_response(400, "bad_request", "id and token are needed");
            };
            match onhome.unlock(id, token) {
                Ok(()) => ok(json!({"done": true})),
                Err(r) if r.code == "needs_shell" => error_response(403, r.code, &r.words),
                Err(r) => error_response(409, r.code, &r.words),
            }
        }
        [] | ["shared" | "history"] => error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "GET"),
        ["picks" | "act" | "unlock"] => error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "POST"),
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}

/// What a test acts through: its own key, the loopback, no announcement.
pub mod testing {
    use super::*;

    pub fn deps() -> Deps {
        keyed_deps(None)
    }

    /// `GET /v1/onhome/shared?<query>` as the router would answer it.
    pub fn shared_of(onhome: &OnHome, query: &str) -> Value {
        let query = query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
        let request = Request { method: "GET".into(), path: "/v1/onhome/shared".into(), query, headers: vec![], body: vec![], peer: std::net::Ipv4Addr::LOCALHOST.into() };
        onhome.shared(&request)
    }

    /// `GET /v1/onhome/history?id=&range=` as the router would answer it: `(status, document)`.
    pub fn history_route(onhome: &OnHome, id: &str, range: &str) -> (u16, Value) {
        let query = vec![("id".to_owned(), id.to_owned()), ("range".to_owned(), range.to_owned())];
        let request = Request { method: "GET".into(), path: "/v1/onhome/history".into(), query, headers: vec![], body: vec![], peer: std::net::Ipv4Addr::LOCALHOST.into() };
        let response = handle(onhome, &request, &["history"]);
        (response.status, serde_json::from_slice(&response.body).unwrap_or(Value::Null))
    }

    /// A route as the router would hand it: `(status, document)`.
    pub fn route(onhome: &OnHome, method: &str, rest: &str, body: Value, peer: &str) -> (u16, Value) {
        let request = Request { method: method.into(), path: format!("/v1/onhome/{rest}"), query: vec![], headers: vec![], body: body.to_string().into_bytes(), peer: peer.parse().expect("an address") };
        let response = handle(onhome, &request, &[rest]);
        (response.status, serde_json::from_slice(&response.body).unwrap_or(Value::Null))
    }

    pub fn keyed_deps(key: Option<[u8; 32]>) -> Deps {
        Deps { key: Arc::new(move || key), bind: Some(Ipv4Addr::LOCALHOST), announce: false, device: device_info("LumaTV", "LG TV", "test"), history: history::Source { links: Vec::new(), policy: ha_link::endpoint::AddressPolicy::TESTING, tls: None } }
    }
}
