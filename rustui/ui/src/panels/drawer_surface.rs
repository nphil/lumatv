//! The notification drawer drawn (`docs/mocks/13-notifications`): a glass sheet down the right side from the top bar's line, a head with
//! "Notifications" and "Clear all", and one column of what has happened grouped by who said it: a header per group, then its items, each
//! with its icon tile, its title, up to three lines of what it says, the time and the buttons it offers. With nothing to list the sheet
//! is the empty state. The sheet is as tall as what it holds, up to the screen less the bar's margins.
//!
//! The model decides which item (and which of its buttons) has the focus. An item with buttons is lit as a whole while one of its
//! buttons has the focus, which flies from button to button; an item without buttons is the focus itself. The column moves on the focus's
//! spring to show the focused item, and only the items near the window are made (the scene has a budget), made again when the window
//! comes near others.

use super::drawer::{EntryView, ItemView, View};
use super::life::Phase;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, Scroll, WordLine, Words};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Spot, tile};
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
const SHEET_W: f32 = 640.0;
const SHEET_Y: f32 = 56.0;
const SHEET_MIN_H: f32 = 240.0;
const TILE: f32 = 48.0;

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Clear,
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
            })
    }

    pub fn from_id(id: Id) -> Option<Target> {
        let n = id.0.checked_sub(BASE)?;
        match n {
            1 => Some(Target::Clear),
            2 => Some(Target::Backdrop),
            0x1000..0x10_0000 => Some(Target::Item((n - 0x1000) as usize)),
            0x10_0000.. if n < 0x1000_0000 => Some(Target::Button(((n - 0x10_0000) / 16) as usize, ((n - 0x10_0000) % 16) as usize)),
            _ => None,
        }
    }
}

/// One entry of the column laid out: its place in the column (from its top) and what its parts say.
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Head { top: f32, h: f32, source: String, icon: String, count: String, count_w: f32 },
    Item { top: f32, h: f32, item: ItemView, title: String, body: Vec<String>, text_w: f32, when_w: f32, buttons: Vec<f32> },
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

fn layout(kit: &mut Kit, v: &View) -> Layout {
    let (s1, s2, s3, s4, s5, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(7));
    let ink = kit.ui.look.ink();
    let ink2 = kit.ui.look.ink2();
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let (title_line, label_line, caption_line) = (kit.line(TypeRole::Title), kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
    let sheet_x = 1920.0 - 64.0 - SHEET_W;
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
    // the column
    let flick_w = SHEET_W - 2.0 * s3;
    let text_x = s4 + TILE + s4;
    let mut slots = Vec::new();
    let mut top = 0.0;
    for e in &v.entries {
        match e {
            EntryView::Head { source, icon, count } => {
                let h = (kit.tokens.row * 0.6).round();
                let count = count.to_string();
                let count_w = width_of(kit, caption_style, &count).ceil();
                slots.push(Slot::Head { top, h, source: source.clone(), icon: icon.clone(), count, count_w });
                top += h;
            }
            EntryView::Item(it) => {
                let when_w = if it.when.is_empty() { 0.0 } else { width_of(kit, caption_style, &it.when).ceil() };
                let text_w = (flick_w - text_x - if when_w > 0.0 { when_w + s3 } else { 0.0 } - s4).max(1.0);
                let title = wrap::elide(kit, label_style, &it.title, text_w);
                let body = if it.body.is_empty() { Vec::new() } else { wrap::lines(kit, caption_style, &it.body, text_w, 3) };
                let text_h = label_line + body.len() as f32 * caption_line;
                let buttons: Vec<f32> = it.buttons.iter().map(|b| Button::width_for(kit, b, false)).collect();
                let h = s3 + TILE.max(text_h) + if buttons.is_empty() { 0.0 } else { s3 + btn_h } + s3;
                slots.push(Slot::Item { top, h, item: it.clone(), title, body, text_w, when_w, buttons });
                top += h + s1;
            }
        }
    }
    let content_h = top;
    let (empty, body_h) = if v.count == 0 {
        let col_w = SHEET_W - 2.0 * s7;
        let text = wrap::lines(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Centre), "What your assistants, plugins and this TV tell you shows here.", col_w, 4);
        let tile_f = Frame::new(sheet_x + s7 + ((col_w - 64.0) / 2.0).round(), SHEET_Y + below_head + s7, 64.0, 64.0);
        let label = Frame::new(sheet_x + s7, tile_f.bottom() + s3, col_w, label_line);
        let text_at = Frame::new(sheet_x + s7, label.bottom() + s3, col_w, text.len() as f32 * caption_line);
        let col_h = text_at.bottom() - tile_f.y;
        (Some(Empty { tile: tile_f, label, text, text_at }), s7 + col_h)
    } else {
        (None, s2 + content_h)
    };
    let sheet_h = SHEET_MIN_H.max((1080.0 - 2.0 * SHEET_Y).min(below_head + body_h + s4));
    let flick = Frame::new(sheet_x + s3, SHEET_Y + below_head + s2, flick_w, (sheet_h - (below_head + s2) - s4).max(1.0));
    Layout { sheet: Frame::new(sheet_x, SHEET_Y, SHEET_W, sheet_h), title_w, title_at, clear, flick, slots, content_h, empty }
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
    _words: Words,
    _when: Option<Words>,
    buttons: Vec<Button>,
    /// The slot's place in the column.
    top: f32,
    h: f32,
}

struct HeadParts {
    _icon: Option<ImageLayer>,
    _source: Words,
    _count: Words,
    top: f32,
    h: f32,
}

struct Built {
    sig: String,
    pane: Pane,
    _title: Words,
    clear: Option<Button>,
    scroll: Option<Scroll>,
    items: Vec<ItemParts>,
    _heads: Vec<HeadParts>,
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
        Self { stage: Stage::new(z, SCOPE), built: None, offset: 0.0, lit: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    /// Where the column goes for the focus: index 0 is the top, an item shows whole with `space(3)` of air beyond it.
    fn target(kit: &Kit, lay: &Layout, v: &View, offset: f32) -> f32 {
        let extent = (lay.content_h - lay.flick.h).max(0.0);
        let Some(Slot::Item { top, h, .. }) = (v.index > 0).then(|| lay.slots.iter().find(|s| matches!(s, Slot::Item { item, .. } if item.idx == v.index))).flatten() else { return 0.0 };
        let margin = kit.space(3);
        let want = if *top < offset { top - margin } else if top + h > offset + lay.flick.h { top + h - lay.flick.h + margin } else { offset };
        want.clamp(0.0, extent)
    }

    fn signature(kit: &Kit, v: &View, lay: &Layout, range: (usize, usize)) -> String {
        format!("{}|{}|{:?}|{:?}|{:?}", kit.generation(), kit.text_scale, lay, range, v.entries.len())
    }

    #[allow(clippy::too_many_arguments)]
    fn build(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, v: &View, lay: Layout, range: (usize, usize), offset: f32, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let (s3, s4) = (kit.space(3), kit.space(4));
        let (ink, ink2, ink3) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.ink3());
        let row_radius = kit.tokens.radius(Radius::Row);
        let btn_h = Button::height(kit);
        let pane = Pane::new(kit, lay.sheet, z + 1, PaneKind::Sheet, v.over_video);
        let (title, clear) = scene.adopting_view(root, || -> std::io::Result<_> {
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
            empty = scene.adopting_view(root, || -> std::io::Result<_> {
                let tile_layer = tile(kit, 64, "bell-off", false).layer(kit, z + 3, Props::at(e.tile.x, e.tile.y))?;
                let label = Words::new(kit, e.label.x, e.label.y, e.label.w, z + 4, &[WordLine::new("Nothing new", TypeRole::Label, ink).aligned(Align::Centre)])?;
                let lines: Vec<WordLine> = e.text.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2).aligned(Align::Centre)).collect();
                let text = Words::new(kit, e.text_at.x, e.text_at.y, e.text_at.w, z + 4, &lines)?;
                Ok(Some((tile_layer, label, text)))
            })?;
        } else {
            let sc = Scroll::new(kit, root, flick, lay.content_h)?;
            let inner = &sc.inner;
            for (i, s) in lay.slots.iter().enumerate() {
                if i < range.0 || i > range.1 {
                    continue;
                }
                match s {
                    Slot::Head { top, h, source, icon: ic, count, count_w } => {
                        let y = flick.y + top;
                        let band = y + h - kit.space(2) - 28.0;
                        let parts = scene.adopting(inner, || -> std::io::Result<HeadParts> {
                            let icon_layer = icon::layer(kit, ic, 24, ink3, z + 3, Props::at(flick.x + kit.space(5), band + 2.0))?;
                            let cl = kit.line(TypeRole::Caption);
                            let ty = (band + ((28.0 - cl) / 2.0).round()).round();
                            let source_w = flick.w - 2.0 * kit.space(5) - 24.0 - kit.space(3) - count_w - kit.space(3);
                            let src = Words::new(kit, flick.x + kit.space(5) + 24.0 + kit.space(3), ty, source_w.max(1.0), z + 4, &[WordLine::new(source, TypeRole::Caption, ink2)])?;
                            let cnt = Words::new(kit, flick.x + flick.w - count_w - kit.space(5), ty, count_w + 2.0, z + 4, &[WordLine::new(count, TypeRole::Caption, ink2).aligned(Align::Right)])?;
                            Ok(HeadParts { _icon: icon_layer, _source: src, _count: cnt, top: *top, h: *h })
                        })?;
                        heads.push(parts);
                    }
                    Slot::Item { top, h, item, title: shown, body, text_w, when_w, buttons } => {
                        let f = Frame::new(flick.x, flick.y + top, flick.w, *h);
                        let id = Target::Item(item.idx).id();
                        let plate = Plate::new(kit, engine, inner, Spot { id, screen: clipped(Frame::new(f.x, f.y - offset, f.w, f.h), flick), local: f, region: REGION_LIST, scope: SCOPE, z: z + 2 }, Lift::Row, row_radius)?;
                        let text_x = f.x + s4 + TILE + s4;
                        let (tile_layer, words, when) = scene.adopting(inner, || -> std::io::Result<_> {
                            let tile_layer = tile(kit, TILE as u32, &item.icon, false).layer(kit, z + 3, Props::at(f.x + s4, f.y + s3))?;
                            let mut lines = vec![WordLine::new(shown, TypeRole::Label, ink)];
                            lines.extend(body.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2)));
                            let words = Words::new(kit, text_x, f.y + s3, *text_w, z + 4, &lines)?;
                            let when = if *when_w > 0.0 { Some(Words::new(kit, f.x + f.w - when_w - s4, f.y + s3, when_w + 2.0, z + 4, &[WordLine::new(&item.when, TypeRole::Caption, ink2).aligned(Align::Right)])?) } else { None };
                            Ok((tile_layer, words, when))
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
                        items.push(ItemParts { idx: item.idx, plate, _tile: tile_layer, _words: words, _when: when, buttons: bs, top: *top, h: *h });
                    }
                }
            }
            scroll = Some(sc);
        }
        let layers = before.saturating_sub(kit.room().layers);
        Ok(Built { sig, pane, _title: title, clear, scroll, items, _heads: heads, _empty: empty, range, over_video: v.over_video, lay, layers })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        self.lit = None;
        if let Some(b) = self.built.take() {
            if let Some(c) = b.clear {
                c.destroy(engine);
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
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let lay = layout(kit, v);
        let arrive = Arrive::Right(SHEET_W + 128.0);
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
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), v, lay, range, self.offset, sig)?;
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
        // the focus: an item with buttons is lit as a whole, and the focus flies between its buttons
        let item = (v.index > 0).then(|| v.entries.iter().find_map(|e| if let EntryView::Item(i) = e { (i.idx == v.index).then_some(i) } else { None })).flatten();
        let (focus, lit) = match item {
            Some(i) if !i.buttons.is_empty() => (Some(Target::Button(i.idx, v.button.min(i.buttons.len() - 1)).id()), Some(Target::Item(i.idx).id())),
            Some(i) => (Some(Target::Item(i.idx).id()), None),
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
                to.and_then(Target::from_id).filter(|t| matches!(t, Target::Item(_))).map(|_| radius)
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
        json!({
            "sheet": f(b.lay.sheet),
            "window": f(b.lay.flick),
            "clear": b.clear.as_ref().map(|c| f(c.frame())),
            "items": items,
            "range": [b.range.0, b.range.1],
            "entries": b.lay.slots.len(),
            "offset": self.offset,
            "empty": b.lay.empty.is_some(),
            "focus": engine.focus.current().map(|i| i.0),
            "layers": self.layers(),
            "scope": SCOPE.0,
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
            it._words.feed_lint(kit, lint, &format!("item {}", it.idx), Some(sheet), (0.0, -offset));
            if let Some(w) = &it._when {
                w.feed_lint(kit, lint, &format!("item {} time", it.idx), Some(sheet), (0.0, -offset));
            }
            for btn in &it.buttons {
                btn.feed_lint_at(kit, lint, sheet, (0.0, -offset));
            }
        }
        for h in &b._heads {
            if shown(h.top, h.h) {
                h._source.feed_lint(kit, lint, "group", Some(sheet), (0.0, -offset));
                h._count.feed_lint(kit, lint, "group count", Some(sheet), (0.0, -offset));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    impl Up {
        fn new(mode: Mode, scale: f32) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let env = env();
            let d = open(&env);
            let mut me = Self { _h: h, kit, engine, s: DrawerSurface::new(crate::kit::z::PANELS), d, env, t: 1000 };
            me.apply();
            me
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
            let v = self.d.view(&self.env, NOW);
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.d.life.phase(), self.t * 1_000_000).expect("the drawer is drawn");
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
        for t in [Target::Clear, Target::Backdrop, Target::Item(1), Target::Item(300), Target::Button(1, 0), Target::Button(40, 3)] {
            assert_eq!(Target::from_id(t.id()), Some(t), "{t:?}");
        }
        assert_eq!(Target::from_id(Id(5)), None);
    }

    #[test]
    fn the_sheet_is_640_wide_64_from_the_right_edge_from_the_bars_line_and_as_tall_as_what_it_holds() {
        let u = Up::new(Mode::Dark, 1.0);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["sheet"]["x"], 1216.0);
        assert_eq!(g["sheet"]["y"], 56.0);
        assert_eq!(g["sheet"]["w"], 640.0);
        let h = g["sheet"]["h"].as_f64().unwrap();
        assert!((240.0..=968.0).contains(&h), "{h}");
        assert_eq!(g["empty"], false);
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        let e = Up::empty(Mode::Dark, 1.0);
        let empty = e.s.geometry(&e.engine);
        assert_eq!(empty["empty"], true);
        assert!(empty["sheet"]["h"].as_f64().unwrap() < h, "the empty state is a shorter sheet");
    }

    #[test]
    fn the_focus_starts_on_the_first_item_and_follows_the_keys_between_items_and_their_buttons() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let items = u.d.items(&u.env);
        assert!(items.len() >= 4, "{items:?}");
        let first = &items[0];
        let expected = if first.actions.is_empty() { Target::Item(1) } else { Target::Button(1, 0) };
        assert_eq!(u.engine.focus.current().and_then(Target::from_id), Some(expected));
        u.key(K::Up);
        assert_eq!(u.engine.focus.current(), Some(Target::Clear.id()), "Up from the first item is Clear all");
        u.key(K::Down);
        u.key(K::Down);
        assert!(u.engine.flow.settling_at(u.t * 1_000_000), "the lift flies from one to the next");
        // an item with buttons: Right moves between them and the item stays lit
        for (i, it) in items.iter().enumerate() {
            if it.actions.len() > 1 {
                while u.d.index() != i + 1 {
                    u.key(K::Down);
                }
                u.key(K::Right);
                assert_eq!(u.engine.focus.current(), Some(Target::Button(i + 1, 1).id()));
                assert_eq!(u.s.lit, Some(Target::Item(i + 1).id()));
            }
        }
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_down_the_whole_list_and_empty() {
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
                let mut e = Up::empty(mode, scale);
                let f = e.lint();
                assert!(f.is_empty(), "{mode:?} {scale} empty: {f:#?}");
            }
        }
    }

    #[test]
    fn the_column_moves_to_show_the_focused_item_and_only_the_items_near_the_window_are_made() {
        let mut u = Up::new(Mode::Dark, 2.0);
        let n = u.d.items(&u.env).len();
        let all = u.s.geometry(&u.engine)["entries"].as_u64().unwrap();
        for _ in 0..=n {
            u.key(K::Down);
        }
        let g = u.s.geometry(&u.engine);
        assert!(g["offset"].as_f64().unwrap() > 0.0, "the column moved: {g}");
        let r = &g["range"];
        assert!(r[1].as_u64().unwrap() - r[0].as_u64().unwrap() < all, "only part of the column is made at 200%: {r} of {all}");
        let f = u.engine.focus.frame(u.engine.focus.current().expect("a focus")).expect("a frame");
        let w = u.s.built.as_ref().unwrap().lay.flick;
        assert!(f.y >= w.y - 0.5 && f.bottom() <= w.bottom() + 0.5, "the focused item shows: {f:?} in {w:?}");
    }

    #[test]
    fn clicks_on_clear_all_an_item_a_button_and_the_dim_name_their_targets() {
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
        let item = u.engine.focus.frame(Target::Item(2).id()).expect("an item");
        let t = click(&mut u, item.x + 10.0, item.y + 6.0);
        assert!(matches!(t, Some(Target::Item(2))), "{t:?}");
        assert_eq!(click(&mut u, 300.0, 300.0), Some(Target::Backdrop));
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let with = u.kit.room().layers;
        u.d.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.d.life.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.d.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 20 && !u.s.is_up());
    }
}
