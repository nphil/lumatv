//! Switching a plugin on and off, and linking Home Assistant: the owner's acts, done from the command line.
//!
//! Installing software on the TV and giving it a key are things only the owner decides (Principle 0): no route of the helper does
//! either, and nothing in the helper switches a plugin on by itself. (The helper's own plugins that ask first are switched in Settings,
//! `routes.rs`: that installs nothing.) The owner runs these at a shell on the TV:
//!
//! | Command | What |
//! |---|---|
//! | `plugin-consent --id <id>` | what the packaged plugin may do, in plain words, and the approval code of exactly that |
//! | `plugin-enable --id <id> --approve <code>` | installs the package and switches the plugin on (the code must be the one consent printed) |
//! | `plugin-disable --id <id>` | switches it off (the program is ended; its files and settings stay); also one of the helper's own |
//! | `plugin-status` | which plugins are on |
//! | `ha-link --url <address>` | saves Home Assistant's address and an access token (read from standard input) for the plugins' lease |
//! | `ha-unlink` | forgets them |
//!
//! The package is a directory (`plugin.json` and the program it names), by default the one the ipk carries at
//! `<app dir>/payload/plugins/<id>`. `plugin-enable` copies it to `<data dir>/plugins/<id>`, owned by the user that runs it (root on
//! the TV) and writable by nobody else, gives the plugin the next free user id of the plugins' range, and writes its entry in
//! `enabled.json` with the approval code and the hash of the program it installed. A running helper is told to read the file again.

use super::roster::{self, Enabled, approval};
use crate::config::{Config, Paths};
use plugin_api::Manifest;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What a plugin package may weigh, so that copying one cannot fill the TV.
const MAX_FILES: usize = 64;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// A plugin package as it sits on disk before it is installed.
#[derive(Debug)]
pub struct Package {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// What a command line ended with: the exit code and the one JSON object to print.
pub type Outcome = (i32, Value);

fn failed(code: i32, why: impl Into<String>) -> Outcome {
    (code, json!({"ok": false, "error": why.into()}))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads and checks a package: its manifest, and that the program the manifest names is there. A package this TV would not run is
/// refused, in the words the roster would give.
pub fn read_package(dir: &Path) -> Result<Package, String> {
    let file = dir.join("plugin.json");
    let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let manifest = Manifest::parse(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    if let Some(why) = roster::refusal(&manifest) {
        return Err(format!(
            "{} is not a plugin this TV runs: {why}",
            manifest.id
        ));
    }
    let program = dir.join(&manifest.run.exe);
    if !program.is_file() {
        return Err(format!(
            "{} names the program {}, which is not in the package",
            file.display(),
            manifest.run.exe
        ));
    }
    Ok(Package {
        dir: dir.to_owned(),
        manifest,
    })
}

fn program_hash(package: &Package) -> Result<String, String> {
    let path = package.dir.join(&package.manifest.run.exe);
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|f| f.take(MAX_BYTES).read_to_end(&mut bytes))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

/// What the owner is told a plugin may do, in plain words: the manifest's own permissions, and what this TV holds it to whatever
/// it asks.
pub fn consent(package: &Package) -> Value {
    let m = &package.manifest;
    let tv = |text: String| {
        text.replace("LumaShow", "this TV")
            .replace("the Show", "this TV")
    };
    let mut may: Vec<String> = Vec::new();
    if m.requires.contains(&plugin_api::Lease::Ha) {
        may.push(
            "Use this TV's connection to Home Assistant (the TV keeps the address and the access token; the plugin never sees them)"
                .to_owned(),
        );
    }
    may.extend(m.permission_order.iter().map(|p| tv(p.describe())));
    let mut never = vec![
        "Reach the network, the internet or any device by itself: it talks only to this TV"
            .to_owned(),
    ];
    if m.requires.contains(&plugin_api::Lease::Ha) {
        never.push(
            "Open or lock a door, work an alarm, a siren, a camera, a garage door or a water valve, or run a script or an automation in Home Assistant: this TV does not let a plugin do that, whatever it asks"
                .to_owned(),
        );
    }
    json!({
        "ok": true,
        "id": m.id,
        "name": m.name,
        "version": m.version,
        "class": m.class.name(),
        "mayDo": may,
        "neverDoes": never,
        "caution": "A plugin is a separate program that runs as a user of its own. This TV cannot box it in as the Show does (its kernel has no seccomp and no namespaces), so switch on only a plugin you trust.",
        "approve": approval(m),
        "program": program_hash(package).ok(),
    })
}

/// Copies a package's files to `to`, each through a temporary file and a rename (the program of a plugin that is running cannot be
/// written to, but a name can be replaced), with the modes an installed plugin has: nobody but the owner can write anything.
fn copy_package(from: &Path, to: &Path) -> Result<(), String> {
    fn walk(
        from: &Path,
        to: &Path,
        count: &mut usize,
        bytes: &mut u64,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 4 {
            return Err("the package is nested too deep".into());
        }
        std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", to.display()))?;
        let mut names: Vec<_> = std::fs::read_dir(from)
            .map_err(|e| format!("{}: {e}", from.display()))?
            .filter_map(Result::ok)
            .collect();
        names.sort_by_key(std::fs::DirEntry::file_name);
        for entry in names {
            let name = entry.file_name();
            // The plugin's own data and log are not part of a package.
            if depth == 0 && (name == "data" || name == "log" || name == "log.1") {
                continue;
            }
            let meta = std::fs::symlink_metadata(entry.path())
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
            if meta.is_dir() {
                walk(&entry.path(), &to.join(&name), count, bytes, depth + 1)?;
            } else if meta.is_file() {
                *count += 1;
                *bytes += meta.len();
                if *count > MAX_FILES || *bytes > MAX_BYTES {
                    return Err("the package is too big".into());
                }
                let dest = to.join(&name);
                let tmp = to.join(format!(".{}.new", name.to_string_lossy()));
                std::fs::copy(entry.path(), &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
                let mode = if meta.permissions().mode() & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                };
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
                    .map_err(|e| format!("{}: {e}", tmp.display()))?;
                std::fs::rename(&tmp, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            } else {
                return Err(format!(
                    "{} is not a plain file or directory (a package has no links)",
                    entry.path().display()
                ));
            }
        }
        Ok(())
    }
    walk(from, to, &mut 0, &mut 0, 0)
}

/// Where the ipk carries a plugin's package: `<app dir>/payload/plugins/<id>`, the app dir being the brand file's.
pub fn packaged(brand: &Path, id: &str) -> PathBuf {
    brand
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("payload")
        .join("plugins")
        .join(id)
}

/// `plugin-consent`.
pub fn run_consent(from: &Path) -> Outcome {
    match read_package(from) {
        Ok(package) => {
            let mut doc = consent(&package);
            doc["then"] = json!(format!(
                "to switch it on: lumatv-helper plugin-enable --id {} --approve {}",
                package.manifest.id,
                approval(&package.manifest)
            ));
            (0, doc)
        }
        Err(why) => failed(2, why),
    }
}

/// `plugin-enable`: installs the package at `from` and switches the plugin on, if `approve` is the code of exactly what it asks.
pub fn run_enable(paths: &Paths, config: &Config, from: &Path, approve: &str) -> Outcome {
    let package = match read_package(from) {
        Ok(package) => package,
        Err(why) => return failed(2, why),
    };
    let id = package.manifest.id.clone();
    let want = approval(&package.manifest);
    if !approve.eq_ignore_ascii_case(&want) {
        let mut doc = consent(&package);
        doc["ok"] = json!(false);
        doc["error"] = json!(
            "the approval code is not the one of what this plugin asks: read what it may do (below), and if it is what you want, run the command with its code"
        );
        return (2, doc);
    }
    if super::local::FIRST_PARTY_IDS.contains(&id.as_str()) {
        return failed(2, "that name belongs to a plugin of the helper's own");
    }
    let program = match program_hash(&package) {
        Ok(hash) => hash,
        Err(why) => return failed(1, why),
    };
    let mut enabled = match roster::read_file(&paths.plugins_dir()) {
        Ok(enabled) => enabled,
        Err(why) => return failed(1, why),
    };
    let entries = &mut enabled.plugins;
    let first = config.plugins.first_uid;
    let taken = |uid: u32| entries.iter().any(|e| e.id != id && e.uid == uid);
    let uid = entries
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.uid)
        .or_else(|| (first..first + plugin_api::MAX_PLUGINS).find(|uid| !taken(*uid)));
    let Some(uid) = uid else {
        return failed(1, "no user id of the plugins' range is free");
    };
    if let Err(why) = copy_package(&package.dir, &paths.plugins_dir().join(&id)) {
        return failed(1, why);
    }
    entries.retain(|e| e.id != id);
    entries.push(Enabled {
        id: id.clone(),
        uid,
        approve: want.clone(),
        program: Some(program.clone()),
    });
    if let Err(why) = roster::write_file(&paths.plugins_dir(), &enabled) {
        return failed(1, why);
    }
    let told = ask_helper(paths, config, "POST", "/v1/plugins/rescan").is_some();
    (
        0,
        json!({"ok": true, "enabled": true, "id": id, "uid": uid, "approve": want, "program": program, "helperTold": told,
               "next": "the plugin starts within a moment of the helper reading this; link Home Assistant with ha-link if it needs it"}),
    )
}

/// `plugin-disable`: switches a plugin off (one of the helper's own that asks first, too). Its program is ended by the helper; its files and
/// settings stay.
pub fn run_disable(paths: &Paths, config: &Config, id: &str) -> Outcome {
    let mut enabled = match roster::read_file(&paths.plugins_dir()) {
        Ok(enabled) => enabled,
        Err(why) => return failed(1, why),
    };
    let before = enabled.plugins.len() + enabled.first_party.len();
    enabled.plugins.retain(|e| e.id != id);
    enabled.first_party.retain(|a| a.id != id);
    let was_on = enabled.plugins.len() + enabled.first_party.len() != before;
    if was_on && let Err(why) = roster::write_file(&paths.plugins_dir(), &enabled) {
        return failed(1, why);
    }
    let told = ask_helper(paths, config, "POST", "/v1/plugins/rescan").is_some();
    (
        0,
        json!({"ok": true, "enabled": false, "id": id, "wasOn": was_on, "helperTold": told}),
    )
}

/// `plugin-status`: the plugins that are switched on, and what a running helper says of them.
pub fn run_status(paths: &Paths, config: &Config) -> Outcome {
    let enabled = match roster::read_file(&paths.plugins_dir()) {
        Ok(enabled) => enabled,
        Err(why) => return failed(1, why),
    };
    let running = ask_helper(paths, config, "GET", "/v1/plugins").map(|(_, doc)| doc);
    (
        0,
        json!({"ok": true,
               "enabled": enabled.plugins.iter().map(|e| json!({"id": e.id, "uid": e.uid, "program": e.program})).collect::<Vec<_>>(),
               "firstParty": enabled.first_party.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
               "helper": running}),
    )
}

/// `ha-link`: saves Home Assistant's address and the access token the owner gave for the plugins' lease, and tells a running helper.
pub fn run_ha_link(paths: &Paths, config: &Config, url: Option<&str>, token: &str) -> Outcome {
    let Some(url) = url else {
        return failed(2, "ha-link needs --url <the address of Home Assistant>");
    };
    let link = match crate::ha_store::save(&paths.ha_link_file(), url, token) {
        Ok(link) => link,
        Err(e) => return failed(2, e.to_string()),
    };
    let told = ask_helper(paths, config, "POST", "/v1/plugins/lease/ha/refresh").is_some();
    // What Home Assistant says of the token is the answer the owner wants: wait a moment for it.
    let lease = if told {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let doc = ask_helper(paths, config, "GET", "/v1/state/ha.link")
                .map(|(_, d)| d["value"].clone());
            let state = doc.as_ref().and_then(|d| d["state"].as_str()).unwrap_or("");
            if !matches!(state, "connecting" | "") || Instant::now() >= end {
                break doc;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    } else {
        None
    };
    let verdict = match lease.as_ref().and_then(|d| d["state"].as_str()) {
        Some("connected") => "Home Assistant took the token",
        Some("auth_failed") => {
            "Home Assistant did not accept the token: make another and run ha-link again"
        }
        Some("offline") => "Home Assistant could not be reached at that address",
        Some("off") => {
            "saved; the plugin is not switched on yet (plugin-enable), and takes the link when it is"
        }
        Some(_) | None if told => "saved; Home Assistant had not answered yet",
        Some(_) | None => "saved; the helper is not running, and reads it when it starts",
    };
    (
        0,
        json!({"ok": true, "linked": true, "url": link.endpoint.display(), "helperTold": told,
               "lease": lease.map(|d| json!({"state": d["state"], "detail": d["detail"]})), "verdict": verdict}),
    )
}

/// `ha-unlink`.
pub fn run_ha_unlink(paths: &Paths, config: &Config) -> Outcome {
    let removed = crate::ha_store::clear(&paths.ha_link_file());
    let told = ask_helper(paths, config, "POST", "/v1/plugins/lease/ha/refresh").is_some();
    (
        0,
        json!({"ok": true, "linked": false, "removed": removed, "helperTold": told}),
    )
}

/// Asks a running helper something over its loopback API: `(status, document)` of a 2xx answer. `None` when none runs, or it did
/// not answer, or said no.
pub fn ask_helper(
    paths: &Paths,
    config: &Config,
    method: &str,
    path: &str,
) -> Option<(u16, Value)> {
    // The lock is held only by a running helper.
    if !matches!(crate::sys::try_lock(&paths.lock_file()), Ok(None)) {
        return None;
    }
    let port = crate::sys::read_info(&paths.lock_file())
        .and_then(|i| i.port)
        .unwrap_or(config.listen.port);
    let ip: IpAddr = config.listen.host.parse().ok()?;
    let mut stream =
        TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(1)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let request = if method == "GET" {
        format!("GET {path} HTTP/1.0\r\nHost: {ip}:{port}\r\n\r\n")
    } else {
        format!(
            "{method} {path} HTTP/1.0\r\nHost: {ip}:{port}\r\nX-Luma-Client: cli\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        )
    };
    stream.write_all(request.as_bytes()).ok()?;
    let mut reply = Vec::new();
    let _ = stream.take(256 * 1024).read_to_end(&mut reply);
    let text = String::from_utf8_lossy(&reply).into_owned();
    let status: u16 = text.split_whitespace().nth(1)?.parse().ok()?;
    if !(200..300).contains(&status) {
        return None;
    }
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    Some((status, serde_json::from_str(body).unwrap_or(Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{"schema":1,"id":"home-assistant","name":"Home Assistant","version":"0.1.0","api":{"min":1,"target":1},"run":{"exe":"bin/home-assistant"},"class":"S","requires":["lease:ha"],"permissions":["settings","cards","tiles","notify","background","ha.read:areas,notifications,light,scene,event,binary_sensor","ha.call:light,scene","signal.provide:occupancy"],"limits":{"storage_mb":1,"flash_kb_per_day":64},"test":false}"#;

    fn package(dir: &Path, manifest: &str) -> PathBuf {
        let package = dir.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("plugin.json"), manifest).unwrap();
        std::fs::write(
            package.join("bin").join("home-assistant"),
            b"#!/bin/sh\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(
            package.join("bin").join("home-assistant"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        package
    }

    fn rig() -> (tempfile::TempDir, Paths, Config) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"));
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        (dir, paths, Config::parse("{}").unwrap())
    }

    fn code(manifest: &str) -> String {
        approval(&Manifest::parse(manifest.as_bytes()).unwrap())
    }

    #[test]
    fn consent_says_what_the_plugin_may_do_in_plain_words_and_what_it_never_does() {
        let dir = tempfile::tempdir().unwrap();
        let package = read_package(&package(dir.path(), MANIFEST)).unwrap();
        let doc = consent(&package);
        let may: Vec<&str> = doc["mayDo"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            may[0].starts_with("Use this TV's connection to Home Assistant"),
            "{may:?}"
        );
        for line in [
            "Have a page of its own in Settings",
            "Show cards on the home screen",
            "Read from Home Assistant: the rooms, Home Assistant's notifications, lights, scenes, events, such as a doorbell ring, sensors that are on or off",
            "Control in Home Assistant: lights, scenes",
            "Tell this TV whether somebody is in the room",
        ] {
            assert!(may.contains(&line), "{line}: {may:?}");
        }
        assert!(!may.iter().any(|l| l.contains("LumaShow")), "{may:?}");
        let never = doc["neverDoes"].to_string();
        assert!(
            never.contains("a door") && never.contains("camera") && never.contains("script"),
            "{never}"
        );
        assert_eq!(doc["approve"], code(MANIFEST));
        assert_eq!(doc["class"], "S");
        assert!(doc["caution"].as_str().unwrap().contains("seccomp"));
    }

    #[test]
    fn a_plugin_is_switched_on_only_with_the_code_of_what_it_asks() {
        let (dir, paths, config) = rig();
        let from = package(dir.path(), MANIFEST);
        let (status, doc) = run_enable(&paths, &config, &from, "not-the-code");
        assert_eq!(status, 2);
        assert_eq!(doc["ok"], false);
        assert_eq!(
            doc["approve"],
            code(MANIFEST),
            "it says what the code is, with what the plugin asks"
        );
        assert!(!paths.plugins_dir().join("enabled.json").exists());
        assert!(
            !paths.plugins_dir().join("home-assistant").exists(),
            "nothing was installed"
        );

        let (status, doc) = run_enable(&paths, &config, &from, &code(MANIFEST).to_uppercase());
        assert_eq!(status, 0, "{doc}");
        assert_eq!(doc["uid"], 20000);
        assert_eq!(doc["helperTold"], false, "no helper runs here");
        let listed: Value = serde_json::from_slice(
            &std::fs::read(paths.plugins_dir().join("enabled.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(listed["plugins"][0]["id"], "home-assistant");
        assert_eq!(listed["plugins"][0]["approve"], code(MANIFEST));
        assert_eq!(listed["plugins"][0]["program"], doc["program"]);
        // The roster the helper reads accepts it as the owner's.
        let (roster, problem) = roster::Roster::load(&paths.plugins_dir(), 20000);
        assert!(problem.is_none() && roster.inert.is_empty(), "{roster:?}");
        assert_eq!(roster.members.len(), 1);
        assert_eq!(
            roster.members[0].program.as_deref(),
            doc["program"].as_str()
        );
        let installed = paths
            .plugins_dir()
            .join("home-assistant")
            .join("bin")
            .join("home-assistant");
        assert_eq!(
            std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(
                paths
                    .plugins_dir()
                    .join("home-assistant")
                    .join("plugin.json")
            )
            .unwrap()
            .permissions()
            .mode()
                & 0o777,
            0o644
        );
    }

    #[test]
    fn switching_on_again_keeps_its_user_and_other_plugins_keep_theirs() {
        let (dir, paths, config) = rig();
        let tuner = roster::Approved {
            id: "live-tv-tuner".into(),
            approve: "c0de".into(),
        };
        roster::write_file(
            &paths.plugins_dir(),
            &roster::EnabledFile {
                plugins: Vec::new(),
                first_party: vec![tuner.clone()],
            },
        )
        .unwrap();
        let from = package(dir.path(), MANIFEST);
        let other = MANIFEST.replace("home-assistant", "other-one");
        let other_from = dir.path().join("other");
        std::fs::create_dir_all(other_from.join("bin")).unwrap();
        std::fs::write(other_from.join("plugin.json"), &other).unwrap();
        std::fs::write(other_from.join("bin").join("other-one"), b"x").unwrap();
        assert_eq!(
            run_enable(&paths, &config, &from, &code(MANIFEST)).1["uid"],
            20000
        );
        assert_eq!(
            run_enable(&paths, &config, &other_from, &code(&other)).1["uid"],
            20001
        );
        assert_eq!(
            run_enable(&paths, &config, &from, &code(MANIFEST)).1["uid"],
            20000,
            "the same user as before"
        );
        let listed = roster::read_file(&paths.plugins_dir()).unwrap();
        assert_eq!(
            listed
                .plugins
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["other-one", "home-assistant"]
        );
        assert_eq!(
            listed.first_party,
            [tuner],
            "a plugin of the helper's own switched on in Settings stays on"
        );
        // Off: the entry goes, the files stay, the other plugin stays on.
        let (status, doc) = run_disable(&paths, &config, "home-assistant");
        assert_eq!((status, doc["wasOn"].clone()), (0, json!(true)));
        assert_eq!(
            roster::read_file(&paths.plugins_dir())
                .unwrap()
                .plugins
                .len(),
            1
        );
        assert!(
            paths
                .plugins_dir()
                .join("home-assistant")
                .join("plugin.json")
                .is_file()
        );
        assert_eq!(
            run_disable(&paths, &config, "home-assistant").1["wasOn"],
            false
        );
    }

    #[test]
    fn a_plugin_this_tv_would_not_run_is_not_installed_and_a_damaged_list_is_not_overwritten() {
        let (dir, paths, config) = rig();
        let door = MANIFEST.replace("ha.call:light,scene", "ha.call:light,cover");
        let from = package(dir.path(), &door);
        let (status, doc) = run_enable(&paths, &config, &from, &code(&door));
        assert_eq!(status, 2);
        assert!(
            doc["error"]
                .as_str()
                .unwrap()
                .contains("never lets a plugin call"),
            "{doc}"
        );
        assert!(!paths.plugins_dir().join("home-assistant").exists());

        let good = package(dir.path().join("two").as_path(), MANIFEST);
        std::fs::create_dir_all(paths.plugins_dir()).unwrap();
        std::fs::write(paths.plugins_dir().join("enabled.json"), "{not json").unwrap();
        let (status, doc) = run_enable(&paths, &config, &good, &code(MANIFEST));
        assert_eq!(status, 1);
        assert!(doc["error"].as_str().unwrap().contains("damaged"), "{doc}");
        assert_eq!(
            std::fs::read_to_string(paths.plugins_dir().join("enabled.json")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn a_package_with_a_link_or_without_its_program_is_refused() {
        let (dir, paths, config) = rig();
        let from = package(dir.path(), MANIFEST);
        std::os::unix::fs::symlink("/etc/passwd", from.join("bin").join("link")).unwrap();
        let (status, doc) = run_enable(&paths, &config, &from, &code(MANIFEST));
        assert_eq!(status, 1, "{doc}");
        assert!(doc["error"].as_str().unwrap().contains("links"), "{doc}");
        std::fs::remove_file(from.join("bin").join("link")).unwrap();
        std::fs::remove_file(from.join("bin").join("home-assistant")).unwrap();
        assert!(
            read_package(&from)
                .unwrap_err()
                .contains("not in the package")
        );
    }

    #[test]
    fn the_link_is_saved_from_the_owners_input_and_the_token_is_never_printed() {
        let (_dir, paths, config) = rig();
        const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.printed-nowhere.signature";
        let (status, doc) = run_ha_link(
            &paths,
            &config,
            Some("192.168.1.146"),
            &format!("{TOKEN}\n"),
        );
        assert_eq!(status, 0, "{doc}");
        assert_eq!(doc["url"], "http://192.168.1.146:8123");
        assert!(!doc.to_string().contains(TOKEN));
        assert_eq!(doc["helperTold"], false);
        assert!(
            doc["verdict"]
                .as_str()
                .unwrap()
                .contains("helper is not running")
        );
        assert_eq!(
            crate::ha_store::load(&paths.ha_link_file())
                .unwrap()
                .unwrap()
                .token
                .expose(),
            TOKEN
        );
        assert_eq!(run_ha_link(&paths, &config, None, TOKEN).0, 2);
        let (status, doc) = run_ha_link(&paths, &config, Some("http://192.168.1.146"), "two words");
        assert_eq!(status, 2);
        assert!(!doc.to_string().contains("two words"));
        assert_eq!(run_ha_unlink(&paths, &config).1["removed"], true);
        assert!(!paths.ha_link_file().exists());
    }
}
