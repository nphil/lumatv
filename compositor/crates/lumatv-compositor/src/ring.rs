//! The trace ring: what the compositor was doing in its last seconds, kept where a crash cannot take it. A fixed-size file in the
//! runtime directory (tmpfs) mapped shared and written as a circular text log, so it is there after a SIGKILL, a signal from the
//! Mali blob or a hang the supervisor ended: the supervisor copies it into the crash bundle (`compositor/sm/lumatv-sm.c`,
//! `crash_bundle`), and `ctl ring` reads it live. The gate's start-up trace (`trace:` lines) is the same idea for a few seconds of
//! a client's start; this is always on, with the loop's passes and wake-ups that ran long, each flip, a snapshot of the counters
//! every second, every line the compositor logs, and a panic's message and place.
//!
//! The format, which `lumatv-sm.c` reads too (keep the two in step; a test of each pins it):
//!
//! | offset | size | |
//! |---|---|---|
//! | 0 | 8 | `LTVRING1` |
//! | 8 | 8 | `size`: the bytes of text area, little endian |
//! | 16 | 8 | `head`: bytes ever written, little endian; the next byte goes at `head % size` |
//! | 24 | 8 | the writer's pid |
//! | 64 | `size` | text, lines ending in `\n`, the oldest overwritten first |
//!
//! Writing is a copy into mapped memory: no system call, no allocation. A line is cut at [`LINE_MAX`] bytes. Nothing here is
//! written on a path that waits: [`note`] takes a mutex that only the rare other thread (a log line from the control socket's
//! thread, a panic) ever contends.

use std::fmt::{self, Write as _};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::OnceLock;

use parking_lot::Mutex;

pub const MAGIC: &[u8; 8] = b"LTVRING1";
pub const HEADER: usize = 64;
/// The text area of the compositor's own ring: about 25 s of the busiest trace, minutes of an idle one.
pub const SIZE: usize = 192 * 1024;
/// The ring's file in the runtime directory; the supervisor looks for it there.
pub const FILE_NAME: &str = "lumatv-trace.ring";
pub const LINE_MAX: usize = 200;

struct Ring {
    base: *mut u8,
    size: usize,
    writer: Mutex<()>,
}

// SAFETY: the mapping lives for the process; writes are under `writer`, and readers (`text`) accept a line torn by a write in progress.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

static RING: OnceLock<Ring> = OnceLock::new();

/// A line's text in a stack buffer, cut at its capacity.
struct Line {
    buf: [u8; LINE_MAX],
    len: usize,
}

impl fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = LINE_MAX - 1 - self.len;
        let mut take = s.len().min(room);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// Maps `path` as the ring (made, or made again, with `size` bytes of text area) and writes its header. The compositor's one ring.
pub fn open(path: &Path, size: usize) -> io::Result<()> {
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
    file.set_len((HEADER + size) as u64)?;
    // SAFETY: a shared read-write mapping of the file just sized to HEADER + size, kept for the life of the process.
    let base = unsafe { libc::mmap(std::ptr::null_mut(), HEADER + size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    if base == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let ring = Ring { base: base as *mut u8, size, writer: Mutex::new(()) };
    // SAFETY: the header is inside the mapping.
    unsafe {
        std::ptr::copy_nonoverlapping(MAGIC.as_ptr(), ring.base, 8);
        std::ptr::write_unaligned(ring.base.add(8) as *mut u64, (size as u64).to_le());
        std::ptr::write_unaligned(ring.base.add(16) as *mut u64, 0);
        std::ptr::write_unaligned(ring.base.add(24) as *mut u64, (std::process::id() as u64).to_le());
    }
    RING.set(ring).map_err(|_| io::Error::other("the trace ring is open already"))
}

pub fn is_open() -> bool {
    RING.get().is_some()
}

/// Appends one line, stamped with CLOCK_MONOTONIC seconds like the log's. Nothing when no ring is open.
pub fn note(args: fmt::Arguments) {
    let Some(ring) = RING.get() else { return };
    let mut line = Line { buf: [0; LINE_MAX], len: 0 };
    let _ = write!(line, "{:.3} ", crate::output::monotonic_ns() as f64 / 1e9);
    let _ = line.write_fmt(args);
    line.buf[line.len] = b'\n';
    ring.append(&line.buf[..line.len + 1]);
}

/// `note` with a format string.
#[macro_export]
macro_rules! ring_note {
    ($($arg:tt)*) => {
        $crate::ring::note(format_args!($($arg)*))
    };
}

impl Ring {
    fn append(&self, bytes: &[u8]) {
        let _w = self.writer.lock();
        // SAFETY: the header and the text area are inside the mapping; `at + n` is cut to the area's end for the first copy.
        unsafe {
            let head = u64::from_le(std::ptr::read_unaligned(self.base.add(16) as *const u64));
            let mut at = (head % self.size as u64) as usize;
            let mut rest = bytes;
            while !rest.is_empty() {
                let n = rest.len().min(self.size - at);
                std::ptr::copy_nonoverlapping(rest.as_ptr(), self.base.add(HEADER + at), n);
                rest = &rest[n..];
                at = (at + n) % self.size;
            }
            std::ptr::write_volatile(self.base.add(16) as *mut u64, (head + bytes.len() as u64).to_le());
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        // SAFETY: the whole mapping is readable; a write in progress can tear the newest line, which `unwrap` leaves to the reader.
        unsafe { std::slice::from_raw_parts(self.base, HEADER + self.size) }.to_vec()
    }
}

/// The ring's text in order, oldest line first, from the bytes of the file (or of a copy of it); `None` when the bytes are not a ring.
/// Once the ring has wrapped its oldest line is left out: the bytes that began it were overwritten, and nothing says where it began.
pub fn unwrap(file: &[u8]) -> Option<String> {
    if file.len() < HEADER || &file[..8] != MAGIC {
        return None;
    }
    let size = u64::from_le_bytes(file[8..16].try_into().ok()?) as usize;
    let head = u64::from_le_bytes(file[16..24].try_into().ok()?);
    if size == 0 || file.len() < HEADER + size {
        return None;
    }
    let area = &file[HEADER..HEADER + size];
    let (first, second): (&[u8], &[u8]) = if head <= size as u64 {
        (&area[..head as usize], &[])
    } else {
        let at = (head % size as u64) as usize;
        (&area[at..], &area[..at])
    };
    let mut text = Vec::with_capacity(first.len() + second.len());
    text.extend_from_slice(first);
    text.extend_from_slice(second);
    let mut text = String::from_utf8_lossy(&text).into_owned();
    // The oldest bytes begin somewhere in a line the newer ones overwrote the start of (nothing says where a line began): that line is left out.
    if head > size as u64 {
        text = text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default();
    }
    Some(text)
}

/// The text of the open ring, oldest line first.
pub fn text() -> Option<String> {
    unwrap(&RING.get()?.snapshot())
}

/// The last `n` lines of the open ring.
pub fn tail(n: usize) -> Vec<String> {
    let text = text().unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

/// A panic's message and place go in the ring before the process ends (`panic = "abort"` on the set), then to the hook that was there.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut line = Line { buf: [0; LINE_MAX], len: 0 };
        let _ = write!(line, "{info}");
        // The message's lines are joined: the ring's lines are one each.
        let flat = String::from_utf8_lossy(&line.buf[..line.len]).replace('\n', " | ");
        note(format_args!("PANIC {flat}"));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring of a few hundred bytes, not the process's: the same writer over a mapping of its own.
    fn small(size: usize) -> (Ring, tempfile_path::Dir) {
        let dir = tempfile_path::Dir::new();
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(dir.path("ring")).unwrap();
        file.set_len((HEADER + size) as u64).unwrap();
        // SAFETY: as in `open`.
        let base = unsafe { libc::mmap(std::ptr::null_mut(), HEADER + size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0) };
        assert_ne!(base, libc::MAP_FAILED);
        let ring = Ring { base: base as *mut u8, size, writer: Mutex::new(()) };
        // SAFETY: the header is inside the mapping.
        unsafe {
            std::ptr::copy_nonoverlapping(MAGIC.as_ptr(), ring.base, 8);
            std::ptr::write_unaligned(ring.base.add(8) as *mut u64, (size as u64).to_le());
        }
        (ring, dir)
    }

    /// A directory under the system's temp dir, removed when dropped.
    mod tempfile_path {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let p = std::env::temp_dir().join(format!("lumatv-ring-test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
            pub fn path(&self, name: &str) -> std::path::PathBuf {
                self.0.join(name)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn lines_come_back_in_order() {
        let (ring, _d) = small(1024);
        ring.append(b"one\n");
        ring.append(b"two\n");
        assert_eq!(unwrap(&ring.snapshot()).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn a_wrap_that_falls_on_a_line_start_leaves_the_oldest_line_out_all_the_same() {
        // Eight-byte lines in a 64-byte area: 20 lines written, the newest eight are there; nothing says the oldest starts a line.
        let (ring, _d) = small(64);
        for i in 0..20 {
            ring.append(format!("line {i:02}\n").as_bytes());
        }
        let text = unwrap(&ring.snapshot()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, ["line 13", "line 14", "line 15", "line 16", "line 17", "line 18", "line 19"]);
    }

    #[test]
    fn a_wrap_inside_a_line_leaves_that_line_out_and_the_rest_whole() {
        let (ring, _d) = small(64);
        for i in 0..30 {
            ring.append(format!("n {i}\n").as_bytes());
        }
        let text = unwrap(&ring.snapshot()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(*lines.last().unwrap(), "n 29");
        assert!(lines.len() >= 9, "{lines:?}");
        for (a, b) in lines.iter().zip(lines.iter().skip(1)) {
            let (x, y): (u32, u32) = (a[2..].parse().unwrap(), b[2..].parse().unwrap());
            assert_eq!(x + 1, y, "the line the wrap cut in two is gone, the rest are in a row: {lines:?}");
        }
    }

    #[test]
    fn a_line_longer_than_the_ring_area_still_leaves_a_ring() {
        let (ring, _d) = small(16);
        ring.append(&[b'x'; 40]);
        ring.append(b"end\n");
        assert!(unwrap(&ring.snapshot()).is_some());
    }

    #[test]
    fn what_is_not_a_ring_is_none() {
        assert_eq!(unwrap(b"short"), None);
        assert_eq!(unwrap(&[0u8; 200]), None);
        let mut bad = vec![0u8; HEADER + 8];
        bad[..8].copy_from_slice(MAGIC);
        assert_eq!(unwrap(&bad), None, "a size of zero");
    }

    #[test]
    fn a_line_is_cut_at_its_capacity_on_a_character() {
        let mut line = Line { buf: [0; LINE_MAX], len: 0 };
        for _ in 0..100 {
            line.write_str("é").unwrap();
        }
        assert!(line.len < LINE_MAX && line.len % 2 == 0, "{} bytes of two-byte characters", line.len);
        assert!(std::str::from_utf8(&line.buf[..line.len]).is_ok());
    }

    #[test]
    fn the_processs_ring_opens_once_and_takes_stamped_lines() {
        let d = tempfile_path::Dir::new();
        assert!(!is_open());
        open(&d.path("trace.ring"), 4096).unwrap();
        assert!(is_open());
        assert!(open(&d.path("again.ring"), 4096).is_err(), "one ring a process");
        crate::ring_note!("F flip at {} reached {} us", 12, 340);
        let tail = tail(5);
        assert_eq!(tail.len(), 1);
        assert!(tail[0].ends_with(" F flip at 12 reached 340 us"), "{tail:?}");
        let (stamp, _) = tail[0].split_once(' ').unwrap();
        assert!(stamp.parse::<f64>().is_ok(), "the line starts with CLOCK_MONOTONIC seconds: {}", tail[0]);
        // The file on disk is the ring: the supervisor reads it after the process is gone.
        let on_disk = std::fs::read(d.path("trace.ring")).unwrap();
        assert_eq!(unwrap(&on_disk).unwrap(), text().unwrap());
        assert_eq!(u64::from_le_bytes(on_disk[24..32].try_into().unwrap()), std::process::id() as u64);
    }
}
