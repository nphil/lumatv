//! The focus treatment every control shares (Lucent's Focus state): the fill steps up to the focus wash with a rim lit from above
//! (the raised lip) and one soft shadow and, with movement allowed, the control lifts and grows a little. There is no
//! underline or outline ring. Colours and scales are the theme's (`focus.*`, `states.focus_wash`).
use crate::game::screens::paint::{Fill, Paint};
use lumatv_tokens::Radius;
use ui::{geometry::Frame, lucent::palette::Mode};

/// What is focused; each kind grows by its own scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A cover or a tile: the art is the control, so it takes the rim and the shadow without a wash.
    Tile,
    /// A glance card or a panel.
    Card,
    /// A list row.
    Row,
    /// A pill: the top bar's items and buttons.
    Button,
}

fn scale(p: &Paint<'_>, kind: Kind) -> f32 {
    let focus = &p.tokens.focus;
    match kind {
        Kind::Tile => focus.tile_scale,
        Kind::Card => focus.card_scale,
        Kind::Row => focus.row_scale,
        Kind::Button => focus.button_scale,
    }
}

/// Lights the control that drew nodes `from..` into `frame` when it is `focused`: the wash, with its lit rim and shadow, is added under
/// id `id`, and with movement allowed everything the control drew grows by the kind's scale and lifts. A control that is not focused is
/// left as drawn. Call it once the control's own nodes are in the plan; the wash goes above them. With reduced motion the wash is all
/// there is.
pub fn treat(p: &mut Paint<'_>, from: usize, id: u64, frame: Frame, kind: Kind, focused: bool) {
    if !focused {
        return;
    }
    let z = p.plan.nodes[from..].iter().map(|node| node.z).max().unwrap_or(0) + 1;
    let radius = match kind {
        Kind::Tile => Radius::Tile,
        Kind::Card => Radius::Card,
        Kind::Row => Radius::Row,
        Kind::Button => Radius::Pill,
    };
    let wash = if kind == Kind::Tile { 0.0 } else { p.tokens.states.focus_wash[usize::from(p.mode == Mode::Light)] };
    // The wash carries the raised lip and the shadow: the control's own surface was drawn at rest. Over a tile's art it is clear.
    let stage = p.art && kind == Kind::Tile;
    p.surface(id, frame, Fill::Tint(wash), radius, stage, true, false, z);
    if !p.reduced() {
        let lift = if kind == Kind::Row { 0.0 } else { p.tokens.focus.card_lift };
        p.raise(from, frame, scale(p, kind), lift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::parts::painted;
    use compositor_protocol::Rgba;

    const BOX: Frame = Frame { x: 100.0, y: 200.0, w: 400.0, h: 80.0 };
    const NONE: Rgba = Rgba(0, 0, 0, 0);

    /// A control of two nodes, as a part draws it, then `treat`ed: the plan's nodes as (id, z, x, y, scale).
    fn lit(kind: Kind, focused: bool, reduce_motion: bool) -> Vec<(u64, i32, f32, f32, f32)> {
        painted(|m| m.prefs.reduce_motion = reduce_motion, |p| {
            p.solid(1, BOX, NONE, 0.0, 5);
            p.solid(2, Frame::new(120.0, 220.0, 40.0, 40.0), NONE, 0.0, 6);
            treat(p, 0, 10, BOX, kind, focused);
            p.plan.nodes.iter().map(|n| (n.id, n.z, n.props.x, n.props.y, n.props.scale)).collect()
        })
    }

    #[test]
    fn a_control_that_is_not_focused_is_left_as_drawn() {
        assert_eq!(lit(Kind::Row, false, false), [(1, 5, 100.0, 200.0, 1.0), (2, 6, 120.0, 220.0, 1.0)]);
    }

    #[test]
    fn a_focused_row_gets_a_wash_above_it_and_grows_without_lifting() {
        let scale = painted(|_| {}, |p| p.tokens.focus.row_scale);
        let nodes = lit(Kind::Row, true, false);
        let ids: Vec<_> = nodes.iter().map(|n| (n.0, n.1)).collect();
        assert_eq!(ids, [(1, 5), (2, 6), (10, 7)], "a wash and no underline");
        assert!(nodes.iter().all(|n| n.4 == scale));
        assert_eq!(nodes[0].3, 200.0, "the row's own centre does not move");
    }

    #[test]
    fn a_button_a_card_and_a_tile_lift_by_the_themes_lift() {
        let lift = painted(|_| {}, |p| p.tokens.focus.card_lift);
        for kind in [Kind::Button, Kind::Card, Kind::Tile] {
            assert_eq!(lit(kind, true, false)[0].3, 200.0 - lift, "{kind:?}");
        }
    }

    #[test]
    fn with_reduced_motion_the_wash_stays_and_nothing_moves() {
        let nodes = lit(Kind::Card, true, true);
        assert_eq!(nodes, [(1, 5, 100.0, 200.0, 1.0), (2, 6, 120.0, 220.0, 1.0), (10, 7, 100.0, 200.0, 1.0)]);
    }
}
