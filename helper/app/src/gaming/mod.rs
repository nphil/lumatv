//! Opt-in gaming privileges, with a pidfd-owned display lease and same-boot rollback.
#![allow(unsafe_code)]

mod device;
mod display;
mod lease;
mod overlay;

use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::plugins::local::{Local, OptIn, Spec};
use crate::plugins::session::Shared;
use device::{Control, Devices, LgDevices};
use lease::{Holder, State};
use parking_lot::Mutex;
use plugin_api::wire::ToPlugin;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub const ID: &str = "gaming";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec { id: ID, name: "Gaming", version: "1", permissions: &["tiles"] },
    consent: "Allows the gaming apps to temporarily use qualified low-latency picture controls, give their controller readers real-time priority, and read and save supported controllers' settings. Original picture and thread settings are restored when a session ends, the app exits, the lease expires, the TV enters standby or this plugin is switched off. Controller settings remain saved on the controller. Display controls remain unavailable until qualified on this TV.",
};

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/gaming/status", "gaming lease, qualification records and restoration results; plugin must be enabled"),
    ("POST", "/v1/gaming/lease", "acquire a display lease {pid, controls}; renew every 2 seconds, expires after 6"),
    ("DELETE", "/v1/gaming/lease", "restore the display lease {lease}"),
    ("POST", "/v1/gaming/renew", "renew the display lease {lease}"),
    ("POST", "/v1/gaming/qualify", "qualify one fixed control {control, lifecycleReceipt?, liveReadbackReceipt?}; read/set/read/restore/read, retaining the result"),
    ("POST", "/v1/gaming/restore", "retry pending same-boot rollback without changing saved originals"),
    ("POST", "/v1/gaming/pad-boost", "controller reader {pid, tid}: FIFO 10, off CPU 0, 90 second lease"),
    ("POST", "/v1/gaming/pad-unboost", "restore controller reader {pid, tid}"),
    ("GET", "/v1/gaming/pad-boost", "the controller readers boosted now"),
    ("GET", "/v1/gaming/display", "the panel's readings for LumaGame's stats overlay (?full=1 adds the firmware's status words); sampled only while asked"),
    ("GET", "/v1/gaming/overlay", "the stats overlay setting {mode: off|compact|full, changed}, also the gaming.overlay topic"),
    ("POST", "/v1/gaming/overlay", "set the stats overlay {mode, changed?}; the newer change wins"),
];

static INSTANCES: LazyLock<Mutex<Vec<Weak<Gaming>>>> = LazyLock::new(|| Mutex::new(Vec::new()));

struct Gaming {
    hub: Arc<Hub>,
    devices: Box<dyn Devices>,
    state: Mutex<State>,
    wake: OwnedFd,
    session: Mutex<Option<Arc<Local>>>,
    display: display::Display,
    overlay: overlay::Store,
}

impl Gaming {
    fn signal(&self) {
        let one = 1u64;
        // SAFETY: eventfd consumes exactly eight bytes; EAGAIN means a wake is already pending.
        unsafe { libc::write(self.wake.as_raw_fd(), (&one as *const u64).cast(), 8); }
    }

    fn follow(self: &Arc<Self>, shared: &Arc<Shared>) {
        let on = shared.switched_on(ID);
        let mut session = self.session.lock();
        let mut state = self.state.lock();
        state.enabled = on;
        if !on {
            if let Err(e) = state.restore("plugin_off") { crate::log_warn!("gaming: {e}"); }
            crate::gfn_boost::restore_all();
            *session = None;
            self.display.stop();
        } else if session.is_none() {
            let weak = Arc::downgrade(self);
            let handler = Arc::new(move |local: &Local, message: ToPlugin| {
                if let ToPlugin::ActionInvoke { op, action, target, .. } = message {
                    let result = weak.upgrade().ok_or_else(|| "Gaming plugin stopped".to_string()).and_then(|gaming| {
                        let mode = target.as_deref().and_then(overlay::Mode::choice)
                            .filter(|_| action == "stats-mode").ok_or_else(|| "Unknown Game stats choice".to_string())?;
                        gaming.set_overlay(mode, None).map(|_| ()).map_err(|error| error.to_string())
                    });
                    let _ = local.answer(op, result.as_ref().map(|_| ()).map_err(String::as_str));
                }
            });
            let register = json!({"title":"Gaming","actions":[{"key":"stats-mode","label":"Set Game stats"}],
                "tiles":[overlay::tile(self.overlay.get().mode)]});
            match Local::start(shared, &OPT_IN.spec, register, handler) {
                Ok(local) => *session = Some(local),
                Err(e) => { state.enabled = false; crate::log_warn!("gaming: {e}"); }
            }
        }
        drop(state);
        self.signal();
    }

    fn set_overlay(&self, mode: overlay::Mode, changed: Option<u64>) -> io::Result<(overlay::Overlay, bool)> {
        // Hold the session across the write and publication so plugin-off cannot leave a late tile behind.
        let session = self.session.lock();
        let local = session.as_ref().filter(|_| self.state.lock().enabled)
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Gaming plugin is off"))?;
        let (value, taken) = self.overlay.set(mode, changed, crate::timeutil::now_ms(), &self.hub)?;
        if taken {
            let mut tile = overlay::tile(value.mode);
            tile["t"] = json!("tile.put");
            local.say(tile).map_err(io::Error::other)?;
        }
        Ok((value, taken))
    }

    fn run(&self, ctx: &Ctx) {
        while !ctx.stop.is_set() {
            let (pidfd, timeout) = {
                let mut state = self.state.lock();
                // Duplicate under the state lock so a simultaneous release cannot recycle the fd.
                let pidfd = state.lease.as_ref().map(|lease| lease.holder.fd.try_clone()).transpose();
                let pidfd = match pidfd {
                    Ok(fd) => fd,
                    Err(e) => {
                        if let Err(restore) = state.restore("watch_failed") { crate::log_warn!("gaming: {restore}"); }
                        state.error = Some(format!("opening pidfd watch: {e}"));
                        None
                    }
                };
                let timeout = if let Some(lease) = &state.lease {
                    (lease.until.saturating_duration_since(Instant::now()).as_millis() + 1).min(250) as i32
                } else if crate::gfn_boost::active() { 250 } else { -1 };
                (pidfd, timeout)
            };
            let mut fds = [
                libc::pollfd { fd: self.wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: pidfd.as_ref().map_or(-1, AsRawFd::as_raw_fd), events: libc::POLLIN, revents: 0 },
            ];
            // pidfd reports death immediately. Only an active lease/boost polls power state;
            // with neither, eventfd sleeps indefinitely until a switch, request or shutdown.
            // SAFETY: fds is valid for the supplied two-element count.
            let result = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if result < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted { continue; }
                let mut state = self.state.lock();
                let _ = state.restore("watch_failed");
                state.enabled = false;
                state.error = Some("gaming pidfd watcher failed".into());
                crate::gfn_boost::restore_all();
                break;
            }
            if fds[0].revents & libc::POLLIN != 0 {
                let mut value = 0u64;
                // SAFETY: the eight-byte destination lives for the nonblocking eventfd read.
                unsafe { libc::read(self.wake.as_raw_fd(), (&mut value as *mut u64).cast(), 8); }
            }
            let mut state = self.state.lock();
            // Recheck the current lease, not the possibly replaced fd that poll observed.
            let dead = state.lease.as_ref().is_some_and(|lease| holder_dead(&lease.holder));
            let on = crate::screen::is_on(&self.hub);
            if let Err(e) = state.tick(Instant::now(), dead, on) { crate::log_warn!("gaming: {e}"); }
            crate::gfn_boost::sweep(Instant::now());
            if !on || !state.enabled { crate::gfn_boost::restore_all(); }
        }
        let mut state = self.state.lock();
        state.enabled = false;
        if let Err(e) = state.restore("helper_stop") { crate::log_warn!("gaming: {e}"); }
        crate::gfn_boost::restore_all();
    }
}

fn holder_dead(holder: &Holder) -> bool {
    let mut fd = libc::pollfd { fd: holder.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: the one-element pollfd is live; zero timeout never blocks the handler.
    let result = unsafe { libc::poll(&mut fd, 1, 0) };
    result < 0 || fd.revents != 0
}

/// Registers the switch and replays rollback before the API can accept a gaming request.
pub fn spawn(ctx: &Ctx, shared: Arc<Shared>) -> io::Result<JoinHandle<()>> {
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim().to_owned();
    if boot.is_empty() { return Err(io::Error::other("gaming cannot read the boot id")); }
    // The rollback journal must survive a helper restart, but not a reboot.
    let journal = PathBuf::from("/run/lumatv-gaming/rollback.json");
    let records = ctx.paths.data_dir.join("gaming/qualifications.json");
    let mut state = State::new(journal, records, boot);
    let devices: Box<dyn Devices> = Box::new(LgDevices::default());
    if let Err(e) = state.recover(&*devices) { state.error = Some(e); }
    // SAFETY: eventfd returns a new fd or -1; no pointers are passed.
    let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if wake < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: the successful eventfd call transfers a fresh fd to this owner.
    let wake = unsafe { OwnedFd::from_raw_fd(wake) };
    let overlay = overlay::Store::load(ctx.paths.data_dir.join("gaming/overlay.json"));
    overlay.publish(&ctx.hub);
    let gaming = Arc::new(Gaming {
        hub: ctx.hub.clone(), devices, state: Mutex::new(state), wake, session: Mutex::new(None), display: display::Display::lg(), overlay,
    });
    let weak = Arc::downgrade(&gaming);
    let roster = Arc::downgrade(&shared);
    shared.offer(&OPT_IN, Arc::new(move || {
        if let (Some(gaming), Some(shared)) = (weak.upgrade(), roster.upgrade()) { gaming.follow(&shared); }
    }));
    let weak = Arc::downgrade(&gaming);
    ctx.stop.on_stop(move || { if let Some(gaming) = weak.upgrade() { gaming.signal(); } });
    let ctx = ctx.clone();
    let worker = gaming.clone();
    let thread = thread::Builder::new().name("gaming-lease".into()).stack_size(256 * 1024).spawn(move || worker.run(&ctx))?;
    let mut instances = INSTANCES.lock();
    instances.retain(|instance| instance.strong_count() != 0);
    instances.push(Arc::downgrade(&gaming));
    Ok(thread)
}

fn instance(hub: &Arc<Hub>) -> Option<Arc<Gaming>> {
    INSTANCES.lock().iter().filter_map(Weak::upgrade).find(|gaming| Arc::ptr_eq(&gaming.hub, hub))
}

fn ok(mut value: Value) -> Response {
    value["ok"] = json!(true);
    value["api"] = json!(crate::API_VERSION);
    value["now"] = json!(crate::timeutil::now_ms());
    Response::json(200, &value)
}

fn gate(state: &State, hub: &Hub, request: &Request) -> Result<(), Response> {
    if !request.peer.is_loopback() { return Err(error_response(403, "refused", "gaming is loopback only")); }
    if !state.enabled { return Err(error_response(403, "plugin_off", "The Gaming plugin is off: switch it on in Settings")); }
    if !crate::screen::is_on(hub) { return Err(error_response(409, "standby", "gaming controls are unavailable while the screen is off")); }
    Ok(())
}

/// Keeps plugin-off and standby restoration serialized with scheduler writes.
pub fn with_enabled(hub: &Arc<Hub>, request: &Request, handler: impl FnOnce() -> Response) -> Response {
    let Some(gaming) = instance(hub) else { return error_response(403, "plugin_off", "The Gaming plugin is off: switch it on in Settings"); };
    let state = gaming.state.lock();
    if let Err(response) = gate(&state, hub, request) { return response; }
    let response = handler();
    gaming.signal();
    response
}

/// Slow controller exchanges must not hold the display lease's restoration lock.
pub(crate) fn check_enabled(hub: &Arc<Hub>, request: &Request) -> Result<(), Response> {
    let Some(gaming) = instance(hub) else { return Err(error_response(403, "plugin_off", "The Gaming plugin is off: switch it on in Settings")); };
    gate(&gaming.state.lock(), hub, request)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Acquire {
    pid: i32,
    #[serde(default = "default_controls")]
    controls: Vec<Control>,
}
fn default_controls() -> Vec<Control> { Control::ALL.to_vec() }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseId { lease: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Qualify {
    control: Control,
    live_readback_receipt: Option<String>,
    lifecycle_receipt: Option<String>,
}

fn body<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, Response> {
    serde_json::from_slice(&request.body).map_err(|e| error_response(400, "bad_request", &e.to_string()))
}

fn receipt(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|value| !value.trim().is_empty() && value.len() <= 1024)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetOverlay {
    mode: overlay::Mode,
    changed: Option<u64>,
}

/// LumaGame's stats overlay: the panel's readings and the one setting that shows it.
fn overlay_route(gaming: &Gaming, request: &Request, rest: &[&str]) -> Response {
    match (request.method.as_str(), rest) {
        ("GET", ["display"]) => ok(json!({"display": gaming.display.ask(request.param("full") == Some("1")), "overlay": gaming.overlay.get()})),
        ("POST", ["overlay"]) => {
            let set: SetOverlay = match body(request) { Ok(value) => value, Err(response) => return response };
            match gaming.set_overlay(set.mode, set.changed) {
                Ok((overlay, taken)) => ok(json!({"overlay": overlay, "taken": taken})),
                Err(e) => error_response(500, "io", &e.to_string()),
            }
        }
        _ => ok(json!({"overlay": gaming.overlay.get()})),
    }
}

pub fn handle(hub: &Arc<Hub>, request: &Request, rest: &[&str]) -> Response {
    if matches!(rest, ["pad-boost" | "pad-unboost"]) { return crate::gfn_boost::handle(hub, request, rest); }
    if rest == ["gamesir"] { return crate::gamesir::handle(hub, request); }
    if rest == ["leadjoy"] { return crate::leadjoy::handle(hub, request); }
    let allow = match rest {
        ["status" | "display"] => "GET",
        ["overlay"] => "GET, POST",
        ["lease"] => "POST, DELETE",
        ["renew" | "qualify" | "restore"] => "POST",
        _ => return error_response(404, "not_found", "no such endpoint"),
    };
    if !allow.split(", ").any(|method| method == request.method) {
        return error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", allow);
    }
    let Some(gaming) = instance(hub) else { return error_response(403, "plugin_off", "The Gaming plugin is off: switch it on in Settings"); };
    let mut state = gaming.state.lock();
    if let Err(response) = gate(&state, hub, request) { return response; }
    if let ["display" | "overlay"] = rest {
        drop(state);
        return overlay_route(&gaming, request, rest);
    }
    let dead = state.lease.as_ref().is_some_and(|lease| holder_dead(&lease.holder));
    if let Err(e) = state.tick(Instant::now(), dead, true) { return error_response(500, "restore_failed", &e); }
    let result = match (request.method.as_str(), rest) {
        ("GET", ["status"]) => return ok(state.status()),
        ("POST", ["lease"]) => {
            let mut acquire: Acquire = match body(request) { Ok(value) => value, Err(response) => return response };
            if acquire.controls.len() > Control::ALL.len() { return error_response(400, "bad_request", "too many controls"); }
            acquire.controls.sort_unstable();
            acquire.controls.dedup();
            let holder = match Holder::open(acquire.pid) { Ok(holder) => holder, Err(e) => return error_response(403, "refused", &e.to_string()) };
            state.acquire(holder, acquire.controls, &*gaming.devices, Instant::now()).map(|lease| json!({"lease": lease}))
        }
        ("POST", ["renew"]) | ("DELETE", ["lease"]) => {
            let lease: LeaseId = match body(request) { Ok(value) => value, Err(response) => return response };
            if request.method == "DELETE" { state.release(&lease.lease).map(|()| json!({"restored": true, "lastRestore": state.last_restore})) }
            else { state.renew(&lease.lease, Instant::now()).map(|lease| json!({"lease": lease})) }
        }
        ("POST", ["qualify"]) => {
            if request.header(crate::http::CLIENT_HEADER) != Some(crate::http::UI_CLIENT) {
                return error_response(403, "refused", "qualification is available only to the TV Settings UI");
            }
            let qualify: Qualify = match body(request) { Ok(value) => value, Err(response) => return response };
            if !receipt(&qualify.live_readback_receipt) || !receipt(&qualify.lifecycle_receipt) {
                return error_response(400, "bad_request", "receipts must be non-empty and at most 1024 bytes");
            }
            state.qualify(qualify.control, &*gaming.devices, qualify.live_readback_receipt, qualify.lifecycle_receipt).map(|record| json!({"qualification": record}))
        }
        ("POST", ["restore"]) => {
            if state.lease.is_some() { Err("release the current lease before replaying rollback".into()) }
            else { state.recover(&*gaming.devices).map(|()| { state.error = None; json!({"restored": true, "lastRestore": state.last_restore}) }) }
        }
        _ => unreachable!(),
    };
    gaming.signal();
    match result { Ok(value) => ok(value), Err(e) => error_response(409, "gaming_failed", &e) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(peer: &str) -> Request {
        Request { method: "POST".into(), path: "/v1/gaming/lease".into(), query: vec![], headers: vec![], body: vec![], peer: peer.parse().unwrap() }
    }

    #[test]
    fn gate_refuses_lan_off_and_standby_before_any_privilege() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State::new(dir.path().join("journal"), dir.path().join("records"), "boot".into());
        let hub = Hub::new(0);
        assert_eq!(gate(&state, &hub, &request("127.0.0.1")).unwrap_err().status, 403);
        state.enabled = true;
        assert_eq!(gate(&state, &hub, &request("192.168.1.2")).unwrap_err().status, 403);
        assert_eq!(gate(&state, &hub, &request("127.0.0.1")).unwrap_err().status, 409);
        hub.set("screen", json!({"on": true}));
        assert!(gate(&state, &hub, &request("127.0.0.1")).is_ok());
    }

    #[test]
    fn lease_request_rejects_unknown_controls_and_arbitrary_parameters() {
        assert!(serde_json::from_str::<Acquire>(r#"{"pid":100,"controls":["memc_low_delay"],"ioctl":1}"#).is_err());
        assert!(serde_json::from_str::<Acquire>(r#"{"pid":100,"controls":["vrr"]}"#).is_err());
        assert!(!receipt(&Some(" ".into())));
    }

    #[test]
    fn pidfd_poll_detects_owner_loss_without_a_clock_or_sleep() {
        let (reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let holder = Holder { pid: 100, fd: reader.into() };
        assert!(!holder_dead(&holder));
        drop(writer);
        assert!(holder_dead(&holder));
    }

    #[test]
    fn first_party_roster_starts_off_and_revokes_an_active_lease_synchronously() {
        use crate::plugins::roster::{Approved, Roster};
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(0));
        let settings = Arc::new(crate::settings::SettingsStore::new(dir.path().join("settings"), hub.clone()));
        let shared = Shared::new(hub.clone(), settings, std::time::Duration::from_secs(1), Arc::new(|| 0));
        let (reader, _writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let gaming = Arc::new(Gaming {
            hub, devices: Box::new(LgDevices::default()),
            state: Mutex::new(State::new(dir.path().join("journal"), dir.path().join("records"), "boot".into())),
            wake: reader.into(), session: Mutex::new(None), display: display::Display::lg(),
            overlay: overlay::Store::load(dir.path().join("overlay.json")),
        });
        let weak = Arc::downgrade(&gaming);
        let roster = Arc::downgrade(&shared);
        shared.offer(&OPT_IN, Arc::new(move || {
            if let (Some(gaming), Some(shared)) = (weak.upgrade(), roster.upgrade()) { gaming.follow(&shared); }
        }));
        assert!(!gaming.state.lock().enabled);
        let approved = Roster { first_party: vec![Approved { id: ID.into(), approve: OPT_IN.approval() }], ..Default::default() };
        shared.set_roster(&approved, None);
        assert!(gaming.state.lock().enabled);
        assert!(gaming.session.lock().is_some());
        assert_eq!(gaming.hub.get("plugin.gaming.tiles").unwrap()["items"][0]["state"]["index"], 0);
        shared.invoke_action(ID, "stats-mode", Some("compact".into())).unwrap();
        assert_eq!(gaming.overlay.get().mode, overlay::Mode::Compact);
        assert_eq!(gaming.hub.get(overlay::TOPIC).unwrap()["mode"], "compact");
        assert_eq!(gaming.hub.get("plugin.gaming.tiles").unwrap()["items"][0]["state"]["index"], 1);
        let rejected = shared.invoke_action(ID, "stats-mode", Some("not-a-choice".into())).unwrap();
        let events = gaming.hub.events_after(Some(0), None, Some(&["plugin.gaming".into()]));
        let answer = events.events.iter()
            .find(|event| event.kind == "action_result" && event.data()["op"] == rejected)
            .expect("the invalid choice receives an action result");
        assert_eq!(answer.data()["outcome"]["type"], "failed");
        assert_eq!(gaming.overlay.get().mode, overlay::Mode::Compact);
        let mut update = request("127.0.0.1");
        update.path = "/v1/gaming/overlay".into();
        update.body = br#"{"mode":"full"}"#.to_vec();
        assert_eq!(overlay_route(&gaming, &update, &["overlay"]).status, 200);
        assert_eq!(gaming.hub.get("plugin.gaming.tiles").unwrap()["items"][0]["state"]["index"], 2);
        gaming.state.lock().acquire(Holder { pid: 100, fd: fs::File::open("/dev/null").unwrap().into() }, vec![], &*gaming.devices, Instant::now()).unwrap();
        shared.set_roster(&Roster::default(), None);
        assert!(!gaming.state.lock().enabled);
        assert!(gaming.state.lock().lease.is_none());
        assert!(gaming.session.lock().is_none());
        assert!(shared.invoke_action(ID, "stats-mode", Some("off".into())).is_err());
        assert!(gaming.set_overlay(overlay::Mode::Off, None).is_err());
        assert!(gaming.hub.get("plugin.gaming.tiles").is_none_or(|topic| topic["items"].as_array().is_none_or(Vec::is_empty)));
    }
}
