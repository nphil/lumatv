//! Frame pacing: which refresh a frame is meant for, when its rendering should start, and whether it arrived
//! late.
//!
//! Two questions get different answers on purpose. *When to start* is answered pessimistically: as late as the
//! slowest recent frame would still make its refresh, with slack besides, so that commits arriving until then are
//! drawn in the frame. *Which refresh to aim for* is answered optimistically: the next one, unless a typical frame
//! could not make it. Giving up a refresh on a pessimistic guess costs a whole refresh for certain; trying costs
//! one only if the frame really is slow.
//!
//! The recent frames say what the next one costs only while the scene stays the same kind of scene. A commit that
//! reshapes it (layers made, an animation started) is drawn cheaply in its own frame, and the animation's first step on
//! the next frame brings the new layers into view: the lab's alarm and notifications screens doubled their cost there
//! (6.6 to 13.1 ms, 5.7 to 11.6 ms) and lost a refresh to the latch. So the first animation frame after a reshaping
//! commit starts at once ([`Pacer::reshaped`]); the one after it is latched on a window that has seen the new scene.
//!
//! Frames are drawn for a *slot*, the time the frame is expected to be on screen, and animations are evaluated at
//! the slot, not at whenever the CPU happened to run. Slots advance by exactly one refresh per frame as long as
//! frames follow each other, whatever the scheduler does to the moment a frame is submitted, so an animation's
//! steps are even. What ties the slot timeline to the real refreshes is the *phase*: a frame planned for `slot`
//! reaches the screen at `slot + phase`, learned from when `FBIOPAN_DISPLAY` returned for earlier frames (which on
//! this kernel is the vsync to within 0.1 ms). A frame that arrives whole refreshes late (or early) shifts the
//! timeline by those refreshes.
//!
//! What counts as a *missed* refresh is a separate question, answered when the frame has reached the screen: it is
//! one that a compositor which started the frame as soon as its content was there would have made. See
//! [`Pacer::presented`].

use std::collections::VecDeque;

/// How long before a refresh a flip must be requested to land on it: what `FBIOPAN_DISPLAY` needs on this kernel
/// (measured in experiments/s1-scanout).
const FLIP_LEAD_NS: u64 = 500_000;
/// Time a frame started at the latch is allowed beyond the slowest recent one: a slower frame, a late wake-up. Sized
/// on the frames that lost a refresh at the glass tiers with the GPU at 520 MHz (the timeline trace of pin, dialog,
/// alarm and notifications at T2): each cost 2 to 4 ms more than the slowest of the eight before it (a picture uploaded
/// on the frame that shows it, a backdrop made again, the first frame of a page's animation), and each was started at a
/// latch that left it 0.8 to 2.8 ms too little. Starting that much earlier lands on the same refresh; what it gives up is
/// the chance for a commit that arrives in those milliseconds to be drawn in the frame instead of the next.
const SLACK_NS: u64 = 5_500_000;
/// Pipeline time assumed until frames have been measured.
const INITIAL_PIPELINE_NS: u64 = 8_000_000;
/// Frames whose pipeline times are remembered.
pub const PIPELINE_WINDOW: usize = 8;
/// After this long without a frame, the refresh timeline is forgotten: the panel's clock drifts against ours.
const STALE_NS: u64 = 2_000_000_000;

pub struct Pacer {
    period_ns: u64,
    pipeline: VecDeque<u64>,
    /// `Some` once a frame of the current run has been seen on screen.
    phase_ns: Option<i64>,
    last_slot_ns: u64,
    /// When the last frame reached the screen; 0 before the first.
    last_presented_ns: u64,
    /// A frame has applied a commit that reshaped the scene, and no frame has been planned since.
    reshaped: bool,
}

fn round_div(value: i64, divisor: i64) -> i64 {
    (value + divisor / 2).div_euclid(divisor)
}

/// What the pacer expects of the next frames (see [`Pacer::forecast`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forecast {
    /// What a typical recent frame took, from starting to being ready to flip.
    pub typical_ns: u64,
    /// What is left of a refresh once a typical frame, the flip's lead and the pacer's slack are taken off it; negative when the
    /// pacer predicts late frames.
    pub headroom_ns: i64,
}

/// A frame that has reached the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    /// The refresh it was planned for ([`Plan::slot_ns`]).
    pub slot_ns: u64,
    /// When the render thread began it, when it was ready to flip, and when the flip returned (its refresh).
    pub started_ns: u64,
    pub ready_ns: u64,
    pub presented_ns: u64,
    /// When the oldest commit it shows reached the compositor; `None` if the scene asked for the frame itself (an
    /// animation, a wallpaper's step) and there was no content to wait for.
    pub content_ns: Option<u64>,
}

/// Where a frame landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landing {
    /// How many refreshes after its slot it arrived: negative if it landed before the refresh it was planned for (the
    /// plan allowed for a slower pipeline than it had), which moves the timeline back by as many refreshes.
    pub late: i32,
    /// How many refreshes it lost (see `Pacer::lost`). Whether they were missed is for the caller: only a frame that
    /// was due at every refresh can miss one.
    pub lost: u32,
}

/// Which refresh a frame is drawn for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// When the frame is expected to be on screen: the time animations are evaluated at.
    pub slot_ns: u64,
    /// Refreshes the frame gives up because it could not start early enough for the next one. Whether they were
    /// missed is [`Pacer::presented`]'s to say: a refresh that no frame with this content could have made was
    /// nobody's to miss.
    pub skipped: u32,
}

/// The pacer's state a plan changes ([`Pacer::mark`]).
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    last_slot_ns: u64,
    phase_ns: Option<i64>,
    reshaped: bool,
}

impl Pacer {
    pub fn new(period_ns: u64) -> Self {
        assert!(period_ns > 0);
        Self {
            period_ns,
            pipeline: VecDeque::from([INITIAL_PIPELINE_NS; PIPELINE_WINDOW]),
            phase_ns: None,
            last_slot_ns: 0,
            last_presented_ns: 0,
            reshaped: false,
        }
    }

    /// How long a frame takes, from starting to being ready to flip: usually, and at worst lately. The median of the recent
    /// frames, worked out on the stack: it is asked for by every frame, and by the ladder (`forecast`).
    fn typical_pipeline_ns(&self) -> u64 {
        let mut sorted = [0u64; PIPELINE_WINDOW];
        for (slot, ns) in sorted.iter_mut().zip(&self.pipeline) {
            *slot = *ns;
        }
        sorted.sort_unstable();
        sorted[PIPELINE_WINDOW / 2]
    }

    /// What the pacer expects of the next frames, if they are wanted at every refresh and cost what a typical recent frame did.
    /// **A negative `headroom_ns` is the pacer's own prediction that they will be late**: a refresh, less what a flip needs, less the
    /// slack the pacer gives itself for a frame that costs more than the ones before it (`SLACK_NS`: what such frames cost), less
    /// that typical frame, is what is left, and with none left the pacer could not start a frame in time for its refresh with a
    /// typical pipeline and the slack besides, so a frame that costs what frames do now and a spike over it is a lost refresh. It is
    /// the median and not the slowest that is meant: one slow frame among quick ones (a screen's first, a picture uploaded) says
    /// nothing of the frames after it, and the slack is there for it.
    pub fn forecast(&self) -> Forecast {
        let typical_ns = self.typical_pipeline_ns();
        Forecast {
            typical_ns,
            headroom_ns: self.period_ns as i64 - (FLIP_LEAD_NS + SLACK_NS + typical_ns) as i64,
        }
    }

    fn slowest_pipeline_ns(&self) -> u64 {
        self.pipeline
            .iter()
            .copied()
            .max()
            .unwrap_or(INITIAL_PIPELINE_NS)
    }

    /// Whether frames are following one another: the last reached the screen no more than a refresh and a half
    /// before `now_ns`. A frame that follows a pause is the first of a run, and is not one that was due.
    pub fn flowing(&self, now_ns: u64) -> bool {
        self.last_presented_ns != 0
            && now_ns.saturating_sub(self.last_presented_ns) <= self.period_ns * 3 / 2
    }

    fn in_run(&self, now_ns: u64) -> bool {
        self.phase_ns.is_some() && now_ns.saturating_sub(self.last_presented_ns) <= STALE_NS
    }

    /// When a frame that nothing but a running animation asks for should start: as late as the slowest recent
    /// frame could still make its refresh, with slack, so that requests arriving in the meantime are drawn in it.
    /// `now_ns` when there is no timeline yet, it is already time, or the frame before reshaped the scene.
    pub fn latch_at(&self, now_ns: u64) -> u64 {
        if self.reshaped {
            return now_ns;
        }
        self.latch_for(now_ns, self.last_slot_ns + self.period_ns)
    }

    /// The frame being drawn applied a commit that reshaped the scene (`ops::Op::reshapes`): the next animation frame
    /// may cost what none of the recent ones did, so it starts as soon as it can.
    pub fn reshaped(&mut self) {
        self.reshaped = true;
    }

    /// The refresh the next frame is drawn for, unless it has to skip one: the slot after the last.
    pub fn next_slot_ns(&self) -> u64 {
        self.last_slot_ns + self.period_ns
    }

    /// Whether the next animation frame starts at once for want of a reason to wait (see [`Pacer::reshaped`]).
    pub fn is_reshaped(&self) -> bool {
        self.reshaped
    }

    /// The same for a frame that is wanted for the refresh at `slot_ns` or, if that has passed, the next one: a
    /// wallpaper that moves every so many refreshes sleeps until then instead of drawing frames nobody asked for.
    pub fn latch_for(&self, now_ns: u64, slot_ns: u64) -> u64 {
        let Some(phase) = self.phase_ns.filter(|_| self.in_run(now_ns)) else {
            return now_ns;
        };
        let next = self.last_slot_ns + self.period_ns;
        let display = slot_ns.max(next) as i64 + phase;
        let budget = (self.slowest_pipeline_ns() + FLIP_LEAD_NS + SLACK_NS) as i64;
        (display - budget).max(now_ns as i64) as u64
    }

    /// Chooses the slot for a frame that starts rendering at `now_ns` and remembers it as the latest.
    pub fn plan(&mut self, now_ns: u64) -> Plan {
        self.reshaped = false;
        // The earliest a typical frame started now could be on screen.
        let reachable = (now_ns + self.typical_pipeline_ns() + FLIP_LEAD_NS) as i64;
        let (slot, skipped) = match self.phase_ns.filter(|_| self.in_run(now_ns)) {
            Some(phase) => {
                let mut slot = (self.last_slot_ns + self.period_ns) as i64;
                let short = reachable - (slot + phase);
                let mut skipped = 0;
                if short > 0 {
                    // Too late for the next refresh: skip to one that can still be made.
                    let period = self.period_ns as i64;
                    skipped = (short + period - 1) / period;
                    slot += skipped * period;
                }
                (slot as u64, skipped as u32)
            }
            None => {
                self.phase_ns = None;
                (reachable as u64, 0)
            }
        };
        self.last_slot_ns = slot;
        Plan {
            slot_ns: slot,
            skipped,
        }
    }

    /// What [`Pacer::plan`] is about to change, for [`Pacer::restore`].
    pub fn mark(&self) -> Mark {
        Mark {
            last_slot_ns: self.last_slot_ns,
            phase_ns: self.phase_ns,
            reshaped: self.reshaped,
        }
    }

    /// Takes back the plan made since `mark`: a frame that was planned and then not drawn (the commit it was planned for showed
    /// nothing new, see `engine::shows_nothing`) must not move the timeline on by a refresh, or the frame after it would be aimed
    /// at a refresh that is later than it need be.
    pub fn restore(&mut self, mark: Mark) {
        self.last_slot_ns = mark.last_slot_ns;
        self.phase_ns = mark.phase_ns;
        self.reshaped = mark.reshaped;
    }

    /// The frame `shown` is on screen. Learns from it: the time the pipeline took, and where its refresh fell against
    /// the slot it was planned for, which realigns the timeline if it fell on another one.
    pub fn presented(&mut self, shown: &Shown) -> Landing {
        let pipeline_ns = shown.ready_ns.saturating_sub(shown.started_ns);
        let lost = self.lost(shown, pipeline_ns);
        if self.pipeline.len() == PIPELINE_WINDOW {
            self.pipeline.pop_front();
        }
        self.pipeline.push_back(pipeline_ns);
        self.last_presented_ns = shown.presented_ns;
        let seen = shown.presented_ns as i64 - shown.slot_ns as i64;
        let Some(phase) = self.phase_ns else {
            self.phase_ns = Some(seen);
            return Landing { late: 0, lost };
        };
        let period = self.period_ns as i64;
        let off = seen - phase;
        let refreshes = round_div(off, period);
        // Follow slow drift between the two clocks, and ignore jitter.
        self.phase_ns = Some(phase + (off - refreshes * period) / 16);
        if refreshes != 0 {
            self.last_slot_ns = (self.last_slot_ns as i64 + refreshes * period) as u64;
        }
        Landing {
            late: refreshes as i32,
            lost,
        }
    }

    /// The refreshes `shown` lost: how many later than the first refresh a frame could have been on had it started as
    /// soon as it could, with the pipeline it took. It could start once its content was there (`content_ns`; a frame
    /// the scene asked for has no content to wait for) and the frame before had been shown: this compositor keeps
    /// one frame in flight.
    ///
    /// This is what a missed refresh is: time the compositor could have saved. A frame that reaches the screen when a
    /// slower or later one could not have is not that: a commit that turns up too late for a refresh has nothing to
    /// show at it (a video that changes every second refresh, a slider that changes the picture every third), and a
    /// frame that costs more than its neighbours lands where its cost puts it (the render time is a gate of its own).
    /// A frame that starts late for its refresh, or waits for the wrong thing, is.
    fn lost(&self, shown: &Shown, pipeline_ns: u64) -> u32 {
        let before = self.last_presented_ns;
        if before == 0 {
            return 0;
        }
        let period = self.period_ns;
        let earliest = shown.content_ns.map_or(before, |c| c.max(before));
        let ready = earliest + pipeline_ns + FLIP_LEAD_NS;
        let best = ready.saturating_sub(before).div_ceil(period).max(1);
        let seen =
            round_div(shown.presented_ns as i64 - before as i64, period as i64).max(1) as u64;
        seen.saturating_sub(best) as u32
    }

    /// How far after a slot its refresh appears, once a frame of the current run has been seen.
    pub fn phase_ns(&self) -> Option<i64> {
        self.phase_ns
    }
}

#[cfg(test)]
mod tests {
    // The same tests at the refresh of each panel the Show is sold with: the Truly one's 59.64 Hz and the BOE one's 62.26 Hz.

    mod truly_panel_59_64_hz {
        const PERIOD: u64 = 16_767_270;
        include!("pacer_tests.rs");
    }

    mod boe_panel_62_26_hz {
        const PERIOD: u64 = 16_061_677;
        include!("pacer_tests.rs");
    }
}
