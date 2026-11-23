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
use compositor_protocol::{Ease, GroupPose, PoseBinding, PoseProperties, Props};
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
    /// The pass of the page's moving bar that is under way, if the page has one.
    sweep: Option<Pass>,
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
    fn set(&mut self, props: Props, animate: bool, ms: u32) {
        if animate { self.ease(props, ms, Ease::Out) } else { self.jump(props) }
    }
    /// One pass of a constant-speed move from `from` to `to`: the compositor runs a move once and holds its end. Nobody's gesture
    /// drives it, so it is drawn at most every 16 ms (60 a second on a 120 Hz panel): the soft edge moves under 10 px a frame.
    fn glide(&mut self, from: Props, to: Props, ms: u32) {
        if let Self::Surface(layer) = self {
            layer.set(from);
            layer.animate_stepped(to, ms, Ease::Linear, 16);
        }
    }
}
/// The pass an indeterminate bar's segment is on. The next is asked for when this one has ended.
struct Pass {
    sweep: paint::Sweep,
    ends_ms: u64,
}
impl Pass {
    fn new(sweep: paint::Sweep) -> Self {
        Self { sweep, ends_ms: 0 }
    }
    /// The move to ask for at `now_ms`, when no pass is under way.
    fn start(&mut self, now_ms: u64) -> Option<(Props, Props, u32)> {
        (now_ms >= self.ends_ms).then(|| {
            self.ends_ms = now_ms + u64::from(self.sweep.ms);
            (self.sweep.from, self.sweep.to, self.sweep.ms)
        })
    }
    fn wait_ms(&self, now_ms: u64) -> u64 {
        self.ends_ms.saturating_sub(now_ms)
    }
}
impl GameUi {
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
            sweep: None,
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
    pub fn invalidate(&mut self) {
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
    pub fn art_busy(&self) -> bool {
        self.loader.busy() || self.art.busy() || self.unfinished_text || !self.ui.glyph_text_settled()
    }
    /// How long until the page has to be shown again with nothing else changed: the next pass of its moving bar.
    pub fn wake_ms(&self, now_ms: u64) -> Option<u64> {
        [self.sweep.as_ref().map(|pass| pass.wait_ms(now_ms)), self.leaving.wait_ms(now_ms)].into_iter().flatten().min()
    }
    /// Starts the next pass of the page's moving bar when the last has ended; true when one started.
    fn continue_sweep(&mut self, now_ms: u64) -> bool {
        let Some(pass) = &mut self.sweep else { return false };
        let Some(kept) = self.nodes.get_mut(&pass.sweep.id) else { return false };
        let Some((from, to, ms)) = pass.start(now_ms) else { return false };
        kept.layer.glide(from, to, ms);
        true
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
        !due.is_empty()
    }
    /// Drops the nodes the plan no longer has. One that arrived with a layer goes off the screen the way it came and is let go when that
    /// ends (`expire`); every other node goes at once.
    fn depart(&mut self, ids: &BTreeSet<u64>, now_ms: u64, reduced: bool) {
        self.nodes.retain(|id, kept| {
            if ids.contains(id) || self.leaving.contains(*id) {
                return true;
            }
            let Some(enter) = kept.enter else { return false };
            let go = motion::leave(kept.node.props, enter, reduced);
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
        if model.view.page == Page::Running && model.stream_bare() && !model.hud_visible() && model.idle_left().is_none() {
            let changed = !self.nodes.is_empty();
            self.nodes.clear();
            self.hits.clear();
            self.groups.clear();
            self.page = None;
            self.sweep = None;
            self.leaving.clear();
            self.revision = Some(model.revision());
            return Ok(changed);
        }
        let swept = self.continue_sweep(model.now_ms()) | self.expire(model.now_ms());
        let scrolling = self.groups.values().any(|g| g.motion.is_active());
        if self.revision == Some(model.revision()) && !decoded && !self.unfinished_text && self.ui.glyph_text_settled() && !scrolling {
            return Ok(swept || dipped);
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
        let sweep = plan.sweep;
        let crossing = sweep.is_some_and(|s| self.nodes.contains_key(&s.id));
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
                } else {
                    node.frame.y += spec.offset;
                    node.props.y += spec.offset;
                }
                parent = spec.parent;
            }
            self.keep(node, reduced, scrolled, group)?;
        }
        // A bar already crossing carries on through a re-plan that leaves it as it was; a new or changed one starts a pass of its own.
        self.sweep = match sweep {
            Some(next) if crossing && self.sweep.as_ref().is_some_and(|pass| pass.sweep == next) => self.sweep.take(),
            Some(next) => Some(Pass::new(next)),
            None => None,
        };
        self.continue_sweep(model.now_ms());
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
    fn keep(&mut self, node: Node, reduced: bool, scrolled: bool, group: Option<u32>) -> io::Result<()> {
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
                self.nodes.remove(&node.id);
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
        if recreate {
            self.nodes.remove(&node.id);
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
                    draw_qr(&mut layer, value)?;
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
            // A cover whose art arrives after its tile is shown fades in rather than popping; the stage art (below 100) is not a tile.
            if !reduced && node.id >= 100 && matches!(node.body, Body::Image(_)) {
                layer.set(Props { opacity: 0.0, ..node.props }, false, 0);
                layer.set(node.props, true, self.tokens.motion.focus_ms);
            } else {
                layer.set(node.props, false, 0);
            }
            if let Some(enter) = enter.filter(|_| fresh) {
                let come = motion::arrive(node.props, enter, reduced, self.tokens.motion.layer_ms);
                layer.jump(come.away);
                layer.ease(node.props, come.ms, come.ease);
            }
            self.nodes.insert(node.id, Kept { node, layer, group, enter });
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
                (Layer::Image(layer), Body::Qr(value)) => draw_qr(layer, value)?,
                (Layer::Image(layer), Body::Symbol(symbol, colour)) => symbols::draw(layer, *symbol, *colour),
                _ => unreachable!(),
            }
        }
        if self.leaving.stop(node.id) {
            let back = motion::arrive(node.props, enter.unwrap_or(Enter::FADE), reduced, self.tokens.motion.layer_ms);
            kept.layer.ease(node.props, back.ms, back.ease);
        }
        if kept.node.props != node.props {
            let animate = !reduced
                && (scrolled || !paint::LIVE.contains(&node.id))
                && (kept.node.props.scale != node.props.scale
                    || (kept.node.props.x - node.props.x).abs() < 100.0 && (kept.node.props.y - node.props.y).abs() < 100.0);
            let ms = self.tokens.motion.focus_ms;
            kept.layer.set(node.props, animate, ms);
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
fn draw_qr(layer: &mut ImageLayer, value: &str) -> io::Result<()> {
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
        Page::Game => game::hub(&mut p),
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
    use super::{Pass, paint::Sweep, scroll_settle};
    use compositor_protocol::{
        Props,
        motion::{MotionBounds, Track, Units},
    };

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

    #[test]
    fn a_bar_segment_starts_its_next_pass_when_the_last_ends_and_the_shell_wakes_for_it() {
        let sweep = Sweep { id: 1, from: Props::at(0.0, 0.0), to: Props::at(100.0, 0.0), ms: 1800 };
        let mut pass = Pass::new(sweep);
        assert!(pass.start(5000).is_some(), "the first pass starts at once");
        assert!(pass.start(5000 + 1799).is_none() && pass.wait_ms(5000 + 1799) == 1, "not before the pass has ended");
        assert!(pass.start(5000 + 1800).is_some() && pass.wait_ms(5000 + 1800) == 1800, "and then again, with a full pass to wait for");
    }
}
