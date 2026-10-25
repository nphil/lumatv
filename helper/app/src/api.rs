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
//! | `POST /v1/refresh` | fetch weather and news now |
//! | `GET /v1/settings`, `GET /v1/settings/{owner}`, `POST /v1/settings/{owner}` | the settings layer |
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
/// A reading stamped further than this into the future was written with a wrong clock.
const FUTURE_SKEW_MS: u64 = 300_000;

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
    ("GET", "/v1/weather", "the weather topic"),
    ("GET", "/v1/news", "the news topic"),
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
    pub receiver_tx: Sender<receiver::Cmd>,
    pub feeds_tx: Sender<homefeeds::Cmd>,
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
fn freshness_word(doc: &Value, now: u64) -> (&'static str, Option<u64>) {
    let (Some(fetched), Some(stale), Some(expires)) = (
        doc.get("fetchedAt").and_then(Value::as_u64),
        doc.get("staleAt").and_then(Value::as_u64),
        doc.get("expiresAt").and_then(Value::as_u64),
    ) else {
        return ("none", None);
    };
    let age = now.saturating_sub(fetched);
    let word = if fetched > now.saturating_add(FUTURE_SKEW_MS) || now >= expires {
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
        let Some(value) = value else {
            return error_response(
                404,
                "no_such_topic",
                "nothing has been published under that topic",
            );
        };
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
            ["v1", "receiver", "nudge"] if post => {
                let _ = self.receiver_tx.send(receiver::Cmd::Nudge);
                self.ok(json!({"requested": true}))
            }
            ["v1", "refresh"] if post => self.refresh(request),
            ["v1", "settings"] if get => self.settings_list(),
            ["v1", "settings", owner] if get => self.settings_get(owner),
            ["v1", "settings", owner] if post => self.settings_post(owner, request),
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
            ]
            | ["v1", "state" | "settings", _]
            | ["v1", "receiver", "nudge"]
    )
}

fn allowed_methods(segments: &[&str]) -> &'static str {
    match segments {
        ["v1", "refresh"] | ["v1", "receiver", "nudge"] => "POST",
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

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(777));
        let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
        settings.register_builtin().unwrap();
        let (receiver_tx, receiver_rx) = mpsc::channel();
        let (feeds_tx, feeds_rx) = mpsc::channel();
        Rig {
            api: Api {
                hub,
                settings,
                started_at: 1000,
                data_dir: "/var/lib/test".into(),
                fetch_backend: json!({"backend": "test"}),
                receiver_tx,
                feeds_tx,
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
        assert_eq!(listed.len(), ROUTES.len());
        assert_eq!(b["helper"]["version"], crate::VERSION);
        assert_eq!(b["boot"], 777);
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
    fn state_carries_the_seq_and_every_topic() {
        let rig = rig();
        rig.api.hub.set("screen", json!({"on": true}));
        rig.api.hub.set("receiver", json!({"fresh": false}));
        let b = body(&rig.api.handle(&request("GET", "/v1/state", "")));
        assert_eq!(b["seq"], 2);
        assert_eq!(b["boot"], 777);
        assert_eq!(b["topics"]["screen"], json!({"on": true}));
        assert_eq!(b["helper"]["name"], "lumatv-helper");
        assert_eq!(b["helper"]["startedAt"], 1000);
        assert!(b["helper"]["rssKb"].as_u64().unwrap() > 0);
    }

    #[test]
    fn one_topic_and_the_aliases() {
        let rig = rig();
        rig.api.hub.set("screen", json!({"on": false}));
        for path in ["/v1/screen", "/v1/state/screen"] {
            let b = body(&rig.api.handle(&request("GET", path, "")));
            assert_eq!(b["topic"], "screen");
            assert_eq!(b["value"], json!({"on": false}));
            assert_eq!(b["seq"], 1);
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
        let future = decorate(
            "weather",
            &weather_doc(now + 400_000, now + 1_000_000, now + 22_000_000),
            now,
        );
        assert_eq!(future["state"], "expired");
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
        rig.api.hub.set("a", json!(1));
        let b = body(&rig.api.handle(&request("GET", "/v1/events?timeout=20", "")));
        assert_eq!(b["seq"], 1);
        assert_eq!(b["events"], json!([]));
        assert_eq!(b["reset"], false);
    }

    #[test]
    fn events_after_a_seq_are_returned_and_filtered() {
        let rig = rig();
        rig.api.hub.set("a", json!(1));
        rig.api.hub.set("b", json!(2));
        let b = body(
            &rig.api
                .handle(&request("GET", "/v1/events?after=0&boot=777&topics=b", "")),
        );
        assert_eq!(b["events"].as_array().unwrap().len(), 1);
        assert_eq!(b["events"][0]["topic"], "b");
        assert_eq!(b["events"][0]["kind"], "changed");
        assert_eq!(b["events"][0]["data"], json!(2));
        assert_eq!(b["seq"], 2);
        let again = body(&rig.api.handle(&request(
            "GET",
            "/v1/events?after=2&boot=777&timeout=0",
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
        let api = Arc::new(rig.api);
        let waiter = {
            let api = api.clone();
            std::thread::spawn(move || {
                body(&api.handle(&request("GET", "/v1/events?after=0&timeout=20", "")))
            })
        };
        std::thread::sleep(Duration::from_millis(60));
        hub.set("receiver", json!({"volumeDb": -30.5}));
        let b = waiter.join().unwrap();
        assert_eq!(b["events"][0]["data"], json!({"volumeDb": -30.5}));
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
        assert_eq!(rig.receiver_rx.try_recv(), Ok(receiver::Cmd::Nudge));
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
        assert_eq!(owners, ["base.news", "base.receiver", "base.weather"]);

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
