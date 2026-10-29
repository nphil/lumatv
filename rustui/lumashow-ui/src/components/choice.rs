//! Choosing one of a few things, and stepping a number: `Choice` is a row of pills with one selected, `Stepper` is a number between a
//! minus and a plus button.
//!
//! Both PROPOSE: they say what the person asked for and keep showing what the model says is true. Neither draws anything new when a
//! selection or a value changes except what changed: a selection restyles surfaces and moves a check, a stepper draws its number
//! again. The pills and buttons are the toolkit's own [`Button`]s, so they press, cancel and take a 64-unit touch like every button.

use super::button::{Button, ButtonKind, ButtonModel};
use super::label::{Align, Label, style};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::slider::{check_range, number_text};
use super::text::ellipsise;
use super::{Component, Damage};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchEvent, TouchPhase};
use crate::lucent::material::MaterialContext;
use crate::lucent::tokens::{smart, space, ty};
use std::io;

/// Sends each touch to the one control it began on, and ignores a second finger while the first is down.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Router {
    pub(crate) owner: Option<(usize, u32)>,
}

impl Router {
    /// The index of the control that takes `touch`, among controls whose touch frames are `hits`.
    pub(crate) fn route(
        &mut self,
        hits: impl Iterator<Item = Frame>,
        touch: &TouchEvent,
    ) -> Option<usize> {
        match (touch.phase, self.owner) {
            (TouchPhase::Down, None) => {
                let i = hits
                    .into_iter()
                    .position(|h| h.contains(touch.x, touch.y))?;
                self.owner = Some((i, touch.id));
                Some(i)
            }
            (phase, Some((i, id))) if id == touch.id && phase != TouchPhase::Down => {
                if matches!(phase, TouchPhase::Up | TouchPhase::Cancel) {
                    self.owner = None;
                }
                Some(i)
            }
            _ => None,
        }
    }
}

pub(crate) fn bounds(frames: impl Iterator<Item = Frame>) -> Frame {
    let mut out: Option<Frame> = None;
    for f in frames {
        out = Some(match out {
            None => f,
            Some(o) => {
                let (x, y) = (o.x.min(f.x), o.y.min(f.y));
                Frame::new(
                    x,
                    y,
                    o.right().max(f.right()) - x,
                    o.bottom().max(f.bottom()) - y,
                )
            }
        });
    }
    out.unwrap_or_default()
}

// ---- Choice -----------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    /// What the option is called to the system: stable, never shown.
    pub id: String,
    /// What the person reads.
    pub label: String,
}

impl ChoiceOption {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceModel {
    pub options: Vec<ChoiceOption>,
    /// The id of the confirmed option, if any.
    pub selected: Option<String>,
    pub enabled: bool,
}

impl ChoiceModel {
    pub fn new(options: Vec<ChoiceOption>, selected: Option<&str>) -> Self {
        Self {
            options,
            selected: selected.map(str::to_owned),
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceAction {
    /// The person asks for this option (its id). The selected option is not proposed again.
    Propose(String),
}

/// How tall a pill is drawn, and how far apart rows of pills are (the touch target: pills of neighbouring rows never share a touch).
const PILL: f32 = 48.0;
const PITCH: f32 = smart::TARGET;

pub struct Choice {
    model: ChoiceModel,
    frame: Frame,
    z: i32,
    context: MaterialContext,
    buttons: Vec<Button>,
    bounds: Frame,
    router: Router,
}

impl Choice {
    fn widths(ui: &mut Ui, options: &[ChoiceOption], max: f32) -> Vec<f32> {
        options
            .iter()
            .map(|o| {
                Button::required_width(ui, ButtonKind::Chip, &ButtonModel::labelled(&o.label))
                    .max(smart::TARGET)
                    .min(max)
            })
            .collect()
    }

    /// Where each pill goes, relative to the top-left of the choice, and how many rows they take. If every pill fits an equal share
    /// of one row, they share it equally (a segmented control); otherwise each is as wide as its text needs and the pills flow onto
    /// further rows, so that large text makes the choice taller, not its labels shorter.
    fn layout(ui: &mut Ui, options: &[ChoiceOption], width: f32) -> (Vec<Frame>, usize) {
        let n = options.len();
        if n == 0 {
            return (Vec::new(), 0);
        }
        let need = Self::widths(ui, options, width);
        let gaps = space::S2 * (n - 1) as f32;
        let widest = need.iter().copied().fold(0.0, f32::max);
        let pad = (PITCH - PILL) / 2.0;
        if widest * n as f32 + gaps <= width {
            let each = (width - gaps) / n as f32;
            let frames = (0..n)
                .map(|i| Frame::new(i as f32 * (each + space::S2), pad, each, PILL))
                .collect();
            return (frames, 1);
        }
        let (mut x, mut row) = (0.0f32, 0usize);
        let frames = need
            .iter()
            .map(|&w| {
                if x > 0.0 && x + w > width {
                    row += 1;
                    x = 0.0;
                }
                let f = Frame::new(x, row as f32 * PITCH + pad, w, PILL);
                x += w + space::S2;
                f
            })
            .collect();
        (frames, row + 1)
    }

    /// The width a choice needs to put every option in one row, each as wide as its text needs.
    pub fn required_width(ui: &mut Ui, model: &ChoiceModel) -> f32 {
        let need = Self::widths(ui, &model.options, f32::MAX);
        need.iter().sum::<f32>() + space::S2 * need.len().saturating_sub(1) as f32
    }

    /// The height a choice needs in `width` at the person's text size: one row of pills, or as many as the labels need.
    pub fn required_height(ui: &mut Ui, model: &ChoiceModel, width: f32) -> f32 {
        Self::layout(ui, &model.options, width).1 as f32 * PITCH
    }

    /// A choice covering `frame`. `context` says whether it sits on a sheet that already has the glass backdrop.
    pub fn new(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        context: MaterialContext,
        model: &ChoiceModel,
    ) -> io::Result<Self> {
        let (frames, _) = Self::layout(ui, &model.options, frame.w);
        let mut buttons = Vec::with_capacity(frames.len());
        for (option, at) in model.options.iter().zip(&frames) {
            buttons.push(Button::chip(
                ui,
                at.moved(frame.x, frame.y),
                z,
                context,
                &Self::pill(model, option),
            )?);
        }
        Ok(Self {
            model: model.clone(),
            frame,
            z,
            context,
            bounds: bounds(buttons.iter().map(Button::hit)),
            buttons,
            router: Router::default(),
        })
    }

    fn pill(model: &ChoiceModel, option: &ChoiceOption) -> ButtonModel {
        ButtonModel::labelled(&option.label)
            .selected(model.selected.as_deref() == Some(option.id.as_str()))
            .enabled(model.enabled)
    }

    pub fn model(&self) -> &ChoiceModel {
        &self.model
    }

    /// The pills, in the order of the options.
    pub fn buttons(&self) -> &[Button] {
        &self.buttons
    }

    /// The pills, to move them with the page they sit on.
    pub fn buttons_mut(&mut self) -> &mut [Button] {
        &mut self.buttons
    }

    /// Ends a press that something else has taken: no proposal.
    pub fn cancel(&mut self, ui: &Ui) {
        self.router.owner = None;
        for b in &mut self.buttons {
            b.cancel(ui);
        }
    }

    /// The look was replaced: restyle every pill.
    pub fn restyle(&mut self, ui: &mut Ui) {
        for b in &mut self.buttons {
            b.restyle(ui);
        }
    }
}

impl Component for Choice {
    type Model = ChoiceModel;
    type Action = ChoiceAction;

    fn update(&mut self, model: &ChoiceModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        if model.options != self.model.options {
            // Other options need another layout and other layers. If they cannot be made the old ones stay, and the next update
            // tries again.
            return match Self::new(ui, self.frame, self.z, self.context, model) {
                Ok(new) => {
                    *self = new;
                    Damage::Content
                }
                Err(_) => Damage::None,
            };
        }
        // The same options: only selection and availability changed, and those only restyle surfaces.
        let mut damage = Damage::None;
        for (button, option) in self.buttons.iter_mut().zip(&model.options) {
            damage = damage.max(button.update(&Self::pill(model, option), ui));
        }
        if !model.enabled {
            self.router.owner = None;
        }
        self.model = model.clone();
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<ChoiceAction> {
        let Event::Touch(t) = event else {
            return None;
        };
        if !self.model.enabled {
            return None;
        }
        let i = self.router.route(self.buttons.iter().map(Button::hit), t)?;
        self.buttons[i].input(event, ui)?;
        let id = &self.model.options[i].id;
        (self.model.selected.as_deref() != Some(id.as_str()))
            .then(|| ChoiceAction::Propose(id.clone()))
    }

    #[allow(clippy::misnamed_getters)] // the frame a component takes a touch in is its hit area, not its drawn one
    fn frame(&self) -> Frame {
        self.bounds
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        for (button, option) in self.buttons.iter().zip(&self.model.options) {
            let selected = self.model.selected.as_deref() == Some(option.id.as_str());
            let node = SemanticNode::new(SemanticKind::Choice, option.label.clone(), button.hit())
                .selected(selected)
                .enabled(self.model.enabled);
            out.push(if self.model.enabled {
                node.actionable()
            } else {
                node
            });
        }
    }
}

// ---- Stepper ----------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct StepperModel {
    /// The confirmed value.
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// What the number is in ("min", "°C"): drawn after it. Empty for a plain number.
    pub unit: String,
    pub enabled: bool,
}

impl StepperModel {
    pub fn new(value: f32, min: f32, max: f32, step: f32, unit: impl Into<String>) -> Self {
        Self {
            value,
            min,
            max,
            step,
            unit: unit.into(),
            enabled: true,
        }
    }

    fn text(&self, value: f32) -> String {
        if self.unit.is_empty() {
            number_text(value)
        } else {
            format!("{} {}", number_text(value), self.unit)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepperAction {
    /// The person asks for this value: one step from the confirmed one, kept inside the range.
    Propose(f32),
}

const STEP_BUTTON_W: f32 = smart::TARGET;
const STEP_BUTTON_H: f32 = 48.0;

pub struct Stepper {
    model: StepperModel,
    frame: Frame,
    minus: Button,
    plus: Button,
    value: Label,
    text_w: f32,
    bounds: Frame,
    router: Router,
}

impl Stepper {
    /// The width a stepper needs to show its widest number (the value now, the min or the max) at the person's text size, between
    /// its two buttons.
    pub fn required_width(ui: &mut Ui, model: &StepperModel) -> f32 {
        let t = ui.look.ty(ty::TITLE);
        let widest = [model.value, model.min, model.max]
            .iter()
            .map(|&v| ui.fonts.measure(&model.text(v), t).width.ceil())
            .fold(0.0, f32::max);
        2.0 * (STEP_BUTTON_W + space::S2) + widest
    }

    /// A stepper covering `frame` (64 high is a comfortable height): the minus button at the left, the plus at the right, the
    /// number between. A range that runs backwards or a step that is not positive is an error.
    pub fn new(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        context: MaterialContext,
        model: &StepperModel,
    ) -> io::Result<Self> {
        check_range(model.min, model.max, model.step)?;
        let y = frame.y + (frame.h - STEP_BUTTON_H) / 2.0;
        let minus = Button::icon_button(
            ui,
            Frame::new(frame.x, y, STEP_BUTTON_W, STEP_BUTTON_H),
            z,
            context,
            &Self::minus_model(model),
        )?;
        let plus = Button::icon_button(
            ui,
            Frame::new(
                frame.right() - STEP_BUTTON_W,
                y,
                STEP_BUTTON_W,
                STEP_BUTTON_H,
            ),
            z,
            context,
            &Self::plus_model(model),
        )?;
        let text_w = (frame.w - 2.0 * (STEP_BUTTON_W + space::S2)).max(0.0);
        let t = ui.look.ty(ty::TITLE);
        let text = ellipsise(&mut ui.fonts, &model.text(model.value), t, text_w);
        let value = Label::new(
            ui,
            Frame::new(
                frame.x + STEP_BUTTON_W + space::S2,
                frame.y,
                text_w,
                frame.h,
            ),
            z + 1,
            style(t, Self::ink(ui, model), Align::Centre),
            &text,
        )?;
        Ok(Self {
            model: model.clone(),
            frame,
            bounds: bounds([minus.hit(), plus.hit()].into_iter()),
            minus,
            plus,
            value,
            text_w,
            router: Router::default(),
        })
    }

    fn ink(ui: &Ui, model: &StepperModel) -> compositor_protocol::Rgba {
        if model.enabled {
            ui.look.ink()
        } else {
            ui.look.ink2()
        }
    }

    fn minus_model(model: &StepperModel) -> ButtonModel {
        ButtonModel::icon(super::icon::Icon::Minus, "Decrease")
            .enabled(model.enabled && model.value > model.min)
    }

    fn plus_model(model: &StepperModel) -> ButtonModel {
        ButtonModel::icon(super::icon::Icon::Plus, "Increase")
            .enabled(model.enabled && model.value < model.max)
    }

    /// One step from `value`, inside the range, without the dust of repeated float addition.
    fn stepped(&self, direction: f32) -> f32 {
        let m = &self.model;
        let v = (m.value + direction * m.step).clamp(m.min, m.max);
        (v * 10_000.0).round() / 10_000.0
    }

    pub fn model(&self) -> &StepperModel {
        &self.model
    }

    pub fn minus(&self) -> &Button {
        &self.minus
    }

    pub fn plus(&self) -> &Button {
        &self.plus
    }

    /// The number as drawn now (after any ellipsis).
    pub fn shown_text(&self) -> &str {
        self.value.text()
    }

    /// Ends a press that something else has taken: no proposal.
    pub fn cancel(&mut self, ui: &Ui) {
        self.router.owner = None;
        self.minus.cancel(ui);
        self.plus.cancel(ui);
    }

    /// The look was replaced: restyle the buttons and draw the number in the new colour.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.minus.restyle(ui);
        self.plus.restyle(ui);
        let ink = Self::ink(ui, &self.model);
        self.value.set_colour(ui, ink);
    }
}

impl Component for Stepper {
    type Model = StepperModel;
    type Action = StepperAction;

    fn update(&mut self, model: &StepperModel, ui: &mut Ui) -> Damage {
        if *model == self.model || check_range(model.min, model.max, model.step).is_err() {
            return Damage::None;
        }
        let mut damage = Damage::None;
        damage = damage.max(self.minus.update(&Self::minus_model(model), ui));
        damage = damage.max(self.plus.update(&Self::plus_model(model), ui));
        let t = ui.look.ty(ty::TITLE);
        let ink = Self::ink(ui, model);
        self.value.set_colour(ui, ink);
        let text = ellipsise(&mut ui.fonts, &model.text(model.value), t, self.text_w);
        if self.value.text() != text {
            self.value.set(ui, &text);
            damage = Damage::Content;
        }
        if !model.enabled {
            self.router.owner = None;
        }
        self.model = model.clone();
        damage.max(Damage::Properties)
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<StepperAction> {
        let Event::Touch(t) = event else {
            return None;
        };
        if !self.model.enabled {
            return None;
        }
        let i = self
            .router
            .route([self.minus.hit(), self.plus.hit()].into_iter(), t)?;
        let (button, direction) = if i == 0 {
            (&mut self.minus, -1.0)
        } else {
            (&mut self.plus, 1.0)
        };
        button.input(event, ui)?;
        let value = self.stepped(direction);
        (value != self.model.value).then_some(StepperAction::Propose(value))
    }

    #[allow(clippy::misnamed_getters)] // the frame a component takes a touch in is its hit area, not its drawn one
    fn frame(&self) -> Frame {
        self.bounds
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        self.minus.semantics(out);
        out.push(SemanticNode::new(
            SemanticKind::Label,
            self.model.text(self.model.value),
            self.frame,
        ));
        self.plus.semantics(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchPhase::{Cancel, Down, Move, Up};
    use crate::testing::{Harness, harness, touch};
    use compositor_protocol::Request;

    fn options() -> Vec<ChoiceOption> {
        vec![
            ChoiceOption::new("dark", "Dark"),
            ChoiceOption::new("light", "Light"),
            ChoiceOption::new("auto", "Auto"),
        ]
    }

    const AT: Frame = Frame::new(50.0, 50.0, 480.0, 64.0);

    fn choice(h: &mut Harness, selected: &str) -> Choice {
        let c = Choice::new(
            &mut h.ui,
            AT,
            10,
            MaterialContext::Nested,
            &ChoiceModel::new(options(), Some(selected)),
        )
        .unwrap();
        h.commit();
        c
    }

    fn tap(c: &mut Choice, h: &mut Harness, x: f32, y: f32) -> Option<ChoiceAction> {
        c.input(&touch(1, Down, x, y, 0), &mut h.ui);
        c.input(&touch(1, Up, x, y, 40), &mut h.ui)
    }

    #[test]
    fn a_tap_on_another_option_proposes_its_id_and_a_tap_on_the_selected_one_proposes_nothing() {
        let mut h = harness();
        let mut c = choice(&mut h, "dark");
        let centres: Vec<_> = c.buttons().iter().map(|b| b.hit().centre()).collect();
        assert_eq!(
            tap(&mut c, &mut h, centres[1].0, centres[1].1),
            Some(ChoiceAction::Propose("light".into()))
        );
        assert_eq!(tap(&mut c, &mut h, centres[0].0, centres[0].1), None);
        assert!(
            c.model().selected.as_deref() == Some("dark"),
            "a proposal is not a selection"
        );
    }

    #[test]
    fn selecting_restyles_the_surfaces_and_draws_nothing() {
        let mut h = harness();
        let mut c = choice(&mut h, "dark");
        let next = ChoiceModel::new(options(), Some("light"));
        assert_eq!(c.update(&next, &mut h.ui), Damage::Properties);
        assert_eq!(c.update(&next, &mut h.ui), Damage::None);
        let sent = h.commit();
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::Damage { .. } | Request::Create { .. })),
            "{sent:?}"
        );
        let wash = h.ui.look.theme.materials.glass_raised_selected;
        assert_eq!(c.buttons()[1].surface().pair().rest.tint, wash);
        assert_ne!(c.buttons()[0].surface().pair().rest.tint, wash);
    }

    #[test]
    fn a_drag_out_a_cancel_and_a_disabled_choice_propose_nothing() {
        let mut h = harness();
        let mut c = choice(&mut h, "dark");
        let (x, y) = c.buttons()[2].hit().centre();
        c.input(&touch(1, Down, x, y, 0), &mut h.ui);
        c.input(&touch(1, Move, x, y + 200.0, 16), &mut h.ui);
        assert_eq!(c.input(&touch(1, Up, x, y + 200.0, 32), &mut h.ui), None);
        c.input(&touch(1, Down, x, y, 100), &mut h.ui);
        assert_eq!(c.input(&touch(1, Cancel, 0.0, 0.0, 110), &mut h.ui), None);
        c.input(&touch(1, Down, x, y, 200), &mut h.ui);
        c.cancel(&h.ui);
        assert_eq!(c.input(&touch(1, Up, x, y, 210), &mut h.ui), None);

        let mut off = ChoiceModel::new(options(), Some("dark"));
        off.enabled = false;
        c.update(&off, &mut h.ui);
        assert_eq!(tap(&mut c, &mut h, x, y), None);
    }

    #[test]
    fn large_text_flows_the_pills_onto_more_rows_and_every_label_stays_whole() {
        let mut h = harness();
        let model = ChoiceModel::new(
            vec![
                ChoiceOption::new("a", "Every morning"),
                ChoiceOption::new("b", "Weekdays"),
                ChoiceOption::new("c", "Weekends only"),
            ],
            Some("a"),
        );
        let one_row = Choice::required_height(&mut h.ui, &model, 640.0);
        h.ui.look.text_scale = 2.0;
        let more = Choice::required_height(&mut h.ui, &model, 420.0);
        assert!(more > one_row, "{one_row} -> {more}");
        let frame = Frame::new(20.0, 20.0, 420.0, more);
        let c = Choice::new(&mut h.ui, frame, 10, MaterialContext::Nested, &model).unwrap();
        for b in c.buttons() {
            assert!(!b.model().label.is_empty());
            assert!(
                b.hit().right() <= frame.right() + 1.0 && b.hit().bottom() <= frame.bottom() + 1.0
            );
        }
        h.commit();
    }

    #[test]
    fn the_pills_are_chips_with_the_depth_pair_and_the_selected_one_shows_its_check() {
        let mut h = harness();
        let c = choice(&mut h, "light");
        for b in c.buttons() {
            let (rest, pressed) = b.surface().shown();
            assert!(rest.depth.is_some_and(|d| !d.inset) && rest.sheen.is_some());
            assert!(pressed.is_some_and(|p| p.depth.is_some_and(|d| d.inset)));
        }
        let mut sink = SemanticsSink::new();
        c.semantics(&mut sink);
        assert!(sink.find("Light").unwrap().selected);
        assert!(!sink.find("Dark").unwrap().selected);
    }

    fn stepper(h: &mut Harness, value: f32) -> Stepper {
        let s = Stepper::new(
            &mut h.ui,
            Frame::new(50.0, 50.0, 360.0, 64.0),
            10,
            MaterialContext::Nested,
            &StepperModel::new(value, 1.0, 60.0, 5.0, "min"),
        )
        .unwrap();
        h.commit();
        s
    }

    #[test]
    fn plus_and_minus_propose_one_step_inside_the_range_and_the_ends_disable_a_button() {
        let mut h = harness();
        let mut s = stepper(&mut h, 10.0);
        let (px, py) = s.plus().hit().centre();
        let (mx, my) = s.minus().hit().centre();
        s.input(&touch(1, Down, px, py, 0), &mut h.ui);
        assert_eq!(
            s.input(&touch(1, Up, px, py, 30), &mut h.ui),
            Some(StepperAction::Propose(15.0))
        );
        assert_eq!(
            s.model().value,
            10.0,
            "the confirmed value is unchanged until the model says so"
        );
        s.input(&touch(1, Down, mx, my, 100), &mut h.ui);
        assert_eq!(
            s.input(&touch(1, Up, mx, my, 130), &mut h.ui),
            Some(StepperAction::Propose(5.0))
        );

        // At the end the button is disabled and the value would not move.
        let at_max = StepperModel::new(60.0, 1.0, 60.0, 5.0, "min");
        s.update(&at_max, &mut h.ui);
        assert!(!s.plus().model().state.enabled && s.minus().model().state.enabled);
        s.input(&touch(1, Down, px, py, 200), &mut h.ui);
        assert_eq!(s.input(&touch(1, Up, px, py, 230), &mut h.ui), None);
        // From 1 a step of 5 down is clamped to the min, which is the value: nothing proposed.
        s.update(&StepperModel::new(1.0, 1.0, 60.0, 5.0, "min"), &mut h.ui);
        s.input(&touch(1, Down, mx, my, 300), &mut h.ui);
        assert_eq!(s.input(&touch(1, Up, mx, my, 330), &mut h.ui), None);
    }

    #[test]
    fn the_number_is_drawn_again_only_when_it_changes_and_float_dust_is_not_proposed() {
        let mut h = harness();
        let mut s = Stepper::new(
            &mut h.ui,
            Frame::new(50.0, 50.0, 360.0, 64.0),
            10,
            MaterialContext::Nested,
            &StepperModel::new(0.1, 0.0, 1.0, 0.1, ""),
        )
        .unwrap();
        h.commit();
        assert_eq!(s.shown_text(), "0.1");
        let (px, py) = s.plus().hit().centre();
        s.input(&touch(1, Down, px, py, 0), &mut h.ui);
        let proposed = s.input(&touch(1, Up, px, py, 30), &mut h.ui);
        assert_eq!(
            proposed,
            Some(StepperAction::Propose(0.2)),
            "0.1 + 0.1 is 0.2, not 0.20000000298"
        );
        assert_eq!(
            s.update(&StepperModel::new(0.1, 0.0, 1.0, 0.1, ""), &mut h.ui),
            Damage::None
        );
        assert_eq!(
            s.update(&StepperModel::new(0.2, 0.0, 1.0, 0.1, ""), &mut h.ui),
            Damage::Content
        );
        assert_eq!(s.shown_text(), "0.2");
    }

    #[test]
    fn a_stepper_with_a_bad_range_is_an_error_and_its_number_fits_at_large_text() {
        let mut h = harness();
        let bad = StepperModel::new(5.0, 10.0, 1.0, 1.0, "");
        assert!(
            Stepper::new(
                &mut h.ui,
                Frame::new(0.0, 0.0, 300.0, 64.0),
                10,
                MaterialContext::Nested,
                &bad
            )
            .is_err()
        );
        let model = StepperModel::new(45.0, 1.0, 120.0, 5.0, "min");
        h.ui.look.text_scale = 2.0;
        let need = Stepper::required_width(&mut h.ui, &model);
        let s = Stepper::new(
            &mut h.ui,
            Frame::new(0.0, 0.0, need, 64.0),
            10,
            MaterialContext::Nested,
            &model,
        )
        .unwrap();
        assert_eq!(
            s.shown_text(),
            "45 min",
            "the number is whole at the width it asked for"
        );
        h.commit();
    }
}
