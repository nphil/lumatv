//! The prompt over a running stream while the far side's idle countdown runs. It is on screen only for that countdown (about a minute),
//! never otherwise: a veil as dark as the stats strip's, in the top centre (below the stats strip when that shows), whose only change is
//! the countdown's digits once a second.
use super::{
    paint::{Fill, HUD, Mark, Paint, W},
    parts::drawer,
};
use lumatv_tokens::{Radius, TypeRole};
use ui::{components::Form, geometry::Frame, lucent::palette::PaletteRole as Role};

/// Layers of the prompt: below 100, so the page's top-edge cull (it drops what ends above y = 150) leaves them.
pub(super) const SURFACE: u64 = 90;
const WORDS: u64 = 91;

pub fn show(p: &mut Paint<'_>) {
    let Some(left) = p.model.idle_left() else { return };
    let scale = p.scale;
    let (pad_x, pad_y, dot) = (28.0 * scale, 16.0 * scale, 12.0);
    let (title, label) = (p.line(TypeRole::Title), p.line(TypeRole::Label));
    let ink = p.ink(Role::Ink, true);
    let mut words = Vec::new();
    let x = pad_x + dot + 14.0 * scale;
    let mut head = p.word("Still playing?", TypeRole::Title, ink, x, pad_y, 0.0);
    head.ty = p.bold(TypeRole::Title);
    head.w = p.fonts.measure(&head.text, head.ty).width.ceil();
    let mut at = x + head.w + 20.0 * scale;
    words.push(head);
    // The smaller words sit on the middle of the headline's line, so the row reads as one.
    let ends = "Session ends in";
    let w = p.width(ends, TypeRole::Label);
    words.push(p.word(ends, TypeRole::Label, p.ink(Role::Ink2, true), at, pad_y + (title - label) / 2.0, w));
    at += w + 10.0 * scale;
    // Tabular digits, so the prompt does not change width as the seconds fall.
    let (digits, width) = p.figures(&format!("{}:{:02}", left / 60, left % 60), p.bold(TypeRole::Title), ink, at, pad_y);
    words.extend(digits);
    at += width;
    let hint = "Press any button to keep playing";
    let w = p.width(hint, TypeRole::Label);
    words.push(p.word(hint, TypeRole::Label, ink, x, pad_y + title + 6.0 * scale, w));
    let frame_w = (at.max(x + w) + pad_x).min(W - p.x * 2.0);
    let frame_h = pad_y * 2.0 + title + 6.0 * scale + label;
    let marks = vec![Mark { form: Form::Disc(12), colour: p.ink(Role::Warning, true), x: pad_x, y: pad_y + (title - dot) / 2.0 }];
    let top = p.plan.nodes.iter().find(|n| n.id == HUD.start).map_or(p.y, |hud| hud.frame.y + hud.frame.h + 14.0 * scale);
    // An open drawer keeps its side of the screen; the prompt centres in the room beside it.
    let (from, to) = match p.plan.nodes.iter().find(|n| n.id == drawer::SHEET) {
        Some(sheet) if sheet.frame.x > W / 2.0 => (0.0, sheet.frame.x),
        Some(sheet) => (sheet.frame.x + sheet.frame.w, W),
        None => (0.0, W),
    };
    let frame = Frame::new(from + (to - from - frame_w) / 2.0, top, frame_w, frame_h);
    p.surface(SURFACE, frame, Fill::Veil(0.80), Radius::Card, true, false, false, 22);
    p.words(WORDS, frame, words, marks, 23);
    #[cfg(test)]
    super::paint::assert_readable(&p.plan, p.fonts, true);
}
