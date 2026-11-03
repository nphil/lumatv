//! Shared scalar motion tracks: the pure equations the compositor evaluates at its predicted presentation time and the
//! app evaluates to estimate (hit-test, virtualise) without a round trip. Both sides run this one code, so an estimate
//! is the image the screen shows, not an approximation of it.
//!
//! A [`Track`] is one number (a dock's progress, a list's scroll offset) with bounds. It is *driven* by pointer samples
//! (the finger's already resampled position; no prediction is added here), *caught* when a finger lands on it while it
//! settles (it keeps its actual value, so nothing jumps), and *settled* from that same actual value and measured velocity
//! by one of three analytic plans: a timed ease, a critically damped spring, or scroll inertia (exponential friction that
//! snaps to rest anchors, stops at a hard bound or springs back from a permitted overscroll). Every plan is a closed
//! form of the time since it began: nothing is integrated per frame, so the value is the same whatever the frame times
//! were, and a frame that comes late lands where an on-time one would have.

use crate::Ease;
use serde::{Deserialize, Serialize};

/// Tracks a client may have at once (the resource ledger of §11.2). A track at rest costs the compositor nothing per frame (no frame, no
/// wake, no evaluation: it is a closed form of the time since it began) and a few dozen bytes, so the number is the room a shell and
/// the shell that replaces it have together (a lab screen is built before the one it replaces goes); the resting home holds
/// at most 16 of them (`shell_smart`'s own pin).
pub const MAX_MOTIONS: usize = 32;
/// Rest anchors a track may name.
pub const MAX_ANCHORS: usize = 8;
/// The pointer samples a velocity is measured over: the slope of the samples this long before the newest. A swipe starts from rest
/// and speeds up as it goes, so a speed averaged over more than this is well below the one the finger lifts at (over 100 ms from
/// where it landed, a flick that had just got up to speed let go at 62 % of it: measured on the swipes made on the Show).
pub const VELOCITY_WINDOW_NS: u64 = 50_000_000;
const HISTORY: usize = 8;

/// Names a track. Allocated by the app from a counter that only rises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MotionId(pub u32);

/// A CSS `cubic-bezier(x1, y1, x2, y2)`: the curve of progress against time from (0, 0) to (1, 1) with these two control points.
/// Evaluated as a browser does (the parameter whose x is the time, found by Newton's method with a bisection to fall back on, then
/// the curve's y there), so that a curve of the design language's tokens is the same curve here as in its CSS, not one it resembles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    /// `x(s) = ((a s + b) s + c) s`, and the same for `y`.
    x: [f64; 3],
    y: [f64; 3],
}

impl CubicBezier {
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        let (cx, cy) = (3.0 * x1, 3.0 * y1);
        let (bx, by) = (3.0 * (x2 - x1) - cx, 3.0 * (y2 - y1) - cy);
        Self {
            x: [1.0 - cx - bx, bx, cx],
            y: [1.0 - cy - by, by, cy],
        }
    }

    fn x_at(&self, s: f64) -> f64 {
        ((self.x[0] * s + self.x[1]) * s + self.x[2]) * s
    }

    fn y_at(&self, s: f64) -> f64 {
        ((self.y[0] * s + self.y[1]) * s + self.y[2]) * s
    }

    fn dx_at(&self, s: f64) -> f64 {
        (3.0 * self.x[0] * s + 2.0 * self.x[1]) * s + self.x[2]
    }

    /// The parameter at which the curve's x is `t` (0..=1): Newton from `t` itself, which is close for every curve of the language,
    /// then bisection, which cannot fail for an x that never runs backwards.
    fn parameter_at(&self, t: f64) -> f64 {
        let mut s = t;
        for _ in 0..8 {
            let error = self.x_at(s) - t;
            if error.abs() < 1e-10 {
                return s;
            }
            let slope = self.dx_at(s);
            if slope.abs() < 1e-9 {
                break;
            }
            s -= error / slope;
            if !(0.0..=1.0).contains(&s) {
                break;
            }
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..64 {
            s = 0.5 * (lo + hi);
            if self.x_at(s) < t {
                lo = s;
            } else {
                hi = s;
            }
        }
        s
    }

    /// The curve's progress at time `t` (0..=1), both ends exact.
    pub fn at(&self, t: f64) -> f64 {
        if t <= 0.0 {
            0.0
        } else if t >= 1.0 {
            1.0
        } else {
            self.y_at(self.parameter_at(t))
        }
    }

    /// How many times its mean speed the curve starts at: its slope at time 0 (`y1 / x1`). What a glide that hands on a finger's speed
    /// is as long as: the distance is the speed times the time over this.
    pub fn start_slope(&self) -> f64 {
        self.y[2] / self.x[2]
    }
}

/// Lucent's `--lu-ease`, `cubic-bezier(.33,1,.68,1)`: every arrival, settle and fade-in ([`Ease::Out`]).
pub const OUT: CubicBezier = CubicBezier::new(0.33, 1.0, 0.68, 1.0);
/// Lucent's `--lu-ease-exit`, `cubic-bezier(.4,0,1,1)`: everything that leaves from rest ([`Ease::In`]).
pub const EXIT: CubicBezier = CubicBezier::new(0.4, 0.0, 1.0, 1.0);

/// The shape of [`Ease::Spring`]: a critically damped spring from rest, `1 - (1 + k u) e^(-k u)` over the time `u` (0..=1) it is given,
/// where `k` is the spring's natural frequency times that time. The shell's one spring has a response of 260 ms (`ω = 2π / 260 ms`) and
/// a release is given Lucent's 300 ms of scroll: `k = 2π · 300 / 260`, a quarter more than 7. Such a spring has come to within a
/// quarter of a pixel of a 32 px pull by then, which is the most a rubber band is pulled; `ui`'s tokens hold this to those numbers.
pub const SPRING_K: f64 = 7.25;

/// [`Ease::Spring`] at `t` (0..=1), stretched by under a hundredth so that it lands exactly.
fn spring_at(t: f64) -> f64 {
    let lands = |u: f64| 1.0 - (1.0 + SPRING_K * u) * (-SPRING_K * u).exp();
    lands(t) / lands(1.0)
}

/// Maps linear progress `t` (0..=1) to eased progress. Both ends are fixed points, so an animation always starts
/// and lands exactly. The one evaluation of every curve there is: the compositor's frames, its tracks and the app's estimates of
/// them all come here (`ease_f64` for the app-side animations that count in double precision).
pub fn ease(curve: Ease, t: f32) -> f32 {
    ease_f64(curve, f64::from(t)) as f32
}

/// [`ease`] in double precision.
pub fn ease_f64(curve: Ease, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        Ease::Linear => t,
        Ease::Out => OUT.at(t),
        Ease::In => EXIT.at(t),
        Ease::Spring if t >= 1.0 => 1.0,
        Ease::Spring => spring_at(t),
    }
}

/// How many times its mean speed an animation along `curve` starts at (see [`CubicBezier::start_slope`]; 1 for a linear one, 0 for a
/// curve that starts at rest).
pub fn start_slope(curve: Ease) -> f32 {
    match curve {
        Ease::Linear => 1.0,
        Ease::Out => OUT.start_slope() as f32,
        Ease::In | Ease::Spring => 0.0,
    }
}

/// How much of a finger's pull past an end the content follows, at the end itself: the one rubber band of every list, pager, sheet
/// and card (decision 35).
pub const RUBBER_FOLLOW: f32 = 0.35;

/// How far content shows past an end when a finger has pulled `excess` beyond it, with the cap `limit` it never reaches: the first
/// pixels follow at [`RUBBER_FOLLOW`] and each one after gives less, `limit * (1 - e^(-RUBBER_FOLLOW * excess / limit))`, continuous
/// and saturating (the approved mocks' `Kit.rubber`, with the follow of decision 35). Zero where there is nothing past the end or no
/// room is allowed.
pub fn rubber(excess: f32, limit: f32) -> f32 {
    if limit <= 0.0 || excess <= 0.0 {
        0.0
    } else {
        -limit * (-RUBBER_FOLLOW * excess / limit).exp_m1()
    }
}

/// The pull whose [`rubber`] shows `shown` (its inverse; a `shown` at the cap is read as just under it).
pub fn unrubber(shown: f32, limit: f32) -> f32 {
    if limit <= 0.0 || shown <= 0.0 {
        0.0
    } else {
        -(limit / RUBBER_FOLLOW) * (-shown.min(limit * 0.999) / limit).ln_1p()
    }
}

/// What a track's numbers mean. It only chooses the thresholds at which a settle counts as at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    /// 0 to 1 over the whole travel (a dock's progress).
    Progress,
    /// Logical pixels (a scroll offset).
    Pixels,
}

impl Units {
    /// The distance and speed under which a settle has arrived (half of the design's 0.5-pixel / 5-pixel-per-second
    /// ceiling for pixels; the same fractions of a unit for progress).
    fn rest(self) -> (f32, f32) {
        match self {
            Units::Pixels => (0.25, 2.5),
            Units::Progress => (0.0005, 0.005),
        }
    }
}

/// The extent of a track, where it rests, and how it responds to a finger.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionBounds {
    pub min: f32,
    pub max: f32,
    /// How far past `min`/`max` a finger may pull the value (rubber-banded: the pull resists), in the track's units;
    /// 0 clamps hard.
    pub overscroll: f32,
    /// Where a scroll settles (a pager's pages): the first `anchor_count` entries.
    pub anchors: [f32; MAX_ANCHORS],
    pub anchor_count: u8,
    pub units: Units,
    /// Track units per pointer unit: how far the value moves for a pointer that travels one logical pixel (negative
    /// when dragging up should raise the value).
    pub ratio: f32,
}

impl MotionBounds {
    /// A track over `min..=max` with no anchors, no overscroll and a pointer ratio of 1.
    pub const fn range(min: f32, max: f32, units: Units) -> Self {
        Self {
            min,
            max,
            overscroll: 0.0,
            anchors: [0.0; MAX_ANCHORS],
            anchor_count: 0,
            units,
            ratio: 1.0,
        }
    }

    pub fn anchors(&self) -> &[f32] {
        &self.anchors[..usize::from(self.anchor_count).min(MAX_ANCHORS)]
    }

    pub fn check(&self) -> Result<(), &'static str> {
        let finite = [self.min, self.max, self.overscroll, self.ratio]
            .iter()
            .all(|v| v.is_finite());
        if !finite || self.min > self.max {
            return Err("motion bounds must be finite with min <= max");
        }
        if self.overscroll < 0.0 {
            return Err("overscroll cannot be negative");
        }
        if self.ratio == 0.0 {
            return Err("the pointer ratio cannot be zero");
        }
        if usize::from(self.anchor_count) > MAX_ANCHORS {
            return Err("too many rest anchors");
        }
        if self
            .anchors()
            .iter()
            .any(|a| !a.is_finite() || *a < self.min || *a > self.max)
        {
            return Err("rest anchors must lie within the bounds");
        }
        Ok(())
    }

    fn clamp(&self, v: f32) -> f32 {
        v.clamp(self.min, self.max)
    }

    /// The pull of a finger past the end: [`rubber`] of a raw excess, with this track's `overscroll` as its cap. Zero where overscroll
    /// is not permitted.
    pub fn resist(&self, excess: f32) -> f32 {
        rubber(excess, self.overscroll)
    }

    /// The raw excess whose resisted pull is `shown` (the inverse of [`MotionBounds::resist`]).
    pub fn unresist(&self, shown: f32) -> f32 {
        unrubber(shown, self.overscroll)
    }

    /// The value the finger's raw position shows: the bounds, with resisted overscroll past them.
    fn limit(&self, raw: f32) -> f32 {
        if raw < self.min {
            self.min - self.resist(self.min - raw)
        } else if raw > self.max {
            self.max + self.resist(raw - self.max)
        } else {
            raw
        }
    }

    /// The raw position that shows `value`.
    fn raw_of(&self, value: f32) -> f32 {
        if value < self.min {
            self.min - self.unresist(self.min - value)
        } else if value > self.max {
            self.max + self.unresist(value - self.max)
        } else {
            value
        }
    }

    fn nearest_anchor(&self, v: f32) -> Option<f32> {
        self.anchors()
            .iter()
            .copied()
            .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
    }
}

/// What a pointer sample is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrivePhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// One already-resampled pointer sample on a track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriveSample {
    /// The contact; a track follows the one that landed on it and ignores the others.
    pub contact: u32,
    pub phase: DrivePhase,
    /// The pointer's scalar position along the track's axis, logical pixels.
    pub position: f32,
    /// `CLOCK_MONOTONIC` nanoseconds of the sample.
    pub time_ns: u64,
}

/// A timed settle: `ease` over `duration_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionToken {
    pub duration_ms: u32,
    pub ease: Ease,
}

/// A critically damped spring that crosses its target no more than once, with a natural period of `response_ms`
/// (angular frequency `2 pi / response`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpringToken {
    pub response_ms: f32,
}

impl SpringToken {
    fn omega(&self) -> f64 {
        std::f64::consts::TAU / (f64::from(self.response_ms) / 1000.0)
    }
}

/// Scroll inertia: the velocity decays as `exp(-friction * t)` per second, and the `spring` is what brings an overscroll
/// (or a snap to an anchor) home.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollToken {
    pub friction: f32,
    pub spring: SpringToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Settle {
    Timed(MotionToken),
    CriticalSpring(SpringToken),
    Scroll(ScrollToken),
    /// A timed settle that starts `delay_ms` after the request is taken in: the track holds the value it has until then, and then goes
    /// by `token`. Parts of one screen that arrive one after another are asked for in one commit, each on a track of its own, and the
    /// compositor keeps the stagger on its own clock (a delay the app waited out itself would start when the app's loop got round to
    /// it).
    TimedAfter {
        delay_ms: u32,
        token: MotionToken,
    },
}

/// The longest a timed settle and the delay before it may each be.
const TIMED_MAX_MS: u32 = 60_000;

impl Settle {
    pub fn check(&self) -> Result<(), &'static str> {
        let spring_ok =
            |s: &SpringToken| s.response_ms.is_finite() && (10.0..=5000.0).contains(&s.response_ms);
        match self {
            Settle::Timed(t) if t.duration_ms > TIMED_MAX_MS => {
                Err("a timed settle is at most a minute")
            }
            Settle::Timed(_) => Ok(()),
            Settle::TimedAfter { delay_ms, token }
                if *delay_ms > TIMED_MAX_MS || token.duration_ms > TIMED_MAX_MS =>
            {
                Err("a timed settle and its delay are each at most a minute")
            }
            Settle::TimedAfter { .. } => Ok(()),
            Settle::CriticalSpring(s) if spring_ok(s) => Ok(()),
            Settle::CriticalSpring(_) => Err("a spring's response must be 10 to 5000 ms"),
            Settle::Scroll(s)
                if spring_ok(&s.spring)
                    && s.friction.is_finite()
                    && s.friction > 0.1
                    && s.friction <= 50.0 =>
            {
                Ok(())
            }
            Settle::Scroll(_) => {
                Err("scroll friction must be 0.1 to 50 per second and its spring 10 to 5000 ms")
            }
        }
    }
}

/// What a cancel leaves a running track as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelPolicy {
    /// Stops where it is.
    Hold,
    /// Jumps to where its settle was going.
    Complete,
}

/// What happened to a track that the app may want to reconcile its estimate with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeKind {
    /// A settle began (from the actual value and velocity).
    Started,
    /// The track came to rest, by arriving or by being cancelled.
    Settled,
    /// A finger landed on a settling track and took it over, at the value and velocity it had.
    Caught,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub kind: NoticeKind,
    /// Changes whenever a settle starts or is caught; a `Settled` has that of the settle it ends.
    pub generation: u32,
    pub value: f32,
    pub velocity: f32,
    pub time_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpringPlan {
    from: f32,
    target: f32,
    v0: f32,
    omega: f64,
    start_ns: u64,
    pos_eps: f32,
    vel_eps: f32,
}

impl SpringPlan {
    fn eval(&self, now_ns: u64) -> (f32, f32, bool) {
        let t = now_ns.saturating_sub(self.start_ns) as f64 / 1e9;
        let e = f64::from(self.from) - f64::from(self.target);
        let c = f64::from(self.v0) + self.omega * e;
        let decay = (-self.omega * t).exp();
        let x = f64::from(self.target) + (e + c * t) * decay;
        let v = (c - self.omega * (e + c * t)) * decay;
        let arrived = (x - f64::from(self.target)).abs() <= f64::from(self.pos_eps)
            && v.abs() <= f64::from(self.vel_eps);
        // Long after any trace of it is gone: stop even if the thresholds were set absurdly tight.
        if arrived || t * self.omega > 40.0 {
            (self.target, 0.0, true)
        } else {
            (x as f32, v as f32, false)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum After {
    Stop,
    Spring {
        target: f32,
        v: f32,
        omega: f64,
        pos_eps: f32,
        vel_eps: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FlightPlan {
    x0: f32,
    v0: f32,
    k: f64,
    start_ns: u64,
    end_s: f64,
    after: After,
}

impl FlightPlan {
    fn at(&self, t: f64) -> (f32, f32) {
        let decay = (-self.k * t).exp();
        (
            (f64::from(self.x0) + f64::from(self.v0) * (1.0 - decay) / self.k) as f32,
            (f64::from(self.v0) * decay) as f32,
        )
    }

    fn eval(&self, now_ns: u64) -> (f32, f32, bool) {
        let t = now_ns.saturating_sub(self.start_ns) as f64 / 1e9;
        if t <= self.end_s {
            let (x, v) = self.at(t);
            return (x, v, false);
        }
        let (x_end, _) = self.at(self.end_s);
        match self.after {
            After::Stop => (x_end, 0.0, true),
            After::Spring {
                target,
                v,
                omega,
                pos_eps,
                vel_eps,
            } => SpringPlan {
                from: x_end,
                target,
                v0: v,
                omega,
                start_ns: self.start_ns + (self.end_s * 1e9) as u64,
                pos_eps,
                vel_eps,
            }
            .eval(now_ns),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Plan {
    Timed {
        from: f32,
        to: f32,
        start_ns: u64,
        /// How long after `start_ns` the track begins to move: until then it holds `from` (0 for a plain timed settle).
        hold_ns: u64,
        duration_ns: u64,
        curve: Ease,
    },
    Spring(SpringPlan),
    Flight(FlightPlan),
}

impl Plan {
    fn eval(&self, now_ns: u64) -> (f32, f32, bool) {
        match self {
            Plan::Timed {
                from,
                to,
                start_ns,
                hold_ns,
                duration_ns,
                curve,
            } => {
                let since = now_ns.saturating_sub(*start_ns);
                // Waiting out its delay, a plan holds where it began and is not over, however short it is to be.
                let Some(elapsed) = since.checked_sub(*hold_ns) else {
                    return (*from, 0.0, false);
                };
                if *duration_ns == 0 || elapsed >= *duration_ns {
                    return (*to, 0.0, true);
                }
                let u = (elapsed as f64 / *duration_ns as f64) as f32;
                let x = from + (to - from) * ease(*curve, u);
                // The slope of the curve, by a one-millisecond difference: a timed settle's velocity is only what a
                // catch hands to the next plan.
                let ahead = (elapsed + 1_000_000).min(*duration_ns);
                let u2 = (ahead as f64 / *duration_ns as f64) as f32;
                let x2 = from + (to - from) * ease(*curve, u2);
                let dt = (ahead - elapsed) as f32 / 1e9;
                (x, if dt > 0.0 { (x2 - x) / dt } else { 0.0 }, false)
            }
            Plan::Spring(plan) => plan.eval(now_ns),
            Plan::Flight(plan) => plan.eval(now_ns),
        }
    }

    /// Where the plan ends.
    fn target(&self) -> f32 {
        match self {
            Plan::Timed { to, .. } => *to,
            Plan::Spring(plan) => plan.target,
            Plan::Flight(plan) => match plan.after {
                After::Stop => plan.at(plan.end_s).0,
                After::Spring { target, .. } => target,
            },
        }
    }
}

/// How fast a pointer was going: the least-squares slope, in value per second, of the `samples` (a time in nanoseconds, oldest first,
/// and a value) that lie within `window_ns` before the newest, which is the last one given. Samples dated after the newest do not
/// count (a lift carries the time of the last position, and a position may be predicted a little past the newest report; two samples
/// may share a time). Zero when fewer than two are inside or they span less than a millisecond: a finger that rested before it lifted
/// has nothing to fling with.
///
/// One estimator for every release: the compositor's tracks (the dock, the day line), the toolkit's scroller and its card and page
/// swipes. A swipe starts from rest and speeds up as it goes, so a speed averaged over more than [`VELOCITY_WINDOW_NS`] is well below
/// the one the finger lifts at.
pub fn slope_per_s(samples: impl Iterator<Item = (u64, f32)> + Clone, window_ns: u64) -> f32 {
    let Some((newest, _)) = samples.clone().last() else {
        return 0.0;
    };
    let inside = |s: &(u64, f32)| s.0 <= newest && newest - s.0 <= window_ns;
    let n = samples.clone().filter(inside).count();
    let first = samples.clone().find(inside).map_or(newest, |s| s.0);
    if n < 2 || newest - first < 1_000_000 {
        return 0.0;
    }
    let at = |t: u64| (t as f64 - newest as f64) / 1e9;
    let mean_t = samples.clone().filter(inside).map(|s| at(s.0)).sum::<f64>() / n as f64;
    let mean_v = samples
        .clone()
        .filter(inside)
        .map(|s| f64::from(s.1))
        .sum::<f64>()
        / n as f64;
    let (mut var, mut cov) = (0.0f64, 0.0f64);
    for s in samples.filter(inside) {
        let dt = at(s.0) - mean_t;
        var += dt * dt;
        cov += dt * (f64::from(s.1) - mean_v);
    }
    (cov / var) as f32
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    contact: u32,
    pointer0: f32,
    raw0: f32,
    history: [(u64, f32); HISTORY],
    len: usize,
}

impl Drag {
    fn push(&mut self, t: u64, value: f32) {
        if self.len == HISTORY {
            self.history.copy_within(1.., 0);
            self.len -= 1;
        }
        self.history[self.len] = (t, value);
        self.len += 1;
    }

    /// Value per second (see [`slope_per_s`]): zero for a finger that rested before it lifted, which has nothing to fling with.
    fn velocity(&self) -> f32 {
        slope_per_s(self.history[..self.len].iter().copied(), VELOCITY_WINDOW_NS)
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Rest,
    Drag(Drag),
    /// The finger is off and no settle has been asked for yet: the value stays.
    Released,
    Settle(Plan),
}

/// One motion track. See the module documentation.
#[derive(Clone, Copy, Debug)]
pub struct Track {
    bounds: MotionBounds,
    value: f32,
    velocity: f32,
    generation: u32,
    mode: Mode,
}

impl Track {
    pub fn new(value: f32, bounds: MotionBounds) -> Self {
        Self {
            bounds,
            value: bounds.clamp(value),
            velocity: 0.0,
            generation: 0,
            mode: Mode::Rest,
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn bounds(&self) -> &MotionBounds {
        &self.bounds
    }

    /// Whether a frame is needed at every refresh for this track: a settle is running or a finger is on it (a
    /// finger that is down and still needs none, but it is moving as far as anyone can tell).
    pub fn is_active(&self) -> bool {
        matches!(self.mode, Mode::Settle(_) | Mode::Drag(_))
    }

    pub fn is_settling(&self) -> bool {
        matches!(self.mode, Mode::Settle(_))
    }

    /// Whether the value is anywhere but at rest: a finger on it, a finger just off it with no settle asked for yet, or a
    /// settle running. A cached picture of what the track moves is taken at the value it had before this began, and again
    /// when it is over (`compositor::scene`).
    pub fn is_moving(&self) -> bool {
        !matches!(self.mode, Mode::Rest)
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.mode, Mode::Drag(_))
    }

    /// Brings the value to `now_ns`, ending a settle that has arrived (and saying so).
    pub fn advance(&mut self, now_ns: u64) -> Option<Notice> {
        let Mode::Settle(plan) = self.mode else {
            return None;
        };
        let (value, velocity, done) = plan.eval(now_ns);
        self.value = value;
        self.velocity = velocity;
        if done {
            self.mode = Mode::Rest;
            self.velocity = 0.0;
            return Some(self.notice(NoticeKind::Settled, now_ns));
        }
        None
    }

    fn notice(&self, kind: NoticeKind, time_ns: u64) -> Notice {
        Notice {
            kind,
            generation: self.generation,
            value: self.value,
            velocity: self.velocity,
            time_ns,
        }
    }

    /// Applies a pointer sample at the frame time `now_ns` (the time the sample is taken into the frame; a `Down`
    /// catches the value the screen shows at that time).
    pub fn drive(&mut self, sample: DriveSample, now_ns: u64) -> Option<Notice> {
        match sample.phase {
            DrivePhase::Down => {
                if matches!(self.mode, Mode::Drag(_)) {
                    // A second finger does not take the track from the first.
                    return None;
                }
                let was_settling = self.is_settling();
                let ended = self.advance(now_ns);
                let notice = if ended.is_some() {
                    // It arrived just as the finger landed: nothing was caught.
                    ended
                } else if was_settling {
                    self.generation = self.generation.wrapping_add(1);
                    Some(self.notice(NoticeKind::Caught, now_ns))
                } else {
                    None
                };
                let mut drag = Drag {
                    contact: sample.contact,
                    pointer0: sample.position,
                    raw0: self.bounds.raw_of(self.value),
                    history: [(0, 0.0); HISTORY],
                    len: 0,
                };
                drag.push(sample.time_ns, self.value);
                self.mode = Mode::Drag(drag);
                self.velocity = 0.0;
                notice
            }
            DrivePhase::Move => {
                if let Mode::Drag(drag) = &mut self.mode {
                    if drag.contact != sample.contact {
                        return None;
                    }
                    let raw = drag.raw0 + (sample.position - drag.pointer0) * self.bounds.ratio;
                    self.value = self.bounds.limit(raw);
                    drag.push(sample.time_ns, self.value);
                    self.velocity = drag.velocity();
                }
                None
            }
            DrivePhase::Up => {
                if let Mode::Drag(drag) = &mut self.mode {
                    if drag.contact != sample.contact {
                        return None;
                    }
                    let raw = drag.raw0 + (sample.position - drag.pointer0) * self.bounds.ratio;
                    self.value = self.bounds.limit(raw);
                    drag.push(sample.time_ns, self.value);
                    self.velocity = drag.velocity();
                    self.mode = Mode::Released;
                }
                None
            }
            DrivePhase::Cancel => {
                if let Mode::Drag(drag) = &self.mode {
                    if drag.contact != sample.contact {
                        return None;
                    }
                    self.velocity = 0.0;
                    self.mode = Mode::Rest;
                }
                None
            }
        }
    }

    /// Starts a settle at `now_ns` from the actual value, with `velocity` (the measured one when `None`), toward
    /// `target` (the nearest anchor, or where it is, when `None`; ignored by scroll inertia). Replaces a running
    /// settle and ends a drag.
    pub fn settle(
        &mut self,
        now_ns: u64,
        target: Option<f32>,
        velocity: Option<f32>,
        kind: Settle,
    ) -> Notice {
        self.advance(now_ns);
        let v0 = velocity.unwrap_or(self.velocity);
        let x0 = self.value;
        let (pos_eps, vel_eps) = self.bounds.units.rest();
        let timed = |delay_ms: u32, token: MotionToken| Plan::Timed {
            from: x0,
            to: self.bounds.clamp(target.unwrap_or(x0)),
            start_ns: now_ns,
            hold_ns: u64::from(delay_ms) * 1_000_000,
            duration_ns: u64::from(token.duration_ms) * 1_000_000,
            curve: token.ease,
        };
        let plan = match kind {
            Settle::Timed(token) => timed(0, token),
            Settle::TimedAfter { delay_ms, token } => timed(delay_ms, token),
            Settle::CriticalSpring(token) => {
                let to = target
                    .or_else(|| self.bounds.nearest_anchor(x0))
                    .unwrap_or(x0);
                Plan::Spring(SpringPlan {
                    from: x0,
                    target: self.bounds.clamp(to),
                    v0,
                    omega: token.omega(),
                    start_ns: now_ns,
                    pos_eps,
                    vel_eps,
                })
            }
            Settle::Scroll(token) => self.scroll_plan(x0, v0, now_ns, token),
        };
        self.generation = self.generation.wrapping_add(1);
        self.velocity = v0;
        self.mode = Mode::Settle(plan);
        self.notice(NoticeKind::Started, now_ns)
    }

    fn scroll_plan(&self, x0: f32, v0: f32, now_ns: u64, token: ScrollToken) -> Plan {
        let b = &self.bounds;
        let (pos_eps, vel_eps) = b.units.rest();
        let omega = token.spring.omega();
        let spring = |from: f32, target: f32, v: f32| {
            Plan::Spring(SpringPlan {
                from,
                target,
                v0: v,
                omega,
                start_ns: now_ns,
                pos_eps,
                vel_eps,
            })
        };
        // Past an end (a finger let go in the overscroll): the spring brings it home.
        if x0 < b.min || x0 > b.max {
            return spring(x0, b.clamp(x0), v0);
        }
        let k = f64::from(token.friction);
        if let Some(anchor) = b.nearest_anchor(b.clamp((f64::from(x0) + f64::from(v0) / k) as f32))
        {
            return spring(x0, anchor, v0);
        }
        if v0.abs() <= vel_eps {
            return spring(x0, x0, 0.0);
        }
        let speed = f64::from(v0.abs());
        let room = if v0 > 0.0 { b.max - x0 } else { x0 - b.min };
        let room = f64::from(room.max(0.0));
        if speed / k > room {
            // The flight reaches an end at `t_hit`, with what is left of its speed.
            let t_hit = -(1.0 - room * k / speed).ln() / k;
            let v_hit = (speed - room * k) as f32 * v0.signum();
            let end = if v0 > 0.0 { b.max } else { b.min };
            let after = if b.overscroll > 0.0 {
                // At most `overscroll` of overshoot: a critically damped spring entered at the end with speed v peaks
                // at v / (omega e).
                let cap = (f64::from(b.overscroll) * omega * std::f64::consts::E) as f32;
                After::Spring {
                    target: end,
                    v: v_hit.clamp(-cap, cap),
                    omega,
                    pos_eps,
                    vel_eps,
                }
            } else {
                After::Stop
            };
            return Plan::Flight(FlightPlan {
                x0,
                v0,
                k,
                start_ns: now_ns,
                end_s: t_hit,
                after,
            });
        }
        let stop_s = (speed / f64::from(vel_eps)).ln() / k;
        Plan::Flight(FlightPlan {
            x0,
            v0,
            k,
            start_ns: now_ns,
            end_s: stop_s.max(0.0),
            after: After::Stop,
        })
    }

    /// Stops the track at `now_ns`, where it is or at the end of its settle.
    pub fn cancel(&mut self, now_ns: u64, policy: CancelPolicy) -> Option<Notice> {
        self.advance(now_ns);
        let Mode::Settle(plan) = self.mode else {
            if let Mode::Drag(_) | Mode::Released = self.mode {
                self.mode = Mode::Rest;
                self.velocity = 0.0;
            }
            return None;
        };
        if policy == CancelPolicy::Complete {
            self.value = plan.target();
        }
        self.mode = Mode::Rest;
        self.velocity = 0.0;
        Some(self.notice(NoticeKind::Settled, now_ns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    fn sample(phase: DrivePhase, position: f32, t_ms: u64) -> DriveSample {
        DriveSample {
            contact: 1,
            phase,
            position,
            time_ns: t_ms * MS,
        }
    }

    fn pixels(min: f32, max: f32) -> MotionBounds {
        MotionBounds::range(min, max, Units::Pixels)
    }

    fn spring_token(ms: f32) -> Settle {
        Settle::CriticalSpring(SpringToken { response_ms: ms })
    }

    fn scroll_token() -> Settle {
        Settle::Scroll(ScrollToken {
            friction: 4.0,
            spring: SpringToken { response_ms: 300.0 },
        })
    }

    /// Runs `track` at 59.64 Hz frames from `from_ms` for `frames`, returning every frame's (value, velocity).
    fn run(track: &mut Track, from_ms: f64, frames: usize) -> Vec<(f32, f32)> {
        (1..=frames)
            .map(|n| {
                let now = ((from_ms + n as f64 * 16.7673) * 1e6) as u64;
                track.advance(now);
                (track.value(), track.velocity())
            })
            .collect()
    }

    /// What a browser's CSS engine gives for the two curves of Lucent's tokens, at 21 times each (the progress after `t` of a
    /// 1 s transition of `translateX(0 -> 3e7 px)`, read back from the computed transform: six significant digits). Chromium
    /// 154.0.8037.92 (Web Animations API, `easing: 'cubic-bezier(...)'`), 2026-10-08. The reference of "within 0.5 %": the
    /// curves here are held to its rounding.
    const CSS_OUT: [(f32, f32); 21] = [
        (0.00, 0.0),
        (0.05, 0.143593),
        (0.10, 0.272092),
        (0.15, 0.386563),
        (0.20, 0.488),
        (0.25, 0.577343),
        (0.30, 0.655473),
        (0.35, 0.723237),
        (0.40, 0.781433),
        (0.45, 0.83084),
        (0.50, 0.872203),
        (0.55, 0.906247),
        (0.60, 0.933687),
        (0.65, 0.955227),
        (0.70, 0.97156),
        (0.75, 0.983383),
        (0.80, 0.991403),
        (0.85, 0.996333),
        (0.90, 0.9989),
        (0.95, 0.99986),
        (1.00, 1.0),
    ];
    const CSS_EXIT: [(f32, f32); 21] = [
        (0.00, 0.0),
        (0.05, 0.004876),
        (0.10, 0.018373),
        (0.15, 0.039137),
        (0.20, 0.066151),
        (0.25, 0.098627),
        (0.30, 0.13594),
        (0.35, 0.177589),
        (0.40, 0.223167),
        (0.45, 0.272335),
        (0.50, 0.324815),
        (0.55, 0.380373),
        (0.60, 0.43882),
        (0.65, 0.5),
        (0.70, 0.563787),
        (0.75, 0.630087),
        (0.80, 0.698843),
        (0.85, 0.770047),
        (0.90, 0.84375),
        (0.95, 0.920147),
        (1.00, 1.0),
    ];

    #[test]
    fn the_two_curves_are_lucents_css_curves_to_the_rounding_of_a_browsers_own_values() {
        for (curve, css, css_text) in [
            (Ease::Out, CSS_OUT, "cubic-bezier(.33,1,.68,1)"),
            (Ease::In, CSS_EXIT, "cubic-bezier(.4,0,1,1)"),
        ] {
            for (t, expected) in css {
                let got = ease(curve, t);
                // Held to the browser's six digits, a hundred times closer than the 0.5 % of the brief.
                assert!(
                    (got - expected).abs() < 2e-6,
                    "{curve:?} at {t} is {got}; {css_text} gives {expected}"
                );
                assert!((got - expected).abs() < 0.005);
            }
        }
    }

    #[test]
    fn the_out_curve_is_the_css_curve_and_not_the_cubic_it_resembles() {
        // `1 - (1 - t)^3` is within 0.28 % of it, which is the tight approximation a compositor could have had; the curve of the
        // tokens is evaluated itself, so this tells the two apart.
        let worst = (0..=1000)
            .map(|i| {
                let t = i as f32 / 1000.0;
                (ease(Ease::Out, t) - (1.0 - (1.0 - t).powi(3))).abs()
            })
            .fold(0.0f32, f32::max);
        assert!((0.002..0.0035).contains(&worst), "{worst}");
    }

    #[test]
    fn every_curve_starts_and_lands_exactly_and_never_runs_backwards() {
        for curve in [Ease::Linear, Ease::Out, Ease::In, Ease::Spring] {
            assert_eq!(ease(curve, 0.0), 0.0);
            assert_eq!(ease(curve, 1.0), 1.0);
            let mut previous = 0.0;
            for i in 1..=1000 {
                let v = ease(curve, i as f32 / 1000.0);
                assert!(v >= previous, "{curve:?} steps back at {i}");
                assert!((0.0..=1.0).contains(&v));
                previous = v;
            }
            // Progress outside the animation is clamped to its ends.
            assert_eq!(ease(curve, -3.0), 0.0);
            assert_eq!(ease(curve, 7.0), 1.0);
        }
        assert_eq!(ease(Ease::Linear, 0.25), 0.25);
    }

    #[test]
    fn out_starts_fast_and_lands_gently_and_in_starts_gently_and_leaves_fast() {
        let slope = |curve: Ease, t: f32| (ease(curve, t + 1e-3) - ease(curve, t - 1e-3)) / 2e-3;
        // Out: three times the mean speed to begin with, at rest on arrival.
        assert!((slope(Ease::Out, 0.002) - 3.03).abs() < 0.05);
        assert!(slope(Ease::Out, 0.998) < 0.02);
        // In: from rest, and still speeding up when it is gone.
        assert!(slope(Ease::In, 0.002) < 0.02);
        assert!((slope(Ease::In, 0.998) - 5.0 / 3.0).abs() < 0.05);
        // A little over three times for Out: `y1 / x1` of the token, the number a hand-off to a glide is as long as.
        assert!((start_slope(Ease::Out) - 1.0 / 0.33).abs() < 1e-6);
        assert_eq!(start_slope(Ease::In), 0.0);
        assert_eq!(start_slope(Ease::Linear), 1.0);
    }

    #[test]
    fn the_spring_curve_is_the_response_260_spring_from_rest_kept_to_300_ms() {
        // x(t) = 1 - (1 + wt) e^(-wt) is what a critically damped spring of response 260 ms does from a pull of 1, from rest; the curve
        // is that over 300 ms, stretched by the 0.6 % that is left of it then so that it lands on time.
        let omega = std::f64::consts::TAU / 0.26;
        assert!(((omega * 0.3) - SPRING_K).abs() < 0.01, "{}", omega * 0.3);
        for ms in [0u32, 10, 25, 50, 100, 150, 200, 250, 290, 299, 300] {
            let t = f64::from(ms) / 1000.0;
            let spring = 1.0 - (1.0 + omega * t) * (-omega * t).exp();
            let got = f64::from(ease(Ease::Spring, ms as f32 / 300.0));
            assert!(
                (got - spring).abs() < 0.006,
                "{ms} ms: {got} against the spring's {spring}"
            );
        }
        // Starts at rest and goes slowly at first (a release is a pull back, not a throw) and lands almost at rest.
        let slope = |t: f32| (ease(Ease::Spring, t + 1e-3) - ease(Ease::Spring, t - 1e-3)) / 2e-3;
        assert!(slope(0.002) < 0.15, "{}", slope(0.002));
        assert!(slope(0.998) < 0.06, "{}", slope(0.998));
        // More than half of the way is done by a quarter of the time, and nearly nine tenths by half of it; it never turns back.
        assert!(ease(Ease::Spring, 0.25) > 0.5 && ease(Ease::Spring, 0.5) > 0.85);
        assert!((1..=1000).all(|i| ease(Ease::Spring, i as f32 / 1000.0)
            >= ease(Ease::Spring, (i - 1) as f32 / 1000.0)));
    }

    #[test]
    fn the_solver_finds_the_time_of_a_curve_whose_slope_is_nearly_flat_in_its_middle() {
        // x'(s) of these control points is 0.075 at its least, in the middle of the curve: Newton from the time itself is slow to
        // get there and the bisection behind it lands where the curve's own parametric form says, for every time.
        let flat = CubicBezier::new(0.95, 0.0, 0.05, 1.0);
        for i in 1..200 {
            let t = f64::from(i) / 200.0;
            let s = flat.parameter_at(t);
            assert!(
                (flat.x_at(s) - t).abs() < 1e-8,
                "x(s) is {} at {t}",
                flat.x_at(s)
            );
            assert!((flat.at(t) - flat.y_at(s)).abs() < 1e-12);
        }
    }

    #[test]
    fn the_rubber_band_follows_35_percent_at_the_end_gives_less_with_every_pixel_and_is_capped() {
        let cap = 32.0;
        // Follows RUBBER_FOLLOW (35 %) at the edge itself.
        let eps = 0.01;
        assert!((rubber(eps, cap) / eps - RUBBER_FOLLOW).abs() < 1e-3);
        assert_eq!(RUBBER_FOLLOW, 0.35);
        // Gives less with every pixel more, and stays under the cap.
        let mut last = 0.0;
        let mut last_step = f32::MAX;
        for r in 1..=600 {
            let shown = rubber(r as f32, cap);
            assert!(shown > last && shown < cap, "{r}: {shown}");
            let step = shown - last;
            assert!(step <= last_step + 1e-6, "{r}: it gives less and less");
            last = shown;
            last_step = step;
        }
        // A finger that goes on for ever gets the cap and no more.
        assert!(rubber(5000.0, cap) <= cap && rubber(5000.0, cap) > cap * 0.9999);
        // A pull of a hundred pixels shows under 35 % of it: only the first pixels follow at 35 %, and each one after gives less.
        assert!(rubber(100.0, cap) < 0.35 * 100.0);
        // Nothing past the end, or nowhere to go: nothing shown.
        assert_eq!(rubber(0.0, cap), 0.0);
        assert_eq!(rubber(-5.0, cap), 0.0);
        assert_eq!(rubber(50.0, 0.0), 0.0);
    }

    #[test]
    fn the_rubber_band_can_be_taken_back_to_the_pull_that_shows_a_value() {
        for cap in [16.0, 32.0, 46.0] {
            for pull in [0.5, 3.0, 20.0, 90.0, 250.0] {
                let shown = rubber(pull, cap);
                assert!(
                    (unrubber(shown, cap) - pull).abs() <= pull * 1e-3 + 1e-3,
                    "cap {cap}, pull {pull}: shown {shown}, taken back {}",
                    unrubber(shown, cap)
                );
            }
            assert_eq!(unrubber(0.0, cap), 0.0);
            assert!(
                unrubber(cap, cap).is_finite(),
                "the cap itself reads as just under it"
            );
        }
        // A track's resistance is this rubber band with its own cap.
        let bounds = MotionBounds {
            overscroll: 32.0,
            ..pixels(0.0, 100.0)
        };
        assert_eq!(bounds.resist(40.0), rubber(40.0, 32.0));
        assert_eq!(bounds.unresist(10.0), unrubber(10.0, 32.0));
    }

    #[test]
    fn a_drag_follows_the_pointer_by_the_ratio_from_where_the_finger_landed() {
        let mut t = Track::new(
            100.0,
            MotionBounds {
                ratio: -1.0,
                ..pixels(0.0, 1000.0)
            },
        );
        t.drive(sample(DrivePhase::Down, 400.0, 0), 0);
        // The value does not jump on the landing; later moves are deltas from the landing.
        assert_eq!(t.value(), 100.0);
        t.drive(sample(DrivePhase::Move, 350.0, 16), 16 * MS);
        assert_eq!(t.value(), 150.0);
        t.drive(sample(DrivePhase::Move, 450.0, 32), 32 * MS);
        assert_eq!(t.value(), 50.0);
    }

    #[test]
    fn only_the_first_contact_drives_and_a_cancelled_drag_does_not_fling() {
        let mut t = Track::new(0.0, pixels(0.0, 1000.0));
        t.drive(sample(DrivePhase::Down, 0.0, 0), 0);
        let other = DriveSample {
            contact: 2,
            ..sample(DrivePhase::Move, 500.0, 10)
        };
        t.drive(other, 10 * MS);
        assert_eq!(t.value(), 0.0);
        t.drive(sample(DrivePhase::Move, 80.0, 10), 10 * MS);
        t.drive(sample(DrivePhase::Move, 160.0, 20), 20 * MS);
        assert!(t.velocity() > 1000.0);
        t.drive(sample(DrivePhase::Cancel, 160.0, 25), 25 * MS);
        assert_eq!(t.velocity(), 0.0);
        assert!(!t.is_active());
        assert_eq!(t.value(), 160.0, "it stays where the finger left it");
    }

    #[test]
    fn velocity_is_measured_over_the_last_fifty_milliseconds_and_a_pause_before_release_is_zero() {
        let mut t = Track::new(0.0, pixels(-10_000.0, 10_000.0));
        t.drive(sample(DrivePhase::Down, 0.0, 0), 0);
        for k in 1..=10u64 {
            t.drive(
                sample(DrivePhase::Move, k as f32 * 20.0, k * 16),
                k * 16 * MS,
            );
        }
        // 20 px every 16 ms.
        assert!((t.velocity() - 1250.0).abs() < 5.0, "{}", t.velocity());
        // The finger rests for 250 ms and lifts: nothing is left to fling with.
        t.drive(sample(DrivePhase::Up, 200.0, 410), 410 * MS);
        assert_eq!(t.velocity(), 0.0);
    }

    #[test]
    fn a_drag_that_has_just_got_up_to_speed_leaves_at_that_speed_and_not_at_its_average_since_the_landing()
     {
        // The shape of the swipes made on the Show: the finger rests 20 ms, speeds up to 3 px/ms over 30 ms and goes on at that;
        // samples a refresh apart. Its speed averaged from the landing is 1.95 px/ms (65 % of what it lifts at).
        let travelled = |t_ms: f32| {
            let ramp = (t_ms - 20.0).clamp(0.0, 30.0);
            0.5 * (3.0 / 30.0) * ramp * ramp + 3.0 * (t_ms - 50.0).max(0.0)
        };
        let mut t = Track::new(0.0, pixels(-100_000.0, 100_000.0));
        t.drive(sample(DrivePhase::Down, 0.0, 0), 0);
        for ms in [17u64, 34, 50, 67, 84, 100] {
            t.drive(sample(DrivePhase::Move, travelled(ms as f32), ms), ms * MS);
        }
        t.drive(sample(DrivePhase::Up, travelled(100.0), 100), 100 * MS);
        assert!(
            (t.velocity() / 3000.0 - 1.0).abs() < 0.05,
            "released at {} px/s; the finger lifted at 3000",
            t.velocity()
        );
    }

    #[test]
    fn the_slope_is_that_of_the_last_window_and_a_single_sample_or_a_rest_has_none() {
        // 5 units a millisecond for 100 ms: 5000 a second, whatever the samples' spacing.
        let line = |ms: u64| (ms * MS, 5.0 * ms as f32);
        let v = slope_per_s(
            [0u64, 16, 33, 50, 66, 83, 100].map(line).into_iter(),
            VELOCITY_WINDOW_NS,
        );
        assert!((v - 5000.0).abs() < 1.0, "{v}");
        // A slow stretch before the window does not count: 1 a millisecond to 50 ms, then 5.
        let bent = |ms: u64| {
            (
                ms * MS,
                if ms <= 50 {
                    ms as f32
                } else {
                    50.0 + 5.0 * (ms - 50) as f32
                },
            )
        };
        let v = slope_per_s(
            [0u64, 25, 50, 67, 83, 100].map(bent).into_iter(),
            VELOCITY_WINDOW_NS,
        );
        assert!((v - 5000.0).abs() < 1.0, "{v}");
        // A lift shares the time of the last position; a sample dated after the newest is left out.
        let v = slope_per_s(
            [
                (0u64, 0.0f32),
                (20 * MS, 100.0),
                (40 * MS, 200.0),
                (40 * MS, 200.0),
            ]
            .into_iter(),
            VELOCITY_WINDOW_NS,
        );
        assert!((v - 5000.0).abs() < 1.0, "{v}");
        // Nothing to measure: one sample, none, samples inside a millisecond, a finger that rested before it lifted.
        assert_eq!(
            slope_per_s([(0u64, 1.0f32)].into_iter(), VELOCITY_WINDOW_NS),
            0.0
        );
        assert_eq!(slope_per_s(std::iter::empty(), VELOCITY_WINDOW_NS), 0.0);
        assert_eq!(
            slope_per_s(
                [(10 * MS, 0.0f32), (10 * MS + 500_000, 3.0)].into_iter(),
                VELOCITY_WINDOW_NS
            ),
            0.0
        );
        assert_eq!(
            slope_per_s(
                [(0u64, 0.0f32), (10 * MS, 50.0), (400 * MS, 50.0)].into_iter(),
                VELOCITY_WINDOW_NS
            ),
            0.0
        );
    }

    #[test]
    fn a_critical_spring_arrives_without_crossing_and_lands_exactly() {
        let mut t = Track::new(0.0, pixels(0.0, 1000.0));
        let started = t.settle(0, Some(300.0), Some(0.0), spring_token(300.0));
        assert_eq!(started.kind, NoticeKind::Started);
        let frames = run(&mut t, 0.0, 120);
        assert!(
            frames.iter().all(|(v, _)| *v <= 300.0),
            "no overshoot from rest"
        );
        assert!(frames.windows(2).all(|w| w[1].0 >= w[0].0), "monotone");
        assert_eq!(t.value(), 300.0, "lands exactly");
        assert!(!t.is_active());
        // The settle's end is announced once.
        let mut again = Track::new(0.0, pixels(0.0, 1000.0));
        again.settle(0, Some(10.0), Some(0.0), spring_token(100.0));
        let done = again.advance(5_000 * MS).expect("it ended");
        assert_eq!((done.kind, done.value), (NoticeKind::Settled, 10.0));
        assert!(again.advance(6_000 * MS).is_none());
    }

    #[test]
    fn a_settle_is_a_function_of_time_alone_and_its_frame_steps_follow_its_speed() {
        // Frames at jittery times land where on-time ones would have: nothing is integrated, so reaching a time in one
        // jump gives the value reached by stepping, and no step is more than the speed it passes times the frame.
        let mut a = Track::new(0.0, pixels(0.0, 800.0));
        a.settle(0, Some(500.0), Some(1500.0), spring_token(260.0));
        let mut b = a;
        let mut last = a.value();
        let mut worst_excess = 0.0f32;
        let mut peak = 0.0f32;
        for n in 1..=90u64 {
            let now = n * 16_767_300;
            a.advance(now);
            peak = peak.max(a.velocity().abs());
            let step = (a.value() - last).abs();
            worst_excess = worst_excess.max(step - peak * 0.016_767_3 * 1.05);
            last = a.value();
        }
        assert!(worst_excess <= 0.5, "{worst_excess}");
        b.advance(90 * 16_767_300);
        assert_eq!(b.value(), a.value());
        // And a frame that is two refreshes late shows where the plan is then, not a step of the two summed.
        let mut c = Track::new(0.0, pixels(0.0, 800.0));
        c.settle(0, Some(500.0), Some(1500.0), spring_token(260.0));
        c.advance(2 * 16_767_300);
        let mut d = Track::new(0.0, pixels(0.0, 800.0));
        d.settle(0, Some(500.0), Some(1500.0), spring_token(260.0));
        d.advance(16_767_300);
        d.advance(2 * 16_767_300);
        assert_eq!(c.value(), d.value());
    }

    #[test]
    fn catching_a_settle_keeps_the_value_and_a_new_settle_continues_from_it() {
        let mut t = Track::new(0.0, pixels(0.0, 1000.0));
        t.settle(0, Some(600.0), Some(0.0), spring_token(400.0));
        let at = 120 * MS;
        t.advance(at);
        let before = t.value();
        let caught = t
            .drive(sample(DrivePhase::Down, 50.0, 120), at)
            .expect("a finger on a settling track catches it");
        assert_eq!(caught.kind, NoticeKind::Caught);
        assert_eq!(caught.value, before);
        assert_eq!(t.value(), before, "no jump on the catch");
        assert!(t.is_dragging() && !t.is_settling());
        // The reversal: the finger drags back, lets go and the new settle starts from what is on screen.
        t.drive(sample(DrivePhase::Move, 20.0, 136), 136 * MS);
        let during = t.value();
        assert!((during - (before - 30.0)).abs() < 1e-3);
        t.drive(sample(DrivePhase::Up, 20.0, 150), 150 * MS);
        let started = t.settle(150 * MS, Some(0.0), None, spring_token(300.0));
        assert_eq!(started.value, t.value());
        assert!(started.generation > caught.generation);
        let frames = run(&mut t, 150.0, 120);
        assert_eq!(t.value(), 0.0);
        assert!(frames.iter().all(|(v, _)| *v >= -1.0 && *v <= 600.0));
    }

    #[test]
    fn the_catch_and_the_next_settle_have_no_gap_in_value() {
        // 40 catch/reverse cycles at random-ish phases: the first value after each is the last before it.
        let mut t = Track::new(0.0, pixels(0.0, 1000.0));
        let mut now = 0u64;
        let mut target = 900.0;
        for cycle in 0..40u64 {
            t.settle(now, Some(target), Some(0.0), spring_token(350.0));
            now += (30 + cycle * 7 % 190) * MS;
            t.advance(now);
            let on_screen = t.value();
            t.drive(sample(DrivePhase::Down, 0.0, now / MS), now);
            assert_eq!(t.value(), on_screen);
            t.drive(sample(DrivePhase::Up, 0.0, now / MS + 1), now + MS);
            let s = t.settle(now + MS, Some(900.0 - target), None, spring_token(350.0));
            assert!((s.value - on_screen).abs() < 1e-3);
            target = 900.0 - target;
            now += MS;
        }
    }

    #[test]
    fn scroll_inertia_decays_to_a_stop_and_matches_its_closed_form() {
        let mut t = Track::new(0.0, pixels(0.0, 100_000.0));
        t.settle(0, None, Some(2000.0), scroll_token());
        // v(t) = v0 e^{-kt}; x(t) = v0 (1 - e^{-kt}) / k.
        t.advance(500 * MS);
        let expect = 2000.0 * (1.0 - (-2.0f32).exp()) / 4.0;
        assert!(
            (t.value() - expect).abs() < 0.05,
            "{} vs {expect}",
            t.value()
        );
        assert!((t.velocity() - 2000.0 * (-2.0f32).exp()).abs() < 0.5);
        t.advance(60_000 * MS);
        assert!(!t.is_active());
        assert!(
            (t.value() - 500.0).abs() < 1.0,
            "comes to rest near v0/k = 500: {}",
            t.value()
        );
    }

    #[test]
    fn scroll_stops_hard_at_a_bound_without_overscroll() {
        let mut t = Track::new(900.0, pixels(0.0, 1000.0));
        t.settle(0, None, Some(3000.0), scroll_token());
        let frames = run(&mut t, 0.0, 200);
        assert!(frames.iter().all(|(v, _)| *v <= 1000.0));
        assert_eq!(t.value(), 1000.0);
        assert!(!t.is_active());
    }

    #[test]
    fn scroll_into_a_permitted_overscroll_goes_no_further_than_it_and_comes_back() {
        let mut t = Track::new(
            900.0,
            MotionBounds {
                overscroll: 60.0,
                ..pixels(0.0, 1000.0)
            },
        );
        t.settle(0, None, Some(8000.0), scroll_token());
        let frames = run(&mut t, 0.0, 240);
        let peak = frames.iter().map(|(v, _)| *v).fold(f32::MIN, f32::max);
        assert!(peak > 1000.0, "it overshoots");
        assert!(peak <= 1060.0 + 0.5, "never past the overscroll: {peak}");
        assert_eq!(t.value(), 1000.0, "and settles on the bound");
        assert!(!t.is_active());
    }

    #[test]
    fn scroll_snaps_to_the_anchor_nearest_where_the_fling_would_end() {
        let anchors = [0.0, 480.0, 960.0, 1440.0, 0.0, 0.0, 0.0, 0.0];
        let mut t = Track::new(
            100.0,
            MotionBounds {
                anchors,
                anchor_count: 4,
                ..pixels(0.0, 1440.0)
            },
        );
        // 100 + 1200/4 = 400: nearest is 480.
        t.settle(0, None, Some(1200.0), scroll_token());
        run(&mut t, 0.0, 200);
        assert_eq!(t.value(), 480.0);
        // A slow release goes back to where it came from.
        let mut u = Track::new(
            100.0,
            MotionBounds {
                anchors,
                anchor_count: 4,
                ..pixels(0.0, 1440.0)
            },
        );
        u.settle(0, None, Some(40.0), scroll_token());
        run(&mut u, 0.0, 200);
        assert_eq!(u.value(), 0.0);
    }

    #[test]
    fn a_drag_past_the_end_resists_and_a_catch_inside_the_resistance_does_not_jump() {
        let b = MotionBounds {
            overscroll: 80.0,
            ..pixels(0.0, 500.0)
        };
        let mut t = Track::new(500.0, b);
        t.drive(sample(DrivePhase::Down, 400.0, 0), 0);
        t.drive(sample(DrivePhase::Move, 500.0, 16), 16 * MS);
        let shown = t.value();
        assert!(
            shown > 500.0 && shown < 580.0,
            "pulled 100 px, shows less than 80 more: {shown}"
        );
        t.drive(sample(DrivePhase::Move, 4000.0, 32), 32 * MS);
        assert!(
            t.value() <= 580.0,
            "it can never go past the limit (at f32 it meets it once the pull is some tens of times the cap)"
        );
        // Let go, spring back, then land a finger partway home: no jump on the first move.
        t.drive(sample(DrivePhase::Up, 4000.0, 33), 33 * MS);
        t.settle(33 * MS, None, Some(0.0), scroll_token());
        t.advance(100 * MS);
        let partway = t.value();
        assert!(partway > 500.0);
        t.drive(sample(DrivePhase::Down, 200.0, 100), 100 * MS);
        t.drive(sample(DrivePhase::Move, 200.0, 116), 116 * MS);
        assert!(
            (t.value() - partway).abs() < 0.01,
            "{} vs {partway}",
            t.value()
        );
    }

    #[test]
    fn a_timed_settle_lands_exactly_and_hands_on_its_velocity() {
        let mut t = Track::new(0.0, MotionBounds::range(0.0, 1.0, Units::Progress));
        t.settle(
            0,
            Some(1.0),
            Some(0.0),
            Settle::Timed(MotionToken {
                duration_ms: 200,
                ease: Ease::Out,
            }),
        );
        t.advance(50 * MS);
        assert!(t.value() > 0.0 && t.value() < 1.0 && t.velocity() > 0.0);
        let done = t.advance(200 * MS).expect("done on time");
        assert_eq!((done.kind, done.value), (NoticeKind::Settled, 1.0));
    }

    /// The track of one part of an entrance: it holds for the delay, then goes exactly as a plain timed settle that began at the
    /// end of the delay would, and lands on the time.
    #[test]
    fn a_timed_settle_after_a_delay_holds_then_goes_as_a_plain_one_would_from_the_end_of_it() {
        let token = MotionToken {
            duration_ms: 280,
            ease: Ease::Out,
        };
        let delayed = Settle::TimedAfter {
            delay_ms: 130,
            token,
        };
        assert!(delayed.check().is_ok());
        let range = MotionBounds::range(0.0, 1.0, Units::Progress);
        let mut waiting = Track::new(0.0, range);
        waiting.settle(40 * MS, Some(1.0), Some(0.0), delayed);
        let mut plain = Track::new(0.0, range);
        plain.settle(170 * MS, Some(1.0), Some(0.0), Settle::Timed(token));
        // It starts at 40 ms and waits until 170: nothing moves, nothing is over, and nothing is said of speed.
        for at in [40, 41, 100, 169] {
            assert_eq!(waiting.advance(at * MS), None, "{at} ms");
            assert_eq!(
                (waiting.value(), waiting.velocity(), waiting.is_settling()),
                (0.0, 0.0, true),
                "{at} ms"
            );
        }
        // From 170 ms on it is the plain settle, at every time until it lands, 280 ms later.
        for at in [170, 171, 200, 310, 449] {
            waiting.advance(at * MS);
            plain.advance(at * MS);
            assert_eq!(waiting.value(), plain.value(), "{at} ms");
        }
        assert!(waiting.value() > 0.9 && waiting.value() < 1.0);
        assert_eq!(waiting.advance(449 * MS), None);
        let done = waiting
            .advance(450 * MS)
            .expect("it lands 280 ms after it began to move");
        assert_eq!((done.kind, done.value), (NoticeKind::Settled, 1.0));
    }

    /// A settle that is not to move for a while and then jumps is not over before the while is.
    #[test]
    fn a_delayed_settle_of_no_length_lands_when_its_delay_is_over_and_not_before() {
        let mut t = Track::new(0.0, MotionBounds::range(0.0, 1.0, Units::Progress));
        t.settle(
            0,
            Some(1.0),
            Some(0.0),
            Settle::TimedAfter {
                delay_ms: 50,
                token: MotionToken {
                    duration_ms: 0,
                    ease: Ease::Linear,
                },
            },
        );
        assert_eq!(t.advance(49 * MS), None);
        assert_eq!(t.value(), 0.0);
        assert_eq!(t.advance(50 * MS).map(|n| n.value), Some(1.0));
    }

    #[test]
    fn a_delay_and_a_duration_are_each_at_most_a_minute() {
        let after = |delay_ms, duration_ms| Settle::TimedAfter {
            delay_ms,
            token: MotionToken {
                duration_ms,
                ease: Ease::Out,
            },
        };
        assert!(after(60_000, 60_000).check().is_ok());
        assert!(after(60_001, 10).check().is_err());
        assert!(after(10, 60_001).check().is_err());
    }

    #[test]
    fn cancel_holds_or_completes() {
        let mut t = Track::new(0.0, pixels(0.0, 100.0));
        t.settle(0, Some(100.0), Some(0.0), spring_token(300.0));
        let held = t
            .cancel(100 * MS, CancelPolicy::Hold)
            .expect("a running settle ends");
        assert_eq!(held.kind, NoticeKind::Settled);
        assert!(held.value > 0.0 && held.value < 100.0);
        assert!(!t.is_active());
        t.settle(100 * MS, Some(100.0), Some(0.0), spring_token(300.0));
        let done = t.cancel(120 * MS, CancelPolicy::Complete).unwrap();
        assert_eq!(done.value, 100.0);
        assert!(t.cancel(130 * MS, CancelPolicy::Hold).is_none());
    }

    #[test]
    fn bounds_are_validated() {
        assert!(pixels(0.0, 1.0).check().is_ok());
        assert!(pixels(2.0, 1.0).check().is_err());
        assert!(pixels(0.0, f32::NAN).check().is_err());
        assert!(
            MotionBounds {
                ratio: 0.0,
                ..pixels(0.0, 1.0)
            }
            .check()
            .is_err()
        );
        assert!(
            MotionBounds {
                overscroll: -1.0,
                ..pixels(0.0, 1.0)
            }
            .check()
            .is_err()
        );
        let anchors = [5.0; MAX_ANCHORS];
        assert!(
            MotionBounds {
                anchors,
                anchor_count: 1,
                ..pixels(0.0, 1.0)
            }
            .check()
            .is_err()
        );
        assert!(spring_token(5.0).check().is_err());
        assert!(spring_token(300.0).check().is_ok());
        assert!(
            Settle::Scroll(ScrollToken {
                friction: 0.0,
                spring: SpringToken { response_ms: 300.0 }
            })
            .check()
            .is_err()
        );
    }
}
