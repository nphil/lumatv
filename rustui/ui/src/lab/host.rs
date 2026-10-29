//! The lab as one part of the shell: the model fed from the channel, the helper, the keys and the clock, and the surface drawn in the lab's
//! band of the engine's z. Nothing here does I/O: effects go to the shell, which hands them to the host.

use super::model::{Lab, Target};
use super::surface::LabSurface;
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input};
use crate::kit::{Engine, Kit};
use serde_json::{Value, json};

pub struct LabHost {
    pub model: Lab,
    surface: LabSurface,
    redraw: bool,
}

impl LabHost {
    /// `z` is the lab's band base (`system_z + kit::z::LAB`).
    pub fn new(z: i32) -> Self {
        Self { model: Lab::new(), surface: LabSurface::new(z), redraw: false }
    }

    pub fn is_open(&self) -> bool {
        self.model.is_open()
    }

    /// The lab has the keys while it is open.
    pub fn wants(&self) -> bool {
        self.model.wants()
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    fn note(&mut self, fx: &[Effect]) {
        if fx.iter().any(|e| matches!(e, Effect::Redraw)) {
            self.redraw = true;
        }
    }

    pub fn open(&mut self, now: u64) -> Vec<Effect> {
        let fx = self.model.open(now);
        self.redraw = true;
        fx
    }

    pub fn close(&mut self, why: &str, now: u64) -> Vec<Effect> {
        let fx = self.model.close(why, now);
        self.redraw = true;
        fx
    }

    /// A topic, a helper's answer, a key or the clock for the model. Topics and answers matter while the lab is closed too (what is in front,
    /// the answer to an end that is out).
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let fx = self.model.update(input, now);
        self.note(&fx);
        fx
    }

    /// A key of the remote: the pointer's cue goes first.
    pub fn key(&mut self, name: &str, down: bool, repeat: bool, engine: &mut Engine, now: u64) -> Vec<Effect> {
        if down && self.model.is_open() {
            self.surface.on_key(engine);
        }
        self.update(&Input::Key { name: name.to_string(), down, repeat }, now)
    }

    /// A sample of the pointer: the cue follows it and a click is what OK would be.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample, now: u64) -> Vec<Effect> {
        if !self.model.is_open() {
            return Vec::new();
        }
        if s.visible {
            self.model.touch(now);
        }
        match self.surface.pointer(kit, engine, now_ns, s) {
            Some(t) => self.click(t, now),
            None => Vec::new(),
        }
    }

    pub fn click(&mut self, t: Target, now: u64) -> Vec<Effect> {
        let fx = self.model.click(t, now);
        self.redraw = true;
        fx
    }

    /// Draws what changed. Returns whether any of the lab is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, now_ns: u64) -> std::io::Result<bool> {
        match self.model.view() {
            Some(v) => self.surface.apply(kit, engine, &v, now_ns),
            None => {
                if self.surface.is_up() {
                    self.surface.drop_all(engine);
                }
                Ok(false)
            }
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.model.due_at()
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.surface.restyle(engine);
        self.redraw = true;
    }

    pub fn state(&self) -> Value {
        json!({"displaylab": self.model.state()})
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        json!({"displaylab": self.surface.geometry(engine)})
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        self.surface.feed_lint(kit, lint);
    }

    /// How many layers the lab holds.
    pub fn layers(&self) -> usize {
        self.surface.layers()
    }
}
