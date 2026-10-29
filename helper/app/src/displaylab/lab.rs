//! The display lab's session: what is read before a write, what is kept so that a restart can put it back, and every way the lab ends.
//!
//! One session at a time. [`Lab::begin`] reads every target (reads only), judges which controls can be used (an original that is not
//! one the set could hold makes its control unusable, never a guess) and starts the dead-man's lease. [`Lab::set`] writes one option of one
//! control: the journal first (the originals and the values about to be written, on disk, before the first byte), then the writes, then a
//! read-back of each. Whatever ends the session puts every target it touched back to the original it read, with a read-back of that too:
//!
//! | Way out | Who |
//! |---|---|
//! | the lab says so (`end`, the lab's Back, Home, its last step) | the shell, `POST /v1/displaylab/end` |
//! | the lab is not heard from for [`DEADMAN_MS`] (the shell died, froze, the screen went off, the link broke) | [`Lab::tick`], the helper's own thread |
//! | the helper stops (SIGTERM, `lumatv-helper stop`) | [`Lab::shutdown`] |
//! | the helper died with something changed (killed, crashed, the TV lost power and came back) | [`Lab::recover`] at the next start, from the journal |
//! | a write or its read-back fails | the same call, before it answers |
//!
//! A restore that cannot be completed (a write that fails) is kept as pending and tried again every two seconds, and the lab starts no new
//! session until it is done. The journal is removed only when every target reads its original again. At start-up a target that does not hold
//! what the lab wrote is left alone: the TV rebooted since, or the firmware changed it, and writing the old original over it would be
//! the lab changing something it did not change.

use super::allow::{Control, Target, permitted, plan, plausible_original};
use super::hw::Regs;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// How long the lab may stay silent before everything is put back.
pub const DEADMAN_MS: u64 = 6000;
/// How often a restore that did not complete is tried again.
const RETRY_MS: u64 = 2000;
/// Tries at one target in one restore.
const TRIES: usize = 3;

/// Why a call of the lab was refused or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabError {
    /// Not the set, or no access to it: nothing is touched.
    Unavailable(String),
    /// No session is open (never begun, or ended: the dead-man, a restore).
    NoSession,
    /// Not on the list: a control or an option the lab does not have.
    NotAllowed(String),
    /// The control cannot be used: a target reads something the set could not hold, so its original is not known.
    NotOriginal(String),
    /// The journal could not be written: nothing is written to the hardware without it.
    Journal(String),
    /// A read or a write failed.
    Hardware(String),
    /// A write did not read back.
    Verify(String),
    /// A restore is still failing: no new session starts until it has completed.
    Stuck(String),
}

impl LabError {
    pub fn code(&self) -> &'static str {
        match self {
            LabError::Unavailable(_) => "unavailable",
            LabError::NoSession => "no_session",
            LabError::NotAllowed(_) => "not_allowed",
            LabError::NotOriginal(_) => "not_original",
            LabError::Journal(_) => "journal",
            LabError::Hardware(_) => "hardware",
            LabError::Verify(_) => "verify_failed",
            LabError::Stuck(_) => "stuck",
        }
    }

    pub fn message(&self) -> String {
        match self {
            LabError::NoSession => {
                "no lab session is open (it ended, or was never begun)".to_owned()
            }
            LabError::Unavailable(m)
            | LabError::NotAllowed(m)
            | LabError::NotOriginal(m)
            | LabError::Journal(m)
            | LabError::Hardware(m)
            | LabError::Verify(m)
            | LabError::Stuck(m) => m.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Entry {
    orig: u32,
    written: u32,
}

/// The journal: what a restart needs to put the hardware back.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct JournalDoc {
    v: u32,
    session: u64,
    started_ms: u64,
    pid: u32,
    /// By target name: what it read before the lab wrote, and what the lab last wrote.
    targets: BTreeMap<String, Entry>,
}

pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn load(&self) -> Result<Option<JournalDoc>, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("the journal is not readable: {e}")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("the journal could not be read: {e}")),
        }
    }

    /// Atomically, and on disk before this returns: a temporary beside it, synced, renamed, and the directory synced.
    fn save(&self, doc: &JournalDoc) -> io::Result<()> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("the journal has no directory"))?;
        std::fs::create_dir_all(dir)?;
        let tmp = self.path.with_extension("tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec(doc).map_err(io::Error::other)?)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    fn clear(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// What a session knows.
struct Session {
    id: u64,
    started_ms: u64,
    deadline_ms: u64,
    /// What every target read before anything was written.
    originals: BTreeMap<Target, u32>,
    /// Which controls can be used, and why not for the others.
    usable: BTreeMap<Control, Result<(), String>>,
    /// What the lab last wrote to each target it has touched.
    written: BTreeMap<Target, u32>,
    /// The option each control shows now.
    shown: BTreeMap<Control, &'static str>,
    sets: u32,
}

/// How a session ended, for the status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub why: &'static str,
    pub at_ms: u64,
    pub ok: bool,
    pub detail: String,
}

/// A restore that did not complete: the targets still to put back, with what to put and what the lab left there.
struct Pending {
    items: Vec<(Target, u32, u32)>,
    why: &'static str,
    next_ms: u64,
}

struct Inner {
    session: Option<Session>,
    pending: Option<Pending>,
    last: Option<Ended>,
    counter: u64,
}

pub struct Lab {
    regs: Arc<dyn Regs>,
    clock: Clock,
    journal: Journal,
    deadman_ms: u64,
    inner: Mutex<Inner>,
}

/// The order targets are put back in: each control's own (a window's select before its rectangle).
fn restore_order() -> Vec<Target> {
    Control::ALL
        .iter()
        .flat_map(|c| c.targets().iter().copied())
        .collect()
}

impl Lab {
    pub fn new(regs: Arc<dyn Regs>, clock: Clock, journal: Journal, deadman_ms: u64) -> Self {
        Self {
            regs,
            clock,
            journal,
            deadman_ms,
            inner: Mutex::new(Inner {
                session: None,
                pending: None,
                last: None,
                counter: 0,
            }),
        }
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Whether the hardware can be reached, and why not.
    pub fn check(&self) -> Result<(), String> {
        self.regs.check()
    }

    // ---- the session

    /// Starts the session, or finds the one that is open (the lease is renewed). Reads only.
    pub fn begin(&self) -> Result<Value, LabError> {
        self.regs.check().map_err(LabError::Unavailable)?;
        let now = self.now();
        let mut g = self.inner.lock();
        if let Some(s) = g.session.as_mut() {
            s.deadline_ms = now + self.deadman_ms;
            return Ok(self.status_of(&g, now));
        }
        if g.pending.is_some() {
            self.retry_pending(&mut g, now);
            if let Some(p) = &g.pending {
                return Err(LabError::Stuck(format!(
                    "the last session is not put back yet: {}",
                    describe(&p.items)
                )));
            }
        }
        match self.journal.load() {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => {
                // a journal with no session in this process: put back what it names first, as a start-up would
                self.recover_locked(&mut g, now);
                if let Some(p) = &g.pending {
                    return Err(LabError::Stuck(format!(
                        "a journal of an earlier run is not put back yet: {}",
                        describe(&p.items)
                    )));
                }
            }
        }
        let mut originals = BTreeMap::new();
        let mut reads: BTreeMap<Target, Result<u32, String>> = BTreeMap::new();
        for t in Target::ALL {
            match self.regs.read(t) {
                Ok(v) => {
                    originals.insert(t, v);
                    reads.insert(t, Ok(v));
                }
                Err(e) => {
                    reads.insert(t, Err(format!("{} could not be read: {e}", t.name())));
                }
            }
        }
        let mut usable = BTreeMap::new();
        for c in Control::ALL {
            let verdict = c.targets().iter().try_for_each(|t| match &reads[t] {
                Ok(v) if plausible_original(*t, *v) => Ok(()),
                Ok(v) => Err(format!(
                    "{} reads {v:#x}, which is not the set's own state",
                    t.name()
                )),
                Err(e) => Err(e.clone()),
            });
            usable.insert(c, verdict);
        }
        if usable.values().all(Result::is_err) {
            let why = usable
                .values()
                .filter_map(|v| v.as_ref().err())
                .cloned()
                .collect::<Vec<_>>()
                .join("; ");
            return Err(LabError::Unavailable(format!(
                "no control can be used: {why}"
            )));
        }
        g.counter += 1;
        let id = g.counter;
        let shown = Control::ALL
            .iter()
            .map(|c| (*c, c.options()[0].id))
            .collect();
        crate::log_info!(
            "displaylab: session {id} begins; originals {}",
            originals
                .iter()
                .map(|(t, v)| format!("{}={v:#x}", t.name()))
                .collect::<Vec<_>>()
                .join(" ")
        );
        g.session = Some(Session {
            id,
            started_ms: now,
            deadline_ms: now + self.deadman_ms,
            originals,
            usable,
            written: BTreeMap::new(),
            shown,
            sets: 0,
        });
        Ok(self.status_of(&g, now))
    }

    /// Shows `option` of `control`: the journal, the writes, the read-back.
    pub fn set(&self, control: &str, option: &str) -> Result<Value, LabError> {
        let now = self.now();
        let mut g = self.inner.lock();
        let Some(session) = g.session.as_mut() else {
            return Err(LabError::NoSession);
        };
        let c = Control::from_id(control)
            .ok_or_else(|| LabError::NotAllowed(format!("there is no control {control:?}")))?;
        c.option(option)
            .ok_or_else(|| LabError::NotAllowed(format!("{control} has no option {option:?}")))?;
        if let Err(why) = &session.usable[&c] {
            return Err(LabError::NotOriginal(why.clone()));
        }
        let writes = plan(c, option, |t| session.originals.get(&t).copied()).ok_or_else(|| {
            LabError::NotAllowed(format!("{control} {option} is not a plan the lab has"))
        })?;
        for (t, v) in &writes {
            if !permitted(*t, *v, session.originals.get(t).copied()) {
                return Err(LabError::NotAllowed(format!(
                    "{v:#x} is not on the list for {}",
                    t.name()
                )));
            }
        }
        // the journal first: the originals and the values about to be written, on disk before the first byte goes out
        let mut doc = JournalDoc {
            v: 1,
            session: session.id,
            started_ms: session.started_ms,
            pid: std::process::id(),
            targets: BTreeMap::new(),
        };
        for (t, v) in session
            .written
            .iter()
            .chain(writes.iter().map(|(t, v)| (t, v)))
        {
            doc.targets.insert(
                t.name().to_owned(),
                Entry {
                    orig: session.originals[t],
                    written: *v,
                },
            );
        }
        if let Err(e) = self.journal.save(&doc) {
            return Err(LabError::Journal(format!(
                "the journal could not be written, so nothing was: {e}"
            )));
        }
        for (t, v) in &writes {
            session.written.insert(*t, *v);
        }
        let mut failure = None;
        for (t, v) in &writes {
            let mut ok = false;
            for attempt in 0..2 {
                if let Err(e) = self.regs.write(*t, *v) {
                    failure = Some(LabError::Hardware(format!(
                        "{} could not be written: {e}",
                        t.name()
                    )));
                    break;
                }
                match self.regs.read(*t) {
                    Ok(read) if read == *v => {
                        ok = true;
                        break;
                    }
                    Ok(read) => {
                        failure = Some(LabError::Verify(format!(
                            "{} reads {read:#x} after {v:#x} was written{}",
                            t.name(),
                            if attempt == 0 { ", once more" } else { "" }
                        )))
                    }
                    Err(e) => {
                        failure = Some(LabError::Hardware(format!(
                            "{} could not be read back: {e}",
                            t.name()
                        )))
                    }
                }
            }
            if !ok {
                break;
            }
            failure = None;
        }
        if let Some(e) = failure {
            crate::log_warn!(
                "displaylab: {control} {option}: {}; putting everything back",
                e.message()
            );
            self.end_locked(&mut g, "failed", now);
            return Err(e);
        }
        let session = g.session.as_mut().expect("the session is open");
        session
            .shown
            .insert(c, c.option(option).expect("checked").id);
        session.sets += 1;
        session.deadline_ms = now + self.deadman_ms;
        Ok(self.status_of(&g, now))
    }

    /// The lab is there: the lease is renewed, and the answer says whether what it wrote still holds.
    pub fn ping(&self) -> Result<Value, LabError> {
        let now = self.now();
        let mut g = self.inner.lock();
        let Some(session) = g.session.as_mut() else {
            return Err(LabError::NoSession);
        };
        session.deadline_ms = now + self.deadman_ms;
        Ok(self.status_of(&g, now))
    }

    /// The lab is done: everything it touched is put back. Never an error for a session that is already gone.
    pub fn end(&self) -> Result<Value, LabError> {
        let now = self.now();
        let mut g = self.inner.lock();
        self.end_locked(&mut g, "end", now);
        match &g.pending {
            Some(p) => Err(LabError::Stuck(format!(
                "not everything could be put back: {}",
                describe(&p.items)
            ))),
            None => Ok(self.status_of(&g, now)),
        }
    }

    /// The dead-man: called every few hundred milliseconds by the helper's own thread.
    pub fn tick(&self) {
        let now = self.now();
        let mut g = self.inner.lock();
        if g.session.as_ref().is_some_and(|s| now >= s.deadline_ms) {
            crate::log_warn!(
                "displaylab: the lab was not heard from for {} ms; putting everything back",
                self.deadman_ms
            );
            self.end_locked(&mut g, "deadman", now);
        } else if g.pending.as_ref().is_some_and(|p| now >= p.next_ms) {
            self.retry_pending(&mut g, now);
        }
    }

    /// The helper is stopping.
    pub fn shutdown(&self) {
        let now = self.now();
        let mut g = self.inner.lock();
        if g.session.is_some() {
            crate::log_info!("displaylab: the helper is stopping; putting everything back");
            self.end_locked(&mut g, "shutdown", now);
        } else if g.pending.is_some() {
            self.retry_pending(&mut g, now);
        }
    }

    /// At start-up: what an earlier helper left in the journal is put back, where the target still holds what the lab wrote.
    pub fn recover(&self) {
        let now = self.now();
        let mut g = self.inner.lock();
        self.recover_locked(&mut g, now);
    }

    // ---- putting back

    /// Puts every target the session touched back to its original and closes the session. A target that will not go stays in `pending`.
    fn end_locked(&self, g: &mut Inner, why: &'static str, now: u64) {
        let Some(s) = g.session.take() else { return };
        let touched: Vec<(Target, u32, u32)> = restore_order()
            .into_iter()
            .filter_map(|t| s.written.get(&t).map(|w| (t, s.originals[&t], *w)))
            .collect();
        let detail = format!("session {} after {} changes", s.id, s.sets);
        self.put_back(g, touched, why, now, detail);
    }

    fn put_back(
        &self,
        g: &mut Inner,
        items: Vec<(Target, u32, u32)>,
        why: &'static str,
        now: u64,
        detail: String,
    ) {
        let left = self.restore_items(&items);
        if left.is_empty() {
            if let Err(e) = self.journal.clear() {
                crate::log_warn!("displaylab: the journal could not be removed: {e}");
            }
            g.pending = None;
            g.last = Some(Ended {
                why,
                at_ms: now,
                ok: true,
                detail,
            });
            crate::log_info!("displaylab: everything is back ({why})");
        } else {
            crate::log_error!(
                "displaylab: {why}: {} did not go back; trying again every {} ms",
                describe(&left),
                RETRY_MS
            );
            g.last = Some(Ended {
                why,
                at_ms: now,
                ok: false,
                detail: format!("not put back: {}", describe(&left)),
            });
            g.pending = Some(Pending {
                items: left,
                why,
                next_ms: now + RETRY_MS,
            });
        }
    }

    /// Writes each item's original and reads it back; the items that did not go back are returned.
    fn restore_items(&self, items: &[(Target, u32, u32)]) -> Vec<(Target, u32, u32)> {
        let mut left = Vec::new();
        for item @ (t, orig, _) in items {
            if !permitted(*t, *orig, Some(*orig)) {
                // an original that could not be one is never written; the journal keeps it for a person to look at
                left.push(*item);
                continue;
            }
            let back = (0..TRIES).any(|_| {
                self.regs.write(*t, *orig).is_ok() && self.regs.read(*t).is_ok_and(|v| v == *orig)
            });
            if !back {
                left.push(*item);
            }
        }
        left
    }

    fn retry_pending(&self, g: &mut Inner, now: u64) {
        let Some(p) = g.pending.take() else { return };
        let why = p.why;
        let n = p.items.len();
        self.put_back(g, p.items, why, now, format!("a retry for {n} targets"));
    }

    fn recover_locked(&self, g: &mut Inner, now: u64) {
        let doc = match self.journal.load() {
            Ok(None) => return,
            Ok(Some(doc)) => doc,
            Err(e) => {
                // a journal nobody can read names nothing to put back; it is set aside so that it does not stop the next run
                crate::log_warn!("displaylab: {e}; set aside");
                let _ = std::fs::rename(
                    self.journal.path(),
                    self.journal.path().with_extension("bad"),
                );
                g.last = Some(Ended {
                    why: "recover",
                    at_ms: now,
                    ok: false,
                    detail: e,
                });
                return;
            }
        };
        if let Err(e) = self.regs.check() {
            // not the set (or no access): nothing can be read or written; the journal stays for a start that can
            crate::log_warn!(
                "displaylab: a journal of session {} is waiting and the hardware is not reachable: {e}",
                doc.session
            );
            g.last = Some(Ended {
                why: "recover",
                at_ms: now,
                ok: false,
                detail: e,
            });
            return;
        }
        let mut items = Vec::new();
        let mut left_alone = Vec::new();
        for (name, entry) in &doc.targets {
            let Some(t) = Target::from_name(name) else {
                left_alone.push(format!("{name}: not a target of the lab"));
                continue;
            };
            if !permitted(t, entry.orig, Some(entry.orig))
                || !(super::allow::fixed_values(t).contains(&entry.written)
                    || permitted(t, entry.written, Some(entry.written)))
            {
                left_alone.push(format!(
                    "{name}: the journal holds values the lab does not write"
                ));
                continue;
            }
            match self.regs.read(t) {
                Ok(v) if v == entry.orig => {}
                Ok(v) if v == entry.written => items.push((t, entry.orig, entry.written)),
                Ok(v) => left_alone.push(format!(
                    "{name} reads {v:#x}: neither the lab's {:#x} nor the original {:#x}",
                    entry.written, entry.orig
                )),
                Err(e) => left_alone.push(format!("{name} could not be read: {e}")),
            }
        }
        let detail = format!(
            "journal of session {} (pid {}): {} to put back, {} left alone{}",
            doc.session,
            doc.pid,
            items.len(),
            left_alone.len(),
            if left_alone.is_empty() {
                String::new()
            } else {
                format!(" ({})", left_alone.join("; "))
            }
        );
        crate::log_info!("displaylab: {detail}");
        let order = restore_order();
        items.sort_by_key(|(t, _, _)| order.iter().position(|o| o == t));
        self.put_back(g, items, "recover", now, detail);
    }

    // ---- what the status says

    fn status_of(&self, g: &Inner, now: u64) -> Value {
        let mut doc = json!({"available": true});
        doc["controls"] = Control::ALL
            .iter()
            .map(|c| {
                (
                    c.id().to_owned(),
                    json!(c.options().iter().map(|o| o.id).collect::<Vec<_>>()),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        doc["deadmanMs"] = json!(self.deadman_ms);
        doc["session"] = match &g.session {
            None => Value::Null,
            Some(s) => {
                let (held, drifted) = self.holds(s);
                let mut usable = serde_json::Map::new();
                let mut why = serde_json::Map::new();
                for (c, v) in &s.usable {
                    usable.insert(c.id().to_owned(), json!(v.is_ok()));
                    if let Err(e) = v {
                        why.insert(c.id().to_owned(), json!(e));
                    }
                }
                json!({
                    "id": s.id,
                    "leftMs": s.deadline_ms.saturating_sub(now),
                    "shown": s.shown.iter().map(|(c, o)| (c.id().to_owned(), json!(o))).collect::<serde_json::Map<_, _>>(),
                    "sets": s.sets,
                    "usable": usable,
                    "why": why,
                    "held": held,
                    "drifted": drifted,
                })
            }
        };
        doc["last"] = match &g.last {
            None => Value::Null,
            Some(e) => json!({"why": e.why, "atMs": e.at_ms, "ok": e.ok, "detail": e.detail}),
        };
        doc["pending"] = match &g.pending {
            None => Value::Null,
            Some(p) => json!(describe(&p.items)),
        };
        doc
    }

    /// Whether every target the lab wrote still reads what it wrote, and the names of those that do not.
    fn holds(&self, s: &Session) -> (bool, Vec<&'static str>) {
        let drifted: Vec<&'static str> = s
            .written
            .iter()
            .filter(|(t, w)| !self.regs.read(**t).is_ok_and(|v| v == **w))
            .map(|(t, _)| t.name())
            .collect();
        (drifted.is_empty(), drifted)
    }

    /// The status without touching the lease: what a page that only looks asks.
    pub fn status(&self) -> Value {
        let now = self.now();
        match self.regs.check() {
            Err(why) => {
                json!({"available": false, "reason": why, "session": Value::Null, "controls": Control::ALL.iter().map(|c| (c.id().to_owned(), json!(c.options().iter().map(|o| o.id).collect::<Vec<_>>()))).collect::<serde_json::Map<_, _>>()})
            }
            Ok(()) => {
                let g = self.inner.lock();
                self.status_of(&g, now)
            }
        }
    }

    /// Whether a session is open (for the tests and the thread).
    pub fn open(&self) -> bool {
        self.inner.lock().session.is_some()
    }

    /// Whether anything is open or still to be put back: what the watching thread keeps a close eye on.
    pub fn busy(&self) -> bool {
        let g = self.inner.lock();
        g.session.is_some() || g.pending.is_some()
    }
}

fn describe(items: &[(Target, u32, u32)]) -> String {
    items
        .iter()
        .map(|(t, o, w)| format!("{} (wrote {w:#x}, original {o:#x})", t.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::displaylab::hw::fake::FakeRegs;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub struct Rig {
        pub lab: Arc<Lab>,
        pub regs: Arc<FakeRegs>,
        pub time: Arc<AtomicU64>,
        pub dir: tempfile::TempDir,
    }

    impl Default for Rig {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Rig {
        pub fn new() -> Rig {
            Self::with_regs(FakeRegs::the_set())
        }

        pub fn with_regs(regs: FakeRegs) -> Rig {
            let dir = tempfile::tempdir().unwrap();
            let regs = Arc::new(regs);
            let time = Arc::new(AtomicU64::new(1_000_000));
            let clock: Clock = {
                let t = time.clone();
                Arc::new(move || t.load(Ordering::SeqCst))
            };
            let lab = Arc::new(Lab::new(
                regs.clone(),
                clock,
                Journal::new(dir.path().join("displaylab/journal.json")),
                DEADMAN_MS,
            ));
            Rig {
                lab,
                regs,
                time,
                dir,
            }
        }

        pub fn advance(&self, ms: u64) {
            self.time.fetch_add(ms, Ordering::SeqCst);
        }

        /// A new lab over the same hardware and journal (a helper that was started again).
        pub fn restarted(&self) -> Arc<Lab> {
            let clock: Clock = {
                let t = self.time.clone();
                Arc::new(move || t.load(Ordering::SeqCst))
            };
            Arc::new(Lab::new(
                self.regs.clone(),
                clock,
                Journal::new(self.dir.path().join("displaylab/journal.json")),
                DEADMAN_MS,
            ))
        }

        pub fn journal_exists(&self) -> bool {
            self.dir.path().join("displaylab/journal.json").exists()
        }
    }

    /// Every target of the set at its original (the gain 0x10, the sharpener on, the VSP at the boot's, the holes clear).
    pub fn at_rest(r: &Rig) -> bool {
        Target::ALL
            .iter()
            .all(|t| r.regs.get(*t) == r.regs.original(*t))
    }

    fn shown(v: &Value, control: &str) -> String {
        v["session"]["shown"][control]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    #[test]
    fn a_session_reads_first_and_writes_nothing_until_an_option_is_shown() {
        let r = Rig::new();
        let v = r.lab.begin().unwrap();
        assert_eq!(v["available"], true);
        assert_eq!(v["session"]["id"], 1);
        assert_eq!(
            v["session"]["usable"],
            json!({"sharpness": true, "dimming": true, "window": true})
        );
        assert_eq!(
            (
                shown(&v, "sharpness"),
                shown(&v, "dimming"),
                shown(&v, "window")
            ),
            ("normal".into(), "normal".into(), "off".into())
        );
        assert!(
            r.regs.writes().is_empty(),
            "begin only reads: {:?}",
            r.regs.writes()
        );
        assert!(!r.journal_exists(), "nothing to put back yet");
        assert!(r.lab.open());
        // begin again finds the session and renews its lease
        r.advance(5000);
        let again = r.lab.begin().unwrap();
        assert_eq!(again["session"]["id"], 1);
        assert_eq!(again["session"]["leftMs"], DEADMAN_MS);
    }

    #[test]
    fn showing_an_option_writes_the_journal_first_then_the_values_then_reads_them_back() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        // the journal is on disk before the register is touched: the fake looks at the disk at the moment of each write
        r.regs.on_write({
            let path = r.dir.path().join("displaylab/journal.json");
            move |t, v| {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("no journal when {} was written", t.name()));
                let doc: JournalDoc = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    doc.targets[t.name()].written,
                    v,
                    "the journal already names what is about to be written to {}",
                    t.name()
                );
            }
        });
        let v = r.lab.set("dimming", "dimmed").unwrap();
        assert_eq!(shown(&v, "dimming"), "dimmed");
        assert_eq!(r.regs.get(Target::Vsp), 0x0100_0001);
        assert_eq!(r.regs.writes(), vec![(Target::Vsp, 0x0100_0001)]);
        assert_eq!(v["session"]["held"], true);
        assert_eq!(v["session"]["sets"], 1);
        let doc: JournalDoc = serde_json::from_str(
            &std::fs::read_to_string(r.dir.path().join("displaylab/journal.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            doc.targets["osd0_vsp"],
            Entry {
                orig: r.regs.original(Target::Vsp),
                written: 0x0100_0001
            }
        );
        assert_eq!(doc.targets.len(), 1, "only what was touched");
        // a second control joins the journal
        r.lab.set("sharpness", "strong").unwrap();
        let doc: JournalDoc = serde_json::from_str(
            &std::fs::read_to_string(r.dir.path().join("displaylab/journal.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            doc.targets.keys().map(String::as_str).collect::<Vec<_>>(),
            ["osd0_vsp", "osd_sr", "osd_sr_gain"]
        );
        assert_eq!(
            doc.targets["osd_sr_gain"],
            Entry {
                orig: 0x10,
                written: 0x80
            }
        );
    }

    #[test]
    fn the_whole_run_of_the_shell_ends_with_everything_where_it_was() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        for (c, o) in [
            ("sharpness", "strong"),
            ("sharpness", "normal"),
            ("sharpness", "soft"),
            ("sharpness", "off"),
            ("dimming", "dimmed"),
            ("dimming", "grey"),
            ("window", "on_a"),
            ("window", "on_b"),
            ("window", "off"),
        ] {
            let v = r.lab.set(c, o).unwrap_or_else(|e| panic!("{c} {o}: {e:?}"));
            assert_eq!(shown(&v, c), o);
            assert_eq!(v["session"]["held"], true, "{c} {o}");
        }
        assert_eq!(r.regs.get(Target::HoleSel), 0, "the window is closed again");
        let done = r.lab.end().unwrap();
        assert_eq!(done["session"], Value::Null);
        assert_eq!(done["last"]["why"], "end");
        assert_eq!(done["last"]["ok"], true);
        assert!(at_rest(&r), "every register reads its original");
        assert!(!r.journal_exists());
        assert!(
            r.regs
                .writes()
                .iter()
                .all(|(t, v)| crate::displaylab::allow::allowed(*t, *v)),
            "every write was on the list"
        );
    }

    #[test]
    fn anything_that_is_not_a_control_and_an_option_of_the_list_is_refused_and_nothing_is_written()
    {
        let r = Rig::new();
        assert_eq!(
            r.lab.set("dimming", "dimmed"),
            Err(LabError::NoSession),
            "no session, no write"
        );
        r.lab.begin().unwrap();
        for (c, o) in [
            ("dimming", "bright"),
            ("sharpness", "dimmed"),
            ("", ""),
            ("vsp", "0x01000001"),
            ("0xCC480360", "1"),
            ("sharpness", "strong; reboot"),
            ("window", "on_c"),
            ("Window", "on_a"),
            ("osd_sr_gain", "255"),
        ] {
            let e = r.lab.set(c, o).unwrap_err();
            assert_eq!(e.code(), "not_allowed", "{c:?} {o:?}: {e:?}");
        }
        assert!(r.regs.writes().is_empty(), "{:?}", r.regs.writes());
        assert!(!r.journal_exists());
        assert!(at_rest(&r));
    }

    #[test]
    fn a_lab_that_is_not_heard_from_is_put_back_by_the_dead_man() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("sharpness", "strong").unwrap();
        r.lab.set("dimming", "grey").unwrap();
        r.lab.set("window", "on_a").unwrap();
        assert_eq!(r.regs.get(Target::HoleSel), 1);
        // not yet: the lease is six seconds from the last word
        r.advance(DEADMAN_MS - 1);
        r.lab.tick();
        assert!(r.lab.open() && r.regs.get(Target::Vsp) == 0x0200_0003);
        // a ping is a word
        r.lab.ping().unwrap();
        r.advance(DEADMAN_MS - 1);
        r.lab.tick();
        assert!(r.lab.open(), "the ping renewed the lease");
        // silence
        r.advance(1);
        r.lab.tick();
        assert!(!r.lab.open());
        assert!(at_rest(&r), "everything is back");
        assert!(!r.journal_exists());
        let v = r.lab.status();
        assert_eq!(
            (v["last"]["why"].as_str(), v["last"]["ok"].as_bool()),
            (Some("deadman"), Some(true))
        );
        // the lab that asks next learns it
        assert_eq!(r.lab.ping(), Err(LabError::NoSession));
        assert_eq!(
            r.lab.set("dimming", "grey"),
            Err(LabError::NoSession),
            "a late write is refused, not applied"
        );
        assert!(at_rest(&r));
    }

    #[test]
    fn a_set_is_a_word_too_and_a_session_that_never_wrote_has_nothing_to_put_back() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.advance(DEADMAN_MS - 10);
        r.lab.set("dimming", "dimmed").unwrap();
        r.advance(DEADMAN_MS - 10);
        r.lab.tick();
        assert!(r.lab.open(), "the set renewed the lease");
        r.lab.end().unwrap();
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.advance(DEADMAN_MS);
        r.lab.tick();
        assert!(!r.lab.open());
        assert!(
            r.regs.writes().is_empty(),
            "a session that wrote nothing writes nothing to put back"
        );
    }

    #[test]
    fn the_helper_stopping_puts_everything_back() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("sharpness", "soft").unwrap();
        r.lab.set("window", "on_b").unwrap();
        r.lab.shutdown();
        assert!(at_rest(&r));
        assert!(!r.lab.open() && !r.journal_exists());
        assert_eq!(r.lab.status()["last"]["why"], "shutdown");
    }

    #[test]
    fn a_helper_that_died_with_something_changed_puts_it_back_from_the_journal_at_its_next_start() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("sharpness", "strong").unwrap();
        r.lab.set("dimming", "grey").unwrap();
        r.lab.set("window", "on_a").unwrap();
        assert!(!at_rest(&r));
        // the process is gone: its journal is on disk, its registers are as it left them
        let next = r.restarted();
        assert!(r.journal_exists());
        next.recover();
        assert!(at_rest(&r), "{:?}", r.regs.dump());
        assert!(!r.journal_exists());
        let v = next.status();
        assert_eq!(
            (v["last"]["why"].as_str(), v["last"]["ok"].as_bool()),
            (Some("recover"), Some(true))
        );
        assert!(!next.open(), "a recovery is not a session");
        // the window's select went before its rectangle
        let writes = r.regs.writes();
        let at = |t: Target| writes.iter().rposition(|w| w.0 == t).unwrap();
        assert!(
            at(Target::HoleSel) < at(Target::HoleX) && at(Target::HoleSel) < at(Target::HoleY),
            "{writes:?}"
        );
    }

    #[test]
    fn a_target_that_does_not_hold_what_the_lab_wrote_is_left_alone_at_start() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("dimming", "dimmed").unwrap();
        r.lab.set("sharpness", "strong").unwrap();
        // the TV rebooted in between: the VSP is whatever the firmware made it, the sharpener's gain is still the lab's
        r.regs.force(Target::Vsp, 0x0200_0001);
        let before = r.regs.writes().len();
        let next = r.restarted();
        next.recover();
        assert_eq!(
            r.regs.get(Target::Vsp),
            0x0200_0001,
            "not the lab's to change"
        );
        assert_eq!(
            r.regs.get(Target::Gain),
            0x10,
            "what it had written is put back"
        );
        let after: Vec<_> = r.regs.writes()[before..].to_vec();
        assert!(after.iter().all(|(t, _)| *t != Target::Vsp), "{after:?}");
        assert!(
            next.status()["last"]["detail"]
                .as_str()
                .unwrap()
                .contains("left alone"),
            "{}",
            next.status()
        );
        assert!(
            !r.journal_exists(),
            "nothing more to do: the journal is done"
        );
    }

    #[test]
    fn a_journal_nobody_can_read_names_nothing_and_is_set_aside() {
        let r = Rig::new();
        std::fs::create_dir_all(r.dir.path().join("displaylab")).unwrap();
        std::fs::write(
            r.dir.path().join("displaylab/journal.json"),
            b"{\"v\": 1, \"targets\": {\"vsp\": ",
        )
        .unwrap();
        r.lab.recover();
        assert!(r.regs.writes().is_empty());
        assert!(!r.journal_exists() && r.dir.path().join("displaylab/journal.bad").exists());
        // names the lab does not have, and values it does not write, are never written either
        let doc = json!({"v": 1, "session": 9, "started_ms": 1, "pid": 1, "targets": {"osd0_vsp": {"orig": 0xFFFF_FFFFu32, "written": 0x0100_0001}, "dev_mem": {"orig": 0, "written": 1}}});
        std::fs::write(
            r.dir.path().join("displaylab/journal.json"),
            doc.to_string(),
        )
        .unwrap();
        r.regs.force(Target::Vsp, 0x0100_0001);
        r.lab.recover();
        assert!(r.regs.writes().is_empty(), "{:?}", r.regs.writes());
        assert_eq!(r.regs.get(Target::Vsp), 0x0100_0001);
    }

    #[test]
    fn a_write_that_does_not_read_back_puts_the_control_back_and_says_so() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("sharpness", "strong").unwrap();
        // the firmware takes the VSP back at once, as a register it owns would be
        r.regs.stick(Target::Vsp);
        let e = r.lab.set("dimming", "dimmed").unwrap_err();
        assert_eq!(e.code(), "verify_failed", "{e:?}");
        assert!(
            !r.lab.open(),
            "a failed write ends the session and puts everything back"
        );
        r.regs.unstick(Target::Vsp);
        assert!(at_rest(&r), "the sharpener too: {:?}", r.regs.dump());
        assert_eq!(r.lab.status()["last"]["why"], "failed");
    }

    #[test]
    fn a_write_that_fails_puts_back_what_was_written() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("window", "on_a").unwrap();
        r.regs.fail_writes(Target::HoleY);
        let e = r.lab.set("window", "on_b").unwrap_err();
        assert_eq!(e.code(), "hardware", "{e:?}");
        r.regs.fail_writes_off();
        // the restore could not write HoleY either while it failed: it is pending, and the next try puts it back
        r.advance(RETRY_MS);
        r.lab.tick();
        assert!(at_rest(&r), "{:?}", r.regs.dump());
        assert!(!r.journal_exists());
    }

    #[test]
    fn a_restore_that_cannot_complete_is_tried_again_and_blocks_a_new_session() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("dimming", "grey").unwrap();
        r.regs.fail_writes(Target::Vsp);
        let e = r.lab.end().unwrap_err();
        assert_eq!(e.code(), "stuck", "{e:?}");
        assert!(
            r.journal_exists(),
            "the journal stays until everything reads its original"
        );
        assert_eq!(r.regs.get(Target::Vsp), 0x0200_0003);
        assert!(
            r.lab.status()["pending"]
                .as_str()
                .unwrap()
                .contains("osd0_vsp")
        );
        assert_eq!(
            r.lab.begin().unwrap_err().code(),
            "stuck",
            "no new session while something is not back"
        );
        r.advance(RETRY_MS);
        r.lab.tick();
        assert_eq!(
            r.regs.get(Target::Vsp),
            0x0200_0003,
            "still failing: tried again, not given up"
        );
        r.regs.fail_writes_off();
        r.advance(RETRY_MS);
        r.lab.tick();
        assert!(at_rest(&r) && !r.journal_exists());
        r.lab.begin().unwrap();
    }

    #[test]
    fn a_journal_that_cannot_be_written_stops_the_write_before_the_hardware() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        // a file where the journal's directory should be
        std::fs::write(r.dir.path().join("displaylab"), b"in the way").unwrap();
        let e = r.lab.set("dimming", "dimmed").unwrap_err();
        assert_eq!(e.code(), "journal", "{e:?}");
        assert!(r.regs.writes().is_empty(), "no journal, no write");
        assert!(at_rest(&r));
    }

    #[test]
    fn an_original_the_set_could_not_hold_makes_its_control_unusable_and_never_a_guess() {
        // a hole register that is not clear (another writer), and a VSP that already holds the lab's value (a crashed run's)
        let regs = FakeRegs::the_set();
        regs.force(Target::HoleX, 0x0000_0005);
        regs.force(Target::Vsp, 0x0100_0001);
        let r = Rig::with_regs(regs);
        let v = r.lab.begin().unwrap();
        assert_eq!(
            v["session"]["usable"],
            json!({"sharpness": true, "dimming": false, "window": false})
        );
        assert!(
            v["session"]["why"]["window"]
                .as_str()
                .unwrap()
                .contains("hole0_wx"),
            "{v}"
        );
        assert!(
            v["session"]["why"]["dimming"]
                .as_str()
                .unwrap()
                .contains("osd0_vsp"),
            "{v}"
        );
        assert_eq!(
            r.lab.set("window", "on_a").unwrap_err().code(),
            "not_original"
        );
        assert_eq!(
            r.lab.set("dimming", "grey").unwrap_err().code(),
            "not_original"
        );
        assert!(r.regs.writes().is_empty());
        r.lab.set("sharpness", "strong").unwrap();
        r.lab.end().unwrap();
        assert_eq!(r.regs.get(Target::Gain), 0x10);
        assert_eq!(r.regs.get(Target::HoleX), 5, "never touched");
        // with nothing usable there is no session at all
        let regs = FakeRegs::the_set();
        for t in Target::ALL {
            regs.fail_reads(t);
        }
        let r = Rig::with_regs(regs);
        assert_eq!(r.lab.begin().unwrap_err().code(), "unavailable");
    }

    #[test]
    fn a_set_that_is_not_the_set_is_not_touched() {
        let r = Rig::with_regs(FakeRegs::not_the_set());
        assert_eq!(r.lab.begin().unwrap_err().code(), "unavailable");
        assert!(r.regs.reads() == 0, "not even a read");
        let v = r.lab.status();
        assert_eq!(v["available"], false);
        assert!(v["reason"].as_str().unwrap().contains("not an LG set"));
        assert_eq!(
            r.lab.set("dimming", "dimmed").unwrap_err().code(),
            "no_session"
        );
        r.lab.recover();
        r.lab.shutdown();
        r.lab.tick();
        assert!(r.regs.writes().is_empty() && r.regs.reads() == 0);
    }

    #[test]
    fn a_register_the_firmware_changes_is_reported_as_not_held_and_not_fought_over() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("dimming", "dimmed").unwrap();
        r.regs.force(Target::Vsp, 0x0200_0001);
        let before = r.regs.writes().len();
        let v = r.lab.ping().unwrap();
        assert_eq!(v["session"]["held"], false);
        assert_eq!(v["session"]["drifted"], json!(["osd0_vsp"]));
        assert_eq!(
            r.regs.writes().len(),
            before,
            "a ping reads and never writes"
        );
        // the end still puts the original back
        r.lab.end().unwrap();
        assert!(at_rest(&r));
    }

    #[test]
    fn a_status_without_a_session_touches_no_lease_and_no_register() {
        let r = Rig::new();
        let v = r.lab.status();
        assert_eq!(v["available"], true);
        assert_eq!(v["session"], Value::Null);
        assert_eq!(v["controls"]["window"], json!(["off", "on_a", "on_b"]));
        assert_eq!(v["deadmanMs"], DEADMAN_MS);
        assert_eq!(r.regs.reads(), 0);
    }

    #[test]
    fn two_runs_one_after_the_other_each_start_from_the_sets_own_state() {
        let r = Rig::new();
        r.lab.begin().unwrap();
        r.lab.set("sharpness", "soft").unwrap();
        r.lab.end().unwrap();
        let v = r.lab.begin().unwrap();
        assert_eq!(v["session"]["id"], 2);
        assert_eq!(shown(&v, "sharpness"), "normal");
        r.lab.set("sharpness", "strong").unwrap();
        assert_eq!(r.regs.get(Target::Gain), 0x80);
        r.lab.end().unwrap();
        assert!(at_rest(&r));
    }

    #[test]
    fn a_journal_waits_for_a_start_that_can_reach_the_hardware() {
        // the run that wrote it died; this start is on a machine that is not the set (or has no access): nothing is read or written, and the
        // journal stays for the start that can
        let r = Rig::with_regs(FakeRegs::not_the_set());
        std::fs::create_dir_all(r.dir.path().join("displaylab")).unwrap();
        let doc = json!({"v": 1, "session": 4, "started_ms": 1, "pid": 1, "targets": {"osd0_vsp": {"orig": 0, "written": 0x0100_0001}}});
        std::fs::write(
            r.dir.path().join("displaylab/journal.json"),
            doc.to_string(),
        )
        .unwrap();
        r.lab.recover();
        assert!(r.regs.writes().is_empty() && r.regs.reads() == 0);
        assert!(r.journal_exists());
        assert_eq!(r.lab.status()["available"], false);
        // the same journal on the set is put back
        let on_the_set = Rig::new();
        std::fs::create_dir_all(on_the_set.dir.path().join("displaylab")).unwrap();
        std::fs::write(
            on_the_set.dir.path().join("displaylab/journal.json"),
            doc.to_string(),
        )
        .unwrap();
        on_the_set.regs.force(Target::Vsp, 0x0100_0001);
        on_the_set.lab.recover();
        assert_eq!(on_the_set.regs.get(Target::Vsp), 0);
        assert!(!on_the_set.journal_exists());
    }
}
