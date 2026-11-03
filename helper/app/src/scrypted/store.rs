//! The saved link to the LumaShow bridge: one private file, `<data dir>/scrypted/link.json`.
//!
//! `{"address": "192.168.1.69:10444", "pin": "<64 hex digits>", "token": "lsb_...", "device": "LumaTV"}`, mode 0600, in a directory of mode
//! 0700. The four are saved together on purpose: the token is only ever sent to the address it was saved with and only over a connection
//! whose certificate has the pin, and nothing that can reach the loopback API can change any of them. They are written by the command
//! line (`scrypted-link`, which makes the token itself and prints only its hash, the verifier) and cleared by `scrypted-unlink` or by the
//! owner's "Forget link" in Settings, which narrows what the TV may do (Principle 0).
//!
//! The token is never logged, never put in a topic or an API answer, and its `Debug` says nothing of it.

use ha_link::AddressPolicy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// The bridge's own port (`docs/scrypted-bridge-api.md` section 1).
pub const DEFAULT_PORT: u16 = 10444;
/// What a bridge calls this TV in its list of devices when the owner gave no other name.
pub const DEFAULT_DEVICE: &str = "LumaTV";
/// The shape of a token the bridge makes (`lsb_` and 43 characters of base64url); the TV makes its own in the same shape.
const TOKEN_PREFIX: &str = "lsb_";

/// Why a link could not be made, saved or read, in words that never carry the token.
#[derive(Debug, PartialEq, Eq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

fn error(text: impl Into<String>) -> StoreError {
    StoreError(text.into())
}

/// The SHA-256 of the one certificate the bridge shows: the only thing the TV trusts (no chain, no name, no date).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Pin([u8; 32]);

impl Pin {
    /// Reads what the bridge's settings page shows (`3A:7F:...`, 32 pairs) or 64 hexadecimal digits: case, colons and spaces are ignored.
    pub fn parse(text: &str) -> Result<Self, StoreError> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|b| !matches!(b, b':' | b' ' | b'-'))
            .collect();
        if digits.len() != 64 || !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err(error(
                "the pin is the bridge's certificate fingerprint: 32 pairs of hexadecimal digits, as its settings show them",
            ));
        }
        let mut out = [0u8; 32];
        for (slot, pair) in out.iter_mut().zip(digits.chunks(2)) {
            let text = std::str::from_utf8(pair).unwrap_or("00");
            *slot = u8::from_str_radix(text, 16).unwrap_or(0);
        }
        Ok(Self(out))
    }

    pub fn of_certificate(der: &[u8]) -> Self {
        Self(Sha256::digest(der).into())
    }

    pub fn hex(&self) -> String {
        hex(&self.0)
    }

    /// Whether the certificate in `der` is the pinned one.
    pub fn matches_certificate(&self, der: &[u8]) -> bool {
        self.matches(&Sha256::digest(der))
    }

    /// Whether `digest` is this pin; the comparison takes the same time wherever the two differ.
    pub fn matches(&self, digest: &[u8]) -> bool {
        digest.len() == self.0.len()
            && self
                .0
                .iter()
                .zip(digest)
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

impl fmt::Debug for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pin({}..)", &self.hex()[..8])
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The device token. Its `Debug` and `Display` say nothing of it; [`Token::expose`] is the only way to read it, and the only readers are
/// the request writer and the verifier.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A token of the kernel's making: `lsb_` and 32 random bytes as base64url, the shape the bridge's own tokens have.
    pub fn make() -> Result<Self, StoreError> {
        let mut bytes = [0u8; 32];
        crate::sys::random_bytes(&mut bytes)
            .map_err(|e| error(format!("no random numbers: {e}")))?;
        let text = esphome_api::base64::encode(&bytes)
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "");
        Ok(Self(format!("{TOKEN_PREFIX}{text}")))
    }

    fn parse(text: &str) -> Result<Self, StoreError> {
        // what `Authorization: Bearer` can carry: printable ASCII, no spaces, at most 200 characters
        if text.is_empty() || text.len() > 200 || !text.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(error("the saved token is not a token"));
        }
        Ok(Self(text.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// What the owner adds to the bridge's list of devices: the SHA-256 of the token's text, 64 hexadecimal digits. A hash is no secret.
    pub fn verifier(&self) -> String {
        Self::verifier_of(&self.0)
    }

    /// The verifier of a token's text (what a bridge holds of a device, and a fake bridge checks a presented token against).
    pub fn verifier_of(text: &str) -> String {
        hex(&Sha256::digest(text.as_bytes()))
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Where the bridge is, which certificate it has, who this TV is to it.
#[derive(Clone, PartialEq, Eq)]
pub struct Link {
    pub address: SocketAddr,
    pub pin: Pin,
    pub token: Token,
    pub device: String,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Link({}, {:?}, token <redacted>, {})",
            self.address, self.pin, self.device
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    address: String,
    pin: String,
    token: String,
    device: String,
}

/// An address the TV may send the token to: an IP literal on the home network, with the bridge's port when none is given. `policy` is
/// the home network and nothing else on the set; a test that runs a bridge on the loopback allows that too.
pub fn parse_address(text: &str, policy: AddressPolicy) -> Result<SocketAddr, StoreError> {
    let text = text.trim();
    let addr = match text.parse::<SocketAddr>() {
        Ok(addr) => addr,
        Err(_) => {
            let ip: IpAddr = text
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse()
                .map_err(|_| {
                    error("the address is the bridge's IP address and port, like 192.168.1.69:10444 (a name is not taken: it could lead anywhere)")
                })?;
            SocketAddr::new(ip, DEFAULT_PORT)
        }
    };
    if addr.port() == 0 {
        return Err(error("the address needs a port from 1 to 65535"));
    }
    if !policy.allows(addr.ip()) {
        return Err(error(
            "that address is not on your home network: the TV sends its token only to a private address",
        ));
    }
    Ok(addr)
}

/// A device name as the bridge's list shows it: 1 to 40 printable characters.
pub fn check_device(name: &str) -> Result<String, StoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(error("a device name is 1 to 40 characters"));
    }
    Ok(name.to_owned())
}

fn parse(stored: &Stored, policy: AddressPolicy) -> Result<Link, StoreError> {
    Ok(Link {
        address: parse_address(&stored.address, policy)?,
        pin: Pin::parse(&stored.pin)?,
        token: Token::parse(&stored.token)?,
        device: check_device(&stored.device)?,
    })
}

/// Checks and saves the link at `path`; the file is replaced whole.
pub fn save(path: &Path, link: &Link, policy: AddressPolicy) -> Result<(), StoreError> {
    let stored = Stored {
        address: link.address.to_string(),
        pin: link.pin.hex(),
        token: link.token.expose().to_owned(),
        device: link.device.clone(),
    };
    // what is written must read back: the same checks as a load
    parse(&stored, policy)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| error(format!("{}: {e}", dir.display())))?;
    // a directory made earlier by something else may be wider than this one wants
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| error(format!("{}: {e}", dir.display())))?;
    let bytes = serde_json::to_vec(&stored).map_err(|e| error(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| error(format!("{}: {e}", path.display())))?;
    let written = (|| {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(error(format!("{}: {e}", path.display())));
    }
    Ok(())
}

/// The saved link: `Ok(None)` when there is none, an error (without the token) when the file is there and cannot be used.
pub fn load(path: &Path, policy: AddressPolicy) -> Result<Option<Link>, StoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(format!("{}: {e}", path.display()))),
    };
    let stored: Stored = serde_json::from_str(&text)
        .map_err(|_| error(format!("{} is not a saved link", path.display())))?;
    parse(&stored, policy)
        .map(Some)
        .map_err(|e| error(format!("{}: {e}", path.display())))
}

/// Removes the saved link. True when there was one.
pub fn clear(path: &Path) -> bool {
    let had = std::fs::remove_file(path).is_ok();
    // the directory goes with it when nothing else is in it
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
    had
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "3A:7F:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E";

    fn link() -> Link {
        Link {
            address: "192.168.1.69:10444".parse().unwrap(),
            pin: Pin::parse(PIN).unwrap(),
            token: Token::make().unwrap(),
            device: "LumaTV".into(),
        }
    }

    #[test]
    fn a_pin_is_read_the_way_the_bridge_shows_it_and_compared_whole() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(pin.hex().len(), 64);
        assert_eq!(&pin.hex()[..6], "3a7f00");
        assert_eq!(
            Pin::parse(&PIN.to_ascii_lowercase().replace(':', "")).unwrap(),
            pin
        );
        assert_eq!(Pin::parse(&PIN.replace(':', " ")).unwrap(), pin);
        for bad in [
            "",
            "3A:7F",
            &PIN[..PIN.len() - 3],
            &format!("{PIN}:00"),
            &PIN.replace("3A", "ZZ"),
        ] {
            assert!(Pin::parse(bad).is_err(), "{bad}");
        }
        let digest = Sha256::digest(b"the certificate");
        let of = Pin::of_certificate(b"the certificate");
        assert!(of.matches(&digest));
        let mut other = digest;
        other[31] ^= 1;
        assert!(!of.matches(&other));
        assert!(!of.matches(&digest[..31]));
        assert!(
            !format!("{pin:?}").contains(&pin.hex()),
            "only the start of a pin is shown"
        );
    }

    #[test]
    fn a_token_has_the_bridges_shape_and_its_verifier_is_the_hash_of_its_text() {
        let token = Token::make().unwrap();
        let text = token.expose();
        assert!(
            text.starts_with("lsb_") && text.len() == 4 + 43,
            "{}",
            text.len()
        );
        assert!(
            text[4..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(
            text,
            Token::make().unwrap().expose(),
            "two tokens are not alike"
        );
        let verifier = token.verifier();
        assert_eq!(verifier.len(), 64);
        assert_eq!(verifier, hex(&Sha256::digest(text.as_bytes())));
        assert!(!verifier.contains(&text[4..]));
        assert!(!format!("{token:?}").contains(&text[4..]));
        // a known vector: the hash of "abc"
        assert_eq!(
            Token("abc".into()).verifier(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn only_a_private_address_is_taken_and_the_bridges_port_is_the_default() {
        let home = AddressPolicy::HOME;
        assert_eq!(
            parse_address("192.168.1.69", home).unwrap().to_string(),
            "192.168.1.69:10444"
        );
        assert_eq!(
            parse_address("192.168.1.69:11444", home).unwrap().port(),
            11444
        );
        assert_eq!(
            parse_address("[fd00::1]:10444", home).unwrap().port(),
            10444
        );
        assert_eq!(parse_address("fd00::1", home).unwrap().port(), 10444);
        for bad in [
            "8.8.8.8:10444",
            "cctv.nateshome.net",
            "127.0.0.1:10444",
            "192.168.1.69:0",
            "",
            "192.168.1.69:99999",
        ] {
            assert!(parse_address(bad, home).is_err(), "{bad}");
        }
        assert!(parse_address("127.0.0.1:10444", AddressPolicy::TESTING).is_ok());
        assert!(parse_address("8.8.8.8:10444", AddressPolicy::TESTING).is_err());
    }

    #[test]
    fn a_saved_link_is_private_reads_back_and_is_cleared_with_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scrypted").join("link.json");
        assert!(load(&path, AddressPolicy::HOME).unwrap().is_none());
        let link = link();
        save(&path, &link, AddressPolicy::HOME).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let back = load(&path, AddressPolicy::HOME).unwrap().unwrap();
        assert_eq!(back, link);
        assert!(clear(&path));
        assert!(
            !path.parent().unwrap().exists(),
            "the directory goes with the file"
        );
        assert!(load(&path, AddressPolicy::HOME).unwrap().is_none());
        assert!(!clear(&path));
    }

    #[test]
    fn replacement_refuses_unowned_temps_and_keeps_the_old_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.json");
        let tmp = path.with_extension("json.tmp");
        let target = dir.path().join("target");
        let old = link();
        let mut new = link();
        new.address = "192.168.1.70:10444".parse().unwrap();
        save(&path, &old, AddressPolicy::HOME).unwrap();
        std::fs::write(&tmp, "unowned").unwrap();
        assert!(save(&path, &new, AddressPolicy::HOME).is_err());
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "unowned");
        std::fs::remove_file(&tmp).unwrap();
        std::fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, &tmp).unwrap();
        assert!(save(&path, &new, AddressPolicy::HOME).is_err());
        assert!(std::fs::symlink_metadata(&tmp).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(load(&path, AddressPolicy::HOME).unwrap().unwrap(), old);
        std::fs::remove_file(&tmp).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        save(&path, &new, AddressPolicy::HOME).unwrap();
        assert_eq!(load(&path, AddressPolicy::HOME).unwrap().unwrap(), new);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_bad_link_saves_nothing_and_a_damaged_file_does_not_quote_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.json");
        let mut bad = link();
        bad.address = "8.8.8.8:10444".parse().unwrap();
        assert!(save(&path, &bad, AddressPolicy::HOME).is_err());
        let mut bad = link();
        bad.device = "".into();
        assert!(save(&path, &bad, AddressPolicy::HOME).is_err());
        assert!(!path.exists());
        let good = link();
        save(&path, &good, AddressPolicy::HOME).unwrap();
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("192.168.1.69", "8.8.8.8");
        std::fs::write(&path, &text).unwrap();
        let e = load(&path, AddressPolicy::HOME).unwrap_err();
        assert!(!e.0.contains(good.token.expose()), "{e}");
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path, AddressPolicy::HOME).is_err());
        std::fs::write(
            &path,
            r#"{"address":"192.168.1.69:10444","pin":"00","token":"x","device":"d","extra":1}"#,
        )
        .unwrap();
        assert!(
            load(&path, AddressPolicy::HOME).is_err(),
            "unknown fields are not taken"
        );
    }

    #[test]
    fn the_debug_form_of_a_link_hides_the_token_and_most_of_the_pin() {
        let link = link();
        let shown = format!("{link:?}");
        assert!(
            !shown.contains(link.token.expose()) && !shown.contains(&link.pin.hex()),
            "{shown}"
        );
    }
}
