//! The quick menu: a drawer from the right with the menu, or Quick settings over it.
//!
//! A row is one text layer (its glyph, name, detail, value and marks together) plus a switch where it has one, and the focus treatment
//! on the lit row: the drawer sits over a whole page, so its own layers stay few.
//!
//! Node ids: 8800-8804 the drawer, 9000 and up each row (20 apart: `+0` the words, `+7` the switch, `+10` the focus), 9200 the group
//! label, 9210 and 9211 the All settings button.
use crate::game::model::{
    Focus, Layer,
    quick::{Item, QuickSetting},
};
use crate::game::screens::{
    paint::{Fill, Mark, Paint},
    parts::{
        Side, drawer,
        focus::{self, Kind},
        hints,
    },
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Form, Icon, ellipsise}, geometry::Frame, lucent::palette::PaletteRole as Role};

/// The room between two rows.
const GAP: f32 = 4.0;
/// The room above and below the game's group label.
const GROUP_ABOVE: f32 = 22.0;
const GROUP_BELOW: f32 = 10.0;
const GROUP_H: f32 = 34.0;
/// The All settings button is 64 px tall.
const BUTTON_H: f32 = 64.0;
/// A row's padding at the sides, and the room after its glyph and before its control.
const PAD_X: f32 = 28.0;
const ROOM: f32 = 22.0;
/// The room between a stepper's value and its arrows.
const ARROW_GAP: f32 = 12.0;
const SWITCH_W: f32 = 82.0;

/// What a row ends in.
enum End<'a> {
    None,
    /// A status mark.
    Mark(Icon, Role),
    /// A stepper: the value between a left and a right arrow.
    Stepper(&'a str),
    Switch(bool),
}

/// What a row says: its glyph, and its name over an optional detail.
struct Label<'a> {
    icon: Icon,
    name: &'a str,
    sub: Option<&'a str>,
}

/// One row of the drawer: its label and its `end`, in one text layer.
fn line(p: &mut Paint<'_>, id: u64, f: Frame, Label { icon, name, sub }: Label<'_>, end: End<'_>, focused: bool) {
    let from = p.plan.nodes.len();
    let quiet = p.ink(if focused { Role::Ink } else { Role::Ink2 }, false);
    let glyph = 30.0 * p.scale;
    let small = 28.0 * p.scale;
    let mut marks = vec![p.mark(icon, glyph, PAD_X, (f.h - glyph) / 2.0, Role::Ink, false)];
    let mut words = Vec::new();
    let mut right = f.w - PAD_X;
    match end {
        End::None => {}
        End::Mark(icon, role) => {
            right -= small;
            marks.push(p.mark(icon, small, right, (f.h - small) / 2.0, role, false));
        }
        End::Stepper(value) => {
            let ty = p.bold(Type::Label);
            let value = ellipsise(p.fonts, value, ty, ((f.w - 2.0 * PAD_X) * 0.45).floor());
            let value_w = p.fonts.measure(&value, ty).width.ceil();
            right -= small;
            marks.push(p.mark(Icon::ChevronRight, small, right, (f.h - small) / 2.0, Role::Ink, false));
            right -= ARROW_GAP + value_w;
            let mut word = p.word(value, Type::Label, p.ink(Role::Ink, false), right, (f.h - p.line(Type::Label)) / 2.0, value_w);
            word.ty = ty;
            words.push(word);
            right -= ARROW_GAP + small;
            marks.push(p.mark(Icon::Back, small, right, (f.h - small) / 2.0, Role::Ink, false));
        }
        End::Switch(on) => {
            // The track is the switch's one layer; the knob and the tick ride in the row's text layer.
            right -= SWITCH_W;
            let top = (f.h - 48.0) / 2.0;
            p.surface(id + 7, Frame::new(f.x + right, f.y + top, SWITCH_W, 48.0), if on { Fill::Accent } else { Fill::Track }, Radius::Pill, false, false, false, 11);
            let knob = Mark { form: Form::Disc(36), colour: p.ink(if on { Role::AccentInk } else { Role::Ink2 }, false), x: right + if on { 40.0 } else { 6.0 }, y: top + 6.0 };
            marks.push(knob);
            if on {
                marks.push(p.mark(Icon::Tick, 24.0, right + 46.0, top + 12.0, Role::Accent, false));
            }
        }
    }
    let x = PAD_X + glyph + ROOM;
    let room = (right - ROOM - x).max(0.0);
    let name_h = p.line(Type::Body);
    let sub_h = sub.map_or(0.0, |_| 2.0 + p.line(Type::Caption));
    let y = (f.h - name_h - sub_h) / 2.0;
    let ty = p.bold(Type::Body);
    let name = ellipsise(p.fonts, name, ty, room);
    let mut word = p.word(name, Type::Body, p.ink(Role::Ink, false), x, y, room);
    word.ty = ty;
    words.push(word);
    if let Some(sub) = sub {
        let ty = p.ty(Type::Caption);
        let sub = ellipsise(p.fonts, sub, ty, room);
        words.push(p.word(sub, Type::Caption, quiet, x, y + name_h + 2.0, room));
    }
    p.words(id, f, words, marks, 12);
    focus::treat(p, from, id + 10, f, Kind::Row, focused);
}

pub(super) fn quick(p: &mut Paint<'_>) {
    if p.model.view.top() == Some(Layer::QuickSettings) {
        settings(p);
    } else {
        menu(p);
    }
}

fn menu(p: &mut Paint<'_>) {
    let m = p.model;
    let content = drawer::open(p, Side::Right, drawer::WIDTH, "Menu");
    let from = p.plan.nodes.len();
    let subject = m.quick_subject();
    let mut y = content.y;
    for (i, item) in m.quick_items().into_iter().enumerate() {
        let (icon, name, sub, end) = match item {
            Item::Refresh => (Icon::Refresh, "Refresh libraries", m.refresh_note(), End::None),
            Item::Accounts => {
                let attention = m.attention();
                let end = if attention.is_some() { End::Mark(Icon::Alert, Role::Warning) } else { End::None };
                (Icon::Person, "Accounts and PCs", attention.map(|a| a.words), end)
            }
            Item::Settings => (Icon::Cog, "Settings", None, End::None),
            Item::GamePage => (Icon::Info, "Game page", None, End::None),
            Item::Pin => (Icon::Pin, if subject.is_some_and(|id| m.pinned(id)) { "Unpin from Home" } else { "Pin to Home" }, None, End::None),
            Item::Hide => (Icon::EyeOff, "Hide", None, End::None),
        };
        if item == Item::GamePage {
            if let Some(game) = subject.and_then(|id| m.game(id)) {
                y += GROUP_ABOVE;
                let mut word = p.word(game.title.as_str(), Type::Label, p.ink(Role::Ink2, false), 0.0, (GROUP_H - p.line(Type::Label)) / 2.0, content.w - 56.0);
                word.ty = p.bold(Type::Label);
                p.words(9200, Frame::new(content.x + 28.0, y, content.w - 56.0, GROUP_H), vec![word], vec![], 12);
                y += GROUP_H + GROUP_BELOW;
            }
        }
        let h = if sub.is_some() { p.tokens.row.max(p.line(Type::Body) + 2.0 + p.line(Type::Caption) + 16.0) } else { p.tokens.row };
        let f = Frame::new(content.x, y, content.w, h);
        let focus = Focus::SheetRow(i);
        line(p, 9000 + 20 * i as u64, f, Label { icon, name, sub: sub.as_deref() }, end, p.focused(focus));
        p.hit(focus, f);
        y += h + GAP;
    }
    hints::bar(p, &m.hints(), Side::Left, m.hints_dim);
    drawer::lift(p, from);
}

fn settings(p: &mut Paint<'_>) {
    let m = p.model;
    let crumb = Focus::SheetAction(0);
    let content = drawer::open_crumbed(p, Side::Right, drawer::WIDTH, ("Menu", crumb), "Quick settings");
    let from = p.plan.nodes.len();
    if let Some(chip) = p.plan.nodes.iter().find(|n| n.id == 8803).map(|n| n.frame) {
        p.hit(crumb, chip);
    }
    let h = p.tokens.row;
    for (i, setting) in QuickSetting::ALL.into_iter().enumerate() {
        let f = Frame::new(content.x, content.y + i as f32 * (h + GAP), content.w, h);
        let focus = Focus::SheetRow(i);
        let focused = p.focused(focus);
        let value = setting.value(m);
        let end = if setting.is_switch() { End::Switch(setting.on(m)) } else { End::Stepper(&value) };
        line(p, 9000 + 20 * i as u64, f, Label { icon: setting.icon(), name: setting.label(), sub: None }, end, focused);
        p.hit(focus, f);
    }
    hints::bar(p, &m.hints(), Side::Left, m.hints_dim);
    all_settings(p);
    drawer::lift(p, from);
}

/// The glass button at the foot's right that opens the full settings. Uncached, like the crumb: it appears after the page's first frames.
fn all_settings(p: &mut Paint<'_>) {
    let focus = Focus::SheetAction(1);
    let label = "All settings";
    let icon = 28.0 * p.scale;
    let ty = p.bold(Type::Label);
    let text_w = p.fonts.measure(label, ty).width.ceil();
    let (w, h) = (24.0 + icon + 12.0 + text_w + 28.0, BUTTON_H.max(p.tokens.target));
    let foot = p.anchor;
    let f = Frame::new(foot.x + foot.w - w, foot.y + foot.h - h, w, h);
    let focused = p.focused(focus);
    let from = p.plan.nodes.len();
    // At rest the button is its words alone: the drawer already sits over a whole page, and a resting pill would be one more layer.
    if focused {
        p.surface(9210, f, Fill::Glass, Radius::Pill, false, true, false, 12);
    }
    let mut word = p.word(label, Type::Label, p.ink(Role::Ink, false), 24.0 + icon + 12.0, (h - p.line(Type::Label)) / 2.0, text_w);
    word.ty = ty;
    let mark = p.mark(Icon::Cog, icon, 24.0, (h - icon) / 2.0, Role::Ink, false);
    p.words(9211, f, vec![word], vec![mark], 13);
    if focused {
        p.zoom(from, f, p.tokens.focus.button_scale);
    }
    p.hit(focus, f);
}
