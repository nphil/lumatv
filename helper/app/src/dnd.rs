//! Do Not Disturb and per-alert snoozes (`docs/dnd.md`): the gate a camera's pop-up and a plugin's note pass on their way to the screen.
//!
//! **Gate.** Every alert has a *kind* (`camera`, or `plugin:<id>`) and a *source* (the camera, the plugin's channel). An alert is held when a
//! snooze of its source is on, or when Do Not Disturb or a snooze of its whole kind is on and it is not a doorbell the owner lets through
//! (`letDoorbell`, on unless turned off). A held camera pop-up is never published and its event goes into `missed`; a held note is published
//! with `held: true`, so it is in the notification list and nothing announces it. The shell's toast feed (LG's) holds outside senders' toasts
//! itself from this topic and records them here (`POST /v1/dnd/missed`).
//!
//! **State.** On or off, when it turns itself off, the snoozes and what was missed, with absolute times in `dnd.json` beside the settings. One
//! thread sleeps until the next expiry; the gate expires what has run out before it answers, so a late wake never lets an alert through.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/dnd` | the topic's document |
//! | `POST /v1/dnd/on` | `{until?: ms or "morning", minutes?}` turns it on; neither: until it is turned off |
//! | `POST /v1/dnd/off` | turns it off |
//! | `POST /v1/dnd/snooze` | `{kind, source?, label?, until?: ms or "morning", minutes?}`: one source, or the whole kind without `source`; replaces one of the same scope |
//! | `DELETE /v1/dnd/snooze/{id}` | ends a snooze |
//! | `POST /v1/dnd/settings` | `{letDoorbell}` |
//! | `POST /v1/dnd/missed` | `{kind, source?, title, body?}`: an alert held outside the helper |
//! | `DELETE /v1/dnd/missed` | forgets what was missed |
//!
//! The topic `dnd`: `{on, until, letDoorbell, snoozes: [{id, kind, source?, label, until}], missed: [{kind, source, title, body, at, by}]}`;
//! `until` is null while it lasts until it is turned off, `by` is `dnd` or `snooze`.

use crate::API_VERSION;
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/dnd", "Do Not Disturb: whether it is on and until when, the snoozes, what was held while they were on"),
    ("POST", "/v1/dnd/on", "turns Do Not Disturb on: {until?: ms or \"morning\" (07:00 on the TV's clock), minutes?}; neither: until it is turned off"),
    ("POST", "/v1/dnd/off", "turns Do Not Disturb off"),
    ("POST", "/v1/dnd/snooze", "snoozes one source's alerts or a whole kind's: {kind, source?, label?, until? or minutes?}"),
    ("DELETE", "/v1/dnd/snooze/{id}", "ends a snooze"),
    ("POST", "/v1/dnd/settings", "{letDoorbell}: whether a doorbell comes through Do Not Disturb"),
    ("POST", "/v1/dnd/missed", "records an alert held outside the helper: {kind, source?, title, body?}"),
    ("DELETE", "/v1/dnd/missed", "forgets what was held"),
];

pub const TOPIC: &str = "dnd";
/// The kind of a camera's alert; a plugin's is `plugin:<id>`.
pub const CAMERA: &str = "camera";
const FILE: &str = "dnd.json";
/// What was held is kept, newest first, up to this many.
const MAX_MISSED: usize = 20;
const MAX_SNOOZES: usize = 32;
/// "Until tomorrow morning": this hour on the TV's clock.
const MORNING_HOUR: i64 = 7;
/// No Do Not Disturb or snooze is timed further ahead than this.
const LONGEST_MS: u64 = 7 * 24 * 3_600_000;
const TEXT_MAX: usize = 120;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snooze {
    pub id: String,
    pub kind: String,
    /// One camera or channel; none: every alert of the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub label: String,
    pub until: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Missed {
    pub kind: String,
    #[serde(default)]
    pub source: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub at: u64,
    /// `dnd` or `snooze`.
    pub by: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    on: bool,
    /// When it turns itself off; none: when it is turned off.
    until: Option<u64>,
    let_doorbell: bool,
    snoozes: Vec<Snooze>,
    missed: Vec<Missed>,
    serial: u64,
}

impl Default for State {
    fn default() -> Self {
        Self { on: false, until: None, let_doorbell: true, snoozes: Vec::new(), missed: Vec::new(), serial: 0 }
    }
}

/// An alert at the gate.
#[derive(Clone, Copy, Debug)]
pub struct Alert<'a> {
    pub kind: &'a str,
    pub source: &'a str,
    pub doorbell: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Pass,
    Dnd,
    Snoozed,
}

impl State {
    /// Ends what has run out; whether anything did.
    fn expire(&mut self, now: u64) -> bool {
        let before = (self.on, self.snoozes.len());
        if self.until.is_some_and(|u| u <= now) {
            self.on = false;
            self.until = None;
        }
        self.snoozes.retain(|s| s.until > now);
        before != (self.on, self.snoozes.len())
    }

    fn gate(&self, a: Alert) -> Gate {
        let snoozed = |whole: bool| self.snoozes.iter().any(|s| s.kind == a.kind && if whole { s.source.is_none() } else { s.source.as_deref() == Some(a.source) });
        // a snooze of the doorbell's own camera is the owner silencing that doorbell
        if snoozed(false) {
            Gate::Snoozed
        } else if a.doorbell && self.let_doorbell {
            Gate::Pass
        } else if self.on {
            Gate::Dnd
        } else if snoozed(true) {
            Gate::Snoozed
        } else {
            Gate::Pass
        }
    }

    fn next_expiry(&self) -> Option<u64> {
        self.snoozes.iter().map(|s| s.until).chain(self.until.filter(|_| self.on)).min()
    }

    fn json(&self) -> Value {
        json!({"on": self.on, "until": self.until, "letDoorbell": self.let_doorbell, "snoozes": self.snoozes, "missed": self.missed})
    }

    fn miss(&mut self, m: Missed) {
        self.missed.insert(0, m);
        self.missed.truncate(MAX_MISSED);
    }
}

#[derive(Default)]
pub struct Dnd {
    state: Mutex<State>,
    /// Where the state is kept; none in a test.
    path: Mutex<Option<PathBuf>>,
    /// Wakes the timer when the next expiry moves.
    changed: Condvar,
}

static DND: LazyLock<Dnd> = LazyLock::new(Dnd::default);

fn ok(mut body: Value, now: u64) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now));
    }
    Response::json(200, &body)
}

fn bad(message: &str) -> Response {
    error_response(400, "bad_request", message)
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").trim().chars().take(TEXT_MAX).collect()
}

/// Seconds east of UTC on the TV's clock at `unix_s` (musl reads `TZ` or `/etc/localtime`); none when the clock cannot say.
fn utc_offset_s(unix_s: i64) -> Option<i64> {
    let t = libc::time_t::try_from(unix_s).ok()?;
    // SAFETY: `tm` is ours to fill; localtime_r writes through the pointer it is given and keeps nothing.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(tm.tm_gmtoff)
}

/// The first `MORNING_HOUR`:00 on the wall clock after `now`, in a zone `offset_s` east of UTC: today's when it is still night, else tomorrow's.
pub fn next_morning(now: u64, offset_s: i64) -> u64 {
    const DAY: i64 = 86_400_000;
    let local = now as i64 + offset_s * 1000;
    let mut at = local.div_euclid(DAY) * DAY + MORNING_HOUR * 3_600_000;
    if at <= local {
        at += DAY;
    }
    (at - offset_s * 1000) as u64
}

/// The morning on the TV's own clock (UTC's when the clock cannot say); a change of the clocks in the night moves it by that hour.
fn morning(now: u64) -> u64 {
    let offset = |ms: u64| utc_offset_s((ms / 1000) as i64).unwrap_or(0);
    next_morning(now, offset(next_morning(now, offset(now))))
}

/// `until` (ms or "morning") or `minutes` of a request: none for neither, an error for a time that is past or too far.
fn ends(body: &Value, now: u64, morning: impl Fn(u64) -> u64) -> Result<Option<u64>, Response> {
    let at = match (body.get("until"), body.get("minutes").and_then(Value::as_u64)) {
        (Some(Value::String(s)), _) if s == "morning" => morning(now),
        (Some(v), _) if !v.is_null() => v.as_u64().ok_or_else(|| bad("until is a time in ms since the epoch or \"morning\""))?,
        (_, Some(m)) => now.saturating_add(m.saturating_mul(60_000)),
        _ => return Ok(None),
    };
    if at <= now || at - now > LONGEST_MS {
        return Err(bad("the end is in the past or more than 7 days ahead"));
    }
    Ok(Some(at))
}

fn body_of(request: &Request) -> Result<Value, Response> {
    if request.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(bad("the body is a JSON object")),
        Err(e) => Err(bad(&format!("the body is not JSON: {e}"))),
    }
}

impl Dnd {
    /// The answer for one alert; a held one is recorded in `missed` unless it is a note (the notification list keeps those).
    fn admit_at(&self, hub: &Hub, alert: Alert, missed: Option<(String, String)>, now: u64) -> Gate {
        let mut st = self.state.lock();
        let expired = st.expire(now);
        let gate = st.gate(alert);
        if let (Gate::Dnd | Gate::Snoozed, Some((title, body))) = (gate, missed) {
            let by = if gate == Gate::Dnd { "dnd" } else { "snooze" };
            st.miss(Missed { kind: alert.kind.into(), source: alert.source.into(), title, body, at: now, by: by.into() });
        } else if !expired {
            return gate;
        }
        self.commit(hub, &st);
        gate
    }

    /// Keeps and publishes the state and tells the timer; called with the state locked.
    fn commit(&self, hub: &Hub, st: &State) {
        if let Some(path) = self.path.lock().clone() {
            let tmp = path.with_extension("json.tmp");
            let text = serde_json::to_vec_pretty(st).unwrap_or_default();
            if std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        hub.set(TOPIC, st.json());
        self.changed.notify_all();
    }

    fn handle_at(&self, hub: &Hub, request: &Request, rest: &[&str], now: u64, morning: impl Fn(u64) -> u64) -> Response {
        let body = match request.method.as_str() {
            "POST" => match body_of(request) {
                Ok(b) => b,
                Err(r) => return r,
            },
            _ => json!({}),
        };
        let mut st = self.state.lock();
        st.expire(now);
        match (request.method.as_str(), rest) {
            ("GET", []) => return ok(st.json(), now),
            ("POST", ["on"]) => {
                let until = match ends(&body, now, morning) {
                    Ok(u) => u,
                    Err(r) => return r,
                };
                if !st.on {
                    st.missed.clear();
                }
                st.on = true;
                st.until = until;
            }
            ("POST", ["off"]) => {
                st.on = false;
                st.until = None;
            }
            ("POST", ["snooze"]) => {
                let kind = text(body.get("kind"));
                if kind.is_empty() {
                    return bad("send {kind, source?, label?, until? or minutes?}");
                }
                let until = match ends(&body, now, morning) {
                    Ok(Some(u)) => u,
                    Ok(None) => return bad("a snooze ends: send until or minutes"),
                    Err(r) => return r,
                };
                let source = Some(text(body.get("source"))).filter(|s| !s.is_empty());
                st.snoozes.retain(|s| !(s.kind == kind && s.source == source));
                if st.snoozes.len() >= MAX_SNOOZES {
                    return error_response(400, "too_many", &format!("at most {MAX_SNOOZES} snoozes at once"));
                }
                st.serial += 1;
                let id = format!("s{}", st.serial);
                let label = text(body.get("label"));
                st.snoozes.push(Snooze { id, kind, source, label, until });
            }
            ("DELETE", ["snooze", id]) => {
                let before = st.snoozes.len();
                st.snoozes.retain(|s| s.id != *id);
                if st.snoozes.len() == before {
                    return error_response(404, "no_such_snooze", "no snooze has that id");
                }
            }
            ("POST", ["settings"]) => {
                let Some(let_doorbell) = body.get("letDoorbell").and_then(Value::as_bool) else {
                    return bad("send {letDoorbell}");
                };
                st.let_doorbell = let_doorbell;
            }
            ("POST", ["missed"]) => {
                let (kind, title) = (text(body.get("kind")), text(body.get("title")));
                if kind.is_empty() || title.is_empty() {
                    return bad("send {kind, source?, title, body?}");
                }
                st.miss(Missed { kind, source: text(body.get("source")), title, body: text(body.get("body")), at: now, by: "dnd".into() });
            }
            ("DELETE", ["missed"]) => st.missed.clear(),
            _ => return error_response(404, "not_found", "no such Do Not Disturb request"),
        }
        self.commit(hub, &st);
        ok(st.json(), now)
    }

    /// Sleeps until the next expiry (or a change) and publishes what ran out. Nothing is polled.
    fn run(&self, hub: &Hub) {
        let mut st = self.state.lock();
        loop {
            let now = now_ms();
            if st.expire(now) {
                self.commit(hub, &st);
            }
            match st.next_expiry() {
                Some(at) => {
                    self.changed.wait_for(&mut st, Duration::from_millis(at.saturating_sub(now).max(1)));
                }
                None => self.changed.wait(&mut st),
            }
        }
    }
}

/// Serves `/v1/dnd/...`.
pub fn handle(hub: &Hub, request: &Request, rest: &[&str]) -> Response {
    DND.handle_at(hub, request, rest, now_ms(), morning)
}

/// Reads the kept state, publishes the topic and starts the timer. Called once, with the helper's context.
pub fn open(ctx: &Ctx) {
    let path = ctx.paths.data_dir.join(FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        *DND.state.lock() = serde_json::from_str(&text).unwrap_or_default();
    }
    *DND.path.lock() = Some(path);
    {
        let mut st = DND.state.lock();
        st.expire(now_ms());
        DND.commit(&ctx.hub, &st);
    }
    let hub: Arc<Hub> = ctx.hub.clone();
    if let Err(e) = std::thread::Builder::new().name("dnd-timer".to_owned()).stack_size(64 * 1024).spawn(move || DND.run(&hub)) {
        crate::log_info!("dnd: the timer did not start ({e}); expiry is applied at the gate only");
    }
}

/// The gate for a camera's pop-up: anything but `Pass` keeps it off the screen, recorded as missed.
pub fn admit_camera(hub: &Hub, camera: &str, doorbell: bool, label: &str, name: &str) -> Gate {
    let mut chars = label.chars();
    let what: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    DND.admit_at(hub, Alert { kind: CAMERA, source: camera, doorbell }, Some((format!("{what} at {name}"), String::new())), now_ms())
}

/// The gate for a plugin's note (`event` is the note as the notification topic has it): whether it is held.
pub fn holds_note(hub: &Hub, plugin: &str, event: &Value, now: u64) -> bool {
    holds_note_at(&DND, hub, plugin, event, now)
}

fn holds_note_at(dnd: &Dnd, hub: &Hub, plugin: &str, event: &Value, now: u64) -> bool {
    let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_owned();
    let channel = s(event.get("channel"));
    let doorbell = lumatv_saver::rules::card_class(&s(event.pointer("/post/kind")), &channel, &s(event.get("channelIcon"))) == "doorbell";
    let kind = format!("plugin:{plugin}");
    dnd.admit_at(hub, Alert { kind: &kind, source: &channel, doorbell }, None, now) != Gate::Pass
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const MIN: u64 = 60_000;

    fn request(method: &str, body: &str) -> Request {
        Request { method: method.into(), path: String::new(), query: Vec::new(), headers: Vec::new(), body: body.as_bytes().to_vec(), peer: std::net::IpAddr::from([127, 0, 0, 1]) }
    }

    fn call(d: &Dnd, hub: &Hub, method: &str, rest: &[&str], body: &str, now: u64) -> (u16, Value) {
        let r = d.handle_at(hub, &request(method, body), rest, now, |n| n + 600 * MIN);
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    fn camera(d: &Dnd, hub: &Hub, id: &str, doorbell: bool, now: u64) -> Gate {
        d.admit_at(hub, Alert { kind: CAMERA, source: id, doorbell }, Some(("Person at Driveway".into(), String::new())), now)
    }

    fn ha_note(d: &Dnd, hub: &Hub, now: u64) -> bool {
        holds_note_at(d, hub, "homeassistant", &json!({"channel": "notify", "channelIcon": "bell", "post": {"kind": "notice", "title": "Washer done"}}), now)
    }

    #[test]
    fn dnd_holds_a_camera_pop_up_and_a_home_assistant_note_and_keeps_the_pop_up_as_missed() {
        let (d, hub) = (Dnd::default(), Hub::new(1));
        assert_eq!(camera(&d, &hub, "cam1", false, NOW), Gate::Pass);
        assert!(!ha_note(&d, &hub, NOW));
        call(&d, &hub, "POST", &["on"], "", NOW);
        assert_eq!(camera(&d, &hub, "cam1", false, NOW), Gate::Dnd);
        assert!(ha_note(&d, &hub, NOW));
        let topic = hub.topic_at(TOPIC).1.unwrap();
        assert_eq!((topic["on"].as_bool(), topic["missed"].as_array().map(Vec::len)), (Some(true), Some(1)), "{topic}");
        assert_eq!((topic["missed"][0]["title"].as_str(), topic["missed"][0]["by"].as_str()), (Some("Person at Driveway"), Some("dnd")));
        call(&d, &hub, "POST", &["off"], "", NOW);
        assert_eq!(camera(&d, &hub, "cam1", false, NOW), Gate::Pass);
        // turned on anew, it starts with nothing missed
        let (_, on) = call(&d, &hub, "POST", &["on"], "", NOW);
        assert_eq!(on["missed"], json!([]));
    }

    #[test]
    fn a_doorbell_comes_through_dnd_with_the_switch_on_and_is_held_with_it_off() {
        let (d, hub) = (Dnd::default(), Hub::new(1));
        call(&d, &hub, "POST", &["on"], "", NOW);
        assert_eq!(camera(&d, &hub, "door", true, NOW), Gate::Pass);
        let doorbell_note = json!({"channel": "door", "channelIcon": "doorbell", "post": {"kind": "doorbell"}});
        assert!(!holds_note_at(&d, &hub, "homeassistant", &doorbell_note, NOW));
        let (status, s) = call(&d, &hub, "POST", &["settings"], r#"{"letDoorbell": false}"#, NOW);
        assert_eq!((status, s["letDoorbell"].as_bool()), (200, Some(false)));
        assert_eq!(camera(&d, &hub, "door", true, NOW), Gate::Dnd);
        assert!(holds_note_at(&d, &hub, "homeassistant", &doorbell_note, NOW));
    }

    #[test]
    fn a_snooze_holds_its_camera_or_every_camera_and_a_doorbell_only_when_its_own_camera_is_snoozed() {
        let (d, hub) = (Dnd::default(), Hub::new(1));
        let (status, one) = call(&d, &hub, "POST", &["snooze"], r#"{"kind": "camera", "source": "cam1", "label": "Driveway", "minutes": 60}"#, NOW);
        assert_eq!((status, one["snoozes"][0]["until"].as_u64()), (200, Some(NOW + 60 * MIN)), "{one}");
        assert_eq!(camera(&d, &hub, "cam1", false, NOW), Gate::Snoozed);
        assert_eq!(camera(&d, &hub, "cam2", false, NOW), Gate::Pass);
        assert_eq!(camera(&d, &hub, "cam1", true, NOW), Gate::Snoozed);
        call(&d, &hub, "POST", &["snooze"], r#"{"kind": "camera", "label": "All camera alerts", "minutes": 15}"#, NOW);
        assert_eq!(camera(&d, &hub, "cam2", false, NOW), Gate::Snoozed);
        assert_eq!(camera(&d, &hub, "cam2", true, NOW), Gate::Pass);
        assert!(!ha_note(&d, &hub, NOW), "a camera snooze leaves other kinds alone");
        // the same scope again replaces its snooze
        let (_, again) = call(&d, &hub, "POST", &["snooze"], r#"{"kind": "camera", "source": "cam1", "minutes": 240}"#, NOW);
        assert_eq!(again["snoozes"].as_array().map(Vec::len), Some(2));
        let id = again["snoozes"][1]["id"].as_str().unwrap().to_owned();
        let (status, after) = call(&d, &hub, "DELETE", &["snooze", id.as_str()], "", NOW);
        assert_eq!((status, after["snoozes"].as_array().map(Vec::len)), (200, Some(1)));
        assert_eq!(call(&d, &hub, "DELETE", &["snooze", id.as_str()], "", NOW).0, 404);
        assert_eq!(call(&d, &hub, "POST", &["snooze"], r#"{"kind": "camera"}"#, NOW).0, 400, "a snooze has an end");
    }

    #[test]
    fn dnd_and_snoozes_end_at_their_time_and_the_timer_aims_at_the_next_end() {
        let (d, hub) = (Dnd::default(), Hub::new(1));
        call(&d, &hub, "POST", &["on"], r#"{"minutes": 60}"#, NOW);
        call(&d, &hub, "POST", &["snooze"], r#"{"kind": "camera", "source": "cam1", "minutes": 15}"#, NOW);
        assert_eq!(d.state.lock().next_expiry(), Some(NOW + 15 * MIN));
        assert_eq!(camera(&d, &hub, "cam1", false, NOW + 15 * MIN - 1), Gate::Snoozed);
        assert_eq!(camera(&d, &hub, "cam1", false, NOW + 15 * MIN), Gate::Dnd);
        assert_eq!(d.state.lock().next_expiry(), Some(NOW + 60 * MIN));
        assert_eq!(camera(&d, &hub, "cam1", false, NOW + 60 * MIN), Gate::Pass);
        let topic = hub.topic_at(TOPIC).1.unwrap();
        assert_eq!((topic["on"].as_bool(), topic["until"].clone(), topic["snoozes"].clone()), (Some(false), Value::Null, json!([])));
        assert_eq!(d.state.lock().next_expiry(), None);
        // "morning" is the TV's next 07:00 and a time already past is refused
        let (_, m) = call(&d, &hub, "POST", &["on"], r#"{"until": "morning"}"#, NOW);
        assert_eq!(m["until"].as_u64(), Some(NOW + 600 * MIN));
        assert_eq!(call(&d, &hub, "POST", &["on"], &format!(r#"{{"until": {}}}"#, NOW - 1), NOW).0, 400);
    }

    #[test]
    fn the_state_is_kept_in_a_file_and_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let (d, hub) = (Dnd::default(), Hub::new(1));
        *d.path.lock() = Some(path.clone());
        call(&d, &hub, "POST", &["on"], r#"{"minutes": 120}"#, NOW);
        call(&d, &hub, "POST", &["snooze"], r#"{"kind": "camera", "source": "cam1", "label": "Driveway", "minutes": 60}"#, NOW);
        call(&d, &hub, "POST", &["settings"], r#"{"letDoorbell": false}"#, NOW);
        camera(&d, &hub, "cam2", false, NOW);
        let read: State = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read, *d.state.lock());
        assert_eq!((read.until, read.snoozes[0].label.as_str(), read.let_doorbell, read.missed.len()), (Some(NOW + 120 * MIN), "Driveway", false, 1));
        assert!(!dir.path().join("dnd.json.tmp").exists());
        assert_eq!(serde_json::from_str::<State>("{}").unwrap(), State::default(), "an old or empty file lets the doorbell through");
    }

    #[test]
    fn tomorrow_morning_is_the_next_seven_o_clock_on_the_tvs_clock_across_midnight() {
        const H: u64 = 3_600_000;
        let day = 1_799_971_200_000; // a midnight UTC
        assert_eq!(next_morning(day + 23 * H + 30 * MIN, 0), day + 24 * H + 7 * H);
        assert_eq!(next_morning(day + 24 * H + 30 * MIN, 0), day + 24 * H + 7 * H, "after midnight it is the same night's morning");
        assert_eq!(next_morning(day + 7 * H, 0), day + 24 * H + 7 * H, "at seven it is tomorrow's");
        // UTC-5: 23:30 local is 04:30 UTC the next day; the morning is 12:00 UTC that day
        assert_eq!(next_morning(day + 24 * H + 4 * H + 30 * MIN, -5 * 3600), day + 24 * H + 12 * H);
        // UTC+10: 01:00 local is 15:00 UTC the day before; the morning is 21:00 UTC
        assert_eq!(next_morning(day + 15 * H, 10 * 3600), day + 21 * H);
    }

    #[test]
    fn a_toast_held_by_the_shell_is_recorded_and_the_list_keeps_twenty() {
        let (d, hub) = (Dnd::default(), Hub::new(1));
        for i in 0..25 {
            call(&d, &hub, "POST", &["missed"], &format!(r#"{{"kind": "toast", "source": "ha", "title": "t{i}"}}"#), NOW + i);
        }
        let (_, s) = call(&d, &hub, "GET", &[], "", NOW + 30);
        assert_eq!((s["missed"].as_array().map(Vec::len), s["missed"][0]["title"].as_str()), (Some(20), Some("t24")));
        assert_eq!(call(&d, &hub, "POST", &["missed"], r#"{"kind": "toast"}"#, NOW).0, 400);
        let (_, cleared) = call(&d, &hub, "DELETE", &["missed"], "", NOW);
        assert_eq!(cleared["missed"], json!([]));
    }
}
