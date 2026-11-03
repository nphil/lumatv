//! `wl_surface` state: the double-buffered contents a client commits, its role and its subsurface tree.
//!
//! Protocol objects stay in the `server` layer; this module keeps plain data keyed by [`SurfaceId`], which is
//! what the scene and the renderers read. Subsurfaces follow `wl_subcompositor`: a synchronized child's commit
//! is cached until its parent commits; a desynchronized one applies at once.

use std::collections::BTreeMap;

use wayland_server::Resource;
use wayland_server::protocol::wl_buffer::WlBuffer;
use wayland_server::protocol::wl_callback::WlCallback;

use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u32);

/// Painted subtitle bounds in the client-specified canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubtitleBounds {
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub rect: Rect,
}

/// The role a surface took; a surface has at most one for its life (`wl_surface` rules).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    None,
    /// `wl_shell_surface` and/or `wl_webos_shell_surface`: a window of its own (`window::WindowId`).
    Toplevel,
    Subsurface {
        parent: SurfaceId,
    },
    /// Attached to a surface group layer (`wl_webos_surface_group.attach`).
    GroupMember {
        owner: SurfaceId,
    },
    /// An exported element's video region (`wl_webos_foreign`), drawn as a hole.
    Exported,
    InputPanel,
    Cursor,
}

/// What a commit carries. `buffer` is `None` for "unchanged", `Some(None)` for "detached".
#[derive(Default)]
pub struct Pending {
    pub buffer: Option<Option<WlBuffer>>,
    pub attach_offset: (i32, i32),
    pub damage: Vec<Rect>,
    pub buffer_damage: Vec<Rect>,
    pub opaque: Option<Option<Vec<Rect>>>,
    pub input: Option<Option<Vec<Rect>>>,
    pub subtitle_bounds: Option<SubtitleBounds>,
    pub frame_callbacks: Vec<WlCallback>,
    pub scale: Option<i32>,
    pub transform: Option<u32>,
    /// `wl_subsurface.set_position`, applied on the parent's commit.
    pub sub_position: Option<(i32, i32)>,
}

impl Pending {
    fn take(&mut self) -> Pending {
        std::mem::take(self)
    }

    fn merge_into(self, into: &mut Pending) {
        if let Some(b) = self.buffer {
            into.buffer = Some(b);
            into.attach_offset = self.attach_offset;
        }
        into.damage.extend(self.damage);
        into.buffer_damage.extend(self.buffer_damage);
        if self.opaque.is_some() {
            into.opaque = self.opaque;
        }
        if self.input.is_some() {
            into.input = self.input;
        }
        if self.subtitle_bounds.is_some() {
            into.subtitle_bounds = self.subtitle_bounds;
        }
        into.frame_callbacks.extend(self.frame_callbacks);
        if self.scale.is_some() {
            into.scale = self.scale;
        }
        if self.transform.is_some() {
            into.transform = self.transform;
        }
        if self.sub_position.is_some() {
            into.sub_position = self.sub_position;
        }
    }
}

/// The committed state the renderers draw.
#[derive(Default)]
pub struct Current {
    pub buffer: Option<WlBuffer>,
    /// Size in surface pixels once known (shm and dma-buf at attach; driver buffers at import).
    pub size: Option<(i32, i32)>,
    /// Whether the buffer's rows run top-down (shm, LG's Mali buffers) rather than GL's bottom-up.
    pub y_invert: bool,
    pub opaque: Option<Vec<Rect>>,
    pub input: Option<Vec<Rect>>,
    /// None means unknown; a zero-sized rectangle means known empty.
    pub subtitle_bounds: Option<SubtitleBounds>,
    pub scale: i32,
    pub transform: u32,
    /// Damage since the last presented frame, in surface pixels; `None` means everything.
    pub damage: Option<Vec<Rect>>,
    /// Frame callbacks owed after the next presented frame.
    pub frame_callbacks: Vec<WlCallback>,
    /// Buffers no longer current, released after the frame that last read them is presented.
    pub retired: Vec<WlBuffer>,
    /// How many commits carried a buffer: a surface is mapped once this is > 0 with a buffer attached.
    pub buffer_commits: u64,
    /// A commit with new content has not been drawn yet.
    pub undrawn: bool,
    /// When the oldest such commit arrived (CLOCK_MONOTONIC ns): the earliest a frame showing it could have started,
    /// which is what tells a refresh the compositor missed from one that had nothing new to show (`Plan::content_ns`).
    pub content_ns: Option<u64>,
}

pub struct Surface {
    pub id: SurfaceId,
    pub role: Role,
    pub pending: Pending,
    /// A synchronized subsurface's commits accumulate here until the parent commits.
    pub cached: Option<Pending>,
    pub current: Current,
    pub subtitle_region_live: bool,
    pub children: Vec<SurfaceId>,
    /// Subsurface placement relative to the parent.
    pub position: (i32, i32),
    pub sync: bool,
    /// Which stacking slot in the parent's `children` this surface occupies is the vector order; `z_above_parent`
    /// says whether it sits above (`true`, the default) or below the parent's own contents.
    pub z_above_parent: bool,
}

impl Surface {
    pub fn new(id: SurfaceId) -> Self {
        Surface {
            id,
            role: Role::None,
            pending: Pending::default(),
            cached: None,
            current: Current { scale: 1, y_invert: true, ..Default::default() },
            subtitle_region_live: false,
            children: Vec::new(),
            position: (0, 0),
            sync: true,
            z_above_parent: true,
        }
    }

    pub fn is_mapped(&self) -> bool {
        self.current.buffer.is_some()
    }

    pub fn size(&self) -> (i32, i32) {
        self.current.size.unwrap_or((0, 0))
    }

    pub fn rect_at(&self, x: i32, y: i32) -> Rect {
        let (w, h) = self.size();
        Rect::new(x, y, w, h)
    }
}

/// Outcome of a commit the server layer must act on.
#[derive(Default, Debug)]
pub struct CommitEffects {
    /// The surface went from unmapped to mapped (first buffer) or the reverse.
    pub mapped_changed: Option<bool>,
    /// Placement, buffer dimensions or scaling changed, independently of pixel damage.
    pub geometry_changed: bool,
    /// A new buffer became current (import it, add damage).
    pub new_buffer: bool,
    pub size: Option<(i32, i32)>,
}

/// Every live surface.
#[derive(Default)]
pub struct Surfaces {
    map: BTreeMap<SurfaceId, Surface>,
    next: u32,
}

impl Surfaces {
    pub fn create(&mut self) -> SurfaceId {
        self.next += 1;
        let id = SurfaceId(self.next);
        self.map.insert(id, Surface::new(id));
        id
    }

    pub fn get(&self, id: SurfaceId) -> Option<&Surface> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: SurfaceId) -> Option<&mut Surface> {
        self.map.get_mut(&id)
    }

    pub fn remove(&mut self, id: SurfaceId) -> Option<Surface> {
        let s = self.map.remove(&id)?;
        if let Role::Subsurface { parent } = s.role
            && let Some(p) = self.map.get_mut(&parent)
        {
            p.children.retain(|c| *c != id);
        }
        for c in &s.children {
            if let Some(child) = self.map.get_mut(c) {
                child.role = Role::None;
            }
        }
        Some(s)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Surface> {
        self.map.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Surface> {
        self.map.values_mut()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `wl_surface.commit`. A synchronized subsurface caches; otherwise the pending state is applied, then the
    /// cached state of every synchronized child, recursively (the protocol's "parent commit applies children").
    pub fn commit(&mut self, id: SurfaceId, mut known_size: impl FnMut(&WlBuffer) -> Option<((i32, i32), bool)>) -> Vec<(SurfaceId, CommitEffects)> {
        let mut effects = Vec::new();
        let Some(s) = self.map.get_mut(&id) else { return effects };
        let is_sync_child = matches!(s.role, Role::Subsurface { .. }) && self.effective_sync(id);
        if is_sync_child {
            let s = self.map.get_mut(&id).unwrap();
            let p = s.pending.take();
            match &mut s.cached {
                Some(c) => p.merge_into(c),
                None => s.cached = Some(p),
            }
            return effects;
        }
        self.apply_pending(id, &mut known_size, &mut effects);
        effects
    }

    /// A subsurface is synchronized when it or any ancestor subsurface is in sync mode.
    fn effective_sync(&self, id: SurfaceId) -> bool {
        let mut cur = id;
        loop {
            let Some(s) = self.map.get(&cur) else { return false };
            match s.role {
                Role::Subsurface { parent } => {
                    if s.sync {
                        return true;
                    }
                    cur = parent;
                }
                _ => return false,
            }
        }
    }

    fn apply_pending(
        &mut self,
        id: SurfaceId,
        known_size: &mut impl FnMut(&WlBuffer) -> Option<((i32, i32), bool)>,
        effects: &mut Vec<(SurfaceId, CommitEffects)>,
    ) {
        let Some(s) = self.map.get_mut(&id) else { return };
        let mut p = s.pending.take();
        if let Some(cached) = s.cached.take() {
            // Cached state is older than the pending one: apply it first.
            let newer = p;
            p = cached;
            newer.merge_into(&mut p);
        }
        let mut eff = CommitEffects::default();
        let was_mapped = s.current.buffer.is_some();
        let geometry = (s.position, s.current.size, s.current.scale, s.current.transform);
        if let Some(b) = p.buffer {
            let old = s.current.buffer.take();
            match b {
                Some(nb) => {
                    if let Some(o) = old
                        && o.id() != nb.id()
                    {
                        s.current.retired.push(o);
                    }
                    match known_size(&nb) {
                        Some((size, y_invert)) => {
                            s.current.size = Some(size);
                            s.current.y_invert = y_invert;
                        }
                        None => s.current.size = None,
                    }
                    s.current.buffer = Some(nb);
                    s.current.buffer_commits += 1;
                    eff.new_buffer = true;
                }
                None => {
                    if let Some(o) = old {
                        s.current.retired.push(o);
                    }
                    s.current.size = None;
                }
            }
        }
        if let Some(o) = p.opaque {
            s.current.opaque = o;
        }
        if let Some(i) = p.input {
            s.current.input = i;
        }
        if let Some(bounds) = p.subtitle_bounds {
            s.current.subtitle_bounds = Some(bounds);
        }
        if let Some(sc) = p.scale {
            s.current.scale = sc.max(1);
        }
        if let Some(t) = p.transform {
            s.current.transform = t;
        }
        if let Some(pos) = p.sub_position {
            s.position = pos;
        }
        eff.geometry_changed = geometry != (s.position, s.current.size, s.current.scale, s.current.transform);
        // Damage: surface-local rectangles, buffer damage divided by scale. Any full-surface rectangle, a new
        // buffer of another size or a first buffer means "all".
        let all = eff.new_buffer && (!was_mapped || s.current.damage.is_none());
        if all || p.damage.iter().any(|r| r.x <= 0 && r.y <= 0 && r.w >= i32::MAX / 2) {
            s.current.damage = None;
        } else {
            let scale = s.current.scale.max(1);
            let mut list = s.current.damage.take().unwrap_or_default();
            list.extend(p.damage);
            list.extend(p.buffer_damage.into_iter().map(|r| Rect::new(r.x / scale, r.y / scale, (r.w + scale - 1) / scale, (r.h + scale - 1) / scale)));
            if eff.new_buffer && list.is_empty() {
                // A buffer without damage: nothing to show new, but LG's clients sometimes omit damage; be safe.
                s.current.damage = None;
            } else {
                s.current.damage = Some(list);
            }
        }
        if eff.new_buffer || s.current.damage.as_ref().map(|d| !d.is_empty()).unwrap_or(true) {
            s.current.undrawn = true;
            s.current.content_ns.get_or_insert_with(crate::output::monotonic_ns);
        }
        s.current.frame_callbacks.extend(p.frame_callbacks);
        let now_mapped = s.current.buffer.is_some();
        if was_mapped && !now_mapped {
            s.current.subtitle_bounds = None;
        }
        if was_mapped != now_mapped {
            eff.mapped_changed = Some(now_mapped);
        }
        eff.size = s.current.size;
        let children = s.children.clone();
        effects.push((id, eff));
        for c in children {
            let sync = self.map.get(&c).map(|cs| cs.sync).unwrap_or(false);
            if sync {
                self.apply_pending(c, known_size, effects);
            }
        }
    }

    /// Takes every frame callback owed by every surface (fired after a presented frame): the callbacks of the surfaces `ahead` names
    /// apart from the others (`State::frames_in_flight`).
    pub fn take_frame_callbacks(&mut self, ahead: impl Fn(SurfaceId) -> bool) -> (Vec<WlCallback>, Vec<WlCallback>) {
        let (mut others, mut named) = (Vec::new(), Vec::new());
        for (id, s) in self.map.iter_mut() {
            if ahead(*id) {
                named.append(&mut s.current.frame_callbacks);
            } else {
                others.append(&mut s.current.frame_callbacks);
            }
        }
        (others, named)
    }

    /// Takes the frame callbacks one surface owes.
    pub fn take_frame_callbacks_of(&mut self, id: SurfaceId) -> Vec<WlCallback> {
        self.map.get_mut(&id).map(|s| std::mem::take(&mut s.current.frame_callbacks)).unwrap_or_default()
    }

    /// Takes every retired buffer (released after the frame that last read them), each into the bucket `bucket` names for its surface
    /// (`State::tick_at`: given back now, with its frame's callbacks, or when its frame lands).
    pub fn take_retired<const N: usize>(&mut self, bucket: impl Fn(SurfaceId) -> usize) -> [Vec<WlBuffer>; N] {
        let mut out = std::array::from_fn(|_| Vec::new());
        for (id, s) in self.map.iter_mut() {
            out[bucket(*id)].append(&mut s.current.retired);
        }
        out
    }

    /// Clears per-surface damage after a presented frame.
    pub fn clear_damage(&mut self) {
        for s in self.map.values_mut() {
            s.current.damage = Some(Vec::new());
            s.current.undrawn = false;
            s.current.content_ns = None;
        }
    }

    /// Whether `id` is mapped with content no frame has shown yet.
    pub fn has_undrawn(&self, id: SurfaceId) -> bool {
        self.map.get(&id).map(|s| s.is_mapped() && s.current.undrawn).unwrap_or(false)
    }

    /// Whether any mapped surface holds content no frame has shown yet.
    pub fn any_undrawn(&self) -> bool {
        self.map.values().any(|s| s.is_mapped() && s.current.undrawn)
    }

    /// Makes `child` a subsurface of `parent`, placed above it.
    pub fn add_subsurface(&mut self, child: SurfaceId, parent: SurfaceId) -> bool {
        if child == parent || !self.map.contains_key(&child) || !self.map.contains_key(&parent) {
            return false;
        }
        if let Some(c) = self.map.get_mut(&child) {
            if c.role != Role::None {
                return false;
            }
            c.role = Role::Subsurface { parent };
            c.sync = true;
        }
        self.map.get_mut(&parent).unwrap().children.push(child);
        true
    }

    /// `place_above`/`place_below` relative to a sibling or the parent itself.
    pub fn place(&mut self, child: SurfaceId, sibling: SurfaceId, above: bool) {
        let Some(Role::Subsurface { parent }) = self.map.get(&child).map(|s| s.role) else { return };
        let Some(p) = self.map.get(&parent) else { return };
        let mut children = p.children.clone();
        children.retain(|c| *c != child);
        // Children are ordered bottom to top with the below-parent ones first; "directly above the parent" is the
        // first slot of the above group and "directly below" the last slot of the below group: the same index.
        let first_above = children.iter().position(|c| self.map_z_above(c)).unwrap_or(children.len());
        let (idx, z_above) = if sibling == parent {
            (first_above, above)
        } else {
            match children.iter().position(|c| *c == sibling) {
                Some(pos) => (if above { pos + 1 } else { pos }, self.map_z_above(&sibling)),
                None => (children.len(), true),
            }
        };
        children.insert(idx, child);
        self.map.get_mut(&parent).unwrap().children = children;
        self.map.get_mut(&child).unwrap().z_above_parent = z_above;
    }

    fn map_z_above(&self, id: &SurfaceId) -> bool {
        self.map.get(id).map(|s| s.z_above_parent).unwrap_or(true)
    }

    /// The draw order of a surface tree rooted at `root`, each with its position relative to the root's origin:
    /// below-parent children (in order), the surface itself, above-parent children (in order), recursively.
    pub fn tree_in_draw_order(&self, root: SurfaceId) -> Vec<(SurfaceId, i32, i32)> {
        let mut out = Vec::new();
        self.collect(root, 0, 0, &mut out);
        out
    }

    /// As [`Surfaces::tree_in_draw_order`], into `out` (cleared first): the frame path reuses one buffer.
    pub fn tree_in_draw_order_into(&self, root: SurfaceId, out: &mut Vec<(SurfaceId, i32, i32)>) {
        out.clear();
        self.collect(root, 0, 0, out);
    }

    fn collect(&self, id: SurfaceId, x: i32, y: i32, out: &mut Vec<(SurfaceId, i32, i32)>) {
        let Some(s) = self.map.get(&id) else { return };
        for c in &s.children {
            if let Some(cs) = self.map.get(c)
                && !cs.z_above_parent
            {
                self.collect(*c, x + cs.position.0, y + cs.position.1, out);
            }
        }
        out.push((id, x, y));
        for c in &s.children {
            if let Some(cs) = self.map.get(c)
                && cs.z_above_parent
            {
                self.collect(*c, x + cs.position.0, y + cs.position.1, out);
            }
        }
    }
}
