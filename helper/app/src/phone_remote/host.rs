//! The TV as the phone remote's engine drives it ([`Host`]). Keys, the pointer, text and edits go to the compositor's control socket as
//! the remote's own would; actions go to the shell, LG's services and the helper's plugins.

use crate::ai::luna::LunaTv;
use crate::ai::types::ActionError;
use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::plugins::session::Shared;
use crate::{receiver, screen};
use lumaremote::Fault;
use lumaremote::datagram::{Button, Event};
use lumaremote::proto::{Act, Edit, List};
use lumaremote::server::Host;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// How long the compositor may take to answer one line.
const WAIT: Duration = Duration::from_secs(2);
/// After a connection fails, the next is tried no sooner: pointer datagrams come 250 times a second.
const REOPEN_AFTER: Duration = Duration::from_secs(1);
/// The longest answer read: `keys` lists the whole key table.
const MAX_ANSWER: u64 = 256 * 1024;
/// The scene the pointer moves in, in pixels.
const SCENE: (i32, i32) = (1920, 1080);
/// One step of the wheel, in the phone's 1/16 px of scrolling.
const WHEEL_STEP: i32 = 48 * 16;
/// The most wheel steps one datagram makes; a fling beyond them is dropped.
const MOST_STEPS: u32 = 16;
/// The pointer is sent again this often while a phone points without moving: the compositor hides it after 10 s without motion.
const KEEP_SHOWN: Duration = Duration::from_secs(5);
const HOME: u32 = 773;
const MENU: u32 = 139;
const GUIDE: u32 = 362;
const INPUT: u32 = 241;
const BACK: u32 = 412;
/// The Magic Remote's cursor-off report: the compositor hides the pointer.
const CURSOR_OFF: u32 = 1199;
const LIVE_TV: &str = "com.webos.app.livetv";
const LAUNCH_URI: &str = "luna://com.webos.applicationManager/launch";
const SETTINGS_URI: &str = "luna://com.webos.service.settings/setSystemSettings";
const READ_SETTINGS_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
/// LumaTV's own windows: a text field in one is LumaTV's.
const OWN_APPS: &str = "org.nphil.lumatv";

/// What the host needs of the rest of the helper.
pub struct Deps {
    pub tv: Arc<dyn lumatuner::Hardware>,
    pub cameras: crate::scrypted::Scrypted,
    pub receiver: Sender<receiver::Cmd>,
}

pub struct TvHost {
    hub: Arc<Hub>,
    shared: Arc<Shared>,
    /// Keys, text, edits, actions and the state's reads.
    ctl: Mutex<Link>,
    /// Pointer datagrams, so that they never wait behind the rest.
    pointer: Mutex<(Link, Aim)>,
    luna: LunaTv,
    theme: PathBuf,
    deps: Deps,
    changed: Box<dyn Fn() + Send + Sync>,
}

impl TvHost {
    pub fn new(ctx: &Ctx, shared: Arc<Shared>, deps: Deps, changed: impl Fn() + Send + Sync + 'static) -> Self {
        let control = &ctx.config.capture.control;
        Self {
            hub: ctx.hub.clone(),
            shared,
            ctl: Mutex::new(Link::new(control)),
            pointer: Mutex::new((Link::new(control), Aim::default())),
            luna: LunaTv::new(ctx.config.luna.send.clone()),
            theme: ctx.paths.theme_file(),
            deps,
            changed: Box::new(changed),
        }
    }

    /// The TV's name as LG's settings have it (Settings › Support shows the same), which phones list the TV by.
    pub fn tv_name(&self) -> String {
        let reply = self.luna.call(READ_SETTINGS_URI, &json!({"category": "network", "keys": ["deviceName"]}));
        reply
            .ok()
            .and_then(|r| r["settings"]["deviceName"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned))
            .unwrap_or_else(|| "LumaTV".to_owned())
    }

    /// No phone is in session: the compositor stops taking text and edits from the helper.
    pub fn session_off(&self) {
        self.ctl.lock().session_off();
    }

    /// The plugin is off: both connections close, which ends the compositor's phone session with them.
    pub fn close(&self) {
        self.ctl.lock().close();
        self.pointer.lock().0.close();
    }

    /// Whether the TV is on for the phone: the panel is lit, or only the picture is off (keys bring it back).
    fn power_on(&self) -> bool {
        screen::is_on(&self.hub) || self.hub.get(screen::TOPIC).is_some_and(|s| s["state"] == "Screen Off")
    }

    fn cameras_on(&self) -> bool {
        self.shared.switched_on(crate::scrypted::options::OWNER)
    }

    fn receiver_on(&self) -> bool {
        self.shared.switched_on(crate::receiver_plugin::ID)
    }

    /// `shell show KIND [PAGE]`, which the shell must take.
    fn show(&self, kind: &str, page: Option<&str>) -> Result<(), Fault> {
        let line = match page {
            None => format!("shell show {kind}"),
            Some(page) if word(page) => format!("shell show {kind} {page}"),
            Some(_) => return Err(Fault::NotFound),
        };
        match self.ctl.lock().tell(&line)?["taken"].as_bool() {
            Some(true) => Ok(()),
            _ => Err(Fault::Unavailable),
        }
    }

    fn luna(&self, uri: &str, payload: &Value) -> Result<(), Fault> {
        self.luna.call(uri, payload).map(drop).map_err(fault)
    }

    fn channels(&self) -> Result<Vec<lumatuner::docs::Entry>, Fault> {
        self.deps.tv.channel_list().map(|reply| lumatuner::docs::entries(&reply)).map_err(|_| Fault::Unavailable)
    }
}

impl Host for TvHost {
    fn key(&self, code: u32, down: Option<bool>) -> Result<(), Fault> {
        let how = match down {
            Some(true) => "press",
            Some(false) => "release",
            None => "tap",
        };
        self.ctl.lock().tell(&format!("remote {code} {how}")).map(drop)
    }

    fn text(&self, text: &str) -> Result<(), Fault> {
        let line = format!("phone text {}", esphome_api::base64::encode(text.as_bytes()));
        field(&self.ctl.lock().phone(&line)?)
    }

    fn edit(&self, edit: Edit) -> Result<(), Fault> {
        field(&self.ctl.lock().phone(&edit_line(edit))?)
    }

    fn pointer(&self, events: &[Event], engaged: bool) {
        let mut pointer = self.pointer.lock();
        let (link, aim) = &mut *pointer;
        if events.first() == Some(&Event::Engaged(true))
            && let Ok(report) = link.ask("pointer")
            && let Some([x, y]) = report["pointer"]["at"].as_array().map(Vec::as_slice)
        {
            aim.x = (x.as_f64().unwrap_or(0.0) * 16.0) as i32;
            aim.y = (y.as_f64().unwrap_or(0.0) * 16.0) as i32;
        }
        for line in aim.lines(events, engaged, Instant::now()) {
            if link.ask(&line).is_err() {
                break;
            }
        }
    }

    fn act(&self, act: &Act) -> Result<(), Fault> {
        match act {
            Act::Home => self.key(HOME, None),
            Act::Quick => self.key(MENU, None),
            Act::Guide => self.key(GUIDE, None),
            Act::Input => self.key(INPUT, None),
            Act::Power(on) if *on == self.power_on() => Ok(()),
            Act::Power(_) => self.key(lumaremote::proto::POWER, None),
            Act::Settings(page) => self.show("settings", page.as_deref()),
            Act::Notifications => self.show("drawer", None),
            Act::Cameras | Act::CameraClose if !self.cameras_on() => Err(Fault::Off),
            Act::Cameras => self.show("cameras", None),
            Act::CameraClose => {
                self.deps.cameras.inner.clear_popup_if(None);
                Ok(())
            }
            // One camera's popup on request is not built: the Scrypted popup opens only for its own events.
            Act::Camera(_) => Err(Fault::Unavailable),
            Act::PictureOff => self.luna(SETTINGS_URI, &json!({"category": "option", "settings": {"screenOff": "on"}})),
            Act::Launch(app) => self.luna.launch_app(app).map(drop).map_err(fault),
            Act::Tune(channel) => {
                if !self.channels()?.iter().any(|c| c.channel_id == *channel) {
                    return Err(Fault::NotFound);
                }
                let params = json!({"id": "channelChange", "channelId": channel, "sender": "livemenu", "type": "channels"});
                self.luna(LAUNCH_URI, &json!({"id": LIVE_TV, "params": params}))
            }
            Act::Volume(_) if !self.receiver_on() => Err(Fault::Off),
            Act::Volume(halves) => {
                let max = self.hub.get(receiver::TOPIC).and_then(|r| r["volumeMax"].as_f64()).unwrap_or(crate::receiver_volume::MAX_LEVEL);
                let level = (f64::from(*halves) / 2.0).min(max).min(crate::receiver_volume::MAX_LEVEL);
                self.deps.receiver.send(receiver::Cmd::Volume((level * 2.0) as u16)).map_err(|_| Fault::Unavailable)
            }
        }
    }

    fn list(&self, what: List) -> Result<Vec<Value>, Fault> {
        Ok(match what {
            List::Apps => self.luna.launchable_apps().map_err(fault)?.into_iter().map(|a| json!({"id": a.id, "title": a.title})).collect(),
            List::Channels => self.channels()?.into_iter().map(|c| json!({"id": c.channel_id, "no": c.guide, "name": c.name})).collect(),
            List::Cameras if !self.cameras_on() => return Err(Fault::Off),
            List::Cameras => {
                let view = self.deps.cameras.inner.view();
                view["cameras"].as_array().into_iter().flatten().map(|c| json!({"id": c["id"], "name": c["name"]})).collect()
            }
        })
    }

    fn state(&self) -> Map<String, Value> {
        let (keys, ime) = {
            let mut ctl = self.ctl.lock();
            (ctl.ask("keys").ok(), ctl.ask("ime").ok())
        };
        let receiver = self.receiver_on().then(|| self.hub.get(receiver::TOPIC).unwrap_or_default());
        let mut state = Map::new();
        state.insert("power".into(), json!(if self.power_on() { "on" } else { "standby" }));
        state.insert("fg".into(), json!(keys.as_ref().and_then(|k| k["foreground"].as_str()).unwrap_or("")));
        state.insert("ime".into(), ime_state(ime.as_ref()));
        state.insert("volume".into(), volume_state(receiver.as_ref()));
        state.insert("acts".into(), json!(acts(self.cameras_on(), receiver.is_some())));
        state
    }

    fn accent(&self) -> String {
        crate::tuner::look::accent(&self.theme).unwrap_or_else(|| "#ffffff".to_owned())
    }

    fn changed(&self) {
        (self.changed)();
    }
}

/// One connection to the compositor's control socket, opened when first needed and again after it fails. One line, one answer.
struct Link {
    path: PathBuf,
    open: Option<BufReader<UnixStream>>,
    failed: Option<Instant>,
    /// This connection said `phone session on`: the compositor takes `phone text` and `phone edit` only from such a connection.
    phone: bool,
}

impl Link {
    fn new(path: &str) -> Self {
        Self { path: path.into(), open: None, failed: None, phone: false }
    }

    /// Sends `line` and reads its answer. A connection that was open already is given one more try on a new one: the compositor may have
    /// restarted since.
    fn ask(&mut self, line: &str) -> Result<Value, Fault> {
        let reused = self.open.is_some();
        let answer = self.exchange(line).or_else(|e| {
            self.close();
            if reused { self.exchange(line) } else { Err(e) }
        });
        answer.map_err(|_| {
            self.close();
            self.failed = Some(Instant::now());
            Fault::Unavailable
        })
    }

    fn exchange(&mut self, line: &str) -> io::Result<Value> {
        if self.open.is_none() {
            if self.failed.is_some_and(|at| at.elapsed() < REOPEN_AFTER) {
                return Err(io::ErrorKind::NotConnected.into());
            }
            let stream = UnixStream::connect(&self.path)?;
            stream.set_read_timeout(Some(WAIT))?;
            stream.set_write_timeout(Some(WAIT))?;
            self.open = Some(BufReader::new(stream));
        }
        let reader = self.open.as_mut().expect("opened above");
        reader.get_mut().write_all(format!("{line}\n").as_bytes())?;
        let mut answer = String::new();
        reader.take(MAX_ANSWER).read_line(&mut answer)?;
        if answer.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&answer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// `line`, which the compositor must answer `ok`.
    fn tell(&mut self, line: &str) -> Result<Value, Fault> {
        let answer = self.ask(line)?;
        if answer["ok"] == true { Ok(answer) } else { Err(Fault::Unavailable) }
    }

    /// A `phone` line, in a phone session: said on this connection first, and again when the line went out on a new connection.
    fn phone(&mut self, line: &str) -> Result<Value, Fault> {
        for _ in 0..2 {
            if !self.phone {
                self.tell("phone session on")?;
                self.phone = true;
            }
            let answer = self.ask(line)?;
            if self.phone {
                return Ok(answer);
            }
        }
        Err(Fault::Unavailable)
    }

    fn session_off(&mut self) {
        if self.phone {
            let _ = self.ask("phone session off");
            self.phone = false;
        }
    }

    fn close(&mut self) {
        self.open = None;
        self.phone = false;
    }
}

/// Where the phone's pointer is, in 1/16 px of the scene, and what is left of its scrolling.
#[derive(Default)]
struct Aim {
    x: i32,
    y: i32,
    scroll: i32,
    /// The pixel last sent, and when.
    sent: Option<((i32, i32), Instant)>,
}

impl Aim {
    /// The control lines one datagram's events make, in their order. The pointer goes once per run of motion, to where it ends, before a
    /// click or a scroll that follows; and again every [`KEEP_SHOWN`] while the phone points without moving.
    fn lines(&mut self, events: &[Event], engaged: bool, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        let mut moved = false;
        for event in events {
            match *event {
                Event::Engaged(true) => moved = true,
                Event::Engaged(false) => {
                    self.go(&mut out, moved, now);
                    moved = false;
                    self.scroll = 0;
                    self.sent = None;
                    out.push(format!("remote {CURSOR_OFF} tap"));
                }
                Event::Centre => {
                    (self.x, self.y) = (SCENE.0 / 2 * 16, SCENE.1 / 2 * 16);
                    moved = true;
                }
                Event::Move { dx, dy } => {
                    self.x = (self.x + dx).clamp(0, SCENE.0 * 16 - 1);
                    self.y = (self.y + dy).clamp(0, SCENE.1 * 16 - 1);
                    moved = true;
                }
                Event::Scroll { dy, .. } => {
                    self.go(&mut out, moved, now);
                    moved = false;
                    self.scroll += dy;
                    for _ in 0..(self.scroll.unsigned_abs() / WHEEL_STEP as u32).min(MOST_STEPS) {
                        out.push(if self.scroll > 0 { "wheel down" } else { "wheel up" }.to_owned());
                    }
                    self.scroll %= WHEEL_STEP;
                }
                Event::Click(button) | Event::Press(button) | Event::Release(button) => {
                    self.go(&mut out, moved, now);
                    moved = false;
                    let how = match event {
                        Event::Click(_) => "tap",
                        Event::Press(_) => "press",
                        _ => "release",
                    };
                    out.push(match button {
                        Button::Left => format!("click {how}"),
                        // the right button is Back
                        Button::Right => format!("remote {BACK} {how}"),
                    });
                }
            }
        }
        let keep = engaged && self.sent.is_none_or(|(_, at)| now.duration_since(at) >= KEEP_SHOWN);
        self.go(&mut out, moved || keep, now);
        out
    }

    /// The pointer to its pixel, unless that pixel went lately.
    fn go(&mut self, out: &mut Vec<String>, moved: bool, now: Instant) {
        let pixel = (self.x / 16, self.y / 16);
        if !moved || self.sent.is_some_and(|(at, when)| at == pixel && now.duration_since(when) < KEEP_SHOWN) {
            return;
        }
        out.push(format!("pointer {} {}", pixel.0, pixel.1));
        self.sent = Some((pixel, now));
    }
}

/// A `phone text` or `phone edit` answer: the field took it, or there was none.
fn field(answer: &Value) -> Result<(), Fault> {
    if answer["field"] == true { Ok(()) } else { Err(Fault::NoField) }
}

fn edit_line(edit: Edit) -> String {
    match edit {
        Edit::Backspace(n) => format!("phone edit backspace {n}"),
        Edit::Enter => "phone edit enter".into(),
        Edit::Tab => "phone edit tab".into(),
        Edit::Left => "phone edit left".into(),
        Edit::Right => "phone edit right".into(),
        Edit::Clear => "phone edit clear".into(),
        Edit::Hide => "phone edit hide".into(),
        Edit::Show => "phone edit show".into(),
    }
}

/// A word that can go on a control line as it is.
fn word(text: &str) -> bool {
    (1..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn fault(e: ActionError) -> Fault {
    match e.code {
        "not_found" | "refused" => Fault::NotFound,
        _ => Fault::Unavailable,
    }
}

/// The `ime` member from the compositor's `ime` report: a field counts while LumaTV's own keyboard serves it, which is when `phone text`
/// and `phone edit` reach it.
fn ime_state(report: Option<&Value>) -> Value {
    let Some(field) = report.map(|r| &r["active"]).filter(|a| a["own"] == true) else {
        return json!({"active": false});
    };
    let app = field["app"].as_str().unwrap_or("");
    json!({
        "active": true,
        "owner": if app.is_empty() || app.starts_with(OWN_APPS) { "lumatv" } else { "app" },
        "purpose": field["purpose"],
        "hint": field["hint"],
        "enter": field["enter"],
        "max": field["max"],
        "text": field["surround"]["len"].as_u64().is_some_and(|n| n > 0),
    })
}

/// The `volume` member: the receiver's topic while its plugin is on, each member only when the receiver said it. `level` and `max` keep the
/// receiver's half steps (41.5), as its display and the TV's volume bar show them; LumaRemoteKit 1.0.3 decodes them as numbers.
fn volume_state(receiver: Option<&Value>) -> Value {
    let Some(r) = receiver else {
        return json!({"via": "tv"});
    };
    let mut volume = json!({"via": "receiver"});
    for (from, to) in [("volumeAbs", "level"), ("volumeMax", "max")] {
        if let Some(n) = r[from].as_f64() {
            volume[to] = json!(n);
        }
    }
    if let Some(muted) = r["mute"].as_bool() {
        volume["muted"] = json!(muted);
    }
    volume
}

/// The actions that work now.
fn acts(cameras: bool, receiver: bool) -> Vec<&'static str> {
    let mut acts = vec!["home", "quick", "settings", "notifications", "input", "pictureoff", "guide", "tune", "launch", "power"];
    if cameras {
        acts.extend(["cameras", "camera-close"]);
    }
    if receiver {
        acts.push("volume");
    }
    acts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_datagrams_become_control_lines_in_order() {
        let mut aim = Aim { x: 100 * 16, y: 100 * 16, ..Aim::default() };
        let t = Instant::now();
        // engaging shows the pointer where it is; moves within a datagram go once, before the click that follows them
        let lines = aim.lines(&[Event::Engaged(true), Event::Move { dx: 8, dy: 0 }, Event::Move { dx: 24, dy: -16 }, Event::Click(Button::Left)], true, t);
        assert_eq!(lines, ["pointer 102 99", "click tap"]);
        // the right button is Back; motion off the scene stops at its edge
        let lines = aim.lines(&[Event::Move { dx: -1 << 16, dy: 0 }, Event::Press(Button::Right)], true, t);
        assert_eq!(lines, ["pointer 0 99", "remote 412 press"]);
        // a pointer that rests is sent again before the compositor would hide it, and not before
        assert!(aim.lines(&[], true, t + Duration::from_secs(1)).is_empty());
        assert_eq!(aim.lines(&[], true, t + KEEP_SHOWN), ["pointer 0 99"]);
        // scrolling is wheel steps, the rest kept for the next; letting go hides the pointer
        let lines = aim.lines(&[Event::Scroll { dx: 0, dy: WHEEL_STEP * 2 + 5 }], true, t + KEEP_SHOWN);
        assert_eq!(lines, ["wheel down", "wheel down"]);
        let lines = aim.lines(&[Event::Scroll { dx: 0, dy: -WHEEL_STEP - 10 }, Event::Engaged(false)], false, t + KEEP_SHOWN);
        assert_eq!(lines, ["wheel up", "remote 1199 tap"]);
        let lines = aim.lines(&[Event::Engaged(true), Event::Centre], true, t + KEEP_SHOWN);
        assert_eq!(lines, ["pointer 960 540"]);
    }

    #[test]
    fn the_state_tells_a_phone_what_works() {
        let own = json!({"active": {"app": "com.netflix.app", "own": true, "purpose": 6, "hint": 0, "enter": 5, "max": 0, "surround": {"len": 3, "cursor": 3}}});
        assert_eq!(ime_state(Some(&own)), json!({"active": true, "owner": "app", "purpose": 6, "hint": 0, "enter": 5, "max": 0, "text": true}));
        let shell = json!({"active": {"app": "org.nphil.lumatv.rustui", "own": true, "surround": null}});
        assert_eq!(ime_state(Some(&shell))["owner"], "lumatv");
        // a field LG's keyboard serves is out of the phone's reach
        assert_eq!(ime_state(Some(&json!({"active": {"app": "x", "own": false}}))), json!({"active": false}));
        assert_eq!(ime_state(None), json!({"active": false}));

        assert_eq!(volume_state(None), json!({"via": "tv"}));
        let receiver = json!({"volumeAbs": 41.5, "volumeMax": 70.0, "mute": null});
        assert_eq!(volume_state(Some(&receiver)), json!({"via": "receiver", "level": 41.5, "max": 70.0}));
        assert_eq!(volume_state(Some(&json!({"volumeAbs": 57, "mute": true})))["muted"], true);

        assert!(!acts(false, false).iter().any(|a| matches!(*a, "cameras" | "camera" | "volume")));
        assert!(acts(true, true).ends_with(&["cameras", "camera-close", "volume"]));
    }
}
