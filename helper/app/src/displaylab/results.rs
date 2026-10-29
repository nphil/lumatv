//! What the person answered in the lab, kept where the next round can read it: `<data dir>/displaylab/results.json` (the last run) and the
//! last ten runs as `results-<ms>.json`. The shell keeps its own copy and sends this one when a run ends; nothing here reads or changes a
//! register. The file names are made here, from the clock: a client names nothing on the disk.

use serde_json::Value;
use std::io::{self, Write};
use std::path::PathBuf;

/// The most a run's document may be.
pub const MAX_BYTES: usize = 32 * 1024;
/// The most steps a run has, and the longest id of one.
const MAX_STEPS: usize = 32;
const MAX_ID: usize = 64;
/// How many runs are kept besides the latest copy.
const KEEP: usize = 10;

pub struct Results {
    dir: PathBuf,
}

impl Results {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The document is an object with a `steps` list of objects that have an `id`, and is not large; `Err` names what is wrong.
    pub fn check(doc: &Value) -> Result<(), String> {
        let Some(map) = doc.as_object() else {
            return Err("the results must be a JSON object".to_owned());
        };
        let Some(steps) = map.get("steps").and_then(Value::as_array) else {
            return Err("the results need a steps list".to_owned());
        };
        if steps.len() > MAX_STEPS {
            return Err(format!("more than {MAX_STEPS} steps"));
        }
        for s in steps {
            match s.get("id").and_then(Value::as_str) {
                Some(id)
                    if !id.is_empty()
                        && id.len() <= MAX_ID
                        && id.bytes().all(|b| {
                            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
                        }) => {}
                _ => return Err("every step needs an id of letters, digits and - _ .".to_owned()),
            }
        }
        if serde_json::to_vec(doc).map_or(usize::MAX, |b| b.len()) > MAX_BYTES {
            return Err(format!("over {} KiB", MAX_BYTES / 1024));
        }
        Ok(())
    }

    /// Saves a run: `results.json` replaced atomically, `results-<now>.json` beside it, the oldest of those beyond ten removed. Returns the
    /// name of the dated file.
    pub fn save(&self, doc: &Value, now: u64) -> io::Result<String> {
        std::fs::create_dir_all(&self.dir)?;
        let mut stamped = doc.clone();
        if let Some(map) = stamped.as_object_mut() {
            map.insert("savedAtMs".to_owned(), Value::from(now));
        }
        let bytes = serde_json::to_vec_pretty(&stamped).map_err(io::Error::other)?;
        let name = format!("results-{now}.json");
        write_atomic(&self.dir.join(&name), &bytes)?;
        write_atomic(&self.dir.join("results.json"), &bytes)?;
        let mut dated: Vec<String> = std::fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("results-") && n.ends_with(".json"))
            .collect();
        dated.sort_by_key(|n| {
            n.trim_start_matches("results-")
                .trim_end_matches(".json")
                .parse::<u64>()
                .unwrap_or(0)
        });
        while dated.len() > KEEP {
            let _ = std::fs::remove_file(self.dir.join(dated.remove(0)));
        }
        Ok(name)
    }

    /// The last run saved, if there is one and it reads.
    pub fn latest(&self) -> Option<Value> {
        serde_json::from_slice(&std::fs::read(self.dir.join("results.json")).ok()?).ok()
    }
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(n: u64) -> Value {
        json!({"run": n, "steps": [{"id": "sharp-strong", "answer": "b"}, {"id": "dim", "answer": "yes"}]})
    }

    #[test]
    fn a_run_is_saved_whole_and_read_back_and_only_ten_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let r = Results::new(dir.path().join("displaylab"));
        assert_eq!(r.latest(), None);
        for n in 1..=12u64 {
            let name = r.save(&run(n), 1000 + n).unwrap();
            assert_eq!(name, format!("results-{}.json", 1000 + n));
        }
        let latest = r.latest().unwrap();
        assert_eq!(latest["run"], 12);
        assert_eq!(latest["savedAtMs"], 1012);
        let mut names: Vec<String> = std::fs::read_dir(dir.path().join("displaylab"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names.len(),
            11,
            "ten dated runs and the latest copy: {names:?}"
        );
        assert!(
            names.contains(&"results.json".to_owned())
                && names.contains(&"results-1012.json".to_owned())
                && !names.contains(&"results-1001.json".to_owned())
                && !names.contains(&"results-1002.json".to_owned())
        );
        assert!(
            !names.iter().any(|n| n.ends_with(".tmp")),
            "no temporary is left"
        );
    }

    #[test]
    fn a_document_that_is_not_a_run_is_refused() {
        assert!(Results::check(&run(1)).is_ok());
        assert!(Results::check(&json!([1])).is_err());
        assert!(Results::check(&json!({"steps": "x"})).is_err());
        assert!(Results::check(&json!({"run": 1})).is_err());
        assert!(
            Results::check(&json!({"steps": [{"answer": "a"}]})).is_err(),
            "a step without an id"
        );
        assert!(Results::check(&json!({"steps": [{"id": "../etc/passwd"}]})).is_err());
        assert!(Results::check(&json!({"steps": [{"id": ""}]})).is_err());
        let many: Vec<Value> = (0..33).map(|i| json!({"id": format!("s{i}")})).collect();
        assert!(Results::check(&json!({"steps": many})).is_err());
        let big = "x".repeat(MAX_BYTES);
        assert!(Results::check(&json!({"steps": [{"id": "a", "note": big}]})).is_err());
    }
}
