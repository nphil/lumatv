//! Everything the route does to the TV, behind one trait: [`System`] is the real thing, `testing::FakeHost` a scripted one.
//!
//! Rules that hold for every method (WEDGE.md of the TrueHD study): nothing here reads `/proc/aaudio/instances/*/prop`, uses
//! `amixer` to read, or opens the sound card's control device; a child process that does not answer is abandoned, never waited on,
//! because a task stuck in the sound card's control lock cannot be killed. A read or write of a driver's file is the same kind of
//! call and gets the same treatment: it runs on a thread of its own and is abandoned at a deadline, so the route is never held for good.

use crate::sndout::Words;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// How long `amixer` and `luna-send` get before they are abandoned.
const CHILD_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a read or write of one of LG's driver files gets. A healthy one answers in microseconds; one that waits on the sound
/// card's control lock never does.
const IO_TIMEOUT: Duration = Duration::from_millis(1500);
const IO_STACK: usize = 128 * 1024;
/// How often a caller that keeps a reading fresh gets control back while a slow call or a wait is outstanding: a player stops
/// sending once the reading it polls is 300 ms old, and one luna call takes longer than that.
pub const PUMP_MS: u64 = 50;
/// Where LG's sound driver keeps its files, which are the ones that can wait on the sound card's control lock.
const DRIVER_ROOTS: [&str; 2] = ["/proc/aaudio/", "/proc/lgtv-driver/"];
/// Where one driver file operation stands: running, finished while its caller waited, or abandoned by its caller.
const IO_RUNNING: u8 = 0;
const IO_FINISHED: u8 = 1;
const IO_ABANDONED: u8 = 2;

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
    /// One luna call's reply text, `None` when it did not answer or failed to run. `pump` is called about every [`PUMP_MS`] while
    /// the call is outstanding.
    fn luna_pumped(&self, uri: &str, payload: &str, pump: &mut dyn FnMut()) -> Option<String>;
    /// One luna call's reply text, `None` when it did not answer or failed to run.
    fn luna(&self, uri: &str, payload: &str) -> Option<String> {
        self.luna_pumped(uri, payload, &mut || {})
    }
    /// Tasks stuck in uninterruptible sleep.
    fn stuck_tasks(&self) -> Vec<Stuck>;
    /// Whether an earlier read or write of a driver file is still blocked, so that new ones fail at once.
    fn driver_busy(&self) -> bool;
    /// Milliseconds since boot, suspend included.
    fn now_ms(&self) -> u64;
    fn sleep_ms(&self, ms: u64);
    /// Sleeps `ms` and calls `pump` after every [`PUMP_MS`] of it.
    fn sleep_pumped(&self, ms: u64, pump: &mut dyn FnMut()) {
        let mut left = ms;
        while left > 0 {
            let slice = left.min(PUMP_MS);
            self.sleep_ms(slice);
            left -= slice;
            pump();
        }
    }
}

/// The real TV.
#[derive(Clone, Debug)]
pub struct System {
    pub amixer: String,
    pub luna_send: String,
    /// How long `amixer` and `luna-send` get before they are abandoned.
    pub timeout: Duration,
    /// How long a read or write of a driver's file gets before it is abandoned.
    pub io_timeout: Duration,
    /// Where LG's driver files are. Only reads and writes under these run on a thread of their own and fail at once while one is
    /// stalled: they are the ones that can wait on the sound card's control lock. Any other file, the firmware's identity and the
    /// journal included, reads as ever.
    pub driver_roots: Vec<String>,
    /// How often [`Host::luna_pumped`] calls its `pump` while the child is outstanding.
    pub pump_every: Duration,
    /// Driver file operations that were abandoned and are still blocked. While there is one, new ones fail at once.
    stalled: Arc<AtomicUsize>,
}

impl Default for System {
    fn default() -> Self {
        Self {
            amixer: "amixer".into(),
            luna_send: "luna-send".into(),
            timeout: CHILD_TIMEOUT,
            io_timeout: IO_TIMEOUT,
            driver_roots: DRIVER_ROOTS.iter().map(|root| (*root).to_owned()).collect(),
            pump_every: Duration::from_millis(PUMP_MS),
            stalled: Arc::default(),
        }
    }
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "LG's sound driver did not answer")
}

impl System {
    /// The real TV, with `luna_send` as the program that makes luna calls.
    pub fn new(luna_send: impl Into<String>) -> Self {
        Self {
            luna_send: luna_send.into(),
            ..Self::default()
        }
    }

    fn is_driver_file(&self, path: &str) -> bool {
        self.driver_roots
            .iter()
            .any(|root| path.starts_with(root.as_str()))
    }

    /// Runs one operation on a driver's file on a thread of its own and waits [`System::io_timeout`] for it. The route is held
    /// while it waits, and a call that waits on the stuck control lock never returns: its thread is left behind, and until that
    /// thread comes back every further call fails at once, so a wedge neither piles up threads nor blocks the caller for good.
    fn bounded<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        if self.stalled.load(Ordering::SeqCst) > 0 {
            return Err(timed_out());
        }
        let (send, receive) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(IO_RUNNING));
        let (worker_state, stalled) = (state.clone(), self.stalled.clone());
        thread::Builder::new()
            .name("audiopass-io".into())
            .stack_size(IO_STACK)
            .spawn(move || {
                let _ = send.send(work());
                // Its caller gave up while it blocked: it no longer counts as stalled.
                if worker_state
                    .compare_exchange(IO_RUNNING, IO_FINISHED, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    stalled.fetch_sub(1, Ordering::SeqCst);
                }
            })?;
        match receive.recv_timeout(self.io_timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(io::Error::other("the file operation failed"))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Counted before the thread can see it was abandoned, so the count never goes below zero.
                self.stalled.fetch_add(1, Ordering::SeqCst);
                let abandoned = state.compare_exchange(
                    IO_RUNNING,
                    IO_ABANDONED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
                if abandoned.is_ok() {
                    return Err(timed_out());
                }
                // It finished between the deadline and now, and its result is already in the channel.
                self.stalled.fetch_sub(1, Ordering::SeqCst);
                receive.try_recv().unwrap_or_else(|_| Err(timed_out()))
            }
        }
    }
}

/// Waits for a child up to `timeout`, calling `pump` about every `pump_every`; on timeout kills it and leaves reaping to a thread
/// that may block for good.
fn wait_or_abandon(
    mut child: Child,
    timeout: Duration,
    pump_every: Duration,
    pump: &mut dyn FnMut(),
) -> Option<std::process::ExitStatus> {
    let end = Instant::now() + timeout;
    let mut pumped = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < end => {
                thread::sleep(Duration::from_millis(20));
                if pumped.elapsed() >= pump_every {
                    pump();
                    pumped = Instant::now();
                }
            }
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

fn write_file(path: &str, text: &str) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .open(path)?
        .write_all(text.as_bytes())
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
        if !self.is_driver_file(path) {
            return fs::read_to_string(path);
        }
        let path = path.to_owned();
        self.bounded(move || fs::read_to_string(path))
    }

    fn is_dir(&self, path: &str) -> bool {
        if !self.is_driver_file(path) {
            return Path::new(path).is_dir();
        }
        let path = path.to_owned();
        self.bounded(move || Ok(Path::new(&path).is_dir()))
            .unwrap_or(false)
    }

    fn write(&self, path: &str, text: &str) -> io::Result<()> {
        if !self.is_driver_file(path) {
            return write_file(path, text);
        }
        let (path, text) = (path.to_owned(), text.to_owned());
        self.bounded(move || write_file(&path, &text))
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
        match wait_or_abandon(child, self.timeout, self.pump_every, &mut || {}) {
            Some(status) if status.success() => Ok(()),
            Some(status) => Err(WriteError::Failed(format!("amixer exited with {status}"))),
            None => Err(WriteError::Hung),
        }
    }

    fn luna_pumped(&self, uri: &str, payload: &str, pump: &mut dyn FnMut()) -> Option<String> {
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
        wait_or_abandon(child, self.timeout, self.pump_every, pump)?;
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

    fn driver_busy(&self) -> bool {
        self.stalled.load(Ordering::SeqCst) > 0
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
            ..System::default()
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

    #[test]
    fn a_slow_luna_call_hands_control_back_while_it_waits() {
        // A script that takes a moment to answer stands in for audiooutputd.
        let dir = std::env::temp_dir().join(format!("audiopass-pump-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("luna-send");
        fs::write(
            &script,
            "#!/bin/sh\nsleep 0.15\necho '{\"returnValue\":true}'\n",
        )
        .unwrap();
        fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let system = System {
            luna_send: script.to_str().unwrap().to_owned(),
            pump_every: Duration::from_millis(10),
            ..System::default()
        };
        let mut pumps = 0;
        let reply = system.luna_pumped("luna://x/y", "{}", &mut || pumps += 1);
        assert_eq!(
            reply.as_deref().map(str::trim),
            Some(r#"{"returnValue":true}"#)
        );
        assert!(pumps >= 3, "{pumps} pumps while the call took 150 ms");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_that_blocks_is_abandoned_and_the_next_driver_calls_do_not_wait() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join(format!("audiopass-fifo-{}", std::process::id()));
        let elsewhere = tmp.join(format!("audiopass-else-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        let (fifo, regular, other) = (
            dir.join("pipe"),
            dir.join("regular"),
            elsewhere.join("identity"),
        );
        let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: `c_path` is a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        fs::write(&regular, "ok").unwrap();
        fs::write(&other, "build").unwrap();
        let (fifo, regular, other) = (
            fifo.to_str().unwrap(),
            regular.to_str().unwrap(),
            other.to_str().unwrap(),
        );
        // The directory stands in for LG's driver; a file elsewhere is not one of its files.
        let system = System {
            io_timeout: Duration::from_millis(100),
            driver_roots: vec![format!("{}/", dir.display())],
            ..System::default()
        };
        assert!(!system.driver_busy());

        // A read that waits for a writer that never comes stands in for one that waits on the control lock.
        let started = Instant::now();
        assert_eq!(
            system.read(fifo).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(system.driver_busy());

        // While it is blocked, no other driver file waits for the deadline again. Files that are not the driver's read as ever:
        // the firmware's identity must not turn into a refusal because the sound driver is busy.
        let again = Instant::now();
        assert_eq!(
            system.read(regular).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            system.write(regular, "x").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(system.read(other).unwrap(), "build");
        assert!(system.is_dir(elsewhere.to_str().unwrap()));
        assert!(again.elapsed() < Duration::from_millis(50));
        assert_eq!(fs::read_to_string(regular).unwrap(), "ok");

        // The writer arrives, the blocked read returns, and the calls work again.
        drop(OpenOptions::new().write(true).open(fifo).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match system.read(regular) {
                Ok(text) => {
                    assert_eq!(text, "ok");
                    break;
                }
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Err(e) => panic!("still blocked: {e}"),
            }
        }
        assert!(!system.driver_busy());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(elsewhere);
    }
}
