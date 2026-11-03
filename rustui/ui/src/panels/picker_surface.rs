//! The input picker drawn (`docs/mocks/08-input-picker`): a glass sheet along the bottom with one tile for Live TV and one for each HDMI
//! port the owner has not hidden. A tile has the port, a device icon, the name the device gave itself and a state line; the input the
//! sound goes out of has a mark, and the sound output is named on the right of the head. The sheet rises from the bottom edge; the tile
//! in front is marked, the one the focus is on lifts on the flowing focus.
//!
//! With the previews on (`inputs.previews`) a tile whose input has a picture is the picture: the live one for the input in front while
//! the picker is up, the last one for the others, dimmed, with its age in the state line (`shot_tile.rs`). The tiles are then the shape of
//! a picture (about 16:10, so the cut is a twentieth of its width each side) and the sheet is as tall as they need; a tile with no picture
//! (nothing connected, nothing taken yet, a picture that came back black) keeps its icon in the same shape.
//!
//! The model decides which tile has the focus; this follows it. The tiles are made again when the inputs say something else (a device
//! connected, a name, a tile that has its first picture or is not live any more); the focus alone moves in place, and the next live
//! picture of a tile is painted into its layer.

use super::life::Phase;
use super::picker::View;
use super::shot_tile::{CHIP, CHIP_GROUND, ON_PICTURE, ON_PICTURE_2, ShotTile, WORST_GROUND, dress};
use super::stage::{Stage, Touch};
use super::ui::{Arrive, WordLine, Words};
use crate::host::inputs::Tile;
use crate::host::previews::{Previews, Shot};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Spot, edge as wash_edge, hover_wash, selected_wash};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, Props, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(201);
const REGION: Region = Region(2100);
const BASE: u32 = 0x2100_0000;
const SHEET_X: f32 = 64.0;
const SHEET_W: f32 = 1792.0;
/// The mock's sheet and tile, as tall as that unless the words need more.
const SHEET_MIN_H: f32 = 404.0;
const TILE_MIN_H: f32 = 264.0;
/// A picture's tile is this wide for each unit of its height: a 16:9 picture loses a twentieth of its width at each side.
const PICTURE_ASPECT: f32 = 1.6;
/// How far above the words the scrim of a picture begins: long enough that a picture fades into it and is not cut by a band.
const SCRIM_RAMP: f32 = 40.0;
/// The corner of the chip behind a port's name: a pill for one line, no rounder than this for two.
const CHIP_RADIUS: f32 = 20.0;
const MARK: f32 = 26.0;

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Tile(usize),
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        match self {
            Target::Tile(i) => Id(BASE + i as u32),
            Target::Backdrop => Id(BASE + 0xFF00),
        }
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            0xFF00 => Some(Target::Backdrop),
            i if i < 0x100 => Some(Target::Tile(i as usize)),
            _ => None,
        }
    }
}

/// One tile laid out: its frame on the screen and where its parts are inside it.
#[derive(Clone, Debug, PartialEq)]
struct TileLay {
    frame: Frame,
    /// The tile is a picture: its words are on the picture, the port's name on a chip, the sound mark on a round one.
    pic: bool,
    port: Vec<String>,
    port_at: (f32, f32, f32),
    pill: Option<Frame>,
    mark: Option<(f32, f32)>,
    chip: Option<Frame>,
    icon_side: u32,
    icon_at: (f32, f32),
    name: Vec<String>,
    name_at: (f32, f32, f32),
    dot: f32,
    dot_at: (f32, f32),
    state: Vec<String>,
    state_at: (f32, f32, f32),
    /// Where the words begin in the tile: the scrim of a picture is as dense as the words need from here.
    text_top: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    sheet: Frame,
    title: Frame,
    sound: Option<(f32, String, f32)>,
    tiles: Vec<TileLay>,
}

/// The state line of a tile: what it said, and for a picture whether it is live or how old it is.
fn state_line(t: &Tile, shot: Option<&Shot>) -> String {
    match shot {
        Some(s) if s.live => format!("{} \u{b7} Live", t.state),
        Some(s) => format!("{} \u{b7} {}", t.state, s.age),
        None => t.state.clone(),
    }
}

/// What a tile's words take, measured before the tiles are given their common height.
struct Measured {
    port: Vec<String>,
    port_w: f32,
    pill_text_w: f32,
    icon_side: u32,
    name: Vec<String>,
    state: Vec<String>,
    name_h: f32,
    row_h: f32,
    need: f32,
}

fn layout(kit: &mut Kit, v: &View, shots: &[Option<&Shot>]) -> Layout {
    let (s1, s2, s3, s4, s5, s6, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(6), kit.space(7));
    let ink = kit.ui.look.ink();
    let ink2 = kit.ui.look.ink2();
    let label_style = role_style(kit, TypeRole::Label, None, ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let (label_line, caption_line, title_line) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption), kit.line(TypeRole::Title));
    let n = v.tiles.len().max(5) as f32;
    let tile_w = ((SHEET_W - 2.0 * s7 - (n - 1.0) * s4) / n).floor();
    let chip_d = MARK + s2;
    let head_y = s5;
    let row_y = head_y + title_line + s5;
    // the row of the port's name above an icon: the mock's 48, or as short as the name and the mark allow when the tiles are a picture's shape
    let top_row = |port_h: f32| if v.pictures { (port_h + s1).max(36.0) } else { 48.0f32.max(port_h + s1) };
    let mut measured = Vec::new();
    for (t, shot) in v.tiles.iter().zip(shots) {
        let pic = shot.is_some();
        // the sound mark: bare at the corner of a plain tile, on its own round chip on a picture
        let mark_room = if !t.sound {
            0.0
        } else if pic {
            chip_d + s3
        } else {
            MARK + s2
        };
        let port_w = tile_w - 2.0 * s5 - mark_room;
        let port = wrap::lines(kit, caption_style, &t.port, port_w, 2);
        let port_h = port.len() as f32 * caption_line;
        let pill_text_w = port.iter().map(|l| width_of(kit, caption_style, l)).fold(0.0f32, f32::max).ceil();
        let icon_side = if port.len() > 1 { 44 } else { 56 };
        let name = wrap::lines(kit, label_style, &t.name, tile_w - 2.0 * s5, 12);
        let dot = s2;
        let state = wrap::lines(kit, caption_style, &state_line(t, *shot), tile_w - 2.0 * s5 - dot - s2, 12);
        let name_h = name.len() as f32 * label_line;
        let state_h = state.len() as f32 * caption_line;
        let row_h = dot.max(state_h);
        let words_h = name_h + row_h;
        let need = if pic {
            // the chip of the port's name, then the words, with room between
            s3 + port_h + 2.0 * s1 + s3 + words_h + s4
        } else {
            s4 + top_row(port_h) + f32::from(icon_side as u16) + s3 + words_h + s4
        };
        // the height the tile would have with its icon: the row keeps one height whether a picture has come yet or not
        let with_icon = if pic {
            let port_i = wrap::lines(kit, caption_style, &t.port, tile_w - 2.0 * s5 - if t.sound { MARK + s2 } else { 0.0 }, 2);
            let state_i = wrap::lines(kit, caption_style, &t.state, tile_w - 2.0 * s5 - dot - s2, 12);
            let icon_i = if port_i.len() > 1 { 44u16 } else { 56 };
            s4 + top_row(port_i.len() as f32 * caption_line) + f32::from(icon_i) + s3 + name_h + dot.max(state_i.len() as f32 * caption_line) + s4
        } else {
            need
        };
        measured.push(Measured { port, port_w, pill_text_w, icon_side, name, state, name_h, row_h, need: need.max(with_icon).ceil() });
    }
    let min_h = if v.pictures { (tile_w / PICTURE_ASPECT).ceil() } else { TILE_MIN_H };
    let tallest = measured.iter().fold(min_h, |a, m| a.max(m.need));
    let mut tiles = Vec::new();
    for ((t, shot), m) in v.tiles.iter().zip(shots).zip(measured) {
        let pic = shot.is_some();
        let dot = s2;
        let words_h = m.name_h + m.row_h;
        let words_y = tallest - words_h - s4;
        let port_h = m.port.len() as f32 * caption_line;
        let (port_at, pill, chip, mark) = if pic {
            // the chip is cut to the text and sits so that its text starts where the name's does
            let pill = Frame::new(s5 - s2, s3, m.pill_text_w + 2.0 * s2, port_h + 2.0 * s1);
            let chip = t.sound.then(|| Frame::new(tile_w - chip_d - s3, s3, chip_d, chip_d));
            ((s5, s3 + s1, m.port_w), Some(pill), chip, chip.map(|c| (c.x + (chip_d - MARK) / 2.0, c.y + (chip_d - MARK) / 2.0)))
        } else {
            ((s5, s4, m.port_w), None, None, t.sound.then_some((tile_w - MARK - s4, s4)))
        };
        let icon_y = s4 + top_row(port_h);
        tiles.push(TileLay {
            frame: Frame::new(0.0, 0.0, tile_w, tallest),
            pic,
            port: m.port,
            port_at,
            pill,
            mark,
            chip,
            icon_side: m.icon_side,
            icon_at: (s5, icon_y),
            name: m.name,
            name_at: (s5, words_y, tile_w - 2.0 * s5),
            dot,
            dot_at: (s5, words_y + m.name_h + ((m.row_h - dot) / 2.0).round()),
            state: m.state,
            state_at: (s5 + dot + s2, words_y + m.name_h, tile_w - 2.0 * s5 - dot - s2),
            text_top: words_y,
        });
    }
    // One row: every tile is as tall as the tallest (a port whose name wraps, as "HDMI 2 · eARC" does at large text), its words kept to the
    // foot, so the focus plates and the washes of neighbours line up. A picture's tiles are as tall as they need; the mock's sheet has the room
    // of its taller tile.
    let sheet_h = if v.pictures { row_y + tallest + s7 } else { SHEET_MIN_H.max(row_y + tallest + s6) };
    let sheet = Frame::new(SHEET_X, 1080.0 - 64.0 - sheet_h, SHEET_W, sheet_h);
    for (i, t) in tiles.iter_mut().enumerate() {
        t.frame = Frame::new(sheet.x + s7 + i as f32 * (tile_w + s4), sheet.y + row_y, tile_w, tallest);
    }
    let title_w = width_of(kit, role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), "Inputs").ceil();
    let title = Frame::new(sheet.x + s7, sheet.y + head_y, title_w, title_line);
    let sound = (!v.sound.is_empty()).then(|| {
        let w = width_of(kit, role_style(kit, TypeRole::Label, None, ink2, Align::Left), &v.sound).ceil();
        (w, v.sound.clone(), MARK + s3 + w)
    });
    Layout { sheet, title, sound, tiles }
}

fn other(e: std::io::Error) -> std::io::Error {
    e
}

/// What a focusable tile is on the screen: the plate behind a plain tile's words, or the lifted picture.
enum TilePlate {
    Plain(Plate),
    Shot(Box<ShotTile>),
}

struct TileParts {
    id: Id,
    plate: TilePlate,
    _base: Option<ShapeLayer>,
    _pill: Option<ShapeLayer>,
    _chip: Option<ShapeLayer>,
    _port: Words,
    _mark: Option<ImageLayer>,
    _icon: Option<ImageLayer>,
    _name: Words,
    _dot: ShapeLayer,
    _state: Words,
    lay: TileLay,
    tile: Tile,
    shot: Option<Shot>,
}

impl TileParts {
    fn layers(&self) -> usize {
        let plate = match &self.plate {
            TilePlate::Plain(p) => p.layers(),
            TilePlate::Shot(s) => s.layers(),
        };
        plate
            + usize::from(self._base.is_some())
            + usize::from(self._pill.is_some())
            + usize::from(self._chip.is_some())
            + self._port.layers()
            + usize::from(self._mark.is_some())
            + usize::from(self._icon.is_some())
            + self._name.layers()
            + 1
            + self._state.layers()
    }
}

struct Built {
    sig: String,
    pane: Pane,
    _title: Words,
    sound: Option<(ImageLayer, Words, Frame)>,
    tiles: Vec<TileParts>,
    over_video: bool,
    lay: Layout,
}

pub struct PickerSurface {
    stage: Stage,
    built: Option<Built>,
}

fn tile_signature(t: &Tile, shot: Option<&Shot>) -> String {
    format!("{}|{}|{}|{}|{}|{}|{}|{}|{}|{}", t.app_id, t.port, t.icon, t.name, t.state, t.on, t.connected, t.front, t.sound, shot.map_or_else(String::new, |s| format!("{}|{}", s.live, s.age)))
}

/// The shot of each tile that is drawn as a picture: the previews are on, something is connected, and the picture has come.
fn shots_drawn<'a>(v: &'a View, store: &Previews) -> Vec<Option<&'a Shot>> {
    v.tiles
        .iter()
        .enumerate()
        .map(|(i, t)| v.shots.get(i).and_then(Option::as_ref).filter(|_| v.pictures && t.connected && store.picture(&t.key).is_some()))
        .collect()
}

/// What a drawing of the picker is made from: the view, the pictures the previews hold, and the shot of each tile that is a picture.
#[derive(Clone, Copy)]
struct Draw<'a> {
    v: &'a View,
    store: &'a Previews,
    shots: &'a [Option<&'a Shot>],
}

impl PickerSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    /// The sheet's place while it is built: what the mute mark keeps clear of.
    pub fn sheet(&self) -> Option<Frame> {
        self.built.as_ref().map(|b| b.lay.sheet)
    }

    fn signature(kit: &Kit, v: &View, lay: &Layout, shots: &[Option<&Shot>]) -> String {
        let tiles: Vec<String> = v.tiles.iter().zip(shots).map(|(t, s)| tile_signature(t, *s)).collect();
        format!("{}|{}|{}|{}|{}|{:?}", kit.generation(), kit.text_scale, v.sound, v.pictures, tiles.join("/"), lay.sheet)
    }

    fn fill(kit: &Kit, front: bool) -> Shape {
        let radius = kit.tokens.radius(Radius::Card);
        Shape { corner_radius: radius, fill: Fill::Solid(if front { selected_wash(kit) } else { hover_wash(kit) }), border_width: 1.0, border: wash_edge(kit) }
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, draw: Draw, lay: Layout, sig: String) -> std::io::Result<Built> {
        let Draw { v, store, shots } = draw;
        let scene = kit.ui.scene.clone();
        let (ink, ink2, positive, ink3) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.positive(), kit.ui.look.ink3());
        let radius = kit.tokens.radius(Radius::Card);
        scene.adopting_view(root, || -> std::io::Result<Built> {
            let pane = Pane::new(kit, lay.sheet, z + 1, PaneKind::Sheet, v.over_video);
            let title = Words::new(kit, lay.title.x, lay.title.y, lay.title.w + 2.0, z + 4, &[WordLine::new("Inputs", TypeRole::Title, ink).weight(Weight::SemiBold)])?;
            let sound = match &lay.sound {
                Some((w, text, total)) => {
                    let x = lay.sheet.x + lay.sheet.w - kit.space(7) - total;
                    let cy = lay.title.y + lay.title.h / 2.0;
                    let icon_layer = icon::layer(kit, "volume-2", MARK as u32, ink2, z + 4, Props::at(x, (cy - MARK / 2.0).round()))?;
                    let line = kit.line(TypeRole::Label);
                    let words = Words::new(kit, x + MARK + kit.space(3), (cy - line / 2.0).round(), w + 2.0, z + 4, &[WordLine::new(text, TypeRole::Label, ink2)])?;
                    icon_layer.map(|i| (i, words, Frame::new(x, cy - MARK / 2.0, *total, MARK)))
                }
                None => None,
            };
            let mut tiles = Vec::new();
            for (i, (t, tl)) in v.tiles.iter().zip(lay.tiles.iter()).enumerate() {
                let id = Target::Tile(i).id();
                let f = tl.frame;
                let shot = shots[i];
                let spot = Spot { id, screen: f, local: f, region: REGION, scope: SCOPE, z: z + 3 };
                let (plate_parts, base, pill, chip, words_z) = match (tl.pic, shot, store.picture(&t.key)) {
                    (true, Some(s), Some(pic)) => {
                        let dressed = dress(pic, f.w.round() as u32, f.h.round() as u32, radius, tl.text_top, SCRIM_RAMP, s.live);
                        let tile = ShotTile::new(kit, engine, root, spot, radius, &dressed, (s.rev, s.live))?;
                        let chip_shape = |c: Frame| plate::shape(&kit.ui, Frame::new(f.x + c.x, f.y + c.y, c.w, c.h), z + 5, plate::solid(CHIP, (c.h / 2.0).min(CHIP_RADIUS)));
                        (TilePlate::Shot(Box::new(tile)), None, tl.pill.map(chip_shape), tl.chip.map(chip_shape), z + 6)
                    }
                    _ => {
                        let plate_parts = Plate::new(kit, engine, root, spot, Lift::Card, radius)?;
                        (TilePlate::Plain(plate_parts), Some(plate::shape(&kit.ui, f, z + 2, Self::fill(kit, t.front))), None, None, z + 4)
                    }
                };
                let (ink_main, ink_quiet, mark_ink) = if tl.pic { (ON_PICTURE, ON_PICTURE_2, ON_PICTURE) } else { (ink, ink2, positive) };
                let port_lines: Vec<WordLine> = tl.port.iter().map(|s| WordLine::new(s, TypeRole::Caption, if tl.pic { ON_PICTURE } else { ink2 })).collect();
                let port = Words::new(kit, f.x + tl.port_at.0, f.y + tl.port_at.1, tl.port_at.2, words_z, &port_lines)?;
                let mark = match tl.mark {
                    Some((x, y)) => icon::layer(kit, "volume-2", MARK as u32, mark_ink, words_z, Props::at(f.x + x, f.y + y))?,
                    None => None,
                };
                let icon_layer = if tl.pic { None } else { icon::layer(kit, &t.icon, tl.icon_side, if t.on { ink } else { ink2 }, words_z, Props::at(f.x + tl.icon_at.0, f.y + tl.icon_at.1))? };
                let name_lines: Vec<WordLine> = tl.name.iter().map(|s| WordLine::new(s, TypeRole::Label, ink_main)).collect();
                let name = Words::new(kit, f.x + tl.name_at.0, f.y + tl.name_at.1, tl.name_at.2, words_z, &name_lines)?;
                let dot = plate::shape(&kit.ui, Frame::new(f.x + tl.dot_at.0, f.y + tl.dot_at.1, tl.dot, tl.dot), words_z, plate::solid(if t.connected { positive } else { ink3 }, tl.dot / 2.0));
                let state_lines: Vec<WordLine> = tl.state.iter().map(|s| WordLine::new(s, TypeRole::Caption, ink_quiet)).collect();
                let state = Words::new(kit, f.x + tl.state_at.0, f.y + tl.state_at.1, tl.state_at.2, words_z, &state_lines)?;
                tiles.push(TileParts {
                    id,
                    plate: plate_parts,
                    _base: base,
                    _pill: pill,
                    _chip: chip,
                    _port: port,
                    _mark: mark,
                    _icon: icon_layer,
                    _name: name,
                    _dot: dot,
                    _state: state,
                    lay: tl.clone(),
                    tile: t.clone(),
                    shot: shot.cloned(),
                });
            }
            Ok(Built { sig, pane, _title: title, sound, tiles, over_video: v.over_video, lay: lay.clone() })
        })
        .map_err(other)
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for t in b.tiles {
                match t.plate {
                    TilePlate::Plain(p) => p.destroy(engine),
                    TilePlate::Shot(s) => s.destroy(engine),
                }
            }
        }
    }

    /// The next live picture of a tile (or a newer last one) goes into its layer, the words and the layers staying as they are.
    fn sync_pictures(&mut self, kit: &Kit, v: &View, store: &Previews) {
        let Some(b) = self.built.as_mut() else { return };
        let radius = kit.tokens.radius(Radius::Card);
        for (i, t) in b.tiles.iter_mut().enumerate() {
            let (TilePlate::Shot(tile), Some(Some(shot))) = (&mut t.plate, v.shots.get(i)) else { continue };
            if tile.shown == (shot.rev, shot.live) {
                continue;
            }
            if let Some(pic) = store.picture(&t.tile.key) {
                let f = t.lay.frame;
                tile.repaint(&dress(pic, f.w.round() as u32, f.h.round() as u32, radius, t.lay.text_top, SCRIM_RAMP, shot.live), (shot.rev, shot.live));
                t.shot = Some(shot.clone());
            }
        }
    }

    /// Fits the picker to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, store: &Previews, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let shots = shots_drawn(v, store);
        let lay = layout(kit, v, &shots);
        let arrive = Arrive::Bottom(lay.sheet.h + 64.0);
        if self.stage.needs_root(arrive) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, arrive, true)?;
        let sig = Self::signature(kit, v, &lay, &shots);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), Draw { v, store, shots: &shots }, lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the input picker could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        } else {
            self.sync_pictures(kit, v, store);
        }
        if self.stage.phase(kit, engine, phase) {
            if let Some(b) = self.built.as_mut()
                && b.over_video != v.over_video
            {
                b.pane.restyle(kit, v.over_video);
                b.over_video = v.over_video;
            }
            self.stage.focus(engine, (!v.tiles.is_empty()).then(|| Target::Tile(v.index).id()), now_ns);
        }
        Ok(true)
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it, and a click is the target it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let radius = kit.tokens.radius(Radius::Card);
        let id = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(Target::from_id).filter(|t| matches!(t, Target::Tile(_))).map(|_| radius),
            Touch::Press { .. } => None,
        })?;
        Target::from_id(id)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    /// How many layers the picker holds.
    pub fn layers(&self) -> usize {
        let Some(b) = &self.built else { return 0 };
        2 + 1 + b.sound.as_ref().map_or(0, |_| 2) + b.tiles.iter().map(TileParts::layers).sum::<usize>()
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let tiles: Vec<Value> = b
            .tiles
            .iter()
            .map(|t| {
                json!({
                    "frame": f(engine.focus.frame(t.id).unwrap_or(t.lay.frame)),
                    "name": t.tile.name,
                    "port": t.tile.port,
                    "front": t.tile.front,
                    "sound": t.tile.sound,
                    "picture": t.lay.pic,
                    "rev": match &t.plate {
                        TilePlate::Shot(s) => s.shown.0,
                        TilePlate::Plain(_) => 0,
                    },
                    "live": t.shot.as_ref().is_some_and(|s| s.live),
                    "age": t.shot.as_ref().map(|s| s.age.clone()),
                    "state": t.lay.state.join(" "),
                })
            })
            .collect();
        json!({"sheet": f(b.lay.sheet), "tiles": tiles, "focus": engine.focus.current().and_then(Target::from_id).and_then(|t| if let Target::Tile(i) = t { Some(i) } else { None }), "layers": self.layers(), "scope": SCOPE.0})
    }

    /// Describes what is on the screen to a lint: the sheet, each tile over it, and the words on them. The ground of the words on a picture
    /// is the picture's scrim over the brightest picture there is, and a chip's black over it.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = lint.panel("input sheet", b.lay.sheet, kit.ui.look.colour(PaletteRole::Sheet), false);
        b._title.feed_lint(kit, lint, "inputs title", Some(sheet), (0.0, 0.0));
        if let Some((_, words, _)) = &b.sound {
            words.feed_lint(kit, lint, "inputs sound", Some(sheet), (0.0, 0.0));
        }
        for (i, t) in b.tiles.iter().enumerate() {
            let paint = if t.lay.pic {
                WORST_GROUND
            } else if t.tile.front {
                selected_wash(kit)
            } else {
                hover_wash(kit)
            };
            let card = lint.panel_on(&format!("tile {i}"), t.lay.frame, paint, false, sheet);
            let port_ground = match t.lay.pill {
                Some(p) => lint.panel_on(&format!("tile {i} chip"), Frame::new(t.lay.frame.x + p.x, t.lay.frame.y + p.y, p.w, p.h), CHIP_GROUND, false, card),
                None => card,
            };
            t._port.feed_lint(kit, lint, &format!("tile {i} port"), Some(port_ground), (0.0, 0.0));
            t._name.feed_lint(kit, lint, &format!("tile {i} name"), Some(card), (0.0, 0.0));
            t._state.feed_lint(kit, lint, &format!("tile {i} state"), Some(card), (0.0, 0.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::life::Phase;
    use crate::panels::env::Env;
    use crate::panels::picker::Picker;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: PickerSurface,
        p: Picker,
        env: Env,
        t: u64,
    }

    fn tv(devices: Value) -> Env {
        // every port is shown, the third with nothing on it
        let mut env = Env::new(json!({"inputs": {"hdmi1": "always", "hdmi2": "always", "hdmi3": "always", "hdmi4": "always"}}));
        env.inputs.parse(&json!({"devices": devices}));
        env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi2", "home": false}) }, 0);
        env
    }

    fn devices(n: usize) -> Value {
        let names = ["Apple TV 4K", "Denon AVR-S760H", "PlayStation 5 with a rather long name that wraps", "Nintendo Switch", "Blu-ray player", "Fire TV", "Shield", "Xbox"];
        Value::Array((0..n).map(|i| json!({"appId": format!("com.webos.app.hdmi{}", i + 1), "label": names[i % names.len()], "connected": i != 2})).collect())
    }

    impl Up {
        fn new(mode: Mode, scale: f32, n: usize) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut env = tv(devices(n));
            env.sio.write("sound.soundOutput", &json!("external_arc"), None, 0);
            let mut p = Picker::new();
            p.life.show(0);
            p.open(&mut env, 0);
            let mut me = Self { _h: h, kit, engine, s: PickerSurface::new(crate::kit::z::PANELS), p, env, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.p.view(&self.env, self.t);
            self.s.apply(&mut self.kit, &mut self.engine, &v, &self.env.previews, self.p.life.phase(), self.t * 1_000_000).expect("the picker is drawn");
        }

        fn key(&mut self, k: K) {
            self.p.key(k, &self.env);
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
    fn the_sheet_rests_64_from_the_bottom_and_holds_a_tile_per_input_in_one_row() {
        let u = Up::new(Mode::Dark, 1.0, 4);
        let g = u.s.geometry(&u.engine);
        assert_eq!(g["sheet"]["x"], 64.0);
        assert_eq!(g["sheet"]["w"], 1792.0);
        let sheet = &g["sheet"];
        assert_eq!(sheet["y"].as_f64().unwrap() + sheet["h"].as_f64().unwrap(), 1080.0 - 64.0);
        assert!(sheet["h"].as_f64().unwrap() >= 404.0);
        let tiles = g["tiles"].as_array().expect("tiles");
        assert_eq!(tiles.len(), 5, "Live TV and the four ports");
        let w0 = tiles[0]["frame"]["w"].as_f64().unwrap();
        assert!(tiles.iter().all(|t| (t["frame"]["w"].as_f64().unwrap() - w0).abs() < 0.5 && t["frame"]["h"].as_f64().unwrap() >= 264.0));
        assert_eq!(w0, ((1792.0f64 - 2.0 * 32.0 - 4.0 * 16.0) / 5.0).floor(), "five columns even when there are fewer tiles: {w0}");
        assert_eq!(g["focus"], 2, "on the input in front");
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
    }

    #[test]
    fn the_tiles_are_one_row_as_tall_as_the_tallest_at_large_text_too() {
        for scale in [1.0, 2.0] {
            let u = Up::new(Mode::Dark, scale, 3);
            let g = u.s.geometry(&u.engine);
            let tiles = g["tiles"].as_array().expect("tiles");
            let (y0, h0) = (tiles[0]["frame"]["y"].as_f64().unwrap(), tiles[0]["frame"]["h"].as_f64().unwrap());
            for t in tiles {
                assert_eq!((t["frame"]["y"].as_f64().unwrap(), t["frame"]["h"].as_f64().unwrap()), (y0, h0), "at {scale}: {tiles:?}");
            }
        }
    }

    #[test]
    fn the_focus_follows_the_choice_with_a_lift_and_the_first_one_is_lifted_at_once() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        assert!(!u.engine.flow.settling_at(u.t * 1_000_000));
        u.key(K::Right);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 3);
        assert!(u.engine.flow.settling_at(u.t * 1_000_000));
        u.key(K::Left);
        u.key(K::Left);
        assert_eq!(u.s.geometry(&u.engine)["focus"], 1);
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_with_few_tiles_and_many() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for n in [2, 3, 4] {
                    let mut u = Up::new(mode, scale, n);
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} {n} ports: {f:#?}");
                }
            }
        }
    }

    #[test]
    fn a_change_in_the_inputs_makes_the_tiles_again_and_a_key_changes_nothing_in_the_scene() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let layers = u.kit.room().layers;
        u.key(K::Right);
        u.key(K::Left);
        assert_eq!(u.kit.room().layers, layers, "the focus is the flowing lift: no layer is made");
        let before = u.s.geometry(&u.engine)["tiles"][2]["name"].clone();
        u.env.inputs.parse(&json!({"devices": devices(1)}));
        u.p.sync(&u.env);
        u.apply();
        assert_ne!(u.s.geometry(&u.engine)["tiles"][2]["name"], before, "the tile that lost its device says so");
        assert_eq!(u.s.geometry(&u.engine)["tiles"].as_array().map(Vec::len), Some(2), "Live TV and the one port left");
    }

    #[test]
    fn a_click_on_a_tile_names_it_and_the_dim_is_a_target_that_closes() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let f = u.engine.focus.frame(Target::Tile(1).id()).expect("a tile");
        let at = |x: f32, y: f32, b: u32| Sample { x, y, buttons: b, visible: true };
        let t = |n: u64| (3000 + n) * 1_000_000;
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(f.x + 20.0, f.y + 20.0, 0)), None);
        assert!(u.s.stage.hover.is_shown(), "a cue under the pointer");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(f.x + 20.0, f.y + 20.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(f.x + 20.0, f.y + 20.0, 0)), Some(Target::Tile(1)));
        // the dim, above the sheet
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(600.0, 100.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(600.0, 100.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(50), at(600.0, 100.0, 0)), Some(Target::Backdrop));
        u.s.on_key(&mut u.engine);
        assert!(!u.s.stage.hover.is_shown());
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Tile(0), Target::Tile(7), Target::Backdrop] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
        assert_eq!(Target::from_id(Id(1)), None);
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        let with = u.kit.room().layers;
        u.p.close(&mut u.env, 2000);
        u.p.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        u.p.life.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.p.life.tick(7000);
        u.apply();
        assert!(u.kit.room().layers > with + 20 && !u.s.is_up());
    }

    fn centre(f: Frame) -> (f32, f32) {
        (f.x + f.w / 2.0, f.y + f.h / 2.0)
    }

    /// The tiles whose lit plate is up by more than half, once the springs have settled (the scene's own clock runs them).
    fn lit(u: &Up) -> Vec<usize> {
        let at = u.kit.ui.scene.now_ns() + 3_000_000_000;
        (0..8).filter(|i| u.engine.flow.lift_at(Target::Tile(*i).id(), at) > 0.5).collect()
    }

    #[test]
    fn the_lifted_plate_is_concentric_with_its_tile_on_every_tile_and_at_both_sizes() {
        for scale in [1.0, 2.0] {
            let u = Up::new(Mode::Dark, scale, 3);
            for i in 0..4 {
                let id = Target::Tile(i).id();
                let tile = u.engine.focus.frame(id).expect("a tile");
                let (plate, _) = u.engine.flow.lifted_plate(id, u.kit.ui.scene.now_ns() + 5_000_000_000).expect("a plate");
                let (a, b) = (centre(tile), centre(plate));
                assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01, "tile {i} at {scale}: plate {plate:?} is off its tile {tile:?}");
                assert!(plate.w >= tile.w && plate.h >= tile.h, "the plate is the tile grown, never smaller");
            }
        }
    }

    #[test]
    fn the_ring_is_on_the_tile_the_keys_chose_and_on_it_alone_also_after_the_picker_is_opened_again() {
        let mut u = Up::new(Mode::Dark, 1.0, 3);
        assert_eq!(lit(&u), vec![2], "opened on the input in front");
        u.key(K::Left);
        assert_eq!(lit(&u), vec![1]);
        u.key(K::Left);
        u.key(K::Left);
        assert_eq!(lit(&u), vec![0], "on Live TV");
        // closed and gone, then opened again with no key: the ring is on the tile in front and nowhere else
        u.p.close(&mut u.env, 2000);
        u.p.life.hide(2000);
        u.apply();
        u.p.life.tick(2200);
        u.apply();
        assert_eq!(u.p.life.phase(), Phase::Parked, "its layers are kept for a while");
        u.t = 2500;
        u.p.life.show(u.t);
        u.p.open(&mut u.env, u.t);
        u.apply();
        assert_eq!(lit(&u), vec![2], "no ghost ring on the other tiles");
    }

    // ---- the pictures of the inputs

    /// A 24-bit BMP of `w` by `h` with the colour `f` gives at each pixel (BGR, as the helper's stills come on the set).
    fn bmp(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let stride = (w * 3).div_ceil(4) * 4;
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[2..6].copy_from_slice(&(54 + stride * h).to_le_bytes());
        b[10..14].copy_from_slice(&54u32.to_le_bytes());
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&w.to_le_bytes());
        b[22..26].copy_from_slice(&h.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&24u16.to_le_bytes());
        // bottom row first
        for y in (0..h).rev() {
            for x in 0..w {
                let [r, g, bl] = f(x, y);
                b.extend_from_slice(&[bl, g, r]);
            }
            b.extend(std::iter::repeat_n(0u8, (stride - w * 3) as usize));
        }
        b
    }

    /// A picture that is not flat: a gradient across, so a cut or a shift shows.
    fn scene_of(w: u32, h: u32, base: u8) -> Vec<u8> {
        bmp(w, h, |x, y| [base.saturating_add((x * 120 / w) as u8), 90, base.saturating_add((y * 100 / h) as u8)])
    }

    use crate::host::previews::tags as ptags;

    /// The tests' wall clock when the picker opens: a day and some, so that a picture can be days old.
    const T0: u64 = 100_000_000;
    /// A second after it opened: the sheet is in and the pictures are decoded as they come.
    const NOW: u64 = T0 + 1_000;

    impl Up {
        /// The previews on, the helper up, and the pictures the helper keeps: `keys` with when each was taken (epoch ms).
        fn with_previews(mode: Mode, scale: f32, n: usize, keys: &[(&str, u64)]) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut env = tv(devices(n));
            env.sio.write("sound.soundOutput", &json!("external_arc"), None, 0);
            env.previews.set_active(true);
            env.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 0);
            let mut p = Picker::new();
            p.life.show(0);
            let mut me = Self { _h: h, kit, engine, s: PickerSurface::new(crate::kit::z::PANELS), p, env, t: T0 };
            me.pictures(keys);
            me.p.open(&mut me.env, me.t);
            me.t = NOW;
            me.env.update(&Input::Tick, me.t);
            me.apply();
            me
        }

        /// The helper's index names `keys` and each one's picture comes back.
        fn pictures(&mut self, keys: &[(&str, u64)]) {
            let items: serde_json::Map<String, Value> = keys.iter().map(|(k, at)| ((*k).to_owned(), json!({"at": at}))).collect();
            let doc = json!({"ok": true, "previews": {"enabled": true, "items": items}});
            let fx = self.env.update(&Input::Fetched { tag: ptags::INDEX, status: 200, data: doc }, self.t).0;
            for e in fx {
                if let crate::kit::screen::Effect::Fetch { tag, path } = e
                    && path.starts_with("/v1/previews/")
                {
                    let key = path.trim_start_matches("/v1/previews/");
                    let base = if key == "hdmi2" { 30 } else { 120 };
                    self.env.update(&Input::Bytes { tag, status: 200, bytes: scene_of(96, 54, base) }, self.t);
                }
            }
        }

        /// A live picture of the input in front, taken at `at`: the next live ask is made when it is due (nothing when the one the picker
        /// opened with is still out), the helper answers it and the picture's bytes arrive.
        fn live(&mut self, at: u64, base: u8) {
            self.t = self.t.max(at);
            self.env.update(&Input::Tick, at);
            let fx = self.env.update(&Input::Fetched { tag: ptags::CAPTURE, status: 200, data: json!({"ok": true, "key": "hdmi2", "at": at}) }, at).0;
            for e in fx {
                if let crate::kit::screen::Effect::Fetch { tag, .. } = e {
                    self.env.update(&Input::Bytes { tag, status: 200, bytes: scene_of(96, 54, base) }, at);
                }
            }
        }
    }

    fn tile(g: &Value, i: usize) -> &Value {
        &g["tiles"][i]
    }

    #[test]
    fn with_the_previews_on_the_tiles_are_the_shape_of_a_picture_and_the_sheet_is_as_tall_as_they_need() {
        let u = Up::with_previews(Mode::Dark, 1.0, 4, &[]);
        let g = u.s.geometry(&u.engine);
        let tiles = g["tiles"].as_array().unwrap();
        assert_eq!(tiles.len(), 5);
        let heights: Vec<f64> = tiles.iter().map(|t| t["frame"]["h"].as_f64().unwrap()).collect();
        assert!(heights.iter().all(|h| *h == heights[0]), "one height for the row: {heights:?}");
        assert!((208.0..=212.0).contains(&heights[0]), "a picture's shape, not the mock's 264: {heights:?}");
        let sheet = &g["sheet"];
        assert!(sheet["h"].as_f64().unwrap() < 404.0, "the sheet is as tall as its tiles need: {sheet}");
        assert_eq!(sheet["y"].as_f64().unwrap() + sheet["h"].as_f64().unwrap(), 1080.0 - 64.0, "and still rests 64 from the bottom");
        assert!(tiles.iter().all(|t| t["picture"] == false), "no picture has come yet: every tile keeps its icon");
    }

    #[test]
    fn with_the_previews_off_the_picker_is_as_the_mock_has_it() {
        let (h, kit) = testkit::kit_in(Mode::Dark, 1.0);
        let engine = Engine::new(&kit).expect("an engine");
        let mut env = Env::new(json!({"inputs": {"hdmi1": "always", "hdmi2": "always", "previews": false}}));
        env.inputs.parse(&json!({"devices": devices(2)}));
        env.previews.set_active(true);
        let mut p = Picker::new();
        p.life.show(0);
        p.open(&mut env, 0);
        let mut u = Up { _h: h, kit, engine, s: PickerSurface::new(crate::kit::z::PANELS), p, env, t: 1000 };
        u.apply();
        let g = u.s.geometry(&u.engine);
        assert!(g["sheet"]["h"].as_f64().unwrap() >= 404.0);
        assert!(g["tiles"].as_array().unwrap().iter().all(|t| t["frame"]["h"].as_f64().unwrap() >= 264.0 && t["picture"] == false));
    }

    #[test]
    fn a_tile_with_its_picture_is_the_picture_and_says_live_or_how_old_it_is() {
        let now = NOW;
        let mut u = Up::with_previews(Mode::Dark, 1.0, 4, &[("hdmi1", now - 7 * 60_000), ("hdmi2", now - 5000)]);
        u.live(now, 30);
        u.apply();
        let g = u.s.geometry(&u.engine);
        // Live TV has none; HDMI 1 has its last picture; HDMI 2 (in front) is live
        assert_eq!(tile(&g, 0)["picture"], false);
        assert_eq!((tile(&g, 1)["picture"].as_bool(), tile(&g, 1)["live"].as_bool(), tile(&g, 1)["age"].as_str()), (Some(true), Some(false), Some("7 min ago")));
        assert!(tile(&g, 1)["state"].as_str().unwrap().ends_with("Connected \u{b7} 7 min ago"), "{}", tile(&g, 1)["state"]);
        assert_eq!((tile(&g, 2)["picture"].as_bool(), tile(&g, 2)["live"].as_bool(), tile(&g, 2)["age"].as_str()), (Some(true), Some(true), Some("live")));
        assert_eq!(tile(&g, 2)["state"], "In front \u{b7} Live");
        // the ports have the shape of a picture and every tile is as tall as the others
        let h0 = tile(&g, 0)["frame"]["h"].as_f64().unwrap();
        assert!((0..5).all(|i| tile(&g, i)["frame"]["h"].as_f64().unwrap() == h0));
    }

    #[test]
    fn a_tile_whose_input_has_nothing_connected_keeps_its_icon_whatever_picture_is_held() {
        let now = NOW;
        // the third port has nothing on it (`devices` leaves it unplugged)
        let u = Up::with_previews(Mode::Dark, 1.0, 4, &[("hdmi3", now - 60_000), ("hdmi2", now - 1000)]);
        let g = u.s.geometry(&u.engine);
        assert_eq!(tile(&g, 3)["name"], "Nothing connected");
        assert_eq!(tile(&g, 3)["picture"], false);
        assert_eq!(tile(&g, 2)["picture"], true);
    }

    #[test]
    fn the_next_live_picture_is_painted_into_the_layer_and_the_tile_is_not_made_again() {
        let now = NOW;
        let mut u = Up::with_previews(Mode::Dark, 1.0, 4, &[("hdmi2", now - 5000)]);
        u.live(now, 30);
        u.apply();
        let layers = u.kit.room().layers;
        let g0 = u.s.geometry(&u.engine);
        let rev0 = tile(&g0, 2)["rev"].as_u64().unwrap();
        for n in 1..=3u64 {
            u.t = now + n * 500;
            u.live(u.t, 30 + n as u8 * 20);
            u.apply();
            let g = u.s.geometry(&u.engine);
            assert_eq!(tile(&g, 2)["rev"].as_u64().unwrap(), rev0 + n, "the layer holds the newest picture");
            assert_eq!(u.kit.room().layers, layers, "a live picture makes no layer");
            assert_eq!(tile(&g, 2)["live"], true);
        }
    }

    #[test]
    fn a_picture_that_arrives_makes_its_tile_once_and_one_that_stops_being_live_says_its_age() {
        let now = NOW;
        let mut u = Up::with_previews(Mode::Dark, 1.0, 4, &[]);
        let plain = u.s.layers();
        let row_h = tile(&u.s.geometry(&u.engine), 0)["frame"]["h"].clone();
        assert_eq!(tile(&u.s.geometry(&u.engine), 1)["picture"], false);
        u.pictures(&[("hdmi1", now - 120_000)]);
        u.p.sync(&u.env);
        u.apply();
        let g = u.s.geometry(&u.engine);
        assert_eq!(tile(&g, 1)["picture"], true, "the first picture of a tile is the tile");
        assert_eq!(tile(&g, 0)["frame"]["h"], row_h, "the row keeps its height when a picture comes");
        let with_one = u.s.layers();
        assert_ne!(with_one, plain);
        u.apply();
        assert_eq!(u.s.layers(), with_one, "and it is not made again");
        // the live word goes after the live pictures stop
        u.live(now, 30);
        u.apply();
        assert_eq!(tile(&u.s.geometry(&u.engine), 2)["state"], "In front \u{b7} Live");
        u.t = now + 4000;
        u.apply();
        let g = u.s.geometry(&u.engine);
        assert_eq!(tile(&g, 2)["live"], false);
        assert_eq!(tile(&g, 2)["state"], "In front \u{b7} just now");
    }

    #[test]
    fn pictures_read_clean_in_both_materials_at_100_and_200_percent() {
        let now = NOW;
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for n in [2, 4] {
                    let mut u = Up::with_previews(mode, scale, n, &[("hdmi1", now - 3 * 3_600_000), ("hdmi2", now - 2000), ("hdmi4", now - 90 * 60_000)]);
                    u.live(now, 200);
                    u.apply();
                    let f = u.lint();
                    assert!(f.is_empty(), "{mode:?} {scale} {n} ports: {f:#?}");
                    let g = u.s.geometry(&u.engine);
                    assert!(g["tiles"].as_array().unwrap().iter().any(|t| t["picture"] == true));
                }
            }
        }
    }

    #[test]
    fn the_picture_tiles_stay_inside_the_scene_with_every_input_pictured() {
        let now = NOW;
        let mut u = Up::with_previews(Mode::Dark, 1.0, 4, &[("hdmi1", now - 1000), ("hdmi2", now - 1000), ("hdmi4", now - 1000), ("livetv", now - 1000)]);
        u.live(now, 90);
        u.apply();
        let g = u.s.geometry(&u.engine);
        assert!(g["layers"].as_u64().unwrap() < 100, "{}", g["layers"]);
        assert!(u.kit.room().layers > 100, "room left: {}", u.kit.room().layers);
    }

    #[test]
    fn the_focus_lifts_a_picture_tile_about_its_own_centre_like_any_tile() {
        let now = NOW;
        let mut u = Up::with_previews(Mode::Dark, 1.0, 4, &[("hdmi1", now - 1000), ("hdmi2", now - 1000)]);
        u.live(now, 60);
        u.apply();
        assert_eq!(lit(&u), vec![2], "the tile in front is lifted at once");
        u.key(K::Left);
        assert_eq!(lit(&u), vec![1], "and the ring moves to the picture the keys chose");
        for i in [1usize, 2] {
            let id = Target::Tile(i).id();
            let tile = u.engine.focus.frame(id).expect("a tile");
            let (lifted, _) = u.engine.flow.lifted_plate(id, u.kit.ui.scene.now_ns() + 5_000_000_000).expect("a lifted tile");
            let (a, b) = (centre(tile), centre(lifted));
            assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01, "tile {i}: the picture grows about its own centre");
        }
    }
}
