//! What the data directory (`/var/lib/lumatv`, on the TV's 1 GB overlay) must not keep: files nothing writes any more and rollback
//! copies nobody removes. Run once at each start, before anything else of the helper's writes there.
//!
//! - Logs of earlier builds, which wrote them under the data directory before logs went to the collector in RAM: a file of the
//!   [`RETIRED`] names that has not been written for [`RETIRED_AFTER`] goes. A file that is still written never goes.
//! - Rollback copies (`*.bak`) lying directly in the data directory: the newest [`KEEP_BACKUPS`] stay, and only while they are younger
//!   than [`BACKUP_MAX_AGE`]. They are copies of the helper's or the tool's own binaries, 10 MB for the helper's.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Log files earlier builds wrote under the data directory, relative to it.
const RETIRED: &[&str] = &[
    "compositor/compositor.log",
    "compositor/shell-rust.log",
    "compositor/shell-rust.log.old",
    "compositor/shell.log",
    "compositor/sm.log",
    "log-helper",
    "capture.log",
];
/// How long a retired file must have been left alone to count as retired.
const RETIRED_AFTER: Duration = Duration::from_secs(7 * 86_400);
const KEEP_BACKUPS: usize = 2;
const BACKUP_MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

/// What a sweep took off the disk.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub files: usize,
    pub bytes: u64,
}

fn age(path: &Path, now: SystemTime) -> Option<(u64, Duration)> {
    let meta = fs::symlink_metadata(path).ok().filter(|m| m.is_file())?;
    Some((meta.len(), now.duration_since(meta.modified().ok()?).unwrap_or_default()))
}

fn remove(path: &Path, size: u64, swept: &mut Swept) {
    if fs::remove_file(path).is_ok() {
        swept.files += 1;
        swept.bytes += size;
    }
}

/// Removes what is described above from `data_dir`, judging ages against `now`.
pub fn sweep(data_dir: &Path, now: SystemTime) -> Swept {
    let mut swept = Swept::default();
    for name in RETIRED {
        let path = data_dir.join(name);
        if let Some((size, idle)) = age(&path, now)
            && idle >= RETIRED_AFTER
        {
            remove(&path, size, &mut swept);
        }
    }
    let mut backups: Vec<(Duration, u64, PathBuf)> = fs::read_dir(data_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bak"))
        .filter_map(|p| age(&p, now).map(|(size, idle)| (idle, size, p)))
        .collect();
    backups.sort();
    for (n, (idle, size, path)) in backups.iter().enumerate() {
        if n >= KEEP_BACKUPS || *idle > BACKUP_MAX_AGE {
            remove(path, *size, &mut swept);
        }
    }
    swept
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    const DAY: Duration = Duration::from_secs(86_400);

    fn put(dir: &Path, name: &str, bytes: usize, idle: Duration, now: SystemTime) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; bytes]).unwrap();
        File::options().write(true).open(&path).unwrap().set_modified(now - idle).unwrap();
    }

    fn present(dir: &Path, name: &str) -> bool {
        dir.join(name).exists()
    }

    #[test]
    fn a_retired_log_goes_once_it_has_been_left_alone_and_a_written_one_stays() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        put(dir.path(), "compositor/compositor.log", 100, 20 * DAY, now);
        put(dir.path(), "compositor/sm.log", 10, 6 * DAY, now);
        put(dir.path(), "log-helper", 5, 8 * DAY, now);
        put(dir.path(), "log", 7, 20 * DAY, now);
        assert_eq!(sweep(dir.path(), now), Swept { files: 2, bytes: 105 });
        assert!(!present(dir.path(), "compositor/compositor.log") && !present(dir.path(), "log-helper"));
        assert!(present(dir.path(), "compositor/sm.log"), "written within the week");
        assert!(present(dir.path(), "log"), "not a retired name: the shell tools' own log");
    }

    #[test]
    fn rollback_copies_keep_the_newest_two_and_nothing_older_than_a_week() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        put(dir.path(), "a.bak", 1, DAY, now);
        put(dir.path(), "b.bak", 2, 2 * DAY, now);
        put(dir.path(), "c.bak", 4, 3 * DAY, now);
        put(dir.path(), "old.bak", 8, 30 * DAY, now);
        put(dir.path(), "profiles.json", 16, 30 * DAY, now);
        assert_eq!(sweep(dir.path(), now), Swept { files: 2, bytes: 12 });
        assert!(present(dir.path(), "a.bak") && present(dir.path(), "b.bak"));
        assert!(!present(dir.path(), "c.bak") && !present(dir.path(), "old.bak"));
        assert!(present(dir.path(), "profiles.json"), "only rollback copies are judged");
    }

    #[test]
    fn two_old_copies_are_both_gone_and_a_missing_directory_is_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        put(dir.path(), "lumatv-helper.46895c9c.bak", 9, 17 * DAY, now);
        put(dir.path(), "lumatv.tool.03ddc6a0.bak", 3, 17 * DAY, now);
        assert_eq!(sweep(dir.path(), now), Swept { files: 2, bytes: 12 });
        assert_eq!(sweep(&dir.path().join("nowhere"), now), Swept::default());
    }
}
