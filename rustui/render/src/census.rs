//! What the compositor holds on the GPU, counted by kind: the memory behind its resident size. A texture's bytes are its width
//! times height times the bytes of a texel (4 for RGBA, 1 for a glyph atlas); the driver's own padding is not here. The tests that
//! pin what a kind holds ask it (`replay_tests::census_recorded_streams` prints it for recorded screens), and with `LUMASHOW_CENSUS=1`
//! the compositor logs it at its quiet moments (`idle.rs`), which is how a resident size is accounted for on a Show.
//!
//! The kinds are the ones that can be given up: a picture layer's texture lives as long as its layer, the pool's free targets are
//! kept for the next frame that wants their size, the prepared backdrops' plates live as long as their set. A kind that grows
//! without a layer or a set to account for it is waste.

use std::collections::BTreeMap;
use std::fmt;

/// How many textures, and how many bytes in them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub count: u32,
    pub bytes: u64,
}

impl Held {
    pub fn texture(&mut self, width: u32, height: u32, texel: u64) {
        self.count += 1;
        self.bytes += u64::from(width) * u64::from(height) * texel;
    }
}

impl fmt::Display for Held {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({} KiB)", self.count, self.bytes.div_ceil(1024))
    }
}

/// The GPU memory the renderer holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Census {
    /// One texture for each picture layer that has been drawn or uploaded.
    pub layers: Held,
    /// The glyph atlases (one byte to a texel).
    pub atlases: Held,
    /// Render targets lying free in the pool, kept for the next picture of their size.
    pub pool_free: Held,
    /// The pool's free targets by size, for a report.
    pub pool_sizes: BTreeMap<(u32, u32), u32>,
    /// The cached backdrops of glass surfaces, and the live ones of the frame drawn last.
    pub backdrops: Held,
    /// The plates of prepared backdrops, finished or being made, and the pictures they are composed into.
    pub plates: Held,
    pub pictures: Held,
    /// The plates of still glass.
    pub still: Held,
    /// Textures of layers that have gone and are not yet freed.
    pub graveyard: u32,
}

impl Census {
    pub fn bytes(&self) -> u64 {
        [
            self.layers,
            self.atlases,
            self.pool_free,
            self.backdrops,
            self.plates,
            self.pictures,
            self.still,
        ]
        .iter()
        .map(|h| h.bytes)
        .sum()
    }
}

impl fmt::Display for Census {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "gpu {} KiB: layers {}, atlases {}, pool free {}, backdrops {}, plates {}, pictures {}, still {}, graveyard {}",
            self.bytes().div_ceil(1024),
            self.layers,
            self.atlases,
            self.pool_free,
            self.backdrops,
            self.plates,
            self.pictures,
            self.still,
            self.graveyard
        )
    }
}
