//! The toasts drawn (the mock `docs/mocks/07-toast`): a column of cards in the upper right safe corner, 560 wide, the first line
//! the title in the label role, the rest the body in the caption role in the secondary ink, a tinted info tile at the left (or the
//! service's own picture, when it gave one that is a file the host can read). Where the column starts is the placement crate's
//! (`layout.overlays.toast`). Three slots are made once, hidden, and shown with each toast's words: the first real toast finds
//! its layers built and its glyphs in the atlas (the QML kept one slot warm for the same reason).

use super::toast::Toast;
use crate::kit::Kit;
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style};
use compositor_protocol::{Props, Rect};
use lumatv_placement::{Frame as PlacementFrame, Registry, Size, Sizes, User};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};

const WIDTH: f32 = 560.0;
const TILE: u32 = 48;
const SLOTS: usize = 3;

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

    /// Where the column starts: the toast item's rectangle for one card.
    fn origin(&self) -> (f32, f32) {
        let mut sizes = Sizes::new();
        sizes.set("toast", Size::new(WIDTH as i32, self.height as i32));
        let laid = self.registry.layout_all(&self.user, &self.frame, &sizes, None, false);
        laid.rect("toast").map_or((1920.0 - 24.0 - WIDTH, 24.0), |r| (r.x as f32, r.y as f32))
    }

    /// Draws `toasts` (newest first). Returns whether any is on screen.
    pub fn apply(&mut self, kit: &mut Kit, toasts: &[Toast]) -> std::io::Result<bool> {
        if self.slots.is_empty() || self.generation != kit.generation() || self.text_scale != kit.text_scale {
            self.build(kit)?;
        }
        let (x0, y0) = self.origin();
        let pad = kit.space(3);
        let gap = kit.space(2);
        let h = self.height;
        let title_h = kit.line(TypeRole::Label);
        let body_h = kit.line(TypeRole::Caption);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let y = y0 + i as f32 * (h + gap);
            match toasts.get(i) {
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
                        let at_once = !fresh;
                        kit.arrive(slot.pane.layer(), pane_props, Beat::Layer, at_once);
                        kit.arrive(&slot.tile, tile_at, Beat::Layer, at_once);
                        kit.arrive(&slot.icon, icon_at, Beat::Layer, at_once);
                        kit.arrive(slot.title.layer(), title_at, Beat::Layer, at_once);
                        if has_body {
                            kit.arrive(slot.body.layer(), body_at, Beat::Layer, at_once);
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
        self.slots.iter().filter(|s| s.showing.is_some()).map(|s| Rect { x: s.at.0, y: s.at.1, w: WIDTH, h: self.height }).collect()
    }

    pub fn restyle(&mut self) {
        self.slots.clear();
    }
}
