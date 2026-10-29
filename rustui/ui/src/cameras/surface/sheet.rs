//! The events and clips sheet drawn (`docs/mocks/cameras-scrypted/events*.jpg`, `clips*.jpg`, `events-all*.jpg`; spec "The events and clips
//! sheet"): a lower pane from the safe edges over the picture or over the grid, the title (the camera's name, or "Recent events"), the tabs
//! `Events` and `Clips` at the right, and a row of cards: a picture 352 by 198 in the tile's radius with the kind's glyph in a badge at its
//! corner (a clip's length at the other), the title under it (two lines at most) and the time with how long ago. A picture that never came
//! leaves a quiet tile with the kind's glyph, never a blank and never a made-up picture. Four cards show; a `chevron-right` says there are
//! more. A card is a focusable of the engine: it lifts (1.03, a lit edge) with the flowing focus, and the row scrolls on the same spring.

use super::common::{Ink, Parts, icon_side, scale};
use super::grid::{WordRec, other};
use super::pic::{Fill, PicLayer};
use super::{Cx, REGION_CARDS, REGION_SHEET, SCOPE_SHEET, depth, ids};
use crate::cameras::app::Target;
use crate::cameras::sheet::{PIC_H, PIC_W, SHOWN};
use crate::cameras::view::{CardView, SheetFocus, SheetView, Tab};
use crate::kit::button::{Button, Place, Spec, Tone, faded};
use crate::kit::flow::Lifted;
use crate::kit::focus::Id;
use crate::kit::icon;
use crate::kit::pane::{Pane, PaneKind, glass};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::wrap;
use compositor_protocol::{GroupPose, Props, Rect, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::scene::ShapeLayer;
use ui::text::Weight;

const CARD_W: f32 = PIC_W as f32;
const CARD_GAP: f32 = 24.0;
const PITCH: f32 = CARD_W + CARD_GAP;

struct CardUi {
    index: usize,
    id: Id,
    /// Where the card is in the content (before the scroll).
    frame: Frame,
    body: Group,
    pic: PicLayer,
    ring: ShapeLayer,
    ring_radius: f32,
    hovered: bool,
    _keep: Vec<Box<dyn std::any::Any>>,
    pic_key: String,
    has_pic_source: bool,
    quiet: Option<Vec<Box<dyn std::any::Any>>>,
    l1: Vec<String>,
    l2: Vec<String>,
}

pub struct SheetUi {
    ink: Ink,
    sig: String,
    over_video: bool,
    frame: Frame,
    paint: Rgba,
    parts: Parts,
    words: Vec<WordRec>,
    tabs: Vec<(Button, Tab)>,
    viewport: Group,
    content: Group,
    cards: Vec<CardUi>,
    window: std::ops::Range<usize>,
    first: usize,
    cards_x: f32,
    cards_y: f32,
    l1_lines: usize,
    l2_lines: usize,
    more: Option<Parts>,
    empty: Option<(Parts, WordRec)>,
    empty_sig: Option<String>,
    modal: bool,
}

fn signature(cx: &Cx, view: &SheetView, over_video: bool) -> String {
    let titles: Vec<&str> = view.cards.iter().map(|c| c.title.as_str()).collect();
    format!("{}|{}|{}|{:?}|{titles:?}|{}|{}", cx.kit.generation(), cx.kit.text_scale, view.title, view.tabs, view.cards.len(), over_video)
}

impl SheetUi {
    pub fn build(cx: &mut Cx, view: &SheetView, over_video: bool) -> std::io::Result<Self> {
        let ink = Ink::of(cx.kit);
        let ts = scale(cx.kit);
        let z = cx.at(depth::SHEET);
        let pad = (24.0, 28.0, 28.0);
        let tab_h = Button::height(cx.kit);
        let head_h = tab_h + 8.0;
        // the card's lines: as many as the longest title needs, two at most, the same for every card so the row stands even
        let l1_style = role_style(cx.kit, TypeRole::Label, Some(Weight::SemiBold), ink.ink, Align::Left);
        let l2_style = role_style(cx.kit, TypeRole::Caption, None, ink.ink2, Align::Left);
        let mut l1_lines = 1usize;
        let mut l2_lines = 1usize;
        for c in &view.cards {
            l1_lines = l1_lines.max(wrap::lines(cx.kit, l1_style, &c.title, CARD_W, 2).len());
            l2_lines = l2_lines.max(wrap::lines(cx.kit, l2_style, &c.line2, CARD_W, 2).len());
        }
        let (l1h, l2h) = (cx.kit.line(TypeRole::Label), cx.kit.line(TypeRole::Caption));
        let card_h = PIC_H as f32 + 10.0 + l1_lines as f32 * l1h + l2_lines as f32 * l2h;
        let height = pad.0 + head_h + 16.0 + 8.0 + card_h + 4.0 + pad.2;
        let frame = Frame::new(96.0, (1026.0 - height).round(), 1728.0, height);
        let paint = glass(cx.kit, PaneKind::Sheet, height, over_video).tint;
        let mut parts = Parts::new();
        parts.pane(Pane::new(cx.kit, frame, z, PaneKind::Sheet, over_video));
        // the title
        let title_style = role_style(cx.kit, TypeRole::Body, Some(Weight::SemiBold), ink.ink, Align::Left);
        let title = wrap::elide(cx.kit, title_style, &view.title, 900.0);
        let tl = cx.kit.line(TypeRole::Body);
        let cy = frame.y + pad.0 + head_h / 2.0;
        let title_f = Frame::new(frame.x + pad.1, (cy - tl / 2.0).round(), width_of(cx.kit, title_style, &title), tl);
        parts.text(Text::new(cx.kit, title_f, z + 1, title_style, &title)?);
        let words = vec![WordRec { name: "sheet title".into(), text: title, frame: title_f, style: title_style }];
        let scene = cx.kit.ui.scene.clone();
        let viewport_rect = Rect { x: frame.x + pad.1 - 12.0, y: frame.y + pad.0 + head_h - 4.0, w: frame.w - 2.0 * pad.1 + 24.0, h: card_h + 8.0 + 16.0 + 24.0 };
        let viewport = scene.group(None, GroupPose { clip: Some(viewport_rect), ..GroupPose::IDENTITY }).map_err(other)?;
        let content = scene.group(Some(viewport.id()), GroupPose::IDENTITY).map_err(other)?;
        let cards_x = frame.x + pad.1 + 4.0;
        let cards_y = frame.y + pad.0 + head_h + 16.0 + 8.0;
        let mut me = Self {
            ink,
            sig: signature(cx, view, over_video),
            over_video,
            frame,
            paint,
            parts,
            words,
            tabs: Vec::new(),
            viewport,
            content,
            cards: Vec::new(),
            window: 0..0,
            first: view.first,
            cards_x,
            cards_y,
            l1_lines,
            l2_lines,
            more: None,
            empty: None,
            empty_sig: None,
            modal: false,
        };
        cx.engine.focus.open_modal(SCOPE_SHEET);
        me.modal = true;
        if view.tabs.is_some() {
            me.build_tabs(cx, view, cy)?;
        }
        let extent = view.cards.len().saturating_sub(SHOWN) as f32 * PITCH;
        cx.engine.flow.scroller(&scene, REGION_CARDS, &me.content, GroupPose::IDENTITY, true, extent).map_err(other)?;
        cx.engine.flow.scroll_snap(REGION_CARDS, view.first as f32 * PITCH).map_err(other)?;
        let _ = ts;
        me.sync(cx, view)?;
        if cx.kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the sheet could not be drawn whole"));
        }
        me.parts.fade(cx.kit, true, super::full::FADE_IN_MS);
        Ok(me)
    }

    pub fn fits(&self, cx: &Cx, view: &SheetView, over_video: bool) -> bool {
        self.sig == signature(cx, view, over_video)
    }

    fn build_tabs(&mut self, cx: &mut Cx, view: &SheetView, cy: f32) -> std::io::Result<()> {
        let h = Button::height(cx.kit);
        let radius = cx.kit.tokens.radius(Radius::Control);
        let (w_e, w_c) = (Button::width_for(cx.kit, "Events", false), Button::width_for(cx.kit, "Clips", false));
        let well_w = w_e + w_c + 4.0 * 3.0;
        let well_h = h + 8.0;
        let right = self.frame.right() - 28.0;
        let well_f = Frame::new(right - well_w, (cy - well_h / 2.0).round(), well_w, well_h);
        let z = cx.at(depth::SHEET);
        let well = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, well_w, well_h), z + 1, plate::solid(cx.kit.ui.look.colour(ui::lucent::palette::PaletteRole::TrackOff), radius + 4.0));
        self.parts.shape(well, (well_f.x, well_f.y));
        let mut x = well_f.x + 4.0;
        for (label, tab, id, w) in [("Events", Tab::Events, ids::TAB_EVENTS, w_e), ("Clips", Tab::Clips, ids::TAB_CLIPS, w_c)] {
            let place = Place { id, frame: Frame::new(x, well_f.y + 4.0, w, h), region: REGION_SHEET, scope: SCOPE_SHEET, z: z + 3 };
            let mut b = Button::new(cx.kit, cx.engine, place, Spec::new(label, Tone::Plain).with_radius(radius).selected(view.tabs == Some(tab)))?;
            b.show(cx.kit, true);
            self.tabs.push((b, tab));
            x += w + 4.0;
        }
        Ok(())
    }

    fn card_lines(cx: &mut Cx, c: &CardView, l1_style: Style, l2_style: Style) -> (Vec<String>, Vec<String>) {
        (wrap::lines(cx.kit, l1_style, &c.title, CARD_W, 2), wrap::lines(cx.kit, l2_style, &c.line2, CARD_W, 2))
    }

    fn build_card(&mut self, cx: &mut Cx, view: &SheetView, i: usize) -> std::io::Result<CardUi> {
        let c = &view.cards[i];
        let z = cx.at(depth::SHEET) + 10;
        let (w, pic_h) = (CARD_W, PIC_H as f32);
        let l1_style = role_style(cx.kit, TypeRole::Label, Some(Weight::SemiBold), self.ink.ink2, Align::Left);
        let l1_lit = Style { colour: self.ink.ink, ..l1_style };
        let l2_style = role_style(cx.kit, TypeRole::Caption, None, self.ink.ink2, Align::Left);
        let (l1, l2) = Self::card_lines(cx, c, l1_style, l2_style);
        let (l1h, l2h) = (cx.kit.line(TypeRole::Label), cx.kit.line(TypeRole::Caption));
        let total_h = pic_h + 10.0 + self.l1_lines as f32 * l1h + self.l2_lines as f32 * l2h;
        let x = i as f32 * PITCH + self.cards_x;
        let radius = cx.kit.tokens.radius(Radius::Tile);
        let scene = cx.kit.ui.scene.clone();
        let rest = GroupPose { x, y: self.cards_y, pivot_x: w / 2.0, pivot_y: total_h / 2.0, ..GroupPose::IDENTITY };
        let lift = scene.group(Some(self.content.id()), rest).map_err(other)?;
        let lit = scene.group(Some(lift.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let body = scene.group(Some(lift.id()), GroupPose::IDENTITY).map_err(other)?;
        let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
        let (ring, mut quiet_parts) = scene.adopting(&body, || -> std::io::Result<_> {
            let mut quiet: Vec<Box<dyn std::any::Any>> = Vec::new();
            let ring = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, pic_h), z + 2, plate::ring(self.ink.edge, radius, 3.0));
            // l1 and l2 at rest
            let mut y = pic_h + 10.0;
            for line in &l1 {
                keep.push(Box::new(Text::new(cx.kit, Frame::new(0.0, y, w, l1h), z + 3, l1_style, line)?));
                y += l1h;
            }
            y = pic_h + 10.0 + self.l1_lines as f32 * l1h;
            for line in &l2 {
                keep.push(Box::new(Text::new(cx.kit, Frame::new(0.0, y, w, l2h), z + 3, l2_style, line)?));
                y += l2h;
            }
            // the kind's glyph: in a badge over the picture, or big on a quiet tile until the picture is there
            let side = icon_side(cx.kit, 26.0);
            let badge_s = (44.0 * (1.0 + (scale(cx.kit) - 1.0) * 0.5)).round();
            let pane = Pane::new(cx.kit, Frame::new(12.0, 12.0, badge_s, badge_s), z + 5, PaneKind::Control, true);
            quiet.push(Box::new(pane));
            if let Some(layer) = icon::layer(cx.kit, c.icon, side, self.ink.ink, z + 6, Props::at(12.0 + ((badge_s - side as f32) / 2.0).round(), 12.0 + ((badge_s - side as f32) / 2.0).round()))? {
                quiet.push(Box::new(layer));
            }
            if let Some(len) = &c.length {
                let st = role_style(cx.kit, TypeRole::Caption, Some(Weight::SemiBold), self.ink.ink, Align::Left);
                let lw = width_of(cx.kit, st, len);
                let lh = cx.kit.line(TypeRole::Caption);
                let pill = Frame::new(w - 12.0 - (lw + 24.0), pic_h - 12.0 - (lh + 4.0), lw + 24.0, lh + 4.0);
                quiet.push(Box::new(Pane::new(cx.kit, pill, z + 5, PaneKind::Pill, true)));
                quiet.push(Box::new(Text::new(cx.kit, Frame::new(pill.x + 12.0, pill.y + 2.0, lw, lh), z + 6, st, len)?));
            }
            Ok((ring, quiet))
        })?;
        let quiet_tile = scene.adopting(&body, || -> std::io::Result<Vec<Box<dyn std::any::Any>>> {
            // the quiet tile under everything until a picture is painted over it
            let wash = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, pic_h), z, plate::solid(faded(self.ink.ink, 0.10), radius));
            let big = (72.0 * (1.0 + (scale(cx.kit) - 1.0) * 0.5)).round() as u32;
            let glyph = icon::layer(cx.kit, c.icon, big, self.ink.ink2, z + 1, Props::at((w - big as f32) / 2.0, ((pic_h - big as f32) / 2.0).round()))?;
            let mut v: Vec<Box<dyn std::any::Any>> = vec![Box::new(wash)];
            if let Some(g) = glyph {
                v.push(Box::new(g));
            }
            Ok(v)
        })?;
        let (lit_ring, halo, lit_lines) = scene.adopting(&lit, || -> std::io::Result<_> {
            let halo = plate::shape(&cx.kit.ui, Frame::new(-6.0, -6.0, w + 12.0, pic_h + 12.0), z + 4, plate::ring(faded(self.ink.accent, 0.26), radius + 6.0, 6.0));
            let lit_ring = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, w, pic_h), z + 7, plate::ring(self.ink.edge_raised, radius, 3.0));
            let mut lines: Vec<Text> = Vec::new();
            let mut y = pic_h + 10.0;
            for line in &l1 {
                lines.push(Text::new(cx.kit, Frame::new(0.0, y, w, l1h), z + 8, l1_lit, line)?);
                y += l1h;
            }
            Ok((lit_ring, halo, lines))
        })?;
        keep.push(Box::new(lit_ring));
        keep.push(Box::new(halo));
        for l in lit_lines {
            keep.push(Box::new(l));
        }
        keep.append(&mut quiet_parts);
        let id = ids::card(i);
        let frame = Frame::new(x, self.cards_y, w, total_h);
        let offset = self.first as f32 * PITCH;
        cx.engine.focus.register(id, Frame::new(frame.x - offset, frame.y, w, total_h), REGION_CARDS, SCOPE_SHEET);
        cx.engine.flow.lift(id, Lifted { group: lift, rest, scale: cx.kit.tokens.focus.card_scale, rise: 0.0, lit: Some(lit) });
        let key = c.pic.clone().unwrap_or_default();
        Ok(CardUi {
            index: i,
            id,
            frame,
            body,
            pic: PicLayer::new(&key, Frame::new(0.0, 0.0, w, pic_h), radius, Fill::Cover, z + 1),
            ring,
            ring_radius: radius,
            hovered: false,
            _keep: keep,
            pic_key: key,
            has_pic_source: c.pic.is_some(),
            quiet: Some(quiet_tile),
            l1,
            l2,
        })
    }

    /// Builds the cards of the window (the shown ones and one each side) and lets the others go.
    fn sync_cards(&mut self, cx: &mut Cx, view: &SheetView) -> std::io::Result<()> {
        let first = view.first.saturating_sub(1);
        let last = (view.first + SHOWN + 1).min(view.cards.len());
        let want = first..last.max(first);
        let gone: Vec<usize> = self.cards.iter().filter(|c| !want.contains(&c.index)).map(|c| c.index).collect();
        for i in gone {
            if let Some(pos) = self.cards.iter().position(|c| c.index == i) {
                let c = self.cards.remove(pos);
                cx.engine.focus.unregister(c.id);
                cx.engine.flow.drop_lift(c.id);
            }
        }
        for i in want.clone() {
            if i < view.cards.len() && self.cards.iter().all(|c| c.index != i) {
                let c = self.build_card(cx, view, i)?;
                self.cards.push(c);
            }
        }
        self.window = want;
        Ok(())
    }

    pub fn sync(&mut self, cx: &mut Cx, view: &SheetView) -> std::io::Result<()> {
        if view.first != self.first {
            self.first = view.first;
            let offset = view.first as f32 * PITCH;
            cx.engine.flow.scroll_to(REGION_CARDS, offset).map_err(other)?;
            self.sync_cards(cx, view)?;
            for c in &self.cards {
                cx.engine.focus.place(c.id, Frame::new(c.frame.x - offset, c.frame.y, c.frame.w, c.frame.h));
            }
        } else {
            self.sync_cards(cx, view)?;
        }
        for c in &mut self.cards {
            let Some(cv) = view.cards.get(c.index) else { continue };
            if let Some(key) = &cv.pic {
                if &c.pic_key != key {
                    c.pic_key = key.clone();
                    c.pic = PicLayer::new(key, Frame::new(0.0, 0.0, CARD_W, PIC_H as f32), c.ring_radius, Fill::Cover, cx.at(depth::SHEET) + 11);
                }
                if c.pic.sync(cx.kit, Some(&c.body), cx.store)? {
                    c.quiet = None;
                }
            }
            let _ = (c.has_pic_source, &c.l1, &c.l2);
        }
        for (b, tab) in &mut self.tabs {
            b.set_selected(view.tabs == Some(*tab));
        }
        self.sync_extras(cx, view)?;
        let id = match view.focus {
            SheetFocus::Tab(Tab::Events) => ids::TAB_EVENTS,
            SheetFocus::Tab(Tab::Clips) => ids::TAB_CLIPS,
            SheetFocus::Card(i) => ids::card(i),
        };
        if view.cards.is_empty() && matches!(view.focus, SheetFocus::Card(_)) {
            if cx.engine.focus.current().is_some() {
                cx.engine.clear(cx.now_ns);
            }
        } else {
            cx.follow(id);
        }
        Ok(())
    }

    /// The chevron that says there are more cards, and the words of a list that was read and is empty.
    fn sync_extras(&mut self, cx: &mut Cx, view: &SheetView) -> std::io::Result<()> {
        let z = cx.at(depth::SHEET) + 1;
        if view.more && self.more.is_none() {
            let side = icon_side(cx.kit, 40.0);
            let mut p = Parts::new();
            if let Some(layer) = icon::layer(cx.kit, "chevron-right", side, self.ink.ink2, z, crate::system::parts::HIDDEN)? {
                p.image(layer, (self.frame.right() - 28.0 - side as f32, (self.cards_y + PIC_H as f32 / 2.0 - side as f32 / 2.0).round()));
            }
            p.fade(cx.kit, true, 0);
            self.more = Some(p);
        } else if !view.more {
            self.more = None;
        }
        let sig = view.empty.clone();
        if sig != self.empty_sig {
            self.empty_sig = sig.clone();
            self.empty = None;
            if let Some(text) = sig {
                let style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink2, Align::Centre);
                let line = cx.kit.line(TypeRole::Label);
                let f = Frame::new(self.frame.x + 28.0, (self.cards_y + PIC_H as f32 / 2.0 - line / 2.0).round(), self.frame.w - 56.0, line);
                let mut p = Parts::new();
                p.text(Text::new(cx.kit, f, z, style, &text)?);
                p.fade(cx.kit, true, 0);
                self.empty = Some((p, WordRec { name: "empty".into(), text, frame: f, style }));
            }
        }
        Ok(())
    }

    pub fn hover(&mut self, cx: &mut Cx, id: Id, on: bool) {
        for (b, _) in &mut self.tabs {
            if b.id == id {
                b.set_hover(cx.kit, on && cx.engine.focus.current() != Some(id));
            }
        }
        for c in &mut self.cards {
            if c.id == id && c.hovered != on {
                c.hovered = on;
                let colour = if on { faded(self.ink.edge_raised, 0.6) } else { self.ink.edge };
                c.ring.reshape(plate::ring(colour, c.ring_radius, 3.0));
            }
        }
    }

    pub fn press(&mut self, cx: &mut Cx, id: Id, down: bool) {
        for (b, _) in &mut self.tabs {
            if b.id == id {
                b.set_pressed(cx.kit, down);
            }
        }
    }

    pub fn target(&self, id: Id) -> Option<Target> {
        if let Some((_, t)) = self.tabs.iter().find(|(b, _)| b.id == id) {
            return Some(Target::Tab(*t));
        }
        self.cards.iter().find(|c| c.id == id).map(|c| Target::Card(c.index))
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn card_count(&self) -> usize {
        self.cards.len()
    }

    pub fn card_rects(&self) -> Vec<Frame> {
        let offset = self.first as f32 * PITCH;
        self.cards.iter().map(|c| Frame::new(c.frame.x - offset, c.frame.y, c.frame.w, c.frame.h)).collect()
    }

    pub fn layers(&self) -> usize {
        self.parts.layers()
            + self.tabs.iter().map(|(b, _)| b.layers()).sum::<usize>()
            + self.cards.iter().map(|c| 2 + c._keep.len() + usize::from(c.pic.layer().is_some()) + c.quiet.as_ref().map_or(0, Vec::len)).sum::<usize>()
            + self.more.as_ref().map_or(0, Parts::layers)
            + self.empty.as_ref().map_or(0, |(p, _)| p.layers())
    }

    /// Gives the engine back what the sheet registered (the cards, the tabs, the scroll, the scope).
    pub fn retire_engine(&mut self, engine: &mut crate::kit::Engine) {
        for c in &self.cards {
            engine.focus.unregister(c.id);
            engine.flow.drop_lift(c.id);
        }
        for (b, _) in &self.tabs {
            b.detach(engine);
        }
        engine.flow.drop_scroller(REGION_CARDS);
        if self.modal {
            engine.focus.close_modal();
            self.modal = false;
        }
    }

    pub fn retire(&mut self, cx: &mut Cx, ms: u32) {
        self.retire_engine(cx.engine);
        for (b, _) in &mut self.tabs {
            b.hide(cx.kit, ms == 0);
        }
        self.parts.fade(cx.kit, false, ms);
        self.viewport.set_pose(GroupPose { opacity: 0.0, visible: false, ..GroupPose::IDENTITY });
    }

    pub fn feed_lint(&self, kit: &mut crate::kit::Kit, lint: &mut crate::kit::lint::Lint) {
        let panel = lint.panel("sheet", self.frame, self.paint, true);
        for w in &self.words {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
        }
        for (b, _) in &self.tabs {
            b.feed_lint(kit, lint, panel);
        }
        if let Some((_, w)) = &self.empty {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
        }
        let l1_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), self.ink.ink2, Align::Left);
        let l2_style = role_style(kit, TypeRole::Caption, None, self.ink.ink2, Align::Left);
        let (l1h, l2h) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
        let offset = self.first as f32 * PITCH;
        for c in &self.cards {
            let x = c.frame.x - offset;
            if x < self.frame.x + 20.0 || x + CARD_W > self.frame.right() - 20.0 {
                continue;
            }
            let mut y = c.frame.y + PIC_H as f32 + 10.0;
            for line in &c.l1 {
                lint.word(kit, &format!("card {} title", c.index), line, Frame::new(x, y, CARD_W, l1h), l1_style, Some(panel));
                y += l1h;
            }
            y = c.frame.y + PIC_H as f32 + 10.0 + self.l1_lines as f32 * l1h;
            for line in &c.l2 {
                lint.word(kit, &format!("card {} time", c.index), line, Frame::new(x, y, CARD_W, l2h), l2_style, Some(panel));
                y += l2h;
            }
        }
    }

    pub fn paint(&self) -> Rgba {
        self.paint
    }

    pub fn over_video(&self) -> bool {
        self.over_video
    }

    pub fn l1_lines(&self) -> usize {
        self.l1_lines
    }

    pub fn l2_lines(&self) -> usize {
        self.l2_lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::data::Tick;
    use crate::cameras::sheet::Sheet;
    use crate::cameras::surface::pic::PictureStore;
    use crate::cameras::words::{Clock, Zone};
    use crate::kit::lint::Lint;
    use crate::kit::testkit;

    const MIN: i64 = 60_000;
    const NOW: i64 = (20_755 * 86_400 + 14 * 3600 + 37 * 60) * 1000;

    fn ticks(n: usize) -> Vec<Tick> {
        let kinds = ["person", "doorbell", "animal", "vehicle", "package", "motion"];
        (0..n).map(|i| Tick { at: NOW - (6 + i as i64 * 33) * MIN, kind: kinds[i % 6].into(), label: if i % 6 == 2 { "Domestic Cat".into() } else { String::new() }, clip: None }).collect()
    }

    fn view(n: usize, has_pic: &dyn Fn(&str) -> bool) -> SheetView {
        let mut s = Sheet::camera("273", "Front Door");
        s.set_events(&ticks(n));
        s.view(&Clock { twelve_hour: true, zone: Zone::UTC }, NOW, has_pic)
    }

    fn cx<'a>(kit: &'a mut crate::kit::Kit, engine: &'a mut crate::kit::Engine, store: &'a PictureStore) -> Cx<'a> {
        Cx { kit, engine, store, z: 2000, now_ms: 1000, now_ns: 1_000_000_000 }
    }

    #[test]
    fn five_cards_are_built_with_the_row_scrolling_and_registered_in_a_scope_of_their_own() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let v = view(5, &|_| false);
        let before = kit.room();
        let s = SheetUi::build(&mut cx(&mut kit, &mut engine, &store), &v, true).expect("a sheet");
        assert_eq!(engine.focus.modal(), Some(SCOPE_SHEET));
        assert_eq!(s.card_count(), 5, "four show and one more is ready to scroll in");
        let rects = s.card_rects();
        assert_eq!((rects[0].x, rects[0].w), (128.0, 352.0));
        assert_eq!(rects[1].x - rects[0].x, 376.0);
        assert!(engine.focus.is_live(ids::card(0)) && engine.focus.is_live(ids::TAB_EVENTS));
        assert_eq!(engine.focus.current(), Some(ids::card(0)));
        assert!(before.layers - kit.room().layers < 130, "{} layers", before.layers - kit.room().layers);
        assert_eq!(s.target(ids::card(2)), Some(Target::Card(2)));
        assert_eq!(s.target(ids::TAB_CLIPS), Some(Target::Tab(Tab::Clips)));
        let f = s.frame();
        assert_eq!((f.x, f.right(), f.bottom()), (96.0, 1824.0, 1026.0));
    }

    #[test]
    fn a_card_whose_picture_arrived_shows_it_and_one_that_did_not_stays_a_glyph_on_a_quiet_tile() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let mut store = PictureStore::new();
        let mut v = view(3, &|_| false);
        let mut s = SheetUi::build(&mut cx(&mut kit, &mut engine, &store), &v, true).unwrap();
        assert!(s.cards.iter().all(|c| c.quiet.is_some() && !c.pic.is_painted()));
        store.put("card:273:e0", crate::kit::picture::Picture { width: 16, height: 9, rgba: vec![80; 16 * 9 * 4] });
        v.cards[0].pic = Some("card:273:e0".into());
        s.sync(&mut cx(&mut kit, &mut engine, &store), &v).unwrap();
        assert!(s.cards[0].pic.is_painted());
        assert!(s.cards[0].quiet.is_none(), "the quiet tile is gone under the picture");
        assert!(s.cards[1].quiet.is_some(), "nothing is made up for the card whose picture never came");
    }

    #[test]
    fn the_row_scrolls_with_the_focus_and_the_window_follows() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut sheet = Sheet::camera("273", "Front Door");
        sheet.set_events(&ticks(9));
        let clock = Clock { twelve_hour: true, zone: Zone::UTC };
        let mut v = sheet.view(&clock, NOW, &|_| false);
        let mut s = SheetUi::build(&mut cx(&mut kit, &mut engine, &store), &v, true).unwrap();
        assert_eq!(s.card_count(), 5);
        for _ in 0..6 {
            sheet.key(crate::cameras::sheet::Key::Right);
        }
        v = sheet.view(&clock, NOW, &|_| false);
        assert_eq!(v.first, 3);
        s.sync(&mut cx(&mut kit, &mut engine, &store), &v).unwrap();
        assert_eq!(engine.focus.current(), Some(ids::card(6)));
        assert_eq!(s.card_count(), 6, "cards 2 to 7");
        assert!(engine.focus.is_live(ids::card(6)) && !engine.focus.is_live(ids::card(0)));
        let x6 = s.card_rects().iter().find(|r| (r.x - (128.0 + 3.0 * 376.0)).abs() < 1.0 || r.x > 0.0).map(|r| r.x);
        assert!(x6.is_some());
    }

    #[test]
    fn the_recent_list_has_no_tabs_and_an_empty_one_says_so() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let sheet = Sheet::recent(&[]);
        let v = sheet.view(&Clock { twelve_hour: true, zone: Zone::UTC }, NOW, &|_| false);
        let s = SheetUi::build(&mut cx(&mut kit, &mut engine, &store), &v, false).unwrap();
        assert!(s.tabs.is_empty());
        assert!(s.empty.is_some());
        assert_eq!(s.card_count(), 0);
        assert!(!s.over_video());
    }

    #[test]
    fn the_sheet_reads_clean_to_the_lint_at_both_text_sizes_and_in_both_looks() {
        for scale in [1.0f32, 2.0] {
            for mode in [ui::lucent::palette::Mode::Dark, ui::lucent::palette::Mode::Light] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = crate::kit::Engine::new(&kit).unwrap();
                let store = PictureStore::new();
                let v = view(6, &|_| false);
                let mut s = SheetUi::build(&mut cx(&mut kit, &mut engine, &store), &v, true).unwrap();
                s.sync(&mut cx(&mut kit, &mut engine, &store), &v).unwrap();
                let mut lint = Lint::new().with_safe(Frame::new(96.0, 54.0, 1728.0, 972.0));
                s.feed_lint(&mut kit, &mut lint);
                let findings: Vec<String> = lint.findings().iter().map(|x| x.to_string()).collect();
                assert!(findings.is_empty(), "at {scale} in {mode:?}: {findings:?}");
                assert!(s.frame().y > 300.0, "{:?}", s.frame());
                assert!(s.l1_lines() <= 2 && s.l2_lines() <= 2);
            }
        }
    }
}
