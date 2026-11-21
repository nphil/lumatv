//! LG's memory manager, memchute (`/usr/sbin/memchute`, `com.webos.memorymanager`): its configuration, and living beside it.
//!
//! **Stock mode** (LG's memchute serves the name): everything the policy does only because memchute also runs is here. memchute
//! makes each app's `memory` and `freezer` cgroups, sets their `memory.swappiness` (10 in front, 100 behind), freezes and closes
//! background apps at its own low-memory levels, and keeps some apps alive. So that the two never fight:
//! - the apps memchute keeps are never frozen or closed here ([`Memchute::keeps`]);
//! - `memory.swappiness` is not written;
//! - a freezer group memchute has frozen is not taken over: only a group that reads `THAWED` is frozen, and only what the policy froze
//!   is thawed (both in the engine, and right without memchute too).
//!
//! **Ours** (LumaTV's shim serves the name, `manager.rs`): the same layers give the lists the engine keeps ([`Memchute::keeps_ours`]),
//! Home ([`Memchute::home`]) and the apps preloads wait for ([`Memchute::efs`]).

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where configd keeps memchute's layered configuration.
pub const LAYERS: &str = "/etc/configd/layers";

const LISTS: [&str; 7] = ["InputList", "KeepOnLaunch", "KeepOnLaunchEx", "KeepAlive", "HomeGroup", "QuickLaunch", "EfsTarget"];
/// What memchute itself never closes.
const KEPT_STOCK: [&str; 5] = ["InputList", "KeepOnLaunch", "KeepOnLaunchEx", "KeepAlive", "HomeGroup"];
/// What LumaTV's memory manager never closes or freezes: the plan's keep lists. `InputList` and `KeepOnLaunch` name apps memchute
/// keeps only while it relaunches them from a snapshot, which is not done here.
const KEPT_OURS: [&str; 4] = ["KeepAlive", "KeepOnLaunchEx", "HomeGroup", "QuickLaunch"];

#[derive(Default)]
pub struct Memchute {
    lists: BTreeMap<&'static str, BTreeSet<String>>,
}

impl Memchute {
    /// memchute as its configuration layers under `layers` describe it: each list the union of every layer's, so no layer's
    /// choice is lost.
    pub fn read(layers: &Path) -> Self {
        let mut files = Vec::new();
        find(layers, "com.webos.memorymanager.json", 4, &mut files);
        let mut lists: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
        for file in files {
            let Some(doc) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
                continue;
            };
            for list in LISTS {
                for id in doc.get(list).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                    lists.entry(list).or_default().insert(id.to_owned());
                }
            }
        }
        Self { lists }
    }

    fn any(&self, names: &[&str], app: &str) -> bool {
        names.iter().any(|n| self.lists.get(n).is_some_and(|l| l.contains(app)))
    }

    /// memchute keeps it alive. `EfsTarget` (apps it preloads for a fast start) is a preference, not a protection, and is left out.
    pub fn keeps(&self, app: &str) -> bool {
        self.any(&KEPT_STOCK, app)
    }

    pub fn keeps_ours(&self, app: &str) -> bool {
        self.any(&KEPT_OURS, app)
    }

    pub fn home(&self, app: &str) -> bool {
        self.any(&["HomeGroup"], app)
    }

    /// An app preloads must not compete with while it is in front (`setPreloadStatus limit`).
    pub fn efs(&self, app: &str) -> bool {
        self.any(&["EfsTarget"], app)
    }

    /// The lists as `getPolicy` shows them.
    pub fn policy(&self) -> Value {
        serde_json::to_value(&self.lists).unwrap_or(Value::Null)
    }
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

/// A memchute keeping `apps` alive in both modes.
#[cfg(test)]
pub fn keeping(apps: &[&str]) -> Memchute {
    with(&[("KeepAlive", apps)])
}

#[cfg(test)]
pub fn with(lists: &[(&'static str, &[&str])]) -> Memchute {
    Memchute { lists: lists.iter().map(|(n, apps)| (*n, apps.iter().map(|a| (*a).to_owned()).collect())).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_list_is_the_union_of_its_layers() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let device = dir.path().join("devicename/o22n2");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&device).unwrap();
        std::fs::write(
            base.join("com.webos.memorymanager.json"),
            r#"{"KeepAlive":["com.webos.app.livehbbtv"],"EfsTarget":["youtube.leanback.v4"],"HomeGroup":["com.webos.app.home"]}"#,
        )
        .unwrap();
        std::fs::write(
            device.join("com.webos.memorymanager.json"),
            r#"{"InputList":["netflix"],"KeepOnLaunchEx":["amazon.alexapr"],"QuickLaunch":["google.assistant"],"EfsTarget":["netflix"]}"#,
        )
        .unwrap();
        let m = Memchute::read(dir.path());
        assert!(m.keeps("netflix") && m.keeps("amazon.alexapr") && m.keeps("com.webos.app.livehbbtv"));
        assert!(!m.keeps("youtube.leanback.v4") && !m.keeps("google.assistant"));
        assert!(m.keeps_ours("google.assistant") && m.keeps_ours("amazon.alexapr") && !m.keeps_ours("netflix"));
        assert!(m.efs("youtube.leanback.v4") && m.efs("netflix") && m.home("com.webos.app.home"));
    }
}
