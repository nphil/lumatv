//! The captures folder (`/var/lib/lumatv/captures`): file names, what the folder holds, the size cap and the free-space floor.
//!
//! Only files this feature makes are its own (`shot-*.png`, `rec-*.mp4` and their `.part` while being written); nothing else in the
//! folder is listed, counted or removed. The cap keeps the newest: making room for a new capture deletes the oldest of ours first, and
//! never the file being written.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shot,
    Recording,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Shot => "shot",
            Kind::Recording => "rec",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Kind::Shot => "png",
            Kind::Recording => "mp4",
        }
    }
}

/// One finished capture in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: u64,
    pub modified: SystemTime,
}

#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
    max_bytes: u64,
}

/// `YYYYMMDD-HHMMSS` in the TV's local time, or `None` while the clock has not been set (before 2020: a TV that has not reached the network
/// yet), when the name carries the time since boot instead.
fn stamp(now: SystemTime) -> Option<String> {
    let secs = now.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() as libc::time_t;
    // SAFETY: localtime_r writes a `tm` of its own on the stack from a plain integer.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return None;
        }
        tm
    };
    if tm.tm_year < 120 {
        return None;
    }
    Some(format!("{:04}{:02}{:02}-{:02}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec))
}

fn since_boot() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a stack timespec for clock_gettime.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64
}

/// Whether `name` is one of ours, finished (`part` false) or still being written.
fn ours(name: &str, part: bool) -> bool {
    let name = if part { name.strip_suffix(".part") } else { Some(name) };
    name.is_some_and(|n| {
        [Kind::Shot, Kind::Recording].iter().any(|k| n.starts_with(&format!("{}-", k.prefix())) && n.ends_with(&format!(".{}", k.extension())))
    })
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> Store {
        Store { dir: dir.into(), max_bytes }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn ensure_dir(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)
    }

    /// A name for a new capture that nothing in the folder has: the local time, then `-2`, `-3` when that second is taken.
    pub fn new_name(&self, kind: Kind, now: SystemTime) -> String {
        let base = match stamp(now) {
            Some(s) => format!("{}-{s}", kind.prefix()),
            None => format!("{}-boot{:06}", kind.prefix(), since_boot()),
        };
        let taken = |n: &str| self.dir.join(n).exists() || self.dir.join(format!("{n}.part")).exists();
        let first = format!("{base}.{}", kind.extension());
        if !taken(&first) {
            return first;
        }
        (2..).map(|i| format!("{base}-{i}.{}", kind.extension())).find(|n| !taken(n)).expect("an unused name")
    }

    /// A name the folder accepts: ours, plain, no path in it.
    pub fn is_name(name: &str) -> bool {
        !name.contains('/') && !name.starts_with('.') && ours(name, false)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The finished captures, newest first.
    pub fn list(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                if !ours(&name, false) {
                    return None;
                }
                let meta = e.metadata().ok()?;
                meta.is_file().then(|| Entry { name, bytes: meta.len(), modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH) })
            })
            .collect();
        v.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.name.cmp(&a.name)));
        v
    }

    /// What the folder's captures take, with the ones still being written.
    pub fn used_bytes(&self) -> u64 {
        std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                (ours(&name, false) || ours(&name, true)).then(|| e.metadata().ok().map_or(0, |m| m.len()))
            })
            .sum()
    }

    /// Deletes the oldest finished captures until `incoming` more bytes fit under the cap; returns the names removed. A capture bigger than
    /// the cap on its own empties the folder of ours and still does not fit: the caller's own size limit is what stops that one.
    pub fn make_room(&self, incoming: u64) -> Vec<String> {
        let mut removed = Vec::new();
        let mut used = self.used_bytes();
        let mut oldest_first = self.list();
        oldest_first.reverse();
        for e in oldest_first {
            if used + incoming <= self.max_bytes {
                break;
            }
            if std::fs::remove_file(self.dir.join(&e.name)).is_ok() {
                used = used.saturating_sub(e.bytes);
                removed.push(e.name);
            }
        }
        removed
    }

    /// Deletes every capture of ours (and unfinished remains of them): Settings' "delete all". Returns how many files went.
    pub fn clear(&self) -> usize {
        let mut n = 0;
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            if (ours(&name, false) || ours(&name, true)) && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
        n
    }
}

/// Bytes free for an ordinary writer on the filesystem holding `path` (`u64::MAX` when it cannot be asked: no floor is enforced blind).
pub fn free_bytes(path: &Path) -> u64 {
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return u64::MAX };
    // SAFETY: statvfs fills a struct of its own from a NUL-terminated path.
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut s) != 0 {
            return u64::MAX;
        }
        (s.f_bavail as u64).saturating_mul(s.f_frsize as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lumatv-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, name: &str, bytes: usize, age_secs: u64) {
        let p = dir.join(name);
        std::fs::write(&p, vec![7u8; bytes]).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn names_carry_the_local_time_and_never_collide_with_a_file_or_a_part() {
        let dir = tmp("names");
        let store = Store::new(&dir, 1 << 30);
        // 2026-10-28 07:00:00 UTC; the name's clock is the TV's local one, so only its shape and its uniqueness are fixed here.
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_170_800);
        let a = store.new_name(Kind::Shot, t);
        assert!(a.starts_with("shot-2026") && a.ends_with(".png") && a.len() == "shot-20261028-070000.png".len(), "{a}");
        std::fs::write(dir.join(&a), b"x").unwrap();
        let b = store.new_name(Kind::Shot, t);
        assert_eq!(b, a.replace(".png", "-2.png"));
        std::fs::write(dir.join(format!("{b}.part")), b"x").unwrap();
        assert_eq!(store.new_name(Kind::Shot, t), a.replace(".png", "-3.png"), "an unfinished file keeps its name too");
        let r = store.new_name(Kind::Recording, t);
        assert!(r.starts_with("rec-2026") && r.ends_with(".mp4"), "{r}");
        // A clock that was never set (the epoch) names by the time since boot.
        let c = store.new_name(Kind::Recording, SystemTime::UNIX_EPOCH);
        assert!(c.starts_with("rec-boot") && c.ends_with(".mp4"), "{c}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_our_names_are_names() {
        for good in ["shot-20261028-070000.png", "rec-20261028-070000-2.mp4", "shot-boot000012.png"] {
            assert!(Store::is_name(good), "{good}");
        }
        for bad in ["../shot-1.png", "shot-1.png/x", ".shot-1.png", "notes.txt", "shot-1.mp4", "rec-1.png", "shot-1.png.part", ""] {
            assert!(!Store::is_name(bad), "{bad}");
        }
    }

    #[test]
    fn making_room_removes_the_oldest_of_ours_and_nothing_else() {
        let dir = tmp("room");
        put(&dir, "shot-20260101-000001.png", 400, 400);
        put(&dir, "rec-20260101-000002.mp4", 400, 300);
        put(&dir, "shot-20260101-000003.png", 400, 200);
        put(&dir, "notes.txt", 5000, 900);
        let store = Store::new(&dir, 1500);
        assert_eq!(store.used_bytes(), 1200, "notes.txt is not ours and is not counted");
        assert!(store.make_room(300).is_empty(), "1200 + 300 fits under 1500");
        let removed = store.make_room(500);
        assert_eq!(removed, vec!["shot-20260101-000001.png".to_string()], "one oldest file frees enough: 800 + 500 fits");
        let removed = store.make_room(1200);
        assert_eq!(removed, vec!["rec-20260101-000002.mp4".to_string(), "shot-20260101-000003.png".to_string()]);
        assert!(dir.join("notes.txt").exists());
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unfinished_file_counts_against_the_cap_and_is_never_the_one_removed() {
        let dir = tmp("part");
        put(&dir, "rec-20260101-000009.mp4.part", 900, 5);
        put(&dir, "shot-20260101-000001.png", 300, 500);
        let store = Store::new(&dir, 1000);
        assert_eq!(store.used_bytes(), 1200);
        assert_eq!(store.make_room(0), vec!["shot-20260101-000001.png".to_string()]);
        assert!(dir.join("rec-20260101-000009.mp4.part").exists(), "the file being written stays");
        assert!(store.list().is_empty(), "and is not a finished capture");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_is_newest_first_and_clear_takes_ours_with_the_unfinished_ones() {
        let dir = tmp("list");
        put(&dir, "shot-20260101-000001.png", 10, 300);
        put(&dir, "shot-20260101-000002.png", 10, 100);
        put(&dir, "rec-20260101-000003.mp4.part", 10, 10);
        put(&dir, "keep.me", 10, 10);
        let store = Store::new(&dir, 1 << 20);
        let names: Vec<_> = store.list().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["shot-20260101-000002.png", "shot-20260101-000001.png"]);
        assert_eq!(store.clear(), 3);
        assert!(dir.join("keep.me").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_space_is_read_and_an_unreadable_path_enforces_no_floor() {
        assert!(free_bytes(&std::env::temp_dir()) > 0);
        assert_eq!(free_bytes(Path::new("/definitely/not/a/path")), u64::MAX);
    }
}
