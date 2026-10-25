//! The scene: which windows show, in what order, where, and what the renderer must draw this frame.
//!
//! Stacking follows LG's views (`docs/research/compositor-c2-result.md` §6): one card at a time (the newest
//! mapped one is the foreground app), then overlays, system UI, popups, the keyboard and the cover. Surface
//! groups stack their members around the owner. Punch-through regions become holes cleared to transparent so the
//! display engine's video window shows through.
//!
//! [`Scene::plan`] writes into storage the scene keeps, so a frame allocates nothing once the scene has been
//! planned at its size (`alloc_tests`); the plan borrows the scene for the frame.

use std::collections::BTreeMap;

use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::damage::Tiles;
use crate::surface::{Role, SurfaceId, Surfaces};
use crate::window::{Layer, ShellState, Window, WindowId, WindowType};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// A named `wl_webos_surface_group`, owned by one surface.
pub struct Group {
    pub name: String,
    pub owner: SurfaceId,
    pub layers: BTreeMap<String, GroupLayer>,
    pub allow_anonymous: bool,
    /// Members attached without a layer: (surface, z_hint).
    pub anonymous: Vec<(SurfaceId, u32)>,
}

pub struct GroupLayer {
    pub z: i32,
    pub key_index: i32,
    pub member: Option<SurfaceId>,
}

/// A rectangle the renderer clears to transparent black: video shows through from the plane below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hole {
    pub surface: SurfaceId,
    pub rect: Rect,
}

/// One surface to draw.
pub struct Item {
    pub window: WindowId,
    pub surface: SurfaceId,
    pub buffer: WlBuffer,
    pub rect: Rect,
    /// This item's damage rectangles are `plan.damage_rects[damage.0..damage.1]`, in output pixels; `None` for
    /// the whole rect.
    pub damage: Option<(usize, usize)>,
    pub opaque: bool,
    pub y_invert: bool,
}

/// Everything the renderer needs for one frame, bottom to top. Lives in the [`Scene`]; `plan` refreshes it.
#[derive(Default)]
pub struct Plan {
    pub items: Vec<Item>,
    pub holes: Vec<Rect>,
    pub damage_rects: Vec<Rect>,
    /// Bounds of everything that changed, in output pixels; empty when nothing did.
    pub damage: Rect,
    /// The changed tiles (`damage::TILE` grid), for the scan-out rings.
    pub tiles: Option<Tiles>,
}

impl Plan {
    /// The C3 direct scan-out hook: the single opaque item that covers the output with nothing above or below
    /// it, if there is one.
    pub fn sole_fullscreen(&self) -> Option<&Item> {
        let full = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        match self.items.as_slice() {
            [one] if one.opaque && one.rect == full && self.holes.is_empty() => Some(one),
            _ => None,
        }
    }

    pub fn item_damage(&self, item: &Item) -> Option<&[Rect]> {
        item.damage.map(|(a, b)| &self.damage_rects[a..b])
    }

    fn clear(&mut self) {
        self.items.clear();
        self.holes.clear();
        self.damage_rects.clear();
        self.damage = Rect::default();
        if let Some(t) = &mut self.tiles {
            t.clear();
        }
    }
}

#[derive(Default)]
pub struct Scene {
    windows: BTreeMap<WindowId, Window>,
    by_surface: BTreeMap<SurfaceId, WindowId>,
    next_window: u32,
    map_counter: u64,
    pub groups: Vec<Group>,
    pub holes: Vec<Hole>,
    /// Set whenever something other than surface damage changed the picture (map, unmap, move, restack).
    pub layout_changed: bool,
    /// What the last plan covered, to damage what a vanished window exposed.
    last_rects: Vec<Rect>,
    plan: Plan,
    // Scratch space for planning, reused across frames.
    stack: Vec<(Layer, u64, WindowId)>,
    members: Vec<(i32, SurfaceId)>,
    tree: Vec<(SurfaceId, i32, i32)>,
    rects: Vec<Rect>,
}

impl Scene {
    pub fn create_window(&mut self, surface: SurfaceId) -> WindowId {
        if let Some(id) = self.by_surface.get(&surface) {
            return *id;
        }
        self.next_window += 1;
        let id = WindowId(self.next_window);
        self.windows.insert(id, Window::new(id, surface));
        self.by_surface.insert(surface, id);
        self.layout_changed = true;
        id
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.layout_changed = true;
        self.windows.get_mut(&id)
    }

    pub fn window_of(&self, surface: SurfaceId) -> Option<WindowId> {
        self.by_surface.get(&surface).copied()
    }

    /// The topmost drawn surface under an output point and the rectangle it is drawn at (the last plan), for
    /// the pointer. Holes are the video's, not a window's, so they do not count.
    pub fn surface_at(&self, x: i32, y: i32) -> Option<(SurfaceId, Rect)> {
        self.plan.items.iter().rev().find(|i| i.rect.contains(x, y)).map(|i| (i.surface, i.rect))
    }

    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    pub fn remove_window(&mut self, id: WindowId) -> Option<Window> {
        let w = self.windows.remove(&id)?;
        self.by_surface.remove(&w.surface);
        self.layout_changed = true;
        Some(w)
    }

    pub fn remove_surface(&mut self, surface: SurfaceId) {
        if let Some(id) = self.by_surface.get(&surface).copied() {
            self.remove_window(id);
        }
        self.groups.retain(|g| g.owner != surface);
        for g in &mut self.groups {
            for l in g.layers.values_mut() {
                if l.member == Some(surface) {
                    l.member = None;
                }
            }
            g.anonymous.retain(|(s, _)| *s != surface);
        }
        self.holes.retain(|h| h.surface != surface);
        self.layout_changed = true;
    }

    /// Records that a window's surface got its first buffer (or lost its last).
    pub fn set_mapped(&mut self, surface: SurfaceId, mapped: bool) {
        if let Some(id) = self.by_surface.get(&surface).copied()
            && let Some(w) = self.windows.get_mut(&id)
        {
            if mapped {
                self.map_counter += 1;
                w.map_serial = self.map_counter;
            } else {
                w.map_serial = 0;
            }
            self.layout_changed = true;
        }
    }

    /// The foreground card: the newest mapped, non-minimized card window.
    pub fn foreground(&self, surfaces: &Surfaces) -> Option<&Window> {
        self.windows
            .values()
            .filter(|w| w.kind.is_card() && w.state != ShellState::Minimized && w.map_serial > 0)
            .filter(|w| surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
            .max_by_key(|w| w.map_serial)
    }

    /// The window that should hold keyboard focus: the topmost mapped window that wants focus.
    pub fn focus(&mut self, surfaces: &Surfaces) -> Option<WindowId> {
        self.build_stack(surfaces);
        self.stack.last().map(|(_, _, id)| *id)
    }

    /// Mapped windows in draw order, bottom to top, into `self.stack`.
    fn build_stack(&mut self, surfaces: &Surfaces) {
        self.stack.clear();
        for w in self.windows.values() {
            if w.map_serial == 0 || w.state == ShellState::Minimized || !surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false) {
                continue;
            }
            self.stack.push((w.kind.layer(), w.map_serial, w.id));
        }
        self.stack.sort_unstable();
        // Only the newest card shows: drop every card but the last of the Cards run.
        let mut last_card = None;
        for (i, (layer, _, _)) in self.stack.iter().enumerate() {
            if *layer == Layer::Cards {
                last_card = Some(i);
            }
        }
        if let Some(keep) = last_card {
            let mut i = 0;
            self.stack.retain(|(layer, _, _)| {
                let keep_it = *layer != Layer::Cards || i == keep;
                i += 1;
                keep_it
            });
        }
    }

    /// Members of the groups `owner` owns, sorted by z, into `self.members` (negative z below the owner).
    fn build_members(&mut self, owner: SurfaceId) {
        self.members.clear();
        for g in self.groups.iter().filter(|g| g.owner == owner) {
            for l in g.layers.values() {
                if let Some(m) = l.member {
                    self.members.push((l.z, m));
                }
            }
            for (s, hint) in &g.anonymous {
                // z_hint: below = 0, above = 1, top = 2.
                self.members.push((
                    match hint {
                        0 => i32::MIN / 2,
                        2 => i32::MAX / 2,
                        _ => i32::MAX / 4,
                    },
                    *s,
                ));
            }
        }
        self.members.sort_unstable();
    }

    /// Builds this frame's plan.
    pub fn plan(&mut self, surfaces: &Surfaces) -> &Plan {
        let output = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        self.plan.clear();
        if self.plan.tiles.is_none() {
            self.plan.tiles = Some(Tiles::new(OUTPUT_WIDTH, OUTPUT_HEIGHT));
        }
        self.build_stack(surfaces);
        let n = self.stack.len();
        for i in 0..n {
            let wid = self.stack[i].2;
            let Some(w) = self.windows.get(&wid) else { continue };
            let origin = if w.kind.is_card() { (0, 0) } else { w.position };
            let owner = w.surface;
            self.build_members(owner);
            let m = self.members.len();
            for k in 0..m {
                let (z, s) = self.members[k];
                if z < 0 {
                    self.push_tree(surfaces, wid, s, origin);
                }
            }
            self.push_tree(surfaces, wid, owner, origin);
            for k in 0..m {
                let (z, s) = self.members[k];
                if z >= 0 {
                    self.push_tree(surfaces, wid, s, origin);
                }
            }
        }
        for h in &self.holes {
            if let Some(item) = self.plan.items.iter().find(|i| i.surface == h.surface) {
                let r = h.rect.translated(item.rect.x, item.rect.y).intersect(&output);
                if !r.is_empty() {
                    self.plan.holes.push(r);
                }
            }
        }
        self.rects.clear();
        self.rects.extend(self.plan.items.iter().map(|i| i.rect));
        if self.layout_changed || self.rects != self.last_rects {
            let tiles = self.plan.tiles.as_mut().unwrap();
            for r in self.last_rects.iter().chain(self.rects.iter()) {
                self.plan.damage = self.plan.damage.union(r);
                tiles.add(r);
            }
            for h in &self.plan.holes {
                self.plan.damage = self.plan.damage.union(h);
                tiles.add(h);
            }
            for i in &mut self.plan.items {
                i.damage = None;
            }
            self.layout_changed = false;
        }
        std::mem::swap(&mut self.last_rects, &mut self.rects);
        self.plan.damage = self.plan.damage.intersect(&output);
        &self.plan
    }

    /// The last plan, as built by [`Scene::plan`].
    pub fn last_plan(&self) -> &Plan {
        &self.plan
    }

    fn push_tree(&mut self, surfaces: &Surfaces, window: WindowId, root: SurfaceId, origin: (i32, i32)) {
        surfaces.tree_in_draw_order_into(root, &mut self.tree);
        let n = self.tree.len();
        for k in 0..n {
            let (sid, dx, dy) = self.tree[k];
            let Some(s) = surfaces.get(sid) else { continue };
            let Some(buffer) = s.current.buffer.clone() else { continue };
            if matches!(s.role, Role::Exported | Role::Cursor) {
                continue;
            }
            let rect = s.rect_at(origin.0 + dx, origin.1 + dy);
            if rect.is_empty() {
                continue;
            }
            let tiles = self.plan.tiles.as_mut().unwrap();
            let damage = match &s.current.damage {
                None => {
                    self.plan.damage = self.plan.damage.union(&rect);
                    tiles.add(&rect);
                    None
                }
                Some(d) => {
                    let start = self.plan.damage_rects.len();
                    for r in d {
                        let r = r.translated(rect.x, rect.y).intersect(&rect);
                        if !r.is_empty() {
                            self.plan.damage = self.plan.damage.union(&r);
                            tiles.add(&r);
                            self.plan.damage_rects.push(r);
                        }
                    }
                    Some((start, self.plan.damage_rects.len()))
                }
            };
            let opaque =
                s.current.opaque.as_ref().map(|o| o.iter().any(|r| r.x <= 0 && r.y <= 0 && r.right() >= rect.w && r.bottom() >= rect.h)).unwrap_or(false)
                    || crate::buffer::kind_of(&buffer).is_opaque_format();
            let y_invert = s.current.y_invert;
            self.plan.items.push(Item { window, surface: sid, buffer, rect, damage, opaque, y_invert });
        }
    }
}

impl crate::buffer::BufferKindRef<'_> {
    /// Whether the pixel format has no alpha (XRGB/XBGR).
    pub fn is_opaque_format(&self) -> bool {
        match self {
            crate::buffer::BufferKindRef::Shm(s) => s.format == 1,
            crate::buffer::BufferKindRef::Dmabuf(d) => matches!(d.format, crate::buffer::fourcc::XRGB8888 | crate::buffer::fourcc::XBGR8888),
            crate::buffer::BufferKindRef::External => false,
        }
    }
}

#[allow(dead_code)]
fn _assert_types(_: WindowType) {}
