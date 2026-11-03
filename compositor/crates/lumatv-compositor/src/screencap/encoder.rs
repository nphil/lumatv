//! The encoder a recording feeds: NV12 frames in, H.264 access units out, behind [`Encoder`] so the pipeline is tested against a fake.
//!
//! The real one is the C4's hardware encoder reached through `lumatv-venc feed` (`venc/`, `docs/research/venc-result.md`), a child
//! process of this one rather than a library in it: the encoder shares the video MCU with the decoder, and a call that never returns
//! (never observed; the note's first risk) must cost this process a killed child, not its frame loop. The child has `/dev/venc` and
//! `/dev/hma` and nothing else of the compositor's.
//!
//! The wire is the child's stdin and stdout, one frame in flight at a time (`venc/feed_proto.h` is the other side of it; a test here
//! reads that header and checks the numbers). Both directions carry a 24-byte record header, little-endian:
//!
//! | Direction | Header | Then |
//! |---|---|---|
//! | child to compositor, once the encoder is open | `"LRDY"`, version, width, height, reserved, reserved | nothing |
//! | compositor to child | `"LFRM"`, flags (bit 0: make this frame an IDR), pts us (u64), size, reserved | `size` bytes of NV12 |
//! | child to compositor | `"LAUN"`, flags (bit 0: keyframe), pts us (u64, the frame's), size, encoder ms | `size` bytes of Annex B (0: none) |
//!
//! Every wait on the child has a deadline; a child that misses one is killed and the recording ends with that said.

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

pub const MAGIC_FRAME: &[u8; 4] = b"LFRM";
pub const MAGIC_UNIT: &[u8; 4] = b"LAUN";
pub const MAGIC_READY: &[u8; 4] = b"LRDY";
pub const HEADER_LEN: usize = 24;
pub const PROTO_VERSION: u32 = 1;
pub const FLAG_KEY: u32 = 1;

/// How long the child has to open the encoder and say so.
const OPEN_DEADLINE: Duration = Duration::from_secs(5);
/// How long one frame may take from the first byte written to the access unit read back (the hardware needs 10-25 ms at 1080p).
const FRAME_DEADLINE: Duration = Duration::from_secs(3);
/// How long a closing child has to finish before it is killed.
const CLOSE_DEADLINE: Duration = Duration::from_secs(3);
/// The largest access unit believed (a 1080p keyframe at the encoder's worst is a few hundred KB).
const MAX_UNIT: u32 = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    /// Frames between keyframes.
    pub gop: u32,
}

impl EncoderSpec {
    pub fn frame_len(&self) -> usize {
        super::nv12::nv12_len(self.width as usize, self.height as usize)
    }
}

/// One access unit out of the encoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub data: Vec<u8>,
    pub key: bool,
    pub pts_us: u64,
}

pub trait Encoder: Send {
    /// Encodes one NV12 frame (`spec.frame_len()` bytes) taken at `pts_us`; `key` asks for a keyframe. `None` when the encoder made
    /// nothing of it.
    fn encode(&mut self, nv12: &[u8], pts_us: u64, key: bool) -> io::Result<Option<Unit>>;

    /// Ends the session and gives the encoder back; the line it had to say about the run (for the log).
    fn close(self: Box<Self>) -> String;
}

pub trait EncoderFactory: Send + Sync {
    /// Opens an encoder for `spec`. Runs on the recording's own thread: it may take a moment (a process starts, a device opens).
    fn open(&self, spec: &EncoderSpec) -> io::Result<Box<dyn Encoder>>;

    /// What this factory opens, for `status`.
    fn describe(&self) -> String;

    /// Whether it could open at all (the binary is there): `status` says so before anyone presses record.
    fn available(&self) -> bool;
}

/// The hardware encoder through `lumatv-venc feed`.
pub struct VencFactory {
    pub binary: PathBuf,
}

impl EncoderFactory for VencFactory {
    fn open(&self, spec: &EncoderSpec) -> io::Result<Box<dyn Encoder>> {
        let log = crate::childlog::pipe("lumatv-compositor", "venc: ");
        let mut child = Command::new(&self.binary)
            .args(["feed", "-w", &spec.width.to_string(), "-h", &spec.height.to_string(), "-r", &spec.fps.to_string()])
            .args(["-b", &spec.kbps.to_string(), "-g", &spec.gop.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log.map_or_else(Stdio::null, Stdio::from))
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", self.binary.display())))?;
        let (stdin, stdout) = (child.stdin.take().expect("piped"), child.stdout.take().expect("piped"));
        if let Err(e) = set_nonblocking(stdin.as_raw_fd()).and_then(|()| set_nonblocking(stdout.as_raw_fd())) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let mut enc = VencChild { child, stdin: Some(stdin), stdout, spec: *spec, frames: 0 };
        let mut hello = [0u8; HEADER_LEN];
        if let Err(e) = read_exact_by(enc.stdout.as_raw_fd(), &mut hello, Instant::now() + OPEN_DEADLINE) {
            let why = enc.why_it_ended(&e);
            enc.kill();
            return Err(io::Error::new(e.kind(), why));
        }
        let (version, w, h) = (u32_at(&hello, 4), u32_at(&hello, 8), u32_at(&hello, 12));
        if &hello[..4] != MAGIC_READY || version != PROTO_VERSION || (w, h) != (spec.width, spec.height) {
            enc.kill();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the encoder said {:?} v{version} {w}x{h} where {}x{} was asked", &hello[..4], spec.width, spec.height),
            ));
        }
        Ok(Box::new(enc))
    }

    fn describe(&self) -> String {
        self.binary.display().to_string()
    }

    fn available(&self) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&self.binary).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
}

struct VencChild {
    child: Child,
    /// Taken (and so closed) when the session ends: closing its input is how the child is told to finish.
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    spec: EncoderSpec,
    frames: u64,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor this process owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Waits for `fd` to be ready for `events` until `deadline`.
fn wait_fd(fd: RawFd, events: i16, deadline: Instant) -> io::Result<()> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the encoder did not answer in time"));
        }
        let mut p = libc::pollfd { fd, events, revents: 0 };
        // SAFETY: one pollfd on a descriptor this process owns.
        let n = unsafe { libc::poll(&mut p, 1, left.as_millis().clamp(1, i32::MAX as u128) as i32) };
        if n > 0 {
            return Ok(());
        }
        if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(io::Error::last_os_error());
        }
    }
}

fn read_exact_by(fd: RawFd, buf: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut done = 0;
    while done < buf.len() {
        wait_fd(fd, libc::POLLIN, deadline)?;
        // SAFETY: reading into the unfilled part of a slice this function was handed.
        let n = unsafe { libc::read(fd, buf[done..].as_mut_ptr() as *mut libc::c_void, buf.len() - done) };
        match n {
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the encoder closed its output")),
            n if n > 0 => done += n as usize,
            _ => {
                let e = io::Error::last_os_error();
                if !matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

fn write_all_by(fd: RawFd, mut buf: &[u8], deadline: Instant) -> io::Result<()> {
    while !buf.is_empty() {
        wait_fd(fd, libc::POLLOUT, deadline)?;
        // SAFETY: writing from a slice this function was handed.
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        match n {
            n if n > 0 => buf = &buf[n as usize..],
            _ => {
                let e = io::Error::last_os_error();
                if n == 0 || !matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
                    return Err(if n == 0 { io::Error::from(io::ErrorKind::WriteZero) } else { e });
                }
            }
        }
    }
    Ok(())
}

fn encoder_tail() -> Option<String> {
    crate::logsink::tail("lumatv-compositor", usize::MAX).into_iter().rev().find_map(|line| line.strip_prefix("venc: ").map(str::to_owned))
}

impl VencChild {
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// The encoder's latest retained diagnostic, if the collector has received it.
    fn why_it_ended(&self, e: &io::Error) -> String {
        let tail = encoder_tail();
        match tail {
            Some(t) => format!("{e}: {t}"),
            None => e.to_string(),
        }
    }
}

impl Encoder for VencChild {
    fn encode(&mut self, nv12: &[u8], pts_us: u64, key: bool) -> io::Result<Option<Unit>> {
        debug_assert_eq!(nv12.len(), self.spec.frame_len());
        let deadline = Instant::now() + FRAME_DEADLINE;
        let mut head = [0u8; HEADER_LEN];
        head[..4].copy_from_slice(MAGIC_FRAME);
        head[4..8].copy_from_slice(&u32::from(key).to_le_bytes());
        head[8..16].copy_from_slice(&pts_us.to_le_bytes());
        head[16..20].copy_from_slice(&(nv12.len() as u32).to_le_bytes());
        let Some(stdin) = &self.stdin else { return Err(io::Error::from(io::ErrorKind::BrokenPipe)) };
        let sent = write_all_by(stdin.as_raw_fd(), &head, deadline).and_then(|()| write_all_by(stdin.as_raw_fd(), nv12, deadline));
        let mut unit = [0u8; HEADER_LEN];
        let got = sent.and_then(|()| read_exact_by(self.stdout.as_raw_fd(), &mut unit, deadline));
        if let Err(e) = got {
            let why = self.why_it_ended(&e);
            self.kill();
            return Err(io::Error::new(e.kind(), why));
        }
        let size = u32_at(&unit, 16);
        if &unit[..4] != MAGIC_UNIT || size > MAX_UNIT {
            self.kill();
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("the encoder answered {:?} with {size} bytes", &unit[..4])));
        }
        self.frames += 1;
        if size == 0 {
            return Ok(None);
        }
        let mut data = vec![0u8; size as usize];
        if let Err(e) = read_exact_by(self.stdout.as_raw_fd(), &mut data, deadline) {
            let why = self.why_it_ended(&e);
            self.kill();
            return Err(io::Error::new(e.kind(), why));
        }
        Ok(Some(Unit { data, key: u32_at(&unit, 4) & FLAG_KEY != 0, pts_us: u64::from_le_bytes(unit[8..16].try_into().expect("eight bytes")) }))
    }

    fn close(mut self: Box<Self>) -> String {
        // The child finishes when its input ends: it closes the hardware and says how the run went.
        drop(self.stdin.take());
        let deadline = Instant::now() + CLOSE_DEADLINE;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    let said = encoder_tail();
                    return format!("encoder exit {status} after {} frames{}", self.frames, said.map(|s| format!(": {s}")).unwrap_or_default());
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    self.kill();
                    return format!("encoder did not finish in time and was killed after {} frames", self.frames);
                }
            }
        }
    }
}

#[cfg(test)]
pub mod fake {
    //! An encoder that needs no hardware: a keyframe (SPS, PPS, IDR) every `gop` frames and a P slice in between, each carrying the
    //! frame's mean luma and its number, so a test can say what the encoder was given.

    use super::*;
    use parking_lot::Mutex;
    use std::sync::Arc;

    #[derive(Default, Debug)]
    pub struct Seen {
        pub frames: Vec<(u64, u8, bool)>,
        pub closed: bool,
        pub opened: Option<EncoderSpec>,
    }

    pub struct FakeFactory {
        pub seen: Arc<Mutex<Seen>>,
        /// Frame number after which the encoder fails (None: never).
        pub fail_after: Option<u64>,
        /// A delay per frame, to make the encoder slower than the producer.
        pub delay: Duration,
        pub open_error: Option<String>,
        pub unit_bytes: usize,
    }

    impl Default for FakeFactory {
        fn default() -> Self {
            FakeFactory { seen: Arc::default(), fail_after: None, delay: Duration::ZERO, open_error: None, unit_bytes: 200 }
        }
    }

    impl FakeFactory {
        pub fn new() -> FakeFactory {
            FakeFactory::default()
        }
    }

    struct Fake {
        seen: Arc<Mutex<Seen>>,
        spec: EncoderSpec,
        n: u64,
        fail_after: Option<u64>,
        delay: Duration,
        unit_bytes: usize,
    }

    impl EncoderFactory for FakeFactory {
        fn open(&self, spec: &EncoderSpec) -> io::Result<Box<dyn Encoder>> {
            if let Some(e) = &self.open_error {
                return Err(io::Error::other(e.clone()));
            }
            self.seen.lock().opened = Some(*spec);
            Ok(Box::new(Fake { seen: self.seen.clone(), spec: *spec, n: 0, fail_after: self.fail_after, delay: self.delay, unit_bytes: self.unit_bytes }))
        }

        fn describe(&self) -> String {
            "fake".into()
        }

        fn available(&self) -> bool {
            self.open_error.is_none()
        }
    }

    impl Encoder for Fake {
        fn encode(&mut self, nv12: &[u8], pts_us: u64, key: bool) -> io::Result<Option<Unit>> {
            assert_eq!(nv12.len(), self.spec.frame_len(), "the pipeline gives whole frames");
            std::thread::sleep(self.delay);
            if self.fail_after.is_some_and(|n| self.n >= n) {
                return Err(io::Error::other("the fake encoder failed"));
            }
            let luma = &nv12[..(self.spec.width * self.spec.height) as usize];
            let mean = (luma.iter().map(|&v| v as u64).sum::<u64>() / luma.len() as u64) as u8;
            let is_key = key || self.n % self.spec.gop as u64 == 0;
            self.seen.lock().frames.push((pts_us, mean, is_key));
            let mut data = Vec::new();
            if is_key {
                data.extend_from_slice(&[0, 0, 0, 1, 0x67, 100, 0, 31, 0xAC, 0xD9, 0, 0, 0, 1, 0x68, 0xEB, 0xE3, 0xCB]);
            }
            data.extend_from_slice(&[0, 0, 1, if is_key { 0x65 } else { 0x41 }, mean, (self.n & 0xff) as u8]);
            data.extend((0..self.unit_bytes).map(|i| (i % 200 + 1) as u8));
            self.n += 1;
            Ok(Some(Unit { data, key: is_key, pts_us }))
        }

        fn close(self: Box<Self>) -> String {
            self.seen.lock().closed = true;
            format!("fake encoder closed after {} frames", self.n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_numbers_are_the_ones_the_c_side_declares() {
        let header = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../venc/feed_proto.h")).expect("venc/feed_proto.h");
        let define = |name: &str| -> String {
            header
                .lines()
                .filter_map(|l| {
                    let mut words = l.split_whitespace();
                    (words.next() == Some("#define") && words.next() == Some(name)).then(|| words.next().unwrap_or("").to_string())
                })
                .next()
                .unwrap_or_else(|| panic!("{name} in feed_proto.h"))
        };
        assert_eq!(define("FEED_MAGIC_FRAME"), format!("\"{}\"", String::from_utf8_lossy(MAGIC_FRAME)));
        assert_eq!(define("FEED_MAGIC_UNIT"), format!("\"{}\"", String::from_utf8_lossy(MAGIC_UNIT)));
        assert_eq!(define("FEED_MAGIC_READY"), format!("\"{}\"", String::from_utf8_lossy(MAGIC_READY)));
        assert_eq!(define("FEED_HEADER_LEN"), HEADER_LEN.to_string());
        assert_eq!(define("FEED_VERSION"), PROTO_VERSION.to_string());
        assert_eq!(define("FEED_FLAG_KEY"), FLAG_KEY.to_string());
    }

    #[test]
    fn a_child_that_is_not_there_is_an_error_that_names_it() {
        let f = VencFactory {
            binary: PathBuf::from("/definitely/not/lumatv-venc"),
        };
        assert!(!f.available());
        let e = f.open(&EncoderSpec { width: 64, height: 64, fps: 30, kbps: 500, gop: 30 }).err().expect("an error");
        assert!(e.to_string().contains("/definitely/not/lumatv-venc"), "{e}");
    }
}
