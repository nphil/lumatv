//! A labelled row with its control. [`row`] is the older page's row; [`link`], [`value`], [`switch`] and [`stepper`] are the rows of the
//! rework: a leading glyph or picture, a name over an optional line of detail, and one control at the right.
//!
//! Node ids of the rework's rows: `id` the glyph's tile, `id + 1` the glyph or the picture, `id + 2` the name, `id + 3` the detail,
//! `id + 4` the value (a stepper's pill), `id + 5` the mark, the chevron or a stepper's content, `id + 7` to `id + 9` a switch, and
//! `id + 10` and `id + 11` the focus treatment. `id + 12` is left to the caller (the rail's wash under its current place). A row
//! registers no hit: the page that places it knows what it opens.
use crate::game::screens::{
    paint::{Fill, Paint},
    parts::focus::{self, Kind},
};
use crate::game::{model::Focus, uplink::BITRATE_MAX_MBPS};
use lumatv_tokens::{Radius, TypeRole as Type};
use std::path::Path;
use ui::{components::{Align, Icon, ellipsise}, geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};

#[derive(Clone, Copy)]
pub enum Control<'a> {
    Value(&'a str),
    /// A value that changes in place, in tabular figures.
    Figure(&'a str),
    Segments(&'a [&'a str], usize),
    Toggle(bool),
    Slider(u32),
    None,
}
fn row_metrics(p: &mut Paint<'_>, label: &str, description: Option<&str>, width: f32, control_w: f32, control_h: f32, rows: usize) -> (bool, f32, f32, f32, f32) {
    let available = width - 52.0;
    let stack = (p.width(label, Type::Body) + control_w + 80.0 > width || rows > 1) && control_w > 0.0;
    let text_w = if stack || control_w == 0.0 { available } else { available - control_w - 32.0 };
    let label_h = ui::components::wrap(p.fonts, label, p.ty(Type::Body), text_w).len().max(1) as f32 * p.line(Type::Body);
    let desc_h = description.map_or(0.0, |s| ui::components::wrap(p.fonts, s, p.ty(Type::Caption), text_w).len() as f32 * p.line(Type::Caption));
    let height = if stack { label_h + desc_h + control_h + 40.0 } else { 80.0f32.max(label_h + desc_h + 24.0).max(control_h + 16.0) };
    (stack, text_w, label_h, desc_h, height)
}

pub fn row(p: &mut Paint<'_>, id: u64, focus: Focus, label: &str, description: Option<&str>, control: Control<'_>, mut f: Frame) -> f32 {
    // An empty value would become a text layer with no width.
    let control = if let Control::Value("") = control { Control::None } else { control };
    let available = f.w - 52.0;
    let line = p.line(Type::Body);
    let sh = p.tokens.target.max(p.line(Type::Label) + 12.0);
    let widths = if let Control::Segments(options, _) = &control {
        options.iter().map(|s| (p.width(s, Type::Label) + 44.0).min(available - 8.0)).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let slider_value = if let Control::Slider(value) = &control { format!("{value} Mb/s") } else { String::new() };
    let value_w = p.width(&slider_value, Type::Label) + 12.0;
    let figure = if let Control::Figure(value) = &control { p.figures(value, p.ty(Type::Body), p.ink(Role::Ink2, false), 0.0, 0.0) } else { Default::default() };
    let control_w = match &control {
        Control::Value(value) => p.width(value, Type::Body),
        Control::Figure(_) => figure.1,
        Control::Segments(..) => widths.iter().sum::<f32>() + 8.0,
        Control::Toggle(_) => 82.0,
        Control::Slider(_) => value_w + 284.0,
        Control::None => 0.0,
    }
    .min(available);
    let mut rows = 1;
    let mut used = 8.0;
    for width in &widths {
        if used + width > control_w && used > 8.0 {
            rows += 1;
            used = 8.0;
        }
        used += width;
    }
    let control_h = if matches!(control, Control::Segments(..)) { rows as f32 * sh + (rows - 1) as f32 * 8.0 } else { sh };
    let (stack, text_w, label_h, desc_h, height) = row_metrics(p, label, description, f.w, control_w, control_h, rows);
    f.h = height;
    let focused = p.focused(focus);
    let from = p.plan.nodes.len();
    if focused {
        p.surface(id, f, Fill::Tint(p.tokens.states.focus_wash[usize::from(p.mode == Mode::Light)]), Radius::Row, false, true, false, 10);
    }
    let text_y = if stack { 12.0 } else { (f.h - label_h - desc_h) / 2.0 };
    p.wrapped(id + 1, label, Frame::new(f.x + 26.0, f.y + text_y, text_w, 0.0), Type::Body, Role::Ink, false, 11);
    if let Some(description) = description {
        p.wrapped(id + 18, description, Frame::new(f.x + 26.0, f.y + text_y + label_h, text_w, 0.0), Type::Caption, Role::Ink2, false, 11);
    }
    let x = f.x + f.w - control_w - 26.0;
    let cy = if stack { f.y + label_h + desc_h + 24.0 } else { f.y + (f.h - control_h) / 2.0 };
    match control {
        Control::Value(value) => {
            let mut word = p.word(value, Type::Body, p.ink(Role::Ink2, false), 0.0, 0.0, control_w);
            word.align = Align::Right;
            p.words(id + 2, Frame::new(x, if stack { cy } else { f.y + (f.h - line) / 2.0 }, control_w, line), vec![word], vec![], 12);
        }
        Control::Figure(_) => {
            p.words(id + 2, Frame::new(x, if stack { cy } else { f.y + (f.h - line) / 2.0 }, control_w, line), figure.0, vec![], 12);
        }
        Control::Toggle(on) => p.toggle(id + 2, x, if stack { cy } else { f.y + (f.h - 48.0) / 2.0 }, on, false, 12),
        Control::Slider(value) => {
            let track = Frame::new(x, cy + (control_h - 6.0) / 2.0, (control_w - value_w - 24.0).max(1.0), 6.0);
            p.solid(id + 2, track, p.ink(Role::TrackOff, false), 3.0, 12);
            let fill = track.w * value as f32 / BITRATE_MAX_MBPS as f32;
            p.solid(id + 3, Frame::new(track.x, track.y, fill, 6.0), p.ink(Role::Accent, false), 3.0, 13);
            p.solid(id + 4, Frame::new(track.x + fill - 10.0, track.y - 7.0, 20.0, 20.0), p.ink(Role::Ink, false), 10.0, 14);
            p.text(
                id + 5,
                &slider_value,
                Frame::new(x + control_w - value_w, cy + (control_h - p.line(Type::Label)) / 2.0, value_w, p.line(Type::Label)),
                Type::Label,
                Role::Ink2,
                false,
                12,
            );
        }
        Control::Segments(options, selected) => {
            let mut sx = x + 4.0;
            let mut sy = cy;
            for (i, (label, width)) in options.iter().zip(widths).enumerate() {
                if sx + width > x + control_w && sx > x + 4.0 {
                    sx = x + 4.0;
                    sy += sh + 8.0;
                }
                let node = 100_000_000 + id * 256 + i as u64 * 2;
                p.surface(
                    node,
                    Frame::new(sx, sy + 4.0, width, sh - 8.0),
                    if i == selected { Fill::Accent } else { Fill::Tint(0.05) },
                    Radius::Pill,
                    false,
                    false,
                    false,
                    13,
                );
                let mut word = p.word(
                    *label,
                    Type::Label,
                    p.ink(if i == selected { Role::AccentInk } else { Role::Ink2 }, false),
                    0.0,
                    (sh - p.line(Type::Label)) / 2.0,
                    width,
                );
                word.align = Align::Centre;
                p.words(node + 1, Frame::new(sx, sy, width, sh), vec![word], vec![], 14);
                sx += width;
            }
        }
        Control::None => {}
    }
    if focused {
        p.zoom(from, f, p.tokens.focus.card_scale);
    }
    if focus != Focus::None {
        p.hit(focus, f);
    }
    f.h + 4.0
}

/// What stands at the left of a line.
#[derive(Clone, Copy)]
pub enum Leading<'a> {
    None,
    /// An icon on a tinted, edged tile.
    Glyph(Icon),
    /// A 64 px picture, such as a game's or an account's.
    Picture(&'a Path),
}

/// What a row says: its leading item, its name and, under it, a line of detail.
#[derive(Clone, Copy)]
pub struct Line<'a> {
    pub lead: Leading<'a>,
    pub name: &'a str,
    pub sub: Option<&'a str>,
}

const PAD_X: f32 = 28.0;
/// The room between the leading item and the words, and between the words and the control.
const GAP: f32 = 22.0;
/// The room between the name and its detail.
const SUB_GAP: f32 = 2.0;
const PICTURE: f32 = 64.0;
/// The room between a value and its mark.
const MARK_GAP: f32 = 12.0;
const STEPPER_H: f32 = 52.0;
const STEPPER_PAD: f32 = 14.0;
const STEPPER_GAP: f32 = 14.0;
const STEPPER_ARROW: f32 = 26.0;
const Z: i32 = 12;
/// A value or a stepper takes at most this share of the row's inner width, so the name keeps the rest.
const CONTROL_SHARE: f32 = 0.45;

/// How wide the leading item is, gap not included.
fn lead_width(p: &Paint<'_>, lead: Leading<'_>) -> f32 {
    match lead {
        Leading::None => 0.0,
        Leading::Glyph(_) => 32.0 * p.scale + 20.0,
        Leading::Picture(_) => PICTURE,
    }
}

/// `text` cut with an ellipsis to `width` in `ty`.
fn clip(p: &mut Paint<'_>, text: &str, ty: ui::Type, width: f32) -> String {
    ellipsise(p.fonts, text, ty, width.max(0.0))
}

/// The most room a control may take in a row of this frame.
fn control_room(frame: Frame) -> f32 {
    ((frame.w - 2.0 * PAD_X) * CONTROL_SHARE).floor()
}

/// The leading item and the words of `line`, with `trail_w` kept free at the right for the control. The words read in ink once the
/// row is lit: the wash lowers the contrast of the quieter colour.
pub(super) fn body(p: &mut Paint<'_>, id: u64, frame: Frame, line: &Line<'_>, trail_w: f32, focused: bool) {
    let mut x = frame.x + PAD_X;
    let lead_w = lead_width(p, line.lead);
    match line.lead {
        Leading::None => {}
        Leading::Glyph(icon) => {
            let tile = Frame::new(x, frame.y + (frame.h - lead_w) / 2.0, lead_w, lead_w);
            p.surface(id, tile, Fill::Glass, Radius::Control, false, false, false, Z);
            let side = 30.0 * p.scale;
            let mark = p.mark(icon, side, (lead_w - side) / 2.0, (lead_w - side) / 2.0, Role::Ink, false);
            p.words(id + 1, tile, vec![], vec![mark], Z + 1);
        }
        Leading::Picture(path) => {
            let radius = p.tokens.radius(Radius::Control);
            p.image(id + 1, path.to_path_buf(), Frame::new(x, frame.y + (frame.h - PICTURE) / 2.0, PICTURE, PICTURE), radius, false, None, Z);
        }
    }
    if lead_w > 0.0 {
        x += lead_w + GAP;
    }
    let room = (frame.x + frame.w - PAD_X - if trail_w > 0.0 { trail_w + GAP } else { 0.0 } - x).max(0.0);
    let name_h = p.line(Type::Body);
    let sub_h = line.sub.map_or(0.0, |_| SUB_GAP + p.line(Type::Caption));
    let y = frame.y + (frame.h - name_h - sub_h) / 2.0;
    let ty = p.bold(Type::Body);
    let name = clip(p, line.name, ty, room);
    let mut word = p.word(name, Type::Body, p.ink(Role::Ink, false), 0.0, 0.0, room);
    word.ty = ty;
    p.words(id + 2, Frame::new(x, y, room, name_h), vec![word], vec![], Z + 1);
    if let Some(sub) = line.sub {
        let ty = p.ty(Type::Caption);
        let text = clip(p, sub, ty, room);
        let word = p.word(text, Type::Caption, p.ink(if focused { Role::Ink } else { Role::Ink2 }, false), 0.0, 0.0, room);
        p.words(id + 3, Frame::new(x, y + name_h + SUB_GAP, room, p.line(Type::Caption)), vec![word], vec![], Z + 1);
    }
}

/// A mark `side` square ending at `right`, centred on the row; its colour is `role`.
fn mark_at(p: &mut Paint<'_>, id: u64, frame: Frame, right: f32, icon: Icon, role: Role) {
    let side = 28.0 * p.scale;
    let mark = p.mark(icon, side, 0.0, 0.0, role, false);
    p.words(id, Frame::new(right - side, frame.y + (frame.h - side) / 2.0, side, side), vec![], vec![mark], Z + 1);
}

/// A row that opens somewhere: a chevron at the right.
pub fn link(p: &mut Paint<'_>, id: u64, frame: Frame, line: &Line<'_>, focused: bool) {
    let from = p.plan.nodes.len();
    let side = 28.0 * p.scale;
    body(p, id, frame, line, side, focused);
    mark_at(p, id + 5, frame, frame.x + frame.w - PAD_X, Icon::ChevronRight, if focused { Role::Ink } else { Role::Ink2 });
    focus::treat(p, from, id + 10, frame, Kind::Row, focused);
}

/// A row that shows a value at the right, and after it `mark` (a status icon and its colour) when there is one.
pub fn value(p: &mut Paint<'_>, id: u64, frame: Frame, line: &Line<'_>, value: &str, mark: Option<(Icon, Role)>, focused: bool) {
    let from = p.plan.nodes.len();
    let side = 28.0 * p.scale;
    let mark_w = if mark.is_some() { side } else { 0.0 };
    let ty = p.ty(Type::Body);
    let text = clip(p, value, ty, control_room(frame) - mark_w - if mark.is_some() { MARK_GAP } else { 0.0 });
    let text_w = if text.is_empty() { 0.0 } else { p.width(&text, Type::Body) };
    let gap = if text_w > 0.0 && mark.is_some() { MARK_GAP } else { 0.0 };
    body(p, id, frame, line, text_w + gap + mark_w, focused);
    let mut right = frame.x + frame.w - PAD_X;
    if let Some((icon, role)) = mark {
        mark_at(p, id + 5, frame, right, icon, role);
        right -= side + gap;
    }
    if text_w > 0.0 {
        let word = p.word(text, Type::Body, p.ink(if focused { Role::Ink } else { Role::Ink2 }, false), 0.0, 0.0, text_w);
        p.words(id + 4, Frame::new(right - text_w, frame.y + (frame.h - p.line(Type::Body)) / 2.0, text_w, p.line(Type::Body)), vec![word], vec![], Z + 1);
    }
    focus::treat(p, from, id + 10, frame, Kind::Row, focused);
}

/// A row with an on-off switch at the right.
pub fn switch(p: &mut Paint<'_>, id: u64, frame: Frame, line: &Line<'_>, on: bool, focused: bool) {
    let from = p.plan.nodes.len();
    body(p, id, frame, line, 82.0, focused);
    p.toggle(id + 7, frame.x + frame.w - PAD_X - 82.0, frame.y + (frame.h - 48.0) / 2.0, on, false, Z + 1);
    focus::treat(p, from, id + 10, frame, Kind::Row, focused);
}

/// A row that steps through choices: a pill at the right with `value` between a left and a right chevron.
pub fn stepper(p: &mut Paint<'_>, id: u64, frame: Frame, line: &Line<'_>, value: &str, focused: bool) {
    let from = p.plan.nodes.len();
    let ty = p.bold(Type::Label);
    let chrome = 2.0 * (STEPPER_PAD + STEPPER_ARROW + STEPPER_GAP);
    let text = clip(p, value, ty, control_room(frame) - chrome);
    let text_w = p.fonts.measure(&text, ty).width.ceil();
    let pill = Frame::new(frame.x + frame.w - PAD_X - chrome - text_w, frame.y + (frame.h - STEPPER_H) / 2.0, chrome + text_w, STEPPER_H);
    body(p, id, frame, line, pill.w, focused);
    p.surface(id + 4, pill, Fill::Glass, Radius::Pill, false, false, false, Z);
    let quiet = if focused { Role::Ink } else { Role::Ink2 };
    let arrow_y = (STEPPER_H - STEPPER_ARROW) / 2.0;
    let marks = vec![
        p.mark(Icon::Back, STEPPER_ARROW, STEPPER_PAD, arrow_y, quiet, false),
        p.mark(Icon::ChevronRight, STEPPER_ARROW, pill.w - STEPPER_PAD - STEPPER_ARROW, arrow_y, quiet, false),
    ];
    let mut word = p.word(text, Type::Label, p.ink(Role::Ink, false), STEPPER_PAD + STEPPER_ARROW + STEPPER_GAP, (STEPPER_H - p.line(Type::Label)) / 2.0, text_w);
    word.ty = ty;
    p.words(id + 5, pill, vec![word], marks, Z + 1);
    focus::treat(p, from, id + 10, frame, Kind::Row, focused);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::{paint::Body, parts::painted};

    const FRAME: Frame = Frame { x: 100.0, y: 200.0, w: 900.0, h: 80.0 };

    fn line<'a>(name: &'a str, sub: Option<&'a str>) -> Line<'a> {
        Line { lead: Leading::Glyph(Icon::Palette), name, sub }
    }
    fn node(p: &Paint<'_>, id: u64) -> Option<Frame> {
        p.plan.nodes.iter().find(|n| n.id == id).map(|n| n.frame)
    }
    fn words(p: &Paint<'_>, id: u64) -> Vec<String> {
        match &p.plan.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("node {id}")).body {
            Body::Text { words, .. } => words.iter().map(|w| w.text.clone()).collect(),
            _ => panic!("node {id} is not text"),
        }
    }

    #[test]
    fn a_link_leads_with_its_tile_then_the_name_over_the_detail_and_ends_in_a_chevron_without_a_hit() {
        painted(|_| {}, |p| {
            link(p, 500, FRAME, &line("Appearance", Some("Rosé Pine")), false);
            let tile = node(p, 500).unwrap();
            assert_eq!((tile.x, tile.w, tile.h), (FRAME.x + 28.0, 32.0 * p.scale + 20.0, 32.0 * p.scale + 20.0));
            assert_eq!(tile.y + tile.h / 2.0, FRAME.y + FRAME.h / 2.0);
            let (name, sub) = (node(p, 502).unwrap(), node(p, 503).unwrap());
            assert_eq!(name.x, tile.x + tile.w + 22.0);
            assert!(sub.y >= name.y + p.line(Type::Body) + 2.0);
            let chevron = node(p, 505).unwrap();
            assert_eq!(chevron.x + chevron.w, FRAME.x + FRAME.w - 28.0);
            assert!(p.plan.hits.is_empty());
            assert!(node(p, 510).is_none(), "nothing is lit at rest");
        });
        painted(|_| {}, |p| {
            link(p, 500, FRAME, &line("Appearance", None), true);
            assert!(node(p, 510).is_some() && node(p, 511).is_some());
            assert!(node(p, 503).is_none());
        });
    }

    #[test]
    fn a_value_is_cut_to_its_share_and_the_name_to_what_is_left() {
        painted(|_| {}, |p| {
            let long = "a value far too long to fit in the share of the row that a value may take ".repeat(3);
            let name = "A setting with a name that is also far too long to fit beside the value ".repeat(3);
            value(p, 500, FRAME, &line(&name, None), &long, Some((Icon::Tick, Role::Positive)), false);
            let text = node(p, 504).unwrap();
            let mark = node(p, 505).unwrap();
            assert!(text.w <= ((FRAME.w - 56.0) * 0.45).floor());
            assert_eq!(mark.x + mark.w, FRAME.x + FRAME.w - 28.0);
            assert_eq!(text.x + text.w, mark.x - 12.0);
            assert!(words(p, 504)[0].ends_with('…'));
            let name_node = node(p, 502).unwrap();
            assert!(name_node.x + name_node.w + 22.0 <= text.x);
            assert!(words(p, 502)[0].ends_with('…'));
        });
        painted(|_| {}, |p| {
            value(p, 500, FRAME, &Line { lead: Leading::None, name: "Name", sub: None }, "", None, false);
            assert!(node(p, 504).is_none() && node(p, 505).is_none());
            assert_eq!(node(p, 502).unwrap().x, FRAME.x + 28.0);
        });
    }

    #[test]
    fn a_switch_ends_in_its_82_by_48_toggle() {
        painted(|_| {}, |p| {
            switch(p, 500, FRAME, &line("Stats", None), true, false);
            let toggle = node(p, 507).unwrap();
            assert_eq!((toggle.x + toggle.w, toggle.w, toggle.h), (FRAME.x + FRAME.w - 28.0, 82.0, 48.0));
            assert!(node(p, 509).is_some(), "on: its tick");
        });
    }

    #[test]
    fn a_stepper_is_a_52_high_pill_around_its_value_between_two_chevrons() {
        painted(|_| {}, |p| {
            stepper(p, 500, FRAME, &line("Frame rate", None), "120 fps", false);
            let pill = node(p, 504).unwrap();
            assert_eq!((pill.h, pill.x + pill.w), (52.0, FRAME.x + FRAME.w - 28.0));
            assert_eq!(words(p, 505), ["120 fps"]);
            match &p.plan.nodes.iter().find(|n| n.id == 505).unwrap().body {
                Body::Text { marks, .. } => assert_eq!(marks.len(), 2),
                _ => unreachable!(),
            }
            assert!(node(p, 502).unwrap().x + node(p, 502).unwrap().w + 22.0 <= pill.x);
        });
    }
}
