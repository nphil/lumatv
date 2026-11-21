//! Living beside LG's memory manager, memchute (`/usr/sbin/memchute`, `com.webos.memorymanager`): everything the policy does only
//! because memchute also runs is here, so it goes when the policy takes memchute's jobs over.
//!
//! memchute makes each app's `memory` and `freezer` cgroups, sets their `memory.swappiness` (10 in front, 100 behind), freezes and
//! closes background apps at its own low-memory levels, and keeps some apps alive. So that the two never fight:
//! - the apps memchute keeps are never frozen or closed here ([`Memchute::keeps`]);
//! - `memory.swappiness` is not written;
//! - a freezer group memchute has frozen is not taken over: only a group that reads `THAWED` is frozen, and only what the policy froze
//!   is thawed (both in the engine, and right without memchute too).

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where configd keeps memchute's layered configuration.
pub const LAYERS: &str = "/etc/configd/layers";

pub struct Memchute {
    kept: BTreeSet<String>,
}

impl Memchute {
    /// memchute as its configuration layers under `layers` describe it.
    pub fn read(layers: &Path) -> Self {
        Self { kept: kept(layers) }
    }

    pub fn keeps(&self, app: &str) -> bool {
        self.kept.contains(app)
    }
}

/// memchute's lists of apps it keeps alive or treats as an input or Home, from every layer of its configd file: the union, so no
/// layer's choice is lost. Its `EfsTarget` (apps it preloads for a fast start) is a preference, not a protection, and is left out.
fn kept(layers: &Path) -> BTreeSet<String> {
    const LISTS: [&str; 5] = ["InputList", "KeepOnLaunch", "KeepOnLaunchEx", "KeepAlive", "HomeGroup"];
    let mut files = Vec::new();
    find(layers, "com.webos.memorymanager.json", 4, &mut files);
    let mut ids = BTreeSet::new();
    for file in files {
        let Some(doc) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
            continue;
        };
        for list in LISTS {
            for id in doc.get(list).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                ids.insert(id.to_owned());
            }
        }
    }
    ids
}

fn find(dir: &Path, name: &str, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                find(&path, name, depth - 1, out);
            }
        } else if entry.file_name() == name {
            out.push(path);
        }
    }
}

#[cfg(test)]
pub fn keeping(apps: &[&str]) -> Memchute {
    Memchute { kept: apps.iter().map(|a| (*a).to_owned()).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kept_apps_are_the_union_of_its_layers() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let device = dir.path().join("devicename/o22n2");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&device).unwrap();
        std::fs::write(base.join("com.webos.memorymanager.json"), r#"{"KeepAlive":["com.webos.app.livehbbtv"],"EfsTarget":["youtube.leanback.v4"]}"#).unwrap();
        std::fs::write(device.join("com.webos.memorymanager.json"), r#"{"InputList":["netflix"],"KeepOnLaunchEx":["amazon.alexapr"]}"#).unwrap();
        let m = Memchute::read(dir.path());
        assert!(m.keeps("netflix") && m.keeps("amazon.alexapr") && m.keeps("com.webos.app.livehbbtv"));
        assert!(!m.keeps("youtube.leanback.v4"));
    }
}
