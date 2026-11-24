//! The game page (Glance): the art, the logo, one line, Play and where to play, and four cards that each open their section as a layer
//! over the dimmed art; the reader of a post over a section; and the chooser of ways to play.
//!
//! Node ids: 10-12 the head, 20-30 the buttons, 100-179 the cards, 1000 and up each section's content (a thousand to a section, so a
//! switched section's content fades in under ids of its own), 8100 and up the chooser's ways, 8700 its Remember row, 9800 and up the
//! reader.
pub mod media;
use crate::game::model::{
    Focus, Layer, Model,
    facts::{Fact, Reason, Section, why},
    game::{Entry, SECTIONS, play_time, section_index, section_name},
};
use crate::game::screens::paint::{H, Paint, W, status_word};
use crate::game::screens::parts::{
    SHEET_Z, Side,
    card::{self, Card, Headline, Source},
    drawer, hints, layer,
    row::{self, Leading, Line},
    stage::*,
};
use lumagame_core::{engine::EngineKind, ids::GameId, ipc::Readiness};
use lumatv_tokens::TypeRole as Type;
use std::path::PathBuf;
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

/// The art fades into the canvas from this height to the bottom, so the cards and the hints sit on its darker foot.
const FADE_FROM: f32 = 520.0;
/// The page's stage art for the focused game on Home or the Library, to be decoded ahead (see [`stage::warm`]): the same picture as `page`'s.
pub fn warm_art(p: &mut Paint<'_>, game: GameId) {
    warm(p, game, H, FADE_FROM, H, false);
}
/// Room between cards, and between rows of a list.
const CARD_GAP: f32 = 24.0;
const ROW_GAP: f32 = 6.0;
/// At 100 % text the head starts here, the logo is this tall and the cards are this tall at this height.
const HEAD_TOP: f32 = 250.0;
const LOGO_H: f32 = 150.0;
const CARD_H: f32 = 276.0;
const CARDS_Y: f32 = 650.0;
/// Larger text moves the head up and lays the cards out two by two.
const BIG_HEAD_TOP: f32 = 70.0;
const BIG_LOGO_H: f32 = 100.0;
/// The head's width beside an open layer, clear of the 1080 px reader on the right.
const HEAD_W: f32 = 600.0;

pub fn page(p: &mut Paint<'_>) {
    let m = p.model;
    let Some(id) = m.view.selected.filter(|id| m.game(*id).is_some()) else {
        return;
    };
    match m.view.top() {
        Some(layer) if matches!(layer, Layer::Section(_) | Layer::Reader | Layer::Chooser) => {
            // The layers lie on the copy of the art decoded under the theme's scrim; the page draws no scrim of its own.
            over_art(p, id, H, FADE_FROM, H, false);
            head(p, id, HEAD_W);
            match layer {
                Layer::Section(section) => section_layer(p, id, section),
                Layer::Reader => reader(p),
                _ => chooser(p, id),
            }
        }
        _ => {
            art(p, id, H, FADE_FROM, H, false);
            let y = head(p, id, W - 2.0 * p.x);
            let y = actions(p, id, y);
            cards(p, id, y);
            // A layer over the page (the quick menu, a dialog) brings its own hints.
            if m.view.top().is_none() {
                hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
            }
        }
    }
}

/// The logo and the one line under it. Returns where the head ends.
fn head(p: &mut Paint<'_>, id: GameId, w: f32) -> f32 {
    let big = p.scale > 1.0;
    let (top, logo_h) = if big { (BIG_HEAD_TOP, BIG_LOGO_H) } else { (HEAD_TOP, LOGO_H) };
    logo(p, 10, id, Frame::new(p.x, top, w.min(HEAD_W), logo_h));
    let (line, role) = line_of(p.model, id);
    let y = top + logo_h + 24.0;
    let on_art = has_hero(p, id);
    let lines = p.fit(&line, Type::Label, w, 1);
    p.lines(12, lines, Frame::new(p.x, y, w, p.line(Type::Label)), Type::Label, role, on_art, 10);
    y + p.line(Type::Label)
}

/// What the line under the logo says: why the last start failed, else the page's notice, else when and how long the game was played.
fn line_of(m: &Model, id: GameId) -> (String, Role) {
    if let Some(reason) = m.last_failure(id) {
        return (reason.to_owned(), Role::Warning);
    }
    if let Some(notice) = &m.view.notice {
        return (notice.clone(), Role::Ink);
    }
    let mut parts = Vec::new();
    if let Some(since) = m.since_played(id) {
        parts.push(format!("Played {}", since.to_lowercase()));
    }
    if let Some((seconds, _)) = m.play_time_of(id) {
        parts.push(play_time(seconds));
    }
    parts.extend(m.game_genres(id).into_iter().take(2));
    (parts.join("  ·  "), Role::Ink2)
}

/// Play ("Try again" after a failed start, "Relaunch" after the far side ended the session for inactivity) and the where-to-play button. Returns where they end.
fn actions(p: &mut Paint<'_>, id: GameId, y: f32) -> f32 {
    let y = y + 28.0;
    let on_art = has_hero(p, id);
    let label = if p.model.last_failure(id).is_some() {
        "Try again"
    } else if p.model.ended_idle(id) {
        "Relaunch"
    } else {
        "Play"
    };
    let w = p.button(20, Focus::Play, label, Some(Icon::Play), p.x, y, true, on_art, 12);
    let target = target_label(p, id);
    p.button(30, Focus::Where, &target, Some(Icon::ChevronDown), p.x + w + CARD_GAP, y, false, on_art, 12);
    // The art dips under it each time a section opens or closes, and a cached backdrop would be captured again after each dip.
    p.tint_only(30, on_art);
    y + p.tokens.target.max(p.line(Type::Label) + 24.0)
}

/// What one card says, owned while the card borrows it.
struct Glance {
    headline: String,
    big: bool,
    detail: Option<String>,
    progress: Option<f32>,
    thumbs: Vec<PathBuf>,
    caption: Option<String>,
    steam: bool,
}

fn cards(p: &mut Paint<'_>, id: GameId, below: f32) {
    let big = p.scale > 1.0;
    let per_row = if big { 2 } else { SECTIONS.len() };
    let w = (W - 2.0 * p.x - (per_row - 1) as f32 * CARD_GAP) / per_row as f32;
    let (y0, h) = if big {
        let y0 = below + 36.0;
        (y0, ((H - p.y - 70.0 - y0 - CARD_GAP) / 2.0).max(160.0))
    } else {
        (CARDS_Y.max(below + 36.0), CARD_H)
    };
    for (i, &section) in SECTIONS.iter().enumerate() {
        let (col, row) = (i % per_row, i / per_row);
        let frame = Frame::new(p.x + col as f32 * (w + CARD_GAP), y0 + row as f32 * (h + CARD_GAP), w, h);
        let glance = glance_of(p.model, id, section);
        let card = Card {
            id: 100 + i as u64 * 20,
            label: section_name(section),
            headline: if glance.big { Headline::Big(&glance.headline) } else { Headline::Text(&glance.headline) },
            detail: glance.detail.as_deref(),
            progress: glance.progress,
            thumbs: &glance.thumbs,
            caption: glance.caption.as_deref(),
            source: glance.steam.then_some(Source { name: "Steam", stale: None }),
        };
        let focused = p.focused(Focus::Card(i));
        card::glance(p, &card, frame, focused);
        p.hit(Focus::Card(i), frame);
    }
}

/// A fact in a few words: its value, or why there is none.
fn said<T>(fact: &Fact<T>, present: impl FnOnce(&T) -> String) -> String {
    match fact {
        Fact::Present(value) => present(value),
        Fact::Loading => "Loading".into(),
        Fact::Absent(reason) => why(*reason).into(),
        Fact::SignIn { expired: false } => "Sign in to Steam".into(),
        Fact::SignIn { expired: true } => "Steam sign-in has expired".into(),
    }
}

/// [`said`], except that "Nothing here yet" is left out while the rest of the card has something to show.
fn said_unless_empty<T>(fact: &Fact<T>, has_content: bool, present: impl FnOnce(&T) -> String) -> Option<String> {
    if has_content && matches!(fact, Fact::Absent(Reason::Empty)) { None } else { Some(said(fact, present)) }
}

fn glance_of(m: &Model, id: GameId, section: Section) -> Glance {
    let facts = m.facts(id);
    let pictures = |urls: Vec<&str>| urls.into_iter().filter_map(|url| m.art_path(url)).collect::<Vec<_>>();
    match section {
        Section::Activity => {
            let achievements = &facts.activity.achievements;
            let (progress, thumbs) = match achievements {
                Fact::Present(a) => (a.total.map(|t| a.unlocked as f32 / t.max(1) as f32), pictures(a.latest.iter().take(3).filter_map(|a| a.icon).collect())),
                _ => (None, Vec::new()),
            };
            // The newest post, where Home's strip used to show it.
            let caption = match facts.activity.news {
                Fact::Present(news) => news.first().map(|n| n.title.clone()),
                _ => None,
            };
            // "Nothing here yet" under a playtime or a post says the card is empty when it is not.
            let has_content = matches!(facts.activity.play_time, Fact::Present(_)) || caption.is_some();
            let detail = said_unless_empty(achievements, has_content, |a| match a.total {
                Some(total) => format!("{} of {total} achievements", a.unlocked),
                None => format!("{} achievements", a.unlocked),
            });
            Glance {
                big: matches!(facts.activity.play_time, Fact::Present(_)),
                headline: said(&facts.activity.play_time, |t| play_time(t.seconds)),
                detail,
                progress,
                thumbs,
                caption,
                steam: matches!(achievements, Fact::Present(_)),
            }
        }
        Section::Community => {
            let reviews = &facts.community.reviews;
            Glance {
                big: false,
                headline: said(reviews, |r| r.label.clone().or(r.percent.map(|p| format!("{p} % positive"))).unwrap_or_else(|| "Reviews".into())),
                detail: match reviews {
                    Fact::Present(r) => Some(format!("{} reviews", thousands(u64::from(r.positive + r.negative)))),
                    _ => None,
                },
                progress: None,
                thumbs: Vec::new(),
                caption: None,
                steam: matches!(reviews, Fact::Present(_)),
            }
        }
        Section::Media => {
            let shots = &facts.media.store_screenshots;
            let trailers = if let Fact::Present(t) = facts.media.trailers { t.len() } else { 0 };
            let thumbs = match shots {
                Fact::Present(s) => pictures(s.iter().take(4).map(|s| s.thumb.as_str()).collect()),
                _ => Vec::new(),
            };
            Glance {
                big: false,
                headline: match shots {
                    Fact::Present(s) => format!("{} screenshots, {trailers} trailers", s.len()),
                    fact => said(fact, |_| String::new()),
                },
                detail: None,
                progress: None,
                thumbs,
                caption: None,
                steam: matches!(shots, Fact::Present(_)),
            }
        }
        Section::About => {
            let about = &facts.about;
            Glance {
                big: false,
                headline: if let Fact::Present(d) = about.developer { d.to_owned() } else { said(&about.description, |d| (*d).to_owned()) },
                detail: if let Fact::Present(r) = &about.release { Some(r.to_string()) } else { None },
                progress: None,
                thumbs: Vec::new(),
                caption: None,
                steam: false,
            }
        }
    }
}

/// A line of a section: a fact that only reads, or a row that OK opens.
enum Item {
    Fact { name: String, value: String },
    Row { focus: Focus, name: String, sub: Option<String>, picture: Option<PathBuf>, glyph: Option<Icon> },
}

fn row_height(p: &Paint<'_>) -> f32 {
    (p.line(Type::Body) + p.line(Type::Caption) + 28.0).max(p.tokens.target)
}

fn section_layer(p: &mut Paint<'_>, id: GameId, section: Section) {
    let m = p.model;
    let names = SECTIONS.map(section_name);
    let slot = layer::section(p, &names, section_index(section));
    if section == Section::Media {
        media::section(p, id, slot);
        hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
        return;
    }
    let items = section_items(m, id, section);
    let base = 1000 + section_index(section) as u64 * 1000;
    let (rh, fh) = (row_height(p), p.line(Type::Body));
    // Where each item starts; the list moves up just far enough to keep the focused row inside the slot.
    let mut tops = Vec::with_capacity(items.len());
    let mut y = 0.0;
    for (k, item) in items.iter().enumerate() {
        let fact = matches!(item, Item::Fact { .. });
        if k > 0 && fact != matches!(items[k - 1], Item::Fact { .. }) {
            y += ROW_GAP * 3.0;
        }
        tops.push(y);
        y += if fact { fh } else { rh + ROW_GAP };
    }
    let at = items.iter().position(|item| matches!(item, Item::Row { focus, .. } if *focus == m.view.focus));
    let shift = at.map_or(0.0, |k| (tops[k] + rh - slot.h).max(0.0));
    let (x, names_w) = (slot.x + 28.0, (slot.w - 56.0) * 0.4);
    let values_w = slot.w - 56.0 - names_w;
    // Each run of facts is two nodes, its names and its values, rather than a row each, to keep the layer count of a section down.
    let mut run: Option<(usize, f32, Vec<String>, Vec<String>)> = None;
    for (k, item) in items.iter().enumerate() {
        let top = slot.y + tops[k] - shift;
        let inside = top >= slot.y - 1.0 && top + if matches!(item, Item::Fact { .. }) { fh } else { rh } <= slot.y + slot.h + 1.0;
        if let Item::Fact { name, value } = item {
            if inside {
                let value = p.fit(value, Type::Body, values_w, 1).pop().unwrap_or_default();
                let name = p.fit(name, Type::Body, names_w - 24.0, 1).pop().unwrap_or_default();
                let run = run.get_or_insert_with(|| (k, top, Vec::new(), Vec::new()));
                run.2.push(name);
                run.3.push(value);
            }
            continue;
        }
        if let Some(run) = run.take() {
            facts_block(p, base, (x, names_w, values_w), run);
        }
        let Item::Row { focus, name, sub, picture, glyph } = item else { continue };
        if !inside {
            continue;
        }
        let lead = match (picture, glyph) {
            (Some(path), _) => Leading::Picture(path),
            (None, Some(icon)) => Leading::Glyph(*icon),
            _ => Leading::None,
        };
        let frame = Frame::new(slot.x, top, slot.w, rh);
        let focused = p.focused(*focus);
        row::link(p, base + k as u64 * 20, frame, &Line { lead, name, sub: sub.as_deref() }, focused);
        p.hit(*focus, frame);
    }
    if let Some(run) = run.take() {
        facts_block(p, base, (x, names_w, values_w), run);
    }
    hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
}

/// A run of facts from item `first` at `y`: the names in one node, the values beside them in another.
fn facts_block(p: &mut Paint<'_>, base: u64, (x, names_w, values_w): (f32, f32, f32), (first, y, names, values): (usize, f32, Vec<String>, Vec<String>)) {
    let id = base + first as u64 * 20;
    p.lines(id, names, Frame::new(x, y, names_w, 0.0), Type::Body, Role::Ink2, false, SHEET_Z + 1);
    p.lines(id + 1, values, Frame::new(x + names_w, y, values_w, 0.0), Type::Body, Role::Ink, false, SHEET_Z + 1);
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

/// A list's place when it has nothing: one fact with why.
fn missing<T>(items: &mut Vec<Item>, name: &str, fact: &Fact<T>) {
    if !matches!(fact, Fact::Present(_)) {
        items.push(Item::Fact { name: name.into(), value: said(fact, |_| String::new()) });
    }
}

fn section_items(m: &Model, id: GameId, section: Section) -> Vec<Item> {
    let facts = m.facts(id);
    let entries = m.section_entries(id, section);
    let mut items = Vec::new();
    let fact = |items: &mut Vec<Item>, name: &str, value: String| items.push(Item::Fact { name: name.into(), value });
    let row = |k: usize, name: String, sub: Option<String>| Item::Row { focus: Focus::Entry(k), name, sub, picture: None, glyph: None };
    match section {
        Section::Activity => {
            let a = &facts.activity;
            for (k, entry) in entries.iter().enumerate() {
                if let Entry::SignIn { expired, .. } = entry {
                    let (name, sub) = if *expired { ("Steam sign-in has expired", "Sign in again") } else { ("Sign in to Steam", "Adds play time and achievements here") };
                    items.push(Item::Row { focus: Focus::Entry(k), name: name.into(), sub: Some(sub.into()), picture: None, glyph: Some(Icon::Person) });
                }
            }
            fact(&mut items, "Play time", said(&a.play_time, |t| match t.recent {
                Some(recent) if recent > 0 => format!("{}, {} lately", play_time(t.seconds), play_time(recent)),
                _ => play_time(t.seconds),
            }));
            fact(&mut items, "Last session", said(&a.last_session, |s| {
                let when = m.ago(s.at);
                let place = m.place_name(&s.place);
                match s.seconds {
                    Some(seconds) => format!("{when}, {} on {place}", play_time(u64::from(seconds))),
                    None => format!("{when} on {place}"),
                }
            }));
            if !matches!(a.achievements, Fact::SignIn { .. }) {
                fact(&mut items, "Achievements", said(&a.achievements, |a| match a.total {
                    Some(total) => format!("{} of {total}", a.unlocked),
                    None => a.unlocked.to_string(),
                }));
            }
            if let Fact::Present(achievements) = &a.achievements {
                for latest in achievements.latest.iter().take(3) {
                    let rarity = latest.rarity.map_or_else(String::new, |r| format!("{:.1} % of players", f32::from(r) / 10.0));
                    fact(&mut items, latest.name, rarity);
                }
            }
            fact(&mut items, "Players now", said(&a.players, |n| thousands(u64::from(*n))));
            missing(&mut items, "News", &a.news);
            if let Fact::Present(news) = a.news {
                for (k, entry) in entries.iter().enumerate() {
                    if let Entry::News(i) = entry {
                        let n = &news[*i];
                        items.push(row(k, n.title.clone(), Some(m.ago(n.at))));
                    }
                }
            }
        }
        Section::Community => {
            let c = &facts.community;
            fact(&mut items, "Reviews", said(&c.reviews, |r| {
                let label = r.label.clone().or(r.percent.map(|p| format!("{p} % positive"))).unwrap_or_default();
                format!("{label}, {} reviews", thousands(u64::from(r.positive + r.negative)))
            }));
            missing(&mut items, "Discussions", &c.discussions);
            missing(&mut items, "Guides", &c.guides);
            for (k, entry) in entries.iter().enumerate() {
                match (entry, c.reviews, c.discussions, c.guides) {
                    (Entry::Review(i), Fact::Present(r), _, _) => {
                        let r = &r.top[*i];
                        let verdict = if r.recommended { "Recommended" } else { "Not recommended" };
                        items.push(row(k, r.text.lines().next().unwrap_or_default().to_owned(), Some(format!("{verdict}, {} on record", play_time(r.playtime_s)))));
                    }
                    (Entry::Thread(i), _, Fact::Present(topics), _) => {
                        let t = &topics[*i];
                        items.push(row(k, t.title.clone(), Some(format!("{} replies, {}", t.replies, t.author))));
                    }
                    (Entry::Guide(i), _, _, Fact::Present(guides)) => {
                        let g = &guides[*i];
                        items.push(row(k, g.title.clone(), Some(g.author.clone())));
                    }
                    _ => {}
                }
            }
        }
        // Its own grid: `media::section`.
        Section::Media => {}
        Section::About => {
            let a = &facts.about;
            if let (Some(k), Fact::Present(description)) = (entries.iter().position(|e| *e == Entry::About), a.description) {
                items.push(row(k, description.lines().next().unwrap_or_default().to_owned(), Some("Read more".into())));
            } else {
                missing(&mut items, "Description", &a.description);
            }
            fact(&mut items, "Developer", said(&a.developer, |d| (*d).to_owned()));
            fact(&mut items, "Release", said(&a.release, |r| r.to_string()));
            fact(&mut items, "Controller", said(&a.controller, |c| (*c).to_owned()));
            fact(&mut items, "Genres", said(&a.genres, |g| g.join(", ")));
        }
    }
    items
}

/// The reader over the section it was opened from, in two text layers: the crumb, the title and the byline; then one page of the body
/// and where it is.
fn reader(p: &mut Paint<'_>) {
    let m = p.model;
    let under = m.view.layers.iter().rev().find_map(|l| if let Layer::Section(s) = l { Some(*s) } else { None }).unwrap_or(Section::Activity);
    let names = SECTIONS.map(section_name);
    let slot = layer::section(p, &names, section_index(under));
    if let Some(text) = m.reader_text() {
        let page = if let Focus::ReaderPage(i) = m.view.focus { i.min(text.pages.len() - 1) } else { 0 };
        let (x, w) = (slot.x + 28.0, slot.w - 56.0);
        let (ink, quiet) = (p.ink(Role::Ink, false), p.ink(Role::Ink2, false));
        let mut head = vec![p.word(section_name(under), Type::Caption, quiet, 0.0, 0.0, w)];
        let mut y = p.line(Type::Caption) + 12.0;
        for line in p.fit(&text.title, Type::Title, w, 2) {
            head.push(p.word(line, Type::Title, ink, 0.0, y, w));
            y += p.line(Type::Title);
        }
        y += 8.0;
        if !text.byline.is_empty() {
            head.push(p.word(text.byline.clone(), Type::Caption, quiet, 0.0, y, w));
            y += p.line(Type::Caption);
        }
        p.words(9800, Frame::new(x, slot.y, w, y), head, vec![], SHEET_Z + 1);
        let top = slot.y + y + 20.0;
        let foot = slot.y + slot.h - p.line(Type::Caption);
        let mut body: Vec<_> = text.pages[page].iter().enumerate().map(|(i, line)| p.word(line.clone(), Type::Body, ink, 0.0, i as f32 * p.line(Type::Body), w)).collect();
        body.push(p.word(format!("Page {} of {}", page + 1, text.pages.len()), Type::Caption, quiet, 0.0, foot - top, w));
        // Each page has its own id, so the next page fades in; content goes at once, so paging never holds two pages.
        p.words(9810 + page as u64, Frame::new(x, top, w, foot + p.line(Type::Caption) - top), body, vec![], SHEET_Z + 1);
        p.hit(Focus::ReaderPage(page), slot);
    }
    hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
}

/// What a way to play says: Ready, Asleep, Offline, Sign in, or Pair for a PC that has to be paired first.
fn way_word(m: &Model, key: &lumagame_core::ids::SourceKey) -> (&'static str, Role) {
    let host = m.integration(&key.engine).is_some_and(|e| e.manifest.kind == EngineKind::Host);
    match m.readiness(key) {
        Readiness::SignIn if host => ("Pair", Role::Warning),
        _ => status_word(m, key),
    }
}

/// The ways to play in a drawer on the right, and Remember under them.
fn chooser(p: &mut Paint<'_>, id: GameId) {
    let m = p.model;
    // Large text gets a wider drawer and one-line rows, so the ways, the switch and the hints still fit under the title.
    let large = p.scale > 1.0;
    let slot = drawer::open(p, Side::Right, if large { drawer::WIDTH * 1.4 } else { drawer::WIDTH }, "Where to play");
    let rh = if large { (p.line(Type::Body) + 28.0).max(p.tokens.target) } else { row_height(p) };
    let mut y = slot.y;
    for (i, source) in m.game(id).map_or(&[][..], |g| g.sources.as_slice()).iter().enumerate() {
        let key = source.key();
        let (word, role) = way_word(m, &key);
        let glyph = match m.integration(&key.engine).map(|e| e.manifest.kind) {
            Some(EngineKind::Cloud) => Icon::CloudOutline,
            Some(EngineKind::Host) => Icon::Cards,
            Some(EngineKind::Account) => Icon::Person,
            _ => Icon::Package,
        };
        let target = if large { None } else { m.target(&key).map(|t| t.name.as_str()) };
        let frame = Frame::new(slot.x, y, slot.w, rh);
        let focused = p.focused(Focus::SheetRow(i));
        row::value(p, 8100 + i as u64 * 20, frame, &Line { lead: Leading::Glyph(glyph), name: m.engine_name(&key.engine), sub: target }, word, Some((Icon::Circle, role)), focused);
        p.hit(Focus::SheetRow(i), frame);
        y += rh + ROW_GAP;
    }
    let frame = Frame::new(slot.x, y + ROW_GAP, slot.w, (p.line(Type::Body) + 28.0).max(p.tokens.target));
    let focused = p.focused(Focus::Remember);
    row::switch(p, 8700, frame, &Line { lead: Leading::None, name: "Remember for this game", sub: None }, m.view.remember, focused);
    p.hit(Focus::Remember, frame);
    hints::bar(p, &m.hints(), Side::Left, m.hints_dim);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::{Page, facts::Reason, tests::*};
    use crate::game::screens::{paint::Body, plan_in};
    use lumagame_core::{catalog::Listing, engine::Feature, state::State};

    fn words(m: &Model) -> Vec<String> {
        plan_in(m, ui::lucent::palette::Mode::Dark)
            .nodes
            .iter()
            .flat_map(|n| if let Body::Text { words, .. } = &n.body { words.iter().map(|w| w.text.clone()).collect() } else { Vec::new() })
            .collect()
    }

    #[test]
    fn the_chooser_keeps_its_title_and_switch_clear_of_the_hints_at_any_text_size() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame]);
        engine_says(&mut m, "gfn", lumagame_core::ipc::Event::Targets { targets: vec![linked("main")] });
        let entries = vec![entry("1", "Quake", lumagame_core::ipc::Role::Game, &[], Default::default())];
        engine_says(&mut m, "gfn", lumagame_core::ipc::Event::Library { target: lumagame_core::ids::TargetId::new("main").unwrap(), entries, next: None });
        m.open_game(id_of(&m, "Quake"), None);
        m.view.push(Layer::Chooser, m.view.focus);
        for percent in [100, 200] {
            m.prefs.text_percent = percent;
            let plan = plan_in(&m, ui::lucent::palette::Mode::Dark);
            assert!(words(&m).iter().any(|w| w == "Where to play"), "{percent} %");
            let switch = plan.hits.iter().find(|(f, _)| *f == Focus::Remember).unwrap().1;
            let hints = plan.nodes.iter().filter(|n| (hints::ID..hints::ID + 100).contains(&n.id)).map(|n| n.frame.y).fold(f32::MAX, f32::min);
            assert!(switch.y + switch.h <= hints, "{percent} %: the switch stays above the hints");
        }
    }

    #[test]
    fn nothing_here_yet_is_left_out_of_a_card_that_has_something_to_show() {
        let say = |fact: &Fact<u32>, has_content| said_unless_empty(fact, has_content, |n| n.to_string());
        assert_eq!(say(&Fact::Absent(Reason::Empty), true), None);
        assert_eq!(say(&Fact::Absent(Reason::Empty), false).as_deref(), Some("Nothing here yet"));
        assert_eq!(say(&Fact::Absent(Reason::Off), true).as_deref(), Some("Off in Settings"));
        assert_eq!(say(&Fact::Present(3), true).as_deref(), Some("3"));
    }

    #[test]
    fn a_fact_that_is_absent_says_why_in_its_place() {
        // Quake has no Steam id: every Steam fact is "Not on Steam". Hades II has one, and with the community off its facts are "Off in
        // Settings".
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame]);
        engine_says(&mut m, "gfn", lumagame_core::ipc::Event::Targets { targets: vec![linked("main")] });
        let entries = vec![
            entry("1", "Hades II", lumagame_core::ipc::Role::Game, &["steam:1145350"], Default::default()),
            entry("2", "Quake", lumagame_core::ipc::Role::Game, &[], Default::default()),
        ];
        engine_says(&mut m, "gfn", lumagame_core::ipc::Event::Library { target: lumagame_core::ids::TargetId::new("main").unwrap(), entries, next: None });
        for (title, reason) in [("Quake", Reason::NoSteam), ("Hades II", Reason::Off)] {
            m.open_game(id_of(&m, title), Some(Section::Community));
            m.rebuild_navigation();
            assert_eq!(m.view.page, Page::Game);
            let shown = words(&m);
            assert!(shown.iter().any(|w| w == why(reason)), "{title}: {shown:?}");
        }
    }
}
