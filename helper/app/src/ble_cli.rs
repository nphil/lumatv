//! `lumatv-helper ble-link` and `ble-unlink`: how the owner of the TV turns the Bluetooth proxy on, sets how much of the radio it
//! may use, and reads the key Home Assistant asks for.
//!
//! ```text
//! lumatv-helper ble-link --brand ... [--port <n>] [--name <node>] [--friendly-name <text>]
//!                        [--scan-seconds <5-30>] [--rest-seconds <window-600>] [--standby | --no-standby]
//!                        [--always | --no-always] [--rotate-key]
//! lumatv-helper ble-unlink --brand ...
//! ```
//!
//! Run again with other flags, `ble-link` changes only what is given and prints the same key (Home Assistant has to be given it once);
//! `--rotate-key` makes a new one (Home Assistant's connection then fails until it is given the new key). The helper looks at the
//! file every ten seconds, so nothing has to be told. Nothing in the helper writes this file: no route, no setting and no
//! assistant can turn the proxy on or change its key (Principle 0: this opens a port on the home network, so it is the owner's act).

use crate::ble_store::{self, BleLink};
use crate::config::{Config, Paths};
use crate::sys;
use esphome_api::base64;
use serde_json::{Value, json};
use std::slice::Iter;

/// What `ble-link` was given beyond `--port`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BleArgs {
    pub name: Option<String>,
    pub friendly_name: Option<String>,
    pub scan_seconds: Option<u32>,
    pub rest_seconds: Option<u32>,
    pub standby: Option<bool>,
    pub always: Option<bool>,
    pub rotate_key: bool,
}

/// Takes one of `ble-link`'s own flags (and its value) off the command line.
pub fn flag(args: &mut BleArgs, flag: &str, rest: &mut Iter<'_, String>) -> Result<(), String> {
    let mut value = |what: &str| {
        rest.next()
            .cloned()
            .ok_or_else(|| format!("{flag} needs {what}"))
    };
    let seconds = |text: String| {
        text.parse::<u32>()
            .map_err(|_| format!("{flag} needs a number of seconds"))
    };
    match flag {
        "--name" => args.name = Some(value("a node name")?),
        "--friendly-name" => args.friendly_name = Some(value("a name")?),
        "--scan-seconds" => args.scan_seconds = Some(seconds(value("seconds")?)?),
        "--rest-seconds" => args.rest_seconds = Some(seconds(value("seconds")?)?),
        "--standby" => args.standby = Some(true),
        "--no-standby" => args.standby = Some(false),
        "--always" => args.always = Some(true),
        "--no-always" => args.always = Some(false),
        "--rotate-key" => args.rotate_key = true,
        other => return Err(format!("unknown option {other}")),
    }
    Ok(())
}

/// The link after `args` are applied to the saved one (or to a new one with a key of the kernel's making), saved; and what to tell
/// the owner. `Err` is (exit code, why).
pub fn link_result(
    paths: &Paths,
    args: &BleArgs,
    port: Option<u16>,
) -> Result<Value, (i32, String)> {
    let saved = ble_store::load(&paths.data_dir).map_err(|e| (2, e.to_string()))?;
    let new_key = saved.is_none() || args.rotate_key;
    let fresh_key = || {
        let mut key = [0u8; 32];
        sys::random_bytes(&mut key).map_err(|e| (1, format!("no random numbers: {e}")))?;
        Ok::<_, (i32, String)>(key)
    };
    let mut link = match saved {
        Some(link) if !args.rotate_key => link,
        // A rotated key keeps the rest of what was set.
        Some(old) => BleLink {
            key: fresh_key()?,
            ..old
        },
        None => BleLink::with_key(fresh_key()?),
    };
    if let Some(name) = &args.name {
        link.name = name.clone();
    }
    if let Some(name) = &args.friendly_name {
        link.friendly_name = name.clone();
    }
    if let Some(port) = port {
        link.port = port;
    }
    if let Some(n) = args.scan_seconds {
        link.scan_seconds = n;
    }
    if let Some(n) = args.rest_seconds {
        link.rest_seconds = n;
    }
    if let Some(on) = args.standby {
        link.standby = on;
    }
    if let Some(on) = args.always {
        link.always = on;
    }
    let link = link.check().map_err(|e| (2, e.to_string()))?;
    ble_store::save(&paths.data_dir, &link).map_err(|e| (1, e.to_string()))?;
    let host = sys::lan_ipv4(None).map(|ip| ip.to_string());
    Ok(json!({
        "ok": true,
        "linked": true,
        "name": link.name,
        "friendlyName": link.friendly_name,
        "port": link.port,
        "key": link.key_text(),
        "keyIsNew": new_key,
        "scanSeconds": link.scan_seconds,
        "restSeconds": link.rest_seconds,
        "dutyPercent": link.duty_percent(),
        "standbyScanning": link.standby,
        "always": link.always,
        "host": host,
        "next": "the helper takes this up within ten seconds. In Home Assistant: Settings, Devices & services, Add integration, ESPHome (it may appear as discovered); host and port as above, then the key",
    }))
}

/// `ble-link`.
pub fn link(paths: &Paths, args: &BleArgs, port: Option<u16>) -> i32 {
    match link_result(paths, args, port) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err((code, why)) => {
            println!("{}", json!({"ok": false, "error": why}));
            code
        }
    }
}

/// `ble-unlink`: the link and its key are removed; the helper closes the socket and ends any scan within ten seconds.
pub fn unlink(paths: &Paths, _config: &Config) -> i32 {
    let removed = ble_store::clear(&paths.data_dir);
    println!(
        "{}",
        json!({"ok": true, "linked": false, "removed": removed, "next": "remove the device in Home Assistant (ESPHome); the helper closes the socket within ten seconds"})
    );
    0
}

/// Whether `text` is a key in the form Home Assistant takes (for tests and checks).
pub fn is_key(text: &str) -> bool {
    base64::decode_key(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<BleArgs, String> {
        let words: Vec<String> = words.iter().map(|s| (*s).to_owned()).collect();
        let mut args = BleArgs::default();
        let mut it = words.iter();
        while let Some(w) = it.next() {
            flag(&mut args, w, &mut it)?;
        }
        Ok(args)
    }

    #[test]
    fn the_flags_are_read() {
        let a = parse(&[
            "--name",
            "tv-ble",
            "--scan-seconds",
            "15",
            "--rest-seconds",
            "45",
            "--standby",
            "--always",
        ])
        .unwrap();
        assert_eq!(a.name.as_deref(), Some("tv-ble"));
        assert_eq!((a.scan_seconds, a.rest_seconds), (Some(15), Some(45)));
        assert_eq!(
            (a.standby, a.always, a.rotate_key),
            (Some(true), Some(true), false)
        );
        assert!(parse(&["--scan-seconds", "x"]).is_err());
        assert!(parse(&["--name"]).is_err());
        assert_eq!(parse(&["--no-standby"]).unwrap().standby, Some(false));
    }

    #[test]
    fn a_first_link_makes_a_key_and_a_second_run_shows_the_same_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let first = link_result(&paths, &BleArgs::default(), None).unwrap();
        assert_eq!(first["keyIsNew"], true);
        let key = first["key"].as_str().unwrap().to_owned();
        assert!(is_key(&key));
        assert_eq!(
            (first["port"].as_u64(), first["dutyPercent"].as_u64()),
            (Some(6053), Some(16))
        );
        // Changing the window keeps the key.
        let again = link_result(
            &paths,
            &BleArgs {
                scan_seconds: Some(20),
                rest_seconds: Some(40),
                standby: Some(true),
                ..Default::default()
            },
            Some(6054),
        )
        .unwrap();
        assert_eq!(
            (again["key"].as_str(), again["keyIsNew"].as_bool()),
            (Some(key.as_str()), Some(false))
        );
        assert_eq!(
            (
                again["scanSeconds"].as_u64(),
                again["restSeconds"].as_u64(),
                again["port"].as_u64(),
                again["standbyScanning"].as_bool()
            ),
            (Some(20), Some(40), Some(6054), Some(true))
        );
        // A rotated key is new and the settings stay.
        let rotated = link_result(
            &paths,
            &BleArgs {
                rotate_key: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_ne!(rotated["key"].as_str(), Some(key.as_str()));
        assert_eq!(rotated["keyIsNew"], true);
        assert_eq!(
            (
                rotated["scanSeconds"].as_u64(),
                rotated["standbyScanning"].as_bool()
            ),
            (Some(20), Some(true))
        );
        let saved = ble_store::load(dir.path()).unwrap().unwrap();
        assert_eq!(saved.key_text(), rotated["key"].as_str().unwrap());
    }

    #[test]
    fn what_the_radio_may_be_asked_is_checked_before_anything_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        for args in [
            BleArgs {
                scan_seconds: Some(60),
                ..Default::default()
            },
            BleArgs {
                scan_seconds: Some(30),
                rest_seconds: Some(10),
                ..Default::default()
            },
            BleArgs {
                rest_seconds: Some(5000),
                ..Default::default()
            },
            BleArgs {
                name: Some("Not Valid".into()),
                ..Default::default()
            },
        ] {
            let (code, _) = link_result(&paths, &args, None).unwrap_err();
            assert_eq!(code, 2);
        }
        assert_eq!(
            link_result(&paths, &BleArgs::default(), Some(80))
                .unwrap_err()
                .0,
            2
        );
        assert_eq!(ble_store::load(dir.path()), Ok(None), "nothing was saved");
    }

    #[test]
    fn unlinking_removes_the_link_and_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        link_result(&paths, &BleArgs::default(), None).unwrap();
        assert_eq!(unlink(&paths, &Config::default()), 0);
        assert_eq!(ble_store::load(dir.path()), Ok(None));
    }
}
