//! Small anti-aliased drawing for the pictures the app makes itself: the clock's face and hands, a hero image. Pixels are
//! premultiplied RGBA, as every picture of the toolkit is.

use compositor_protocol::Rgba;

fn over(px: &mut [u8], colour: Rgba, coverage: f32) {
    let k = coverage.clamp(0.0, 1.0);
    if k <= 0.0 {
        return;
    }
    let s = [colour.0, colour.1, colour.2, colour.3].map(|c| f32::from(c) * k);
    let inv = 1.0 - s[3] / 255.0;
    for i in 0..4 {
        px[i] = (s[i] + f32::from(px[i]) * inv).round().clamp(0.0, 255.0) as u8;
    }
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let ap = (p.0 - a.0, p.1 - a.1);
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len2 == 0.0 { 0.0 } else { ((ap.0 * ab.0 + ap.1 * ab.1) / len2).clamp(0.0, 1.0) };
    let d = (p.0 - (a.0 + ab.0 * t), p.1 - (a.1 + ab.1 * t));
    (d.0 * d.0 + d.1 * d.1).sqrt()
}

/// A round-capped line of width `2 * half` from `a` to `b`, only over the pixels its bounding box covers.
pub fn stroke(pixels: &mut [u8], size: u32, a: (f32, f32), b: (f32, f32), half: f32, colour: Rgba) {
    let pad = half + 1.5;
    let (x0, x1) = ((a.0.min(b.0) - pad).floor().max(0.0) as u32, (a.0.max(b.0) + pad).ceil().min(size as f32) as u32);
    let (y0, y1) = ((a.1.min(b.1) - pad).floor().max(0.0) as u32, (a.1.max(b.1) + pad).ceil().min(size as f32) as u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let d = segment_distance((x as f32 + 0.5, y as f32 + 0.5), a, b);
            let at = ((y * size + x) * 4) as usize;
            over(&mut pixels[at..at + 4], colour, 0.5 + (half - d));
        }
    }
}

/// A filled disc.
pub fn disc(pixels: &mut [u8], size: u32, c: (f32, f32), radius: f32, colour: Rgba) {
    stroke(pixels, size, c, c, radius, colour);
}

/// A ring `width` wide just inside `radius`.
pub fn ring(pixels: &mut [u8], size: u32, c: (f32, f32), radius: f32, width: f32, colour: Rgba) {
    let pad = radius + 1.5;
    let (x0, x1) = ((c.0 - pad).floor().max(0.0) as u32, (c.0 + pad).ceil().min(size as f32) as u32);
    let (y0, y1) = ((c.1 - pad).floor().max(0.0) as u32, (c.1 + pad).ceil().min(size as f32) as u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f32 + 0.5 - c.0, y as f32 + 0.5 - c.1);
            let d = (dx * dx + dy * dy).sqrt();
            let outer = (radius - d + 0.5).clamp(0.0, 1.0);
            let inner = (radius - width - d + 0.5).clamp(0.0, 1.0);
            let at = ((y * size + x) * 4) as usize;
            over(&mut pixels[at..at + 4], colour, outer - inner);
        }
    }
}
