//! A finger's position once per displayed frame, handed to the shell at the moment the compositor can still use it.
//!
//! **Once per refresh.** The digitiser reports every ~10 ms (measured on the Show: 10.00 ms, no batching) and the panel refreshes
//! every ~16.8 ms, so passing each report on makes a page that follows the finger move one report on some frames and two on
//! others: at a steady finger, steps of 15 px and 30 px where every step should be 25 px (measured on the Show, 09-30). Here moves
//! are held back and handed on once per refresh, at where the finger is at that moment: between the two reports around it or, as a
//! rule, past the newest along the finger's speed (the way phones keep a dragged page even and close under the finger). When the
//! finger stops the digitiser stops reporting, so a page handed a prediction is then handed the finger's last real position, and
//! never rests ahead of it.
//!
//! **At the right moment of the refresh.** The compositor draws one frame at a time and takes into a frame every commit that has
//! reached it when it starts the frame. It starts at once when a commit is waiting and it is free, and it is free again only when the
//! frame before has been flipped; it aims the frame at the first refresh that its recent frames say it can reach, and a commit that
//! arrives too late for that refresh is drawn for the one after. So a delivery made too late before a refresh is shown a whole refresh
//! later, and, worse, then stays a refresh late: the next one arrives while that frame is still waiting for its flip, is taken when
//! it has flipped, and is again a refresh behind (trace of the Show 10-06: the age of the finger on the screen was 7.5 ms at one
//! moment of a drag and 24.3 ms at the next, for a whole gesture, with the same code, and always 24 ms on the pages whose frames cost
//! 7 ms or more, the eight milliseconds of lead being just short of what they need). The lead is therefore what the frames say:
//! their render time, the compositor's own lead for the flip, the loop's time to act and a margin; it moves a millisecond a frame
//! at most, so that the steps a page makes stay even; and it is held to what the screen shows: a delivery whose frame was flipped a
//! refresh later than it was aimed at (the pipeline holds a frame in flight) makes the deliveries move later, a millisecond a
//! frame, through the refresh, until they come after the frame in flight and it has drained. While the compositor draws frames of its
//! own (an animation) the frame in flight is never drained, and the deliveries move later only as far as the least lead
//! ([`Resampler::animating`]): through the refresh they would leave one without a delivery.
//!
//! **A little ahead.** What is handed on is where the finger will be a few milliseconds past the moment it is handed on (about half
//! of what the pipeline takes), from the speed of its two newest reports: fitted to more of them it trails a finger that speeds up
//! (measured on 23 swipes made on the Show: 6.4 px rms at 8 ms against 3.5 px for the two newest, 9.4 against 5.7 at 12 ms).
//!
//! **Lifting.** A finger that has travelled lifts at the next delivery, with the last position it would have been handed on (so that
//! the fling it starts begins on the sequence of frames the finger made, with the step the finger was making, and not at an
//! arbitrary moment of the refresh), as does one that is still. `Down`, `Cancel` and the lift of a finger that never went anywhere
//! (a tap) pass on at once.

use super::{TouchEvent, TouchPhase};
use std::collections::VecDeque;

/// The compositor aims a frame at the first refresh at least its typical render time and this much away (`pacer::FLIP_LEAD_NS`:
/// what `FBIOPAN_DISPLAY` needs to land on a refresh).
const FLIP_LEAD_NS: u64 = 500_000;
/// The loop acting on a move and committing it, until the compositor has the commit (trace of the Show: 0.3 to 0.5 ms).
const HANDLING_NS: u64 = 700_000;
/// What a delivery keeps in hand for a wake-up that is a little late and a frame that costs a little more than the ones before it.
const MARGIN_NS: u64 = 1_500_000;
/// The render time assumed until frames have said (the cost of a settings page being dragged on the Show is 4 to 8 ms). Too long a
/// lead only shows the finger a little later than it need be, and is shortened a frame at a time; too short a one is shown a whole
/// refresh later.
const FIRST_RENDER_NS: u64 = 9_000_000;
/// A finger that starts after a pause starts with this much more lead than the frames ask for, and gives it up a frame at a time:
/// the frames the compositor drew last may not have been like the ones it will draw now.
const START_EXTRA_NS: u64 = 3_000_000;
/// Deliveries are never less than this long before a refresh.
const MIN_LEAD_NS: u64 = 3_000_000;
/// Nor less than this short of a whole refresh: a delivery has to come after the frame before it has been shown.
const LEAD_GUARD_NS: u64 = 1_000_000;
/// The lead moves at most this much per delivery: while it does the deliveries are 4 ms further apart than the refreshes, and the
/// page shows the finger as of each, so it moves 24 % faster than the finger for as long (a catching up, not a jump; a finger's
/// first frames can be behind a frame the compositor was drawing for the touch, and four steps take it out of it). Less than half
/// of what the frames allow for between the finger and the refresh, so that the move cannot step over the moments that work.
const SLEW_NS: u64 = 4_000_000;
/// What a position is handed on ahead of the moment it is handed on, at most (about half of what the pipeline takes).
const AHEAD_NS: u64 = 6_000_000;
/// Frames whose render times are remembered, and deliveries whose frames are awaited.
const RENDERS: usize = 8;
const SENT: usize = 8;
/// A finger that has travelled this far from where it landed is a drag, not a tap.
const TRAVEL_PX: f32 = 10.0;
/// After this long without a delivery a finger is new: the lead goes straight to what the frames ask for.
const IDLE_NS: u64 = 100_000_000;
/// Never predict further past the newest report than this: a report and a bit (one can go missing).
const MAX_PREDICTION_NS: u64 = 20_000_000;
/// The finger's speed is that of its two newest reports when they are no further apart than this (a report that went missing is
/// not a speed).
const SPEED_SPAN_NS: u64 = 25_000_000;
/// Reports spanning less than this give no usable speed to predict with.
const MIN_SPAN_NS: u64 = 2_000_000;
/// Below this speed (px per ms) a pixel of the digitiser's rounding is most of the speed and nothing is predicted; it is all of
/// the prediction from twice this on.
const MIN_SPEED: f32 = 0.05;
const KEPT: usize = 5;

struct Track {
    id: u32,
    /// The newest reports, oldest first.
    reports: VecDeque<TouchEvent>,
    /// Where the finger landed (or was first seen), and whether it has gone [`TRAVEL_PX`] from it.
    origin: (f32, f32),
    travelled: bool,
    /// A report has come in since this contact's position was last handed on.
    fresh: bool,
    /// What was last handed on lies past the newest report: the finger has to be caught up with if it stops.
    ahead: bool,
    /// The lift of a finger that travelled, waiting for the next delivery.
    up: Option<TouchEvent>,
}

/// A delivery whose frame has not been reported yet.
#[derive(Clone, Copy)]
struct Sent {
    at_ns: u64,
    lead_ns: u64,
    tag: Option<u64>,
}

/// How long before a refresh deliveries are made, and the evidence it is kept to (see the module documentation).
struct Pace {
    period_ns: u64,
    lead_ns: u64,
    renders: VecDeque<u64>,
    sent: VecDeque<Sent>,
    /// The lead has moved since the last delivery: it moves once per delivery at most, so that two deliveries are never more than a
    /// step further from a refresh apart than the refreshes are (two frames can be reported between two deliveries).
    stepped: bool,
    /// The compositor is drawing frames of its own, one at every refresh (a settle of a motion track runs; see
    /// [`Resampler::animating`]).
    animating: bool,
}

impl Pace {
    fn new(period_ns: u64) -> Self {
        let mut pace = Self {
            period_ns,
            lead_ns: 0,
            renders: VecDeque::new(),
            sent: VecDeque::new(),
            stepped: false,
            animating: false,
        };
        pace.lead_ns = pace.target();
        pace
    }

    /// What the lead should be: the slowest of the last frames (the compositor starts a frame as late as its slowest recent one
    /// allows), the flip's lead, the loop's time and the margin, between the least worth having and a refresh less a guard.
    fn target(&self) -> u64 {
        let render = self
            .renders
            .iter()
            .copied()
            .max()
            .unwrap_or(FIRST_RENDER_NS);
        (render + FLIP_LEAD_NS + HANDLING_NS + MARGIN_NS)
            .clamp(MIN_LEAD_NS, self.period_ns - LEAD_GUARD_NS)
    }

    fn rendered(&mut self, render_ns: u64) {
        if self.renders.len() == RENDERS {
            self.renders.pop_front();
        }
        self.renders.push_back(render_ns);
    }

    fn delivered(&mut self, at_ns: u64) {
        self.stepped = false;
        if self.sent.len() == SENT {
            self.sent.pop_front();
        }
        self.sent.push_back(Sent {
            at_ns,
            lead_ns: self.lead_ns,
            tag: None,
        });
    }

    fn committed(&mut self, tag: u64) {
        if let Some(s) = self.sent.iter_mut().rev().find(|s| s.tag.is_none()) {
            s.tag = Some(tag);
        }
    }

    /// The frame with commits up to `tag` was flipped at `presented_ns`. Whether the newest delivery it shows was a refresh later
    /// than it was aimed at.
    fn shown(&mut self, tag: u64, presented_ns: u64) -> Option<bool> {
        let mut newest = None;
        self.sent.retain(|s| match s.tag {
            Some(t) if t <= tag => {
                newest = Some(*s);
                false
            }
            Some(_) => true,
            // Handed on without a commit (nothing on the screen changed): nothing to learn from it.
            None => false,
        });
        let s = newest?;
        Some(presented_ns.saturating_sub(s.at_ns) > s.lead_ns + self.period_ns / 2)
    }

    /// One step of the lead: towards what the frames ask for, or, when the last delivery shown was a refresh late, later in the
    /// refresh (the lead shorter). A pipeline that holds a frame in flight drains when nothing asks for the next one, so the lead
    /// goes on through nothing and round to a refresh less a millisecond, where the deliveries come after the frame in flight; one
    /// that the compositor's own animation keeps full does not (see [`Resampler::animating`]): the lead stops at the least there is.
    fn adapt(&mut self, late: bool) {
        if std::mem::replace(&mut self.stepped, true) {
            return;
        }
        let target = self.target();
        self.lead_ns = if late {
            if self.animating {
                self.lead_ns.saturating_sub(SLEW_NS).max(MIN_LEAD_NS)
            } else if self.lead_ns >= SLEW_NS {
                self.lead_ns - SLEW_NS
            } else {
                self.lead_ns + self.period_ns - SLEW_NS
            }
        } else if self.lead_ns > target {
            (self.lead_ns - SLEW_NS).max(target)
        } else {
            (self.lead_ns + SLEW_NS).min(target)
        };
    }

    /// How far past the moment of a delivery a position is handed on.
    fn ahead_ns(&self) -> u64 {
        AHEAD_NS.min(self.lead_ns)
    }
}

pub struct Resampler {
    period_ns: u64,
    /// A refresh the display made (its presentation time); refreshes follow it every `period_ns`.
    vsync_ns: u64,
    /// When moves were last handed on: the next delivery is a later refresh.
    delivered_ns: u64,
    /// The delivery moment fixed for what is held now, kept so that waking a little after it still finds it due.
    due_ns: Option<u64>,
    tracks: Vec<Track>,
    pace: Pace,
}

impl Resampler {
    pub fn new(refresh_hz: f32) -> Self {
        let period_ns = (1e9 / f64::from(refresh_hz.max(1.0))).round() as u64;
        Self {
            period_ns,
            vsync_ns: 0,
            delivered_ns: 0,
            due_ns: None,
            tracks: Vec::new(),
            pace: Pace::new(period_ns),
        }
    }

    /// The display refreshed at `presented_ns`.
    pub fn vsync(&mut self, presented_ns: u64) {
        self.vsync_ns = presented_ns;
    }

    /// A frame was on the screen at `presented_ns` after `render_ns` of drawing, and shows the commits up to `tag`: the refresh
    /// the deliveries follow, how long a frame takes, and whether the newest delivery it shows came a refresh later than it was
    /// aimed at.
    pub fn frame(&mut self, presented_ns: u64, render_ns: u64, tag: u64) {
        self.vsync(presented_ns);
        self.pace.rendered(render_ns);
        if let Some(late) = self.pace.shown(tag, presented_ns) {
            self.pace.adapt(late);
        }
    }

    /// The loop committed (with `tag`) what it did in this round: the newest delivery went out in it.
    pub fn committed(&mut self, tag: u64) {
        self.pace.committed(tag);
    }

    /// Whether the compositor is drawing frames of its own now, one at every refresh, as it does while a motion track settles (the
    /// loop says it each round, from the tracks' estimates: `Scene::motion_settling_at`). Such a frame starts the moment the frame
    /// before has flipped (the pacer's latch for frames that cost 9 ms is the flip: the slowest of the last eight, the flip's lead
    /// and 5.5 ms of slack fill the refresh), commits or not, so the pipeline is never drained: a commit comes while a frame is in
    /// flight in any part of the refresh, and is taken by the one that starts at its flip. The delivery that is best off is the one
    /// just before a flip (the lead's least: a refresh and that much from the finger to the screen, 19 ms) and every other is the
    /// same refresh later and a part of one more. A lead that went on later through the refresh to look for a drained pipeline
    /// would take the moment of its deliveries past a flip: the frame that starts there finds nothing new, and the screen shows
    /// the finger's step before it again (the day line, dragged to a focus: one refresh in six, deterministic, trace of the Show
    /// 10-08). So while it is on a late delivery moves the lead later no further than that least.
    pub fn animating(&mut self, on: bool) {
        self.pace.animating = on;
    }

    /// How long before a refresh deliveries are made now.
    pub fn lead_ns(&self) -> u64 {
        self.pace.lead_ns
    }

    /// A report from the digitiser. `Down` and `Cancel` come back to be acted on now (a `Cancel` after the finger's last real
    /// position, if that was still held); a `Move` is kept for [`Resampler::take`], and so is the `Up` of a finger that travelled,
    /// which comes with the last position it would have been handed on; the `Up` of one that did not comes at once. A finger that
    /// lands while another's lift waits for its delivery has that lift handed on first (the last position, the lift), then itself.
    pub fn feed(&mut self, e: TouchEvent) -> [Option<TouchEvent>; 4] {
        match e.phase {
            TouchPhase::Down => {
                let mut out = [None; 4];
                let mut n = 0;
                if let Some(id) = self.tracks.iter().find(|t| t.up.is_some()).map(|t| t.id) {
                    for lifted in self.finish(id, e.at_ns).into_iter().flatten() {
                        out[n] = Some(lifted);
                        n += 1;
                    }
                }
                self.tracks.retain(|t| t.id != e.id);
                self.tracks.push(Track {
                    id: e.id,
                    reports: VecDeque::from([e]),
                    origin: (e.x, e.y),
                    travelled: false,
                    fresh: false,
                    ahead: false,
                    up: None,
                });
                out[n] = Some(e);
                out
            }
            TouchPhase::Move => {
                // A move without its down (the down went to a wake) is followed from here.
                let at = match self.tracks.iter().position(|t| t.id == e.id) {
                    Some(at) => at,
                    None => {
                        self.tracks.push(Track {
                            id: e.id,
                            reports: VecDeque::new(),
                            origin: (e.x, e.y),
                            travelled: false,
                            fresh: false,
                            ahead: false,
                            up: None,
                        });
                        self.tracks.len() - 1
                    }
                };
                let t = &mut self.tracks[at];
                if t.reports.len() == KEPT {
                    t.reports.pop_front();
                }
                t.reports.push_back(e);
                t.fresh = true;
                t.travelled |= (e.x - t.origin.0).hypot(e.y - t.origin.1) >= TRAVEL_PX;
                if self.due_ns.is_none() {
                    if e.at_ns.saturating_sub(self.delivered_ns) > IDLE_NS {
                        // A finger that starts moving after a pause: what the frames ask for, and a little more at first.
                        self.pace.lead_ns = (self.pace.target() + START_EXTRA_NS)
                            .min(self.period_ns - LEAD_GUARD_NS);
                    }
                    self.due_ns = Some(self.next_delivery(e.at_ns));
                }
                [None; 4]
            }
            TouchPhase::Up => {
                let waiting = self
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == e.id && t.travelled && (t.fresh || t.ahead))
                    .map(|t| t.up = Some(e))
                    .is_some();
                if waiting {
                    if self.due_ns.is_none() {
                        self.due_ns = Some(self.next_delivery(e.at_ns));
                    }
                    return [None; 4];
                }
                self.leave(e)
            }
            TouchPhase::Cancel => self.leave(e),
        }
    }

    /// The contact `id`, whose lift is waiting, ends now: the position it would have been handed on at `now_ns`, and the lift.
    fn finish(&mut self, id: u32, now_ns: u64) -> [Option<TouchEvent>; 2] {
        let at = now_ns + self.pace.ahead_ns();
        let mut out = [None; 2];
        if let Some(t) = self.tracks.iter_mut().find(|t| t.id == id) {
            if let (Some(up), false) = (t.up.take(), t.reports.is_empty()) {
                let e = position_at(&t.reports, at);
                out = [
                    Some(e),
                    Some(TouchEvent {
                        phase: TouchPhase::Up,
                        x: e.x,
                        y: e.y,
                        at_ns: e.at_ns,
                        ..up
                    }),
                ];
            }
        }
        self.tracks.retain(|t| t.id != id);
        if !self
            .tracks
            .iter()
            .any(|t| t.fresh || t.ahead || t.up.is_some())
        {
            self.due_ns = None;
        }
        out
    }

    /// The contact ends now: its last real position if that was still held, then the end itself.
    fn leave(&mut self, e: TouchEvent) -> [Option<TouchEvent>; 4] {
        let held = self
            .tracks
            .iter()
            .find(|t| t.id == e.id && t.fresh)
            .and_then(|t| t.reports.back())
            .map(|&last| TouchEvent {
                phase: TouchPhase::Move,
                ..last
            });
        self.tracks.retain(|t| t.id != e.id);
        if !self
            .tracks
            .iter()
            .any(|t| t.fresh || t.ahead || t.up.is_some())
        {
            self.due_ns = None;
        }
        [held, Some(e), None, None]
    }

    /// Until when a commit that is not the finger's should wait for the next delivery, if one should: while a delivery is due (a move
    /// is held), and for a refresh and a half after one was made while a finger is down. A commit of its own (the shell making a few
    /// cells ahead of the window, a label drawn again) starts a frame that the compositor holds until its refresh, and the commit the
    /// next delivery makes, which arrives while that frame is in flight, is shown a refresh late, and so is every one after it (trace
    /// of the Show 10-06: a drag began with its fourth move a frame late, and the whole gesture with it, after a lone commit 4 ms
    /// behind a delivery; another began with one 1.5 ms before the first). Waiting for the delivery puts the work in its commit. A
    /// finger that stopped has nothing due and has made no delivery in that time, and nothing waits for one.
    pub fn hold_until(&self) -> Option<u64> {
        let after_one = (!self.tracks.is_empty() && self.delivered_ns != 0)
            .then(|| self.delivered_ns + self.period_ns * 3 / 2);
        match (self.due_ns, after_one) {
            (Some(due), Some(after)) => Some(due.max(after)),
            (due, after) => due.or(after),
        }
    }

    /// Whether a commit made at `now_ns` that is not a delivery's should wait (see [`Resampler::hold_until`]).
    pub fn expecting(&self, now_ns: u64) -> bool {
        self.hold_until().is_some_and(|until| now_ns < until)
    }

    /// When the next held move is due to be handed on, if one is: a contact that has moved, one left ahead of a finger that may
    /// have stopped, or a lift that waits. Due stays due until [`Resampler::take`], however late the caller wakes.
    pub fn due(&self) -> Option<u64> {
        self.due_ns
    }

    /// The first delivery moment (a refresh less the lead) at or after `after_ns`, and a good part of a refresh after the last one:
    /// the moment moves a few microseconds with the refresh that was last reported, and a delivery has to be a new one, not the same
    /// moment seen again.
    fn next_delivery(&self, after_ns: u64) -> u64 {
        let p = self.period_ns;
        let after = after_ns.max(self.delivered_ns + p * 3 / 5);
        let phase = (self.vsync_ns % p + p - self.pace.lead_ns % p) % p;
        after + (phase + p - after % p) % p
    }

    /// The moves due at `now_ns`, one per contact: where a moving finger will be a little past now, or, for one that has sent nothing
    /// since a prediction was handed on, its last real position; and, after the move of a finger that lifted, the lift.
    pub fn take(&mut self, now_ns: u64) -> Vec<TouchEvent> {
        self.delivered_ns = now_ns;
        let at = now_ns + self.pace.ahead_ns();
        let mut out = Vec::new();
        for t in &mut self.tracks {
            let Some(&newest) = t.reports.back() else {
                continue;
            };
            let lifted = t.up.take();
            let moved = if t.fresh || lifted.is_some() {
                let e = position_at(&t.reports, at);
                t.ahead = e.at_ns > newest.at_ns;
                Some(e)
            } else if t.ahead {
                t.ahead = false;
                Some(TouchEvent {
                    phase: TouchPhase::Move,
                    ..newest
                })
            } else {
                None
            };
            t.fresh = false;
            if let Some(e) = moved {
                out.push(e);
                if let Some(up) = lifted {
                    out.push(TouchEvent {
                        phase: TouchPhase::Up,
                        x: e.x,
                        y: e.y,
                        at_ns: e.at_ns,
                        ..up
                    });
                }
            }
        }
        // A lift has been handed on: its contact is over.
        self.tracks.retain(|t| {
            !out.iter()
                .any(|e| e.id == t.id && e.phase == TouchPhase::Up)
        });
        if !out.is_empty() {
            self.pace.delivered(now_ns);
        }
        self.due_ns = self
            .tracks
            .iter()
            .any(|t| t.ahead)
            .then(|| self.next_delivery(now_ns));
        out
    }
}

/// Where a finger that made `reports` (oldest first) is at `at`: between the two reports around it, or past the newest along the
/// speed of its two newest reports, at most `MAX_PREDICTION_NS` on.
fn position_at(reports: &VecDeque<TouchEvent>, at: u64) -> TouchEvent {
    let newest = *reports.back().expect("a contact has a report");
    let moved = |x: f32, y: f32, at_ns: u64| TouchEvent {
        id: newest.id,
        phase: TouchPhase::Move,
        x,
        y,
        at_ns,
    };
    if at <= newest.at_ns {
        let pair = reports
            .iter()
            .zip(reports.iter().skip(1))
            .find(|(_, b)| b.at_ns >= at);
        return match pair {
            Some((a, b)) if b.at_ns > a.at_ns && at >= a.at_ns => {
                let f = (at - a.at_ns) as f32 / (b.at_ns - a.at_ns) as f32;
                moved(a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f, at)
            }
            _ => moved(newest.x, newest.y, newest.at_ns),
        };
    }
    let ahead = (at - newest.at_ns).min(MAX_PREDICTION_NS);
    let to = newest.at_ns + ahead;
    let Some(prev) = reports.iter().rev().nth(1) else {
        return moved(newest.x, newest.y, to);
    };
    let span = newest.at_ns.saturating_sub(prev.at_ns);
    if !(MIN_SPAN_NS..=SPEED_SPAN_NS).contains(&span) {
        return moved(newest.x, newest.y, to);
    }
    let span_ms = span as f32 / 1e6;
    let (vx, vy) = ((newest.x - prev.x) / span_ms, (newest.y - prev.y) / span_ms);
    // A finger this slow is the digitiser's rounding: not predicted; a quick one all the way, in between by degrees.
    let trust = ((vx.hypot(vy) - MIN_SPEED) / MIN_SPEED).clamp(0.0, 1.0);
    let dt = ahead as f32 / 1e6 * trust;
    moved(newest.x + vx * dt, newest.y + vy * dt, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;
    const US: u64 = 1_000;
    /// The Show's refresh as the resampler rounds it.
    const PERIOD: u64 = 16_767_270;

    fn ev(phase: TouchPhase, x: f32, y: f32, at_ns: u64) -> TouchEvent {
        TouchEvent {
            id: 7,
            phase,
            x,
            y,
            at_ns,
        }
    }

    // ---- the compositor, as far as the app's timing depends on it -----------------------------------------------------------
    //
    // As crates/compositor/src/engine.rs and pacer.rs do it: one frame in flight (the render thread is free again 70 µs after the
    // flip of the frame before); a free render thread starts a frame the moment a commit is waiting and takes every commit that has
    // arrived; the frame is aimed at the first refresh at least the median of its last eight render times and the flip's lead away
    // (and after the frame before), and lands on the one after if it is not ready for it; the app hears of a frame 100 µs after its
    // flip. A scene that animates (a motion track settles) draws a frame of its own the moment the render thread is free, with the
    // commits that are there by then and none if none is: the pacer latches such a frame as late as the slowest of the last frames, the
    // flip's lead and 5.5 ms of slack allow, which for frames that cost 9 ms or more is the flip of the frame before.

    struct Frame {
        flip: u64,
        render: u64,
        tags: Vec<u64>,
    }

    struct Model {
        f0: u64,
        free_at: u64,
        last: Option<u64>,
        pending: Vec<(u64, u64)>,
        renders: VecDeque<u64>,
        cost: Box<dyn FnMut(usize) -> u64>,
        frames: Vec<Frame>,
        told: usize,
        /// From when until when the scene animates.
        animation: Option<(u64, u64)>,
    }

    impl Model {
        fn new(f0: u64, render_ns: u64, cost: Box<dyn FnMut(usize) -> u64>) -> Self {
            Self {
                f0,
                free_at: 0,
                last: None,
                pending: Vec::new(),
                renders: VecDeque::from(vec![render_ns; 8]),
                cost,
                frames: Vec::new(),
                told: 0,
                animation: None,
            }
        }

        fn flip(&self, k: u64) -> u64 {
            self.f0 + k * PERIOD
        }

        /// Starts every frame that starts before `limit`.
        fn advance(&mut self, limit: u64) {
            loop {
                let first = self.pending.iter().map(|p| p.0).min();
                let drawn_by_itself = self
                    .animation
                    .map(|(from, until)| (from.max(self.free_at), until))
                    .filter(|&(at, until)| at < until)
                    .map(|(at, _)| at);
                let start = match (drawn_by_itself, first) {
                    (Some(at), _) => at,
                    (None, Some(first)) => first.max(self.free_at),
                    (None, None) => return,
                };
                if start >= limit {
                    return;
                }
                let tags: Vec<u64> = self
                    .pending
                    .iter()
                    .filter(|p| p.0 <= start)
                    .map(|p| p.1)
                    .collect();
                self.pending.retain(|p| p.0 > start);
                let mut sorted: Vec<u64> = self.renders.iter().copied().collect();
                sorted.sort_unstable();
                let reach = start + sorted[sorted.len() / 2] + FLIP_LEAD_NS;
                let mut k = (reach.saturating_sub(self.f0)).div_ceil(PERIOD);
                k = k.max(self.last.map_or(0, |l| l + 1));
                let render = (self.cost)(self.frames.len());
                while self.flip(k) < start + render + FLIP_LEAD_NS {
                    k += 1;
                }
                self.frames.push(Frame {
                    flip: self.flip(k),
                    render,
                    tags,
                });
                self.renders.pop_front();
                self.renders.push_back(render);
                self.last = Some(k);
                self.free_at = self.flip(k) + 70 * US;
            }
        }

        fn commit(&mut self, arrival: u64, tag: u64) {
            self.advance(arrival);
            self.pending.push((arrival, tag));
        }

        /// The frame events the app has by `now`: (presented, render, tag of the newest commit it shows).
        fn events(&mut self, now: u64) -> Vec<(u64, u64, u64)> {
            self.advance(now);
            let mut out = Vec::new();
            while let Some(f) = self.frames.get(self.told) {
                if f.flip + 100 * US > now {
                    break;
                }
                out.push((f.flip, f.render, f.tags.iter().copied().max().unwrap_or(0)));
                self.told += 1;
            }
            out
        }

        fn next_event(&self) -> Option<u64> {
            self.frames.get(self.told).map(|f| f.flip + 100 * US)
        }

        fn flip_of(&self, tag: u64) -> Option<u64> {
            self.frames
                .iter()
                .find(|f| f.tags.contains(&tag))
                .map(|f| f.flip)
        }
    }

    /// A finger dragged `deliveries` frames' worth at `px_per_ms`, played through the resampler and the compositor model: for each
    /// delivery, when it was made and how long after that the frame that showed it was flipped; and the position it handed on; and how
    /// many frames between the first delivery's and the last one's showed no new commit (the gate's held refreshes: the finger moved
    /// in all of them).
    struct Drag {
        made: Vec<u64>,
        lag: Vec<u64>,
        stamps: Vec<u64>,
        held: usize,
    }

    struct Setup {
        render_ns: u64,
        /// A frame is in flight when the finger starts.
        busy: bool,
        /// Frames (by index) that cost this much more.
        spikes: Vec<(usize, u64)>,
        /// The refresh reports wobble by up to this much.
        wobble_ns: u64,
        /// The first delivery starts an animation of this long (the shell's focus: a settle of a motion track, whose request goes out
        /// as the delivery is handled, 0.1 ms ahead of its commit), and the loop says `animating` for as long as it runs.
        animation_ns: Option<u64>,
    }

    impl Setup {
        fn new(render_ns: u64) -> Self {
            Self {
                render_ns,
                busy: false,
                spikes: Vec::new(),
                wobble_ns: 0,
                animation_ns: None,
            }
        }
    }

    fn drag(setup: &Setup, deliveries: usize, px_per_ms: f32) -> Drag {
        let render_ns = setup.render_ns;
        let spikes = setup.spikes.clone();
        let f0 = 10_000 * MS;
        let mut model = Model::new(
            f0,
            render_ns,
            Box::new(move |i| render_ns + spikes.iter().find(|s| s.0 == i).map_or(0, |s| s.1)),
        );
        if setup.busy {
            // An animation's frame the commits have to wait for.
            model.frames.push(Frame {
                flip: model.flip(6),
                render: 6 * MS,
                tags: vec![0],
            });
            model.told = 0;
            model.free_at = model.flip(6) + 70 * US;
            model.last = Some(6);
        }
        let mut r = Resampler::new(59.64);
        // The app has heard of a refresh before the finger lands (it knows the phase of the panel from the frames before).
        r.vsync(model.f0 + 2 * PERIOD);
        let mut seed = 12_345u64;
        let mut wobble = |max: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if max == 0 {
                0
            } else {
                (seed >> 33) % (2 * max + 1)
            }
        };
        let start = f0 + 3 * PERIOD + 4_300 * US;
        let y_at = |t: u64| 400.0 - (t - start) as f32 / MS as f32 * px_per_ms;
        r.feed(ev(TouchPhase::Down, 600.0, 400.0, start));
        let mut next_report = 1u64;
        let mut tag = 0u64;
        let mut out = Drag {
            made: Vec::new(),
            lag: Vec::new(),
            stamps: Vec::new(),
            held: 0,
        };
        let mut tags = Vec::new();
        let mut now = start;
        while out.made.len() < deliveries {
            let report_at = start + next_report * 10 * MS;
            let arrival = report_at + 60 * US;
            let event_at = model.next_event().unwrap_or(u64::MAX);
            let due = r.due().map_or(u64::MAX, |d| d + 30 * US);
            let t = arrival.min(event_at).min(due).max(now);
            now = t;
            r.animating(
                model
                    .animation
                    .is_some_and(|(from, until)| t >= from && t < until),
            );
            for (presented, render, tag) in model.events(t) {
                let wobbled = (presented + wobble(setup.wobble_ns)).saturating_sub(setup.wobble_ns);
                r.frame(wobbled, render, tag);
            }
            if t == arrival {
                r.feed(ev(TouchPhase::Move, 600.0, y_at(report_at), report_at));
                next_report += 1;
            }
            if r.due().is_some_and(|d| t >= d) {
                let moves = r.take(t);
                if !moves.is_empty() {
                    tag += 1;
                    if let (1, Some(duration)) = (tag, setup.animation_ns) {
                        let from = t + 100 * US;
                        model.animation = Some((from, from + duration));
                    }
                    // The loop acts on it and commits; the compositor has the commit 0.7 ms after the delivery.
                    model.commit(t + HANDLING_NS, tag);
                    r.committed(tag);
                    tags.push((tag, t, moves[0].at_ns));
                    out.made.push(t);
                }
            }
        }
        model.advance(u64::MAX);
        if let (Some(first), Some(last)) = (model.flip_of(1), model.flip_of(tag)) {
            out.held = model
                .frames
                .iter()
                .filter(|f| f.tags.is_empty() && f.flip > first && f.flip < last)
                .count();
        }
        for (tag, t, stamp) in tags {
            if let Some(flip) = model.flip_of(tag) {
                out.lag.push(flip - t);
                out.stamps.push(stamp);
            }
        }
        out
    }

    fn within_a_refresh(lag: &[u64]) -> usize {
        lag.iter().filter(|&&l| l < PERIOD * 3 / 4).count()
    }

    #[test]
    fn a_page_that_costs_7_ms_a_frame_is_shown_at_the_refresh_it_is_aimed_at_and_not_a_refresh_later()
     {
        // The pages of the Show that cost this much were shown 24 ms after the finger on every frame of every drag (a delivery 8 ms
        // before the refresh is 1.4 ms too late for a frame that needs 7.5 ms and a flip's lead, and then every one after it).
        let run = drag(&Setup::new(7_500 * US), 80, 1.5);
        let steady = &run.lag[25..];
        assert_eq!(within_a_refresh(steady), steady.len(), "{:?}", run.lag);
        // And no sooner than the frame needs: the lead is what the frames ask for.
        let lead = 7_500 * US + FLIP_LEAD_NS + HANDLING_NS + MARGIN_NS;
        for &l in steady {
            assert!(
                l + MS >= lead && l <= lead + 2 * MS,
                "{l} for a lead of {lead}"
            );
        }
    }

    #[test]
    fn the_finger_is_shown_as_soon_as_the_frames_allow_a_light_page_sooner_than_a_heavy_one() {
        let lag_of = |render: u64| {
            let run = drag(&Setup::new(render), 70, 1.0);
            let steady = &run.lag[25..];
            assert_eq!(within_a_refresh(steady), steady.len(), "{:?}", run.lag);
            steady.iter().sum::<u64>() / steady.len() as u64
        };
        let (light, middle, heavy) = (lag_of(3 * MS), lag_of(5 * MS), lag_of(9 * MS));
        assert!(
            light + MS < middle && middle + MS < heavy,
            "{light} {middle} {heavy}"
        );
        assert!(light < 8 * MS && heavy < PERIOD * 3 / 4, "{light} {heavy}");
    }

    #[test]
    fn a_frame_in_flight_when_the_finger_starts_is_got_out_of_in_a_dozen_frames_and_the_page_never_jumps()
     {
        let mut setup = Setup::new(4_300 * US);
        setup.busy = true;
        let run = drag(&setup, 80, 1.5);
        let steady = &run.lag[30..];
        assert_eq!(within_a_refresh(steady), steady.len(), "{:?}", run.lag);
        // The deliveries stay a refresh apart, give or take what the lead may move in a frame: so a steady finger moves the page by a
        // steady step all the way, and never by two steps or none.
        for w in run.made.windows(2) {
            let gap = w[1] - w[0];
            assert!(
                gap + SLEW_NS + 300 * US >= PERIOD && gap <= PERIOD + SLEW_NS + 300 * US,
                "{gap} between two deliveries: {:?}",
                run.made
            );
        }
    }

    #[test]
    fn a_frame_that_lands_a_refresh_late_costs_the_finger_a_few_frames_not_the_rest_of_the_drag() {
        let mut setup = Setup::new(4_300 * US);
        setup.spikes = vec![(30, 9 * MS)];
        let run = drag(&setup, 100, 1.5);
        let steady = &run.lag[60..];
        assert_eq!(within_a_refresh(steady), steady.len(), "{:?}", run.lag);
        // It did leave the refresh it was aimed at, and the pipeline held a frame back for a while.
        assert!(within_a_refresh(&run.lag[28..45]) < 17, "{:?}", run.lag);
    }

    #[test]
    fn deliveries_stay_a_refresh_apart_whatever_the_refresh_reports_wobble_by_a_few_microseconds() {
        // A delivery taken a few microseconds before the moment the refresh report had moved to was due again at once (the same
        // moment seen twice): a second delivery 0.3 ms after the first, a second commit, and a frame late for the rest of the drag.
        for seed_offset in 0..4u64 {
            let mut setup = Setup::new(4_300 * US + seed_offset * 700 * US);
            setup.wobble_ns = 25 * US;
            let run = drag(&setup, 90, 0.8);
            for w in run.made.windows(2) {
                assert!(w[1] - w[0] > PERIOD / 2, "{:?}", run.made);
            }
            let steady = &run.lag[25..];
            assert_eq!(within_a_refresh(steady), steady.len(), "{:?}", run.lag);
        }
    }

    /// The swipe of the release gate's dock workload as the Show draws it (trace of black, 10-08): frames that cost 9.7 ms and one
    /// of 12.5 (a backdrop), a finger that moves 1 px a millisecond, and the focus it begins: a settle of a motion track that runs a
    /// few hundred milliseconds, a frame of the compositor's own at every refresh from the moment it was asked for.
    fn swipe(animation_ms: u64) -> Setup {
        let mut setup = Setup::new(9_700 * US);
        setup.spikes = vec![(4, 2_800 * US)];
        setup.animation_ns = Some(animation_ms * MS);
        setup
    }

    #[test]
    fn a_finger_that_drags_while_the_compositor_animates_is_never_held_a_refresh_and_is_shown_a_refresh_and_the_least_lead_after()
     {
        // Every frame starts the moment the one before has flipped, so a commit is taken by the frame that starts at the flip after it
        // came, whichever part of the refresh it came in: the finger is shown a refresh and a part of one after it was handed on.
        // The lead that went on later through the refresh to find a pipeline that had drained took its deliveries past a flip: the
        // frame that started there found nothing new and showed the finger's step before it again (one of the five or six refreshes in
        // which the finger moved, in every dock swipe of the release gate: 20 of 113 on both Shows).
        for deliveries in [10, 24, 40] {
            let run = drag(&swipe(700), deliveries, 1.0);
            assert_eq!(run.held, 0, "{deliveries} deliveries: {:?}", run.lag);
            // The lead comes down from the start's 14 ms by a step a delivery (31, 27, 23, 20 ms from the finger to the screen) and
            // stays at the least: from the ninth delivery on the finger is shown a refresh and that lead after it was handed on.
            let after = &run.lag[run.lag.len().min(8)..];
            for &l in after {
                assert!(
                    (PERIOD..=PERIOD + MIN_LEAD_NS + MS).contains(&l),
                    "{l} ns after the delivery: {:?}",
                    run.lag
                );
            }
        }
    }

    #[test]
    fn when_the_animation_is_over_the_lead_goes_on_through_the_refresh_and_the_page_is_shown_within_a_refresh_again_without_a_hold()
     {
        let run = drag(&swipe(300), 60, 1.0);
        assert_eq!(run.held, 0, "{:?}", run.lag);
        let steady = &run.lag[45..];
        assert_eq!(within_a_refresh(steady), steady.len(), "{:?}", run.lag);
    }

    // ---- the lift ---------------------------------------------------------------------------------------------------------

    #[test]
    fn a_finger_that_travelled_lifts_at_the_next_delivery_with_the_position_it_would_have_had_and_a_tap_lifts_at_once()
     {
        let mut r = Resampler::new(59.64);
        r.vsync(1_000 * MS);
        let mut events = vec![(1_000 * MS, ev(TouchPhase::Down, 600.0, 400.0, 1_000 * MS))];
        for k in 1..=4u64 {
            let at = 1_000 * MS + k * 10 * MS;
            events.push((at, ev(TouchPhase::Move, 600.0, 400.0 - 20.0 * k as f32, at)));
        }
        // The lift arrives a digitiser report after the last move, 9 ms after it.
        let lift_at = 1_049 * MS;
        events.push((lift_at, ev(TouchPhase::Up, 600.0, 320.0, lift_at)));
        let out = play(&mut r, &events, 1_200 * MS);
        let (n, up) = (out.len(), out.last().unwrap());
        assert_eq!(up.1.phase, TouchPhase::Up);
        // It was not handed on when it arrived, but at a delivery after it, together with the move before it…
        assert!(up.0 > lift_at, "lifted at {} ms", up.0 / MS);
        let (moved_at, moved) = out[n - 2];
        assert_eq!((moved_at, moved.phase), (up.0, TouchPhase::Move));
        // …at the same moment and place: a little past the last report along its speed (2 px/ms), not where the finger was last seen.
        assert_eq!(
            (moved.x, moved.y, moved.at_ns),
            (up.1.x, up.1.y, up.1.at_ns)
        );
        assert!(
            moved.y < 320.0 - 10.0 && moved.at_ns > 1_040 * MS,
            "{moved:?}"
        );
        assert_eq!(r.due(), None, "the contact is over");
        // A tap, even with a pixel of wobble, is lifted at once, as it always was.
        r.feed(ev(TouchPhase::Down, 100.0, 100.0, 2_000 * MS));
        r.feed(ev(TouchPhase::Move, 101.0, 100.0, 2_010 * MS));
        let out = r.feed(ev(TouchPhase::Up, 101.0, 100.0, 2_060 * MS));
        let phases: Vec<TouchPhase> = out.into_iter().flatten().map(|e| e.phase).collect();
        assert_eq!(phases, [TouchPhase::Move, TouchPhase::Up]);
    }

    #[test]
    fn a_finger_that_lands_before_another_finger_lift_was_handed_on_has_that_lift_handed_on_first()
    {
        let mut r = Resampler::new(59.64);
        r.vsync(1_000 * MS);
        r.feed(ev(TouchPhase::Down, 600.0, 400.0, 1_000 * MS));
        r.feed(ev(TouchPhase::Move, 600.0, 380.0, 1_010 * MS));
        r.feed(ev(TouchPhase::Move, 600.0, 340.0, 1_020 * MS));
        assert_eq!(
            r.feed(ev(TouchPhase::Up, 600.0, 340.0, 1_030 * MS)),
            [None; 4]
        );
        let out = r.feed(ev(TouchPhase::Down, 300.0, 200.0, 1_035 * MS));
        let phases: Vec<TouchPhase> = out.into_iter().flatten().map(|e| e.phase).collect();
        // The last position of the first finger, its lift, and the new finger: in that order, at once.
        assert_eq!(phases, [TouchPhase::Move, TouchPhase::Up, TouchPhase::Down]);
        assert_eq!(r.due(), None);
    }

    // ---- where the finger is ---------------------------------------------------------------------------------------------

    /// Swipes of the Reminders page made with a real finger on the Show (bundles 20261005T220331Z and T220522Z): the digitiser's
    /// stamp in microseconds from the landing, the phase, the position.
    const SWIPES: &[&[(u64, char, f32, f32)]] = &[
        &[
            (0, 'D', 854.0, 206.0),
            (30093, 'M', 852.0, 210.0),
            (40160, 'M', 850.0, 224.0),
            (50190, 'M', 848.0, 244.0),
            (60340, 'M', 846.0, 266.0),
            (70385, 'M', 846.0, 285.0),
            (80419, 'M', 845.0, 307.0),
            (90328, 'M', 845.0, 331.0),
            (100389, 'M', 848.0, 357.0),
            (109883, 'U', 848.0, 357.0),
        ],
        &[
            (0, 'D', 846.0, 360.0),
            (20105, 'M', 846.0, 358.0),
            (30227, 'M', 847.0, 341.0),
            (40396, 'M', 847.0, 314.0),
            (50337, 'M', 850.0, 278.0),
            (60330, 'M', 857.0, 246.0),
            (70455, 'M', 866.0, 213.0),
            (80474, 'M', 877.0, 182.0),
            (90280, 'M', 892.0, 151.0),
            (99735, 'U', 892.0, 151.0),
        ],
        &[
            (0, 'D', 852.0, 372.0),
            (20220, 'M', 852.0, 370.0),
            (30290, 'M', 852.0, 354.0),
            (40415, 'M', 852.0, 328.0),
            (50431, 'M', 854.0, 294.0),
            (60463, 'M', 860.0, 262.0),
            (70531, 'M', 868.0, 228.0),
            (80476, 'M', 879.0, 195.0),
            (90384, 'M', 893.0, 163.0),
            (100350, 'M', 909.0, 133.0),
            (110273, 'M', 928.0, 105.0),
            (119721, 'U', 928.0, 105.0),
        ],
        &[
            (0, 'D', 756.0, 362.0),
            (30248, 'M', 756.0, 359.0),
            (40571, 'M', 757.0, 345.0),
            (50475, 'M', 758.0, 326.0),
            (60587, 'M', 760.0, 302.0),
            (70606, 'M', 762.0, 280.0),
            (80562, 'M', 766.0, 256.0),
            (90592, 'M', 772.0, 228.0),
            (100788, 'M', 779.0, 200.0),
            (110712, 'M', 788.0, 172.0),
            (120665, 'M', 800.0, 147.0),
            (130627, 'M', 815.0, 122.0),
            (139959, 'U', 815.0, 122.0),
        ],
        &[
            (0, 'D', 821.0, 317.0),
            (60390, 'M', 821.0, 315.0),
            (70474, 'M', 823.0, 291.0),
            (80564, 'M', 827.0, 257.0),
            (90540, 'M', 835.0, 214.0),
            (100564, 'M', 845.0, 180.0),
            (110469, 'M', 857.0, 148.0),
            (120567, 'M', 868.0, 120.0),
            (129991, 'U', 868.0, 120.0),
        ],
    ];

    /// Where the real finger was at `t_us` from the landing, on the line through its reports.
    fn finger_at(swipe: &[(u64, char, f32, f32)], t_us: f64) -> Option<f32> {
        swipe
            .windows(2)
            .filter(|w| w[1].1 == 'M' || w[1].1 == 'D')
            .find(|w| (w[0].0 as f64..=w[1].0 as f64).contains(&t_us))
            .map(|w| {
                let f = (t_us - w[0].0 as f64) / (w[1].0 - w[0].0) as f64;
                w[0].3 + (w[1].3 - w[0].3) * f as f32
            })
    }

    #[test]
    fn a_swipe_made_with_a_real_finger_is_handed_on_where_the_finger_is_to_a_few_pixels_at_the_moment_it_is_stamped()
     {
        // Fitted to the reports of 35 ms the speed of a finger that speeds up lags it (6 px of error at 8 ms past the newest report,
        // 9 at 12 ms, over these swipes); from the two newest reports, 3.5 and 5.7. A flick starts from rest, and the first two
        // reports after the dwell do not say how fast it will be: the deliveries before the third are left out.
        let mut errs: Vec<f32> = Vec::new();
        let mut ahead_ms: Vec<f64> = Vec::new();
        for swipe in SWIPES {
            for phase in 0..6u64 {
                let start = 5_000 * MS;
                let mut r = Resampler::new(59.64);
                // A refresh `phase` sixths of a period after the landing.
                r.vsync(start + phase * PERIOD / 6);
                let mut reports = 0;
                for &(us, ph, x, y) in swipe.iter() {
                    let at = start + us * US;
                    let phase_of = match ph {
                        'D' => TouchPhase::Down,
                        'M' => TouchPhase::Move,
                        _ => continue,
                    };
                    // Handed on as the loop would: a delivery that falls due before this report arrives comes first.
                    while let Some(due) = r.due().filter(|&d| d < at) {
                        for e in r.take(due) {
                            let t_us = (e.at_ns as f64 - start as f64) / 1e3;
                            if let Some(truth) = finger_at(swipe, t_us)
                                .filter(|_| e.phase == TouchPhase::Move && reports >= 3)
                            {
                                errs.push((e.y - truth).abs());
                                ahead_ms.push((e.at_ns as f64 - due as f64) / 1e6);
                            }
                        }
                    }
                    r.feed(ev(phase_of, x, y, at));
                    reports += usize::from(ph == 'M');
                }
            }
        }
        errs.sort_by(f32::total_cmp);
        let rms = (errs.iter().map(|e| e * e).sum::<f32>() / errs.len() as f32).sqrt();
        let (p50, p90) = (errs[errs.len() / 2], errs[errs.len() * 9 / 10]);
        let ahead = ahead_ms.iter().sum::<f64>() / ahead_ms.len() as f64;
        assert!(errs.len() > 60, "{}", errs.len());
        // The position is stamped ahead of the moment of the delivery (so that it is where the finger will be when the frame
        // shows) and is still within a few pixels of the finger there.
        assert!(ahead > 4.0, "stamped {ahead:.1} ms past the delivery");
        assert!(
            p50 < 3.0 && p90 < 10.0 && rms < 6.0,
            "p50 {p50:.1} p90 {p90:.1} rms {rms:.1} px over {}",
            errs.len()
        );
    }

    #[test]
    fn a_prediction_never_goes_further_than_a_few_reports_past_the_newest_and_a_slow_finger_is_not_predicted()
     {
        let reports: VecDeque<TouchEvent> = (0..4)
            .map(|k| ev(TouchPhase::Move, 10.0 * k as f32, 240.0, (10 + k * 10) * MS))
            .collect();
        // Between reports: on the line through them.
        assert_eq!(position_at(&reports, 25 * MS).x, 15.0);
        // 5 ms past the newest (at 40 ms, x 30): 1 px/ms on.
        assert!((position_at(&reports, 45 * MS).x - 35.0).abs() < 1e-3);
        // Far past it: no more than 20 ms on.
        let far = position_at(&reports, 200 * MS);
        assert!(
            (far.x - 50.0).abs() < 1e-3 && far.at_ns == 60 * MS,
            "{far:?}"
        );
        // One report, or reports too close together or too far apart to give a speed: the newest as it is, stamped for the moment.
        let one = VecDeque::from([ev(TouchPhase::Move, 7.0, 240.0, 10 * MS)]);
        assert_eq!(position_at(&one, 30 * MS).x, 7.0);
        let close = VecDeque::from([
            ev(TouchPhase::Move, 7.0, 240.0, 10 * MS),
            ev(TouchPhase::Move, 8.0, 240.0, 11 * MS),
        ]);
        assert_eq!(position_at(&close, 30 * MS).x, 8.0);
        let missed = VecDeque::from([
            ev(TouchPhase::Move, 7.0, 240.0, 10 * MS),
            ev(TouchPhase::Move, 27.0, 240.0, 50 * MS),
        ]);
        assert_eq!(position_at(&missed, 60 * MS).x, 27.0);
        // A finger at 0.04 px/ms (a pixel in 25 ms) is the digitiser's rounding, not a speed: it stays where it is.
        let slow = VecDeque::from([
            ev(TouchPhase::Move, 100.0, 240.0, 10 * MS),
            ev(TouchPhase::Move, 100.4, 240.0, 20 * MS),
        ]);
        assert_eq!(position_at(&slow, 30 * MS).x, 100.4);
    }

    // ---- what is held, and when ----------------------------------------------------------------------------------------

    #[test]
    fn moves_are_handed_on_once_per_refresh_at_the_lead_the_frames_ask_for() {
        let mut r = Resampler::new(59.64);
        let vsync = 1_000 * MS;
        r.vsync(vsync);
        assert_eq!(r.due(), None, "no finger");
        r.feed(ev(TouchPhase::Down, 0.0, 0.0, vsync));
        assert_eq!(r.due(), None, "a finger that has not moved");
        r.feed(ev(TouchPhase::Move, 10.0, 0.0, vsync + 2 * MS));
        let due = r.due().expect("a move is held");
        // A refresh less the lead: what the (assumed) frame cost, the flip, the loop and the margin ask for, and a little more at first.
        let lead = r.lead_ns();
        assert_eq!(
            lead,
            FIRST_RENDER_NS + FLIP_LEAD_NS + HANDLING_NS + MARGIN_NS + START_EXTRA_NS
        );
        assert_eq!(due, vsync + PERIOD - lead);
        assert_eq!(r.take(due).len(), 1);
        // Past this refresh's moment, the next is the refresh after.
        r.feed(ev(TouchPhase::Move, 20.0, 0.0, due + MS));
        assert_eq!(r.due(), Some(vsync + 2 * PERIOD - lead));
        // The frame that showed it cost 4 ms: the lead moves towards what that asks for, a step a frame.
        r.committed(1);
        r.frame(vsync + PERIOD, 4 * MS, 1);
        assert_eq!(r.lead_ns(), lead - SLEW_NS);
        // A delivery shown a refresh later than it was aimed at makes the next ones later in the refresh, not earlier.
        let due = r.due().expect("held");
        r.take(due);
        r.committed(2);
        r.frame(due + lead + PERIOD, 4 * MS, 2);
        assert_eq!(r.lead_ns(), lead - 2 * SLEW_NS);
    }

    /// Feeds `events` (arrival time, event) in order, taking what falls due before each, and then what falls due up to `until`;
    /// returns what was handed on and when.
    fn play(r: &mut Resampler, events: &[(u64, TouchEvent)], until: u64) -> Vec<(u64, TouchEvent)> {
        let mut out = Vec::new();
        for &(at, e) in events {
            while let Some(due) = r.due().filter(|&d| d <= at) {
                out.extend(r.take(due).into_iter().map(|e| (due, e)));
            }
            out.extend(r.feed(e).into_iter().flatten().map(|e| (at, e)));
        }
        while let Some(due) = r.due().filter(|&d| d <= until) {
            out.extend(r.take(due).into_iter().map(|e| (due, e)));
        }
        out
    }

    #[test]
    fn work_that_is_not_the_fingers_waits_for_the_next_delivery_while_a_finger_is_moving_and_never_once_it_has_stopped()
     {
        let mut r = Resampler::new(59.64);
        r.vsync(1_000 * MS);
        assert_eq!(r.hold_until(), None, "no finger");
        r.feed(ev(TouchPhase::Down, 0.0, 0.0, 1_000 * MS));
        assert!(
            !r.expecting(1_002 * MS),
            "a finger that has not moved has nothing due and has made no delivery to wait for"
        );
        r.feed(ev(TouchPhase::Move, 0.0, -20.0, 1_010 * MS));
        // A move is held for a delivery that is due: work 1.5 ms before it waits for it (the lone commit before a drag's first one).
        let first_due = r.due().expect("a move is held");
        assert!(r.expecting(first_due - 1_500_000));
        for k in 2..=3u64 {
            r.feed(ev(
                TouchPhase::Move,
                0.0,
                -20.0 * k as f32,
                1_000 * MS + k * 10 * MS,
            ));
        }
        let due = r.due().expect("a move is held");
        r.take(due);
        // Four milliseconds behind a delivery (the lone commit that cost a drag its start): it waits, for a refresh and a half.
        assert!(r.expecting(due + 4 * MS));
        assert!(r.expecting(due + PERIOD));
        // The finger went on and the next delivery was made: the wait runs from that one.
        r.feed(ev(TouchPhase::Move, 0.0, -80.0, due + 6 * MS));
        let next = r.due().expect("held");
        r.take(next);
        assert_eq!(r.hold_until(), Some(next + PERIOD * 3 / 2));
        // A finger that stopped (nothing more came) is not waited for past that.
        assert!(!r.expecting(next + PERIOD * 3 / 2));
        // Lifted: nothing is held.
        r.feed(ev(TouchPhase::Up, 0.0, -80.0, next + 40 * MS));
        let lift = r.due().expect("the lift waits for a delivery");
        r.take(lift);
        assert_eq!(r.hold_until(), None);
    }

    #[test]
    fn a_finger_that_stops_is_caught_up_with_and_nothing_rests_ahead_of_it() {
        let mut r = Resampler::new(59.64);
        let vsync = 1_000 * MS;
        r.vsync(vsync);
        let mut events = vec![(vsync, ev(TouchPhase::Down, 0.0, 0.0, vsync))];
        for k in 1..=5u64 {
            let at = vsync + k * 10 * MS;
            events.push((at, ev(TouchPhase::Move, 10.0 * k as f32, 0.0, at)));
        }
        // Five reports at 1 px/ms, then silence: the finger stopped at 50.
        let out = play(&mut r, &events, vsync + 400 * MS);
        let xs: Vec<f32> = out.iter().map(|(_, e)| e.x).collect();
        assert!(xs.len() >= 3, "{xs:?}");
        // The last delivery is the finger's last real position, the one before it was predicted past it.
        assert_eq!(*xs.last().unwrap(), 50.0, "{xs:?}");
        assert!(xs[xs.len() - 2] > 50.0, "{xs:?}");
        // And nothing after: the finger is where the page is.
        assert_eq!(r.due(), None);
        assert!(
            xs.windows(2).all(|w| w[1] >= w[0] || w[1] == 50.0),
            "{xs:?}"
        );
    }

    #[test]
    fn down_and_cancel_are_acted_on_at_once_after_a_held_last_position() {
        let mut r = Resampler::new(60.0);
        let phases = |out: [Option<TouchEvent>; 4]| -> Vec<(TouchPhase, f32)> {
            out.into_iter().flatten().map(|e| (e.phase, e.x)).collect()
        };
        assert_eq!(
            phases(r.feed(ev(TouchPhase::Down, 5.0, 0.0, MS))),
            [(TouchPhase::Down, 5.0)]
        );
        // A flick quicker than a frame, then the contact is lost: its last position is handed on with the cancel, not lost.
        assert!(phases(r.feed(ev(TouchPhase::Move, 9.0, 0.0, 11 * MS))).is_empty());
        assert_eq!(
            phases(r.feed(ev(TouchPhase::Cancel, 12.0, 0.0, 14 * MS))),
            [(TouchPhase::Move, 9.0), (TouchPhase::Cancel, 12.0)]
        );
        assert_eq!(r.due(), None, "a lost finger has no move to hand on");
        // Nothing held (already handed on): the cancel alone.
        r.feed(ev(TouchPhase::Down, 5.0, 0.0, 30 * MS));
        r.feed(ev(TouchPhase::Move, 9.0, 0.0, 40 * MS));
        let due = r.due().expect("held");
        assert_eq!(r.take(due).len(), 1);
        let out = phases(r.feed(ev(TouchPhase::Cancel, 9.0, 0.0, due + MS)));
        assert_eq!(out, [(TouchPhase::Cancel, 9.0)]);
        assert!(r.take(due + 20 * MS).is_empty());
    }
}
