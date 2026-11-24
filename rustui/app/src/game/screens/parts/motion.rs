//! How a layer comes onto the screen and goes off it, and the durations the theme's tokens do not carry.
//!
//! A node tagged with an [`Enter`] (`Paint::arrives`) starts away from its own pose and moves to it on the spring, stretched from the
//! theme's `layerMs` (`tokens.motion.layer_ms`, 220 ms: the in time) to the spring's period. A tagged node the plan drops stays on screen and moves back to that pose over
//! [`LAYER_OUT_MS`] before it is let go, except content changed in place ([`Enter::CONTENT`]), which goes at once so that a slot never
//! holds the old content and the new together. Only opacity, x, y and scale move, so the compositor eases layers it already holds. With
//! reduced motion both are a linear fade of at most `REDUCED_MAX_MS`.
use compositor_protocol::{Ease, Props};
use std::collections::BTreeMap;
use ui::lucent::tokens::motion::REDUCED_MAX_MS;

/// A layer leaves in 180 ms; the theme's `layerMs` is its 220 ms in.
pub const LAYER_OUT_MS: u32 = 180;
/// A section's content fades in in 180 ms, on the spot.
pub const CONTENT_MS: u32 = 180;
/// Focus rests this long on a game before the stage art changes (`parts::stage`).
pub const ART_REST_MS: u32 = 250;
/// One leg of the art's dip: it fades out, the next art takes its place, it fades in.
pub const ART_DIP_MS: u32 = 100;
/// A leaving layer is let go this long after its out ends, so a late frame never cuts the last of the fade.
pub const LEAVE_SLACK_MS: u64 = 17;

/// Where a node is away from its own pose, which it comes from and goes back to. It is always transparent there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enter {
    pub dx: f32,
    pub dy: f32,
    /// A factor of the node's own scale.
    pub scale: f32,
    /// Content that changes in place: it fades in over this and goes at once when the plan drops it. `None` takes the layer's in and out.
    pub fade_ms: Option<u32>,
}

impl Enter {
    /// Opacity only, over the layer's in and out.
    pub const FADE: Self = Self { dx: 0.0, dy: 0.0, scale: 1.0, fade_ms: None };
    /// Opacity only, in [`CONTENT_MS`]; it goes at once.
    pub const CONTENT: Self = Self { fade_ms: Some(CONTENT_MS), ..Self::FADE };
    pub const fn slide(dx: f32, dy: f32) -> Self {
        Self { dx, dy, ..Self::FADE }
    }
    pub const fn grow(scale: f32) -> Self {
        Self { scale, ..Self::FADE }
    }
}

/// One move onto the screen or off it: the pose that is away, how long it takes and how it eases.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub away: Props,
    pub ms: u32,
    pub ease: Ease,
}

fn away(rest: Props, enter: Enter, reduced: bool) -> Props {
    if reduced {
        Props { opacity: 0.0, ..rest }
    } else {
        Props { x: rest.x + enter.dx, y: rest.y + enter.dy, scale: rest.scale * enter.scale, opacity: 0.0, ..rest }
    }
}

/// How long a move on [`Ease::Spring`] takes for a theme time of `ms`: Lucent's spring has a response of 260 ms and is let go for 300
/// (`motion::SPRING_K`), so a time the theme gives for an ease-out becomes this many for the spring that LumaTV Home moves on.
pub const fn spring_ms(ms: u32) -> u32 {
    ms * 300 / 260
}

/// The move that brings a node to its pose `rest`: from away, springing in over `layer_ms`; content fades in over its own time, easing out.
pub fn arrive(rest: Props, enter: Enter, reduced: bool, layer_ms: u32) -> Move {
    let ms = enter.fade_ms.unwrap_or(layer_ms);
    if reduced {
        Move { away: away(rest, enter, true), ms: ms.min(REDUCED_MAX_MS), ease: Ease::Linear }
    } else if enter.fade_ms.is_some() {
        Move { away: away(rest, enter, false), ms, ease: Ease::Out }
    } else {
        Move { away: away(rest, enter, false), ms: spring_ms(ms), ease: Ease::Spring }
    }
}

/// The move that takes a node from its pose `rest` to away, easing in; none for content, which goes at once.
pub fn leave(rest: Props, enter: Enter, reduced: bool) -> Option<Move> {
    if enter.fade_ms.is_some() {
        return None;
    }
    if reduced {
        Some(Move { away: away(rest, enter, true), ms: LAYER_OUT_MS.min(REDUCED_MAX_MS), ease: Ease::Linear })
    } else {
        Some(Move { away: away(rest, enter, false), ms: LAYER_OUT_MS, ease: Ease::In })
    }
}

/// The nodes the plan dropped that are still on their way out, and when each is let go.
#[derive(Default)]
pub struct Leaving(BTreeMap<u64, u64>);

impl Leaving {
    pub fn begin(&mut self, id: u64, now_ms: u64, ms: u32) {
        self.0.insert(id, now_ms + u64::from(ms) + LEAVE_SLACK_MS);
    }
    /// The node is back in the plan: it stays. True when it was on its way out.
    pub fn stop(&mut self, id: u64) -> bool {
        self.0.remove(&id).is_some()
    }
    pub fn contains(&self, id: u64) -> bool {
        self.0.contains_key(&id)
    }
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.0.keys().copied()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    /// The nodes whose out has ended by `now_ms`; they are no longer tracked.
    pub fn due(&mut self, now_ms: u64) -> Vec<u64> {
        let due: Vec<u64> = self.0.iter().filter(|(_, until)| **until <= now_ms).map(|(id, _)| *id).collect();
        for id in &due {
            self.0.remove(id);
        }
        due
    }
    /// How long until the next node is let go.
    pub fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        self.0.values().min().map(|until| until.saturating_sub(now_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REST: Props = Props::at(100.0, 200.0);

    #[test]
    fn a_layer_springs_in_from_its_offset_and_leaves_in_its_own_time() {
        let enter = Enter { dx: 40.0, dy: -24.0, scale: 0.96, fade_ms: None };
        let come = arrive(REST, enter, false, 220);
        assert_eq!(come, Move { away: Props { x: 140.0, y: 176.0, scale: 0.96, opacity: 0.0, ..REST }, ms: spring_ms(220), ease: Ease::Spring });
        let go = leave(REST, enter, false).unwrap();
        assert_eq!((go.away, go.ms, go.ease), (come.away, LAYER_OUT_MS, Ease::In));
    }

    #[test]
    fn content_fades_in_place_and_goes_at_once() {
        let come = arrive(REST, Enter::CONTENT, false, 220);
        assert_eq!((come.away, come.ms), (Props { opacity: 0.0, ..REST }, CONTENT_MS));
        assert_eq!(leave(REST, Enter::CONTENT, false), None);
    }

    #[test]
    fn reduced_motion_is_a_short_linear_fade_that_moves_nothing() {
        let enter = Enter { dx: 40.0, dy: 24.0, scale: 0.9, fade_ms: None };
        for mv in [arrive(REST, enter, true, 220), leave(REST, enter, true).unwrap()] {
            assert_eq!(mv, Move { away: Props { opacity: 0.0, ..REST }, ms: REDUCED_MAX_MS, ease: Ease::Linear });
        }
    }

    #[test]
    fn a_leaver_is_let_go_after_its_out_and_one_that_returns_stays() {
        let mut leaving = Leaving::default();
        assert_eq!(leaving.wait_ms(0), None);
        leaving.begin(7, 1000, LAYER_OUT_MS);
        leaving.begin(8, 1000, LAYER_OUT_MS);
        let end = 1000 + u64::from(LAYER_OUT_MS);
        assert_eq!(leaving.wait_ms(1100), Some(end + LEAVE_SLACK_MS - 1100));
        assert!(leaving.due(end).is_empty(), "the out has not finished with its slack");
        assert!(leaving.stop(8) && !leaving.stop(8));
        assert_eq!(leaving.due(end + LEAVE_SLACK_MS), [7]);
        assert_eq!((leaving.contains(7), leaving.wait_ms(end + 99)), (false, None));
    }
}
