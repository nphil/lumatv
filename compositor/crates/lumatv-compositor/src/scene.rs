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
use crate::transition::{Transitions, Visual};
use crate::window::{Layer, ShellState, Window, WindowId, WindowType};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// SAM's Home app: the Home stub's transparent card under LumaTV's compositor (`tools/lumatv homestub`), LG's own Home
/// otherwise. The card LumaTV's Home is drawn in place of.
pub const HOME_APP: &str = "com.webos.app.home";

/// How recent a commit of the shell's idle system layer must be to count as the frame its `live` announces: the word follows the frame by
/// what the client's request takes, 100 ms and more under load on the set (the layer's empty frame after its last content is older still).
const JOIN_FRESH_NS: u64 = 400_000_000;

/// LG's keyboard at 1080p: maliit's `keyboardDefaultHeight` (`876 / resolution`, resolution 2) and what a window that was sized to the
/// keyboard is on the set (1920x438 in the gate's good runs). The keyboard sits in the top rows of its window (`normalKeyboard` at y 0).
pub const KEYBOARD_HEIGHT: i32 = 438;

/// The part of a keyboard window that is the keyboard, in the window's own coordinates. maliit's window is not always the keyboard's size:
/// on the TV it came 1920x1080 (the keyboard drawn in the top rows, the rest transparent) in 5 of 10 runs and 1920x438 in the other 5, and
/// round 7's journal shows the 1080 window with no input region, so neither the region nor the size says where the keyboard is. In order:
/// the bounding box of the window's input region, clipped to the window; else the whole window when it is not taller than three quarters
/// of the output (a window sized to its keyboard); else the top [`KEYBOARD_HEIGHT`] rows of it (a window the size of the screen, which
/// no keyboard is).
pub fn keyboard_geometry(size: (i32, i32), input: Option<&[Rect]>) -> Rect {
    let whole = Rect::new(0, 0, size.0, size.1);
    let bounds = input.map(|rects| rects.iter().map(|r| r.intersect(&whole)).fold(Rect::default(), |a, r| a.union(&r))).unwrap_or_default();
    if !bounds.is_empty() {
        bounds
    } else if size.1 > OUTPUT_HEIGHT as i32 * 3 / 4 {
        Rect::new(0, 0, size.0, KEYBOARD_HEIGHT.min(size.1))
    } else {
        whole
    }
}

/// Where a keyboard window's origin goes so that its keyboard (`g`, from [`keyboard_geometry`]) is LG's `KeyboardView` panel: the full
/// width's centre and the bottom edge of the output, or the corner of the rectangle the app asked for (`preferred`).
pub fn keyboard_origin(g: Rect, preferred: Option<Rect>) -> (i32, i32) {
    match preferred {
        Some(r) => (r.x - g.x, r.y - g.y),
        None => ((OUTPUT_WIDTH as i32 - g.w) / 2 - g.x, OUTPUT_HEIGHT as i32 - g.h - g.y),
    }
}

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

impl Group {
    /// Whether `surface` is attached to this group, to a named layer or anonymously.
    pub fn has_member(&self, surface: SurfaceId) -> bool {
        self.layers.values().any(|l| l.member == Some(surface)) || self.anonymous.iter().any(|(s, _)| *s == surface)
    }

    /// The attached surfaces, each with where it stacks against the owner: a layer's z (negative below the owner), an anonymous
    /// surface's `z_hint` (below = 0, above = 1, top = 2) as a z beyond every layer's. Not sorted.
    pub fn member_stack(&self) -> impl Iterator<Item = (i32, SurfaceId)> + '_ {
        let layered = self.layers.values().filter_map(|l| l.member.map(|m| (l.z, m)));
        let anonymous = self.anonymous.iter().map(|(s, hint)| {
            (
                match hint {
                    0 => i32::MIN / 2,
                    2 => i32::MAX / 2,
                    _ => i32::MAX / 4,
                },
                *s,
            )
        });
        layered.chain(anonymous)
    }
}

/// A rectangle the renderer clears to transparent black: video shows through from the plane below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hole {
    pub surface: SurfaceId,
    pub rect: Rect,
}

/// A hole as the renderer draws it: where, and the index in the plan's items of the surface it belongs to. LG's compositor draws a
/// video's punch-through at z -1 inside its window (`views/punchThrough.js`): what is under the window is cleared and the window's own
/// pixels go on top of the clear, so the renderer clears a hole just before it draws that item. What is drawn above the item (Home's glass over the
/// moving wallpaper's video, an app's controls over its own video) is not touched by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlannedHole {
    pub rect: Rect,
    pub before: usize,
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
    pub holes: Vec<PlannedHole>,
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
    /// Where the shell's own keyboard is on the screen while it is: the system layer takes the pointer there and nowhere else (its window has no
    /// key focus to be the pointer's by: the field's app keeps that).
    pub shell_pointer: Option<Rect>,
    /// The next plan damages everything it covers whatever changed ([`Scene::redraw_all`]).
    redraw_all: bool,
    /// What the last plan drew and where, to damage what moved, appeared, restacked or vanished.
    last_placed: Vec<Placed>,
    last_holes: Vec<PlannedHole>,
    plan: Plan,
    /// Whether the last stack built drew LumaTV's Home in the card slot ([`Scene::shell_home_shown`]).
    home_shown: bool,
    /// The shell's system layer has nothing to show ([`Scene::set_shell_system_live`]): its window is left out of the
    /// stack, mapped and warm as it stays.
    system_idle: bool,
    /// The windows in transition (`transition.rs`): drawn through poses while their tracks run.
    pub transitions: Transitions,
    /// The clock of the last plan, CLOCK_MONOTONIC ns: what `take_visual` answers at.
    plan_ns: u64,
    /// The system layer came back live since the last plan: it joins without damage of its own (see `plan`).
    system_joining: bool,
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
    /// while it wants the keys or within its keyboard's rectangle (a transparent layer must not take the pointer from the app under it).
    pub fn surface_at(&self, x: i32, y: i32) -> Option<(SurfaceId, Rect)> {
        let keyboard = self.shell_pointer.is_some_and(|r| r.contains(x, y));
        self.plan
            .items
            .iter()
            .rev()
            .filter(|i| i.rect.contains(x, y))
            .find(|i| self.windows.get(&i.window).is_none_or(|w| w.kind != WindowType::ShellSystem || w.takes_keys || keyboard))
            .map(|i| (i.surface, i.rect))
    }

    /// Where `window`'s picture is now, taken out of its transition: a second press of Home hands the card that was on its way home to
    /// the switcher, which carries it on to its own slot (`Transitions::take_visual`, at the last plan's clock).
    pub fn take_visual(&mut self, window: WindowId) -> Option<Visual> {
        let now = self.plan_ns;
        self.transitions.take_visual(window, now)
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

    /// Whether the moving wallpaper's window is mapped and has a video rectangle: the hole that goes under LumaTV's Home (the caller adds
    /// that Home is drawn: the wallpaper's window is left out of the stack otherwise, and so is its hole). The shell takes its own
    /// wallpaper away only while this holds (without the hole the plane's black would show), and puts it back the frame it stops.
    pub fn wall_hole(&self) -> bool {
        self.holes.iter().any(|h| self.by_surface.get(&h.surface).and_then(|w| self.windows.get(w)).is_some_and(|w| w.kind == WindowType::Wallpaper && w.map_serial > 0))
    }

    /// Whether LumaTV's Home, drawn or not, is under a window that covers the whole output (an app's card, LG's Settings, the gate's
    /// X1 probe as an overlay): the client stops drawing it then, as nothing of it can be seen. Builds the stack.
    pub fn shell_home_covered(&mut self, surfaces: &Surfaces) -> bool {
        self.build_stack(surfaces);
        let Some(home) = self.stack.iter().position(|(layer, _, _)| *layer == Layer::Home) else { return true };
        self.stack[home + 1..].iter().any(|(_, _, id)| {
            self.windows.get(id).is_some_and(|w| {
                !w.kind.is_shell() && surfaces.get(w.surface).map(|s| s.size()).is_some_and(|(sw, sh)| sw >= OUTPUT_WIDTH as i32 && sh >= OUTPUT_HEIGHT as i32)
            })
        })
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

    /// The shell says its system layer has something to show, or nothing any more. The window keeps its surface, its
    /// buffers and their imports either way (a toast's window made anew paid a window, three CMA buffers and three warms
    /// before its first frame: 44-52 ms on the tenth gate against 50 allowed), so the next thing it shows costs one frame.
    /// A layer with nothing on it is transparent: left out of the stack an app's buffer goes to the plane whole, and
    /// brought back it joins quietly, its next commit the frame that shows it (a frame for the join alone would take the
    /// refresh that commit is meant for). Returns whether a frame is wanted now: the layer's frame came in the last
    /// [`JOIN_FRESH_NS`] while it was idle, ahead of the word on the channel, so the join is what shows it. A frame older
    /// than that is what the layer left when it went idle (Qt draws the layer once more, empty, after a toast's end).
    pub fn set_shell_system_live(&mut self, live: bool, now_ns: u64) -> bool {
        let joining = live && self.system_idle;
        self.system_idle = !live;
        if !joining {
            return false;
        }
        let fresh = self
            .windows
            .values()
            .any(|w| w.kind == WindowType::ShellSystem && w.commits > w.commits_drawn && now_ns.saturating_sub(w.last_commit_ns) < JOIN_FRESH_NS);
        self.system_joining = !fresh;
        fresh
    }

    /// How long ago the system layer's last commit arrived, in ms (`None` when it has no window).
    pub fn shell_system_commit_age_ms(&self, now_ns: u64) -> Option<u64> {
        self.windows.values().find(|w| w.kind == WindowType::ShellSystem).map(|w| now_ns.saturating_sub(w.last_commit_ns) / 1_000_000)
    }

    pub fn shell_system_live(&self) -> bool {
        !self.system_idle
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
                // A group member never comes to the front: it is no card (`Window::in_group`).
                if !w.in_group {
                    w.front = self.map_counter;
                }
            } else {
                w.map_serial = 0;
                w.front = 0;
            }
        }
    }

    /// Whether `w` is at the front of the cards: mapped, or raised by a fullscreen request while it waits for its
    /// first buffer; a window whose surface lost its buffer behind the bookkeeping's back is not.
    fn in_front(w: &Window, surfaces: &Surfaces) -> bool {
        w.kind.is_card() && !w.in_group && w.front > 0 && (w.map_serial == 0 || surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
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
            && !w.in_group
        {
            self.map_counter += 1;
            w.front = self.map_counter;
            if w.map_serial > 0 {
                w.map_serial = self.map_counter;
            }
        }
    }

    /// Makes the window of `surface`, if it has one, a member of a surface group or not (`wl_webos_surface_group.attach` and `detach`).
    /// LG's compositor takes no window attached to a group for a fullscreen card (`StarfishFullscreenController.__allowedFullscreen`):
    /// one that was at the front leaves it (the card it covered comes back), and none is at the front again until it is detached and asks
    /// for fullscreen. Returns whether a card was at the front and is not now: the cards are to be settled again then.
    pub fn set_group_member(&mut self, surface: SurfaceId, member: bool) -> bool {
        let Some(w) = self.by_surface.get(&surface).copied().and_then(|id| self.windows.get_mut(&id)) else { return false };
        let left_the_front = w.kind.is_card() && w.front > 0;
        w.in_group = member;
        w.front = 0;
        left_the_front
    }

    /// The group `surface` is attached to as a member, if it is attached to one.
    pub fn group_of_member(&self, surface: SurfaceId) -> Option<&Group> {
        self.groups.iter().find(|g| g.owner != surface && g.has_member(surface))
    }

    /// The members of the groups `owner` owns, from the lowest (negative z, below the owner) to the highest.
    pub fn group_members(&self, owner: SurfaceId) -> Vec<(i32, SurfaceId)> {
        let mut members: Vec<(i32, SurfaceId)> = self.groups.iter().filter(|g| g.owner == owner).flat_map(Group::member_stack).collect();
        members.sort_unstable();
        members
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
        let top = self.stack.iter().rev().map(|(_, _, id)| *id).find(|id| self.windows.get(id).is_some_and(|w| w.needs_focus()))?;
        Some(self.group_focus(top, surfaces).unwrap_or(top))
    }

    /// LG's group model (`StarfishGroupModel.updateFocus`): of the members of the group a window owns, the topmost mapped one takes the
    /// keys when it stacks above the owner (z of 0 or more); a member below the owner, or none, leaves them with the owner. A member
    /// that is a bare surface or says `needFocus` false is not one the keys can go to. LG's key ordering (`commit_key_index`) is not
    /// here: the layers' key indexes are kept, not read.
    fn group_focus(&self, owner: WindowId, surfaces: &Surfaces) -> Option<WindowId> {
        let owner_surface = self.windows.get(&owner)?.surface;
        let (z, member) = self
            .groups
            .iter()
            .filter(|g| g.owner == owner_surface)
            .flat_map(Group::member_stack)
            .filter(|(_, m)| surfaces.get(*m).is_some_and(|s| s.is_mapped()))
            .max()?;
        if z < 0 {
            return None;
        }
        let w = self.windows.get(self.by_surface.get(&member)?)?;
        w.needs_focus().then_some(w.id)
    }

    /// Mapped windows in draw order, bottom to top, into `self.stack`. LumaTV's Home (the shell's `ShellHome` window) takes the
    /// card slot while the card at the front is SAM's Home, the stub that draws nothing, or while there is no card at all; under
    /// an app's card it is left out, as the stub is while Home is drawn in its place. The moving wallpaper's window (`Wallpaper`, the
    /// lowest layer) goes with it: it is drawn, with its hole, only under Home that is drawn, so an app that takes the front gets the
    /// plane's picture back the same frame, whatever the player is still doing.
    fn build_stack(&mut self, surfaces: &Surfaces) {
        self.stack.clear();
        for w in self.windows.values() {
            if w.map_serial == 0 || w.state == ShellState::Minimized || !surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false) {
                continue;
            }
            // A group member is drawn with its owner (`plan`), and a card that is not in front is none of the stack's: a window that left
            // its group while mapped waits for its own `set_state(fullscreen)`.
            if w.in_group || (w.kind.is_card() && w.front == 0) {
                continue;
            }
            if w.kind == WindowType::ShellSystem && self.system_idle {
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
                Layer::Home | Layer::Wallpaper => has_shell_home && card_is_home,
                _ => true,
            };
            i += 1;
            keep_it
        });
    }

    /// Members of the groups `owner` owns, sorted by z, into `self.members` (negative z below the owner).
    fn build_members(&mut self, owner: SurfaceId) {
        self.members.clear();
        self.members.extend(self.groups.iter().filter(|g| g.owner == owner).flat_map(Group::member_stack));
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
                let g = surfaces.get(w.surface).map(|s| keyboard_geometry(s.size(), s.current.input.as_deref())).unwrap_or_default();
                keyboard_origin(g, self.keyboard_pref)
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
        for (_, _, wid) in &self.stack {
            if let Some(w) = self.windows.get_mut(wid) {
                w.commits_drawn = w.commits;
            }
        }
        for h in &self.holes {
            if let Some(before) = self.plan.items.iter().position(|i| i.surface == h.surface) {
                let r = h.rect.translated(self.plan.items[before].rect.x, self.plan.items[before].rect.y).intersect(&output);
                if !r.is_empty() {
                    self.plan.holes.push(PlannedHole { rect: r, before });
                }
            }
        }
        // Whatever moved, appeared, restacked or vanished is drawn again in full, old place and new. A request that
        // changes nothing on the screen (a window made, named or given a state before its first buffer) draws no
        // frame: on every gate the full redraw of the same picture at X1's window creation flipped 14-15 ms before
        // X1's first commit, and that frame's flip two refreshes later was logged as a skipped refresh (33.37 ms).
        self.placed.clear();
        self.placed.extend(self.plan.items.iter().map(|i| Placed { surface: i.surface, rect: i.rect, opaque: i.opaque, y_invert: i.y_invert }));
        // The system layer back in the stack counts as having been there: no damage for the join itself.
        if std::mem::take(&mut self.system_joining) {
            for (i, p) in self.placed.iter().enumerate() {
                let of_layer = self.windows.get(&self.plan.items[i].window).is_some_and(|w| w.kind == WindowType::ShellSystem);
                if of_layer && !self.last_placed.contains(p) {
                    let at = i.min(self.last_placed.len());
                    self.last_placed.insert(at, *p);
                }
            }
        }
        if std::mem::take(&mut self.redraw_all) || self.placed != self.last_placed || self.plan.holes != self.last_holes {
            let tiles = self.plan.tiles.as_mut().unwrap();
            for r in self.last_placed.iter().chain(self.placed.iter()).map(|p| &p.rect).chain(self.last_holes.iter().map(|h| &h.rect)).chain(self.plan.holes.iter().map(|h| &h.rect)) {
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
            // LumaTV's Home fills its window with an opaque wallpaper, whatever its buffer's format says: alone on the screen it goes to
            // the plane whole (direct scan-out), and its idle tick costs the compositor a flip, not a composition.
            let shell_home = self.windows.get(&window).is_some_and(|w| w.kind == WindowType::ShellHome);
            let opaque = shell_home
                || s.current.opaque.as_ref().map(|o| o.iter().any(|r| r.x <= 0 && r.y <= 0 && r.right() >= rect.w && r.bottom() >= rect.h)).unwrap_or(false)
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
