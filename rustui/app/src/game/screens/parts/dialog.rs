//! The curtain behind a layer, its opaque panel and the scrolling list inside it, and the confirmation dialog (`modal`).
//!
//! Node ids of `modal`: 8810 the backdrop, 8811 the panel, 8812 the title, 8813 the body, 8820 and up four to an action (its pill, its
//! words, its pad glyph).
use crate::game::model::Focus;
use crate::game::screens::{
    paint::{Body, Fill, H, PAGE_EXTENT, Paint, SHEET_GROUP, ScrollGroup, W},
    parts::{SCRIM_Z, SHEET_Z, motion::Enter},
};
use compositor_protocol::{Backdrop, Edges, Rect};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Icon, wrap}, geometry::Frame, lucent::palette::PaletteRole as Role};

/// Paints a scrim over `frame` at the layer scrim's depth. A layer that `hides` the page (97 % canvas) leaves nothing readable under it;
/// the others dim it with the theme's scrim.
pub fn veil(p: &mut Paint<'_>, id: u64, frame: Frame, hides: bool) {
    let colour = if hides {
        let mut colour = p.straight(Role::Canvas, false);
        colour.a = 247;
        colour.premultiply(0.0)
    } else {
        p.ink(Role::Scrim, false)
    };
    p.solid(id, frame, colour, 0.0, SCRIM_Z);
}
/// The dim over the whole screen behind the open layer.
pub fn curtain(p: &mut Paint<'_>) {
    veil(p, 8000, Frame::new(0.0, 0.0, W, H), false);
}
pub fn panel(p: &mut Paint<'_>, width: f32, height: f32) -> Frame {
    let frame = Frame::new((W - width) / 2.0, (H - height) / 2.0, width, height);
    let from = p.plan.nodes.len();
    p.surface(8001, frame, Fill::Sheet, Radius::Sheet, false, false, true, 30);
    opaque(p, from);
    frame
}
/// What was painted from node `from` on keeps its glass's edge and shadow but captures no backdrop: on an opaque sheet there is
/// nothing to blur, and a capture costs 12 to 15 ms cold on the TV.
pub fn opaque(p: &mut Paint<'_>, from: usize) {
    for node in &mut p.plan.nodes[from..] {
        if let Body::Surface(glass) = &mut node.body {
            glass.backdrop = Backdrop::None;
        }
    }
}
/// Lays out rows `0..rows`, each `pitch` tall, of a list that shows from row `first` inside `view`, as a scroll group: the rows are built
/// at their place for that offset and the group's spring carries the list there, so a list that moves on goes on from the speed it has.
/// Only the rows the travel from the list's present offset to that one passes through are built. `pad` is how far the clip reaches past
/// `view` above and below (and 32 px at the sides) for a focused row's lift; it is less than the gap between rows.
pub fn scrolling(p: &mut Paint<'_>, view: Rect, pitch: f32, rows: usize, first: usize, pad: f32, mut row: impl FnMut(&mut Paint<'_>, usize, f32)) {
    let offset = first as f32 * pitch;
    let from = p.scroll_positions.get(&SHEET_GROUP).copied().unwrap_or(offset);
    let (lo, hi) = (from.min(offset), from.max(offset));
    let start = p.plan.nodes.len();
    for i in 0..rows {
        let top = i as f32 * pitch;
        if top + pitch <= lo - pad || top >= hi + view.h + pad {
            continue;
        }
        let hits = p.plan.hits.len();
        row(p, i, view.y + top - offset);
        for k in hits..p.plan.hits.len() {
            let focus = p.plan.hits[k].0;
            p.plan.hit_groups.insert(focus, SHEET_GROUP);
        }
    }
    let nodes = p.plan.nodes[start..].iter().map(|node| node.id).collect();
    p.plan.groups.push(ScrollGroup {
        id: SHEET_GROUP,
        parent: None,
        clip: Rect { x: view.x - 32.0, y: view.y - pad, w: view.w + 64.0, h: view.h + 2.0 * pad },
        feather: Edges::NONE,
        horizontal: false,
        offset,
        extent: PAGE_EXTENT,
        nodes,
    });
}

/// One button of a dialog: its words, an optional icon and the focus its hit registers.
pub struct Action<'a> {
    pub label: &'a str,
    pub icon: Option<Icon>,
    pub focus: Focus,
}

/// The panel is this wide, and wider at large text.
const MODAL_W: f32 = 840.0;
const MODAL_W_LARGE: f32 = 1080.0;
/// Panel padding: top, sides, bottom.
const PAD_TOP: f32 = 40.0;
const PAD_SIDE: f32 = 36.0;
const PAD_BOTTOM: f32 = 34.0;
/// Between the title and the body, the body and the buttons, and two buttons.
const TITLE_GAP: f32 = 14.0;
const BODY_GAP: f32 = 28.0;
const ACTION_GAP: f32 = 14.0;
/// The panel grows from this fraction of its size as it opens.
const GROW: f32 = 0.96;

/// A dialog over a page that is all but hidden: a title, a body and a row of actions, of which `actions[safe]` is the primary one and
/// the one the model puts focus on first. Returns the panel's frame; the panel is as tall as its words. The page's art is covered, and
/// the hints are set in the panel.
pub fn modal(p: &mut Paint<'_>, title: &str, body: &str, actions: &[Action<'_>], safe: usize) -> Frame {
    p.art = false;
    let width = if p.scale > 1.0 { MODAL_W_LARGE } else { MODAL_W };
    let inner = width - 2.0 * PAD_SIDE;
    let title_lines = wrap(p.fonts, title, p.ty(Type::Title), inner);
    let body_lines = wrap(p.fonts, body, p.ty(Type::Body), inner);
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    let height = PAD_TOP + p.block(Type::Title, title_lines.len()) + TITLE_GAP + p.block(Type::Body, body_lines.len()) + BODY_GAP + button_h + PAD_BOTTOM;
    let panel = Frame::new((W - width) / 2.0, (H - height) / 2.0, width, height);

    p.arrives(Frame::new(0.0, 0.0, W, H), Enter::FADE);
    veil(p, 8810, Frame::new(0.0, 0.0, W, H), true);

    p.arrives(panel, Enter::grow(GROW));
    p.surface(8811, panel, Fill::Sheet, Radius::Sheet, false, false, false, SHEET_Z);
    let (x, mut y) = (panel.x + PAD_SIDE, panel.y + PAD_TOP);
    let at = |y: f32| Frame::new(x, y, inner, 0.0);
    y += p.lines(8812, title_lines, at(y), Type::Title, Role::Ink, false, SHEET_Z + 1) + TITLE_GAP;
    y += p.lines(8813, body_lines, at(y), Type::Body, Role::Ink2, false, SHEET_Z + 1) + BODY_GAP;
    let (mut left, buttons) = (x, p.plan.nodes.len());
    for (i, action) in actions.iter().enumerate() {
        left += p.button(8820 + 4 * i as u64, action.focus, action.label, action.icon, left, y, i == safe, false, SHEET_Z + 1) + ACTION_GAP;
    }
    opaque(p, buttons);
    p.anchor = Frame::new(x, panel.y + PAD_TOP, inner, height - PAD_TOP - PAD_BOTTOM);
    panel
}
/// Marks the `modal`'s action `i` as the one that cannot be undone: its icon takes the danger colour, its label stays in ink.
pub fn danger(p: &mut Paint<'_>, i: usize) {
    let colour = p.ink(Role::Danger, false);
    if let Some(Body::Text { marks, .. }) = p.plan.nodes.iter_mut().find(|n| n.id == 8820 + 4 * i as u64 + 1).map(|n| &mut n.body) {
        for mark in marks {
            mark.colour = colour;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::{paint::Body, parts::painted};

    fn actions() -> [Action<'static>; 2] {
        [Action { label: "Cancel", icon: None, focus: Focus::Nav(0) }, Action { label: "End session", icon: Some(Icon::Back), focus: Focus::Nav(1) }]
    }
    fn tint_of(p: &Paint<'_>, id: u64) -> compositor_protocol::Rgba {
        match &p.plan.nodes.iter().find(|n| n.id == id).unwrap().body {
            Body::Surface(glass) => glass.tint,
            _ => panic!("node {id} is not a surface"),
        }
    }

    #[test]
    fn the_panel_is_centred_hides_the_art_and_holds_the_hints() {
        painted(|_| {}, |p| {
            p.art = true;
            let panel = modal(p, "End the session?", "The game keeps running on the host.", &actions(), 0);
            assert!(!p.art);
            assert_eq!(panel.w, MODAL_W);
            assert!((panel.x * 2.0 + panel.w - W).abs() < 0.01 && (panel.y * 2.0 + panel.h - H).abs() < 0.01);
            assert!(p.anchor.x >= panel.x && p.anchor.y + p.anchor.h <= panel.y + panel.h);
            let backdrop = p.plan.nodes.iter().find(|n| n.id == 8810).unwrap();
            assert_eq!(backdrop.z, SCRIM_Z);
            assert_eq!(tint_of(p, 8810).3, 247, "the page is all but hidden");
        });
    }

    #[test]
    fn only_the_safe_action_is_primary_and_each_registers_its_focus() {
        painted(|_| {}, |p| {
            modal(p, "Delete?", "Gone for good.", &actions(), 1);
            let hits: Vec<Focus> = p.plan.hits.iter().map(|(focus, _)| *focus).collect();
            assert_eq!(hits, [Focus::Nav(0), Focus::Nav(1)]);
            assert_eq!(tint_of(p, 8824), p.colour(Fill::Primary, false));
            assert_ne!(tint_of(p, 8820), p.colour(Fill::Primary, false));
        });
    }

    #[test]
    fn the_panel_grows_with_its_body() {
        let height = |body: &str| painted(|_| {}, |p| modal(p, "Sure?", body, &actions(), 0).h);
        assert!(height(&"Many words to wrap over lines. ".repeat(40)) > height("Yes."));
    }

    #[test]
    fn the_backdrop_fades_and_the_panel_grows_about_itself() {
        painted(|_| {}, |p| {
            modal(p, "Sure?", "Yes.", &actions(), 0);
            p.finish();
            assert_eq!((p.plan.enters[&8810].scale, p.plan.enters[&8810].dx), (1.0, 0.0));
            assert_eq!(p.plan.enters[&8811].scale, GROW);
        });
    }
}
