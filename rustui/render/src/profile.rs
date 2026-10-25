//! Where a frame's time goes, on request.
//!
//! `LUMASHOW_PROFILE=1` in the compositor's environment makes the renderer time the parts of every frame and say, in
//! the log every 120 frames, how long each took at the median and at the worst, how many backdrops were made per frame
//! and *which layers* made the cached ones stale. `LUMASHOW_PROFILE=sync` also waits for the GPU (`glFinish`) at the end
//! of each part, so that a part's time is the GPU's and not only the time it took to hand the GPU its work: that costs
//! the frame the overlap of the parts (and on a tile-based GPU each wait is a resolve of everything drawn), so the
//! frame is slower than it is, but what one part costs relative to another is what it is.
//!
//! `LUMASHOW_PROFILE_SKIP=rim,core` (with either of the above) leaves out parts of what is drawn, and so shows what
//! they cost in the real scenes: the frame gets faster by what they took. The parts are `wallpaper`, `shadows`, `raised`
//! (the light and shade copies of raised controls), `core` and `rim` (the two programs that draw a glass surface's body),
//! and `feather` (the fade of feathered clips: they are drawn as hard ones, by the plain programs).
//!
//! Without the variables every call below is one comparison.

use crate::gl::Gles;
use crate::log::log;
use compositor_protocol::LayerId;
use std::collections::HashMap;
use std::time::Instant;

/// Parts of what is drawn that are left out, to see what they cost (`LUMASHOW_PROFILE_SKIP`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skips {
    pub wallpaper: bool,
    pub shadows: bool,
    pub raised: bool,
    pub core: bool,
    pub rim: bool,
    pub feather: bool,
}

impl Skips {
    /// The parts a comma separated list names; the ones it does not know are reported and left out of the list.
    pub fn parse(list: &str) -> Self {
        let mut skips = Self::default();
        for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            match name {
                "wallpaper" => skips.wallpaper = true,
                "shadows" => skips.shadows = true,
                "raised" => skips.raised = true,
                "core" => skips.core = true,
                "rim" => skips.rim = true,
                "feather" => skips.feather = true,
                other => log(format!("LUMASHOW_PROFILE_SKIP: no part called {other:?}")),
            }
        }
        skips
    }
}

/// How many frames a summary covers.
const WINDOW: usize = 120;

/// The parts of a frame that are timed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Bringing textures up to date, placing every layer and planning the backdrops.
    Place,
    /// Making backdrops: the picture of what is below, blurred (one part for all of a frame's).
    Backdrops,
    /// The frame itself.
    Frame,
}

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    place_us: u32,
    backdrops_us: u32,
    frame_us: u32,
    made: u16,
    /// Glass surfaces drawn, and the panel pixels their bodies and their shadows (and raised controls' light and
    /// shade) cover, counted once for each surface: what the frame's fill is made of.
    surfaces: u16,
    body_px: u64,
    shadow_px: u64,
    /// How much of the bodies is drawn by the rim program (the expensive one) and how many vertices the shadows, the
    /// raised controls' copies, the interiors and the rims take.
    rim_px: u64,
    vertices: [u32; 4],
    /// Panel pixels the plain layers (pictures, glyph runs, shapes, meshes) were drawn over, together: the frame's fill besides
    /// the glass, overdraw and all.
    plain_px: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    On,
    /// Waits for the GPU at the end of each part.
    Sync,
}

pub struct Profile {
    mode: Mode,
    skips: Skips,
    mark: Instant,
    now: Sample,
    window: Vec<Sample>,
    /// What each cached backdrop was made of when it was last made, by its first surface: the layers below it and
    /// their versions.
    made_of: HashMap<LayerId, Vec<(LayerId, u64)>>,
    /// How many times each layer made a cached backdrop stale, in this window.
    culprits: HashMap<LayerId, u32>,
    /// The panel's pixels, for saying how many screens a fill is.
    screen_px: u32,
    /// The plain fill of every layer drawn in this window, summed by layer, with what the layer is: which layers the fill is
    /// made of.
    plain_by_layer: HashMap<LayerId, Fill>,
}

/// One plain layer's draw in a frame, for the profile: what it is and the panel pixels it was drawn over (summed over a window
/// in `plain_by_layer`).
#[derive(Clone, Copy, Debug)]
pub struct Fill {
    pub id: LayerId,
    pub kind: &'static str,
    pub size: (u32, u32),
    pub px: u64,
}

impl Profile {
    pub fn from_env(screen_px: u32) -> Self {
        let mode = match std::env::var("LUMASHOW_PROFILE").as_deref() {
            Ok("sync") => Mode::Sync,
            Ok("") | Ok("0") | Err(_) => Mode::Off,
            Ok(_) => Mode::On,
        };
        if mode != Mode::Off {
            log(format!(
                "profiling frames ({}): a summary every {WINDOW} of them",
                if mode == Mode::Sync {
                    "sync"
                } else {
                    "issue times"
                }
            ));
        }
        let skips = if mode == Mode::Off {
            Skips::default()
        } else {
            std::env::var("LUMASHOW_PROFILE_SKIP")
                .map_or_else(|_| Skips::default(), |list| Skips::parse(&list))
        };
        if skips != Skips::default() {
            log(format!("profiling with parts left out: {skips:?}"));
        }
        Self {
            mode,
            skips,
            mark: Instant::now(),
            now: Sample::default(),
            window: Vec::with_capacity(WINDOW),
            made_of: HashMap::new(),
            culprits: HashMap::new(),
            screen_px,
            plain_by_layer: HashMap::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.mode != Mode::Off
    }

    pub fn skips(&self) -> Skips {
        self.skips
    }

    pub fn frame_begins(&mut self) {
        if self.enabled() {
            self.now = Sample::default();
            self.mark = Instant::now();
        }
    }

    /// `part` of the frame is over.
    pub fn part_ends(&mut self, gl: &Gles, part: Part) {
        if !self.enabled() {
            return;
        }
        if self.mode == Mode::Sync {
            gl.finish();
        }
        let us = self.mark.elapsed().as_micros() as u32;
        match part {
            Part::Place => self.now.place_us += us,
            Part::Backdrops => self.now.backdrops_us += us,
            Part::Frame => self.now.frame_us += us,
        }
        self.mark = Instant::now();
    }

    /// The cached backdrop whose first surface is `leader` is being made again from `below` (each layer that is
    /// there and its version): notes which of them are not what it was last made of.
    pub fn remade(&mut self, leader: LayerId, below: Vec<(LayerId, u64)>) {
        if !self.enabled() {
            return;
        }
        if let Some(before) = self.made_of.get(&leader) {
            let was: HashMap<LayerId, u64> = before.iter().copied().collect();
            let is: HashMap<LayerId, u64> = below.iter().copied().collect();
            for (id, version) in &below {
                if was.get(id) != Some(version) {
                    *self.culprits.entry(*id).or_default() += 1;
                }
            }
            for id in was.keys().filter(|id| !is.contains_key(id)) {
                *self.culprits.entry(*id).or_default() += 1;
            }
        }
        self.made_of.insert(leader, below);
    }

    /// What the frame's glass covers: `surfaces` of them, `body_px` pixels of bodies and `shadow_px` of what is drawn
    /// around them.
    pub fn glass(
        &mut self,
        surfaces: u16,
        body_px: u64,
        shadow_px: u64,
        rim_px: u64,
        vertices: [u32; 4],
    ) {
        if self.enabled() {
            self.now.surfaces = surfaces;
            self.now.body_px = body_px;
            self.now.shadow_px = shadow_px;
            self.now.rim_px = rim_px;
            self.now.vertices = vertices;
        }
    }

    /// The panel pixels each plain layer was drawn over in the frame (`renderer::Drawing::fills`): a layer drawn in several
    /// rectangles of a region is in it once for each.
    pub fn fills(&mut self, fills: &[Fill]) {
        if !self.enabled() {
            return;
        }
        for f in fills {
            self.now.plain_px += f.px;
            self.plain_by_layer
                .entry(f.id)
                .or_insert(Fill { px: 0, ..*f })
                .px += f.px;
        }
    }

    pub fn frame_ends(&mut self, made: u16) {
        if !self.enabled() {
            return;
        }
        self.now.made = made;
        self.window.push(self.now);
        if self.window.len() < WINDOW {
            return;
        }
        let of = |f: fn(&Sample) -> u32| -> (f64, f64) {
            let mut v: Vec<u32> = self.window.iter().map(f).collect();
            v.sort_unstable();
            (
                f64::from(v[v.len() / 2]) / 1000.0,
                f64::from(*v.last().unwrap_or(&0)) / 1000.0,
            )
        };
        let (place, backdrops) = (of(|s| s.place_us), of(|s| s.backdrops_us));
        let (frame, total) = (
            of(|s| s.frame_us),
            of(|s| s.place_us + s.backdrops_us + s.frame_us),
        );
        let made_total: u32 = self.window.iter().map(|s| u32::from(s.made)).sum();
        let screen = f64::from(self.screen_px.max(1));
        let mean = |f: fn(&Sample) -> f64| {
            self.window.iter().map(f).sum::<f64>() / self.window.len() as f64
        };
        let (surfaces, bodies, shadows, rims) = (
            mean(|s| f64::from(s.surfaces)),
            mean(|s| s.body_px as f64) / screen,
            mean(|s| s.shadow_px as f64) / screen,
            mean(|s| s.rim_px as f64) / screen,
        );
        let [shadow_v, raised_v, core_v, rim_v] = [
            mean(|s| f64::from(s.vertices[0])),
            mean(|s| f64::from(s.vertices[1])),
            mean(|s| f64::from(s.vertices[2])),
            mean(|s| f64::from(s.vertices[3])),
        ];
        let with_backdrops = self.window.iter().filter(|s| s.made > 0).count();
        let mut culprits: Vec<(LayerId, u32)> = self.culprits.drain().collect();
        culprits.sort_by_key(|&(id, times)| (std::cmp::Reverse(times), id));
        let culprits: Vec<String> = culprits
            .iter()
            .take(6)
            .map(|(id, times)| format!("layer {} x{times}", id.0))
            .collect();
        let plain = mean(|s| s.plain_px as f64) / screen;
        let frames = self.window.len() as f64;
        let mut biggest: Vec<Fill> = self.plain_by_layer.drain().map(|(_, f)| f).collect();
        biggest.sort_by_key(|f| (std::cmp::Reverse(f.px), f.id));
        let biggest: Vec<String> = biggest
            .iter()
            .take(6)
            .map(|f| {
                format!(
                    "layer {} {} {}x{} {:.2}",
                    f.id.0,
                    f.kind,
                    f.size.0,
                    f.size.1,
                    f.px as f64 / frames / screen
                )
            })
            .collect();
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        log(format!(
            "profile{} at unix {unix}: {} frames; place p50 {:.2} max {:.2} | backdrops p50 {:.2} max {:.2} ({made_total} made, in {with_backdrops} frames) | frame p50 {:.2} max {:.2} | total p50 {:.2} max {:.2} ms | glass {surfaces:.1} surfaces, bodies {bodies:.2} screens (rims {rims:.2}), shadows {shadows:.2} screens; vertices: shadows {shadow_v:.0} raised {raised_v:.0} cores {core_v:.0} rims {rim_v:.0}; plain fill {plain:.2} screens, most of it: [{}]; stale because of: [{}]",
            if self.mode == Mode::Sync {
                " (sync)"
            } else {
                ""
            },
            self.window.len(),
            place.0,
            place.1,
            backdrops.0,
            backdrops.1,
            frame.0,
            frame.1,
            total.0,
            total.1,
            biggest.join(", "),
            culprits.join(", ")
        ));
        self.window.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feather_can_be_left_out_to_see_what_it_costs() {
        let skips = Skips::parse("rim, feather");
        assert!(skips.feather && skips.rim && !skips.core && !skips.wallpaper);
        assert!(!Skips::parse("core").feather);
    }
}
