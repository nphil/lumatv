//! The scene: which windows show, in what order, where, and what the renderer must draw this frame.
//!
//! Stacking follows LG's views (`docs/research/compositor-c2-result.md` §6): one card at a time (the newest
//! mapped one is the foreground app), then overlays, system UI, popups, the keyboard and the cover. Surface
//! groups stack their members around the owner. Punch-through regions become holes cleared to transparent so the
//! display engine's video window shows through.

use std::collections::BTreeMap;

use wayland_server::protocol::wl_buffer::WlBuffer;

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
    /// Damage in output pixels, `None` for the whole rect.
    pub damage: Option<Vec<Rect>>,
    pub opaque: bool,
    pub y_invert: bool,
}

/// Everything the renderer needs for one frame, bottom to top.
pub struct Plan {
    pub items: Vec<Item>,
    pub holes: Vec<Rect>,
    /// Union of the damage of every item plus anything that moved or vanished, in output pixels.
    pub damage: Rect,
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
    pub fn focus(&self, surfaces: &Surfaces) -> Option<WindowId> {
        self.stack(surfaces).last().map(|w| w.id)
    }

    /// Mapped windows in draw order, bottom to top.
    fn stack<'a>(&'a self, surfaces: &Surfaces) -> Vec<&'a Window> {
        let mut out = Vec::new();
        for layer in Layer::ALL {
            let mut ws: Vec<&Window> = self
                .windows
                .values()
                .filter(|w| w.kind.layer() == layer && w.map_serial > 0 && w.state != ShellState::Minimized)
                .filter(|w| surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
                .collect();
            ws.sort_by_key(|w| w.map_serial);
            if layer == Layer::Cards {
                if let Some(fg) = ws.pop() {
                    out.push(fg);
                }
            } else {
                out.extend(ws);
            }
        }
        out
    }

    /// Members of the groups `owner` owns, (surface, below_owner), sorted by z.
    fn group_members(&self, owner: SurfaceId) -> Vec<(SurfaceId, bool)> {
        let mut v: Vec<(i32, SurfaceId)> = Vec::new();
        for g in self.groups.iter().filter(|g| g.owner == owner) {
            for l in g.layers.values() {
                if let Some(m) = l.member {
                    v.push((l.z, m));
                }
            }
            for (s, hint) in &g.anonymous {
                // z_hint: below = 0, above = 1, top = 2.
                v.push((
                    match hint {
                        0 => i32::MIN / 2,
                        2 => i32::MAX / 2,
                        _ => i32::MAX / 4,
                    },
                    *s,
                ));
            }
        }
        v.sort_by_key(|(z, _)| *z);
        v.into_iter().map(|(z, s)| (s, z < 0)).collect()
    }

    /// Builds this frame's plan.
    pub fn plan(&mut self, surfaces: &Surfaces) -> Plan {
        let output = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        let mut items = Vec::new();
        let mut damage = Rect::default();
        for w in self.stack(surfaces) {
            let origin = if w.kind.is_card() { (0, 0) } else { w.position };
            let members = self.group_members(w.surface);
            for (s, _) in members.iter().filter(|(_, below)| *below) {
                push_tree(surfaces, w.id, *s, origin, &mut items, &mut damage);
            }
            push_tree(surfaces, w.id, w.surface, origin, &mut items, &mut damage);
            for (s, _) in members.iter().filter(|(_, below)| !*below) {
                push_tree(surfaces, w.id, *s, origin, &mut items, &mut damage);
            }
        }
        let holes: Vec<Rect> = self
            .holes
            .iter()
            .filter_map(|h| {
                let item = items.iter().find(|i| i.surface == h.surface)?;
                Some(h.rect.translated(item.rect.x, item.rect.y).intersect(&output))
            })
            .filter(|r| !r.is_empty())
            .collect();
        let rects: Vec<Rect> = items.iter().map(|i| i.rect).collect();
        if self.layout_changed || rects != self.last_rects {
            for r in self.last_rects.iter().chain(rects.iter()) {
                damage = damage.union(r);
            }
            for i in &mut items {
                i.damage = None;
            }
            self.layout_changed = false;
        }
        self.last_rects = rects;
        Plan { items, holes, damage: damage.intersect(&output) }
    }
}

fn push_tree(surfaces: &Surfaces, window: WindowId, root: SurfaceId, origin: (i32, i32), items: &mut Vec<Item>, damage: &mut Rect) {
    for (sid, dx, dy) in surfaces.tree_in_draw_order(root) {
        let Some(s) = surfaces.get(sid) else { continue };
        let Some(buffer) = s.current.buffer.clone() else { continue };
        if matches!(s.role, Role::Exported | Role::Cursor) {
            continue;
        }
        let rect = s.rect_at(origin.0 + dx, origin.1 + dy);
        if rect.is_empty() {
            continue;
        }
        let item_damage =
            s.current.damage.as_ref().map(|d| d.iter().map(|r| r.translated(rect.x, rect.y).intersect(&rect)).filter(|r| !r.is_empty()).collect::<Vec<_>>());
        match &item_damage {
            None => *damage = damage.union(&rect),
            Some(d) => {
                for r in d {
                    *damage = damage.union(r);
                }
            }
        }
        let opaque = s.current.opaque.as_ref().map(|o| o.iter().any(|r| r.x <= 0 && r.y <= 0 && r.right() >= rect.w && r.bottom() >= rect.h)).unwrap_or(false)
            || crate::buffer::kind_of(&buffer).is_opaque_format();
        let y_invert = s.current.y_invert;
        items.push(Item { window, surface: sid, buffer, rect, damage: item_damage, opaque, y_invert });
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
