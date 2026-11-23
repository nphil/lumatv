//! What the task manager draws between Home and the shell's system layer: one scrim over the whole output and, on it, the pictures of
//! the running apps' windows, each scaled into a rectangle the switcher moves (`switcher.rs`), and the parts of the basic screen.
//!
//! The pictures are the windows' own buffers drawn a second time, scaled, or the small copy the renderer kept of an app that left the
//! front (a client releases its buffer then); nothing is read back to the CPU. A window whose picture is on the video plane (it has a
//! hole) is never given a picture here: the plane cannot be sampled and the switcher shows that app's icon instead. The basic screen
//! (`taskmgr/fallback.rs`) arrives as overlays: finished pixels the CPU painted, drawn over the scrim and under the pictures. Without a
//! spec (the switcher closed) the plan carries nothing of this and costs nothing.

use std::fmt;
use std::sync::Arc;

use wayland_server::Resource;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::wl_buffer::WlBuffer;

use super::Scene;
use crate::surface::{SurfaceId, Surfaces};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// Where a picture comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewSource {
    /// A window's current buffer.
    Window(SurfaceId),
    /// The copy the renderer kept of this app's last frame (`Output::keep_thumb`).
    Thumb(String),
}

/// One picture the switcher wants: the source scaled into `rect`, `alpha` opaque, its corners rounded by `radius` pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewSpec {
    pub source: PreviewSource,
    pub rect: Rect,
    pub alpha: f32,
    pub radius: f32,
}

/// A part of the basic screen: premultiplied RGBA8, rows top first, `rect.w * rect.h * 4` bytes. `rev` is new whenever the pixels are.
#[derive(Clone)]
pub struct Overlay {
    pub key: u8,
    pub rev: u64,
    pub rect: Rect,
    pub rgba: Arc<Vec<u8>>,
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Overlay) -> bool {
        self.key == other.key && self.rev == other.rev && self.rect == other.rect
    }
}

impl fmt::Debug for Overlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlay").field("key", &self.key).field("rev", &self.rev).field("rect", &self.rect).finish_non_exhaustive()
    }
}

/// What the switcher asks of one frame: the scrim (premultiplied RGBA, 0 to 1), the parts of the basic screen and the pictures, bottom to top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackdropSpec {
    pub scrim: [f32; 4],
    pub overlays: Vec<Overlay>,
    pub previews: Vec<PreviewSpec>,
}

/// What a planned picture is drawn from.
pub enum PlannedSource {
    Window {
        surface: SurfaceId,
        buffer: WlBuffer,
        /// The buffer's size in pixels.
        src: (i32, i32),
        y_invert: bool,
        /// Its window drew since the last frame: a copy made earlier is stale.
        dirty: bool,
    },
    Thumb(String),
}

/// A picture as the renderers draw it.
pub struct PlannedPreview {
    pub source: PlannedSource,
    pub rect: Rect,
    pub alpha: f32,
    pub radius: f32,
}

/// The backdrop follows all client windows and sits under native UI chrome.
pub struct Backdrop {
    pub before: usize,
    pub scrim: [f32; 4],
    pub overlays: Vec<Overlay>,
    pub previews: Vec<PlannedPreview>,
}

/// What the last plan drew, to damage what changed and nothing else.
#[derive(Default, PartialEq)]
pub(super) struct BackdropSig {
    scrim: [u32; 4],
    overlays: Vec<(u8, u64, Rect)>,
    previews: Vec<(PreviewSource, Rect, u32, u32, Option<ObjectId>)>,
}

impl Scene {
    /// The switcher's frame: where its pictures are and how dark its scrim is now. `None` once it is closed.
    pub fn set_backdrop(&mut self, spec: Option<BackdropSpec>) {
        self.backdrop = spec;
    }

    /// Builds the plan's backdrop from the spec and damages what moved. Called by `plan` after the items.
    pub(super) fn plan_backdrop(&mut self, surfaces: &Surfaces) {
        let output = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        let mut sig = BackdropSig::default();
        let mut dirty = false;
        if let Some(spec) = &self.backdrop {
            let before = self.plan.items.len();
            let mut previews = Vec::with_capacity(spec.previews.len());
            let mut keys = Vec::with_capacity(spec.previews.len());
            for p in &spec.previews {
                if p.rect.intersect(&output).is_empty() || p.alpha < 1.0 / 255.0 {
                    continue;
                }
                let source = match &p.source {
                    PreviewSource::Thumb(app) => PlannedSource::Thumb(app.clone()),
                    PreviewSource::Window(surface) => {
                        let Some(s) = surfaces.get(*surface) else { continue };
                        let Some(buffer) = s.current.buffer.clone() else { continue };
                        let src = s.size();
                        if src.0 <= 0 || src.1 <= 0 {
                            continue;
                        }
                        // A picture whose source drew something since the last frame is drawn again (`None` is everything, an empty list nothing).
                        let changed = s.current.damage.as_ref().is_none_or(|d| !d.is_empty());
                        dirty |= changed;
                        PlannedSource::Window { surface: *surface, buffer, src, y_invert: s.current.y_invert, dirty: changed }
                    }
                };
                let buffer_id = match &source {
                    PlannedSource::Window { buffer, .. } => Some(buffer.id()),
                    PlannedSource::Thumb(_) => None,
                };
                keys.push((p.source.clone(), p.rect, p.alpha.to_bits(), p.radius.to_bits(), buffer_id));
                previews.push(PlannedPreview { source, rect: p.rect, alpha: p.alpha, radius: p.radius });
            }
            sig.scrim = spec.scrim.map(f32::to_bits);
            sig.overlays = spec.overlays.iter().map(|o| (o.key, o.rev, o.rect)).collect();
            sig.previews = keys;
            self.plan.backdrop = Some(Backdrop { before, scrim: spec.scrim, overlays: spec.overlays.clone(), previews });
        }
        if sig == self.last_backdrop && !dirty {
            return;
        }
        let scrim_changed = sig.scrim != self.last_backdrop.scrim;
        let reach = |r: &Rect, plan: &mut super::Plan| {
            let r = r.intersect(&output);
            if !r.is_empty() {
                plan.damage = plan.damage.union(&r);
                if let Some(t) = plan.tiles.as_mut() {
                    t.add(&r);
                }
            }
        };
        if scrim_changed {
            reach(&output, &mut self.plan);
        }
        for (_, r, _, _, _) in self.last_backdrop.previews.iter().chain(sig.previews.iter()) {
            reach(r, &mut self.plan);
        }
        if self.last_backdrop.overlays != sig.overlays {
            for (_, _, r) in self.last_backdrop.overlays.iter().chain(sig.overlays.iter()) {
                reach(r, &mut self.plan);
            }
        }
        self.last_backdrop = sig;
    }
}

/// How much of the pixel at (`x`, `y`) of a `w` by `h` picture with corners of `radius` is inside it: 1 well inside, 0 outside the rounded
/// rectangle, a one-pixel ramp across its edge. The fragment shader of `render/gl.rs` computes the same signed distance.
pub fn coverage(x: f32, y: f32, w: f32, h: f32, radius: f32) -> f32 {
    let r = radius.clamp(0.0, w.min(h) / 2.0);
    let (px, py) = ((x - w / 2.0).abs(), (y - h / 2.0).abs());
    let (qx, qy) = (px - (w / 2.0 - r), py - (h / 2.0 - r));
    let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}
