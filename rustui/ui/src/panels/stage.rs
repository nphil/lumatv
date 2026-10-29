//! What the surfaces of the panels share: the root their layers hang under and how it comes and goes with the panel's life, the dim behind
//! them, the modal scope that makes the panel's controls the only ones the keys reach while it is up, the first focus (lifted at once, not
//! flown to), and the pointer's cue and click.
//!
//! A surface keeps what is its own (its layout, its layers and what it changes in place); this keeps the rest the same for all of them.

use super::life::{ENTER_MS, EXIT_MS, Phase};
use super::ui::{Arrive, Root, follow, scrim, scrim_set};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::motion::Beat;
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::widgets::Hover;
use crate::kit::{Engine, Kit};
use ui::geometry::Frame;
use ui::group::Group;
use ui::scene::ShapeLayer;

/// How long a panel slides: its own beat, or the fade of reduced motion.
pub fn slide_ms(kit: &Kit, ms: u32) -> u32 {
    if kit.ui.look.reduced_motion { kit.beat_ms(Beat::Label) } else { ms }
}

/// What the stage tells a surface about the pointer.
#[derive(Clone, Copy, Debug)]
pub enum Touch {
    /// The cue moved from one control to another (either may be none).
    Hover { from: Option<Id>, to: Option<Id> },
    /// A press began (`down`) or ended on a control.
    Press { id: Id, down: bool },
}

/// A click on the dim behind a panel. The backdrop is a disabled focusable (the keys never reach it), and the engine swallows a click on
/// a disabled control, so the click is told here: a press that began on the dim and ended on it.
#[derive(Debug, Default)]
pub struct Dim {
    began: bool,
    pressed: bool,
}

impl Dim {
    /// One sample of the pointer; true when it ended a click on the dim. `on_dim` is whether the pointer is over the dim.
    pub fn feed(&mut self, on_dim: bool, s: &Sample) -> bool {
        let down = s.visible && s.buttons & 1 != 0;
        let mut click = false;
        if down && !self.pressed {
            self.began = on_dim;
        } else if !down && self.pressed {
            click = self.began && on_dim && s.visible;
            self.began = false;
        }
        self.pressed = down;
        click
    }
}

pub struct Stage {
    z: i32,
    scope: Scope,
    arrive: Option<Arrive>,
    root: Option<Root>,
    scrim: Option<ShapeLayer>,
    backdrop: Option<Id>,
    pub hover: Hover,
    pointer: Pointer,
    dim: Dim,
    modal: bool,
    /// The first focus after the panel comes up (or its content is made again) is lifted at once; the next ones fly.
    first: bool,
}

impl Stage {
    /// `z` is the panel's depth: the dim at `z`, the hover cue above the plates at `z + 8`.
    pub fn new(z: i32, scope: Scope) -> Self {
        Self { z, scope, arrive: None, root: None, scrim: None, backdrop: None, hover: Hover::new(z + 8), pointer: Pointer::new(), dim: Dim::default(), modal: false, first: true }
    }

    pub fn z(&self) -> i32 {
        self.z
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    /// Whether [`Stage::ensure`] would make a root (there is none, or it arrives another way): what hung under the old one is to be dropped
    /// and made again.
    pub fn needs_root(&self, arrive: Arrive) -> bool {
        self.root.is_none() || self.arrive != Some(arrive)
    }

    /// The root the layers hang under, and the dim behind them when `dim`.
    pub fn ensure(&mut self, kit: &Kit, phase: Phase, arrive: Arrive, dim: bool) -> std::io::Result<()> {
        if !self.needs_root(arrive) {
            return Ok(());
        }
        self.root = Some(Root::new(kit, arrive, phase == Phase::Open)?);
        self.arrive = Some(arrive);
        if dim && self.scrim.is_none() {
            self.scrim = Some(scrim(kit, self.z, false));
        }
        Ok(())
    }

    /// The group the layers hang under (after [`Stage::ensure`]).
    pub fn group(&self) -> &Group {
        &self.root.as_ref().expect("a stage that has a root").group
    }

    pub fn root(&self) -> Option<&Root> {
        self.root.as_ref()
    }

    /// A focusable the size of the screen behind everything: the pointer's click on the dim lands on it (it never takes the focus).
    pub fn backdrop(&mut self, engine: &mut Engine, id: Id, region: Region) {
        self.drop_backdrop(engine);
        engine.focus.register(id, Frame::new(0.0, 0.0, 1920.0, 1080.0), region, self.scope);
        engine.focus.set_enabled(id, false);
        self.backdrop = Some(id);
    }

    pub fn drop_backdrop(&mut self, engine: &mut Engine) {
        if let Some(id) = self.backdrop.take() {
            engine.focus.unregister(id);
        }
    }

    /// Fits the stage to the phase of the panel's life; true while the panel is up (its keys are the panel's, its content is to be kept).
    pub fn phase(&mut self, kit: &Kit, engine: &mut Engine, phase: Phase) -> bool {
        let (enter, exit) = (slide_ms(kit, ENTER_MS), slide_ms(kit, EXIT_MS));
        match phase {
            Phase::Opening | Phase::Open => {
                if !self.modal {
                    engine.focus.open_modal(self.scope);
                    self.modal = true;
                    self.first = true;
                }
                if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
                    root.show(enter);
                }
                if let Some(s) = &self.scrim {
                    scrim_set(s, true);
                }
                true
            }
            Phase::Leaving => {
                if let Some(root) = self.root.as_mut().filter(|r| r.is_shown()) {
                    root.hide(exit);
                }
                false
            }
            Phase::Parked => {
                if let Some(root) = self.root.as_mut() {
                    root.park();
                }
                if let Some(s) = &self.scrim {
                    scrim_set(s, false);
                }
                self.release_keys(engine);
                self.hover.hide();
                false
            }
            Phase::Gone => false,
        }
    }

    fn release_keys(&mut self, engine: &mut Engine) {
        if self.modal {
            engine.focus.close_modal();
            self.modal = false;
        }
    }

    /// The panel's content was made again: its focus is lifted at once the next time it is set.
    pub fn refocus(&mut self) {
        self.first = true;
    }

    /// The focus follows the model's `want`; the first one after the panel comes up is lifted at once.
    pub fn focus(&mut self, engine: &mut Engine, want: Option<Id>, now_ns: u64) {
        match want {
            Some(id) if self.first => {
                if engine.focus.focus(id) {
                    let _ = engine.flow.snap(id, true, now_ns);
                    self.first = false;
                }
            }
            w => follow(engine, w, now_ns),
        }
    }

    /// A key of the remote: the pointer's cue goes (and does not come back until the pointer moves).
    pub fn key(&mut self, engine: &mut Engine) {
        self.pointer.key(engine);
        self.hover.hide();
    }

    /// A sample of the pointer. `on` is told what happened to the controls: where the cue moved (a surface changes the look of its own
    /// buttons there, and answers the corner radius of the plate cue over the new control, or none) and a press beginning or ending on a
    /// control. The control a click ended on is returned.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample, mut on: impl FnMut(Touch) -> Option<f32>) -> Option<Id> {
        let root = self.root.as_ref()?;
        let on_dim = self.backdrop.is_some_and(|b| s.visible && engine.focus.hit(s.x, s.y) == Some(b));
        let dim_click = self.dim.feed(on_dim, &s);
        let mut click = None;
        for g in self.pointer.feed(engine, now_ns, s) {
            match g {
                Gesture::Hover { from, to } => {
                    let radius = on(Touch::Hover { from, to });
                    let frame = to.filter(|t| engine.focus.current() != Some(*t)).and_then(|t| engine.focus.frame(t));
                    self.hover.show(kit, &root.group, radius.zip(frame).map(|(r, f)| (f, r)));
                }
                Gesture::Down(id) => {
                    on(Touch::Press { id, down: true });
                }
                Gesture::Up(Some(id)) => {
                    on(Touch::Press { id, down: false });
                }
                Gesture::Click(id) => click = Some(id),
                Gesture::Up(None) => {}
            }
        }
        click.or(self.backdrop.filter(|_| dim_click))
    }

    /// Everything goes: the panel is gone (or its look changed and it is made again).
    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_backdrop(engine);
        self.release_keys(engine);
        self.root = None;
        self.scrim = None;
        self.arrive = None;
        self.hover = Hover::new(self.z + 8);
        self.first = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    #[test]
    fn a_stage_slides_in_takes_the_keys_and_gives_them_back_when_parked() {
        let (_h, kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut s = Stage::new(100, Scope(250));
        assert!(s.needs_root(Arrive::Right(600.0)));
        s.ensure(&kit, Phase::Opening, Arrive::Right(600.0), true).expect("a root");
        assert!(!s.needs_root(Arrive::Right(600.0)) && s.needs_root(Arrive::Bottom(300.0)), "another way of arriving is another root");
        assert!(s.root().is_some_and(|r| r.group.is_inert()), "inert until it is shown");
        assert!(s.phase(&kit, &mut engine, Phase::Opening));
        assert_eq!(engine.focus.modal(), Some(Scope(250)));
        assert!(s.root().is_some_and(|r| r.is_shown() && !r.group.is_inert()));
        assert!(!s.phase(&kit, &mut engine, Phase::Leaving));
        assert_eq!(engine.focus.modal(), Some(Scope(250)), "the keys stay while it slides out");
        assert!(!s.phase(&kit, &mut engine, Phase::Parked));
        assert_eq!(engine.focus.modal(), None);
        assert!(s.root().is_some_and(|r| r.group.is_inert()));
        s.drop_all(&mut engine);
        assert!(!s.is_up());
    }

    #[test]
    fn the_first_focus_is_snapped_and_a_backdrop_is_a_focusable_the_keys_never_reach() {
        let (_h, kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut s = Stage::new(100, Scope(250));
        s.ensure(&kit, Phase::Opening, Arrive::Fade, false).expect("a root");
        s.backdrop(&mut engine, Id(7), Region(9));
        assert!(engine.focus.frame(Id(7)).is_some_and(|f| f.w == 1920.0));
        assert!(!engine.focus.is_live(Id(7)), "disabled: no key lands on it");
        s.drop_backdrop(&mut engine);
        assert!(engine.focus.frame(Id(7)).is_none());
    }
}
