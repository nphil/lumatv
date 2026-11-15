//! Native, retained LumaGame surfaces. Plans change on input or completed artwork, never on an idle video frame.
mod art;
mod pages;
mod paint;
mod settings;
mod sheets;
mod symbols;

use crate::game::model::{Focus, Model, Page};
use art::Key;
use compositor_protocol::{Ease, Props};
use lumagame_meta::Image;
use lumatv_tokens::{Tokens, TypeRole};
use paint::{Body, Node, Paint};
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
    lucent::palette::Mode,
};

pub struct GameUi {
    pub ui: Ui,
    host: Tokens,
    tokens: Tokens,
    appearance: Option<(String, bool, u16, bool)>,
    revision: Option<u64>,
    nodes: BTreeMap<u64, Kept>,
    hits: Vec<(Focus, Frame)>,
    loader: art::Loader,
    pending: BTreeSet<Key>,
    failed: BTreeSet<Key>,
    images: BTreeMap<Key, Cached>,
    bytes: usize,
    generation: u64,
    unfinished_text: bool,
}
struct Cached {
    image: Image,
    used: u64,
}
struct Kept {
    node: Node,
    layer: Layer,
}
enum Layer {
    Surface(GlassLayer),
    Text(TextLayer),
    Image(ImageLayer),
}
impl Layer {
    fn set(&mut self, props: Props, animate: bool, ms: u32) {
        match self {
            Self::Surface(layer) => {
                if animate {
                    layer.animate(props, ms, Ease::Out)
                } else {
                    layer.set(props)
                }
            }
            Self::Text(layer) => {
                if animate {
                    layer.animate_props(props, ms, Ease::Out)
                } else {
                    layer.set_props(props)
                }
            }
            Self::Image(layer) => {
                if animate {
                    layer.animate(props, ms, Ease::Out)
                } else {
                    layer.set(props)
                }
            }
        }
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
            pending: BTreeSet::new(),
            failed: BTreeSet::new(),
            images: BTreeMap::new(),
            bytes: 0,
            generation: 0,
            unfinished_text: false,
        })
    }
    pub fn set_host_tokens(&mut self, tokens: Tokens) {
        self.host = tokens;
        self.appearance = None;
        self.revision = None;
    }
    pub fn invalidate(&mut self) {
        self.revision = None;
    }
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Focus> {
        self.hits.iter().rev().find(|(_, f)| x >= f.x && x < f.x + f.w && y >= f.y && y < f.y + f.h).map(|(focus, _)| *focus)
    }
    pub fn art_busy(&self) -> bool {
        self.loader.busy() || self.unfinished_text || !self.ui.glyph_text_settled()
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
        if model.view.page == Page::Running {
            let changed = !self.nodes.is_empty();
            self.nodes.clear();
            self.hits.clear();
            self.revision = Some(model.revision());
            return Ok(changed);
        }
        if self.revision == Some(model.revision()) && !decoded && !self.unfinished_text && self.ui.glyph_text_settled() {
            return Ok(false);
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
        let mut p = Paint::new(&self.tokens, model, &mut self.ui.fonts);
        pages::canvas(&mut p);
        match model.view.page {
            Page::Home => pages::home(&mut p),
            Page::Library => pages::library(&mut p, false),
            Page::Search => pages::library(&mut p, true),
            Page::Hub => pages::hub(&mut p),
            Page::Settings => settings::settings(&mut p),
            Page::Integration => settings::integration(&mut p),
            Page::Launch => sheets::launch(&mut p),
            Page::Overlay => sheets::overlay(&mut p),
            Page::Media => sheets::media(&mut p),
            Page::Running => {}
        }
        p.fit_focus();
        let sheet_start = p.plan.nodes.len();
        if model.view.sheet.is_some() {
            p.plan.hits.clear();
        }
        sheets::sheet(&mut p);
        for node in &mut p.plan.nodes[sheet_start..] {
            node.z += 100;
        }
        let plan = p.plan;
        self.hits = plan.hits;
        let ids = plan.nodes.iter().map(|n| n.id).collect::<BTreeSet<_>>();
        debug_assert_eq!(ids.len(), plan.nodes.len(), "duplicate retained node on {:?}", model.view.page);
        self.nodes.retain(|id, _| ids.contains(id));
        self.unfinished_text = false;
        for node in plan.nodes {
            self.keep(node, model.prefs.reduce_motion || self.tokens.motion.reduced)?;
        }
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
    fn keep(&mut self, node: Node, reduced: bool) -> io::Result<()> {
        if let Body::Image(key) = &node.body {
            if let Some(cached) = self.images.get_mut(key) {
                cached.used = self.generation;
            } else {
                if !self.pending.contains(key) && !self.failed.contains(key) && self.loader.request(key.clone()) {
                    self.pending.insert(key.clone());
                }
                self.nodes.remove(&node.id);
                return Ok(());
            }
        }
        let recreate = self.nodes.get(&node.id).is_none_or(|kept| {
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
            layer.set(node.props, false, 0);
            self.nodes.insert(node.id, Kept { node, layer });
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
        if kept.node.props != node.props {
            let animate = !reduced
                && (kept.node.props.scale != node.props.scale
                    || (kept.node.props.x - node.props.x).abs() < 100.0 && (kept.node.props.y - node.props.y).abs() < 100.0);
            kept.layer.set(node.props, animate, self.tokens.motion.focus_ms);
        }
        kept.node = node;
        Ok(())
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
