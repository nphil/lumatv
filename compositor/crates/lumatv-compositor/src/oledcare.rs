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
//! While the shell's screensaver is up the care stands aside (the saver moves, is mostly black and dims over an hour: [`Care::saver`]).
//!
//! **Dim.** Home (the status bar, the clock and the dock stay put for hours on a Home nobody touches, when the saver is off) that has been
//! still (no key or pointer, and fewer than [`MOVING_PER_S`] commits a second) for `dimAfterMin` is dimmed over `dimOverMin` down to
//! `dimTo` of its brightness, in steps of one hundredth, each taken at a beat. The first key or pointer movement brings it back at
//! once (the frame that key changes anyway); a Home that starts moving again (the wallpaper's motion) is bright again, and one that
//! has just appeared starts bright. The system layer is not dimmed: what it holds for long is a camera's pop-up or a prompt that waits
//! for an answer, and the compositor cannot tell a still picture from a video there. The dim multiplies the layer's colour, so it
//! lowers the light of everything Home draws, text included: `dimTo` is not allowed below [`MIN_DIM_TO`], where the palette's
//! second ink still reads 4.5 : 1 (`tests/test_oledcare.py`).

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
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: false, radius: 3, step_ns: 30 * SEC, dim_after_ns: 5 * 60 * SEC, dim_over_ns: 10 * 60 * SEC, dim_floor: 0.85 }
    }
}

const SEC: u64 = 1_000_000_000;

impl Settings {
    /// The `oledcare` object of a theme: `enabled`, `orbitPx`, `stepSec`, `dimAfterMin`, `dimOverMin`, `dimTo`. Out-of-range numbers are
    /// clamped, wrong types are the shipped value.
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

#[derive(Default)]
pub struct Care {
    pub settings: Settings,
    /// The screensaver is up (the shell's `state.saver`): it is the burn-in protection of its own (black, everything moving, dimming over an
    /// hour), so the care stands aside, as it does while no shell window is on the screen.
    pub saver: bool,
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

    /// The brightness of layer `layer` (0 Home, 1 the system layer, which is never dimmed), 1 when it is not dimmed.
    pub fn dim(&self, layer: usize) -> f32 {
        if self.settings.enabled { self.layers[layer].level } else { 1.0 }
    }

    /// Whether the last [`Care::step`] changed what is drawn (a beat moved or dimmed a window, or the care was switched off).
    pub fn beat_frame(&self) -> bool {
        self.changed
    }

    /// A key or the pointer: everything is bright again, and the stillness starts over.
    pub fn input(&mut self, now_ns: u64) {
        if !self.settings.enabled {
            return;
        }
        for l in &mut self.layers {
            l.still_since_ns = now_ns;
            l.level = 1.0;
        }
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
                self.changed = true;
            }
        }
        self.changed
    }

    /// One pixel on one axis toward the next point of the ring (the x axis first); the next point once this one is reached.
    fn walk(&mut self) {
        let r = self.settings.radius;
        let mut target = ring(r, self.phase);
        if self.offset == target && r > 0 {
            self.phase = (self.phase + 1) % (8 * r as usize);
            target = ring(r, self.phase);
        }
        let before = self.offset;
        if self.offset.0 != target.0 {
            self.offset.0 += (target.0 - self.offset.0).signum();
        } else if self.offset.1 != target.1 {
            self.offset.1 += (target.1 - self.offset.1).signum();
        }
        self.changed |= self.offset != before;
    }
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
}
