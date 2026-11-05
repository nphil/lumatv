//! Which plugins may connect: the ones `plugins/enabled.json` lists, each by the user id its process runs as; and which of the helper's
//! own plugins that ask first (`local::OptIn`) the owner switched on.
//!
//! A plugin has no name or token on the connection: the kernel says which user is at the other end (`SO_PEERCRED`) and this table
//! says which plugin that user is. Switching a plugin on the socket on installs software on the TV, so no route of the helper writes
//! its entry (the owner's command line does, `install.rs`); the same goes for the manifests. The helper writes only `firstParty`: the
//! owner's switch of one of its own plugins in Settings (`routes.rs`), which installs nothing.
//!
//! ```json
//! {"plugins": [{"id": "hello", "uid": 20000, "approve": "<sha-256 of the manifest's consent document>", "program": "<sha-256 of the program>"}],
//!  "firstParty": [{"id": "live-tv-tuner", "approve": "<sha-256 of its consent>"}]}
//! ```
//!
//! `approve` is what the owner approved: the hash of `Manifest::consent_document` (id, levels, program, arguments, class, leases,
//! permissions, limits). A manifest changed since then no longer matches and the plugin stays inert; a new display name or version
//! does not change the document and asks nothing new. `program` (optional) is the hash of the program file that was installed
//! beside it (`install::enable` writes it): the manifest names a program by path and not by content, so the runner starts only a
//! file that is that one. A first-party entry's `approve` is the hash of the words its plugin asks with (`OptIn::approval`): new
//! words ask again.

use plugin_api::permission::{Lease, Permission, SignalKind};
use plugin_api::{Manifest, check_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// One plugin that may connect.
#[derive(Clone, Debug)]
pub struct Member {
    pub id: String,
    pub uid: u32,
    pub manifest: Manifest,
    /// The hash of the program file the owner installed, when the entry says (see the module's header).
    pub program: Option<String>,
}

/// A plugin the file lists but that stays inert, and why in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inert {
    pub id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct Roster {
    pub members: Vec<Member>,
    pub inert: Vec<Inert>,
    /// The helper's own plugins the file switches on, each with the code it was approved by: the plugin that offers itself checks it
    /// against its consent (`session::Shared::offer`).
    pub first_party: Vec<Approved>,
}

/// `enabled.json`: the plugins that are switched on.
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnabledFile {
    pub plugins: Vec<Enabled>,
    #[serde(default, rename = "firstParty", skip_serializing_if = "Vec::is_empty")]
    pub first_party: Vec<Approved>,
}

/// One plugin that is switched on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enabled {
    pub id: String,
    pub uid: u32,
    pub approve: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

/// One of the helper's own plugins that the owner switched on, and the code of the consent they approved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approved {
    pub id: String,
    pub approve: String,
}

/// `enabled.json` in `dir`: empty when there is none. A file that cannot be read or understood is an error, so that nothing writes over
/// it.
pub fn read_file(dir: &Path) -> Result<EnabledFile, String> {
    let file = dir.join("enabled.json");
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "{} is damaged ({e}): fix it or remove it, nothing was changed",
                file.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(EnabledFile::default()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Writes `enabled.json` in `dir` whole, through a temporary file and a rename: a reader sees the old file or the new one.
pub fn write_file(dir: &Path, enabled: &EnabledFile) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("enabled.json");
    let tmp = dir.join("enabled.json.tmp");
    let text = serde_json::to_vec_pretty(enabled).map_err(|e| e.to_string())?;
    let done = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&tmp)?;
        f.write_all(&text)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &file)
    })();
    if let Err(e) = done {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", file.display()));
    }
    Ok(())
}

/// Why this TV will not run a plugin with this manifest, in words, or `None`: a level it does not speak, a permission it does not
/// grant, something it does not serve (see [`unserved`]).
pub fn refusal(manifest: &Manifest) -> Option<String> {
    if let Err(refusal) = manifest.check_level() {
        return Some(refusal.to_string());
    }
    let unsupported = manifest.unsupported();
    if !unsupported.is_empty() {
        return Some(format!("needs a newer LumaTV ({})", unsupported.join(", ")));
    }
    let not_served = unserved(manifest);
    if !not_served.is_empty() {
        return Some(format!(
            "asks for what this TV does not serve yet ({})",
            not_served.join(", ")
        ));
    }
    None
}

/// The approval of a manifest: what `approve` in `enabled.json` must be.
pub fn approval(manifest: &Manifest) -> String {
    let digest = Sha256::digest(manifest.consent_document().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// What LumaTV does not serve at level 1. A plugin that asks for any of it is inert, and says so, instead of connecting to be
/// refused message by message: the TV has no timeline, no pane host and no network or Bluetooth brokering. It does serve the Home
/// Assistant lease (`ha.rs`) with `ha.read` and `ha.call`, and the occupancy signal, but never a call on what is the owner's to
/// operate ([`super::ha::NEVER_CALLED`]). A lease or a permission of a later level (the decoder, a widget) is not refused: this TV
/// speaks level 1 (`session::TV_LEVEL`) and a plugin whose `min` is 1 runs without them, as the plugin API's level rule says.
fn unserved(manifest: &Manifest) -> Vec<String> {
    let level = super::session::TV_LEVEL;
    let mut found: Vec<String> = manifest
        .requires
        .iter()
        .filter(|l| **l != Lease::Ha && l.available_at(level))
        .map(|l| l.name().to_owned())
        .collect();
    for permission in manifest
        .permissions
        .iter()
        .filter(|p| p.available_at(level))
    {
        match permission {
            Permission::Settings
            | Permission::Cards
            | Permission::Tiles
            | Permission::Notify
            | Permission::Background
            | Permission::ProcessSpawn
            | Permission::UiView
            | Permission::SignalProvide(SignalKind::Occupancy) => {}
            // The permissions of the lease need `lease:ha`, which the manifest's own parse holds to.
            Permission::HaRead(_) => {}
            Permission::HaCall(domains) => {
                for domain in domains {
                    if super::ha::NEVER_CALLED.contains(&domain.as_str()) {
                        found.push(format!(
                            "ha.call:{domain} (this TV never lets a plugin call those)"
                        ));
                    }
                }
            }
            other => found.push(other.to_string()),
        }
    }
    found
}

fn read_limited(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl Roster {
    /// Reads `enabled.json` in `dir` (a plugin's user id is `first_uid` up to 63 above it) and each listed plugin's `<dir>/<id>/plugin.json`. A missing file is an empty roster; a file
    /// that cannot be read or understood is an empty roster too (nothing is switched on by a damaged file) with the reason.
    pub fn load(dir: &Path, first_uid: u32) -> (Self, Option<String>) {
        let file = dir.join("enabled.json");
        let bytes = match read_limited(&file, 64 * 1024) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => return (Self::default(), Some(format!("enabled.json: {e}"))),
        };
        if bytes.len() > 64 * 1024 {
            return (Self::default(), Some("enabled.json: over 64 KiB".into()));
        }
        let listed: EnabledFile = match serde_json::from_slice(&bytes) {
            Ok(listed) => listed,
            Err(e) => return (Self::default(), Some(format!("enabled.json: {e}"))),
        };
        let mut roster = Self::default();
        for approved in listed.first_party {
            if !roster.first_party.iter().any(|a| a.id == approved.id) {
                roster.first_party.push(approved);
            }
        }
        let mut ids = BTreeSet::new();
        let mut uids = BTreeSet::new();
        for entry in listed.plugins {
            let mut inert = |reason: String| {
                roster.inert.push(Inert {
                    id: entry.id.clone(),
                    reason,
                });
            };
            if let Err(e) = check_id(&entry.id) {
                inert(format!("not a plugin id: {e}"));
                continue;
            }
            if !ids.insert(entry.id.clone()) {
                inert("listed twice".into());
                continue;
            }
            if super::local::FIRST_PARTY_IDS.contains(&entry.id.as_str()) {
                inert("that name belongs to a plugin of the helper's own".into());
                continue;
            }
            if entry.uid < first_uid || entry.uid - first_uid >= plugin_api::MAX_PLUGINS {
                inert(format!(
                    "user id {} is outside the plugin range {} to {}",
                    entry.uid,
                    first_uid,
                    first_uid + plugin_api::MAX_PLUGINS - 1
                ));
                continue;
            }
            if !uids.insert(entry.uid) {
                inert(format!("user id {} is another plugin's", entry.uid));
                continue;
            }
            if roster.members.len() >= plugin_api::MAX_PLUGINS as usize {
                inert("more plugins than may be on at once".into());
                continue;
            }
            let path = dir.join(&entry.id).join("plugin.json");
            let manifest = match read_limited(&path, plugin_api::manifest::MAX_BYTES)
                .map_err(|e| e.to_string())
                .and_then(|bytes| Manifest::parse(&bytes).map_err(|e| e.to_string()))
            {
                Ok(manifest) if manifest.id == entry.id => manifest,
                Ok(_) => {
                    inert("its plugin.json names another plugin".into());
                    continue;
                }
                Err(e) => {
                    inert(format!("plugin.json: {e}"));
                    continue;
                }
            };
            if let Some(why) = refusal(&manifest) {
                inert(why);
                continue;
            }
            if !entry.approve.eq_ignore_ascii_case(&approval(&manifest)) {
                inert("its manifest is not the one that was approved".into());
                continue;
            }
            roster.members.push(Member {
                id: entry.id.clone(),
                uid: entry.uid,
                manifest,
                program: entry.program.clone(),
            });
        }
        (roster, None)
    }

    pub fn by_uid(&self, uid: u32) -> Option<&Member> {
        self.members.iter().find(|m| m.uid == uid)
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn manifest_text(id: &str, permissions: &str, requires: &str) -> String {
        format!(
            r#"{{"schema":1,"id":"{id}","name":"X","version":"1","api":{{"min":1,"target":1}},"run":{{"exe":"bin/x"}},"class":"S","requires":[{requires}],"permissions":[{permissions}]}}"#
        )
    }

    fn install(dir: &Path, id: &str, text: &str) -> String {
        std::fs::create_dir_all(dir.join(id)).unwrap();
        std::fs::write(dir.join(id).join("plugin.json"), text).unwrap();
        approval(&Manifest::parse(text.as_bytes()).unwrap())
    }

    fn enable(dir: &Path, entries: &[(&str, u32, &str)]) {
        let list: Vec<String> = entries
            .iter()
            .map(|(id, uid, approve)| {
                format!(r#"{{"id":"{id}","uid":{uid},"approve":"{approve}"}}"#)
            })
            .collect();
        std::fs::write(
            dir.join("enabled.json"),
            format!(r#"{{"plugins":[{}]}}"#, list.join(",")),
        )
        .unwrap();
    }

    #[test]
    fn the_home_assistant_lease_is_served_but_never_a_call_on_what_is_the_owners_to_operate() {
        let ha = r#""settings","cards","ha.read:areas,light,lock""#;
        let on = |permissions: &str, requires: &str| {
            let dir = tempfile::tempdir().unwrap();
            let text = manifest_text("a-one", permissions, requires);
            let approve = install(dir.path(), "a-one", &text);
            enable(
                dir.path(),
                &[("a-one", plugin_api::PLUGIN_UID_BASE, &approve)],
            );
            Roster::load(dir.path(), plugin_api::PLUGIN_UID_BASE).0
        };
        // Reading a lock's state is allowed; the lease and the occupancy signal are served.
        let roster = on(
            &format!("{ha},\"ha.call:light,scene\",\"signal.provide:occupancy\""),
            "\"lease:ha\"",
        );
        assert_eq!(roster.members.len(), 1, "{:?}", roster.inert);
        // The permissions of the lease need the lease: the plugin API holds that at the manifest.
        assert!(
            Manifest::parse(manifest_text("a-one", ha, "").as_bytes()).is_err(),
            "ha.read without lease:ha"
        );
        // Calls on a door, an alarm, a camera, a garage or water are never served, whatever the plugin asks. The plugin API itself
        // now refuses every domain outside its callable list (a camera, a valve) at the manifest; a cover parses there and this TV
        // refuses it.
        for domain in ["camera", "valve"] {
            assert!(
                Manifest::parse(
                    manifest_text(
                        "a-one",
                        &format!("{ha},\"ha.call:light,{domain}\""),
                        "\"lease:ha\""
                    )
                    .as_bytes()
                )
                .is_err(),
                "ha.call:{domain} parses"
            );
        }
        let roster = on(&format!("{ha},\"ha.call:light,cover\""), "\"lease:ha\"");
        let why = &roster.inert[0].reason;
        assert!(
            why.contains("ha.call:cover") && why.contains("never lets a plugin call"),
            "{why}"
        );
        // A lock parses at the plugin API (the Show may call one with a gesture token); this TV never serves the call.
        let roster = on(&format!("{ha},\"ha.call:lock\""), "\"lease:ha\"");
        assert!(
            roster.inert[0].reason.contains("ha.call:lock"),
            "{:?}",
            roster.inert
        );
        // Other leases and the network are still not served; a permission of a later level (the LAN, level 2) is not granted at level 1
        // and makes nothing inert.
        let roster = on(r#""settings","net.mdns""#, "");
        assert!(
            roster.inert[0].reason.contains("net.mdns"),
            "{:?}",
            roster.inert
        );
        let roster = on(r#""settings","net.lan:192.168.1.5:80""#, "");
        assert_eq!((roster.members.len(), roster.inert.len()), (1, 0));
        let roster = on(r#""settings""#, r#""lease:camera""#);
        assert!(
            roster.inert[0].reason.contains("lease:camera"),
            "{:?}",
            roster.inert
        );
    }

    #[test]
    fn a_missing_file_is_an_empty_roster_and_a_damaged_one_switches_nothing_on() {
        let dir = tempfile::tempdir().unwrap();
        let (roster, problem) = Roster::load(dir.path(), plugin_api::PLUGIN_UID_BASE);
        assert!(roster.is_empty() && problem.is_none());
        std::fs::write(dir.path().join("enabled.json"), "{not json").unwrap();
        let (roster, problem) = Roster::load(dir.path(), plugin_api::PLUGIN_UID_BASE);
        assert!(roster.is_empty());
        assert!(problem.unwrap().starts_with("enabled.json"));
        std::fs::write(
            dir.path().join("enabled.json"),
            r#"{"plugins":[],"extra":1}"#,
        )
        .unwrap();
        assert!(
            Roster::load(dir.path(), plugin_api::PLUGIN_UID_BASE)
                .1
                .is_some(),
            "an unknown key is an error"
        );
    }

    #[test]
    fn only_approved_unchanged_servable_plugins_in_the_uid_range_are_members() {
        let dir = tempfile::tempdir().unwrap();
        let ok = install(
            dir.path(),
            "a-one",
            &manifest_text("a-one", r#""settings","cards""#, ""),
        );
        let changed = install(
            dir.path(),
            "b-two",
            &manifest_text("b-two", r#""cards""#, ""),
        );
        let net = install(
            dir.path(),
            "c-three",
            &manifest_text("c-three", r#""cards","net.mdns""#, ""),
        );
        let lease = install(
            dir.path(),
            "d-four",
            &manifest_text("d-four", r#""cards""#, r#""lease:camera""#),
        );
        let other = install(
            dir.path(),
            "e-five",
            &manifest_text("e-five", r#""cards""#, ""),
        );
        let later = install(
            dir.path(),
            "f-six",
            &manifest_text("f-six", r#""cards""#, ""),
        );
        // b-two is edited after it was approved (a permission added).
        std::fs::write(
            dir.path().join("b-two").join("plugin.json"),
            manifest_text("b-two", r#""cards","tiles""#, ""),
        )
        .unwrap();
        // e-five's directory holds another plugin's manifest.
        std::fs::write(
            dir.path().join("e-five").join("plugin.json"),
            manifest_text("g-seven", r#""cards""#, ""),
        )
        .unwrap();
        enable(
            dir.path(),
            &[
                ("a-one", 20_000, &ok),
                ("b-two", 20_001, &changed),
                ("c-three", 20_002, &net),
                ("d-four", 20_003, &lease),
                ("e-five", 20_004, &other),
                ("f-six", 1000, &later),
                ("a-one", 20_006, &ok),
                ("Bad", 20_007, ""),
                ("missing", 20_008, ""),
            ],
        );
        let (roster, problem) = Roster::load(dir.path(), plugin_api::PLUGIN_UID_BASE);
        assert!(problem.is_none());
        assert_eq!(
            roster
                .members
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["a-one"]
        );
        assert_eq!(roster.by_uid(20_000).unwrap().id, "a-one");
        for refused in [20_001, 20_002, 1000, 0, u32::MAX] {
            assert!(roster.by_uid(refused).is_none(), "{refused}");
        }
        let why = |id: &str| {
            roster
                .inert
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.reason.clone())
                .unwrap_or_default()
        };
        assert!(
            why("b-two").contains("not the one that was approved"),
            "{}",
            why("b-two")
        );
        assert!(why("c-three").contains("does not serve") && why("c-three").contains("net.mdns"));
        assert!(why("d-four").contains("lease:camera"));
        assert!(why("e-five").contains("another plugin"));
        assert!(why("f-six").contains("outside the plugin range"));
        assert!(roster.inert.iter().any(|i| i.reason == "listed twice"));
        assert!(why("Bad").contains("not a plugin id"));
        assert!(why("missing").contains("plugin.json"));
    }

    #[test]
    fn a_new_display_name_or_version_keeps_the_approval_and_a_new_program_loses_it() {
        let a = Manifest::parse(manifest_text("hello", r#""cards""#, "").as_bytes()).unwrap();
        let renamed = Manifest::parse(
            manifest_text("hello", r#""cards""#, "")
                .replace(r#""name":"X""#, r#""name":"Renamed""#)
                .replace(r#""version":"1""#, r#""version":"2""#)
                .as_bytes(),
        )
        .unwrap();
        let other_program = Manifest::parse(
            manifest_text("hello", r#""cards""#, "")
                .replace("bin/x", "bin/y")
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(approval(&a), approval(&renamed));
        assert_ne!(approval(&a), approval(&other_program));
        assert_eq!(approval(&a).len(), 64);
    }
}
