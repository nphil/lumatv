//! The focus treatment every control shares: a wash over it, a landing light under it and, with movement allowed, a lift and a small
//! growth. Colours, scales and the light's thickness are the theme's (`focus.*`, `states.focus_wash`).
use crate::game::screens::paint::{Fill, Paint};
use lumatv_tokens::Radius;
use ui::{geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};

/// What is focused; each kind grows by its own scale, and the light lands in a different place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A cover or a tile: the art is the control, so it has no wash; the light sits under the art.
    Tile,
    /// A glance card or a panel.
    Card,
    /// A list row.
    Row,
    /// A pill: the top bar's items and buttons.
    Button,
}

/// The light on a row or a button stands in this far from each side (an icon-only button this much less) and this far above the bottom edge.
const LANDING_INSET: f32 = 26.0;
const LANDING_INSET_ICON: f32 = 22.0;
const LANDING_RAISE: f32 = 8.0;
/// On a card the light runs along the bottom edge, in this far from each side.
const CARD_INSET: f32 = 30.0;
/// Under a tile it is in 22 % of the art's width from each side and this far below it.
const TILE_INSET: f32 = 0.22;
const TILE_BELOW: f32 = 18.0;

/// Where the light sits for a control in `frame`.
pub fn landing(p: &Paint<'_>, frame: Frame, kind: Kind) -> Frame {
    let thick = p.tokens.focus.landing;
    match kind {
        Kind::Tile => Frame::new(frame.x + frame.w * TILE_INSET, frame.y + frame.h + TILE_BELOW, frame.w * (1.0 - 2.0 * TILE_INSET), thick),
        // A row's second line sits low, so its light hugs the bottom edge like a card's rather than rising to a button's height.
        Kind::Card | Kind::Row => Frame::new(frame.x + CARD_INSET, frame.y + frame.h - thick, frame.w - 2.0 * CARD_INSET, thick),
        Kind::Button => {
            let inset = if frame.w <= frame.h { LANDING_INSET_ICON } else { LANDING_INSET };
            Frame::new(frame.x + inset, frame.y + frame.h - LANDING_RAISE - thick, frame.w - 2.0 * inset, thick)
        }
    }
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

/// Lights the control that drew nodes `from..` into `frame` when it is `focused`: the wash and the landing light are added under ids `id`
/// and `id + 1`, and with movement allowed everything the control drew grows by the kind's scale and lifts. A control that is not
/// focused is left as drawn. Call it once the control's own nodes are in the plan; the wash and the light go above them. With reduced
/// motion the wash and the light are all there is.
pub fn treat(p: &mut Paint<'_>, from: usize, id: u64, frame: Frame, kind: Kind, focused: bool) {
    if !focused {
        return;
    }
    let z = p.plan.nodes[from..].iter().map(|node| node.z).max().unwrap_or(0) + 1;
    // A tile's light sits on the canvas below art that is dark whatever the theme; a card, a row and a pill sit on their own surface.
    let stage = p.art && kind == Kind::Tile;
    let radius = match kind {
        Kind::Tile => None,
        Kind::Card => Some(Radius::Card),
        Kind::Row => Some(Radius::Row),
        Kind::Button => Some(Radius::Pill),
    };
    if let Some(radius) = radius {
        let light = usize::from(p.mode == Mode::Light);
        let wash = p.tokens.states.focus_wash[light];
        // The wash carries the raised edge and the active shadow: the control's own surface was drawn at rest.
        p.surface(id, frame, Fill::Tint(wash), radius, false, true, false, z);
    }
    let light = landing(p, frame, kind);
    let ink = p.ink(Role::Ink, stage);
    p.solid(id + 1, light, ink, light.h / 2.0, z + 1);
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
    fn a_focused_row_gets_a_wash_and_a_light_above_it_and_grows_without_lifting() {
        let scale = painted(|_| {}, |p| p.tokens.focus.row_scale);
        let nodes = lit(Kind::Row, true, false);
        let ids: Vec<_> = nodes.iter().map(|n| (n.0, n.1)).collect();
        assert_eq!(ids, [(1, 5), (2, 6), (10, 7), (11, 8)]);
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
    fn with_reduced_motion_the_wash_and_the_light_stay_and_nothing_moves() {
        let nodes = lit(Kind::Card, true, true);
        assert_eq!(nodes, [(1, 5, 100.0, 200.0, 1.0), (2, 6, 120.0, 220.0, 1.0), (10, 7, 100.0, 200.0, 1.0), (11, 8, 130.0, 278.0, 1.0)]);
    }

    #[test]
    fn the_light_lands_where_the_mock_puts_it_for_each_kind() {
        painted(|_| {}, |p| {
            assert_eq!(landing(p, BOX, Kind::Row), landing(p, BOX, Kind::Card), "a row's light sits where a card's does");
            assert_eq!(landing(p, Frame::new(0.0, 0.0, 64.0, 64.0), Kind::Button), Frame::new(22.0, 54.0, 20.0, 2.0));
            assert_eq!(landing(p, BOX, Kind::Card), Frame::new(130.0, 278.0, 340.0, 2.0));
            assert_eq!(landing(p, BOX, Kind::Tile), Frame::new(188.0, 298.0, 224.0, 2.0));
        });
    }
}
