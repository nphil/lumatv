//! The guide's drawing: a page over the live picture with the filters and the way back to now in its head, the times over a grid of
//! programmes beside a rail of channels, what the focus is on in a strip under it, the keys as glyphs, and one detail sheet over it all.
//!
//! The grid is cached and moved by the compositor: the rows and the time are scrolled on tracks of the flow engine (the rail and the rows by
//! one, the cells and the ruler by two more), so a key only retargets a spring and nothing is drawn by the frame. Only the rows on the
//! screen (and one more each side) have layers: a row is a logo's tile, a text of the number and name, one run of glyph quads with every cell of
//! the stretch of time round the view (titles, dividers, how far the programme on the air has gone) and a lead, the title of the cell at the
//! left edge, which stays at the edge. A title is said in the part of its cell the view shows (`cells`) and no line of the grid runs through
//! a word. The focus is a lit plate lifted by the flow engine, one layer a move. Everything is drawn from the theme's tokens and the look's
//! roles; nothing here names a colour or a size.

use super::cells;
use super::layout::{self, Layout};
use super::model::{Chip, Filter, Guide, HOUR, MIN, Page, Span, Target};
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt};
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Px, Spot, edge, selected_wash};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use crate::panels::life::Phase;
use crate::panels::stage::Stage;
use crate::panels::ui::{Arrive, scrim, scrim_set};
use compositor_protocol::{Ease, Edges, GroupPose, Props, Rect, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use std::io;
use ui::components::{Align, Form, Item, Mark, Run, TextLayer, ellipsise, line_height};
use ui::geometry::Frame;
use ui::group::Group;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Type;

pub const SCOPE: Scope = Scope(290);
const R_CHIPS: Region = Region(2950);
const R_GRID: Region = Region(2951);
const R_SHEET: Region = Region(2952);
const SCROLL_ROWS: Region = Region(2960);
const SCROLL_TIME: Region = Region(2961);
const SCROLL_RULER: Region = Region(2962);
const BASE: u32 = 0x4755_0000;
const WATCH: Id = Id(BASE + 40);
const PLATE_BASE: u32 = BASE + 0x100;

/// The layers a row holds: a logo, the rail's words, the cells and the lead.
const ROW_LAYERS: usize = 4;
/// Rows made in one frame: a key that jumps a page makes the rest on the frames after it.
const BUILD_PER_FRAME: usize = 3;
/// Layers the scene must have left for a row to be made (the sheet, the plate and what a move makes).
const SPARE_LAYERS: usize = 12;
/// A run holds 256 quads; a layer asks for no more than this of them.
const QUADS: usize = 248;
/// Plates kept for the cells the focus has been on (they settle to rest and go).
const PLATES: usize = 6;

fn other(e: ui::error::UiError) -> io::Error {
    io::Error::other(format!("{e}"))
}

/// A text layer showing `paint`: made empty and drawn, so that a glyph the compositor has not yet been handed waits for it (a layer made with
/// its words is a picture of its whole frame when one is not ready).
fn text_layer(ui: &mut ui::Ui, frame: Frame, z: i32, paint: &Paint) -> io::Result<TextLayer> {
    let mut layer = TextLayer::new(ui, frame, z, &[])?;
    layer.draw(ui, &paint.items());
    Ok(layer)
}

fn chip_id(i: usize) -> Id {
    Id(BASE + 1 + i as u32)
}

/// Words and shapes to put in a text layer, in the order of their colours: a layer's inks are four at most.
#[derive(Default)]
struct Paint {
    runs: Vec<(String, Type, Rgba, f32, f32, f32)>,
    marks: Vec<Mark>,
}

impl Paint {
    fn text(&mut self, text: String, ty: Type, colour: Rgba, x: f32, y: f32, w: f32) {
        self.runs.push((text, ty, colour, x, y, w));
    }

    fn items(&self) -> Vec<Item<'_>> {
        let words = self.runs.iter().map(|(t, ty, c, x, y, w)| Item::Text(Run::left(t, *ty, *c, *x, *y, *w)));
        words.chain(self.marks.iter().map(|m| Item::Mark(*m))).collect()
    }

    fn quads(&self) -> usize {
        self.items().iter().map(Item::quads).sum()
    }
}

/// What a built row was drawn from: when any of it changes the row is drawn again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Drawn {
    rev: u64,
    view: i64,
    minute: i64,
}

/// A word as the screen has it, for the lint and the geometry report: its box on the screen at rest, with the grid scrolled to the view, and
/// the frame it must stay inside where it has one (the part of its cell the view shows, a logo's tile).
#[derive(Clone, Debug)]
struct Said {
    text: String,
    frame: Frame,
    ty: Type,
    colour: Rgba,
    within: Option<Frame>,
}

impl Said {
    fn new(text: String, frame: Frame, ty: Type, colour: Rgba) -> Self {
        Self { text, frame, ty, colour, within: None }
    }

    fn within(self, bounds: Frame) -> Self {
        Self { within: Some(bounds), ..self }
    }
}

struct Row {
    index: usize,
    id: String,
    _logo: ImageLayer,
    _rail: TextLayer,
    cells: TextLayer,
    lead: TextLayer,
    drawn: Option<Drawn>,
    /// The cell the lead shows (its start), once the row has been drawn; and whether the lead waits, hidden, for the time to stop moving.
    lead_of: Option<Option<i64>>,
    lead_wait: bool,
    /// The words of the rail, and of the cells and the lead, as drawn.
    rail_words: Vec<Said>,
    words: Vec<Said>,
    /// The hairlines and rules of the cells, as the screen has them at rest like the words: no word is on one.
    marks: Vec<Frame>,
}

struct ChipView {
    chip: Chip,
    label: String,
    /// Whether it is the chosen filter, as it was made.
    on: bool,
    button: Button,
}

struct Groups {
    _body: Group,
    rows: Group,
    grid_clip: Group,
    grid: Group,
    _ruler_clip: Group,
    _ruler: Group,
}

struct Ruler {
    layer: TextLayer,
    drawn: Option<(i64, i64, i64)>,
    words: Vec<Said>,
    /// The ticks and the marker of now, on the screen at rest.
    marks: Vec<Frame>,
}

struct Strip {
    _pane: Pane,
    text: TextLayer,
    sig: String,
    words: Vec<Said>,
}

struct SheetBuilt {
    sig: String,
    frame: Frame,
    _pane: Pane,
    _text: TextLayer,
    watch: Button,
    _dim: ShapeLayer,
    words: Vec<Said>,
}

struct Chrome {
    lay: Layout,
    groups: Groups,
    _title: Text,
    chips: Vec<ChipView>,
    ruler: Ruler,
    strip: Strip,
    _hints: Vec<HintRow>,
    rows: Vec<Row>,
    /// What the grid says when it has no rows: the words and the layer.
    empty: Option<(String, TextLayer, Said)>,
    sheet: Option<SheetBuilt>,
}

/// Where the lit plate is: the row, the stretch of time it covers and the view it was made for (the plate keeps to the part of the cell the view
/// shows).
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlateKey {
    channel: String,
    from: i64,
    to: i64,
    view: i64,
}

pub struct GuideSurface {
    stage: Stage,
    z: i32,
    layout: Option<((u64, u32), Layout)>,
    chrome: Option<Chrome>,
    made_for: (u64, u32),
    /// The scroll targets last sent to the tracks (rows, time).
    sent: Option<(f32, f32)>,
    plates: Vec<(PlateKey, Plate)>,
    serial: u32,
    hovered: Option<Target>,
    pressed: Option<Target>,
    /// Rows are still to be made (the next frame does them).
    pending: bool,
}

impl GuideSurface {
    /// `z` is the guide's band base (`system_z + kit::z::GUIDE`).
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).hiding(), z, layout: None, chrome: None, made_for: (0, 0), sent: None, plates: Vec::new(), serial: 0, hovered: None, pressed: None, pending: false }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    /// Rows are still to be made: the next frame makes them.
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// The rows and the minutes the grid holds at this text size and theme: the model is told them before it is drawn.
    pub fn page(&mut self, kit: &mut Kit) -> Page {
        self.layout(kit).page()
    }

    fn layout(&mut self, kit: &mut Kit) -> Layout {
        let key = (kit.generation(), kit.text_scale.to_bits());
        if let Some((k, l)) = &self.layout
            && *k == key
        {
            return l.clone();
        }
        let l = layout::plan(kit);
        self.layout = Some((key, l.clone()));
        l
    }

    /// How many layers the guide holds (the scene's ceiling is 256, shared with every kind).
    pub fn layers(&self) -> usize {
        let Some(c) = &self.chrome else { return 0 };
        let chips: usize = c.chips.iter().map(|v| v.button.layers()).sum();
        let sheet = c.sheet.as_ref().map_or(0, |s| 3 + s.watch.layers());
        // the dim behind the page, the title, the ruler, the strip's glass and words, the three hints
        1 + 1 + chips + 1 + 2 + 3 + c.rows.len() * ROW_LAYERS + self.plates.len() + sheet + usize::from(c.empty.is_some())
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        if let Some(c) = self.chrome.take() {
            for v in c.chips {
                v.button.destroy(engine);
            }
            if let Some(s) = c.sheet {
                s.watch.destroy(engine);
            }
        }
        for (_, p) in self.plates.drain(..) {
            p.destroy(engine);
        }
        for region in [SCROLL_ROWS, SCROLL_TIME, SCROLL_RULER] {
            engine.flow.drop_scroller(region);
        }
        self.stage.drop_all(engine);
        self.sent = None;
        self.hovered = None;
        self.pressed = None;
        self.pending = false;
    }

    /// A new look or text size: everything is made again.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.layout = None;
        self.drop_all(engine);
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
        self.hovered = None;
    }

    /// Whether another frame is wanted: rows still to be made, or a scroll or a lift still moving.
    pub fn busy(&self, engine: &Engine, now_ns: u64) -> bool {
        self.pending || (self.is_up() && engine.flow.settling_at(now_ns))
    }

    // ------------------------------------------------------------------------------------------------------------------------ building

    fn make_chips(kit: &mut Kit, engine: &mut Engine, root: &Group, lay: &Layout, g: &Guide, z: i32) -> io::Result<Vec<ChipView>> {
        let scene = kit.ui.scene.clone();
        let (s3, s6) = (kit.space(3), kit.space(6));
        let (btn_h, mut x) = (Button::height(kit), lay.chips_x);
        let mut out = Vec::new();
        for (i, chip) in g.chips().into_iter().enumerate() {
            let label = chip_label(g, chip);
            let on = matches!(chip, Chip::Filter(f) if f == g.filter());
            // every filter chip has the room of the check that marks the chosen one (meaning survives without colour), so none moves when
            // the choice does
            let w = Button::width_for(kit, &label, matches!(chip, Chip::Filter(_)));
            if chip == Chip::Now {
                x += s6;
            }
            let frame = Frame::new(x, lay.chips_y, w, btn_h);
            let mut spec = ButtonSpec::new(&label, Tone::Plain).selected(on);
            if on {
                spec = spec.with_icon("check");
            }
            let place = Place { id: chip_id(i), frame, region: R_CHIPS, scope: SCOPE, z };
            let mut button = scene.adopting(root, || Button::new(kit, engine, place, spec))?;
            button.show(kit, true);
            out.push(ChipView { chip, label, on, button });
            x += w + s3;
        }
        Ok(out)
    }

    fn build_chrome(&mut self, kit: &mut Kit, engine: &mut Engine, g: &Guide, lay: Layout) -> io::Result<()> {
        let room = kit.room();
        if room.layers < 40 || room.groups < 24 || room.tracks < 12 {
            return Err(io::Error::other(format!("no room for the guide in the scene ({} layers, {} groups, {} tracks left)", room.layers, room.groups, room.tracks)));
        }
        let scene = kit.ui.scene.clone();
        let z = self.z;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let (s3, s4, s6) = (kit.space(3), kit.space(4), kit.space(6));
        let bar_h = (lay.row_h - 2.0 * lay.mark_inset) as u32;
        // the dividers, the ticks and the marker of now are marks of runs: handed to the compositor before they are made
        kit.ui.warm_marks(&[Form::Bar { w: lay.hair as u32, h: bar_h }, Form::Rule(1), Form::Disc(lay.knob as u32), Form::Bar { w: lay.hair as u32, h: lay.tick_h as u32 }]);

        let root = self.stage.group();
        let root_id = root.id();
        let body = scene
            .group(Some(root_id), GroupPose { clip: Some(Rect { x: layout::LEFT, y: lay.grid.y, w: layout::RIGHT - layout::LEFT, h: lay.grid.h }), ..GroupPose::IDENTITY })
            .map_err(other)?;
        let rows_rest = GroupPose { y: lay.grid.y, ..GroupPose::IDENTITY };
        let rows = scene.group(Some(body.id()), rows_rest).map_err(other)?;
        let grid_clip = scene
            .group(Some(rows.id()), GroupPose { clip: Some(Rect { x: lay.grid.x, y: 0.0, w: lay.grid.w, h: layout::EXTENT_MAX }), ..GroupPose::IDENTITY })
            .map_err(other)?;
        let time_rest = GroupPose { x: lay.grid.x, ..GroupPose::IDENTITY };
        let grid = scene.group(Some(grid_clip.id()), time_rest).map_err(other)?;
        let ruler_clip = scene
            .group(Some(root_id), GroupPose { clip: Some(Rect { x: lay.grid.x, y: lay.ruler.y, w: lay.grid.w, h: lay.ruler.h }), feather: Edges::NONE, ..GroupPose::IDENTITY })
            .map_err(other)?;
        let ruler = scene.group(Some(ruler_clip.id()), time_rest).map_err(other)?;
        engine.flow.scroller(&scene, SCROLL_ROWS, &rows, rows_rest, false, layout::EXTENT_MAX).map_err(other)?;
        engine.flow.scroller(&scene, SCROLL_TIME, &grid, time_rest, true, layout::EXTENT_MAX).map_err(other)?;
        engine.flow.scroller(&scene, SCROLL_RULER, &ruler, time_rest, true, layout::EXTENT_MAX).map_err(other)?;

        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let title = scene.adopting(root, || Text::new(kit, lay.title, z + 12, title_style, "Guide"))?;
        let chips = Self::make_chips(kit, engine, root, &lay, g, z + 12)?;

        let ruler_layer = scene.adopting(&ruler, || TextLayer::new(&mut kit.ui, Frame::new(0.0, lay.ruler.y, lay.grid.w * 2.0, lay.ruler.h), z + 7, &[]))?;
        let pane = scene.adopting(root, || Pane::new(kit, lay.strip, z + 5, PaneKind::Strip, false));
        let strip_frame = Frame::new(lay.strip.x + s4, lay.strip.y + s3, lay.strip.w - 2.0 * s4, lay.strip.h - 2.0 * s3);
        let strip_text = scene.adopting(root, || TextLayer::new(&mut kit.ui, strip_frame, z + 6, &[]))?;

        // the keys as glyphs: the arrows move, OK opens, Back leaves
        let mut hints = Vec::new();
        let mut hx = lay.hints.x;
        let list = [
            Hint { keys: vec![(Key::Up, false), (Key::Down, false), (Key::Left, false), (Key::Right, false)], action: Some(Action::Move), label: None },
            Hint::key(Key::Ok, Action::Open),
            Hint::key(Key::Back, Action::Close),
        ];
        for hint in &list {
            let at = RowAt { x: hx, y: lay.hints.y, h: lay.hints.h, z: z + 12 };
            let row = scene.adopting(root, || HintRow::new(kit, hint, TypeRole::Caption, at, ink2))?;
            hx += row.width() + 2.0 * s6;
            hints.push(row);
        }

        self.chrome = Some(Chrome {
            lay,
            groups: Groups { _body: body, rows, grid_clip, grid, _ruler_clip: ruler_clip, _ruler: ruler },
            _title: title,
            chips,
            ruler: Ruler { layer: ruler_layer, drawn: None, words: Vec::new(), marks: Vec::new() },
            strip: Strip { _pane: pane, text: strip_text, sig: String::new(), words: Vec::new() },
            _hints: hints,
            rows: Vec::new(),
            empty: None,
            sheet: None,
        });
        self.made_for = (kit.generation(), kit.text_scale.to_bits());
        self.sent = None;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------------------------------ applying

    /// Draws what changed. Returns whether any of the guide is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, g: &Guide, now_ns: u64) -> io::Result<bool> {
        let phase = g.phase();
        if !g.built() {
            if self.is_up() {
                self.drop_all(engine);
            }
            return Ok(false);
        }
        let lay = self.layout(kit);
        if self.chrome.is_some() && self.made_for != (kit.generation(), kit.text_scale.to_bits()) {
            self.drop_all(engine);
        }
        self.stage.ensure(kit, phase, Arrive::Fade, true)?;
        if self.chrome.is_none()
            && let Err(e) = self.build_chrome(kit, engine, g, lay)
        {
            self.drop_all(engine);
            return Err(e);
        }
        let up = self.stage.phase(kit, engine, phase);
        if !up {
            self.pending = false;
            if phase == Phase::Parked {
                // the next showing is a new one: its scroll is set, not flown to
                self.sent = None;
            }
            return Ok(phase != Phase::Parked && phase != Phase::Gone);
        }
        self.sync_chips(kit, engine, g)?;
        let first = self.sent.is_none();
        self.sync_scroll(engine, g, first);
        self.sync_ruler(kit, g)?;
        let est = engine.flow.scroll_at(SCROLL_ROWS, now_ns).unwrap_or(0.0);
        // the time is flying while its spring is moving and has not yet reached the view the model has
        let time = self.chrome.as_ref().map_or(0.0, |c| c.lay.offset_of(g.view(), g.origin()));
        let flying = engine.flow.settling_at(now_ns) && (engine.flow.scroll_at(SCROLL_TIME, now_ns).unwrap_or(time) - time).abs() > 0.5;
        self.sync_rows(kit, g, est, flying)?;
        self.sync_empty(kit, g)?;
        self.sync_strip(kit, g)?;
        self.sync_sheet(kit, engine, g)?;
        let want = self.sync_plate(kit, engine, g, now_ns)?;
        self.stage.focus(engine, want, now_ns);
        engine.flow.tidy(now_ns);
        self.drop_resting(engine, want, now_ns);
        Ok(true)
    }

    fn sync_chips(&mut self, kit: &mut Kit, engine: &mut Engine, g: &Guide) -> io::Result<()> {
        let c = self.chrome.as_mut().expect("built");
        let chips = g.chips();
        let same = chips.len() == c.chips.len() && chips.iter().zip(&c.chips).all(|(a, v)| *a == v.chip && chip_label(g, *a) == v.label && v.on == matches!(a, Chip::Filter(f) if *f == g.filter()));
        if !same {
            // the set of chips changed (the internet's list came, the favourites are gone) or the chosen one did: they are made again
            for v in c.chips.drain(..) {
                v.button.destroy(engine);
            }
            c.chips = Self::make_chips(kit, engine, self.stage.group(), &c.lay, g, self.z + 12)?;
        }
        Ok(())
    }

    /// The tracks follow what the model asks of the view: the rows by whole rows, the time by whole pixels. A jump of more than two screens
    /// (the Now chip from a day away, a new filter's rows) is a cut: nobody can follow what a spring would fly over, and its rows would
    /// be made to be gone again.
    fn sync_scroll(&mut self, engine: &mut Engine, g: &Guide, snap: bool) {
        let c = self.chrome.as_ref().expect("built");
        let rows = (g.top() as f32 * c.lay.row_h).min(layout::EXTENT_MAX);
        let time = c.lay.offset_of(g.view(), g.origin());
        let to = (rows, time);
        if self.sent == Some(to) {
            return;
        }
        let (far_rows, far_time) = self.sent.map_or((true, true), |(r, t)| ((rows - r).abs() > 2.0 * c.lay.grid.h, (time - t).abs() > 2.0 * c.lay.grid.w));
        for (region, offset, far) in [(SCROLL_ROWS, rows, far_rows), (SCROLL_TIME, time, far_time), (SCROLL_RULER, time, far_time)] {
            let _ = if snap || far { engine.flow.scroll_snap(region, offset) } else { engine.flow.scroll_to(region, offset) };
        }
        self.sent = Some(to);
    }

    /// The times over the grid, from half an hour to half an hour, for the stretch of time round the view: the words and the ticks in the upper
    /// band, the marker of now (a disc that ends where the grid begins) in the band under it, so that no word is ever on it.
    fn sync_ruler(&mut self, kit: &mut Kit, g: &Guide) -> io::Result<()> {
        let c = self.chrome.as_mut().expect("built");
        let (lay, view, origin) = (&c.lay, g.view(), g.origin());
        let span = lay.page().span;
        let from = (view - span / 2).max(origin);
        let to = (view + span + span / 2).min(g.horizon());
        let key = (from, view, g.now() / MIN);
        if c.ruler.drawn == Some(key) {
            return Ok(());
        }
        let (ink, ink2, accent, edge_colour) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), edge(kit));
        let ty = kit.ty(TypeRole::Caption);
        let line = line_height(ty);
        // a time begins its words where a cell that begins at that time begins its own: a cell's air
        let (pad, y) = (lay.pad, ((lay.labels_h - line) / 2.0).floor());
        let x0 = lay.x_of(from, origin);
        let offset = lay.offset_of(view, origin);
        let screen_x = |x: f32| lay.grid.x + x0 + x - offset;
        let mut paint = Paint::default();
        let mut words = Vec::new();
        let mut marks = Vec::new();
        let half = 30 * MIN;
        let mut t = (from + half - 1).div_euclid(half) * half;
        while t < to {
            let x = lay.x_of(t, origin) - x0;
            let c0 = civil_of(t, g.offset_s());
            let at_midnight = c0.hour == 0 && c0.minute == 0;
            let text = if at_midnight { day_text(t, g.offset_s()) } else { clock_text(t, g.offset_s()) };
            let colour = if at_midnight { ink } else { ink2 };
            let w = width_of(kit, role_style(kit, TypeRole::Caption, None, colour, Align::Left), &text);
            // The words of a time begin where the words of a cell that begins then begin: a cell's air from the tick, and no nearer the left
            // edge than the first column's titles. A time whose tick is out of the grid has words only in a flight, outside it; one the right
            // edge would cut has none when the grid is at rest, and no tick either (a tick alone at the edge says nothing).
            let at = x0 + x;
            let a = if at >= offset { (at + pad).max(offset + lay.clear() + pad) } else { at + pad };
            let b = a + w;
            let cut = |edge: f32| a < edge && b > edge;
            if !((at < offset && b > offset) || cut(offset + lay.grid.w)) {
                words.push(Said::new(text.clone(), Frame::new(screen_x(a - x0), lay.ruler.y + y, w, line), ty, colour));
                paint.text(text, ty, colour, a - x0, y, w);
                // a tick is centred on its time, as the divider under it is
                let tick = (x - lay.hair / 2.0).floor();
                paint.marks.push(Mark::bar(tick as i32, 0, lay.hair as u32, lay.tick_h as u32, edge_colour));
                marks.push(Frame::new(screen_x(tick), lay.ruler.y, lay.hair, lay.tick_h));
            }
            t += half;
        }
        let now_x = lay.x_of(g.now(), origin) - x0;
        if (0.0..=lay.grid.w * 2.0).contains(&now_x) {
            let at = (now_x - lay.knob / 2.0).floor();
            paint.marks.push(Mark::disc(at as i32, (lay.ruler.h - lay.knob) as i32, lay.knob as u32, accent));
            marks.push(Frame::new(screen_x(at), lay.ruler.y + lay.ruler.h - lay.knob, lay.knob, lay.knob));
        }
        c.ruler.layer.draw(&mut kit.ui, &paint.items());
        c.ruler.layer.set_props(Props::at(x0, lay.ruler.y));
        c.ruler.drawn = Some(key);
        c.ruler.words = words;
        c.ruler.marks = marks;
        Ok(())
    }

    fn sync_rows(&mut self, kit: &mut Kit, g: &Guide, est: f32, flying: bool) -> io::Result<()> {
        let c = self.chrome.as_mut().expect("built");
        let lay = c.lay.clone();
        let n = g.rows().min(lay.most_rows());
        // the rows the screen shows now and the rows it will show when the scroll has settled, a row beside each: what a spring flies over
        // between them is not held, so a page of CH+ holds two screens and a held key holds no more
        let window = |at: f32| (((at / lay.row_h).floor() as isize - 1).max(0) as usize, (((at + lay.grid.h) / lay.row_h).ceil() as usize + 1).min(n));
        let windows = [window(est), window(g.top() as f32 * lay.row_h)];
        let near = |i: usize, spare: usize| windows.iter().any(|&(first, last)| i + spare >= first && i < last + spare);
        // a row that is not the channel the model has there any more, or has left the windows with room to spare, goes
        c.rows.retain(|r| r.index < n && g.channel(r.index).is_some_and(|ch| ch.id == r.id) && near(r.index, 2));
        let mut wanted: Vec<usize> = windows.iter().flat_map(|&(first, last)| first..last).filter(|i| !c.rows.iter().any(|r| r.index == *i)).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let focus = g.row();
        wanted.sort_by_key(|i| i.abs_diff(focus));
        self.pending = false;
        for (made, index) in wanted.into_iter().enumerate() {
            if made >= BUILD_PER_FRAME {
                self.pending = true;
                break;
            }
            if kit.room().layers < ROW_LAYERS + SPARE_LAYERS {
                break;
            }
            let row = self.build_row(kit, g, &lay, index)?;
            self.chrome.as_mut().expect("built").rows.push(row);
        }
        let c = self.chrome.as_mut().expect("built");
        c.rows.sort_by_key(|r| r.index);
        let come_in = kit.beat_ms(Beat::Label);
        for r in &mut c.rows {
            let want = Drawn { rev: g.rev(&r.id), view: g.view(), minute: g.now() / MIN };
            if r.drawn != Some(want) {
                draw_cells(kit, g, &lay, r, flying);
                r.drawn = Some(want);
            }
            if r.lead_wait && !flying {
                r.lead_wait = false;
                r.lead.animate_props(Props::at(lay.grid.x, lay.row_h * r.index as f32), come_in, Ease::Out);
            }
        }
        Ok(())
    }

    fn build_row(&mut self, kit: &mut Kit, g: &Guide, lay: &Layout, index: usize) -> io::Result<Row> {
        let ch = g.channel(index).expect("a row of the model").clone();
        let scene = kit.ui.scene.clone();
        let z = self.z;
        let c = self.chrome.as_ref().expect("built");
        let y = lay.row_h * index as f32;
        let s2 = kit.space(2);
        // the logo on a quiet tile with a hairline edge, one layer; the tile starts where every block of the left edge does
        let (lw, lh) = lay.logo;
        let tile_y = ((lay.row_h - lh as f32) / 2.0).floor();
        let mut px = Px::new(lw, lh);
        px.round_rect(Frame::new(0.0, 0.0, lw as f32, lh as f32), kit.tokens.radius(Radius::Control).min(lh as f32 / 2.0), selected_wash(kit), Some((edge(kit), 1.0)));
        let art = ch.logo.as_deref().and_then(|file| crate::kit::avatar::contained(file, lw.saturating_sub(8), lh.saturating_sub(8)));
        if let Some(p) = &art {
            px.lay(&p.rgba, p.width, p.height, ((lw - p.width) / 2) as i32, ((lh - p.height) / 2) as i32);
        }
        let at = Props::at(layout::LEFT, y + tile_y);
        let logo = scene.adopting(&c.groups.rows, || px.layer(kit, z + 3, at))?;

        // the rail's words stand beside the tile; a channel with no logo says its call sign on the tile
        let (ink2, ink) = (kit.ui.look.ink2(), kit.ui.look.ink());
        let (num_ty, name_ty) = (kit.ty(TypeRole::Label), kit.ty(TypeRole::Caption));
        let (num_line, name_line) = (line_height(num_ty), line_height(name_ty));
        let beside = lay.text_x - layout::LEFT;
        let number = ellipsise(&mut kit.ui.fonts, &ch.number, num_ty, lay.text_w);
        let mut paint = Paint::default();
        let mut said = Vec::new();
        if lay.two_lines {
            let top = ((lay.row_h - num_line - name_line) / 2.0).floor();
            let name = ellipsise(&mut kit.ui.fonts, &ch.name, name_ty, lay.text_w);
            said.push(Said::new(number.clone(), Frame::new(lay.text_x, y + top, lay.text_w, num_line), num_ty, ink));
            said.push(Said::new(name.clone(), Frame::new(lay.text_x, y + top + num_line, lay.text_w, name_line), name_ty, ink2));
            paint.text(number, num_ty, ink, beside, top, lay.text_w);
            paint.text(name, name_ty, ink2, beside, top + num_line, lay.text_w);
        } else {
            let top = ((lay.row_h - num_line) / 2.0).floor();
            said.push(Said::new(number.clone(), Frame::new(lay.text_x, y + top, lay.text_w, num_line), num_ty, ink));
            paint.text(number, num_ty, ink, beside, top, lay.text_w);
        }
        if art.is_none() {
            let style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
            let mut sign = monogram(&ch.name);
            while sign.chars().count() > 1 && width_of(kit, style, &sign) > lw as f32 - 2.0 * s2 {
                sign.pop();
            }
            if !sign.is_empty() {
                let w = width_of(kit, style, &sign);
                let (mx, my) = (((lw as f32 - w) / 2.0).floor(), tile_y + ((lh as f32 - name_line) / 2.0).floor());
                said.push(Said::new(sign.clone(), Frame::new(layout::LEFT + mx, y + my, w, name_line), name_ty, ink2).within(Frame::new(layout::LEFT, y + tile_y, lw as f32, lh as f32)));
                paint.text(sign, name_ty, ink2, mx, my, w);
            }
        }
        let rail_frame = Frame::new(layout::LEFT, y, lay.text_x + lay.text_w - layout::LEFT, lay.row_h);
        let rail = scene.adopting(&c.groups.rows, || text_layer(&mut kit.ui, rail_frame, z + 4, &paint))?;
        let cells = scene.adopting(&c.groups.grid, || TextLayer::new(&mut kit.ui, Frame::new(0.0, y, lay.grid.w * 2.0, lay.row_h), z + 2, &[]))?;
        let lead = scene.adopting(&c.groups.grid_clip, || TextLayer::new(&mut kit.ui, Frame::new(lay.grid.x, y, lay.grid.w, lay.row_h), z + 3, &[]))?;
        Ok(Row { index, id: ch.id, _logo: logo, _rail: rail, cells, lead, drawn: None, lead_of: None, lead_wait: false, rail_words: said, words: Vec::new(), marks: Vec::new() })
    }

    /// What the grid says when there is no row: the list is on its way, did not come, or is empty.
    fn sync_empty(&mut self, kit: &mut Kit, g: &Guide) -> io::Result<()> {
        let c = self.chrome.as_mut().expect("built");
        let words = if g.rows() > 0 {
            None
        } else if let Some(f) = g.list_fault() {
            Some(f.words())
        } else if g.loading_list() {
            Some("Loading channels")
        } else {
            Some("No channels")
        };
        match (words, c.empty.as_ref()) {
            (None, _) => c.empty = None,
            (Some(w), Some((have, ..))) if have == w => {}
            (Some(w), _) => {
                let scene = kit.ui.scene.clone();
                let (ty, ink2) = (kit.ty(TypeRole::Body), kit.ui.look.ink2());
                let line = line_height(ty);
                let width = width_of(kit, role_style(kit, TypeRole::Body, None, ink2, Align::Left), w);
                let frame = Frame::new((c.lay.grid.x + (c.lay.grid.w - width) / 2.0).floor(), (c.lay.grid.y + (c.lay.grid.h - line) / 2.0).floor(), width, line);
                let mut paint = Paint::default();
                paint.text(w.to_string(), ty, ink2, 0.0, 0.0, width);
                let layer = scene.adopting(self.stage.group(), || text_layer(&mut kit.ui, frame, self.z + 6, &paint))?;
                c.empty = Some((w.to_string(), layer, Said::new(w.to_string(), frame, ty, ink2)));
            }
        }
        Ok(())
    }

    /// What the programme in focus is, under the grid: its title, when and where, and a few lines of what it is about.
    fn sync_strip(&mut self, kit: &mut Kit, g: &Guide) -> io::Result<()> {
        let c = self.chrome.as_mut().expect("built");
        let lay = c.lay.clone();
        let sig = strip_sig(g);
        if c.strip.sig == sig {
            return Ok(());
        }
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (body_ty, cap_ty) = (kit.ty(TypeRole::Body), kit.ty(TypeRole::Caption));
        let (body_line, cap_line) = (line_height(body_ty), line_height(cap_ty));
        let (s3, s4) = (kit.space(3), kit.space(4));
        let w = lay.strip.w - 2.0 * s4;
        let words = strip_words(g);
        let mut paint = Paint::default();
        let mut said = Vec::new();
        let (x, mut y) = (lay.strip.x + s4, lay.strip.y + s3);
        let mut add = |kit: &mut Kit, paint: &mut Paint, text: &str, ty: Type, colour: Rgba, line: f32, y: &mut f32| {
            let shown = ellipsise(&mut kit.ui.fonts, text, ty, w);
            said.push(Said::new(shown.clone(), Frame::new(x, *y, w, line), ty, colour));
            paint.text(shown, ty, colour, 0.0, *y - (lay.strip.y + s3), w);
            *y += line;
        };
        add(kit, &mut paint, &words.title, body_ty, if words.quiet { ink2 } else { ink }, body_line, &mut y);
        add(kit, &mut paint, &words.meta, cap_ty, ink2, cap_line, &mut y);
        if lay.desc_lines > 0 && !words.desc.is_empty() {
            let style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
            for line in wrap::lines(kit, style, &words.desc, w, lay.desc_lines) {
                add(kit, &mut paint, &line, cap_ty, ink2, cap_line, &mut y);
            }
        }
        c.strip.text.draw(&mut kit.ui, &paint.items());
        c.strip.sig = sig;
        c.strip.words = said;
        Ok(())
    }

    /// The detail sheet over the page: made when it opens, gone when it closes. The page behind it is hidden (the scrim that hides, as every
    /// sheet over words has), so no word of it reads through the sheet or is cut by its edge, and the sheet is as tall as what it says: the
    /// lines it has, never the lines it might have had.
    fn sync_sheet(&mut self, kit: &mut Kit, engine: &mut Engine, g: &Guide) -> io::Result<()> {
        let c = self.chrome.as_mut().expect("built");
        let want = g.sheet().map(|s| sheet_sig(g, &s));
        if c.sheet.as_ref().map(|s| &s.sig) == want.as_ref() {
            return Ok(());
        }
        if let Some(old) = c.sheet.take() {
            old.watch.destroy(engine);
        }
        let Some(sig) = want else { return Ok(()) };
        let s = g.sheet().expect("a sheet");
        let scene = kit.ui.scene.clone();
        let z = self.z + 20;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (title_ty, body_ty, cap_ty) = (kit.ty(TypeRole::Title), kit.ty(TypeRole::Body), kit.ty(TypeRole::Caption));
        let (title_line, body_line, cap_line) = (line_height(title_ty), line_height(body_ty), line_height(cap_ty));
        let (s3, s4, s6) = (kit.space(3), kit.space(4), kit.space(6));
        let scale = kit.text_scale.clamp(1.0, 2.0);
        // a reading measure that grows with the text but never past seven eighths of the safe width
        let base = 15.0 * kit.tokens.target;
        let w = (base * scale.powf(0.8)).round().clamp(base, ((layout::RIGHT - layout::LEFT) * 7.0 / 8.0).floor());
        let inner = w - 2.0 * s6;
        let btn_h = Button::height(kit);
        let room = layout::BOTTOM - layout::TOP;
        let title = s.main.map_or("No information", |p| p.title.as_str());
        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let title_lines = wrap::lines(kit, title_style, title, inner, 3);
        let (when, meta) = sheet_meta(g, &s);
        let head = if when.is_empty() { sheet_channel(&s) } else { format!("{}  ·  {}", sheet_channel(&s), when) };
        let after: Vec<String> = s.after.iter().take(2).map(|p| format!("{}  {}", clock_text(p.start, g.offset_s()), p.title)).collect();
        let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
        let text = s.main.map_or("", |p| p.text.as_str());
        // the programmes after give way first and then the description's lines, before the title does
        let meta_h = if meta.is_empty() { 0.0 } else { cap_line };
        let (mut desc_max, mut after_n) = (if text.is_empty() { 0 } else { 5 }, after.len());
        let (desc, after_n, h) = loop {
            let desc = if desc_max > 0 { wrap::lines(kit, body_style, text, inner, desc_max) } else { Vec::new() };
            let h = (2.0 * s6 + cap_line + title_lines.len() as f32 * title_line + meta_h + desc.len() as f32 * body_line + if after_n > 0 { s3 } else { 0.0 } + after_n as f32 * cap_line + s4 + btn_h).ceil();
            if h <= room || (desc_max == 0 && after_n == 0) {
                break (desc, after_n, h);
            }
            if after_n > 0 {
                after_n -= 1;
            } else {
                desc_max -= 1;
            }
        };
        let frame = Frame::new(((1920.0 - w) / 2.0).floor(), ((1080.0 - h) / 2.0).floor(), w, h);
        let mut paint = Paint::default();
        let mut said = Vec::new();
        let (x, mut y) = (frame.x + s6, frame.y + s6);
        let mut put = |kit: &mut Kit, paint: &mut Paint, text: String, ty: Type, colour: Rgba, line: f32, y: &mut f32| {
            let shown = ellipsise(&mut kit.ui.fonts, &text, ty, inner);
            said.push(Said::new(shown.clone(), Frame::new(x, *y, inner, line), ty, colour));
            paint.text(shown, ty, colour, x - frame.x, *y - frame.y, inner);
            *y += line;
        };
        put(kit, &mut paint, head, cap_ty, ink2, cap_line, &mut y);
        for l in &title_lines {
            put(kit, &mut paint, l.clone(), title_ty, ink, title_line, &mut y);
        }
        if !meta.is_empty() {
            put(kit, &mut paint, meta, cap_ty, ink2, cap_line, &mut y);
        }
        for l in desc {
            put(kit, &mut paint, l, body_ty, ink2, body_line, &mut y);
        }
        if after_n > 0 {
            y += s3;
        }
        for l in after.iter().take(after_n) {
            put(kit, &mut paint, l.clone(), cap_ty, ink2, cap_line, &mut y);
        }
        let dim = scene.adopting(self.stage.group(), || scrim(kit, z, true));
        scrim_set(&dim, true);
        let pane = scene.adopting(self.stage.group(), || Pane::new(kit, frame, z + 1, PaneKind::Sheet, false));
        let layer_frame = frame;
        let text_layer = scene.adopting(self.stage.group(), || text_layer(&mut kit.ui, layer_frame, z + 2, &paint))?;
        let label = if s.main.is_some_and(|p| p.airing(g.now())) { "Watch" } else { "Tune in" };
        let bw = Button::width_for(kit, label, false);
        let place = Place { id: WATCH, frame: Frame::new(frame.x + s6, y + s4, bw, btn_h), region: R_SHEET, scope: SCOPE, z: z + 3 };
        let mut watch = scene.adopting(self.stage.group(), || Button::new(kit, engine, place, ButtonSpec::new(label, Tone::Primary)))?;
        watch.show(kit, true);
        c.sheet = Some(SheetBuilt { sig, frame, _pane: pane, _text: text_layer, watch, _dim: dim, words: said });
        Ok(())
    }

    /// The plate under the cell the focus is on, and which focusable the focus is on: the plate, a chip or the sheet's button.
    fn sync_plate(&mut self, kit: &mut Kit, engine: &mut Engine, g: &Guide, now_ns: u64) -> io::Result<Option<Id>> {
        use super::model::Region as Where;
        if g.sheet().is_some() {
            return Ok(Some(WATCH));
        }
        if g.region() == Where::Chips {
            return Ok(Some(chip_id(g.chip_index())));
        }
        let c = self.chrome.as_ref().expect("built");
        let (Some(ch), Some((from, to))) = (g.channel(g.row()), g.focus_extent()) else { return Ok(None) };
        if !c.rows.iter().any(|r| r.index == g.row()) {
            return Ok(self.plates.last().map(|(_, p)| p.id));
        }
        let key = PlateKey { channel: ch.id.clone(), from, to, view: g.view() };
        if let Some((_, p)) = self.plates.iter().find(|(k, _)| *k == key) {
            return Ok(Some(p.id));
        }
        // a new plate for the new cell; the oldest that is at rest makes room
        if self.plates.len() >= PLATES
            && let Some(i) = self.plates.iter().position(|(_, p)| engine.flow.resting(p.id, now_ns))
        {
            let (_, p) = self.plates.remove(i);
            p.destroy(engine);
        }
        let lay = &c.lay;
        let origin = g.origin();
        let left = lay.offset_of(g.view(), origin);
        let lifted = plate_frame(lay, g.row(), lay.x_of(from, origin), lay.x_of(to, origin), left);
        // at rest the plate is what the theme's lift grows to `lifted`, whatever the cell's width, so it never stands on the next cell
        let scale = Lift::Row.scale(kit);
        let (w, h) = ((lifted.w / scale).round(), (lifted.h / scale).round());
        let local = Frame::new((lifted.x + (lifted.w - w) / 2.0).round(), (lifted.y + (lifted.h - h) / 2.0).round(), w, h);
        let screen = Frame::new(lay.grid.x + local.x - left, lay.grid.y + local.y - lay.row_h * g.top() as f32, w, h);
        self.serial = (self.serial + 1) & 0xFFFF;
        let id = Id(PLATE_BASE + self.serial);
        let spot = Spot { id, screen, local, region: R_GRID, scope: SCOPE, z: self.z + 1 };
        let radius = kit.tokens.radius(Radius::Row).min(local.h / 2.0);
        let plate = Plate::new(kit, engine, &c.groups.grid, spot, Lift::Row, radius)?;
        self.plates.push((key, plate));
        Ok(Some(id))
    }

    /// Plates the focus has left settle to rest and go.
    fn drop_resting(&mut self, engine: &mut Engine, want: Option<Id>, now_ns: u64) {
        let mut i = 0;
        while i < self.plates.len() {
            let id = self.plates[i].1.id;
            if Some(id) != want && engine.flow.resting(id, now_ns) {
                let (_, p) = self.plates.remove(i);
                p.destroy(engine);
            } else {
                i += 1;
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------------------ pointer

    /// What is under the pointer: a chip, a cell of a row, a channel in the rail, the sheet's button or the page behind the sheet.
    fn hit(&self, g: &Guide, x: f32, y: f32) -> Option<Target> {
        let c = self.chrome.as_ref()?;
        if let Some(s) = &c.sheet {
            return if s.watch.frame().contains(x, y) {
                Some(Target::Watch)
            } else if s.frame.contains(x, y) {
                None
            } else {
                Some(Target::Dismiss)
            };
        }
        if let Some(i) = c.chips.iter().position(|v| v.button.frame().contains(x, y)) {
            return Some(Target::Chip(i));
        }
        let lay = &c.lay;
        if !lay.grid.contains(x, y) && !lay.rail.contains(x, y) {
            return None;
        }
        let (_, ly) = c.groups.rows.to_local(x, y);
        let row = (ly / lay.row_h).floor();
        if row < 0.0 || row as usize >= g.rows() {
            return None;
        }
        let row = row as usize;
        if lay.grid.contains(x, y) {
            let (gx, _) = c.groups.grid.to_local(x, y);
            let at = lay.t_at(gx, g.origin()).clamp(g.origin(), g.horizon() - 1);
            Some(Target::Cell { row, at })
        } else {
            Some(Target::Rail(row))
        }
    }

    /// A sample of the pointer: the cue follows it, and a press that ends on what it began on is a click.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, _now_ns: u64, s: Sample, g: &Guide) -> Option<Target> {
        self.chrome.as_ref()?;
        let at = if s.visible { self.hit(g, s.x, s.y) } else { None };
        if at != self.hovered {
            if let Some(c) = self.chrome.as_mut() {
                for (i, v) in c.chips.iter_mut().enumerate() {
                    v.button.set_hover(kit, matches!(at, Some(Target::Chip(j)) if j == i) && engine.focus.current() != Some(chip_id(i)));
                }
                if let Some(sheet) = c.sheet.as_mut() {
                    sheet.watch.set_hover(kit, matches!(at, Some(Target::Watch)) && engine.focus.current() != Some(WATCH));
                }
            }
            let cue = match at {
                Some(Target::Cell { row, at }) => self.cell_frame(g, row, at).map(|f| (f, kit.tokens.radius(Radius::Row).min(f.h / 2.0))),
                _ => None,
            };
            if let Some(c) = &self.chrome {
                self.stage.hover.show(kit, &c.groups.grid, cue);
            }
            self.hovered = at;
        }
        let down = s.visible && s.buttons & 1 != 0;
        match (down, self.pressed) {
            (true, None) => {
                self.pressed = at;
                None
            }
            (false, Some(began)) => {
                self.pressed = None;
                (Some(began) == at).then_some(began)
            }
            _ => None,
        }
    }

    /// The plate the cell at an instant of a row has at its full lift, in the space of the grid's content: what the focus and the pointer's
    /// cue both show.
    fn cell_frame(&self, g: &Guide, row: usize, at: i64) -> Option<Frame> {
        let c = self.chrome.as_ref()?;
        let ch = g.channel(row)?;
        let s = g.span_at(&ch.id, at);
        let (from, to) = match s.programme {
            Some(_) => (s.start, s.end),
            None => {
                let hour = at.div_euclid(HOUR) * HOUR;
                (s.start.max(hour), s.end.min(hour + HOUR))
            }
        };
        let (x0, x1) = (c.lay.x_of(from, g.origin()), c.lay.x_of(to, g.origin()));
        let left = c.lay.offset_of(g.view(), g.origin());
        Some(plate_frame(&c.lay, row, x0, x1, left))
    }

    // ------------------------------------------------------------------------------------------------------------------------ reports

    /// Where things are, for the gate and the harness.
    pub fn geometry(&self, engine: &Engine) -> Value {
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let Some(c) = &self.chrome else { return json!({"up": false}) };
        let lay = &c.lay;
        json!({
            "up": true,
            "layers": self.layers(),
            "rows": c.rows.iter().map(|r| r.index).collect::<Vec<_>>(),
            "plates": self.plates.len(),
            "leadsWaiting": c.rows.iter().filter(|r| r.lead_wait).count(),
            "page": {"rows": lay.rows, "minutes": lay.page().span / MIN, "rowH": lay.row_h, "ppm": lay.ppm},
            "head": f(lay.head), "ruler": f(lay.ruler), "rail": f(lay.rail), "grid": f(lay.grid), "strip": f(lay.strip), "hints": f(lay.hints),
            "chips": c.chips.iter().map(|v| json!({"label": v.label, "on": v.on, "frame": f(v.button.frame())})).collect::<Vec<_>>(),
            "sheet": c.sheet.as_ref().map(|s| f(s.frame)),
            "watch": c.sheet.as_ref().map(|s| f(s.watch.frame())),
            "focus": engine.focus.current().map(|id| id.0),
            "empty": c.empty.as_ref().map(|(w, ..)| w.as_str()),
        })
    }

    /// The words of the cells of the rows (row, words, box), as the screen has them at rest.
    #[cfg(test)]
    pub(super) fn cell_words(&self) -> Vec<(usize, String, Frame)> {
        let Some(c) = &self.chrome else { return Vec::new() };
        c.rows.iter().flat_map(|r| r.words.iter().map(|w| (r.index, w.text.clone(), w.frame))).collect()
    }

    /// The words of the rail (row, words): the number, the name, and the call sign on a tile with no logo.
    #[cfg(test)]
    pub(super) fn rail_words(&self) -> Vec<(usize, String)> {
        let Some(c) = &self.chrome else { return Vec::new() };
        c.rows.iter().flat_map(|r| r.rail_words.iter().map(|w| (r.index, w.text.clone()))).collect()
    }

    /// The words of the sheet, line by line.
    #[cfg(test)]
    pub(super) fn sheet_words(&self) -> Vec<String> {
        self.chrome.as_ref().and_then(|c| c.sheet.as_ref()).map_or_else(Vec::new, |s| s.words.iter().map(|w| w.text.clone()).collect())
    }

    /// Describes what is on the screen to a lint: the page, the words on it with the cells they are in, the lines of the grid, the plate of the
    /// focus and the buttons.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint, g: &Guide) {
        let Some(c) = &self.chrome else { return };
        let page = kit.tokens.extra(kit.mode).scrim_hide;
        let base = lint.panel("guide page", Frame::new(0.0, 0.0, 1920.0, 1080.0), page, false);
        let said = |kit: &mut Kit, lint: &mut Lint, name: &str, w: &Said, panel: usize| {
            let style = ui::components::style(w.ty, w.colour, Align::Left);
            match w.within {
                Some(bounds) => lint.word_within(kit, name, &w.text, w.frame, style, Some(panel), bounds),
                None => lint.word(kit, name, &w.text, w.frame, style, Some(panel)),
            }
        };
        // the sheet is over the page, which its scrim hides: what is under it is not read
        if let Some(s) = &c.sheet {
            let sheet = kit.ui.look.colour(ui::lucent::palette::PaletteRole::Sheet);
            let panel = lint.panel_on("sheet", s.frame, sheet, true, base);
            for w in &s.words {
                said(kit, lint, "sheet", w, panel);
            }
            s.watch.feed_lint(kit, lint, panel);
            return;
        }
        let ink = kit.ui.look.ink();
        let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        lint.word(kit, "title", "Guide", c.lay.title, title_style, Some(base));
        for v in &c.chips {
            v.button.feed_lint(kit, lint, base);
        }
        let grid = c.lay.grid;
        let (ruler, body) = (c.lay.ruler, c.lay.grid);
        for w in &c.ruler.words {
            if w.frame.x >= ruler.x && w.frame.right() <= ruler.right() {
                said(kit, lint, "ruler", w, base);
            }
        }
        for m in &c.ruler.marks {
            if m.x >= ruler.x && m.right() <= ruler.right() {
                lint.line("ruler mark", *m);
            }
        }
        let top = g.top() as f32 * c.lay.row_h;
        for r in &c.rows {
            let up = |f: Frame| Frame::new(f.x, body.y + f.y - top, f.w, f.h);
            for (w, in_the_grid) in r.rail_words.iter().map(|w| (w, false)).chain(r.words.iter().map(|w| (w, true))) {
                let at = Said { frame: up(w.frame), within: w.within.map(up), ..w.clone() };
                // the words of a cell the view does not reach are not on the screen at rest (they are in the run for a flight)
                let reached = !in_the_grid || at.within.is_some_and(|b| b.right() > grid.x && b.x < grid.right());
                if reached && at.frame.y >= body.y && at.frame.bottom() <= body.bottom() {
                    said(kit, lint, "cell", &at, base);
                }
            }
            for m in &r.marks {
                // a line the grid's edge cuts is a line that goes on: what shows of it is what words must keep off
                let f = up(*m);
                let (x0, x1) = (f.x.max(grid.x), f.right().min(grid.right()));
                if f.y >= body.y && f.bottom() <= body.bottom() && x1 > x0 {
                    lint.line("cell line", Frame::new(x0, f.y, x1 - x0, f.h));
                }
            }
        }
        // the plate of the focus, whole inside the grid at its full lift
        if g.region() == super::model::Region::Grid
            && let Some((from, to)) = g.focus_extent()
            && c.rows.iter().any(|r| r.index == g.row())
        {
            let (origin, left) = (g.origin(), c.lay.offset_of(g.view(), g.origin()));
            let f = plate_frame(&c.lay, g.row(), c.lay.x_of(from, origin), c.lay.x_of(to, origin), left);
            lint.lifted("focus plate", Frame::new(grid.x + f.x - left, body.y + f.y - top, f.w, f.h), Some(grid));
        }
        for w in &c.strip.words {
            said(kit, lint, "strip", w, base);
        }
        if let Some((_, _, w)) = &c.empty {
            said(kit, lint, "empty", w, base);
        }
    }
}

fn chip_label(g: &Guide, chip: Chip) -> String {
    match chip {
        Chip::Filter(Filter::All) => "All".into(),
        Chip::Filter(Filter::Antenna) => g.antenna_label().into(),
        Chip::Filter(Filter::Internet) => "Internet".into(),
        Chip::Filter(Filter::Favourites) => "Favourites".into(),
        Chip::Now => "Now".into(),
    }
}

fn civil_of(t: i64, offset_s: i32) -> crate::host::clock::Civil {
    crate::host::clock::civil_at(t.max(0) as u64, offset_s)
}

/// `9:52 PM` on the broadcast's clock.
pub fn clock_text(t: i64, offset_s: i32) -> String {
    let c = civil_of(t, offset_s);
    let h12 = match c.hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h12}:{:02} {}", c.minute, if c.hour < 12 { "AM" } else { "PM" })
}

/// `Wed Nov 4`: the day a stretch begins, at midnight in the ruler and in the strip.
pub fn day_text(t: i64, offset_s: i32) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let c = civil_of(t, offset_s);
    format!("{} {} {}", DAYS[usize::from(c.weekday) % 7], MONTHS[usize::from(c.month.clamp(1, 12)) - 1], c.day)
}

/// What a stretch of a row says in its cell: the programme's title, or that nothing is known of the hours.
fn title_of<'a>(s: &Span<'a>) -> Option<(&'a str, bool)> {
    match s.programme {
        Some(p) => Some((p.title.as_str(), false)),
        None if s.loading => None,
        None => Some(("No information", true)),
    }
}

/// The plate of the focus on a cell `from..to` of the grid's content at its full lift, when the grid is scrolled to `left`: the cell, kept
/// `plate_inset` from the divider at each end and from the row's edge, and never nearer the grid's edges than the clearance the view keeps the
/// focus off them by. It never stands on the next cell and never under the grid's edge, whatever the cell's width.
fn plate_frame(lay: &Layout, row: usize, from: f32, to: f32, left: f32) -> Frame {
    let clear = lay.clear();
    let (a, b) = ((from + lay.plate_inset).max(left + clear), (to - lay.plate_inset).min(left + lay.grid.w - clear));
    Frame::new(a, lay.row_h * row as f32 + lay.plate_inset, (b - a).max(lay.knob), lay.row_h - 2.0 * lay.plate_inset)
}

/// The letters a channel's tile says when it has no logo: a call sign (`WSB` of `WSB-HD`) as it is, the initials of a name of several words
/// (`PTCS` of `Pluto TV Classic Sitcoms`), four at most.
pub(super) fn monogram(name: &str) -> String {
    let words: Vec<&str> = name.split([' ', '-']).filter(|w| !w.is_empty()).collect();
    let Some(first) = words.first() else { return String::new() };
    let letters: String = if words.len() == 1 || !first.chars().any(char::is_lowercase) {
        first.chars().filter(|c| c.is_alphanumeric()).collect()
    } else {
        words.iter().filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).collect()
    };
    letters.to_uppercase().chars().take(4).collect()
}

/// Draws a row's cells and its lead: one run for every cell of the stretch round the view, and the title of the cell at the left edge. `flying`:
/// the time is still moving to the view. A title is said in the part of its cell the view shows (`cells`); the line of progress of the
/// programme on the air ends at now, and nothing of the grid runs through a word.
fn draw_cells(kit: &mut Kit, g: &Guide, lay: &Layout, row: &mut Row, flying: bool) {
    let (origin, view, now) = (g.origin(), g.view(), g.now());
    let span = lay.page().span;
    let (ink, ink2, accent, edge_colour) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), edge(kit));
    let pad = lay.pad;
    let ty = kit.ty(TypeRole::Label);
    let line = line_height(ty);
    let y_text = ((lay.row_h - line) / 2.0).floor();
    let y_row = lay.row_h * row.index as f32;
    // what the view shows of the content at rest
    let (left, shown) = (lay.x_of(view, origin), lay.grid.w);
    // where titles begin is the clearance in from the left edge: the first column's titles begin in one place
    let screen = cells::Screen { left, right: left + shown, lead: lay.clear() };
    let say = |kit: &mut Kit, text: &str, room: f32| cells::fit(text, room, &mut |t: &str| kit.ui.fonts.measure(t, ty).width);
    let (bar_h, rule_y) = (lay.row_h - 2.0 * lay.mark_inset, lay.row_h - lay.mark_inset - lay.rule);
    let compose = |kit: &mut Kit, from: i64, to: i64| -> (Paint, f32, Vec<Frame>) {
        let mut paint = Paint::default();
        let mut quiet = Paint::default();
        let mut rules = Vec::new();
        let mut lines = Vec::new();
        let x0 = lay.x_of(from, origin);
        for s in g.spans(&row.id, from, to) {
            let (cs, ce) = (lay.x_of(s.start.max(from), origin), lay.x_of(s.end.min(to), origin));
            if s.start >= from && s.start > origin {
                let x = (cs - x0 - lay.hair / 2.0).floor();
                paint.marks.push(Mark::bar(x as i32, lay.mark_inset as i32, lay.hair as u32, bar_h as u32, edge_colour));
                lines.push(Frame::new(x, lay.mark_inset, lay.hair, bar_h));
            }
            if let Some(slot) = cells::slot(cs, ce, screen, pad)
                && !slot.pinned
                && let Some((text, is_quiet)) = title_of(&s)
                && let Some(words) = say(kit, text, slot.room)
            {
                let (colour, target) = if is_quiet { (ink2, &mut quiet) } else { (ink, &mut paint) };
                target.text(words, ty, colour, slot.x - x0, y_text, slot.room);
            }
            if let Some(p) = s.programme.filter(|p| p.airing(now)) {
                let start = lay.x_of(p.start.max(from), origin) - x0;
                rules.push((start, (lay.x_of(now, origin) - x0 - start).max(1.0)));
            }
        }
        paint.runs.append(&mut quiet.runs);
        for (start, done) in rules {
            lines.push(Frame::new(start, rule_y, done, lay.rule));
            let (mut x, mut rest) = (start, done);
            while rest > 0.0 {
                let w = rest.min(1000.0);
                for k in 0..lay.rule as i32 {
                    paint.marks.push(Mark::rule(x as i32, rule_y as i32 + k, w as u32, accent));
                }
                x += w;
                rest -= w;
            }
        }
        (paint, x0, lines)
    };
    // too many glyphs for one run: a narrower stretch round the view
    let mut made = None;
    for (before, after) in [(span / 2, span + span / 2), (span / 4, span + span / 4), (0, span)] {
        let (from, to) = ((view - before).max(origin), (view + after).min(g.horizon()));
        let (paint, x0, lines) = compose(kit, from, to);
        let fits = paint.quads() <= QUADS;
        made = Some((paint, x0, lines));
        if fits {
            break;
        }
    }
    let (paint, x0, lines) = made.expect("a stretch");
    let mut said = Vec::new();
    let screen_x = |x: f32| lay.grid.x + x - left;
    for (text, t, colour, x, y, w) in &paint.runs {
        // a title's cell, as far as the view shows it: the room it has and the air on both sides
        let cell = Frame::new(screen_x(x0 + *x - pad), y_row, *w + 2.0 * pad, lay.row_h);
        said.push(Said::new(text.clone(), Frame::new(screen_x(x0 + *x), y_row + *y, *w, line), *t, *colour).within(cell));
    }
    row.marks = lines.iter().map(|f| Frame::new(screen_x(x0 + f.x), y_row + f.y, f.w, f.h)).collect();
    row.cells.draw(&mut kit.ui, &paint.items());
    row.cells.set_props(Props::at(x0, y_row));

    // the lead: the title of the cell that is crossing the place where titles begin, kept there
    let mut lead = Paint::default();
    let begin = view + lay.span / 100;
    let crossing = g.spans(&row.id, begin, begin + 1).into_iter().next().filter(|s| s.start < begin);
    if let Some(s) = &crossing
        && let Some(slot) = cells::slot(lay.x_of(s.start, origin), lay.x_of(s.end, origin), screen, pad)
        && slot.pinned
        && let Some((text, is_quiet)) = title_of(s)
        && let Some(words) = say(kit, text, slot.room)
    {
        let colour = if is_quiet { ink2 } else { ink };
        let cell = Frame::new(screen_x(slot.x - pad), y_row, slot.room + 2.0 * pad, lay.row_h);
        said.push(Said::new(words.clone(), Frame::new(screen_x(slot.x), y_row + y_text, slot.room, line), ty, colour).within(cell));
        lead.text(words, ty, colour, slot.x - left, y_text, slot.room);
    }
    row.lead.draw(&mut kit.ui, &lead.items());
    // The lead is pinned to the edge while the cells under it fly with the time. One that goes on speaking for the same cell stays; one
    // that has a new cell to speak for (or a new row) in a flight waits, hidden, until the time stops, and then comes in.
    let cell = crossing.map(|s| s.start);
    let changed = row.lead_of.replace(cell) != Some(cell);
    if flying && changed && cell.is_some() {
        row.lead_wait = true;
        row.lead.set_props(Props { opacity: 0.0, ..Props::at(lay.grid.x, y_row) });
    }
    row.words = said;
}

/// What the strip says: a signature of its inputs, so that it is drawn again only when one changes.
fn strip_sig(g: &Guide) -> String {
    let id = g.channel(g.row()).map_or("", |c| c.id.as_str());
    let s = g.focus_span();
    format!("{id}|{}|{}|{}|{}|{}|{}|{}|{}", s.as_ref().map_or(0, |s| s.start), s.as_ref().map_or(0, |s| s.end), s.as_ref().is_some_and(|s| s.loading), g.rev(id), g.now() / MIN, g.rows(), g.tuning(), g.tune_fault())
}

struct StripWords {
    title: String,
    quiet: bool,
    meta: String,
    desc: String,
}

/// `2-1 WSB-HD`
fn channel_line(g: &Guide) -> String {
    g.channel(g.row()).map(|c| format!("{} {}", c.number, c.name)).unwrap_or_default()
}

fn flags(p: &crate::host::channels::Programme) -> Vec<&'static str> {
    let mut out = Vec::new();
    if p.captions {
        out.push("CC");
    }
    if p.described {
        out.push("AD");
    }
    out
}

fn strip_words(g: &Guide) -> StripWords {
    let off = g.offset_s();
    if g.tune_fault() {
        return StripWords { title: "Couldn't switch the channel".into(), quiet: false, meta: channel_line(g), desc: String::new() };
    }
    if g.tuning() {
        return StripWords { title: "Switching channel".into(), quiet: true, meta: channel_line(g), desc: String::new() };
    }
    let Some(s) = g.focus_span() else {
        return StripWords { title: String::new(), quiet: true, meta: String::new(), desc: String::new() };
    };
    match s.programme {
        Some(p) => {
            let day = if civil_of(p.start, off).day != civil_of(g.now(), off).day { format!("{}  ", day_text(p.start, off)) } else { String::new() };
            let mut parts = vec![format!("{day}{} - {}", clock_text(p.start, off), clock_text(p.end, off)), channel_line(g)];
            if !p.rating.is_empty() {
                parts.push(p.rating.clone());
            }
            parts.extend(flags(p).into_iter().map(String::from));
            StripWords { title: p.title.clone(), quiet: false, meta: parts.join("  ·  "), desc: p.text.clone() }
        }
        None => StripWords { title: if s.loading { "Loading".into() } else { "No information".into() }, quiet: true, meta: channel_line(g), desc: String::new() },
    }
}

/// How the programme in the sheet stands to now: `On now`, `Next`, `Later` or `Earlier`.
fn sheet_meta(g: &Guide, s: &super::model::Sheet<'_>) -> (String, String) {
    let off = g.offset_s();
    match s.main {
        Some(p) => {
            let mut parts = vec![format!("{} - {}", clock_text(p.start, off), clock_text(p.end, off))];
            if !p.rating.is_empty() {
                parts.push(p.rating.clone());
            }
            parts.extend(flags(p).into_iter().map(String::from));
            (s.when.to_string(), parts.join("  ·  "))
        }
        None => (String::new(), String::new()),
    }
}

fn sheet_channel(s: &super::model::Sheet<'_>) -> String {
    format!("{} {}", s.channel.number, s.channel.name)
}

fn sheet_sig(g: &Guide, s: &super::model::Sheet<'_>) -> String {
    format!("{}|{}|{}|{}", s.channel.id, s.main.map_or("", |p| p.id.as_str()), s.when, g.rev(&s.channel.id))
}
