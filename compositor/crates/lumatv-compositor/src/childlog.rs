//! A client's stdout and stderr, kept to a fixed size. A client given an append-mode file grows it without limit (the Rust shell's
//! `shell-rust.log` reached 180 MiB on the set, 90 % of a 966 MiB /var that LG's own cleanup empties at 98 %), and a rename from outside
//! cannot bound a file the running client still holds. The client gets a pipe instead; a thread here owns the file, cuts overlong
//! lines and, past `LOG_MAX`, moves the file to `<name>.old` (one generation) and starts a new one: at most twice `LOG_MAX` on disk.
//! The frame loop never writes to it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::thread;

/// One generation's size; the file and its `.old` together stay under twice this.
pub const LOG_MAX: u64 = 1 << 20;
/// A line is cut here (a client's helper request with a whole feed in its body was the longest kind).
pub const LINE_MAX: usize = 4096;

fn old_of(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Brings a file left by an earlier build (append-only, any size) down to its last `max` bytes, from the first whole line on.
fn shrink(path: &Path, max: u64) -> io::Result<()> {
    let len = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return Ok(()),
    };
    if len <= max {
        return Ok(());
    }
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(len - max))?;
    let mut tail = Vec::with_capacity(max as usize);
    f.take(max).read_to_end(&mut tail)?;
    let from = tail.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, &tail[from..])?;
    fs::rename(&tmp, path)
}

pub struct Bounded {
    path: PathBuf,
    file: File,
    size: u64,
    max: u64,
    line: Vec<u8>,
    cut: bool,
    out: Vec<u8>,
}

impl Bounded {
    pub fn open(path: &Path, max: u64) -> io::Result<Bounded> {
        shrink(path, max)?;
        shrink(&old_of(path), max)?;
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata()?.len();
        Ok(Bounded { path: path.to_owned(), file, size, max, line: Vec::new(), cut: false, out: Vec::new() })
    }

    /// Takes what the client wrote, in whatever pieces the pipe gave it.
    pub fn push(&mut self, data: &[u8]) {
        for piece in data.split_inclusive(|b| *b == b'\n') {
            let (body, end) = match piece.split_last() {
                Some((b'\n', rest)) => (rest, true),
                _ => (piece, false),
            };
            let room = LINE_MAX - self.line.len();
            if body.len() > room {
                self.cut = true;
            }
            self.line.extend_from_slice(&body[..body.len().min(room)]);
            if end {
                self.end_line();
            }
        }
        self.flush();
    }

    /// A last line the client left without its newline.
    pub fn finish(&mut self) {
        if !self.line.is_empty() {
            self.end_line();
        }
        self.flush();
    }

    fn end_line(&mut self) {
        let extra = if self.cut { "…".len() } else { 0 } + 1;
        if self.size + (self.out.len() + self.line.len() + extra) as u64 > self.max {
            self.flush();
            self.rotate();
        }
        self.out.append(&mut self.line);
        if self.cut {
            self.out.extend_from_slice("…".as_bytes());
            self.cut = false;
        }
        self.out.push(b'\n');
    }

    fn flush(&mut self) {
        if !self.out.is_empty() {
            if self.file.write_all(&self.out).is_ok() {
                self.size += self.out.len() as u64;
            }
            self.out.clear();
        }
    }

    fn rotate(&mut self) {
        let _ = fs::rename(&self.path, old_of(&self.path));
        if let Ok(f) = OpenOptions::new().create(true).append(true).truncate(false).open(&self.path) {
            self.file = f;
            self.size = 0;
        }
    }
}

/// The write end of a pipe for a client's stdout and stderr (clone it for the second); a thread copies the pipe into `path`, bounded,
/// until every write end is closed. None when the file or the pipe cannot be made: the client then has no log.
pub fn pipe(path: &Path) -> Option<OwnedFd> {
    let mut sink = Bounded::open(path, LOG_MAX).ok()?;
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 fills the two descriptors, which are ours and owned below.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    // SAFETY: both are fresh descriptors from pipe2, each owned once.
    let (mut read, write) = unsafe { (File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let spawned = thread::Builder::new().name("client-log".into()).spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match read.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => sink.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        sink.finish();
    });
    spawned.ok().map(|_| write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lumatv-childlog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_stream_of_any_size_stays_under_two_generations() {
        let d = dir("bound");
        let p = d.join("x.log");
        let mut b = Bounded::open(&p, 1000).unwrap();
        for i in 0..5000 {
            b.push(format!("line {i} {}\n", "x".repeat(40)).as_bytes());
        }
        b.finish();
        let (new, old) = (fs::metadata(&p).unwrap().len(), fs::metadata(old_of(&p)).unwrap().len());
        assert!(new <= 1000 && old <= 1000, "{new} {old}");
        assert!(fs::read_to_string(&p).unwrap().contains("line 4999 "), "the newest line is kept");
        assert!(!d.join("x.log.tmp").exists());
    }

    #[test]
    fn a_long_line_is_cut_and_a_split_one_is_joined() {
        let d = dir("cut");
        let p = d.join("x.log");
        let mut b = Bounded::open(&p, 1 << 20).unwrap();
        b.push(format!("{}\n", "y".repeat(LINE_MAX * 3)).as_bytes());
        b.push(b"par");
        b.push(b"ts\nlast");
        b.finish();
        let text = fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), LINE_MAX + "…".len());
        assert!(lines[0].ends_with('…'));
        assert_eq!(&lines[1..], ["parts", "last"]);
    }

    #[test]
    fn a_file_an_older_build_left_is_brought_down_to_its_tail() {
        let d = dir("shrink");
        let p = d.join("x.log");
        let mut big = String::new();
        for i in 0..1000 {
            big.push_str(&format!("old {i}\n"));
        }
        fs::write(&p, &big).unwrap();
        let b = Bounded::open(&p, 500).unwrap();
        assert!(b.size <= 500);
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("old ") && text.ends_with("old 999\n"));
    }

    #[test]
    fn the_pipe_carries_a_clients_output_into_the_bounded_file() {
        let d = dir("pipe");
        let p = d.join("x.log");
        let w = pipe(&p).unwrap();
        let mut f = File::from(w);
        f.write_all(b"hello\nworld").unwrap();
        drop(f);
        for _ in 0..100 {
            if fs::read_to_string(&p).map(|t| t == "hello\nworld\n").unwrap_or(false) {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("{:?}", fs::read_to_string(&p));
    }
}
