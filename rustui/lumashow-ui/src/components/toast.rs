//! The toast (design section 3.3): a short message at the bottom of the screen, with one optional action. One is shown at a time,
//! it never takes focus or input beyond its own button, and it goes by itself.
//!
//! An action that undoes something stays at least five seconds ([`UNDO_MIN_MS`]), whatever the model asks, and the clock stops while
//! the person is reading it or something in it is focused ([`Toast::pause`]); it carries on with what was left. The panel is the
//! `Overlay` material like a dialog's, its button a toolkit button. Its words wrap onto up to two lines and are never shrunk: large
//! text makes the toast taller and wider.

use super::button::{Button, ButtonAction, ButtonKind, ButtonModel};
use super::group::{Group, Part, Pose};
use super::mark::Mark;
use super::press::{PressTracker, Step};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::text::{Run, line_height, wrap_to};
use super::text_layer::{Item, TextLayer};
use super::{Component, Damage, target_around};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::tokens::{motion, smart, space, ty};
use crate::text::{Type, Weight};
use compositor_protocol::{Glass, Rgba};
use std::io;

/// An action that undoes something is on screen at least this long.
pub const UNDO_MIN_MS: u32 = 5_000;
/// How long a message with no action stays unless the model says otherwise.
pub const DEFAULT_MS: u32 = 4_000;
const LINES: usize = 2;
const PAD_X: f32 = space::S5;
const PAD_Y: f32 = space::S4;
const GAP: f32 = space::S3;
const RISE: f32 = space::S4;

/// Where a toast stands and how it looks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ToastLook {
    /// A pill centred above the foot of the screen, its action a raised chip: the toolkit's own.
    #[default]
    Centred,
    /// The toast of the Settings pages as the approved mock draws it: its left edge at `x`, 24 above the foot of the screen and
    /// 60 high, corners of 18, the words at 22, and the action a word in the accent on a wash (not a button of glass).
    Page { x: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToastModel {
    pub text: String,
    /// The label of the one action (Undo), if there is one.
    pub action: Option<String>,
    /// How long it stays, in milliseconds. With an action it is at least [`UNDO_MIN_MS`].
    pub ttl_ms: u32,
    pub look: ToastLook,
}

impl ToastModel {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            action: None,
            ttl_ms: DEFAULT_MS,
            look: ToastLook::Centred,
        }
    }

    /// The toast of a Settings page, its left edge at `x`: see [`ToastLook::Page`].
    pub fn on_page(mut self, x: f32) -> Self {
        self.look = ToastLook::Page { x };
        self
    }

    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action = Some(label.into());
        self
    }

    pub fn lasting(mut self, ttl_ms: u32) -> Self {
        self.ttl_ms = ttl_ms;
        self
    }

    /// How long the toast stays: what was asked, and at least five seconds when it has an action.
    pub fn effective_ms(&self) -> u32 {
        if self.action.is_some() {
            self.ttl_ms.max(UNDO_MIN_MS)
        } else {
            self.ttl_ms
        }
    }
}

/// What a toast says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastEvent {
    /// The action was pressed (and the toast goes).
    Action,
    /// The time ran out (and the toast goes).
    Expired,
}

struct Placed {
    panel: Frame,
    lines: Vec<String>,
    text: Frame,
    action: Option<Frame>,
}

/// The toast of a page: 24 above the foot, 60 high; 22 before the words, 18 between them and the action and 10 after it; the action's
/// word 14 in from the edges of its wash, which is 50 high with corners of 12; the panel's corners are 18; the wash is 7 % of the ink.
const PAGE_BOTTOM: f32 = 24.0;
const PAGE_H: f32 = 60.0;
const PAGE_RADIUS: f32 = 18.0;
const PAGE_PAD_L: f32 = 22.0;
const PAGE_PAD_R: f32 = 10.0;
/// Above and below the words of a toast of two lines (one line is the 60 of the panel).
const PAGE_PAD_Y: f32 = 14.0;
const PAGE_GAP: f32 = 18.0;
const PAGE_ACTION_PAD: f32 = 14.0;
const PAGE_ACTION_H: f32 = 50.0;
const PAGE_ACTION_RADIUS: u32 = 12;
const PAGE_WASH: u8 = 18;

/// The types of a page toast: the words at the label size and weight, the action's word in the heavier one.
fn page_types(ui: &Ui) -> (Type, Type) {
    let words = ui.look.ty(ty::LABEL);
    (
        words,
        Type {
            weight: Weight::SemiBold,
            ..words
        },
    )
}

fn place_page(ui: &mut Ui, screen: Frame, model: &ToastModel, x: f32) -> Placed {
    let (words_ty, action_ty) = page_types(ui);
    let action_w = model
        .action
        .as_ref()
        .map(|label| ui.fonts.measure(label, action_ty).width.ceil() + 2.0 * PAGE_ACTION_PAD);
    let pad_r = if action_w.is_some() {
        PAGE_PAD_R
    } else {
        PAGE_PAD_L
    };
    let reserved = action_w.map_or(0.0, |w| PAGE_GAP + w);
    let room = (screen.right() - space::S6 - x - PAGE_PAD_L - pad_r - reserved).max(smart::TARGET);
    let want = ui.fonts.measure(&model.text, words_ty).width.ceil();
    let text_w = want.min(room);
    let lines = wrap_to(&mut ui.fonts, &model.text, words_ty, text_w, LINES);
    let text_h = lines.len() as f32 * line_height(words_ty);
    let width = PAGE_PAD_L + text_w + reserved + pad_r;
    let height = (text_h + 2.0 * PAGE_PAD_Y).max(PAGE_H);
    let panel = Frame::new(x, screen.bottom() - PAGE_BOTTOM - height, width, height);
    let text = Frame::new(
        panel.x + PAGE_PAD_L,
        panel.y + (height - text_h) / 2.0,
        text_w,
        text_h,
    );
    let action = action_w.map(|w| {
        Frame::new(
            panel.right() - pad_r - w,
            panel.y + (height - PAGE_ACTION_H) / 2.0,
            w,
            PAGE_ACTION_H,
        )
    });
    Placed {
        panel,
        lines,
        text,
        action,
    }
}

/// `colour` at `alpha`, premultiplied: a layer's colour for a tint of an ink.
fn tint(colour: Rgba, alpha: u8) -> Rgba {
    let scale = |c: u8| (u32::from(c) * u32::from(alpha) / 255) as u8;
    Rgba(scale(colour.0), scale(colour.1), scale(colour.2), alpha)
}

fn place(ui: &mut Ui, screen: Frame, model: &ToastModel) -> Placed {
    if let ToastLook::Page { x } = model.look {
        return place_page(ui, screen, model, x);
    }
    let width_max = (screen.w - 2.0 * space::S6).min(640.0);
    let bt = ui.look.ty(ty::BODY);
    let action_w = model.action.as_ref().map(|label| {
        Button::required_width(ui, ButtonKind::Chip, &ButtonModel::labelled(label))
            .max(smart::TARGET)
    });
    let reserved = action_w.map_or(0.0, |w| w + GAP);
    let want = ui.fonts.measure(&model.text, bt).width.ceil();
    let text_w = want
        .min(width_max - 2.0 * PAD_X - reserved)
        .max(smart::TARGET);
    let lines = wrap_to(&mut ui.fonts, &model.text, bt, text_w, LINES);
    let text_h = lines.len() as f32 * line_height(bt);
    let width = text_w + reserved + 2.0 * PAD_X;
    let height = (text_h + 2.0 * PAD_Y).max(smart::TARGET);
    let panel = Frame::new(
        screen.x + (screen.w - width) / 2.0,
        screen.bottom() - space::S6 - height,
        width,
        height,
    );
    let text = Frame::new(
        panel.x + PAD_X,
        panel.y + (height - text_h) / 2.0,
        text_w,
        text_h,
    );
    let action = action_w.map(|w| {
        Frame::new(
            panel.right() - PAD_X - w,
            panel.y + (height - 48.0) / 2.0,
            w,
            48.0,
        )
    });
    Placed {
        panel,
        lines,
        text,
        action,
    }
}

/// What the toast's text layer shows, and where that layer stands: for a pill the words alone; for a page toast the whole panel, so
/// that the action's wash and its word are drawn with the words.
fn items<'a>(ui: &Ui, placed: &'a Placed, model: &'a ToastModel) -> (Frame, Vec<Item<'a>>) {
    let ink = ui.look.ink();
    match model.look {
        ToastLook::Centred => {
            let bt = ui.look.ty(ty::BODY);
            let runs = placed
                .lines
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    Item::Text(Run::left(
                        line,
                        bt,
                        ink,
                        0.0,
                        i as f32 * line_height(bt),
                        placed.text.w,
                    ))
                })
                .collect();
            (placed.text, runs)
        }
        ToastLook::Page { .. } => {
            let (words_ty, action_ty) = page_types(ui);
            let (px, py) = (placed.panel.x, placed.panel.y);
            let mut items: Vec<Item> = placed
                .lines
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    Item::Text(Run::left(
                        line,
                        words_ty,
                        ink,
                        placed.text.x - px,
                        placed.text.y - py + i as f32 * line_height(words_ty),
                        placed.text.w,
                    ))
                })
                .collect();
            if let (Some(label), Some(at)) = (model.action.as_deref(), placed.action) {
                items.push(
                    Mark::round(
                        (at.x - px).round() as i32,
                        (at.y - py).round() as i32,
                        at.w.round() as u32,
                        at.h.round() as u32,
                        PAGE_ACTION_RADIUS,
                        tint(ink, PAGE_WASH),
                    )
                    .into(),
                );
                let top = at.y - py + ((at.h - line_height(action_ty)) / 2.0).round();
                items.push(Item::Text(Run::left(
                    label,
                    action_ty,
                    ui.look.accent(),
                    at.x - px + PAGE_ACTION_PAD,
                    top,
                    at.w - 2.0 * PAGE_ACTION_PAD,
                )));
            }
            (placed.panel, items)
        }
    }
}

pub struct Toast {
    model: ToastModel,
    placed: Placed,
    panel: Group,
    /// The panel's focus ring: a panel is never focused, so a pose never shows it.
    ring: usize,
    text: usize,
    action: Option<Button>,
    /// Where a touch counts on a page toast's action (its wash is not a button of glass), and the press on it.
    page_hit: Option<Frame>,
    press: PressTracker,
    shown: bool,
    /// When the toast goes: the clock of an unpaused toast. `None` while hidden or paused.
    due_ns: Option<u64>,
    /// What is left of its time while it is paused.
    left_ms: u64,
    paused: bool,
    /// When an exit that has begun is over.
    exit_until: Option<u64>,
}

impl Toast {
    /// The toast for `model` on `screen`, hidden: [`Toast::show`] brings it in and starts its clock.
    pub fn new(ui: &mut Ui, screen: Frame, z: i32, model: &ToastModel) -> io::Result<Self> {
        let placed = place(ui, screen, model);
        let mut panel = Group::new(placed.panel.centre());
        let ring = Surface::add(
            ui,
            &mut panel,
            placed.panel,
            z,
            SmartMaterial::Overlay,
            MaterialContext::Independent,
            ControlState::default(),
        )
        .ring_part();
        let text = {
            let (at, drawn) = items(ui, &placed, model);
            let layer = TextLayer::new(ui, at, z + 1, &drawn)?;
            panel.push(Part::new(layer, at))
        };
        let page = matches!(model.look, ToastLook::Page { .. });
        let action = match (&model.action, placed.action) {
            (Some(label), Some(at)) if !page => Some(Button::chip(
                ui,
                at,
                z + 1,
                MaterialContext::Nested,
                &ButtonModel::labelled(label),
            )?),
            _ => None,
        };
        let page_hit = placed
            .action
            .filter(|_| page && model.action.is_some())
            .map(|at| target_around(at, smart::TARGET));
        let mut toast = Self {
            model: model.clone(),
            placed,
            panel,
            ring,
            text,
            action,
            page_hit,
            press: PressTracker::new(),
            shown: false,
            due_ns: None,
            left_ms: 0,
            paused: false,
            exit_until: None,
        };
        if page {
            let glass = toast.panel_glass(ui);
            if let Some(g) = toast.panel.parts[0].glass() {
                g.restyle(glass);
            }
        }
        toast.pose(Pose::hidden(), None);
        Ok(toast)
    }

    /// The panel's glass for the toast's look: the Overlay material, with the corners of a page toast's own.
    fn panel_glass(&self, ui: &Ui) -> Glass {
        let pair = ui
            .look
            .material(
                SmartMaterial::Overlay,
                &ControlState::default(),
                MaterialContext::Independent,
            )
            .sized(self.placed.panel.w, self.placed.panel.h);
        match self.model.look {
            ToastLook::Centred => pair.rest,
            ToastLook::Page { .. } => Glass {
                corner_radius: PAGE_RADIUS,
                ..pair.rest
            },
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (_, drawn) = items(ui, &self.placed, &self.model);
        if let Some(layer) = self.panel.parts[self.text].lines_mut() {
            layer.draw(ui, &drawn);
        }
    }

    fn pose(&mut self, pose: Pose, ms: Option<u32>) {
        match ms {
            Some(ms) => {
                self.panel
                    .animate_in(self.panel.about, pose, &[self.ring], ms, motion::SETTLE);
                if let Some(b) = &mut self.action {
                    b.animate_pose(pose, ms, motion::SETTLE);
                }
            }
            None => {
                self.panel.set_in(self.panel.about, pose, &[self.ring]);
                if let Some(b) = &mut self.action {
                    b.set_pose(pose);
                }
            }
        }
    }

    fn away(ui: &Ui) -> Pose {
        if ui.look.moves() {
            Pose::slid(0.0, RISE, 0.0)
        } else {
            Pose::fade(0.0)
        }
    }

    pub fn model(&self) -> &ToastModel {
        &self.model
    }

    pub fn panel(&self) -> Frame {
        self.placed.panel
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// The words as drawn, one string per line.
    pub fn lines(&self) -> &[String] {
        &self.placed.lines
    }

    pub fn button(&self) -> Option<&Button> {
        self.action.as_ref()
    }

    /// When the toast goes if nothing pauses it, on the frame clock; `None` while hidden or paused.
    pub fn due_ns(&self) -> Option<u64> {
        self.due_ns
    }

    /// Brings the toast in at `now_ns` and starts its clock.
    pub fn show(&mut self, ui: &Ui, now_ns: u64) {
        self.exit_until = None;
        self.paused = false;
        self.shown = true;
        self.left_ms = u64::from(self.model.effective_ms());
        self.due_ns = Some(now_ns + self.left_ms * 1_000_000);
        let away = Self::away(ui);
        self.pose(away, None);
        self.pose(Pose::REST, Some(ui.look.duration_ms(motion::CARD_MS)));
    }

    /// Stops the clock while the person is reading the toast or something in it is focused, and starts it again with what was left.
    pub fn pause(&mut self, now_ns: u64, paused: bool) {
        if !self.shown || paused == self.paused {
            return;
        }
        self.paused = paused;
        if paused {
            if let Some(due) = self.due_ns.take() {
                self.left_ms = due.saturating_sub(now_ns) / 1_000_000;
            }
        } else {
            self.due_ns = Some(now_ns + self.left_ms * 1_000_000);
        }
    }

    fn leave(&mut self, ui: &Ui, now_ns: u64) {
        self.shown = false;
        self.due_ns = None;
        let ms = ui.look.duration_ms(motion::EXIT_MS);
        let away = Self::away(ui);
        self.pose(away, Some(ms));
        self.exit_until = Some(now_ns + u64::from(ms) * 1_000_000);
    }

    /// Ends it now with no event (something replaced it).
    pub fn dismiss(&mut self, ui: &Ui, now_ns: u64) {
        if self.shown {
            self.leave(ui, now_ns);
        }
    }

    /// Lets time pass: the toast goes when its time is up (and says so), and one that has finished leaving is put away for good.
    pub fn tick(&mut self, ui: &Ui, now_ns: u64) -> Option<ToastEvent> {
        if self.exit_until.is_some_and(|t| now_ns >= t) {
            self.exit_until = None;
            self.pose(Pose::hidden(), None);
        }
        if self.due_ns.is_some_and(|due| now_ns >= due) {
            self.leave(ui, now_ns);
            return Some(ToastEvent::Expired);
        }
        None
    }

    /// The look was replaced: restyle the glass and button and draw the words again.
    pub fn restyle(&mut self, ui: &mut Ui) {
        let glass = self.panel_glass(ui);
        if let Some(g) = self.panel.parts[0].glass() {
            g.restyle(glass);
        }
        if let Some(b) = &mut self.action {
            b.restyle(ui);
        }
        self.draw(ui);
    }
}

impl Component for Toast {
    type Model = ToastModel;
    type Action = ToastEvent;

    /// A model with the same words and action label changes only its time; anything else is another toast, which replaces this
    /// one ([`Toast::new`]).
    fn update(&mut self, model: &ToastModel, _ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        if model.text != self.model.text || model.action != self.model.action {
            return Damage::None;
        }
        self.model = model.clone();
        Damage::Properties
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<ToastEvent> {
        if !self.shown {
            return None;
        }
        let now = match event {
            Event::Touch(t) => t.at_ns,
            Event::Gesture(_) => return None,
        };
        let activated = match (self.page_hit, event) {
            (Some(hit), Event::Touch(t)) => self.press.feed(&hit, t) == Step::Activated,
            _ => self.action.as_mut()?.input(event, ui)? == ButtonAction::Activated,
        };
        if activated {
            self.leave(ui, now);
            return Some(ToastEvent::Action);
        }
        None
    }

    /// Only what can take a touch: the action's target, and nothing while hidden. The rest of the screen keeps its input.
    fn frame(&self) -> Frame {
        match (&self.action, self.page_hit, self.shown) {
            (Some(b), _, true) => b.hit(),
            (None, Some(hit), true) => hit,
            _ => Frame::default(),
        }
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        if !self.shown {
            return;
        }
        out.push(SemanticNode::new(
            SemanticKind::Toast,
            self.model.text.clone(),
            self.placed.panel,
        ));
        if let Some(b) = &self.action {
            b.semantics(out);
        }
        if let (Some(hit), Some(label)) = (self.page_hit, &self.model.action) {
            out.push(SemanticNode::new(SemanticKind::Button, label.clone(), hit).actionable());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Look;
    use crate::input::TouchPhase::{Cancel, Down, Move, Up};
    use crate::lucent::palette::Mode;
    use crate::testing::{
        Harness, count_requests, harness, made_as_picture, pictures_made, pixels_damaged_in, touch,
        warm_glyph_harness,
    };
    use compositor_protocol::{Content, LayerId, Request};

    const SCREEN: Frame = Frame::new(0.0, 0.0, 960.0, 480.0);
    const S: u64 = 1_000_000_000;

    fn toast(h: &mut Harness, model: &ToastModel) -> Toast {
        let t = Toast::new(&mut h.ui, SCREEN, 90, model).unwrap();
        h.commit();
        t
    }

    fn text_id(t: &Toast) -> LayerId {
        t.panel.parts[t.text].layer().id()
    }

    fn is_run(t: &mut Toast) -> bool {
        t.panel.parts[t.text]
            .lines_mut()
            .is_some_and(|l| l.is_glyph_run())
    }

    /// A message of one line and one that wraps onto two beside an action.
    fn looks() -> [ToastModel; 2] {
        [
            ToastModel::new("Saved"),
            ToastModel::new("The alarm for tomorrow was moved to seven thirty in the morning")
                .with_action("Undo"),
        ]
    }

    fn warm(model: &ToastModel) -> Harness {
        warm_glyph_harness(|h| toast(h, model))
    }

    #[test]
    fn a_toasts_words_are_the_same_pixels_as_glyph_quads_and_as_a_picture_in_every_look() {
        for model in looks() {
            let mut run = warm(&model);
            let mut glyphs = toast(&mut run, &model);
            assert!(is_run(&mut glyphs));
            let mut pic = harness();
            let picture = toast(&mut pic, &model);
            let (a, b) = (
                run.layer_pixels(text_id(&glyphs)),
                pic.layer_pixels(text_id(&picture)),
            );
            assert!(a.iter().any(|v| *v != 0), "{:?}", model.text);
            assert_eq!(a, b, "{:?}", model.text);
            // The light look draws the same words in another ink, again as the picture does.
            for h in [&mut run, &mut pic] {
                h.ui.look = Look::standard(Mode::Light);
            }
            glyphs.restyle(&mut run.ui);
            let mut picture = picture;
            picture.restyle(&mut pic.ui);
            run.commit();
            pic.commit();
            let (c, d) = (
                run.layer_pixels(text_id(&glyphs)),
                pic.layer_pixels(text_id(&picture)),
            );
            assert_ne!(a, c, "the ink did not change");
            assert_eq!(c, d, "{:?} in the light look", model.text);
        }
    }

    #[test]
    fn on_glyph_text_a_toasts_words_make_no_picture_and_write_no_pixel_and_no_layer_more() {
        let creates =
            |sent: &[Request]| count_requests(sent, |r| matches!(r, Request::Create { .. }));
        for model in looks() {
            let mut run = warm(&model);
            let mut glyphs = Toast::new(&mut run.ui, SCREEN, 90, &model).unwrap();
            let sent = run.commit();
            assert!(is_run(&mut glyphs), "{:?}", model.text);
            let text = text_id(&glyphs);
            assert!(!made_as_picture(&sent, text), "{:?}", model.text);
            assert_eq!(pixels_damaged_in(&sent, text), 0.0, "{:?}", model.text);
            // The only picture of the toast is the action chip's check mark, an icon of its own.
            assert_eq!(
                pictures_made(&sent),
                usize::from(model.action.is_some()),
                "{:?}: {sent:?}",
                model.text
            );
            assert_eq!(run.ui.glyph_text_fallbacks, 0);
            let mut pic = harness();
            let _picture = Toast::new(&mut pic.ui, SCREEN, 90, &model).unwrap();
            let as_pictures = pic.commit();
            assert!(pictures_made(&as_pictures) >= 1, "the words are a picture");
            assert!(
                creates(&sent) <= creates(&as_pictures),
                "{} layers on glyph text, {} on pictures",
                creates(&sent),
                creates(&as_pictures)
            );
        }
    }

    #[test]
    fn on_glyph_text_a_new_look_recolours_the_words_without_a_picture_or_a_pixel() {
        let model = ToastModel::new("Alarm moved to 7:30");
        let mut run = warm(&model);
        let mut t = toast(&mut run, &model);
        run.ui.look = Look::standard(Mode::Light);
        t.restyle(&mut run.ui);
        let sent = run.commit();
        assert_eq!(pictures_made(&sent), 0, "{sent:?}");
        assert_eq!(pixels_damaged_in(&sent, text_id(&t)), 0.0);
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::Restyle {
                    content: Content::Glyphs(_),
                    ..
                }
            )),
            1,
            "the words take the new ink with one restyle: {sent:?}"
        );
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::Create { .. })),
            0
        );
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
    }

    /// A toast is never focused: showing it must not light the panel's focus ring.
    #[test]
    fn showing_a_toast_never_lights_its_panels_focus_ring() {
        let mut h = harness();
        let mut t = toast(
            &mut h,
            &ToastModel::new("Alarm deleted").with_action("Undo"),
        );
        t.show(&h.ui, 10 * S);
        h.commit();
        let ring = t.panel.parts[t.ring]
            .shape()
            .expect("the ring is a shape")
            .id();
        let shown = h
            .layers()
            .into_iter()
            .find(|(id, _, _)| *id == ring)
            .expect("the ring layer")
            .2;
        assert!(
            !shown.visible,
            "the focus ring of an unfocused toast is shown"
        );
        let panel = t.panel.parts[0].glass().expect("the panel's glass").id();
        assert!(
            h.layers()
                .into_iter()
                .find(|(id, _, _)| *id == panel)
                .unwrap()
                .2
                .visible
        );
    }

    #[test]
    fn a_toast_with_an_action_stays_at_least_five_seconds_whatever_the_model_asks() {
        assert_eq!(
            ToastModel::new("Saved").lasting(1_000).effective_ms(),
            1_000
        );
        assert_eq!(
            ToastModel::new("Deleted")
                .with_action("Undo")
                .lasting(1_000)
                .effective_ms(),
            UNDO_MIN_MS
        );
        assert_eq!(
            ToastModel::new("Deleted")
                .with_action("Undo")
                .lasting(9_000)
                .effective_ms(),
            9_000
        );
        let mut h = harness();
        let mut t = toast(
            &mut h,
            &ToastModel::new("Reminder deleted")
                .with_action("Undo")
                .lasting(500),
        );
        t.show(&h.ui, 10 * S);
        assert_eq!(
            t.tick(&h.ui, 10 * S + 4_900_000_000),
            None,
            "still there at 4.9 s"
        );
        assert_eq!(t.tick(&h.ui, 10 * S + 5 * S), Some(ToastEvent::Expired));
        assert!(!t.is_shown());
    }

    #[test]
    fn the_clock_stops_while_reading_and_goes_on_with_what_was_left() {
        let mut h = harness();
        let mut t = toast(
            &mut h,
            &ToastModel::new("Alarm moved to 7:30").lasting(4_000),
        );
        t.show(&h.ui, 0);
        t.pause(3 * S, true);
        assert!(t.is_paused() && t.due_ns().is_none());
        assert_eq!(
            t.tick(&h.ui, 60 * S),
            None,
            "a minute later it is still there"
        );
        t.pause(60 * S, false);
        assert_eq!(t.due_ns(), Some(61 * S), "one second was left");
        assert_eq!(t.tick(&h.ui, 60 * S + 900_000_000), None);
        assert_eq!(t.tick(&h.ui, 61 * S), Some(ToastEvent::Expired));
        // Pausing a toast that is not shown does nothing.
        t.pause(100 * S, true);
        assert!(!t.is_paused());
    }

    #[test]
    fn the_action_reports_once_and_a_drag_off_it_a_cancel_and_a_hidden_toast_report_nothing() {
        let mut h = harness();
        let mut t = toast(
            &mut h,
            &ToastModel::new("Reminder deleted").with_action("Undo"),
        );
        let (x, y) = t.button().unwrap().hit().centre();
        assert_eq!(
            t.input(&touch(1, Down, x, y, 0), &mut h.ui),
            None,
            "hidden: nothing"
        );
        t.show(&h.ui, 0);
        t.input(&touch(1, Down, x, y, 100), &mut h.ui);
        t.input(&touch(1, Move, x, y - 300.0, 116), &mut h.ui);
        assert_eq!(t.input(&touch(1, Up, x, y - 300.0, 130), &mut h.ui), None);
        t.input(&touch(1, Down, x, y, 200), &mut h.ui);
        assert_eq!(t.input(&touch(1, Cancel, 0.0, 0.0, 210), &mut h.ui), None);
        t.input(&touch(1, Down, x, y, 300), &mut h.ui);
        assert_eq!(
            t.input(&touch(1, Up, x, y, 330), &mut h.ui),
            Some(ToastEvent::Action)
        );
        assert!(!t.is_shown(), "the toast goes with its action");
        assert_eq!(
            t.tick(&h.ui, 100 * S),
            None,
            "an acted-on toast does not also expire"
        );
    }

    #[test]
    fn a_page_toast_stands_where_the_mock_puts_it_and_its_action_is_a_word_on_a_wash_that_takes_a_touch()
     {
        let mut h = harness();
        // The settings view draws at a fifth more than the person's size: words at 22.
        h.ui.look.text_scale = 1.2;
        let model = ToastModel::new("Bug saved · 1.2 MB · copied to the NAS")
            .with_action("See")
            .on_page(404.0);
        let mut t = toast(&mut h, &model);
        let panel = t.panel();
        // The mock: its left edge at 404, 24 above the foot of the 480 screen, 60 high.
        assert_eq!((panel.x, panel.h, panel.bottom()), (404.0, 60.0, 456.0));
        assert!(panel.w > 400.0 && panel.w < 560.0, "{}", panel.w);
        assert!(
            t.button().is_none(),
            "the action is a word on a wash, not a button of glass"
        );
        t.show(&h.ui, 0);
        let (x, y) = t.frame().centre();
        t.input(&touch(1, Down, x, y, 100), &mut h.ui);
        t.input(&touch(1, Move, x, y - 300.0, 116), &mut h.ui);
        assert_eq!(
            t.input(&touch(1, Up, x, y - 300.0, 130), &mut h.ui),
            None,
            "a drag off it is a cancel"
        );
        t.input(&touch(1, Down, x, y, 300), &mut h.ui);
        assert_eq!(
            t.input(&touch(1, Up, x, y, 330), &mut h.ui),
            Some(ToastEvent::Action)
        );
        // The toolkit's own look is where it was: centred on the screen.
        let pill = toast(&mut h, &ToastModel::new("Saved"));
        let middle = pill.panel().x + pill.panel().w / 2.0;
        assert!((middle - 480.0).abs() < 1.0, "{middle}");
    }

    #[test]
    fn a_toast_never_takes_input_beyond_its_own_button() {
        let mut h = harness();
        let mut with = toast(&mut h, &ToastModel::new("Saved").with_action("Undo"));
        let mut without = toast(&mut h, &ToastModel::new("Saved"));
        assert_eq!(with.frame(), Frame::default(), "hidden takes nothing");
        with.show(&h.ui, 0);
        without.show(&h.ui, 0);
        assert!(
            with.frame().w <= 140.0 && with.frame().h <= 70.0,
            "{:?}",
            with.frame()
        );
        assert_eq!(
            without.frame(),
            Frame::default(),
            "a toast with no action takes no touch at all"
        );
        assert_eq!(
            without.input(&touch(1, Down, 480.0, 440.0, 0), &mut h.ui),
            None
        );
    }

    #[test]
    fn large_text_makes_the_toast_taller_and_wider_and_it_keeps_every_word_up_to_two_lines() {
        let mut h = harness();
        let model =
            ToastModel::new("The alarm for tomorrow was moved to seven thirty").with_action("Undo");
        let normal = toast(&mut h, &model);
        h.ui.look.text_scale = 2.0;
        let large = toast(&mut h, &model);
        assert!(
            large.panel().h > normal.panel().h,
            "{:?} -> {:?}",
            normal.panel(),
            large.panel()
        );
        assert!(large.lines().len() <= LINES);
        assert!(large.panel().x >= SCREEN.x && large.panel().right() <= SCREEN.right());
        assert!(large.panel().bottom() <= SCREEN.bottom());
        let joined = large.lines().join(" ");
        assert!(joined.starts_with("The alarm"), "{joined}");
    }

    #[test]
    fn it_is_the_overlay_material_and_arrives_and_leaves_as_poses_with_no_layer_made() {
        let mut h = harness();
        let mut t = toast(&mut h, &ToastModel::new("Saved"));
        let layers = h.layers().len();
        t.show(&h.ui, 0);
        let sent = h.commit();
        assert!(sent.iter().any(|r| matches!(r, Request::Animate { .. })));
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::Create { .. } | Request::Damage { .. }))
        );
        let glass = h
            .layers()
            .into_iter()
            .find_map(|(_, c, _)| match c {
                compositor_protocol::Content::Glass(g) => Some(g),
                _ => None,
            })
            .unwrap();
        assert_eq!(glass.tint, h.ui.look.theme.materials.overlay);
        assert!(glass.depth.is_some_and(|d| !d.inset) && glass.sheen.is_some());
        t.dismiss(&h.ui, S);
        assert!(!t.is_shown());
        t.tick(&h.ui, 2 * S);
        h.commit();
        assert_eq!(h.layers().len(), layers);
        let mut sink = SemanticsSink::new();
        t.semantics(&mut sink);
        assert!(sink.nodes().is_empty());
    }
}
