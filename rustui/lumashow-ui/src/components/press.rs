//! Press tracking for one control: commit on release inside, cancel on a lost or taken contact. A control feeds it the raw
//! contacts of the host's input and acts on what it says; the visual part of a press (the material pair crossfade, the scale) is
//! the control's own and follows the same steps.
//!
//! Only the first finger counts: a second contact is ignored until the first has gone, so a stray palm cannot turn a press into
//! an activation. The press starts on `Down`, so feedback appears on the next frame; the action waits for the release, and fires
//! then without waiting for any animation.

use crate::geometry::Frame;
use crate::input::{TouchEvent, TouchPhase};

/// What a contact did to a control's press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing for this control (another finger, a move before any press, a contact that never started inside).
    Idle,
    /// A finger went down on the control: show it pressed.
    Pressed,
    /// The finger moved out of the control: show it at rest; releasing now does nothing.
    Left,
    /// The finger came back in: show it pressed again.
    Entered,
    /// The finger lifted inside: this is the activation. Show it at rest.
    Activated,
    /// The finger lifted outside: no activation. Show it at rest.
    Released,
    /// The contact was lost (or taken by a drag elsewhere): no activation. Show it at rest.
    Cancelled,
}

impl Step {
    /// Whether the control is drawn pressed after this step.
    pub fn pressed_after(self, was_pressed: bool) -> bool {
        match self {
            Step::Pressed | Step::Entered => true,
            Step::Left | Step::Activated | Step::Released | Step::Cancelled => false,
            Step::Idle => was_pressed,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u32,
    inside: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PressTracker {
    contact: Option<Contact>,
}

impl PressTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a contact that began on the control is still down.
    pub fn is_active(&self) -> bool {
        self.contact.is_some()
    }

    /// Whether the control is drawn pressed: a contact began on it and is inside it now.
    pub fn is_pressed(&self) -> bool {
        self.contact.is_some_and(|c| c.inside)
    }

    /// Feeds one contact event; `hit` is where the control takes a touch (half-open, like every frame).
    pub fn feed(&mut self, hit: &Frame, e: &TouchEvent) -> Step {
        match (e.phase, self.contact) {
            (TouchPhase::Down, None) => {
                if hit.contains(e.x, e.y) {
                    self.contact = Some(Contact {
                        id: e.id,
                        inside: true,
                    });
                    Step::Pressed
                } else {
                    Step::Idle
                }
            }
            (TouchPhase::Move, Some(c)) if c.id == e.id => {
                let inside = hit.contains(e.x, e.y);
                self.contact = Some(Contact { inside, ..c });
                match (c.inside, inside) {
                    (true, false) => Step::Left,
                    (false, true) => Step::Entered,
                    _ => Step::Idle,
                }
            }
            (TouchPhase::Up, Some(c)) if c.id == e.id => {
                self.contact = None;
                if hit.contains(e.x, e.y) {
                    Step::Activated
                } else {
                    Step::Released
                }
            }
            (TouchPhase::Cancel, Some(c)) if c.id == e.id => {
                self.contact = None;
                Step::Cancelled
            }
            _ => Step::Idle,
        }
    }

    /// Ends a press that something else has taken (a list that began to scroll, a sheet that began to follow the finger): no
    /// activation. Returns whether there was a press to end.
    pub fn cancel(&mut self) -> bool {
        self.contact.take().is_some()
    }
}

/// A control of a [`PressSet`] changed how it looks: `pressed` is the new state of control `index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub index: usize,
    pub pressed: bool,
}

/// Which of a screen's several touch targets the finger is holding, for screens that decide what is under a touch themselves
/// (`hit` answers with an index) instead of owning one [`PressTracker`] per control. The same rules: only the first finger
/// counts, sliding off lets the target go once, and lifting or losing the contact lets it go. The action is the caller's, from
/// the recognised tap.
#[derive(Debug, Default)]
pub struct PressSet {
    active: Option<(u32, usize)>,
}

impl PressSet {
    /// The target being held, if any.
    pub fn active(&self) -> Option<usize> {
        self.active.map(|(_, index)| index)
    }

    /// One raw touch sample; `hit` says which target (if any) is at a point. At most one target changes per sample.
    pub fn feed(
        &mut self,
        e: &TouchEvent,
        hit: impl Fn(f32, f32) -> Option<usize>,
    ) -> Option<Change> {
        match (e.phase, self.active) {
            (TouchPhase::Down, None) => {
                let index = hit(e.x, e.y)?;
                self.active = Some((e.id, index));
                Some(Change {
                    index,
                    pressed: true,
                })
            }
            (TouchPhase::Move, Some((id, index))) if id == e.id => {
                if hit(e.x, e.y) == Some(index) {
                    return None;
                }
                self.active = None;
                Some(Change {
                    index,
                    pressed: false,
                })
            }
            (TouchPhase::Up | TouchPhase::Cancel, Some((id, index))) if id == e.id => {
                self.active = None;
                Some(Change {
                    index,
                    pressed: false,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIT: Frame = Frame::new(100.0, 100.0, 200.0, 64.0);

    fn ev(id: u32, phase: TouchPhase, x: f32, y: f32) -> TouchEvent {
        TouchEvent {
            id,
            phase,
            x,
            y,
            at_ns: 0,
        }
    }

    #[test]
    fn a_press_released_inside_is_the_activation_and_one_released_outside_is_not() {
        let mut p = PressTracker::new();
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Down, 150.0, 130.0)),
            Step::Pressed
        );
        assert!(p.is_pressed());
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Up, 160.0, 130.0)),
            Step::Activated
        );
        assert!(!p.is_active());

        assert_eq!(
            p.feed(&HIT, &ev(2, TouchPhase::Down, 150.0, 130.0)),
            Step::Pressed
        );
        assert_eq!(
            p.feed(&HIT, &ev(2, TouchPhase::Move, 150.0, 400.0)),
            Step::Left
        );
        assert!(
            !p.is_pressed() && p.is_active(),
            "out of the control: shown at rest, contact still tracked"
        );
        assert_eq!(
            p.feed(&HIT, &ev(2, TouchPhase::Up, 150.0, 400.0)),
            Step::Released
        );
    }

    #[test]
    fn leaving_and_coming_back_presses_again_and_releasing_inside_then_activates() {
        let mut p = PressTracker::new();
        p.feed(&HIT, &ev(1, TouchPhase::Down, 150.0, 130.0));
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Move, 50.0, 130.0)),
            Step::Left
        );
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Move, 60.0, 130.0)),
            Step::Idle
        );
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Move, 120.0, 130.0)),
            Step::Entered
        );
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Up, 120.0, 130.0)),
            Step::Activated
        );
    }

    #[test]
    fn a_lost_contact_or_one_taken_elsewhere_never_activates() {
        let mut p = PressTracker::new();
        p.feed(&HIT, &ev(1, TouchPhase::Down, 150.0, 130.0));
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Cancel, 0.0, 0.0)),
            Step::Cancelled
        );
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Up, 150.0, 130.0)),
            Step::Idle
        );

        p.feed(&HIT, &ev(3, TouchPhase::Down, 150.0, 130.0));
        assert!(p.cancel());
        assert!(!p.cancel(), "nothing left to cancel");
        assert_eq!(
            p.feed(&HIT, &ev(3, TouchPhase::Up, 150.0, 130.0)),
            Step::Idle
        );
    }

    #[test]
    fn a_second_finger_and_a_finger_that_began_elsewhere_are_ignored() {
        let mut p = PressTracker::new();
        assert_eq!(p.feed(&HIT, &ev(1, TouchPhase::Down, 5.0, 5.0)), Step::Idle);
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Move, 150.0, 130.0)),
            Step::Idle
        );
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Up, 150.0, 130.0)),
            Step::Idle,
            "dragged in from outside is not a press"
        );

        p.feed(&HIT, &ev(2, TouchPhase::Down, 150.0, 130.0));
        assert_eq!(
            p.feed(&HIT, &ev(9, TouchPhase::Down, 160.0, 130.0)),
            Step::Idle
        );
        assert_eq!(
            p.feed(&HIT, &ev(9, TouchPhase::Up, 160.0, 130.0)),
            Step::Idle
        );
        assert!(p.is_pressed(), "the first finger still holds it");
        assert_eq!(
            p.feed(&HIT, &ev(2, TouchPhase::Up, 150.0, 130.0)),
            Step::Activated
        );
    }

    #[test]
    fn the_edges_follow_the_half_open_rule() {
        let mut p = PressTracker::new();
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Down, 300.0, 130.0)),
            Step::Idle,
            "the right edge is outside"
        );
        assert_eq!(
            p.feed(&HIT, &ev(1, TouchPhase::Down, 100.0, 100.0)),
            Step::Pressed,
            "the left and top edges are inside"
        );
    }

    #[test]
    fn what_is_drawn_after_each_step() {
        assert!(Step::Pressed.pressed_after(false));
        assert!(Step::Entered.pressed_after(false));
        assert!(!Step::Activated.pressed_after(true));
        assert!(!Step::Cancelled.pressed_after(true));
        assert!(Step::Idle.pressed_after(true));
        assert!(!Step::Idle.pressed_after(false));
    }

    mod set {
        use super::super::*;

        fn ev(id: u32, phase: TouchPhase, x: f32) -> TouchEvent {
            TouchEvent {
                id,
                phase,
                x,
                y: 10.0,
                at_ns: 0,
            }
        }

        /// Two 100 wide targets: 0..100 is target 0, 120..220 is target 1.
        fn hit(x: f32, _y: f32) -> Option<usize> {
            match x {
                x if (0.0..100.0).contains(&x) => Some(0),
                x if (120.0..220.0).contains(&x) => Some(1),
                _ => None,
            }
        }

        #[test]
        fn a_target_is_pressed_while_the_finger_is_on_it_and_let_go_on_lifting() {
            let mut p = PressSet::default();
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Down, 150.0), hit),
                Some(Change {
                    index: 1,
                    pressed: true
                })
            );
            assert_eq!(p.active(), Some(1));
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Move, 160.0), hit),
                None,
                "wobbling on the same target changes nothing"
            );
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Up, 160.0), hit),
                Some(Change {
                    index: 1,
                    pressed: false
                })
            );
            assert_eq!(p.active(), None);
        }

        #[test]
        fn sliding_off_lets_it_go_once_and_lifting_afterwards_does_nothing() {
            let mut p = PressSet::default();
            p.feed(&ev(1, TouchPhase::Down, 50.0), hit);
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Move, 110.0), hit),
                Some(Change {
                    index: 0,
                    pressed: false
                })
            );
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Move, 50.0), hit),
                None,
                "coming back does not press again"
            );
            assert_eq!(p.feed(&ev(1, TouchPhase::Up, 50.0), hit), None);
        }

        #[test]
        fn a_touch_on_nothing_presses_nothing_and_a_lost_contact_lets_go() {
            let mut p = PressSet::default();
            assert_eq!(p.feed(&ev(1, TouchPhase::Down, 110.0), hit), None);
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Move, 50.0), hit),
                None,
                "dragged in from nothing"
            );
            p.feed(&ev(2, TouchPhase::Down, 50.0), hit);
            assert_eq!(
                p.feed(&ev(2, TouchPhase::Cancel, 0.0), hit),
                Some(Change {
                    index: 0,
                    pressed: false
                })
            );
        }

        #[test]
        fn a_second_finger_never_takes_over_or_lets_go_of_the_first() {
            let mut p = PressSet::default();
            p.feed(&ev(1, TouchPhase::Down, 50.0), hit);
            assert_eq!(p.feed(&ev(2, TouchPhase::Down, 150.0), hit), None);
            assert_eq!(p.feed(&ev(2, TouchPhase::Up, 150.0), hit), None);
            assert_eq!(p.active(), Some(0));
            assert_eq!(
                p.feed(&ev(1, TouchPhase::Up, 50.0), hit),
                Some(Change {
                    index: 0,
                    pressed: false
                })
            );
        }
    }
}
