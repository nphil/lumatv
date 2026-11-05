//! The notification drawer drawn (`docs/mocks/13-notifications`): a glass sheet in the quick panel's place and width (560 wide, 40 from
//! the right edge, from the top bar's line), a head with "Notifications" and "Clear all", and one column of what has happened grouped by
//! who said it. A group of one entry is a label and the entry; a group of several is a header (the source, how many it holds, the OK
//! glyph and what it does, and its own Clear) and its newest entry, the rest behind OK. Each entry has its icon tile, its title, up to
//! three lines of what it says, the time (with "×n" where n notifications said exactly this) and the buttons it offers. With nothing to
//! list the sheet is the empty state. The sheet is as tall as what it holds, up to the quick panel's height.
//!
//! The model decides which row (and which of its buttons) has the focus. A row with buttons is lit as a whole while one of its buttons
//! has the focus, which flies from button to button; a row without buttons is the focus itself. The column moves on the focus's
//! spring to show the focused row, and only the rows near the window are made (the scene has a budget), made again when the window
//! comes near others.

use super::drawer::{EntryView, ItemView, View};
use super::life::Phase;
use super::quick_layout as quick;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, Root, Scroll, WordLine, Words, hint_width};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Spot, tile};
use crate::kit::viewport::Reach;
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::Props;
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::Align;
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::ImageLayer;
use ui::text::Weight;

pub const SCOPE: Scope = Scope(202);
const REGION_HEAD: Region = Region(2200);
const REGION_LIST: Region = Region(2201);
const BASE: u32 = 0x2200_0000;
const SHEET_W: f32 = quick::SHEET_W;
const SHEET_MIN_H: f32 = 240.0;
const TILE: f32 = 48.0;
/// Where Home's bell is with the default layout and no profile name yet (`Home::bell_frame`, which the shell hands to the panels each frame): the
/// drawer's way in and out until it is told.
pub const BELL: [f32; 4] = [1678.0, 60.0, 48.0, 48.0];

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Clear,
    /// The header of a stacked group (its number among the rows).
    Head(usize),
    /// The Clear of that header.
    HeadClear(usize),
    Item(usize),
    Button(usize, usize),
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        Id(BASE
            + match self {
                Target::Clear => 1,
                Target::Backdrop => 2,
                Target::Item(i) => 0x1000 + i as u32,
                Target::Button(i, b) => 0x10_0000 + (i as u32) * 16 + b as u32,
                Target::Head(i) => 0x2000_0000 + i as u32,
                Target::HeadClear(i) => 0x3000_0000 + i as u32,
            })
    }

    pub fn from_id(id: Id) -> Option<Target> {
        let n = id.0.checked_sub(BASE)?;
        match n {
            1 => Some(Target::Clear),
            2 => Some(Target::Backdrop),
            0x1000..0x10_0000 => Some(Target::Item((n - 0x1000) as usize)),
            0x10_0000.. if n < 0x1000_0000 => Some(Target::Button(((n - 0x10_0000) / 16) as usize, ((n - 0x10_0000) % 16) as usize)),
            0x2000_0000..0x3000_0000 => Some(Target::Head((n - 0x2000_0000) as usize)),
            0x3000_0000..0x4000_0000 => Some(Target::HeadClear((n - 0x3000_0000) as usize)),
            _ => None,
        }
    }
}

/// What a stacked group's header holds besides its words.
#[derive(Clone, Debug, PartialEq)]
struct Stack {
    /// The header's number among the rows.
    idx: usize,
    /// The OK hint and its width; left out where the words and the Clear leave no room for it.
    hint: Option<(Hint, f32)>,
    clear_w: f32,
    /// The Clear is a row under the words (where the name would be cut beside it at a large text size).
    two_rows: bool,
}

/// One entry of the column laid out: its place in the column (from its top) and what its parts say.
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    /// `source_w` is the width of `source` as shown (elided to what the room allows); `count` is empty for a label.
    Head { top: f32, h: f32, source: String, icon: String, count: String, count_w: f32, source_w: f32, stack: Option<Stack> },
    /// The title is a line of `title_w`; `when_w` is the width of `stamp` where it sits on the title's line (0: it did not fit there and is the first of
    /// `lines`, which are what is under the title, `text_w` wide).
    Item { top: f32, h: f32, item: ItemView, stamp: String, title: String, title_w: f32, lines: Vec<String>, text_w: f32, when_w: f32, buttons: Vec<f32> },
}

impl Slot {
    fn top(&self) -> f32 {
        match self {
            Slot::Head { top, .. } | Slot::Item { top, .. } => *top,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Slot::Head { h, .. } => *h,
            Slot::Item { h, .. } => *h,
        }
    }

    /// Its number among the rows the focus visits (a label has none).
    fn idx(&self) -> Option<usize> {
        match self {
            Slot::Head { stack, .. } => stack.as_ref().map(|s| s.idx),
            Slot::Item { item, .. } => Some(item.idx),
        }
    }
}

/// What an item says of when: the time, and how many notifications said exactly this.
fn stamp(it: &ItemView) -> String {
    match (it.count > 1, it.when.is_empty()) {
        (true, false) => format!("×{} · {}", it.count, it.when),
        (true, true) => format!("×{}", it.count),
        _ => it.when.clone(),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Empty {
    tile: Frame,
    label: Frame,
    text: Vec<String>,
    text_at: Frame,
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    sheet: Frame,
    title_w: f32,
    title_at: (f32, f32),
    clear: Option<Frame>,
    flick: Frame,
    slots: Vec<Slot>,
    content_h: f32,
    empty: Option<Empty>,
}

fn layout(kit: &mut Kit, v: &View, place: Frame) -> Layout {
    let (s1, s2, s3, s4, s5, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(7));
    let ink = kit.ui.look.ink();
    let ink2 = kit.ui.look.ink2();
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let (title_line, label_line, caption_line) = (kit.line(TypeRole::Title), kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
    let sheet_x = place.x;
    let head_y = s4;
    let inner_w = SHEET_W - 2.0 * s5;
    // the head: the button sits beside the title while both fit on a line, else under it (a large text size)
    let natural = width_of(kit, title_style, "Notifications").ceil();
    let (btn_w, btn_h) = (Button::width_for(kit, "Clear all", true), Button::height(kit));
    let has_clear = v.count > 0;
    let stacked = has_clear && natural + btn_w + s4 > inner_w;
    // one line as tall as the button's row, or as the title where that is larger (the empty state at a large text size has no button)
    let line_h = (kit.tokens.target * 1.1).round().max(title_line);
    let head_h = if stacked { title_line + s2 + btn_h } else { line_h };
    let title_w = natural.min(inner_w);
    let title_at = (s5, head_y + if stacked { 0.0 } else { ((line_h - title_line) / 2.0).round() });
    let clear = has_clear.then(|| {
        let (x, y) = if stacked { (s5, head_y + title_line + s2) } else { (SHEET_W - s5 - btn_w, head_y + ((line_h - btn_h) / 2.0).round()) };
        Frame::new(x, y, btn_w, btn_h)
    });
    let below_head = head_y + head_h;
    // the column: what the words of a row start at is the same for a header and an item (under the tile's column)
    let flick_w = SHEET_W - 2.0 * s3;
    let text_x = s4 + TILE + s4;
    let mut slots = Vec::new();
    let mut top = 0.0;
    for e in &v.entries {
        match e {
            EntryView::Head { idx, source, icon, count, expanded } if *idx > 0 => {
                if !slots.is_empty() {
                    top += s2;
                }
                let count = count.to_string();
                let count_w = width_of(kit, caption_style, &count).ceil();
                let clear_w = Button::width_for(kit, "Clear", false);
                let natural = width_of(kit, label_style, source).ceil();
                let hint = Hint::key(Key::Ok, if *expanded { Action::Collapse } else { Action::Expand });
                let hint_w = hint_width(kit, &hint, TypeRole::Caption).ceil();
                // the Clear is at the right of the words; where the name and the count would not fit beside it, it is a row under them
                let beside = flick_w - s4 - clear_w - s3 - text_x;
                let two_rows = natural + s2 + count_w > beside;
                let room = if two_rows { flick_w - s4 - text_x } else { beside };
                let band_h = 28.0f32.max(label_line);
                let h = if two_rows { s2 + band_h + s2 + btn_h + s2 } else { btn_h + 2.0 * s2 };
                // the words, the count and the hint share the room; the hint goes first where they do not all fit, then the name is cut
                let (shown, source_w, hint) = if natural + s2 + count_w + s3 + hint_w <= room {
                    (source.clone(), natural, Some((hint, hint_w)))
                } else {
                    let shown = wrap::elide(kit, label_style, source, (room - s2 - count_w).max(1.0));
                    let w = width_of(kit, label_style, &shown).ceil();
                    (shown, w, None)
                };
                slots.push(Slot::Head { top, h, source: shown, icon: icon.clone(), count, count_w, source_w, stack: Some(Stack { idx: *idx, hint, clear_w, two_rows }) });
                top += h + s1;
            }
            EntryView::Head { source, icon, .. } => {
                let h = (kit.tokens.row * 0.6).round();
                let shown = wrap::elide(kit, caption_style, source, (flick_w - text_x - s5).max(1.0));
                let source_w = width_of(kit, caption_style, &shown).ceil();
                slots.push(Slot::Head { top, h, source: shown, icon: icon.clone(), count: String::new(), count_w: 0.0, source_w, stack: None });
                top += h;
            }
            EntryView::Item(it) => {
                let stamp = stamp(it);
                let stamp_w = if stamp.is_empty() { 0.0 } else { width_of(kit, caption_style, &stamp).ceil() };
                let text_w = (flick_w - text_x - s4).max(1.0);
                // the time and count sit at the right of the title where both fit there; else they are a line under it
                let title_nat = width_of(kit, label_style, &it.title).ceil();
                let beside = stamp_w > 0.0 && title_nat + s3 + stamp_w <= text_w;
                let (when_w, title_w) = if beside { (stamp_w, text_w - stamp_w - s3) } else { (0.0, text_w) };
                let title = wrap::elide(kit, label_style, &it.title, title_w);
                let mut lines = Vec::new();
                if stamp_w > 0.0 && !beside {
                    lines.push(stamp.clone());
                }
                if !it.body.is_empty() {
                    lines.extend(wrap::lines(kit, caption_style, &it.body, text_w, 3));
                }
                let text_h = label_line + lines.len() as f32 * caption_line;
                let buttons: Vec<f32> = it.buttons.iter().map(|b| Button::width_for(kit, b, false)).collect();
                let h = s3 + TILE.max(text_h) + if buttons.is_empty() { 0.0 } else { s3 + btn_h } + s3;
                slots.push(Slot::Item { top, h, item: it.clone(), stamp, title, title_w, lines, text_w, when_w, buttons });
                top += h + s1;
            }
        }
    }
    let content_h = top;
    let (empty, body_h) = if v.rows == 0 {
        let col_w = SHEET_W - 2.0 * s7;
        let text = wrap::lines(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Centre), "What your assistants, plugins and this TV tell you shows here.", col_w, 4);
        let tile_f = Frame::new(sheet_x + s7 + ((col_w - 64.0) / 2.0).round(), place.y + below_head + s7, 64.0, 64.0);
        let label = Frame::new(sheet_x + s7, tile_f.bottom() + s3, col_w, label_line);
        let text_at = Frame::new(sheet_x + s7, label.bottom() + s3, col_w, text.len() as f32 * caption_line);
        let col_h = text_at.bottom() - tile_f.y;
        (Some(Empty { tile: tile_f, label, text, text_at }), s7 + col_h)
    } else {
        (None, s2 + content_h)
    };
    let sheet_h = SHEET_MIN_H.max(place.h.min(below_head + body_h + s4));
    let flick = Frame::new(sheet_x + s3, place.y + below_head + s2, flick_w, (sheet_h - (below_head + s2) - s4).max(1.0));
    Layout { sheet: Frame::new(sheet_x, place.y, SHEET_W, sheet_h), title_w, title_at, clear, flick, slots, content_h, empty }
}

/// The part of `f` that shows in `window`; nothing where none of it does.
fn clipped(f: Frame, window: Frame) -> Frame {
    let (x0, y0) = (f.x.max(window.x), f.y.max(window.y));
    let (x1, y1) = (f.right().min(window.right()), f.bottom().min(window.bottom()));
    if x1 > x0 && y1 > y0 { Frame::new(x0, y0, x1 - x0, y1 - y0) } else { Frame::new(0.0, 0.0, 0.0, 0.0) }
}

struct ItemParts {
    idx: usize,
    plate: Plate,
    _tile: ImageLayer,
    _title: Words,
    _text: Option<Words>,
    _when: Option<Words>,
    buttons: Vec<Button>,
    /// The slot's place in the column.
    top: f32,
    h: f32,
}

struct HeadParts {
    /// Its number among the rows (0: a label, which has none).
    idx: usize,
    plate: Option<Plate>,
    clear: Option<Button>,
    _icon: Option<ImageLayer>,
    _source: Words,
    _count: Option<Words>,
    _hint: Option<HintRow>,
    top: f32,
    h: f32,
}

struct Built {
    sig: String,
    pane: Pane,
    /// What the words, buttons and rows hang under: it comes in late, within the container's way out of the bell.
    _content: Group,
    _title: Words,
    clear: Option<Button>,
    scroll: Option<Scroll>,
    items: Vec<ItemParts>,
    heads: Vec<HeadParts>,
    _empty: Option<(ImageLayer, Words, Words)>,
    /// The entries made, from the first to the last (a part of the column near the window).
    range: (usize, usize),
    over_video: bool,
    lay: Layout,
    layers: usize,
}

pub struct DrawerSurface {
    stage: Stage,
    built: Option<Built>,
    /// Where the column is going (the focused item shows whole): kept across the column being made again.
    offset: f32,
    /// The lit item whose button has the focus.
    lit: Option<Id>,
}

fn need(lay: &Layout, from: f32, to: f32) -> (usize, usize) {
    let mut lo = usize::MAX;
    let mut hi = 0;
    for (i, s) in lay.slots.iter().enumerate() {
        if s.top() + s.height() >= from && s.top() <= to {
            lo = lo.min(i);
            hi = hi.max(i);
        }
    }
    if lo == usize::MAX { (0, 0) } else { (lo, hi) }
}

impl DrawerSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).springy(), built: None, offset: 0.0, lit: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    /// The sheet's place while it is built: what the mute mark keeps clear of.
    pub fn sheet(&self) -> Option<Frame> {
        self.built.as_ref().map(|b| b.lay.sheet)
    }

    /// Where the column goes for the focus: index 0 is the top, an item shows whole with `space(3)` of air beyond it.
    fn target(kit: &Kit, lay: &Layout, v: &View, offset: f32) -> f32 {
        let extent = (lay.content_h - lay.flick.h).max(0.0);
        let Some(slot) = (v.index > 0).then(|| lay.slots.iter().find(|s| s.idx() == Some(v.index))).flatten() else { return 0.0 };
        let (top, h) = (slot.top(), slot.height());
        let margin = kit.space(3);
        let want = if top < offset { top - margin } else if top + h > offset + lay.flick.h { top + h - lay.flick.h + margin } else { offset };
        want.clamp(0.0, extent)
    }

    fn signature(kit: &Kit, v: &View, lay: &Layout, range: (usize, usize)) -> String {
        format!("{}|{}|{:?}|{:?}|{:?}", kit.generation(), kit.text_scale, lay, range, v.entries.len())
    }

    #[allow(clippy::too_many_arguments)]
    fn build(kit: &mut Kit, engine: &mut Engine, root: &Root, z: i32, v: &View, lay: Layout, range: (usize, usize), offset: f32, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let (s2, s3, s4) = (kit.space(2), kit.space(3), kit.space(4));
        let (ink, ink2, ink3) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.ink3());
        let row_radius = kit.tokens.radius(Radius::Row);
        let btn_h = Button::height(kit);
        let content = root.content(kit)?;
        let pane = scene.adopting_view(&root.group, || Pane::new(kit, lay.sheet, z + 1, PaneKind::Sheet, v.over_video));
        let (title, clear) = scene.adopting_view(&content, || -> std::io::Result<_> {
            let title = Words::new(kit, lay.sheet.x + lay.title_at.0, lay.sheet.y + lay.title_at.1, lay.title_w + 2.0, z + 4, &[WordLine::new("Notifications", TypeRole::Title, ink).weight(Weight::SemiBold)])?;
            let clear = match lay.clear {
                Some(f) => {
                    let frame = Frame::new(lay.sheet.x + f.x, lay.sheet.y + f.y, f.w, f.h);
                    let mut b = Button::new(kit, engine, Place { id: Target::Clear.id(), frame, region: REGION_HEAD, scope: SCOPE, z: z + 4 }, Spec::new("Clear all", Tone::Plain).with_icon("check"))?;
                    b.show(kit, true);
                    Some(b)
                }
                None => None,
            };
            Ok((title, clear))
        })?;
        let flick = lay.flick;
        let mut items = Vec::new();
        let mut heads = Vec::new();
        let mut scroll = None;
        let mut empty = None;
        if let Some(e) = &lay.empty {
            empty = scene.adopting_view(&content, || -> std::io::Result<_> {
                let tile_layer = tile(kit, 64, "bell-off", false).layer(kit, z + 3, Props::at(e.tile.x, e.tile.y))?;
                let label = Words::new(kit, e.label.x, e.label.y, e.label.w, z + 4, &[WordLine::new("Nothing new", TypeRole::Label, ink).aligned(Align::Centre)])?;
                let lines: Vec<WordLine> = e.text.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2).aligned(Align::Centre)).collect();
                let text = Words::new(kit, e.text_at.x, e.text_at.y, e.text_at.w, z + 4, &lines)?;
                Ok(Some((tile_layer, label, text)))
            })?;
        } else {
            let tallest = lay.slots.iter().fold(0.0, |m: f32, s| if let Slot::Item { h, .. } = s { m.max(*h) } else { m });
            let sc = Scroll::new(kit, &content, flick, lay.content_h, Reach::lift(kit, Lift::Row, (flick.w, tallest)))?;
            let inner = &sc.inner;
            for (i, s) in lay.slots.iter().enumerate() {
                if i < range.0 || i > range.1 {
                    continue;
                }
                match s {
                    Slot::Head { top, h, source, icon: ic, count, count_w, source_w, stack } => {
                        let f = Frame::new(flick.x, flick.y + top, flick.w, *h);
                        let text_x = f.x + s4 + TILE + s4;
                        // a stacked group's header is a row of its own, in the first ink; a label is a caption in the second
                        let (role, ink_of) = if stack.is_some() { (TypeRole::Label, ink) } else { (TypeRole::Caption, ink2) };
                        let band = match stack {
                            Some(st) if st.two_rows => f.y + s2 + ((28.0f32.max(kit.line(TypeRole::Label)) - 28.0) / 2.0).round(),
                            Some(_) => f.y + ((f.h - 28.0) / 2.0).round(),
                            None => f.y + f.h - s2 - 28.0,
                        };
                        let plate = match stack {
                            Some(st) => Some(Plate::new(kit, engine, inner, Spot { id: Target::Head(st.idx).id(), screen: clipped(Frame::new(f.x, f.y - offset, f.w, f.h), flick), local: f, region: REGION_LIST, scope: SCOPE, z: z + 2 }, Lift::Row, row_radius)?),
                            None => None,
                        };
                        let (icon_layer, src, cnt, hint) = scene.adopting(inner, || -> std::io::Result<_> {
                            let icon_layer = icon::layer(kit, ic, 24, ink3, z + 3, Props::at(f.x + s4 + ((TILE - 24.0) / 2.0).round(), band + 2.0))?;
                            let ty = |kit: &Kit, role: TypeRole| (band + ((28.0 - kit.line(role)) / 2.0).round()).round();
                            let src = Words::new(kit, text_x, ty(kit, role), source_w + 2.0, z + 4, &[WordLine::new(source, role, ink_of)])?;
                            let count_x = text_x + source_w + s2;
                            let cnt = if count.is_empty() { None } else { Some(Words::new(kit, count_x, ty(kit, TypeRole::Caption), count_w + 2.0, z + 4, &[WordLine::new(count, TypeRole::Caption, ink2)])?) };
                            let hint = match stack.as_ref().and_then(|s| s.hint.as_ref()) {
                                Some((hint, _)) => {
                                    let (y, h) = if stack.as_ref().is_some_and(|s| s.two_rows) { (f.y + s2, 28.0f32.max(kit.line(TypeRole::Label))) } else { (f.y, f.h) };
                                    Some(HintRow::new(kit, hint, TypeRole::Caption, RowAt { x: count_x + count_w + s3, y, h, z: z + 3 }, ink2)?)
                                }
                                None => None,
                            };
                            Ok((icon_layer, src, cnt, hint))
                        })?;
                        let clear = match stack {
                            Some(st) => {
                                let frame = if st.two_rows {
                                    Frame::new(text_x, f.y + s2 + 28.0f32.max(kit.line(TypeRole::Label)) + s2, st.clear_w, btn_h)
                                } else {
                                    Frame::new(f.x + f.w - s4 - st.clear_w, f.y + s2, st.clear_w, btn_h)
                                };
                                let bid = Target::HeadClear(st.idx).id();
                                let mut btn = scene.adopting_view(inner, || Button::new(kit, engine, Place { id: bid, frame, region: REGION_LIST, scope: SCOPE, z: z + 4 }, Spec::new("Clear", Tone::Plain)))?;
                                btn.show(kit, true);
                                engine.focus.place(bid, clipped(Frame::new(frame.x, frame.y - offset, frame.w, frame.h), flick));
                                Some(btn)
                            }
                            None => None,
                        };
                        heads.push(HeadParts { idx: stack.as_ref().map_or(0, |s| s.idx), plate, clear, _icon: icon_layer, _source: src, _count: cnt, _hint: hint, top: *top, h: *h });
                    }
                    Slot::Item { top, h, item, stamp, title: shown, title_w, lines: under, text_w, when_w, buttons } => {
                        let f = Frame::new(flick.x, flick.y + top, flick.w, *h);
                        let id = Target::Item(item.idx).id();
                        let plate = Plate::new(kit, engine, inner, Spot { id, screen: clipped(Frame::new(f.x, f.y - offset, f.w, f.h), flick), local: f, region: REGION_LIST, scope: SCOPE, z: z + 2 }, Lift::Row, row_radius)?;
                        let text_x = f.x + s4 + TILE + s4;
                        let label_line = kit.line(TypeRole::Label);
                        let (tile_layer, title, text, when) = scene.adopting(inner, || -> std::io::Result<_> {
                            let tile_layer = tile(kit, TILE as u32, &item.icon, false).layer(kit, z + 3, Props::at(f.x + s4, f.y + s3))?;
                            let title = Words::new(kit, text_x, f.y + s3, title_w + 2.0, z + 4, &[WordLine::new(shown, TypeRole::Label, ink)])?;
                            let text = if under.is_empty() {
                                None
                            } else {
                                let lines: Vec<WordLine> = under.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2)).collect();
                                Some(Words::new(kit, text_x, f.y + s3 + label_line, *text_w, z + 4, &lines)?)
                            };
                            let when = if *when_w > 0.0 { Some(Words::new(kit, f.x + f.w - when_w - s4, f.y + s3, when_w + 2.0, z + 4, &[WordLine::new(stamp, TypeRole::Caption, ink2).aligned(Align::Right)])?) } else { None };
                            Ok((tile_layer, title, text, when))
                        })?;
                        let mut bs = Vec::new();
                        let mut x = text_x;
                        for (b, w) in buttons.iter().enumerate() {
                            let frame = Frame::new(x, f.y + f.h - s3 - btn_h, *w, btn_h);
                            let bid = Target::Button(item.idx, b).id();
                            let mut btn = scene.adopting_view(inner, || Button::new(kit, engine, Place { id: bid, frame, region: REGION_LIST, scope: SCOPE, z: z + 4 }, Spec::new(&item.buttons[b], Tone::Plain)))?;
                            btn.show(kit, true);
                            engine.focus.place(bid, clipped(Frame::new(frame.x, frame.y - offset, frame.w, frame.h), flick));
                            bs.push(btn);
                            x += w + s3;
                        }
                        items.push(ItemParts { idx: item.idx, plate, _tile: tile_layer, _title: title, _text: text, _when: when, buttons: bs, top: *top, h: *h });
                    }
                }
            }
            scroll = Some(sc);
        }
        let layers = before.saturating_sub(kit.room().layers);
        Ok(Built { sig, pane, _content: content, _title: title, clear, scroll, items, heads, _empty: empty, range, over_video: v.over_video, lay, layers })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        self.lit = None;
        if let Some(b) = self.built.take() {
            if let Some(c) = b.clear {
                c.destroy(engine);
            }
            for hd in b.heads {
                if let Some(p) = hd.plate {
                    p.destroy(engine);
                }
                if let Some(btn) = hd.clear {
                    btn.destroy(engine);
                }
            }
            for it in b.items {
                it.plate.destroy(engine);
                for btn in it.buttons {
                    btn.destroy(engine);
                }
            }
        }
    }

    /// Fits the drawer to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, bell: [f32; 4], now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        // under the bell: its middle under the bell's, its top just under the pill, kept off the screen's edges
        let lay = layout(kit, v, quick::dropdown(bell, SHEET_W, quick::SHEET_H, kit.space(2)));
        // The drawer grows out of the bell and goes back into it. While it is on its way in or out it keeps the rectangles it started with (a
        // change of the sheet's height as the audit arrives must not restart the motion); at rest and parked it follows the sheet, and a
        // parked panel that opens again is made afresh at the start of its motion.
        let to = [lay.sheet.x, lay.sheet.y, lay.sheet.w, lay.sheet.h];
        let wanted = Arrive::Container { from: bell, to };
        let arrive = match (phase, self.stage.arrive()) {
            (Phase::Leaving, Some(held)) => held,
            (Phase::Opening, Some(held)) if self.stage.root().is_some_and(Root::is_shown) => held,
            _ => wanted,
        };
        if self.stage.needs_root(arrive) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, arrive, true)?;
        let up = matches!(phase, Phase::Opening | Phase::Open);
        // where the column goes, and which entries have to be there to show it
        let want = if up { Self::target(kit, &lay, v, self.offset) } else { self.offset };
        let extent = (lay.content_h - lay.flick.h).max(0.0);
        let want = want.min(extent);
        let held = self.built.as_ref().filter(|b| b.lay == lay).map(|b| b.range);
        let showing = need(&lay, want, want + lay.flick.h);
        let range = match held {
            Some(r) if r.0 <= showing.0 && r.1 >= showing.1 => r,
            _ => need(&lay, (want.min(self.offset)) - lay.flick.h / 2.0, want.max(self.offset) + lay.flick.h * 1.5),
        };
        let sig = Self::signature(kit, v, &lay, range);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION_LIST);
            let built = Self::build(kit, engine, self.stage.root().expect("a root"), self.stage.z(), v, lay, range, self.offset, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the notification drawer could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, phase) {
            self.update(kit, engine, v, want, now_ns);
        }
        Ok(true)
    }

    fn update(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, want: f32, now_ns: u64) {
        if let Some(b) = self.built.as_mut()
            && b.over_video != v.over_video
        {
            b.pane.restyle(kit, v.over_video);
            b.over_video = v.over_video;
        }
        // the focus: a row with buttons is lit as a whole, and the focus flies between its buttons (a header's Clear is one)
        let row = (v.index > 0).then(|| v.entries.iter().find(|e| matches!(e, EntryView::Item(i) if i.idx == v.index) || matches!(e, EntryView::Head { idx, .. } if *idx == v.index))).flatten();
        let (focus, lit) = match row {
            Some(EntryView::Item(i)) if !i.buttons.is_empty() => (Some(Target::Button(i.idx, v.button.min(i.buttons.len() - 1)).id()), Some(Target::Item(i.idx).id())),
            Some(EntryView::Item(i)) => (Some(Target::Item(i.idx).id()), None),
            Some(EntryView::Head { idx, .. }) if v.button > 0 => (Some(Target::HeadClear(*idx).id()), Some(Target::Head(*idx).id())),
            Some(EntryView::Head { idx, .. }) => (Some(Target::Head(*idx).id()), None),
            None => (self.built.as_ref().and_then(|b| b.clear.as_ref()).map(|_| Target::Clear.id()), None),
        };
        self.stage.focus(engine, focus, now_ns);
        if self.lit != lit {
            let _ = engine.flow.go(self.lit, lit, now_ns);
            self.lit = lit;
        }
        // the column moves on the focus's spring; the frames the pointer hits are where the items will be
        if (want - self.offset).abs() > 0.5
            && let Some(b) = self.built.as_mut()
            && let Some(sc) = b.scroll.as_mut()
        {
            sc.to(engine, want, false);
            self.offset = want;
            Self::place(b, engine, want);
        }
    }

    /// Puts the registry's frames where the items are at `offset`: what shows of them.
    fn place(b: &Built, engine: &mut Engine, offset: f32) {
        let flick = b.lay.flick;
        for hd in &b.heads {
            if let Some(p) = &hd.plate {
                engine.focus.place(p.id, clipped(Frame::new(flick.x, flick.y + hd.top - offset, flick.w, hd.h), flick));
            }
            if let Some(btn) = &hd.clear {
                let bf = btn.frame();
                engine.focus.place(btn.id, clipped(Frame::new(bf.x, bf.y - offset, bf.w, bf.h), flick));
            }
        }
        for it in &b.items {
            let f = Frame::new(flick.x, flick.y + it.top - offset, flick.w, it.h);
            engine.focus.place(it.plate.id, clipped(f, flick));
            for btn in &it.buttons {
                let bf = btn.frame();
                engine.focus.place(btn.id, clipped(Frame::new(bf.x, bf.y - offset, bf.w, bf.h), flick));
            }
        }
    }

    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.stage.key(engine);
        if let Some(b) = self.built.as_mut() {
            if let Some(c) = b.clear.as_mut() {
                c.set_hover(kit, false);
            }
            for hd in &mut b.heads {
                if let Some(btn) = hd.clear.as_mut() {
                    btn.set_hover(kit, false);
                }
            }
            for it in &mut b.items {
                for btn in &mut it.buttons {
                    btn.set_hover(kit, false);
                }
            }
        }
    }

    /// A sample of the pointer: the cue follows it (buttons light themselves), a press shrinks a button, and a click is the target it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let radius = kit.tokens.radius(Radius::Row);
        let built = &mut self.built;
        let current = engine.focus.current();
        let mut set = |id: Id, f: &dyn Fn(&mut Button)| {
            let Some(b) = built.as_mut() else { return };
            match Target::from_id(id) {
                Some(Target::Clear) => {
                    if let Some(c) = b.clear.as_mut() {
                        f(c);
                    }
                }
                Some(Target::Button(i, k)) => {
                    if let Some(btn) = b.items.iter_mut().find(|it| it.idx == i).and_then(|it| it.buttons.get_mut(k)) {
                        f(btn);
                    }
                }
                Some(Target::HeadClear(i)) => {
                    if let Some(btn) = b.heads.iter_mut().find(|h| h.idx == i).and_then(|h| h.clear.as_mut()) {
                        f(btn);
                    }
                }
                _ => {}
            }
        };
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { from, to } => {
                if let Some(f) = from {
                    set(f, &|b| b.set_hover(kit, false));
                }
                if let Some(t) = to.filter(|t| current != Some(*t)) {
                    set(t, &|b| b.set_hover(kit, true));
                }
                to.and_then(Target::from_id).filter(|t| matches!(t, Target::Item(_) | Target::Head(_))).map(|_| radius)
            }
            Touch::Press { id, down } => {
                set(id, &|b| b.set_pressed(kit, down));
                None
            }
        })?;
        Target::from_id(click)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
        self.offset = 0.0;
    }

    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers + 2)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let items: Vec<Value> = b
            .items
            .iter()
            .map(|it| json!({"idx": it.idx, "frame": engine.focus.frame(it.plate.id).map(f), "buttons": it.buttons.iter().map(|x| engine.focus.frame(x.id).map(f)).collect::<Vec<_>>()}))
            .collect();
        let heads: Vec<Value> = b
            .heads
            .iter()
            .filter(|h| h.idx > 0)
            .map(|h| json!({"idx": h.idx, "frame": h.plate.as_ref().and_then(|p| engine.focus.frame(p.id)).map(f), "clear": h.clear.as_ref().and_then(|c| engine.focus.frame(c.id)).map(f), "hint": h._hint.is_some()}))
            .collect();
        json!({
            "sheet": f(b.lay.sheet),
            "window": f(b.lay.flick),
            "clear": b.clear.as_ref().map(|c| f(c.frame())),
            "items": items,
            "heads": heads,
            "range": [b.range.0, b.range.1],
            "entries": b.lay.slots.len(),
            "offset": self.offset,
            "empty": b.lay.empty.is_some(),
            "focus": engine.focus.current().map(|i| i.0),
            "layers": self.layers(),
            "scope": SCOPE.0,
            "origin": self.stage.arrive().and_then(|a| a.origin()),
        })
    }

    /// Describes what is on the screen to a lint: the sheet and the words on it; of the column, the items that show whole.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = lint.panel("drawer sheet", b.lay.sheet, kit.ui.look.colour(PaletteRole::Sheet), false);
        b._title.feed_lint(kit, lint, "drawer title", Some(sheet), (0.0, 0.0));
        if let Some(c) = &b.clear {
            c.feed_lint(kit, lint, sheet);
        }
        if let Some((_, label, text)) = &b._empty {
            label.feed_lint(kit, lint, "empty label", Some(sheet), (0.0, 0.0));
            text.feed_lint(kit, lint, "empty text", Some(sheet), (0.0, 0.0));
        }
        let (flick, offset) = (b.lay.flick, self.offset);
        let shown = |top: f32, h: f32| top - offset >= -0.5 && top - offset + h <= flick.h + 0.5;
        for it in &b.items {
            if !shown(it.top, it.h) {
                continue;
            }
            it._title.feed_lint(kit, lint, &format!("item {}", it.idx), Some(sheet), (0.0, -offset));
            if let Some(w) = &it._text {
                w.feed_lint(kit, lint, &format!("item {} text", it.idx), Some(sheet), (0.0, -offset));
            }
            if let Some(w) = &it._when {
                w.feed_lint(kit, lint, &format!("item {} time", it.idx), Some(sheet), (0.0, -offset));
            }
            for btn in &it.buttons {
                btn.feed_lint_at(kit, lint, sheet, (0.0, -offset));
            }
        }
        for h in &b.heads {
            if shown(h.top, h.h) {
                h._source.feed_lint(kit, lint, "group", Some(sheet), (0.0, -offset));
                if let Some(c) = &h._count {
                    c.feed_lint(kit, lint, "group count", Some(sheet), (0.0, -offset));
                }
                if let Some(btn) = &h.clear {
                    btn.feed_lint_at(kit, lint, sheet, (0.0, -offset));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::life::SPRING_MS;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::drawer::Drawer;
    use crate::panels::drawer::tests::{NOW, env, open};
    use crate::panels::env::Env;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: DrawerSurface,
        d: Drawer,
        env: Env,
        t: u64,
    }

    /// The lock saying `n` different things (one group of many), beside what `env()` has.
    fn crowded_env(n: usize) -> Env {
        let mut env = env();
        let at = (NOW / 1000 - 600) as f64;
        let items: Vec<Value> = (0..n)
            .map(|i| json!({"id": format!("n{i}"), "channel": "events", "revision": 1, "receivedAt": at + i as f64, "post": {"title": format!("Event number {i}"), "body": "Something happened at the front door, said at a length that needs a second line on a sheet this wide.", "actions": []}}))
            .collect();
        env.update(&Input::Helper { topic: "plugin.lock.notifications".into(), data: json!({"items": items}) }, 0);
        env
    }

    impl Up {
        fn new(mode: Mode, scale: f32) -> Self {
            Self::with(env(), mode, scale)
        }

        fn with(env: Env, mode: Mode, scale: f32) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let d = open(&env);
            let mut me = Self { _h: h, kit, engine, s: DrawerSurface::new(crate::kit::z::PANELS), d, env, t: 1000 };
            me.apply();
            me
        }

        /// Opens every stacked group (last first, so the rows above keep their numbers).
        fn open_all(&mut self) {
            let v = self.d.view(&self.env, NOW);
            let heads: Vec<usize> = v.entries.iter().filter_map(|e| if let EntryView::Head { idx, .. } = e { Some(*idx).filter(|i| *i > 0) } else { None }).collect();
            for idx in heads.into_iter().rev() {
                self.d.click_head(idx, &self.env);
            }
            self.t += 40;
            self.apply();
        }

        fn empty(mode: Mode, scale: f32) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let env = Env::new(json!({}));
            let mut d = Drawer::new();
            d.life.show(0);
            d.open(&env, NOW);
            let mut me = Self { _h: h, kit, engine, s: DrawerSurface::new(crate::kit::z::PANELS), d, env, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            self.apply_at(BELL);
        }

        fn apply_at(&mut self, bell: [f32; 4]) {
            let v = self.d.view(&self.env, NOW);
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.d.life.phase(), bell, self.t * 1_000_000).expect("the drawer is drawn");
        }

        fn sheet(&self) -> [f32; 4] {
            let s = self.s.built.as_ref().expect("built").lay.sheet;
            [s.x, s.y, s.w, s.h]
        }

        fn key(&mut self, k: K) {
            self.d.key(k, &self.env);
            self.t += 40;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Clear, Target::Backdrop, Target::Item(1), Target::Item(300), Target::Button(1, 0), Target::Button(40, 3), Target::Head(2), Target::Head(300), Target::HeadClear(2)] {
            assert_eq!(Target::from_id(t.id()), Some(t), "{t:?}");
        }
        assert_eq!(Target::from_id(Id(5)), None);
    }

    #[test]
    fn the_sheet_is_the_quick_panels_560_wide_40_from_the_right_edge_from_the_bars_line_and_as_tall_as_what_it_holds() {
        let u = Up::new(Mode::Dark, 1.0);
        let g = u.s.geometry(&u.engine);
        let q = quick::dropdown(BELL, quick::SHEET_W, quick::SHEET_H, u.kit.space(2));
        assert_eq!((q.x, q.w), (1320.0, 560.0), "under the bell, against the right margin");
        assert_eq!((g["sheet"]["x"].as_f64().unwrap() as f32, g["sheet"]["y"].as_f64().unwrap() as f32, g["sheet"]["w"].as_f64().unwrap() as f32), (q.x, q.y, q.w), "just under the pill, in the bell's column");
        assert_eq!(g["sheet"]["x"].as_f64().unwrap() + g["sheet"]["w"].as_f64().unwrap(), 1880.0, "40 from the right edge");
        let h = g["sheet"]["h"].as_f64().unwrap();
        assert!((240.0..=f64::from(q.h)).contains(&h), "{h}");
        assert_eq!(g["empty"], false);
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        let e = Up::empty(Mode::Dark, 1.0);
        let empty = e.s.geometry(&e.engine);
        assert_eq!(empty["empty"], true);
        assert!(empty["sheet"]["h"].as_f64().unwrap() < h, "the empty state is a shorter sheet");
        assert_eq!(empty["sheet"]["x"], 1320.0);
    }

    #[test]
    fn the_sheet_grows_with_its_notices_and_stops_at_the_quick_panels_full_height() {
        let mut u = Up::with(crowded_env(2), Mode::Dark, 1.0);
        let short = u.s.geometry(&u.engine)["sheet"]["h"].as_f64().unwrap();
        assert!(short < quick::SHEET_H as f64, "a few notices do not fill the sheet: {short}");
        u.open_all();
        let open = u.s.geometry(&u.engine)["sheet"]["h"].as_f64().unwrap();
        assert!(open > short, "opening a group grows the sheet: {short} to {open}");
        let mut many = Up::with(crowded_env(30), Mode::Dark, 1.0);
        many.open_all();
        let g = many.s.geometry(&many.engine);
        let full = quick::dropdown(BELL, quick::SHEET_W, quick::SHEET_H, many.kit.space(2));
        assert_eq!(g["sheet"]["h"], f64::from(full.h), "many notices fill the room under the pill and no more");
        assert_eq!(g["sheet"]["y"], f64::from(full.y));
        let (w, s) = (many.s.built.as_ref().unwrap().lay.flick, many.s.built.as_ref().unwrap().lay.sheet);
        assert!(w.bottom() <= s.bottom() && w.y >= s.y, "the column stays on the sheet: {w:?} in {s:?}");
        let mut at200 = Up::with(crowded_env(30), Mode::Dark, 2.0);
        at200.open_all();
        assert_eq!(at200.s.geometry(&at200.engine)["sheet"]["h"], f64::from(quick::dropdown(BELL, quick::SHEET_W, quick::SHEET_H, at200.kit.space(2)).h), "and at 200 % text");
    }

    #[test]
    fn the_focus_starts_on_the_first_row_and_follows_the_keys_between_headers_items_and_their_buttons() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let focus = |u: &Up| u.engine.focus.current().and_then(Target::from_id);
        assert_eq!(focus(&u), Some(Target::Item(1)), "the TV's item has no buttons");
        u.key(K::Up);
        assert_eq!(u.engine.focus.current(), Some(Target::Clear.id()), "Up from the first row is Clear all");
        u.key(K::Down);
        u.key(K::Down);
        assert_eq!(focus(&u), Some(Target::Head(2)), "the assistant's group header");
        assert!(u.engine.flow.settling_at(u.t * 1_000_000), "the lift flies from one to the next");
        assert_eq!(u.s.lit, None);
        u.key(K::Right);
        assert_eq!(focus(&u), Some(Target::HeadClear(2)), "Right reaches the group's Clear");
        assert_eq!(u.s.lit, Some(Target::Head(2).id()), "the header stays lit while its Clear has the focus");
        u.key(K::Left);
        assert_eq!((focus(&u), u.s.lit), (Some(Target::Head(2)), None));
        u.key(K::Down);
        assert_eq!(focus(&u), Some(Target::Button(3, 0)), "the newest of the group is Resume");
        assert_eq!(u.s.lit, Some(Target::Item(3).id()));
    }

    #[test]
    fn ok_on_a_header_shows_the_rest_of_its_group_and_its_hint_says_what_ok_does() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["heads"].as_array().unwrap().len(), 2, "two stacked groups");
        assert_eq!(g["heads"][0]["hint"], true, "OK and expand at 100 %");
        let before = g["entries"].as_u64().unwrap();
        u.key(K::Down);
        u.key(K::Ok);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["entries"].as_u64().unwrap(), before + 1, "the group's second notification is there");
        assert_eq!(u.engine.focus.current().and_then(Target::from_id), Some(Target::Head(2)), "the focus stays on the header");
        let built = u.s.built.as_ref().unwrap();
        let hint = built.lay.slots.iter().find_map(|s| if let Slot::Head { stack: Some(st), .. } = s { st.hint.clone() } else { None }).expect("a hint").0;
        assert_eq!(hint.glyphs(), vec!["key-ok", "collapse"]);
        u.key(K::Ok);
        assert_eq!(u.s.geometry(&u.engine)["entries"].as_u64().unwrap(), before, "OK again folds it");
    }

    #[test]
    fn the_count_of_what_said_the_same_is_on_the_entry() {
        let mut env = env();
        let at = (NOW / 1000 - 600) as f64;
        let same = |id: &str, t: f64| json!({"id": id, "channel": "events", "revision": 1, "receivedAt": t, "post": {"title": "Battery low", "body": "12 %", "actions": []}});
        env.update(&Input::Helper { topic: "plugin.lock.notifications".into(), data: json!({"items": [same("a", at), same("b", at + 1.0), same("c", at + 2.0)]}) }, 0);
        let u = Up::with(env, Mode::Dark, 1.0);
        let stamps: Vec<String> = u.s.built.as_ref().unwrap().lay.slots.iter().filter_map(|s| if let Slot::Item { stamp, item, .. } = s { Some(format!("{}:{stamp}", item.count)) } else { None }).collect();
        assert!(stamps.iter().any(|s| s.starts_with("3:×3 · ")), "{stamps:?}");
        assert!(stamps.iter().any(|s| s.starts_with("1:") && !s.contains('×')), "{stamps:?}");
    }

    #[test]
    fn where_the_name_would_be_cut_beside_the_clear_the_clear_is_a_row_under_it_and_the_time_goes_under_a_title_that_leaves_it_no_room() {
        let small = Up::new(Mode::Dark, 1.0);
        let big = Up::new(Mode::Dark, 2.0);
        let two_rows = |u: &Up| u.s.built.as_ref().unwrap().lay.slots.iter().filter_map(|s| if let Slot::Head { stack: Some(st), .. } = s { Some(st.two_rows) } else { None }).collect::<Vec<_>>();
        assert_eq!(two_rows(&small), vec![false, false], "beside the words at 100 %");
        assert_eq!(two_rows(&big), vec![true, true], "under them at 200 %");
        let beside = |u: &Up| u.s.built.as_ref().unwrap().lay.slots.iter().filter_map(|s| if let Slot::Item { when_w, .. } = s { Some(*when_w > 0.0) } else { None }).collect::<Vec<_>>();
        assert!(beside(&small).iter().all(|b| *b), "the time is at the right of each title at 100 %: {:?}", beside(&small));
        assert!(beside(&big).iter().any(|b| !*b), "and under a title at 200 %, where it leaves no room");
        let full = |u: &Up| u.s.built.as_ref().unwrap().lay.slots.iter().filter_map(|s| if let Slot::Item { title, item, .. } = s { Some(title.ends_with('…') && !item.title.ends_with('…')) } else { None }).any(|cut| cut);
        assert!(!full(&big), "no title is cut at 200 % for a time");
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_down_the_whole_list_open_and_empty() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
                for step in 0..10 {
                    u.key(K::Down);
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} after {step} downs: {f:#?}");
                }
                u.open_all();
                u.d.click_head(2, &u.env);
                u.d.click_head(2, &u.env);
                for step in 0..12 {
                    u.key(K::Down);
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} open, after {step} downs: {f:#?}");
                }
                let mut c = Up::with(crowded_env(12), mode, scale);
                c.open_all();
                for step in 0..20 {
                    c.key(K::Down);
                    let f = c.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} crowded, after {step} downs: {f:#?}");
                }
                let mut e = Up::empty(mode, scale);
                let f = e.lint();
                assert!(f.is_empty(), "{mode:?} {scale} empty: {f:#?}");
            }
        }
    }

    #[test]
    fn the_column_moves_to_show_the_focused_row_and_only_the_rows_near_the_window_are_made() {
        let mut u = Up::with(crowded_env(14), Mode::Dark, 2.0);
        u.open_all();
        let rows = u.d.view(&u.env, NOW).rows;
        let all = u.s.geometry(&u.engine)["entries"].as_u64().unwrap();
        for _ in 0..=rows {
            u.key(K::Down);
        }
        let g = u.s.geometry(&u.engine);
        assert!(g["offset"].as_f64().unwrap() > 0.0, "the column moved: {g}");
        let r = &g["range"];
        assert!(r[1].as_u64().unwrap() - r[0].as_u64().unwrap() < all, "only part of the column is made at 200%: {r} of {all}");
        let f = u.engine.focus.frame(u.engine.focus.current().expect("a focus")).expect("a frame");
        let w = u.s.built.as_ref().unwrap().lay.flick;
        assert!(f.y >= w.y - 0.5 && f.bottom() <= w.bottom() + 0.5, "the focused row shows: {f:?} in {w:?}");
    }

    #[test]
    fn clicks_on_clear_all_a_header_its_clear_an_item_a_button_and_the_dim_name_their_targets() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
        let mut n = 0u64;
        let mut click = |u: &mut Up, x: f32, y: f32| {
            n += 100;
            let t = (5000 + n) * 1_000_000;
            u.s.pointer(&u.kit, &mut u.engine, t, at(x, y, 0));
            u.s.pointer(&u.kit, &mut u.engine, t + 1_000_000, at(x, y, 1));
            u.s.pointer(&u.kit, &mut u.engine, t + 2_000_000, at(x, y, 0))
        };
        let clear = u.engine.focus.frame(Target::Clear.id()).expect("clear all");
        assert_eq!(click(&mut u, clear.x + 10.0, clear.y + 10.0), Some(Target::Clear));
        let item = u.engine.focus.frame(Target::Item(1).id()).expect("an item");
        let t = click(&mut u, item.x + 10.0, item.y + 6.0);
        assert!(matches!(t, Some(Target::Item(1))), "{t:?}");
        let head = u.engine.focus.frame(Target::Head(2).id()).expect("a header");
        assert_eq!(click(&mut u, head.x + 10.0, head.y + 6.0), Some(Target::Head(2)));
        let hc = u.engine.focus.frame(Target::HeadClear(2).id()).expect("a header's clear");
        assert_eq!(click(&mut u, hc.x + 10.0, hc.y + 10.0), Some(Target::HeadClear(2)));
        assert_eq!(click(&mut u, 300.0, 300.0), Some(Target::Backdrop));
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let with = u.kit.room().layers;
        u.d.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.d.life.tick(2000 + u64::from(SPRING_MS));
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.d.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 20 && !u.s.is_up());
    }

    #[test]
    fn it_grows_out_of_the_bell_into_the_sheet_and_goes_back_into_the_bell() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let first = u.s.stage.arrive();
        assert_eq!(first, Some(Arrive::Container { from: BELL, to: u.sheet() }), "out of the bell into the sheet, not in from the edge");
        // opening: the sheet grows as a group opens, and the motion under way keeps the rectangles it started from
        u.key(K::Down);
        u.key(K::Ok);
        assert_ne!(u.sheet(), match first { Some(Arrive::Container { to, .. }) => to, _ => unreachable!() }, "the sheet is taller now");
        assert_eq!(u.s.stage.arrive(), first, "a motion under way is not restarted by it");
        // at rest it follows the sheet, and where the bell is
        u.d.life.tick(u64::from(SPRING_MS) + 1);
        let moved = [1700.0, 70.0, 56.0, 56.0];
        u.apply_at(moved);
        assert_eq!(u.s.stage.arrive(), Some(Arrive::Container { from: moved, to: u.sheet() }));
        // going: back into the bell it was at when it left, whatever the bell does meanwhile
        let leaving = u.s.stage.arrive();
        u.d.life.hide(1000);
        u.apply_at(BELL);
        assert_eq!(u.s.stage.arrive(), leaving);
        assert!(!u.s.stage.root().expect("a root").is_shown(), "the root is on its way back");
        // parked and opened again: made afresh at the start of its motion, from the bell where it is now
        u.d.life.tick(1000 + u64::from(SPRING_MS));
        u.apply_at(BELL);
        u.d.life.show(5000);
        u.apply_at(BELL);
        assert_eq!(u.s.stage.arrive(), Some(Arrive::Container { from: BELL, to: u.sheet() }));
        assert!(u.s.stage.root().expect("a root").is_shown());
    }

    #[test]
    fn the_content_comes_in_late_under_a_container_and_with_the_root_under_reduced_motion() {
        let u = Up::new(Mode::Dark, 1.0);
        assert!(u.s.stage.root().expect("a root").is_container(), "the glass grows first and the words come in as it settles");
        let (h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        kit.ui.look.reduced_motion = true;
        let engine = Engine::new(&kit).expect("an engine");
        let env = env();
        let d = open(&env);
        let mut r = Up { _h: h, kit, engine, s: DrawerSurface::new(crate::kit::z::PANELS), d, env, t: 1000 };
        r.apply();
        assert!(!r.s.stage.root().expect("a root").is_container(), "reduced motion is a quick fade of the whole");
    }
}
