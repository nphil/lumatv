//! The Library and Search pages and the sort and genre choices.
use crate::game::screens::paint::{Fill, H, Paint, W};
use crate::game::model::Focus;
use compositor_protocol::Rect;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::parts::{dialog::*, keyboard::*, stage::*, topbar::{self, Here, refresh_note}};

pub fn library(p: &mut Paint<'_>, search: bool) {
    let attention = topbar::needs_attention(p);
    let nav_end = topbar::bar(p, if search { Here::Search } else { Here::Library }, attention);
    refresh_note(p, 40, nav_end, p.y);
    let m = p.model;
    let mut y = 162.0;
    if search {
        let f = Frame::new(p.x, y, W - 2.0 * p.x, p.tokens.target.max(p.line(Type::Body) + 24.0));
        let from = p.plan.nodes.len();
        p.surface(100, f, Fill::Glass, Radius::Pill, false, p.focused(Focus::SearchField), false, 10);
        let text = if m.view.query.is_empty() { "Search games" } else { &m.view.query };
        p.words(
            101,
            f,
            vec![p.word(
                text,
                Type::Body,
                p.ink(if m.view.query.is_empty() { Role::Ink2 } else { Role::Ink }, false),
                72.0,
                (f.h - p.line(Type::Body)) / 2.0,
                f.w - 96.0,
            )],
            vec![p.mark(Icon::Search, 32.0, 24.0, (f.h - 32.0) / 2.0, Role::Ink2, false)],
            11,
        );
        if p.focused(Focus::SearchField) {
            p.zoom(from, f, p.tokens.focus.button_scale);
        }
        p.hit(Focus::SearchField, f);
        y += f.h + 24.0;
        if matches!(m.view.focus, Focus::SearchField | Focus::Keyboard(_)) {
            keyboard(p, 200, p.x, y, W - 2.0 * p.x, false, false);
            y += 5.0 * ((52.0 * p.scale).min(80.0) + 8.0) + 24.0;
        }
    } else {
        let mut labels = vec![format!("All  {}", m.filter_count(None))];
        for e in &m.integrations {
            if m.enabled(&e.manifest.id) {
                labels.push(format!("{}  {}", e.manifest.name, m.filter_count(Some(&e.manifest.id))));
            }
        }
        labels.extend([m.view.filter.genre.clone().unwrap_or_else(|| "Genre".into()), "Favourites".into(), "Sort".into(), "Refresh libraries".into()]);
        let widths: Vec<_> = labels.iter().map(|s| p.width(s, Type::Label) + 52.0).collect();
        let focused = if let Focus::Filter(i) = m.view.focus { Some(i) } else { None };
        let total_before = focused.map_or(0.0, |i| widths.iter().take(i + 1).sum::<f32>() + 12.0 * i as f32);
        let shift = (total_before - (W - p.x * 2.0)).max(0.0);
        let mut x = p.x - shift;
        let h = p.tokens.target.max(p.line(Type::Label) + 24.0);
        for (i, label) in labels.iter().enumerate() {
            let f = Frame::new(x, y, widths[i], h);
            if x + f.w >= p.x && x < W - p.x {
                let active = if i == 0 {
                    m.view.filter == Default::default()
                } else if i <= labels.len() - 5 {
                    m.view.filter.engine.as_ref().is_some_and(|id| label.starts_with(m.engine_name(id)))
                } else {
                    i == labels.len() - 3 && m.view.filter.favourites
                };
                let from = p.plan.nodes.len();
                p.surface(
                    100 + i as u64 * 4,
                    f,
                    if active { Fill::Tint(0.12) } else { Fill::Glass },
                    Radius::Pill,
                    false,
                    p.focused(Focus::Filter(i)),
                    false,
                    10,
                );
                p.text(
                    101 + i as u64 * 4,
                    label,
                    Frame::new(x + 26.0, y + (h - p.line(Type::Label)) / 2.0, f.w - 52.0, p.line(Type::Label)),
                    Type::Label,
                    Role::Ink,
                    false,
                    11,
                );
                p.hit(Focus::Filter(i), f);
                if p.focused(Focus::Filter(i)) {
                    p.zoom(from, f, p.tokens.focus.button_scale);
                }
            }
            x += f.w + 12.0;
        }
        y += h + 40.0;
    }
    if m.filtered.is_empty() {
        p.text(
            500,
            if search && m.view.query.is_empty() { "Search the connected libraries" } else { "No games found" },
            Frame::new(p.x, y + 64.0, 1500.0, p.line(Type::Title)),
            Type::Title,
            Role::Ink,
            false,
            10,
        );
        return;
    }
    if y >= H && matches!(m.view.focus, Focus::SearchField | Focus::Keyboard(_)) {
        return;
    }
    let columns = m.prefs.covers.columns();
    let w = (W - p.x * 2.0 - p.gap * (columns - 1) as f32) / columns as f32;
    let row_h = w * 1.5 + 40.0 + p.line(Type::Body) + p.line(Type::Caption) + 26.0;
    let first = m.view.first_grid_row;
    let current = p.scroll_positions.get(&1).copied().unwrap_or(first as f32 * row_h);
    let current_row = (current / row_h).floor().max(0.0) as usize;
    let total = m.filtered.len().div_ceil(columns);
    let mut rows = Vec::with_capacity(8);
    for anchor in [first, current_row] {
        for row in anchor.saturating_sub(1)..(anchor + 3).min(total) {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    rows.sort_unstable();
    let start = p.plan.nodes.len();
    for row in rows {
        let yy = y + (row as f32 - first as f32) * row_h;
        for (i, game) in m.filtered.iter().enumerate().skip(row * columns).take(columns) {
            let hits = p.plan.hits.len();
            cover(p, 1000 + i as u64 * 20, *game, Focus::Grid(i), p.x + (i % columns) as f32 * (w + p.gap), yy, w);
            if p.plan.hits.len() > hits {
                p.plan.hit_groups.insert(Focus::Grid(i), 1);
            }
        }
    }
    let nodes = p.plan.nodes[start..].iter().map(|node| node.id).collect();
    p.plan.groups.push(super::paint::ScrollGroup {
        id: 1, parent: None, clip: compositor_protocol::Rect { x: p.x - p.gap, y, w: W - 2.0 * (p.x - p.gap), h: H - y },
        feather: compositor_protocol::Edges::NONE, horizontal: false, offset: first as f32 * row_h, extent: total as f32 * row_h + H, nodes,
    });
}
pub fn list(p: &mut Paint<'_>, title: &str, values: Vec<String>) {
    let h = 80.0f32.max(p.line(Type::Body) + 24.0);
    let height = (p.line(Type::Title) + 106.0 + values.len() as f32 * (h + 6.0)).min(H - 108.0);
    let f = panel(p, 960.0, height);
    p.text(8010, title, Frame::new(f.x + 36.0, f.y + 40.0, f.w - 72.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let y = f.y + 66.0 + p.line(Type::Title);
    let visible = ((height - p.line(Type::Title) - 106.0) / (h + 6.0)).floor().max(1.0) as usize;
    let first = if let Focus::SheetRow(i) = p.model.view.focus { i.saturating_sub(visible - 1) } else { 0 };
    let view = Rect { x: f.x + 36.0, y, w: f.w - 72.0, h: visible as f32 * (h + 6.0) - 6.0 };
    scrolling(p, view, h + 6.0, values.len(), first, 4.0, |p, i, y| {
        let row = Frame::new(f.x + 36.0, y, f.w - 72.0, h);
        let from = p.plan.nodes.len();
        p.surface(
            8100 + i as u64 * 3,
            row,
            Fill::Tint(if p.focused(Focus::SheetRow(i)) { 0.09 } else { 0.0 }),
            Radius::Row,
            false,
            p.focused(Focus::SheetRow(i)),
            false,
            32,
        );
        p.text(
            8101 + i as u64 * 3,
            &values[i],
            Frame::new(row.x + 26.0, y + (h - p.line(Type::Body)) / 2.0, row.w - 52.0, p.line(Type::Body)),
            Type::Body,
            Role::Ink,
            false,
            33,
        );
        if p.focused(Focus::SheetRow(i)) {
            p.zoom(from, row, p.tokens.focus.card_scale);
        }
        p.hit(Focus::SheetRow(i), row);
    });
}
