//! The camera list drawn (`docs/research/camera-list-result.md`): a glass sheet in the notification drawer's place and width (560 wide, 40 from
//! the right edge, from the top bar's line) that grows out of Home's camera button, a head with "Cameras" and how many there are, and one
//! column of rows. A row is a small picture of what the camera sees (16:9, 160 by 90, whole pixels), its name and, under it, what its picture is
//! doing: "Live" with the live dot while pictures arrive, else the Cameras grid's words. The sheet is as tall as what it holds, up to five rows
//! and the top of the sixth, and the column scrolls by the focus's spring with a half row of the next one showing.
//!
//! The model decides which row has the focus; the pictures it holds are painted into the rows' layers when they change (nothing is rebuilt for a
//! picture), and only the rows near the window are made (the scene has a budget), made again when the window comes near others.

use super::camlist::{CameraList, PICTURE, ROWS_SHOWN, View};
use super::life::Phase;
use super::quick_layout as quick;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, Root, Scroll, WordLine, Words};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::picture::{Fit, Picture};
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::viewport::Reach;
use crate::kit::widgets::{Lift, Plate, Spot, tile};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use crate::system::popup_surface::placeholder;
use compositor_protocol::Props;
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(212);
const REGION: Region = Region(3000);
const BASE: u32 = 0x2800_0000;
const SHEET_W: f32 = quick::SHEET_W;
const SHEET_MIN_H: f32 = 240.0;
const TILE: f32 = 64.0;
const TITLE: &str = "Cameras";
/// Where Home's camera button is with the shipped layout (`Home::camera_frame`, which the shell hands to the panels each frame): the list's way
/// in and out until it is told.
pub const BUTTON: [f32; 4] = [1537.0, 60.0, 48.0, 48.0];

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Row(usize),
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        Id(BASE
            + match self {
                Target::Backdrop => 2,
                Target::Row(i) => 0x1000 + i as u32,
            })
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            2 => Some(Target::Backdrop),
            n @ 0x1000.. => Some(Target::Row((n - 0x1000) as usize)),
            _ => None,
        }
    }
}

/// One row laid out: its place in the column (from its top) and the lines its name takes.
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    top: f32,
    id: String,
    name: Vec<String>,
    /// Where the name's first line and the state's line start, from the row's top.
    name_y: f32,
    state_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Empty {
    icon: String,
    tile: Frame,
    label: Frame,
    label_text: String,
    text: Vec<String>,
    text_at: Frame,
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    sheet: Frame,
    title_w: f32,
    title_at: (f32, f32),
    /// The head's words at the right, their width and place.
    note: Option<(String, f32, (f32, f32))>,
    flick: Frame,
    slots: Vec<Slot>,
    row_h: f32,
    pitch: f32,
    /// How much of the next row shows under the last whole one (and of the one above after a step up).
    peek: f32,
    content_h: f32,
    /// How many rows show whole.
    fit: usize,
    pic_y: f32,
    text_x: f32,
    text_w: f32,
    dot: f32,
    empty: Option<Empty>,
}

fn layout(kit: &mut Kit, v: &View, place: Frame) -> Layout {
    let (s1, s2, s3, s4, s5, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(7));
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let (title_line, label_line, caption_line) = (kit.line(TypeRole::Title), kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
    let sheet_x = place.x;
    let head_y = s4;
    let inner_w = SHEET_W - 2.0 * s5;
    // the head: the title and, at the right, how many cameras there are (or what is wrong with the link) while there is room beside the title
    let title_w = width_of(kit, title_style, TITLE).ceil().min(inner_w);
    let line_h = (kit.tokens.target * 1.1).round().max(title_line);
    let title_at = (s5, head_y + ((line_h - title_line) / 2.0).round());
    let room = inner_w - title_w - s4;
    // whole or not at all: a count cut short to "8 ca…" says nothing
    let note_w = width_of(kit, caption_style, &v.note).ceil();
    let note = (!v.note.is_empty() && note_w <= room).then(|| (v.note.clone(), note_w, (SHEET_W - s5 - note_w, head_y + ((line_h - caption_line) / 2.0).round())));
    let below_head = head_y + line_h;
    let head_block = below_head + s2;
    let flick_w = SHEET_W - 2.0 * s3;
    let (pic_w, pic_h) = (PICTURE.0 as f32, PICTURE.1 as f32);
    let text_x = s4 + pic_w + s4;
    let text_w = (flick_w - text_x - s4).max(1.0);
    let dot = s2;
    // a name takes one line, two where the text is large
    let name_lines = if kit.text_scale >= 1.5 { 2 } else { 1 };
    let names: Vec<Vec<String>> = v
        .rows
        .iter()
        .map(|r| if name_lines == 1 { vec![wrap::elide(kit, label_style, &r.name, text_w)] } else { wrap::lines(kit, label_style, &r.name, text_w, name_lines) })
        .collect();
    let tallest = names.iter().map(Vec::len).max().unwrap_or(1) as f32 * label_line + caption_line;
    let row_h = (pic_h + 2.0 * s3).max(tallest + 2.0 * s3).round();
    let pitch = row_h + s1;
    let peek = (pitch / 2.0).floor();
    let slots: Vec<Slot> = v
        .rows
        .iter()
        .zip(names)
        .enumerate()
        .map(|(i, (r, name))| {
            let block = name.len() as f32 * label_line + caption_line;
            let name_y = ((row_h - block) / 2.0).round();
            Slot { top: i as f32 * pitch, id: r.id.clone(), state_y: name_y + name.len() as f32 * label_line, name, name_y }
        })
        .collect();
    let n = slots.len();
    let content_h = if n == 0 { 0.0 } else { n as f32 * pitch - s1 };
    let (empty, sheet_h) = match &v.empty {
        Some((icon, label, text)) => {
            let col_w = SHEET_W - 2.0 * s7;
            let lines = wrap::lines(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Centre), text, col_w, 3);
            let tile_f = Frame::new(sheet_x + s7 + ((col_w - TILE) / 2.0).round(), place.y + below_head + s7, TILE, TILE);
            let label_f = Frame::new(sheet_x + s7, tile_f.bottom() + s3, col_w, label_line);
            let text_at = Frame::new(sheet_x + s7, label_f.bottom() + s3, col_w, lines.len() as f32 * caption_line);
            let label_text = wrap::elide(kit, role_style(kit, TypeRole::Label, None, ink, Align::Centre), label, col_w);
            let body = s7 + text_at.bottom() - tile_f.y;
            (Some(Empty { icon: icon.clone(), tile: tile_f, label: label_f, label_text, text: lines, text_at }), SHEET_MIN_H.max(below_head + body + s4))
        }
        None => {
            // five rows and a half of the sixth where there are more than five
            let shown = n.min(ROWS_SHOWN);
            let want = shown as f32 * pitch - s1 + if n > ROWS_SHOWN { peek } else { 0.0 };
            (None, (head_block + want + s4).min(place.h))
        }
    };
    let flick_h = (sheet_h - head_block - s4).max(1.0);
    let fit = (((flick_h + s1) / pitch).floor() as usize).clamp(1, ROWS_SHOWN.max(1));
    Layout {
        sheet: Frame::new(sheet_x, place.y, SHEET_W, sheet_h),
        title_w,
        title_at,
        note,
        flick: Frame::new(sheet_x + s3, place.y + head_block, flick_w, flick_h),
        slots,
        row_h,
        pitch,
        peek,
        content_h,
        fit,
        pic_y: ((row_h - pic_h) / 2.0).round(),
        text_x,
        text_w,
        dot,
        empty,
    }
}

/// The part of `f` that shows in `window`; nothing where none of it does.
fn clipped(f: Frame, window: Frame) -> Frame {
    let (x0, y0) = (f.x.max(window.x), f.y.max(window.y));
    let (x1, y1) = (f.right().min(window.right()), f.bottom().min(window.bottom()));
    if x1 > x0 && y1 > y0 { Frame::new(x0, y0, x1 - x0, y1 - y0) } else { Frame::new(0.0, 0.0, 0.0, 0.0) }
}

/// What a row's state line shows now.
#[derive(Clone, Debug, PartialEq)]
struct Say {
    words: String,
    live: bool,
    dim: bool,
}

struct RowParts {
    idx: usize,
    plate: Plate,
    picture: ImageLayer,
    picture_rest: Props,
    /// The picture's revision and dimming as painted (none: not painted yet).
    painted: Option<(u64, bool)>,
    _name: Words,
    state: Words,
    dot: ShapeLayer,
    say: Say,
    top: f32,
}

struct Built {
    sig: String,
    pane: Pane,
    /// What the words and the rows hang under: it comes in late, within the container's way out of the button.
    _content: Group,
    _title: Words,
    _note: Option<Words>,
    scroll: Option<Scroll>,
    rows: Vec<RowParts>,
    _empty: Option<(ImageLayer, Words, Words)>,
    /// The rows made, from the first to the last (a part of the column near the window).
    range: (usize, usize),
    over_video: bool,
    lay: Layout,
    layers: usize,
}

pub struct CameraListSurface {
    stage: Stage,
    built: Option<Built>,
    /// Where the column is going (the focused row shows whole): kept across the column being made again.
    offset: f32,
    /// The layout and what it was made from: words are measured again when they or the look change, not for each picture that arrives.
    laid: Option<(String, Layout)>,
}

/// The rows (first and last) that meet `from` to `to` of the column.
fn need(lay: &Layout, from: f32, to: f32) -> (usize, usize) {
    let mut lo = usize::MAX;
    let mut hi = 0;
    for (i, s) in lay.slots.iter().enumerate() {
        if s.top + lay.row_h >= from && s.top <= to {
            lo = lo.min(i);
            hi = hi.max(i);
        }
    }
    if lo == usize::MAX { (0, 0) } else { (lo, hi) }
}

impl CameraListSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).springy(), built: None, offset: 0.0, laid: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    /// The sheet's place while it is built: what the mute mark keeps clear of.
    pub fn sheet(&self) -> Option<Frame> {
        self.built.as_ref().map(|b| b.lay.sheet)
    }

    /// How many rows show whole and the size of a row's picture, while built: what the model asks the helper for.
    pub fn shape(&self) -> Option<(usize, (u32, u32))> {
        self.built.as_ref().map(|b| (b.lay.fit, PICTURE))
    }

    /// Where the column goes for the focus: the focused row shows whole, with a half row of the one beyond it where there is one.
    fn target(lay: &Layout, v: &View, offset: f32) -> f32 {
        let extent = (lay.content_h - lay.flick.h).max(0.0);
        let Some(slot) = lay.slots.get(v.index) else { return 0.0 };
        let (top, bottom) = (slot.top, slot.top + lay.row_h);
        let above = if v.index > 0 { lay.peek } else { 0.0 };
        let below = if v.index + 1 < lay.slots.len() { lay.peek } else { 0.0 };
        let want = if top - above < offset { top - above } else if bottom + below > offset + lay.flick.h { bottom + below - lay.flick.h } else { offset };
        want.clamp(0.0, extent).round()
    }

    fn signature(kit: &Kit, lay: &Layout, range: (usize, usize)) -> String {
        format!("{}|{}|{:?}|{:?}", kit.generation(), kit.text_scale, lay, range)
    }

    fn dot_shape(kit: &Kit, live: bool, side: f32) -> compositor_protocol::Shape {
        let colour = if live { kit.ui.look.colour(PaletteRole::Live) } else { kit.ui.look.ink3() };
        plate::solid(colour, side / 2.0)
    }

    /// The state line as it is shown: cut to its room.
    fn state_words(kit: &mut Kit, lay: &Layout, say: &Say) -> String {
        let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink2(), Align::Left);
        let room = lay.text_w - lay.dot - kit.space(2);
        wrap::elide(kit, style, &say.words, room.max(1.0))
    }

    #[allow(clippy::too_many_arguments)]
    fn build(kit: &mut Kit, engine: &mut Engine, root: &Root, z: i32, v: &View, lay: Layout, range: (usize, usize), offset: f32, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let (s2, s4) = (kit.space(2), kit.space(4));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let row_radius = kit.tokens.radius(Radius::Row);
        let content = root.content(kit)?;
        let pane = scene.adopting_view(&root.group, || Pane::new(kit, lay.sheet, z + 1, PaneKind::Sheet, v.over_video));
        let (title, note) = scene.adopting_view(&content, || -> std::io::Result<_> {
            let title = Words::new(kit, lay.sheet.x + lay.title_at.0, lay.sheet.y + lay.title_at.1, lay.title_w + 2.0, z + 4, &[WordLine::new(TITLE, TypeRole::Title, ink).weight(Weight::SemiBold)])?;
            let note = match &lay.note {
                Some((text, w, at)) => Some(Words::new(kit, lay.sheet.x + at.0, lay.sheet.y + at.1, w + 2.0, z + 4, &[WordLine::new(text, TypeRole::Caption, ink2)])?),
                None => None,
            };
            Ok((title, note))
        })?;
        let flick = lay.flick;
        let mut rows = Vec::new();
        let mut scroll = None;
        let mut empty = None;
        if let Some(e) = &lay.empty {
            empty = scene.adopting_view(&content, || -> std::io::Result<_> {
                let tile_layer = tile(kit, TILE as u32, &e.icon, false).layer(kit, z + 3, Props::at(e.tile.x, e.tile.y))?;
                let label = Words::new(kit, e.label.x, e.label.y, e.label.w, z + 4, &[WordLine::new(&e.label_text, TypeRole::Label, ink).aligned(Align::Centre)])?;
                let lines: Vec<WordLine> = e.text.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2).aligned(Align::Centre)).collect();
                let text = Words::new(kit, e.text_at.x, e.text_at.y, e.text_at.w, z + 4, &lines)?;
                Ok(Some((tile_layer, label, text)))
            })?;
        } else {
            let sc = Scroll::new(kit, &content, flick, lay.content_h, Reach::lift(kit, Lift::Row, (flick.w, lay.row_h)))?;
            let inner = &sc.inner;
            let (pw, ph) = PICTURE;
            let dot_side = lay.dot;
            for (i, s) in lay.slots.iter().enumerate() {
                if i < range.0 || i > range.1 {
                    continue;
                }
                let row = &v.rows[i];
                let f = Frame::new(flick.x, flick.y + s.top, flick.w, lay.row_h);
                let plate_part = Plate::new(kit, engine, inner, Spot { id: Target::Row(i).id(), screen: clipped(Frame::new(f.x, f.y - offset, f.w, f.h), flick), local: f, region: REGION, scope: SCOPE, z: z + 2 }, Lift::Row, row_radius)?;
                let say = Say { words: row.words.clone(), live: row.live, dim: row.dim };
                let shown = Self::state_words(kit, &lay, &say);
                let text_x = f.x + lay.text_x;
                let picture_rest = Props::at(f.x + s4, f.y + lay.pic_y);
                let (picture, name, state, dot) = scene.adopting(inner, || -> std::io::Result<_> {
                    let picture = kit.ui.scene.image(pw, ph, z + 3, picture_rest)?;
                    let lines: Vec<WordLine> = s.name.iter().map(|t| WordLine::new(t, TypeRole::Label, ink)).collect();
                    let name = Words::new(kit, text_x, f.y + s.name_y, lay.text_w, z + 4, &lines)?;
                    let line = kit.line(TypeRole::Caption);
                    let dot = plate::shape(&kit.ui, Frame::new(text_x, f.y + s.state_y + ((line - dot_side) / 2.0).round(), dot_side, dot_side), z + 4, Self::dot_shape(kit, say.live, dot_side));
                    let state = Words::new(kit, text_x + dot_side + s2, f.y + s.state_y, (lay.text_w - dot_side - s2).max(1.0), z + 4, &[WordLine::new(&shown, TypeRole::Caption, ink2)])?;
                    Ok((picture, name, state, dot))
                })?;
                rows.push(RowParts { idx: i, plate: plate_part, picture, picture_rest, painted: None, _name: name, state, dot, say, top: s.top });
            }
            scroll = Some(sc);
        }
        let layers = before.saturating_sub(kit.room().layers);
        Ok(Built { sig, pane, _content: content, _title: title, _note: note, scroll, rows, _empty: empty, range, over_video: v.over_video, lay, layers })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for r in b.rows {
                r.plate.destroy(engine);
            }
        }
    }

    /// The layout of `v`: its words are measured again only when they or the look changed.
    fn laid_out(&mut self, kit: &mut Kit, v: &View, place: Frame) -> Layout {
        let names: Vec<(&String, &String)> = v.rows.iter().map(|r| (&r.id, &r.name)).collect();
        let key = format!("{}|{}|{:?}|{:?}|{:?}|{:?}", kit.generation(), kit.text_scale, v.note, v.empty, names, place);
        match &self.laid {
            Some((k, lay)) if *k == key => lay.clone(),
            _ => {
                let lay = layout(kit, v, place);
                self.laid = Some((key, lay.clone()));
                lay
            }
        }
    }

    /// Fits the list to the view and to the phase of its life. `button` is where the camera button is: the sheet grows out of it and goes back
    /// into it. Returns whether anything of it is on the screen.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, button: [f32; 4], list: &CameraList, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        // under the camera button: its middle under the button's, its top just under the pill, kept off the screen's edges
        let place = quick::dropdown(button, SHEET_W, quick::SHEET_H, kit.space(2));
        let lay = self.laid_out(kit, v, place);
        // The list grows out of the button and goes back into it. While it is on its way in or out it keeps the rectangles it started with (a
        // change of the sheet's height as a camera comes or goes must not restart the motion); at rest and parked it follows the sheet, and a
        // parked list that opens again is made afresh at the start of its motion.
        let to = [lay.sheet.x, lay.sheet.y, lay.sheet.w, lay.sheet.h];
        let wanted = Arrive::Container { from: button, to };
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
        // where the column goes, and which rows have to be there to show it
        let want = if up { Self::target(&lay, v, self.offset) } else { self.offset };
        let extent = (lay.content_h - lay.flick.h).max(0.0);
        let want = want.min(extent);
        let held = self.built.as_ref().filter(|b| b.lay == lay).map(|b| b.range);
        let showing = need(&lay, want, want + lay.flick.h);
        let range = match held {
            Some(r) if r.0 <= showing.0 && r.1 >= showing.1 => r,
            _ => need(&lay, want.min(self.offset) - lay.pitch, want.max(self.offset) + lay.flick.h + lay.pitch),
        };
        let sig = Self::signature(kit, &lay, range);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION);
            let built = Self::build(kit, engine, self.stage.root().expect("a root"), self.stage.z(), v, lay, range, self.offset, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the camera list could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, phase) {
            self.update(kit, engine, v, want, list, now_ns);
        }
        Ok(true)
    }

    fn update(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, want: f32, list: &CameraList, now_ns: u64) {
        if let Some(b) = self.built.as_mut()
            && b.over_video != v.over_video
        {
            b.pane.restyle(kit, v.over_video);
            b.over_video = v.over_video;
        }
        self.stage.focus(engine, (!v.rows.is_empty()).then(|| Target::Row(v.index).id()), now_ns);
        if let Some(b) = self.built.as_mut() {
            Self::paint(kit, b, v, list);
        }
        // the column moves on the focus's spring; the frames the pointer hits are where the rows will be
        if (want - self.offset).abs() > 0.5
            && let Some(b) = self.built.as_mut()
            && let Some(sc) = b.scroll.as_mut()
        {
            sc.to(engine, want, false);
            self.offset = want;
            Self::place(b, engine, want);
        }
    }

    /// Paints the pictures that changed and puts each row's words where they are: nothing is made again for either.
    fn paint(kit: &mut Kit, b: &mut Built, v: &View, list: &CameraList) {
        let radius = (kit.tokens.radius(Radius::Row) - kit.space(3)).max(0.0);
        let dim = kit.tokens.states.disabled_opacity;
        for r in &mut b.rows {
            let Some(row) = v.rows.get(r.idx) else { continue };
            let say = Say { words: row.words.clone(), live: row.live, dim: row.dim };
            if r.painted != Some((row.rev, say.dim)) {
                let (w, h) = (r.picture.width(), r.picture.height());
                let shown = match list.picture(&row.id) {
                    Some(p) => rounded(p, w, h, radius),
                    None => placeholder(kit, w, h, radius),
                };
                shown.paint(&mut r.picture);
                r.picture.set(Props { opacity: if say.dim { dim } else { 1.0 }, ..r.picture_rest });
                r.painted = Some((row.rev, say.dim));
            }
            if r.say.words != say.words {
                let shown = Self::state_words(kit, &b.lay, &say);
                r.state.set(kit, 0, &shown);
            }
            if r.say.live != say.live {
                r.dot.reshape(Self::dot_shape(kit, say.live, b.lay.dot));
            }
            r.say = say;
        }
    }

    /// Puts the registry's frames where the rows are at `offset`: what shows of them.
    fn place(b: &Built, engine: &mut Engine, offset: f32) {
        let flick = b.lay.flick;
        for r in &b.rows {
            engine.focus.place(r.plate.id, clipped(Frame::new(flick.x, flick.y + r.top - offset, flick.w, b.lay.row_h), flick));
        }
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it, and a click is the target it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let radius = kit.tokens.radius(Radius::Row);
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(Target::from_id).filter(|t| matches!(t, Target::Row(_))).map(|_| radius),
            Touch::Press { .. } => None,
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
        self.laid = None;
    }

    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers + 2)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let rows: Vec<Value> = b.rows.iter().map(|r| json!({"idx": r.idx, "frame": engine.focus.frame(r.plate.id).map(f), "painted": r.painted.map(|p| p.0), "words": r.say.words})).collect();
        json!({
            "sheet": f(b.lay.sheet),
            "window": f(b.lay.flick),
            "rows": rows,
            "range": [b.range.0, b.range.1],
            "entries": b.lay.slots.len(),
            "fit": b.lay.fit,
            "offset": self.offset,
            "empty": b.lay.empty.is_some(),
            "focus": engine.focus.current().map(|i| i.0),
            "layers": self.layers(),
            "scope": SCOPE.0,
            "origin": self.stage.arrive().and_then(|a| a.origin()),
        })
    }

    /// Describes what is on the screen to a lint: the sheet and the words on it; of the column, the rows that show whole.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = lint.panel("camera list sheet", b.lay.sheet, kit.ui.look.colour(PaletteRole::Sheet), false);
        b._title.feed_lint(kit, lint, "camera list title", Some(sheet), (0.0, 0.0));
        if let Some(n) = &b._note {
            n.feed_lint(kit, lint, "camera list count", Some(sheet), (0.0, 0.0));
        }
        if let Some((_, label, text)) = &b._empty {
            label.feed_lint(kit, lint, "empty label", Some(sheet), (0.0, 0.0));
            text.feed_lint(kit, lint, "empty text", Some(sheet), (0.0, 0.0));
        }
        let (flick, offset) = (b.lay.flick, self.offset);
        for r in &b.rows {
            if r.top - offset >= -0.5 && r.top - offset + b.lay.row_h <= flick.h + 0.5 {
                r._name.feed_lint(kit, lint, &format!("camera {}", r.idx), Some(sheet), (0.0, -offset));
                r.state.feed_lint(kit, lint, &format!("camera {} state", r.idx), Some(sheet), (0.0, -offset));
            }
        }
    }
}

/// The picture in a box of `w` by `h` with the corners rounded: as it is where it already is that size (the helper decodes at the box's size).
fn rounded(p: &Picture, w: u32, h: u32, radius: f32) -> Picture {
    let mut shown = if (p.width, p.height) == (w, h) { p.clone() } else { p.fitted(w, h, Fit::Cover) };
    shown.round_corners(radius);
    shown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::{Effect, Input};
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::life::SPRING_MS;
    use ui::lucent::palette::Mode;

    fn topic(n: usize) -> Value {
        json!({"state": "ready", "cameras": (0..n).map(|i| json!({"id": format!("c{i}"), "name": format!("Front door camera number {i}"), "online": true})).collect::<Vec<_>>()})
    }

    fn picture_bytes() -> Vec<u8> {
        include_bytes!("../../../../assets/icon160.png").to_vec()
    }

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: CameraListSurface,
        l: CameraList,
        asked: Vec<u64>,
        t: u64,
    }

    impl Up {
        fn new(mode: Mode, scale: f32, n: usize) -> Self {
            Self::with(mode, scale, topic(n))
        }

        fn with(mode: Mode, scale: f32, data: Value) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut l = CameraList::new();
            l.update(&Input::Helper { topic: "cameras".into(), data }, 1000);
            l.life.show(1000);
            let asked = l.open(1000).effects.iter().filter_map(|e| if let Effect::Fetch { tag, .. } = e { Some(*tag) } else { None }).collect();
            let mut me = Self { _h: h, kit, engine, s: CameraListSurface::new(crate::kit::z::PANELS), l, asked, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            self.apply_at(BUTTON);
        }

        fn apply_at(&mut self, button: [f32; 4]) {
            let v = self.l.view(false, self.t);
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.l.life.phase(), button, &self.l, self.t * 1_000_000).expect("the list is drawn");
            if let Some((fit, size)) = self.s.shape() {
                self.l.set_shape(fit, size);
            }
        }

        fn key(&mut self, k: K) {
            self.l.key(k, self.t);
            self.t += 40;
            self.apply();
        }

        fn geometry(&self) -> Value {
            self.s.geometry(&self.engine)
        }

        fn sheet(&self) -> [f32; 4] {
            let s = self.s.built.as_ref().expect("built").lay.sheet;
            [s.x, s.y, s.w, s.h]
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Backdrop, Target::Row(0), Target::Row(31)] {
            assert_eq!(Target::from_id(t.id()), Some(t), "{t:?}");
        }
        assert_eq!(Target::from_id(Id(5)), None);
    }

    #[test]
    fn the_sheet_is_the_drawers_place_and_width_as_tall_as_its_rows_and_five_and_a_half_at_most() {
        let high = |n: usize| Up::new(Mode::Dark, 1.0, n).geometry()["sheet"]["h"].as_f64().unwrap();
        let (one, three, five, six, eight, many) = (high(1), high(3), high(5), high(6), high(8), high(30));
        assert!(one < three && three < five && five < six, "{one} {three} {five} {six}");
        assert_eq!((six, eight, many), (eight, eight, eight), "more than five rows show five and the top of the next, whatever the count");
        assert!(eight <= f64::from(quick::SHEET_H) && eight < 800.0, "{eight}");
        let u = Up::new(Mode::Dark, 1.0, 8);
        let g = u.geometry();
        let q = quick::dropdown(BUTTON, quick::SHEET_W, quick::SHEET_H, u.kit.space(2));
        assert_eq!((g["sheet"]["x"].as_f64().unwrap() as f32, g["sheet"]["y"].as_f64().unwrap() as f32, g["sheet"]["w"].as_f64().unwrap() as f32), (q.x, q.y, q.w), "under the button, its middle under the button's");
        assert_eq!(g["sheet"]["x"].as_f64().unwrap() + g["sheet"]["w"].as_f64().unwrap() / 2.0, f64::from(BUTTON[0] + BUTTON[2] / 2.0), "centred on the button");
        assert_eq!((g["fit"].clone(), u.engine.focus.modal()), (json!(5), Some(SCOPE)));
        let rows = g["rows"].as_array().unwrap();
        let (a, b) = (&rows[0]["frame"], &rows[1]["frame"]);
        let row_h = 90.0 + 2.0 * f64::from(u.kit.space(3));
        assert_eq!((a["h"].as_f64().unwrap(), b["y"].as_f64().unwrap() - a["y"].as_f64().unwrap()), (row_h, row_h + f64::from(u.kit.space(1))), "rows of the picture and its air, a row of air between");
        assert!(rows.iter().all(|r| r["frame"]["x"].as_f64().unwrap().fract() == 0.0 && r["frame"]["y"].as_f64().unwrap().fract() == 0.0), "whole pixels: {rows:?}");
        assert_eq!(u.engine.focus.current(), Some(Target::Row(0).id()), "the focus starts on the first camera");
    }

    #[test]
    fn the_column_scrolls_by_whole_pixels_to_keep_the_focused_row_whole_with_half_a_row_of_the_next_under_it() {
        let mut u = Up::new(Mode::Dark, 1.0, 8);
        let lay = u.s.built.as_ref().unwrap().lay.clone();
        for _ in 0..4 {
            u.key(K::Down);
        }
        assert_eq!(u.geometry()["offset"], 0.0, "the fifth row and half of the sixth show without moving");
        u.key(K::Down);
        assert_eq!(u.geometry()["offset"].as_f64().unwrap(), f64::from(lay.pitch), "the sixth row takes one row's step");
        for _ in 0..2 {
            u.key(K::Down);
        }
        let g = u.geometry();
        assert_eq!(g["offset"].as_f64().unwrap(), f64::from(lay.content_h - lay.flick.h), "the last row ends at the window's foot");
        let f = u.engine.focus.frame(Target::Row(7).id()).expect("the last row");
        assert_eq!(f.bottom(), lay.flick.bottom());
        for _ in 0..7 {
            u.key(K::Up);
        }
        assert_eq!(u.geometry()["offset"], 0.0, "back at the top");
        assert_eq!(u.engine.focus.current(), Some(Target::Row(0).id()));
    }

    #[test]
    fn only_the_rows_near_the_window_are_made_and_the_window_keeps_the_focused_row_in_it() {
        let mut u = Up::new(Mode::Dark, 1.0, 30);
        let r = u.geometry()["range"].clone();
        assert!(r[1].as_u64().unwrap() < 10, "a few rows beyond the window, not the thirty: {r}");
        for _ in 0..20 {
            u.key(K::Down);
        }
        let g = u.geometry();
        let r = &g["range"];
        assert!(r[0].as_u64().unwrap() > 5 && r[1].as_u64().unwrap() >= 20, "the made rows moved with the focus: {r}");
        let f = u.engine.focus.frame(u.engine.focus.current().expect("a focus")).expect("a frame");
        let w = u.s.built.as_ref().unwrap().lay.flick;
        assert!(f.y >= w.y - 0.5 && f.bottom() <= w.bottom() + 0.5, "the focused row shows: {f:?} in {w:?}");
        assert!(u.kit.ui.scene.groups_alive() < 40, "the rows stay inside the scene's budget of groups");
    }

    #[test]
    fn a_picture_is_painted_into_its_row_and_its_words_follow_without_making_the_rows_again() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let g = u.geometry();
        assert_eq!((g["rows"][0]["painted"].clone(), g["rows"][0]["words"].clone()), (json!(0), json!("Connecting")), "the plate and the first words until a picture comes");
        let (sig, layers) = (u.s.built.as_ref().unwrap().sig.clone(), u.kit.room().layers);
        u.t += 150;
        u.l.update(&Input::Bytes { tag: u.asked[0], status: 200, bytes: picture_bytes() }, u.t);
        u.apply();
        let g = u.geometry();
        assert_eq!((g["rows"][0]["painted"].clone(), g["rows"][0]["words"].clone()), (json!(1), json!("Live")));
        assert_eq!(g["rows"][1]["painted"], 0, "the other rows still wait");
        assert_eq!((u.s.built.as_ref().unwrap().sig.clone(), u.kit.room().layers), (sig, layers), "nothing is made again for a picture or its words");
        // the picture is in the layer, the box's own size
        let row = &u.s.built.as_ref().unwrap().rows[0];
        assert_eq!((row.picture.width(), row.picture.height()), PICTURE);
        assert!(row.picture.pixels().chunks_exact(4).any(|p| p[3] == 255), "painted");
        // an old picture is dimmed and says its age
        u.t += 14_000;
        u.apply();
        let g = u.geometry();
        assert_eq!(g["rows"][0]["words"], "14 s ago");
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_open_scrolled_and_empty() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale, 8);
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
                for step in 0..8 {
                    u.key(K::Down);
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} after {step} downs: {f:#?}");
                }
                for (state, why) in [("unlinked", "link"), ("ready", "none")] {
                    let mut e = Up::with(mode, scale, json!({"state": state, "cameras": []}));
                    assert_eq!(e.geometry()["empty"], true, "{why}");
                    let f = e.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} {why}: {f:#?}");
                }
            }
        }
    }

    #[test]
    fn clicks_on_a_row_and_on_the_dim_name_their_targets() {
        let mut u = Up::new(Mode::Dark, 1.0, 4);
        let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
        let mut n = 0u64;
        let mut click = |u: &mut Up, x: f32, y: f32| {
            n += 100;
            let t = (5000 + n) * 1_000_000;
            u.s.pointer(&u.kit, &mut u.engine, t, at(x, y, 0));
            u.s.pointer(&u.kit, &mut u.engine, t + 1_000_000, at(x, y, 1));
            u.s.pointer(&u.kit, &mut u.engine, t + 2_000_000, at(x, y, 0))
        };
        for i in [0usize, 2, 3] {
            let f = u.engine.focus.frame(Target::Row(i).id()).expect("a row");
            assert_eq!(click(&mut u, f.x + 10.0, f.y + 10.0), Some(Target::Row(i)));
        }
        assert_eq!(click(&mut u, 300.0, 300.0), Some(Target::Backdrop));
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, 5);
        let with = u.kit.room().layers;
        u.l.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.l.life.tick(2000 + u64::from(SPRING_MS));
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.l.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 20 && !u.s.is_up());
    }

    #[test]
    fn it_grows_out_of_the_camera_button_into_the_sheet_and_goes_back_into_it() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let first = u.s.stage.arrive();
        assert_eq!(first, Some(Arrive::Container { from: BUTTON, to: u.sheet() }), "out of the button into the sheet, not in from the edge");
        assert!(u.s.stage.root().expect("a root").is_container(), "the glass grows first and the words come in as it settles");
        // at rest it follows where the button is
        u.l.life.tick(1000 + u64::from(SPRING_MS) + 1);
        let moved = [1600.0, 70.0, 56.0, 56.0];
        u.apply_at(moved);
        assert_eq!(u.s.stage.arrive(), Some(Arrive::Container { from: moved, to: u.sheet() }));
        // going: back into the button it was at when it left, whatever the button does meanwhile
        let leaving = u.s.stage.arrive();
        u.l.life.hide(1000);
        u.apply_at(BUTTON);
        assert_eq!(u.s.stage.arrive(), leaving);
        assert!(!u.s.stage.root().expect("a root").is_shown(), "the root is on its way back");
    }
}
