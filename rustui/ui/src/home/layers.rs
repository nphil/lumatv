//! The layers that open over Home, drawn: the library (`docs/mocks/12-all-apps`), the search sheet with its keyboard
//! (`09-search-keyboard`), the layout editor (`38-home-edit*`: the free cells, the selection ring, the bar, the add list, the question),
//! "Who's watching?" with the PIN pad, and a tile's options (`16-context-menu`). Each is made when its layer opens and dropped when it
//! closes; what moves between focusables flows on the shared engine (the cursor of a keyboard glides on the focus spring too).

use super::apps::{self, Apps};
use super::editor::{self, Editor, Mode, Zone};
use super::grid;
use super::model::{Home, Layer};
use super::picker;
use super::search::{self, Search};
use super::surface::{HIDDEN, HomeSurface, Lift, faded, glass_of};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::flow::Lifted;
use crate::kit::focus::Region;
use crate::kit::pane::PaneKind;
use crate::kit::picture::{self, Fit};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Engine, Id, Kit};
use compositor_protocol::{Backdrop, Fill, GroupPose, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::{Align, Block, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

const CURSOR_X: Region = Region(20);
const CURSOR_Y: Region = Region(21);
const PAD_X: Region = Region(22);
const PAD_Y: Region = Region(23);

fn other(e: ui::error::UiError) -> std::io::Error {
    std::io::Error::other(format!("{e}"))
}

/// The page-hiding scrim: the mode's `scrimHide`.
fn scrim(kit: &Kit, z: i32) -> ShapeLayer {
    let c = kit.tokens.extra(kit.mode).scrim_hide;
    kit.ui.scene.shape(1920, 1080, z, plate::solid(c, 0.0), Props::at(0.0, 0.0))
}

/// A glass cursor that glides to the focused key of a pad on the focus spring: a group for x over a group for y.
struct Cursor {
    outer: Group,
    inner: Group,
    glass: GlassLayer,
    rx: Region,
    ry: Region,
    w: f32,
    h: f32,
    at: Option<(f32, f32)>,
}

impl Cursor {
    #[allow(clippy::too_many_arguments)]
    fn new(kit: &mut Kit, engine: &mut Engine, rx: Region, ry: Region, w: f32, h: f32, radius: f32, z: i32) -> std::io::Result<Self> {
        let outer = kit.ui.scene.group(None, GroupPose { x: 1920.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let inner = kit.ui.scene.group(Some(outer.id()), GroupPose { y: 1080.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let scene = kit.ui.scene.clone();
        let mut g = glass_of(kit, PaneKind::Control, h, true, false);
        g.corner_radius = radius;
        let glass = scene.adopting(&inner, || kit.ui.scene.glass(w as u32, h as u32, z, g, HIDDEN));
        engine.flow.scroller(&kit.ui.scene, rx, &outer, GroupPose { x: 1920.0, ..GroupPose::IDENTITY }, true, 1920.0).map_err(other)?;
        engine.flow.scroller(&kit.ui.scene, ry, &inner, GroupPose { y: 1080.0, ..GroupPose::IDENTITY }, false, 1080.0).map_err(other)?;
        Ok(Self { outer, inner, glass, rx, ry, w, h, at: None })
    }

    /// Glides to `frame`'s place (the cursor's size is the pad's key; a wider key stretches it by remaking the glass).
    fn go(&mut self, kit: &Kit, engine: &mut Engine, frame: Frame) {
        if (frame.w - self.w).abs() > 0.5 || (frame.h - self.h).abs() > 0.5 {
            let mut g = glass_of(kit, PaneKind::Control, frame.h, true, false);
            g.corner_radius = kit.tokens.radius(Radius::Control);
            self.glass.restyle(g);
            let _ = (&self.outer, &self.inner);
        }
        let first = self.at.is_none();
        self.at = Some((frame.x, frame.y));
        let _ = engine.flow.scroll_to(self.rx, 1920.0 - frame.x);
        let _ = engine.flow.scroll_to(self.ry, 1080.0 - frame.y);
        self.glass.set(Props::at(0.0, 0.0));
        if first {
            // the first place is taken at once: the springs start from where the pad is, not from the corner
            let _ = engine.flow.scroll_to(self.rx, 1920.0 - frame.x);
        }
    }
}

// ---- the library
struct Cell {
    id: Id,
    plate: ShapeLayer,
    icon: Option<ImageLayer>,
    name: Text,
    app: String,
}

pub struct AppsSurface {
    scrim: ShapeLayer,
    bar: GlassLayer,
    bar_text: Block,
    rail: GlassLayer,
    rail_rows: Block,
    rail_mark: ShapeLayer,
    rail_focus: ShapeLayer,
    page: GlassLayer,
    heading: Block,
    search: Button,
    grid: Group,
    cells: Vec<Cell>,
    empty: Text,
    key: String,
    scroll: f32,
    z: i32,
}

impl AppsSurface {
    pub fn new(kit: &mut Kit, engine: &mut Engine, home: &Home, z: i32) -> std::io::Result<Self> {
        let scrim = scrim(kit, z);
        let bar_f = Frame::new(64.0, 56.0, 1792.0, 56.0);
        let bar = kit.ui.scene.glass(1792, 56, z + 1, glass_of(kit, PaneKind::Pill, 56.0, false, false), Props::at(bar_f.x, bar_f.y));
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let t = super::clock::civil(home.now);
        let (time, ampm) = t.time_words(home.theme.pointer("/clock/twelveHour").and_then(Value::as_bool) != Some(false));
        let bar_lines = vec![
            (Frame::new(kit.space(5) + 36.0, 0.0, 400.0, 56.0), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), "Apps".to_owned()),
            (Frame::new(1792.0 - 300.0 - kit.space(6), 0.0, 300.0, 56.0), role_style(kit, TypeRole::Label, None, ink2, Align::Right), format!("{time} {ampm}").trim().to_owned()),
        ];
        let bar_text = Block::new(&mut kit.ui, bar_f, z + 2, bar_lines)?;
        let rail_f = apps::RAIL_FRAME;
        let rail = kit.ui.scene.glass(rail_f.w as u32, rail_f.h as u32, z + 1, glass_of(kit, PaneKind::Card, rail_f.h, false, false), Props::at(rail_f.x, rail_f.y));
        // the rows hold a title and a detail line at the text size; a row that cannot hold both at 200 % keeps the title alone
        let mut rows = Vec::new();
        let (lh, ch) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
        let row_h = home.apps.rail_frame(0, false).h;
        let two = lh + ch + 8.0 <= row_h;
        for i in 0..12 {
            let f = home.apps.rail_frame(i, false);
            let top = if two { ((row_h - lh - ch) / 2.0).round() } else { ((row_h - lh) / 2.0).round() };
            rows.push((Frame::new(f.x - rail_f.x + 24.0, f.y - rail_f.y + top, f.w - 48.0, lh), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left), String::new()));
            rows.push((Frame::new(f.x - rail_f.x + 24.0, f.y - rail_f.y + top + lh, f.w - 48.0, if two { ch } else { 0.0 }), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()));
        }
        let rail_rows = Block::new(&mut kit.ui, rail_f, z + 3, rows)?;
        let rail_mark = kit.ui.scene.shape(5, (row_h - 44.0).max(12.0) as u32, z + 3, plate::solid(kit.ui.look.accent(), 3.0), HIDDEN);
        let rail_focus = kit.ui.scene.shape((rail_f.w - 32.0) as u32, row_h as u32, z + 2, plate::solid(faded(ink, kit.tokens.states.focus_wash[0]), kit.tokens.radius(Radius::Row)), HIDDEN);
        let page_f = apps::PAGE_FRAME;
        let page = kit.ui.scene.glass(page_f.w as u32, page_f.h as u32, z + 1, glass_of(kit, PaneKind::Card, page_f.h, false, false), Props::at(page_f.x, page_f.y));
        let head_h = 56.0 * kit.text_scale.max(1.0);
        let title_w = (420.0 * kit.text_scale.max(1.0)).round();
        let sf = home.apps.search_frame();
        let sub_w = (sf.x - 24.0 - (page_f.x + 40.0 + title_w + 10.0)).max(0.0);
        let heading_lines = vec![
            (Frame::new(0.0, 0.0, title_w, head_h), role_style(kit, TypeRole::Title, None, ink, Align::Left), String::new()),
            (Frame::new(title_w + 10.0, 0.0, sub_w, head_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
        ];
        let heading = Block::new(&mut kit.ui, Frame::new(page_f.x + 40.0, page_f.y + 28.0, title_w + 10.0 + sub_w, head_h), z + 2, heading_lines)?;
        let mut search = Button::new(kit, engine, Place { id: Id(apps::SEARCH_ID), frame: sf, region: apps::TOP, scope: super::model::SCOPE_APPS, z: z + 3 }, Spec::new("Search apps", Tone::Plain).with_icon("search"))?;
        search.show(kit, true);
        let grid_y = home.apps.grid_y();
        let grid = kit.ui.scene.group(None, GroupPose { clip: Some(compositor_protocol::Rect { x: apps::GRID_X, y: grid_y, w: page_f.w - 80.0, h: page_f.h - (grid_y - page_f.y) - 70.0 }), ..GroupPose::IDENTITY }).map_err(other)?;
        engine.flow.scroller(&kit.ui.scene, apps::CELLS, &grid, GroupPose::IDENTITY, false, 6000.0).map_err(other)?;
        let mut empty = Text::line_at(kit, page_f.x, page_f.y + page_f.h / 2.0, page_f.w, z + 3, role_style(kit, TypeRole::Label, None, ink2, Align::Centre), "Nothing here")?;
        empty.set_props(HIDDEN);
        Ok(Self { scrim, bar, bar_text, rail, rail_rows, rail_mark, rail_focus, page, heading, search, grid, cells: Vec::new(), empty, key: String::new(), scroll: 0.0, z })
    }

    fn build_cells(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home) -> std::io::Result<()> {
        let key = format!("{}|{}", home.apps.section, home.apps.list().iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(","));
        if key == self.key {
            return Ok(());
        }
        self.key = key;
        for c in self.cells.drain(..) {
            engine.flow.drop_lift(c.id);
        }
        let ink = kit.ui.look.ink();
        let wash = faded(ink, kit.tokens.states.focus_wash[0]);
        let edge = kit.ui.look.colour(PaletteRole::EdgeRaised);
        let room = kit.room();
        let most = room.layers.saturating_sub(24).min(room.groups.saturating_sub(4) * 3) / 3;
        let list: Vec<_> = home.apps.list().into_iter().cloned().collect();
        for (i, a) in list.iter().enumerate().take(most.min(apps::MAX_CELLS as usize)) {
            let id = Id(apps::CELL_ID + i as u32);
            let f = Frame::new(apps::GRID_X + (i % apps::COLS) as f32 * apps::COL_PITCH, home.apps.grid_y() + (i / apps::COLS) as f32 * apps::ROW_PITCH, apps::CELL_W, apps::CELL_H);
            let rest = GroupPose { x: f.x, y: f.y, pivot_x: f.w / 2.0, pivot_y: f.h / 2.0, ..GroupPose::IDENTITY };
            let group = kit.ui.scene.group(Some(self.grid.id()), rest).map_err(other)?;
            let scene = kit.ui.scene.clone();
            let z = self.z;
            let (plate_l, icon, name) = scene.adopting(&group, || -> std::io::Result<_> {
                let shape = Shape { corner_radius: kit.tokens.radius(Radius::Card), fill: Fill::Solid(wash), border_width: 1.0, border: edge };
                let plate_l = kit.ui.scene.shape(f.w as u32, f.h as u32, z + 2, shape, HIDDEN);
                let icon = HomeSurface::tile_icon_pub(kit, &a.icon, 152, 20, kit.tokens.radius(Radius::Tile), z + 3)?;
                let mut icon = icon;
                if let Some(ic) = &mut icon {
                    let side = ic.width() as f32;
                    ic.set(Props { opacity: if a.hidden { 0.5 } else { 1.0 }, ..Props::at(((f.w - side) / 2.0).round(), 14.0 + (152.0 - side) / 2.0) });
                }
                let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
                let title = ui::components::text::ellipsise(&mut kit.ui.fonts, &a.title, style.ty, f.w - 24.0);
                let line = kit.line(TypeRole::Label);
                let mut name = Text::new(kit, Frame::new(12.0, 14.0 + 152.0 + 12.0, f.w - 24.0, line), z + 3, style, &title)?;
                name.set_props(Props::at(12.0, 14.0 + 152.0 + 12.0));
                Ok((plate_l, icon, name))
            })?;
            engine.flow.lift(id, Lifted { group, rest, scale: kit.tokens.focus.card_scale, rise: 0.0, lit: None });
            self.cells.push(Cell { id, plate: plate_l, icon, name, app: a.id.clone() });
        }
        self.empty.set_props(if list.is_empty() { Props::at(apps::PAGE_FRAME.x, apps::PAGE_FRAME.y + apps::PAGE_FRAME.h / 2.0 - 20.0) } else { HIDDEN });
        Ok(())
    }

    pub fn update(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, _now_ns: u64) -> std::io::Result<()> {
        self.build_cells(kit, engine, home)?;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let rail = home.apps.rail();
        let mut changes: Vec<(usize, String, Rgba)> = Vec::new();
        for i in 0..12 {
            match rail.get(i) {
                Some(s) => {
                    changes.push((2 * i, s.title.clone(), ink));
                    changes.push((2 * i + 1, s.sub.clone(), ink2));
                }
                None => {
                    changes.push((2 * i, String::new(), ink));
                    changes.push((2 * i + 1, String::new(), ink2));
                }
            }
        }
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
        self.rail_rows.set_many(&mut kit.ui, &refs);
        let h = home.apps.heading();
        let sub = Apps::sub_of(&h.id).to_owned();
        self.heading.set_many(&mut kit.ui, &[(0, &h.title, ink), (1, &sub, ink2)]);
        let rf = home.apps.rail_frame(home.apps.rail_index, h.id == "hidden");
        self.rail_mark.set(Props::at(rf.x - 6.0, rf.y + 22.0 * kit.text_scale.max(1.0)));
        let on_rail = !home.apps.in_grid && !home.apps.search_focused && home.layer == Layer::Apps;
        self.rail_focus.set(if on_rail { Props::at(rf.x, rf.y) } else { HIDDEN });
        let scroll = home.apps.scroll_row as f32 * apps::ROW_PITCH;
        if scroll != self.scroll {
            self.scroll = scroll;
            let _ = engine.flow.scroll_to(apps::CELLS, scroll);
        }
        let focused = home.focus.current();
        for c in &self.cells {
            let on = Some(c.id) == focused && home.apps.in_grid && home.layer == Layer::Apps;
            c.plate.set(if on { Props::at(0.0, 0.0) } else { HIDDEN });
            let _ = (&c.name, &c.icon, &c.app);
        }
        self.search.set_hover(kit, home.focus.hovered() == Some(Id(apps::SEARCH_ID)));
        let _ = (&self.scrim, &self.bar, &self.bar_text, &self.rail, &self.page);
        Ok(())
    }

    pub fn destroy(self, engine: &mut Engine) {
        for c in self.cells {
            engine.flow.drop_lift(c.id);
        }
        self.search.destroy(engine);
    }
}

impl HomeSurface {
    /// A launch point's icon for a cell of `side`: the tile rule (edge to edge when the canvas is painted, inset otherwise).
    pub fn tile_icon_pub(kit: &mut Kit, path: &str, side: u32, inset: u32, radius: f32, z: i32) -> std::io::Result<Option<ImageLayer>> {
        if path.is_empty() {
            return Ok(None);
        }
        let Ok(bytes) = std::fs::read(path) else { return Ok(None) };
        let Ok(pic) = picture::decode(&bytes) else { return Ok(None) };
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in 0..pic.height {
            for x in 0..pic.width {
                if x == 0 || y == 0 || x + 1 == pic.width || y + 1 == pic.height {
                    sum += pic.rgba[((y * pic.width + x) * 4 + 3) as usize] as u64;
                    n += 1;
                }
            }
        }
        let full = n > 0 && sum as f32 / n as f32 / 255.0 >= 0.9;
        let s = if full { side } else { side - 2 * inset };
        let mut fitted = pic.fitted(s, s, Fit::Contain);
        if full {
            fitted.round_corners(radius);
        }
        let mut layer = kit.ui.scene.image(s, s, z, Props::at(0.0, 0.0))?;
        fitted.paint(&mut layer);
        layer.damage(None);
        Ok(Some(layer))
    }
}

// ---- search
pub struct SearchSurface {
    scrim: ShapeLayer,
    sheet: GlassLayer,
    field: ShapeLayer,
    field_icon: Option<ImageLayer>,
    typed: Text,
    caret: ShapeLayer,
    hint: Text,
    answers: Vec<(ShapeLayer, Block)>,
    entry_note: Block,
    keys_art: ImageLayer,
    caps: Block,
    cursor: Cursor,
    caps_key: String,
    shown_text: String,
}

impl SearchSurface {
    pub fn new(kit: &mut Kit, engine: &mut Engine, home: &Home, z: i32) -> std::io::Result<Self> {
        let scrim = scrim(kit, z);
        let sf = search::SHEET;
        let sheet = kit.ui.scene.glass(sf.w as u32, sf.h as u32, z + 1, glass_of(kit, PaneKind::Sheet, sf.h, false, false), Props::at(sf.x, sf.y));
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let ff = Search::field_frame();
        let field = kit.ui.scene.shape(ff.w as u32, ff.h as u32, z + 2, Shape { corner_radius: kit.tokens.radius(Radius::Tile), fill: Fill::Solid(faded(ink, 0.06)), border_width: 1.0, border: kit.ui.look.colour(PaletteRole::Edge) }, Props::at(ff.x, ff.y));
        let field_icon = crate::kit::icon::layer(kit, if home.search.entry { "keyboard" } else { "search" }, 32, ink2, z + 3, Props::at(ff.x + 26.0, ff.y + 26.0))?;
        let mut ty = kit.ty(TypeRole::Title);
        ty.px = (32.0 * kit.text_scale).round() as u32;
        let typed_style = ui::components::style(ty, ink, Align::Left);
        let typed = Text::line_at(kit, ff.x + 76.0, ff.y + ff.h / 2.0, ff.w - 76.0 - 420.0, z + 3, typed_style, "")?;
        let caret = kit.ui.scene.shape(3, 38, z + 3, plate::solid(kit.ui.look.accent(), 2.0), Props::at(ff.x + 80.0, ff.y + 23.0));
        let hint_words = if home.search.entry { "Back cancels" } else if home.search.voice { "Hold the mic key to speak" } else { "" };
        let hint = Text::line_at(kit, ff.x + ff.w - 26.0 - 400.0, ff.y + ff.h / 2.0, 400.0, z + 3, role_style(kit, TypeRole::Caption, None, ink2, Align::Right), hint_words)?;
        let mut answers = Vec::new();
        for i in 0..4 {
            let af = Search::answer_frame(i);
            let plate_l = kit.ui.scene.shape(af.w as u32, af.h as u32, z + 2, Shape { corner_radius: kit.tokens.radius(Radius::Row), fill: Fill::Solid(faded(ink, 0.05)), border_width: 1.0, border: Rgba(0, 0, 0, 0) }, HIDDEN);
            let lines = vec![
                (Frame::new(100.0, 22.0, af.w - 122.0, kit.line(TypeRole::Label)), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(100.0, 22.0 + kit.line(TypeRole::Label), af.w - 122.0, kit.line(TypeRole::Caption)), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
            ];
            let block = Block::new(&mut kit.ui, af, z + 3, lines)?;
            answers.push((plate_l, block));
        }
        let nf = Frame::new(sf.x + 40.0, sf.y + 134.0, sf.w - 80.0, 108.0);
        let note_lines = vec![
            (Frame::new(100.0, 22.0, nf.w - 122.0, kit.line(TypeRole::Label)), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left), home.search.prompt.clone()),
            (Frame::new(100.0, 22.0 + kit.line(TypeRole::Label), nf.w - 122.0, kit.line(TypeRole::Caption)), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), "Done saves the name here and on the dock. An empty name brings back the one LG gave.".to_owned()),
        ];
        let entry_note = Block::new(&mut kit.ui, nf, z + 3, note_lines)?;
        let kf = Frame::new(search::KEYS_X, search::KEYS_Y, 930.0, 400.0);
        let mut keys_art = kit.ui.scene.image(kf.w as u32, kf.h as u32, z + 2, Props::at(kf.x, kf.y))?;
        {
            // the shift and backspace keys carry icons, not characters the face has
            let shift = kit.icons.draw("arrow-big-up", 28, ink).map(|rgba| picture::Picture { width: 28, height: 28, rgba });
            let erase = kit.icons.draw("delete", 28, ink).map(|rgba| picture::Picture { width: 28, height: 28, rgba });
            let (w, h) = (keys_art.width(), keys_art.height());
            let mut canvas = super::widgets::Canvas { px: keys_art.pixels_mut(), w, h };
            let glass = kit.ui.look.colour(PaletteRole::Glass);
            let r = kit.tokens.radius(Radius::Control);
            for i in 0..search::N_KEYS {
                let f = Search::key_frame(i);
                canvas.rounded(Frame::new(f.x - kf.x, f.y - kf.y, f.w, f.h), r, glass);
                let icon = match home.search.key_at(i) {
                    "\u{21e7}" => shift.as_ref(),
                    "\u{232b}" => erase.as_ref(),
                    _ => None,
                };
                if let Some(p) = icon {
                    canvas.picture(p, (f.x - kf.x + (f.w - 28.0) / 2.0).round() as i32, (f.y - kf.y + (f.h - 28.0) / 2.0).round() as i32, 28, 28);
                }
            }
        }
        keys_art.damage(None);
        let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
        let cap_of = |k: &str| if k == "\u{21e7}" || k == "\u{232b}" { String::new() } else { k.to_owned() };
        let caps = Block::new(&mut kit.ui, kf, z + 5, (0..search::N_KEYS).map(|i| { let f = Search::key_frame(i); (Frame::new(f.x - kf.x, f.y - kf.y, f.w, f.h), style, cap_of(home.search.key_at(i))) }).collect())?;
        let cursor = Cursor::new(kit, engine, CURSOR_X, CURSOR_Y, 84.0, 72.0, kit.tokens.radius(Radius::Control), z + 4)?;
        let _ = (&entry_note, &scrim, &sheet, &field);
        Ok(Self { scrim, sheet, field, field_icon, typed, caret, hint, answers, entry_note, keys_art, caps, cursor, caps_key: String::new(), shown_text: "\u{0}".into() })
    }

    pub fn update(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now: u64) -> std::io::Result<()> {
        let s = &home.search;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let shown = s.shown();
        if shown != self.shown_text {
            self.shown_text = shown.clone();
            self.typed.set(kit, &shown);
            let style = self.typed_style(kit);
            let w = width_of(kit, style, &shown);
            let ff = Search::field_frame();
            self.caret.set(Props::at(ff.x + 76.0 + w + 2.0, ff.y + 23.0));
        }
        self.caret.set(Props { opacity: if s.caret_on(now) { 1.0 } else { 0.0 }, ..Props::at(self.caret_x(kit, &shown), Search::field_frame().y + 23.0) });
        let caps_key = format!("{}{}", s.symbols, s.shift);
        if caps_key != self.caps_key {
            self.caps_key = caps_key;
            let caps: Vec<String> = (0..search::N_KEYS)
                .map(|i| {
                    let k = s.key_at(i);
                    if k == "\u{21e7}" || k == "\u{232b}" {
                        String::new()
                    } else if s.shift && k.chars().count() == 1 {
                        k.to_uppercase()
                    } else {
                        k.to_owned()
                    }
                })
                .collect();
            let refs: Vec<(usize, &str, Rgba)> = caps.iter().enumerate().map(|(i, c)| (i, c.as_str(), ink)).collect();
            self.caps.set_many(&mut kit.ui, &refs);
        }
        let answers = s.answers();
        for (i, (plate_l, block)) in self.answers.iter_mut().enumerate() {
            match answers.get(i) {
                Some(a) if !s.entry => {
                    let f = Search::answer_frame(i);
                    let focused = home.focus.current() == Some(Id(search::ANSWER_ID + i as u32));
                    plate_l.set(Props::at(f.x, f.y));
                    plate_l.reshape(Shape { corner_radius: kit.tokens.radius(Radius::Row), fill: Fill::Solid(faded(ink, if focused { 0.12 } else { 0.05 })), border_width: 1.0, border: if focused { kit.ui.look.colour(PaletteRole::EdgeRaised) } else { Rgba(0, 0, 0, 0) } });
                    block.set_many(&mut kit.ui, &[(0, &a.title, ink), (1, &a.sub, ink2)]);
                    block.set_props(Props::at(f.x, f.y));
                }
                _ => {
                    plate_l.set(HIDDEN);
                    block.set_props(HIDDEN);
                }
            }
        }
        let nf = Frame::new(search::SHEET.x + 40.0, search::SHEET.y + 134.0, 10.0, 10.0);
        self.entry_note.set_props(if s.entry { Props::at(nf.x, nf.y) } else { HIDDEN });
        if let Some(id) = home.focus.current().filter(|_| home.layer == Layer::Search) {
            if let Some(i) = id.0.checked_sub(search::KEY_ID).filter(|i| (*i as usize) < search::N_KEYS) {
                self.cursor.go(kit, engine, Search::key_frame(i as usize));
            }
        }
        let _ = (&self.field_icon, &self.hint, &self.keys_art, &self.scrim, &self.sheet, &self.field);
        Ok(())
    }

    fn typed_style(&self, kit: &Kit) -> ui::components::Style {
        let mut ty = kit.ty(TypeRole::Title);
        ty.px = (32.0 * kit.text_scale).round() as u32;
        ui::components::style(ty, kit.ui.look.ink(), Align::Left)
    }

    fn caret_x(&self, kit: &mut Kit, shown: &str) -> f32 {
        let style = self.typed_style(kit);
        Search::field_frame().x + 76.0 + width_of(kit, style, shown) + 2.0
    }

    pub fn destroy(self, _engine: &mut Engine) {}
}

// ---- the layout editor
pub struct EditorSurface {
    cells: ImageLayer,
    ring: ShapeLayer,
    corner: ShapeLayer,
    corner_glyph: Option<ImageLayer>,
    ghost: ShapeLayer,
    bar: GlassLayer,
    bar_text: Block,
    buttons: Vec<Button>,
    add_pane: GlassLayer,
    add_rows: Block,
    add_focus: ShapeLayer,
    toast: GlassLayer,
    toast_text: Text,
    ask_pane: GlassLayer,
    ask_title: Text,
    ask_buttons: Vec<Button>,
    cells_key: String,
    toast_key: String,
    enabled_key: String,
    z: i32,
}

impl EditorSurface {
    pub fn new(kit: &mut Kit, engine: &mut Engine, home: &Home, z: i32) -> std::io::Result<Self> {
        let ed = home.editor.as_ref().ok_or_else(|| std::io::Error::other("no editor"))?;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let a = grid::area(&ed.work);
        let cells = kit.ui.scene.image(a.w as u32, a.h as u32, z, Props::at(a.x as f32, a.y as f32))?;
        let ring = kit.ui.scene.shape(8, 8, z + 1, plate::ring(kit.ui.look.accent(), kit.tokens.radius(Radius::Card) + 6.0, 3.0), HIDDEN);
        let corner = kit.ui.scene.shape(36, 36, z + 2, plate::solid(kit.ui.look.accent(), 18.0), HIDDEN);
        let corner_glyph = crate::kit::icon::layer(kit, "chevron-right", 22, kit.ui.look.accent_ink(), z + 3, HIDDEN)?;
        let ghost = kit.ui.scene.shape(8, 8, z + 1, plate::ring(kit.ui.look.accent(), kit.tokens.radius(Radius::Card), 3.0), HIDDEN);
        let bf = Frame::new(64.0, editor::BAR_Y, 1792.0, editor::BAR_H);
        let mut g = glass_of(kit, PaneKind::Sheet, bf.h, false, false);
        g.corner_radius = kit.tokens.radius(Radius::Sheet) / 2.0;
        let bar = kit.ui.scene.glass(bf.w as u32, bf.h as u32, z + 4, g, Props::at(bf.x, bf.y));
        let bar_lines = vec![
            (Frame::new(kit.space(6), 0.0, 200.0, bf.h), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left), "Edit Home".to_owned()),
            (Frame::new(kit.space(6) + 200.0, 0.0, 700.0, bf.h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), ed.hint_words()),
        ];
        let bar_text = Block::new(&mut kit.ui, bf, z + 5, bar_lines)?;
        let mut buttons = Vec::new();
        for (i, b) in ed.buttons().iter().enumerate() {
            let tone = if b.primary { Tone::Primary } else { Tone::Plain };
            let mut btn = Button::new(kit, engine, Place { id: Id(editor::BAR_ID + i as u32), frame: b.frame, region: editor::BAR, scope: super::model::SCOPE_EDIT, z: z + 6 }, Spec::new(b.label, tone).with_icon(b.icon))?;
            btn.set_enabled(kit, engine, b.enabled);
            btn.show(kit, true);
            buttons.push(btn);
        }
        let af = ed.add_frame();
        let mut ag = glass_of(kit, PaneKind::Sheet, af.h, false, false);
        ag.corner_radius = kit.tokens.radius(Radius::Sheet) / 2.0;
        let add_pane = kit.ui.scene.glass(af.w as u32, af.h as u32, z + 7, ag, HIDDEN);
        let mut rows = Vec::new();
        for (i, r) in ed.add_rows().iter().enumerate() {
            let f = Frame::new(r.4.x - af.x, r.4.y - af.y, r.4.w, r.4.h);
            rows.push((Frame::new(f.x + 64.0, f.y, f.w - 180.0, f.h), role_style(kit, TypeRole::Label, None, ink, Align::Left), r.1.to_owned()));
            rows.push((Frame::new(f.x + f.w - 120.0, f.y, 100.0, f.h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), if r.3 { "Hidden".into() } else { String::new() }));
            let _ = i;
        }
        let add_rows = Block::new(&mut kit.ui, af, z + 9, rows)?;
        let add_focus = kit.ui.scene.shape((af.w - 16.0) as u32, 80, z + 8, plate::solid(faded(ink, kit.tokens.states.focus_wash[0]), kit.tokens.radius(Radius::Row)), HIDDEN);
        let toast = kit.ui.scene.glass(10, 56, z + 10, glass_of(kit, PaneKind::Pill, 56.0, false, false), HIDDEN);
        let warning = kit.ui.look.colour(PaletteRole::Warning);
        let mut toast_text = Text::new(kit, Frame::new(0.0, 0.0, 10.0, 56.0), z + 11, role_style(kit, TypeRole::Label, Some(Weight::SemiBold), warning, Align::Centre), "")?;
        toast_text.set_props(HIDDEN);
        let (qf, answers) = ed.ask_frames();
        let ask_pane = kit.ui.scene.glass(qf.w as u32, qf.h as u32, z + 12, glass_of(kit, PaneKind::Sheet, qf.h, false, false), HIDDEN);
        let mut ask_title = Text::new(kit, Frame::new(qf.x + 32.0, qf.y + 32.0, qf.w - 64.0, kit.line(TypeRole::Title)), z + 13, role_style(kit, TypeRole::Title, None, ink, Align::Left), "Save changes?")?;
        ask_title.set_props(HIDDEN);
        let mut ask_buttons = Vec::new();
        for (i, (_, label, danger)) in editor::ASK_ANSWERS.iter().enumerate() {
            let tone = if *danger { Tone::Danger } else if i == 1 { Tone::Primary } else { Tone::Plain };
            let btn = Button::new(kit, engine, Place { id: Id(editor::ASK_ID + i as u32), frame: answers[i], region: editor::ASK, scope: super::model::SCOPE_EDIT, z: z + 14 }, Spec::new(label, tone))?;
            ask_buttons.push(btn);
        }
        Ok(Self { cells, ring, corner, corner_glyph, ghost, bar, bar_text, buttons, add_pane, add_rows, add_focus, toast, toast_text, ask_pane, ask_title, ask_buttons, cells_key: String::new(), toast_key: String::new(), enabled_key: String::new(), z })
    }

    /// The cells where no card is: rounded squares, drawn once per layout.
    fn paint_cells(&mut self, kit: &Kit, ed: &Editor) {
        let key = ed.work.to_string();
        if key == self.cells_key {
            return;
        }
        self.cells_key = key;
        let a = grid::area(&ed.work);
        let (cw, ch) = grid::cell_size(&ed.work);
        let gutter = ed.work.pointer("/grid/gutter").and_then(Value::as_f64).unwrap_or(20.0) as f32;
        let cols = ed.work.pointer("/grid/cols").and_then(Value::as_u64).unwrap_or(12) as usize;
        let rows = grid::row_count(&ed.work) as usize;
        let ink = kit.ui.look.ink();
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        let fill = faded(ink, if light { 0.06 } else { 0.05 });
        let line = faded(ink, if light { 0.26 } else { 0.20 });
        let taken: Vec<grid::Rect> = grid::entries(&ed.work).iter().filter(|e| !e.hidden).map(|e| grid::rect_of(&ed.work, e)).collect();
        self.cells.clear();
        {
            let (w, h) = (self.cells.width(), self.cells.height());
            let mut canvas = super::widgets::Canvas { px: self.cells.pixels_mut(), w, h };
            for r in 0..rows {
                for c in 0..cols {
                    let x = c as f32 * (cw as f32 + gutter);
                    let y = r as f32 * (ch as f32 + gutter);
                    let (cx, cy) = (a.x as f32 + x + cw as f32 / 2.0, a.y as f32 + y + ch as f32 / 2.0);
                    if taken.iter().any(|t| cx >= t.x as f32 && cx < (t.x + t.w) as f32 && cy >= t.y as f32 && cy < (t.y + t.h) as f32) {
                        continue;
                    }
                    canvas.rounded(Frame::new(x, y, cw as f32, ch as f32), 8.0, line);
                    canvas.rounded(Frame::new(x + 1.0, y + 1.0, cw as f32 - 2.0, ch as f32 - 2.0), 7.0, fill);
                }
            }
        }
        self.cells.damage(None);
    }

    pub fn update(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, _now: u64) -> std::io::Result<()> {
        let Some(ed) = home.editor.as_ref() else { return Ok(()) };
        self.paint_cells(kit, ed);
        let ink2 = kit.ui.look.ink2();
        let accent = kit.ui.look.accent();
        let danger = kit.ui.look.danger();
        let colour = if ed.refused() { danger } else { accent };
        match ed.sel_rect() {
            Some(r) => {
                let f = Frame::new(r.x as f32 - 6.0, r.y as f32 - 6.0, r.w as f32 + 12.0, r.h as f32 + 12.0);
                let radius = kit.tokens.radius(Radius::Card) + 6.0;
                let shape = if ed.mode == Mode::Select { plate::ring(colour, radius, 3.0) } else { Shape { corner_radius: radius, fill: Fill::Solid(faded(colour, 0.10)), border_width: 4.0, border: colour } };
                self.ring = kit.ui.scene.shape(f.w as u32, f.h as u32, self.z + 1, shape, Props::at(f.x, f.y));
                let show_corner = ed.mode == Mode::Resize || (ed.mode == Mode::Select && home.cursor_visible);
                let (cx, cy) = (r.x as f32 + r.w as f32 - 36.0 - 12.0, r.y as f32 + r.h as f32 - 36.0 - 12.0);
                self.corner.reshape(plate::solid(colour, 18.0));
                self.corner.set(if show_corner { Props::at(cx, cy) } else { HIDDEN });
                if let Some(g) = &self.corner_glyph {
                    g.set(if ed.mode == Mode::Resize { Props::at(cx + 7.0, cy + 7.0) } else { HIDDEN });
                }
            }
            None => {
                self.ring.set(HIDDEN);
                self.corner.set(HIDDEN);
            }
        }
        match ed.ghost {
            Some((r, ok)) => {
                self.ghost = kit.ui.scene.shape(r.w as u32, r.h as u32, self.z + 1, plate::ring(if ok { accent } else { danger }, kit.tokens.radius(Radius::Card), 3.0), Props::at(r.x as f32, r.y as f32));
            }
            None => self.ghost.set(HIDDEN),
        }
        let hint = ed.hint_words();
        self.bar_text.set_many(&mut kit.ui, &[(1, &hint, ink2)]);
        let bs = ed.buttons();
        let enabled_key: String = bs.iter().map(|b| if b.enabled { '1' } else { '0' }).collect();
        if enabled_key != self.enabled_key {
            self.enabled_key = enabled_key;
            for (b, spec) in self.buttons.iter_mut().zip(bs.iter()) {
                b.set_enabled(kit, engine, spec.enabled);
            }
        }
        for b in &mut self.buttons {
            b.set_hover(kit, home.focus.hovered() == Some(b.id));
        }
        let af = ed.add_frame();
        if ed.zone == Zone::Add {
            self.add_pane.set(Props::at(af.x, af.y));
            self.add_rows.set_props(Props::at(af.x, af.y));
            let rows = ed.add_rows();
            let ink = kit.ui.look.ink();
            let changes: Vec<(usize, String, Rgba)> = rows.iter().enumerate().map(|(i, r)| (2 * i + 1, if r.3 { "Hidden".into() } else { String::new() }, ink2)).collect();
            let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, s, c)| (*i, s.as_str(), *c)).collect();
            self.add_rows.set_many(&mut kit.ui, &refs);
            let _ = ink;
            if let Some(r) = rows.get(ed.add_index) {
                self.add_focus.set(Props::at(r.4.x, r.4.y));
            }
        } else {
            self.add_pane.set(HIDDEN);
            self.add_rows.set_props(HIDDEN);
            self.add_focus.set(HIDDEN);
        }
        if ed.toast != self.toast_key {
            self.toast_key = ed.toast.clone();
            if ed.toast.is_empty() {
                self.toast.set(HIDDEN);
                self.toast_text.set_props(HIDDEN);
            } else {
                let warning = kit.ui.look.colour(PaletteRole::Warning);
                let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), warning, Align::Centre);
                let w = (width_of(kit, style, &ed.toast) + 2.0 * kit.space(5)).round();
                let (x, y) = (((1920.0 - w) / 2.0).round(), 1080.0 - 54.0 - 56.0 - 8.0);
                let mut g = glass_of(kit, PaneKind::Pill, 56.0, false, false);
                g.tint = faded(warning, 0.18);
                self.toast = kit.ui.scene.glass(w as u32, 56, self.z + 10, g, Props::at(x, y));
                self.toast_text = Text::new(kit, Frame::new(x, y, w, 56.0), self.z + 11, style, &ed.toast)?;
                self.toast_text.set_props(Props::at(x, y));
            }
        }
        let (qf, _) = ed.ask_frames();
        let asking = ed.zone == Zone::Ask;
        self.ask_pane.set(if asking { Props::at(qf.x, qf.y) } else { HIDDEN });
        self.ask_title.set_props(if asking { Props::at(qf.x + 32.0, qf.y + 32.0) } else { HIDDEN });
        for b in &mut self.ask_buttons {
            if asking && !b.is_shown() {
                b.show(kit, false);
            } else if !asking && b.is_shown() {
                b.hide(kit, true);
            }
            b.set_hover(kit, home.focus.hovered() == Some(b.id));
        }
        let _ = (&self.bar, &self.cells);
        Ok(())
    }

    pub fn destroy(self, engine: &mut Engine) {
        for b in self.buttons {
            b.destroy(engine);
        }
        for b in self.ask_buttons {
            b.destroy(engine);
        }
    }
}

// ---- "Who's watching?"
struct ProfileTile {
    id: Id,
    lift: Lift,
    avatar: ImageLayer,
    name: Text,
    tick: Option<ImageLayer>,
}

pub struct PickerSurface {
    blur: GlassLayer,
    title: Text,
    tiles: Vec<ProfileTile>,
    dots: Vec<ShapeLayer>,
    pad_art: ImageLayer,
    pad_caps: Block,
    pad_cursor: Cursor,
    words: Text,
    pin_key: String,
    z: i32,
}

impl PickerSurface {
    pub fn new(kit: &mut Kit, engine: &mut Engine, home: &Home, z: i32) -> std::io::Result<Self> {
        let p = &home.picker;
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        // the page under a blur: a full-screen glass over the wallpaper (the cards step aside)
        let mut g = glass_of(kit, PaneKind::Sheet, 1080.0, false, false);
        g.corner_radius = 0.0;
        g.backdrop = Backdrop::Cached { radius: kit.tokens.blur * 1.5 };
        g.shadows = [None, None];
        let blur = kit.ui.scene.glass(1920, 1080, z, g, Props::at(0.0, 0.0));
        let mut title = Text::new(kit, Frame::new(0.0, 240.0, 1920.0, kit.line(TypeRole::Display)), z + 2, role_style(kit, TypeRole::Display, None, ink, Align::Centre), "Who\u{2019}s watching?")?;
        title.set_props(Props::at(0.0, 240.0));
        let mut tiles = Vec::new();
        for (i, pr) in p.profiles.iter().enumerate() {
            let id = Id(picker::TILE_ID + i as u32);
            let f = p.tile_frame(i);
            let lift = Lift::new(kit, engine, id, f, z + 3, PaneKind::Card, kit.tokens.focus.tile_scale, 0.0, false)?;
            let profile = serde_json::json!({"color": pr.color, "initial": pr.initial, "avatarFile": pr.avatar_file});
            let scene = kit.ui.scene.clone();
            let (avatar, name, tick) = lift
                .adopting(engine, &scene, || -> std::io::Result<_> {
                    let side = (f.w - 24.0) as u32;
                    let mut av = kit.ui.scene.image(side, side, z + 5, Props::at(12.0, 12.0))?;
                    HomeSurface::paint_avatar(kit, &mut av, &profile, side);
                    let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), if pr.active { ink } else { ink2 }, Align::Centre);
                    let mut name = Text::new(kit, Frame::new(-40.0, f.h + 16.0, f.w + 80.0, kit.line(TypeRole::Label)), z + 5, style, &pr.name)?;
                    name.set_props(Props::at(-40.0, f.h + 16.0));
                    // the current person's tick: an accent disc with the check in the accent's ink, over the tile's corner
                    let tick = if pr.active {
                        let mut disc = kit.ui.scene.image(44, 44, z + 6, Props::at(f.w - 40.0, -4.0))?;
                        {
                            let mut canvas = super::widgets::Canvas { px: disc.pixels_mut(), w: 44, h: 44 };
                            canvas.disc(22.0, 22.0, 22.0, kit.ui.look.accent());
                            if let Some(px) = kit.icons.draw("check", 26, kit.ui.look.accent_ink()) {
                                canvas.picture(&picture::Picture { width: 26, height: 26, rgba: px }, 9, 9, 26, 26);
                            }
                        }
                        disc.damage(None);
                        Some(disc)
                    } else {
                        None
                    };
                    Ok((av, name, tick))
                })
                .ok_or_else(|| std::io::Error::other("the tile's group"))??;
            tiles.push(ProfileTile { id, lift, avatar, name, tick });
        }
        let mut dots = Vec::new();
        for i in 0..4 {
            let x = 960.0 - 2.0 * 44.0 + i as f32 * 44.0 - 12.0;
            dots.push(kit.ui.scene.shape(24, 24, z + 4, plate::ring(ink2, 12.0, 3.0), Props { ..HIDDEN }).tap(|_| ()));
            let _ = x;
        }
        let pf = Frame::new(p.pad_frame(0).x, p.pad_frame(0).y, 3.0 * picker::PAD_KEY + 2.0 * picker::PAD_GAP, 4.0 * picker::PAD_KEY + 3.0 * picker::PAD_GAP);
        let mut pad_art = kit.ui.scene.image(pf.w as u32, pf.h as u32, z + 3, HIDDEN)?;
        {
            let erase = kit.icons.draw("delete", 32, ink).map(|rgba| picture::Picture { width: 32, height: 32, rgba });
            let (w, h) = (pad_art.width(), pad_art.height());
            let mut canvas = super::widgets::Canvas { px: pad_art.pixels_mut(), w, h };
            let glass = kit.ui.look.colour(PaletteRole::Glass);
            for (i, k) in picker::PAD_KEYS.iter().enumerate() {
                if k.is_empty() {
                    continue;
                }
                let f = p.pad_frame(i);
                canvas.rounded(Frame::new(f.x - pf.x, f.y - pf.y, f.w, f.h), kit.tokens.radius(Radius::Control), glass);
                if *k == "\u{232b}"
                    && let Some(e) = &erase
                {
                    canvas.picture(e, (f.x - pf.x + (f.w - 32.0) / 2.0).round() as i32, (f.y - pf.y + (f.h - 32.0) / 2.0).round() as i32, 32, 32);
                }
            }
        }
        pad_art.damage(None);
        let style = role_style(kit, TypeRole::Title, None, ink, Align::Centre);
        let cap = |k: &str| if k == "\u{232b}" { String::new() } else { k.to_owned() };
        let mut pad_caps = Block::new(&mut kit.ui, pf, z + 6, picker::PAD_KEYS.iter().enumerate().map(|(i, k)| { let f = p.pad_frame(i); (Frame::new(f.x - pf.x, f.y - pf.y, f.w, f.h), style, cap(k)) }).collect())?;
        pad_caps.set_props(HIDDEN);
        let pad_cursor = Cursor::new(kit, engine, PAD_X, PAD_Y, picker::PAD_KEY, picker::PAD_KEY, kit.tokens.radius(Radius::Control), z + 5)?;
        pad_cursor.glass.set(HIDDEN);
        let mut words = Text::new(kit, Frame::new(0.0, 940.0, 1920.0, kit.line(TypeRole::Label)), z + 2, role_style(kit, TypeRole::Label, None, ink2, Align::Centre), "")?;
        words.set_props(HIDDEN);
        Ok(Self { blur, title, tiles, dots, pad_art, pad_caps, pad_cursor, words, pin_key: String::new(), z })
    }

    pub fn update(&mut self, kit: &mut Kit, engine: &mut Engine, home: &Home, now: u64) -> std::io::Result<()> {
        let p = &home.picker;
        let asking = p.pin.is_some();
        for t in &self.tiles {
            t.lift.pose(engine, if asking { 0.0 } else { 1.0 }, !asking);
        }
        self.title.set(kit, if asking { "Enter the PIN" } else { "Who\u{2019}s watching?" });
        let pf = p.pad_frame(0);
        let pad_at = Props::at(pf.x, pf.y);
        self.pad_art.set(if asking { pad_at } else { HIDDEN });
        self.pad_caps.set_props(if asking { pad_at } else { HIDDEN });
        let ink2 = kit.ui.look.ink2();
        let accent = kit.ui.look.accent();
        let key = p.pin.as_ref().map(|pin| format!("{}:{}:{}", pin.digits.len(), pin.shake_until > 0, pin.locked_until > now)).unwrap_or_default();
        if key != self.pin_key {
            self.pin_key = key.clone();
            let n = p.pin.as_ref().map_or(0, |pin| pin.digits.len());
            let shake = p.pin.as_ref().is_some_and(|pin| pin.shake_until > now);
            for (i, d) in self.dots.iter_mut().enumerate() {
                let x = 960.0 - 2.0 * 44.0 + i as f32 * 44.0 - 12.0 + if shake { if i % 2 == 0 { 6.0 } else { -6.0 } } else { 0.0 };
                if asking {
                    d.reshape(if i < n { plate::solid(accent, 12.0) } else { plate::ring(ink2, 12.0, 3.0) });
                    d.set(Props::at(x, 400.0));
                } else {
                    d.set(HIDDEN);
                }
            }
        }
        let words = p.pin_words(now);
        self.words.set(kit, &words);
        self.words.set_props(if asking { Props::at(0.0, 940.0) } else { HIDDEN });
        if let Some(pin) = &p.pin {
            self.pad_cursor.glass.set(Props::at(0.0, 0.0));
            self.pad_cursor.go(kit, engine, p.pad_frame(pin.pad_index));
        } else {
            self.pad_cursor.glass.set(HIDDEN);
        }
        let _ = (&self.blur, self.z);
        for t in &self.tiles {
            let _ = (&t.avatar, &t.name, &t.tick, t.id);
        }
        Ok(())
    }

    /// The picker's words for the lint: the title and the names (over the blur, which the lint takes as the sheet).
    pub fn lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint, home: &Home) {
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        let p = lint.panel("picker", Frame::new(0.0, 0.0, 1920.0, 1080.0), paint, false);
        let ink = kit.ui.look.ink();
        let tf = self.title.frame();
        lint.word(kit, "picker.title", self.title.text(), tf, role_style(kit, TypeRole::Display, None, ink, Align::Centre), Some(p));
        for (i, t) in self.tiles.iter().enumerate() {
            let f = home.picker.tile_frame(i);
            let nf = t.name.frame();
            let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
            lint.word(kit, &format!("picker.name.{i}"), t.name.text(), Frame::new(f.x + nf.x, f.y + nf.y, nf.w, nf.h), style, Some(p));
        }
    }

    pub fn destroy(self, engine: &mut Engine) {
        for t in self.tiles {
            engine.flow.drop_lift(t.id);
        }
    }
}

trait Tap: Sized {
    fn tap(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }
}
impl<T> Tap for T {}

// ---- a tile's options
pub struct MenuSurface {
    pane: GlassLayer,
    rows: Block,
    focus: ShapeLayer,
    frame: Frame,
    key: String,
}

impl MenuSurface {
    pub fn new(kit: &mut Kit, home: &Home, z: i32) -> std::io::Result<Self> {
        let n = home.menu.len().max(1) as f32;
        let row = kit.tokens.row;
        let frame = Frame::new(((1920.0f32 - 520.0) / 2.0).round(), ((1080.0f32 - (n * row + 48.0)) / 2.0).round(), 520.0, n * row + 48.0);
        let pane = kit.ui.scene.glass(frame.w as u32, frame.h as u32, z, glass_of(kit, PaneKind::Sheet, frame.h, false, false), Props::at(frame.x, frame.y));
        let ink = kit.ui.look.ink();
        let danger = kit.ui.look.danger();
        let lines = home.menu.iter().enumerate().map(|(i, r)| (Frame::new(32.0, 24.0 + i as f32 * row, frame.w - 64.0, row), role_style(kit, TypeRole::Label, None, if r.danger { danger } else { ink }, Align::Left), r.label.clone())).collect();
        let rows = Block::new(&mut kit.ui, frame, z + 2, lines)?;
        let focus = kit.ui.scene.shape((frame.w - 24.0) as u32, row as u32, z + 1, plate::solid(faded(ink, kit.tokens.states.focus_wash[0]), kit.tokens.radius(Radius::Row)), HIDDEN);
        Ok(Self { pane, rows, focus, frame, key: String::new() })
    }

    pub fn update(&mut self, kit: &mut Kit, home: &Home) -> std::io::Result<()> {
        let key = format!("{}", home.menu_index);
        if key != self.key {
            self.key = key;
            self.focus.set(Props::at(self.frame.x + 12.0, self.frame.y + 24.0 + home.menu_index as f32 * kit.tokens.row));
        }
        let _ = (&self.pane, &self.rows);
        Ok(())
    }
}
