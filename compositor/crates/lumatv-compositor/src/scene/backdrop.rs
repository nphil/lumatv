//! What the app switcher draws between Home and the shell's system layer: one scrim over the whole output and, on it, the pictures
//! of the running apps' windows, each scaled into a rectangle the switcher moves (`switcher.rs`).
//!
//! The pictures are the windows' own buffers drawn a second time, scaled; nothing is copied to the CPU and nothing is read back. A
//! window whose picture is on the video plane (it has a hole) is never given a picture here: the plane cannot be sampled and the
//! switcher shows that app's icon instead. Without a spec (the switcher closed) the plan carries nothing of this and costs nothing.

use wayland_server::Resource;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::wl_buffer::WlBuffer;

use super::Scene;
use crate::surface::{SurfaceId, Surfaces};
use crate::window::WindowType;
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// One picture the switcher wants: the surface's buffer scaled into `rect`, `alpha` opaque, its corners rounded by `radius` pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewSpec {
    pub surface: SurfaceId,
    pub rect: Rect,
    pub alpha: f32,
    pub radius: f32,
}

/// What the switcher asks of one frame: the scrim (premultiplied RGBA, 0 to 1) and the pictures, bottom to top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackdropSpec {
    pub scrim: [f32; 4],
    pub previews: Vec<PreviewSpec>,
}

/// A picture as the renderers draw it.
pub struct PlannedPreview {
    pub surface: SurfaceId,
    pub buffer: WlBuffer,
    /// The buffer's size in pixels.
    pub src: (i32, i32),
    pub y_invert: bool,
    pub rect: Rect,
    pub alpha: f32,
    pub radius: f32,
    /// Its window drew since the last frame: a copy made earlier is stale.
    pub dirty: bool,
}

/// The backdrop in a plan: drawn just before the item at index `before` (the first item of the shell's system layer, or past the last
/// one when that layer is idle), so the layer's own chrome stands on the pictures and no window of LG's is above them.
pub struct Backdrop {
    pub before: usize,
    pub scrim: [f32; 4],
    pub previews: Vec<PlannedPreview>,
}

/// What the last plan drew, to damage what changed and nothing else.
#[derive(Default, PartialEq)]
pub(super) struct BackdropSig {
    scrim: [u32; 4],
    previews: Vec<(SurfaceId, Rect, u32, u32, Option<ObjectId>)>,
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
            let before = self
                .plan
                .items
                .iter()
                .position(|i| self.windows.get(&i.window).is_some_and(|w| w.kind == WindowType::ShellSystem))
                .unwrap_or(self.plan.items.len());
            let mut previews = Vec::with_capacity(spec.previews.len());
            for p in &spec.previews {
                let Some(s) = surfaces.get(p.surface) else { continue };
                let Some(buffer) = s.current.buffer.clone() else { continue };
                let src = s.size();
                let rect = p.rect.intersect(&output);
                if src.0 <= 0 || src.1 <= 0 || rect.is_empty() || p.alpha < 1.0 / 255.0 {
                    continue;
                }
                // A picture whose source drew something since the last frame is drawn again (`None` is everything, an empty list nothing).
                let changed = s.current.damage.as_ref().is_none_or(|d| !d.is_empty());
                dirty |= changed;
                previews.push(PlannedPreview {
                    surface: p.surface,
                    buffer,
                    src,
                    y_invert: s.current.y_invert,
                    rect: p.rect,
                    alpha: p.alpha,
                    radius: p.radius,
                    dirty: changed,
                });
            }
            sig.scrim = spec.scrim.map(f32::to_bits);
            sig.previews = previews.iter().map(|p| (p.surface, p.rect, p.alpha.to_bits(), p.radius.to_bits(), Some(p.buffer.id()))).collect();
            self.plan.backdrop = Some(Backdrop { before, scrim: spec.scrim, previews });
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
