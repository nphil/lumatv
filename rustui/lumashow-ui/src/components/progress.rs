//! The progress bar: how far something has got, as a recessed track with an accent fill, and an optional line of text above it.
//!
//! The fill is a full-width accent glass segment of which only the part up to `fraction` is shown, so progress is a moving clip
//! rectangle and never a new layer or a restyle. The label is drawn when its text changes and not otherwise. A bar takes no touch.

use super::group::{Group, Part, Pose};
use super::label::{Align, Label, style};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::text::{ellipsise, line_height};
use super::toggle::Well;
use super::{Component, Damage};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::tokens::{space, ty};
use compositor_protocol::Rect;
use std::convert::Infallible;
use std::io;

/// What a progress bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressModel {
    /// How far, 0 to 1. Anything outside is clamped, and a number that is not a number is 0.
    pub fraction: f32,
    /// What is progressing ("Downloading update"), above the bar.
    pub label: Option<String>,
}

impl ProgressModel {
    pub fn new(fraction: f32) -> Self {
        Self {
            fraction,
            label: None,
        }
    }

    pub fn labelled(fraction: f32, label: impl Into<String>) -> Self {
        Self {
            fraction,
            label: Some(label.into()),
        }
    }

    /// The fraction as shown: inside 0..=1, and 0 for NaN.
    pub fn shown_fraction(&self) -> f32 {
        if self.fraction.is_nan() {
            0.0
        } else {
            self.fraction.clamp(0.0, 1.0)
        }
    }
}

pub const BAR_HEIGHT: f32 = 12.0;

pub struct Progress {
    model: ProgressModel,
    frame: Frame,
    bar: Frame,
    track: Well,
    fill: Well,
    /// The label, in a group of its own so that it can be hidden when the model has none.
    label: Group,
    fraction: f32,
}

impl Progress {
    /// The height a bar needs: the bar, and above it a line at the person's text size and a gap when it has a label. Large text makes
    /// the area taller; it never makes the label smaller.
    pub fn required_height(ui: &Ui, model: &ProgressModel) -> f32 {
        BAR_HEIGHT
            + if model.label.is_some() {
                space::S2 + line_height(ui.look.ty(ty::LABEL))
            } else {
                0.0
            }
    }

    /// A bar in `frame`: the track is along the bottom of it, as wide as the frame, and a label sits above.
    pub fn new(ui: &mut Ui, frame: Frame, z: i32, model: &ProgressModel) -> io::Result<Self> {
        let bar = Frame::new(frame.x, frame.bottom() - BAR_HEIGHT, frame.w, BAR_HEIGHT);
        let state = ControlState::default();
        let track = Well::new(
            ui,
            bar,
            z,
            SmartMaterial::SliderTrack,
            MaterialContext::Nested,
            state,
        );
        let fill = Well::new(
            ui,
            bar,
            z + 1,
            SmartMaterial::SliderFill,
            MaterialContext::Nested,
            state,
        );
        let label_ty = ui.look.ty(ty::LABEL);
        let line = line_height(label_ty);
        let label_frame = Frame::new(frame.x, bar.y - space::S2 - line, frame.w, line);
        let text = match &model.label {
            Some(text) => ellipsise(&mut ui.fonts, text, label_ty, frame.w),
            None => String::new(),
        };
        let layer = Label::new(
            ui,
            label_frame,
            z + 1,
            style(label_ty, ui.look.ink2(), Align::Left),
            &text,
        )?;
        let mut label = Group::new(label_frame.centre());
        label.push(Part::new(layer, label_frame));
        let mut progress = Self {
            model: model.clone(),
            frame,
            bar,
            track,
            fill,
            label,
            fraction: -1.0,
        };
        progress.show_label(model.label.is_some());
        progress.show_fraction(model.shown_fraction());
        Ok(progress)
    }

    fn show_fraction(&mut self, fraction: f32) {
        self.fraction = fraction;
        let b = self.bar;
        self.fill.show(
            Some(Rect {
                x: b.x,
                y: b.y,
                w: b.w * fraction,
                h: b.h,
            }),
            fraction > 0.0,
        );
    }

    fn show_label(&mut self, shown: bool) {
        self.label
            .set(if shown { Pose::REST } else { Pose::hidden() });
    }

    pub fn model(&self) -> &ProgressModel {
        &self.model
    }

    /// The fraction the bar shows, clamped.
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn bar_frame(&self) -> Frame {
        self.bar
    }

    /// The look was replaced: restyle the glass and draw the label in the new colour.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.track.restyle(ui);
        self.fill.restyle(ui);
        let ink = ui.look.ink2();
        if let Some(label) = self.label.label_mut(0) {
            label.set_colour(ui, ink);
        }
    }
}

impl Component for Progress {
    type Model = ProgressModel;
    /// A bar takes no touch and says nothing.
    type Action = Infallible;

    fn update(&mut self, model: &ProgressModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let mut damage = Damage::None;
        let fraction = model.shown_fraction();
        if fraction != self.fraction {
            self.show_fraction(fraction);
            damage = Damage::Properties;
        }
        if model.label != self.model.label {
            if let Some(text) = &model.label {
                let label_ty = ui.look.ty(ty::LABEL);
                let text = ellipsise(&mut ui.fonts, text, label_ty, self.frame.w);
                if let Some(label) = self.label.label_mut(0) {
                    label.set(ui, &text);
                }
            }
            self.show_label(model.label.is_some());
            damage = Damage::Content;
        }
        self.model = model.clone();
        damage
    }

    fn input(&mut self, _event: &Event, _ui: &mut Ui) -> Option<Infallible> {
        None
    }

    fn frame(&self) -> Frame {
        self.frame
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        let name = self
            .model
            .label
            .clone()
            .unwrap_or_else(|| "Progress".to_owned());
        out.push(
            SemanticNode::new(SemanticKind::Progress, name, self.frame)
                .value(format!("{}%", (self.fraction * 100.0).round())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, harness};
    use compositor_protocol::Request;

    const AT: Frame = Frame::new(40.0, 60.0, 400.0, 48.0);

    fn bar(h: &mut Harness, model: &ProgressModel) -> Progress {
        let p = Progress::new(&mut h.ui, AT, 10, model).unwrap();
        h.commit();
        p
    }

    #[test]
    fn the_fraction_is_clamped_and_progress_is_a_moving_clip_never_a_new_layer_or_pixels() {
        let mut h = harness();
        let mut p = bar(&mut h, &ProgressModel::new(0.25));
        assert_eq!(p.fraction(), 0.25);
        assert_eq!(p.update(&ProgressModel::new(0.25), &mut h.ui), Damage::None);
        for f in [0.5, 1.7, -3.0, f32::NAN] {
            p.update(&ProgressModel::new(f), &mut h.ui);
            assert!(
                (0.0..=1.0).contains(&p.fraction()),
                "{f} -> {}",
                p.fraction()
            );
        }
        p.update(&ProgressModel::new(0.75), &mut h.ui);
        assert_eq!(p.fraction(), 0.75);
        let sent = h.commit();
        assert!(
            sent.iter().all(|r| matches!(
                r,
                Request::Set { .. } | Request::Animate { .. } | Request::Commit { .. }
            )),
            "{sent:?}"
        );
        let clip = sent
            .iter()
            .rev()
            .find_map(|r| match r {
                Request::Set { props, .. } => props.clip,
                _ => None,
            })
            .expect("the fill is clipped");
        assert!((clip.w - 300.0).abs() < 0.01, "75% of 400: {}", clip.w);
    }

    #[test]
    fn the_label_is_drawn_when_its_words_change_and_a_bar_without_one_has_no_text() {
        let mut h = harness();
        let mut p = bar(&mut h, &ProgressModel::labelled(0.1, "Downloading 10%"));
        assert_eq!(
            p.update(&ProgressModel::labelled(0.2, "Downloading 10%"), &mut h.ui),
            Damage::Properties
        );
        assert_eq!(
            p.update(&ProgressModel::labelled(0.2, "Downloading 20%"), &mut h.ui),
            Damage::Content
        );
        let sent = h.commit();
        assert_eq!(
            sent.iter()
                .filter(|r| matches!(r, Request::Damage { .. }))
                .count(),
            1
        );
        let mut sink = SemanticsSink::new();
        p.semantics(&mut sink);
        assert_eq!(
            sink.find("Downloading 20%").unwrap().value.as_deref(),
            Some("20%")
        );
        let mut quiet = bar(&mut h, &ProgressModel::new(0.5));
        let mut sink = SemanticsSink::new();
        quiet.semantics(&mut sink);
        assert_eq!(sink.find("Progress").unwrap().value.as_deref(), Some("50%"));
        assert_eq!(
            quiet.input(
                &crate::testing::touch(1, crate::input::TouchPhase::Down, 100.0, 90.0, 0),
                &mut h.ui
            ),
            None
        );
    }

    #[test]
    fn large_text_makes_the_area_taller_and_a_label_that_is_too_long_is_cut() {
        let mut h = harness();
        let m = ProgressModel::labelled(0.4, "Checking for a newer system image to install");
        let normal = Progress::required_height(&h.ui, &m);
        h.ui.look.text_scale = 2.0;
        let large = Progress::required_height(&h.ui, &m);
        assert!(large > normal + 10.0, "{normal} -> {large}");
        let frame = Frame::new(0.0, 0.0, 240.0, large);
        let p = Progress::new(&mut h.ui, frame, 10, &m).unwrap();
        let shown = p.label.parts[0].frame;
        assert!(
            shown.y >= 0.0 && shown.bottom() <= p.bar_frame().y,
            "the label sits above the bar inside the area"
        );
        h.commit();
    }

    #[test]
    fn the_track_is_inset_and_the_fill_is_the_accent_in_both_modes() {
        for mode in [
            crate::lucent::palette::Mode::Dark,
            crate::lucent::palette::Mode::Light,
        ] {
            let mut h = harness();
            h.ui.look = crate::context::Look::standard(mode);
            let p = bar(&mut h, &ProgressModel::new(0.5));
            assert!(
                p.track.pair().rest.depth.is_some_and(|d| d.inset),
                "{mode:?}"
            );
            assert_eq!(
                p.fill.pair().rest.tint,
                h.ui.look.theme.materials.accent,
                "{mode:?}"
            );
        }
    }
}
