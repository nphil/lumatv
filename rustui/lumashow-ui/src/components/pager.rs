//! A pager: pages side by side that a horizontal swipe turns. The pager owns what makes it a pager and nothing of what a page shows:
//! the clip that cuts the pages to the viewport, the snapping, the neighbours that peek in and fade by distance, and the dots (and
//! optionally the previous/next buttons) that say where you are. It tells its owner which page is showing and which pages are close
//! enough to be worth preparing; it never builds a page.
//!
//! A page is a [`Group`] the owner built, with its content resting where the page shows when it is the current one. The pager
//! poses each page group: displaced by its distance from the current page, faded by that distance, cut to the viewport, and hidden
//! when none of it can be seen. While a finger drags, the pages follow it one for one with a `Set` on the layers of the pages that
//! can be seen and nothing else (no layer is made, no text drawn); when the finger lifts the pages settle with one `Animate` per
//! layer, by distance and by speed ([`super::swipe`]).
//!
//! **Whole pages are inert unless they are the current one and still:** a page that is moving, or not the current page, is absent
//! from hit testing ([`Pager::hit_page`], [`Pager::sync_focus`]) and from semantics. There is no whole-home horizontal pager in the
//! shell; this is for a settings section, a card carousel, a gallery.

use super::button::{Button, ButtonAction, ButtonModel};
use super::group::{Group, Part, Pose};
use super::icon::Icon;
use super::plate::{shape, solid};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::swipe::{self, Contact, PagerDrag, SLOP, Sample};
use super::{Component, Damage};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::focus::{Focus, Scope};
use crate::input::{Event, TouchPhase};
use crate::lucent::material::MaterialContext;
use crate::lucent::tokens::{motion, space};
use compositor_protocol::{Ease, Rect};
use std::io;

/// A page one stride away shows this opaque; the page in the middle is fully so.
const NEIGHBOUR_OPACITY: f32 = 0.4;
/// An indicator dot is this wide and high; the selected one is drawn [`SELECTED_DOT`] times larger.
const DOT: f32 = 8.0;
const SELECTED_DOT: f32 = 1.5;
/// How far apart the dots' centres are.
const DOT_PITCH: f32 = 20.0;
/// The visual size of a previous/next button (its touch target is larger, [`super::target_around`]).
const ARROW: f32 = 48.0;

/// Which page shows, out of how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagerModel {
    pub pages: usize,
    pub page: usize,
}

/// Where a pager's pages and its indicators go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagerLayout {
    /// The window the pages are cut to.
    pub viewport: Frame,
    /// How wide a page is. Narrower than the viewport, the neighbours peek in at both sides.
    pub page_width: f32,
    /// The space between pages.
    pub gap: f32,
    /// The vertical middle of the row of dots.
    pub indicator_y: f32,
    /// Whether previous and next buttons stand on either side of the dots: the explicit way to turn a page, for a screen too small
    /// to swipe comfortably or for a person who cannot.
    pub arrows: bool,
}

impl PagerLayout {
    /// Pages as wide as the viewport, so that only the current one is seen at rest, with dots at `indicator_y`.
    pub fn full(viewport: Frame, indicator_y: f32) -> Self {
        Self {
            viewport,
            page_width: viewport.w,
            gap: 0.0,
            indicator_y,
            arrows: false,
        }
    }

    /// The distance from one page to the next.
    pub fn stride(&self) -> f32 {
        self.page_width + self.gap
    }

    /// Where the current page rests: centred in the viewport.
    pub fn slot(&self) -> Frame {
        Frame::new(
            self.viewport.x + (self.viewport.w - self.page_width) / 2.0,
            self.viewport.y,
            self.page_width,
            self.viewport.h,
        )
    }
}

/// What a pager tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagerAction {
    /// A swipe or a button turned to this page.
    PageChanged(usize),
    /// A page came within a swipe of showing (visible) or went further than that (not visible): the owner prepares its content when
    /// it becomes visible and may drop it when it does not.
    Visibility { page: usize, visible: bool },
}

/// A finger that went down in the viewport.
struct Drag {
    id: u32,
    contact: Contact,
    /// Where the finger was when the swipe was taken: the pages follow it from there. `None` until the finger has left the slop
    /// sideways.
    anchor_x: Option<f32>,
    /// The finger left the slop up or down: this is a scroll for something else.
    other_way: bool,
}

pub struct Pager {
    layout: PagerLayout,
    model: PagerModel,
    pages: Vec<Group>,
    /// The pose each page group was last given (and, for an animation, will end at), so that nothing is sent twice.
    posed: Vec<Option<Pose>>,
    /// The strip's offset: `-page * stride` at rest, following the finger while it is down.
    offset: f32,
    dots: Group,
    prev: Option<Button>,
    next: Option<Button>,
    drag: Option<Drag>,
    pending: Vec<PagerAction>,
    /// The latest time any call was told, in nanoseconds.
    now_ns: u64,
    /// The pages are on their way until then.
    settling_until_ns: u64,
}

impl Pager {
    /// A pager over `pages`, which has to be exactly `model.pages` groups, showing `model.page`. The pages are posed at once. `z` is the
    /// layer order of the indicators; the owner's page content is its own.
    pub fn new(
        ui: &mut Ui,
        layout: PagerLayout,
        z: i32,
        model: PagerModel,
        pages: Vec<Group>,
    ) -> io::Result<Self> {
        if pages.is_empty() || pages.len() != model.pages || model.page >= model.pages {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a pager needs one group for each of its pages and a page that exists",
            ));
        }
        let mut dots = Group::new((layout.viewport.centre().0, layout.indicator_y));
        if model.pages > 1 {
            let first = layout.viewport.centre().0 - (model.pages - 1) as f32 * DOT_PITCH / 2.0;
            for i in 0..model.pages {
                let frame = Frame::new(
                    first + i as f32 * DOT_PITCH - DOT / 2.0,
                    layout.indicator_y - DOT / 2.0,
                    DOT,
                    DOT,
                );
                let selected = i == model.page;
                let layer = shape(ui, frame, z, solid(dot_colour(ui, selected), DOT / 2.0));
                let mut part = Part::new(layer, frame);
                part.rest_scale = if selected { SELECTED_DOT } else { 1.0 };
                dots.push(part);
            }
        }
        let (prev, next) = if layout.arrows && model.pages > 1 {
            let half = (model.pages - 1) as f32 * DOT_PITCH / 2.0 + DOT * SELECTED_DOT / 2.0;
            let cx = layout.viewport.centre().0;
            let y = layout.indicator_y - ARROW / 2.0;
            let reach = half + space::S4;
            let prev = Button::icon_button(
                ui,
                Frame::new(cx - reach - ARROW, y, ARROW, ARROW),
                z,
                MaterialContext::Independent,
                &arrow_model(Icon::Back, "Previous page", model.page > 0),
            )?;
            let next = Button::icon_button(
                ui,
                Frame::new(cx + reach, y, ARROW, ARROW),
                z,
                MaterialContext::Independent,
                &arrow_model(
                    Icon::ChevronRight,
                    "Next page",
                    model.page + 1 < model.pages,
                ),
            )?;
            (Some(prev), Some(next))
        } else {
            (None, None)
        };
        let count = pages.len();
        let mut pager = Self {
            layout,
            model,
            pages,
            posed: vec![None; count],
            offset: -(model.page as f32) * layout.stride(),
            dots,
            prev,
            next,
            drag: None,
            pending: Vec::new(),
            now_ns: 0,
            settling_until_ns: 0,
        };
        let offset = pager.offset;
        pager.pose_all(offset);
        Ok(pager)
    }

    pub fn layout(&self) -> PagerLayout {
        self.layout
    }

    pub fn model(&self) -> PagerModel {
        self.model
    }

    /// The page that shows, or is turning towards showing.
    pub fn page(&self) -> usize {
        self.model.page
    }

    /// The pages worth having prepared: the current one and the ones a single swipe could bring in.
    pub fn visible_pages(&self) -> Vec<usize> {
        prepared(self.model.page, self.model.pages)
    }

    /// Whether a swipe has been taken by the pager: a screen that gave its own children the same contact cancels their presses.
    pub fn is_dragging(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.anchor_x.is_some())
    }

    /// Tells the pager the time, so that it knows when the pages have arrived.
    pub fn tick(&mut self, now_ns: u64) {
        self.now_ns = self.now_ns.max(now_ns);
    }

    /// Whether the pages have stopped: no finger has them and no settle is under way.
    pub fn is_still(&self) -> bool {
        !self.is_dragging() && self.now_ns >= self.settling_until_ns
    }

    /// Whether `page` takes a touch now: it is the current page and nothing is moving. Every other page is inert.
    pub fn is_live(&self, page: usize) -> bool {
        page == self.model.page && self.is_still()
    }

    /// The page at (`x`, `y`) if it takes a touch there: the live page, inside the viewport.
    pub fn hit_page(&self, x: f32, y: f32) -> Option<usize> {
        (self.is_live(self.model.page) && self.layout.viewport.contains(x, y))
            .then_some(self.model.page)
    }

    /// Switches the focus scope of every page on or off to match: only the live page's scope takes input, the others (and all of
    /// them while the pages move) are absent from hit testing and focus navigation.
    pub fn sync_focus(&self, focus: &mut Focus, scope_of: impl Fn(usize) -> Scope) {
        for page in 0..self.model.pages {
            focus.set_inert(scope_of(page), !self.is_live(page));
        }
    }

    /// The group of page `page`, to build its content when the pager says it became visible; the pager poses it again when the
    /// owner is done (the new layers sit where they were made until then).
    pub fn fill(&mut self, page: usize, build: impl FnOnce(&mut Group)) {
        build(&mut self.pages[page]);
        self.posed[page] = None;
        let offset = self.offset;
        self.pose_page(page, offset, None);
    }

    /// Reads a page's group.
    pub fn page_group(&self, page: usize) -> &Group {
        &self.pages[page]
    }

    /// Turns to `page` (clamped), as the previous/next buttons and an accessibility action do. Returns what the pager tells the owner.
    pub fn go_to(&mut self, ui: &mut Ui, page: usize, now_ns: u64) -> Vec<PagerAction> {
        self.tick(now_ns);
        let page = page.min(self.model.pages - 1);
        if page != self.model.page {
            self.turn_to(ui, page, true, None);
        }
        std::mem::take(&mut self.pending)
    }

    /// Takes the actions that a model change has made (visibility changes). A touch's actions come back from `input`.
    pub fn take_actions(&mut self) -> Vec<PagerAction> {
        std::mem::take(&mut self.pending)
    }

    /// The look was replaced: the dots and buttons are drawn in the new colours. (The pages are the owner's.)
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.refresh_dots(ui, false);
        for button in [&mut self.prev, &mut self.next].into_iter().flatten() {
            button.restyle(ui);
        }
    }

    // ---- posing ----

    /// How page `page` is posed with the strip at `offset`.
    fn strip_pose(&self, page: usize, offset: f32) -> Pose {
        let stride = self.layout.stride().max(1.0);
        let dx = page as f32 * self.layout.stride() + offset;
        let distance = (dx / stride).abs().min(1.0);
        let visible = self
            .layout
            .slot()
            .moved(dx, 0.0)
            .intersect(&self.layout.viewport)
            .is_some();
        Pose {
            dx,
            dy: 0.0,
            scale: 1.0,
            opacity: 1.0 - (1.0 - NEIGHBOUR_OPACITY) * distance,
            visible,
        }
    }

    /// Cuts every part of page `page` to the viewport, then moves it to `pose` at once or glides it there.
    fn put(&mut self, page: usize, pose: Pose, glide: Option<(u32, Ease)>) {
        let v = self.layout.viewport;
        let clip = Rect {
            x: v.x,
            y: v.y,
            w: v.w,
            h: v.h,
        };
        let group = &mut self.pages[page];
        for part in &mut group.parts {
            part.clip = Some(clip);
        }
        match glide {
            Some((ms, ease)) => group.animate(pose, ms, ease),
            None => group.set(pose),
        }
        self.posed[page] = Some(pose);
    }

    /// Whether page `page` is already posed as `pose` says. A page that is hidden and stays hidden needs nothing, wherever it sits.
    fn already(&self, page: usize, pose: Pose) -> bool {
        match self.posed[page] {
            Some(was) => was == pose || (!was.visible && !pose.visible),
            None => false,
        }
    }

    fn pose_page(&mut self, page: usize, offset: f32, glide: Option<(u32, Ease)>) {
        let pose = self.strip_pose(page, offset);
        if !self.already(page, pose) {
            self.put(page, pose, glide);
        }
    }

    /// Poses every page for the strip at `offset` with a `Set`; a page that is where it was told to be is not touched.
    fn pose_all(&mut self, offset: f32) {
        self.offset = offset;
        for page in 0..self.model.pages {
            self.pose_page(page, offset, None);
        }
    }

    /// Glides every page to where it belongs with `page` current.
    fn glide_to(&mut self, ui: &Ui, page: usize, ms: u32, ease: Ease) {
        let offset = -(page as f32) * self.layout.stride();
        self.offset = offset;
        if ui.look.moves() {
            for p in 0..self.model.pages {
                let pose = self.strip_pose(p, offset);
                // A page that stays hidden and is already where it will be needs nothing; one that is leaving still glides.
                if !self.already(p, pose) {
                    self.put(p, pose, Some((ms, ease)));
                }
            }
        } else {
            self.crossfade(page, ms);
        }
        self.settling_until_ns = self.now_ns + u64::from(ms) * 1_000_000;
    }

    /// Reduced motion: no pages travel. The new page fades in where it belongs and the old one fades out where it was.
    fn crossfade(&mut self, to: usize, ms: u32) {
        let from = self.model.page;
        let offset = -(to as f32) * self.layout.stride();
        for p in 0..self.model.pages {
            let end = self.strip_pose(p, offset);
            if p == to {
                self.put(
                    p,
                    Pose {
                        opacity: 0.0,
                        ..end
                    },
                    None,
                );
                self.put(p, end, Some((ms, motion::SETTLE)));
            } else if p == from && !end.visible {
                let here = self.posed[p].unwrap_or(end);
                self.put(
                    p,
                    Pose {
                        opacity: 0.0,
                        visible: false,
                        ..here
                    },
                    Some((ms, motion::EXIT)),
                );
            } else if !self.already(p, end) {
                self.put(p, end, None);
            }
        }
    }

    // ---- turning ----

    /// Makes `page` the current page: the dots move, the buttons enable, visibility changes are noted and the pages glide. `announce`
    /// adds [`PagerAction::PageChanged`] (a change the owner did not make itself). `release` is the finger's glide, if a finger
    /// is what turned it.
    fn turn_to(&mut self, ui: &mut Ui, page: usize, announce: bool, release: Option<(u32, Ease)>) {
        let before = self.visible_pages();
        let ms_default = ui.look.duration_ms(motion::LAYER_MS);
        let (ms, ease) = match release {
            Some((ms, ease)) => (ui.look.duration_ms(ms), ease),
            None => (ms_default, motion::SETTLE),
        };
        self.glide_to(ui, page, ms, ease);
        self.model.page = page;
        if announce {
            self.pending.push(PagerAction::PageChanged(page));
        }
        let after = self.visible_pages();
        for &p in after.iter().filter(|p| !before.contains(p)) {
            self.pending.push(PagerAction::Visibility {
                page: p,
                visible: true,
            });
        }
        for &p in before.iter().filter(|p| !after.contains(p)) {
            self.pending.push(PagerAction::Visibility {
                page: p,
                visible: false,
            });
        }
        self.refresh_dots(ui, true);
        self.refresh_arrows(ui);
    }

    fn refresh_dots(&mut self, ui: &Ui, animate: bool) {
        let page = self.model.page;
        for (i, part) in self.dots.parts.iter_mut().enumerate() {
            let selected = i == page;
            part.rest_scale = if selected { SELECTED_DOT } else { 1.0 };
            if let Some(layer) = part.shape() {
                layer.reshape(solid(dot_colour(ui, selected), DOT / 2.0));
            }
        }
        let all: Vec<usize> = (0..self.dots.parts.len()).collect();
        if animate && ui.look.moves() {
            self.dots.animate_parts(
                &all,
                Pose::REST,
                ui.look.duration_ms(motion::LABEL_MS),
                motion::SETTLE,
            );
        } else {
            self.dots.set_parts(&all, Pose::REST);
        }
    }

    fn refresh_arrows(&mut self, ui: &mut Ui) {
        let (page, pages) = (self.model.page, self.model.pages);
        if let Some(b) = &mut self.prev {
            b.update(&arrow_model(Icon::Back, "Previous page", page > 0), ui);
        }
        if let Some(b) = &mut self.next {
            b.update(
                &arrow_model(Icon::ChevronRight, "Next page", page + 1 < pages),
                ui,
            );
        }
    }

    fn take_input_actions(&mut self) -> Option<Vec<PagerAction>> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }

    fn finger(&mut self, ui: &mut Ui, touch: &crate::input::TouchEvent) {
        let sample = Sample::of(touch);
        let stride = self.layout.stride();
        match touch.phase {
            TouchPhase::Down => {
                if self.drag.is_none()
                    && self.layout.viewport.contains(touch.x, touch.y)
                    && self.is_still()
                {
                    let mut contact = Contact::default();
                    contact.down(sample);
                    self.drag = Some(Drag {
                        id: touch.id,
                        contact,
                        anchor_x: None,
                        other_way: false,
                    });
                }
            }
            TouchPhase::Move => {
                let Some(drag) = self.drag.as_mut().filter(|d| d.id == touch.id) else {
                    return;
                };
                drag.contact.moved(sample);
                if drag.anchor_x.is_none() && !drag.other_way && drag.contact.dragged() {
                    let (dx, dy) = drag.contact.travel();
                    if dx.abs() > dy.abs() {
                        let start = drag.contact.start().map_or(touch.x, |s| s.x);
                        drag.anchor_x = Some(start + SLOP * dx.signum());
                    } else {
                        drag.other_way = true;
                    }
                }
                if let Some(anchor) = drag.anchor_x {
                    let offset = PagerDrag {
                        pages: self.model.pages,
                        width: stride,
                    }
                    .offset(self.model.page, touch.x - anchor);
                    self.pose_all(offset);
                }
            }
            TouchPhase::Up | TouchPhase::Cancel => {
                if !self.drag.as_ref().is_some_and(|d| d.id == touch.id) {
                    return;
                }
                let Some(mut drag) = self.drag.take() else {
                    return;
                };
                let Some(anchor) = drag.anchor_x else {
                    return;
                };
                if touch.phase == TouchPhase::Up {
                    drag.contact.moved(sample);
                }
                let cancelled = touch.phase == TouchPhase::Cancel;
                let (vx, _) = drag.contact.velocity();
                let dx = if cancelled { 0.0 } else { touch.x - anchor };
                let drag_rule = PagerDrag {
                    pages: self.model.pages,
                    width: stride,
                };
                let target = if cancelled {
                    self.model.page
                } else {
                    drag_rule.settle(self.model.page, dx, vx)
                };
                let end = -(target as f32) * stride;
                // The finger's speed along the way the strip has to go.
                let way = (end - self.offset).signum();
                let speed = if cancelled { 0.0 } else { vx * way };
                let glide = swipe::release(end - self.offset, speed, stride, true);
                if target != self.model.page {
                    self.turn_to(ui, target, true, Some(glide));
                } else {
                    let ms = ui.look.duration_ms(glide.0);
                    self.glide_to(ui, target, ms, glide.1);
                }
            }
        }
    }
}

fn arrow_model(icon: Icon, name: &str, enabled: bool) -> ButtonModel {
    ButtonModel::icon(icon, name).enabled(enabled)
}

fn dot_colour(ui: &Ui, selected: bool) -> compositor_protocol::Rgba {
    if selected {
        ui.look.ink()
    } else {
        ui.look.ink3()
    }
}

/// `page` and the pages next to it, out of `pages`.
fn prepared(page: usize, pages: usize) -> Vec<usize> {
    (page.saturating_sub(1)..=(page + 1).min(pages.saturating_sub(1))).collect()
}

impl Component for Pager {
    type Model = PagerModel;
    /// Everything one touch or one model change made the pager say, in order: the page change, then the pages that became visible,
    /// then the ones that stopped being.
    type Action = Vec<PagerAction>;

    fn update(&mut self, model: &PagerModel, ui: &mut Ui) -> Damage {
        debug_assert_eq!(
            model.pages, self.model.pages,
            "a pager's page count is its groups; build a new pager for another count"
        );
        let page = model.page.min(self.model.pages - 1);
        if page == self.model.page {
            return Damage::None;
        }
        self.turn_to(ui, page, false, None);
        Damage::Properties
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<Vec<PagerAction>> {
        let Event::Touch(touch) = event else {
            return None;
        };
        self.tick(touch.at_ns);
        if self.drag.is_none() {
            let (page, pages) = (self.model.page, self.model.pages);
            let mut turn = None;
            if let Some(b) = &mut self.prev {
                if b.input(event, ui) == Some(ButtonAction::Activated) {
                    turn = page.checked_sub(1);
                }
            }
            if let Some(b) = &mut self.next {
                if b.input(event, ui) == Some(ButtonAction::Activated) && page + 1 < pages {
                    turn = Some(page + 1);
                }
            }
            if let Some(to) = turn {
                self.turn_to(ui, to, true, None);
            }
        }
        self.finger(ui, touch);
        self.take_input_actions()
    }

    fn frame(&self) -> Frame {
        self.layout.viewport
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        let m = self.model;
        out.push(
            SemanticNode::new(
                SemanticKind::Page,
                format!("Page {} of {}", m.page + 1, m.pages),
                self.layout.viewport,
            )
            .value((m.page + 1).to_string()),
        );
        for button in [&self.prev, &self.next].into_iter().flatten() {
            button.semantics(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Focus, FocusId, TouchEvent};
    use crate::lucent::palette::Mode;
    use crate::testing::{Harness, harness};
    use compositor_protocol::{LayerId, Request, Rgba};

    const VIEW: Frame = Frame::new(0.0, 0.0, 960.0, 400.0);

    fn touch(phase: TouchPhase, x: f32, y: f32, ms: u64) -> Event {
        Event::Touch(TouchEvent {
            id: 1,
            phase,
            x,
            y,
            at_ns: ms * 1_000_000,
        })
    }

    /// A page of two plates, at the slot the layout gives a page.
    fn page(h: &Harness, layout: &PagerLayout) -> (Group, Vec<LayerId>) {
        let slot = layout.slot();
        let mut g = Group::new(slot.centre());
        let mut ids = Vec::new();
        for k in 0..2 {
            let frame = Frame::new(slot.x + 20.0, slot.y + 20.0 + k as f32 * 100.0, 200.0, 80.0);
            let layer = shape(&h.ui, frame, 5, solid(Rgba(200, 100, 50, 255), 8.0));
            ids.push(layer.id());
            g.push(Part::new(layer, frame));
        }
        (g, ids)
    }

    struct Rig {
        h: Harness,
        pager: Pager,
        ids: Vec<Vec<LayerId>>,
    }

    fn rig(layout: PagerLayout, pages: usize, at: usize) -> Rig {
        let mut h = harness();
        let mut groups = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..pages {
            let (g, i) = page(&h, &layout);
            groups.push(g);
            ids.push(i);
        }
        let pager = Pager::new(
            &mut h.ui,
            layout,
            20,
            PagerModel { pages, page: at },
            groups,
        )
        .unwrap();
        h.commit();
        Rig { h, pager, ids }
    }

    fn layout() -> PagerLayout {
        PagerLayout::full(VIEW, 440.0)
    }

    fn props_of(h: &Harness, id: LayerId) -> compositor_protocol::Props {
        h.layers().into_iter().find(|l| l.0 == id).unwrap().2
    }

    fn created_or_deleted(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Create { .. } | Request::Delete { .. }))
            .count()
    }

    fn drag_to(r: &mut Rig, from: f32, to: f32) {
        r.pager
            .input(&touch(TouchPhase::Down, from, 200.0, 0), &mut r.h.ui);
        let steps = 8;
        for k in 1..=steps {
            let x = from + (to - from) * k as f32 / steps as f32;
            r.pager
                .input(&touch(TouchPhase::Move, x, 200.0, 16 * k), &mut r.h.ui);
        }
    }

    #[test]
    fn at_rest_only_the_current_page_shows_and_it_is_cut_to_the_viewport() {
        let r = rig(layout(), 3, 1);
        for (i, ids) in r.ids.iter().enumerate() {
            for id in ids {
                let p = props_of(&r.h, *id);
                assert_eq!(p.visible, i == 1, "page {i}");
                let clip = p.clip.expect("every page layer is clipped to the viewport");
                assert_eq!((clip.x, clip.y, clip.w, clip.h), (0.0, 0.0, 960.0, 400.0));
            }
        }
        assert_eq!(r.pager.visible_pages(), [0, 1, 2]);
    }

    #[test]
    fn the_pages_follow_the_finger_one_for_one_with_a_set_and_make_no_layer() {
        let mut r = rig(layout(), 3, 1);
        // The slop is taken up first; after it the strip moves exactly as the finger does.
        r.pager
            .input(&touch(TouchPhase::Down, 500.0, 200.0, 0), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 440.0, 200.0, 16), &mut r.h.ui);
        assert!(r.pager.is_dragging());
        r.h.commit();
        let before = props_of(&r.h, r.ids[1][0]).x;
        r.pager
            .input(&touch(TouchPhase::Move, 340.0, 200.0, 32), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 300.0, 200.0, 48), &mut r.h.ui);
        let sent = r.h.commit();
        assert_eq!(
            created_or_deleted(&sent),
            0,
            "no layer made or dropped while a finger drags"
        );
        assert!(
            sent.iter()
                .all(|q| matches!(q, Request::Set { .. } | Request::Commit { .. })),
            "a drag is Set only: {sent:?}"
        );
        let after = props_of(&r.h, r.ids[1][0]).x;
        assert!(
            (after - (before - 140.0)).abs() < 0.01,
            "followed 140 units: {before} -> {after}"
        );
        // The neighbour to the right is coming in and is fading in with its distance.
        let next = props_of(&r.h, r.ids[2][0]);
        assert!(
            next.visible && next.opacity < 1.0 && next.opacity > NEIGHBOUR_OPACITY - 0.01,
            "{next:?}"
        );
    }

    #[test]
    fn a_drag_sample_sets_only_the_pages_that_can_be_seen() {
        let mut r = rig(layout(), 6, 2);
        r.pager
            .input(&touch(TouchPhase::Down, 500.0, 200.0, 0), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 440.0, 200.0, 16), &mut r.h.ui);
        r.h.commit();
        r.pager
            .input(&touch(TouchPhase::Move, 400.0, 200.0, 32), &mut r.h.ui);
        let sets =
            r.h.commit()
                .iter()
                .filter(|q| matches!(q, Request::Set { .. }))
                .count();
        assert_eq!(
            sets, 4,
            "two plates on each of the two pages that show: the current and the next"
        );
    }

    #[test]
    fn a_vertical_swipe_is_not_taken_and_a_sideways_one_past_the_slop_is() {
        let mut r = rig(layout(), 3, 1);
        r.pager
            .input(&touch(TouchPhase::Down, 500.0, 200.0, 0), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 510.0, 280.0, 16), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 300.0, 285.0, 32), &mut r.h.ui);
        assert!(
            !r.pager.is_dragging(),
            "it went up or down first: a scroll for something else"
        );
        assert_eq!(
            r.pager
                .input(&touch(TouchPhase::Up, 300.0, 285.0, 48), &mut r.h.ui),
            None
        );
        assert_eq!(r.pager.page(), 1);

        r.pager.tick(1_000_000_000);
        r.pager
            .input(&touch(TouchPhase::Down, 500.0, 200.0, 1000), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 480.0, 200.0, 1016), &mut r.h.ui);
        assert!(
            !r.pager.is_dragging(),
            "inside the slop it is still a press"
        );
        r.pager
            .input(&touch(TouchPhase::Move, 440.0, 202.0, 1032), &mut r.h.ui);
        assert!(r.pager.is_dragging());
    }

    #[test]
    fn a_swipe_past_a_quarter_page_turns_it_and_says_so_with_the_pages_that_became_visible() {
        let mut r = rig(layout(), 5, 1);
        drag_to(&mut r, 700.0, 380.0);
        r.h.commit();
        let actions = r
            .pager
            .input(&touch(TouchPhase::Up, 380.0, 200.0, 140), &mut r.h.ui)
            .expect("the swipe turned the page");
        assert_eq!(
            actions,
            [
                PagerAction::PageChanged(2),
                PagerAction::Visibility {
                    page: 3,
                    visible: true
                },
                PagerAction::Visibility {
                    page: 0,
                    visible: false
                },
            ]
        );
        assert_eq!(r.pager.page(), 2);
        let sent = r.h.commit();
        assert!(
            sent.iter().any(|q| matches!(q, Request::Animate { .. })),
            "the pages glide into place"
        );
        assert_eq!(created_or_deleted(&sent), 0);
        // Page 2 ends at rest, shown; page 1 ends off to the left and hidden.
        let end = |id: LayerId| -> compositor_protocol::Props {
            sent.iter()
                .rev()
                .find_map(|q| match q {
                    Request::Animate { id: i, to, .. } if *i == id => Some(*to),
                    _ => None,
                })
                .expect("animated")
        };
        let shown = end(r.ids[2][0]);
        assert!(shown.visible && shown.opacity == 1.0);
        assert!(
            (shown.x - (20.0)).abs() < 0.01,
            "back at its rest place: {}",
            shown.x
        );
        assert!(!end(r.ids[1][0]).visible);
    }

    #[test]
    fn a_short_swipe_springs_back_and_a_fast_flick_turns_it_however_short() {
        let mut r = rig(layout(), 3, 1);
        drag_to(&mut r, 600.0, 520.0);
        let none = r
            .pager
            .input(&touch(TouchPhase::Up, 520.0, 200.0, 400), &mut r.h.ui);
        assert_eq!(none, None, "80 units slowly is not a turn");
        assert_eq!(r.pager.page(), 1);
        r.pager.tick(5_000_000_000);

        // 60 units in 30 ms is 2000 units per second: a flick.
        r.pager
            .input(&touch(TouchPhase::Down, 600.0, 200.0, 5000), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 560.0, 200.0, 5010), &mut r.h.ui);
        r.pager
            .input(&touch(TouchPhase::Move, 520.0, 200.0, 5020), &mut r.h.ui);
        let flicked = r
            .pager
            .input(&touch(TouchPhase::Up, 500.0, 200.0, 5030), &mut r.h.ui)
            .unwrap();
        assert_eq!(flicked[0], PagerAction::PageChanged(2));
    }

    #[test]
    fn the_ends_resist_and_never_turn_past_the_first_or_last_page() {
        let mut r = rig(layout(), 3, 0);
        drag_to(&mut r, 300.0, 700.0);
        let x = props_of(&r.h, r.ids[0][0]);
        r.h.commit();
        let shifted = props_of(&r.h, r.ids[0][0]).x - 20.0;
        assert!(
            shifted > 0.0 && shifted < (700.0 - 300.0 - SLOP) * 0.5,
            "resisted: {shifted}, {x:?}"
        );
        assert_eq!(
            r.pager
                .input(&touch(TouchPhase::Up, 700.0, 200.0, 140), &mut r.h.ui),
            None
        );
        assert_eq!(r.pager.page(), 0);
    }

    #[test]
    fn a_lost_contact_springs_back_without_turning() {
        let mut r = rig(layout(), 3, 1);
        drag_to(&mut r, 700.0, 300.0);
        assert_eq!(
            r.pager
                .input(&touch(TouchPhase::Cancel, 0.0, 0.0, 200), &mut r.h.ui),
            None
        );
        assert_eq!(r.pager.page(), 1);
        let sent = r.h.commit();
        assert!(sent.iter().any(|q| matches!(q, Request::Animate { .. })));
    }

    #[test]
    fn peeking_neighbours_show_at_rest_and_fade_with_their_distance() {
        let layout = PagerLayout {
            viewport: VIEW,
            page_width: 760.0,
            gap: 16.0,
            indicator_y: 440.0,
            arrows: false,
        };
        let r = rig(layout, 3, 1);
        let centre = props_of(&r.h, r.ids[1][0]);
        let right = props_of(&r.h, r.ids[2][0]);
        assert!(centre.visible && centre.opacity == 1.0);
        assert!(right.visible, "a sliver of the next page shows");
        assert!(
            (right.opacity - NEIGHBOUR_OPACITY).abs() < 0.001,
            "a stride away is the faintest: {}",
            right.opacity
        );
        assert_eq!(right.x - centre.x, 776.0, "one page and one gap along");
    }

    #[test]
    fn the_dots_show_the_page_and_the_selected_one_is_larger() {
        let mut r = rig(layout(), 4, 0);
        let selected = |r: &Rig| -> Vec<usize> {
            r.pager
                .dots
                .parts
                .iter()
                .enumerate()
                .filter(|(_, p)| p.rest_scale > 1.0)
                .map(|(i, _)| i)
                .collect()
        };
        assert_eq!(selected(&r), [0]);
        assert_eq!(
            r.pager.dots.parts.len(),
            4,
            "one retained plate for each dot"
        );
        r.pager.go_to(&mut r.h.ui, 2, 0);
        assert_eq!(selected(&r), [2]);
        let ink = r.h.ui.look.ink();
        let ink3 = r.h.ui.look.ink3();
        r.h.commit();
        let shapes: Vec<_> =
            r.h.layers()
                .into_iter()
                .filter_map(|(_, c, _)| match c {
                    compositor_protocol::Content::Shape(s) => Some(s),
                    _ => None,
                })
                .collect();
        let coloured = |c: Rgba| {
            shapes
                .iter()
                .filter(|s| s.fill == compositor_protocol::Fill::Solid(c))
                .count()
        };
        assert_eq!(
            coloured(ink),
            1,
            "only the selected dot is in the primary ink"
        );
        assert_eq!(coloured(ink3), 3);
    }

    #[test]
    fn a_single_page_has_no_dots_and_a_pager_needs_a_group_for_every_page() {
        let r = rig(layout(), 1, 0);
        assert!(r.pager.dots.parts.is_empty());
        let mut h = harness();
        let (g, _) = page(&h, &layout());
        let err = Pager::new(
            &mut h.ui,
            layout(),
            20,
            PagerModel { pages: 2, page: 0 },
            vec![g],
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = Pager::new(
            &mut h.ui,
            layout(),
            20,
            PagerModel { pages: 0, page: 0 },
            vec![],
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn go_to_turns_the_page_for_a_button_or_an_assistive_action_and_clamps() {
        let mut r = rig(layout(), 3, 0);
        let actions = r.pager.go_to(&mut r.h.ui, 7, 0);
        assert_eq!(actions[0], PagerAction::PageChanged(2));
        assert_eq!(r.pager.page(), 2);
        assert!(r.pager.go_to(&mut r.h.ui, 2, 0).is_empty(), "already there");
    }

    #[test]
    fn the_previous_and_next_buttons_turn_the_page_and_end_with_the_pages() {
        let layout = PagerLayout {
            arrows: true,
            ..layout()
        };
        let mut r = rig(layout, 3, 0);
        let mut sink = SemanticsSink::new();
        r.pager.semantics(&mut sink);
        assert!(
            !sink.find("Previous page").unwrap().enabled,
            "nothing before the first"
        );
        let next = sink.find("Next page").unwrap().frame;
        assert!(next.w >= 64.0 && next.h >= 64.0, "a 64 unit target");
        let (cx, cy) = next.centre();
        r.pager
            .input(&touch(TouchPhase::Down, cx, cy, 0), &mut r.h.ui);
        let actions = r
            .pager
            .input(&touch(TouchPhase::Up, cx, cy, 50), &mut r.h.ui)
            .unwrap();
        assert_eq!(actions[0], PagerAction::PageChanged(1));
        r.pager.tick(1_000_000_000);
        for ms in [2000, 3000] {
            r.pager
                .input(&touch(TouchPhase::Down, cx, cy, ms), &mut r.h.ui);
            r.pager
                .input(&touch(TouchPhase::Up, cx, cy, ms + 40), &mut r.h.ui);
            r.pager.tick((ms + 40) * 1_000_000);
        }
        assert_eq!(r.pager.page(), 2);
        let mut sink = SemanticsSink::new();
        r.pager.semantics(&mut sink);
        assert!(
            !sink.find("Next page").unwrap().enabled,
            "nothing after the last"
        );
        assert!(sink.find("Previous page").unwrap().enabled);
    }

    #[test]
    fn a_page_that_is_not_current_or_is_moving_is_absent_from_hit_testing_focus_and_semantics() {
        let mut r = rig(layout(), 3, 1);
        assert_eq!(r.pager.hit_page(100.0, 100.0), Some(1));
        assert_eq!(r.pager.hit_page(100.0, 450.0), None, "outside the viewport");
        let mut focus = Focus::new();
        let scope = |p: usize| Scope(10 + p as u32);
        for p in 0..3 {
            focus.register(
                FocusId(p as u32),
                Frame::new(10.0, 10.0, 100.0, 64.0),
                scope(p),
            );
        }
        r.pager.sync_focus(&mut focus, scope);
        assert_eq!(
            focus.hit(20.0, 20.0),
            Some(FocusId(1)),
            "only page 1 takes a touch"
        );
        // A swipe is under way: nothing takes a touch.
        drag_to(&mut r, 700.0, 400.0);
        r.pager.sync_focus(&mut focus, scope);
        assert_eq!(focus.hit(20.0, 20.0), None);
        assert_eq!(r.pager.hit_page(100.0, 100.0), None);
        // It lands on page 2: that page is live once the time has passed.
        r.pager
            .input(&touch(TouchPhase::Up, 400.0, 200.0, 150), &mut r.h.ui);
        assert!(!r.pager.is_live(2), "still gliding");
        r.pager.tick(1_000_000_000);
        r.pager.sync_focus(&mut focus, scope);
        assert_eq!(focus.hit(20.0, 20.0), Some(FocusId(2)));
        assert_eq!(r.pager.hit_page(100.0, 100.0), Some(2));
        let mut sink = SemanticsSink::new();
        r.pager.semantics(&mut sink);
        assert_eq!(sink.nodes().len(), 1, "one page is described, not three");
        assert_eq!(sink.nodes()[0].name, "Page 3 of 3");
    }

    #[test]
    fn a_model_that_changes_the_page_glides_there_and_tells_the_owner_what_to_prepare() {
        let mut r = rig(layout(), 5, 0);
        assert_eq!(
            r.pager
                .update(&PagerModel { pages: 5, page: 0 }, &mut r.h.ui),
            Damage::None
        );
        assert_eq!(
            r.pager
                .update(&PagerModel { pages: 5, page: 3 }, &mut r.h.ui),
            Damage::Properties
        );
        assert_eq!(
            r.pager.take_actions(),
            [
                PagerAction::Visibility {
                    page: 2,
                    visible: true
                },
                PagerAction::Visibility {
                    page: 3,
                    visible: true
                },
                PagerAction::Visibility {
                    page: 4,
                    visible: true
                },
                PagerAction::Visibility {
                    page: 0,
                    visible: false
                },
                PagerAction::Visibility {
                    page: 1,
                    visible: false
                },
            ]
        );
        assert!(r.pager.take_actions().is_empty());
    }

    #[test]
    fn content_the_owner_builds_for_a_page_that_became_visible_is_posed_with_the_rest() {
        let mut r = rig(layout(), 3, 1);
        let late = {
            let slot = layout().slot();
            let frame = Frame::new(slot.x + 300.0, slot.y + 20.0, 100.0, 50.0);
            shape(&r.h.ui, frame, 5, solid(Rgba(1, 2, 3, 255), 4.0))
        };
        let id = late.id();
        r.pager.fill(0, |g| {
            g.push(Part::new(late, Frame::new(300.0, 20.0, 100.0, 50.0)));
        });
        r.h.commit();
        let p = props_of(&r.h, id);
        assert!(!p.visible, "page 0 is off to the left, hidden");
        assert!(p.clip.is_some());
    }

    #[test]
    fn reduced_motion_fades_the_pages_and_none_of_them_travels() {
        let mut r = rig(layout(), 3, 0);
        r.h.ui.look.reduced_motion = true;
        r.pager.go_to(&mut r.h.ui, 1, 0);
        let sent = r.h.commit();
        let animated: Vec<_> = sent
            .iter()
            .filter_map(|q| match q {
                Request::Animate {
                    to,
                    duration_ms,
                    id,
                    ..
                } => Some((*id, *to, *duration_ms)),
                _ => None,
            })
            .collect();
        assert!(!animated.is_empty());
        for (_, _, ms) in &animated {
            assert!(*ms <= motion::REDUCED_MAX_MS);
        }
        // The page that comes in is made transparent where it belongs and fades up; it never slides from the side.
        let (_, to, _) = animated.iter().find(|(id, ..)| *id == r.ids[1][0]).unwrap();
        assert!((to.x - 20.0).abs() < 0.01 && to.opacity == 1.0 && to.visible);
        let start = sent
            .iter()
            .find_map(|q| match q {
                Request::Set { id, props } if *id == r.ids[1][0] => Some(*props),
                _ => None,
            })
            .unwrap();
        assert!((start.x - 20.0).abs() < 0.01 && start.opacity == 0.0);
    }

    #[test]
    fn light_and_dark_draw_the_dots_in_their_own_ink() {
        for mode in [Mode::Dark, Mode::Light] {
            let mut h = harness();
            h.ui.look = crate::Look::standard(mode);
            let layout = layout();
            let (g, _) = page(&h, &layout);
            let (g2, _) = page(&h, &layout);
            let pager = Pager::new(
                &mut h.ui,
                layout,
                20,
                PagerModel { pages: 2, page: 0 },
                vec![g, g2],
            )
            .unwrap();
            h.commit();
            let ink = h.ui.look.ink();
            assert!(
                h.layers().iter().any(|(_, c, _)| matches!(c, compositor_protocol::Content::Shape(s) if s.fill == compositor_protocol::Fill::Solid(ink))),
                "{mode:?}"
            );
            drop(pager);
        }
    }
}
