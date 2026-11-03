//! Retained, palette-owned miniature pictures for theme cards.

use std::cell::Cell;
use std::rc::Rc;

use compositor_protocol::{Props, Rgba};
use lumatv_tokens::themes;
use ui::group::Group;
use ui::lucent::palette::{Mode, PaletteRole, Roles};
use ui::scene::{ImageLayer, Scene};

const SIDE: u32 = 64;
const LIMIT: usize = 40;
const HIDDEN: Props = Props { visible: false, opacity: 0.0, ..Props::at(0.0, 0.0) };

struct Entry {
    marker: String,
    layer: ImageLayer,
    serial: Cell<u64>,
}

/// A lease moves the retained picture into the current scroll group. An older
/// row can be dropped after its replacement without hiding the replacement.
pub(super) struct Lease {
    entry: Rc<Entry>,
    serial: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.entry.serial.get() == self.serial {
            self.entry.layer.detach(HIDDEN);
            self.entry.layer.set(HIDDEN);
        }
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    entries: Vec<Rc<Entry>>,
}

impl Cache {
    /// A panel can reuse layers held only by the idle thumbnail cache.
    pub(crate) fn release_unleased(&mut self) {
        self.entries.retain(|entry| Rc::strong_count(entry) > 1);
    }

    pub(super) fn acquire(&mut self, scene: &Scene, scroll: &Group, marker: &str, z: i32, at: Props, side: u32) -> std::io::Result<Option<Lease>> {
        let Some(rest) = marker.strip_prefix("theme:") else { return Ok(None) };
        let Some((mode, id)) = rest.split_once(':') else { return Ok(None) };
        let mode = match mode {
            "light" => Mode::Light,
            "dark" => Mode::Dark,
            _ => return Ok(None),
        };
        let Some(family) = themes::find(id) else { return Ok(None) };
        let entry = if let Some(entry) = self.entries.iter().find(|entry| entry.marker == marker) {
            Rc::clone(entry)
        } else if let Some(slot) = self.entries.iter_mut().find(|entry| Rc::strong_count(entry) == 1) {
            // Only an unleased image may be repainted: old rows can outlive their replacements.
            let entry = Rc::get_mut(slot).expect("unleased thumbnail");
            entry.marker = marker.to_owned();
            entry.layer.clear();
            raster(entry.layer.pixels_mut(), family.roles(mode));
            entry.layer.damage(None);
            Rc::clone(slot)
        } else {
            if self.entries.len() >= LIMIT {
                return Err(std::io::Error::other("theme thumbnail cache is fully leased"));
            }
            let mut layer = scene.image(SIDE, SIDE, z, HIDDEN)?;
            raster(layer.pixels_mut(), family.roles(mode));
            layer.damage(None);
            let entry = Rc::new(Entry { marker: marker.to_owned(), layer, serial: Cell::new(0) });
            self.entries.push(Rc::clone(&entry));
            entry
        };
        let serial = entry.serial.get().wrapping_add(1);
        let scale = side as f32 / SIDE as f32;
        // Scaling is about the image centre, not its top-left corner.
        let inset = (SIDE as f32 - side as f32) / 2.0;
        let at = Props { x: at.x - inset, y: at.y - inset, scale, ..at };
        scroll.attach(&entry.layer, at).map_err(super::other)?;
        entry.serial.set(serial);
        Ok(Some(Lease { entry, serial }))
    }
}

fn over(src: Rgba, dst: Rgba) -> Rgba {
    let inverse = 255 - u16::from(src.3);
    let channel = |s: u8, d: u8| (u16::from(s) + u16::from(d) * inverse / 255).min(255) as u8;
    Rgba(channel(src.0, dst.0), channel(src.1, dst.1), channel(src.2, dst.2), channel(src.3, dst.3))
}

fn raster(pixels: &mut [u8], roles: &Roles) {
    // The miniature owns its background; translucent roles must not sample the gallery behind it.
    let canvas = roles[PaletteRole::Canvas].premultiply(1.0);
    let sheet = over(roles[PaletteRole::Sheet].premultiply(0.0), canvas);
    let ink = over(roles[PaletteRole::Ink].premultiply(0.0), sheet);
    let ink2 = over(roles[PaletteRole::Ink2].premultiply(0.0), sheet);
    let accent = over(roles[PaletteRole::Accent].premultiply(0.0), sheet);
    let set = |pixels: &mut [u8], x: usize, y: usize, c: Rgba| {
        let i = (y * SIDE as usize + x) * 4;
        pixels[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, c.3]);
    };
    for y in 0..SIDE as usize {
        for x in 0..SIDE as usize {
            // Rounded outer corners, with a miniature settings panel inside.
            let dx = if x < 7 { 7 - x } else { x.saturating_sub(56) };
            let dy = if y < 7 { 7 - y } else { y.saturating_sub(56) };
            if dx * dx + dy * dy > 49 { continue; }
            let c = if (10..54).contains(&x) && (12..52).contains(&y) {
                if (16..48).contains(&x) && (18..21).contains(&y) { ink }
                else if (16..38).contains(&x) && (26..28).contains(&y) { ink2 }
                else if (16..44).contains(&x) && (33..35).contains(&y) { ink2 }
                else if (16..32).contains(&x) && (41..46).contains(&y) { accent }
                else { sheet }
            } else { canvas };
            set(pixels, x, y, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::lucent::palette::Straight;

    #[test]
    fn translucent_roles_are_composited_over_their_own_canvas() {
        let mut colours = [Straight::new(0, 0, 0, 255); 20];
        colours[PaletteRole::Canvas as usize] = Straight::new(20, 40, 60, 255);
        colours[PaletteRole::Sheet as usize] = Straight::new(200, 100, 0, 128);
        colours[PaletteRole::Ink as usize] = Straight::new(255, 255, 255, 128);
        let roles = Roles::new(colours);
        let mut pixels = vec![0; (SIDE * SIDE * 4) as usize];
        raster(&mut pixels, &roles);
        let pixel = |x: usize, y: usize| &pixels[(y * SIDE as usize + x) * 4..][..4];
        assert_eq!(pixel(0, 0), &[0, 0, 0, 0]);
        assert_eq!(pixel(8, 8), &[20, 40, 60, 255]);
        assert_eq!(pixel(12, 14), &[109, 69, 29, 255]);
        assert_eq!(pixel(20, 19), &[182, 162, 142, 255]);
    }

    #[test]
    fn gallery_focus_ink_contrasts_with_card_and_sheet_in_every_palette() {
        use crate::kit::{button::faded, lint::contrast_on, testkit};
        for mode in [Mode::Light, Mode::Dark] {
            let (_, kit) = testkit::kit_in(mode, 1.0);
            for family in themes::catalog() {
                let roles = family.roles(mode);
                let canvas = roles[PaletteRole::Canvas].premultiply(1.0);
                let sheet = over(roles[PaletteRole::Sheet].premultiply(0.0), canvas);
                let ink = roles[PaletteRole::Ink].premultiply(0.0);
                let hover = faded(ink, kit.tokens.states.hover_wash);
                let focus = faded(ink, kit.tokens.states.focus_wash[usize::from(mode == Mode::Light)]);
                let card = over(focus, over(hover, sheet));
                for adjacent in [sheet, card] {
                    let bg = [f32::from(adjacent.0), f32::from(adjacent.1), f32::from(adjacent.2)];
                    assert!(contrast_on(ink, Rgba(0, 0, 0, 0), bg) >= 3.0, "{} {mode:?}", family.id);
                }
            }
        }
    }

    #[test]
    fn scrolling_both_modes_recycles_only_unleased_images() {
        let h = ui::testing::harness();
        let scene = &h.ui.scene;
        let scroll = scene.group(None, compositor_protocol::GroupPose::IDENTITY).unwrap();
        let mut cache = Cache::default();
        let mut visible = std::collections::VecDeque::new();
        for (name, mode) in [("light", Mode::Light), ("dark", Mode::Dark), ("light", Mode::Light), ("dark", Mode::Dark)] {
            for family in themes::catalog() {
                if visible.len() == 6 { visible.pop_front(); }
                let marker = format!("theme:{name}:{}", family.id);
                let lease = cache.acquire(scene, &scroll, &marker, 1, Props::at(0.0, 0.0), 32).unwrap().unwrap();
                let mut expected = vec![0; (SIDE * SIDE * 4) as usize];
                raster(&mut expected, family.roles(mode));
                assert_eq!(lease.entry.layer.pixels(), expected, "{marker}");
                assert!(lease.entry.layer.group().is_some(), "{marker}");
                visible.push_back(lease);
                assert!(cache.entries.len() <= 6);
                assert!(!scene.take_overflow());
            }
        }
    }

    #[test]
    fn releasing_idle_images_keeps_visible_leases() {
        let h = ui::testing::harness();
        let scene = &h.ui.scene;
        let scroll = scene.group(None, compositor_protocol::GroupPose::IDENTITY).unwrap();
        let mut cache = Cache::default();
        let baseline = scene.live_layers();
        let first = format!("theme:dark:{}", themes::catalog()[0].id);
        let second = format!("theme:dark:{}", themes::catalog()[1].id);
        let visible = cache.acquire(scene, &scroll, &first, 1, Props::at(0.0, 0.0), 32).unwrap().unwrap();
        let idle = cache.acquire(scene, &scroll, &second, 1, Props::at(0.0, 0.0), 32).unwrap().unwrap();
        drop(idle);
        cache.release_unleased();
        assert_eq!(scene.live_layers(), baseline + 1);
        assert!(visible.entry.layer.group().is_some());
        drop(visible);
        cache.release_unleased();
        assert_eq!(scene.live_layers(), baseline);
    }

    #[test]
    fn full_gallery_mode_switch_reuses_all_inactive_layers() {
        let h = ui::testing::harness();
        let scene = &h.ui.scene;
        let scroll = scene.group(None, compositor_protocol::GroupPose::IDENTITY).unwrap();
        let mut cache = Cache::default();
        let baseline = scene.live_layers();
        for (name, mode) in [("light", Mode::Light), ("dark", Mode::Dark), ("light", Mode::Light)] {
            let mut visible = Vec::new();
            for family in themes::catalog() {
                let marker = format!("theme:{name}:{}", family.id);
                let lease = cache.acquire(scene, &scroll, &marker, 1, Props::at(0.0, 0.0), 32).unwrap().unwrap();
                let mut expected = vec![0; (SIDE * SIDE * 4) as usize];
                raster(&mut expected, family.roles(mode));
                assert_eq!(lease.entry.layer.pixels(), expected, "{marker}");
                visible.push(lease);
            }
            assert_eq!(cache.entries.len(), themes::catalog().len());
            assert_eq!(scene.live_layers(), baseline + themes::catalog().len());
            assert!(!scene.take_overflow());
            drop(visible);
        }
    }

    #[test]
    fn replacing_a_row_does_not_hide_its_new_lease() {
        let h = ui::testing::harness();
        let scene = &h.ui.scene;
        let scroll = scene.group(None, compositor_protocol::GroupPose::IDENTITY).unwrap();
        let marker = format!("theme:dark:{}", themes::catalog()[0].id);
        let mut cache = Cache::default();
        let old = cache.acquire(scene, &scroll, &marker, 1, Props::at(0.0, 0.0), 32).unwrap().unwrap();
        let new = cache.acquire(scene, &scroll, &marker, 1, Props::at(10.0, 10.0), 32).unwrap().unwrap();
        assert!(Rc::ptr_eq(&old.entry, &new.entry));
        drop(old);
        assert!(new.entry.layer.group().is_some());
        drop(new);
        assert!(cache.entries[0].layer.group().is_none());
    }
}
