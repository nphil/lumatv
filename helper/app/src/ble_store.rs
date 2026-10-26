//! Where the Bluetooth proxy keeps what the owner set: the node's name, the encryption key Home Assistant is given, the port, and
//! how the radio is shared with Wi-Fi.
//!
//! `<data dir>/ble/proxy.json`, mode 0600: `{"name", "friendlyName", "key", "port", "scanSeconds", "restSeconds", "standby",
//! "always"}`. It is written by `lumatv-helper ble-link` and removed by `ble-unlink`; the owner of the TV runs those, nothing in
//! the helper does, and no route (the settings API included) can change it. **No file is the off state**: nothing listens, nothing
//! is announced and the radio is never asked to scan. The key is shown by `ble-link` and `ble-key` (Home Assistant asks for it
//! once) and is never logged, never put in a topic or an API answer; `Debug` says nothing of it.

use crate::hue_store::{StoreError, write_private};
use esphome_api::base64;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// ESPHome's own port: what Home Assistant offers first when a device is added by address.
pub const DEFAULT_PORT: u16 = 6053;
pub const DEFAULT_NAME: &str = "lumatv-ble";
pub const DEFAULT_FRIENDLY_NAME: &str = "LumaTV Bluetooth";

/// The radio is shared with Wi-Fi (one MediaTek module, one antenna), so what may be asked of it is bounded here, not by a
/// setting that can be made too generous: a window of 5 to 30 seconds, then a rest at least as long as the window (a duty of 50 %
/// at the most) and at most ten minutes.
pub const MIN_SCAN_SECONDS: u32 = 5;
pub const MAX_SCAN_SECONDS: u32 = 30;
pub const MAX_REST_SECONDS: u32 = 600;
pub const DEFAULT_SCAN_SECONDS: u32 = 10;
pub const DEFAULT_REST_SECONDS: u32 = 50;

/// `<data dir>/ble/proxy.json`.
pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("ble").join("proxy.json")
}

#[derive(Clone, PartialEq, Eq)]
pub struct BleLink {
    /// The node name: what the mDNS announcement says, what the API says, what Home Assistant calls the device.
    pub name: String,
    pub friendly_name: String,
    pub key: [u8; 32],
    pub port: u16,
    /// How long one scan window lasts, and how long the radio rests before the next.
    pub scan_seconds: u32,
    pub rest_seconds: u32,
    /// Scan while the panel is off too (the TV in standby). Off: the radio is left alone whenever the screen is dark.
    pub standby: bool,
    /// Ask LG's service to report every advertisement, not only a device that is new or changed (`always` of `le/startScan`).
    pub always: bool,
}

impl fmt::Debug for BleLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BleLink({:?}, port {}, {} s on / {} s off, standby {}, always {}, key <redacted>)",
            self.name, self.port, self.scan_seconds, self.rest_seconds, self.standby, self.always
        )
    }
}

/// A node name ESPHome and DNS accept: lower-case letters, digits and hyphens, not starting or ending with a hyphen, at most 31.
pub fn valid_name(name: &str) -> bool {
    (1..=31).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

impl BleLink {
    /// A link with the defaults and a key made from `key`.
    pub fn with_key(key: [u8; 32]) -> Self {
        Self {
            name: DEFAULT_NAME.to_owned(),
            friendly_name: DEFAULT_FRIENDLY_NAME.to_owned(),
            key,
            port: DEFAULT_PORT,
            scan_seconds: DEFAULT_SCAN_SECONDS,
            rest_seconds: DEFAULT_REST_SECONDS,
            standby: false,
            always: false,
        }
    }

    /// Checks what a person gave.
    pub fn check(self) -> Result<Self, StoreError> {
        let bad = |why: String| Err(StoreError(why));
        if !valid_name(&self.name) {
            return bad(format!(
                "the name is 1 to 31 lower-case letters, digits or hyphens, not starting or ending with a hyphen: {:?}",
                self.name
            ));
        }
        if self.friendly_name.is_empty()
            || self.friendly_name.chars().count() > 64
            || self.friendly_name.chars().any(char::is_control)
        {
            return bad("the friendly name is 1 to 64 characters".to_owned());
        }
        if self.port < 1024 {
            return bad("the port is 1024 or higher".to_owned());
        }
        if !(MIN_SCAN_SECONDS..=MAX_SCAN_SECONDS).contains(&self.scan_seconds) {
            return bad(format!(
                "a scan window is {MIN_SCAN_SECONDS} to {MAX_SCAN_SECONDS} seconds"
            ));
        }
        if self.rest_seconds < self.scan_seconds || self.rest_seconds > MAX_REST_SECONDS {
            return bad(format!(
                "the rest between windows is at least the window ({} s) and at most {MAX_REST_SECONDS} seconds: the radio is shared with Wi-Fi and is never asked for more than half the time",
                self.scan_seconds
            ));
        }
        Ok(self)
    }

    /// The key as Home Assistant is given it (base64 of the 32 bytes).
    pub fn key_text(&self) -> String {
        base64::encode(&self.key)
    }

    /// The share of time the radio is asked to scan while it is allowed to.
    pub fn duty_percent(&self) -> u32 {
        self.scan_seconds * 100 / (self.scan_seconds + self.rest_seconds)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Stored {
    name: String,
    friendly_name: String,
    key: String,
    port: u16,
    scan_seconds: u32,
    rest_seconds: u32,
    standby: bool,
    always: bool,
}

/// Saves the link; the file is replaced whole.
pub fn save(data_dir: &Path, link: &BleLink) -> Result<(), StoreError> {
    let folder = data_dir.join("ble");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&folder)
        .map_err(|e| StoreError(format!("{}: {e}", folder.display())))?;
    let stored = Stored {
        name: link.name.clone(),
        friendly_name: link.friendly_name.clone(),
        key: link.key_text(),
        port: link.port,
        scan_seconds: link.scan_seconds,
        rest_seconds: link.rest_seconds,
        standby: link.standby,
        always: link.always,
    };
    let bytes = serde_json::to_vec(&stored).map_err(|e| StoreError(e.to_string()))?;
    let path = file(data_dir);
    write_private(&path, &bytes, 0o600).map_err(|e| StoreError(format!("{}: {e}", path.display())))
}

/// The saved link: `Ok(None)` when there is none, an error (without the key) when the file is there and cannot be used.
pub fn load(data_dir: &Path) -> Result<Option<BleLink>, StoreError> {
    let path = file(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError(format!("{}: {e}", path.display()))),
    };
    let unusable = || {
        StoreError(format!(
            "{} is not a saved Bluetooth proxy link",
            path.display()
        ))
    };
    let stored: Stored = serde_json::from_str(&text).map_err(|_| unusable())?;
    let key = base64::decode_key(&stored.key).ok_or_else(unusable)?;
    BleLink {
        name: stored.name,
        friendly_name: stored.friendly_name,
        key,
        port: stored.port,
        scan_seconds: stored.scan_seconds,
        rest_seconds: stored.rest_seconds,
        standby: stored.standby,
        always: stored.always,
    }
    .check()
    .map(Some)
    .map_err(|e| StoreError(format!("{}: {e}", path.display())))
}

/// Removes the saved link (and with it the key). True when there was one.
pub fn clear(data_dir: &Path) -> bool {
    let path = file(data_dir);
    let _ = std::fs::remove_file(path.with_extension("tmp"));
    std::fs::remove_file(&path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn link() -> BleLink {
        BleLink::with_key([7; 32])
    }

    #[test]
    fn a_saved_link_comes_back_and_the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Ok(None));
        let mut l = link();
        l.standby = true;
        l.scan_seconds = 15;
        l.rest_seconds = 45;
        save(dir.path(), &l).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(l));
        let mode = std::fs::metadata(file(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(clear(dir.path()));
        assert!(!clear(dir.path()));
        assert_eq!(load(dir.path()), Ok(None));
    }

    #[test]
    fn the_key_is_never_in_debug_output_or_an_error() {
        let l = link();
        let text = format!("{l:?}");
        assert!(!text.contains(&l.key_text()) && text.contains("redacted"));
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ble")).unwrap();
        std::fs::write(
            file(dir.path()),
            format!(r#"{{"key":"{}","bad":1}}"#, l.key_text()),
        )
        .unwrap();
        let why = load(dir.path()).unwrap_err().to_string();
        assert!(why.contains("not a saved Bluetooth proxy link") && !why.contains(&l.key_text()));
    }

    #[test]
    fn what_a_person_gives_is_checked() {
        let with = |f: &dyn Fn(&mut BleLink)| {
            let mut l = link();
            f(&mut l);
            l.check()
        };
        assert!(with(&|_| {}).is_ok());
        assert!(with(&|l| l.name = "Has Space".into()).is_err());
        assert!(with(&|l| l.name = "-lead".into()).is_err());
        assert!(with(&|l| l.name = "x".repeat(32)).is_err());
        assert!(with(&|l| l.port = 80).is_err());
        assert!(with(&|l| l.friendly_name = String::new()).is_err());
        // The radio's budget: window 5..=30 s, rest at least the window and at most 600 s.
        assert!(with(&|l| l.scan_seconds = 4).is_err());
        assert!(with(&|l| l.scan_seconds = 31).is_err());
        assert!(
            with(&|l| {
                l.scan_seconds = 30;
                l.rest_seconds = 29;
            })
            .is_err()
        );
        assert!(with(&|l| l.rest_seconds = 601).is_err());
        assert!(
            with(&|l| {
                l.scan_seconds = 30;
                l.rest_seconds = 30;
            })
            .is_ok()
        );
    }

    #[test]
    fn the_default_share_of_the_radio_is_a_sixth() {
        assert_eq!(link().duty_percent(), 16);
    }
}
