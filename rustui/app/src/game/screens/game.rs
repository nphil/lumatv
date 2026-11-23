//! The game's page, its ways to play and the chooser of a way.
pub mod media;
use crate::game::screens::paint::{Fill, H, Paint, status_word, W};
use crate::game::model::{Focus, game::HubTab, Key};
use compositor_protocol::Rect;
use lumagame_core::{catalog::SaveCompat, engine::EngineKind, ids::GameId};
use lumatv_tokens::{Radius, TypeRole as Type};
use std::path::PathBuf;
use ui::{components::Icon, geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};
use crate::game::screens::parts::{dialog::*, stage::*};

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
            let mut facts = vec![(last_label, Some(m.since_played(game.id).unwrap_or_else(|| "Not played yet".into()))), (play_label, play.map(|(seconds, _)| crate::game::model::game::play_time(seconds)))];
            facts.push(("Player reviews".to_string(), details.and_then(|d| d.hub.review.as_ref()).map(|r| r.label.clone())));
            facts.push(("Achievements".to_string(), achievement_summary(m, game.id, details)));
            facts.push(("Played in the last 2 weeks".to_string(), m.recent_play(game.id).map(crate::game::model::game::play_time)));
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
pub fn chooser(p: &mut Paint<'_>) {
    let Some(game) = p.model.selected_game() else {
        return;
    };
    let row_h = if p.scale > 1.0 { p.line(Type::Body) + p.line(Type::Caption) * 2.0 + 44.0 } else { 80.0 };
    let header = p.line(Type::Title) + 26.0;
    let footer = p.tokens.target.max(p.line(Type::Label) + 24.0) + 26.0;
    let remember_h = 80.0f32.max(p.line(Type::Body) + 24.0);
    let height = (74.0 + header + game.sources.len() as f32 * (row_h + 6.0) + remember_h + footer).min(H - 108.0);
    let f = panel(p, 1320.0, height);
    let x = f.x + 36.0;
    let w = f.w - 72.0;
    let mut y = f.y + 40.0;
    p.text(8010, &format!("Where to play {}", game.title), Frame::new(x, y, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    y += header;
    let focus = if let Focus::SheetRow(i) = p.model.view.focus {
        i
    } else {
        p.model.view.chooser_selected.as_ref().and_then(|key| game.sources.iter().position(|s| s.key() == *key)).unwrap_or(0)
    };
    let room = height - 74.0 - header - footer - remember_h;
    let visible = (room / (row_h + 6.0)).floor().max(1.0) as usize;
    let first = focus.saturating_sub(visible - 1);
    let view = Rect { x, y, w, h: visible as f32 * (row_h + 6.0) - 6.0 };
    scrolling(p, view, row_h + 6.0, game.sources.len(), first, 4.0, |p, i, y| {
        way(p, 8100 + i as u64 * 20, i, game.sources[i].key(), Frame::new(x, y, w, 0.0), true);
    });
    let remember_y = f.y + height - 34.0 - footer - remember_h;
    let rf = Frame::new(x, remember_y, w, remember_h);
    let from = p.plan.nodes.len();
    if p.focused(Focus::Remember) {
        p.surface(8700, rf, Fill::Tint(0.09), Radius::Row, false, true, false, 32);
    }
    p.text(
        8701,
        "Remember for this game",
        Frame::new(x + 26.0, remember_y + (remember_h - p.line(Type::Body)) / 2.0, w - 160.0, p.line(Type::Body)),
        Type::Body,
        Role::Ink,
        false,
        33,
    );
    p.toggle(8702, x + w - 108.0, remember_y + (remember_h - 48.0) / 2.0, p.model.view.remember, false, 33);
    if p.focused(Focus::Remember) {
        p.zoom(from, rf, p.tokens.focus.card_scale);
    }
    p.hit(Focus::Remember, rf);
    let y = f.y + height - 34.0 - p.tokens.target.max(p.line(Type::Label) + 24.0);
    let back = p.lead(Key::Back, Icon::Back);
    let choose = p.lead(Key::Ok, Icon::Circle);
    let h = p.hint(8710, "Back", back, x, y + 12.0, false, 33);
    p.hint(8714, "Choose", choose, x + h + 24.0, y + 12.0, false, 33);
    let label = "Play here";
    let button_w = p.width(label, Type::Label) + 110.0;
    p.button(8720, Focus::SheetAction(0), label, Some(Icon::Play), x + w - button_w, y, true, false, 34);
}
