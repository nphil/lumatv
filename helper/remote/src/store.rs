//! The paired phones and the TV id, in one JSON file only root reads (0600): `{"tv", "devices": [{"id", "name", "model", "app", "key",
//! "paired", "last"}]}`, keys in base64, times in Unix ms. The TV id is made the first time the file is opened.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use esphome_api::base64;
use ring::rand::SecureRandom;
use serde_json::{Value, json};

use crate::{hex, unhex};

/// Paired phones at most.
pub const MAX_DEVICES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// 16 hex.
    pub id: String,
    pub name: String,
    pub model: String,
    pub app: String,
    pub key: [u8; 32],
    pub paired: u64,
    pub last: u64,
}

pub struct Store {
    path: PathBuf,
    tv: [u8; 8],
    devices: Vec<Device>,
}

impl Store {
    /// Opens the file, or starts it with a new TV id. A file that does not parse is started anew (its phones pair again), and said so.
    pub fn open(path: &Path, random: &dyn SecureRandom) -> io::Result<Store> {
        let parsed = match fs::read(path) {
            Ok(bytes) => {
                let parsed = serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| Self::parse(path, &v));
                if parsed.is_none() {
                    log::warn!(target: "lumaremote", "{} does not parse; the paired phones are forgotten", path.display());
                }
                parsed
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if let Some(store) = parsed {
            return Ok(store);
        }
        let mut tv = [0u8; 8];
        random.fill(&mut tv).map_err(|_| io::Error::other("system random source"))?;
        let store = Store { path: path.to_path_buf(), tv, devices: Vec::new() };
        store.save()?;
        Ok(store)
    }

    fn parse(path: &Path, v: &Value) -> Option<Store> {
        let tv = unhex::<8>(v.get("tv")?.as_str()?)?;
        let text = |d: &Value, k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let devices = v
            .get("devices")?
            .as_array()?
            .iter()
            .filter_map(|d| {
                Some(Device {
                    id: text(d, "id"),
                    name: text(d, "name"),
                    model: text(d, "model"),
                    app: text(d, "app"),
                    key: base64::decode_key(d.get("key")?.as_str()?)?,
                    paired: d.get("paired").and_then(Value::as_u64).unwrap_or(0),
                    last: d.get("last").and_then(Value::as_u64).unwrap_or(0),
                })
            })
            .take(MAX_DEVICES)
            .collect();
        Some(Store { path: path.to_path_buf(), tv, devices })
    }

    /// Writes the whole file anew beside the old one and renames it over, so a power cut leaves one or the other.
    fn save(&self) -> io::Result<()> {
        let devices: Vec<Value> = self
            .devices
            .iter()
            .map(|d| {
                json!({"id": d.id, "name": d.name, "model": d.model, "app": d.app, "key": base64::encode(&d.key), "paired": d.paired, "last": d.last})
            })
            .collect();
        let body = serde_json::to_vec_pretty(&json!({"tv": hex(&self.tv), "devices": devices}))?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        let temp = self.path.with_extension("json.new");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        file.write_all(&body)?;
        file.sync_all()?;
        fs::rename(&temp, &self.path)
    }

    pub fn tv(&self) -> [u8; 8] {
        self.tv
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn full(&self) -> bool {
        self.devices.len() >= MAX_DEVICES
    }

    pub fn add(&mut self, device: Device) -> io::Result<()> {
        if self.full() {
            return Err(io::Error::other("16 phones are paired"));
        }
        self.devices.push(device);
        self.save()
    }

    /// A phone's session opened: its row takes what `hello` said and the time.
    pub fn seen(&mut self, id: &str, name: &str, model: &str, app: &str, now: u64) -> io::Result<()> {
        let Some(d) = self.devices.iter_mut().find(|d| d.id == id) else { return Ok(()) };
        for (field, value) in [(&mut d.name, name), (&mut d.model, model), (&mut d.app, app)] {
            if !value.is_empty() {
                *field = value.to_string();
            }
        }
        d.last = now;
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> io::Result<bool> {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        if self.devices.len() == before {
            return Ok(false);
        }
        self.save().map(|()| true)
    }

    pub fn clear(&mut self) -> io::Result<()> {
        self.devices.clear();
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> Device {
        Device { id: id.into(), name: "Phone".into(), model: "iPhone17,1".into(), app: "Marquee".into(), key: [5; 32], paired: 1, last: 1 }
    }

    #[test]
    fn phones_and_the_tv_id_persist_only_root_reads_them_and_sixteen_is_the_most() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("phone-remote/devices.json");
        let random = ring::rand::SystemRandom::new();
        let mut store = Store::open(&path, &random).unwrap();
        let tv = store.tv();
        store.add(device("0011223344556677")).unwrap();
        store.seen("0011223344556677", "Kitchen", "", "", 99).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        let mut again = Store::open(&path, &random).unwrap();
        assert_eq!(again.tv(), tv);
        assert_eq!(again.devices(), [Device { name: "Kitchen".into(), last: 99, ..device("0011223344556677") }]);
        for i in 1..MAX_DEVICES {
            again.add(device(&format!("{i:016x}"))).unwrap();
        }
        assert!(again.add(device("ffffffffffffffff")).is_err());
        assert!(again.remove("0011223344556677").unwrap());
        assert!(!again.remove("0011223344556677").unwrap());
        again.clear().unwrap();
        assert!(Store::open(&path, &random).unwrap().devices().is_empty());

        fs::write(&path, "{").unwrap();
        assert_ne!(Store::open(&path, &random).unwrap().tv(), tv, "a broken file starts anew");
    }
}
