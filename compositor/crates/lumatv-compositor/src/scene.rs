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

/// SAM's Home app: the Home stub's transparent card under LumaTV's compositor (`tools/lumatv homestub`), LG's own Home
/// otherwise. The card LumaTV's Home is drawn in place of.
pub const HOME_APP: &str = "com.webos.app.home";

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
    /// Where the pointer's sprite goes, above everything, while the cursor shows.
    pub cursor: Option<Rect>,
    /// When the oldest commit this frame shows for the first time arrived (CLOCK_MONOTONIC ns); `None` when it
    /// shows no new commit (a layout request, the system UI), which is judged as due since the last flip.
    pub content_ns: Option<u64>,
}

impl Plan {
    /// The C3 direct scan-out hook: the single opaque item that covers the output with nothing above or below
    /// it, if there is one.
    pub fn sole_fullscreen(&self) -> Option<&Item> {
        let full = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        match self.items.as_slice() {
            [one] if one.opaque && one.rect == full && self.holes.is_empty() && self.cursor.is_none() => Some(one),
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
        self.cursor = None;
        self.content_ns = None;
        if let Some(t) = &mut self.tiles {
            t.clear();
        }
    }
}

/// What one drawn item looked like to the eye, apart from its content: the plans of two frames with the same list of
/// these, in the same order, and the same holes show the same picture wherever no surface damage says otherwise.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Placed {
    surface: SurfaceId,
    rect: Rect,
    opaque: bool,
    y_invert: bool,
}

#[derive(Default)]
pub struct Scene {
    windows: BTreeMap<WindowId, Window>,
    by_surface: BTreeMap<SurfaceId, WindowId>,
    next_window: u32,
    map_counter: u64,
    pub groups: Vec<Group>,
    pub holes: Vec<Hole>,
    /// The pointer's sprite rectangle while the cursor shows (set by the state before each plan).
    pub cursor: Option<Rect>,
    last_cursor: Option<Rect>,
    /// Where the field that holds the text-input activation wants the keyboard (`text_model.set_input_panel_rect`), in output pixels.
    pub keyboard_pref: Option<Rect>,
    /// The next plan damages everything it covers whatever changed ([`Scene::redraw_all`]).
    redraw_all: bool,
    /// What the last plan drew and where, to damage what moved, appeared, restacked or vanished.
    last_placed: Vec<Placed>,
    last_holes: Vec<Rect>,
    plan: Plan,
    /// Whether the last stack built drew LumaTV's Home in the card slot ([`Scene::shell_home_shown`]).
    home_shown: bool,
    // Scratch space for planning, reused across frames.
    stack: Vec<(Layer, u64, WindowId)>,
    members: Vec<(i32, SurfaceId)>,
    tree: Vec<(SurfaceId, i32, i32)>,
    placed: Vec<Placed>,
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
        id
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.get_mut(&id)
    }

    pub fn window_of(&self, surface: SurfaceId) -> Option<WindowId> {
        self.by_surface.get(&surface).copied()
    }

    /// The topmost drawn surface under an output point and the rectangle it is drawn at (the last plan), for
    /// the pointer. Holes are the video's, not a window's, so they do not count; the shell's system layer counts only
    /// while it wants the keys (a transparent layer must not take the pointer from the app under it).
    pub fn surface_at(&self, x: i32, y: i32) -> Option<(SurfaceId, Rect)> {
        self.plan
            .items
            .iter()
            .rev()
            .filter(|i| i.rect.contains(x, y))
            .find(|i| self.windows.get(&i.window).is_none_or(|w| w.kind != WindowType::ShellSystem || w.takes_keys))
            .map(|i| (i.surface, i.rect))
    }

    /// The shell client's window of `kind` (`ShellHome` or `ShellSystem`), if it has one.
    pub fn shell_window(&self, kind: WindowType) -> Option<&Window> {
        self.windows.values().find(|w| w.kind == kind)
    }

    /// Whether the last stack drew LumaTV's Home in the card slot: the shell's Home window is mapped and the card at the front is
    /// SAM's Home (the stub) or there is none.
    pub fn shell_home_shown(&self) -> bool {
        self.home_shown
    }

    /// The shell's system layer wants the keys, or stops wanting them; returns whether that changed (the focus then moves).
    pub fn set_shell_takes_keys(&mut self, takes: bool) -> bool {
        let mut changed = false;
        for w in self.windows.values_mut().filter(|w| w.kind == WindowType::ShellSystem) {
            changed |= w.takes_keys != takes;
            w.takes_keys = takes;
        }
        changed
    }

    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    pub fn remove_window(&mut self, id: WindowId) -> Option<Window> {
        let w = self.windows.remove(&id)?;
        self.by_surface.remove(&w.surface);
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
    }

    /// Records that a window's surface got its first buffer (or lost its last). A card that maps comes to the front
    /// (LG's fullscreen container takes every mapping as a fullscreen request); one that unmaps leaves it.
    pub fn set_mapped(&mut self, surface: SurfaceId, mapped: bool) {
        if let Some(id) = self.by_surface.get(&surface).copied()
            && let Some(w) = self.windows.get_mut(&id)
        {
            if mapped {
                self.map_counter += 1;
                w.map_serial = self.map_counter;
                w.front = self.map_counter;
            } else {
                w.map_serial = 0;
                w.front = 0;
            }
        }
    }

    /// Whether `w` is at the front of the cards: mapped, or raised by a fullscreen request while it waits for its
    /// first buffer; a window whose surface lost its buffer behind the bookkeeping's back is not.
    fn in_front(w: &Window, surfaces: &Surfaces) -> bool {
        w.kind.is_card() && w.front > 0 && (w.map_serial == 0 || surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
    }

    /// The foreground card: the card that came to the front last and is not minimized, whether or not it has a
    /// buffer yet (SAM is told, and tells a preloaded app it is foreground, which is when it draws).
    pub fn foreground(&self, surfaces: &Surfaces) -> Option<&Window> {
        self.windows.values().filter(|w| Self::in_front(w, surfaces) && w.state != ShellState::Minimized).max_by_key(|w| w.front)
    }

    /// Brings the card `id` to the front: what LG's fullscreen container does for a card that asks for fullscreen,
    /// mapped (a relaunch over another card) or not yet (a preloaded app relaunched). Nothing for a window that is
    /// not a card.
    pub fn raise(&mut self, id: WindowId) {
        if let Some(w) = self.windows.get_mut(&id)
            && w.kind.is_card()
        {
            self.map_counter += 1;
            w.front = self.map_counter;
            if w.map_serial > 0 {
                w.map_serial = self.map_counter;
            }
        }
    }

    /// The card at the front whatever its state, other than `except`: the one that comes back when the foreground
    /// card goes or minimizes itself.
    pub fn front_card(&self, surfaces: &Surfaces, except: Option<WindowId>) -> Option<WindowId> {
        self.windows.values().filter(|w| Self::in_front(w, surfaces) && Some(w.id) != except).max_by_key(|w| w.front).map(|w| w.id)
    }

    /// The window that should hold keyboard focus: the topmost mapped window that takes it (the on-screen keyboard, over the app
    /// whose field it types into, does not).
    pub fn focus(&mut self, surfaces: &Surfaces) -> Option<WindowId> {
        self.build_stack(surfaces);
        self.stack.iter().rev().map(|(_, _, id)| *id).find(|id| self.windows.get(id).is_some_and(|w| w.needs_focus()))
    }

    /// Mapped windows in draw order, bottom to top, into `self.stack`. LumaTV's Home (the shell's `ShellHome` window) takes the
    /// card slot while the card at the front is SAM's Home, the stub that draws nothing, or while there is no card at all; under
    /// an app's card it is left out, as the stub is while Home is drawn in its place.
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
        let card_is_home = last_card.map(|i| self.windows.get(&self.stack[i].2).is_some_and(|w| w.app_id() == HOME_APP)).unwrap_or(true);
        let has_shell_home = self.stack.iter().any(|(layer, _, _)| *layer == Layer::Home);
        self.home_shown = has_shell_home && card_is_home;
        let drop_card = self.home_shown;
        let mut i = 0;
        self.stack.retain(|(layer, _, _)| {
            let keep_it = match layer {
                Layer::Cards => last_card == Some(i) && !drop_card,
                Layer::Home => has_shell_home && card_is_home,
                _ => true,
            };
            i += 1;
            keep_it
        });
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
            let origin = if w.kind.is_card() {
                (0, 0)
            } else if w.kind == WindowType::Keyboard {
                // LG's KeyboardView: anchored at the bottom of the output, as high as the window, the window centred in that width; the
                // rectangle the app asked for when it asked for one.
                let (sw, sh) = surfaces.get(w.surface).map(|s| s.size()).unwrap_or((0, 0));
                match self.keyboard_pref {
                    Some(r) => (r.x, r.y),
                    None => ((OUTPUT_WIDTH as i32 - sw) / 2, OUTPUT_HEIGHT as i32 - sh),
                }
            } else {
                w.position
            };
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
        // Whatever moved, appeared, restacked or vanished is drawn again in full, old place and new. A request that
        // changes nothing on the screen (a window made, named or given a state before its first buffer) draws no
        // frame: on every gate the full redraw of the same picture at X1's window creation flipped 14-15 ms before
        // X1's first commit, and that frame's flip two refreshes later was logged as a skipped refresh (33.37 ms).
        self.placed.clear();
        self.placed.extend(self.plan.items.iter().map(|i| Placed { surface: i.surface, rect: i.rect, opaque: i.opaque, y_invert: i.y_invert }));
        if std::mem::take(&mut self.redraw_all) || self.placed != self.last_placed || self.plan.holes != self.last_holes {
            let tiles = self.plan.tiles.as_mut().unwrap();
            for r in self.last_placed.iter().chain(self.placed.iter()).map(|p| &p.rect).chain(self.last_holes.iter()).chain(self.plan.holes.iter()) {
                self.plan.damage = self.plan.damage.union(r);
                tiles.add(r);
            }
            for i in &mut self.plan.items {
                i.damage = None;
            }
        }
        std::mem::swap(&mut self.last_placed, &mut self.placed);
        self.last_holes.clear();
        self.last_holes.extend_from_slice(&self.plan.holes);
        // The sprite sits above everything: where it was and where it is are redrawn when it moved or hid.
        self.plan.cursor = self.cursor;
        if self.cursor != self.last_cursor {
            let tiles = self.plan.tiles.as_mut().unwrap();
            for r in self.last_cursor.iter().chain(self.cursor.iter()) {
                let r = r.intersect(&output);
                if !r.is_empty() {
                    self.plan.damage = self.plan.damage.union(&r);
                    tiles.add(&r);
                }
            }
            self.last_cursor = self.cursor;
        }
        self.plan.damage = self.plan.damage.intersect(&output);
        &self.plan
    }

    /// The last plan, as built by [`Scene::plan`].
    pub fn last_plan(&self) -> &Plan {
        &self.plan
    }

    /// Damages everything the next plan covers, whatever changed.
    pub fn redraw_all(&mut self) {
        self.redraw_all = true;
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
            if let Some(c) = s.current.content_ns {
                self.plan.content_ns = Some(self.plan.content_ns.map_or(c, |p| p.min(c)));
            }
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
