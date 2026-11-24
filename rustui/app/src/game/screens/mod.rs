//! Native, retained LumaGame surfaces. Plans change on input or completed artwork, never on an idle video frame.
mod accounts;
mod art;
mod game;
mod glyph;
mod home;
mod lab;
mod launch;
mod library;
mod idle;
pub(super) mod paint;
pub(super) mod parts;
mod quick;
mod settings;
mod symbols;
mod stats;

use crate::game::model::{Focus, Model, Page};
use art::Key;
use compositor_protocol::{Ease, GroupPose, PoseBinding, PoseProperties, Props, Rect};
use lumagame_meta::Image;
use lumatv_tokens::{Tokens, TypeRole};
use paint::{Body, Node, Paint};
use parts::motion::{self, Enter, Leaving};
#[cfg(test)]
pub(super) use paint::HUD;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};
use ui::{GlassLayer, ImageLayer};
use ui::{
    Ui,
    components::{Item, Mark, Run, TextLayer},
    geometry::Frame,
    group::Group,
    motion::{Motion, MotionBounds, MotionToken, Settle, SpringToken, Units},
    lucent::palette::Mode,
};

pub struct GameUi {
    pub ui: Ui,
    host: Tokens,
    tokens: Tokens,
    appearance: Option<(String, bool, u16, bool)>,
    revision: Option<u64>,
    nodes: BTreeMap<u64, Kept>,
    hits: Vec<(Focus, Frame, Option<u32>)>,
    loader: art::Loader,
    art: art::Art,
    pending: BTreeSet<Key>,
    failed: BTreeSet<Key>,
    images: BTreeMap<Key, Cached>,
    bytes: usize,
    generation: u64,
    unfinished_text: bool,
    /// The page's scroll at the last plan: live nodes ease only while it changes, so they move with the page.
    shift: f32,
    page: Option<Page>,
    groups: BTreeMap<u32, Scroll>,
    /// The nodes the plan dropped that are still going off the screen with their layer, and when each is let go.
    leaving: Leaving,
    /// How the nodes of the last plan that arrive with a layer (`Paint::arrives`) come on and go off.
    enters: BTreeMap<u64, Enter>,
    /// The last plan was drawn on a page the screen already showed: a node of a layer that is new to it arrives, where on a page that has
    /// only just been cut in everything is simply there.
    settled: bool,
}
struct Cached {
    image: Image,
    used: u64,
}
struct Kept {
    node: Node,
    layer: Layer,
    group: Option<u32>,
    /// How the node came onto the screen with its layer, so it goes off the same way.
    enter: Option<Enter>,
    /// The art the node showed before it changed shape, while the change lands.
    other: Option<Other>,
}
struct Scroll {
    clip: Group,
    content: Group,
    motion: Motion,
    parent: Option<u32>,
    horizontal: bool,
    extent: f32,
    target: f32,
    clip_rect: compositor_protocol::Rect,
}
enum Layer {
    Surface(GlassLayer),
    Text(TextLayer),
    Image(ImageLayer),
}
impl Layer {
    fn jump(&mut self, props: Props) {
        match self {
            Self::Surface(layer) => layer.set(props),
            Self::Text(layer) => layer.set_props(props),
            Self::Image(layer) => layer.set(props),
        }
    }
    fn ease(&mut self, props: Props, ms: u32, ease: Ease) {
        match self {
            Self::Surface(layer) => layer.animate(props, ms, ease),
            Self::Text(layer) => layer.animate_props(props, ms, ease),
            Self::Image(layer) => layer.animate(props, ms, ease),
        }
    }
    /// Moves to `props` on the spring (retargeted from where the layer is), or at once.
    fn set(&mut self, props: Props, animate: bool, ms: u32) {
        if animate { self.ease(props, ms, Ease::Spring) } else { self.jump(props) }
    }
}
/// Whether `new` is `old` changed in shape in place: art that opted into a window (`home::tile`) and is a picture of another size.
fn reshaped(old: &Node, new: &Node) -> bool {
    matches!((&old.body, &new.body), (Body::Image(_), Body::Image(_)))
        && old.props.clip.is_some()
        && new.props.clip.is_some()
        && old.z == new.z
        && (old.frame.w != new.frame.w || old.frame.h != new.frame.h)
}
/// The share of the spring the old art's copy takes to dissolve (an `Ease::In` fade, so it fades late in that time): 75 ms of a 346 ms
/// spring, which ends past the spring's peak speed (at 1/7.25 of its time), where the box moves fastest.
const BLEND: f32 = 0.22;
/// How `node`'s picture lies to fill `window` the way a cover fills its box: scaled about its centre by the larger of the two ratios
/// and centred, so the crop keeps the middle of the art, and clipped to the window. Everything the compositor eases on the way from
/// one such pose to another (the window, the centre, the scale) is linear in the spring's progress, and the larger of two linear
/// ratios is never above the line between its ends, so the art covers the window all the way. The window's corners keep the radius the
/// node's own has (`Props::radius`), so they stay round while it grows from one shape to the other.
fn fill(node: &Node, window: Rect, opacity: f32) -> Props {
    let (w, h) = (node.frame.w, node.frame.h);
    let scale = (window.w / w).max(window.h / h);
    Props { scale, opacity, clip: Some(window), radius: node.props.radius, ..Props::at(window.x + (window.w - w) / 2.0, window.y + (window.h - h) / 2.0) }
}
/// The art a tile showed before it changed shape, still drawn under (or over) the tile's own while the morph lands.
struct Other {
    layer: Layer,
    node: Node,
    /// Drawn over the tile's own layer, which then is the opaque one and this the one that fades away; under it, the tile's own layer
    /// fades in over this one, which stays opaque.
    above: bool,
    /// When it is let go: the spring's end and a frame.
    until: u64,
}
impl Other {
    /// Sends it to fill `window`, the tile's new box, on the spring the tile's own layer is on.
    fn follow(&mut self, window: Rect, ms: u32, now_ms: u64) {
        self.layer.ease(fill(&self.node, window, if self.above { 0.0 } else { 1.0 }), ms, Ease::Spring);
        self.until = now_ms + u64::from(ms) + motion::LEAVE_SLACK_MS;
    }
}
impl GameUi {
    /// How long a spring-eased move takes: the theme's focus time stretched to the spring's own period.
    fn spring_ms(&self) -> u32 {
        motion::spring_ms(self.tokens.motion.focus_ms)
    }
    pub fn build(mut ui: Ui, _assets: &Path, land: &mut dyn FnMut(&mut Ui) -> io::Result<()>) -> io::Result<Self> {
        let host = Tokens::shipped();
        let tokens = lumatv_tokens::themes::game_tokens("rose-pine", Mode::Dark, &host).ok_or_else(|| io::Error::other("missing LumaGame default theme"))?;
        let mut types = Vec::new();
        for role in TypeRole::ALL {
            for scale in [1.0f32, 1.5, 2.0] {
                let mut ty = tokens.ty(role);
                ty.px = (ty.px as f32 * scale).round() as u32;
                types.push(ty);
            }
        }
        ui.warm(&types);
        land(&mut ui)?;
        Ok(Self {
            ui,
            host,
            tokens,
            appearance: None,
            revision: None,
            nodes: BTreeMap::new(),
            hits: Vec::new(),
            loader: art::Loader::new()?,
            art: art::Art::new()?,
            pending: BTreeSet::new(),
            failed: BTreeSet::new(),
            images: BTreeMap::new(),
            bytes: 0,
            generation: 0,
            unfinished_text: false,
            shift: 0.0,
            page: None,
            groups: BTreeMap::new(),
            leaving: Leaving::default(),
            enters: BTreeMap::new(),
            settled: false,
        })
    }
    pub fn set_host_tokens(&mut self, tokens: Tokens) {
        self.host = tokens;
        self.appearance = None;
        self.revision = None;
    }
    /// Redraws from scratch: nodes still on their way out go at once, since a fresh model's clock would never let them expire.
    pub fn invalidate(&mut self) {
        for id in self.leaving.ids().collect::<Vec<_>>() {
            self.nodes.remove(&id);
        }
        self.leaving.clear();
        self.revision = None;
        self.art.reset();
    }
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Focus> {
        self.hits.iter().rev().find(|(_, f, group)| {
            if let Some(group) = group.and_then(|id| self.groups.get(&id)) {
                group.content.hit(&Props::at(f.x, f.y), (f.w, f.h), x, y)
            } else {
                x >= f.x && x < f.x + f.w && y >= f.y && y < f.y + f.h
            }
        }).map(|(focus, _, _)| *focus)
    }
    /// Where `focus` was last drawn, on the screen.
    pub fn frame_of(&self, focus: Focus) -> Option<Frame> {
        self.hits.iter().find(|(f, _, _)| *f == focus).map(|(_, frame, _)| *frame)
    }
    pub fn art_busy(&self) -> bool {
        self.loader.busy() || self.art.busy() || self.unfinished_text || !self.ui.glyph_text_settled()
    }
    /// How long until the page has to be shown again with nothing else changed: the next node to finish leaving.
    pub fn wake_ms(&self, now_ms: u64) -> Option<u64> {
        let landing = self.nodes.values().filter_map(|kept| kept.other.as_ref()).map(|other| other.until.saturating_sub(now_ms)).min();
        landing.into_iter().chain(self.leaving.wait_ms(now_ms)).min()
    }
    /// Lets go of the nodes whose way off the screen has ended; true when any went. The next plan then reaches the groups they held.
    fn expire(&mut self, now_ms: u64) -> bool {
        let due = self.leaving.due(now_ms);
        for id in &due {
            self.nodes.remove(id);
        }
        if !due.is_empty() {
            self.revision = None;
        }
        let mut landed = false;
        for kept in self.nodes.values_mut() {
            if kept.other.as_ref().is_some_and(|other| other.until <= now_ms) {
                kept.other = None;
                landed = true;
            }
        }
        landed || !due.is_empty()
    }
    /// Drops the nodes the plan no longer has. One that arrived with a layer goes off the screen the way it came and is let go when that
    /// ends (`expire`); content and every other node go at once.
    fn depart(&mut self, ids: &BTreeSet<u64>, now_ms: u64, reduced: bool) {
        self.nodes.retain(|id, kept| {
            if ids.contains(id) || self.leaving.contains(*id) {
                return true;
            }
            let Some(go) = kept.enter.and_then(|enter| motion::leave(kept.node.props, enter, reduced)) else { return false };
            kept.layer.ease(go.away, go.ms, go.ease);
            self.leaving.begin(*id, now_ms, go.ms);
            true
        });
    }
    #[cfg(test)]
    pub(super) fn retained_counts(&self) -> (usize, usize) {
        (self.nodes.len(), self.groups.len())
    }
    pub fn show(&mut self, model: &Model) -> io::Result<bool> {
        let mut decoded = false;
        while let Some(done) = self.loader.poll() {
            self.pending.remove(&done.key);
            match done.image {
                Ok(image) => {
                    self.bytes += image.rgba.len();
                    if let Some(previous) = self.images.insert(done.key, Cached { image, used: self.generation }) {
                        self.bytes -= previous.image.rgba.len();
                    }
                    decoded = true;
                }
                Err(_) => {
                    self.failed.insert(done.key);
                }
            }
        }
        self.art.poll();
        let dipped = self.step_art(model)?;
        let bare_stream = model.view.page == Page::Running && model.stream_bare() && !model.hud_visible() && model.idle_left().is_none();
        // A full-screen player in the frame app sits on the video plane under this window: the page and its art leave the screen clear.
        if bare_stream || model.video_full {
            self.art.want_warm(Vec::new());
            if model.video_full {
                self.art.reset();
            }
            let changed = !self.nodes.is_empty() || (model.video_full && self.revision != Some(model.revision()));
            self.nodes.clear();
            self.hits.clear();
            self.groups.clear();
            self.page = None;
            self.leaving.clear();
            self.revision = Some(model.revision());
            return Ok(changed);
        }
        let expired = self.expire(model.now_ms());
        let scrolling = self.groups.values().any(|g| g.motion.is_active());
        if self.revision == Some(model.revision()) && !decoded && !self.unfinished_text && self.ui.glyph_text_settled() && !scrolling {
            return Ok(expired || dipped);
        }
        let appearance = (model.prefs.family.clone(), model.prefs.light, model.prefs.text_percent, model.prefs.reduce_motion);
        if self.appearance.as_ref() != Some(&appearance) {
            let mode = if model.prefs.family == lumatv_tokens::themes::MATCH_LUMATV {
                self.host.mode(None)
            } else if model.prefs.light {
                Mode::Light
            } else {
                Mode::Dark
            };
            self.tokens = lumatv_tokens::themes::game_tokens(&model.prefs.family, mode, &self.host).unwrap_or_else(|| self.host.clone());
            self.appearance = Some(appearance);
        }
        self.generation = self.generation.wrapping_add(1);
        let scroll_positions = if self.page == Some(model.view.page) {
            self.groups.iter().map(|(id, g)| (*id, g.motion.value())).collect()
        } else {
            BTreeMap::new()
        };
        let mut plan = plan_page(&self.tokens, model, &mut self.ui.fonts, scroll_positions);
        self.hits.clear();
        let scrolled = plan.shift != self.shift;
        self.shift = plan.shift;
        let cut = self.page != Some(model.view.page);
        if cut {
            self.nodes.clear();
            self.groups.clear();
            self.leaving.clear();
        }
        self.page = Some(model.view.page);
        let ids = plan.nodes.iter().map(|n| n.id).collect::<BTreeSet<_>>();
        debug_assert_eq!(ids.len(), plan.nodes.len(), "duplicate retained node on {:?}", model.view.page);
        let reduced = model.prefs.reduce_motion || self.tokens.motion.reduced;
        self.depart(&ids, model.now_ms(), reduced);
        let cleared = self.sync_groups(&plan.groups, reduced)?;
        self.settled = !cut && !cleared;
        for (focus, mut frame) in plan.hits.drain(..) {
            let group = plan.hit_groups.get(&focus).copied();
            let mut parent = group;
            while let Some(id) = parent {
                let spec = plan.groups.iter().find(|g| g.id == id).expect("hit group in plan");
                if spec.horizontal { frame.x += spec.offset; } else { frame.y += spec.offset; }
                parent = spec.parent;
            }
            self.hits.push((focus, frame, group));
        }
        let mut membership = BTreeMap::new();
        for group in &plan.groups {
            for &id in &group.nodes {
                membership.insert(id, group.id);
            }
        }
        self.unfinished_text = false;
        self.enters = std::mem::take(&mut plan.enters);
        for mut node in plan.nodes.drain(..) {
            let group = membership.get(&node.id).copied();
            let mut parent = group;
            while let Some(id) = parent {
                let spec = plan.groups.iter().find(|g| g.id == id).expect("scroll group in plan");
                if spec.horizontal {
                    node.frame.x += spec.offset;
                    node.props.x += spec.offset;
                    if let Some(clip) = &mut node.props.clip {
                        clip.x += spec.offset;
                    }
                } else {
                    node.frame.y += spec.offset;
                    node.props.y += spec.offset;
                    if let Some(clip) = &mut node.props.clip {
                        clip.y += spec.offset;
                    }
                }
                parent = spec.parent;
            }
            self.keep(node, reduced, scrolled, group, model.now_ms())?;
        }
        // The game page names none: it holds what Home or the Library had decoded, for the way back.
        if model.view.page != Page::Game {
            self.art.want_warm(std::mem::take(&mut plan.warm));
        }
        self.end_art(model)?;
        while self.bytes > 64 << 20 {
            let victim = self.images.iter().filter(|(_, c)| c.used != self.generation).min_by_key(|(_, c)| c.used).map(|(k, _)| k.clone());
            let Some(victim) = victim else {
                break;
            };
            if let Some(image) = self.images.remove(&victim) {
                self.bytes -= image.image.rgba.len();
            }
        }
        self.revision = Some(model.revision());
        Ok(true)
    }
    /// Makes the scroll groups the plan names; true when a group changed in a way that rebuilt every node.
    fn sync_groups(&mut self, specs: &[paint::ScrollGroup], reduced: bool) -> io::Result<bool> {
        let cleared = specs.iter().any(|spec| self.groups.get(&spec.id).is_some_and(|group| {
            group.parent != spec.parent || group.horizontal != spec.horizontal || group.extent != spec.extent.max(1.0)
        }));
        if cleared {
            self.nodes.clear();
            self.groups.clear();
            self.leaving.clear();
        }
        // A group stays while a node going off the screen is in it.
        self.groups.retain(|id, _| {
            specs.iter().any(|spec| spec.id == *id) || self.nodes.iter().any(|(node, kept)| kept.group == Some(*id) && self.leaving.contains(*node))
        });
        for spec in specs {
            let extent = spec.extent.max(1.0);
            if !self.groups.contains_key(&spec.id) {
                let parent = spec.parent.and_then(|id| self.groups.get(&id)).map(|g| g.content.id());
                let clip = self.ui.scene.group(parent, GroupPose::IDENTITY.clip_feathered(spec.clip, spec.feather))
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let content = self.ui.scene.group(Some(clip.id()), GroupPose::IDENTITY)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let motion = self.ui.scene.motion(spec.offset, MotionBounds::range(0.0, extent, Units::Pixels))
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let to = if spec.horizontal { GroupPose { x: -1.0, ..GroupPose::IDENTITY } } else { GroupPose { y: -1.0, ..GroupPose::IDENTITY } };
                content.bind(&motion, PoseBinding { values: [0.0, 1.0], from: GroupPose::IDENTITY, to, properties: PoseProperties::TRANSLATION })
                    .map_err(|e| io::Error::other(e.to_string()))?;
                self.groups.insert(spec.id, Scroll { clip, content, motion, parent: spec.parent, horizontal: spec.horizontal, extent, target: spec.offset, clip_rect: spec.clip });
            } else {
                let group = self.groups.get_mut(&spec.id).unwrap();
                if group.clip_rect != spec.clip {
                    group.clip.set_clip_feathered(Some(spec.clip), spec.feather);
                    group.clip_rect = spec.clip;
                }
                if group.target != spec.offset || reduced && group.motion.is_active() {
                    group.motion.settle(Some(spec.offset), None, scroll_settle(self.tokens.motion.scroll_ms, reduced))
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    group.target = spec.offset;
                }
            }
        }
        Ok(cleared)
    }
    /// The old art of a tile that changes shape, as a copy drawn over the tile's new layer: it follows the box for the first `BLEND` of
    /// the spring and dissolves on the way, so the two pictures overlap only while the box moves fastest. It is kept until the morph
    /// lands, hidden, so that a change back turns it round. None when the old picture is no longer held.
    fn dissolve(&mut self, old: Node, from: Rect, to: Rect, group: Option<u32>, ms: u32, now_ms: u64) -> io::Result<Option<Other>> {
        let Body::Image(key) = &old.body else { return Ok(None) };
        let Some(cached) = self.images.get(key) else { return Ok(None) };
        let image = &cached.image;
        let start = fill(&old, from, 1.0);
        let mut layer = self.ui.scene.image(image.width, image.height, old.z, start)?;
        layer.pixels_mut().copy_from_slice(&image.rgba);
        layer.damage(None);
        if let Some(id) = group {
            self.groups[&id].content.attach(&layer, start).map_err(|e| io::Error::other(e.to_string()))?;
        }
        let blend = (ms as f32 * BLEND).round() as u32;
        let at = compositor_protocol::motion::ease(Ease::Spring, blend as f32 / ms as f32);
        let lerp = |a: f32, b: f32| a + (b - a) * at;
        let reach = Rect { x: lerp(from.x, to.x), y: lerp(from.y, to.y), w: lerp(from.w, to.w), h: lerp(from.h, to.h) };
        let mut layer = Layer::Image(layer);
        layer.ease(fill(&old, reach, 0.0), blend, Ease::In);
        Ok(Some(Other { layer, node: old, above: true, until: now_ms + u64::from(ms) + motion::LEAVE_SLACK_MS }))
    }
    fn keep(&mut self, node: Node, reduced: bool, scrolled: bool, group: Option<u32>, now_ms: u64) -> io::Result<()> {
        if let Body::Image(key) = &node.body {
            if key.spec.stage.is_some() {
                self.keep_art(node);
                return Ok(());
            }
            if let Some(cached) = self.images.get_mut(key) {
                cached.used = self.generation;
            } else {
                if !self.pending.contains(key) && !self.failed.contains(key) && self.loader.request(key.clone(), false) {
                    self.pending.insert(key.clone());
                }
                // A tile that only changes shape keeps its old art on show until the new art has decoded.
                if !self.nodes.get(&node.id).is_some_and(|kept| reshaped(&kept.node, &node)) {
                    self.nodes.remove(&node.id);
                }
                return Ok(());
            }
        }
        let enter = self.enters.get(&node.id).copied();
        let fresh = self.settled && !self.nodes.contains_key(&node.id);
        let recreate = self.nodes.get(&node.id).is_none_or(|kept| {
            kept.group != group ||
            kept.node.frame.w != node.frame.w
                || kept.node.frame.h != node.frame.h
                || kept.node.z != node.z
                || std::mem::discriminant(&kept.node.body) != std::mem::discriminant(&node.body)
        });
        // Art that only changes shape in place (a cover becoming the banner and back) morphs: the old layer is let go by the new one.
        let morph = self.nodes.get(&node.id).filter(|kept| !reduced && self.settled && kept.group == group && reshaped(&kept.node, &node)).and_then(|kept| kept.node.props.clip);
        // Changing back while the morph runs turns it round: the art that was leaving comes back as the tile's own, and the layers keep
        // going from where they are, so nothing jumps.
        if !reduced && self.settled {
            let ms = self.spring_ms();
            if let Some(kept) = self.nodes.get_mut(&node.id).filter(|kept| kept.group == group && kept.other.as_ref().is_some_and(|other| other.node.body == node.body && other.node.z == node.z)) {
                let other = kept.other.as_mut().unwrap();
                std::mem::swap(&mut kept.layer, &mut other.layer);
                std::mem::swap(&mut kept.node, &mut other.node);
                other.above = !other.above;
                kept.layer.ease(node.props, ms, Ease::Spring);
                if let Some(window) = node.props.clip {
                    other.follow(window, ms, now_ms);
                }
                kept.enter = enter;
                kept.node = node;
                return Ok(());
            }
        }
        if recreate {
            let old = self.nodes.remove(&node.id);
            let mut layer = match &node.body {
                Body::Surface(glass) => Layer::Surface(self.ui.scene.glass(node.frame.w.ceil() as u32, node.frame.h.ceil() as u32, node.z, *glass, node.props)),
                Body::Text { words, marks } => {
                    let items = items(words, marks);
                    let Some(layer) = TextLayer::new_when_ready(&mut self.ui, node.frame, node.z, &items)? else {
                        self.unfinished_text = true;
                        return Ok(());
                    };
                    Layer::Text(layer)
                }
                Body::Image(key) => {
                    let image = &self.images[key].image;
                    let mut layer = self.ui.scene.image(image.width, image.height, node.z, node.props)?;
                    layer.pixels_mut().copy_from_slice(&image.rgba);
                    layer.damage(None);
                    Layer::Image(layer)
                }
                Body::Qr(value) => {
                    let mut layer = self.ui.scene.image(node.frame.w as u32, node.frame.h as u32, node.z, node.props)?;
                    draw_qr(&mut layer, value, self.tokens.radius(lumatv_tokens::Radius::Card) as u32)?;
                    Layer::Image(layer)
                }
                Body::Symbol(symbol, colour) => {
                    let mut layer = self.ui.scene.image(node.frame.w as u32, node.frame.h as u32, node.z, node.props)?;
                    symbols::draw(&mut layer, *symbol, *colour);
                    Layer::Image(layer)
                }
            };
            if let Some(id) = group {
                let parent = &self.groups[&id].content;
                let result = match &layer {
                    Layer::Surface(layer) => parent.attach(layer, node.props),
                    Layer::Text(layer) => parent.attach(layer.layer(), node.props),
                    Layer::Image(layer) => parent.attach(layer, node.props),
                };
                result.map_err(|e| io::Error::other(e.to_string()))?;
            }
            // A tile that changed shape in place morphs: its new art starts as the old box's crop and the old art, a copy drawn over it,
            // goes on to the new box's while it dissolves in the first part of the spring, where the box moves fastest. A cover whose
            // art arrives after its tile is shown fades in rather than popping; the stage art (below 100) is not a tile.
            let mut other = None;
            if let (Some(window), Some(old)) = (morph, old) {
                let ms = self.spring_ms();
                layer.jump(fill(&node, window, 1.0));
                layer.ease(node.props, ms, Ease::Spring);
                if let Some(target) = node.props.clip {
                    other = self.dissolve(old.node, window, target, group, ms, now_ms)?;
                }
            } else if !reduced && node.id >= 100 && matches!(node.body, Body::Image(_)) {
                layer.set(Props { opacity: 0.0, ..node.props }, false, 0);
                layer.ease(node.props, self.tokens.motion.focus_ms, Ease::Out);
            } else {
                layer.set(node.props, false, 0);
            }
            if let Some(enter) = enter.filter(|_| fresh) {
                let come = motion::arrive(node.props, enter, reduced, self.tokens.motion.layer_ms);
                layer.jump(come.away);
                layer.ease(node.props, come.ms, come.ease);
            }
            self.nodes.insert(node.id, Kept { node, layer, group, enter, other });
            return Ok(());
        }
        let kept = self.nodes.get_mut(&node.id).unwrap();
        if kept.node.body != node.body {
            match (&mut kept.layer, &node.body) {
                (Layer::Surface(layer), Body::Surface(glass)) => layer.restyle(*glass),
                (Layer::Text(layer), Body::Text { words, marks }) => layer.draw(&mut self.ui, &items(words, marks)),
                (Layer::Image(layer), Body::Image(key)) => {
                    let image = &self.images[key].image;
                    layer.pixels_mut().copy_from_slice(&image.rgba);
                    layer.damage(None);
                }
                (Layer::Image(layer), Body::Qr(value)) => draw_qr(layer, value, self.tokens.radius(lumatv_tokens::Radius::Card) as u32)?,
                (Layer::Image(layer), Body::Symbol(symbol, colour)) => symbols::draw(layer, *symbol, *colour),
                _ => unreachable!(),
            }
        }
        if self.leaving.stop(node.id) {
            let back = motion::arrive(node.props, enter.unwrap_or(Enter::FADE), reduced, self.tokens.motion.layer_ms);
            kept.layer.ease(node.props, back.ms, back.ease);
        }
        if kept.node.props != node.props {
            // A tile whose old art is still going keeps its move on the spring, however far the box has to go.
            let animate = !reduced
                && (kept.other.is_some()
                    || (scrolled || !paint::LIVE.contains(&node.id))
                        && (kept.node.props.scale != node.props.scale
                            || (kept.node.props.x - node.props.x).abs() < 100.0 && (kept.node.props.y - node.props.y).abs() < 100.0));
            let ms = motion::spring_ms(self.tokens.motion.focus_ms);
            kept.layer.set(node.props, animate, ms);
            if let (Some(other), Some(window), true) = (&mut kept.other, node.props.clip, animate) {
                other.follow(window, ms, now_ms);
            }
        }
        kept.enter = enter;
        kept.node = node;
        Ok(())
    }
}
/// How a scroll group moves to a new offset: the theme's critically damped spring (`motion.scrollMs` is its period), which a retarget
/// continues at the speed the group has, so a held key glides on without restarting. A timed curve restarted from rest at each press,
/// at the curve's own opening speed, was the abrupt scroll. Reduced motion moves at once.
fn scroll_settle(scroll_ms: u32, reduced: bool) -> Settle {
    if reduced {
        Settle::Timed(MotionToken { duration_ms: 0, ease: Ease::Out })
    } else {
        Settle::CriticalSpring(SpringToken { response_ms: (scroll_ms as f32).clamp(10.0, 5000.0) })
    }
}
fn items<'a>(words: &'a [paint::Word], marks: &[paint::Mark]) -> Vec<Item<'a>> {
    words
        .iter()
        .map(|w| Item::Text(Run { text: &w.text, ty: w.ty, colour: w.colour, x: w.x, y: w.y, w: w.w, align: w.align }))
        .chain(marks.iter().map(|m| Item::Mark(Mark { form: m.form, colour: m.colour, x: m.x as i32, y: m.y as i32 })))
        .collect()
}
/// The QR code on a white plate with the card's corners (`.qr` in the mock).
fn draw_qr(layer: &mut ImageLayer, value: &str, radius: u32) -> io::Result<()> {
    let qr = qrcodegen::QrCode::encode_text(value, qrcodegen::QrCodeEcc::Medium).map_err(io::Error::other)?;
    let modules = qr.size() + 8;
    let side = layer.width().min(layer.height()) as i32;
    let scale = (side / modules).max(1);
    let offset = (side - modules * scale) / 2;
    let width = layer.width() as usize;
    layer.pixels_mut().fill(255);
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            if !qr.get_module(x, y) {
                continue;
            }
            for yy in 0..scale {
                for xx in 0..scale {
                    let px = offset + (x + 4) * scale + xx;
                    let py = offset + (y + 4) * scale + yy;
                    if px >= 0 && py >= 0 && px < side && py < side {
                        let index = (py as usize * width + px as usize) * 4;
                        layer.pixels_mut()[index..index + 4].copy_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
        }
    }
    let (width, height) = (layer.width(), layer.height());
    art::round(layer.pixels_mut(), width, height, radius);
    layer.damage(None);
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// What the parts gallery has `plan_page` paint in place of the page (`shots::foundation`).
    static GALLERY: std::cell::Cell<Option<fn(&mut Paint<'_>)>> = const { std::cell::Cell::new(None) };
}
/// Has `plan_page` paint `painter` in place of the page, or the page again with `None`.
#[cfg(test)]
pub(super) fn gallery(painter: Option<fn(&mut Paint<'_>)>) {
    GALLERY.with(|g| g.set(painter));
}

/// Everything the page at `model.view.page` puts on screen, before layers are made for it.
fn plan_page(tokens: &Tokens, model: &Model, fonts: &mut ui::Fonts, scroll_positions: BTreeMap<u32, f32>) -> paint::Plan {
    let mut p = Paint::new(tokens, model, fonts);
    p.scroll_positions = scroll_positions;
    #[cfg(test)]
    if let Some(gallery) = GALLERY.with(|g| g.get()) {
        gallery(&mut p);
        p.finish();
        return p.plan;
    }
    let view = &model.view;
    if view.page != Page::Running { parts::stage::canvas(&mut p, false); }
    match view.page {
        Page::Home => home::home(&mut p),
        Page::Library => library::library(&mut p, view.search),
        Page::Game if view.top() == Some(crate::game::model::Layer::Viewer) => game::media::media(&mut p),
        Page::Game => game::page(&mut p),
        Page::Settings => settings::settings(&mut p),
        Page::Launch => launch::launch(&mut p),
        Page::Running => launch::running(&mut p),
    }
    p.finish();
    idle::show(&mut p);
    p.fit_focus();
    let sheet_start = p.plan.nodes.len();
    if view.top().is_some_and(crate::game::model::Layer::is_sheet) {
        p.plan.hits.clear();
    }
    parts::layer::sheet(&mut p);
    p.finish();
    for node in &mut p.plan.nodes[sheet_start..] {
        node.z += 100;
    }
    p.plan
}
/// The ids and frames of what the page would put on screen under the shipped theme.
#[cfg(test)]
pub(super) fn planned(model: &Model) -> Vec<(u64, ui::geometry::Frame)> {
    planned_with_opacity(model).into_iter().map(|(id, frame, _)| (id, frame)).collect()
}
/// `planned`, with how opaque each node is drawn.
#[cfg(test)]
pub(super) fn planned_with_opacity(model: &Model) -> Vec<(u64, ui::geometry::Frame, f32)> {
    plan_in(model, ui::lucent::palette::Mode::Dark).nodes.iter().map(|n| (n.id, n.frame, n.props.opacity)).collect()
}
/// The page's plan under the shipped theme in `mode`.
#[cfg(test)]
pub(super) fn plan_in(model: &Model, mode: ui::lucent::palette::Mode) -> paint::Plan {
    let host = Tokens::shipped();
    let tokens = lumatv_tokens::themes::game_tokens(&model.prefs.family, mode, &host).unwrap_or(host);
    let mut fonts = ui::Fonts::new();
    plan_page(&tokens, model, &mut fonts, BTreeMap::new())
}
#[cfg(test)]
mod tests {
    use super::scroll_settle;
    use super::{Body, Node, fill};
    use compositor_protocol::{Props, Rect};
    use compositor_protocol::motion::{MotionBounds, Track, Units};
    use ui::geometry::Frame;

    fn art(w: f32, h: f32) -> Node {
        Node { id: 1, frame: Frame::new(0.0, 0.0, w, h), props: Props::at(0.0, 0.0), z: 10, body: Body::Qr(String::new()) }
    }
    fn window(x: f32, w: f32) -> Rect {
        Rect { x, y: 0.0, w, h: 300.0 }
    }

    #[test]
    fn art_fills_its_window_centred_by_the_larger_ratio_all_the_way_through_a_morph() {
        let (cover, banner) = (art(200.0, 300.0), art(640.0, 300.0));
        let grown = fill(&cover, window(0.0, 640.0), 1.0);
        assert_eq!((grown.scale, grown.x + 100.0), (3.2, 320.0), "a cover in the banner's box: scaled to its width, on its centre");
        let kept = fill(&banner, window(0.0, 200.0), 1.0);
        assert_eq!((kept.scale, kept.x + 320.0), (1.0, 100.0), "the banner in a cover's box: cropped to the middle");
        // The compositor eases the window, the centre and the scale in a straight line; the art never leaves the window bare.
        let (from, to) = (window(500.0, 200.0), window(60.0, 640.0));
        let (a, b) = (fill(&cover, from, 1.0), fill(&cover, to, 1.0));
        for t in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let at = |a: f32, b: f32| a + (b - a) * t;
            let (x, w, scale) = (at(from.x, to.x), at(from.w, to.w), at(a.scale, b.scale));
            let (left, width) = (at(a.x, b.x) + 100.0 - 100.0 * scale, 200.0 * scale);
            assert!(left <= x + 1e-3 && left + width >= x + w - 1e-3, "t {t}: art {left}..{} covers {x}..{}", left + width, x + w);
        }
    }

    /// One refresh at 120 Hz.
    const FRAME_NS: u64 = 8_333_333;

    struct Run {
        track: Track,
        now: u64,
        seen: Vec<f32>,
    }

    impl Run {
        fn new(from: f32) -> Self {
            Self { track: Track::new(from, MotionBounds::range(0.0, 20_000.0, Units::Pixels)), now: 0, seen: vec![from] }
        }
        fn go(&mut self, to: f32, scroll_ms: u32, reduced: bool) {
            self.track.settle(self.now, Some(to), None, scroll_settle(scroll_ms, reduced));
        }
        fn frames(&mut self, n: usize) {
            for _ in 0..n {
                self.now += FRAME_NS;
                self.track.advance(self.now);
                self.seen.push(self.track.value());
            }
        }
    }

    /// The offsets never go back, no refresh moves them more than `step` of the way they have to go in all, and the step itself changes by
    /// no more than `jerk` of it from one refresh to the next (a curve that restarts from rest changes it by several times that).
    fn assert_glides(seen: &[f32], travel: f32, step: f32, jerk: f32) {
        let steps: Vec<f32> = seen.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.iter().all(|s| *s >= 0.0), "offsets went back: {steps:?}");
        let biggest = steps.iter().copied().fold(0.0, f32::max);
        assert!(biggest <= step * travel, "a refresh moved {biggest} px of {travel}");
        let worst = steps.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(worst <= jerk * travel, "the step changed by {worst} px between two refreshes of {travel}");
    }

    #[test]
    fn a_scroll_glides_from_rest_to_its_target_without_a_jump_at_120_hz() {
        let mut run = Run::new(0.0);
        run.go(600.0, 260, false);
        run.frames(80);
        assert_glides(&run.seen, 600.0, 0.08, 0.05);
        assert_eq!(*run.seen.last().unwrap(), 600.0);
        assert!(run.seen[1] < 20.0, "the first refresh already jumped {}", run.seen[1]);
    }

    #[test]
    fn a_retarget_in_mid_scroll_keeps_the_speed_it_has() {
        let mut run = Run::new(0.0);
        run.go(300.0, 260, false);
        run.frames(12);
        let (at, speed) = (run.track.value(), run.track.velocity());
        assert!(speed > 1000.0, "not moving yet: {speed}");
        run.go(600.0, 260, false);
        assert_eq!((run.track.value(), run.track.velocity()), (at, speed), "the retarget stopped or jumped the offset");
        run.frames(80);
        assert_glides(&run.seen, 600.0, 0.08, 0.05);
        assert_eq!(*run.seen.last().unwrap(), 600.0);
    }

    #[test]
    fn a_held_key_that_outruns_the_scroll_glides_on_instead_of_restarting_at_each_repeat() {
        let mut run = Run::new(0.0);
        // A repeat every 33 ms (4 refreshes), a row of 120 px each.
        for press in 1..=12 {
            run.go(120.0 * press as f32, 260, false);
            run.frames(4);
            if press > 1 {
                assert!(run.track.velocity() > 500.0, "stalled at repeat {press}: {}", run.track.velocity());
            }
        }
        run.frames(80);
        assert_glides(&run.seen, 1440.0, 0.05, 0.02);
        assert_eq!(*run.seen.last().unwrap(), 1440.0);
    }

    #[test]
    fn the_theme_sets_how_long_a_scroll_takes_and_reduced_motion_removes_it() {
        let reach = |ms| {
            let mut run = Run::new(0.0);
            run.go(600.0, ms, false);
            run.frames(12);
            run.track.value()
        };
        assert!(reach(520) < reach(260) && reach(260) < reach(130));
        let mut run = Run::new(0.0);
        run.go(600.0, 260, true);
        run.frames(1);
        assert_eq!(run.track.value(), 600.0);
    }
}
