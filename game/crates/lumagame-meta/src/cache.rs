use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, time::SystemTime};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Debug, Clone, Copy)]
pub struct CacheLimits { pub bytes: u64, pub files: usize, pub image_bytes: usize }
impl Default for CacheLimits {
    fn default() -> Self { Self { bytes: 128 << 20, files: 8192, image_bytes: 16 << 20 } }
}
struct Entry { bytes: u64, used: u64 }
/// Private directory, opaque SHA-256 filenames, atomic writes and an in-memory LRU index.
/// Callers serialize access; network and decode never run under the cache lock.
pub struct Cache {
    root: PathBuf,
    limits: CacheLimits,
    entries: BTreeMap<String, Entry>,
    bytes: u64,
    clock: u64,
}
impl Cache {
    pub fn open(root: PathBuf, limits: CacheLimits) -> Result<Self> {
        if limits.image_bytes == 0 || limits.image_bytes > 16 << 20 { return Err(Error::Invalid("invalid image byte limit")); }
        fs::create_dir_all(&root)?;
        let meta = fs::symlink_metadata(&root)?;
        if !meta.is_dir() { return Err(Error::Invalid("cache directory is not a real directory")); }
        #[cfg(unix)]
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let mut found = Vec::new();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.strip_suffix(".new").is_some_and(is_key) {
                fs::remove_file(entry.path())?;
                continue;
            }
            if !is_key(&name) { continue; }
            let meta = fs::symlink_metadata(entry.path())?;
            if !meta.is_file() || meta.len() > limits.image_bytes as u64 {
                let _ = fs::remove_file(entry.path());
                continue;
            }
            found.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), name, meta.len()));
        }
        found.sort_unstable();
        let mut cache = Self { root, limits, entries: BTreeMap::new(), bytes: 0, clock: 0 };
        for (_, name, bytes) in found {
            cache.clock += 1;
            cache.bytes += bytes;
            cache.entries.insert(name, Entry { bytes, used: cache.clock });
        }
        cache.trim(0, 0)?;
        Ok(cache)
    }

    pub fn get(&mut self, source: &str) -> Result<Option<(PathBuf, Vec<u8>)>> {
        let key = key(source);
        if !self.entries.contains_key(&key) { return Ok(None); }
        let path = self.root.join(&key);
        let read = (|| {
            let meta = fs::symlink_metadata(&path)?;
            if !meta.is_file() || meta.len() > self.limits.image_bytes as u64 { return Err(Error::Invalid("unsafe cache entry")); }
            let mut bytes = Vec::with_capacity(meta.len() as usize);
            File::open(&path)?.take(self.limits.image_bytes as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > self.limits.image_bytes { return Err(Error::Invalid("cache entry exceeds limit")); }
            Ok(bytes)
        })();
        match read {
            Ok(bytes) => {
                self.clock += 1;
                if let Some(entry) = self.entries.get_mut(&key) { entry.used = self.clock; }
                // Persist recency without a second index file or retaining source URLs.
                let _ = File::options().write(true).open(&path).and_then(|f| f.set_modified(SystemTime::now()));
                Ok(Some((path, bytes)))
            }
            Err(_) => { self.remove(source)?; Ok(None) }
        }
    }

    pub fn put(&mut self, source: &str, bytes: &[u8]) -> Result<Option<PathBuf>> {
        if bytes.len() > self.limits.image_bytes || bytes.len() as u64 > self.limits.bytes || self.limits.files == 0 {
            return Ok(None);
        }
        self.remove(source)?;
        self.trim(bytes.len() as u64, 1)?;
        let key = key(source);
        let path = self.root.join(&key);
        let temporary = self.root.join(format!("{key}.new"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        let result = (|| {
            file.write_all(bytes)?;
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if let Err(e) = result { let _ = fs::remove_file(temporary); return Err(e); }
        self.clock += 1;
        self.bytes += bytes.len() as u64;
        self.entries.insert(key, Entry { bytes: bytes.len() as u64, used: self.clock });
        Ok(Some(path))
    }

    pub fn remove(&mut self, source: &str) -> Result<()> {
        let key = key(source);
        if let Some(entry) = self.entries.get(&key) {
            match fs::remove_file(self.root.join(&key)) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            self.bytes -= entry.bytes;
            self.entries.remove(&key);
        }
        Ok(())
    }

    fn trim(&mut self, incoming: u64, files: usize) -> Result<()> {
        while self.bytes.saturating_add(incoming) > self.limits.bytes || self.entries.len().saturating_add(files) > self.limits.files {
            let Some(old) = self.entries.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone()) else { break };
            fs::remove_file(self.root.join(&old))?;
            if let Some(entry) = self.entries.remove(&old) { self.bytes -= entry.bytes; }
        }
        Ok(())
    }

    pub fn bytes(&self) -> u64 { self.bytes }
    pub fn root(&self) -> &Path { &self.root }
}
fn key(source: &str) -> String { format!("{:x}", Sha256::digest(source.as_bytes())) }
fn is_key(name: &str) -> bool { name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_and_file_caps_evict_the_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let limits = CacheLimits { bytes: 8, files: 2, image_bytes: 8 };
        let mut cache = Cache::open(dir.path().join("art"), limits).unwrap();
        let first = cache.put("https://private.invalid/a?token=not-saved", b"aaaa").unwrap().unwrap();
        cache.put("b", b"bbbb").unwrap();
        assert!(cache.get("https://private.invalid/a?token=not-saved").unwrap().is_some());
        cache.put("c", b"cccc").unwrap();
        assert_eq!(cache.bytes(), 8);
        assert!(cache.get("b").unwrap().is_none());
        assert_eq!(fs::read(first).unwrap(), b"aaaa");
        assert!(cache.put("oversize", b"123456789").unwrap().is_none());
        assert_eq!(cache.bytes(), 8);
        assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 2);
        assert!(fs::read_dir(cache.root()).unwrap().all(|p| is_key(&p.unwrap().file_name().to_string_lossy())));
    }
    #[test]
    fn startup_rebuilds_accounting_and_cleans_incomplete_writes() {
        let dir = tempfile::tempdir().unwrap();
        let limits = CacheLimits { bytes: 8, files: 2, image_bytes: 8 };
        let root = dir.path().join("art");
        let mut cache = Cache::open(root.clone(), limits).unwrap();
        let a = cache.put("a", b"aaaa").unwrap().unwrap();
        let b = cache.put("b", b"bbbb").unwrap().unwrap();
        File::options().write(true).open(&b).unwrap().set_modified(SystemTime::UNIX_EPOCH).unwrap();
        File::options().write(true).open(&a).unwrap().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
        fs::write(root.join(format!("{}.new", key("interrupted"))), b"unfinished").unwrap();
        drop(cache);
        let mut cache = Cache::open(root.clone(), CacheLimits { bytes: 4, ..limits }).unwrap();
        assert_eq!(cache.bytes(), 4);
        assert!(cache.get("a").unwrap().is_some());
        assert!(cache.get("b").unwrap().is_none());
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }
    #[test]
    fn disabled_cache_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = Cache::open(dir.path().join("art"), CacheLimits { bytes: 0, files: 0, image_bytes: 8 }).unwrap();
        assert!(cache.put("a", b"a").unwrap().is_none());
        assert_eq!(cache.bytes(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_directories_and_entries_are_never_followed() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, dir.path().join("link")).unwrap();
        assert!(Cache::open(dir.path().join("link"), CacheLimits::default()).is_err());
        let root = dir.path().join("art");
        fs::create_dir(&root).unwrap();
        let private = outside.join("private");
        fs::write(&private, b"not artwork").unwrap();
        symlink(&private, root.join(key("a"))).unwrap();
        let mut cache = Cache::open(root, CacheLimits::default()).unwrap();
        assert!(cache.get("a").unwrap().is_none());
        assert_eq!(fs::read(private).unwrap(), b"not artwork");
    }
}
