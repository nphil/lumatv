//! The glass pills over the picture (`docs/mocks/cameras-scrypted/index.html` `.pill`): a row of an icon, words, a dot, a row of hints,
//! 64 px high (more where the type is large), with 24 px each side, on a reading surface (the glass at `readingAlpha`: nothing is
//! sampled behind it, the picture is video). One builder serves the name pill, the info pill, the line "Opens in a few seconds", the
//! hints, a refusal's toast.

use super::common::{Ink, Parts, icon_side, scale};
use crate::cameras::view::{Do, HintView};
use crate::kit::Kit;
use crate::kit::glyph;
use crate::kit::hint::Key;
use crate::kit::icon;
use crate::kit::pane::{Pane, PaneKind, glass};
use crate::kit::text::{Text, role_style, width_of};
use compositor_protocol::Rgba;
use lumatv_tokens::TypeRole;
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::text::Weight;

/// One thing in a pill, left to right.
pub enum Piece<'a> {
    Icon { name: &'a str, base: f32, ink: Rgba },
    Text { text: &'a str, role: TypeRole, weight: Option<Weight>, ink: Rgba },
    Dot(Rgba),
    Hints(&'a [HintView]),
    Gap(f32),
}

/// Where the pill is held.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    TopLeft { x: f32, y: f32 },
    TopRight { right: f32, y: f32 },
    BottomLeft { x: f32, bottom: f32 },
    BottomRight { right: f32, bottom: f32 },
    BottomCentre { cx: f32, bottom: f32 },
}

/// A run of words as the pill drew it, for the lint.
#[derive(Clone, Debug)]
pub struct WordRec {
    pub name: String,
    pub text: String,
    pub frame: Frame,
    pub style: Style,
}

/// What a pill drew.
#[derive(Clone, Debug)]
pub struct Drawn {
    pub frame: Frame,
    /// What the pane paints (premultiplied), for the contrast.
    pub paint: Rgba,
    pub words: Vec<WordRec>,
}

/// The side of a hint's glyphs beside the caption: 1.45 times the type.
pub fn hint_side(kit: &Kit) -> u32 {
    (kit.ty(TypeRole::Caption).px as f32 * 1.45).round() as u32
}

/// The space between the glyphs of a hint, between its keys and what they do, and between hints.
fn hint_gaps(kit: &Kit) -> (f32, f32, f32) {
    let ts = scale(kit);
    ((6.0 * ts).round(), (12.0 * ts).round(), (22.0 * ts).round())
}

fn key_names(h: &HintView) -> Vec<&'static str> {
    h.keys
        .iter()
        .map(|k| match (k, h.held) {
            (Key::Ok, true) => "key-ok-hold",
            (k, _) => k.glyph(),
        })
        .collect()
}

/// The width a row of hints takes.
pub fn hints_width(kit: &Kit, hints: &[HintView]) -> f32 {
    let side = hint_side(kit) as f32;
    let (g_key, g_do, g_hint) = hint_gaps(kit);
    let mut w = 0.0;
    for (i, h) in hints.iter().enumerate() {
        let n = h.keys.len() as f32 + 1.0;
        w += n * side + (h.keys.len() as f32 - 1.0).max(0.0) * g_key + g_do;
        if i + 1 < hints.len() {
            w += g_hint;
        }
    }
    w
}

/// Draws a row of hints from `x`, centred on `cy`: the keys in the reading ink, then what they do in the second ink.
pub fn draw_hints(kit: &mut Kit, hints: &[HintView], x: f32, cy: f32, z: i32, ink: &Ink, parts: &mut Parts) -> std::io::Result<()> {
    let side = hint_side(kit);
    let (g_key, g_do, g_hint) = hint_gaps(kit);
    let y = (cy - side as f32 / 2.0).round();
    let mut pen = x;
    for h in hints {
        for name in key_names(h) {
            let mut layer = kit.ui.scene.image(side, side, z, crate::system::parts::HIDDEN)?;
            glyph::draw(name, side, ink.ink, layer.pixels_mut());
            layer.damage(None);
            parts.image(layer, (pen, y));
            pen += side as f32 + g_key;
        }
        pen += g_do - g_key;
        match h.does {
            Do::Kit(action) => {
                let mut layer = kit.ui.scene.image(side, side, z, crate::system::parts::HIDDEN)?;
                glyph::draw(action.glyph(), side, ink.ink2, layer.pixels_mut());
                layer.damage(None);
                parts.image(layer, (pen, y));
            }
            Do::Icon(name) => {
                if let Some(layer) = icon::layer(kit, name, side, ink.ink2, z, crate::system::parts::HIDDEN)? {
                    parts.image(layer, (pen, y));
                }
            }
        }
        pen += side as f32 + g_hint;
    }
    Ok(())
}

fn text_w(kit: &mut Kit, role: TypeRole, weight: Option<Weight>, text: &str) -> f32 {
    let style = role_style(kit, role, weight, Rgba(0, 0, 0, 255), Align::Left);
    width_of(kit, style, text)
}

/// The pill's height at this text size: the target, or its tallest content with the padding where the type is large.
pub fn height(kit: &Kit, content_h: f32) -> f32 {
    let pad_v = (10.0 * (scale(kit) - 1.0)).round();
    kit.tokens.target.max(content_h + 2.0 * pad_v).ceil()
}

/// Builds a pill of `pieces` at `anchor`: its pane, and what is in it, hidden. `z` is the pane's depth; the pieces are above it.
pub fn build(kit: &mut Kit, name: &str, pieces: &[Piece], anchor: Anchor, z: i32, ink: &Ink) -> std::io::Result<(Parts, Drawn)> {
    let pad_x = kit.space(5);
    let gap = kit.space(2);
    // measure
    let mut widths: Vec<f32> = Vec::new();
    let mut content_h = 0.0f32;
    for p in pieces {
        let (w, h) = match p {
            Piece::Icon { base, .. } => {
                let s = icon_side(kit, *base) as f32;
                (s, s)
            }
            Piece::Text { text, role, weight, .. } => (text_w(kit, *role, *weight, text), kit.line(*role)),
            Piece::Dot(_) => (12.0, 12.0),
            Piece::Hints(h) => (hints_width(kit, h), hint_side(kit) as f32),
            Piece::Gap(g) => (*g, 0.0),
        };
        widths.push(w);
        content_h = content_h.max(h);
    }
    let gaps = pieces.iter().filter(|p| !matches!(p, Piece::Gap(_))).count().saturating_sub(1) as f32 * gap;
    let inner: f32 = widths.iter().sum::<f32>() + gaps;
    let w = (inner + 2.0 * pad_x).ceil();
    let h = height(kit, content_h);
    let (x, y) = match anchor {
        Anchor::TopLeft { x, y } => (x, y),
        Anchor::TopRight { right, y } => (right - w, y),
        Anchor::BottomLeft { x, bottom } => (x, bottom - h),
        Anchor::BottomRight { right, bottom } => (right - w, bottom - h),
        Anchor::BottomCentre { cx, bottom } => ((cx - w / 2.0).round(), bottom - h),
    };
    let frame = Frame::new(x, y, w, h);
    let mut parts = Parts::new();
    let pane = Pane::new(kit, frame, z, PaneKind::Pill, true);
    let paint = glass(kit, PaneKind::Pill, h, true).tint;
    parts.pane(pane);
    let cy = y + h / 2.0;
    let mut pen = x + pad_x;
    let mut words = Vec::new();
    let hidden = crate::system::parts::HIDDEN;
    for (p, pw) in pieces.iter().zip(&widths) {
        match p {
            Piece::Icon { name: icon_name, base, ink: colour } => {
                let side = icon_side(kit, *base);
                if let Some(layer) = icon::layer(kit, icon_name, side, *colour, z + 1, hidden)? {
                    parts.image(layer, (pen, (cy - side as f32 / 2.0).round()));
                }
            }
            Piece::Text { text, role, weight, ink: colour } => {
                let style = role_style(kit, *role, *weight, *colour, Align::Left);
                let line = kit.line(*role);
                let frame = Frame::new(pen, (cy - line / 2.0).round(), *pw, line);
                let t = Text::new(kit, frame, z + 1, style, text)?;
                parts.text(t);
                words.push(WordRec { name: format!("{name} {text}"), text: (*text).to_string(), frame, style });
            }
            Piece::Dot(colour) => {
                let shape = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 12.0, 12.0), z + 1, plate::solid(*colour, 6.0));
                parts.shape(shape, (pen, (cy - 6.0).round()));
            }
            Piece::Hints(h) => {
                draw_hints(kit, h, pen, cy, z + 1, ink, &mut parts)?;
            }
            Piece::Gap(_) => {}
        }
        pen += pw;
        if !matches!(p, Piece::Gap(_)) {
            pen += gap;
        }
    }
    Ok((parts, Drawn { frame, paint, words }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::view::HintView;
    use crate::kit::hint::Action;
    use crate::kit::lint::Lint;
    use crate::kit::testkit;

    #[test]
    fn a_pill_is_64_high_with_its_words_inside_at_100_percent_and_taller_at_200() {
        for (scale, min_h) in [(1.0f32, 64.0f32), (2.0, 90.0)] {
            let (_h, mut kit) = testkit::kit(scale);
            let ink = Ink::of(&kit);
            let pieces = [
                Piece::Icon { name: "video", base: 32.0, ink: ink.ink },
                Piece::Text { text: "Front Door", role: TypeRole::Body, weight: Some(Weight::SemiBold), ink: ink.ink },
                Piece::Dot(ink.live),
                Piece::Text { text: "Live", role: TypeRole::Body, weight: Some(Weight::SemiBold), ink: ink.ink },
                Piece::Text { text: "2 of 8", role: TypeRole::Label, weight: None, ink: ink.ink2 },
            ];
            let (parts, d) = build(&mut kit, "name", &pieces, Anchor::TopLeft { x: 96.0, y: 54.0 }, 10, &ink).expect("a pill");
            assert!(d.frame.h >= min_h, "{scale}: {}", d.frame.h);
            assert_eq!((d.frame.x, d.frame.y), (96.0, 54.0));
            assert_eq!(parts.layers(), 1 + 1 + 3 + 1, "the pane, the icon, three words and the dot");
            let mut lint = Lint::new();
            let panel = lint.panel("name pill", d.frame, d.paint, true);
            for w in &d.words {
                lint.word(&mut kit, &w.name, &w.text, w.frame, w.style, Some(panel));
            }
            assert!(lint.findings().is_empty(), "{scale}: {:?}", lint.findings());
        }
    }

    #[test]
    fn a_right_anchored_pill_ends_at_the_safe_edge_and_a_bottom_one_sits_on_it() {
        let (_h, mut kit) = testkit::kit(1.0);
        let ink = Ink::of(&kit);
        let pieces = [Piece::Text { text: "3840\u{d7}2160 \u{b7} H.265 \u{b7} 15 fps", role: TypeRole::Caption, weight: None, ink: ink.ink2 }];
        let (_p, d) = build(&mut kit, "info", &pieces, Anchor::TopRight { right: 1824.0, y: 54.0 }, 10, &ink).unwrap();
        assert!((d.frame.right() - 1824.0).abs() < 0.01, "{:?}", d.frame);
        let (_p, d) = build(&mut kit, "line", &pieces, Anchor::BottomLeft { x: 96.0, bottom: 1026.0 }, 10, &ink).unwrap();
        assert_eq!(d.frame.bottom(), 1026.0);
    }

    #[test]
    fn hints_are_glyphs_in_a_row_and_their_width_follows_the_text_size() {
        let hints = [
            HintView::new(&[Key::Left, Key::Right], Do::Icon("video"), "next camera"),
            HintView::new(&[Key::Ok], Do::Icon("layout-grid"), "grid"),
            HintView::new(&[Key::Back], Do::Kit(Action::Close), "close"),
        ];
        let (_h1, kit1) = testkit::kit(1.0);
        let (_h2, kit2) = testkit::kit(2.0);
        let (w1, w2) = (hints_width(&kit1, &hints), hints_width(&kit2, &hints));
        assert!(w1 > 300.0 && w1 < 500.0, "{w1}");
        assert!((w2 / w1 - 2.0).abs() < 0.15, "{w1} -> {w2}");
        assert_eq!(hint_side(&kit1), 35, "1.45 times the caption");
    }

    #[test]
    fn the_hints_pill_draws_a_glyph_for_each_key_and_each_action() {
        let (_h, mut kit) = testkit::kit(1.0);
        let ink = Ink::of(&kit);
        let hints = [HintView::new(&[Key::Left, Key::Right], Do::Icon("video"), "x"), HintView::new(&[Key::Back], Do::Kit(Action::Close), "y")];
        let (parts, d) = build(&mut kit, "hints", &[Piece::Hints(&hints)], Anchor::BottomRight { right: 1824.0, bottom: 1026.0 }, 10, &ink).unwrap();
        assert_eq!(parts.layers(), 1 + 3 + 2, "the pane, left right video, back close");
        assert!(d.words.is_empty());
        assert_eq!(d.frame.right(), 1824.0);
    }
}
