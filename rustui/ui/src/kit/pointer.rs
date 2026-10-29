//! The Magic Remote's pointer as the kit's rule has it (PointerHover, 2026-10-29: hover is a cue, click is the act): the samples of the
//! key socket (`x`, `y`, the buttons, whether it is over the shell) become what a screen acts on. [`Pointer::sample`] turns them into
//! [`Gesture`]s over the focus [`Engine`]: the cue moves from one control to another, a press begins on a control, and a press let go
//! over the control it began on is a click, which focuses that control and is the one thing that activates it. A sample at the place
//! the pointer already was (content moving under a still pointer, the pacer's repeat) is not a movement and never brings the cue back
//! after a key hid it.

use super::Engine;
use super::focus::Id;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// The cue moved from one control to another (either may be none).
    Hover { from: Option<Id>, to: Option<Id> },
    /// The button went down on a control (it takes its pressed look).
    Down(Id),
    /// The button went up: the pressed look ends (the control it was down on, if any).
    Up(Option<Id>),
    /// The button went up over the control it went down on: the control is focused and acted on.
    Click(Id),
}

#[derive(Debug, Default)]
pub struct Pointer {
    at: Option<(f32, f32)>,
    down: Option<Id>,
    pressed: bool,
}

impl Pointer {
    pub fn new() -> Self {
        Self::default()
    }

    /// One sample of the pointer at `now_ns` (the engine's monotonic clock, for the focus move a click makes).
    pub fn sample(&mut self, engine: &mut Engine, now_ns: u64, x: f32, y: f32, buttons: u32, visible: bool) -> Vec<Gesture> {
        let mut out = Vec::new();
        if !visible {
            let gone = engine.focus.pointer_gone();
            if gone.from.is_some() {
                out.push(Gesture::Hover { from: gone.from, to: None });
            }
            if let Some(d) = self.down.take() {
                out.push(Gesture::Up(Some(d)));
            }
            self.at = None;
            self.pressed = false;
            return out;
        }
        let moved = self.at != Some((x, y));
        self.at = Some((x, y));
        let h = engine.focus.hover(x, y, moved);
        if h.from != h.to {
            out.push(Gesture::Hover { from: h.from, to: h.to });
        }
        let now_down = buttons & 1 != 0;
        if now_down && !self.pressed {
            self.down = engine.focus.hit(x, y).filter(|id| engine.focus.is_live(*id));
            if let Some(id) = self.down {
                out.push(Gesture::Down(id));
            }
        } else if !now_down && self.pressed {
            let began = self.down.take();
            if let Some(id) = began.filter(|id| engine.focus.hit(x, y) == Some(*id)) {
                engine.click(x, y, now_ns);
                out.push(Gesture::Up(Some(id)));
                out.push(Gesture::Click(id));
            } else {
                out.push(Gesture::Up(began));
            }
        }
        self.pressed = now_down;
        out
    }

    /// A key of the remote: the cue goes and stays gone until the pointer moves (a sample at the place it already was is no move).
    pub fn key(&mut self, engine: &mut Engine) -> Option<Gesture> {
        engine.focus.key();
        self.down.take().map(|d| Gesture::Up(Some(d)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::focus::{Region, Scope};
    use crate::kit::testkit;
    use ui::geometry::Frame;

    fn rig() -> (ui::testing::Harness, crate::kit::Kit, Engine, Pointer) {
        let (h, kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        engine.focus.register(Id(1), Frame::new(100.0, 100.0, 200.0, 64.0), Region(0), Scope::BASE);
        engine.focus.register(Id(2), Frame::new(100.0, 200.0, 200.0, 64.0), Region(0), Scope::BASE);
        engine.focus.register(Id(3), Frame::new(100.0, 300.0, 200.0, 64.0), Region(0), Scope::BASE);
        engine.focus.set_enabled(Id(3), false);
        (h, kit, engine, Pointer::new())
    }

    #[test]
    fn hover_is_a_cue_that_follows_the_pointer_and_never_moves_the_focus() {
        let (_h, _kit, mut e, mut p) = rig();
        e.focus.focus(Id(1));
        let g = p.sample(&mut e, 0, 150.0, 220.0, 0, true);
        assert_eq!(g, vec![Gesture::Hover { from: None, to: Some(Id(2)) }]);
        assert_eq!(e.focus.current(), Some(Id(1)), "the focus did not move");
        let g = p.sample(&mut e, 1, 150.0, 120.0, 0, true);
        assert_eq!(g, vec![Gesture::Hover { from: Some(Id(2)), to: Some(Id(1)) }]);
        assert!(p.sample(&mut e, 2, 150.0, 120.0, 0, true).is_empty(), "the same place again says nothing");
        let g = p.sample(&mut e, 3, 0.0, 0.0, 0, false);
        assert_eq!(g, vec![Gesture::Hover { from: Some(Id(1)), to: None }], "the pointer going takes the cue with it");
    }

    #[test]
    fn a_key_hides_the_cue_and_a_still_pointer_does_not_bring_it_back() {
        let (_h, _kit, mut e, mut p) = rig();
        p.sample(&mut e, 0, 150.0, 120.0, 0, true);
        assert_eq!(e.focus.hovered(), Some(Id(1)));
        p.key(&mut e);
        assert_eq!(e.focus.hovered(), None);
        assert!(p.sample(&mut e, 1, 150.0, 120.0, 0, true).is_empty(), "content under a still pointer: no cue");
        let g = p.sample(&mut e, 2, 151.0, 120.0, 0, true);
        assert_eq!(g, vec![Gesture::Hover { from: None, to: Some(Id(1)) }], "a movement brings it back");
    }

    #[test]
    fn a_press_let_go_over_the_same_control_is_a_click_that_focuses_it() {
        let (_h, _kit, mut e, mut p) = rig();
        e.focus.focus(Id(1));
        p.sample(&mut e, 0, 150.0, 220.0, 0, true);
        assert_eq!(p.sample(&mut e, 1, 150.0, 220.0, 1, true), vec![Gesture::Down(Id(2))]);
        assert_eq!(e.focus.current(), Some(Id(1)), "a press alone does not focus");
        let g = p.sample(&mut e, 2, 150.0, 220.0, 0, true);
        assert_eq!(g, vec![Gesture::Up(Some(Id(2))), Gesture::Click(Id(2))]);
        assert_eq!(e.focus.current(), Some(Id(2)), "the click focused it");
    }

    #[test]
    fn a_press_let_go_elsewhere_or_on_a_disabled_control_is_no_click() {
        let (_h, _kit, mut e, mut p) = rig();
        p.sample(&mut e, 0, 150.0, 120.0, 1, true);
        let g = p.sample(&mut e, 1, 150.0, 220.0, 0, true);
        assert!(g.contains(&Gesture::Up(Some(Id(1)))) && !g.iter().any(|g| matches!(g, Gesture::Click(_))), "{g:?}");
        assert!(p.sample(&mut e, 2, 150.0, 320.0, 1, true).iter().all(|g| !matches!(g, Gesture::Down(_))), "a disabled control takes no press");
        let g = p.sample(&mut e, 3, 150.0, 320.0, 0, true);
        assert!(g.iter().all(|g| !matches!(g, Gesture::Click(_))), "{g:?}");
        assert_eq!(e.focus.current(), None);
    }
}
