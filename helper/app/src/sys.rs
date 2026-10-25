//! The operating-system calls the helper makes, with the `unsafe` kept in this one module behind safe functions.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
}

/// Opens (creating) a lock file and takes an exclusive `flock` on it without waiting. `Some(file)` holds the lock for as long as
/// it lives; `None` means another process holds it. The kernel drops the lock when the holder dies, whatever way it dies.
/// The file is opened close-on-exec, so no program the helper starts can inherit the lock and outlive the helper with it.
pub fn try_lock(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o644)
        .open(path)?;
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(file));
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(error)
    }
}

/// What a lock file says about the process that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockInfo {
    pub pid: u32,
    /// The port the API listens on, once it does.
    pub port: Option<u16>,
}

/// Replaces the content of a lock file we hold with our pid and, once known, the API's port: `pid\nport\n`.
pub fn write_info(file: &mut File, pid: u32, port: Option<u16>) -> io::Result<()> {
    file.set_len(0)?;
    std::io::Seek::seek(file, io::SeekFrom::Start(0))?;
    let text = match port {
        Some(port) => format!("{pid}\n{port}\n"),
        None => format!("{pid}\n"),
    };
    file.write_all(text.as_bytes())?;
    file.flush()
}

/// What a lock file names, if it holds a pid.
pub fn read_info(path: &Path) -> Option<LockInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse::<u32>().ok().filter(|p| *p > 1)?;
    let port = lines.next().and_then(|l| l.trim().parse::<u16>().ok());
    Some(LockInfo { pid, port })
}

pub fn pid() -> u32 {
    std::process::id()
}

/// Sends `signal` to `pid`.
pub fn kill(pid: u32, signal: i32) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `kill` takes plain integers.
    check(unsafe { libc::kill(pid, signal) }).map(drop)
}

/// Whether a process with this pid exists.
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process exists.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The user id and group id of a user in the passwd database.
pub fn lookup_user(name: &str) -> Option<(u32, u32)> {
    let name = CString::new(name).ok()?;
    // SAFETY: `getpwnam` returns null or a pointer to a static record that is read at once, before any other call.
    unsafe {
        let entry = libc::getpwnam(name.as_ptr());
        if entry.is_null() { None } else { Some(((*entry).pw_uid, (*entry).pw_gid)) }
    }
}

pub fn is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// Lowers the process's scheduling priority by `increment` (the helper is background work and must never take a core from the
/// compositor); children inherit it.
pub fn lower_priority(increment: i32) {
    // SAFETY: `nice` takes a plain integer; a failure is harmless and ignored.
    unsafe {
        libc::nice(increment);
    }
}

/// Blocks the termination signals in the calling thread, so threads started after this inherit the mask and the signals are
/// delivered only to [`wait_termination`].
pub fn block_termination_signals() {
    // SAFETY: the set is initialised before use and only plain signal numbers are added.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
            libc::sigaddset(&mut set, signal);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Waits for a termination signal (they must be blocked first) and returns its number. A hangup is not a request to stop:
/// it is waited past, so an SSH session closing never takes the helper down.
pub fn wait_termination() -> i32 {
    // SAFETY: as in `block_termination_signals`; `sigwait` writes the signal number to a valid integer.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
            libc::sigaddset(&mut set, signal);
        }
        loop {
            let mut got: libc::c_int = 0;
            if libc::sigwait(&set, &mut got) != 0 {
                continue;
            }
            if got != libc::SIGHUP {
                return got;
            }
        }
    }
}

/// What `fork_detached` leaves each side holding.
pub enum Forked {
    /// The original process: the child's pid and the read end of the readiness pipe.
    Parent { child: u32, ready: File },
    /// The detached child: the write end of the readiness pipe.
    Child { ready: File },
}

/// Forks a detached daemon: the child becomes a session leader, has no controlling terminal, its standard streams point at
/// `/dev/null`, and every other inherited descriptor is closed (a shell's lock descriptor must not outlive the command that
/// started the helper, and a captured stdout must reach end of file when the starter exits). Call before any thread exists.
pub fn fork_detached() -> io::Result<Forked> {
    let mut fds: [RawFd; 2] = [0; 2];
    // SAFETY: `fds` has room for the two descriptors `pipe` writes.
    check(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    // SAFETY: `fork` in a process with one thread; the child only calls async-signal-safe functions until it returns to Rust code
    // that does not depend on other threads (there are none).
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        let error = io::Error::last_os_error();
        // SAFETY: both descriptors were just created and are owned here.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        return Err(error);
    }
    if pid > 0 {
        // SAFETY: the write end belongs to the child now; this side owns the read end.
        unsafe { libc::close(fds[1]) };
        // SAFETY: `fds[0]` is a valid descriptor owned only by this `File`.
        let ready = unsafe { File::from_raw_fd(fds[0]) };
        return Ok(Forked::Parent {
            child: u32::try_from(pid).unwrap_or(0),
            ready,
        });
    }
    // The child.
    // SAFETY: plain calls on descriptors this process owns.
    unsafe {
        libc::close(fds[0]);
        libc::setsid();
        libc::umask(0o022);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            for target in 0..3 {
                libc::dup2(null, target);
            }
            if null > 2 {
                libc::close(null);
            }
        }
        libc::chdir(c"/".as_ptr());
    }
    close_other_descriptors(fds[1]);
    // SAFETY: `fds[1]` is a valid descriptor owned only by this `File`.
    let ready = unsafe { File::from_raw_fd(fds[1]) };
    Ok(Forked::Child { ready })
}

fn close_other_descriptors(keep: RawFd) {
    let open: Vec<RawFd> = std::fs::read_dir("/proc/self/fd")
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<RawFd>().ok()))
                .collect()
        })
        .unwrap_or_else(|_| (3..256).collect());
    for fd in open {
        if fd > 2 && fd != keep {
            // SAFETY: closing a descriptor number; the directory handle's own number is already closed and gives EBADF, which is ignored.
            unsafe {
                libc::close(fd);
            }
        }
    }
}

/// Reads one line from a pipe, waiting at most `timeout`. `None` if nothing came (the other end closed or time ran out).
pub fn read_line_timeout(pipe: &mut File, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut poll = libc::pollfd {
            fd: pipe.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: one valid `pollfd` is passed with a count of one.
        let ready = unsafe { libc::poll(&mut poll, 1, millis) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if ready == 0 {
            return None;
        }
        match pipe.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => return Some(String::from_utf8_lossy(&line).into_owned()),
            Ok(1) => line.push(byte[0]),
            _ => return (!line.is_empty()).then(|| String::from_utf8_lossy(&line).into_owned()),
        }
    }
}

/// Resident memory of this process in KiB, from `/proc/self/status`.
pub fn rss_kb() -> Option<u64> {
    status_field("VmRSS:")
}

/// The number of threads of this process.
pub fn threads() -> Option<u64> {
    status_field("Threads:")
}

fn status_field(name: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(name))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_is_exclusive_and_released_with_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper.lock");
        let mut first = try_lock(&path).unwrap().expect("first lock");
        write_info(&mut first, 4242, Some(18731)).unwrap();
        assert!(try_lock(&path).unwrap().is_none());
        assert_eq!(read_info(&path), Some(LockInfo { pid: 4242, port: Some(18731) }));
        drop(first);
        assert!(try_lock(&path).unwrap().is_some());
    }

    #[test]
    fn the_info_is_rewritten_not_appended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper.lock");
        let mut file = try_lock(&path).unwrap().unwrap();
        write_info(&mut file, 123456, Some(18731)).unwrap();
        write_info(&mut file, 7, None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "7\n");
        assert_eq!(read_info(&path), Some(LockInfo { pid: 7, port: None }));
    }

    #[test]
    fn info_that_is_not_a_pid_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x");
        assert_eq!(read_info(&path), None);
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_info(&path), None);
        std::fs::write(&path, "abc\n").unwrap();
        assert_eq!(read_info(&path), None);
        std::fs::write(&path, "1\n").unwrap();
        assert_eq!(read_info(&path), None);
        std::fs::write(&path, "55\nnot-a-port\n").unwrap();
        assert_eq!(read_info(&path), Some(LockInfo { pid: 55, port: None }));
    }

    #[test]
    fn this_process_is_alive_and_a_huge_pid_is_not() {
        assert!(pid_alive(pid()));
        assert!(!pid_alive(4_000_000_000));
    }

    #[test]
    fn the_passwd_database_is_read() {
        assert_eq!(lookup_user("root"), Some((0, 0)));
        assert_eq!(lookup_user("no-such-user-here"), None);
    }

    #[test]
    fn memory_and_threads_are_reported() {
        assert!(rss_kb().is_some_and(|k| k > 100));
        assert!(threads().is_some_and(|t| t >= 1));
    }

    #[test]
    fn a_line_is_read_from_a_pipe_with_a_timeout() {
        let mut fds: [RawFd; 2] = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut read = unsafe { File::from_raw_fd(fds[0]) };
        let mut write = unsafe { File::from_raw_fd(fds[1]) };
        assert_eq!(read_line_timeout(&mut read, Duration::from_millis(30)), None);
        write.write_all(b"ok 18731\nrest").unwrap();
        assert_eq!(read_line_timeout(&mut read, Duration::from_secs(1)).as_deref(), Some("ok 18731"));
        drop(write);
        assert_eq!(read_line_timeout(&mut read, Duration::from_secs(1)).as_deref(), Some("rest"));
    }
}
