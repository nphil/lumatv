//! The shell's draft field above the shared custom keyboard. Geometry and text metrics are retained between focus moves.

use super::entry::View;
use super::ime_surface::ImeSurface;
use super::life::Phase;
use super::ui::{WordLine, Words};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt};
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::edge;
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, Props, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Block, Style, plate};
use ui::geometry::Frame;
use ui::scene::ShapeLayer;
use ui::text::Weight;
use unicode_segmentation::UnicodeSegmentation;

pub const SCOPE: Scope = Scope(208);
const REGION: Region = Region(2600);
const BASE: u32 = 0x2600_0000;
const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target { Key(usize), Backdrop }
impl Target {
    pub fn id(self) -> Id { Id(BASE + match self { Self::Key(i) => i as u32, Self::Backdrop => 0xFF00 }) }
    pub fn from_id(id: Id) -> Option<Self> {
        match id.0.checked_sub(BASE)? { 0xFF00 => Some(Self::Backdrop), i if i < 64 => Some(Self::Key(i as usize)), _ => None }
    }
}

struct Header {
    generation: u64,
    scale: f32,
    prompt: String,
    pad: bool,
    height: f32,
    lines: Vec<String>,
    title_width: f32,
    hints_width: f32,
    field_height: f32,
}

impl Header {
    fn new(kit: &mut Kit, v: &View<'_>) -> Self {
        let pad = v.keyboard.layout.pad;
        let scale = kit.text_scale.clamp(1.0, 1.5);
        let width = if pad { (176.0 * scale).round().min(220.0) * 4.0 + 36.0 + 128.0 } else { 1920.0 };
        let ink = kit.ui.look.ink();
        let style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let hints_width = crate::panels::ui::hints_width(kit, &hints(), TypeRole::Label, kit.space(5)).ceil();
        let title_width = width - 2.0 * kit.space(7) - hints_width - kit.space(5);
        let lines = wrap::lines(kit, style, v.prompt, title_width, 2);
        let head = (lines.len() as f32 * kit.line(TypeRole::Title)).max(kit.tokens.target);
        let field_height = 84.0f32.max(kit.line(TypeRole::Title) + 2.0 * kit.space(3)).ceil();
        let height = head + kit.space(3) + field_height + kit.space(4);
        Self { generation: kit.generation(), scale: kit.text_scale, prompt: v.prompt.into(), pad, height, lines, title_width, hints_width, field_height }
    }
    fn matches(&self, kit: &Kit, v: &View<'_>) -> bool {
        self.generation == kit.generation() && self.scale == kit.text_scale && self.prompt == v.prompt && self.pad == v.keyboard.layout.pad
    }
}

fn hints() -> [Hint; 2] { [Hint::key(Key::Ok, Action::Confirm), Hint::key(Key::Back, Action::Close)] }

struct Built {
    _prompt: Words,
    _hints: Vec<HintRow>,
    _field: ShapeLayer,
    typed: Block,
    style: Style,
    at: Frame,
    field: Frame,
    selection: ShapeLayer,
    caret: ShapeLayer,
    caret_x: f32,
    caret_on: bool,
    revision: Option<u64>,
    chars: usize,
    secret: bool,
    selection_range: Option<(usize, usize)>,
    /// Only the visible excerpt is retained here; diagnostics never expose it.
    excerpt: String,
}

pub struct EntrySurface {
    keyboard: ImeSurface,
    header: Option<Header>,
    built: Option<Built>,
}

/// A caret-centred excerpt, measured only when the draft or caret changes, never when focus moves between keys.
pub(crate) fn excerpt(kit: &mut Kit, style: Style, text: &str, cursor: usize, room: f32) -> (String, usize, usize, f32) {
    let mut cuts: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    cuts.push(text.len());
    let caret = cuts.binary_search(&cursor).unwrap_or(0);
    let ellipsis = width_of(kit, style, "…");
    let left_room = (room - ellipsis * 2.0 - 6.0) * if cursor == text.len() { 1.0 } else { 0.75 };
    let (mut lo, mut hi) = (0, caret);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if width_of(kit, style, &text[cuts[mid]..cursor]) <= left_room { hi = mid; } else { lo = mid + 1; }
    }
    let start = cuts[lo];
    let prefix = if start > 0 { "…" } else { "" };
    let available = room - if start > 0 { ellipsis } else { 0.0 } - ellipsis - 6.0;
    let (mut a, mut b) = (caret, cuts.len() - 1);
    while a < b {
        let mid = (a + b).div_ceil(2);
        if width_of(kit, style, &text[start..cuts[mid]]) <= available { a = mid; } else { b = mid - 1; }
    }
    let end = cuts[a];
    let suffix = if end < text.len() { "…" } else { "" };
    let mut shown = String::with_capacity(prefix.len() + end - start + suffix.len());
    shown.push_str(prefix); shown.push_str(&text[start..end]); shown.push_str(suffix);
    let caret_x = width_of(kit, style, &text[start..cursor]) + if start > 0 { ellipsis } else { 0.0 };
    (shown, start, end, caret_x)
}

impl EntrySurface {
    pub fn new(z: i32) -> Self {
        Self { keyboard: ImeSurface::for_owner(z, SCOPE, REGION, BASE, Target::Backdrop.id()), header: None, built: None }
    }
    pub fn is_up(&self) -> bool { self.keyboard.is_up() }

    fn build(&self, kit: &mut Kit, header: &Header) -> std::io::Result<Built> {
        let (x, y, w, _) = self.keyboard.sheet().unwrap();
        let (x, y, w) = (x as f32, y as f32, w as f32);
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let z = self.keyboard.stage.z();
        let left = x + kit.space(7);
        let top = y + kit.space(5);
        let title_h = (header.lines.len() as f32 * kit.line(TypeRole::Title)).max(kit.tokens.target);
        let field = Frame::new(left, top + title_h + kit.space(3), w - 2.0 * kit.space(7), header.field_height);
        let at = Frame::new(field.x + kit.space(4), field.y + ((field.h - kit.line(TypeRole::Title)) / 2.0).round(), field.w - 2.0 * kit.space(4), kit.line(TypeRole::Title));
        let style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let scene = kit.ui.scene.clone();
        scene.adopting_view(self.keyboard.stage.group(), || -> std::io::Result<Built> {
            let lines: Vec<WordLine> = header.lines.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
            let prompt = Words::new(kit, left, top, header.title_width, z + 3, &lines)?;
            let mut rows = Vec::new();
            let mut hx = x + w - kit.space(7) - header.hints_width;
            for hint in hints() {
                let row = HintRow::new(kit, &hint, TypeRole::Label, RowAt { x: hx, y: top, h: title_h, z: z + 3 }, ink2)?;
                hx += row.width() + kit.space(5); rows.push(row);
            }
            let field_plate = plate::shape(&kit.ui, field, z + 2, Shape { corner_radius: kit.tokens.radius(Radius::Tile), fill: Fill::Solid(crate::kit::button::faded(ink, 0.06)), border_width: 1.0, border: edge(kit) });
            let typed = Block::new(&mut kit.ui, at, z + 5, vec![(Frame::new(0.0, 0.0, at.w, at.h), style, String::new())])?;
            let selection = plate::shape(&kit.ui, Frame::new(0.0, 0.0, at.w, at.h), z + 4, plate::solid(crate::kit::button::faded(kit.ui.look.accent(), 0.24), 0.0));
            selection.set(HIDDEN);
            let caret = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 3.0, at.h), z + 6, plate::solid(kit.ui.look.accent(), 1.5));
            caret.set(HIDDEN);
            Ok(Built { _prompt: prompt, _hints: rows, _field: field_plate, typed, style, at, field, selection, caret, caret_x: 0.0, caret_on: false, revision: None, chars: 0, secret: false, selection_range: None, excerpt: String::new() })
        })
    }

    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View<'_>, phase: Phase, now_ms: u64, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone { self.drop_all(engine); return Ok(false); }
        if self.header.as_ref().is_none_or(|h| !h.matches(kit, v)) {
            self.built = None;
            self.header = Some(Header::new(kit, v));
        }
        self.keyboard.header_height = self.header.as_ref().unwrap().height;
        let was = self.keyboard.sheet();
        self.keyboard.apply(kit, engine, &v.keyboard, phase, now_ns)?;
        if was != self.keyboard.sheet() { self.built = None; }
        if self.built.is_none() { self.built = Some(self.build(kit, self.header.as_ref().unwrap())?); }
        let b = self.built.as_mut().unwrap();
        if b.revision != Some(v.revision) {
            let (shown, start, end, caret_x) = excerpt(kit, b.style, v.shown, v.cursor, b.at.w);
            b.typed.set(&mut kit.ui, 0, &shown);
            b.excerpt = shown;
            b.caret_x = caret_x;
            b.selection_range = v.selection;
            if let Some((lo, hi)) = v.selection.filter(|(lo, hi)| *hi > start && *lo < end) {
                let prefix = if start > 0 { width_of(kit, b.style, "…") } else { 0.0 };
                let left = width_of(kit, b.style, &v.shown[start..lo.max(start).min(end)]) + prefix;
                let right = width_of(kit, b.style, &v.shown[start..hi.min(end).max(start)]) + prefix;
                b.selection.set(Props { clip: Some(compositor_protocol::Rect { x: b.at.x + left, y: b.at.y, w: right - left, h: b.at.h }), ..Props::at(b.at.x + left, b.at.y) });
            } else { b.selection.set(HIDDEN); }
            b.revision = Some(v.revision); b.chars = v.chars; b.secret = v.secret;
        }
        b.caret_on = Self::blink(now_ms);
        b.caret.set(if b.caret_on { Props::at(b.at.x + b.caret_x + 2.0, b.at.y) } else { HIDDEN });
        Ok(true)
    }

    fn blink(now: u64) -> bool { (now / 500) % 2 == 0 }
    pub fn due_at(&self, now: u64) -> Option<u64> { self.built.as_ref().map(|_| (now / 500 + 1) * 500) }
    pub fn tick(&self, now: u64) -> bool { self.built.as_ref().is_some_and(|b| b.caret_on != Self::blink(now)) }
    pub fn on_key(&mut self, engine: &mut Engine) { self.keyboard.on_key(engine); }
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now: u64, s: Sample) -> Option<Target> {
        self.keyboard.stage.pointer(kit, engine, now, s, |touch| match touch {
            super::stage::Touch::Hover { to, .. } => to.and_then(Target::from_id).filter(|t| matches!(t, Target::Key(_))).map(|_| kit.tokens.radius(Radius::Row)),
            _ => None,
        }).and_then(Target::from_id)
    }
    pub fn restyle(&mut self, engine: &mut Engine) { self.drop_all(engine); }
    pub fn drop_all(&mut self, engine: &mut Engine) { self.built = None; self.header = None; self.keyboard.drop_all(engine); }
    pub fn layers(&self) -> usize { self.keyboard.layers() + self.built.as_ref().map_or(0, |b| b._prompt.layers() + b._hints.len() * 2 + 4) }
    pub fn geometry(&self, engine: &Engine) -> Value {
        let mut g = self.keyboard.geometry(engine);
        if let Some(b) = &self.built {
            g["field"] = json!({"x": b.field.x, "y": b.field.y, "w": b.field.w, "h": b.field.h});
            g["chars"] = json!(b.chars); g["secret"] = json!(b.secret); g["selection"] = json!(b.selection_range);
            g["layers"] = json!(self.layers());
        }
        g
    }
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        self.keyboard.feed_lint(kit, lint);
        let Some(b) = &self.built else { return };
        b._prompt.feed_lint(kit, lint, "entry prompt", None, (0.0, 0.0));
        lint.word_on(kit, "entry field", &b.excerpt, b.at, b.style, None, (0.0, 0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::entry::Entry;
    use crate::panels::ime;
    use ui::lucent::palette::Mode;

    #[test]
    fn an_empty_field_types_and_edits_without_losing_focus_and_every_layout_fits_large_text() {
        for mode in [Mode::Dark, Mode::Light] { for scale in [1.0, 2.0] { for purpose in [ime::PURPOSE_NORMAL, ime::PURPOSE_URL, ime::PURPOSE_EMAIL, ime::PURPOSE_DIGITS, ime::PURPOSE_NUMBER, ime::PURPOSE_PHONE, ime::PURPOSE_DATE] {
            let (_h, mut kit) = testkit::kit_in(mode, scale); let mut engine = Engine::new(&kit).unwrap();
            let mut e = Entry::new(); e.open_purpose("Server address", "", false, purpose, 0); e.life.show(0);
            let mut surface = EntrySurface::new(crate::kit::z::KEYBOARD);
            surface.apply(&mut kit, &mut engine, &e.view(false), e.life.phase(), 500, 500_000_000).unwrap();
            let layers = kit.room().layers;
            e.key(crate::panels::K::Digit(5), 0);
            surface.apply(&mut kit, &mut engine, &e.view(false), e.life.phase(), 600, 600_000_000).unwrap();
            assert_eq!(kit.room().layers, layers, "empty-to-typed changes a retained field");
            assert_eq!(surface.geometry(&engine)["chars"], 1);
            assert!(surface.geometry(&engine)["sheet"]["y"].as_f64().unwrap() >= 54.0);
            assert!(surface.geometry(&engine).get("shown").is_none(), "diagnostics do not expose entry content");
        } } }
    }

    #[test]
    fn collapsing_selection_at_its_existing_end_removes_the_visible_highlight() {
        let (_h, mut kit) = testkit::kit(1.0); let mut engine = Engine::new(&kit).unwrap();
        let mut e = Entry::new(); e.open("Name", "abc", false, 0); e.life.show(0);
        let mut surface = EntrySurface::new(crate::kit::z::KEYBOARD);
        e.input("selectall", true, false, 0);
        surface.apply(&mut kit, &mut engine, &e.view(false), e.life.phase(), 500, 500_000_000).unwrap();
        assert_eq!(surface.geometry(&engine)["selection"], json!([0, 3]));
        let right = e.view(false).keyboard.layout.index_of("right").unwrap();
        e.click(right, 600);
        surface.apply(&mut kit, &mut engine, &e.view(false), e.life.phase(), 600, 600_000_000).unwrap();
        assert!(surface.geometry(&engine)["selection"].is_null());
    }
}
