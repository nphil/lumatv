use super::{
    art::Stage,
    paint::{Fill, H, Paint, W, status_word},
};
use crate::game::model::{Density, Focus, HomeItem, HubTab, Key, Page};
use lumagame_core::{
    catalog::{ArtKind, SaveCompat},
    engine::EngineKind,
    ids::GameId,
};
use lumatv_tokens::{Radius, TypeRole as Type};
use std::path::PathBuf;
use ui::{
    components::Icon,
    geometry::Frame,
    lucent::palette::{Mode, PaletteRole as Role},
};

/// The page's canvas. A curtain page is all stage: its text is light in every theme, so it never sits on a light canvas.
pub fn canvas(p: &mut Paint<'_>, stage: bool) {
    p.solid(1, Frame::new(0.0, 0.0, W, H), p.ink(Role::Canvas, stage), 0.0, 0);
}
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
                super::symbols::Symbol::Library,
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
pub fn stage(p: &mut Paint<'_>, game: GameId, art_h: f32, fade_from: f32, fade_to: f32, dim: bool, curtain: bool) {
    if let Some(path) = p.model.art_url(game, ArtKind::Hero).and_then(|url| p.model.art_path(&url)) {
        let c = p.straight(Role::Canvas, curtain);
        p.image(
            2,
            path,
            Frame::new(0.0, 0.0, W, H),
            0.0,
            false,
            Some(Stage { art_height: art_h as u32, fade_from: fade_from as u32, fade_to: fade_to as u32, canvas: [c.r, c.g, c.b], dim, curtain }),
            1,
        );
    }
}
/// The game's logo art, when it has some.
pub fn logo_art(p: &Paint<'_>, game: GameId) -> Option<PathBuf> {
    p.model.art_url(game, ArtKind::Logo).and_then(|url| p.model.art_path(&url))
}
/// Whether the game has hero art, which the text over it takes its (stage) ink from; without art the page's own ink applies.
fn has_hero(p: &Paint<'_>, game: GameId) -> bool {
    p.model.art_url(game, ArtKind::Hero).and_then(|url| p.model.art_path(&url)).is_some()
}
/// The game's logo, or its title in text when it has no logo art: as many lines as `frame` is tall, never more, so a long title
/// ends in an ellipsis instead of running down into what sits under the frame.
pub fn logo(p: &mut Paint<'_>, id: u64, game: GameId, frame: Frame) {
    let on_art = has_hero(p, game);
    if let Some(path) = logo_art(p, game) {
        p.image(id, path, frame, 0.0, true, None, 10);
    } else if let Some(game) = p.model.game(game) {
        let lines = p.fit(&game.title, Type::Title, frame.w, p.rows(Type::Title, frame.h));
        p.lines(id, lines, frame, Type::Title, Role::Ink, on_art, 10);
    }
}
fn target_label(p: &Paint<'_>, game: GameId) -> String {
    p.model.play_target(game).map(|key| p.model.engine_name(&key.engine).to_string()).unwrap_or_else(|| "Choose where to play".into())
}
#[allow(clippy::too_many_arguments)]
pub fn target_line(p: &mut Paint<'_>, id: u64, game: GameId, x: f32, y: f32, w: f32, stage: bool, backed: bool, with_target: bool) {
    let Some(key) = p.model.play_target(game) else {
        return;
    };
    let target = p.model.target(&key);
    let mut text = p.model.engine_name(&key.engine).to_string();
    if with_target {
        if let Some(target) = target {
            if target.name != text {
                text.push_str("  ·  ");
                text.push_str(&target.name);
            }
        }
    }
    let (ready, role) = status_word(p.model, &key);
    let base = p.width(&text, Type::Caption) + 46.0;
    let f = Frame::new(x, y, w, p.line(Type::Caption));
    let icon = match p.model.integration(&key.engine).map(|e| e.manifest.kind) {
        Some(EngineKind::Cloud) => Icon::CloudOutline,
        Some(EngineKind::Host) => Icon::Cards,
        Some(EngineKind::Account) => Icon::Person,
        _ => Icon::Package,
    };
    if backed {
        let width = base + 22.0 + p.width(ready, Type::Caption) + 30.0;
        p.surface(id + 100, Frame::new(x - 14.0, y - 6.0, width, f.h + 12.0), Fill::Sheet, Radius::Pill, false, false, false, 10);
    }
    p.words(
        id,
        f,
        vec![
            p.word(text, Type::Caption, p.ink(Role::Ink2, stage), 38.0, 0.0, base - 38.0),
            p.word(ready, Type::Caption, p.ink(Role::Ink, stage), base + 22.0, 0.0, w - base - 22.0),
        ],
        vec![
            p.mark(icon, 26.0, 0.0, 4.0, Role::Ink2, stage),
            super::paint::Mark { form: ui::components::Form::Disc(12), colour: p.ink(role, stage), x: base, y: 11.0 },
        ],
        11,
    );
}
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
        meta_parts.push(format!("{} played", crate::game::model::play_time(seconds)));
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
    let row_height = |row: &crate::game::model::HomeRow| {
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
pub fn cover(p: &mut Paint<'_>, id: u64, game: GameId, focus: Focus, x: f32, y: f32, w: f32) {
    let Some(game) = p.model.game(game) else {
        return;
    };
    let f = Frame::new(x, y, w, w * 1.5);
    let focused = p.focused(focus);
    let start = p.plan.nodes.len();
    p.surface(id, f, Fill::Sheet, Radius::Tile, false, focused, false, 10);
    let count = ui::components::wrap(p.fonts, &game.title, p.bold(Type::Body), w - 48.0).len();
    let h = count as f32 * p.line(Type::Body);
    p.wrapped(id + 1, &game.title, Frame::new(x + 24.0, y + (f.h - h) / 2.0, w - 48.0, h), Type::Body, Role::Ink, false, 11);
    if let Some(path) = p.model.art_url(game.id, ArtKind::Cover).and_then(|url| p.model.art_path(&url)) {
        p.image(id + 5, path, f, p.tokens.radius(Radius::Tile), false, None, 12);
    }
    if focused {
        p.zoom(start, f, p.tokens.focus.tile_scale);
        let caption = Frame::new(x, y + f.h + 26.0, (w * 2.2).min(W - p.x - x), p.line(Type::Body));
        p.text(id + 3, &game.title, caption, Type::Body, Role::Ink, false, 15);
        target_line(p, id + 4, game.id, x, caption.y + caption.h, w * 2.2, false, false, false);
    }
    p.hit(focus, f);
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
pub fn library(p: &mut Paint<'_>, search: bool) {
    let nav_end = nav(p, false);
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
        let face = p.pad_face(Key::Menu);
        let side = 30.0 * p.scale;
        let extra = |i: usize| if face.is_some() && i + 2 == labels.len() { side + 4.0 } else { 0.0 };
        let widths: Vec<_> = labels.iter().enumerate().map(|(i, s)| p.width(s, Type::Label) + 52.0 + extra(i)).collect();
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
                    Frame::new(x + 26.0 + extra(i), y + (h - p.line(Type::Label)) / 2.0, f.w - 52.0 - extra(i), p.line(Type::Label)),
                    Type::Label,
                    Role::Ink,
                    false,
                    11,
                );
                if let (Some(face), true) = (face, extra(i) > 0.0) {
                    p.pad_glyph(102 + i as u64 * 4, face, Frame::new(x + 18.0, y + (h - side) / 2.0, side, side), false, false, 11);
                }
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
pub fn keyboard(p: &mut Paint<'_>, id: u64, x: f32, y: f32, w: f32, numeric: bool, stage: bool) {
    let diagnostic = p.model.view.page == Page::Settings && p.model.view.section == crate::game::model::Section::Diagnostics && p.model.view.sheet == Some(crate::game::model::Sheet::EditText);
    let letters = match (p.model.view.keyboard_lower, diagnostic) {
        (true, true) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/_",
        (false, true) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/_",
        (true, false) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/",
        (false, false) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/",
    };
    let keys: Vec<String> = if numeric {
        "1234567890".chars().map(|n| n.to_string()).chain(["⌫".into(), "Done".into()]).collect()
    } else {
        letters
            .chars()
            .map(|c| c.to_string())
            .chain(["⌫".into(), "Space".into(), "Done".into(), if p.model.view.keyboard_lower { "ABC" } else { "abc" }.into()])
            .collect()
    };
    let columns = if numeric { 6 } else { 10 };
    let gap = 8.0;
    let width = (w - gap * (columns - 1) as f32) / columns as f32;
    let h = (52.0 * p.scale).min(80.0);
    for (i, key) in keys.iter().enumerate() {
        let f = Frame::new(x + (i % columns) as f32 * (width + gap), y + (i / columns) as f32 * (h + gap), width, h);
        let from = p.plan.nodes.len();
        p.surface(id + i as u64 * 3, f, Fill::Tint(0.06), Radius::Control, stage, p.focused(Focus::Keyboard(i)), false, 30);
        let mut word = p.word(key, Type::Caption, p.ink(Role::Ink, stage), 2.0, (h - p.line(Type::Caption)) / 2.0, width - 4.0);
        word.align = ui::components::Align::Centre;
        p.words(id + i as u64 * 3 + 1, f, vec![word], vec![], 31);
        if p.focused(Focus::Keyboard(i)) {
            p.zoom(from, f, p.tokens.focus.button_scale);
        }
        p.hit(Focus::Keyboard(i), f);
    }
}
pub fn hub(p: &mut Paint<'_>) {
    let m = p.model;
    let Some(game) = m.selected_game() else {
        return;
    };
    let deep = m.view.hub_tab > 0;
    let details = m.details.get(&game.id);
    // The hero art fades into the page's canvas between these heights. In the light theme ink over the art is the stage's (light) ink
    // and ink over the canvas below it the page's (dark) ink, so a row that would sit across the fade starts below it.
    let (fade_from, fade_to) = (if p.mode == Mode::Light { 740.0 } else { 540.0 }, 800.0);
    let on_art = !deep && has_hero(p, game.id);
    let light_hero = on_art && p.mode == Mode::Light;
    if !deep || p.mode == Mode::Dark {
        stage(p, game.id, 760.0, fade_from, fade_to, deep, false);
    }
    let topstage = on_art;
    p.icon_button(10, Focus::HubTop(0), Icon::Back, p.x, p.y, topstage, 60);
    p.text(20, &game.title, Frame::new(p.x + 88.0, p.y + 12.0, 1200.0, p.line(Type::Label)), Type::Label, Role::Ink, topstage, 62);
    p.symbol_button(30, Focus::HubTop(1), super::symbols::Symbol::Heart, m.state().profile().user.favourites.contains(&game.id), W - p.x - 144.0, p.y, topstage, 60);
    p.symbol_button(40, Focus::HubTop(2), super::symbols::Symbol::More, false, W - p.x - 64.0, p.y, topstage, 60);
    let mut y = 170.0;
    if !deep {
        y = 214.0;
        logo(p, 100, game.id, Frame::new(p.x, y, 600.0, 168.0));
        y += 188.0;
        let mut meta = m.game_genres(game.id);
        meta.extend(m.game_tags(game.id).into_iter().take(3));
        if let Some(year) = details.and_then(|d| d.year) {
            meta.push(year.to_string());
        }
        if let Some(players) = details.and_then(|d| d.players.as_ref()) {
            meta.push(players.clone());
        }
        if !meta.is_empty() {
            y += p.wrapped(101, &meta.join("  ·  "), Frame::new(p.x, y, 1000.0, 0.0), Type::Label, Role::Ink, on_art, 10) + 10.0;
        }
        if let Some(description) = details.map(|d| d.description.as_str()).filter(|s| !s.is_empty()) {
            y += p.wrapped(102, description, Frame::new(p.x, y, 820.0, 0.0), Type::Body, Role::Ink, on_art, 10);
        }
        let mut chips = details.map(|d| d.capabilities.clone()).unwrap_or_default();
        chips.extend(m.favourite_on(game.id).into_iter().map(|name| format!("Favourite on {name}")));
        if !chips.is_empty() {
            y += 20.0;
            let mut x = p.x;
            for (i, label) in chips.iter().enumerate() {
                let width = p.width(label, Type::Caption) + 26.0 * p.scale + 34.0;
                if x + width > p.x + 1100.0 {
                    x = p.x;
                    y += p.line(Type::Caption) + 20.0;
                }
                x += p.status(110 + i as u64 * 3, label, Role::Positive, x, y, on_art, 10) + 12.0;
            }
            y += 44.0f32.max(p.line(Type::Caption) + 8.0);
        }
        y += 30.0;
        let label = if p.model.play_target(game.id).is_some() { format!("Play  ·  {}", target_label(p, game.id)) } else { "Choose where to play".into() };
        let w = p.button(140, Focus::HubAction(0), &label, Some(Icon::Play), p.x, y, on_art, true, 12);
        let other = "Choose where to play";
        let other_w = p.width(other, Type::Label) + 110.0;
        if p.scale > 1.0 || p.x + w + 16.0 + other_w > W - p.x {
            y += p.tokens.target.max(p.line(Type::Label) + 24.0) + 16.0;
            p.button(145, Focus::HubAction(1), other, Some(Icon::Refresh), p.x, y, false, true, 12);
        } else {
            p.button(145, Focus::HubAction(1), other, Some(Icon::Refresh), p.x + w + 16.0, y, false, true, 12);
        }
        y += p.tokens.target.max(p.line(Type::Label) + 24.0) + 16.0;
        if let Some(notice) = &m.view.notice {
            p.surface(150, Frame::new(p.x, y, 1100.0, p.line(Type::Label) * 2.0 + 32.0), Fill::Canvas, Radius::Row, false, false, false, 20);
            y += p.wrapped(151, notice, Frame::new(p.x + 24.0, y + 16.0, 1052.0, 0.0), Type::Label, Role::Ink, false, 21) + 32.0;
        } else {
            let line = p.line(Type::Caption);
            if light_hero && y < fade_to && y + line > fade_from {
                y = fade_to;
            }
            let on_stage = on_art && (!light_hero || y + line <= fade_from);
            // Scrolled up over the art at large text sizes, the page's dark ink needs the sheet behind it.
            target_line(p, 150, game.id, p.x, y, 1200.0, on_stage, light_hero && !on_stage, true);
            y += line;
        }
        y += 42.0;
    }
    let tab_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    if light_hero && y < fade_to && y + tab_h > fade_from {
        y = fade_to;
    }
    // Over the hero art the page's (dark) ink would be faint in the light theme, and the art can sit behind the row at any scroll
    // position, so the row and the tiles below it sit on an opaque capsule or sheet there.
    if light_hero {
        let total: f32 = m.hub_tabs().iter().map(|t| p.width(t.label(), Type::Label) + 66.0).sum();
        p.surface(199, Frame::new(p.x - 8.0, y - 6.0, total + 8.0, tab_h + 12.0), Fill::Sheet, Radius::Pill, false, false, false, 14);
    }
    let mut x = p.x;
    for (i, label) in m.hub_tabs().into_iter().map(|tab| tab.label()).enumerate() {
        let w = p.width(label, Type::Label) + 58.0;
        let f = Frame::new(x, y, w, tab_h);
        let from = p.plan.nodes.len();
        if i == m.view.hub_tab || p.focused(Focus::Tab(i)) {
            p.surface(200 + i as u64 * 3, f, Fill::Tint(if p.focused(Focus::Tab(i)) { 0.16 } else { 0.09 }), Radius::Pill, false, false, false, 15);
        }
        p.text(
            201 + i as u64 * 3,
            label,
            Frame::new(x + 29.0, y + (tab_h - p.line(Type::Label)) / 2.0, w - 58.0, p.line(Type::Label)),
            Type::Label,
            Role::Ink,
            false,
            16,
        );
        if p.focused(Focus::Tab(i)) {
            p.zoom(from, f, p.tokens.focus.button_scale);
        }
        p.hit(Focus::Tab(i), f);
        x += w + 8.0;
    }
    y += tab_h + 28.0;
    match m.hub_tab() {
        HubTab::Overview => {
            let last_label = match m.last_played(game.id).map(|last| last.place) {
                Some(lumagame_core::activity::Place::Service(engine)) => format!("Last played on {}", m.engine_name(&engine)),
                _ => "Last played".to_string(),
            };
            let play = m.play_time_of(game.id);
            let play_label = format!("Play time on {}", play.as_ref().map_or("this TV".to_string(), |(_, place)| m.place_name(place)));
            let cols = if p.scale > 1.0 { 2 } else { 4 };
            let w = (W - p.x * 2.0 - p.gap * (cols - 1) as f32) / cols as f32;
            let mut facts = vec![(last_label, Some(m.since_played(game.id).unwrap_or_else(|| "Not played yet".into()))), (play_label, play.map(|(seconds, _)| crate::game::model::play_time(seconds)))];
            facts.push(("Player reviews".to_string(), details.and_then(|d| d.hub.review.as_ref()).map(|r| r.label.clone())));
            facts.push(("Achievements".to_string(), achievement_summary(m, game.id, details)));
            facts.push(("Played in the last 2 weeks".to_string(), m.recent_play(game.id).map(crate::game::model::play_time)));
            facts.push(("Metacritic".to_string(), details.and_then(|d| d.hub.metacritic).map(|s| format!("{s} out of 100"))));
            facts.push(("Released".to_string(), details.and_then(|d| d.hub.release.clone())));
            facts.push(("Saves".to_string(), details.and_then(|d| d.cloud_saves).map(|cloud| if cloud { "Cloud saves" } else { "Local saves" }.into())));
            // Only what is known is shown, and as many tiles as the page has room for.
            let facts: Vec<(String, String)> = facts.into_iter().filter_map(|(label, value)| Some((label, value?))).take(4).collect();
            for (i, (label, value)) in facts.into_iter().enumerate() {
                let h = p.line(Type::Caption) + p.line(Type::Body) + 48.0;
                let f = Frame::new(p.x + (i % cols) as f32 * (w + p.gap), y + (i / cols) as f32 * (h + p.gap), w, h);
                p.surface(300 + i as u64 * 2, f, if light_hero { Fill::Sheet } else { Fill::Glass }, Radius::Tile, false, false, false, 10);
                p.words(
                    301 + i as u64 * 2,
                    f,
                    vec![
                        p.word(label, Type::Caption, p.ink(Role::Ink2, false), 24.0, 20.0, w - 48.0),
                        p.word(value, Type::Body, p.ink(Role::Ink, false), 24.0, 20.0 + p.line(Type::Caption), w - 48.0),
                    ],
                    vec![],
                    11,
                );
            }
        }
        HubTab::Ways => {
            let focused = if let Focus::Way(i) = m.view.focus { i } else { 0 };
            let first = focused.saturating_sub(if p.scale > 1.0 { 2 } else { 6 });
            for (i, source) in game.sources.iter().enumerate().skip(first) {
                if y > H - 100.0 {
                    break;
                }
                y += way(p, 300 + i as u64 * 20, i, source.key(), Frame::new(p.x, y, 1248.0, 0.0), false) + 6.0;
            }
            p.button(700, Focus::Way(game.sources.len()), "Add a way to play", Some(Icon::Plus), p.x, y + 16.0, false, false, 10);
        }
        HubTab::Achievements => achievements_tab(p, game.id, y),
        HubTab::News => news_tab(p, game.id, y),
        HubTab::Details => details_tab(p, game.id, y),
        HubTab::Media => {
            let media = details.map(|d| d.media.as_slice()).unwrap_or(&[]);
            if media.is_empty() {
                p.text(300, "No media supplied by the connected sources", Frame::new(p.x, y, 1500.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
            } else {
                for (video, label) in [(true, "Trailers"), (false, "Screenshots")] {
                    let items = media.iter().enumerate().filter(|(_, item)| item.video.is_some() == video).collect::<Vec<_>>();
                    if items.is_empty() {
                        continue;
                    }
                    p.text(if video { 270 } else { 271 }, label, Frame::new(p.x, y, W - p.x * 2.0, p.line(Type::Body)), Type::Body, Role::Ink, false, 10);
                    y += p.line(Type::Body) + 24.0;
                    let first =
                        if let Focus::Media(index) = m.view.focus { items.iter().position(|(i, _)| *i == index).unwrap_or(0).saturating_sub(2) } else { 0 };
                    for (column, (i, item)) in items.iter().skip(first).take(4).enumerate() {
                        let x = p.x + column as f32 * (448.0 + p.gap);
                        let f = Frame::new(x, y, 448.0, 252.0);
                        let from = p.plan.nodes.len();
                        p.surface(300 + *i as u64 * 10, f, Fill::Sheet, Radius::Tile, false, p.focused(Focus::Media(*i)), false, 10);
                        if let Some(path) = m.art_path(&item.image) {
                            p.image(301 + *i as u64 * 10, path, f, p.tokens.radius(Radius::Tile), false, None, 11);
                        }
                        if video {
                            p.solid(302 + *i as u64 * 10, Frame::new(x + 188.0, y + 90.0, 72.0, 72.0), p.ink(Role::Sheet, true), 36.0, 12);
                            p.words(303 + *i as u64 * 10, f, vec![], vec![p.mark(Icon::Play, 32.0, 208.0, 110.0, Role::Ink, true)], 13);
                        }
                        if p.focused(Focus::Media(*i)) {
                            p.zoom(from, f, p.tokens.focus.tile_scale);
                        }
                        p.text(304 + *i as u64 * 10, &item.title, Frame::new(x, y + 278.0, 448.0, p.line(Type::Body) * 2.0), Type::Body, Role::Ink, false, 12);
                        p.hit(Focus::Media(*i), f);
                    }
                    y += 278.0 + p.line(Type::Body) * 2.0 + 32.0;
                }
            }
        }
    }
}
/// "18 of 51 unlocked" from the owner's own list, or the store's count of what the game has.
fn achievement_summary(m: &crate::game::model::Model, id: GameId, details: Option<&crate::game::model::GameDetails>) -> Option<String> {
    let total = details.map_or(0, |d| d.hub.achievements.total);
    match (m.unlocked(id), total) {
        (Some(unlocked), 0) => Some(format!("{} unlocked", unlocked.len())),
        (Some(unlocked), total) => Some(format!("{} of {total} unlocked", unlocked.len())),
        (None, 0) => None,
        (None, total) => Some(format!("{total} to earn")),
    }
}
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
/// One row of a section list: an optional picture, a title, up to `lines` lines of detail and a short word at the right. Returns its height.
#[allow(clippy::too_many_arguments)]
fn item_row(p: &mut Paint<'_>, id: u64, index: usize, f: Frame, icon: Option<PathBuf>, title: &str, detail: &str, lines: usize, side: Option<(String, Role)>) -> f32 {
    let focus = Focus::Item(index);
    let focused = p.focused(focus);
    let pad = 24.0;
    let text_x = pad + if icon.is_some() { 92.0 } else { 0.0 };
    let side_w = side.as_ref().map_or(0.0, |(s, _)| p.width(s, Type::Caption) + 32.0);
    let text_w = f.w - text_x - pad - side_w;
    let detail_lines = if detail.is_empty() { Vec::new() } else { p.fit(detail, Type::Caption, text_w, lines) };
    let text_h = p.line(Type::Body) + detail_lines.len() as f32 * p.line(Type::Caption);
    let f = Frame::new(f.x, f.y, f.w, (text_h + pad * 2.0).max(if icon.is_some() { 72.0 + pad * 2.0 } else { 0.0 }));
    let from = p.plan.nodes.len();
    p.surface(id, f, Fill::Tint(if focused { p.tokens.states.focus_wash[usize::from(p.mode == Mode::Light)] } else { 0.04 }), Radius::Row, false, focused, false, 32);
    if let Some(path) = icon {
        p.image(id + 1, path, Frame::new(f.x + pad, f.y + (f.h - 72.0) / 2.0, 72.0, 72.0), p.tokens.radius(Radius::Control), false, None, 33);
    }
    let top = (f.h - text_h) / 2.0;
    let mut words = vec![p.word(title, Type::Body, p.ink(Role::Ink, false), text_x, top, text_w)];
    for (i, line) in detail_lines.iter().enumerate() {
        words.push(p.word(line.clone(), Type::Caption, p.ink(Role::Ink2, false), text_x, top + p.line(Type::Body) + i as f32 * p.line(Type::Caption), text_w));
    }
    if let Some((label, role)) = side {
        words.push(p.word(label, Type::Caption, p.ink(role, false), f.w - pad - side_w + 32.0, (f.h - p.line(Type::Caption)) / 2.0, side_w - 32.0));
    }
    p.words(id + 2, f, words, vec![], 34);
    if focused {
        p.zoom(from, f, p.tokens.focus.card_scale);
    }
    p.hit(focus, f);
    f.h
}
fn achievements_tab(p: &mut Paint<'_>, game: GameId, mut y: f32) {
    let m = p.model;
    let Some(hub) = m.details.get(&game).map(|d| &d.hub) else { return };
    let summary = achievement_summary(m, game, m.details.get(&game)).unwrap_or_default();
    y += p.wrapped(300, &summary, Frame::new(p.x, y, W - p.x * 2.0, 0.0), Type::Body, Role::Ink, false, 10) + 6.0;
    if let Some(rarest) = hub.achievements.rarest {
        y += p.wrapped(301, &format!("The rarest is earned by {}% of players.", percent(rarest)), Frame::new(p.x, y, W - p.x * 2.0, 0.0), Type::Caption, Role::Ink2, false, 10);
    }
    y += 16.0;
    let unlocked = m.unlocked(game);
    for (i, a) in hub.achievements.shown.iter().enumerate() {
        let state = match (unlocked, &a.api) {
            (Some(list), Some(api)) => Some(if list.iter().any(|u| &u.name == api) { ("Unlocked".to_string(), Role::Positive) } else { ("Not yet".to_string(), Role::Ink2) }),
            _ => None,
        };
        let share = a.permille.map(|s| format!("{}% of players have it", percent(s)));
        y += item_row(p, 400 + i as u64 * 4, i, Frame::new(p.x, y, W - p.x * 2.0, 0.0), m.art_path(&a.icon), &a.name, share.as_deref().unwrap_or(""), 1, state) + 8.0;
    }
    if !hub.achievements.shown.is_empty() {
        p.wrapped(380, "The list shows the achievements the Steam store highlights.", Frame::new(p.x, y + 8.0, W - p.x * 2.0, 0.0), Type::Caption, Role::Ink2, false, 10);
    }
}
fn percent(permille: u32) -> String {
    if permille % 10 == 0 { (permille / 10).to_string() } else { format!("{}.{}", permille / 10, permille % 10) }
}
fn news_tab(p: &mut Paint<'_>, game: GameId, mut y: f32) {
    let m = p.model;
    let Some(hub) = m.details.get(&game).map(|d| &d.hub) else { return };
    for (i, n) in hub.news.iter().enumerate() {
        let when = lumagame_meta::hub::date(n.date);
        let by = if n.author.is_empty() { when } else { format!("{when}  ·  {}", n.author) };
        let lines = if p.scale > 1.0 { 2 } else { 3 };
        let detail = if n.summary.is_empty() { by } else { format!("{by}  ·  {}", n.summary) };
        y += item_row(p, 400 + i as u64 * 4, i, Frame::new(p.x, y, W - p.x * 2.0, 0.0), None, &n.title, &detail, lines, Some(("Open".into(), Role::Ink2))) + 8.0;
    }
}
fn details_tab(p: &mut Paint<'_>, game: GameId, mut y: f32) {
    let m = p.model;
    let Some(hub) = m.details.get(&game).map(|d| &d.hub) else { return };
    let join = |v: &[String]| (!v.is_empty()).then(|| v.join(", "));
    let deck = hub.deck.map(|d| match d {
        lumagame_meta::hub::Deck::Verified => "Verified",
        lumagame_meta::hub::Deck::Playable => "Playable",
        lumagame_meta::hub::Deck::Unsupported => "Unsupported",
    });
    let reviews = hub.review.as_ref().map(|r| match r.percent {
        Some(percent) => format!("{}, {percent}% of {} reviews", r.label, thousands(r.count)),
        None => r.label.clone(),
    });
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut add = |label: &str, value: Option<String>| rows.extend(value.map(|v| (label.to_string(), v)));
    add("Developer", join(&hub.developers));
    add("Publisher", join(&hub.publishers));
    add("Released", hub.release.clone());
    add("Steam Deck", deck.map(str::to_owned));
    add("Player reviews", reviews);
    add("Metacritic", hub.metacritic.map(|s| format!("{s} out of 100")));
    add("Features", join(&hub.categories));
    add("Extra content", (hub.dlc > 0).then(|| format!("{} add-ons", hub.dlc)));
    for fact in &hub.facts {
        add(&fact.label, Some(fact.value.clone()));
    }
    let cols = if p.scale > 1.0 { 1 } else { 2 };
    let w = (W - p.x * 2.0 - p.gap * (cols - 1) as f32) / cols as f32;
    // A long value (the feature list) wraps at word boundaries; a row is as tall as its tallest cell.
    let (caption, body) = (p.line(Type::Caption), p.line(Type::Body));
    let cells: Vec<(String, Vec<String>)> = rows.into_iter().map(|(label, value)| (label, p.fit(&value, Type::Body, w - 48.0, 3))).collect();
    let mut tops = Vec::new();
    let mut cursor = y;
    for pair in cells.chunks(cols) {
        let h = pair.iter().map(|(_, lines)| caption + lines.len() as f32 * body + 32.0).fold(0.0, f32::max);
        tops.push((cursor, h));
        cursor += h + 8.0;
    }
    for (i, (label, lines)) in cells.into_iter().enumerate() {
        let (top, h) = tops[i / cols];
        let f = Frame::new(p.x + (i % cols) as f32 * (w + p.gap), top, w, h);
        p.surface(300 + i as u64 * 2, f, Fill::Glass, Radius::Row, false, false, false, 10);
        p.words(301 + i as u64 * 2, f, vec![p.word(label, Type::Caption, p.ink(Role::Ink2, false), 24.0, 14.0, w - 48.0)], vec![], 11);
        let value = Frame::new(f.x + 24.0, f.y + 14.0 + caption, w - 48.0, lines.len() as f32 * body);
        p.lines(600 + i as u64, lines, value, Type::Body, Role::Ink, false, 11);
    }
    y = cursor + 16.0;
    let credits = hub.credits.join("  ");
    p.wrapped(290, &credits, Frame::new(p.x, y, W - p.x * 2.0, 0.0), Type::Caption, Role::Ink2, false, 10);
}
pub fn way(p: &mut Paint<'_>, id: u64, index: usize, key: lumagame_core::ids::SourceKey, mut f: Frame, sheet: bool) -> f32 {
    let focus = if sheet { Focus::SheetRow(index) } else { Focus::Way(index) };
    let focused = p.focused(focus);
    let target = p.model.target(&key);
    let name = p.model.engine_name(&key.engine);
    let sub = target.map(|t| t.name.as_str()).unwrap_or("Target unavailable");
    let (status, role) = status_word(p.model, &key);
    let save = match p.model.save_compat(&key) {
        SaveCompat::Shared => "Shared saves",
        SaveCompat::Separate => "Separate saves",
        SaveCompat::Unknown => "Saves unknown",
    };
    f.h = if p.scale > 1.0 { p.line(Type::Body) + p.line(Type::Caption) * 2.0 + 44.0 } else { 80.0 };
    let from = p.plan.nodes.len();
    p.surface(
        id,
        f,
        Fill::Tint(if focused { p.tokens.states.focus_wash[usize::from(p.mode == Mode::Light)] } else { 0.04 }),
        Radius::Row,
        false,
        focused,
        false,
        32,
    );
    let icon = match p.model.integration(&key.engine).map(|e| e.manifest.kind) {
        Some(EngineKind::Cloud) => Icon::CloudOutline,
        Some(EngineKind::Host) => Icon::Cards,
        Some(EngineKind::Account) => Icon::Person,
        _ => Icon::Package,
    };
    let mut words = vec![p.word(name, Type::Body, p.ink(Role::Ink, false), 102.0, if p.scale > 1.0 { 16.0 } else { 10.0 }, f.w - 200.0)];
    let mut marks = vec![p.mark(icon, 40.0, 30.0, 20.0, Role::Ink2, false)];
    if p.scale > 1.0 {
        words.push(p.word(format!("{sub}  ·  {save}"), Type::Caption, p.ink(Role::Ink2, false), 102.0, 16.0 + p.line(Type::Body), f.w - 150.0));
        words.push(p.word(status, Type::Caption, p.ink(role, false), 102.0, 16.0 + p.line(Type::Body) + p.line(Type::Caption), f.w - 150.0));
    } else {
        words.push(p.word(sub, Type::Caption, p.ink(Role::Ink2, false), 102.0, 43.0, (f.w - 740.0).max(180.0)));
        words.push(p.word(save, Type::Caption, p.ink(Role::Ink2, false), f.w - 610.0, 24.0, 230.0));
        words.push(p.word(status, Type::Caption, p.ink(role, false), f.w - 300.0, 24.0, 230.0));
    }
    if sheet {
        let selected = p.model.view.chooser_selected.as_ref() == Some(&key);
        marks.push(p.mark(
            if selected { Icon::Tick } else { Icon::Circle },
            28.0,
            f.w - 54.0,
            (f.h - 28.0) / 2.0,
            if selected { Role::Accent } else { Role::Ink2 },
            false,
        ));
    }
    p.words(id + 1, f, words, marks, 33);
    if focused {
        p.zoom(from, f, p.tokens.focus.card_scale);
    }
    p.hit(focus, f);
    f.h
}
pub fn first_run(p: &mut Paint<'_>) {
    nav(p, false);
    let title = "Add a way to play";
    let y = if p.scale > 1.0 { 220.0 } else { 300.0 };
    let title_h = p.wrapped(100, title, Frame::new(p.x, y, W - p.x * 2.0, 0.0), Type::Display, Role::Ink, false, 10);
    let sub = "Connect a service or a PC. Games appear here after a library is linked.";
    let sub_h = p.wrapped(101, sub, Frame::new(p.x, y + title_h + 12.0, W - p.x * 2.0, 0.0), Type::Body, Role::Ink2, false, 10);
    let top = y + title_h + sub_h + 68.0;
    let cols = if p.scale > 1.0 { 2 } else { 4 };
    let w = (W - p.x * 2.0 - p.gap * (cols - 1) as f32) / cols as f32;
    let focus = if let Focus::FirstRun(i) = p.model.view.focus { i } else { 0 };
    let start = focus / cols * cols;
    for (i, e) in p.model.integrations.iter().enumerate().skip(start).take(cols) {
        let f = Frame::new(p.x + (i - start) as f32 * (w + p.gap), top, w, 320.0);
        let from = p.plan.nodes.len();
        p.surface(200 + i as u64 * 10, f, Fill::Glass, Radius::Card, false, p.focused(Focus::FirstRun(i)), false, 10);
        let icon = match e.manifest.kind {
            EngineKind::Cloud => Icon::CloudOutline,
            EngineKind::Host => Icon::Tiles,
            EngineKind::Account => Icon::Person,
            _ => Icon::Package,
        };
        p.words(201 + i as u64 * 10, f, vec![], vec![p.mark(icon, 64.0, 28.0, 28.0, Role::Ink, false)], 11);
        let used = p.wrapped(202 + i as u64 * 10, &e.manifest.name, Frame::new(f.x + 28.0, f.y + 116.0, w - 56.0, 0.0), Type::Body, Role::Ink, false, 11);
        p.text(
            203 + i as u64 * 10,
            if e.manifest.kind == EngineKind::Host {
                "Pair a PC"
            } else if e.manifest.kind == EngineKind::Native {
                "Choose apps"
            } else {
                "Sign in"
            },
            Frame::new(f.x + 28.0, f.y + 132.0 + used, w - 56.0, p.line(Type::Caption)),
            Type::Caption,
            Role::Ink2,
            false,
            11,
        );
        if p.focused(Focus::FirstRun(i)) {
            p.zoom(from, f, p.tokens.focus.card_scale);
        }
        p.hit(Focus::FirstRun(i), f);
    }
    if p.model.integrations.is_empty() {
        p.text(200, "No integrations are installed", Frame::new(p.x, top, 1400.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    }
}
