//! Decode and theme-composite once on bounded workers; focus frames only move retained layers.
use lumagame_meta::{Image, ImageSpec};
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stage {
    pub art_height: u32,
    pub fade_from: u32,
    pub fade_to: u32,
    pub canvas: [u8; 3],
    pub dim: bool,
    pub curtain: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spec {
    pub width: u32,
    pub height: u32,
    pub radius: u32,
    pub logo: bool,
    pub stage: Option<Stage>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub path: PathBuf,
    pub spec: Spec,
}
pub struct Done {
    pub key: Key,
    pub image: Result<Image, String>,
}
pub struct Loader {
    tx: Option<SyncSender<Key>>,
    rx: Receiver<Done>,
    inflight: usize,
}
impl Loader {
    pub fn new() -> std::io::Result<Self> {
        let (tx, jobs) = mpsc::sync_channel::<Key>(16);
        let (done, rx) = mpsc::sync_channel(16);
        let jobs = Arc::new(Mutex::new(jobs));
        for n in 0..2 {
            let jobs = jobs.clone();
            let done = done.clone();
            thread::Builder::new().name(format!("game-decode-{n}")).spawn(move || {
                loop {
                    let key = match jobs.lock() {
                        Ok(rx) => match rx.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        },
                        Err(_) => break,
                    };
                    let image = decode(&key);
                    if done.send(Done { key, image }).is_err() {
                        break;
                    }
                }
            })?;
        }
        Ok(Self { tx: Some(tx), rx, inflight: 0 })
    }
    pub fn request(&mut self, key: Key) -> bool {
        if self.inflight >= 16 {
            return false;
        }
        if self.tx.as_ref().is_some_and(|tx| tx.try_send(key).is_ok()) {
            self.inflight += 1;
            true
        } else {
            false
        }
    }
    pub fn poll(&mut self) -> Option<Done> {
        let result = self.rx.try_recv().ok()?;
        self.inflight -= 1;
        Some(result)
    }
    pub fn busy(&self) -> bool {
        self.inflight > 0
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        self.tx = None;
    }
}
fn decode(key: &Key) -> Result<Image, String> {
    let file = std::fs::File::open(&key.path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 16 << 20 {
        return Err("Artwork exceeds 16 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take((16 << 20) + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let s = &key.spec;
    let mut image = lumagame_meta::decode_image(
        &bytes,
        ImageSpec { width: s.width, height: s.stage.as_ref().map_or(s.height, |s| s.art_height), contain: s.logo, trim_alpha: s.logo },
    )
    .map_err(|e| e.to_string())?;
    if s.logo {
        align_logo(&mut image);
    }
    if let Some(stage) = &s.stage {
        image = backdrop(image, s.height, stage);
    }
    if s.radius > 0 {
        round(&mut image, s.radius);
    }
    Ok(image)
}
fn align_logo(image: &mut Image) {
    let (w, h) = (image.width as usize, image.height as usize);
    let (mut left, mut bottom) = (w, 0);
    for (i, px) in image.rgba.chunks_exact(4).enumerate() {
        if px[3] > 24 {
            left = left.min(i % w);
            bottom = bottom.max(i / w + 1);
        }
    }
    if left == w {
        return;
    }
    let mut aligned = vec![0; image.rgba.len()];
    let dy = h - bottom;
    for y in 0..bottom {
        let dst = ((y + dy) * w) * 4;
        let src = (y * w + left) * 4;
        let len = (w - left) * 4;
        aligned[dst..dst + len].copy_from_slice(&image.rgba[src..src + len]);
    }
    image.rgba = aligned;
}
fn backdrop(image: Image, height: u32, stage: &Stage) -> Image {
    let (w, h) = (image.width, image.height);
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let mut histogram = [0u32; 256];
    let mut samples = 0;
    for y in (0..h).step_by(8) {
        for x in (0..w.min(1000)).step_by(8) {
            let p = ((y * w + x) * 4) as usize;
            let lum = 0.2126 * linear(image.rgba[p]) + 0.7152 * linear(image.rgba[p + 1]) + 0.0722 * linear(image.rgba[p + 2]);
            histogram[(lum * 255.0).round() as usize] += 1;
            samples += 1;
        }
    }
    let mut count = 0;
    let p90 = histogram
        .iter()
        .position(|n| {
            count += n;
            count * 10 >= samples * 9
        })
        .unwrap_or(255) as f32
        / 255.0;
    let alpha = (1.0 - (0.021 / p90.max(0.021)).powf(1.0 / 2.2)).clamp(0.72, 0.90);
    let mut rgba = vec![0; (w * height * 4) as usize];
    for y in 0..height {
        let fade = ((y as f32 - stage.fade_from as f32) / stage.fade_to.saturating_sub(stage.fade_from).max(1) as f32).clamp(0.0, 1.0)
            * if stage.curtain { 0.88 } else { 1.0 };
        let top = 0.34 * (1.0 - y as f32 / 200.0).clamp(0.0, 1.0);
        for x in 0..w {
            let left = if x < 900 {
                alpha + 0.08 * (1.0 - x as f32 / 900.0)
            } else if x < 1200 {
                alpha * (1.0 - 0.65 * (x - 900) as f32 / 300.0)
            } else {
                alpha * 0.35 * (1.0 - (x - 1200) as f32 / 280.0).clamp(0.0, 1.0)
            };
            let gain = (1.0 - left) * (1.0 - top) * if stage.dim { 0.5 } else { 1.0 };
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                let art = if y < h { f32::from(image.rgba[i + c]) * gain } else { f32::from(stage.canvas[c]) };
                rgba[i + c] = (art * (1.0 - fade) + f32::from(stage.canvas[c]) * fade).round() as u8;
            }
            rgba[i + 3] = 255;
        }
    }
    Image { width: w, height, rgba }
}
fn round(image: &mut Image, radius: u32) {
    let r = radius.min(image.width / 2).min(image.height / 2) as f32;
    for y in 0..image.height {
        if y >= radius && y < image.height.saturating_sub(radius) {
            continue;
        }
        for x in 0..image.width {
            if x >= radius && x < image.width.saturating_sub(radius) {
                continue;
            }
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, image.width as f32 - r);
            let cy = fy.clamp(r, image.height as f32 - r);
            let a = (r + 0.5 - (fx - cx).hypot(fy - cy)).clamp(0.0, 1.0);
            let i = ((y * image.width + x) * 4) as usize;
            for c in &mut image.rgba[i..i + 4] {
                *c = (f32::from(*c) * a).round() as u8;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logos_align_by_ink_and_corners_are_premultiplied() {
        let mut image = Image { width: 4, height: 4, rgba: vec![0; 64] };
        image.rgba[20..24].copy_from_slice(&[128, 64, 0, 128]);
        align_logo(&mut image);
        assert_eq!(&image.rgba[48..52], &[128, 64, 0, 128]);
        round(&mut image, 2);
        assert!(image.rgba.chunks_exact(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
    }
}
