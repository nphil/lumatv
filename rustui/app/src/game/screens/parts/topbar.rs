//! The top bar: Home, Library and Search in a pill on the left, the Steam profile picture with the word Menu on the right, and under
//! them the note of a running library refresh. Settings, accounts and Refresh live in the quick menu, which the avatar opens.
//!
//! Node ids: 10 the pill, 11-13 each item's label, 14 Library's mark, 15-17 the current item's wash, 20-25 the focus treatment of each
//! item, 30-37 the avatar, 45-48 the shoulder marks.
use crate::game::{
    input::Source,
    model::Focus,
    screens::{
        paint::{Fill, Paint, W},
        parts::focus::{self, Kind},
        symbols::Symbol,
    },
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Align, Icon}, geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};

/// Where the player is, for the bar to mark: one of its own places, or somewhere it has none for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Here {
    Home,
    Library,
    Search,
    Profile,
    Elsewhere,
}

/// The avatar's disc.
fn disc_side(p: &Paint<'_>) -> f32 {
    (30.0 * p.scale + 14.0).min(60.0)
}
/// The pill that holds the avatar and the word Menu beside it: the pill's padding, the disc, a gap, the word and its end padding. The
/// word is what tells that the avatar opens the quick menu.
fn avatar_width(p: &mut Paint<'_>) -> f32 {
    12.0 + disc_side(p) + 14.0 + p.width("Menu", Type::Label) + 24.0
}
/// The room between a shoulder mark and the pill it belongs to.
const BUMP_GAP: f32 = 14.0;

/// Whether an account needs the player: the avatar then wears a dot.
pub fn needs_attention(p: &Paint<'_>) -> bool {
    p.model.attention().is_some()
}

/// The pill's look: glass over art, which is captured once with the page; over a flat canvas there is nothing to blur, so it is a tint;
/// in the light theme a sheet.
fn backdrop(p: &Paint<'_>) -> (Fill, bool) {
    match (p.mode, p.art) {
        (Mode::Light, _) => (Fill::Sheet, false),
        (_, art) => (Fill::Glass, art),
    }
}

/// A shoulder button's mark beside the bar, shown while a pad is in hand: its two letters in a small tile. Returns its width.
fn bump(p: &mut Paint<'_>, id: u64, label: &str, x: f32, y: f32, h: f32) -> f32 {
    let side = 37.0 * p.scale;
    let w = (side * 1.5).round();
    let f = Frame::new(x, y + (h - side) / 2.0, w, side);
    p.surface(id, f, Fill::Glass, Radius::Control, false, false, false, 61);
    let mut word = p.word(label, Type::Caption, p.ink(Role::Ink2, false), 0.0, (side - p.line(Type::Caption)) / 2.0, w);
    word.ty = p.bold(Type::Caption);
    word.align = Align::Centre;
    p.words(id + 1, f, vec![word], vec![], 62);
    w
}

/// Draws the bar for a player who is `here`, with a dot on the avatar when `attention`. Returns where the navigation ends on the right
/// (a shoulder mark included), for what sits after it.
pub fn bar(p: &mut Paint<'_>, here: Here, attention: bool) -> f32 {
    let labels = [("Home", Icon::Home, Here::Home), ("Library", Icon::Cards, Here::Library), ("Search", Icon::Search, Here::Search)];
    let side = 30.0 * p.scale;
    let widths: Vec<f32> = labels.iter().map(|(label, ..)| p.width(label, Type::Label) + side + 66.0).collect();
    let w = widths.iter().sum::<f32>() + 12.0;
    let h = p.tokens.target.max(p.line(Type::Label) + 12.0) + 12.0;
    let (fill, cached) = backdrop(p);
    let pad = p.model.input_source() == Source::Controller;
    let mut left = p.x;
    if pad {
        left += bump(p, 45, "LB", left, p.y, h) + BUMP_GAP;
    }
    p.surface(10, Frame::new(left, p.y, w, h), fill, Radius::Pill, false, false, cached, 60);
    let mut x = left + 6.0;
    for (i, (label, icon, place)) in labels.into_iter().enumerate() {
        let from = p.plan.nodes.len();
        let f = Frame::new(x, p.y + 6.0, widths[i], h - 12.0);
        let current = here == place;
        if current {
            p.surface(15 + i as u64, f, Fill::Tint(p.tokens.states.selected_wash), Radius::Pill, false, false, false, 61);
        }
        let ink = if current { Role::Ink } else { Role::Ink2 };
        let marks = if i == 1 { Vec::new() } else { vec![p.mark(icon, side, 24.0, (f.h - side) / 2.0, ink, false)] };
        p.words(
            11 + i as u64,
            f,
            vec![p.word(label, Type::Label, p.ink(ink, false), side + 36.0, (f.h - p.line(Type::Label)) / 2.0, f.w - side - 48.0)],
            marks,
            62,
        );
        if i == 1 {
            p.symbol(14, Symbol::Library, Frame::new(x + 24.0, f.y + (f.h - side) / 2.0, side, side), p.ink(ink, false), 62);
        }
        focus::treat(p, from, 20 + 2 * i as u64, f, Kind::Button, p.focused(Focus::Nav(i)));
        p.hit(Focus::Nav(i), f);
        x += widths[i];
    }
    let mut end = left + w;
    if pad {
        end += BUMP_GAP + bump(p, 47, "RB", end + BUMP_GAP, p.y, h);
    }
    avatar(p, here == Here::Profile, attention, fill, cached, h);
    end
}

/// The avatar: the signed-in Steam account's picture in a disc, or a person in a neutral disc when there is none (signed out, offline or
/// not fetched yet), with the word Menu beside it.
fn avatar(p: &mut Paint<'_>, current: bool, attention: bool, fill: Fill, cached: bool, h: f32) {
    let w = avatar_width(p);
    let pill = Frame::new(W - p.x - w, p.y, w, h);
    p.surface(30, pill, fill, Radius::Pill, false, false, cached, 60);
    let item = Frame::new(pill.x + 6.0, p.y + 6.0, w - 12.0, h - 12.0);
    let from = p.plan.nodes.len();
    if current {
        p.surface(33, item, Fill::Tint(p.tokens.states.selected_wash), Radius::Pill, false, false, false, 61);
    }
    let d = disc_side(p);
    let disc = Frame::new(item.x + 6.0, item.y + (item.h - d) / 2.0, d, d);
    let mut marks = Vec::new();
    match p.model.avatar_path() {
        Some(path) => p.image(31, path, disc, d / 2.0, false, None, 62),
        None => {
            p.surface(31, disc, Fill::Tint(p.tokens.states.selected_wash), Radius::Pill, false, false, false, 61);
            let side = d * 0.64;
            marks.push(p.mark(Icon::Person, side, 6.0 + (d - side) / 2.0, (item.h - side) / 2.0, Role::Ink2, false));
        }
    }
    let label = p.word("Menu", Type::Label, p.ink(Role::Ink2, false), 6.0 + d + 14.0, (item.h - p.line(Type::Label)) / 2.0, item.w - d - 20.0);
    p.words(32, item, vec![label], marks, 62);
    focus::treat(p, from, 34, item, Kind::Button, p.focused(Focus::Nav(3)));
    if attention {
        // 14 px on the disc's top-right corner, on a ring of the canvas that keeps it apart from the disc.
        let dot = Frame::new(disc.x + d - 10.0, item.y + 6.0, 14.0, 14.0);
        let ring = Frame::new(dot.x - 3.0, dot.y - 3.0, 20.0, 20.0);
        p.solid(36, ring, p.ink(Role::Canvas, false), 10.0, 70);
        p.solid(37, dot, p.ink(Role::Warning, false), 7.0, 71);
    }
    p.hit(Focus::Nav(3), item);
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
    let avatar = W - p.x - avatar_width(p) - 24.0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{model::Input, screens::{paint::Body, parts::{painted, painted_in}}};
    use compositor_protocol::Backdrop;

    fn ids(p: &Paint<'_>) -> Vec<u64> {
        p.plan.nodes.iter().map(|n| n.id).collect()
    }
    fn backdrop_of(p: &Paint<'_>, id: u64) -> Backdrop {
        match &p.plan.nodes.iter().find(|n| n.id == id).unwrap().body {
            Body::Surface(glass) => glass.backdrop,
            _ => panic!("node {id} is not a surface"),
        }
    }

    #[test]
    fn the_bar_has_its_three_places_and_the_avatar_each_a_focus_and_reports_where_it_ends() {
        painted(|_| {}, |p| {
            let end = bar(p, Here::Home, false);
            let hits: Vec<_> = p.plan.hits.iter().map(|(focus, _)| *focus).collect();
            assert_eq!(hits, [Focus::Nav(0), Focus::Nav(1), Focus::Nav(2), Focus::Nav(3)]);
            let pill = p.plan.nodes.iter().find(|n| n.id == 10).unwrap().frame;
            assert_eq!(end, pill.x + pill.w);
            let avatar = p.plan.nodes.iter().find(|n| n.id == 30).unwrap().frame;
            assert_eq!(avatar.x + avatar.w, W - p.x, "the avatar's pill ends at the safe edge");
        });
    }

    #[test]
    fn the_current_place_is_washed_the_focused_one_is_lit_and_the_avatar_has_a_dot_only_when_asked() {
        painted(|m| m.view.focus = Focus::Nav(2), |p| {
            bar(p, Here::Library, false);
            let all = ids(p);
            assert!(all.contains(&16) && !all.contains(&15) && !all.contains(&17), "only Library is current");
            assert!(all.contains(&24) && !all.contains(&25), "Search is focused: its wash, no underline");
            assert!(!all.contains(&20) && !all.contains(&36) && !all.contains(&37));
        });
        painted(|_| {}, |p| {
            bar(p, Here::Home, true);
            assert!(ids(p).contains(&37));
        });
    }

    #[test]
    fn the_pill_is_captured_glass_over_art_a_tint_over_a_flat_canvas_and_a_sheet_in_the_light_theme() {
        painted(|_| {}, |p| {
            p.art = true;
            bar(p, Here::Home, false);
            assert!(matches!(backdrop_of(p, 10), Backdrop::Cached { .. }) && matches!(backdrop_of(p, 30), Backdrop::Cached { .. }));
        });
        painted(|_| {}, |p| {
            bar(p, Here::Home, false);
            assert_eq!(backdrop_of(p, 10), Backdrop::None);
        });
        painted_in(Mode::Light, |_| {}, |p| {
            p.art = true;
            bar(p, Here::Home, false);
            assert_eq!(backdrop_of(p, 10), Backdrop::None);
        });
    }

    #[test]
    fn shoulder_marks_flank_the_bar_while_a_pad_is_in_hand() {
        painted(|_| {}, |p| {
            bar(p, Here::Home, false);
            assert!(!ids(p).contains(&45) && !ids(p).contains(&47));
        });
        painted(|m| { m.step(Input::Controllers([Some("Pad".to_owned()), None, None, None])); }, |p| {
            let end = bar(p, Here::Home, false);
            assert!(ids(p).contains(&45) && ids(p).contains(&47));
            let pill = p.plan.nodes.iter().find(|n| n.id == 10).unwrap().frame;
            let lb = p.plan.nodes.iter().find(|n| n.id == 45).unwrap().frame;
            assert_eq!(pill.x, lb.x + lb.w + BUMP_GAP);
            assert!(end > pill.x + pill.w);
        });
    }
}
