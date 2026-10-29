//! A picture the compositor keeps from one owner to the next.
//!
//! Some pictures outlive what asked for them: the shell's wallpaper is the picture of the theme generation in force, and it is the
//! same picture every time the shell is mounted again (the look was made again for a new text size, a screen that went in front of
//! the shell and out). A layer made each time makes the compositor make a texture of the whole screen and copy the 1.8 MB of the
//! picture into it, which is the largest upload of the commit that mounts the shell, for pixels the compositor already holds.
//!
//! [`Ui::keep_image`] hands out the layer it made for these pixels the last time, with nothing made and nothing written, and shows
//! it again; it is hidden (a hidden layer asks for no frames and nothing is drawn of it) while nobody holds it. One picture is
//! kept: another one replaces it, and the old layer goes when its last hold does.

use crate::context::Ui;
use compositor_protocol::{LayerId, Props};
use std::cell::Cell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;

/// The picture a [`Ui`] keeps.
pub(crate) struct Kept {
    /// The pixels the layer was made from. Nothing writes to an `Arc<[u8]>`, so the same allocation is the same picture.
    source: Arc<[u8]>,
    z: i32,
    layer: crate::scene::ImageLayer,
    /// Holds out: the layer is shown while there is one.
    holds: Cell<usize>,
    shown: Cell<bool>,
}

impl Kept {
    fn show(&self, shown: bool) {
        if self.shown.replace(shown) != shown {
            self.layer.set(Props {
                visible: shown,
                ..Props::at(0.0, 0.0)
            });
        }
    }
}

/// A hold on a kept picture: it is on the screen while one is held, and when the last is let go it stays on the compositor,
/// hidden, for whoever holds the same pixels next.
pub struct KeptImage {
    kept: Rc<Kept>,
}

impl KeptImage {
    fn hold(kept: Rc<Kept>) -> Self {
        kept.holds.set(kept.holds.get() + 1);
        kept.show(true);
        Self { kept }
    }

    pub fn id(&self) -> LayerId {
        self.kept.layer.id()
    }

    pub fn width(&self) -> u32 {
        self.kept.layer.width()
    }

    pub fn height(&self) -> u32 {
        self.kept.layer.height()
    }

    /// The pixels the compositor was given, as they are.
    pub fn pixels(&self) -> &[u8] {
        self.kept.layer.pixels()
    }
}

impl Drop for KeptImage {
    fn drop(&mut self) {
        let holds = self.kept.holds.get() - 1;
        self.kept.holds.set(holds);
        if holds == 0 {
            self.kept.show(false);
        }
    }
}

impl Ui {
    /// The picture of `source` (premultiplied RGBA, `width` by `height`) at the top left, at depth `z`, held. Asked again for the
    /// same pixels (the same allocation) at the same size and depth, it is the layer made before, shown again, and the compositor
    /// is sent nothing but that; for other pixels, size or depth a new layer is made and written once, and it is the one kept from
    /// then on. Whoever holds the old layer still shows it until the hold is let go. A `source` that is not `width * height * 4`
    /// bytes is refused with nothing made.
    pub fn keep_image(
        &mut self,
        source: &Arc<[u8]>,
        width: u32,
        height: u32,
        z: i32,
    ) -> io::Result<KeptImage> {
        if let Some(kept) = self.kept_image.as_ref().filter(|kept| {
            Arc::ptr_eq(&kept.source, source)
                && kept.z == z
                && (kept.layer.width(), kept.layer.height()) == (width, height)
        }) {
            return Ok(KeptImage::hold(Rc::clone(kept)));
        }
        let wanted = width as usize * height as usize * 4;
        if source.len() != wanted {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "a {width}x{height} picture has {} bytes, not {wanted}",
                    source.len()
                ),
            ));
        }
        let mut layer = self.scene.image(width, height, z, Props::at(0.0, 0.0))?;
        layer.pixels_mut().copy_from_slice(source);
        layer.damage(None);
        // Written once and never edited: the app does not keep the file's pages mapped (a wallpaper is 1.8 MB of them).
        layer.release_resident();
        let kept = Rc::new(Kept {
            source: Arc::clone(source),
            z,
            layer,
            holds: Cell::new(0),
            shown: Cell::new(true),
        });
        self.kept_image = Some(Rc::clone(&kept));
        Ok(KeptImage::hold(kept))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, harness, pictures_made, pixels_damaged};
    use compositor_protocol::Request;

    fn picture(width: u32, height: u32, seed: u8) -> Arc<[u8]> {
        (0..width * height * 4)
            .map(|i| (i as u8).wrapping_mul(seed))
            .collect::<Vec<u8>>()
            .into()
    }

    fn deletes(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Delete { .. }))
            .count()
    }

    /// What the commit did to the layer's visibility, in order.
    fn shown(requests: &[Request]) -> Vec<bool> {
        requests
            .iter()
            .filter_map(|r| match r {
                Request::Set { props, .. } => Some(props.visible),
                _ => None,
            })
            .collect()
    }

    fn keep(h: &mut Harness, source: &Arc<[u8]>) -> KeptImage {
        h.ui.keep_image(source, 8, 4, 3).expect("a kept image")
    }

    /// The kilobytes of layer `id`'s file this process has mapped, as `/proc/self/smaps` counts them.
    fn mapped_kb(h: &Harness, id: compositor_protocol::LayerId) -> usize {
        // The whole path: the other tests of this binary have layers of the same names in directories of their own.
        let name = std::fs::canonicalize(h.shm_dir().join(format!("layer-{}.rgba", id.0)))
            .expect("the layer's file")
            .to_string_lossy()
            .into_owned();
        let mut mine = false;
        let mut kb = 0;
        for line in std::fs::read_to_string("/proc/self/smaps").unwrap().lines() {
            if line.as_bytes().first().is_some_and(u8::is_ascii_hexdigit) && line.contains('-') {
                mine = line.ends_with(&name);
            } else if mine && let Some(rest) = line.strip_prefix("Rss:") {
                kb += rest
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<usize>()
                    .unwrap();
            }
        }
        kb
    }

    #[test]
    fn a_kept_picture_is_written_once_and_its_pages_are_not_kept_mapped() {
        // A wallpaper: 960 x 480 pixels written into the file, which the compositor reads. The app's resident size counted the
        // 1.8 MB of mapped pages for as long as the picture lived.
        let mut h = harness();
        let source = picture(960, 480, 3);
        let kept = h.ui.keep_image(&source, 960, 480, 3).expect("a kept image");
        assert_eq!(mapped_kb(&h, kept.id()), 0, "written, and let go of");
        // What the compositor reads is what was written.
        assert_eq!(h.pixels(kept.id()), &source[..]);
        // Touched again, it is mapped again: nothing was lost with the mapping.
        assert_eq!(kept.pixels(), &source[..]);
        assert!(mapped_kb(&h, kept.id()) >= 1_800);
    }

    #[test]
    fn the_first_hold_makes_the_layer_and_writes_the_pixels_once() {
        let mut h = harness();
        let source = picture(8, 4, 7);
        let first = keep(&mut h, &source);
        assert_eq!(first.pixels(), &source[..]);
        assert_eq!((first.width(), first.height()), (8, 4));
        let made = h.commit();
        assert_eq!(pictures_made(&made), 1);
        assert_eq!(pixels_damaged(&made), 8.0 * 4.0);
        assert!(shown(&made).is_empty(), "made shown: {made:?}");
    }

    #[test]
    fn the_same_pixels_again_make_nothing_and_write_nothing_whatever_came_between() {
        let mut h = harness();
        let source = picture(8, 4, 7);
        let first = keep(&mut h, &source);
        let id = first.id();
        h.commit();
        drop(first);
        // Nobody holds it: it stays on the compositor, hidden (nothing of it is drawn, and it asks for no frame).
        let gone = h.commit();
        assert_eq!(shown(&gone), [false]);
        assert_eq!(deletes(&gone), 0);
        let again = keep(&mut h, &source);
        let back = h.commit();
        assert_eq!(again.id(), id, "the layer made before");
        assert_eq!(pictures_made(&back), 0, "{back:?}");
        assert_eq!(pixels_damaged(&back), 0.0);
        assert_eq!(deletes(&back), 0);
        assert_eq!(shown(&back), [true]);
    }

    #[test]
    fn a_hold_taken_before_the_last_is_let_go_shares_the_layer_and_the_commit_says_nothing() {
        // The lab builds the next screen before the one it replaces goes.
        let mut h = harness();
        let source = picture(8, 4, 7);
        let a = keep(&mut h, &source);
        let id = a.id();
        h.commit();
        let b = keep(&mut h, &source);
        drop(a);
        let next = h.commit();
        assert_eq!(b.id(), id);
        assert!(
            next.iter().all(|r| matches!(r, Request::Commit { .. })),
            "{next:?}"
        );
        drop(b);
    }

    #[test]
    fn other_pixels_are_a_new_layer_and_the_old_one_goes_with_its_last_hold() {
        let mut h = harness();
        let (old, new) = (picture(8, 4, 7), picture(8, 4, 9));
        let first = keep(&mut h, &old);
        h.commit();
        // A picture with the same bytes in another allocation is another picture: pixels are told apart by identity, never read.
        let same_bytes: Arc<[u8]> = old.to_vec().into();
        let second = keep(&mut h, &same_bytes);
        assert_ne!(first.id(), second.id());
        let made = h.commit();
        assert_eq!((pictures_made(&made), pixels_damaged(&made)), (1, 32.0));
        drop(first);
        let after = h.commit();
        assert_eq!(deletes(&after), 1, "the layer nobody holds goes: {after:?}");
        let third = keep(&mut h, &new);
        assert_ne!(second.id(), third.id());
        drop(second);
        let swapped = h.commit();
        assert_eq!(pictures_made(&swapped), 1);
        assert_eq!(deletes(&swapped), 1);
        assert_eq!(third.pixels(), &new[..]);
    }

    #[test]
    fn another_size_or_depth_is_another_layer() {
        let mut h = harness();
        let source = picture(8, 4, 7);
        let a = keep(&mut h, &source);
        let deeper = h.ui.keep_image(&source, 8, 4, 4).expect("another depth");
        assert_ne!(a.id(), deeper.id());
        let narrower = picture(4, 4, 7);
        let b = h.ui.keep_image(&narrower, 4, 4, 4).expect("another size");
        assert_ne!(deeper.id(), b.id());
        assert_eq!(pictures_made(&h.commit()), 3);
    }

    #[test]
    fn pixels_that_do_not_fit_their_size_are_refused_and_nothing_is_made() {
        let mut h = harness();
        let short: Arc<[u8]> = vec![0u8; 10].into();
        let refused = h.ui.keep_image(&short, 8, 4, 3);
        assert!(matches!(refused, Err(e) if e.kind() == io::ErrorKind::InvalidData));
        let made = h.commit();
        assert_eq!(pictures_made(&made), 0, "{made:?}");
        assert_eq!(pixels_damaged(&made), 0.0);
    }
}
