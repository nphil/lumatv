//! Where frames go: a window of LG's compositor on the set, a pbuffer on a host.

use lumatv_render::Gles;
use lumatv_render::host::Engine;
use std::io;
use std::time::Duration;

use crate::home::Key;

pub trait Output {
    /// Shows the frame the engine drew into the window's framebuffer. On a window this waits for the compositor's frame callback.
    fn present(&mut self, engine: &mut Engine) -> io::Result<()>;
    /// Sleeps until `deadline_ns` (`CLOCK_MONOTONIC`) or until the compositor says something.
    fn wait(&mut self, deadline_ns: Option<u64>) -> io::Result<()>;
    /// Takes in what the compositor has said without waiting: the keys it sent.
    fn dispatch(&mut self) -> io::Result<Vec<Key>>;
    /// Writes the window's picture as a PNG.
    fn screenshot(&mut self, engine: &mut Engine, path: &str) -> io::Result<()>;
}

/// Mesa's software GL into a pbuffer the size of the screen, paced at 60 Hz by sleeping, for the host's runs.
struct Offscreen {
    width: u32,
    height: u32,
    last: u64,
}

pub fn offscreen(width: u32, height: u32) -> io::Result<(Gles, Box<dyn Output>)> {
    let gles = Gles::software_pbuffer(c"libEGL.so.1", width, height)?;
    Ok((gles, Box::new(Offscreen { width, height, last: 0 })))
}

impl Output for Offscreen {
    fn present(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.gles().finish();
        // a window's swap waits for the next refresh: so does this, to the 60 Hz grid
        let period = 16_666_667u64;
        let now = lumatv_render::now_ns();
        let next = self.last + period;
        if next > now {
            std::thread::sleep(Duration::from_nanos(next - now));
        }
        self.last = lumatv_render::now_ns();
        Ok(())
    }

    fn wait(&mut self, deadline_ns: Option<u64>) -> io::Result<()> {
        let now = lumatv_render::now_ns();
        let until = deadline_ns.unwrap_or(now + 5_000_000);
        if until > now {
            std::thread::sleep(Duration::from_nanos((until - now).min(50_000_000)));
        }
        Ok(())
    }

    fn dispatch(&mut self) -> io::Result<Vec<Key>> {
        Ok(Vec::new())
    }

    fn screenshot(&mut self, engine: &mut Engine, path: &str) -> io::Result<()> {
        let rows = engine.read_window();
        write_png(path, self.width, self.height, &rows)
    }
}

/// `rows` are RGBA from the bottom row up, premultiplied: written top row first, over black, which is what the screen shows.
pub fn write_png(path: &str, width: u32, height: u32, rows: &[u8]) -> io::Result<()> {
    let mut rgba = Vec::with_capacity(rows.len());
    for y in (0..height as usize).rev() {
        let row = &rows[y * width as usize * 4..(y + 1) * width as usize * 4];
        for px in row.chunks_exact(4) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(io::Error::other)
}
