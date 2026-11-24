//! The panel at 120 Hz whatever is in front: when a video has left it at another rate and nothing feeds the main window, a clip of 30 pictures a second plays
//! there, out of sight, until the panel is back.
//!
//! **What sets the rate.** LG's kernel picks the display engine's output rate from the rate of the source on window 0, at a timing change of that window and
//! at nothing else: `_vsc_check_fr` gives 50 (100 Hz on this panel) for 25 and 50 fps and 60 (120 Hz) for everything else, and the window with no source
//! sets nothing (`v4l2_vsc_event.c`, `default: return`), so the rate a video left stays until the next video's first frame (`framerate-result.md`).
//!
//! **What left it off its rate.** A 25 or 50 fps video (the Ring's stream, 100 Hz), a 71 or 72 fps source declared through the starfish pipe (144 Hz), and a Dolby
//! Vision film at 23.976 fps: Marquee's "Backrooms" (HEVC 3840x2160, profile 7) ran the OSD interrupt (`osd_irq`) at 95.5 a second while the display engine's
//! vsync (`DE2VSYNC`) read 119.4, and with the film closed and nothing on the main window both read 95.5 at Home until another source connected (TV,
//! 2026-11-23). The same film as plain H.264, H.265 or HDR10 H.265 through the starfish, ES and NDL pipelines left 119.5 Hz in every run. A rate the panel
//! holds at Home or in an app after any video is the same fault. Each of 96, 100 and 144 Hz stays until the next video's first frame (`decoded-vrr-4k.md`).
//!
//! **What does not work.** The same request through `/proc/lg/de/fr_rate` (`1 60`, `0 60`, `0 50`, `1 50`) was ignored every time it was made (no source,
//! and with the Ring's 25 fps stream connected). It is not made here.
//!
//! **What does.** A source's first frame at another rate. Every return to 120 Hz on the set was one: YouTube at 30 and 60 fps, a camera pop-up at 20 fps and
//! Live TV's video. So the way back is the feeder's way there with another rate: [`CLIP`], black and 830 bytes, and `lumatv-gfn-feeder --play --pipe es` loops
//! it in a video window of its own, the decoder opened for the app in front. The ES pipe presents 60 pictures a second to the display engine whatever the clip
//! declares (`vFreq 600`, 2026-11-24), a rate of the 120 Hz family. On the set the panel was back at 119.4 Hz 1.7 s after the feeder started, from 95.5 Hz at Home
//! after the film (2026-11-23), and 144 Hz went to 120 in about 0.6 s at the next clip; the ES pipe leaves the picture mode alone (NDL declares game mode).
//!
//! **Out of sight, and not small.** The window is the whole output and no hole is cut for it (`Window::vwin_quiet`): the video plane is under every pixel of the
//! UI, which is opaque there, so nothing of it shows. A small window would show nothing either, and is wrong: the TV's picture controller (pqcontroller) treats a
//! small video window on the main sink as picture-in-graphics and applies colour gamut 0 and local dimming 0 where the owner's Filmmaker Mode has 1 and 1
//! (`_pq_setAdvCtrlMenu`, TV 2026-10-30). A video of the whole screen, as LG sends it (`fullScreen`), keeps the values. If the decoder does not connect for a
//! window under the UI, `ctl retime show` cuts a hole for a small one.
//!
//! **When.** Whatever the foreground. Anything on the main window (a source the scaler reports, a live video window, a hardware claim) or the screen being
//! off makes the rate after it unknown: the machine is *suspect*. Once the window is clear the rate is read from the OSD interrupt over 300 ms; at its rate
//! (or above) nothing is played and nothing is said; outside 115 to 125 Hz the clip plays, and the player is ended when the panel reads back, after a short hold, or at
//! once when anything takes the window again. Under Home the clip starts as soon as the reading is in; under an app 1.5 s later, so that a video the app is
//! about to open finds the window free. Two tries an episode, 20 s apart; an episode ends with the panel back or anything else taking the window. One look a
//! minute at rest is the backstop for a change nothing announced. Every switch is one line in the journal (`retime:`), with the player's last words when it fails.
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
use crate::scene::HOME_APP;
use crate::window::WindowType;

/// The clip: 640x360, black, one IDR and 29 skipped pictures, the SPS timing `1/60 s` a tick with two ticks a picture (30 a second, fixed), one
/// reference, no B pictures, an access unit delimiter before each picture and the parameter sets repeated at the IDR. The ES pipe presents 60 pictures a second
/// to the display engine whatever the SPS says (`vFreq 600`), so the declared 30 is not what sets the rate. Made with
/// `ffmpeg -f lavfi -i color=c=black:size=640x360:rate=30 -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -preset veryslow
/// -x264-params aud=1:repeat-headers=1:slices=1:bframes=0:keyint=30:scenecut=0:ref=1:force-cfr=1 -bsf:v filter_units=remove_types=6 -f h264 retime-30fps-640x360.h264`.
pub const CLIP: &[u8] = include_bytes!("retime/retime-30fps-640x360.h264");
pub const CLIP_FILE: &str = "retime-30fps-640x360.h264";
const CLIP_SIZE: &str = "640x360";
const CLIP_FPS: &str = "30";

/// The app id of the clip's video window: the compositor places it by this id and no claim may take it.
pub const APP_ID: &str = "org.nphil.lumatv.retime";
/// LumaTV's own windows (Settings, the cameras, the pop-ups) are not on SAM's foreground list, which videooutputd connects a video by: the decoder is opened for
/// Home under them, and for the app in front under any other.
const OWN_APPS: &str = "org.nphil.lumatv";
/// Where the clip is placed, in output pixels: the whole output (see the module doc: a smaller window changes the picture).
pub const RECT: Rect = Rect::new(0, 0, crate::OUTPUT_WIDTH as i32, crate::OUTPUT_HEIGHT as i32);
/// Where `ctl retime show` places it, with a hole, to see it: 128 by 72 at 16,16 in the UI's space.
pub const SHOW_RECT: Rect = Rect::new(16, 16, 128, 72);
/// How long the player runs if nobody ends it first.
const PLAYER_SECONDS: u32 = 12;

/// The scaler's status for the main window (`connect_src=NONE` when no video feeds it) and the interrupt table that holds `osd_irq`.
pub const SCALER_PATH: &str = "/proc/lgtv-driver/scaler/main.p0";
pub const INTERRUPTS_PATH: &str = "/proc/interrupts";
/// The feeder, where `lumatv` stages it beside the helper's.
pub const FEEDER_PATH: &str = "bin/lumatv-gfn-feeder";

const MS: u64 = 1_000_000;
const S: u64 = 1_000 * MS;
/// How long the main window must stay clear before the clip starts, with the panel read low: at Home nothing opens a video by itself (the reading takes that long),
/// under an app its next video may be on its way (a next episode, a trailer) and must find the window free.
const SETTLE_HOME: u64 = 300 * MS;
const SETTLE_APP: u64 = 1_500 * MS;
/// How long the clip may play before it is given up: the Ring's stream took 1.7 s from its decoder's connection to the re-time.
const TRY_FOR: u64 = 8 * S;
/// How long the clip plays on once the panel is back.
const HOLD: u64 = 500 * MS;
/// How long a clip started by hand plays unless `ctl retime now SECONDS` asks for another time (the player ends itself at [`PLAYER_SECONDS`] in any case).
const FORCED_FOR: u64 = 4 * S;
const FORCED_MAX: u64 = 10 * S;
/// The wait before a second try.
const RETRY_AFTER: u64 = 20 * S;
const MAX_TRIES: u32 = 2;
/// The look a minute at rest for a change nothing announced.
const BACKSTOP: u64 = 60 * S;
/// A reading of the rate counts this long.
const FRESH: u64 = S;
/// How often the main window is looked at and the interrupt counted, and the least time a rate is read over: 36 interrupts at 120 Hz (117 to 123 read), 29 at
/// 95.5, 30 at 100.
const LOOK_EVERY: u64 = 100 * MS;
const MIN_SPAN: u64 = 300 * MS;
/// How long a player that was asked to end has before it is killed.
const KILL_AFTER: u64 = 2 * S;
/// A measured rate outside this band is a panel off its rate: 95.5 (a Dolby Vision film at 23.976 fps), 100 (25 and 50 fps) and 144 Hz (a 71 or 72 fps
/// source) are the ones seen. The 23.976 fps output is phase-locked at 119.88 and the display engine's own count reads 119.3 for a second after a re-time.
const RATE_BAND_HZ: std::ops::RangeInclusive<f64> = 115.0..=125.0;

pub fn off_rate(hz: f64) -> bool {
    !RATE_BAND_HZ.contains(&hz)
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
    /// Nothing to do: the panel is known to be at its rate, or the main window is not clear.
    #[default]
    Idle,
    /// The rate is unknown and the main window is clear since: the rate is read, and the clip starts when it is low.
    Clear(u64),
    /// The clip has played since.
    Playing(u64),
    /// The panel is back (or the clip was started by hand): the clip plays on until.
    Holding(u64),
    /// No clip before then: a try that did not take. `u64::MAX` is out of tries.
    Rest(u64),
}

/// What one look at the screen says.
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    /// Screen on and nothing but the clip feeds the main window.
    pub clear: bool,
    /// The player runs (false while there is none).
    pub alive: bool,
    /// A reading of the panel's rate taken at this look, if one was completed.
    pub hz: Option<f64>,
    /// How long the window must stay clear before the clip starts: [`SETTLE_HOME`] or [`SETTLE_APP`].
    pub settle: u64,
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
    /// A video or a claim took the main window, or the screen went off.
    Busy,
    /// Switched off, or the refresh was set by hand.
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    None,
    /// Start the player.
    Start,
    /// The panel is back at its rate this long after the clip began; the clip plays on for its hold.
    Back(u64),
    /// End the player.
    Stop(Why),
}

/// The decision. The rate is *suspect* from the start and whenever anything else has had the main window or the screen was off; it is read when the window is
/// clear, and the clip plays only for a reading below the rate. A look finding the panel at its rate clears the suspicion until the next such event (or the
/// backstop's minute).
#[derive(Debug)]
pub struct Machine {
    phase: Phase,
    tries: u32,
    suspect: bool,
    reading: Option<(u64, f64)>,
    /// When the window cleared for the look in hand, and when the panel was last found at its rate.
    since: u64,
    checked: u64,
}

impl Default for Machine {
    fn default() -> Self {
        Machine { phase: Phase::Idle, tries: 0, suspect: true, reading: None, since: 0, checked: 0 }
    }
}

impl Machine {
    /// Whether the clip plays (or is meant to).
    pub fn playing(&self) -> bool {
        matches!(self.phase, Phase::Playing(_) | Phase::Holding(_))
    }

    /// Whether the machine needs a reading of the panel's rate at the next look: while a look is due or the clip plays, not at rest (an interrupt table
    /// read a pass costs).
    pub fn wants_rate(&self) -> bool {
        matches!(self.phase, Phase::Clear(_) | Phase::Playing(_)) || (self.suspect && self.phase == Phase::Idle)
    }

    pub fn suspect(&self) -> bool {
        self.suspect
    }

    pub fn tries(&self) -> u32 {
        self.tries
    }

    /// When the main window cleared for the look in hand.
    pub fn since(&self) -> u64 {
        self.since
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

    /// Back to no episode (the switch went off, or the refresh is set by hand); what is suspected stays.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.tries = 0;
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

    /// The panel is at its rate: nothing is suspected until something takes the window.
    fn found_back(&mut self, now_ns: u64) {
        self.suspect = false;
        self.checked = now_ns;
        self.reset();
    }

    fn read_since(&self, now_ns: u64, since: u64) -> Option<f64> {
        self.reading.filter(|(at, _)| *at >= since && now_ns.saturating_sub(*at) <= FRESH).map(|(_, hz)| hz)
    }

    /// What to do at `now_ns` given what the screen says.
    pub fn step(&mut self, now_ns: u64, o: Obs) -> Step {
        if let Some(hz) = o.hz {
            self.reading = Some((now_ns, hz));
        }
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
                if self.read_since(now_ns, since).is_some_and(|hz| !off_rate(hz)) {
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
                    self.found_back(now_ns);
                    return Step::Stop(Why::Busy);
                }
                if !o.alive || now_ns >= until {
                    self.found_back(now_ns);
                    return Step::Stop(Why::Done);
                }
                Step::None
            }
            _ => self.look(now_ns, o),
        }
    }

    /// The look at rest: is the rate suspect, the window clear, the panel low.
    fn look(&mut self, now_ns: u64, o: Obs) -> Step {
        if !o.clear {
            // what the panel runs at after whatever has the window (or the screen being off) is not known
            self.suspect = true;
            self.reset();
            return Step::None;
        }
        if !self.suspect && now_ns.saturating_sub(self.checked) >= BACKSTOP {
            self.suspect = true;
        }
        if !self.suspect {
            return Step::None;
        }
        match self.phase {
            Phase::Idle => {
                self.phase = Phase::Clear(now_ns);
                self.since = now_ns;
            }
            Phase::Clear(since) => match self.read_since(now_ns, since) {
                Some(hz) if !off_rate(hz) => self.found_back(now_ns),
                Some(_) if now_ns.saturating_sub(since) >= o.settle => {
                    self.tries += 1;
                    self.phase = Phase::Playing(now_ns);
                    return Step::Start;
                }
                _ => {}
            },
            Phase::Rest(until) => {
                if now_ns >= until {
                    self.phase = Phase::Clear(now_ns);
                    self.since = now_ns;
                }
            }
            Phase::Playing(_) | Phase::Holding(_) => {}
        }
        Step::None
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
    /// The app the decoder is opened for.
    pub app: String,
}

impl Spec {
    /// The feeder's command line: `tools/tv-hwplanes.sh feeder`'s, which ran on the set, with the clip looped in a window of the video kind, through the ES pipe
    /// (the NDL one declares game mode, which flips the picture mode) and the decoder opened for the app in front.
    pub fn args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = ["--play"].map(OsString::from).into();
        a.push(self.clip.clone().into_os_string());
        for w in ["--codec", "h264", "--size", CLIP_SIZE, "--fps", CLIP_FPS, "--seconds"] {
            a.push(w.into());
        }
        a.push(PLAYER_SECONDS.to_string().into());
        for w in ["--loop", "-w", "video", "-a", APP_ID, "--pipe", "es", "--ndl-app", self.app.as_str(), "--stats"] {
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
    /// `ctl retime on|off`, `--no-retime`; on unless told otherwise.
    pub enabled: bool,
    /// `ctl retime show`: the clip's window gets its hole, to see where it is (and for a decoder that does not connect under the UI).
    pub show: bool,
    pub machine: Machine,
    meter: Meter,
    pub player: Player,
    /// The last time the main window was looked at, and the interrupt counted.
    looked_ns: u64,
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
            looked_ns: 0,
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

    /// The panel's rate, when a reading is complete at `now_ns` (the interrupt is counted every [`LOOK_EVERY`]).
    fn sample(&mut self, now_ns: u64) -> Option<f64> {
        if now_ns.saturating_sub(self.sampled_ns) < LOOK_EVERY {
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
    /// Keeps the panel at 120 Hz (`retime`). Each pass of the frame loop; at rest it looks at the main window every [`LOOK_EVERY`] and reads the interrupt
    /// only while the rate is suspect.
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
        if !playing && now_ns.saturating_sub(self.retime.looked_ns) < LOOK_EVERY {
            return;
        }
        self.retime.looked_ns = now_ns;
        let clear = self.panel_rate_clear(!playing);
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
        let settle = if self.retime_app().is_some() { SETTLE_APP } else { SETTLE_HOME };
        match self.retime.machine.step(now_ns, Obs { clear, alive, hz, settle }) {
            Step::None => {}
            Step::Start => self.retime_start(now_ns, false),
            Step::Back(after) => {
                let (hz, cleared) = (self.retime.last_hz.unwrap_or_default(), now_ns.saturating_sub(self.retime.machine.since()));
                self.retime_say(format!(
                    "retime: the panel is back at {hz:.1} Hz, {:.1} s after the clip began and {:.1} s after the main window cleared",
                    after as f64 / 1e9,
                    cleared as f64 / 1e9
                ));
            }
            Step::Stop(why) => self.retime_stop(now_ns, why),
        }
    }

    /// The app in front when it is a TV app: the clip's decoder is opened for it. `None` under Home, under LumaTV's own windows and with nothing in front.
    fn retime_app(&self) -> Option<&str> {
        self.scene.foreground(&self.surfaces).map(|w| w.app_id()).filter(|a| !a.is_empty() && *a != HOME_APP && !a.starts_with(OWN_APPS))
    }

    /// Screen on, no video window that plays a hardware video, no claim for one and, when `read_source`, no source on the main window. The clip's own window and
    /// its source are not counted while it plays.
    fn panel_rate_clear(&mut self, read_source: bool) -> bool {
        // a window with no claim holds no decoder (the Cameras app's feeder stays 120 s after its view, hidden and 16 by 16), and a texture is decoded on
        // the CPU: neither feeds the main window
        let video = self.scene.windows().any(|w| w.kind == WindowType::VideoWindow && w.vwin_live && !w.vwin_texture && w.app_id() != APP_ID)
            || self.vwin.alloc.granted().any(|c| !c.texture);
        if !self.luna_state.power.rendering || video {
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
        let app = self.retime_app().unwrap_or(HOME_APP).to_owned();
        let spec = Spec {
            program: self.retime.program.clone(),
            clip: self.config.runtime_dir.join(CLIP_FILE),
            stats: self.config.runtime_dir.join("retime-stats.jsonl"),
            runtime_dir: self.config.runtime_dir.clone(),
            socket: self.config.socket.clone(),
            app,
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
                let (n, hz, cleared) = (self.retime.machine.tries(), self.retime.last_hz.unwrap_or_default(), now_ns.saturating_sub(self.retime.machine.since()));
                self.retime_say(format!(
                    "retime: the panel reads {hz:.1} Hz with nothing on the main window for {:.1} s; a {CLIP_FPS} fps clip plays there under the UI to re-time it (player {pid} for {}, try {n} of {MAX_TRIES})",
                    cleared as f64 / 1e9,
                    spec.app
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
            "spent" => "not tried again until something else has had the main window".to_owned(),
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
            Why::Busy => "retime: the clip is stopped: a video or a claim took the main window, or the screen went off".to_owned(),
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
                    return json!({"ok": false, "error": "the main window is not clear (no video, no claim, the screen on), or the retime is off", "retime": self.retime_report()});
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
            "suspect": r.machine.suspect(),
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
            "showRect": [SHOW_RECT.x, SHOW_RECT.y, SHOW_RECT.w, SHOW_RECT.h],
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

    const T: u64 = 100 * MS;

    fn look(clear: bool, alive: bool, hz: Option<f64>, settle: u64) -> Obs {
        Obs { clear, alive, hz, settle }
    }

    #[test]
    fn the_rate_bands_and_the_two_files_read_as_the_set_writes_them() {
        assert!(off_rate(95.5) && off_rate(99.9) && off_rate(100.3) && off_rate(109.9) && off_rate(114.9) && off_rate(125.1) && off_rate(143.9) && off_rate(144.2));
        assert!(!off_rate(115.0) && !off_rate(116.7) && !off_rate(119.4) && !off_rate(119.9) && !off_rate(120.5) && !off_rate(123.3) && !off_rate(125.0));
        assert_eq!(source_connected("[main.p0]\nconnect_src=NONE\n"), Some(false));
        assert_eq!(source_connected("[main.p0]\n  connect_src=VDEC \n"), Some(true));
        assert_eq!(source_connected("[main.p0]\n"), None);
        let table = "           CPU0       CPU1\n 68:       12345        200   GICv2 104 Edge      osd_irq\n 69:          12          0   GICv2 105 Edge      other\n";
        assert_eq!(osd_count(table), Some(12_545));
        assert_eq!(osd_count("no such line\n"), None);
    }

    #[test]
    fn the_meter_reads_a_rate_once_it_has_two_counts_far_enough_apart() {
        let mut m = Meter::default();
        assert_eq!(m.feed(S, 1_000), None, "the first count only starts it");
        assert_eq!(m.feed(S + 200 * MS, 1_023), None, "too soon: 200 ms");
        let hz = m.feed(S + 300 * MS, 1_029).unwrap();
        assert!((hz - 96.67).abs() < 0.1, "{hz}");
        // the next reading starts where this one ended
        let hz = m.feed(S + 600 * MS, 1_065).unwrap();
        assert!((hz - 120.0).abs() < 0.1, "{hz}");
    }

    const H: u64 = SETTLE_HOME;

    #[test]
    fn a_panel_read_low_with_the_window_clear_gets_the_clip_after_the_reading_and_it_ends_after_its_hold() {
        let mut m = Machine::default();
        assert!(m.suspect() && m.wants_rate(), "a start is a look");
        assert_eq!(m.step(0, look(true, false, None, H)), Step::None);
        assert_eq!(m.phase_name(), "clear");
        assert_eq!(m.step(2 * T, look(true, false, None, H)), Step::None, "no reading yet");
        // the reading completes 300 ms on, and the settle for Home is as long
        assert_eq!(m.step(3 * T, look(true, false, Some(95.5), H)), Step::Start);
        assert!(m.playing() && m.tries() == 1);
        assert_eq!(m.step(10 * T, look(true, true, Some(95.5), H)), Step::None, "the panel has not moved");
        assert_eq!(m.step(20 * T, look(true, true, Some(119.4), H)), Step::Back(17 * T));
        assert_eq!(m.step(24 * T, look(true, true, None, H)), Step::None, "held");
        assert_eq!(m.step(25 * T, look(true, true, None, H)), Step::Stop(Why::Done));
        assert!(!m.playing() && !m.suspect() && !m.wants_rate());
        for i in 26..400 {
            assert_eq!(m.step(i * T, look(true, false, None, H)), Step::None);
        }
    }

    #[test]
    fn a_panel_found_at_its_rate_starts_nothing_and_is_looked_at_again_only_after_a_video_or_the_minute() {
        let mut m = Machine::default();
        assert_eq!(m.step(0, look(true, false, None, H)), Step::None);
        assert_eq!(m.step(3 * T, look(true, false, Some(119.9), H)), Step::None);
        assert!(!m.suspect() && !m.wants_rate() && m.phase_name() == "idle");
        for i in 4..600 {
            assert_eq!(m.step(i * T, look(true, false, None, H)), Step::None);
            assert!(!m.wants_rate(), "{i}");
        }
        // the backstop: one look a minute at rest, which finds the panel at its rate
        assert_eq!(m.step(603 * T, look(true, false, None, H)), Step::None);
        assert!(m.suspect() && m.wants_rate());
        assert_eq!(m.step(606 * T, look(true, false, Some(119.9), H)), Step::None);
        assert!(!m.suspect());
        // a video on the window makes the rate suspect, and its end is the look
        assert_eq!(m.step(610 * T, look(false, false, None, H)), Step::None);
        assert!(m.suspect());
        assert_eq!(m.step(700 * T, look(true, false, None, H)), Step::None);
        assert_eq!(m.phase_name(), "clear");
    }

    #[test]
    fn anything_else_taking_the_window_ends_the_clip_and_the_rate_is_suspect_again() {
        let mut m = Machine::default();
        m.step(0, look(true, false, None, H));
        assert_eq!(m.step(3 * T, look(true, false, Some(96.0), H)), Step::Start);
        assert_eq!(m.step(5 * T, look(false, true, None, H)), Step::Stop(Why::Busy));
        assert!(m.suspect() && !m.playing());
        assert_eq!(m.step(6 * T, look(true, false, None, H)), Step::None);
        assert_eq!(m.phase_name(), "clear");
        assert_eq!(m.step(9 * T, look(true, false, Some(96.0), H)), Step::Start);
        assert_eq!(m.tries(), 1, "a new episode has its tries again");
    }

    #[test]
    fn a_clip_that_does_not_move_the_panel_is_given_up_after_eight_seconds_tried_again_once_and_then_left() {
        let mut m = Machine::default();
        m.step(0, look(true, false, None, H));
        assert_eq!(m.step(3 * T, look(true, false, Some(96.0), H)), Step::Start);
        let one = 3 * T + 8 * S;
        assert_eq!(m.step(one - T, look(true, true, Some(96.0), H)), Step::None);
        assert_eq!(m.step(one, look(true, true, Some(96.0), H)), Step::Stop(Why::Timeout));
        assert_eq!(m.phase_name(), "rest");
        assert_eq!(m.step(one + 19 * S, look(true, false, Some(96.0), H)), Step::None);
        assert_eq!(m.step(one + 20 * S, look(true, false, None, H)), Step::None);
        assert_eq!(m.phase_name(), "clear");
        let two = one + 20 * S + 3 * T;
        assert_eq!(m.step(two, look(true, false, Some(96.0), H)), Step::Start);
        assert_eq!(m.tries(), 2);
        assert_eq!(m.step(two + 8 * S, look(true, true, Some(96.0), H)), Step::Stop(Why::Timeout));
        assert_eq!(m.phase_name(), "spent");
        for k in 1..100 {
            assert_eq!(m.step(two + 8 * S + k * 10 * S, look(true, false, Some(96.0), H)), Step::None);
        }
        assert_eq!(m.phase_name(), "spent");
        // something else having the window ends the episode
        let t = two + 2_000 * S;
        m.step(t, look(false, false, None, H));
        m.step(t + T, look(true, false, None, H));
        assert_eq!(m.phase_name(), "clear");
    }

    #[test]
    fn a_player_that_ends_by_itself_before_the_panel_moves_is_a_try_that_did_not_take() {
        let mut m = Machine::default();
        m.step(0, look(true, false, None, H));
        assert_eq!(m.step(3 * T, look(true, false, Some(96.0), H)), Step::Start);
        assert_eq!(m.step(5 * T, look(true, false, Some(96.0), H)), Step::Stop(Why::Ended));
        assert_eq!(m.phase_name(), "rest");
        let t = 5 * T + RETRY_AFTER;
        assert_eq!(m.step(t, look(true, false, None, H)), Step::None);
        assert_eq!(m.step(t + 3 * T, look(true, false, Some(96.0), H)), Step::Start);
        assert_eq!(m.step(t + 5 * T, look(true, false, Some(96.0), H)), Step::Stop(Why::Ended));
        assert_eq!(m.phase_name(), "spent", "the second try is the last");
    }

    #[test]
    fn under_an_app_the_clip_waits_for_the_window_to_stay_clear_a_second_and_a_half() {
        let mut m = Machine::default();
        m.step(0, look(true, false, None, SETTLE_APP));
        assert_eq!(m.step(3 * T, look(true, false, Some(95.5), SETTLE_APP)), Step::None, "the reading is in, the app's next video may not be");
        assert_eq!(m.step(14 * T, look(true, false, Some(95.5), SETTLE_APP)), Step::None);
        assert_eq!(m.step(15 * T, look(true, false, Some(95.5), SETTLE_APP)), Step::Start);
    }

    #[test]
    fn a_clip_started_by_hand_plays_its_seconds_or_until_the_window_is_taken_whatever_the_panel_reads() {
        let mut m = Machine::default();
        assert_eq!(m.force(10 * S, 3 * S), Step::Start);
        assert!(m.playing());
        assert_eq!(m.step(12 * S, look(true, true, Some(119.9), H)), Step::None, "the panel reads back: the clip plays its time anyway");
        assert_eq!(m.step(13 * S, look(true, true, None, H)), Step::Stop(Why::Done));
        m.force(20 * S, 600 * S);
        assert_eq!(m.step(29 * S, look(true, true, None, H)), Step::None);
        assert_eq!(m.step(30 * S, look(true, true, None, H)), Step::Stop(Why::Done), "ten seconds at most");
    }

    #[test]
    fn the_players_command_line_is_the_one_the_tv_rounds_ran_through_the_es_pipe_for_the_app_in_front() {
        let spec = Spec {
            program: "/var/lib/lumatv/bin/lumatv-gfn-feeder".into(),
            clip: "/tmp/xdg/retime-30fps-640x360.h264".into(),
            stats: "/tmp/xdg/retime-stats.jsonl".into(),
            runtime_dir: "/tmp/xdg".into(),
            socket: "wayland-0".into(),
            app: "com.beb.plxnative".into(),
        };
        let args: Vec<String> = spec.args().into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args.join(" "),
            "--play /tmp/xdg/retime-30fps-640x360.h264 --codec h264 --size 640x360 --fps 30 --seconds 12 --loop -w video -a org.nphil.lumatv.retime --pipe es --ndl-app com.beb.plxnative --stats /tmp/xdg/retime-stats.jsonl"
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

    use crate::render::software::Software;
    use crate::surface::SurfaceId;
    use crate::vwin::Mode;

    /// A feeder that plays until it is ended.
    const PLAYS: &str = "#!/bin/sh\nexec sleep 30\n";

    /// A compositor state on a fake clock: the interrupt table and the scaler's status are files in a directory of its own, and the player is a script there
    /// (or none).
    struct Rig {
        state: crate::State,
        _display: wayland_server::Display<crate::State>,
        dir: PathBuf,
        count: f64,
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
            let display: wayland_server::Display<crate::State> = wayland_server::Display::new().unwrap();
            let config = crate::config::Config { runtime_dir: dir.clone(), retime: true, ..crate::config::Config::default() };
            let mut state = crate::State::new(display.handle(), config, Box::new(Software::new(64, 64)), Box::new(crate::luna::Null::default()));
            state.retime.program = dir.join("feeder");
            state.retime.scaler = dir.join("scaler");
            state.retime.interrupts = dir.join("interrupts");
            let rig = Rig { state, _display: display, dir, count: 1_000_000.0, t: S };
            rig.write_count();
            rig
        }

        fn write_count(&self) {
            std::fs::write(self.dir.join("interrupts"), format!(" 68: {:>10} 0 0 0 GICv2 104 Edge osd_irq\n", self.count as u64)).unwrap();
        }

        /// A look every 100 ms of the fake clock for `ms`, on a panel whose OSD interrupt comes `hz` times a second.
        fn run(&mut self, ms: u64, hz: f64) {
            for _ in 0..ms / 100 {
                self.t += 100 * MS;
                self.count += hz / 10.0;
                self.write_count();
                self.state.panel_rate_sync(self.t);
            }
        }

        /// Looks until the player has been started.
        fn until_started(&mut self, hz: f64) {
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

        /// A source on the main window, which only the scaler's status says.
        fn source(&self, on: bool) {
            std::fs::write(self.dir.join("scaler"), format!("[main.p0]\nconnect_src={}\n", if on { "VDEC" } else { "NONE" })).unwrap();
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
    fn a_panel_read_low_with_nothing_on_the_main_window_gets_the_clip_and_it_ends_a_moment_after_the_panel_is_back() {
        let mut r = Rig::new("episode", Some(PLAYS));
        r.run(200, 95.5);
        assert!(r.state.retime.player.pid().is_none(), "not before the reading is in");
        r.until_started(95.5);
        assert!(r.t < 2 * S, "the clip started {:.1} s in", r.t as f64 / 1e9);
        let pid = r.state.retime.player.pid().unwrap();
        assert!(r.said("retime: the panel reads 9"), "{:?}", r.state.journal);
        assert!(r.said("nothing on the main window for 0.3 s; a 30 fps clip plays there under the UI"), "{:?}", r.state.journal);
        assert!(r.state.retime.machine.playing());
        // the panel follows the clip: read back within a reading or two, and the clip plays on for its hold
        r.run(600, 119.4);
        assert!(r.said("retime: the panel is back at 1"), "{:?}", r.state.journal);
        assert_eq!(r.state.retime.player.pid(), Some(pid), "the clip plays on for its hold");
        r.run(800, 119.4);
        assert!(r.state.retime.player.pid().is_none() && !r.state.retime.machine.playing());
        assert!(r.reaped(), "the player is collected");
        assert!(r.said("retime: the clip is stopped"), "{:?}", r.state.journal);
        // the episode is over: a panel at its rate starts nothing
        r.run(5_000, 119.4);
        assert!(r.state.retime.player.pid().is_none());
        assert_eq!(r.state.journal.iter().filter(|l| l.starts_with("retime:")).count(), 3, "one line a switch: {:?}", r.state.journal);
    }

    #[test]
    fn a_panel_at_its_rate_plays_nothing_and_says_nothing() {
        let mut r = Rig::new("rate", Some(PLAYS));
        r.run(3_000, 119.9);
        assert!(r.state.retime.player.pid().is_none());
        assert!(r.state.journal.iter().all(|l| !l.starts_with("retime:")), "{:?}", r.state.journal);
        assert!(!r.state.retime.machine.suspect());
    }

    #[test]
    fn a_video_on_the_main_window_holds_the_clip_back_and_its_end_starts_the_look() {
        let mut r = Rig::new("video", Some(PLAYS));
        r.source(true);
        r.run(3_000, 95.5);
        assert!(r.state.retime.player.pid().is_none());
        assert!(r.state.retime.last_hz.is_none(), "the interrupt is not read while a source feeds the window");
        // a Dolby Vision film closed at Home: the source is gone and the panel is where the film left it
        r.source(false);
        let closed = r.t;
        r.until_started(95.5);
        assert!(r.t - closed < 800 * MS, "the clip started {:.1} s after the source went", (r.t - closed) as f64 / 1e9);
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
        // a source on the main window holds it until the clip itself is the source
        r.source(true);
        assert!(!r.state.panel_rate_clear(true));
        assert!(r.state.panel_rate_clear(false), "the clip's own source is not counted while it plays");
        r.source(false);
        // and the screen being off does
        r.state.luna_state.power.rendering = false;
        assert!(!r.state.panel_rate_clear(true));
        assert!(r.state.retime_app().is_none(), "no window in front: the decoder is opened for Home");
    }

    #[test]
    fn the_cameras_app_s_idle_window_does_not_keep_the_clip_back_after_a_view() {
        // the TV session of 2026-11-04: Back at srv 32360, the clear at 32480 (the window's 120 s), the panel at 100 Hz all that time
        let mut r = Rig::new("ring", Some(PLAYS));
        video_window(&mut r.state, 40, "org.nphil.lumatv.cameras", false, false);
        r.until_started(100.0);
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
            r.until_started(95.5);
            interrupt(&mut r);
            r.run(200, 95.5);
            assert!(r.state.retime.player.pid().is_none() && !r.state.retime.machine.playing(), "{name}: the clip is ended at the next look");
            assert!(r.reaped(), "{name}");
            assert!(r.said("retime: the clip is stopped"), "{name}: {:?}", r.state.journal);
        }
    }

    #[test]
    fn with_no_player_installed_nothing_plays_and_the_reason_is_logged_twice_and_then_not_again() {
        let mut r = Rig::new("noplayer", None);
        r.run(3_000, 100.0);
        assert!(r.said("retime: the clip could not be played: no player at"), "{:?}", r.state.journal);
        assert!(r.said("tried again in 20 s"));
        assert!(r.state.retime.player.pid().is_none());
        // 20 s on, the second try; then the panel is left alone until something else has had the main window
        r.run(24_000, 100.0);
        let tries = |r: &Rig| r.state.journal.iter().filter(|l| l.contains("the clip could not be played")).count();
        assert_eq!(tries(&r), 2, "{:?}", r.state.journal);
        assert!(r.said("not tried again until something else has had the main window"));
        r.run(100_000, 100.0);
        assert_eq!(tries(&r), 2, "much later still two");
        // a video on the window and its end make a new episode, with its tries again
        r.source(true);
        r.run(300, 100.0);
        r.source(false);
        r.run(3_000, 100.0);
        assert_eq!(tries(&r), 3, "{:?}", r.state.journal);
    }

    #[test]
    fn a_player_that_ends_before_the_panel_moves_is_logged_with_how_it_ended_and_what_it_said() {
        let mut r = Rig::new("early", Some("#!/bin/sh\necho 'pipeline: NDL_DirectVideoOpen failed' >&2\nexit 4\n"));
        r.until_started(95.5);
        ended(r.state.retime.player.pid().unwrap());
        // its last words reach the journal's line through a thread of their own
        for _ in 0..400 {
            if !r.state.retime.player.said().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        r.run(200, 95.5);
        assert!(
            r.said("retime: the player ended (exit status: 4) before the panel moved (it said: pipeline: NDL_DirectVideoOpen failed); tried again in 20 s"),
            "{:?}",
            r.state.journal
        );
        assert!(r.state.retime.player.pid().is_none() && !r.state.retime.machine.playing());
    }

    #[test]
    fn a_clip_that_does_not_move_the_panel_is_ended_after_eight_seconds_with_the_reason_in_the_journal() {
        let mut r = Rig::new("timeout", Some(PLAYS));
        r.until_started(95.5);
        r.run(7_500, 95.5);
        assert!(r.state.retime.machine.playing(), "still within its eight seconds");
        r.run(1_000, 95.5);
        assert!(!r.state.retime.machine.playing());
        assert!(r.said("retime: the clip did not move the panel in 8 s; tried again in 20 s"), "{:?}", r.state.journal);
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
        // the clip plays: the window is the whole output, out of the stack, and no hole is cut for it
        assert_eq!(r.state.retime_ctl(&["now"])["ok"], true);
        let w = r.state.scene.window(id).unwrap();
        assert!(w.vwin_quiet && !w.vwin_live && w.vwin_rect == RECT, "{:?}", (w.vwin_quiet, w.vwin_live, w.vwin_rect));
        assert_eq!((RECT.w, RECT.h), (crate::OUTPUT_WIDTH as i32, crate::OUTPUT_HEIGHT as i32), "not small: the picture controller reads a small window as picture-in-graphics");
        r.state.retime_ctl(&["show"]);
        let w = r.state.scene.window(id).unwrap();
        assert!(w.vwin_live && !w.vwin_quiet && w.vwin_rect == SHOW_RECT, "shown, it is a live small window with a hole");
        r.state.retime_ctl(&["hide"]);
        assert!(r.state.scene.window(id).unwrap().vwin_quiet);
        r.state.retime_ctl(&["stop"]);
        let w = r.state.scene.window(id).unwrap();
        assert!(!w.vwin_quiet && !w.vwin_live && w.vwin_rect.is_empty(), "hidden again");
        assert!(r.reaped());
    }
}
