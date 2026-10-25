//! Smooth first, "Full": the ladder of visible steps the compositor takes, one at a time and only while its own pacer predicts that
//! frames will be late, and takes back, one at a time, when the load eases (`experiments/s9-glass/GOVERNOR.md`, "The ladder").
//!
//! Below `Full` (`compositor_protocol::SmoothFirst`) this does nothing at all: [`Ladder::observe`] is not even called, the level
//! stays 0 and no layer is looked at. At `Full`, with no step in force, a frame costs the pacer's median of eight numbers and a few
//! integer comparisons ([`Ladder::observe`]); a step costs a look at the scene, once, when it is taken or given back.
//!
//! **What is predicted.** The pacer's own prediction, `Pacer::forecast`: the time a frame wanted at every refresh has to spare if it
//! takes what a typical recent frame did, with the slack the pacer gives itself for a frame that costs more than its neighbours.
//! Negative, the pacer could not start a frame in time to make its refresh; that is what "late" means here, and no other number is
//! made up. (The design proposed a cost model fitted from the lab's sweeps; the pacer's median of what frames did takes the same
//! GPU clock, heat and load into account without a table that goes stale.) It is believed only once the eight frames it is made
//! of were drawn at the level that is in force now ([`WINDOW`]), so a step is never taken twice for the frames before it.
//!
//! **When a step is taken.** At a settled boundary: never while a finger is on a track, a settle runs or a layer is animated
//! ([`World::moving`]); a step that the evidence wants waits for the motion to end and is taken by the frame that ends it (or
//! the first one after it). It is taken at the first step that would change what the scene draws ([`World::changes`]): a screen
//! with no live blur goes to the wallpaper, and a screen with nothing that moves goes to the shadows; and one step at a time:
//! the next needs the pacer's window to hold eight frames of the level the last one made.
//!
//! **When a step is given back.** The highest step in force goes when the frames, as they would be without it, are predicted to
//! fit with a refresh's eighth to spare for three seconds of frames drawn, and ten seconds have passed since a step was taken. What
//! they would cost without it is what they cost with it over what the step leaves of a frame (its *share*: what the frames after
//! it were seen to save, learned when it was taken, 15 % of the frame until then). A load that made the step is not
//! given the step back for the same frames; one that has eased by the share is. That is the hysteresis, and the two times bound how
//! often the level can change at all.
//!
//! The steps themselves are plain functions of the layers: [`rewrite`] is the glass as it is drawn at a level, [`Ambient`] is what a
//! wallpaper and an orbit may do. Every one is a function of the level alone, so a step given back leaves the next frame as it
//! would have been had it never been taken.

use crate::log::log;
use crate::ops::Op;
use crate::pacer::{Forecast, PIPELINE_WINDOW};
use compositor_protocol::ladder::{RUNGS, STEPS};
use compositor_protocol::{Backdrop, Content, Glass, Rgba, Shadow, Sheen, SmoothFirst};
use std::sync::LazyLock;

/// The steps, in the order they are taken (`compositor_protocol::ladder::STEPS`): a level `n` has steps `1..=n` in force.
pub const LIVE_BLUR: u8 = 1;
pub const WALLPAPER_HALF: u8 = 2;
pub const WALLPAPER_STILL: u8 = 3;
pub const SOFTEN: u8 = 4;
pub const PLAIN_TINT: u8 = 5;
const _: () = assert!(PLAIN_TINT == RUNGS);

/// Frames of one level the pacer's window must hold before what it predicts is believed: its whole window, so that none of the
/// frames it is made of was drawn at another level.
pub const WINDOW: u32 = PIPELINE_WINDOW as u32;
/// How long frames must be comfortably under the prediction before a step is given back, in time spent drawing them (a long pause
/// between two frames counts for no more than a few refreshes).
const CALM_NS: u64 = 3_000_000_000;
/// How long after a step was taken none is given back.
const HOLD_NS: u64 = 10_000_000_000;
/// The most a gap between two frames counts for in the time spent drawing, in refreshes.
const MOST_GAP_REFRESHES: u64 = 4;
/// A step is given back only when the frames, as they would be without it, would have this fraction of a refresh to spare.
const SPARE_FRACTION: u64 = 8;
/// What a step is taken to save of a frame until the frames after it have said, in parts of a thousand, and the most it can be
/// learned to (a step is never taken to leave nothing of a frame).
const DEFAULT_SHARE: u32 = 150;
const MOST_SHARE: u32 = 900;
/// What a wallpaper may move at while the wallpaper is slowed, frames a second.
const HALF_FPS: u8 = 30;
/// How much of a shadow's blur and offset, and of a sheen, is left when they are softened.
const SOFT: f32 = 0.5;

/// Whether this compositor has the ladder to offer: it is, unless the compositor's environment says `LUMASHOW_LADDER=off` (to
/// measure what the ladder is worth, as `LUMASHOW_WARMUP=off` does for the warm-up). What `Welcome::smooth_first_full` says.
pub fn available() -> bool {
    static AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
        let off = std::env::var("LUMASHOW_LADDER").as_deref() == Ok("off");
        if off {
            log("the Full ladder is off (LUMASHOW_LADDER=off)");
        }
        !off
    });
    *AVAILABLE
}

/// Why a compositor whose ladder is, or is not, `available` cannot draw `level`: the words of the refusal. A level it cannot carry
/// out is refused and never accepted to do nothing, so that Settings does not show one.
pub fn refusal(level: SmoothFirst, available: bool) -> Option<&'static str> {
    (level.ladder() && !available)
        .then_some("this compositor cannot draw Smooth first: Full (its ladder is switched off)")
}

/// What a frame is, for the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Observed {
    /// When the frame starts, `CLOCK_MONOTONIC` nanoseconds.
    pub now_ns: u64,
    /// The frame was wanted at every refresh (`engine::FrameLoop::frame`): only such a frame can be late, so only one can ask for a
    /// step to be taken.
    pub due: bool,
    /// What the pacer expects of the next frames (`Pacer::forecast`).
    pub forecast: Forecast,
}

/// What the ladder asks of the scene, once it has something to decide.
pub trait World {
    /// Whether something a person set going still moves after this frame: a finger on a track, a settle that does not end with it,
    /// a layer's animation that does not. A step is not taken, or given back, then.
    fn moving(&mut self) -> bool;
    /// Whether step `rung` would change anything the scene draws (see [`changes`]).
    fn changes(&mut self, rung: u8) -> bool;
}

/// What a frame's observation did to the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Stay,
    /// A step was taken: the level is now this.
    Down(u8),
    /// A step was given back: the level is now this.
    Up(u8),
}

/// What a step is being learned to save: the typical frame before it, for when the frames after it have said.
#[derive(Clone, Copy, Debug)]
struct Learning {
    rung: u8,
    typical_before_ns: u64,
}

/// The ladder's state: one small record, no heap.
#[derive(Debug)]
pub struct Ladder {
    period_ns: u64,
    /// `SmoothFirst::Full` is chosen.
    on: bool,
    /// The steps in force: 0 for none.
    level: u8,
    /// Frames drawn since the level last changed, at most [`WINDOW`] counted.
    fresh: u32,
    /// The evidence asks for a step, which waits for a settled boundary.
    wanted: bool,
    /// Time spent drawing frames comfortably under the prediction, since the last that was not.
    calm_ns: u64,
    last_ns: Option<u64>,
    held_until_ns: u64,
    /// The share of a frame each step took off it the last time frames showed it, parts of a thousand.
    share: [u32; RUNGS as usize],
    learning: Option<Learning>,
}

impl Ladder {
    /// A ladder for a panel that refreshes every `period_ns`, off.
    pub fn new(period_ns: u64) -> Self {
        assert!(period_ns > 0);
        Self {
            period_ns,
            on: false,
            level: 0,
            fresh: 0,
            wanted: false,
            calm_ns: 0,
            last_ns: None,
            held_until_ns: 0,
            share: [DEFAULT_SHARE; RUNGS as usize],
            learning: None,
        }
    }

    /// Whether the ladder runs (`SmoothFirst::Full`).
    pub fn on(&self) -> bool {
        self.on
    }

    /// The steps in force, 0 when there are none (always so when the ladder is off).
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Takes in a change of level or the client going away: switched on, it starts from no step, and switched off every step is
    /// given back and what was learned is forgotten.
    pub fn apply(&mut self, op: &Op) {
        match op {
            Op::SmoothFirst(level) => self.set_on(level.ladder()),
            Op::Clear => self.set_on(false),
            _ => {}
        }
    }

    fn set_on(&mut self, on: bool) {
        if on != self.on || !on {
            *self = Self::new(self.period_ns);
            self.on = on;
        }
    }

    fn spare_ns(&self) -> i64 {
        (self.period_ns / SPARE_FRACTION) as i64
    }

    /// What is left of a refresh for frames as they would be without the step on top, given what they cost with it and the share
    /// that step takes off a frame: negative when they would be late.
    fn headroom_without_the_top_step(&self, forecast: Forecast) -> i64 {
        let share = u64::from(self.share[usize::from(self.level) - 1]);
        let without = forecast.typical_ns.saturating_mul(1000) / (1000 - share);
        forecast.headroom_ns + forecast.typical_ns as i64 - without as i64
    }

    /// Looks at a frame that is about to be drawn and says whether the level changes for it. Called for every frame of a ladder
    /// that is on.
    pub fn observe(&mut self, seen: Observed, world: &mut dyn World) -> Move {
        if !self.on {
            return Move::Stay;
        }
        let gap = self.last_ns.map_or(0, |last| {
            seen.now_ns
                .saturating_sub(last)
                .min(MOST_GAP_REFRESHES * self.period_ns)
        });
        self.last_ns = Some(seen.now_ns);
        self.fresh = self.fresh.saturating_add(1).min(WINDOW);
        if self.fresh < WINDOW {
            return Move::Stay;
        }
        // The frames since a step was taken have filled the window: what they predict says what the step saved.
        if let Some(Learning {
            rung,
            typical_before_ns,
        }) = self.learning.take()
        {
            let saved = typical_before_ns.saturating_sub(seen.forecast.typical_ns);
            let share = saved.saturating_mul(1000) / typical_before_ns.max(1);
            self.share[usize::from(rung) - 1] = u32::try_from(share).unwrap_or(0).min(MOST_SHARE);
        }
        let late = seen.forecast.headroom_ns < 0;
        if late {
            // A frame that nothing wanted at this refresh can be as slow as it likes: it cannot be late.
            self.wanted |= seen.due;
        } else {
            self.wanted = false;
        }
        let comfortable = self.level > 0
            && !self.wanted
            && self.headroom_without_the_top_step(seen.forecast) >= self.spare_ns();
        if comfortable {
            self.calm_ns += gap;
        } else {
            self.calm_ns = 0;
        }
        if self.wanted && self.level < RUNGS && !world.moving() {
            return self.take_a_step(seen, world);
        }
        if self.calm_ns >= CALM_NS && seen.now_ns >= self.held_until_ns && !world.moving() {
            return self.give_a_step_back(world);
        }
        Move::Stay
    }

    /// The first step above the level that changes anything the scene draws, if there is one.
    fn take_a_step(&mut self, seen: Observed, world: &mut dyn World) -> Move {
        self.wanted = false;
        let Some(rung) = (self.level + 1..=RUNGS).find(|&rung| world.changes(rung)) else {
            // Nothing above would change what is drawn: ask the evidence again in a window's time, not at every frame.
            self.fresh = 0;
            return Move::Stay;
        };
        self.level = rung;
        self.fresh = 0;
        self.calm_ns = 0;
        self.held_until_ns = seen.now_ns + HOLD_NS;
        self.learning = Some(Learning {
            rung,
            typical_before_ns: seen.forecast.typical_ns,
        });
        Move::Down(rung)
    }

    /// The top step goes, and with it the steps below it that changed nothing: the level lands on the highest step that still does.
    fn give_a_step_back(&mut self, world: &mut dyn World) -> Move {
        let level = (1..self.level)
            .rev()
            .find(|&rung| world.changes(rung))
            .unwrap_or(0);
        self.level = level;
        self.fresh = 0;
        self.calm_ns = 0;
        self.learning = None;
        Move::Up(level)
    }
}

/// What a step that is in force says in the compositor's log.
pub fn words(level: u8) -> &'static str {
    STEPS[usize::from(level.clamp(1, RUNGS)) - 1]
}

/// The glass as it is drawn with the steps `1..=level` in force: a function of the glass and the level alone.
///
/// * Step 1: a live backdrop is a cached one, the same blur radius (made again only when what is below it changes).
/// * Step 4: a shadow's blur and offset, and the sheen, are half what they were, and a surface casts one shadow and not two.
///   The light and shade pair of a raised or inset control is not touched: it is what makes a control look like one.
/// * Step 5: the glass is its tint: no backdrop, no refraction, no sheen, no lip, no shadow. The edge, and a control's light and
///   shade, stay.
pub fn rewrite(glass: Glass, level: u8) -> Glass {
    let mut glass = glass;
    if level >= LIVE_BLUR {
        if let Backdrop::Live { radius, .. } = glass.backdrop {
            glass.backdrop = Backdrop::Cached { radius };
        }
    }
    if level >= SOFTEN {
        glass.shadows = [
            glass.shadows.into_iter().flatten().next().map(softened),
            None,
        ];
        glass.sheen = glass.sheen.map(|sheen| Sheen {
            peak: sheen.peak * SOFT,
            floor: sheen.floor * SOFT,
            ..sheen
        });
    }
    if level >= PLAIN_TINT {
        glass = Glass {
            backdrop: Backdrop::None,
            refraction: 0.0,
            highlight: Rgba(0, 0, 0, 0),
            sheen: None,
            shadows: [None, None],
            ..glass
        };
    }
    glass
}

fn softened(shadow: Shadow) -> Shadow {
    Shadow {
        offset_y: shadow.offset_y * SOFT,
        blur: shadow.blur * SOFT,
        ..shadow
    }
}

/// Whether step `rung` changes how `content` is drawn.
pub fn changes(rung: u8, content: &Content) -> bool {
    match (rung, content) {
        (LIVE_BLUR | SOFTEN | PLAIN_TINT, Content::Glass(glass)) => {
            rewrite(*glass, rung) != rewrite(*glass, rung - 1)
        }
        (WALLPAPER_HALF, Content::Wallpaper(wallpaper)) => {
            wallpaper.speed != 0.0 && wallpaper.fps > HALF_FPS
        }
        (WALLPAPER_STILL, Content::Wallpaper(wallpaper)) => {
            wallpaper.speed != 0.0 && wallpaper.fps > 0
        }
        (WALLPAPER_STILL, Content::Mesh(mesh)) => mesh.orbit_deg_per_s != 0.0,
        _ => false,
    }
}

/// How much the scene's ambient motion may move: what the wallpapers and the orbiting meshes are held to (steps 2 and 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ambient {
    /// As the layers ask.
    #[default]
    Asked,
    /// A wallpaper moves at most [`HALF_FPS`] frames a second.
    Half,
    /// A wallpaper stands where it is and an orbiting mesh stops turning; both go on from there when it is over.
    Still,
}

impl Ambient {
    /// What the steps `1..=level` allow.
    pub fn at(level: u8) -> Self {
        if level >= WALLPAPER_STILL {
            Self::Still
        } else if level >= WALLPAPER_HALF {
            Self::Half
        } else {
            Self::Asked
        }
    }

    /// The rate a wallpaper that asked to move at `asked` frames a second moves at.
    pub fn fps(self, asked: u8) -> u8 {
        match self {
            Self::Asked => asked,
            Self::Half => asked.min(HALF_FPS),
            Self::Still => 0,
        }
    }

    /// Whether a mesh that asked to orbit does.
    pub fn orbits(self) -> bool {
        self != Self::Still
    }
}
