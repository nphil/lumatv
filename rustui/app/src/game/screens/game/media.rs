//! A game's pictures and videos: the Media section's filter and grid of stills, and the full-screen viewer. A trailer or video shows
//! its still with a play mark; the video itself plays on the shell's own layer, never as a texture here.
//!
//! Node ids: the filter 3000-3009; a still 5 ids from `TILES` + 10 000 a group (its focus treatment takes the last two); the viewer's
//! chrome 9600 and up.
use crate::game::model::{Focus, Model, game::media::{Group, Item}};
use crate::game::screens::art::Stage;
use crate::game::screens::paint::{Body, Fill, H, Mark, Paint, W};
use crate::game::screens::parts::{SHEET_Z, Side, focus, hints, stage};
use compositor_protocol::{Rgba, TintGradient};
use lumagame_core::ids::GameId;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Form, Icon}, geometry::Frame, lucent::palette::PaletteRole as Role};

const FILTER: u64 = 3000;
const TILES: u64 = 20_000;
const VIEWER: u64 = 9600;
/// The grid's inset in the section and the space between stills (`.gp-grid`).
const INSET: f32 = 28.0;
const GAP: f32 = 24.0;
/// The filter's padding, a segment's own padding and its least height (`.seg`).
const SEG_PAD: f32 = 6.0;
const SEG_SIDE: f32 = 24.0;
const SEG_H: f32 = 52.0;
/// The play mark: a disc in a still's lower left (`.gp-playmark`).
const DISC: f32 = 48.0;
const DISC_IN: f32 = 14.0;
const PLAY: f32 = 22.0;
/// The viewer's scrim under its caption.
const SCRIM_H: f32 = 300.0;

/// The Media section's body in `slot`: the filter over the stills of its group, only the rows on screen; or one line of why when Steam
/// has nothing to show.
pub fn section(p: &mut Paint<'_>, id: GameId, slot: Frame) {
    let m = p.model;
    let z = SHEET_Z + 1;
    let Some(group) = m.media_group(id) else {
        p.text(FILTER, m.media_absent(id), Frame::new(slot.x + INSET, slot.y, slot.w - 2.0 * INSET, p.line(Type::Body)), Type::Body, Role::Ink2, false, z);
        return;
    };
    let seg_h = filter(p, id, group, Frame::new(slot.x + INSET, slot.y, slot.w - 2.0 * INSET, 0.0), z);
    let top = slot.y + seg_h + 22.0;
    let columns = m.media_columns();
    let w = (slot.w - 2.0 * INSET - GAP * (columns - 1) as f32) / columns as f32;
    let h = (w * 9.0 / 16.0).round();
    let fit = (((slot.y + slot.h - top + GAP) / (h + GAP)).floor() as usize).max(1);
    let row = if let Focus::Media(i) = m.view.focus { i / columns } else { 0 };
    let first = (row + 1).saturating_sub(fit);
    let base = TILES + 10_000 * Group::ALL.iter().position(|g| *g == group).unwrap_or(0) as u64;
    let items = m.media_items(id, group);
    for (i, item) in items.iter().enumerate().skip(first * columns).take(fit * columns) {
        let (r, c) = ((i / columns - first) as f32, (i % columns) as f32);
        let frame = Frame::new(slot.x + INSET + c * (w + GAP), top + r * (h + GAP), w, h);
        still(p, m, base + 5 * i as u64, frame, item, Focus::Media(i), z);
    }
}

/// The filter: one segment a group, the chosen one filled; "From Steam" at the right. Returns its height.
fn filter(p: &mut Paint<'_>, id: GameId, group: Group, row: Frame, z: i32) -> f32 {
    let m = p.model;
    let groups = m.media_groups(id);
    let h = SEG_H.max(p.line(Type::Label) + 16.0) + 2.0 * SEG_PAD;
    let ty = p.bold(Type::Label);
    let (fill, ink) = p.primary(false);
    let at = groups.iter().position(|g| *g == group).unwrap_or(0);
    // Large text can make the five groups wider than the row: the chosen one stays, with as many neighbours as fit beside it.
    let widths = groups.iter().map(|g| p.width(g.name(), Type::Label) + 2.0 * SEG_SIDE + SEG_PAD).collect::<Vec<_>>();
    let (mut first, mut last, mut span) = (at, at, SEG_PAD + widths[at]);
    while let Some(i) = [last + 1, first.wrapping_sub(1)].into_iter().find(|&i| widths.get(i).is_some_and(|w| span + w <= row.w)) {
        span += widths[i];
        if i > last { last = i } else { first = i }
    }
    let mut words = Vec::with_capacity(last + 1 - first);
    let mut x = SEG_PAD;
    let mut chosen = Frame::new(0.0, 0.0, 0.0, 0.0);
    for (g, w) in groups[first..=last].iter().zip(&widths[first..]) {
        let w = w - SEG_PAD;
        let colour = if *g == group {
            chosen = Frame::new(row.x + x, row.y + SEG_PAD, w, h - 2.0 * SEG_PAD);
            ink
        } else {
            p.ink(Role::Ink2, false)
        };
        let mut word = p.word(g.name(), Type::Label, colour, x + SEG_SIDE, (h - p.line(Type::Label)) / 2.0, w - 2.0 * SEG_SIDE);
        word.ty = ty;
        words.push(word);
        x += w + SEG_PAD;
    }
    let track = Frame::new(row.x, row.y, x, h);
    let from = p.plan.nodes.len();
    p.surface(FILTER, track, Fill::Tint(p.tokens.states.selected_wash), Radius::Pill, false, false, false, z);
    p.solid(FILTER + 1, chosen, fill, p.tokens.radius(Radius::Pill).min(chosen.h / 2.0), z + 1);
    p.words(FILTER + 2, track, words, vec![], z + 2);
    focus::treat(p, from, FILTER + 5, track, focus::Kind::Button, p.focused(Focus::MediaFilter(at)));
    if groups.len() > 1 {
        p.hit(Focus::MediaFilter(at), track);
    }
    // A full filter leaves no room for the source beside it at its focused size.
    let source = "From Steam";
    let sw = p.width(source, Type::Caption);
    let sh = p.line(Type::Caption);
    if x * p.tokens.focus.button_scale + SEG_SIDE <= row.w - sw {
        p.text(FILTER + 4, source, Frame::new(row.x + row.w - sw, row.y + h - sh, sw, sh), Type::Caption, Role::Ink2, false, z);
    }
    h
}

/// One still: its picture (the stage's colour until it is on disk), a play mark on a video, and the focus light.
fn still(p: &mut Paint<'_>, m: &Model, id: u64, frame: Frame, item: &Item<'_>, focus: Focus, z: i32) {
    let radius = p.tokens.radius(Radius::Tile);
    let from = p.plan.nodes.len();
    match m.art_path(item.thumb) {
        Some(path) => p.image(id, path, frame, radius, false, None, z),
        None => p.solid(id + 1, frame, p.ink(Role::Canvas, true), radius, z),
    }
    if item.video.is_some() {
        let (x, y) = (DISC_IN, frame.h - DISC_IN - DISC);
        let disc = Mark { form: Form::Disc(DISC as u32), colour: p.ink(Role::Canvas, true), x, y };
        let play = p.mark(Icon::Play, PLAY, x + (DISC - PLAY) / 2.0, y + (DISC - PLAY) / 2.0, Role::Ink, true);
        p.words(id + 2, frame, vec![], vec![disc, play], z + 1);
    }
    focus::treat(p, from, id + 3, frame, focus::Kind::Tile, p.focused(focus));
    p.hit(focus, frame);
}

/// The viewer: the picture full screen on the stage's one art layer, and over it the caption and hints, which leave after three quiet
/// seconds.
pub fn media(p: &mut Paint<'_>) {
    let m = p.model;
    let Some(id) = m.view.selected else { return };
    let viewer = &m.view.media;
    let items = m.media_items(id, viewer.group);
    let Some(item) = items.get(viewer.at) else { return };
    // The viewer is stage whatever the theme: a light canvas would flash through each step's dip.
    let canvas = p.straight(Role::Canvas, true);
    if let Some(Body::Surface(glass)) = p.plan.nodes.iter_mut().find(|n| n.id == stage::CANVAS).map(|n| &mut n.body) {
        glass.tint = canvas.premultiply(1.0);
    }
    p.art = true;
    if let Some(path) = m.art_path(item.full) {
        let scrim = p.straight(Role::Scrim, true);
        let stage = Stage {
            art_height: H as u32,
            fade_from: H as u32,
            fade_to: H as u32,
            canvas: [canvas.r, canvas.g, canvas.b],
            scrim: [scrim.r, scrim.g, scrim.b, scrim.a],
            dim: false,
            curtain: true,
        };
        p.image(stage::ART, path, Frame::new(0.0, 0.0, W, H), 0.0, false, Some(stage), 1);
    }
    p.hit(Focus::Media(viewer.at), Frame::new(0.0, 0.0, W, H));
    if !viewer.chrome.visible(m.now_ms()) {
        return;
    }
    let shade = Frame::new(0.0, H - SCRIM_H, W, SCRIM_H);
    p.solid(VIEWER, shade, Rgba(0, 0, 0, 0), 0.0, 10);
    let bottom = p.straight(Role::Scrim, true).premultiply(0.0);
    if let Some(Body::Surface(glass)) = p.plan.nodes.last_mut().map(|n| &mut n.body) {
        glass.tint_gradient = Some(TintGradient { top: Rgba(0, 0, 0, 0), bottom });
    }
    let title = if item.title.is_empty() { m.game(id).map_or("", |g| g.title.as_str()) } else { item.title };
    let mut line = format!("{} of {}, {}", viewer.at + 1, items.len(), viewer.group.one());
    if !item.author.is_empty() {
        line.push_str(&format!(" by {}", item.author));
    }
    let (head, cap) = (p.line(Type::Body), p.line(Type::Caption));
    let w = W / 2.0;
    let y = H - p.y - head - 4.0 - cap;
    let mut heading = p.word(title, Type::Body, p.ink(Role::Ink, true), 0.0, 0.0, w);
    heading.ty = p.bold(Type::Body);
    let caption = p.word(line, Type::Caption, p.ink(Role::Ink2, true), 0.0, head + 4.0, w);
    p.words(VIEWER + 1, Frame::new(p.x, y, w, head + 4.0 + cap), vec![heading, caption], vec![], 12);
    hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
}
