//! Where ambient light keeps its way to the Hue bridge: the address, the entertainment area, the two keys, and the bridge's
//! certificate.
//!
//! `<data dir>/ambient/hue.json`, mode 0600: `{"host", "port", "streamPort", "area", "appKey", "clientKey"}`, and next to it
//! `hue-cert.pem`, the certificate the bridge showed when it was linked. The keys are only ever sent to the address they were
//! saved with, over a connection that must present that certificate, and neither the settings API nor any other route can change
//! any of it: they are written by the command line (`lumatv-helper hue-link`, the keys come from the bridge or from standard input)
//! and cleared by `hue-unlink`; the owner of the TV runs those, nothing in the helper does.
//!
//! The application key (what the bridge calls `username`) and the client key are never logged, never put in a topic or an API
//! answer, never put on a command line, and their `Debug` says nothing of them.

use lumatv_ambient::huestream::{decode_client_key, is_uuid};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The bridge's HTTPS port, and the UDP port of its entertainment stream.
pub const DEFAULT_PORT: u16 = 443;
pub const DEFAULT_STREAM_PORT: u16 = lumatv_ambient::huestream::PORT;
/// Bifrost's default `streaming_fps`, which is what the stream is paced to unless told otherwise.
pub const DEFAULT_FPS: u8 = 20;
pub const MAX_FPS: u8 = 60;

/// `<data dir>/ambient/`.
fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("ambient")
}

/// `<data dir>/ambient/hue.json`.
pub fn file(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("hue.json")
}

/// The bridge's pinned certificate, a plain PEM file (the TLS child may run as another user and has to read it).
pub fn certificate_file(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("hue-cert.pem")
}

/// A key. It says nothing of itself when printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Where the bridge is and what to speak to it with.
#[derive(Clone, PartialEq, Eq)]
pub struct HueLink {
    pub host: Ipv4Addr,
    pub port: u16,
    pub stream_port: u16,
    /// Updates a second sent to the bridge. Bifrost forwards at most its own `streaming_fps` (default 20) and smooths for the rate it
    /// sees, so this is best equal to that setting; a Hue bridge takes more, but Zigbee lamps about 25.
    pub fps: u8,
    /// The entertainment area (its id, a UUID) that is streamed to; none until one is chosen.
    pub area: Option<String>,
    /// `hue-application-key`.
    pub app_key: Secret,
    /// The 32 hex characters of the DTLS key.
    pub client_key: Secret,
}

impl fmt::Debug for HueLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HueLink({}:{}, stream {}, area {:?}, keys <redacted>)",
            self.host, self.port, self.stream_port, self.area
        )
    }
}

impl HueLink {
    /// The link a person gave, checked: a home-network address (the picture's colours and the keys never leave it), a UUID for the
    /// area when there is one, a key that is one word, a client key of 32 hex characters.
    pub fn new(
        host: &str,
        port: u16,
        stream_port: u16,
        area: Option<&str>,
        app_key: &str,
        client_key: &str,
    ) -> Result<Self, StoreError> {
        let host: Ipv4Addr = host
            .trim()
            .parse()
            .map_err(|_| StoreError("the bridge's address must be an IPv4 address".to_owned()))?;
        if !crate::sys::is_lan_address(host) {
            return Err(StoreError(
                "the bridge's address must be a private (or loopback) IPv4 address".to_owned(),
            ));
        }
        if port == 0 || stream_port == 0 {
            return Err(StoreError("a port of 0 is not a port".to_owned()));
        }
        let area = area.map(|a| a.trim().to_ascii_lowercase());
        if area.as_deref().is_some_and(|a| !is_uuid(a)) {
            return Err(StoreError(
                "the entertainment area is its id, 36 characters like 7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10".to_owned(),
            ));
        }
        let app_key = app_key.trim();
        if !(8..=128).contains(&app_key.len()) || !app_key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(StoreError(
                "the application key is 8 to 128 printable characters with no spaces".to_owned(),
            ));
        }
        let client_key = client_key.trim();
        if decode_client_key(client_key).is_err() {
            return Err(StoreError(
                "the client key is 32 hexadecimal characters".to_owned(),
            ));
        }
        Ok(Self {
            fps: DEFAULT_FPS,
            host,
            port,
            stream_port,
            area,
            app_key: Secret(app_key.to_owned()),
            client_key: Secret(client_key.to_ascii_lowercase()),
        })
    }
}

fn default_fps() -> u8 {
    DEFAULT_FPS
}

impl HueLink {
    /// The same link streaming to another area.
    pub fn with_area(&self, area: &str) -> Result<Self, StoreError> {
        Self::new(
            &self.host.to_string(),
            self.port,
            self.stream_port,
            Some(area),
            self.app_key.expose(),
            self.client_key.expose(),
        )
        .and_then(|link| link.with_fps(self.fps))
    }

    /// The same link with another stream rate, in updates a second.
    pub fn with_fps(mut self, fps: u8) -> Result<Self, StoreError> {
        if !(1..=MAX_FPS).contains(&fps) {
            return Err(StoreError(format!(
                "the stream rate is 1 to {MAX_FPS} updates a second"
            )));
        }
        self.fps = fps;
        Ok(self)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Stored {
    host: String,
    port: u16,
    stream_port: u16,
    #[serde(default = "default_fps")]
    fps: u8,
    #[serde(default)]
    area: Option<String>,
    app_key: String,
    client_key: String,
}

/// Why the link could not be saved or read, in words that never carry a key.
#[derive(Debug, PartialEq, Eq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

fn write_private(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // A file made earlier by something else may have another mode, and `mode` above only applies to a new one.
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Writes the certificate a bridge showed (a PEM text) next to the saved one, not over it: the keys are fetched over a connection
/// pinned to it, and a link that fails half way must leave the working one as it was. Returns where it is.
pub fn stage_certificate(data_dir: &Path, pem: &str) -> Result<PathBuf, StoreError> {
    let folder = dir(data_dir);
    // The directory is open to read: the TLS child may run as another user and reads the certificate. The keys are in a file of
    // their own, mode 0600.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&folder)
        .map_err(|e| StoreError(format!("{}: {e}", folder.display())))?;
    let staged = folder.join("hue-cert.staged.pem");
    write_private(&staged, pem.as_bytes(), 0o644)
        .map_err(|e| StoreError(format!("{}: {e}", staged.display())))?;
    Ok(staged)
}

/// Makes the staged certificate the one a connection to the bridge must present, or (`keep` false) throws it away.
pub fn settle_certificate(data_dir: &Path, keep: bool) -> Result<(), StoreError> {
    let staged = dir(data_dir).join("hue-cert.staged.pem");
    if !keep {
        let _ = std::fs::remove_file(&staged);
        return Ok(());
    }
    let cert = certificate_file(data_dir);
    std::fs::rename(&staged, &cert).map_err(|e| StoreError(format!("{}: {e}", cert.display())))
}

/// Saves the link (and with it the certificate written before it); the file is replaced whole.
pub fn save(data_dir: &Path, link: &HueLink) -> Result<(), StoreError> {
    if !certificate_file(data_dir).is_file() {
        return Err(StoreError(
            "the bridge's certificate is not saved yet".to_owned(),
        ));
    }
    let stored = Stored {
        host: link.host.to_string(),
        port: link.port,
        stream_port: link.stream_port,
        fps: link.fps,
        area: link.area.clone(),
        app_key: link.app_key.expose().to_owned(),
        client_key: link.client_key.expose().to_owned(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(|e| StoreError(e.to_string()))?;
    let path = file(data_dir);
    write_private(&path, &bytes, 0o600).map_err(|e| StoreError(format!("{}: {e}", path.display())))
}

/// The saved link: `Ok(None)` when there is none, an error (without a key) when the file is there and cannot be used.
pub fn load(data_dir: &Path) -> Result<Option<HueLink>, StoreError> {
    let path = file(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError(format!("{}: {e}", path.display()))),
    };
    let stored: Stored = serde_json::from_str(&text)
        .map_err(|_| StoreError(format!("{} is not a saved link", path.display())))?;
    let link = HueLink::new(
        &stored.host,
        stored.port,
        stored.stream_port,
        stored.area.as_deref(),
        &stored.app_key,
        &stored.client_key,
    )
    .and_then(|link| link.with_fps(stored.fps))
    .map_err(|e| StoreError(format!("{}: {e}", path.display())))?;
    if !certificate_file(data_dir).is_file() {
        return Err(StoreError(format!(
            "{} is missing: link the bridge again",
            certificate_file(data_dir).display()
        )));
    }
    Ok(Some(link))
}

/// Removes the saved link and certificate. True when there was a link.
pub fn clear(data_dir: &Path) -> bool {
    let _ = std::fs::remove_file(certificate_file(data_dir));
    std::fs::remove_file(file(data_dir)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: &str = "7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10";
    const APP: &str = "ab12CD34ef56GH78ij90KL12mn34OP56qr78ST90";
    const CLIENT: &str = "00112233445566778899AABBCCDDEEFF";

    fn link() -> HueLink {
        HueLink::new("192.168.1.60", 443, 2100, Some(AREA), APP, CLIENT).unwrap()
    }

    #[test]
    fn a_link_round_trips_with_a_private_key_file() {
        let dir = tempfile::tempdir().unwrap();
        stage_certificate(dir.path(), "PEM").unwrap();
        settle_certificate(dir.path(), true).unwrap();
        save(dir.path(), &link()).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file(dir.path())), 0o600);
        assert_eq!(mode(&certificate_file(dir.path())), 0o644);
        let back = load(dir.path()).unwrap().unwrap();
        assert_eq!(back, link());
        assert_eq!(back.client_key.expose(), CLIENT.to_ascii_lowercase());
        assert!(clear(dir.path()));
        assert_eq!(load(dir.path()), Ok(None));
        assert!(!clear(dir.path()));
    }

    #[test]
    fn a_bad_link_saves_nothing_and_says_nothing_of_the_keys() {
        for (host, area, app, client) in [
            ("8.8.8.8", AREA, APP, CLIENT),
            ("bridge.local", AREA, APP, CLIENT),
            ("192.168.1.60", "not-a-uuid", APP, CLIENT),
            ("192.168.1.60", AREA, "short", CLIENT),
            ("192.168.1.60", AREA, "has a space in the key", CLIENT),
            ("192.168.1.60", AREA, APP, "00112233"),
            (
                "192.168.1.60",
                AREA,
                APP,
                "zz112233445566778899AABBCCDDEEFF",
            ),
        ] {
            let e = HueLink::new(host, 443, 2100, Some(area), app, client).unwrap_err();
            assert!(!e.0.contains(APP) && !e.0.contains(CLIENT), "{e}");
        }
        assert!(HueLink::new("192.168.1.60", 0, 2100, Some(AREA), APP, CLIENT).is_err());
        assert!(HueLink::new("192.168.1.60", 443, 0, Some(AREA), APP, CLIENT).is_err());
    }

    #[test]
    fn a_damaged_file_or_a_missing_certificate_is_an_error_without_a_key() {
        let dir = tempfile::tempdir().unwrap();
        stage_certificate(dir.path(), "PEM").unwrap();
        settle_certificate(dir.path(), true).unwrap();
        save(dir.path(), &link()).unwrap();
        std::fs::remove_file(certificate_file(dir.path())).unwrap();
        let e = load(dir.path()).unwrap_err();
        assert!(e.0.contains("link the bridge again"), "{e}");
        std::fs::write(
            file(dir.path()),
            format!("{{\"host\":\"8.8.8.8\",\"appKey\":\"{APP}\"}}"),
        )
        .unwrap();
        let e = load(dir.path()).unwrap_err();
        assert!(!e.0.contains(APP), "{e}");
    }

    #[test]
    fn debug_hides_the_keys() {
        let text = format!("{:?}", link());
        assert!(!text.contains(APP) && !text.contains(&CLIENT.to_ascii_lowercase()));
        assert!(text.contains("192.168.1.60"));
    }

    #[test]
    fn the_stream_rate_defaults_to_bifrosts_and_is_kept_with_the_link() {
        let dir = tempfile::tempdir().unwrap();
        stage_certificate(dir.path(), "PEM").unwrap();
        settle_certificate(dir.path(), true).unwrap();
        assert_eq!(link().fps, DEFAULT_FPS);
        save(dir.path(), &link().with_fps(12).unwrap()).unwrap();
        let back = load(dir.path()).unwrap().unwrap();
        assert_eq!(back.fps, 12);
        // Choosing another area keeps the rate; a rate outside 1..=60 is refused.
        assert_eq!(back.with_area(AREA).unwrap().fps, 12);
        assert!(link().with_fps(0).is_err() && link().with_fps(61).is_err());
        // A link saved before the rate existed reads as the default.
        let text = std::fs::read_to_string(file(dir.path())).unwrap();
        let old = text.replace(",\"fps\":12", "");
        assert_ne!(old, text);
        std::fs::write(file(dir.path()), old).unwrap();
        assert_eq!(load(dir.path()).unwrap().unwrap().fps, DEFAULT_FPS);
    }
}
