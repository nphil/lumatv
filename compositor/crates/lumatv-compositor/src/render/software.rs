//! CPU composition of `wl_shm` buffers into an RGBA target. Exact and host-testable; dma-buf and driver buffers
//! cannot be read here (AFBC, no mapping) and are counted as skipped.

use std::io;

use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::Rect;
use crate::buffer::{BufferKindRef, kind_of};
use crate::output::{FrameClock, Image, Output, Presented, RenderStats};
use crate::scene::Plan;
use crate::surface::Surfaces;

pub struct Software {
    width: u32,
    height: u32,
    /// Premultiplied 0xAARRGGBB, row-major, top-down.
    pixels: Vec<u32>,
    cursor: Vec<u32>,
    clock: FrameClock,
    skipped_logged: bool,
}

impl Software {
    pub fn new(width: u32, height: u32) -> Self {
        Software {
            width,
            height,
            pixels: vec![0xff00_0000; (width * height) as usize],
            cursor: super::cursor::sprite_argb(),
            clock: FrameClock::new(),
            skipped_logged: false,
        }
    }

    fn fill(&mut self, r: &Rect, value: u32) {
        let r = r.intersect(&Rect::new(0, 0, self.width as i32, self.height as i32));
        for y in r.y..r.bottom() {
            let row = y as usize * self.width as usize;
            self.pixels[row + r.x as usize..row + r.right() as usize].fill(value);
        }
    }

    pub fn pixel_argb(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }
}

/// Source-over of premultiplied 0xAARRGGBB `src` onto `dst`.
#[inline]
fn blend(dst: u32, src: u32) -> u32 {
    let sa = src >> 24;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return dst;
    }
    let inv = 255 - sa;
    let ch = |shift: u32| -> u32 {
        let s = (src >> shift) & 255;
        let d = (dst >> shift) & 255;
        (s + (d * inv + 127) / 255).min(255)
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

impl Output for Software {
    fn name(&self) -> &'static str {
        "software"
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn query_buffer(&mut self, _buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        None
    }

    fn forget_buffer(&mut self, _buffer: &WlBuffer) {}

    fn render(&mut self, plan: &Plan, _surfaces: &Surfaces) -> io::Result<RenderStats> {
        let t0 = std::time::Instant::now();
        let mut stats = RenderStats { items: plan.items.len(), ..Default::default() };
        let area = plan.damage;
        if area.is_empty() {
            return Ok(stats);
        }
        self.fill(&area, 0xff00_0000);
        for (index, item) in plan.items.iter().enumerate() {
            // a hole is cleared just before the item of its surface (LG's order): what is under it goes, what it and the items above draw stays
            for h in plan.holes.iter().filter(|h| h.before == index) {
                let r = h.rect.intersect(&area);
                if !r.is_empty() {
                    self.fill(&r, 0);
                }
            }
            let clip = item.rect.intersect(&area);
            if clip.is_empty() {
                continue;
            }
            match kind_of(&item.buffer) {
                BufferKindRef::Shm(shm) => {
                    let shm = shm.clone();
                    let width = self.width as usize;
                    let pixels = &mut self.pixels;
                    shm.pool.with_bytes(|bytes| {
                        for y in clip.y..clip.bottom() {
                            let sy = y - item.rect.y;
                            let row = y as usize * width;
                            for x in clip.x..clip.right() {
                                let sx = x - item.rect.x;
                                if let Some(src) = shm.pixel(bytes, sx, sy) {
                                    let d = &mut pixels[row + x as usize];
                                    *d = blend(*d, src);
                                }
                            }
                        }
                    });
                    stats.drawn += 1;
                }
                BufferKindRef::Dmabuf(_) | BufferKindRef::External => {
                    stats.skipped += 1;
                    if !self.skipped_logged {
                        log::warn!("software renderer cannot read {} buffers; use the GL renderer", kind_of(&item.buffer).name());
                        self.skipped_logged = true;
                    }
                }
            }
        }
        if let Some(c) = plan.cursor {
            let clip = c.intersect(&area);
            for y in clip.y..clip.bottom() {
                for x in clip.x..clip.right() {
                    let src = self.cursor[((y - c.y) * c.w + (x - c.x)) as usize];
                    let d = &mut self.pixels[(y as u32 * self.width + x as u32) as usize];
                    *d = blend(*d, src);
                }
            }
        }
        stats.render_us = t0.elapsed().as_micros() as u64;
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        Ok(self.clock.stamp())
    }

    fn capture(&mut self) -> io::Result<Image> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            rgba.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]);
        }
        Ok(Image { width: self.width, height: self.height, rgba })
    }
}

impl BufferKindRef<'_> {
    pub fn name(&self) -> &'static str {
        match self {
            BufferKindRef::Shm(_) => "shm",
            BufferKindRef::Dmabuf(_) => "dmabuf",
            BufferKindRef::External => "external",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::blend;

    #[test]
    fn source_over_premultiplied() {
        assert_eq!(blend(0xff00_0000, 0xffff_0000), 0xffff_0000);
        assert_eq!(blend(0xff00_0000, 0x0000_0000), 0xff00_0000);
        // Half-transparent white (premultiplied 0x80808080) over black: 0x80 per channel, alpha stays 255.
        let r = blend(0xff00_0000, 0x8080_8080);
        assert_eq!(r >> 24, 255);
        assert!((r >> 16 & 255).abs_diff(0x80) <= 1);
    }
}
