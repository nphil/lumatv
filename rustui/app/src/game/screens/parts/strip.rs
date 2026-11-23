//! A strip of choices along one edge: the in-stream quick strip, a centred pill along the bottom of the screen with one pill per
//! choice, an icon and, for some, a word.
//!
//! Node ids: 8830 the strip's pill, then four to an item from 8831: its words, its focus wash, its focus light and a drawn symbol.
use crate::game::{
    model::Focus,
    screens::{
        paint::{Fill, H, Paint, W},
        symbols::Symbol,
        parts::{SHEET_Z, focus::{self, Kind}, motion::Enter},
    },
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

/// An item's icon: one of the toolkit's, or one of LumaGame's drawn symbols, which takes a layer of its own.
pub enum Sign {
    Icon(Icon),
    Symbol(Symbol),
}

/// One choice: its icon, an optional word beside it, the focus its hit registers and whether it ends something (the icon is then red).
pub struct Item<'a> {
    pub icon: Sign,
    pub label: Option<&'a str>,
    pub focus: Focus,
    pub danger: bool,
}

/// The strip comes up from this far below.
const RISE: f32 = 24.0;
/// The strip's padding and the gap between items.
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
/// An item is a 64 px pill; with a word it has this room each side of its content, and this between icon and word.
const ITEM_H: f32 = 64.0;
const LABEL_PAD: f32 = 22.0;
const LABEL_GAP: f32 = 12.0;

/// Draws `items` as a pill centred along the bottom safe edge and returns its frame; the item at `focused` is lit. Every item
/// registers its own hit.
pub fn bar(p: &mut Paint<'_>, items: &[Item<'_>], focused: Option<usize>) -> Frame {
    let side = 30.0 * p.scale;
    let widths: Vec<f32> = items
        .iter()
        .map(|item| match item.label {
            Some(label) => (p.width(label, Type::Label) + side + LABEL_GAP + 2.0 * LABEL_PAD).max(ITEM_H),
            None => ITEM_H.max(side + 2.0 * LABEL_GAP),
        })
        .collect();
    let w = widths.iter().sum::<f32>() + GAP * items.len().saturating_sub(1) as f32 + 2.0 * PAD;
    let h = ITEM_H + 2.0 * PAD;
    let strip = Frame::new((W - w) / 2.0, H - p.y - h, w, h);

    p.arrives(strip, Enter::slide(0.0, RISE));
    p.surface(8830, strip, Fill::Sheet, Radius::Pill, false, false, false, SHEET_Z);
    let mut x = strip.x + PAD;
    for (i, item) in items.iter().enumerate() {
        let id = 8831 + 4 * i as u64;
        let frame = Frame::new(x, strip.y + PAD, widths[i], ITEM_H);
        let from = p.plan.nodes.len();
        let ink = if item.danger { Role::Danger } else { Role::Ink };
        let label_w = item.label.map_or(0.0, |label| p.width(label, Type::Label) + side + LABEL_GAP);
        let mark_x = (frame.w - label_w.max(side)) / 2.0;
        let words = item.label.map_or_else(Vec::new, |label| {
            let mut word = p.word(label, Type::Label, p.ink(Role::Ink, false), mark_x + side + LABEL_GAP, (ITEM_H - p.line(Type::Label)) / 2.0, label_w - side - LABEL_GAP);
            word.ty = p.bold(Type::Label);
            vec![word]
        });
        let mark_y = (ITEM_H - side) / 2.0;
        let marks = match item.icon {
            Sign::Icon(icon) => vec![p.mark(icon, side, mark_x, mark_y, ink, false)],
            Sign::Symbol(symbol) => {
                let colour = p.ink(ink, false);
                p.symbol(id + 3, symbol, Frame::new(frame.x + mark_x, frame.y + mark_y, side, side), colour, SHEET_Z + 1);
                Vec::new()
            }
        };
        p.words(id, frame, words, marks, SHEET_Z + 1);
        focus::treat(p, from, id + 1, frame, Kind::Button, focused == Some(i));
        p.hit(item.focus, frame);
        x += widths[i] + GAP;
    }
    strip
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::{paint::Body, parts::painted};

    fn items() -> [Item<'static>; 3] {
        [
            Item { icon: Sign::Icon(Icon::Sliders), label: None, focus: Focus::Nav(0), danger: false },
            Item { icon: Sign::Icon(Icon::Back), label: Some("Resume"), focus: Focus::Nav(1), danger: false },
            Item { icon: Sign::Icon(Icon::Close), label: None, focus: Focus::Nav(2), danger: true },
        ]
    }

    #[test]
    fn the_pill_is_centred_on_the_bottom_edge_and_every_item_registers_a_hit_inside_it() {
        painted(|_| {}, |p| {
            let strip = bar(p, &items(), None);
            assert!((strip.x * 2.0 + strip.w - W).abs() < 0.01);
            assert_eq!(strip.y + strip.h, H - p.y);
            assert_eq!(p.plan.hits.len(), 3);
            for (_, hit) in &p.plan.hits {
                assert_eq!(hit.h, ITEM_H);
                assert!(hit.x >= strip.x && hit.x + hit.w <= strip.x + strip.w);
            }
            assert!(p.plan.hits[1].1.w > p.plan.hits[0].1.w, "a word widens its item");
        });
    }

    #[test]
    fn only_the_focused_item_is_lit_and_a_dangerous_one_has_a_red_icon() {
        painted(|_| {}, |p| {
            bar(p, &items(), Some(1));
            let ids: Vec<u64> = p.plan.nodes.iter().map(|n| n.id).collect();
            assert!(ids.contains(&8836) && ids.contains(&8837) && !ids.contains(&8832) && !ids.contains(&8840));
            let mark = |id: u64| match &p.plan.nodes.iter().find(|n| n.id == id).unwrap().body {
                Body::Text { marks, .. } => marks[0].colour,
                _ => panic!("not text"),
            };
            assert_eq!(mark(8839), p.ink(Role::Danger, false));
            assert_eq!(mark(8831), p.ink(Role::Ink, false));
        });
    }

    #[test]
    fn the_strip_rises_into_place() {
        painted(|_| {}, |p| {
            bar(p, &items(), None);
            p.finish();
            assert_eq!((p.plan.enters[&8830].dy, p.plan.enters[&8831].dy), (RISE, RISE));
        });
    }
}
