//! The refusal's words (spec "Words of a refusal"): one line in a pill over the screen it came from, for 3 s; the picture goes on as it was.
//! The pill sits at the bottom centre, above the strip when the strip is up.

use super::common::{Ink, Parts};
use super::grid::WordRec;
use super::pill::{self, Anchor, Piece};
use super::{Cx, depth};
use compositor_protocol::Rgba;
use lumatv_tokens::TypeRole;
use ui::geometry::Frame;

pub struct ToastUi {
    pub text: String,
    pub above: Option<f32>,
    parts: Parts,
    frame: Frame,
    paint: Rgba,
    words: Vec<WordRec>,
}

impl ToastUi {
    /// The pill for `text`; `above` is the top of what it must clear (the strip).
    pub fn build(cx: &mut Cx, text: &str, above: Option<f32>) -> std::io::Result<Self> {
        let ink = Ink::of(cx.kit);
        let bottom = above.map_or(950.0, |y| y - 24.0);
        let pieces = [Piece::Text { text, role: TypeRole::Label, weight: None, ink: ink.ink }];
        let (mut parts, drawn) = pill::build(cx.kit, "toast", &pieces, Anchor::BottomCentre { cx: 960.0, bottom }, cx.at(depth::TOAST), &ink)?;
        parts.fade(cx.kit, true, super::full::FADE_IN_MS);
        Ok(Self {
            text: text.to_string(),
            above,
            parts,
            frame: drawn.frame,
            paint: drawn.paint,
            words: drawn.words.into_iter().map(|w| WordRec { name: w.name, text: w.text, frame: w.frame, style: w.style }).collect(),
        })
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn layers(&self) -> usize {
        self.parts.layers()
    }

    pub fn retire(&mut self, cx: &mut Cx, ms: u32) {
        self.parts.fade(cx.kit, false, ms);
    }

    pub fn feed_lint(&self, kit: &mut crate::kit::Kit, lint: &mut crate::kit::lint::Lint) {
        let panel = lint.panel("toast", self.frame, self.paint, true);
        for w in &self.words {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::surface::pic::PictureStore;
    use crate::kit::lint::Lint;
    use crate::kit::testkit;

    #[test]
    fn the_toast_is_a_centred_pill_above_the_strip_and_reads_clean() {
        for scale in [1.0f32, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            let mut engine = crate::kit::Engine::new(&kit).unwrap();
            let store = PictureStore::new();
            let mut cx = Cx { kit: &mut kit, engine: &mut engine, store: &store, z: 2000, now_ms: 0, now_ns: 0 };
            let t = ToastUi::build(&mut cx, "No recording at that time", Some(700.0)).unwrap();
            let f = t.frame();
            assert!(((f.x + f.w / 2.0) - 960.0).abs() < 1.0, "{f:?}");
            assert!(f.bottom() <= 676.1, "above the strip: {f:?}");
            let mut lint = Lint::new();
            t.feed_lint(&mut kit, &mut lint);
            assert!(lint.findings().is_empty(), "{scale}: {:?}", lint.findings());
            let plain = ToastUi::build(&mut Cx { kit: &mut kit, engine: &mut engine, store: &store, z: 2000, now_ms: 0, now_ns: 0 }, "The camera is offline", None).unwrap();
            assert_eq!(plain.frame().bottom(), 950.0);
        }
    }
}
