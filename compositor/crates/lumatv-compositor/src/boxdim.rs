//! Box dim: the bright parts of an app's picture that have not changed for minutes are drawn with less light, the rest is not
//! (`theme.oledcare.boxDim`, off until it passes on the set; the design and its reasons: `docs/research/boxdim-design.md`).
//!
//! The screen is a grid of [`COLS`] x [`ROWS`] cells of [`CELL`] pixels. Each cell knows when its content last changed (from the
//! commits' damage the plan lays out: nothing is read for it), its last two sampled brightnesses (a sample pass the renderer runs
//! every [`SAMPLE_NS`] on a screen that has been still that long), and its gain. A cell still for `boxDimAfterMin` whose two samples
//! agree targets a gain that falls with its brightness, to `boxDimTo` at the brightest; the targets are dilated by one cell and
//! blurred, so a bright shape is dimmed uniformly to past its edge and the feather lies on the dark around it; the gains move toward
//! their targets slowly down (`boxDimRampSec`) and fast up ([`RESTORE_NS`]). The map the renderer multiplies the app's items with is
//! the gains, 8-bit, sampled bilinearly ([`GainMap::at`] is that sampling on the CPU).
//!
//! A key, a window moving, appearing or leaving, a hole changing, a transition, the saver, a capture and damage of [`BIG_CELLS`] or
//! more restart every cell; smaller damage restarts its own cells. While a cell or its neighbour is still and bright, a small change
//! beside it (a clock) keeps the neighbourhood's gain through the dilation, so nothing pulses.

use serde_json::{Map, Value, json};

use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// Cell edge in output pixels.
pub const CELL: i32 = 30;
pub const COLS: usize = (OUTPUT_WIDTH / CELL as u32) as usize;
pub const ROWS: usize = (OUTPUT_HEIGHT / CELL as u32) as usize;
pub const CELLS: usize = COLS * ROWS;

const SEC: u64 = 1_000_000_000;
/// A sample is taken this long after the last one, on a screen still for as long.
pub const SAMPLE_NS: u64 = 4 * SEC;
/// The ramp down steps this often (a step is under two code values).
pub const STEP_NS: u64 = 2 * SEC;
/// The ramp up: a restored cell is at 1 this long after the restart.
pub const RESTORE_NS: u64 = 150_000_000;
/// Damage touching this many cells in one frame is a change of the picture: every cell restarts.
pub const BIG_CELLS: usize = 16;
/// The lowest `boxDimTo`.
pub const MIN_FLOOR: f32 = 0.75;
/// Two samples of a cell agree within this many code values.
const AGREE: u8 = 2;
/// Above `min_luma` the gain falls linearly over this much brightness to the floor.
const LUMA_SPAN: f32 = 0.3;
/// The ramp's time step is capped: a step after a long pause is one step, not a jump.
const MAX_DT_NS: u64 = 2 * STEP_NS;
/// The first samples whose cost is logged (a measurement on the set, not a watch).
const SAMPLE_LINES: u64 = 5;

/// The `boxDim*` keys of `theme.oledcare`, with the shipped values for what the theme leaves out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub on: bool,
    /// Still for this long before a cell may dim.
    pub after_ns: u64,
    /// A cell goes from 1 to `floor` over this long.
    pub ramp_ns: u64,
    /// The gain of the brightest content, [`MIN_FLOOR`] to 1.
    pub floor: f32,
    /// Brightness (0 to 1) at and below which nothing is dimmed.
    pub min_luma: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { on: false, after_ns: 3 * 60 * SEC, ramp_ns: 60 * SEC, floor: 0.8, min_luma: 0.35 }
    }
}

impl Settings {
    /// From the `oledcare` object of a theme: `boxDim`, `boxDimAfterMin`, `boxDimRampSec`, `boxDimTo`, `boxDimMinLuma`. Out-of-range numbers
    /// are clamped, wrong types are the shipped value.
    pub fn from_object(o: &Map<String, Value>) -> Settings {
        let d = Settings::default();
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
        Settings {
            on: o.get("boxDim").and_then(Value::as_bool).unwrap_or(d.on),
            after_ns: num("boxDimAfterMin").map_or(d.after_ns, |v| (v.clamp(1.0, 120.0) * 60.0 * SEC as f64) as u64),
            ramp_ns: num("boxDimRampSec").map_or(d.ramp_ns, |v| (v.clamp(5.0, 600.0) * SEC as f64) as u64),
            floor: num("boxDimTo").map_or(d.floor, |v| (v as f32).clamp(MIN_FLOOR, 1.0)),
            min_luma: num("boxDimMinLuma").map_or(d.min_luma, |v| (v as f32).clamp(0.1, 0.9)),
        }
    }

    pub fn from_theme(theme: &Value) -> Settings {
        theme.get("oledcare").and_then(Value::as_object).map_or_else(Settings::default, Settings::from_object)
    }

    /// The target gain of a still cell of brightness `luma`: 1 up to `min_luma`, the floor from `min_luma + LUMA_SPAN` on.
    pub fn gain_for(&self, luma: u8) -> f32 {
        let t = ((luma as f32 / 255.0 - self.min_luma) / LUMA_SPAN).clamp(0.0, 1.0);
        1.0 - (1.0 - self.floor) * t
    }
}

/// The gains, one byte a cell (255: untouched), row 0 the top of the screen.
#[derive(Clone, PartialEq, Eq)]
pub struct GainMap {
    pub cells: [u8; CELLS],
}

impl Default for GainMap {
    fn default() -> Self {
        GainMap { cells: [255; CELLS] }
    }
}

impl GainMap {
    pub fn is_flat(&self) -> bool {
        self.cells.iter().all(|c| *c == 255)
    }

    pub fn lowest(&self) -> u8 {
        self.cells.iter().copied().min().unwrap_or(255)
    }

    /// Cells below 1.
    pub fn dimmed(&self) -> usize {
        self.cells.iter().filter(|c| **c < 255).count()
    }

    /// The gain at output pixel (`x`, `y`), 0 to 1: the map sampled bilinearly with texel centres at the cells' centres and the edges
    /// clamped, as the GL renderer's texture is.
    pub fn at(&self, x: i32, y: i32) -> f32 {
        let axis = |p: i32, n: usize| {
            let u = ((p as f32 + 0.5) / CELL as f32 - 0.5).clamp(0.0, (n - 1) as f32);
            let i0 = u as usize;
            (i0, (i0 + 1).min(n - 1), u - i0 as f32)
        };
        let (x0, x1, tx) = axis(x, COLS);
        let (y0, y1, ty) = axis(y, ROWS);
        let g = |c: usize, r: usize| self.cells[r * COLS + c] as f32 / 255.0;
        let top = g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx;
        let bottom = g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

/// The cells `r` touches, as inclusive column and row ranges; `None` when it touches none.
pub fn cells_of(r: &Rect) -> Option<(usize, usize, usize, usize)> {
    let r = r.intersect(&Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32));
    if r.is_empty() {
        return None;
    }
    let c0 = (r.x / CELL) as usize;
    let r0 = (r.y / CELL) as usize;
    let c1 = ((r.right() - 1) / CELL) as usize;
    let r1 = ((r.bottom() - 1) / CELL) as usize;
    Some((c0, r0, c1.min(COLS - 1), r1.min(ROWS - 1)))
}

#[derive(Clone, Copy)]
struct Cell {
    /// When its content last changed (damage of its own); the whole grid's restart is `BoxDim::epoch_ns`.
    since_ns: u64,
    sample_ns: u64,
    prev_sample_ns: u64,
    luma: u8,
    prev_luma: u8,
    gain: f32,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { since_ns: 0, sample_ns: 0, prev_sample_ns: 0, luma: 0, prev_luma: 0, gain: 1.0 }
    }
}

/// Why the grid last restarted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    Key,
    Change,
    Appeared,
    Transition,
    Saver,
    Capture,
    Off,
    Hidden,
}

impl Restart {
    fn name(self) -> &'static str {
        match self {
            Restart::Key => "key",
            Restart::Change => "change",
            Restart::Appeared => "appeared",
            Restart::Transition => "transition",
            Restart::Saver => "saver",
            Restart::Capture => "capture",
            Restart::Off => "off",
            Restart::Hidden => "hidden",
        }
    }

    /// The map goes to 1 in one step, not over [`RESTORE_NS`]: nothing of the picture shows the step (a capture wants the picture as
    /// drawn in its first frame, the saver covers the screen, nothing is drawn through the map while it is off or no app shows).
    fn snaps(self) -> bool {
        matches!(self, Restart::Saver | Restart::Capture | Restart::Off | Restart::Hidden)
    }
}

/// The state of the box dim: the grid, its map, and what the plan tells it each frame.
pub struct BoxDim {
    /// A screenshot or recording is armed (set by the state before each plan): the map stays at 1, the picture written is the app's own.
    pub capturing: bool,
    cells: Box<[Cell; CELLS]>,
    target: Box<[f32; CELLS]>,
    scratch: Box<[f32; CELLS]>,
    /// Every cell counts as changed at this time (a restart is one write).
    epoch_ns: u64,
    /// The last damage of any size, for the sample's stillness.
    damage_ns: u64,
    restart: Restart,
    /// The gains move toward 1 (a restart while something was dimmed): a step every frame until they are there.
    restoring: bool,
    snap: bool,
    started: bool,
    shown: bool,
    shown_last: bool,
    blocked: bool,
    last_sample_ns: u64,
    last_step_ns: u64,
    map: GainMap,
    serial: u64,
    engaged: bool,
    changed: bool,
    pub samples: u64,
    pub steps: u64,
    /// The longest a sample pass took (us), for `ctl status`.
    pub sample_us_max: u32,
    journal: Vec<String>,
}

impl Default for BoxDim {
    fn default() -> Self {
        BoxDim {
            capturing: false,
            cells: Box::new([Cell::default(); CELLS]),
            target: Box::new([1.0; CELLS]),
            scratch: Box::new([1.0; CELLS]),
            epoch_ns: 0,
            damage_ns: 0,
            restart: Restart::Appeared,
            restoring: false,
            snap: false,
            started: false,
            shown: false,
            shown_last: false,
            blocked: false,
            last_sample_ns: 0,
            last_step_ns: 0,
            map: GainMap::default(),
            serial: 0,
            engaged: false,
            changed: false,
            samples: 0,
            steps: 0,
            sample_us_max: 0,
            journal: Vec::new(),
        }
    }
}

impl BoxDim {
    /// Every cell starts over at `now_ns`: still from now, its samples no longer of this content.
    pub fn restart(&mut self, now_ns: u64, why: Restart) {
        self.epoch_ns = now_ns.max(self.epoch_ns);
        self.damage_ns = self.damage_ns.max(now_ns);
        self.restart = why;
        self.snap |= why.snaps();
        if self.engaged && !self.restoring {
            self.restoring = true;
            // The ramp up is timed from here, not from the last step seconds ago.
            self.last_step_ns = now_ns;
        }
    }

    /// A key or the pointer.
    pub fn input(&mut self, now_ns: u64) {
        self.restart(now_ns, Restart::Key);
    }

    /// A plan begins: no app item is on it yet.
    pub fn begin(&mut self) {
        self.shown = false;
    }

    /// An app item of the plan at `rect` (output pixels) with its damage: `None` the whole rect, `Some` the rectangles (empty: nothing new).
    /// Damage of [`BIG_CELLS`] cells or more restarts the grid; less restarts the cells it touches.
    pub fn seen(&mut self, now_ns: u64, rect: Rect, damage: Option<&[Rect]>) {
        self.shown = true;
        let mut touched = 0usize;
        let mut rects: [Option<(usize, usize, usize, usize)>; 8] = [None; 8];
        let mut n = 0;
        let mut big = false;
        let mut add = |r: &Rect| {
            if let Some(c) = cells_of(r) {
                touched += (c.2 - c.0 + 1) * (c.3 - c.1 + 1);
                if n < rects.len() {
                    rects[n] = Some(c);
                    n += 1;
                } else {
                    big = true;
                }
            }
        };
        match damage {
            None => add(&rect),
            Some(d) => {
                if d.is_empty() {
                    return;
                }
                for r in d {
                    add(&r.intersect(&rect));
                }
            }
        }
        if touched == 0 {
            return;
        }
        self.damage_ns = self.damage_ns.max(now_ns);
        if big || touched >= BIG_CELLS {
            self.restart(now_ns, Restart::Change);
            return;
        }
        for (c0, r0, c1, r1) in rects.iter().flatten() {
            for r in *r0..=*r1 {
                for c in *c0..=*c1 {
                    self.cells[r * COLS + c].since_ns = now_ns;
                }
            }
        }
    }

    /// Whether the renderer should sample the screen now: on, an app shown, nothing blocking, still (no damage, no key) and unsampled for
    /// [`SAMPLE_NS`].
    pub fn sample_due(&self, now_ns: u64, s: &Settings) -> bool {
        s.on && self.shown
            && !self.blocked
            && now_ns.saturating_sub(self.damage_ns.max(self.epoch_ns)) >= SAMPLE_NS
            && now_ns.saturating_sub(self.last_sample_ns) >= SAMPLE_NS
    }

    /// The output could not sample now: the next try is a sample period away, not the next tick.
    pub fn sample_skipped(&mut self, now_ns: u64) {
        self.last_sample_ns = now_ns;
    }

    /// The brightness of every cell (8-bit luma, row 0 the top), as the renderer sampled it at `now_ns`, and what the pass took (us: the
    /// first few are logged, the worst is reported). A cell whose brightness moved since its last sample with no damage recorded restarts:
    /// a change the damage did not say.
    pub fn take_sample(&mut self, now_ns: u64, luma: &[u8; CELLS], cost_us: u32) {
        let epoch = self.epoch_ns;
        for (c, l) in self.cells.iter_mut().zip(luma) {
            let changed_at = c.since_ns.max(epoch);
            if c.sample_ns > changed_at && c.luma.abs_diff(*l) > AGREE {
                c.since_ns = now_ns;
            }
            c.prev_luma = c.luma;
            c.prev_sample_ns = c.sample_ns;
            c.luma = *l;
            c.sample_ns = now_ns;
        }
        self.last_sample_ns = now_ns;
        self.samples += 1;
        self.sample_us_max = self.sample_us_max.max(cost_us);
        if self.samples <= SAMPLE_LINES {
            log::info!("boxdim: sample {} took {cost_us} us", self.samples);
        }
    }

    /// The gain map while any cell is below 1, with a serial that changes when the map does (the renderer re-uploads on it).
    pub fn map(&self) -> Option<(&GainMap, u64)> {
        self.engaged.then_some((&self.map, self.serial))
    }

    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// Whether the last [`BoxDim::step`] changed the map.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Journal lines since the last call.
    pub fn take_journal(&mut self) -> Vec<String> {
        std::mem::take(&mut self.journal)
    }

    /// One plan: `blocked` says the care stands aside (and why); `scene_changed` that a window appeared, moved, left or restacked or a
    /// hole changed. Moves the gains and returns the output rectangle to redraw when the map changed.
    pub fn step(&mut self, now_ns: u64, s: &Settings, blocked: Option<Restart>, scene_changed: bool) -> Option<Rect> {
        self.changed = false;
        if !s.on {
            if self.engaged || self.started {
                self.restart(now_ns, Restart::Off);
            }
            self.started = false;
            self.blocked = true;
        } else {
            if !self.started {
                self.started = true;
                self.restart(now_ns, Restart::Appeared);
            }
            if let Some(why) = blocked {
                self.restart(now_ns, why);
            } else if scene_changed {
                self.restart(now_ns, Restart::Change);
            } else if self.shown && !self.shown_last {
                self.restart(now_ns, Restart::Appeared);
            }
            if !self.shown {
                self.restart(now_ns, Restart::Hidden);
            }
            self.blocked = blocked.is_some() || !self.shown;
        }
        self.shown_last = self.shown;
        // Nothing to do: no cell is dimmed and none can be yet (every cell restarted less than the wait ago).
        if !self.engaged && !self.snap && (self.blocked || now_ns.saturating_sub(self.epoch_ns) < s.after_ns) {
            return None;
        }
        if !self.restoring && !self.snap && now_ns.saturating_sub(self.last_step_ns) < STEP_NS {
            return None;
        }
        let dt = now_ns.saturating_sub(self.last_step_ns).min(MAX_DT_NS) as f32;
        self.last_step_ns = now_ns;
        self.steps += 1;
        if self.snap {
            for c in self.cells.iter_mut() {
                c.gain = 1.0;
            }
            self.restoring = false;
        } else {
            if self.blocked {
                self.target.fill(1.0);
            } else {
                self.targets(now_ns, s);
            }
            let down = (1.0 - s.floor) * dt / s.ramp_ns.max(1) as f32;
            let up = (1.0 - s.floor) * dt / RESTORE_NS as f32;
            let mut rising = false;
            for (c, t) in self.cells.iter_mut().zip(self.target.iter()) {
                if *t < c.gain {
                    c.gain = (c.gain - down).max(*t);
                } else if *t > c.gain {
                    c.gain = (c.gain + up).min(*t);
                    rising |= c.gain < *t;
                }
            }
            self.restoring = rising;
        }
        self.snap = false;
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for (i, c) in self.cells.iter().enumerate() {
            let q = (c.gain * 255.0).round().clamp(0.0, 255.0) as u8;
            if q != self.map.cells[i] {
                self.map.cells[i] = q;
                let (col, row) = (i % COLS, i / COLS);
                bounds = Some(match bounds {
                    None => (col, row, col, row),
                    Some((c0, r0, c1, r1)) => (c0.min(col), r0.min(row), c1.max(col), r1.max(row)),
                });
            }
        }
        let was = self.engaged;
        self.engaged = !self.map.is_flat();
        if self.engaged && !was {
            self.journal.push(format!("oledcare: boxDim on: {} cells, lowest {:.2}", self.map.dimmed(), self.map.lowest() as f32 / 255.0));
        } else if was && !self.engaged {
            self.journal.push(format!("oledcare: boxDim off ({})", self.restart.name()));
        }
        let (c0, r0, c1, r1) = bounds?;
        self.changed = true;
        self.serial += 1;
        // One cell of margin: the bilinear sampling reaches into the neighbours.
        let x0 = (c0.saturating_sub(1) * CELL as usize) as i32;
        let y0 = (r0.saturating_sub(1) * CELL as usize) as i32;
        let x1 = (((c1 + 2) * CELL as usize) as i32).min(OUTPUT_WIDTH as i32);
        let y1 = (((r1 + 2) * CELL as usize) as i32).min(OUTPUT_HEIGHT as i32);
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    /// Every cell's target gain into `target`: its own by stillness and brightness, dilated by one cell, blurred by [1 2 1] on both axes.
    fn targets(&mut self, now_ns: u64, s: &Settings) {
        let epoch = self.epoch_ns;
        let min_luma = (s.min_luma * 255.0) as u8;
        for (c, t) in self.cells.iter().zip(self.scratch.iter_mut()) {
            let changed_at = c.since_ns.max(epoch);
            let still = now_ns.saturating_sub(changed_at) >= s.after_ns;
            let agreed = c.prev_sample_ns > changed_at && c.luma.abs_diff(c.prev_luma) <= AGREE;
            *t = if still && agreed && c.luma > min_luma { s.gain_for(c.luma) } else { 1.0 };
        }
        // Dilate: the lowest of the 3 x 3 around.
        for r in 0..ROWS {
            for c in 0..COLS {
                let mut m = 1.0f32;
                for rr in r.saturating_sub(1)..=(r + 1).min(ROWS - 1) {
                    for cc in c.saturating_sub(1)..=(c + 1).min(COLS - 1) {
                        m = m.min(self.scratch[rr * COLS + cc]);
                    }
                }
                self.target[r * COLS + c] = m;
            }
        }
        // Blur rows, then columns, edges replicated.
        for r in 0..ROWS {
            for c in 0..COLS {
                let at = |cc: usize| self.target[r * COLS + cc.clamp(0, COLS - 1)];
                self.scratch[r * COLS + c] = (at(c.saturating_sub(1)) + 2.0 * at(c) + at((c + 1).min(COLS - 1))) * 0.25;
            }
        }
        for r in 0..ROWS {
            for c in 0..COLS {
                let at = |rr: usize| self.scratch[rr * COLS + c];
                self.target[r * COLS + c] = (at(r.saturating_sub(1)) + 2.0 * at(r) + at((r + 1).min(ROWS - 1))) * 0.25;
            }
        }
    }

    /// The last sampled brightness of cell (`col`, `row`).
    pub fn luma(&self, col: usize, row: usize) -> u8 {
        self.cells[row * COLS + col].luma
    }

    /// The still time of the grid as a whole: since the last restart.
    pub fn still_ns(&self, now_ns: u64) -> u64 {
        now_ns.saturating_sub(self.epoch_ns)
    }

    /// For `ctl status`.
    pub fn report(&self, now_ns: u64, s: &Settings) -> Value {
        json!({
            "on": s.on,
            "engaged": self.engaged,
            "cells": self.map.dimmed(),
            "lowest": (self.map.lowest() as f64 / 255.0 * 100.0).round() / 100.0,
            "stillSec": self.still_ns(now_ns) / SEC,
            "samples": self.samples,
            "steps": self.steps,
            "sampleUsMax": self.sample_us_max,
            "restart": self.restart.name(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60 * SEC;

    fn on() -> Settings {
        Settings { on: true, after_ns: 2 * MIN, ramp_ns: 20 * SEC, floor: 0.8, min_luma: 0.35 }
    }

    fn full() -> Rect {
        Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32)
    }

    /// A white box of cells (`c0..=c1`, `r0..=r1`) on black.
    fn picture(c0: usize, r0: usize, c1: usize, r1: usize) -> [u8; CELLS] {
        let mut l = [0u8; CELLS];
        for r in r0..=r1 {
            for c in c0..=c1 {
                l[r * COLS + c] = 255;
            }
        }
        l
    }

    /// A still screen showing `pic` from `t`: plans every 2 s, samples every 4 s, for `for_ns`. Returns the time reached.
    fn still(b: &mut BoxDim, s: &Settings, pic: &[u8; CELLS], mut t: u64, for_ns: u64) -> u64 {
        let end = t + for_ns;
        while t < end {
            t += 2 * SEC;
            b.begin();
            b.seen(t, full(), Some(&[]));
            b.step(t, s, None, false);
            if b.sample_due(t, s) {
                b.take_sample(t, pic, 0);
            }
        }
        t
    }

    #[test]
    fn the_theme_keys_are_read_clamped_and_wrong_types_are_the_defaults() {
        let d = Settings::default();
        assert!(!d.on);
        let s = Settings::from_theme(&json!({"oledcare": {"boxDim": true, "boxDimAfterMin": 0.1, "boxDimRampSec": 1000, "boxDimTo": 0.1, "boxDimMinLuma": 2}}));
        assert_eq!((s.on, s.after_ns, s.ramp_ns, s.floor, s.min_luma), (true, MIN, 600 * SEC, MIN_FLOOR, 0.9));
        let wrong = Settings::from_theme(&json!({"oledcare": {"boxDim": "yes", "boxDimAfterMin": "soon", "boxDimTo": null}}));
        assert_eq!(wrong, d);
        assert_eq!(Settings::from_theme(&json!({})), d);
    }

    #[test]
    fn the_gain_follows_brightness_with_no_step_in_it() {
        let s = on();
        assert_eq!(s.gain_for(0), 1.0);
        assert_eq!(s.gain_for((0.35 * 255.0) as u8), 1.0);
        assert_eq!(s.gain_for(255), 0.8);
        let mut last = 1.0;
        for l in 0..=255u8 {
            let g = s.gain_for(l);
            assert!(g <= last && last - g < 0.01, "monotone and gentle at {l}: {g} after {last}");
            last = g;
        }
    }

    #[test]
    fn cells_of_a_rectangle_are_the_ones_it_touches() {
        assert_eq!(cells_of(&Rect::new(0, 0, 30, 30)), Some((0, 0, 0, 0)));
        assert_eq!(cells_of(&Rect::new(29, 29, 2, 2)), Some((0, 0, 1, 1)));
        assert_eq!(cells_of(&full()), Some((0, 0, COLS - 1, ROWS - 1)));
        assert_eq!(cells_of(&Rect::new(-10, -10, 5, 5)), None);
        assert_eq!(cells_of(&Rect::new(1900, 1070, 100, 100)), Some((63, 35, 63, 35)));
    }

    #[test]
    fn off_does_nothing_and_a_still_bright_box_dims_after_the_wait_over_the_ramp() {
        let mut b = BoxDim::default();
        let pic = picture(10, 10, 20, 20);
        let t = still(&mut b, &Settings::default(), &pic, SEC, 10 * MIN);
        assert!(!b.engaged() && b.samples == 0 && b.steps == 0, "off: no sample, no step");
        let s = on();
        let mut b = BoxDim::default();
        let t = still(&mut b, &s, &pic, t, 2 * MIN - 10 * SEC);
        assert!(!b.engaged(), "not before the wait");
        assert!(b.samples >= 20, "sampled every 4 s: {}", b.samples);
        let t = still(&mut b, &s, &pic, t, 20 * SEC);
        assert!(b.engaged(), "engaged after the wait");
        let (map, _) = b.map().unwrap();
        let early = map.cells[15 * COLS + 15];
        assert!((200..255).contains(&early), "part of the way in: {early}");
        let t = still(&mut b, &s, &pic, t, 30 * SEC);
        let (map, _) = b.map().unwrap();
        assert_eq!(map.cells[15 * COLS + 15], 204, "the floor, 0.8, after the ramp");
        assert_eq!(map.cells[10 * COLS + 10], 204, "the box's own edge cell is at the floor");
        assert_eq!(map.cells[11 * COLS + 9], 217, "the ring one cell out is the feather's first step, 0.85");
        assert!(map.cells[9 * COLS + 9] > 217 && map.cells[9 * COLS + 9] < 240, "its corner is rounded: {}", map.cells[9 * COLS + 9]);
        assert_eq!(map.cells[8 * COLS + 15], 242, "two cells out, 0.95");
        assert_eq!(map.cells[6 * COLS + 15], 255, "the dark beyond is untouched");
        assert_eq!(map.cells[30 * COLS + 40], 255);
        assert!(b.take_journal().iter().any(|l| l.starts_with("oledcare: boxDim on:")), "journalled");
        let _ = t;
    }

    #[test]
    fn a_key_restores_within_the_restore_time_and_the_wait_starts_over() {
        let s = on();
        let mut b = BoxDim::default();
        let pic = picture(0, 0, COLS - 1, ROWS - 1);
        let mut t = still(&mut b, &s, &pic, SEC, 3 * MIN);
        assert_eq!(b.map().unwrap().0.lowest(), 204);
        b.input(t);
        let mut frames = 0;
        while b.engaged() {
            t += 16_666_667;
            frames += 1;
            b.begin();
            b.seen(t, full(), Some(&[]));
            let r = b.step(t, &s, None, false);
            assert!(r.is_some() || !b.engaged(), "every frame of the ramp up redraws");
            assert!(frames < 20, "gone within 250 ms");
        }
        assert!(frames >= 4, "ramped, not snapped: {frames} frames");
        assert!(b.take_journal().iter().any(|l| l == "oledcare: boxDim off (key)"));
        let t = still(&mut b, &s, &pic, t, 2 * MIN - 10 * SEC);
        assert!(!b.engaged(), "still again for the full wait first");
        let _ = still(&mut b, &s, &pic, t, 20 * SEC);
        assert!(b.engaged());
    }

    #[test]
    fn small_damage_restarts_its_cells_only_and_a_clock_in_a_dimmed_region_keeps_its_gain() {
        let s = on();
        let mut b = BoxDim::default();
        let pic = picture(0, 0, COLS - 1, ROWS - 1);
        let mut t = still(&mut b, &s, &pic, SEC, 3 * MIN);
        assert_eq!(b.map().unwrap().0.lowest(), 204);
        // A clock of 3 x 1 cells ticks once a minute in the middle.
        for _ in 0..3 {
            t += 2 * SEC;
            b.begin();
            b.seen(t, full(), Some(&[Rect::new(900, 500, 90, 30)]));
            b.step(t, &s, None, false);
            assert!(b.engaged(), "a small change does not restore the screen");
            assert_eq!(b.map().unwrap().0.cells[16 * COLS + 30], 204, "the clock's cell keeps the region's gain through its neighbours");
            t = still(&mut b, &s, &pic, t, MIN);
        }
        // The clock alone, on black: its own cells never dim, the dark around is at 1.
        let mut b = BoxDim::default();
        let clock = picture(30, 16, 32, 16);
        let mut t = SEC;
        for _ in 0..4 {
            t = still(&mut b, &s, &clock, t, MIN);
            t += 2 * SEC;
            b.begin();
            b.seen(t, full(), Some(&[Rect::new(900, 480, 90, 30)]));
            b.step(t, &s, None, false);
        }
        assert!(!b.engaged(), "a cell that changes every minute never dims");
    }

    #[test]
    fn big_damage_a_scene_change_a_transition_the_saver_and_a_capture_restart_everything() {
        let s = on();
        let pic = picture(0, 0, COLS - 1, ROWS - 1);
        let engaged = || {
            let mut b = BoxDim::default();
            let t = still(&mut b, &s, &pic, SEC, 3 * MIN);
            assert!(b.engaged());
            (b, t)
        };
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), Some(&[Rect::new(0, 0, 400, 200)]));
        b.step(t + SEC, &s, None, false);
        assert!(b.restoring, "16 cells and more: the grid restarts");
        assert_eq!(b.restart, Restart::Change);
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), None);
        b.step(t + SEC, &s, None, false);
        assert!(b.restoring, "whole-surface damage");
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), Some(&[]));
        b.step(t + SEC, &s, None, true);
        assert!(b.restoring && b.restart == Restart::Change, "a window moved or appeared");
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), Some(&[]));
        let r = b.step(t + SEC, &s, Some(Restart::Transition), false);
        assert!(r.is_none() && b.restoring, "the frame of the restart itself moves nothing yet");
        let r = b.step(t + SEC + 16_000_000, &s, Some(Restart::Transition), false);
        assert!(r.is_some() && b.engaged() && b.restoring, "a transition ramps the map out, it does not snap");
        for why in [Restart::Saver, Restart::Capture] {
            let (mut b, t) = engaged();
            b.begin();
            b.seen(t + SEC, full(), Some(&[]));
            let r = b.step(t + SEC, &s, Some(why), false);
            assert!(r.is_some() && !b.engaged(), "{why:?} snaps the map to 1 in one step");
            assert!(!b.sample_due(t + 10 * SEC, &s), "and nothing is sampled while it is up");
        }
        let (mut b, t) = engaged();
        b.begin();
        b.step(t + SEC, &s, None, false);
        assert!(!b.engaged(), "no app on the screen: flat at once");
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), Some(&[]));
        let r = b.step(t + SEC, &Settings { on: false, ..s }, None, false);
        assert!(r.is_some() && !b.engaged() && b.map().is_none(), "switched off: one step back to 1");
        assert!(b.take_journal().iter().any(|l| l == "oledcare: boxDim off (off)"));
    }

    #[test]
    fn a_sample_that_moved_without_damage_restarts_the_cell_and_dark_content_never_dims() {
        let s = on();
        let mut b = BoxDim::default();
        let bright = picture(0, 0, COLS - 1, ROWS - 1);
        let t = still(&mut b, &s, &bright, SEC, 2 * MIN - 10 * SEC);
        let mut moved = bright;
        moved[5 * COLS + 5] = 200;
        b.take_sample(t + SEC, &moved, 0);
        let t = still(&mut b, &s, &moved, t + SEC, 30 * SEC);
        assert!(b.engaged());
        let (map, _) = b.map().unwrap();
        assert!(map.cells[20 * COLS + 40] < 255, "the rest dims");
        let ring = map.cells[5 * COLS + 5];
        assert!(ring < 255, "the moved cell is held at its neighbours' gain by the dilation: {ring}");
        let mut b = BoxDim::default();
        let dark = {
            let mut l = [0u8; CELLS];
            l.iter_mut().for_each(|c| *c = (0.3 * 255.0) as u8);
            l
        };
        still(&mut b, &s, &dark, SEC, 5 * MIN);
        assert!(!b.engaged(), "below min_luma nothing dims");
        let _ = t;
    }

    #[test]
    fn the_step_damages_the_changed_cells_with_a_margin_and_no_work_is_done_on_a_still_dark_screen() {
        let s = on();
        let mut b = BoxDim::default();
        let pic = picture(10, 10, 12, 12);
        let mut t = still(&mut b, &s, &pic, SEC, 2 * MIN);
        let mut r = None;
        for _ in 0..3 {
            t += 2 * SEC;
            b.begin();
            b.seen(t, full(), Some(&[]));
            r = r.or(b.step(t, &s, None, false));
        }
        let r = r.expect("a step changed the map");
        // The box is cells 10..=12, dilated to 9..=13, blurred to 8..=14, plus one cell of margin: 7..=15 -> 210..480.
        assert_eq!((r.x, r.y, r.right(), r.bottom()), (210, 210, 480, 480), "{r:?}");
        let mut b = BoxDim::default();
        let dark = [0u8; CELLS];
        let t = still(&mut b, &s, &dark, SEC, 5 * MIN);
        let steps = b.steps;
        for i in 0..100 {
            b.begin();
            b.seen(t + i * SEC, full(), Some(&[]));
            assert!(b.step(t + i * SEC, &s, None, false).is_none());
        }
        assert!(b.steps - steps <= 50, "a target pass every 2 s at most on a still screen: {}", b.steps - steps);
    }

    #[test]
    fn the_map_is_sampled_bilinearly_with_feathered_edges() {
        let mut m = GainMap::default();
        for r in 0..ROWS {
            for c in 0..32 {
                m.cells[r * COLS + c] = 204;
            }
        }
        assert_eq!(m.at(0, 0), 0.8);
        assert_eq!(m.at(1919, 500), 1.0);
        // From the centre of cell 31 (x 945) to the centre of cell 32 (x 975) the gain climbs linearly.
        assert!((m.at(944, 10) - 0.8).abs() < 1e-5);
        assert!((m.at(975, 10) - 1.0).abs() < 1e-5);
        assert!((m.at(959, 10) - 0.9).abs() < 0.01, "{}", m.at(959, 10));
        let mut last = m.at(0, 100);
        for x in 1..1920 {
            let g = m.at(x, 100);
            assert!(g >= last - 1e-6 && g - last <= 0.2 / 30.0 + 1e-5, "a step of {} at {x}", g - last);
            last = g;
        }
        assert_eq!(m.dimmed(), 32 * ROWS);
        assert_eq!(m.lowest(), 204);
        assert!(!m.is_flat() && GainMap::default().is_flat());
    }
}
