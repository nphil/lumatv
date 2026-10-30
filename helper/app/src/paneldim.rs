//! The net under the compositor's panel dim: a panel left dim by a compositor that died is put back.
//!
//! The compositor dims the panel's own light (LG's picture setting "OLED pixel brightness") for the screensaver and an idle Home, and
//! keeps a journal of what it did in `paneldim.json` of the data directory: `{orig, written, dimension, pid, at}`, the user's value, the
//! value last written, the picture mode, input and range it belongs to, and its own pid. It puts the value back on every way out it
//! sees, and a restarted compositor reads a journal left behind (`compositor/.../src/paneldim.rs`, `docs/research/panel-dim-result.md`).
//! This module is for the time in between: when the compositor is gone and nothing has restarted it, a dim panel would stay dim.
//!
//! Every [`EVERY`] it looks at the journal. A journal whose pid is a running compositor is left alone. One that is under [`GRACE`] old is
//! left to the compositor that is about to restart. Otherwise the setting is read (with the journal's dimension); a value that is the one
//! last written is replaced by the user's, one that is anything else was changed by someone since and is left alone; either way the journal
//! goes. A read or a write that fails leaves the journal for the next look. Only `com.webos.service.settings` is called: nothing here
//! touches the power state, so LG's panel refresh is never woken or cut short (`docs/research/refresh-guard-result.md`).

use crate::ctx::Ctx;
use crate::refreshguard::LunaSource;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

/// The journal's name in the data directory (the compositor's `paneldim::JOURNAL`).
pub const JOURNAL: &str = "paneldim.json";
const READ_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
const WRITE_URI: &str = "luna://com.webos.service.settings/setSystemSettings";
/// How often the journal is looked at.
const EVERY: Duration = Duration::from_secs(15);
/// A journal this young belongs to a compositor that may be restarting: its own recovery comes first.
pub const GRACE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub struct Journal {
    pub orig: u32,
    pub written: u32,
    pub dimension: Value,
    pub pid: u32,
}

pub fn read_journal(path: &Path) -> Option<Journal> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let n = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .filter(|n| *n <= 100)
            .map(|n| n as u32)
    };
    Some(Journal {
        orig: n("orig")?,
        written: n("written")?,
        dimension: v
            .get("dimension")
            .filter(|d| d.is_object())
            .cloned()
            .unwrap_or(Value::Null),
        pid: v.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
    })
}

/// What one look did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No journal.
    Nothing,
    /// The compositor that wrote it is running.
    Alive,
    /// Younger than [`GRACE`]: the compositor's own recovery comes first.
    Waiting,
    /// The panel's value is the user's again.
    Restored(u32),
    /// The setting was not the one written: someone changed it, and it stays; the journal went.
    LeftAlone(u32),
    /// The service did not do it; the journal stays.
    Failed(String),
}

/// The settings service: `luna-send` on the set, a stand-in in a test.
pub trait Settings {
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, String>;
}

impl Settings for LunaSource {
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
        let text = LunaSource::call(self, uri, &payload.to_string())?;
        let v: Value = serde_json::Deserializer::from_str(&text)
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .ok_or_else(|| "no JSON reply".to_owned())?;
        Ok(v)
    }
}

/// Whether `pid` is a running LumaTV compositor.
pub fn compositor_running(pid: u32) -> bool {
    pid != 0
        && std::fs::read(format!("/proc/{pid}/cmdline"))
            .is_ok_and(|c| String::from_utf8_lossy(&c).contains("lumatv-compositor"))
}

/// The part of a dimension the service takes in a call: `{pictureMode, input}`. One that names `dynamicRange` too is refused by a read ("no
/// result from DB", TV round 20); the compositor's journal holds only these two, and an older one is cut to them here.
fn call_dimension(d: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for k in ["pictureMode", "input"] {
        if let Some(v) = d.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Object(out)
    }
}

fn backlight(reply: &Value) -> Option<u32> {
    let n = match reply.pointer("/settings/backlight")? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && (0.0..=100.0).contains(&n)).then(|| n.round() as u32)
}

/// One look at the journal at `path`: `age` is how old the file is, `alive` says whether a pid is a running compositor.
pub fn look(
    path: &Path,
    age: Duration,
    alive: &dyn Fn(u32) -> bool,
    settings: &dyn Settings,
) -> Outcome {
    let Some(j) = read_journal(path) else {
        return Outcome::Nothing;
    };
    if alive(j.pid) {
        return Outcome::Alive;
    }
    if age < GRACE {
        return Outcome::Waiting;
    }
    let dimension = call_dimension(&j.dimension);
    let mut read = json!({"category": "picture", "keys": ["backlight"]});
    if dimension.is_object() {
        read["dimension"] = dimension.clone();
    }
    let now = match settings.call(READ_URI, &read) {
        Ok(r) => match backlight(&r) {
            Some(v) => v,
            None => return Outcome::Failed("no OLED pixel brightness in the answer".into()),
        },
        Err(e) => return Outcome::Failed(e),
    };
    if now != j.written {
        let _ = std::fs::remove_file(path);
        return Outcome::LeftAlone(now);
    }
    let mut write = json!({"category": "picture", "settings": {"backlight": j.orig.to_string()}});
    if dimension.is_object() {
        write["dimension"] = dimension;
    }
    match settings.call(WRITE_URI, &write) {
        Ok(r) if r.get("returnValue").and_then(Value::as_bool) != Some(false) => {
            let _ = std::fs::remove_file(path);
            Outcome::Restored(j.orig)
        }
        Ok(r) => Outcome::Failed(
            r.get("errorText")
                .and_then(Value::as_str)
                .unwrap_or("refused")
                .to_owned(),
        ),
        Err(e) => Outcome::Failed(e),
    }
}

fn age_of(path: &Path) -> Duration {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .unwrap_or_default()
}

/// Starts the watch: a look at the journal at start and every [`EVERY`].
pub fn spawn(ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("paneldim".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            let path: PathBuf = ctx.paths.data_dir.join(JOURNAL);
            let luna = LunaSource::new(&ctx.config.luna.send);
            let mut last = Outcome::Nothing;
            loop {
                if path.exists() {
                    let now = look(&path, age_of(&path), &compositor_running, &luna);
                    if now != last {
                        match &now {
                            Outcome::Restored(v) => crate::log_info!(
                                "panel dim: the compositor is gone and left the panel dim: OLED pixel brightness put back to {v}"
                            ),
                            Outcome::LeftAlone(v) => crate::log_info!(
                                "panel dim: a journal of a compositor that ended; the value is {v}, changed since: left alone"
                            ),
                            Outcome::Failed(why) => {
                                crate::log_warn!("panel dim: could not put the panel back: {why}")
                            }
                            _ => {}
                        }
                    }
                    last = now;
                }
                if ctx.stop.sleep(EVERY) {
                    return;
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    /// A settings service with one value of one dimension.
    struct Tv {
        value: Mutex<u32>,
        dimension: Value,
        refuse: bool,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl Tv {
        fn new(value: u32) -> Tv {
            Tv {
                value: Mutex::new(value),
                dimension: json!({"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"}),
                refuse: false,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Settings for Tv {
        fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
            self.calls.lock().push((uri.to_owned(), payload.clone()));
            if uri == READ_URI {
                if payload["dimension"].get("dynamicRange").is_some() {
                    return Ok(json!({"returnValue": false, "errorText": "no result from DB"}));
                }
                return Ok(
                    json!({"returnValue": true, "settings": {"backlight": self.value.lock().to_string()}, "dimension": self.dimension}),
                );
            }
            if self.refuse {
                return Ok(json!({"returnValue": false, "errorText": "not now"}));
            }
            *self.value.lock() = payload["settings"]["backlight"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            Ok(json!({"returnValue": true}))
        }
    }

    fn journal(dir: &Path, orig: u32, written: u32, pid: u32) -> PathBuf {
        let path = dir.join(JOURNAL);
        let dimension =
            json!({"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"});
        std::fs::write(
            &path,
            json!({"orig": orig, "written": written, "dimension": dimension, "pid": pid, "at": 1})
                .to_string(),
        )
        .unwrap();
        path
    }

    const OLD: Duration = Duration::from_secs(120);

    #[test]
    fn a_journal_of_a_compositor_that_is_gone_puts_the_value_back_with_its_dimension_and_goes() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal(dir.path(), 80, 44, 4242);
        let tv = Tv::new(44);
        assert_eq!(look(&path, OLD, &|_| false, &tv), Outcome::Restored(80));
        assert_eq!(*tv.value.lock(), 80);
        assert!(!path.exists());
        let calls = tv.calls.lock();
        assert_eq!(calls.len(), 2, "a read, then a write");
        let named = json!({"pictureMode": "filmMaker", "input": "hdmi2"});
        assert_eq!(
            calls[0].1["dimension"], named,
            "the read names the picture mode and input that were dimmed, and no range"
        );
        assert_eq!(calls[1].1["dimension"], named, "and so does the write");
        assert_eq!(
            calls[1].1["settings"]["backlight"], "80",
            "as text, the way the service wants it"
        );
        assert!(
            calls
                .iter()
                .all(|c| c.0.starts_with("luna://com.webos.service.settings/")),
            "no other service is touched"
        );
    }

    #[test]
    fn nothing_happens_without_a_journal_while_the_compositor_runs_and_while_the_journal_is_young()
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(JOURNAL);
        let tv = Tv::new(44);
        assert_eq!(look(&path, OLD, &|_| false, &tv), Outcome::Nothing);
        journal(dir.path(), 80, 44, 4242);
        assert_eq!(look(&path, OLD, &|pid| pid == 4242, &tv), Outcome::Alive);
        assert_eq!(
            look(&path, Duration::from_secs(5), &|_| false, &tv),
            Outcome::Waiting,
            "a restarting compositor's own recovery comes first"
        );
        assert!(tv.calls.lock().is_empty() && path.exists());
    }

    #[test]
    fn a_value_someone_changed_since_is_left_alone_and_the_journal_goes() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal(dir.path(), 80, 44, 4242);
        let tv = Tv::new(61);
        assert_eq!(look(&path, OLD, &|_| false, &tv), Outcome::LeftAlone(61));
        assert_eq!(*tv.value.lock(), 61);
        assert_eq!(tv.calls.lock().len(), 1, "no write");
        assert!(!path.exists());
    }

    #[test]
    fn a_refused_write_leaves_the_journal_for_the_next_look() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal(dir.path(), 80, 44, 4242);
        let mut tv = Tv::new(44);
        tv.refuse = true;
        assert_eq!(
            look(&path, OLD, &|_| false, &tv),
            Outcome::Failed("not now".into())
        );
        assert!(path.exists());
        tv.refuse = false;
        assert_eq!(look(&path, OLD, &|_| false, &tv), Outcome::Restored(80));
    }

    #[test]
    fn a_journal_that_cannot_be_read_is_nothing_and_a_pid_of_zero_is_not_a_running_compositor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(JOURNAL);
        std::fs::write(&path, "{\"orig\": 300, \"written\": 4}").unwrap();
        assert_eq!(read_journal(&path), None);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read_journal(&path), None);
        assert!(!compositor_running(0));
        assert!(
            !compositor_running(std::process::id()),
            "the test process is not a compositor"
        );
    }

    #[test]
    fn the_journal_is_the_one_the_compositor_writes() {
        // the compositor's `save_journal` body: {orig, written, dimension, pid, at}
        let dir = tempfile::tempdir().unwrap();
        let path = journal(dir.path(), 73, 58, 7);
        let j = read_journal(&path).unwrap();
        assert_eq!((j.orig, j.written, j.pid), (73, 58, 7));
        assert_eq!(j.dimension["pictureMode"], "filmMaker");
        assert_eq!(JOURNAL, "paneldim.json");
    }

    #[test]
    fn the_reply_is_read_with_the_value_as_text_or_a_number() {
        assert_eq!(
            backlight(&json!({"settings": {"backlight": "80"}})),
            Some(80)
        );
        assert_eq!(
            backlight(&json!({"settings": {"backlight": 79.6}})),
            Some(80)
        );
        assert_eq!(backlight(&json!({"settings": {"backlight": "x"}})), None);
        assert_eq!(backlight(&json!({"settings": {"backlight": 101}})), None);
    }
}
