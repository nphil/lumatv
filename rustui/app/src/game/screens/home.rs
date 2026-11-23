//! Home: the featured game, the shelves and their collection and genre tiles.
use crate::game::screens::paint::{Fill, H, Paint, W};
use crate::game::model::{Density, Focus, home::HomeItem};
use lumagame_core::catalog::ArtKind;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};
use crate::game::screens::parts::{stage::*, topbar::*};
use super::accounts::first_run;

pub fn home(p: &mut Paint<'_>) {
    let m = p.model;
    if m.games().is_empty() {
        first_run(p);
        return;
    }
    let Some(game) = m.view.featured.and_then(|id| m.game(id)) else {
        return;
    };
    let density = m.prefs.hero;
    let deep = m.view.home_row > 0 || density == Density::Cinematic && p.scale > 1.0 && matches!(m.view.focus, Focus::Home(_, _));
    let mut hero_end = p.y + density.hero_height();
    let meta = m.details.get(&game.id);
    let mut meta_parts = Vec::new();
    if let Some(genre) = m.game_genres(game.id).into_iter().next() {
        meta_parts.push(genre);
    }
    if let Some(ago) = m.since_played(game.id) {
        meta_parts.push(format!("Played {ago}"));
    }
    if let Some((seconds, _)) = m.play_time_of(game.id) {
        meta_parts.push(format!("{} played", crate::game::model::game::play_time(seconds)));
    }
    let meta_text = meta_parts.join("  ·  ");
    let meta_h = ui::components::wrap(p.fonts, &meta_text, p.ty(Type::Label), 820.0).len() as f32 * p.line(Type::Label);
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    let actions_h = if p.scale > 1.0 { button_h * 3.0 + 32.0 } else { button_h };
    let description = if density == Density::Cinematic { meta.map(|d| d.description.as_str()).unwrap_or("") } else { "" };
    let desc_lines = if description.is_empty() { Vec::new() } else { ui::components::wrap(p.fonts, description, p.ty(Type::Body), 820.0) };
    let desc_h = desc_lines.len() as f32 * p.line(Type::Body);
    let content = density.logo_height()
        + 18.0
        + meta_h
        + if desc_h > 0.0 { 14.0 + desc_h } else { 0.0 }
        + 26.0
        + actions_h;
    hero_end = hero_end.max(p.y + 100.0 + content);
    let actions_y = hero_end - actions_h;
    stage(
        p,
        game.id,
        620.0,
        if p.mode == Mode::Light { hero_end + 40.0 } else { 360.0 },
        if p.mode == Mode::Light { hero_end + 84.0 } else { 700.0 },
        false,
        false,
    );
    p.solid(3, Frame::new(0.0, 0.0, W, H), p.ink(Role::Canvas, false), 0.0, 2);
    p.plan.nodes.last_mut().unwrap().props.opacity = if deep { if p.mode == Mode::Light { 1.0 } else { 0.5 } } else { 0.0 };
    let nav_end = nav(p, !deep);
    refresh_note(p, 40, nav_end, p.y);
    if !deep {
        let top = hero_end - content;
        logo(p, 100, game.id, Frame::new(p.x, top, 600.0, density.logo_height()));
        let my = top + density.logo_height() + 18.0;
        p.wrapped(101, &meta_text, Frame::new(p.x, my, 820.0, meta_h), Type::Label, Role::Ink, true, 10);
        if desc_h > 0.0 {
            p.wrapped(102, description, Frame::new(p.x, my + meta_h + 14.0, 820.0, desc_h), Type::Body, Role::Ink, true, 10);
        }
        let label = if p.model.play_target(game.id).is_some() { format!("Play  ·  {}", target_label(p, game.id)) } else { "Choose where to play".into() };
        let width = p.button(105, Focus::Hero(0), &label, Some(Icon::Play), p.x, actions_y, true, true, 12);
        let (x, y) = if p.scale > 1.0 { (p.x, actions_y + button_h + 16.0) } else { (p.x + width + 16.0, actions_y) };
        let details = p.button(110, Focus::Hero(1), "Game details", Some(Icon::Info), x, y, false, true, 12);
        let (x, y) = if p.scale > 1.0 { (p.x, y + button_h + 16.0) } else { (x + details + 16.0, y) };
        p.button(114, Focus::Hero(2), "Refresh libraries", Some(if p.model.refreshing() { Icon::Hourglass } else { Icon::Refresh }), x, y, false, true, 12);
    }
    let body = p.line(Type::Body);
    let caption = p.line(Type::Caption);
    let cover_h = m.prefs.covers.cover_width() * 1.5;
    let row_height = |row: &crate::game::model::home::HomeRow| {
        let tile = if matches!(row.items.first(), Some(HomeItem::Collection(_) | HomeItem::Genre(_))) { 252.0 } else { cover_h };
        body + 18.0 + tile + 26.0 + body + caption + 48.0
    };
    let hero_top = hero_end + 48.0;
    let before: f32 = m.home_rows.iter().take(m.view.home_row).map(&row_height).sum();
    let offset = before + if deep { hero_top - 170.0 } else { 0.0 };
    let current = p.scroll_positions.get(&1).copied().unwrap_or(offset);
    let mut top = hero_top;
    let mut current_row = 0;
    for (index, row) in m.home_rows.iter().enumerate() {
        if top + row_height(row) - current > 150.0 {
            current_row = index;
            break;
        }
        top += row_height(row);
    }
    let total: f32 = m.home_rows.iter().map(&row_height).sum();
    let group_index = p.plan.groups.len();
    p.plan.groups.push(super::paint::ScrollGroup {
        id: 1, parent: None, clip: compositor_protocol::Rect { x: 0.0, y: 150.0, w: W, h: H - 150.0 },
        feather: compositor_protocol::Edges::NONE, horizontal: false, offset, extent: total + hero_top, nodes: Vec::new(),
    });
    let start = p.plan.nodes.len();
    let mut row_top = hero_top;
    for (row_index, row) in m.home_rows.iter().enumerate() {
        let height = row_height(row);
        let visible = [m.view.home_row, current_row].iter().any(|&at| row_index >= at.saturating_sub(1) && row_index <= at + 2);
        if !visible {
            row_top += height;
            continue;
        }
        let y = row_top - offset;
        p.text(200 + row_index as u64 * 2000, &row.title, Frame::new(p.x, y, 1500.0, body), Type::Body, Role::Ink, false, 10);
        let tile_y = y + body + 18.0;
        let focused = match m.view.focus { Focus::Home(r, i) if r == row_index => Some(i), _ => None };
        let first = focused.map_or(0, |i| i.saturating_sub(m.prefs.covers.columns() - 2));
        let width_of = |item: &HomeItem| match item {
            HomeItem::Game(_) => m.prefs.covers.cover_width(),
            HomeItem::Collection(_) => 448.0,
            HomeItem::Genre(_) => 336.0,
        };
        // Home rows are built from one item kind, so their tile pitch is constant even in a large library.
        let Some(prototype) = row.items.first() else {
            row_top += height;
            continue;
        };
        let width = width_of(prototype);
        debug_assert!(row.items.iter().all(|item| width_of(item) == width));
        let pitch = width + p.gap;
        let x_offset = first as f32 * pitch;
        let id = 100 + row_index as u32;
        let current_x = p.scroll_positions.get(&id).copied().unwrap_or(x_offset);
        let current_item = ((current_x / pitch).floor().max(0.0) as usize).min(row.items.len() - 1);
        let count = ((W - p.x) / pitch).ceil() as usize;
        let mut items = Vec::with_capacity((count + 2) * 2);
        for at in [first, current_item] {
            for i in at.saturating_sub(1)..(at + count + 1).min(row.items.len()) {
                if !items.contains(&i) {
                    items.push(i);
                }
            }
        }
        items.sort_unstable();
        let nodes_start = p.plan.nodes.len();
        for i in items {
            let item = &row.items[i];
            let x = p.x + i as f32 * pitch - x_offset;
            let hits = p.plan.hits.len();
            let id = (1_u64 << 48) | ((row_index as u64) << 32) | ((i as u64) << 5);
            let focus = Focus::Home(row_index, i);
            match item {
                HomeItem::Game(game) => cover(p, id, *game, focus, x, tile_y, width),
                HomeItem::Collection(index) => collection(p, id, *index, focus, x, tile_y),
                HomeItem::Genre(name) => genre(p, id, name, focus, x, tile_y),
            }
            if p.plan.hits.len() > hits {
                p.plan.hit_groups.insert(focus, 100 + row_index as u32);
            }
        }
        let nodes = p.plan.nodes[nodes_start..].iter().map(|node| node.id).collect();
        p.plan.groups.push(super::paint::ScrollGroup {
            id, parent: Some(1), clip: compositor_protocol::Rect { x: p.x - p.gap, y: row_top, w: W - p.x + p.gap, h: height },
            feather: compositor_protocol::Edges::NONE, horizontal: true, offset: x_offset, extent: row.items.len() as f32 * pitch + W, nodes,
        });
        row_top += height;
    }
    p.plan.groups[group_index].nodes = p.plan.nodes[start..].iter().map(|node| node.id).collect();
}
fn collection(p: &mut Paint<'_>, id: u64, index: usize, focus: Focus, x: f32, y: f32) {
    let c = &p.model.state().profile().user.collections[index];
    let f = Frame::new(x, y, 448.0, 252.0);
    let from = p.plan.nodes.len();
    p.surface(id, f, Fill::Sheet, Radius::Tile, false, p.focused(focus), false, 10);
    if let Some(game) = c.games.first() {
        if let Some(path) = p.model.art_url(*game, ArtKind::Hero).and_then(|url| p.model.art_path(&url)) {
            p.image(id + 1, path, f, p.tokens.radius(Radius::Tile), false, None, 11);
        }
    }
    if p.focused(focus) {
        p.zoom(from, f, p.tokens.focus.tile_scale);
    }
    p.text(id + 2, &c.name, Frame::new(x, y + 278.0, 448.0, p.line(Type::Body)), Type::Body, Role::Ink, false, 12);
    p.text(
        id + 3,
        &format!("{} games", c.games.len()),
        Frame::new(x, y + 278.0 + p.line(Type::Body), 448.0, p.line(Type::Caption)),
        Type::Caption,
        Role::Ink2,
        false,
        12,
    );
    p.hit(focus, f);
}
fn genre(p: &mut Paint<'_>, id: u64, name: &str, focus: Focus, x: f32, y: f32) {
    let f = Frame::new(x, y, 336.0, 252.0);
    let from = p.plan.nodes.len();
    p.surface(id, f, Fill::Sheet, Radius::Tile, false, p.focused(focus), false, 10);
    let ids = p.model.in_genre(name);
    p.text(id + 1, name, Frame::new(x + 28.0, y + 28.0, 280.0, p.line(Type::Body)), Type::Body, Role::Ink, false, 12);
    p.text(
        id + 2,
        &format!("{} games", ids.len()),
        Frame::new(x + 28.0, y + 28.0 + p.line(Type::Body), 280.0, p.line(Type::Caption)),
        Type::Caption,
        Role::Ink2,
        false,
        12,
    );
    for (i, game) in ids.iter().take(3).enumerate() {
        if let Some(path) = p.model.art_url(*game, ArtKind::Cover).and_then(|url| p.model.art_path(&url)) {
            p.image(id + 3 + i as u64, path, Frame::new(x + 24.0 + i as f32 * 100.0, y + 160.0, 88.0, 92.0), p.tokens.radius(Radius::Control), false, None, 11);
        }
    }
    p.hit(focus, f);
    if p.focused(focus) {
        p.zoom(from, f, p.tokens.focus.tile_scale);
    }
}
