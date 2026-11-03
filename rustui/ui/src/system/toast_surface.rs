//! A single compact toast at the owner's saved overlay placement. The default is centred above the TV-safe bottom edge.

use super::toast::Toast;
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::role_style;
use crate::kit::{Kit, wrap};
use compositor_protocol::{Props, Rect};
use lumatv_placement::{Frame as PlacementFrame, Rect as PlacedRect, Registry, Size, User, above_subtitles, model::STACK, stack_beside};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::components::{Align, text::{Run, draw_runs, line_height}};
use ui::geometry::Frame;
use ui::scene::ImageLayer;

const MIN_WIDTH: f32 = 320.0;
const MAX_WIDTH: f32 = 480.0;
const MIN_HEIGHT: f32 = 64.0;
const ICON: u32 = 24;
const RISE: f32 = 24.0;

struct Card {
    pane: Pane,
    icon: ImageLayer,
    words: ImageLayer,
    id: u64,
    at: (f32, f32),
    size: (f32, f32),
    visible: bool,
}

pub struct ToastSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    card: Option<Card>,
    generation: u64,
    text_scale: f32,
}

impl ToastSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, card: None, generation: 0, text_scale: 1.0 }
    }

    /// The compact single-line stand-in used by the placement editor.
    pub fn sample_height(&self, kit: &Kit) -> f32 {
        (kit.line(TypeRole::Caption).max(ICON as f32) + 2.0 * kit.space(4)).max(MIN_HEIGHT).ceil()
    }

    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    fn build(&mut self, kit: &mut Kit, toast: &Toast) -> std::io::Result<()> {
        let pad = kit.space(4);
        let gap = kit.space(3);
        let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink(), Align::Left);
        let line = line_height(style.ty);
        let text = if toast.body.is_empty() { toast.title.clone() } else { format!("{}\n{}", toast.title, toast.body) };
        let overhead = 2.0 * pad + ICON as f32 + gap;
        let widest = text.split('\n').map(|s| kit.ui.fonts.measure(s, style.ty).width).fold(0.0, f32::max);
        let width = (widest + overhead + 2.0).ceil().clamp(MIN_WIDTH, MAX_WIDTH);
        let text_width = width - overhead;
        let room = self.registry.zone_of("toast", &self.frame).h as f32;
        let max_lines = (((room - 2.0 * pad) / line).floor() as usize).max(1);
        let lines = wrap::lines(kit, style, &text, text_width - 2.0, max_lines);
        let text_height = line * lines.len() as f32;
        let height = (text_height.max(ICON as f32) + 2.0 * pad).max(MIN_HEIGHT).ceil();
        let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
        let pane = Pane::new(kit, Frame::new(0.0, 0.0, width, height), self.z, PaneKind::Card, false);
        pane.layer().set(hidden);
        let mut icon = kit.ui.scene.image(ICON, ICON, self.z + 1, hidden)?;
        glyph::draw("info", ICON, kit.ui.look.accent(), icon.pixels_mut());
        icon.damage(None);
        let mut words = kit.ui.scene.image(text_width.ceil() as u32, text_height.ceil() as u32, self.z + 2, hidden)?;
        let runs: Vec<Run<'_>> = lines.iter().enumerate().map(|(i, s)| Run::left(s, style.ty, style.colour, 0.0, i as f32 * line, text_width)).collect();
        draw_runs(&mut kit.ui, &mut words, &runs);
        self.card = Some(Card { pane, icon, words, id: toast.id, at: (0.0, 0.0), size: (width, height), visible: false });
        self.generation = kit.generation();
        self.text_scale = kit.text_scale;
        Ok(())
    }

    /// Draws the current toast, clearing the volume bar and subtitles using the placement rules.
    pub fn apply(&mut self, kit: &mut Kit, toasts: &[Toast], bar: Option<Rect>, over_app: bool) -> std::io::Result<bool> {
        let Some(toast) = toasts.first() else {
            if let Some(card) = self.card.as_mut().filter(|c| c.visible) {
                kit.leave(card.pane.layer(), card.pane.at(), Beat::Layer);
                let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
                card.icon.set(hidden);
                card.words.set(hidden);
                card.visible = false;
            }
            return Ok(false);
        };
        let fresh = self.card.as_ref().is_none_or(|c| c.id != toast.id || !c.visible)
            || self.generation != kit.generation() || self.text_scale != kit.text_scale;
        if fresh {
            self.card = None;
            self.build(kit, toast)?;
        }
        let card = self.card.as_mut().unwrap();
        let (width, height) = card.size;
        let down = self.registry.placement_of("toast", &self.user).is_none_or(|p| p.placement.grows_down());
        let bar = bar.map(|b| PlacedRect::new(b.x.round() as i32, b.y.round() as i32, b.w.round() as i32, b.h.round() as i32));
        let rect = self.registry.rect_of("toast", &self.user, &self.frame, Size::new(width as i32, height as i32));
        let rect = rect.map(|r| if over_app { above_subtitles(r, STACK) } else { r });
        let rect = rect.map(|r| stack_beside(bar.as_ref(), r, STACK, down));
        let (x, y) = rect.map_or(((1920.0 - width) / 2.0, 1026.0 - height), |r| (r.x as f32, r.y as f32));
        if fresh || card.at != (x, y) {
            let from = self.registry.slide_of("toast", &self.user);
            let travel = if kit.ui.look.reduced_motion { 0.0 } else { RISE };
            let delta = (from.x as f32 * travel, from.y as f32 * travel);
            let pad = kit.space(4);
            let gap = kit.space(3);
            let text_height = card.words.height() as f32;
            let pane_at = Props::at(x, y);
            let icon_at = Props::at(x + pad, y + ((height - ICON as f32) / 2.0).round());
            let words_at = Props::at(x + pad + ICON as f32 + gap, y + ((height - text_height) / 2.0).round());
            let move_layer = |layer: &ui::scene::Layer, at: Props| {
                if fresh { kit.arrive_from(layer, at, delta, Beat::Layer) } else { kit.glide(layer, at, Beat::Layer) }
            };
            move_layer(card.pane.layer(), pane_at);
            move_layer(&card.icon, icon_at);
            move_layer(&card.words, words_at);
            card.pane.move_to(x, y);
            card.at = (x, y);
            card.visible = true;
        }
        Ok(true)
    }

    /// The current toast rectangle, if visible.
    pub fn rects(&self) -> Vec<Rect> {
        self.card.as_ref().filter(|c| c.visible).map(|c| vec![Rect { x: c.at.0, y: c.at.1, w: c.size.0, h: c.size.1 }]).unwrap_or_default()
    }

    pub fn restyle(&mut self) {
        self.card = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use serde_json::json;

    fn toast(title: &str, body: &str) -> Toast {
        Toast { id: 1, title: title.into(), body: body.into(), icon: String::new(), expires: 5000 }
    }

    #[test]
    fn compact_default_and_adaptive_multiline_stay_in_safe_area() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            let mut surface = ToastSurface::new(1900, &json!({}), &json!({}));
            surface.apply(&mut kit, &[toast("Ready", "")], None, false).unwrap();
            let short = surface.rects()[0];
            assert_eq!(short.w, 320.0);
            assert!(short.h >= 64.0);
            assert_eq!(short.x + short.w / 2.0, 960.0);
            assert_eq!(short.y + short.h, 1026.0);
            let mut long = toast("A much longer notification title that wraps", "Another line with meaningful detail and a lengthy continuation.");
            long.id = 2;
            surface.apply(&mut kit, &[long], None, false).unwrap();
            let r = surface.rects()[0];
            assert!(r.w > short.w && r.w <= 480.0);
            assert!(r.h > short.h);
            assert!(r.y >= 24.0 && r.y + r.h == 1026.0);
        }
    }

    #[test]
    fn saved_anchor_and_clearance_are_preserved() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut surface = ToastSurface::new(1900, &json!({}), &json!({}));
        let t = toast("Ready", "");
        let bar = Rect { x: 740.0, y: 988.0, w: 440.0, h: 60.0 };
        surface.apply(&mut kit, &[t.clone()], Some(bar), false).unwrap();
        let r = surface.rects()[0];
        assert_eq!(r.y + r.h, bar.y - 16.0);
        surface.apply(&mut kit, &[t.clone()], None, true).unwrap();
        assert_eq!(surface.rects()[0].y + surface.rects()[0].h, 889.0);
        surface.apply(&mut kit, &[], None, false).unwrap();
        assert!(surface.rects().is_empty());
        let theme = json!({"layout": {"overlays": {"toast": {"anchor": "top-right", "dx": 0.0, "dy": 0.0}}}});
        let mut top = ToastSurface::new(1900, &theme, &json!({}));
        top.apply(&mut kit, &[t], None, false).unwrap();
        assert_eq!(top.rects()[0].y, 24.0);
    }
}
