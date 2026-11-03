//! Where everything of the quick panel goes (`QuickPanel.qml`, `QuickTile.qml`, `QuickEdit.qml`, `QuickPopup.qml` of the QML shell, the
//! same equations): the sheet, its head, the profiles' strip, the rows with their words broken into lines by the measure of the type, the
//! foot, the PIN pad, the edit list and the card beside the sheet. Pure: a function of the view and the kit (the type roles at the person's
//! text size, the tokens), so what is drawn is what the tests measure and the lint reads.
//!
//! Sizes are the theme's: `space` steps, the `target` and `row`, the radii. The row's rule is the QML's: an icon tile, the name and, only
//! when it says something, a second line; a control that fits on the name's line sits at its right, one that needs width (a stepper's value)
//! goes at the right of the words on the same line when the words, as they would be written on one line, and the control fit side by side,
//! else on a line of its own under them. Words wrap and the row grows: nothing is cut off, at any text size.

use super::quick::tiles::{Kind, Row};
use super::quick::{PopupView, View};
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Chip, Stepper, Toggle};
use crate::kit::wrap;
use crate::kit::{Kit, hint::Hint};
use lumatv_tokens::TypeRole;
use ui::components::Align;
use ui::geometry::Frame;

pub const SHEET_W: f32 = 560.0;
/// 40 from the right edge and 56 from the top and bottom (the Notification Centre's place).
pub const EDGE_X: f32 = 40.0;
pub const EDGE_Y: f32 = 56.0;
pub const SHEET_H: f32 = 1080.0 - 2.0 * EDGE_Y;
/// How far the sheet travels to come in: the edge and its own width.
pub const TRAVEL: f32 = EDGE_X + SHEET_W;

pub fn sheet() -> Frame {
    Frame::new(1920.0 - EDGE_X - SHEET_W, EDGE_Y, SHEET_W, SHEET_H)
}

/// What sits at a row's right.
#[derive(Clone, Debug, PartialEq)]
pub enum Trail {
    None,
    Toggle,
    /// A chip: a state tag (AI access's On) or an action's button words.
    Chip {
        text: String,
        tone: String,
        w: f32,
        h: f32,
    },
    /// The value's words and a chevron.
    Value {
        text: String,
        text_w: f32,
        chevron: bool,
        w: f32,
        h: f32,
    },
}

impl Trail {
    pub fn width(&self, kit: &Kit) -> f32 {
        match self {
            Trail::None => 0.0,
            Trail::Toggle => Toggle::size(kit).0,
            Trail::Chip { w, .. } | Trail::Value { w, .. } => *w,
        }
    }

    pub fn height(&self, kit: &Kit) -> f32 {
        match self {
            Trail::None => 0.0,
            Trail::Toggle => Toggle::size(kit).1,
            Trail::Chip { h, .. } | Trail::Value { h, .. } => *h,
        }
    }
}

/// A control that needs width: a stepper's value (also a nudge's).
#[derive(Clone, Debug, PartialEq)]
pub struct Below {
    pub need: f32,
    pub h: f32,
    pub value: String,
    /// Where it is in the row and how wide.
    pub frame: Frame,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowLayout {
    pub id: String,
    pub kind: Kind,
    pub icon: String,
    pub h: f32,
    pub stacked: bool,
    pub beside: bool,
    pub tile: Frame,
    pub words: Frame,
    pub label: Vec<String>,
    pub detail: Vec<String>,
    pub trail: Trail,
    pub trail_at: (f32, f32),
    pub below: Option<Below>,
}

impl RowLayout {
    /// What the row's layers depend on: when this differs from what was built the row is built again (a value that only changes in place
    /// is not in it).
    pub fn signature(&self) -> String {
        format!(
            "{}|{:?}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            self.id,
            self.kind,
            self.icon,
            self.h,
            self.label,
            self.detail,
            self.trail,
            self.below.as_ref().map(|b| (b.need, b.h, b.frame)),
            (self.stacked, self.beside),
            self.words
        )
    }
}

/// A row's value text as the control shows it: a stepper's current choice, a nudge's reading.
pub fn below_text(r: &Row) -> Option<String> {
    match r.kind {
        Kind::Stepper => Some(r.index.and_then(|i| r.options.get(i)).cloned().unwrap_or_else(|| r.value.clone())),
        Kind::Nudge => Some(r.value.clone()),
        _ => None,
    }
}

/// One row laid out for a column `width` wide (its own box; the caller places it).
pub fn row_layout(kit: &mut Kit, r: &Row, width: f32) -> RowLayout {
    let ink = kit.ui.look.ink();
    let ink2 = kit.ui.look.ink2();
    let pad = kit.space(3);
    let gap = kit.space(4);
    let tile = (kit.tokens.target * 0.75).round();
    let text_x = pad + if r.icon.is_empty() { 0.0 } else { tile + gap };
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let label_line = kit.line(TypeRole::Label);
    let caption_line = kit.line(TypeRole::Caption);

    let trail = match r.kind {
        Kind::Toggle => Trail::Toggle,
        Kind::Value | Kind::Action => {
            let (text, tone, chip) =
                if r.kind == Kind::Action { (r.button.clone(), String::new(), true) } else { (r.value.clone(), r.tone.to_string(), r.chip) };
            if chip {
                let (w, h) = Chip::size(kit, &text, &tone);
                Trail::Chip { text, tone, w, h }
            } else {
                let chevron_side = (kit.tokens.target * 0.5).round();
                let text_w = if text.is_empty() { 0.0 } else { width_of(kit, role_style(kit, TypeRole::Label, None, ink2, Align::Left), &text) };
                let chevron = r.chevron;
                let w = text_w + if text_w > 0.0 && chevron { kit.space(3) } else { 0.0 } + if chevron { chevron_side } else { 0.0 };
                let h = if text_w > 0.0 {
                    label_line.max(if chevron { chevron_side } else { 0.0 })
                } else if chevron {
                    chevron_side
                } else {
                    0.0
                };
                Trail::Value { text, text_w, chevron, w, h }
            }
        }
        _ => Trail::None,
    };
    let trail_w = trail.width(kit);
    let trail_h = trail.height(kit);

    let below_value = below_text(r);
    let (need, below_h) = match &below_value {
        Some(v) => (Stepper::need(kit, v, r.widest), Stepper::height(kit)),
        None => (0.0, 0.0),
    };
    let label_w = width_of(kit, label_style, &r.label);
    let detail_w = width_of(kit, caption_style, &r.detail);
    // A row that keeps lines for an answer that comes late (`detail_lines`) is always stacked: whether the answer would fit beside the control
    // is not known until it comes, and a row that moved when it did would move the rows below it while the panel slides in.
    let beside = need > 0.0 && r.detail_lines <= 1 && label_w.max(detail_w) + gap + need <= width - text_x - pad;
    let text_w = (width - text_x - pad - if trail_w > 0.0 { trail_w + gap } else { 0.0 } - if beside { need + gap } else { 0.0 }).max(1.0);
    let shown_detail = if detail_w > text_w { r.detail.replace(" \u{b7} ", "\n") } else { r.detail.clone() };
    let label = wrap::lines(kit, label_style, &r.label, text_w, 12);
    let detail = if r.detail.is_empty() { Vec::new() } else { wrap::lines(kit, caption_style, &shown_detail, text_w, 12) };
    let words_h = label.len() as f32 * label_line + detail.len() as f32 * caption_line;
    // a row whose second line is an answer that arrives later keeps its lines, so the answer changes no row's height
    let kept = if r.detail_lines > 1 && !r.detail.is_empty() { label.len() as f32 * label_line + f32::from(r.detail_lines) * caption_line } else { 0.0 };
    let text_h = words_h.max(kept);
    let stacked = below_h > 0.0 && !beside;
    let h = if stacked {
        pad + text_h + kit.space(2) + below_h + pad
    } else {
        kit.tokens.row.max(pad + text_h.max(tile).max(trail_h).max(if beside { below_h } else { 0.0 }) + pad)
    };
    let tile_y = if stacked { pad } else { ((h - tile) / 2.0).round() };
    let words_y = if stacked { pad } else { ((h - words_h) / 2.0).round() };
    let below = below_value.map(|value| {
        let (x, y, w) =
            if beside { (width - pad - need, ((h - below_h) / 2.0).round(), need) } else { (text_x, pad + text_h + kit.space(2), width - text_x - pad) };
        Below { need, h: below_h, value, frame: Frame::new(x, y, w, below_h) }
    });
    RowLayout {
        id: r.id.clone(),
        kind: r.kind,
        icon: r.icon.clone(),
        h,
        stacked,
        beside,
        tile: Frame::new(pad, tile_y, tile, tile),
        words: Frame::new(text_x, words_y, text_w, words_h),
        label,
        detail,
        trail_at: (width - pad - trail_w, ((h - trail_h) / 2.0).round()),
        trail,
        below,
    }
}

/// The head: the title in lines, the clock beside it while both fit on a line, else under it (a large text size); in edit mode and on the PIN pad the hints
/// of what the keys do take the clock's place.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadLayout {
    pub title: Vec<String>,
    pub title_w: f32,
    pub clock: Option<(String, f32, f32)>,
    pub hints: Vec<Hint>,
    pub hints_at: (f32, f32),
    pub below: bool,
    pub h: f32,
}

pub fn head_layout(kit: &mut Kit, v: &View, inner: f32) -> HeadLayout {
    let title_style = role_style(kit, TypeRole::Title, Some(ui::text::Weight::SemiBold), kit.ui.look.ink(), Align::Left);
    let title_line = kit.line(TypeRole::Title);
    let label_line = kit.line(TypeRole::Label);
    let one_line = width_of(kit, title_style, &v.title);
    let gap = kit.space(4);
    let hinted = v.editing || !v.hints_head.is_empty();
    let (side_w, side_h) = if hinted {
        (
            super::ui::hints_width(kit, &v.hints_head, TypeRole::Label, kit.space(5)),
            super::ui::hint_width(kit, &Hint::key(crate::kit::hint::Key::Ok, crate::kit::hint::Action::Move), TypeRole::Label) * 0.0
                + crate::kit::hint::glyph_side(kit, TypeRole::Label) as f32,
        )
    } else {
        let w = if v.clock.is_empty() { 0.0 } else { width_of(kit, role_style(kit, TypeRole::Label, None, kit.ui.look.ink2(), Align::Left), &v.clock) };
        (w, label_line)
    };
    let below = side_w > 0.0 && one_line + side_w + gap > inner;
    let title_w = if below || side_w == 0.0 { inner } else { inner - side_w - gap };
    let title = wrap::lines(kit, title_style, &v.title, title_w, 3);
    let title_h = title.len() as f32 * title_line;
    let h = if side_w == 0.0 {
        title_h
    } else if below {
        title_h + side_h
    } else {
        title_h.max(side_h)
    };
    let (clock, hints_at) = if hinted {
        (None, if below { (0.0, title_h) } else { (inner - side_w, ((title_h.min(title_line) - side_h) / 2.0).round().max(0.0)) })
    } else if v.clock.is_empty() {
        (None, (0.0, 0.0))
    } else if below {
        (Some((v.clock.clone(), 0.0, title_h)), (0.0, 0.0))
    } else {
        (Some((v.clock.clone(), inner - side_w, ((title_line - label_line) / 2.0).round())), (0.0, 0.0))
    };
    HeadLayout { title, title_w, clock, hints: if hinted { v.hints_head.clone() } else { Vec::new() }, hints_at, below, h }
}

/// Where the parts of an edit row go (relative to the row), and how tall every row is.
#[derive(Clone, Debug, PartialEq)]
pub struct EditMetrics {
    pub row_h: f32,
    /// The marks (up, down, remove) are under the words, not beside them: at a text size where the names would be cut beside them.
    pub stacked: bool,
    pub tile_side: f32,
    pub words_x: f32,
    pub words_y: f32,
    pub words_w: f32,
    /// The most lines a detail takes (stacked rows give a long one a second line).
    pub detail_lines: usize,
    pub mark: f32,
    pub grip_w: f32,
    pub grip_x: f32,
    pub grip_y: f32,
}

/// The edit list's rows: an icon tile, the name (and what it says under it) and the three marks. Beside the words while every name and every
/// line under it fits there, else under them with the words given the row's width: a large text size grows every row alike and no name is cut
/// off (an elision is the last resort, for a name that is longer than the row itself).
pub fn edit_metrics(kit: &mut Kit, area_w: f32, texts: &[(&str, &str)]) -> EditMetrics {
    let tile_side = (kit.tokens.target * 0.75).round();
    let words_x = kit.space(3) + tile_side + kit.space(4);
    let mark = tile_side;
    let grip_w = 3.0 * mark + 2.0 * kit.space(1);
    let beside_w = area_w - words_x - grip_w - 2.0 * kit.space(3);
    let (label, detail) =
        (role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Left), role_style(kit, TypeRole::Caption, None, kit.ui.look.ink2(), Align::Left));
    let fits = texts.iter().all(|(l, d)| width_of(kit, label, l) <= beside_w && (d.is_empty() || width_of(kit, detail, d) <= beside_w));
    let (label_line, caption_line) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
    let two_lines = (kit.tokens.row * 0.95).round().max(label_line + caption_line + kit.space(4));
    if fits {
        return EditMetrics {
            row_h: two_lines,
            stacked: false,
            tile_side,
            words_x,
            words_y: 0.0,
            words_w: beside_w,
            detail_lines: 1,
            mark,
            grip_w,
            grip_x: area_w - kit.space(3) - grip_w,
            grip_y: ((two_lines - mark) / 2.0).round(),
        };
    }
    // the marks take a line of their own at the foot of the row, at its right; a detail longer than the words' room goes on to a second line
    let top = kit.space(2);
    let words_w = area_w - words_x - kit.space(3);
    let detail_lines = texts.iter().map(|(_, d)| if d.is_empty() { 1 } else { wrap::lines(kit, detail, d, words_w, 2).len() }).max().unwrap_or(1);
    let row_h = (top + label_line + caption_line * detail_lines as f32 + kit.space(1) + mark + top).ceil().max(two_lines);
    EditMetrics {
        row_h,
        stacked: true,
        tile_side,
        words_x,
        words_y: top,
        words_w,
        detail_lines,
        mark,
        grip_w,
        grip_x: area_w - kit.space(3) - grip_w,
        grip_y: row_h - top - mark,
    }
}

/// The strip of profiles above the list.
#[derive(Clone, Debug, PartialEq)]
pub struct StripLayout {
    pub cells: Vec<Frame>,
    pub avatar: f32,
    pub h: f32,
}

pub fn avatar_side(kit: &Kit) -> f32 {
    (kit.tokens.target * 0.875).round()
}

pub fn strip_layout(kit: &Kit, n: usize, inner: f32) -> StripLayout {
    let gap = kit.space(2);
    let n_f = n.max(1) as f32;
    let cell_w = ((inner - gap * (n_f - 1.0)) / n_f).floor().min(120.0);
    let total = cell_w * n_f + gap * (n_f - 1.0);
    let x0 = ((inner - total) / 2.0).floor();
    let h = gap + avatar_side(kit) + kit.space(1) + kit.line(TypeRole::Caption) + gap;
    StripLayout { cells: (0..n).map(|i| Frame::new(x0 + i as f32 * (cell_w + gap), 0.0, cell_w, h)).collect(), avatar: avatar_side(kit), h }
}

/// The PIN pad in place: an avatar and the name, four dots, what happened, and twelve keys.
#[derive(Clone, Debug, PartialEq)]
pub struct PadLayout {
    pub avatar: Frame,
    pub name: Frame,
    pub dots: Vec<Frame>,
    pub note: Frame,
    pub cells: Vec<Frame>,
    pub bottom: f32,
}

/// `avail` is how tall the room under the head is: the keys give way to it (never below a label and a hair of air), and when the words above
/// them still take too much (large text) the avatar is left out, since the name says who it is.
pub fn pad_layout(kit: &Kit, inner: f32, avail: f32) -> PadLayout {
    let s1 = kit.space(1);
    let gap = kit.space(3);
    let name_h = kit.line(TypeRole::Title);
    let d = 28.0;
    let note_h = kit.line(TypeRole::Caption);
    let ch_nominal = kit.tokens.target.max(kit.line(TypeRole::Label) + 2.0 * kit.space(2)).ceil();
    let ch_least = (kit.line(TypeRole::Label) + 2.0 * s1).ceil();
    let words = name_h + kit.space(3) + d + kit.space(3) + note_h + kit.space(4);
    let keys = |ch: f32| 4.0 * ch + 3.0 * gap;
    let mut av = (kit.tokens.target * 1.125).round();
    let mut ch = ch_nominal;
    if av + s1 + words + keys(ch) > avail {
        ch = ((avail - av - s1 - words - 3.0 * gap) / 4.0).floor().clamp(ch_least, ch_nominal);
    }
    if av + s1 + words + keys(ch) > avail {
        av = 0.0;
        ch = ((avail - words - 3.0 * gap) / 4.0).floor().clamp(ch_least, ch_nominal);
    }
    let mut y = 0.0;
    let avatar = Frame::new(((inner - av) / 2.0).round(), y, av, av);
    if av > 0.0 {
        y += av + s1;
    }
    let name = Frame::new(0.0, y, inner, name_h);
    y += name_h + kit.space(3);
    let dgap = kit.space(4);
    let dots_w = 4.0 * d + 3.0 * dgap;
    let dots = (0..4).map(|i| Frame::new(((inner - dots_w) / 2.0).round() + i as f32 * (d + dgap), y, d, d)).collect();
    y += d + kit.space(3);
    let note = Frame::new(0.0, y, inner, note_h);
    y += note.h + kit.space(4);
    let cw = ((inner - 2.0 * gap) / 3.0).floor();
    let x0 = ((inner - (3.0 * cw + 2.0 * gap)) / 2.0).floor();
    let cells: Vec<Frame> = (0..12).map(|i| Frame::new(x0 + (i % 3) as f32 * (cw + gap), y + (i / 3) as f32 * (ch + gap), cw, ch)).collect();
    let bottom = y + keys(ch);
    PadLayout { avatar, name, dots, note, cells, bottom }
}

/// The card beside the sheet: 48 px from it, level with the second row of the head. `h` is as tall as its content.
pub fn popup_x(kit: &Kit) -> f32 {
    sheet().x - (SHEET_W + kit.space(8) + kit.space(2))
}

pub const POPUP_Y: f32 = EDGE_Y + 81.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PopupLayout {
    pub frame: Frame,
    pub title: Vec<String>,
    pub title_w: f32,
    pub value: Option<(String, f32)>,
    pub text: Vec<String>,
    /// The slider's box and the places in it, or the option rows.
    pub slider: Option<SliderLayout>,
    pub options: Vec<Frame>,
    pub body_y: f32,
    /// The options are taller than the room the card has (a long list at a large text size): the card is as tall as the screen allows and the
    /// options move in a window of it.
    pub scroll: Option<PopupScroll>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PopupScroll {
    /// Where the options show, on the screen.
    pub window: Frame,
    /// How tall the options are together, from the window's top.
    pub content_h: f32,
    /// Each option's top from the window's top.
    pub tops: Vec<f32>,
}

impl PopupLayout {
    /// The card rises to the top of the safe area and fills the room to `limit`; the options go in a window of what is under its words.
    fn scroll_options(&mut self, kit: &Kit, limit: f32) {
        let (Some(first), Some(last)) = (self.options.first(), self.options.last()) else { return };
        let content_h = last.y + last.h - first.y;
        self.frame.y = POPUP_EDGE;
        self.frame.h = limit - POPUP_EDGE;
        let window = Frame::new(self.frame.x + first.x, self.frame.y + first.y, first.w, (self.frame.h - first.y - kit.space(5)).max(1.0));
        self.scroll = Some(PopupScroll { window, content_h, tops: self.options.iter().map(|o| o.y - first.y).collect() });
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SliderLayout {
    pub box_: Frame,
    pub icon_l: Frame,
    pub icon_r: Frame,
    pub track: Frame,
    pub marks: Frame,
    pub mark_w: f32,
}

/// The card keeps this far from the top and bottom of the screen (the safe area).
const POPUP_EDGE: f32 = 54.0;

/// The card beside the sheet. When its content is taller than the room (a list of options at large text) it rises to the top of the safe area,
/// then gives up lines of its explanation one at a time; it never leaves the screen for a list of up to seven or eight options.
pub fn popup_layout(kit: &mut Kit, p: &PopupView) -> PopupLayout {
    let limit = 1080.0 - POPUP_EDGE;
    let mut text_max = 6;
    loop {
        let mut l = popup_layout_with(kit, p, text_max);
        if l.frame.y + l.frame.h > limit {
            l.frame.y = (limit - l.frame.h).max(POPUP_EDGE);
        }
        let fits = l.frame.y + l.frame.h <= limit;
        if fits || text_max == 1 || l.text.is_empty() {
            if !fits {
                l.scroll_options(kit, limit);
            }
            return l;
        }
        text_max -= 1;
    }
}

fn popup_layout_with(kit: &mut Kit, p: &PopupView, text_max: usize) -> PopupLayout {
    let pad = kit.space(6);
    let w = SHEET_W;
    let body_w = w - 2.0 * pad;
    let ink = kit.ui.look.ink();
    let title_style = role_style(kit, TypeRole::Title, Some(ui::text::Weight::SemiBold), ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink2(), Align::Left);
    let (title_s, text_s) = match p {
        PopupView::Slider { title, text, .. } | PopupView::Pick { title, text, .. } => (title.clone(), text.clone()),
    };
    let value = match p {
        PopupView::Slider { value, .. } => Some(format!("{}", value.round() as i64)),
        PopupView::Pick { .. } => None,
    };
    let value_w = value.as_ref().map_or(0.0, |v| width_of(kit, title_style, v));
    let title_w = body_w - if value_w > 0.0 { value_w + kit.space(3) } else { 0.0 };
    let title = wrap::lines(kit, title_style, &title_s, title_w, 1);
    let title_h = kit.line(TypeRole::Title);
    let text = if text_s.is_empty() { Vec::new() } else { wrap::lines(kit, caption_style, &text_s, body_w, text_max) };
    let gap = kit.space(3);
    let mut y = pad + title_h + gap;
    if !text.is_empty() {
        y += text.len() as f32 * kit.line(TypeRole::Caption) + gap;
    }
    let body_y = y;
    let (mut slider, mut options) = (None, Vec::new());
    match p {
        PopupView::Slider { marks, .. } => {
            let icon = 26.0;
            let row_h = kit.tokens.target;
            let in_x = kit.space(4);
            let box_h = row_h + kit.line(TypeRole::Caption) + kit.space(5);
            let box_ = Frame::new(pad, body_y, body_w, box_h);
            let row_y = body_y + kit.space(3);
            let track_x = pad + in_x + icon + gap;
            let track_w = body_w - 2.0 * (in_x + icon + gap);
            let icon_l = Frame::new(pad + in_x, row_y + ((row_h - icon) / 2.0).round(), icon, icon);
            let icon_r = Frame::new(pad + body_w - in_x - icon, icon_l.y, icon, icon);
            let track = Frame::new(track_x, row_y, track_w, row_h);
            let marks_f = Frame::new(track_x, row_y + row_h, track_w, kit.line(TypeRole::Caption));
            let _ = marks;
            slider = Some(SliderLayout { box_, icon_l, icon_r, track, marks: marks_f, mark_w: track_w / 3.0 });
            y = body_y + box_h;
        }
        PopupView::Pick { options: opts, .. } => {
            let label_style = role_style(kit, TypeRole::Label, Some(ui::text::Weight::SemiBold), ink, Align::Left);
            for o in opts {
                let x = kit.space(4) + if o.icon.is_empty() { 0.0 } else { 28.0 + kit.space(4) };
                let lines = wrap::lines(kit, label_style, &o.label, body_w - x - 120.0, 4);
                let h = 72f32.max(lines.len() as f32 * kit.line(TypeRole::Label) + kit.space(4));
                options.push(Frame::new(pad, y, body_w, h));
                y += h + kit.space(1);
            }
            y -= if opts.is_empty() { 0.0 } else { kit.space(1) };
        }
    }
    let h = y + kit.space(5);
    let frame = Frame::new(popup_x(kit), POPUP_Y, w, h);
    PopupLayout { frame, title, title_w, value: value.map(|v| (v, value_w)), text, slider, options, body_y, scroll: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::quick::tiles::Kind;

    fn row(kind: Kind) -> Row {
        let mut r = tiles_row("picture", kind);
        r.label = "Picture mode".into();
        r
    }

    fn tiles_row(id: &str, kind: Kind) -> Row {
        let env = crate::panels::env::Env::new(serde_json::json!({}));
        let mut r = crate::panels::quick::tiles::tiles(&env).remove(0);
        r.id = id.into();
        r.kind = kind;
        r
    }

    #[test]
    fn the_sheet_is_560_wide_40_from_the_right_edge_and_56_from_the_top_and_bottom() {
        let s = sheet();
        assert_eq!((s.x, s.y, s.w, s.h), (1320.0, 56.0, 560.0, 968.0));
        assert_eq!(TRAVEL, 600.0);
    }

    #[test]
    fn a_stepper_whose_words_and_value_fit_side_by_side_sits_at_the_right_else_on_a_line_of_its_own() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut r = row(Kind::Stepper);
        r.options = vec!["Filmmaker".into(), "Cinema".into()];
        r.index = Some(1);
        r.detail = String::new();
        let beside = row_layout(&mut kit, &r, 512.0);
        assert!(beside.beside && !beside.stacked, "a short name leaves room for the control");
        assert_eq!(beside.h, 80.0, "the row's own height");
        let b = beside.below.as_ref().unwrap();
        assert_eq!(b.frame.x + b.frame.w, 512.0 - 12.0, "at the row's right inside its padding");
        r.detail = "HDMI 1 \u{b7} Dolby Vision".into();
        let stacked = row_layout(&mut kit, &r, 512.0);
        assert!(stacked.stacked && !stacked.beside, "the long second line pushes the control down");
        let b = stacked.below.as_ref().unwrap();
        assert_eq!((b.frame.x, b.frame.w), (76.0, 512.0 - 76.0 - 12.0), "under the words, as wide as they are");
        assert!(stacked.h > 130.0, "{}", stacked.h);
        assert_eq!(b.frame.y, 12.0 + kit.line(TypeRole::Label) + kit.line(TypeRole::Caption) + 8.0);
    }

    #[test]
    fn the_receivers_row_keeps_two_lines_for_its_answer_so_the_answer_changes_no_height() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut r = row(Kind::Nudge);
        r.id = "receiver".into();
        r.label = "Denon receiver".into();
        r.detail_lines = 2;
        r.widest = "\u{2212}34.5 dB";
        r.value = "\u{2013}".into();
        r.detail = "Not answering".into();
        let silent = row_layout(&mut kit, &r, 512.0);
        r.detail = "TV Audio \u{b7} Dolby Surround".into();
        r.value = "-34.5 dB".into();
        let answered = row_layout(&mut kit, &r, 512.0);
        assert_eq!(silent.h, answered.h, "the same two lines either way");
        assert!(silent.stacked && answered.stacked, "a row that waits for an answer is stacked whatever the answer says");
        assert_eq!(silent.below.as_ref().map(|b| b.need), answered.below.as_ref().map(|b| b.need), "the control is sized to the widest value");
    }

    #[test]
    fn a_second_line_that_does_not_fit_breaks_at_its_dots_and_a_toggle_and_a_chip_sit_at_the_right() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut toggle = tiles_row("plugin.ambient.ambient", Kind::Toggle);
        toggle.label = "Ambient light".into();
        toggle.detail = "Follows the picture \u{b7} 70% of full brightness".into();
        let t = row_layout(&mut kit, &toggle, 512.0);
        assert_eq!(t.trail, Trail::Toggle);
        assert_eq!(t.trail_at.0, 512.0 - 12.0 - 84.0);
        assert!(t.detail.len() >= 2 && t.detail[0] == "Follows the picture", "{:?}", t.detail);
        assert!(t.words.w <= 512.0 - 76.0 - 12.0 - 84.0 - 16.0 + 0.5, "the words give the toggle its room");
        let mut ai = tiles_row("ai", Kind::Value);
        ai.chip = true;
        ai.chevron = false;
        ai.value = "On".into();
        ai.tone = "positive";
        let a = row_layout(&mut kit, &ai, 512.0);
        assert!(matches!(a.trail, Trail::Chip { .. }));
    }

    #[test]
    fn at_200_percent_every_row_grows_and_nothing_is_cut() {
        let (_h, mut kit1) = testkit::kit(1.0);
        let (_h2, mut kit2) = testkit::kit(2.0);
        let mut r = row(Kind::Toggle);
        r.label = "Magic Remote pointer".into();
        r.detail = "Speakers and headphones".into();
        let a = row_layout(&mut kit1, &r, 512.0);
        let b = row_layout(&mut kit2, &r, 512.0);
        assert!(b.h > a.h, "{} against {}", b.h, a.h);
        assert!(b.label.len() > 1, "the name wraps at 200 %: {:?}", b.label);
        assert!(b.words.w >= 100.0);
    }

    #[test]
    fn the_head_puts_the_clock_beside_the_title_while_both_fit_else_under_it() {
        let (_h, mut kit) = testkit::kit(1.0);
        let env = crate::panels::env::Env::new(serde_json::json!({}));
        let q = crate::panels::quick::Quick::new();
        let mut v = q.view(&env, 1_761_000_000_000);
        v.clock = "9:52 PM".into();
        let head = head_layout(&mut kit, &v, 512.0);
        assert!(!head.below);
        let (_, x, _) = head.clock.clone().unwrap();
        assert!(x > 300.0, "{x}");
        let (_h2, mut big) = testkit::kit(2.0);
        let head2 = head_layout(&mut big, &v, 512.0);
        assert!(head2.below, "at 200 % the title leaves no room for the clock");
        assert!(head2.title.len() >= 2, "{:?}", head2.title);
        assert!(head2.h > head.h);
    }

    #[test]
    fn the_strip_centres_its_cells_and_the_pad_has_four_dots_and_twelve_keys_three_by_four() {
        let (_h, kit) = testkit::kit(1.0);
        let s = strip_layout(&kit, 2, 512.0);
        assert_eq!(s.cells.len(), 2);
        assert_eq!(s.cells[0].w, 120.0);
        assert_eq!(s.cells[0].x + (s.cells[1].x + s.cells[1].w), 512.0, "centred");
        let six = strip_layout(&kit, 6, 512.0);
        assert!(six.cells[5].x + six.cells[5].w <= 512.0 + 0.5);
        let p = pad_layout(&kit, 512.0, 840.0);
        assert_eq!((p.dots.len(), p.cells.len()), (4, 12));
        assert!(p.cells[0].w > 150.0 && p.cells[0].h >= 64.0);
        assert!(p.cells[11].y > p.cells[0].y && p.cells[1].x > p.cells[0].x);
        assert!(p.bottom < 840.0, "the pad fits the sheet: {}", p.bottom);
        let (_h2, big) = testkit::kit(2.0);
        let squeezed = pad_layout(&big, 512.0, 640.0);
        assert!(squeezed.bottom <= 640.0, "and at 200 % in the room the sheet leaves: {}", squeezed.bottom);
        assert!(squeezed.cells[0].h >= big.line(TypeRole::Label), "the keys keep room for their digit");
    }

    #[test]
    fn the_card_sits_48_px_from_the_sheet_and_is_as_tall_as_its_content() {
        let (_h, mut kit) = testkit::kit(1.0);
        let slider = PopupView::Slider {
            title: "OLED pixel brightness".into(),
            text: "How bright the screen is.".into(),
            from: 0.0,
            to: 100.0,
            step: 5.0,
            value: 40.0,
            marks: ["0", "50", "100"],
            icons: ["sun-dim", "sun"],
        };
        let p = popup_layout(&mut kit, &slider);
        assert_eq!((p.frame.x, p.frame.y, p.frame.w), (712.0, 137.0, 560.0));
        assert_eq!(p.frame.x + p.frame.w + 48.0, sheet().x, "48 px from the sheet");
        let s = p.slider.as_ref().unwrap();
        assert_eq!(s.box_.w, 512.0);
        assert!(s.track.w > 380.0 && s.track.w < 420.0, "{}", s.track.w);
        assert!(p.value.is_some());
        let pick = PopupView::Pick {
            title: "Picture mode".into(),
            text: "How the picture looks.".into(),
            options: (0..7)
                .map(|i| super::super::quick::OptionView { label: format!("Mode {i}"), icon: "image".into(), detail: String::new(), current: i == 1, enabled: true })
                .collect(),
            index: 1,
        };
        let q = popup_layout(&mut kit, &pick);
        assert_eq!(q.options.len(), 7);
        assert!(q.options[0].h >= 72.0);
        assert!(q.frame.h > p.frame.h, "seven rows are taller than a slider");
        assert!(q.frame.y + q.frame.h < 1080.0 - 48.0, "inside the screen: {}", q.frame.h);
    }

    #[test]
    fn the_pick_card_stays_inside_the_screen_at_200_percent() {
        let (_h, mut kit) = testkit::kit(2.0);
        let pick = PopupView::Pick {
            title: "Picture mode".into(),
            text: "How the picture looks. Each input keeps its own.".into(),
            options: (0..7)
                .map(|i| super::super::quick::OptionView { label: format!("Mode {i}"), icon: "image".into(), detail: String::new(), current: i == 1, enabled: true })
                .collect(),
            index: 1,
        };
        let q = popup_layout(&mut kit, &pick);
        assert!(q.frame.y + q.frame.h <= 1080.0 - 54.0, "inside the screen: {:?}", q.frame);
    }
}
