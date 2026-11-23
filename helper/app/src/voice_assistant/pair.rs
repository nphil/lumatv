//! Who the TV is to Home Assistant, and how it comes to be paired with it.
//!
//! Home Assistant adds the TV as an ESPHome device and makes the key itself (the zero-key provisioning `voice-satellite` serves
//! while the window to pair is open), so nothing is typed on either side. The key is kept in one private file; the device is
//! announced on the network (`_esphomelib._tcp`) only while there is something to reach: a window to pair, or a key.

use crate::ble::Announcer;
use mdns::Service;
use std::io;
use std::net::Ipv4Addr;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use voice_satellite::Identity;

/// How long the window in which Home Assistant may add the TV stays open.
pub const WINDOW: Duration = Duration::from_secs(300);

const PROJECT: &str = "lumatv.voice-assistant";

/// The hardware address the voice device reports. The Bluetooth proxy reports the interface's own, and Home Assistant keys a device
/// by it, so this one has the locally administered bit set and its last byte flipped: never the interface's, always the same one.
pub fn device_mac(interface: &str) -> Option<String> {
    let mut octets = [0u8; 6];
    let mut parts = interface.split(':');
    for octet in &mut octets {
        *octet = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    octets[0] |= 0x02;
    octets[5] ^= 0x01;
    Some(
        octets
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// What Home Assistant is told the device is: `lumatv-voice-<last six hex digits of its address>`.
pub fn identity(mac: &str) -> Identity {
    let digits: Vec<char> = mac.chars().filter(char::is_ascii_hexdigit).collect();
    let tail: String = digits[digits.len().saturating_sub(6)..]
        .iter()
        .map(char::to_ascii_lowercase)
        .collect();
    Identity {
        name: format!("lumatv-voice-{tail}"),
        friendly_name: format!("LumaTV Voice {tail}"),
        mac: mac.to_owned(),
        esphome_version: crate::ble_proxy::ESPHOME_VERSION.to_owned(),
        project_version: crate::VERSION.to_owned(),
    }
}

/// A number for the device among the others that may hear the same word, made from its address (arbitration is off, so only
/// distinctness matters).
pub fn device_number(mac: &str) -> u64 {
    mac.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The service Home Assistant's discovery looks for. A device with a key says `api_encryption`; one without says that it can be given
/// one (`api_provisioning`), which Home Assistant reads as "add it, it will make the key".
pub fn service(identity: &Identity, port: u16, paired: bool) -> Service {
    use voice_satellite::net::{API_ENCRYPTION, API_PROVISIONING};
    let encryption: Vec<(&str, String)> = if paired {
        vec![("api_encryption", API_ENCRYPTION.to_owned())]
    } else {
        vec![
            ("api_encryption_supported", API_ENCRYPTION.to_owned()),
            ("api_provisioning", API_PROVISIONING.to_owned()),
        ]
    };
    Service {
        instance: identity.name.clone(),
        kind: "_esphomelib._tcp".to_owned(),
        port,
        txt: [
            ("mac", identity.mac.replace(':', "").to_ascii_lowercase()),
            ("version", identity.esphome_version.clone()),
            ("friendly_name", identity.friendly_name.clone()),
            ("network", "wifi".to_owned()),
            ("project_name", PROJECT.to_owned()),
            ("project_version", identity.project_version.clone()),
        ]
        .into_iter()
        .chain(encryption)
        .map(|(k, v)| (k.to_owned(), v))
        .collect(),
    }
}

/// The key Home Assistant set, in one private file (44 characters of base64, as Home Assistant reads it).
#[derive(Clone)]
pub struct KeyFile(PathBuf);

impl KeyFile {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// All zeros when there is no key, or the file is not one.
    pub fn load(&self) -> [u8; 32] {
        std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|text| voice_satellite::key::decode(&text))
            .unwrap_or([0; 32])
    }

    pub fn save(&self, key: &[u8; 32]) -> io::Result<()> {
        if let Some(dir) = self.0.parent() {
            private_dir(dir)?;
        }
        crate::hue_store::write_private(&self.0, voice_satellite::key::encode(key).as_bytes(), 0o600)
    }

    pub fn forget(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A directory only the helper's user may enter, made as far up as needed.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Keeps the announcement as what is reachable says: `Some((address, paired))` announces the device there, `None` withdraws it.
pub struct Announcement<A: Announcer> {
    announcer: A,
    held: Option<(Ipv4Addr, bool)>,
}

impl<A: Announcer> Announcement<A> {
    pub fn new(announcer: A) -> Self {
        Self { announcer, held: None }
    }

    /// Announces, withdraws or leaves as it is. An announcement that could not be made is tried again at the next call.
    pub fn keep(
        &mut self,
        wanted: Option<(Ipv4Addr, bool)>,
        identity: &Identity,
        port: u16,
    ) -> Result<(), String> {
        if self.held == wanted {
            return Ok(());
        }
        self.held = None;
        let Some((address, paired)) = wanted else {
            self.announcer.withdraw();
            return Ok(());
        };
        self.announcer
            .announce(address, service(identity, port, paired))?;
        self.held = Some((address, paired));
        Ok(())
    }

    pub fn withdraw(&mut self) {
        self.held = None;
        self.announcer.withdraw();
    }
}

/// The window in which Home Assistant may add the TV and make its key.
#[derive(Default)]
pub struct Window {
    until: Option<Instant>,
}

impl Window {
    pub fn open(&mut self, now: Instant) {
        self.until = Some(now + WINDOW);
    }

    pub fn close(&mut self) {
        self.until = None;
    }

    pub fn is_open(&self) -> bool {
        self.until.is_some()
    }

    /// Whether the window ran out at `now` (it is closed then).
    pub fn ran_out(&mut self, now: Instant) -> bool {
        self.until.take_if(|until| now >= *until).is_some()
    }

    /// How long until it runs out.
    pub fn left(&self, now: Instant) -> Option<Duration> {
        self.until.map(|until| until.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_devices_address_is_never_the_interfaces_and_always_the_same() {
        for interface in ["A8:23:FE:01:02:03", "02:00:00:00:00:00", "FF:FF:FF:FF:FF:FF"] {
            let mac = device_mac(interface).unwrap();
            assert_ne!(mac, interface);
            assert_eq!(device_mac(interface).unwrap(), mac);
            let first = u8::from_str_radix(&mac[..2], 16).unwrap();
            assert_eq!(first & 0x02, 0x02, "locally administered");
        }
        assert_eq!(device_mac("A8:23:FE:01:02:03").unwrap(), "AA:23:FE:01:02:02");
        assert_eq!(device_mac("A8:23:FE:01:02"), None);
        assert_eq!(device_mac("A8:23:FE:01:02:03:04"), None);
        assert_eq!(device_mac("not a mac at all!"), None);
    }

    #[test]
    fn the_name_carries_the_last_six_digits_of_the_address() {
        let id = identity("AA:23:FE:01:02:02");
        assert_eq!(id.name, "lumatv-voice-010202");
        assert_eq!(id.friendly_name, "LumaTV Voice 010202");
        assert_ne!(device_number("AA:23:FE:01:02:02"), device_number("AA:23:FE:01:02:03"));
    }

    fn txt(service: &Service) -> std::collections::BTreeMap<String, String> {
        service.txt.iter().cloned().collect()
    }

    #[test]
    fn an_unpaired_device_says_it_can_be_given_a_key_and_a_paired_one_says_it_has_one() {
        let id = identity("AA:23:FE:01:02:02");
        let open = txt(&service(&id, 6055, false));
        assert_eq!(open["api_provisioning"], "zero-psk");
        assert!(open.contains_key("api_encryption_supported"));
        assert!(!open.contains_key("api_encryption"));
        let paired = service(&id, 6055, true);
        let keyed = txt(&paired);
        assert!(keyed.contains_key("api_encryption"));
        assert!(!keyed.contains_key("api_provisioning"));
        assert_eq!(keyed["mac"], "aa23fe010202");
        assert_eq!(keyed["project_name"], PROJECT);
        assert_eq!((paired.port, paired.kind.as_str()), (6055, "_esphomelib._tcp"));
    }

    #[test]
    fn the_key_is_kept_privately_and_forgotten_by_removing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeyFile::new(dir.path().join("voice-assistant").join("key"));
        assert_eq!(file.load(), [0; 32]);
        let key: [u8; 32] = std::array::from_fn(|i| i as u8 + 1);
        file.save(&key).unwrap();
        assert_eq!(file.load(), key);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join("voice-assistant").join("key")), 0o600);
        assert_eq!(mode(&dir.path().join("voice-assistant")), 0o700);
        std::fs::write(dir.path().join("voice-assistant").join("key"), "not a key").unwrap();
        assert_eq!(file.load(), [0; 32], "a file that is not a key is no key");
        file.save(&key).unwrap();
        file.forget();
        assert_eq!(file.load(), [0; 32]);
    }

    #[derive(Default)]
    struct Log(Arc<Mutex<Vec<String>>>);

    struct Fake(Arc<Mutex<Vec<String>>>, bool);

    impl Announcer for Fake {
        fn announce(&mut self, address: Ipv4Addr, service: Service) -> Result<(), String> {
            if self.1 {
                return Err("no multicast".into());
            }
            let kind = if txt(&service).contains_key("api_encryption") { "paired" } else { "pairable" };
            self.0.lock().unwrap().push(format!("announce {address} {kind}"));
            Ok(())
        }
        fn withdraw(&mut self) {
            self.0.lock().unwrap().push("withdraw".into());
        }
    }

    #[test]
    fn the_announcement_follows_what_is_reachable_and_is_not_repeated() {
        let log = Log::default();
        let mut held = Announcement::new(Fake(log.0.clone(), false));
        let id = identity("AA:23:FE:01:02:02");
        let a = Ipv4Addr::new(192, 168, 1, 242);
        held.keep(Some((a, false)), &id, 6055).unwrap();
        held.keep(Some((a, false)), &id, 6055).unwrap();
        held.keep(Some((a, true)), &id, 6055).unwrap();
        held.keep(None, &id, 6055).unwrap();
        held.keep(None, &id, 6055).unwrap();
        assert_eq!(
            *log.0.lock().unwrap(),
            ["announce 192.168.1.242 pairable", "announce 192.168.1.242 paired", "withdraw"]
        );
    }

    #[test]
    fn an_announcement_that_failed_is_tried_again_at_the_next_look() {
        let log = Log::default();
        let mut held = Announcement::new(Fake(log.0.clone(), true));
        let id = identity("AA:23:FE:01:02:02");
        let wanted = Some((Ipv4Addr::new(10, 0, 0, 2), true));
        assert_eq!(held.keep(wanted, &id, 6055), Err("no multicast".into()));
        assert_eq!(held.keep(wanted, &id, 6055), Err("no multicast".into()));
    }

    #[test]
    fn the_window_runs_out_once() {
        let t0 = Instant::now();
        let mut window = Window::default();
        assert!(!window.is_open() && !window.ran_out(t0));
        window.open(t0);
        assert!(window.is_open() && !window.ran_out(t0 + WINDOW - Duration::from_secs(1)));
        assert_eq!(window.left(t0), Some(WINDOW));
        assert!(window.ran_out(t0 + WINDOW));
        assert!(!window.is_open() && !window.ran_out(t0 + WINDOW));
        window.open(t0);
        window.close();
        assert_eq!(window.left(t0), None);
    }
}
