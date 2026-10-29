//! The layout and contrast lint of the TV kit: the rules `tests/qmlhost/layoutaudit.py` holds the QML surfaces to, as a pure check on
//! what a surface says it drew, run from host tests at 100 % and 200 % text (`docs/research/rust-shell-kit.md`, "The lint").
//!
//! A surface describes its view to a [`Lint`]: the panels (a pane, a card, a pill, a button: a rectangle with the colour it paints) and the
//! words (a text's painted rectangle, its ink, the panel it sits in). [`Lint::findings`] then reports, per fault:
//!
//! - `Outside`: words leave their panel by more than a pixel, or the window;
//! - `Tight`: words closer than 8 px (`space.2`) to the panel's sides or 4 px to its top and bottom;
//! - `Overlap`: two runs of words on top of each other;
//! - `OffCentre`: words in a panel that holds one run (a button, a pill; up to 160 px tall) not centred in it to within 2.5 px;
//! - `Safe`: a panel outside the safe area its surface is held to;
//! - `Contrast`: the ink against what is behind it: 4.5 to 1 under 48 px, 3 to 1 from 48 px up and for muted ink (ink2, a dimmed
//!   row), the panel's paint over the worst of black, white and mid grey (a pane over video lets the picture through down to the
//!   reading alpha, so the backdrop is not known).
//!
//! The pixel check of the finished picture stays with the Python host tests (`tests/test_rust_system.py` reads the renders); this is
//! the check that needs no picture and runs in `cargo test`.

use super::Kit;
use compositor_protocol::Rgba;
use ui::components::{Align, Style};
use ui::geometry::Frame;

/// Air kept between words and the sides of the panel they are in (Lucent `space.2`), and above and below them.
pub const PAD_X: f32 = 8.0;
pub const PAD_Y: f32 = 4.0;
/// How far words may leave a panel or the window.
pub const TOL: f32 = 1.0;
/// How far the centre of words in a button may be from the button's.
pub const CENTRE: f32 = 2.5;
/// Panels taller than this hold a layout, not one run of words.
pub const CENTRED_MAX_H: f32 = 160.0;
/// Text from this size up reads at the lower ratio.
pub const LARGE_PX: u32 = 48;
pub const CONTRAST_TEXT: f32 = 4.5;
pub const CONTRAST_LARGE: f32 = 3.0;
pub const CONTRAST_MUTED: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Outside,
    Tight,
    Overlap,
    OffCentre,
    Safe,
    Contrast,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Outside => "outside",
            Kind::Tight => "tight",
            Kind::Overlap => "overlap",
            Kind::OffCentre => "off-centre",
            Kind::Safe => "safe",
            Kind::Contrast => "contrast",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    pub subject: String,
    pub detail: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} ({})", self.kind.name(), self.subject, self.detail)
    }
}

/// A rectangle with the colour it paints: a pane, a card, a pill, a button.
#[derive(Clone, Debug)]
pub struct Panel {
    pub name: String,
    pub rect: Frame,
    /// What it paints, premultiplied; fully clear for a panel that only groups words.
    pub paint: Rgba,
    /// The panel is held to the safe area (a toast's corner is its own zone: false).
    pub safe: bool,
}

/// One run of words as drawn.
#[derive(Clone, Debug)]
pub struct Word {
    pub name: String,
    /// The painted rectangle: the measured width by the line's height, at the text's alignment inside its box.
    pub rect: Frame,
    pub ink: Rgba,
    pub px: u32,
    /// Ink that is muted by design (ink2, a dimmed row).
    pub muted: bool,
    pub panel: Option<usize>,
}

#[derive(Debug)]
pub struct Lint {
    screen: Frame,
    safe: Frame,
    panels: Vec<Panel>,
    words: Vec<Word>,
}

impl Default for Lint {
    fn default() -> Self {
        Self::new()
    }
}

/// The relative luminance of an opaque colour (WCAG).
fn luminance(c: [f32; 3]) -> f32 {
    let lin = |v: f32| {
        let v = v / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// `over` composited on the opaque `under`, both premultiplied.
fn composite(over: Rgba, under: [f32; 3]) -> [f32; 3] {
    let k = 1.0 - f32::from(over.3) / 255.0;
    [f32::from(over.0) + under[0] * k, f32::from(over.1) + under[1] * k, f32::from(over.2) + under[2] * k]
}

/// The contrast ratio of `ink` on `paint` laid over `backdrop`.
pub fn contrast_on(ink: Rgba, paint: Rgba, backdrop: [f32; 3]) -> f32 {
    let bg = composite(paint, backdrop);
    let fg = composite(ink, bg);
    let (a, b) = (luminance(fg), luminance(bg));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The least contrast of `ink` on `paint` over the worst of black, white and mid grey.
pub fn worst_contrast(ink: Rgba, paint: Rgba) -> f32 {
    [[0.0; 3], [255.0; 3], [128.0; 3]].into_iter().map(|b| contrast_on(ink, paint, b)).fold(f32::MAX, f32::min)
}

fn overlap_area(a: &Frame, b: &Frame) -> f32 {
    let w = a.right().min(b.right()) - a.x.max(b.x);
    let h = a.bottom().min(b.bottom()) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

impl Lint {
    /// A lint of the whole 1920 by 1080 screen, held to Home's safe area (64 and 54, `layout.json`).
    pub fn new() -> Self {
        Self { screen: Frame::new(0.0, 0.0, 1920.0, 1080.0), safe: Frame::new(64.0, 54.0, 1920.0 - 128.0, 1080.0 - 108.0), panels: Vec::new(), words: Vec::new() }
    }

    /// The safe area panels marked `safe` are held to.
    pub fn with_safe(mut self, safe: Frame) -> Self {
        self.safe = safe;
        self
    }

    pub fn panel(&mut self, name: &str, rect: Frame, paint: Rgba, safe: bool) -> usize {
        self.panels.push(Panel { name: name.to_string(), rect, paint, safe });
        self.panels.len() - 1
    }

    /// A run of words whose painted rectangle is known.
    pub fn word_at(&mut self, name: &str, rect: Frame, ink: Rgba, px: u32, muted: bool, panel: Option<usize>) {
        self.words.push(Word { name: name.to_string(), rect, ink, px, muted, panel });
    }

    /// A run of `text` in `style` drawn in a box `frame` wide (the box the text layer has): its painted rectangle is the measured width
    /// (cut to the box) by the line height, at the style's alignment; vertically it is the box's (a layer is one line high).
    pub fn word(&mut self, kit: &mut Kit, name: &str, text: &str, frame: Frame, style: Style, panel: Option<usize>) {
        let w = kit.ui.fonts.measure(text, style.ty).width.ceil();
        let shown = w.min(frame.w);
        let x = match style.align {
            Align::Left => frame.x,
            Align::Centre => frame.x + (frame.w - shown) / 2.0,
            Align::Right => frame.right() - shown,
        };
        let line = ui::components::line_height(style.ty).min(frame.h.max(1.0));
        let y = frame.y + (frame.h - line) / 2.0;
        // ink that is not the full reading ink (ink2, ink3, a dimmed row) is muted by design
        let muted = style.colour != kit.ui.look.ink() || style.colour.3 < 230;
        self.word_at(name, Frame::new(x, y, shown, line), style.colour, style.ty.px, muted, panel);
    }

    pub fn words(&self) -> &[Word] {
        &self.words
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    /// Every fault found, in the order of the rules above.
    pub fn findings(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        let finding = |kind: Kind, subject: &str, detail: String| Finding { kind, subject: subject.to_string(), detail };
        for p in &self.panels {
            if p.safe && !(p.rect.x >= self.safe.x - TOL && p.rect.y >= self.safe.y - TOL && p.rect.right() <= self.safe.right() + TOL && p.rect.bottom() <= self.safe.bottom() + TOL) {
                out.push(finding(Kind::Safe, &p.name, format!("{:?} is not inside the safe area {:?}", rect_text(&p.rect), rect_text(&self.safe))));
            }
        }
        for w in &self.words {
            let r = &w.rect;
            if r.x < self.screen.x - TOL || r.y < self.screen.y - TOL || r.right() > self.screen.right() + TOL || r.bottom() > self.screen.bottom() + TOL {
                out.push(finding(Kind::Outside, &w.name, "leaves the screen".to_string()));
            }
            let Some(p) = w.panel.and_then(|i| self.panels.get(i)) else { continue };
            let b = &p.rect;
            if r.x < b.x - TOL || r.y < b.y - TOL || r.right() > b.right() + TOL || r.bottom() > b.bottom() + TOL {
                out.push(finding(Kind::Outside, &w.name, format!("{:?} leaves {} {:?}", rect_text(r), p.name, rect_text(b))));
            } else if r.x < b.x + PAD_X - TOL || r.right() > b.right() - PAD_X + TOL || r.y < b.y + PAD_Y - TOL || r.bottom() > b.bottom() - PAD_Y + TOL {
                out.push(finding(Kind::Tight, &w.name, format!("{:?} is crammed against {} {:?}", rect_text(r), p.name, rect_text(b))));
            }
            let alone = self.words.iter().filter(|o| o.panel == w.panel).count() == 1;
            if alone && b.h <= CENTRED_MAX_H && p.paint.3 > 0 {
                let (wc, bc) = (r.centre(), b.centre());
                if (wc.0 - bc.0).abs() > CENTRE && (r.x - b.x - PAD_X).abs() > CENTRE && ((b.right() - PAD_X) - r.right()).abs() > CENTRE || (wc.1 - bc.1).abs() > CENTRE {
                    out.push(finding(Kind::OffCentre, &w.name, format!("centre {:?} against {} {:?}", wc, p.name, bc)));
                }
            }
        }
        for (i, a) in self.words.iter().enumerate() {
            for b in &self.words[i + 1..] {
                let area = overlap_area(&a.rect, &b.rect);
                if area > 1.0 {
                    out.push(finding(Kind::Overlap, &a.name, format!("on top of {} ({area:.0} px\u{b2})", b.name)));
                }
            }
        }
        for w in &self.words {
            // words with no panel are on the bare screen: nothing is known of what is behind them
            let Some(paint) = w.panel.and_then(|i| self.panels.get(i)).map(|p| p.paint) else { continue };
            let need = if w.muted { CONTRAST_MUTED } else if w.px >= LARGE_PX { CONTRAST_LARGE } else { CONTRAST_TEXT };
            let got = worst_contrast(w.ink, paint);
            if got + 0.005 < need {
                out.push(finding(Kind::Contrast, &w.name, format!("{got:.2}:1 on {} where {need}:1 is needed", w.panel.and_then(|i| self.panels.get(i)).map_or("the screen", |p| p.name.as_str()))));
            }
        }
        out
    }
}

fn rect_text(f: &Frame) -> (i32, i32, i32, i32) {
    (f.x.round() as i32, f.y.round() as i32, f.w.round() as i32, f.h.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgba = Rgba(240, 240, 245, 255);

    fn lint_with(rect: Frame, word: Frame, paint: Rgba, ink: Rgba, px: u32) -> Vec<Finding> {
        let mut l = Lint::new();
        let p = l.panel("pill", rect, paint, true);
        l.word_at("label", word, ink, px, false, Some(p));
        l.findings()
    }

    #[test]
    fn words_inside_with_air_and_centred_read_clean() {
        let f = lint_with(Frame::new(200.0, 200.0, 240.0, 64.0), Frame::new(260.0, 218.0, 120.0, 28.0), Rgba(30, 30, 40, 255), INK, 26);
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn words_that_leave_or_crowd_their_panel_are_found() {
        let out = lint_with(Frame::new(200.0, 200.0, 240.0, 64.0), Frame::new(190.0, 218.0, 120.0, 28.0), Rgba(30, 30, 40, 255), INK, 26);
        assert!(out.iter().any(|f| f.kind == Kind::Outside), "{out:?}");
        let tight = lint_with(Frame::new(200.0, 200.0, 240.0, 64.0), Frame::new(204.0, 218.0, 120.0, 28.0), Rgba(30, 30, 40, 255), INK, 26);
        assert!(tight.iter().any(|f| f.kind == Kind::Tight), "{tight:?}");
    }

    #[test]
    fn two_runs_on_top_of_each_other_are_found_and_side_by_side_are_not() {
        let mut l = Lint::new();
        l.word_at("a", Frame::new(100.0, 100.0, 200.0, 30.0), INK, 26, false, None);
        l.word_at("b", Frame::new(250.0, 110.0, 200.0, 30.0), INK, 26, false, None);
        l.word_at("c", Frame::new(450.0, 100.0, 100.0, 30.0), INK, 26, false, None);
        let out = l.findings();
        assert!(out.iter().any(|f| f.kind == Kind::Overlap && f.subject == "a" && f.detail.contains("on top of b")), "{out:?}");
        assert!(!out.iter().any(|f| f.detail.contains("on top of c")), "b ends where c begins: {out:?}");
    }

    #[test]
    fn a_button_whose_words_sit_to_one_side_is_off_centre() {
        let out = lint_with(Frame::new(200.0, 200.0, 300.0, 64.0), Frame::new(230.0, 218.0, 120.0, 28.0), Rgba(30, 30, 40, 255), INK, 26);
        assert!(out.iter().any(|f| f.kind == Kind::OffCentre), "{out:?}");
    }

    #[test]
    fn contrast_is_held_to_the_worst_backdrop_and_the_size_of_the_text() {
        // light ink on a glass that is half clear: over white it does not read
        let weak = lint_with(Frame::new(200.0, 200.0, 240.0, 64.0), Frame::new(260.0, 218.0, 120.0, 28.0), Rgba(40, 40, 50, 128), INK, 26);
        assert!(weak.iter().any(|f| f.kind == Kind::Contrast), "{weak:?}");
        // the same on an opaque sheet reads
        let solid = lint_with(Frame::new(200.0, 200.0, 240.0, 64.0), Frame::new(260.0, 218.0, 120.0, 28.0), Rgba(40, 40, 50, 255), INK, 26);
        assert!(solid.is_empty(), "{solid:?}");
        assert!(contrast_on(INK, Rgba(0, 0, 0, 255), [255.0; 3]) > 15.0);
        assert!((worst_contrast(Rgba(255, 255, 255, 255), Rgba(255, 255, 255, 255)) - 1.0).abs() < 0.01);
    }

    #[test]
    fn a_panel_outside_the_safe_area_is_found_unless_it_is_not_held_to_it() {
        let mut l = Lint::new();
        l.panel("card", Frame::new(1400.0, 24.0, 560.0, 98.0), Rgba(30, 30, 40, 255), true);
        l.panel("toast", Frame::new(1336.0, 24.0, 560.0, 98.0), Rgba(30, 30, 40, 255), false);
        let out = l.findings();
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind, Kind::Safe);
    }
}
