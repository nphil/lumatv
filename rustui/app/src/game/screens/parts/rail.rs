//! The Settings rail.
use crate::game::screens::paint::{Fill, Paint};
use crate::game::model::{Focus, settings::Place};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

pub fn rail(p: &mut Paint<'_>) -> f32 {
    let icons = [Icon::Palette, Icon::Cards, Icon::Puzzle, Icon::Apps, Icon::Info, Icon::Info];
    let side = 36.0 * p.scale;
    let mut w = 420.0f32;
    for section in Place::ALL {
        w = w.max(p.width(section.name(), Type::Body) + side + 80.0);
    }
    w = w.min(640.0);
    p.text(100, "Settings", Frame::new(p.x + 28.0, 170.0, w - 56.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
    let top = 170.0 + p.line(Type::Title) + 24.0;
    let h = 80.0f32.max(p.line(Type::Body) + 24.0);
    for (i, section) in Place::ALL.into_iter().enumerate() {
        let f = Frame::new(p.x, top + i as f32 * (h + 4.0), w, h);
        let from = p.plan.nodes.len();
        if p.model.view.place == section || p.focused(Focus::Rail(i)) {
            p.surface(110 + i as u64 * 3, f, Fill::Tint(if p.focused(Focus::Rail(i)) { 0.16 } else { 0.09 }), Radius::Row, false, false, false, 10);
        }
        let marks = if i == 3 { Vec::new() } else { vec![p.mark(icons[i], side, 28.0, (h - side) / 2.0, Role::Ink2, false)] };
        p.words(
            111 + i as u64 * 3,
            f,
            vec![p.word(section.name(), Type::Body, p.ink(Role::Ink, false), side + 52.0, (h - p.line(Type::Body)) / 2.0, w - side - 72.0)],
            marks,
            11,
        );
        if i == 3 {
            p.symbol(
                112 + i as u64 * 3,
                crate::game::screens::symbols::Symbol::Controller,
                Frame::new(f.x + 28.0, f.y + (h - side) / 2.0, side, side),
                p.ink(Role::Ink2, false),
                11,
            );
        }
        if p.focused(Focus::Rail(i)) {
            p.zoom(from, f, p.tokens.focus.card_scale);
        }
        p.hit(Focus::Rail(i), f);
    }
    p.x + w + 48.0
}
