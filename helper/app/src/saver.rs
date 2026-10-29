//! The screensaver's two helper jobs: its slots for anything that is not a plugin card, and what may show while it is up.
//!
//! **Complications.** A bridge from ntfy, a script, an app posts a *complication* (a glyph, a short label and value, a priority and an expiry;
//! the contract and its validation are `lumatv-saver`'s, `docs/research/saver-complications.md`) and the helper holds it on a board. A plugin on
//! the plugin API does not post: its cards are complications by the crate's rules, read by the saver from the `plugin.<id>.cards` topics.
//!
//! **Rules.** Everything that wants the screen has a *source class* (`lumatv_saver::rules`): the camera's video pop-up, what a camera saw, a
//! doorbell, a timer, Home Assistant, ntfy, a system notice, the board's items, and one class per installed plugin. The owner gives a class one
//! of four answers: `over`, `inside`, `wake`, `hide`. While the saver is up the helper applies the answer where a thing is *delivered*, so that
//! the QML shell and the Rust shell obey it alike: a camera pop-up that is hidden is never published, one that is `inside` becomes a dim line
//! in the topic's `lines`, `wake` asks the TV to turn the screen on. The shells hold what the helper cannot stop (a card of the notification
//! queue waits while the saver is up, as it always did) and the Rust saver reads `rules` from the topic to answer the rest itself.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/saver` | the topic's document: `{items, rules, lines, wake}` |
//! | `GET /v1/saver/rules` | `{answers: [{id, glyph, title}], classes: [{id, title, glyph, choice, default, choices}]}`: one row per class, one value per row |
//! | `POST /v1/saver/rules` | `{id, choice}` sets one class; `{id, reset: true}` puts one back; `{reset: true}` puts all back; the answer is the rows. The owner's |
//! | `GET /v1/saver/complications` | `{items: [...]}`: what is held, in the order it shows |
//! | `POST /v1/saver/complications` | `{id, source?, glyph?, text?, value?, priority?, expiresAt? or ttlS?, tone?}`: puts (or replaces) one; `source` is the caller's name (default `api`) |
//! | `DELETE /v1/saver/complications/{source}/{id}` | takes one away; `DELETE /v1/saver/complications/{source}` takes away everything the source said |
//!
//! The topic `saver`: `{items, rules: {class: answer}, lines: [{id, class, glyph, title, text, at, expiresAt}], wake: {seq, class, at}}`. `lines` are what
//! was kept out of the way for the saver's notification area; `wake.seq` counts the requests to wake the screen, for a shell that acts on them itself.
//!
//! Whether the saver is up: LG's power state says `Screen Saver`, or the compositor's `status` says the shell's saver is up (`oledcare.saver`), which is
//! asked only when something is about to be delivered. Nothing is polled. The complications are the helper's memory and a restarted helper starts
//! empty, which is right for things that are true now; the owner's answers are kept in `saver-rules.json` beside the other settings.

use crate::API_VERSION;
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::timeutil::now_ms;
use lumatv_saver::rules::{self, Choice, Delivery, Plugin, Rules};
use lumatv_saver::{Board, parse};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/saver", "what the screensaver reads: its complications, the rules of what shows while it is up, the lines kept for its notification area"),
    (
        "GET",
        "/v1/saver/rules",
        "what shows while the screensaver is up, by source class: one row per class with its answer (over, inside, wake or hide), its default and the answers it offers",
    ),
    (
        "POST",
        "/v1/saver/rules",
        "{id, choice} gives a class an answer; {id, reset: true} puts one back, {reset: true} all (the owner's)",
    ),
    ("GET", "/v1/saver/complications", "what the screensaver's slots hold, in the order they show"),
    (
        "POST",
        "/v1/saver/complications",
        "puts one complication on the screensaver: {id, source?, glyph?, text?, value?, priority?, ttlS? or expiresAt?, tone?}",
    ),
    (
        "DELETE",
        "/v1/saver/complications/{source}/{id}",
        "takes one complication away (without the id: everything the source said)",
    ),
];

/// A line for the saver's notification area is kept this long, and at most this many.
const LINE_MS: u64 = 30 * 60 * 1000;
const MAX_LINES: usize = 6;
/// How long the compositor's answer about the saver is believed.
const PROBE_MS: u64 = 1000;
const PROBE_WAIT: Duration = Duration::from_secs(1);
const MAX_ANSWER: u64 = 256 * 1024;
const WAKE_URI: &str = "luna://com.webos.service.tvpower/power/turnOnScreen";
const WAKE_WAIT: Duration = Duration::from_secs(4);
/// `saver-rules.json`.
const RULES_FILE: &str = "saver-rules.json";

#[derive(Clone, Debug, PartialEq)]
struct Line {
    id: String,
    class: String,
    glyph: String,
    title: String,
    text: String,
    at: u64,
    expires_at: u64,
}

impl Line {
    fn json(&self) -> Value {
        json!({"id": self.id, "class": self.class, "glyph": self.glyph, "title": self.title, "text": self.text, "at": self.at, "expiresAt": self.expires_at})
    }
}

#[derive(Default)]
struct Wake {
    seq: u64,
    class: String,
    at: u64,
}

#[derive(Default)]
struct Setup {
    /// Where the owner's answers are kept; none in a test.
    path: Option<PathBuf>,
    /// The compositor's control socket; none in a test.
    control: String,
    /// `luna-send`; none in a test.
    luna: String,
}

#[derive(Default)]
pub struct Saver {
    board: Mutex<Board>,
    rules: Mutex<Rules>,
    lines: Mutex<Vec<Line>>,
    wake: Mutex<Wake>,
    setup: Mutex<Setup>,
    /// What the compositor said of the saver, and when.
    probe: Mutex<Option<(u64, bool)>>,
}

static SAVER: LazyLock<Saver> = LazyLock::new(Saver::default);

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

/// A camera's event, as far as the rules care.
#[derive(Clone, Copy)]
pub struct Seen<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub label: &'a str,
    pub event: &'a str,
}

/// What the camera's event does with the pop-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraRoute {
    /// Whether the pop-up is published at all.
    pub popup: bool,
    /// Whether it may show over the saver (the owner's `over`): the Rust shell reads it, the QML shell holds the card until the saver ends.
    pub over_saver: bool,
}

/// The installed plugins as the rules' rows list them (`plugins` topic: the ones with a name, not the helper's own feeds).
fn roster(hub: &Hub) -> Vec<Plugin> {
    let Some(doc) = hub.get("plugins") else {
        return Vec::new();
    };
    doc.get("plugins")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .filter_map(|p| {
                    let id = p.get("id")?.as_str()?;
                    let title = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())?;
                    Some(Plugin { id: id.to_owned(), title: title.to_owned(), glyph: String::new() })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Saver {
    fn publish(&self, hub: &Hub, now: u64) {
        let mut doc = {
            let mut board = self.board.lock();
            board.expire(now);
            board.document(now)
        };
        let rules = self.rules.lock().effective(&roster(hub));
        let lines: Vec<Value> = {
            let mut lines = self.lines.lock();
            lines.retain(|l| l.expires_at > now);
            lines.iter().map(Line::json).collect()
        };
        let wake = {
            let w = self.wake.lock();
            json!({"seq": w.seq, "class": w.class, "at": w.at})
        };
        if let Some(map) = doc.as_object_mut() {
            map.insert("rules".into(), rules);
            map.insert("lines".into(), Value::Array(lines));
            map.insert("wake".into(), wake);
        }
        hub.set("saver", doc);
    }

    /// The rows of the rules for what is installed.
    fn rows(&self, hub: &Hub) -> Value {
        json!({"answers": Choice::answers(), "classes": self.rules.lock().rows(&roster(hub))})
    }

    fn save(&self) {
        let (path, doc) = (self.setup.lock().path.clone(), self.rules.lock().to_json());
        let Some(path) = path else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_vec_pretty(&doc).unwrap_or_default();
        if std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Whether the saver is up: LG's own (power state `Screen Saver`, on the `screen` topic) or the shell's (the compositor's status).
    fn up(&self, hub: &Hub, now: u64) -> bool {
        if hub.get("screen").is_some_and(|s| s.get("state").and_then(Value::as_str) == Some("Screen Saver")) {
            return true;
        }
        let control = self.setup.lock().control.clone();
        if control.is_empty() {
            return false;
        }
        let mut probe = self.probe.lock();
        if let Some((at, up)) = *probe
            && now.saturating_sub(at) < PROBE_MS
        {
            return up;
        }
        let up = ask_compositor(&control).unwrap_or(false);
        *probe = Some((now, up));
        up
    }

    /// Asks the TV to turn the screen on, once, and says so on the topic. Only called while the saver is up, so a standby TV is never woken.
    fn wake(&self, hub: &Hub, class: &str, now: u64) {
        {
            let mut w = self.wake.lock();
            w.seq += 1;
            w.class = class.to_owned();
            w.at = now;
        }
        let luna = self.setup.lock().luna.clone();
        if !luna.is_empty() {
            let _ = std::thread::Builder::new().name("saver-wake".to_owned()).stack_size(128 * 1024).spawn(move || turn_on_screen(&luna));
        }
        self.publish(hub, now);
    }

    fn add_line(&self, hub: &Hub, line: Line, now: u64) {
        {
            let mut lines = self.lines.lock();
            lines.retain(|l| l.id != line.id && l.expires_at > now);
            lines.insert(0, line);
            lines.truncate(MAX_LINES);
        }
        self.publish(hub, now);
    }

    /// What a camera event does while the saver may be up. The class of the pop-up is the doorbell's or the camera's video; a video that is hidden
    /// still leaves what was seen to the `camera.still` class (a dim line, or a wake). `up` is whether the saver is up.
    fn route_camera_at(&self, hub: &Hub, seen: Seen, up: bool, now: u64) -> CameraRoute {
        let Seen { kind, name, label, event } = seen;
        let (class, choice) = {
            let rules = self.rules.lock();
            let class = rules::camera_class(kind);
            (class, rules.choice(class))
        };
        let line = |class: &str| Line {
            id: format!("camera.{event}"),
            class: class.to_owned(),
            glyph: kind_glyph(kind).to_owned(),
            title: name.to_owned(),
            text: label.to_owned(),
            at: now,
            expires_at: now + LINE_MS,
        };
        match rules::delivery(choice, up) {
            Delivery::Normal => CameraRoute { popup: true, over_saver: false },
            Delivery::OverSaver => CameraRoute { popup: true, over_saver: true },
            Delivery::Wake => {
                self.wake(hub, class, now);
                CameraRoute { popup: true, over_saver: false }
            }
            Delivery::Inside => {
                self.add_line(hub, line(class), now);
                CameraRoute { popup: false, over_saver: false }
            }
            Delivery::Withheld => {
                // the video is kept out; what was seen is the camera's event, which has its own answer (a doorbell has none)
                if class == "camera.popup" {
                    // (the answer is read before the arms: a guard held across them would be locked again by `publish`)
                    let still = self.rules.lock().choice(rules::CAMERA_EVENT);
                    match rules::delivery(still, up) {
                        Delivery::Inside => self.add_line(hub, line(rules::CAMERA_EVENT), now),
                        Delivery::Wake => self.wake(hub, rules::CAMERA_EVENT, now),
                        _ => {}
                    }
                }
                CameraRoute { popup: false, over_saver: false }
            }
        }
    }

    /// A plugin's note was posted: when its class says `wake` and the saver is up, the screen is woken. The card itself is the shells' to show.
    fn note_at(&self, hub: &Hub, plugin: &str, event: &Value, up: impl FnOnce() -> bool, now: u64) {
        let post = event.get("post").unwrap_or(&Value::Null);
        let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_owned();
        let card = rules::card_class(&text(post.get("kind")), &text(event.get("channel")), &text(event.get("channelIcon")));
        let class = rules::note_class(plugin, card);
        // the saver is asked about only for a class that wakes: most notes never reach the compositor's socket
        if self.rules.lock().choice(&class) == Choice::Wake && up() {
            self.wake(hub, &class, now);
        }
    }

    pub fn handle_at(&self, hub: &Hub, request: &Request, rest: &[&str], now: u64) -> Response {
        match (request.method.as_str(), rest) {
            ("GET", []) => {
                self.publish(hub, now);
                ok(hub.get("saver").unwrap_or_else(|| json!({"items": []})))
            }
            ("GET", ["rules"]) => ok(self.rows(hub)),
            ("POST", ["rules"]) => {
                let body: Value = match serde_json::from_slice(&request.body) {
                    Ok(v) => v,
                    Err(e) => return error_response(400, "bad_request", &format!("the body is not JSON: {e}")),
                };
                let id = body.get("id").and_then(Value::as_str);
                if body.get("reset").and_then(Value::as_bool) == Some(true) {
                    self.rules.lock().reset(id);
                } else {
                    let (Some(id), Some(choice)) = (id, body.get("choice").and_then(Value::as_str)) else {
                        return error_response(400, "bad_request", "send {id, choice}, {id, reset: true} or {reset: true}");
                    };
                    if let Err(why) = self.rules.lock().set(id, choice) {
                        return error_response(400, why.code(), why.message());
                    }
                }
                self.save();
                self.publish(hub, now);
                ok(self.rows(hub))
            }
            ("GET", ["complications"]) => {
                let mut board = self.board.lock();
                board.expire(now);
                ok(board.document(now))
            }
            ("POST", ["complications"]) => {
                let body: Value = match serde_json::from_slice(&request.body) {
                    Ok(v) => v,
                    Err(e) => return error_response(400, "bad_request", &format!("the body is not JSON: {e}")),
                };
                let source = body.get("source").and_then(Value::as_str).unwrap_or("api").to_string();
                match parse(&body, &source, now) {
                    Ok(c) => {
                        let id = c.id.clone();
                        let lost = self.board.lock().put(c);
                        self.publish(hub, now);
                        ok(json!({"id": id, "source": source, "dropped": lost}))
                    }
                    Err(why) => error_response(400, "bad_complication", &why),
                }
            }
            ("DELETE", ["complications", source, id]) => {
                let gone = self.board.lock().remove(source, id);
                self.publish(hub, now);
                ok(json!({"removed": usize::from(gone)}))
            }
            ("DELETE", ["complications", source]) => {
                let gone = self.board.lock().remove_source(source);
                self.publish(hub, now);
                ok(json!({"removed": gone}))
            }
            (_, ["complications"] | ["complications", _] | ["complications", _, _]) => {
                error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "GET, POST, DELETE")
            }
            (_, [] | ["rules"]) => error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", if rest.is_empty() { "GET" } else { "GET, POST" }),
            _ => error_response(404, "not_found", "no such saver route"),
        }
    }
}

/// The glyph of what a camera saw.
fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "person" => "user",
        "vehicle" => "car",
        "animal" => "paw-print",
        "package" => "package",
        "doorbell" => "bell-ring",
        _ => "activity",
    }
}

/// `oledcare.saver` of the compositor's status: whether the shell's saver is up. None when the compositor does not answer.
fn ask_compositor(control: &str) -> Option<bool> {
    let mut stream = UnixStream::connect(control).ok()?;
    stream.set_write_timeout(Some(PROBE_WAIT)).ok()?;
    stream.set_read_timeout(Some(PROBE_WAIT)).ok()?;
    stream.write_all(b"status\n").ok()?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_ANSWER)).read_line(&mut line).ok()?;
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    Some(v.pointer("/oledcare/saver").and_then(Value::as_bool).unwrap_or(false))
}

/// `luna-send` of the power service's turnOnScreen; killed if it hangs. The answer is not read: the screen's state is the proof.
fn turn_on_screen(luna: &str) {
    let Ok(mut child) = Command::new(luna).args(["-n", "1", WAKE_URI, "{}"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return;
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if start.elapsed() < WAKE_WAIT => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

/// Serves `/v1/saver/...` on the helper's one saver.
pub fn handle(hub: &Hub, request: &Request, rest: &[&str]) -> Response {
    SAVER.handle_at(hub, request, rest, now_ms())
}

/// Starts the helper's saver: the owner's answers are read from the data directory and the topic is published. Called once, with the helper's context.
pub fn open(ctx: &Ctx) {
    let path = ctx.paths.data_dir.join(RULES_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        *SAVER.rules.lock() = serde_json::from_str(&text).map(|v| Rules::from_json(&v)).unwrap_or_default();
    }
    *SAVER.setup.lock() = Setup { path: Some(path), control: ctx.config.capture.control.clone(), luna: ctx.config.luna.send.clone() };
    SAVER.publish(&ctx.hub, now_ms());
}

/// What a camera event does with its pop-up under the owner's rules; called where the pop-up would be published.
pub fn route_camera(hub: &Hub, seen: Seen) -> CameraRoute {
    let now = now_ms();
    let up = SAVER.up(hub, now);
    SAVER.route_camera_at(hub, seen, up, now)
}

/// A plugin's note was posted (the `posted` event of `plugin.<id>.notifications`): applies its class's answer where the helper can (`wake`).
pub fn note_posted(hub: &Hub, plugin: &str, event: &Value) {
    let now = now_ms();
    SAVER.note_at(hub, plugin, event, || SAVER.up(hub, now), now);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
        }
    }

    fn rest(path: &str) -> Vec<String> {
        path.trim_start_matches("/v1/saver").trim_start_matches('/').split('/').filter(|s| !s.is_empty()).map(str::to_string).collect()
    }

    fn call(s: &Saver, hub: &Hub, method: &str, path: &str, body: &str, now: u64) -> (u16, Value) {
        let segs = rest(path);
        let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
        let r = s.handle_at(hub, &request(method, path, body), &refs, now);
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    const NOW: u64 = 1_800_000_000_000;

    #[test]
    fn a_post_is_held_published_on_the_topic_and_taken_away_again() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let (status, a) = call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "door", "source": "ntfy", "glyph": "bell-ring", "text": "Front door", "value": "Open", "priority": 1, "ttlS": 600}"#, NOW);
        assert_eq!(status, 200, "{a}");
        assert_eq!((a["source"].as_str(), a["dropped"].as_u64()), (Some("ntfy"), Some(0)));
        let topic = hub.topic_at("saver").1.expect("the topic is published");
        assert_eq!(topic["items"][0]["text"], "Front door");
        assert_eq!(topic["items"][0]["expiresAt"], NOW + 600_000);
        assert_eq!(topic["rules"]["doorbell"], "wake", "the topic carries the rules");
        let (_, got) = call(&s, &hub, "GET", "/v1/saver/complications", "", NOW + 1);
        assert_eq!(got["items"].as_array().unwrap().len(), 1);
        let (_, gone) = call(&s, &hub, "DELETE", "/v1/saver/complications/ntfy/door", "", NOW + 2);
        assert_eq!(gone["removed"], 1);
        assert_eq!(hub.topic_at("saver").1.unwrap()["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_source_with_no_name_is_api_a_bad_one_is_refused_with_its_words_and_a_source_can_be_cleared_at_once() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let (_, a) = call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "x", "text": "Hi"}"#, NOW);
        assert_eq!(a["source"], "api");
        let (status, bad) = call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "y"}"#, NOW);
        assert_eq!(status, 400);
        assert_eq!(bad["error"]["code"], "bad_complication");
        assert!(bad["error"]["message"].as_str().unwrap().contains("glyph, a text or a value"));
        assert_eq!(call(&s, &hub, "POST", "/v1/saver/complications", "not json", NOW).0, 400);
        for i in 0..3 {
            call(&s, &hub, "POST", "/v1/saver/complications", &format!(r#"{{"id": "i{i}", "source": "bridge", "text": "n{i}"}}"#), NOW);
        }
        let (_, cleared) = call(&s, &hub, "DELETE", "/v1/saver/complications/bridge", "", NOW);
        assert_eq!(cleared["removed"], 3);
        assert_eq!(call(&s, &hub, "PUT", "/v1/saver/complications", "", NOW).0, 405);
        assert_eq!(call(&s, &hub, "GET", "/v1/saver/other", "", NOW).0, 404);
    }

    #[test]
    fn what_has_expired_is_not_served() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "short", "text": "Short", "ttlS": 5}"#, NOW);
        let (_, got) = call(&s, &hub, "GET", "/v1/saver/complications", "", NOW + 6000);
        assert_eq!(got["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_rules_are_one_row_per_class_with_one_value_and_a_plugin_that_is_installed_has_its_row() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        hub.set("plugins", json!({"plugins": [{"id": "photos", "name": "Photos", "state": "connected"}, {"id": "off", "state": "inert"}], "firstParty": [{"id": "weather", "name": "Weather"}]}));
        let (status, v) = call(&s, &hub, "GET", "/v1/saver/rules", "", NOW);
        assert_eq!(status, 200);
        assert_eq!(v["answers"].as_array().unwrap().len(), 4);
        assert_eq!(v["answers"][0], json!({"id": "over", "glyph": "picture-in-picture", "title": "Over"}));
        let rows = v["classes"].as_array().unwrap();
        let find = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no row {id}"));
        assert_eq!(find("camera.popup")["choice"], "hide");
        assert_eq!(find("doorbell")["choice"], "wake");
        assert_eq!(find("plugin:photos")["title"], "Photos", "a new plugin appears by itself");
        assert!(rows.iter().all(|r| r["id"] != "plugin:off" && r["id"] != "plugin:weather"), "a plugin with no name and the helper's own feeds have no row");
    }

    #[test]
    fn a_rule_is_set_reset_and_refused_with_its_reason_and_the_topic_follows() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let (status, v) = call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "ha", "choice": "wake"}"#, NOW);
        assert_eq!(status, 200, "{v}");
        assert_eq!(v["classes"].as_array().unwrap().iter().find(|r| r["id"] == "ha").unwrap()["choice"], "wake");
        assert_eq!(hub.topic_at("saver").1.unwrap()["rules"]["ha"], "wake");
        let (status, bad) = call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "camera.popup", "choice": "inside"}"#, NOW);
        assert_eq!((status, bad["error"]["code"].as_str()), (400, Some("not_offered")));
        assert_eq!(call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "nope", "choice": "hide"}"#, NOW).1["error"]["code"], "unknown_class");
        assert_eq!(call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "ha", "choice": "loud"}"#, NOW).1["error"]["code"], "unknown_choice");
        assert_eq!(call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "ha"}"#, NOW).0, 400);
        call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "ntfy", "choice": "hide"}"#, NOW);
        let (_, v) = call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "ha", "reset": true}"#, NOW);
        let pick = |v: &Value, id: &str| v["classes"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap()["choice"].clone();
        assert_eq!((pick(&v, "ha"), pick(&v, "ntfy")), (json!("inside"), json!("hide")));
        let (_, v) = call(&s, &hub, "POST", "/v1/saver/rules", r#"{"reset": true}"#, NOW);
        assert_eq!(pick(&v, "ntfy"), json!("inside"));
        assert_eq!(call(&s, &hub, "DELETE", "/v1/saver/rules", "", NOW).0, 405);
    }

    #[test]
    fn the_owners_answers_are_kept_in_a_file_and_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RULES_FILE);
        let (s, hub) = (Saver::default(), Hub::new(1));
        s.setup.lock().path = Some(path.clone());
        call(&s, &hub, "POST", "/v1/saver/rules", r#"{"id": "timer", "choice": "over"}"#, NOW);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(Rules::from_json(&serde_json::from_str(&text).unwrap()).choice("timer"), Choice::Over);
        assert!(!dir.path().join("saver-rules.json.tmp").exists());
    }

    fn cam(s: &Saver, hub: &Hub, kind: &str, up: bool) -> CameraRoute {
        s.route_camera_at(hub, Seen { kind, name: "Driveway", label: "Person", event: "e1" }, up, NOW)
    }

    #[test]
    fn with_the_saver_down_a_camera_event_pops_up_as_it_always_did() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        assert_eq!(cam(&s, &hub, "person", false), CameraRoute { popup: true, over_saver: false });
        assert_eq!(cam(&s, &hub, "doorbell", false), CameraRoute { popup: true, over_saver: false });
        assert_eq!(s.wake.lock().seq, 0, "nothing is woken for a screen that is on");
    }

    #[test]
    fn with_the_saver_up_a_video_is_hidden_its_event_is_a_dim_line_and_a_doorbell_wakes_the_screen() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let r = cam(&s, &hub, "person", true);
        assert!(!r.popup, "the video does not cover the saver");
        let doc = hub.topic_at("saver").1.unwrap();
        assert_eq!(doc["lines"][0]["title"], "Driveway");
        assert_eq!(doc["lines"][0]["glyph"], "user");
        assert_eq!(doc["lines"][0]["class"], "camera.still");
        assert_eq!(doc["wake"]["seq"], 0);
        let d = s.route_camera_at(&hub, Seen { kind: "doorbell", name: "Front", label: "Doorbell", event: "e2" }, true, NOW + 5);
        assert!(d.popup, "a doorbell is published, and the screen is woken for it");
        let doc = hub.topic_at("saver").1.unwrap();
        assert_eq!((doc["wake"]["seq"].as_u64(), doc["wake"]["class"].as_str()), (Some(1), Some("doorbell")));
    }

    #[test]
    fn the_owner_can_let_the_video_show_over_the_saver_or_wake_for_it_or_hide_the_events_too() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        s.rules.lock().set("camera.popup", "over").unwrap();
        assert_eq!(cam(&s, &hub, "person", true), CameraRoute { popup: true, over_saver: true });
        s.rules.lock().set("camera.popup", "wake").unwrap();
        assert!(cam(&s, &hub, "person", true).popup);
        assert_eq!(s.wake.lock().seq, 1);
        s.rules.lock().set("camera.popup", "hide").unwrap();
        s.rules.lock().set("camera.still", "hide").unwrap();
        let before = hub.topic_at("saver").1.unwrap()["lines"].as_array().unwrap().len();
        assert!(!s.route_camera_at(&hub, Seen { kind: "vehicle", name: "Drive", label: "Vehicle", event: "e9" }, true, NOW + 9).popup);
        assert_eq!(hub.topic_at("saver").1.unwrap()["lines"].as_array().unwrap().len(), before, "nothing is kept for the saver");
        s.rules.lock().set("camera.still", "wake").unwrap();
        s.route_camera_at(&hub, Seen { kind: "vehicle", name: "Drive", label: "Vehicle", event: "e10" }, true, NOW + 10);
        assert_eq!(s.wake.lock().seq, 2, "a camera event may wake the screen though its video is hidden");
        s.rules.lock().set("doorbell", "hide").unwrap();
        s.rules.lock().set("camera.still", "inside").unwrap();
        let lines = hub.topic_at("saver").1.unwrap()["lines"].as_array().unwrap().len();
        assert!(!s.route_camera_at(&hub, Seen { kind: "doorbell", name: "Front", label: "Doorbell", event: "e11" }, true, NOW + 11).popup);
        assert_eq!(hub.topic_at("saver").1.unwrap()["lines"].as_array().unwrap().len(), lines, "a hidden doorbell is not an event line");
    }

    #[test]
    fn a_line_is_replaced_by_its_event_cut_to_six_and_expires() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        for i in 0..9 {
            s.route_camera_at(&hub, Seen { kind: "person", name: "Cam", label: "Person", event: &format!("e{i}") }, true, NOW + i);
        }
        assert_eq!(hub.topic_at("saver").1.unwrap()["lines"].as_array().unwrap().len(), MAX_LINES);
        s.route_camera_at(&hub, Seen { kind: "person", name: "Cam", label: "Person again", event: "e8" }, true, NOW + 20);
        let lines = hub.topic_at("saver").1.unwrap()["lines"].clone();
        assert_eq!(lines[0]["text"], "Person again");
        assert_eq!(lines.as_array().unwrap().iter().filter(|l| l["id"] == "camera.e8").count(), 1);
        s.publish(&hub, NOW + 20 + LINE_MS);
        assert_eq!(hub.topic_at("saver").1.unwrap()["lines"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_plugins_note_wakes_the_screen_only_when_its_class_says_so_and_the_saver_is_up() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let door = json!({"id": "a", "channel": "doorbell", "channelIcon": "base.icons/doorbell", "post": {"title": "Ding"}});
        s.note_at(&hub, "home-assistant", &door, || false, NOW);
        assert_eq!(s.wake.lock().seq, 0);
        s.note_at(&hub, "home-assistant", &door, || true, NOW);
        assert_eq!(s.wake.lock().seq, 1, "a doorbell wakes");
        let low = json!({"id": "b", "channel": "alerts", "channelIcon": "base.icons/home", "post": {"title": "Low battery"}});
        s.note_at(&hub, "home-assistant", &low, || true, NOW);
        assert_eq!(s.wake.lock().seq, 1, "Home Assistant's notices stay in the saver by default");
        s.rules.lock().set("ha", "wake").unwrap();
        s.note_at(&hub, "home-assistant", &low, || true, NOW);
        assert_eq!(s.wake.lock().seq, 2);
    }

    #[test]
    fn the_saver_is_up_when_lg_says_screen_saver() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        assert!(!s.up(&hub, NOW));
        hub.set("screen", json!({"on": true, "state": "Active"}));
        assert!(!s.up(&hub, NOW));
        hub.set("screen", json!({"on": true, "state": "Screen Saver"}));
        assert!(s.up(&hub, NOW));
    }
}
