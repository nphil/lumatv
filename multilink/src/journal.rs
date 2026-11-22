use crate::{Device, Key, Result, Value, validate};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change { key: Key, before: Value, after: Value }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal { version: u32, boot: String, changes: Vec<Change> }

pub trait JournalStore {
    fn load(&self) -> Result<Option<Journal>>;
    fn save(&self, journal: &Journal) -> Result<()>;
}

pub struct FileJournal { dir: PathBuf }
impl FileJournal { pub fn new(dir: PathBuf) -> Self { Self { dir } } }
impl JournalStore for FileJournal {
    fn load(&self) -> Result<Option<Journal>> {
        let file = match File::open(self.dir.join("rollback.json")) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(65_537).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 65_536 { return Err("Network rollback record is too large".into()); }
        serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("Cannot read network rollback: {e}"))
    }
    fn save(&self, journal: &Journal) -> Result<()> {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.dir).map_err(|e| e.to_string())?;
        let tmp = self.dir.join("rollback.next");
        let mut file = OpenOptions::new().create(true).truncate(true).write(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&tmp).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, journal).map_err(|e| e.to_string())?;
        file.flush().and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
        fs::rename(tmp, self.dir.join("rollback.json")).map_err(|e| e.to_string())?;
        File::open(&self.dir).and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())
    }
}

/// Each original is persisted before its first write. Restoration never overwrites a later owner's value.
pub struct Transaction<D: Device, S: JournalStore> { device: D, store: S, journal: Journal, boot: String }
impl<D: Device, S: JournalStore> Transaction<D, S> {
    pub fn open(device: D, store: S, boot: String) -> Result<Self> {
        if boot.is_empty() || boot.len() > 128 { return Err("Cannot identify this boot".into()); }
        let journal = store.load()?.unwrap_or(Journal { version: 1, boot: boot.clone(), changes: Vec::new() });
        if journal.version != 1 || journal.changes.len() > 64 { return Err("Unknown network rollback format".into()); }
        for (index, c) in journal.changes.iter().enumerate() {
            validate(&c.key, &c.before)?;
            validate(&c.key, &c.after)?;
            if journal.changes[..index].iter().any(|p| p.key == c.key) { return Err("Duplicate network rollback key".into()); }
        }
        Ok(Self { device, store, journal, boot })
    }
    pub fn device(&self) -> &D { &self.device }
    pub fn pending(&self) -> bool { !self.journal.changes.is_empty() }

    pub fn set(&mut self, key: Key, after: Value) -> Result<()> {
        validate(&key, &after)?;
        if !self.device.allowed() { return Err("Network preparation is no longer enabled".into()); }
        if self.journal.boot != self.boot { return Err("Network rollback must finish before changes".into()); }
        let before = self.device.read(&key)?;
        validate(&key, &before)?;
        if before == after { return Ok(()); }
        if let Some(change) = self.journal.changes.iter().find(|c| c.key == key) {
            if before != change.after && before != change.before { return Err("Network setting changed outside this plugin".into()); }
            if after != change.after { return Err("Network topology changed; restore before preparing it again".into()); }
        } else {
            if self.journal.changes.len() >= 64 { return Err("Too many network changes".into()); }
            self.journal.changes.push(Change { key: key.clone(), before, after: after.clone() });
        }
        self.store.save(&self.journal)?;
        if !self.device.allowed() { return Err("Network preparation is no longer enabled".into()); }
        self.device.write(&key, &after)?;
        if self.device.read(&key)? != after { return Err("Network change did not read back".into()); }
        Ok(())
    }

    fn restore_index(&mut self, index: usize) -> Result<()> {
        let c = &self.journal.changes[index];
        if self.journal.boot == self.boot || c.key.radio() {
            let current = self.device.read(&c.key)?;
            if current == c.after {
                self.device.write(&c.key, &c.before)?;
                if self.device.read(&c.key)? != c.before { return Err("Network restoration did not read back".into()); }
            }
        }
        // A failed save leaves this entry in memory as well as on disk: another restore can safely repeat it.
        let change = self.journal.changes.remove(index);
        if let Err(error) = self.store.save(&self.journal) {
            self.journal.changes.insert(index, change);
            return Err(error);
        }
        Ok(())
    }

    pub fn restore(&mut self) -> Result<()> {
        let mut error = None;
        for i in (0..self.journal.changes.len()).rev() {
            if let Err(e) = self.restore_index(i) { error = Some(e); }
        }
        if let Some(error) = error { return Err(error); }
        if self.journal.boot != self.boot {
            self.journal.boot.clone_from(&self.boot);
            self.store.save(&self.journal)?;
        }
        Ok(())
    }

    /// Sysctls follow live interfaces. A borrowed host route stays wired until the transaction ends.
    pub fn network(&mut self, desired: &[(Key, Value)]) -> Result<()> {
        for i in (0..self.journal.changes.len()).rev() {
            let c = &self.journal.changes[i];
            if matches!(c.key, Key::Sysctl { .. }) && !desired.iter().any(|(key, _)| key == &c.key) { self.restore_index(i)?; }
        }
        for (key, value) in desired { self.set(key.clone(), value.clone())?; }
        Ok(())
    }
}

impl<D: Device, S: JournalStore> Drop for Transaction<D, S> {
    fn drop(&mut self) { let _ = self.restore(); }
}
