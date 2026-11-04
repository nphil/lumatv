//! Boot direct: LumaTV's Home from the first frame, with LG's Flutter Home out of the way (`docs/research/boot-direct.md`).
//!
//! `lumatv boot-direct on` writes `<data dir>/boot-direct`: the line `on` and a `tool PATH` line naming itself. While the file is there
//! the tool keeps the Home stub over LG's Home directory, so what SAM, bootd and the preload manager open as "Home" is a page that
//! draws nothing and the native Home is what is on the screen. What differs in the compositor, all of it behind that file:
//!
//! - the Rust shell's client starts with the compositor, not when SAM's Home maps (the stub's launch waits for WebAppMgr, which comes up
//!   with the compositor);
//! - a Home key held for [`HOME_HOLD`] asks the tool for LG's real Home (`lumatv home lg`), and the native Home steps aside once LG's
//!   Home is the front card ([`Phase`]);
//! - Home or Back there, or another app taking the front, asks for the stub back (`lumatv home luma`);
//! - the stub in front with no client to draw Home ([`Seen::ui_gone`]) for [`GUARD_AFTER`] is a black screen: LG's Home is asked for.
//!
//! The supervisor's own escape (Home held 10 s, a failed start) is not here and does not need this: every way out of LumaTV's
//! compositor ends in the tool's `stock-ui-restore`, which drops the stub.
//!
//! State only, time is an argument: `input.rs` feeds it the keys and the tick, so every decision is a host test. [`Run`] is the one
//! place that starts a process.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use wayland_server::Resource;

use crate::State;
use crate::input::{Route, code};
use crate::scene::HOME_APP;

/// A Home key held this long asks for LG's Home (the hold of the remote's other buttons, `keys::HOLD`).
pub const HOME_HOLD: Duration = crate::keys::HOLD;
/// LG's Home asked for and not in front after this long: the native Home stays and nothing more is waited for.
pub const LG_WAIT: Duration = Duration::from_secs(15);
/// The stub in front and nothing to draw Home for this long is a black screen: LG's Home is asked for.
pub const GUARD_AFTER: Duration = Duration::from_secs(5);
/// A press of Home after this long without one is a new press (the release of the earlier one was lost), not its repeat.
const REPEAT_WINDOW: Duration = Duration::from_secs(2);
/// The flag file is looked at most once a second, from the tick.
const POLL: Duration = Duration::from_secs(1);
/// A `lumatv home` that has not ended by now is stopped: the later request would wait behind it.
const RUN_MAX: Duration = Duration::from_secs(120);
/// The flag file's name in the data directory (`BD_FLAG` in `tools/lumatv`).
pub const FLAG_FILE: &str = "boot-direct";
/// What LG's Home is as a client: LG's Flutter runtime started for the Home app (`HOME_PKILL_PATTERN` in `tools/lumatv`; the command line
/// on the set is `/usr/bin/flutter-client -i com.webos.app.home {...}`). The stub is a page in WebAppMgr's process and has no such name, and
/// the screensaver is a Flutter client too, for another app.
const LG_HOME_CLIENT: &str = "flutter-client -i com.webos.app.home";

/// The flag file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flag {
    pub on: bool,
    /// `tools/lumatv`, which the compositor runs for `home lg` and `home luma`.
    pub tool: Option<PathBuf>,
}

impl Flag {
    /// `on` on the first line; a `tool PATH` line anywhere after it. Anything else is off.
    pub fn parse(text: &str) -> Flag {
        let mut lines = text.lines();
        let on = lines.next().is_some_and(|l| l.trim() == "on");
        let tool = lines.find_map(|l| l.strip_prefix("tool ")).map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from);
        Flag { on, tool }
    }
}

/// What the tool is asked to do: `lumatv home lg` or `lumatv home luma`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// The stub goes away and LG's Home is opened.
    Lg,
    /// The stub comes back.
    Luma,
}

impl Verb {
    pub fn arg(self) -> &'static str {
        match self {
            Verb::Lg => "lg",
            Verb::Luma => "luma",
        }
    }
}

/// Whose Home is on the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// LumaTV's: the native Home is drawn wherever Home is in front.
    #[default]
    Off,
    /// LG's Home asked for, since the instant: the native Home stays until LG's is in front.
    Waiting(Instant),
    /// LG's Home is the front card and the native Home is not drawn.
    Shown,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Off => "off",
            Phase::Waiting(_) => "waiting",
            Phase::Shown => "shown",
        }
    }
}

/// What the tick sees of the screen.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seen {
    /// The front card is a Home card of either kind.
    pub home: bool,
    /// The front card is LG's own Home (Flutter), not the stub.
    pub lg_home: bool,
    /// The Rust shell has no client to draw LumaTV's Home and none is coming.
    pub ui_gone: bool,
    /// The panel is lit.
    pub screen_on: bool,
}

/// What a decision asks the caller to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Out {
    /// A `lumatv home VERB` to run.
    pub run: Option<Verb>,
    /// The native Home is drawn, or not, by the phase: the caller looks at it again.
    pub resync: bool,
    /// A line for the journal.
    pub note: Option<String>,
}

/// The Home key's hold: pressed at an instant, due [`HOME_HOLD`] later, and whether it has run.
#[derive(Default)]
struct HomeHold {
    down: Option<Instant>,
    last: Option<Instant>,
    fired: bool,
}

impl HomeHold {
    /// A press, or a repeat of it, which does not start the hold again.
    fn press(&mut self, now: Instant) {
        let repeat = self.down.is_some() && self.last.is_some_and(|t| now.saturating_duration_since(t) < REPEAT_WINDOW);
        if !repeat {
            self.down = Some(now);
            self.fired = false;
        }
        self.last = Some(now);
    }

    /// The release: whether the hold had run, in which case the release is no Home tap.
    fn release(&mut self) -> bool {
        let fired = self.fired;
        *self = HomeHold::default();
        fired
    }

    /// Whether the hold comes due now. Once per press.
    fn due(&mut self, now: Instant) -> bool {
        match self.down {
            Some(t) if !self.fired && now.saturating_duration_since(t) >= HOME_HOLD => {
                self.fired = true;
                true
            }
            _ => false,
        }
    }
}

/// The compositor's boot direct: the flag, whose Home is shown, and the tool's runs.
#[derive(Default)]
pub struct BootDirect {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    polled: Option<Instant>,
    flag: Flag,
    phase: Phase,
    hold: HomeHold,
    /// A Back taken on LG's Home: its repeats and release are taken too.
    back_taken: bool,
    /// Since when the stub has been in front with no client to draw Home.
    guard: Option<Instant>,
    run: Run,
}

impl BootDirect {
    /// The state of a compositor whose data directory is `data_dir`, with the flag as it is now.
    pub fn new(data_dir: &Path) -> BootDirect {
        let mut b = BootDirect { path: data_dir.join(FLAG_FILE), ..BootDirect::default() };
        b.read();
        b
    }

    pub fn on(&self) -> bool {
        self.flag.on
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether the native Home is withheld because LG's is on the screen.
    pub fn hides_home(&self) -> bool {
        self.flag.on && self.phase == Phase::Shown
    }

    /// Reads the flag again when the file is not the one last read, at most once a second. Returns whether `on` changed; a flag that
    /// went off takes whatever was in progress with it.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.polled.is_some_and(|t| now.saturating_duration_since(t) < POLL) {
            return false;
        }
        self.polled = Some(now);
        let before = self.flag.on;
        self.read();
        if before && !self.flag.on {
            self.phase = Phase::Off;
            self.hold = HomeHold::default();
            self.back_taken = false;
            self.guard = None;
        }
        before != self.flag.on
    }

    fn read(&mut self) {
        let stamp = std::fs::metadata(&self.path).ok().map(|m| (m.modified().unwrap_or(UNIX_EPOCH), m.len()));
        if stamp == self.stamp {
            return;
        }
        match stamp {
            None => self.flag = Flag::default(),
            // A file that cannot be read now is looked at again at the next poll.
            Some(_) => match std::fs::read_to_string(&self.path) {
                Ok(text) => self.flag = Flag::parse(&text),
                Err(_) => return,
            },
        }
        self.stamp = stamp;
    }

    /// The Home key: at the routing (a Home the focused window did not take), or on LG's Home, whose own Home key is the way back to
    /// LumaTV's whatever the app asked for. Returns whether it is taken: on LG's Home the press and the release both, so that LG's Home is
    /// never left with a key down; otherwise a release that ends a hold that ran is no Home tap.
    pub fn home_key(&mut self, pressed: bool, ui_gone: bool, now: Instant) -> (bool, Out) {
        if !self.flag.on {
            return (false, Out::default());
        }
        if pressed {
            self.hold.press(now);
            return (self.phase == Phase::Shown && !ui_gone, Out::default());
        }
        if self.hold.release() {
            return (true, Out::default());
        }
        // With no client for LumaTV's Home there is nothing to go back to: the tap is LG's own.
        if self.phase != Phase::Off && !ui_gone {
            return (true, self.leave("Home"));
        }
        (false, Out::default())
    }

    /// Back or Exit at the routing. On LG's Home it is the way back to LumaTV's.
    pub fn back_key(&mut self, pressed: bool, ui_gone: bool) -> (bool, Out) {
        if !self.flag.on {
            return (false, Out::default());
        }
        if self.back_taken {
            self.back_taken = pressed;
            return (true, Out::default());
        }
        if pressed && self.phase == Phase::Shown && !ui_gone {
            self.back_taken = true;
            return (true, self.leave("Back"));
        }
        (false, Out::default())
    }

    fn leave(&mut self, why: &str) -> Out {
        self.phase = Phase::Off;
        Out { run: Some(Verb::Luma), resync: true, note: Some(format!("boot-direct: {why} on LG's Home, LumaTV's Home is back")) }
    }

    /// One look at the screen from the tick.
    pub fn tick(&mut self, now: Instant, seen: &Seen) -> Out {
        let mut out = Out::default();
        if !self.flag.on {
            return out;
        }
        match self.phase {
            Phase::Off => {
                let stub_front = seen.home && !seen.lg_home;
                if self.hold.due(now) {
                    self.phase = Phase::Waiting(now);
                    out.run = Some(Verb::Lg);
                    out.note = Some("boot-direct: Home held, LG's Home asked for".into());
                } else if seen.ui_gone && stub_front && seen.screen_on {
                    let since = *self.guard.get_or_insert(now);
                    if now.saturating_duration_since(since) >= GUARD_AFTER {
                        self.guard = None;
                        self.phase = Phase::Waiting(now);
                        out.run = Some(Verb::Lg);
                        out.note = Some("boot-direct: the stub is in front and nothing draws Home, LG's Home asked for".into());
                    }
                } else {
                    self.guard = None;
                }
            }
            Phase::Waiting(since) => {
                if seen.lg_home {
                    self.phase = Phase::Shown;
                    out.resync = true;
                    out.note = Some(format!("boot-direct: LG's Home is in front after {} ms", now.saturating_duration_since(since).as_millis()));
                } else if now.saturating_duration_since(since) >= LG_WAIT {
                    self.phase = Phase::Off;
                    out.note = Some(format!("boot-direct: LG's Home not in front after {} s, LumaTV's Home stays", LG_WAIT.as_secs()));
                }
            }
            Phase::Shown => {
                if !seen.lg_home {
                    self.phase = Phase::Off;
                    out.resync = true;
                    // With no client for LumaTV's Home the stub would be a black screen: LG's stays.
                    out.run = (!seen.ui_gone).then_some(Verb::Luma);
                    out.note = Some("boot-direct: LG's Home is not in front any more".into());
                }
            }
        }
        out
    }

    /// Asks for a run of the tool: now, or after the one that is running.
    pub fn ask(&mut self, verb: Verb) {
        self.run.queued = Some(verb);
    }

    /// Starts what was asked for, collects what ended. Lines for the journal.
    pub fn pump(&mut self, now: Instant) -> Vec<String> {
        let tool = self.flag.tool.clone();
        self.run.pump(tool.as_deref(), now)
    }

    /// `ctl shell`'s `bootDirect` part.
    pub fn report(&self) -> Value {
        json!({
            "on": self.flag.on,
            "tool": self.flag.tool,
            "phase": self.phase.name(),
            "running": self.run.child.as_ref().map(|(_, v, _)| v.arg()),
            "queued": self.run.queued.map(Verb::arg),
        })
    }
}

/// The tool's runs: one at a time, the latest ask after the one that is running.
#[derive(Default)]
struct Run {
    child: Option<(Child, Verb, Instant)>,
    queued: Option<Verb>,
}

impl Run {
    fn pump(&mut self, tool: Option<&Path>, now: Instant) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some((child, verb, started)) = &mut self.child {
            let took = now.saturating_duration_since(*started).as_millis();
            match child.try_wait() {
                Ok(Some(status)) => {
                    notes.push(format!("boot-direct: lumatv home {} ended ({status}) after {took} ms", verb.arg()));
                    self.child = None;
                }
                Ok(None) if now.saturating_duration_since(*started) > RUN_MAX => {
                    let _ = child.kill();
                    let _ = child.wait();
                    notes.push(format!("boot-direct: lumatv home {} stopped after {took} ms", verb.arg()));
                    self.child = None;
                }
                Ok(None) => {}
                Err(e) => {
                    notes.push(format!("boot-direct: lumatv home {}: {e}", verb.arg()));
                    self.child = None;
                }
            }
        }
        if self.child.is_none()
            && let Some(verb) = self.queued.take()
        {
            match tool {
                None => notes.push(format!("boot-direct: the flag file names no tool, `home {}` not run", verb.arg())),
                Some(tool) => match spawn(tool, verb) {
                    Ok(child) => {
                        notes.push(format!("boot-direct: {} home {} started, pid {}", tool.display(), verb.arg(), child.id()));
                        self.child = Some((child, verb, now));
                    }
                    Err(e) => notes.push(format!("boot-direct: {} home {} did not start: {e}", tool.display(), verb.arg())),
                },
            }
        }
        notes
    }
}

/// `tool home VERB`, in a group of its own so that what takes the compositor's group down does not stop it half way, with nothing on its
/// standard streams (the tool keeps its own log) and without the variables of the compositor's own supervision.
fn spawn(tool: &Path, verb: Verb) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    Command::new(tool)
        .args(["home", verb.arg()])
        .env_remove("NOTIFY_SOCKET")
        .env_remove("LUMATV_LUNA_BRIDGE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
}

/// Whether the client with this command line is LG's Home rather than the stub.
pub fn is_lg_home_client(cmd: &str) -> bool {
    cmd.contains(LG_HOME_CLIENT)
}

impl State {
    /// A Home or Back key at the routing: whether boot direct takes it (`input.rs`). Nothing at all while the flag is off.
    pub(crate) fn boot_direct_key(&mut self, key: u32, pressed: bool, route: Route, now: Instant) -> bool {
        if !self.boot_direct.on() {
            return false;
        }
        let ui_gone = self.boot_direct_ui_gone();
        let (taken, out) = match key {
            code::KEY_HOME | code::KEY_HOMEPAGE | code::KEY_RF_HOME if route == Route::Home || self.boot_direct.phase() == Phase::Shown => {
                self.boot_direct.home_key(pressed, ui_gone, now)
            }
            code::KEY_BACK | code::KEY_PREVIOUS | code::KEY_EXIT => self.boot_direct.back_key(pressed, ui_gone),
            _ => return false,
        };
        self.boot_direct_apply(out, now);
        taken
    }

    /// From the tick: the flag, the hold, whose Home is on the screen, the tool's runs.
    pub(crate) fn boot_direct_tick(&mut self, now: Instant) {
        if self.boot_direct.poll(now) {
            let on = self.boot_direct.on();
            self.journal(format!("boot-direct: {}", if on { "on" } else { "off, LumaTV's Home is the Home" }));
            self.shell_rust_sync_home();
        }
        if self.boot_direct.on() {
            let seen = self.boot_direct_seen();
            let out = self.boot_direct.tick(now, &seen);
            self.boot_direct_apply(out, now);
        }
        for line in self.boot_direct.pump(now) {
            self.journal(line);
        }
    }

    fn boot_direct_apply(&mut self, out: Out, now: Instant) {
        if let Some(note) = out.note {
            self.journal(note);
        }
        if out.resync {
            self.shell_rust_sync_home();
        }
        if let Some(verb) = out.run {
            self.boot_direct.ask(verb);
            for line in self.boot_direct.pump(now) {
                self.journal(line);
            }
        }
    }

    /// The Rust shell has no client to draw LumaTV's Home and none is coming (it gave up, or was never run).
    fn boot_direct_ui_gone(&self) -> bool {
        self.shell_rust.gave_up || !self.shell_rust.enabled()
    }

    fn boot_direct_seen(&self) -> Seen {
        let front = self.scene.foreground(&self.surfaces).filter(|w| w.app_id() == HOME_APP);
        let lg_home = front.is_some_and(|w| {
            let client = self.seat.surfaces.get(&w.surface).and_then(|s| s.client());
            client.and_then(|c| self.clients.get(&c.id())).is_some_and(|c| is_lg_home_client(&c.cmd))
        });
        Seen { home: front.is_some(), lg_home, ui_gone: self.boot_direct_ui_gone(), screen_on: self.luna_state.power.rendering }
    }

    /// `ctl shell`'s `bootDirect` part: the flag and the phase, and what the front card is. The checks of a boot read it to see that LG's
    /// Home is not the one in front.
    pub(crate) fn boot_direct_report(&self) -> Value {
        let seen = self.boot_direct_seen();
        let mut report = self.boot_direct.report();
        report["homeFront"] = json!(seen.home);
        report["lgHomeFront"] = json!(seen.lg_home);
        report["uiGone"] = json!(seen.ui_gone);
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn on() -> BootDirect {
        BootDirect { flag: Flag { on: true, tool: Some("/tool".into()) }, ..BootDirect::default() }
    }

    fn seen(home: bool, lg_home: bool) -> Seen {
        Seen { home, lg_home, ui_gone: false, screen_on: true }
    }

    /// A temp directory of its own per test.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumatv-boot-direct-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_flag_is_on_only_for_on_on_the_first_line() {
        assert_eq!(Flag::parse("on\ntool /a/b/lumatv\n"), Flag { on: true, tool: Some("/a/b/lumatv".into()) });
        assert_eq!(Flag::parse("on\n"), Flag { on: true, tool: None });
        assert_eq!(Flag::parse("on"), Flag { on: true, tool: None });
        assert_eq!(Flag::parse("on\ntool   \n"), Flag { on: true, tool: None }, "a tool line with no path names nothing");
        for off in ["", "off\ntool /x\n", "\non\n", "tool /x\non\n", "onn\n", "yes\n"] {
            assert!(!Flag::parse(off).on, "{off:?}");
        }
        assert_eq!(Flag::parse("tool /x\n").tool, None, "the first line is not a tool line");
    }

    #[test]
    fn the_hold_comes_due_once_after_a_second_and_its_release_is_taken() {
        let t0 = Instant::now();
        let mut h = HomeHold::default();
        assert!(!h.due(t0), "nothing is down");
        h.press(t0);
        assert!(!h.due(t0 + MS(999)));
        h.press(t0 + MS(500));
        assert!(!h.due(t0 + MS(999)), "a repeat does not start the hold again");
        assert!(h.due(t0 + MS(1000)));
        assert!(!h.due(t0 + MS(1500)), "once per press");
        assert!(h.release(), "the release of a hold that ran");
        assert!(!h.release(), "a release with nothing down");
        h.press(t0 + MS(5000));
        assert!(!h.release(), "a tap");
    }

    #[test]
    fn a_press_long_after_the_last_one_is_a_new_press() {
        let t0 = Instant::now();
        let mut h = HomeHold::default();
        h.press(t0);
        assert!(h.due(t0 + MS(1000)));
        // the release was lost; the next press, long after, starts again
        h.press(t0 + MS(10_000));
        assert!(!h.due(t0 + MS(10_500)));
        assert!(h.due(t0 + MS(11_000)));
    }

    #[test]
    fn nothing_happens_while_the_flag_is_off() {
        let t0 = Instant::now();
        let mut b = BootDirect::default();
        assert_eq!(b.home_key(true, false, t0), (false, Out::default()));
        assert_eq!(b.tick(t0 + MS(5000), &seen(true, false)), Out::default());
        assert_eq!(b.home_key(false, false, t0 + MS(5000)), (false, Out::default()), "the release is a Home tap as ever");
        assert_eq!(b.back_key(true, false), (false, Out::default()));
        assert!(!b.hides_home());
    }

    #[test]
    fn a_home_hold_asks_for_lgs_home_and_its_release_is_not_a_home_tap() {
        let t0 = Instant::now();
        let mut b = on();
        assert_eq!(b.home_key(true, false, t0), (false, Out::default()));
        assert_eq!(b.tick(t0 + MS(900), &seen(true, false)).run, None, "a Home tap has not become a hold");
        let out = b.tick(t0 + MS(1000), &seen(true, false));
        assert_eq!(out.run, Some(Verb::Lg));
        assert!(!out.resync, "the native Home stays until LG's is in front");
        assert!(matches!(b.phase(), Phase::Waiting(_)));
        assert!(!b.hides_home());
        assert_eq!(b.tick(t0 + MS(1100), &seen(true, false)).run, None, "one ask per hold");
        assert_eq!(b.home_key(false, false, t0 + MS(1500)), (true, Out::default()), "the release is taken");
        assert!(matches!(b.phase(), Phase::Waiting(_)), "and does not leave");
    }

    #[test]
    fn a_home_tap_in_the_off_phase_is_left_to_the_switcher_and_the_taps() {
        let t0 = Instant::now();
        let mut b = on();
        b.home_key(true, false, t0);
        assert_eq!(b.home_key(false, false, t0 + MS(80)), (false, Out::default()));
        assert_eq!(b.tick(t0 + MS(5000), &seen(true, false)).run, None);
    }

    #[test]
    fn lgs_home_in_front_hides_the_native_one_and_home_or_back_returns() {
        let t0 = Instant::now();
        let mut b = on();
        b.home_key(true, false, t0);
        b.tick(t0 + HOME_HOLD, &seen(true, false));
        b.home_key(false, false, t0 + MS(1200));

        let out = b.tick(t0 + MS(2000), &seen(true, true));
        assert_eq!(b.phase(), Phase::Shown);
        assert!(out.resync && out.run.is_none());
        assert!(b.hides_home());

        // Home
        let t1 = t0 + MS(5000);
        assert_eq!(b.home_key(true, false, t1), (true, Out::default()), "LG's Home never sees the press: it would be left with a key down");
        let (taken, out) = b.home_key(false, false, t1 + MS(60));
        assert!(taken);
        assert_eq!(out.run, Some(Verb::Luma));
        assert!(out.resync);
        assert_eq!(b.phase(), Phase::Off);
        assert!(!b.hides_home());

        // Back: the press leaves, its repeats and release are taken
        let mut b = on();
        b.phase = Phase::Shown;
        let (taken, out) = b.back_key(true, false);
        assert!(taken && out.run == Some(Verb::Luma) && out.resync);
        assert_eq!(b.back_key(true, false), (true, Out::default()), "a repeat");
        assert_eq!(b.back_key(false, false), (true, Out::default()), "the release");
        assert_eq!(b.back_key(false, false), (false, Out::default()), "and nothing is held after it");
        assert_eq!(b.back_key(true, false), (false, Out::default()), "a Back in the Off phase is the usual Back");
    }

    #[test]
    fn a_long_press_on_lgs_home_is_a_home_tap_that_leaves() {
        let t0 = Instant::now();
        let mut b = on();
        b.phase = Phase::Shown;
        b.home_key(true, false, t0);
        assert_eq!(b.tick(t0 + MS(1500), &seen(true, true)).run, None, "no second hold while LG's Home is shown");
        let (taken, out) = b.home_key(false, false, t0 + MS(2000));
        assert!(taken);
        assert_eq!(out.run, Some(Verb::Luma));
    }

    #[test]
    fn home_in_the_waiting_phase_cancels_it() {
        let t0 = Instant::now();
        let mut b = on();
        b.phase = Phase::Waiting(t0);
        b.home_key(true, false, t0 + MS(300));
        let (taken, out) = b.home_key(false, false, t0 + MS(360));
        assert!(taken);
        assert_eq!(out.run, Some(Verb::Luma), "queued behind the `lg` that is running");
        assert_eq!(b.phase(), Phase::Off);
    }

    #[test]
    fn lgs_home_that_never_comes_up_is_given_up_after_the_wait() {
        let t0 = Instant::now();
        let mut b = on();
        b.phase = Phase::Waiting(t0);
        assert_eq!(b.tick(t0 + LG_WAIT - MS(1), &seen(false, false)), Out::default());
        let out = b.tick(t0 + LG_WAIT, &seen(false, false));
        assert_eq!(b.phase(), Phase::Off);
        assert!(out.note.is_some() && out.run.is_none() && !out.resync, "the native Home was never hidden: nothing to put back");
    }

    #[test]
    fn leaving_lgs_home_for_an_app_puts_the_stub_back() {
        let t0 = Instant::now();
        let mut b = on();
        b.phase = Phase::Shown;
        let out = b.tick(t0, &seen(false, false));
        assert_eq!(b.phase(), Phase::Off);
        assert_eq!(out.run, Some(Verb::Luma));
        assert!(out.resync);
    }

    #[test]
    fn with_no_client_for_lumatvs_home_lgs_stays_and_its_keys_are_its_own() {
        let t0 = Instant::now();
        let mut b = on();
        b.phase = Phase::Shown;
        assert_eq!(b.home_key(true, true, t0), (false, Out::default()));
        assert_eq!(b.home_key(false, true, t0 + MS(50)), (false, Out::default()), "the Home tap is LG's own");
        assert_eq!(b.back_key(true, true), (false, Out::default()));
        assert_eq!(b.phase(), Phase::Shown);
        let gone = Seen { ui_gone: true, ..seen(false, false) };
        let out = b.tick(t0 + MS(100), &gone);
        assert_eq!(out.run, None, "the stub would be a black screen");
        assert_eq!(b.phase(), Phase::Off);
    }

    #[test]
    fn a_stub_that_nothing_draws_over_is_replaced_by_lgs_home_after_the_guard() {
        let t0 = Instant::now();
        let mut b = on();
        let gone = Seen { home: true, lg_home: false, ui_gone: true, screen_on: true };
        assert_eq!(b.tick(t0, &gone), Out::default());
        assert_eq!(b.tick(t0 + GUARD_AFTER - MS(1), &gone), Out::default());
        let out = b.tick(t0 + GUARD_AFTER, &gone);
        assert_eq!(out.run, Some(Verb::Lg));
        assert!(matches!(b.phase(), Phase::Waiting(_)));

        // it starts counting again from the moment the stub is in front with no UI
        let mut b = on();
        assert_eq!(b.tick(t0, &gone), Out::default());
        assert_eq!(b.tick(t0 + MS(3000), &Seen { home: false, ..gone }), Out::default(), "an app is in front");
        assert_eq!(b.tick(t0 + MS(3001), &gone), Out::default());
        assert_eq!(b.tick(t0 + MS(3001) + GUARD_AFTER - MS(1), &gone), Out::default());
        assert_eq!(b.tick(t0 + MS(3001) + GUARD_AFTER, &gone).run, Some(Verb::Lg));
    }

    #[test]
    fn the_guard_does_not_fire_with_the_ui_up_with_lgs_home_in_front_or_with_the_screen_off() {
        let t0 = Instant::now();
        for s in [
            Seen { home: true, lg_home: false, ui_gone: false, screen_on: true },
            Seen { home: true, lg_home: true, ui_gone: true, screen_on: true },
            Seen { home: true, lg_home: false, ui_gone: true, screen_on: false },
            Seen { home: false, lg_home: false, ui_gone: true, screen_on: true },
        ] {
            let mut b = on();
            b.tick(t0, &s);
            assert_eq!(b.tick(t0 + GUARD_AFTER * 3, &s), Out::default(), "{s:?}");
            assert_eq!(b.phase(), Phase::Off);
        }
    }

    #[test]
    fn the_flag_file_is_followed_and_going_off_ends_what_was_in_progress() {
        let dir = temp("flag");
        let t0 = Instant::now();
        let mut b = BootDirect::new(&dir);
        assert!(!b.on(), "no file, no boot direct");
        assert!(!b.poll(t0), "nothing changed");
        std::fs::write(dir.join(FLAG_FILE), "on\ntool /x/lumatv\n").unwrap();
        assert!(!b.poll(t0 + MS(500)), "looked at most once a second");
        assert!(b.poll(t0 + MS(1000)), "on now");
        assert!(b.on());
        assert_eq!(b.flag.tool.as_deref(), Some(Path::new("/x/lumatv")));
        assert!(!b.poll(t0 + MS(1500)), "rate limited");

        b.phase = Phase::Shown;
        b.back_taken = true;
        b.guard = Some(t0);
        b.hold.press(t0);
        std::fs::remove_file(dir.join(FLAG_FILE)).unwrap();
        assert!(!b.poll(t0 + MS(1900)), "within the second of the last look");
        assert!(b.poll(t0 + MS(2000)), "the file is gone: off");
        assert!(!b.on());
        assert_eq!((b.phase(), b.back_taken, b.guard, b.hold.down), (Phase::Off, false, None, None));
        assert!(!b.hides_home());

        std::fs::write(dir.join(FLAG_FILE), "off\n").unwrap();
        assert!(!b.poll(t0 + MS(3000)), "a file that does not say on is off");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn it_reads_the_flag_at_start() {
        let dir = temp("start");
        std::fs::write(dir.join(FLAG_FILE), "on\n").unwrap();
        assert!(BootDirect::new(&dir).on());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_lgs_home_is_a_flutter_client() {
        assert!(is_lg_home_client("flutter-client -i com.webos.app.home"));
        assert!(is_lg_home_client(r#"/usr/bin/flutter-client -i com.webos.app.home {"@system_native_app":true,"event":"launch"}"#));
        assert!(!is_lg_home_client(r#"/usr/bin/flutter-client -i com.webos.app.screensaver {"parameters":{}}"#), "another Flutter app");
        assert!(!is_lg_home_client("/usr/sbin/webapp-mgr --webos-wayland"));
        assert!(!is_lg_home_client(""));
    }

    /// A tool that records its arguments and then runs `tail`.
    fn fake_tool(dir: &Path, tail: &str) -> PathBuf {
        let tool = dir.join("lumatv");
        std::fs::write(&tool, format!("#!/bin/sh\necho \"$@\" >> '{}'\n{tail}\n", dir.join("calls").display())).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        tool
    }

    /// A tool that ends once `go` exists in `dir`.
    fn gated_tool(dir: &Path) -> PathBuf {
        fake_tool(dir, &format!("while [ ! -e '{}' ]; do sleep 0.02; done\nexit 0", dir.join("go").display()))
    }

    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn wait_calls(dir: &Path, n: usize) -> Vec<String> {
        for _ in 0..500 {
            if calls(dir).len() >= n {
                break;
            }
            std::thread::sleep(MS(10));
        }
        calls(dir)
    }

    /// Pumps until nothing runs any more; the lines it collected.
    fn wait_idle(b: &mut BootDirect, now: Instant) -> Vec<String> {
        let mut lines = Vec::new();
        for _ in 0..500 {
            lines.extend(b.pump(now));
            if b.run.child.is_none() {
                break;
            }
            std::thread::sleep(MS(10));
        }
        lines
    }

    #[test]
    fn the_tool_is_run_with_home_and_the_verb_one_at_a_time_and_the_latest_ask_wins() {
        let dir = temp("run");
        let mut b = BootDirect { flag: Flag { on: true, tool: Some(gated_tool(&dir)) }, ..BootDirect::default() };
        let t0 = Instant::now();

        b.ask(Verb::Lg);
        let lines = b.pump(t0);
        assert!(lines.iter().any(|l| l.contains("home lg started")), "{lines:?}");
        assert_eq!(wait_calls(&dir, 1), ["home lg"]);
        assert_eq!(b.report()["running"], "lg");

        // while it runs, asks wait; the last one is the one that runs
        b.ask(Verb::Luma);
        b.ask(Verb::Lg);
        b.ask(Verb::Luma);
        assert!(b.pump(t0).is_empty(), "nothing starts beside the running one");
        assert_eq!(b.report()["queued"], "luma");
        assert_eq!(calls(&dir), ["home lg"]);

        std::fs::write(dir.join("go"), "").unwrap();
        let lines = wait_idle(&mut b, t0);
        assert!(lines.iter().any(|l| l.contains("home lg ended") && l.contains("exit status: 0")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("home luma started")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("home luma ended")), "{lines:?}");
        assert_eq!(calls(&dir), ["home lg", "home luma"]);
        assert_eq!(b.report()["running"], Value::Null);
        assert_eq!(b.report()["queued"], Value::Null);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_tool_that_fails_or_is_missing_is_said_and_never_retried() {
        let dir = temp("fail");
        let mut b = BootDirect { flag: Flag { on: true, tool: Some(fake_tool(&dir, "exit 3")) }, ..BootDirect::default() };
        let t0 = Instant::now();
        b.ask(Verb::Lg);
        b.pump(t0);
        let lines = wait_idle(&mut b, t0);
        assert!(lines.iter().any(|l| l.contains("exit status: 3")), "{lines:?}");
        assert!(b.pump(t0).is_empty());
        assert_eq!(calls(&dir).len(), 1);

        let mut b = BootDirect { flag: Flag { on: true, tool: Some(dir.join("absent")) }, ..BootDirect::default() };
        b.ask(Verb::Lg);
        let lines = b.pump(t0);
        assert!(lines.iter().any(|l| l.contains("did not start")), "{lines:?}");
        assert!(b.pump(t0).is_empty());

        let mut b = BootDirect { flag: Flag { on: true, tool: None }, ..BootDirect::default() };
        b.ask(Verb::Luma);
        let lines = b.pump(t0);
        assert!(lines.iter().any(|l| l.contains("names no tool")), "{lines:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_tool_that_does_not_end_is_stopped_after_the_limit() {
        let dir = temp("hang");
        let mut b = BootDirect { flag: Flag { on: true, tool: Some(fake_tool(&dir, "exec sleep 30")) }, ..BootDirect::default() };
        let t0 = Instant::now();
        b.ask(Verb::Lg);
        b.pump(t0);
        assert!(b.pump(t0 + RUN_MAX).is_empty(), "at the limit it still runs");
        let lines = b.pump(t0 + RUN_MAX + MS(1));
        assert!(lines.iter().any(|l| l.contains("stopped")), "{lines:?}");
        assert!(b.run.child.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
