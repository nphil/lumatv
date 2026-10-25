//! Feathered clips on the GPU: what a fragment shader with the fade compiled in (`shaders::feathered`) is told.
//!
//! A clip that is feathered is still drawn within its scissor, as a plain clip is; the fade is an alpha ramp, worked out at
//! every pixel from where the pixel is, that the program multiplies its colour by. Its edges and widths are a few numbers for
//! the whole layer, so the program takes them as two vectors (`u_feather`), set once for a run of layers that share a clip
//! and not again until another is asked for: [`Slot`] remembers what its program has.
//!
//! **Where a pixel is** is `gl_FragCoord`, which is the pixel's place in the render target: the panel's pixel, times how much
//! smaller the target is than the panel (a backdrop's small picture), plus where the viewport begins in it (the plate of a
//! prepared backdrop has a margin round what is drawn). [`Uniforms::new`] puts the clip's edges there.

use crate::geometry::Ramp;
use crate::gl::Gles;
use std::cell::Cell;

/// What `u_feather` holds: along each panel axis, for the low edge and the high edge, the ramp as `x * scale + offset`
/// (0 at the edge, 1 at the fade's width in). Two vectors: the four scales `(low x, high x, low y, high y)`, then the four
/// offsets in the same order. An edge with no fade has scale 0 and offset 1: the ramp is 1 everywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uniforms(pub [f32; 8]);

impl Uniforms {
    /// The ramp of a clip as the pixels of a target see it: `factor` target pixels to a panel pixel, the viewport starting at
    /// `origin` target pixels.
    pub fn new(ramp: &Ramp, factor: f32, origin: [i32; 2]) -> Self {
        let mut u = [0.0; 8];
        for axis in 0..2 {
            for side in 0..2 {
                let slot = axis * 2 + side;
                let width = ramp.width[axis][side] * factor;
                let edge = ramp.edge[axis][side] * factor + origin[axis] as f32;
                // The ramp grows inwards: from the low edge it is (x - edge) / width, from the high edge (edge - x) / width.
                let (scale, offset) = if width > 0.0 {
                    let k = 1.0 / width;
                    if side == 0 {
                        (k, -edge * k)
                    } else {
                        (-k, edge * k)
                    }
                } else {
                    (0.0, 1.0)
                };
                u[slot] = scale;
                u[4 + slot] = offset;
            }
        }
        Self(u)
    }

    /// The fade at a pixel whose centre is at (`x`, `y`) in the target, as the fragment shader works it out: the product
    /// over the two axes of the least of the two edges' ramps, each held to 0..=1. Tests check the shader's pixels by this.
    #[cfg(test)]
    pub fn at(&self, x: f32, y: f32) -> f32 {
        let u = &self.0;
        let axis = |p: f32, lo: usize, hi: usize| {
            (p * u[lo] + u[4 + lo])
                .min(p * u[hi] + u[4 + hi])
                .clamp(0.0, 1.0)
        };
        axis(x, 0, 1) * axis(y, 2, 3)
    }
}

/// Where a feathered program has its `u_feather`, and what it holds now.
pub struct Slot {
    location: i32,
    /// What was set last. Not a number to begin with, so that the first call always sets.
    last: Cell<[f32; 8]>,
}

impl Slot {
    pub fn new(location: i32) -> Self {
        Self {
            location,
            last: Cell::new([f32::NAN; 8]),
        }
    }

    /// The slot of a program that has no feather (a plain one): nothing is ever set in it.
    pub fn none() -> Self {
        Self::new(-1)
    }

    /// Tells the program the fade, unless it has it already.
    pub fn set(&self, gl: &Gles, uniforms: &Uniforms) {
        debug_assert!(
            self.location >= 0,
            "a program without a feather was given one"
        );
        if self.last.get() != uniforms.0 {
            self.last.set(uniforms.0);
            gl.uniform4fv(self.location, &uniforms.0);
        }
    }
}
