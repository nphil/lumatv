//! Coverage canvases of one line of text for the text-rendering study (`docs/research/text-render-result.md`, `tools/textsim.py`).
//!
//! `textcov OUT TEXT PX WEIGHT [SCALE [PHASE_X PHASE_Y]]` writes four 8-bit PGM files, `OUT.cov1.pgm`, `OUT.cov3.pgm`, `OUT.ref.pgm` and
//! `OUT.meta.json`:
//!
//! - `cov1`: the line as the UI draws it (`ui::text::Fonts::draw`): fontdue coverage at `PX`, each glyph on a whole pixel.
//! - `cov3`: three samples to the pixel across (the glyph outline rasterised at 3 x `PX`, rows averaged by three, the pen at a
//!   third of a pixel): what a subpixel-antialiased rasteriser starts from.
//! - `ref`: the same line drawn natively at `SCALE` x the UI size (default 2.0104, the OSD scaler's factor), glyph origins where
//!   `cov1` has them times `SCALE`: the picture a 4K UI would put on the panel. Drawn at four times that size and averaged, so
//!   the origin is not rounded to a panel pixel.
//!
//! `PHASE_X`/`PHASE_Y` (panel pixels, 0 to 1) shift where the panel's pixel grid falls on the page's: a line at another place on the screen
//! meets the scaler at another phase.
//!
//! The canvas is the line's advance plus 8 pixels each side, ascent plus descent plus 8 pixels high; the baseline is `meta.baseline`.

use fontdue::{Font, FontSettings};
use std::fs;

const FACES: [&[u8]; 3] = [
    include_bytes!("../../../../LumaShow/lumashow/assets/fonts/Manrope-Regular.ttf"),
    include_bytes!("../../../../LumaShow/lumashow/assets/fonts/Manrope-Medium.ttf"),
    include_bytes!("../../../../LumaShow/lumashow/assets/fonts/Manrope-SemiBold.ttf"),
];
const K: usize = 4;
const PAD: usize = 8;

fn pgm(path: &str, w: usize, h: usize, px: &[u8]) {
    let mut out = format!("P5\n{w} {h}\n255\n").into_bytes();
    out.extend_from_slice(px);
    fs::write(path, out).unwrap();
}

/// Adds a glyph bitmap into a canvas, keeping the larger coverage where two overlap (glyphs of one line barely do).
/// A glyph bitmap `bm` (`bw` x `bh`) and where its top left goes (`left`, `top`) in a canvas `cw` x `ch`.
fn put(canvas: (&mut [u8], usize, usize), bm: (&[u8], usize, usize), at: (i64, i64)) {
    let ((canvas, cw, ch), (bm, bw, bh), (left, top)) = (canvas, bm, at);
    for r in 0..bh {
        let y = top + r as i64;
        if y < 0 || y >= ch as i64 {
            continue;
        }
        for c in 0..bw {
            let x = left + c as i64;
            if x < 0 || x >= cw as i64 {
                continue;
            }
            let a = bm[r * bw + c];
            let d = &mut canvas[y as usize * cw + x as usize];
            *d = (*d as u32 + a as u32).min(255) as u8;
        }
    }
}

fn down(src: &[u8], sw: usize, sh: usize, kx: usize, ky: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (sw / kx, sh / ky);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            for j in 0..ky {
                for i in 0..kx {
                    sum += src[(y * ky + j) * sw + x * kx + i] as u32;
                }
            }
            out[y * w + x] = ((sum as f32 / (kx * ky) as f32) + 0.5) as u8;
        }
    }
    (out, w, h)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (out, text, px, weight) = (&a[1], &a[2], a[3].parse::<f32>().unwrap(), a[4].parse::<usize>().unwrap());
    let scale: f32 = a.get(5).map_or(2.0104, |s| s.parse().unwrap());
    let (phx, phy): (f32, f32) = (a.get(6).map_or(0.0, |s| s.parse().unwrap()), a.get(7).map_or(0.0, |s| s.parse().unwrap()));
    let font = Font::from_bytes(FACES[weight], FontSettings::default()).unwrap();
    let line = font.horizontal_line_metrics(px).unwrap();
    let (asc, desc) = (line.ascent.ceil() as usize, (-line.descent).ceil() as usize);
    let width: f32 = text.chars().map(|c| font.metrics(c, px).advance_width).sum();
    let (w, h) = (width.ceil() as usize + 2 * PAD, asc + desc + 2 * PAD);
    let baseline = PAD + asc;
    let mut origins = Vec::new();

    // cov1: Fonts::draw's placement
    let mut cov1 = vec![0u8; w * h];
    let mut pen = PAD as f32;
    for c in text.chars() {
        let (m, bm) = font.rasterize(c, px);
        let left = (pen + m.xmin as f32).round() as i64;
        let top = (baseline as f32 - m.height as f32 - m.ymin as f32).round() as i64;
        put((&mut cov1, w, h), (&bm, m.width, m.height), (left, top));
        origins.push((pen.round() as i64, m.advance_width));
        pen += m.advance_width;
    }
    pgm(&format!("{out}.cov1.pgm"), w, h, &cov1);

    // cov3: three samples to the pixel, glyph origin at a third of a pixel
    let (w3, h3) = (w * 3, h * 3);
    let mut hi3 = vec![0u8; w3 * h3];
    let mut pen = PAD as f32;
    for c in text.chars() {
        let (m, bm) = font.rasterize(c, px * 3.0);
        let origin = (pen * 3.0).round() as i64;
        let top = ((baseline * 3) as f32 - m.height as f32 - m.ymin as f32).round() as i64;
        put((&mut hi3, w3, h3), (&bm, m.width, m.height), (origin + m.xmin as i64, top));
        pen += font.metrics(c, px).advance_width;
    }
    let (cov3, w3, h3) = down(&hi3, w3, h3, 1, 3);
    pgm(&format!("{out}.cov3.pgm"), w3, h3, &cov3);

    // ref: native at scale x, origins as cov1's
    let (wp, hp) = ((w as f32 * scale).ceil() as usize, (h as f32 * scale).ceil() as usize);
    let (wh, hh) = (wp * K, hp * K);
    let mut hi = vec![0u8; wh * hh];
    for (c, (origin, _)) in text.chars().zip(origins.iter()) {
        let (m, bm) = font.rasterize(c, px * scale * K as f32);
        let ox = ((*origin as f32 * scale + phx) * K as f32).round() as i64;
        let by = ((baseline as f32 * scale + phy) * K as f32).round() as i64;
        put((&mut hi, wh, hh), (&bm, m.width, m.height), (ox + m.xmin as i64, by - m.height as i64 - m.ymin as i64));
    }
    let (r, rw, rh) = down(&hi, wh, hh, K, K);
    pgm(&format!("{out}.ref.pgm"), rw, rh, &r);

    fs::write(
        format!("{out}.meta.json"),
        format!(
            "{{\"w\":{w},\"h\":{h},\"baseline\":{baseline},\"pad\":{PAD},\"scale\":{scale},\"refW\":{rw},\"refH\":{rh},\"origins\":[{}]}}",
            origins.iter().map(|(o, _)| o.to_string()).collect::<Vec<_>>().join(",")
        ),
    )
    .unwrap();
}
