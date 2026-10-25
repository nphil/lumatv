//! The audit trail: every call an assistant makes, and what became of it.
//!
//! "Every call is logged and shown in Settings › AI access" (the plan). Entries are kept in memory (the last 500, for the TV's
//! page) and appended to `<data dir>/ai/audit.jsonl` (rotated at 256 KiB to `audit.jsonl.1`, so it is bounded to twice that),
//! and each one is announced as an `ai` / `audit` event so the page updates as it happens. The file survives a restart: the last
//! entries are read back at start. Arguments are kept, cut short; tokens and pairing codes never reach this module.

use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;

/// How many entries stay in memory.
pub const RING: usize = 500;
/// The size at which the file becomes `.1` and a new one starts.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// How many entries are read back at start.
const PRELOAD: usize = 200;

/// What became of a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ran and did what was asked.
    Ok,
    /// It was allowed and ran, and the TV or the receiver could not do it.
    Failed,
    /// The TV's owner pressed Back on the prompt: the assistant is paused.
    Denied,
    /// Nobody answered the prompt.
    Expired,
    /// The prompt was withdrawn (a lock-out).
    Cancelled,
    /// The assistant is paused.
    Paused,
    /// The assistant has `Never` for the tool's group.
    Never,
    /// Another prompt was open.
    Busy,
    /// The arguments or the target were not acceptable.
    Invalid,
    /// Too many calls lately.
    RateLimited,
    /// Too soon after the last unanswered prompt.
    Cooldown,
    /// The assistant is no longer paired.
    Unauthorized,
}

impl Outcome {
    pub fn id(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
            Self::Paused => "paused",
            Self::Never => "never",
            Self::Busy => "busy",
            Self::Invalid => "invalid",
            Self::RateLimited => "rate_limited",
            Self::Cooldown => "cooldown",
            Self::Unauthorized => "unauthorized",
        }
    }
}

/// One call, as it is recorded.
pub struct Entry<'a> {
    pub assistant: Option<(&'a str, &'a str)>,
    pub peer: Option<std::net::IpAddr>,
    pub tool: &'a str,
    pub args: &'a Value,
    pub level: Option<&'a str>,
    pub outcome: Outcome,
    pub message: &'a str,
    pub took_ms: u64,
}

struct Sink {
    file: Option<File>,
    size: u64,
}

pub struct Audit {
    path: PathBuf,
    hub: Arc<Hub>,
    ring: Mutex<VecDeque<Value>>,
    sink: Mutex<Sink>,
}

/// An argument document made small enough to keep: strings cut to 120 characters, nesting flattened to its text.
fn keep(args: &Value) -> Value {
    match args {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map.iter().take(12) {
                let short = match value {
                    Value::String(s) => Value::String(crate::ai::tools::clean_text(s, 120)),
                    Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
                    other => Value::String(crate::ai::tools::clean_text(&other.to_string(), 120)),
                };
                out.insert(key.chars().take(40).collect(), short);
            }
            Value::Object(out)
        }
        Value::Null => json!({}),
        other => json!({"arguments": crate::ai::tools::clean_text(&other.to_string(), 120)}),
    }
}

fn open_append(path: &PathBuf) -> Option<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .ok()
}

impl Audit {
    /// Opens the trail at `<dir>/audit.jsonl`, reading back the last entries.
    pub fn open(dir: PathBuf, hub: Arc<Hub>) -> Arc<Self> {
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("audit.jsonl");
        let mut ring = VecDeque::new();
        if let Ok(text) = fs::read_to_string(&path) {
            let lines: Vec<&str> = text.lines().collect();
            for line in lines.iter().skip(lines.len().saturating_sub(PRELOAD)) {
                if let Ok(value) = serde_json::from_str::<Value>(line)
                    && value.is_object()
                {
                    ring.push_back(value);
                }
            }
        }
        let size = fs::metadata(&path).map_or(0, |m| m.len());
        Arc::new(Self {
            sink: Mutex::new(Sink {
                file: open_append(&path),
                size,
            }),
            path,
            hub,
            ring: Mutex::new(ring),
        })
    }

    /// Records one call and announces it.
    pub fn record(&self, entry: &Entry<'_>) {
        let doc = json!({
            "at": now_ms(),
            "assistant": entry.assistant.map(|(id, name)| json!({"id": id, "name": name})),
            "peer": entry.peer.map(|p| p.to_string()),
            "tool": entry.tool,
            "args": keep(entry.args),
            "level": entry.level,
            "outcome": entry.outcome.id(),
            "message": crate::ai::tools::clean_text(entry.message, 200),
            "ms": entry.took_ms,
        });
        {
            let mut ring = self.ring.lock();
            if ring.len() == RING {
                ring.pop_front();
            }
            ring.push_back(doc.clone());
        }
        let line = format!("{doc}\n");
        {
            let mut sink = self.sink.lock();
            if sink.size + line.len() as u64 > MAX_FILE_BYTES {
                sink.file = None;
                let mut old = self.path.as_os_str().to_owned();
                old.push(".1");
                let _ = fs::rename(&self.path, PathBuf::from(old));
                sink.file = open_append(&self.path);
                sink.size = 0;
            }
            if let Some(file) = sink.file.as_mut()
                && file.write_all(line.as_bytes()).is_ok()
            {
                sink.size += line.len() as u64;
            }
        }
        self.hub.emit("ai", "audit", doc);
    }

    /// The newest `limit` entries, newest last, optionally of one assistant.
    pub fn recent(&self, limit: usize, assistant: Option<&str>) -> Vec<Value> {
        let ring = self.ring.lock();
        let mut picked: Vec<Value> = ring
            .iter()
            .rev()
            .filter(|e| assistant.is_none_or(|id| e["assistant"]["id"] == id))
            .take(limit.min(RING))
            .cloned()
            .collect();
        picked.reverse();
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit(dir: &std::path::Path) -> (Arc<Audit>, Arc<Hub>) {
        let hub = Arc::new(Hub::new(1));
        (Audit::open(dir.to_owned(), hub.clone()), hub)
    }

    fn record(audit: &Audit, tool: &str, outcome: Outcome, id: &str) {
        audit.record(&Entry {
            assistant: Some((id, "Cody")),
            peer: Some("192.168.1.9".parse().unwrap()),
            tool,
            args: &json!({"app_id": "netflix"}),
            level: Some("ask"),
            outcome,
            message: "message",
            took_ms: 3,
        });
    }

    #[test]
    fn an_entry_is_kept_announced_and_written() {
        let dir = tempfile::tempdir().unwrap();
        let (audit, hub) = audit(dir.path());
        record(&audit, "tv_launch_app", Outcome::Denied, "a1");
        let all = audit.recent(10, None);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["tool"], "tv_launch_app");
        assert_eq!(all[0]["outcome"], "denied");
        assert_eq!(all[0]["assistant"], json!({"id": "a1", "name": "Cody"}));
        assert_eq!(all[0]["peer"], "192.168.1.9");
        assert_eq!(all[0]["args"], json!({"app_id": "netflix"}));
        assert_eq!(all[0]["level"], "ask");
        assert!(all[0]["at"].as_u64().unwrap() > 1_700_000_000_000);
        let event = hub.events_after(Some(0), None, None).events.pop().unwrap();
        assert_eq!((event.topic.as_str(), event.kind.as_str()), ("ai", "audit"));
        assert_eq!(event.data["tool"], "tv_launch_app");
        let text = fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(serde_json::from_str::<Value>(text.trim()).is_ok());
    }

    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (audit, _) = audit(dir.path());
        record(&audit, "tv_standby", Outcome::Ok, "a1");
        let mode = fs::metadata(dir.path().join("audit.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn entries_can_be_filtered_by_assistant_and_limited() {
        let dir = tempfile::tempdir().unwrap();
        let (audit, _) = audit(dir.path());
        for i in 0..6 {
            record(
                &audit,
                "tv_get_power",
                Outcome::Ok,
                if i % 2 == 0 { "a1" } else { "b2" },
            );
        }
        assert_eq!(audit.recent(100, None).len(), 6);
        assert_eq!(audit.recent(100, Some("a1")).len(), 3);
        assert_eq!(audit.recent(2, None).len(), 2);
        assert!(audit.recent(100, Some("nobody")).is_empty());
        assert_eq!(audit.recent(0, None).len(), 0);
    }

    #[test]
    fn the_ring_is_bounded_and_the_file_is_read_back_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        {
            let (audit, _) = audit(dir.path());
            for i in 0..(RING + 20) {
                record(&audit, "tv_get_power", Outcome::Ok, &format!("a{i}"));
            }
            assert_eq!(audit.recent(10_000, None).len(), RING);
        }
        let (again, _) = audit(dir.path());
        let back = again.recent(10_000, None);
        assert_eq!(back.len(), PRELOAD);
        assert_eq!(
            back.last().unwrap()["assistant"]["id"],
            format!("a{}", RING + 19)
        );
    }

    #[test]
    fn the_file_is_rotated_and_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let (audit, _) = audit(dir.path());
        let big = "x".repeat(150);
        for i in 0..4000 {
            audit.record(&Entry {
                assistant: None,
                peer: None,
                tool: "tv_get_power",
                args: &json!({"pad": big, "i": i}),
                level: None,
                outcome: Outcome::Ok,
                message: "m",
                took_ms: 1,
            });
        }
        let now = fs::metadata(dir.path().join("audit.jsonl")).unwrap().len();
        let old = fs::metadata(dir.path().join("audit.jsonl.1"))
            .unwrap()
            .len();
        assert!(
            now <= MAX_FILE_BYTES && old <= MAX_FILE_BYTES,
            "{now} {old}"
        );
        assert!(!dir.path().join("audit.jsonl.2").exists());
    }

    #[test]
    fn arguments_and_messages_are_cut_and_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let (audit, _) = audit(dir.path());
        let long = "y".repeat(500);
        audit.record(&Entry {
            assistant: None,
            peer: None,
            tool: "tv_launch_app",
            args: &json!({"app_id": long, "nested": {"a": [1, 2]}, "n": 5, "b": true}),
            level: None,
            outcome: Outcome::Invalid,
            message: &"m\u{7}".repeat(300),
            took_ms: 0,
        });
        let e = &audit.recent(1, None)[0];
        assert_eq!(e["args"]["app_id"].as_str().unwrap().chars().count(), 120);
        assert!(e["args"]["nested"].is_string());
        assert_eq!(e["args"]["n"], 5);
        assert!(e["message"].as_str().unwrap().chars().count() <= 200);
        assert!(!e["message"].as_str().unwrap().contains('\u{7}'));
        assert_eq!(e["assistant"], Value::Null);
    }

    #[test]
    fn a_damaged_file_is_skipped_line_by_line() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("audit.jsonl"),
            "not json\n{\"tool\":\"ok\"}\n[1,2]\n\n{\"tool\":\"two\"}\n",
        )
        .unwrap();
        let (audit, _) = audit(dir.path());
        let tools: Vec<_> = audit
            .recent(10, None)
            .iter()
            .map(|e| e["tool"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(tools, ["ok", "two"]);
    }
}
