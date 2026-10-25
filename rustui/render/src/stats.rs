//! Counters the present and render threads keep and the command thread reports on request.

use compositor_protocol::Stats;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

#[derive(Default)]
pub struct SharedStats {
    frames: AtomicU64,
    missed: AtomicU64,
    layers: AtomicU32,
    last_render_us: AtomicU32,
    worst_render_us: AtomicU32,
    scanout_planes: AtomicU32,
    groups: AtomicU32,
    motions: AtomicU32,
    captures: AtomicU64,
}

impl SharedStats {
    pub fn set_scanout_planes(&self, planes: usize) {
        self.scanout_planes.store(planes as u32, Relaxed);
    }
    /// One frame reached the screen.
    pub fn frame(&self, render_us: u32, missed: u32) {
        self.frames.fetch_add(1, Relaxed);
        self.missed.fetch_add(u64::from(missed), Relaxed);
        self.last_render_us.store(render_us, Relaxed);
        self.worst_render_us.fetch_max(render_us, Relaxed);
    }

    pub fn set_layers(&self, layers: usize) {
        self.layers
            .store(u32::try_from(layers).unwrap_or(u32::MAX), Relaxed);
    }

    /// The groups and motion tracks in the scene (the scene's, like the layers).
    pub fn set_groups(&self, groups: usize, motions: usize) {
        self.groups
            .store(u32::try_from(groups).unwrap_or(u32::MAX), Relaxed);
        self.motions
            .store(u32::try_from(motions).unwrap_or(u32::MAX), Relaxed);
    }

    /// Plate captures made.
    pub fn captured(&self, captures: u16) {
        self.captures.fetch_add(u64::from(captures), Relaxed);
    }

    /// Starts the counts over for a new client. The layer count is the scene's, not the client's, so it stays.
    pub fn reset(&self) {
        self.frames.store(0, Relaxed);
        self.missed.store(0, Relaxed);
        self.last_render_us.store(0, Relaxed);
        self.worst_render_us.store(0, Relaxed);
    }

    pub fn snapshot(&self) -> Stats {
        Stats {
            frames: self.frames.load(Relaxed),
            missed: self.missed.load(Relaxed),
            layers: self.layers.load(Relaxed),
            last_render_us: self.last_render_us.load(Relaxed),
            scanout_planes: self.scanout_planes.load(Relaxed),
            worst_render_us: self.worst_render_us.load(Relaxed),
            groups: self.groups.load(Relaxed),
            motions: self.motions.load(Relaxed),
            captures: self.captures.load(Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_accumulate_and_the_worst_render_time_sticks() {
        let s = SharedStats::default();
        s.frame(2000, 0);
        s.frame(5000, 2);
        s.frame(1000, 1);
        s.set_layers(7);
        let snap = s.snapshot();
        assert_eq!(
            (
                snap.frames,
                snap.missed,
                snap.layers,
                snap.last_render_us,
                snap.worst_render_us
            ),
            (3, 3, 7, 1000, 5000)
        );
    }

    #[test]
    fn a_new_client_starts_from_zero_but_the_scene_keeps_its_layers() {
        let s = SharedStats::default();
        s.frame(9000, 4);
        s.set_layers(3);
        s.reset();
        let snap = s.snapshot();
        assert_eq!((snap.frames, snap.missed, snap.worst_render_us), (0, 0, 0));
        assert_eq!(snap.layers, 3);
    }
}
