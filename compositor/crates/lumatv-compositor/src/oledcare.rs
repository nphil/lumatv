//! OLED care: what the compositor does so LumaTV's own static screens do not wear the panel (`theme.oledcare`, off until switched on).
//!
//! Two things, both on LumaTV's two shell windows (`ShellHome`, `ShellSystem`) and on nothing else: an app's picture, a video and the
//! moving wallpaper's plane are never moved or dimmed.
//!
//! **Orbit.** Both windows are drawn at the same small offset that walks a square ring of `orbitPx` around the origin, one pixel on
//! one axis per beat (`stepSec`, 30 s): a pixel step of a layer that stays on the screen for minutes is not seen, and the ring is
//! 8 x `orbitPx` beats long (12 minutes at 3 px). The first beat after the switch is turned on walks from the origin to the ring the
//! same way, so nothing ever jumps. The beat rides on a frame that is drawn anyway (any window's fresh commit) and, on a screen
//! at rest, comes [`GRACE_NS`] after it is due: it is drawn by the idle tick the loop already has (`ticker`), no timer of its own, and
//! its frame does not keep the tick at the panel's rate (`Care::beat_frame`, `Output::quiet_frame`). A beat is counted only while a
//! shell window is on the screen: the clock stands still while none is, so the time on each position is the time shown, and nothing
//! costs anything while hidden.
//!
//! While the shell's screensaver is up the care stands aside (the saver moves, is mostly black and dims over minutes: [`Care::saver`]).
//!
//! **Dim.** Home (the status bar, the clock and the dock stay put for hours on a Home nobody touches, when the saver is off) that has been
//! still (no key or pointer, and fewer than [`MOVING_PER_S`] commits a second) for `dimAfterMin` is dimmed over `dimOverMin` down to
//! `dimTo` of its brightness, in steps of one hundredth, each taken at a beat. The first key or pointer movement brings it back at
//! once (the frame that key changes anyway); a Home that starts moving again (the wallpaper's motion) is bright again, and one that
//! has just appeared starts bright. The system layer is not dimmed: what it holds for long is a camera's pop-up or a prompt that waits
//! for an answer, and the compositor cannot tell a still picture from a video there. The dim multiplies the layer's colour, so it
//! lowers the light of everything Home draws, text included: `dimTo` is not allowed below [`MIN_DIM_TO`], where the palette's
//! second ink still reads 4.5 : 1 (`tests/test_oledcare.py`).
//!
//! **The panel's own light** (`panelDim`, on unless the theme says off; `paneldim.rs`). Home's ease is not drawn while the panel can carry it:
//! [`Care::dim`] is 1 for Home (so the plane takes it whole) and [`Care::home_want`] is the fraction of the user's OLED pixel brightness the
//! panel is taken to, one unit at a time. A call that fails hands the ease back to the GPU the next plan (`panel_carry` false).
//!
//! **Apps** (`appDim`, `appShift`, each its own switch, independent of `enabled`). The card in front (`WindowType::Card`, not Home, not a
//! covering overlay) is cared for while it is *still*: no key or pointer, no change of its picture over [`CHANGE_FRACTION`] of the screen,
//! no video hole of its own, no transition. What is a change is the box dim's samples' to say (`boxdim`), not the commits': LG's browser
//! and Plex commit the same frame again every few seconds, and a card that draws the same picture is still however often it commits it.
//! After `appDimAfterMin` (3) of that it eases down over `dimOverMin` to `appDimTo` (0.7 to 1; steps of one hundredth at its own beats)
//! and, with `appShift`, its whole graphics window walks the same ring as the shell's (`orbitPx`, `stepSec`). Any key or pointer
//! movement, a hole appearing (a video starts in it) or a transition puts it back at once, brightness 1 and offset (0, 0) in the very plan
//! that sees it, so the plan is again a sole full-screen item and goes to the plane whole; a change of its picture puts it back in the plan
//! after the sample that sees it, a frame or two after the commit (an engaged card's damage is checked at once, `boxdim::CHECK_NS`). While
//! the care is not engaged nothing differs from the care being off. The shift is never held away from the origin while a hole is present:
//! the hole moves with the graphics but the video plane does not (its rectangle comes from the window at the origin), so a shifted card
//! over a hole would show the video through a misplaced cut-out. A video that is drawn into the app's own surface (no hole) is a picture
//! that changes at every sample: never still.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// The shell windows the care covers, in the order of [`Care::step`]'s views and [`Care::dim`]: Home, then the system layer. Both orbit; only Home dims.
pub const LAYERS: usize = 2;
/// A beat that is due waits this long for a frame that is drawn anyway; then it is drawn alone.
pub const GRACE_NS: u64 = 2_000_000_000;
/// More commits a second than this is movement (video, an animation), not a still layer: a clock's one a second is not.
pub const MOVING_PER_S: f64 = 2.0;
/// The theme file is looked at no more often than this; a change reaches the screen within it.
const POLL_NS: u64 = 2_000_000_000;
/// The lowest `dimTo`: the palette's second ink keeps 4.5 : 1 on its sheet at this brightness, dark and light.
pub const MIN_DIM_TO: f32 = 0.8;
/// The lowest `appDimTo`: an app's own colours are not ours to keep legible to a ratio, so the floor is the contract's 0.7.
pub const MIN_APP_DIM_TO: f32 = 0.7;
/// A change of the app's picture over at least this fraction of the output is a real change (a page, a video frame, a menu), not a
/// caret, a clock or a progress bar: an app dimmed or shifted comes back for it. A smaller change does not count by itself, and neither
/// does a commit of the same picture (`boxdim::APP_CELLS`, the box dim's cells this fraction is).
pub const CHANGE_FRACTION: u64 = 64;

/// `theme.oledcare`, with the shipped values for what the theme leaves out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// The ring's radius in output pixels, 0 to 4 (0: no orbit).
    pub radius: i32,
    /// Time between two beats.
    pub step_ns: u64,
    /// Still for this long before the dim begins.
    pub dim_after_ns: u64,
    /// The dim goes from full to `dim_floor` over this long.
    pub dim_over_ns: u64,
    /// The brightness the dim ends at, [`MIN_DIM_TO`] to 1 (1: no dim).
    pub dim_floor: f32,
    /// `appDim`: the card in front eases down while it is still.
    pub app_dim: bool,
    /// `appShift`: the card in front walks the ring while it is still.
    pub app_shift: bool,
    /// Still for this long before an app's dim and shift begin (`appDimAfterMin`).
    pub app_after_ns: u64,
    /// The brightness an app's dim ends at, [`MIN_APP_DIM_TO`] to 1 (`appDimTo`).
    pub app_floor: f32,
    /// `boxDim` and its tuning keys (`boxdim.rs`).
    pub boxdim: crate::boxdim::Settings,
    /// `panelDim` and its tuning key (`paneldim.rs`): the whole-screen dims go to the panel's own light.
    pub paneldim: crate::paneldim::Settings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            radius: 3,
            step_ns: 30 * SEC,
            dim_after_ns: 5 * 60 * SEC,
            dim_over_ns: 10 * 60 * SEC,
            dim_floor: 0.85,
            app_dim: false,
            app_shift: false,
            app_after_ns: 3 * 60 * SEC,
            app_floor: 0.75,
            boxdim: crate::boxdim::Settings::default(),
            paneldim: crate::paneldim::Settings::default(),
        }
    }
}

const SEC: u64 = 1_000_000_000;

impl Settings {
    /// The `oledcare` object of a theme: `enabled`, `orbitPx`, `stepSec`, `dimAfterMin`, `dimOverMin`, `dimTo`, `appDim`, `appShift`,
    /// `appDimAfterMin`, `appDimTo`. Out-of-range numbers are clamped, wrong types are the shipped value.
    pub fn from_theme(theme: &Value) -> Settings {
        let d = Settings::default();
        let Some(o) = theme.get("oledcare").and_then(Value::as_object) else { return d };
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
        Settings {
            enabled: o.get("enabled").and_then(Value::as_bool).unwrap_or(d.enabled),
            radius: num("orbitPx").map_or(d.radius, |v| v.round().clamp(0.0, 4.0) as i32),
            step_ns: num("stepSec").map_or(d.step_ns, |v| (v.clamp(10.0, 600.0) * SEC as f64) as u64),
            dim_after_ns: num("dimAfterMin").map_or(d.dim_after_ns, |v| (v.clamp(1.0, 120.0) * 60.0 * SEC as f64) as u64),
            dim_over_ns: num("dimOverMin").map_or(d.dim_over_ns, |v| (v.clamp(1.0, 240.0) * 60.0 * SEC as f64) as u64),
            dim_floor: num("dimTo").map_or(d.dim_floor, |v| (v as f32).clamp(MIN_DIM_TO, 1.0)),
            app_dim: o.get("appDim").and_then(Value::as_bool).unwrap_or(d.app_dim),
            app_shift: o.get("appShift").and_then(Value::as_bool).unwrap_or(d.app_shift),
            app_after_ns: num("appDimAfterMin").map_or(d.app_after_ns, |v| (v.clamp(1.0, 120.0) * 60.0 * SEC as f64) as u64),
            app_floor: num("appDimTo").map_or(d.app_floor, |v| (v as f32).clamp(MIN_APP_DIM_TO, 1.0)),
            boxdim: crate::boxdim::Settings::from_object(o),
            paneldim: crate::paneldim::Settings::from_object(o),
        }
    }
}

/// The `i`-th point of the square ring of radius `r` (`r` >= 1), clockwise from its top left corner, `8 r` points, each one pixel
/// from the one before on one axis.
pub fn ring(r: i32, i: usize) -> (i32, i32) {
    if r <= 0 {
        return (0, 0);
    }
    let n = 8 * r as usize;
    let i = (i % n) as i32;
    match i / (2 * r) {
        0 => (-r + i, -r),
        1 => (r, -r + (i - 2 * r)),
        2 => (r - (i - 4 * r), r),
        _ => (-r, r - (i - 6 * r)),
    }
}

/// Brightness of a window that has been still for `still_ns`: 1 until `after_ns`, then down to `floor` over `over_ns`, in steps of 0.01.
pub fn dim_level(still_ns: u64, after_ns: u64, over_ns: u64, floor: f32) -> f32 {
    let into = still_ns.saturating_sub(after_ns) as f64 / over_ns.max(1) as f64;
    let level = 1.0 - (1.0 - floor as f64) * into.clamp(0.0, 1.0);
    ((level * 100.0).round() / 100.0) as f32
}

/// What the care knows of one shell window.
#[derive(Clone, Copy, Debug)]
struct Layer {
    shown: bool,
    /// Its commit count and the time at the last look, for the rate of commits.
    commits: u64,
    looked_ns: u64,
    /// When it last moved, or a key or the pointer last did, or it appeared.
    still_since_ns: u64,
    level: f32,
}

impl Default for Layer {
    fn default() -> Self {
        Layer { shown: false, commits: 0, looked_ns: 0, still_since_ns: 0, level: 1.0 }
    }
}

/// A window of the shell on the screen this frame: how many frames it has committed so far.
pub type View = Option<u64>;

/// The card in front, as the scene shows it to the care at a frame about to be planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppView {
    /// Its window.
    pub window: u32,
    /// Its picture changed over [`CHANGE_FRACTION`] of the screen or more since the last look, as the box dim's samples see it
    /// (`boxdim`): a commit that draws the same picture again, as LG's browser and Plex do every few seconds, is not a change.
    pub changed: bool,
    /// Something is on the screen that makes it not still: a video's hole of its own, a transition.
    pub busy: bool,
}

/// What the care knows of the card in front.
#[derive(Clone, Copy, Debug)]
struct App {
    window: Option<u32>,
    /// When it last changed, or a key or the pointer last did, or it appeared.
    still_since_ns: u64,
    level: f32,
    offset: (i32, i32),
    phase: usize,
    /// When the next beat is due; 0 while it is not engaged.
    next_beat_ns: u64,
    beats: u64,
    /// A dim or a shift is on it: the journal says so once, and once when it is put back.
    engaged: bool,
}

impl Default for App {
    fn default() -> Self {
        App { window: None, still_since_ns: 0, level: 1.0, offset: (0, 0), phase: 0, next_beat_ns: 0, beats: 0, engaged: false }
    }
}

impl App {
    fn away(&self) -> bool {
        self.level != 1.0 || self.offset != (0, 0)
    }
}

#[derive(Default)]
pub struct Care {
    pub settings: Settings,
    /// The screensaver is up (the shell's `state.saver`): it is the burn-in protection of its own (black, everything moving, dimming over an
    /// hour), so the care stands aside, as it does while no shell window is on the screen.
    pub saver: bool,
    /// The panel carries the dim of Home (`paneldim::PanelDim::carries`, set by the state before each plan): Home's own ease still runs, as the
    /// level the panel is asked for ([`Care::home_want`]), and is not drawn.
    pub panel_carry: bool,
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    polled_ns: u64,
    offset: (i32, i32),
    phase: usize,
    /// When the next beat is due; 0 until the first frame with a shell window on it.
    next_beat_ns: u64,
    /// The last frame with a shell window on it, and whether the one before had one.
    seen_ns: u64,
    was_shown: bool,
    layers: [Layer; LAYERS],
    /// A beat moved or dimmed something at the last [`Care::step`].
    changed: bool,
    pub beats: u64,
    app: App,
    /// Lines for the journal (engaged, restored), taken by [`Care::take_events`].
    events: Vec<String>,
}

impl Care {
    /// A care reading `theme/theme.json` of the data directory.
    pub fn new(data_dir: &Path) -> Care {
        Care { path: data_dir.join("theme/theme.json"), ..Care::default() }
    }

    /// Reads the theme again when the file is not the one last read: absent, the shipped values (off); unreadable now (the theme is
    /// written in place, not atomically), what was read stays and the next look tries again.
    pub fn refresh(&mut self, now_ns: u64) {
        if self.path.as_os_str().is_empty() || (self.polled_ns != 0 && now_ns.saturating_sub(self.polled_ns) < POLL_NS) {
            return;
        }
        self.polled_ns = now_ns;
        let stamp = std::fs::metadata(&self.path).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()));
        if stamp == self.stamp {
            return;
        }
        if stamp.is_none() {
            self.settings = Settings::default();
            self.stamp = None;
        } else if let Some(theme) = std::fs::read_to_string(&self.path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            self.settings = Settings::from_theme(&theme);
            self.stamp = stamp;
        }
    }

    pub fn enabled(&self) -> bool {
        self.settings.enabled
    }

    /// Where the shell windows are drawn relative to where they are placed.
    pub fn offset(&self) -> (i32, i32) {
        if self.settings.enabled { self.offset } else { (0, 0) }
    }

    /// The brightness of layer `layer` (0 Home, 1 the system layer, which is never dimmed), 1 when it is not dimmed. While the panel carries
    /// Home's dim (`panelDim`) Home is drawn at full brightness: the plane takes it whole.
    pub fn dim(&self, layer: usize) -> f32 {
        if self.settings.enabled && !(self.panel_carry && layer == 0) { self.layers[layer].level } else { 1.0 }
    }

    /// The fraction of the user's brightness Home's ease asks of the panel: 1 unless the care is on and Home has been still long enough.
    pub fn home_want(&self) -> f32 {
        if self.settings.enabled && !self.saver { self.layers[0].level } else { 1.0 }
    }

    /// `ctl status`'s `oledcare`: each feature's switch and what it is doing now. `dim` and `offset` are LumaTV's own screens (Home, the
    /// system layer); the app features say what the card in front is drawn with (`window` is 0 while none is looked at).
    pub fn status(&self, now_ns: u64) -> Value {
        let (o, a) = (self.offset(), self.app_offset());
        serde_json::json!({
            "enabled": self.settings.enabled,
            "offset": [o.0, o.1],
            "dim": [self.dim(0), self.dim(1)],
            // what Home's ease asks of the panel while `panelDim` carries it (`dim` is what is drawn: 1 then)
            "homeWant": self.home_want(),
            "beats": self.beats,
            "appDim": {"enabled": self.settings.app_dim, "level": self.app_dim(), "to": self.settings.app_floor, "afterMin": self.settings.app_after_ns / (60 * SEC)},
            "appShift": {"enabled": self.settings.app_shift, "offset": [a.0, a.1]},
            "app": {"window": self.app.window.unwrap_or(0), "stillSec": self.app_still_s(now_ns), "beats": self.app.beats, "engaged": self.app.engaged},
            // the helper asks this before it delivers a camera pop-up (its saver rules): the shell's screensaver is up
            "saver": self.saver,
        })
    }

    /// Whether the last [`Care::step`] changed what is drawn (a beat moved or dimmed a window, or the care was switched off).
    pub fn beat_frame(&self) -> bool {
        self.changed
    }

    /// A key or the pointer: everything is bright again, and the stillness starts over. The card in front is put back too, whether or not
    /// the shell's own care is on.
    pub fn input(&mut self, now_ns: u64) {
        self.app.still_since_ns = now_ns;
        self.app_restore("key or pointer");
        if !self.settings.enabled {
            return;
        }
        for l in &mut self.layers {
            l.still_since_ns = now_ns;
            l.level = 1.0;
        }
    }

    /// Whether either app feature is switched on: the scene looks at the card in front only then.
    pub fn wants_app(&self) -> bool {
        self.settings.app_dim || self.settings.app_shift
    }

    /// The window the app features last looked at.
    pub fn app_window(&self) -> Option<u32> {
        self.app.window
    }

    /// Whether a dim or a shift is on the card in front: a change of its picture must be seen at once (`boxdim::CHECK_NS`).
    pub fn app_engaged(&self) -> bool {
        self.app.engaged
    }

    /// Where the card in front is drawn relative to where it is placed: (0, 0) unless `appShift` has it away.
    pub fn app_offset(&self) -> (i32, i32) {
        if self.settings.app_shift { self.app.offset } else { (0, 0) }
    }

    /// The brightness of the card in front: 1 unless `appDim` has it down.
    pub fn app_dim(&self) -> f32 {
        if self.settings.app_dim { self.app.level } else { 1.0 }
    }

    /// How long the card in front has been still, in seconds (0 while no app is looked at).
    pub fn app_still_s(&self, now_ns: u64) -> u64 {
        if self.app.window.is_some() { now_ns.saturating_sub(self.app.still_since_ns) / SEC } else { 0 }
    }

    /// How many beats the card in front has had since it was last put back.
    pub fn app_beats(&self) -> u64 {
        self.app.beats
    }

    /// The lines the journal is owed (a dim or shift engaged, an app put back).
    pub fn take_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    /// Puts the card in front back at full brightness at the origin, with a journal line when something was on it. Whether the last
    /// [`Care::step`]'s frame changed is told by the return value (and [`Care::beat_frame`]).
    fn app_restore(&mut self, why: &str) -> bool {
        let a = &mut self.app;
        let moved = a.away();
        if a.engaged {
            self.events.push(format!("oledcare: app window {} put back ({why}): brightness 1, offset (0, 0)", a.window.unwrap_or(0)));
        }
        a.level = 1.0;
        a.offset = (0, 0);
        a.next_beat_ns = 0;
        a.engaged = false;
        self.changed |= moved;
        moved
    }

    /// One look at the card in front at a frame about to be planned, after [`Care::step`]: `view` is the card (`None`: none is in front, or
    /// it is not a plain app), `pending` as for `step`. The card still for `appDimAfterMin` is eased down (`appDim`) and walked round the
    /// ring (`appShift`) at beats of its own; it is put back in this very call when it is not still any more, and nothing is ever
    /// engaged under the saver or with both switches off. Returns whether what is drawn changed.
    pub fn step_app(&mut self, now_ns: u64, view: Option<AppView>, pending: bool) -> bool {
        let s = self.settings;
        let Some(v) = view.filter(|_| (s.app_dim || s.app_shift) && !self.saver) else {
            let why = if !(s.app_dim || s.app_shift) {
                "switched off"
            } else if self.saver {
                "screensaver"
            } else {
                "no longer in front"
            };
            let moved = self.app_restore(why);
            self.app = App::default();
            return moved;
        };
        if self.app.window != Some(v.window) {
            let moved = self.app_restore("another window");
            self.app = App { window: Some(v.window), still_since_ns: now_ns, ..App::default() };
            return moved;
        }
        if v.busy || v.changed {
            self.app.still_since_ns = now_ns;
            return self.app_restore(if v.busy { "video or transition" } else { "redrawn" });
        }
        let a = &mut self.app;
        // A switch turned off while it is engaged: that feature is put back, the other one goes on.
        let mut changed = false;
        if !s.app_dim && a.level != 1.0 {
            a.level = 1.0;
            changed = true;
        }
        if !s.app_shift && a.offset != (0, 0) {
            a.offset = (0, 0);
            changed = true;
        }
        let still = now_ns.saturating_sub(a.still_since_ns);
        if still >= s.app_after_ns {
            if a.next_beat_ns == 0 {
                a.next_beat_ns = now_ns;
            }
            if now_ns >= a.next_beat_ns && (pending || now_ns >= a.next_beat_ns + GRACE_NS) {
                a.next_beat_ns = now_ns + s.step_ns;
                a.beats += 1;
                if s.app_shift {
                    changed |= walk_ring(s.radius, &mut a.offset, &mut a.phase);
                }
                if s.app_dim {
                    let level = dim_level(still, s.app_after_ns, s.dim_over_ns, s.app_floor);
                    if level != a.level {
                        a.level = level;
                        changed = true;
                    }
                }
            }
        }
        if changed && !a.engaged && a.away() {
            a.engaged = true;
            let (window, level, offset) = (v.window, a.level, a.offset);
            self.events.push(format!("oledcare: app window {window} still for {} s: engaged (brightness {level}, offset {offset:?})", still / SEC));
        } else if a.engaged && !a.away() {
            a.engaged = false;
        }
        self.changed |= changed;
        changed
    }

    /// One look at a frame about to be planned: `views` is each shell window on the screen (its commit count), `pending` whether any
    /// window has a commit to draw, so a frame is being drawn anyway. A beat that is due is taken when `pending`, or [`GRACE_NS`] after
    /// it was due. Returns whether the beat changed anything.
    pub fn step(&mut self, now_ns: u64, views: [View; LAYERS], pending: bool) -> bool {
        self.changed = false;
        if !self.settings.enabled {
            let moved = self.offset != (0, 0) || self.layers.iter().any(|l| l.level != 1.0);
            self.offset = (0, 0);
            self.phase = 0;
            self.next_beat_ns = 0;
            self.was_shown = false;
            self.layers = Default::default();
            self.changed = moved;
            return moved;
        }
        if self.saver || views.iter().all(Option::is_none) {
            self.was_shown = false;
            self.layers = Default::default();
            return false;
        }
        if self.next_beat_ns == 0 {
            self.next_beat_ns = now_ns + self.settings.step_ns;
        } else if !self.was_shown && self.seen_ns != 0 {
            // The clock stood still while nothing of the shell was on the screen.
            self.next_beat_ns += now_ns.saturating_sub(self.seen_ns);
        }
        self.was_shown = true;
        self.seen_ns = now_ns;
        for (l, v) in self.layers.iter_mut().zip(views) {
            match v {
                Some(commits) if !l.shown => *l = Layer { shown: true, commits, looked_ns: now_ns, still_since_ns: now_ns, level: 1.0 },
                Some(_) => {}
                None => *l = Layer::default(),
            }
        }
        if now_ns < self.next_beat_ns || !(pending || now_ns >= self.next_beat_ns + GRACE_NS) {
            return false;
        }
        self.next_beat_ns = now_ns + self.settings.step_ns;
        self.beats += 1;
        self.walk();
        let s = self.settings;
        for (i, (l, v)) in self.layers.iter_mut().zip(views).enumerate() {
            let Some(commits) = v else { continue };
            if i != 0 {
                continue;
            }
            let secs = now_ns.saturating_sub(l.looked_ns) as f64 / SEC as f64;
            if secs >= 1.0 && commits.saturating_sub(l.commits) as f64 / secs > MOVING_PER_S {
                l.still_since_ns = now_ns;
            }
            l.commits = commits;
            l.looked_ns = now_ns;
            let level = dim_level(now_ns.saturating_sub(l.still_since_ns), s.dim_after_ns, s.dim_over_ns, s.dim_floor);
            if level != l.level {
                l.level = level;
                // drawn only when the panel is not the one dimming
                self.changed |= !self.panel_carry;
            }
        }
        self.changed
    }

    /// One pixel on one axis toward the next point of the ring (the x axis first); the next point once this one is reached.
    fn walk(&mut self) {
        let moved = walk_ring(self.settings.radius, &mut self.offset, &mut self.phase);
        self.changed |= moved;
    }
}

/// One step of [`Care::walk`]'s rule on `offset` and `phase`; whether the offset moved.
fn walk_ring(r: i32, offset: &mut (i32, i32), phase: &mut usize) -> bool {
    let mut target = ring(r, *phase);
    if *offset == target && r > 0 {
        *phase = (*phase + 1) % (8 * r as usize);
        target = ring(r, *phase);
    }
    let before = *offset;
    if offset.0 != target.0 {
        offset.0 += (target.0 - offset.0).signum();
    } else if offset.1 != target.1 {
        offset.1 += (target.1 - offset.1).signum();
    }
    *offset != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MIN: u64 = 60 * SEC;

    fn on() -> Care {
        let mut c = Care::default();
        c.settings.enabled = true;
        c
    }

    #[test]
    fn the_theme_block_is_read_clamped_and_off_by_default() {
        assert_eq!(Settings::from_theme(&json!({})), Settings::default());
        assert!(!Settings::default().enabled);
        let s = Settings::from_theme(&json!({"oledcare": {"enabled": true, "orbitPx": 9, "stepSec": 1, "dimAfterMin": 3, "dimOverMin": 20, "dimTo": 0.1}}));
        assert_eq!((s.enabled, s.radius, s.step_ns, s.dim_after_ns, s.dim_over_ns), (true, 4, 10 * SEC, 3 * MIN, 20 * MIN));
        assert_eq!(s.dim_floor, MIN_DIM_TO, "a dim too deep for the text to read is held at its floor");
        let wrong = Settings::from_theme(&json!({"oledcare": {"enabled": "yes", "orbitPx": "big", "dimTo": null}}));
        assert_eq!(wrong, Settings::default());
    }

    #[test]
    fn the_ring_is_one_pixel_between_neighbours_and_closes() {
        for r in 1..=4 {
            let n = 8 * r as usize;
            for i in 0..n {
                let (a, b) = (ring(r, i), ring(r, i + 1));
                assert_eq!((a.0 - b.0).abs() + (a.1 - b.1).abs(), 1, "r {r} step {i}: {a:?} to {b:?}");
                assert_eq!(a.0.abs().max(a.1.abs()), r, "on the ring");
            }
            assert_eq!(ring(r, 0), ring(r, n), "closed");
        }
        assert_eq!(ring(0, 5), (0, 0));
    }

    #[test]
    fn the_dim_starts_after_the_stillness_and_ends_at_the_floor_in_hundredths() {
        let (after, over) = (5 * MIN, 10 * MIN);
        assert_eq!(dim_level(0, after, over, 0.8), 1.0);
        assert_eq!(dim_level(after, after, over, 0.8), 1.0);
        assert_eq!(dim_level(after + over / 2, after, over, 0.8), 0.9);
        assert_eq!(dim_level(after + over, after, over, 0.8), 0.8);
        assert_eq!(dim_level(after + 100 * over, after, over, 0.8), 0.8);
        assert_eq!(dim_level(after + over, after, over, 1.0), 1.0, "dimTo 1 is no dim");
    }

    /// A beat every 30 s on a still screen, taken 2 s late, one pixel at a time; the offset never jumps and never leaves the ring.
    #[test]
    fn the_orbit_walks_to_the_ring_then_round_it_one_pixel_at_a_time() {
        let mut c = on();
        let mut t = 100 * SEC;
        let mut last = (0, 0);
        let mut seen = std::collections::BTreeSet::new();
        for beat in 0..60 {
            // Calm ticks until the beat is taken: the first look arms, a due beat is held for its grace.
            let mut taken = false;
            for _ in 0..400 {
                t += SEC / 10;
                if c.step(t, [Some(1), None], false) {
                    taken = true;
                    break;
                }
                if c.beats > beat {
                    break;
                }
            }
            assert!(taken || c.beats > beat, "beat {beat} never came");
            let o = c.offset();
            assert!((o.0 - last.0).abs() + (o.1 - last.1).abs() <= 1, "beat {beat}: {last:?} to {o:?} is a jump");
            assert!(o.0.abs() <= 3 && o.1.abs() <= 3);
            seen.insert(o);
            last = o;
        }
        assert_eq!(seen.len(), 24 + 2, "the 24 points of the ring and the two of the walk to it that are not on it, once each");
        assert!(seen.contains(&(-3, -3)) && seen.contains(&(3, 3)) && seen.contains(&(3, -3)) && seen.contains(&(-3, 3)));
    }

    #[test]
    fn a_beat_waits_two_seconds_for_a_frame_drawn_anyway_and_takes_it_when_one_comes() {
        let mut c = on();
        let t0 = 10 * SEC;
        assert!(!c.step(t0, [Some(1), None], false), "armed");
        let due = t0 + 30 * SEC;
        assert!(!c.step(due + SEC, [Some(1), None], false), "due, the screen is at rest: held for its grace");
        assert_eq!(c.beats, 0);
        c.step(due + SEC + 1, [Some(2), None], true);
        assert_eq!(c.beats, 1, "a frame is being drawn: the beat rides on it");
        assert_eq!(c.offset(), (-1, 0));
        let mut d = on();
        d.step(t0, [Some(1), None], false);
        d.step(due + GRACE_NS, [Some(1), None], false);
        assert_eq!(d.beats, 1, "at rest the beat is taken once the grace is over");
        assert!(d.beat_frame());
    }

    #[test]
    fn the_beat_clock_stands_still_while_nothing_of_the_shell_is_shown() {
        let mut c = on();
        c.step(10 * SEC, [Some(1), None], false);
        // 20 s on the screen, then an app for ten minutes, then the shell again.
        c.step(30 * SEC, [Some(1), None], false);
        assert!(!c.step(31 * SEC, [None, None], true));
        assert!(!c.step(10 * MIN, [None, None], true), "hidden: nothing happens");
        assert!(!c.step(10 * MIN + 5 * SEC, [Some(1), None], true), "back: 10 s of the 30 are left");
        assert_eq!(c.beats, 0);
        c.step(10 * MIN + 17 * SEC, [Some(1), None], true);
        assert_eq!(c.beats, 1);
    }

    #[test]
    fn a_still_home_dims_in_steps_a_moving_one_and_a_key_bring_it_back_and_the_system_layer_never_dims() {
        let mut c = on();
        c.settings.dim_after_ns = 2 * MIN;
        c.settings.dim_over_ns = 10 * MIN;
        c.settings.dim_floor = 0.8;
        let mut t = 10 * SEC;
        c.step(t, [Some(0), Some(0)], false);
        let mut levels = Vec::new();
        for _ in 0..40 {
            t += 30 * SEC + GRACE_NS;
            c.step(t, [Some(0), Some(0)], false);
            levels.push(c.dim(0));
        }
        assert!(levels.windows(2).all(|w| w[1] <= w[0]), "never brighter by itself: {levels:?}");
        assert!(levels.iter().all(|l| ((f64::from(*l) * 100.0).round() / 100.0 - f64::from(*l)).abs() < 1e-6), "hundredths");
        assert_eq!(c.dim(0), 0.8, "{levels:?}");
        assert_eq!(c.dim(1), 1.0, "the system layer holds cameras and prompts: not dimmed");
        c.input(t);
        assert_eq!(c.dim(0), 1.0, "a key");
        // Dim again, one commit a beat is a clock, not movement.
        let mut commits = 0;
        for _ in 0..40 {
            t += 30 * SEC + GRACE_NS;
            commits += 1;
            c.step(t, [Some(commits), Some(0)], false);
        }
        assert_eq!(c.dim(0), 0.8);
        // Then Home moves: 10 000 commits in one beat.
        t += 30 * SEC + GRACE_NS;
        c.step(t, [Some(commits + 10_000), Some(0)], false);
        assert_eq!(c.dim(0), 1.0, "a Home that moves is not still");
    }

    #[test]
    fn the_care_stands_aside_while_the_saver_is_up() {
        let mut c = on();
        c.settings.dim_after_ns = MIN;
        c.settings.dim_floor = 0.8;
        let mut t = 10 * SEC;
        c.step(t, [Some(0), None], false);
        for _ in 0..60 {
            t += 30 * SEC + GRACE_NS;
            c.step(t, [Some(0), None], false);
        }
        assert!(c.dim(0) < 1.0);
        c.saver = true;
        t += 30 * SEC + GRACE_NS;
        c.step(t, [Some(0), None], true);
        assert_eq!(c.dim(0), 1.0, "the saver's own dim is the only one");
        let (o, beats) = (c.offset(), c.beats);
        for _ in 0..10 {
            t += 30 * SEC + GRACE_NS;
            c.step(t, [Some(0), None], true);
        }
        assert_eq!((c.offset(), c.beats), (o, beats), "no beats under the saver");
        c.saver = false;
        t += SEC;
        c.step(t, [Some(0), None], false);
        assert_eq!(c.dim(0), 1.0, "and bright when it ends");
    }

    #[test]
    fn a_home_that_appears_starts_bright_and_switching_off_puts_everything_back() {
        let mut c = on();
        c.settings.dim_after_ns = MIN;
        let mut t = 10 * SEC;
        c.step(t, [Some(0), None], false);
        for _ in 0..60 {
            t += 30 * SEC + GRACE_NS;
            c.step(t, [Some(0), None], false);
        }
        assert!(c.dim(0) < 1.0);
        t += SEC;
        c.step(t, [None, Some(0)], false);
        t += SEC;
        c.step(t, [Some(0), Some(0)], false);
        assert_eq!(c.dim(0), 1.0, "Home came back from under an app: new to the eye");
        c.settings.enabled = false;
        assert!(c.step(t + SEC, [Some(0), Some(0)], false), "going back to the origin is a change to draw");
        assert_eq!((c.offset(), c.dim(0)), ((0, 0), 1.0));
        assert!(!c.step(t + 2 * SEC, [Some(0), Some(0)], false), "and then nothing, until it is on again");
    }

    #[test]
    fn the_theme_file_is_read_when_it_changes_and_a_missing_one_is_off() {
        let dir = std::env::temp_dir().join(format!("oledcare-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        let mut c = Care::new(&dir);
        c.refresh(10 * SEC);
        assert!(!c.enabled(), "no file");
        std::fs::write(dir.join("theme/theme.json"), r#"{"oledcare": {"enabled": true, "orbitPx": 2}}"#).unwrap();
        c.refresh(10 * SEC + SEC);
        assert!(!c.enabled(), "looked at a second ago: the poll is 2 s");
        c.refresh(13 * SEC);
        assert!(c.enabled() && c.settings.radius == 2);
        std::fs::write(dir.join("theme/theme.json"), "{ not json").unwrap();
        c.refresh(16 * SEC);
        assert!(c.enabled(), "unreadable now: what was read stays");
        std::fs::remove_file(dir.join("theme/theme.json")).unwrap();
        c.refresh(19 * SEC);
        assert!(!c.enabled(), "gone: off");
        std::fs::remove_dir_all(&dir).ok();
    }
    fn app_on() -> Care {
        let mut c = Care::default();
        c.settings.app_dim = true;
        c.settings.app_shift = true;
        c.settings.app_after_ns = 3 * MIN;
        c.settings.dim_over_ns = 10 * MIN;
        c.settings.app_floor = 0.7;
        c
    }

    fn view() -> Option<AppView> {
        Some(AppView { window: 7, changed: false, busy: false })
    }

    /// Looks at a still app every 2 s (a frame drawn anyway each time, so every due beat is taken) until `until`.
    fn idle_until(c: &mut Care, t: &mut u64, until: u64) {
        while *t < until {
            *t += 2 * SEC;
            c.step_app(*t, view(), true);
        }
    }

    #[test]
    fn the_app_keys_are_read_clamped_and_off_by_default() {
        let d = Settings::default();
        assert!(!d.app_dim && !d.app_shift);
        assert_eq!((d.app_after_ns, d.app_floor), (3 * MIN, 0.75));
        let s = Settings::from_theme(&json!({"oledcare": {"appDim": true, "appShift": true, "appDimAfterMin": 0, "appDimTo": 0.2}}));
        assert!(s.app_dim && s.app_shift);
        assert_eq!((s.app_after_ns, s.app_floor), (MIN, MIN_APP_DIM_TO), "a dim too deep is held at its floor, a wait at its minute");
        let wrong = Settings::from_theme(&json!({"oledcare": {"appDim": "yes", "appShift": 1, "appDimAfterMin": "soon", "appDimTo": null}}));
        assert_eq!(wrong, d);
        assert!(!Care::default().wants_app());
    }

    #[test]
    fn a_still_app_is_left_alone_for_the_wait_then_eased_down_and_walked_and_a_key_puts_it_back_at_once() {
        let mut c = app_on();
        assert!(c.wants_app());
        let mut t = 10 * SEC;
        c.step_app(t, view(), false);
        let start = t;
        idle_until(&mut c, &mut t, start + 3 * MIN - 4 * SEC);
        assert_eq!((c.app_dim(), c.app_offset()), (1.0, (0, 0)), "not before appDimAfterMin");
        idle_until(&mut c, &mut t, start + 3 * MIN + 5 * MIN);
        assert!(c.app_dim() < 1.0 && c.app_dim() > 0.7, "easing down: {}", c.app_dim());
        assert_ne!(c.app_offset(), (0, 0));
        assert!(c.app_offset().0.abs() <= 3 && c.app_offset().1.abs() <= 3);
        let events = c.take_events();
        assert_eq!(events.len(), 1, "engaged once: {events:?}");
        assert!(events[0].contains("engaged"), "{events:?}");
        c.input(t + 1);
        assert_eq!((c.app_dim(), c.app_offset()), (1.0, (0, 0)), "a key: bright and home in the same call");
        let events = c.take_events();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("put back"), "{events:?}");
        // and the wait starts over from the key
        c.step_app(t + 2, view(), false);
        let mut u = t + 2;
        idle_until(&mut c, &mut u, t + 3 * MIN - 5 * SEC);
        assert_eq!(c.app_dim(), 1.0);
        c.input(u);
        assert!(c.take_events().is_empty(), "nothing was engaged: nothing to say");
    }

    #[test]
    fn the_dim_reaches_its_floor_in_hundredths_and_never_lifts_by_itself() {
        let mut c = app_on();
        c.settings.app_shift = false;
        let mut t = 10 * SEC;
        c.step_app(t, view(), false);
        let mut levels = Vec::new();
        for _ in 0..400 {
            t += 2 * SEC;
            c.step_app(t, view(), true);
            levels.push(c.app_dim());
        }
        assert!(levels.windows(2).all(|w| w[1] <= w[0]), "{levels:?}");
        assert_eq!(*levels.last().unwrap(), 0.7);
        assert!(levels.iter().all(|l| ((f64::from(*l) * 100.0).round() / 100.0 - f64::from(*l)).abs() < 1e-6), "hundredths");
        assert_eq!(c.app_offset(), (0, 0), "appShift is its own switch");
    }

    #[test]
    fn a_change_of_its_picture_or_a_video_hole_puts_it_back_in_the_same_look() {
        for why in 0..2 {
            let mut c = app_on();
            let mut t = 10 * SEC;
            c.step_app(t, view(), false);
            idle_until(&mut c, &mut t, 10 * SEC + 8 * MIN);
            assert!(c.app_dim() < 1.0 && c.app_offset() != (0, 0), "engaged before case {why}");
            t += SEC;
            let v = match why {
                0 => AppView { changed: true, ..view().unwrap() },
                _ => AppView { busy: true, ..view().unwrap() },
            };
            assert!(c.step_app(t, Some(v), false), "case {why}: the frame is a change to draw");
            assert_eq!((c.app_dim(), c.app_offset()), (1.0, (0, 0)), "case {why}");
            assert!(c.take_events().iter().any(|e| e.contains("put back")));
            // and then it is not still: nothing engages for the wait again
            let from = t;
            let mut u = t;
            while u < from + 3 * MIN - 4 * SEC {
                u += 2 * SEC;
                c.step_app(u, view(), true);
            }
            assert_eq!((c.app_dim(), c.app_offset()), (1.0, (0, 0)), "case {why}: the wait starts over");
        }
    }

    #[test]
    fn a_hole_present_never_lets_an_app_dim_or_shift() {
        let mut c = app_on();
        let mut t = 10 * SEC;
        let busy = Some(AppView { busy: true, ..view().unwrap() });
        c.step_app(t, busy, false);
        for _ in 0..600 {
            t += 2 * SEC;
            c.step_app(t, busy, true);
            assert_eq!((c.app_dim(), c.app_offset()), (1.0, (0, 0)));
        }
        assert!(c.take_events().is_empty());
    }

    #[test]
    fn the_screensaver_a_switched_off_feature_and_leaving_the_front_stand_it_down() {
        let mut c = app_on();
        let mut t = 10 * SEC;
        c.step_app(t, view(), false);
        idle_until(&mut c, &mut t, 10 * SEC + 8 * MIN);
        assert!(c.app_dim() < 1.0);
        c.saver = true;
        t += 2 * SEC;
        assert!(c.step_app(t, view(), true), "the saver is up: the card is put back");
        assert_eq!((c.app_dim(), c.app_offset()), (1.0, (0, 0)));
        for _ in 0..200 {
            t += 2 * SEC;
            c.step_app(t, view(), true);
            assert_eq!(c.app_dim(), 1.0, "never under the saver");
        }
        c.saver = false;
        let mut c = app_on();
        let mut t = 10 * SEC;
        c.step_app(t, view(), false);
        idle_until(&mut c, &mut t, 10 * SEC + 8 * MIN);
        let (dim, off) = (c.app_dim(), c.app_offset());
        assert!(dim < 1.0 && off != (0, 0));
        c.settings.app_shift = false;
        t += 2 * SEC;
        assert!(c.step_app(t, view(), true));
        assert_eq!((c.app_offset(), c.app_dim() <= dim), ((0, 0), true), "appShift off: home at once, the dim goes on");
        assert!(c.app_dim() < 1.0);
        c.settings.app_dim = false;
        t += 2 * SEC;
        assert!(c.step_app(t, view(), true));
        assert_eq!(c.app_dim(), 1.0, "appDim off: bright at once");
        t += 2 * SEC;
        assert!(!c.step_app(t, None, true), "no card in front: nothing to say");
        assert_eq!(c.app_window(), None);
    }

    #[test]
    fn a_new_window_starts_bright_and_the_shells_own_switch_is_not_needed() {
        let mut c = app_on();
        assert!(!c.settings.enabled, "LumaTV's screens are off");
        let mut t = 10 * SEC;
        c.step_app(t, view(), false);
        idle_until(&mut c, &mut t, 10 * SEC + 8 * MIN);
        assert!(c.app_dim() < 1.0, "the app features run without `enabled`");
        t += SEC;
        assert!(c.step_app(t, Some(AppView { window: 8, ..view().unwrap() }), true), "another app: the old one is put back");
        assert_eq!((c.app_dim(), c.app_offset(), c.app_window()), (1.0, (0, 0), Some(8)));
    }

    #[test]
    fn an_apps_beat_waits_for_a_frame_drawn_anyway_or_the_grace_and_is_a_frame_of_its_own() {
        let mut c = app_on();
        c.settings.app_dim = false;
        let t0 = 10 * SEC;
        c.step_app(t0, view(), false);
        let engage = t0 + 3 * MIN;
        assert!(!c.step_app(engage, view(), false), "engaged: the first beat is due, the screen is at rest");
        assert!(!c.step_app(engage + SEC, view(), false));
        assert_eq!(c.app_offset(), (0, 0));
        assert!(c.step_app(engage + GRACE_NS, view(), false));
        assert!(c.beat_frame());
        assert_eq!(c.app_offset(), (-1, 0));
        assert!(!c.step_app(engage + GRACE_NS + SEC, view(), true), "the next beat is a step away");
        assert!(c.step_app(engage + GRACE_NS + 30 * SEC, view(), true), "a frame drawn anyway carries it");
        assert_eq!(c.app_offset(), (-2, 0));
    }
}
