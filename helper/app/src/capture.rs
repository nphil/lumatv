//! Screenshots and screen recordings: the helper's door to what LumaTV's compositor can do (`compositor/.../screencap`, which has the
//! design and the rules of it).
//!
//! The compositor composes the picture, so it takes the capture; this module is the switch and the way in. `capture.enabled` in
//! `helper.json` is off by default and decides here, before the compositor is asked anything; the compositor reads the same switch itself.
//! Every answer is the compositor's, relayed: nothing is captured, kept or decided here. Nothing runs without a command, and a recording
//! also needs the shell's dot on the screen (the compositor will not start or go on without it), so a caller on the TV cannot record in secret.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/capture` | `{capture: {enabled, available, ...}}`: while off only `enabled: false`; else the compositor's own status (recording, encoder, folder, last capture) |
//! | `POST /v1/capture/screenshot` | `{name, path, bytes, width, height, ms}`: a PNG in `captures/` of the picture as composed; video is black |
//! | `POST /v1/capture/record/start` | `{recording: true, name, width, height}` once the encoder is open and the dot is up |
//! | `POST /v1/capture/record/stop` | `{name, path, bytes, seconds, samples}` once the file is whole |
//!
//! Failures carry the compositor's reason: 403 `capture_disabled`, 409 `no_indicator` (the shell is not there to say it), `busy`,
//! `not_recording`, 501 `no_encoder` (recording is not set up), 503 `compositor_unavailable` (LG's compositor has the screen), 507
//! `disk_full`, 504 `timeout`, 500 `capture_failed` (the message says what). The shell's QML reads a body only on 200, 202 and 409 (`kit/Helper.qml`),
//! and the workstation host's network stack (Qt 5.15; the set's 5.12 is not measured) reports a 409 as no answer at all, status 0 and no body: so
//! the other statuses are chosen to be told apart by the status alone, and the 409 group is told from the state read afterwards
//! (`settings/Capture.js`).

use crate::API_VERSION;
use crate::config::CaptureConfig;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/capture",
        "whether screen capture is on and, when it is, what the compositor says: recording or not, the encoder, the captures folder, the last capture",
    ),
    (
        "POST",
        "/v1/capture/screenshot",
        "a PNG of the picture the compositor composed, saved in the captures folder (video is black); 403 while capture.enabled is off",
    ),
    (
        "POST",
        "/v1/capture/record/start",
        "starts a screen recording (H.264 in an MP4, through the hardware encoder); the shell shows a red dot while it runs",
    ),
    (
        "POST",
        "/v1/capture/record/stop",
        "ends the recording and answers with its file",
    ),
];

/// The longest a screenshot, a recording's start and its stop may take at the compositor (it waits for a frame, the encoder and the dot,
/// the file).
const SHOT_WAIT: Duration = Duration::from_secs(20);
const START_WAIT: Duration = Duration::from_secs(16);
const STOP_WAIT: Duration = Duration::from_secs(18);
const STATUS_WAIT: Duration = Duration::from_secs(4);
/// The most of an answer that is read: a status is a few kilobytes.
const MAX_ANSWER: u64 = 256 * 1024;

pub struct Capture {
    enabled: bool,
    control: String,
}

enum Asked {
    Answer(Value),
    /// The compositor did not answer: the socket is not there, refused, or said nothing in time or nonsense.
    Down(String),
}

impl Capture {
    pub fn new(config: &CaptureConfig) -> Capture {
        Capture {
            enabled: config.enabled,
            control: config.control.clone(),
        }
    }

    fn ask(&self, command: &str, within: Duration) -> Asked {
        let mut stream = match UnixStream::connect(&self.control) {
            Ok(s) => s,
            Err(e) => return Asked::Down(format!("{}: {e}", self.control)),
        };
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        // The compositor's own deadlines are inside this one; this is only for a compositor that hangs.
        let _ = stream.set_read_timeout(Some(within));
        if let Err(e) = stream.write_all(format!("{command}\n").as_bytes()) {
            return Asked::Down(format!("the compositor's control socket: {e}"));
        }
        let mut line = String::new();
        if let Err(e) = BufReader::new(stream.take(MAX_ANSWER)).read_line(&mut line) {
            return Asked::Down(format!("no answer from the compositor: {e}"));
        }
        match serde_json::from_str::<Value>(line.trim()) {
            Ok(v) => Asked::Answer(v),
            Err(_) => {
                Asked::Down("the compositor answered with something that is not JSON".to_owned())
            }
        }
    }

    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        match (request.method.as_str(), rest) {
            ("GET", []) => self.status(),
            ("POST", ["screenshot"]) => self.forward("screencap shot", SHOT_WAIT),
            ("POST", ["record", "start"]) => self.forward("screencap record start", START_WAIT),
            ("POST", ["record", "stop"]) => self.forward("screencap record stop", STOP_WAIT),
            (_, [] | ["screenshot"] | ["record", "start" | "stop"]) => {
                error_response(405, "method_not_allowed", "that method is not served here")
                    .with_header("Allow", if rest.is_empty() { "GET" } else { "POST" })
            }
            _ => error_response(404, "not_found", "no such endpoint"),
        }
    }

    fn status(&self) -> Response {
        if !self.enabled {
            return ok(json!({"capture": {"enabled": false, "available": false}}));
        }
        match self.ask("screencap status", STATUS_WAIT) {
            Asked::Answer(mut v) => {
                if let Some(map) = v.as_object_mut() {
                    map.remove("ok");
                    map.insert("available".into(), json!(true));
                }
                ok(json!({"capture": v}))
            }
            Asked::Down(why) => {
                ok(json!({"capture": {"enabled": true, "available": false, "reason": why}}))
            }
        }
    }

    fn forward(&self, command: &str, within: Duration) -> Response {
        if !self.enabled {
            return error_response(
                403,
                "capture_disabled",
                "screen capture is off (capture.enabled in helper.json)",
            );
        }
        match self.ask(command, within) {
            Asked::Answer(v) => relay(v),
            Asked::Down(why) => error_response(503, "compositor_unavailable", &why),
        }
    }
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

/// The compositor's answer as the helper's: its fields on success, its code and words on failure.
fn relay(v: Value) -> Response {
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        return ok(v);
    }
    let (status, code) = match v.get("code").and_then(Value::as_str).unwrap_or("") {
        "disabled" => (403, "capture_disabled"),
        "no-indicator" => (409, "no_indicator"),
        "busy" => (409, "busy"),
        "not-recording" => (409, "not_recording"),
        "no-encoder" => (501, "no_encoder"),
        "disk-full" => (507, "disk_full"),
        "timeout" => (504, "timeout"),
        _ => (500, "capture_failed"),
    };
    error_response(
        status,
        code,
        v.get("error")
            .and_then(Value::as_str)
            .unwrap_or("the capture failed"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn request(method: &str, path: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: Vec::new(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn body(r: &Response) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lumatv-capture-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A compositor that answers each line it is sent with the next of `answers`, and counts the lines. Returns the lines it saw.
    fn fake(
        path: &std::path::Path,
        answers: Vec<&'static str>,
    ) -> (Arc<AtomicUsize>, std::thread::JoinHandle<Vec<String>>) {
        let listener = UnixListener::bind(path).unwrap();
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        let t = std::thread::spawn(move || {
            let mut lines = Vec::new();
            for answer in answers {
                let (mut s, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(s.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                lines.push(line.trim().to_string());
                counter.fetch_add(1, Ordering::SeqCst);
                s.write_all(format!("{answer}\n").as_bytes()).unwrap();
            }
            lines
        });
        (seen, t)
    }

    fn capture(enabled: bool, control: &std::path::Path) -> Capture {
        Capture::new(&CaptureConfig {
            enabled,
            control: control.display().to_string(),
            ..CaptureConfig::default()
        })
    }

    #[test]
    fn off_it_refuses_before_the_compositor_is_asked_anything() {
        let d = dir("off");
        let sock = d.join("control");
        let (seen, t) = fake(&sock, vec![]);
        let c = capture(false, &sock);
        for (method, path, rest) in [
            ("POST", "/v1/capture/screenshot", vec!["screenshot"]),
            ("POST", "/v1/capture/record/start", vec!["record", "start"]),
            ("POST", "/v1/capture/record/stop", vec!["record", "stop"]),
        ] {
            let r = c.handle(&request(method, path), &rest);
            assert_eq!(r.status, 403, "{path}");
            assert_eq!(body(&r)["error"]["code"], "capture_disabled");
        }
        let r = c.handle(&request("GET", "/v1/capture"), &[]);
        assert_eq!(
            (r.status, body(&r)["capture"].clone()),
            (200, json!({"enabled": false, "available": false}))
        );
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        t.join().unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_screenshot_is_the_compositors_answer_relayed() {
        let d = dir("shot");
        let sock = d.join("control");
        let (_, t) = fake(
            &sock,
            vec![
                r#"{"ok":true,"name":"shot-20261028-070000.png","path":"/var/lib/lumatv/captures/shot-20261028-070000.png","bytes":812345,"width":1920,"height":1080,"ms":310}"#,
            ],
        );
        let r = capture(true, &sock)
            .handle(&request("POST", "/v1/capture/screenshot"), &["screenshot"]);
        assert_eq!(r.status, 200);
        let b = body(&r);
        assert_eq!(
            (
                b["ok"].clone(),
                b["api"].clone(),
                b["name"].clone(),
                b["bytes"].clone()
            ),
            (
                json!(true),
                json!(1),
                json!("shot-20261028-070000.png"),
                json!(812345)
            )
        );
        assert!(b["now"].as_u64().unwrap() > 0);
        assert_eq!(t.join().unwrap(), ["screencap shot"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_recording_starts_and_stops_through_the_same_door() {
        let d = dir("rec");
        let sock = d.join("control");
        let (_, t) = fake(
            &sock,
            vec![
                r#"{"ok":true,"recording":true,"name":"rec-1.mp4","width":1280,"height":720}"#,
                r#"{"ok":true,"recording":false,"name":"rec-1.mp4","path":"/x/rec-1.mp4","bytes":99,"seconds":4.2,"samples":120}"#,
            ],
        );
        let c = capture(true, &sock);
        let start = c.handle(
            &request("POST", "/v1/capture/record/start"),
            &["record", "start"],
        );
        assert_eq!(
            (start.status, body(&start)["recording"].clone()),
            (200, json!(true))
        );
        let stop = c.handle(
            &request("POST", "/v1/capture/record/stop"),
            &["record", "stop"],
        );
        assert_eq!(
            (stop.status, body(&stop)["samples"].clone()),
            (200, json!(120))
        );
        assert_eq!(
            t.join().unwrap(),
            ["screencap record start", "screencap record stop"]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_compositors_reasons_become_statuses_and_keep_their_words() {
        let cases = [
            ("disabled", 403, "capture_disabled"),
            ("no-indicator", 409, "no_indicator"),
            ("busy", 409, "busy"),
            ("not-recording", 409, "not_recording"),
            ("no-encoder", 501, "no_encoder"),
            ("disk-full", 507, "disk_full"),
            ("timeout", 504, "timeout"),
            ("grab", 500, "capture_failed"),
            ("encoder", 500, "capture_failed"),
        ];
        for (code, status, name) in cases {
            let r = relay(json!({"ok": false, "code": code, "error": format!("because {code}")}));
            assert_eq!(r.status, status, "{code}");
            let b = body(&r);
            assert_eq!(
                (
                    b["ok"].clone(),
                    b["error"]["code"].clone(),
                    b["error"]["message"].clone()
                ),
                (json!(false), json!(name), json!(format!("because {code}")))
            );
        }
        assert_eq!(relay(json!({"ok": false})).status, 500);
    }

    #[test]
    fn a_compositor_that_is_not_there_is_a_503_for_a_command_and_a_note_for_a_status() {
        let d = dir("down");
        let c = capture(true, &d.join("nothing-here"));
        let r = c.handle(&request("POST", "/v1/capture/screenshot"), &["screenshot"]);
        assert_eq!(r.status, 503);
        let b = body(&r);
        assert_eq!(b["error"]["code"], "compositor_unavailable");
        assert!(
            b["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing-here")
        );
        let r = c.handle(&request("GET", "/v1/capture"), &[]);
        let b = body(&r);
        assert_eq!(
            (
                r.status,
                b["capture"]["enabled"].clone(),
                b["capture"]["available"].clone()
            ),
            (200, json!(true), json!(false))
        );
        assert!(
            b["capture"]["reason"]
                .as_str()
                .unwrap()
                .contains("nothing-here")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_status_is_the_compositors_with_available_added() {
        let d = dir("status");
        let sock = d.join("control");
        let (_, t) = fake(
            &sock,
            vec![
                r#"{"ok":true,"enabled":true,"recording":false,"encoder":{"path":"/x","available":true},"taken":2}"#,
            ],
        );
        let r = capture(true, &sock).handle(&request("GET", "/v1/capture"), &[]);
        let b = body(&r);
        assert_eq!(r.status, 200);
        assert_eq!(
            (
                b["capture"]["available"].clone(),
                b["capture"]["taken"].clone(),
                b["capture"].get("ok").cloned()
            ),
            (json!(true), json!(2), None)
        );
        assert_eq!(t.join().unwrap(), ["screencap status"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_compositor_that_answers_nonsense_or_closes_on_us_is_down_not_a_crash() {
        let d = dir("nonsense");
        let sock = d.join("control");
        let (_, t) = fake(&sock, vec!["this is not json"]);
        let r = capture(true, &sock)
            .handle(&request("POST", "/v1/capture/screenshot"), &["screenshot"]);
        assert_eq!(r.status, 503);
        t.join().unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn methods_and_paths_are_told_apart() {
        let c = capture(false, std::path::Path::new("/x"));
        let r = c.handle(&request("GET", "/v1/capture/screenshot"), &["screenshot"]);
        assert_eq!(r.status, 405);
        assert!(r.headers.iter().any(|(n, v)| *n == "Allow" && v == "POST"));
        let r = c.handle(&request("POST", "/v1/capture"), &[]);
        assert_eq!(r.status, 405);
        assert!(r.headers.iter().any(|(n, v)| *n == "Allow" && v == "GET"));
        assert_eq!(
            c.handle(&request("POST", "/v1/capture/nope"), &["nope"])
                .status,
            404
        );
        assert_eq!(
            c.handle(&request("POST", "/v1/capture/record"), &["record"])
                .status,
            404
        );
    }

    #[test]
    fn the_defaults_are_off_and_the_section_parses_as_the_compositor_reads_it() {
        assert!(!crate::config::Config::default().capture.enabled);
        let c = crate::config::Config::parse(
            r#"{"capture": {"enabled": true, "maxBytes": 33554432, "maxRecordSeconds": 60, "record": {"height": 540, "fps": 24, "kbps": 3000}, "encoder": "/x/venc", "control": "/tmp/lumatv/lumatv-control"}}"#,
        )
        .unwrap();
        assert!(
            c.capture.enabled && c.capture.record.height == 540 && c.capture.encoder == "/x/venc"
        );
        for bad in [
            r#"{"capture": {"maxBytes": 1000}}"#,
            r#"{"capture": {"record": {"height": 100}}}"#,
            r#"{"capture": {"record": {"fps": 60}}}"#,
            r#"{"capture": {"maxRecordSeconds": 99999}}"#,
            r#"{"capture": {"control": "relative/path"}}"#,
            r#"{"capture": {"typo": true}}"#,
        ] {
            assert!(crate::config::Config::parse(bad).is_err(), "{bad}");
        }
    }
}
