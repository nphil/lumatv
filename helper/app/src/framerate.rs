//! The panel's output rate after 25 or 50 fps video: back to 120 Hz once Home is in front with no video window.
//!
//! LG's kernel picks the output rate from the source alone, at every timing change of window 0: 100 Hz for the 25/50 family,
//! 120 Hz for everything else (docs/research/framerate-result.md). It sets nothing when the window loses its source
//! (`v4l2_vsc_event.c`, `default: return`), so after a 25 or 50 fps video the panel stays at 100 Hz, and the UI at 50 Hz, until
//! the next video's first frame: 2.6 s inside YouTube in round 11, and on Home until the next video.
//!
//! Inside an app nothing is done: the next video's start carries LG's own switch, inside the firmware's start mute (round 11).
//! When SAM reports Home in front, one check runs: the output is measured (the `osd_irq` count over 0.5 s); at 100 Hz the watcher
//! waits until no source is connected to the main window (`connect_src=NONE` in the scaler's status), measures again, writes
//! `1 60` to `/proc/lg/de/fr_rate` (free run at the 60 family, 120 Hz on this panel), measures once more and logs what it found.
//! Nothing is written while a source is connected, with one exception. The screen coming on with Home in front is a check too, and so is
//! the end of a camera stream (`cameras.live` idle or failed): a pop-up on Home leaves the panel at 100 Hz with Home in front all along.
//!
//! The exception: when a camera stream starts to play with Home in front (`cameras.live` playing), the check waits a second for the
//! kernel's re-timing at the stream's first frame and, at 100 Hz, writes at once with the source connected. Round 22 (2026-10-30): five
//! variants of the request (`0 60`, `1 60`, `0 50`, `1 50`) written with no source connected left the output at 100 Hz, and only a playing
//! 30/60 fps video put 120 back, so the firmware seems to take the request only with a window connected [INFERENCE, tested next]. The
//! kernel re-sends its own only at a timing change, which a steady stream does not have. The rate is free run (`1 60`).
//!
//! The price: at the next video's connect the kernel applies the rate still held in the decoder's register (the 25/50 video's), so
//! that start goes to 100 Hz and back within its first frames, under the app's video mute and the firmware's start mute.
//!
//! Home is learnt from SAM's `getForegroundAppInfo` subscription (one `luna-send` child); while it is down nothing is done.
//! `helper.json` `frameRate.returnOnHome: false` turns the watcher off, and it does not start where `/proc/lg/de/fr_rate` is missing.

use crate::control::Stop;
use crate::ctx::Ctx;
use crate::screen;
use parking_lot::Mutex;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// LG's Home: SAM reports it in front while LumaHome is drawn (the Home stub keeps LG's id).
pub const HOME: &str = "com.webos.app.home";
const FOREGROUND_URI: &str = "luna://com.webos.applicationManager/getForegroundAppInfo";
/// The interrupt that follows the panel's refresh: 120 a second at 120 Hz, 100 at 100 Hz (`DE2VSYNC` also counts input-side
/// events, 150 to 245 a second on the round-11 runs).
const VSYNC_IRQ: &str = "osd_irq";
/// What is written: free run, the 60 family (120 Hz on this panel). `_DE_WriteProcFunc` reads it with `sscanf("%d %d")`.
const SET_120: &[u8] = b"1 60\n";

/// The steps of a check.
const POLL: Duration = Duration::from_millis(250);
/// How long a rate is counted: 50 or 60 interrupts, so one interrupt more or less is 2 Hz.
const MEASURE: Duration = Duration::from_millis(500);
/// The wait between the write and the measurement that checks it: the kernel's own switch completed within one 50 ms sample of
/// the DE reading the new rate in round 11.
const SETTLE: Duration = Duration::from_millis(200);
/// A source that stays connected on Home is looked at every `POLL` for this long, then every `SLOW_POLL`.
const FAST_FOR: Duration = Duration::from_secs(15);
const SLOW_POLL: Duration = Duration::from_secs(2);
/// A measured rate counts as 100 or 120 Hz within this much (119.6 Hz is the 23.976 fps output, phase-locked at 119.88).
const BAND_HZ: f64 = 3.5;
/// Measurements in a row that fit neither band before the check gives up.
const MAX_UNSETTLED: u32 = 8;
/// The longest the loop blocks with nothing owed (commands and the stop wake it at once).
const IDLE: Duration = Duration::from_secs(60);
/// The subscription is started again after this, doubling up to `RESTART_MAX` while it keeps ending early.
const RESTART_MIN: Duration = Duration::from_secs(5);
const RESTART_MAX: Duration = Duration::from_secs(300);
/// A subscription that delivered for this long before it ended restarts after `RESTART_MIN`.
const WORTH_RESTARTING: Duration = Duration::from_secs(30);
/// The most of one reply that is kept while it is incomplete.
const MAX_REPLY: usize = 64 * 1024;
/// The most of a status file that is read.
const MAX_STATUS: u64 = 64 * 1024;

/// What the watcher thread is told.
#[derive(Debug)]
pub enum Cmd {
    /// The screen topic changed (the screen watcher's notify).
    Wake,
    /// SAM's foreground app, from the subscription (`None`: a reply without one).
    Foreground(Option<String>),
    /// The subscription's child ended.
    Ended,
    /// A camera video ended (`cameras.live` went idle or failed): its decoder may have left the panel at 100 Hz with Home in front all along, which
    /// no foreground change says.
    VideoEnded,
    /// A camera video began to play (`cameras.live` went to playing): the kernel re-times the panel for a 25 or 50 fps stream at its first frame.
    VideoPlaying,
    Stop,
}

/// The files the watcher reads and writes.
#[derive(Clone, Debug)]
pub struct Sources {
    /// The main window's scaler status: `connect_src=NONE` when no source is connected.
    pub scaler: PathBuf,
    pub interrupts: PathBuf,
    /// LG's lever, `echo [freerun] [50|60]`: the `DE_IOW_SET_FR_RATE` the kernel issues at every timing change of window 0.
    pub fr_rate: PathBuf,
}

impl Sources {
    /// The set's own.
    pub fn lg() -> Self {
        Self {
            scaler: PathBuf::from("/proc/lgtv-driver/scaler/main.p0"),
            interrupts: PathBuf::from("/proc/interrupts"),
            fr_rate: PathBuf::from("/proc/lg/de/fr_rate"),
        }
    }
}

/// Whether the scaler's status says a source is connected (`connect_src` other than `NONE`); `None` without that line.
pub fn parse_connected(status: &str) -> Option<bool> {
    status
        .lines()
        .find_map(|line| line.trim().strip_prefix("connect_src="))
        .map(|src| src.trim() != "NONE")
}

/// The `osd_irq` count of `/proc/interrupts`, summed over the CPU columns the line has (two to four, as CPUs come and go).
pub fn parse_vsync_count(interrupts: &str) -> Option<u64> {
    let line = interrupts
        .lines()
        .find(|l| l.split_whitespace().next_back() == Some(VSYNC_IRQ))?;
    let mut fields = line.split_whitespace();
    fields.next()?.strip_suffix(':')?;
    Some(fields.map_while(|f| f.parse::<u64>().ok()).sum())
}

/// The output-rate band of a measured vsync rate: 100 or 120, or `None` for anything else.
pub fn band(hz: f64) -> Option<u32> {
    [120, 100]
        .into_iter()
        .find(|b| (hz - f64::from(*b)).abs() <= BAND_HZ)
}

/// The app in front, from one `getForegroundAppInfo` reply: the application manager's flat shape, or the list of the surface
/// manager's, where the primary entry counts. An empty id is none; a refused call is an error with the TV's text.
pub fn foreground_of(reply: &Value) -> Result<Option<String>, String> {
    if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return Err(reply
            .get("errorText")
            .and_then(Value::as_str)
            .unwrap_or("refused")
            .to_owned());
    }
    let entry = match reply.get("foregroundAppInfo").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .find(|e| e.get("primary").and_then(Value::as_bool) == Some(true))
            .or_else(|| list.first()),
        None => Some(reply),
    };
    Ok(entry
        .and_then(|e| e.get("appId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned))
}

/// What a check reads and writes. `&mut self` lets the real one keep its read buffer.
pub trait Panel {
    fn window_connected(&mut self) -> Result<bool, String>;
    fn vsync_count(&mut self) -> Result<u64, String>;
    fn set_120(&mut self) -> Result<(), String>;
}

/// The set's panel through `/proc`.
pub struct ProcPanel {
    sources: Sources,
    buf: String,
}

impl ProcPanel {
    pub fn new(sources: Sources) -> Self {
        Self {
            sources,
            buf: String::with_capacity(16 * 1024),
        }
    }
}

fn read_into(path: &Path, buf: &mut String) -> Result<(), String> {
    buf.clear();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_STATUS).read_to_string(buf))
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", path.display()))
}

impl Panel for ProcPanel {
    fn window_connected(&mut self) -> Result<bool, String> {
        read_into(&self.sources.scaler, &mut self.buf)?;
        parse_connected(&self.buf)
            .ok_or_else(|| format!("{}: no connect_src line", self.sources.scaler.display()))
    }

    fn vsync_count(&mut self) -> Result<u64, String> {
        read_into(&self.sources.interrupts, &mut self.buf)?;
        parse_vsync_count(&self.buf)
            .ok_or_else(|| format!("{}: no {VSYNC_IRQ} line", self.sources.interrupts.display()))
    }

    fn set_120(&mut self) -> Result<(), String> {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&self.sources.fr_rate)
            .and_then(|mut f| f.write_all(SET_120))
            .map_err(|e| format!("{}: {e}", self.sources.fr_rate.display()))
    }
}

/// What a check found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// The output was in the 120 Hz band already.
    Already(f64),
    /// It was at 100 Hz with no source connected: `1 60` was written, and `after` is the rate measured since.
    Returned { before: f64, after: f64 },
    /// The rate fit neither band, measurement after measurement: left alone.
    Unknown(f64),
}

/// What the loop does next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Step again after this long.
    Again(Duration),
    Done(Outcome),
}

/// A measurement in progress: when it started and the count then.
type From = Option<(Instant, u64)>;

#[derive(Debug)]
enum Phase {
    /// The output's rate, whatever is connected.
    First(From),
    /// At 100 Hz with a source connected (since when): waiting for it to go.
    AwaitClosed(Instant),
    /// No source connected: the rate once more before the write.
    Confirm(From),
    /// `1 60` written at `wrote` over `before` Hz; the rate since.
    Verify {
        wrote: Instant,
        before: f64,
        from: From,
    },
}

/// One check: measure, wait until no source is connected, measure again, write, measure. A check made for a camera stream that plays
/// (`Check::playing`) writes at once while the source is connected: with no source the firmware did not take the request on the set (round 22:
/// five writes at 100 Hz with no window, none changed the output), and the kernel only re-sends its own at a timing change, which a steady
/// stream does not have.
#[derive(Debug)]
pub struct Check {
    phase: Phase,
    unsettled: u32,
    playing: bool,
}

impl Default for Check {
    fn default() -> Self {
        Self::new()
    }
}

/// The rate since `from` once `MEASURE` has passed (and a new measurement starts then); the first call only takes the count.
fn measure(panel: &mut dyn Panel, from: &mut From, now: Instant) -> Result<Option<f64>, String> {
    if let Some((at, _)) = *from
        && now.saturating_duration_since(at) < MEASURE
    {
        return Ok(None);
    }
    let count = panel.vsync_count()?;
    let Some((at, before)) = from.replace((now, count)) else {
        return Ok(None);
    };
    Ok(Some(
        count.saturating_sub(before) as f64 / now.saturating_duration_since(at).as_secs_f64(),
    ))
}

impl Check {
    pub fn new() -> Self {
        Self {
            phase: Phase::First(None),
            unsettled: 0,
            playing: false,
        }
    }

    /// A check for a camera stream that has begun to play with Home in front.
    pub fn playing() -> Self {
        Self {
            playing: true,
            ..Self::new()
        }
    }

    /// One step at `now`. An error is the panel's (a file missing or not as expected).
    pub fn step(&mut self, panel: &mut dyn Panel, now: Instant) -> Result<Step, String> {
        loop {
            match &mut self.phase {
                Phase::First(from) => {
                    let Some(hz) = measure(panel, from, now)? else {
                        return Ok(Step::Again(POLL));
                    };
                    match band(hz) {
                        Some(120) => return Ok(Step::Done(Outcome::Already(hz))),
                        Some(_) => {
                            let connected = panel.window_connected()?;
                            crate::log_info!(
                                "frame rate: Home measures {hz:.1} Hz{}",
                                if connected { " with a source connected to the main window" } else { " with no source connected" }
                            );
                            if self.playing && connected {
                                panel.set_120()?;
                                self.phase = Phase::Verify {
                                    wrote: now,
                                    before: hz,
                                    from: None,
                                };
                                return Ok(Step::Again(SETTLE));
                            }
                            self.phase = Phase::AwaitClosed(now);
                        }
                        None => return Ok(self.unsettled(hz)),
                    }
                }
                Phase::AwaitClosed(since) => {
                    if panel.window_connected()? {
                        let fast = now.saturating_duration_since(*since) < FAST_FOR;
                        return Ok(Step::Again(if fast { POLL } else { SLOW_POLL }));
                    }
                    self.phase = Phase::Confirm(None);
                }
                Phase::Confirm(from) => {
                    if panel.window_connected()? {
                        self.phase = Phase::AwaitClosed(now);
                        return Ok(Step::Again(POLL));
                    }
                    let Some(hz) = measure(panel, from, now)? else {
                        return Ok(Step::Again(POLL));
                    };
                    match band(hz) {
                        Some(120) => return Ok(Step::Done(Outcome::Already(hz))),
                        // The last look before the write: a source that connected since gets its own rate from the kernel.
                        Some(_) if panel.window_connected()? => {
                            self.phase = Phase::AwaitClosed(now);
                            return Ok(Step::Again(POLL));
                        }
                        Some(_) => {
                            panel.set_120()?;
                            self.phase = Phase::Verify {
                                wrote: now,
                                before: hz,
                                from: None,
                            };
                            return Ok(Step::Again(SETTLE));
                        }
                        None => return Ok(self.unsettled(hz)),
                    }
                }
                Phase::Verify {
                    wrote,
                    before,
                    from,
                } => {
                    if now.saturating_duration_since(*wrote) < SETTLE {
                        return Ok(Step::Again(SETTLE));
                    }
                    let before = *before;
                    return Ok(match measure(panel, from, now)? {
                        Some(after) => Step::Done(Outcome::Returned { before, after }),
                        None => Step::Again(POLL),
                    });
                }
            }
        }
    }

    /// A rate in neither band (a switch in the middle of the count, or a rate this set does not use): count it again, a few times.
    fn unsettled(&mut self, hz: f64) -> Step {
        self.unsettled += 1;
        if self.unsettled >= MAX_UNSETTLED {
            Step::Done(Outcome::Unknown(hz))
        } else {
            Step::Again(POLL)
        }
    }
}

/// The subscription's child, in a slot the stop hook also reaches, and the thread that reads it.
struct Subscription {
    slot: Arc<Mutex<Option<Child>>>,
    reader: Option<JoinHandle<()>>,
    started: Instant,
}

impl Subscription {
    fn start(
        program: &str,
        slot: Arc<Mutex<Option<Child>>>,
        tx: Sender<Cmd>,
    ) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(["-n", "1000000", FOREGROUND_URI, r#"{"subscribe":true}"#])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no output pipe".to_owned());
        };
        let spawned = thread::Builder::new()
            .name("framerate-sub".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                // A reply is one line when luna-send prints it compact; a pretty-printed one is joined until it parses.
                let mut pending = String::new();
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    pending.push_str(&line);
                    pending.push('\n');
                    match serde_json::from_str::<Value>(&pending) {
                        Ok(reply) => {
                            pending.clear();
                            match foreground_of(&reply) {
                                Ok(app) => {
                                    if tx.send(Cmd::Foreground(app)).is_err() {
                                        return;
                                    }
                                }
                                Err(why) => crate::log_warn!(
                                    "frame rate: SAM refused the foreground subscription ({why})"
                                ),
                            }
                        }
                        Err(e) if e.is_eof() && pending.len() < MAX_REPLY => {}
                        Err(_) => pending.clear(),
                    }
                }
                let _ = tx.send(Cmd::Ended);
            });
        let reader = match spawned {
            Ok(reader) => reader,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("could not start a reader: {e}"));
            }
        };
        *slot.lock() = Some(child);
        Ok(Self {
            slot,
            reader: Some(reader),
            started: Instant::now(),
        })
    }

    fn end(mut self) {
        end_child(&self.slot);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn end_child(slot: &Mutex<Option<Child>>) {
    if let Some(mut child) = slot.lock().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The camera stream's topic.
const LIVE_TOPIC: &str = "cameras.live";
/// How long the live watcher waits for an event before it looks at the stop flag.
const LIVE_WAIT: Duration = Duration::from_secs(1);

/// Whether a `cameras.live` view says no stream is playing or on its way.
pub fn live_ended(view: &Value) -> bool {
    matches!(view.get("state").and_then(Value::as_str), Some("idle" | "failed"))
}

/// Whether a `cameras.live` view says a stream plays.
pub fn live_playing(view: &Value) -> bool {
    view.get("state").and_then(Value::as_str) == Some("playing")
}

/// Tells the watcher when a camera stream ends. A pop-up on Home is a 25 or 50 fps video the kernel re-times the panel for, and Home never left the
/// front, so SAM's foreground subscription has nothing to say about it.
fn watch_live(ctx: &Ctx, tx: Sender<Cmd>) -> std::io::Result<JoinHandle<()>> {
    let (hub, stop) = (ctx.hub.clone(), ctx.stop.clone());
    thread::Builder::new()
        .name("framerate-live".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let topics = [LIVE_TOPIC.to_owned()];
            let boot = hub.boot();
            let mut seq = hub.events_after(None, Some(boot), Some(&topics[..])).seq;
            let mut playing = false;
            while !stop.is_set() {
                let batch = hub.wait_events(Some(seq), Some(boot), Some(&topics[..]), LIVE_WAIT);
                seq = batch.seq;
                for event in &batch.events {
                    let now_playing = live_playing(&event.data);
                    let sent = if now_playing && !playing {
                        tx.send(Cmd::VideoPlaying)
                    } else if live_ended(&event.data) {
                        tx.send(Cmd::VideoEnded)
                    } else {
                        Ok(())
                    };
                    playing = now_playing;
                    if sent.is_err() {
                        return;
                    }
                }
            }
        })
}

fn report(outcome: Outcome) {
    match outcome {
        Outcome::Already(hz) => crate::log_debug!("frame rate: Home at {hz:.1} Hz"),
        Outcome::Returned { before, after } if band(after) == Some(120) => crate::log_info!(
            "frame rate: Home was left at {before:.1} Hz; set back to 120 Hz ({after:.1} Hz measured)"
        ),
        Outcome::Returned { before, after } => crate::log_warn!(
            "frame rate: wrote 1 60 to fr_rate with Home in front at {before:.1} Hz, but the output measures {after:.1} Hz"
        ),
        Outcome::Unknown(hz) => {
            crate::log_info!(
                "frame rate: the output measures {hz:.1} Hz on Home, neither 100 nor 120; left as it is"
            )
        }
    }
}

/// Starts the watcher on the set's own files. `None` when `frameRate.returnOnHome` is off or the set has no `fr_rate` (a host).
pub fn spawn(
    ctx: &Ctx,
    tx: Sender<Cmd>,
    rx: Receiver<Cmd>,
) -> std::io::Result<Option<JoinHandle<()>>> {
    spawn_with(ctx, Sources::lg(), tx, rx)
}

/// [`spawn`] on other files (a test's).
pub fn spawn_with(
    ctx: &Ctx,
    sources: Sources,
    tx: Sender<Cmd>,
    rx: Receiver<Cmd>,
) -> std::io::Result<Option<JoinHandle<()>>> {
    if !ctx.config.frame_rate.return_on_home {
        crate::log_info!(
            "frame rate: the return to 120 Hz on Home is off (frameRate.returnOnHome)"
        );
        return Ok(None);
    }
    if !sources.fr_rate.exists() {
        crate::log_debug!(
            "frame rate: no {}; the return to 120 Hz on Home is off",
            sources.fr_rate.display()
        );
        return Ok(None);
    }
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("framerate".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || run(&ctx, sources, &tx, &rx))
        .map(Some)
}

fn run(ctx: &Ctx, sources: Sources, tx: &Sender<Cmd>, rx: &Receiver<Cmd>) {
    let program = crate::sys::which(&ctx.config.luna.send)
        .map_or_else(|| ctx.config.luna.send.clone(), |p| p.display().to_string());
    let stop: &Stop = &ctx.stop;
    // The stop hook kills the subscription's child, so its reader ends at once.
    let slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    stop.on_stop({
        let slot = slot.clone();
        move || end_child(&slot)
    });
    let live_watch = watch_live(ctx, tx.clone())
        .map_err(|e| crate::log_warn!("frame rate: no watch on the camera streams ({e}); a pop-up's end is not looked at"))
        .ok();
    let mut panel = ProcPanel::new(sources);
    let mut subscription: Option<Subscription> = None;
    let mut restart_at = Instant::now();
    let mut backoff = RESTART_MIN;
    let mut start_failed = false;
    let mut home = false;
    let mut screen_on = screen::is_on(&ctx.hub);
    let mut check: Option<Check> = None;
    let mut next_step = Instant::now();
    let mut broken = false;
    loop {
        if stop.is_set() {
            break;
        }
        let now = Instant::now();
        if subscription.is_none() && now >= restart_at {
            match Subscription::start(&program, slot.clone(), tx.clone()) {
                Ok(s) => {
                    start_failed = false;
                    subscription = Some(s);
                    // A stop that came while the child was starting ran its hook against an empty slot.
                    if stop.is_set() {
                        break;
                    }
                }
                Err(why) => {
                    if !start_failed {
                        crate::log_warn!(
                            "frame rate: no foreground subscription ({why}); trying again"
                        );
                    }
                    start_failed = true;
                    restart_at = now + backoff;
                    backoff = (backoff * 2).min(RESTART_MAX);
                }
            }
        }
        let mut deadline = now + IDLE;
        if subscription.is_none() {
            deadline = deadline.min(restart_at);
        }
        if check.is_some() && screen_on && !broken {
            deadline = deadline.min(next_step);
        }
        match rx.recv_timeout(deadline.saturating_duration_since(now)) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::Wake) => {
                let on = screen::is_on(&ctx.hub);
                if on && !screen_on && home {
                    check = Some(Check::new());
                    next_step = Instant::now();
                }
                if !on {
                    check = None;
                }
                screen_on = on;
            }
            Ok(Cmd::VideoPlaying) => {
                if home && screen_on {
                    check = Some(Check::playing());
                    // the kernel re-times the panel at the stream's first frame, which a state of "playing" follows
                    next_step = Instant::now() + Duration::from_millis(1000);
                }
            }
            Ok(Cmd::VideoEnded) => {
                if home && screen_on && check.is_none() {
                    check = Some(Check::new());
                    next_step = Instant::now();
                }
            }
            Ok(Cmd::Foreground(app)) => {
                let is_home = app.as_deref() == Some(HOME);
                if is_home && !home && screen_on {
                    check = Some(Check::new());
                    next_step = Instant::now();
                }
                if !is_home {
                    check = None;
                }
                home = is_home;
            }
            Ok(Cmd::Ended) => {
                if let Some(s) = subscription.take() {
                    let lived = s.started.elapsed();
                    s.end();
                    backoff = if lived >= WORTH_RESTARTING {
                        RESTART_MIN
                    } else {
                        (backoff * 2).min(RESTART_MAX)
                    };
                    restart_at = Instant::now() + backoff;
                    if !stop.is_set() {
                        crate::log_info!(
                            "frame rate: the foreground subscription ended; subscribing again in {} s",
                            backoff.as_secs()
                        );
                    }
                }
                // Without the subscription Home is not known: nothing is done until it is back.
                home = false;
                check = None;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if broken || !screen_on || Instant::now() < next_step {
            continue;
        }
        let Some(c) = check.as_mut() else { continue };
        match c.step(&mut panel, Instant::now()) {
            Ok(Step::Again(wait)) => next_step = Instant::now() + wait,
            Ok(Step::Done(outcome)) => {
                report(outcome);
                check = None;
            }
            Err(why) => {
                crate::log_warn!(
                    "frame rate: {why}; the return to 120 Hz on Home is off until the helper restarts"
                );
                broken = true;
                check = None;
            }
        }
    }
    if let Some(s) = subscription.take() {
        s.end();
    }
    if let Some(handle) = live_watch {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `/proc/lgtv-driver/scaler/main.p0` as the set printed it with no source (round 11, `youtube50-20261026-0424/before.txt`).
    const SCALER_NONE: &str = "[main.p0]\nopen=0\nconnect_src=NONE\nconnect_index=0\nconnect_attr=0\noutput_mode=NONE\nmute=1\nhdr=0\n";

    #[test]
    fn the_scaler_says_whether_a_source_is_connected() {
        assert_eq!(parse_connected(SCALER_NONE), Some(false));
        assert_eq!(
            parse_connected(&SCALER_NONE.replace("connect_src=NONE", "connect_src=VDEC")),
            Some(true)
        );
        assert_eq!(
            parse_connected(&SCALER_NONE.replace("connect_src=NONE", "connect_src=HDMI")),
            Some(true)
        );
        assert_eq!(parse_connected("[main.p0]\nopen=0\n"), None);
    }

    #[test]
    fn the_vsync_count_sums_the_cpu_columns_there_are() {
        // Lines as the set printed them, with four and with three CPUs online (docs/research/ambient/probe-2026-10-25-1234).
        let four = "           CPU0       CPU1       CPU2       CPU3\n 99:        652          0          0          0     GICv2  99 Level\n104:    3644835          0          0          0     GICv2 104 Edge      osd_irq\n111:          0          0          0          0     GICv2 111 Edge      JpgDec\n";
        let three = "           CPU0       CPU1       CPU2\n104:    3648452          2          0     GICv2 104 Edge      osd_irq\n";
        assert_eq!(parse_vsync_count(four), Some(3_644_835));
        assert_eq!(parse_vsync_count(three), Some(3_648_454));
        assert_eq!(
            parse_vsync_count("175:    4821871          0     GICv2 207 Level     DE2VSYNC\n"),
            None
        );
    }

    #[test]
    fn rates_fall_in_the_two_bands_or_none() {
        assert_eq!(band(119.6), Some(120));
        assert_eq!(band(100.0), Some(100));
        assert_eq!(band(97.0), Some(100));
        assert_eq!(band(110.0), None);
        assert_eq!(band(96.0), None);
        assert_eq!(band(0.0), None);
    }

    #[test]
    fn the_foreground_is_read_from_either_reply_shape() {
        let flat: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/luna/getForegroundAppInfo-appmanager.json"
        ))
        .unwrap();
        assert_eq!(foreground_of(&flat).unwrap().as_deref(), Some("netflix"));
        let list: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/luna/getForegroundAppInfo.json"
        ))
        .unwrap();
        assert_eq!(foreground_of(&list).unwrap().as_deref(), Some(HOME));
        // A subscription's push, as SAM sent it to pqcontroller in round 11.
        let push = json!({"subscribed": true, "returnValue": true, "appId": "com.beb.plxnative", "appCategory": [], "processId": "", "windowId": ""});
        assert_eq!(
            foreground_of(&push).unwrap().as_deref(),
            Some("com.beb.plxnative")
        );
        assert_eq!(
            foreground_of(&json!({"returnValue": true, "appId": ""})).unwrap(),
            None
        );
        assert_eq!(
            foreground_of(&json!({"returnValue": false, "errorText": "Denied"})).unwrap_err(),
            "Denied"
        );
    }

    /// A panel whose vsync count runs at `hz` on a clock the test moves.
    struct FakePanel {
        at: Instant,
        count: f64,
        hz: f64,
        connected: bool,
        /// The answers of the next `window_connected` calls, before `connected` again.
        connected_script: Vec<bool>,
        writes: u32,
        /// Whether a write takes the output to 120 Hz.
        write_takes: bool,
        broken: bool,
    }

    impl FakePanel {
        fn new(at: Instant, hz: f64, connected: bool) -> Self {
            Self {
                at,
                count: 1_000_000.0,
                hz,
                connected,
                connected_script: Vec::new(),
                writes: 0,
                write_takes: true,
                broken: false,
            }
        }

        fn advance(&mut self, by: Duration) -> Instant {
            self.count += self.hz * by.as_secs_f64();
            self.at += by;
            self.at
        }
    }

    impl Panel for FakePanel {
        fn window_connected(&mut self) -> Result<bool, String> {
            if self.broken {
                return Err("main.p0: gone".to_owned());
            }
            if self.connected_script.is_empty() {
                Ok(self.connected)
            } else {
                Ok(self.connected_script.remove(0))
            }
        }

        fn vsync_count(&mut self) -> Result<u64, String> {
            Ok(self.count as u64)
        }

        fn set_120(&mut self) -> Result<(), String> {
            self.writes += 1;
            if self.write_takes {
                self.hz = 120.0;
            }
            Ok(())
        }
    }

    /// Steps the check, moving the fake clock by what each step asks, until it is done or `limit` has passed.
    fn run_check(
        panel: &mut FakePanel,
        limit: Duration,
        mut between: impl FnMut(&mut FakePanel, Duration),
    ) -> Option<Outcome> {
        let mut check = Check::new();
        let start = panel.at;
        loop {
            let now = panel.at;
            match check.step(panel, now).unwrap() {
                Step::Done(outcome) => return Some(outcome),
                Step::Again(wait) => {
                    panel.advance(wait);
                    let elapsed = panel.at - start;
                    if elapsed > limit {
                        return None;
                    }
                    between(panel, elapsed);
                }
            }
        }
    }

    #[test]
    fn home_at_120_hz_is_left_alone() {
        let mut panel = FakePanel::new(Instant::now(), 119.6, false);
        let outcome = run_check(&mut panel, Duration::from_secs(5), |_, _| {}).unwrap();
        assert!(
            matches!(outcome, Outcome::Already(hz) if band(hz) == Some(120)),
            "{outcome:?}"
        );
        assert_eq!(panel.writes, 0);
    }

    #[test]
    fn home_at_100_hz_is_set_back_only_once_the_window_has_closed() {
        let mut panel = FakePanel::new(Instant::now(), 100.0, true);
        let mut wrote_while_connected = false;
        let (mut closed_at, mut wrote_by) = (None, None);
        let outcome = run_check(&mut panel, Duration::from_secs(10), |p, elapsed| {
            wrote_while_connected |= p.writes > 0 && p.connected;
            if p.writes > 0 && wrote_by.is_none() {
                wrote_by = Some(elapsed);
            }
            if elapsed >= Duration::from_secs(2) && p.connected {
                p.connected = false;
                closed_at = Some(elapsed);
            }
        })
        .unwrap();
        assert_eq!(panel.writes, 1);
        assert!(!wrote_while_connected);
        let Outcome::Returned { before, after } = outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(band(before), Some(100));
        assert_eq!(band(after), Some(120));
        // A whole measurement with the window closed comes before the write, and the write follows the close within a second.
        let gap = wrote_by.unwrap() - closed_at.unwrap();
        assert!(gap >= MEASURE && gap <= Duration::from_secs(1), "{gap:?}");
    }

    #[test]
    fn a_source_that_connects_during_the_last_measurement_holds_the_write_back() {
        let mut panel = FakePanel::new(Instant::now(), 100.0, false);
        let mut reconnected = false;
        let outcome = run_check(&mut panel, Duration::from_secs(4), |p, elapsed| {
            // A new video connects 0.8 s in, during the measurement with the window closed, and stays.
            if elapsed >= Duration::from_millis(800) && !reconnected {
                p.connected = true;
                reconnected = true;
            }
        });
        assert_eq!(
            outcome, None,
            "the check waits while the source is connected"
        );
        assert_eq!(panel.writes, 0);
    }

    #[test]
    fn a_source_that_connects_just_before_the_write_holds_it_back() {
        let mut panel = FakePanel::new(Instant::now(), 100.0, false);
        let mut check = Check::new();
        let mut now = panel.at;
        // First measurement (100 Hz), then the confirmation with the window closed.
        let mut steps = 0;
        while steps < 40 {
            if let Step::Again(wait) = check.step(&mut panel, now).unwrap() {
                now = panel.advance(wait);
            }
            steps += 1;
            if matches!(check.phase, Phase::Confirm(Some(_))) {
                break;
            }
        }
        // The confirmation's measurement ends now; the look before the write sees a source.
        now = panel.advance(MEASURE);
        panel.connected_script = vec![false, true];
        assert_eq!(check.step(&mut panel, now).unwrap(), Step::Again(POLL));
        assert_eq!(panel.writes, 0);
        assert!(matches!(check.phase, Phase::AwaitClosed(_)));
    }

    #[test]
    fn a_write_that_does_not_take_is_reported_as_measured() {
        let mut panel = FakePanel::new(Instant::now(), 100.0, false);
        panel.write_takes = false;
        let outcome = run_check(&mut panel, Duration::from_secs(5), |_, _| {}).unwrap();
        let Outcome::Returned { after, .. } = outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(band(after), Some(100));
        assert_eq!(panel.writes, 1, "written once, never retried");
    }

    #[test]
    fn a_rate_in_neither_band_is_left_alone() {
        let mut panel = FakePanel::new(Instant::now(), 110.0, false);
        let outcome = run_check(&mut panel, Duration::from_secs(10), |_, _| {}).unwrap();
        assert!(matches!(outcome, Outcome::Unknown(_)), "{outcome:?}");
        assert_eq!(panel.writes, 0);
    }

    #[test]
    fn a_source_that_stays_connected_is_looked_at_less_often_after_a_while() {
        let mut panel = FakePanel::new(Instant::now(), 100.0, true);
        let mut check = Check::new();
        let start = panel.at;
        let mut now = start;
        let mut last = Duration::ZERO;
        while now - start < Duration::from_secs(20) {
            let Step::Again(wait) = check.step(&mut panel, now).unwrap() else {
                panic!("done while connected")
            };
            last = wait;
            now = panel.advance(wait);
        }
        assert_eq!(last, SLOW_POLL);
        assert_eq!(panel.writes, 0);
    }

    #[test]
    fn a_panel_that_cannot_be_read_is_an_error() {
        let mut panel = FakePanel::new(Instant::now(), 100.0, false);
        panel.broken = true;
        let mut check = Check::new();
        let mut now = panel.at;
        let mut result = Ok(Step::Again(POLL));
        for _ in 0..10 {
            result = check.step(&mut panel, now);
            match result {
                Ok(Step::Again(wait)) => now = panel.advance(wait),
                _ => break,
            }
        }
        assert_eq!(result.unwrap_err(), "main.p0: gone");
        assert_eq!(panel.writes, 0);
    }

    #[test]
    fn the_proc_panel_reads_the_status_files_and_writes_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let sources = Sources {
            scaler: dir.path().join("main.p0"),
            interrupts: dir.path().join("interrupts"),
            fr_rate: dir.path().join("fr_rate"),
        };
        std::fs::write(&sources.scaler, SCALER_NONE).unwrap();
        std::fs::write(
            &sources.interrupts,
            "104:    3644835          0          0          0     GICv2 104 Edge      osd_irq\n",
        )
        .unwrap();
        std::fs::write(&sources.fr_rate, "").unwrap();
        let mut panel = ProcPanel::new(sources.clone());
        assert!(!panel.window_connected().unwrap());
        assert_eq!(panel.vsync_count().unwrap(), 3_644_835);
        panel.set_120().unwrap();
        assert_eq!(std::fs::read(&sources.fr_rate).unwrap(), b"1 60\n");
        std::fs::remove_file(&sources.scaler).unwrap();
        assert!(panel.window_connected().unwrap_err().contains("main.p0"));
        std::fs::write(
            &sources.interrupts,
            "175:  1  0  GICv2 207 Level  DE2VSYNC\n",
        )
        .unwrap();
        assert!(panel.vsync_count().unwrap_err().contains("no osd_irq line"));
    }
}
