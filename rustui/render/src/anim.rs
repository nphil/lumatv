//! Animation of layer properties along the curves of Lucent's motion language. Animations run on the compositor's clock, so an
//! app hands over a target and a duration once and costs nothing per frame afterwards. The curves themselves are
//! `compositor_protocol::motion::ease`'s: one evaluation, for these frames and for the app's estimates of them.

use compositor_protocol::motion::ease;
use compositor_protocol::{Ease, Edges, Props, Rect};

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A clip that is `None` on one side is read as the whole screen there, so a layer can be revealed by animating
/// its clip from "no clip" to a rectangle (or back) and the edge moves continuously.
fn lerp_clip(from: Option<Rect>, to: Option<Rect>, t: f32, screen: Rect) -> Option<Rect> {
    if from.is_none() && to.is_none() {
        return None;
    }
    let a = from.unwrap_or(screen);
    let b = to.unwrap_or(screen);
    Some(Rect {
        x: lerp(a.x, b.x, t),
        y: lerp(a.y, b.y, t),
        w: lerp(a.w, b.w, t),
        h: lerp(a.h, b.h, t),
    })
}

/// The feather of the two ends' clips, moved between: an end with no clip has none (the clip grows out of the whole screen,
/// hard-edged there), so a feathered clip that is revealed from "no clip" fades in its edges as it narrows.
fn lerp_feather(from: &Props, to: &Props, t: f32) -> Edges {
    let of = |p: &Props| {
        if p.clip.is_some() {
            p.feather
        } else {
            Edges::NONE
        }
    };
    let (a, b) = (of(from), of(to));
    Edges {
        left: lerp(a.left, b.left, t),
        right: lerp(a.right, b.right, t),
        top: lerp(a.top, b.top, t),
        bottom: lerp(a.bottom, b.bottom, t),
    }
}

/// One layer animation, from where the layer was when the request was applied to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animation {
    from: Props,
    to: Props,
    start_ns: u64,
    duration_ns: u64,
    curve: Ease,
    /// The least time between the frames the motion asks for: 0 for a motion drawn at every refresh.
    step_ns: u64,
}

impl Animation {
    pub fn new(from: Props, to: Props, start_ns: u64, duration_ms: u32, curve: Ease) -> Self {
        Self {
            from,
            to,
            start_ns,
            duration_ns: u64::from(duration_ms) * 1_000_000,
            curve,
            step_ns: 0,
        }
    }

    /// The same animation as an ambient motion that asks for a frame at most every `step_ms` (`Request::Animate::step_ms`).
    pub fn stepped(self, step_ms: u32) -> Self {
        Self {
            step_ns: u64::from(step_ms) * 1_000_000,
            ..self
        }
    }

    /// Whether it is an ambient motion: it asks for a frame every [`Animation::step_ns`] and no sooner.
    pub fn is_paced(&self) -> bool {
        self.step_ns > 0
    }

    pub fn step_ns(&self) -> u64 {
        self.step_ns
    }

    /// When the animation lands.
    pub fn ends_at(&self) -> u64 {
        self.start_ns.saturating_add(self.duration_ns)
    }

    pub fn is_done(&self, now_ns: u64) -> bool {
        self.duration_ns == 0 || now_ns >= self.start_ns.saturating_add(self.duration_ns)
    }

    /// The same animation between the same two states with each carried through `f` (a layer moved from one group's space
    /// to another's keeps its animation, in the new space).
    pub fn map_props(self, f: impl Fn(&Props) -> Props) -> Self {
        Self {
            from: f(&self.from),
            to: f(&self.to),
            ..self
        }
    }

    /// Where the animation lands.
    pub fn target(&self) -> Props {
        self.to
    }

    /// The layer's properties at `now_ns`. `x`, `y`, `scale` and `opacity` follow the curve; the clip and its feather move
    /// between their two ends; `visible` is on for as long as either end shows the layer, so both a fade-in from
    /// hidden and a fade-out to hidden play out.
    pub fn props_at(&self, now_ns: u64, screen: Rect) -> Props {
        if self.is_done(now_ns) {
            return self.to;
        }
        let elapsed = now_ns.saturating_sub(self.start_ns);
        let linear = (elapsed as f64 / self.duration_ns as f64) as f32;
        let t = ease(self.curve, linear);
        Props {
            x: lerp(self.from.x, self.to.x, t),
            y: lerp(self.from.y, self.to.y, t),
            scale: lerp(self.from.scale, self.to.scale, t),
            opacity: lerp(self.from.opacity, self.to.opacity, t),
            clip: lerp_clip(self.from.clip, self.to.clip, t, screen),
            feather: lerp_feather(&self.from, &self.to, t),
            visible: self.from.visible || self.to.visible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 960.0,
        h: 480.0,
    };
    const MS: u64 = 1_000_000;

    #[test]
    fn an_animation_follows_the_shared_curve_it_was_asked_for_at_every_moment() {
        // Position, opacity and scale all take the one evaluation of the curve (`compositor_protocol::motion::ease`): the app
        // estimates a layer's place with the same function, so a frame of the compositor is where the app expects it.
        let from = Props {
            opacity: 0.0,
            ..Props::at(0.0, 0.0)
        };
        let to = Props {
            scale: 2.0,
            ..Props::at(100.0, 200.0)
        };
        for curve in [Ease::Linear, Ease::Out, Ease::In, Ease::Spring] {
            let a = Animation::new(from, to, 0, 400, curve);
            for ms in [0u64, 1, 40, 100, 200, 301, 399, 400] {
                let at = a.props_at(ms * MS, SCREEN);
                let t = ease(curve, ms as f32 / 400.0);
                assert!((at.x - 100.0 * t).abs() < 1e-3, "{curve:?} x at {ms} ms");
                assert!((at.y - 200.0 * t).abs() < 1e-3, "{curve:?} y at {ms} ms");
                assert!(
                    (at.opacity - t).abs() < 1e-5,
                    "{curve:?} opacity at {ms} ms"
                );
                assert!(
                    (at.scale - (1.0 + t)).abs() < 1e-5,
                    "{curve:?} scale at {ms} ms"
                );
            }
        }
    }

    #[test]
    fn position_and_opacity_follow_the_curve_and_land_exactly_on_the_target() {
        let from = Props {
            opacity: 0.0,
            ..Props::at(10.0, 20.0)
        };
        let to = Props {
            scale: 2.0,
            ..Props::at(110.0, 220.0)
        };
        let a = Animation::new(from, to, 1000 * MS, 200, Ease::Linear);
        assert_eq!(a.props_at(1000 * MS, SCREEN), from);
        let mid = a.props_at(1100 * MS, SCREEN);
        assert_eq!(
            (mid.x, mid.y, mid.scale, mid.opacity),
            (60.0, 120.0, 1.5, 0.5)
        );
        assert!(!a.is_done(1199 * MS));
        assert!(a.is_done(1200 * MS));
        assert_eq!(a.props_at(1200 * MS, SCREEN), to);
        assert_eq!(a.props_at(5000 * MS, SCREEN), to);
    }

    #[test]
    fn a_layer_shows_for_the_whole_of_a_fade_in_and_a_fade_out() {
        let shown = Props::at(0.0, 0.0);
        let hidden = Props {
            visible: false,
            ..shown
        };
        let fade_in = Animation::new(hidden, shown, 0, 100, Ease::Linear);
        assert!(fade_in.props_at(0, SCREEN).visible);
        let fade_out = Animation::new(shown, hidden, 0, 100, Ease::Linear);
        assert!(fade_out.props_at(99 * MS, SCREEN).visible);
        assert!(!fade_out.props_at(100 * MS, SCREEN).visible);
    }

    #[test]
    fn a_clip_animates_from_the_whole_screen_when_one_end_has_none() {
        let hole = Rect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        let from = Props::at(0.0, 0.0);
        let to = Props {
            clip: Some(hole),
            ..from
        };
        let a = Animation::new(from, to, 0, 100, Ease::Linear);
        let mid = a.props_at(50 * MS, SCREEN).clip.unwrap();
        assert_eq!((mid.x, mid.y, mid.w, mid.h), (50.0, 50.0, 580.0, 290.0));
        assert_eq!(a.props_at(100 * MS, SCREEN).clip, Some(hole));
        let back = Animation::new(to, from, 0, 100, Ease::Linear);
        assert_eq!(back.props_at(100 * MS, SCREEN).clip, None);
        let none = Animation::new(from, from, 0, 100, Ease::Linear);
        assert_eq!(none.props_at(50 * MS, SCREEN).clip, None);
    }

    #[test]
    fn a_feather_animates_with_its_clip_and_lands_exactly_on_it() {
        let hole = Rect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        let plain = Props::at(0.0, 0.0);
        let feathered = plain.clip_feathered(hole, Edges::new(10.0, 20.0, 30.0, 40.0));
        // From no clip to a feathered one: the feather grows from nothing as the clip closes in.
        let a = Animation::new(plain, feathered, 0, 100, Ease::Linear);
        assert_eq!(a.props_at(0, SCREEN).feather, Edges::NONE);
        assert_eq!(
            a.props_at(50 * MS, SCREEN).feather,
            Edges::new(5.0, 10.0, 15.0, 20.0)
        );
        assert_eq!(a.props_at(100 * MS, SCREEN), feathered);
        // Between two clips, both ends' feathers are used.
        let wider = plain.clip_feathered(hole, Edges::horizontal(30.0));
        let b = Animation::new(feathered, wider, 0, 100, Ease::Linear);
        assert_eq!(
            b.props_at(50 * MS, SCREEN).feather,
            Edges::new(20.0, 25.0, 15.0, 20.0)
        );
        // A feather that belongs to no clip is nothing, at either end.
        let stray = Props {
            feather: Edges::all(9.0),
            ..plain
        };
        let c = Animation::new(stray, plain, 0, 100, Ease::Linear);
        assert_eq!(c.props_at(50 * MS, SCREEN).feather, Edges::NONE);
    }

    #[test]
    fn a_zero_length_animation_is_already_at_its_target() {
        let to = Props::at(5.0, 5.0);
        let a = Animation::new(Props::at(0.0, 0.0), to, 10, 0, Ease::Out);
        assert!(a.is_done(0));
        assert_eq!(a.props_at(0, SCREEN), to);
    }
}
