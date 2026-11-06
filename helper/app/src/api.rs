//! The v1 API: JSON over the loopback HTTP server, on the hub and the settings store.
//!
//! Every answer is an object with `"ok"` and `"api": 1`; a failure is `{"ok": false, "api": 1, "error": {"code", "message"}}`.
//! Reads carry `"now"` (the helper's clock, Unix ms) so a client can judge ages without trusting its own, and `"boot"` (when
//! this run of the helper started: a different value means a restart, so read the state again).
//!
//! | Method and path | What |
//! |---|---|
//! | `GET /v1` | this list |
//! | `GET /v1/health` | liveness |
//! | `GET /v1/state` | every topic, the `seq` they are current to, and facts about the helper |
//! | `GET /v1/state/{topic}`, `/v1/screen`, `/v1/receiver`, `/v1/weather`, `/v1/news` | one topic |
//! | `GET /v1/events?after=&boot=&timeout=&topics=` | long poll for changes after `seq` |
//! | `POST /v1/receiver/nudge` | read the receiver now and fast for a few seconds |
//! | `POST /v1/receiver/volume` | send the level the volume display shows to the receiver: `{"level": 0 to 98}` (`receiver_volume.rs`) |
//! | `GET /v1/receiver/settings`, `POST /v1/receiver/set` | receiver settings and typed writes |
//! | `POST /v1/refresh` | fetch weather and news now |
//! | `GET /v1/settings`, `GET /v1/settings/{owner}`, `POST /v1/settings/{owner}` | the settings layer |
//! | `GET /v1/voice`, `POST /v1/voice/{listen,stop,cancel,refresh}` | voice search (`voice.rs`) |
//! | `GET /v1/profiles`, `POST /v1/profiles/switch`, `POST|DELETE /v1/profiles/{id}`, `/v1/profiles/{id}/pin`, `/{id}/avatar`, `/options`, `/gallery` | local profiles (`profiles.rs`) |
//!
//! A client's loop: `GET /v1/state` once (it carries `seq` and `boot`), then `GET /v1/events?after=<seq>&boot=<boot>` over and
//! over, each time with the `seq` of the last answer. An answer with `"reset": true` means the client fell behind or the helper
//! restarted: read `/v1/state` again. Every write sends the `X-Luma-Client` header and `Content-Type: application/json`.
//!
//! Topic documents for `weather` and `news` are decorated on the way out with `state` (`fresh`, `stale`, `expired`, `none`) and
//! `ageMs`, and an expired reading's `doc` is `null` (a reading older than 6 hours is never shown). Events carry the stored
//! value as it is.

use crate::API_VERSION;
use crate::homefeeds;
use crate::http::{Handler, Request, Response, error_response};
use crate::hub::Hub;
use crate::receiver;
use crate::settings::{ApplyError, SettingsStore};
use crate::sys;
use crate::timeutil::now_ms;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use settings_schema::SettingChange;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// The default and the longest long-poll wait, in seconds.
const DEFAULT_WAIT_SECS: f64 = 25.0;
const MAX_WAIT_SECS: f64 = 30.0;
use lumatv_feeds::cache::FUTURE_SKEW_MS;

/// Method, path, and what it does: the discovery document and the router's own test read this.
pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1", "the list of endpoints"),
    ("GET", "/v1/health", "liveness"),
    (
        "GET",
        "/v1/state",
        "every topic, the seq they are current to, and facts about the helper",
    ),
    ("GET", "/v1/state/{topic}", "one topic"),
    ("GET", "/v1/screen", "the screen topic"),
    ("GET", "/v1/receiver", "the receiver topic"),
    (
        "GET",
        "/v1/receiver/settings",
        "the receiver topic and interest in its settings",
    ),
    ("GET", "/v1/weather", "the weather topic"),
    ("GET", "/v1/weather/entities", "the Home Assistant weather entities the weather source can use, by name"),
    ("GET", "/v1/news", "the news topic"),
    (
        "GET",
        "/v1/logging",
        "bounded local and best-effort UDP logging status",
    ),
    (
        "GET",
        "/v1/device",
        "what this device is: its screen class and how a person drives it (what plugins are told in welcome)",
    ),
    (
        "GET",
        "/v1/ambient",
        "the ambient light: whether it is on, what it is doing, and why not when it cannot",
    ),
    (
        "POST",
        "/v1/ambient/test",
        "the test pattern, each section of the strip in turn: {\"run\": true} or {\"run\": false}",
    ),
    (
        "POST",
        "/v1/ambient/context",
        "what covers the picture: {\"covered\": bool, \"game\": bool}, either may be left out",
    ),
    (
        "POST",
        "/v1/ambient/refresh",
        "read the saved Hue link again (the command line asks after it changes the link)",
    ),
    (
        "GET",
        "/v1/events",
        "long poll: ?after=<seq>&boot=<boot>&timeout=<seconds, max 30>&topics=<a,b>",
    ),
    (
        "POST",
        "/v1/receiver/nudge",
        "read the receiver now and fast for a few seconds",
    ),
    (
        "POST",
        "/v1/receiver/volume",
        "send the volume display's level to the receiver: {\"level\": 0 to 98}, in halves, coalesced to 10 a second",
    ),
    (
        "POST",
        "/v1/receiver/set",
        "set a receiver setting: setting, value and session",
    ),
    (
        "POST",
        "/v1/refresh",
        "fetch weather and news now: {\"what\": [\"weather\", \"news\"]} (optional)",
    ),
    ("GET", "/v1/settings", "the settings owners"),
    (
        "GET",
        "/v1/settings/{owner}",
        "one owner's schema, stored and effective values",
    ),
    (
        "POST",
        "/v1/settings/{owner}",
        "a transaction: {\"changes\": [{\"key\", \"value\"}], \"ifVersion\": n (optional)}",
    ),
];

/// The API's handler.
pub struct Api {
    pub hub: Arc<Hub>,
    pub settings: Arc<SettingsStore>,
    pub started_at: u64,
    pub data_dir: String,
    /// Which HTTPS transport feeds the weather and news and why: `{"backend": "openssl" | "curl" | "none", "why": "..."}`.
    pub fetch_backend: Value,
    /// The AI-access module: the TV's UI asks it for changes through `/v1/ai/...`.
    pub ai: Arc<crate::ai::Ai>,
    /// The plugin broker: the TV's UI asks a plugin to run an action through `/v1/plugins/...`.
    pub plugins: Arc<crate::plugins::Plugins>,
    /// Ambient light: its status, and the test pattern the Settings page runs.
    pub ambient: Arc<crate::ambient::Ambient>,
    /// Voice search: the mic key's words as text, through Home Assistant's Assist pipeline (`/v1/voice/...`).
    pub voice: Arc<crate::voice::Voice>,
    /// Pictures for the shell, decoded off the CPU where the TV can (`/v1/image`, `/v1/images`).
    pub images: Arc<crate::images::Images>,
    /// Cameras through Scrypted's LumaShow bridge: stills, live video, the timeline, events and the pop-ups (`/v1/cameras/...`).
    pub cameras: crate::scrypted::Scrypted,
    /// On Home: Home Assistant's shared entities, live, and the controls of their tiles (`/v1/onhome/...`, the `onhome` topic).
    pub onhome: crate::onhome::OnHome,
    /// Screenshots and screen recordings: the way in to the compositor's capture, behind `capture.enabled` (`/v1/capture/...`).
    pub capture: crate::capture::Capture,
    /// The picker's pictures of the inputs, behind the owner's `inputs.previews` switch (`/v1/previews/...`).
    pub previews: crate::previews::Previews,
    /// The display lab: the shell's guided look at the display chip, the TV's own shell only (`/v1/displaylab/...`).
    pub displaylab: Arc<crate::displaylab::DisplayLab>,
    /// Local profiles: each person's Home and who may change what (`/v1/profiles/...`, the `profiles` topic).
    pub profiles: Arc<crate::profiles::Profiles>,
    pub privacy: Arc<lumatv_privacy::Privacy>,
    pub receiver_tx: Sender<receiver::Cmd>,
    pub feeds_tx: Sender<homefeeds::Cmd>,
    /// The full weather view's radar and base map (`/v1/wx/...`).
    pub wx: Arc<crate::wxview::WxView>,
    /// Weather from a Home Assistant entity: the list the Settings picker offers (`/v1/weather/entities`).
    pub weather_ha: Arc<crate::ha_weather::HaWeather>,
    /// The Guide data plugin's listings, which the TV's guide and banner lay over LG's (`/v1/guide/...`).
    pub guide: Arc<crate::guidedata::GuideData>,
    /// The TV's Plex account: the sign-in by code and the server everything of Plex uses (`/v1/plex/...`, the `plex.account` topic).
    pub plex: Arc<crate::plex::PlexAccount>,
}

/// A topic name as it may appear in a path or a filter.
fn topic_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// Adds `ok`, `api` and `now` to an object body.
fn envelope(mut body: Value, now: u64) -> Value {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now));
    }
    body
}

/// What a saved document's age makes of it: `fresh`, `stale` (due for a refetch, still shown), `expired` (never shown) or `none`.
/// A reading stamped further ahead than ordinary clock skew was written by (or is judged by) a wrong clock: it is shown, as stale.
fn freshness_word(doc: &Value, now: u64) -> (&'static str, Option<u64>) {
    let (Some(fetched), Some(stale), Some(expires)) = (
        doc.get("fetchedAt").and_then(Value::as_u64),
        doc.get("staleAt").and_then(Value::as_u64),
        doc.get("expiresAt").and_then(Value::as_u64),
    ) else {
        return ("none", None);
    };
    let age = now.saturating_sub(fetched);
    let word = if fetched > now.saturating_add(FUTURE_SKEW_MS) {
        "stale"
    } else if now >= expires {
        "expired"
    } else if now >= stale {
        "stale"
    } else {
        "fresh"
    };
    (word, Some(age))
}

/// `weather` and `news` topics with `state` and `ageMs` added, and an expired `doc` removed; any other topic as it is.
pub fn decorate(topic: &str, value: &Value, now: u64) -> Value {
    if topic != homefeeds::WEATHER_TOPIC && topic != homefeeds::NEWS_TOPIC {
        return value.clone();
    }
    let mut out = value.clone();
    let Some(map) = out.as_object_mut() else {
        return out;
    };
    let doc = map.get("doc").cloned().unwrap_or(Value::Null);
    let (state, age) = if doc.is_null() {
        ("none", None)
    } else {
        freshness_word(&doc, now)
    };
    if state == "expired" {
        map.insert("doc".into(), Value::Null);
    }
    map.insert("state".into(), json!(state));
    map.insert("ageMs".into(), json!(age));
    out
}

fn bad_request(message: &str) -> Response {
    error_response(400, "bad_request", message)
}

fn not_found() -> Response {
    error_response(404, "not_found", "no such endpoint")
}

/// A JSON body that is an object, or an empty body as an empty object.
fn body_object(request: &Request) -> Result<Map<String, Value>, Response> {
    if request.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(bad_request("the body must be a JSON object")),
        Err(e) => Err(bad_request(&format!("the body is not JSON: {e}"))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverSetBody {
    setting: String,
    #[serde(deserialize_with = "Value::deserialize")]
    value: Value,
    session: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ApplyBody {
    changes: Vec<SettingChange>,
    #[serde(default)]
    if_version: Option<u64>,
}

impl Api {
    fn ok(&self, body: Value) -> Response {
        Response::json(200, &envelope(body, now_ms()))
    }

    /// The facts plugins are told in `welcome`, as they stand now (a game controller that is plugged in shows).
    fn device(&self) -> Response {
        match self.plugins.shared().device_facts() {
            Some(device) => self.ok(json!({"device": device})),
            None => error_response(404, "no_device_facts", "this helper has no device facts"),
        }
    }

    fn ambient_status(&self) -> Response {
        self.ok(json!({"ambient": self.ambient.status()}))
    }

    fn ambient_test(&self, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(b) => b,
            Err(r) => return r,
        };
        let Some(run) = body.get("run").and_then(Value::as_bool) else {
            return bad_request("run must be true or false");
        };
        match self.ambient.test(run) {
            Ok(()) => self.ok(json!({"ambient": self.ambient.status()})),
            Err(crate::ambient::TestError::NoStrip) => error_response(
                409,
                "no_strip",
                "no strip is set up: helper.json needs ambient.wled.host",
            ),
            Err(crate::ambient::TestError::NoLights(why)) => error_response(409, "no_lights", &why),
        }
    }

    /// Reads the saved Zigbee2MQTT and Hue links again (the command line asks after it changes one).
    fn ambient_refresh(&self) -> Response {
        self.ambient.reload_links();
        self.ok(json!({"ambient": self.ambient.status()}))
    }

    fn ambient_context(&self, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(b) => b,
            Err(r) => return r,
        };
        let mut flags = [None, None];
        for (slot, name) in flags.iter_mut().zip(["covered", "game"]) {
            match body.get(name) {
                None => {}
                Some(Value::Bool(b)) => *slot = Some(*b),
                Some(_) => return bad_request(&format!("{name} must be true or false")),
            }
        }
        self.ambient.set_context(flags[0], flags[1]);
        self.ok(json!({"ambient": self.ambient.status()}))
    }

    fn receiver_settings(&self, request: &Request) -> Response {
        if !request.peer.is_loopback() {
            return error_response(403, "local_only", "receiver settings are local only");
        }
        let _ = self.receiver_tx.send(receiver::Cmd::SettingsInterest);
        self.topic("receiver")
    }

    fn receiver_set(&self, request: &Request) -> Response {
        if !request.peer.is_loopback() {
            return error_response(403, "local_only", "receiver settings are local only");
        }
        if let Err(response) = self.profiles.require_owner() {
            return response;
        }
        if request
            .body
            .iter()
            .copied()
            .find(|b| !b.is_ascii_whitespace())
            != Some(b'{')
        {
            return bad_request("expected exactly setting, value and session");
        }
        let body: ReceiverSetBody = match serde_json::from_slice(&request.body) {
            Ok(body) => body,
            Err(_) => return bad_request("expected exactly setting, value and session"),
        };
        let (reply, response) = std::sync::mpsc::channel();
        if self
            .receiver_tx
            .send(receiver::Cmd::SettingsSet {
                setting: body.setting,
                value: body.value,
                session: body.session,
                reply,
            })
            .is_err()
        {
            return error_response(503, "worker_error", "receiver worker unavailable");
        }
        match response.recv() {
            Ok(receiver::SettingsResponse::Accepted {
                request_id,
                session,
            }) => Response::json(
                202,
                &envelope(
                    json!({"accepted": true, "requestId": request_id, "session": session}),
                    now_ms(),
                ),
            ),
            Ok(receiver::SettingsResponse::Invalid(message)) => bad_request(&message),
            Ok(receiver::SettingsResponse::Busy) => error_response(409, "busy", "receiver is busy"),
            Ok(receiver::SettingsResponse::Unavailable(message)) => {
                error_response(503, "unavailable", &message)
            }
            Ok(receiver::SettingsResponse::WorkerError) | Err(_) => {
                error_response(503, "worker_error", "receiver worker unavailable")
            }
        }
    }

    /// The volume display's level, for the receiver thread to send as an absolute level. Only the newest level of a burst is sent.
    fn receiver_volume(&self, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(b) => b,
            Err(r) => return r,
        };
        let Some(level) = body.get("level").and_then(Value::as_f64) else {
            return bad_request("level must be a number from 0 to 98");
        };
        if !(0.0..=crate::receiver_volume::MAX_LEVEL).contains(&level) {
            return bad_request("level must be a number from 0 to 98");
        }
        let level = crate::receiver_volume::quantize(level).unwrap_or(level);
        let _ = self
            .receiver_tx
            .send(receiver::Cmd::Volume((level * 2.0) as u16));
        self.ok(json!({"requested": true, "level": level}))
    }

    fn helper_info(&self, now: u64) -> Value {
        json!({
            "name": "lumatv-helper",
            "version": crate::VERSION,
            "api": API_VERSION,
            "pid": sys::pid(),
            "startedAt": self.started_at,
            "uptimeMs": now.saturating_sub(self.started_at),
            "rssKb": sys::rss_kb(),
            "threads": sys::threads(),
            "dataDir": self.data_dir,
            "fetch": self.fetch_backend,
        })
    }

    fn discovery(&self) -> Response {
        let endpoints: Vec<Value> = ROUTES
            .iter()
            .chain(crate::ai::routes::ROUTES.iter())
            .chain(crate::plugins::routes::ROUTES.iter())
            .chain(crate::voice::ROUTES.iter())
            .chain(crate::images::ROUTES.iter())
            .chain(crate::gfn_boost::ROUTES.iter())
            .chain(crate::crash::ROUTES.iter())
            .chain(crate::openapps::ROUTES.iter())
            .chain(crate::capture::ROUTES.iter())
            .chain(crate::previews::ROUTES.iter())
            .chain(crate::placement::ROUTES.iter())
            .chain(crate::displaylab::ROUTES.iter())
            .chain(crate::saver::ROUTES.iter())
            .chain(crate::profiles::ROUTES.iter())
            .chain(crate::privacy::ROUTES.iter())
            .chain(crate::scrypted::routes::ROUTES.iter())
            .chain(crate::onhome::ROUTES.iter())
            .chain(crate::guidedata::ROUTES.iter())
            .chain(crate::plex::ROUTES.iter())
            .map(|(method, path, what)| json!({"method": method, "path": path, "what": what}))
            .collect();
        let now = now_ms();
        self.ok(json!({"boot": self.hub.boot(), "helper": self.helper_info(now), "endpoints": endpoints}))
    }

    fn state(&self) -> Response {
        let now = now_ms();
        let snapshot = self.hub.snapshot();
        let topics: Map<String, Value> = snapshot
            .topics
            .iter()
            .map(|(name, value)| (name.clone(), decorate(name, value, now)))
            .collect();
        Response::json(
            200,
            &envelope(
                json!({"boot": self.hub.boot(), "seq": snapshot.seq, "helper": self.helper_info(now), "topics": topics}),
                now,
            ),
        )
    }

    fn topic(&self, name: &str) -> Response {
        if !topic_ok(name) {
            return not_found();
        }
        let now = now_ms();
        let (seq, value) = self.hub.topic_at(name);
        let Some(mut value) = value else {
            return error_response(
                404,
                "no_such_topic",
                "nothing has been published under that topic",
            );
        };
        if name == "logging" {
            // Desired settings remain authoritative even if RAM protection stopped the worker.
            let enabled = self
                .settings
                .flag("base.logging", "enabled")
                .unwrap_or(false);
            let server = json!({"host": self.settings.text("base.logging", "host").unwrap_or_default(), "port": self.settings.integer("base.logging", "port").unwrap_or(514)});
            if value["server"] != server || value["enabled"] != enabled {
                value["active"] = json!(false);
            }
            value["enabled"] = json!(enabled);
            value["server"] = server;
            if let Some(sources) = self.hub.get("loggingSources") {
                value["sources"] = sources;
            }
        }
        Response::json(
            200,
            &envelope(
                json!({"boot": self.hub.boot(), "seq": seq, "topic": name, "value": decorate(name, &value, now)}),
                now,
            ),
        )
    }

    fn events(&self, request: &Request) -> Response {
        let parse_u64 = |name: &str| -> Result<Option<u64>, Response> {
            match request.param(name) {
                None | Some("") => Ok(None),
                Some(text) => text
                    .parse::<u64>()
                    .map(Some)
                    .map_err(|_| bad_request(&format!("{name} must be a whole number"))),
            }
        };
        let after = match parse_u64("after") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let boot = match parse_u64("boot") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let wait = match request.param("timeout") {
            None | Some("") => DEFAULT_WAIT_SECS,
            Some(text) => match text.parse::<f64>() {
                Ok(secs) if secs.is_finite() && secs >= 0.0 => secs.min(MAX_WAIT_SECS),
                _ => return bad_request("timeout must be a number of seconds"),
            },
        };
        let topics: Option<Vec<String>> = match request.param("topics") {
            None | Some("") => None,
            Some(list) => {
                let names: Vec<String> = list.split(',').map(str::to_owned).collect();
                if names.len() > 16 || !names.iter().all(|n| topic_ok(n)) {
                    return bad_request(
                        "topics is a comma-separated list of at most 16 topic names",
                    );
                }
                Some(names)
            }
        };
        let batch = self.hub.wait_events(
            after,
            boot,
            topics.as_deref(),
            Duration::from_secs_f64(wait),
        );
        let events: Vec<Value> = batch
            .events
            .iter()
            .map(crate::hub::Event::to_json)
            .collect();
        self.ok(json!({"boot": self.hub.boot(), "seq": batch.seq, "reset": batch.reset, "events": events}))
    }

    fn weather_entities(&self) -> Response {
        match self.weather_ha.entities() {
            Ok(list) => self.ok(json!({"entities": list.into_iter().map(|e| json!({"id": e.id, "name": e.name})).collect::<Vec<_>>()})),
            Err(w) => error_response(if w.code == "no_link" { 409 } else { 502 }, w.code, &w.words),
        }
    }

    fn refresh(&self, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(b) => b,
            Err(r) => return r,
        };
        if let Some(what) = body.get("what") {
            let known = |v: &Value| matches!(v.as_str(), Some("weather" | "news"));
            if !what.as_array().is_some_and(|a| a.iter().all(known)) {
                return bad_request("what is a list of \"weather\" and \"news\"");
            }
        }
        // Both feeds share one thread, and a refresh of one is cheap next to the connection: both are fetched.
        let _ = self.feeds_tx.send(homefeeds::Cmd::Refresh);
        self.ok(json!({"requested": true}))
    }

    fn settings_list(&self) -> Response {
        let owners: Vec<Value> = self
            .settings
            .owners()
            .into_iter()
            .map(|s| json!({"owner": s.owner, "title": s.title, "schemaRevision": s.schema_revision, "version": s.version}))
            .collect();
        self.ok(json!({"owners": owners}))
    }

    fn settings_get(&self, owner: &str) -> Response {
        match self.settings.describe(owner) {
            Some(view) => self.ok(view),
            None => error_response(404, "no_such_owner", "no such settings owner"),
        }
    }

    fn settings_post(&self, owner: &str, request: &Request) -> Response {
        let body: ApplyBody = match serde_json::from_slice(&request.body) {
            Ok(b) => b,
            Err(e) => return bad_request(&format!("the body is not a settings transaction: {e}")),
        };
        match self.settings.apply(owner, body.changes, body.if_version) {
            Ok(applied) => {
                let view = self.settings.describe(owner).unwrap_or(Value::Null);
                self.ok(json!({"changed": applied.changed, "version": applied.version, "settings": view}))
            }
            Err(ApplyError::UnknownOwner) => {
                error_response(404, "no_such_owner", "no such settings owner")
            }
            Err(ApplyError::Conflict { current }) => Response::json(
                409,
                &json!({"ok": false, "api": API_VERSION, "error": {
                    "code": "conflict", "message": "the settings changed since they were read", "version": current}}),
            ),
            Err(ApplyError::Invalid(fields)) => {
                let map: Map<String, Value> = fields
                    .iter()
                    .map(|(k, e)| (k.as_str().to_owned(), Value::String(e.to_string())))
                    .collect();
                Response::json(
                    422,
                    &json!({"ok": false, "api": API_VERSION, "error": {
                        "code": "invalid", "message": "a setting was refused", "fields": map}}),
                )
            }
            Err(ApplyError::Protected) => error_response(
                403,
                "protected_owner",
                "these settings can only be changed on the TV",
            ),
            Err(ApplyError::Io(why)) => error_response(500, "io", &why),
        }
    }
}

impl Handler for Api {
    fn handle(&self, request: &Request) -> Response {
        let trimmed = request.path.trim_matches('/');
        let segments: Vec<&str> = trimmed.split('/').collect();
        let get = request.method == "GET";
        let post = request.method == "POST";
        match segments.as_slice() {
            [""] | ["v1"] if get => self.discovery(),
            ["v1", "health"] if get => self.ok(json!({"boot": self.hub.boot()})),
            ["v1", "state"] if get => self.state(),
            ["v1", "state", topic] if get => self.topic(topic),
            ["v1", name @ ("screen" | "receiver" | "weather" | "news")] if get => self.topic(name),
            ["v1", "events"] if get => self.events(request),
            ["v1", "device"] if get => self.device(),
            ["v1", "ambient"] if get => self.ambient_status(),
            ["v1", "ambient", "test"] if post => self.ambient_test(request),
            ["v1", "ambient", "context"] if post => self.ambient_context(request),
            ["v1", "ambient", "refresh"] if post => self.ambient_refresh(),
            ["v1", "receiver", "nudge"] if post => {
                let _ = self.receiver_tx.send(receiver::Cmd::Nudge);
                self.ok(json!({"requested": true}))
            }
            ["v1", "receiver", "volume"] if post => self.receiver_volume(request),
            ["v1", "receiver", "settings"] if get => self.receiver_settings(request),
            ["v1", "receiver", "set"] if post => self.receiver_set(request),
            ["v1", "refresh"] if post => self.refresh(request),
            ["v1", "settings"] if get => self.settings_list(),
            ["v1", "settings", owner] if get => self.settings_get(owner),
            ["v1", "settings", owner] if post => self.settings_post(owner, request),
            ["v1", "ai", rest @ ..] => crate::ai::routes::handle(&self.ai, request, rest),
            ["v1", "plugins", rest @ ..] => {
                crate::plugins::routes::handle(&self.plugins, request, rest)
            }
            ["v1", "guide", rest @ ..] => crate::guidedata::handle(&self.guide, request, rest),
            ["v1", "plex", rest @ ..] => crate::plex::handle(&self.plex, &self.hub, request, rest, self.profiles.require_owner()),
            ["v1", "voice", rest @ ..] => crate::voice::handle(&self.voice, request, rest),
            ["v1", "weather", "entities"] if get => self.weather_entities(),
            ["v1", "wx", rest @ ..] => self.wx.handle(request, rest),
            ["v1", "onhome", rest @ ..] => {
                // which of the shared things Home shows is the owner's, as the rest of Home's layout is
                if post && rest == ["picks"] {
                    if let Err(resp) = self.profiles.require_owner() {
                        return resp;
                    }
                }
                crate::onhome::handle(&self.onhome, request, rest)
            }
            ["v1", "gfn", rest @ ..] => crate::gfn_boost::handle(request, rest),
            ["v1", "logging"] if get => self.topic("logging"),
            ["v1", "crash", rest @ ..] => crate::crash::handle(request, rest),
            ["v1", "openapps"] => {
                if post {
                    if let Err(resp) = self.profiles.require_owner() {
                        return resp;
                    }
                }
                crate::openapps::handle(&self.hub, request)
            }
            ["v1", "capture", rest @ ..] => self.capture.handle(request, rest),
            ["v1", "previews", rest @ ..] => self.previews.handle(request, rest),
            ["v1", "placement", rest @ ..] => crate::placement::handle(request, rest),
            ["v1", "displaylab", rest @ ..] => self.displaylab.handle(request, rest),
            ["v1", "saver", rest @ ..] => {
                // what shows over the saver is the owner's to change; the complications are for any tool on the TV to post
                if post && rest.first() == Some(&"rules") {
                    if let Err(resp) = self.profiles.require_owner() {
                        return resp;
                    }
                }
                crate::saver::handle(&self.hub, request, rest)
            }
            ["v1", "profiles", rest @ ..] => self.profiles.handle(request, rest),
            ["v1", "privacy", rest @ ..] => {
                crate::privacy::handle(&self.privacy, request, rest, self.profiles.can_admin())
            }
            ["v1", "image", root, file] if get => self.images.serve(request, root, file),
            ["v1", "images"] if get => self.ok(self.images.status()),
            ["v1", "cameras", rest @ ..] => self.cameras.handle(request, rest),
            _ if is_known_path(&segments) => {
                error_response(405, "method_not_allowed", "that method is not served here")
                    .with_header("Allow", allowed_methods(&segments))
            }
            _ => not_found(),
        }
    }
}

/// Whether the path exists for some method.
fn is_known_path(segments: &[&str]) -> bool {
    matches!(
        segments,
        [""] | ["v1"]
            | [
                "v1",
                "health"
                    | "state"
                    | "events"
                    | "refresh"
                    | "settings"
                    | "screen"
                    | "receiver"
                    | "weather"
                    | "news"
                    | "logging"
                    | "device"
                    | "ambient"
                    | "images"
            ]
            | ["v1", "image", _, _]
            | ["v1", "state" | "settings", _]
            | ["v1", "receiver", "nudge" | "volume" | "settings" | "set"]
            | ["v1", "ambient", "test" | "context" | "refresh"]
    )
}

fn allowed_methods(segments: &[&str]) -> &'static str {
    match segments {
        ["v1", "refresh"]
        | ["v1", "receiver", "nudge" | "volume" | "set"]
        | ["v1", "ambient", "test" | "context" | "refresh"] => "POST",
        ["v1", "settings", _] => "GET, POST",
        _ => "GET",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct Rig {
        api: Api,
        _dir: tempfile::TempDir,
        receiver_rx: mpsc::Receiver<receiver::Cmd>,
        feeds_rx: mpsc::Receiver<homefeeds::Cmd>,
    }

    struct NoFetch;

    impl fetch::Fetch for NoFetch {
        fn get(
            &self,
            _url: &str,
            _limits: fetch::FetchLimits,
        ) -> Result<Vec<u8>, fetch::FetchError> {
            Err(fetch::FetchError::Offline)
        }
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(777));
        let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
        settings.register_builtin().unwrap();
        let (receiver_tx, receiver_rx) = mpsc::channel();
        let (feeds_tx, feeds_rx) = mpsc::channel();
        let ctx = crate::ctx::Ctx {
            config: Arc::new(crate::config::Config::default()),
            paths: crate::config::Paths::new(dir.path()),
            hub: hub.clone(),
            settings: settings.clone(),
            stop: crate::control::Stop::new(),
        };
        let ai =
            crate::ai::Ai::start(&ctx, crate::ai::testing::inert_deps(), Arc::new(|| {})).unwrap();
        let plugins = crate::plugins::Plugins::new(&ctx);
        let images = Arc::new(crate::images::Images::new(&ctx.config, &ctx.paths));
        Rig {
            api: Api {
                hub,
                settings,
                ai,
                ambient: crate::ambient::Ambient::new(&ctx),
                voice: crate::voice::Voice::new(&ctx),
                cameras: crate::scrypted::Scrypted::new(
                    &ctx,
                    crate::scrypted::testing::deps(),
                    images.clone(),
                    plugins.shared(),
                ),
                onhome: crate::onhome::OnHome::new(&ctx, crate::onhome::testing::deps()),
                images,
                plugins,
                capture: crate::capture::Capture::new(&ctx.config.capture),
                previews: crate::previews::Previews::new(&ctx.config, &ctx.paths, ctx.hub.clone()),
                displaylab: crate::displaylab::DisplayLab::start(&ctx),
                profiles: crate::profiles::Profiles::open(&ctx),
                privacy: crate::privacy::new(&ctx),
                started_at: 1000,
                data_dir: "/var/lib/test".into(),
                fetch_backend: json!({"backend": "test"}),
                receiver_tx,
                feeds_tx,
                wx: crate::wxview::WxView::new(Arc::new(NoFetch), dir.path()),
                weather_ha: Arc::new(crate::ha_weather::HaWeather::new(Vec::new(), ha_link::AddressPolicy::HOME, None)),
                guide: Arc::default(),
                plex: crate::plex::PlexAccount::fixed(None),
            },
            _dir: dir,
            receiver_rx,
            feeds_rx,
        }
    }

    fn request(method: &str, target: &str, body: &str) -> Request {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        Request {
            method: method.into(),
            path: path.into(),
            query: query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap_or((p, ""));
                    (k.to_owned(), v.to_owned())
                })
                .collect(),
            headers: vec![],
            body: body.as_bytes().to_vec(),
            peer: std::net::Ipv4Addr::LOCALHOST.into(),
        }
    }

    fn body(response: &Response) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn every_answer_has_the_envelope() {
        let rig = rig();
        for path in ["/", "/v1", "/v1/health", "/v1/state", "/v1/settings"] {
            let r = rig.api.handle(&request("GET", path, ""));
            assert_eq!(r.status, 200, "{path}");
            let b = body(&r);
            assert_eq!(b["ok"], true, "{path}");
            assert_eq!(b["api"], 1, "{path}");
            assert!(b["now"].as_u64().unwrap() > 1_700_000_000_000, "{path}");
        }
    }

    #[test]
    fn errors_have_the_error_envelope() {
        let rig = rig();
        let r = rig.api.handle(&request("GET", "/v1/nope", ""));
        assert_eq!(r.status, 404);
        let b = body(&r);
        assert_eq!(
            (b["ok"].clone(), b["api"].clone()),
            (json!(false), json!(1))
        );
        assert_eq!(b["error"]["code"], "not_found");
        let r = rig.api.handle(&request("POST", "/v1/state", ""));
        assert_eq!(r.status, 405);
        assert!(r.headers.iter().any(|(n, v)| *n == "Allow" && v == "GET"));
        let r = rig.api.handle(&request("GET", "/v1/refresh", ""));
        assert_eq!(r.status, 405);
        assert!(r.headers.iter().any(|(n, v)| *n == "Allow" && v == "POST"));
    }

    #[test]
    fn the_discovery_document_lists_every_route_and_every_route_is_served() {
        let rig = rig();
        let b = body(&rig.api.handle(&request("GET", "/v1", "")));
        let listed = b["endpoints"].as_array().unwrap();
        assert_eq!(
            listed.len(),
            ROUTES.len()
                + crate::ai::routes::ROUTES.len()
                + crate::plugins::routes::ROUTES.len()
                + crate::voice::ROUTES.len()
                + crate::images::ROUTES.len()
                + crate::gfn_boost::ROUTES.len()
                + crate::crash::ROUTES.len()
                + crate::openapps::ROUTES.len()
                + crate::capture::ROUTES.len()
                + crate::previews::ROUTES.len()
                + crate::placement::ROUTES.len()
                + crate::displaylab::ROUTES.len()
                + crate::saver::ROUTES.len()
                + crate::profiles::ROUTES.len()
                + crate::privacy::ROUTES.len()
                + crate::scrypted::routes::ROUTES.len()
                + crate::onhome::ROUTES.len()
                + crate::guidedata::ROUTES.len()
                + crate::plex::ROUTES.len()
        );
        assert_eq!(b["helper"]["version"], crate::VERSION);
        assert_eq!(b["boot"], 777);
        // The AI routes have their own tests (tests/ai.rs); here only that the router hands them on.
        assert_eq!(rig.api.handle(&request("GET", "/v1/ai", "")).status, 200);
        assert_eq!(
            rig.api.handle(&request("GET", "/v1/ai/nope", "")).status,
            404
        );
        for (method, path, _) in ROUTES {
            let concrete = path
                .replace("{topic}", "screen")
                .replace("{owner}", "base.news");
            let r = rig.api.handle(&request(
                method,
                &concrete,
                if *method == "POST" && concrete.contains("settings") {
                    r#"{"changes":[]}"#
                } else {
                    ""
                },
            ));
            // 404 would mean the table lists a route the router does not have; a missing topic is the one honest 404 here.
            assert!(
                r.status != 405
                    && (r.status != 404 || body(&r)["error"]["code"] == "no_such_topic"),
                "{method} {path} -> {} {}",
                r.status,
                String::from_utf8_lossy(&r.body)
            );
        }
    }

    #[test]
    fn the_device_route_serves_what_a_plugin_is_told() {
        let rig = rig();
        let b = body(&rig.api.handle(&request("GET", "/v1/device", "")));
        assert_eq!(b["ok"], true);
        assert_eq!(b["device"]["screen"]["class"], "tv");
        assert_eq!(b["device"]["screen"]["width"], 1920);
        // The same facts the broker puts in welcome (a controller may have been plugged in between, hence only the fixed part).
        assert_eq!(b["device"]["input"][0], "remote");
        let r = rig.api.handle(&request("POST", "/v1/device", ""));
        assert_eq!(r.status, 405);
    }

    #[test]
    fn the_image_routes_are_listed_and_answer_through_the_router() {
        let rig = rig();
        let listed = body(&rig.api.handle(&request("GET", "/v1", "")));
        let paths: Vec<&str> = listed["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["path"].as_str())
            .collect();
        assert!(paths.contains(&"/v1/image/{root}/{file}") && paths.contains(&"/v1/images"));
        let status = body(&rig.api.handle(&request("GET", "/v1/images", "")));
        assert_eq!(status["images"]["enabled"], true);
        assert_eq!(
            status["images"]["roots"]["images"],
            rig._dir.path().join("images").display().to_string()
        );
        assert_eq!(
            rig.api
                .handle(&request("GET", "/v1/image/images/none.jpg", ""))
                .status,
            404
        );
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/image/images/none.jpg", ""))
                .status,
            405
        );
        assert_eq!(
            rig.api
                .handle(&request("GET", "/v1/image/images", ""))
                .status,
            404
        );
    }

    #[test]
    fn logging_status_keeps_desired_settings_when_the_worker_is_unavailable() {
        let rig = rig();
        rig.api.hub.set("logging", json!({"enabled":false,"active":false,"server":{"host":"","port":514},"transport":"udp","error":"RAM filesystem required","ramLimitBytes":1048576}));
        let response = rig.api.handle(&request("POST", "/v1/settings/base.logging", r#"{"changes":[{"key":"host","value":{"type":"text","value":"127.0.0.1"}},{"key":"port","value":{"type":"integer","value":1514}},{"key":"enabled","value":{"type":"bool","value":true}}]}"#));
        assert_eq!(response.status, 200);
        let value = body(&rig.api.handle(&request("GET", "/v1/logging", "")));
        assert_eq!(value["ok"], true);
        assert_eq!(value["topic"], "logging");
        assert_eq!(
            value["value"]["server"],
            json!({"host":"127.0.0.1","port":1514})
        );
        assert_eq!(value["value"]["enabled"], true);
        assert_eq!(value["value"]["active"], false);
        assert_eq!(value["value"]["error"], "RAM filesystem required");
    }

    #[test]
    fn the_ambient_routes_report_run_the_test_pattern_and_take_what_covers_the_picture() {
        let rig = rig();
        let status = body(&rig.api.handle(&request("GET", "/v1/ambient", "")));
        assert_eq!(status["ambient"]["state"], "off");
        assert_eq!(status["ambient"]["available"], false);
        assert_eq!(status["ambient"]["sections"], 58);
        // With no strip named in helper.json the test is refused, in words, and nothing is started.
        let refused = rig
            .api
            .handle(&request("POST", "/v1/ambient/test", r#"{"run":true}"#));
        assert_eq!(refused.status, 409);
        assert_eq!(body(&refused)["error"]["code"], "no_strip");
        // Stopping is always allowed; the request must say which.
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/ambient/test", r#"{"run":false}"#))
                .status,
            200
        );
        for bad in ["{}", r#"{"run":"yes"}"#, "[true]"] {
            let r = rig.api.handle(&request("POST", "/v1/ambient/test", bad));
            assert_eq!(r.status, 400, "{bad}");
        }
        // What covers the picture.
        let covered = body(&rig.api.handle(&request(
            "POST",
            "/v1/ambient/context",
            r#"{"covered":true}"#,
        )));
        assert_eq!(covered["ambient"]["covered"], true);
        let r = rig.api.handle(&request(
            "POST",
            "/v1/ambient/context",
            r#"{"game":"maybe"}"#,
        ));
        assert_eq!(r.status, 400);
        let r = rig.api.handle(&request("GET", "/v1/ambient/test", ""));
        assert_eq!(r.status, 405);
        assert!(r.headers.iter().any(|(n, v)| *n == "Allow" && v == "POST"));
    }

    #[test]
    fn state_carries_the_seq_and_every_topic() {
        let rig = rig();
        let base = rig.api.hub.snapshot().seq;
        rig.api.hub.set("screen", json!({"on": true}));
        rig.api.hub.set("receiver", json!({"fresh": false}));
        let b = body(&rig.api.handle(&request("GET", "/v1/state", "")));
        assert_eq!(b["seq"], base + 2);
        assert_eq!(b["boot"], 777);
        assert_eq!(b["topics"]["screen"], json!({"on": true}));
        assert_eq!(b["helper"]["name"], "lumatv-helper");
        assert_eq!(b["helper"]["startedAt"], 1000);
        assert!(b["helper"]["rssKb"].as_u64().unwrap() > 0);
    }

    #[test]
    fn one_topic_and_the_aliases() {
        let rig = rig();
        let base = rig.api.hub.snapshot().seq;
        rig.api.hub.set("screen", json!({"on": false}));
        for path in ["/v1/screen", "/v1/state/screen"] {
            let b = body(&rig.api.handle(&request("GET", path, "")));
            assert_eq!(b["topic"], "screen");
            assert_eq!(b["value"], json!({"on": false}));
            assert_eq!(b["seq"], base + 1);
        }
        let r = rig.api.handle(&request("GET", "/v1/receiver", ""));
        assert_eq!(r.status, 404);
        assert_eq!(body(&r)["error"]["code"], "no_such_topic");
        assert_eq!(
            rig.api
                .handle(&request("GET", "/v1/state/Bad%20Name", ""))
                .status,
            404
        );
    }

    fn weather_doc(fetched: u64, stale: u64, expires: u64) -> Value {
        json!({"doc": {"fetchedAt": fetched, "staleAt": stale, "expiresAt": expires, "place": "Atlanta"}, "status": {}})
    }

    #[test]
    fn saved_documents_are_decorated_with_their_age() {
        let now = 10_000_000_000u64;
        let fresh = decorate(
            "weather",
            &weather_doc(now - 1_000, now + 899_000, now + 21_599_000),
            now,
        );
        assert_eq!(
            (fresh["state"].clone(), fresh["ageMs"].clone()),
            (json!("fresh"), json!(1_000))
        );
        assert_eq!(fresh["doc"]["place"], "Atlanta");
        let stale = decorate(
            "news",
            &weather_doc(now - 1_000_000, now - 100_000, now + 100_000),
            now,
        );
        assert_eq!(stale["state"], "stale");
        assert_eq!(stale["doc"]["place"], "Atlanta");
        let expired = decorate(
            "weather",
            &weather_doc(now - 30_000_000, now - 29_000_000, now - 1),
            now,
        );
        assert_eq!(expired["state"], "expired");
        assert!(expired["doc"].is_null());
        // Stamped in the future (a clock that was wrong): shown, as stale, never dropped.
        let future = decorate(
            "weather",
            &weather_doc(now + 400_000, now + 1_000_000, now + 22_000_000),
            now,
        );
        assert_eq!(future["state"], "stale");
        assert_eq!(future["doc"]["place"], "Atlanta");
        let none = decorate(
            "weather",
            &json!({"doc": null, "status": {"failures": 1}}),
            now,
        );
        assert_eq!(
            (none["state"].clone(), none["ageMs"].clone()),
            (json!("none"), Value::Null)
        );
        // Other topics are left as they are.
        assert_eq!(
            decorate("screen", &json!({"on": true}), now),
            json!({"on": true})
        );
        // The boundary: exactly at staleAt it is stale, exactly at expiresAt expired.
        assert_eq!(
            decorate("weather", &weather_doc(now - 5, now, now + 5), now)["state"],
            "stale"
        );
        assert_eq!(
            decorate("weather", &weather_doc(now - 5, now - 1, now), now)["state"],
            "expired"
        );
    }

    #[test]
    fn events_without_after_report_the_seq_at_once() {
        let rig = rig();
        let base = rig.api.hub.snapshot().seq;
        rig.api.hub.set("a", json!(1));
        let b = body(&rig.api.handle(&request("GET", "/v1/events?timeout=20", "")));
        assert_eq!(b["seq"], base + 1);
        assert_eq!(b["events"], json!([]));
        assert_eq!(b["reset"], false);
    }

    #[test]
    fn events_after_a_seq_are_returned_and_filtered() {
        let rig = rig();
        let base = rig.api.hub.snapshot().seq;
        rig.api.hub.set("a", json!(1));
        rig.api.hub.set("b", json!(2));
        let b = body(&rig.api.handle(&request(
            "GET",
            &format!("/v1/events?after={base}&boot=777&topics=b"),
            "",
        )));
        assert_eq!(b["events"].as_array().unwrap().len(), 1);
        assert_eq!(b["events"][0]["topic"], "b");
        assert_eq!(b["events"][0]["kind"], "changed");
        assert_eq!(b["events"][0]["data"], json!(2));
        assert_eq!(b["seq"], base + 2);
        let again = body(&rig.api.handle(&request(
            "GET",
            &format!("/v1/events?after={}&boot=777&timeout=0", base + 2),
            "",
        )));
        assert_eq!(again["events"], json!([]));
    }

    #[test]
    fn events_from_another_run_say_reset() {
        let rig = rig();
        rig.api.hub.set("a", json!(1));
        let b = body(
            &rig.api
                .handle(&request("GET", "/v1/events?after=1&boot=1", "")),
        );
        assert_eq!(b["reset"], true);
        let b = body(&rig.api.handle(&request("GET", "/v1/events?after=99", "")));
        assert_eq!(b["reset"], true);
    }

    #[test]
    fn bad_event_parameters_are_refused() {
        let rig = rig();
        for target in [
            "/v1/events?after=abc",
            "/v1/events?after=-1",
            "/v1/events?boot=x",
            "/v1/events?timeout=soon",
            "/v1/events?timeout=-5",
            "/v1/events?timeout=NaN",
            "/v1/events?topics=ok,Not%20Ok",
            "/v1/events?topics=a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q",
        ] {
            let r = rig.api.handle(&request("GET", target, ""));
            assert_eq!(r.status, 400, "{target}");
        }
    }

    #[test]
    fn a_long_poll_wakes_when_something_is_published() {
        let rig = rig();
        let hub = rig.api.hub.clone();
        let base = hub.snapshot().seq;
        let api = Arc::new(rig.api);
        let waiter = {
            let api = api.clone();
            std::thread::spawn(move || {
                body(&api.handle(&request(
                    "GET",
                    &format!("/v1/events?after={base}&timeout=20"),
                    "",
                )))
            })
        };
        std::thread::sleep(Duration::from_millis(60));
        hub.set("receiver", json!({"volumeDb": -30.5}));
        let b = waiter.join().unwrap();
        assert_eq!(b["events"][0]["data"], json!({"volumeDb": -30.5}));
    }

    #[test]
    fn receiver_settings_reads_the_topic_and_signals_interest() {
        let rig = rig();
        rig.api
            .hub
            .set("receiver", json!({"settings": {"ready": true}}));
        let expected = body(&rig.api.handle(&request("GET", "/v1/receiver", "")));
        let actual = body(&rig.api.handle(&request("GET", "/v1/receiver/settings", "")));
        for key in ["boot", "seq", "topic", "value", "ok", "api"] {
            assert_eq!(actual[key], expected[key], "{key}");
        }
        assert!(matches!(
            rig.receiver_rx.try_recv(),
            Ok(receiver::Cmd::SettingsInterest)
        ));
    }

    #[test]
    fn receiver_settings_rejects_nonlocal_and_malformed_requests() {
        let rig = rig();
        for path in ["/v1/receiver/settings", "/v1/receiver/set"] {
            let method = if path.ends_with("set") { "POST" } else { "GET" };
            let mut req = request(method, path, r#"{"setting":"x","value":1,"session":"s"}"#);
            req.peer = std::net::Ipv4Addr::new(192, 168, 1, 2).into();
            assert_eq!(rig.api.handle(&req).status, 403);
        }
        for input in [
            "",
            "null",
            "[]",
            r#"["x",1,"s"]"#,
            "{}",
            r#"{"setting":"x","session":"s"}"#,
            r#"{"value":1,"session":"s"}"#,
            r#"{"setting":"x","value":1}"#,
            r#"{"setting":1,"value":1,"session":"s"}"#,
            r#"{"setting":"x","value":1,"session":1}"#,
            r#"{"setting":"x","value":1,"session":"s","extra":true}"#,
            r#"{"setting":"x","setting":"y","value":1,"session":"s"}"#,
        ] {
            assert_eq!(
                rig.api
                    .handle(&request("POST", "/v1/receiver/set", input))
                    .status,
                400,
                "{input}"
            );
        }
        assert!(rig.receiver_rx.try_recv().is_err());
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/receiver/settings", ""))
                .status,
            405
        );
        assert_eq!(
            rig.api
                .handle(&request("GET", "/v1/receiver/set", ""))
                .status,
            405
        );
    }

    #[test]
    fn receiver_settings_maps_worker_responses_and_preserves_typed_values() {
        let cases = [
            (
                receiver::SettingsResponse::Accepted {
                    request_id: "request-1".into(),
                    session: "screen-1".into(),
                },
                202,
            ),
            (
                receiver::SettingsResponse::Invalid("invalid value".into()),
                400,
            ),
            (receiver::SettingsResponse::Busy, 409),
            (
                receiver::SettingsResponse::Unavailable("offline".into()),
                503,
            ),
            (receiver::SettingsResponse::WorkerError, 503),
        ];
        for (answer, status) in cases {
            let Rig {
                api,
                receiver_rx,
                _dir,
                ..
            } = rig();
            let worker = std::thread::spawn(move || match receiver_rx.recv().unwrap() {
                receiver::Cmd::SettingsSet {
                    setting,
                    value,
                    session,
                    reply,
                } => {
                    assert_eq!(setting, "test.setting");
                    assert_eq!(value, json!({"enabled": true, "level": 3}));
                    assert_eq!(session, "screen-1");
                    reply.send(answer).unwrap();
                }
                other => panic!("unexpected command: {other:?}"),
            });
            let response = api.handle(&request("POST", "/v1/receiver/set", r#"{"setting":"test.setting","value":{"enabled":true,"level":3},"session":"screen-1"}"#));
            assert_eq!(response.status, status);
            if status == 202 {
                let result = body(&response);
                assert_eq!(result["accepted"], true);
                assert_eq!(result["requestId"], "request-1");
                assert_eq!(result["session"], "screen-1");
            }
            worker.join().unwrap();
        }
    }

    #[test]
    fn receiver_settings_worker_disconnect_is_unavailable() {
        let Rig {
            api,
            receiver_rx,
            _dir,
            ..
        } = rig();
        drop(receiver_rx);
        let response = api.handle(&request(
            "POST",
            "/v1/receiver/set",
            r#"{"setting":"x","value":null,"session":"s"}"#,
        ));
        assert_eq!(response.status, 503);

        let Rig {
            api,
            receiver_rx,
            _dir,
            ..
        } = rig();
        let worker = std::thread::spawn(move || drop(receiver_rx.recv().unwrap()));
        let response = api.handle(&request(
            "POST",
            "/v1/receiver/set",
            r#"{"setting":"x","value":null,"session":"s"}"#,
        ));
        assert_eq!(response.status, 503);
        worker.join().unwrap();
    }

    #[test]
    fn receiver_settings_write_requires_owner() {
        let mut rig = rig();
        let paths = crate::config::Paths::new(rig._dir.path());
        let theme = paths.data_dir.join("theme");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("theme.json"), "{}").unwrap();
        std::fs::write(theme.join("layout.json"), r#"{"widgets":[]}"#).unwrap();
        let ctx = crate::ctx::Ctx {
            config: Arc::new(crate::config::Config::default()),
            paths,
            hub: rig.api.hub.clone(),
            settings: rig.api.settings.clone(),
            stop: crate::control::Stop::new(),
        };
        rig.api.profiles = crate::profiles::Profiles::open(&ctx);
        assert_eq!(
            rig.api
                .handle(&request(
                    "POST",
                    "/v1/profiles/switch",
                    r#"{"id":"isabel"}"#
                ))
                .status,
            200
        );
        let response = rig.api.handle(&request(
            "POST",
            "/v1/receiver/set",
            r#"{"setting":"x","value":1,"session":"s"}"#,
        ));
        assert_eq!(response.status, 403);
        assert_eq!(body(&response)["error"]["code"], "owner_only");
        assert!(rig.receiver_rx.try_recv().is_err());
    }

    #[test]
    fn receiver_settings_real_http_and_telnet_round_trip() {
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Instant;

        struct FakeReceiver {
            stop: Arc<AtomicBool>,
            disconnect: Arc<AtomicBool>,
            hold: Arc<AtomicBool>,
            mismatch: Arc<AtomicBool>,
            missing_prerequisites: Arc<AtomicBool>,
            writes: Arc<std::sync::Mutex<Vec<String>>>,
            threads: Vec<std::thread::JoinHandle<()>>,
        }
        impl Drop for FakeReceiver {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                for thread in self.threads.drain(..) {
                    thread.join().unwrap();
                }
            }
        }
        fn exchange(
            addr: std::net::SocketAddr,
            method: &str,
            path: &str,
            data: &str,
        ) -> (u16, Value) {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(stream, "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nX-Luma-Client: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}", data.len()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            let (headers, body) = response.split_once("\r\n\r\n").unwrap();
            let status = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            (status, serde_json::from_str(body).unwrap())
        }
        fn wait_for(mut f: impl FnMut() -> Option<Value>) -> Value {
            let end = Instant::now() + Duration::from_secs(12);
            while Instant::now() < end {
                if let Some(value) = f() {
                    return value;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            panic!("receiver settings integration timed out");
        }

        let web = TcpListener::bind("127.0.0.1:0").unwrap();
        let web_port = web.local_addr().unwrap().port();
        let telnet = TcpListener::bind("127.0.0.1:0").unwrap();
        let telnet_port = telnet.local_addr().unwrap().port();
        web.set_nonblocking(true).unwrap();
        telnet.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let disconnect = Arc::new(AtomicBool::new(false));
        let hold = Arc::new(AtomicBool::new(true));
        let mismatch = Arc::new(AtomicBool::new(false));
        let missing_prerequisites = Arc::new(AtomicBool::new(false));
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let web_thread = {
            let stop = stop.clone();
            let missing_prerequisites = missing_prerequisites.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match web.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_millis(200)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut chunk = [0; 2048];
                            loop {
                                let n = stream.read(&mut chunk).unwrap_or(0);
                                if n == 0 {
                                    break;
                                }
                                request.extend_from_slice(&chunk[..n]);
                                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n")
                                {
                                    let head = String::from_utf8_lossy(&request[..end]);
                                    let length = head
                                        .lines()
                                        .find_map(|line| {
                                            let (key, value) = line.split_once(':')?;
                                            key.eq_ignore_ascii_case("content-length")
                                                .then(|| value.trim().parse::<usize>().ok())
                                                .flatten()
                                        })
                                        .unwrap_or(0);
                                    if request.len() >= end + 4 + length {
                                        break;
                                    }
                                }
                            }
                            let request = String::from_utf8_lossy(&request);
                            let missing = missing_prerequisites.load(Ordering::SeqCst);
                            let (status, body) = if request
                                .starts_with("GET /goform/Deviceinfo.xml ")
                            {
                                if missing {
                                    (
                                        "200 OK",
                                        "<Device_Info><ModelName>AVR-S760H</ModelName></Device_Info>",
                                    )
                                } else {
                                    (
                                        "200 OK",
                                        concat!(
                                            "<Device_Info><ModelName>AVR-S760H</ModelName>",
                                            "<DeviceZoneCapabilities><Zone><No>0</No></Zone><InputSource><Control>1</Control><List>",
                                            "<Source><FuncName>TV AUDIO</FuncName><DefaultName>TV Audio</DefaultName></Source>",
                                            "<Source><FuncName>Blu-ray</FuncName><DefaultName>Blu-ray</DefaultName></Source>",
                                            "<Source><FuncName>CD</FuncName><DefaultName>CD</DefaultName></Source>",
                                            "</List></InputSource></DeviceZoneCapabilities>",
                                            "<DeviceZoneCapabilities><Zone><No>1</No></Zone><InputSource><Control>1</Control><List>",
                                            "<Source><FuncName>CD</FuncName><DefaultName>CD</DefaultName></Source>",
                                            "</List></InputSource></DeviceZoneCapabilities></Device_Info>"
                                        ),
                                    )
                                }
                            } else if missing {
                                ("404 Not Found", "")
                            } else if request.starts_with("POST /goform/AppCommand.xml ") {
                                for command in [
                                    "GetToneControl",
                                    "GetRenameSource",
                                    "GetDeletedSource",
                                    "GetAllZoneVolume",
                                ] {
                                    assert!(request.contains(command));
                                }
                                (
                                    "200 OK",
                                    concat!(
                                        "<rx><cmd><status>1</status><adjust>1</adjust></cmd>",
                                        "<cmd><functionrename><list><name>TV AUDIO</name><rename>Television</rename></list></functionrename></cmd>",
                                        "<cmd><functiondelete><list><FuncName>TV AUDIO</FuncName><use>1</use></list><list><FuncName>CD</FuncName><use>1</use></list><list><FuncName>Blu-ray</FuncName><use>0</use></list></functiondelete></cmd>",
                                        "<cmd><zone2><state>variable</state><limit>0.0</limit></zone2></cmd></rx>"
                                    ),
                                )
                            } else if request.starts_with("POST /goform/AppCommand0300.xml ") {
                                assert!(request.contains("GetAudyssey"));
                                (
                                    "200 OK",
                                    "<rx><cmd><list><param name=\"dynamiceq\" control=\"1\">0</param><param name=\"reflevoffset\" control=\"1\">0</param><param name=\"dynamicvol\" control=\"1\">0</param><param name=\"multeq\" control=\"1\">1</param></list></cmd></rx>",
                                )
                            } else {
                                ("404 Not Found", "")
                            };
                            let _ = write!(
                                stream,
                                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(e) => panic!("fake HTTP: {e}"),
                    }
                }
            })
        };
        let telnet_thread = {
            let (stop, disconnect, hold, mismatch, writes) = (
                stop.clone(),
                disconnect.clone(),
                hold.clone(),
                mismatch.clone(),
                writes.clone(),
            );
            std::thread::spawn(move || {
                let mut power = false;
                let mut direct = false;
                let mut bass_one = false;
                while !stop.load(Ordering::SeqCst) {
                    let (mut stream, _) = match telnet.accept() {
                        Ok(connection) => connection,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(e) => panic!("fake telnet: {e}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_millis(20)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut desired = None;
                    let mut awaiting = false;
                    let mut direct_requested = false;
                    let mut direct_readback = false;
                    let mut bass_requested = false;
                    let mut bass_readback = false;
                    while !stop.load(Ordering::SeqCst) && !disconnect.swap(false, Ordering::SeqCst)
                    {
                        let mut chunk = [0; 512];
                        match stream.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) => {}
                            Err(_) => break,
                        }
                        while let Some(end) = bytes.iter().position(|b| *b == b'\r') {
                            let line = String::from_utf8_lossy(&bytes[..end]).into_owned();
                            bytes.drain(..=end);
                            let reply = match line.as_str() {
                                "PW?" => Some("PWON\r"),
                                "MV?" => Some("MV45\r"),
                                "MU?" => Some("MUOFF\r"),
                                "SI?" => Some("SITV\r"),
                                "MS?" if direct_requested => {
                                    direct_readback = true;
                                    None
                                }
                                "MS?" => Some(if direct { "MSDIRECT\r" } else { "MSSTEREO\r" }),
                                "SSINFAISSIG ?" => Some("SYSDA PCM ZERO\r"),
                                "MSDIRECT" => {
                                    writes.lock().unwrap().push(line.clone());
                                    direct_requested = true;
                                    None
                                }
                                "PSTONE CTRL ?" => Some("PSTONE CTRL ON\r"),
                                "PSDYNEQ ?" => Some("PSDYNEQ OFF\r"),
                                "PSBAS ?" if bass_requested => {
                                    bass_readback = true;
                                    None
                                }
                                "PSBAS ?" => Some(if bass_one { "PSBAS51\r" } else { "PSBAS50\r" }),
                                "PSBAS51" => {
                                    writes.lock().unwrap().push(line.clone());
                                    bass_requested = true;
                                    None
                                }
                                "PSTRE ?" => Some("PSTRE50\r"),
                                "PSMULTEQ: ?" => Some("PSMULTEQ:AUDYSSEY\r"),
                                "PSDYNVOL ?" => Some("PSDYNVOL OFF\r"),
                                "PSREFLEV ?" => Some("PSREFLEV 0\r"),
                                "Z2ON" | "Z2OFF" => {
                                    writes.lock().unwrap().push(line.clone());
                                    desired = Some(line == "Z2ON");
                                    None
                                }
                                "Z2?" if desired.is_some() => {
                                    awaiting = true;
                                    None
                                }
                                "Z2?" => Some(if power {
                                    "Z2ON\rZ2CD\rZ235\r"
                                } else {
                                    "Z2OFF\rZ2CD\rZ235\r"
                                }),
                                _ => None,
                            };
                            if let Some(reply) = reply {
                                let _ = stream.write_all(reply.as_bytes());
                            }
                        }
                        if bass_readback && !hold.load(Ordering::SeqCst) {
                            bass_one = true;
                            let _ = stream.write_all(b"PSBAS51\r");
                            bass_requested = false;
                            bass_readback = false;
                        }
                        if direct_readback && !hold.load(Ordering::SeqCst) {
                            direct = true;
                            let _ = stream.write_all(b"MSDIRECT\r");
                            direct_requested = false;
                            direct_readback = false;
                        }
                        if awaiting && !hold.load(Ordering::SeqCst) {
                            if !mismatch.load(Ordering::SeqCst) {
                                power = desired.unwrap();
                            }
                            let _ = stream.write_all(if power {
                                b"Z2ON\rZ2CD\rZ235\r"
                            } else {
                                b"Z2OFF\rZ2CD\rZ235\r"
                            });
                            awaiting = false;
                            desired = None;
                        }
                    }
                    let _ = stream.shutdown(Shutdown::Both);
                }
            })
        };
        let fake = FakeReceiver {
            stop,
            disconnect,
            hold,
            mismatch,
            missing_prerequisites,
            writes,
            threads: vec![web_thread, telnet_thread],
        };
        let Rig {
            api,
            receiver_rx,
            _dir,
            ..
        } = rig();
        let changes = serde_json::from_value(json!([
            {"key":"host", "value":{"type":"text", "value":"127.0.0.1"}},
            {"key":"port", "value":{"type":"integer", "value":web_port}}
        ]))
        .unwrap();
        api.settings.apply("base.receiver", changes, None).unwrap();
        api.hub.set(
            "screen",
            json!({"on":true, "source":"assumed", "state":"Active"}),
        );
        let mut config = crate::config::Config::default();
        config.receiver.telnet_port = telnet_port;
        config.receiver.poll_ms = 2000;
        config.receiver.max_backoff_ms = 2000;
        config.receiver.connect_timeout_ms = 300;
        config.receiver.request_timeout_ms = 2000;
        let ctx = crate::ctx::Ctx {
            config: Arc::new(config),
            paths: crate::config::Paths::new(_dir.path()),
            hub: api.hub.clone(),
            settings: api.settings.clone(),
            stop: crate::control::Stop::new(),
        };
        let tx = api.receiver_tx.clone();
        let receiver = receiver::spawn(&ctx, tx.clone(), receiver_rx).unwrap();
        let server = crate::http::Server::start(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(api),
            Default::default(),
        )
        .unwrap();
        let addr = server.local_addr();
        let read = || exchange(addr, "GET", "/v1/receiver/settings", "").1;
        let initial = wait_for(|| {
            let value = read();
            (value["value"]["settings"]["zone2Power"]["writable"] == true).then_some(value)
        });
        let session = initial["value"]["settingsSession"]
            .as_str()
            .unwrap()
            .to_owned();
        let post = |value: bool, session: &str| {
            exchange(
                addr,
                "POST",
                "/v1/receiver/set",
                &json!({"setting":"zone2Power", "value":value, "session":session}).to_string(),
            )
        };
        let (status, accepted) = post(true, &session);
        assert_eq!(status, 202);
        let first_id = accepted["requestId"].as_str().unwrap().to_owned();
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["zone2Power"]["pending"]["requestId"] == first_id)
                .then_some(value)
        });
        fake.hold.store(false, Ordering::SeqCst);
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["zone2Power"]["result"]["requestId"] == first_id
                && value["value"]["settings"]["zone2Power"]["result"]["status"] == "applied"
                && value["value"]["settings"]["zone2Power"]["value"] == true)
                .then_some(value)
        });
        let controls = wait_for(|| {
            let value = read();
            let settings = &value["value"]["settings"];
            (settings["input"]["writable"] == true
                && settings["zone2Input"]["writable"] == true
                && settings["zone2Volume"]["writable"] == true
                && settings["bass"]["writable"] == true
                && settings["dynVol"]["writable"] == true)
                .then_some(value)
        });
        let settings = &controls["value"]["settings"];
        assert_eq!(
            settings["input"]["options"],
            json!([{"value":"TV","label":"Television"},{"value":"CD","label":"CD"}])
        );
        assert_eq!(
            settings["zone2Input"]["options"],
            json!([{"value":"CD","label":"CD"}])
        );
        assert_eq!(settings["zone2Volume"]["value"], 35.0);
        assert_eq!(settings["zone2Volume"]["max"], 80.0);
        let rejected = |setting: &str, value: Value| {
            exchange(
                addr,
                "POST",
                "/v1/receiver/set",
                &json!({"setting":setting,"value":value,"session":session}).to_string(),
            )
            .0
        };
        assert_eq!(rejected("input", json!("BD")), 400);
        assert_eq!(rejected("zone2Input", json!("TV")), 400);
        assert_eq!(rejected("zone2Volume", json!(81)), 400);
        assert_eq!(rejected("bass", json!(1.5)), 400);
        fake.hold.store(true, Ordering::SeqCst);
        let (status, accepted) = exchange(
            addr,
            "POST",
            "/v1/receiver/set",
            &json!({
                "setting":"bass", "value":1.0, "session":session
            })
            .to_string(),
        );
        assert_eq!(status, 202);
        let bass_id = accepted["requestId"].as_str().unwrap().to_owned();
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["bass"]["pending"]["requestId"] == bass_id).then_some(value)
        });
        fake.hold.store(false, Ordering::SeqCst);
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["bass"]["result"]["requestId"] == bass_id
                && value["value"]["settings"]["bass"]["result"]["status"] == "applied"
                && value["value"]["settings"]["bass"]["value"] == 1)
                .then_some(value)
        });
        let mode = wait_for(|| {
            let value = read();
            (value["value"]["settings"]["soundMode"]["writable"] == true).then_some(value)
        });
        assert_eq!(
            mode["value"]["settings"]["soundMode"]["options"],
            json!([
                {"value":"STEREO","label":"Stereo"}, {"value":"DIRECT","label":"Direct"}
            ])
        );
        assert_eq!(rejected("soundMode", json!("PURE DIRECT")), 400);
        fake.hold.store(true, Ordering::SeqCst);
        let (status, accepted) = exchange(
            addr,
            "POST",
            "/v1/receiver/set",
            &json!({
                "setting":"soundMode", "value":"DIRECT", "session":session
            })
            .to_string(),
        );
        assert_eq!(status, 202);
        let mode_id = accepted["requestId"].as_str().unwrap().to_owned();
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["soundMode"]["pending"]["requestId"] == mode_id)
                .then_some(value)
        });
        fake.hold.store(false, Ordering::SeqCst);
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["soundMode"]["result"]["requestId"] == mode_id
                && value["value"]["settings"]["soundMode"]["result"]["status"] == "applied"
                && value["value"]["settings"]["soundMode"]["value"] == "DIRECT")
                .then_some(value)
        });
        fake.mismatch.store(true, Ordering::SeqCst);
        let (status, accepted) = post(false, &session);
        assert_eq!(status, 202);
        let second_id = accepted["requestId"].as_str().unwrap().to_owned();
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["zone2Power"]["result"]["requestId"] == second_id
                && value["value"]["settings"]["zone2Power"]["result"]["status"] == "failed")
                .then_some(value)
        });
        fake.mismatch.store(false, Ordering::SeqCst);
        fake.hold.store(true, Ordering::SeqCst);
        let (status, accepted) = post(false, &session);
        assert_eq!(status, 202);
        let interrupted_id = accepted["requestId"].as_str().unwrap().to_owned();
        wait_for(|| (fake.writes.lock().unwrap().len() == 5).then_some(json!(true)));
        fake.missing_prerequisites.store(true, Ordering::SeqCst);
        fake.disconnect.store(true, Ordering::SeqCst);
        wait_for(|| {
            let value = read();
            (value["value"]["settings"]["zone2Power"]["result"]["requestId"] == interrupted_id
                && value["value"]["settings"]["zone2Power"]["result"]["status"] == "failed")
                .then_some(value)
        });
        wait_for(|| {
            let value = read();
            value["value"]["settingsSession"]
                .as_str()
                .filter(|new| *new != session)
                .map(|_| value.clone())
        });
        fake.hold.store(false, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            fake.writes.lock().unwrap().len(),
            5,
            "interrupted write was retried"
        );
        let unavailable = wait_for(|| {
            let value = read();
            let settings = &value["value"]["settings"];
            (settings["input"]["support"] == "supported"
                && settings["zone2Power"]["support"] == "supported")
                .then_some(value)
        });
        let settings = &unavailable["value"]["settings"];
        assert_eq!(settings["input"]["writable"], false);
        assert_eq!(settings["input"]["options"], json!([]));
        assert_eq!(settings["zone2Input"]["writable"], false);
        assert_eq!(settings["zone2Volume"]["writable"], false);
        let count = fake.writes.lock().unwrap().len();
        assert_eq!(post(false, &session).0, 503);
        assert_eq!(fake.writes.lock().unwrap().len(), count);
        assert_eq!(
            *fake.writes.lock().unwrap(),
            vec!["Z2ON", "PSBAS51", "MSDIRECT", "Z2OFF", "Z2OFF"]
        );
        server.stop(Duration::from_secs(2));
        tx.send(receiver::Cmd::Stop).unwrap();
        receiver.join().unwrap();
    }

    #[test]
    fn nudge_and_refresh_reach_their_threads() {
        let rig = rig();
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/receiver/nudge", ""))
                .status,
            200
        );
        assert!(matches!(
            rig.receiver_rx.try_recv(),
            Ok(receiver::Cmd::Nudge)
        ));
        assert_eq!(
            rig.api
                .handle(&request(
                    "POST",
                    "/v1/receiver/volume",
                    r#"{"level":46.74}"#
                ))
                .status,
            200
        );
        assert!(matches!(
            rig.receiver_rx.try_recv(),
            Ok(receiver::Cmd::Volume(93))
        ));
        for bad in [
            r#"{"level":99}"#,
            r#"{"level":-1}"#,
            r#"{"level":"loud"}"#,
            "{}",
        ] {
            assert_eq!(
                rig.api
                    .handle(&request("POST", "/v1/receiver/volume", bad))
                    .status,
                400,
                "{bad}"
            );
        }
        assert!(rig.receiver_rx.try_recv().is_err());
        assert_eq!(
            rig.api.handle(&request("POST", "/v1/refresh", "")).status,
            200
        );
        assert_eq!(rig.feeds_rx.try_recv(), Ok(homefeeds::Cmd::Refresh));
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/refresh", r#"{"what":["news"]}"#))
                .status,
            200
        );
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/refresh", r#"{"what":["tv"]}"#))
                .status,
            400
        );
        assert_eq!(
            rig.api
                .handle(&request("POST", "/v1/refresh", "[1]"))
                .status,
            400
        );
        assert_eq!(
            rig.api.handle(&request("POST", "/v1/refresh", "{")).status,
            400
        );
    }

    #[test]
    fn settings_are_listed_described_and_changed() {
        let rig = rig();
        let list = body(&rig.api.handle(&request("GET", "/v1/settings", "")));
        let owners: Vec<_> = list["owners"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["owner"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            owners,
            [
                "base.ambient",
                "base.logging",
                "base.news",
                "base.receiver",
                "base.voice",
                "base.weather",
                "scrypted-cameras"
            ]
        );

        let view = body(
            &rig.api
                .handle(&request("GET", "/v1/settings/base.weather", "")),
        );
        assert_eq!(view["schema"]["title"], "Weather");
        assert_eq!(view["effective"]["zip"]["value"], "30311");

        let change =
            r#"{"changes":[{"key":"zip","value":{"type":"text","value":"10001"}}],"ifVersion":0}"#;
        let r = rig
            .api
            .handle(&request("POST", "/v1/settings/base.weather", change));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let b = body(&r);
        assert_eq!(
            (b["changed"].clone(), b["version"].clone()),
            (json!(true), json!(1))
        );
        assert_eq!(b["settings"]["effective"]["zip"]["value"], "10001");

        // The same precondition again is stale.
        let r = rig
            .api
            .handle(&request("POST", "/v1/settings/base.weather", change));
        assert_eq!(r.status, 409);
        assert_eq!(body(&r)["error"]["version"], 1);
    }

    #[test]
    fn refused_settings_say_which_field_and_why() {
        let rig = rig();
        let bad_zip = r#"{"changes":[{"key":"zip","value":{"type":"text","value":"abcde"}}]}"#;
        let r = rig
            .api
            .handle(&request("POST", "/v1/settings/base.weather", bad_zip));
        assert_eq!(r.status, 422);
        let b = body(&r);
        assert_eq!(b["error"]["code"], "invalid");
        assert!(
            b["error"]["fields"]["zip"]
                .as_str()
                .unwrap()
                .contains("five digits")
        );
        let wrong_type =
            r#"{"changes":[{"key":"units","value":{"type":"choice","value":"kelvin"}}]}"#;
        let r = rig
            .api
            .handle(&request("POST", "/v1/settings/base.weather", wrong_type));
        assert_eq!(r.status, 422);
        assert!(body(&r)["error"]["fields"]["units"].is_string());
        for malformed in [
            "",
            "{}",
            r#"{"changes":[{"key":"zip"}]}"#,
            r#"{"changes":[],"extra":1}"#,
            "not json",
        ] {
            let r = rig
                .api
                .handle(&request("POST", "/v1/settings/base.weather", malformed));
            assert_eq!(r.status, 400, "{malformed}");
        }
        assert_eq!(
            rig.api
                .handle(&request("GET", "/v1/settings/base.nope", ""))
                .status,
            404
        );
        assert_eq!(
            rig.api
                .handle(&request(
                    "POST",
                    "/v1/settings/base.nope",
                    r#"{"changes":[]}"#
                ))
                .status,
            404
        );
    }
}
