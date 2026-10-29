//! Buttons: the settings button, the one primary action, the pill chip and the icon button. All four are one component with a
//! kind: they differ in their material role and in how their content sits, not in how they respond.
//!
//! A button commits on release inside and cancels on a drag out or a lost contact; it shows its press at once, as the crossfade of
//! its prepared rest and pressed glass. The label is rastered once and again only when its text or colour changes.

use super::group::{Group, Movable, Part, Pose};
use super::icon::Icon;
use super::label::{Align, Label, style};
use super::press::{PressTracker, Step};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::{Component, Damage, target_around};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::palette::PaletteRole;
use crate::lucent::tokens::{smart, space, ty};
use compositor_protocol::Rgba;
use std::io;

/// Which button it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// A settings button: raised glass with a label.
    Standard,
    /// The one primary action of a task, on the accent.
    Primary,
    /// A pill with a label (Back, Close, a filter, a status): selected when its state says so.
    Chip,
    /// A pill with an icon only; its `name` is what it is called.
    Icon,
    /// A round chip with one letter, a member of a short set (a day of a repeat): [`Button::DAY`] across, in and out like a chip.
    /// Its `name`, when it has one, is what it is called (the letter is not enough: two days start with T).
    Day,
}

impl ButtonKind {
    fn material(self) -> SmartMaterial {
        match self {
            ButtonKind::Standard => SmartMaterial::Button,
            ButtonKind::Primary => SmartMaterial::PrimaryButton,
            ButtonKind::Chip | ButtonKind::Icon | ButtonKind::Day => SmartMaterial::StatusButton,
        }
    }
}

/// What a button shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonModel {
    pub label: String,
    /// A leading icon (the only content of an icon button).
    pub icon: Option<Icon>,
    /// Selected, enabled, pending and focused. A chip that is selected shows a check as well as its wash: no meaning is the
    /// colour alone.
    pub state: ControlState,
    /// What the button is called when it has no label (an icon button).
    pub name: Option<String>,
}

impl ButtonModel {
    pub fn labelled(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            state: ControlState::default(),
            name: None,
        }
    }

    pub fn icon(icon: Icon, name: impl Into<String>) -> Self {
        Self {
            label: String::new(),
            icon: Some(icon),
            state: ControlState::default(),
            name: Some(name.into()),
        }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.state.selected = selected;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.state.enabled = enabled;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonAction {
    /// Released inside: do the thing.
    Activated,
}

const ICON: f32 = 24.0;

/// The type of a button's label. In the Settings' page look (`Look::page`) every button that has words is the approved mock's `.btn`:
/// 22 px in the heavier weight (the mock's 650; the Show has 400, 500 and 600). Elsewhere a chip is the label type and every other
/// button the button type, both at the Medium weight.
fn label_type(ui: &Ui, kind: ButtonKind) -> crate::text::Type {
    if ui.look.page {
        // The mock's day chip is 20 px; every other button with words is 22.
        let px = if kind == ButtonKind::Day { 20 } else { 22 };
        ui.look.page_ty(px, crate::text::Weight::SemiBold)
    } else {
        ui.look
            .ty(if matches!(kind, ButtonKind::Chip | ButtonKind::Day) {
                ty::LABEL
            } else {
                ty::BUTTON
            })
    }
}

/// The glass of a button: its kind's. In the Settings' page look a chip with a glyph and words (the way back, Search) is the mock's
/// `.btn.back`: the same raised glass with corners of 14 as every other `.btn`, and not the pill of a status, and so is a button with
/// only a glyph (the mock's `.ib`: 56 square, corners of 14); a chip without a glyph (a day) keeps its pill.
fn material_of(ui: &Ui, kind: ButtonKind, glyph_and_words: bool) -> SmartMaterial {
    if ui.look.page && (kind == ButtonKind::Icon || kind == ButtonKind::Chip && glyph_and_words) {
        SmartMaterial::Button
    } else {
        kind.material()
    }
}

/// Whether a chip keeps a slot at its start for a check that shows while it is selected, so that no meaning is the colour alone.
/// The Settings' page look has none: its chips are the approved mock's, the words centred in the glass, and what is selected is
/// the accent wash (an unselected day would otherwise stand with a hole at its start, 32 wide, which the words are not centred
/// round).
fn shows_check(ui: &Ui, kind: ButtonKind) -> bool {
    kind == ButtonKind::Chip && !ui.look.page
}

/// The room a button keeps at its left and its right, and between its glyph and its words. A page button is the mock's `.btn`
/// (24 at each side, 10 after the glyph); the mock's Back button, a chip with a glyph, is `.btn.back` (14 before the glyph, 6 after
/// it, 22 at the end); a chip without one keeps the toolkit's 16 and 8, as does every button outside the Settings.
struct Room {
    left: f32,
    right: f32,
    gap: f32,
}

fn room(ui: &Ui, kind: ButtonKind, has_glyph: bool) -> Room {
    match (ui.look.page, kind) {
        // A day is its letter, centred in the circle: no room at the sides.
        (_, ButtonKind::Day) => Room {
            left: 0.0,
            right: 0.0,
            gap: 0.0,
        },
        (true, ButtonKind::Chip) if has_glyph => Room {
            left: 14.0,
            right: 22.0,
            gap: 6.0,
        },
        (true, ButtonKind::Standard | ButtonKind::Primary) => Room {
            left: space::S6,
            right: space::S6,
            gap: 10.0,
        },
        _ => Room {
            left: space::S4,
            right: space::S4,
            gap: space::S2,
        },
    }
}

pub struct Button {
    kind: ButtonKind,
    hit: Frame,
    model: ButtonModel,
    group: Group,
    surface: Surface,
    label: Option<usize>,
    icon: Option<usize>,
    /// The check a selected chip shows, always present for a chip so that selecting it draws nothing new.
    check: Option<usize>,
    press: PressTracker,
    /// The width the label's text may take.
    text_w: f32,
    /// A palette role for the label that replaces the kind's own while the button is enabled (a destructive action's danger
    /// ink); a role, not a colour, so that a new theme restyles it.
    ink: Option<PaletteRole>,
}

impl Button {
    /// The width and height of a [`ButtonKind::Day`]: the mock's day chip, 48 round.
    pub const DAY: f32 = 48.0;

    /// A button of `kind` covering `frame`. `context` says whether it sits on a sheet that already has the glass backdrop.
    pub fn new(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        kind: ButtonKind,
        context: MaterialContext,
        model: &ButtonModel,
    ) -> io::Result<Self> {
        let mut group = Group::new(frame.centre());
        let surface = Surface::add(
            ui,
            &mut group,
            frame,
            z,
            material_of(ui, kind, model.icon.is_some() && !model.label.is_empty()),
            context,
            model.state,
        );
        let icon_only = kind == ButtonKind::Icon || model.label.is_empty();
        let has_check = shows_check(ui, kind);
        let has_glyph = has_check || model.icon.is_some();
        let room = room(ui, kind, model.icon.is_some());
        let pad = room.left;
        let lead = if has_glyph { ICON + room.gap } else { 0.0 };
        let text_w =
            (frame.w - room.left - room.right - if icon_only { 0.0 } else { lead }).max(0.0);
        let ink = content_ink(ui, kind, &model.state);
        let mut icon = None;
        let mut check = None;
        if model.icon.is_some() || has_check {
            // A chip's lead slot shows its icon, or the check when selected; a button's shows its icon.
            let x = if icon_only {
                frame.x + (frame.w - ICON) / 2.0
            } else {
                frame.x + pad
            };
            let slot = Frame::new(x, frame.y + (frame.h - ICON) / 2.0, ICON, ICON);
            if let Some(i) = model.icon {
                let mut part = super::icon::icon_part(ui, i, slot, z + 1, ink)?;
                part.rest_opacity = 1.0;
                icon = Some(group.push(part));
            }
            if has_check && model.icon.is_none() {
                let mut part = super::icon::icon_part(ui, Icon::Check, slot, z + 1, ink)?;
                part.rest_opacity = if model.state.selected { 1.0 } else { 0.0 };
                let slot_index = group.push(part);
                // The layer is created fully opaque; the part's resting opacity is applied now, so an unselected chip's check
                // is not visible for the first frame.
                group.set_parts(&[slot_index], Pose::REST);
                check = Some(slot_index);
            }
        }
        let label = if icon_only {
            None
        } else {
            let (x, align) = if lead > 0.0 {
                (frame.x + pad + lead, Align::Left)
            } else {
                (frame.x + pad, Align::Centre)
            };
            let ty = label_type(ui, kind);
            let text = ellipsise_for(ui, &model.label, ty, text_w);
            let l = Label::new(
                ui,
                Frame::new(x, frame.y, text_w, frame.h),
                z + 1,
                style(ty, ink, align),
                &text,
            )?;
            Some(group.push(Part::new(l, Frame::new(x, frame.y, text_w, frame.h))))
        };
        Ok(Self {
            kind,
            hit: target_around(frame, smart::TARGET),
            model: model.clone(),
            group,
            surface,
            label,
            icon,
            check,
            press: PressTracker::new(),
            text_w,
            ink: None,
        })
    }

    /// The width a button of `kind` needs to show `model` whole at the person's text size: padding, the lead icon slot and the
    /// text. A screen sizes its buttons with this, so that large text makes a button wider and never a label shorter.
    pub fn required_width(ui: &mut Ui, kind: ButtonKind, model: &ButtonModel) -> f32 {
        let room = room(ui, kind, model.icon.is_some());
        if kind == ButtonKind::Day {
            return Self::DAY;
        }
        if kind == ButtonKind::Icon || model.label.is_empty() {
            return (ICON + room.left + room.right).max(smart::TARGET);
        }
        let lead = if shows_check(ui, kind) || model.icon.is_some() {
            ICON + room.gap
        } else {
            0.0
        };
        let ty = label_type(ui, kind);
        room.left + room.right + lead + ui.fonts.measure(&model.label, ty).width.ceil()
    }

    /// A pill with a label.
    pub fn chip(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        context: MaterialContext,
        model: &ButtonModel,
    ) -> io::Result<Self> {
        Self::new(ui, frame, z, ButtonKind::Chip, context, model)
    }

    /// A pill with only an icon.
    pub fn icon_button(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        context: MaterialContext,
        model: &ButtonModel,
    ) -> io::Result<Self> {
        Self::new(ui, frame, z, ButtonKind::Icon, context, model)
    }

    pub fn kind(&self) -> ButtonKind {
        self.kind
    }

    pub fn model(&self) -> &ButtonModel {
        &self.model
    }

    /// Where a touch counts: the button, and at least 64 units around its centre in each direction.
    pub fn hit(&self) -> Frame {
        self.hit
    }

    pub fn group(&self) -> &Group {
        &self.group
    }

    pub fn group_mut(&mut self) -> &mut Group {
        &mut self.group
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Whether the button is drawn pressed.
    pub fn is_pressed(&self) -> bool {
        self.press.is_pressed()
    }

    /// Ends a press that something else has taken (a list that began to scroll): no activation.
    pub fn cancel(&mut self, ui: &Ui) {
        if self.press.cancel() {
            self.surface.set_down(ui, &mut self.group, false);
        }
    }

    /// The colour the label is drawn in now.
    #[cfg(test)]
    pub(crate) fn label_colour(&mut self) -> Option<Rgba> {
        let i = self.label?;
        self.group.label_mut(i).map(|l| l.style().colour)
    }

    /// The parts that show: all of them but the focus ring, which only a focused button shows.
    fn live(&self) -> Vec<usize> {
        let ring = self.surface.ring_part();
        (0..self.group.parts.len())
            .filter(|&i| i != ring || self.model.state.focus_visible)
            .collect()
    }

    /// Draws the button pressed or at rest for a host that follows the contacts itself and decides what is under a finger (a
    /// screen with a [`super::PressSet`], where the recogniser's tap is the action). The button's own tracker is not involved, so
    /// this and [`Component::input`] are not mixed on one button.
    pub fn show_pressed(&mut self, ui: &Ui, pressed: bool) {
        self.surface.set_down(ui, &mut self.group, pressed);
    }

    /// Moves the button and all it holds to `pose` at once. The focus ring stays away unless the button is focused.
    pub fn set_pose(&mut self, pose: Pose) {
        self.surface.set_rest(pose);
        let live = self.live();
        self.group.set_parts(&live, pose);
    }

    /// Moves the button and all it holds to `pose` over `ms`, on the compositor's clock (a dialog's buttons arriving with it).
    pub fn animate_pose(&mut self, pose: Pose, ms: u32, ease: compositor_protocol::Ease) {
        self.surface.set_rest(pose);
        let live = self.live();
        self.group.animate_parts(&live, pose, ms, ease);
    }

    /// The look was replaced: restyle the glass and redraw the content in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.surface.restyle(ui, &mut self.group);
        self.recolour(ui);
    }

    /// Draws the label (and any icon) in `role` while the button is enabled, or in the kind's own ink for `None`.
    pub fn set_ink(&mut self, ui: &mut Ui, role: Option<PaletteRole>) {
        if role != self.ink {
            self.ink = role;
            self.recolour(ui);
        }
    }

    fn ink_for(&self, ui: &Ui, state: &ControlState) -> Rgba {
        match self.ink {
            Some(role) if state.enabled => ui.look.colour(role),
            _ => content_ink(ui, self.kind, state),
        }
    }

    fn recolour(&mut self, ui: &mut Ui) {
        let ink = self.ink_for(ui, &self.model.state);
        if let Some(i) = self.label {
            if let Some(l) = self.group.label_mut(i) {
                l.set_colour(ui, ink);
            }
        }
        self.redraw_icons(ui);
    }

    fn redraw_icons(&mut self, ui: &Ui) {
        let ink = if self.model.state.enabled {
            self.ink_for(ui, &self.model.state)
        } else {
            ui.look.disabled_ink()
        };
        if let (Some(slot), Some(icon)) = (self.icon, self.model.icon) {
            if let Some(layer) = self.group.parts[slot].image_mut() {
                super::icon::paint(layer, icon, ink);
            }
        }
        if let Some(slot) = self.check {
            if let Some(layer) = self.group.parts[slot].image_mut() {
                super::icon::paint(layer, Icon::Check, ink);
            }
        }
    }

    fn name(&self) -> String {
        match &self.model.name {
            Some(name) if self.model.label.is_empty() || self.kind == ButtonKind::Day => {
                name.clone()
            }
            _ => self.model.label.clone(),
        }
    }
}

/// The colour of a button's label and glyphs: the ink, the accent's ink on the accent, and the secondary ink when disabled (the
/// reason a control is unavailable stays readable; only an icon takes the .55 coefficient).
fn content_ink(ui: &Ui, kind: ButtonKind, state: &ControlState) -> Rgba {
    match (kind, state.enabled) {
        (ButtonKind::Primary, true) => ui.look.colour(PaletteRole::AccentInk),
        (ButtonKind::Primary, false) => ui.look.colour(PaletteRole::AccentInk),
        // A chip that is in is on the accent in the Settings' page look (`Look::material`).
        (ButtonKind::Chip | ButtonKind::Day, true) if ui.look.page && state.selected => {
            ui.look.colour(PaletteRole::AccentInk)
        }
        (_, true) => ui.look.ink(),
        (_, false) => ui.look.ink2(),
    }
}

fn ellipsise_for(ui: &mut Ui, text: &str, ty: crate::text::Type, width: f32) -> String {
    super::text::ellipsise(&mut ui.fonts, text, ty, width)
}

impl Movable for Button {
    fn set_in(&mut self, about: (f32, f32), pose: Pose) {
        self.group
            .set_in(about, pose, self.surface.idle_ring().as_slice());
    }

    fn animate_in(
        &mut self,
        about: (f32, f32),
        pose: Pose,
        duration_ms: u32,
        ease: compositor_protocol::Ease,
    ) {
        self.group.animate_in(
            about,
            pose,
            self.surface.idle_ring().as_slice(),
            duration_ms,
            ease,
        );
    }
}

impl Component for Button {
    type Model = ButtonModel;
    type Action = ButtonAction;

    fn update(&mut self, model: &ButtonModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let mut damage = Damage::Properties;
        if model.state != self.model.state {
            self.surface.set_state(ui, &mut self.group, model.state);
        }
        let ink = self.ink_for(ui, &model.state);
        if let Some(i) = self.label {
            let ty = label_type(ui, self.kind);
            let text = ellipsise_for(ui, &model.label, ty, self.text_w);
            if let Some(l) = self.group.label_mut(i) {
                l.set_colour(ui, ink);
                if l.text() != text {
                    l.set(ui, &text);
                    damage = Damage::Content;
                }
            }
        }
        if let Some(slot) = self.check {
            self.group.parts[slot].rest_opacity = if model.state.selected { 1.0 } else { 0.0 };
            self.group.set_parts(&[slot], self.surface.rest());
        }
        let icon_changed = model.icon != self.model.icon;
        let ink_changed = model.state.enabled != self.model.state.enabled;
        self.model = model.clone();
        if icon_changed || ink_changed {
            self.redraw_icons(ui);
            damage = Damage::Content;
        }
        if !model.state.enabled {
            self.press.cancel();
        }
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<ButtonAction> {
        let Event::Touch(touch) = event else {
            return None;
        };
        if !self.model.state.enabled {
            return None;
        }
        let was = self.press.is_pressed();
        let step = self.press.feed(&self.hit, touch);
        let now = step.pressed_after(was);
        if now != was {
            self.surface.set_down(ui, &mut self.group, now);
        }
        (step == Step::Activated).then_some(ButtonAction::Activated)
    }

    fn frame(&self) -> Frame {
        self.hit
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        out.push(
            SemanticNode::new(SemanticKind::Button, self.name(), self.hit)
                .actionable()
                .enabled(self.model.state.enabled)
                .selected(self.model.state.selected),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{TouchEvent, TouchPhase};
    use crate::testing::{Harness, harness};
    use compositor_protocol::{Content, Request};

    fn touch(phase: TouchPhase, x: f32, y: f32) -> Event {
        Event::Touch(TouchEvent {
            id: 1,
            phase,
            x,
            y,
            at_ns: 0,
        })
    }

    const AT: Frame = Frame::new(100.0, 100.0, 240.0, 64.0);

    fn button(h: &mut Harness, kind: ButtonKind, model: &ButtonModel) -> Button {
        let b = Button::new(&mut h.ui, AT, 10, kind, MaterialContext::Nested, model).unwrap();
        h.commit();
        b
    }

    fn animates(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Animate { .. }))
            .count()
    }

    #[test]
    fn a_release_inside_activates_and_the_press_shows_on_down_not_on_release() {
        let mut h = harness();
        let mut b = button(
            &mut h,
            ButtonKind::Standard,
            &ButtonModel::labelled("Connect"),
        );
        assert_eq!(
            b.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui),
            None
        );
        assert!(b.is_pressed());
        let down = h.commit();
        assert!(
            animates(&down) >= 2,
            "rest and pressed glass crossfade: {down:?}"
        );
        assert_eq!(
            b.input(&touch(TouchPhase::Up, 150.0, 130.0), &mut h.ui),
            Some(ButtonAction::Activated)
        );
        assert!(!b.is_pressed());
        assert!(animates(&h.commit()) >= 2, "and fade back");
    }

    #[test]
    fn a_drag_out_a_cancel_or_a_second_finger_never_activates() {
        let mut h = harness();
        let mut b = button(
            &mut h,
            ButtonKind::Standard,
            &ButtonModel::labelled("Connect"),
        );
        b.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui);
        b.input(&touch(TouchPhase::Move, 150.0, 400.0), &mut h.ui);
        assert!(!b.is_pressed(), "out of the button it is shown at rest");
        assert_eq!(
            b.input(&touch(TouchPhase::Up, 150.0, 400.0), &mut h.ui),
            None
        );

        b.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui);
        assert_eq!(
            b.input(&touch(TouchPhase::Cancel, 0.0, 0.0), &mut h.ui),
            None
        );
        assert!(!b.is_pressed());

        // A parent that takes the contact (a list that began to scroll) cancels it.
        b.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui);
        b.cancel(&h.ui);
        assert_eq!(
            b.input(&touch(TouchPhase::Up, 150.0, 130.0), &mut h.ui),
            None
        );
    }

    #[test]
    fn a_disabled_button_ignores_every_touch_and_keeps_its_substrate_and_its_reason_readable() {
        let mut h = harness();
        let model = ButtonModel::labelled("Connect").enabled(false);
        let mut b = button(&mut h, ButtonKind::Standard, &model);
        let before = h.all().len();
        assert_eq!(
            b.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui),
            None
        );
        assert_eq!(
            b.input(&touch(TouchPhase::Up, 150.0, 130.0), &mut h.ui),
            None
        );
        assert_eq!(h.all().len(), before, "nothing was asked of the compositor");
        let mut sink = SemanticsSink::new();
        b.semantics(&mut sink);
        assert!(!sink.find("Connect").unwrap().enabled);
        // The text is the secondary ink, not the primary one.
        let ink2 = h.ui.look.ink2();
        assert_eq!(
            b.group.label_mut(b.label.unwrap()).unwrap().style().colour,
            ink2
        );
    }

    #[test]
    fn a_chip_drawn_small_still_takes_a_full_size_touch_around_its_centre() {
        let mut h = harness();
        let visual = Frame::new(100.0, 100.0, 120.0, 40.0);
        let mut b = Button::chip(
            &mut h.ui,
            visual,
            10,
            MaterialContext::Nested,
            &ButtonModel::labelled("Back"),
        )
        .unwrap();
        assert!(b.hit().h >= 64.0 && b.hit().w >= 120.0);
        // 10 units under the chip's drawn bottom edge (140) is still the chip.
        assert_eq!(
            b.input(&touch(TouchPhase::Down, 150.0, 150.0), &mut h.ui),
            None
        );
        assert!(b.is_pressed());
        assert_eq!(
            b.input(&touch(TouchPhase::Up, 150.0, 150.0), &mut h.ui),
            Some(ButtonAction::Activated)
        );
    }

    #[test]
    fn every_button_kind_is_glass_with_its_depth_pair_and_the_primary_is_on_the_accent() {
        let mut h = harness();
        let mut kept = Vec::new();
        for (kind, model) in [
            (ButtonKind::Standard, ButtonModel::labelled("Open")),
            (ButtonKind::Primary, ButtonModel::labelled("Save")),
            (ButtonKind::Chip, ButtonModel::labelled("Back")),
            (ButtonKind::Icon, ButtonModel::icon(Icon::Close, "Close")),
        ] {
            let b = button(&mut h, kind, &model);
            let (rest, pressed) = b.surface().shown();
            assert!(
                rest.depth.is_some_and(|d| !d.inset) && rest.sheen.is_some(),
                "{kind:?}"
            );
            assert!(
                pressed.is_some_and(|p| p.depth.is_some_and(|d| d.inset) && p.sheen.is_none()),
                "{kind:?}"
            );
            if kind == ButtonKind::Primary {
                assert_eq!(rest.tint, h.ui.look.theme.materials.accent);
            }
            kept.push(b);
        }
        let glass = h
            .layers()
            .iter()
            .filter(|(_, c, _)| matches!(c, Content::Glass(_)))
            .count();
        assert_eq!(glass, 8, "a rest and a pressed layer for each of the four");
    }

    #[test]
    fn selecting_a_chip_changes_properties_and_draws_a_check_without_a_new_layer() {
        let mut h = harness();
        let mut b = Button::chip(
            &mut h.ui,
            AT,
            10,
            MaterialContext::Nested,
            &ButtonModel::labelled("Rain"),
        )
        .unwrap();
        h.commit();
        let layers_before = h.layers().len();
        let on = ButtonModel::labelled("Rain").selected(true);
        assert_eq!(b.update(&on, &mut h.ui), Damage::Properties);
        assert_eq!(b.update(&on, &mut h.ui), Damage::None);
        h.commit();
        assert_eq!(h.layers().len(), layers_before);
        assert_eq!(
            b.surface().pair().rest.tint,
            h.ui.look.theme.materials.glass_raised_selected
        );
        let check = b.check.unwrap();
        assert_eq!(b.group.parts[check].rest_opacity, 1.0);
    }

    #[test]
    fn an_unselected_chips_check_is_not_visible_from_the_first_frame() {
        let mut h = harness();
        let chip = Button::chip(
            &mut h.ui,
            AT,
            10,
            MaterialContext::Nested,
            &ButtonModel::labelled("Rain"),
        )
        .unwrap();
        h.commit();
        let invisible_images = h
            .layers()
            .iter()
            .filter(|(_, c, p)| matches!(c, Content::Image) && p.opacity == 0.0)
            .count();
        assert_eq!(invisible_images, 1, "the check waits at opacity 0");
        drop(chip);
    }

    #[test]
    fn large_text_makes_a_button_wider_and_not_its_label_shorter() {
        let mut h = harness();
        let model = ButtonModel::labelled("Forget network");
        let normal = Button::required_width(&mut h.ui, ButtonKind::Standard, &model);
        h.ui.look.text_scale = 2.0;
        let large = Button::required_width(&mut h.ui, ButtonKind::Standard, &model);
        assert!(large > normal * 1.8, "{normal} -> {large}");
        // Built at the width it asks for, nothing is cut.
        let frame = Frame::new(20.0, 20.0, large, 64.0);
        let mut b = Button::new(
            &mut h.ui,
            frame,
            10,
            ButtonKind::Standard,
            MaterialContext::Nested,
            &model,
        )
        .unwrap();
        assert_eq!(
            b.group.label_mut(b.label.unwrap()).unwrap().text(),
            "Forget network"
        );
        h.commit();
    }

    #[test]
    fn a_new_label_is_drawn_again_and_a_long_one_is_cut_to_fit_the_button() {
        let mut h = harness();
        let mut b = button(
            &mut h,
            ButtonKind::Standard,
            &ButtonModel::labelled("Connect"),
        );
        assert_eq!(
            b.update(&ButtonModel::labelled("Disconnect"), &mut h.ui),
            Damage::Content
        );
        let long =
            ButtonModel::labelled("A very long button label that cannot possibly fit in the box");
        b.update(&long, &mut h.ui);
        let shown = b
            .group
            .label_mut(b.label.unwrap())
            .unwrap()
            .text()
            .to_owned();
        assert!(shown.ends_with('…'), "{shown}");
        assert!(h.ui.fonts.measure(&shown, ty::BUTTON).width <= b.text_w);
        h.commit();
    }

    #[test]
    fn a_host_that_follows_the_finger_itself_can_show_the_press_and_the_release() {
        let mut h = harness();
        let mut b = button(
            &mut h,
            ButtonKind::Standard,
            &ButtonModel::labelled("Connect"),
        );
        b.show_pressed(&h.ui, true);
        assert!(
            animates(&h.commit()) >= 2,
            "rest and pressed glass crossfade"
        );
        assert!(!b.is_pressed(), "the button's own tracker saw no contact");
        b.show_pressed(&h.ui, true);
        assert_eq!(
            animates(&h.commit()),
            0,
            "showing the same press again draws nothing"
        );
        b.show_pressed(&h.ui, false);
        assert!(animates(&h.commit()) >= 2, "and fades back");
    }

    #[test]
    fn a_button_a_page_has_moved_is_pressed_and_let_go_where_it_stands_and_not_where_it_was_made() {
        let mut h = harness();
        h.ui.look.text_scale = crate::lucent::tokens::PAGE_SCALE;
        h.ui.look.page = true;
        let mut button = Button::new(
            &mut h.ui,
            AT,
            10,
            ButtonKind::Standard,
            MaterialContext::Nested,
            &ButtonModel::labelled("Connect"),
        )
        .unwrap();
        h.commit();
        // A page moved it 228 down, as a cell of a list stands below where it was made.
        button.set_pose(Pose::at(0.0, 228.0));
        h.commit();
        let on_screen_y = AT.y + 228.0;
        let landings = |requests: &[Request]| -> Vec<f32> {
            requests
                .iter()
                .filter_map(|r| match r {
                    Request::Animate { to, .. } => Some(to.y),
                    _ => None,
                })
                .collect()
        };
        button.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui);
        let down = landings(&h.commit());
        assert!(!down.is_empty(), "the press is drawn");
        assert!(
            down.iter().all(|y| *y == on_screen_y),
            "pressed where it stands, at {on_screen_y}: {down:?}"
        );
        button.input(&touch(TouchPhase::Up, 150.0, 130.0), &mut h.ui);
        let up = landings(&h.commit());
        assert!(
            up.iter().all(|y| *y == on_screen_y),
            "and let go where it stands: {up:?}"
        );
        // Disabled while a finger is on it (the owner is working): it lets go where it stands too.
        button.input(&touch(TouchPhase::Down, 150.0, 130.0), &mut h.ui);
        h.commit();
        button.update(&ButtonModel::labelled("Connect").enabled(false), &mut h.ui);
        let disabled = landings(&h.commit());
        assert!(disabled.iter().all(|y| *y == on_screen_y), "{disabled:?}");
    }
}

#[cfg(test)]
mod page_look_tests {
    use super::*;
    use crate::testing::harness;
    use crate::text::Weight;

    fn page(h: &mut crate::testing::Harness) {
        h.ui.look.text_scale = crate::lucent::tokens::PAGE_SCALE;
        h.ui.look.page = true;
    }

    #[test]
    fn a_settings_button_is_the_mocks_btn_22_px_in_the_heavier_weight_with_24_at_each_side() {
        let mut h = harness();
        page(&mut h);
        let ty = label_type(&h.ui, ButtonKind::Standard);
        assert_eq!((ty.px, ty.weight), (22, Weight::SemiBold));
        assert_eq!(label_type(&h.ui, ButtonKind::Chip), ty, "a chip too");
        let words =
            |h: &mut crate::testing::Harness, text: &str| h.ui.fonts.measure(text, ty).width.ceil();
        let install = words(&mut h, "Install");
        let model = ButtonModel::labelled("Install");
        for kind in [ButtonKind::Standard, ButtonKind::Primary] {
            assert_eq!(
                Button::required_width(&mut h.ui, kind, &model),
                48.0 + install,
                "{kind:?}: 24 at each side"
            );
        }
        // A glyph before the words (Check now): 24 at each side and 10 after the glyph.
        let check = words(&mut h, "Check now");
        let with = ButtonModel::labelled("Check now").with_icon(Icon::Check);
        assert_eq!(
            Button::required_width(&mut h.ui, ButtonKind::Standard, &with),
            48.0 + ICON + 10.0 + check
        );
        // The Back button, a chip with a glyph, is the mock's `.btn.back`: 14 before the glyph, 6 after it, 22 at the end.
        let plugins = words(&mut h, "Plugins");
        let back = ButtonModel::labelled("Plugins").with_icon(Icon::Back);
        assert_eq!(
            Button::required_width(&mut h.ui, ButtonKind::Chip, &back),
            14.0 + ICON + 6.0 + plugins + 22.0
        );
        // A chip without a glyph keeps the toolkit's room (16 at each side), and none for a check: the page look has no check (a day
        // is in or out by its wash), so that its words stand in the middle of the glass and not 32 from it.
        let chip = ButtonModel::labelled("Mon");
        let mon = words(&mut h, "Mon");
        assert_eq!(
            Button::required_width(&mut h.ui, ButtonKind::Chip, &chip),
            32.0 + mon
        );
    }

    #[test]
    fn a_chip_of_the_page_look_has_its_words_in_the_middle_of_its_glass_and_no_slot_for_a_check() {
        let mut h = harness();
        page(&mut h);
        let at = Frame::new(100.0, 100.0, 100.0, 48.0);
        let chip = Button::chip(
            &mut h.ui,
            at,
            10,
            MaterialContext::Nested,
            &ButtonModel::labelled("Mo"),
        )
        .unwrap();
        // The glass at rest, the pressed glass, the ring that shows with the focus, and the words: nothing else.
        assert_eq!(chip.group().parts.len(), 4, "no layer for a check");
        let words = chip.group().parts.last().unwrap().frame;
        assert_eq!(
            words.x - at.x,
            at.right() - words.right(),
            "the words have as much room before them as after"
        );
        // Outside the settings a chip keeps its check: no meaning is the colour alone.
        let mut h = harness();
        let plain = Button::chip(
            &mut h.ui,
            at,
            10,
            MaterialContext::Nested,
            &ButtonModel::labelled("Mo"),
        )
        .unwrap();
        assert_eq!(
            plain.group().parts.len(),
            5,
            "the check is a layer of its own"
        );
    }

    #[test]
    fn a_chip_that_is_in_is_the_accent_with_the_accents_ink_and_one_that_is_out_is_glass_in_the_page_look()
     {
        let mut h = harness();
        page(&mut h);
        let at = Frame::new(100.0, 100.0, 100.0, 48.0);
        let make = |h: &mut crate::testing::Harness, selected: bool| {
            Button::chip(
                &mut h.ui,
                at,
                10,
                MaterialContext::Nested,
                &ButtonModel::labelled("Mo").selected(selected),
            )
            .unwrap()
        };
        let mut out = make(&mut h, false);
        let mut chosen = make(&mut h, true);
        let accent_of = |h: &crate::testing::Harness| {
            h.ui.look
                .material(
                    SmartMaterial::PrimaryButton,
                    &ControlState {
                        selected: true,
                        ..ControlState::default()
                    },
                    MaterialContext::Nested,
                )
                .rest
                .tint
        };
        assert_eq!(chosen.surface().shown().0.tint, accent_of(&h));
        assert_ne!(out.surface().shown().0.tint, accent_of(&h));
        assert_eq!(
            chosen.surface().shown().0.corner_radius,
            24.0,
            "a pill still, whatever it is made of"
        );
        assert_eq!(
            chosen.label_colour(),
            Some(h.ui.look.colour(PaletteRole::AccentInk))
        );
        assert_eq!(out.label_colour(), Some(h.ui.look.ink()));
        // The tap on a day turns it from one to the other where it stands.
        out.update(&ButtonModel::labelled("Mo").selected(true), &mut h.ui);
        assert_eq!(out.surface().shown().0.tint, accent_of(&h));
        assert_eq!(
            out.label_colour(),
            Some(h.ui.look.colour(PaletteRole::AccentInk))
        );
        // Outside the settings it is the toolkit's selected chip: glass with its wash, and the check that says it.
        let mut plain = harness();
        let b = Button::chip(
            &mut plain.ui,
            at,
            10,
            MaterialContext::Nested,
            &ButtonModel::labelled("Mo").selected(true),
        )
        .unwrap();
        assert_ne!(b.surface().shown().0.tint, accent_of(&h));
    }

    #[test]
    fn outside_the_settings_a_button_keeps_the_toolkits_type_and_room() {
        let mut h = harness();
        let button = h.ui.look.ty(ty::BUTTON);
        assert_eq!(label_type(&h.ui, ButtonKind::Primary), button);
        assert_eq!(button.weight, Weight::Medium);
        assert_eq!(label_type(&h.ui, ButtonKind::Chip), h.ui.look.ty(ty::LABEL));
        let words = h.ui.fonts.measure("Install", button).width.ceil();
        assert_eq!(
            Button::required_width(
                &mut h.ui,
                ButtonKind::Primary,
                &ButtonModel::labelled("Install")
            ),
            32.0 + words
        );
    }

    #[test]
    fn the_way_back_is_the_mocks_raised_btn_with_corners_of_14_a_day_keeps_its_pill_and_outside_the_settings_all_are_pills()
     {
        let corner = |h: &mut crate::testing::Harness, kind, model: ButtonModel| {
            let frame = Frame::new(20.0, 20.0, 145.0, 56.0);
            let b =
                Button::new(&mut h.ui, frame, 1, kind, MaterialContext::Nested, &model).unwrap();
            b.surface().shown().0.corner_radius
        };
        let mut h = harness();
        page(&mut h);
        let back = ButtonModel::labelled("Plugins").with_icon(Icon::Back);
        assert_eq!(corner(&mut h, ButtonKind::Chip, back.clone()), 14.0);
        assert_eq!(
            corner(
                &mut h,
                ButtonKind::Standard,
                ButtonModel::labelled("Install")
            ),
            14.0
        );
        // A chip with no glyph (a day of a repeat) is a pill.
        assert_eq!(
            corner(&mut h, ButtonKind::Chip, ButtonModel::labelled("Mo")),
            28.0
        );
        // Outside the Settings the way back is the toolkit's pill.
        h.ui.look.page = false;
        assert_eq!(corner(&mut h, ButtonKind::Chip, back), 28.0);
    }

    #[test]
    fn a_day_is_a_48_circle_with_its_letter_called_by_its_whole_label() {
        let mut h = harness();
        page(&mut h);
        let model = ButtonModel::labelled("T").selected(true);
        let named = ButtonModel {
            name: Some("Th".to_owned()),
            ..model.clone()
        };
        assert_eq!(
            Button::required_width(&mut h.ui, ButtonKind::Day, &named),
            Button::DAY
        );
        assert_eq!(Button::DAY, 48.0);
        let frame = Frame::new(20.0, 20.0, Button::DAY, Button::DAY);
        let day = Button::new(
            &mut h.ui,
            frame,
            1,
            ButtonKind::Day,
            MaterialContext::Nested,
            &named,
        )
        .unwrap();
        assert_eq!(
            day.surface().shown().0.corner_radius,
            24.0,
            "a circle: half of 48"
        );
        assert_eq!(day.name(), "Th", "called by its whole label");
        // The same letter and no name is called by its letter, and a pill chip with a name keeps calling itself by its words.
        let plain = Button::new(
            &mut h.ui,
            frame,
            1,
            ButtonKind::Day,
            MaterialContext::Nested,
            &model,
        )
        .unwrap();
        assert_eq!(plain.name(), "T");
        let chip = Button::new(
            &mut h.ui,
            Frame::new(20.0, 20.0, 145.0, 56.0),
            1,
            ButtonKind::Chip,
            MaterialContext::Nested,
            &named,
        )
        .unwrap();
        assert_eq!(chip.name(), "T");
    }
}
