//! Saying why there is nothing to show, and a small status in words: `EmptyState` and `StatusBadge`.
//!
//! An empty state is calm: a large icon, a title and a few lines of explanation, centred, and a primary button when there is a real
//! next step. There are five reasons for having nothing to show, and each looks and reads differently (its own icon, its own title
//! colour, and a name a screen reader says), because "nothing here yet", "not set up", "no connection", "ran out" and "went wrong"
//! call for different things from the person. The reason is never told by colour alone.
//!
//! A status badge is a small monochrome icon and a few words in one state colour (good, warning, problem, information); the words are
//! always there.

use super::button::{Button, ButtonAction, ButtonKind, ButtonModel};
use super::group::{Group, Part};
use super::icon::{Icon, icon_part, paint};
use super::label::{Align, Label, style};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::text::{Run, ellipsise, line_height, wrap_to};
use super::text_layer::{Item, TextLayer};
use super::{Component, Damage};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;
use crate::lucent::material::MaterialContext;
use crate::lucent::palette::PaletteRole;
use crate::lucent::tokens::{smart, space, ty};
use std::convert::Infallible;
use std::io;

/// Why there is nothing to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmptyKind {
    /// There is simply nothing yet (no alarms, no messages).
    Empty,
    /// Something has to be set up first.
    Unconfigured,
    /// There is no connection to what would be shown.
    Offline,
    /// What was shown ran out (a sign-in, a download, a timer).
    Expired,
    /// Something went wrong.
    Failed,
}

impl EmptyKind {
    pub const ALL: [EmptyKind; 5] = [
        EmptyKind::Empty,
        EmptyKind::Unconfigured,
        EmptyKind::Offline,
        EmptyKind::Expired,
        EmptyKind::Failed,
    ];

    /// The kind's own icon.
    pub fn icon(self) -> Icon {
        match self {
            EmptyKind::Empty => Icon::Tray,
            EmptyKind::Unconfigured => Icon::Sliders,
            EmptyKind::Offline => Icon::CloudOff,
            EmptyKind::Expired => Icon::Hourglass,
            EmptyKind::Failed => Icon::Warning,
        }
    }

    /// The palette role of the kind's icon and title.
    pub fn role(self) -> PaletteRole {
        match self {
            EmptyKind::Empty => PaletteRole::Ink2,
            EmptyKind::Unconfigured => PaletteRole::Info,
            EmptyKind::Offline => PaletteRole::Warning,
            EmptyKind::Expired => PaletteRole::Ink,
            EmptyKind::Failed => PaletteRole::Danger,
        }
    }

    /// What a screen reader says the state is.
    pub fn name(self) -> &'static str {
        match self {
            EmptyKind::Empty => "Empty",
            EmptyKind::Unconfigured => "Not set up",
            EmptyKind::Offline => "Offline",
            EmptyKind::Expired => "Expired",
            EmptyKind::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmptyModel {
    pub kind: EmptyKind,
    pub title: String,
    pub body: String,
    /// The label of the button that does the real next thing ("Set up", "Try again"); none when there is nothing for the person to do.
    pub action: Option<String>,
}

impl EmptyModel {
    pub fn new(kind: EmptyKind, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            body: body.into(),
            action: None,
        }
    }

    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action = Some(label.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyAction {
    /// The next-step button was pressed.
    Act,
}

const ICON: f32 = 48.0;
const SIDE: f32 = space::S4;
const BUTTON_H: f32 = 56.0;
const BODY_LINES: usize = 6;
const TITLE_LINES: usize = 2;

#[derive(Clone, Debug, PartialEq)]
struct Lines {
    title: Vec<String>,
    body: Vec<String>,
}

fn compose(ui: &mut Ui, model: &EmptyModel, width: f32) -> Lines {
    let inner = (width - 2.0 * SIDE).max(0.0);
    let (tt, bt) = (ui.look.ty(ty::TITLE), ui.look.ty(ty::BODY));
    Lines {
        title: wrap_to(&mut ui.fonts, &model.title, tt, inner, TITLE_LINES),
        body: wrap_to(&mut ui.fonts, &model.body, bt, inner, BODY_LINES),
    }
}

fn text_height(ui: &Ui, lines: &Lines) -> f32 {
    lines.title.len() as f32 * line_height(ui.look.ty(ty::TITLE))
        + space::S2
        + lines.body.len() as f32 * line_height(ui.look.ty(ty::BODY))
}

fn button_width(ui: &mut Ui, label: &str, max: f32) -> f32 {
    Button::required_width(ui, ButtonKind::Primary, &ButtonModel::labelled(label))
        .max(2.0 * smart::TARGET)
        .min(max)
}

/// The title's lines in the kind's colour and the explanation's in the second ink, all centred in `w`.
fn text_runs<'a>(ui: &Ui, lines: &'a Lines, kind: EmptyKind, w: f32) -> Vec<Run<'a>> {
    let (tt, bt) = (ui.look.ty(ty::TITLE), ui.look.ty(ty::BODY));
    let (title_ink, body_ink) = (ui.look.colour(kind.role()), ui.look.ink2());
    let mut y = 0.0;
    let mut runs = Vec::new();
    for line in &lines.title {
        runs.push(Run::centre(line, tt, title_ink, 0.0, y, w));
        y += line_height(tt);
    }
    y += space::S2;
    for line in &lines.body {
        runs.push(Run::centre(line, bt, body_ink, 0.0, y, w));
        y += line_height(bt);
    }
    runs
}

pub struct EmptyState {
    model: EmptyModel,
    frame: Frame,
    z: i32,
    context: MaterialContext,
    /// The icon (part 0) and the one text layer (part 1).
    group: Group,
    lines: Lines,
    button: Option<Button>,
}

impl EmptyState {
    /// The height an empty state needs in `width` at the person's text size: the icon, the title, the explanation and the button.
    pub fn required_height(ui: &mut Ui, model: &EmptyModel, width: f32) -> f32 {
        let lines = compose(ui, model, width);
        ICON + space::S4
            + text_height(ui, &lines)
            + if model.action.is_some() {
                space::S6 + BUTTON_H
            } else {
                0.0
            }
    }

    /// An empty state centred in `frame`. `context` is that of its button: whether it sits on a sheet that has the glass backdrop.
    pub fn new(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        context: MaterialContext,
        model: &EmptyModel,
    ) -> io::Result<Self> {
        let lines = compose(ui, model, frame.w);
        let text_h = text_height(ui, &lines);
        let total = ICON
            + space::S4
            + text_h
            + if model.action.is_some() {
                space::S6 + BUTTON_H
            } else {
                0.0
            };
        let top = frame.y + ((frame.h - total) / 2.0).max(0.0);
        let colour = ui.look.colour(model.kind.role());
        let mut group = Group::new(frame.centre());
        let icon_frame = Frame::new(frame.x + (frame.w - ICON) / 2.0, top, ICON, ICON);
        group.push(icon_part(ui, model.kind.icon(), icon_frame, z, colour)?);
        let text_frame = Frame::new(frame.x, top + ICON + space::S4, frame.w, text_h);
        let runs = text_runs(ui, &lines, model.kind, frame.w);
        let layer = TextLayer::new(
            ui,
            text_frame,
            z,
            &runs.iter().copied().map(Item::Text).collect::<Vec<_>>(),
        )?;
        group.push(Part::new(layer, text_frame));
        let button = match &model.action {
            Some(label) => {
                let w = button_width(ui, label, frame.w - 2.0 * SIDE);
                let at = Frame::new(
                    frame.x + (frame.w - w) / 2.0,
                    text_frame.bottom() + space::S6,
                    w,
                    BUTTON_H,
                );
                Some(Button::new(
                    ui,
                    at,
                    z + 1,
                    ButtonKind::Primary,
                    context,
                    &ButtonModel::labelled(label),
                )?)
            }
            None => None,
        };
        Ok(Self {
            model: model.clone(),
            frame,
            z,
            context,
            group,
            lines,
            button,
        })
    }

    fn draw_text(&mut self, ui: &mut Ui) {
        let runs = text_runs(ui, &self.lines, self.model.kind, self.frame.w);
        if let Some(layer) = self.group.parts[1].lines_mut() {
            layer.draw_runs(ui, &runs);
        }
    }

    fn draw_icon(&mut self, ui: &Ui) {
        let colour = ui.look.colour(self.model.kind.role());
        if let Some(layer) = self.group.parts[0].image_mut() {
            paint(layer, self.model.kind.icon(), colour);
        }
    }

    pub fn model(&self) -> &EmptyModel {
        &self.model
    }

    /// The button, when there is a next step.
    pub fn button(&self) -> Option<&Button> {
        self.button.as_ref()
    }

    /// The lines of the title and of the explanation, as drawn.
    pub fn lines(&self) -> (&[String], &[String]) {
        (&self.lines.title, &self.lines.body)
    }

    /// The look was replaced: restyle the button and draw the icon and the text again in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        if let Some(b) = &mut self.button {
            b.restyle(ui);
        }
        self.draw_icon(ui);
        self.draw_text(ui);
    }
}

impl Component for EmptyState {
    type Model = EmptyModel;
    type Action = EmptyAction;

    fn update(&mut self, model: &EmptyModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let lines = compose(ui, model, self.frame.w);
        let same_room = lines.title.len() == self.lines.title.len()
            && lines.body.len() == self.lines.body.len()
            && model.action == self.model.action;
        if !same_room {
            // Another amount of text or another button changes where everything goes: build it again in the same frame. If that
            // cannot be done the old one stays and the next update tries again.
            return match Self::new(ui, self.frame, self.z, self.context, model) {
                Ok(new) => {
                    *self = new;
                    Damage::Content
                }
                Err(_) => Damage::None,
            };
        }
        let kind_changed = model.kind != self.model.kind;
        let text_changed = lines != self.lines;
        self.model = model.clone();
        if kind_changed {
            self.draw_icon(ui);
        }
        if kind_changed || text_changed {
            self.lines = lines;
            self.draw_text(ui);
        }
        Damage::Content
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<EmptyAction> {
        let button = self.button.as_mut()?;
        (button.input(event, ui)? == ButtonAction::Activated).then_some(EmptyAction::Act)
    }

    fn frame(&self) -> Frame {
        self.button
            .as_ref()
            .map_or_else(Frame::default, Button::hit)
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        out.push(
            SemanticNode::new(SemanticKind::Status, self.model.title.clone(), self.frame)
                .value(format!("{}: {}", self.model.kind.name(), self.model.body)),
        );
        if let Some(b) = &self.button {
            b.semantics(out);
        }
    }
}

// ---- StatusBadge ------------------------------------------------------------------------------------------------------

/// What a status badge says about its subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusKind {
    Positive,
    Warning,
    Danger,
    Info,
}

impl StatusKind {
    pub const ALL: [StatusKind; 4] = [
        StatusKind::Positive,
        StatusKind::Warning,
        StatusKind::Danger,
        StatusKind::Info,
    ];

    pub fn icon(self) -> Icon {
        match self {
            StatusKind::Positive => Icon::Check,
            StatusKind::Warning => Icon::Warning,
            StatusKind::Danger => Icon::Close,
            StatusKind::Info => Icon::Info,
        }
    }

    pub fn role(self) -> PaletteRole {
        match self {
            StatusKind::Positive => PaletteRole::Positive,
            StatusKind::Warning => PaletteRole::Warning,
            StatusKind::Danger => PaletteRole::Danger,
            StatusKind::Info => PaletteRole::Info,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            StatusKind::Positive => "Good",
            StatusKind::Warning => "Warning",
            StatusKind::Danger => "Problem",
            StatusKind::Info => "Information",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatusBadgeModel {
    pub kind: StatusKind,
    /// A few words. Always shown: the icon and the colour only back them up.
    pub text: String,
}

impl StatusBadgeModel {
    pub fn new(kind: StatusKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

pub struct StatusBadge {
    model: StatusBadgeModel,
    frame: Frame,
    text_w: f32,
    group: Group,
}

impl StatusBadge {
    /// The side of the badge's icon at the person's text size: a little taller than the text, so large text gets a larger icon.
    fn icon_side(ui: &Ui) -> f32 {
        ui.look.ty(ty::LABEL).px as f32 + 2.0
    }

    /// The width a badge needs to show `model` whole, and the height of its line, at the person's text size.
    pub fn required_size(ui: &mut Ui, model: &StatusBadgeModel) -> (f32, f32) {
        let t = ui.look.ty(ty::LABEL);
        let text = ui.fonts.measure(&model.text, t).width.ceil();
        (Self::icon_side(ui) + space::S2 + text, line_height(t))
    }

    /// A badge in `frame`: the icon at the left, the words after it, both centred in the height.
    pub fn new(ui: &mut Ui, frame: Frame, z: i32, model: &StatusBadgeModel) -> io::Result<Self> {
        let colour = ui.look.colour(model.kind.role());
        let side = Self::icon_side(ui);
        let mut group = Group::new(frame.centre());
        let icon_frame = Frame::new(frame.x, frame.y + (frame.h - side) / 2.0, side, side);
        group.push(icon_part(ui, model.kind.icon(), icon_frame, z, colour)?);
        let text_x = frame.x + side + space::S2;
        let text_w = (frame.right() - text_x).max(0.0);
        let t = ui.look.ty(ty::LABEL);
        let text = ellipsise(&mut ui.fonts, &model.text, t, text_w);
        let text_frame = Frame::new(text_x, frame.y, text_w, frame.h);
        let label = Label::new(ui, text_frame, z, style(t, colour, Align::Left), &text)?;
        group.push(Part::new(label, text_frame));
        Ok(Self {
            model: model.clone(),
            frame,
            text_w,
            group,
        })
    }

    pub fn model(&self) -> &StatusBadgeModel {
        &self.model
    }

    /// The words as drawn (after any ellipsis).
    pub fn shown_text(&mut self) -> &str {
        self.group.label_mut(1).map_or("", |l| l.text())
    }

    /// The look was replaced: draw the icon and the words again in the new colour.
    pub fn restyle(&mut self, ui: &mut Ui) {
        let colour = ui.look.colour(self.model.kind.role());
        if let Some(layer) = self.group.parts[0].image_mut() {
            paint(layer, self.model.kind.icon(), colour);
        }
        if let Some(l) = self.group.label_mut(1) {
            l.set_colour(ui, colour);
        }
    }
}

impl Component for StatusBadge {
    type Model = StatusBadgeModel;
    /// A badge takes no touch and says nothing.
    type Action = Infallible;

    fn update(&mut self, model: &StatusBadgeModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let colour = ui.look.colour(model.kind.role());
        if model.kind != self.model.kind {
            if let Some(layer) = self.group.parts[0].image_mut() {
                paint(layer, model.kind.icon(), colour);
            }
        }
        let t = ui.look.ty(ty::LABEL);
        let text = ellipsise(&mut ui.fonts, &model.text, t, self.text_w);
        if let Some(l) = self.group.label_mut(1) {
            l.set_colour(ui, colour);
            l.set(ui, &text);
        }
        self.model = model.clone();
        Damage::Content
    }

    fn input(&mut self, _event: &Event, _ui: &mut Ui) -> Option<Infallible> {
        None
    }

    fn frame(&self) -> Frame {
        self.frame
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        out.push(
            SemanticNode::new(SemanticKind::Status, self.model.text.clone(), self.frame)
                .value(self.model.kind.name()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchPhase::{Cancel, Down, Move, Up};
    use crate::testing::{
        Harness, count_requests, harness, made_as_picture, pictures_made, pixels_damaged,
        pixels_damaged_in, touch, warm_glyph_harness,
    };
    use compositor_protocol::{Content, LayerId, Request};

    const AT: Frame = Frame::new(40.0, 60.0, 560.0, 340.0);

    fn state(h: &mut Harness, model: &EmptyModel) -> EmptyState {
        let s = EmptyState::new(&mut h.ui, AT, 10, MaterialContext::Independent, model).unwrap();
        h.commit();
        s
    }

    fn text_id(s: &EmptyState) -> LayerId {
        s.group.parts[1].layer().id()
    }

    fn is_run(s: &mut EmptyState) -> bool {
        s.group.parts[1]
            .lines_mut()
            .is_some_and(|l| l.is_glyph_run())
    }

    /// One of each reason: five title colours, one of which is the body's own ink.
    fn looks() -> Vec<EmptyModel> {
        EmptyKind::ALL
            .iter()
            .map(|kind| {
                EmptyModel::new(*kind, "Nothing here yet", "Reminders you add show up here.")
                    .with_action("Add a reminder")
            })
            .collect()
    }

    fn warm(models: &[EmptyModel]) -> Harness {
        warm_glyph_harness(|h| models.iter().map(|m| state(h, m)).collect::<Vec<_>>())
    }

    #[test]
    fn an_empty_states_words_are_the_same_pixels_as_glyph_quads_and_as_a_picture_for_every_reason()
    {
        let models = looks();
        let mut run = warm(&models);
        for model in &models {
            let mut glyphs = state(&mut run, model);
            assert!(is_run(&mut glyphs), "{:?}", model.kind);
            let mut pic = harness();
            let picture = state(&mut pic, model);
            let (a, b) = (
                run.layer_pixels(text_id(&glyphs)),
                pic.layer_pixels(text_id(&picture)),
            );
            assert!(a.iter().any(|v| *v != 0), "{:?}", model.kind);
            assert_eq!(a, b, "{:?}", model.kind);
        }
    }

    #[test]
    fn on_glyph_text_an_empty_states_words_make_no_picture_and_write_no_pixel_and_no_layer_more() {
        let creates =
            |sent: &[Request]| count_requests(sent, |r| matches!(r, Request::Create { .. }));
        let models = looks();
        let mut run = warm(&models);
        for model in &models {
            let mut glyphs =
                EmptyState::new(&mut run.ui, AT, 10, MaterialContext::Independent, model).unwrap();
            let sent = run.commit();
            assert!(is_run(&mut glyphs), "{:?}", model.kind);
            let text = text_id(&glyphs);
            assert!(!made_as_picture(&sent, text), "{:?}", model.kind);
            assert_eq!(pixels_damaged_in(&sent, text), 0.0, "{:?}", model.kind);
            // The only picture is the reason's icon, its own layer.
            assert_eq!(pictures_made(&sent), 1, "{:?}: {sent:?}", model.kind);
            assert_eq!(run.ui.glyph_text_fallbacks, 0);
            let mut pic = harness();
            let _picture =
                EmptyState::new(&mut pic.ui, AT, 10, MaterialContext::Independent, model).unwrap();
            let as_pictures = pic.commit();
            assert!(creates(&sent) <= creates(&as_pictures));
            assert!(pictures_made(&as_pictures) > pictures_made(&sent));
        }
    }

    #[test]
    fn on_glyph_text_new_words_of_the_same_size_are_one_swap_and_a_new_reason_is_one_restyle() {
        let saved = EmptyModel::new(EmptyKind::Failed, "Could not save", "Try again.");
        let loaded = EmptyModel::new(EmptyKind::Failed, "Could not load", "Try later.");
        let offline = EmptyModel::new(EmptyKind::Offline, "Could not load", "Try later.");
        let mut run = warm(&[saved.clone(), loaded.clone(), offline.clone()]);
        let mut s = state(&mut run, &saved);
        assert_eq!(s.update(&loaded, &mut run.ui), Damage::Content);
        let sent = run.commit();
        assert_eq!(
            count_requests(
                &sent,
                |r| matches!(r, Request::SetGlyphs { id, .. } if *id == text_id(&s))
            ),
            1,
            "{sent:?}"
        );
        assert_eq!(pictures_made(&sent), 0);
        assert_eq!(pixels_damaged(&sent), 0.0);
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::Create { .. })),
            0
        );
        // The same words for another reason: the title takes the new colour (one restyle of the words), the icon is redrawn.
        assert_eq!(s.update(&offline, &mut run.ui), Damage::Content);
        let sent = run.commit();
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::Restyle { id, content: Content::Glyphs(_) } if *id == text_id(&s)
            )),
            1,
            "{sent:?}"
        );
        assert_eq!(pixels_damaged_in(&sent, text_id(&s)), 0.0);
        assert_eq!(pictures_made(&sent), 0);
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
    }

    #[test]
    fn the_five_reasons_differ_in_icon_colour_and_name_and_never_by_colour_alone() {
        let icons: Vec<_> = EmptyKind::ALL.iter().map(|k| k.icon()).collect();
        let names: Vec<_> = EmptyKind::ALL.iter().map(|k| k.name()).collect();
        for (i, a) in icons.iter().enumerate() {
            for b in &icons[i + 1..] {
                assert_ne!(a, b, "two reasons share an icon");
            }
        }
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "{names:?}");
        let mut h = harness();
        let s = state(
            &mut h,
            &EmptyModel::new(
                EmptyKind::Offline,
                "No connection",
                "Join a network to see the forecast.",
            ),
        );
        let mut sink = SemanticsSink::new();
        s.semantics(&mut sink);
        assert_eq!(
            sink.find("No connection").unwrap().value.as_deref(),
            Some(
                format!(
                    "{}: Join a network to see the forecast.",
                    EmptyKind::Offline.name()
                )
                .as_str()
            )
        );
    }

    #[test]
    fn the_next_step_button_activates_on_release_and_there_is_none_without_a_real_action() {
        let mut h = harness();
        let mut with = state(
            &mut h,
            &EmptyModel::new(EmptyKind::Unconfigured, "No location", "Add one.")
                .with_action("Add location"),
        );
        let (x, y) = with.button().unwrap().hit().centre();
        assert_eq!(with.input(&touch(1, Down, x, y, 0), &mut h.ui), None);
        assert_eq!(
            with.input(&touch(1, Up, x, y, 30), &mut h.ui),
            Some(EmptyAction::Act)
        );
        with.input(&touch(1, Down, x, y, 100), &mut h.ui);
        with.input(&touch(1, Move, x, y + 300.0, 116), &mut h.ui);
        assert_eq!(
            with.input(&touch(1, Up, x, y + 300.0, 130), &mut h.ui),
            None
        );
        with.input(&touch(1, Down, x, y, 200), &mut h.ui);
        assert_eq!(
            with.input(&touch(1, Cancel, 0.0, 0.0, 210), &mut h.ui),
            None
        );

        let mut without = state(
            &mut h,
            &EmptyModel::new(
                EmptyKind::Empty,
                "Nothing yet",
                "Reminders you add show up here.",
            ),
        );
        assert!(without.button().is_none());
        assert_eq!(
            without.input(&touch(1, Down, 300.0, 300.0, 0), &mut h.ui),
            None
        );
        assert_eq!(
            without.input(&touch(1, Up, 300.0, 300.0, 20), &mut h.ui),
            None
        );
    }

    #[test]
    fn new_words_of_the_same_length_are_drawn_in_place_and_another_amount_of_text_rebuilds_it() {
        let mut h = harness();
        let mut s = state(
            &mut h,
            &EmptyModel::new(EmptyKind::Failed, "Could not save", "Try again."),
        );
        let layers = h.layers().len();
        assert_eq!(
            s.update(
                &EmptyModel::new(EmptyKind::Failed, "Could not save", "Try again."),
                &mut h.ui
            ),
            Damage::None
        );
        assert_eq!(
            s.update(
                &EmptyModel::new(EmptyKind::Failed, "Could not load", "Try later."),
                &mut h.ui
            ),
            Damage::Content
        );
        assert_eq!(s.lines().0[0], "Could not load");
        h.commit();
        assert_eq!(
            h.layers().len(),
            layers,
            "no layer was made for words of the same size"
        );
        let longer = "The disk is full, so nothing more can be saved until something has been removed to make room again. ".repeat(3);
        s.update(
            &EmptyModel::new(EmptyKind::Expired, "Out of space", longer).with_action("Free space"),
            &mut h.ui,
        );
        assert!(s.button().is_some());
        assert!(s.lines().1.len() > 1);
        h.commit();
    }

    #[test]
    fn large_text_makes_the_state_taller_wraps_the_title_and_keeps_the_button_inside() {
        let mut h = harness();
        let m = EmptyModel::new(
            EmptyKind::Offline,
            "You are not connected",
            "Join a Wi-Fi network to see the forecast and the day line.",
        )
        .with_action("Choose a network");
        let normal = EmptyState::required_height(&mut h.ui, &m, 480.0);
        h.ui.look.text_scale = 2.0;
        let large = EmptyState::required_height(&mut h.ui, &m, 480.0);
        assert!(large > normal * 1.4, "{normal} -> {large}");
        let frame = Frame::new(0.0, 0.0, 480.0, large);
        let s = EmptyState::new(&mut h.ui, frame, 10, MaterialContext::Independent, &m).unwrap();
        let b = s.button().unwrap().hit();
        // The touch target is 64 high around a 56-high button: it may overhang by the 4 units each side.
        let overhang = (smart::TARGET - BUTTON_H) / 2.0 + 0.5;
        assert!(
            b.right() <= frame.right() + overhang && b.bottom() <= frame.bottom() + overhang,
            "{b:?} in {frame:?}"
        );
        assert!(s.lines().0.len() <= TITLE_LINES);
        h.commit();
    }

    #[test]
    fn a_status_badge_always_has_words_and_its_own_icon_per_state() {
        let mut h = harness();
        let icons: Vec<_> = StatusKind::ALL.iter().map(|k| k.icon()).collect();
        for (i, a) in icons.iter().enumerate() {
            for b in &icons[i + 1..] {
                assert_ne!(a, b);
            }
        }
        let model = StatusBadgeModel::new(StatusKind::Warning, "Clock not set yet");
        let (w, hgt) = StatusBadge::required_size(&mut h.ui, &model);
        let mut badge =
            StatusBadge::new(&mut h.ui, Frame::new(10.0, 10.0, w, hgt), 10, &model).unwrap();
        assert_eq!(badge.shown_text(), "Clock not set yet");
        h.commit();
        h.ui.look.text_scale = 2.0;
        let (w2, _) = StatusBadge::required_size(&mut h.ui, &model);
        assert!(w2 > w * 1.5, "{w} -> {w2}");
    }
}
