//! The curtain behind a layer, its opaque panel and the scrolling list inside it.
use crate::game::screens::paint::{Fill, H, PAGE_EXTENT, Paint, ScrollGroup, SHEET_GROUP, W};
use crate::game::model::Layer;
use compositor_protocol::{Edges, Rect};
use lumatv_tokens::Radius;
use ui::{geometry::Frame, lucent::palette::PaletteRole as Role};

pub fn curtain(p: &mut Paint<'_>) {
    let colour = if p.model.view.sheet == Some(Layer::EndSession) {
        let mut colour = p.straight(Role::Canvas, false);
        colour.a = 247;
        colour.premultiply(0.0)
    } else {
        p.ink(Role::Scrim, false)
    };
    p.solid(8000, Frame::new(0.0, 0.0, W, H), colour, 0.0, 25);
}
pub fn panel(p: &mut Paint<'_>, width: f32, height: f32) -> Frame {
    let frame = Frame::new((W - width) / 2.0, (H - height) / 2.0, width, height);
    p.surface(8001, frame, Fill::Sheet, Radius::Sheet, false, false, true, 30);
    frame
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
