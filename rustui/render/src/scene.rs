//! The layers on screen and how they move: creation order and z, resting properties, running animations, and the
//! motion of wallpapers and 3D scenes. It is pure data, driven by the render thread; the GPU never appears here.
//!
//! A frame is made in three steps: the changes of the commits that are due are `apply`d, `advance` brings the scene to
//! the frame's time (animations land, wallpapers and orbits step), and `resolve` lists what is drawn.

use crate::anim::Animation;
use crate::groups::Groups;
use crate::ladder::Ambient;
use crate::ops::Op;
use crate::prepared;
use crate::wallpaper::{self, Clock, MAX_RIPPLES, Ripple, Ripples};
use compositor_protocol::group::World;
use compositor_protocol::motion::Notice;
use compositor_protocol::{
    Content, GroupId, LayerId, MotionId, Props, Rect, Wallpaper, WallpaperKind,
};
use std::collections::HashMap;

struct Layer {
    width: u32,
    height: u32,
    z: i32,
    content: Content,
    props: Props,
    animation: Option<Animation>,
    /// Changes whenever the layer changes what it is or where it rests: what a cached backdrop depends on.
    version: u64,
    /// The motion of a wallpaper (seconds of its designed pace) or a mesh (degrees of orbit).
    clock: Clock,
    /// Taps on a `Liquid` wallpaper, each with the wallpaper's phase when it happened.
    ripples: Vec<(f32, f32, f64)>,
    /// Whether nothing the layer moves by itself showed in the last frame (`Scene::observe`).
    unseen: bool,
    /// The group the layer hangs under, if any: `props` are then in that group's space.
    group: Option<GroupId>,
    /// A grouped layer's props at rest as the last frame flattened them: a change to them (a group the layer hangs under
    /// came to rest somewhere else) is a change to where the layer rests, and so to `version`.
    flat_rest: Option<Props>,
}

impl Layer {
    fn props_at(&self, now_ns: u64, screen: Rect) -> Props {
        match &self.animation {
            Some(animation) => animation.props_at(now_ns, screen),
            None => self.props,
        }
    }

    /// Whether the layer is on screen when it rests.
    fn shown_at_rest(&self) -> bool {
        is_shown(&self.flat_rest.unwrap_or(self.props))
    }

    fn orbits(&self, ambient: Ambient) -> bool {
        ambient.orbits() && matches!(self.content, Content::Mesh(m) if m.orbit_deg_per_s != 0.0)
    }
}

fn is_shown(props: &Props) -> bool {
    props.visible && props.opacity > 0.0 && props.scale > 0.0
}

/// A layer as it is drawn this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub id: LayerId,
    pub width: u32,
    pub height: u32,
    pub content: Content,
    pub props: Props,
    /// Where the layer rests: `props` without the animation that is running on it, if any.
    pub rest: Props,
    /// Changes whenever the layer changes what it is or where it rests (it is created, restyled, damaged or set, or an
    /// animation on it lands), and only then.
    pub version: u64,
    /// The layer's own motion: for a wallpaper, seconds of its designed pace; for a mesh, degrees of orbit.
    pub phase: f64,
    pub ripples: Ripples,
}

/// When the scene next wants a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demand {
    /// Nothing moves: no frame is due until something is committed.
    Idle,
    /// Something needs a frame at every refresh: an animation, an orbiting mesh, a wallpaper at the panel's rate.
    EveryRefresh,
    /// Only wallpapers that move slower than the panel: the frame time at which the first is due to step.
    At(u64),
}

pub struct Scene {
    layers: HashMap<LayerId, Layer>,
    /// The layer ids by stacking order, kept as layers come and go.
    order: Vec<LayerId>,
    groups: Groups,
    /// The time `resolve` last drew for, for `prepared::Source::layer`.
    last_now: u64,
    next_version: u64,
    screen: Rect,
    refresh_period_ns: u64,
    /// The renderer is bringing a glass surface's refraction back (`Renderer::adapt`) as of the frame just drawn: the frame that does
    /// it is wanted ([`Scene::demand`]).
    fading: Option<Fading>,
    /// What the Full ladder lets the wallpapers and the orbiting meshes do (`ladder::Ambient`): all that they ask for, unless a
    /// step is in force.
    ambient: Ambient,
}

/// The frame just drawn moved glass ([`Scene::fade`]): the refresh it was drawn for, and whether it took what the app had sent
/// (a commit, a pointer sample): an app that sends a frame's worth at every refresh is a finger on the screen.
#[derive(Clone, Copy, Debug)]
struct Fading {
    slot_ns: u64,
    driven: bool,
}

/// How many refreshes glass that moved in a frame that took what the app sent stands still before the frame that brings its
/// refraction back is drawn, if nothing comes first (`Scene::demand`): 6, 100 ms.
///
/// A glass surface that moved in a frame has none of its refraction, and the first frame that finds it standing has it all (no
/// fade: `renderer::CALM_NS`). That frame has no commit in it, so it is drawn on the scene's own account, and a commit that arrives
/// while it is in flight waits for it and is shown a refresh late (one frame is in flight at a time, and this one is latched as late
/// as its pipeline allows, 12 ms before the refresh it is aimed at). Wanted for the next refresh it was drawn after every frame of
/// a drag of a page with glass in it (the Home screen's pills and switch): the commit for the finger's next step arrives 9 ms after
/// the flip, the frame has been in flight since 5, so the commit is a refresh late, and so is every one after it until the
/// resampler has slid its deliveries off the frame's time. A refresh in six showed the commit before it again (17 % of the
/// refreshes of two of Nitin's drags on the Show, 10-06), and in a replay of those touches through the resampler and a model of
/// the compositor the finger's position took 20 ms to reach the screen instead of 3.
///
/// A finger that drags pauses between its steps for a refresh or two all the time (152 pauses of 1 or 2 refreshes in those drags,
/// 94 of 3 or 4, 36 of 5, 30 of 6 or more), and a frame that comes while it is paused is in the way of the step that ends the
/// pause. In the replay a frame wanted for 2 refreshes on was in the way of 2 % of the finger's steps, for 4 of 0.5-0.9 %, for 6
/// of 0.1 %. A refraction that comes back 100 ms after the glass stood still is not seen: Nitin compared one that came back at
/// once with one that took 100 and 200 ms on the Show (10-01) and could not tell them apart.
///
/// Only a frame that took what the app sent waits: no commit is on its way after the last frame of an animation or a settle, nor
/// after a frame that is itself bringing refraction back over several frames (`LUMASHOW_CALM_MS`), and those are drawn at every
/// refresh as before.
const GLASS_STILL_REFRESHES: u64 = 6;

/// A mesh's orbit steps at every frame, by the time since the last one.
fn orbit_interval(refresh_period_ns: u64) -> Option<u64> {
    Some(refresh_period_ns)
}

impl Scene {
    pub fn new(logical_width: f32, logical_height: f32, refresh_period_ns: u64) -> Self {
        Self {
            layers: HashMap::new(),
            order: Vec::new(),
            groups: Groups::new(),
            last_now: 0,
            next_version: 0,
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: logical_width,
                h: logical_height,
            },
            refresh_period_ns,
            fading: None,
            ambient: Ambient::Asked,
        }
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    fn fresh_version(&mut self) -> u64 {
        self.next_version += 1;
        self.next_version
    }

    /// Applies one change at the frame time `now_ns`, which is when an animation it starts begins. Changes to
    /// layers that are not there are ignored: the command thread never sends them, and a scene must survive a
    /// stale one anyway.
    pub fn apply(&mut self, op: &Op, now_ns: u64) {
        match op {
            Op::Create {
                id,
                width,
                height,
                z,
                content,
                props,
                ..
            } => {
                let version = self.fresh_version();
                self.remove_layer(*id);
                self.layers.insert(
                    *id,
                    Layer {
                        width: *width,
                        height: *height,
                        z: *z,
                        content: *content,
                        props: *props,
                        animation: None,
                        version,
                        clock: Clock::default(),
                        ripples: Vec::new(),
                        unseen: false,
                        group: None,
                        flat_rest: None,
                    },
                );
                // After every layer of its z that is already there: it was made last.
                let at = self.order.partition_point(|o| self.layers[o].z <= *z);
                self.order.insert(at, *id);
            }
            Op::Restyle { id, content } => {
                // The session only lets a layer be restyled into its own kind; a stale change to another kind is
                // ignored rather than turning a layer into something its resources do not fit.
                let version = self.next_version + 1;
                let same_kind = |l: &&mut Layer| {
                    std::mem::discriminant(&l.content) == std::mem::discriminant(content)
                };
                // The look it already has is no change: a toolkit says the look of every glass again each time it draws, and a
                // new version is the whole layer drawn again (and, over a glass, its backdrop sampled again). How often a wallpaper
                // steps says nothing of the picture it shows now, so that is no new version either: the steps that follow are.
                if let Some(layer) = self
                    .layers
                    .get_mut(id)
                    .filter(same_kind)
                    .filter(|l| l.content != *content)
                {
                    let shows_the_same = matches!(
                        (layer.content, *content),
                        (Content::Wallpaper(old), Content::Wallpaper(new)) if Wallpaper { fps: new.fps, ..old } == new
                    );
                    layer.content = *content;
                    if !shows_the_same {
                        layer.version = version;
                        self.next_version = version;
                    }
                }
            }
            Op::Damage { id, .. } => {
                let version = self.next_version + 1;
                if let Some(layer) = self.layers.get_mut(id) {
                    layer.version = version;
                    self.next_version = version;
                }
            }
            Op::Ripple { id, x, y } => {
                if let Some(layer) = self.layers.get_mut(id).filter(|l| {
                    matches!(
                        l.content,
                        Content::Wallpaper(Wallpaper {
                            kind: WallpaperKind::Liquid,
                            ..
                        })
                    )
                }) {
                    if layer.ripples.len() == MAX_RIPPLES {
                        layer.ripples.remove(0);
                    }
                    layer.ripples.push((*x, *y, layer.clock.phase));
                }
            }
            Op::Set { id, props } => {
                let version = self.next_version + 1;
                // The props it already has with nothing animating them are no change (the toolkit poses a card or a dock each time it
                // draws): not a new version, which would be the layer drawn again. A running animation is still ended by it.
                if let Some(layer) = self
                    .layers
                    .get_mut(id)
                    .filter(|l| l.animation.is_some() || l.props != *props)
                {
                    layer.props = *props;
                    layer.animation = None;
                    layer.version = version;
                    self.next_version = version;
                }
            }
            Op::Animate {
                id,
                to,
                duration_ms,
                ease,
                step_ms,
            } => {
                let version = self.next_version + 1;
                if let Some(layer) = self.layers.get_mut(id) {
                    // From wherever the layer is right now, so a new animation takes over a running one smoothly.
                    let from = layer.props_at(now_ns, self.screen);
                    if *duration_ms == 0 || from == *to {
                        if layer.props != *to {
                            layer.version = version;
                            self.next_version = version;
                        }
                        layer.props = *to;
                        layer.animation = None;
                    } else {
                        layer.props = from;
                        layer.animation = Some(
                            Animation::new(from, *to, now_ns, *duration_ms, *ease).stepped(*step_ms),
                        );
                    }
                }
            }
            Op::Delete { id } => {
                self.remove_layer(*id);
            }
            Op::Clear => {
                self.layers.clear();
                self.order.clear();
                self.groups.clear();
                // The glass that was bringing its refraction back is gone with the client: a frame for it would draw an empty
                // scene over the last picture, which stays on the screen until the next client shows something.
                self.fading = None;
            }
            Op::GroupCreate { id, parent, pose } => {
                self.groups.create(*id, *parent, *pose);
            }
            Op::GroupSet { id, pose } => self.groups.set_pose(*id, *pose),
            Op::GroupParent {
                id,
                parent,
                preserve_world,
            } => {
                self.groups.reparent(*id, *parent, *preserve_world);
            }
            Op::GroupBind {
                id,
                motion,
                binding,
            } => self.groups.bind(*id, *motion, *binding),
            Op::GroupUnbind { id, properties } => self.groups.unbind(*id, *properties),
            Op::GroupDelete { id } => {
                if let Some((parent, relative)) = self.groups.delete(*id) {
                    self.release_layers(*id, parent, &relative);
                }
            }
            Op::Attach { id, group, local } => {
                let version = self.next_version + 1;
                let known = group.is_none_or(|g| self.groups.contains(g));
                if let Some(layer) = self.layers.get_mut(id).filter(|_| known) {
                    layer.group = *group;
                    layer.props = *local;
                    layer.animation = None;
                    layer.flat_rest = None;
                    layer.version = version;
                    self.next_version = version;
                }
            }
            Op::MotionCreate { id, value, bounds } => {
                self.groups.create_motion(*id, *value, *bounds);
            }
            Op::MotionDelete { id } => self.groups.delete_motion(*id),
            Op::MotionDrive { id, sample } => self.groups.drive(*id, *sample, now_ns),
            Op::MotionSettle {
                id,
                target,
                velocity,
                settle,
            } => self.groups.settle(*id, *target, *velocity, *settle, now_ns),
            Op::MotionCancel { id, policy } => self.groups.cancel(*id, *policy, now_ns),
            // The renderer's: prepared sets, atlases and the glyphs of a run. A glyph run that changes is the renderer's to
            // say (its revision joins the layer's version where that matters).
            Op::HoldGpuClock(_)
            | Op::Ideas(_)
            | Op::SmoothFirst(_)
            | Op::Display(_)
            | Op::BackdropCreate { .. }
            | Op::BackdropRefresh { .. }
            | Op::BackdropFreeze { .. }
            | Op::BackdropRelease { .. }
            | Op::BackdropDelete { .. }
            | Op::AtlasCreate { .. }
            | Op::AtlasUpload { .. }
            | Op::AtlasDelete { .. }
            | Op::SetGlyphs { .. }
            | Op::LiveAttach { .. } => {}
            // A producer's picture came or went: every layer that shows the source is drawn again, as for a damaged image.
            Op::LiveFrame { source, .. } | Op::LiveClear { source } | Op::LiveDetach { source } => {
                let version = self.next_version + 1;
                let mut bumped = false;
                for layer in self.layers.values_mut() {
                    if matches!(layer.content, Content::Live(live) if live.source == *source) {
                        layer.version = version;
                        bumped = true;
                    }
                }
                if bumped {
                    self.next_version = version;
                }
            }
        }
    }

    /// Takes a layer out of the scene and the stacking order.
    fn remove_layer(&mut self, id: LayerId) {
        if self.layers.remove(&id).is_some() {
            self.order.retain(|o| *o != id);
        }
    }

    /// The layers under the deleted group `deleted` go to `parent`, where `relative` (the deleted group's pose evaluated
    /// now) turns their props in its space into props in the parent's: nothing moves, and no group is left holding them.
    fn release_layers(&mut self, deleted: GroupId, parent: Option<GroupId>, relative: &World) {
        let version = self.next_version + 1;
        let mut bumped = false;
        for layer in self
            .layers
            .values_mut()
            .filter(|l| l.group == Some(deleted))
        {
            let size = (layer.width as f32, layer.height as f32);
            let carry = |props: &Props| relative.props(props, size);
            layer.props = relative.props(&layer.props, size);
            layer.animation = layer.animation.map(|a| a.map_props(carry));
            layer.group = parent;
            layer.flat_rest = None;
            layer.version = version;
            bumped = true;
        }
        if bumped {
            self.next_version = version;
        }
    }

    /// Brings the scene to the frame time `now_ns`: the animations that have finished land (their layers rest on the
    /// target, and a cached backdrop below or above them will be told), wallpapers and orbiting meshes take a step if
    /// one is due, and ripples that have run their course are gone.
    pub fn advance(&mut self, now_ns: u64) {
        let (screen, period, ambient) = (self.screen, self.refresh_period_ns, self.ambient);
        let mut landed = Vec::new();
        for (id, layer) in &mut self.layers {
            if let Some(animation) = layer.animation.filter(|a| a.is_done(now_ns)) {
                layer.props = animation.target();
                layer.animation = None;
                landed.push(*id);
            }
        }
        for id in landed {
            let version = self.fresh_version();
            if let Some(layer) = self.layers.get_mut(&id) {
                layer.version = version;
            }
        }
        // The tracks come to the frame's time, the tree is evaluated once, and a layer whose group came to rest
        // somewhere else rests somewhere else: its version changes, so that a cached backdrop below glass is made again
        // when the motion is over and not before.
        self.groups.advance(now_ns);
        self.groups.evaluate();
        let mut next = self.next_version;
        for layer in self.layers.values_mut() {
            let Some(w) = layer.group.and_then(|g| self.groups.worlds(g)) else {
                layer.flat_rest = None;
                continue;
            };
            let size = (layer.width as f32, layer.height as f32);
            let rest = w.rest.props(&layer.props, size);
            if layer.flat_rest.is_some_and(|before| before != rest) {
                next += 1;
                layer.version = next;
            }
            layer.flat_rest = Some(rest);
        }
        self.next_version = next;
        self.last_now = now_ns;
        for layer in self.layers.values_mut() {
            let local = layer.props_at(now_ns, screen);
            let shown = match layer.group.and_then(|g| self.groups.worlds(g)) {
                Some(w) => is_shown(
                    &w.now
                        .props(&local, (layer.width as f32, layer.height as f32)),
                ),
                None => is_shown(&local),
            };
            match layer.content {
                Content::Wallpaper(w) => {
                    let interval = wallpaper::step_interval_ns(ambient.fps(w.fps), period);
                    if layer
                        .clock
                        .advance(now_ns, shown, interval, w.speed, period / 2)
                    {
                        let phase = layer.clock.phase;
                        layer
                            .ripples
                            .retain(|&(_, _, born)| phase - born < wallpaper::RIPPLE_LIFE_S);
                    }
                }
                Content::Mesh(m) => {
                    layer.clock.advance(
                        now_ns,
                        shown,
                        orbit_interval(period),
                        if ambient.orbits() {
                            m.orbit_deg_per_s
                        } else {
                            0.0
                        },
                        period / 2,
                    );
                }
                _ => {}
            }
        }
    }

    /// The renderer's account of the frame just drawn: the layers that nothing of showed, neither in the frame nor in a
    /// picture behind glass, because an opaque layer covered them (`Renderer::unseen`). A wallpaper or a turning mesh
    /// among them asks for no frames, since nobody would see what they draw; it asks again with the first frame that
    /// shows any of it. Every frame's account replaces the one before.
    pub fn observe(&mut self, unseen: &[LayerId]) {
        for layer in self.layers.values_mut() {
            layer.unseen = false;
        }
        for id in unseen {
            if let Some(layer) = self.layers.get_mut(id) {
                layer.unseen = true;
            }
        }
    }

    /// Holds the wallpapers and the orbiting meshes to what the steps of the Full ladder allow (`ladder::Ambient`). A wallpaper
    /// that is held still stands where it is, and goes on from there when it is let go of; the frame that asks for its next step
    /// is wanted again as soon as it is ([`Scene::demand`]).
    pub fn set_ambient(&mut self, ambient: Ambient) {
        self.ambient = ambient;
    }

    /// What the layers are made of, in no order: what the ladder asks whether a step would change anything it draws.
    pub fn contents(&self) -> impl Iterator<Item = &Content> {
        self.layers.values().map(|layer| &layer.content)
    }

    /// Whether something a person set going still moves after the frame drawn for `at_ns`: a finger on a track (down, or just off
    /// it with no settle asked for yet), a settle that does not come to rest by then, a layer's animation that is not over. What
    /// the ladder's steps wait for: they are taken, and given back, at a settled boundary and not in the middle of a motion. The
    /// ambient motion (a wallpaper, an orbiting mesh) is nobody's gesture and is not counted.
    pub fn moving_after(&self, at_ns: u64) -> bool {
        self.groups.any_moving_after(at_ns)
            || self
                .layers
                .values()
                .any(|l| l.animation.is_some_and(|a| !a.is_done(at_ns)))
    }

    /// Whether any layer needs a frame at every refresh: an animation is running (whether or not its layer shows: it may
    /// be moving into sight) or a mesh that shows is orbiting. An ambient motion (`Animation::stepped`) is not: it wants a frame
    /// every so often ([`Scene::demand`]).
    pub fn animating(&self) -> bool {
        self.groups.any_settling()
            || self.layers.values().any(|l| {
                l.animation.is_some_and(|a| !a.is_paced())
                    || (l.orbits(self.ambient) && l.shown_at_rest() && !l.unseen)
            })
    }

    /// Whether the frame drawn for `at_ns` brings a motion to rest: a settle arrives by then, or a layer's own animation
    /// is over by then. Their layers rest somewhere else from that frame, so the cached backdrops below glass are made again
    /// in it: it costs what none of the motion's frames did.
    pub fn rests_by(&self, at_ns: u64) -> bool {
        self.groups.any_settles_by(at_ns)
            || self
                .layers
                .values()
                .any(|l| l.animation.is_some_and(|a| a.is_done(at_ns)))
    }

    /// Whether the renderer was fading something in as of the frame just drawn (see [`Scene::fade`]).
    pub fn is_fading(&self) -> bool {
        self.fading.is_some()
    }

    /// Says whether the renderer is fading something in, as of the frame just drawn (for the slot the scene was last brought to):
    /// glass that moved in it has no refraction, and a frame that brings it back is wanted ([`Scene::demand`]). `driven`: the frame
    /// took what the app had sent.
    pub fn fade(&mut self, fading: bool, driven: bool) {
        self.fading = fading.then_some(Fading {
            slot_ns: self.last_now,
            driven,
        });
    }

    /// When the scene next wants a frame with nothing committed: an animation wants one at every refresh, and so does glass that moved
    /// in a frame that took nothing from the app (see [`Scene::fade`]); glass that moved in one that did wants the frame that brings
    /// its refraction back once it has stood still for [`GLASS_STILL_REFRESHES`] refreshes (a commit that comes sooner takes its
    /// place: it moves the glass again, or draws it still); a wallpaper wants its next step; and an ambient motion wants one a step
    /// after the last frame drawn (whatever that frame was for: it showed the motion where it was), or when it lands if that is
    /// sooner.
    pub fn demand(&self) -> Demand {
        if self.animating() || self.fading.is_some_and(|f| !f.driven) {
            return Demand::EveryRefresh;
        }
        let mut next: Option<u64> = self
            .fading
            .map(|f| f.slot_ns + GLASS_STILL_REFRESHES * self.refresh_period_ns);
        for animation in self.layers.values().filter_map(|l| l.animation) {
            if animation.is_paced() {
                let due = self
                    .last_now
                    .saturating_add(animation.step_ns())
                    .min(animation.ends_at());
                next = Some(next.map_or(due, |n| n.min(due)));
            }
        }
        for layer in self.layers.values().filter(|l| !l.unseen) {
            let Content::Wallpaper(w) = layer.content else {
                continue;
            };
            let interval =
                wallpaper::step_interval_ns(self.ambient.fps(w.fps), self.refresh_period_ns);
            let Some(due) = layer.clock.next_step(interval) else {
                continue;
            };
            if interval.is_some_and(|i| i <= self.refresh_period_ns) {
                return Demand::EveryRefresh;
            }
            next = Some(next.map_or(due, |n| n.min(due)));
        }
        next.map_or(Demand::Idle, Demand::At)
    }

    /// The layers to draw at `now_ns`, back to front. Layers that are hidden, transparent or scaled to nothing
    /// are left out. A layer under a group is drawn as that group's world (as of the last `advance`) puts it.
    pub fn resolve(&self, now_ns: u64, out: &mut Vec<Drawn>) {
        out.clear();
        for id in &self.order {
            let layer = &self.layers[id];
            let (props, rest) = self.flatten(layer, now_ns);
            if !is_shown(&props) {
                continue;
            }
            out.push(self.drawn(*id, layer, props, rest));
        }
    }

    /// `layer` as it is drawn with `props` now and `rest` at rest.
    fn drawn(&self, id: LayerId, layer: &Layer, props: Props, rest: Props) -> Drawn {
        let phase = layer.clock.phase;
        let mut ripples = Ripples::default();
        for &(x, y, born) in &layer.ripples {
            ripples.items[ripples.count] = Ripple {
                x,
                y,
                age_s: (phase - born).max(0.0) as f32,
            };
            ripples.count += 1;
        }
        Drawn {
            id,
            width: layer.width,
            height: layer.height,
            content: layer.content,
            props,
            rest,
            version: layer.version,
            phase,
            ripples,
        }
    }

    /// The props of `layer` on screen now and at rest: its own (animated) props as they are, or, under a group, in the
    /// group's world. A group that is not in the last evaluation (made since) leaves the layer where its own props say.
    fn flatten(&self, layer: &Layer, now_ns: u64) -> (Props, Props) {
        let local = layer.props_at(now_ns, self.screen);
        let worlds = layer.group.and_then(|g| self.groups.worlds(g));
        match worlds {
            Some(w) => {
                let size = (layer.width as f32, layer.height as f32);
                (w.now.props(&local, size), w.rest.props(&layer.props, size))
            }
            None => (local, layer.props),
        }
    }

    /// The groups and tracks the client holds now.
    pub fn counts(&self) -> (usize, usize) {
        (self.groups.group_count(), self.groups.motion_count())
    }

    /// The notices of motion tracks since the last call, appended to `out` (`Event::Motion`).
    pub fn take_notices(&mut self, out: &mut Vec<(MotionId, Notice)>) {
        self.groups.take_notices(out);
    }
}

impl prepared::Source for Scene {
    fn plate_layers(&self, group: GroupId, out: &mut Vec<Drawn>) {
        out.clear();
        for id in &self.order {
            let layer = &self.layers[id];
            let Some(from) = layer.group else { continue };
            let Some(world) = self.groups.world_below(from, group, true) else {
                continue;
            };
            let size = (layer.width as f32, layer.height as f32);
            // Plates are made of the layers as they rest: whatever moves when the plate is made is where it comes to.
            let props = world.props(&layer.props, size);
            if is_shown(&props) {
                out.push(self.drawn(*id, layer, props, props));
            }
        }
    }

    fn layer(&self, id: LayerId) -> Option<Drawn> {
        let layer = self.layers.get(&id)?;
        let (props, rest) = self.flatten(layer, self.last_now);
        Some(self.drawn(id, layer, props, rest))
    }

    fn group_world(&self, group: GroupId) -> Option<World> {
        self.groups.worlds(group).map(|w| w.now)
    }

    fn moving(&self) -> bool {
        self.groups.any_active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::{Ease, Fill, Rgba, Shape};

    const MS: u64 = 1_000_000;

    fn shape() -> Shape {
        Shape {
            corner_radius: 4.0,
            fill: Fill::Solid(Rgba(1, 2, 3, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        }
    }

    fn create(id: u32, z: i32, props: Props) -> Op {
        Op::Create {
            id: LayerId(id),
            width: 100,
            height: 50,
            z,
            content: Content::Shape(shape()),
            props,
            image: None,
            mesh: None,
        }
    }

    fn ids(scene: &Scene, now: u64) -> Vec<u32> {
        let mut out = Vec::new();
        scene.resolve(now, &mut out);
        out.iter().map(|d| d.id.0).collect()
    }

    /// 59.64 Hz.
    const PERIOD: u64 = 16_767_000;

    fn scene() -> Scene {
        Scene::new(960.0, 480.0, PERIOD)
    }

    #[test]
    fn layers_stack_by_z_and_then_by_creation_order() {
        let mut s = scene();
        for (id, z) in [(1, 5), (2, 0), (3, 5), (4, -2)] {
            s.apply(&create(id, z, Props::at(0.0, 0.0)), 0);
        }
        assert_eq!(ids(&s, 0), vec![4, 2, 1, 3]);
    }

    #[test]
    fn creating_an_existing_id_replaces_the_layer_and_puts_it_on_top_of_its_z() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(&create(2, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(50.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        s.apply(&create(1, 0, Props::at(7.0, 0.0)), 0);
        assert_eq!(ids(&s, 0), vec![2, 1]);
        assert!(!s.animating(), "the replaced layer's animation is gone");
        let mut out = Vec::new();
        s.resolve(50 * MS, &mut out);
        assert_eq!(out[1].props.x, 7.0);
    }

    #[test]
    fn hidden_transparent_and_vanishing_layers_are_not_drawn() {
        let mut s = scene();
        let hidden = Props {
            visible: false,
            ..Props::at(0.0, 0.0)
        };
        let clear = Props {
            opacity: 0.0,
            ..Props::at(0.0, 0.0)
        };
        let tiny = Props {
            scale: 0.0,
            ..Props::at(0.0, 0.0)
        };
        s.apply(&create(1, 0, hidden), 0);
        s.apply(&create(2, 0, clear), 0);
        s.apply(&create(3, 0, tiny), 0);
        s.apply(&create(4, 0, Props::at(0.0, 0.0)), 0);
        assert_eq!(ids(&s, 0), vec![4]);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn set_moves_at_once_and_cancels_an_animation() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(100.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        assert!(s.animating());
        s.apply(
            &Op::Set {
                id: LayerId(1),
                props: Props::at(3.0, 4.0),
            },
            10 * MS,
        );
        assert!(!s.animating());
        let mut out = Vec::new();
        s.resolve(90 * MS, &mut out);
        assert_eq!((out[0].props.x, out[0].props.y), (3.0, 4.0));
    }

    #[test]
    fn an_animation_starts_at_the_frame_that_applies_it_and_ends_when_it_lands() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(100.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            1000 * MS,
        );
        let mut out = Vec::new();
        s.resolve(1000 * MS, &mut out);
        assert_eq!(out[0].props.x, 0.0);
        s.resolve(1050 * MS, &mut out);
        assert_eq!(out[0].props.x, 50.0);
        s.advance(1050 * MS);
        assert!(s.animating(), "still running halfway");
        s.advance(1100 * MS);
        assert!(!s.animating());
        s.resolve(2000 * MS, &mut out);
        assert_eq!(out[0].props.x, 100.0);
    }

    #[test]
    fn a_new_animation_continues_from_where_the_old_one_had_got_to() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(100.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(0.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            40 * MS,
        );
        let mut out = Vec::new();
        s.resolve(40 * MS, &mut out);
        assert_eq!(out[0].props.x, 40.0, "no jump at the hand-over");
        s.resolve(90 * MS, &mut out);
        assert_eq!(out[0].props.x, 20.0);
    }

    #[test]
    fn animating_to_where_it_already_is_or_for_no_time_needs_no_frames() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(5.0, 5.0)), 0);
        for (to, duration_ms) in [(Props::at(5.0, 5.0), 200), (Props::at(9.0, 5.0), 0)] {
            s.apply(
                &Op::Animate {
                    id: LayerId(1),
                    to,
                    duration_ms,
                    ease: Ease::Out,
                    step_ms: 0,
                },
                0,
            );
            assert!(!s.animating());
        }
        let mut out = Vec::new();
        s.resolve(0, &mut out);
        assert_eq!(out[0].props.x, 9.0);
    }

    #[test]
    fn a_settle_of_no_length_lands_in_the_frame_that_carries_it_and_one_of_a_millisecond_needs_another()
     {
        use compositor_protocol::motion::{MotionBounds, MotionId, MotionToken, Settle, Units};
        // The jump the line makes to follow the clock: a frame of its own that the compositor draws at once is a frame in the way of
        // what the finger commits next, so it must not need a second one.
        for (duration_ms, frames_needed) in [(0u32, false), (1, true)] {
            let mut s = scene();
            s.apply(
                &Op::MotionCreate {
                    id: MotionId(1),
                    value: 5.0,
                    bounds: MotionBounds::range(0.0, 100.0, Units::Pixels),
                },
                0,
            );
            s.apply(
                &Op::MotionSettle {
                    id: MotionId(1),
                    target: Some(5.4),
                    velocity: Some(0.0),
                    settle: Settle::Timed(MotionToken {
                        duration_ms,
                        ease: Ease::Linear,
                    }),
                },
                1_000,
            );
            // The frame that applied it is drawn for the same slot.
            s.advance(1_000);
            assert_eq!(
                s.animating(),
                frames_needed,
                "a settle of {duration_ms} ms, after the frame that carries it"
            );
        }
    }

    #[test]
    fn restyle_changes_a_layer_within_its_kind_and_delete_forgets_the_layer() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &Op::Create {
                id: LayerId(2),
                width: 10,
                height: 10,
                z: 0,
                content: Content::Image,
                props: Props::at(0.0, 0.0),
                image: None,
                mesh: None,
            },
            0,
        );
        let other = Shape {
            corner_radius: 40.0,
            ..shape()
        };
        s.apply(
            &Op::Restyle {
                id: LayerId(1),
                content: Content::Shape(other),
            },
            0,
        );
        // An image has no look to change, and a shape does not become an image.
        s.apply(
            &Op::Restyle {
                id: LayerId(2),
                content: Content::Shape(other),
            },
            0,
        );
        s.apply(
            &Op::Restyle {
                id: LayerId(1),
                content: Content::Image,
            },
            0,
        );
        let mut out = Vec::new();
        s.resolve(0, &mut out);
        assert_eq!(out[0].content, Content::Shape(other));
        assert_eq!(out[1].content, Content::Image);
        s.apply(&Op::Delete { id: LayerId(1) }, 0);
        s.apply(&Op::Delete { id: LayerId(1) }, 0);
        assert_eq!(ids(&s, 0), vec![2]);
        s.apply(&Op::Clear, 0);
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn changes_to_layers_that_are_not_there_are_ignored() {
        let mut s = scene();
        s.apply(
            &Op::Set {
                id: LayerId(9),
                props: Props::at(1.0, 1.0),
            },
            0,
        );
        s.apply(
            &Op::Animate { id: LayerId(9), to: Props::at(1.0, 1.0), duration_ms: 10, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        assert_eq!(s.len(), 0);
        assert!(!s.animating());
    }

    fn glass_content() -> Content {
        Content::Glass(compositor_protocol::Glass {
            corner_radius: 8.0,
            tint: Rgba(1, 1, 1, 1),
            edge: Rgba(0, 0, 0, 0),
            edge_width: 0.0,
            highlight: Rgba(0, 0, 0, 0),
            sheen: None,
            shadows: [None, None],
            depth: None,
            backdrop: compositor_protocol::Backdrop::None,
            saturation: 1.0,
            refraction: 0.0,
        })
    }

    fn create_content(id: u32, content: Content) -> Op {
        Op::Create {
            id: LayerId(id),
            width: 100,
            height: 50,
            z: 0,
            content,
            props: Props::at(0.0, 0.0),
            image: None,
            mesh: None,
        }
    }

    fn wallpaper(kind: WallpaperKind, fps: u8, speed: f32) -> Content {
        Content::Wallpaper(Wallpaper {
            kind,
            speed,
            intensity: 0.5,
            fps,
        })
    }

    fn versions(scene: &Scene, now: u64) -> Vec<(u32, u64)> {
        let mut out = Vec::new();
        scene.resolve(now, &mut out);
        out.iter().map(|d| (d.id.0, d.version)).collect()
    }

    #[test]
    fn a_producers_picture_gives_the_layers_that_show_its_source_a_new_version_and_no_other_layer()
    {
        use compositor_protocol::{Live, SourceId};
        let mut s = scene();
        let live = |source| {
            Content::Live(Live {
                source: SourceId(source),
            })
        };
        s.apply(&create_content(1, live(7)), 0);
        s.apply(&create_content(2, live(7)), 0);
        s.apply(&create_content(3, live(8)), 0);
        s.apply(&create(4, 0, Props::at(0.0, 0.0)), 0);
        let before = versions(&s, 0);
        let frame = Op::LiveFrame {
            source: SourceId(7),
            buffer: 1,
        };
        let after = |s: &Scene| versions(s, 0);
        s.apply(&frame, 0);
        let (now, then) = (after(&s), &before);
        assert_ne!(now[0].1, then[0].1);
        assert_ne!(now[1].1, then[1].1);
        assert_eq!(now[0].1, now[1].1, "the same change, the same version");
        assert_eq!(now[2], then[2], "another source's layer is not touched");
        assert_eq!(now[3], then[3], "nor is a shape");
        // A clear and a detach are changes of what the layers show as well.
        for op in [
            Op::LiveClear {
                source: SourceId(8),
            },
            Op::LiveDetach {
                source: SourceId(8),
            },
        ] {
            let was = after(&s);
            s.apply(&op, 0);
            let is = after(&s);
            assert_ne!(is[2].1, was[2].1, "{op:?}");
            assert_eq!(is[0], was[0]);
        }
        // A source nobody shows changes nothing.
        let was = after(&s);
        s.apply(
            &Op::LiveFrame {
                source: SourceId(99),
                buffer: 0,
            },
            0,
        );
        assert_eq!(after(&s), was);
    }

    #[test]
    fn a_layers_version_moves_when_it_changes_what_it_is_or_where_it_rests_and_not_otherwise() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(&create(2, 1, Props::at(0.0, 0.0)), 0);
        let after_create = versions(&s, 0);
        assert_ne!(after_create[0].1, after_create[1].1);
        let of = |s: &Scene, id: u32| versions(s, 0).into_iter().find(|v| v.0 == id).unwrap().1;
        let untouched = of(&s, 2);
        let rect = crate::geometry::PixRect::full(4, 4);
        // Damaging, restyling and setting layer 1 each give it a new version; layer 2 is not affected.
        let mut seen = vec![of(&s, 1)];
        for op in [
            Op::Damage {
                id: LayerId(1),
                rect,
            },
            Op::Restyle {
                id: LayerId(1),
                content: Content::Shape(shape()),
            },
            Op::Set {
                id: LayerId(1),
                props: Props::at(3.0, 3.0),
            },
        ] {
            s.apply(&op, 0);
            let now = of(&s, 1);
            assert!(!seen.contains(&now), "{op:?}");
            seen.push(now);
        }
        assert_eq!(of(&s, 2), untouched);
        // An animation does not change what a layer rests as until it lands.
        let before = of(&s, 1);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(80.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            10 * MS,
        );
        s.advance(60 * MS);
        assert_eq!(
            versions(&s, 60 * MS).iter().find(|v| v.0 == 1).unwrap().1,
            before
        );
        s.advance(110 * MS);
        assert_ne!(
            versions(&s, 110 * MS).iter().find(|v| v.0 == 1).unwrap().1,
            before
        );
        // A pointless animation, to where the layer is, changes nothing.
        let landed = of(&s, 1);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(80.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            200 * MS,
        );
        assert_eq!(of(&s, 1), landed);
    }

    #[test]
    fn a_drawn_layer_says_where_it_rests_while_an_animation_moves_it() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(10.0, 0.0)), 0);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(110.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        let mut out = Vec::new();
        s.resolve(50 * MS, &mut out);
        assert_eq!((out[0].props.x, out[0].rest.x), (60.0, 10.0));
        s.advance(100 * MS);
        s.resolve(100 * MS, &mut out);
        assert_eq!((out[0].props.x, out[0].rest.x), (110.0, 110.0));
    }

    #[test]
    fn a_wallpaper_asks_for_frames_at_its_own_rate_and_a_still_one_for_none() {
        let mut s = scene();
        s.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 30, 1.0)),
            0,
        );
        // Not started: the first frame that shows it starts its clock.
        assert_eq!(s.demand(), Demand::Idle);
        s.advance(1_000 * MS);
        assert_eq!(s.demand(), Demand::At(1_000 * MS + 2 * PERIOD));
        // Frames at every refresh: it steps on every second one and its phase follows.
        let mut out = Vec::new();
        let mut phases = Vec::new();
        for frame in 1..=4u64 {
            let now = 1_000 * MS + frame * PERIOD;
            s.advance(now);
            s.resolve(now, &mut out);
            phases.push(out[0].phase);
        }
        assert_eq!(phases[0], 0.0);
        assert!(phases[1] > 0.0 && phases[2] == phases[1] && phases[3] > phases[2]);
        // At the panel's own rate it wants every refresh; at fps 0 it wants nothing.
        let mut fast = scene();
        fast.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 60, 1.0)),
            0,
        );
        fast.advance(0);
        assert_eq!(fast.demand(), Demand::EveryRefresh);
        let mut still = scene();
        still.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 0, 1.0)),
            0,
        );
        still.advance(0);
        still.advance(10 * PERIOD);
        assert_eq!(still.demand(), Demand::Idle);
        // Restyled to move, it starts; restyled to fps 0, it stops where it is.
        still.apply(
            &Op::Restyle {
                id: LayerId(1),
                content: wallpaper(WallpaperKind::Starfield, 20, 1.0),
            },
            0,
        );
        still.advance(20 * PERIOD);
        assert_eq!(still.demand(), Demand::At(20 * PERIOD + 3 * PERIOD));
        still.apply(
            &Op::Restyle {
                id: LayerId(1),
                content: wallpaper(WallpaperKind::Starfield, 0, 1.0),
            },
            0,
        );
        still.advance(30 * PERIOD);
        assert_eq!(still.demand(), Demand::Idle);
    }

    #[test]
    fn a_hidden_wallpaper_asks_for_nothing() {
        let mut s = scene();
        s.apply(
            &create_content(1, wallpaper(WallpaperKind::Liquid, 30, 1.0)),
            0,
        );
        s.advance(0);
        assert!(matches!(s.demand(), Demand::At(_)));
        s.apply(
            &Op::Set {
                id: LayerId(1),
                props: Props {
                    visible: false,
                    ..Props::at(0.0, 0.0)
                },
            },
            0,
        );
        s.advance(PERIOD);
        assert_eq!(s.demand(), Demand::Idle);
    }

    #[test]
    fn a_wallpaper_or_a_mesh_nothing_of_which_shows_asks_for_no_frames_until_some_of_it_does() {
        let mut s = scene();
        s.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 60, 1.0)),
            0,
        );
        s.apply(
            &create_content(2, wallpaper(WallpaperKind::Liquid, 30, 1.0)),
            0,
        );
        s.apply(&create_content(3, mesh_content(30.0)), 0);
        s.advance(0);
        assert_eq!(s.demand(), Demand::EveryRefresh);
        // An opaque page covers all three, the renderer says: none is due a frame, though their clocks would go on.
        s.observe(&[LayerId(1), LayerId(2), LayerId(3)]);
        assert!(!s.animating());
        assert_eq!(s.demand(), Demand::Idle);
        // Every frame's account replaces the one before: as soon as any of a layer shows again it asks again.
        s.observe(&[LayerId(1), LayerId(3)]);
        assert_eq!(s.demand(), Demand::At(2 * PERIOD));
        s.observe(&[LayerId(1)]);
        assert!(s.animating(), "the mesh shows");
        s.observe(&[]);
        assert_eq!(s.demand(), Demand::EveryRefresh);
        // Layers that are gone are not a mistake.
        s.observe(&[LayerId(99)]);
        assert_eq!(s.demand(), Demand::EveryRefresh);
    }

    #[test]
    fn an_animation_asks_for_its_frames_whether_or_not_its_layer_shows() {
        // It may be moving into sight (from under a page), and only frames can say.
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &create_content(2, wallpaper(WallpaperKind::Starfield, 60, 1.0)),
            0,
        );
        s.advance(0);
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(50.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        s.observe(&[LayerId(1), LayerId(2)]);
        assert_eq!(s.demand(), Demand::EveryRefresh);
        s.advance(200 * MS);
        assert_eq!(
            s.demand(),
            Demand::Idle,
            "landed, and the wallpaper is covered"
        );
    }

    fn mesh_content(orbit: f32) -> Content {
        Content::Mesh(compositor_protocol::Mesh {
            eye: [1.0, 1.0, 1.0],
            target: [0.0; 3],
            fov_deg: 40.0,
            light: [0.0, 1.0, 0.0],
            ambient: 0.3,
            orbit_deg_per_s: orbit,
            antialias: false,
            background: Rgba(0, 0, 0, 0),
        })
    }

    #[test]
    fn an_orbiting_mesh_turns_by_the_time_that_passes_and_asks_for_every_refresh() {
        let mut s = scene();
        s.apply(&create_content(1, mesh_content(30.0)), 0);
        assert!(
            s.animating(),
            "an orbiting mesh needs a frame at every refresh"
        );
        s.advance(0);
        let mut out = Vec::new();
        s.advance(1_000 * MS);
        s.resolve(1_000 * MS, &mut out);
        // A stall does not make it jump: at most four refreshes of turning per frame.
        assert!(
            (out[0].phase - 30.0 * 4.0 * PERIOD as f64 / 1e9).abs() < 1e-6,
            "{}",
            out[0].phase
        );
        let before = out[0].phase;
        s.advance(1_000 * MS + PERIOD);
        s.resolve(1_000 * MS + PERIOD, &mut out);
        assert!((out[0].phase - before - 30.0 * PERIOD as f64 / 1e9).abs() < 1e-6);
        // A mesh that does not orbit needs nothing; a reversed orbit turns the other way.
        let mut still = scene();
        still.apply(&create_content(1, mesh_content(0.0)), 0);
        assert!(!still.animating());
        let mut back = scene();
        back.apply(&create_content(1, mesh_content(-30.0)), 0);
        back.advance(0);
        back.advance(PERIOD);
        back.resolve(PERIOD, &mut out);
        assert!(out[0].phase < 0.0);
        // Restyled to a different speed it carries on from where it is.
        s.apply(
            &Op::Restyle {
                id: LayerId(1),
                content: mesh_content(90.0),
            },
            0,
        );
        s.resolve(1_000 * MS + PERIOD, &mut out);
        assert!((out[0].phase - before - 30.0 * PERIOD as f64 / 1e9).abs() < 1e-6);
    }

    #[test]
    fn taps_on_a_liquid_wallpaper_age_with_it_run_out_and_are_only_a_handful() {
        let mut s = scene();
        s.apply(
            &create_content(1, wallpaper(WallpaperKind::Liquid, 60, 1.0)),
            0,
        );
        s.apply(
            &create_content(2, wallpaper(WallpaperKind::Starfield, 60, 1.0)),
            0,
        );
        s.advance(0);
        let tap = |x: f32| Op::Ripple {
            id: LayerId(1),
            x,
            y: 5.0,
        };
        s.apply(&tap(10.0), 0);
        // Only a Liquid wallpaper takes taps.
        s.apply(
            &Op::Ripple {
                id: LayerId(2),
                x: 1.0,
                y: 1.0,
            },
            0,
        );
        let mut out = Vec::new();
        s.resolve(0, &mut out);
        assert_eq!(out[0].ripples.live().len(), 1);
        assert_eq!(out[1].ripples.live().len(), 0);
        // It ages by the wallpaper's own time.
        for frame in 1..=60u64 {
            s.advance(frame * PERIOD);
        }
        s.resolve(60 * PERIOD, &mut out);
        let age = out[0].ripples.live()[0].age_s;
        assert!((age - 60.0 * PERIOD as f32 / 1e9).abs() < 0.01, "{age}");
        // Only the newest few live: the oldest goes when a seventh arrives.
        for n in 0..(MAX_RIPPLES + 2) {
            s.apply(&tap(100.0 + n as f32), 0);
        }
        s.resolve(60 * PERIOD, &mut out);
        let xs: Vec<f32> = out[0].ripples.live().iter().map(|r| r.x).collect();
        assert_eq!(xs.len(), MAX_RIPPLES);
        assert_eq!(xs[MAX_RIPPLES - 1], 100.0 + (MAX_RIPPLES + 1) as f32);
        assert!(!xs.contains(&10.0));
        // And they run out.
        let end = 60 + (wallpaper::RIPPLE_LIFE_S * 1e9 / PERIOD as f64).ceil() as u64 + 2;
        for frame in 61..=end {
            s.advance(frame * PERIOD);
        }
        s.resolve(end * PERIOD, &mut out);
        assert_eq!(out[0].ripples.live().len(), 0);
    }

    #[test]
    fn a_backdrop_relevant_change_is_not_made_by_a_wallpapers_motion_or_a_ripple() {
        let mut s = scene();
        s.apply(
            &create_content(1, wallpaper(WallpaperKind::Liquid, 60, 1.0)),
            0,
        );
        s.apply(&create_content(2, glass_content()), 0);
        s.advance(0);
        let v = versions(&s, 0);
        s.apply(
            &Op::Ripple {
                id: LayerId(1),
                x: 3.0,
                y: 3.0,
            },
            0,
        );
        for frame in 1..=30u64 {
            s.advance(frame * PERIOD);
        }
        assert_eq!(versions(&s, 30 * PERIOD), v);
    }

    #[test]
    fn a_client_that_goes_away_takes_the_glass_it_was_bringing_back_with_it() {
        // A frame for glass that is gone would draw an empty scene over the picture the next client has not replaced yet.
        for driven in [true, false] {
            let mut s = scene();
            s.apply(&create_content(1, glass_content()), 0);
            s.advance(40 * PERIOD);
            s.fade(true, driven);
            assert_ne!(s.demand(), Demand::Idle);
            s.apply(&Op::Clear, 41 * PERIOD);
            assert!(!s.is_fading());
            assert_eq!(s.demand(), Demand::Idle, "driven {driven}");
        }
    }

    #[test]
    fn glass_that_moved_under_a_finger_waits_for_the_next_commit_and_what_moved_by_itself_is_drawn_at_every_refresh()
     {
        let mut s = scene();
        s.apply(&create_content(1, glass_content()), 0);
        assert_eq!(s.demand(), Demand::Idle);
        // A frame that took what the app sent moved glass: the frame that brings it back is wanted 6 refreshes on, not at the next.
        let slot = 40 * PERIOD;
        s.advance(slot);
        s.fade(true, true);
        assert!(s.is_fading());
        assert_eq!(s.demand(), Demand::At(slot + 6 * PERIOD));
        // Another frame that moved glass puts it off again: the slot is that frame's.
        s.advance(slot + PERIOD);
        s.fade(true, true);
        assert_eq!(s.demand(), Demand::At(slot + 7 * PERIOD));
        // The last frame of an animation, or one that has more of a slow fade to draw: nothing is on its way, a frame at every refresh.
        s.advance(slot + 2 * PERIOD);
        s.fade(true, false);
        assert_eq!(s.demand(), Demand::EveryRefresh);
        // Done: nothing is wanted.
        s.fade(false, false);
        assert!(!s.is_fading());
        assert_eq!(s.demand(), Demand::Idle);
        // An animation, with or without glass in motion, is a frame at every refresh.
        s.apply(&create(2, 0, Props::at(0.0, 0.0)), 0);
        s.apply(
            &Op::Animate { id: LayerId(2), to: Props::at(50.0, 0.0), duration_ms: 300, ease: Ease::Out, step_ms: 0 },
            slot,
        );
        s.fade(true, true);
        assert_eq!(s.demand(), Demand::EveryRefresh);
    }

    #[test]
    fn the_ladder_slows_a_wallpaper_to_thirty_frames_then_holds_it_where_it_is_and_it_goes_on_from_there_when_let_go()
     {
        use crate::ladder::Ambient;
        let mut s = scene();
        s.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 60, 1.0)),
            0,
        );
        let mut out = Vec::new();
        let mut phase_at = |s: &mut Scene, at: u64| {
            s.advance(at);
            s.resolve(at, &mut out);
            out[0].phase
        };
        phase_at(&mut s, 0);
        assert_eq!(s.demand(), Demand::EveryRefresh, "as asked: every refresh");
        let before = phase_at(&mut s, PERIOD);
        assert!(before > 0.0);
        // At most 30: every second refresh, and the phase steps by two refreshes' time.
        s.set_ambient(Ambient::Half);
        assert_eq!(s.demand(), Demand::At(PERIOD + 2 * PERIOD));
        assert_eq!(
            phase_at(&mut s, 2 * PERIOD),
            before,
            "a refresh early: no step"
        );
        let half = phase_at(&mut s, 3 * PERIOD);
        assert!((half - before - 2.0 * PERIOD as f64 / 1e9).abs() < 1e-9);
        // Still: it asks for nothing, and stands.
        s.set_ambient(Ambient::Still);
        assert_eq!(s.demand(), Demand::Idle);
        let held = phase_at(&mut s, 400 * PERIOD);
        assert_eq!(held, half);
        // Let go of: the frame that does it (the engine draws one when the level changes) starts the clock again, and a wallpaper of
        // 60 asks for every refresh and goes on from where it stood, not from where the clock is.
        s.set_ambient(Ambient::Asked);
        assert_eq!(phase_at(&mut s, 401 * PERIOD), half);
        assert_eq!(s.demand(), Demand::EveryRefresh);
        let later = phase_at(&mut s, 402 * PERIOD);
        assert!((later - half - PERIOD as f64 / 1e9).abs() < 1e-9);
        // A wallpaper that asked for 20 frames is not made faster by Half, and one that asked for none stays still.
        let mut slow = scene();
        slow.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 20, 1.0)),
            0,
        );
        slow.advance(0);
        let before = slow.demand();
        slow.set_ambient(Ambient::Half);
        assert_eq!(slow.demand(), before);
    }

    #[test]
    fn the_ladder_stops_an_orbit_and_the_frames_it_asked_for_and_the_mesh_stands_at_the_angle_it_had()
     {
        use crate::ladder::Ambient;
        let mut s = scene();
        s.apply(&create_content(1, mesh_content(30.0)), 0);
        s.advance(0);
        assert!(s.animating());
        let mut out = Vec::new();
        s.advance(PERIOD);
        s.resolve(PERIOD, &mut out);
        let angle = out[0].phase;
        assert!(angle > 0.0);
        // Half does not touch an orbit; Still stops it, and with it the frames it wanted.
        s.set_ambient(Ambient::Half);
        assert!(s.animating());
        s.set_ambient(Ambient::Still);
        assert!(!s.animating());
        assert_eq!(s.demand(), Demand::Idle);
        s.advance(100 * PERIOD);
        s.resolve(100 * PERIOD, &mut out);
        assert_eq!(out[0].phase, angle);
        // And it turns again from there.
        s.set_ambient(Ambient::Asked);
        assert!(s.animating());
        s.advance(101 * PERIOD);
        s.advance(102 * PERIOD);
        s.resolve(102 * PERIOD, &mut out);
        assert!((out[0].phase - angle - 30.0 * PERIOD as f64 / 1e9).abs() < 1e-6);
    }

    #[test]
    fn a_layers_animation_is_motion_until_it_is_over_and_the_ambient_motion_is_none_of_a_persons() {
        let mut s = scene();
        s.apply(&create(1, 0, Props::at(0.0, 0.0)), 0);
        assert!(!s.moving_after(0));
        s.apply(
            &Op::Animate { id: LayerId(1), to: Props::at(50.0, 0.0), duration_ms: 100, ease: Ease::Linear, step_ms: 0 },
            0,
        );
        assert!(s.moving_after(50 * MS), "half way");
        assert!(s.moving_after(99 * MS));
        assert!(
            !s.moving_after(100 * MS),
            "the frame that lands it ends the motion"
        );
        // A wallpaper that steps at every refresh and a mesh that orbits are motion nobody set going.
        let mut ambient = scene();
        ambient.apply(
            &create_content(1, wallpaper(WallpaperKind::Starfield, 60, 1.0)),
            0,
        );
        ambient.apply(&create_content(2, mesh_content(30.0)), 0);
        ambient.advance(0);
        assert_eq!(ambient.demand(), Demand::EveryRefresh);
        assert!(!ambient.moving_after(10 * PERIOD));
    }
}
