//! The context menu drawn (`docs/mocks/16-context-menu`, `17-power-menu`, `33-pointer-menu`; `panels/ContextMenu.qml`): a glass card that opens
//! beside what it belongs to (or where it is told), a head with the thing's icon, name and a line under it, one row per option and, when the
//! owner has a hint for it, the glyphs of the keys in place of its words. A row has an icon tile, its label with a detail line when there
//! is one, and at its right either a caption or, for a row with options (the sleep timer), a tray of segments: Left and Right move inside
//! it, a click picks the segment under the pointer. The thing the menu belongs to (a dock tile, a library icon) is drawn again above the dim
//! so it stays lit.
//!
//! The model decides which row (and which segment) has the focus. The rows are made again when the spec says something else; the focus and
//! the segment's plate move in place on the flowing focus.

use super::life::Phase;
use super::menu::{Item, MARGIN, Spec, View};
use super::stage::{Stage, Touch};
use super::ui::{Arrive, WordLine, Words, picture_layer};
use crate::kit::button::faded;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt, glyph_side};
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Px, Spot, glyph_of};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use crate::settings::surface::paint::Paint;
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};

pub const SCOPE: Scope = Scope(203);
const REGION: Region = Region(2300);
const BASE: u32 = 0x2300_0000;
/// The head's own icon (the tile of a thing the menu belongs to).
const HEAD_ICON: f32 = 48.0;

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Row(usize),
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        match self {
            Target::Row(i) => Id(BASE + i as u32),
            Target::Backdrop => Id(BASE + 0xFF00),
        }
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            0xFF00 => Some(Target::Backdrop),
            i if i < 0x100 => Some(Target::Row(i as usize)),
            _ => None,
        }
    }
}

/// The tray of a row with options: its place in the row, a segment's width and the labels.
#[derive(Clone, Debug, PartialEq)]
struct SegLay {
    tray: Frame,
    seg_w: f32,
    labels: Vec<String>,
}

/// One row laid out: everything inside it is placed from the row's own top left.
#[derive(Clone, Debug, PartialEq)]
struct RowLay {
    frame: Frame,
    tile: Option<Frame>,
    label: Vec<String>,
    detail: Vec<String>,
    words: Frame,
    hint: Option<(String, f32, f32, f32)>,
    seg: Option<SegLay>,
}

#[derive(Clone, Debug, PartialEq)]
struct HeadLay {
    frame: Frame,
    tile: Frame,
    title: String,
    sub: String,
    text_x: f32,
    text_w: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    card: Frame,
    head: Option<HeadLay>,
    rows: Vec<RowLay>,
    /// The hints' row: its top and height, none without a hint.
    foot: Option<(f32, f32)>,
}

fn layout(kit: &mut Kit, spec: &Spec) -> Layout {
    let (s1, s2, s3, s4, s5, s6) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(6));
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let (label_line, caption_line) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
    let pad = s3;
    let tile = (kit.tokens.target * 0.75).round();
    let target = kit.tokens.target.round();
    let base_h = (kit.tokens.row * 0.8).round();
    // the tray holds a line of captions with air above and below, which a large text size makes taller than the target
    let tray_h = target.max(caption_line + 2.0 * s2).ceil();
    let detail_h = kit.tokens.row + s2;

    // what each row needs at its right, and so how wide the card must be to hold a tray under the words
    struct Pre {
        text_x: f32,
        hint_w: f32,
        seg_w: f32,
        need: f32,
    }
    let mut pre = Vec::new();
    let mut widest_need = 0.0f32;
    for it in &spec.items {
        let text_x = s6 + if it.icon.is_empty() { 0.0 } else { tile + s5 };
        if it.options.is_empty() {
            let hint_w = if it.hint.is_empty() { 0.0 } else { width_of(kit, caption_style, &it.hint).ceil() };
            pre.push(Pre { text_x, hint_w, seg_w: 0.0, need: 0.0 });
        } else {
            let widest = it.options.iter().map(|o| width_of(kit, caption_style, o)).fold(0.0f32, f32::max).ceil();
            let seg_w = target.max(widest + 2.0 * s4 + 2.0 * s1).ceil();
            let need = seg_w * it.options.len() as f32;
            widest_need = widest_need.max(text_x + need + s6);
            pre.push(Pre { text_x, hint_w: 0.0, seg_w, need });
        }
    }
    let card_w = spec.width.max(widest_need + 2.0 * pad).min(1920.0 - 2.0 * MARGIN).round();
    let inner = card_w - 2.0 * pad;

    let head_h =
        if spec.has_head() { kit.tokens.row.max(kit.line(TypeRole::Label) + if spec.sub.is_empty() { 0.0 } else { caption_line } + s4).ceil() } else { 0.0 };
    let mut rows = Vec::new();
    let mut top = pad + head_h;
    for (it, p) in spec.items.iter().zip(&pre) {
        let label_w = width_of(kit, label_style, &it.label);
        let avail = inner - p.text_x - s6;
        let beside = p.need > 0.0 && label_w + s4 + p.need <= avail;
        let trail = if p.need > 0.0 { if beside { p.need } else { 0.0 } } else { p.hint_w };
        let text_w = (avail - if trail > 0.0 { trail + s4 } else { 0.0 }).max(1.0);
        let label = wrap::lines(kit, label_style, &it.label, text_w, 3);
        let detail = if it.detail.is_empty() { Vec::new() } else { wrap::lines(kit, caption_style, &it.detail, text_w, 2) };
        let words_h = label.len() as f32 * label_line + detail.len() as f32 * caption_line;
        let stacked = p.need > 0.0 && !beside;
        let want = if it.detail.is_empty() { base_h } else { detail_h };
        let h =
            if stacked { want.max(s3 + words_h + s2 + tray_h + s3) } else { want.max(words_h + s4).max(if p.need > 0.0 { tray_h + s2 } else { 0.0 }) }.ceil();
        let words_y = if stacked { s3 } else { ((h - words_h) / 2.0).round() };
        let seg = (p.need > 0.0).then(|| {
            let (x, y) = if beside { (inner - s6 - p.need, ((h - tray_h) / 2.0).round()) } else { (p.text_x, s3 + words_h + s2) };
            SegLay { tray: Frame::new(x, y, p.need, tray_h), seg_w: p.seg_w, labels: it.options.clone() }
        });
        let hint = (p.hint_w > 0.0).then(|| (it.hint.clone(), p.hint_w, inner - s6 - p.hint_w, ((h - caption_line) / 2.0).round()));
        rows.push(RowLay {
            frame: Frame::new(0.0, top, inner, h),
            tile: (!it.icon.is_empty()).then(|| Frame::new(s6, ((h - tile) / 2.0).round(), tile, tile)),
            label,
            detail,
            words: Frame::new(p.text_x, words_y, text_w, words_h),
            hint,
            seg,
        });
        top += h;
    }
    let foot_h = if spec.hint.is_empty() { 0.0 } else { (kit.tokens.row * 0.55).max(glyph_side(kit, TypeRole::Caption) as f32 + s2).round() };
    let card_h = top + foot_h + pad;
    let (x, y) = spec.place_card(card_w, card_h, s4, s1);
    let (x, y) = (x.round(), y.round());
    let card = Frame::new(x, y, card_w, card_h);
    for r in &mut rows {
        r.frame = Frame::new(x + pad, y + r.frame.y, inner, r.frame.h);
    }
    let head = spec.has_head().then(|| {
        let has_icon = !spec.icon.is_empty() || !spec.icon_url.is_empty();
        let text_x = s3 + if has_icon { HEAD_ICON + s4 } else { 0.0 };
        HeadLay {
            frame: Frame::new(x + pad, y + pad, inner, head_h),
            tile: Frame::new(s3, ((head_h - HEAD_ICON) / 2.0).round(), HEAD_ICON, HEAD_ICON),
            title: spec.title.clone(),
            sub: spec.sub.clone(),
            text_x,
            text_w: (inner - text_x - s3).max(1.0),
        }
    });
    Layout { card, head, rows, foot: (foot_h > 0.0).then_some((y + card_h - pad - foot_h, foot_h)) }
}

/// A rounded square holding an icon in `ink`.
fn tile_px(kit: &mut Kit, side: u32, name: &str, ink: Rgba) -> Px {
    let paint = Paint::of(kit);
    let mut px = Px::new(side, side);
    px.round_rect(Frame::new(0.0, 0.0, side as f32, side as f32), (side as f32 * 0.32).round(), paint.wash_selected, Some((paint.edge, 1.0)));
    let g = glyph_of(side);
    if let Some(icon) = kit.icons.draw(name, g, ink) {
        let o = ((side - g) / 2) as i32;
        px.lay(&icon, g, g, o, o);
    }
    px
}

struct SegParts {
    _tray: ShapeLayer,
    _chosen: Option<ShapeLayer>,
    focus: ShapeLayer,
    _labels: Vec<Words>,
}

struct RowParts {
    id: Id,
    plate: Plate,
    _tile: Option<ImageLayer>,
    words: Words,
    hint: Option<Words>,
    seg: Option<SegParts>,
    lay: RowLay,
    item: Item,
}

struct HeadParts {
    _tile: Option<ImageLayer>,
    title: Words,
    sub: Option<Words>,
}

struct RaisedParts {
    _plate: Option<ShapeLayer>,
    _picture: Option<ImageLayer>,
}

struct Built {
    sig: String,
    pane: Pane,
    head: Option<HeadParts>,
    rows: Vec<RowParts>,
    _hints: Vec<HintRow>,
    _raised: Option<RaisedParts>,
    over_video: bool,
    lay: Layout,
    layers: usize,
    /// Where the segment plate is now: (row, segment), none while it is hidden.
    seg_at: Option<(usize, usize)>,
}

pub struct MenuSurface {
    stage: Stage,
    built: Option<Built>,
    /// Whether the dim was made with the root (the spec says whether there is one).
    dimmed: Option<bool>,
}

impl MenuSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None, dimmed: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, spec: &Spec, lay: &Layout) -> String {
        format!("{}|{}|{:?}|{:?}", kit.generation(), kit.text_scale, spec, lay.card)
    }

    fn seg_frame(lay: &RowLay, seg: usize) -> Option<Frame> {
        lay.seg.as_ref().map(|s| Frame::new(lay.frame.x + s.tray.x + seg as f32 * s.seg_w, lay.frame.y + s.tray.y, s.seg_w, s.tray.h))
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, v: &View, lay: Layout, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let paint = Paint::of(kit);
        let (ink, ink2, ink3) = (paint.ink, paint.ink2, paint.ink3);
        let row_radius = kit.tokens.radius(Radius::Row);
        let spec = &v.spec;
        scene
            .adopting_view(root, || -> std::io::Result<Built> {
                let raised = match &spec.raised {
                    Some(r) => {
                        let f = r.frame;
                        let plate = r.plate.as_deref().map(|c| {
                            let fill = crate::kit::avatar::parse(c, paint.sheet);
                            let radius = r.radius.unwrap_or_else(|| kit.tokens.radius(Radius::Tile));
                            plate::shape(
                                &kit.ui,
                                f,
                                z + 1,
                                Shape { corner_radius: radius, fill: Fill::Solid(fill), border_width: 1.0, border: paint.edge_raised },
                            )
                        });
                        let inner = (f.w - 2.0 * r.inset).max(1.0);
                        let inner_h = (f.h - 2.0 * r.inset).max(1.0);
                        let picture =
                            picture_layer(kit, &r.icon_url, inner.round() as u32, inner_h.round() as u32, z + 2, Props::at(f.x + r.inset, f.y + r.inset))?;
                        Some(RaisedParts { _plate: plate, _picture: picture })
                    }
                    None => None,
                };
                let pane = Pane::new(kit, lay.card, z + 2, PaneKind::Card, v.over_video);
                let head = match &lay.head {
                    Some(h) => {
                        let f = h.frame;
                        let tile_layer = if !spec.icon_url.is_empty() {
                            picture_layer(kit, &spec.icon_url, HEAD_ICON as u32, HEAD_ICON as u32, z + 4, Props::at(f.x + h.tile.x, f.y + h.tile.y))?
                        } else if !spec.icon.is_empty() {
                            Some(tile_px(kit, HEAD_ICON as u32, &spec.icon, ink).layer(kit, z + 4, Props::at(f.x + h.tile.x, f.y + h.tile.y))?)
                        } else {
                            None
                        };
                        let (title_line, sub_line) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
                        let text_h = title_line + if h.sub.is_empty() { 0.0 } else { sub_line };
                        let y = f.y + ((f.h - text_h) / 2.0).round();
                        let title_text = wrap::elide(kit, role_style(kit, TypeRole::Label, None, ink, Align::Left), &h.title, h.text_w);
                        let title = Words::new(kit, f.x + h.text_x, y, h.text_w, z + 4, &[WordLine::new(&title_text, TypeRole::Label, ink)])?;
                        let sub = if h.sub.is_empty() {
                            None
                        } else {
                            let t = wrap::elide(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Left), &h.sub, h.text_w);
                            Some(Words::new(kit, f.x + h.text_x, y + title_line, h.text_w, z + 4, &[WordLine::new(&t, TypeRole::Caption, ink2)])?)
                        };
                        Some(HeadParts { _tile: tile_layer, title, sub })
                    }
                    None => None,
                };
                let mut rows = Vec::new();
                for (i, (it, rl)) in spec.items.iter().zip(lay.rows.iter()).enumerate() {
                    let id = Target::Row(i).id();
                    let f = rl.frame;
                    let plate_parts =
                        Plate::new(kit, engine, root, Spot { id, screen: f, local: f, region: REGION, scope: SCOPE, z: z + 3 }, Lift::Row, row_radius)?;
                    engine.focus.set_enabled(id, !it.disabled);
                    let dim = |c: Rgba| if it.disabled { faded(c, paint.disabled) } else { c };
                    let label_ink = dim(if it.danger { paint.danger_ink } else { ink });
                    let tile_layer = match rl.tile {
                        Some(t) => Some(tile_px(kit, t.w as u32, &it.icon, dim(if it.danger { paint.danger_ink } else { ink })).layer(
                            kit,
                            z + 4,
                            Props::at(f.x + t.x, f.y + t.y),
                        )?),
                        None => None,
                    };
                    let mut lines: Vec<WordLine> = rl.label.iter().map(|t| WordLine::new(t, TypeRole::Label, label_ink)).collect();
                    lines.extend(rl.detail.iter().map(|t| WordLine::new(t, TypeRole::Caption, dim(ink2))));
                    let words = Words::new(kit, f.x + rl.words.x, f.y + rl.words.y, rl.words.w, z + 4, &lines)?;
                    let hint = match &rl.hint {
                        Some((text, w, x, y)) => Some(Words::new(kit, f.x + x, f.y + y, w + 2.0, z + 4, &[WordLine::new(text, TypeRole::Caption, dim(ink2))])?),
                        None => None,
                    };
                    let seg = match &rl.seg {
                        Some(s) => {
                            let tray_at = Frame::new(f.x + s.tray.x, f.y + s.tray.y, s.tray.w, s.tray.h);
                            let control = kit.tokens.radius(Radius::Control);
                            let tray = plate::shape(&kit.ui, tray_at, z + 4, plate::solid(paint.tray(), control));
                            let chosen = (!it.options.is_empty()).then(|| {
                                let at = Frame::new(tray_at.x + it.index.min(s.labels.len() - 1) as f32 * s.seg_w, tray_at.y, s.seg_w, tray_at.h);
                                plate::shape(&kit.ui, at, z + 5, plate::solid(paint.seg_selected(), control))
                            });
                            let focus = kit.ui.scene.shape(
                                s.seg_w as u32,
                                tray_at.h as u32,
                                z + 5,
                                plate::solid(paint.seg_focus(), control),
                                crate::kit::widgets::HIDDEN,
                            );
                            let mut labels = Vec::new();
                            for (k, text) in s.labels.iter().enumerate() {
                                let line = kit.line(TypeRole::Caption);
                                labels.push(Words::new(
                                    kit,
                                    tray_at.x + k as f32 * s.seg_w,
                                    tray_at.y + ((tray_at.h - line) / 2.0).round(),
                                    s.seg_w,
                                    z + 6,
                                    &[WordLine::new(text, TypeRole::Caption, dim(ink)).aligned(Align::Centre)],
                                )?);
                            }
                            Some(SegParts { _tray: tray, _chosen: chosen, focus, _labels: labels })
                        }
                        None => None,
                    };
                    rows.push(RowParts { id, plate: plate_parts, _tile: tile_layer, words, hint, seg, lay: rl.clone(), item: it.clone() });
                }
                let mut hints = Vec::new();
                if let Some((y, h)) = lay.foot {
                    let mut x = lay.card.x + kit.space(3) + kit.space(6);
                    for hint in [Hint::key(Key::Ok, Action::Confirm), Hint::key(Key::Back, Action::Close)] {
                        let row = HintRow::new(kit, &hint, TypeRole::Caption, RowAt { x, y, h, z: z + 4 }, ink3)?;
                        x += row.width() + kit.space(6);
                        hints.push(row);
                    }
                }
                let layers = before.saturating_sub(kit.room().layers);
                Ok(Built { sig, pane, head, rows, _hints: hints, _raised: raised, over_video: v.over_video, lay: lay.clone(), layers, seg_at: None })
            })
            .map_err(std::io::Error::other)
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for r in b.rows {
                r.plate.destroy(engine);
            }
        }
    }

    /// Fits the menu to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let lay = layout(kit, &v.spec);
        if self.dimmed.is_some_and(|d| d != v.spec.scrim) {
            self.drop_all(engine);
        }
        if self.stage.needs_root(Arrive::Fade) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, Arrive::Fade, v.spec.scrim)?;
        self.dimmed = Some(v.spec.scrim);
        let sig = Self::signature(kit, &v.spec, &lay);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), v, lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the menu could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, phase) {
            if let Some(b) = self.built.as_mut() {
                if b.over_video != v.over_video {
                    b.pane.restyle(kit, v.over_video);
                    b.over_video = v.over_video;
                }
                Self::seg_focus(kit, b, v);
            }
            let want = v.spec.items.get(v.index).filter(|i| !i.disabled).map(|_| Target::Row(v.index).id());
            self.stage.focus(engine, want, now_ns);
        }
        Ok(true)
    }

    /// The plate under the focused segment follows the model: it is shown on the focused row of options and glides along it.
    fn seg_focus(kit: &Kit, b: &mut Built, v: &View) {
        let want = b.rows.get(v.index).filter(|r| r.seg.is_some()).map(|_| (v.index, v.seg));
        if want == b.seg_at {
            return;
        }
        if let Some((i, _)) = b.seg_at.filter(|(i, _)| Some(*i) != want.map(|w| w.0))
            && let Some(s) = b.rows.get(i).and_then(|r| r.seg.as_ref())
        {
            s.focus.set(crate::kit::widgets::HIDDEN);
        }
        if let Some((i, k)) = want
            && let Some(f) = Self::seg_frame(&b.rows[i].lay, k)
            && let Some(s) = b.rows[i].seg.as_ref()
        {
            let to = Props::at(f.x, f.y);
            if b.seg_at.map(|(r, _)| r) == Some(i) {
                kit.glide(&s.focus, to, Beat::Focus);
            } else {
                s.focus.set(to);
            }
        }
        b.seg_at = want;
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it, and a click is the target it ended on, with the segment under it for a row of options.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<(Target, Option<usize>)> {
        let radius = kit.tokens.radius(Radius::Row);
        let s_x = s.x;
        let id = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(Target::from_id).filter(|t| matches!(t, Target::Row(_))).map(|_| radius),
            Touch::Press { .. } => None,
        })?;
        let target = Target::from_id(id)?;
        let seg = match target {
            Target::Row(i) => self.built.as_ref().and_then(|b| b.rows.get(i)).and_then(|r| {
                let s = r.lay.seg.as_ref()?;
                let x0 = r.lay.frame.x + s.tray.x;
                (s_x >= x0 && s_x < x0 + s.tray.w).then(|| (((s_x - x0) / s.seg_w) as usize).min(s.labels.len() - 1))
            }),
            Target::Backdrop => None,
        };
        Some((target, seg))
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
        self.dimmed = None;
    }

    /// How many layers the menu holds.
    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let rows: Vec<Value> = b
            .rows
            .iter()
            .map(|r| json!({"frame": f(engine.focus.frame(r.id).unwrap_or(r.lay.frame)), "label": r.item.label, "options": r.item.options.len(), "danger": r.item.danger, "disabled": r.item.disabled}))
            .collect();
        json!({"card": f(b.lay.card), "rows": rows, "focus": engine.focus.current().and_then(Target::from_id).and_then(|t| if let Target::Row(i) = t { Some(i) } else { None }), "seg": b.seg_at.map(|s| s.1), "layers": self.layers(), "scope": SCOPE.0})
    }

    /// Describes what is on the screen to a lint: the card, the words on it, the rows' captions and the trays.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let paint = Paint::of(kit);
        let card = lint.panel("menu card", b.lay.card, kit.ui.look.colour(PaletteRole::Sheet), false);
        if let Some(h) = &b.head {
            h.title.feed_lint(kit, lint, "menu title", Some(card), (0.0, 0.0));
            if let Some(s) = &h.sub {
                s.feed_lint(kit, lint, "menu sub", Some(card), (0.0, 0.0));
            }
        }
        for (i, r) in b.rows.iter().enumerate() {
            r.words.feed_lint(kit, lint, &format!("menu row {i}"), Some(card), (0.0, 0.0));
            if let Some(h) = &r.hint {
                h.feed_lint(kit, lint, &format!("menu row {i} hint"), Some(card), (0.0, 0.0));
            }
            if let (Some(seg), Some(sl)) = (&r.seg, &r.lay.seg) {
                let at = Frame::new(r.lay.frame.x + sl.tray.x, r.lay.frame.y + sl.tray.y, sl.tray.w, sl.tray.h);
                let tray = lint.panel_on(&format!("menu row {i} tray"), at, paint.tray(), false, card);
                let chosen = r.item.index.min(sl.labels.len().saturating_sub(1));
                let under = lint.panel_on(
                    &format!("menu row {i} chosen"),
                    Frame::new(at.x + chosen as f32 * sl.seg_w, at.y, sl.seg_w, at.h),
                    paint.seg_selected(),
                    false,
                    tray,
                );
                for (k, w) in seg._labels.iter().enumerate() {
                    w.feed_lint(kit, lint, &format!("menu row {i} option {k}"), Some(if k == chosen { under } else { tray }), (0.0, 0.0));
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
    use crate::panels::menu::Menu;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: MenuSurface,
        m: Menu,
        t: u64,
    }

    fn dock() -> Value {
        json!({
            "title": "Crunchyroll", "sub": "Streaming", "icon": "play",
            "anchor": {"x": 700, "y": 600, "width": 160, "height": 160},
            "raised": {"x": 700, "y": 600, "width": 160, "height": 160, "iconUrl": "/nowhere.png", "plate": "#3A3350", "inset": 12},
            "items": [
                {"id": "open", "icon": "play", "label": "Open"},
                {"id": "move", "icon": "move", "label": "Move", "disabled": true},
                {"id": "unpin", "icon": "eye-off", "label": "Hide from the dock", "detail": "Stays in Apps", "hint": "Hold OK"},
                {"id": "uninstall", "icon": "trash-2", "label": "Uninstall", "danger": true}
            ],
            "hint": "Back closes"
        })
    }

    fn power() -> Value {
        json!({"title": "Power", "icon": "power", "width": 480, "items": [
            {"id": "sleep", "icon": "moon", "label": "Sleep timer", "options": ["Off", "15 min", "30 min", "60 min", "90 min"], "index": 1},
            {"id": "restart", "icon": "refresh-cw", "label": "Restart"},
            {"id": "off", "icon": "power", "label": "Turn off", "danger": true, "detail": "The TV goes to standby"}
        ], "hint": "OK"})
    }

    fn pointer_menu() -> Value {
        json!({"x": 1200, "y": 400, "scrim": false, "items": [{"id": "a", "label": "Open in a new window of a very long name that wraps"}, {"id": "b", "label": "Copy"}]})
    }

    impl Up {
        fn new(mode: Mode, scale: f32, spec: Value) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut m = Menu::new();
            m.life.show(0);
            m.open(Spec::from_json(&spec));
            let mut me = Self { _h: h, kit, engine, s: MenuSurface::new(crate::kit::z::PANELS), m, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.m.view(false).expect("a view");
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.m.life.phase(), self.t * 1_000_000).expect("the menu is drawn");
        }

        fn key(&mut self, k: K) {
            self.m.key(k);
            self.t += 40;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }
    }

    fn at(x: f32, y: f32, b: u32) -> Sample {
        Sample { x, y, buttons: b, visible: true }
    }

    #[test]
    fn the_card_opens_beside_what_it_belongs_to_inside_the_margin_with_a_row_per_option() {
        let u = Up::new(Mode::Dark, 1.0, dock());
        let g = u.s.geometry(&u.engine);
        let card = &g["card"];
        assert_eq!(card["x"], 700.0 + 160.0 + 16.0, "right of the tile");
        assert_eq!(card["y"].as_f64().unwrap() + card["h"].as_f64().unwrap(), 760.0, "level with its bottom edge");
        assert_eq!(card["w"], 520.0);
        let rows = g["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 4);
        assert!(rows[3]["danger"] == true && rows[1]["disabled"] == true);
        assert_eq!(g["focus"], 0, "the first item that is not disabled");
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        let ys: Vec<f64> = rows.iter().map(|r| r["frame"]["y"].as_f64().unwrap()).collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "one under the other: {ys:?}");
        let near = Up::new(Mode::Dark, 1.0, json!({"x": 1900, "y": 1070, "items": [{"id": "a", "label": "A"}]}));
        let c = &near.s.geometry(&near.engine)["card"];
        assert!(c["x"].as_f64().unwrap() + c["w"].as_f64().unwrap() <= 1920.0 - 48.0 && c["y"].as_f64().unwrap() + c["h"].as_f64().unwrap() <= 1080.0 - 48.0);
    }

    #[test]
    fn the_focus_skips_what_is_disabled_flies_between_rows_and_a_segment_moves_in_place() {
        let mut u = Up::new(Mode::Dark, 1.0, dock());
        assert!(!u.engine.flow.settling_at(u.t * 1_000_000));
        u.key(K::Down);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 2);
        assert!(u.engine.flow.settling_at(u.t * 1_000_000));
        let mut p = Up::new(Mode::Dark, 1.0, power());
        assert_eq!(p.s.geometry(&p.engine)["seg"], 1, "the plate is under the current choice");
        let layers = p.kit.room().layers;
        p.key(K::Right);
        p.key(K::Right);
        assert_eq!(p.s.geometry(&p.engine)["seg"], 3);
        p.key(K::Down);
        assert_eq!(p.s.geometry(&p.engine)["seg"], Value::Null, "no plate on a row without segments");
        p.key(K::Up);
        assert_eq!(p.s.geometry(&p.engine)["seg"], 1);
        assert_eq!(p.kit.room().layers, layers, "keys make no layer: the focus and the plate are moved");
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_with_every_kind_of_row() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for (name, spec) in [("dock", dock()), ("power", power()), ("pointer", pointer_menu())] {
                    let mut u = Up::new(mode, scale, spec);
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} {mode:?} {scale}: {f:#?}");
                    let g = u.s.geometry(&u.engine);
                    let c = &g["card"];
                    assert!(
                        c["y"].as_f64().unwrap() >= 48.0 && c["y"].as_f64().unwrap() + c["h"].as_f64().unwrap() <= 1080.0 - 48.0,
                        "{name} {scale}: the card stays inside the safe area: {c}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_click_names_the_row_and_the_segment_under_the_pointer_and_the_dim_is_a_target() {
        let mut u = Up::new(Mode::Dark, 1.0, power());
        let g = u.s.geometry(&u.engine);
        let row = &g["rows"][0]["frame"];
        let (rx, ry, rh) = (row["x"].as_f64().unwrap() as f32, row["y"].as_f64().unwrap() as f32, row["h"].as_f64().unwrap() as f32);
        let b = u.s.built.as_ref().unwrap();
        let tray = b.rows[0].lay.seg.as_ref().unwrap().tray;
        let seg_w = b.rows[0].lay.seg.as_ref().unwrap().seg_w;
        let x = rx + tray.x + seg_w * 2.5;
        let t = |n: u64| (3000 + n) * 1_000_000;
        let f1 = u.engine.focus.frame(Target::Row(1).id()).expect("a row");
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(f1.x + 20.0, f1.y + 10.0, 0)), None);
        assert!(u.s.stage.hover.is_shown(), "a cue under the pointer, on a row that is not the focus");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(x, ry + rh / 2.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(x, ry + rh / 2.0, 0)), Some((Target::Row(0), Some(2))));
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(f1.x + 20.0, f1.y + 10.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(40), at(f1.x + 20.0, f1.y + 10.0, 0)), Some((Target::Row(1), None)));
        u.s.pointer(&u.kit, &mut u.engine, t(50), at(60.0, 60.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(60), at(60.0, 60.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(70), at(60.0, 60.0, 0)), Some((Target::Backdrop, None)));
        u.s.on_key(&mut u.engine);
        assert!(!u.s.stage.hover.is_shown());
    }

    #[test]
    fn a_menu_without_a_scrim_is_made_without_one_and_a_new_spec_makes_the_rows_again() {
        let mut u = Up::new(Mode::Dark, 1.0, pointer_menu());
        assert_eq!(u.s.dimmed, Some(false));
        let layers = u.s.layers();
        u.m.open(Spec::from_json(&dock()));
        u.apply();
        assert_eq!(u.s.dimmed, Some(true));
        assert_eq!(u.s.geometry(&u.engine)["rows"].as_array().map(Vec::len), Some(4));
        assert_ne!(u.s.layers(), layers);
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Row(0), Target::Row(9), Target::Backdrop] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
        assert_eq!(Target::from_id(Id(1)), None);
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, dock());
        let with = u.kit.room().layers;
        u.m.close();
        u.m.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.m.life.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.m.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 10 && !u.s.is_up());
    }
}
