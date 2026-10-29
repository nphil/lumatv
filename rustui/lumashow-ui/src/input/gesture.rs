//! Turns raw contacts into what the shell acts on. Only the first finger down drives gestures; a second
//! finger is ignored until every finger has lifted, so a stray palm cannot turn a drag into a tap.

use super::{TouchEvent, TouchPhase};
use crate::lucent::tokens::motion::commit;
use crate::lucent::tokens::smart;
use compositor_protocol::motion::{VELOCITY_WINDOW_NS, slope_per_s};

/// Farther than this from the down point and a contact is a drag, not a tap.
const TAP_SLOP: f32 = 24.0;
/// Longer than this and a still contact is not a tap.
const TAP_MS: u64 = 700;
/// A back swipe starts this close to the left edge, goes mostly horizontally, and goes on by the one commit rule
/// ([`commit::BACK_SWIPE`]): 90 units of the display's width, or a flick.
const EDGE_BAND: f32 = 40.0;
/// The samples a back swipe's speed is the slope of.
const KEPT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// A finger went down (sent before anything else, for press feedback and dragging controls).
    Down {
        x: f32,
        y: f32,
    },
    /// The finger moved; `from` is where it went down.
    Drag {
        from: (f32, f32),
        x: f32,
        y: f32,
    },
    /// Lifted after a drag.
    Release {
        x: f32,
        y: f32,
    },
    Tap {
        x: f32,
        y: f32,
    },
    /// Swiped in from the left edge: go back.
    Back,
    /// The contact was lost (the kernel dropped events); undo any press state.
    Cancel,
}

#[derive(Default)]
pub struct Recognizer {
    active: Option<Contact>,
    /// A second finger was seen; ignore everything until all fingers lift.
    spoiled: bool,
    fingers: u32,
}

struct Contact {
    id: u32,
    start: (f32, f32),
    start_ns: u64,
    moved: bool,
    edge: bool,
    /// The newest samples along x, (time, x), oldest first: what a back swipe's speed at the lift is the slope of.
    recent: [(u64, f32); KEPT],
    count: usize,
}

impl Contact {
    fn note(&mut self, at_ns: u64, x: f32) {
        if self.count == KEPT {
            self.recent.copy_within(1.., 0);
            self.count -= 1;
        }
        self.recent[self.count] = (at_ns, x);
        self.count += 1;
    }

    /// How fast the finger was going right when it lifted, units a second (the slope of the last 50 ms, the estimator every release
    /// uses).
    fn speed(&self) -> f32 {
        slope_per_s(
            self.recent[..self.count].iter().copied(),
            VELOCITY_WINDOW_NS,
        )
    }
}

impl Recognizer {
    pub fn feed(&mut self, e: &TouchEvent) -> Option<Gesture> {
        match e.phase {
            TouchPhase::Down => {
                self.fingers += 1;
                if self.active.is_some() {
                    self.spoiled = true;
                    return Some(Gesture::Cancel);
                }
                if self.spoiled {
                    return None;
                }
                self.active = Some(Contact {
                    id: e.id,
                    start: (e.x, e.y),
                    start_ns: e.at_ns,
                    moved: false,
                    edge: e.x < EDGE_BAND,
                    recent: [(e.at_ns, e.x); KEPT],
                    count: 1,
                });
                Some(Gesture::Down { x: e.x, y: e.y })
            }
            TouchPhase::Move => {
                let c = self
                    .active
                    .as_mut()
                    .filter(|c| c.id == e.id && !self.spoiled)?;
                c.note(e.at_ns, e.x);
                if !c.moved && dist(c.start, (e.x, e.y)) > TAP_SLOP {
                    c.moved = true;
                }
                c.moved.then_some(Gesture::Drag {
                    from: c.start,
                    x: e.x,
                    y: e.y,
                })
            }
            TouchPhase::Up => {
                self.fingers = self.fingers.saturating_sub(1);
                let ours = self.active.as_ref().is_some_and(|c| c.id == e.id);
                let result = if ours && !self.spoiled {
                    let mut c = self.active.take().expect("checked above");
                    c.note(e.at_ns, e.x);
                    let (dx, dy) = (e.x - c.start.0, e.y - c.start.1);
                    if c.edge
                        && dy.abs() < dx * 0.6
                        && commit::BACK_SWIPE.decides(dx, smart::WIDTH, c.speed())
                    {
                        Some(Gesture::Back)
                    } else if !c.moved && e.at_ns.saturating_sub(c.start_ns) < TAP_MS * 1_000_000 {
                        Some(Gesture::Tap {
                            x: c.start.0,
                            y: c.start.1,
                        })
                    } else {
                        Some(Gesture::Release { x: e.x, y: e.y })
                    }
                } else {
                    if ours {
                        self.active = None;
                    }
                    None
                };
                if self.fingers == 0 {
                    self.spoiled = false;
                    self.active = None;
                }
                result
            }
            TouchPhase::Cancel => {
                *self = Self::default();
                Some(Gesture::Cancel)
            }
        }
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: u32, phase: TouchPhase, x: f32, y: f32, ms: u64) -> TouchEvent {
        TouchEvent {
            id,
            phase,
            x,
            y,
            at_ns: ms * 1_000_000,
        }
    }

    fn run(events: &[TouchEvent]) -> Vec<Gesture> {
        let mut r = Recognizer::default();
        events.iter().filter_map(|e| r.feed(e)).collect()
    }

    #[test]
    fn a_short_still_touch_is_a_tap_at_the_down_point() {
        let g = run(&[
            ev(0, TouchPhase::Down, 100.0, 100.0, 0),
            ev(0, TouchPhase::Move, 110.0, 104.0, 50),
            ev(0, TouchPhase::Up, 110.0, 104.0, 120),
        ]);
        assert_eq!(g.last(), Some(&Gesture::Tap { x: 100.0, y: 100.0 }));
        assert!(
            !g.iter().any(|g| matches!(g, Gesture::Drag { .. })),
            "wobble within the slop is not a drag"
        );
    }

    #[test]
    fn a_long_press_is_not_a_tap() {
        let g = run(&[
            ev(0, TouchPhase::Down, 100.0, 100.0, 0),
            ev(0, TouchPhase::Up, 100.0, 100.0, 900),
        ]);
        assert_eq!(g.last(), Some(&Gesture::Release { x: 100.0, y: 100.0 }));
    }

    #[test]
    fn moving_past_the_slop_drags_then_releases() {
        let g = run(&[
            ev(0, TouchPhase::Down, 100.0, 100.0, 0),
            ev(0, TouchPhase::Move, 160.0, 100.0, 40),
            ev(0, TouchPhase::Up, 170.0, 100.0, 80),
        ]);
        assert_eq!(
            g[1],
            Gesture::Drag {
                from: (100.0, 100.0),
                x: 160.0,
                y: 100.0
            }
        );
        assert_eq!(g[2], Gesture::Release { x: 170.0, y: 100.0 });
    }

    #[test]
    fn a_swipe_in_from_the_left_edge_goes_back_but_one_from_the_middle_does_not() {
        let edge = run(&[
            ev(0, TouchPhase::Down, 10.0, 200.0, 0),
            ev(0, TouchPhase::Move, 80.0, 210.0, 40),
            ev(0, TouchPhase::Up, 150.0, 215.0, 90),
        ]);
        assert_eq!(edge.last(), Some(&Gesture::Back));
        let middle = run(&[
            ev(0, TouchPhase::Down, 300.0, 200.0, 0),
            ev(0, TouchPhase::Move, 380.0, 210.0, 40),
            ev(0, TouchPhase::Up, 450.0, 215.0, 90),
        ]);
        assert_eq!(
            middle.last(),
            Some(&Gesture::Release { x: 450.0, y: 215.0 })
        );
        let steep = run(&[
            ev(0, TouchPhase::Down, 10.0, 100.0, 0),
            ev(0, TouchPhase::Move, 60.0, 200.0, 40),
            ev(0, TouchPhase::Up, 110.0, 300.0, 90),
        ]);
        assert_ne!(
            steep.last(),
            Some(&Gesture::Back),
            "a mostly vertical stroke from the edge is not back"
        );
    }

    #[test]
    fn a_back_swipe_goes_on_by_its_distance_or_a_flick_and_a_flick_back_cancels_it() {
        // The finger's x at the given times (ms), from the left edge, along a horizontal line.
        let swipe = |points: &[(f32, u64)]| {
            let (last, rest) = points.split_last().expect("points");
            let mut events = vec![ev(0, TouchPhase::Down, points[0].0, 200.0, points[0].1)];
            events.extend(
                rest[1..]
                    .iter()
                    .map(|&(x, ms)| ev(0, TouchPhase::Move, x, 200.0, ms)),
            );
            events.push(ev(0, TouchPhase::Up, last.0, 200.0, last.1));
            run(&events).last().copied()
        };
        // By distance, let go slowly: 90 of the display's 960 units, to the unit.
        assert_ne!(
            swipe(&[(10.0, 0), (60.0, 300), (99.0, 600)]),
            Some(Gesture::Back)
        );
        assert_eq!(
            swipe(&[(10.0, 0), (60.0, 300), (100.0, 600)]),
            Some(Gesture::Back)
        );
        // By a flick: 70 units in 30 ms (2,300 a second) goes back short of its distance; 20 units in 10 ms is a twitch.
        assert_eq!(
            swipe(&[(10.0, 0), (40.0, 10), (70.0, 20), (80.0, 30)]),
            Some(Gesture::Back)
        );
        assert_ne!(
            swipe(&[(10.0, 0), (20.0, 5), (30.0, 10)]),
            Some(Gesture::Back)
        );
        // Far enough and then flicked back to the left: the finger took it back.
        assert_ne!(
            swipe(&[
                (10.0, 0),
                (150.0, 100),
                (260.0, 200),
                (200.0, 220),
                (150.0, 230)
            ]),
            Some(Gesture::Back)
        );
        // The same far swipe let go as it was going on goes back.
        assert_eq!(
            swipe(&[(10.0, 0), (150.0, 100), (260.0, 200), (300.0, 230)]),
            Some(Gesture::Back)
        );
    }

    #[test]
    fn a_second_finger_cancels_and_nothing_fires_until_all_fingers_lift() {
        let g = run(&[
            ev(0, TouchPhase::Down, 100.0, 100.0, 0),
            ev(1, TouchPhase::Down, 300.0, 100.0, 20),
            ev(0, TouchPhase::Up, 100.0, 100.0, 60),
            ev(1, TouchPhase::Up, 300.0, 100.0, 80),
            ev(2, TouchPhase::Down, 200.0, 200.0, 200),
            ev(2, TouchPhase::Up, 200.0, 200.0, 250),
        ]);
        assert_eq!(
            g,
            vec![
                Gesture::Down { x: 100.0, y: 100.0 },
                Gesture::Cancel,
                Gesture::Down { x: 200.0, y: 200.0 },
                Gesture::Tap { x: 200.0, y: 200.0 }
            ]
        );
    }

    #[test]
    fn a_lost_contact_cancels_and_the_next_touch_starts_clean() {
        let g = run(&[
            ev(0, TouchPhase::Down, 100.0, 100.0, 0),
            ev(0, TouchPhase::Cancel, 0.0, 0.0, 10),
            ev(0, TouchPhase::Down, 50.0, 60.0, 30),
            ev(0, TouchPhase::Up, 50.0, 60.0, 60),
        ]);
        assert_eq!(g[1], Gesture::Cancel);
        assert_eq!(g.last(), Some(&Gesture::Tap { x: 50.0, y: 60.0 }));
    }
}
