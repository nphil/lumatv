//! The record `ssgw-accept` keeps each time it gives a port 3001 connection to LG's gateway: why, and how the gateway stood.
//!
//! `ssgw-accept` runs once per connection with the client on its standard streams, so it cannot log; the record is a file the helper
//! reads and puts in its log and on the plugin's page. A gateway that is down while Home Assistant retries would otherwise write one
//! per attempt, so connections that fall back for the same reason within [`QUIET`] of the last one share an episode: one file
//! (`<first time>-<n>.txt`) with the first and last time and the count. The newest [`KEEP`] episodes are kept.

use crate::util::utc_stamp;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

/// Where the records are on the TV: beside the gateway's state and crash records, in the data directory the plugin lays out.
pub const DIR: &str = "/var/lib/lumatv/ssgw/fallback";
/// Episodes kept.
const KEEP: usize = 16;
/// Silence that ends an episode, in seconds.
const QUIET: i64 = 300;
/// The most a connection waits for another `ssgw-accept` to finish its record.
const LOCK_WAIT: Duration = Duration::from_millis(200);

/// Why a connection went to LG's gateway: a stable code, and the words that say what was seen.
pub struct Why {
    pub code: &'static str,
    pub detail: String,
}

/// The gateway's run directory and sockets as `ssgw-accept` finds them.
pub fn gateway_state(run: &Path) -> String {
    let present = |name: &str| if std::fs::symlink_metadata(run.join(name)).is_ok() { "present" } else { "missing" };
    let dir = match std::fs::symlink_metadata(run) {
        Ok(m) if m.is_dir() => format!("run directory mode {:04o} uid {}", m.mode() & 0o7777, m.uid()),
        Ok(_) => "run path is not a directory".into(),
        Err(e) => format!("run directory {e}"),
    };
    format!("{dir}; accept.sock {}; control.sock {}", present("accept.sock"), present("control.sock"))
}

/// Records one fallback in `dir` at `now` (Unix seconds); every failure is dropped, as the connection has LG's gateway to go to.
/// `args` are xinetd's arguments, `state` is [`gateway_state`], `took` how long the hand-over was tried and `boot` the boot id.
pub fn record(dir: &Path, why: &Why, args: &str, state: &str, took: Duration, boot: &str, now: i64) {
    let _ = keep(dir, why, args, state, took, boot, now);
}

fn keep(dir: &Path, why: &Why, args: &str, state: &str, took: Duration, boot: &str, now: i64) -> std::io::Result<()> {
    match DirBuilder::new().mode(0o700).create(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    // only a directory of ours that others cannot write into is trusted: this runs as root
    let meta = std::fs::symlink_metadata(dir)?;
    // SAFETY: `geteuid` has no preconditions.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let lock = OpenOptions::new().create(true).append(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(dir.join("lock"))?;
    lock_for(&lock)?;

    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".txt"))
        .collect();
    names.sort();
    let (mut name, mut first, mut count) = (None, now, 0u64);
    if let Some(last) = names.last()
        && let Ok(text) = std::fs::read_to_string(dir.join(last))
    {
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(": "));
        let at = field("last-unix").and_then(|v| v.parse::<i64>().ok());
        if field("code") == Some(why.code)
            && at.is_some_and(|at| (0..=QUIET).contains(&(now - at)))
            && let (Some(f), Some(c)) = (field("first-unix").and_then(|v| v.parse().ok()), field("count").and_then(|v| v.parse::<u64>().ok()))
        {
            (name, first, count) = (Some(last.clone()), f, c);
        }
    }
    let count = count + 1;
    let name = name.unwrap_or_else(|| {
        let stamp = utc_stamp(now).replace(['-', ':'], "").replace('T', "-").replace('Z', "");
        // the number keeps two episodes begun in one second apart, and in order
        (1..)
            .map(|n| format!("{stamp}-{n:03}.txt"))
            .find(|name| !names.contains(name))
            .unwrap_or_default()
    });
    let (first_at, last_at) = (utc_stamp(first), utc_stamp(now));
    let text = format!(
        "code: {}\ndetail: {}\nfirst: {first_at}\nlast: {last_at}\nfirst-unix: {first}\nlast-unix: {now}\ncount: {count}\nboot: {boot}\n\
         connection: {args}\ngateway: {state}; hand-over tried for {} ms\nsummary: {} UTC to {} UTC: {count} connection{} to LG's gateway, {}\n",
        why.code,
        why.detail,
        took.as_millis(),
        first_at[..16].replace('T', " "),
        last_at[..16].replace('T', " "),
        if count == 1 { "" } else { "s" },
        why.detail,
    );
    let temp = dir.join("write.tmp");
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&temp)?;
    file.write_all(text.as_bytes())?;
    // `/var/lib/lumatv` is persistent across power cuts only after the data and rename are synced.
    file.sync_all()?;
    std::fs::rename(&temp, dir.join(&name))?;
    File::open(dir)?.sync_all()?;
    for old in names.iter().filter(|n| **n != name).rev().skip(KEEP - 1) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

/// An exclusive lock on `file`, held until it closes.
fn lock_for(file: &File) -> std::io::Result<()> {
    let start = Instant::now();
    loop {
        // SAFETY: the descriptor is open for as long as `file` is.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::WouldBlock || start.elapsed() >= LOCK_WAIT {
            return Err(e);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn why(code: &'static str) -> Why {
        Why { code, detail: format!("{code} detail") }
    }

    fn kept(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with(".txt")).collect();
        names.sort();
        names
    }

    #[test]
    fn one_reason_in_a_row_is_one_episode_with_its_first_and_last_time_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("fallback");
        let t0 = 1_795_000_000; // 2026-11-18T11:06:40Z
        for at in [t0, t0 + 10, t0 + 10 + QUIET] {
            record(&dir, &why("not-running"), "3001", "state", Duration::from_millis(3), "boot-1", at);
        }
        assert_eq!(kept(&dir).len(), 1);
        let text = std::fs::read_to_string(dir.join(&kept(&dir)[0])).unwrap();
        assert!(text.contains("first: 2026-11-18T11:06:40Z\nlast: 2026-11-18T11:11:50Z\n"), "{text}");
        assert!(text.contains("count: 3\n") && text.contains("boot: boot-1\n"), "{text}");

        // another reason, or the same one after a quiet spell, is a new episode
        record(&dir, &why("refused"), "3001", "state", Duration::ZERO, "boot-1", t0 + 400 + QUIET);
        record(&dir, &why("refused"), "3001", "state", Duration::ZERO, "boot-1", t0 + 400 + 2 * QUIET + 1);
        assert_eq!(kept(&dir).len(), 3);
    }

    #[test]
    fn only_the_newest_episodes_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("fallback");
        for n in 0..(KEEP as i64 + 4) {
            record(&dir, &why("not-running"), "3001", "state", Duration::ZERO, "b", 1_795_000_000 + n * (QUIET + 1));
        }
        let names = kept(&dir);
        assert_eq!(names.len(), KEEP);
        assert!(names[0].as_str() > "20261118-110640-001.txt", "the oldest went first: {names:?}");
    }

    #[test]
    fn a_directory_others_can_write_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let open = dir.path().join("fallback");
        DirBuilder::new().mode(0o777).create(&open).unwrap();
        std::fs::set_permissions(&open, std::os::unix::fs::PermissionsExt::from_mode(0o777)).unwrap();
        record(&open, &why("not-running"), "3001", "state", Duration::ZERO, "b", 1_795_000_000);
        assert!(kept(&open).is_empty());
    }
}
