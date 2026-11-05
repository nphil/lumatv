//! The panel back at 120 Hz after a 25 or 50 fps video left it at 100: a clip of 30 pictures a second plays on the main window, out of sight.
//!
//! **What sets the rate.** LG's kernel picks the display engine's output rate from the rate of the source on window 0, at a timing change of that window and
//! at nothing else: `_vsc_check_fr` gives 50 (100 Hz on this panel) for 25 and 50 fps and 60 (120 Hz) for everything else, and the window with no source
//! sets nothing (`v4l2_vsc_event.c`, `default: return`), so the rate a video left stays until the next video's first frame (`framerate-result.md`).
//!
//! **What does not work.** The same request through `/proc/lg/de/fr_rate` (`1 60`, `0 60`, `0 50`, `1 50`) was ignored every time it was made: with no source
//! (round 22, five variants; the compositor's own at the TV session of 2026-11-04 night, 16 s on, still 100 Hz), and with the Ring's 25 fps stream connected
//! (the helper's, "wrote 1 60 ... but the output measures 99.9 Hz"). It is not made here.
//!
//! **What does.** A source's first frame at another rate. Every return to 120 Hz on the set was one: YouTube at 30 and 60 fps (round 22), a camera pop-up
//! at 20 fps (round 23), and Live TV's DTV video (the panel went to 120 Hz 1.2 s after `legacy-broadcast-atsc` was attached, `setDisplayWindow` at
//! srv 23538.483 and "refreshes every 8.33 ms now" at 23539.656 in the evening session's RAM log). And the same route took the panel the other way: the Ring's
//! 25 fps stream through the camera feeder went to 100 Hz 1.7 s after the feeder's decoder connected. So the way back is the feeder's way there with another
//! rate: [`CLIP`], black and 830 bytes, declares 30 pictures a second in its SPS, and `lumatv-gfn-feeder --play` loops it in a video window of its own
//! with the decoder opened for Home (the recipe of `tools/tv-hwplanes.sh feeder`).
//!
//! **Out of sight.** The window is placed at [`RECT`] and no hole is cut for it (`Window::vwin_quiet`): the video plane is under every pixel of the UI,
//! which is opaque there, so nothing of it shows. If the decoder does not connect for a window under the UI, `ctl retime show` cuts the hole.
//!
//! **When.** The flips measure a period that is a fifth to a third over the mode's (`refresh::retimed`), Home is in front, and nothing feeds the main window
//! for 1.5 s (no live video window, no hardware claim, `connect_src=NONE`). The panel's rate is then read from the OSD interrupt (`osd_irq`, as the helper
//! does), because a Home at rest makes no flips to say that it changed: 100 Hz starts the clip, 120 Hz means the flips were old and nothing is played. The
//! clip ends one second after the panel is back, when Home is left, a video or a claim takes the main window, or the screen goes off, and the player is
//! ended when it has not moved the panel in 8 s. Two tries an episode, a minute apart; an episode ends with the panel back at its rate or anything else
//! taking the window. Every step is in the journal (`retime:`), with the player's last words when it fails.
//!
//! The decision is [`Machine`], pure and tested on a fake clock; [`Player`] is the process; [`Controller`] and the `impl State` are the glue.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::Rect;
use crate::refresh::{period_ns, retimed};
use crate::window::WindowType;

/// The clip: 640x360, black, one IDR and 29 skipped pictures, the SPS timing `1/60 s` a tick with two ticks a picture (30 a second, fixed), one
/// reference, no B pictures, an access unit delimiter before each picture and the parameter sets repeated at the IDR. The SPS rate is what the decoder
/// reports to the display engine (`rate_src CODEC`), so it is the number that counts. Made with
/// `ffmpeg -f lavfi -i color=c=black:size=640x360:rate=30 -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -preset veryslow
/// -x264-params aud=1:repeat-headers=1:slices=1:bframes=0:keyint=30:scenecut=0:ref=1:force-cfr=1 -bsf:v filter_units=remove_types=6 -f h264 retime-30fps-640x360.h264`.
pub const CLIP: &[u8] = include_bytes!("retime/retime-30fps-640x360.h264");
pub const CLIP_FILE: &str = "retime-30fps-640x360.h264";
const CLIP_SIZE: &str = "640x360";
const CLIP_FPS: &str = "30";

/// The app id of the clip's video window: the compositor places it by this id and no claim may take it.
pub const APP_ID: &str = "org.nphil.lumatv.retime";
/// The app the clip's decoder is opened for: videooutputd connects a video only for an app on SAM's foreground list, and Home is where the clip plays.
const NDL_APP: &str = "com.webos.app.home";
/// Where the clip is placed, in output pixels (a 128 by 72 window at 16,16 in the UI's space; the panel's are twice that).
pub const RECT: Rect = Rect::new(16, 16, 128, 72);
/// How long the player runs if nobody ends it first.
const PLAYER_SECONDS: u32 = 12;

/// The scaler's status for the main window (`connect_src=NONE` when no video feeds it) and the interrupt table that holds `osd_irq`.
pub const SCALER_PATH: &str = "/proc/lgtv-driver/scaler/main.p0";
pub const INTERRUPTS_PATH: &str = "/proc/interrupts";
/// The feeder, where `lumatv` stages it beside the helper's.
pub const FEEDER_PATH: &str = "bin/lumatv-gfn-feeder";

const MS: u64 = 1_000_000;
const S: u64 = 1_000 * MS;
/// How long the main window must stay clear, with the panel held, before the rate is read and the clip started: a closing video's window takes a moment to
/// go, and the kernel re-times at a window's first frame.
const CLEAR_FOR: u64 = 1_500 * MS;
/// How long the clip may play before it is given up: the Ring's stream took 1.7 s from its decoder's connection to the re-time.
const TRY_FOR: u64 = 8 * S;
/// How long the clip plays on once the panel is back.
const HOLD: u64 = S;
/// How long a clip started by hand plays unless `ctl retime now SECONDS` asks for another time (the player ends itself at [`PLAYER_SECONDS`] in any case).
const FORCED_FOR: u64 = 4 * S;
const FORCED_MAX: u64 = 10 * S;
/// The wait before a second try, and before the flips' word is asked again when the panel was found at its rate.
const RETRY_AFTER: u64 = 60 * S;
const STALE_REST: u64 = 30 * S;
const MAX_TRIES: u32 = 2;
/// A reading of the rate counts this long.
const FRESH: u64 = S;
/// How often the interrupt is counted, and the least time a rate is read over.
const SAMPLE_EVERY: u64 = 250 * MS;
const MIN_SPAN: u64 = 400 * MS;
/// How long a player that was asked to end has before it is killed.
const KILL_AFTER: u64 = 2 * S;
/// A measured rate counts as 100 or 120 Hz within this much (the helper's band).
const BAND_HZ: f64 = 3.5;

/// The output-rate band of a measured rate: 100 or 120, or `None` for anything else (a switch in the middle of the count).
pub fn band(hz: f64) -> Option<u32> {
    [120, 100].into_iter().find(|b| (hz - f64::from(*b)).abs() <= BAND_HZ)
}

/// Whether the scaler status says a source feeds the main window; `None` when the text has no such line.
pub fn source_connected(status: &str) -> Option<bool> {
    status.lines().find_map(|l| l.trim().strip_prefix("connect_src=")).map(|s| s.trim() != "NONE")
}

/// The `osd_irq` count of `/proc/interrupts`: the OSD's vsync, 120 a second at 120 Hz and 100 at 100, summed over the CPU columns the line has.
pub fn osd_count(interrupts: &str) -> Option<u64> {
    let line = interrupts.lines().find(|l| l.split_whitespace().next_back() == Some("osd_irq"))?;
    let mut fields = line.split_whitespace();
    fields.next()?.strip_suffix(':')?;
    Some(fields.map_while(|f| f.parse::<u64>().ok()).sum())
}

/// The rate over the counts taken at least [`MIN_SPAN`] apart.
#[derive(Debug, Default)]
pub struct Meter {
    base: Option<(u64, u64)>,
}

impl Meter {
    /// A count of the interrupt taken at `now_ns`; the rate in Hz once a reading is complete (and the next starts there).
    pub fn feed(&mut self, now_ns: u64, count: u64) -> Option<f64> {
        let Some((at, before)) = self.base else {
            self.base = Some((now_ns, count));
            return None;
        };
        let span = now_ns.saturating_sub(at);
        if span < MIN_SPAN {
            return None;
        }
        self.base = Some((now_ns, count));
        Some(count.saturating_sub(before) as f64 * 1e9 / span as f64)
    }

    pub fn reset(&mut self) {
        self.base = None;
    }
}

// ---- the decision --------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// The panel is not held at a re-timed rate, or something else feeds the main window.
    #[default]
    Idle,
    /// Held, with the main window clear since.
    Clear(u64),
    /// The panel was read at 100 Hz: the clip has played since.
    Playing(u64),
    /// The panel is back (or the clip was started by hand): the clip plays on until.
    Holding(u64),
    /// No clip before then: a try that did not take, or a panel found at its rate. `u64::MAX` is out of tries.
    Rest(u64),
}

/// What one look at the screen says.
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    /// The flips measure a re-timed period.
    pub stuck: bool,
    /// Home is in front and nothing but the clip feeds the main window.
    pub clear: bool,
    /// The player runs (false while there is none).
    pub alive: bool,
    /// A reading of the panel's rate taken at this look, if one was completed.
    pub hz: Option<f64>,
}

/// Why the clip was ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The panel was back and the clip played on for its hold, or a clip started by hand ran out.
    Done,
    /// It did not move the panel in [`TRY_FOR`].
    Timeout,
    /// The player ended by itself before the panel moved.
    Ended,
    /// Home left the front, a video or a claim took the main window, or the screen went off.
    Busy,
    /// Switched off, or the refresh was set by hand.
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    None,
    /// Start the player.
    Start,
    /// The panel is back at 120 Hz this long after the clip began; the clip plays on for its hold.
    Back(u64),
    /// End the player.
    Stop(Why),
}

/// The decision. An episode is a stretch with the panel held at a re-timed rate and Home in front with nothing else on the main window: it ends when the
/// panel is back by any means, or something takes the window. Within one the clip is tried twice, a minute apart.
#[derive(Debug, Default)]
pub struct Machine {
    phase: Phase,
    tries: u32,
    reading: Option<(u64, f64)>,
}

impl Machine {
    /// Whether the clip plays (or is meant to).
    pub fn playing(&self) -> bool {
        matches!(self.phase, Phase::Playing(_) | Phase::Holding(_))
    }

    /// Whether the machine needs a reading of the panel's rate at the next look.
    pub fn wants_rate(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Clear(_) | Phase::Playing(_))
    }

    /// Whether the scaler's status is wanted at the next look: while a clip is about to start (an app's source may have come), not while it plays (its own
    /// source) or has been tried and left, which cost a read of a proc file at every pass of the frame loop.
    pub fn reads_source(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Clear(_))
    }

    pub fn tries(&self) -> u32 {
        self.tries
    }

    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "idle",
            Phase::Clear(_) => "clear",
            Phase::Playing(_) => "playing",
            Phase::Holding(_) => "holding",
            Phase::Rest(u64::MAX) => "spent",
            Phase::Rest(_) => "rest",
        }
    }

    /// Back to no episode (the panel is at its rate, or the switch went off).
    pub fn reset(&mut self) {
        *self = Machine { reading: self.reading, ..Machine::default() };
    }

    /// A clip started by hand (`ctl retime now`): it plays for `for_ns` (a second to [`FORCED_MAX`]) whatever the rate is. The caller has seen the window clear.
    pub fn force(&mut self, now_ns: u64, for_ns: u64) -> Step {
        self.phase = Phase::Holding(now_ns + for_ns.clamp(S, FORCED_MAX));
        Step::Start
    }

    /// The player did not start: a try that did not take.
    pub fn start_failed(&mut self, now_ns: u64) {
        self.fail(now_ns);
    }

    fn fail(&mut self, now_ns: u64) {
        self.phase = Phase::Rest(if self.tries >= MAX_TRIES { u64::MAX } else { now_ns + RETRY_AFTER });
    }

    /// What to do at `now_ns` given what the screen says.
    pub fn step(&mut self, now_ns: u64, o: Obs) -> Step {
        if let Some(hz) = o.hz {
            self.reading = Some((now_ns, hz));
        }
        let band = self.reading.filter(|(at, _)| now_ns.saturating_sub(*at) <= FRESH).and_then(|(_, hz)| band(hz));
        match self.phase {
            Phase::Playing(since) => {
                if !o.clear {
                    self.reset();
                    return Step::Stop(Why::Busy);
                }
                if !o.alive {
                    self.fail(now_ns);
                    return Step::Stop(Why::Ended);
                }
                if !o.stuck || band == Some(120) {
                    self.phase = Phase::Holding(now_ns + HOLD);
                    return Step::Back(now_ns.saturating_sub(since));
                }
                if now_ns.saturating_sub(since) >= TRY_FOR {
                    self.fail(now_ns);
                    return Step::Stop(Why::Timeout);
                }
                Step::None
            }
            Phase::Holding(until) => {
                if !o.clear {
                    self.reset();
                    return Step::Stop(Why::Busy);
                }
                if !o.alive || now_ns >= until {
                    self.reset();
                    return Step::Stop(Why::Done);
                }
                Step::None
            }
            _ if !o.stuck || !o.clear => {
                self.reset();
                Step::None
            }
            Phase::Idle => {
                self.phase = Phase::Clear(now_ns);
                Step::None
            }
            Phase::Clear(since) => {
                if now_ns.saturating_sub(since) < CLEAR_FOR {
                    return Step::None;
                }
                match band {
                    Some(100) => {
                        self.tries += 1;
                        self.phase = Phase::Playing(now_ns);
                        Step::Start
                    }
                    // the flips' period was an old one: the panel is where it should be
                    Some(_) => {
                        self.phase = Phase::Rest(now_ns + STALE_REST);
                        Step::None
                    }
                    None => Step::None,
                }
            }
            Phase::Rest(until) => {
                if now_ns >= until {
                    self.phase = Phase::Clear(now_ns);
                }
                Step::None
            }
        }
    }
}

// ---- the process ---------------------------------------------------------------------------------------------------------------------------------

/// What the player is started with.
#[derive(Clone, Debug)]
pub struct Spec {
    pub program: PathBuf,
    pub clip: PathBuf,
    pub stats: PathBuf,
    pub runtime_dir: PathBuf,
    pub socket: String,
}

impl Spec {
    /// The feeder's command line: `tools/tv-hwplanes.sh feeder`'s, which ran on the set, with the clip looped in a window of the video kind and the decoder
    /// opened for Home.
    pub fn args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = ["--play"].map(OsString::from).into();
        a.push(self.clip.clone().into_os_string());
        for w in ["--codec", "h264", "--size", CLIP_SIZE, "--fps", CLIP_FPS, "--seconds"] {
            a.push(w.into());
        }
        a.push(PLAYER_SECONDS.to_string().into());
        for w in ["--loop", "-w", "video", "-a", APP_ID, "--ndl-app", NDL_APP, "--stats"] {
            a.push(w.into());
        }
        a.push(self.stats.clone().into_os_string());
        a
    }
}

/// The feeder in its one job here. Ended with SIGTERM (the feeder closes its pipeline), and killed if it is still there [`KILL_AFTER`] later; a frame loop
/// never waits for it.
#[derive(Default)]
pub struct Player {
    child: Option<Child>,
    /// Told to end: the monotonic nanosecond it is killed at.
    dying: Vec<(Child, u64)>,
    /// The last line it said, for the journal.
    said: Arc<Mutex<String>>,
}

impl Player {
    /// Starts the player; its pid.
    pub fn start(&mut self, spec: &Spec) -> io::Result<u32> {
        self.said.lock().clear();
        let log = crate::childlog::pipe_noting("lumatv-compositor", "retime: ", self.said.clone());
        let mut cmd = Command::new(&spec.program);
        cmd.args(spec.args())
            .env_remove("NOTIFY_SOCKET")
            .env_remove("LUMATV_LUNA_BRIDGE")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            // LG's AV connector opens a Wayland connection of its own to register the window id and reads these two to find the compositor
            .env("XDG_RUNTIME_DIR", &spec.runtime_dir)
            .env("WAYLAND_DISPLAY", &spec.socket)
            .stdin(Stdio::null());
        match log {
            Some(fd) => {
                let err = fd.try_clone().map(Stdio::from).unwrap_or_else(|_| Stdio::null());
                cmd.stdout(Stdio::from(fd)).stderr(err);
            }
            None => {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: prctl and getppid in the child between fork and exec touch nothing shared.
            let parent = std::process::id() as libc::pid_t;
            unsafe {
                cmd.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        return Err(io::Error::other("the compositor exited during the player's launch"));
                    }
                    Ok(())
                });
            }
        }
        let child = cmd.spawn().map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", spec.program.display())))?;
        let pid = child.id();
        self.child = Some(child);
        Ok(pid)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// How the player ended, once it has (and then there is none); `None` while it runs or when there is none.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        let status = self.child.as_mut()?.try_wait().ok().flatten()?;
        self.child = None;
        Some(status)
    }

    /// Asks the player to end; [`Player::reap`] collects it.
    pub fn stop(&mut self, now_ns: u64) {
        if let Some(child) = self.child.take() {
            // SAFETY: kill(2) of a child of ours that has not been waited for.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
            self.dying.push((child, now_ns + KILL_AFTER));
        }
    }

    /// Collects the players that went, and kills the ones that did not at their time.
    pub fn reap(&mut self, now_ns: u64) {
        self.dying.retain_mut(|(child, kill_at)| match child.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) if now_ns < *kill_at => true,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                false
            }
        });
    }

    /// Whether any player of this one's is still there (running, or told to end and not gone).
    pub fn any(&self) -> bool {
        self.child.is_some() || !self.dying.is_empty()
    }

    /// The last line the player said.
    pub fn said(&self) -> String {
        self.said.lock().clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        for mut child in self.child.take().into_iter().chain(self.dying.drain(..).map(|(c, _)| c)) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---- the glue ------------------------------------------------------------------------------------------------------------------------------------

/// What the compositor keeps of the retime.
pub struct Controller {
    /// `ctl retime on|off`, `--no-retime`.
    pub enabled: bool,
    /// `ctl retime show`: the clip's window gets its hole, to see where it is (and for a decoder that does not connect under the UI).
    pub show: bool,
    pub machine: Machine,
    meter: Meter,
    pub player: Player,
    sampled_ns: u64,
    /// The feeder program, the scaler's status and the interrupt table: the set's, and a test's own.
    pub program: PathBuf,
    pub scaler: PathBuf,
    pub interrupts: PathBuf,
    buf: String,
    /// The panel's rate as last read, how the last player ended, and the last thing the retime said, for `ctl retime`.
    last_hz: Option<f64>,
    exit: String,
    last: String,
}

impl Controller {
    pub fn new(data_dir: &Path, enabled: bool) -> Self {
        Controller {
            enabled,
            show: false,
            machine: Machine::default(),
            meter: Meter::default(),
            player: Player::default(),
            sampled_ns: 0,
            program: data_dir.join(FEEDER_PATH),
            scaler: PathBuf::from(SCALER_PATH),
            interrupts: PathBuf::from(INTERRUPTS_PATH),
            buf: String::new(),
            last_hz: None,
            exit: String::new(),
            last: String::new(),
        }
    }

    fn read(&mut self, path: &Path) -> Option<&str> {
        use std::io::Read;
        self.buf.clear();
        std::fs::File::open(path).ok()?.take(64 * 1024).read_to_string(&mut self.buf).ok()?;
        Some(&self.buf)
    }

    /// The panel's rate, when a reading is complete at `now_ns` (the interrupt is counted every [`SAMPLE_EVERY`]).
    fn sample(&mut self, now_ns: u64) -> Option<f64> {
        if now_ns.saturating_sub(self.sampled_ns) < SAMPLE_EVERY {
            return None;
        }
        self.sampled_ns = now_ns;
        let path = self.interrupts.clone();
        let count = osd_count(self.read(&path)?)?;
        let hz = self.meter.feed(now_ns, count)?;
        self.last_hz = Some(hz);
        Some(hz)
    }
}

impl crate::State {
    /// Puts the panel back to 120 Hz after a 25/50 fps video left it at 100 (`retime`). Each pass of the frame loop; with the panel at its rate and no clip
    /// playing it looks at the flips' period and nothing else.
    pub fn panel_rate_sync(&mut self, now_ns: u64) {
        self.retime.player.reap(now_ns);
        let playing = self.retime.machine.playing();
        if !self.retime.enabled || self.refresh_policy.manual {
            self.retime.machine.reset();
            self.retime.meter.reset();
            if playing {
                self.retime_stop(now_ns, Why::Off);
            }
            return;
        }
        let flips = self.output.flip_period_ns().filter(|&p| retimed(period_ns(self.config.refresh_hz), p));
        if flips.is_none() && !playing {
            self.retime.machine.reset();
            self.retime.meter.reset();
            return;
        }
        let clear = self.panel_rate_clear(self.retime.machine.reads_source());
        let hz = if clear && self.retime.machine.wants_rate() {
            self.retime.sample(now_ns)
        } else {
            self.retime.meter.reset();
            None
        };
        let alive = playing && {
            if let Some(status) = self.retime.player.exited() {
                self.retime.exit = status.to_string();
            }
            self.retime.player.pid().is_some()
        };
        match self.retime.machine.step(now_ns, Obs { stuck: flips.is_some(), clear, alive, hz }) {
            Step::None => {}
            Step::Start => self.retime_start(now_ns, false),
            Step::Back(after) => self.retime_say(format!("retime: the panel is back at 120 Hz {:.1} s after the clip began", after as f64 / 1e9)),
            Step::Stop(why) => self.retime_stop(now_ns, why),
        }
    }

    /// Screen on, Home in front (the screen at rest also has no card), no video window that plays a hardware video, no claim for one and, when
    /// `read_source`, no source on the main window. The clip's own window and its source are not counted while it plays.
    fn panel_rate_clear(&mut self, read_source: bool) -> bool {
        let home = self.scene.foreground(&self.surfaces).is_none_or(|w| w.app_id() == crate::scene::HOME_APP);
        // a window with no claim holds no decoder (the Cameras app's feeder stays 120 s after its view, hidden and 16 by 16), and a texture is decoded on
        // the CPU: neither feeds the main window
        let video = self.scene.windows().any(|w| w.kind == WindowType::VideoWindow && w.vwin_live && !w.vwin_texture && w.app_id() != APP_ID)
            || self.vwin.alloc.granted().any(|c| !c.texture);
        if !self.luna_state.power.rendering || !home || video {
            return false;
        }
        if !read_source {
            return true;
        }
        let path = self.retime.scaler.clone();
        self.retime.read(&path).and_then(source_connected) != Some(true)
    }

    fn retime_say(&mut self, line: String) {
        self.retime.last = line.trim_start_matches("retime: ").to_owned();
        self.journal(line);
    }

    fn retime_start(&mut self, now_ns: u64, by_hand: bool) {
        let spec = Spec {
            program: self.retime.program.clone(),
            clip: self.config.runtime_dir.join(CLIP_FILE),
            stats: self.config.runtime_dir.join("retime-stats.jsonl"),
            runtime_dir: self.config.runtime_dir.clone(),
            socket: self.config.socket.clone(),
        };
        let started = if !spec.program.is_file() {
            Err(io::Error::new(io::ErrorKind::NotFound, format!("no player at {}", spec.program.display())))
        } else {
            write_clip(&spec.clip).and_then(|()| self.retime.player.start(&spec))
        };
        self.retime.exit.clear();
        match started {
            Ok(pid) if by_hand => self.retime_say(format!("retime: a {CLIP_FPS} fps clip plays under the UI by hand (player {pid})")),
            Ok(pid) => {
                let (n, hz) = (self.retime.machine.tries(), self.retime.last_hz.unwrap_or_default());
                self.retime_say(format!(
                    "retime: the panel is held at {hz:.0} Hz with nothing on the main window; a {CLIP_FPS} fps clip plays there under the UI to re-time it (player {pid}, try {n} of {MAX_TRIES})"
                ));
            }
            Err(e) if by_hand => {
                self.retime.machine.reset();
                self.retime_say(format!("retime: the clip could not be played by hand: {e}"));
            }
            Err(e) => {
                self.retime.machine.start_failed(now_ns);
                let next = self.retime_next();
                self.retime_say(format!("retime: the clip could not be played: {e}; {next}"));
            }
        }
        // the window the player makes is bound the moment it is there
        self.vwin_sync();
    }

    /// What happens after a try that did not take.
    fn retime_next(&self) -> String {
        match self.retime.machine.phase_name() {
            "spent" => "not tried again until the panel changes or Home is left".to_owned(),
            _ => format!("tried again in {} s", RETRY_AFTER / S),
        }
    }

    fn retime_stop(&mut self, now_ns: u64, why: Why) {
        let said = self.retime.player.said();
        let words = if said.is_empty() { String::new() } else { format!(" (it said: {said})") };
        let exit = if self.retime.exit.is_empty() { String::new() } else { format!(" ({})", self.retime.exit) };
        self.retime.player.stop(now_ns);
        // the window is hidden again at once
        self.vwin_sync();
        let line = match why {
            Why::Done => "retime: the clip is stopped".to_owned(),
            Why::Timeout => format!("retime: the clip did not move the panel in {} s{words}; {}", TRY_FOR / S, self.retime_next()),
            Why::Ended => format!("retime: the player ended{exit} before the panel moved{words}; {}", self.retime_next()),
            Why::Busy => "retime: the clip is stopped: Home left the front, a video or a claim took the main window, or the screen went off".to_owned(),
            Why::Off => "retime: the clip is stopped: switched off, or the refresh is set by hand".to_owned(),
        };
        self.retime_say(line);
    }

    /// `ctl retime`: the state; `on`, `off`, `show`, `hide`, `now` (play the clip for a few seconds, wherever the panel is) and `stop`.
    pub fn retime_ctl(&mut self, words: &[&str]) -> Value {
        let now = crate::output::monotonic_ns();
        match words {
            [] | ["status"] => {}
            ["on"] => self.retime.enabled = true,
            ["off"] => {
                self.retime.enabled = false;
                self.panel_rate_sync(now);
            }
            ["show"] => {
                self.retime.show = true;
                self.vwin_sync();
            }
            ["hide"] => {
                self.retime.show = false;
                self.vwin_sync();
            }
            ["now", ..] => {
                // `now SECONDS`: another time than the default (a second to ten)
                let secs = words.get(1).map(|w| w.parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.0));
                if secs == Some(None) {
                    return json!({"ok": false, "error": "retime now [SECONDS]"});
                }
                if self.retime.machine.playing() {
                    return json!({"ok": false, "error": "the clip is playing", "retime": self.retime_report()});
                }
                if !self.retime.enabled || !self.panel_rate_clear(true) {
                    return json!({"ok": false, "error": "the main window is not clear (Home in front, no video, no claim, the screen on), or the retime is off", "retime": self.retime_report()});
                }
                self.retime.machine.force(now, secs.flatten().map_or(FORCED_FOR, |s| (s * 1e9) as u64));
                self.retime_start(now, true);
            }
            ["stop"] => {
                if self.retime.machine.playing() {
                    self.retime.machine.reset();
                    self.retime_stop(now, Why::Off);
                }
            }
            _ => return json!({"ok": false, "error": "retime [status|on|off|show|hide|now [SECONDS]|stop]"}),
        }
        json!({"ok": true, "retime": self.retime_report()})
    }

    pub fn retime_report(&self) -> Value {
        let r = &self.retime;
        json!({
            "enabled": r.enabled,
            "show": r.show,
            "phase": r.machine.phase_name(),
            "tries": r.machine.tries(),
            "playing": r.machine.playing(),
            "pid": r.player.pid(),
            "player": r.program,
            "playerPresent": r.program.is_file(),
            "playerSaid": r.player.said(),
            "exit": r.exit,
            "panelHz": r.last_hz,
            "last": r.last,
            "app": APP_ID,
            "rect": [RECT.x, RECT.y, RECT.w, RECT.h],
            "clip": {"bytes": CLIP.len(), "size": CLIP_SIZE, "fps": CLIP_FPS},
        })
    }
}

/// Puts the clip where the player reads it (`path`), unless it is there already.
fn write_clip(path: &Path) -> io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() == CLIP.len() as u64) {
        return Ok(());
    }
    std::fs::write(path, CLIP)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = S;

    fn look(stuck: bool, clear: bool, alive: bool, hz: Option<f64>) -> Obs {
        Obs { stuck, clear, alive, hz }
    }

    /// The machine at `t` seconds, held and clear, with a reading `hz`.
    fn held(m: &mut Machine, t: u64, hz: Option<f64>) -> Step {
        m.step(t * T, look(true, true, false, hz))
    }

    #[test]
    fn the_rate_bands_and_the_two_files_read_as_the_set_writes_them() {
        assert_eq!((band(99.2), band(100.8), band(119.9), band(121.0), band(131.8), band(110.0)), (Some(100), Some(100), Some(120), Some(120), None, None));
        assert_eq!(source_connected("[main.p0]\nopen=0\nconnect_src=NONE\n"), Some(false));
        assert_eq!(source_connected("connect_src=VDEC"), Some(true));
        assert_eq!(source_connected("nothing"), None);
        // /proc/interrupts: the CPU columns are the numbers before the controller's name
        let line = " 68:     240000      12       0       3     GICv2 104 Edge      osd_irq\n 69:    5     GICv2 105 Edge      other\n";
        assert_eq!(osd_count(line), Some(240_015));
        assert_eq!(osd_count("69: 5 GICv2 Edge other"), None);
        assert_eq!(osd_count(""), None);
    }

    #[test]
    fn the_meter_reads_a_rate_once_it_has_two_counts_far_enough_apart() {
        let mut m = Meter::default();
        assert_eq!(m.feed(0, 1000), None, "the first count is a start");
        assert_eq!(m.feed(300 * MS, 1030), None, "too soon");
        let hz = m.feed(500 * MS, 1050).unwrap();
        assert_eq!(band(hz), Some(100), "{hz}");
        let hz = m.feed(1000 * MS, 1110).unwrap();
        assert_eq!(band(hz), Some(120), "the next reading starts where the last ended: {hz}");
        m.reset();
        assert_eq!(m.feed(2 * S, 5), None);
    }

    #[test]
    fn a_held_panel_with_a_clear_window_is_read_after_one_and_a_half_seconds_and_the_clip_starts_at_100_hz() {
        let mut m = Machine::default();
        assert_eq!(held(&mut m, 10, None), Step::None);
        assert_eq!(m.step(10 * T + CLEAR_FOR - 1, look(true, true, false, Some(100.0))), Step::None, "not before the window has been clear that long, whatever the panel reads");
        assert!(!m.playing() && m.wants_rate());
        // a reading is good for a second: the look at the end of the wait takes it
        assert_eq!(m.step(10 * T + CLEAR_FOR, look(true, true, false, None)), Step::Start);
        assert!(m.playing() && m.tries() == 1 && m.phase_name() == "playing");
        // with no reading it waits for one, and a rate in neither band is a switch under way
        let mut m = Machine::default();
        held(&mut m, 10, None);
        assert_eq!(m.step(10 * T + CLEAR_FOR, look(true, true, false, None)), Step::None);
        assert_eq!(m.step(10 * T + CLEAR_FOR + 100 * MS, look(true, true, false, Some(110.0))), Step::None);
        assert_eq!(m.step(10 * T + CLEAR_FOR + 500 * MS, look(true, true, false, Some(99.8))), Step::Start);
        // and a reading older than a second is no reading
        let mut m = Machine::default();
        held(&mut m, 10, Some(100.0));
        assert_eq!(m.step(10 * T + CLEAR_FOR, look(true, true, false, None)), Step::None);
    }

    #[test]
    fn flips_that_say_100_over_a_panel_that_reads_120_start_nothing_and_are_not_asked_again_for_a_while() {
        let mut m = Machine::default();
        held(&mut m, 0, None);
        assert_eq!(held(&mut m, 2, Some(120.2)), Step::None);
        assert!(!m.playing() && !m.wants_rate() && m.phase_name() == "rest");
        assert_eq!(held(&mut m, 30, Some(100.0)), Step::None, "inside the rest the reading is not even wanted");
        assert_eq!(held(&mut m, 32, None), Step::None);
        assert_eq!(held(&mut m, 34, Some(100.0)), Step::Start, "after it the wait starts over and a 100 Hz panel gets the clip");
        // the panel back at its rate forgets the episode
        let mut m = Machine::default();
        held(&mut m, 0, None);
        assert_eq!(held(&mut m, 2, Some(120.0)), Step::None);
        assert_eq!(m.step(3 * T, look(false, true, false, None)), Step::None);
        assert_eq!(m.phase_name(), "idle");
    }

    #[test]
    fn anything_else_on_the_main_window_ends_the_episode_and_the_wait_starts_over() {
        let mut m = Machine::default();
        held(&mut m, 0, None);
        // a video (or Home leaving, or the screen off) in the middle of the wait
        assert_eq!(m.step(T, look(true, false, false, None)), Step::None);
        assert_eq!(m.phase_name(), "idle");
        assert_eq!(held(&mut m, 2, Some(100.0)), Step::None, "the wait begins again: nothing is read into the past");
        assert_eq!(held(&mut m, 3, Some(100.0)), Step::None);
        assert_eq!(held(&mut m, 4, Some(100.0)), Step::Start);
        // and in the middle of the clip: it is ended, and the tries are the next episode's
        assert_eq!(m.step(5 * T, look(true, false, true, None)), Step::Stop(Why::Busy));
        assert!(!m.playing() && m.tries() == 0);
    }

    #[test]
    fn a_clip_that_works_is_held_a_second_and_then_stopped() {
        let mut m = Machine::default();
        held(&mut m, 0, None);
        assert_eq!(held(&mut m, 2, Some(100.0)), Step::Start);
        // 100 Hz is still the panel's rate a moment on: nothing happens
        assert_eq!(m.step(3 * T, look(true, true, true, Some(99.9))), Step::None);
        // 120 Hz is read (or the flips re-key to the mode's period)
        assert_eq!(m.step(3 * T + 500 * MS, look(true, true, true, Some(120.3))), Step::Back(T + 500 * MS));
        assert!(m.playing() && m.phase_name() == "holding");
        assert_eq!(m.step(4 * T, look(false, true, true, None)), Step::None);
        assert_eq!(m.step(4 * T + 500 * MS, look(false, true, true, None)), Step::Stop(Why::Done));
        assert!(!m.playing() && m.tries() == 0, "a panel that is back ends the episode");
        // the flips alone can say it, on a screen that draws
        let mut m = Machine::default();
        held(&mut m, 0, None);
        held(&mut m, 2, Some(100.0));
        assert_eq!(m.step(3 * T, look(false, true, true, None)), Step::Back(T));
    }

    #[test]
    fn a_clip_that_does_not_move_the_panel_is_given_up_after_eight_seconds_tried_again_once_and_then_left() {
        let mut m = Machine::default();
        held(&mut m, 0, None);
        assert_eq!(held(&mut m, 2, Some(100.0)), Step::Start);
        assert_eq!(m.step(2 * T + TRY_FOR - 1, look(true, true, true, Some(100.1))), Step::None);
        assert_eq!(m.step(2 * T + TRY_FOR, look(true, true, true, Some(100.1))), Step::Stop(Why::Timeout));
        assert!(!m.playing() && !m.wants_rate() && m.phase_name() == "rest");
        // a minute later it waits again, reads, and plays the second time
        let t = 2 * T + TRY_FOR + RETRY_AFTER;
        assert_eq!(m.step(t - T, look(true, true, false, None)), Step::None);
        assert_eq!(m.step(t, look(true, true, false, None)), Step::None);
        assert_eq!(m.step(t + T, look(true, true, false, Some(100.0))), Step::None, "the wait is a wait");
        assert_eq!(m.step(t + CLEAR_FOR, look(true, true, false, Some(100.0))), Step::Start);
        assert_eq!(m.tries(), 2);
        assert_eq!(m.step(t + CLEAR_FOR + TRY_FOR, look(true, true, true, Some(100.0))), Step::Stop(Why::Timeout));
        assert_eq!(m.phase_name(), "spent");
        // out of tries: nothing more however long the panel stays held, until something ends the episode
        for s in 100..160 {
            assert_eq!(held(&mut m, s * 10, Some(100.0)), Step::None);
        }
        assert_eq!(m.step(2000 * T, look(true, false, false, None)), Step::None);
        assert_eq!(held(&mut m, 2001, None), Step::None);
        assert_eq!(held(&mut m, 2003, Some(100.0)), Step::Start, "a new episode has its tries again");
    }

    #[test]
    fn a_player_that_ends_by_itself_before_the_panel_moves_is_a_try_that_did_not_take() {
        let mut m = Machine::default();
        held(&mut m, 0, None);
        assert_eq!(held(&mut m, 2, Some(100.0)), Step::Start);
        assert_eq!(m.step(3 * T, look(true, true, false, None)), Step::Stop(Why::Ended));
        assert_eq!(m.phase_name(), "rest");
        // a player that could not be started is the same
        let mut m = Machine::default();
        held(&mut m, 0, None);
        assert_eq!(held(&mut m, 2, Some(100.0)), Step::Start);
        m.start_failed(2 * T);
        assert_eq!(m.phase_name(), "rest");
        assert!(!m.playing());
    }

    #[test]
    fn a_clip_started_by_hand_plays_its_seconds_or_until_the_window_is_taken_whatever_the_panel_reads() {
        let mut m = Machine::default();
        assert_eq!(m.force(10 * T, FORCED_FOR), Step::Start);
        assert!(m.playing());
        assert_eq!(m.step(12 * T, look(false, true, true, Some(120.0))), Step::None);
        assert_eq!(m.step(14 * T, look(false, true, true, None)), Step::Stop(Why::Done));
        m.force(20 * T, FORCED_FOR);
        assert_eq!(m.step(21 * T, look(false, false, true, None)), Step::Stop(Why::Busy));
        // a time of its own, kept between a second and ten
        m.force(30 * T, 6 * T);
        assert_eq!(m.step(35 * T, look(false, true, true, None)), Step::None);
        assert_eq!(m.step(36 * T, look(false, true, true, None)), Step::Stop(Why::Done));
        m.force(40 * T, 600 * T);
        assert_eq!(m.step(49 * T, look(false, true, true, None)), Step::None);
        assert_eq!(m.step(50 * T, look(false, true, true, None)), Step::Stop(Why::Done), "ten seconds at most");
    }

    #[test]
    fn the_players_command_line_is_the_one_the_tv_rounds_ran_with_this_clip_in_a_video_window_for_home() {
        let spec = Spec {
            program: "/var/lib/lumatv/bin/lumatv-gfn-feeder".into(),
            clip: "/tmp/xdg/retime-30fps-640x360.h264".into(),
            stats: "/tmp/xdg/retime-stats.jsonl".into(),
            runtime_dir: "/tmp/xdg".into(),
            socket: "wayland-0".into(),
        };
        let args: Vec<String> = spec.args().into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args.join(" "),
            "--play /tmp/xdg/retime-30fps-640x360.h264 --codec h264 --size 640x360 --fps 30 --seconds 12 --loop -w video -a org.nphil.lumatv.retime \
             --ndl-app com.webos.app.home --stats /tmp/xdg/retime-stats.jsonl"
        );
    }

    /// The bits of an SPS NAL (emulation prevention removed), read with the Exp-Golomb codes the syntax uses.
    struct Bits<'a> {
        d: &'a [u8],
        at: usize,
    }

    impl Bits<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.d[self.at / 8] >> (7 - self.at % 8)) & 1;
            self.at += 1;
            u32::from(b)
        }
        fn u(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, _| v << 1 | self.bit())
        }
        fn ue(&mut self) -> u32 {
            let mut zeros = 0;
            while self.bit() == 0 {
                zeros += 1;
            }
            (1 << zeros) - 1 + self.u(zeros)
        }
        fn se(&mut self) -> i32 {
            let k = self.ue() as i32;
            if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) }
        }
    }

    /// `(width, height, num_units_in_tick, time_scale, fixed_frame_rate_flag)` of an SPS NAL's payload (after its header byte).
    fn sps_of(nal: &[u8]) -> (u32, u32, u32, u32, u32) {
        let mut rbsp = Vec::new();
        for (i, &b) in nal.iter().enumerate() {
            if !(i >= 2 && b == 3 && nal[i - 1] == 0 && nal[i - 2] == 0) {
                rbsp.push(b);
            }
        }
        let mut r = Bits { d: &rbsp, at: 0 };
        let profile = r.u(8);
        r.u(16);
        r.ue();
        assert_eq!(profile, 100, "High profile: the chroma format and scaling fields follow");
        assert_eq!(r.ue(), 1, "4:2:0");
        r.ue();
        r.ue();
        r.u(1);
        assert_eq!(r.u(1), 0, "no scaling matrices");
        r.ue();
        let poc = r.ue();
        if poc == 0 {
            r.ue();
        } else if poc == 1 {
            r.u(1);
            r.se();
            r.se();
            for _ in 0..r.ue() {
                r.se();
            }
        }
        r.ue();
        r.u(1);
        let (w_mbs, h_map) = (r.ue() + 1, r.ue() + 1);
        let frame_mbs_only = r.u(1);
        if frame_mbs_only == 0 {
            r.u(1);
        }
        r.u(1);
        let (mut cw, mut ch) = (0, 0);
        if r.u(1) == 1 {
            let (l, rt, t, b) = (r.ue(), r.ue(), r.ue(), r.ue());
            (cw, ch) = (2 * (l + rt), 2 * (t + b) * (2 - frame_mbs_only));
        }
        assert_eq!(r.u(1), 1, "VUI present");
        if r.u(1) == 1 {
            // aspect ratio
            if r.u(8) == 255 {
                r.u(32);
            }
        }
        if r.u(1) == 1 {
            r.u(1);
        }
        if r.u(1) == 1 {
            r.u(4);
            if r.u(1) == 1 {
                r.u(24);
            }
        }
        if r.u(1) == 1 {
            r.ue();
            r.ue();
        }
        assert_eq!(r.u(1), 1, "timing info present");
        let (ticks, scale, fixed) = (r.u(32), r.u(32), r.u(1));
        (w_mbs * 16 - cw, (2 - frame_mbs_only) * h_map * 16 - ch, ticks, scale, fixed)
    }

    #[test]
    fn the_clip_declares_640x360_at_30_pictures_a_second_and_every_picture_is_one_access_unit() {
        // the NALs: start codes, then the header byte's type
        let mut starts = CLIP.windows(3).enumerate().filter(|(_, w)| *w == [0, 0, 1]).map(|(i, _)| i).collect::<Vec<_>>();
        starts.push(CLIP.len());
        let nals: Vec<(u8, &[u8])> = starts.windows(2).map(|p| (CLIP[p[0] + 3] & 0x1f, &CLIP[p[0] + 4..p[1]])).collect();
        let types: Vec<u8> = nals.iter().map(|n| n.0).collect();
        assert_eq!(&types[..5], &[9, 7, 8, 5, 9], "an access unit delimiter, the parameter sets and the IDR, then the next picture's delimiter");
        assert_eq!(types.iter().filter(|&&t| t == 9).count(), 30, "30 pictures");
        assert_eq!(types.iter().filter(|&&t| t == 5).count(), 1, "one IDR: the loop is a restart of the stream");
        assert_eq!(types.iter().filter(|&&t| t == 1).count(), 29);
        assert!(!types.contains(&6), "no SEI: the encoder's own words are not shipped");
        let sps = nals.iter().find(|n| n.0 == 7).unwrap().1;
        // the NAL's payload with its trailing zero bytes cut (the next start code's)
        let (w, h, ticks, scale, fixed) = sps_of(sps);
        assert_eq!((w, h), (640, 360));
        assert_eq!(f64::from(scale) / f64::from(ticks) / 2.0, 30.0, "the SPS says 30 pictures a second, which is the rate the display engine is told");
        assert_eq!(fixed, 1);
    }

    #[test]
    fn the_clip_is_written_once_where_the_player_reads_it() {
        let dir = std::env::temp_dir().join(format!("lumatv-retime-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CLIP_FILE);
        write_clip(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), CLIP);
        // a file of the right size is left alone
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_clip(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- the glue, on a State whose panel the test drives ----

    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::output::{Image, Output, Presented, RenderStats};
    use crate::render::software::Software;
    use crate::scene::Plan;
    use crate::surface::{SurfaceId, Surfaces};
    use crate::vwin::Mode;
    use wayland_server::protocol::wl_buffer::WlBuffer;

    /// The software output, saying what the flips measure.
    struct Flips {
        inner: Software,
        period: Arc<AtomicU64>,
    }

    impl Output for Flips {
        fn name(&self) -> &'static str {
            "fake-flips"
        }
        fn size(&self) -> (u32, u32) {
            self.inner.size()
        }
        fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
            self.inner.query_buffer(buffer)
        }
        fn forget_buffer(&mut self, buffer: &WlBuffer) {
            self.inner.forget_buffer(buffer)
        }
        fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
            self.inner.render(plan, surfaces)
        }
        fn present(&mut self) -> io::Result<Presented> {
            self.inner.present()
        }
        fn capture(&mut self) -> io::Result<Image> {
            self.inner.capture()
        }
        fn flip_period_ns(&self) -> Option<u64> {
            Some(self.period.load(Ordering::Relaxed))
        }
    }

    /// A feeder that plays until it is ended.
    const PLAYS: &str = "#!/bin/sh\nexec sleep 30\n";

    /// A compositor state on a fake clock: the 60 Hz mode's flips measure what the test says, the interrupt table and the scaler's status are files in a
    /// directory of its own, and the player is a script there (or none).
    struct Rig {
        state: crate::State,
        _display: wayland_server::Display<crate::State>,
        dir: PathBuf,
        period: Arc<AtomicU64>,
        count: u64,
        t: u64,
    }

    impl Rig {
        fn new(name: &str, feeder: Option<&str>) -> Rig {
            let dir = std::env::temp_dir().join(format!("lumatv-retime-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("scaler"), "[main.p0]\nconnect_src=NONE\n").unwrap();
            if let Some(script) = feeder {
                use std::os::unix::fs::PermissionsExt;
                let program = dir.join("feeder");
                std::fs::write(&program, script).unwrap();
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let period = Arc::new(AtomicU64::new(period_ns(60)));
            let display: wayland_server::Display<crate::State> = wayland_server::Display::new().unwrap();
            let config = crate::config::Config { runtime_dir: dir.clone(), ..crate::config::Config::default() };
            let output = Flips { inner: Software::new(64, 64), period: period.clone() };
            let mut state = crate::State::new(display.handle(), config, Box::new(output), Box::new(crate::luna::Null::default()));
            state.retime.program = dir.join("feeder");
            state.retime.scaler = dir.join("scaler");
            state.retime.interrupts = dir.join("interrupts");
            let rig = Rig { state, _display: display, dir, period, count: 1_000_000, t: S };
            rig.write_count();
            rig
        }

        fn write_count(&self) {
            std::fs::write(self.dir.join("interrupts"), format!(" 68: {:>10} 0 0 0 GICv2 104 Edge osd_irq\n", self.count)).unwrap();
        }

        /// The flips of a panel held at 100 Hz (20 ms apart under the 60 Hz mode), and of one back at its rate.
        fn hold(&self) {
            self.period.store(20_000_000, Ordering::Relaxed);
        }

        fn release(&self) {
            self.period.store(period_ns(60), Ordering::Relaxed);
        }

        /// A look every 100 ms of the fake clock for `ms`, on a panel whose OSD interrupt comes `hz` times a second.
        fn run(&mut self, ms: u64, hz: u64) {
            for _ in 0..ms / 100 {
                self.t += 100 * MS;
                self.count += hz / 10;
                self.write_count();
                self.state.panel_rate_sync(self.t);
            }
        }

        /// Looks until the player has been started.
        fn until_started(&mut self, hz: u64) {
            while self.state.retime.player.pid().is_none() {
                self.run(100, hz);
                assert!(self.t < 60 * S, "no player in a minute: {:?}", self.state.journal);
            }
        }

        fn said(&self, what: &str) -> bool {
            self.state.journal.iter().any(|l| l.contains(what))
        }

        /// Waits (real time, a few milliseconds) for the players told to end to be gone.
        fn reaped(&mut self) -> bool {
            for _ in 0..400 {
                self.state.retime.player.reap(self.t);
                if !self.state.retime.player.any() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Waits for a child that has ended on its own to be a zombie (ended, not yet collected).
    fn ended(pid: u32) {
        for _ in 0..400 {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            if stat.rsplit_once(") ").is_none_or(|(_, rest)| rest.starts_with('Z')) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("process {pid} did not end");
    }

    fn video_window(state: &mut crate::State, surface: u32, app: &str, live: bool, texture: bool) -> crate::window::WindowId {
        let id = state.scene.create_window(SurfaceId(surface));
        let w = state.scene.window_mut(id).unwrap();
        w.kind = WindowType::VideoWindow;
        w.props.insert("appId".into(), app.into());
        w.map_serial = 1;
        w.vwin_live = live;
        w.vwin_texture = texture;
        id
    }

    #[test]
    fn a_panel_held_at_100_hz_gets_the_clip_after_a_second_and_a_half_and_it_ends_a_second_after_the_panel_is_back() {
        let mut r = Rig::new("episode", Some(PLAYS));
        r.hold();
        r.run(1_400, 100);
        assert!(r.state.retime.player.pid().is_none(), "not before the main window has been clear for a second and a half");
        r.until_started(100);
        let pid = r.state.retime.player.pid().unwrap();
        assert!(r.said("retime: the panel is held at 100 Hz with nothing on the main window"), "{:?}", r.state.journal);
        assert!(r.state.retime.machine.playing());
        // the panel follows the clip, and the flips say so
        r.release();
        r.run(300, 120);
        assert!(r.said("retime: the panel is back at 120 Hz"), "{:?}", r.state.journal);
        assert_eq!(r.state.retime.player.pid(), Some(pid), "the clip plays on for its hold");
        r.run(1_200, 120);
        assert!(r.state.retime.player.pid().is_none() && !r.state.retime.machine.playing());
        assert!(r.reaped(), "the player is collected");
        assert!(r.said("retime: the clip is stopped"), "{:?}", r.state.journal);
        // the episode is over: a panel at its rate starts nothing
        r.run(5_000, 120);
        assert!(r.state.retime.player.pid().is_none());
    }

    #[test]
    fn a_home_at_rest_makes_no_flips_and_the_interrupt_alone_says_the_panel_is_back() {
        let mut r = Rig::new("rest", Some(PLAYS));
        r.hold();
        r.until_started(100);
        // the flips still say 100 Hz (nothing is drawn to say otherwise); the OSD interrupt says 120
        r.run(1_500, 120);
        assert!(r.said("retime: the panel is back at 120 Hz"), "{:?}", r.state.journal);
        assert!(r.state.retime.machine.playing(), "held for its second");
    }

    #[test]
    fn flips_that_still_say_100_over_a_panel_that_reads_120_play_nothing_and_say_nothing() {
        let mut r = Rig::new("stale", Some(PLAYS));
        r.hold();
        r.run(4_000, 120);
        assert!(r.state.retime.player.pid().is_none());
        assert!(r.state.journal.iter().all(|l| !l.starts_with("retime:")), "{:?}", r.state.journal);
        assert_eq!(r.state.retime.machine.phase_name(), "rest");
    }

    #[test]
    fn a_window_with_no_claim_does_not_hold_the_clear_but_one_that_plays_a_hardware_video_does() {
        let mut r = Rig::new("clear", Some(PLAYS));
        // the Cameras app's feeder stays 120 s after its view: its 16 by 16 window is mapped, was bound once and is live no more
        video_window(&mut r.state, 40, "org.nphil.lumatv.cameras", false, false);
        assert!(r.state.panel_rate_clear(true), "the window the Ring's view left behind held the clear for 120 s");
        // a texture is decoded on the CPU: no hardware video, whatever the window
        video_window(&mut r.state, 41, "org.nphil.lumatv.popup", true, true);
        assert!(r.state.panel_rate_clear(true));
        // the clip's own window, placed under the UI or shown
        let own = video_window(&mut r.state, 42, APP_ID, true, false);
        assert!(r.state.panel_rate_clear(true));
        r.state.scene.window_mut(own).unwrap().vwin_live = false;
        // a window with a granted hardware claim does
        let playing = video_window(&mut r.state, 43, "org.nphil.lumatv.cameras2", true, false);
        assert!(!r.state.panel_rate_clear(true));
        r.state.scene.window_mut(playing).unwrap().vwin_live = false;
        assert!(r.state.panel_rate_clear(true));
        // so does a claim that has no window yet
        r.state.vwin_set_mode(Mode::Apps);
        let reply = r.state.vwin_command(&json!({"op": "claim", "id": "c", "kind": "camera-popup", "rect": {"x": 1300, "y": 60, "w": 560, "h": 315}}));
        assert_eq!(reply["claim"]["state"], "granted", "{reply}");
        assert!(!r.state.panel_rate_clear(true));
        r.state.vwin_command(&json!({"op": "release", "id": "c"}));
        assert!(r.state.panel_rate_clear(true));
        // a source on the main window, which only the scaler's status says, holds it until the clip itself is the source
        std::fs::write(r.dir.join("scaler"), "[main.p0]\nconnect_src=VDEC\n").unwrap();
        assert!(!r.state.panel_rate_clear(true));
        assert!(r.state.panel_rate_clear(false), "the clip's own source is not counted while it plays");
        std::fs::write(r.dir.join("scaler"), "[main.p0]\nconnect_src=NONE\n").unwrap();
        // and the screen being off does
        r.state.luna_state.power.rendering = false;
        assert!(!r.state.panel_rate_clear(true));
    }

    #[test]
    fn the_cameras_app_s_idle_window_does_not_keep_the_clip_back_after_a_view() {
        // the TV session of 2026-11-04: Back at srv 32360, the clear at 32480 (the window's 120 s), the panel at 100 Hz all that time
        let mut r = Rig::new("ring", Some(PLAYS));
        video_window(&mut r.state, 40, "org.nphil.lumatv.cameras", false, false);
        r.hold();
        r.until_started(100);
        assert!(r.t < 5 * S, "the clip started {:.1} s in, not 120 s", r.t as f64 / 1e9);
    }

    #[test]
    fn the_clip_is_ended_when_the_screen_goes_off_a_claim_takes_the_window_or_the_refresh_is_set_by_hand() {
        type Interrupt = fn(&mut Rig);
        let cases: [(&str, Interrupt); 3] = [
            ("off", |r| r.state.luna_state.power.rendering = false),
            ("claim", |r| {
                r.state.vwin_set_mode(Mode::Apps);
                let reply = r.state.vwin_command(&json!({"op": "claim", "id": "c", "kind": "camera-popup", "rect": {"x": 1300, "y": 60, "w": 560, "h": 315}}));
                assert_eq!(reply["claim"]["state"], "granted", "{reply}");
            }),
            ("hand", |r| r.state.refresh_policy.manual = true),
        ];
        for (name, interrupt) in cases {
            let mut r = Rig::new(name, Some(PLAYS));
            r.hold();
            r.until_started(100);
            interrupt(&mut r);
            r.run(200, 100);
            assert!(r.state.retime.player.pid().is_none() && !r.state.retime.machine.playing(), "{name}: the clip is ended at the next look");
            assert!(r.reaped(), "{name}");
            assert!(r.said("retime: the clip is stopped"), "{name}: {:?}", r.state.journal);
        }
    }

    #[test]
    fn with_no_player_installed_nothing_plays_and_the_reason_is_logged_twice_and_then_not_again() {
        let mut r = Rig::new("noplayer", None);
        r.hold();
        r.run(3_000, 100);
        assert!(r.said("retime: the clip could not be played: no player at"), "{:?}", r.state.journal);
        assert!(r.said("tried again in 60 s"));
        assert!(r.state.retime.player.pid().is_none());
        // a minute on, the second try; then the panel is left alone until something changes
        r.run(64_000, 100);
        let tries = |r: &Rig| r.state.journal.iter().filter(|l| l.contains("the clip could not be played")).count();
        assert_eq!(tries(&r), 2, "{:?}", r.state.journal);
        assert!(r.said("not tried again until the panel changes or Home is left"));
        r.run(300_000, 100);
        assert_eq!(tries(&r), 2, "five minutes later still two");
        // the panel changing (a video, a retime by anything) forgets the episode
        r.release();
        r.run(300, 120);
        r.hold();
        r.run(3_000, 100);
        assert_eq!(tries(&r), 3, "a new episode has its tries again");
    }

    #[test]
    fn a_player_that_ends_before_the_panel_moves_is_logged_with_how_it_ended_and_what_it_said() {
        let mut r = Rig::new("early", Some("#!/bin/sh\necho 'pipeline: NDL_DirectVideoOpen failed' >&2\nexit 4\n"));
        r.hold();
        r.until_started(100);
        ended(r.state.retime.player.pid().unwrap());
        // its last words reach the journal's line through a thread of their own
        for _ in 0..400 {
            if !r.state.retime.player.said().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        r.run(200, 100);
        assert!(
            r.said("retime: the player ended (exit status: 4) before the panel moved (it said: pipeline: NDL_DirectVideoOpen failed); tried again in 60 s"),
            "{:?}",
            r.state.journal
        );
        assert!(r.state.retime.player.pid().is_none() && !r.state.retime.machine.playing());
    }

    #[test]
    fn a_clip_that_does_not_move_the_panel_is_ended_after_eight_seconds_with_the_reason_in_the_journal() {
        let mut r = Rig::new("timeout", Some(PLAYS));
        r.hold();
        r.until_started(100);
        r.run(7_500, 100);
        assert!(r.state.retime.machine.playing(), "still within its eight seconds");
        r.run(1_000, 100);
        assert!(!r.state.retime.machine.playing());
        assert!(r.said("retime: the clip did not move the panel in 8 s; tried again in 60 s"), "{:?}", r.state.journal);
        assert!(r.reaped());
    }

    #[test]
    fn ctl_retime_plays_the_clip_by_hand_wherever_the_panel_is_and_stops_it() {
        let mut r = Rig::new("ctl", Some(PLAYS));
        let status = r.state.retime_ctl(&[]);
        assert_eq!((status["ok"].as_bool(), status["retime"]["playing"].as_bool(), status["retime"]["playerPresent"].as_bool()), (Some(true), Some(false), Some(true)));
        let now = r.state.retime_ctl(&["now"]);
        assert_eq!(now["ok"], true, "{now}");
        assert_eq!(now["retime"]["playing"], true);
        assert!(now["retime"]["pid"].as_u64().is_some());
        assert_eq!(r.state.retime_ctl(&["now"])["ok"], false, "one at a time");
        let stopped = r.state.retime_ctl(&["stop"]);
        assert_eq!(stopped["retime"]["playing"], false);
        assert!(r.reaped());
        // a time of its own, or a word that is none
        assert_eq!(r.state.retime_ctl(&["now", "soon"])["ok"], false);
        assert!(!r.state.retime.machine.playing(), "a time that is none starts nothing");
        let timed = r.state.retime_ctl(&["now", "6"]);
        assert_eq!(timed["ok"], true, "{timed}");
        r.state.retime_ctl(&["stop"]);
        assert!(r.reaped());
        // with no player the reason is said and the automatic retime is as it was
        std::fs::remove_file(r.dir.join("feeder")).unwrap();
        let none = r.state.retime_ctl(&["now"]);
        assert_eq!(none["retime"]["phase"], "idle");
        assert!(r.said("retime: the clip could not be played by hand: no player at"), "{:?}", r.state.journal);
        assert_eq!(r.state.retime_ctl(&["bogus"])["ok"], false);
        assert_eq!(r.state.retime_ctl(&["off"])["retime"]["enabled"], false);
        assert_eq!(r.state.retime_ctl(&["now"])["ok"], false, "off is off");
    }

    #[test]
    fn the_clips_window_is_placed_under_the_ui_while_it_plays_and_no_claim_may_take_it() {
        let mut r = Rig::new("window", Some(PLAYS));
        let id = video_window(&mut r.state, 40, APP_ID, false, false);
        r.state.vwin_sync();
        let w = r.state.scene.window(id).unwrap();
        assert!(!w.vwin_live && !w.vwin_quiet && w.vwin_rect.is_empty() && w.vwin_bound, "bound (never closed as unclaimed) and hidden");
        // a claim for the first window without one does not take it
        r.state.vwin_set_mode(Mode::Apps);
        let reply = r.state.vwin_command(&json!({"op": "claim", "id": "c", "kind": "camera-popup", "rect": {"x": 1300, "y": 60, "w": 560, "h": 315}}));
        assert_eq!(reply["claim"]["state"], "granted", "{reply}");
        r.state.vwin_sync();
        assert!(!r.state.scene.window(id).unwrap().vwin_live);
        assert_eq!(r.state.vwin.alloc.claim("c").unwrap().window, None);
        r.state.vwin_command(&json!({"op": "release", "id": "c"}));
        // the clip plays: the window is placed at the clip's rectangle, out of the stack, and no hole is cut for it
        assert_eq!(r.state.retime_ctl(&["now"])["ok"], true);
        let w = r.state.scene.window(id).unwrap();
        assert!(w.vwin_quiet && !w.vwin_live && w.vwin_rect == RECT, "{:?}", (w.vwin_quiet, w.vwin_live, w.vwin_rect));
        r.state.retime_ctl(&["show"]);
        let w = r.state.scene.window(id).unwrap();
        assert!(w.vwin_live && !w.vwin_quiet && w.vwin_rect == RECT, "shown, it is a live window with a hole");
        r.state.retime_ctl(&["hide"]);
        assert!(r.state.scene.window(id).unwrap().vwin_quiet);
        r.state.retime_ctl(&["stop"]);
        let w = r.state.scene.window(id).unwrap();
        assert!(!w.vwin_quiet && !w.vwin_live && w.vwin_rect.is_empty(), "hidden again");
        assert!(r.reaped());
    }
}
