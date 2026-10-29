//! The toggle: an on/off switch that proposes a change and shows the answer.
//!
//! A toggle never decides its own value. The model's `on` is the last value the system CONFIRMED; a tap, or a drag of the thumb
//! to the other end, only PROPOSES the opposite (`ToggleAction::Propose`). Until the screen confirms it (by putting the new
//! value in the model) the thumb is at the confirmed end, and while a change is `pending` it also wears a ring, so the person can
//! see "asked, not yet done" without the control ever showing a value that is not true.
//!
//! "On" is never colour alone: the knob sits at the right-hand end and carries a tick, and the track turns to the accent. The toggle
//! is the approved mock-ups' `.sw` (Nitin's decision 36): a track of 68 by 40, an off track in the palette's own `trackOff` inside a
//! ring, a light knob of 30 with a small shadow that shrinks a little under a finger, and, while it is on, an accent tick of 18 on the
//! knob. The knob is a few retained layers moved together with a `Set` or an `Animate`: nothing is restyled or drawn again to move it.
//!
//! This file also holds the few helpers the slider shares with it (a plain glass well, and moving a thumb's layers).

use super::group::{Group, Movable, Part, Pose};
use super::icon::{Icon, icon_part, paint};
use super::plate::{ring, shape};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::{Component, Damage, target_around};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchPhase};
use crate::lucent::material::{
    ControlState, MaterialContext, MaterialPair, PressTreatment, SmartMaterial,
};
use crate::lucent::tokens::{motion, smart, switch};
use compositor_protocol::{Ease, Props, Rect};
use std::io;

/// What a toggle shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ToggleModel {
    /// The last CONFIRMED value. A proposal is not shown until it is confirmed.
    pub on: bool,
    pub enabled: bool,
    /// A change has been asked for and not confirmed: the thumb stays at the confirmed end and wears a ring.
    pub pending: bool,
    /// What the toggle is called ("Wi-Fi"): what an assistive reader says.
    pub name: String,
}

impl ToggleModel {
    pub fn new(name: impl Into<String>, on: bool) -> Self {
        Self {
            on,
            enabled: true,
            pending: false,
            name: name.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleAction {
    /// The person asks for this value. It is a proposal: the toggle keeps showing the confirmed one.
    Propose(bool),
}

/// How far a finger may wander from where it went down before the touch counts as a drag and not a tap.
const SLOP: f32 = 8.0;
const THUMB: f32 = switch::KNOB;
const INSET: f32 = (Toggle::HEIGHT - THUMB) / 2.0;
const CHECK: f32 = switch::TICK;
const RING_OUTSET: f32 = 3.0;
const RING_WIDTH: f32 = 2.0;

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u32,
    /// Where the finger went down, and how far from the thumb's middle (so grabbing the thumb does not make it jump).
    start_x: f32,
    grab: f32,
    dragging: bool,
}

pub struct Toggle {
    model: ToggleModel,
    track_frame: Frame,
    hit: Frame,
    track: Well,
    thumb: Group,
    surface: Surface,
    check: usize,
    pending: usize,
    /// The layers a move touches: all of the thumb's, and its focus ring only while it is shown.
    moving: Vec<usize>,
    /// How far the thumb's layers are from the OFF end now.
    offset: f32,
    focus: bool,
    contact: Option<Contact>,
}

impl Toggle {
    pub const WIDTH: f32 = switch::WIDTH;
    pub const HEIGHT: f32 = switch::HEIGHT;
    /// How far the thumb travels between the two ends.
    pub const TRAVEL: f32 = Self::WIDTH - Self::HEIGHT;

    /// A toggle whose track has its top-left corner at (`x`, `y`). It takes a touch in at least 64 x 64 around the track.
    pub fn new(ui: &mut Ui, x: f32, y: f32, z: i32, model: &ToggleModel) -> io::Result<Self> {
        let track_frame = Frame::new(x, y, Self::WIDTH, Self::HEIGHT);
        let track = Well::new(
            ui,
            track_frame,
            z,
            SmartMaterial::ToggleTrack,
            MaterialContext::Nested,
            Self::track_state(model, false),
        );
        let thumb_frame = Frame::new(x + INSET, y + INSET, THUMB, THUMB);
        let mut thumb = Group::new(thumb_frame.centre());
        let surface = Surface::add(
            ui,
            &mut thumb,
            thumb_frame,
            z + 1,
            SmartMaterial::ToggleThumb,
            MaterialContext::Nested,
            Self::thumb_state(model, false),
        );
        let (cx, cy) = thumb_frame.centre();
        let check_frame = Frame::new(cx - CHECK / 2.0, cy - CHECK / 2.0, CHECK, CHECK);
        let mut check_part = icon_part(ui, Icon::Tick, check_frame, z + 2, check_ink(ui, model))?;
        check_part.rest_opacity = if model.on { 1.0 } else { 0.0 };
        let check = thumb.push(check_part);
        let around = thumb_frame.outset(RING_OUTSET);
        let mut ring_part = Part::new(
            shape(
                ui,
                around,
                z + 2,
                ring(ui.look.ink2(), around.w / 2.0, RING_WIDTH),
            ),
            around,
        );
        ring_part.rest_opacity = if model.pending { 1.0 } else { 0.0 };
        let pending = thumb.push(ring_part);
        let moving = live_parts(&thumb, &surface);
        let mut toggle = Self {
            model: model.clone(),
            track_frame,
            hit: target_around(track_frame, smart::TARGET),
            track,
            thumb,
            surface,
            check,
            pending,
            moving,
            offset: 0.0,
            focus: false,
            contact: None,
        };
        // The thumb is built at the OFF end; an ON toggle starts at the other end without travelling there.
        toggle.place(ui, Self::end(model.on), false);
        Ok(toggle)
    }

    fn track_state(model: &ToggleModel, focus: bool) -> ControlState {
        ControlState {
            selected: model.on,
            enabled: model.enabled,
            pending: model.pending,
            focus_visible: focus,
            pressed: false,
        }
    }

    fn thumb_state(model: &ToggleModel, focus: bool) -> ControlState {
        Self::track_state(model, focus)
    }

    /// The thumb's offset from the OFF end when it rests at the end for `on`.
    fn end(on: bool) -> f32 {
        if on { Self::TRAVEL } else { 0.0 }
    }

    pub fn model(&self) -> &ToggleModel {
        &self.model
    }

    /// Where a touch counts: the track, and at least 64 units around its centre in each direction.
    pub fn hit(&self) -> Frame {
        self.hit
    }

    pub fn track_frame(&self) -> Frame {
        self.track_frame
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Whether the thumb is held by a finger (it shows pressed, and may be dragged).
    pub fn is_held(&self) -> bool {
        self.contact.is_some()
    }

    /// Whether the thumb is being dragged along its track.
    pub fn is_dragging(&self) -> bool {
        self.contact.is_some_and(|c| c.dragging)
    }

    /// Where the thumb is now, as a share of its travel: 0 at the OFF end, 1 at the ON end.
    pub fn thumb_position(&self) -> f32 {
        self.offset / Self::TRAVEL
    }

    /// Shows or hides the keyboard-focus cue (a ring just outside the thumb).
    pub fn set_focus_visible(&mut self, ui: &Ui, visible: bool) {
        if visible == self.focus {
            return;
        }
        self.focus = visible;
        self.track
            .set_state(ui, Self::track_state(&self.model, visible));
        self.surface
            .set_state(ui, &mut self.thumb, Self::thumb_state(&self.model, visible));
        self.moving = live_parts(&self.thumb, &self.surface);
    }

    /// Ends a press that something else has taken (a list that began to scroll): nothing is proposed, and the thumb goes back to
    /// the confirmed end.
    pub fn cancel(&mut self, ui: &Ui) {
        if self.contact.take().is_some() {
            self.release_visuals(ui);
        }
    }

    /// The look was replaced: restyle the glass and draw the check and ring again in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.track.restyle(ui);
        self.surface.restyle(ui, &mut self.thumb);
        let ink = check_ink(ui, &self.model);
        if let Some(layer) = self.thumb.parts[self.check].image_mut() {
            paint(layer, Icon::Tick, ink);
        }
        if let Some(layer) = self.thumb.parts[self.pending].shape() {
            let r = self.thumb.parts[self.pending].frame.w / 2.0;
            layer.reshape(ring(ui.look.ink2(), r, RING_WIDTH));
        }
    }

    /// Moves the thumb to `to` (an offset from the OFF end): at once, or travelling there over the label duration. Reduced motion
    /// has no travel.
    fn place(&mut self, ui: &Ui, to: f32, travel: bool) {
        shift_group(&mut self.thumb, to - self.offset);
        self.offset = to;
        let pose = rest_pose(&self.surface, ui);
        if travel && ui.look.moves() {
            let ms = ui.look.duration_ms(motion::LABEL_MS);
            self.thumb
                .animate_parts(&self.moving, pose, ms, motion::SETTLE);
        } else {
            self.thumb.set_parts(&self.moving, pose);
        }
    }

    fn set_down(&mut self, ui: &Ui, down: bool) {
        self.surface.set_down(ui, &mut self.thumb, down);
        hide_idle_ring(&self.surface, &mut self.thumb);
    }

    /// The finger is gone without a result: the thumb stops showing pressed and goes back to the confirmed end.
    fn release_visuals(&mut self, ui: &Ui) {
        self.set_down(ui, false);
        let home = Self::end(self.model.on);
        if (self.offset - home).abs() > f32::EPSILON {
            self.place(ui, home, true);
        }
    }

    fn accepts_touch(&self) -> bool {
        self.model.enabled && !self.model.pending
    }
}

fn check_ink(ui: &Ui, model: &ToggleModel) -> compositor_protocol::Rgba {
    if model.enabled {
        ui.look.accent()
    } else {
        ui.look.disabled_ink()
    }
}

impl Movable for Toggle {
    fn set_in(&mut self, about: (f32, f32), pose: Pose) {
        self.track.set_in(about, pose);
        self.thumb
            .set_in(about, pose, self.surface.idle_ring().as_slice());
    }

    fn animate_in(&mut self, about: (f32, f32), pose: Pose, duration_ms: u32, ease: Ease) {
        self.track.animate_in(about, pose, duration_ms, ease);
        self.thumb.animate_in(
            about,
            pose,
            self.surface.idle_ring().as_slice(),
            duration_ms,
            ease,
        );
    }
}

impl Component for Toggle {
    type Model = ToggleModel;
    type Action = ToggleAction;

    fn update(&mut self, model: &ToggleModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let mut damage = Damage::Properties;
        let old = std::mem::replace(&mut self.model, model.clone());
        if model.enabled != old.enabled || model.on != old.on || model.pending != old.pending {
            self.track
                .set_state(ui, Self::track_state(model, self.focus));
            self.surface
                .set_state(ui, &mut self.thumb, Self::thumb_state(model, self.focus));
        }
        if !self.accepts_touch() && self.contact.take().is_some() {
            self.set_down(ui, false);
        }
        if model.enabled != old.enabled {
            let ink = check_ink(ui, model);
            if let Some(layer) = self.thumb.parts[self.check].image_mut() {
                paint(layer, Icon::Tick, ink);
            }
            damage = Damage::Content;
        }
        self.thumb.parts[self.check].rest_opacity = if model.on { 1.0 } else { 0.0 };
        self.thumb.parts[self.pending].rest_opacity = if model.pending { 1.0 } else { 0.0 };
        // A finger on the thumb decides where it is until it lets go; then the thumb goes to the confirmed end.
        if self.contact.is_none() {
            self.place(ui, Self::end(model.on), true);
        }
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<ToggleAction> {
        let Event::Touch(t) = event else {
            return None;
        };
        if !self.accepts_touch() {
            return None;
        }
        match (t.phase, self.contact) {
            (TouchPhase::Down, None) if self.hit.contains(t.x, t.y) => {
                let (cx, _) = self.thumb.parts[self.surface.parts().0].frame.centre();
                let on_thumb = (t.x - cx).abs() <= THUMB / 2.0;
                self.contact = Some(Contact {
                    id: t.id,
                    start_x: t.x,
                    grab: if on_thumb { t.x - cx } else { 0.0 },
                    dragging: false,
                });
                self.set_down(ui, true);
                None
            }
            (TouchPhase::Move, Some(mut c)) if c.id == t.id => {
                if !c.dragging && (t.x - c.start_x).abs() > SLOP {
                    c.dragging = true;
                }
                self.contact = Some(c);
                if c.dragging {
                    // The thumb follows the finger along its track and no further than its ends.
                    let off_cx = self.track_frame.x + INSET + THUMB / 2.0;
                    let to = (t.x - c.grab - off_cx).clamp(0.0, Self::TRAVEL);
                    if (to - self.offset).abs() > f32::EPSILON {
                        self.place(ui, to, false);
                    }
                }
                None
            }
            (TouchPhase::Up, Some(c)) if c.id == t.id => {
                self.contact = None;
                let proposal = if c.dragging {
                    // The side nearer the thumb. The same side as now is no change, so nothing is proposed.
                    let side = self.offset >= Self::TRAVEL / 2.0;
                    (side != self.model.on).then_some(ToggleAction::Propose(side))
                } else {
                    self.hit
                        .contains(t.x, t.y)
                        .then_some(ToggleAction::Propose(!self.model.on))
                };
                // Whatever was proposed, the thumb shows the confirmed value until it is confirmed.
                self.release_visuals(ui);
                proposal
            }
            (TouchPhase::Cancel, Some(c)) if c.id == t.id => {
                self.contact = None;
                self.release_visuals(ui);
                None
            }
            _ => None,
        }
    }

    fn frame(&self) -> Frame {
        self.hit
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        let node = SemanticNode::new(SemanticKind::Toggle, self.model.name.clone(), self.hit)
            .value(if self.model.on { "On" } else { "Off" })
            .selected(self.model.on)
            .enabled(self.model.enabled);
        out.push(if self.accepts_touch() {
            node.actionable()
        } else {
            node
        });
    }
}

// ---- helpers the slider shares ----------------------------------------------------------------------------------------

/// A plain glass part with no press and no focus ring of its own: a track, a fill, a well. Its material is the resolver's.
pub(crate) struct Well {
    part: Part,
    role: SmartMaterial,
    context: MaterialContext,
    state: ControlState,
    pair: MaterialPair,
    visible: bool,
}

impl Well {
    pub(crate) fn new(
        ui: &Ui,
        frame: Frame,
        z: i32,
        role: SmartMaterial,
        context: MaterialContext,
        state: ControlState,
    ) -> Self {
        let pair = ui
            .look
            .material(role, &state, context)
            .sized(frame.w, frame.h);
        let layer = ui.scene.glass(
            frame.w.round() as u32,
            frame.h.round() as u32,
            z,
            pair.rest,
            Props::at(frame.x, frame.y),
        );
        Self {
            part: Part::new(layer, frame),
            role,
            context,
            state,
            pair,
            visible: true,
        }
    }

    /// The material the glass was last resolved to.
    #[cfg(test)]
    pub(crate) fn pair(&self) -> &MaterialPair {
        &self.pair
    }

    /// The glass is resolved again for `state` and restyled if it differs.
    pub(crate) fn set_state(&mut self, ui: &Ui, state: ControlState) {
        self.state = state;
        self.restyle(ui);
    }

    /// The look changed: resolve again for the current state.
    pub(crate) fn restyle(&mut self, ui: &Ui) {
        let frame = self.part.frame;
        let pair = ui
            .look
            .material(self.role, &self.state, self.context)
            .sized(frame.w, frame.h);
        if pair != self.pair {
            if let Some(layer) = self.part.glass() {
                layer.restyle(pair.rest);
            }
            self.pair = pair;
        }
    }

    /// Shows only `clip` of the glass (a fill showing part of a full-width segment), or hides it.
    pub(crate) fn show(&mut self, clip: Option<Rect>, visible: bool) {
        self.part.clip = clip;
        self.visible = visible;
        let pose = if visible { Pose::REST } else { Pose::hidden() };
        self.part.set((0.0, 0.0), pose);
    }

    /// Moves the glass with a page: its pose about `about`, kept hidden while it is hidden.
    pub(crate) fn set_in(&mut self, about: (f32, f32), pose: Pose) {
        let visible = pose.visible && self.visible;
        self.part.set(about, Pose { visible, ..pose });
    }

    pub(crate) fn animate_in(
        &mut self,
        about: (f32, f32),
        pose: Pose,
        duration_ms: u32,
        ease: Ease,
    ) {
        let visible = pose.visible && self.visible;
        self.part
            .animate(about, Pose { visible, ..pose }, duration_ms, ease);
    }
}

/// The pose a thumb's layers rest in: pressed (shrunk) while a finger holds it, else at rest.
pub(crate) fn rest_pose(surface: &Surface, ui: &Ui) -> Pose {
    match surface.pair().press {
        PressTreatment::Scale { factor } if surface.is_down() && ui.look.moves() => {
            Pose::scaled(factor)
        }
        _ => Pose::REST,
    }
}

/// The indices of the layers of `group` that are shown: all of them, but the focus ring only while it is wanted.
pub(crate) fn live_parts(group: &Group, surface: &Surface) -> Vec<usize> {
    (0..group.parts.len())
        .filter(|&i| i != surface.ring_part() || surface.state().focus_visible)
        .collect()
}

/// Moves where every layer of `group` rests by `dx`, so that the group's own press and release land at its new place.
pub(crate) fn shift_group(group: &mut Group, dx: f32) {
    for part in &mut group.parts {
        part.frame.x += dx;
        if let Some(clip) = &mut part.clip {
            clip.x += dx;
        }
    }
    group.about.0 += dx;
}

/// A scale press shows the group's focus ring along with the rest (the group moves as one); put it away again unless the control is
/// focused.
pub(crate) fn hide_idle_ring(surface: &Surface, group: &mut Group) {
    if !surface.state().focus_visible {
        group.set_parts(&[surface.ring_part()], Pose::hidden());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchPhase::{Cancel, Down, Move, Up};
    use crate::testing::{Harness, harness, touch};
    use compositor_protocol::{Content, Request};

    fn toggle(h: &mut Harness, on: bool) -> Toggle {
        let t = Toggle::new(&mut h.ui, 100.0, 100.0, 10, &ToggleModel::new("Wi-Fi", on)).unwrap();
        h.commit();
        t
    }

    fn middle(t: &Toggle) -> (f32, f32) {
        t.track_frame().centre()
    }

    #[test]
    fn a_tap_proposes_the_opposite_and_keeps_showing_what_is_confirmed() {
        let mut h = harness();
        let mut t = toggle(&mut h, false);
        let (x, y) = middle(&t);
        assert_eq!(t.input(&touch(1, Down, x, y, 0), &mut h.ui), None);
        assert!(t.is_held());
        assert_eq!(
            t.input(&touch(1, Up, x, y, 40), &mut h.ui),
            Some(ToggleAction::Propose(true))
        );
        assert!(!t.is_held());
        assert_eq!(
            t.thumb_position(),
            0.0,
            "the thumb stays at the confirmed (off) end"
        );
        // Confirmed: now it moves.
        t.update(&ToggleModel::new("Wi-Fi", true), &mut h.ui);
        assert_eq!(t.thumb_position(), 1.0);
    }

    #[test]
    fn a_drag_to_the_other_end_proposes_it_and_a_drag_back_proposes_nothing() {
        let mut h = harness();
        let mut t = toggle(&mut h, false);
        let (x, y) = middle(&t);
        let thumb_x = t.track_frame().x + 18.0;
        t.input(&touch(1, Down, thumb_x, y, 0), &mut h.ui);
        t.input(&touch(1, Move, thumb_x + 14.0, y, 16), &mut h.ui);
        t.input(&touch(1, Move, thumb_x + 80.0, y, 32), &mut h.ui);
        assert!(t.is_dragging());
        assert_eq!(t.thumb_position(), 1.0, "the thumb stops at its end");
        assert_eq!(
            t.input(&touch(1, Up, x + 80.0, y, 48), &mut h.ui),
            Some(ToggleAction::Propose(true))
        );

        t.input(&touch(1, Down, thumb_x, y, 100), &mut h.ui);
        t.input(&touch(1, Move, thumb_x + 20.0, y, 116), &mut h.ui);
        t.input(&touch(1, Move, thumb_x - 2.0, y, 132), &mut h.ui);
        assert_eq!(
            t.input(&touch(1, Up, thumb_x - 2.0, y, 148), &mut h.ui),
            None,
            "back where it was: no change"
        );
        assert_eq!(t.thumb_position(), 0.0);
    }

    #[test]
    fn a_cancel_a_second_finger_and_a_disabled_toggle_propose_nothing() {
        let mut h = harness();
        let mut t = toggle(&mut h, true);
        let (x, y) = middle(&t);
        t.input(&touch(1, Down, x, y, 0), &mut h.ui);
        assert_eq!(t.input(&touch(2, Down, x, y, 5), &mut h.ui), None);
        assert_eq!(t.input(&touch(2, Up, x, y, 10), &mut h.ui), None);
        assert_eq!(t.input(&touch(1, Cancel, 0.0, 0.0, 20), &mut h.ui), None);
        assert!(!t.is_held());
        assert_eq!(t.input(&touch(1, Up, x, y, 30), &mut h.ui), None);

        let mut model = ToggleModel::new("Wi-Fi", true);
        model.enabled = false;
        t.update(&model, &mut h.ui);
        let before = h.all().len();
        assert_eq!(t.input(&touch(1, Down, x, y, 40), &mut h.ui), None);
        assert_eq!(t.input(&touch(1, Up, x, y, 50), &mut h.ui), None);
        assert_eq!(
            h.all().len(),
            before,
            "a disabled toggle asks nothing of the compositor"
        );
        let mut sink = SemanticsSink::new();
        t.semantics(&mut sink);
        assert!(!sink.find("Wi-Fi").unwrap().enabled);
    }

    #[test]
    fn a_toggle_takes_a_full_size_touch_around_its_small_track() {
        let mut h = harness();
        let mut t = toggle(&mut h, false);
        assert!(t.hit().w >= 64.0 && t.hit().h >= 64.0);
        let (x, y) = middle(&t);
        // 20 units above the 40-high track is inside the 64-high target.
        t.input(&touch(1, Down, x, y - 20.0, 0), &mut h.ui);
        assert!(t.is_held());
        assert_eq!(
            t.input(&touch(1, Up, x, y - 20.0, 30), &mut h.ui),
            Some(ToggleAction::Propose(true))
        );
    }

    #[test]
    fn pending_wears_a_ring_without_showing_an_unconfirmed_value_and_moving_the_thumb_draws_nothing()
     {
        let mut h = harness();
        let mut t = toggle(&mut h, false);
        let mut model = ToggleModel::new("Wi-Fi", false);
        model.pending = true;
        assert_eq!(t.update(&model, &mut h.ui), Damage::Properties);
        assert_eq!(t.thumb_position(), 0.0);
        let sent = h.commit();
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::Damage { .. } | Request::Create { .. })),
            "{sent:?}"
        );

        // Confirmed on: only properties and the two prepared glass states change; no pixels are drawn.
        let on = ToggleModel::new("Wi-Fi", true);
        assert_eq!(t.update(&on, &mut h.ui), Damage::Properties);
        let sent = h.commit();
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::Damage { .. } | Request::Create { .. }))
        );
        let (rest, pressed) = t.surface().shown();
        assert!(rest.shadows[0].is_some(), "the knob's own small shadow");
        assert!(pressed.is_some());
    }

    #[test]
    fn on_is_not_colour_alone_the_check_shows_and_the_track_is_the_accent() {
        let mut h = harness();
        let mut t = toggle(&mut h, true);
        let mut sink = SemanticsSink::new();
        t.semantics(&mut sink);
        let node = sink.find("Wi-Fi").unwrap();
        assert!(node.selected);
        assert_eq!(node.value.as_deref(), Some("On"));
        let accent = h.ui.look.theme.materials.accent;
        let tints: Vec<_> = h
            .layers()
            .iter()
            .filter_map(|(_, c, _)| match c {
                Content::Glass(g) => Some(g.tint),
                _ => None,
            })
            .collect();
        assert!(tints.contains(&accent), "an on track is the accent");
        assert_eq!(t.thumb.parts[t.check].rest_opacity, 1.0);
        t.update(&ToggleModel::new("Wi-Fi", false), &mut h.ui);
        assert_eq!(t.thumb.parts[t.check].rest_opacity, 0.0);
    }

    #[test]
    fn the_track_is_inset_and_the_knob_is_the_light_one_with_its_small_shadow_in_dark_and_in_light()
    {
        for mode in [
            crate::lucent::palette::Mode::Dark,
            crate::lucent::palette::Mode::Light,
        ] {
            let mut h = harness();
            h.ui.look = crate::context::Look::standard(mode);
            let t = toggle(&mut h, false);
            let m = h.ui.look.theme.materials;
            let (rest, pressed) = t.surface().shown();
            assert_eq!(rest.tint, m.knob, "{mode:?}: light in both modes");
            assert!(
                rest.depth.is_none() && rest.sheen.is_none(),
                "{mode:?}: flat but for its lip and its shadow"
            );
            assert!(
                rest.shadows[0].is_some_and(|s| (s.offset_y, s.blur) == (2.0, 4.0)),
                "{mode:?}: the mock's small shadow"
            );
            // The knob presses by scaling and keeps its shadow.
            assert!(
                pressed.is_some_and(|p| p.shadows == rest.shadows),
                "{mode:?}"
            );
            assert!(
                matches!(t.surface().pair().press, PressTreatment::Scale { .. }),
                "{mode:?}"
            );
            let track = h
                .layers()
                .into_iter()
                .filter_map(|(_, c, _)| match c {
                    Content::Glass(g) => Some(g),
                    _ => None,
                })
                .find(|g| g.corner_radius >= 19.9 && g.sheen.is_none());
            let track = track.unwrap_or_else(|| panic!("{mode:?}: no track glass"));
            assert!(track.depth.is_some_and(|d| d.inset), "{mode:?}");
            // An off track is the palette's own `trackOff` inside a two-wide ring in the readable track's colour.
            assert_eq!(track.tint, m.track_off, "{mode:?}");
            assert_eq!((track.edge, track.edge_width), (m.track, 2.0), "{mode:?}");
        }
    }

    /// Nitin's decision 36 (10-08): the approved mock-ups' `.sw` is 68 by 40 with a knob of 30 five in from the ends and the top,
    /// travelling 28, and a tick of 18 on it, drawn in the accent.
    #[test]
    fn a_toggle_is_the_mocks_68_by_40_with_a_knob_of_30_that_travels_28_and_a_tick_of_18_in_the_accent()
     {
        assert_eq!(
            (Toggle::WIDTH, Toggle::HEIGHT, Toggle::TRAVEL),
            (68.0, 40.0, 28.0)
        );
        let mut h = harness();
        let mut t = toggle(&mut h, false);
        let track = t.track_frame();
        assert_eq!((track.w, track.h), (68.0, 40.0));
        let knob = t.thumb.parts[t.surface().parts().0].frame;
        assert_eq!((knob.w, knob.h), (30.0, 30.0));
        assert_eq!(
            (knob.x - track.x, knob.y - track.y),
            (5.0, 5.0),
            "5 in from the end and from the top"
        );
        let tick = t.thumb.parts[t.check].frame;
        assert_eq!((tick.w, tick.h), (18.0, 18.0));
        assert_eq!(tick.centre(), knob.centre(), "in the middle of the knob");
        t.update(&ToggleModel::new("Wi-Fi", true), &mut h.ui);
        let knob = t.thumb.parts[t.surface().parts().0].frame;
        assert_eq!(track.right() - knob.right(), 5.0, "5 from the other end");
        let on = ToggleModel::new("Wi-Fi", true);
        assert_eq!(check_ink(&h.ui, &on), h.ui.look.accent());
        let mut disabled = on;
        disabled.enabled = false;
        assert_eq!(check_ink(&h.ui, &disabled), h.ui.look.disabled_ink());
    }
}
