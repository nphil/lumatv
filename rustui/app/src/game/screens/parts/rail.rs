//! The Settings shell: two sheets, a rail of places beside a pane.
//!
//! Node ids of the shell: 100 the title with its back chevron, 101 the rail's sheet, 102 the pane's, 103 and 104 the crumb, 105 the
//! pane's heading, 106 and 107 the state chip, and from 110 each place's 13 ids (see [`row`](super::row): the place's wash is its
//! `id + 12`), six places at most, so that all stay under 200. The shell registers `Focus::Rail(i)` for each place.
use crate::game::model::{Focus, nav::Hints};
use crate::game::screens::{
    paint::{Fill, H, Paint, W},
    parts::row::{self, Leading, Line, Trail},
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Icon, ellipsise}, geometry::Frame, lucent::palette::PaletteRole as Role};

/// One place of the rail: its icon, its name and, as the second line, where it stands now (`4K, 120 fps`).
#[derive(Clone, Copy)]
pub struct Place<'a> {
    pub icon: Icon,
    pub name: &'a str,
    pub state: Option<&'a str>,
}

/// What the shell shows: the page's `title` at the top left, the `places` with the `current` one washed, and the pane's head, a
/// back `crumb` when the pane is a level down, its `heading` and a state `chip` at the right. The `hints` are painted by the
/// page; the shell keeps their room clear below the sheets.
#[derive(Clone, Copy)]
pub struct RailSpec<'a> {
    pub title: &'a str,
    pub places: &'a [Place<'a>],
    pub current: usize,
    pub crumb: Option<&'a str>,
    pub heading: &'a str,
    pub chip: Option<&'a str>,
    pub hints: &'a Hints,
}

const RAIL_W: f32 = 500.0;
const GAP: f32 = 24.0;
const PAD: f32 = 16.0;
const PLACES: usize = 6;
const PLACE_STRIDE: u64 = 13;
/// The sheets lie at this depth, what they hold above it.
const Z: i32 = 10;

/// Lays out the rail and the pane and returns the pane's content frame: under its head, inside its padding. Sets `art` off: the
/// sheets are opaque, there is no art to capture behind them.
pub fn shell(p: &mut Paint<'_>, spec: &RailSpec<'_>) -> Frame {
    p.art = false;
    let target = p.tokens.target;
    let side = 30.0 * p.scale;
    let ty = p.bold(Type::Title);
    let title_w = p.fonts.measure(spec.title, ty).width.ceil();
    let mut word = p.word(spec.title, Type::Title, p.ink(Role::Ink, false), side + 20.0, (target - p.line(Type::Title)) / 2.0, title_w);
    word.ty = ty;
    let back = p.mark(Icon::Back, side, 0.0, (target - side) / 2.0, Role::Ink2, false);
    p.words(100, Frame::new(p.x - 6.0, p.y, side + 20.0 + title_w, target), vec![word], vec![back], Z + 2);

    let top = p.y + target + 28.0;
    let bottom = H - (p.y + 37.0 * p.scale + 28.0);
    let rail = Frame::new(p.x, top, RAIL_W, bottom - top);
    let pane = Frame::new(p.x + RAIL_W + GAP, top, W - p.x - (p.x + RAIL_W + GAP), bottom - top);
    p.surface(101, rail, Fill::Sheet, Radius::Sheet, false, false, false, Z);
    p.surface(102, pane, Fill::Sheet, Radius::Sheet, false, false, false, Z);

    // A place's state line sits under its name, so at 200% text the pair outgrows the row token; where the places would then
    // run past the rail, they show their names alone.
    let count = spec.places.len().min(PLACES);
    let paired = p.tokens.row.max(p.line(Type::Body) + 2.0 + p.line(Type::Caption) + 24.0);
    let states = count as f32 * (paired + 4.0) - 4.0 <= rail.h - 2.0 * PAD;
    let height = if states { paired } else { p.tokens.row.max(p.line(Type::Body) + 24.0) };
    let pitch = height + 4.0;
    for (i, place) in spec.places.iter().take(PLACES).enumerate() {
        let id = 110 + i as u64 * PLACE_STRIDE;
        let frame = Frame::new(rail.x + PAD, rail.y + PAD + i as f32 * pitch, rail.w - 2.0 * PAD, height);
        let focused = p.focused(Focus::Rail(i));
        let sub = place.state.filter(|_| states);
        row::body(p, id, frame, &Line { lead: Leading::Glyph(place.icon), name: place.name, sub }, Trail::NONE, focused);
        if i == spec.current {
            let at = p.plan.nodes.len();
            p.surface(id + 12, frame, Fill::Tint(p.tokens.states.selected_wash), Radius::Row, false, false, false, Z + 1);
            if focused && !p.reduced() {
                p.raise(at, frame, p.tokens.focus.row_scale, 0.0);
            }
        }
        p.hit(Focus::Rail(i), frame);
    }

    head(p, spec, pane)
}

/// The pane's head, and the frame under it that the page fills.
fn head(p: &mut Paint<'_>, spec: &RailSpec<'_>, pane: Frame) -> Frame {
    let (x, y) = (pane.x + PAD, pane.y + PAD + 12.0);
    let heading_h = p.line(Type::Title);
    let crumb_h = if spec.crumb.is_some() { p.tokens.target } else { 0.0 };
    let chip_h = if spec.chip.is_some() { 44.0f32.max(p.line(Type::Caption) + 8.0) } else { 0.0 };
    let head_h = heading_h.max(crumb_h).max(chip_h);

    let mut left = x + 14.0;
    if let Some(crumb) = spec.crumb {
        let side = 28.0 * p.scale;
        let ty = p.bold(Type::Label);
        let text_w = p.fonts.measure(crumb, ty).width.ceil();
        let w = 24.0 + side + 12.0 + text_w + 28.0;
        let f = Frame::new(left, y + (head_h - crumb_h) / 2.0, w, crumb_h);
        p.surface(103, f, Fill::Glass, Radius::Pill, false, false, false, Z + 1);
        let mut word = p.word(crumb, Type::Label, p.ink(Role::Ink, false), 24.0 + side + 12.0, (crumb_h - p.line(Type::Label)) / 2.0, text_w);
        word.ty = ty;
        let back = p.mark(Icon::Back, side, 24.0, (crumb_h - side) / 2.0, Role::Ink, false);
        p.words(104, f, vec![word], vec![back], Z + 2);
        left += w + 20.0;
    }
    let mut right = pane.x + pane.w - PAD - 28.0;
    if let Some(chip) = spec.chip {
        let text_w = p.width(chip, Type::Caption);
        let w = 16.0 + text_w + 18.0;
        let f = Frame::new(right - w, y + (head_h - chip_h) / 2.0, w, chip_h);
        p.surface(106, f, Fill::Tint(0.09), Radius::Pill, false, false, false, Z + 1);
        let word = p.word(chip, Type::Caption, p.ink(Role::Ink, false), 16.0, (chip_h - p.line(Type::Caption)) / 2.0, text_w);
        p.words(107, f, vec![word], vec![], Z + 2);
        right -= w + 20.0;
    }
    let room = (right - left).max(0.0);
    let ty = p.bold(Type::Title);
    let heading = ellipsise(p.fonts, spec.heading, ty, room);
    let mut word = p.word(heading, Type::Title, p.ink(Role::Ink, false), 0.0, 0.0, room);
    word.ty = ty;
    p.words(105, Frame::new(left, y + (head_h - heading_h) / 2.0, room, heading_h), vec![word], vec![], Z + 2);

    let top = y + head_h + 18.0;
    Frame::new(pane.x + PAD, top, pane.w - 2.0 * PAD, (pane.y + pane.h - PAD - top).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::parts::painted;

    fn places() -> [Place<'static>; 3] {
        [
            Place { icon: Icon::Cards, name: "Accounts and PCs", state: Some("1 offline") },
            Place { icon: Icon::Palette, name: "Appearance", state: None },
            Place { icon: Icon::Info, name: "System", state: Some("Version 0.9.2") },
        ]
    }
    fn frame_of(p: &Paint<'_>, id: u64) -> Frame {
        p.plan.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("node {id}")).frame
    }

    #[test]
    fn the_shell_puts_a_500_wide_rail_beside_a_pane_and_keeps_the_hint_bar_clear() {
        let hints = Hints(vec![]);
        painted(|_| {}, |p| {
            p.art = true;
            let spec = RailSpec { title: "Settings", places: &places(), current: 1, crumb: None, heading: "Appearance", chip: None, hints: &hints };
            let content = shell(p, &spec);
            assert!(!p.art);
            let (rail, pane) = (frame_of(p, 101), frame_of(p, 102));
            assert_eq!((rail.x, rail.w), (p.x, 500.0));
            assert_eq!(pane.x, rail.x + rail.w + 24.0);
            assert_eq!(pane.x + pane.w, W - p.x);
            assert_eq!(rail.y, p.y + p.tokens.target + 28.0);
            assert_eq!(rail.y + rail.h, H - (p.y + 37.0 * p.scale + 28.0));
            assert_eq!(frame_of(p, 100).x, p.x - 6.0);
            assert!(content.x >= pane.x + 16.0 && content.y > pane.y + 16.0 && content.y + content.h <= pane.y + pane.h - 16.0);
            let hits: Vec<_> = p.plan.hits.iter().map(|(focus, _)| *focus).collect();
            assert_eq!(hits, [Focus::Rail(0), Focus::Rail(1), Focus::Rail(2)]);
            assert!(p.plan.nodes.iter().all(|n| n.id < 200));
        });
    }

    #[test]
    fn five_places_stay_inside_the_rail_at_200_percent_text() {
        let hints = Hints(vec![]);
        painted(|m| m.prefs.text_percent = 200, |p| {
            let five = [places(), places()].concat()[..5].to_vec();
            let spec = RailSpec { title: "Settings", places: &five, current: 0, crumb: None, heading: "Accounts", chip: None, hints: &hints };
            shell(p, &spec);
            let rail = frame_of(p, 101);
            assert!(p.plan.hits.iter().all(|(_, f)| f.y + f.h <= rail.y + rail.h), "a place runs past the rail");
        });
    }

    #[test]
    fn only_the_current_place_is_washed_and_the_pane_head_shows_what_it_is_given() {
        let hints = Hints(vec![]);
        painted(|_| {}, |p| {
            let spec = RailSpec { title: "Settings", places: &places(), current: 2, crumb: Some("Accounts"), heading: "Desk PC", chip: Some("Offline"), hints: &hints };
            let with = shell(p, &spec);
            let ids: Vec<u64> = p.plan.nodes.iter().map(|n| n.id).collect();
            assert!(ids.contains(&(110 + 2 * 13 + 12)) && !ids.contains(&(110 + 12)) && !ids.contains(&(110 + 13 + 12)));
            assert!(ids.contains(&104) && ids.contains(&107));
            let (crumb, heading, chip) = (frame_of(p, 103), frame_of(p, 105), frame_of(p, 106));
            assert!(heading.x >= crumb.x + crumb.w && heading.x + heading.w <= chip.x);
            assert!(with.y >= crumb.y + crumb.h, "the page starts under the head");
        });
    }
}
