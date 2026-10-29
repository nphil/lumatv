//! CPU composition of `wl_shm` buffers into an RGBA target. Exact and host-testable; dma-buf and driver buffers
//! cannot be read here (AFBC, no mapping) and are counted as skipped.

use std::io;

use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::Rect;
use crate::buffer::{BufferKindRef, kind_of};
use crate::output::{FrameClock, Image, Output, Presented, RenderStats};
use crate::scene::{Backdrop, HoleVeil, Plan, Posed, coverage};
use crate::surface::Surfaces;
use crate::transition::{VEIL_RGB, mask_coverage};

pub struct Software {
    width: u32,
    height: u32,
    /// Premultiplied 0xAARRGGBB, row-major, top-down.
    pixels: Vec<u32>,
    cursor: Vec<u32>,
    /// The app's pictures composed for the box dim's sample (`sample_begin`), kept between samples; empty until the first. With the
    /// numbers of the sample begun last, until `sample_take` reads them.
    sample: Vec<u32>,
    sigs: Option<Box<[crate::boxdim::Sig; crate::boxdim::CELLS]>>,
    sample_ready: bool,
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
            sample: Vec::new(),
            sigs: None,
            sample_ready: false,
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

    /// The app switcher's scrim and pictures (`scene/backdrop.rs`), inside `area`. Only `wl_shm` pictures can be read here.
    fn draw_backdrop(&mut self, b: &Backdrop, area: &Rect, stats: &mut RenderStats) {
        let scrim = pack(b.scrim);
        if b.scrim[3] > 0.0 {
            for y in area.y..area.bottom() {
                let row = y as usize * self.width as usize;
                for x in area.x..area.right() {
                    let d = &mut self.pixels[row + x as usize];
                    *d = blend(*d, scrim);
                }
            }
        }
        for p in &b.previews {
            let clip = p.rect.intersect(area);
            if clip.is_empty() {
                continue;
            }
            let BufferKindRef::Shm(shm) = kind_of(&p.buffer) else {
                stats.skipped += 1;
                continue;
            };
            let shm = shm.clone();
            let width = self.width as usize;
            let pixels = &mut self.pixels;
            let (w, h) = (p.rect.w as f32, p.rect.h as f32);
            shm.pool.with_bytes(|bytes| {
                for y in clip.y..clip.bottom() {
                    let fy = (y - p.rect.y) as f32 + 0.5;
                    let sy = ((fy / h * p.src.1 as f32) as i32).clamp(0, p.src.1 - 1);
                    for x in clip.x..clip.right() {
                        let fx = (x - p.rect.x) as f32 + 0.5;
                        let cover = coverage(fx, fy, w, h, p.radius) * p.alpha;
                        if cover <= 0.0 {
                            continue;
                        }
                        let sx = ((fx / w * p.src.0 as f32) as i32).clamp(0, p.src.0 - 1);
                        if let Some(src) = shm.pixel(bytes, sx, sy) {
                            let d = &mut pixels[y as usize * width + x as usize];
                            *d = blend(*d, scale(src, cover));
                        }
                    }
                }
            });
            stats.drawn += 1;
        }
    }

    /// A veil's fill (`scene::HoleVeil`): the flat dark colour at the veil's opacity, over the pixels of its quad that its card's mask shows.
    fn fill_veil(&mut self, v: &HoleVeil, area: &Rect) {
        let r = v.quad.bounds().intersect(&v.clip.bounds()).intersect(area);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                if cx < v.quad.x || cx >= v.quad.right() || cy < v.quad.y || cy >= v.quad.bottom() {
                    continue;
                }
                let a = v.alpha * mask_coverage(&v.clip, v.radius, cx, cy);
                let ch = |c: u8| (c as f32 * a + 0.5) as u32;
                let src = (((a * 255.0 + 0.5) as u32) << 24) | (ch(VEIL_RGB[0]) << 16) | (ch(VEIL_RGB[1]) << 8) | ch(VEIL_RGB[2]);
                let d = &mut self.pixels[(y as u32 * self.width + x as u32) as usize];
                *d = blend(*d, src);
            }
        }
    }
}

/// Premultiplied RGBA floats (0 to 1) as 0xAARRGGBB.
fn pack(c: [f32; 4]) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    (q(c[3]) << 24) | (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2])
}

/// A premultiplied 0xAARRGGBB pixel times `c` (0 to 1).
fn scale(p: u32, c: f32) -> u32 {
    let ch = |shift: u32| -> u32 { (((p >> shift) & 0xff) as f32 * c + 0.5) as u32 };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Bilinear sample of a shm buffer at the texel coordinate (`u`, `v`) (texel centres at half integers, edges clamped), premultiplied channels.
fn sample(shm: &crate::buffer::ShmBuffer, bytes: &[u8], u: f32, v: f32) -> [f32; 4] {
    let (fx, fy) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |x: f32, y: f32| -> [f32; 4] {
        let (x, y) = ((x as i32).clamp(0, shm.width - 1), (y as i32).clamp(0, shm.height - 1));
        let p = shm.pixel(bytes, x, y).unwrap_or(0);
        [(p >> 16 & 255) as f32, (p >> 8 & 255) as f32, (p & 255) as f32, (p >> 24) as f32]
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty;
    }
    out
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

/// `src` with its colour scaled by `dim` (0 to 1) and its alpha left: the OLED care's dim, as the GL renderer's `u_dim` does it.
#[inline]
fn dimmed(src: u32, dim: u32) -> u32 {
    let ch = |shift: u32| -> u32 { (((src >> shift) & 255) * dim + 128) / 256 };
    (src & 0xff00_0000) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// An item through its pose: its buffer scaled onto `pose.quad` (bilinear, as the GPU samples), behind the window's rounded mask, at the pose's
/// opacity and brightness, over what is there.
fn draw_posed(pixels: &mut [u32], width: usize, shm: &crate::buffer::ShmBuffer, bytes: &[u8], pose: &Posed, clip: &Rect) {
    for y in clip.y..clip.bottom() {
        for x in clip.x..clip.right() {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (u, v) = ((cx - pose.quad.x) / pose.quad.w, (cy - pose.quad.y) / pose.quad.h);
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let c = sample(shm, bytes, u * shm.width as f32, v * shm.height as f32);
            let k = pose.alpha * mask_coverage(&pose.clip, pose.radius, cx, cy);
            let ch = |i: usize, shade: f32| ((c[i] * shade * k + 0.5) as u32).min(255);
            let src = (((c[3] * k + 0.5) as u32).min(255) << 24) | (ch(0, pose.shade) << 16) | (ch(1, pose.shade) << 8) | ch(2, pose.shade);
            let d = &mut pixels[y as usize * width + x as usize];
            *d = blend(*d, src);
        }
    }
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
            if let Some(b) = plan.backdrop.as_ref().filter(|b| b.before == index) {
                self.draw_backdrop(b, &area, &mut stats);
            }
            // a hole is cleared just before the item of its surface (LG's order): what is under it goes, what it and the items above draw stays
            // (an arriving card's veil goes on the clear, under the card's own pixels)
            for h in plan.holes.iter().filter(|h| h.before == index) {
                let r = h.rect.intersect(&area);
                if !r.is_empty() && h.veil.is_none_or(|v| v.clear) {
                    self.fill(&r, 0);
                }
                if let Some(v) = &h.veil {
                    self.fill_veil(v, &area);
                }
            }
            let clip = item.drawn_rect().intersect(&area);
            if clip.is_empty() {
                continue;
            }
            // 256 is "not dimmed": the scale in 256ths
            let dim = (item.dim * 256.0).round() as u32;
            match kind_of(&item.buffer) {
                BufferKindRef::Shm(shm) if item.pose.is_some() => {
                    let shm = shm.clone();
                    let mut pose = item.pose.unwrap();
                    pose.shade *= item.dim;
                    let width = self.width as usize;
                    let pixels = &mut self.pixels;
                    shm.pool.with_bytes(|bytes| draw_posed(pixels, width, &shm, bytes, &pose, &clip));
                    stats.drawn += 1;
                }
                BufferKindRef::Shm(shm) => {
                    let shm = shm.clone();
                    let width = self.width as usize;
                    let pixels = &mut self.pixels;
                    // The box dim's map on an app's picture, sampled per pixel as the GL renderer's texture is (`boxdim`).
                    let gain = plan.gain().filter(|_| item.gain);
                    shm.pool.with_bytes(|bytes| {
                        for y in clip.y..clip.bottom() {
                            let sy = y - item.rect.y;
                            let row = y as usize * width;
                            for x in clip.x..clip.right() {
                                let sx = x - item.rect.x;
                                if let Some(src) = shm.pixel(bytes, sx, sy) {
                                    let src = if dim < 256 { dimmed(src, dim) } else { src };
                                    let src = match gain {
                                        Some(g) => dimmed(src, (g.at(x, y) * 256.0).round() as u32),
                                        None => src,
                                    };
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
        // an idle system layer leaves the backdrop above every item
        if let Some(b) = plan.backdrop.as_ref().filter(|b| b.before >= plan.items.len()) {
            self.draw_backdrop(b, &area, &mut stats);
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

    /// The app's pictures composed over black on the CPU (each where its window places it, `Item::drift` taken away), holes cleared, then
    /// each cell's mean luma (Rec. 709 weights on the 8-bit values), of the cell, of its left 14 columns and of its lower 14 rows, as the
    /// GL renderer's passes take them. Done at once: `sample_take` has it at the next call.
    fn sample_begin(&mut self, plan: &Plan, _surfaces: &Surfaces) -> io::Result<bool> {
        use crate::boxdim::{CELL, CELLS, COLS, ROWS};
        let (w, h) = (self.width as usize, self.height as usize);
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        let pixels = &mut self.sample;
        pixels.clear();
        pixels.resize(w * h, 0xff00_0000);
        let fill = |pixels: &mut Vec<u32>, r: &Rect, value: u32| {
            let r = r.intersect(&full);
            for y in r.y..r.bottom() {
                let row = y as usize * w;
                pixels[row + r.x as usize..row + r.right() as usize].fill(value);
            }
        };
        for (index, item) in plan.items.iter().enumerate() {
            if !item.gain || item.pose.is_some() || item.ghost {
                continue;
            }
            for hole in plan.holes.iter().filter(|h| h.before == index) {
                fill(pixels, &hole.rect, 0);
            }
            let BufferKindRef::Shm(shm) = kind_of(&item.buffer) else { continue };
            let shm = shm.clone();
            let rect = item.rect.translated(-item.drift.0, -item.drift.1);
            let clip = rect.intersect(&full);
            shm.pool.with_bytes(|bytes| {
                for y in clip.y..clip.bottom() {
                    let row = y as usize * w;
                    for x in clip.x..clip.right() {
                        if let Some(src) = shm.pixel(bytes, x - rect.x, y - rect.y) {
                            let d = &mut pixels[row + x as usize];
                            *d = blend(*d, src);
                        }
                    }
                }
            });
        }
        let sigs = self.sigs.get_or_insert_with(|| Box::new([[0; 3]; CELLS]));
        let cell = CELL as usize;
        for r in 0..ROWS {
            for c in 0..COLS {
                let (mut whole, mut left, mut lower) = (0u32, 0u32, 0u32);
                for dy in 0..cell {
                    let row = (r * cell + dy) * w + c * cell;
                    for (dx, p) in pixels[row..row + cell].iter().enumerate() {
                        let l = (54 * ((p >> 16) & 255) + 183 * ((p >> 8) & 255) + 19 * (p & 255)) >> 8;
                        whole += l;
                        left += if dx < 14 { l } else { 0 };
                        lower += if dy >= cell - 14 { l } else { 0 };
                    }
                }
                let n = (cell * cell) as u32;
                sigs[r * COLS + c] = [(whole / n) as u8, (left / (14 * cell as u32)) as u8, (lower / (14 * cell as u32)) as u8];
            }
        }
        self.sample_ready = true;
        Ok(true)
    }

    fn sample_take(&mut self, sigs: &mut [crate::boxdim::Sig; crate::boxdim::CELLS]) -> io::Result<bool> {
        match self.sigs.as_deref() {
            Some(s) if std::mem::take(&mut self.sample_ready) => {
                *sigs = *s;
                Ok(true)
            }
            _ => Ok(false),
        }
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
