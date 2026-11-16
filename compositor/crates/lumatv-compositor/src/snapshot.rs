//! Memory-only app pictures. The scene owns identity and lifetime; GL owns independent output-sized copies.

use crate::surface::SurfaceId;
use crate::transition::Pose;
use crate::window::{Layer, WindowId};

pub const BUDGET_BYTES: usize = 24 * 1024 * 1024;
pub const PICTURE_BYTES: usize = crate::OUTPUT_WIDTH as usize * crate::OUTPUT_HEIGHT as usize * 4;
pub const CAPACITY: usize = BUDGET_BYTES / PICTURE_BYTES;
/// Failure releases the visual cover, not the app's navigation or lifecycle.
pub const RESUME_CAP_NS: u64 = 1_500_000_000;

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
    pub pose: Pose,
    pub resume: Option<Resume>,
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
    pub used: u64,
}

#[derive(Default)]
pub struct Snapshots {
    pub(crate) pictures: Vec<Picture>,
    serial: u64,
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
        if !self.get(window).is_some_and(|p| p.resumable && p.resume.is_none()) {
            return;
        }
        let attempt = self.next();
        let p = self.pictures.iter_mut().find(|p| p.window == window).unwrap();
        p.used = attempt;
        p.resume = Some(Resume { attempt, after, since_ns: now_ns });
    }

    /// Only an actual output receipt can advance the handoff. Old attempts and callback-only commits do not count.
    pub fn presented(&mut self, receipt: Receipt) -> bool {
        let Some(p) = self.pictures.iter_mut().find(|p| p.id == receipt.id) else { return false };
        let Some(r) = p.resume.as_mut() else { return false };
        if r.attempt != receipt.attempt || receipt.content <= r.after {
            return false;
        }
        self.pictures.retain(|p| p.id != receipt.id);
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

    pub fn due_within(&self, now_ns: u64, within_ns: u64) -> bool {
        self.pictures.iter().any(|p| p.resume.is_some_and(|r| r.since_ns + RESUME_CAP_NS <= now_ns.saturating_add(within_ns)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Snapshots {
        let mut s = Snapshots::default();
        s.pictures.push(Picture { id: SnapshotId(1), window: WindowId(2), root: SurfaceId(3), app: "app".into(),
            layer: Layer::Cards, order: 1, resumable: true, resume: None, used: 1 });
        s.serial = 1;
        s
    }

    #[test]
    fn only_fresh_presented_content_can_release_the_cover() {
        let mut s = saved();
        s.resume(WindowId(2), 10, 100);
        let r = s.get(WindowId(2)).unwrap().resume.unwrap();
        let mut receipt = Receipt { id: SnapshotId(1), attempt: r.attempt, content: 10 };
        assert!(!s.presented(receipt));
        receipt.content = 11;
        assert!(s.presented(receipt));
        assert!(s.is_empty());
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
    fn the_byte_budget_evicts_the_oldest_inactive_picture() {
        let mut s = saved();
        for n in 2..=CAPACITY as u64 {
            let mut p = saved().pictures.remove(0);
            p.id = SnapshotId(n);
            p.window = WindowId(n as u32 + 1);
            p.used = n;
            s.pictures.push(p);
        }
        assert_eq!(s.len(), 3);
        assert_eq!(s.bytes(), 24_883_200);
        assert!(s.bytes() <= BUDGET_BYTES);
        assert!(!s.make_room(|_| true), "active pictures cannot be evicted mid-move");
        assert!(s.make_room(|id| id == WindowId(2)));
        assert!(s.contains(SnapshotId(1)));
        assert!(!s.contains(SnapshotId(2)), "the oldest unprotected picture is evicted deterministically");
    }
}
