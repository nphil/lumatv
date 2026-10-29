//! The slider: a value along a track, tried live while a finger moves and committed when it lets go.
//!
//! The track is a recessed glass pill, the fill is a full-width accent glass segment of which only the part up to the thumb is shown
//! (a clip rectangle that moves), and the thumb is a raised glass disc that shrinks a little under a finger. While a finger drags,
//! only the thumb's layers and the fill's clip are moved, and only when the stepped value changes: no text is drawn, no layer is
//! made, nothing is restyled, and the track never changes (no inset jump).
//!
//! What the slider says: `Preview(v)` each time the value under the finger changes to a new step, `Commit(v)` when the finger lifts,
//! `Cancel` when the contact is lost or taken (and the thumb goes back to the actual value). After a `Commit` the thumb stays where
//! it was let go until the screen's next `update` says what is true: the screen shows the value being tried as `preview: Some(v)`
//! while the change is in flight, and `actual` once it is confirmed.

use super::group::{Group, Movable, Pose};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::toggle::{Well, hide_idle_ring, live_parts, rest_pose, shift_group};
use super::{Component, Damage, target_around};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchPhase};
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::tokens::smart;
use compositor_protocol::{Ease, Rect};
use std::io;

/// What a slider shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderModel {
    pub min: f32,
    pub max: f32,
    /// The distance between the values a person can choose.
    pub step: f32,
    /// The confirmed value.
    pub actual: f32,
    /// A value being tried (the service is previewing it): shown instead of `actual` until it is confirmed or dropped.
    pub preview: Option<f32>,
    pub enabled: bool,
    /// What the slider is called ("Volume").
    pub name: String,
}

impl SliderModel {
    pub fn new(name: impl Into<String>, min: f32, max: f32, step: f32, actual: f32) -> Self {
        Self {
            min,
            max,
            step,
            actual,
            preview: None,
            enabled: true,
            name: name.into(),
        }
    }

    /// The value the thumb shows when no finger is on it, kept inside the range.
    fn target(&self) -> f32 {
        self.preview
            .unwrap_or(self.actual)
            .clamp(self.min, self.max)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderAction {
    /// The value under the finger is now this (a new step).
    Preview(f32),
    /// The finger lifted at this value.
    Commit(f32),
    /// The contact was lost or taken: drop the preview; the thumb is back at the actual value.
    Cancel,
}

pub const THUMB: f32 = 32.0;
pub const TRACK_HEIGHT: f32 = 12.0;

/// The range must run upwards and the step must be a positive number.
pub(crate) fn check_range(min: f32, max: f32, step: f32) -> io::Result<()> {
    let ok = min.is_finite() && max.is_finite() && step.is_finite() && min < max && step > 0.0;
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a slider needs min < max and a positive step, not {min}..{max} step {step}"),
        ))
    }
}

/// The value a finger at `x` chooses on a track whose min end is at `left` and whose max end is `width` further: clamped to the
/// range and rounded to the nearest step from `min`. The max itself can be chosen even when the steps do not end on it.
pub fn value_at(min: f32, max: f32, step: f32, left: f32, width: f32, x: f32) -> f32 {
    let t = if width > 0.0 {
        ((x - left) / width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let raw = min + t * (max - min);
    let n = ((raw - min) / step).round();
    let stepped = (min + n * step).clamp(min, max);
    if (raw - max).abs() < (raw - stepped).abs() {
        max
    } else {
        stepped
    }
}

/// A number as short text: no trailing zeros ("0.5", "40"). One place for what a slider and a stepper say.
pub(crate) fn number_text(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u32,
    /// How far the finger was from the thumb's middle when it went down on the thumb (0 when it went down on the track).
    grab: f32,
    /// The last value shown and announced.
    value: f32,
}

pub struct Slider {
    model: SliderModel,
    frame: Frame,
    track_frame: Frame,
    hit: Frame,
    track: Well,
    fill: Well,
    thumb: Group,
    surface: Surface,
    moving: Vec<usize>,
    /// The value the thumb shows, and where its middle is.
    shown: f32,
    thumb_x: f32,
    focus: bool,
    contact: Option<Contact>,
}

impl Slider {
    /// A slider covering `frame` (the track is as wide as the frame and centred in its height). It takes a touch in at least 64
    /// units of height around the track. The range is checked here: a reversed or empty range, or a step that is not positive,
    /// is an error, and so is a frame too narrow to slide in.
    pub fn new(ui: &mut Ui, frame: Frame, z: i32, model: &SliderModel) -> io::Result<Self> {
        check_range(model.min, model.max, model.step)?;
        if frame.w < 2.0 * THUMB {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a slider needs a frame at least {} wide, not {}",
                    2.0 * THUMB,
                    frame.w
                ),
            ));
        }
        let track_frame = Frame::new(
            frame.x,
            frame.y + (frame.h - TRACK_HEIGHT) / 2.0,
            frame.w,
            TRACK_HEIGHT,
        );
        let state = Self::state(model, false);
        let track = Well::new(
            ui,
            track_frame,
            z,
            SmartMaterial::SliderTrack,
            MaterialContext::Nested,
            state,
        );
        let fill = Well::new(
            ui,
            track_frame,
            z + 1,
            SmartMaterial::SliderFill,
            MaterialContext::Nested,
            state,
        );
        let thumb_frame = Frame::new(frame.x, frame.y + (frame.h - THUMB) / 2.0, THUMB, THUMB);
        let mut thumb = Group::new(thumb_frame.centre());
        let surface = Surface::add(
            ui,
            &mut thumb,
            thumb_frame,
            z + 2,
            SmartMaterial::SliderThumb,
            MaterialContext::Nested,
            state,
        );
        let moving = live_parts(&thumb, &surface);
        let mut slider = Self {
            model: model.clone(),
            frame,
            track_frame,
            hit: target_around(frame, smart::TARGET),
            track,
            fill,
            thumb,
            surface,
            moving,
            shown: model.min,
            thumb_x: thumb_frame.centre().0,
            focus: false,
            contact: None,
        };
        slider.show(ui, model.target());
        Ok(slider)
    }

    fn state(model: &SliderModel, focus: bool) -> ControlState {
        ControlState {
            enabled: model.enabled,
            focus_visible: focus,
            ..ControlState::default()
        }
    }

    /// Where the thumb's middle is when the value is the min and when it is the max.
    fn ends(&self) -> (f32, f32) {
        (
            self.frame.x + THUMB / 2.0,
            self.frame.x + self.frame.w - THUMB / 2.0,
        )
    }

    /// The value a finger at `x` chooses, for this slider's range and where its track is.
    pub fn value_at(&self, x: f32) -> f32 {
        let (left, right) = self.ends();
        value_at(
            self.model.min,
            self.model.max,
            self.model.step,
            left,
            right - left,
            x,
        )
    }

    fn x_of(&self, value: f32) -> f32 {
        let (left, right) = self.ends();
        let t = (value - self.model.min) / (self.model.max - self.model.min);
        left + t.clamp(0.0, 1.0) * (right - left)
    }

    /// Shows `value`: the thumb's layers at its place and the fill's clip up to the thumb's middle. Only `Set`s, a few layers.
    fn show(&mut self, ui: &Ui, value: f32) {
        self.shown = value;
        let cx = self.x_of(value);
        shift_group(&mut self.thumb, cx - self.thumb_x);
        self.thumb_x = cx;
        let pose = rest_pose(&self.surface, ui);
        self.thumb.set_parts(&self.moving, pose);
        let t = self.track_frame;
        let clip = Rect {
            x: t.x,
            y: t.y,
            w: cx - t.x,
            h: t.h,
        };
        self.fill.show(Some(clip), clip.w > 0.0);
    }

    pub fn model(&self) -> &SliderModel {
        &self.model
    }

    /// The value the thumb shows now.
    pub fn shown(&self) -> f32 {
        self.shown
    }

    /// Where a touch counts: the frame, and at least 64 units of height around its middle.
    pub fn hit(&self) -> Frame {
        self.hit
    }

    pub fn track_frame(&self) -> Frame {
        self.track_frame
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn is_held(&self) -> bool {
        self.contact.is_some()
    }

    /// Shows or hides the keyboard-focus cue (a ring just outside the thumb).
    pub fn set_focus_visible(&mut self, ui: &Ui, visible: bool) {
        if visible == self.focus {
            return;
        }
        self.focus = visible;
        let state = Self::state(&self.model, visible);
        self.track.set_state(ui, state);
        self.fill.set_state(ui, state);
        self.surface.set_state(ui, &mut self.thumb, state);
        self.moving = live_parts(&self.thumb, &self.surface);
    }

    /// Ends a drag that something else has taken: `Cancel` if a finger was on the slider, and the thumb goes back to `actual`.
    pub fn cancel(&mut self, ui: &Ui) -> Option<SliderAction> {
        self.contact.take()?;
        self.surface.set_down(ui, &mut self.thumb, false);
        hide_idle_ring(&self.surface, &mut self.thumb);
        self.show(ui, self.model.actual.clamp(self.model.min, self.model.max));
        Some(SliderAction::Cancel)
    }

    /// The look was replaced: restyle the glass.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.track.restyle(ui);
        self.fill.restyle(ui);
        self.surface.restyle(ui, &mut self.thumb);
    }
}

impl Movable for Slider {
    fn set_in(&mut self, about: (f32, f32), pose: Pose) {
        self.track.set_in(about, pose);
        self.fill.set_in(about, pose);
        self.thumb
            .set_in(about, pose, self.surface.idle_ring().as_slice());
    }

    fn animate_in(&mut self, about: (f32, f32), pose: Pose, duration_ms: u32, ease: Ease) {
        self.track.animate_in(about, pose, duration_ms, ease);
        self.fill.animate_in(about, pose, duration_ms, ease);
        self.thumb.animate_in(
            about,
            pose,
            self.surface.idle_ring().as_slice(),
            duration_ms,
            ease,
        );
    }
}

impl Component for Slider {
    type Model = SliderModel;
    type Action = SliderAction;

    fn update(&mut self, model: &SliderModel, ui: &mut Ui) -> Damage {
        if check_range(model.min, model.max, model.step).is_err() {
            // The range of a slider is checked where it is built; a model with a bad one changes nothing.
            return Damage::None;
        }
        let was_enabled = self.model.enabled;
        let changed = *model != self.model;
        let range_changed = (model.min, model.max) != (self.model.min, self.model.max);
        if changed {
            self.model = model.clone();
        }
        if model.enabled != was_enabled {
            let state = Self::state(model, self.focus);
            self.track.set_state(ui, state);
            self.fill.set_state(ui, state);
            self.surface.set_state(ui, &mut self.thumb, state);
            if !model.enabled && self.contact.take().is_some() {
                self.surface.set_down(ui, &mut self.thumb, false);
                hide_idle_ring(&self.surface, &mut self.thumb);
            }
        }
        // A finger on the thumb decides where it is. Otherwise the thumb shows what the model says is true or being tried; the
        // comparison is with what is shown, so a value let go and not confirmed is put right by the next update.
        let target = model.target();
        let mut damage = if changed {
            Damage::Properties
        } else {
            Damage::None
        };
        if self.contact.is_none() && (target != self.shown || range_changed) {
            self.show(ui, target);
            damage = Damage::Properties;
        }
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<SliderAction> {
        let Event::Touch(t) = event else {
            return None;
        };
        if !self.model.enabled {
            return None;
        }
        match (t.phase, self.contact) {
            (TouchPhase::Down, None) if self.hit.contains(t.x, t.y) => {
                let on_thumb = (t.x - self.thumb_x).abs() <= THUMB / 2.0;
                let grab = if on_thumb { t.x - self.thumb_x } else { 0.0 };
                let value = self.value_at(t.x - grab);
                self.contact = Some(Contact {
                    id: t.id,
                    grab,
                    value,
                });
                self.surface.set_down(ui, &mut self.thumb, true);
                hide_idle_ring(&self.surface, &mut self.thumb);
                if value != self.shown {
                    self.show(ui, value);
                    return Some(SliderAction::Preview(value));
                }
                None
            }
            (TouchPhase::Move, Some(mut c)) if c.id == t.id => {
                let value = self.value_at(t.x - c.grab);
                if value == c.value {
                    return None;
                }
                c.value = value;
                self.contact = Some(c);
                self.show(ui, value);
                Some(SliderAction::Preview(value))
            }
            (TouchPhase::Up, Some(c)) if c.id == t.id => {
                self.contact = None;
                self.surface.set_down(ui, &mut self.thumb, false);
                hide_idle_ring(&self.surface, &mut self.thumb);
                Some(SliderAction::Commit(c.value))
            }
            (TouchPhase::Cancel, Some(c)) if c.id == t.id => self.cancel(ui),
            _ => None,
        }
    }

    #[allow(clippy::misnamed_getters)] // the frame a component takes a touch in is its hit area, not its drawn one
    fn frame(&self) -> Frame {
        self.hit
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        let node = SemanticNode::new(SemanticKind::Slider, self.model.name.clone(), self.hit)
            .value(number_text(self.shown))
            .enabled(self.model.enabled);
        out.push(if self.model.enabled {
            node.actionable()
        } else {
            node
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchPhase::{Cancel, Down, Move, Up};
    use crate::lucent::material::PressTreatment;
    use crate::testing::{Harness, harness, touch};
    use compositor_protocol::Request;

    const FRAME: Frame = Frame::new(100.0, 100.0, 432.0, 64.0);

    fn slider(h: &mut Harness, actual: f32) -> Slider {
        let s = Slider::new(
            &mut h.ui,
            FRAME,
            10,
            &SliderModel::new("Brightness", 0.0, 100.0, 5.0, actual),
        )
        .unwrap();
        h.commit();
        s
    }

    fn x_of(s: &Slider, value: f32) -> f32 {
        let (left, right) = s.ends();
        left + (value / 100.0) * (right - left)
    }

    #[test]
    fn a_value_is_the_nearest_step_clamped_and_the_max_is_always_reachable() {
        assert_eq!(value_at(0.0, 100.0, 5.0, 0.0, 400.0, 100.0), 25.0);
        assert_eq!(value_at(0.0, 100.0, 5.0, 0.0, 400.0, -50.0), 0.0);
        assert_eq!(value_at(0.0, 100.0, 5.0, 0.0, 400.0, 900.0), 100.0);
        // Steps that do not end on the max: the end of the track is still the max.
        assert_eq!(value_at(0.0, 10.0, 3.0, 0.0, 100.0, 100.0), 10.0);
        assert_eq!(value_at(0.0, 10.0, 3.0, 0.0, 100.0, 62.0), 6.0);
        assert_eq!(value_at(10.0, 20.0, 2.5, 0.0, 100.0, 50.0), 15.0);
    }

    #[test]
    fn a_bad_range_or_a_frame_too_narrow_to_slide_in_is_an_error() {
        let mut h = harness();
        for model in [
            SliderModel::new("x", 5.0, 5.0, 1.0, 5.0),
            SliderModel::new("x", 10.0, 0.0, 1.0, 5.0),
            SliderModel::new("x", 0.0, 10.0, 0.0, 5.0),
            SliderModel::new("x", 0.0, f32::NAN, 1.0, 5.0),
        ] {
            assert!(
                Slider::new(&mut h.ui, FRAME, 10, &model).is_err(),
                "{model:?}"
            );
        }
        let narrow = Frame::new(0.0, 0.0, 40.0, 64.0);
        assert!(
            Slider::new(
                &mut h.ui,
                narrow,
                10,
                &SliderModel::new("x", 0.0, 1.0, 0.1, 0.5)
            )
            .is_err()
        );
    }

    #[test]
    fn a_drag_previews_each_new_step_once_and_commits_where_the_finger_lets_go() {
        let mut h = harness();
        let mut s = slider(&mut h, 20.0);
        let thumb = x_of(&s, 20.0);
        assert_eq!(
            s.input(&touch(1, Down, thumb, 132.0, 0), &mut h.ui),
            None,
            "grabbing the thumb does not move it"
        );
        let mut previews = Vec::new();
        for (i, v) in [30.0, 30.4, 30.9, 45.0, 41.0].into_iter().enumerate() {
            if let Some(SliderAction::Preview(p)) = s.input(
                &touch(1, Move, x_of(&s, v), 132.0, 16 * (i as u64 + 1)),
                &mut h.ui,
            ) {
                previews.push(p);
            }
        }
        assert_eq!(
            previews,
            [30.0, 45.0, 40.0],
            "a step is announced once, and only when it changes: {previews:?}"
        );
        assert_eq!(
            s.input(&touch(1, Up, x_of(&s, 41.0), 132.0, 200), &mut h.ui),
            Some(SliderAction::Commit(40.0))
        );
        assert_eq!(
            s.shown(),
            40.0,
            "the thumb stays where it was let go until the model says what is true"
        );
        // The service confirms something else: now the thumb goes there.
        let mut model = SliderModel::new("Brightness", 0.0, 100.0, 5.0, 35.0);
        s.update(&model, &mut h.ui);
        assert_eq!(s.shown(), 35.0);
        // A preview in flight is shown instead of the actual value.
        model.preview = Some(60.0);
        s.update(&model, &mut h.ui);
        assert_eq!(s.shown(), 60.0);
    }

    #[test]
    fn a_drag_moves_only_layers_it_makes_nothing_draws_nothing_and_never_restyles_the_track() {
        let mut h = harness();
        let mut s = slider(&mut h, 0.0);
        s.input(&touch(1, Down, x_of(&s, 0.0), 132.0, 0), &mut h.ui);
        h.commit();
        for i in 1..=40u64 {
            s.input(
                &touch(1, Move, x_of(&s, i as f32 * 2.5), 132.0, i * 16),
                &mut h.ui,
            );
        }
        s.input(&touch(1, Up, x_of(&s, 100.0), 132.0, 700), &mut h.ui);
        let sent = h.commit();
        assert!(
            sent.iter().all(|r| matches!(
                r,
                Request::Set { .. } | Request::Animate { .. } | Request::Commit { .. }
            )),
            "a drag is positions and the press only: {sent:?}"
        );
    }

    #[test]
    fn a_cancel_and_a_disabled_slider_leave_the_actual_value() {
        let mut h = harness();
        let mut s = slider(&mut h, 50.0);
        s.input(&touch(1, Down, x_of(&s, 50.0), 132.0, 0), &mut h.ui);
        s.input(&touch(1, Move, x_of(&s, 90.0), 132.0, 16), &mut h.ui);
        assert_eq!(s.shown(), 90.0);
        assert_eq!(
            s.input(&touch(1, Cancel, 0.0, 0.0, 32), &mut h.ui),
            Some(SliderAction::Cancel)
        );
        assert_eq!(s.shown(), 50.0);
        // A parent that takes the contact does the same.
        s.input(&touch(1, Down, x_of(&s, 50.0), 132.0, 100), &mut h.ui);
        s.input(&touch(1, Move, x_of(&s, 10.0), 132.0, 116), &mut h.ui);
        assert_eq!(s.cancel(&h.ui), Some(SliderAction::Cancel));
        assert_eq!(s.shown(), 50.0);
        assert_eq!(s.cancel(&h.ui), None);

        let mut model = SliderModel::new("Brightness", 0.0, 100.0, 5.0, 50.0);
        model.enabled = false;
        s.update(&model, &mut h.ui);
        let before = h.all().len();
        assert_eq!(
            s.input(&touch(1, Down, x_of(&s, 70.0), 132.0, 200), &mut h.ui),
            None
        );
        assert_eq!(h.all().len(), before);
        let mut sink = SemanticsSink::new();
        s.semantics(&mut sink);
        let node = sink.find("Brightness").unwrap();
        assert!(!node.enabled);
        assert_eq!(node.value.as_deref(), Some("50"));
    }

    #[test]
    fn a_touch_on_the_track_goes_there_and_the_touch_target_is_64_high_around_a_thin_track() {
        let mut h = harness();
        let mut s = slider(&mut h, 0.0);
        assert!(s.hit().h >= 64.0);
        // 20 units above the 12-high track still takes the touch.
        assert_eq!(
            s.input(&touch(1, Down, x_of(&s, 70.0), 132.0 - 20.0, 0), &mut h.ui),
            Some(SliderAction::Preview(70.0))
        );
        assert_eq!(
            s.input(&touch(1, Up, x_of(&s, 70.0), 132.0, 30), &mut h.ui),
            Some(SliderAction::Commit(70.0))
        );
        assert_eq!(
            s.input(&touch(1, Down, 5.0, 5.0, 100), &mut h.ui),
            None,
            "outside the target there is nothing"
        );
    }

    #[test]
    fn the_thumb_is_raised_and_scales_while_the_track_stays_inset_dark_and_light() {
        for mode in [
            crate::lucent::palette::Mode::Dark,
            crate::lucent::palette::Mode::Light,
        ] {
            let mut h = harness();
            h.ui.look = crate::context::Look::standard(mode);
            let s = slider(&mut h, 30.0);
            let (rest, pressed) = s.surface().shown();
            assert!(
                rest.depth.is_some_and(|d| !d.inset) && rest.sheen.is_some(),
                "{mode:?}"
            );
            assert!(
                pressed.is_some_and(|p| p.depth.is_some_and(|d| !d.inset)),
                "{mode:?}: no inset jump on the thumb"
            );
            assert!(
                matches!(s.surface().pair().press, PressTreatment::Scale { .. }),
                "{mode:?}"
            );
            assert!(
                s.track.pair().rest.depth.is_some_and(|d| d.inset),
                "{mode:?}"
            );
            assert_eq!(
                s.track.pair().rest,
                s.track.pair().pressed,
                "{mode:?}: the track never presses"
            );
        }
    }
}
