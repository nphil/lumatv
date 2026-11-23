//! The reply's sound: a player behind a trait, `aplay` on the TV.
//!
//! The reply arrives as 16-bit mono pieces at the rate the speech service made it at; the volume is the voice's own setting, applied
//! here to the samples (a square law, so that the lower half of the slider is not all of the range a living room uses). One reply
//! at a time: a new one replaces the one playing. Nothing is played unless the host asks, and the host asks only when the voice
//! may speak (the setting, and not in quiet hours).

use crate::input::stop;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use voice_wire::SpeechEnd;

/// The longest the player waits for the next piece of a reply before it plays what it has and calls the reply starved.
const STARVED: Duration = Duration::from_secs(10);

pub trait Speaker: Send {
    /// Begins a reply of mono 16-bit samples at `rate`, at `percent` of the volume. An error is why it will not be played.
    fn begin(&mut self, rate: u32, percent: u8) -> Result<(), String>;
    /// A piece of the reply (little-endian bytes; an odd last byte waits for the next piece).
    fn push(&mut self, pcm: &[u8]);
    /// The reply is all sent: it ends when what is held has played.
    fn finish(&mut self);
    /// Cuts the reply now.
    fn stop(&mut self);
    /// Whether a reply is still being played, and if it has just ended, how.
    fn poll(&mut self) -> Poll;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    Idle,
    Playing,
    Ended(SpeechEnd),
}

/// The loudness a percent of the voice's volume stands for, as a factor on the samples.
pub fn gain(percent: u8) -> f32 {
    let p = f32::from(percent.clamp(1, 100)) / 100.0;
    p * p
}

/// `aplay` (or any program that plays raw 16-bit mono PCM from its standard input) as the player.
pub struct Aplay {
    argv: Vec<String>,
    current: Option<Reply>,
}

struct Reply {
    tx: Option<Sender<Vec<u8>>>,
    ended: Arc<Mutex<Option<SpeechEnd>>>,
    cut: Arc<AtomicBool>,
    group: libc::pid_t,
    gain: f32,
    odd: Option<u8>,
    thread: Option<JoinHandle<()>>,
}

impl Aplay {
    /// `argv` is the program and the arguments before the rate: for `aplay`, `-q -t raw -f S16_LE -c 1`. `-r <rate>` is added.
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            argv,
            current: None,
        }
    }

    fn spawn(&self, rate: u32) -> Result<Child, String> {
        let (program, args) = self.argv.split_first().ok_or("there is no player configured")?;
        Command::new(program)
            .args(args)
            .arg("-r")
            .arg(rate.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("the player did not start: {e}"))
    }
}

impl Speaker for Aplay {
    fn begin(&mut self, rate: u32, percent: u8) -> Result<(), String> {
        self.stop();
        let mut child = self.spawn(rate)?;
        let Some(stdin) = child.stdin.take() else {
            stop(&mut child);
            return Err("the player has no input".to_owned());
        };
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let ended = Arc::new(Mutex::new(None));
        let cut = Arc::new(AtomicBool::new(false));
        let group = child.id() as libc::pid_t;
        let (thread_ended, thread_cut) = (ended.clone(), cut.clone());
        let thread = thread::Builder::new()
            .name("voice-play".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut stdin = Some(stdin);
                let mut starved = false;
                loop {
                    match rx.recv_timeout(STARVED) {
                        Ok(bytes) => {
                            if stdin.as_mut().is_none_or(|s| s.write_all(&bytes).is_err()) {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            starved = true;
                            break;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                // End of input: the player plays what it holds and exits.
                drop(stdin.take());
                let _ = child.wait();
                let end = if thread_cut.load(Ordering::Acquire) {
                    SpeechEnd::Cut
                } else if starved {
                    SpeechEnd::Starved
                } else {
                    SpeechEnd::Played
                };
                if let Ok(mut slot) = thread_ended.lock() {
                    *slot = Some(end);
                }
            })
            .map_err(|e| format!("the player did not start: {e}"))?;
        self.current = Some(Reply {
            tx: Some(tx),
            ended,
            cut,
            group,
            gain: gain(percent),
            odd: None,
            thread: Some(thread),
        });
        Ok(())
    }

    fn push(&mut self, pcm: &[u8]) {
        let Some(reply) = self.current.as_mut() else {
            return;
        };
        let mut bytes = Vec::with_capacity(pcm.len() + 1);
        bytes.extend(reply.odd.take());
        bytes.extend_from_slice(pcm);
        if bytes.len() % 2 == 1 {
            reply.odd = bytes.pop();
        }
        for pair in bytes.chunks_exact_mut(2) {
            let v = i16::from_le_bytes([pair[0], pair[1]]);
            let scaled = (f32::from(v) * reply.gain).round().clamp(-32_768.0, 32_767.0) as i16;
            pair.copy_from_slice(&scaled.to_le_bytes());
        }
        if let Some(tx) = reply.tx.as_ref() {
            let _ = tx.send(bytes);
        }
    }

    fn finish(&mut self) {
        if let Some(reply) = self.current.as_mut() {
            reply.tx = None;
        }
    }

    fn stop(&mut self) {
        let Some(mut reply) = self.current.take() else {
            return;
        };
        if reply.ended.lock().is_ok_and(|e| e.is_some()) {
            return;
        }
        reply.cut.store(true, Ordering::Release);
        reply.tx = None;
        // SAFETY: a signal to the group of a child this process started.
        unsafe { libc::killpg(reply.group, libc::SIGKILL) };
        if let Some(thread) = reply.thread.take() {
            let _ = thread.join();
        }
    }

    fn poll(&mut self) -> Poll {
        let Some(reply) = self.current.as_mut() else {
            return Poll::Idle;
        };
        let end = reply.ended.lock().ok().and_then(|mut e| e.take());
        match end {
            None => Poll::Playing,
            Some(end) => {
                if let Some(thread) = reply.thread.take() {
                    let _ = thread.join();
                }
                self.current = None;
                Poll::Ended(end)
            }
        }
    }
}

impl Drop for Aplay {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn player(file: &std::path::Path) -> Aplay {
        Aplay::new(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!("cat > {}", file.display()),
        ])
    }

    fn wait_end(p: &mut Aplay) -> SpeechEnd {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if let Poll::Ended(end) = p.poll() {
                return end;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("the reply did not end");
    }

    #[test]
    fn a_reply_is_scaled_by_the_square_of_the_volume_and_ends_played() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.pcm");
        let mut p = player(&file);
        p.begin(24_000, 50).unwrap();
        let samples: Vec<u8> = [16_000i16, -16_000, 1_000].iter().flat_map(|s| s.to_le_bytes()).collect();
        // an odd cut in the middle of a sample must not shift the rest
        p.push(&samples[..3]);
        p.push(&samples[3..]);
        p.finish();
        assert_eq!(wait_end(&mut p), SpeechEnd::Played);
        let out = std::fs::read(&file).unwrap();
        let got: Vec<i16> = out.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(got, vec![4_000, -4_000, 250], "50 % is a quarter");
        assert_eq!(p.poll(), Poll::Idle);
    }

    #[test]
    fn a_reply_that_is_stopped_ends_cut_and_the_player_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = player(&dir.path().join("out.pcm"));
        p.begin(16_000, 100).unwrap();
        p.push(&[0u8; 64]);
        p.stop();
        assert_eq!(p.poll(), Poll::Idle);
    }

    #[test]
    fn a_player_that_cannot_start_refuses_in_words() {
        let mut p = Aplay::new(vec!["/nonexistent/aplay".to_owned()]);
        assert!(p.begin(16_000, 50).unwrap_err().contains("did not start"));
        let mut none = Aplay::new(Vec::new());
        assert!(none.begin(16_000, 50).is_err());
    }

    #[test]
    fn the_volume_curve_is_a_square_and_never_silent() {
        assert_eq!(gain(100), 1.0);
        assert_eq!(gain(50), 0.25);
        assert_eq!(gain(0), 0.0001);
    }
}
