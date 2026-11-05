//! Where the guide keeps what it fetched, as the host lends it: small documents by name (`state.json`, `plex/2026-11-05.json`).

use parking_lot::{Mutex, MutexGuard};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

pub trait Store: Send + Sync {
    /// The document `key`, when there is one.
    fn load(&self, key: &str) -> Option<Vec<u8>>;
    /// Keeps `bytes` as `key`, whole: a reader sees the old document or the new one.
    fn save(&self, key: &str, bytes: &[u8]) -> std::io::Result<()>;
    fn remove(&self, key: &str);
    /// The keys under `prefix` (`plex/`), sorted.
    fn keys(&self, prefix: &str) -> Vec<String>;
}

/// Whether `key` is a relative name the stores take: `/`-separated plain parts, nothing that climbs out.
fn plain_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
}

/// Documents as files under one directory.
#[derive(Clone, Debug)]
pub struct DirStore {
    dir: PathBuf,
}

impl DirStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Store for DirStore {
    fn load(&self, key: &str) -> Option<Vec<u8>> {
        plain_key(key)
            .then(|| std::fs::read(self.dir.join(key)).ok())
            .flatten()
    }

    fn save(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        if !plain_key(key) {
            return Err(std::io::Error::other(format!("not a key: {key}")));
        }
        let path = self.dir.join(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("new");
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)
    }

    fn remove(&self, key: &str) {
        if plain_key(key) {
            let _ = std::fs::remove_file(self.dir.join(key));
        }
    }

    fn keys(&self, prefix: &str) -> Vec<String> {
        let (dir, start) = match prefix.rsplit_once('/') {
            Some((dir, start)) => (Some(dir), start),
            None => (None, prefix),
        };
        let path = match dir {
            Some(d) if plain_key(d) => self.dir.join(d),
            Some(_) => return Vec::new(),
            None => self.dir.clone(),
        };
        let mut out: Vec<String> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.starts_with(start) && !name.ends_with(".new"))
            .map(|name| match dir {
                Some(d) => format!("{d}/{name}"),
                None => name,
            })
            .collect();
        out.sort();
        out
    }
}

/// Documents in memory: for tests, and for a host that keeps nothing.
#[derive(Debug, Default)]
pub struct MemStore {
    docs: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn docs(&self) -> MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.docs.lock()
    }
}

impl Store for MemStore {
    fn load(&self, key: &str) -> Option<Vec<u8>> {
        self.docs().get(key).cloned()
    }

    fn save(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.docs().insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn remove(&self, key: &str) {
        self.docs().remove(key);
    }

    fn keys(&self, prefix: &str) -> Vec<String> {
        self.docs()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_store_keeps_documents_by_name_and_lists_them() {
        let dir = std::env::temp_dir().join(format!("lumaguide-store-{}", std::process::id()));
        let store = DirStore::new(&dir);
        store.save("plex/2026-11-05.json", b"[1]").unwrap();
        store.save("plex/2026-11-06.json", b"[2]").unwrap();
        store.save("state.json", b"{}").unwrap();
        assert_eq!(
            store.load("plex/2026-11-05.json").as_deref(),
            Some(&b"[1]"[..])
        );
        assert_eq!(
            store.keys("plex/"),
            ["plex/2026-11-05.json", "plex/2026-11-06.json"]
        );
        store.remove("plex/2026-11-05.json");
        assert_eq!(store.keys("plex/2026"), ["plex/2026-11-06.json"]);
        assert!(store.save("../out", b"x").is_err());
        assert_eq!(store.load("../state.json"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
