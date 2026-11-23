//! Everything the route does to the TV, behind one trait: [`System`] is the real thing, `testing::FakeHost` a scripted one.
//!
//! Rules that hold for every method (WEDGE.md of the TrueHD study): nothing here reads `/proc/aaudio/instances/*/prop`, uses
//! `amixer` to read, or opens the sound card's control device; a child process that does not answer is abandoned, never waited on,
//! because a task stuck in the sound card's control lock cannot be killed.

use crate::sndout::Words;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How long `amixer` and `luna-send` get before they are abandoned.
const CHILD_TIMEOUT: Duration = Duration::from_secs(5);

/// A crashed child must not leave a core dump on the TV: SAM starts processes with an unlimited core size and a dump is 160-290 MB.
pub fn no_core_dumps() -> io::Result<()> {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `setrlimit` only reads the struct it is given.
    match unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

/// A child that sets its own core size to 0 before it runs, whatever limit the helper was started with.
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    // SAFETY: `no_core_dumps` only calls `setrlimit`, which is async-signal-safe.
    unsafe { command.pre_exec(no_core_dumps) };
    command
}

/// A task in uninterruptible sleep (state `D`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stuck {
    pub comm: String,
    pub wchan: String,
    pub stack: String,
}

/// Why an eARC words write did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    Failed(String),
    /// `amixer` did not answer in time and was abandoned.
    Hung,
}

pub trait Host: Send + Sync {
    fn read(&self, path: &str) -> io::Result<String>;
    fn is_dir(&self, path: &str) -> bool;
    /// One write to a control file such as a converter's `cmd`.
    fn write(&self, path: &str, text: &str) -> io::Result<()>;
    /// Replaces a file atomically and durably (mode 0600), creating its directory.
    fn save(&self, path: &str, text: &str) -> io::Result<()>;
    fn remove(&self, path: &str) -> io::Result<()>;
    fn sha256(&self, path: &str) -> io::Result<String>;
    /// Where a process's open descriptors point.
    fn fd_targets(&self, pid: u32) -> io::Result<Vec<String>>;
    /// A process's start time (field 22 of its `stat`), `None` when it is not running.
    fn start_time(&self, pid: u32) -> Option<u64>;
    /// Sets `Sndout EARC Outputtype`, the one control LG's route is changed through.
    fn set_earc_words(&self, words: Words) -> Result<(), WriteError>;
    /// One luna call's reply text, `None` when it did not answer or failed to run.
    fn luna(&self, uri: &str, payload: &str) -> Option<String>;
    /// Tasks stuck in uninterruptible sleep.
    fn stuck_tasks(&self) -> Vec<Stuck>;
    /// Milliseconds since boot, suspend included.
    fn now_ms(&self) -> u64;
    fn sleep_ms(&self, ms: u64);
}

/// The real TV.
#[derive(Clone, Debug)]
pub struct System {
    pub amixer: String,
    pub luna_send: String,
    /// How long `amixer` and `luna-send` get before they are abandoned.
    pub timeout: Duration,
}

impl Default for System {
    fn default() -> Self {
        Self {
            amixer: "amixer".into(),
            luna_send: "luna-send".into(),
            timeout: CHILD_TIMEOUT,
        }
    }
}

/// Waits for a child up to `timeout`; on timeout kills it and leaves reaping to a thread that may block for good.
fn wait_or_abandon(mut child: Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let end = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                thread::spawn(move || {
                    let _ = child.wait();
                });
                return None;
            }
        }
    }
}

/// `comm` and state of a task from its `stat` line. `comm` may hold spaces and parentheses, so it ends at the last `)`.
fn stat_head(stat: &str) -> Option<(&str, char)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let state = stat.get(close + 1..)?.trim_start().chars().next()?;
    Some((stat.get(open + 1..close)?, state))
}

impl Host for System {
    fn read(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn is_dir(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }

    fn write(&self, path: &str, text: &str) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .open(path)?
            .write_all(text.as_bytes())
    }

    fn save(&self, path: &str, text: &str) -> io::Result<()> {
        let path = Path::new(path);
        let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
        if let Some(dir) = dir {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Some(dir) = dir {
            File::open(dir)?.sync_all()?;
        }
        Ok(())
    }

    fn remove(&self, path: &str) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => {
                if let Some(dir) = Path::new(path)
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                {
                    if let Ok(dir) = File::open(dir) {
                        let _ = dir.sync_all();
                    }
                }
                Ok(())
            }
        }
    }

    fn sha256(&self, path: &str) -> io::Result<String> {
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                return Ok(format!("{:x}", hash.finalize()));
            }
            hash.update(&buf[..n]);
        }
    }

    fn fd_targets(&self, pid: u32) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(format!("/proc/{pid}/fd"))? {
            if let Ok(target) = fs::read_link(entry?.path()) {
                out.push(target.to_string_lossy().into_owned());
            }
        }
        Ok(out)
    }

    fn start_time(&self, pid: u32) -> Option<u64> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Fields after `comm`: state is field 3, starttime field 22.
        let after = stat.get(stat.rfind(')')? + 1..)?;
        after.split_whitespace().nth(19)?.parse().ok()
    }

    fn set_earc_words(&self, words: Words) -> Result<(), WriteError> {
        let child = command(&self.amixer)
            .args([
                "-c",
                "0",
                "cset",
                "name=Sndout EARC Outputtype",
                &words.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| WriteError::Failed(format!("amixer did not start: {e}")))?;
        match wait_or_abandon(child, self.timeout) {
            Some(status) if status.success() => Ok(()),
            Some(status) => Err(WriteError::Failed(format!("amixer exited with {status}"))),
            None => Err(WriteError::Hung),
        }
    }

    fn luna(&self, uri: &str, payload: &str) -> Option<String> {
        let mut child = command(&self.luna_send)
            .args(["-n", "1", uri, payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let reader = thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            text
        });
        wait_or_abandon(child, self.timeout)?;
        reader.join().ok()
    }

    fn stuck_tasks(&self) -> Vec<Stuck> {
        let mut out = Vec::new();
        let Ok(procs) = fs::read_dir("/proc") else {
            return out;
        };
        for process in procs.flatten() {
            let name = process.file_name();
            if !name.as_encoded_bytes().iter().all(u8::is_ascii_digit) {
                continue;
            }
            let Ok(tasks) = fs::read_dir(process.path().join("task")) else {
                continue;
            };
            for task in tasks.flatten() {
                let dir = task.path();
                let Ok(stat) = fs::read_to_string(dir.join("stat")) else {
                    continue;
                };
                let Some((comm, 'D')) = stat_head(&stat) else {
                    continue;
                };
                let text = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
                out.push(Stuck {
                    comm: comm.to_owned(),
                    wchan: text("wchan"),
                    stack: text("stack"),
                });
            }
        }
        out
    }

    fn now_ms(&self) -> u64 {
        let mut now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `now` is a valid timespec the call fills in.
        unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) };
        now.tv_sec as u64 * 1000 + now.tv_nsec as u64 / 1_000_000
    }

    fn sleep_ms(&self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_head_survives_odd_names() {
        assert_eq!(
            stat_head("12 (audiooutputd) D 1 2 3"),
            Some(("audiooutputd", 'D'))
        );
        assert_eq!(stat_head("12 (a (b) c) S 1"), Some(("a (b) c", 'S')));
        assert_eq!(stat_head("garbage"), None);
    }

    #[test]
    fn own_process_start_time_and_descriptors_read_back() {
        let system = System::default();
        let pid = std::process::id();
        assert!(system.start_time(pid).is_some_and(|t| t > 0));
        assert!(system.start_time(u32::MAX).is_none());
        assert!(
            system
                .fd_targets(pid)
                .unwrap()
                .iter()
                .any(|t| t.starts_with('/') || t.contains(':'))
        );
    }

    #[test]
    fn save_replaces_atomically_with_owner_only_mode_and_remove_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("audiopass-host-{}", std::process::id()));
        let path = dir.join("nested").join("journal");
        let path = path.to_str().unwrap();
        let system = System::default();
        system.save(path, "one").unwrap();
        system.save(path, "two").unwrap();
        assert_eq!(system.read(path).unwrap(), "two");
        let mode =
            std::os::unix::fs::PermissionsExt::mode(&fs::metadata(path).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600);
        system.remove(path).unwrap();
        system.remove(path).unwrap();
        assert!(system.read(path).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sha256_matches_a_known_digest() {
        let path = std::env::temp_dir().join(format!("audiopass-sha-{}", std::process::id()));
        fs::write(&path, "abc").unwrap();
        assert_eq!(
            System::default().sha256(path.to_str().unwrap()).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_programs_fail_instead_of_hanging() {
        let system = System {
            amixer: "/nonexistent/amixer".into(),
            luna_send: "/nonexistent/luna-send".into(),
            ..System::default()
        };
        let words = Words {
            set_type: 2,
            codec: 1,
            channels: 0,
            rate: 0,
            mix: 0,
        };
        assert!(matches!(
            system.set_earc_words(words),
            Err(WriteError::Failed(_))
        ));
        assert_eq!(system.luna("luna://x/y", "{}"), None);
    }

    #[test]
    fn a_child_that_does_not_answer_is_abandoned() {
        // A shell script stands in for amixer stuck in the control lock.
        let dir = std::env::temp_dir().join(format!("audiopass-hang-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("hang");
        fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let program = script.to_str().unwrap().to_owned();
        let system = System {
            amixer: program.clone(),
            luna_send: program,
            timeout: Duration::from_millis(150),
        };
        let words = Words {
            set_type: 2,
            codec: 1,
            channels: 0,
            rate: 0,
            mix: 0,
        };
        let started = Instant::now();
        assert_eq!(system.set_earc_words(words), Err(WriteError::Hung));
        assert_eq!(system.luna("luna://x/y", "{}"), None);
        assert!(started.elapsed() < Duration::from_secs(2));
        let _ = fs::remove_dir_all(dir);
    }
}
