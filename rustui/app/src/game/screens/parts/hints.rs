//! The key hints: at most three keys and one word each, in a bottom corner of the screen or of the layer that is open. A key's mark is
//! the face button of the pad in hand, otherwise the remote's icon. A row of marks that never changes would wear the panel, so after
//! a few seconds without a key the bar rests at 40 % (the model says when) and any key brings it back.
//!
//! Node ids: `ID` and up, four to a hint.
use crate::game::{
    model::{Key, nav::Hints},
    screens::{paint::Paint, parts::Side},
};
use lumatv_tokens::TypeRole as Type;
use ui::components::Icon;

/// The first of the bar's node ids.
pub const ID: u64 = 8900;
/// What stands between two hints.
const GAP: f32 = 32.0;
/// A resting bar shows at this opacity.
const REST_OPACITY: f32 = 0.4;
/// The bar sits above the page's own marks.
const Z: i32 = 60;
/// A bar holds this many hints; more than three is a menu, not a hint.
const MAX: usize = 3;

/// What `key` looks like on a remote, which has no face buttons.
fn remote(key: Key) -> Icon {
    match key {
        Key::Ok => Icon::Circle,
        Key::Back | Key::Left => Icon::Back,
        Key::Menu => Icon::Sliders,
        Key::Alt | Key::Guide => Icon::Apps,
        Key::Up => Icon::ChevronUp,
        Key::Down => Icon::ChevronDown,
        Key::Right => Icon::ChevronRight,
    }
}

/// Draws `hints` against `side` of the anchor box (the screen's safe area, or the layer that is open), along its bottom edge; `dim` is
/// the resting state the model reports after a few seconds without a key.
pub fn bar(p: &mut Paint<'_>, hints: &Hints, side: Side, dim: bool) {
    let list = &hints.0[..hints.0.len().min(MAX)];
    if list.is_empty() {
        return;
    }
    let widths: Vec<f32> = list.iter().map(|&(_, label)| p.hint_width(label)).collect();
    let total = widths.iter().sum::<f32>() + GAP * (list.len() - 1) as f32;
    let anchor = p.anchor;
    let y = anchor.y + anchor.h - (37.0 * p.scale).max(p.line(Type::Label));
    let mut x = match side {
        Side::Right => anchor.x + anchor.w - total,
        Side::Left => anchor.x,
    };
    let from = p.plan.nodes.len();
    for (i, &(key, label)) in list.iter().enumerate() {
        let lead = p.lead(key, remote(key));
        x += p.hint(ID + 4 * i as u64, label, lead, x, y, p.art, Z) + GAP;
    }
    if dim {
        for node in &mut p.plan.nodes[from..] {
            node.props.opacity = REST_OPACITY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{model::Input, screens::parts::painted};
    use ui::geometry::Frame;

    fn keys(list: &[(Key, &'static str)]) -> Hints {
        Hints(list.to_vec())
    }
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn hints_sit_along_the_bottom_of_the_anchor_against_the_side_asked_for() {
        painted(|_| {}, |p| {
            let list = keys(&[(Key::Back, "Back"), (Key::Ok, "Choose")]);
            bar(p, &list, Side::Right, false);
            let frames: Vec<Frame> = p.plan.nodes.iter().map(|n| n.frame).collect();
            let a = p.anchor;
            assert_eq!(frames.len(), 2);
            assert!(near(frames[1].x + frames[1].w, a.x + a.w), "the last hint ends at the anchor's right edge");
            assert!(near(frames[1].x - (frames[0].x + frames[0].w), GAP));
            // The text box is as tall as the glyph renderer's line box, so it may hang a little below the 37 px mark that sits on the edge.
            assert!(frames[0].y + 37.0 * p.scale <= a.y + a.h + 0.01 && frames[0].y + frames[0].h >= a.y + a.h, "the bar rests on the anchor's bottom edge");
            p.plan.nodes.clear();
            bar(p, &list, Side::Left, false);
            assert!(near(p.plan.nodes[0].frame.x, a.x), "the first hint starts at the anchor's left edge");
        });
    }

    #[test]
    fn a_bar_follows_its_anchor_and_holds_three_hints() {
        painted(|_| {}, |p| {
            p.anchor = Frame::new(1000.0, 100.0, 640.0, 500.0);
            let four = keys(&[(Key::Ok, "A"), (Key::Back, "B"), (Key::Menu, "C"), (Key::Alt, "D")]);
            bar(p, &four, Side::Left, false);
            assert_eq!(p.plan.nodes.len(), 3);
            let node = &p.plan.nodes[2];
            assert!(near(p.plan.nodes[0].frame.x, 1000.0) && node.frame.y >= 100.0 && node.frame.y + 37.0 * p.scale <= 600.01);
        });
    }

    #[test]
    fn a_resting_bar_is_at_forty_percent_and_a_live_one_at_full() {
        painted(|_| {}, |p| {
            let list = keys(&[(Key::Back, "Back")]);
            bar(p, &list, Side::Right, false);
            assert_eq!(p.plan.nodes[0].props.opacity, 1.0);
            p.plan.nodes.clear();
            bar(p, &list, Side::Right, true);
            assert_eq!(p.plan.nodes[0].props.opacity, REST_OPACITY);
        });
    }

    #[test]
    fn the_marks_are_the_remotes_icons_until_a_pad_is_used_then_its_face_buttons() {
        painted(|_| {}, |p| {
            bar(p, &keys(&[(Key::Ok, "Choose")]), Side::Right, false);
            assert_eq!(p.plan.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [ID], "one text node with the icon as a mark");
        });
        painted(
            |m| {
                m.step(Input::Controllers([Some("Pad".to_owned()), None, None, None]));
            },
            |p| {
                bar(p, &keys(&[(Key::Ok, "Choose")]), Side::Right, false);
                assert_eq!(p.plan.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [ID, ID + 1, ID + 2], "a disc, its letter, then the word");
            },
        );
    }
}
