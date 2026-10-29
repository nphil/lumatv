//! The parts of the screen that are not rows: the bar (the way back, the title, the hints, what is true now, the time), the head of a
//! page (the back pill of a child page, the title, the subtitle, the chip) and the tab bar. Each is made once for what it shows and
//! set in place when it moves; layers are made under the surface's root group (the caller wraps the call in `adopting_view`).

use super::layout::{SegGeom, Sizes, seg_geom};
use super::paint::Paint;
use super::rows::pill;
use super::{BACK_ID, BAR_BACK_ID, Geometry, HIDDEN, REGION_HEAD, REGION_TABS, TAB_ID};
use crate::kit::button::faded;
use crate::kit::focus::Scope;
use crate::kit::hint::{Hint, HintRow, RowAt, glyph_side};
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Engine, Kit, icon};
use crate::settings::view::{Bar as BarView, PageView};
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::Align;
use ui::components::text::{Run, ellipsise, line_height};
use ui::components::text_layer::{Item, TextLayer};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};

const STATUS_ICON: u32 = 28;

/// The bar: the way back, the title, the hints, what is true now and the time.
pub struct Bar {
    pub pane: Pane,
    _chevron: Option<ImageLayer>,
    _title: Text,
    clock: Text,
    clock_text: String,
    clock_x: f32,
    status: Vec<ImageLayer>,
    status_key: Option<(bool, bool)>,
    cluster_left: f32,
    hints: Vec<Hint>,
    hint_rows: Vec<HintRow>,
    title_end: f32,
}

impl Bar {
    pub fn new(kit: &mut Kit, engine: &mut Engine, paint: &Paint, sz: &Sizes, geo: &Geometry, z: i32, over_video: bool) -> std::io::Result<Bar> {
        let pane = Pane::new(kit, geo.bar, z + 2, PaneKind::Strip, over_video);
        let chevron = icon::layer(kit, "chevron-left", STATUS_ICON, paint.ink2, z + 12, Props::at(geo.bar.x + sz.s5, geo.bar.y + ((geo.bar.h - STATUS_ICON as f32) / 2.0).round()))?;
        let style = role_style(kit, TypeRole::Title, None, paint.ink, Align::Left);
        let title_w = width_of(kit, style, "Settings");
        let tline = kit.line(TypeRole::Title);
        let tx = geo.bar.x + sz.s5 + 40.0;
        let title = Text::new(kit, Frame::new(tx, geo.bar.y + ((geo.bar.h - tline) / 2.0).round(), title_w, tline), z + 12, style, "Settings")?;
        let cstyle = role_style(kit, TypeRole::Label, None, paint.ink2, Align::Right);
        let clock_w = width_of(kit, cstyle, "12:59 PM");
        let lline = kit.line(TypeRole::Label);
        let clock_x = geo.bar.right() - sz.s6 - clock_w;
        let clock = Text::new(kit, Frame::new(clock_x, geo.bar.y + ((geo.bar.h - lline) / 2.0).round(), clock_w, lline), z + 12, cstyle, "")?;
        // the pointer's way back: the left of the bar is the Back key
        engine.focus.register(crate::kit::focus::Id(BAR_BACK_ID), Frame::new(geo.bar.x, geo.bar.y, 360.0, geo.bar.h), REGION_HEAD, Scope::BASE);
        Ok(Bar {
            pane,
            _chevron: chevron,
            _title: title,
            clock,
            clock_text: String::new(),
            clock_x,
            status: Vec::new(),
            status_key: None,
            cluster_left: clock_x - sz.s5,
            hints: Vec::new(),
            hint_rows: Vec::new(),
            title_end: tx + title_w + sz.s6,
        })
    }

    /// The clock, the cable or the Wi-Fi (drawn only with no link), and the hints.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, kit: &mut Kit, paint: &Paint, sz: &Sizes, geo: &Geometry, z: i32, state: &BarView, hints: &[Hint]) -> std::io::Result<()> {
        if self.clock_text != state.clock {
            self.clock.set(kit, &state.clock);
            self.clock_text = state.clock.clone();
        }
        // quiet while the TV is online: the link's glyph is only drawn when there is none
        let key = (state.wired, state.link);
        if self.status_key != Some(key) {
            self.status.clear();
            self.cluster_left = self.clock_x - sz.s5;
            if !state.link {
                let y = geo.bar.y + ((geo.bar.h - STATUS_ICON as f32) / 2.0).round();
                let x = self.clock_x - sz.s5 - STATUS_ICON as f32;
                self.status.extend(icon::layer(kit, if state.wired { "ethernet-port" } else { "wifi" }, STATUS_ICON, kit.ui.look.colour(PaletteRole::Warning), z + 12, Props::at(x, y))?);
                self.cluster_left = x - sz.s5;
            }
            self.status_key = Some(key);
            self.hints.clear();
        }
        if self.hints != hints {
            self.hint_rows.clear();
            let side = glyph_side(kit, TypeRole::Caption) as f32;
            let gap = kit.space(2);
            let ink = paint.ink2;
            let widths: Vec<f32> = hints
                .iter()
                .map(|h| {
                    let n = h.glyphs().len() as f32;
                    match &h.label {
                        Some(l) => {
                            let ty = kit.ty(TypeRole::Caption);
                            n * (side + gap) + kit.ui.fonts.measure(l, ty).width.ceil() + 2.0
                        }
                        None => n * (side + gap) - gap,
                    }
                })
                .collect();
            // from the right: what does not fit before the title is left out
            let mut right = self.cluster_left;
            let mut placed: Vec<(usize, f32)> = Vec::new();
            for (i, w) in widths.iter().enumerate().rev() {
                let left = right - w;
                if left < self.title_end {
                    break;
                }
                placed.push((i, left));
                right = left - sz.s6;
            }
            for (i, x) in placed.into_iter().rev() {
                self.hint_rows.push(HintRow::new(kit, &hints[i], TypeRole::Caption, RowAt { x, y: geo.bar.y, h: geo.bar.h, z: z + 12 }, ink)?);
            }
            self.hints = hints.to_vec();
        }
        Ok(())
    }
}

fn tone_colour(kit: &Kit, tone: &str) -> Option<Rgba> {
    let role = match tone {
        "positive" => PaletteRole::Positive,
        "warning" => PaletteRole::Warning,
        "danger" => PaletteRole::Danger,
        "info" => PaletteRole::Info,
        "live" => PaletteRole::Live,
        _ => return None,
    };
    Some(kit.ui.look.colour(role))
}

/// The head of a page: the back pill (a child page), the title, the subtitle and the chip.
pub struct Head {
    pub sig: String,
    _pill: Option<ShapeLayer>,
    _icon: Option<ImageLayer>,
    _words: Vec<Text>,
    _chip: Vec<ShapeLayer>,
    _chip_text: Option<Text>,
    pub height: f32,
}

impl Head {
    #[allow(clippy::too_many_arguments)]
    pub fn build(kit: &mut Kit, engine: &mut Engine, paint: &Paint, sz: &Sizes, geo: &Geometry, page: &PageView, sig: String, z: i32) -> std::io::Result<Head> {
        let c = geo.content;
        let tstyle = role_style(kit, TypeRole::Title, None, paint.ink, Align::Left);
        let tline = kit.line(TypeRole::Title);
        let mut title_x = c.x;
        let mut pill_layer = None;
        let mut icon_layer = None;
        let mut words = Vec::new();
        if let Some(back) = &page.back {
            let h = (sz.target * 0.7).round();
            let side = (sz.target * 0.36).round() as u32;
            let cstyle = role_style(kit, TypeRole::Caption, None, paint.ink2, Align::Left);
            let text_w = width_of(kit, cstyle, back);
            let w = sz.s3 + side as f32 + sz.s1 + text_w + sz.s4;
            let y = c.y + ((tline - h) / 2.0).round();
            pill_layer = Some(kit.ui.scene.shape(w.round() as u32, h as u32, z, pill(paint.wash_selected, paint.edge, (h / 2.0).floor(), 1.0), Props::at(c.x, y)));
            icon_layer = icon::layer(kit, "chevron-left", side, paint.ink2, z + 1, Props::at(c.x + sz.s3, y + ((h - side as f32) / 2.0).round()))?;
            let cline = kit.line(TypeRole::Caption);
            words.push(Text::new(kit, Frame::new(c.x + sz.s3 + side as f32 + sz.s1, y + ((h - cline) / 2.0).round(), text_w, cline), z + 1, cstyle, back)?);
            engine.focus.register(crate::kit::focus::Id(BACK_ID), Frame::new(c.x, y, w, h), REGION_HEAD, Scope::BASE);
            title_x = c.x + w + sz.s4;
        }
        let title_w = width_of(kit, tstyle, &page.title).min(c.right() - title_x);
        words.push(Text::new(kit, Frame::new(title_x, c.y, title_w, tline), z + 1, tstyle, &page.title)?);
        // the chip first, so the subtitle knows how much room it has
        let mut chip_layers = Vec::new();
        let mut chip_text = None;
        let mut head_h = tline;
        let mut chip_w = 0.0;
        let mut below = false;
        if let Some((text, tone)) = &page.chip {
            let cs = role_style(kit, TypeRole::Caption, None, paint.ink, Align::Left);
            let tw = width_of(kit, cs, text);
            let dot = if tone_colour(kit, tone).is_some() { sz.s2 + sz.s2 } else { 0.0 };
            chip_w = (tw + dot + 2.0 * sz.s4).ceil();
            let chip_h = (kit.line(TypeRole::Caption) + 2.0 * sz.s2).ceil();
            below = title_x + title_w + sz.s4 + chip_w > c.right();
            let (cx, cy) = if below { (title_x, c.y + tline + sz.s2) } else { (c.right() - chip_w, c.y + ((tline - chip_h) / 2.0).round()) };
            chip_layers.push(kit.ui.scene.shape(chip_w as u32, chip_h as u32, z, pill(paint.wash_selected, paint.edge, (chip_h / 2.0).floor(), 1.0), Props::at(cx, cy)));
            if let Some(col) = tone_colour(kit, tone) {
                chip_layers.push(kit.ui.scene.shape(sz.s2 as u32, sz.s2 as u32, z + 1, pill(col, Rgba(0, 0, 0, 0), sz.s2 / 2.0, 0.0), Props::at(cx + sz.s4, cy + ((chip_h - sz.s2) / 2.0).round())));
            }
            let cline = kit.line(TypeRole::Caption);
            chip_text = Some(Text::new(kit, Frame::new(cx + sz.s4 + dot, cy + ((chip_h - cline) / 2.0).round(), tw, cline), z + 2, cs, text)?);
            if below {
                head_h = tline + sz.s2 + chip_h;
            }
        }
        // the subtitle after the title, its baseline on the title's
        if !page.subtitle.is_empty() {
            let ls = role_style(kit, TypeRole::Label, None, paint.ink2, Align::Left);
            let room = c.right() - (title_x + title_w + sz.s4) - if chip_w > 0.0 && !below { chip_w + sz.s4 } else { 0.0 };
            if room > 80.0 {
                let ty = kit.ty(TypeRole::Label);
                let text = ellipsise(&mut kit.ui.fonts, &page.subtitle, ty, room);
                let (tt, lt) = (kit.ty(TypeRole::Title), kit.ty(TypeRole::Label));
                let (te, le) = (kit.ui.fonts.measure(&page.title, tt), kit.ui.fonts.measure(&text, lt));
                let tbase = (tline - te.height()) / 2.0 + te.ascent;
                let lline = line_height(lt);
                let lbase = (lline - le.height()) / 2.0 + le.ascent;
                let y = (c.y + tbase - lbase).round();
                words.push(Text::new(kit, Frame::new(title_x + title_w + sz.s4, y, room, lline), z + 1, ls, &text)?);
            }
        }
        let _ = faded;
        Ok(Head { sig, _pill: pill_layer, _icon: icon_layer, _words: words, _chip: chip_layers, _chip_text: chip_text, height: head_h.ceil() })
    }
}

/// The tab bar: a tray of segments, the chosen one on a plate with the accent's mark under its label, the focused one on the focus
/// plate, the pointer's cue; every tab is a focusable of its own for the pointer.
pub struct Tabs {
    pub sig: String,
    geom: SegGeom,
    at: (f32, f32),
    n: usize,
    _tray: ShapeLayer,
    chosen: ShapeLayer,
    mark: ShapeLayer,
    focus: ShapeLayer,
    hover: ShapeLayer,
    labels: TextLayer,
    names: Vec<String>,
    state: (usize, bool, Option<usize>, usize),
}

impl Tabs {
    #[allow(clippy::too_many_arguments)]
    pub fn build(kit: &mut Kit, engine: &mut Engine, paint: &Paint, sz: &Sizes, geo: &Geometry, page: &PageView, sig: String, y: f32, z: i32) -> std::io::Result<Tabs> {
        let width = geo.content.w;
        let mut g = seg_geom(kit, sz, &page.tabs, TypeRole::Label, Some(width));
        let share = (width / page.tabs.len().max(1) as f32).floor();
        g.seg_w = g.seg_w.max(share);
        g.in_view = if page.tabs.len() as f32 * g.seg_w > width { ((width / g.seg_w).floor() as usize).max(1) } else { page.tabs.len() };
        let at = (geo.content.x, y);
        let radius = kit.tokens.radius(Radius::Control);
        let total = g.total().min(width);
        let tray = kit.ui.scene.shape(total as u32, g.h as u32, z, pill(paint.tray(), Rgba(0, 0, 0, 0), radius, 0.0), Props::at(at.0, at.1));
        let (sw, sh) = (g.seg_w - 2.0 * sz.s1, g.h - 2.0 * sz.s1);
        let r2 = radius - sz.s1;
        let chosen = kit.ui.scene.shape(sw as u32, sh as u32, z + 1, pill(paint.seg_selected(), Rgba(0, 0, 0, 0), r2, 0.0), HIDDEN);
        let mark = kit.ui.scene.shape(sz.s7 as u32, sz.s1 as u32, z + 3, pill(paint.accent, Rgba(0, 0, 0, 0), sz.s1 / 2.0, 0.0), HIDDEN);
        let focus = kit.ui.scene.shape(sw as u32, sh as u32, z + 1, pill(paint.seg_focus(), paint.edge_raised, r2, 1.0), HIDDEN);
        let hover = kit.ui.scene.shape(sw as u32, sh as u32, z + 1, pill(paint.wash_hover, faded(paint.edge_raised, 0.6), r2, 1.0), HIDDEN);
        let ty = kit.ty(TypeRole::Label);
        let top = ((g.h - line_height(ty)) / 2.0).round();
        let items: Vec<Item> = page.tabs.iter().enumerate().take(g.in_view).map(|(i, t)| Item::Text(Run { text: t, ty, colour: paint.ink, x: i as f32 * g.seg_w, y: top, w: g.seg_w, align: Align::Centre })).collect();
        let labels = TextLayer::new(&mut kit.ui, Frame::new(at.0, at.1, total, g.h), z + 2, &items)?;
        for i in 0..g.in_view {
            engine.focus.register(crate::kit::focus::Id(TAB_ID + i as u32), Frame::new(at.0 + i as f32 * g.seg_w, at.1, g.seg_w, g.h), REGION_TABS, Scope::BASE);
        }
        Ok(Tabs { sig, geom: g, at, n: page.tabs.len(), _tray: tray, chosen, mark, focus, hover, labels, names: page.tabs.clone(), state: (usize::MAX, false, None, 0) })
    }

    /// The chosen tab, whether the bar has the focus (then the chosen tab is the focused one), the tab the pointer's cue is on.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, kit: &mut Kit, engine: &mut Engine, paint: &Paint, sz: &Sizes, tab: usize, focused: bool, hovered: Option<usize>) {
        let g = self.geom;
        let anchor = tab.min(self.n.saturating_sub(1));
        let mut first = self.state.3;
        if anchor < first {
            first = anchor;
        } else if anchor >= first + g.in_view {
            first = anchor + 1 - g.in_view;
        }
        if self.state == (tab, focused, hovered, first) {
            return;
        }
        let place = |i: usize| Props::at(self.at.0 + (i - first) as f32 * g.seg_w + sz.s1, self.at.1 + sz.s1);
        let in_view = |i: usize| i >= first && i < first + g.in_view;
        let glide = |layer: &ui::scene::Layer, to: Props| kit.glide(layer, to, Beat::Label);
        if in_view(anchor) {
            glide(&self.chosen, place(anchor));
            let p = place(anchor);
            glide(&self.mark, Props::at(p.x + ((g.seg_w - 2.0 * sz.s1 - sz.s7) / 2.0).round(), self.at.1 + g.h - sz.s1 - sz.s2));
        } else {
            self.chosen.set(HIDDEN);
            self.mark.set(HIDDEN);
        }
        if focused && in_view(anchor) {
            glide(&self.focus, place(anchor));
        } else {
            self.focus.set(HIDDEN);
        }
        match hovered.filter(|h| in_view(*h) && !(focused && *h == anchor)) {
            Some(h) => self.hover.set(place(h)),
            None => self.hover.set(HIDDEN),
        }
        if first != self.state.3 {
            let ty = kit.ty(TypeRole::Label);
            let top = ((g.h - line_height(ty)) / 2.0).round();
            let items: Vec<Item> = self.names.iter().enumerate().skip(first).take(g.in_view).map(|(i, t)| Item::Text(Run { text: t, ty, colour: paint.ink, x: (i - first) as f32 * g.seg_w, y: top, w: g.seg_w, align: Align::Centre })).collect();
            self.labels.draw(&mut kit.ui, &items);
            for i in 0..16 {
                engine.focus.unregister(crate::kit::focus::Id(TAB_ID + i));
            }
            for i in 0..g.in_view {
                engine.focus.register(crate::kit::focus::Id(TAB_ID + (first + i) as u32), Frame::new(self.at.0 + i as f32 * g.seg_w, self.at.1, g.seg_w, g.h), REGION_TABS, Scope::BASE);
            }
        }
        self.state = (tab, focused, hovered, first);
    }
}
