//! The clients LG has accepted, by the SHA-256 of their keys (never the keys), with the permissions LG granted them.

use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// LG keeps at most this many client profiles.
const MAX_CLIENTS: usize = 100;
/// A stored key older than this is confirmed with LG again (a relayed session) before Rust serves it.
pub const RECHECK_SECS: u64 = 30 * 24 * 3600;

#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    pub fp: String,
    pub grant: Vec<String>,
    pub app_id: Option<String>,
    /// Unix seconds when LG last answered `registered` for the key.
    pub checked: u64,
}

pub struct Store {
    path: PathBuf,
    clients: Vec<Client>,
}

pub fn fingerprint(key: &str) -> String {
    crate::util::sha256_hex(key.as_bytes())
}

impl Store {
    pub fn load(path: PathBuf) -> Store {
        let clients = match std::fs::read(&path) {
            Ok(data) => match serde_json::from_slice::<Value>(&data) {
                Ok(v) => parse(&v),
                Err(e) => {
                    crate::warn!("{} is unreadable ({e}); starting with no clients", path.display());
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                crate::warn!("{}: {e}; starting with no clients", path.display());
                Vec::new()
            }
        };
        Store { path, clients }
    }

    pub fn get(&self, fp: &str) -> Option<&Client> {
        self.clients.iter().find(|c| c.fp == fp)
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn put(&mut self, client: Client) {
        self.clients.retain(|c| c.fp != client.fp);
        self.clients.push(client);
        if self.clients.len() > MAX_CLIENTS {
            if let Some(oldest) = (0..self.clients.len()).min_by_key(|&i| self.clients[i].checked) {
                self.clients.remove(oldest);
            }
        }
        self.save();
    }

    pub fn remove(&mut self, fp: &str) -> bool {
        let before = self.clients.len();
        self.clients.retain(|c| c.fp != fp);
        let removed = self.clients.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    pub fn clear(&mut self) -> usize {
        let n = self.clients.len();
        self.clients.clear();
        self.save();
        n
    }

    fn save(&self) {
        let clients: Vec<Value> = self
            .clients
            .iter()
            .map(|c| json!({"fp": c.fp, "grant": c.grant, "appId": c.app_id, "checked": c.checked}))
            .collect();
        let body = json!({"version": 1, "clients": clients}).to_string();
        let tmp = self.path.with_extension("json.tmp");
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| {
                f.write_all(body.as_bytes())?;
                f.sync_all()
            })
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = written {
            crate::error!("could not save {}: {e}", self.path.display());
        }
    }
}

fn parse(v: &Value) -> Vec<Client> {
    v.get("clients")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let fp = c.get("fp")?.as_str()?;
            (fp.len() == 64).then(|| Client {
                fp: fp.to_owned(),
                grant: c
                    .get("grant")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|g| g.as_str().map(str::to_owned))
                    .collect(),
                app_id: c.get("appId").and_then(Value::as_str).map(str::to_owned),
                checked: c.get("checked").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn clients_persist_privately_by_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clients.json");
        let mut store = Store::load(path.clone());
        let fp = fingerprint("secret-key");
        store.put(Client { fp: fp.clone(), grant: vec!["LAUNCH".into()], app_id: Some("com.example".into()), checked: 5 });
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("secret-key"));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let again = Store::load(path.clone());
        assert_eq!(again.get(&fp).unwrap().grant, ["LAUNCH"]);
        let mut again = again;
        assert!(again.remove(&fp));
        assert!(Store::load(path.clone()).get(&fp).is_none());
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(Store::load(path).len(), 0);
    }
}
