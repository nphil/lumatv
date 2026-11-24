//! Pictures decoded and theme-composited once on a bounded worker, and the stage art that is built from them.
//!
//! The stage art is one full-screen layer that the plan never owns: the page names the picture it wants (`stage::art`, `over_art`) and
//! [`Art`] changes what the layer shows by a dip (see [`dip`]): focus rests, the old picture fades out, the new one goes into the
//! layer's buffer and fades in. Each picture is decoded with a copy under the theme's scrim, so a layer that opens over the art
//! dips to a darker picture instead of drawing a scrim over it. Everything is in memory.
//!
//! Panel care: the layer is static between dips, as the art always was. A dip changes its opacity for 200 ms and never lifts a pixel
//! above the picture's own, and the dimmed copy is the picture blended towards the scrim, never brighter than the art.
mod ahead;
mod dip;

use super::{
    GameUi,
    paint::{Body, Node},
    parts::motion::ART_DIP_MS,
};
use crate::game::model::{Model, Page};
use compositor_protocol::{Ease, Props};
use dip::{Cue, Dip, Pictures};
use lumagame_meta::{Image, ImageSpec};
use std::{
    io::{self, Read},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use ui::ImageLayer;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stage {
    pub art_height: u32,
    pub fade_from: u32,
    pub fade_to: u32,
    pub canvas: [u8; 3],
    /// The theme's scrim, straight (not premultiplied): what the dimmed copy is blended towards.
    pub scrim: [u8; 4],
    /// Which copy the layer shows: the art, or the art under the scrim.
    pub dim: bool,
    pub curtain: bool,
}
impl Stage {
    /// Whether both decode to the same picture, whichever copy they show.
    fn same_picture(&self, other: &Stage) -> bool {
        let Stage { art_height, fade_from, fade_to, canvas, scrim, dim: _, curtain } = self;
        (*art_height, *fade_from, *fade_to, *canvas, *scrim, *curtain)
            == (other.art_height, other.fade_from, other.fade_to, other.canvas, other.scrim, other.curtain)
    }
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
impl Key {
    /// Whether the key asks for the art under the scrim.
    pub fn dim(&self) -> bool {
        self.spec.stage.as_ref().is_some_and(|stage| stage.dim)
    }
    /// The key of the art itself, whichever copy this one asks for.
    fn base(&self) -> Key {
        let mut key = self.clone();
        if let Some(stage) = &mut key.spec.stage {
            stage.dim = false;
        }
        key
    }
    /// Whether both decode to the same picture: they differ, if at all, in the copy they show.
    fn same_picture(&self, other: &Key) -> bool {
        let (a, b) = (&self.spec, &other.spec);
        self.path == other.path
            && (a.width, a.height, a.radius, a.logo) == (b.width, b.height, b.radius, b.logo)
            && match (&a.stage, &b.stage) {
                (Some(a), Some(b)) => a.same_picture(b),
                (None, None) => true,
                _ => false,
            }
    }
}
pub struct Done {
    pub key: Key,
    pub image: Result<Image, String>,
    /// The picture under the scrim, when the request asked for it.
    pub dim: Option<Image>,
}
enum Job {
    /// Decodes the picture, with its copy under the scrim when the flag is set.
    Decode(Key, bool),
    /// Makes the copy under the scrim of a picture that is already decoded, and hands the picture back with it.
    Scrim(Key, Image),
}
pub struct Loader {
    tx: Option<SyncSender<Job>>,
    rx: Receiver<Done>,
    inflight: usize,
}
impl Loader {
    pub fn new() -> io::Result<Self> {
        Self::with_threads(2, 0)
    }
    /// `nice` is the threads' niceness: the pictures decoded ahead yield the cores to the frame loop.
    pub fn with_threads(threads: usize, nice: i32) -> io::Result<Self> {
        let (tx, jobs) = mpsc::sync_channel::<Job>(16);
        let (done, rx) = mpsc::sync_channel(16);
        let jobs = Arc::new(Mutex::new(jobs));
        for n in 0..threads {
            let jobs = jobs.clone();
            let done = done.clone();
            thread::Builder::new().name(format!("game-decode-{n}")).spawn(move || {
                if nice != 0 {
                    // On Linux `who` 0 is the calling thread, not the process.
                    // SAFETY: setpriority takes plain integers and touches no memory.
                    unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) };
                }
                loop {
                    let job = match jobs.lock() {
                        Ok(rx) => match rx.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        },
                        Err(_) => break,
                    };
                    let done_one = match job {
                        Job::Decode(key, dim) => match decode(&key, dim) {
                            Ok((image, dim)) => Done { key, image: Ok(image), dim },
                            Err(error) => Done { key, image: Err(error), dim: None },
                        },
                        Job::Scrim(key, image) => {
                            let dim = key.spec.stage.as_ref().map(|stage| scrimmed(&image, stage.scrim));
                            Done { key, image: Ok(image), dim }
                        }
                    };
                    if done.send(done_one).is_err() {
                        break;
                    }
                }
            })?;
        }
        Ok(Self { tx: Some(tx), rx, inflight: 0 })
    }
    /// Queues the picture for decoding, with its copy under the scrim when `dim`.
    pub fn request(&mut self, key: Key, dim: bool) -> bool {
        self.send(Job::Decode(key, dim))
    }
    /// Queues the making of the decoded picture's copy under the scrim. The picture is gone for good when this is false.
    pub fn scrim(&mut self, key: Key, image: Image) -> bool {
        self.send(Job::Scrim(key, image))
    }
    fn send(&mut self, job: Job) -> bool {
        if self.inflight >= 16 {
            return false;
        }
        if self.tx.as_ref().is_some_and(|tx| tx.try_send(job).is_ok()) {
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
fn decode(key: &Key, dim: bool) -> Result<(Image, Option<Image>), String> {
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
        round(&mut image.rgba, image.width, image.height, s.radius);
    }
    let dimmed = s.stage.as_ref().filter(|_| dim).map(|stage| scrimmed(&image, stage.scrim));
    Ok((image, dimmed))
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
    // What a pixel takes of the art depends on its column and its row apart, so each is worked out once rather than per pixel.
    let columns: Vec<f32> = (0..w)
        .map(|x| {
            let left = if x < 900 {
                alpha + 0.08 * (1.0 - x as f32 / 900.0)
            } else if x < 1200 {
                alpha * (1.0 - 0.65 * (x - 900) as f32 / 300.0)
            } else {
                alpha * 0.35 * (1.0 - (x - 1200) as f32 / 280.0).clamp(0.0, 1.0)
            };
            1.0 - left
        })
        .collect();
    let canvas = stage.canvas.map(f32::from);
    for (y, row) in rgba.chunks_exact_mut(w as usize * 4).enumerate() {
        let y = y as u32;
        let fade = ((y as f32 - stage.fade_from as f32) / stage.fade_to.saturating_sub(stage.fade_from).max(1) as f32).clamp(0.0, 1.0)
            * if stage.curtain { 0.88 } else { 1.0 };
        let keep = 1.0 - fade;
        let rest = canvas.map(|c| c * fade);
        if y >= h {
            // Below the art the row is the canvas.
            let canvas = canvas.map(|c| (c * keep + c * fade).round() as u8);
            for px in row.chunks_exact_mut(4) {
                px.copy_from_slice(&[canvas[0], canvas[1], canvas[2], 255]);
            }
            continue;
        }
        let top = 1.0 - 0.34 * (1.0 - y as f32 / 200.0).clamp(0.0, 1.0);
        let source = &image.rgba[(y * w) as usize * 4..][..w as usize * 4];
        for ((px, src), column) in row.chunks_exact_mut(4).zip(source.chunks_exact(4)).zip(&columns) {
            let gain = column * top;
            for c in 0..3 {
                px[c] = (f32::from(src[c]) * gain * keep + rest[c]).round() as u8;
            }
            px[3] = 255;
        }
    }
    Image { width: w, height, rgba }
}
/// The opaque picture under `scrim` (straight colour and alpha): what a scrim layer over it would show, made once.
fn scrimmed(image: &Image, scrim: [u8; 4]) -> Image {
    let a = u32::from(scrim[3]);
    let mut rgba = Vec::with_capacity(image.rgba.len());
    for px in image.rgba.chunks_exact(4) {
        for c in 0..3 {
            rgba.push(((u32::from(px[c]) * (255 - a) + u32::from(scrim[c]) * a + 127) / 255) as u8);
        }
        rgba.push(px[3]);
    }
    Image { width: image.width, height: image.height, rgba }
}
/// Rounds the corners of premultiplied `rgba`, `width` by `height`, to `radius`.
pub(super) fn round(rgba: &mut [u8], width: u32, height: u32, radius: u32) {
    let r = radius.min(width / 2).min(height / 2) as f32;
    for y in 0..height {
        if y >= radius && y < height.saturating_sub(radius) {
            continue;
        }
        for x in 0..width {
            if x >= radius && x < width.saturating_sub(radius) {
                continue;
            }
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, width as f32 - r);
            let cy = fy.clamp(r, height as f32 - r);
            let a = (r + 0.5 - (fx - cx).hypot(fy - cy)).clamp(0.0, 1.0);
            let i = ((y * width + x) * 4) as usize;
            for c in &mut rgba[i..i + 4] {
                *c = (f32::from(*c) * a).round() as u8;
            }
        }
    }
}

/// The page's stage art: the loader that decodes it, the pictures held, the dip that decides what the one layer shows, and the layer.
pub struct Art {
    /// One thread: a page has one picture to decode at a time (`Pictures` sends one), and the cover loader keeps its own two.
    loader: Loader,
    pictures: Pictures,
    dip: Dip,
    layer: Option<ImageLayer>,
    /// The plan's props and z for the layer, as of the last plan that had art.
    props: Props,
    z: i32,
    /// The model's clock at the last step.
    now: u64,
    /// Since a page began, until its first plan is done: art that is up by then appears as it is, later art fades in.
    fresh: bool,
    /// Whether the plan under way has art.
    seen: bool,
    /// The picture the layer last showed: when a page begins, it stays held while the page's own is decoded, so that coming back to the
    /// last page finds it.
    last: Option<Key>,
    /// The backdrops of the game pages that Home or the Library lead to, decoded ahead.
    ahead: ahead::Ahead,
    /// The picture that went on the page from `ahead`: it has no copy under the scrim yet, which is made once the page has opened.
    bare: Option<Key>,
    /// Pictures put on the layer since the budget walks last asked, and swaps in all: what the walks hold the art to.
    #[cfg(test)]
    uploads: u32,
    #[cfg(test)]
    swaps: u32,
}
impl Art {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            loader: Loader::with_threads(1, 0)?,
            pictures: Pictures::default(),
            dip: Dip::default(),
            layer: None,
            props: Props::at(0.0, 0.0),
            z: 0,
            now: 0,
            fresh: true,
            seen: false,
            last: None,
            ahead: ahead::Ahead::new()?,
            bare: None,
            #[cfg(test)]
            uploads: 0,
            #[cfg(test)]
            swaps: 0,
        })
    }
    /// Takes in the decodes that have finished: the dip picks the pictures up on its next step.
    pub fn poll(&mut self) {
        while let Some(done) = self.loader.poll() {
            self.pictures.arrive(done);
        }
        self.ahead.poll();
    }
    /// Whether the art has work under way: a decode in the air, a dip that is waiting for focus to rest or running, or a backdrop
    /// waiting to be decoded ahead.
    pub fn busy(&self) -> bool {
        self.loader.busy() || self.dip.pending() || self.ahead.busy()
    }
    /// Names the backdrops of the game pages the tiles on screen lead to, each with how far its tile is from focus (none when the page
    /// has no such tiles): they are decoded ahead, nearest first, so that opening a page finds its art held.
    pub fn want_warm(&mut self, keys: Vec<(u32, Key)>) {
        self.ahead.want(keys);
    }
    /// Puts the page's first picture on it from those decoded ahead, when it is among them.
    fn adopt(&mut self) {
        if self.dip.shown().is_some() {
            return;
        }
        let Some(key) = self.dip.wanted().filter(|key| !key.dim() && self.pictures.needs(key) && self.ahead.holds(key)).cloned() else { return };
        if !self.pictures.make_room(self.last.as_ref()) {
            return;
        }
        if let Some((held, image)) = self.ahead.take(&key) {
            self.pictures.adopt(held, image);
            self.bare = Some(key);
        }
    }
    /// Has the copy under the scrim made for the picture adopted, for the layers that open over the page: a pass over the pixels on the
    /// decode thread, not a second decode.
    fn top_up(&mut self) {
        let Some(key) = self.bare.clone() else { return };
        if !self.dip.shown().is_some_and(|shown| shown.same_picture(&key)) {
            self.bare = None;
            return;
        }
        if let Some((held, image)) = self.pictures.lend(&key) {
            if !self.loader.scrim(held, image) {
                self.pictures.lost();
            }
            self.bare = None;
        }
    }
    /// A new page, or a page that has no art: nothing of the last art stays on a layer, and the next art goes up as it is.
    pub fn reset(&mut self) {
        self.last = self.dip.shown().cloned().or_else(|| self.last.take());
        self.layer = None;
        self.dip = Dip::default();
        self.fresh = true;
    }
    /// Asks for the picture, with its copy under the scrim, when it is not held and no other decode is in the air.
    fn fetch(&mut self, key: &Key) {
        if !self.pictures.needs(key) || self.ahead.flying(key) || !self.pictures.make_room(self.dip.shown().or(self.last.as_ref())) {
            return;
        }
        let base = key.base();
        if self.loader.request(base.clone(), true) {
            self.pictures.fly(base, true);
        }
    }
}

impl GameUi {
    /// The stage node of the plan: the picture the page wants on the stage now. It does not become a retained node; [`Art`] holds the one
    /// layer.
    pub(super) fn keep_art(&mut self, node: Node) {
        let Body::Image(key) = node.body else { return };
        let art = &mut self.art;
        art.seen = true;
        art.props = node.props;
        art.z = node.z;
        art.dip.want(key, art.now);
    }
    /// The art's turn at the start of a show: a new page starts it afresh, then whatever is due runs; the backdrops ahead go on while no
    /// list scrolls. True when the layer was told something.
    pub(super) fn step_art(&mut self, model: &Model) -> io::Result<bool> {
        if self.page != Some(model.view.page) {
            self.art.reset();
        }
        self.art.now = model.now_ms();
        let told = self.run_art(model)?;
        let scrolling = self.groups.values().any(|g| g.motion.is_active());
        self.art.top_up();
        self.art.ahead.run(matches!(model.view.page, Page::Home | Page::Library), scrolling);
        Ok(told)
    }
    /// The end of a plan: a page that put up no art has none on a layer, and what the plan asked for runs at once.
    pub(super) fn end_art(&mut self, model: &Model) -> io::Result<()> {
        let art = &mut self.art;
        if !art.seen {
            art.reset();
        }
        art.seen = false;
        self.run_art(model)?;
        self.art.top_up();
        self.art.fresh = false;
        Ok(())
    }
    fn run_art(&mut self, model: &Model) -> io::Result<bool> {
        let scrolling = self.groups.values().any(|g| g.motion.is_active());
        self.art.adopt();
        let reduced = model.prefs.reduce_motion || self.tokens.motion.reduced;
        let art = &mut self.art;
        let step = art.dip.step(art.now, scrolling, reduced, &art.pictures);
        if let Some(key) = &step.request {
            art.fetch(key);
        }
        let Some(cue) = step.cue else { return Ok(false) };
        match cue {
            Cue::Out => {
                if let Some(layer) = &art.layer {
                    layer.animate(Props { opacity: 0.0, ..art.props }, ART_DIP_MS, Ease::In);
                }
            }
            Cue::In => {
                if let Some(layer) = &art.layer {
                    layer.animate(art.props, ART_DIP_MS, Ease::Out);
                }
            }
            Cue::Drop => art.layer = None,
            Cue::Place(key) => {
                self.lay(&key)?;
                if !self.art.fresh && !reduced {
                    self.fade_in();
                }
            }
            Cue::Swap { key, fade } => {
                self.lay(&key)?;
                #[cfg(test)]
                {
                    self.art.swaps += 1;
                }
                if fade {
                    self.fade_in();
                }
            }
        }
        Ok(true)
    }
    /// Puts the picture on the layer: into its buffer when it fits, on a new layer when there is none.
    fn lay(&mut self, key: &Key) -> io::Result<()> {
        let art = &mut self.art;
        let Some(image) = art.pictures.image(key) else { return Ok(()) };
        match &mut art.layer {
            Some(layer) if (layer.width(), layer.height()) == (image.width, image.height) => {
                layer.pixels_mut().copy_from_slice(&image.rgba);
                layer.damage(None);
            }
            slot => {
                let mut layer = self.ui.scene.image(image.width, image.height, art.z, art.props)?;
                layer.pixels_mut().copy_from_slice(&image.rgba);
                layer.damage(None);
                *slot = Some(layer);
            }
        }
        #[cfg(test)]
        {
            art.uploads += 1;
        }
        Ok(())
    }
    /// The layer comes up from dark to the plan's props.
    fn fade_in(&self) {
        if let Some(layer) = &self.art.layer {
            layer.set(Props { opacity: 0.0, ..self.art.props });
            layer.animate(self.art.props, ART_DIP_MS, Ease::Out);
        }
    }
}

#[cfg(test)]
impl GameUi {
    /// The pictures put on the layer since the last call: the budget walks allow one a frame.
    pub(in crate::game) fn take_stage_uploads(&mut self) -> u32 {
        std::mem::take(&mut self.art.uploads)
    }
    /// How many times the art changed picture, from the first to now.
    pub(in crate::game) fn art_swaps(&self) -> u32 {
        self.art.swaps
    }
    /// Whether the stage art is on its layer.
    pub(in crate::game) fn art_layer_up(&self) -> bool {
        self.art.layer.is_some()
    }
    /// How many layers that cover the whole screen are drawn above the canvas: the art, a wash over it, a page's own floor. The budget
    /// walks hold it to one. It is counted from the retained plan and the art layer, all the page knows of what is drawn (the render
    /// crate has no per-layer geometry to ask): a layer under an opaque one is not counted, as the renderer does not draw it, and a
    /// picture covers nothing, having the alpha of its pixels.
    pub(in crate::game) fn full_screen_layers(&self) -> usize {
        use super::paint::{H, W};
        let covers = |body: &Body| match body {
            Body::Surface(glass) => {
                glass.tint_gradient.map_or(glass.tint.3 == 255, |g| g.top.3 == 255 && g.bottom.3 == 255)
                    || glass.backdrop != compositor_protocol::Backdrop::None
            }
            _ => false,
        };
        let mut layers: Vec<(i32, bool)> = self
            .nodes
            .values()
            .map(|kept| &kept.node)
            .filter(|node| node.id != super::parts::stage::CANVAS && node.frame.w >= W && node.frame.h >= H && node.props.visible && node.props.opacity > 0.0)
            .map(|node| (node.z, node.props.opacity >= 1.0 && covers(&node.body)))
            .collect();
        if self.art.layer.is_some() && !self.art.dip.dark(self.art.now) {
            layers.push((self.art.z, false));
        }
        let floor = layers.iter().filter(|(_, covers)| *covers).map(|(z, _)| *z).max();
        layers.iter().filter(|(z, _)| floor.is_none_or(|floor| *z >= floor)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_page_opens_on_the_picture_decoded_ahead_and_its_copy_under_the_scrim_is_made_after() {
        let stage = Stage { art_height: 1, fade_from: 0, fade_to: 1, canvas: [0; 3], scrim: [0, 0, 0, 128], dim: false, curtain: false };
        let key = Key { path: "page".into(), spec: Spec { width: 1, height: 1, radius: 0, logo: false, stage: Some(stage) } };
        let mut art = Art::new().unwrap();
        art.ahead.keep(key.clone(), Image { width: 1, height: 1, rgba: vec![9; 4] });
        art.dip.want(key.clone(), 0);
        art.adopt();
        assert!(art.pictures.has(&key) && !art.ahead.holds(&key), "the page has the picture, and decodes nothing");
        let step = art.dip.step(0, false, false, &art.pictures);
        assert_eq!(step.cue, Some(Cue::Place(key.clone())));
        assert_eq!(step.request, None);
        art.top_up();
        assert!(art.loader.busy() && art.bare.is_none(), "the copy under the scrim is on its way");
        assert!(!art.pictures.needs(&key), "and the picture is not decoded again meanwhile");
    }

    #[test]
    fn logos_align_by_ink_and_corners_are_premultiplied() {
        let mut image = Image { width: 4, height: 4, rgba: vec![0; 64] };
        image.rgba[20..24].copy_from_slice(&[128, 64, 0, 128]);
        align_logo(&mut image);
        assert_eq!(&image.rgba[48..52], &[128, 64, 0, 128]);
        round(&mut image.rgba, image.width, image.height, 2);
        assert!(image.rgba.chunks_exact(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
    }
    #[test]
    fn the_dimmed_copy_is_the_picture_blended_towards_the_scrim() {
        let image = Image { width: 2, height: 1, rgba: vec![255, 255, 255, 255, 0, 0, 0, 255] };
        let half = scrimmed(&image, [0, 0, 0, 128]);
        assert_eq!(half.rgba, [127, 127, 127, 255, 0, 0, 0, 255]);
        assert_eq!(scrimmed(&image, [9, 9, 9, 0]).rgba, image.rgba);
        assert_eq!(scrimmed(&image, [10, 20, 30, 255]).rgba, [10, 20, 30, 255, 10, 20, 30, 255]);
    }
}
