//! The log makes the loop wait on nothing: a line logged on the loop's thread is copied into a queue and written by a thread of the
//! log's own (`logsink`; round 17's trace had the loop asleep in `write` to the log file, on the journal). The loop thread's `write`
//! calls are counted by the kernel (`syscw` in /proc/thread-self/io): none over any number of lines, whether the file takes them or
//! not (a pipe nobody reads, which a direct write would hold the thread on for good).

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use log::{Level, Log, Record};
use lumatv_compositor::logsink::{LINE_MAX, Logger, QUEUE_LINES};

/// A file the log can always write to, and what has been written to it since it was last read.
struct TempFile {
    path: PathBuf,
    file: File,
    seen: usize,
}

impl TempFile {
    fn new(tag: &str) -> TempFile {
        let path = std::env::temp_dir().join(format!("lumatv-logsink-{}-{tag}", std::process::id()));
        let file = File::create(&path).unwrap();
        TempFile { path, file, seen: 0 }
    }

    fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// The whole file.
    fn all(&self) -> String {
        String::from_utf8(std::fs::read(&self.path).unwrap()).expect("the log is UTF-8, cut at a character")
    }

    /// What was written since the last call.
    fn take(&mut self) -> String {
        let all = self.all();
        let new = all[self.seen..].to_string();
        self.seen = all.len();
        new
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

/// A pipe, its reading end not waiting: the file that takes lines only as it is read.
fn pipe() -> (File, File) {
    let mut fds = [0i32; 2];
    // SAFETY: two descriptors for pipe2 to fill.
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    // SAFETY: the descriptors are new and owned here only.
    let (read, write) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
    // SAFETY: the size of a pipe (a page at the least, whatever the user's pipe pages allow) and the flags of a descriptor, on the ones just made.
    unsafe {
        libc::fcntl(fds[1], libc::F_SETPIPE_SZ, 4096);
        let flags = libc::fcntl(fds[0], libc::F_GETFL);
        assert_eq!(libc::fcntl(fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK), 0);
    }
    (read, write)
}

/// The calling thread's count of write system calls, from the kernel's accounting per thread; None on a kernel without it.
fn syscw() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/thread-self/io").ok()?;
    text.lines().find_map(|l| l.strip_prefix("syscw:")?.trim().parse().ok())
}

fn log_line(logger: &Logger, args: std::fmt::Arguments) {
    logger.log(&Record::builder().level(Level::Info).target("test").args(args).build());
}

fn monotonic() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a stack timespec for clock_gettime.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

/// The numbers of the `line N` lines in `text`, in file order.
fn numbered(text: &str) -> Vec<u64> {
    text.lines().filter_map(|l| l.split("] info line ").nth(1)?.parse().ok()).collect()
}

/// The lines the writer says it was not given, from the notes in `text`.
fn dropped(text: &str) -> u64 {
    text.lines().filter_map(|l| l.split("warn log: ").nth(1)?.split(' ').next()?.parse::<u64>().ok()).sum()
}

#[test]
fn the_loop_makes_no_write_call_to_log_its_lines() {
    let file = TempFile::new("loop");
    let fd = file.fd();
    let logger = Logger::start(fd).unwrap();
    let lines = QUEUE_LINES - 16;
    let (from, to, counted) = thread::scope(|s| {
        s.spawn(|| {
            let start = syscw();
            // The counter sees a write when there is one.
            // SAFETY: a live buffer and the file's descriptor.
            assert_eq!(unsafe { libc::write(fd, b"direct\n".as_ptr().cast(), 7) }, 7);
            let direct = syscw().zip(start).map(|(a, b)| a - b);
            let before = syscw();
            let from = monotonic();
            for n in 0..lines {
                log_line(&logger, format_args!("line {n}"));
            }
            let to = monotonic();
            (from, to, direct.zip(syscw().zip(before).map(|(a, b)| a - b)))
        })
        .join()
        .unwrap()
    });
    match counted {
        Some((direct, logged)) => {
            assert_eq!(direct, 1, "the kernel's count must see a write made on the thread");
            assert_eq!(logged, 0, "the loop's thread made {logged} write call(s) to log {lines} lines");
        }
        None => eprintln!("skipped the count: this kernel has no per-thread I/O accounting (/proc/thread-self/io)"),
    }
    assert!(logger.flush_within(Duration::from_secs(5)));
    let text = file.all();
    assert!(text.starts_with("direct\n"), "{text:?}");
    assert_eq!(numbered(&text), (0..lines as u64).collect::<Vec<_>>(), "every line, once, in order");
    // The format the gate reads: CLOCK_MONOTONIC seconds in brackets, the level, the text.
    let first = text.lines().nth(1).unwrap();
    let stamp: f64 = first.strip_prefix("[srv ").unwrap().split(']').next().unwrap().trim().parse().unwrap();
    assert!(stamp >= from - 0.001 && stamp <= to + 0.001, "{first:?} is not between {from} and {to}");
    assert!(first.ends_with("] info line 0"), "{first:?}");
}

#[test]
fn a_file_that_takes_no_lines_holds_the_writer_and_not_the_loop() {
    // The pipe takes a page of lines and then holds every write, as a log on a disk that is waiting for its journal would.
    let (mut read, write) = pipe();
    let logger = Arc::new(Logger::start(write.as_raw_fd()).unwrap());
    const LINES: u64 = 20_000;
    let (tx, rx) = mpsc::channel();
    let on_loop = Arc::clone(&logger);
    // Not a scoped thread: if the loop did wait for the pipe, the test says so instead of waiting with it.
    let on_the_loop = thread::Builder::new()
        .name("loop".into())
        .spawn(move || {
            let before = syscw();
            let t0 = Instant::now();
            for n in 0..LINES {
                log_line(&on_loop, format_args!("line {n}"));
            }
            tx.send((t0.elapsed(), syscw().zip(before).map(|(a, b)| a - b))).ok();
        })
        .unwrap();
    let (took, writes) = rx.recv_timeout(Duration::from_secs(30)).expect("the loop did not come back from logging into a pipe nobody reads");
    on_the_loop.join().unwrap();
    assert!(took < Duration::from_secs(10), "{LINES} lines took the loop {took:?}");
    if let Some(writes) = writes {
        assert_eq!(writes, 0, "the loop's thread made {writes} write call(s) with the file not taking lines");
    }
    let refused = logger.refused();
    assert!(refused > 0, "{LINES} lines into a pipe nobody reads must overflow a queue of {QUEUE_LINES}");
    let flush_started = Instant::now();
    assert!(!logger.flush_within(Duration::from_millis(20)), "an unread full pipe cannot acknowledge a flush");
    assert!(flush_started.elapsed() < Duration::from_secs(1), "a blocked sink ignored the flush deadline");

    // Read now: the writer, which was held, writes what it has, and says what it dropped.
    let reader = thread::spawn(move || {
        // The reading end does not wait: poll it until the writing end is closed and it is empty.
        let mut chunk = [0u8; 4096];
        let mut bytes = Vec::new();
        loop {
            match read.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(1)),
                Err(e) => panic!("{e}"),
            }
        }
        String::from_utf8(bytes).unwrap()
    });
    drop(Arc::into_inner(logger).expect("the loop's thread has finished with the logger"));
    drop(write);
    let text = reader.join().unwrap();
    let numbers = numbered(&text);
    assert_eq!(numbers.first(), Some(&0));
    assert!(numbers.windows(2).all(|w| w[0] < w[1]), "the lines are in the order they were logged");
    assert_eq!(numbers.len() as u64 + refused, LINES, "every line is either in the file or counted as dropped");
    assert_eq!(dropped(&text), refused, "the file says how many it was not given");
}

#[test]
fn a_line_too_long_is_cut_at_a_character_and_marked() {
    let mut file = TempFile::new("cut");
    let logger = Logger::start(file.fd()).unwrap();
    // The prefix (`[srv <seconds>] info `) is measured, not assumed: its width grows with the uptime.
    log_line(&logger, format_args!("x"));
    assert!(logger.flush_within(Duration::from_secs(5)));
    let prefix = file.take().len() - "x\n".len();
    // A line fits whole up to LINE_MAX - 4 bytes of text (the `…` and the newline keep the rest).
    let fits = LINE_MAX - 4 - prefix;
    for (len, whole) in [(fits - 1, true), (fits, true), (fits + 1, false)] {
        log_line(&logger, format_args!("{}", "a".repeat(len)));
        assert!(logger.flush_within(Duration::from_secs(5)));
        let line = file.take();
        if whole {
            assert_eq!(line.len(), prefix + len + 1, "{len} characters fit whole");
            assert!(line.ends_with("a\n"));
        } else {
            assert_eq!(line.len(), LINE_MAX, "{len} characters are cut to the line's size");
            assert!(line.ends_with("a…\n"), "{line:?}");
        }
    }
    // Two-byte characters: the cut falls between them, never inside one.
    log_line(&logger, format_args!("{}", "é".repeat(5000)));
    log_line(&logger, format_args!("short"));
    assert!(logger.flush_within(Duration::from_secs(5)));
    let text = file.take();
    let mut lines = text.split_inclusive('\n');
    let long = lines.next().unwrap();
    assert!(long.len() <= LINE_MAX && long.ends_with("é…\n"), "{long:?}");
    assert!(lines.next().unwrap().ends_with("] info short\n"));
}

#[test]
fn flush_returns_when_the_lines_are_in_the_file() {
    let file = TempFile::new("flush");
    let logger = Logger::start(file.fd()).unwrap();
    for n in 0..100 {
        log_line(&logger, format_args!("line {n}"));
    }
    logger.flush();
    let text = file.all();
    assert_eq!(logger.refused(), 0);
    assert_eq!(numbered(&text).len(), 100);
    assert!(text.ends_with("] info line 99\n"), "{text:?}");
}

#[test]
fn lines_from_many_threads_are_whole_and_in_each_threads_order() {
    let file = TempFile::new("threads");
    let logger = Arc::new(Logger::start(file.fd()).unwrap());
    let threads = 4;
    let per = 400u64;
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let logger = Arc::clone(&logger);
            thread::spawn(move || {
                for n in 0..per {
                    log_line(&logger, format_args!("t{t} n{n}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert!(logger.flush_within(Duration::from_secs(5)));
    let text = file.all();
    let mut last = vec![None::<u64>; threads];
    let mut seen = 0u64;
    for line in text.lines().filter(|l| !l.contains("] warn log: ")) {
        let body = line.split("] info ").nth(1).unwrap_or_else(|| panic!("a torn line: {line:?}"));
        let (t, n) = body.strip_prefix('t').and_then(|b| b.split_once(" n")).unwrap_or_else(|| panic!("a torn line: {line:?}"));
        let (t, n): (usize, u64) = (t.parse().unwrap(), n.parse().unwrap());
        assert!(last[t].is_none_or(|l| l < n), "thread {t}'s lines out of order at {n}");
        last[t] = Some(n);
        seen += 1;
    }
    assert_eq!(seen + logger.refused(), threads as u64 * per);
    assert_eq!(dropped(&text), logger.refused());
}

#[test]
fn an_idle_writer_is_woken_by_the_next_line() {
    let file = TempFile::new("wake");
    let logger = Logger::start(file.fd()).unwrap();
    log_line(&logger, format_args!("line 0"));
    assert!(logger.flush_within(Duration::from_secs(5)));
    // Let recv block with an empty queue: a later line must wake it without a polling timeout.
    thread::sleep(Duration::from_millis(100));
    let t0 = Instant::now();
    log_line(&logger, format_args!("line 1"));
    while numbered(&file.all()).len() < 2 && t0.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(1));
    }
    let arrived = t0.elapsed();
    assert_eq!(numbered(&file.all()), [0, 1]);
    assert!(arrived < Duration::from_millis(500), "the line reached the file after {arrived:?}: the writer was not woken");
}

#[test]
fn lines_logged_before_the_logger_is_dropped_are_written() {
    // The writer is held on a full pipe when the logger closes, with lines still queued behind the write it is in.
    let (mut read, write) = pipe();
    let logger = Logger::start(write.as_raw_fd()).unwrap();
    for n in 0..QUEUE_LINES as u64 {
        log_line(&logger, format_args!("line {n}"));
    }
    let reader = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        let mut chunk = [0u8; 4096];
        let mut bytes = Vec::new();
        loop {
            match read.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(1)),
                Err(e) => panic!("{e}"),
            }
        }
        String::from_utf8(bytes).unwrap()
    });
    let refused = logger.refused();
    drop(logger);
    drop(write);
    let text = reader.join().unwrap();
    assert_eq!(numbered(&text).len() as u64 + refused, QUEUE_LINES as u64, "{text:?}");
}

#[test]
fn a_failed_file_does_not_block_the_loop_or_poison_the_next_flush() {
    let full = File::options().write(true).open("/dev/full").unwrap();
    let logger = Logger::start(full.as_raw_fd()).unwrap();
    let before = syscw();
    for n in 0..2000 {
        log_line(&logger, format_args!("line {n}"));
    }
    let after = syscw();
    if let Some((before, after)) = before.zip(after) {
        assert_eq!(before, after, "failed log writes reached the calling thread");
    }
    assert!(logger.flush_within(Duration::from_secs(5)), "the failed write must release its flush marker");
    log_line(&logger, format_args!("after ENOSPC"));
    assert!(logger.flush_within(Duration::from_secs(5)), "a failed sink must not kill the writer");
}

#[test]
fn the_real_host_compositor_starts_the_writer_and_flushes_before_exit() {
    use std::process::{Command, Stdio};
    let file = TempFile::new("real");
    let dir = std::env::temp_dir().join(format!("lumatv-logsink-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "1", "--mlock", "off"])
        .stdout(Stdio::null())
        .stderr(file.file.try_clone().unwrap())
        .spawn()
        .unwrap();
    let mut saw_writer = false;
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Ok(tasks) = std::fs::read_dir(format!("/proc/{}/task", child.id())) {
            saw_writer |= tasks.flatten().any(|t| std::fs::read_to_string(t.path().join("comm")).is_ok_and(|s| s.trim() == "lumatv-log"));
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            panic!("host compositor did not exit: {}", file.all());
        }
        thread::sleep(Duration::from_millis(10));
    };
    std::fs::remove_dir_all(&dir).unwrap();
    let text = file.all();
    assert!(status.success(), "{status}: {text}");
    assert!(saw_writer, "the real compositor never started lumatv-log: {text}");
    assert!(text.contains("] info deadline after 1 s"), "{text}");
    assert!(text.contains("] info summary:"), "the final queued summary was lost: {text}");
}
