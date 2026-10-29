//! The text field: a persistent label, the value (as typed, or as bullets when it is a secret and hidden), a Show/Hide control for a
//! secret, a caret, and one line under it that says what is wrong (in the danger colour, with a mark in front, so the colour is never
//! the only sign) or what is asked for (in the quiet one).
//!
//! The field only shows what its model holds: the text, whether a secret is revealed, the message. Which message stays and when it
//! goes is the screen's rule. What the person types arrives from the [`super::Keyboard`] as [`super::KeyOutput`]s, which the screen
//! applies to the text before it updates the field in the same turn: the value layer is then rewritten once and the caret moved with
//! one `Set`, so a typed character shows by the next frame.
//!
//! A secret's characters are never drawn while it is hidden (only bullets are) and never described to accessibility: the field does
//! not keep them anywhere but in the model it was given.
//!
//! Selection is supported as a range of characters of the text, shown as a wash behind them (the caret then hides). There is no
//! composition: an input method that composes text before committing it is not part of this field.

use super::button::{Button, ButtonKind, ButtonModel};
use super::icon::Icon;
use super::label::{Align, Label, style};
use super::mark::Mark;
use super::plate::solid;
use super::press::{PressTracker, Step};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::text::{Run, ellipsise, fit_tail, line_height};
use super::text_layer::{Item, TextLayer};
use super::{Component, Damage, Group};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchPhase};
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::palette::PaletteRole;
use crate::lucent::tokens::{smart, space, ty};
use crate::scene::ShapeLayer;
use crate::text::{Fonts, Type};
use compositor_protocol::{Props, Rect};
use std::io;
use std::ops::Range;

/// What a hidden secret shows in place of each character.
const BULLET: char = '•';
/// Room between the well's edge and what it holds.
const PAD: f32 = space::S5;
/// How far under the well the note sits.
const NOTE_GAP: f32 = space::S1;
/// The caret is this wide, and this much wider than its text's pixel size is tall.
const CARET_W: f32 = 2.0;
const CARET_HEIGHT: f32 = 1.15;

/// What the field shows.
#[derive(Clone, Default, PartialEq)]
pub struct TextFieldModel {
    /// Its persistent name: it stays when there is text.
    pub label: String,
    pub text: String,
    /// A password: bullets while hidden, and a Show/Hide control. A field is a secret or not for its whole life: build another to
    /// change it (a field built plain that is given a secret is shown as one, but has no control to reveal it).
    pub secret: bool,
    /// A secret is shown as typed.
    pub revealed: bool,
    /// What went wrong, in the danger colour with a mark in front.
    pub error: Option<String>,
    /// What is asked for, when nothing went wrong.
    pub hint: Option<String>,
    /// The field is being typed into: its caret shows and its well has the focus cue.
    pub focused: bool,
    /// The selected characters, as indexes into the text's characters (not bytes). Shown as a wash behind them; the caret hides.
    pub selection: Option<Range<usize>>,
}

/// A secret's characters are never printed, so a model in a log or a failed assertion cannot leak a password.
impl std::fmt::Debug for TextFieldModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = if self.secret {
            format!("<{} hidden characters>", self.text.chars().count())
        } else {
            format!("{:?}", self.text)
        };
        f.debug_struct("TextFieldModel")
            .field("label", &self.label)
            .field("text", &format_args!("{text}"))
            .field("secret", &self.secret)
            .field("revealed", &self.revealed)
            .field("error", &self.error)
            .field("hint", &self.hint)
            .field("focused", &self.focused)
            .field("selection", &self.selection)
            .finish()
    }
}

impl TextFieldModel {
    /// An empty plain field called `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    /// An empty password field called `label`.
    pub fn secret(label: impl Into<String>) -> Self {
        Self {
            secret: true,
            ..Self::new(label)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldAction {
    /// A tap on the field: the screen focuses it and shows its keyboard.
    Activated,
    /// A tap on the Show/Hide control: the screen asks for the other of `revealed`.
    ToggleReveal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteKind {
    Error,
    Hint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    Toggle,
    Body,
}

/// The type of the value: the title size at the person's text size. Bullets and typed characters are set the same, big enough to read
/// at arm's length.
fn value_type(ui: &Ui) -> Type {
    ui.look.ty(ty::TITLE)
}

fn label_type(ui: &Ui) -> Type {
    ui.look.ty(ty::LABEL)
}

fn note_type(ui: &Ui) -> Type {
    ui.look.ty(ty::CAPTION)
}

/// The string the value line draws and how many characters of the text were cut off its start: a bullet for each character, or the
/// text itself when `plain`, cut at its start (behind an ellipsis) when it is longer than the line, so that what was typed last,
/// where the caret is, is always in view.
fn shown_for(fonts: &mut Fonts, text: &str, plain: bool, ty: Type, max_w: f32) -> (String, usize) {
    let full: String = if plain {
        text.to_owned()
    } else {
        text.chars().map(|_| BULLET).collect()
    };
    let shown = fit_tail(fonts, &full, ty, max_w);
    let dropped = if shown == full {
        0
    } else {
        full.chars().count() - (shown.chars().count() - 1)
    };
    (shown, dropped)
}

/// The string the value line draws: see [`shown_for`].
pub fn displayed(fonts: &mut Fonts, text: &str, plain: bool, ty: Type, max_w: f32) -> String {
    shown_for(fonts, text, plain, ty, max_w).0
}

pub struct TextField {
    model: TextFieldModel,
    secret: bool,
    well: Frame,
    group: Group,
    surface: Surface,
    label: Label,
    label_w: f32,
    value: Label,
    value_x: f32,
    /// The widest the value's text may be: its box, less the room the caret takes.
    text_w: f32,
    caret: ShapeLayer,
    caret_props: Props,
    caret_y: f32,
    caret_h: f32,
    /// The selection's wash: as wide as the value's box, clipped to the selected characters.
    wash: ShapeLayer,
    wash_props: Props,
    note: TextLayer,
    note_frame: Frame,
    note_shown: Option<(NoteKind, String)>,
    /// What the note's text is as drawn (cut if it did not fit).
    note_text: String,
    toggle: Option<Button>,
    press: PressTracker,
    grab: Option<(u32, Grab)>,
}

impl TextField {
    /// The least a well is high at the person's text size: a touch target, or the value's line with room above and below.
    pub fn well_height(ui: &Ui) -> f32 {
        (line_height(value_type(ui)) + 2.0 * space::S2).max(smart::TARGET)
    }

    /// The height of the line under the well, at the person's text size.
    pub fn note_height(ui: &Ui) -> f32 {
        line_height(note_type(ui))
    }

    /// What a field takes from the top of its well to the bottom of its note.
    pub fn required_height(ui: &Ui) -> f32 {
        Self::well_height(ui) + NOTE_GAP + Self::note_height(ui)
    }

    /// Rasters every glyph the field draws at the sizes it draws them, so that no glyph is rastered when a character is typed.
    pub fn warm(ui: &mut Ui) {
        let types = [value_type(ui), label_type(ui), note_type(ui)];
        ui.warm(&types);
    }

    /// A field whose well covers `frame` (at least [`TextField::well_height`] high), its note line under it, all from `z` up.
    pub fn new(ui: &mut Ui, frame: Frame, z: i32, model: &TextFieldModel) -> io::Result<Self> {
        let need = Self::well_height(ui);
        if frame.h < need {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a field's well needs {need} units at this text size, not {}",
                    frame.h
                ),
            ));
        }
        let (vty, lty, nty) = (value_type(ui), label_type(ui), note_type(ui));
        let mut group = Group::new(frame.centre());
        let surface = Surface::add(
            ui,
            &mut group,
            frame,
            z,
            SmartMaterial::FieldWell,
            MaterialContext::Nested,
            ControlState::default(),
        );

        let label_w = ui
            .fonts
            .measure(&model.label, lty)
            .width
            .ceil()
            .min((frame.w * 0.35).floor());
        let label_text = ellipsise(&mut ui.fonts, &model.label, lty, label_w);
        let label_ink = ui.look.ink2();
        let label = super::label::Label::new(
            ui,
            Frame::new(frame.x + PAD, frame.y, label_w, frame.h),
            z + 1,
            style(lty, label_ink, Align::Left),
            &label_text,
        )?;

        let toggle_size = frame.h - 2.0 * space::S2;
        let toggle_frame = Frame::new(
            frame.right() - space::S3 - toggle_size,
            frame.y + space::S2,
            toggle_size,
            toggle_size,
        );
        let value_x = frame.x + PAD + label_w + space::S4;
        let limit = if model.secret {
            toggle_frame.x - space::S2
        } else {
            frame.right() - PAD
        };
        let value_w = limit - value_x;
        if value_w < 3.0 * vty.px as f32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a field {} units wide has no room for its value after its label",
                    frame.w
                ),
            ));
        }
        let value = Label::new(
            ui,
            Frame::new(value_x, frame.y, value_w, frame.h),
            z + 1,
            style(vty, ui.look.ink(), Align::Left),
            "",
        )?;

        let caret_h = (vty.px as f32 * CARET_HEIGHT).ceil();
        let caret_y = frame.y + ((frame.h - caret_h) / 2.0).floor();
        let hidden = |x: f32| Props {
            visible: false,
            ..Props::at(x, caret_y)
        };
        let caret = ui.scene.shape(
            CARET_W as u32,
            caret_h as u32,
            z + 1,
            solid(ui.look.accent(), CARET_W / 2.0),
            hidden(value_x),
        );
        let wash = ui.scene.shape(
            value_w as u32,
            caret_h as u32,
            z + 1,
            solid(ui.look.theme.colour_scaled(PaletteRole::Accent, 0.32), 4.0),
            hidden(value_x),
        );

        let note_frame = Frame::new(
            frame.x + PAD,
            frame.bottom() + NOTE_GAP,
            frame.w - 2.0 * PAD,
            line_height(nty),
        );
        let note = TextLayer::new(ui, note_frame, z + 1, &[])?;

        let toggle = if model.secret {
            Some(Button::new(
                ui,
                toggle_frame,
                z + 2,
                ButtonKind::Icon,
                MaterialContext::Nested,
                &toggle_model(&model.label, model.revealed),
            )?)
        } else {
            None
        };

        let mut field = Self {
            // What is drawn so far is the blank field: `apply` draws what the model differs in.
            model: TextFieldModel {
                label: model.label.clone(),
                secret: model.secret,
                revealed: model.revealed,
                ..TextFieldModel::default()
            },
            secret: model.secret,
            well: frame,
            group,
            surface,
            label,
            label_w,
            value,
            value_x,
            text_w: value_w - CARET_W - 2.0,
            caret,
            caret_props: hidden(value_x),
            caret_y,
            caret_h,
            wash,
            wash_props: hidden(value_x),
            note,
            note_frame,
            note_shown: None,
            note_text: String::new(),
            toggle,
            press: PressTracker::new(),
            grab: None,
        };
        field.apply(ui, model);
        field.model = model.clone();
        Ok(field)
    }

    pub fn model(&self) -> &TextFieldModel {
        &self.model
    }

    /// Whether the value is drawn as bullets: a secret that is not revealed.
    fn hidden(&self, model: &TextFieldModel) -> bool {
        (self.secret || model.secret) && !model.revealed
    }

    /// The text of the note line as drawn, empty when there is none.
    pub fn note_text(&self) -> &str {
        &self.note_text
    }

    /// Where the Show/Hide control takes a touch, if the field has one: at least 64 units each way.
    pub fn toggle_hit(&self) -> Option<Frame> {
        self.toggle.as_ref().map(Button::hit)
    }

    /// The look was replaced: the glass is resolved again and the text, caret and note drawn in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.surface.restyle(ui, &mut self.group);
        let (ink, ink2) = (ui.look.ink(), ui.look.ink2());
        self.label.set_colour(ui, ink2);
        self.value.set_colour(ui, ink);
        self.caret.reshape(solid(ui.look.accent(), CARET_W / 2.0));
        self.wash.reshape(solid(
            ui.look.theme.colour_scaled(PaletteRole::Accent, 0.32),
            4.0,
        ));
        if let Some((kind, text)) = self.note_shown.take() {
            self.draw_note(ui, Some((kind, &text)));
            self.note_shown = Some((kind, text));
        }
        if let Some(toggle) = &mut self.toggle {
            toggle.restyle(ui);
        }
    }

    /// Shows `model`: what differs from what is drawn is drawn, nothing else.
    fn apply(&mut self, ui: &mut Ui, model: &TextFieldModel) -> Damage {
        let mut damage = Damage::None;
        let vty = value_type(ui);
        if model.focused != self.model.focused {
            let state = ControlState {
                focus_visible: model.focused,
                ..ControlState::default()
            };
            self.surface.set_state(ui, &mut self.group, state);
            damage = damage.max(Damage::Properties);
        }
        if model.label != self.model.label {
            let lty = label_type(ui);
            let text = ellipsise(&mut ui.fonts, &model.label, lty, self.label_w);
            self.label.set(ui, &text);
            damage = damage.max(Damage::Content);
        }

        let plain = !self.hidden(model);
        let (shown, dropped) = shown_for(&mut ui.fonts, &model.text, plain, vty, self.text_w);
        if shown != self.value.text() {
            self.value.set(ui, &shown);
            damage = damage.max(Damage::Content);
        }
        let end = self.value_x + ui.fonts.measure(&shown, vty).width;
        let total = model.text.chars().count();
        let selected = model
            .selection
            .clone()
            .map(|r| r.start.min(total)..r.end.min(total))
            .filter(|r| r.start < r.end);

        let caret = Props {
            visible: model.focused && selected.is_none(),
            ..Props::at(end + 2.0, self.caret_y)
        };
        if caret != self.caret_props {
            self.caret.set(caret);
            self.caret_props = caret;
            damage = damage.max(Damage::Properties);
        }
        let wash = self.wash_props_for(ui, &shown, dropped, selected);
        if wash != self.wash_props {
            self.wash.set(wash);
            self.wash_props = wash;
            damage = damage.max(Damage::Properties);
        }

        let note = match (&model.error, &model.hint) {
            (Some(error), _) => Some((NoteKind::Error, error.as_str())),
            (None, Some(hint)) => Some((NoteKind::Hint, hint.as_str())),
            (None, None) => None,
        };
        if note != self.note_shown.as_ref().map(|(k, t)| (*k, t.as_str())) {
            self.draw_note(ui, note);
            self.note_shown = note.map(|(k, t)| (k, t.to_owned()));
            damage = damage.max(Damage::Content);
        }

        if let Some(toggle) = &mut self.toggle {
            damage = damage.max(toggle.update(&toggle_model(&model.label, model.revealed), ui));
        }
        damage
    }

    /// The wash behind the selected characters: the layer clipped to where they are in what is shown. Characters cut off the start
    /// of a long value are under its ellipsis, which the wash covers when the selection reaches into them.
    fn wash_props_for(
        &self,
        ui: &mut Ui,
        shown: &str,
        dropped: usize,
        selected: Option<Range<usize>>,
    ) -> Props {
        let hidden = Props {
            visible: false,
            ..Props::at(self.value_x, self.caret_y)
        };
        let Some(sel) = selected else {
            return hidden;
        };
        if sel.end <= dropped {
            return hidden;
        }
        let cut = usize::from(dropped > 0);
        let from = if sel.start < dropped {
            0
        } else {
            sel.start - dropped + cut
        };
        let to = sel.end - dropped + cut;
        let vty = value_type(ui);
        let chars: Vec<char> = shown.chars().collect();
        let mut width = |n: usize| {
            let prefix: String = chars[..n.min(chars.len())].iter().collect();
            ui.fonts.measure(&prefix, vty).width
        };
        let (x0, x1) = (width(from), width(to));
        Props {
            clip: Some(Rect {
                x: self.value_x + x0,
                y: self.caret_y,
                w: x1 - x0,
                h: self.caret_h,
            }),
            ..Props::at(self.value_x, self.caret_y)
        }
    }

    /// Draws the note line: nothing, an error with its mark in front in the danger colour, or a hint in the quiet one.
    fn draw_note(&mut self, ui: &mut Ui, note: Option<(NoteKind, &str)>) {
        let nty = note_type(ui);
        let (w, h) = (self.note_frame.w.ceil(), self.note_frame.h.ceil());
        self.note_text.clear();
        let Some((kind, text)) = note else {
            self.note.draw(ui, &[]);
            return;
        };
        let (colour, lead) = match kind {
            NoteKind::Error => (ui.look.danger(), nty.px as f32 + space::S1),
            NoteKind::Hint => (ui.look.ink2(), 0.0),
        };
        let cut = ellipsise(&mut ui.fonts, text, nty, w - lead);
        // The line box is the layer's own height, so the line sits at its top.
        let mut items = vec![Item::Text(Run::left(
            &cut,
            nty,
            colour,
            lead,
            0.0,
            w - lead,
        ))];
        if kind == NoteKind::Error {
            let size = nty.px;
            let top = ((h - size as f32) / 2.0).round() as i32;
            items.push(Item::Mark(Mark::icon(Icon::Alert, size, 0, top, colour)));
        }
        self.note.draw(ui, &items);
        self.note_text = cut;
    }

    /// The layer the value is drawn on, for tests that look at what the compositor was asked.
    #[cfg(test)]
    pub(crate) fn value_layer(&self) -> compositor_protocol::LayerId {
        self.value.id()
    }

    /// The text of the value line as drawn (bullets for a hidden secret).
    #[cfg(any(test, feature = "testing"))]
    pub fn shown_text(&self) -> &str {
        self.value.text()
    }
}

/// What the Show/Hide control says: the eye shows a hidden secret, the struck eye hides a shown one.
fn toggle_model(label: &str, revealed: bool) -> ButtonModel {
    if revealed {
        ButtonModel::icon(Icon::EyeOff, format!("Hide {label}"))
    } else {
        ButtonModel::icon(Icon::Eye, format!("Show {label}"))
    }
}

impl Component for TextField {
    type Model = TextFieldModel;
    type Action = TextFieldAction;

    fn update(&mut self, model: &TextFieldModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        debug_assert_eq!(
            model.secret, self.secret,
            "a field is a secret or not for its whole life"
        );
        let damage = self.apply(ui, model);
        self.model = model.clone();
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<TextFieldAction> {
        let Event::Touch(touch) = event else {
            return None;
        };
        if touch.phase == TouchPhase::Down && self.grab.is_none() {
            let on_toggle = self
                .toggle
                .as_ref()
                .is_some_and(|t| t.hit().contains(touch.x, touch.y));
            self.grab = if on_toggle {
                Some((touch.id, Grab::Toggle))
            } else if self.well.contains(touch.x, touch.y) {
                Some((touch.id, Grab::Body))
            } else {
                None
            };
        }
        let (id, grab) = self.grab?;
        let ends = touch.id == id && matches!(touch.phase, TouchPhase::Up | TouchPhase::Cancel);
        let action = match grab {
            Grab::Toggle => self
                .toggle
                .as_mut()
                .and_then(|t| t.input(event, ui))
                .map(|_| TextFieldAction::ToggleReveal),
            Grab::Body => (self.press.feed(&self.well, touch) == Step::Activated)
                .then_some(TextFieldAction::Activated),
        };
        if ends {
            self.grab = None;
        }
        action
    }

    fn frame(&self) -> Frame {
        self.well
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        // A hidden secret is described by how many characters it has, never by which.
        let value = if self.hidden(&self.model) {
            match self.model.text.chars().count() {
                0 => "empty".to_owned(),
                1 => "1 character".to_owned(),
                n => format!("{n} characters"),
            }
        } else {
            self.model.text.clone()
        };
        out.push(
            SemanticNode::new(SemanticKind::TextField, self.model.label.clone(), self.well)
                .value(value)
                .actionable(),
        );
        if let Some(toggle) = &self.toggle {
            toggle.semantics(out);
        }
        match (&self.model.error, &self.model.hint) {
            (Some(error), _) => out.push(
                SemanticNode::new(SemanticKind::Status, error.clone(), self.note_frame)
                    .value("Error"),
            ),
            (None, Some(hint)) => out.push(SemanticNode::new(
                SemanticKind::Label,
                hint.clone(),
                self.note_frame,
            )),
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchEvent;
    use crate::testing::{Harness, harness};
    use compositor_protocol::Request;

    const FRAME: Frame = Frame::new(12.0, 84.0, 936.0, 64.0);
    const SECRET: &str = "hunter2-é";

    fn field(h: &mut Harness, model: &TextFieldModel) -> TextField {
        let f = TextField::new(&mut h.ui, FRAME, 10, model).unwrap();
        h.commit();
        f
    }

    fn touch(
        f: &mut TextField,
        h: &mut Harness,
        phase: TouchPhase,
        (x, y): (f32, f32),
        at_ns: u64,
    ) -> Option<TextFieldAction> {
        f.input(
            &Event::Touch(TouchEvent {
                id: 1,
                phase,
                x,
                y,
                at_ns,
            }),
            &mut h.ui,
        )
    }

    fn tap(f: &mut TextField, h: &mut Harness, at: (f32, f32)) -> Option<TextFieldAction> {
        touch(f, h, TouchPhase::Down, at, 0);
        touch(f, h, TouchPhase::Up, at, 50_000_000)
    }

    #[test]
    fn hidden_text_is_one_bullet_per_character_and_shown_text_is_as_typed() {
        let mut fonts = Fonts::new();
        let t = ty::TITLE;
        assert_eq!(displayed(&mut fonts, "", false, t, 600.0), "");
        assert_eq!(displayed(&mut fonts, "abc", false, t, 600.0), "•••");
        assert_eq!(displayed(&mut fonts, "p@ss é", false, t, 600.0), "••••••");
        assert_eq!(displayed(&mut fonts, "p@ss é", true, t, 600.0), "p@ss é");
    }

    #[test]
    fn a_value_longer_than_its_line_keeps_its_end_in_view_behind_an_ellipsis_and_fits() {
        let mut fonts = Fonts::new();
        let t = ty::TITLE;
        let text = "correct horse battery staple, then some more words to fill the line right up";
        let max_w = 300.0;
        let shown = displayed(&mut fonts, text, true, t, max_w);
        assert!(
            shown.starts_with('…') && shown.ends_with("right up"),
            "{shown}"
        );
        assert!(fonts.measure(&shown, t).width <= max_w, "{shown}");
        assert!(
            shown.chars().count() > 8,
            "as much as fits is kept, not just the end: {shown}"
        );
        let bullets = displayed(&mut fonts, &"x".repeat(63), false, t, max_w);
        assert!(bullets.starts_with('…') && bullets.ends_with('•'));
        assert!(fonts.measure(&bullets, t).width <= max_w);
        assert_eq!(displayed(&mut fonts, "short", true, t, max_w), "short");
    }

    #[test]
    fn a_hidden_secret_is_bullets_on_screen_and_a_count_in_semantics_and_debug_and_the_eye_reveals_it()
     {
        let mut h = harness();
        let mut model = TextFieldModel::secret("Password");
        model.text = SECRET.to_owned();
        let mut f = field(&mut h, &model);
        assert_eq!(f.shown_text(), "•".repeat(SECRET.chars().count()));
        let mut sink = SemanticsSink::new();
        f.semantics(&mut sink);
        let node = sink.find("Password").expect("the field");
        assert_eq!(node.value.as_deref(), Some("9 characters"));
        assert!(
            !format!("{:?} {:?}", sink.nodes(), model).contains("hunter"),
            "no plaintext in semantics or debug"
        );
        assert!(sink.find("Show Password").is_some(), "the eye");

        // Revealing shows the text and swaps the eye for its struck twin.
        model.revealed = true;
        f.update(&model, &mut h.ui);
        assert_eq!(f.shown_text(), SECRET);
        let mut sink = SemanticsSink::new();
        f.semantics(&mut sink);
        assert!(sink.find("Hide Password").is_some() && sink.find("Show Password").is_none());
        model.revealed = false;
        f.update(&model, &mut h.ui);
        assert_eq!(f.shown_text(), "•".repeat(9));
    }

    #[test]
    fn the_eye_asks_to_reveal_and_a_tap_on_the_well_asks_for_the_keyboard_and_a_slide_off_asks_for_nothing()
     {
        let mut h = harness();
        let mut model = TextFieldModel::secret("Password");
        model.text = "abc".to_owned();
        let mut f = field(&mut h, &model);
        let eye = f.toggle_hit().expect("a secret has an eye");
        assert!(eye.w >= 64.0 && eye.h >= 64.0);
        assert_eq!(
            tap(&mut f, &mut h, eye.centre()),
            Some(TextFieldAction::ToggleReveal)
        );
        assert_eq!(
            tap(&mut f, &mut h, (FRAME.x + 200.0, FRAME.y + 32.0)),
            Some(TextFieldAction::Activated)
        );
        // Down on the well, slide out, lift: nothing.
        let inside = (FRAME.x + 200.0, FRAME.y + 32.0);
        touch(&mut f, &mut h, TouchPhase::Down, inside, 0);
        touch(
            &mut f,
            &mut h,
            TouchPhase::Move,
            (FRAME.x + 200.0, FRAME.bottom() + 300.0),
            10_000_000,
        );
        assert_eq!(
            touch(
                &mut f,
                &mut h,
                TouchPhase::Up,
                (FRAME.x + 200.0, FRAME.bottom() + 300.0),
                20_000_000
            ),
            None
        );
        // A cancelled touch is nothing either, and the next tap works.
        touch(&mut f, &mut h, TouchPhase::Down, inside, 0);
        assert_eq!(
            touch(&mut f, &mut h, TouchPhase::Cancel, inside, 10_000_000),
            None
        );
        assert_eq!(
            tap(&mut f, &mut h, inside),
            Some(TextFieldAction::Activated)
        );
        // A touch outside the field is not the field's.
        assert_eq!(
            tap(&mut f, &mut h, (FRAME.x + 200.0, FRAME.bottom() + 300.0)),
            None
        );
        // A plain field has no eye.
        let plain = field(&mut h, &TextFieldModel::new("Name"));
        assert!(plain.toggle_hit().is_none());
    }

    #[test]
    fn the_caret_follows_focus_and_the_end_of_the_text_and_hides_behind_a_selection() {
        let mut h = harness();
        let mut model = TextFieldModel::new("Name");
        let mut f = field(&mut h, &model);
        assert!(!f.caret_props.visible);
        model.focused = true;
        f.update(&model, &mut h.ui);
        let empty_x = f.caret_props.x;
        assert!(f.caret_props.visible);
        model.text = "abc".to_owned();
        f.update(&model, &mut h.ui);
        assert!(f.caret_props.x > empty_x, "the caret sits after the text");
        model.selection = Some(0..2);
        f.update(&model, &mut h.ui);
        assert!(!f.caret_props.visible && f.wash_props.clip.is_some_and(|c| c.w > 0.0));
        model.selection = Some(5..9);
        f.update(&model, &mut h.ui);
        assert!(
            f.wash_props.clip.is_none(),
            "a selection past the end is clamped away"
        );
    }

    #[test]
    fn a_typed_character_rewrites_the_value_once_and_an_unchanged_model_asks_for_nothing() {
        let mut h = harness();
        TextField::warm(&mut h.ui);
        let mut model = TextFieldModel::new("Name");
        model.focused = true;
        let mut f = field(&mut h, &model);
        assert_eq!(f.update(&model, &mut h.ui), Damage::None);
        model.text.push('a');
        assert_eq!(f.update(&model, &mut h.ui), Damage::Content);
        let requests = h.commit();
        assert_eq!(
            requests
                .iter()
                .filter(|r| matches!(r, Request::Damage { .. }))
                .count(),
            1,
            "{requests:?}"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| matches!(r, Request::Create { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn an_error_wins_over_a_hint_with_a_mark_and_a_long_message_is_cut_to_its_line() {
        let mut h = harness();
        let mut model = TextFieldModel::new("Name");
        model.hint = Some("At least 8 characters".to_owned());
        let mut f = field(&mut h, &model);
        assert_eq!(f.note_text(), "At least 8 characters");
        let plain_ink = h.pixels(f.note.id());
        model.error =
            Some("That password did not work, please try typing it once more. ".repeat(4));
        f.update(&model, &mut h.ui);
        assert!(f.note_text().ends_with('…'), "{}", f.note_text());
        let mut sink = SemanticsSink::new();
        f.semantics(&mut sink);
        assert!(
            sink.nodes().iter().any(|n| n.kind == SemanticKind::Status),
            "an error is a status"
        );
        assert_ne!(h.pixels(f.note.id()), plain_ink);
        model.error = None;
        model.hint = None;
        f.update(&model, &mut h.ui);
        assert_eq!(f.note_text(), "");
    }

    #[test]
    fn at_200_percent_text_the_well_grows_and_a_frame_too_small_for_it_is_refused() {
        let mut h = harness();
        let normal = TextField::well_height(&h.ui);
        assert!(normal >= smart::TARGET);
        h.ui.look.text_scale = 2.0;
        let big = TextField::well_height(&h.ui);
        assert!(big > normal, "{big} vs {normal}");
        assert!(
            TextField::new(
                &mut h.ui,
                Frame::new(12.0, 84.0, 936.0, normal),
                10,
                &TextFieldModel::new("Name")
            )
            .is_err()
        );
        let mut model = TextFieldModel::secret("Network password for the guest network");
        model.text = "x".repeat(40);
        let f = TextField::new(&mut h.ui, Frame::new(12.0, 84.0, 936.0, big), 10, &model).unwrap();
        assert!(
            f.shown_text().starts_with('…') && f.shown_text().ends_with('•'),
            "{}",
            f.shown_text()
        );
    }

    // ---- the note line: a picture as it always was, and glyph quads ----

    const HINT: &str = "At least 8 characters";
    const OTHER_HINT: &str = "At most 63 characters";
    const ERROR: &str = "That password did not work";

    fn long_error() -> String {
        "That password did not work, please try typing it once more. ".repeat(4)
    }

    /// The note line painted the way it was before it was a text layer (`paint_over` and `draw_line` into a picture).
    fn painted_note(h: &mut Harness, f: &TextField, kind: NoteKind, text: &str) -> Vec<u8> {
        use crate::components::icon::paint_over;
        use crate::components::label::draw_line;
        use crate::text::Canvas;
        let nty = note_type(&h.ui);
        let (w, hh) = (f.note_frame.w.ceil(), f.note_frame.h.ceil());
        let mut layer =
            h.ui.scene
                .image(w as u32, hh as u32, 0, Props::at(0.0, 0.0))
                .unwrap();
        let (colour, lead) = match kind {
            NoteKind::Error => (h.ui.look.danger(), nty.px as f32 + space::S1),
            NoteKind::Hint => (h.ui.look.ink2(), 0.0),
        };
        if kind == NoteKind::Error {
            let size = nty.px;
            paint_over(
                &mut layer,
                Icon::Alert,
                0,
                ((hh - size as f32) / 2.0).round() as i32,
                size,
                colour,
            );
        }
        let cut = ellipsise(&mut h.ui.fonts, text, nty, w - lead);
        draw_line(
            &mut h.ui.fonts,
            &mut Canvas {
                pixels: layer.pixels_mut(),
                width: w as usize,
            },
            Frame::new(lead, 0.0, w - lead, hh),
            &cut,
            style(nty, colour, Align::Left),
        );
        layer.damage(None);
        h.commit();
        h.pixels(layer.id())
    }

    fn with_note(hint: Option<&str>, error: Option<&str>) -> TextFieldModel {
        let mut model = TextFieldModel::new("Name");
        model.hint = hint.map(str::to_owned);
        model.error = error.map(str::to_owned);
        model
    }

    /// Answers every atlas upload the way the compositor does, so that what was prepared counts as shown.
    fn land_uploads(h: &mut Harness) {
        for _ in 0..16 {
            for request in h.all() {
                if let Request::AtlasUpload {
                    id,
                    slot,
                    generation,
                    ..
                } = request
                {
                    h.ui.scene
                        .observe(&compositor_protocol::Event::AtlasUploaded {
                            id,
                            slot,
                            generation,
                        });
                }
            }
            h.commit();
        }
    }

    /// A glyph-text UI whose atlas has seen every note the tests show, as one that showed them before is.
    fn primed() -> Harness {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let long = long_error();
        for model in [
            with_note(Some(HINT), None),
            with_note(Some(OTHER_HINT), None),
            with_note(None, Some(ERROR)),
            with_note(None, Some(&long)),
        ] {
            let f = field(&mut h, &model);
            land_uploads(&mut h);
            drop(f);
        }
        h.commit();
        h
    }

    fn pictures(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Request::Create {
                        content: compositor_protocol::Content::Image,
                        ..
                    }
                )
            })
            .count()
    }

    fn written(requests: &[Request]) -> f32 {
        requests
            .iter()
            .map(|r| match r {
                Request::Damage { rect, .. } => rect.w * rect.h,
                _ => 0.0,
            })
            .sum()
    }

    fn count(requests: &[Request], f: impl Fn(&Request) -> bool) -> usize {
        requests.iter().filter(|r| f(r)).count()
    }

    #[test]
    fn as_a_picture_the_note_is_the_pixels_the_text_painters_made() {
        let long = long_error();
        for (kind, text) in [
            (NoteKind::Hint, HINT),
            (NoteKind::Error, ERROR),
            (NoteKind::Error, long.as_str()),
        ] {
            let mut h = harness();
            let model = match kind {
                NoteKind::Hint => with_note(Some(text), None),
                NoteKind::Error => with_note(None, Some(text)),
            };
            let f = field(&mut h, &model);
            let painted = painted_note(&mut h, &f, kind, text);
            assert!(painted.iter().any(|b| *b != 0));
            assert_eq!(h.pixels(f.note.id()), painted, "{kind:?} {text}");
        }
    }

    #[test]
    fn on_glyph_text_the_note_shows_the_same_pixels_as_the_picture_in_every_state_it_goes_through()
    {
        let long = long_error();
        let states = [
            with_note(Some(HINT), None),
            with_note(None, Some(ERROR)),
            with_note(None, Some(&long)),
            with_note(None, None),
            with_note(Some(OTHER_HINT), None),
        ];
        let mut pic = harness();
        let mut fp = field(&mut pic, &states[0]);
        let mut run = primed();
        let mut fr = field(&mut run, &states[0]);
        assert!(fr.note.is_glyph_run());
        for (i, state) in states.iter().enumerate() {
            fp.update(state, &mut pic.ui);
            fr.update(state, &mut run.ui);
            pic.commit();
            run.commit();
            assert_eq!(fr.note_text(), fp.note_text());
            let (a, b) = (
                run.layer_pixels(fr.note.id()),
                pic.layer_pixels(fp.note.id()),
            );
            assert_eq!(a, b, "state {i}: the run is not the picture");
            assert_eq!(
                a.iter().any(|v| *v != 0),
                state.error.is_some() || state.hint.is_some(),
                "state {i}"
            );
        }
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
    }

    #[test]
    fn on_glyph_text_a_field_with_a_note_makes_no_picture_and_writes_no_pixel_and_a_new_note_is_a_swap()
     {
        let mut h = primed();
        let mut f = TextField::new(&mut h.ui, FRAME, 10, &with_note(Some(HINT), None)).unwrap();
        let sent = h.commit();
        assert!(f.note.is_glyph_run());
        assert_eq!(pictures(&sent), 0, "{sent:?}");
        assert_eq!(written(&sent), 0.0);

        // Other words in the same ink: one swap.
        f.update(&with_note(Some(OTHER_HINT), None), &mut h.ui);
        let sent = h.commit();
        assert_eq!(
            count(&sent, |r| matches!(r, Request::SetGlyphs { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(count(&sent, |r| matches!(r, Request::Create { .. })), 0);
        assert_eq!(written(&sent), 0.0);

        // An error is another ink and a mark: the palette is replaced and the quads swapped, still nothing written.
        f.update(&with_note(None, Some(ERROR)), &mut h.ui);
        let sent = h.commit();
        assert_eq!(count(&sent, |r| matches!(r, Request::Create { .. })), 0);
        assert_eq!(written(&sent), 0.0);
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
    }
}
