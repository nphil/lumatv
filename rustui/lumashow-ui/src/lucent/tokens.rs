//! Lucent's structural tokens (design section 3.3): space, radius, light, type, motion and state. These are the only copies
//! of these numbers in the program: the application's old theme and the lab's token file are gone, and every component and screen
//! reads them from here. Colours are not here: they are the palette's roles ([`super::data`]) after resolution.
//!
//! Sizes are logical units. The type roles are the smart-display profile's, the profile of the shell this toolkit is built for.

use crate::text::{Type, Weight};

/// The spacing steps: 4, 8, 12, 16, 20, 24, 32 and 40.
pub mod space {
    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 20.0;
    pub const S6: f32 = 24.0;
    pub const S8: f32 = 32.0;
    pub const S10: f32 = 40.0;
}

/// Corner radii: sheet, card, tile, row, control, and the pill that is as round as its height allows.
pub mod radius {
    pub const SHEET: f32 = 28.0;
    pub const CARD: f32 = 24.0;
    pub const TILE: f32 = 20.0;
    pub const ROW: f32 = 18.0;
    pub const CONTROL: f32 = 14.0;
    /// Larger than any control is high: the shape clamps it to half its height.
    pub const PILL: f32 = 999.0;
}

/// The light of the language: upper left, in screen space, with an edge one unit wide. The wire has no field for either; the
/// compositor's shader holds the direction, so these are for the tests and for the numbers a screen computes with.
pub mod light {
    pub const DIRECTION: (f32, f32) = (-0.65, -0.76);
    pub const EDGE_WIDTH: f32 = 1.0;
}

/// The smart-display profile: its type scale, its grid and its targets.
pub mod smart {
    /// The display's logical size on the Echo, and the grid the screens are laid out on.
    pub const WIDTH: f32 = 960.0;
    pub const HEIGHT: f32 = 480.0;
    pub const GUTTER: f32 = 16.0;
    pub const EDGE_X: f32 = 24.0;
    pub const EDGE_Y: f32 = 20.0;
    pub const ROW: f32 = 64.0;
    /// The smallest touch target (Lucent section 7).
    pub const TARGET: f32 = 64.0;
    /// The target of a frequent action on the Echo, derived from its measured density (design section 2.3).
    pub const FREQUENT_TARGET: f32 = 72.0;
    pub const DISPLAY_PX: u32 = 72;
    pub const TITLE_PX: u32 = 28;
    pub const BODY_PX: u32 = 20;
    pub const LABEL_PX: u32 = 18;
    pub const CAPTION_PX: u32 = 16;
    /// The home clock's numerals are as large as they fit, up to this.
    pub const CLOCK_MAX_PX: u32 = 144;
}

/// The Settings pages draw the toolkit's type at this many times (a row's label is 24, its detail and a heading 20, a value and a
/// button 22): the size the approved Settings mock-ups are set in. [`crate::Look::page_ty`] gives a mock's own pixel size at it.
pub const PAGE_SCALE: f32 = 1.2;

/// The type roles of the smart-display profile: Manrope, 400 body and caption, 500 display and label, 600 title.
pub mod ty {
    use super::{Type, Weight, smart};

    pub const DISPLAY: Type = Type {
        px: smart::DISPLAY_PX,
        weight: Weight::Medium,
    };
    pub const TITLE: Type = Type {
        px: smart::TITLE_PX,
        weight: Weight::SemiBold,
    };
    pub const BODY: Type = Type {
        px: smart::BODY_PX,
        weight: Weight::Regular,
    };
    pub const LABEL: Type = Type {
        px: smart::LABEL_PX,
        weight: Weight::Medium,
    };
    pub const CAPTION: Type = Type {
        px: smart::CAPTION_PX,
        weight: Weight::Regular,
    };
    /// Key legends: the title size at the label weight.
    pub const KEY: Type = Type {
        px: smart::TITLE_PX,
        weight: Weight::Medium,
    };
    /// A button's label: the body size at the label weight.
    pub const BUTTON: Type = Type {
        px: smart::BODY_PX,
        weight: Weight::Medium,
    };
}

/// Motion: the one set of rules (mock decision 35, 10-08). Lucent's durations and curves, the one spring, the one fling, the one
/// rubber band and the one commit rule are here and nowhere else; a screen or a component asks for them by name, and
/// `tests/motion_rules.rs` holds the rest of the program to that (no curve, spring, friction, cap, flick speed or share of a travel
/// written out anywhere else, and no copy of a Lucent duration).
pub mod motion {
    pub use compositor_protocol::motion::{
        RUBBER_FOLLOW, SPRING_K, ease, ease_f64, rubber, start_slope, unrubber,
    };
    use compositor_protocol::{Ease, MotionToken, ScrollToken, Settle, SpringToken};

    // ---- Lucent's durations (tokens.json `motion.*.ms`) ---------------------------------------------------------------------
    pub const PRESS_MS: u32 = 90;
    pub const LABEL_MS: u32 = 120;
    pub const FOCUS_MS: u32 = 150;
    pub const CARD_MS: u32 = 180;
    pub const LAYER_MS: u32 = 220;
    pub const EXIT_MS: u32 = 180;
    /// The longest an ordinary reveal takes.
    pub const SCROLL_MS: u32 = 300;
    /// With reduced motion a change is an opacity change, and no longer than this.
    pub const REDUCED_MAX_MS: u32 = 120;
    /// How long a reorderable thing shows how to be reordered before it is touched.
    pub const REORDER_DISCOVERY_MS: u32 = 600;
    /// The optional press-and-hold of a gesture: never the only way to do something.
    pub const HOLD_MS: u32 = 1500;

    // ---- the curves: Lucent's, and the compositor has no others -------------------------------------------------------------
    /// `--lu-ease`, `cubic-bezier(.33,1,.68,1)`: every arrival, settle and fade-in. The compositor evaluates the CSS curve itself
    /// ([`compositor_protocol::motion::ease`]), for its frames and for the estimates the app makes of them.
    pub const SETTLE: Ease = Ease::Out;
    /// `--lu-ease-exit`, `cubic-bezier(.4,0,1,1)`: everything that leaves from rest.
    pub const EXIT: Ease = Ease::In;
    /// Lucent's `motion.reduce` (a change of opacity and nothing else), a jump that has to take a frame, a hold's progress, and the
    /// departure of a thing flung away, which goes on at the speed of the finger that flung it.
    pub const LINEAR: Ease = Ease::Linear;
    /// The release of something held out of place (a rubber band, a card let go of short of leaving): the one spring
    /// ([`SPRING_MS`]) from rest, kept to [`SCROLL_MS`] ([`SPRING_K`] holds the two to each other: a test pins it).
    pub const RELEASE: Ease = Ease::Spring;
    pub const RELEASE_MS: u32 = SCROLL_MS;
    /// One half of an ambient pulse (a breathing ring, a listening dot): it swells on [`SETTLE`] and lets go on [`EXIT`], so the two
    /// halves meet at rest at the top.
    pub const fn pulse(swelling: bool) -> Ease {
        if swelling { SETTLE } else { EXIT }
    }

    // ---- what a track settles by ---------------------------------------------------------------------------------------------
    /// The response of the one critically damped spring (the day-line mock's ~260 ms): the dock, the day line, the shade's sheet and
    /// list, the pages, whatever a finger lets go of in the middle of a settle. Its release from a rubber band is [`RELEASE`].
    pub const SPRING_MS: f32 = 260.0;

    pub const fn spring() -> Settle {
        Settle::CriticalSpring(SpringToken {
            response_ms: SPRING_MS,
        })
    }

    /// A settle by a timed curve.
    pub const fn timed(ms: u32, ease: Ease) -> Settle {
        Settle::Timed(MotionToken {
            duration_ms: ms,
            ease,
        })
    }

    /// A settle that lands in the next frame: a jump on the compositor's own clock.
    pub const fn at_once() -> Settle {
        timed(1, LINEAR)
    }

    /// A timed settle that waits `delay_ms` on the compositor's clock before it moves (the parts of an entrance that come one after
    /// another, asked for in one commit).
    pub const fn timed_after(delay_ms: u32, ms: u32, ease: Ease) -> Settle {
        Settle::TimedAfter {
            delay_ms,
            token: MotionToken {
                duration_ms: ms,
                ease,
            },
        }
    }

    /// A settle of no length: the compositor lands it in the frame that carries it and draws no frame of its own for it (a step the
    /// day line takes as the clock turns a minute, which must not come between a finger's first moves and their frames).
    pub const fn no_length() -> Settle {
        timed(0, LINEAR)
    }

    /// A timed settle of `ms` on [`SETTLE`], or, for a person who asked for reduced motion, the plain fade Lucent makes of it: no
    /// longer than [`REDUCED_MAX_MS`], linear.
    pub const fn timed_or_reduced(ms: u32, reduced: bool) -> Settle {
        if reduced {
            timed(REDUCED_MAX_MS, LINEAR)
        } else {
            timed(ms, SETTLE)
        }
    }

    // ---- the fling (design 8.4) -----------------------------------------------------------------------------------------------
    /// Scroll inertia. A fling's speed decays exponentially with this time constant, so the glide lasts as long as the decay takes
    /// to bring the release speed down to the rest speed: a faster finger glides longer, and only logarithmically so. A glide planned
    /// up front on [`SETTLE`] covers the release speed times its duration over [`start_slope`] of that curve: at 1 px/ms 340 px in
    /// 1.0 s, at 3 px/ms 1,200 px in 1.2 s, about three quarters of what an iOS scroll view (0.998 per ms, a time constant of half a
    /// second) carries a finger of the same speed.
    pub const SCROLL_TAU_MS: f32 = 170.0;
    /// The speed (px per second) at which a fling has come to rest: half of the design's 5 px/s tolerance, as the compositor's own
    /// motion tracks rest.
    pub const SCROLL_REST_PX_PER_S: f32 = 2.5;
    /// A fling that nothing cuts short glides at least and at most this long.
    pub const FLING_MIN_MS: u32 = 300;
    pub const FLING_MAX_MS: u32 = 1_200;
    /// A fling that is cut short by the end of the content or by the rows it has glides at least this long, however little is left.
    pub const FLING_CUT_MIN_MS: u32 = 160;
    /// Slower than this (px per ms) at release and the content just stops where it is.
    pub const FLING_MIN_SPEED: f32 = 0.15;
    /// A finger that has not moved for this long before it lifts has stopped: no fling.
    pub const FLING_STILL_NS: u64 = 80_000_000;
    /// A finger that lands on a flight catches it: the layers glide to where the content had got, this long, so that nothing jumps.
    pub const CATCH_MS: u32 = 60;
    /// The friction (per second) of a fling that has all the room it wants: `1 / SCROLL_TAU_MS`.
    pub const FRICTION: f32 = 1000.0 / SCROLL_TAU_MS;
    /// The most a track's friction is raised to, so that a fling stops short of the end of what is made.
    pub const FRICTION_MAX: f32 = 50.0;

    /// The friction of a track's fling let go at `speed` (units a second) with `room` units of what is made ahead of it: the glide's
    /// own, and as much more as keeps the fling (which goes `speed / friction` far) to 85 % of the room.
    pub fn fling_friction(speed: f32, room: f32) -> f32 {
        (speed.abs() / (room * 0.85).max(1.0)).clamp(FRICTION, FRICTION_MAX)
    }

    /// A track's settle by scroll inertia with `friction`, the spring [`SPRING_MS`] bringing it home from an end.
    pub const fn fling(friction: f32) -> Settle {
        Settle::Scroll(ScrollToken {
            friction,
            spring: SpringToken {
                response_ms: SPRING_MS,
            },
        })
    }

    // ---- the rubber band: `RUBBER_FOLLOW` of the pull at the end, never more than the cap -----------------------------------
    /// The farthest content is ever shown past an end of its range, in displayed pixels (design 8.4): the cap of [`rubber`] for a list,
    /// a pager, a sheet and the day line (in its own units, which are scaled), where the track is.
    pub const OVERSCROLL_PX: f32 = 32.0;
    /// How far a card that cannot be dismissed gives to a finger (the shade mock's `Kit.rubber(|dx|, 16)`).
    pub const GIVE_PX: f32 = 16.0;
    /// How far the dock, which rests against the bottom edge, may be pulled past shown, of its own travel.
    pub const DOCK_GIVE: f32 = 0.04;
    /// How far past the zoom limits a pinch may take the day line, in the log of its pixels an hour (the day-line mock's `softLog`).
    pub const ZOOM_GIVE: f32 = 0.22;

    // ---- what lifting a finger decides ---------------------------------------------------------------------------------------
    /// One rule for every swipe that may go on or come back (a page that leaves, a sheet, a card, a pager's page): it goes on when it
    /// has travelled `share` of its extent toward its end, or when the finger lifts at `flick` or faster that way (with at least
    /// `min_travel` behind it, so that a twitch is no flick); a flick the other way, at `flick` or faster, brings it back however far
    /// it had come. The presets are the approved mock-ups' own numbers (the speeds are in units a second of the screen).
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Commit {
        pub share: f32,
        pub flick: f32,
        pub min_travel: f32,
    }

    impl Commit {
        /// Whether a release that has travelled `travel` toward the end of a swipe `extent` long, moving at `speed` that way (both
        /// negative for the way back), goes on to the end.
        pub fn decides(self, travel: f32, extent: f32, speed: f32) -> bool {
            if speed <= -self.flick {
                return false;
            }
            travel >= self.share * extent || (speed >= self.flick && travel >= self.min_travel)
        }

        /// Which end a track between two (`0` and `1`, `extent` units long) is let go of at `value` and `velocity` (of the track a
        /// second, positive toward 1) comes to rest at: the end a flick points to, otherwise the nearer one. `true` for 1.
        pub fn nearer_end(self, value: f32, velocity: f32, extent: f32) -> bool {
            let speed = velocity * extent;
            if speed >= self.flick {
                true
            } else if speed <= -self.flick {
                false
            } else {
                value >= 1.0 - self.share
            }
        }

        /// Which way a pager is turned by a finger that travelled `travel` (positive: forward) and lifts moving `speed` (the same
        /// sign): `1` for a page on, `-1` for one back, `0` for the page it was on.
        pub fn turn(self, travel: f32, extent: f32, speed: f32) -> i32 {
            if self.decides(travel, extent, speed) {
                1
            } else if self.decides(-travel, extent, -speed) {
                -1
            } else {
                0
            }
        }
    }

    /// The presets: where a number is an approved mock-up's it is that mock-up's.
    pub mod commit {
        use super::Commit;

        /// A sheet pulled from its edge (the shade mock: half the way, or 220 px/s: `0.5` of its 440 a second).
        pub const SHEET: Commit = Commit {
            share: 0.5,
            flick: 220.0,
            min_travel: 12.0,
        };
        /// A pager's page (the live-cards mock: 0.22 of a page, or 350 px/s, never more than one page).
        pub const PAGER: Commit = Commit {
            share: 0.22,
            flick: 350.0,
            min_travel: 12.0,
        };
        /// A card swiped away, and a page that follows the finger as it is swiped back from the edge (the shade mock: 0.38 of its
        /// width, or 650 px/s, and 24 px of travel to count a flick).
        pub const DISMISS: Commit = Commit {
            share: 0.38,
            flick: 650.0,
            min_travel: 24.0,
        };
        /// The dock, which a finger shows and hides: half its travel, and a flick of 0.8 of its 112 units a second (tuned on the Show).
        pub const DOCK: Commit = Commit {
            share: 0.5,
            flick: 90.0,
            min_travel: 0.0,
        };
        /// A deep screen carried home from the bottom edge: goes at 0.35 of its 300 units of travel, or 1.4 of them a second (tuned
        /// on the Show).
        pub const LEAVE: Commit = Commit {
            share: 0.35,
            flick: 420.0,
            min_travel: 0.0,
        };
        /// The back swipe from the left edge that has no page following the finger (the gesture recogniser's `Back`): the 90 of the
        /// display's 960 units it always took, or 650 px/s with 24 px of travel. A page that does follow the finger is a card
        /// ([`DISMISS`]).
        pub const BACK_SWIPE: Commit = Commit {
            share: 90.0 / 960.0,
            flick: 650.0,
            min_travel: 24.0,
        };
    }
}

/// State coefficients (section 3.3, 3.4).
pub mod state {
    /// A large actionable group, and a slider or toggle thumb, scales to this about its centre while pressed.
    pub const PRESSED_SCALE: f32 = 0.97;
    /// The focus wash: the ink at this alpha over the surface, dark and light.
    pub const FOCUS_WASH_DARK: f64 = 0.07;
    pub const FOCUS_WASH_LIGHT: f64 = 0.06;
    /// The selected wash: the accent at this alpha over the raised glass.
    pub const SELECTED_WASH: f64 = 0.04;
    /// A focus landing cue is a ring this wide, in units, and at least 3:1 against what it sits on.
    pub const FOCUS_CUE_WIDTH: f32 = 2.0;
    /// The coefficient of a disabled control's icon.
    pub const DISABLED_ICON: f32 = 0.55;
}

/// Layer stacking bands, named and allocated here so that screens and overlays never fight over order. Layers of one `z` stack in
/// creation order, so a part builds its glass first and its text last inside its own band; a component takes its own `z` and the
/// few above it.
pub mod z {
    pub const BACKGROUND: i32 = 0;
    pub const CARD: i32 = 10;
    pub const CONTENT: i32 = 20;
    pub const CHROME: i32 = 30;
    /// The cover that fades out over a newly opened screen.
    pub const CURTAIN: i32 = 99;
    /// The night clock's face: a black plate over everything the shell draws, with the time, the date and the next alarm on it (layers
    /// `NIGHT` to `NIGHT + 3`). Above every screen of the host, under what a person switched on to look at.
    pub const NIGHT: i32 = 500;
    /// What a person switched on to look at over every screen: the frame counter, the debug toast.
    pub const READOUT: i32 = 1000;
}

/// Contrast the design requires after composition (section 10.1).
pub mod contrast {
    /// Body text.
    pub const BODY: f64 = 4.5;
    /// Glance text: the clock, titles, dock labels.
    pub const GLANCE: f64 = 7.0;
    /// The boundary of a control, and a focus cue.
    pub const CONTROL: f64 = 3.0;
}

/// The least alpha of a sheet's and of an overlay's tint, so that text stays readable over any wallpaper.
pub mod floor {
    pub const SHEET: f64 = 0.88;
    pub const OVERLAY: f64 = 0.96;
    /// A thumb is nearly opaque so that it reads as a physical piece on its track.
    pub const THUMB: f64 = 0.96;
}

/// The glass of a card and a timeline chip (design 3.4.1, `GC`): the deeper glass that the raised glass of a control lies on.
pub mod card {
    /// The alpha of a dark variant's card glass. The approved mock-ups draw a card as `rgba(24,21,32,.62)` (`live-cards/app.css` `.lc`;
    /// `mock-kit/kit.css` `.m-card` .58 and `dayline.css` `.dl-chip` .64 round about it): `GR`'s own premultiplied colour (Rosé Pine's
    /// `#2C263A8F` is 24, 21, 32), read as a straight colour, at this alpha.
    pub const GLASS_ALPHA: f64 = 0.62;
}

/// A toggle, as the approved mock-ups draw it (`.sw` in `ux-prototype/settings-*` and `mock-kit/kit.css`; Nitin's decision 36, 10-08): a
/// track of 68 by 40, a knob of 30 five in from the track's ends that travels 28, a tick of 18 on the knob while it is on, and, round an
/// off track, a ring two wide. The knob is light (the ink in a dark variant, white in a light one) and casts the mock's small shadow,
/// `1px 2px 4px rgba(0,0,0,.35)`.
pub mod switch {
    pub const WIDTH: f32 = 68.0;
    pub const HEIGHT: f32 = 40.0;
    pub const KNOB: f32 = 30.0;
    /// The knob's distance from the track's ends and from its top.
    pub const INSET: f32 = 5.0;
    /// How far the knob travels from one end to the other.
    pub const TRAVEL: f32 = WIDTH - 2.0 * INSET - KNOB;
    /// The tick on the knob.
    pub const TICK: f32 = 18.0;
    /// The ring round an off track, in the readable track's colour (the mock draws it in the ink at .55; the track is the ink at .60 or
    /// more, raised until it is 3:1 against its sheet on every picture, which the appearance fit proves, and a ring that is no weaker
    /// holds the same boundary).
    pub const RING_WIDTH: f32 = 2.0;
    /// The knob's cast shadow: down by 2, blurred by 4, in black at this alpha (the mock's offset of 1 to the right is not drawn).
    pub const SHADOW_OFFSET: f32 = 2.0;
    pub const SHADOW_BLUR: f32 = 4.0;
    pub const SHADOW_ALPHA: f64 = 0.35;
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::Ease;

    #[test]
    fn the_smart_type_roles_are_the_floors_the_design_names() {
        assert_eq!(
            [
                ty::DISPLAY.px,
                ty::TITLE.px,
                ty::BODY.px,
                ty::LABEL.px,
                ty::CAPTION.px
            ],
            [72, 28, 20, 18, 16]
        );
        assert_eq!(ty::TITLE.weight, Weight::SemiBold);
        assert_eq!(ty::BODY.weight, Weight::Regular);
        assert_eq!(ty::KEY.px, smart::TITLE_PX);
    }

    #[test]
    fn a_reduced_motion_change_is_never_longer_than_an_ordinary_one_and_press_is_the_shortest() {
        const { assert!(motion::REDUCED_MAX_MS <= motion::LABEL_MS) };
        let all = [
            motion::PRESS_MS,
            motion::LABEL_MS,
            motion::FOCUS_MS,
            motion::CARD_MS,
            motion::LAYER_MS,
            motion::EXIT_MS,
            motion::SCROLL_MS,
        ];
        assert_eq!(all.iter().min(), Some(&motion::PRESS_MS));
    }

    #[test]
    fn the_durations_are_lucents_and_the_curves_are_the_four_the_compositor_has() {
        // tokens.json `motion.*.ms`, as Lucent v2 has them.
        assert_eq!(
            [
                motion::PRESS_MS,
                motion::LABEL_MS,
                motion::FOCUS_MS,
                motion::CARD_MS,
                motion::LAYER_MS,
                motion::EXIT_MS,
                motion::SCROLL_MS,
                motion::REDUCED_MAX_MS,
                motion::REORDER_DISCOVERY_MS,
                motion::HOLD_MS,
            ],
            [90, 120, 150, 180, 220, 180, 300, 120, 600, 1500]
        );
        assert_eq!(motion::SETTLE, Ease::Out);
        assert_eq!(motion::EXIT, Ease::In);
        assert_eq!(motion::LINEAR, Ease::Linear);
        assert_eq!(motion::RELEASE, Ease::Spring);
        assert_eq!(motion::pulse(true), motion::SETTLE);
        assert_eq!(motion::pulse(false), motion::EXIT);
    }

    #[test]
    fn the_springs_release_is_the_one_spring_kept_to_the_scroll_time() {
        // `Ease::Spring`'s shape is a critically damped spring of response `SPRING_MS` that has the scroll time to land in.
        assert_eq!(motion::RELEASE_MS, motion::SCROLL_MS);
        let k =
            std::f64::consts::TAU * f64::from(motion::RELEASE_MS) / f64::from(motion::SPRING_MS);
        assert!((k - motion::SPRING_K).abs() < 0.01, "{k}");
        // And the rubber band's most (32 px) is home within a quarter of a pixel of it by then.
        let omega = std::f64::consts::TAU / (f64::from(motion::SPRING_MS) / 1000.0);
        let t = f64::from(motion::RELEASE_MS) / 1000.0;
        let left = f64::from(motion::OVERSCROLL_PX) * (1.0 + omega * t) * (-omega * t).exp();
        assert!(left < 0.25, "{left}");
        // Its friction is the glide's time constant the other way up.
        assert!((motion::FRICTION * motion::SCROLL_TAU_MS - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn what_the_settle_helpers_build_is_what_they_are_called() {
        use compositor_protocol::{MotionToken, ScrollToken, Settle, SpringToken};
        let timed = |duration_ms, ease| Settle::Timed(MotionToken { duration_ms, ease });
        assert_eq!(motion::timed(220, Ease::Out), timed(220, Ease::Out));
        assert_eq!(motion::at_once(), timed(1, Ease::Linear));
        assert_eq!(motion::no_length(), timed(0, Ease::Linear));
        assert_eq!(motion::timed_or_reduced(220, false), timed(220, Ease::Out));
        assert_eq!(
            motion::timed_or_reduced(220, true),
            timed(motion::REDUCED_MAX_MS, Ease::Linear)
        );
        assert_eq!(
            motion::spring(),
            Settle::CriticalSpring(SpringToken { response_ms: 260.0 })
        );
        assert_eq!(
            motion::fling(7.0),
            Settle::Scroll(ScrollToken {
                friction: 7.0,
                spring: SpringToken { response_ms: 260.0 }
            })
        );
        assert_eq!(
            motion::timed_after(50, 280, Ease::Out),
            Settle::TimedAfter {
                delay_ms: 50,
                token: MotionToken {
                    duration_ms: 280,
                    ease: Ease::Out
                }
            }
        );
    }

    #[test]
    fn a_fling_with_room_glides_as_long_as_the_shared_inertia_and_one_without_is_made_to_stop_short()
     {
        // All the room it wants: the glide's own friction, however slow the finger.
        assert_eq!(motion::fling_friction(300.0, 100_000.0), motion::FRICTION);
        assert_eq!(motion::fling_friction(-300.0, 100_000.0), motion::FRICTION);
        // It goes `speed / friction` far: in 85 % of the room when the room is short, and no friction is more than the most.
        let (speed, room) = (3000.0, 200.0);
        let friction = motion::fling_friction(speed, room);
        assert!((speed / friction - room * 0.85).abs() < 1e-2 || friction == motion::FRICTION_MAX);
        assert_eq!(motion::fling_friction(speed, 10.0), motion::FRICTION_MAX);
    }

    #[test]
    fn the_rubber_bands_caps_and_the_follow_are_the_designs() {
        assert_eq!(motion::RUBBER_FOLLOW, 0.35);
        assert_eq!(motion::OVERSCROLL_PX, 32.0);
        assert_eq!(motion::GIVE_PX, 16.0);
        // A band with a cap shows less than the cap however far, and the same pull shows less with a smaller one.
        assert!(motion::rubber(1000.0, motion::GIVE_PX) <= motion::GIVE_PX);
        assert!(
            motion::rubber(40.0, motion::GIVE_PX) < motion::rubber(40.0, motion::OVERSCROLL_PX)
        );
    }

    #[test]
    fn the_commit_presets_are_the_approved_mock_ups_numbers() {
        use motion::commit::{DISMISS, DOCK, LEAVE, PAGER, SHEET};
        assert_eq!(smart::WIDTH, 960.0);
        // The shade's mock: half the way or 220 px/s for the sheet; 0.38 of the width or 650 px/s (24 px of travel) for a card.
        assert_eq!((SHEET.share, SHEET.flick), (0.5, 220.0));
        assert_eq!(
            (DISMISS.share, DISMISS.flick, DISMISS.min_travel),
            (0.38, 650.0, 24.0)
        );
        // The live-cards mock: 0.22 of a page or 350 px/s.
        assert_eq!((PAGER.share, PAGER.flick), (0.22, 350.0));
        // What was tuned on the Show, in the units of the track each is: the dock's 0.8 of its 112 a second, the leave's 0.35 of
        // its 300 and 1.4 of them a second.
        assert_eq!((DOCK.share, DOCK.flick), (0.5, 90.0));
        assert!((DOCK.flick - 0.8 * 112.0).abs() < 0.5);
        assert_eq!((LEAVE.share, LEAVE.flick), (0.35, 1.4 * 300.0));
        // The discrete back swipe from the edge keeps the 90 of the display's 960 units it always took, and a flick as a card's.
        assert_eq!(
            (
                motion::commit::BACK_SWIPE.share * smart::WIDTH,
                motion::commit::BACK_SWIPE.flick
            ),
            (90.0, DISMISS.flick)
        );
    }

    #[test]
    fn a_swipe_commits_at_its_share_or_its_flick_and_a_flick_back_cancels_it_at_every_boundary() {
        for rule in [
            motion::commit::SHEET,
            motion::commit::PAGER,
            motion::commit::DISMISS,
            motion::commit::DOCK,
            motion::commit::LEAVE,
            motion::commit::BACK_SWIPE,
        ] {
            let extent = 400.0;
            let edge = rule.share * extent;
            // By distance: not a hair before its share, at it and past it.
            assert!(!rule.decides(edge - 0.5, extent, 0.0), "{rule:?}");
            assert!(rule.decides(edge + 0.5, extent, 0.0), "{rule:?}");
            // By a flick: at the flick's speed with the travel to count it, not a hair under it, and not before the travel counts.
            let travel = rule.min_travel.max(1.0);
            assert!(rule.decides(travel, extent, rule.flick), "{rule:?}");
            assert!(!rule.decides(travel, extent, rule.flick - 0.5), "{rule:?}");
            if rule.min_travel > 0.0 {
                assert!(
                    !rule.decides(rule.min_travel - 0.5, extent, rule.flick),
                    "{rule:?}"
                );
            }
            // A flick the other way, at the flick's speed, brings it back however far it had come; slower does not.
            assert!(!rule.decides(edge + 50.0, extent, -rule.flick), "{rule:?}");
            assert!(
                rule.decides(edge + 50.0, extent, -rule.flick + 0.5),
                "{rule:?}"
            );
        }
    }

    #[test]
    fn a_track_between_two_ends_goes_to_the_nearer_or_the_way_a_flick_points_and_a_pager_turns_one_page()
     {
        let sheet = motion::commit::SHEET;
        // Half the way is the line; a flick (0.5 of the track a second over 440 units is 220) overrides in both directions.
        assert!(sheet.nearer_end(0.5, 0.0, 440.0));
        assert!(!sheet.nearer_end(0.499, 0.0, 440.0));
        assert!(sheet.nearer_end(0.1, 0.5, 440.0));
        assert!(!sheet.nearer_end(0.9, -0.5, 440.0));
        assert!(
            sheet.nearer_end(0.9, -0.49, 440.0),
            "just under a flick it stays at the end it is nearer to"
        );
        let pager = motion::commit::PAGER;
        assert_eq!(pager.turn(-300.0, 480.0, 0.0), -1);
        assert_eq!(pager.turn(300.0, 480.0, 0.0), 1);
        assert_eq!(pager.turn(50.0, 480.0, 0.0), 0);
        assert_eq!(pager.turn(50.0, 480.0, 400.0), 1);
        assert_eq!(pager.turn(-50.0, 480.0, -400.0), -1);
        assert_eq!(
            pager.turn(300.0, 480.0, -400.0),
            0,
            "a flick back is no turn either way"
        );
    }
}
