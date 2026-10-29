//! Quick Access drawn (`panels/QuickAccess.qml`, the mocks `31-quick-access-toast` and `31-quick-access-assign`):
//!
//! - the **toast** for a number that opened something: a pill at the top right, 64 from the edge and 137 from the top, with the digit on a plate,
//!   "N \u{b7} Name" and the line under it ("Opening.", "Assigned. Opening.");
//! - the **question** for a number with nothing behind it: over a scrim that hides the page, a sheet with the digit and "Nothing on N yet", the
//!   four apps no number opens yet and More as cards in a row, "Not now" at the right of the foot and the keys that answer it as glyphs on the
//!   left (no sentence tells what they do). The model decides which card is chosen; this follows it. A click is the card it ended on.

use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt, glyph_side};
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::Sample;
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::{Lift, Plate, Spot, edge, selected_wash};
use crate::kit::{Engine, Kit, wrap};
use crate::panels::life::Phase;
use crate::panels::stage::{Stage, Touch};
use crate::panels::ui::{Arrive, Root, WordLine, Words, hints_width};
use crate::system::extras::{Ask, Toast};
use super::app_icon;
use compositor_protocol::{Fill, Props, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(210);
const REGION: Region = Region(3000);
const BASE: u32 = 0x3100_0000;
const FADE_MS: u32 = 140;
const TOAST_X: f32 = 64.0;
const TOAST_Y: f32 = 137.0;
const PLATE: f32 = 64.0;
const CARD_W: f32 = 820.0;
/// A card of the row is 140 wide at 100 % text and grows with the text, so a name keeps its words whole.
const PICK_W: f32 = 140.0;
const PICK_ICON: u32 = 76;

/// The digit's plate: 64, or what a digit in the title role needs at a large text size.
fn plate_side(kit: &Kit) -> f32 {
    PLATE.max(kit.line(TypeRole::Title) + 2.0 * kit.space(2)).ceil()
}

// ---- the toast
struct ToastBuilt {
    sig: String,
    pane: Pane,
    plate_f: Frame,
    _plate: ShapeLayer,
    digit: Text,
    title: Text,
    line: Text,
}

pub struct AccessToast {
    z: i32,
    root: Option<Root>,
    built: Option<ToastBuilt>,
    leaving_until: Option<u64>,
}

impl AccessToast {
    pub fn new(z: i32) -> Self {
        Self { z, root: None, built: None, leaving_until: None }
    }

    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    fn build(kit: &mut Kit, root: &Root, z: i32, t: &Toast, sig: String) -> std::io::Result<ToastBuilt> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (s4, s5) = (kit.space(4), kit.space(5));
        let title_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let line_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let digit_style = role_style(kit, TypeRole::Title, None, ink, Align::Centre);
        let title_text = format!("{} \u{b7} {}", t.digit, t.title);
        let side = plate_side(kit);
        let widest = width_of(kit, title_style, &title_text).max(width_of(kit, line_style, &t.line));
        let w = (2.0 * s5 + side + s5 + widest).max(560.0).ceil().min(1920.0 - 2.0 * TOAST_X);
        let room = w - 2.0 * s5 - side - s5;
        let title_text = wrap::elide(kit, title_style, &title_text, room);
        let line_text = wrap::elide(kit, line_style, &t.line, room);
        let (title_h, line_h) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
        let h = (2.0 * s5 + side).max(2.0 * s4 + title_h + line_h).ceil();
        let frame = Frame::new(1920.0 - TOAST_X - w, TOAST_Y, w, h);
        let plate_f = Frame::new(frame.x + s5, (frame.y + (h - side) / 2.0).round(), side, side);
        let shape = Shape { corner_radius: kit.tokens.radius(Radius::Control), fill: Fill::Solid(selected_wash(kit)), border_width: 1.0, border: edge(kit) };
        scene.adopting_view(&root.group, || -> std::io::Result<ToastBuilt> {
            let pane = Pane::new(kit, frame, z, PaneKind::Card, false);
            let plate_layer = plate::shape(&kit.ui, plate_f, z + 1, shape);
            let digit_h = kit.line(TypeRole::Title);
            let digit = Text::new(kit, Frame::new(plate_f.x, (plate_f.y + (side - digit_h) / 2.0).round(), side, digit_h), z + 2, digit_style, &t.digit)?;
            let x = plate_f.right() + s5;
            let top = (frame.y + (h - title_h - line_h) / 2.0).round();
            let title = Text::new(kit, Frame::new(x, top, room, title_h), z + 2, title_style, &title_text)?;
            let line = Text::new(kit, Frame::new(x, top + title_h, room, line_h), z + 2, line_style, &line_text)?;
            Ok(ToastBuilt { sig, pane, plate_f, _plate: plate_layer, digit, title, line })
        })
    }

    pub fn apply(&mut self, kit: &mut Kit, view: Option<&Toast>, now_ms: u64) -> std::io::Result<bool> {
        let Some(t) = view else {
            if self.root.is_some() && self.leaving_until.is_none() {
                if let Some(root) = self.root.as_mut() {
                    root.hide(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
                }
                self.leaving_until = Some(now_ms + u64::from(FADE_MS) + 60);
            }
            if self.leaving_until.is_some_and(|at| now_ms >= at) {
                self.drop_all();
            }
            return Ok(self.root.is_some());
        };
        let sig = format!("{}|{}|{:?}", kit.generation(), kit.text_scale, (&t.digit, &t.title, &t.line));
        if self.built.as_ref().is_some_and(|b| b.sig != sig) {
            self.drop_all();
        }
        self.leaving_until = None;
        if self.root.is_none() {
            self.root = Some(Root::new(kit, Arrive::Fade, false)?);
        }
        if self.built.is_none() {
            let root = self.root.as_ref().expect("a root");
            let built = Self::build(kit, root, self.z, t, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the Quick Access toast could not be drawn whole"));
            }
            self.built = Some(built);
        }
        if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
            root.show(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
        }
        Ok(true)
    }

    /// The fade out is over: the next frame drops it.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.leaving_until.is_some_and(|t| now_ms >= t) {
            self.drop_all();
            return true;
        }
        false
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving_until
    }

    fn drop_all(&mut self) {
        self.built = None;
        self.root = None;
        self.leaving_until = None;
    }

    pub fn restyle(&mut self) {
        self.drop_all();
    }

    pub fn geometry(&self) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = b.pane.frame();
        json!({"x": f.x, "y": f.y, "w": f.w, "h": f.h, "digit": b.digit.text(), "title": b.title.text(), "line": b.line.text()})
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let panel = lint.panel("access toast", b.pane.frame(), kit.ui.look.colour(PaletteRole::Sheet), true);
        let title = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Left);
        let line = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink2(), Align::Left);
        lint.word(kit, "access toast title", b.title.text(), b.title.frame(), title, Some(panel));
        lint.word(kit, "access toast line", b.line.text(), b.line.frame(), line, Some(panel));
        let digit = role_style(kit, TypeRole::Title, None, kit.ui.look.ink(), Align::Centre);
        let wash = crate::kit::button::faded(kit.ui.look.ink(), kit.tokens.states.selected_wash);
        let on = lint.panel_on("access digit plate", b.plate_f, wash, false, panel);
        lint.word(kit, "access digit", b.digit.text(), b.digit.frame(), digit, Some(on));
    }
}

// ---- the question
/// Where the pointer can land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A card of the row: the apps, then More.
    Pick(usize),
    Later,
    Backdrop,
}

impl Target {
    fn id(self) -> Id {
        match self {
            Target::Pick(i) => Id(BASE + i as u32),
            Target::Later => Id(BASE + 0x20),
            Target::Backdrop => Id(BASE + 0xFF00),
        }
    }

    fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            0xFF00 => Some(Target::Backdrop),
            0x20 => Some(Target::Later),
            i if i < 0x10 => Some(Target::Pick(i as usize)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PickLay {
    frame: Frame,
    icon_at: (f32, f32),
    name: Vec<String>,
    name_at: (f32, f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
struct AskLay {
    card: Frame,
    plate: Frame,
    digit_at: Frame,
    title: Vec<String>,
    title_at: (f32, f32, f32),
    picks: Vec<PickLay>,
    later: Frame,
    hints: Vec<Hint>,
    hints_at: (f32, f32, f32),
}

fn hints() -> Vec<Hint> {
    vec![
        Hint { keys: vec![(Key::Left, false), (Key::Right, false)], action: Some(Action::Move), label: None },
        Hint::key(Key::Ok, Action::Confirm),
        Hint::key(Key::Back, Action::Close),
    ]
}

fn layout(kit: &mut Kit, ask: &Ask) -> AskLay {
    let (s1, s2, s3, s4, s5, s6, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(4), kit.space(5), kit.space(6), kit.space(7));
    let ink = kit.ui.look.ink();
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink, Align::Centre);
    let (title_h, caption_h) = (kit.line(TypeRole::Title), kit.line(TypeRole::Caption));
    let n = ask.picks.len() + 1;
    let pick_w = (PICK_W * kit.text_scale.max(1.0)).round();
    let row_w = n as f32 * pick_w + (n - 1) as f32 * s3;
    let w = CARD_W.max(row_w + 2.0 * s6).min(1792.0);
    let inner = w - 2.0 * s6;
    let side = plate_side(kit);
    let title = wrap::lines(kit, title_style, &format!("Nothing on {} yet", ask.digit), inner - side - s4, 2);
    let head_h = side.max(title.len() as f32 * title_h);
    let name_y = s3 + PICK_ICON as f32 + s2;
    let mut names: Vec<Vec<String>> = ask.picks.iter().map(|p| wrap::lines(kit, caption_style, &p.title, pick_w - 2.0 * s1, 3)).collect();
    names.push(wrap::lines(kit, caption_style, "More\u{2026}", pick_w - 2.0 * s1, 3));
    let pick_h = names.iter().map(|l| (name_y + l.len() as f32 * caption_h + s3).ceil()).fold(150.0, f32::max);
    let later_w = Button::width_for(kit, "Not now", false);
    let btn_h = Button::height(kit);
    let glyph = glyph_side(kit, TypeRole::Label) as f32;
    let foot_h = btn_h.max(glyph);
    // the keys as glyphs at the left of the foot; at a large text size what does not fit beside the button is the first to go
    let mut hs = hints();
    while hs.len() > 1 && hints_width(kit, &hs, TypeRole::Label, s7) > inner - later_w - s5 {
        hs.remove(0);
    }
    let h = (s6 + head_h + s5 + pick_h + s5 + foot_h + s6).ceil();
    let card = Frame::new(((1920.0 - w) / 2.0).round(), ((1080.0 - h) / 2.0).round(), w, h);
    let plate_f = Frame::new(card.x + s6, card.y + s6, side, side);
    let digit_h = kit.line(TypeRole::Title);
    let digit_at = Frame::new(plate_f.x, (plate_f.y + (side - digit_h) / 2.0).round(), side, digit_h);
    let title_x = plate_f.right() + s4;
    let title_y = (card.y + s6 + (head_h - title.len() as f32 * title_h) / 2.0).round();
    let row_y = card.y + s6 + head_h + s5;
    let row_x = card.x + s6 + ((inner - row_w) / 2.0).max(0.0);
    let picks = names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let frame = Frame::new((row_x + i as f32 * (pick_w + s3)).round(), row_y, pick_w, pick_h);
            PickLay { frame, icon_at: (((pick_w - PICK_ICON as f32) / 2.0).round(), s3), name, name_at: (s1, name_y, pick_w - 2.0 * s1) }
        })
        .collect();
    let foot_y = row_y + pick_h + s5;
    let later = Frame::new(card.right() - s6 - later_w, foot_y + ((foot_h - btn_h) / 2.0).round(), later_w, btn_h);
    AskLay {
        card,
        plate: plate_f,
        digit_at,
        title,
        title_at: (title_x, title_y, card.right() - s6 - title_x),
        picks,
        later,
        hints: hs,
        hints_at: (card.x + s6, foot_y, foot_h),
    }
}

struct PickParts {
    plate: Plate,
    _icon: ImageLayer,
    _name: Words,
    lay: PickLay,
}

struct AskBuilt {
    sig: String,
    lay: AskLay,
    _card: Pane,
    _plate: ShapeLayer,
    digit: Text,
    title: Words,
    picks: Vec<PickParts>,
    more: usize,
    later: Button,
    hint_rows: Vec<HintRow>,
    ask: Ask,
}

pub struct AccessAsk {
    stage: Stage,
    built: Option<AskBuilt>,
}

impl AccessAsk {
    /// `z` is the depth of the scrim; the sheet is above it.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).hiding(), built: None }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, ask: &Ask, lay: &AskLay) -> String {
        let picks: Vec<_> = ask.picks.iter().map(|p| (&p.id, &p.title, &p.icon)).collect();
        format!("{}|{}|{}|{:?}|{:?}", kit.generation(), kit.text_scale, ask.digit, picks, lay.card)
    }

    fn build(kit: &mut Kit, engine: &mut Engine, group: &ui::group::Group, z: i32, ask: &Ask, lay: AskLay, sig: String) -> std::io::Result<AskBuilt> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let radius = kit.tokens.radius(Radius::Row);
        let plate_shape = Shape { corner_radius: kit.tokens.radius(Radius::Control), fill: Fill::Solid(selected_wash(kit)), border_width: 1.0, border: edge(kit) };
        let digit_style = role_style(kit, TypeRole::Title, None, ink, Align::Centre);
        scene.adopting_view(group, || -> std::io::Result<AskBuilt> {
            let card = Pane::new(kit, lay.card, z + 1, PaneKind::Sheet, false);
            let plate_layer = plate::shape(&kit.ui, lay.plate, z + 2, plate_shape);
            let digit = Text::new(kit, lay.digit_at, z + 3, digit_style, &ask.digit)?;
            let title_lines: Vec<WordLine> = lay.title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
            let title = Words::new(kit, lay.title_at.0, lay.title_at.1, lay.title_at.2, z + 3, &title_lines)?;
            let mut picks = Vec::new();
            for (i, pl) in lay.picks.iter().enumerate() {
                let f = pl.frame;
                let id = Target::Pick(i).id();
                let parts = Plate::new(kit, engine, group, Spot { id, screen: f, local: f, region: REGION, scope: SCOPE, z: z + 3 }, Lift::Row, radius)?;
                let at = Props::at(f.x + pl.icon_at.0, f.y + pl.icon_at.1);
                let icon = match ask.picks.get(i) {
                    Some(p) => app_icon(kit, &p.icon, PICK_ICON, 18.0, z + 4, at)?,
                    None => crate::kit::widgets::tile(kit, PICK_ICON, "ellipsis", false).layer(kit, z + 4, at)?,
                };
                let lines: Vec<WordLine> = pl.name.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink).aligned(Align::Centre)).collect();
                let name = Words::new(kit, f.x + pl.name_at.0, f.y + pl.name_at.1, pl.name_at.2, z + 4, &lines)?;
                picks.push(PickParts { plate: parts, _icon: icon, _name: name, lay: pl.clone() });
            }
            let mut later = Button::new(kit, engine, Place { id: Target::Later.id(), frame: lay.later, region: REGION, scope: SCOPE, z: z + 4 }, ButtonSpec::new("Not now", Tone::Plain))?;
            later.show(kit, true);
            let mut rows = Vec::new();
            let mut x = lay.hints_at.0;
            for h in &lay.hints {
                let row = HintRow::new(kit, h, TypeRole::Label, RowAt { x, y: lay.hints_at.1, h: lay.hints_at.2, z: z + 3 }, ink2)?;
                x += row.width() + kit.space(7);
                rows.push(row);
            }
            Ok(AskBuilt { sig, lay: lay.clone(), _card: card, _plate: plate_layer, digit, title, more: ask.more(), picks, later, hint_rows: rows, ask: ask.clone() })
        })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for p in b.picks {
                p.plate.destroy(engine);
            }
            b.later.destroy(engine);
        }
    }

    fn id_of(ask: &Ask) -> Id {
        if ask.index == ask.later() { Target::Later.id() } else { Target::Pick(ask.index.min(ask.more())).id() }
    }

    /// Fits the screen to the question: made when one comes (or its apps, look or size change), the focus follows the model's choice, dropped at
    /// once when it has been answered. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: Option<&Ask>, now_ns: u64) -> std::io::Result<bool> {
        let Some(ask) = view else {
            self.drop_all(engine);
            return Ok(false);
        };
        let lay = layout(kit, ask);
        self.stage.ensure(kit, Phase::Opening, Arrive::Fade, true)?;
        let sig = Self::signature(kit, ask, &lay);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), ask, lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the Quick Access question could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if self.stage.phase(kit, engine, Phase::Opening) {
            self.stage.focus(engine, Some(Self::id_of(ask)), now_ns);
            if let Some(b) = self.built.as_mut() {
                b.ask.index = ask.index;
            }
        }
        Ok(true)
    }

    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.stage.key(engine);
        if let Some(b) = self.built.as_mut() {
            b.later.set_hover(kit, false);
        }
    }

    /// A sample of the pointer: the cue follows it and a click is the card or the button it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let radius = kit.tokens.radius(Radius::Row);
        let built = &mut self.built;
        let current = engine.focus.current();
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { from, to } => {
                if let (Some(b), Some(Target::Later)) = (built.as_mut(), from.and_then(Target::from_id)) {
                    b.later.set_hover(kit, false);
                }
                let landed = to.filter(|t| current != Some(*t)).and_then(Target::from_id);
                if let (Some(b), Some(Target::Later)) = (built.as_mut(), landed) {
                    b.later.set_hover(kit, true);
                }
                matches!(landed, Some(Target::Pick(_))).then_some(radius)
            }
            Touch::Press { id, down } => {
                if let (Some(b), Some(Target::Later)) = (built.as_mut(), Target::from_id(id)) {
                    b.later.set_pressed(kit, down);
                }
                None
            }
        })?;
        Target::from_id(click)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let picks: Vec<Value> = b.picks.iter().map(|p| f(engine.focus.frame(p.plate.id).unwrap_or(p.lay.frame))).collect();
        json!({
            "card": f(b.lay.card), "title": b.lay.title, "digit": b.digit.text(), "picks": picks, "more": b.more, "later": f(b.lay.later),
            "hints": b.hint_rows.iter().map(|h| h.name().to_string()).collect::<Vec<_>>(),
            "focus": engine.focus.current().and_then(Target::from_id).map(|t| match t { Target::Pick(i) => json!(i), Target::Later => json!("later"), Target::Backdrop => Value::Null }),
            "scope": SCOPE.0,
        })
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let hide = kit.tokens.extra(kit.mode).scrim_hide;
        lint.panel("access dim", Frame::new(0.0, 0.0, 1920.0, 1080.0), hide, false);
        let sheet = lint.panel("access card", b.lay.card, kit.ui.look.colour(PaletteRole::Sheet), true);
        let digit = role_style(kit, TypeRole::Title, None, kit.ui.look.ink(), Align::Centre);
        let wash = crate::kit::button::faded(kit.ui.look.ink(), kit.tokens.states.selected_wash);
        let on = lint.panel_on("access digit plate", b.lay.plate, wash, false, sheet);
        lint.word(kit, "access digit", b.digit.text(), b.digit.frame(), digit, Some(on));
        b.title.feed_lint(kit, lint, "access title", Some(sheet), (0.0, 0.0));
        for (i, p) in b.picks.iter().enumerate() {
            p._name.feed_lint(kit, lint, &format!("access pick {i}"), Some(sheet), (0.0, 0.0));
        }
        b.later.feed_lint(kit, lint, sheet);
    }
}
