use super::device::{Control, ControlFd, Devices};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const EXPIRY: Duration = Duration::from_secs(6);
pub const RENEW: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedControl {
    control: Control,
    value: i32,
    device: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    boot: String,
    controls: Vec<SavedControl>,
}

struct Entry {
    saved: SavedControl,
    fd: Box<dyn ControlFd>,
}

pub struct Holder {
    pub pid: i32,
    pub fd: OwnedFd,
}

impl Holder {
    pub fn open(pid: i32) -> io::Result<Self> {
        let proc = crate::gfn_boost::Proc::new("/proc");
        let identity = proc.app_process(pid).ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "lease holder is not a non-root gaming app process"))?;
        // SAFETY: pidfd_open takes an integer PID and zero flags, and returns a new owned fd.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        // SAFETY: the successful syscall returned a new fd owned by this Holder.
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        if proc.app_process(pid).as_deref() != Some(&identity) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "lease holder changed while opening pidfd"));
        }
        Ok(Self { pid, fd })
    }
}

pub struct Lease {
    pub id: String,
    pub holder: Holder,
    pub until: Instant,
    applied: Vec<Control>,
    skipped: Vec<Value>,
}

impl Lease {
    fn view(&self) -> Value {
        json!({"id": self.id, "pid": self.holder.pid, "leaseSecs": EXPIRY.as_secs(),
            "renewEverySecs": RENEW.as_secs(), "applied": self.applied, "skipped": self.skipped})
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Qualification {
    control: Control,
    boot: String,
    device: String,
    at: u64,
    original: Option<i32>,
    requested: i32,
    readback: Option<i32>,
    restored: Option<i32>,
    sequence_passed: bool,
    qualified: bool,
    live_readback_receipt: Option<String>,
    lifecycle_receipt: Option<String>,
    error: Option<String>,
}

/// Atomic replacement on the same filesystem, with the journal durable before any set.
fn save(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("missing gaming state directory"))?;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
    let temp = path.with_extension("new");
    let mut file = OpenOptions::new().create(true).truncate(true).write(true)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(temp, path)?;
    File::open(parent)?.sync_all()
}

pub struct State {
    pub enabled: bool,
    pub lease: Option<Lease>,
    entries: Vec<Entry>,
    journal: PathBuf,
    records_path: PathBuf,
    boot: String,
    records: BTreeMap<Control, Qualification>,
    pub error: Option<String>,
    pub last_restore: Value,
}

impl State {
    pub fn new(journal: PathBuf, records_path: PathBuf, boot: String) -> Self {
        let records = fs::read(&records_path).ok()
            .and_then(|data| serde_json::from_slice::<Vec<Qualification>>(&data).ok())
            .unwrap_or_default().into_iter().map(|record| (record.control, record)).collect();
        Self { enabled: false, lease: None, entries: Vec::new(), journal, records_path, boot, records, error: None, last_restore: Value::Null }
    }

    fn write_journal(&self) -> Result<(), String> {
        save(&self.journal, &Journal { boot: self.boot.clone(), controls: self.entries.iter().map(|entry| entry.saved.clone()).collect() })
            .map_err(|e| format!("saving rollback journal: {e}"))
    }

    fn clear_journal(&self) -> Result<(), String> {
        match fs::remove_file(&self.journal) {
            Ok(()) => File::open(self.journal.parent().unwrap()).and_then(|file| file.sync_all()).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("removing rollback journal: {e}")),
        }
    }

    /// Always called at helper start, even while disabled. A failed replay blocks new sets.
    pub fn recover(&mut self, devices: &dyn Devices) -> Result<(), String> {
        let bytes = match fs::read(&self.journal) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("reading rollback journal: {e}")),
        };
        let journal: Journal = serde_json::from_slice(&bytes).map_err(|e| format!("invalid rollback journal: {e}"))?;
        if journal.boot != self.boot { return self.clear_journal(); }
        // Open and validate every fd before restoring any value.
        let mut entries = Vec::new();
        for saved in journal.controls {
            let fd = devices.open(&[saved.control]).map_err(|e| format!("opening rollback device: {e}"))?;
            if fd.identity() != saved.device { return Err("rollback device identity changed".into()); }
            entries.push(Entry { saved, fd });
        }
        self.entries = entries;
        self.error = None;
        if self.entries.is_empty() { return self.clear_journal(); }
        self.restore("helper_restart")
    }

    pub fn restore(&mut self, reason: &str) -> Result<(), String> {
        self.lease = None;
        if self.entries.is_empty() {
            self.last_restore = json!({"reason": reason, "controls": []});
            return self.error.clone().map_or(Ok(()), Err);
        }
        let mut results = Vec::new();
        let mut pending = Vec::new();
        for mut entry in self.entries.drain(..).rev() {
            let result = entry.fd.write(entry.saved.control, entry.saved.value)
                .and_then(|()| entry.fd.read(entry.saved.control))
                .and_then(|value| if value == entry.saved.value { Ok(value) } else { Err(io::Error::other(format!("restore readback {value} differs from {}", entry.saved.value))) });
            results.push(json!({"control": entry.saved.control, "saved": entry.saved.value,
                "readback": result.as_ref().ok(), "error": result.as_ref().err().map(ToString::to_string)}));
            if result.is_err() { pending.push(entry); }
        }
        pending.reverse();
        self.entries = pending;
        self.last_restore = json!({"reason": reason, "controls": results});
        let result = if self.entries.is_empty() { self.clear_journal() } else { self.write_journal().and(Err("one or more controls failed restoration; rollback journal retained".into())) };
        self.error = result.as_ref().err().cloned();
        result
    }

    pub fn tick(&mut self, now: Instant, dead: bool, screen_on: bool) -> Result<(), String> {
        if self.lease.as_ref().is_some_and(|lease| dead || now >= lease.until || !screen_on || !self.enabled) {
            let reason = if !self.enabled { "plugin_off" } else if !screen_on { "standby" } else if dead { "holder_died" } else { "lease_expired" };
            self.restore(reason)?;
        }
        Ok(())
    }

    pub fn acquire(&mut self, holder: Holder, controls: Vec<Control>, devices: &dyn Devices, now: Instant) -> Result<Value, String> {
        if self.lease.is_some() { return Err("a gaming lease is already held".into()); }
        if self.error.is_some() || !self.entries.is_empty() { return Err("rollback must complete before another lease".into()); }
        let mut random = [0u8; 16];
        File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut random)).map_err(|e| e.to_string())?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let id: String = random.iter().flat_map(|byte| [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]])
            .map(char::from).collect();
        let mut skipped = Vec::new();
        let mut prepared = Vec::new();
        for control in controls {
            let Some(record) = self.records.get(&control).filter(|record| record.qualified && record.boot == self.boot) else {
                skipped.push(json!({"control": control, "reason": "unqualified"}));
                continue;
            };
            let mut fd = devices.open(&[control]).map_err(|e| format!("opening {control:?}: {e}"))?;
            if fd.identity() != record.device {
                skipped.push(json!({"control": control, "reason": "device_changed"}));
                continue;
            }
            let value = fd.read(control).map_err(|e| format!("saving {control:?}: {e}"))?;
            prepared.push(Entry { saved: SavedControl { control, value, device: fd.identity().to_owned() }, fd });
        }
        self.entries = prepared;
        if !self.entries.is_empty() {
            if let Err(e) = self.write_journal() { self.entries.clear(); return Err(e); }
        }
        let applied = self.entries.iter().map(|entry| entry.saved.control).collect();
        // Install the owner before the first set; all error exits take the same rollback path.
        self.lease = Some(Lease { id, holder, until: now + EXPIRY, applied, skipped });
        for entry in &mut self.entries {
            let control = entry.saved.control;
            let result = entry.fd.write(control, control.target()).and_then(|()| entry.fd.read(control))
                .and_then(|value| if value == control.target() { Ok(()) } else { Err(io::Error::other("set readback differs from requested value")) });
            if let Err(e) = result {
                let message = format!("setting {control:?}: {e}");
                let restored = self.restore("set_failed");
                return Err(format!("{message}{}", restored.err().map(|e| format!("; {e}")).unwrap_or_default()));
            }
        }
        Ok(self.lease.as_ref().unwrap().view())
    }

    pub fn renew(&mut self, id: &str, now: Instant) -> Result<Value, String> {
        let lease = self.lease.as_mut().filter(|lease| lease.id == id).ok_or("no such gaming lease")?;
        if now >= lease.until { self.restore("lease_expired")?; return Err("gaming lease expired".into()); }
        lease.until = now + EXPIRY;
        Ok(lease.view())
    }

    pub fn release(&mut self, id: &str) -> Result<(), String> {
        if !self.lease.as_ref().is_some_and(|lease| lease.id == id) { return Err("no such gaming lease".into()); }
        self.restore("stop")
    }

    pub fn qualify(&mut self, control: Control, devices: &dyn Devices, live_readback_receipt: Option<String>, lifecycle_receipt: Option<String>) -> Result<Value, String> {
        if self.lease.is_some() || self.error.is_some() || !self.entries.is_empty() { return Err("qualification requires no lease and a complete rollback".into()); }
        if self.records.remove(&control).is_some() {
            // A crash during requalification must not leave an older passing record usable.
            save(&self.records_path, &self.records.values().collect::<Vec<_>>())
                .map_err(|e| format!("revoking the previous qualification: {e}"))?;
        }
        let mut record = Qualification { control, boot: self.boot.clone(), device: String::new(), at: crate::timeutil::now_ms(), original: None, requested: control.target(), readback: None, restored: None, sequence_passed: false, qualified: false, live_readback_receipt, lifecycle_receipt, error: None };
        let sequence = (|| -> Result<(), String> {
            let mut fd = devices.open(&[control]).map_err(|e| format!("discovery: {e}"))?;
            record.device = fd.identity().to_owned();
            let value = fd.read(control).map_err(|e| format!("original read: {e}"))?;
            record.original = Some(value);
            self.entries.push(Entry { saved: SavedControl { control, value, device: record.device.clone() }, fd });
            if let Err(e) = self.write_journal() { self.entries.clear(); return Err(e); }
            let entry = &mut self.entries[0];
            let set = entry.fd.write(control, control.target()).and_then(|()| entry.fd.read(control));
            if let Ok(readback) = set.as_ref() { record.readback = Some(*readback); }
            let restore = self.restore("qualification");
            record.restored = self.last_restore["controls"][0]["readback"].as_i64().and_then(|n| i32::try_from(n).ok());
            set.map_err(|e| format!("set/readback: {e}"))?;
            restore?;
            if record.readback != Some(control.target()) { return Err("set readback differs from requested value".into()); }
            if record.restored != record.original { return Err("restored readback differs from original value".into()); }
            Ok(())
        })();
        record.sequence_passed = sequence.is_ok();
        record.error = sequence.err();
        // A getter round-trip cannot prove helper-death recovery or live hardware state.
        // Those TV receipts are explicit rather than inferred from de_kwrap's cached getter.
        record.qualified = record.sequence_passed && record.lifecycle_receipt.is_some()
            && (!control.cache_caveat() || record.live_readback_receipt.is_some());
        let value = serde_json::to_value(&record).map_err(|e| e.to_string())?;
        self.records.insert(control, record);
        save(&self.records_path, &self.records.values().collect::<Vec<_>>()).map_err(|e| {
            if let Some(record) = self.records.get_mut(&control) { record.qualified = false; }
            format!("saving qualification result: {e}")
        })?;
        Ok(value)
    }

    pub fn status(&self) -> Value {
        json!({"enabled": self.enabled, "lease": self.lease.as_ref().map(Lease::view),
            "qualifications": self.records.values().collect::<Vec<_>>(), "restorePending": self.entries.len(),
            "error": self.error, "lastRestore": self.last_restore,
            "cacheCaveat": "de_kwrap caches low-delay state; DE/VPQ low-delay qualification requires a live-hardware readback receipt",
            "controls": Control::ALL})
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
