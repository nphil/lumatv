//! The Settings surface: the model's [`View`] drawn with the kit. The chrome (a scrim, the bar with the way back, the hints and what is
//! true now, the rail of sections, the pane of the page with its head, its tabs and its list of rows) is made once for a look and a
//! text size and kept; what changes (the clock, the hints, a row's switch, the chosen segment, the focus) is set in place; the rows
//! of a page are built where they can be seen (`list`). Everything hangs under one root group, so Settings fades in and out as one
//! thing on the layer beat. The focus is the kit's: the model decides where it is, and the engine's flowing lift shows each move.

pub mod chrome;
pub mod figure;
pub mod layout;
pub mod list;
pub mod paint;
pub mod rows;
pub mod sheets;

use self::chrome::{Bar, Head, Tabs};
use self::figure::{FigureArt, FigureGeo};
use self::layout::Sizes;
use self::list::{ListArt, row_of};
use self::paint::Paint;
use self::rows::other;
use self::sheets::Sheets;
use super::model::Target;
use super::row::Kind;
use super::view::{PageView, SheetView, StopView, View, Zone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::glyph_side;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::{Gesture, Pointer};
use crate::kit::{Engine, Kit, z};
use compositor_protocol::{Ease, GroupPose, Props};
use lumatv_tokens::TypeRole;
use ui::components::plate;
use ui::geometry::Frame;
use ui::group::Group;
use ui::motion::{Motion, MotionBounds, MotionToken, Settle, Units};
use ui::scene::{ImageLayer, ShapeLayer};

pub const RAIL_ID: u32 = 1_000;
pub const TAB_ID: u32 = 3_000;
pub const BACK_ID: u32 = 3_100;
pub const BAR_BACK_ID: u32 = 3_101;
pub const SHEET_ID: u32 = 3_200;
pub const PICK_ID: u32 = 4_000;
pub const REGION_RAIL: Region = Region(1);
pub const REGION_PAGE: Region = Region(2);
pub const REGION_TABS: Region = Region(3);
pub const REGION_HEAD: Region = Region(4);
pub const REGION_SHEET: Region = Region(5);
pub const SHEET_SCOPE: Scope = Scope(7);

pub(crate) const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

/// The screen's measures, as the QML Settings lays them out and as the text size scales them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub bar: Frame,
    pub rail: Frame,
    pub page: Frame,
    pub rail_list: Frame,
    pub content: Frame,
}

impl Geometry {
    pub fn of(kit: &Kit, sz: &Sizes) -> Self {
        let title = kit.line(TypeRole::Title);
        let glyph = glyph_side(kit, TypeRole::Caption) as f32;
        let bar_h = sz.target.max(title + sz.s2).max(glyph + sz.s2).ceil();
        let bar = Frame::new(64.0, 56.0, 1792.0, bar_h);
        let top = bar.bottom() + 17.0;
        let h = 1020.0 - top;
        let rail = Frame::new(64.0, top, 446.0, h);
        let page = Frame::new(536.0, top, 1320.0, h);
        let rail_list = Frame::new(rail.x + sz.s3, rail.y + sz.s3, rail.w - 2.0 * sz.s3, rail.h - 2.0 * sz.s3);
        let content = Frame::new(page.x + sz.s6, page.y + sz.s5, page.w - 2.0 * sz.s6, page.h - 2.0 * sz.s5);
        Geometry { bar, rail, page, rail_list, content }
    }
}

/// What is made for a look and a text size.
struct Built {
    z: i32,
    generation: u64,
    scale_bits: u32,
    paint: Paint,
    sz: Sizes,
    geo: Geometry,
    root: Group,
    fade: Motion,
    _scrim: ShapeLayer,
    bar: Bar,
    _rail_pane: Pane,
    _page_pane: Pane,
    rail: Option<ListArt>,
    head: Option<Head>,
    tabs: Option<Tabs>,
    figure: Option<FigureArt>,
    /// Where the page's rows are on the screen (what a picker keeps to, so the picture over them stays in sight).
    viewport: Frame,
    list: Option<ListArt>,
    chevrons: Option<(ImageLayer, ImageLayer)>,
    sheets: Sheets,
    over_video: bool,
}

pub struct SettingsSurface {
    z: i32,
    built: Option<Built>,
    pointer: Pointer,
    hover: Option<Id>,
    last_xy: (f32, f32),
    buttons: u32,
    /// When the fade out ends and the layers go (ms).
    closing_at: Option<u64>,
    /// The ids of the page's rows in the order the model last gave them, for the pointer's targets.
    row_ids: Vec<String>,
    view: Option<View>,
}

impl SettingsSurface {
    /// `system_z` is where stack 1 begins; Settings draws in its own band over it ([`z::SETTINGS`]).
    pub fn new(system_z: i32) -> Self {
        SettingsSurface { z: system_z + z::SETTINGS, built: None, pointer: Pointer::new(), hover: None, last_xy: (0.0, 0.0), buttons: 0, closing_at: None, row_ids: Vec::new(), view: None }
    }

    pub fn is_up(&self) -> bool {
        self.built.is_some()
    }

    /// Everything is made again at the next draw (the look or the text size changed).
    pub fn restyle(&mut self) {
        self.built = None;
    }

    pub fn due_at(&self) -> Option<u64> {
        self.closing_at
    }

    /// The fade out is over: the layers go.
    pub fn tick(&mut self, engine: &mut Engine, now: u64) -> bool {
        if self.closing_at.is_some_and(|at| now >= at) {
            self.closing_at = None;
            self.drop_all(engine);
            return true;
        }
        false
    }

    fn drop_all(&mut self, engine: &mut Engine) {
        if let Some(mut b) = self.built.take() {
            if let Some(l) = &mut b.list {
                l.release(engine);
            }
            if let Some(l) = &mut b.rail {
                l.release(engine);
            }
            b.sheets.release(engine);
            for i in 0..16 {
                engine.focus.unregister(Id(TAB_ID + i));
            }
            engine.focus.unregister(Id(BACK_ID));
            engine.focus.unregister(Id(BAR_BACK_ID));
        }
        self.row_ids.clear();
        self.view = None;
        self.hover = None;
    }

    /// The rectangle of the page's pane and the rail's, for the host's checks of the picture.
    pub fn geometry(&self) -> Option<Geometry> {
        self.built.as_ref().map(|b| b.geo)
    }

    /// The layers Settings holds now (the scene's budget).
    pub fn layers(&self, kit: &Kit) -> usize {
        let _ = kit;
        0
    }

    pub fn row_count_built(&self) -> usize {
        self.built.as_ref().and_then(|b| b.list.as_ref()).map_or(0, ListArt::built)
    }

    // ---------------------------------------------------------------------------------------------------------------- drawing

    /// Draws `view` at `now` (ms, the models' clock) and `now_ns` (the engine's clock). Returns whether anything of Settings is on screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, over_video: bool, now: u64, now_ns: u64) -> std::io::Result<bool> {
        if !view.open {
            if let Some(b) = &self.built
                && self.closing_at.is_none()
            {
                let ms = kit.beat_ms(Beat::Layer);
                let _ = b.fade.settle(Some(0.0), None, Settle::Timed(MotionToken { duration_ms: ms, ease: Ease::In }));
                self.closing_at = Some(now + u64::from(ms) + 40);
                if ms == 0 {
                    self.tick(engine, now + 100);
                }
            }
            return Ok(self.built.is_some());
        }
        self.closing_at = None;
        let scale_bits = kit.text_scale.to_bits();
        let needs_build = self.built.as_ref().is_none_or(|b| b.generation != kit.generation() || b.scale_bits != scale_bits || b.over_video != over_video);
        if needs_build {
            self.drop_all(engine);
            let b = self.build(kit, engine, over_video)?;
            self.built = Some(b);
            let ms = kit.beat_ms(Beat::Layer);
            if let Some(b) = &self.built {
                let _ = b.fade.settle(Some(1.0), if ms == 0 { Some(0.0) } else { None }, Settle::Timed(MotionToken { duration_ms: ms, ease: Ease::Out }));
            }
        }
        self.row_ids = view.page.as_ref().map(|p| p.rows.iter().map(|r| r.id.clone()).collect()).unwrap_or_default();
        let hover = self.hover;
        let b = self.built.as_mut().expect("built above");
        let paint = b.paint;
        let sz = b.sz;
        let geo = b.geo;
        // the bar: the clock, what is true now, and the hints (its layers hang under the root)
        let scene = kit.ui.scene.clone();
        {
            let Built { root, bar, z: bz, .. } = b;
            scene.adopting_view(root, || bar.update(kit, &paint, &sz, &geo, *bz, &view.bar, &view.hints))?;
        }
        // the rail
        let rail_rows: Vec<super::row::Row> = view
            .rail
            .iter()
            .map(|r| {
                let mut row = super::row::Row::link(&r.id).icon(&r.icon).label(&r.title).detail(&r.summary).chevron(false);
                row.accent = r.selected;
                row.wash = r.selected;
                row
            })
            .collect();
        let rail_stop = view.rail.iter().position(|r| r.focused).or_else(|| view.rail.iter().position(|r| r.selected));
        let rail_hover = hover.and_then(|h| row_of(RAIL_ID, rail_rows.len(), h)).map(|(i, _)| i);
        if b.rail.is_none() {
            let rl = ListArt::new(kit, engine, Some(&b.root), "rail".into(), (geo.rail_list.x, geo.rail_list.y), (geo.rail_list.w, geo.rail_list.h), REGION_RAIL, Scope::BASE, self.z + 10, RAIL_ID, true)?;
            b.rail = Some(rl);
        }
        if let Some(rl) = &mut b.rail {
            rl.apply(kit, engine, paint, &rail_rows, rail_stop, rail_hover, 0, view.zone == Zone::Rail, now_ns, false)?;
        }
        // the page
        match &view.page {
            Some(page) => self.page(kit, engine, view, page, hover, now_ns)?,
            None => {
                let b = self.built.as_mut().expect("built");
                if let Some(l) = &mut b.list {
                    l.release(engine);
                }
                b.list = None;
                b.head = None;
                b.tabs = None;
                if let Some((u, d)) = &b.chevrons {
                    u.set(HIDDEN);
                    d.set(HIDDEN);
                }
            }
        }
        let _ = (sz, paint);
        // the sheets
        let b = self.built.as_mut().expect("built");
        let page_rect = b.geo.page;
        let anchor = view.sheet.as_ref().and_then(|s| match s {
            SheetView::Help { row, .. } => self.row_ids.iter().position(|r| r == row).and_then(|i| b.list.as_ref().and_then(|l| l.frame_of(i))),
            _ => None,
        });
        let anchor = anchor.or_else(|| match (&view.sheet, &view.page) {
            (Some(SheetView::Pick { .. }), Some(p)) => match p.stop {
                StopView::Row(i) => b.list.as_ref().and_then(|l| l.frame_of(i)),
                _ => None,
            },
            _ => None,
        });
        let pick_hover = hover.and_then(|h| row_of(PICK_ID, 64, h)).map(|(i, _)| i);
        let bounds = if matches!(view.sheet, Some(SheetView::Pick { .. })) && b.viewport.h > 0.0 { b.viewport } else { page_rect };
        b.sheets.apply(kit, engine, paint, view.sheet.as_ref(), anchor, bounds, pick_hover, now_ns)?;
        self.sync_focus(kit, engine, view, now_ns);
        self.view = Some(view.clone());
        Ok(true)
    }

    fn build(&mut self, kit: &mut Kit, engine: &mut Engine, over_video: bool) -> std::io::Result<Built> {
        let sz = Sizes::of(kit);
        let paint = Paint::of(kit);
        let geo = Geometry::of(kit, &sz);
        let z = self.z;
        let scene = kit.ui.scene.clone();
        let root = scene.group(None, GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let fade = scene.motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress)).map_err(other)?;
        root.bind(
            &fade,
            compositor_protocol::PoseBinding {
                values: [0.0, 1.0],
                from: GroupPose { opacity: 0.0, ..GroupPose::IDENTITY },
                to: GroupPose::IDENTITY,
                properties: compositor_protocol::PoseProperties::OPACITY,
            },
        )
        .map_err(other)?;
        let (scrim, bar, rail_pane, page_pane) = scene.adopting_view(&root, || -> std::io::Result<_> {
            let scrim = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(paint.scrim, 0.0));
            let rail_pane = Pane::new(kit, geo.rail, z + 2, PaneKind::Sheet, over_video);
            let page_pane = Pane::new(kit, geo.page, z + 2, PaneKind::Sheet, over_video);
            let bar = Bar::new(kit, engine, &paint, &sz, &geo, z, over_video)?;
            Ok((scrim, bar, rail_pane, page_pane))
        })?;
        let built = Built {
            z,
            generation: kit.generation(),
            scale_bits: kit.text_scale.to_bits(),
            paint,
            sz,
            geo,
            root,
            fade,
            _scrim: scrim,
            bar,
            _rail_pane: rail_pane,
            _page_pane: page_pane,
            rail: None,
            head: None,
            tabs: None,
            figure: None,
            viewport: Frame::new(0.0, 0.0, 0.0, 0.0),
            list: None,
            chevrons: None,
            sheets: Sheets::new(z + 100),
            over_video,
        };
        Ok(built)
    }

    /// The page: its head, its tabs and its rows, made again when what they are made of changed.
    fn page(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, page: &PageView, hover: Option<Id>, now_ns: u64) -> std::io::Result<()> {
        let z = self.z;
        let b = self.built.as_mut().expect("built");
        let (sz, paint, geo) = (b.sz, b.paint, b.geo);
        let scene = kit.ui.scene.clone();
        // the head
        let head_sig = format!("{}|{}|{}|{:?}|{}", page.title, page.subtitle, page.back.as_deref().unwrap_or(""), page.chip, page.id);
        if b.head.as_ref().is_none_or(|h| h.sig != head_sig) {
            engine.focus.unregister(Id(BACK_ID));
            let head = scene.adopting_view(&b.root, || Head::build(kit, engine, &paint, &sz, &geo, page, head_sig.clone(), z + 12))?;
            b.head = Some(head);
        }
        let head_h = b.head.as_ref().map_or(0.0, |h| h.height);
        // the tabs
        let tabs_y = geo.content.y + head_h + sz.s4;
        let tab_h = if page.tabs.len() > 1 { sz.target } else { 0.0 };
        let after_tabs = if page.tabs.len() > 1 { tabs_y + tab_h + sz.s4 } else { geo.content.y + head_h + sz.s4 };
        if page.tabs.len() > 1 {
            let sig = format!("{}|{}|{}", page.id, page.tabs.join(","), tabs_y);
            if b.tabs.as_ref().is_none_or(|t| t.sig != sig) {
                let tabs = scene.adopting_view(&b.root, || Tabs::build(kit, engine, &paint, &sz, &geo, page, sig, tabs_y, z + 12))?;
                b.tabs = Some(tabs);
            }
            if let Some(t) = &mut b.tabs {
                let focus = page.active && page.stop == StopView::Tabs;
                let hovered = hover.and_then(|h| h.0.checked_sub(TAB_ID)).filter(|i| (*i as usize) < page.tabs.len()).map(|i| i as usize);
                t.update(kit, engine, &paint, &sz, page.tab, focus, hovered);
            }
        } else {
            for i in 0..16 {
                engine.focus.unregister(Id(TAB_ID + i));
            }
            b.tabs = None;
        }
        // the picture over the rows, when the page has one: it stays while the rows scroll under it
        let list_y = match &page.figure {
            Some(fig) => {
                let fg = FigureGeo::of(kit, &sz, geo.content.x + sz.s6, after_tabs);
                let sig = format!("{}|{}", page.id, FigureArt::signature(kit, fig, &fg));
                let b = self.built.as_mut().expect("built");
                if b.figure.as_ref().is_none_or(|f| f.sig() != sig) {
                    let art = scene.adopting_view(&b.root, || FigureArt::new(kit, &paint, &sz, &b.root, fg, fig, z + 14, sig.clone()))?;
                    b.figure = Some(art);
                }
                if let Some(f) = &mut b.figure {
                    f.update(kit, &paint, &sz, fig);
                }
                after_tabs + fg.height + sz.s4
            }
            None => {
                self.built.as_mut().expect("built").figure = None;
                after_tabs
            }
        };
        let b = self.built.as_mut().expect("built");
        // the list: made again for another page or tab or head height
        let viewport = Frame::new(geo.content.x, list_y, geo.content.w, geo.content.bottom() - list_y);
        b.viewport = viewport;
        let key = format!("{}|{}|{}|{}", page.id, page.tab, list_y, viewport.h);
        if b.list.as_ref().is_none_or(|l| l.key() != key) {
            if let Some(l) = &mut b.list {
                l.release(engine);
            }
            let list = ListArt::new(kit, engine, Some(&b.root), key, (viewport.x, viewport.y), (viewport.w, viewport.h), REGION_PAGE, Scope::BASE, z + 20, list::ROW_ID, false)?;
            b.list = Some(list);
        }
        let stop = match page.stop {
            StopView::Row(i) => Some(i),
            _ => None,
        };
        let hovered = hover.and_then(|h| row_of(list::ROW_ID, page.rows.len(), h));
        let (hov_row, hov_card) = (hovered.map(|(r, _)| r), hovered.and_then(|(r, c)| c.map(|c| (r, c))));
        let focused = page.active && stop.is_some();
        if let Some(l) = &mut b.list {
            l.apply(kit, engine, paint, &page.rows, stop, hov_row, page.seg, focused, now_ns, false)?;
            if let Some((r, c)) = hov_card {
                l.set_card_hover(r, Id(list::id_at(list::ROW_ID, r).0 + 1 + c as u32), true);
            }
        }
        // the chevrons: more rows that way
        let (above, below, zone) = b.list.as_ref().map_or((false, false, 0.0), |l| (l.more_above(), l.more_below(), if l.total() > viewport.h { sz.s6 } else { 0.0 }));
        let side = sz.s5.round() as u32;
        if b.chevrons.is_none() {
            let up = scene.adopting_view(&b.root, || make_chevron(kit, &paint, side, true, z + 24))?;
            let down = scene.adopting_view(&b.root, || make_chevron(kit, &paint, side, false, z + 24))?;
            if let (Some(u), Some(d)) = (up, down) {
                b.chevrons = Some((u, d));
            }
        }
        if let Some((u, d)) = &b.chevrons {
            let x = viewport.x + ((viewport.w - side as f32) / 2.0).round();
            u.set(if above { Props::at(x, viewport.y + ((zone - side as f32) / 2.0).round()) } else { HIDDEN });
            d.set(if below { Props::at(x, viewport.bottom() - zone + ((zone - side as f32) / 2.0).round()) } else { HIDDEN });
        }
        let _ = view;
        Ok(())
    }

    /// The engine's focus is where the model's is: the flowing lift shows the move.
    fn sync_focus(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ns: u64) {
        let _ = kit;
        // a question over the page is a modal scope of its own
        let asking = matches!(view.sheet, Some(SheetView::Confirm { .. } | SheetView::Pick { .. }));
        if asking && engine.focus.modal() != Some(SHEET_SCOPE) {
            engine.focus.open_modal(SHEET_SCOPE);
        } else if !asking && engine.focus.modal() == Some(SHEET_SCOPE) {
            engine.focus.close_modal();
        }
        let target = match &view.sheet {
            Some(SheetView::Confirm { index, .. }) => Some(Id(SHEET_ID + *index as u32)),
            Some(SheetView::Pick { index, .. }) => Some(list::id_at(PICK_ID, *index)),
            _ => match view.zone {
                Zone::Rail => view.rail.iter().position(|r| r.focused).map(|i| list::id_at(RAIL_ID, i)),
                Zone::Page => view.page.as_ref().and_then(|p| match p.stop {
                    StopView::Tabs => Some(Id(TAB_ID + p.tab as u32)),
                    StopView::Row(i) => {
                        let id = list::id_at(list::ROW_ID, i);
                        Some(if p.rows.get(i).is_some_and(|r| r.kind == Kind::Cards) { Id(id.0 + 1 + p.seg as u32) } else { id })
                    }
                    StopView::None => None,
                }),
            },
        };
        match target {
            Some(id) => {
                if engine.focus.current() != Some(id) {
                    engine.focus(id, now_ns);
                }
            }
            None => {
                if engine.focus.current().is_some() {
                    engine.clear(now_ns);
                }
            }
        }
    }

    // --------------------------------------------------------------------------------------------------------------- pointer

    /// One sample of the pointer: the cue follows it, a click is what OK would be. The targets of the clicks.
    pub fn pointer(&mut self, engine: &mut Engine, now_ns: u64, x: f32, y: f32, buttons: u32, visible: bool) -> Vec<Target> {
        let mut out = Vec::new();
        let rising = buttons & 1 != 0 && self.buttons & 1 == 0;
        let falling = buttons & 1 == 0 && self.buttons & 1 != 0;
        self.buttons = buttons;
        self.last_xy = (x, y);
        let (help_up, confirm) = match self.view.as_ref().and_then(|v| v.sheet.as_ref()) {
            Some(SheetView::Help { .. }) => (true, None),
            Some(SheetView::Confirm { .. } | SheetView::Pick { .. }) => (false, Some(())),
            _ => (false, None),
        };
        let gestures = self.pointer.sample(engine, now_ns, x, y, buttons, visible);
        let mut clicked = false;
        for g in gestures {
            match g {
                Gesture::Hover { to, .. } => self.hover = to,
                Gesture::Click(id) => {
                    clicked = true;
                    if help_up {
                        out.push(Target::Outside);
                    } else if let Some(t) = self.target_of(id, x, y) {
                        out.push(t);
                    }
                }
                Gesture::Down(id) => {
                    if let Some(b) = self.built.as_mut() {
                        b.sheets.set_pressed(id, true);
                    }
                }
                Gesture::Up(Some(id)) => {
                    if let Some(b) = self.built.as_mut() {
                        b.sheets.set_pressed(id, false);
                    }
                }
                Gesture::Up(None) => {}
            }
        }
        if !visible {
            self.hover = None;
        }
        if help_up && rising && !clicked {
            out.push(Target::Outside);
        } else if confirm.is_some() && falling && !clicked && self.view.is_some() {
            let on_card = self.built.as_ref().is_some_and(|b| b.sheets.card_contains(x, y));
            if !on_card {
                out.push(Target::Outside);
            }
        }
        out
    }

    /// A key of the remote: the cue goes with the pointer.
    pub fn key(&mut self, engine: &mut Engine) {
        self.pointer.key(engine);
        self.hover = None;
    }

    /// The cue's control, for the surface's own redraw.
    pub fn hovered(&self) -> Option<Id> {
        self.hover
    }

    /// What a click on the control `id` at (`x`, `y`) is, as the model takes it.
    fn target_of(&self, id: Id, x: f32, y: f32) -> Option<Target> {
        let view = self.view.as_ref()?;
        if let Some(SheetView::Pick { rows, .. }) = &view.sheet
            && let Some((i, _)) = row_of(PICK_ID, rows.len(), id)
        {
            return Some(Target::Pick(i));
        }
        if let Some(k) = id.0.checked_sub(SHEET_ID).filter(|k| *k < 8) {
            return Some(Target::Answer(k as usize));
        }
        if id.0 == BACK_ID || id.0 == BAR_BACK_ID {
            return Some(Target::Back);
        }
        if let Some(t) = id.0.checked_sub(TAB_ID).filter(|t| (*t as usize) < 16) {
            return Some(Target::Tab(t as usize));
        }
        if let Some((i, _)) = row_of(RAIL_ID, view.rail.len(), id) {
            return Some(Target::Rail(i));
        }
        let page = view.page.as_ref()?;
        let (i, card) = row_of(list::ROW_ID, page.rows.len(), id)?;
        let row = page.rows.get(i)?;
        let b = self.built.as_ref()?;
        let (_, control, info) = b.list.as_ref()?.control_of(i)?;
        if info.is_some_and(|f| f.contains(x, y)) {
            return Some(Target::Info(row.id.clone()));
        }
        match row.kind {
            Kind::Cards => Some(Target::Row { id: row.id.clone(), seg: card, value: None }),
            Kind::Seg => {
                let c = control?;
                let seg = if c.contains(x, y) { b.list.as_ref()?.art(i).and_then(|_| seg_at(row, c, x)) } else { None };
                Some(Target::Row { id: row.id.clone(), seg: Some(seg.unwrap_or(0)), value: None })
            }
            Kind::Slider => {
                let c = control?;
                let value = c.contains(x, y).then(|| {
                    let f = ((x - c.x) / c.w).clamp(0.0, 1.0) as f64;
                    let raw = row.min + f * (row.max - row.min);
                    let step = if row.step > 0.0 { row.step } else { 1.0 };
                    (((raw - row.min) / step).round() * step + row.min).clamp(row.min, row.max)
                });
                Some(Target::Row { id: row.id.clone(), seg: None, value })
            }
            _ => Some(Target::Row { id: row.id.clone(), seg: None, value: None }),
        }
    }

    /// Describes what is on screen to the layout lint: the three panes, the words of the rows in view (the rail's and the page's), the
    /// picture's caption and the picker's strip.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut crate::kit::lint::Lint) {
        let (Some(b), Some(view)) = (&self.built, &self.view) else { return };
        let paint = b.paint;
        lint.panel("settings bar", b.geo.bar, paint.sheet, true);
        let rail = lint.panel("settings rail", b.geo.rail, paint.sheet, true);
        let page = lint.panel("settings page", b.geo.page, paint.sheet, true);
        if let Some(l) = &b.rail {
            let rows: Vec<super::row::Row> = view.rail.iter().map(|r| super::row::Row::link(&r.id).icon(&r.icon).label(&r.title).detail(&r.summary).chevron(false)).collect();
            l.feed_lint(kit, &paint, &rows, lint, rail);
        }
        if let (Some(l), Some(p)) = (&b.list, &view.page) {
            l.feed_lint(kit, &paint, &p.rows, lint, page);
        }
        if let Some(f) = &b.figure {
            for (name, frame) in f.frames() {
                lint.panel(&format!("figure {name}"), frame, compositor_protocol::Rgba(0, 0, 0, 0), true);
            }
        }
        if let Some(r) = b.sheets.strip_frame() {
            lint.panel("picker strip", r, compositor_protocol::Rgba(0, 0, 0, 0), true);
        }
    }

    /// Where things are on the screen, for the host's checks and the lint.
    pub fn frames(&self) -> Vec<(String, Frame)> {
        let mut out = Vec::new();
        if let Some(b) = &self.built {
            out.push(("bar".into(), b.geo.bar));
            out.push(("rail".into(), b.geo.rail));
            out.push(("page".into(), b.geo.page));
            if let Some(l) = &b.list {
                for (i, id) in l.ids().iter().enumerate() {
                    if let Some(f) = l.frame_of(i) {
                        out.push((format!("row:{id}"), f));
                    }
                }
            }
        }
        out
    }
}

/// The segment of a segmented row under `x`: the segments share the control's width.
fn seg_at(row: &super::row::Row, control: Frame, x: f32) -> Option<usize> {
    let n = row.options.len();
    if n == 0 {
        return None;
    }
    let w = control.w / n.min(n) as f32;
    Some((((x - control.x) / w) as usize).min(n - 1))
}

fn make_chevron(kit: &mut Kit, paint: &Paint, side: u32, up: bool, z: i32) -> std::io::Result<Option<ImageLayer>> {
    let Some(mut px) = kit.icons.draw("chevron-down", side, paint.ink3) else { return Ok(None) };
    if up {
        let row = side as usize * 4;
        for y in 0..side as usize / 2 {
            let (a, b) = (y * row, (side as usize - 1 - y) * row);
            for x in 0..row {
                px.swap(a + x, b + x);
            }
        }
    }
    let mut layer = kit.ui.scene.image(side, side, z, HIDDEN)?;
    layer.pixels_mut().copy_from_slice(&px);
    layer.damage(None);
    Ok(Some(layer))
}
