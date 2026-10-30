//! The toasts drawn (the mock `docs/mocks/07-toast`): a column of cards, 560 wide, the first line the title in the label role, the rest
//! the body in the caption role in the secondary ink, a tinted info tile at the left (or the service's own picture, when it gave one that
//! is a file the host can read). Where the column rests is the placement crate's (`layout.overlays.toast`): by default centred at the foot of
//! the screen, 5 % of the height above the bottom edge, and above the volume bar while that is up. A card comes in from the edge it rests
//! against (a rise from below at the foot) while it fades in. Three slots are made once, hidden, and shown with each toast's words: the
//! first real toast finds its layers built and its glyphs in the atlas (the QML kept one slot warm for the same reason).

use super::toast::Toast;
use crate::kit::Kit;
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style};
use compositor_protocol::{Props, Rect};
use lumatv_placement::{Frame as PlacementFrame, Rect as PlacedRect, Registry, Size, User, model::STACK, stack_beside};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};

const WIDTH: f32 = 560.0;
const TILE: u32 = 48;
const SLOTS: usize = 3;
/// How far a card travels while it fades in, from the edge it rests against.
const RISE: f32 = 24.0;

struct Slot {
    pane: Pane,
    tile: ShapeLayer,
    icon: ImageLayer,
    title: Text,
    body: Text,
    /// The toast shown in it, and where the slot is.
    showing: Option<u64>,
    at: (f32, f32),
}

pub struct ToastSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    slots: Vec<Slot>,
    generation: u64,
    text_scale: f32,
    height: f32,
}

impl ToastSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, slots: Vec::new(), generation: 0, text_scale: 1.0, height: 0.0 }
    }

    /// One card's height, for the editor's stand-in.
    pub fn sample_height(&self, kit: &Kit) -> f32 {
        Self::card_height(kit)
    }

    /// The owner's places changed (`layout.overlays`).
    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    /// One card's height: the title and the body lines inside the padding, never less than the tile's row.
    fn card_height(kit: &Kit) -> f32 {
        let pad = kit.space(3);
        let inner = (kit.line(TypeRole::Label) + kit.space(1) + kit.line(TypeRole::Caption)).max(TILE as f32);
        (inner + 2.0 * pad).ceil()
    }

    fn build(&mut self, kit: &mut Kit) -> std::io::Result<()> {
        self.slots.clear();
        let h = Self::card_height(kit);
        self.height = h;
        let pad = kit.space(3);
        let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
        let accent = kit.ui.look.accent();
        let tint = compositor_protocol::Rgba(
            (u32::from(accent.0) * 41 / 255) as u8,
            (u32::from(accent.1) * 41 / 255) as u8,
            (u32::from(accent.2) * 41 / 255) as u8,
            41,
        );
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        for _ in 0..SLOTS {
            let pane = Pane::new(kit, Frame::new(0.0, 0.0, WIDTH, h), self.z, PaneKind::Card, false);
            pane.layer().set(hidden);
            let tile =
                plate::shape(&kit.ui, Frame::new(0.0, 0.0, TILE as f32, TILE as f32), self.z + 1, plate::solid(tint, kit.tokens.radius(Radius::Control)));
            tile.set(hidden);
            let mut icon = kit.ui.scene.image(TILE * 7 / 12, TILE * 7 / 12, self.z + 2, hidden)?;
            glyph::draw("info", TILE * 7 / 12, accent, icon.pixels_mut());
            icon.damage(None);
            let text_w = WIDTH - 2.0 * pad - TILE as f32 - pad;
            let mut title = Text::new(
                kit,
                Frame::new(0.0, 0.0, text_w, kit.line(TypeRole::Label)),
                self.z + 3,
                role_style(kit, TypeRole::Label, None, ink, Align::Left),
                "",
            )?;
            title.set_props(hidden);
            let mut body = Text::new(
                kit,
                Frame::new(0.0, 0.0, text_w, kit.line(TypeRole::Caption)),
                self.z + 3,
                role_style(kit, TypeRole::Caption, None, ink2, Align::Left),
                "",
            )?;
            body.set_props(hidden);
            self.slots.push(Slot { pane, tile, icon, title, body, showing: None, at: (0.0, 0.0) });
        }
        self.generation = kit.generation();
        self.text_scale = kit.text_scale;
        Ok(())
    }

    /// Where a column of `n` cards starts: the toast item's rectangle for a column of that height (the place is a fraction of the room the
    /// column leaves, so it never runs past its zone), and whether the newest card is at the top of it. An item anchored in the upper half
    /// grows downwards from its edge (newest on top, the older ones under it); one anchored low grows upwards (newest at the foot, nearest
    /// the edge, the older ones above it). `bar` is the volume bar while it is up: a column that would meet it goes beside it, above it
    /// for one that grows up.
    fn column(&self, n: usize, gap: f32, bar: Option<&PlacedRect>) -> ((f32, f32), bool) {
        let n = n.max(1);
        let total = (n as f32 * self.height + (n - 1) as f32 * gap).ceil() as i32;
        let down = self.registry.placement_of("toast", &self.user).is_none_or(|p| p.placement.grows_down());
        let rect = self.registry.rect_of("toast", &self.user, &self.frame, Size::new(WIDTH as i32, total));
        let rect = rect.map(|r| stack_beside(bar, r, STACK, down));
        let at = rect.map_or((1920.0 - 24.0 - WIDTH, 24.0), |r| (r.x as f32, r.y as f32));
        (at, down)
    }

    /// How many cards the zone holds one over the other, at the card's height now (never fewer than one: a toast is not left unseen for its size).
    fn fit(&self, gap: f32) -> usize {
        let room = self.registry.zone_of("toast", &self.frame).h as f32;
        (((room + gap) / (self.height + gap)).floor() as usize).max(1)
    }

    /// Draws `toasts` (newest first). `bar`: the volume bar's rectangle while it is up. Returns whether any is on screen.
    pub fn apply(&mut self, kit: &mut Kit, toasts: &[Toast], bar: Option<Rect>) -> std::io::Result<bool> {
        if self.slots.is_empty() || self.generation != kit.generation() || self.text_scale != kit.text_scale {
            self.build(kit)?;
        }
        let pad = kit.space(3);
        let gap = kit.space(2);
        let h = self.height;
        let shown = toasts.len().min(SLOTS).min(self.fit(gap));
        let bar = bar.map(|b| PlacedRect::new(b.x.round() as i32, b.y.round() as i32, b.w.round() as i32, b.h.round() as i32));
        let ((x0, y0), down) = self.column(shown, gap, bar.as_ref());
        // a card arrives from the edge it rests against
        let from = self.registry.slide_of("toast", &self.user);
        let travel = if kit.ui.look.reduced_motion { 0.0 } else { RISE };
        let (dx, dy) = (from.x as f32 * travel, from.y as f32 * travel);
        let title_h = kit.line(TypeRole::Label);
        let body_h = kit.line(TypeRole::Caption);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            // the newest is nearest the edge the item is anchored to
            let row = if down { i } else { shown.saturating_sub(1).saturating_sub(i) };
            let y = y0 + row as f32 * (h + gap);
            match toasts.get(i).filter(|_| i < shown) {
                Some(t) => {
                    let fresh = slot.showing != Some(t.id);
                    slot.title.set(kit, &t.title);
                    slot.body.set(kit, &t.body);
                    let pane_props = Props::at(x0, y);
                    let tile_at = Props::at(x0 + pad, y + ((h - TILE as f32) / 2.0).round());
                    let icon_side = (TILE * 7 / 12) as f32;
                    let icon_at = Props::at(tile_at.x + ((TILE as f32 - icon_side) / 2.0).round(), tile_at.y + ((TILE as f32 - icon_side) / 2.0).round());
                    let text_x = x0 + pad + TILE as f32 + pad;
                    let has_body = !t.body.is_empty();
                    let block = if has_body { title_h + kit.space(1) + body_h } else { title_h };
                    let title_at = Props::at(text_x, y + ((h - block) / 2.0).round());
                    let body_at = Props::at(text_x, title_at.y + title_h + kit.space(1));
                    if fresh || slot.at != (x0, y) {
                        let to = |kit: &Kit, layer: &ui::scene::Layer, at: Props| {
                            if fresh { kit.arrive_from(layer, at, (dx, dy), Beat::Layer) } else { kit.glide(layer, at, Beat::Layer) }
                        };
                        to(kit, slot.pane.layer(), pane_props);
                        to(kit, &slot.tile, tile_at);
                        to(kit, &slot.icon, icon_at);
                        to(kit, slot.title.layer(), title_at);
                        if has_body {
                            to(kit, slot.body.layer(), body_at);
                        } else {
                            slot.body.set_props(Props { opacity: 0.0, visible: false, ..body_at });
                        }
                        slot.pane.move_to(x0, y);
                        slot.at = (x0, y);
                        slot.showing = Some(t.id);
                    } else if !has_body {
                        slot.body.set_props(Props { opacity: 0.0, visible: false, ..body_at });
                    } else {
                        slot.body.set_props(body_at);
                    }
                }
                None => {
                    if slot.showing.is_some() {
                        let from = slot.pane.at();
                        kit.leave(slot.pane.layer(), from, Beat::Layer);
                        let gone = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
                        slot.tile.set(gone);
                        slot.icon.set(gone);
                        slot.title.set_props(gone);
                        slot.body.set_props(gone);
                        slot.showing = None;
                    }
                }
            }
        }
        Ok(!toasts.is_empty())
    }

    /// The cards' rectangles, top to bottom, while they show (the tests read where they are).
    pub fn rects(&self) -> Vec<Rect> {
        let mut rects: Vec<Rect> = self.slots.iter().filter(|s| s.showing.is_some()).map(|s| Rect { x: s.at.0, y: s.at.1, w: WIDTH, h: self.height }).collect();
        rects.sort_by(|a, b| a.y.total_cmp(&b.y));
        rects
    }

    pub fn restyle(&mut self) {
        self.slots.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use serde_json::json;

    fn toasts(n: u64) -> Vec<Toast> {
        (0..n).map(|i| Toast { id: n - i, title: format!("Toast {}", n - i), body: "Something happened".into(), icon: String::new(), expires: 99_000 }).collect()
    }

    fn surface(anchor: &str, dy: f64) -> ToastSurface {
        let theme = json!({"layout": {"overlays": {"toast": {"anchor": anchor, "dx": 0.0, "dy": dy}}}});
        ToastSurface::new(1900, &theme, &json!({}))
    }

    /// A surface the owner never placed: where the toasts rest by default.
    fn resting() -> ToastSurface {
        ToastSurface::new(1900, &json!({}), &json!({}))
    }

    #[test]
    fn by_default_the_toasts_rest_centred_at_the_foot_with_five_percent_of_the_height_under_them() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            let mut s = resting();
            s.apply(&mut kit, &toasts(1), None).unwrap();
            let r = s.rects()[0];
            assert_eq!(r.x + r.w / 2.0, 960.0, "centred at {scale}: {r:?}");
            assert!(1080.0 - (r.y + r.h) >= 54.0, "5 % of 1080 under it at {scale}: {r:?}");
            assert!(1080.0 - (r.y + r.h) < 54.0 + 1.0, "and not floating higher than that at {scale}: {r:?}");
            s.apply(&mut kit, &toasts(3), None).unwrap();
            let rects = s.rects();
            assert_eq!(rects.len(), 3);
            assert_eq!(rects[2].y + rects[2].h, 1026.0, "the newest at the foot, the older ones above it: {rects:?}");
            assert!(s.slots[0].at.1 > s.slots[1].at.1 && s.slots[1].at.1 > s.slots[2].at.1);
        }
    }

    #[test]
    fn above_the_volume_bar_while_it_is_up_and_at_the_foot_again_when_it_goes() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut s = resting();
        let bar = Rect { x: 740.0, y: 956.0, w: 440.0, h: 60.0 };
        s.apply(&mut kit, &toasts(2), None).unwrap();
        let at_foot = s.rects();
        s.apply(&mut kit, &toasts(2), Some(bar)).unwrap();
        let over = s.rects();
        let low = over.last().unwrap();
        assert_eq!(low.y + low.h, bar.y - 16.0, "the lowest card is 16 above the bar: {over:?}");
        assert_eq!(over[1].y - over[0].y, at_foot[1].y - at_foot[0].y, "the column keeps its shape");
        s.apply(&mut kit, &toasts(2), None).unwrap();
        assert_eq!(s.rects(), at_foot, "the bar gone, the column is back at the foot");
        // a bar lifted clear of a single card (over an app) moves nothing
        s.apply(&mut kit, &toasts(1), None).unwrap();
        let one = s.rects();
        s.apply(&mut kit, &toasts(1), Some(Rect { x: 740.0, y: 836.0, w: 440.0, h: 60.0 })).unwrap();
        assert_eq!(s.rects(), one);
    }

    #[test]
    fn a_column_anchored_high_grows_down_from_its_edge_and_one_anchored_low_grows_up_from_its_edge_the_newest_nearest_it() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut top = surface("top-right", 0.0);
        assert!(top.apply(&mut kit, &toasts(3), None).unwrap());
        let rects = top.rects();
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0].y, 24.0, "the first card is at the top edge of the zone");
        assert!(rects[1].y > rects[0].y && rects[2].y > rects[1].y, "the older ones under it");
        // the newest (the first of the list) is the one nearest the edge
        assert_eq!(top.slots[0].at.1, rects[0].y);

        let mut low = surface("bottom-right", 0.0);
        assert!(low.apply(&mut kit, &toasts(3), None).unwrap());
        let rects = low.rects();
        let zone_bottom = 1026.0;
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[2].y + rects[2].h, zone_bottom, "the lowest card ends at the zone's foot: {rects:?}");
        assert!(rects[0].y >= 24.0, "the column stays inside its zone: {rects:?}");
        assert!(low.slots[0].at.1 > low.slots[1].at.1 && low.slots[1].at.1 > low.slots[2].at.1, "the newest at the foot, the older ones above it");
    }

    #[test]
    fn one_toast_anchored_low_is_at_the_foot_and_the_column_never_leaves_its_zone_at_large_text() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            for (anchor, dy) in [("top-right", 0.0), ("right", 0.3), ("bottom-right", 0.0), ("bottom-left", 0.0)] {
                let mut s = surface(anchor, dy);
                s.apply(&mut kit, &toasts(1), None).unwrap();
                let one = s.rects();
                assert_eq!(one.len(), 1);
                s.apply(&mut kit, &toasts(3), None).unwrap();
                let rects = s.rects();
                assert!(!rects.is_empty());
                for r in &rects {
                    assert!(r.y >= 24.0 - 0.5 && r.y + r.h <= 1026.0 + 0.5, "{anchor} at {scale}: {r:?} is outside the zone");
                }
                for pair in rects.windows(2) {
                    assert!(pair[1].y >= pair[0].y + pair[0].h, "{anchor} at {scale}: cards overlap: {rects:?}");
                }
                assert!(s.fit(kit.space(2)) >= rects.len(), "{anchor} at {scale}: the zone holds what is drawn");
            }
        }
    }
}
