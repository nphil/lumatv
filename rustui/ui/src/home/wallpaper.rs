//! Home's wallpaper: the theme's picture (`theme.wallpaper`, a file of the theme directory) decoded once, sampled for its luma (which
//! decides light or dark glass for an `auto` material) and its dominant tint (the accent, `Tint.js`'s rules, kept in `theme/tint.json`
//! across starts), and drawn by the compositor as the GPU wallpaper (`WallpaperKind::Silk`, the picture through a slow domain warp at
//! `wallpaperMotion.fps`, still when the motion is off or reduced) under the mode's scrim (`wallScrim`: three stops of black at 0, 55
//! and 100 %, two gradient plates). The video wallpaper hook: while the compositor's `wall` topic says the moving wallpaper's window has
//! the hole under Home, the picture and the scrim fade out (`layerMs`) and the panes over it paint at the reading alpha, so the video
//! plane shows through; they come back the frame after the hole closes.

use crate::kit::Kit;
use crate::kit::motion::Beat;
use crate::kit::picture::{self, Fit, Picture};
use compositor_protocol::{Fill, Props, Rgba, Shape, Wallpaper as WallSpec, WallpaperKind};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::scene::{ImageLayer, ShapeLayer, WallpaperLayer};

pub const W: u32 = 1920;
pub const H: u32 = 1080;
const STEP: u32 = 32;
const CLEAR_BELOW: u8 = 128;

/// What a picture says about itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Mean luma 0..1 of the opaque pixels, -1 for none.
    pub luma: f32,
    /// The dominant colour (the most common of 512 bins, near-black skipped) and its share, or none.
    pub tint: Option<(u8, u8, u8, f32)>,
}

/// The dominant tint and the mean luma of `rgba` (premultiplied or straight: the alpha gate reads the same).
pub fn sample(rgba: &[u8]) -> Sample {
    let mut count = [0u32; 512];
    let (mut sum, mut n) = (0.0f64, 0u32);
    for px in rgba.chunks_exact(4) {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        if a < CLEAR_BELOW {
            continue;
        }
        sum += 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
        n += 1;
        if (r as u32) < STEP && (g as u32) < STEP && (b as u32) < STEP {
            continue;
        }
        let bin = ((r as usize >> 5) << 6) | ((g as usize >> 5) << 3) | (b as usize >> 5);
        count[bin] += 1;
    }
    let total = rgba.len() / 4;
    let mut best: Option<(usize, u32)> = None;
    for (bin, &c) in count.iter().enumerate() {
        if c > 0 && best.is_none_or(|b| c > b.1) {
            best = Some((bin, c));
        }
    }
    let tint = best.map(|(bin, c)| {
        let v = |x: usize| (x as u32 * STEP + STEP / 2) as u8;
        (v((bin >> 6) & 7), v((bin >> 3) & 7), v(bin & 7), c as f32 / total.max(1) as f32)
    });
    Sample { luma: if n > 0 { (sum / n as f64 / 255.0) as f32 } else { -1.0 }, tint }
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if mx == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let channel = |p: f32, q: f32, mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    ((channel(p, q, h + 1.0 / 3.0) * 255.0).round() as u8, (channel(p, q, h) * 255.0).round() as u8, (channel(p, q, h - 1.0 / 3.0) * 255.0).round() as u8)
}

/// The accent a control is drawn in: the tint's hue, with saturation and lightness set so it reads on glass of either material.
pub fn accent_from(tint: (u8, u8, u8), light: bool) -> (u8, u8, u8) {
    let (h, s, _) = rgb_to_hsl(tint.0, tint.1, tint.2);
    hsl_to_rgb(h, s.max(if light { 0.35 } else { 0.55 }), if light { 0.42 } else { 0.74 })
}

pub fn hex(c: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// `theme/tint.json` as Home keeps it: the sampled source, luma and tint.
pub fn tint_file(source: &str, s: &Sample) -> Value {
    json!({"source": source, "luma": s.luma, "tint": s.tint.map(|t| json!({"r": t.0, "g": t.1, "b": t.2, "share": t.3}))})
}

/// The wallpaper's file: the theme's name under the theme directory, an absolute path as it is, the shipped picture when neither is there.
pub fn path_of(theme: &Value, data_dir: Option<&Path>, module_dir: Option<&Path>) -> Option<PathBuf> {
    let name = theme.get("wallpaper").and_then(Value::as_str).unwrap_or("wall_1080.png");
    if name.starts_with('/') {
        return Some(PathBuf::from(name));
    }
    let candidates = [
        data_dir.map(|d| d.join("theme").join(name)),
        module_dir.map(|m| m.join("../../wallpaper").join(name)),
        module_dir.map(|m| m.join("../../wallpaper/wall_1080.png")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// The scrim's alphas at the top, the middle and the bottom (`<mode>.wallScrim`).
pub fn scrim_of(theme: &Value, mode: ui::lucent::palette::Mode) -> [f32; 3] {
    let key = if mode == ui::lucent::palette::Mode::Light { "light" } else { "dark" };
    let a = theme.pointer(&format!("/{key}/wallScrim")).and_then(Value::as_array);
    let at = |i: usize| a.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [at(0), at(1), at(2)]
}

/// Whether the theme asks for the `clear` card glass (the default) and not `frosted`.
pub fn clear_glass(theme: &Value) -> bool {
    theme.get("cardGlass").and_then(Value::as_str) != Some("frosted")
}

/// The scrim the Silk wallpaper's shader applies by itself, at the top, the middle and the bottom (`SILK_SCRIM` in `rustui/render/src/wallpapers.rs`).
pub const SILK_SCRIM: [f32; 3] = [0.10, 0.18, 0.46];

pub struct WallpaperSurface {
    /// The prepared set of the plain picture (the `clear` card glass), then the plain picture it captures, then the Silk layer: dropped in this order.
    backdrop: Option<ui::PreparedBackdrop>,
    plain: Option<ImageLayer>,
    plain_hidden: bool,
    layer: Option<WallpaperLayer>,
    scrim_top: Option<ShapeLayer>,
    scrim_bottom: Option<ShapeLayer>,
    z: i32,
    pub source: String,
    pub sample: Option<Sample>,
    hole: bool,
    generation: u64,
    spec: Option<WallSpec>,
    /// Whether the picture is up: the hole took it down.
    shown: bool,
    /// Something is over Home: the warp stands where it is (fps 0, the rest of the look kept, so nothing jumps), the steps owe the screen.
    held: bool,
}

impl WallpaperSurface {
    pub fn new(z: i32) -> Self {
        Self { backdrop: None, plain: None, plain_hidden: false, layer: None, scrim_top: None, scrim_bottom: None, z, source: String::new(), sample: None, hole: false, generation: 0, spec: None, shown: false, held: false }
    }

    /// Holds the warp still while something is over Home, and lets it go on when nothing is: the spec is restyled by the next `load`.
    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// The motion the theme asks for: Silk at the theme's fps and amplitude, still when the motion is off or reduced, and stopped (the same look, no steps)
    /// while `held`.
    fn spec_of(theme: &Value, reduced: bool, held: bool) -> WallSpec {
        let wm = theme.get("wallpaperMotion").cloned().unwrap_or(Value::Null);
        let on = wm.get("enabled").and_then(Value::as_bool) != Some(false) && !reduced;
        let amp = wm.get("amplitude").and_then(Value::as_f64).unwrap_or(4.0) as f32;
        let speed = wm.get("speed").and_then(Value::as_f64).unwrap_or(1.0) as f32;
        let fps = wm.get("fps").and_then(Value::as_u64).unwrap_or(10).clamp(1, 60) as u8;
        WallSpec { kind: WallpaperKind::Silk, speed: if on { speed } else { 0.0 }, intensity: if on { (amp / 16.0).clamp(0.0, 1.0) } else { 0.0 }, fps: if on && !held { fps } else { 0 } }
    }

    /// Loads the picture when the theme names another one (or none is up yet); the sample of a new picture is returned.
    pub fn load(&mut self, kit: &mut Kit, theme: &Value, data_dir: Option<&Path>, module_dir: Option<&Path>) -> std::io::Result<Option<Sample>> {
        let path = path_of(theme, data_dir, module_dir);
        let source = path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let spec = Self::spec_of(theme, kit.ui.look.reduced_motion, self.held);
        if source == self.source && self.layer.is_some() {
            if self.spec != Some(spec)
                && let Some(l) = &self.layer
            {
                l.restyle(spec)?;
                self.spec = Some(spec);
            }
            return Ok(None);
        }
        self.source = source;
        self.drop_backdrop(kit);
        self.layer = None;
        let Some(path) = path else {
            self.sample = None;
            return Ok(None);
        };
        let bytes = std::fs::read(&path)?;
        let pic: Picture = picture::decode(&bytes).map_err(std::io::Error::other)?;
        let fitted = pic.fitted(W, H, Fit::Cover);
        let s = sample(&fitted.rgba);
        let hidden = Props { opacity: if self.hole { 0.0 } else { 1.0 }, visible: !self.hole, ..Props::at(0.0, 0.0) };
        let mut layer = kit.ui.scene.wallpaper(W, H, self.z, spec, hidden)?;
        if let Some(px) = layer.pixels_mut() {
            px.copy_from_slice(&fitted.rgba);
        }
        layer.damage(None);
        self.layer = Some(layer);
        self.spec = Some(spec);
        self.sample = Some(s);
        self.shown = !self.hole;
        Ok(Some(s))
    }

    fn drop_backdrop(&mut self, kit: &mut Kit) {
        kit.wall_backdrop = None;
        self.backdrop = None;
        self.plain = None;
        self.plain_hidden = false;
    }

    /// The glass over the picture, as the person chose it (theme `cardGlass`): `clear` (the default) shows the picture alone, blurred, as the QML Home's
    /// Glass does (it samples the graded wallpaper, and the scrim over the page is not under it); `frosted` shows the page below the glass, scrim and
    /// all, which is the more opaque look. The picture's blur is a prepared set of a plain copy of it, made once; the copy is on the screen (under the
    /// wallpaper that covers it) until the compositor has made the set, then hidden, and the glass is told only when the set can be shown.
    pub fn sync_backdrop(&mut self, kit: &mut Kit, theme: &Value, data_dir: Option<&Path>, module_dir: Option<&Path>) {
        let want = clear_glass(theme) && kit.tier != ui::lucent::material::MaterialTier::T0 && self.layer.is_some();
        if !want {
            if self.backdrop.is_some() {
                self.drop_backdrop(kit);
            }
            return;
        }
        if self.backdrop.is_none() {
            let Some(path) = path_of(theme, data_dir, module_dir) else { return };
            let Ok(bytes) = std::fs::read(&path) else { return };
            let Ok(pic) = picture::decode(&bytes) else { return };
            let fitted = pic.fitted(W, H, Fit::Cover);
            let on = Props::at(0.0, 0.0);
            let Ok(mut plain) = kit.ui.scene.image(W, H, self.z - 1, on) else { return };
            plain.pixels_mut().copy_from_slice(&fitted.rgba);
            plain.damage(None);
            let spec = ui::BackdropSpec { wallpaper: Some(plain.id()), plates: Vec::new(), radius: kit.tokens.blur, downsample: 4 };
            let Ok(set) = kit.ui.scene.prepare_backdrop(&spec) else { return };
            set.request_refresh(ui::ContentGeneration(1));
            self.backdrop = Some(set);
            self.plain = Some(plain);
            self.plain_hidden = false;
        }
        if let (Some(set), Some(plain)) = (&self.backdrop, &self.plain)
            && set.ready().is_some()
        {
            if !self.plain_hidden {
                plain.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
                self.plain_hidden = true;
            }
            kit.wall_backdrop = Some(set.wire_id());
        }
    }

    /// The scrim for the mode, made again when the look changes.
    pub fn restyle(&mut self, kit: &Kit, theme: &Value) {
        if self.generation == kit.generation() && self.scrim_top.is_some() {
            return;
        }
        self.generation = kit.generation();
        // The Silk wallpaper has the dark three-stop scrim in its own shader (`SILK_SCRIM`), as the QML Home's live wallpaper has its mode's: what
        // Home puts over it is only what is still owed to reach the mode's scrim. Home used to lay the whole mode scrim over it, so the page was
        // scrimmed twice (the bottom at .46 twice: 0.29 of the picture, the QML Home's 0.54). A scrim lighter than Silk's (the light material) cannot
        // be taken off again, so it is nothing.
        let target = scrim_of(theme, kit.mode);
        let a: [f32; 3] = std::array::from_fn(|i| (1.0 - (1.0 - target[i]) / (1.0 - SILK_SCRIM[i])).max(0.0));
        let black = |alpha: f32| Rgba(0, 0, 0, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8);
        let top_h = (H as f32 * 0.55).round() as u32;
        let props = Props { opacity: if self.hole { 0.0 } else { 1.0 }, visible: !self.hole, ..Props::at(0.0, 0.0) };
        let shape = |from: Rgba, to: Rgba| Shape { corner_radius: 0.0, fill: Fill::Linear { from, to, angle_deg: 90.0 }, border_width: 0.0, border: Rgba(0, 0, 0, 0) };
        self.scrim_top = Some(kit.ui.scene.shape(W, top_h, self.z + 1, shape(black(a[0]), black(a[1])), props));
        self.scrim_bottom = Some(kit.ui.scene.shape(W, H - top_h, self.z + 1, shape(black(a[1]), black(a[2])), Props { y: top_h as f32, ..props }));
    }

    /// The compositor's `wall` topic: the hole under Home is open (the video wallpaper shows through) or closed.
    pub fn set_hole(&mut self, kit: &Kit, hole: bool) {
        if hole == self.hole {
            return;
        }
        self.hole = hole;
        let mut layers: Vec<&ui::scene::Layer> = Vec::new();
        if let Some(l) = &self.layer {
            layers.push(l);
        }
        if let Some(l) = &self.scrim_top {
            layers.push(l);
        }
        if let Some(l) = &self.scrim_bottom {
            layers.push(l);
        }
        let top_h = (H as f32 * 0.55).round();
        for (i, l) in layers.iter().enumerate() {
            let at = Props::at(0.0, if i == 2 { top_h } else { 0.0 });
            if hole {
                kit.leave(l, at, Beat::Layer);
            } else {
                kit.arrive(l, at, Beat::Layer, false);
            }
        }
        self.shown = !hole;
    }

    pub fn hole(&self) -> bool {
        self.hole
    }

    pub fn is_up(&self) -> bool {
        self.layer.is_some() && self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_wallpaper_keeps_its_look_and_takes_no_steps() {
        let theme = json!({"wallpaperMotion": {"enabled": true, "amplitude": 8.0, "speed": 2.0, "fps": 10}});
        let moving = WallpaperSurface::spec_of(&theme, false, false);
        let held = WallpaperSurface::spec_of(&theme, false, true);
        assert_eq!(moving.fps, 10);
        assert_eq!(held.fps, 0, "no step while something is over Home");
        assert_eq!((held.speed, held.intensity), (moving.speed, moving.intensity), "the look is kept, so that nothing jumps when it goes on");
        let off = json!({"wallpaperMotion": {"enabled": false}});
        assert_eq!(WallpaperSurface::spec_of(&off, false, false), WallpaperSurface::spec_of(&off, false, true), "a still wallpaper is the same held or not");
        assert_eq!(WallpaperSurface::spec_of(&theme, true, false).fps, 0, "reduced motion stills it as it did");
    }

    #[test]
    fn the_sample_skips_black_and_finds_the_dominant_colour_and_the_accent_reads_on_glass() {
        let mut px = Vec::new();
        for i in 0..100 {
            let c: [u8; 4] = if i < 60 { [10, 10, 10, 255] } else if i < 90 { [40, 90, 200, 255] } else { [200, 40, 40, 255] };
            px.extend_from_slice(&c);
        }
        let s = sample(&px);
        let t = s.tint.unwrap();
        assert_eq!((t.0, t.1, t.2), (48, 80, 208));
        assert!((t.3 - 0.3).abs() < 0.001);
        assert!(s.luma > 0.0 && s.luma < 0.4);
        let dark = accent_from((48, 80, 208), false);
        let light = accent_from((48, 80, 208), true);
        let (_, sd, ld) = rgb_to_hsl(dark.0, dark.1, dark.2);
        let (_, _, ll) = rgb_to_hsl(light.0, light.1, light.2);
        assert!(sd >= 0.54 && (ld - 0.74).abs() < 0.02);
        assert!((ll - 0.42).abs() < 0.02);
        assert_eq!(hex((1, 2, 255)), "#0102ff");
        assert_eq!(sample(&[0, 0, 0, 0]).tint, None);
    }
}
