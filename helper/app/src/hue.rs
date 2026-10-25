//! The Hue Entertainment output of ambient light: one thread that owns the stream to the bridge.
//!
//! ```text
//!   ambient (30 Hz)  --set(demand, frame)-->  this thread  --HTTPS-->  bridge   start / stop the area
//!                                                          --DTLS-->   bridge   UDP 2100, HueStream v2
//! ```
//!
//! The bridge accepts a stream only for an entertainment area that was started over HTTPS, and the HTTPS calls and the DTLS
//! handshake take seconds, so none of it may happen on the thread that drives the strip: this thread takes what ambient light wants
//! ([`Demand`]) and the latest frame, and does the rest.
//!
//! - **Off**: nothing is open. Going to Off ends a stream that was running: the lights are sent black twice (the datagrams may be
//!   lost, and a lamp left lit at its last colour would stay that way), the DTLS session is closed with its alert, and the area is
//!   stopped over HTTPS, which hands the lights back to the bridge. This is what happens when the screen goes off.
//! - **Discover**: the area is read (its channels and where each sits) so the engine can have a light for each, and nothing is
//!   started. Asked for while ambient light is on and the lights are not known yet.
//! - **Stream**: read the area, start it, open DTLS with the application id as identity and the client key, then send the latest
//!   frame at most `fps` times a second (the link's rate, 20 by default: Bifrost's own `streaming_fps` default, which it cannot tell
//!   a client; it drops what comes faster and smooths for the rate it sees) and again at least once a second so the bridge, which
//!   ends a stream after about 10 s of silence, never does. The bridge ending the session (another application took the area, or
//!   it was stopped from the Hue app) is noticed within 250 ms and the stream is started again after a pause.
//!
//! An area that is streaming for another application is never taken over: the thread waits and looks again. One that is streaming
//! under this application's own id is this helper's own earlier stream (it was killed before it could stop the area, or the bridge
//! never cleared it: Bifrost leaves an area active when a stream ends): it is stopped and started afresh. A failure waits 5 s, then
//! 10, up to 60, and the words of it are the output's [`Status`], which ambient light shows as the `problem`.
//!
//! The keys are only used here, to speak to the address they were saved with; nothing here logs, prints or returns them.

use crate::hue_rest::{Area, Bridge, HueError, Tls};
use crate::hue_store::{self, HueLink};
use lumatv_ambient::Light;
use lumatv_ambient::huestream::{self, Session, SessionConfig};
use parking_lot::{Condvar, Mutex};
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// One update for the entertainment channels: `(channel id, [r, g, b])` as 16-bit values.
pub type Frame = Vec<(u8, [u16; 3])>;

/// A frame is sent again after this long without a new one (the bridge ends a stream after about 10 s of silence).
const KEEPALIVE: Duration = Duration::from_secs(1);
/// How often a stream is asked whether the bridge has closed it.
const POLL: Duration = Duration::from_millis(250);
/// The pause after a failure, which doubles up to the longest.
const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_LONGEST: Duration = Duration::from_secs(60);
/// A stop on the way out must not hold the helper up.
const STOP_WITHIN: Duration = Duration::from_secs(4);

/// What ambient light wants of the bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demand {
    Off,
    Discover,
    Stream,
}

/// What the output is doing, and the words of what went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// `idle`, `connecting`, `streaming` or `error`.
    pub state: &'static str,
    pub problem: Option<String>,
}

/// What the output tells ambient light, from its own thread and with none of its locks held.
pub enum Event {
    /// The lights the area has, where they sit (all of them, every time they change).
    Lights(Vec<Light>),
    /// The status changed.
    Status,
}

struct Slot {
    demand: Demand,
    frame: Option<Frame>,
    fresh: bool,
    stop: bool,
    status: Status,
}

struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}

/// The thread, and the way to talk to it. Dropping it ends the stream (see [`Output::shutdown`]).
pub struct Output {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Output {
    /// Starts the thread for `link` (which must name an area). It does nothing until it is given a demand.
    pub fn spawn(
        link: HueLink,
        data_dir: &Path,
        tls: Tls,
        events: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> io::Result<Self> {
        let area = link
            .area
            .clone()
            .ok_or_else(|| io::Error::other("the link names no entertainment area"))?;
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                demand: Demand::Off,
                frame: None,
                fresh: false,
                stop: false,
                status: Status {
                    state: "idle",
                    problem: None,
                },
            }),
            wake: Condvar::new(),
        });
        let bridge = Bridge::new(
            tls,
            SocketAddr::from((link.host, link.port)),
            hue_store::certificate_file(data_dir),
        );
        let worker = Worker {
            shared: shared.clone(),
            events,
            bridge,
            link,
            area,
            app_id: None,
            lights: None,
            channels: Vec::new(),
            live: None,
            last_frame: None,
            seen: Demand::Off,
            retry_at: None,
            backoff: RETRY_FIRST,
            last_problem: String::new(),
            pending: false,
        };
        let thread = thread::Builder::new()
            .name("hue".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || worker.run())?;
        Ok(Self {
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// What is wanted now, and the latest frame (only meaningful for [`Demand::Stream`]). Never blocks on the network.
    pub fn set(&self, demand: Demand, frame: Option<Frame>) {
        let mut slot = self.shared.slot.lock();
        let mut changed = slot.demand != demand;
        slot.demand = demand;
        if demand == Demand::Stream {
            if let Some(frame) = frame
                && !frame.is_empty()
            {
                slot.frame = Some(frame);
                slot.fresh = true;
                changed = true;
            }
        } else {
            slot.frame = None;
            slot.fresh = false;
        }
        drop(slot);
        if changed {
            self.shared.wake.notify_all();
        }
    }

    pub fn status(&self) -> Status {
        self.shared.slot.lock().status.clone()
    }

    /// Ends a running stream (black, close, stop over HTTPS) and the thread. Waits for it.
    pub fn shutdown(&self) {
        self.shared.slot.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Live {
    session: Session,
    sent_at: Option<Instant>,
    polled_at: Instant,
}

struct Worker {
    shared: Arc<Shared>,
    events: Arc<dyn Fn(Event) + Send + Sync>,
    bridge: Bridge,
    link: HueLink,
    area: String,
    app_id: Option<String>,
    /// The lights last told to ambient light; `None` until the area has been read.
    lights: Option<Vec<Light>>,
    /// The channels of the area, for the black frame at the end.
    channels: Vec<u8>,
    live: Option<Live>,
    last_frame: Option<Frame>,
    /// The demand the loop last acted on.
    seen: Demand,
    retry_at: Option<Instant>,
    backoff: Duration,
    last_problem: String,
    /// A frame has come that has not been sent yet.
    pending: bool,
}

/// What one pass of the loop is told.
struct Pass {
    demand: Demand,
    frame: Option<Frame>,
    stop: bool,
}

impl Worker {
    /// The least time between two updates.
    fn interval(&self) -> Duration {
        Duration::from_millis(1000 / u64::from(self.link.fps.max(1)))
    }

    fn run(mut self) {
        loop {
            let pass = self.wait();
            if pass.stop {
                self.end_stream();
                self.set_status("idle", None);
                return;
            }
            self.seen = pass.demand;
            if let Some(frame) = pass.frame {
                self.last_frame = Some(frame);
                self.pending = true;
            }
            match pass.demand {
                Demand::Off => {
                    self.end_stream();
                    self.retry_at = None;
                    self.set_status("idle", None);
                }
                Demand::Discover => {
                    self.end_stream();
                    if self.lights.is_none() {
                        self.discover();
                    } else {
                        self.set_status("idle", None);
                    }
                }
                Demand::Stream => self.stream_pass(),
            }
        }
    }

    fn retry_ready(&self, now: Instant) -> bool {
        self.retry_at.is_none_or(|t| now >= t)
    }

    /// Whether the thread has something to do, and when it next will have.
    fn due(&self, slot: &Slot, now: Instant) -> (bool, Option<Instant>) {
        if slot.stop || slot.demand != self.seen {
            return (true, None);
        }
        match (slot.demand, &self.live) {
            (Demand::Off, None) => (false, None),
            (Demand::Off | Demand::Discover, Some(_)) => (true, None),
            (Demand::Discover, None) => {
                if self.lights.is_some() {
                    (false, None)
                } else if self.retry_ready(now) {
                    (true, None)
                } else {
                    (false, self.retry_at)
                }
            }
            (Demand::Stream, None) => {
                if self.retry_ready(now) {
                    (true, None)
                } else {
                    (false, self.retry_at)
                }
            }
            (Demand::Stream, Some(live)) => {
                let from_sent = |gap: Duration| live.sent_at.map_or(now, |t| t + gap);
                let mut next = live.polled_at + POLL;
                if self.last_frame.is_some() {
                    next = next.min(from_sent(KEEPALIVE));
                    if slot.fresh || self.pending {
                        next = next.min(from_sent(self.interval()));
                    }
                } else if slot.fresh {
                    // Nothing to send yet, but a first frame is waiting to be taken.
                    next = now;
                }
                (now >= next, Some(next))
            }
        }
    }

    /// Blocks until there is something to do.
    fn wait(&mut self) -> Pass {
        let mut slot = self.shared.slot.lock();
        loop {
            let now = Instant::now();
            let (due, wake_at) = self.due(&slot, now);
            if due {
                let frame = if slot.fresh {
                    slot.fresh = false;
                    slot.frame.clone()
                } else {
                    None
                };
                return Pass {
                    demand: slot.demand,
                    frame,
                    stop: slot.stop,
                };
            }
            match wake_at {
                Some(at) => {
                    let _ = self.shared.wake.wait_until(&mut slot, at);
                }
                None => self.shared.wake.wait(&mut slot),
            }
        }
    }

    fn set_status(&mut self, state: &'static str, problem: Option<String>) {
        let status = Status { state, problem };
        {
            let mut slot = self.shared.slot.lock();
            if slot.status == status {
                return;
            }
            slot.status = status;
        }
        (self.events)(Event::Status);
    }

    fn fail(&mut self, words: String) {
        if self.last_problem != words {
            crate::log_warn!("hue: {words}");
            self.last_problem.clone_from(&words);
        }
        self.retry_at = Some(Instant::now() + self.backoff);
        self.backoff = (self.backoff * 2).min(RETRY_LONGEST);
        self.set_status("error", Some(words));
    }

    fn succeeded(&mut self) {
        self.retry_at = None;
        self.backoff = RETRY_FIRST;
        self.last_problem.clear();
    }

    fn key_and_id(&mut self) -> Result<String, HueError> {
        if let Some(id) = &self.app_id {
            return Ok(id.clone());
        }
        let id = self.bridge.application_id(&self.link.app_key)?;
        self.app_id = Some(id.clone());
        Ok(id)
    }

    /// Reads the area and tells ambient light where its lights are, when that is not what it was last told.
    fn read_area(&mut self) -> Result<Area, HueError> {
        let area = self.bridge.area(&self.link.app_key, &self.area)?;
        let lights: Vec<Light> = area
            .channels
            .iter()
            .map(|c| huestream::channel_light(c.id, c.x, c.z))
            .collect();
        self.channels = area.channels.iter().map(|c| c.id).collect();
        if self.lights.as_ref() != Some(&lights) {
            self.lights = Some(lights.clone());
            (self.events)(Event::Lights(lights));
        }
        Ok(area)
    }

    fn discover(&mut self) {
        self.set_status("connecting", None);
        let found = self.key_and_id().and_then(|_| self.read_area());
        match found {
            Ok(area) if area.channels.is_empty() => {
                self.lights = None;
                self.fail("the entertainment area has no lights in it".to_owned());
            }
            Ok(_) => {
                self.succeeded();
                self.set_status("idle", None);
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn stream_pass(&mut self) {
        if self.live.is_none() {
            self.begin();
            return;
        }
        let now = Instant::now();
        let interval = self.interval();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if now >= live.polled_at + POLL {
            live.polled_at = now;
            if let Err(e) = live.session.poll() {
                return self.lost(&e.to_string());
            }
        }
        let since = live
            .sent_at
            .map_or(Duration::MAX, |t| now.saturating_duration_since(t));
        // A new frame goes out as soon as the pace allows; the last one goes again when nothing new has come for a second.
        let send = (since >= interval && self.pending) || since >= KEEPALIVE;
        let Some(frame) = self.last_frame.as_ref().filter(|_| send) else {
            return;
        };
        match live.session.send(frame) {
            Ok(()) => {
                live.sent_at = Some(now);
                self.pending = false;
            }
            Err(e) => self.lost(&e.to_string()),
        }
    }

    /// The stream is gone without being asked: the bridge closed it, or the network did.
    fn lost(&mut self, why: &str) {
        if let Some(mut live) = self.live.take() {
            live.session.close();
        }
        // The area may still be marked as streaming; ending it puts it right for the next start.
        self.stop_area();
        self.fail(format!("the stream ended ({why})"));
    }

    fn begin(&mut self) {
        self.set_status("connecting", None);
        match self.open() {
            Ok(Some(live)) => {
                crate::log_info!("hue: streaming to area {}", self.area);
                self.live = Some(live);
                self.succeeded();
                self.set_status("streaming", None);
            }
            Ok(None) => self.set_status("idle", None),
            Err(words) => self.fail(words),
        }
    }

    /// Reads the area, starts it and opens the DTLS session. `Ok(None)`: no longer wanted by the time it was ready.
    fn open(&mut self) -> Result<Option<Live>, String> {
        let app_id = self.key_and_id().map_err(|e| e.to_string())?;
        let area = self.read_area().map_err(|e| e.to_string())?;
        if area.channels.is_empty() {
            return Err("the entertainment area has no lights in it".to_owned());
        }
        if area.active {
            if area.streamer.as_deref() == Some(app_id.as_str()) {
                // The same application id is this one's own earlier stream: the helper was killed before it could stop the area, or
                // the bridge never cleared it (Bifrost leaves an area active when a stream ends, until it is stopped). Nobody else
                // can hold it under this id, so it is stopped and started afresh.
                crate::log_info!("hue: stopping the area's stale stream before starting");
                self.bridge
                    .stop(&self.link.app_key, &self.area)
                    .map_err(|e| e.to_string())?;
            } else if area.streamer.is_some() {
                return Err("the entertainment area is streaming to another application".to_owned());
            } else {
                return Err(
                    "the entertainment area is in use; waiting for the bridge to let go".to_owned(),
                );
            }
        }
        let wanted = {
            let slot = self.shared.slot.lock();
            slot.demand == Demand::Stream && !slot.stop
        };
        if !wanted {
            return Ok(None);
        }
        self.bridge
            .start(&self.link.app_key, &self.area)
            .map_err(|e| e.to_string())?;
        let cfg = SessionConfig::new(
            SocketAddr::from((self.link.host, self.link.stream_port)),
            &app_id,
            self.link.client_key.expose(),
            &self.area,
        );
        match Session::connect(&cfg) {
            Ok(session) => Ok(Some(Live {
                session,
                sent_at: None,
                polled_at: Instant::now(),
            })),
            Err(e) => {
                self.stop_area();
                Err(format!("the stream could not be opened ({e})"))
            }
        }
    }

    /// Ends the stream the way a person would want the room left: black, closed, handed back to the bridge.
    fn end_stream(&mut self) {
        let Some(mut live) = self.live.take() else {
            return;
        };
        let black: Frame = self.channels.iter().map(|c| (*c, [0u16; 3])).collect();
        for _ in 0..2 {
            if live.session.send(&black).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        live.session.close();
        self.last_frame = None;
        self.stop_area();
    }

    fn stop_area(&mut self) {
        let total = self.bridge.total;
        self.bridge.total = STOP_WITHIN;
        let stopped = self.bridge.stop(&self.link.app_key, &self.area);
        self.bridge.total = total;
        if let Err(e) = stopped {
            crate::log_warn!("hue: stopping the area: {e}");
        }
    }
}
