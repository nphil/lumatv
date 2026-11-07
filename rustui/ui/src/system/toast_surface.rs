//! A single compact toast at the owner's saved overlay placement, on the theme's glass with its lines as glyph runs on whole pixels. The
//! default is centred above the TV-safe bottom edge.

use super::toast::Toast;
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style};
use crate::kit::{Kit, wrap};
use compositor_protocol::{BackdropId, Props, Rect};
use lumatv_placement::{Frame as PlacementFrame, Rect as PlacedRect, Registry, Size, User, above_subtitles, model::STACK, stack_beside};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::components::{Align, text::line_height};
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
    lines: Vec<Text>,
    line: f32,
    id: u64,
    /// The look and what is under it when it was made (generation, text size, Home's wallpaper blur, over an app): another makes it again.
    made: (u64, u32, Option<BackdropId>, bool),
    at: (f32, f32),
    size: (f32, f32),
    visible: bool,
}

impl Card {
    /// Where the icon and each line rest with the pane's corner at (`x`, `y`): whole pixels.
    fn spots(&self, kit: &Kit, (x, y): (f32, f32)) -> (Props, Vec<Props>) {
        let (pad, gap, height) = (kit.space(4), kit.space(3), self.size.1);
        let words_x = x + pad + ICON as f32 + gap;
        let words_y = y + ((height - self.line * self.lines.len() as f32) / 2.0).round();
        let icon = Props::at(x + pad, y + ((height - ICON as f32) / 2.0).round());
        (icon, (0..self.lines.len()).map(|i| Props::at(words_x, words_y + i as f32 * self.line)).collect())
    }
}

pub struct ToastSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    card: Option<Card>,
    /// The look changed: the toast is made again in place, without coming in a second time.
    restyled: bool,
}

impl ToastSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, card: None, restyled: false }
    }

    /// The compact single-line stand-in used by the placement editor.
    pub fn sample_height(&self, kit: &Kit) -> f32 {
        (kit.line(TypeRole::Caption).max(ICON as f32) + 2.0 * kit.space(4)).max(MIN_HEIGHT).ceil()
    }

    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    fn made(kit: &Kit, over_app: bool) -> (u64, u32, Option<BackdropId>, bool) {
        (kit.generation(), kit.text_scale.to_bits(), kit.wall_backdrop, over_app)
    }

    fn build(&mut self, kit: &mut Kit, toast: &Toast, over_app: bool) -> std::io::Result<()> {
        let pad = kit.space(4);
        let gap = kit.space(3);
        let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink(), Align::Left);
        let line = line_height(style.ty).round();
        let text = if toast.body.is_empty() { toast.title.clone() } else { format!("{}\n{}", toast.title, toast.body) };
        let overhead = 2.0 * pad + ICON as f32 + gap;
        let widest = text.split('\n').map(|s| kit.ui.fonts.measure(s, style.ty).width).fold(0.0, f32::max);
        let width = (widest + overhead + 2.0).ceil().clamp(MIN_WIDTH, MAX_WIDTH);
        let text_width = width - overhead;
        let room = self.registry.zone_of("toast", &self.frame).h as f32;
        let max_lines = (((room - 2.0 * pad) / line).floor() as usize).max(1);
        let wrapped = wrap::lines(kit, style, &text, text_width - 2.0, max_lines);
        let text_height = line * wrapped.len() as f32;
        let height = (text_height.max(ICON as f32) + 2.0 * pad).max(MIN_HEIGHT).ceil();
        let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
        let pane = Pane::themed(kit, Frame::new(0.0, 0.0, width, height), self.z, PaneKind::Card, over_app);
        pane.layer().set(hidden);
        let mut icon = kit.ui.scene.image(ICON, ICON, self.z + 1, hidden)?;
        glyph::draw("info", ICON, kit.ui.look.accent(), icon.pixels_mut());
        icon.damage(None);
        let mut lines = Vec::with_capacity(wrapped.len());
        for s in &wrapped {
            let mut t = Text::new(kit, Frame::new(0.0, 0.0, text_width, line), self.z + 2, style, s)?;
            t.set_props(hidden);
            lines.push(t);
        }
        let made = Self::made(kit, over_app);
        self.card = Some(Card { pane, icon, lines, line, id: toast.id, made, at: (0.0, 0.0), size: (width, height), visible: false });
        Ok(())
    }

    /// Draws the current toast, clearing the volume bar and subtitles using the placement rules.
    pub fn apply(&mut self, kit: &mut Kit, toasts: &[Toast], bar: Option<Rect>, over_app: bool) -> std::io::Result<bool> {
        let Some(toast) = toasts.first() else {
            if let Some(card) = self.card.as_mut().filter(|c| c.visible) {
                let (icon, lines) = card.spots(kit, card.at);
                kit.leave(card.pane.layer(), card.pane.at(), Beat::Layer);
                kit.leave(&card.icon, icon, Beat::Layer);
                for (t, at) in card.lines.iter().zip(lines) {
                    kit.leave(t.layer(), at, Beat::Layer);
                }
                card.visible = false;
            }
            return Ok(false);
        };
        // the toast that is up made again (the look or what is under it changed) is in place at once; a new one comes in
        let continuing = self.card.as_ref().is_some_and(|c| c.visible && c.id == toast.id);
        let stale = self.card.as_ref().is_none_or(|c| c.id != toast.id || !c.visible || c.made != Self::made(kit, over_app));
        if std::mem::take(&mut self.restyled) || stale {
            self.card = None;
            self.build(kit, toast, over_app)?;
        }
        let card = self.card.as_mut().expect("built above");
        let (width, height) = card.size;
        let down = self.registry.placement_of("toast", &self.user).is_none_or(|p| p.placement.grows_down());
        let bar = bar.map(|b| PlacedRect::new(b.x.round() as i32, b.y.round() as i32, b.w.round() as i32, b.h.round() as i32));
        let rect = self.registry.rect_of("toast", &self.user, &self.frame, Size::new(width as i32, height as i32));
        let rect = rect.map(|r| if over_app { above_subtitles(r, STACK) } else { r });
        let rect = rect.map(|r| stack_beside(bar.as_ref(), r, STACK, down));
        let (x, y) = rect.map_or((((1920.0 - width) / 2.0).round(), 1026.0 - height), |r| (r.x as f32, r.y as f32));
        if !card.visible || card.at != (x, y) {
            let from = self.registry.slide_of("toast", &self.user);
            let travel = if kit.ui.look.reduced_motion { 0.0 } else { RISE };
            let delta = (from.x as f32 * travel, from.y as f32 * travel);
            let (icon, lines) = card.spots(kit, (x, y));
            let (arrive, at_once) = (!card.visible && !continuing, !card.visible);
            let place = |layer: &ui::scene::Layer, at: Props| {
                if arrive { kit.arrive_from(layer, at, delta, Beat::Layer) } else if at_once { layer.set(at) } else { kit.glide(layer, at, Beat::Layer) }
            };
            place(card.pane.layer(), Props::at(x, y));
            place(&card.icon, icon);
            for (t, at) in card.lines.iter().zip(lines) {
                place(t.layer(), at);
            }
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
        self.restyled = true;
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
    fn the_toast_is_the_themes_glass_with_glyph_run_lines_on_whole_pixels_and_a_new_look_keeps_it_in_place() {
        let (mut h, mut kit) = testkit::kit(1.0);
        // the warmed glyphs on the compositor, as on a set that has drawn a page
        h.settle_atlas();
        let mut surface = ToastSurface::new(1900, &json!({}), &json!({}));
        let t = toast("A much longer notification title that wraps", "Another line with meaningful detail and a lengthy continuation.");
        surface.apply(&mut kit, std::slice::from_ref(&t), None, false).unwrap();
        h.commit();
        let card = surface.card.as_ref().unwrap();
        let glass = h.layers().into_iter().find(|l| l.0 == card.pane.layer().id()).map(|l| l.1);
        assert_eq!(glass, Some(compositor_protocol::Content::Glass(crate::kit::pane::glass_of(&kit, PaneKind::Card, card.size.1, false, false))));
        let runs: Vec<bool> = card.lines.iter().map(Text::is_glyph_run).collect();
        assert!(runs.len() > 1 && runs.iter().all(|r| *r), "every line glyph quads: {runs:?} {:?}", card.lines.iter().map(Text::text).collect::<Vec<_>>());
        let (icon, lines) = card.spots(&kit, card.at);
        for p in std::iter::once(icon).chain(lines) {
            assert_eq!([p.x.fract(), p.y.fract()], [0.0, 0.0], "{p:?}");
        }
        let at = surface.rects();
        surface.restyle();
        surface.apply(&mut kit, &[t], None, false).unwrap();
        assert_eq!(surface.rects(), at);
        let pane = surface.card.as_ref().unwrap().pane.layer().id();
        let rest = h.commit().into_iter().any(|r| matches!(r, compositor_protocol::Request::Set { id, props } if id == pane && props.opacity == 1.0));
        assert!(rest, "made again, the toast is set where it was rather than coming in again");
    }

    #[test]
    fn saved_anchor_and_clearance_are_preserved() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut surface = ToastSurface::new(1900, &json!({}), &json!({}));
        let t = toast("Ready", "");
        let bar = Rect { x: 740.0, y: 988.0, w: 440.0, h: 60.0 };
        surface.apply(&mut kit, std::slice::from_ref(&t), Some(bar), false).unwrap();
        let r = surface.rects()[0];
        assert_eq!(r.y + r.h, bar.y - 16.0);
        surface.apply(&mut kit, std::slice::from_ref(&t), None, true).unwrap();
        assert_eq!(surface.rects()[0].y + surface.rects()[0].h, 889.0);
        surface.apply(&mut kit, &[], None, false).unwrap();
        assert!(surface.rects().is_empty());
        let theme = json!({"layout": {"overlays": {"toast": {"anchor": "top-right", "dx": 0.0, "dy": 0.0}}}});
        let mut top = ToastSurface::new(1900, &theme, &json!({}));
        top.apply(&mut kit, &[t], None, false).unwrap();
        assert_eq!(top.rects()[0].y, 24.0);
    }
}
