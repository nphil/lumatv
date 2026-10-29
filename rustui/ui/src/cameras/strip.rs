//! The timeline strip (`docs/research/cameras-ui-spec.md`, "The strip"): the span and the rate, the cursor and how it scrubs, the
//! window it moves in, the recorded ranges and the gaps among them, the event ticks and the jump from one to the next, the axis under
//! the bar. Pure: the keys it is given come back as what the app is to do (commit a time, go live, pause, close), so every rule is a
//! host test and the surface only draws [`Strip`].
//!
//! Three rows of focus, top to bottom: the span and the rate; the bar (Left and Right scrub, OK commits the cursor when it is not where
//! the picture is or toggles pause in a playback); the controls (`Live`, -10 s, Pause or Play, +10 s, the previous and the next event).
//! All times are the bridge's clock in epoch milliseconds; the app hands in `now` already on it (the TV's clock plus the skew).

use super::data::{Recordings, Span, Tick};
use super::words::{Clock, Zone};

/// The spans of the window, in minutes; the default is the second.
pub const SPANS: [u32; 4] = [15, 60, 360, 1440];
pub const DEFAULT_SPAN: usize = 1;
/// The rates of a playback.
pub const RATES: [f32; 3] = [1.0, 2.0, 4.0];
/// A jump to an event starts this long before it.
pub const LEAD_MS: i64 = 3000;
/// A repeat is the same key within this long of the last press.
pub const REPEAT_MS: u64 = 350;
/// Where releases are not delivered the cursor commits this long after the last press.
pub const COMMIT_MS: u64 = 450;
/// The data of the window is read again this often while the strip is open.
pub const RELOAD_MS: u64 = 30_000;
/// The bubble's picture is asked for this long after the cursor settled.
pub const BUBBLE_MS: u64 = 250;
/// A jump to "-10 s" or "+10 s".
pub const HOP_MS: i64 = 10_000;
/// Within this of an event's time (or of its lead-in) the cursor is "on" it.
pub const HERE_MS: i64 = 1000;

/// What the focus can be on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Span,
    Rate,
    Bar,
    Live,
    Back,
    Play,
    Forward,
    PrevEvent,
    NextEvent,
}

impl Item {
    /// The row of focus the item is in: 0 the top row, 1 the bar, 2 the controls.
    pub fn row(self) -> usize {
        match self {
            Item::Span | Item::Rate => 0,
            Item::Bar => 1,
            _ => 2,
        }
    }

    const CONTROLS: [Item; 6] = [Item::Live, Item::Back, Item::Play, Item::Forward, Item::PrevEvent, Item::NextEvent];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
}

/// What the app is to do about a key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    Nothing,
    /// The focus or the strip's own state changed: draw it again.
    Redraw,
    /// Up from the top row: the strip closes.
    Close,
    /// Down from the controls: the events and clips sheet opens.
    OpenSheet,
    /// Play the recording from this moment at the strip's rate (`POST playback {camera, at, rate}`).
    Commit { at: i64 },
    /// `Live`: back to the live picture.
    GoLive,
    /// Pause or play a recording where it is (`POST live/pause`); in live, a recording from the live edge held at its first picture.
    TogglePause,
    /// The rate changed: a playback re-dials at the playhead.
    Redial { rate: f32 },
}

/// What the strip needs to know of the world for a key or a draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ctx {
    /// The bridge's clock now.
    pub now: i64,
    /// Where the picture is, in a playback or a clip; `None` in live (the right edge).
    pub picture_at: Option<i64>,
    /// A recording plays (or is held): the pause control means something.
    pub recording: bool,
    pub paused: bool,
}

#[derive(Clone, Debug)]
pub struct Strip {
    pub span: usize,
    pub rate: usize,
    pub focus: Item,
    /// The cursor, when the person has moved it off the picture.
    cursor: Option<i64>,
    /// The right edge of the window when it is not following now.
    window_end: Option<i64>,
    /// The first moment of footage the bridge knows.
    pub since: Option<i64>,
    pub recordings: Recordings,
    pub ticks: Vec<Tick>,
    /// Whether the data of the window was read (a failed read shows an empty bar and is read again on the next beat).
    pub loaded: bool,
    dirty: bool,
    loaded_at: Option<u64>,
    /// The scrub key in a run: its direction, how many repeats, when it was last pressed.
    run: Option<(Key, u32, u64)>,
    /// The cursor commits at this time on the model's clock when no release says so earlier.
    commit_due: Option<u64>,
    /// The moment the cursor last moved on the model's clock: the bubble's picture is asked for `BUBBLE_MS` after.
    settled_at: Option<u64>,
}

impl Strip {
    pub fn new(since: Option<i64>) -> Self {
        Self {
            span: DEFAULT_SPAN,
            rate: 0,
            focus: Item::Bar,
            cursor: None,
            window_end: None,
            since,
            recordings: Recordings::default(),
            ticks: Vec::new(),
            loaded: false,
            dirty: true,
            loaded_at: None,
            run: None,
            commit_due: None,
            settled_at: None,
        }
    }

    pub fn span_minutes(&self) -> u32 {
        SPANS[self.span]
    }

    pub fn rate_value(&self) -> f32 {
        RATES[self.rate]
    }

    /// The window `[start, end]` that shows now: it follows now until the cursor leaves it.
    pub fn window(&self, now: i64) -> Span {
        let end = self.window_end.unwrap_or(now).min(now);
        Span { from: end - i64::from(self.span_minutes()) * 60_000, to: end }
    }

    /// Whether the window follows now (live is the right edge).
    pub fn follows_now(&self) -> bool {
        self.window_end.is_none()
    }

    /// The cursor: where the person put it, else where the picture is, else live (now).
    pub fn cursor(&self, ctx: &Ctx) -> i64 {
        self.cursor.or(ctx.picture_at).unwrap_or(ctx.now).min(ctx.now)
    }

    /// The cursor is away from the picture: scrubbing, with a bubble.
    pub fn scrubbing(&self) -> bool {
        self.cursor.is_some()
    }

    /// The bubble's moment, once the cursor has rested for `BUBBLE_MS` (the app asks for its picture then).
    pub fn bubble_due(&self, ms: u64) -> Option<u64> {
        self.cursor.and(self.settled_at).map(|t| t + BUBBLE_MS).filter(|due| *due > ms)
    }

    pub fn bubble_ready(&self, ms: u64) -> bool {
        self.cursor.is_some() && self.settled_at.is_some_and(|t| ms >= t + BUBBLE_MS)
    }

    /// When the strip next wants a tick: a commit that waits for its release, the bubble's rest, the next read of the data.
    pub fn due_at(&self, ms: u64) -> Option<u64> {
        let reload = self.loaded_at.map(|t| t + RELOAD_MS).or(Some(ms));
        [self.commit_due, self.bubble_due(ms), reload].into_iter().flatten().min()
    }

    /// The base of a step: the person's cursor, else the picture, else now.
    fn base(&self, ctx: &Ctx) -> i64 {
        self.cursor(ctx)
    }

    /// The least the cursor may be: the first footage.
    fn floor(&self, ctx: &Ctx) -> i64 {
        self.since.unwrap_or(ctx.now - i64::from(*SPANS.last().expect("spans")) * 60_000)
    }

    /// One step of the scrub for a span and a run of repeats (seconds): a sixtieth of the span, never under 5 s, x3 after 6 repeats,
    /// x10 after 14, x30 after 28.
    pub fn step_seconds(span_minutes: u32, repeats: u32) -> i64 {
        let base = (i64::from(span_minutes) * 60 / 60).max(5);
        let mult = if repeats >= 28 {
            30
        } else if repeats >= 14 {
            10
        } else if repeats >= 6 {
            3
        } else {
            1
        };
        base * mult
    }

    fn move_cursor(&mut self, to: i64, ctx: &Ctx, ms: u64) {
        let to = to.clamp(self.floor(ctx), ctx.now);
        self.cursor = Some(to);
        self.settled_at = Some(ms);
        self.fit_window(to, ctx);
    }

    /// The cursor put at `at` by the pointer (a click on the bar): the commit that follows is the click's OK.
    pub fn point(&mut self, at: i64, ctx: &Ctx, ms: u64) {
        self.move_cursor(at, ctx, ms);
    }

    /// The window moves only when the cursor leaves it, by half a span, never into the future.
    fn fit_window(&mut self, cursor: i64, ctx: &Ctx) {
        let span = i64::from(self.span_minutes()) * 60_000;
        let mut w = self.window(ctx.now);
        let mut moved = false;
        while cursor < w.from {
            w = Span { from: w.from - span / 2, to: w.to - span / 2 };
            moved = true;
        }
        while cursor > w.to {
            let to = (w.to + span / 2).min(ctx.now);
            w = Span { from: to - span, to };
            moved = true;
        }
        if moved {
            self.window_end = if w.to >= ctx.now { None } else { Some(w.to) };
            self.invalidate();
        }
    }

    /// The window is not the one the data was read for: the bar is empty, and no gap is claimed, until the read of the new one is back.
    fn invalidate(&mut self) {
        self.dirty = true;
        self.loaded = false;
        self.recordings = Recordings::default();
        self.ticks.clear();
    }

    /// A key pressed (`repeat` for a held key's repeats). The scrub keys move the cursor and the commit follows the release.
    pub fn press(&mut self, key: Key, repeat: bool, ctx: &Ctx, ms: u64) -> Out {
        match (self.focus, key) {
            (Item::Span | Item::Rate, Key::Up) => Out::Close,
            (Item::Span, Key::Right) => {
                self.focus = Item::Rate;
                Out::Redraw
            }
            (Item::Rate, Key::Left) => {
                self.focus = Item::Span;
                Out::Redraw
            }
            (Item::Span | Item::Rate, Key::Down) => {
                self.focus = Item::Bar;
                Out::Redraw
            }
            (Item::Span, Key::Ok) if !repeat => {
                self.cycle_span(ctx);
                Out::Redraw
            }
            (Item::Rate, Key::Ok) if !repeat => {
                self.rate = (self.rate + 1) % RATES.len();
                if ctx.recording { Out::Redial { rate: self.rate_value() } } else { Out::Redraw }
            }
            (Item::Bar, Key::Up) => {
                self.focus = Item::Span;
                Out::Redraw
            }
            (Item::Bar, Key::Down) => {
                self.focus = Item::Live;
                Out::Redraw
            }
            (Item::Bar, Key::Left | Key::Right) => {
                let repeats = match self.run {
                    Some((k, n, at)) if k == key && ms.saturating_sub(at) <= REPEAT_MS => n + 1,
                    _ => 0,
                };
                self.run = Some((key, repeats, ms));
                let step = Self::step_seconds(self.span_minutes(), repeats) * 1000;
                let from = self.base(ctx);
                let to = if key == Key::Left { from - step } else { from + step };
                self.move_cursor(to, ctx, ms);
                self.commit_due = Some(ms + COMMIT_MS);
                Out::Redraw
            }
            (Item::Bar, Key::Ok) if !repeat => self.ok_on_bar(ctx, ms),
            (item, Key::Up) if item.row() == 2 => {
                self.focus = Item::Bar;
                Out::Redraw
            }
            (item, Key::Down) if item.row() == 2 => Out::OpenSheet,
            (item, Key::Left | Key::Right) if item.row() == 2 => {
                let i = Item::CONTROLS.iter().position(|c| *c == item).unwrap_or(0);
                let j = if key == Key::Left { i.saturating_sub(1) } else { (i + 1).min(Item::CONTROLS.len() - 1) };
                self.focus = Item::CONTROLS[j];
                Out::Redraw
            }
            (item, Key::Ok) if item.row() == 2 && !repeat => self.control(item, ctx, ms),
            _ => Out::Nothing,
        }
    }

    fn ok_on_bar(&mut self, ctx: &Ctx, _ms: u64) -> Out {
        if let Some(at) = self.cursor {
            return Out::Commit { at: at.min(ctx.now) };
        }
        if ctx.recording { Out::TogglePause } else { Out::Nothing }
    }

    fn control(&mut self, item: Item, ctx: &Ctx, ms: u64) -> Out {
        match item {
            Item::Live => Out::GoLive,
            Item::Play => Out::TogglePause,
            Item::Back => {
                let at = (self.base(ctx) - HOP_MS).max(self.floor(ctx));
                Out::Commit { at }
            }
            Item::Forward => {
                let at = self.base(ctx) + HOP_MS;
                if at >= ctx.now - 2000 || !ctx.recording && self.cursor.is_none() { Out::GoLive } else { Out::Commit { at } }
            }
            Item::PrevEvent => self.jump(false, ctx, ms),
            Item::NextEvent => self.jump(true, ctx, ms),
            _ => Out::Nothing,
        }
    }

    fn jump(&mut self, next: bool, ctx: &Ctx, _ms: u64) -> Out {
        match event_from(&self.ticks, self.base(ctx), next) {
            Some(tick) => Out::Commit { at: (tick - LEAD_MS).max(self.floor(ctx)) },
            None => Out::Nothing,
        }
    }

    fn cycle_span(&mut self, ctx: &Ctx) {
        let cursor = self.cursor;
        self.span = (self.span + 1) % SPANS.len();
        // the window keeps its right edge; the cursor, if the person put it somewhere, stays in view
        if let Some(c) = cursor {
            self.fit_window(c, ctx);
        }
        self.invalidate();
    }

    /// A key released: the scrub commits where it is.
    pub fn release(&mut self, key: Key, ctx: &Ctx) -> Out {
        if self.focus == Item::Bar && matches!(key, Key::Left | Key::Right) {
            self.run = None;
            self.commit_due = None;
            if let Some(at) = self.cursor {
                return self.commit_or_hold(at, ctx);
            }
        }
        Out::Nothing
    }

    /// The commit that waits for a release that never came: 450 ms after the last press.
    pub fn tick(&mut self, ctx: &Ctx, ms: u64) -> Out {
        if self.commit_due.is_some_and(|due| ms >= due) {
            self.commit_due = None;
            self.run = None;
            if let Some(at) = self.cursor {
                return self.commit_or_hold(at, ctx);
            }
        }
        Out::Nothing
    }

    /// A cursor that is not where the picture is commits; one on the picture's own moment (a scrub that came back) is no request.
    fn commit_or_hold(&mut self, at: i64, ctx: &Ctx) -> Out {
        let here = ctx.picture_at.unwrap_or(ctx.now);
        if (at - here).abs() < 1000 && ctx.picture_at.is_none() {
            self.cursor = None;
            self.window_end = None;
            return Out::Redraw;
        }
        Out::Commit { at }
    }

    /// The commit went out (or the picture is where the person asked): the cursor lets go to the picture.
    pub fn committed(&mut self) {
        self.cursor = None;
        self.commit_due = None;
        self.run = None;
        self.settled_at = None;
    }

    /// Back to live: the cursor and the window follow now again.
    pub fn live(&mut self) {
        self.cursor = None;
        self.window_end = None;
        self.commit_due = None;
        self.run = None;
        self.settled_at = None;
        self.invalidate();
    }

    /// A cursor in a confirmed gap asks for nothing. Nothing is confirmed before the window's ranges were read, or after a read failed.
    pub fn in_gap(&self, at: i64, ctx: &Ctx) -> bool {
        self.loaded && gaps(&self.recordings, self.since, self.window(ctx.now), ctx.now).iter().any(|g| at >= g.from && at < g.to)
    }

    /// Whether the data of the window is due to be read (on opening, a span change, a window that moved, and every 30 s), and the
    /// window to ask for.
    pub fn load_due(&self, ctx: &Ctx, ms: u64) -> Option<Span> {
        let again = self.loaded_at.is_none_or(|t| ms >= t + RELOAD_MS);
        (self.dirty || again).then(|| self.window(ctx.now))
    }

    pub fn mark_loading(&mut self, ms: u64) {
        self.dirty = false;
        self.loaded_at = Some(ms);
    }

    /// The reads came back.
    pub fn set_data(&mut self, recordings: Option<Recordings>, ticks: Option<Vec<Tick>>) {
        if let Some(r) = recordings {
            self.recordings = r;
            self.loaded = true;
        }
        if let Some(t) = ticks {
            self.ticks = t;
        }
    }

    /// The read failed: an empty bar (the data of an earlier window is not the window's).
    pub fn set_failed(&mut self) {
        self.recordings = Recordings::default();
        self.ticks.clear();
        self.loaded = false;
    }

    /// The window moved or changed span: what was read is not it.
    pub fn bar(&self, ctx: &Ctx) -> Bar {
        let w = self.window(ctx.now);
        Bar {
            window: w,
            recorded: clip_spans(&self.recordings.ranges, w),
            unknown: clip_spans(&self.recordings.unknown, w),
            gaps: if self.loaded { gaps(&self.recordings, self.since, w, ctx.now) } else { Vec::new() },
            events: self.ticks.iter().map(|t| t.at).filter(|a| *a >= w.from && *a <= w.to).collect(),
        }
    }
}

/// What the bar draws, all inside the window: the recorded ranges (accent bars), the `unknown` ranges (the same bar dimmed to 30 %),
/// only a confirmed gap hatched, and the event ticks as dots above.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bar {
    pub window: Span,
    pub recorded: Vec<Span>,
    pub unknown: Vec<Span>,
    pub gaps: Vec<Span>,
    pub events: Vec<i64>,
}

fn clip_spans(list: &[Span], w: Span) -> Vec<Span> {
    list.iter().filter(|s| s.to > w.from && s.from < w.to).map(|s| Span { from: s.from.max(w.from), to: s.to.min(w.to) }).collect()
}

/// The confirmed gaps in the window: inside what neither the recorded ranges nor the `unknown` ones name, between the first footage
/// and now. The part of a window after now is no gap.
pub fn gaps(rec: &Recordings, since: Option<i64>, w: Span, now: i64) -> Vec<Span> {
    let lo = since.map_or(w.from, |s| s.max(w.from));
    let hi = w.to.min(now);
    if hi <= lo {
        return Vec::new();
    }
    let mut named: Vec<Span> = rec.ranges.iter().chain(rec.unknown.iter()).copied().filter(|s| s.to > lo && s.from < hi).collect();
    named.sort_by_key(|s| s.from);
    let mut out = Vec::new();
    let mut at = lo;
    for s in named {
        if s.from > at {
            out.push(Span { from: at, to: s.from.min(hi) });
        }
        at = at.max(s.to);
        if at >= hi {
            break;
        }
    }
    if at < hi {
        out.push(Span { from: at, to: hi });
    }
    out
}

/// The nearest event time after (`next`) or before the cursor, skipping the one the cursor is on (within a second of its time or of
/// its lead-in): "from 2:20 PM with events at 1:58, 2:12 and 2:31 PM, next is 2:31, previous 2:12; standing on 2:12 next is 2:31 and
/// previous 1:58".
pub fn event_from(ticks: &[Tick], cursor: i64, next: bool) -> Option<i64> {
    let here = |t: &Tick| (cursor - t.at).abs() <= HERE_MS || (cursor - (t.at - LEAD_MS)).abs() <= HERE_MS;
    let mut times: Vec<i64> = ticks.iter().filter(|t| !here(t)).map(|t| t.at).collect();
    times.sort_unstable();
    if next { times.into_iter().find(|t| *t > cursor) } else { times.into_iter().rev().find(|t| *t < cursor) }
}

/// The step of the axis under the bar, in minutes: 3 for 15 min, 10 for 1 h, 60 for 6 h, 240 for 24 h.
pub fn axis_step_minutes(span_minutes: u32) -> i64 {
    match span_minutes {
        0..=15 => 3,
        16..=60 => 10,
        61..=360 => 60,
        _ => 240,
    }
}

/// The labels of the axis: the multiples of the step on the local clock inside the window, with their times.
pub fn axis(w: Span, span_minutes: u32, clock: &Clock) -> Vec<(i64, String)> {
    let step = axis_step_minutes(span_minutes) * 60_000;
    let zone: Zone = clock.zone;
    let local_start = zone.local_ms(w.from);
    let offset = local_start - w.from;
    let mut t = (local_start + step - 1).div_euclid(step) * step - offset;
    let mut out = Vec::new();
    while t <= w.to {
        out.push((t, clock.clock(t)));
        t += step;
    }
    out
}

/// Where a moment is across a bar `width` wide, in pixels from its left edge, for the window.
pub fn x_of(at: i64, w: Span, width: f32) -> f32 {
    let span = (w.to - w.from).max(1) as f64;
    (((at - w.from) as f64 / span) as f32 * width).clamp(0.0, width)
}

/// The moment at pixel `x` of a bar `width` wide.
pub fn moment_at(x: f32, w: Span, width: f32) -> i64 {
    w.from + (((w.to - w.from) as f64) * f64::from((x / width.max(1.0)).clamp(0.0, 1.0))) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 3_600_000;

    /// 2026-10-29 14:37:00 UTC: the strip's "now" of the spec's vectors (2:37 PM).
    const NOW: i64 = (20_755 * 86_400 + 14 * 3600 + 37 * 60) * 1000;

    fn clock() -> Clock {
        Clock { twelve_hour: true, zone: Zone::UTC }
    }

    fn live() -> Ctx {
        Ctx { now: NOW, picture_at: None, recording: false, paused: false }
    }

    fn at(ms: i64) -> Ctx {
        Ctx { now: NOW, picture_at: Some(ms), recording: true, paused: false }
    }

    #[test]
    fn the_scrub_step_follows_the_span_and_the_run_of_repeats() {
        // the specs vector: 0, 6, 14 and 28 repeats
        let table = [(15, [15, 45, 150, 450]), (60, [60, 180, 600, 1800]), (360, [360, 1080, 3600, 10800]), (1440, [1440, 4320, 14400, 43200])];
        for (span, want) in table {
            let got: Vec<i64> = [0, 6, 14, 28].iter().map(|r| Strip::step_seconds(span, *r)).collect();
            assert_eq!(got, want, "{span} min");
        }
        assert_eq!(Strip::step_seconds(15, 5), 15, "the run's tiers begin at their count");
        assert_eq!(Strip::step_seconds(1, 0), 5, "never under 5 s");
    }

    #[test]
    fn the_axis_falls_on_the_local_clock_by_the_specs_vectors() {
        let labels = |span: u32| -> Vec<String> {
            let w = Span { from: NOW - i64::from(span) * MIN, to: NOW };
            axis(w, span, &clock()).into_iter().map(|(_, l)| l).collect()
        };
        assert_eq!(labels(15), ["2:24 PM", "2:27 PM", "2:30 PM", "2:33 PM", "2:36 PM"]);
        assert_eq!(labels(60), ["1:40 PM", "1:50 PM", "2:00 PM", "2:10 PM", "2:20 PM", "2:30 PM"]);
        assert_eq!(labels(360), ["9:00 AM", "10:00 AM", "11:00 AM", "12:00 PM", "1:00 PM", "2:00 PM"]);
        assert_eq!(labels(1440), ["4:00 PM", "8:00 PM", "12:00 AM", "4:00 AM", "8:00 AM", "12:00 PM"]);
    }

    #[test]
    fn the_axis_counts_from_the_local_midnight_not_the_utc_one() {
        // a zone 30 minutes off: the 10-minute marks are the same marks; a 4 hour mark is local midnight's multiple
        let c = Clock { twelve_hour: true, zone: Zone::fixed(5 * 3600 + 1800) };
        let w = Span { from: NOW - 24 * HOUR, to: NOW };
        let marks: Vec<String> = axis(w, 1440, &c).into_iter().map(|(_, l)| l).collect();
        assert!(marks.iter().all(|m| m.contains(":00 ")), "{marks:?}");
        assert_eq!(marks.len(), 6);
    }

    #[test]
    fn a_confirmed_gap_is_what_neither_a_range_nor_unknown_names_between_the_first_footage_and_now() {
        let rec = Recordings {
            from: 0,
            to: 0,
            ranges: vec![Span { from: NOW - 50 * MIN, to: NOW - 10 * MIN }, Span { from: NOW - 8 * MIN, to: NOW }],
            unknown: vec![Span { from: NOW - 10 * MIN, to: NOW - 8 * MIN }],
        };
        let w = Span { from: NOW - 60 * MIN, to: NOW };
        assert_eq!(gaps(&rec, None, w, NOW), vec![Span { from: NOW - 60 * MIN, to: NOW - 50 * MIN }], "the only gap is the first 10 minutes");
        assert!(gaps(&rec, Some(NOW - 50 * MIN), w, NOW).is_empty(), "with `since` at now-50 min there is none");
        let future = Span { from: NOW - 30 * MIN, to: NOW + 30 * MIN };
        assert!(gaps(&rec, None, future, NOW).is_empty(), "the part of a window after now is no gap");
        // a gap between two ranges
        let two = Recordings { ranges: vec![Span { from: 0, to: 100 }, Span { from: 200, to: 300 }], ..Recordings::default() };
        assert_eq!(gaps(&two, Some(0), Span { from: 0, to: 300 }, 300), vec![Span { from: 100, to: 200 }]);
    }

    fn ticks(times: &[i64]) -> Vec<Tick> {
        times.iter().map(|t| Tick { at: *t, kind: "person".into(), label: String::new(), clip: None }).collect()
    }

    #[test]
    fn the_jump_to_an_event_follows_the_specs_vector() {
        let (a, b, c) = (NOW - 39 * MIN, NOW - 25 * MIN, NOW - 6 * MIN); // 1:58, 2:12, 2:31 PM
        let t = ticks(&[a, b, c]);
        let from_220 = NOW - 17 * MIN;
        assert_eq!(event_from(&t, from_220, true), Some(c), "next is 2:31");
        assert_eq!(event_from(&t, from_220, false), Some(b), "previous 2:12");
        assert_eq!(event_from(&t, b, true), Some(c), "standing on 2:12: next is 2:31");
        assert_eq!(event_from(&t, b + 900, false), Some(a), "and previous 1:58, within a second");
        assert_eq!(event_from(&t, b - LEAD_MS, false), Some(a), "standing on its lead-in is standing on it");
        assert_eq!(event_from(&t, b - LEAD_MS, true), Some(c));
        assert_eq!(event_from(&t, c, true), None, "no later event");
        assert_eq!(event_from(&t, a, false), None, "no earlier event");
    }

    #[test]
    fn the_window_follows_now_and_moves_by_half_a_span_only_when_the_cursor_leaves_it() {
        let mut s = Strip::new(None);
        let c = live();
        assert_eq!(s.window(NOW), Span { from: NOW - HOUR, to: NOW });
        // 5 presses of Left at 1 h: 60 s each, the cursor stays inside the window
        for i in 0..5 {
            s.press(Key::Left, i > 0, &c, 1000 + i as u64 * 100);
        }
        assert_eq!(s.cursor(&c), NOW - 5 * MIN);
        assert!(s.follows_now(), "the window did not move");
        // a cursor that leaves the window to the left moves it back by half a span
        let mut s = Strip::new(None);
        s.press(Key::Left, false, &c, 0);
        s.release(Key::Left, &c);
        s.committed();
        let far = NOW - 61 * MIN;
        s.move_cursor(far, &c, 0);
        assert_eq!(s.window(NOW), Span { from: NOW - 90 * MIN, to: NOW - 30 * MIN }, "half a span back");
        assert!(!s.follows_now());
        // and back to the right it never goes into the future
        s.move_cursor(NOW - 20 * MIN, &c, 10);
        assert_eq!(s.window(NOW), Span { from: NOW - 60 * MIN, to: NOW });
        assert!(s.follows_now(), "at now the window follows it again");
    }

    #[test]
    fn the_cursor_never_goes_past_now_or_before_the_first_footage() {
        let mut s = Strip::new(Some(NOW - 2 * MIN));
        let c = live();
        s.press(Key::Right, false, &c, 0);
        assert_eq!(s.cursor(&c), NOW, "never past now");
        for i in 0..5 {
            s.press(Key::Left, true, &c, 100 + i * 100);
        }
        assert_eq!(s.cursor(&c), NOW - 2 * MIN, "never before `since`");
    }

    #[test]
    fn a_run_of_repeats_speeds_the_scrub_and_a_pause_between_presses_starts_it_again() {
        let mut s = Strip::new(None);
        let c = live();
        let mut at_ms = 0u64;
        let mut positions = Vec::new();
        for _ in 0..8 {
            s.press(Key::Left, true, &c, at_ms);
            positions.push(NOW - s.cursor(&c));
            at_ms += 40;
        }
        // 60 s per press for repeats 0..5 and 180 s from the sixth repeat on
        assert_eq!(positions, [60_000, 120_000, 180_000, 240_000, 300_000, 360_000, 540_000, 720_000]);
        // a pause of more than 350 ms: the run is a new one
        at_ms += 1000;
        let before = s.cursor(&c);
        s.press(Key::Left, true, &c, at_ms);
        assert_eq!(before - s.cursor(&c), 60_000);
    }

    #[test]
    fn the_cursor_commits_on_the_release_or_450_ms_after_the_last_press_and_a_scrub_back_to_live_asks_for_nothing() {
        let mut s = Strip::new(None);
        let c = live();
        s.press(Key::Left, false, &c, 1000);
        assert_eq!(s.tick(&c, 1400), Out::Nothing, "not yet");
        assert_eq!(s.tick(&c, 1450), Out::Commit { at: NOW - MIN }, "450 ms after the last press where releases are not delivered");
        s.committed();
        let mut s = Strip::new(None);
        s.press(Key::Left, false, &c, 1000);
        assert_eq!(s.release(Key::Left, &c), Out::Commit { at: NOW - MIN }, "on the key's release");
        // left, then right again: the cursor is back on live and no request is made
        let mut s = Strip::new(None);
        s.press(Key::Left, false, &c, 1000);
        s.press(Key::Right, false, &c, 1100);
        assert_eq!(s.release(Key::Right, &c), Out::Redraw);
        assert!(!s.scrubbing(), "the cursor let go to the picture");
    }

    #[test]
    fn ok_on_the_bar_commits_a_cursor_that_is_not_the_picture_or_toggles_pause_in_a_playback() {
        let mut s = Strip::new(None);
        let c = at(NOW - 20 * MIN);
        assert_eq!(s.press(Key::Ok, false, &c, 0), Out::TogglePause, "the cursor is where the picture is: pause");
        s.press(Key::Left, false, &c, 100);
        s.release(Key::Left, &c);
        assert_eq!(s.press(Key::Ok, false, &c, 500), Out::Commit { at: NOW - 21 * MIN }, "the cursor moved off the picture: commit it");
        let mut s = Strip::new(None);
        assert_eq!(s.press(Key::Ok, false, &live(), 0), Out::Nothing, "live with nothing to commit");
    }

    #[test]
    fn the_rows_of_focus_are_left_right_up_down_and_the_top_row_closes_and_the_controls_open_the_sheet() {
        let mut s = Strip::new(None);
        let c = live();
        assert_eq!(s.focus, Item::Bar);
        assert_eq!(s.press(Key::Up, false, &c, 0), Out::Redraw);
        assert_eq!(s.focus, Item::Span);
        assert_eq!(s.press(Key::Right, false, &c, 0), Out::Redraw);
        assert_eq!(s.focus, Item::Rate);
        assert_eq!(s.press(Key::Right, false, &c, 0), Out::Nothing, "the end of the row holds");
        assert_eq!(s.press(Key::Up, false, &c, 0), Out::Close, "Up from the top row closes the strip");
        s.press(Key::Down, false, &c, 0);
        assert_eq!(s.focus, Item::Bar);
        s.press(Key::Down, false, &c, 0);
        assert_eq!(s.focus, Item::Live);
        for want in [Item::Back, Item::Play, Item::Forward, Item::PrevEvent, Item::NextEvent, Item::NextEvent] {
            s.press(Key::Right, false, &c, 0);
            assert_eq!(s.focus, want);
        }
        assert_eq!(s.press(Key::Down, false, &c, 0), Out::OpenSheet);
        assert_eq!(s.press(Key::Up, false, &c, 0), Out::Redraw);
        assert_eq!(s.focus, Item::Bar);
    }

    #[test]
    fn ok_cycles_the_span_and_the_rate_and_a_rate_change_in_a_playback_redials() {
        let mut s = Strip::new(None);
        let c = live();
        s.press(Key::Up, false, &c, 0);
        let spans: Vec<u32> = (0..5)
            .map(|_| {
                s.press(Key::Ok, false, &c, 0);
                s.span_minutes()
            })
            .collect();
        assert_eq!(spans, [360, 1440, 15, 60, 360], "the default is 1 h");
        s.press(Key::Right, false, &c, 0);
        assert_eq!(s.press(Key::Ok, false, &c, 0), Out::Redraw);
        assert_eq!(s.rate_value(), 2.0);
        assert_eq!(s.press(Key::Ok, false, &at(NOW - MIN), 0), Out::Redial { rate: 4.0 }, "in a playback the rate re-dials at the playhead");
        s.press(Key::Ok, false, &c, 0);
        assert_eq!(s.rate_value(), 1.0);
    }

    #[test]
    fn the_controls_commit_hop_jump_and_go_live() {
        let mut s = Strip::new(None);
        let c = at(NOW - 30 * MIN);
        s.focus = Item::Live;
        assert_eq!(s.press(Key::Ok, false, &c, 0), Out::GoLive);
        s.focus = Item::Back;
        assert_eq!(s.press(Key::Ok, false, &c, 0), Out::Commit { at: NOW - 30 * MIN - 10_000 }, "-10 s from the playhead");
        s.focus = Item::Forward;
        assert_eq!(s.press(Key::Ok, false, &c, 0), Out::Commit { at: NOW - 30 * MIN + 10_000 });
        assert_eq!(s.press(Key::Ok, false, &at(NOW - 5000), 0), Out::GoLive, "+10 s reaches now: live");
        s.focus = Item::Play;
        assert_eq!(s.press(Key::Ok, false, &c, 0), Out::TogglePause);
        s.ticks = ticks(&[NOW - 39 * MIN, NOW - 25 * MIN, NOW - 6 * MIN]);
        s.focus = Item::PrevEvent;
        assert_eq!(s.press(Key::Ok, false, &at(NOW - 17 * MIN), 0), Out::Commit { at: NOW - 25 * MIN - LEAD_MS }, "an event is entered 3 s before");
        s.focus = Item::NextEvent;
        assert_eq!(s.press(Key::Ok, false, &at(NOW - 17 * MIN), 0), Out::Commit { at: NOW - 6 * MIN - LEAD_MS });
        assert_eq!(s.press(Key::Ok, false, &at(NOW - 5000), 0), Out::Nothing, "no event later than that");
    }

    #[test]
    fn the_data_of_the_window_is_read_on_opening_a_span_change_a_moved_window_and_every_30_seconds() {
        let mut s = Strip::new(None);
        let c = live();
        assert_eq!(s.load_due(&c, 0), Some(Span { from: NOW - HOUR, to: NOW }), "on opening");
        s.mark_loading(0);
        assert_eq!(s.load_due(&c, 1000), None);
        assert_eq!(s.load_due(&c, 30_000), Some(s.window(NOW)), "every 30 s");
        s.mark_loading(30_000);
        s.focus = Item::Span;
        s.press(Key::Ok, false, &c, 31_000);
        assert!(s.load_due(&c, 31_000).is_some(), "a span change");
        s.mark_loading(31_000);
        s.focus = Item::Bar;
        s.move_cursor(NOW - 7 * HOUR, &c, 32_000);
        assert!(s.load_due(&c, 32_000).is_some(), "the window moved");
    }

    #[test]
    fn a_failed_read_shows_an_empty_bar_and_the_bar_holds_only_what_lies_in_the_window() {
        let mut s = Strip::new(Some(NOW - 50 * MIN));
        let c = live();
        s.set_data(
            Some(Recordings { from: 0, to: 0, ranges: vec![Span { from: NOW - 90 * MIN, to: NOW - 20 * MIN }, Span { from: NOW - 5 * MIN, to: NOW }], unknown: vec![] }),
            Some(ticks(&[NOW - 70 * MIN, NOW - 30 * MIN])),
        );
        let bar = s.bar(&c);
        assert_eq!(bar.recorded, vec![Span { from: NOW - 60 * MIN, to: NOW - 20 * MIN }, Span { from: NOW - 5 * MIN, to: NOW }], "clipped to the window");
        assert_eq!(bar.events, vec![NOW - 30 * MIN], "an event outside the window has no tick");
        assert_eq!(bar.gaps, vec![Span { from: NOW - 20 * MIN, to: NOW - 5 * MIN }]);
        s.set_failed();
        let bar = s.bar(&c);
        assert!(bar.recorded.is_empty() && bar.events.is_empty());
    }

    #[test]
    fn a_cursor_in_a_confirmed_gap_is_known_to_be() {
        let mut s = Strip::new(Some(NOW - 50 * MIN));
        let c = live();
        s.set_data(Some(Recordings { ranges: vec![Span { from: NOW - 50 * MIN, to: NOW - 30 * MIN }, Span { from: NOW - 10 * MIN, to: NOW }], ..Recordings::default() }), None);
        assert!(s.in_gap(NOW - 20 * MIN, &c));
        assert!(!s.in_gap(NOW - 40 * MIN, &c));
        assert!(!s.in_gap(NOW - 5 * MIN, &c));
    }

    #[test]
    fn the_bubble_is_asked_for_250_ms_after_the_cursor_settled_and_only_while_it_is_away() {
        let mut s = Strip::new(None);
        let c = live();
        assert!(!s.bubble_ready(0));
        s.press(Key::Left, false, &c, 1000);
        assert_eq!(s.bubble_due(1000), Some(1250));
        assert!(!s.bubble_ready(1200));
        assert!(s.bubble_ready(1250));
        s.press(Key::Left, true, &c, 1100);
        assert!(!s.bubble_ready(1250), "a moving cursor has not settled");
        s.committed();
        assert!(!s.bubble_ready(5000), "the cursor let go to the picture");
    }

    #[test]
    fn positions_map_to_pixels_and_back_across_the_bar() {
        let w = Span { from: 0, to: 3_600_000 };
        assert_eq!(x_of(1_800_000, w, 1672.0), 836.0);
        assert_eq!(x_of(-5, w, 1672.0), 0.0);
        assert_eq!(x_of(9_000_000, w, 1672.0), 1672.0);
        assert_eq!(moment_at(836.0, w, 1672.0), 1_800_000);
    }
}
