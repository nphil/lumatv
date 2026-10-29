//! Shared scalar motion tracks, as the app holds them: public handles and the policy for estimating what the screen shows.
//!
//! A [`Motion`] is one number with bounds (a dock's progress, a list's scroll offset) that lives in the compositor, which
//! drives it from the finger's samples, catches it when a finger lands on it while it settles, and settles it by closed forms
//! of the time since they began (`compositor_protocol::motion`). The app never hears from a track every frame. What it needs
//! of the value in between (which card is under the finger, which rows to rasterise next) it estimates with **the same
//! equations**: every `Motion` mirrors its track with a local `motion::Track`, fed with the same samples and the same settles,
//! at the time the screen will show them: the monotonic clock plus a *lead*, the measured delay from a commit to the frame
//! that shows it ([`Scene::present_lead_ns`], [`DEFAULT_PRESENT_LEAD_NS`] before the first frame has said).
//!
//! The estimate is reconciled, never trusted over the compositor: `Scene::observe` on `Event::Motion` replaces the mirror's
//! plan by one started from the value, velocity and time the compositor reports, with the settle parameters the app asked for
//! (`Started`), re-seats a catch on the value the compositor caught (`Caught`), and brings a settle's end to its value
//! (`Settled`). An event that carries an older generation than the mirror has reached is a late one and is ignored. None of
//! this sends anything: the image on the screen is the compositor's and never moves because an estimate was corrected.

use crate::error::{Kind, Limit, UiError};
use crate::handle::handle;
use crate::scene::{Inner, Scene};
use crate::writer::Urgent;
use compositor_protocol as wire;
use compositor_protocol::Request;
pub use compositor_protocol::motion::{MAX_ANCHORS, MAX_MOTIONS, Notice, VELOCITY_WINDOW_NS, ease};
use compositor_protocol::motion::{NoticeKind, Track};
pub use compositor_protocol::{
    CancelPolicy, DrivePhase, DriveSample, MotionBounds, MotionToken, ScrollToken, Settle,
    SpringToken, Units,
};

const _: () = assert!(MAX_MOTIONS == crate::limits::MAX_MOTION_TRACKS);

handle!(
    /// Names a motion track in this process: an index and a generation, never reused. The compositor's id for it is
    /// [`Motion::wire_id`].
    MotionId
);

/// How long after a request is made the screen shows its result, before any frame has said otherwise: one refresh at the
/// Echo's 59.64 Hz (the fixed offset the hand-written scroll of the first lab used for the start of a settle). Estimates add
/// this to the clock until [`Scene::observe`] has measured it on real frames.
pub const DEFAULT_PRESENT_LEAD_NS: u64 = 17_000_000;
/// A measured lead outside these is a stalled frame or a clock jump, not the pipeline.
const LEAD_RANGE_NS: (u64, u64) = (2_000_000, 100_000_000);
/// Commits remembered while their frames are awaited.
const COMMIT_LOG: usize = 16;

/// The measured delay from a commit to the frame that shows it: a running average (an eighth of each new difference) of
/// `presented_ns` minus the time the commit was taken, starting from [`DEFAULT_PRESENT_LEAD_NS`].
#[derive(Debug)]
pub(crate) struct Lead {
    ns: u64,
    measured: u32,
    log: [(u64, u64); COMMIT_LOG],
    len: usize,
}

impl Default for Lead {
    fn default() -> Self {
        Self {
            ns: DEFAULT_PRESENT_LEAD_NS,
            measured: 0,
            log: [(0, 0); COMMIT_LOG],
            len: 0,
        }
    }
}

impl Lead {
    /// A lead of `ns` to start from (see [`Scene::set_clock`]).
    pub(crate) fn fixed(ns: u64) -> Self {
        Self {
            ns,
            ..Self::default()
        }
    }

    pub fn ns(&self) -> u64 {
        self.ns
    }

    #[cfg(test)]
    pub fn measured(&self) -> u32 {
        self.measured
    }

    /// Commit `tag` was taken by the writer at `at_ns`.
    pub fn committed(&mut self, tag: u64, at_ns: u64) {
        if self.len == COMMIT_LOG {
            self.log.copy_within(1.., 0);
            self.len -= 1;
        }
        self.log[self.len] = (tag, at_ns);
        self.len += 1;
    }

    /// The frame that shows commit `tag` was on the screen at `presented_ns`.
    pub fn presented(&mut self, tag: u64, presented_ns: u64) {
        let Some(at) = self.log[..self.len].iter().position(|(t, _)| *t == tag) else {
            return;
        };
        let committed_ns = self.log[at].1;
        // This commit and every one before it have had their frame.
        self.log.copy_within(at + 1..self.len, 0);
        self.len -= at + 1;
        let Some(sample) = presented_ns.checked_sub(committed_ns) else {
            return;
        };
        if !(LEAD_RANGE_NS.0..=LEAD_RANGE_NS.1).contains(&sample) {
            return;
        }
        self.ns = if self.measured == 0 {
            sample
        } else {
            (self.ns as i64 + (sample as i64 - self.ns as i64) / 8) as u64
        };
        self.measured += 1;
    }
}

/// The settle the app last asked for, to start the mirror's plan again from the compositor's start of it.
#[derive(Clone, Copy, Debug)]
struct SettleRequest {
    target: Option<f32>,
    kind: Settle,
}

/// The samples of the drag in progress, the landing first: what a catch re-seats the mirror with. A track keeps eight
/// samples for its velocity; so does this, the landing and the seven newest.
#[derive(Clone, Copy)]
struct Replay {
    samples: [DriveSample; Replay::KEPT],
    len: usize,
}

impl Replay {
    const KEPT: usize = 8;

    const EMPTY: DriveSample = DriveSample {
        contact: 0,
        phase: DrivePhase::Cancel,
        position: 0.0,
        time_ns: 0,
    };

    fn new() -> Self {
        Self {
            samples: [Self::EMPTY; Self::KEPT],
            len: 0,
        }
    }

    fn begin(&mut self, down: DriveSample) {
        self.samples[0] = down;
        self.len = 1;
    }

    fn push(&mut self, sample: DriveSample) {
        if self.len == 0 {
            return;
        }
        if self.len == Self::KEPT {
            self.samples.copy_within(2.., 1);
            self.len -= 1;
        }
        self.samples[self.len] = sample;
        self.len += 1;
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The compositor's track and the app's estimate of it.
pub(crate) struct MotionEntry {
    pub(crate) wire: wire::MotionId,
    /// The commit that creates the track: requests about it wait for that commit to have been written.
    pub(crate) created_tag: u64,
    pub(crate) track: Track,
    /// A rebuilt track counts its generations from zero: this puts them back on the compositor's numbering.
    gen_base: u32,
    replay: Replay,
    settle: Option<SettleRequest>,
}

/// A track holding `value`, even where `Track::new` would clamp it: a value past an end (a finger let go in the overscroll,
/// a settle stopped there) is reached by a short drag of its own, which leaves the track with the finger lifted and no velocity.
fn track_at(bounds: MotionBounds, value: f32) -> Track {
    let value = if value.is_finite() { value } else { bounds.min };
    if (bounds.min..=bounds.max).contains(&value) {
        return Track::new(value, bounds);
    }
    let (edge, excess) = if value < bounds.min {
        (bounds.min, bounds.min - value)
    } else {
        (bounds.max, value - bounds.max)
    };
    let raw = bounds.unresist(excess);
    let signed = if value < bounds.min { -raw } else { raw };
    let mut track = Track::new(edge, bounds);
    let contact = u32::MAX;
    let at = |phase, position, time_ns| DriveSample {
        contact,
        phase,
        position,
        time_ns,
    };
    track.drive(at(DrivePhase::Down, 0.0, 0), 0);
    track.drive(at(DrivePhase::Move, signed / bounds.ratio, 0), 0);
    // A second later, so that no velocity is measured over the 100 ms window.
    track.drive(at(DrivePhase::Up, signed / bounds.ratio, 1_000_000_000), 0);
    track
}

impl MotionEntry {
    fn new(value: f32, bounds: MotionBounds, created_tag: u64, wire: wire::MotionId) -> Self {
        Self {
            wire,
            created_tag,
            track: Track::new(value, bounds),
            gen_base: 0,
            replay: Replay::new(),
            settle: None,
        }
    }

    /// The compositor's numbering of the generation the estimate has reached.
    pub(crate) fn generation(&self) -> u32 {
        self.track.generation().wrapping_add(self.gen_base)
    }

    /// The value of the track at `at_ns`, without moving the mirror.
    pub(crate) fn value_at(&self, at_ns: u64) -> f32 {
        let mut t = self.track;
        t.advance(at_ns);
        t.value()
    }

    fn velocity_at(&self, at_ns: u64) -> f32 {
        let mut t = self.track;
        t.advance(at_ns);
        t.velocity()
    }

    /// The mirror takes a sample at the time the screen will show it.
    fn apply_drive(&mut self, sample: DriveSample, est_ns: u64) {
        let was_dragging = self.track.is_dragging();
        self.track.drive(sample, est_ns);
        match sample.phase {
            DrivePhase::Down => {
                if !was_dragging {
                    self.replay.begin(sample);
                }
            }
            DrivePhase::Move | DrivePhase::Up => {
                if was_dragging {
                    self.replay.push(sample);
                }
            }
            DrivePhase::Cancel => {
                if was_dragging {
                    self.replay.clear();
                }
            }
        }
    }

    /// The compositor reports what happened to the track. The estimate moves to agree; nothing is sent.
    fn reconcile(
        &mut self,
        generation: u32,
        kind: NoticeKind,
        value: f32,
        velocity: f32,
        time_ns: u64,
    ) {
        // A late event of a settle or a catch the estimate has already moved past says nothing new.
        if (generation.wrapping_sub(self.generation()) as i32) < 0 {
            return;
        }
        let bounds = *self.track.bounds();
        match kind {
            NoticeKind::Started => {
                let Some(request) = self.settle else {
                    return;
                };
                if self.track.is_dragging() {
                    // A finger has taken the track again since: the compositor's next event is its catch.
                    return;
                }
                let mut track = track_at(bounds, value);
                track.settle(time_ns, request.target, Some(velocity), request.kind);
                self.gen_base = generation.wrapping_sub(track.generation());
                self.track = track;
            }
            NoticeKind::Caught => {
                let mut track = track_at(bounds, value);
                for sample in &self.replay.samples[..self.replay.len] {
                    track.drive(*sample, time_ns);
                }
                self.gen_base = generation.wrapping_sub(track.generation());
                self.track = track;
                self.settle = None;
            }
            NoticeKind::Settled => {
                if self.track.is_dragging() {
                    return;
                }
                let track = track_at(bounds, value);
                self.gen_base = generation.wrapping_sub(track.generation());
                self.track = track;
                self.settle = None;
            }
        }
    }
}

impl Inner {
    pub(crate) fn motion_create(
        &mut self,
        value: f32,
        bounds: MotionBounds,
    ) -> Result<MotionId, UiError> {
        bounds.check().map_err(UiError::Invalid)?;
        if !value.is_finite() {
            return Err(UiError::Invalid("a motion track starts at a number"));
        }
        if self.motions.is_full() {
            return Err(UiError::Limit(Limit::Motions));
        }
        let wire = wire::MotionId(self.next_motion);
        self.next_motion += 1;
        let entry = MotionEntry::new(value, bounds, self.next_tag, wire);
        let start = entry.track.value();
        let key = self.motions.insert(entry).expect("room was checked above");
        self.batch.push(Request::MotionCreate {
            id: wire,
            value: start,
            bounds,
        });
        Ok(MotionId::from_key(key))
    }

    /// The estimated value of a track at `at_ns`.
    pub(crate) fn motion_value_at(&self, id: MotionId, at_ns: u64) -> Option<f32> {
        self.motions.get(id.key()).map(|m| m.value_at(at_ns))
    }

    /// A motion handle is dropped: what is bound to it is freed (keeping the values it has), then the track is deleted.
    pub(crate) fn motion_release(&mut self, id: MotionId) {
        let now = self.estimate_ns();
        let Some(entry) = self.motions.get(id.key()) else {
            return;
        };
        let wire = entry.wire;
        let value = entry.value_at(now);
        for (_, _, group) in self.groups.iter_mut() {
            group.unbind_motion(id, value, &mut self.batch);
        }
        // What was asked of the track and has not started is moot: the track goes with the next commit.
        self.out.forget_stream(wire.0);
        self.batch.push(Request::MotionDelete { id: wire });
        self.motions.remove(id.key());
    }

    pub(crate) fn observe_motion(
        &mut self,
        id: wire::MotionId,
        generation: u32,
        kind: NoticeKind,
        value: f32,
        velocity: f32,
        time_ns: u64,
    ) {
        if let Some((_, _, entry)) = self.motions.iter_mut().find(|(_, _, m)| m.wire == id) {
            entry.reconcile(generation, kind, value, velocity, time_ns);
        }
    }
}

/// A scalar motion track: drive it with the finger, settle it, cancel it, and bind groups' poses to it. Dropping it frees what
/// is bound to it and deletes it on the compositor.
pub struct Motion {
    id: MotionId,
    scene: Scene,
}

impl Scene {
    /// A motion track at `value` within `bounds`, in the units `bounds` declares. At most [`MAX_MOTIONS`] at once.
    ///
    /// `bounds.ratio` converts the pointer's travel to the track's units (negative when dragging up should raise the value);
    /// `bounds.anchors` are where a scroll settles (a pager's pages); `bounds.overscroll` how far past an end a finger may pull
    /// it, rubber-banded.
    pub fn motion(&self, value: f32, bounds: MotionBounds) -> Result<Motion, UiError> {
        let id = self.inner.borrow_mut().motion_create(value, bounds)?;
        Ok(Motion {
            id,
            scene: self.clone(),
        })
    }

    /// How many motion tracks this scene holds on the compositor now: [`MAX_MOTIONS`] less this is the room left.
    pub fn motions_alive(&self) -> usize {
        self.inner.borrow().motions.live()
    }

    /// Whether any motion track is settling by the estimate at `at_ns` (a settle that has arrived by then is not). While one is, the
    /// compositor draws a frame at every refresh by itself, a commit or none (`Track::is_active`): the loop tells the finger's
    /// resampler (`Resampler::animating`). A finger on a track is not a settle: its frames come with the finger's own samples.
    pub fn motion_settling_at(&self, at_ns: u64) -> bool {
        self.inner.borrow().motions.iter().any(|(_, _, m)| {
            let mut track = m.track;
            track.advance(at_ns);
            track.is_settling()
        })
    }

    /// What the compositor tells the app when a track it settled has arrived (`NoticeKind::Settled`), for every track whose settle has
    /// arrived by the estimate at `at_ns`. A recording compositor never says it, so a test that lets time pass says it for the
    /// compositor, as the host's loop would hear it: until it does, a finger that lands on a track that has long since come to rest
    /// finds it still settling, and catches it.
    #[cfg(any(test, feature = "testing"))]
    pub fn settle_arrived(&self, at_ns: u64) {
        let arrived: Vec<(wire::MotionId, u32, f32)> = self
            .inner
            .borrow()
            .motions
            .iter()
            .filter_map(|(_, _, m)| {
                let mut track = m.track;
                track.advance(at_ns);
                (m.track.is_active() && !m.track.is_dragging() && !track.is_settling())
                    .then(|| (m.wire, m.generation(), m.value_at(at_ns)))
            })
            .collect();
        for (id, generation, value) in arrived {
            self.inner.borrow_mut().observe_motion(
                id,
                generation,
                NoticeKind::Settled,
                value,
                0.0,
                at_ns,
            );
        }
    }
}

impl Motion {
    pub fn id(&self) -> MotionId {
        self.id
    }

    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The compositor's name for the track.
    pub fn wire_id(&self) -> wire::MotionId {
        self.with(|e| e.wire).expect("a live handle")
    }

    /// The bounds the track was made with.
    pub fn bounds(&self) -> MotionBounds {
        self.with(|e| *e.track.bounds()).expect("a live handle")
    }

    fn with<R>(&self, f: impl FnOnce(&MotionEntry) -> R) -> Option<R> {
        self.scene.inner.borrow().motions.get(self.id.key()).map(f)
    }

    /// Applies one already-resampled pointer sample: the compositor takes it into the frame it makes next, with no
    /// prediction of its own added. A `Down` on a settling track catches it at the value it has; later `Move`s apply deltas to
    /// the landing. Goes out at once, ahead of anything not yet started (and replaces a `Move` of the same contact that has not
    /// started). A position that is not a number is refused; a failed connection is reported here and by the next commit.
    ///
    /// *Differs from the design's sketch* only in returning a `Result` (the sketch returns nothing).
    pub fn drive(&self, sample: DriveSample) -> Result<(), UiError> {
        if !sample.position.is_finite() {
            return Err(UiError::Invalid(
                "a pointer sample's position must be a number",
            ));
        }
        let mut guard = self.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let est = inner.estimate_ns();
        let entry = inner
            .motions
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Motion))?;
        entry.apply_drive(sample, est);
        let request = Request::MotionDrive {
            id: entry.wire,
            sample,
        };
        let how = Urgent {
            after_tag: entry.created_tag,
            stream: entry.wire.0,
            replaceable: (sample.phase == DrivePhase::Move).then_some(sample.contact),
        };
        inner.out.push_urgent(&request, how).map_err(UiError::Io)
    }

    /// Starts a settle from the track's actual value, with `velocity` (the measured one when `None`), toward `target` (the
    /// nearest anchor, or where it is, when `None`; scroll inertia takes none). At once, like a drive sample. The settle is
    /// checked first (`Settle::check`): a settle the compositor would refuse is an `Err`, and nothing is sent.
    ///
    /// *Differs from the design's sketch* (`settle(target: f32, velocity: f32, ..)`) in taking options, which the protocol
    /// has (`MotionSettle::{target, velocity}`), and in returning a `Result`.
    pub fn settle(
        &self,
        target: Option<f32>,
        velocity: Option<f32>,
        settle: Settle,
    ) -> Result<(), UiError> {
        settle.check().map_err(UiError::Invalid)?;
        if target.is_some_and(|t| !t.is_finite()) || velocity.is_some_and(|v| !v.is_finite()) {
            return Err(UiError::Invalid(
                "a settle's target and velocity must be numbers",
            ));
        }
        let mut guard = self.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let est = inner.estimate_ns();
        let entry = inner
            .motions
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Motion))?;
        entry.track.settle(est, target, velocity, settle);
        entry.settle = Some(SettleRequest {
            target,
            kind: settle,
        });
        entry.replay.clear();
        let request = Request::MotionSettle {
            id: entry.wire,
            target,
            velocity,
            settle,
        };
        let how = Urgent {
            after_tag: entry.created_tag,
            stream: entry.wire.0,
            replaceable: None,
        };
        inner.out.push_urgent(&request, how).map_err(UiError::Io)
    }

    /// Stops the track: where it is (`Hold`) or at the end of its settle (`Complete`). At once.
    pub fn cancel(&self, policy: CancelPolicy) -> Result<(), UiError> {
        let mut guard = self.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let est = inner.estimate_ns();
        let entry = inner
            .motions
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Motion))?;
        entry.track.cancel(est, policy);
        entry.settle = None;
        entry.replay.clear();
        let request = Request::MotionCancel {
            id: entry.wire,
            policy,
        };
        let how = Urgent {
            after_tag: entry.created_tag,
            stream: entry.wire.0,
            replaceable: None,
        };
        inner.out.push_urgent(&request, how).map_err(UiError::Io)
    }

    /// The value the screen shows by the time a request made now is on it: the estimate at the clock plus the lead.
    pub fn value(&self) -> f32 {
        let at = self.scene.inner.borrow().estimate_ns();
        self.value_at(at)
    }

    /// The estimated value at `at_ns` (`CLOCK_MONOTONIC`), by the same equations the compositor runs.
    pub fn value_at(&self, at_ns: u64) -> f32 {
        self.with(|e| e.value_at(at_ns)).expect("a live handle")
    }

    /// The estimated velocity (track units per second) at `at_ns`.
    pub fn velocity_at(&self, at_ns: u64) -> f32 {
        self.with(|e| e.velocity_at(at_ns)).expect("a live handle")
    }

    /// A copy of the mirror, to ask whatever `Track` can answer.
    pub fn track(&self) -> Track {
        self.with(|e| e.track).expect("a live handle")
    }

    /// Whether a finger is on the track or a settle is running (frames are needed at every refresh).
    pub fn is_active(&self) -> bool {
        self.with(|e| e.track.is_active()).expect("a live handle")
    }

    pub fn is_dragging(&self) -> bool {
        self.with(|e| e.track.is_dragging()).expect("a live handle")
    }

    /// Whether a settle is running by the estimate at `at_ns` (a settle that has arrived by then is not).
    pub fn is_settling_at(&self, at_ns: u64) -> bool {
        self.with(|e| {
            let mut t = e.track;
            t.advance(at_ns);
            t.is_settling()
        })
        .expect("a live handle")
    }

    /// The compositor's generation number the estimate has reached: it changes with every settle and catch.
    pub fn generation(&self) -> u32 {
        self.with(MotionEntry::generation).expect("a live handle")
    }
}

impl Drop for Motion {
    fn drop(&mut self) {
        self.scene.inner.borrow_mut().motion_release(self.id);
    }
}

/// `DriveSample`s for the four phases of a contact, so that call sites read as what they mean.
pub mod sample {
    use super::{DrivePhase, DriveSample};

    fn of(phase: DrivePhase, contact: u32, position: f32, time_ns: u64) -> DriveSample {
        DriveSample {
            contact,
            phase,
            position,
            time_ns,
        }
    }

    /// A finger lands at `position` (along the track's axis, logical pixels) at `time_ns` (`CLOCK_MONOTONIC`).
    pub fn down(contact: u32, position: f32, time_ns: u64) -> DriveSample {
        of(DrivePhase::Down, contact, position, time_ns)
    }

    pub fn moved(contact: u32, position: f32, time_ns: u64) -> DriveSample {
        of(DrivePhase::Move, contact, position, time_ns)
    }

    /// The finger lifts at `position`: the final position, from which the velocity of the release is measured.
    pub fn up(contact: u32, position: f32, time_ns: u64) -> DriveSample {
        of(DrivePhase::Up, contact, position, time_ns)
    }

    /// The touch was taken away (no fling).
    pub fn cancel(contact: u32, position: f32, time_ns: u64) -> DriveSample {
        of(DrivePhase::Cancel, contact, position, time_ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneOptions;
    use crate::scene::testing::{connect, drain};
    use compositor_protocol::{Ease, Event};

    const MS: u64 = 1_000_000;

    fn pixels(min: f32, max: f32) -> MotionBounds {
        MotionBounds::range(min, max, Units::Pixels)
    }

    fn scroll() -> Settle {
        Settle::Scroll(ScrollToken {
            friction: 4.0,
            spring: SpringToken { response_ms: 300.0 },
        })
    }

    fn spring() -> Settle {
        Settle::CriticalSpring(SpringToken { response_ms: 300.0 })
    }

    /// The clock the scene reads, set by the test, and a zero lead: the estimate is then the track at that time.
    fn fixed_clock(scene: &Scene) -> std::rc::Rc<std::cell::Cell<u64>> {
        let now = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let read = std::rc::Rc::clone(&now);
        let mut inner = scene.inner.borrow_mut();
        inner.clock = std::rc::Rc::new(move || read.get());
        inner.lead = Lead::fixed(0);
        now
    }

    #[test]
    fn a_value_past_an_end_is_reached_by_the_pull_the_track_itself_resists_to_it() {
        // A drag far past the end shows `resist(excess)` (the one rubber band); reaching a shown value by `track_at`, which takes the
        // pull back with the track's own `unresist`, must land on it.
        let bounds = MotionBounds {
            overscroll: 80.0,
            ratio: -1.0,
            ..pixels(0.0, 500.0)
        };
        for shown in [-79.0, -40.0, -1.0, 0.5, 250.0, 500.0, 501.0, 540.0, 579.0] {
            let t = track_at(bounds, shown);
            assert!((t.value() - shown).abs() < 1e-2, "{shown}: {}", t.value());
            assert_eq!(t.velocity(), 0.0);
            assert!(!t.is_active(), "a finger-lifted track is no settle");
        }
        // And it continues like the real thing: a finger landing there moves from that value without a jump.
        let mut t = track_at(bounds, 540.0);
        t.drive(sample::down(1, 100.0, 0), 0);
        t.drive(sample::moved(1, 100.0, 16 * MS), 16 * MS);
        assert!((t.value() - 540.0).abs() < 1e-2);
    }

    #[test]
    fn the_estimate_follows_a_drag_a_release_and_a_scroll_settle_exactly_as_the_shared_track_does()
    {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let now = fixed_clock(&scene);
        let bounds = MotionBounds {
            ratio: -1.0,
            overscroll: 60.0,
            anchors: [0.0, 480.0, 960.0, 1440.0, 0.0, 0.0, 0.0, 0.0],
            anchor_count: 4,
            ..pixels(0.0, 1440.0)
        };
        let motion = scene.motion(0.0, bounds).unwrap();
        scene.try_commit().unwrap();
        // The same samples and settle into a track of our own.
        let mut own = Track::new(0.0, bounds);
        let mut t = 0u64;
        let drive = |s: DriveSample, at: u64, own: &mut Track| {
            now.set(at);
            motion.drive(s).unwrap();
            own.drive(s, at);
        };
        drive(sample::down(7, 800.0, 0), 0, &mut own);
        for k in 1..=10u64 {
            t = k * 16 * MS;
            drive(sample::moved(7, 800.0 - 24.0 * k as f32, t), t, &mut own);
        }
        t += 16 * MS;
        drive(sample::up(7, 800.0 - 24.0 * 11.0, t), t, &mut own);
        now.set(t);
        motion.settle(None, None, scroll()).unwrap();
        own.settle(t, None, None, scroll());
        assert_eq!(motion.generation(), own.generation());
        for k in 0..60u64 {
            let at = t + k * 16 * MS;
            own.advance(at);
            assert_eq!(motion.value_at(at), own.value(), "frame {k}");
            assert_eq!(motion.velocity_at(at), own.velocity(), "frame {k}");
        }
        // It lands on an anchor, as the compositor's plan does.
        assert_eq!(motion.value_at(t + 20_000 * MS), 480.0);
        // Everything went to the compositor in order, the samples ahead of nothing.
        let sent = drain(&mut comp);
        assert!(
            sent.iter()
                .any(|r| matches!(r, Request::MotionSettle { .. }))
        );
    }

    #[test]
    fn started_re_seats_the_estimate_on_the_compositors_start_and_a_late_event_is_ignored() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let now = fixed_clock(&scene);
        let motion = scene.motion(100.0, pixels(0.0, 10_000.0)).unwrap();
        // The app settles at its own idea of the frame time (t = 1000 ms)...
        now.set(1000 * MS);
        motion.settle(None, Some(2000.0), scroll()).unwrap();
        let wire = motion.wire_id();
        // ...the compositor started it 6 ms later, from the same value and velocity.
        let started = 1006 * MS;
        scene.observe(&Event::Motion {
            id: wire,
            generation: motion.generation(),
            kind: NoticeKind::Started,
            value: 100.0,
            velocity: 2000.0,
            time_ns: started,
        });
        let mut theirs = Track::new(100.0, pixels(0.0, 10_000.0));
        theirs.settle(started, None, Some(2000.0), scroll());
        for k in 0..30u64 {
            let at = started + k * 16 * MS;
            theirs.advance(at);
            assert_eq!(motion.value_at(at), theirs.value(), "{k}");
        }
        // An event of an older settle arrives late: nothing changes.
        let before = motion.value_at(started + 100 * MS);
        scene.observe(&Event::Motion {
            id: wire,
            generation: motion.generation().wrapping_sub(1),
            kind: NoticeKind::Settled,
            value: 7.0,
            velocity: 0.0,
            time_ns: started + 50 * MS,
        });
        assert_eq!(motion.value_at(started + 100 * MS), before);
    }

    #[test]
    fn a_catch_re_seats_the_drag_on_the_value_the_compositor_caught_and_a_settled_event_brings_the_end()
     {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let now = fixed_clock(&scene);
        let bounds = MotionBounds {
            ratio: -1.0,
            ..pixels(0.0, 1000.0)
        };
        let motion = scene.motion(0.0, bounds).unwrap();
        let wire = motion.wire_id();
        now.set(0);
        motion.settle(Some(800.0), Some(0.0), spring()).unwrap();
        let g_settle = motion.generation();
        // A finger lands at 200 ms and moves 30 px; the app's idea of the catch is at 200 ms exactly...
        now.set(200 * MS);
        motion.drive(sample::down(1, 400.0, 200 * MS)).unwrap();
        motion.drive(sample::moved(1, 370.0, 216 * MS)).unwrap();
        assert_eq!(motion.generation(), g_settle + 1);
        // ...the compositor caught it at 204 ms, 3 px further along, and says so.
        let caught_value = motion.value_at(200 * MS) - 30.0 + 3.0;
        scene.observe(&Event::Motion {
            id: wire,
            generation: g_settle + 1,
            kind: NoticeKind::Caught,
            value: caught_value,
            velocity: 0.0,
            time_ns: 204 * MS,
        });
        // The drag now sits on the compositor's value: the 30 px move since the landing is applied to it.
        assert!(
            (motion.value_at(216 * MS) - (caught_value + 30.0)).abs() < 1e-3,
            "{}",
            motion.value_at(216 * MS)
        );
        assert!(motion.is_dragging());
        // The finger lifts and the compositor's settle ends: the estimate rests on the reported value.
        motion.drive(sample::up(1, 370.0, 232 * MS)).unwrap();
        now.set(232 * MS);
        motion.settle(Some(0.0), None, spring()).unwrap();
        scene.observe(&Event::Motion {
            id: wire,
            generation: motion.generation(),
            kind: NoticeKind::Settled,
            value: 0.0,
            velocity: 0.0,
            time_ns: 900 * MS,
        });
        assert_eq!(motion.value_at(900 * MS), 0.0);
        assert!(!motion.is_active());
    }

    #[test]
    fn invalid_arguments_are_refused_and_nothing_is_sent_for_them() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let motion = scene.motion(0.0, pixels(0.0, 100.0)).unwrap();
        scene.try_commit().unwrap();
        assert!(matches!(
            motion.drive(sample::moved(1, f32::NAN, 0)),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            motion.settle(
                None,
                None,
                Settle::CriticalSpring(SpringToken { response_ms: 1.0 })
            ),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            motion.settle(Some(f32::INFINITY), None, spring()),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.motion(f32::NAN, pixels(0.0, 1.0)),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.motion(0.0, pixels(2.0, 1.0)),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            scene.motion(
                0.0,
                MotionBounds {
                    ratio: 0.0,
                    ..pixels(0.0, 1.0)
                }
            ),
            Err(UiError::Invalid(_))
        ));
        let sent = drain(&mut comp);
        assert_eq!(
            sent.len(),
            2,
            "the track's creation and its commit, and nothing for the refusals: {sent:?}"
        );
        let _ = Ease::Out;
    }

    #[test]
    fn at_most_the_limit_of_tracks_and_a_dropped_ones_slot_is_free_again_under_a_new_generation() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let mut tracks: Vec<Motion> = (0..MAX_MOTIONS)
            .map(|_| scene.motion(0.0, pixels(0.0, 1.0)).unwrap())
            .collect();
        assert!(matches!(
            scene.motion(0.0, pixels(0.0, 1.0)),
            Err(UiError::Limit(Limit::Motions))
        ));
        let gone = tracks.pop().unwrap();
        let (old_id, old_wire) = (gone.id(), gone.wire_id());
        drop(gone);
        let fresh = scene.motion(0.0, pixels(0.0, 1.0)).unwrap();
        assert_eq!(fresh.id().index(), old_id.index());
        assert_ne!(fresh.id(), old_id, "the generation moved on");
        assert_ne!(
            fresh.wire_id(),
            old_wire,
            "and the compositor's id is never reused"
        );
    }

    #[test]
    fn the_lead_is_the_default_until_frames_say_and_then_follows_them_inside_a_sane_range() {
        let mut lead = Lead::default();
        assert_eq!(lead.ns(), DEFAULT_PRESENT_LEAD_NS);
        lead.committed(1, 1_000 * MS);
        lead.committed(2, 1_020 * MS);
        // Frame of commit 2 on the screen 25 ms after it was taken: the first measurement is taken whole.
        lead.presented(2, 1_045 * MS);
        assert_eq!(lead.ns(), 25 * MS);
        assert_eq!(lead.measured(), 1);
        // Commit 1's frame never comes separately: it was folded into the frame of commit 2.
        lead.presented(1, 1_030 * MS);
        assert_eq!(lead.measured(), 1, "already accounted for");
        // A stalled frame (700 ms) is no measurement of the pipeline.
        lead.committed(3, 2_000 * MS);
        lead.presented(3, 2_700 * MS);
        assert_eq!(lead.ns(), 25 * MS);
        // Later ones move it an eighth of the way.
        lead.committed(4, 3_000 * MS);
        lead.presented(4, 3_017 * MS);
        assert_eq!(lead.ns(), 25 * MS - 8 * MS / 8);
    }

    #[test]
    fn the_scene_is_settling_from_a_settle_until_the_estimate_of_its_end_and_a_finger_on_a_track_is_no_settle()
     {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let now = fixed_clock(&scene);
        let a = scene.motion(0.0, pixels(0.0, 1000.0)).unwrap();
        let b = scene.motion(0.0, pixels(0.0, 1000.0)).unwrap();
        let timed = |ms| {
            Settle::Timed(MotionToken {
                duration_ms: ms,
                ease: Ease::Out,
            })
        };
        assert!(!scene.motion_settling_at(0), "tracks at rest");
        // A finger on a track makes the compositor draw when the finger moves, not at every refresh.
        now.set(50 * MS);
        b.drive(sample::down(1, 500.0, 50 * MS)).unwrap();
        b.drive(sample::moved(1, 480.0, 66 * MS)).unwrap();
        assert!(b.is_active() && !scene.motion_settling_at(70 * MS));
        // A settle of the other track is one from the request to the estimate of its end, and not after.
        now.set(100 * MS);
        a.settle(Some(800.0), Some(0.0), timed(220)).unwrap();
        assert!(scene.motion_settling_at(100 * MS));
        assert!(scene.motion_settling_at(319 * MS));
        assert!(!scene.motion_settling_at(321 * MS), "arrived by then");
        // And the finger's lift followed by its settle is one, whichever track.
        now.set(400 * MS);
        b.drive(sample::up(1, 480.0, 400 * MS)).unwrap();
        assert!(!scene.motion_settling_at(401 * MS));
        b.settle(None, Some(2000.0), scroll()).unwrap();
        assert!(scene.motion_settling_at(401 * MS));
    }
}
