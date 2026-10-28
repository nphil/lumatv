//! What LumaTV's compositor left behind when it was put out of the way: the crash notice Settings shows.
//!
//! When the supervisor (`compositor/sm/lumatv-sm.c`) hands the screen back to LG's compositor because of a problem (the compositor died, hung,
//! or drew nothing the panel showed), it saves a bundle under `/var/lib/lumatv/crash/<time>-<n>/`: `reason.txt` (reason, kind, time, how long the
//! compositor had run, failures counted), the compositor's last log lines, its own, the compositor's trace ring (what the loop did in its last
//! seconds) and a few numbers of the system. `latest` names the newest bundle, `pending` the newest one a problem made that nobody has read. A
//! hand-back the owner asked for (the hold-Home escape) leaves a bundle with kind `escape` and no `pending`.
//!
//! Settings > Support shows "LumaTV recovered from a problem" with a Details row while `pending` is there, and removes it when the owner has read
//! it. Nothing here polls: the page asks when it opens.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/crash` | `{crash: {pending, name, at, kind, reason, uptimeSecs, failures, path, log, trace, count} \| null}`: `log` and `trace` are the last lines |
//! | `POST /v1/crash/ack` | `{cleared}`: `pending` removed; the bundles stay on the TV for whoever reads them |

use crate::API_VERSION;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const DEFAULT_ROOT: &str = "/var/lib/lumatv/crash";
/// The most of a file's end that is read, and the lines of it that are returned.
const TAIL_BYTES: u64 = 32 * 1024;
const TAIL_LINES: usize = 30;

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/crash",
        "the newest crash bundle the supervisor saved when LumaTV's compositor was put out of the way: why, when, its last log and trace lines, and whether the owner has read it",
    ),
    (
        "POST",
        "/v1/crash/ack",
        "the owner has read the crash notice: it stops being shown (the bundles stay on the TV)",
    ),
];

/// The last `lines` lines of a file's last [`TAIL_BYTES`] bytes; none when it cannot be read.
fn tail(path: &Path, lines: usize) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_BYTES);
    let mut bytes = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err()
        || f.take(TAIL_BYTES).read_to_end(&mut bytes).is_err()
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    // The cut began inside a line: the first line is not whole.
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        all.remove(0);
    }
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

/// The value of `key: value` in a `reason.txt`.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
}

/// `20261028-031244-001` as `2026-10-28 03:12:44`; the name itself when it is not that (a clock that was not set: `boot-000123-001`).
fn when(name: &str) -> String {
    let b = name.as_bytes();
    if name.len() >= 15 && b[8] == b'-' && b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit) {
        format!(
            "{}-{}-{} {}:{}:{}",
            &name[0..4],
            &name[4..6],
            &name[6..8],
            &name[9..11],
            &name[11..13],
            &name[13..15]
        )
    } else {
        name.to_string()
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// How many bundles are kept.
fn count(root: &Path) -> usize {
    std::fs::read_dir(root)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0)
}

/// The newest bundle, and whether its notice is waiting. `None` when the supervisor has never saved one.
pub fn latest(root: &Path) -> Option<Value> {
    let name = read(&root.join("latest"))?.trim().to_string();
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return None;
    }
    let dir = root.join(&name);
    let reason = read(&dir.join("reason.txt"))?;
    let pending = read(&root.join("pending")).is_some_and(|p| p.trim() == name);
    Some(json!({
        "pending": pending,
        "name": name,
        "at": when(&name),
        "kind": field(&reason, "kind").unwrap_or("problem"),
        "reason": field(&reason, "reason").unwrap_or(""),
        "uptimeSecs": field(&reason, "compositor uptime").and_then(|v| v.trim_end_matches(" s").parse::<u64>().ok()),
        "failures": field(&reason, "failures").and_then(|v| v.parse::<u64>().ok()),
        "path": dir.display().to_string(),
        "log": tail(&dir.join("compositor.log.tail"), TAIL_LINES),
        "trace": tail(&dir.join("trace.txt"), TAIL_LINES),
        "count": count(root),
    }))
}

/// Takes the notice away: `pending` is removed, whichever bundle it names. True when it was there.
pub fn ack(root: &Path) -> bool {
    std::fs::remove_file(root.join("pending")).is_ok()
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

/// Serves `/v1/crash/...` on the bundles under `root`.
pub fn handle_with(root: &Path, request: &Request, rest: &[&str]) -> Response {
    match (request.method.as_str(), rest) {
        ("GET", []) => ok(json!({"crash": latest(root)})),
        ("POST", ["ack"]) => ok(json!({"cleared": ack(root)})),
        (_, [] | ["ack"]) => {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", if rest.is_empty() { "GET" } else { "POST" })
        }
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}

/// `LUMATV_CRASH_DIR` (a test, a development TV) or [`DEFAULT_ROOT`].
fn root() -> PathBuf {
    std::env::var_os("LUMATV_CRASH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT))
}

pub fn handle(request: &Request, rest: &[&str]) -> Response {
    handle_with(&root(), request, rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    /// A crash directory as the supervisor leaves it: one bundle, `latest` and (when asked) `pending` naming it.
    fn bundle(root: &Path, name: &str, kind: &str, pending: bool) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("reason.txt"),
            format!("reason: compositor killed by signal 9\nkind: {kind}\ntime: {name}\ncompositor uptime: 754 s\nboot: abc\nfailures: 1\n"),
        )
        .unwrap();
        let log: String = (1..=50)
            .map(|i| format!("[srv {i}.000] info line {i}\n"))
            .collect();
        fs::write(dir.join("compositor.log.tail"), log).unwrap();
        fs::write(
            dir.join("trace.txt"),
            "1.000 F flip 1.000 reached +90 us\n2.000 D frame 5 render 1200 us\n",
        )
        .unwrap();
        fs::write(root.join("latest"), name).unwrap();
        if pending {
            fs::write(root.join("pending"), name).unwrap();
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lumatv-crash-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_pending_bundle_is_read_with_its_reason_its_time_and_its_last_lines() {
        let root = tmp("read");
        bundle(&root, "20261028-031244-001", "problem", true);
        let r = handle_with(&root, &request("GET", "/v1/crash"), &[]);
        assert_eq!(r.status, 200);
        let c = &body(&r)["crash"];
        assert_eq!(
            (c["pending"].as_bool(), c["kind"].as_str()),
            (Some(true), Some("problem"))
        );
        assert_eq!(c["reason"], "compositor killed by signal 9");
        assert_eq!(c["at"], "2026-10-28 03:12:44");
        assert_eq!(
            (
                c["uptimeSecs"].as_u64(),
                c["failures"].as_u64(),
                c["count"].as_u64()
            ),
            (Some(754), Some(1), Some(1))
        );
        let log = c["log"].as_array().unwrap();
        assert_eq!(log.len(), TAIL_LINES);
        assert_eq!(
            log.last().unwrap(),
            "[srv 50.000] info line 50",
            "the newest lines, the last one last"
        );
        assert_eq!(c["trace"].as_array().unwrap().len(), 2);
        assert!(c["path"].as_str().unwrap().ends_with("20261028-031244-001"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_escape_is_not_a_notice_and_acknowledging_removes_the_one_that_is() {
        let root = tmp("ack");
        bundle(&root, "20261028-031244-001", "problem", true);
        assert_eq!(
            body(&handle_with(
                &root,
                &request("POST", "/v1/crash/ack"),
                &["ack"]
            ))["cleared"],
            true
        );
        assert_eq!(
            body(&handle_with(&root, &request("GET", "/v1/crash"), &[]))["crash"]["pending"],
            false,
            "read: not pending, still there"
        );
        assert_eq!(
            body(&handle_with(
                &root,
                &request("POST", "/v1/crash/ack"),
                &["ack"]
            ))["cleared"],
            false,
            "nothing left to clear"
        );
        bundle(&root, "20261028-040000-002", "escape", false);
        let c = body(&handle_with(&root, &request("GET", "/v1/crash"), &[]))["crash"].clone();
        assert_eq!(
            (
                c["kind"].as_str(),
                c["pending"].as_bool(),
                c["count"].as_u64()
            ),
            (Some("escape"), Some(false), Some(2))
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pending_that_names_an_older_bundle_is_not_the_newests() {
        let root = tmp("older");
        bundle(&root, "20261028-031244-001", "problem", true);
        bundle(&root, "20261028-040000-002", "problem", false);
        assert_eq!(
            body(&handle_with(&root, &request("GET", "/v1/crash"), &[]))["crash"]["pending"],
            false
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_directory_no_bundle_a_name_that_leaves_the_directory_or_a_bundle_without_reason_is_none()
    {
        let root = tmp("none");
        assert_eq!(
            body(&handle_with(
                &root.join("missing"),
                &request("GET", "/v1/crash"),
                &[]
            ))["crash"],
            Value::Null
        );
        fs::write(root.join("latest"), "../../etc").unwrap();
        assert_eq!(
            body(&handle_with(&root, &request("GET", "/v1/crash"), &[]))["crash"],
            Value::Null
        );
        fs::write(root.join("latest"), "gone").unwrap();
        assert_eq!(
            body(&handle_with(&root, &request("GET", "/v1/crash"), &[]))["crash"],
            Value::Null
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_time_of_a_clockless_boot_is_its_name() {
        assert_eq!(when("20261028-031244-001"), "2026-10-28 03:12:44");
        assert_eq!(when("boot-000123-001"), "boot-000123-001");
    }

    #[test]
    fn other_methods_and_paths_are_refused_by_name() {
        let root = tmp("refuse");
        assert_eq!(
            handle_with(&root, &request("POST", "/v1/crash"), &[]).status,
            405
        );
        assert_eq!(
            handle_with(&root, &request("GET", "/v1/crash/ack"), &["ack"]).status,
            405
        );
        assert_eq!(
            handle_with(&root, &request("GET", "/v1/crash/x"), &["x"]).status,
            404
        );
        let _ = fs::remove_dir_all(&root);
    }
}
