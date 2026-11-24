//! The optional gaming lease is renewed only when the UI loop ticks, never by an independent heartbeat.
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, SyncSender}};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const RENEW_MS: u64 = 2_000;
const TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum GameSirState {
    #[default]
    Idle,
    Off,
    Missing,
    Mode { device: String },
    Ready(GameSirProfile),
}
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct GameSirProfile {
    pub device: String,
    pub firmware: String,
    pub receiver: String,
    pub profile: u8,
    pub profile_name: String,
    pub verified: bool,
    pub values: GameSirValues,
}
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct GameSirValues {
    pub report_rate: Option<u16>,
    pub rumble_left: u8,
    pub rumble_right: u8,
    pub dpad_diagonal_lock: bool,
    pub xinput_abxy_swap: bool,
    pub deadzone_left: u8,
    pub deadzone_right: u8,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerChange { pub setting: std::borrow::Cow<'static, str>, pub value: Value }
#[derive(Clone, Debug, Default)]
pub struct GameSir {
    pub state: GameSirState,
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum LeadjoyState {
    #[default]
    Idle,
    Off,
    Missing,
    Unsupported { device: String, firmware: String },
    Ready(LeadjoyProfile),
}
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct LeadjoyProfile {
    pub device: String,
    pub firmware: String,
    pub verified: bool,
    pub view: LeadjoyView,
    pub schema: lumagame_core::settings::SettingsSchema,
    pub values: lumagame_core::settings::Values,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LeadjoyView {
    pub section: String,
    pub profile: u8,
    pub sub: u8,
    pub side: u8,
    pub button: u8,
    pub action: u8,
    #[serde(default)]
    pub step: u16,
    #[serde(default = "first_leadjoy_page")]
    pub page: u16,
}
impl Default for LeadjoyView {
    fn default() -> Self { Self { section: "general".into(), profile: 1, sub: 0, side: 0, button: 1, action: 0, step: 1, page: 1 } }
}
fn first_leadjoy_page() -> u16 { 1 }
#[derive(Clone, Debug, Default)]
pub struct Leadjoy { pub state: LeadjoyState, pub view: LeadjoyView, pub pending: bool, pub error: Option<String> }
pub enum ControllerRequest {
    GameSir(Option<ControllerChange>),
    Leadjoy { view: LeadjoyView, change: Option<ControllerChange> },
}
pub enum ControllerReply { GameSir(Result<GameSirState, String>), Leadjoy(Result<LeadjoyState, String>) }
pub struct ControllerClient {
    jobs: SyncSender<ControllerRequest>,
    results: Receiver<ControllerReply>,
}
impl ControllerClient {
    pub fn new(address: SocketAddr) -> io::Result<Self> {
        let (jobs, requests) = mpsc::sync_channel::<ControllerRequest>(1);
        let (report, results) = mpsc::sync_channel(1);
        thread::Builder::new().name("controller-settings".into()).spawn(move || {
            while let Ok(request) = requests.recv() {
                let (change, view) = match request {
                    ControllerRequest::GameSir(change) => (change, None),
                    ControllerRequest::Leadjoy { view, change } => (change, Some(view)),
                };
                let (method, mut body) = change.map_or(("GET", Value::Null), |change| ("POST", json!({"setting":change.setting, "value":change.value})));
                let path = if let Some(view) = &view {
                    if method == "POST" { body["view"] = json!(view); }
                    if method == "GET" { format!("/v1/gaming/leadjoy?section={}&profile={}&sub={}&side={}&button={}&action={}&step={}&page={}", view.section, view.profile, view.sub, view.side, view.button, view.action, view.step, view.page) }
                    else { "/v1/gaming/leadjoy".into() }
                } else { "/v1/gaming/gamesir".into() };
                let result = http_timeout(address, method, &path, &body, Duration::from_secs(30));
                let reply = if view.is_some() { ControllerReply::Leadjoy(leadjoy_reply(result)) } else { ControllerReply::GameSir(controller_reply(result)) };
                if report.send(reply).is_err() { break; }
            }
        })?;
        Ok(Self { jobs, results })
    }
    pub fn request(&self, request: ControllerRequest) -> Result<(), String> {
        self.jobs.try_send(request).map_err(|error| format!("Controller request could not start: {error}"))
    }
    pub fn poll(&self) -> Option<ControllerReply> { self.results.try_recv().ok() }
}
fn controller_reply<T: serde::de::DeserializeOwned>(result: io::Result<Value>) -> Result<T, String> {
    let value = match result {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => json!({"state":"off"}),
        Err(error) => return Err(format!("Controller settings are unavailable: {error}")),
    };
    serde_json::from_value(value).map_err(|error| format!("The controller helper sent an unreadable reply: {error}"))
}
fn leadjoy_reply(result: io::Result<Value>) -> Result<LeadjoyState, String> {
    let state = controller_reply(result)?;
    if let LeadjoyState::Ready(profile) = &state {
        profile.schema.validate().map_err(|error| format!("The controller helper sent invalid settings: {error}"))?;
        for (key, value) in &profile.values {
            profile.schema.check(key, value).map_err(|_| format!("The controller helper sent an invalid value for {key}"))?;
        }
        if profile.schema.fields().any(|field| field.kind.default().is_some() && !profile.values.contains_key(&field.key)) {
            return Err("The controller helper omitted a settings value".into());
        }
    }
    Ok(state)
}

/// What the stats worker reads from the helper besides the overlay setting: the panel's state, every `every_ms`, with the firmware words when `full`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample { pub every_ms: u64, pub full: bool }
impl Sample {
    /// The retained HUD's reading: four times a second while it is on screen, with the firmware words in Full.
    pub fn hud(overlay: super::stats::Overlay, visible: bool) -> Option<Self> {
        (visible && overlay.mode.on()).then(|| Self { every_ms: super::stats::INTERVAL_MS, full: overlay.mode == super::stats::Mode::Full })
    }
    /// Two readers of the same endpoint share one request, at the faster rate and with the firmware words if either wants them.
    pub fn merge(a: Option<Self>, b: Option<Self>) -> Option<Self> {
        match (a, b) {
            (Some(a), Some(b)) => Some(Self { every_ms: a.every_ms.min(b.every_ms), full: a.full || b.full }),
            (a, b) => a.or(b),
        }
    }
}

/// One worker owns helper I/O. A single queued wake always reads the newest desired setting.
pub struct OverlayClient {
    wanted: Arc<std::sync::Mutex<(super::stats::Overlay, Option<Sample>)>>,
    wake: SyncSender<()>,
    results: Receiver<(super::stats::Overlay, Option<Value>)>,
    due: u64,
}
impl OverlayClient {
    pub fn new(address: SocketAddr) -> io::Result<Self> {
        let wanted = Arc::new(std::sync::Mutex::new((super::stats::Overlay::default(), None)));
        let state = Arc::clone(&wanted);
        let (wake, requests) = mpsc::sync_channel(1);
        let (report, results) = mpsc::sync_channel(1);
        thread::Builder::new().name("game-stats".into()).spawn(move || {
            while requests.recv().is_ok() {
                let (local, sample) = *state.lock().unwrap();
                if let Ok(value) = overlay_sync(local, sample, &mut |method, path, body| http(address,method,path,body))
                    && report.send(value).is_err() { break; }
            }
        })?;
        Ok(Self { wanted, wake, results, due: 0 })
    }
    pub fn tick(&mut self, overlay: super::stats::Overlay, sample: Option<Sample>, now: u64) {
        let mut state = self.wanted.lock().unwrap();
        let changed = *state != (overlay, sample);
        *state = (overlay, sample);
        if changed || now >= self.due {
            let _ = self.wake.try_send(());
            self.due = now.saturating_add(sample.map_or(1000, |s| s.every_ms));
        }
    }
    pub fn poll(&self) -> Option<(super::stats::Overlay, Option<Value>)> { self.results.try_recv().ok() }
    pub fn wait_ms(&self, now: u64) -> u64 { self.due.saturating_sub(now) }
}

fn overlay_sync(local: super::stats::Overlay, sample: Option<Sample>, http: &mut impl FnMut(&str,&str,&Value)->io::Result<Value>)
    -> io::Result<(super::stats::Overlay,Option<Value>)> {
    let path = match sample {
        Some(Sample { full: true, .. }) => "/v1/gaming/display?full=1",
        Some(_) => "/v1/gaming/display",
        None => "/v1/gaming/overlay",
    };
    let mut value=http("GET",path,&Value::Null)?;
    let mut overlay:super::stats::Overlay=serde_json::from_value(value["overlay"].take())?;
    if local.changed > overlay.changed {
        let mut saved=http("POST","/v1/gaming/overlay",&json!(local))?;
        overlay=serde_json::from_value(saved["overlay"].take())?;
    }
    Ok((overlay,value.get_mut("display").map(Value::take)))
}

pub struct Lease {
    wanted: Arc<AtomicBool>,
    wake: Option<SyncSender<()>>,
    notices: Receiver<String>,
    worker: Option<JoinHandle<()>>,
    due: u64,
}

impl Lease {
    pub fn new(address: SocketAddr) -> io::Result<Self> {
        let wanted = Arc::new(AtomicBool::new(false));
        let active = wanted.clone();
        let (wake, requests) = mpsc::sync_channel(1);
        let (report, notices) = mpsc::sync_channel(1);
        let worker = thread::Builder::new().name("gaming-lease".into()).spawn(move || {
            let mut lease = LeaseState::default();
            let mut last_error = None;
            let mut http = |method: &str, path: &str, body: &Value| request(address, method, path, body);
            while requests.recv().is_ok() {
                let result = lease.sync(active.load(Ordering::Acquire), &mut http);
                if let Err(why) = result {
                    if last_error.as_ref() != Some(&why) { let _ = report.try_send(why.clone()); }
                    last_error = Some(why);
                } else { last_error = None; }
            }
            let _ = lease.sync(false, &mut http);
        })?;
        Ok(Self { wanted, wake: Some(wake), notices, worker: Some(worker), due: 0 })
    }

    pub fn set(&mut self, wanted: bool, now_ms: u64) {
        if self.wanted.swap(wanted, Ordering::AcqRel) != wanted {
            self.due = now_ms;
            self.notify();
            self.due = now_ms.saturating_add(RENEW_MS);
        }
    }

    pub fn tick(&mut self, now_ms: u64) {
        if self.wanted.load(Ordering::Acquire) && now_ms >= self.due {
            self.notify();
            self.due = now_ms.saturating_add(RENEW_MS);
        }
    }

    fn notify(&self) { if let Some(wake) = &self.wake { let _ = wake.try_send(()); } }
    pub fn notice(&self) -> Option<String> { self.notices.try_recv().ok() }
    /// Whether the pads are leased, or asked for and not yet given.
    pub fn held(&self) -> bool { self.wanted.load(Ordering::Acquire) }
    pub fn wait_ms(&self, now_ms: u64) -> u64 { if self.wanted.load(Ordering::Acquire) { self.due.saturating_sub(now_ms) } else { u64::MAX } }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.wanted.store(false, Ordering::Release);
        self.wake = None;
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[derive(Default)]
struct LeaseState { id: Option<Value> }

impl LeaseState {
    fn sync(&mut self, wanted: bool, http: &mut impl FnMut(&str, &str, &Value) -> Result<Value, String>) -> Result<(), String> {
        if !wanted {
            if let Some(id) = self.id.take() { http("DELETE", "/v1/gaming/lease", &json!({"lease":id}))?; }
            return Ok(());
        }
        if let Some(id) = &self.id {
            if let Err(why) = http("POST", "/v1/gaming/renew", &json!({"lease":id})) {
                let _ = http("DELETE", "/v1/gaming/lease", &json!({"lease":id}));
                self.id = None;
                return Err(why);
            }
        } else {
            let result = http("POST", "/v1/gaming/lease", &json!({"pid":std::process::id()}))?;
            let id = result.get("lease").and_then(|lease| lease.get("id")).filter(|id| id.is_string() || id.is_u64()).ok_or("Gaming helper returned no lease; play continues without it.")?;
            self.id = Some(id.clone());
        }
        Ok(())
    }
}

fn request(address: SocketAddr, method: &str, path: &str, body: &Value) -> Result<Value, String> {
    http(address, method, path, body).map_err(|e| format!("Gaming helper is unavailable; play continues without it ({e})."))
}

fn http(address: SocketAddr, method: &str, path: &str, body: &Value) -> io::Result<Value> {
    http_timeout(address, method, path, body, TIMEOUT)
}

fn http_timeout(address: SocketAddr, method: &str, path: &str, body: &Value, timeout: Duration) -> io::Result<Value> {
    if !address.ip().is_loopback() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "the gaming helper must be on loopback")); }
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let body = body.to_string();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nX-Luma-Client: lumagame\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
    let mut raw = Vec::new();
    stream.take(64 * 1024 + 1).read_to_end(&mut raw)?;
    if raw.len() > 64 * 1024 { return Err(io::Error::new(io::ErrorKind::InvalidData, "helper reply is too large")); }
    parse_reply(&raw)
}

fn parse_reply(raw: &[u8]) -> io::Result<Value> {
    let text = std::str::from_utf8(raw).map_err(io::Error::other)?;
    let (headers, body) = text.split_once("\r\n\r\n").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "helper sent no HTTP headers"))?;
    let status = headers.lines().next().and_then(|line| line.split_whitespace().nth(1)).and_then(|code| code.parse::<u16>().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "helper sent no status"))?;
    let reply: Value = serde_json::from_str(body).map_err(io::Error::other)?;
    if status == 403 { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "enable the Gaming plugin in LumaTV to use its controls")); }
    if status != 200 || reply.get("ok") != Some(&Value::Bool(true)) {
        let message = reply.pointer("/error/message").and_then(Value::as_str).unwrap_or("request was not accepted");
        return Err(io::Error::other(format!("{status}: {message}")));
    }
    Ok(reply)
}

#[cfg(test)]
pub(crate) fn gamesir_fixture() -> GameSirState {
    serde_json::from_value(json!({"ok":true, "state":"ready", "device":"GameSir Cyclone 2", "firmware":"1.2.3", "receiver":"1.0.0",
        "profile":1, "profile_name":"Default", "verified":true, "values":{"report_rate":500, "rumble_left":4, "rumble_right":3,
        "dpad_diagonal_lock":false, "xinput_abxy_swap":false, "deadzone_left":5, "deadzone_right":5}})).unwrap()
}

/// The helper's reply for one settings section, as the helper's own test generates it (helper/app/src/leadjoy.rs checks this file).
#[cfg(test)]
pub(crate) fn leadjoy_fixture(section: &str) -> LeadjoyState {
    let all: Value = serde_json::from_str(include_str!("leadjoy-fixtures.json")).unwrap();
    leadjoy_reply(Ok(all[section].clone())).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_leadjoy_fixture_fits_and_validates_as_a_real_reply() {
        let all: Value = serde_json::from_str(include_str!("leadjoy-fixtures.json")).unwrap();
        for (name, value) in all.as_object().unwrap() {
            assert!(serde_json::to_vec(value).unwrap().len() < 64 * 1024 - 512, "{name}");
            assert!(matches!(leadjoy_reply(Ok(value.clone())), Ok(LeadjoyState::Ready(_))), "{name}");
        }
        let mut bad = all["keyboard"].clone();
        let options = bad["schema"]["sections"][0]["fields"].as_array_mut().unwrap().iter_mut()
            .find(|field| field["key"] == "key-0").unwrap()["options"].as_array_mut().unwrap();
        options.push(json!({"id":"999","label":"Extra"}));
        assert!(leadjoy_reply(Ok(bad)).is_err(), "more than 32 choices must be rejected");
    }

    #[test]
    fn acquire_renew_release_and_failed_renew_are_best_effort() {
        let mut state = LeaseState::default();
        let mut seen = Vec::new();
        let mut http = |method: &str, path: &str, body: &Value| {
            seen.push((method.to_owned(), path.to_owned(), body.clone()));
            Ok(json!({"ok":true,"lease":{"id":"one"}}))
        };
        state.sync(false, &mut http).unwrap();
        state.sync(true, &mut http).unwrap();
        state.sync(true, &mut http).unwrap();
        state.sync(false, &mut http).unwrap();
        assert_eq!(seen.iter().map(|(m,p,_)| (m.as_str(),p.as_str())).collect::<Vec<_>>(), [("POST","/v1/gaming/lease"),("POST","/v1/gaming/renew"),("DELETE","/v1/gaming/lease")]);
        assert_eq!(seen[1].2, json!({"lease":"one"}));
        state.id = Some(json!("one"));
        assert!(state.sync(true, &mut |_,_,_| Err("helper restarted".into())).is_err());
        assert!(state.id.is_none());
    }

    #[test]
    fn renewal_needs_a_ui_tick_every_two_seconds() {
        let (wake, requests) = mpsc::sync_channel(1);
        let (_report, notices) = mpsc::sync_channel(1);
        let mut lease = Lease { wanted: Arc::new(AtomicBool::new(false)), wake: Some(wake), notices, worker: None, due: 0 };
        lease.set(true, 10);
        assert!(requests.try_recv().is_ok());
        lease.tick(2009);
        assert!(requests.try_recv().is_err());
        lease.tick(2010);
        assert!(requests.try_recv().is_ok());
        lease.set(false, 2011);
        assert!(requests.try_recv().is_ok());
        lease.tick(10_000);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn helper_off_is_not_a_game_failure() {
        let reply = b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\n\r\n{\"ok\":false,\"error\":{\"code\":\"plugin_off\"}}";
        assert_eq!(parse_reply(reply).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\n\r\n{\"ok\":true}").is_ok());
    }

    #[test]
    fn overlay_sync_samples_only_when_asked_and_saves_only_the_newer_setting() {
        use super::super::stats::{Mode,Overlay};
        for (sample,path) in [(None,"/v1/gaming/overlay"),(Some(Sample {every_ms:250,full:false}),"/v1/gaming/display"),(Some(Sample {every_ms:1000,full:true}),"/v1/gaming/display?full=1")] {
            let local=Overlay {mode:Mode::Compact,changed:2};
            let mut calls=Vec::new();
            let (setting,_)=overlay_sync(local,sample,&mut |method,p,body| {
                calls.push((method.to_owned(),p.to_owned()));
                Ok(json!({"overlay":if method=="POST" {body.clone()} else {json!({"mode":"off","changed":1})}}))
            }).unwrap();
            assert_eq!(setting,local);
            assert_eq!(calls,[("GET".into(),path.into()),("POST".into(),"/v1/gaming/overlay".into())]);
        }
        let (setting,_)=overlay_sync(Overlay::default(),None,&mut |method,_,_| {
            assert_eq!(method,"GET");
            Ok(json!({"overlay":{"mode":"full","changed":4}}))
        }).unwrap();
        assert_eq!(setting.mode,Mode::Full);
    }

    #[test]
    fn the_hud_and_diagnostics_share_one_reading() {
        use super::super::stats::{Mode,Overlay};
        let hud=|mode,visible| Sample::hud(Overlay {mode,changed:0},visible);
        assert_eq!(hud(Mode::Off,true),None);
        assert_eq!(hud(Mode::Compact,false),None);
        assert_eq!(hud(Mode::Compact,true),Some(Sample {every_ms:250,full:false}));
        assert_eq!(hud(Mode::Full,true),Some(Sample {every_ms:250,full:true}));
        let diagnostics=Some(Sample {every_ms:1000,full:true});
        assert_eq!(Sample::merge(hud(Mode::Compact,true),diagnostics),Some(Sample {every_ms:250,full:true}));
        assert_eq!(Sample::merge(None,diagnostics),diagnostics);
        assert_eq!(Sample::merge(None,None),None);
    }
}
