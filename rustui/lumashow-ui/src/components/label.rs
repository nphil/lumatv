//! Text on a layer: a [`Label`] is one line in a fixed box, a [`Block`] is several lines in one layer. Shapes are drawn by the
//! compositor; only text is rastered here, and only when it changes.

use super::text::{Run, aligned, line_height};
use super::text_layer::{Item, TextLayer, prepare_text};
use crate::atlas::{GlyphRun, one_ink};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::scene::ImageLayer;
use crate::text::{Canvas, Fonts, Type};
use compositor_protocol::{Ease, Props, Rgba};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// How a label's text looks: size in pixels, colour and horizontal alignment in its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub ty: Type,
    pub colour: Rgba,
    pub align: Align,
}

pub const fn style(ty: Type, colour: Rgba, align: Align) -> Style {
    Style { ty, colour, align }
}

/// One line of text in a fixed box. The box never changes size, so updating the text rewrites pixels in place (or swaps quads) and
/// never gives the label another layer: even a run that has to become a picture does it in the same compositor layer.
pub struct Label {
    face: Face,
    text: String,
    style: Style,
}

/// What a label is on the compositor: a picture of its text, or (with `Ui::enable_glyph_text`) glyph quads of an atlas. A run that
/// cannot show new words becomes a picture, in the same layer, and stays one.
enum Face {
    Image(ImageLayer),
    Run(GlyphRun),
}

impl Label {
    pub fn new(ui: &mut Ui, at: Frame, z: i32, style: Style, text: &str) -> io::Result<Self> {
        let (w, h) = (at.w.ceil() as u32, at.h.ceil() as u32);
        if ui.text_atlas.is_some() && w > 0 && h > 0 {
            let run = ui
                .scene
                .glyph_run(
                    ui.text_atlas.as_ref().expect("checked"),
                    w,
                    h,
                    z,
                    one_ink(style.colour),
                    Props::at(at.x, at.y),
                )
                .ok();
            if let Some(mut run) = run {
                if show_run(ui, &mut run, style, text, false) {
                    return Ok(Self {
                        face: Face::Run(run),
                        text: text.to_owned(),
                        style,
                    });
                }
                // Made and dropped before any commit: nothing of it is ever sent.
            }
        }
        if ui.text_atlas.is_some() {
            ui.text_pictures += 1;
        }
        let layer = ui.scene.image(w, h, z, Props::at(at.x, at.y))?;
        let mut label = Self {
            face: Face::Image(layer),
            text: String::new(),
            style,
        };
        label.draw(ui, text);
        Ok(label)
    }

    /// The compositor layer the text is drawn on.
    pub fn id(&self) -> compositor_protocol::LayerId {
        self.layer().id()
    }

    /// The label's compositor layer, to hang it under a native group.
    pub fn layer(&self) -> &crate::scene::Layer {
        match &self.face {
            Face::Image(layer) => layer,
            Face::Run(run) => run,
        }
    }

    /// Whether the text is glyph quads of the shared atlas and not a picture.
    pub fn is_glyph_run(&self) -> bool {
        matches!(self.face, Face::Run(..))
    }

    /// A label whose box is just tall enough for one line of its style's text.
    pub fn line(
        ui: &mut Ui,
        x: f32,
        y: f32,
        w: f32,
        z: i32,
        style: Style,
        text: &str,
    ) -> io::Result<Self> {
        Self::new(
            ui,
            Frame::new(x, y, w, super::text::line_height(style.ty)),
            z,
            style,
            text,
        )
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> Style {
        self.style
    }

    pub fn set(&mut self, ui: &mut Ui, text: &str) {
        if text != self.text {
            self.draw(ui, text);
        }
    }

    /// Moves the label at once, cancelling any animation on it: for a layout that repositions a label instead of only ever
    /// changing its text (a paged screen sliding its whole content), or hides it.
    pub fn set_props(&mut self, props: Props) {
        self.layer().set(props);
    }

    /// Animates the label's position, opacity and scale, exactly like a plain layer.
    pub fn animate_props(&mut self, to: Props, duration_ms: u32, ease: Ease) {
        self.layer().animate(to, duration_ms, ease);
    }

    pub fn set_colour(&mut self, ui: &mut Ui, colour: Rgba) {
        if colour != self.style.colour {
            self.style.colour = colour;
            if let Face::Run(run) = &self.face {
                if run.set_colour(colour).is_ok() {
                    return;
                }
                // The run cannot take the colour: its words are a picture from here on, in the colour.
                self.fall_back(ui);
            }
            let text = std::mem::take(&mut self.text);
            self.paint(ui, &text);
        }
    }

    fn draw(&mut self, ui: &mut Ui, text: &str) {
        if let Face::Run(run) = &mut self.face {
            if show_run(ui, run, self.style, text, true) {
                self.text = text.to_owned();
                return;
            }
            self.fall_back(ui);
        }
        self.paint(ui, text);
    }

    /// The run cannot show its words (the atlas it draws from is full, or the text is too long for a run): the label becomes a
    /// picture, in the same compositor layer, and stays one. The caller paints the words into it.
    fn fall_back(&mut self, ui: &mut Ui) {
        let Face::Run(run) = &mut self.face else {
            return;
        };
        match run.become_picture() {
            Ok(picture) => {
                // The run is dropped here: it lets go of its atlas, and with it a list that was waiting for glyphs.
                self.face = Face::Image(picture);
                ui.glyph_text_fallbacks += 1;
            }
            // No file for the pixels: the run keeps the words it has, and the scene is told that what it shows is not whole.
            Err(_) => ui.scene.note_refusal(),
        }
    }

    /// Paints `text` into the picture the label is, as it always was painted; a run that could not become one keeps what it shows.
    fn paint(&mut self, ui: &mut Ui, text: &str) {
        let style = self.style;
        if let Face::Image(layer) = &mut self.face {
            let (w, h) = (layer.width() as f32, layer.height() as f32);
            let width = layer.width() as usize;
            layer.clear();
            draw_line(
                &mut ui.fonts,
                &mut Canvas {
                    pixels: layer.pixels_mut(),
                    width,
                },
                Frame::new(0.0, 0.0, w, h),
                text,
                style,
            );
            layer.damage(None);
        }
        self.text = text.to_owned();
    }
}

/// Puts `text` in `run` as the quads `draw_line` would have painted, in the same places: its glyphs are made ready in the atlas
/// *the run draws from* (not the one new labels are made on: that may be another page by now) and handed over, and the run swaps
/// its quads. `false` when they cannot be shown: a run that is being made (`later` false) needs them at once; one that changes
/// later (`later` true) shows them as soon as the atlas has handed them over.
fn show_run(ui: &mut Ui, run: &mut GlyphRun, style: Style, text: &str, later: bool) -> bool {
    if text.chars().count() > compositor_protocol::resources::MAX_RUN_GLYPHS {
        return false;
    }
    let atlas = run.atlas();
    let (w, h) = run.size();
    let extent = ui.fonts.measure(text, style.ty);
    let x = aligned(0.0, w as f32, extent.width, style.align);
    let baseline = ((h as f32 - extent.height()) / 2.0 + extent.ascent).round();
    if !prepare_text(ui, &atlas, style.ty, text) {
        return false;
    }
    atlas.pump();
    let mut placed = Vec::with_capacity(text.len());
    if atlas
        .layout(text, style.ty, (x, baseline), 0, &mut placed)
        .is_err()
    {
        return false;
    }
    if later {
        run.set_glyphs_when_ready(&placed).is_ok()
    } else {
        run.set_glyphs(&placed).is_ok()
    }
}

/// Draws `text` into `canvas` inside `frame` (canvas pixels): placed horizontally in the frame by the style's alignment, its line
/// centred vertically, so every string of one type sits on the same baseline.
pub fn draw_line(
    fonts: &mut Fonts,
    canvas: &mut Canvas<'_>,
    frame: Frame,
    text: &str,
    style: Style,
) {
    let extent = fonts.measure(text, style.ty);
    let x = aligned(frame.x, frame.w, extent.width, style.align);
    let baseline = (frame.y + (frame.h - extent.height()) / 2.0 + extent.ascent).round();
    fonts.draw(canvas, text, style.ty, (x, baseline), style.colour);
}

/// Lines of text placed anywhere in one layer: one layer, one upload (or one swap of glyph quads), however many lines, for what
/// changes together. Without a ground the layer is a [`TextLayer`]: a run of glyph quads when the UI has glyph text and every
/// word can be shown from the atlas, a picture otherwise; the lines may be in at most four colours to be a run. With a ground it is
/// always a picture.
pub struct Block {
    face: BlockFace,
    lines: Vec<Line>,
}

enum BlockFace {
    Text(TextLayer),
    /// A picture painted with the ground (premultiplied RGBA, the layer's size) before the text goes on: a plate to write on that
    /// costs the compositor no layer of its own.
    Ground(ImageLayer, Vec<u8>),
}

struct Line {
    /// Where the line sits in the layer's own pixels.
    frame: Frame,
    style: Style,
    text: String,
}

impl Line {
    /// The line as a run that [`draw_line`] would have painted in the same place: the run's top is chosen so that its baseline is
    /// the one `draw_line` rounds to.
    fn run(&self, ui: &mut Ui) -> Run<'_> {
        let ty = self.style.ty;
        let extent = ui.fonts.measure(&self.text, ty);
        let baseline =
            (self.frame.y + (self.frame.h - extent.height()) / 2.0 + extent.ascent).round();
        let inside = ((line_height(ty) - extent.height()) / 2.0 + extent.ascent).round();
        Run {
            text: &self.text,
            ty,
            colour: self.style.colour,
            x: self.frame.x,
            y: baseline - inside,
            w: self.frame.w,
            align: self.style.align,
        }
    }
}

fn lines_of(lines: Vec<(Frame, Style, String)>) -> Vec<Line> {
    lines
        .into_iter()
        .map(|(frame, style, text)| Line { frame, style, text })
        .collect()
}

impl Block {
    /// A layer at `at` holding `lines`, each given as its frame in the layer, its look and its text.
    pub fn new(
        ui: &mut Ui,
        at: Frame,
        z: i32,
        lines: Vec<(Frame, Style, String)>,
    ) -> io::Result<Self> {
        let lines = lines_of(lines);
        let items: Vec<Item> = lines.iter().map(|l| Item::Text(l.run(ui))).collect();
        let layer = TextLayer::new(ui, at, z, &items)?;
        Ok(Self {
            face: BlockFace::Text(layer),
            lines,
        })
    }

    /// The same on a `ground` (premultiplied RGBA, `at.w` by `at.h` pixels) the text is written over: a picture.
    pub fn on_ground(
        ui: &mut Ui,
        at: Frame,
        z: i32,
        ground: Vec<u8>,
        lines: Vec<(Frame, Style, String)>,
    ) -> io::Result<Self> {
        debug_assert_eq!(
            ground.len(),
            at.w.ceil() as usize * at.h.ceil() as usize * 4
        );
        let layer = ui.scene.image(
            at.w.ceil() as u32,
            at.h.ceil() as u32,
            z,
            Props::at(at.x, at.y),
        )?;
        let mut block = Self {
            face: BlockFace::Ground(layer, ground),
            lines: lines_of(lines),
        };
        block.draw(ui);
        Ok(block)
    }

    /// Changes line `line`'s text; the layer is drawn again only if it differs.
    pub fn set(&mut self, ui: &mut Ui, line: usize, text: &str) {
        if self.lines[line].text != text {
            self.lines[line].text = text.to_owned();
            self.draw(ui);
        }
    }

    /// Changes the text and colour of several lines, `(line, text, colour)`; the layer is drawn again once, and only if something
    /// differs.
    pub fn set_many(&mut self, ui: &mut Ui, changes: &[(usize, &str, Rgba)]) -> bool {
        let mut changed = false;
        for &(line, text, colour) in changes {
            let line = &mut self.lines[line];
            if line.text != text || line.style.colour != colour {
                text.clone_into(&mut line.text);
                line.style.colour = colour;
                changed = true;
            }
        }
        if changed {
            self.draw(ui);
        }
        changed
    }

    /// Moves, hides or clips the layer at once, cancelling any animation on it.
    pub fn set_props(&mut self, props: Props) {
        self.layer().set(props);
    }

    /// Sends the layer to `to` on the compositor's clock.
    pub fn animate_props(&mut self, to: Props, duration_ms: u32, ease: Ease) {
        self.layer().animate(to, duration_ms, ease);
    }

    pub fn layer(&self) -> &crate::scene::Layer {
        match &self.face {
            BlockFace::Text(layer) => layer.layer(),
            BlockFace::Ground(layer, _) => layer,
        }
    }

    /// Whether the text is glyph quads of the shared atlas and not a picture.
    pub fn is_glyph_run(&self) -> bool {
        matches!(&self.face, BlockFace::Text(layer) if layer.is_glyph_run())
    }

    fn draw(&mut self, ui: &mut Ui) {
        match &mut self.face {
            BlockFace::Text(layer) => {
                let runs: Vec<Run> = self.lines.iter().map(|l| l.run(ui)).collect();
                layer.draw_runs(ui, &runs);
            }
            BlockFace::Ground(layer, ground) => {
                let width = layer.width() as usize;
                layer.pixels_mut().copy_from_slice(ground);
                for line in &self.lines {
                    draw_line(
                        &mut ui.fonts,
                        &mut Canvas {
                            pixels: layer.pixels_mut(),
                            width,
                        },
                        line.frame,
                        &line.text,
                        line.style,
                    );
                }
                layer.damage(None);
            }
        }
    }
}

#[cfg(test)]
mod glyph_text_tests {
    use super::*;
    use crate::testing::harness;
    use crate::text::Weight;
    use compositor_protocol::{Content, Request};

    const TY: Type = Type {
        px: 16,
        weight: Weight::Regular,
    };
    const INK: Rgba = Rgba(236, 236, 240, 255);

    fn made(requests: &[Request], f: impl Fn(&Request) -> bool) -> usize {
        requests.iter().filter(|r| f(r)).count()
    }

    fn pictures(requests: &[Request]) -> usize {
        made(requests, |r| {
            matches!(
                r,
                Request::Create {
                    content: Content::Image,
                    ..
                }
            )
        })
    }

    fn pixels_written(requests: &[Request]) -> f32 {
        requests
            .iter()
            .map(|r| match r {
                Request::Damage { rect, .. } => rect.w * rect.h,
                _ => 0.0,
            })
            .sum()
    }

    fn quads(requests: &[Request]) -> Vec<compositor_protocol::Glyph> {
        requests
            .iter()
            .filter_map(|r| match r {
                Request::SetGlyphs { glyphs, .. } => Some(glyphs.clone()),
                _ => None,
            })
            .next_back()
            .unwrap_or_default()
    }

    #[test]
    fn a_label_is_a_picture_until_glyph_text_is_switched_on() {
        let mut h = harness();
        let label = Label::line(
            &mut h.ui,
            10.0,
            20.0,
            200.0,
            5,
            style(TY, INK, Align::Left),
            "Wi-Fi",
        )
        .unwrap();
        assert!(!label.is_glyph_run());
        let sent = h.commit();
        assert_eq!(pictures(&sent), 1);
        assert!(pixels_written(&sent) > 0.0);
    }

    #[test]
    fn a_label_on_glyph_text_makes_no_picture_and_writes_no_pixel() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let label = Label::line(
            &mut h.ui,
            10.0,
            20.0,
            200.0,
            5,
            style(TY, INK, Align::Left),
            "Wi-Fi",
        )
        .unwrap();
        assert!(label.is_glyph_run());
        let sent = h.commit();
        assert_eq!(pictures(&sent), 0, "{sent:?}");
        assert_eq!(pixels_written(&sent), 0.0);
        // The five characters are four quads (the hyphen and letters; no space) of one run layer, and the glyphs travel as one
        // upload of the atlas, not as pixels of the layer.
        assert_eq!(quads(&sent).len(), 5);
        assert!(made(&sent, |r| matches!(r, Request::AtlasUpload { .. })) >= 1);
    }

    #[test]
    fn the_quads_cover_the_same_pixels_the_picture_would_have_been_painted_on() {
        // The picture is the oracle: the box that holds all its ink is the box that holds all the quads.
        let text = "Brightness 80%";
        let mut picture = harness();
        let image = Label::line(
            &mut picture.ui,
            0.0,
            0.0,
            300.0,
            1,
            style(TY, INK, Align::Centre),
            text,
        )
        .unwrap();
        picture.commit();
        let bytes = picture.pixels(image.id());
        let width = 300usize;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (i, px) in bytes.chunks_exact(4).enumerate() {
            if px[3] != 0 {
                let (x, y) = (i % width, i / width);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        let mut run = harness();
        run.ui.enable_glyph_text().unwrap();
        let _label = Label::line(
            &mut run.ui,
            0.0,
            0.0,
            300.0,
            1,
            style(TY, INK, Align::Centre),
            text,
        )
        .unwrap();
        let sent = run.commit();
        let quads = quads(&sent);
        assert!(!quads.is_empty());
        let left = quads.iter().map(|q| q.x).fold(f32::MAX, f32::min);
        let top = quads.iter().map(|q| q.y).fold(f32::MAX, f32::min);
        let right = quads.iter().map(|q| q.x + q.w as f32).fold(0.0, f32::max);
        let bottom = quads.iter().map(|q| q.y + q.h as f32).fold(0.0, f32::max);
        // A glyph bitmap may carry a row or column of nothing at its edge, never more than one pixel.
        assert!((left - x0 as f32).abs() <= 1.0, "left {left} vs ink {x0}");
        assert!(
            (right - x1 as f32).abs() <= 1.0,
            "right {right} vs ink {x1}"
        );
        assert!((top - y0 as f32).abs() <= 1.0, "top {top} vs ink {y0}");
        assert!(
            (bottom - y1 as f32).abs() <= 1.0,
            "bottom {bottom} vs ink {y1}"
        );
    }

    #[test]
    fn the_same_characters_again_cost_no_upload_and_a_new_text_is_a_swap_of_quads() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let mut first = Label::line(
            &mut h.ui,
            0.0,
            0.0,
            200.0,
            1,
            style(TY, INK, Align::Left),
            "Volume",
        )
        .unwrap();
        h.commit();
        let _second = Label::line(
            &mut h.ui,
            0.0,
            30.0,
            200.0,
            1,
            style(TY, INK, Align::Left),
            "Volume",
        )
        .unwrap();
        let sent = h.commit();
        assert_eq!(
            made(&sent, |r| matches!(r, Request::AtlasUpload { .. })),
            0,
            "{sent:?}"
        );
        assert_eq!(pictures(&sent), 0);
        // Changing what a label says with letters the atlas holds is one swap: no picture, no pixel, no upload, no new layer.
        first.set(&mut h.ui, "Volume");
        first.set(&mut h.ui, "Vol");
        let swap = h.commit();
        assert_eq!(
            made(&swap, |r| matches!(r, Request::SetGlyphs { .. })),
            1,
            "{swap:?}"
        );
        assert_eq!(made(&swap, |r| matches!(r, Request::AtlasUpload { .. })), 0);
        assert_eq!(made(&swap, |r| matches!(r, Request::Create { .. })), 0);
        assert_eq!(pixels_written(&swap), 0.0);
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
    }

    #[test]
    fn a_text_too_long_for_a_run_is_a_picture() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let long = "a".repeat(300);
        let label = Label::line(
            &mut h.ui,
            0.0,
            0.0,
            200.0,
            1,
            style(TY, INK, Align::Left),
            &long,
        )
        .unwrap();
        assert!(!label.is_glyph_run());
    }

    #[test]
    fn recolouring_a_run_restyles_it_and_leaves_its_quads() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let mut label = Label::line(
            &mut h.ui,
            0.0,
            0.0,
            200.0,
            1,
            style(TY, INK, Align::Left),
            "Mute",
        )
        .unwrap();
        h.commit();
        label.set_colour(&mut h.ui, Rgba(160, 100, 100, 255));
        let sent = h.commit();
        assert_eq!(
            made(&sent, |r| matches!(r, Request::Restyle { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(made(&sent, |r| matches!(r, Request::SetGlyphs { .. })), 0);
        assert_eq!(pixels_written(&sent), 0.0);
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    use crate::group::GroupPose;
    use crate::limits::MAX_DRAWABLE_NODES;
    use crate::testing::{Harness, count_requests, harness, pixels_damaged, warm_glyph_harness};
    use crate::text::Weight;
    use compositor_protocol::{Content, Fill, LayerId, Request, Shape};

    const TY: Type = Type {
        px: 16,
        weight: Weight::Regular,
    };
    const INK: Rgba = Rgba(236, 236, 240, 255);
    const BOX: Frame = Frame::new(10.0, 20.0, 240.0, 22.0);

    fn make(h: &mut Harness, text: &str) -> Label {
        Label::new(&mut h.ui, BOX, 3, style(TY, INK, Align::Left), text).unwrap()
    }

    /// A glyph UI whose page has seen `text` and been told it landed, and a label of `text` on it: a run, committed.
    fn up(text: &str) -> (Harness, Label) {
        let mut h = warm_glyph_harness(|h| make(h, text));
        let label = make(&mut h, text);
        assert!(label.is_glyph_run());
        h.commit();
        (h, label)
    }

    /// What the plain path paints for `text` in `colour`: the pixels a label that is a picture has.
    fn picture_of(text: &str, colour: Rgba) -> Vec<u8> {
        let mut pic = harness();
        let label = Label::new(&mut pic.ui, BOX, 3, style(TY, colour, Align::Left), text).unwrap();
        pic.commit();
        pic.layer_pixels(label.id())
    }

    /// Whether `r` is about layer `id`.
    fn about(r: &Request, id: LayerId) -> bool {
        match r {
            Request::Create { id: of, .. }
            | Request::Damage { id: of, .. }
            | Request::Restyle { id: of, .. }
            | Request::Set { id: of, .. }
            | Request::Animate { id: of, .. }
            | Request::Attach { id: of, .. }
            | Request::SetGlyphs { id: of, .. }
            | Request::Delete { id: of } => *of == id,
            _ => false,
        }
    }

    #[test]
    fn a_label_whose_page_is_full_becomes_a_picture_of_its_new_words_in_the_same_layer() {
        let (mut h, mut label) = up("Volume");
        let (id, layers) = (label.id(), h.ui.scene.live_layers());
        let c = h.fill_text_page(TY);
        // Words it has the glyphs of are shown from the page it has, full or not.
        label.set(&mut h.ui, "Vol");
        assert!(label.is_glyph_run());
        assert_eq!(
            (h.ui.glyph_text_fallbacks, h.ui.text_atlas_full),
            (0, false)
        );
        h.commit();
        // A glyph it has not: a picture of the words, in the same layer, replaced once, no quads; and the page says it is full.
        let text = format!("Vol{c}");
        label.set(&mut h.ui, &text);
        assert!(!label.is_glyph_run());
        assert_eq!(label.id(), id);
        assert_eq!(h.ui.scene.live_layers(), layers, "replaced, not added");
        assert_eq!((h.ui.glyph_text_fallbacks, h.ui.text_atlas_full), (1, true));
        let sent = h.commit();
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::Create { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(
            count_requests(
                &sent,
                |r| matches!(r, Request::Create { id: of, content: Content::Image, .. } if *of == id)
            ),
            1,
            "{sent:?}"
        );
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::SetGlyphs { .. } | Request::Restyle { .. } | Request::Delete { .. }
            )),
            0,
            "{sent:?}"
        );
        assert!(pixels_damaged(&sent) > 0.0);
        let shown = h.layer_pixels(id);
        assert!(shown.iter().any(|v| *v != 0));
        assert_eq!(shown, picture_of(&text, INK));
        // It stays a picture: the next words and the next colour are painted into it, in the same layer; no run comes back.
        label.set(&mut h.ui, "Mute");
        let dim = Rgba(160, 100, 100, 255);
        label.set_colour(&mut h.ui, dim);
        let sent = h.commit();
        assert!(!label.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 1, "a picture is not a fallback");
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::Create { .. } | Request::SetGlyphs { .. } | Request::Restyle { .. }
            )),
            0,
            "{sent:?}"
        );
        assert_eq!(h.layer_pixels(id), picture_of("Mute", dim));
    }

    #[test]
    fn a_label_told_to_say_more_than_a_run_can_becomes_a_picture_of_it_in_the_same_layer() {
        let (mut h, mut label) = up("Volume");
        let id = label.id();
        let long = "i".repeat(300);
        label.set(&mut h.ui, &long);
        assert!(!label.is_glyph_run());
        assert_eq!(label.id(), id);
        assert_eq!(h.ui.glyph_text_fallbacks, 1);
        assert!(
            !h.ui.text_atlas_full,
            "the page had room: it is the run that has not"
        );
        h.commit();
        assert_eq!(h.layer_pixels(id), picture_of(&long, INK));
    }

    #[test]
    fn a_label_that_becomes_a_picture_is_made_again_where_it_was_last_put() {
        let long = "i".repeat(300);
        let moved = Props::at(33.0, 44.0);
        let fading = Props {
            opacity: 0.25,
            ..Props::at(5.0, 6.0)
        };
        for (how, wanted) in [
            ("where it was made", Props::at(10.0, 20.0)),
            ("moved, not yet committed", moved),
            ("moved and committed", moved),
            ("sent somewhere: where the animation ends", fading),
        ] {
            let (mut h, mut label) = up("Mute");
            match how {
                "moved, not yet committed" => label.set_props(moved),
                "moved and committed" => {
                    label.set_props(moved);
                    h.commit();
                }
                "sent somewhere: where the animation ends" => {
                    label.animate_props(fading, 300, Ease::Out);
                }
                _ => {}
            }
            label.set(&mut h.ui, &long);
            let sent = h.commit();
            let made = sent.iter().find_map(|r| match r {
                Request::Create { props, .. } => Some(*props),
                _ => None,
            });
            assert_eq!(made, Some(wanted), "{how}: {sent:?}");
        }
    }

    #[test]
    fn a_converted_label_is_still_under_its_group_with_the_place_it_had_in_it() {
        // Attached with a place, or attached and then placed in the group's space: that is the place the layer has in it.
        for moved in [false, true] {
            let (mut h, mut label) = up("Mute");
            let group = h.ui.scene.group(None, GroupPose::at(5.0, 6.0)).unwrap();
            group.attach(label.layer(), Props::at(30.0, 40.0)).unwrap();
            let local = if moved {
                label.set_props(Props::at(31.0, 41.0));
                Props::at(31.0, 41.0)
            } else {
                Props::at(30.0, 40.0)
            };
            h.commit();
            label.set(&mut h.ui, &"i".repeat(300));
            let sent = h.commit();
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
                    id: label.id(),
                    group: Some(group.wire_id()),
                    local
                },
                "moved: {moved}"
            );
            assert_eq!(label.layer().group(), Some(group.id()));
            // The group is held by the layer that hangs in it, as before: it goes when that does, and the layer is deleted once.
            assert_eq!(h.ui.scene.groups_alive(), 1);
            drop(group);
            assert_eq!(h.ui.scene.groups_alive(), 1, "the label still hangs in it");
            let id = label.id();
            drop(label);
            let sent = h.commit();
            assert_eq!(h.ui.scene.groups_alive(), 0);
            assert_eq!(
                count_requests(
                    &sent,
                    |r| matches!(r, Request::Delete { id: of } if *of == id)
                ),
                1,
                "{sent:?}"
            );
        }
    }

    #[test]
    fn a_label_goes_on_drawing_from_its_own_page_when_new_labels_are_made_on_another() {
        let (mut h, mut old) = up("Volume");
        let first = h.ui.text_atlas.as_ref().unwrap().view();
        // The page new text is made on is replaced; the old one stays for the label that draws from it.
        h.ui.text_atlas = Some(h.ui.scene.atlas(1024, 1024).unwrap());
        h.ui.warm(&[TY]);
        h.settle_atlas();
        let second = h.ui.text_atlas.as_ref().unwrap().view();
        assert_ne!(first.id(), second.id());
        // New words with a glyph neither page has: the old label prepares it on its own page and lays out against that page. (A
        // glyph of the new page's table, offered to the old run, would be a wrong glyph or an error.)
        assert!(first.glyph(TY, 'é').is_none() && second.glyph(TY, 'é').is_none());
        old.set(&mut h.ui, "Café");
        h.settle_atlas();
        assert!(old.is_glyph_run());
        assert!(
            first.glyph(TY, 'é').is_some(),
            "made ready where the run is"
        );
        assert!(
            second.glyph(TY, 'é').is_none(),
            "and not on the page it is not on"
        );
        assert_eq!(h.layer_pixels(old.id()), picture_of("Café", INK));
        // A label made now is made on the new page.
        let new = make(&mut h, "Volume");
        assert!(new.is_glyph_run());
        let Face::Run(run) = &new.face else {
            unreachable!("a run")
        };
        assert_eq!(run.atlas().id(), second.id());
        let Face::Run(run) = &old.face else {
            unreachable!("a run")
        };
        assert_eq!(run.atlas().id(), first.id(), "the old one did not move");
        h.commit();
        // When the old label stops being a run the page it drew from is let go of: after the picture that replaces the run, since
        // the compositor refuses to delete an atlas a layer still draws from.
        let (wire, id) = (first.wire_id(), old.id());
        old.set(&mut h.ui, &"i".repeat(300));
        let sent = h.commit();
        let at = |f: &dyn Fn(&Request) -> bool| sent.iter().position(f).expect("sent");
        let create = at(&|r| matches!(r, Request::Create { id: of, .. } if *of == id));
        let delete = at(&|r| matches!(r, Request::AtlasDelete { id: of } if *of == wire));
        assert!(create < delete, "{sent:?}");
    }

    #[test]
    fn a_label_whose_new_glyphs_are_still_on_their_way_stays_a_run() {
        let (mut h, mut label) = up("Mute");
        // Both staging slots are busy with other bands, so the glyph the new words need waits for one.
        let page = h.ui.text_atlas.as_ref().unwrap().view();
        for px in [48, 64] {
            let big = Type {
                px,
                weight: TY.weight,
            };
            page.prepare(&mut h.ui.fonts, big, "0123").unwrap();
        }
        assert_eq!(page.pump(), 2);
        h.commit();
        label.set(&mut h.ui, "Mute 9");
        assert!(label.is_glyph_run(), "it waits, it is not a picture");
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
        let Face::Run(run) = &label.face else {
            unreachable!("a run")
        };
        assert!(run.is_waiting());
        // The compositor answers; the words appear whole.
        h.settle_atlas();
        let Face::Run(run) = &label.face else {
            unreachable!("a run")
        };
        assert!(!run.is_waiting());
        assert_eq!(h.layer_pixels(label.id()), picture_of("Mute 9", INK));
    }

    #[test]
    fn a_label_the_compositor_never_heard_of_stays_a_dummy_when_it_becomes_a_picture() {
        let mut h = warm_glyph_harness(|h| make(h, "Volume"));
        let shape = Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(1, 2, 3, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        };
        let _full: Vec<_> = (0..MAX_DRAWABLE_NODES)
            .map(|_| h.ui.scene.shape(4, 4, 1, shape, Props::at(0.0, 0.0)))
            .collect();
        h.commit();
        assert!(!h.ui.scene.take_overflow());
        // No room for it: the label is a run that exists only here, and nothing is sent for it.
        let mut label = make(&mut h, "Volume");
        assert!(h.ui.scene.take_overflow());
        assert!(label.is_glyph_run());
        let id = label.id();
        label.set(&mut h.ui, &"i".repeat(300));
        assert!(!label.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 1);
        assert_eq!(h.ui.scene.live_layers(), MAX_DRAWABLE_NODES);
        let sent = h.commit();
        assert!(
            sent.iter().all(|r| !about(r, id)),
            "nothing is sent for it: {sent:?}"
        );
    }

    #[test]
    fn a_label_made_as_a_picture_while_glyph_text_is_on_is_counted_and_one_made_without_it_is_not()
    {
        let mut plain = harness();
        make(&mut plain, "Mute");
        assert_eq!(
            plain.ui.text_pictures, 0,
            "no glyph text: no picture is a miss"
        );
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        // Too long for a run: a picture at its creation, which is no fallback.
        make(&mut h, &"i".repeat(300));
        assert_eq!((h.ui.text_pictures, h.ui.glyph_text_fallbacks), (1, 0));
    }

    #[test]
    fn what_was_staged_for_the_quads_of_a_run_that_becomes_a_picture_is_taken_back() {
        let (mut h, mut label) = up("Volume");
        let id = label.id();
        // In one batch: a swap of quads and a recolour, then words the run cannot show. The colour goes into the picture.
        let dim = Rgba(160, 100, 100, 255);
        label.set(&mut h.ui, "Vol");
        label.set_colour(&mut h.ui, dim);
        let long = "i".repeat(300);
        label.set(&mut h.ui, &long);
        let sent = h.commit();
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::SetGlyphs { .. } | Request::Restyle { .. }
            )),
            0,
            "{sent:?}"
        );
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::Create { .. })),
            1
        );
        assert_eq!(h.layer_pixels(id), picture_of(&long, dim));
    }

    #[test]
    fn a_list_that_waited_for_its_glyphs_is_forgotten_when_the_run_becomes_a_picture() {
        let (mut h, mut label) = up("Mute");
        let id = label.id();
        let page = h.ui.text_atlas.as_ref().unwrap().view();
        for px in [48, 64] {
            let big = Type {
                px,
                weight: TY.weight,
            };
            page.prepare(&mut h.ui.fonts, big, "0123").unwrap();
        }
        assert_eq!(page.pump(), 2);
        h.commit();
        label.set(&mut h.ui, "Mute 9");
        assert!(label.is_glyph_run());
        // Before the glyph arrives the words change to ones that cannot be a run: the waiting list must not be sent later, to a
        // layer that is a picture by then (the compositor would refuse it).
        let long = "i".repeat(300);
        label.set(&mut h.ui, &long);
        assert!(!label.is_glyph_run());
        h.settle_atlas();
        let all = h.all();
        let made = all
            .iter()
            .rposition(|r| matches!(r, Request::Create { id: of, .. } if *of == id))
            .expect("made again");
        assert!(
            all[made..]
                .iter()
                .all(|r| !matches!(r, Request::SetGlyphs { id: of, .. } if *of == id)),
            "a list was sent to the picture"
        );
        assert_eq!(h.layer_pixels(id), picture_of(&long, INK));
    }

    #[test]
    fn a_run_that_cannot_get_a_file_for_its_pixels_keeps_what_it_shows_and_tells_the_scene() {
        let (mut h, mut label) = up("Mute");
        let id = label.id();
        assert!(!h.ui.scene.take_overflow());
        std::fs::remove_dir_all(h.shm_dir()).unwrap();
        label.set(&mut h.ui, &"i".repeat(300));
        assert!(label.is_glyph_run(), "it has no picture to become");
        assert_eq!(h.ui.glyph_text_fallbacks, 0, "and it is not counted as one");
        assert!(
            h.ui.scene.take_overflow(),
            "what is shown is not whole: the scene is told"
        );
        let sent = h.commit();
        assert!(sent.iter().all(|r| !about(r, id)), "{sent:?}");
    }

    #[test]
    fn only_the_page_new_text_is_made_on_being_full_is_news() {
        let (mut h, mut old) = up("Volume");
        let c = h.fill_text_page(TY);
        // The page is replaced by a fresh one; the old page, full, is still what the old label draws from.
        h.ui.text_atlas = Some(h.ui.scene.atlas(1024, 1024).unwrap());
        old.set(&mut h.ui, &format!("Vol{c}"));
        assert!(!old.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 1);
        assert!(
            !h.ui.text_atlas_full,
            "the page it draws from is not the text page"
        );
        // The text page itself found full is news, for a label made on it too.
        let c = h.fill_text_page(TY);
        let pictures = h.ui.text_pictures;
        let made = make(&mut h, &format!("Vol{c}"));
        assert!(!made.is_glyph_run());
        assert!(h.ui.text_atlas_full);
        assert_eq!(h.ui.text_pictures, pictures + 1);
        // And `warm` finding it full says so too.
        h.ui.text_atlas_full = false;
        h.ui.warm(&[Type {
            px: TY.px + 1,
            weight: TY.weight,
        }]);
        assert!(h.ui.text_atlas_full);
    }

    #[test]
    fn a_label_taken_out_of_its_group_is_made_again_on_the_screen_where_it_was_put() {
        let (mut h, mut label) = up("Mute");
        let group = h.ui.scene.group(None, GroupPose::at(5.0, 6.0)).unwrap();
        group.attach(label.layer(), Props::at(30.0, 40.0)).unwrap();
        label.layer().detach(Props::at(50.0, 60.0));
        h.commit();
        label.set(&mut h.ui, &"i".repeat(300));
        let sent = h.commit();
        let made = sent.iter().find_map(|r| match r {
            Request::Create { props, .. } => Some(*props),
            _ => None,
        });
        assert_eq!(made, Some(Props::at(50.0, 60.0)), "{sent:?}");
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::Attach { .. })),
            0,
            "it hangs under no group: {sent:?}"
        );
        assert_eq!(label.layer().group(), None);
    }
}

#[cfg(test)]
mod block_tests {
    use super::*;
    use crate::testing::{
        Harness, count_requests, harness, made_as_picture, pictures_made, pixels_damaged,
        pixels_damaged_in, warm_glyph_harness,
    };
    use crate::text::Weight;
    use compositor_protocol::{Content, Request};

    const HEAD: Type = Type {
        px: 22,
        weight: Weight::SemiBold,
    };
    const BODY: Type = Type {
        px: 16,
        weight: Weight::Regular,
    };
    const NOTE: Type = Type {
        px: 13,
        weight: Weight::Regular,
    };
    const INK: Rgba = Rgba(236, 236, 240, 255);
    const INK2: Rgba = Rgba(150, 150, 165, 255);
    const INK3: Rgba = Rgba(100, 100, 115, 255);
    const AT: Frame = Frame::new(20.0, 30.0, 300.0, 120.0);

    /// A heading, a centred line and a right-aligned footnote, on fractional frames so that each baseline is a rounded one.
    fn lines(head: &str, body: &str) -> Vec<(Frame, Style, String)> {
        vec![
            (
                Frame::new(16.0, 6.0, 268.0, 26.0),
                style(HEAD, INK, Align::Left),
                head.to_owned(),
            ),
            (
                Frame::new(16.0, 38.5, 268.0, 20.7),
                style(BODY, INK2, Align::Centre),
                body.to_owned(),
            ),
            (
                Frame::new(16.25, 70.25, 268.0, 18.0),
                style(NOTE, INK3, Align::Right),
                "Updated just now".to_owned(),
            ),
        ]
    }

    fn block(h: &mut Harness, head: &str, body: &str) -> Block {
        let b = Block::new(&mut h.ui, AT, 4, lines(head, body)).unwrap();
        h.commit();
        b
    }

    fn id(b: &Block) -> compositor_protocol::LayerId {
        b.layer().id()
    }

    #[test]
    fn a_block_without_a_ground_is_painted_exactly_as_it_was_painted_line_by_line() {
        let mut h = harness();
        let plain = block(&mut h, "Display", "Brightness and sleep");
        let ground = vec![0u8; 300 * 120 * 4];
        let old = Block::on_ground(
            &mut h.ui,
            AT,
            4,
            ground,
            lines("Display", "Brightness and sleep"),
        )
        .unwrap();
        h.commit();
        assert!(!plain.is_glyph_run());
        let pixels = h.pixels(id(&plain));
        assert!(pixels.iter().any(|v| *v != 0));
        assert_eq!(pixels, h.pixels(id(&old)));
    }

    #[test]
    fn a_blocks_words_are_the_same_pixels_as_glyph_quads_and_as_a_picture() {
        let mut run = warm_glyph_harness(|h| block(h, "Display", "Brightness and sleep"));
        let glyphs = block(&mut run, "Display", "Brightness and sleep");
        assert!(glyphs.is_glyph_run());
        let mut pic = harness();
        let picture = block(&mut pic, "Display", "Brightness and sleep");
        let (a, b) = (
            run.layer_pixels(id(&glyphs)),
            pic.layer_pixels(id(&picture)),
        );
        assert!(a.iter().any(|v| *v != 0));
        assert_eq!(a, b);
    }

    #[test]
    fn on_glyph_text_a_block_makes_no_picture_and_writes_no_pixel_and_new_words_are_one_swap() {
        let mut run = warm_glyph_harness(|h| {
            (
                block(h, "Display", "Brightness and sleep"),
                block(h, "Sound", "Volume and alerts"),
            )
        });
        let mut glyphs =
            Block::new(&mut run.ui, AT, 4, lines("Display", "Brightness and sleep")).unwrap();
        let sent = run.commit();
        assert!(glyphs.is_glyph_run());
        assert_eq!(pictures_made(&sent), 0, "{sent:?}");
        assert_eq!(pixels_damaged(&sent), 0.0);
        // The heading and the centred line change: one swap of quads, no layer, no pixel.
        glyphs.set(&mut run.ui, 0, "Sound");
        glyphs.set(&mut run.ui, 1, "Volume and alerts");
        let sent = run.commit();
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::SetGlyphs { .. })),
            1,
            "{sent:?}"
        );
        assert_eq!(
            count_requests(&sent, |r| matches!(r, Request::Create { .. })),
            0
        );
        assert_eq!(pixels_damaged(&sent), 0.0);
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
        // Words and colour of two lines at once are still one drawing; the picture of the same lines is the same pixels.
        assert!(glyphs.set_many(
            &mut run.ui,
            &[
                (1, "Brightness and sleep", INK3),
                (2, "Updated just now", INK)
            ]
        ));
        let sent = run.commit();
        assert_eq!(pixels_damaged(&sent), 0.0);
        assert_eq!(pictures_made(&sent), 0);
        assert_eq!(
            count_requests(&sent, |r| matches!(
                r,
                Request::Restyle {
                    content: Content::Glyphs(_),
                    ..
                }
            )),
            1,
            "{sent:?}"
        );
        let mut pic = harness();
        let mut picture = block(&mut pic, "Sound", "Volume and alerts");
        picture.set_many(
            &mut pic.ui,
            &[
                (1, "Brightness and sleep", INK3),
                (2, "Updated just now", INK),
            ],
        );
        pic.commit();
        assert_eq!(
            run.layer_pixels(id(&glyphs)),
            pic.layer_pixels(id(&picture))
        );
    }

    #[test]
    fn a_block_on_a_ground_is_a_picture_even_on_glyph_text() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let block = Block::on_ground(
            &mut h.ui,
            AT,
            4,
            vec![9u8; 300 * 120 * 4],
            lines("Frame", "60.0 fps"),
        )
        .unwrap();
        let sent = h.commit();
        assert!(!block.is_glyph_run());
        assert!(made_as_picture(&sent, id(&block)));
        assert!(pixels_damaged_in(&sent, id(&block)) > 0.0);
    }

    #[test]
    fn lines_in_more_than_four_colours_are_a_picture_that_shows_them_all() {
        let five = |h: &mut Harness| {
            let colours = [
                INK,
                INK2,
                INK3,
                Rgba(200, 80, 80, 255),
                Rgba(80, 200, 80, 255),
            ];
            let lines = colours
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    (
                        Frame::new(4.0, 4.0 + 22.0 * i as f32, 200.0, 22.0),
                        style(BODY, *c, Align::Left),
                        format!("Line {i}"),
                    )
                })
                .collect();
            let b = Block::new(&mut h.ui, AT, 4, lines).unwrap();
            h.commit();
            b
        };
        let mut run = warm_glyph_harness(|h| five(h));
        let glyphs = five(&mut run);
        assert!(!glyphs.is_glyph_run());
        let mut pic = harness();
        let picture = five(&mut pic);
        assert_eq!(run.pixels(id(&glyphs)), pic.pixels(id(&picture)));
    }
}
