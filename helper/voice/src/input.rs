//! Where the sound comes from and goes to: the card behind a trait, so the engine is tested on recorded audio and a fake player,
//! and on the TV the device is `arecord` and `aplay`.

use crate::pcm::{Format, Sample};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// A running capture. Reads never block: [`Input::read`] returns 0 when nothing has arrived yet.
pub trait Input: Send {
    /// What `poll` waits on for data.
    fn fd(&self) -> RawFd;
    /// Takes what has arrived. `Ok(0)` is "nothing yet"; an error is a stream that has ended, in words.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

/// The sound card as the engine uses it.
pub trait Card: Send + Sync {
    /// Opens the microphone. The stream is in [`Card::mic_format`].
    fn open_mic(&self) -> io::Result<Box<dyn Input>>;
    fn mic_format(&self) -> Format;
    /// Opens the stream that carries what the TV plays, for its level. An error is "there is none"; the engine goes on without.
    fn open_reference(&self) -> io::Result<Box<dyn Input>>;
    fn reference_format(&self) -> Format;
}

/// `arecord` (or any program that writes raw PCM to its standard output) as the card.
pub struct Alsa {
    /// The program and its arguments for the microphone.
    pub mic: Vec<String>,
    pub mic_format: Format,
    /// The program for the TV's mix; empty: none.
    pub reference: Vec<String>,
    pub reference_format: Format,
}

impl Alsa {
    /// The arguments that open an ALSA capture device at one format, with the period pinned. A device that has a fixed period (the
    /// TV's microphone has 256 frames) refuses a buffer that is not a multiple of it, which is what `plughw` picks on its own.
    pub fn arecord(device: &str, format: Format, period: u32, buffer: u32) -> Vec<String> {
        let mut argv: Vec<String> = ["arecord", "-q", "-D", device, "-t", "raw", "-f"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        argv.push(format.sample.alsa().to_owned());
        argv.push("-r".to_owned());
        argv.push(format.rate.to_string());
        argv.push("-c".to_owned());
        argv.push(format.channels.to_string());
        if period > 0 {
            argv.push(format!("--period-size={period}"));
        }
        if buffer > 0 {
            argv.push(format!("--buffer-size={buffer}"));
        }
        argv
    }
}

impl Card for Alsa {
    fn open_mic(&self) -> io::Result<Box<dyn Input>> {
        Ok(Box::new(Program::start(&self.mic)?))
    }

    fn mic_format(&self) -> Format {
        self.mic_format
    }

    fn open_reference(&self) -> io::Result<Box<dyn Input>> {
        if self.reference.is_empty() {
            return Err(io::Error::other("no reference stream"));
        }
        Ok(Box::new(Program::start(&self.reference)?))
    }

    fn reference_format(&self) -> Format {
        self.reference_format
    }
}

/// A capture program whose standard output is read without blocking. It is the only holder of the device: it is stopped, and
/// waited for, when this is dropped, so that the device is free the moment the microphone is "off".
pub struct Program {
    child: Child,
    out: ChildStdout,
    err: ChildStderr,
}

impl Program {
    pub fn start(argv: &[String]) -> io::Result<Self> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::other("no capture program"))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group, so that stopping it stops whatever it started too.
            .process_group(0)
            .spawn()?;
        let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("the capture has no output"));
        };
        for fd in [out.as_raw_fd(), err.as_raw_fd()] {
            set_nonblocking(fd);
        }
        Ok(Self { child, out, err })
    }

    /// The first line the program wrote to its standard error: why it stopped, when it did.
    fn reason(&mut self) -> String {
        let mut text = [0u8; 400];
        let n = self.err.read(&mut text).unwrap_or(0);
        let text = String::from_utf8_lossy(&text[..n]);
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let status = self
            .child
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.to_string())
            .unwrap_or_default();
        match (line.is_empty(), status.is_empty()) {
            (true, true) => "the capture program stopped".to_owned(),
            (true, false) => format!("the capture program stopped ({status})"),
            (false, _) => line.to_owned(),
        }
    }
}

impl Input for Program {
    fn fd(&self) -> RawFd {
        self.out.as_raw_fd()
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.out.read(buf) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, self.reason())),
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(0),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// Ends a child and its group: asked to stop first (a capture closes its device cleanly), then killed if it has not within 400 ms.
pub(crate) fn stop(child: &mut Child) {
    let group = child.id() as libc::pid_t;
    // SAFETY: a signal to the group of a child this process started.
    unsafe { libc::killpg(group, libc::SIGTERM) };
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // SAFETY: as above.
    unsafe { libc::killpg(group, libc::SIGKILL) };
    let _ = child.wait();
}

pub(crate) fn set_nonblocking(fd: RawFd) {
    // SAFETY: flags of a descriptor this process owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// The format of the TV's microphone as the card offers it: two PDM microphones, 24-bit samples.
pub fn tv_mic_format() -> Format {
    Format {
        rate: 16_000,
        channels: 2,
        sample: Sample::S24,
    }
}
