//! Memory-only app pictures. The scene owns identity and lifetime; GL owns independent copies at half the output's width and height.

use crate::surface::SurfaceId;
use crate::transition::Pose;
use crate::window::{Layer, WindowId};

/// A picture is the app drawn at half the output's size each way. It covers the window for one reopen and dissolves over the app's first
/// real frame, so a bilinear 2x upscale is never on screen for long, and a quarter of the bytes keeps eight apps where three fitted.
pub const PICTURE_WIDTH: usize = crate::OUTPUT_WIDTH as usize / 2;
pub const PICTURE_HEIGHT: usize = crate::OUTPUT_HEIGHT as usize / 2;
pub const PICTURE_BYTES: usize = PICTURE_WIDTH * PICTURE_HEIGHT * 4;
pub const CAPACITY: usize = 8;
pub const BUDGET_BYTES: usize = CAPACITY * PICTURE_BYTES;
/// Failure releases the visual cover, not the app's navigation or lifecycle.
pub const RESUME_CAP_NS: u64 = 1_500_000_000;
/// How long the picture takes to dissolve over the app's first real frame.
pub const FADE_NS: u64 = 140_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resume {
    pub attempt: u64,
    pub after: u64,
    pub since_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub id: SnapshotId,
    pub attempt: u64,
    pub content: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub id: SnapshotId,
    pub window: WindowId,
    pub root: SurfaceId,
    /// Drawn above this window's items, before the next window or the system UI.
    pub before: usize,
    /// Where the picture goes; its opacity already holds the dissolve while `fading`.
    pub pose: Pose,
    pub resume: Option<Resume>,
    /// The app's first real frame is on screen under the picture, which is dissolving: the window's own items are drawn.
    pub fading: bool,
}

pub(crate) struct Picture {
    pub id: SnapshotId,
    pub window: WindowId,
    pub root: SurfaceId,
    pub app: String,
    pub layer: Layer,
    pub order: u64,
    pub resumable: bool,
    pub resume: Option<Resume>,
    /// When the dissolve began: 0 from the receipt until the next plan stamps the time.
    pub fade: Option<u64>,
    /// Made of a window that had video on the plane: its own layer around a transparent hole, or a card. Video arriving over it later
    /// does not invalidate it.
    pub video: bool,
    /// Painted from the theme (`Capture::Card`), not copied from the app's pixels.
    pub card: bool,
    pub used: u64,
}

#[derive(Default)]
pub struct Snapshots {
    pub(crate) pictures: Vec<Picture>,
    serial: u64,
}

/// The picture's opacity `now_ns` into a dissolve that began at `since_ns`: 1 at the start, 0 at [`FADE_NS`], eased at both ends.
pub fn fade_alpha(since_ns: u64, now_ns: u64) -> f32 {
    let t = (now_ns.saturating_sub(since_ns) as f32 / FADE_NS as f32).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

impl Snapshots {
    pub fn contains(&self, id: SnapshotId) -> bool {
        self.pictures.iter().any(|p| p.id == id)
    }

    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.len() * PICTURE_BYTES
    }

    pub(crate) fn get(&self, window: WindowId) -> Option<&Picture> {
        self.pictures.iter().find(|p| p.window == window)
    }

    pub(crate) fn make_room(&mut self, moving: impl Fn(WindowId) -> bool) -> bool {
        if self.len() < CAPACITY { return true; }
        let oldest = self.pictures.iter().enumerate()
            .filter(|(_, p)| p.resume.is_none() && !moving(p.window))
            .min_by_key(|(_, p)| p.used).map(|(i, _)| i);
        if let Some(i) = oldest {
            self.pictures.remove(i);
            true
        } else {
            false
        }
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    pub(crate) fn park(&mut self, window: WindowId) {
        if let Some(p) = self.pictures.iter_mut().find(|p| p.window == window) {
            p.resume = None;
        }
    }

    pub(crate) fn resume(&mut self, window: WindowId, after: u64, now_ns: u64) {
        if !self.get(window).is_some_and(|p| p.resumable && p.resume.is_none() && p.fade.is_none()) {
            return;
        }
        let attempt = self.next();
        let p = self.pictures.iter_mut().find(|p| p.window == window).unwrap();
        p.used = attempt;
        p.resume = Some(Resume { attempt, after, since_ns: now_ns });
    }

    /// Only an actual output receipt can advance the handoff. Old attempts and callback-only commits do not count. The picture is not
    /// released then: the live window is on screen under it, and it dissolves over [`FADE_NS`].
    pub fn presented(&mut self, receipt: Receipt) -> bool {
        let Some(p) = self.pictures.iter_mut().find(|p| p.id == receipt.id) else { return false };
        let Some(r) = p.resume else { return false };
        if r.attempt != receipt.attempt || receipt.content <= r.after {
            return false;
        }
        p.resume = None;
        p.fade = Some(0);
        true
    }

    pub(crate) fn forget(&mut self, window: WindowId) {
        self.pictures.retain(|p| p.window != window);
    }

    /// A destroyed or unmapped window may finish leaving, but can never be resumed from this identity.
    pub(crate) fn closed(&mut self, window: WindowId) {
        if let Some(p) = self.pictures.iter_mut().find(|p| p.window == window) {
            p.resumable = false;
            p.resume = None;
        }
    }

    pub fn clear(&mut self) {
        self.pictures.clear();
    }

    /// Whether a picture is dissolving: every refresh draws a frame of it.
    pub fn fading(&self) -> bool {
        self.pictures.iter().any(|p| p.fade.is_some())
    }

    /// Whether a frame is owed within `within_ns`: a picture dissolving, or one whose cover cap runs out by then.
    pub fn due_within(&self, now_ns: u64, within_ns: u64) -> bool {
        self.pictures.iter().any(|p| p.fade.is_some() || p.resume.is_some_and(|r| r.since_ns + RESUME_CAP_NS <= now_ns.saturating_add(within_ns)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Snapshots {
        let mut s = Snapshots::default();
        s.pictures.push(Picture { id: SnapshotId(1), window: WindowId(2), root: SurfaceId(3), app: "app".into(),
            layer: Layer::Cards, order: 1, resumable: true, resume: None, fade: None, video: false, card: false, used: 1 });
        s.serial = 1;
        s
    }

    #[test]
    fn only_fresh_presented_content_starts_the_dissolve() {
        let mut s = saved();
        s.resume(WindowId(2), 10, 100);
        let r = s.get(WindowId(2)).unwrap().resume.unwrap();
        let mut receipt = Receipt { id: SnapshotId(1), attempt: r.attempt, content: 10 };
        assert!(!s.presented(receipt));
        assert!(!s.fading());
        receipt.content = 11;
        assert!(s.presented(receipt));
        let p = s.get(WindowId(2)).unwrap();
        assert!(p.resume.is_none() && p.fade == Some(0), "the cover stays, as a dissolve, until the plan ends it");
        assert!(s.fading() && s.due_within(0, 0));
        assert!(!s.presented(receipt), "a receipt starts one dissolve");
    }

    #[test]
    fn a_dissolving_picture_is_not_resumed_again() {
        let mut s = saved();
        s.resume(WindowId(2), 10, 100);
        let attempt = s.get(WindowId(2)).unwrap().resume.unwrap().attempt;
        assert!(s.presented(Receipt { id: SnapshotId(1), attempt, content: 11 }));
        s.resume(WindowId(2), 12, 200);
        assert!(s.get(WindowId(2)).unwrap().resume.is_none());
    }

    #[test]
    fn the_dissolve_runs_from_opaque_to_clear_and_eases() {
        assert_eq!(fade_alpha(1_000, 1_000), 1.0);
        assert_eq!(fade_alpha(1_000, 1_000 + FADE_NS), 0.0);
        assert_eq!(fade_alpha(1_000, 1_000 + 10 * FADE_NS), 0.0);
        assert_eq!(fade_alpha(1_000, 0), 1.0, "a start in the future is not a dissolve yet");
        let half = fade_alpha(0, FADE_NS / 2);
        assert!((half - 0.5).abs() < 1e-6);
        assert!(fade_alpha(0, FADE_NS / 10) > 0.97 && fade_alpha(0, FADE_NS * 9 / 10) < 0.03, "slow at both ends");
    }

    #[test]
    fn reversal_and_close_reject_old_receipts() {
        let mut s = saved();
        s.resume(WindowId(2), 10, 100);
        let old = s.get(WindowId(2)).unwrap().resume.unwrap();
        s.park(WindowId(2));
        s.resume(WindowId(2), 11, 200);
        assert!(!s.presented(Receipt { id: SnapshotId(1), attempt: old.attempt, content: 20 }));
        assert!(!s.due_within(200, RESUME_CAP_NS - 1));
        assert!(s.due_within(200, RESUME_CAP_NS));
        s.closed(WindowId(2));
        s.resume(WindowId(2), 20, 300);
        assert!(s.get(WindowId(2)).unwrap().resume.is_none());
        assert!(!s.due_within(300, RESUME_CAP_NS));
    }

    #[test]
    fn eight_half_size_pictures_fit_the_budget_and_the_oldest_inactive_one_is_evicted() {
        let mut s = saved();
        for n in 2..=CAPACITY as u64 {
            let mut p = saved().pictures.remove(0);
            p.id = SnapshotId(n);
            p.window = WindowId(n as u32 + 1);
            p.used = n;
            s.pictures.push(p);
        }
        assert_eq!((PICTURE_WIDTH, PICTURE_HEIGHT, PICTURE_BYTES), (960, 540, 2_073_600));
        assert_eq!(s.len(), 8);
        assert_eq!(s.bytes(), 16_588_800);
        assert_eq!(s.bytes(), BUDGET_BYTES);
        assert!(!s.make_room(|_| true), "active pictures cannot be evicted mid-move");
        assert!(s.make_room(|id| id == WindowId(2)));
        assert!(s.contains(SnapshotId(1)));
        assert!(!s.contains(SnapshotId(2)), "the oldest unprotected picture is evicted deterministically");
    }
}
