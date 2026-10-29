//! Box dim: the bright parts of an app's picture that have not changed for minutes are drawn with less light, the rest is not
//! (`theme.oledcare.boxDim`, off until it passes on the set; the design and its reasons: `docs/research/boxdim-design.md`). The same grid
//! tells the OLED care's app features (`appDim`, `appShift`) a still app from a changing one ([`BoxDim::take_app_change`]).
//!
//! The screen is a grid of [`COLS`] x [`ROWS`] cells of [`CELL`] pixels. What says a cell changed is its picture, not its commits: on the
//! set LG's browser and Plex commit the same frame again every 4 to 5 s with the whole surface damaged, a static page included (round
//! 16), and a rule that took a commit for a change never saw them still. A picture changes through a commit or not at all, so the
//! commits' damage says which cells may have changed, and a sample says whether they did. A sample is the app's pictures composed at the
//! grid's resolution on the GPU, three 8-bit numbers a cell ([`Sig`]), read back once the GPU is done (`Output::sample_begin`,
//! `Output::sample_take`): the loop never waits for it. One is begun [`SAMPLE_NS`] after the last when anything was damaged since, on a
//! tick that drew nothing (twice that on any tick, for a screen that commits every frame). While a dimmed cell or an engaged app has
//! damage no sample has judged, a check is begun after the frame that drew it, at once for the first commit after a quiet stretch and
//! [`CHECK_NS`] after the last check within a run of commits: a new picture under a dim shows dimmed for a frame, not longer. None is
//! begun within [`INPUT_SETTLE_NS`] of a key. A cell whose numbers moved by more than [`AGREE`] changed at its last damage; one whose
//! numbers did not is as still as it was, whatever was committed. Damage left unjudged for [`STALE_NS`] counts as change, and an output
//! that cannot sample makes every damaged cell a changed one: a picture nobody looked at is never taken for still.
//!
//! A cell still for `boxDimAfterMin` targets a gain that falls with its brightness, to `boxDimTo` at the brightest; the targets are
//! dilated by one cell and blurred, so a bright shape is dimmed uniformly to past its edge and the feather lies on the dark around it. The
//! gains move toward their targets slowly down (`boxDimRampSec`) and fast up
//! ([`RESTORE_NS`]). The map the renderer multiplies the app's items with is the gains, 8-bit, sampled bilinearly ([`GainMap::at`] is
//! that sampling on the CPU).
//!
//! A key, a window moving, appearing or leaving, a hole changing, a transition, the saver, a capture and a sample that finds
//! [`BIG_CELLS`] or more cells changed restart every cell; a smaller change restarts its own cells. While a cell or its neighbour is
//! still and bright, a small change beside it (a clock) keeps the neighbourhood's gain through the dilation, so nothing pulses.

use serde_json::{Map, Value, json};

use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// Cell edge in output pixels.
pub const CELL: i32 = 30;
pub const COLS: usize = (OUTPUT_WIDTH / CELL as u32) as usize;
pub const ROWS: usize = (OUTPUT_HEIGHT / CELL as u32) as usize;
pub const CELLS: usize = COLS * ROWS;

const SEC: u64 = 1_000_000_000;
const MS: u64 = 1_000_000;
/// A sample is begun this long after the last one when anything was damaged since.
pub const SAMPLE_NS: u64 = 4 * SEC;
/// Within a run of commits on a dimmed cell or an engaged app, a check is begun this long after the last one (the first of the run at
/// once).
pub const CHECK_NS: u64 = 250 * MS;
/// No sample this soon after a key or the pointer: the user is moving around, and every clock restarted anyway.
pub const INPUT_SETTLE_NS: u64 = 2 * SEC;
/// The ramp down steps this often (a step is under two code values).
pub const STEP_NS: u64 = 2 * SEC;
/// The ramp up: a restored cell is at 1 this long after the restart.
pub const RESTORE_NS: u64 = 150 * MS;
/// A sample that finds this many cells changed restarts every cell: a change of the picture, not of a clock in it.
pub const BIG_CELLS: usize = 16;
/// A sample that finds this many cells changed (a [`crate::oledcare::CHANGE_FRACTION`] of the output) is a change of the app's picture
/// for the app features.
pub const APP_CELLS: usize = CELLS / crate::oledcare::CHANGE_FRACTION as usize;
/// The lowest `boxDimTo`.
pub const MIN_FLOOR: f32 = 0.75;
/// Two samples of a cell are the same picture while none of its numbers differs by more than this many code values.
pub const AGREE: u8 = 2;
/// Above `min_luma` the gain falls linearly over this much brightness to the floor.
const LUMA_SPAN: f32 = 0.3;
/// The ramp's time step is capped: a step after a long pause is one step, not a jump.
const MAX_DT_NS: u64 = 2 * STEP_NS;
/// The first samples whose cost is logged (a measurement on the set, not a watch).
const SAMPLE_LINES: u64 = 5;
/// Damage that no sample has judged for this long (sampling stalled: a buffer the driver stays busy with, samples lost) makes its cells
/// count as changed: a picture nobody looked at is never taken for still.
const STALE_NS: u64 = 3 * SAMPLE_NS;
const WORDS: usize = CELLS.div_ceil(64);

/// What a sample says of one cell: the mean luma of the cell, of its left part and of its lower part (8-bit, premultiplied over black).
/// The parts make a change that keeps the cell's mean (a word replaced by one as long) a change all the same.
pub type Sig = [u8; 3];

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

/// Whether two samples of a cell are different pictures.
fn differs(a: Sig, b: Sig) -> bool {
    a.iter().zip(b).any(|(x, y)| x.abs_diff(y) > AGREE)
}

/// A set of cells.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Bits([u64; WORDS]);

impl Bits {
    const NONE: Bits = Bits([0; WORDS]);

    fn set(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }

    fn get(&self, i: usize) -> bool {
        self.0[i / 64] & (1 << (i % 64)) != 0
    }

    fn any(&self) -> bool {
        self.0.iter().any(|w| *w != 0)
    }

    fn count(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }

    fn or(&mut self, o: &Bits) {
        for (w, v) in self.0.iter_mut().zip(o.0) {
            *w |= v;
        }
    }

    fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(k, w)| {
            let mut w = *w;
            std::iter::from_fn(move || {
                (w != 0).then(|| {
                    let b = w.trailing_zeros() as usize;
                    w &= w - 1;
                    k * 64 + b
                })
            })
        })
    }
}

#[derive(Clone, Copy)]
struct Cell {
    /// The last damage that touched it: its picture is the one the last sample saw unless this is later.
    damage_ns: u64,
    /// When its picture last changed, as the samples tell (a change is put at the damage before the sample that saw it); the grid's
    /// restarts are `BoxDim::epoch_ns`.
    since_ns: u64,
    sig: Sig,
    gain: f32,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { damage_ns: 0, since_ns: 0, sig: [0; 3], gain: 1.0 }
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

/// How long since `then` (CLOCK_MONOTONIC ns), forever when it never happened (0).
fn since(now_ns: u64, then: u64) -> u64 {
    if then == 0 { u64::MAX } else { now_ns.saturating_sub(then) }
}

/// The state of the box dim: the grid, its map, and what the plan and the samples tell it.
pub struct BoxDim {
    /// A screenshot or recording is armed (set by the state before each plan): the map stays at 1, the picture written is the app's own.
    pub capturing: bool,
    /// The app features' card is dimmed or moved (set by the scene before each plan): its damage is checked at [`CHECK_NS`].
    pub app_engaged: bool,
    /// Where the output writes a finished sample (`Output::sample_take`) for [`BoxDim::take_sample`].
    pub incoming: Box<[Sig; CELLS]>,
    cells: Box<[Cell; CELLS]>,
    target: Box<[f32; CELLS]>,
    scratch: Box<[f32; CELLS]>,
    /// Cells damaged since the sample in flight (or the last) began, and those damaged before it began: what it judges. With the time of
    /// the oldest damage in each (0: none).
    dirty: Bits,
    judging: Bits,
    dirty_since_ns: u64,
    judging_since_ns: u64,
    /// Every cell counts as changed at this time (a restart is one write).
    epoch_ns: u64,
    restart: Restart,
    /// The gains move toward 1 (a restart or a change while something was dimmed): a step every frame until they are there.
    restoring: bool,
    snap: bool,
    started: bool,
    /// The grid is kept: the box dim or an app feature is on.
    tracking: bool,
    shown: bool,
    shown_last: bool,
    blocked: bool,
    /// The output has made what sampling needs (`Output::boxdim_warm`).
    warm: bool,
    /// The output cannot sample: damage is change.
    blind: bool,
    /// A sample has been taken since the grid was last started: every cell has its numbers.
    sampled: bool,
    in_flight: bool,
    /// The sample in flight is a check ([`CHECK_NS`]), and what beginning it cost the loop (us).
    in_flight_check: bool,
    begin_us: u32,
    last_input_ns: u64,
    last_sample_ns: u64,
    /// The last damage, and whether damage came after a stretch of [`CHECK_NS`] without any since the last sample began: the first commit
    /// after a quiet screen is checked at once, the ones that follow it at [`CHECK_NS`].
    last_mark_ns: u64,
    fresh: bool,
    /// A sample found the app's picture changed ([`APP_CELLS`]), not yet taken by the app features.
    app_change: bool,
    last_step_ns: u64,
    map: GainMap,
    serial: u64,
    engaged: bool,
    changed: bool,
    pub samples: u64,
    pub checks: u64,
    pub steps: u64,
    /// The longest the loop spent beginning or reading a sample, and on a piece of the output's warm-up (us), for `ctl status`.
    pub sample_us_max: u32,
    pub warm_us_max: u32,
    journal: Vec<String>,
}

impl Default for BoxDim {
    fn default() -> Self {
        BoxDim {
            capturing: false,
            app_engaged: false,
            incoming: Box::new([[0; 3]; CELLS]),
            cells: Box::new([Cell::default(); CELLS]),
            target: Box::new([1.0; CELLS]),
            scratch: Box::new([1.0; CELLS]),
            dirty: Bits::NONE,
            judging: Bits::NONE,
            dirty_since_ns: 0,
            judging_since_ns: 0,
            epoch_ns: 0,
            restart: Restart::Appeared,
            restoring: false,
            snap: false,
            started: false,
            tracking: false,
            shown: false,
            shown_last: false,
            blocked: false,
            warm: false,
            blind: false,
            sampled: false,
            in_flight: false,
            in_flight_check: false,
            begin_us: 0,
            last_input_ns: 0,
            last_sample_ns: 0,
            last_mark_ns: 0,
            fresh: false,
            app_change: false,
            last_step_ns: 0,
            map: GainMap::default(),
            serial: 0,
            engaged: false,
            changed: false,
            samples: 0,
            checks: 0,
            steps: 0,
            sample_us_max: 0,
            warm_us_max: 0,
            journal: Vec::new(),
        }
    }
}

impl BoxDim {
    /// Every cell starts over at `now_ns`: still from now.
    pub fn restart(&mut self, now_ns: u64, why: Restart) {
        self.restart_at(now_ns, now_ns, why);
    }

    /// Every cell starts over at `at_ns` (a change a sample at `now_ns` saw happen then).
    fn restart_at(&mut self, now_ns: u64, at_ns: u64, why: Restart) {
        self.epoch_ns = at_ns.max(self.epoch_ns);
        self.restart = why;
        if self.engaged {
            self.snap |= why.snaps();
            self.rise(now_ns);
        }
    }

    /// Something dimmed is to come back: the gains are stepped every frame, timed from now (not from the last step seconds ago).
    fn rise(&mut self, now_ns: u64) {
        if !self.restoring {
            self.restoring = true;
            self.last_step_ns = now_ns;
        }
    }

    /// A key or the pointer.
    pub fn input(&mut self, now_ns: u64) {
        self.restart(now_ns, Restart::Key);
        self.last_input_ns = now_ns;
    }

    /// A plan begins: no app item is on it yet.
    pub fn begin(&mut self) {
        self.shown = false;
    }

    /// An app item of the plan, drawn at `rect` (output pixels) moved by `drift` (the app features' shift, taken away: the grid is the app's
    /// own picture), with its damage: `None` the whole rect, `Some` the rectangles in output pixels (empty: nothing new). The cells it
    /// touches may have changed; a sample says whether they did.
    pub fn seen(&mut self, now_ns: u64, rect: Rect, drift: (i32, i32), damage: Option<&[Rect]>) {
        self.shown = true;
        let rect = rect.translated(-drift.0, -drift.1);
        match damage {
            None => self.mark(now_ns, &rect),
            Some(d) => {
                for r in d {
                    self.mark(now_ns, &r.translated(-drift.0, -drift.1).intersect(&rect));
                }
            }
        }
    }

    fn mark(&mut self, now_ns: u64, r: &Rect) {
        let Some((c0, r0, c1, r1)) = cells_of(r) else { return };
        if self.dirty_since_ns == 0 {
            self.dirty_since_ns = now_ns;
        }
        self.fresh |= since(now_ns, self.last_mark_ns) >= CHECK_NS;
        self.last_mark_ns = now_ns;
        for row in r0..=r1 {
            for col in c0..=c1 {
                let i = row * COLS + col;
                self.cells[i].damage_ns = now_ns;
                self.dirty.set(i);
            }
        }
    }

    /// Damage no sample has judged yet on a dimmed cell, or on an engaged app: a change there shows dimmed until a sample sees it.
    fn urgent(&self) -> bool {
        self.dirty.any() && (self.app_engaged || self.dirty.ones().any(|i| self.map.cells[i] < 255))
    }

    /// Whether the output should begin a sample now (`drew`: this tick drew a frame). Only while the grid is kept, an app shows, nothing
    /// stands the care aside, the output is warm, none is in flight and no key came in the last [`INPUT_SETTLE_NS`]: [`CHECK_NS`] after the
    /// last sample while [`BoxDim::urgent`], else, when anything was damaged since or nothing has been sampled yet, [`SAMPLE_NS`] after it
    /// on a tick that drew nothing or twice that on any tick.
    pub fn wants_sample(&self, now_ns: u64, drew: bool) -> bool {
        if !self.tracking || !self.shown || self.blocked || !self.warm || self.in_flight || since(now_ns, self.last_input_ns) < INPUT_SETTLE_NS {
            return false;
        }
        let gap = since(now_ns, self.last_sample_ns);
        if self.urgent() {
            return gap >= CHECK_NS || self.fresh;
        }
        // A screen that commits every frame has no tick that draws nothing: it is sampled at twice the period then.
        (self.dirty.any() || !self.sampled) && gap >= SAMPLE_NS && (!drew || gap >= 2 * SAMPLE_NS)
    }

    /// Whether the output should make the next piece of what sampling needs now (`Output::boxdim_warm`): a tick that drew nothing, no key
    /// in the last [`INPUT_SETTLE_NS`], once the grid is kept and until all is made.
    pub fn wants_warm(&self, now_ns: u64, drew: bool) -> bool {
        self.tracking && !self.warm && !drew && since(now_ns, self.last_input_ns) >= INPUT_SETTLE_NS
    }

    /// A piece of the warm-up took `cost_us`; `done` once all is made.
    pub fn warmed(&mut self, cost_us: u32, done: bool) {
        self.warm_us_max = self.warm_us_max.max(cost_us);
        self.warm |= done;
        log::info!("boxdim: warm-up piece took {cost_us} us{}", if done { ", ready" } else { "" });
    }

    /// The output began a sample at `now_ns`, which cost the loop `cost_us`: it judges what was damaged until now.
    pub fn sample_started(&mut self, now_ns: u64, cost_us: u32) {
        let check = self.urgent();
        self.checks += u64::from(check);
        self.in_flight_check = check;
        self.fresh = false;
        self.judging = self.dirty;
        self.judging_since_ns = self.dirty_since_ns;
        self.dirty = Bits::NONE;
        self.dirty_since_ns = 0;
        self.in_flight = true;
        self.last_sample_ns = now_ns;
        self.begin_us = cost_us;
        self.sample_us_max = self.sample_us_max.max(cost_us);
    }

    /// The output could not begin one now (a buffer the driver is still busy with): tried again a sample period on.
    pub fn sample_skipped(&mut self, now_ns: u64) {
        self.last_sample_ns = now_ns;
    }

    /// The sample in flight never came back: what it was to judge is judged by the next one.
    pub fn sample_failed(&mut self, now_ns: u64) {
        self.in_flight = false;
        let judging = self.judging;
        self.dirty.or(&judging);
        self.dirty_since_ns = match (self.dirty_since_ns, self.judging_since_ns) {
            (0, j) => j,
            (d, 0) => d,
            (d, j) => d.min(j),
        };
        self.judging = Bits::NONE;
        self.judging_since_ns = 0;
        self.last_sample_ns = now_ns;
    }

    /// The output cannot sample at all: what was damaged counts as changed, as it did before the samples. `true` the first time.
    pub fn sample_unavailable(&mut self, now_ns: u64) -> bool {
        let first = !self.blind;
        self.blind = true;
        self.last_sample_ns = now_ns;
        let mut changed = 0;
        let mut latest = 0;
        for i in self.dirty.ones() {
            let c = &mut self.cells[i];
            c.since_ns = c.damage_ns;
            changed += 1;
            latest = latest.max(c.damage_ns);
        }
        self.dirty = Bits::NONE;
        self.dirty_since_ns = 0;
        self.verdict(now_ns, changed, latest);
        first
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// The sample begun last, finished at `now_ns` and read in `cost_us`, is in [`BoxDim::incoming`] (row 0 the top). A cell whose numbers
    /// moved changed at its last damage (or, with none since the last sample, a change the damage did not say, at the sample); the
    /// first sample since the grid started is the picture every later one is held against.
    pub fn take_sample(&mut self, now_ns: u64, cost_us: u32) {
        self.in_flight = false;
        self.blind = false;
        self.sample_us_max = self.sample_us_max.max(cost_us);
        let first = !self.sampled;
        let mut changed = 0;
        let mut latest = 0;
        for (i, (c, new)) in self.cells.iter_mut().zip(self.incoming.iter()).enumerate() {
            if first {
                c.since_ns = c.damage_ns;
            } else if differs(c.sig, *new) {
                let at = if self.judging.get(i) || self.dirty.get(i) { c.damage_ns } else { self.last_sample_ns };
                c.since_ns = at;
                changed += 1;
                latest = latest.max(at);
            }
            c.sig = *new;
        }
        self.sampled = true;
        self.judging = Bits::NONE;
        self.judging_since_ns = 0;
        self.samples += 1;
        if self.samples <= SAMPLE_LINES {
            log::info!(
                "boxdim: sample {}{}: begun in {} us, read in {cost_us} us, {changed} cells changed",
                self.samples,
                if self.in_flight_check { " (check)" } else { "" },
                self.begin_us
            );
        }
        self.verdict(now_ns, changed, latest);
    }

    /// What a judgement found: `changed` cells, the latest of them at `latest_ns`.
    fn verdict(&mut self, now_ns: u64, changed: usize, latest_ns: u64) {
        self.app_change |= changed >= APP_CELLS;
        if changed >= BIG_CELLS {
            self.restart_at(now_ns, latest_ns, Restart::Change);
        } else if changed > 0 && self.engaged {
            self.rise(now_ns);
        }
    }

    /// Whether a sample found the app's picture changed since the last call ([`APP_CELLS`] cells or more): what the app features take for
    /// a change of the card.
    pub fn take_app_change(&mut self) -> bool {
        std::mem::take(&mut self.app_change)
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

    /// One plan: `track` whether the grid is kept at all (the box dim or an app feature is on; `s.on` is the box dim's own map),
    /// `blocked` says the care stands aside (and why), `scene_changed` that a window appeared, moved, left or restacked or a hole changed.
    /// Moves the gains and returns the output rectangle to redraw when the map changed.
    pub fn step(&mut self, now_ns: u64, s: &Settings, track: bool, blocked: Option<Restart>, scene_changed: bool) -> Option<Rect> {
        self.changed = false;
        if !track {
            if self.engaged || self.started {
                self.restart(now_ns, Restart::Off);
            }
            self.started = false;
            self.tracking = false;
            self.sampled = false;
            self.dirty = Bits::NONE;
            self.dirty_since_ns = 0;
            self.blocked = true;
        } else {
            self.tracking = true;
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
            if !self.shown && self.shown_last {
                self.restart(now_ns, Restart::Hidden);
            }
            if !s.on && self.engaged {
                self.restart(now_ns, Restart::Off);
            }
            self.blocked = blocked.is_some() || !self.shown;
        }
        self.shown_last = self.shown;
        let map_on = track && s.on && !self.blocked;
        // Nothing to do: no cell is dimmed and none can be yet (every cell restarted less than the wait ago).
        if !self.engaged && !self.snap && (!map_on || now_ns.saturating_sub(self.epoch_ns) < s.after_ns) {
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
            if map_on {
                self.targets(now_ns, s);
            } else {
                self.target.fill(1.0);
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

    /// Every cell's target gain into `target`: its own by stillness and brightness, dilated by one cell, blurred by [1 2 1] on both axes. A
    /// cell is still while its picture has not changed for the wait, as the samples tell; damage no sample has judged for [`STALE_NS`]
    /// makes it changed.
    fn targets(&mut self, now_ns: u64, s: &Settings) {
        let epoch = self.epoch_ns;
        let min_luma = (s.min_luma * 255.0) as u8;
        let sampled = self.sampled;
        let oldest = [self.dirty_since_ns, self.judging_since_ns].into_iter().filter(|t| *t != 0).min();
        let stale = oldest.is_some_and(|t| now_ns.saturating_sub(t) > STALE_NS);
        let (dirty, judging) = (self.dirty, self.judging);
        for (i, (c, t)) in self.cells.iter().zip(self.scratch.iter_mut()).enumerate() {
            let still = now_ns.saturating_sub(c.since_ns.max(epoch)) >= s.after_ns && !(stale && (dirty.get(i) || judging.get(i)));
            *t = if sampled && still && c.sig[0] > min_luma { s.gain_for(c.sig[0]) } else { 1.0 };
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
                let at = |cc: usize| self.target[r * COLS + cc];
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

    /// The last sampled numbers of cell (`col`, `row`).
    pub fn sig(&self, col: usize, row: usize) -> Sig {
        self.cells[row * COLS + col].sig
    }

    /// The still time of the grid as a whole: since the last restart.
    pub fn still_ns(&self, now_ns: u64) -> u64 {
        now_ns.saturating_sub(self.epoch_ns)
    }

    /// For `ctl status`.
    pub fn report(&self, now_ns: u64, s: &Settings) -> Value {
        json!({
            "on": s.on,
            "tracking": self.tracking,
            "engaged": self.engaged,
            "cells": self.map.dimmed(),
            "lowest": (self.map.lowest() as f64 / 255.0 * 100.0).round() / 100.0,
            "stillSec": self.still_ns(now_ns) / SEC,
            "unjudged": self.dirty.count() + self.judging.count(),
            "samples": self.samples,
            "checks": self.checks,
            "steps": self.steps,
            "warm": self.warm,
            "blind": self.blind,
            "sampleUsMax": self.sample_us_max,
            "warmUsMax": self.warm_us_max,
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

    /// A white box of cells (`c0..=c1`, `r0..=r1`) on black, as a sample sees it.
    fn picture(c0: usize, r0: usize, c1: usize, r1: usize) -> Box<[Sig; CELLS]> {
        let mut p = Box::new([[0u8; 3]; CELLS]);
        for r in r0..=r1 {
            for c in c0..=c1 {
                p[r * COLS + c] = [255; 3];
            }
        }
        p
    }

    /// A screen and an output that samples it at once: plans at `t` (+ `dt`), with an app item over the whole output.
    struct Sim {
        b: BoxDim,
        s: Settings,
        track: bool,
        t: u64,
        pic: Box<[Sig; CELLS]>,
    }

    impl Sim {
        fn new(s: Settings, pic: Box<[Sig; CELLS]>) -> Sim {
            let mut b = BoxDim::default();
            b.warmed(0, true);
            Sim { b, s, track: s.on, t: SEC, pic }
        }

        /// One plan `dt` on, with `damage` (`None`: nothing committed; `Some(None)`: the whole surface; `Some(Some(rects))`), then the
        /// sample the loop would begin after the frame, read back by the next tick (here at once). Returns the map's redraw.
        fn frame(&mut self, dt: u64, damage: Option<Option<&[Rect]>>) -> Option<Rect> {
            self.t += dt;
            let t = self.t;
            self.b.begin();
            self.b.seen(t, full(), (0, 0), damage.unwrap_or(Some(&[])));
            let r = self.b.step(t, &self.s, self.track, None, false);
            if self.b.wants_sample(t, damage.is_some()) {
                self.b.sample_started(t, 0);
                *self.b.incoming = *self.pic;
                self.b.take_sample(t, 0);
            }
            r
        }

        /// Plans every 2 s for `for_ns`, nothing committed.
        fn still(&mut self, for_ns: u64) {
            let end = self.t + for_ns;
            while self.t < end {
                self.frame(2 * SEC, None);
            }
        }

        fn gain(&self, c: usize, r: usize) -> u8 {
            self.b.map.cells[r * COLS + c]
        }
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
        assert_eq!(APP_CELLS, 36, "a 64th of the output");
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
    fn off_does_nothing_and_a_still_bright_box_dims_after_the_wait_over_the_ramp_with_one_sample() {
        let mut sim = Sim::new(Settings::default(), picture(10, 10, 20, 20));
        sim.still(10 * MIN);
        assert!(!sim.b.engaged() && sim.b.samples == 0 && sim.b.steps == 0, "off: no sample, no step");
        let mut sim = Sim::new(on(), picture(10, 10, 20, 20));
        sim.frame(2 * SEC, Some(None));
        sim.still(2 * MIN - 12 * SEC);
        assert!(!sim.b.engaged(), "not before the wait");
        assert_eq!(sim.b.samples, 1, "nothing committed after the first sample: nothing to judge, nothing sampled");
        sim.still(20 * SEC);
        assert!(sim.b.engaged(), "engaged after the wait");
        let early = sim.gain(15, 15);
        assert!((200..255).contains(&early), "part of the way in: {early}");
        sim.still(30 * SEC);
        assert_eq!(sim.gain(15, 15), 204, "the floor, 0.8, after the ramp");
        assert_eq!(sim.gain(10, 10), 204, "the box's own edge cell is at the floor");
        assert_eq!(sim.gain(9, 11), 217, "the ring one cell out is the feather's first step, 0.85");
        assert!(sim.gain(9, 9) > 217 && sim.gain(9, 9) < 240, "its corner is rounded: {}", sim.gain(9, 9));
        assert_eq!(sim.gain(15, 8), 242, "two cells out, 0.95");
        assert_eq!(sim.gain(15, 6), 255, "the dark beyond is untouched");
        assert_eq!(sim.gain(40, 30), 255);
        assert!(sim.b.take_journal().iter().any(|l| l.starts_with("oledcare: boxDim on:")), "journalled");
    }

    #[test]
    fn an_app_that_commits_the_same_picture_every_few_seconds_is_still_and_its_dim_never_pumps() {
        // Round 16: LG's browser on a static page and Plex commit the whole surface again every 4 to 5 s, in bursts of up to 12 frames.
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        let burst = |sim: &mut Sim| {
            for _ in 0..12 {
                sim.frame(16 * MS, Some(None));
            }
        };
        let mut lowest = Vec::new();
        for _ in 0..60 {
            burst(&mut sim);
            sim.still(4 * SEC);
            lowest.push(sim.b.map.lowest());
        }
        assert!(sim.b.engaged(), "still for the wait, the commits notwithstanding: {lowest:?}");
        assert_eq!(sim.b.map.lowest(), 204, "and at the floor after the ramp");
        assert!(lowest.windows(2).all(|w| w[1] <= w[0]), "never brighter by itself, no pumping: {lowest:?}");
        assert!(sim.b.samples >= 50, "every burst is judged: {}", sim.b.samples);
        assert!(sim.b.checks > 0, "and judged at once while dimmed: {}", sim.b.checks);
        assert!(!sim.b.take_app_change(), "and never a change of the app's picture");
        assert_eq!(sim.b.restart, Restart::Appeared);
    }

    #[test]
    fn a_changed_picture_restarts_what_changed_and_a_big_change_everything_and_tells_the_app_features() {
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        sim.frame(2 * SEC, Some(None));
        sim.still(3 * MIN);
        assert_eq!(sim.b.map.lowest(), 204);
        // Three cells of a clock change: their own clocks start over; the dilation keeps them at their neighbours' gain.
        for c in 30..33 {
            sim.pic[16 * COLS + c] = [180, 170, 190];
        }
        let clock = [Rect::new(900, 480, 90, 30)];
        sim.frame(SEC, Some(Some(&clock)));
        assert!(sim.b.engaged() && sim.b.restart != Restart::Change, "a small change does not restore the screen");
        assert!(!sim.b.take_app_change());
        assert!(now_still(&sim, 31, 16) < SEC, "the clock's cell started over");
        assert!(now_still(&sim, 40, 20) > 2 * MIN, "the rest did not");
        let before = sim.b.map.cells;
        for _ in 0..10 {
            sim.frame(16 * MS, None);
        }
        assert!(before == sim.b.map.cells, "nothing around the clock pulses");
        assert_eq!(sim.gain(31, 16), 204, "and its gain is its neighbours'");
        // A popup: 40 cells change at once.
        for c in 20..30 {
            for r in 10..14 {
                sim.pic[r * COLS + c] = [40, 40, 40];
            }
        }
        sim.frame(4 * SEC, Some(Some(&[Rect::new(600, 300, 300, 120)])));
        assert_eq!(sim.b.restart, Restart::Change);
        assert!(sim.b.restoring, "every cell starts over and the dim comes off");
        assert!(sim.b.take_app_change(), "a change of the app's picture");
        let mut frames = 0;
        while sim.b.engaged() {
            sim.frame(16 * MS, None);
            frames += 1;
            assert!(frames < 15, "off within 250 ms");
        }
    }

    fn now_still(sim: &Sim, c: usize, r: usize) -> u64 {
        let cell = sim.b.cells[r * COLS + c];
        sim.t.saturating_sub(cell.since_ns.max(sim.b.epoch_ns))
    }

    #[test]
    fn damage_on_a_dimmed_cell_is_checked_within_a_quarter_second_even_on_frames_that_draw() {
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        sim.frame(2 * SEC, Some(None));
        sim.still(2 * MIN + 10 * SEC);
        assert!(sim.b.engaged());
        let (samples, checks) = (sim.b.samples, sim.b.checks);
        // A commit, and the frame drawing it: no sample is due by the period, a check is.
        sim.t += 3 * SEC;
        sim.b.begin();
        sim.b.seen(sim.t, full(), (0, 0), None);
        assert!(sim.b.wants_sample(sim.t, true), "a check rides on a frame that draws");
        sim.b.step(sim.t, &sim.s, true, None, false);
        sim.b.sample_started(sim.t, 0);
        assert!(!sim.b.wants_sample(sim.t + CHECK_NS, true), "one in flight at a time");
        *sim.b.incoming = *sim.pic;
        sim.b.take_sample(sim.t + 16 * MS, 0);
        assert_eq!((sim.b.samples, sim.b.checks), (samples + 1, checks + 1));
        assert!(sim.b.engaged() && !sim.b.restoring, "the same picture: nothing comes back");
        // An engaged app (the app features) is checked the same way, the box dim's map flat or not.
        sim.b.seen(sim.t + SEC, Rect::new(0, 0, 30, 30), (0, 0), None);
        sim.b.map.cells.fill(255);
        assert!(!sim.b.wants_sample(sim.t + SEC, true));
        sim.b.app_engaged = true;
        assert!(sim.b.wants_sample(sim.t + SEC, true));
        // Not engaged: damage waits for the sample period, on a tick that draws nothing (twice the period on any tick).
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        sim.frame(2 * SEC, Some(None));
        sim.frame(SEC, None);
        sim.b.seen(sim.t, full(), (0, 0), None);
        let last = sim.b.last_sample_ns;
        assert!(!sim.b.wants_sample(last + SEC, false), "not before the period");
        assert!(!sim.b.wants_sample(last + SAMPLE_NS, true), "not on a frame that draws");
        assert!(sim.b.wants_sample(last + SAMPLE_NS, false));
        assert!(sim.b.wants_sample(last + 2 * SAMPLE_NS, true), "a screen that draws every frame is sampled too");
    }

    #[test]
    fn a_screen_that_commits_every_frame_is_still_when_its_picture_is_and_not_when_it_moves() {
        // 60 commits a second, the whole surface damaged each time (an SDL app, a game's menu), no tick that draws nothing.
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        for _ in 0..(3 * 60 * 60) {
            sim.frame(16_666_667, Some(None));
        }
        assert!(sim.b.engaged(), "the same picture 60 times a second is a still picture");
        assert!(sim.b.samples >= 20, "{}", sim.b.samples);
        assert!(!sim.b.take_app_change());
        // The same, but it moves: every frame a different picture.
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        for f in 0..(3 * 60 * 60) {
            sim.pic[0] = [(f % 200) as u8; 3];
            sim.pic[40 * COLS % CELLS] = [(f % 200) as u8; 3];
            for c in 0..40 {
                sim.pic[10 * COLS + c] = [(f % 250) as u8; 3];
            }
            sim.frame(16_666_667, Some(None));
        }
        assert!(!sim.b.engaged(), "a moving picture is never dimmed");
        assert!(sim.b.take_app_change(), "and it is a change for the app features");
    }

    #[test]
    fn damage_left_unjudged_counts_as_change() {
        // The output never manages to begin a sample after the first (a buffer the driver stays busy with): the damaged cells are not still.
        let mut b = BoxDim::default();
        b.warmed(0, true);
        let s = on();
        let mut t = SEC;
        b.begin();
        b.seen(t, full(), (0, 0), None);
        b.step(t, &s, true, None, false);
        assert!(b.wants_sample(t + SAMPLE_NS, false));
        b.sample_started(t + SAMPLE_NS, 0);
        *b.incoming = *picture(0, 0, COLS - 1, ROWS - 1);
        b.take_sample(t + SAMPLE_NS, 0);
        t += SAMPLE_NS;
        for _ in 0..200 {
            t += 2 * SEC;
            b.begin();
            b.seen(t, Rect::new(0, 0, 960, 1080), (0, 0), None);
            b.step(t, &s, true, None, false);
            if b.wants_sample(t, false) {
                b.sample_skipped(t);
            }
        }
        assert!(b.engaged(), "the half nobody damaged dims");
        assert_eq!(b.map.cells[10 * COLS + 10], 255, "the half nothing judged does not");
        assert_eq!(b.map.cells[10 * COLS + 50], 204);
    }

    #[test]
    fn damage_that_comes_while_a_sample_is_in_flight_is_judged_by_the_next_and_a_lost_one_by_the_next_too() {
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        sim.frame(2 * SEC, Some(None));
        sim.still(10 * SEC);
        let t = sim.t + SAMPLE_NS;
        sim.b.seen(t - SEC, Rect::new(0, 0, 30, 30), (0, 0), None);
        sim.b.sample_started(t, 0);
        sim.b.seen(t + MS, Rect::new(60, 0, 30, 30), (0, 0), None);
        sim.pic[2] = [9, 9, 9];
        *sim.b.incoming = *sim.pic;
        sim.b.take_sample(t + 2 * MS, 0);
        assert!(sim.b.dirty.get(2) && !sim.b.dirty.get(0), "cell 2 was damaged after the sample began: still to be judged");
        // A sample lost in flight: its cells go back to be judged.
        sim.b.sample_started(t + 3 * MS, 0);
        assert!(!sim.b.dirty.get(2));
        sim.b.sample_failed(t + SEC);
        assert!(sim.b.dirty.get(2) && !sim.b.in_flight());
    }

    #[test]
    fn a_change_the_damage_did_not_say_still_counts() {
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        sim.frame(2 * SEC, Some(None));
        sim.still(MIN);
        sim.pic[5 * COLS + 5] = [10, 10, 10];
        let t = sim.t + SAMPLE_NS;
        sim.b.dirty.set(100);
        sim.b.sample_started(t, 0);
        *sim.b.incoming = *sim.pic;
        sim.b.take_sample(t, 0);
        assert_eq!(sim.b.cells[5 * COLS + 5].since_ns, t, "it changed with no damage said: at the sample");
        assert_eq!(sim.b.cells[6 * COLS + 5].since_ns, sim.b.cells[0].since_ns, "its neighbours did not change");
    }

    #[test]
    fn a_key_restores_within_the_restore_time_and_nothing_is_sampled_while_keys_flow() {
        let mut sim = Sim::new(on(), picture(0, 0, COLS - 1, ROWS - 1));
        sim.frame(2 * SEC, Some(None));
        sim.still(3 * MIN);
        assert_eq!(sim.b.map.lowest(), 204);
        sim.b.input(sim.t);
        let mut frames = 0;
        while sim.b.engaged() {
            let r = sim.frame(16_666_667, Some(None));
            frames += 1;
            assert!(r.is_some() || !sim.b.engaged(), "every frame of the ramp up redraws");
            assert!(frames < 20, "gone within 250 ms");
        }
        assert!(frames >= 4, "ramped, not snapped: {frames} frames");
        assert!(sim.b.take_journal().iter().any(|l| l == "oledcare: boxDim off (key)"));
        let samples = sim.b.samples;
        for _ in 0..50 {
            sim.b.input(sim.t);
            sim.frame(100 * MS, Some(None));
        }
        assert_eq!(sim.b.samples, samples, "keys flowing: no sample");
        sim.still(2 * MIN - 10 * SEC);
        assert!(!sim.b.engaged(), "still again for the full wait first");
        sim.still(20 * SEC);
        assert!(sim.b.engaged());
    }

    #[test]
    fn an_output_that_cannot_sample_takes_damage_for_change() {
        let mut b = BoxDim::default();
        b.warmed(0, true);
        let s = on();
        let mut t = SEC;
        for _ in 0..200 {
            t += 2 * SEC;
            b.begin();
            b.seen(t, full(), (0, 0), None);
            b.step(t, &s, true, None, false);
            if b.wants_sample(t, false) {
                b.sample_unavailable(t);
            }
        }
        assert!(b.blind && !b.engaged(), "nothing is dimmed that cannot be seen");
        assert!(b.take_app_change(), "and a full redraw is a change for the app features");
        assert_eq!(b.restart, Restart::Change);
    }

    #[test]
    fn a_scene_change_a_transition_the_saver_a_capture_no_app_and_off_restart_everything() {
        let s = on();
        let engaged = || {
            let mut sim = Sim::new(s, picture(0, 0, COLS - 1, ROWS - 1));
            sim.frame(2 * SEC, Some(None));
            sim.still(3 * MIN);
            assert!(sim.b.engaged());
            (sim.b, sim.t)
        };
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), (0, 0), Some(&[]));
        b.step(t + SEC, &s, true, None, true);
        assert!(b.restoring && b.restart == Restart::Change, "a window moved or appeared");
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), (0, 0), Some(&[]));
        let r = b.step(t + SEC, &s, true, Some(Restart::Transition), false);
        assert!(r.is_none() && b.restoring, "the frame of the restart itself moves nothing yet");
        let r = b.step(t + SEC + 16 * MS, &s, true, Some(Restart::Transition), false);
        assert!(r.is_some() && b.engaged() && b.restoring, "a transition ramps the map out, it does not snap");
        assert!(!b.wants_sample(t + 10 * SEC, false), "nothing is sampled through a transition");
        for why in [Restart::Saver, Restart::Capture] {
            let (mut b, t) = engaged();
            b.begin();
            b.seen(t + SEC, full(), (0, 0), Some(&[]));
            let r = b.step(t + SEC, &s, true, Some(why), false);
            assert!(r.is_some() && !b.engaged(), "{why:?} snaps the map to 1 in one step");
            assert!(!b.wants_sample(t + 10 * SEC, false), "and nothing is sampled while it is up");
        }
        let (mut b, t) = engaged();
        b.begin();
        b.step(t + SEC, &s, true, None, false);
        assert!(!b.engaged(), "no app on the screen: flat at once");
        let steps = b.steps;
        for i in 2..100 {
            b.begin();
            b.step(t + i * SEC, &s, true, None, false);
        }
        assert_eq!(b.steps, steps, "and nothing runs while none shows");
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), (0, 0), Some(&[]));
        let r = b.step(t + SEC, &Settings { on: false, ..s }, false, None, false);
        assert!(r.is_some() && !b.engaged() && b.map().is_none(), "switched off: one step back to 1");
        assert!(b.take_journal().iter().any(|l| l == "oledcare: boxDim off (off)"));
        let (mut b, t) = engaged();
        b.begin();
        b.seen(t + SEC, full(), (0, 0), Some(&[]));
        let r = b.step(t + SEC, &Settings { on: false, ..s }, true, None, false);
        assert!(r.is_some() && !b.engaged(), "the box dim off while the app features keep the grid: the map goes, the grid stays");
        assert!(b.tracking);
    }

    #[test]
    fn the_step_damages_the_changed_cells_with_a_margin_and_no_work_is_done_on_a_still_dark_screen() {
        let mut sim = Sim::new(on(), picture(10, 10, 12, 12));
        sim.frame(2 * SEC, Some(None));
        sim.still(2 * MIN);
        let mut r = None;
        for _ in 0..3 {
            r = r.or(sim.frame(2 * SEC, None));
        }
        let r = r.expect("a step changed the map");
        // The box is cells 10..=12, dilated to 9..=13, blurred to 8..=14, plus one cell of margin: 7..=15 -> 210..480.
        assert_eq!((r.x, r.y, r.right(), r.bottom()), (210, 210, 480, 480), "{r:?}");
        let mut sim = Sim::new(on(), Box::new([[0u8; 3]; CELLS]));
        sim.frame(2 * SEC, Some(None));
        sim.still(5 * MIN);
        let (steps, samples) = (sim.b.steps, sim.b.samples);
        for _ in 0..100 {
            assert!(sim.frame(SEC, None).is_none());
        }
        assert!(sim.b.steps - steps <= 50, "a target pass every 2 s at most on a still screen: {}", sim.b.steps - steps);
        assert_eq!(sim.b.samples, samples, "and no sample");
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

    #[test]
    fn the_cell_sets_count_and_list_their_members() {
        let mut b = Bits::NONE;
        for i in [0, 63, 64, 2303] {
            b.set(i);
        }
        assert_eq!(b.ones().collect::<Vec<_>>(), vec![0, 63, 64, 2303]);
        assert_eq!(b.count(), 4);
        assert!(b.get(64) && !b.get(65) && b.any() && !Bits::NONE.any());
        assert!(differs([10, 10, 10], [10, 13, 10]) && !differs([10, 10, 10], [12, 8, 10]));
    }
}
