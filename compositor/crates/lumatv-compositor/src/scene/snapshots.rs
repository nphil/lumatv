use super::*;
use crate::snapshot::{CAPACITY, Draw, Picture, RESUME_CAP_NS, SnapshotId};

impl Scene {
    fn owns_surface(&self, root: SurfaceId, mut surface: SurfaceId, surfaces: &Surfaces) -> bool {
        loop {
            if root == surface { return true; }
            // A grouped surface can keep its Toplevel role and belong to several owners; match the requested owner's memberships.
            if self.groups.iter().any(|group| group.owner == root && group.has_member(surface)) { return true; }
            surface = match surfaces.get(surface).map(|s| s.role) {
                Some(Role::Subsurface { parent }) => parent,
                _ => return false,
            };
        }
    }

    fn window_has_video(&self, window: WindowId, surfaces: &Surfaces) -> bool {
        let Some(w) = self.windows.get(&window) else { return false };
        crate::switcher::input_app(w.app_id()) || self.holes.iter().any(|h| self.owns_surface(w.surface, h.surface, surfaces))
    }

    pub(crate) fn window_video(&self, window: WindowId, surfaces: &Surfaces) -> bool {
        if self.window_has_video(window, surfaces) { return true; }
        let Some(w) = self.windows.get(&window) else { return false };
        // An export reserves a future video element. Only a successful UI copy can override its conservative cold-launch fade.
        self.snapshots.get(window).is_none() && self.video.iter().any(|s| self.owns_surface(w.surface, *s, surfaces))
    }

    /// Reserves a bounded slot before copying the last drawn app group, never the output page.
    pub(crate) fn reserve_snapshot(&mut self, window: WindowId, surfaces: &Surfaces) -> Option<SnapshotId> {
        if !self.transitions.enabled() || self.transitions.suspended || self.last_front != Front::Card(window) {
            return None;
        }
        let w = self.windows.get(&window)?;
        if w.app_id().is_empty() || w.app_id() == HOME_APP || self.window_has_video(window, surfaces)
            || !matches!(w.kind, WindowType::Card | WindowType::Overlay) || w.in_group
        {
            return None;
        }
        if self.snapshots.get(window).is_some_and(|p| p.root == w.surface && p.app == w.app_id() && p.resumable) {
            self.snapshots.park(window);
            return None;
        }
        self.snapshots.forget(window);
        // An undrawn commit may have replaced or changed the source since the last plan; use the ordinary transition instead of guessing.
        let full = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        let items = || self.plan.items.iter().filter(|i| i.window == window);
        if !items().any(|i| i.surface == w.surface && i.rect == full)
            || items().any(|i| i.xform || i.drift != (0, 0) || i.dim != 1.0 || !self.owns_surface(w.surface, i.surface, surfaces)
                || !surfaces.get(i.surface).is_some_and(|s| !s.current.undrawn && s.current.buffer.as_ref() == Some(&i.buffer)))
            || self.plan.gain_on || self.plan.masks.iter().any(|m| m.window == window)
            || self.plan.holes.iter().any(|h| self.plan.items.get(h.before).is_some_and(|i| i.window == window))
        {
            return None;
        }
        if !self.snapshots.make_room(|w| self.transitions.track_of(w).is_some()) {
            return None;
        }
        let id = SnapshotId(self.snapshots.next());
        self.snapshots.pictures.push(Picture { id, window, root: w.surface, app: w.app_id().into(),
            layer: w.kind.layer(), order: w.map_serial, resumable: true, resume: None, used: id.0 });
        Some(id)
    }

    pub fn invalidate_snapshot(&mut self, window: WindowId) {
        if self.snapshots.get(window).is_some() {
            self.snapshots.forget(window);
            self.redraw_all = true;
        }
    }

    pub(crate) fn resume_snapshot(&mut self, window: WindowId, surfaces: &Surfaces) {
        if self.transitions.enabled() && !self.transitions.suspended {
            // Taken before exposed/fullscreen is sent, so a repaint in response is newer even before the next plan.
            self.snapshots.resume(window, surfaces.content_serial(), 0);
        }
    }

    pub(super) fn step_snapshots(&mut self, surfaces: &Surfaces, now_ns: u64) {
        let was = self.snapshots.len();
        if !self.transitions.enabled() {
            self.snapshots.clear();
        } else {
            let windows = &self.windows;
            let mut invalid = [None; CAPACITY];
            for (p, slot) in self.snapshots.pictures.iter().zip(&mut invalid) {
                let video = self.window_has_video(p.window, surfaces);
                if windows.get(&p.window).is_some_and(|w| w.surface != p.root || w.app_id() != p.app
                    || !matches!(w.kind, WindowType::Card | WindowType::Overlay) || w.in_group)
                    || video
                    // Switcher activation bypasses the resume track and can already be showing newer live pixels.
                    || (self.transitions.suspended && self.front == Front::Card(p.window))
                {
                    *slot = Some((p.window, video));
                }
            }
            for (window, video) in invalid.into_iter().flatten() {
                if video { self.transitions.stationary_video(window, now_ns); }
                self.snapshots.forget(window);
            }
            for p in &mut self.snapshots.pictures {
                if self.transitions.suspended || self.front != Front::Card(p.window) {
                    p.resume = None;
                }
                if let Some(r) = &mut p.resume
                    && r.since_ns == 0
                {
                    r.since_ns = now_ns;
                }
            }
            self.snapshots.pictures.retain(|p| {
                (p.resumable || self.transitions.track_of(p.window).is_some())
                    && p.resume.is_none_or(|r| now_ns.saturating_sub(r.since_ns) < RESUME_CAP_NS)
            });
        }
        self.redraw_all |= was != self.snapshots.len();
    }

    pub(super) fn push_snapshot(&mut self, window: WindowId, now_ns: u64) {
        if self.transitions.suspended { return; }
        let Some(p) = self.snapshots.get(window) else { return };
        let track = self.transitions.track_of(window);
        if track.is_none() && p.resume.is_none() { return; }
        let full = FRect::new(0.0, 0.0, OUTPUT_WIDTH as f32, OUTPUT_HEIGHT as f32);
        let pose = track.map_or_else(|| Pose::rest(full), |t| t.pose_at(now_ns));
        self.plan.snapshots.push(Draw { id: p.id, window, root: p.root, before: self.plan.items.len(), pose, resume: p.resume });
    }

    pub(super) fn damage_snapshots(&mut self, surfaces: &Surfaces, output: Rect) {
        let preparing = self.plan.snapshots.iter().any(|p| p.resume.is_some_and(|r| {
            p.pose.is_rest_on(&FRect::from_rect(output)) && self.plan.items.iter().filter(|i| i.window == p.window)
                .any(|i| surfaces.get(i.surface).is_some_and(|s| s.current.content_serial > r.after))
        }));
        if preparing || self.plan.snapshots != self.last_snapshots {
            self.plan.damage = self.plan.damage.union(&output);
            self.plan.tiles.as_mut().unwrap().add(&output);
        }
        self.last_snapshots.clone_from(&self.plan.snapshots);
    }
}

impl crate::State {
    /// Called before background/exposure notifications let the client change or release its picture.
    pub(crate) fn freeze_app(&mut self, window: WindowId) {
        let Some(id) = self.scene.reserve_snapshot(window, &self.surfaces) else { return };
        if !self.output.freeze_app(id, window, self.scene.last_plan(), &self.scene.snapshots) {
            self.scene.invalidate_snapshot(window);
        } else {
            self.scene.transitions.confirm_ui_picture(window);
        }
        self.output.retain_app_snapshots(&self.scene.snapshots);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> (Scene, Surfaces, WindowId, SurfaceId) {
        let mut scene = Scene::default();
        scene.transitions.settings.enabled = true;
        let mut surfaces = Surfaces::default();
        let root = surfaces.create();
        let window = scene.create_window(root);
        scene.windows.get_mut(&window).unwrap().set_property("appId", "org.example.app");
        scene.set_mapped(root, true);
        let id = SnapshotId(scene.snapshots.next());
        scene.snapshots.pictures.push(Picture { id, window, root, app: "org.example.app".into(),
            layer: Layer::Cards, order: 1, resumable: true, resume: None, used: id.0 });
        (scene, surfaces, window, root)
    }

    #[test]
    fn identity_changes_and_remapping_cannot_reuse_a_previous_picture() {
        for replace_root in [false, true] {
            let (mut scene, mut surfaces, window, _) = saved();
            let w = scene.windows.get_mut(&window).unwrap();
            if replace_root {
                w.surface = surfaces.create();
            } else {
                w.set_property("appId", "org.example.other");
            }
            scene.step_snapshots(&surfaces, 1);
            assert!(scene.snapshots.is_empty());
        }
        let (mut scene, _, window, root) = saved();
        scene.set_mapped(root, false);
        assert!(!scene.snapshots.get(window).unwrap().resumable);
        scene.set_mapped(root, true);
        assert!(scene.snapshots.is_empty(), "even the same surface and app id form a new mapping");
    }

    #[test]
    fn a_video_element_in_a_child_invalidates_the_owners_saved_ui() {
        let (mut scene, mut surfaces, _, root) = saved();
        let member = surfaces.create();
        surfaces.get_mut(member).unwrap().role = Role::Toplevel;
        let first_owner = surfaces.create();
        scene.groups.push(Group {
            name: "other-app".into(), owner: first_owner, allow_anonymous: true, anonymous: vec![(member, 1)], layers: BTreeMap::new(),
        });
        scene.groups.push(Group {
            name: "app-tools".into(), owner: root, allow_anonymous: false, anonymous: Vec::new(),
            layers: BTreeMap::from([("tools".into(), GroupLayer { z: 1, key_index: 0, member: Some(member) })]),
        });
        let child = surfaces.create();
        surfaces.get_mut(child).unwrap().role = Role::Subsurface { parent: member };
        assert!(scene.owns_surface(first_owner, child, &surfaces));
        assert!(scene.owns_surface(root, child, &surfaces), "an earlier membership must not hide this app's group");
        let unrelated = surfaces.create();
        assert!(!scene.owns_surface(unrelated, child, &surfaces), "membership never extends to an unrelated app");
        scene.set_video_expected(child, true);
        scene.step_snapshots(&surfaces, 1);
        assert!(!scene.snapshots.is_empty(), "a dormant exported element is not a video picture");
        scene.holes.push(Hole { surface: child, rect: Rect::new(0, 0, 100, 100) });
        scene.step_snapshots(&surfaces, 2);
        assert!(scene.snapshots.is_empty());
    }
}
