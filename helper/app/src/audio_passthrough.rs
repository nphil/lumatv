//! Opt-in audio passthrough: Dolby TrueHD from a player that decodes for itself (Marquee) to the receiver over eARC.
//!
//! The route is the `audiopass` crate. This module is the plugin around it: the switch and its page, LG's sound output, the monitor
//! thread that carries a session through connecting and watches it, and the three calls a player makes
//! (`docs/plugins/audio-passthrough.md`):
//!
//! | Request | Answer |
//! |---|---|
//! | `POST /v1/audio-passthrough/arm` | `{pid, tail, adec}`: `{armed, converter, reason, retry}` |
//! | `GET /v1/audio-passthrough/route` | `{armed, phase, bypassing, mixed, converterPresent, connected, audible, reason, ageMs, sampledUptimeMs}` |
//! | `POST /v1/audio-passthrough/release` | `{pid}`: `{released}`; the TV is put back before the answer |
//!
//! All three are for a player on the TV itself (loopback). The topic `audio.passthrough` carries the same state, plus readiness, the
//! firmware verdict and how the last session ended; see [`topic`].
//!
//! Nothing is read from or written to the TV's audio stack while the plugin is off, except putting back what an earlier run of this
//! plugin changed (its journal).

use crate::API_VERSION;
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::luna::Follow;
use crate::plugins::local::{Local, OptIn, Spec};
use crate::plugins::session::Shared;
use crate::timeutil::now_ms;
use audiopass::host::{Host, System};
use audiopass::route::{Env, MAX_ADEC, Phase, Route, Snapshot, State, Why};
use parking_lot::{Condvar, Mutex, RwLock};
use serde_json::{Value, json};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const ID: &str = "audio-passthrough";
pub const TOPIC: &str = "audio.passthrough";

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "POST",
        "/v1/audio-passthrough/arm",
        "a player that holds LG's renderer asks for the TrueHD passthrough route: {pid, tail, adec} (adec is the decoder input LG granted that renderer); loopback only",
    ),
    (
        "GET",
        "/v1/audio-passthrough/route",
        "the route as the player must watch it while it streams: {armed, phase, bypassing, mixed, converterPresent, connected, audible, reason, ageMs, sampledUptimeMs}",
    ),
    (
        "POST",
        "/v1/audio-passthrough/release",
        "the player's session is over: {pid}; the TV is put back before this answers",
    ),
];

pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Audio passthrough",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Lets an app that decodes Dolby TrueHD itself (Marquee) pass it unchanged to the receiver over eARC. While the app plays, the helper changes the TV's audio route: it stops LG mixing system sounds into the stream and connects the TV's HDMI audio output. It puts everything back when playback stops, the app exits, the TV goes to standby, this is switched off or the helper stops. Works only on the TV software it was tested on (LG C4, 33.22.52); anything else is refused with the reason. Needs the receiver on eARC. Never edits LG's files.",
};

const URI: &str = "luna://com.webos.settingsservice/getSystemSettings";
const PAYLOAD: &str =
    r#"{"category":"sound","keys":["soundOutput","soundOutputDigital"],"subscribe":true}"#;
/// Before the sound output subscription starts again after it ended.
const FOLLOW_RETRY: Duration = Duration::from_secs(5);
/// Before the page is registered again after the base refused it.
const PAGE_RETRY: Duration = Duration::from_secs(30);
/// How long a reading of the route can be trusted by a player (`validForMs`).
const VALID_FOR_MS: u64 = 300;
/// How long the monitor sleeps while the plugin is off; switching it on wakes it.
const OFF_WAIT_MS: u64 = 30_000;

/// The running module, for the router. Replaced by each start.
static SERVICE: RwLock<Option<Arc<Service>>> = RwLock::new(None);

/// LG's sound output as the settings service last said it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sound {
    output: Option<String>,
    digital: Option<String>,
}

impl Sound {
    /// Takes in one reply of the subscription: the first carries both settings, later ones what changed.
    fn accept(&mut self, reply: &Value) {
        if reply.get("returnValue").and_then(Value::as_bool) != Some(true) {
            return;
        }
        let text = |key: &str| {
            reply
                .pointer(&format!("/settings/{key}"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if let Some(output) = text("soundOutput") {
            self.output = Some(output);
        }
        if let Some(digital) = text("soundOutputDigital") {
            self.digital = Some(digital);
        }
    }
}

/// Wakes the monitor before its wait is over.
#[derive(Default)]
struct Wake {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl Wake {
    fn notify(&self) {
        *self.pending.lock() = true;
        self.changed.notify_all();
    }

    /// Sleeps up to `ms`, or less when woken (including a wake that came while the monitor was busy).
    fn wait(&self, ms: u64) {
        let mut pending = self.pending.lock();
        if !*pending {
            self.changed
                .wait_for(&mut pending, Duration::from_millis(ms));
        }
        *pending = false;
    }
}

struct Service {
    hub: Arc<Hub>,
    host: Arc<dyn Host>,
    stop: Stop,
    enabled: Box<dyn Fn() -> bool + Send + Sync>,
    route: Mutex<Route>,
    sound: Mutex<Sound>,
    /// The latest snapshot of the route. A player polls it every few tens of milliseconds, so answering from it never waits for the
    /// route, which an arm or a restore holds for seconds.
    view: Arc<Mutex<Option<Snapshot>>>,
    wake: Wake,
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

fn bad(message: &str) -> Response {
    error_response(400, "bad_request", message)
}

/// The topic's session object.
fn session_json(s: &audiopass::route::SessionView) -> Value {
    json!({
        "pid": s.pid,
        "adec": s.adec,
        "tail": s.tail,
        "converter": s.converter,
        "phase": s.phase.name(),
        "bypassing": s.bypassing,
        "mixed": s.mixed,
        "converterPresent": s.converter_present,
        "connected": s.connected,
        "audible": s.audible,
        "why": s.why,
    })
}

/// The topic `audio.passthrough`: `{schema, enabled, state, ready, reason, firmware, output, digital, earc, words, codec, carrier,
/// session, end, pending, after, sampledUptimeMs, validForMs}`.
///
/// `state` is `off`, `refused` (not the tested firmware), `not-ready` (with the `reason`), `ready`, `armed` (a session; see
/// `session.phase`) or `lost` (the last session ended because LG changed the route; LG's own mix is playing). `ready` says whether an
/// arm would be accepted now. `carrier` is what the player sends: IEC 61937 bursts (`burstBytes` every `burstMs`) as 16-bit PCM,
/// 8 channels at 192 kHz, the 16-bit words in `byteOrder` (the proven bursts start `f8 72 4e 1f`).
/// `after` is a sentence when the TV, read once the last session was put back, differs from what that session found or LG's HDMI
/// input service did not answer; null when it matches.
fn topic(s: &Snapshot) -> Value {
    json!({
        "schema": 1,
        "enabled": s.state != State::Off,
        "state": s.state.name(),
        "ready": s.ready,
        "reason": s.reason,
        "firmware": s.firmware.as_ref().map(|f| json!({
            "match": f.verdict.is_ok(),
            "release": f.release,
            "kernel": f.kernel,
            "detail": f.verdict.as_ref().err(),
        })),
        "output": s.output,
        "digital": s.digital,
        "earc": s.earc,
        "words": s.words,
        "codec": s.codec,
        "carrier": {
            "format": "S16LE",
            "byteOrder": "be",
            "rate": 192_000,
            "channels": 8,
            "burstBytes": 61_440,
            "burstMs": 20,
        },
        "session": s.session.as_ref().map(session_json),
        "end": s.end.as_ref().map(|e| json!({"why": e.why.name(), "detail": e.detail})),
        "pending": s.pending,
        "after": s.after,
        "sampledUptimeMs": s.at,
        "validForMs": VALID_FOR_MS,
    })
}

/// Keeps the snapshot for `GET route` and publishes the topic. A topic that differs only in its sample time raises no event.
fn publish(hub: &Hub, view: &Mutex<Option<Snapshot>>, snapshot: &Snapshot) {
    hub.set_ignoring(TOPIC, topic(snapshot), &["sampledUptimeMs"]);
    *view.lock() = Some(snapshot.clone());
}

fn pid_of(body: &Value) -> Result<u32, Response> {
    body.get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0)
        .ok_or_else(|| bad("pid is required: the player's process id"))
}

fn tail_of(body: &Value) -> Result<&str, Response> {
    body.get("tail")
        .and_then(Value::as_str)
        .filter(|tail| {
            tail.len() == 8 && tail.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
        .ok_or_else(|| {
            bad("tail is required: the 8 lowercase hex digits after lipsync_ in LG's audio graph")
        })
}

/// The decoder input LG's resource manager granted the player's renderer (the sink's `index`).
fn adec_of(body: &Value) -> Result<u8, Response> {
    body.get("adec")
        .and_then(Value::as_u64)
        .and_then(|adec| u8::try_from(adec).ok())
        .filter(|adec| *adec <= MAX_ADEC)
        .ok_or_else(|| {
            bad(&format!(
                "adec is required: the decoder input LG granted the player's renderer, 0 to {MAX_ADEC}"
            ))
        })
}

fn body_of(request: &Request) -> Result<Value, Response> {
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(body) if body.is_object() => Ok(body),
        Ok(_) => Err(bad("the body must be a JSON object")),
        Err(e) => Err(bad(&format!("the body is not JSON: {e}"))),
    }
}

impl Service {
    fn new(
        hub: Arc<Hub>,
        host: Arc<dyn Host>,
        stop: Stop,
        journal: String,
        enabled: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        let view = Arc::new(Mutex::new(None));
        let mut route = Route::new(host.clone(), journal);
        {
            let (hub, view) = (hub.clone(), view.clone());
            route.observe_with(move |snapshot| publish(&hub, &view, snapshot));
        }
        Self {
            hub,
            host,
            stop,
            enabled,
            route: Mutex::new(route),
            sound: Mutex::new(Sound::default()),
            view,
            wake: Wake::default(),
        }
    }

    fn env(&self, stopping: bool) -> Env {
        let sound = self.sound.lock().clone();
        Env {
            enabled: (self.enabled)(),
            screen_on: crate::screen::is_on(&self.hub),
            stopping,
            sound_output: sound.output,
            digital: sound.digital,
        }
    }

    /// One turn of the monitor: ends and restores what must end, carries a session through connecting, watches it, and keeps readiness
    /// fresh. Returns how long to wait before the next turn, and whether the helper is stopping (this was the last).
    fn step(&self) -> (u64, bool) {
        let stopping = self.stop.is_set();
        let env = self.env(stopping);
        let wait = self.route.lock().tick(&env);
        (if env.enabled { wait } else { OFF_WAIT_MS }, stopping)
    }

    fn handle(&self, request: &Request, op: &str) -> Response {
        if !request.peer.is_loopback() {
            return error_response(
                403,
                "local_only",
                "the passthrough route is for a player on the TV itself",
            );
        }
        match (request.method.as_str(), op) {
            ("POST", "arm") => self.arm(request),
            ("GET", "route") => self.route(),
            ("POST", "release") => self.release(request),
            (_, "arm" | "route" | "release") => {
                error_response(405, "method_not_allowed", "that method is not served here")
                    .with_header("Allow", if op == "route" { "GET" } else { "POST" })
            }
            _ => error_response(404, "not_found", "no such passthrough call"),
        }
    }

    /// 200 with `armed: false` and the reason when the route refuses: the player decodes in software then. 400 is for a request that
    /// is not one.
    fn arm(&self, request: &Request) -> Response {
        let body = match body_of(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        let (pid, tail, adec) = match (pid_of(&body), tail_of(&body), adec_of(&body)) {
            (Ok(pid), Ok(tail), Ok(adec)) => (pid, tail, adec),
            (Err(response), ..) | (_, Err(response), _) | (.., Err(response)) => return response,
        };
        let env = self.env(self.stop.is_set());
        let armed = self.route.lock().arm(pid, tail, adec, &env);
        // The monitor sleeps long while idle; the session's own steps start now.
        self.wake.notify();
        match armed {
            Ok(converter) => ok(json!({
                "armed": true, "converter": converter, "reason": null, "retry": false,
            })),
            Err(refusal) => ok(json!({
                "armed": false, "converter": null, "reason": refusal.reason, "retry": refusal.retry,
            })),
        }
    }

    fn release(&self, request: &Request) -> Response {
        let pid = match body_of(request).and_then(|body| pid_of(&body)) {
            Ok(pid) => pid,
            Err(response) => return response,
        };
        let released = self.route.lock().release(pid);
        self.wake.notify();
        match released {
            Ok(released) => ok(json!({ "released": released })),
            Err(why) => error_response(409, "not_the_holder", &why),
        }
    }

    /// What a player watches while it streams: stop sending the moment `bypassing` is false or `ageMs` passes `validForMs`.
    fn route(&self) -> Response {
        let Some(s) = self.view.lock().clone() else {
            return error_response(
                503,
                "audio_passthrough_unavailable",
                "the route has not been read yet",
            );
        };
        let session = s.session.as_ref();
        ok(json!({
            "armed": session.is_some_and(|v| v.phase != Phase::Ending),
            "phase": session.map(|v| v.phase.name()),
            "bypassing": session.is_some_and(|v| v.bypassing),
            "mixed": session.is_some_and(|v| v.mixed),
            "converterPresent": session.is_some_and(|v| v.converter_present),
            "connected": session.is_some_and(|v| v.connected),
            "audible": session.is_some_and(|v| v.audible),
            "reason": s.reason,
            "ageMs": self.host.now_ms().saturating_sub(s.at),
            "sampledUptimeMs": s.at,
        }))
    }
}

/// Serves `/v1/audio-passthrough/<op>`.
pub fn handle(request: &Request, op: &str) -> Response {
    let service = SERVICE.read().clone();
    match service {
        Some(service) => service.handle(request, op),
        None => error_response(
            503,
            "audio_passthrough_unavailable",
            "the audio passthrough module has not started",
        ),
    }
}

fn register() -> Value {
    let field = |key: &str, label: &str, help: &str| {
        json!({"key": key, "label": label, "help": help,
            "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}, "apply": "on-commit"})
    };
    json!({"title": OPT_IN.spec.name, "settings": {"format": 1, "owner": ID, "revision": 1, "title": OPT_IN.spec.name,
        "sections": [{"id": "route", "title": "Passthrough", "fields": [
            field("status", "Status", "Whether a player can use it now, and what it is doing."),
            field("tv", "TV software", "Only the build it was tested on is accepted."),
            field("output", "Sound output", "The receiver on eARC is needed. LG's own setting is not changed."),
            field("session", "Playback", "The app using the route now, if any."),
            field("receiver", "Receiver shows", "What the receiver reports it plays, when the Denon receiver plugin is on."),
            field("ended", "Last session", "How the last session ended. LG's audio settings are put back every time."),
            field("after", "TV afterwards", "What the TV's audio reads like once a session has ended, compared with before it.")]}]}})
}

fn output_words(output: Option<&str>, earc: Option<bool>, digital: Option<&str>) -> String {
    let output = match output {
        Some("external_arc") => "Receiver (eARC)".to_owned(),
        Some("tv_speaker_external_arc") => "TV speakers and receiver".to_owned(),
        Some("tv_speaker") => "TV speakers".to_owned(),
        Some(other) => other.to_owned(),
        None => "Not read yet".to_owned(),
    };
    let earc = match earc {
        Some(true) => ". eARC is on",
        Some(false) => ". eARC is off",
        None => "",
    };
    match digital {
        Some(digital) => format!("{output}{earc}. Digital output: {digital}"),
        None => format!("{output}{earc}"),
    }
}

fn ended_words(s: &Snapshot) -> String {
    let Some(end) = &s.end else {
        return "None since the helper started".into();
    };
    let why = match end.why {
        Why::Released => "The app stopped playing",
        Why::OwnerExited => "The app exited",
        Why::RouteLost => "LG changed the audio route",
        Why::Disabled => "Switched off",
        Why::Standby => "The TV went to standby",
        Why::HelperExit => "The helper stopped",
        Why::Fault => "The helper failed",
        Why::Recovered => "Put back after the helper restarted",
    };
    format!("{why}: {}", end.detail)
}

fn after_words(s: &Snapshot) -> String {
    match (&s.after, &s.end) {
        (Some(note), _) => note.clone(),
        (None, Some(_)) => "As it was before the session".into(),
        (None, None) => "No session yet".into(),
    }
}

/// The page's rows, in words.
fn facts(s: &Snapshot, receiver_mode: Option<&str>) -> Value {
    let status = match s.state {
        State::Off => "Off".to_owned(),
        State::Refused => format!("Not available: {}", s.reason),
        State::NotReady => format!("Not ready: {}", s.reason),
        State::Ready => "Ready for a player that decodes TrueHD itself".to_owned(),
        State::Armed => s.reason.clone(),
        State::Lost => format!("The route was lost: {}. LG's own mix is playing", s.reason),
    };
    let tv = match &s.firmware {
        None => "Not checked yet".to_owned(),
        Some(f) => {
            let release = f.release.as_deref().unwrap_or("unknown");
            match &f.verdict {
                Ok(()) => format!("{release}: the tested build"),
                Err(why) => format!("{release}: {why}"),
            }
        }
    };
    let session = match &s.session {
        None => "No app is using it".to_owned(),
        Some(v) => format!("Process {} on ADEC{}: {}", v.pid, v.adec, v.phase.name()),
    };
    json!({
        "status": status,
        "tv": tv,
        "output": output_words(s.output.as_deref(), s.earc, s.digital.as_deref()),
        "session": session,
        "receiver": receiver_mode.unwrap_or("Not reported"),
        "ended": ended_words(s),
        "after": after_words(s),
    })
}

fn handler(local: &Local, message: plugin_api::wire::ToPlugin) {
    match message {
        plugin_api::wire::ToPlugin::SettingsApply { txn, values, .. } => {
            let _ = local.say(
                json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}}),
            );
        }
        plugin_api::wire::ToPlugin::ActionInvoke { op, .. } => {
            let _ = local.answer(op, Err("This page only reports status"));
        }
        _ => {}
    }
}

pub fn spawn(ctx: &Ctx, shared: Arc<Shared>) -> std::io::Result<JoinHandle<()>> {
    let host: Arc<dyn Host> = Arc::new(System {
        luna_send: ctx.config.luna.send.clone(),
        ..System::default()
    });
    let journal = ctx.paths.data_dir.join(ID).join("journal");
    let switched = shared.clone();
    let service = Arc::new(Service::new(
        ctx.hub.clone(),
        host,
        ctx.stop.clone(),
        journal.to_string_lossy().into_owned(),
        Box::new(move || switched.switched_on(ID)),
    ));
    // What an earlier run changed goes back before anything can arm again.
    let note = service.route.lock().recover();
    if let Some(note) = note {
        crate::log_info!("audio passthrough: {note}");
    }
    let first = service.route.lock().snapshot();
    publish(&service.hub, &service.view, &first);
    *SERVICE.write() = Some(service.clone());
    {
        let service = service.clone();
        shared.offer(&OPT_IN, Arc::new(move || service.wake.notify()));
    }
    {
        let service = service.clone();
        ctx.stop.on_stop(move || service.wake.notify());
    }
    let ctx = ctx.clone();
    let guard_shared = shared.clone();
    crate::plugins::guard::spawn(
        "audio-passthrough",
        256 * 1024,
        guard_shared,
        &[ID],
        ctx.stop.clone(),
        move |again| run(&service, &ctx, &shared, again),
    )
}

/// The page, LG's sound output subscription and the facts the page shows, around [`Service::step`].
struct Surroundings {
    page: Option<Arc<Local>>,
    page_retry: Option<Instant>,
    follow: Option<Follow>,
    follow_retry: Option<Instant>,
    facts: Value,
}

impl Surroundings {
    fn page(&mut self, service: &Service, shared: &Arc<Shared>, enabled: bool) {
        if !enabled {
            self.page = None;
            return;
        }
        let due = self.page_retry.is_none_or(|at| Instant::now() >= at);
        if self.page.is_none() && due {
            match Local::start(shared, &OPT_IN.spec, register(), Arc::new(handler)) {
                Ok(page) => {
                    self.page = Some(page);
                    self.facts = Value::Null;
                }
                Err(e) => {
                    crate::log_info!("audio passthrough: the page did not register: {e}");
                    self.page_retry = Some(Instant::now() + PAGE_RETRY);
                }
            }
        }
        let snapshot = service.view.lock().clone();
        let (Some(page), Some(snapshot)) = (&self.page, snapshot) else {
            return;
        };
        let mode = service.hub.get(crate::receiver::TOPIC).and_then(|r| {
            r.get("soundMode")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let next = facts(&snapshot, mode.as_deref());
        if next != self.facts {
            let _ = page.say(json!({"t": "facts.put", "facts": next}));
            self.facts = next;
        }
    }

    /// LG's sound output is followed only while the plugin is on.
    fn follow(&mut self, service: &Arc<Service>, luna: &str, enabled: bool) {
        if !enabled {
            if self.follow.take().is_some() {
                *service.sound.lock() = Sound::default();
            }
            return;
        }
        if self.follow.as_ref().is_some_and(Follow::alive) {
            return;
        }
        if self.follow.take().is_some() {
            // It ended: what it last said may be stale.
            *service.sound.lock() = Sound::default();
            self.follow_retry = Some(Instant::now() + FOLLOW_RETRY);
        }
        if self.follow_retry.is_some_and(|at| Instant::now() < at) {
            return;
        }
        let listener = service.clone();
        let started = Follow::start(
            "audio-passthrough-sound",
            luna,
            URI,
            PAYLOAD,
            Box::new(move |reply| {
                listener.sound.lock().accept(reply);
                listener.wake.notify();
            }),
        );
        match started {
            Ok(follow) => self.follow = Some(follow),
            Err(e) => {
                crate::log_info!("audio passthrough: LG's sound output cannot be followed: {e}");
                self.follow_retry = Some(Instant::now() + FOLLOW_RETRY);
            }
        }
    }
}

fn run(service: &Arc<Service>, ctx: &Ctx, shared: &Arc<Shared>, again: bool) {
    if again {
        // A turn failed half way: whatever it had changed goes back.
        service.route.lock().end(
            Why::Fault,
            "the audio passthrough thread failed and started again",
        );
    }
    let mut around = Surroundings {
        page: None,
        page_retry: None,
        follow: None,
        follow_retry: None,
        facts: Value::Null,
    };
    loop {
        let enabled = (service.enabled)();
        around.follow(service, &ctx.config.luna.send, enabled);
        let (wait, stopping) = service.step();
        around.page(service, shared, enabled);
        if stopping {
            break;
        }
        service.wake.wait(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audiopass::testing::{ADEC, CONVERTER, FakeHost, PID, TAIL};
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicBool, Ordering};

    const JOURNAL: &str = "/data/audio-passthrough/journal";

    struct Rig {
        service: Arc<Service>,
        host: Arc<FakeHost>,
        on: Arc<AtomicBool>,
    }

    impl Rig {
        fn new() -> Self {
            let hub = Arc::new(Hub::new(1));
            hub.set(crate::screen::TOPIC, json!({"on": true}));
            let host = Arc::new(FakeHost::c4());
            let on = Arc::new(AtomicBool::new(true));
            let flag = on.clone();
            let service = Arc::new(Service::new(
                hub,
                host.clone(),
                Stop::new(),
                JOURNAL.into(),
                Box::new(move || flag.load(Ordering::Relaxed)),
            ));
            *service.sound.lock() = Sound {
                output: Some("external_arc".into()),
                digital: Some("passThrough".into()),
            };
            Self { service, host, on }
        }

        /// The monitor for `ms` of the fake clock.
        fn run(&self, ms: u64) {
            let end = self.host.now() + ms;
            while self.host.now() < end {
                let (wait, _) = self.service.step();
                self.host.sleep_ms(wait);
            }
        }

        fn call(&self, method: &str, op: &str, body: &str) -> (u16, Value) {
            self.call_from([127, 0, 0, 1], method, op, body)
        }

        fn call_from(&self, peer: [u8; 4], method: &str, op: &str, body: &str) -> (u16, Value) {
            let request = Request {
                method: method.into(),
                path: format!("/v1/audio-passthrough/{op}"),
                query: Vec::new(),
                headers: Vec::new(),
                body: body.as_bytes().to_vec(),
                peer: IpAddr::from(peer),
            };
            let response = self.service.handle(&request, op);
            (
                response.status,
                serde_json::from_slice(&response.body).unwrap(),
            )
        }

        fn arm(&self) -> (u16, Value) {
            self.call(
                "POST",
                "arm",
                &format!(r#"{{"pid":{PID},"tail":"{TAIL}","adec":{ADEC}}}"#),
            )
        }

        fn topic(&self) -> Value {
            self.service.hub.get(TOPIC).expect("the topic")
        }
    }

    #[test]
    fn a_player_arms_polls_and_releases_and_the_topic_follows() {
        let rig = Rig::new();
        rig.run(2_100);
        let topic = rig.topic();
        assert_eq!(topic["state"], "ready");
        assert_eq!(topic["ready"], true);
        assert_eq!(topic["schema"], 1);
        assert_eq!(topic["carrier"]["burstBytes"], 61_440);
        assert_eq!(topic["firmware"]["match"], true);
        assert_eq!(topic["session"], Value::Null);

        let (status, body) = rig.arm();
        assert_eq!(status, 200);
        assert_eq!(body["armed"], true);
        assert_eq!(body["converter"], CONVERTER);

        // The monitor carries the session through connecting without any further call.
        rig.run(10_000);
        let (_, route) = rig.call("GET", "route", "");
        assert_eq!(route["armed"], true);
        assert_eq!(route["phase"], "audible");
        assert!(
            route["bypassing"] == true && route["connected"] == true && route["audible"] == true
        );
        assert!(route["ageMs"].as_u64().unwrap() < VALID_FOR_MS, "{route}");
        let session = &rig.topic()["session"];
        assert_eq!(session["pid"], PID);
        assert_eq!(session["tail"], TAIL);
        assert_eq!(session["adec"], ADEC);

        let (status, body) = rig.call("POST", "release", &format!(r#"{{"pid":{PID}}}"#));
        assert_eq!(status, 200);
        assert_eq!(body["released"], true);
        let topic = rig.topic();
        assert_eq!(topic["session"], Value::Null);
        assert_eq!(topic["end"]["why"], "released");
        assert_eq!(topic["after"], Value::Null);
        let log = rig.host.log();
        assert!(
            log.ends_with(&[
                "luna mute true".to_owned(),
                "luna disconnect".to_owned(),
                "words 2,1,0,0,1".to_owned(),
                "remove".to_owned(),
            ]),
            "{log:?}"
        );
        let (_, route) = rig.call("GET", "route", "");
        assert_eq!(route["armed"], false);
        assert_eq!(route["bypassing"], false);
    }

    #[test]
    fn calls_are_told_apart_by_what_is_wrong_with_them() {
        let rig = Rig::new();
        let body = format!(r#"{{"pid":{PID},"tail":"{TAIL}","adec":{ADEC}}}"#);
        assert_eq!(rig.call_from([192, 168, 1, 9], "POST", "arm", &body).0, 403);
        for (op, body) in [
            ("arm", "not json"),
            ("arm", "[]"),
            ("arm", r#"{"tail":"0424000e"}"#),
            ("arm", r#"{"pid":0,"tail":"0424000e"}"#),
            ("arm", r#"{"pid":4242,"tail":"0424000E"}"#),
            ("arm", r#"{"pid":4242,"tail":"24"}"#),
            ("arm", r#"{"pid":4242,"tail":"0424000e"}"#),
            ("arm", r#"{"pid":4242,"tail":"0424000e","adec":8}"#),
            ("arm", r#"{"pid":4242,"tail":"0424000e","adec":"1"}"#),
            ("arm", r#"{"pid":4242,"tail":"0424000e","adec":-1}"#),
            ("release", "{}"),
        ] {
            assert_eq!(rig.call("POST", op, body).0, 400, "{op} {body}");
        }
        assert_eq!(rig.call("GET", "arm", "").0, 405);
        assert_eq!(rig.call("POST", "route", "{}").0, 405);
        assert_eq!(rig.call("POST", "nothing", "{}").0, 404);
        assert_eq!(
            rig.call("GET", "route", "").0,
            503,
            "nothing has been read yet"
        );

        // A refusal is an answer, not an error.
        rig.on.store(false, Ordering::Relaxed);
        let (status, body) = rig.call("POST", "arm", &body);
        assert_eq!(status, 200);
        assert_eq!(body["armed"], false);
        assert_eq!(body["retry"], false);
        assert!(
            body["reason"].as_str().unwrap().contains("switched off"),
            "{body}"
        );
        rig.on.store(true, Ordering::Relaxed);
        assert_eq!(rig.arm().1["armed"], true);
        rig.host.with(|s| {
            s.procs
                .insert(7, (5, vec![audiopass::route::AREN.to_owned()]));
        });
        assert_eq!(rig.call("POST", "release", r#"{"pid":7}"#).0, 409);
    }

    #[test]
    fn switching_the_plugin_off_puts_everything_back_and_the_topic_says_off() {
        let rig = Rig::new();
        assert_eq!(rig.arm().1["armed"], true);
        rig.run(10_000);
        assert_eq!(rig.topic()["session"]["phase"], "audible");
        rig.on.store(false, Ordering::Relaxed);
        rig.run(300);
        let topic = rig.topic();
        assert_eq!(topic["state"], "off");
        assert_eq!(topic["enabled"], false);
        assert_eq!(topic["session"], Value::Null);
        assert_eq!(topic["end"]["why"], "disabled");
        let log = rig.host.log();
        let position = |entry: &str| log.iter().rposition(|e| e == entry).unwrap();
        assert!(
            position("luna mute true") < position("luna disconnect")
                && position("luna disconnect") < position("words 2,1,0,0,1"),
            "{log:?}"
        );
        // Off, nothing more is read from the audio stack.
        let reads = rig.host.with(|s| s.sndout_reads);
        rig.run(5_000);
        assert_eq!(rig.host.with(|s| s.sndout_reads), reads);
    }

    #[test]
    fn the_sound_output_subscription_is_read_as_it_comes() {
        let mut sound = Sound::default();
        let first = json!({"returnValue": true, "subscribed": true,
            "settings": {"soundOutput": "tv_speaker", "soundOutputDigital": "passThrough"}});
        sound.accept(&first);
        assert_eq!(
            (sound.output.as_deref(), sound.digital.as_deref()),
            (Some("tv_speaker"), Some("passThrough"))
        );
        sound.accept(&json!({"returnValue": true, "settings": {"soundOutput": "external_arc"}}));
        assert_eq!(
            (sound.output.as_deref(), sound.digital.as_deref()),
            (Some("external_arc"), Some("passThrough"))
        );
        let before = sound.clone();
        sound.accept(&json!({"returnValue": false, "settings": {"soundOutput": "tv_speaker"}}));
        sound.accept(&json!({"returnValue": true}));
        assert_eq!(sound, before);
    }

    #[test]
    fn the_page_says_it_in_words() {
        let rig = Rig::new();
        rig.run(2_100);
        let ready = facts(&rig.service.view.lock().clone().unwrap(), None);
        assert_eq!(
            ready["status"],
            "Ready for a player that decodes TrueHD itself"
        );
        assert_eq!(
            ready["tv"],
            "Rockhopper release 10.2.1-5202 (ponytail-paparoa): the tested build"
        );
        assert_eq!(
            ready["output"],
            "Receiver (eARC). eARC is on. Digital output: passThrough"
        );
        assert_eq!(ready["session"], "No app is using it");
        assert_eq!(ready["receiver"], "Not reported");
        rig.arm();
        rig.run(10_000);
        let playing = facts(
            &rig.service.view.lock().clone().unwrap(),
            Some("DOLBY ATMOS"),
        );
        assert_eq!(
            playing["session"],
            format!("Process {PID} on ADEC{ADEC}: audible")
        );
        assert_eq!(playing["receiver"], "DOLBY ATMOS");
        rig.call("POST", "release", &format!(r#"{{"pid":{PID}}}"#));
        let after = facts(&rig.service.view.lock().clone().unwrap(), None);
        assert_eq!(
            after["ended"],
            "The app stopped playing: the player ended the session"
        );
        let register: plugin_api::wire::Register = serde_json::from_value(register()).unwrap();
        assert!(register.settings.is_some());
    }

    #[test]
    fn what_the_tv_reads_like_after_a_restore_is_on_the_topic_and_the_page() {
        let rig = Rig::new();
        rig.arm();
        rig.run(10_000);
        rig.host.with(|s| s.hdmi_ok = false);
        rig.call("POST", "release", &format!(r#"{{"pid":{PID}}}"#));
        let note = rig.topic()["after"].as_str().expect("a note").to_owned();
        assert!(note.contains("HDMI input service"), "{note}");
        let page = facts(&rig.service.view.lock().clone().unwrap(), None);
        assert_eq!(page["after"], note);
        rig.run(2_100);
        assert_eq!(rig.topic()["ready"], false);
    }

    #[test]
    fn the_player_names_the_decoder_it_was_granted_and_no_other_is_assumed() {
        let rig = Rig::new();
        rig.host.with(|s| s.adec = 3);
        let body = |adec: u8| format!(r#"{{"pid":{PID},"tail":"{TAIL}","adec":{adec}}}"#);
        let (status, refused) = rig.call("POST", "arm", &body(1));
        assert_eq!(status, 200);
        assert_eq!(refused["armed"], false);
        let reason = refused["reason"].as_str().unwrap();
        assert!(reason.contains("ADEC1 cannot be read"), "{reason}");
        assert_eq!(rig.host.log(), Vec::<String>::new());
        assert_eq!(rig.call("POST", "arm", &body(3)).1["armed"], true);
        rig.run(10_000);
        assert_eq!(rig.topic()["session"]["adec"], 3);
    }
}
