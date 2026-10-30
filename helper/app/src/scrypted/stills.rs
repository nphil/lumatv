//! The stills: the grid's tiles, the pop-up's picture and the full view's picture over an app are JPEGs the bridge's host makes (one shared
//! ffmpeg a camera, not one decoder a tile; design 2.3), read here from `GET /v1/stills` and kept in memory.
//!
//! While the shell asks for a camera's picture the helper holds one connection to the bridge for it, keeps the newest picture, and answers
//! every ask from that picture (so a grid of tiles asking five times a second costs one stream a camera, not a request an ask). Three seconds
//! after the last ask the connection is closed and the bridge stops its ffmpeg. Nothing is asked of the bridge for a camera nobody looks at,
//! and no picture is ever written to a file: they are in memory until the link is forgotten, the TV sleeps or the camera is gone.

use super::bridge::Bridge;
use super::multipart::{PartReader, boundary_of};
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// How long after the last ask the connection stays.
const IDLE: Duration = Duration::from_secs(3);
/// How long the first picture of a stream may take (a stream that is not prebuffered waits for the camera's next key picture), and how long
/// a later one may.
const FIRST_WITHIN: Duration = Duration::from_secs(8);
const NEXT_WITHIN: Duration = Duration::from_millis(2500);
/// The bridge serves at most eight cameras at once.
const MAX_STREAMS: usize = 8;
/// The widths a stream is asked at: a camera has one at a time, the least that is at least the width the shell asked for.
const WIDTHS: [u32; 5] = [320, 480, 640, 960, 1280];

pub fn bucket(width: u32) -> u32 {
    WIDTHS.into_iter().find(|w| *w >= width).unwrap_or(1280)
}

/// One picture and when it came.
#[derive(Clone, Debug)]
pub struct Picture {
    pub jpeg: Arc<Vec<u8>>,
    /// The bridge's own stamp (`X-Captured-At`, epoch milliseconds), when it gave one.
    pub captured_at: Option<u64>,
    pub taken: Instant,
}

impl Picture {
    pub fn age(&self) -> Duration {
        self.taken.elapsed()
    }
}

struct Slot {
    latest: Mutex<Option<Picture>>,
    changed: Condvar,
    last_ask: Mutex<Instant>,
    width: AtomicU32,
    /// The most pictures a second anybody asked of this camera (0: the grid's pace): a camera pop-up that has no video asks for more than a tile does.
    fps: AtomicU32,
    running: AtomicBool,
}

struct Inner {
    /// A still older than this is no picture (the shell keeps its own last good one, dimmed), and the most a picture may be.
    fresh: Duration,
    max_picture: usize,
    bridge: Mutex<Option<Bridge>>,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    fps: AtomicU32,
    /// Raised when the bridge changes or goes: every reader of an earlier one ends.
    generation: AtomicU64,
    running: AtomicU32,
}

#[derive(Clone)]
pub struct Stills {
    inner: Arc<Inner>,
}

impl Stills {
    pub fn new(fresh: Duration, max_picture: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                fresh,
                max_picture,
                bridge: Mutex::new(None),
                slots: Mutex::new(HashMap::new()),
                fps: AtomicU32::new(5),
                generation: AtomicU64::new(0),
                running: AtomicU32::new(0),
            }),
        }
    }

    /// The bridge the pictures come from; `None` (the link went, or the TV sleeps) ends every stream and forgets every picture.
    pub fn set_bridge(&self, bridge: Option<Bridge>) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        *self.inner.bridge.lock() = bridge;
        self.clear();
    }

    /// Pictures a second a stream is asked at (the grid setting: 3, 5 or 8).
    pub fn set_fps(&self, fps: u32) {
        self.inner.fps.store(fps.clamp(1, 10), Ordering::Relaxed);
    }

    pub fn clear(&self) {
        self.inner.slots.lock().retain(|_, slot| {
            *slot.latest.lock() = None;
            slot.running.load(Ordering::SeqCst)
        });
    }

    /// Forgets the pictures of cameras that are no longer in `keep`.
    pub fn keep_only(&self, keep: &[String]) {
        self.inner.slots.lock().retain(|camera, slot| {
            let wanted = keep.iter().any(|k| k == camera);
            if !wanted {
                *slot.latest.lock() = None;
            }
            wanted || slot.running.load(Ordering::SeqCst)
        });
    }

    /// Streams open now.
    pub fn open_streams(&self) -> usize {
        self.inner.running.load(Ordering::SeqCst) as usize
    }

    /// The newest picture of a camera, whatever its age, without asking for one.
    pub fn latest(&self, camera: &str) -> Option<Picture> {
        let slot = self.inner.slots.lock().get(camera).cloned()?;
        slot.latest.lock().clone()
    }

    /// Asks for a camera's picture at about `width` pixels wide, `fps` a second at least where that is more than the grid's pace (0: the grid's): opens
    /// its stream if none is open, and answers the newest picture that is fresh, waiting up to `wait` for the first one. `Err(busy)` when eight other
    /// cameras are being served already.
    pub fn ask(&self, camera: &str, width: u32, fps: u32, wait: Duration) -> Result<Option<Picture>, Busy> {
        let slot = {
            let mut slots = self.inner.slots.lock();
            slots
                .entry(camera.to_owned())
                .or_insert_with(|| {
                    Arc::new(Slot {
                        latest: Mutex::new(None),
                        changed: Condvar::new(),
                        last_ask: Mutex::new(Instant::now()),
                        width: AtomicU32::new(0),
                        fps: AtomicU32::new(0),
                        running: AtomicBool::new(false),
                    })
                })
                .clone()
        };
        *slot.last_ask.lock() = Instant::now();
        slot.width.fetch_max(bucket(width), Ordering::SeqCst);
        slot.fps.fetch_max(fps.min(10), Ordering::SeqCst);
        if !slot.running.swap(true, Ordering::SeqCst) {
            if self.inner.running.load(Ordering::SeqCst) as usize >= MAX_STREAMS {
                slot.running.store(false, Ordering::SeqCst);
                return Err(Busy);
            }
            self.inner.running.fetch_add(1, Ordering::SeqCst);
            let (inner, name, s) = (self.inner.clone(), camera.to_owned(), slot.clone());
            let generation = self.inner.generation.load(Ordering::SeqCst);
            let spawned = thread::Builder::new()
                .name("scrypted-stills".to_owned())
                // the TLS connection (rustls's state is a few kilobytes and a debug build copies it) lives on this stack
                .stack_size(super::http::TLS_STACK)
                .spawn(move || {
                    read_stills(&inner, &name, &s, generation);
                    s.running.store(false, Ordering::SeqCst);
                    s.width.store(0, Ordering::SeqCst);
                    s.fps.store(0, Ordering::SeqCst);
                    inner.running.fetch_sub(1, Ordering::SeqCst);
                });
            if spawned.is_err() {
                slot.running.store(false, Ordering::SeqCst);
                self.inner.running.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let deadline = Instant::now() + wait;
        let mut latest = slot.latest.lock();
        loop {
            if let Some(picture) = latest.as_ref()
                && picture.age() < self.inner.fresh
            {
                return Ok(Some(picture.clone()));
            }
            if wait.is_zero() || slot.changed.wait_until(&mut latest, deadline).timed_out() {
                return Ok(latest.clone().filter(|p| p.age() < self.inner.fresh));
            }
        }
    }
}

/// Eight cameras' streams are open: the ninth waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

fn idle(slot: &Slot) -> bool {
    slot.last_ask.lock().elapsed() >= IDLE
}

/// One camera's reader: opens the stream, keeps the newest picture, and ends when nobody has asked for three seconds.
fn read_stills(inner: &Arc<Inner>, camera: &str, slot: &Arc<Slot>, generation: u64) {
    let current = || inner.generation.load(Ordering::SeqCst) == generation;
    let pause = |wait: Duration| {
        let end = Instant::now() + wait;
        while Instant::now() < end && !idle(slot) && current() {
            thread::sleep(Duration::from_millis(50));
        }
    };
    while current() && !idle(slot) {
        let Some(bridge) = inner.bridge.lock().clone() else {
            return;
        };
        let width = slot.width.load(Ordering::SeqCst).max(WIDTHS[0]);
        let fps = inner.fps.load(Ordering::Relaxed).max(slot.fps.load(Ordering::SeqCst)).clamp(1, 10);
        let opened = bridge.open_stills(camera, width, fps);
        let (head, body) = match opened {
            Ok(opened) => opened,
            // too many streams at the bridge, or the camera is off: asked again in a moment, while somebody wants it
            Err(trouble) => {
                pause(if trouble.code() == "busy" {
                    Duration::from_millis(1500)
                } else {
                    Duration::from_secs(1)
                });
                continue;
            }
        };
        let boundary = head
            .header("content-type")
            .and_then(boundary_of)
            .unwrap_or_else(|| "lumashow".to_owned());
        let socket = body.socket().try_clone().ok();
        let set_wait = |within: Duration| {
            if let Some(socket) = &socket {
                let _ = socket.set_read_timeout(Some(within));
            }
        };
        let mut parts = PartReader::new(BufReader::new(body), &boundary, inner.max_picture);
        let mut within = FIRST_WITHIN;
        set_wait(within);
        loop {
            match parts.next_part() {
                Ok(Some(part)) if !part.body.is_empty() => {
                    let picture = Picture {
                        jpeg: Arc::new(part.body),
                        captured_at: part.captured_at,
                        taken: Instant::now(),
                    };
                    *slot.latest.lock() = Some(picture);
                    slot.changed.notify_all();
                    if within != NEXT_WITHIN {
                        within = NEXT_WITHIN;
                        set_wait(within);
                    }
                }
                Ok(Some(_)) => {}
                // the stream ended or failed: opened again below if somebody still wants the camera
                Ok(None) | Err(_) => break,
            }
            // a wider picture, or a faster pace, was asked for: the stream is opened again at that
            if !current() || idle(slot) || slot.width.load(Ordering::SeqCst) > width || slot.fps.load(Ordering::SeqCst) > fps {
                break;
            }
        }
        drop(parts);
        if !current() || idle(slot) {
            return;
        }
        pause(Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_asked_at_the_least_width_that_covers_the_ask() {
        assert_eq!(
            [
                bucket(1),
                bucket(320),
                bucket(321),
                bucket(480),
                bucket(560),
                bucket(1000),
                bucket(1280),
                bucket(4000)
            ],
            [320, 320, 480, 480, 640, 1280, 1280, 1280]
        );
    }

    #[test]
    fn with_no_bridge_an_ask_is_nothing_and_leaves_no_stream_open() {
        let stills = Stills::new(Duration::from_secs(10), 500 * 1024);
        assert!(matches!(
            stills.ask("273", 480, 0, Duration::from_millis(30)),
            Ok(None)
        ));
        // the reader had no bridge to ask and ended
        let end = Instant::now() + Duration::from_secs(2);
        while stills.open_streams() > 0 && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(stills.open_streams(), 0);
        assert!(stills.latest("273").is_none());
        stills.set_bridge(None);
        stills.keep_only(&[]);
    }
}
