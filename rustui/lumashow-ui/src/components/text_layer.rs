//! A layer of text: several lines, in as many as four colours, and the marks beside them (an icon, a hairline), as ONE compositor
//! layer. It is what a row, a card, a toast, a message or a page of keys keeps its words in.
//!
//! The layer is one of two things, and what is drawn is the same in both. With `Ui::enable_glyph_text` it is a **run** of glyph
//! quads of an atlas, each in an ink of the run's palette ([`GlyphRun`]): no texture, no pixel written, and a change of words is
//! a swap of quads. Otherwise (or when the glyphs it needs cannot be shown from the atlas at once) it is a **picture** the words
//! and marks are blended into. A run shows what it is given from the atlas it was made on, whatever atlas new layers are made on
//! by then; one that cannot show a change at all (that atlas is full, a text too long for a run, more than four colours, a
//! hairline wider than the atlas) becomes a picture of the new words in the same compositor layer (same id, same group, same
//! place) and stays one, counted in `Ui::glyph_text_fallbacks`: a picture is always right, and the next look change builds the
//! screen again.
//!
//! What a layer shows is a list of [`Item`]s in the order they are painted. Items that overlap (a count on a disc) must be in the
//! order of their colours' first appearance, since a run draws its inks in the palette's order; items that do not overlap may
//! come in any order.

use super::mark::{Form, Mark};
use super::text::{Run, aligned, line_height};
use crate::atlas::{AtlasView, GlyphRun, Palette, PlacedGlyph};
use crate::context::Ui;
use crate::error::UiError;
use crate::geometry::Frame;
use crate::scene::{ImageLayer, Layer};
use crate::text::{Canvas, Type};
use compositor_protocol::resources::{INKS, MAX_RUN_GLYPHS};
use compositor_protocol::{Ease, LayerId, Props, Rgba};
use std::io;

/// One thing a text layer shows.
#[derive(Clone, Copy, Debug)]
pub enum Item<'a> {
    /// A piece of text, placed as [`Run`] says.
    Text(Run<'a>),
    /// A shape.
    Mark(Mark),
}

impl<'a> From<Run<'a>> for Item<'a> {
    fn from(run: Run<'a>) -> Self {
        Item::Text(run)
    }
}

impl From<Mark> for Item<'_> {
    fn from(mark: Mark) -> Self {
        Item::Mark(mark)
    }
}

impl Item<'_> {
    /// The most quads the item takes in a run, which is what a run's ceiling ([`MAX_RUN_GLYPHS`]) is counted in: a mark is one, a
    /// text is one for each character that is not blank (a space is placed and never drawn; a character the font has no shape for
    /// takes none, so a run never takes more than this says).
    pub fn quads(&self) -> usize {
        match self {
            Item::Mark(_) => 1,
            Item::Text(run) => run.text.chars().filter(|c| !c.is_whitespace()).count(),
        }
    }
}

const CLEAR: Rgba = Rgba(0, 0, 0, 0);

/// A compositor layer of text and marks, a run of glyph quads or a picture.
pub struct TextLayer {
    face: Face,
}

enum Face {
    Image(ImageLayer),
    /// The run and the palette it has now.
    Run(GlyphRun, Palette),
}

impl TextLayer {
    /// A layer of `at`'s size at its place, showing `items`. A run of glyph quads when the UI has an atlas and the items can be
    /// shown from it at once; a picture otherwise.
    pub fn new(ui: &mut Ui, at: Frame, z: i32, items: &[Item]) -> io::Result<Self> {
        Ok(Self::made(ui, at, z, items, false)?.expect("a layer that does not wait is made"))
    }

    /// [`TextLayer::new`] for a layer that is made ahead of being shown, and can wait a few frames for its words: when the only thing
    /// that keeps `items` from being a run is that glyphs the atlas has prepared have not been handed over to the compositor yet (the
    /// bands of a mount, or of a refresh, are limited to the slots it has free), nothing is made and the answer is `None`. The glyphs
    /// are on their way, so the same call a refresh or two later finds them handed over, and the layer is a run where `new` would have
    /// made a picture, a texture and the pixels of its whole frame, for good. Anything that cannot be mended by waiting (the page is
    /// full, more than four colours, too many quads, no atlas) is a picture, as for `new`.
    pub fn new_when_ready(
        ui: &mut Ui,
        at: Frame,
        z: i32,
        items: &[Item],
    ) -> io::Result<Option<Self>> {
        Self::made(ui, at, z, items, true)
    }

    fn made(
        ui: &mut Ui,
        at: Frame,
        z: i32,
        items: &[Item],
        wait: bool,
    ) -> io::Result<Option<Self>> {
        let (w, h) = (at.w.ceil() as u32, at.h.ceil() as u32);
        let page = ui.text_atlas.as_ref().map(|atlas| atlas.view());
        if let Some(page) = page.filter(|_| w > 0 && h > 0) {
            if let Some((palette, placed)) = place(ui, &page, items) {
                let run = ui
                    .scene
                    .glyph_run(
                        ui.text_atlas.as_ref().expect("checked"),
                        w,
                        h,
                        z,
                        palette,
                        Props::at(at.x, at.y),
                    )
                    .ok();
                if let Some(mut run) = run {
                    // A run that shows nothing yet sends no list at all.
                    if placed.is_empty() {
                        return Ok(Some(Self {
                            face: Face::Run(run, palette),
                        }));
                    }
                    match run.set_glyphs(&placed) {
                        Ok(()) => {
                            return Ok(Some(Self {
                                face: Face::Run(run, palette),
                            }));
                        }
                        // Made and dropped before any commit: nothing of it is ever sent.
                        Err(UiError::GlyphNotUploaded) if wait => return Ok(None),
                        Err(_) => {}
                    }
                }
            }
        }
        if ui.text_atlas.is_some() {
            ui.text_pictures += 1;
        }
        let mut layer = ui.scene.image(w, h, z, Props::at(at.x, at.y))?;
        paint(ui, &mut layer, items);
        Ok(Some(Self {
            face: Face::Image(layer),
        }))
    }

    /// Shows `items` in place of what the layer showed. A picture is drawn again. A run shows them from the atlas it draws from;
    /// one that cannot (see the module's account) becomes a picture of them, in the same compositor layer, and the UI counts it
    /// (`Ui::glyph_text_fallbacks`): what the layer shows is always `items`.
    pub fn draw(&mut self, ui: &mut Ui, items: &[Item]) {
        if let Face::Run(run, shown) = &mut self.face {
            if show(ui, run, shown, items) {
                return;
            }
            match run.become_picture() {
                Ok(picture) => {
                    // The run is dropped here: it lets go of its atlas, and with it a list that was waiting for glyphs.
                    self.face = Face::Image(picture);
                    ui.glyph_text_fallbacks += 1;
                }
                // No file for the pixels: the run keeps what it shows, and the scene is told that what it shows is not whole.
                Err(_) => {
                    ui.scene.note_refusal();
                    return;
                }
            }
        }
        if let Face::Image(layer) = &mut self.face {
            paint(ui, layer, items);
        }
    }

    /// [`TextLayer::draw`] of lines of text and nothing else (what `draw_runs` is for a picture).
    pub fn draw_runs(&mut self, ui: &mut Ui, runs: &[Run]) {
        let items: Vec<Item> = runs.iter().copied().map(Item::Text).collect();
        self.draw(ui, &items);
    }

    /// Whether the text is glyph quads of the shared atlas and not a picture.
    pub fn is_glyph_run(&self) -> bool {
        matches!(self.face, Face::Run(..))
    }

    /// The compositor layer, to hang it under a native group.
    pub fn layer(&self) -> &Layer {
        match &self.face {
            Face::Image(layer) => layer,
            Face::Run(run, _) => run,
        }
    }

    pub fn id(&self) -> LayerId {
        self.layer().id()
    }

    /// Moves, hides or clips the layer at once, cancelling any animation on it.
    pub fn set_props(&mut self, props: Props) {
        self.layer().set(props);
    }

    /// Animates the layer's position, opacity and scale, exactly like a plain layer.
    pub fn animate_props(&mut self, to: Props, duration_ms: u32, ease: Ease) {
        self.layer().animate(to, duration_ms, ease);
    }
}

/// Clears `layer` and blends `items` into it, in order, then tells the compositor the whole layer changed.
fn paint(ui: &mut Ui, layer: &mut ImageLayer, items: &[Item]) {
    layer.clear();
    let width = layer.width() as usize;
    for item in items {
        match item {
            Item::Text(run) => {
                let extent = ui.fonts.measure(run.text, run.ty);
                let x = aligned(run.x, run.w, extent.width, run.align);
                let baseline =
                    run.y + ((line_height(run.ty) - extent.height()) / 2.0 + extent.ascent).round();
                ui.fonts.draw(
                    &mut Canvas {
                        pixels: layer.pixels_mut(),
                        width,
                    },
                    run.text,
                    run.ty,
                    (x, baseline),
                    run.colour,
                );
            }
            Item::Mark(mark) => mark.paint(layer.pixels_mut(), width),
        }
    }
    layer.damage(None);
}

/// Shows `items` in `run`, from the atlas it draws from: its palette if that differs, and its quads. `false` when it cannot.
fn show(ui: &mut Ui, run: &mut GlyphRun, shown: &mut Palette, items: &[Item]) -> bool {
    let atlas = run.atlas();
    let Some((palette, placed)) = place(ui, &atlas, items) else {
        return false;
    };
    if palette != *shown {
        if run.set_palette(palette).is_err() {
            return false;
        }
        *shown = palette;
    }
    run.set_glyphs_when_ready(&placed).is_ok()
}

/// Makes `text` at `ty` ready in `atlas`. `false` when it cannot be; when that is for want of room on the page new text is made on
/// (`Ui::text_atlas`), `Ui::text_atlas_full` says so (the page of an older run being full is no news).
pub(super) fn prepare_text(ui: &mut Ui, atlas: &AtlasView, ty: Type, text: &str) -> bool {
    let made = atlas.prepare(&mut ui.fonts, ty, text);
    made_or_full(ui, atlas, made)
}

/// Whether a prepare of text or a mark worked. One that did not (the page has no room, a glyph is too large) has still made ready
/// and queued what it could place before it stopped, and the caller makes no layer and so pumps nothing: the bands it left are
/// handed over here, or they wait for a slot's acknowledgement that may never come (no upload in flight, nothing to answer).
fn made_or_full(ui: &mut Ui, atlas: &AtlasView, made: Result<(), UiError>) -> bool {
    match made {
        Ok(()) => true,
        Err(error) => {
            atlas.pump();
            if matches!(error, UiError::AtlasFull)
                && ui
                    .text_atlas
                    .as_ref()
                    .is_some_and(|page| page.id() == atlas.id())
            {
                ui.text_atlas_full = true;
            }
            false
        }
    }
}

/// Makes the shape of `mark` ready in `atlas` (the same as [`prepare_text`] is for words). `false` when it cannot be: the page has
/// no room, or a hairline is wider than the page.
fn prepare_mark(ui: &mut Ui, atlas: &AtlasView, mark: &Mark) -> bool {
    let form = mark.form;
    if matches!(form, Form::Rule(w) if w > atlas.widest()) {
        return false;
    }
    let made = atlas.prepare_sprite(form.sprite(), |strip_w| match form {
        Form::Rule(_) => (strip_w, 1, vec![255; strip_w as usize]),
        _ => {
            let (w, h) = form.size();
            (w, h, form.coverage())
        }
    });
    made_or_full(ui, atlas, made)
}

/// The palette and the quads that show `items` as a run of `atlas`: the glyphs and marks are made ready in it and handed over, each
/// colour is an ink in the order of its first appearance. `None` when they cannot be shown as a run (more than four colours, too
/// many quads, an atlas that is full, a hairline wider than the atlas).
fn place(ui: &mut Ui, atlas: &AtlasView, items: &[Item]) -> Option<(Palette, Vec<PlacedGlyph>)> {
    // Everything is made ready before anything is laid out: a layout names glyphs that must be there.
    for item in items {
        match item {
            Item::Text(run) => {
                if run.text.chars().count() > MAX_RUN_GLYPHS {
                    return None;
                }
                if !prepare_text(ui, atlas, run.ty, run.text) {
                    return None;
                }
            }
            Item::Mark(mark) => {
                if !prepare_mark(ui, atlas, mark) {
                    return None;
                }
            }
        }
    }
    atlas.pump();
    let mut palette = [CLEAR; INKS];
    let mut inks = 0;
    let mut ink_of = |colour: Rgba| -> Option<u8> {
        if let Some(i) = palette[..inks].iter().position(|c| *c == colour) {
            return Some(i as u8);
        }
        if inks == INKS {
            return None;
        }
        palette[inks] = colour;
        inks += 1;
        Some((inks - 1) as u8)
    };
    let mut placed = Vec::new();
    for item in items {
        match item {
            Item::Text(run) => {
                let ink = ink_of(run.colour)?;
                let extent = ui.fonts.measure(run.text, run.ty);
                let x = aligned(run.x, run.w, extent.width, run.align);
                let baseline =
                    run.y + ((line_height(run.ty) - extent.height()) / 2.0 + extent.ascent).round();
                atlas
                    .layout(run.text, run.ty, (x, baseline), ink, &mut placed)
                    .ok()?;
            }
            Item::Mark(mark) => {
                let ink = ink_of(mark.colour)?;
                let crop = match mark.form {
                    Form::Rule(w) => w as u16,
                    _ => 0,
                };
                atlas
                    .layout_sprite(mark.form.sprite(), (mark.x, mark.y), crop, ink, &mut placed)
                    .ok()?;
            }
        }
    }
    (placed.len() <= MAX_RUN_GLYPHS).then_some((palette, placed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::icon::Icon;
    use crate::group::GroupPose;
    use crate::testing::{Harness, harness};
    use crate::text::{Type, Weight};
    use compositor_protocol::{Content, Request};

    const BODY: Type = Type {
        px: 16,
        weight: Weight::Regular,
    };
    const CAPTION: Type = Type {
        px: 13,
        weight: Weight::Regular,
    };
    const INK: Rgba = Rgba(236, 236, 240, 255);
    const INK2: Rgba = Rgba(150, 150, 165, 255);
    const RULE: Rgba = Rgba(30, 30, 33, 51);
    const ACCENT: Rgba = Rgba(120, 90, 220, 255);

    fn count(sent: &[Request], f: impl Fn(&Request) -> bool) -> usize {
        sent.iter().filter(|r| f(r)).count()
    }

    fn pictures(sent: &[Request]) -> usize {
        count(sent, |r| {
            matches!(
                r,
                Request::Create {
                    content: Content::Image,
                    ..
                }
            )
        })
    }

    fn pixels_written(sent: &[Request]) -> f32 {
        sent.iter()
            .map(|r| match r {
                Request::Damage { rect, .. } => rect.w * rect.h,
                _ => 0.0,
            })
            .sum()
    }

    fn quads(sent: &[Request]) -> Vec<compositor_protocol::Glyph> {
        sent.iter()
            .filter_map(|r| match r {
                Request::SetGlyphs { glyphs, .. } => Some(glyphs.clone()),
                _ => None,
            })
            .next_back()
            .unwrap_or_default()
    }

    /// A row's worth: a title and a detail line, an icon before them, a chevron after, a hairline under them.
    fn row_items<'a>(title: &'a str, detail: &'a str) -> Vec<Item<'a>> {
        vec![
            Run::left(title, BODY, INK, 36.0, 8.0, 220.0).into(),
            Run::left(detail, CAPTION, INK2, 36.0, 30.0, 220.0).into(),
            Mark::icon(Icon::Alarm, 28, 0, 18, INK).into(),
            Mark::icon(Icon::ChevronRight, 24, 276, 20, INK2).into(),
            Mark::rule(0, 63, 300, RULE).into(),
        ]
    }

    /// A glyph-text UI whose atlas has seen `items` and been told it landed, as one that showed the page before is.
    fn primed(items: &[Item]) -> Harness {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let first = TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, items).unwrap();
        // Whatever it could not show at once is a picture; settle the atlas and make it again from a warm one.
        for _ in 0..8 {
            for request in h.commit() {
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
        }
        drop(first);
        h.commit();
        h
    }

    #[test]
    fn a_text_layer_is_a_picture_until_glyph_text_is_switched_on() {
        let mut h = harness();
        let items = row_items("Alarm", "Mon to Fri");
        let layer =
            TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &items).unwrap();
        assert!(!layer.is_glyph_run());
        let sent = h.commit();
        assert_eq!(pictures(&sent), 1);
        assert!(pixels_written(&sent) > 0.0);
        assert!(h.pixels(layer.id()).iter().any(|b| *b != 0));
    }

    #[test]
    fn on_glyph_text_a_row_of_words_icons_and_a_hairline_is_one_run_that_writes_no_pixel() {
        let items = row_items("Alarm", "Mon to Fri");
        let mut h = primed(&items);
        let layer =
            TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &items).unwrap();
        assert!(layer.is_glyph_run());
        let sent = h.commit();
        assert_eq!(
            count(&sent, |r| matches!(r, Request::Create { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(pictures(&sent), 0);
        assert_eq!(pixels_written(&sent), 0.0);
        assert_eq!(
            count(&sent, |r| matches!(r, Request::AtlasUpload { .. })),
            0
        );
        // The quads are in ink order: the title's (ink 0), the detail's (1), the two icons' (0 and 1 again: the colours are the
        // title's and the detail's), and the hairline's (2).
        let inks: Vec<u8> = quads(&sent).iter().map(|q| q.ink).collect();
        assert!(inks.windows(2).all(|w| w[0] <= w[1]), "{inks:?}");
        assert_eq!(inks.iter().copied().max(), Some(2));
        let (_, content, _) = h.layers().into_iter().last().unwrap();
        let Content::Glyphs(style) = content else {
            panic!("{content:?}")
        };
        assert_eq!(&style.palette[..3], &[INK, INK2, RULE]);
    }

    /// A layer made ahead of being shown can wait the frame or two its glyphs need to be handed over: it is a run, where a layer made
    /// the same moment with `new` is a picture of the whole frame, for good. What waiting cannot mend is a picture at once.
    #[test]
    fn a_layer_made_ahead_waits_for_glyphs_that_are_on_their_way_where_one_made_now_is_a_picture() {
        let items = row_items("Alarm", "Mon to Fri");
        let at = Frame::new(0.0, 0.0, 300.0, 64.0);
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        // Nothing of the words has been handed over: it waits, and makes nothing.
        assert!(
            TextLayer::new_when_ready(&mut h.ui, at, 1, &items)
                .unwrap()
                .is_none()
        );
        assert_eq!(h.ui.text_pictures, 0, "a layer that waits is not a picture");
        let sent = h.commit();
        assert_eq!(
            count(&sent, |r| matches!(r, Request::Create { .. })),
            0,
            "and no layer was made: {sent:?}"
        );
        assert!(
            count(&sent, |r| matches!(r, Request::AtlasUpload { .. })) > 0,
            "the glyphs are on their way"
        );
        // The compositor lands them: the same call is a run, which writes no pixel.
        h.settle_atlas();
        let layer = TextLayer::new_when_ready(&mut h.ui, at, 1, &items)
            .unwrap()
            .expect("the glyphs have landed");
        assert!(layer.is_glyph_run());
        let sent = h.commit();
        assert_eq!((pictures(&sent), pixels_written(&sent)), (0, 0.0));
        assert_eq!(h.ui.text_pictures, 0);

        // Five colours cannot be a run however long it waits: a picture at once, as `new` makes it.
        let five = five_colours();
        let picture = TextLayer::new_when_ready(&mut h.ui, at, 2, &five)
            .unwrap()
            .expect("a picture, at once");
        assert!(!picture.is_glyph_run());
        assert_eq!(h.ui.text_pictures, 1);
    }

    #[test]
    fn what_a_run_blends_to_is_what_the_picture_is_pixel_for_pixel() {
        let items = row_items("Alarm", "Mon to Fri");
        let mut run = primed(&items);
        let glyphs =
            TextLayer::new(&mut run.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &items).unwrap();
        assert!(glyphs.is_glyph_run());
        run.commit();
        let mut pic = harness();
        let picture =
            TextLayer::new(&mut pic.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &items).unwrap();
        pic.commit();
        let (a, b) = (
            run.layer_pixels(glyphs.id()),
            pic.layer_pixels(picture.id()),
        );
        assert_eq!(a.len(), b.len());
        assert!(
            a.iter().any(|v| *v != 0),
            "the comparison is not of two clear layers"
        );
        let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        assert_eq!(worst, 0, "the run and the picture differ by {worst}");
    }

    #[test]
    fn marks_that_overlap_keep_the_order_of_their_colours_first_appearance() {
        // A count on a disc: the disc's colour comes first, so it is the first ink and the count is drawn over it.
        let digit = Run::centre("3", CAPTION, INK, 0.0, 0.0, 24.0);
        let items = [Mark::disc(0, 0, 24, ACCENT).into(), Item::Text(digit)];
        let mut run = primed(&items);
        let glyphs =
            TextLayer::new(&mut run.ui, Frame::new(0.0, 0.0, 24.0, 24.0), 1, &items).unwrap();
        assert!(glyphs.is_glyph_run());
        run.commit();
        let mut pic = harness();
        let picture =
            TextLayer::new(&mut pic.ui, Frame::new(0.0, 0.0, 24.0, 24.0), 1, &items).unwrap();
        pic.commit();
        assert_eq!(
            run.layer_pixels(glyphs.id()),
            pic.layer_pixels(picture.id())
        );
    }

    #[test]
    fn a_change_of_words_is_one_swap_and_a_change_of_colours_is_one_restyle_and_neither_writes_a_pixel()
     {
        let items = row_items("Alarm", "Mon to Fri");
        let mut h = primed(&items);
        let mut layer =
            TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &items).unwrap();
        h.commit();
        layer.draw(&mut h.ui, &row_items("Alarms", "Mon to Fri"));
        let sent = h.commit();
        assert_eq!(
            count(&sent, |r| matches!(r, Request::SetGlyphs { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(count(&sent, |r| matches!(r, Request::Restyle { .. })), 0);
        assert_eq!(pixels_written(&sent) + pictures(&sent) as f32, 0.0);
        // The same words in a selected row's accent: the palette changes, and the quads are sent again (the hairline and the
        // icons move to other inks).
        let mut accent = row_items("Alarm", "Mon to Fri");
        accent[0] = Run::left("Alarm", BODY, ACCENT, 36.0, 8.0, 220.0).into();
        layer.draw(&mut h.ui, &accent);
        let sent = h.commit();
        assert_eq!(
            count(&sent, |r| matches!(r, Request::Restyle { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(pixels_written(&sent), 0.0);
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
    }

    /// A glyph UI whose page has seen `first`, and a run of it in `frame`, as a screen that is up has: committed, its glyphs landed.
    fn up(frame: Frame, first: &[Item]) -> (Harness, TextLayer) {
        let mut h = primed(first);
        let layer = TextLayer::new(&mut h.ui, frame, 1, first).unwrap();
        assert!(layer.is_glyph_run());
        h.commit();
        (h, layer)
    }

    /// Tells a run to show `then` and checks what became of it: a picture of `then` in the same compositor layer (the same id, one
    /// `Create` that replaces the layer, nothing that is about quads or a palette, no `Delete`), the very pixels a picture made
    /// of `then` has, one fallback counted, and no layer more. Returns what the commit sent.
    fn becomes_a_picture_of(
        h: &mut Harness,
        layer: &mut TextLayer,
        frame: Frame,
        then: &[Item],
    ) -> Vec<Request> {
        let (id, layers, before) = (
            layer.id(),
            h.ui.scene.live_layers(),
            h.ui.glyph_text_fallbacks,
        );
        layer.draw(&mut h.ui, then);
        assert!(!layer.is_glyph_run());
        assert_eq!(layer.id(), id, "the same compositor layer");
        assert_eq!(h.ui.scene.live_layers(), layers, "replaced, not added");
        assert_eq!(h.ui.glyph_text_fallbacks, before + 1);
        let sent = h.commit();
        let creates: Vec<&Request> = sent
            .iter()
            .filter(|r| matches!(r, Request::Create { .. }))
            .collect();
        assert_eq!(creates.len(), 1, "{sent:?}");
        assert!(
            matches!(creates[0], Request::Create { id: made, content: Content::Image, .. } if *made == id),
            "{sent:?}"
        );
        assert!(
            sent.iter().all(|r| !matches!(
                r,
                Request::SetGlyphs { id: of, .. }
                    | Request::Restyle { id: of, .. }
                    | Request::Delete { id: of } if *of == id
            )),
            "{sent:?}"
        );
        let mut pic = harness();
        let reference = TextLayer::new(&mut pic.ui, frame, 1, then).unwrap();
        pic.commit();
        let (shown, wanted) = (h.layer_pixels(id), pic.layer_pixels(reference.id()));
        assert!(
            shown.iter().any(|v| *v != 0),
            "the comparison is not of two clear layers"
        );
        assert_eq!(
            shown, wanted,
            "what the layer shows is a picture of the words"
        );
        sent
    }

    fn five_colours<'a>() -> [Item<'a>; 5] {
        [
            Run::left("a", BODY, Rgba(1, 1, 1, 255), 0.0, 0.0, 50.0).into(),
            Run::left("b", BODY, Rgba(2, 2, 2, 255), 10.0, 0.0, 50.0).into(),
            Run::left("c", BODY, Rgba(3, 3, 3, 255), 20.0, 0.0, 50.0).into(),
            Run::left("d", BODY, Rgba(4, 4, 4, 255), 30.0, 0.0, 50.0).into(),
            Run::left("e", BODY, Rgba(5, 5, 5, 255), 40.0, 0.0, 50.0).into(),
        ]
    }

    #[test]
    fn a_run_told_to_show_five_colours_becomes_a_picture_of_them_in_the_same_layer() {
        let frame = Frame::new(0.0, 0.0, 100.0, 24.0);
        let five = five_colours();
        // Made with five colours, it is a picture from the start (and is not a fallback: it was never a run).
        let mut h = primed(&five);
        let pictures_before = h.ui.text_pictures;
        let picture = TextLayer::new(&mut h.ui, frame, 1, &five).unwrap();
        assert!(!picture.is_glyph_run());
        assert_eq!(
            (h.ui.glyph_text_fallbacks, h.ui.text_pictures),
            (0, pictures_before + 1)
        );
        // Made with one, it is a run; told to show five, it is a picture of them.
        let one = [Run::left("a", BODY, INK, 0.0, 0.0, 50.0).into()];
        let (mut h, mut run) = up(frame, &one);
        becomes_a_picture_of(&mut h, &mut run, frame, &five);
        // And stays one: the words change and the pixels are written again, in the same layer, with no list of quads.
        let id = run.id();
        let again = [Run::left("e", BODY, INK, 0.0, 0.0, 50.0).into()];
        run.draw(&mut h.ui, &again);
        let sent = h.commit();
        assert!(!run.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 1, "a picture is not a fallback");
        assert!(
            sent.iter().all(|r| !matches!(
                r,
                Request::Create { .. } | Request::SetGlyphs { .. } | Request::Restyle { .. }
            )),
            "{sent:?}"
        );
        assert!(
            sent.iter()
                .any(|r| matches!(r, Request::Damage { id: of, .. } if *of == id))
        );
    }

    #[test]
    fn a_run_told_to_show_more_quads_than_a_run_has_becomes_a_picture_of_them_in_the_same_layer() {
        // Two lines of 150 glyphs: each is a run's worth, together they are not.
        let frame = Frame::new(0.0, 0.0, 700.0, 48.0);
        let line = "i".repeat(150);
        let many = [
            Run::left(&line, CAPTION, INK, 0.0, 0.0, 600.0).into(),
            Run::left(&line, CAPTION, INK, 0.0, 24.0, 600.0).into(),
        ];
        let one = [Run::left("i", CAPTION, INK, 0.0, 0.0, 50.0).into()];
        let (mut h, mut run) = up(frame, &one);
        becomes_a_picture_of(&mut h, &mut run, frame, &many);
    }

    #[test]
    fn a_run_told_to_show_a_text_longer_than_a_run_becomes_a_picture_of_it_in_the_same_layer() {
        let frame = Frame::new(0.0, 0.0, 300.0, 24.0);
        let long = "i".repeat(300);
        let items = [Run::left(&long, CAPTION, INK, 0.0, 0.0, 280.0).into()];
        let one = [Run::left("i", CAPTION, INK, 0.0, 0.0, 50.0).into()];
        let (mut h, mut run) = up(frame, &one);
        becomes_a_picture_of(&mut h, &mut run, frame, &items);
    }

    #[test]
    fn a_run_told_to_show_a_hairline_wider_than_the_atlas_becomes_a_picture_of_it_in_the_same_layer()
     {
        let frame = Frame::new(0.0, 0.0, 2000.0, 4.0);
        let wide = [Mark::rule(0, 0, 2000, RULE).into()];
        let one = [Run::left("a", BODY, INK, 0.0, 0.0, 50.0).into()];
        let (mut h, mut run) = up(frame, &one);
        becomes_a_picture_of(&mut h, &mut run, frame, &wide);
    }

    #[test]
    fn a_run_whose_page_is_full_becomes_a_picture_of_its_new_words_in_the_same_layer() {
        let frame = Frame::new(0.0, 0.0, 300.0, 64.0);
        let first = row_items("Alarm", "Mon to Fri");
        let (mut h, mut run) = up(frame, &first);
        let c = h.fill_text_page(BODY);
        assert!(!h.ui.text_atlas_full, "nothing asked the page for room yet");
        // Words made of glyphs the page has are shown from it still; the page being full is no reason to leave it.
        let known = row_items("Alarm", "Mon to");
        run.draw(&mut h.ui, &known);
        assert!(run.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
        let text = format!("Alarm {c}");
        let new = row_items(&text, "Mon to Fri");
        becomes_a_picture_of(&mut h, &mut run, frame, &new);
        assert!(h.ui.text_atlas_full, "the page said it had no room");
    }

    #[test]
    fn a_run_goes_on_drawing_from_its_own_page_when_new_layers_are_made_on_another() {
        let frame = Frame::new(0.0, 0.0, 300.0, 64.0);
        let (mut h, mut old) = up(frame, &row_items("Alarm", "Mon to Fri"));
        let page = h.ui.text_atlas.as_ref().unwrap().view();
        // The page new text is made on is replaced; the run keeps the one it was made on.
        h.ui.text_atlas = Some(h.ui.scene.atlas(1024, 1024).unwrap());
        h.ui.warm(&[BODY, CAPTION]);
        h.settle_atlas();
        let fresh = h.ui.text_atlas.as_ref().unwrap().view();
        assert!(page.glyph(BODY, 'é').is_none() && fresh.glyph(BODY, 'é').is_none());
        // Words with a glyph neither page has, and the icons and the hairline only its own page has made ready: all laid out
        // against the page the run draws from.
        let words = row_items("Café", "Mon to Fri");
        old.draw(&mut h.ui, &words);
        h.settle_atlas();
        assert!(old.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
        assert!(page.glyph(BODY, 'é').is_some());
        assert!(fresh.glyph(BODY, 'é').is_none());
        let mut pic = harness();
        let picture = TextLayer::new(&mut pic.ui, frame, 1, &words).unwrap();
        assert_eq!(
            h.layer_pixels(old.id()),
            pic.layer_pixels(picture.id()),
            "the words of the page the run draws from, not another page's glyphs"
        );
    }

    #[test]
    fn a_converted_layer_is_made_again_where_it_was_last_put() {
        let frame = Frame::new(10.0, 20.0, 100.0, 24.0);
        let one = [Run::left("a", BODY, INK, 0.0, 0.0, 50.0).into()];
        let (mut h, mut layer) = up(frame, &one);
        let moved = Props {
            opacity: 0.5,
            ..Props::at(77.0, 88.0)
        };
        layer.set_props(moved);
        let sent = becomes_a_picture_of(&mut h, &mut layer, frame, &five_colours());
        let made = sent.iter().find_map(|r| match r {
            Request::Create { props, .. } => Some(*props),
            _ => None,
        });
        assert_eq!(made, Some(moved), "{sent:?}");
    }

    #[test]
    fn a_converted_layer_is_still_under_its_group_with_the_place_it_had_in_it() {
        let frame = Frame::new(10.0, 20.0, 100.0, 24.0);
        let one = [Run::left("a", BODY, INK, 0.0, 0.0, 50.0).into()];
        let (mut h, mut layer) = up(frame, &one);
        let group = h.ui.scene.group(None, GroupPose::at(5.0, 6.0)).unwrap();
        group.attach(layer.layer(), Props::at(30.0, 40.0)).unwrap();
        // Sent somewhere in the group's space: where the animation ends is the place it has.
        let end = Props {
            opacity: 0.5,
            ..Props::at(32.0, 42.0)
        };
        layer.animate_props(end, 200, Ease::Out);
        h.commit();
        let sent = becomes_a_picture_of(&mut h, &mut layer, frame, &five_colours());
        let at = |f: fn(&Request) -> bool| sent.iter().position(f).expect("sent");
        let (create, attach) = (
            at(|r| matches!(r, Request::Create { .. })),
            at(|r| matches!(r, Request::Attach { .. })),
        );
        assert!(
            create < attach,
            "the group is given back after the layer is made again"
        );
        assert_eq!(
            sent[attach],
            Request::Attach {
                id: layer.id(),
                group: Some(group.wire_id()),
                local: end
            }
        );
        assert_eq!(layer.layer().group(), Some(group.id()));
        // The layer holds its group as before: the group goes when the layer does.
        assert_eq!(h.ui.scene.groups_alive(), 1);
        drop(group);
        assert_eq!(h.ui.scene.groups_alive(), 1, "the layer still hangs in it");
        drop(layer);
        h.commit();
        assert_eq!(h.ui.scene.groups_alive(), 0);
    }

    #[test]
    fn a_text_too_long_for_a_run_or_a_hairline_wider_than_the_atlas_is_a_picture() {
        let long = "a".repeat(300);
        let items = [Run::left(&long, BODY, INK, 0.0, 0.0, 200.0).into()];
        let mut h = primed(&[]);
        let layer =
            TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 200.0, 24.0), 1, &items).unwrap();
        assert!(!layer.is_glyph_run());
        let wide = [Mark::rule(0, 0, 2000, RULE).into()];
        let layer = TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 2000.0, 4.0), 1, &wide).unwrap();
        assert!(!layer.is_glyph_run());
    }

    #[test]
    fn a_layer_with_nothing_to_show_yet_is_a_run_that_sends_no_list() {
        let mut h = primed(&[]);
        let layer = TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &[]).unwrap();
        assert!(layer.is_glyph_run());
        let sent = h.commit();
        assert_eq!(count(&sent, |r| matches!(r, Request::Create { .. })), 1);
        assert_eq!(count(&sent, |r| matches!(r, Request::SetGlyphs { .. })), 0);
    }

    /// What a run's ceiling is counted in: the quads an item says it takes are the quads the run sends (spaces take none), so a list
    /// of items can be cut into runs by their sum and never be one too many.
    #[test]
    fn an_item_says_how_many_quads_it_takes_and_a_run_sends_exactly_that_many() {
        let items = row_items("Alarm clock", "Mon to Fri");
        let says: Vec<usize> = items.iter().map(Item::quads).collect();
        assert_eq!(
            says,
            [10, 8, 1, 1, 1],
            "words without their spaces, a mark one each"
        );
        let mut h = primed(&items);
        let layer =
            TextLayer::new(&mut h.ui, Frame::new(0.0, 0.0, 300.0, 64.0), 1, &items).unwrap();
        assert!(layer.is_glyph_run());
        let sent = h.commit();
        assert_eq!(quads(&sent).len(), says.iter().sum::<usize>(), "{sent:?}");
    }

    /// A text the page cannot take whole still has the glyphs it could place made ready and queued, and the layer that asked is a
    /// picture and pumps nothing: the bands are handed over by the failure itself, or, with no upload in flight, no acknowledgement
    /// ever comes for them to be staged by.
    #[test]
    fn a_text_the_page_cannot_take_whole_still_has_the_glyphs_it_could_place_handed_over() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        // Two shelves of a small type and no room for a third.
        h.ui.text_atlas = Some(h.ui.scene.atlas(64, 36).unwrap());
        let page = h.ui.text_atlas.as_ref().unwrap().view();
        let text = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert!(!prepare_text(&mut h.ui, &page, CAPTION, text));
        assert!(h.ui.text_atlas_full, "the page said it was full");
        assert_eq!(
            page.pending_bands(),
            0,
            "what it placed is in the staging slots, not waiting for a pump nobody makes"
        );
        assert!(!page.is_settled(), "and on its way");
    }
}
