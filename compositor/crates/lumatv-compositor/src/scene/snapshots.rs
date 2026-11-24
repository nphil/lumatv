use super::*;
use crate::snapshot::{CAPACITY, Draw, FADE_NS, Picture, RESUME_CAP_NS, SnapshotId, fade_alpha};

/// What a window's picture is made of when it is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Capture {
    /// The app's own layer, drawn again at half size.
    Ui,
    /// The app's own layer around a hole: the video is on the plane under a small part of the window (a preview inside a page).
    Video,
    /// A card painted from the theme: the window is mostly video, or only a hole, so its own pixels say nothing of the app.
    Card,
}

impl Capture {
    /// What a window's picture is made of, from `pixels` of the output its holes take in the last plan and whether it owns a hole at all,
    /// shown or not. A hole over a small part of the window (a preview in a page) leaves the page as the app's picture. Over half the
    /// output the app's pixels are a transparent window, and a card says more. No hole in the plan is the page, unless the window owns one
    /// the plan did not show (hidden, off screen): video of unknown size. `hardware` is an app that plays on the plane and is a page when it
    /// browses (YouTube): its picture is flagged as one made around video, which the window having video by its id does not end.
    fn of(hardware: bool, owns_hole: bool, pixels: i64) -> Capture {
        match pixels {
            0 if owns_hole => Capture::Card,
            0 if hardware => Capture::Video,
            0 => Capture::Ui,
            n if n * 2 < i64::from(OUTPUT_WIDTH) * i64::from(OUTPUT_HEIGHT) => Capture::Video,
            _ => Capture::Card,
        }
    }
}

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

    /// Whether a surface of the window's own has a hole over the video plane, shown or not.
    fn window_has_hole(&self, window: WindowId, surfaces: &Surfaces) -> bool {
        let Some(w) = self.windows.get(&window) else { return false };
        self.holes.iter().any(|h| self.owns_surface(w.surface, h.surface, surfaces))
    }

    fn window_has_video(&self, window: WindowId, surfaces: &Surfaces) -> bool {
        self.windows.get(&window).is_some_and(|w| crate::switcher::input_app(w.app_id())) || self.window_has_hole(window, surfaces)
    }

    pub(crate) fn window_video(&self, window: WindowId, surfaces: &Surfaces) -> bool {
        if self.window_has_video(window, surfaces) { return true; }
        let Some(w) = self.windows.get(&window) else { return false };
        // An export reserves a future video element. Only a successful UI copy can override its conservative cold-launch fade: a picture made
        // of a window that had video proves nothing about the next one.
        self.snapshots.get(window).is_none_or(|p| p.video) && self.video.iter().any(|s| self.owns_surface(w.surface, *s, surfaces))
    }

    /// The output pixels the window's holes take in the last plan.
    fn hole_pixels(&self, window: WindowId) -> i64 {
        let output = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        self.plan.holes.iter()
            .filter(|h| self.plan.items.get(h.before).is_some_and(|i| i.window == window))
            .map(|h| h.rect.intersect(&output))
            .map(|r| i64::from(r.w) * i64::from(r.h))
            .sum()
    }

    /// What the picture of `window` is made of; `None` for a real input, whose screen is the source's video and no app's.
    fn capture_of(&self, window: WindowId, surfaces: &Surfaces) -> Option<Capture> {
        let id = self.windows.get(&window)?.app_id();
        let hardware = id.starts_with("youtube.leanback");
        if crate::switcher::input_app(id) && !hardware {
            return None;
        }
        Some(Capture::of(hardware, self.window_has_hole(window, surfaces), self.hole_pixels(window)))
    }

    /// Reserves a bounded slot before the last drawn app group is copied, never the output page, or before a card is painted for it.
    pub(crate) fn reserve_snapshot(&mut self, window: WindowId, surfaces: &Surfaces) -> Option<(SnapshotId, Capture)> {
        if !self.transitions.enabled() || self.transitions.suspended || self.last_front != Front::Card(window) {
            return None;
        }
        let w = self.windows.get(&window)?;
        if w.app_id().is_empty() || w.app_id() == HOME_APP || !matches!(w.kind, WindowType::Card | WindowType::Overlay) || w.in_group {
            return None;
        }
        let capture = self.capture_of(window, surfaces)?;
        // A picture the app never replaced is still the best one of it. A dissolving one is not: the live frame under it is newer.
        if self.snapshots.get(window).is_some_and(|p| p.root == w.surface && p.app == w.app_id() && p.resumable && p.fade.is_none()) {
            self.snapshots.park(window);
            return None;
        }
        self.snapshots.forget(window);
        if capture != Capture::Card {
            // An undrawn commit may have replaced or changed the source since the last plan; use the ordinary transition instead of guessing.
            let full = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
            let items = || self.plan.items.iter().filter(|i| i.window == window);
            if !items().any(|i| i.surface == w.surface && i.rect == full)
                || items().any(|i| i.xform || i.drift != (0, 0) || i.dim != 1.0 || !self.owns_surface(w.surface, i.surface, surfaces)
                    || !surfaces.get(i.surface).is_some_and(|s| !s.current.undrawn && s.current.buffer.as_ref() == Some(&i.buffer)))
                || self.plan.gain_on || self.plan.masks.iter().any(|m| m.window == window)
            {
                return None;
            }
        }
        if !self.snapshots.make_room(|w| self.transitions.track_of(w).is_some()) {
            return None;
        }
        let id = SnapshotId(self.snapshots.next());
        self.snapshots.pictures.push(Picture { id, window, root: w.surface, app: w.app_id().into(), layer: w.kind.layer(), order: w.map_serial,
            resumable: true, resume: None, fade: None, video: capture != Capture::Ui, card: capture == Capture::Card, used: id.0 });
        Some((id, capture))
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
                // Video arriving in a window whose picture holds none ends the picture; one made around video already has it.
                let video = !p.video && self.window_has_video(p.window, surfaces);
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
                if p.fade == Some(0) {
                    p.fade = Some(now_ns);
                }
            }
            let (front, suspended) = (self.front, self.transitions.suspended);
            self.snapshots.pictures.retain(|p| {
                (p.resumable || self.transitions.track_of(p.window).is_some())
                    && p.resume.is_none_or(|r| now_ns.saturating_sub(r.since_ns) < RESUME_CAP_NS)
                    // A dissolve ends with its time, or when the app is no longer what the screen shows.
                    && p.fade.is_none_or(|since| {
                        now_ns.saturating_sub(since) < FADE_NS && !suspended && (front == Front::Card(p.window) || self.transitions.track_of(p.window).is_some())
                    })
            });
        }
        self.redraw_all |= was != self.snapshots.len();
    }

    /// One trace-ring line (`L`, `ctl ring`) for a frame just drawn: what the front window is on screen as, so a reopen's first frames can be read
    /// back without a capture (a capture takes 1.5 s while an app opens). `pic` is the saved picture (`snap`, `card`) or its dissolve (`fade`, live
    /// window under it); `live` is the window's own items in the plan, `hole` its video holes, `trk` the open motion's opacity. `none` is a front
    /// app with nothing to draw: a black frame.
    pub(crate) fn note_layers(&self, frame: u64) {
        if !crate::ring::is_open() { return; }
        let Front::Card(window) = self.front else {
            crate::ring::note(format_args!("L {frame} {}", if self.front == Front::Home { "home" } else { "-" }));
            return;
        };
        let app = self.windows.get(&window).map_or("?", |w| w.app_id());
        let live = self.plan.items.iter().filter(|i| i.window == window).count();
        let hole = self.plan.holes.iter().filter(|h| self.plan.items.get(h.before).is_some_and(|i| i.window == window)).count();
        let draw = self.plan.snapshots.iter().find(|d| d.window == window);
        let layer = match draw {
            Some(d) if d.fading => "fade",
            Some(_) if self.snapshots.get(window).is_some_and(|p| p.card) => "card",
            Some(_) => "snap",
            None if live > 0 => "live",
            None => "none",
        };
        let pic = draw.map_or(0.0, |d| d.pose.alpha);
        match self.transitions.track_of(window) {
            Some(t) => crate::ring::note(format_args!("L {frame} {app} {layer} pic={pic:.2} live={live} hole={hole} trk={:.2}", t.pose_at(self.plan_ns).alpha)),
            None => crate::ring::note(format_args!("L {frame} {app} {layer} pic={pic:.2} live={live} hole={hole} trk=-")),
        }
    }

    pub(super) fn push_snapshot(&mut self, window: WindowId, now_ns: u64) {
        if self.transitions.suspended { return; }
        let Some(p) = self.snapshots.get(window) else { return };
        let track = self.transitions.track_of(window);
        if track.is_none() && p.resume.is_none() && p.fade.is_none() { return; }
        let full = FRect::new(0.0, 0.0, OUTPUT_WIDTH as f32, OUTPUT_HEIGHT as f32);
        let mut pose = track.map_or_else(|| Pose::rest(full), |t| t.pose_at(now_ns));
        if let Some(since) = p.fade {
            pose.alpha *= fade_alpha(since, now_ns);
        }
        self.plan.snapshots.push(Draw { id: p.id, window, root: p.root, before: self.plan.items.len(), pose, resume: p.resume, fading: p.fade.is_some() });
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
    /// Called before background/exposure notifications let the client change or release its picture. The app's last frame is kept for the
    /// task manager first (`switcher/list.rs`): an app that has left the front has no picture of its own after this.
    pub(crate) fn freeze_app(&mut self, window: WindowId) {
        self.keep_app_thumb(window);
        let Some((id, capture)) = self.scene.reserve_snapshot(window, &self.surfaces) else { return };
        let frozen = match (capture, self.scene.window(window)) {
            (Capture::Card, Some(w)) => {
                let pixels = crate::card::paint(&self.switcher.palette(), &w.title, w.app_id());
                self.output.freeze_card(id, &pixels, &self.scene.snapshots)
            }
            (Capture::Card, None) => false,
            (Capture::Ui | Capture::Video, _) => self.output.freeze_app(id, window, self.scene.last_plan(), &self.scene.snapshots),
        };
        if !frozen {
            self.scene.invalidate_snapshot(window);
        } else if capture == Capture::Ui {
            self.scene.transitions.confirm_ui_picture(window);
        }
        self.output.retain_app_snapshots(&self.scene.snapshots);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Receipt;

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
            layer: Layer::Cards, order: 1, resumable: true, resume: None, fade: None, video: false, card: false, used: id.0 });
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

    #[test]
    fn the_first_real_frame_dissolves_the_picture_and_the_time_or_a_change_of_front_ends_it() {
        let (mut scene, surfaces, window, _) = saved();
        scene.front = Front::Card(window);
        scene.resume_snapshot(window, &surfaces);
        scene.step_snapshots(&surfaces, 1_000);
        scene.push_snapshot(window, 1_000);
        let draw = scene.plan.snapshots[0];
        assert!(!draw.fading && draw.resume.is_some() && draw.pose.alpha == 1.0);
        let receipt = Receipt { id: draw.id, attempt: draw.resume.unwrap().attempt, content: surfaces.content_serial() + 1 };
        assert!(scene.snapshots.presented(receipt));

        scene.step_snapshots(&surfaces, 2_000);
        scene.plan.snapshots.clear();
        scene.push_snapshot(window, 2_000 + FADE_NS / 2);
        let draw = scene.plan.snapshots[0];
        assert!(draw.fading && draw.resume.is_none(), "the picture stays, dissolving, over the app's own frame");
        assert!((draw.pose.alpha - 0.5).abs() < 1e-6, "half way through: half of the picture");

        scene.step_snapshots(&surfaces, 2_000 + FADE_NS - 1);
        assert!(scene.snapshots.get(window).is_some());
        scene.step_snapshots(&surfaces, 2_000 + FADE_NS);
        assert!(scene.snapshots.is_empty(), "freed when the dissolve is done");

        let (mut scene, surfaces, window, _) = saved();
        scene.front = Front::Card(window);
        scene.snapshots.pictures[0].fade = Some(0);
        scene.step_snapshots(&surfaces, 5_000);
        assert!(scene.snapshots.fading());
        scene.front = Front::Home;
        scene.step_snapshots(&surfaces, 6_000);
        assert!(scene.snapshots.is_empty(), "the app left the screen under its own dissolve");
    }

    #[test]
    fn a_picture_the_app_never_replaced_is_parked_and_a_dissolving_one_is_recaptured() {
        let (mut scene, surfaces, window, _) = saved();
        scene.last_front = Front::Card(window);
        assert!(scene.reserve_snapshot(window, &surfaces).is_none());
        assert!(scene.snapshots.get(window).is_some_and(|p| p.resumable), "the old picture is still the best one");

        scene.snapshots.pictures[0].fade = Some(10);
        assert!(scene.reserve_snapshot(window, &surfaces).is_none(), "no plan to copy from here");
        assert!(scene.snapshots.get(window).is_none(), "the live frame under a dissolving picture is newer: it is not kept");
    }

    #[test]
    fn the_picture_of_a_window_with_video_follows_how_much_of_the_screen_the_video_takes() {
        let total = i64::from(OUTPUT_WIDTH) * i64::from(OUTPUT_HEIGHT);
        assert_eq!(Capture::of(false, false, 0), Capture::Ui);
        assert_eq!(Capture::of(false, true, 0), Capture::Card, "a hole the plan did not show is video of unknown size");
        assert_eq!(Capture::of(true, false, 0), Capture::Video, "the YouTube app browsing is a page");
        assert_eq!(Capture::of(false, true, total / 10), Capture::Video, "a preview in a page");
        assert_eq!(Capture::of(false, true, total / 2 - 1), Capture::Video);
        assert_eq!(Capture::of(false, true, total / 2), Capture::Card);
        assert_eq!(Capture::of(true, true, total), Capture::Card, "a player");
    }

    #[test]
    fn a_window_that_is_only_a_hole_gets_a_card_that_its_video_does_not_end() {
        let (mut scene, mut surfaces, window, root) = saved();
        scene.snapshots.clear();
        scene.last_front = Front::Card(window);
        scene.holes.push(Hole { surface: root, rect: Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32) });
        let (id, capture) = scene.reserve_snapshot(window, &surfaces).expect("a card needs no copy of the window");
        assert_eq!(capture, Capture::Card);
        let p = scene.snapshots.get(window).unwrap();
        assert!(p.id == id && p.video && p.resumable);
        scene.front = Front::Card(window);
        scene.step_snapshots(&surfaces, 1);
        assert!(scene.snapshots.get(window).is_some(), "the hole it was made around is not video arriving");
        assert!(scene.window_video(window, &surfaces));

        // a real input has no picture of its own to keep
        scene.snapshots.clear();
        let input = surfaces.create();
        let hdmi = scene.create_window(input);
        scene.windows.get_mut(&hdmi).unwrap().set_property("appId", "com.webos.app.hdmi1");
        scene.set_mapped(input, true);
        scene.last_front = Front::Card(hdmi);
        assert!(scene.reserve_snapshot(hdmi, &surfaces).is_none() && scene.snapshots.is_empty());
    }

    #[test]
    fn a_ui_picture_overrides_a_dormant_export_and_one_made_around_video_does_not() {
        let (mut scene, surfaces, window, root) = saved();
        scene.set_video_expected(root, true);
        assert!(!scene.window_video(window, &surfaces));
        scene.snapshots.pictures[0].video = true;
        assert!(scene.window_video(window, &surfaces));
    }
}
