//! Home: the game's logo, one line and Play over the focused game's art, and under them one row low on the screen: the focused game as a
//! wide capsule, the others as tall covers beside it; Down swaps the row for the game's cards.
//!
//! Node ids: 2 and 3 the art, 10-48 the top bar, 100 the row's group and 1000 + 8 per row slot its tiles (`+0` the art or the plate,
//! `+1` the title, `+2` the mark, `+3` the focus), 300-305 the logo, line and Play, 400 + 16 per card, 500-512 the empty Home.
use crate::game::model::{
    Focus, Layer,
    facts::{Fact, Start, Why},
    home::{HomeCard, Spot, TILES_SHOWN},
};
use crate::game::screens::{
    paint::{Fill, H, Paint, ScrollGroup, W},
    parts::{
        Side,
        card::{self, Card, Headline, Source},
        drawer,
        focus::{self, Kind},
        hints,
        stage::*,
        topbar::{self, Here},
    },
};
use crate::game::model::Key;
use lumagame_core::{catalog::ArtKind, ids::GameId};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

/// The measures: the row's top, a cover's width, the focused capsule's width, the row's height and the gap between its items; the
/// stage's foot, the logo's height and width.
const ROW_Y: f32 = 650.0;
const TILE_W: f32 = 200.0;
const FEATURED_W: f32 = 640.0;
const TILE_H: f32 = 300.0;
const TILE_GAP: f32 = 28.0;
const STAGE_FOOT: f32 = 482.0;
const LOGO_H: f32 = 150.0;
const LOGO_W: f32 = 560.0;
/// Down: the logo and Play at the top, the cards under them.
const TOP_Y: f32 = 172.0;
const TOP_LOGO_H: f32 = 96.0;
const TOP_LOGO_W: f32 = 360.0;
const CARDS_Y: f32 = 430.0;
const CARD_W: f32 = 520.0;
const CARD_H: f32 = 400.0;
const CARD_GAP: f32 = 24.0;

pub fn home(p: &mut Paint<'_>) {
    let m = p.model;
    let Some(game) = m.view.featured.filter(|_| !m.home_games.is_empty()) else {
        empty(p);
        return;
    };
    let down = matches!(m.view.focus, Focus::Home(Spot::Card(_)));
    // Down dims the art behind the cards with the stage's own darker copy; at rest the art fades out low on the left behind the words.
    if down { over_art(p, game, H, 0.0, H, false) } else { art(p, game, H, 560.0, H, false) }
    // The art changes with every tile, so the bar is the raised glass's flat colour: captured glass would capture again on each change.
    p.art = false;
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Home, attention);
    if down {
        let play_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
        logo(p, 300, game, Frame::new(p.x, TOP_Y + (play_h - TOP_LOGO_H) / 2.0, TOP_LOGO_W, TOP_LOGO_H));
        play(p, game, p.x + TOP_LOGO_W + 36.0, TOP_Y);
        cards(p, game);
    } else {
        row(p);
        let play_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
        let play_y = H - STAGE_FOOT - play_h;
        let line = m.home_line(game);
        let line_y = play_y - 28.0 - p.line(Type::Label);
        logo(p, 300, game, Frame::new(p.x, line_y - 22.0 - LOGO_H, LOGO_W, LOGO_H));
        if !line.is_empty() {
            p.text(301, &line, Frame::new(p.x, line_y, W - 2.0 * p.x, p.line(Type::Label)), Type::Label, Role::Ink, true, 10);
        }
        // Under a layer OK is the layer's, so the pill that names it steps away with the hints.
        if m.view.layers.is_empty() {
            play(p, game, p.x, play_y);
        }
    }
    // A layer over Home draws its own hints.
    if m.view.layers.is_empty() {
        hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
    }
}

/// The pill names where the game plays; OK on the tile plays it, so the pill takes no focus of its own.
fn play(p: &mut Paint<'_>, game: GameId, x: f32, y: f32) {
    let label = if p.model.play_target(game).is_some() { format!("Play  ·  {}", target_label(p, game)) } else { "Choose where to play".into() };
    let lead = p.lead(Key::Ok, Icon::Circle);
    p.button_lead(302, Focus::None, &label, Some(lead), x, y, true, true, 12);
}

/// The row: the focused tile always sits first, so the stage above it reads as its game; the row slides under it.
fn row(p: &mut Paint<'_>) {
    let m = p.model;
    // Under a layer focus is the layer's; the row stays where its featured game is.
    let at = match m.view.focus {
        Focus::Home(Spot::Tile(i)) => i,
        _ => m.home_tile().unwrap_or(0),
    };
    // The quick menu's sheet hides the tiles that reach under it, and nothing slides while it is open.
    let drawer = matches!(m.view.top(), Some(Layer::Quick | Layer::QuickSettings));
    let edge = if drawer { W - p.x - drawer::WIDTH } else { W };
    let pitch = TILE_W + TILE_GAP;
    let extra = FEATURED_W - TILE_W;
    let offset = at as f32 * pitch;
    let start = p.plan.nodes.len();
    // The tile that just slid off to the left stays for its way out; the rest are the ones on screen.
    let first = if drawer { at } else { at.saturating_sub(1) };
    for (k, &id) in m.home_games.iter().enumerate().skip(first).take(TILES_SHOWN + at - first) {
        // The featured game's capsule is wider than a cover: what sits after it moves right by the difference.
        let x = p.x + (k as f32 - at as f32) * pitch + if k > at { extra } else { 0.0 };
        let w = if k == at { FEATURED_W } else { TILE_W };
        // The cover after the last that fits shows its left part at the screen's edge, so the row reads as going on.
        if x >= edge || (drawer && x + w > edge) {
            break;
        }
        tile(p, 1000 + k as u64 * 8, id, Focus::Home(Spot::Tile(k)), x, w);
    }
    let nodes = p.plan.nodes[start..].iter().map(|node| node.id).collect();
    p.plan.groups.push(ScrollGroup {
        id: 100,
        parent: None,
        clip: compositor_protocol::Rect { x: 0.0, y: ROW_Y - 40.0, w: W, h: TILE_H + 130.0 },
        feather: compositor_protocol::Edges::NONE,
        horizontal: true,
        offset,
        extent: m.home_games.len() as f32 * pitch + extra + W,
        nodes,
    });
}

/// One image per item: the featured game's wide key art cut to the capsule, or a cover. A game without art shows its title on a plate;
/// one that cannot start now carries a mark.
fn tile(p: &mut Paint<'_>, id: u64, game: GameId, focus: Focus, x: f32, w: f32) {
    let m = p.model;
    let f = Frame::new(x, ROW_Y, w, TILE_H);
    let from = p.plan.nodes.len();
    let kinds: &[ArtKind] = if w == FEATURED_W { &[ArtKind::Hero, ArtKind::Cover] } else { &[ArtKind::Cover] };
    let art = kinds.iter().find_map(|&kind| m.art_url(game, kind).and_then(|url| m.art_path(&url)));
    match art {
        Some(path) => p.image(id, path, f, p.tokens.radius(Radius::Tile), false, None, 10),
        None => {
            p.surface(id, f, Fill::Sheet, Radius::Tile, false, false, false, 10);
            let title = m.game(game).map_or("", |g| g.title.as_str());
            p.wrapped(id + 1, title, Frame::new(x + 18.0, ROW_Y + 18.0, w - 36.0, 0.0), Type::Caption, Role::Ink, false, 11);
        }
    }
    if let Start::Cannot(why) = m.can_start(game) {
        let icon = match why {
            Why::Offline => Icon::WifiOff,
            Why::SignIn | Why::Pair => Icon::Lock,
            Why::Install => Icon::Download,
        };
        let side = 28.0 * p.scale;
        p.words(id + 2, Frame::new(x + 12.0, ROW_Y + 12.0, side, side), vec![], vec![p.mark(icon, side, 0.0, 0.0, Role::Ink, true)], 12);
    }
    focus::treat(p, from, id + 3, f, Kind::Tile, p.focused(focus));
    p.hit(focus, f);
}

/// The facts the featured game has, one card each; none is drawn for a fact that is not known.
fn cards(p: &mut Paint<'_>, game: GameId) {
    let m = p.model;
    let facts = m.facts(game);
    let list = m.home_cards();
    let focused = match m.view.focus {
        Focus::Home(Spot::Card(c)) => list.iter().position(|x| *x == c).unwrap_or(0),
        _ => 0,
    };
    let pitch = CARD_W + CARD_GAP;
    // The lit card stays on screen: the strip moves left only as far as it has to.
    let shift = (p.x + focused as f32 * pitch + CARD_W - (W - p.x)).max(0.0);
    let steam = Some(Source { name: "Steam", stale: None });
    for (i, kind) in list.iter().enumerate() {
        let frame = Frame::new(p.x + i as f32 * pitch - shift, CARDS_Y, CARD_W, CARD_H);
        let id = 400 + i as u64 * 16;
        let lit = i == focused;
        match kind {
            HomeCard::Achievements => {
                let Fact::Present(a) = &facts.activity.achievements else { continue };
                let big = match a.total {
                    Some(total) => format!("{} of {total}", a.unlocked),
                    None => a.unlocked.to_string(),
                };
                let progress = a.total.filter(|t| *t > 0).map(|t| a.unlocked as f32 / t as f32);
                let card = Card { id, label: "Achievements", headline: Headline::Big(&big), detail: None, progress, thumbs: &[], caption: None, source: steam };
                card::glance(p, &card, frame, lit);
            }
            HomeCard::LastPlayed => {
                let Fact::Present(s) = &facts.activity.last_session else { continue };
                let when = m.since_played(game).unwrap_or_default();
                let detail = s.seconds.map(|secs| format!("Played for {}", crate::game::model::game::play_time(u64::from(secs))));
                let total = match &facts.activity.play_time {
                    Fact::Present(t) => Some(format!("{} in total", crate::game::model::game::play_time(t.seconds))),
                    _ => None,
                };
                let card = Card {
                    id,
                    label: "Last played",
                    headline: Headline::Big(&when),
                    detail: detail.as_deref(),
                    progress: None,
                    thumbs: &[],
                    caption: total.as_deref(),
                    source: None,
                };
                card::glance(p, &card, frame, lit);
            }
            HomeCard::News => {
                let Fact::Present(news) = &facts.activity.news else { continue };
                let Some(item) = news.first() else { continue };
                let card =
                    Card { id, label: "News", headline: Headline::Text(&item.title), detail: None, progress: None, thumbs: &[], caption: None, source: steam };
                card::glance(p, &card, frame, lit);
            }
            HomeCard::Reviews => {
                let Fact::Present(r) = &facts.community.reviews else { continue };
                let count = r.positive + r.negative;
                let share = r.percent.map(|pc| format!("{pc}%"));
                let Some(big) = r.label.clone().or(share.clone()) else { continue };
                let detail = match &share {
                    Some(pc) if r.label.is_some() => format!("{pc} of {count} reviews"),
                    _ => format!("{count} reviews"),
                };
                let card = Card {
                    id,
                    label: "Reviews",
                    headline: Headline::Big(&big),
                    detail: Some(&detail),
                    progress: None,
                    thumbs: &[],
                    caption: None,
                    source: steam,
                };
                card::glance(p, &card, frame, lit);
            }
        }
        p.hit(Focus::Home(Spot::Card(*kind)), frame);
    }
}

/// No games yet: one line and the way to add a service or a PC.
fn empty(p: &mut Paint<'_>) {
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Home, attention);
    let y = H / 2.0 - 80.0;
    let h = p.wrapped(500, "Games appear here after a service or a PC is linked.", Frame::new(p.x, y, W - p.x * 2.0, 0.0), Type::Title, Role::Ink, false, 10);
    p.button(510, Focus::Home(Spot::Add), "Add accounts and PCs", Some(Icon::Plus), p.x, y + h + 32.0, true, false, 12);
    let m = p.model;
    if m.view.layers.is_empty() {
        hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
    }
}
