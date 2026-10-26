//! Sound-reactive ambient light, the capture: the program that writes the TV's sound mix to its standard output, and the thread
//! that feeds the analyser from it.
//!
//! The program is `ambient.sound.capture` in `helper.json`: by default `arecord` on card 1 `LGSIC`, device 2 `MixerCapture`
//! (`hw:1,2`), 48 kHz, 16-bit, two channels, raw. That device carries the mix before the TV's volume and before the receiver's
//! mute, so the lights follow the sound whether or not it is audible, and exact silence when nothing plays (hardware census,
//! `ambient-result.md`). Its hardware period is 512 frames at most, so a read comes about every 10 ms. It is the owner's to change
//! (an empty list switches sound-reactive light off), and it is started only while the settings, the screen and the lights want
//! it: the PCM stream is read, summed and dropped; nothing is saved, and the child's output goes to a pipe nobody else holds.

use crate::ambient_audio::{Analyser, Features, Meter};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

/// What one read takes: 21 ms of the stream.
const READ: usize = 4096;

pub struct SoundCapture {
    child: Child,
    reader: Option<JoinHandle<()>>,
    meter: Arc<Meter>,
    /// The reading thread has seen the program's output end (set before it says so).
    done: Arc<AtomicBool>,
    /// Why it ended, once it has.
    ended: Option<String>,
}

impl SoundCapture {
    /// Starts `argv` (a program and its arguments) and the thread reading it. `wake` is called from that thread each time the
    /// sound starts after a silence, for whoever sleeps until then, and when the program's output ends.
    pub fn start(
        argv: &[String],
        meter: Arc<Meter>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("no sound capture program"))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group, so stopping it stops whatever it started too.
            .process_group(0)
            .spawn()?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("the sound capture has no output"));
        };
        let thread_meter = meter.clone();
        let done = Arc::new(AtomicBool::new(false));
        let thread_done = done.clone();
        let reader = thread::Builder::new()
            .name("ambient-sound".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut analyser = Analyser::new();
                let mut buf = [0u8; READ];
                let mut was = 0.0f32;
                loop {
                    let n = match out.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let mut latest = None;
                    let mut started = false;
                    analyser.push(&buf[..n], |f: &Features| {
                        started |= was < 0.5 && f.active >= 0.5;
                        was = f.active;
                        latest = Some(*f);
                    });
                    if let Some(f) = latest {
                        thread_meter.put(f, Instant::now());
                    }
                    if started {
                        wake();
                    }
                }
                // The program has ended: whoever tends the capture is told, to see why.
                thread_done.store(true, Ordering::Release);
                wake();
            });
        match reader {
            Ok(reader) => Ok(Self {
                child,
                reader: Some(reader),
                meter,
                done,
                ended: None,
            }),
            Err(e) => {
                kill(&mut child);
                Err(e)
            }
        }
    }

    /// Why the capture has ended, once it has (the program exited or its output closed); `None` while it runs.
    pub fn ended(&mut self) -> Option<String> {
        if self.ended.is_some() {
            return self.ended.clone();
        }
        if !self.done.load(Ordering::Acquire) {
            return None;
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        // Closed output: the program is gone or about to be (if it is not, it is stopped); what it said on its way is the reason.
        kill(&mut self.child);
        let status = self.child.wait().ok();
        let mut said = String::new();
        if let Some(err) = self.child.stderr.as_mut() {
            let _ = err.take(400).read_to_string(&mut said);
        }
        let said: String = said
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        let how = status.map_or_else(|| "ended".to_owned(), |s| s.to_string());
        let why = if said.is_empty() {
            format!("the sound capture stopped ({how})")
        } else {
            format!("the sound capture stopped ({how}): {said}")
        };
        self.ended = Some(why.clone());
        Some(why)
    }

    fn stop(&mut self) {
        // An ended program is already collected; its group is not signalled (its number may be someone else's by now).
        if self.ended.is_none() {
            kill(&mut self.child);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.meter.clear();
    }
}

impl Drop for SoundCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Kills the program and everything it started, and collects it.
fn kill(child: &mut Child) {
    // SAFETY: a signal to the process group this child leads; the pid is the child's own, which is not yet collected.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::time::Duration;

    /// A second of a 440 Hz tone, then a second of silence, in the capture's format.
    fn tone_then_silence(dir: &std::path::Path) -> std::path::PathBuf {
        let mut bytes = Vec::new();
        for i in 0..48_000 {
            let v = ((std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 8000.0) as i16;
            for _ in 0..2 {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        bytes.extend(vec![0u8; 48_000 * 4]);
        let path = dir.join("mix.pcm");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_programs_pcm_becomes_features_and_start_and_end_are_announced() {
        let dir = tempfile::tempdir().unwrap();
        let pcm = tone_then_silence(dir.path());
        let meter = Arc::new(Meter::new());
        let starts = Arc::new(AtomicU32::new(0));
        let counted = starts.clone();
        // `cat` writes the file at once and ends; a real capture writes at the rate of the sound and never ends.
        let argv = vec!["cat".to_owned(), pcm.display().to_string()];
        let mut capture = SoundCapture::start(&argv, meter.clone(), move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        wait_for("the program to end", || capture.ended().is_some());
        let why = capture.ended().unwrap();
        assert!(why.starts_with("the sound capture stopped"), "{why}");
        // Once when the tone began, once when the output ended.
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        // The last hops were silence: the meter has them (not yet stale, by the clock this reads with).
        let f = meter.get(Instant::now());
        assert!(f.level < 0.01, "{f:?}");
        assert!(f.active > 0.0 && f.active < 1.0, "{f:?}");
    }

    #[test]
    fn a_program_that_cannot_run_is_an_error_and_one_that_fails_says_why() {
        let meter = Arc::new(Meter::new());
        assert!(SoundCapture::start(&[], meter.clone(), || {}).is_err());
        let missing = vec!["/nonexistent/arecord".to_owned()];
        assert!(SoundCapture::start(&missing, meter.clone(), || {}).is_err());
        let argv: Vec<String> = [
            "sh",
            "-c",
            "echo 'arecord: audio open error: Device busy' >&2; exit 1",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let mut capture = SoundCapture::start(&argv, meter, || {}).unwrap();
        wait_for("the program to end", || capture.ended().is_some());
        let why = capture.ended().unwrap();
        assert!(
            why.contains("exit status: 1") && why.contains("Device busy"),
            "{why}"
        );
    }

    #[test]
    fn stopping_kills_the_program_and_its_children_and_clears_the_meter() {
        let meter = Arc::new(Meter::new());
        meter.put(
            Features {
                level: 1.0,
                ..Features::default()
            },
            Instant::now(),
        );
        // A shell that starts a child of its own and never writes.
        let argv: Vec<String> = ["sh", "-c", "sleep 300 & sleep 300"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let capture = SoundCapture::start(&argv, meter.clone(), || {}).unwrap();
        let pid = capture.child.id();
        let started = Instant::now();
        drop(capture);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(meter.get(Instant::now()), Features::SILENT);
        // The group is gone: nothing answers a signal sent to it.
        // SAFETY: signal 0 only checks whether the group exists.
        let alive = unsafe { libc::kill(-(pid as libc::pid_t), 0) } == 0;
        assert!(!alive);
    }

    /// The CPU the reading thread uses for a stream at the speed of sound, in this process (`/proc/self/task`).
    /// `LUMATV_SOUND_BENCH=<shell command that writes 48 kHz S16 stereo PCM at real time>`; the program's own cost is not counted
    /// (on the TV it is `arecord`'s, measured there with `top -H`).
    /// `LUMATV_SOUND_BENCH=... cargo test --release -p lumatv-helper -- --ignored pipeline_cost --nocapture`
    #[test]
    #[ignore = "a measurement, not a check"]
    fn pipeline_cost() {
        let Ok(command) = std::env::var("LUMATV_SOUND_BENCH") else {
            eprintln!("skipped: LUMATV_SOUND_BENCH is not set");
            return;
        };
        let seconds = 20;
        // Nanoseconds on a CPU, from the scheduler: the first field of a thread's `schedstat`.
        let thread_ns = || -> u64 {
            let mut ns = 0;
            for entry in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
                let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
                if comm.trim() == "ambient-sound" {
                    let stat = std::fs::read_to_string(entry.path().join("schedstat")).unwrap();
                    ns += stat
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap();
                }
            }
            ns
        };
        let meter = Arc::new(Meter::new());
        let argv = vec!["sh".to_owned(), "-c".to_owned(), command];
        let mut capture = SoundCapture::start(&argv, meter.clone(), || {}).unwrap();
        thread::sleep(Duration::from_millis(500));
        let (before, started) = (thread_ns(), Instant::now());
        thread::sleep(Duration::from_secs(seconds));
        let (after, took) = (thread_ns(), started.elapsed());
        assert!(capture.ended().is_none(), "the program ended");
        let cpu = (after - before) as f64 / 1e9;
        eprintln!(
            "reading thread: {cpu:.3} s of CPU in {:.1} s = {:.3} % of a core; last features {:?}",
            took.as_secs_f64(),
            cpu / took.as_secs_f64() * 100.0,
            meter.get(Instant::now())
        );
    }
}
