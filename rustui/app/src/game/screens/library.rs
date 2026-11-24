//! The Library page: the filter panel (or, searching, the keys) on the left, five covers across beside it, and the Refine lists as a
//! drawer over the panel.
//!
//! Node ids: 100 the panel, 101 the query, 104 its field, 105 the words of every panel row but the lit one, 102 and 103 the group
//! labels (folded into 105), from `PANEL_ROWS` sixteen per panel row, 200 the keys' words and 201 the lit key's wash, 500 and 501 the
//! empty or no-results line and the Accounts button, from 1000 twenty per cover, from 8100 sixteen per list row.
use crate::game::model::{
    Focus, Layer,
    facts::Start,
    library::Row,
};
use crate::game::screens::paint::{Body, Fill, H, Paint, ScrollGroup, W};
use crate::game::screens::parts::{
    Side,
    dialog::scrolling,
    drawer,
    focus::{self, Kind},
    hints,
    keyboard,
    row::{self, Leading, Line},
    topbar::{self, Here, refresh_note},
};
use compositor_protocol::{Edges, Rect};
use lumagame_core::{catalog::ArtKind, ids::GameId};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

/// The panel's width at 100 % text; it grows with the text.
const PANEL_W: f32 = 420.0;
/// Room between the panel and the grid.
const PANEL_GAP: f32 = 48.0;
/// The panel's inner padding.
const PAD: f32 = 14.0;
/// Room between the top bar and the page.
const BELOW_BAR: f32 = 32.0;
/// Room between two cover rows, beside the focused cover's caption.
const ROW_GAP: f32 = 40.0;
/// The first panel row's id, clear of the covers' ids.
const PANEL_ROWS: u64 = 1_000_000;
/// The scroll group of the grid.
const GRID_GROUP: u32 = 1;

pub fn library(p: &mut Paint<'_>, search: bool) {
    let attention = topbar::needs_attention(p);
    let nav_end = topbar::bar(p, if search { Here::Search } else { Here::Library }, attention);
    refresh_note(p, 40, nav_end, p.y);
    let top = p.y + p.tokens.target.max(p.line(Type::Label) + 12.0) + 12.0 + BELOW_BAR;
    let panel_w = PANEL_W * p.scale.max(1.0);
    let panel = Frame::new(p.x, top, panel_w, H - top - p.y - 48.0 * p.scale);
    if search {
        keys(p, panel);
    } else {
        filters(p, panel);
    }
    grid(p, Frame::new(panel.x + panel.w + PANEL_GAP, top, W - p.x - (panel.x + panel.w + PANEL_GAP), H - top), search);
    let m = p.model;
    if m.view.layers.is_empty() {
        hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
    }
}

/// The panel: Search, then the Show views with their counts, then the Refine rows with their choice. It scrolls with focus when its rows
/// outgrow it (large text, many collections).
fn filters(p: &mut Paint<'_>, panel: Frame) {
    let m = p.model;
    let rows = m.panel();
    let h = (64.0 * p.scale).max(p.line(Type::Body) + 24.0);
    let label_h = 22.0 + p.line(Type::Caption) + 8.0;
    // Each row's top, inside the panel's content.
    let mut tops = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for (i, row) in rows.iter().enumerate() {
        if matches!((i.checked_sub(1).map(|k| rows[k]), row), (Some(Row::Search), Row::Show(_)) | (Some(Row::Show(_)), Row::Plays)) {
            y += label_h;
        }
        tops.push(y);
        y += h;
    }
    let content_h = y;
    let room = panel.h - 2.0 * PAD;
    let focused = if let Focus::Filter(i) = m.view.focus { Some(i) } else { None };
    let shift = focused.map_or(0.0, |i| (tops[i] + h - room).max(0.0)).min((content_h - room).max(0.0));
    let height = (content_h + 2.0 * PAD).min(panel.h);
    p.surface(100, Frame::new(panel.x, panel.y, panel.w, height), Fill::Glass, Radius::Sheet, false, false, false, 10);
    let inner = Frame::new(panel.x + PAD, panel.y + PAD, panel.w - 2.0 * PAD, room);
    let visible = |y: f32, h: f32| y >= inner.y - 0.5 && y + h <= inner.y + inner.h + 0.5;
    let start = p.plan.nodes.len();
    for (i, row) in rows.iter().enumerate() {
        let y = inner.y + tops[i] - shift;
        let label = match (i.checked_sub(1).map(|k| rows[k]), row) {
            (Some(Row::Search), Row::Show(_)) => Some((102, "Show")),
            (Some(Row::Show(_)), Row::Plays) => Some((103, "Refine")),
            _ => None,
        };
        if let Some((id, name)) = label.filter(|_| visible(y - label_h + 22.0, p.line(Type::Caption))) {
            p.text(id, name, Frame::new(inner.x + 28.0, y - 8.0 - p.line(Type::Caption), inner.w - 56.0, p.line(Type::Caption)), Type::Caption, Role::Ink2, false, 12);
        }
        if !visible(y, h) {
            continue;
        }
        let f = Frame::new(inner.x, y, inner.w, h);
        let id = PANEL_ROWS + 16 * i as u64;
        let on = p.focused(Focus::Filter(i));
        match *row {
            Row::Search => row::link(p, id, f, &Line { lead: Leading::Glyph(Icon::Search), name: "Search", sub: None }, on),
            Row::Show(show) => {
                let count = m.library_counts.0.get(&show).copied().unwrap_or(0).to_string();
                let current = m.view.filter.show == show;
                let name = show.name(m);
                row::value(p, id, f, &Line { lead: Leading::None, name: &name, sub: None }, &count, current.then_some((Icon::Check, Role::Ink)), on);
            }
            Row::Plays | Row::Genre | Row::Sort => {
                let (name, value) = match row {
                    Row::Plays => ("Plays on", m.plays_name(m.view.filter.plays.as_ref())),
                    Row::Genre => ("Genre", m.view.filter.genre.clone().unwrap_or_else(|| "Any".into())),
                    _ => ("Sort", m.view.sort.name().into()),
                };
                row::value(p, id, f, &Line { lead: Leading::None, name, sub: None }, &value, Some((Icon::ChevronRight, Role::Ink2)), on);
            }
        }
        p.hit(Focus::Filter(i), f);
    }
    // Every row's words but the lit row's share one text layer on the panel; the lit row keeps its own, as it grows with focus.
    let lit = focused.map(|i| PANEL_ROWS + 16 * i as u64 + 2);
    let frame = Frame::new(panel.x, panel.y, panel.w, height);
    let (mut words, mut marks, mut z) = (Vec::new(), Vec::new(), 12);
    for node in p.plan.nodes.drain(start..).collect::<Vec<_>>() {
        match node.body {
            Body::Text { words: w, marks: k } if Some(node.id) != lit => {
                let (dx, dy) = (node.frame.x - frame.x, node.frame.y - frame.y);
                words.extend(w.into_iter().map(|mut word| {
                    word.x += dx;
                    word.y += dy;
                    word
                }));
                marks.extend(k.into_iter().map(|mut mark| {
                    mark.x += dx;
                    mark.y += dy;
                    mark
                }));
                z = z.max(node.z);
            }
            _ => p.plan.nodes.push(node),
        }
    }
    p.words(105, frame, words, marks, z);
}

/// Search mode: the query over six-wide keys, where the panel stands.
fn keys(p: &mut Paint<'_>, panel: Frame) {
    let m = p.model;
    let key_h = (52.0 * p.scale).min(80.0);
    let field_h = p.line(Type::Body) + 24.0;
    let rows = crate::game::model::library::KEY_ROWS as f32;
    let height = 2.0 * PAD + field_h + 16.0 + rows * (key_h + 8.0) - 8.0;
    p.surface(100, Frame::new(panel.x, panel.y, panel.w, height), Fill::Glass, Radius::Sheet, false, false, false, 10);
    let x = panel.x + PAD;
    let w = panel.w - 2.0 * PAD;
    let field = Frame::new(x, panel.y + PAD, w, field_h);
    let empty = m.view.query.is_empty();
    let side = 28.0 * p.scale;
    let word = p.word(if empty { "Search games" } else { &m.view.query }, Type::Body, p.ink(if empty { Role::Ink2 } else { Role::Ink }, false), 22.0 + side + 14.0, 12.0, w - side - 58.0);
    let mark = p.mark(Icon::Search, side, 22.0, (field_h - side) / 2.0, Role::Ink2, false);
    p.surface(104, field, Fill::Tint(0.09), Radius::Control, false, false, false, 11);
    p.words(101, field, vec![word], vec![mark], 12);
    keyboard::flat(p, 200, x, field.y + field_h + 16.0, w, crate::game::model::library::KEY_COLUMNS, 12);
}

/// The covers, or the line that stands in for them.
fn grid(p: &mut Paint<'_>, area: Frame, search: bool) {
    let m = p.model;
    if m.filtered.is_empty() {
        let line = if m.games().is_empty() {
            "No games yet"
        } else if search {
            "No games match the search"
        } else {
            "No games match these filters"
        };
        p.text(500, line, Frame::new(area.x, area.y + 24.0, area.w, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
        if m.games().is_empty() {
            p.button(501, Focus::SheetAction(0), "Add accounts and PCs", Some(Icon::Plus), area.x, area.y + 48.0 + p.line(Type::Title), true, false, 10);
        }
        return;
    }
    let columns = m.library_columns();
    let w = (area.w - p.gap * (columns - 1) as f32) / columns as f32;
    let row_h = w * 1.5 + ROW_GAP + caption_h(p);
    let first = m.view.first_grid_row;
    // Where focus is: the covers near it are those whose pages are decoded ahead.
    let focus = match m.view.focus {
        Focus::Grid(i) => (i / columns, i % columns),
        _ => (first, 0),
    };
    let current = p.scroll_positions.get(&GRID_GROUP).copied().unwrap_or(first as f32 * row_h);
    let total = m.filtered.len().div_ceil(columns);
    // Only the rows on screen at the scroll's start and its end are drawn: rows past them cost layers on every frame, and a cover
    // uploads when its row scrolls in (one per frame at most).
    let view = H - area.y;
    let mut rows = Vec::with_capacity(6);
    for top in [first as f32 * row_h, current] {
        let from = (top / row_h).floor().max(0.0) as usize;
        let to = (((top + view) / row_h).ceil() as usize).min(total);
        for row in from..to {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    rows.sort_unstable();
    let start = p.plan.nodes.len();
    for row in rows {
        let y = area.y + (row as f32 - first as f32) * row_h;
        for (i, game) in m.filtered.iter().enumerate().skip(row * columns).take(columns) {
            let hits = p.plan.hits.len();
            super::game::warm_art(p, *game, near((i / columns, i % columns), focus));
            cover(p, 1000 + i as u64 * 20, *game, Focus::Grid(i), Frame::new(area.x + (i % columns) as f32 * (w + p.gap), y, w, w * 1.5));
            if p.plan.hits.len() > hits {
                p.plan.hit_groups.insert(Focus::Grid(i), GRID_GROUP);
            }
        }
    }
    let nodes = p.plan.nodes[start..].iter().map(|node| node.id).collect();
    p.plan.groups.push(ScrollGroup {
        id: GRID_GROUP,
        parent: None,
        clip: Rect { x: area.x - p.gap, y: area.y - p.gap, w: area.w + 2.0 * p.gap, h: H - area.y + p.gap },
        feather: Edges::NONE,
        horizontal: false,
        offset: first as f32 * row_h,
        extent: total as f32 * row_h + H,
        nodes,
    });
}

/// The caption's height: the name over the time played.
fn caption_h(p: &Paint<'_>) -> f32 {
    26.0 + p.line(Type::Body) + p.line(Type::Caption)
}

/// How many places a cover is from focus, at `(row, column)`: right and down come before left and up, as that is where focus goes next.
fn near((row, column): (usize, usize), (focus_row, focus_column): (usize, usize)) -> u32 {
    let (down, right) = (row as i64 - focus_row as i64, column as i64 - focus_column as i64);
    let places = 2 * (down.unsigned_abs() + right.unsigned_abs()) as u32;
    if down > 0 || right > 0 { places - 1 } else { places }
}
/// One cover. Only the focused cover has a caption; only a game that cannot start now carries a mark, naming what stops it.
fn cover(p: &mut Paint<'_>, id: u64, game: GameId, focus: Focus, f: Frame) {
    let m = p.model;
    let Some(g) = m.game(game) else { return };
    let focused = p.focused(focus);
    let from = p.plan.nodes.len();
    // The title on a plate stands in for the cover until its art is on disk; an opaque cover needs no plate, which saves a layer a
    // cover (15 on screen).
    if let Some(path) = m.art_url(game, ArtKind::Cover).and_then(|url| m.art_path(&url)) {
        p.image(id + 2, path, f, p.tokens.radius(Radius::Tile), false, None, 12);
    } else {
        p.surface(id, f, Fill::Sheet, Radius::Tile, false, false, false, 10);
        let count = ui::components::wrap(p.fonts, &g.title, p.bold(Type::Body), f.w - 48.0).len();
        let h = count as f32 * p.line(Type::Body);
        p.wrapped(id + 1, &g.title, Frame::new(f.x + 24.0, f.y + (f.h - h) / 2.0, f.w - 48.0, h), Type::Body, Role::Ink, false, 11);
    }
    if let Start::Cannot(why) = m.can_start(game) {
        let side = 24.0 * p.scale;
        let fh = 40.0f32.max(p.line(Type::Caption) + 8.0);
        let fw = p.width(why.word(), Type::Caption) + side + 34.0;
        let flag = Frame::new(f.x + 10.0, f.y + 10.0, fw.min(f.w - 20.0), fh);
        p.surface(id + 3, flag, Fill::Sheet, Radius::Pill, false, false, false, 13);
        let word = p.word(why.word(), Type::Caption, p.ink(Role::Ink, false), side + 22.0, (fh - p.line(Type::Caption)) / 2.0, flag.w - side - 34.0);
        let mark = p.mark(Icon::Alert, side, 12.0, (fh - side) / 2.0, Role::Warning, false);
        p.words(id + 4, flag, vec![word], vec![mark], 14);
    }
    focus::treat(p, from, id + 5, f, Kind::Tile, focused);
    if focused {
        let w = (f.w * 2.2).min(W - p.x - f.x);
        let y = f.y + f.h + 26.0;
        let ty = p.bold(Type::Body);
        let mut name = p.word(g.title.clone(), Type::Body, p.ink(Role::Ink, false), 0.0, 0.0, w);
        name.ty = ty;
        p.words(id + 7, Frame::new(f.x, y, w, p.line(Type::Body)), vec![name], vec![], 15);
        if let Some((seconds, _)) = m.play_time_of(game).filter(|(s, _)| *s > 0) {
            let hours = seconds / 3600;
            let played = if hours > 0 { format!("{hours} h played") } else { format!("{} min played", (seconds / 60).max(1)) };
            p.text(id + 8, &played, Frame::new(f.x, y + p.line(Type::Body), w, p.line(Type::Caption)), Type::Caption, Role::Ink2, false, 15);
        }
    }
    p.hit(focus, f);
}

/// A Refine list: Plays on, Genre or Sort, as a drawer from the left over the panel, the current choice ticked.
pub fn refine(p: &mut Paint<'_>, layer: Layer) {
    let m = p.model;
    let title = match layer {
        Layer::PlaysOn => "Plays on",
        Layer::Genre => "Genre",
        _ => "Sort",
    };
    let (names, current) = m.choices(layer);
    let slot = drawer::open(p, Side::Left, drawer::WIDTH, title);
    let h = (72.0 * p.scale).max(p.line(Type::Body) + 24.0);
    let pitch = h + 4.0;
    let visible = ((slot.h / pitch).floor() as usize).max(1);
    let first = if let Focus::SheetRow(i) = m.view.focus { (i + 1).saturating_sub(visible) } else { 0 };
    let from = p.plan.nodes.len();
    scrolling(p, Rect { x: slot.x, y: slot.y, w: slot.w, h: slot.h }, pitch, names.len(), first, 4.0, |p, i, y| {
        let f = Frame::new(slot.x, y, slot.w, h);
        let mark = (i == current).then_some((Icon::Check, Role::Ink));
        let on = p.focused(Focus::SheetRow(i));
        row::value(p, 8100 + 16 * i as u64, f, &Line { lead: Leading::None, name: &names[i], sub: None }, "", mark, on);
        p.hit(Focus::SheetRow(i), f);
    });
    drawer::lift(p, from);
    hints::bar(p, &m.hints(), Side::Left, m.hints_dim);
}
