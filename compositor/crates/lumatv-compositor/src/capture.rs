//! Writing a captured frame as PNG (straight alpha, so a viewer shows what the panel would). Encoding 1080p takes a
//! few hundred milliseconds on the set, so it never runs on the loop: the loop reads the page back (a copy) and hands
//! the picture to [`Writer`].

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use crate::output::Image;

pub fn write_png(path: &Path, image: &Image) -> io::Result<()> {
    let file = File::create(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    // Un-premultiply for the file; the compositor keeps premultiplied pixels.
    let mut straight = Vec::with_capacity(image.rgba.len());
    for px in image.rgba.chunks_exact(4) {
        let a = px[3] as u32;
        if a == 0 || a == 255 {
            straight.extend_from_slice(px);
        } else {
            straight.extend_from_slice(&[
                ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8,
                ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8,
                ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8,
                px[3],
            ]);
        }
    }
    writer.write_image_data(&straight).map_err(io::Error::other)
}

/// Writes captured frames as PNG on a thread of its own, one at a time: a capture that arrives while the last one is
/// still being written is dropped (with a warning), so a run that captures every frame never piles up pictures.
#[derive(Default)]
pub struct Writer {
    busy: Option<JoinHandle<()>>,
}

impl Writer {
    /// Hands `image` over to be written at `path`; `false` when the last one is still being written and this one is
    /// dropped. The write's outcome is logged.
    pub fn write(&mut self, path: PathBuf, image: Image) -> bool {
        if let Some(last) = self.busy.take() {
            if last.is_finished() {
                let _ = last.join();
            } else {
                self.busy = Some(last);
                log::warn!("capture {}: dropped, the last capture is still being written", path.display());
                return false;
            }
        }
        match std::thread::Builder::new().name("png".into()).spawn(move || {
            crate::sched::time_shared();
            if let Err(e) = write_png(&path, &image) {
                log::warn!("capture {}: {e}", path.display());
            }
        }) {
            Ok(handle) => {
                self.busy = Some(handle);
                true
            }
            Err(e) => {
                log::warn!("capture: no thread: {e}");
                false
            }
        }
    }

    /// Waits for the write in progress, if any (at exit, so the last picture is whole).
    pub fn finish(&mut self) {
        if let Some(last) = self.busy.take() {
            let _ = last.join();
        }
    }
}

/// The X2 pattern: frame `k` is R = 25k, G = 0x80, B = 255 - 25k, A = 255.
pub fn x2_colour(k: u32) -> [u8; 4] {
    [(25 * k).min(255) as u8, 0x80, 255u8.saturating_sub((25 * k).min(255) as u8), 255]
}

/// Whether a frame's three probe pixels (top-left, centre, bottom-right of `rect`) are all `expected`.
pub fn matches(image: &Image, rect: (u32, u32, u32, u32), expected: [u8; 4]) -> bool {
    let (x, y, w, h) = rect;
    if w == 0 || h == 0 {
        return false;
    }
    [(x, y), (x + w / 2, y + h / 2), (x + w - 1, y + h - 1)].iter().all(|&(px, py)| image.pixel(px, py) == expected)
}
