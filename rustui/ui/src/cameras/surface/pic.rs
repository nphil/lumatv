//! The pictures of the Cameras screens as the surfaces keep them: the decoded stills and thumbnails by key ([`PictureStore`]), and the layer
//! a picture is shown in ([`PicLayer`]).
//!
//! A picture is shown at the size it came in and **scaled by the compositor** (the layer's `scale`, about its centre), never resampled
//! here: a tile's still arrives as a BMP a few hundred pixels wide, and scaling 40 of them a second on the CPU would cost more than the
//! screen is worth. What the CPU does is cut the picture to the box's shape when the box is to be covered (a portrait camera in a 16:9
//! tile), copy it into the layer's pixels and round the corners at the size the picture will have on the screen, so the arc comes out at the
//! theme's radius after the compositor's scale.

use crate::kit::Kit;
use crate::kit::picture::Picture;
use compositor_protocol::{Props, Rect};
use std::collections::HashMap;
use ui::geometry::Frame;
use ui::group::Group;
use ui::scene::ImageLayer;

struct Stored {
    pic: Picture,
    rev: u64,
    /// The order the pictures arrived in, for the oldest to go first.
    seq: u64,
}

/// The decoded pictures by key; a key's `rev` changes each time a new picture arrives.
#[derive(Default)]
pub struct PictureStore {
    map: HashMap<String, Stored>,
    seq: u64,
}

/// How many pictures of each kind are kept: the grid paints the last picture of every camera (16) in its first frame.
const KEEP: [(&str, usize); 4] = [("tile:", 16), ("full:", 2), ("bubble:", 4), ("card:", 40)];

impl PictureStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps `pic` under `key`; returns the key's new revision.
    pub fn put(&mut self, key: &str, pic: Picture) -> u64 {
        self.seq += 1;
        let rev = self.map.get(key).map_or(1, |s| s.rev + 1);
        self.map.insert(key.to_string(), Stored { pic, rev, seq: self.seq });
        self.trim(key);
        rev
    }

    /// Lets the oldest of `key`'s kind go when there are more than the kind keeps.
    fn trim(&mut self, key: &str) {
        let Some((prefix, keep)) = KEEP.iter().find(|(p, _)| key.starts_with(p)) else { return };
        let mut of_kind: Vec<(&String, u64)> = self.map.iter().filter(|(k, _)| k.starts_with(prefix)).map(|(k, s)| (k, s.seq)).collect();
        if of_kind.len() <= *keep {
            return;
        }
        of_kind.sort_by_key(|(_, seq)| *seq);
        let drop: Vec<String> = of_kind.iter().take(of_kind.len() - keep).map(|(k, _)| (*k).clone()).collect();
        for k in drop {
            self.map.remove(&k);
        }
    }

    pub fn get(&self, key: &str) -> Option<(&Picture, u64)> {
        self.map.get(key).map(|s| (&s.pic, s.rev))
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn rev(&self, key: &str) -> u64 {
        self.map.get(key).map_or(0, |s| s.rev)
    }

    /// Forgets `key`, or every key that begins with it when it ends in `:`.
    pub fn forget(&mut self, key: &str) {
        if key.ends_with(':') {
            self.map.retain(|k, _| !k.starts_with(key));
        } else {
            self.map.remove(key);
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The keys kept, in order (for a test or a debug document).
    pub fn keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.map.keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The memory the pictures hold.
    pub fn bytes(&self) -> usize {
        self.map.values().map(|s| s.pic.rgba.len()).sum()
    }
}

/// The part of `pic` that covers a box of `aspect` (width over height), centred: the whole picture when its shape is the box's.
pub fn crop_cover(pic: &Picture, aspect: f32) -> Picture {
    let (w, h) = (pic.width as f32, pic.height as f32);
    let (cw, ch) = if w / h > aspect { (((h * aspect).round() as u32).clamp(1, pic.width), pic.height) } else { (pic.width, ((w / aspect).round() as u32).clamp(1, pic.height)) };
    if (cw, ch) == (pic.width, pic.height) {
        return pic.clone();
    }
    let (x0, y0) = ((pic.width - cw) / 2, (pic.height - ch) / 2);
    let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
    for y in 0..ch {
        let start = (((y0 + y) * pic.width + x0) * 4) as usize;
        rgba.extend_from_slice(&pic.rgba[start..start + (cw * 4) as usize]);
    }
    Picture { width: cw, height: ch, rgba }
}

/// How a picture fills the box it is shown in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    /// The box is covered and the overhang is cut (a tile, a card).
    Cover,
    /// The whole picture shows inside the box (the full view): the rest of the box is what is behind it.
    Contain,
}

/// A picture shown in a box on the screen (in the space of the group it hangs under, or the screen's).
pub struct PicLayer {
    layer: Option<ImageLayer>,
    pub key: String,
    /// The revision painted (0: none yet).
    rev: u64,
    z: i32,
    /// The box, and the radius its corners take on the screen.
    pub frame: Frame,
    pub radius: f32,
    pub fill: Fill,
    /// The size of the picture in the layer, and the scale it is shown at.
    shown: (u32, u32),
    scale: f32,
    /// The layer is under a group (its props are in that group's space).
    pub grouped: bool,
}

impl PicLayer {
    pub fn new(key: &str, frame: Frame, radius: f32, fill: Fill, z: i32) -> Self {
        Self { layer: None, key: key.to_string(), rev: 0, z, frame, radius, fill, shown: (0, 0), scale: 1.0, grouped: false }
    }

    pub fn is_painted(&self) -> bool {
        self.layer.is_some() && self.rev > 0
    }

    /// The scale the picture is shown at on the screen.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The layer's size (the picture as it was cut), for the harness.
    pub fn size(&self) -> (u32, u32) {
        self.shown
    }

    /// Paints the store's picture for the key when it is newer than what is shown. `group` is the group the layer hangs under, if any.
    /// Returns whether the layer changed.
    pub fn sync(&mut self, kit: &mut Kit, group: Option<&Group>, store: &PictureStore) -> std::io::Result<bool> {
        let Some((pic, rev)) = store.get(&self.key) else { return Ok(false) };
        if rev == self.rev && self.layer.is_some() {
            return Ok(false);
        }
        let cut;
        let source: &Picture = match self.fill {
            Fill::Cover => {
                cut = crop_cover(pic, self.frame.w / self.frame.h.max(1.0));
                &cut
            }
            Fill::Contain => pic,
        };
        let (w, h) = (source.width, source.height);
        let scale = match self.fill {
            Fill::Cover => (self.frame.w / w as f32).max(self.frame.h / h as f32),
            Fill::Contain => (self.frame.w / w as f32).min(self.frame.h / h as f32),
        };
        // the corners are rounded at the size the picture is drawn in, so the arc is the theme's after the scale
        let mut pixels_pic;
        let painted: &Picture = if self.radius > 0.5 && self.fill == Fill::Cover {
            pixels_pic = source.clone();
            pixels_pic.round_corners(self.radius / scale.max(0.01));
            &pixels_pic
        } else {
            source
        };
        let at = self.place(w, h, scale);
        if self.layer.as_ref().is_none_or(|l| (l.width(), l.height()) != (w, h)) {
            let made = match group {
                Some(g) => {
                    let scene = kit.ui.scene.clone();
                    scene.adopting(g, || scene.image(w, h, self.z, at))?
                }
                None => kit.ui.scene.image(w, h, self.z, at)?,
            };
            self.grouped = group.is_some();
            self.layer = Some(made);
        }
        let Some(layer) = self.layer.as_mut() else { return Ok(false) };
        painted.paint(layer);
        layer.set(at);
        self.shown = (w, h);
        self.scale = scale;
        self.rev = rev;
        Ok(true)
    }

    /// Where the layer goes so that its centre is the box's (a Cover picture), or the box's centre (Contain), at the scale it is shown.
    fn place(&self, w: u32, h: u32, scale: f32) -> Props {
        let (cx, cy) = self.frame.centre();
        let mut p = Props::at(cx - w as f32 / 2.0, cy - h as f32 / 2.0);
        p.scale = scale;
        if self.fill == Fill::Cover {
            // what overhangs the box is cut: the clip is the box, in the space of the layer's parent
            p.clip = Some(Rect { x: self.frame.x, y: self.frame.y, w: self.frame.w, h: self.frame.h });
        }
        p
    }

    /// Moves the box (the grid scrolled, a tile was placed again); the picture stays where it is in the box.
    pub fn place_box(&mut self, frame: Frame) {
        self.frame = frame;
        if let Some(l) = &self.layer {
            let at = self.place(self.shown.0, self.shown.1, self.scale);
            l.set(at);
        }
    }

    /// Hides or shows the layer.
    pub fn set_visible(&self, on: bool) {
        if let Some(l) = &self.layer {
            let at = self.place(self.shown.0, self.shown.1, self.scale);
            l.set(Props { visible: on, opacity: if on { 1.0 } else { 0.0 }, ..at });
        }
    }

    pub fn layer(&self) -> Option<&ImageLayer> {
        self.layer.as_ref()
    }

    /// Lets the layer go (a picture that is no longer wanted); the next sync makes it again.
    pub fn clear(&mut self) {
        self.layer = None;
        self.rev = 0;
    }

    /// Fades the layer in over `ms` (a picture arriving over a quiet tile).
    pub fn arrive(&self, kit: &Kit, ms: u32) {
        if let Some(l) = &self.layer {
            let at = self.place(self.shown.0, self.shown.1, self.scale);
            if ms == 0 || kit.ui.look.duration_ms(ms) == 0 {
                l.set(at);
            } else {
                l.set(Props { opacity: 0.0, ..at });
                l.animate(at, kit.ui.look.duration_ms(ms), compositor_protocol::Ease::Out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    fn pic(w: u32, h: u32, rgba: [u8; 4]) -> Picture {
        Picture { width: w, height: h, rgba: rgba.iter().cycle().take((w * h * 4) as usize).copied().collect() }
    }

    #[test]
    fn a_picture_the_shape_of_its_box_is_whole_and_a_taller_one_is_cut_top_and_bottom() {
        let wide = pic(480, 270, [10, 20, 30, 255]);
        assert_eq!((crop_cover(&wide, 16.0 / 9.0).width, crop_cover(&wide, 16.0 / 9.0).height), (480, 270));
        let portrait = pic(480, 640, [10, 20, 30, 255]);
        let cut = crop_cover(&portrait, 16.0 / 9.0);
        assert_eq!((cut.width, cut.height), (480, 270), "a portrait camera in a 16:9 tile: the middle of it");
        let narrow = pic(1000, 100, [1, 2, 3, 255]);
        let cut = crop_cover(&narrow, 1.0);
        assert_eq!((cut.width, cut.height), (100, 100));
    }

    #[test]
    fn the_cut_is_the_middle_rows() {
        let mut p = pic(4, 8, [0, 0, 0, 255]);
        for y in 0..8u32 {
            for x in 0..4u32 {
                let i = ((y * 4 + x) * 4) as usize;
                p.rgba[i] = y as u8;
            }
        }
        let cut = crop_cover(&p, 2.0);
        assert_eq!((cut.width, cut.height), (4, 2));
        assert_eq!(cut.at(0, 0)[0], 3, "rows 3 and 4 of 8");
        assert_eq!(cut.at(0, 1)[0], 4);
    }

    #[test]
    fn the_store_keeps_the_newest_of_each_kind_and_counts_revisions() {
        let mut s = PictureStore::new();
        assert_eq!(s.put("tile:1", pic(2, 2, [1, 1, 1, 255])), 1);
        assert_eq!(s.put("tile:1", pic(2, 2, [2, 2, 2, 255])), 2);
        assert_eq!(s.rev("tile:1"), 2);
        assert_eq!(s.get("tile:1").unwrap().0.rgba[0], 2);
        for i in 0..20 {
            s.put(&format!("tile:c{i}"), pic(2, 2, [9, 9, 9, 255]));
        }
        assert_eq!(s.map.keys().filter(|k| k.starts_with("tile:")).count(), 16, "the last picture of at most 16 cameras");
        assert!(!s.has("tile:1") && s.has("tile:c19"));
        s.put("card:a", pic(2, 2, [1, 1, 1, 255]));
        s.put("bubble:x", pic(2, 2, [1, 1, 1, 255]));
        s.forget("card:");
        assert!(!s.has("card:a") && s.has("bubble:x"));
        s.forget("bubble:x");
        assert!(!s.has("bubble:x"));
        assert!(s.bytes() > 0 && !s.is_empty());
    }

    #[test]
    fn a_layer_is_made_at_the_pictures_own_size_scaled_to_the_box_and_painted_once_per_revision() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut store = PictureStore::new();
        let mut l = PicLayer::new("tile:1", Frame::new(100.0, 200.0, 416.0, 234.0), 22.0, Fill::Cover, 5);
        assert!(!l.sync(&mut kit, None, &store).unwrap(), "nothing arrived: nothing is drawn, nothing is made up");
        assert!(l.layer().is_none());
        store.put("tile:1", pic(480, 270, [200, 100, 50, 255]));
        assert!(l.sync(&mut kit, None, &store).unwrap());
        assert_eq!(l.size(), (480, 270), "the picture's own size: the compositor scales it");
        assert!((l.scale() - 416.0 / 480.0).abs() < 1e-4);
        let layer = l.layer().unwrap();
        assert_eq!(layer.pixels()[((135 * 480 + 240) * 4) as usize], 200, "painted");
        assert_eq!(layer.pixels()[3], 0, "the corner is rounded away: the first pixel is clear");
        assert!(!l.sync(&mut kit, None, &store).unwrap(), "the same revision is not painted again");
        store.put("tile:1", pic(480, 270, [10, 10, 10, 255]));
        assert!(l.sync(&mut kit, None, &store).unwrap());
        assert_eq!(l.layer().unwrap().pixels()[((135 * 480 + 240) * 4) as usize], 10);
    }

    #[test]
    fn a_picture_of_another_shape_is_cut_to_the_box_and_one_of_another_size_makes_the_layer_again() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut store = PictureStore::new();
        let mut l = PicLayer::new("tile:2", Frame::new(0.0, 0.0, 416.0, 234.0), 0.0, Fill::Cover, 5);
        store.put("tile:2", pic(480, 640, [5, 5, 5, 255]));
        l.sync(&mut kit, None, &store).unwrap();
        assert_eq!(l.size(), (480, 270));
        store.put("tile:2", pic(640, 360, [5, 5, 5, 255]));
        l.sync(&mut kit, None, &store).unwrap();
        assert_eq!(l.size(), (640, 360));
        assert!((l.scale() - 0.65).abs() < 1e-4);
    }

    #[test]
    fn the_full_view_shows_the_whole_picture_inside_the_screen() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut store = PictureStore::new();
        let mut l = PicLayer::new("full:1", Frame::new(0.0, 0.0, 1920.0, 1080.0), 0.0, Fill::Contain, 5);
        store.put("full:1", pic(480, 640, [7, 7, 7, 255]));
        l.sync(&mut kit, None, &store).unwrap();
        assert_eq!(l.size(), (480, 640), "a portrait camera is not cut");
        assert!((l.scale() - 1080.0 / 640.0).abs() < 1e-4, "it fits the height and the black shows beside it");
    }
}
