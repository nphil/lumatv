//! The quick panel drawn (`docs/mocks/10-quick-panel`, `29-qp-*`): the sheet docked to the right edge on glass, its head, the profiles'
//! strip, the rows of tiles in a window that scrolls by whole rows on the focus's spring, "All settings" under them and the glyphs of what
//! the keys do; the PIN pad in place of the list; the edit list with its move and remove marks; and the card beside the sheet (a value
//! picker, a slider, the tiles to add). The layers hang under one root that slides in from the right edge on a track of the compositor's
//! ([`Root`]); the dim behind them is a layer of its own. Every focusable is registered with the shared engine (its own scope, a modal one
//! while the panel is up), so the focus moves with the flowing lift, the pointer's hover is a cue and its click is the act.
//!
//! The model decides where the focus is; this follows it. What changes without changing the layout (a stepper's value, a switch, the PIN's
//! dots, the card's slider, the clock) is changed in place; what changes the layout builds the panel's content again under the same root.

use super::life::Phase;
use super::stage::Dim;
use super::quick::tiles::Row;
use super::quick::{EditView, Focus, PadView, Person, PopupView, Target, View};
use super::quick_layout::{self as lay, PopupLayout, RowLayout, Trail};
use super::ui::{Arrive, Line, Root, Scroll, WordLine, Words, follow, hints_width, scrim, scrim_set};
use crate::kit::avatar::{self, Who};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{HintRow, RowAt, glyph_side};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Chip, Hover, Lift, Plate, Px, Slider, Spot, Stepper, Toggle, edge as wash_edge, focus_wash, lit_edge, rotate_cw, selected_wash, tile};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Ease, Fill, GroupPose, Props, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(200);
pub const POPUP_SCOPE: Scope = Scope(206);
const REGION_STRIP: Region = Region(2001);
const REGION_LIST: Region = Region(2002);
const REGION_FOOT: Region = Region(2003);
const REGION_POPUP: Region = Region(2004);
const REGION_PAD: Region = Region(2005);

/// How long after a scroll starts the rows that do not fit whole are taken away (the spring has landed).
const SETTLE_MS: u64 = 300;
/// The steps of a wrong PIN's shake: how far the dots go to each side and for how long.
const SHAKE: [f32; 6] = [-14.0, 12.0, -9.0, 6.0, -3.0, 0.0];
const SHAKE_STEP_MS: u64 = 50;

fn other(e: ui::error::UiError) -> std::io::Error {
    std::io::Error::other(format!("{e}"))
}

fn zero() -> Frame {
    Frame::new(0.0, 0.0, 0.0, 0.0)
}

enum Ctrl {
    None,
    Toggle(Toggle),
    Chip(#[allow(dead_code)] Chip),
    Value { _text: Option<Line>, _chevron: Option<ImageLayer> },
    Stepper(Stepper),
}

struct RowParts {
    id: Id,
    row_id: String,
    cell: Group,
    plate: Plate,
    _tile: ImageLayer,
    words: Words,
    ctrl: Ctrl,
    layout: RowLayout,
    /// The row's top in the list's content, from the window's top.
    top: f32,
    enabled: bool,
    visible: bool,
    layers: usize,
}

struct ListParts {
    scroll: Scroll,
    rows: Vec<RowParts>,
    tops: Vec<f32>,
    x0: f32,
    /// The rows are hidden when they do not fit once the spring has landed (ms).
    settle_due: Option<u64>,
}

struct EditRow {
    id: Id,
    plate: Plate,
    _tile: ImageLayer,
    words: Words,
    grip: ImageLayer,
    marks: [Id; 3],
    top: f32,
}

struct EditParts {
    rows: Vec<EditRow>,
    add_id: Id,
    add_plate: Plate,
    _add: (ShapeLayer, ImageLayer, Words),
    scroll: Scroll,
    tops: Vec<f32>,
    row_h: f32,
    x0: f32,
    grips: Option<(usize, bool, bool)>,
    /// Where the three marks of a row are inside it.
    mark_rel: [Frame; 3],
}

struct StripCell {
    id: Id,
    plate: Plate,
    _avatar: ImageLayer,
    _name: Words,
}

enum PadFace {
    Words(#[allow(dead_code)] Words),
    Icon(#[allow(dead_code)] ImageLayer),
}

struct PadCell {
    id: Id,
    plate: Plate,
    _base: ShapeLayer,
    _face: PadFace,
    frame: Frame,
}

struct PadParts {
    _avatar: Option<ImageLayer>,
    _name: Words,
    dots: Vec<(ShapeLayer, Frame)>,
    filled: usize,
    note: Words,
    cells: Vec<PadCell>,
    shake: u32,
}

struct Foot {
    button: Button,
    _hints: Vec<HintRow>,
}

struct Head {
    title: Words,
    clock: Option<Words>,
    _hints: Vec<HintRow>,
}

struct Built {
    sig: String,
    pane: Pane,
    head: Head,
    backdrop: Id,
    strip: Vec<StripCell>,
    list: Option<ListParts>,
    edit: Option<EditParts>,
    pad: Option<PadParts>,
    foot: Option<Foot>,
    over_video: bool,
}

struct PickRow {
    plate: Plate,
    _icon: Option<ImageLayer>,
    _words: Words,
    _detail: Option<Words>,
    _tick: Option<ImageLayer>,
}

// one card is up at a time: boxing the slider would only add an indirection to what is made and dropped with it
#[allow(clippy::large_enum_variant)]
enum PopupFace {
    Slider { slider: Slider, value: Line, shown: f64 },
    Pick { rows: Vec<PickRow> },
}

struct PopupBuilt {
    sig: String,
    _group: Option<Group>,
    _pane: Pane,
    _title: Words,
    _text: Option<Words>,
    face: PopupFace,
    layout: PopupLayout,
    _box: Option<ShapeLayer>,
    _marks: Vec<Words>,
    _icons: Vec<ImageLayer>,
}

pub struct QuickSurface {
    z: i32,
    root: Option<Root>,
    scrim: Option<ShapeLayer>,
    built: Option<Built>,
    popup: Option<PopupBuilt>,
    pointer: Pointer,
    dim: Dim,
    hover: Hover,
    popup_hover: Hover,
    modal: bool,
    popup_modal: bool,
    /// A focus follows without a lift's animation the first time after the panel comes up.
    first_focus: bool,
    shake_due: Option<(u64, usize)>,
}

fn person_who(p: &Person) -> Who {
    Who { initial: p.initial.clone(), colour: p.color.clone(), file: p.avatar_file.clone() }
}

impl QuickSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self {
            z,
            root: None,
            scrim: None,
            built: None,
            popup: None,
            pointer: Pointer::new(),
            dim: Dim::default(),
            hover: Hover::new(z + 4),
            popup_hover: Hover::new(z + 24),
            modal: false,
            popup_modal: false,
            first_focus: true,
            shake_due: None,
        }
    }

    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    // ---- building

    fn signature(kit: &mut Kit, v: &View, rows: &[RowLayout]) -> String {
        let strip: Vec<String> = v.strip.iter().flatten().map(|p| format!("{}/{}/{}/{}/{:?}", p.id, p.name, p.active, p.locked, p.avatar_file)).collect();
        let edit: Vec<String> = v.edit.iter().flat_map(|e| e.rows.iter().map(|r| format!("{}/{}/{}/{}", r.id, r.label, r.detail, r.icon))).collect();
        let head = lay::head_layout(kit, v, lay::SHEET_W - 2.0 * kit.space(6));
        format!(
            "{}|{}|{}|{}|{:?}|{:?}|{}|{}|{:?}|{:?}|{:?}",
            kit.generation(),
            kit.text_scale,
            v.editing,
            v.pad.is_some(),
            strip,
            edit,
            v.hints_foot.len(),
            rows.iter().map(RowLayout::signature).collect::<Vec<_>>().join("\n"),
            head.title,
            head.below,
            v.pad.as_ref().map(|p| p.pin.name.clone())
        )
    }

    // the arguments are the places `Spot` names, passed through
    #[allow(clippy::too_many_arguments)]
    fn make_plate(kit: &mut Kit, engine: &mut Engine, parent: &Group, id: Id, frame: Frame, local: Frame, region: Region, scope: Scope, z: i32, lift: Lift, radius: Radius) -> std::io::Result<Plate> {
        let r = kit.tokens.radius(radius);
        Plate::new(kit, engine, parent, Spot { id, screen: frame, local, region, scope, z }, lift, r)
    }

    fn build_ctrl(kit: &mut Kit, r: &Row, l: &RowLayout, x: f32, y: f32, z: i32) -> std::io::Result<Ctrl> {
        let ink2 = kit.ui.look.ink2();
        Ok(match &l.trail {
            Trail::None => match &l.below {
                Some(b) => Ctrl::Stepper(Stepper::new(kit, Frame::new(x + b.frame.x, y + b.frame.y, b.frame.w, b.frame.h), z, &b.value, false)?),
                None => Ctrl::None,
            },
            Trail::Toggle => {
                let (w, h) = Toggle::size(kit);
                Ctrl::Toggle(Toggle::new(kit, Frame::new(x + l.trail_at.0, y + l.trail_at.1, w, h), z, r.on))
            }
            Trail::Chip { text, tone, .. } => Ctrl::Chip(Chip::new(kit, x + l.trail_at.0, y + l.trail_at.1, z, text, tone)?),
            Trail::Value { text, text_w, chevron, w, h } => {
                let side = (kit.tokens.target * 0.5).round();
                let line = kit.line(TypeRole::Label);
                let text_line = if *text_w > 0.0 {
                    let style = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
                    Some(Line::new(kit, Frame::new(x + l.trail_at.0, y + l.trail_at.1 + ((h - line) / 2.0).round(), text_w + 2.0, line), z, style, text)?)
                } else {
                    None
                };
                let chev = if *chevron { icon::layer(kit, "chevron-right", side as u32, ink2, z, Props::at(x + l.trail_at.0 + w - side, y + l.trail_at.1 + ((h - side) / 2.0).round()))? } else { None };
                Ctrl::Value { _text: text_line, _chevron: chev }
            }
        })
    }

    fn build_list(kit: &mut Kit, engine: &mut Engine, root: &Group, rows: &[Row], layouts: Vec<RowLayout>, window: Frame, z: i32) -> std::io::Result<ListParts> {
        let spacing = kit.space(1);
        let mut tops = Vec::new();
        let mut top = 0.0;
        for l in &layouts {
            tops.push(top);
            top += l.h + spacing;
        }
        let content_h = (top - spacing).max(0.0);
        let scroll = Scroll::new(kit, root, window, content_h)?;
        let scene = kit.ui.scene.clone();
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let x0 = window.x;
        let mut parts = Vec::new();
        for (i, (r, l)) in rows.iter().zip(layouts).enumerate() {
            let before = kit.room().layers;
            let id = Target::Row(i).id();
            let top = tops[i];
            let (x, y) = (x0, window.y + top);
            let visible = scroll.fits(top, l.h, 0.0);
            let cell = scene
                .group(Some(scroll.inner.id()), GroupPose { visible, opacity: if r.enabled { 1.0 } else { kit.tokens.states.disabled_opacity }, ..GroupPose::IDENTITY })
                .map_err(other)?;
            let local = Frame::new(x, y, window.w, l.h);
            let plate = Self::make_plate(kit, engine, &cell, id, if visible { local } else { zero() }, local, REGION_LIST, SCOPE, z + 1, Lift::Row, Radius::Row)?;
            engine.focus.set_enabled(id, r.enabled);
            let (tile_px, words, ctrl) = scene.adopting(&cell, || -> std::io::Result<_> {
                let tile_px = tile(kit, l.tile.w as u32, &l.icon, false).layer(kit, z + 2, Props::at(x + l.tile.x, y + l.tile.y))?;
                let mut lines: Vec<WordLine> = l.label.iter().map(|t| WordLine::new(t, TypeRole::Label, ink)).collect();
                lines.extend(l.detail.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2)));
                let words = Words::new(kit, x + l.words.x, y + l.words.y, l.words.w, z + 3, &lines)?;
                let ctrl = Self::build_ctrl(kit, r, &l, x, y, z + 3)?;
                Ok((tile_px, words, ctrl))
            })?;
            let layers = before.saturating_sub(kit.room().layers);
            parts.push(RowParts { id, row_id: r.id.clone(), cell, plate, _tile: tile_px, words, ctrl, layout: l, top, enabled: r.enabled, visible, layers });
        }
        Ok(ListParts { scroll, rows: parts, tops, x0, settle_due: None })
    }

    fn build_strip(kit: &mut Kit, engine: &mut Engine, root: &Group, people: &[Person], origin: Frame, z: i32) -> std::io::Result<(Vec<StripCell>, f32)> {
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let sl = lay::strip_layout(kit, people.len(), origin.w);
        let mut cells = Vec::new();
        for (i, p) in people.iter().enumerate() {
            let f = Frame::new(origin.x + sl.cells[i].x, origin.y, sl.cells[i].w, sl.h);
            let id = Target::Strip(i).id();
            let plate = Self::make_plate(kit, engine, root, id, f, f, REGION_STRIP, SCOPE, z + 1, Lift::Tile, Radius::Tile)?;
            let who = person_who(p);
            let px = avatar::px(kit, &who, sl.avatar as u32, p.active);
            let ax = f.x + ((f.w - sl.avatar) / 2.0).round();
            let avatar_layer = px.layer(kit, z + 2, Props::at(ax, f.y + kit.space(2)))?;
            let tint = if p.active { ink } else { ink2 };
            let name = wrap::elide(kit, role_style(kit, TypeRole::Caption, None, tint, Align::Centre), &p.name, f.w);
            let words = Words::new(kit, f.x, f.y + kit.space(2) + sl.avatar + kit.space(1), f.w, z + 2, &[WordLine::new(&name, TypeRole::Caption, tint).aligned(Align::Centre)])?;
            cells.push(StripCell { id, plate, _avatar: avatar_layer, _name: words });
        }
        Ok((cells, sl.h))
    }

    fn dot_shape(kit: &Kit, filled: bool, d: f32) -> Shape {
        let look = &kit.ui.look;
        let (fill, edge) = if filled { (look.accent(), look.colour(PaletteRole::Edge)) } else { (look.colour(PaletteRole::Glass), look.colour(PaletteRole::EdgeRaised)) };
        Shape { corner_radius: d / 2.0, fill: Fill::Solid(fill), border_width: 1.0, border: edge }
    }

    fn build_pad(kit: &mut Kit, engine: &mut Engine, root: &Group, p: &PadView, area: Frame, z: i32) -> std::io::Result<PadParts> {
        let l = lay::pad_layout(kit, area.w);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let (ox, oy) = (area.x, area.y);
        let avatar_layer = match &p.avatar {
            Some(person) => Some(avatar::px(kit, &person_who(person), l.avatar.w as u32, false).layer(kit, z + 2, Props::at(ox + l.avatar.x, oy + l.avatar.y))?),
            None => None,
        };
        let name = Words::new(kit, ox, oy + l.name.y, l.name.w, z + 2, &[WordLine::new(&p.pin.name, TypeRole::Title, ink).weight(Weight::SemiBold).aligned(Align::Centre)])?;
        let dots = l
            .dots
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let f = Frame::new(ox + d.x, oy + d.y, d.w, d.h);
                (plate::shape(&kit.ui, f, z + 2, Self::dot_shape(kit, i < p.pin.filled, d.w)), f)
            })
            .collect();
        let note = Words::new(kit, ox, oy + l.note.y, l.note.w, z + 2, &[WordLine::new(&p.pin.note, TypeRole::Caption, ink2).aligned(Align::Centre)])?;
        let label_line = kit.line(TypeRole::Label);
        let radius = kit.tokens.radius(Radius::Control);
        let mut cells = Vec::new();
        for (i, c) in l.cells.iter().enumerate() {
            let f = Frame::new(ox + c.x, oy + c.y, c.w, c.h);
            let id = Target::Pad(i).id();
            let plate_parts = Plate::new(kit, engine, root, Spot { id, screen: f, local: f, region: REGION_PAD, scope: SCOPE, z: z + 1 }, Lift::Button, radius)?;
            let look = &kit.ui.look;
            let base = plate::shape(&kit.ui, f, z, Shape { corner_radius: radius, fill: Fill::Solid(look.colour(PaletteRole::Glass)), border_width: 1.0, border: look.colour(PaletteRole::Edge) });
            let face = match super::pin::cell(i) {
                super::pin::Cell::Digit(d) => PadFace::Words(Words::new(kit, f.x, f.y + ((f.h - label_line) / 2.0).round(), f.w, z + 2, &[WordLine::new(&d.to_string(), TypeRole::Label, ink).weight(Weight::SemiBold).aligned(Align::Centre)])?),
                cell => {
                    let name = if matches!(cell, super::pin::Cell::Delete) { "delete" } else { "x" };
                    let side = 32u32;
                    let at = Props::at(f.x + ((f.w - side as f32) / 2.0).round(), f.y + ((f.h - side as f32) / 2.0).round());
                    PadFace::Icon(icon::layer(kit, name, side, ink2, z + 2, at)?.ok_or_else(|| std::io::Error::other("no such icon"))?)
                }
            };
            cells.push(PadCell { id, plate: plate_parts, _base: base, _face: face, frame: f });
        }
        Ok(PadParts { _avatar: avatar_layer, _name: name, dots, filled: p.pin.filled, note, cells, shake: p.pin.shake })
    }

    fn build_edit(kit: &mut Kit, engine: &mut Engine, root: &Group, e: &EditView, area: Frame, z: i32) -> std::io::Result<EditParts> {
        let row_h = lay::edit_row_h(kit);
        let spacing = kit.space(1);
        let n = e.rows.len();
        let content_h = (n as f32 + 1.0) * row_h + n as f32 * spacing;
        let scroll = Scroll::new(kit, root, area, content_h)?;
        let scene = kit.ui.scene.clone();
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let tile_side = (kit.tokens.target * 0.75).round();
        let words_x = kit.space(3) + tile_side + kit.space(4);
        let mark = tile_side;
        let grip_w = 3.0 * mark + 2.0 * kit.space(1);
        let words_w = area.w - words_x - grip_w - 2.0 * kit.space(3);
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        for (i, r) in e.rows.iter().enumerate() {
            let top = i as f32 * (row_h + spacing);
            tops.push(top);
            let id = Target::EditRow(i).id();
            let f = Frame::new(area.x, area.y + top, area.w, row_h);
            let fits = scroll.fits(top, row_h, 0.0);
            let plate = Self::make_plate(kit, engine, &scroll.inner, id, if fits { f } else { zero() }, f, REGION_LIST, SCOPE, z + 1, Lift::Row, Radius::Row)?;
            let (tile_px, words, grip) = scene.adopting(&scroll.inner, || -> std::io::Result<_> {
                let tile_px = tile(kit, tile_side as u32, &r.icon, false).layer(kit, z + 2, Props::at(f.x + kit.space(3), f.y + ((row_h - tile_side) / 2.0).round()))?;
                let label = wrap::elide(kit, role_style(kit, TypeRole::Label, None, ink, Align::Left), &r.label, words_w);
                let detail = wrap::elide(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Left), &r.detail, words_w);
                let mut lines = vec![WordLine::new(&label, TypeRole::Label, ink)];
                if !detail.is_empty() {
                    lines.push(WordLine::new(&detail, TypeRole::Caption, ink2));
                }
                let wh = Words::height_of(kit, &lines);
                let words = Words::new(kit, f.x + words_x, f.y + ((row_h - wh) / 2.0).round(), words_w, z + 3, &lines)?;
                let grip = Px::new(grip_w as u32, mark as u32).layer(kit, z + 3, Props::at(f.x + area.w - kit.space(3) - grip_w, f.y + ((row_h - mark) / 2.0).round()))?;
                Ok((tile_px, words, grip))
            })?;
            // the marks are pointer targets too: up, down and remove
            let gx = f.x + area.w - kit.space(3) - grip_w;
            let gy = f.y + ((row_h - mark) / 2.0).round();
            let marks = [Target::EditMove(i, -1).id(), Target::EditMove(i, 1).id(), Target::EditRemove(i).id()];
            for (k, id) in marks.iter().enumerate() {
                let mf = Frame::new(gx + k as f32 * (mark + kit.space(1)), gy, mark, mark);
                engine.focus.register(*id, if fits { mf } else { zero() }, REGION_LIST, SCOPE);
            }
            rows.push(EditRow { id, plate, _tile: tile_px, words, grip, marks, top });
        }
        // the way to put one back
        let add_top = n as f32 * (row_h + spacing);
        tops.push(add_top);
        let add_id = Target::EditAdd.id();
        let f = Frame::new(area.x, area.y + add_top, area.w, row_h);
        let fits = scroll.fits(add_top, row_h, 0.0);
        let add_plate = Self::make_plate(kit, engine, &scroll.inner, add_id, if fits { f } else { zero() }, f, REGION_LIST, SCOPE, z + 1, Lift::Row, Radius::Row)?;
        let radius = kit.tokens.radius(Radius::Row);
        let parts = scene.adopting(&scroll.inner, || -> std::io::Result<_> {
            let outline = plate::shape(&kit.ui, f, z, plate::ring(wash_edge(kit), radius, 1.0));
            let plus = icon::layer(kit, "plus", 28, ink2, z + 2, Props::at(f.x + kit.space(4), f.y + ((row_h - 28.0) / 2.0).round()))?.ok_or_else(|| std::io::Error::other("no plus icon"))?;
            let lh = kit.line(TypeRole::Label);
            let words = Words::new(kit, f.x + kit.space(4) + 28.0 + kit.space(3), f.y + ((row_h - lh) / 2.0).round(), area.w - 80.0, z + 2, &[WordLine::new("Add a tile", TypeRole::Label, ink)])?;
            Ok((outline, plus, words))
        })?;
        let gx = area.w - kit.space(3) - grip_w;
        let gy = ((row_h - mark) / 2.0).round();
        let mark_rel = [0, 1, 2].map(|k| Frame::new(gx + k as f32 * (mark + kit.space(1)), gy, mark, mark));
        let mut me = EditParts { rows, add_id, add_plate, _add: parts, scroll, tops, row_h, x0: area.x, grips: None, mark_rel };
        Self::paint_grips(kit, &mut me, e);
        Ok(me)
    }

    /// Paints the marks of every row: the up and down marks light for the tile that is picked up, the cross for the row whose remove button
    /// has the focus.
    fn paint_grips(kit: &mut Kit, p: &mut EditParts, e: &EditView) {
        let state = (e.index, e.grab, e.on_remove);
        if p.grips == Some(state) {
            return;
        }
        p.grips = Some(state);
        let ink2 = kit.ui.look.ink2();
        let accent = kit.ui.look.accent();
        let accent_ink = kit.ui.look.accent_ink();
        let mark = (kit.tokens.target * 0.75).round();
        let radius = kit.tokens.radius(Radius::Control);
        let gap = kit.space(1);
        let side = (mark * 0.5).round() as u32;
        for (i, row) in p.rows.iter_mut().enumerate() {
            let focused = i == e.index;
            let mut px = Px::new(row.grip.width(), row.grip.height());
            for k in 0..3u32 {
                let x = k as f32 * (mark + gap);
                let lit = if k == 2 { focused && e.on_remove } else { focused && e.grab };
                let (fill, edge) = if lit { (accent, None) } else { (selected_wash(kit), Some((wash_edge(kit), 1.0))) };
                px.round_rect(Frame::new(x, 0.0, mark, mark), radius, fill, edge);
                let ink = if lit { accent_ink } else { ink2 };
                let name = if k == 2 { "x" } else { "chevron-left" };
                if let Some(mut g) = kit.icons.draw(name, side, ink) {
                    // chevron-left turned a quarter clockwise points up, three quarters down
                    for _ in 0..[1, 3, 0][k as usize] {
                        g = rotate_cw(&g, side);
                    }
                    px.lay(&g, side, side, (x + (mark - side as f32) / 2.0) as i32, ((mark - side as f32) / 2.0) as i32);
                }
            }
            px.paint(&mut row.grip);
        }
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root_group: &Group, z: i32, view: &View, rows: Vec<RowLayout>, sig: String) -> std::io::Result<Built> {
        let scene = kit.ui.scene.clone();
        let sheet = lay::sheet();
        let pad = kit.space(6);
        let inner = sheet.w - 2.0 * pad;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        scene.adopting_view(root_group, || -> std::io::Result<Built> {
            let pane = Pane::new(kit, sheet, z + 1, PaneKind::Sheet, view.over_video);
            // a click on nothing is Back: the whole screen is a focusable under everything else, absent from the keys
            let backdrop = Target::Backdrop.id();
            engine.focus.register(backdrop, Frame::new(0.0, 0.0, 1920.0, 1080.0), REGION_LIST, SCOPE);
            engine.focus.set_enabled(backdrop, false);
            // ---- the head
            let hl = lay::head_layout(kit, view, inner);
            let title_lines: Vec<WordLine> = hl.title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
            let title = Words::new(kit, sheet.x + pad, sheet.y + pad, hl.title_w, z + 3, &title_lines)?;
            let clock = match &hl.clock {
                Some((text, x, y)) => {
                    // a box as wide as the widest time, the words at its right: the minute that changes moves nothing
                    let style = role_style(kit, TypeRole::Label, None, ink2, Align::Right);
                    let w = width_of(kit, style, "12:59 AM") + 2.0;
                    let right = sheet.x + pad + x + width_of(kit, style, text) + 2.0;
                    Some(Words::new(kit, right - w, sheet.y + pad + y, w, z + 3, &[WordLine::new(text, TypeRole::Label, ink2).aligned(Align::Right)])?)
                }
                None => None,
            };
            let mut hint_rows = Vec::new();
            let mut hx = sheet.x + pad + hl.hints_at.0;
            let side = glyph_side(kit, TypeRole::Label) as f32;
            for h in &hl.hints {
                let row = HintRow::new(kit, h, TypeRole::Label, RowAt { x: hx, y: sheet.y + pad + hl.hints_at.1, h: side, z: z + 3 }, ink2)?;
                hx += row.width() + kit.space(5);
                hint_rows.push(row);
            }
            let head_h = hl.h;
            let head = Head { title, clock, _hints: hint_rows };
            let mut cursor = sheet.y + pad + head_h + kit.space(4);
            let sheet_bottom = sheet.y + sheet.h - pad;
            let (mut strip, mut list, mut edit, mut pad_parts, mut foot) = (Vec::new(), None, None, None, None);
            if let Some(p) = &view.pad {
                pad_parts = Some(Self::build_pad(kit, engine, root_group, p, Frame::new(sheet.x + pad, cursor, inner, sheet_bottom - cursor), z)?);
            } else if let Some(e) = view.edit.as_ref().filter(|_| view.editing) {
                edit = Some(Self::build_edit(kit, engine, root_group, e, Frame::new(sheet.x + pad, cursor, inner, sheet_bottom - cursor), z)?);
            } else {
                // the foot: "All settings" and what the keys do
                let btn_h = Button::height(kit);
                let hints_h = if view.hints_foot.is_empty() { 0.0 } else { kit.line(TypeRole::Caption) };
                let foot_h = btn_h + if hints_h > 0.0 { hints_h + kit.space(2) } else { 0.0 };
                let foot_y = sheet_bottom - foot_h;
                let place = Place { id: Target::Foot.id(), frame: Frame::new(sheet.x + pad, foot_y, inner, btn_h), region: REGION_FOOT, scope: SCOPE, z: z + 3 };
                let button = Button::new(kit, engine, place, Spec::new("All settings", Tone::Plain).with_icon("settings"))?;
                let mut hint_rows = Vec::new();
                if hints_h > 0.0 {
                    let gap = kit.space(7);
                    let total = hints_width(kit, &view.hints_foot, TypeRole::Caption, gap);
                    let mut x = sheet.x + pad + ((inner - total) / 2.0).round();
                    let cs = glyph_side(kit, TypeRole::Caption) as f32;
                    for h in &view.hints_foot {
                        let row = HintRow::new(kit, h, TypeRole::Caption, RowAt { x, y: foot_y + btn_h + kit.space(2), h: cs.max(hints_h), z: z + 3 }, ink2)?;
                        x += row.width() + gap;
                        hint_rows.push(row);
                    }
                }
                foot = Some(Foot { button, _hints: hint_rows });
                if let Some(people) = &view.strip {
                    let (cells, h) = Self::build_strip(kit, engine, root_group, people, Frame::new(sheet.x + pad, cursor, inner, 0.0), z)?;
                    strip = cells;
                    cursor += h + kit.space(2);
                }
                let window = Frame::new(sheet.x + pad, cursor, inner, (foot_y - kit.space(4) - cursor).max(1.0));
                list = Some(Self::build_list(kit, engine, root_group, &view.rows, rows, window, z)?);
            }
            Ok(Built { sig, pane, head, backdrop, strip, list, edit, pad: pad_parts, foot, over_video: view.over_video })
        })
    }

    fn popup_signature(kit: &Kit, p: &PopupView) -> String {
        match p {
            PopupView::Slider { title, text, from, to, step, marks, icons, .. } => format!("slider|{}|{}|{title}|{text}|{from}|{to}|{step}|{marks:?}|{icons:?}", kit.generation(), kit.text_scale),
            PopupView::Pick { title, text, options, .. } => {
                format!("pick|{}|{}|{title}|{text}|{:?}", kit.generation(), kit.text_scale, options.iter().map(|o| (&o.label, &o.icon, &o.detail, o.current)).collect::<Vec<_>>())
            }
        }
    }

    fn build_popup(kit: &mut Kit, engine: &mut Engine, root_group: &Group, z: i32, p: &PopupView, sig: String) -> std::io::Result<PopupBuilt> {
        let scene = kit.ui.scene.clone();
        let z = z + 20;
        let layout = lay::popup_layout(kit, p);
        let group = scene.group(Some(root_group.id()), GroupPose::IDENTITY).map_err(other)?;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let ink3 = kit.ui.look.ink3();
        let pad = kit.space(6);
        let f = layout.frame;
        let mut built = scene.adopting_view(&group, || -> std::io::Result<PopupBuilt> {
            let pane = Pane::new(kit, f, z, PaneKind::Card, false);
            let title_lines: Vec<WordLine> = layout.title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
            let title = Words::new(kit, f.x + pad, f.y + pad, layout.title_w, z + 2, &title_lines)?;
            let text = if layout.text.is_empty() {
                None
            } else {
                let lines: Vec<WordLine> = layout.text.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink2)).collect();
                Some(Words::new(kit, f.x + pad, f.y + pad + kit.line(TypeRole::Title) + kit.space(3), f.w - 2.0 * pad, z + 2, &lines)?)
            };
            let (mut marks_words, mut icons, mut box_shape) = (Vec::new(), Vec::new(), None);
            let face = match p {
                PopupView::Slider { from, to, value, marks, icons: ends, .. } => {
                    let s = layout.slider.clone().expect("a slider layout");
                    let bx = Frame::new(f.x + s.box_.x, f.y + s.box_.y, s.box_.w, s.box_.h);
                    box_shape = Some(plate::shape(&kit.ui, bx, z + 1, Shape { corner_radius: kit.tokens.radius(Radius::Row), fill: Fill::Solid(focus_wash(kit)), border_width: 1.0, border: lit_edge(kit) }));
                    for (k, m) in marks.iter().enumerate() {
                        let al = if k == 0 { Align::Left } else if k == 2 { Align::Right } else { Align::Centre };
                        marks_words.push(Words::new(kit, f.x + s.marks.x + k as f32 * s.mark_w, f.y + s.marks.y, s.mark_w, z + 2, &[WordLine::new(m, TypeRole::Caption, ink3).aligned(al)])?);
                    }
                    for (k, name) in ends.iter().enumerate() {
                        let fr = if k == 0 { s.icon_l } else { s.icon_r };
                        if let Some(i) = icon::layer(kit, name, 26, ink3, z + 2, Props::at(f.x + fr.x, f.y + fr.y))? {
                            icons.push(i);
                        }
                    }
                    let track = Frame::new(f.x + s.track.x, f.y + s.track.y, s.track.w, s.track.h);
                    let slider = Slider::new(kit, track, z + 2, *from, *to, *value);
                    let vstyle = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Right);
                    let vw = layout.value.as_ref().map_or(80.0, |v| width_of(kit, vstyle, "100").max(v.1 + 2.0));
                    let value_line = Line::new(kit, Frame::new(f.x + f.w - pad - vw, f.y + pad, vw, kit.line(TypeRole::Title)), z + 2, vstyle, &format!("{}", value.round() as i64))?;
                    PopupFace::Slider { slider, value: value_line, shown: *value }
                }
                PopupView::Pick { options, .. } => {
                    let label_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
                    let mut rows = Vec::new();
                    for (i, o) in options.iter().enumerate() {
                        let of = layout.options[i];
                        let rf = Frame::new(f.x + of.x, f.y + of.y, of.w, of.h);
                        let id = Target::Option(i).id();
                        let plate_parts = Self::make_plate(kit, engine, &group, id, rf, rf, REGION_POPUP, POPUP_SCOPE, z + 1, Lift::Row, Radius::Row)?;
                        let x = kit.space(4) + if o.icon.is_empty() { 0.0 } else { 28.0 + kit.space(4) };
                        let icon_layer = if o.icon.is_empty() { None } else { icon::layer(kit, &o.icon, 28, ink2, z + 2, Props::at(rf.x + kit.space(4), rf.y + ((rf.h - 28.0) / 2.0).round()))? };
                        let lines = wrap::lines(kit, label_style, &o.label, rf.w - x - 120.0, 4);
                        let wl: Vec<WordLine> = lines.iter().map(|t| WordLine::new(t, TypeRole::Label, ink).weight(Weight::SemiBold)).collect();
                        let wh = Words::height_of(kit, &wl);
                        let words = Words::new(kit, rf.x + x, rf.y + ((rf.h - wh) / 2.0).round(), rf.w - x - 120.0, z + 2, &wl)?;
                        let (detail, tick) = if o.current {
                            (None, icon::layer(kit, "check", 28, kit.ui.look.accent(), z + 2, Props::at(rf.x + rf.w - kit.space(4) - 28.0, rf.y + ((rf.h - 28.0) / 2.0).round()))?)
                        } else if !o.detail.is_empty() {
                            let dw = width_of(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Right), &o.detail) + 2.0;
                            let w = Words::new(kit, rf.x + rf.w - kit.space(4) - dw, rf.y + ((rf.h - kit.line(TypeRole::Caption)) / 2.0).round(), dw, z + 2, &[WordLine::new(&o.detail, TypeRole::Caption, ink2).aligned(Align::Right)])?;
                            (Some(w), None)
                        } else {
                            (None, None)
                        };
                        rows.push(PickRow { plate: plate_parts, _icon: icon_layer, _words: words, _detail: detail, _tick: tick });
                    }
                    PopupFace::Pick { rows }
                }
            };
            Ok(PopupBuilt { sig, _group: None, _pane: pane, _title: title, _text: text, face, layout: layout.clone(), _box: box_shape, _marks: marks_words, _icons: icons })
        })?;
        built._group = Some(group);
        Ok(built)
    }

    // ---- tearing down

    fn drop_popup(&mut self, engine: &mut Engine) {
        if let Some(p) = self.popup.take() {
            if let PopupFace::Pick { rows } = p.face {
                for r in rows {
                    r.plate.destroy(engine);
                }
            }
        }
        self.popup_hover.hide();
        if self.popup_modal {
            engine.focus.close_modal();
            self.popup_modal = false;
        }
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        let Some(b) = self.built.take() else { return };
        engine.focus.unregister(b.backdrop);
        for c in b.strip {
            c.plate.destroy(engine);
        }
        if let Some(l) = b.list {
            for r in l.rows {
                r.plate.destroy(engine);
            }
        }
        if let Some(e) = b.edit {
            for r in e.rows {
                r.plate.destroy(engine);
                for m in r.marks {
                    engine.focus.unregister(m);
                }
            }
            e.add_plate.destroy(engine);
        }
        if let Some(p) = b.pad {
            for c in p.cells {
                c.plate.destroy(engine);
            }
        }
        if let Some(f) = b.foot {
            f.button.destroy(engine);
        }
    }

    /// Everything goes: the panel is gone (or its look changed and it is made again).
    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_popup(engine);
        self.drop_built(engine);
        self.root = None;
        self.scrim = None;
        self.hover = Hover::new(self.z + 4);
        self.popup_hover = Hover::new(self.z + 24);
        if self.modal {
            engine.focus.close_modal();
            self.modal = false;
        }
        self.first_focus = true;
        self.shake_due = None;
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }
}

/// How long the panel slides: its own beat, or the fade of reduced motion.
fn slide_ms(kit: &Kit, ms: u32) -> u32 {
    if kit.ui.look.reduced_motion { kit.beat_ms(crate::kit::motion::Beat::Label) } else { ms }
}

fn set_cell(cell: &Group, visible: bool, enabled: bool, disabled: f32) {
    cell.set_pose(GroupPose { visible, opacity: if enabled { 1.0 } else { disabled }, ..GroupPose::IDENTITY });
}

impl QuickSurface {
    // ---- applying

    /// Fits the panel to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, phase: Phase, now_ms: u64, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        if self.root.is_none() {
            self.root = Some(Root::new(kit, Arrive::Right(lay::TRAVEL), phase == Phase::Open)?);
            self.scrim = Some(scrim(kit, self.z, false));
        }
        let width = lay::SHEET_W - 2.0 * kit.space(6);
        let layouts: Vec<RowLayout> = if view.editing || view.pad.is_some() { Vec::new() } else { view.rows.iter().map(|r| lay::row_layout(kit, r, width)).collect() };
        let sig = Self::signature(kit, view, &layouts);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            let root_group = &self.root.as_ref().expect("a root").group;
            let built = Self::build(kit, engine, root_group, self.z, view, layouts, sig)?;
            if kit.ui.scene.take_overflow() {
                drop(built);
                return Err(std::io::Error::other("the scene is full: the quick panel could not be drawn whole"));
            }
            self.built = Some(built);
            // the focused row of what was just made is lifted at once, not flown to
            self.first_focus = true;
        }
        self.apply_popup(kit, engine, view, phase)?;
        let (enter, exit) = (slide_ms(kit, super::life::ENTER_MS), slide_ms(kit, super::life::EXIT_MS));
        match phase {
            Phase::Opening | Phase::Open => {
                if !self.modal {
                    engine.focus.open_modal(SCOPE);
                    self.modal = true;
                    self.first_focus = true;
                }
                if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
                    root.show(enter);
                }
                if let Some(s) = &self.scrim {
                    scrim_set(s, true);
                }
                self.update(kit, engine, view, now_ms, now_ns);
            }
            Phase::Leaving => {
                if let Some(root) = self.root.as_mut().filter(|r| r.is_shown()) {
                    root.hide(exit);
                }
            }
            Phase::Parked => {
                self.drop_popup(engine);
                if let Some(root) = self.root.as_mut() {
                    root.park();
                }
                if let Some(s) = &self.scrim {
                    scrim_set(s, false);
                }
                if self.modal {
                    engine.focus.close_modal();
                    self.modal = false;
                }
            }
            Phase::Gone => {}
        }
        Ok(true)
    }

    fn apply_popup(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, phase: Phase) -> std::io::Result<()> {
        let up = matches!(phase, Phase::Opening | Phase::Open);
        let Some(p) = view.popup.as_ref().filter(|_| up) else {
            if self.popup.is_some() {
                self.drop_popup(engine);
            }
            return Ok(());
        };
        let sig = Self::popup_signature(kit, p);
        if self.popup.as_ref().is_none_or(|c| c.sig != sig) {
            self.drop_popup(engine);
            let root_group = &self.root.as_ref().expect("a root").group;
            let built = Self::build_popup(kit, engine, root_group, self.z, p, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the card could not be drawn whole"));
            }
            self.popup = Some(built);
            engine.focus.open_modal(POPUP_SCOPE);
            self.popup_modal = true;
            self.first_focus = true;
        }
        // the slider follows the value
        if let (Some(PopupBuilt { face: PopupFace::Slider { slider, value, shown }, .. }), PopupView::Slider { value: v, .. }) = (self.popup.as_mut(), p)
            && (*v - *shown).abs() > f64::EPSILON
        {
            slider.set(kit, *v);
            value.text.set(kit, &format!("{}", v.round() as i64));
            *shown = *v;
        }
        Ok(())
    }

    fn focus_id(view: &View) -> Option<Id> {
        match view.focus.as_ref()? {
            Focus::Strip(i) => Some(Target::Strip(*i).id()),
            Focus::Row(i) => Some(Target::Row(*i).id()),
            Focus::Foot => Some(Target::Foot.id()),
            Focus::EditRow(i) => Some(Target::EditRow(*i).id()),
            Focus::EditAdd => Some(Target::EditAdd.id()),
            Focus::Option(i) => Some(Target::Option(*i).id()),
            Focus::Pad(i) => Some(Target::Pad(*i).id()),
        }
    }

    /// What changes without changing the layout, and the focus.
    fn update(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ms: u64, now_ns: u64) {
        let focus_row = if let Some(Focus::Row(i)) = view.focus { Some(i) } else { None };
        let disabled = kit.tokens.states.disabled_opacity;
        let mut shake = None;
        if let Some(b) = self.built.as_mut() {
            if b.over_video != view.over_video {
                b.pane.restyle(kit, view.over_video);
                b.over_video = view.over_video;
            }
            if let Some(c) = b.head.clock.as_mut()
                && !view.clock.is_empty()
            {
                c.set(kit, 0, &view.clock);
            }
            if let Some(list) = b.list.as_mut() {
                for (i, (row, parts)) in view.rows.iter().zip(list.rows.iter_mut()).enumerate() {
                    if parts.enabled != row.enabled {
                        parts.enabled = row.enabled;
                        set_cell(&parts.cell, parts.visible, row.enabled, disabled);
                        engine.focus.set_enabled(parts.id, row.enabled);
                    }
                    match &mut parts.ctrl {
                        Ctrl::Toggle(t) => t.set(kit, row.on),
                        Ctrl::Stepper(s) => {
                            if let Some(v) = lay::below_text(row) {
                                s.set_value(kit, &v);
                            }
                            s.set_focused(kit, focus_row == Some(i));
                        }
                        _ => {}
                    }
                }
            }
            if let (Some(edit), Some(e)) = (b.edit.as_mut(), view.edit.as_ref()) {
                Self::paint_grips(kit, edit, e);
            }
            if let (Some(pad), Some(p)) = (b.pad.as_mut(), view.pad.as_ref()) {
                if p.pin.filled != pad.filled {
                    pad.filled = p.pin.filled;
                    for (i, (dot, f)) in pad.dots.iter().enumerate() {
                        dot.reshape(Self::dot_shape(kit, i < p.pin.filled, f.w));
                    }
                }
                pad.note.set(kit, 0, &p.pin.note);
                if p.pin.shake != pad.shake {
                    pad.shake = p.pin.shake;
                    shake = Some(0);
                }
            }
        }
        if let Some(step) = shake {
            self.shake_step(kit, step, now_ms);
        }
        // the focus follows the model's; the first one after the panel comes up is lifted at once
        let want = Self::focus_id(view);
        match want {
            Some(id) if self.first_focus => {
                if engine.focus.focus(id) {
                    let _ = engine.flow.snap(id, true, now_ns);
                    self.first_focus = false;
                }
            }
            w => follow(engine, w, now_ns),
        }
        // the list moves to show the focused row whole
        match &view.focus {
            Some(Focus::Row(i)) => self.reveal_row(engine, *i, now_ms),
            Some(Focus::EditRow(i)) => self.reveal_edit(engine, *i),
            Some(Focus::EditAdd) => {
                let n = self.built.as_ref().and_then(|b| b.edit.as_ref()).map_or(0, |e| e.rows.len());
                self.reveal_edit(engine, n);
            }
            _ => {}
        }
    }

    fn shake_step(&mut self, kit: &Kit, step: usize, now_ms: u64) {
        let Some(pad) = self.built.as_mut().and_then(|b| b.pad.as_mut()) else { return };
        let off = SHAKE[step.min(SHAKE.len() - 1)];
        let ms = if kit.ui.look.reduced_motion { 0 } else { SHAKE_STEP_MS as u32 };
        for (dot, f) in &pad.dots {
            let to = Props::at(f.x + if kit.ui.look.reduced_motion { 0.0 } else { off }, f.y);
            if ms == 0 { dot.set(to) } else { dot.animate(to, ms, Ease::Out) }
        }
        self.shake_due = (step + 1 < SHAKE.len() && !kit.ui.look.reduced_motion).then_some((now_ms + SHAKE_STEP_MS, step + 1));
    }

    /// Moves the list so that the focused row shows whole, on the focus's spring, and puts the registry's frames where the rows will be.
    fn reveal_row(&mut self, engine: &mut Engine, i: usize, now_ms: u64) {
        let Some(list) = self.built.as_mut().and_then(|b| b.list.as_mut()) else { return };
        let Some(r) = list.rows.get(i) else { return };
        let target = list.scroll.reveal(r.top, r.layout.h, &list.tops);
        if (target - list.scroll.offset()).abs() < 0.5 {
            return;
        }
        let old = list.scroll.offset();
        list.scroll.to(engine, target, false);
        let window = list.scroll.window();
        for r in &mut list.rows {
            let was = list.scroll.fits(r.top, r.layout.h, old);
            let will = list.scroll.fits(r.top, r.layout.h, target);
            // both stay drawn while it moves; the ones that will not fit go when it has landed
            r.visible = was || will;
            engine.focus.place(r.id, if will { Frame::new(list.x0, window.y + r.top - target, window.w, r.layout.h) } else { zero() });
        }
        list.settle_due = Some(now_ms + SETTLE_MS);
    }

    fn reveal_edit(&mut self, engine: &mut Engine, i: usize) {
        let Some(edit) = self.built.as_mut().and_then(|b| b.edit.as_mut()) else { return };
        let Some(top) = edit.tops.get(i).copied() else { return };
        let target = edit.scroll.reveal(top, edit.row_h, &edit.tops);
        if (target - edit.scroll.offset()).abs() < 0.5 {
            return;
        }
        edit.scroll.to(engine, target, false);
        let window = edit.scroll.window();
        for r in &edit.rows {
            let will = edit.scroll.fits(r.top, edit.row_h, target);
            let f = Frame::new(edit.x0, window.y + r.top - target, window.w, edit.row_h);
            engine.focus.place(r.id, if will { f } else { zero() });
            for (k, id) in r.marks.iter().enumerate() {
                let m = edit.mark_rel[k];
                engine.focus.place(*id, if will { Frame::new(f.x + m.x, f.y + m.y, m.w, m.h) } else { zero() });
            }
        }
        let top = edit.tops[edit.tops.len() - 1];
        let will = edit.scroll.fits(top, edit.row_h, target);
        engine.focus.place(edit.add_id, if will { Frame::new(edit.x0, window.y + top - target, window.w, edit.row_h) } else { zero() });
    }

    // ---- time

    /// The spring has landed: the rows that do not fit whole are taken away; the shake goes on. Returns whether anything changed.
    pub fn tick(&mut self, kit: &Kit, now_ms: u64) -> bool {
        let mut changed = false;
        let disabled = kit.tokens.states.disabled_opacity;
        if let Some(list) = self.built.as_mut().and_then(|b| b.list.as_mut())
            && list.settle_due.is_some_and(|d| now_ms >= d)
        {
            list.settle_due = None;
            let offset = list.scroll.offset();
            for r in &mut list.rows {
                let fits = list.scroll.fits(r.top, r.layout.h, offset);
                if r.visible != fits {
                    r.visible = fits;
                    set_cell(&r.cell, fits, r.enabled, disabled);
                    changed = true;
                }
            }
            // the rows are only taken away here; the registry's frames were put at the target when the move started
        }
        if let Some((due, step)) = self.shake_due
            && now_ms >= due
        {
            self.shake_step(kit, step, now_ms);
            changed = true;
        }
        changed
    }

    pub fn due_at(&self) -> Option<u64> {
        let settle = self.built.as_ref().and_then(|b| b.list.as_ref()).and_then(|l| l.settle_due);
        [settle, self.shake_due.map(|(d, _)| d)].into_iter().flatten().min()
    }

    // ---- the pointer

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.pointer.key(engine);
        self.hover.hide();
        self.popup_hover.hide();
        if let Some(f) = self.built.as_mut().and_then(|b| b.foot.as_mut()) {
            f.button.set_hover(kit, false);
        }
    }

    /// The radius of the cue over a target.
    fn cue_radius(kit: &Kit, t: Target) -> Option<f32> {
        match t {
            Target::Row(_) | Target::EditRow(_) | Target::EditAdd | Target::Option(_) => Some(kit.tokens.radius(Radius::Row)),
            Target::Strip(_) => Some(kit.tokens.radius(Radius::Tile)),
            Target::Pad(_) => Some(kit.tokens.radius(Radius::Control)),
            Target::Foot | Target::EditMove(..) | Target::EditRemove(_) | Target::Backdrop => None,
        }
    }

    fn hover_cue(&mut self, kit: &Kit, engine: &Engine, from: Option<Id>, to: Option<Id>) {
        let foot_id = Target::Foot.id();
        if let Some(f) = self.built.as_mut().and_then(|b| b.foot.as_mut()) {
            if from == Some(foot_id) {
                f.button.set_hover(kit, false);
            }
            if to == Some(foot_id) && engine.focus.current() != Some(foot_id) {
                f.button.set_hover(kit, true);
            }
        }
        let target = to.and_then(Target::from_id).filter(|_| to != engine.focus.current());
        let radius = target.and_then(|t| Self::cue_radius(kit, t));
        let frame = to.and_then(|id| engine.focus.frame(id));
        let popup = matches!(target, Some(Target::Option(_)));
        let shown = radius.zip(frame);
        let (Some(root), false) = (self.root.as_ref(), shown.is_none()) else {
            self.hover.hide();
            self.popup_hover.hide();
            return;
        };
        if popup {
            self.hover.hide();
            let group: &Group = self.popup.as_ref().and_then(|p| p._group.as_ref()).unwrap_or(&root.group);
            self.popup_hover.show(kit, group, shown.map(|(r, f)| (f, r)));
        } else {
            self.popup_hover.hide();
            self.hover.show(kit, &root.group, shown.map(|(r, f)| (f, r)));
        }
    }

    /// Which way a click on a stepper or a nudge steps it: -1 on the left half of the control, 1 on the right.
    fn side_of(&self, engine: &Engine, t: Target, x: f32, y: f32) -> Option<i32> {
        let Target::Row(i) = t else { return None };
        let part = self.built.as_ref()?.list.as_ref()?.rows.get(i)?;
        let b = part.layout.below.as_ref()?;
        let at = engine.focus.frame(part.id)?;
        let (bx, by) = (at.x + b.frame.x, at.y + b.frame.y);
        if x < bx || x >= bx + b.frame.w || y < by || y >= by + b.frame.h {
            return None;
        }
        Some(if x < bx + b.frame.w / 2.0 { -1 } else { 1 })
    }

    /// A sample of the pointer: the hover cue follows it, a press shrinks "All settings", a click is the target to act on (with the side of
    /// a stepper's control it was on).
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<(Target, Option<i32>)> {
        self.root.as_ref()?;
        let foot_id = Target::Foot.id();
        let on_dim = self.built.as_ref().is_some_and(|b| s.visible && engine.focus.hit(s.x, s.y) == Some(b.backdrop));
        let dim_click = self.dim.feed(on_dim, &s);
        let mut click = None;
        for g in self.pointer.feed(engine, now_ns, s) {
            match g {
                Gesture::Hover { from, to } => self.hover_cue(kit, engine, from, to),
                Gesture::Down(id) if id == foot_id => {
                    if let Some(f) = self.built.as_mut().and_then(|b| b.foot.as_mut()) {
                        f.button.set_pressed(kit, true);
                    }
                }
                Gesture::Up(Some(id)) if id == foot_id => {
                    if let Some(f) = self.built.as_mut().and_then(|b| b.foot.as_mut()) {
                        f.button.set_pressed(kit, false);
                    }
                }
                Gesture::Click(id) => {
                    if let Some(t) = Target::from_id(id) {
                        click = Some((t, self.side_of(engine, t, s.x, s.y)));
                    }
                }
                _ => {}
            }
        }
        click.or(dim_click.then_some((Target::Backdrop, None)))
    }

    // ---- what the tests, the lint and the harness read

    /// How many layers the panel holds (the scene's budget).
    pub fn layers(&self) -> usize {
        let Some(b) = &self.built else { return usize::from(self.scrim.is_some()) };
        let rows = b.list.as_ref().map_or(0, |l| l.rows.iter().map(|r| r.layers).sum());
        let edit = b.edit.as_ref().map_or(0, |e| e.rows.len() * 5 + 5);
        let strip = b.strip.len() * 3;
        let pad = b.pad.as_ref().map_or(0, |p| 5 + p.cells.len() * 3);
        let foot = b.foot.as_ref().map_or(0, |f| f.button.layers() + f._hints.len() * 3);
        1 + b.head.title.layers() + usize::from(b.head.clock.is_some()) + b.head._hints.len() * 3 + rows + edit + strip + pad + foot + usize::from(self.scrim.is_some())
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let rows: Vec<Value> = b
            .list
            .iter()
            .flat_map(|l| l.rows.iter())
            .map(|r| {
                let fr = engine.focus.frame(r.id).unwrap_or_else(zero);
                json!({"id": r.row_id, "frame": f(fr), "h": r.layout.h, "stacked": r.layout.stacked, "beside": r.layout.beside, "label": r.layout.label.join(" "), "visible": r.visible})
            })
            .collect();
        let foot = b.foot.as_ref().map(|x| f(x.button.frame()));
        let strip: Vec<Value> = b.strip.iter().filter_map(|c| engine.focus.frame(c.id)).map(f).collect();
        let edit: Vec<Value> = b.edit.iter().flat_map(|e| e.rows.iter()).filter_map(|r| engine.focus.frame(r.id)).map(f).collect();
        let pad: Vec<Value> = b.pad.iter().flat_map(|p| p.cells.iter()).filter_map(|c| engine.focus.frame(c.id)).map(f).collect();
        json!({
            "sheet": f(lay::sheet()),
            "rows": rows,
            "foot": foot,
            "strip": strip,
            "edit": edit,
            "pad": pad,
            "popup": self.popup.as_ref().map(|p| f(p.layout.frame)),
            "offset": b.list.as_ref().map_or(0.0, |l| l.scroll.offset()),
            "window": b.list.as_ref().map(|l| f(l.scroll.window())),
            "focus": engine.focus.current().map(|i| i.0),
            "layers": self.layers(),
            "scope": SCOPE.0,
        })
    }

    /// Describes what is on the screen to a lint: the sheet and the card are panels, the words in them are theirs.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        let sheet = lint.panel("quick sheet", lay::sheet(), paint, false);
        b.head.title.feed_lint(kit, lint, "quick title", Some(sheet), (0.0, 0.0));
        if let Some(c) = &b.head.clock {
            c.feed_lint(kit, lint, "quick clock", Some(sheet), (0.0, 0.0));
        }
        if let Some(list) = &b.list {
            let offset = list.scroll.offset();
            for r in &list.rows {
                if !list.scroll.fits(r.top, r.layout.h, offset) {
                    continue;
                }
                let shift = (0.0, -offset);
                r.words.feed_lint(kit, lint, &format!("row {}", r.row_id), Some(sheet), shift);
                match &r.ctrl {
                    Ctrl::Stepper(s) => s.feed_lint(kit, lint, &format!("row {} value", r.row_id), Some(sheet), shift),
                    Ctrl::Chip(c) => c.feed_lint(kit, lint, &format!("row {}", r.row_id), shift, sheet),
                    Ctrl::Value { _text: Some(t), .. } => t.feed_lint(kit, lint, &format!("row {} value", r.row_id), Some(sheet), shift),
                    _ => {}
                }
            }
        }
        for (i, c) in b.strip.iter().enumerate() {
            c._name.feed_lint(kit, lint, &format!("profile {i}"), Some(sheet), (0.0, 0.0));
        }
        if let Some(e) = &b.edit {
            let offset = e.scroll.offset();
            for (i, r) in e.rows.iter().enumerate() {
                if e.scroll.fits(r.top, e.row_h, offset) {
                    r.words.feed_lint(kit, lint, &format!("edit row {i}"), Some(sheet), (0.0, -offset));
                }
            }
            if e.scroll.fits(e.tops[e.tops.len() - 1], e.row_h, offset) {
                e._add.2.feed_lint(kit, lint, "add a tile", Some(sheet), (0.0, -offset));
            }
        }
        if let Some(p) = &b.pad {
            p._name.feed_lint(kit, lint, "pad name", Some(sheet), (0.0, 0.0));
            p.note.feed_lint(kit, lint, "pad note", Some(sheet), (0.0, 0.0));
            for (i, c) in p.cells.iter().enumerate() {
                if let PadFace::Words(w) = &c._face {
                    let key = lint.panel_on(&format!("pad key {i}"), c.frame, kit.ui.look.colour(PaletteRole::Glass), false, sheet);
                    w.feed_lint(kit, lint, &format!("pad key {i}"), Some(key), (0.0, 0.0));
                }
            }
        }
        if let Some(f) = &b.foot {
            f.button.feed_lint(kit, lint, sheet);
        }
        if let Some(p) = &self.popup {
            let card = lint.panel("quick card", p.layout.frame, paint, false);
            p._title.feed_lint(kit, lint, "card title", Some(card), (0.0, 0.0));
            if let Some(t) = &p._text {
                t.feed_lint(kit, lint, "card text", Some(card), (0.0, 0.0));
            }
            for (i, m) in p._marks.iter().enumerate() {
                m.feed_lint(kit, lint, &format!("card mark {i}"), Some(card), (0.0, 0.0));
            }
            match &p.face {
                PopupFace::Slider { value, .. } => value.feed_lint(kit, lint, "card value", Some(card), (0.0, 0.0)),
                PopupFace::Pick { rows } => {
                    for (i, r) in rows.iter().enumerate() {
                        r._words.feed_lint(kit, lint, &format!("card option {i}"), Some(card), (0.0, 0.0));
                        if let Some(d) = &r._detail {
                            d.feed_lint(kit, lint, &format!("card option {i} detail"), Some(card), (0.0, 0.0));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::env::Env;
    use crate::panels::quick::Quick;
    use crate::panels::quick::tests::{feed, press, rig, two_people};
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: QuickSurface,
        q: Quick,
        env: Env,
        t: u64,
    }

    impl Up {
        fn new(mode: Mode, scale: f32, popups: bool) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let (q, env) = rig(popups);
            let mut me = Self { _h: h, kit, engine, s: QuickSurface::new(crate::kit::z::PANELS), q, env, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) -> bool {
            let view = self.q.view(&self.env, self.t);
            let phase = self.q.life.phase();
            self.s.apply(&mut self.kit, &mut self.engine, &view, phase, self.t, self.t * 1_000_000).expect("the panel is drawn")
        }

        fn key(&mut self, k: K) {
            press(&mut self.q, &mut self.env, k);
            self.t += 40;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }

        fn focused_row(&self) -> Option<usize> {
            (0..64).find(|i| self.engine.focus.current() == Some(Target::Row(*i).id()))
        }
    }

    #[test]
    fn the_sheet_is_560_wide_and_forty_pixels_from_the_right_edge_and_holds_the_rows_the_foot_and_the_glyphs() {
        let u = Up::new(Mode::Dark, 1.0, false);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["sheet"], json!({"x": 1320.0, "y": 56.0, "w": 560.0, "h": 968.0}));
        let rows = g["rows"].as_array().expect("rows");
        assert_eq!(rows.len(), u.q.rows(&u.env).len());
        for r in rows.iter().filter(|r| r["visible"] == true) {
            let f = &r["frame"];
            assert!(f["x"].as_f64().unwrap() >= 1320.0 + 24.0 - 0.5 && f["x"].as_f64().unwrap() + f["w"].as_f64().unwrap() <= 1880.0 - 24.0 + 0.5, "{r}");
        }
        assert!(g["foot"]["y"].as_f64().unwrap() > rows[0]["frame"]["y"].as_f64().unwrap(), "All settings is under the rows");
        assert_eq!(g["scope"], 200);
        assert_eq!(u.engine.focus.modal(), Some(SCOPE), "the panel takes the keys while it is up");
    }

    #[test]
    fn the_first_focus_is_lifted_at_once_and_the_next_flies_on_the_spring() {
        let mut u = Up::new(Mode::Dark, 1.0, false);
        let first = u.engine.focus.current().expect("a focus");
        assert_eq!(first, Target::Row(0).id());
        assert_eq!(u.engine.flow.in_flight(), 1, "the one lifted tile");
        assert!(!u.engine.flow.settling_at(u.t * 1_000_000), "snapped, not flown");
        assert!(u.engine.flow.lift_at(first, u.t * 1_000_000) > 0.99);
        u.key(K::Down);
        assert_eq!(u.focused_row(), Some(1));
        assert!(u.engine.flow.settling_at(u.t * 1_000_000), "the lift moves on the spring");
        assert_eq!(u.engine.flow.in_flight(), 2, "the one leaving and the one coming");
    }

    #[test]
    fn the_panel_reads_clean_in_both_materials_at_100_and_200_percent_text_down_the_whole_list() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale, true);
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
                for step in 0..14 {
                    u.key(K::Down);
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} after {step} downs: {f:#?}");
                }
            }
        }
    }

    #[test]
    fn the_list_moves_to_show_the_focused_row_whole_at_200_percent() {
        let mut u = Up::new(Mode::Dark, 2.0, false);
        let window = u.s.built.as_ref().and_then(|b| b.list.as_ref()).map(|l| l.scroll.window()).expect("a list");
        let n = u.q.rows(&u.env).len();
        for _ in 0..n {
            u.key(K::Down);
            let id = u.engine.focus.current().expect("a focus");
            let f = u.engine.focus.frame(id).expect("a frame");
            if u.focused_row().is_some() {
                assert!(f.y >= window.y - 0.5 && f.bottom() <= window.bottom() + 0.5, "the focused row shows whole: {f:?} in {window:?}");
            }
        }
        assert!(u.s.built.as_ref().and_then(|b| b.list.as_ref()).map_or(0.0, |l| l.scroll.offset()) > 0.0, "the list moved");
        assert!(u.s.due_at().is_some(), "the rows that do not fit are taken away when the spring has landed");
        u.t += 400;
        assert!(u.s.tick(&u.kit, u.t));
        assert!(u.s.due_at().is_none());
    }

    #[test]
    fn what_changes_without_changing_the_layout_is_changed_in_place() {
        let mut u = Up::new(Mode::Dark, 1.0, false);
        u.key(K::Down);
        let before = (u.s.built.as_ref().map(|b| b.sig.clone()), u.s.layers(), u.kit.room().layers);
        u.key(K::Right);
        u.key(K::Right);
        u.key(K::Left);
        let after = (u.s.built.as_ref().map(|b| b.sig.clone()), u.s.layers(), u.kit.room().layers);
        assert_eq!(before, after, "a stepper's value is text in a layer that is already there");
    }

    #[test]
    fn the_card_opens_beside_the_panel_in_its_own_scope_and_goes_with_back() {
        let mut u = Up::new(Mode::Dark, 1.0, true);
        u.key(K::Ok);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["popup"]["x"], 712.0);
        assert_eq!(g["popup"]["w"], 560.0);
        assert_eq!(u.engine.focus.modal(), Some(POPUP_SCOPE));
        assert_eq!(u.engine.focus.current(), Some(Target::Option(1).id()), "on the current choice");
        assert!(u.lint().is_empty(), "{:?}", u.lint());
        u.key(K::Down);
        assert_eq!(u.engine.focus.current(), Some(Target::Option(2).id()));
        u.key(K::Back);
        assert!(u.s.popup.is_none() && u.engine.focus.modal() == Some(SCOPE));
        assert_eq!(u.focused_row(), Some(0), "the focus is back on the tile it came from");
    }

    #[test]
    fn the_oled_slider_card_follows_the_value_in_place() {
        let mut u = Up::new(Mode::Dark, 1.0, true);
        u.env.theme = json!({"remote": {"quickPanelEdit": true}, "quickPanel": {"tiles": ["oled", "picture"]}});
        u.q.open(&mut u.env, 10);
        u.apply();
        u.key(K::Ok);
        let layers = u.kit.room().layers;
        assert!(u.s.popup.is_some());
        u.key(K::Right);
        u.key(K::Right);
        assert_eq!(u.kit.room().layers, layers, "the thumb, the fill and the figure move; nothing is made");
        assert!(u.lint().is_empty(), "{:?}", u.lint());
    }

    #[test]
    fn the_pin_pad_takes_the_place_of_the_list_and_shakes_on_a_wrong_pin() {
        let mut u = Up::new(Mode::Dark, 1.0, false);
        feed(&mut u.env, "profiles", two_people("isabel", &["nitin"]), 10);
        u.apply();
        assert_eq!(u.s.geometry(&u.engine)["strip"].as_array().map(Vec::len), Some(2));
        u.key(K::Up);
        u.key(K::Left);
        u.key(K::Ok);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["pad"].as_array().map(Vec::len), Some(12), "3 by 4 keys");
        assert!(g["strip"].as_array().is_some_and(Vec::is_empty) && g["rows"].as_array().is_some_and(Vec::is_empty));
        assert!(u.lint().is_empty(), "{:?}", u.lint());
        for d in 1..=3u8 {
            u.key(K::Digit(d));
        }
        assert_eq!(u.s.built.as_ref().and_then(|b| b.pad.as_ref()).map(|p| p.filled), Some(3));
        u.key(K::Digit(4));
        let wrong = crate::kit::screen::Input::Fetched { tag: crate::panels::env::tags::PROFILE_SWITCH, status: 403, data: json!({"error": {"code": "wrong_pin", "triesLeft": 4}}) };
        u.q.answer(&wrong, &mut u.env, u.t);
        u.apply();
        assert_eq!(u.s.built.as_ref().and_then(|b| b.pad.as_ref()).map(|p| p.filled), Some(0), "the dots are cleared");
        assert!(u.s.due_at().is_some(), "the shake goes on in steps");
        let mut steps = 0;
        while let Some(due) = u.s.due_at() {
            u.t = due;
            assert!(u.s.tick(&u.kit, u.t));
            steps += 1;
            assert!(steps < 20);
        }
        assert!(steps >= 3);
    }

    #[test]
    fn the_edit_list_lifts_its_rows_the_marks_are_targets_and_the_list_follows_the_focus() {
        let mut u = Up::new(Mode::Dark, 2.0, true);
        u.key(K::Down);
        u.q.held(&u.env);
        u.apply();
        let g = u.s.geometry(&u.engine);
        assert!(g["edit"].as_array().is_some_and(|e| e.len() > 3), "{g}");
        assert!(u.q.is_editing());
        assert!(u.lint().is_empty(), "{:?}", u.lint());
        for _ in 0..12 {
            u.key(K::Down);
        }
        assert!(u.s.built.as_ref().and_then(|b| b.edit.as_ref()).is_some_and(|e| e.scroll.offset() > 0.0));
        assert!(u.lint().is_empty(), "{:?}", u.lint());
        u.key(K::Back); // puts the picked-up tile back
        u.key(K::Back);
        assert!(!u.q.is_editing() && u.s.built.as_ref().is_some_and(|b| b.edit.is_none() && b.list.is_some()));
    }

    #[test]
    fn leaving_hides_in_place_parking_frees_the_keys_and_gone_frees_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, false);
        let with = u.kit.room().layers;
        u.q.close(&mut u.env, 2000);
        u.q.life.hide(2000);
        assert_eq!(u.q.life.phase(), Phase::Leaving);
        assert!(u.apply());
        assert_eq!(u.engine.focus.modal(), Some(SCOPE), "the keys stay with the panel while it slides out");
        u.q.life.tick(2200);
        assert_eq!(u.q.life.phase(), Phase::Parked);
        assert!(u.apply());
        assert_eq!(u.engine.focus.modal(), None);
        assert_eq!(u.kit.room().layers, with, "the layers are kept hidden for the next opening");
        assert!(u.s.root.as_ref().is_some_and(|r| r.group.is_inert()));
        u.q.life.tick(7000);
        assert_eq!(u.q.life.phase(), Phase::Gone);
        assert!(!u.apply());
        assert!(u.kit.room().layers > with + 40, "everything goes");
    }

    #[test]
    fn the_pointer_cues_on_hover_and_a_click_names_the_target_and_the_side_of_a_stepper() {
        let mut u = Up::new(Mode::Dark, 1.0, false);
        let f = u.engine.focus.frame(Target::Row(1).id()).expect("the picture row");
        let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
        let t = |n: u64| (2000 + n) * 1_000_000;
        assert!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(f.x + 40.0, f.y + 10.0, 0)).is_none());
        assert!(u.s.hover.is_shown(), "a cue under the pointer");
        assert!(u.s.pointer(&u.kit, &mut u.engine, t(10), at(f.x + 40.0, f.y + 10.0, 1)).is_none());
        let click = u.s.pointer(&u.kit, &mut u.engine, t(20), at(f.x + 40.0, f.y + 10.0, 0));
        assert_eq!(click, Some((Target::Row(1), None)));
        // over the foot button
        let foot = u.s.geometry(&u.engine)["foot"].clone();
        let (x, y) = (foot["x"].as_f64().unwrap() as f32 + 20.0, foot["y"].as_f64().unwrap() as f32 + 10.0);
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(x, y, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(x, y, 1));
        let click = u.s.pointer(&u.kit, &mut u.engine, t(50), at(x, y, 0));
        assert_eq!(click, Some((Target::Foot, None)));
        // a key hides the cue
        u.s.on_key(&u.kit, &mut u.engine);
        assert!(!u.s.hover.is_shown());
    }

    #[test]
    fn a_click_on_the_dim_beside_the_panel_is_the_backdrop() {
        let mut u = Up::new(Mode::Dark, 1.0, false);
        let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
        let t = |n: u64| (4000 + n) * 1_000_000;
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(300.0, 300.0, 0)), None);
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(10), at(300.0, 300.0, 1)), None);
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(300.0, 300.0, 0)), Some((Target::Backdrop, None)));
        // a press that began on the sheet and ended on the dim is no click
        let f = u.engine.focus.frame(Target::Row(1).id()).expect("a row");
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(f.x + 20.0, f.y + 10.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(40), at(300.0, 300.0, 0)), None);
    }

    #[test]
    fn the_panel_fits_the_scene_budget_with_every_tile_at_200_percent() {
        let u = Up::new(Mode::Light, 2.0, true);
        let room = u.kit.room();
        assert!(u.s.layers() < 120, "{} layers", u.s.layers());
        assert!(room.layers > 40, "the testkit starts with some room: {room:?}");
    }
}
