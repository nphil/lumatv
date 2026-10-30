//! The TV motion set (`ten-foot-motion.md`, Lucent section 8), from the theme's `motion` block through [`Kit`]: the one set of
//! durations every screen uses, reduced motion applied once here. Layers arrive with `Ease::Out` and leave with `Ease::In`; a
//! focus move is the flowing spring of [`super::flow`], not a timed curve.

use super::Kit;
use compositor_protocol::{Ease, Props};
use ui::scene::Layer;

/// Which of the theme's durations a change takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beat {
    /// A press: 90 ms.
    Press,
    /// A label or a value that changes: 120 ms.
    Label,
    /// The focus highlight: 150 ms.
    Focus,
    /// A card's lift or a bar's fill: 180 ms.
    Card,
    /// A layer that opens or closes: 220 ms.
    Layer,
    /// A scroll: 300 ms.
    Scroll,
    /// The rest after which a focus is settled (the name after the lift): 150 ms.
    Rest,
}

impl Kit {
    /// The duration of `beat` with reduced motion applied.
    pub fn beat_ms(&self, beat: Beat) -> u32 {
        let m = &self.tokens.motion;
        let ms = match beat {
            Beat::Press => m.press_ms,
            Beat::Label => m.label_ms,
            Beat::Focus => m.focus_ms,
            Beat::Card => m.card_ms,
            Beat::Layer => m.layer_ms,
            Beat::Scroll => m.scroll_ms,
            Beat::Rest => m.rest_ms,
        };
        self.ui.look.duration_ms(ms)
    }

    /// Shows `layer` at `to`: at once when `at_once` (a volume key's bar appears on its own frame), else fading in over `beat`.
    pub fn arrive(&self, layer: &Layer, to: Props, beat: Beat, at_once: bool) {
        if at_once || self.beat_ms(beat) == 0 {
            layer.set(to);
        } else {
            layer.set(Props { opacity: 0.0, ..to });
            layer.animate(to, self.beat_ms(beat), Ease::Out);
        }
    }

    /// Shows `layer` at `to`, coming in from `by` away from it (the edge it rests against) while it fades in over `beat`.
    pub fn arrive_from(&self, layer: &Layer, to: Props, by: (f32, f32), beat: Beat) {
        if self.beat_ms(beat) == 0 {
            layer.set(to);
        } else {
            layer.set(Props { x: to.x + by.0, y: to.y + by.1, opacity: 0.0, ..to });
            layer.animate(to, self.beat_ms(beat), Ease::Out);
        }
    }

    /// Takes `layer` away from `from`: fades out over `beat` and is hidden at the end (the compositor hides a layer whose animation
    /// ends invisible).
    pub fn leave(&self, layer: &Layer, from: Props, beat: Beat) {
        let gone = Props { opacity: 0.0, visible: false, ..from };
        if self.beat_ms(beat) == 0 {
            layer.set(gone);
        } else {
            layer.animate(gone, self.beat_ms(beat), Ease::In);
        }
    }

    /// Moves `layer` to `to` over `beat` (a bar that glides when the app under it comes or goes).
    pub fn glide(&self, layer: &Layer, to: Props, beat: Beat) {
        if self.beat_ms(beat) == 0 {
            layer.set(to);
        } else {
            layer.animate(to, self.beat_ms(beat), Ease::Out);
        }
    }
}
