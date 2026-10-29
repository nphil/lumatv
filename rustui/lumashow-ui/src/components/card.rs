//! The card: a raised glass tile with a title, a big value and a few lines of text, and an icon.
//!
//! A card stands on its own over the wallpaper, so its glass has the cached backdrop and the single raised shadow, and the depth
//! pair and sheen that every card has (§3.4). When the card itself can be tapped it shrinks to .97 about its middle while pressed
//! and casts its prepared pressed shadow; a card that cannot be tapped has exactly the same glass and does nothing when touched.
//!
//! All of a card's text is ONE layer (a [`TextLayer`]: a run of glyph quads with glyph text, a picture otherwise), drawn again only
//! when the words or their colour change. The block of text sits at the bottom of the card, left-aligned, and the icon at the top
//! right: that keeps the words out of the sheen's strongest band, which is the card's upper-left corner. The caller gives the card
//! its frame; [`Card::min_height`] says how tall the card must be to show all of its text at the person's text size.

use super::group::{Group, Part};
use super::icon::{Icon, icon_part, paint};
use super::press::{PressTracker, Step};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::text::{Run, ellipsise, line_height, wrap_to};
use super::text_layer::{Item, TextLayer};
use super::toggle::hide_idle_ring;
use super::{Component, Damage, target_around};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::tokens::{smart, space, ty};
use crate::text::Type;
use compositor_protocol::Rgba;
use std::io;

/// Which of the two card shapes it is. The caller decides the size in units; the size decides how much text the card keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardSize {
    Full,
    Half,
}

impl CardSize {
    /// The most lines of body text a card of this size keeps; the last ends in an ellipsis when there is more.
    pub fn max_lines(self) -> usize {
        match self {
            CardSize::Full => 3,
            CardSize::Half => 2,
        }
    }
}

/// What a card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct CardModel {
    pub title: String,
    /// The big number or word ("21°", "Playing").
    pub value: Option<String>,
    pub body: Option<String>,
    pub icon: Option<Icon>,
    /// Whether the card itself can be tapped. Only then does it press and activate.
    pub actionable: bool,
    /// Selected, enabled, focused. A disabled card keeps its glass and its readable text and takes no touch.
    pub state: ControlState,
}

impl CardModel {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            value: None,
            body: None,
            icon: None,
            actionable: false,
            state: ControlState::default(),
        }
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn tappable(mut self) -> Self {
        self.actionable = true;
        self
    }

    fn takes_touch(&self) -> bool {
        self.actionable && self.state.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// Released inside an actionable card.
    Activated,
}

const PAD: f32 = space::S5;
const GAP: f32 = space::S1;
const ICON: f32 = 32.0;
const TITLE_LINES: usize = 2;

/// The words of a card, broken into the lines that are drawn.
#[derive(Clone, Debug, PartialEq, Default)]
struct Lines {
    title: Vec<String>,
    value: Option<String>,
    body: Vec<String>,
}

/// The type of each part of the text at the person's text size. A card with a body keeps its value at title size, so that the body
/// has room; a full card with no body gives its value the display size.
fn types(ui: &Ui, size: CardSize, model: &CardModel) -> (Type, Type, Type) {
    let value = if size == CardSize::Full && model.body.is_none() {
        ty::DISPLAY
    } else {
        ty::TITLE
    };
    (
        ui.look.ty(ty::LABEL),
        ui.look.ty(value),
        ui.look.ty(ty::BODY),
    )
}

fn icon_row(model: &CardModel) -> f32 {
    if model.icon.is_some() {
        ICON + space::S2
    } else {
        0.0
    }
}

/// The lines of `model` in a card `width` wide. With `height` given the body keeps only as many lines as fit (and at most the
/// size's); with `None` it keeps all it may.
fn compose(
    ui: &mut Ui,
    model: &CardModel,
    size: CardSize,
    width: f32,
    height: Option<f32>,
) -> (Lines, f32) {
    let inner = (width - 2.0 * PAD).max(0.0);
    let (tt, vt, bt) = types(ui, size, model);
    let title = wrap_to(&mut ui.fonts, &model.title, tt, inner, TITLE_LINES);
    let value = model
        .value
        .as_ref()
        .map(|v| ellipsise(&mut ui.fonts, v, vt, inner));
    let mut used = title.len() as f32 * line_height(tt);
    if value.is_some() {
        used += GAP + line_height(vt);
    }
    let body = match &model.body {
        Some(text) => {
            let mut allowed = size.max_lines();
            if let Some(h) = height {
                let room = h - 2.0 * PAD - icon_row(model) - used - GAP;
                allowed = allowed.min((room / line_height(bt)).floor().max(0.0) as usize);
            }
            if allowed == 0 {
                Vec::new()
            } else {
                wrap_to(&mut ui.fonts, text, bt, inner, allowed)
            }
        }
        None => Vec::new(),
    };
    if !body.is_empty() {
        used += GAP + body.len() as f32 * line_height(bt);
    }
    (Lines { title, value, body }, used)
}

/// The lines of a card as runs: the block of title (quiet), value (loud) and body (quiet) at the bottom of `frame`.
fn text_runs<'a>(
    ui: &Ui,
    size: CardSize,
    model: &CardModel,
    lines: &'a Lines,
    frame: Frame,
) -> Vec<Run<'a>> {
    let (tt, vt, bt) = types(ui, size, model);
    let (quiet, loud) = Card::text_ink(ui, model);
    let (x, w) = (PAD, (frame.w - 2.0 * PAD).max(0.0));
    let mut block = lines.title.len() as f32 * line_height(tt);
    if lines.value.is_some() {
        block += GAP + line_height(vt);
    }
    if !lines.body.is_empty() {
        block += GAP + lines.body.len() as f32 * line_height(bt);
    }
    let mut y = (frame.h - PAD - block).max(PAD);
    let mut runs = Vec::new();
    for line in &lines.title {
        runs.push(Run::left(line, tt, quiet, x, y, w));
        y += line_height(tt);
    }
    if let Some(value) = &lines.value {
        y += GAP;
        runs.push(Run::left(value, vt, loud, x, y, w));
        y += line_height(vt);
    }
    if !lines.body.is_empty() {
        y += GAP;
        for line in &lines.body {
            runs.push(Run::left(line, bt, quiet, x, y, w));
            y += line_height(bt);
        }
    }
    runs
}

pub struct Card {
    size: CardSize,
    model: CardModel,
    frame: Frame,
    hit: Frame,
    z: i32,
    group: Group,
    surface: Surface,
    text: usize,
    icon: Option<usize>,
    lines: Lines,
    press: PressTracker,
}

impl Card {
    /// The height a card of `size` needs to show all of `model`'s text in `width` at the person's text size: padding, a row for the
    /// icon if there is one, the title, the value and the body (up to the size's lines). A card given less keeps fewer body lines.
    pub fn min_height(ui: &mut Ui, size: CardSize, model: &CardModel, width: f32) -> f32 {
        let (_, used) = compose(ui, model, size, width, None);
        (2.0 * PAD + icon_row(model) + used).max(smart::TARGET)
    }

    /// A card of `size` covering `frame`.
    pub fn new(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        size: CardSize,
        model: &CardModel,
    ) -> io::Result<Self> {
        let lines = compose(ui, model, size, frame.w, Some(frame.h)).0;
        let mut group = Group::new(frame.centre());
        let surface = Surface::add(
            ui,
            &mut group,
            frame,
            z,
            SmartMaterial::Card,
            MaterialContext::Independent,
            model.state,
        );
        let runs = text_runs(ui, size, model, &lines, frame);
        let layer = TextLayer::new(
            ui,
            frame,
            z + 1,
            &runs.iter().copied().map(Item::Text).collect::<Vec<_>>(),
        )?;
        let text = group.push(Part::new(layer, frame));
        let mut card = Self {
            size,
            model: model.clone(),
            frame,
            hit: target_around(frame, smart::TARGET),
            z,
            group,
            surface,
            text,
            icon: None,
            lines,
            press: PressTracker::new(),
        };
        if let Some(icon) = model.icon {
            card.icon = Some(card.add_icon(ui, icon, model)?);
        }
        Ok(card)
    }

    fn icon_frame(&self) -> Frame {
        Frame::new(
            self.frame.right() - PAD - ICON,
            self.frame.y + PAD,
            ICON,
            ICON,
        )
    }

    fn icon_ink(ui: &Ui, model: &CardModel) -> Rgba {
        if model.state.enabled {
            ui.look.ink()
        } else {
            ui.look.disabled_ink()
        }
    }

    fn add_icon(&mut self, ui: &mut Ui, icon: Icon, model: &CardModel) -> io::Result<usize> {
        let part = icon_part(
            ui,
            icon,
            self.icon_frame(),
            self.z + 1,
            Self::icon_ink(ui, model),
        )?;
        Ok(self.group.push(part))
    }

    fn text_ink(ui: &Ui, model: &CardModel) -> (Rgba, Rgba) {
        if model.state.enabled {
            (ui.look.ink2(), ui.look.ink())
        } else {
            (ui.look.ink3(), ui.look.ink2())
        }
    }

    /// Draws the title, value and body into the one text layer, the block of them at the bottom of the card.
    fn draw_text(&mut self, ui: &mut Ui) {
        let runs = text_runs(ui, self.size, &self.model, &self.lines, self.frame);
        if let Some(layer) = self.group.parts[self.text].lines_mut() {
            layer.draw_runs(ui, &runs);
        }
    }

    pub fn model(&self) -> &CardModel {
        &self.model
    }

    pub fn group(&self) -> &Group {
        &self.group
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Where a touch counts: the card, and at least 64 units around its centre in each direction.
    pub fn hit(&self) -> Frame {
        self.hit
    }

    pub fn is_pressed(&self) -> bool {
        self.press.is_pressed()
    }

    /// The lines of text the card draws now: title, value, body.
    pub fn lines(&self) -> (&[String], Option<&str>, &[String]) {
        (
            &self.lines.title,
            self.lines.value.as_deref(),
            &self.lines.body,
        )
    }

    /// Ends a press that something else has taken (a list that began to scroll): no activation.
    pub fn cancel(&mut self, ui: &Ui) {
        if self.press.cancel() {
            self.set_down(ui, false);
        }
    }

    /// The look was replaced: restyle the glass, draw the text and icon again in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.surface.restyle(ui, &mut self.group);
        self.draw_text(ui);
        self.paint_icon(ui);
    }

    fn paint_icon(&mut self, ui: &Ui) {
        if let (Some(slot), Some(icon)) = (self.icon, self.model.icon) {
            let ink = Self::icon_ink(ui, &self.model);
            if let Some(layer) = self.group.parts[slot].image_mut() {
                paint(layer, icon, ink);
            }
        }
    }

    fn set_down(&mut self, ui: &Ui, down: bool) {
        self.surface.set_down(ui, &mut self.group, down);
        hide_idle_ring(&self.surface, &mut self.group);
    }
}

impl Component for Card {
    type Model = CardModel;
    type Action = CardAction;

    fn update(&mut self, model: &CardModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let mut damage = Damage::Properties;
        // An icon that appears needs a layer; if one cannot be made nothing changes and the next update tries again.
        let mut fresh_icon = false;
        if let (Some(icon), None) = (model.icon, self.icon) {
            match self.add_icon(ui, icon, model) {
                Ok(slot) => {
                    self.icon = Some(slot);
                    fresh_icon = true;
                    damage = Damage::Content;
                }
                Err(_) => return Damage::None,
            }
        }
        if model.state != self.model.state {
            self.surface.set_state(ui, &mut self.group, model.state);
        }
        if !model.takes_touch() && self.press.cancel() {
            self.set_down(ui, false);
        }
        let old = std::mem::replace(&mut self.model, model.clone());
        let lines = compose(ui, model, self.size, self.frame.w, Some(self.frame.h)).0;
        let ink_changed = old.state.enabled != model.state.enabled;
        let body_type_changed = old.body.is_some() != model.body.is_some();
        if lines != self.lines || ink_changed || body_type_changed {
            self.lines = lines;
            self.draw_text(ui);
            damage = Damage::Content;
        }
        if let (Some(slot), false) = (self.icon, fresh_icon) {
            let wanted = model.icon;
            if wanted != old.icon || ink_changed {
                if wanted.is_some() {
                    self.paint_icon(ui);
                }
                let shown = wanted.is_some();
                self.group.parts[slot].rest_opacity = if shown { 1.0 } else { 0.0 };
                self.group.set_parts(&[slot], super::group::Pose::REST);
                damage = Damage::Content;
            }
        }
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<CardAction> {
        let Event::Touch(touch) = event else {
            return None;
        };
        if !self.model.takes_touch() {
            return None;
        }
        let was = self.press.is_pressed();
        let step = self.press.feed(&self.hit, touch);
        let now = step.pressed_after(was);
        if now != was {
            self.set_down(ui, now);
        }
        (step == Step::Activated).then_some(CardAction::Activated)
    }

    #[allow(clippy::misnamed_getters)] // the frame a component takes a touch in is its hit area, not its drawn one
    fn frame(&self) -> Frame {
        self.hit
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        let words: Vec<&str> = [self.model.value.as_deref(), self.model.body.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let mut node = SemanticNode::new(SemanticKind::Card, self.model.title.clone(), self.hit)
            .enabled(self.model.state.enabled)
            .selected(self.model.state.selected);
        if !words.is_empty() {
            node = node.value(words.join(", "));
        }
        out.push(if self.model.takes_touch() {
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
    use crate::lucent::palette::Mode;
    use crate::testing::{
        Harness, count_requests, harness, made_as_picture, pictures_made, pixels_damaged,
        pixels_damaged_in, touch, warm_glyph_harness,
    };
    use compositor_protocol::{Content, LayerId, Request};

    const AT: Frame = Frame::new(40.0, 60.0, 280.0, 200.0);

    fn model() -> CardModel {
        CardModel::new("Weather")
            .with_value("14°")
            .with_body("Light rain until 15:00, then clearing")
            .with_icon(Icon::Cloud)
            .tappable()
    }

    fn card(h: &mut Harness, size: CardSize, model: &CardModel) -> Card {
        let c = Card::new(&mut h.ui, AT, 10, size, model).unwrap();
        h.commit();
        c
    }

    fn text_id(c: &Card) -> LayerId {
        c.group.parts[c.text].layer().id()
    }

    fn is_run(c: &mut Card) -> bool {
        let text = c.text;
        c.group.parts[text]
            .lines_mut()
            .is_some_and(|l| l.is_glyph_run())
    }

    /// The looks of a card: both sizes, a value in the display type, a title alone, and a disabled card (its dimmer inks).
    fn looks() -> Vec<(CardSize, CardModel)> {
        let mut disabled = model();
        disabled.state.enabled = false;
        vec![
            (CardSize::Full, model()),
            (CardSize::Half, model()),
            (
                CardSize::Full,
                CardModel::new("Now playing")
                    .with_value("Paused")
                    .with_icon(Icon::Cloud),
            ),
            (CardSize::Half, CardModel::new("Living room")),
            (CardSize::Full, disabled),
        ]
    }

    fn warm(models: &[(CardSize, CardModel)]) -> Harness {
        warm_glyph_harness(|h| {
            models
                .iter()
                .map(|(size, m)| card(h, *size, m))
                .collect::<Vec<_>>()
        })
    }

    #[test]
    fn a_cards_words_are_the_same_pixels_as_glyph_quads_and_as_a_picture_in_every_look() {
        let all = looks();
        let mut run = warm(&all);
        for (size, model) in &all {
            let mut glyphs = card(&mut run, *size, model);
            assert!(is_run(&mut glyphs), "{size:?} {:?}", model.title);
            let mut pic = harness();
            let picture = card(&mut pic, *size, model);
            let (a, b) = (
                run.layer_pixels(text_id(&glyphs)),
                pic.layer_pixels(text_id(&picture)),
            );
            assert!(a.iter().any(|v| *v != 0), "{size:?} {:?}", model.title);
            assert_eq!(
                a, b,
                "{size:?} {:?} enabled={}",
                model.title, model.state.enabled
            );
        }
    }

    #[test]
    fn on_glyph_text_a_cards_words_make_no_picture_and_write_no_pixel_and_no_layer_more() {
        let creates =
            |sent: &[Request]| count_requests(sent, |r| matches!(r, Request::Create { .. }));
        let all = looks();
        let mut run = warm(&all);
        for (size, model) in &all {
            let mut glyphs = Card::new(&mut run.ui, AT, 10, *size, model).unwrap();
            let sent = run.commit();
            assert!(is_run(&mut glyphs), "{size:?} {:?}", model.title);
            let text = text_id(&glyphs);
            assert!(!made_as_picture(&sent, text), "{:?}", model.title);
            assert_eq!(pixels_damaged_in(&sent, text), 0.0, "{:?}", model.title);
            // The only picture is the corner icon, a layer of its own.
            assert_eq!(
                pictures_made(&sent),
                usize::from(model.icon.is_some()),
                "{:?}: {sent:?}",
                model.title
            );
            assert_eq!(run.ui.glyph_text_fallbacks, 0);
            let mut pic = harness();
            let _picture = Card::new(&mut pic.ui, AT, 10, *size, model).unwrap();
            let as_pictures = pic.commit();
            assert!(creates(&sent) <= creates(&as_pictures));
            assert_eq!(pictures_made(&as_pictures), pictures_made(&sent) + 1);
        }
    }

    #[test]
    fn on_glyph_text_a_new_value_is_one_swap_and_a_disabled_card_is_one_restyle() {
        let now = model();
        let later = model().with_value("15°");
        let mut dimmed = later.clone();
        dimmed.state.enabled = false;
        let mut run = warm(&[
            (CardSize::Full, now.clone()),
            (CardSize::Full, later.clone()),
            (CardSize::Full, dimmed.clone()),
        ]);
        let mut c = card(&mut run, CardSize::Full, &now);
        assert_eq!(c.update(&later, &mut run.ui), Damage::Content);
        let sent = run.commit();
        assert_eq!(
            count_requests(
                &sent,
                |r| matches!(r, Request::SetGlyphs { id, .. } if *id == text_id(&c))
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
        // Disabled: the text takes the dimmer inks with one restyle of the words; the icon, a picture, is drawn again in its own.
        assert_eq!(c.update(&dimmed, &mut run.ui), Damage::Content);
        let sent = run.commit();
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::Restyle { id, content: Content::Glyphs(_) } if *id == text_id(&c)
            )),
            1,
            "{sent:?}"
        );
        assert_eq!(pixels_damaged_in(&sent, text_id(&c)), 0.0);
        assert_eq!(pictures_made(&sent), 0);
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
    }

    #[test]
    fn a_tappable_card_activates_on_release_inside_and_scales_with_its_prepared_shadow() {
        let mut h = harness();
        let mut c = card(&mut h, CardSize::Full, &model());
        assert!(matches!(
            c.surface().pair().press,
            PressTreatment::Scale { .. }
        ));
        let (rest, pressed) = c.surface().shown();
        assert!(
            rest.shadows[0].is_some(),
            "an independent card casts the single raised shadow"
        );
        assert_ne!(
            pressed.unwrap().shadows,
            rest.shadows,
            "and the prepared pressed shadow while pressed"
        );
        assert!(
            rest.depth.is_some_and(|d| !d.inset)
                && pressed.unwrap().depth.is_some_and(|d| !d.inset),
            "the pair stays"
        );
        c.input(&touch(1, Down, 100.0, 120.0, 0), &mut h.ui);
        assert!(c.is_pressed());
        assert!(
            h.commit()
                .iter()
                .any(|r| matches!(r, Request::Animate { .. }))
        );
        assert_eq!(
            c.input(&touch(1, Up, 100.0, 120.0, 40), &mut h.ui),
            Some(CardAction::Activated)
        );
        assert!(!c.is_pressed());
    }

    #[test]
    fn a_drag_out_a_cancel_and_a_card_that_is_not_tappable_activate_nothing() {
        let mut h = harness();
        let mut c = card(&mut h, CardSize::Full, &model());
        c.input(&touch(1, Down, 100.0, 120.0, 0), &mut h.ui);
        c.input(&touch(1, Move, 600.0, 120.0, 16), &mut h.ui);
        assert_eq!(c.input(&touch(1, Up, 600.0, 120.0, 32), &mut h.ui), None);
        c.input(&touch(1, Down, 100.0, 120.0, 100), &mut h.ui);
        assert_eq!(c.input(&touch(1, Cancel, 0.0, 0.0, 110), &mut h.ui), None);
        c.input(&touch(1, Down, 100.0, 120.0, 200), &mut h.ui);
        c.cancel(&h.ui);
        assert_eq!(c.input(&touch(1, Up, 100.0, 120.0, 210), &mut h.ui), None);

        let mut plain = card(
            &mut h,
            CardSize::Full,
            &CardModel::new("Now playing").with_value("Idle"),
        );
        let before = h.all().len();
        assert_eq!(
            plain.input(&touch(1, Down, 100.0, 120.0, 0), &mut h.ui),
            None
        );
        assert_eq!(
            plain.input(&touch(1, Up, 100.0, 120.0, 30), &mut h.ui),
            None
        );
        assert_eq!(h.all().len(), before);
        // It has the same glass as an actionable one: actionability changes behaviour, not the material.
        assert_eq!(plain.surface().pair().rest, c.surface().pair().rest);
    }

    #[test]
    fn all_of_a_cards_text_is_one_layer_drawn_once_and_again_only_when_the_words_change() {
        let mut h = harness();
        let mut c = card(&mut h, CardSize::Full, &model());
        let images = h
            .layers()
            .iter()
            .filter(|(_, c, _)| matches!(c, Content::Image))
            .count();
        assert_eq!(images, 2, "one text layer and one icon");
        let same = model();
        assert_eq!(c.update(&same, &mut h.ui), Damage::None);
        let mut pressed_only = model();
        pressed_only.state.selected = true;
        assert_eq!(
            c.update(&pressed_only, &mut h.ui),
            Damage::Properties,
            "selection restyles and draws nothing"
        );
        let sent = h.commit();
        assert!(
            !sent.iter().any(|r| matches!(r, Request::Damage { .. })),
            "{sent:?}"
        );
        let words = model().with_value("15°");
        assert_eq!(c.update(&words, &mut h.ui), Damage::Content);
        assert_eq!(c.lines().1, Some("15°"));
    }

    #[test]
    fn a_half_card_keeps_fewer_lines_the_last_ends_in_an_ellipsis_and_large_text_asks_for_more_height()
     {
        let mut h = harness();
        let m = model();
        let full = Card::new(&mut h.ui, AT, 10, CardSize::Full, &m).unwrap();
        let half = Card::new(&mut h.ui, AT, 10, CardSize::Half, &m).unwrap();
        assert!(half.lines().2.len() <= 2 && full.lines().2.len() <= 3);
        assert!(half.lines().2.len() <= full.lines().2.len());
        let long = CardModel::new("Reminders").with_body("A body of words long enough that it has to be cut off somewhere along the way, at its end");
        let cut = Card::new(&mut h.ui, AT, 10, CardSize::Half, &long).unwrap();
        assert!(
            cut.lines().2.last().is_some_and(|l| l.ends_with('…')),
            "{:?}",
            cut.lines().2
        );
        let normal = Card::min_height(&mut h.ui, CardSize::Full, &m, 280.0);
        h.ui.look.text_scale = 2.0;
        let large = Card::min_height(&mut h.ui, CardSize::Full, &m, 280.0);
        assert!(large > normal * 1.4, "{normal} -> {large}");
        h.commit();
    }

    #[test]
    fn a_disabled_card_takes_no_touch_and_keeps_its_text_readable() {
        let mut h = harness();
        let mut m = model();
        m.state.enabled = false;
        let mut c = card(&mut h, CardSize::Full, &m);
        assert_eq!(c.input(&touch(1, Down, 100.0, 120.0, 0), &mut h.ui), None);
        assert_eq!(c.input(&touch(1, Up, 100.0, 120.0, 20), &mut h.ui), None);
        let mut sink = SemanticsSink::new();
        c.semantics(&mut sink);
        let node = sink.find("Weather").unwrap();
        assert!(!node.enabled && !node.actionable);
        assert_eq!(
            node.value.as_deref(),
            Some("14°, Light rain until 15:00, then clearing")
        );
    }

    #[test]
    fn a_card_is_raised_glass_with_the_cached_backdrop_in_dark_and_light() {
        for mode in [Mode::Dark, Mode::Light] {
            let mut h = harness();
            h.ui.look = crate::context::Look::standard(mode);
            let c = card(&mut h, CardSize::Full, &model());
            let (rest, _) = c.surface().shown();
            assert_eq!(rest.corner_radius, 24.0);
            assert!(
                rest.sheen.is_some() && rest.depth.is_some_and(|d| !d.inset),
                "{mode:?}"
            );
            assert!(
                matches!(rest.backdrop, compositor_protocol::Backdrop::Cached { .. }),
                "{mode:?}"
            );
            assert_eq!(
                rest.shadows[1], None,
                "{mode:?}: one cast shadow, never the far lobe"
            );
        }
    }
}
