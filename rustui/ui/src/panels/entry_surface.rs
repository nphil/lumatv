//! The entry sheet drawn: a glass sheet along the bottom with what is being asked ("Wi-Fi password"), the field with what has been typed (a
//! password as bullets, the last character apart) and the keyboard of Home's search under it (`home/search.rs`: the number row, three rows of
//! letters, shift and delete, `?123`, space and Done). The keys are one painted picture and their caps one block of words, so the whole
//! keyboard costs a handful of layers however many keys it has; the focus is one lit plate with a ring that glides from key to key.
//!
//! The model decides which key has the focus and what the field says; this follows it. The sheet is made again when the question, the look or
//! the symbols' layout (which changes every cap) says something else; typing, shift, the caret and the focus change in place.

use super::entry::{KEYS, View};
use super::life::Phase;
use super::stage::{Stage, Touch};
use super::ui::{Arrive, WordLine, Words, hints_width};
use crate::home::search::Search;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt, glyph_side};
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::picture::Picture;
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::{edge, lit_fill};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Block, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(208);
const REGION: Region = Region(2600);
const BASE: u32 = 0x2600_0000;
const SHEET_X: f32 = 64.0;
const SHEET_W: f32 = 1792.0;
const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
/// The icons of the shift and delete keys, before the sheet's scale.
const ICON: f32 = 28.0;
const SHIFT: &str = "\u{21e7}";
const ERASE: &str = "\u{232b}";
const ELLIPSIS: &str = "\u{2026}";

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Key(usize),
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        match self {
            Target::Key(i) => Id(BASE + i as u32),
            Target::Backdrop => Id(BASE + 0xFF00),
        }
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            0xFF00 => Some(Target::Backdrop),
            i if (i as usize) < KEYS => Some(Target::Key(i as usize)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    sheet: Frame,
    prompt: Vec<String>,
    prompt_at: (f32, f32, f32),
    hints: Frame,
    field: Frame,
    /// Where the typed words go, and how wide they may be.
    typed: Frame,
    grid: Frame,
    keys: Vec<Frame>,
    scale: f32,
}

fn hints() -> [Hint; 2] {
    [Hint::key(Key::Ok, Action::Confirm), Hint::key(Key::Back, Action::Close)]
}

fn layout(kit: &mut Kit, prompt: &str) -> Layout {
    let (s3, s4, s5, s6, s7) = (kit.space(3), kit.space(4), kit.space(5), kit.space(6), kit.space(7));
    let ink = kit.ui.look.ink();
    let title_line = kit.line(TypeRole::Title);
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let hs = hints();
    let hints_w = hints_width(kit, &hs, TypeRole::Label, kit.space(7)).ceil() + 2.0;
    let hints_h = kit.tokens.target.max(glyph_side(kit, TypeRole::Label) as f32 + s3).ceil();
    let prompt_w = SHEET_W - 2.0 * s7 - hints_w - s5;
    let lines = wrap::lines(kit, title_style, prompt, prompt_w, 2);
    let head_h = (lines.len() as f32 * title_line).max(hints_h);
    let field_h = 84.0f32.max(title_line + 2.0 * s4).ceil();
    // the keys grow with larger text, to a point, and give way when the sheet would not fit the screen
    let (unit_h, unit_w) = (4.0 * 82.0 + 72.0, 930.0);
    let fixed = s6 + head_h + s4 + field_h + s5 + s6;
    let fit = ((972.0 - 10.0 - fixed) / unit_h).max(0.5);
    let scale = kit.text_scale.clamp(1.0, 1.5).min(fit).min((SHEET_W - 2.0 * s6) / unit_w);
    let (grid_w, grid_h) = ((unit_w * scale).round(), (unit_h * scale).round());
    let sheet_h = (fixed + grid_h).ceil();
    let sheet = Frame::new(SHEET_X, 1080.0 - 64.0 - sheet_h, SHEET_W, sheet_h);
    let head_y = sheet.y + s6;
    let hints = Frame::new(sheet.right() - s7 - hints_w, head_y + ((head_h - hints_h) / 2.0).round(), hints_w, hints_h);
    let field = Frame::new(sheet.x + s7, head_y + head_h + s4, SHEET_W - 2.0 * s7, field_h);
    let grid = Frame::new(sheet.x + ((SHEET_W - grid_w) / 2.0).round(), field.bottom() + s5, grid_w, grid_h);
    let typed = Frame::new(field.x + s5, field.y + ((field_h - title_line) / 2.0).round(), field.w - 2.0 * s5, title_line);
    let keys = (0..KEYS)
        .map(|i| {
            let f = Search::key_frame(i);
            Frame::new(grid.x + (f.x - crate::home::search::KEYS_X) * scale, grid.y + (f.y - crate::home::search::KEYS_Y) * scale, (f.w * scale).round(), (f.h * scale).round())
        })
        .collect();
    Layout { sheet, prompt: lines, prompt_at: (sheet.x + s7, head_y, prompt_w), hints, field, typed, grid, keys, scale }
}

/// What of `shown` is drawn when the field is too narrow for all of it: the end, which is where the typing is, led by an ellipsis.
fn tail(kit: &mut Kit, style: Style, shown: &str, max_w: f32) -> String {
    if width_of(kit, style, shown) <= max_w {
        return shown.to_string();
    }
    let chars: Vec<char> = shown.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    // the fewest leading characters dropped for which the rest, with the ellipsis, fits
    while lo < hi {
        let mid = (lo + hi) / 2;
        let text: String = std::iter::once(ELLIPSIS.to_string()).chain(std::iter::once(chars[mid..].iter().collect())).collect();
        if width_of(kit, style, &text) <= max_w { hi = mid } else { lo = mid + 1 }
    }
    format!("{ELLIPSIS}{}", chars[lo..].iter().collect::<String>())
}

/// How a key is drawn: its class (the lit plate has the size of its class) and what it is.
fn class_of(i: usize) -> usize {
    match i {
        41 => 2,
        40 | 42 => 1,
        _ => 0,
    }
}

struct Lit {
    shapes: [ShapeLayer; 3],
    at: Option<(usize, usize)>,
}

impl Lit {
    fn new(kit: &Kit, lay: &Layout, z: i32) -> Self {
        let shape = Shape { corner_radius: kit.tokens.radius(Radius::Control), fill: Fill::Solid(lit_fill(kit)), border_width: 3.0, border: kit.ui.look.ink() };
        let make = |i: usize| plate::shape(&kit.ui, Frame::new(0.0, 0.0, lay.keys[i].w, lay.keys[i].h), z, shape);
        let shapes = [make(0), make(40), make(41)];
        for s in &shapes {
            s.set(HIDDEN);
        }
        Self { shapes, at: None }
    }

    /// The ring goes to key `i`: it glides from a key of its own size, and is there at once from another's.
    fn go(&mut self, kit: &Kit, lay: &Layout, i: usize) {
        let class = class_of(i);
        let f = lay.keys[i];
        let to = Props::at(f.x, f.y);
        match self.at {
            Some((c, _)) if c == class => kit.glide(&self.shapes[class], to, Beat::Focus),
            _ => {
                for (c, s) in self.shapes.iter().enumerate() {
                    s.set(if c == class { to } else { HIDDEN });
                }
            }
        }
        self.at = Some((class, i));
    }
}

struct Built {
    sig: String,
    _sheet: ShapeLayer,
    _prompt: Words,
    _hints: Vec<HintRow>,
    _field: ShapeLayer,
    typed: Words,
    typed_style: Style,
    shown: String,
    caret: ShapeLayer,
    caret_on: bool,
    _art: ImageLayer,
    caps: Block,
    caps_now: Vec<String>,
    lit: Lit,
    lay: Layout,
}

pub struct EntrySurface {
    stage: Stage,
    built: Option<Built>,
}

impl EntrySurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).hiding(), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, v: &View, lay: &Layout) -> String {
        format!("{}|{}|{}|{:?}", kit.generation(), kit.text_scale, v.prompt, lay.sheet)
    }

    /// The text of key `i` as it is drawn: a letter is a capital while shift is on.
    fn cap_text(v: &View, i: usize) -> String {
        let cap = v.caps[i];
        if cap == SHIFT || cap == ERASE {
            String::new()
        } else if v.shift && cap.chars().count() == 1 {
            cap.to_uppercase()
        } else {
            cap.to_string()
        }
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &ui::group::Group, z: i32, v: &View, lay: Layout, sig: String) -> std::io::Result<Built> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent_ink = kit.ui.look.accent_ink();
        let glass = kit.ui.look.colour(PaletteRole::Glass);
        let accent = kit.ui.look.accent();
        let radius = kit.tokens.radius(Radius::Control);
        let typed_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let caps_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
        let done_style = Style { colour: accent_ink, ..caps_style };
        scene
            .adopting_view(root, || -> std::io::Result<Built> {
                // a solid sheet over a dim that hides the screen (Settings, full of words): a password is typed against nothing else
                let sheet_look = Shape { corner_radius: kit.tokens.radius(Radius::Sheet), fill: Fill::Solid(kit.ui.look.colour(PaletteRole::Sheet)), border_width: 1.0, border: edge(kit) };
                let sheet = plate::shape(&kit.ui, lay.sheet, z + 1, sheet_look);
                let prompt_lines: Vec<WordLine> = lay.prompt.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
                let prompt = Words::new(kit, lay.prompt_at.0, lay.prompt_at.1, lay.prompt_at.2, z + 3, &prompt_lines)?;
                let mut hint_rows = Vec::new();
                let mut x = lay.hints.x;
                for h in hints() {
                    let row = HintRow::new(kit, &h, TypeRole::Label, RowAt { x, y: lay.hints.y, h: lay.hints.h, z: z + 3 }, ink2)?;
                    x += row.width() + kit.space(7);
                    hint_rows.push(row);
                }
                let field = plate::shape(
                    &kit.ui,
                    lay.field,
                    z + 2,
                    Shape { corner_radius: kit.tokens.radius(Radius::Tile), fill: Fill::Solid(crate::kit::button::faded(ink, 0.06)), border_width: 1.0, border: edge(kit) },
                );
                let shown = tail(kit, typed_style, &v.shown, lay.typed.w);
                let typed = Words::new(kit, lay.typed.x, lay.typed.y, lay.typed.w, z + 3, &[WordLine::new(&shown, TypeRole::Title, ink)])?;
                let caret = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 3.0, lay.typed.h), z + 3, plate::solid(accent, 1.5));
                caret.set(HIDDEN);
                // the keys' plates are one picture; the shift and delete keys carry icons, the characters are words
                let mut art = kit.ui.scene.image(lay.grid.w as u32, lay.grid.h as u32, z + 2, Props::at(lay.grid.x, lay.grid.y))?;
                {
                    let side = (ICON * lay.scale).round() as u32;
                    let icon = |kit: &mut Kit, name: &str, colour: Rgba| kit.icons.draw(name, side, colour).map(|rgba| Picture { width: side, height: side, rgba });
                    let (shift, erase) = (icon(kit, "arrow-big-up", ink), icon(kit, "delete", ink));
                    let (w, h) = (art.width(), art.height());
                    let mut canvas = crate::home::widgets::Canvas { px: art.pixels_mut(), w, h };
                    for (i, f) in lay.keys.iter().enumerate() {
                        let local = Frame::new(f.x - lay.grid.x, f.y - lay.grid.y, f.w, f.h);
                        canvas.rounded(local, radius, if i == 42 { accent } else { glass });
                        let pic = match v.caps[i] {
                            SHIFT => shift.as_ref(),
                            ERASE => erase.as_ref(),
                            _ => None,
                        };
                        if let Some(p) = pic {
                            canvas.picture(p, (local.x + (local.w - side as f32) / 2.0).round() as i32, (local.y + (local.h - side as f32) / 2.0).round() as i32, side, side);
                        }
                    }
                }
                art.damage(None);
                let lit = Lit::new(kit, &lay, z + 4);
                let caps_now: Vec<String> = (0..KEYS).map(|i| Self::cap_text(v, i)).collect();
                let lines = lay.keys.iter().enumerate().map(|(i, f)| (Frame::new(f.x - lay.grid.x, f.y - lay.grid.y, f.w, f.h), if i == 42 { done_style } else { caps_style }, caps_now[i].clone())).collect();
                let caps = Block::new(&mut kit.ui, lay.grid, z + 6, lines)?;
                Ok(Built {
                    sig,
                    _sheet: sheet,
                    _prompt: prompt,
                    _hints: hint_rows,
                    _field: field,
                    typed,
                    typed_style,
                    shown,
                    caret,
                    caret_on: false,
                    _art: art,
                    caps,
                    caps_now,
                    lit,
                    lay: lay.clone(),
                })
            })
            .map_err(|e| std::io::Error::other(format!("{e}")))
            .inspect(|_| {
                for (i, f) in lay.keys.iter().enumerate() {
                    engine.focus.register(Target::Key(i).id(), *f, REGION, SCOPE);
                }
            })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for i in 0..b.lay.keys.len() {
                engine.focus.unregister(Target::Key(i).id());
            }
        }
    }

    /// Fits the sheet to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ms: u64, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let lay = layout(kit, &v.prompt);
        let arrive = Arrive::Bottom(lay.sheet.h + 64.0);
        if self.stage.needs_root(arrive) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, arrive, true)?;
        let sig = Self::signature(kit, v, &lay);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), v, lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the entry sheet could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if let Some(b) = self.built.as_mut() {
            Self::update(kit, self.stage.group(), self.stage.z(), b, v, now_ms);
        }
        if self.stage.phase(kit, engine, phase) {
            self.stage.focus(engine, Some(Target::Key(v.index).id()), now_ns);
        }
        Ok(true)
    }

    /// What changes in place: the typed words, the caret, the caps of a new case or layout, and the ring.
    fn update(kit: &mut Kit, root: &ui::group::Group, z: i32, b: &mut Built, v: &View, now_ms: u64) {
        let shown = tail(kit, b.typed_style, &v.shown, b.lay.typed.w);
        if shown != b.shown {
            // one layer of words, made again: a line that began empty has no glyphs to change
            let ink = kit.ui.look.ink();
            let scene = kit.ui.scene.clone();
            let at = b.lay.typed;
            let made = scene.adopting_view(root, || -> std::io::Result<Words> { Words::new(kit, at.x, at.y, at.w, z + 3, &[WordLine::new(&shown, TypeRole::Title, ink)]) });
            if let Ok(words) = made {
                b.typed = words;
            }
            b.shown = shown;
        }
        let text_w = width_of(kit, b.typed_style, &b.shown);
        let caret_on = Self::blink(now_ms);
        b.caret_on = caret_on;
        b.caret.set(if caret_on { Props::at(b.lay.typed.x + text_w + 2.0, b.lay.typed.y) } else { HIDDEN });
        let ink = kit.ui.look.ink();
        let accent_ink = kit.ui.look.accent_ink();
        let now: Vec<String> = (0..KEYS).map(|i| Self::cap_text(v, i)).collect();
        if now != b.caps_now {
            let changes: Vec<(usize, &str, Rgba)> = now.iter().enumerate().map(|(i, t)| (i, t.as_str(), if i == 42 { accent_ink } else { ink })).collect();
            b.caps.set_many(&mut kit.ui, &changes);
            b.caps_now = now;
        }
        if b.lit.at.map(|(_, i)| i) != Some(v.index) {
            b.lit.go(kit, &b.lay, v.index);
        }
    }

    /// The caret is on for the first half of every second.
    fn blink(now_ms: u64) -> bool {
        (now_ms / 500) % 2 == 0
    }

    /// When the caret next changes (the sheet draws again then).
    pub fn due_at(&self, now_ms: u64) -> Option<u64> {
        self.built.as_ref().map(|_| (now_ms / 500 + 1) * 500)
    }

    /// Whether the caret turned since the last draw.
    pub fn tick(&self, now_ms: u64) -> bool {
        self.built.as_ref().is_some_and(|b| b.caret_on != Self::blink(now_ms))
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it over the keys, and a click is the key it ended on (the dim is Back).
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let radius = kit.tokens.radius(Radius::Control);
        let id = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { to, .. } => to.and_then(Target::from_id).filter(|t| matches!(t, Target::Key(_))).map(|_| radius),
            Touch::Press { .. } => None,
        })?;
        Target::from_id(id)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    /// How many layers the sheet holds.
    pub fn layers(&self) -> usize {
        let Some(b) = &self.built else { return 0 };
        // the scrim, the pane, the field, the caret, the keys' picture, the caps, the three lit plates, the hints, the words
        1 + 2 + 1 + 1 + 1 + 1 + 3 + b._hints.len() * 2 + b._prompt.layers() + b.typed.layers()
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        json!({
            "sheet": f(b.lay.sheet), "field": f(b.lay.field), "grid": f(b.lay.grid), "scale": b.lay.scale,
            "keys": b.lay.keys.iter().map(|k| f(*k)).collect::<Vec<_>>(),
            "caps": b.caps_now, "shown": b.shown,
            "focus": engine.focus.current().and_then(Target::from_id).and_then(|t| if let Target::Key(i) = t { Some(i) } else { None }),
            "layers": self.layers(), "scope": SCOPE.0,
        })
    }

    /// Describes what is on the screen to a lint: the sheet, the field and its words, and every cap on its key.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = lint.panel("entry sheet", b.lay.sheet, kit.ui.look.colour(PaletteRole::Sheet), true);
        b._prompt.feed_lint(kit, lint, "entry prompt", Some(sheet), (0.0, 0.0));
        let ink = kit.ui.look.ink();
        // the field is a faint wash on the sheet: its words are read against the sheet
        b.typed.feed_lint(kit, lint, "entry text", Some(sheet), (0.0, 0.0));
        let glass = kit.ui.look.colour(PaletteRole::Glass);
        let accent = kit.ui.look.accent();
        let accent_ink = kit.ui.look.accent_ink();
        let caps_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
        // the caps are one block at the grid: the compositor places the grid, the caps sit on whole pixels of it
        let grid = (b.lay.grid.x, b.lay.grid.y);
        for (i, f) in b.lay.keys.iter().enumerate() {
            let key = lint.panel_on(&format!("key {i}"), *f, if i == 42 { accent } else { glass }, false, sheet);
            let text = &b.caps_now[i];
            if !text.is_empty() {
                let style = Style { colour: if i == 42 { accent_ink } else { ink }, ..caps_style };
                lint.word_on(kit, &format!("cap {i}"), text, *f, style, Some(key), grid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::entry::Entry;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: EntrySurface,
        e: Entry,
        t: u64,
    }

    impl Up {
        fn new(mode: Mode, scale: f32, prompt: &str, text: &str, secret: bool) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut e = Entry::new();
            e.life.show(0);
            e.open(prompt, text, secret, 0);
            let mut me = Self { _h: h, kit, engine, s: EntrySurface::new(crate::kit::z::PANELS), e, t: 1000 };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.e.view(false);
            self.s.apply(&mut self.kit, &mut self.engine, &v, self.e.life.phase(), self.t, self.t * 1_000_000).expect("the sheet is drawn");
        }

        fn key(&mut self, k: K) {
            self.e.key(k, self.t);
            self.t += 40;
            self.apply();
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }

        fn geo(&self) -> Value {
            self.s.geometry(&self.engine)
        }
    }

    fn at(x: f32, y: f32, b: u32) -> Sample {
        Sample { x, y, buttons: b, visible: true }
    }

    #[test]
    fn the_sheet_sits_along_the_bottom_with_the_keys_in_ten_columns_and_the_focus_on_t() {
        let u = Up::new(Mode::Dark, 1.0, "TV name", "", false);
        let g = u.geo();
        assert_eq!(g["sheet"]["w"], 1792.0);
        assert!(g["sheet"]["y"].as_f64().unwrap() + g["sheet"]["h"].as_f64().unwrap() <= 1026.0, "inside the safe area: {g}");
        let keys = g["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 43);
        assert!(keys[1]["x"].as_f64().unwrap() > keys[0]["x"].as_f64().unwrap() && keys[10]["y"].as_f64().unwrap() > keys[0]["y"].as_f64().unwrap());
        assert_eq!(g["caps"][14], "t");
        assert_eq!(g["focus"], 14);
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
    }

    #[test]
    fn the_focus_glides_between_the_keys_and_typing_changes_the_words_in_place_with_no_layer_made() {
        let mut u = Up::new(Mode::Dark, 1.0, "TV name", "", false);
        let layers = u.kit.room().layers;
        u.key(K::Right);
        assert_eq!(u.geo()["focus"], 15);
        u.key(K::Ok);
        assert_eq!(u.geo()["shown"], "y");
        u.key(K::Digit(4));
        assert_eq!(u.geo()["shown"], "y4");
        assert_eq!(u.kit.room().layers, layers, "typing makes and drops nothing");
        u.key(K::Down);
        u.key(K::Down);
        u.key(K::Down);
        assert_eq!(u.geo()["focus"], 41, "straight down from y is the space bar, which spans the column");
    }

    #[test]
    fn shift_makes_one_capital_and_the_marks_replace_the_letters_without_making_the_sheet_again() {
        let mut u = Up::new(Mode::Dark, 1.0, "Name", "", false);
        let layers = u.kit.room().layers;
        let shift = (0..KEYS).find(|i| u.e.cap(*i) == SHIFT).unwrap();
        u.e.click(shift, 0);
        u.apply();
        assert_eq!(u.geo()["caps"][14], "T");
        let symbols = (0..KEYS).find(|i| u.e.cap(*i) == "?123").unwrap();
        u.e.click(symbols, 0);
        u.apply();
        assert_eq!(u.geo()["caps"][10], "@");
        assert_eq!(u.kit.room().layers, layers);
    }

    #[test]
    fn a_password_is_bullets_and_a_long_text_shows_its_end_led_by_an_ellipsis() {
        let mut u = Up::new(Mode::Dark, 1.0, "Wi-Fi password", "", true);
        for _ in 0..3 {
            u.key(K::Ok);
        }
        assert_eq!(u.geo()["shown"], "\u{2022}\u{2022}t");
        let long = format!("{}end", "w".repeat(120));
        let v = Up::new(Mode::Dark, 1.0, "Name", &long, false);
        let shown = v.geo()["shown"].as_str().unwrap().to_string();
        assert!(shown.starts_with('\u{2026}') && shown.ends_with("wend") && shown.chars().count() < 123, "{shown}");
    }

    #[test]
    fn a_click_on_a_key_names_it_and_the_dim_is_back() {
        let mut u = Up::new(Mode::Dark, 1.0, "Name", "", false);
        let k = u.geo()["keys"][3].clone();
        let (x, y) = (k["x"].as_f64().unwrap() as f32 + 10.0, k["y"].as_f64().unwrap() as f32 + 10.0);
        let t = |n: u64| (3000 + n) * 1_000_000;
        u.s.pointer(&u.kit, &mut u.engine, t(0), at(x, y, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(x, y, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(x, y, 0)), Some(Target::Key(3)));
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(20.0, 20.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(20.0, 20.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(50), at(20.0, 20.0, 0)), Some(Target::Backdrop));
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_with_a_short_prompt_a_long_one_and_symbols() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for (name, prompt, text, secret) in [
                    ("short", "TV name", "Living room", false),
                    ("long", "Password for the network called Nitin and family guests upstairs", "hunter2hunter2", true),
                ] {
                    let mut u = Up::new(mode, scale, prompt, text, secret);
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} {mode:?} {scale}: {f:#?}");
                    let symbols = (0..KEYS).find(|i| u.e.cap(*i) == "?123").unwrap();
                    u.e.click(symbols, 0);
                    u.apply();
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} marks {mode:?} {scale}: {f:#?}");
                    let g = u.geo();
                    assert!(g["sheet"]["y"].as_f64().unwrap() >= 54.0, "{name} {scale}: the sheet is on the screen: {}", g["sheet"]);
                }
            }
        }
    }

    #[test]
    fn leaving_and_going_free_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, "Name", "", false);
        u.e.close();
        u.e.life.hide(2000);
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE), "the keys stay while it slides out");
        u.e.life.tick(2200);
        u.apply();
        assert_eq!(u.engine.focus.modal(), None);
        u.e.life.tick(7000);
        u.apply();
        assert!(!u.s.is_up());
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Key(0), Target::Key(42), Target::Backdrop] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
        assert_eq!(Target::from_id(Id(1)), None);
    }
}
