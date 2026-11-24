//! What the recorder says over a running stream. The confirmation of a clip is one sentence on a veil as dark as the stats strip's,
//! top centre under the stats strip and the idle prompt when those show. It stays 4 s, comes and goes with a fade and never moves;
//! nothing else on it changes. The mark of a running recording is a dot and a word on the same veil, top left.
use super::{
    paint::{Fill, HUD, Mark, Paint, W},
    parts::{drawer, motion::Enter},
};
use lumatv_tokens::{Radius, TypeRole};
use ui::{components::Form, geometry::Frame, lucent::palette::PaletteRole as Role};

/// Layers of the confirmation and of the recording mark: below 100, so the page's top-edge cull (it drops what ends above y = 150)
/// leaves them.
const SURFACE: u64 = 92;
const WORDS: u64 = 93;
const REC_SURFACE: u64 = 94;
const REC_WORDS: u64 = 95;
/// The recording mark's four places are this far apart: right, down, left and back.
const REC_STEP: f32 = 24.0;

/// The mark of a running recording. The model shows it for 4 s of each minute, one place on every time, so it never rests on the
/// same pixels; the confirmation has the top of the screen to itself while it is up.
pub fn recording(p: &mut Paint<'_>) {
    let Some(place) = p.model.rec_mark().filter(|_| p.model.clip_note().is_none()) else { return };
    let scale = p.scale;
    let (pad_x, pad_y, dot) = (20.0 * scale, 10.0 * scale, 12.0);
    let x = pad_x + dot + 12.0 * scale;
    let text_w = p.width("Recording", TypeRole::Label);
    let words = vec![p.word("Recording", TypeRole::Label, p.ink(Role::Ink, true), x, pad_y, text_w)];
    let marks = vec![Mark { form: Form::Disc(12), colour: p.ink(Role::Danger, true), x: pad_x, y: pad_y + (p.line(TypeRole::Label) - dot) / 2.0 }];
    let (dx, dy) = [(0.0, 0.0), (REC_STEP, 0.0), (REC_STEP, REC_STEP), (0.0, REC_STEP)][usize::from(place)];
    let frame = Frame::new(p.x + dx, p.y + dy, x + text_w + pad_x, pad_y * 2.0 + p.line(TypeRole::Label));
    p.arrives(frame, Enter::FADE);
    p.surface(REC_SURFACE, frame, Fill::Veil(0.80), Radius::Pill, true, false, false, 22);
    p.words(REC_WORDS, frame, words, marks, 23);
    p.finish();
    #[cfg(test)]
    super::paint::assert_readable(&p.plan, p.fonts, true);
}

pub fn show(p: &mut Paint<'_>) {
    let Some(note) = p.model.clip_note() else { return };
    let scale = p.scale;
    let (pad_x, pad_y, dot) = (24.0 * scale, 14.0 * scale, 12.0);
    let ink = p.ink(Role::Ink, true);
    let x = pad_x + dot + 14.0 * scale;
    let text_w = p.width(&note.text, TypeRole::Body);
    let words = vec![p.word(&note.text, TypeRole::Body, ink, x, pad_y, text_w)];
    let marks = vec![Mark {
        form: Form::Disc(12),
        colour: p.ink(if note.saved { Role::Positive } else { Role::Warning }, true),
        x: pad_x,
        y: pad_y + (p.line(TypeRole::Body) - dot) / 2.0,
    }];
    let frame_w = (x + text_w + pad_x).min(W - p.x * 2.0);
    let frame_h = pad_y * 2.0 + p.line(TypeRole::Body);
    let below = |p: &Paint<'_>, id: u64| p.plan.nodes.iter().find(|n| n.id == id).map(|n| n.frame.y + n.frame.h + 14.0 * scale);
    let top = [below(p, HUD.start), below(p, super::idle::SURFACE)].into_iter().flatten().fold(p.y, f32::max);
    let (from, to) = match p.plan.nodes.iter().find(|n| n.id == drawer::SHEET) {
        Some(sheet) if sheet.frame.x > W / 2.0 => (0.0, sheet.frame.x),
        Some(sheet) => (sheet.frame.x + sheet.frame.w, W),
        None => (0.0, W),
    };
    let frame = Frame::new(from + (to - from - frame_w) / 2.0, top, frame_w, frame_h);
    p.arrives(frame, Enter::FADE);
    p.surface(SURFACE, frame, Fill::Veil(0.80), Radius::Card, true, false, false, 22);
    p.words(WORDS, frame, words, marks, 23);
    p.finish();
    #[cfg(test)]
    super::paint::assert_readable(&p.plan, p.fonts, true);
}
