//! Box art for the library's tiles. The core's `artwork.resolve` downloads a picture once and keeps it on disk (its own cache, 512 MB,
//! `native/opennow-core/src/artwork_cache.rs`) and answers with the file's path; this module turns such a file into the pixels of one
//! tile: decoded by a small software decoder (JPEG with the decoder's own 1/2, 1/4 and 1/8 scaling, PNG, WebP), cut to the tile's shape
//! (cover: the middle of the picture, nothing stretched), darkened under the title, with its corners rounded, premultiplied for the
//! toolkit's image layer.
//!
//! A tile never waits for any of this. The library draws its tiles at once, with a placeholder picture of the same shape; decoding runs
//! on a worker thread ([`Loader`]) and the picture replaces the placeholder when it arrives. A file that does not decode (a format this
//! module does not read, a damaged download) leaves the placeholder, which is a finished look, not an error state.
//!
//! The hardware JPEG decoder of `jpeg/` (`lumatv-jpeg`) is not used: it is a 32-bit service started by LumaTV's helper and not yet run
//! on the TV (`docs/research/hwjpeg-result.md`), where a tile's picture is about 300x400 pixels after the decoder's scaling, a few
//! milliseconds in this decoder on a worker thread.
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// Files and pictures beyond these are refused before they are decoded.
const MAX_FILE_BYTES: u64 = 16 << 20;
const MAX_PIXELS: u64 = 48_000_000;

/// What one tile's picture looks like: its size, the radius of its corners, the title strip (a solid band `strip` high at the bottom
/// with a fade of `fade` rows above it) and the strip's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub w: u32,
    pub h: u32,
    pub radius: u32,
    pub strip: u32,
    pub fade: u32,
    /// Straight RGB of the strip and its opacity at the solid part (0-255).
    pub scrim: [u8; 3],
    pub scrim_alpha: u8,
}

/// Premultiplied RGBA, `w * 4` bytes a row, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Straight RGB, rows packed.
struct Rgb {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// What a file is, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
}

pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(Format::Png)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Format::Webp)
    } else {
        None
    }
}

/// The part of a picture drawn behind a transparent pixel: the strip's own colour, so a PNG with holes in it reads as part of the tile.
fn flatten(rgba: &[u8], under: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.chunks_exact(4) {
        let a = u32::from(p[3]);
        for c in 0..3 {
            out.push(((u32::from(p[c]) * a + u32::from(under[c]) * (255 - a) + 127) / 255) as u8);
        }
    }
    out
}

fn decode_jpeg(bytes: &[u8], want_w: u32, want_h: u32) -> Result<Rgb, String> {
    let mut d = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    d.read_info().map_err(|e| format!("jpeg: {e}"))?;
    let info = d.info().ok_or("jpeg: no header")?;
    let (sw, sh) = (u32::from(info.width), u32::from(info.height));
    if sw == 0 || sh == 0 || u64::from(sw) * u64::from(sh) > MAX_PIXELS {
        return Err(format!("jpeg: {sw}x{sh} is not a size this decodes"));
    }
    // The scale that still covers the tile: the smallest of 1/8, 1/4, 1/2 and 1 whose output is at least as large as the part of the
    // picture the tile shows, in both axes (the decoder's own rule is "at least one axis", which the proportional request satisfies).
    let factor = (want_w as f32 / sw as f32).max(want_h as f32 / sh as f32).min(1.0);
    let rw = ((sw as f32 * factor).ceil() as u32).clamp(1, u32::from(u16::MAX)) as u16;
    let rh = ((sh as f32 * factor).ceil() as u32).clamp(1, u32::from(u16::MAX)) as u16;
    d.scale(rw, rh).map_err(|e| format!("jpeg: {e}"))?;
    let px = d.decode().map_err(|e| format!("jpeg: {e}"))?;
    let info = d.info().ok_or("jpeg: no header")?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let n = (w * h) as usize;
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 if px.len() == n * 3 => px,
        jpeg_decoder::PixelFormat::L8 if px.len() == n => px.iter().flat_map(|&g| [g, g, g]).collect(),
        jpeg_decoder::PixelFormat::L16 if px.len() == n * 2 => px.chunks_exact(2).flat_map(|g| [g[0], g[0], g[0]]).collect(),
        other => return Err(format!("jpeg: {other:?} with {} bytes for {w}x{h} is not read", px.len())),
    };
    Ok(Rgb { w, h, px: rgb })
}

fn decode_png(bytes: &[u8], under: [u8; 3]) -> Result<Rgb, String> {
    let mut d = png::Decoder::new(Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| format!("png: {e}"))?;
    let (w, h) = (r.info().width, r.info().height);
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!("png: {w}x{h} is not a size this decodes"));
    }
    let mut buf = vec![0u8; r.output_buffer_size()];
    let frame = r.next_frame(&mut buf).map_err(|e| format!("png: {e}"))?;
    let data = &buf[..frame.buffer_size()];
    let px = match frame.color_type {
        png::ColorType::Rgb => data.to_vec(),
        png::ColorType::Rgba => flatten(data, under),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => {
            let rgba: Vec<u8> = data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect();
            flatten(&rgba, under)
        }
        png::ColorType::Indexed => return Err("png: an indexed picture was not expanded".into()),
    };
    Ok(Rgb { w, h, px })
}

fn decode_webp(bytes: &[u8], under: [u8; 3]) -> Result<Rgb, String> {
    let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|e| format!("webp: {e}"))?;
    let (w, h) = d.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!("webp: {w}x{h} is not a size this decodes"));
    }
    let alpha = d.has_alpha();
    let mut buf = vec![0u8; d.output_buffer_size().ok_or("webp: no output size")?];
    d.read_image(&mut buf).map_err(|e| format!("webp: {e}"))?;
    let px = if alpha { flatten(&buf, under) } else { buf };
    Ok(Rgb { w, h, px })
}

fn decode(bytes: &[u8], spec: &Spec) -> Result<Rgb, String> {
    match sniff(bytes) {
        Some(Format::Jpeg) => decode_jpeg(bytes, spec.w, spec.h),
        Some(Format::Png) => decode_png(bytes, spec.scrim),
        Some(Format::Webp) => decode_webp(bytes, spec.scrim),
        None => Err("not a JPEG, PNG or WebP picture".to_owned()),
    }
}

/// For each output pixel of an axis, the first source pixel it reads and the weights of the ones from there: an area average where
/// the picture is shrunk, the two nearest pixels (linear) where it is enlarged. `start` and `len` are the part of the source's axis that
/// shows, in source pixels.
fn axis(src_len: u32, start: f32, len: f32, dst_len: u32) -> Vec<(usize, Vec<f32>)> {
    let step = len / dst_len as f32;
    (0..dst_len)
        .map(|d| {
            let (a, b) = (start + d as f32 * step, start + (d + 1) as f32 * step);
            if step < 1.0 {
                let c = (a + b) / 2.0 - 0.5;
                let i0 = (c.floor().max(0.0) as usize).min(src_len as usize - 1);
                let i1 = (i0 + 1).min(src_len as usize - 1);
                let f = (c - i0 as f32).clamp(0.0, 1.0);
                (i0, if i1 == i0 { vec![1.0] } else { vec![1.0 - f, f] })
            } else {
                let i0 = (a.floor().max(0.0) as usize).min(src_len as usize - 1);
                let i1 = ((b.ceil() as usize).max(i0 + 1)).min(src_len as usize);
                let mut w: Vec<f32> = (i0..i1).map(|i| ((i + 1) as f32).min(b) - (i as f32).max(a)).map(|x| x.max(0.0)).collect();
                let sum: f32 = w.iter().sum();
                if sum <= 0.0 {
                    w = vec![1.0];
                } else {
                    w.iter_mut().for_each(|x| *x /= sum);
                }
                (i0, w)
            }
        })
        .collect()
}

/// The middle of `src` that has the target's shape, scaled to `tw` x `th`.
fn cover(src: &Rgb, tw: u32, th: u32) -> Vec<u8> {
    let (sw, sh) = (src.w as f32, src.h as f32);
    let k = (sw / tw as f32).min(sh / th as f32);
    let (cw, ch) = (tw as f32 * k, th as f32 * k);
    let (cx, cy) = ((sw - cw) / 2.0, (sh - ch) / 2.0);
    let xs = axis(src.w, cx, cw, tw);
    let ys = axis(src.h, cy, ch, th);
    // rows of the source that any output row reads
    let first = ys.first().map_or(0, |y| y.0);
    let last = ys.iter().map(|(i, w)| i + w.len()).max().unwrap_or(0).min(src.h as usize);
    let (stride, tw_us) = (src.w as usize * 3, tw as usize);
    let mut across = vec![0f32; (last - first) * tw_us * 3];
    for (row, y) in (first..last).enumerate() {
        let line = &src.px[y * stride..(y + 1) * stride];
        for (x, (i0, w)) in xs.iter().enumerate() {
            let mut acc = [0f32; 3];
            for (k, wk) in w.iter().enumerate() {
                let p = &line[(i0 + k) * 3..(i0 + k) * 3 + 3];
                acc[0] += p[0] as f32 * wk;
                acc[1] += p[1] as f32 * wk;
                acc[2] += p[2] as f32 * wk;
            }
            let at = (row * tw_us + x) * 3;
            across[at..at + 3].copy_from_slice(&acc);
        }
    }
    let mut out = vec![0u8; tw_us * th as usize * 3];
    for (y, (j0, w)) in ys.iter().enumerate() {
        for x in 0..tw_us {
            let mut acc = [0f32; 3];
            for (k, wk) in w.iter().enumerate() {
                let at = ((j0 + k - first) * tw_us + x) * 3;
                acc[0] += across[at] * wk;
                acc[1] += across[at + 1] * wk;
                acc[2] += across[at + 2] * wk;
            }
            let o = (y * tw_us + x) * 3;
            out[o] = acc[0].round().clamp(0.0, 255.0) as u8;
            out[o + 1] = acc[1].round().clamp(0.0, 255.0) as u8;
            out[o + 2] = acc[2].round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The strip's opacity at row `y` of a tile `h` high: 0 above the fade, rising smoothly to `scrim_alpha`, solid across the strip.
fn scrim_at(spec: &Spec, y: u32) -> f32 {
    let solid_from = spec.h.saturating_sub(spec.strip);
    let fade_from = solid_from.saturating_sub(spec.fade);
    let a = f32::from(spec.scrim_alpha) / 255.0;
    if y >= solid_from {
        a
    } else if y >= fade_from && spec.fade > 0 {
        let t = (y - fade_from) as f32 / spec.fade as f32;
        a * t * t * (3.0 - 2.0 * t)
    } else {
        0.0
    }
}

/// How much of the pixel at (`x`, `y`) of a `w` x `h` tile lies inside its rounded corners, 0 to 1.
fn coverage(x: u32, y: u32, w: u32, h: u32, r: u32) -> f32 {
    if r == 0 {
        return 1.0;
    }
    let r = r as f32;
    let cx = (x as f32 + 0.5).clamp(r, w as f32 - r);
    let cy = (y as f32 + 0.5).clamp(r, h as f32 - r);
    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() - r;
    (0.5 - d).clamp(0.0, 1.0)
}

/// Straight RGB pixels of the tile's shape to the finished tile: the strip, the corners, premultiplied.
fn finish(rgb: &[u8], spec: &Spec) -> Thumb {
    let (w, h) = (spec.w, spec.h);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let s = scrim_at(spec, y);
        for x in 0..w {
            let o = ((y * w + x) * 3) as usize;
            let c = coverage(x, y, w, h, spec.radius);
            for k in 0..3 {
                let v = f32::from(rgb[o + k]) * (1.0 - s) + f32::from(spec.scrim[k]) * s;
                rgba.push((v * c).round() as u8);
            }
            rgba.push((255.0 * c).round() as u8);
        }
    }
    Thumb { w, h, rgba }
}

/// A tile's picture from the bytes of a picture file.
pub fn tile(bytes: &[u8], spec: &Spec) -> Result<Thumb, String> {
    let src = decode(bytes, spec)?;
    Ok(finish(&cover(&src, spec.w, spec.h), spec))
}

/// A tile's picture from a file.
pub fn tile_from_file(path: &std::path::Path, spec: &Spec) -> Result<Thumb, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{}: {} bytes is more than a tile's picture is allowed", path.display(), meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    tile(&bytes, spec)
}

/// What a tile shows until its picture is there (and for good if it never is): a vertical wash from `top` to `bottom`, with the strip and
/// the corners of a real tile, so that nothing moves when the picture arrives.
pub fn placeholder(spec: &Spec, top: [u8; 3], bottom: [u8; 3]) -> Thumb {
    let mut rgb = Vec::with_capacity((spec.w * spec.h * 3) as usize);
    for y in 0..spec.h {
        let t = y as f32 / (spec.h - 1).max(1) as f32;
        for _ in 0..spec.w {
            for k in 0..3 {
                rgb.push((f32::from(top[k]) * (1.0 - t) + f32::from(bottom[k]) * t).round() as u8);
            }
        }
    }
    finish(&rgb, spec)
}

/// One picture to decode: `key` comes back with the answer.
pub struct Job {
    pub key: String,
    pub path: PathBuf,
    pub spec: Spec,
}

pub struct Done {
    pub key: String,
    pub result: Result<Thumb, String>,
}

/// The worker threads and their queues. [`Loader::request`] never blocks; [`Loader::poll`] takes whatever has finished.
pub struct Loader {
    jobs: Option<Sender<Job>>,
    done: Receiver<Done>,
    inflight: usize,
}

impl Loader {
    /// `workers` threads (the TV has four cores and a stream may be starting; one or two is plenty for a screenful of tiles).
    pub fn new(workers: usize) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (tx, done) = mpsc::channel::<Done>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..workers.max(1) {
            let (rx, tx) = (Arc::clone(&rx), tx.clone());
            let _ = thread::Builder::new().name(format!("gfn-art-{i}")).spawn(move || {
                loop {
                    let job = match rx.lock().map(|r| r.recv()) {
                        Ok(Ok(job)) => job,
                        _ => return,
                    };
                    let result = tile_from_file(&job.path, &job.spec);
                    if tx.send(Done { key: job.key, result }).is_err() {
                        return;
                    }
                }
            });
        }
        Self { jobs: Some(jobs), done, inflight: 0 }
    }

    pub fn request(&mut self, job: Job) {
        if let Some(jobs) = &self.jobs {
            if jobs.send(job).is_ok() {
                self.inflight += 1;
            }
        }
    }

    pub fn poll(&mut self) -> Vec<Done> {
        let out: Vec<Done> = self.done.try_iter().collect();
        self.inflight -= out.len();
        out
    }

    /// Requests whose answer has not been taken yet.
    pub fn inflight(&self) -> usize {
        self.inflight
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.jobs = None;   /* the workers' recv fails and they end */
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const SPEC: Spec = Spec { w: 272, h: 360, radius: 22, strip: 96, fade: 40, scrim: [0x12, 0x0E, 0x1A], scrim_alpha: 224 };

    /// A picture with a red left half and a blue right half and a green band across its middle third, `w` x `h`.
    fn bands(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(if y > h / 3 && y < 2 * h / 3 { &[0, 200, 0] } else if x < w / 2 { &[220, 0, 0] } else { &[0, 0, 220] });
            }
        }
        px
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 90).encode(&bands(w, h), w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
        out
    }

    fn png_bytes(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(if alpha { png::ColorType::Rgba } else { png::ColorType::Rgb });
        e.set_depth(png::BitDepth::Eight);
        let rgb = bands(w, h);
        let data: Vec<u8> = if alpha { rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() } else { rgb };
        e.write_header().unwrap().write_image_data(&data).unwrap();
        out
    }

    fn webp_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image_webp::WebPEncoder::new(&mut out).encode(&bands(w, h), w, h, image_webp::ColorType::Rgb8).unwrap();
        out
    }

    fn px(t: &Thumb, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * t.w + x) * 4) as usize;
        [t.rgba[o], t.rgba[o + 1], t.rgba[o + 2], t.rgba[o + 3]]
    }

    #[test]
    fn the_format_is_read_from_the_first_bytes() {
        assert_eq!(sniff(&jpeg(32, 32)), Some(Format::Jpeg));
        assert_eq!(sniff(&png_bytes(8, 8, false)), Some(Format::Png));
        assert_eq!(sniff(&webp_bytes(8, 8)), Some(Format::Webp));
        assert_eq!(sniff(b"GIF89a......"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn a_big_jpeg_is_scaled_while_it_is_decoded_and_covers_the_tile_from_its_middle() {
        // 1200 x 1600 (3:4): the tile is 272 x 360, 0.756, so the picture fills it in width and loses a sliver of height
        let t = tile(&jpeg(1200, 1600), &SPEC).unwrap();
        assert_eq!((t.w, t.h, t.rgba.len()), (272, 360, 272 * 360 * 4));
        let left = px(&t, 40, 60);
        let right = px(&t, 230, 60);
        assert!(left[0] > 180 && left[2] < 40, "red on the left: {left:?}");
        assert!(right[2] > 180 && right[0] < 40, "blue on the right: {right:?}");
        let band = px(&t, 136, 180);
        assert!(band[1] > 150 && band[0] < 40, "the green band across the middle: {band:?}");
    }

    #[test]
    fn a_wide_picture_is_cut_at_its_sides_not_stretched() {
        // 1600 x 900: the middle 680 x 900 shows, so both halves are in it, and the band keeps the picture's own proportions
        let t = tile(&jpeg(1600, 900), &SPEC).unwrap();
        assert!(px(&t, 20, 100)[0] > 150 && px(&t, 250, 100)[2] > 150);
        let band_rows = (0..250).filter(|&y| px(&t, 136, y)[1] > 150 && px(&t, 136, y)[0] < 40).count();
        assert!((110..125).contains(&band_rows), "a third of the 900 rows shows in 120 of the tile's 360: {band_rows}");
    }

    #[test]
    fn a_small_picture_is_enlarged_smoothly() {
        let t = tile(&png_bytes(34, 45, false), &SPEC).unwrap();
        assert_eq!((t.w, t.h), (272, 360));
        let left = px(&t, 40, 100);
        assert!(left[0] > 180, "{left:?}");
    }

    #[test]
    fn png_with_and_without_alpha_and_webp_decode_to_the_same_tile_as_the_jpeg_does_in_shape() {
        for bytes in [png_bytes(300, 400, false), png_bytes(300, 400, true), webp_bytes(300, 400)] {
            let t = tile(&bytes, &SPEC).unwrap();
            assert_eq!((t.w, t.h), (272, 360));
            assert!(px(&t, 40, 100)[0] > 180 && px(&t, 230, 100)[2] > 180);
        }
    }

    #[test]
    fn a_transparent_png_shows_the_strips_colour_where_it_is_clear() {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, 4, 4);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&[0u8; 4 * 4 * 4]).unwrap();
        let t = tile(&out, &SPEC).unwrap();
        assert_eq!(&px(&t, 136, 100)[..3], &SPEC.scrim);
    }

    #[test]
    fn the_corners_are_round_and_the_strip_darkens_the_bottom_and_nothing_overflows_alpha() {
        let t = tile(&jpeg(600, 800), &SPEC).unwrap();
        assert_eq!(px(&t, 0, 0)[3], 0, "the corner pixel is outside the radius");
        assert_eq!(px(&t, 136, 0)[3], 255);
        assert_eq!(px(&t, 0, 180)[3], 255);
        let mid = px(&t, 100, 100);
        let strip = px(&t, 100, 340);
        // solid strip at .88: the picture's colour has mostly gone
        assert!(u32::from(strip[0]) + u32::from(strip[1]) + u32::from(strip[2]) < 150, "{strip:?}");
        assert!(u32::from(mid[0]) + u32::from(mid[1]) + u32::from(mid[2]) > 150, "{mid:?}");
        for p in t.rgba.chunks_exact(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "premultiplied: {p:?}");
        }
        // the fade is monotonic: darker as it goes down
        let sum = |y| { let p = px(&t, 136, y); u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) };
        assert!(sum(240) >= sum(260) && sum(260) >= sum(280));
    }

    #[test]
    fn the_placeholder_has_the_shape_of_a_real_tile() {
        let p = placeholder(&SPEC, [0x2C, 0x26, 0x3A], [0x1F, 0x1D, 0x2E]);
        assert_eq!((p.w, p.h), (272, 360));
        assert_eq!(px(&p, 0, 0)[3], 0);
        assert_eq!(&px(&p, 136, 100)[..3], &[40, 35, 55][..], "row 100 of 360 is .279 of the way from the top colour to the bottom one");
    }

    #[test]
    fn what_is_not_a_picture_or_is_too_big_is_refused_with_a_reason() {
        assert!(tile(b"not a picture at all", &SPEC).unwrap_err().contains("not a JPEG"));
        let mut cut = jpeg(300, 400);
        cut.truncate(cut.len() / 3);
        assert!(tile(&cut, &SPEC).is_err() || tile(&cut, &SPEC).is_ok(), "a cut file is either refused or decodes what is there; it must not panic");
        assert!(tile_from_file(std::path::Path::new("/nonexistent/art.jpg"), &SPEC).unwrap_err().contains("/nonexistent/art.jpg"));
    }

    #[test]
    fn the_loader_decodes_off_the_calling_thread_and_answers_every_request_once() {
        let dir = std::env::temp_dir().join(format!("gfn-art-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.jpg");
        std::fs::write(&good, jpeg(600, 800)).unwrap();
        let bad = dir.join("bad.jpg");
        std::fs::write(&bad, b"nope").unwrap();
        let mut loader = Loader::new(2);
        let started = Instant::now();
        loader.request(Job { key: "good".into(), path: good, spec: SPEC });
        loader.request(Job { key: "bad".into(), path: bad, spec: SPEC });
        loader.request(Job { key: "gone".into(), path: dir.join("missing.jpg"), spec: SPEC });
        assert!(started.elapsed() < Duration::from_millis(100), "a request does not wait for the decode");
        assert_eq!(loader.inflight(), 3);
        let mut got = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        while got.len() < 3 && Instant::now() < end {
            got.extend(loader.poll());
            thread::sleep(Duration::from_millis(5));
        }
        got.sort_by(|a, b| a.key.cmp(&b.key));
        let keys: Vec<&str> = got.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, ["bad", "gone", "good"]);
        assert!(got[0].result.is_err() && got[1].result.is_err() && got[2].result.is_ok());
        assert_eq!(loader.inflight(), 0);
        assert!(loader.poll().is_empty());
        drop(loader);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
