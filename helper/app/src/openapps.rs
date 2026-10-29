//! Open apps without updating: LG's list of apps that "must be updated before they open", kept empty while the owner wants it.
//!
//! LG's app updater (`appinstalld`, the `com.webos.appUpdateService` name) asks LG's server which installed apps have a newer
//! version and keeps the answer as a list: in memory and in `/mnt/lg/cmn_data/var/palm/data/com.webos.appInstallService/updateInfo`,
//! a JSON array of `{id, status, nsu, prohibit, type}`. SAM (the application manager) subscribes to that list
//! (`appUpdateService/statusAll`), rebuilds its own table from it at every notice and at every start of the updater, and an app that
//! is in the table gets LG's "A new version of the app is available" or "You must update this app to launch it" question when it is
//! launched. The homebrew *LG App Update Blocker* (`org.webosbrew.appupdateblocker`, dr0dr1dr2dr3/lgappupdateblocker, BSD-3-Clause)
//! found that the file can be emptied and, besides, maps LG's `*.lgtvsdp.com` names to `0.0.0.0`. This set's updater asks
//! `US.nextlgsdp.com` (`serverInfo`, the address LG's service-delivery service `sdx` gives every LG service), which that list does
//! not name, and a block of it would reach every other service that asks `sdx`, so the hosts block is not reproduced: only the list
//! is, and here it is kept empty instead of emptied once by hand
//! (docs/research/app-update-block-result.md).
//!
//! While the setting is on (flag file `<data dir>/openapps/on`), a tick every [`POLL`]:
//! - the list is empty: nothing to do (`holding`);
//! - the list has entries and the updater process is running: nothing is touched (`waiting`): the process may be installing an app the
//!   owner asked for in the store, and it can write its memory back to the file when it ends. The next tick looks again;
//! - the list has entries and the updater is not running: the list is saved (`<data dir>/openapps/updateInfo.saved`), the file is
//!   replaced by `[]` (written beside it and renamed), and `appUpdateService/statusAll` is asked once, which starts the updater from
//!   the empty file; SAM sees the service come up and rebuilds its table empty (`clearing` until the next tick reads the file).
//!
//! Turning it off removes the flag; the saved list goes back into the file when the file is empty (or missing) and the updater is
//! not running, and is dropped when LG has written a newer list since. The store keeps working throughout: nothing of the network
//! and nothing of the install service is touched, so an app is updated by hand from the store as before.
//!
//! Topic `openapps`: `{enabled, entries, ids, state, saved}`.
//! - `enabled`: the flag file exists;
//! - `entries`, `ids`: how many apps LG's list names and which (the first [`MAX_IDS`]);
//! - `state`: `holding` (on, list empty), `clearing` (on, list not empty, the next tick empties it), `waiting` (on, list not empty,
//!   LG's updater is running), `off` (flag not set), `unreadable` (the file is not a JSON array: it is left as it is);
//! - `saved`: a copy of an emptied list is kept, for turning it off.
//!
//! Endpoints: `GET /v1/openapps` the topic's value, `POST /v1/openapps` `{"enabled": bool}` sets the flag, ticks once and answers with
//! the value. `tools/lumatv` answers the same fields (`lumatv openapps status`, and `openApps` in `lumatv status`), read only.

use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::log::{self, Level};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const TOPIC: &str = "openapps";
/// LG's persisted update list.
pub const INFO: &str = "/mnt/lg/cmn_data/var/palm/data/com.webos.appInstallService/updateInfo";
/// The process name of LG's updater (`/usr/sbin/appinstalld`).
pub const UPDATER: &str = "appinstalld";
const POLL: Duration = Duration::from_secs(10);
const MAX_BYTES: u64 = 256 * 1024;
const MAX_IDS: usize = 8;
const DEADLINE: Duration = Duration::from_secs(8);

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/openapps",
        "open apps without updating: whether it is on, and what LG's list of apps waiting for an update holds",
    ),
    (
        "POST",
        "/v1/openapps",
        "turn it on or off: {\"enabled\": bool}; answers with the new state",
    ),
];

/// The files and programs used.
#[derive(Clone, Debug)]
pub struct Sources {
    pub info: PathBuf,
    pub flag: PathBuf,
    pub saved: PathBuf,
    pub proc_dir: PathBuf,
    pub luna_send: String,
}

impl Sources {
    /// The set's own: the state lives in the helper's data dir.
    pub fn lg(data_dir: &Path, luna_send: &str) -> Self {
        let dir = data_dir.join("openapps");
        Self {
            info: PathBuf::from(INFO),
            flag: dir.join("on"),
            saved: dir.join("updateInfo.saved"),
            proc_dir: PathBuf::from("/proc"),
            luna_send: luna_send.to_owned(),
        }
    }
}

/// What the update file says.
#[derive(Debug, PartialEq, Eq)]
pub enum Info {
    /// The ids in the array (a missing file is an empty array).
    Entries(Vec<String>),
    /// Present, but not a JSON array: not touched.
    Unreadable,
}

fn read_text(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    f.take(MAX_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// The list in `text`. An entry without an `id` still counts, by an empty id.
pub fn parse(text: &str) -> Info {
    if text.trim().is_empty() {
        return Info::Entries(Vec::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => Info::Entries(
            items
                .iter()
                .map(|e| e.get("id").and_then(Value::as_str).unwrap_or("").to_owned())
                .collect(),
        ),
        _ => Info::Unreadable,
    }
}

pub fn read_info(path: &Path) -> Info {
    match read_text(path) {
        Some(text) => parse(&text),
        None if path.exists() => Info::Unreadable,
        None => Info::Entries(Vec::new()),
    }
}

/// LG's updater, as far as this module needs to know it.
pub trait Updater {
    /// Whether the updater process is alive.
    fn running(&self) -> bool;
    /// Starts it (if it is not running) by asking for its list, and says how many apps the answer names.
    fn status_all(&self) -> Result<usize, String>;
}

/// The real one: `/proc` and `luna-send`.
pub struct Lg {
    proc_dir: PathBuf,
    luna_send: String,
}

impl Lg {
    pub fn new(s: &Sources) -> Self {
        Self {
            proc_dir: s.proc_dir.clone(),
            luna_send: s.luna_send.clone(),
        }
    }
}

impl Updater for Lg {
    fn running(&self) -> bool {
        let Ok(dir) = std::fs::read_dir(&self.proc_dir) else {
            return false;
        };
        dir.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
                && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == UPDATER)
        })
    }

    fn status_all(&self) -> Result<usize, String> {
        let mut child = Command::new(&self.luna_send)
            .args([
                "-n",
                "1",
                "luna://com.webos.appUpdateService/statusAll",
                "{}",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.luna_send))?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no output pipe".to_owned());
        };
        let (tx, rx) = mpsc::channel();
        let started = thread::Builder::new()
            .name("openapps-read".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut text = String::new();
                let _ = out.by_ref().take(MAX_BYTES).read_to_string(&mut text);
                let _ = tx.send(text);
            });
        if let Err(e) = started {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("could not start a reader: {e}"));
        }
        let text = rx.recv_timeout(DEADLINE).map_err(|_| {
            let _ = child.kill();
            "luna-send did not answer".to_owned()
        });
        let _ = child.wait();
        let doc: Value =
            serde_json::from_str(&text?).map_err(|_| "LG's updater gave no document".to_owned())?;
        if doc.get("returnValue") != Some(&Value::Bool(true)) {
            return Err("LG's updater refused the question".to_owned());
        }
        Ok(doc["status"]["apps"].as_array().map_or(0, Vec::len))
    }
}

/// Replaces the file's content: written beside it and renamed, so the updater never reads half a file.
fn write_info(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("lumatv");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn state_value(enabled: bool, info: &Info, waiting: bool, saved: bool) -> Value {
    let (entries, ids) = match info {
        Info::Entries(ids) => (
            ids.len(),
            ids.iter().take(MAX_IDS).cloned().collect::<Vec<_>>(),
        ),
        Info::Unreadable => (0, Vec::new()),
    };
    let state = match (enabled, info) {
        (_, Info::Unreadable) => "unreadable",
        (false, _) => "off",
        (true, _) if entries == 0 => "holding",
        (true, _) if waiting => "waiting",
        (true, _) => "clearing",
    };
    json!({"enabled": enabled, "entries": entries, "ids": ids, "state": state, "saved": saved})
}

/// The topic's value for the files as they are now, without touching anything.
pub fn read(s: &Sources, updater: &dyn Updater) -> Value {
    let info = read_info(&s.info);
    let waiting = matches!(&info, Info::Entries(ids) if !ids.is_empty()) && updater.running();
    state_value(s.flag.exists(), &info, waiting, s.saved.exists())
}

/// One pass: does what the flag asks and the updater allows, and returns the topic's value.
pub fn tick(s: &Sources, updater: &dyn Updater) -> Value {
    let enabled = s.flag.exists();
    let mut info = read_info(&s.info);
    if let Info::Entries(ids) = &info {
        if enabled && !ids.is_empty() && !updater.running() {
            clear(s, updater);
            info = read_info(&s.info);
        } else if !enabled && s.saved.exists() && !updater.running() {
            restore(s, updater, ids.is_empty());
            info = read_info(&s.info);
        }
    }
    let waiting = matches!(&info, Info::Entries(ids) if !ids.is_empty()) && updater.running();
    state_value(enabled, &info, waiting, s.saved.exists())
}

fn clear(s: &Sources, updater: &dyn Updater) {
    if let Some(dir) = s.saved.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Some(text) = read_text(&s.info) else {
        return;
    };
    if let Err(e) = std::fs::write(&s.saved, &text) {
        log::log(
            Level::Warn,
            format_args!("openapps: the list could not be saved ({e}); left as it is"),
        );
        return;
    }
    if let Err(e) = write_info(&s.info, "[]") {
        log::log(
            Level::Warn,
            format_args!("openapps: the list could not be emptied ({e})"),
        );
        return;
    }
    match updater.status_all() {
        Ok(0) => log::log(
            Level::Info,
            format_args!("openapps: LG's list emptied; the updater answers with no app"),
        ),
        Ok(n) => log::log(
            Level::Warn,
            format_args!("openapps: LG's list emptied, but the updater still names {n} app(s)"),
        ),
        Err(e) => log::log(
            Level::Warn,
            format_args!("openapps: LG's list emptied; the updater was not asked ({e})"),
        ),
    }
}

fn restore(s: &Sources, updater: &dyn Updater, file_empty: bool) {
    if !file_empty {
        // LG wrote a list since: that one is newer than the saved one.
        let _ = std::fs::remove_file(&s.saved);
        return;
    }
    let Some(text) = read_text(&s.saved) else {
        let _ = std::fs::remove_file(&s.saved);
        return;
    };
    if matches!(parse(&text), Info::Unreadable) {
        let _ = std::fs::remove_file(&s.saved);
        return;
    }
    match write_info(&s.info, &text) {
        Ok(()) => {
            let _ = std::fs::remove_file(&s.saved);
            let _ = updater.status_all();
            log::log(Level::Info, format_args!("openapps: LG's list put back"));
        }
        Err(e) => log::log(
            Level::Warn,
            format_args!("openapps: LG's list could not be put back ({e})"),
        ),
    }
}

/// Starts the watcher thread.
pub fn spawn(ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    let sources = Sources::lg(&ctx.paths.data_dir, &ctx.config.luna.send);
    let _ = SOURCES.set(sources.clone());
    thread::Builder::new()
        .name("openapps".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let updater = Lg::new(&sources);
            loop {
                ctx.hub.set(TOPIC, tick(&sources, &updater));
                if ctx.stop.sleep(POLL) {
                    return;
                }
            }
        })
}

static SOURCES: OnceLock<Sources> = OnceLock::new();

fn ok(value: Value) -> Response {
    Response::json(200, &json!({"ok": true, "openapps": value}))
}

/// Serves `/v1/openapps` on `sources`.
pub fn handle_with(
    sources: &Sources,
    updater: &dyn Updater,
    hub: &Hub,
    request: &Request,
) -> Response {
    match request.method.as_str() {
        "GET" => ok(read(sources, updater)),
        "POST" => {
            let enabled = match serde_json::from_slice::<Value>(&request.body) {
                Ok(Value::Object(m)) => match m.get("enabled") {
                    Some(Value::Bool(b)) if m.len() == 1 => *b,
                    _ => {
                        return error_response(
                            400,
                            "bad_request",
                            "the body is {\"enabled\": true or false}",
                        );
                    }
                },
                _ => {
                    return error_response(
                        400,
                        "bad_request",
                        "the body is {\"enabled\": true or false}",
                    );
                }
            };
            let changed = if enabled {
                sources
                    .flag
                    .parent()
                    .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
                    && std::fs::write(&sources.flag, b"").is_ok()
            } else {
                !sources.flag.exists() || std::fs::remove_file(&sources.flag).is_ok()
            };
            if !changed {
                return error_response(500, "write_failed", "the setting could not be saved");
            }
            log::log(
                Level::Info,
                format_args!("openapps: {}", if enabled { "on" } else { "off" }),
            );
            let value = tick(sources, updater);
            hub.set(TOPIC, value.clone());
            ok(value)
        }
        _ => error_response(405, "method_not_allowed", "that method is not served here")
            .with_header("Allow", "GET, POST"),
    }
}

pub fn handle(hub: &Hub, request: &Request) -> Response {
    match SOURCES.get() {
        Some(s) => handle_with(s, &Lg::new(s), hub, request),
        None => error_response(503, "not_ready", "the helper is still starting"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Fake {
        running: Cell<bool>,
        asked: Cell<u32>,
        /// What LG's updater would answer after it was started: the file's content then.
        info: PathBuf,
        answers: RefCell<Vec<usize>>,
    }

    impl Fake {
        fn new(info: &Path) -> Self {
            Self {
                running: Cell::new(false),
                asked: Cell::new(0),
                info: info.to_owned(),
                answers: RefCell::new(Vec::new()),
            }
        }
    }

    impl Updater for Fake {
        fn running(&self) -> bool {
            self.running.get()
        }
        fn status_all(&self) -> Result<usize, String> {
            self.asked.set(self.asked.get() + 1);
            let n = match read_info(&self.info) {
                Info::Entries(ids) => ids.len(),
                Info::Unreadable => 0,
            };
            self.answers.borrow_mut().push(n);
            Ok(n)
        }
    }

    const LISTED: &str = r#"[{"nsu":"Y","status":"F","prohibit":"N","id":"com.webos.app.lifeonscreen","type":"S"},{"nsu":"N","status":"N","prohibit":"Y","id":"dazn","type":"A"}]"#;

    struct Rig {
        _dir: tempfile::TempDir,
        s: Sources,
    }

    fn rig(info: Option<&str>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let s = Sources {
            info: dir.path().join("updateInfo"),
            flag: dir.path().join("state").join("openapps").join("on"),
            saved: dir
                .path()
                .join("state")
                .join("openapps")
                .join("updateInfo.saved"),
            proc_dir: dir.path().join("proc"),
            luna_send: "luna-send".into(),
        };
        if let Some(t) = info {
            std::fs::write(&s.info, t).unwrap();
        }
        Rig { _dir: dir, s }
    }

    fn on(s: &Sources) {
        std::fs::create_dir_all(s.flag.parent().unwrap()).unwrap();
        std::fs::write(&s.flag, "").unwrap();
    }

    #[test]
    fn parse_reads_ids_and_refuses_what_is_not_an_array() {
        assert_eq!(parse("[]"), Info::Entries(vec![]));
        assert_eq!(parse(" \n"), Info::Entries(vec![]));
        assert_eq!(
            parse(LISTED),
            Info::Entries(vec!["com.webos.app.lifeonscreen".into(), "dazn".into()])
        );
        assert_eq!(
            parse(r#"[{"status":"F"}]"#),
            Info::Entries(vec![String::new()])
        );
        for bad in ["{", "{}", "null", "\"x\"", "[1,", "abc"] {
            assert_eq!(parse(bad), Info::Unreadable, "{bad}");
        }
    }

    #[test]
    fn off_changes_nothing_and_says_what_lg_would_ask() {
        let r = rig(Some(LISTED));
        let u = Fake::new(&r.s.info);
        let v = tick(&r.s, &u);
        assert_eq!(
            (
                v["state"].as_str(),
                v["enabled"].as_bool(),
                v["entries"].as_u64()
            ),
            (Some("off"), Some(false), Some(2))
        );
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), LISTED);
        assert_eq!(u.asked.get(), 0);
        assert!(!r.s.saved.exists());
    }

    #[test]
    fn on_saves_the_list_empties_the_file_and_asks_the_updater_once() {
        let r = rig(Some(LISTED));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        let v = tick(&r.s, &u);
        assert_eq!(v["state"], "holding");
        assert_eq!(v["entries"], 0);
        assert_eq!(v["saved"], true);
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), "[]");
        assert_eq!(std::fs::read_to_string(&r.s.saved).unwrap(), LISTED);
        assert_eq!((u.asked.get(), u.answers.borrow().clone()), (1, vec![0]));
        assert!(
            !r.s.info.with_extension("lumatv").exists(),
            "no temporary file is left in LG's directory"
        );
        // A second pass has nothing to do.
        tick(&r.s, &u);
        assert_eq!(u.asked.get(), 1);
    }

    #[test]
    fn a_running_updater_is_left_alone_until_it_ends() {
        let r = rig(Some(LISTED));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        u.running.set(true);
        let v = tick(&r.s, &u);
        assert_eq!(
            (v["state"].as_str(), v["entries"].as_u64()),
            (Some("waiting"), Some(2))
        );
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), LISTED);
        assert_eq!(u.asked.get(), 0);
        u.running.set(false);
        assert_eq!(tick(&r.s, &u)["state"], "holding");
    }

    #[test]
    fn a_list_lg_writes_later_is_emptied_again_and_the_newest_is_the_one_kept() {
        let r = rig(Some(LISTED));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        tick(&r.s, &u);
        let later = r#"[{"nsu":"N","status":"F","prohibit":"N","id":"netflix","type":"A"}]"#;
        std::fs::write(&r.s.info, later).unwrap();
        assert_eq!(tick(&r.s, &u)["state"], "holding");
        assert_eq!(std::fs::read_to_string(&r.s.saved).unwrap(), later);
        assert_eq!(u.asked.get(), 2);
    }

    #[test]
    fn a_missing_or_unreadable_file_is_not_written() {
        let r = rig(None);
        on(&r.s);
        let u = Fake::new(&r.s.info);
        assert_eq!(tick(&r.s, &u)["state"], "holding");
        assert!(!r.s.info.exists());
        let r = rig(Some("{not json"));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        assert_eq!(tick(&r.s, &u)["state"], "unreadable");
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), "{not json");
        assert_eq!(u.asked.get(), 0);
    }

    #[test]
    fn off_puts_the_saved_list_back_when_the_file_is_empty() {
        let r = rig(Some(LISTED));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        tick(&r.s, &u);
        std::fs::remove_file(&r.s.flag).unwrap();
        u.running.set(true);
        assert_eq!(tick(&r.s, &u)["saved"], true, "not while the updater runs");
        u.running.set(false);
        let v = tick(&r.s, &u);
        assert_eq!(
            (
                v["state"].as_str(),
                v["entries"].as_u64(),
                v["saved"].as_bool()
            ),
            (Some("off"), Some(2), Some(false))
        );
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), LISTED);
        assert_eq!(u.asked.get(), 2);
    }

    #[test]
    fn off_drops_the_saved_list_when_lg_has_written_a_newer_one() {
        let r = rig(Some(LISTED));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        tick(&r.s, &u);
        std::fs::remove_file(&r.s.flag).unwrap();
        let newer = r#"[{"id":"hulu","status":"N"}]"#;
        std::fs::write(&r.s.info, newer).unwrap();
        let v = tick(&r.s, &u);
        assert_eq!(
            (v["saved"].as_bool(), v["entries"].as_u64()),
            (Some(false), Some(1))
        );
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), newer);
    }

    #[test]
    fn read_touches_nothing() {
        let r = rig(Some(LISTED));
        on(&r.s);
        let u = Fake::new(&r.s.info);
        assert_eq!(read(&r.s, &u)["state"], "clearing");
        u.running.set(true);
        assert_eq!(read(&r.s, &u)["state"], "waiting");
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), LISTED);
        assert!(!r.s.saved.exists());
    }

    fn request(method: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: "/v1/openapps".into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn body(r: &Response) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    #[test]
    fn the_endpoint_sets_the_flag_ticks_and_publishes() {
        let r = rig(Some(LISTED));
        let u = Fake::new(&r.s.info);
        let hub = Hub::new(1);
        let g = handle_with(&r.s, &u, &hub, &request("GET", ""));
        assert_eq!(body(&g)["openapps"]["state"], "off");
        let p = handle_with(&r.s, &u, &hub, &request("POST", r#"{"enabled":true}"#));
        assert_eq!(p.status, 200);
        assert_eq!(body(&p)["openapps"]["state"], "holding");
        assert!(r.s.flag.exists());
        assert_eq!(hub.get(TOPIC).unwrap()["state"], "holding");
        let p = handle_with(&r.s, &u, &hub, &request("POST", r#"{"enabled":false}"#));
        assert_eq!(body(&p)["openapps"]["state"], "off");
        assert!(!r.s.flag.exists());
        assert_eq!(std::fs::read_to_string(&r.s.info).unwrap(), LISTED);
        for bad in [
            "",
            "{",
            "[]",
            r#"{"enabled":"yes"}"#,
            r#"{"enabled":true,"x":1}"#,
            "{}",
        ] {
            assert_eq!(
                handle_with(&r.s, &u, &hub, &request("POST", bad)).status,
                400,
                "{bad}"
            );
        }
        assert_eq!(handle_with(&r.s, &u, &hub, &request("PUT", "")).status, 405);
    }

    /// `luna-send` as a shell script that prints what LG's updater printed on the set (2026-10-29, pretty-printed).
    fn fake_luna(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("luna-send");
        std::fs::write(&path, format!("#!/bin/sh\ncat <<'EOF'\n{body}\nEOF\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn the_updaters_answer_is_read_from_luna_sends_output() {
        let r = rig(None);
        let dir = r._dir.path();
        let mut s = r.s.clone();
        s.luna_send = fake_luna(
            dir,
            "{\n    \"subscribed\": false,\n    \"status\": {\n        \"apps\": [\n        ]\n    },\n    \"returnValue\": true\n}",
        );
        assert_eq!(Lg::new(&s).status_all(), Ok(0));
        s.luna_send = fake_luna(
            dir,
            r#"{"subscribed":false,"status":{"apps":[{"id":"dazn","status":"F"},{"id":"x"}]},"returnValue":true}"#,
        );
        assert_eq!(Lg::new(&s).status_all(), Ok(2));
        s.luna_send = fake_luna(
            dir,
            r#"{"returnValue":false,"errorText":"Service does not exist"}"#,
        );
        assert!(Lg::new(&s).status_all().is_err());
        s.luna_send = fake_luna(dir, "not json");
        assert!(Lg::new(&s).status_all().is_err());
        s.luna_send = dir.join("nothing-here").display().to_string();
        assert!(Lg::new(&s).status_all().is_err());
    }

    #[test]
    fn the_real_updater_is_found_by_its_process_name() {
        let r = rig(None);
        let lg = Lg::new(&r.s);
        assert!(!lg.running(), "no /proc at all");
        std::fs::create_dir_all(r.s.proc_dir.join("101")).unwrap();
        std::fs::write(r.s.proc_dir.join("101").join("comm"), "sam\n").unwrap();
        std::fs::create_dir_all(r.s.proc_dir.join("self")).unwrap();
        std::fs::write(r.s.proc_dir.join("self").join("comm"), "appinstalld\n").unwrap();
        assert!(!lg.running(), "only numbered directories are processes");
        std::fs::create_dir_all(r.s.proc_dir.join("202")).unwrap();
        std::fs::write(r.s.proc_dir.join("202").join("comm"), "appinstalld\n").unwrap();
        assert!(lg.running());
    }
}
