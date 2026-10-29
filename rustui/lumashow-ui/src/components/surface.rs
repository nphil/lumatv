//! A material surface: the glass of a button, card, key, track or sheet, with its prepared pressed state above it. The surface asks
//! the resolver ([`crate::lucent::smart_material`]) for its [`MaterialPair`] once, when it is built and again when its state or
//! the look changes, and a press is a crossfade between the two layers (and, for a card or a thumb, a scale of the whole group): no
//! text is rastered, no texture made, no backdrop captured.
//!
//! The surface lives in its component's [`Group`] as one or two parts and is handed the group when it acts, so that the same
//! group's other parts (label, icon) share the press scale.

use super::group::{Group, Part, Pose};
use super::plate::ring;
use crate::context::Ui;
use crate::geometry::Frame;
use crate::lucent::material::{
    ControlState, MaterialContext, MaterialPair, PressTreatment, SmartMaterial,
};
use crate::lucent::tokens::{motion, state};
use compositor_protocol::{Glass, Props};

pub struct Surface {
    role: SmartMaterial,
    context: MaterialContext,
    frame: Frame,
    state: ControlState,
    pair: MaterialPair,
    /// Index of the rest glass in the group.
    rest: usize,
    /// Index of the pressed glass, present for every material that responds to a press.
    pressed: Option<usize>,
    /// The focus landing cue: a ring just outside the glass, shown while the control is focused.
    ring: Option<usize>,
    down: bool,
    /// The pose the surface's layers are in while nothing presses it: where the component that holds them has put them (a cell
    /// that stands below where it was made). A press crossfades the glass where it stands, and a release goes back to this.
    rest_pose: Pose,
}

impl Surface {
    /// Adds the surface's glass to `group`: the rest glass at `z` and, above it and invisible, the pressed glass.
    pub fn add(
        ui: &Ui,
        group: &mut Group,
        frame: Frame,
        z: i32,
        role: SmartMaterial,
        context: MaterialContext,
        state: ControlState,
    ) -> Self {
        Self::build(ui, group, frame, z, role, context, state, true)
    }

    /// [`Surface::add`] for one of many alike that a touch alone presses (a key of the on-screen keyboard): no focus ring and no
    /// pressed twin, two layers fewer each. Its owner shows one pressed glass over whichever is pressed ([`Surface::pressed_glass`]),
    /// and neither [`Surface::ring_part`] nor a press of the surface itself is asked of it.
    pub fn add_light(
        ui: &Ui,
        group: &mut Group,
        frame: Frame,
        z: i32,
        role: SmartMaterial,
        context: MaterialContext,
        state: ControlState,
    ) -> Self {
        Self::build(ui, group, frame, z, role, context, state, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        ui: &Ui,
        group: &mut Group,
        frame: Frame,
        z: i32,
        role: SmartMaterial,
        context: MaterialContext,
        state: ControlState,
        ringed: bool,
    ) -> Self {
        let pair = ui
            .look
            .material(role, &state, context)
            .sized(frame.w, frame.h);
        let (w, h) = (frame.w.round() as u32, frame.h.round() as u32);
        let rest = group.push(Part::new(
            ui.scene
                .glass(w, h, z, pair.rest, Props::at(frame.x, frame.y)),
            frame,
        ));
        let responds = ringed
            && ui
                .look
                .material(role, &ControlState::default(), context)
                .press
                != PressTreatment::None;
        let pressed = responds.then(|| {
            let mut part = Part::new(
                ui.scene.glass(
                    w,
                    h,
                    z,
                    pair.pressed,
                    Props {
                        opacity: 0.0,
                        ..Props::at(frame.x, frame.y)
                    },
                ),
                frame,
            );
            part.rest_opacity = 0.0;
            group.push(part)
        });
        let cue = state::FOCUS_CUE_WIDTH;
        let around = frame.outset(cue);
        let ring = ringed.then(|| {
            group.push(Part::new(
                ui.scene.shape(
                    around.w.round() as u32,
                    around.h.round() as u32,
                    z,
                    ring(ui.look.focus_cue(), pair.rest.corner_radius + cue, cue),
                    Props {
                        visible: state.focus_visible,
                        ..Props::at(around.x, around.y)
                    },
                ),
                around,
            ))
        });
        Self {
            role,
            context,
            frame,
            state,
            pair,
            rest,
            pressed,
            ring,
            down: false,
            rest_pose: Pose::REST,
        }
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn pair(&self) -> &MaterialPair {
        &self.pair
    }

    pub fn state(&self) -> ControlState {
        self.state
    }

    pub fn role(&self) -> SmartMaterial {
        self.role
    }

    /// The group indices of the surface's glass layers: the rest glass and, if it has one, the pressed glass.
    pub fn parts(&self) -> (usize, Option<usize>) {
        (self.rest, self.pressed)
    }

    /// Whether the surface is drawn pressed.
    pub fn is_down(&self) -> bool {
        self.down
    }

    /// The group index of the focus ring.
    pub fn ring_part(&self) -> usize {
        self.ring.expect("a surface made with a focus ring")
    }

    /// The focus ring's index while it is not shown: a pose applied to the whole group keeps it hidden.
    pub fn idle_ring(&self) -> Option<usize> {
        self.ring.filter(|_| !self.state.focus_visible)
    }

    /// The glass a pressed `role` is at `size`, for an owner that shows one over whichever of many surfaces is pressed.
    pub fn pressed_glass(
        ui: &Ui,
        role: SmartMaterial,
        context: MaterialContext,
        size: (f32, f32),
    ) -> Glass {
        ui.look
            .material(role, &ControlState::default(), context)
            .sized(size.0, size.1)
            .pressed
    }

    /// The glass each layer shows now (the pressed one is the same as rest while the control is disabled).
    pub fn shown(&self) -> (Glass, Option<Glass>) {
        (self.pair.rest, self.pressed.map(|_| self.pair.pressed))
    }

    /// Moves the surface's layers with the rest of its group. The pressed layer keeps its crossfade state.
    fn resolve(&mut self, ui: &Ui, group: &Group, state: ControlState) {
        let pair = ui
            .look
            .material(self.role, &state, self.context)
            .sized(self.frame.w, self.frame.h);
        if pair != self.pair {
            if let Some(layer) = group.parts[self.rest].glass() {
                layer.restyle(pair.rest);
            }
            if let Some(layer) = self.pressed.and_then(|i| group.parts[i].glass()) {
                layer.restyle(pair.pressed);
            }
            self.pair = pair;
        }
        self.state = state;
    }

    /// Shows or hides the focus ring to match the state.
    fn show_ring(&self, group: &mut Group) {
        let pose = if self.state.focus_visible {
            self.rest_pose
        } else {
            Pose::hidden()
        };
        if let Some(ring) = self.ring {
            group.set_parts(&[ring], pose);
        }
    }

    /// The control's state changed (selected, enabled, focused): the pair is resolved again and both layers restyled, only if it
    /// differs. A press in progress stays.
    pub fn set_state(&mut self, ui: &Ui, group: &mut Group, state: ControlState) {
        if state == self.state {
            return;
        }
        let was_enabled = self.state.enabled;
        let focus_changed = state.focus_visible != self.state.focus_visible;
        self.resolve(ui, group, state);
        if focus_changed {
            self.show_ring(group);
        }
        if was_enabled && !state.enabled && self.down {
            self.set_down(ui, group, false);
        }
    }

    /// The look changed (theme or tier): the pair is resolved again. Components call this for each surface when the host installs
    /// a new generation; nothing is rebuilt.
    pub fn restyle(&mut self, ui: &Ui, group: &mut Group) {
        let state = self.state;
        self.resolve(ui, group, state);
    }

    /// Shows the surface pressed or at rest. The two states crossfade over the press duration; a card or a thumb also scales its
    /// whole group about the group's point, unless motion is reduced. A disabled control, and a material with no press, ignore it.
    pub fn set_down(&mut self, ui: &Ui, group: &mut Group, down: bool) {
        self.set_down_at(ui, group, down, self.rest_pose);
    }

    /// Says where the component's layers rest from now on (it moved them to `pose`, or is moving them there): what a press shows
    /// itself on and a release returns to.
    pub fn set_rest(&mut self, pose: Pose) {
        self.rest_pose = pose;
    }

    /// The pose the surface's layers rest in ([`Surface::set_rest`]).
    pub fn rest(&self) -> Pose {
        self.rest_pose
    }

    /// `set_down` for a group that is not at rest: `rest` is the pose its layers are in while nothing presses (a page that a finger
    /// or an animation has moved), the pose a release returns to and a scale press is made about.
    pub fn set_down_at(&mut self, ui: &Ui, group: &mut Group, down: bool, rest: Pose) {
        let about = group.about;
        self.press_among(ui, group, down, rest, about, None);
    }

    /// `set_down` for one surface among many in a group: a material that scales scales only this surface and the group's parts
    /// `with` (its icon and words), about `about` (the surface's own middle), and not the whole group.
    pub fn set_down_among(
        &mut self,
        ui: &Ui,
        group: &mut Group,
        down: bool,
        rest: Pose,
        about: (f32, f32),
        with: &[usize],
    ) {
        self.press_among(ui, group, down, rest, about, Some(with));
    }

    fn press_among(
        &mut self,
        ui: &Ui,
        group: &mut Group,
        down: bool,
        rest: Pose,
        about: (f32, f32),
        with: Option<&[usize]>,
    ) {
        let (Some(pressed), true) = (self.pressed, self.state.enabled) else {
            return;
        };
        if down == self.down {
            return;
        }
        self.down = down;
        group.parts[self.rest].rest_opacity = if down { 0.0 } else { 1.0 };
        group.parts[pressed].rest_opacity = if down { 1.0 } else { 0.0 };
        let ms = ui.look.duration_ms(motion::PRESS_MS);
        match (self.pair.press, with) {
            (PressTreatment::Scale { factor }, None) if ui.look.moves() => {
                let pose = if down {
                    Pose {
                        scale: factor,
                        ..rest
                    }
                } else {
                    rest
                };
                group.animate(pose, ms, motion::SETTLE);
            }
            (PressTreatment::Scale { factor }, Some(with)) if ui.look.moves() => {
                let pose = if down {
                    Pose {
                        scale: factor,
                        ..rest
                    }
                } else {
                    rest
                };
                for &i in [self.rest, pressed].iter().chain(with) {
                    group.parts[i].animate(about, pose, ms, motion::SETTLE);
                }
            }
            _ => {
                group.animate_parts(&[self.rest, pressed], rest, ms, motion::SETTLE);
            }
        }
    }
}
