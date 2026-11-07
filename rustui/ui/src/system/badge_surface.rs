//! The format badge drawn: one pill of the theme's glass with the picture's marks side by side in the look's ink, at the owner's place for
//! `format` (the top right of the content zone, under the status bar's row, by default). It steps aside from what is up (the pop-up, the cards,
//! the volume bar and the mute mark, a toast) and stays down while there is no room clear of them.

use super::badge::Badge;
use crate::kit::Kit;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use compositor_protocol::{BackdropId, Props, Rect};
use lumatv_placement::{Frame as PlacementFrame, Rect as PlacedRect, Registry, Size, User, above_subtitles, model::{GAP, STACK}, step_aside};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::geometry::Frame;
use ui::scene::ImageLayer;

/// How far the badge travels in from its edge.
const TRAVEL: f32 = 24.0;
/// The volume bar's least height: the two pills are one family.
const MIN_HEIGHT: f32 = 56.0;
/// What the placement editor's stand-in shows: a 4K Dolby Vision film with Atmos.
pub const SAMPLE: [&str; 3] = ["uhd", "dolby-vision", "atmos"];

/// The marks' height, the pane's side pads, the gap between marks and the pane's height, all whole pixels.
fn metrics(kit: &Kit) -> (u32, f32, f32, f32) {
    let mark_h = (kit.ty(TypeRole::Label).px as f32 * 1.08).round();
    let h = (kit.line(TypeRole::Label).max(mark_h).ceil() + 2.0 * kit.space(3)).max(MIN_HEIGHT);
    (mark_h as u32, kit.space(5), kit.space(4), h)
}

/// The pane's size for `marks` (the ones the kit has art for).
pub fn measure(kit: &Kit, marks: &[&str]) -> (f32, f32) {
    let (mark_h, pad_x, gap, h) = metrics(kit);
    let arts: Vec<u32> = marks.iter().filter_map(|m| kit.marks.iter().find(|a| a.name == *m)).map(|a| a.width_at(mark_h)).collect();
    let row = arts.iter().sum::<u32>() as f32 + gap * arts.len().saturating_sub(1) as f32;
    (2.0 * pad_x + row, h)
}

/// The marks as one picture in the look's ink, and where it sits in the pane (whole pixels).
pub fn draw_marks(kit: &mut Kit, marks: &[&str], z: i32, props: Props) -> std::io::Result<(ImageLayer, (f32, f32))> {
    let (mark_h, pad_x, gap, h) = metrics(kit);
    let (w, _) = measure(kit, marks);
    let row_w = ((w - 2.0 * pad_x) as u32).max(1);
    let ink = kit.ui.look.ink();
    let mut layer = kit.ui.scene.image(row_w, mark_h, z, props)?;
    let stride = row_w as usize * 4;
    let px = layer.pixels_mut();
    px.fill(0);
    let mut x = 0;
    for art in marks.iter().filter_map(|m| kit.marks.iter().find(|a| a.name == *m)) {
        let (mw, mh, pixels) = art.render(mark_h, f32::MAX, ink);
        let span = mw as usize * 4;
        for row in 0..mh as usize {
            px[row * stride + x..row * stride + x + span].copy_from_slice(&pixels[row * span..(row + 1) * span]);
        }
        x += span + gap as usize * 4;
    }
    layer.damage(None);
    Ok((layer, (pad_x, ((h - mark_h as f32) / 2.0).round())))
}

/// What a badge is made of, for one set of marks in one look.
struct Built {
    key: (u64, Vec<&'static str>, u64, u32, Option<BackdropId>, bool),
    pane: Pane,
    marks: ImageLayer,
    /// The marks' corner inside the pane.
    inset: (f32, f32),
    size: (f32, f32),
    at: (f32, f32),
    visible: bool,
}

pub struct BadgeSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    built: Option<Built>,
    /// The look changed: the badge is made again in place, without coming in a second time.
    restyled: bool,
}

impl BadgeSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, built: None, restyled: false }
    }

    /// The editor's stand-in's size.
    pub fn sample_size(kit: &Kit) -> Size {
        let (w, h) = measure(kit, &SAMPLE);
        Size::new(w as i32, h as i32)
    }

    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    fn build(&self, kit: &mut Kit, badge: &Badge, over_app: bool) -> std::io::Result<Built> {
        let (w, h) = measure(kit, &badge.marks);
        let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
        let pane = Pane::themed(kit, Frame::new(0.0, 0.0, w, h), self.z, PaneKind::Pill, over_app);
        pane.layer().set(hidden);
        let (marks, inset) = draw_marks(kit, &badge.marks, self.z + 1, hidden)?;
        let key = (badge.id, badge.marks.clone(), kit.generation(), kit.text_scale.to_bits(), kit.wall_backdrop, over_app);
        Ok(Built { key, pane, marks, inset, size: (w, h), at: (0.0, 0.0), visible: false })
    }

    /// Draws `badge` clear of `up` (what is up); `over_app` is an app's picture under it. Returns whether the badge is on screen.
    pub fn apply(&mut self, kit: &mut Kit, badge: Option<&Badge>, up: &[PlacedRect], over_app: bool) -> std::io::Result<bool> {
        let Some(badge) = badge.filter(|b| !b.marks.is_empty()) else {
            self.hide(kit);
            return Ok(false);
        };
        let key = (badge.id, badge.marks.clone(), kit.generation(), kit.text_scale.to_bits(), kit.wall_backdrop, over_app);
        // the same showing made again (its marks or the look changed) is in place at once; a new one comes in
        let continuing = self.built.as_ref().is_some_and(|b| b.visible && b.key.0 == badge.id);
        if std::mem::take(&mut self.restyled) || self.built.as_ref().is_none_or(|b| b.key != key) {
            self.built = Some(self.build(kit, badge, over_app)?);
        }
        let (w, h) = self.built.as_ref().expect("built above").size;
        let zone = self.registry.zone_of("format", &self.frame);
        let rect = self.registry.rect_of("format", &self.user, &self.frame, Size::new(w as i32, h as i32));
        let rect = rect.map(|r| if over_app { above_subtitles(r, STACK) } else { r }).and_then(|r| step_aside(r, up, &zone, GAP));
        let Some(r) = rect else {
            self.hide(kit);
            return Ok(false);
        };
        let b = self.built.as_mut().expect("built above");
        let (x, y) = (r.x as f32, r.y as f32);
        let pane_at = Props::at(x, y);
        let marks_at = Props::at(x + b.inset.0, y + b.inset.1);
        if !b.visible && !continuing {
            let from = self.registry.slide_of("format", &self.user);
            let travel = if kit.ui.look.reduced_motion { 0.0 } else { TRAVEL };
            let by = (from.x as f32 * travel, from.y as f32 * travel);
            kit.arrive_from(b.pane.layer(), pane_at, by, Beat::Layer);
            kit.arrive_from(&b.marks, marks_at, by, Beat::Layer);
        } else if !b.visible {
            b.pane.layer().set(pane_at);
            b.marks.set(marks_at);
        } else if b.at != (x, y) {
            kit.glide(b.pane.layer(), pane_at, Beat::Layer);
            kit.glide(&b.marks, marks_at, Beat::Layer);
        }
        b.pane.move_to(x, y);
        b.at = (x, y);
        b.visible = true;
        Ok(true)
    }

    fn hide(&mut self, kit: &Kit) {
        if let Some(b) = self.built.as_mut().filter(|b| b.visible) {
            kit.leave(b.pane.layer(), b.pane.at(), Beat::Layer);
            kit.leave(&b.marks, Props::at(b.at.0 + b.inset.0, b.at.1 + b.inset.1), Beat::Layer);
            b.visible = false;
        }
    }

    /// The badge's rectangle while it is up.
    pub fn rects(&self) -> Vec<Rect> {
        self.built.as_ref().filter(|b| b.visible).map(|b| vec![Rect { x: b.at.0, y: b.at.1, w: b.size.0, h: b.size.1 }]).unwrap_or_default()
    }

    pub fn restyle(&mut self) {
        self.restyled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::logo::{self, VIDEO_MARKS};
    use crate::kit::pane::glass_of;
    use crate::kit::testkit;
    use compositor_protocol::Content;
    use lumatv_tokens::CardGlass;
    use serde_json::json;
    use ui::lucent::palette::Mode;

    const Z: i32 = 1900;

    fn with_marks(mode: Mode) -> (ui::testing::Harness, Kit) {
        let (h, mut kit) = testkit::kit_in(mode, 1.0);
        kit.marks = logo::load_all(&testkit::module_dir().join("icons/video"), &VIDEO_MARKS);
        (h, kit)
    }

    fn badge(id: u64) -> Badge {
        Badge { id, marks: SAMPLE.to_vec(), since: 0, expires: 5000 }
    }

    fn meets(a: &Rect, b: &PlacedRect) -> bool {
        a.x < (b.x + b.w + GAP) as f32 && a.x + a.w + GAP as f32 > b.x as f32 && a.y < (b.y + b.h + GAP) as f32 && a.y + a.h + GAP as f32 > b.y as f32
    }

    #[test]
    fn the_badge_sits_top_right_under_the_status_bar_and_steps_aside_from_what_is_up() {
        let (_h, mut kit) = with_marks(Mode::Dark);
        let mut s = BadgeSurface::new(Z, &json!({}), &json!({}));
        assert!(s.apply(&mut kit, Some(&badge(1)), &[], false).unwrap());
        let r = s.rects()[0];
        let zone = s.registry.zone_of("format", &s.frame);
        assert_eq!((r.x + r.w, r.y), ((zone.x + zone.w) as f32, zone.y as f32), "flush with the content zone's top right");
        assert!(r.y >= 134.0, "below the status bar's row");
        assert_eq!([r.x, r.y, r.w, r.h].map(f32::fract), [0.0; 4], "whole pixels");
        // the pop-up at its corner and the volume bar under the badge: it moves to the nearest place clear of both, inside its zone
        let popup = PlacedRect::new(1888 - 560, 32, 560, 469);
        let volume = PlacedRect::new(r.x as i32 - 200, r.y as i32 + 10, 440, 60);
        for up in [vec![popup], vec![volume], vec![popup, volume]] {
            assert!(s.apply(&mut kit, Some(&badge(1)), &up, false).unwrap());
            let moved = s.rects()[0];
            assert!(up.iter().all(|o| !meets(&moved, o)), "{moved:?} clear of {up:?}");
            assert!(moved.x >= zone.x as f32 && moved.y >= zone.y as f32 && moved.x + moved.w <= (zone.x + zone.w) as f32 && moved.y + moved.h <= (zone.y + zone.h) as f32);
        }
        // nowhere clear of what is up: the badge stays down
        assert!(!s.apply(&mut kit, Some(&badge(1)), &[zone], false).unwrap());
        assert!(s.rects().is_empty());
        assert!(s.apply(&mut kit, None, &[], false).is_ok_and(|on| !on));
    }

    #[test]
    fn the_badge_is_the_themes_glass_and_its_marks_the_looks_ink_in_either_mode() {
        let mut inks = Vec::new();
        for mode in [Mode::Dark, Mode::Light] {
            for card_glass in [CardGlass::Liquid, CardGlass::Frosted] {
                let (mut h, mut kit) = with_marks(mode);
                kit.tokens.card_glass = card_glass;
                let mut s = BadgeSurface::new(Z, &json!({}), &json!({}));
                assert!(s.apply(&mut kit, Some(&badge(1)), &[], false).unwrap());
                h.commit();
                let b = s.built.as_ref().unwrap();
                let glass = h.layers().into_iter().find(|l| l.0 == b.pane.layer().id()).map(|l| l.1);
                assert_eq!(glass, Some(Content::Glass(glass_of(&kit, PaneKind::Pill, b.size.1, false, false))), "{mode:?} {card_glass:?}");
                let ink = kit.ui.look.ink();
                let px = b.marks.pixels();
                assert!(px.chunks_exact(4).any(|p| p == [ink.0, ink.1, ink.2, ink.3]), "{mode:?}: solid ink");
                assert!(px.chunks_exact(4).all(|p| p[0] <= ink.0 && p[1] <= ink.1 && p[2] <= ink.2 && p[3] <= ink.3), "{mode:?}: the ink alone, by coverage");
                inks.push(ink);
            }
        }
        assert_ne!(inks[0], inks[2], "the light look's ink is its own");
    }
}
