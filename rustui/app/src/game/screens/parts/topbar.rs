//! The top bar and the note of a running library refresh under it.
use crate::game::screens::paint::{Fill, Paint, W};
use crate::game::model::{Focus, Page};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};

pub fn nav(p: &mut Paint<'_>, stage: bool) -> f32 {
    let labels = [("Home", Icon::Home), ("Library", Icon::Cards), ("Search", Icon::Search)];
    let side = 30.0 * p.scale;
    let mut widths = Vec::new();
    for (label, _) in labels {
        widths.push(p.width(label, Type::Label) + side + 66.0);
    }
    let w = widths.iter().sum::<f32>() + 12.0;
    let h = p.tokens.target.max(p.line(Type::Label) + 12.0) + 12.0;
    let stage = stage && p.mode == Mode::Dark;
    p.surface(10, Frame::new(p.x, p.y, w, h), if p.mode == Mode::Light { Fill::Sheet } else { Fill::Glass }, Radius::Pill, stage, false, true, 60);
    let current = match p.model.view.page {
        Page::Home => Some(0),
        Page::Library => Some(1),
        Page::Search => Some(2),
        _ => None,
    };
    let mut x = p.x + 6.0;
    for (i, (label, icon)) in labels.into_iter().enumerate() {
        let from = p.plan.nodes.len();
        let f = Frame::new(x, p.y + 6.0, widths[i], h - 12.0);
        let focused = p.focused(Focus::Nav(i));
        if focused || current == Some(i) {
            p.surface(11 + i as u64 * 4, f, Fill::Tint(if focused { 0.16 } else { 0.10 }), Radius::Pill, stage, false, false, 61);
        }
        let marks = if i == 1 { Vec::new() } else { vec![p.mark(icon, side, 24.0, (f.h - side) / 2.0, Role::Ink, stage)] };
        p.words(
            12 + i as u64 * 4,
            f,
            vec![p.word(label, Type::Label, p.ink(Role::Ink, stage), side + 36.0, (f.h - p.line(Type::Label)) / 2.0, f.w - side - 48.0)],
            marks,
            62,
        );
        if i == 1 {
            p.symbol(
                14 + i as u64 * 4,
                crate::game::screens::symbols::Symbol::Library,
                Frame::new(x + 24.0, f.y + (f.h - side) / 2.0, side, side),
                p.ink(Role::Ink, stage),
                62,
            );
        }
        if focused {
            p.zoom(from, f, p.tokens.focus.button_scale);
        }
        p.hit(Focus::Nav(i), f);
        x += widths[i];
    }
    let f = Frame::new(W - p.x - 144.0, p.y, 144.0, h);
    p.surface(30, f, if p.mode == Mode::Light { Fill::Sheet } else { Fill::Glass }, Radius::Pill, stage, false, true, 60);
    p.solid(31, Frame::new(f.x + 16.0, f.y + (h - 44.0) / 2.0, 44.0, 44.0), p.ink(Role::Accent, stage), 22.0, 61);
    let initial = p.model.state().profile().name.chars().next().unwrap_or('N').to_string();
    let mut word = p.word(initial, Type::Caption, p.ink(Role::AccentInk, stage), 16.0, (h - p.line(Type::Caption)) / 2.0, 44.0);
    word.align = ui::components::Align::Centre;
    p.words(32, f, vec![word], vec![], 62);
    let target = Frame::new(f.x + 74.0, f.y + 6.0, 64.0, h - 12.0);
    let from = p.plan.nodes.len();
    if p.focused(Focus::Nav(3)) {
        p.surface(33, target, Fill::Tint(0.16), Radius::Pill, stage, false, false, 61);
    }
    p.words(
        34,
        target,
        vec![],
        vec![p.mark(Icon::Cog, 30.0 * p.scale.min(1.5), (64.0 - 30.0 * p.scale.min(1.5)) / 2.0, (target.h - 30.0 * p.scale.min(1.5)) / 2.0, Role::Ink, stage)],
        62,
    );
    if p.focused(Focus::Nav(3)) {
        p.zoom(from, target, p.tokens.focus.button_scale);
    }
    p.hit(Focus::Nav(3), target);
    p.x + w
}
/// What a refresh is doing or found: a pill between the navigation and the avatar. Progress counts seconds once it is slow, a result
/// stays a few seconds and goes (the model clears it), so nothing bright keeps its place on the panel. The mark and the words carry the
/// meaning, the colour only backs them.
pub fn refresh_note(p: &mut Paint<'_>, id: u64, left: f32, y: f32) {
    use crate::game::model::RefreshLine;
    let Some(line) = p.model.refresh_line() else { return };
    let (text, icon, role) = match line {
        RefreshLine::Working { text, seconds: Some(s) } => (format!("{text}…  {s} s"), Icon::Hourglass, Role::Ink2),
        RefreshLine::Working { text, seconds: None } => (format!("{text}…"), Icon::Hourglass, Role::Ink2),
        RefreshLine::Done { text, problem: false } => (text, Icon::Check, Role::Positive),
        RefreshLine::Done { text, problem: true } => (text, Icon::Alert, Role::Warning),
    };
    let x = left + 24.0;
    let avatar = W - p.x - 144.0 - 24.0;
    let side = 26.0 * p.scale;
    let text_w = (avatar - x - side - 52.0).max(200.0);
    let lines = p.fit(&text, Type::Caption, text_w, 3);
    let h = (lines.len() as f32 * p.line(Type::Caption) + 16.0).max(44.0);
    let widest = lines.iter().map(|l| p.width(l, Type::Caption)).fold(0.0, f32::max);
    let w = widest.ceil() + side + 44.0;
    let f = Frame::new(x, (y + (p.tokens.target.max(p.line(Type::Label) + 12.0) + 12.0 - h) / 2.0).max(24.0), w, h);
    p.surface(id, f, Fill::Sheet, Radius::Pill, false, false, false, 60);
    p.words(id + 1, f, vec![], vec![p.mark(icon, side, 12.0, (h - side) / 2.0, role, false)], 61);
    p.lines(id + 2, lines, Frame::new(f.x + side + 26.0, f.y + 8.0, widest.ceil() + 2.0, h - 16.0), Type::Caption, Role::Ink, false, 61);
}
