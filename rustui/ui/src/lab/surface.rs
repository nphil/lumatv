//! The lab drawn (`docs/mocks/host-rust/displaylab/`): a test card over the whole screen with the zone to look at marked (a frame, a word
//! and an arrow), and beside it one sheet: where to look, what the difference looks like, the option on the screen in big letters (A or B, and
//! its name), the two switches for the pointer, the answers one under the other, and the keys as glyphs. The intro and the results are one
//! sheet over a plain dark page.
//!
//! The model decides which answer has the focus and which option shows; this follows it, and a switch between A and B changes two words and a
//! chip's wash, never the layers. The layers are made again when the step, its answers, its note or the look or the text size change.
//!
//! At large text the sheet gives up what is least needed so that nothing leaves the screen: the answers go two to a row, then the chips (the
//! left and right keys and the wheel switch too), then the line on what the difference looks like.

use super::card;
use super::model::{Body, IntroView, ResultsView, StepView, Target, View};
use super::plan::Card;
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt};
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::{edge, tile};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use crate::panels::life::Phase;
use crate::panels::stage::{Stage, Touch};
use crate::panels::ui::{Arrive, WordLine, Words};
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Style, plate, style};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::{Type, Weight};

pub const SCOPE: Scope = Scope(280);
const REGION: Region = Region(2800);
const BASE: u32 = 0x4C41_0000;
/// The safe area: the sheet stays inside it.
const SAFE_TOP: f32 = 54.0;
const SAFE_H: f32 = 972.0;
const SAFE_RIGHT: f32 = 1824.0;
/// The most the sheet is wide beside the window: from the frame round it (right edge 1216) to the safe area, a hand's width apart.
const WINDOW_SIDE: f32 = 592.0;
/// The tile with the lab's glyph in the head of the intro and of the results (the confirm's, 56 square).
const TILE: f32 = 56.0;

pub fn id_of(t: Target) -> Id {
    Id(BASE
        + match t {
            Target::Start => 1,
            Target::Leave => 2,
            Target::Chip(i) => 0x10 + i as u32,
            Target::Answer(i) => 0x20 + i as u32,
            Target::Done => 0x30,
            Target::Again => 0x31,
        })
}

pub fn target_of(id: Id) -> Option<Target> {
    Some(match id.0.checked_sub(BASE)? {
        1 => Target::Start,
        2 => Target::Leave,
        n @ 0x10..=0x11 => Target::Chip((n - 0x10) as usize),
        n @ 0x20..=0x2F => Target::Answer((n - 0x20) as usize),
        0x30 => Target::Done,
        0x31 => Target::Again,
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------------------------------------------- the layout

/// What a sheet gives up, in order, when it does not fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Variant {
    /// The answers side by side, each as wide as its words.
    grid: bool,
    chips: bool,
    hint: bool,
    /// The keys as glyphs under the answers.
    keys: bool,
}

const VARIANTS: [Variant; 5] = [
    Variant { grid: false, chips: true, hint: true, keys: true },
    Variant { grid: true, chips: true, hint: true, keys: true },
    Variant { grid: true, chips: false, hint: true, keys: true },
    Variant { grid: true, chips: false, hint: false, keys: true },
    Variant { grid: true, chips: false, hint: false, keys: false },
];

#[derive(Clone, Debug, PartialEq)]
struct StepLay {
    variant: Variant,
    panel: Frame,
    title: Frame,
    counter: Frame,
    hint: Vec<String>,
    hint_at: (f32, f32, f32),
    note: Vec<String>,
    note_at: (f32, f32, f32),
    letter: Frame,
    name: Frame,
    question: Vec<String>,
    question_at: (f32, f32, f32),
    chips: Vec<Frame>,
    answers: Vec<Frame>,
    /// Where the key glyphs start, if they are drawn.
    hints_at: Option<(f32, f32, f32)>,
    /// The zone on the card, the frame round it, the tag that says what to look at (an eye, then the words), and the arrow from the sheet to it.
    zone: Frame,
    mark: Frame,
    tag: Frame,
    arrow: Option<Frame>,
}

fn pad(kit: &Kit) -> f32 {
    kit.space(6)
}

/// How wide the sheet is: wider as the text grows, and no wider than the window step leaves room for.
fn sheet_w(kit: &Kit, window: bool) -> f32 {
    let w = (560.0 * kit.text_scale.powf(0.8)).round().clamp(560.0, 900.0);
    // the window is 480 wide in the middle: the hole would cut a sheet that reached into it
    if window { w.min(WINDOW_SIDE) } else { w }
}

/// The type of the big letter: the display role, at the person's text size up to a quarter more. The letter is a mark (as big as the sheet is
/// wide at 100 %), and a bigger one only costs the room the answers need.
fn letter_type(kit: &Kit) -> Type {
    let base = kit.tokens.ty(TypeRole::Display);
    Type { px: (base.px as f32 * kit.text_scale.clamp(1.0, 1.25)).ceil() as u32, weight: base.weight }
}

fn step_layout(kit: &mut Kit, v: &StepView, variant: Variant) -> StepLay {
    let (s2, s3, s4, s6) = (kit.space(2), kit.space(3), kit.space(4), pad(kit));
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let window = v.card == Card::Window;
    let w = sheet_w(kit, window);
    let (x0, inner) = (SAFE_RIGHT - w + s6, w - 2.0 * s6);
    let hint_style = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
    let note_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
    let q_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
    let hint = if variant.hint { wrap::lines(kit, hint_style, v.hint, inner, 3) } else { Vec::new() };
    let note_max = if kit.text_scale > 1.25 { 2 } else { 3 };
    let note = v.note.as_deref().map(|n| wrap::lines(kit, note_style, n, inner, note_max)).unwrap_or_default();
    let question = wrap::lines(kit, q_style, v.question, inner, 4);
    let (bh, gap) = (Button::height(kit), s2);
    let title_h = kit.line(TypeRole::Title);
    let mut y = s6;
    let split = (inner * 0.62).round();
    let title = Frame::new(x0, y, split, title_h);
    let counter = Frame::new(x0 + split, y + ((title_h - kit.line(TypeRole::Caption)) / 2.0).round(), inner - split, kit.line(TypeRole::Caption));
    y += title_h + s2;
    let hint_at = (x0, y, inner);
    if !hint.is_empty() {
        y += hint.len() as f32 * kit.line(TypeRole::Label) + s2;
    }
    let note_at = (x0, y, inner);
    if !note.is_empty() {
        y += note.len() as f32 * kit.line(TypeRole::Label) + s2;
    }
    y += s3;
    let big = ui::components::line_height(letter_type(kit));
    let letter_style = style(letter_type(kit), ink, Align::Left);
    let letter_w = width_of(kit, letter_style, "B") + s2;
    let letter = Frame::new(x0, y, letter_w, big);
    let name_h = kit.line(TypeRole::Title);
    let name = Frame::new(x0 + letter_w + s3, y + ((big - name_h) / 2.0).round(), inner - letter_w - s3, name_h);
    y += big + s2;
    // the two looks, as a pair of switches right under the one that is on the screen, and then what is asked
    let mut chips = Vec::new();
    if variant.chips {
        let cw = ((inner - s2) / 2.0).floor();
        chips = vec![Frame::new(x0, y, cw, bh), Frame::new(x0 + cw + s2, y, cw, bh)];
        y += bh + s4;
    }
    let question_at = (x0, y, inner);
    y += question.len() as f32 * kit.line(TypeRole::Label) + s2;
    let mut answers = Vec::new();
    if variant.grid {
        // side by side at the width each label needs, on to the next row where the sheet is full
        let (mut x, mut row_y) = (x0, y);
        for r in &v.replies {
            let bw = Button::width_for(kit, r.label(), false).max(kit.tokens.target * 2.0).min(inner);
            if x > x0 && x + bw > x0 + inner {
                x = x0;
                row_y += bh + gap;
            }
            answers.push(Frame::new(x, row_y, bw, bh));
            x += bw + gap;
        }
        y = row_y + bh;
    } else {
        let n = v.replies.len();
        for i in 0..n {
            answers.push(Frame::new(x0, y + i as f32 * (bh + gap), inner, bh));
        }
        y += n as f32 * bh + n.saturating_sub(1) as f32 * gap;
    }
    y += s4;
    let side = crate::kit::hint::glyph_side(kit, TypeRole::Caption) as f32;
    let hints_at = variant.keys.then_some((x0, y, side));
    if variant.keys {
        y += side;
    }
    y += s6;
    let h = y.ceil();
    let top = (SAFE_TOP + ((SAFE_H - h) / 2.0).max(0.0)).round();
    let shift = |f: Frame| Frame::new(f.x, f.y + top, f.w, f.h);
    let at3 = |a: (f32, f32, f32)| (a.0, a.1 + top, a.2);
    let panel = Frame::new(SAFE_RIGHT - w, top, w, h);
    let zone = v.card.zone();
    let grow = 16.0;
    let mark = Frame::new(zone.x - grow, zone.y - grow, zone.w + 2.0 * grow, zone.h + 2.0 * grow);
    let tag_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.accent_ink(), Align::Left);
    let tag_h = (kit.line(TypeRole::Label) + 2.0 * kit.space(1)).ceil();
    let tag_w = (tag_icon(kit) as f32 + kit.space(2) + width_of(kit, tag_style, v.look) + 2.0 * kit.space(4)).ceil();
    let above = mark.y - tag_h - kit.space(2);
    // above the frame where there is room under the top of the safe area, else under it
    let tag = Frame::new(mark.x, if above >= SAFE_TOP { above } else { mark.y + mark.h + kit.space(2) }, tag_w.min(mark.w), tag_h);
    // the arrow stands between the frame and the sheet when there is room for it
    let side_arrow = 96.0;
    let room = panel.x - (mark.x + mark.w);
    let arrow = (room >= side_arrow + 2.0 * s4)
        .then(|| Frame::new(mark.x + mark.w + ((room - side_arrow) / 2.0).round(), (zone.y + zone.h / 2.0 - side_arrow / 2.0).round(), side_arrow, side_arrow));
    StepLay {
        variant,
        panel,
        title: shift(title),
        counter: shift(counter),
        hint,
        hint_at: at3(hint_at),
        note,
        note_at: at3(note_at),
        letter: shift(letter),
        name: shift(name),
        question,
        question_at: at3(question_at),
        chips: chips.into_iter().map(shift).collect(),
        answers: answers.into_iter().map(shift).collect(),
        hints_at: hints_at.map(at3),
        zone,
        mark,
        tag,
        arrow,
    }
}

/// The eye in the tag: the height of the words beside it.
fn tag_icon(kit: &Kit) -> u32 {
    (kit.line(TypeRole::Label) * 0.8).round() as u32
}

/// The first variant that fits the safe height, or the last one (and the lint says what is left).
fn plan_step(kit: &mut Kit, v: &StepView) -> StepLay {
    let mut last = None;
    for variant in VARIANTS {
        let lay = step_layout(kit, v, variant);
        let fits = lay.panel.h <= SAFE_H;
        last = Some(lay);
        if fits {
            break;
        }
    }
    last.expect("a variant")
}

// ------------------------------------------------------------------------------------------------------------------------- what is built

struct StepBuilt {
    lay: StepLay,
    card: ImageLayer,
    /// How the text step's letters are drawn on it now (see `StepView::fit`).
    fit: Option<bool>,
    _mark: ShapeLayer,
    _tag: ShapeLayer,
    _eye: Option<ImageLayer>,
    tag_text: Text,
    tag_text_style: Style,
    _arrow: Option<ImageLayer>,
    _panel: ShapeLayer,
    title: Text,
    counter: Text,
    hint: Words,
    note: Words,
    letter: Text,
    name: Text,
    question: Words,
    chips: Vec<Button>,
    answers: Vec<Button>,
    _hints: Vec<HintRow>,
    styles: Styles,
    shown: usize,
    layers: usize,
}

/// The styles the words were made in, for the lint.
#[derive(Clone, Copy)]
struct Styles {
    title: Style,
    counter: Style,
    letter: Style,
    name: Style,
}

struct SheetBuilt {
    panel_frame: Frame,
    _panel: ShapeLayer,
    _tile: ImageLayer,
    title: Text,
    title_style: Style,
    body: Words,
    note: Words,
    rows_left: Words,
    rows_right: Words,
    saved: Words,
    buttons: Vec<(Target, Button)>,
    _hints: Vec<HintRow>,
    layers: usize,
}

enum Built {
    Step(Box<StepBuilt>),
    Sheet(Box<SheetBuilt>),
}

pub struct LabSurface {
    stage: Stage,
    z: i32,
    built: Option<(String, Built)>,
    /// The rasterisers of the text step's two looks.
    faces: card::TextFaces,
}

impl LabSurface {
    /// `z` is the lab's band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE).hiding(), z, built: None, faces: card::TextFaces::default() }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, v: &View) -> String {
        let body = match &v.body {
            Body::Intro(i) => format!("intro|{i:?}"),
            Body::Step(s) => format!("step|{}|{:?}|{:?}", s.id, s.replies, s.note),
            Body::Results(r) => format!("results|{:?}|{}", r.rows, r.saved),
        };
        format!("{}|{}|{body}", kit.generation(), kit.text_scale)
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some((_, built)) = self.built.take() {
            match built {
                Built::Step(b) => {
                    for btn in b.chips.into_iter().chain(b.answers) {
                        btn.destroy(engine);
                    }
                }
                Built::Sheet(b) => {
                    for (_, btn) in b.buttons {
                        btn.destroy(engine);
                    }
                }
            }
        }
    }

    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, now_ns: u64) -> std::io::Result<bool> {
        if v.phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        if self.stage.needs_root(Arrive::Fade) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, v.phase, Arrive::Fade, true)?;
        let sig = Self::signature(kit, v);
        if self.built.as_ref().is_none_or(|(s, _)| *s != sig) {
            self.drop_built(engine);
            if kit.room().layers < 90 {
                return Err(std::io::Error::other("the scene has no room for the lab"));
            }
            let built = self.build(kit, engine, v)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the lab could not be drawn whole"));
            }
            self.built = Some((sig, built));
            self.stage.refocus();
        }
        self.follow(kit, engine, v);
        if self.stage.phase(kit, engine, v.phase) {
            let want = match &v.body {
                Body::Intro(i) => (i.ready || i.refused).then_some(if i.ready { Target::Start } else { Target::Leave }),
                Body::Step(s) => Some(Target::Answer(s.focus.min(s.replies.len().saturating_sub(1)))),
                Body::Results(r) => Some(if r.focus == 0 { Target::Done } else { Target::Again }),
            };
            self.stage.focus(engine, want.map(id_of), now_ns);
        }
        Ok(true)
    }

    /// What changes without a rebuild: the option on the screen.
    fn follow(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View) {
        let _ = engine;
        let (Some((_, Built::Step(b))), Body::Step(s)) = (self.built.as_mut(), &v.body) else { return };
        if let Some(fit) = s.fit
            && b.fit != Some(fit)
        {
            // the text step: the same letters again, drawn the other way
            card::paint(s.card, b.card.pixels_mut(), self.faces.get(fit));
            b.card.damage(None);
            b.fit = Some(fit);
        }
        if b.shown != s.shown || b.letter.text() != s.options[s.shown].letter {
            b.shown = s.shown;
            b.letter.set(kit, s.options[s.shown].letter);
            b.name.set(kit, s.options[s.shown].name);
            for (i, c) in b.chips.iter_mut().enumerate() {
                c.set_selected(i == s.shown);
            }
        }
    }

    fn build(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let root = self.stage.group();
        let z = self.z;
        match &v.body {
            Body::Step(s) => {
                let lay = plan_step(kit, s);
                let faces = &mut self.faces;
                let mut b = scene.adopting_view(root, || Self::build_step(kit, engine, faces, z, s, lay)).map_err(std::io::Error::other)?;
                b.layers = before.saturating_sub(kit.room().layers);
                Ok(Built::Step(Box::new(b)))
            }
            Body::Intro(i) => {
                let mut b = scene.adopting_view(root, || Self::build_intro(kit, engine, z, i)).map_err(std::io::Error::other)?;
                b.layers = before.saturating_sub(kit.room().layers);
                Ok(Built::Sheet(Box::new(b)))
            }
            Body::Results(r) => {
                let mut b = scene.adopting_view(root, || Self::build_results(kit, engine, z, r)).map_err(std::io::Error::other)?;
                b.layers = before.saturating_sub(kit.room().layers);
                Ok(Built::Sheet(Box::new(b)))
            }
        }
    }

    fn sheet_shape(kit: &Kit) -> Shape {
        Shape {
            corner_radius: kit.tokens.radius(Radius::Sheet),
            fill: Fill::Solid(kit.ui.look.colour(PaletteRole::Sheet)),
            border_width: 1.0,
            border: edge(kit),
        }
    }

    fn build_step(kit: &mut Kit, engine: &mut Engine, faces: &mut card::TextFaces, z: i32, s: &StepView, lay: StepLay) -> std::io::Result<StepBuilt> {
        let (ink, ink2, accent) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent());
        // the card: the whole page, painted in place
        let mut card_layer = kit.ui.scene.image(card::W as u32, card::H as u32, z + 1, Props::at(0.0, 0.0))?;
        match s.fit {
            Some(fit) => card::paint(s.card, card_layer.pixels_mut(), faces.get(fit)),
            None => card::paint(s.card, card_layer.pixels_mut(), &mut kit.ui.fonts),
        }
        card_layer.damage(None);
        // the frame round the zone, the word on it, and the arrow
        let radius = kit.tokens.radius(Radius::Card);
        let mark =
            plate::shape(&kit.ui, lay.mark, z + 2, Shape { corner_radius: radius, fill: Fill::Solid(Rgba(0, 0, 0, 0)), border_width: 4.0, border: accent });
        let tag = plate::shape(&kit.ui, lay.tag, z + 3, plate::solid(accent, (lay.tag.h / 2.0).floor()));
        let tag_ink = kit.ui.look.accent_ink();
        let tag_text_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), tag_ink, Align::Left);
        let (icon, s2, s4) = (tag_icon(kit), kit.space(2), kit.space(4));
        let eye = crate::kit::icon::layer(kit, "eye", icon, tag_ink, z + 4, Props::at(lay.tag.x + s4, lay.tag.y + ((lay.tag.h - icon as f32) / 2.0).round()))?;
        let text_x = lay.tag.x + s4 + icon as f32 + s2;
        let line = kit.line(TypeRole::Label);
        let tag_text = Text::new(
            kit,
            Frame::new(text_x, lay.tag.y + ((lay.tag.h - line) / 2.0).round(), lay.tag.x + lay.tag.w - s4 - text_x, line),
            z + 4,
            tag_text_style,
            s.look,
        )?;
        let arrow = match lay.arrow {
            Some(f) => crate::kit::icon::layer(kit, "arrow-left", f.w as u32, accent, z + 3, Props::at(f.x, f.y))?,
            None => None,
        };
        let panel = plate::shape(&kit.ui, lay.panel, z + 5, Self::sheet_shape(kit));
        let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let counter_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
        let letter_style = style(letter_type(kit), ink, Align::Left);
        let name_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let title = Text::new(kit, lay.title, z + 6, title_style, s.group)?;
        let counter = Text::new(kit, lay.counter, z + 6, counter_style, &s.counter)?;
        let lines = |kit: &Kit, texts: &[String], role: TypeRole, weight: Option<Weight>, ink: Rgba| -> Vec<WordLine> {
            let _ = kit;
            texts
                .iter()
                .map(|t| {
                    let l = WordLine::new(t, role, ink);
                    match weight {
                        Some(w) => l.weight(w),
                        None => l,
                    }
                })
                .collect()
        };
        let hint = Words::new(kit, lay.hint_at.0, lay.hint_at.1, lay.hint_at.2, z + 6, &lines(kit, &lay.hint, TypeRole::Label, None, ink2))?;
        let note = Words::new(kit, lay.note_at.0, lay.note_at.1, lay.note_at.2, z + 6, &lines(kit, &lay.note, TypeRole::Label, Some(Weight::SemiBold), ink))?;
        let opt = &s.options[s.shown];
        let letter = Text::new(kit, lay.letter, z + 6, letter_style, opt.letter)?;
        let name = Text::new(kit, lay.name, z + 6, name_style, opt.name)?;
        let question = Words::new(
            kit,
            lay.question_at.0,
            lay.question_at.1,
            lay.question_at.2,
            z + 6,
            &lines(kit, &lay.question.clone(), TypeRole::Label, Some(Weight::SemiBold), ink),
        )?;
        let mut chips = Vec::new();
        for (i, f) in lay.chips.iter().enumerate() {
            let label = format!("{}  {}", s.options[i].letter, s.options[i].name);
            let mut b = Button::new(
                kit,
                engine,
                Place { id: id_of(Target::Chip(i)), frame: *f, region: REGION, scope: SCOPE, z: z + 7 },
                ButtonSpec::new(&label, Tone::Plain).selected(i == s.shown),
            )?;
            b.show(kit, true);
            chips.push(b);
        }
        let mut answers = Vec::new();
        for (i, (r, f)) in s.replies.iter().zip(&lay.answers).enumerate() {
            let mut b = Button::new(
                kit,
                engine,
                Place { id: id_of(Target::Answer(i)), frame: *f, region: REGION, scope: SCOPE, z: z + 7 },
                ButtonSpec::new(r.label(), Tone::Plain),
            )?;
            b.show(kit, true);
            answers.push(b);
        }
        let mut hints = Vec::new();
        if let Some((hx, hy, hh)) = lay.hints_at {
            let mut x = hx;
            for h in step_hints() {
                let row = HintRow::new(kit, &h, TypeRole::Caption, RowAt { x, y: hy, h: hh, z: z + 6 }, ink2)?;
                x += row.width() + kit.space(6);
                hints.push(row);
            }
        }
        for h in &mut hints {
            h.set_visible(true);
        }
        Ok(StepBuilt {
            lay,
            card: card_layer,
            fit: s.fit,
            _mark: mark,
            _tag: tag,
            _eye: eye,
            tag_text,
            tag_text_style,
            _arrow: arrow,
            _panel: panel,
            title,
            counter,
            hint,
            note,
            letter,
            name,
            question,
            chips,
            answers,
            _hints: hints,
            styles: Styles { title: title_style, counter: counter_style, letter: letter_style, name: name_style },
            shown: s.shown,
            layers: 0,
        })
    }

    fn build_intro(kit: &mut Kit, engine: &mut Engine, z: i32, i: &IntroView) -> std::io::Result<SheetBuilt> {
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (s3, s4, s6) = (kit.space(3), kit.space(4), pad(kit));
        let w = (760.0 * kit.text_scale.powf(0.8)).round().clamp(760.0, 1500.0);
        let inner = w - 2.0 * s6;
        let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
        let note_style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left);
        let body = wrap::lines(kit, body_style, "Compare two looks of the screen, one step at a time. Everything goes back when you leave.", inner, 6);
        let note = i.note.as_deref().map(|n| wrap::lines(kit, note_style, n, inner, 5)).unwrap_or_default();
        let (bh, side) = (Button::height(kit), crate::kit::hint::glyph_side(kit, TypeRole::Caption) as f32);
        let x = ((1920.0 - w) / 2.0).round();
        let mut y = s6;
        let title_h = kit.line(TypeRole::Title);
        let head_h = TILE.max(title_h);
        let tile_at = Frame::new(x + s6, y + ((head_h - TILE) / 2.0).round(), TILE, TILE);
        let title_at = Frame::new(x + s6 + TILE + s4, y + ((head_h - title_h) / 2.0).round(), inner - TILE - s4, title_h);
        y += head_h + s3;
        let body_at = (x + s6, y, inner);
        y += body.len() as f32 * kit.line(TypeRole::Body) + s3;
        let note_at = (x + s6, y, inner);
        if !note.is_empty() {
            y += note.len() as f32 * kit.line(TypeRole::Label) + s3;
        }
        let button = i.ready || i.refused;
        let btn_w = if button { Button::width_for(kit, if i.ready { "Start" } else { "Leave" }, false).max(240.0) } else { 0.0 };
        let btn_at = Frame::new(x + s6, y, btn_w, bh);
        if button {
            y += bh + s4;
        }
        let hints_at = (x + s6, y, side);
        y += side + s6;
        let h = y.ceil();
        let top = (SAFE_TOP + ((SAFE_H - h) / 2.0).max(0.0)).round();
        let panel_frame = Frame::new(x, top, w, h);
        let shift_y = |f: Frame| Frame::new(f.x, f.y + top, f.w, f.h);
        let panel = plate::shape(&kit.ui, panel_frame, z + 5, Self::sheet_shape(kit));
        let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let title = Text::new(kit, shift_y(title_at), z + 6, title_style, "Display lab")?;
        let tile = tile(kit, TILE as u32, "scan-eye", true).layer(kit, z + 6, Props::at(tile_at.x, tile_at.y + top))?;
        let mk = |kit: &mut Kit, at: (f32, f32, f32), texts: &[String], role, weight: Option<Weight>, colour| -> std::io::Result<Words> {
            let lines: Vec<WordLine> = texts
                .iter()
                .map(|t| {
                    let l = WordLine::new(t, role, colour);
                    match weight {
                        Some(w) => l.weight(w),
                        None => l,
                    }
                })
                .collect();
            Words::new(kit, at.0, at.1 + top, at.2, z + 6, &lines)
        };
        let body_w = mk(kit, body_at, &body, TypeRole::Body, None, ink2)?;
        let note_w = mk(kit, note_at, &note, TypeRole::Label, Some(Weight::SemiBold), ink)?;
        let mut buttons = Vec::new();
        if button {
            let (target, label, tone) = if i.ready { (Target::Start, "Start", Tone::Primary) } else { (Target::Leave, "Leave", Tone::Plain) };
            let mut b = Button::new(
                kit,
                engine,
                Place { id: id_of(target), frame: shift_y(btn_at), region: REGION, scope: SCOPE, z: z + 7 },
                ButtonSpec::new(label, tone),
            )?;
            b.show(kit, true);
            buttons.push((target, b));
        }
        let mut hints = Vec::new();
        let mut hx = hints_at.0;
        for hint in [Hint::key(Key::Ok, Action::Confirm), Hint::key(Key::Back, Action::Close)] {
            let row = HintRow::new(kit, &hint, TypeRole::Caption, RowAt { x: hx, y: hints_at.1 + top, h: hints_at.2, z: z + 6 }, ink2)?;
            hx += row.width() + kit.space(6);
            hints.push(row);
        }
        let empty = Words::new(kit, 0.0, 0.0, 1.0, z + 6, &[])?;
        let empty2 = Words::new(kit, 0.0, 0.0, 1.0, z + 6, &[])?;
        let empty3 = Words::new(kit, 0.0, 0.0, 1.0, z + 6, &[])?;
        Ok(SheetBuilt {
            panel_frame,
            _panel: panel,
            _tile: tile,
            title,
            title_style,
            body: body_w,
            note: note_w,
            rows_left: empty,
            rows_right: empty2,
            saved: empty3,
            buttons,
            _hints: hints,
            layers: 0,
        })
    }

    fn build_results(kit: &mut Kit, engine: &mut Engine, z: i32, r: &ResultsView) -> std::io::Result<SheetBuilt> {
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (s2, s3, s4, s6) = (kit.space(2), kit.space(3), kit.space(4), pad(kit));
        let w = (820.0 * kit.text_scale.powf(0.9)).round().clamp(820.0, 1700.0);
        let inner = w - 2.0 * s6;
        let (bh, side) = (Button::height(kit), crate::kit::hint::glyph_side(kit, TypeRole::Caption) as f32);
        let x = ((1920.0 - w) / 2.0).round();
        let row_h = kit.line(TypeRole::Label) + kit.space(1);
        let mut y = s6;
        let title_h = kit.line(TypeRole::Title);
        let head_h = TILE.max(title_h);
        let tile_at = Frame::new(x + s6, y + ((head_h - TILE) / 2.0).round(), TILE, TILE);
        let title_at = Frame::new(x + s6 + TILE + s4, y + ((head_h - title_h) / 2.0).round(), inner - TILE - s4, title_h);
        y += head_h + s2;
        let rows_at = (x + s6, y);
        y += r.rows.len() as f32 * row_h + s2;
        let saved_at = (x + s6, y, inner);
        y += kit.line(TypeRole::Caption) + s3;
        let labels = ["Done", "Again"];
        let widths: Vec<f32> = labels.iter().map(|l| Button::width_for(kit, l, false).max(200.0)).collect();
        let mut bx = x + s6;
        let mut button_frames = Vec::new();
        for bw in &widths {
            button_frames.push(Frame::new(bx, y, *bw, bh));
            bx += bw + s2;
        }
        y += bh + s3;
        let hints_at = (x + s6, y, side);
        y += side + s6;
        let h = y.ceil();
        let top = (SAFE_TOP + ((SAFE_H - h) / 2.0).max(0.0)).round();
        let panel_frame = Frame::new(x, top, w, h);
        let shift_y = |f: Frame| Frame::new(f.x, f.y + top, f.w, f.h);
        let panel = plate::shape(&kit.ui, panel_frame, z + 5, Self::sheet_shape(kit));
        let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let title = Text::new(kit, shift_y(title_at), z + 6, title_style, "Results")?;
        let tile = tile(kit, TILE as u32, "list-checks", true).layer(kit, z + 6, Props::at(tile_at.x, tile_at.y + top))?;
        let left: Vec<WordLine> = r.rows.iter().map(|(a, _)| WordLine::new(a, TypeRole::Label, ink2).aligned(Align::Left)).collect();
        let right: Vec<WordLine> = r.rows.iter().map(|(_, b)| WordLine::new(b, TypeRole::Label, ink).weight(Weight::SemiBold).aligned(Align::Right)).collect();
        // each column is made of lines one row tall, so the two line up row by row
        let rows_left = Words::new(kit, rows_at.0, rows_at.1 + top, inner * 0.5, z + 6, &left)?;
        let rows_right = Words::new(kit, rows_at.0 + inner * 0.5, rows_at.1 + top, inner * 0.5, z + 6, &right)?;
        let saved = Words::new(kit, saved_at.0, saved_at.1 + top, saved_at.2, z + 6, &[WordLine::new(&r.saved, TypeRole::Caption, ink2)])?;
        let mut buttons = Vec::new();
        for ((target, label, tone), f) in [(Target::Done, "Done", Tone::Primary), (Target::Again, "Again", Tone::Plain)].into_iter().zip(button_frames) {
            let mut b =
                Button::new(kit, engine, Place { id: id_of(target), frame: shift_y(f), region: REGION, scope: SCOPE, z: z + 7 }, ButtonSpec::new(label, tone))?;
            b.show(kit, true);
            buttons.push((target, b));
        }
        let mut hints = Vec::new();
        let mut hx = hints_at.0;
        for hint in [Hint::key(Key::Ok, Action::Confirm), Hint::key(Key::Back, Action::Close)] {
            let row = HintRow::new(kit, &hint, TypeRole::Caption, RowAt { x: hx, y: hints_at.1 + top, h: hints_at.2, z: z + 6 }, ink2)?;
            hx += row.width() + kit.space(6);
            hints.push(row);
        }
        let body = Words::new(kit, 0.0, 0.0, 1.0, z + 6, &[])?;
        let note = Words::new(kit, 0.0, 0.0, 1.0, z + 6, &[])?;
        Ok(SheetBuilt {
            panel_frame,
            _panel: panel,
            _tile: tile,
            title,
            title_style,
            body,
            note,
            rows_left,
            rows_right,
            saved,
            buttons,
            _hints: hints,
            layers: 0,
        })
    }

    // ---- keys and the pointer

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue and the press follow it on the buttons; a click is the control it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let built = &mut self.built;
        let current = engine.focus.current();
        let mut set = |id: Id, f: &dyn Fn(&mut Button)| {
            let Some(t) = target_of(id) else { return };
            match built.as_mut().map(|(_, b)| b) {
                Some(Built::Step(b)) => {
                    let btn = match t {
                        Target::Chip(i) => b.chips.get_mut(i),
                        Target::Answer(i) => b.answers.get_mut(i),
                        _ => None,
                    };
                    if let Some(btn) = btn {
                        f(btn);
                    }
                }
                Some(Built::Sheet(b)) => {
                    if let Some((_, btn)) = b.buttons.iter_mut().find(|(bt, _)| *bt == t) {
                        f(btn);
                    }
                }
                None => {}
            }
        };
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| {
            match touch {
                Touch::Hover { from, to } => {
                    if let Some(f) = from {
                        set(f, &|b| b.set_hover(kit, false));
                    }
                    if let Some(t) = to.filter(|t| current != Some(*t)) {
                        set(t, &|b| b.set_hover(kit, true));
                    }
                }
                Touch::Press { id, down } => set(id, &|b| b.set_pressed(kit, down)),
            }
            None
        })?;
        target_of(click)
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.stage.drop_all(engine);
    }

    pub fn tick(&mut self, _now: u64) -> bool {
        false
    }

    /// How many layers the lab holds.
    pub fn layers(&self) -> usize {
        match self.built.as_ref().map(|(_, b)| b) {
            Some(Built::Step(b)) => b.layers + 1,
            Some(Built::Sheet(b)) => b.layers + 1,
            None => 0,
        }
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let focus = engine.focus.current().and_then(target_of).map(|t| format!("{t:?}"));
        match self.built.as_ref().map(|(_, b)| b) {
            None => Value::Null,
            Some(Built::Step(b)) => json!({
                "stage": "step", "panel": f(b.lay.panel), "zone": f(b.lay.zone), "mark": f(b.lay.mark), "tag": f(b.lay.tag), "arrow": b.lay.arrow.map(f),
                "letter": b.letter.text(), "name": b.name.text(), "variant": {"grid": b.lay.variant.grid, "chips": b.lay.variant.chips, "hint": b.lay.variant.hint, "keys": b.lay.variant.keys},
                "chips": b.chips.iter().map(|c| f(c.frame())).collect::<Vec<_>>(), "answers": b.answers.iter().map(|c| json!({"frame": f(c.frame()), "label": c.spec().label})).collect::<Vec<_>>(),
                "focus": focus, "layers": self.layers(), "scope": SCOPE.0,
            }),
            Some(Built::Sheet(b)) => json!({
                "stage": "sheet", "panel": f(b.panel_frame), "buttons": b.buttons.iter().map(|(t, c)| json!({"target": format!("{t:?}"), "frame": f(c.frame())})).collect::<Vec<_>>(),
                "focus": focus, "layers": self.layers(), "scope": SCOPE.0,
            }),
        }
    }

    /// Describes what is on the screen to a lint: the sheet, the words on it, the buttons, and the word on the frame.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some((_, built)) = &self.built else { return };
        let sheet = kit.ui.look.colour(PaletteRole::Sheet);
        match built {
            Built::Step(b) => {
                let panel = lint.panel("lab sheet", b.lay.panel, sheet, true);
                lint.word(kit, "lab title", b.title.text(), b.title.frame(), b.styles.title, Some(panel));
                lint.word(kit, "lab counter", b.counter.text(), b.counter.frame(), b.styles.counter, Some(panel));
                b.hint.feed_lint(kit, lint, "lab hint", Some(panel), (0.0, 0.0));
                b.note.feed_lint(kit, lint, "lab note", Some(panel), (0.0, 0.0));
                lint.word(kit, "lab letter", b.letter.text(), b.letter.frame(), b.styles.letter, Some(panel));
                lint.word(kit, "lab name", b.name.text(), b.name.frame(), b.styles.name, Some(panel));
                b.question.feed_lint(kit, lint, "lab question", Some(panel), (0.0, 0.0));
                for btn in b.chips.iter().chain(&b.answers) {
                    btn.feed_lint(kit, lint, panel);
                }
                // the words stand to the right of the eye: the pill, as far as they are concerned, begins a hand before them
                let pad = kit.space(4);
                let words = Frame::new(b.tag_text.frame().x - pad, b.lay.tag.y, b.lay.tag.x + b.lay.tag.w - b.tag_text.frame().x + pad, b.lay.tag.h);
                let tag = lint.panel("lab tag", words, kit.ui.look.accent(), false);
                lint.word(kit, "lab tag word", b.tag_text.text(), b.tag_text.frame(), b.tag_text_style, Some(tag));
            }
            Built::Sheet(b) => {
                let panel = lint.panel("lab sheet", b.panel_frame, sheet, true);
                lint.word(kit, "lab title", b.title.text(), b.title.frame(), b.title_style, Some(panel));
                b.body.feed_lint(kit, lint, "lab body", Some(panel), (0.0, 0.0));
                b.note.feed_lint(kit, lint, "lab note", Some(panel), (0.0, 0.0));
                b.rows_left.feed_lint(kit, lint, "lab row", Some(panel), (0.0, 0.0));
                b.rows_right.feed_lint(kit, lint, "lab verdict", Some(panel), (0.0, 0.0));
                b.saved.feed_lint(kit, lint, "lab saved", Some(panel), (0.0, 0.0));
                for (_, btn) in &b.buttons {
                    btn.feed_lint(kit, lint, panel);
                }
            }
        }
    }
}

fn step_hints() -> [Hint; 3] {
    [
        Hint { keys: vec![(Key::Left, false), (Key::Right, false)], action: Some(Action::Swap), label: None },
        Hint::key(Key::Ok, Action::Confirm),
        Hint::key(Key::Back, Action::Close),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::{Effect, Input};
    use crate::kit::testkit;
    use crate::lab::model::{Lab, TAG_BEGIN, TAG_SET};
    use crate::lab::plan::STEPS;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: LabSurface,
        m: Lab,
        t: u64,
    }

    fn session() -> Value {
        json!({"ok": true, "displaylab": {"session": {"held": true, "usable": {"sharpness": true, "window": true}, "why": {}}}})
    }

    impl Up {
        /// A lab over an app (so the window has a picture behind it), begun, on its intro.
        fn new(mode: Mode, scale: f32) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut m = Lab::new();
            m.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix"}) }, 0);
            let fx = m.open(1000);
            let mut me = Self { _h: h, kit, engine, s: LabSurface::new(crate::kit::z::LAB), m, t: 1000 };
            me.answer(fx);
            me.tick(200);
            me
        }

        /// Answers what the model asked of the helper, as a helper that works would.
        fn answer(&mut self, fx: Vec<Effect>) {
            let mut pending: Vec<(u64, u16, Value)> = fx
                .iter()
                .filter_map(|e| match e {
                    Effect::Request { tag, .. } if *tag == TAG_BEGIN => Some((*tag, 200, session())),
                    Effect::Request { tag, .. } if *tag == TAG_SET => Some((*tag, 200, session())),
                    Effect::Request { tag, .. } => Some((*tag, 200, json!({"ok": true}))),
                    _ => None,
                })
                .collect();
            while let Some((tag, status, data)) = pending.pop() {
                let more = self.m.update(&Input::Fetched { tag, status, data }, self.t);
                pending.extend(more.iter().filter_map(|e| match e {
                    Effect::Request { tag, .. } if *tag == TAG_BEGIN || *tag == TAG_SET => Some((*tag, 200, session())),
                    Effect::Request { tag, .. } => Some((*tag, 200, json!({"ok": true}))),
                    _ => None,
                }));
            }
            self.apply();
        }

        fn tick(&mut self, ms: u64) {
            self.t += ms;
            let fx = self.m.update(&Input::Tick, self.t);
            self.answer(fx);
        }

        /// What the shell does after every input: draw the view, or let go of what was drawn when there is none.
        fn apply(&mut self) {
            match self.m.view() {
                Some(v) => {
                    self.s.apply(&mut self.kit, &mut self.engine, &v, self.t * 1_000_000).expect("the lab is drawn");
                }
                None => self.s.drop_all(&mut self.engine),
            }
        }

        fn key(&mut self, name: &str) {
            self.t += 40;
            self.s.on_key(&mut self.engine);
            let fx = self.m.update(&Input::Key { name: name.into(), down: true, repeat: false }, self.t);
            self.answer(fx);
        }

        fn click(&mut self, t: Target) {
            self.t += 40;
            let fx = self.m.click(t, self.t);
            self.answer(fx);
        }

        /// To step `n` (0-based): Start, then Skip on every step before it.
        fn go_to_step(&mut self, n: usize) {
            self.key("ok");
            for _ in 0..n {
                let skip = crate::lab::model::replies(self.m.step().unwrap().ask).len() - 1;
                self.click(Target::Answer(skip));
            }
            assert_eq!(self.m.step().map(|s| s.id), Some(STEPS[n].id));
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new().with_safe(Frame::new(64.0, 54.0, 1792.0, 972.0));
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }

        fn geometry(&self) -> Value {
            self.s.geometry(&self.engine)
        }
    }

    fn at(x: f32, y: f32, b: u32) -> Sample {
        Sample { x, y, buttons: b, visible: true }
    }

    #[test]
    fn a_step_is_a_card_with_its_zone_framed_and_one_sheet_beside_it_with_the_option_in_big_letters() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(0);
        let g = u.geometry();
        assert_eq!(g["stage"], "step");
        let (panel, mark, zone) = (&g["panel"], &g["mark"], &g["zone"]);
        assert_eq!(
            (panel["x"].as_f64().unwrap() + panel["w"].as_f64().unwrap(), panel["w"].as_f64().unwrap()),
            (1824.0, 560.0),
            "the sheet is on the right of the safe area"
        );
        assert_eq!((zone["x"].as_f64().unwrap(), zone["y"].as_f64().unwrap(), zone["w"].as_f64().unwrap()), (72.0, 150.0, 760.0));
        assert!(
            mark["x"].as_f64().unwrap() < zone["x"].as_f64().unwrap() && mark["w"].as_f64().unwrap() > zone["w"].as_f64().unwrap(),
            "a frame round the zone"
        );
        assert!(g["arrow"].is_object(), "there is room for the arrow between the frame and the sheet: {}", g["arrow"]);
        assert_eq!((g["letter"].as_str(), g["name"].as_str()), (Some("A"), Some("Normal")));
        assert_eq!(g["chips"].as_array().unwrap().len(), 2);
        let answers = g["answers"].as_array().unwrap();
        assert_eq!(answers.iter().map(|a| a["label"].as_str().unwrap()).collect::<Vec<_>>(), ["This one is better", "Can't tell", "Skip"]);
        assert!(answers[0]["frame"]["y"].as_f64().unwrap() < answers[1]["frame"]["y"].as_f64().unwrap(), "one under the other");
        assert_eq!(g["focus"], "Answer(0)");
        assert_eq!(u.engine.focus.modal(), Some(SCOPE));
        // the sheet is inside the safe area and the layers are the lab's alone
        assert!(panel["y"].as_f64().unwrap() >= 54.0 && panel["y"].as_f64().unwrap() + panel["h"].as_f64().unwrap() <= 1026.0);
        assert!(u.s.layers() > 20 && u.s.layers() < 120, "{}", u.s.layers());
    }

    #[test]
    fn a_switch_changes_two_words_and_a_chip_and_makes_no_layer() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(0);
        let layers = u.kit.room().layers;
        u.key("right");
        let g = u.geometry();
        assert_eq!((g["letter"].as_str(), g["name"].as_str()), (Some("B"), Some("Strong")));
        assert_eq!(u.kit.room().layers, layers, "nothing was made");
        u.key("wheel");
        assert_eq!(u.geometry()["letter"], "A");
        u.click(Target::Chip(1));
        assert_eq!(u.geometry()["letter"], "B");
        assert_eq!(u.kit.room().layers, layers);
        // the focus flies between the answers with nothing made either
        u.key("down");
        assert_eq!(u.geometry()["focus"], "Answer(1)");
        assert!(u.engine.flow.settling_at(u.t * 1_000_000));
        assert_eq!(u.kit.room().layers, layers);
    }

    #[test]
    fn the_text_step_draws_its_card_again_when_the_look_is_switched_and_no_other_step_does() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(STEPS.len() - 1);
        u.apply();
        let sum = |u: &mut Up| match u.s.built.as_mut() {
            Some((_, Built::Step(b))) => b.card.pixels_mut().iter().map(|p| u64::from(*p)).sum::<u64>(),
            _ => panic!("a step is up"),
        };
        let a = sum(&mut u);
        u.key("right");
        u.apply();
        let b = sum(&mut u);
        assert_ne!(a, b, "B is drawn grid-fitted");
        u.key("left");
        u.apply();
        assert_eq!(sum(&mut u), a, "and A again is what it was");
        // a sharpness step is drawn with the shell's fonts however it is switched
        let mut v = Up::new(Mode::Dark, 1.0);
        v.go_to_step(0);
        v.apply();
        let before = sum(&mut v);
        v.key("right");
        v.apply();
        assert_eq!(sum(&mut v), before, "a register change is not on the card");
    }

    #[test]
    fn a_new_step_is_a_new_card_and_the_old_one_is_let_go() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(0);
        let layers = u.kit.room().layers;
        u.click(Target::Answer(2));
        assert_eq!(u.m.step().unwrap().id, "sharpness-medium");
        u.apply();
        assert!(u.kit.room().layers.abs_diff(layers) <= 2, "the step's layers replaced the step's layers: {} then {}", layers, u.kit.room().layers);
        assert!(u.engine.focus.frame(id_of(Target::Answer(0))).is_some());
    }

    #[test]
    fn every_step_the_intro_and_the_results_read_clean_in_both_materials_at_100_and_200_percent() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                let f = u.lint();
                assert!(f.is_empty(), "intro {mode:?} {scale}: {f:#?}");
                u.key("ok");
                for step in &STEPS {
                    let f = u.lint();
                    assert!(f.is_empty(), "{} {mode:?} {scale}: {f:#?}\n{}", step.id, u.geometry());
                    let p = &u.geometry()["panel"];
                    assert!(
                        p["y"].as_f64().unwrap() >= 54.0 && p["y"].as_f64().unwrap() + p["h"].as_f64().unwrap() <= 1026.5,
                        "{} {scale}: inside the safe height: {p}",
                        step.id
                    );
                    assert!(
                        p["x"].as_f64().unwrap() >= 64.0 && p["x"].as_f64().unwrap() + p["w"].as_f64().unwrap() <= 1856.0,
                        "{} {scale}: inside the safe width: {p}",
                        step.id
                    );
                    let skip = crate::lab::model::replies(u.m.step().unwrap().ask).len() - 1;
                    u.click(Target::Answer(skip));
                    if u.m.step().is_none() {
                        break;
                    }
                }
                assert_eq!(u.m.stage(), crate::lab::model::Stage::Results, "{mode:?} {scale}");
                let f = u.lint();
                assert!(f.is_empty(), "results {mode:?} {scale}: {f:#?}");
                let p = &u.geometry()["panel"];
                assert!(p["y"].as_f64().unwrap() >= 54.0 && p["y"].as_f64().unwrap() + p["h"].as_f64().unwrap() <= 1026.5, "results {scale}: {p}");
            }
        }
    }

    #[test]
    fn a_note_the_lab_has_to_make_reads_clean_too() {
        for scale in [1.0, 2.0] {
            let mut u = Up::new(Mode::Dark, scale);
            u.go_to_step(5);
            // a refused write: the helper's own sentence, as long as it can be
            let bad = json!({"ok": false, "error": {"code": "verify_failed", "message": "osd0_vsp reads 0x0 after 0x1000001 was written and it was written again and still did not read back"}});
            u.m.update(&Input::Fetched { tag: TAG_SET, status: 502, data: bad }, u.t);
            u.apply();
            let f = u.lint();
            assert!(f.is_empty(), "{scale}: {f:#?}");
            assert!(u.geometry()["panel"]["h"].as_f64().unwrap() <= 972.5);
        }
    }

    #[test]
    fn a_look_that_could_not_be_applied_shows_its_note_and_one_answer_and_reads_clean_in_both_materials_at_100_and_200_percent() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                u.go_to_step(0);
                // B is asked for and the request is left unanswered by the rig, which would say it holds
                u.m.update(&Input::Key { name: "right".into(), down: true, repeat: false }, u.t);
                // the helper wrote B, read it back, and the firmware had taken it back by its answer
                let gone = json!({"ok": true, "displaylab": {"session": {"held": false, "heldBy": {"sharpness": false}, "drifted": ["osd_sr_gain"], "usable": {"sharpness": true, "window": true}, "why": {}}}});
                u.m.update(&Input::Fetched { tag: TAG_SET, status: 200, data: gone }, u.t);
                u.apply();
                let g = u.geometry();
                assert_eq!(g["answers"].as_array().unwrap().iter().map(|a| a["label"].as_str().unwrap()).collect::<Vec<_>>(), ["Next"], "{mode:?} {scale}");
                assert_eq!(g["focus"], "Answer(0)");
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
                let p = &g["panel"];
                assert!(p["y"].as_f64().unwrap() >= 54.0 && p["y"].as_f64().unwrap() + p["h"].as_f64().unwrap() <= 1026.5, "{mode:?} {scale}: {p}");
            }
        }
    }

    #[test]
    fn at_large_text_the_sheet_gives_up_the_chips_and_two_to_a_row_before_it_leaves_the_screen() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(3);
        let v = u.geometry()["variant"].clone();
        assert_eq!(v, json!({"grid": false, "chips": true, "hint": true, "keys": true}), "all of it at 100 %");
        let mut big = Up::new(Mode::Dark, 2.0);
        big.go_to_step(3);
        let v = big.geometry()["variant"].clone();
        assert_eq!(v["grid"], true, "the answers go two to a row at 200 %: {v}");
        assert!(big.geometry()["panel"]["h"].as_f64().unwrap() <= 972.5);
    }

    #[test]
    fn the_window_step_keeps_the_sheet_clear_of_the_window_at_100_percent() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(5);
        let g = u.geometry();
        assert_eq!(g["zone"], json!({"x": 720.0, "y": 405.0, "w": 480.0, "h": 270.0}));
        assert!(g["panel"]["x"].as_f64().unwrap() >= 1200.0, "the hole would cut the sheet: {}", g["panel"]);
        assert!(g["arrow"].is_null(), "no room for an arrow beside a frame that is nearly at the sheet");
    }

    #[test]
    fn the_pointer_cue_follows_it_and_a_click_names_the_control_it_ended_on() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(0);
        let f = u.engine.focus.frame(id_of(Target::Answer(1))).expect("a button");
        let t = |n: u64| (9000 + n) * 1_000_000;
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(f.x + 10.0, f.y + 10.0, 0)), None, "hover is a cue: no click");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(f.x + 10.0, f.y + 10.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(f.x + 10.0, f.y + 10.0, 0)), Some(Target::Answer(1)));
        let chip = u.engine.focus.frame(id_of(Target::Chip(1))).expect("a chip");
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(chip.x + 10.0, chip.y + 10.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(40), at(chip.x + 10.0, chip.y + 10.0, 0)), Some(Target::Chip(1)));
        // the card is no control
        u.s.pointer(&u.kit, &mut u.engine, t(50), at(200.0, 400.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(60), at(200.0, 400.0, 0)), None);
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Start, Target::Leave, Target::Chip(0), Target::Chip(1), Target::Answer(0), Target::Answer(3), Target::Done, Target::Again] {
            assert_eq!(target_of(id_of(t)), Some(t));
        }
        assert_eq!(target_of(Id(1)), None);
        assert_eq!(target_of(Id(BASE + 0x12)), None);
    }

    #[test]
    fn the_intro_and_the_results_are_one_sheet_with_their_buttons() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let g = u.geometry();
        assert_eq!(g["stage"], "sheet");
        assert_eq!(g["buttons"].as_array().unwrap().iter().map(|b| b["target"].as_str().unwrap()).collect::<Vec<_>>(), ["Start"]);
        assert_eq!(g["focus"], "Start");
        u.go_to_step(0);
        for _ in 0..STEPS.len() {
            if u.m.step().is_none() {
                break;
            }
            let skip = crate::lab::model::replies(u.m.step().unwrap().ask).len() - 1;
            u.click(Target::Answer(skip));
        }
        let g = u.geometry();
        assert_eq!(g["buttons"].as_array().unwrap().iter().map(|b| b["target"].as_str().unwrap()).collect::<Vec<_>>(), ["Done", "Again"]);
        assert_eq!(g["focus"], "Done");
        u.key("down");
        assert_eq!(u.geometry()["focus"], "Again");
    }

    #[test]
    fn a_lab_that_cannot_run_offers_leaving_and_says_why() {
        let (h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let mut engine = Engine::new(&kit).unwrap();
        let mut m = Lab::new();
        m.open(1000);
        m.update(
            &Input::Fetched {
                tag: TAG_BEGIN,
                status: 503,
                data: json!({"error": {"code": "unavailable", "message": "not an LG set: /proc/lg/fbdev/chip/osd_sr_gain is missing"}}),
            },
            1010,
        );
        let mut s = LabSurface::new(crate::kit::z::LAB);
        s.apply(&mut kit, &mut engine, &m.view().unwrap(), 2_000_000_000).unwrap();
        let g = s.geometry(&engine);
        assert_eq!(g["buttons"].as_array().unwrap().iter().map(|b| b["target"].as_str().unwrap()).collect::<Vec<_>>(), ["Leave"]);
        assert_eq!(g["focus"], "Leave");
        let mut lint = Lint::new().with_safe(Frame::new(64.0, 54.0, 1792.0, 972.0));
        s.feed_lint(&mut kit, &mut lint);
        assert!(lint.findings().is_empty(), "{:?}", lint.findings().iter().map(ToString::to_string).collect::<Vec<_>>());
        drop(h);
    }

    #[test]
    fn leaving_frees_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.go_to_step(0);
        let with = u.kit.room().layers;
        u.key("back");
        u.apply();
        assert_eq!(u.engine.focus.modal(), Some(SCOPE), "the keys stay while it fades out");
        u.tick(200);
        u.apply();
        u.tick(10);
        assert!(u.m.view().is_none());
        u.s.drop_all(&mut u.engine);
        assert_eq!(u.engine.focus.modal(), None);
        assert!(u.kit.room().layers > with + 20 && !u.s.is_up());
    }
}

#[cfg(test)]
mod probe {
    use super::*;
    use crate::kit::testkit;
    use crate::lab::model::{OptView, replies};
    use crate::lab::plan::{Ask, STEPS};
    use ui::lucent::palette::Mode;

    #[test]
    #[ignore]
    fn heights() {
        for scale in [1.0f32, 1.5, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            for s in STEPS.iter() {
                let v = StepView {
                    id: s.id,
                    group: s.group,
                    counter: s.counter(),
                    card: s.card,
                    look: s.look,
                    hint: s.hint,
                    options: [OptView { letter: "A", name: s.a.name }, OptView { letter: "B", name: s.b.name }],
                    shown: 0,
                    fit: None,
                    question: match s.ask {
                        Ask::Compare => "Which looks better?",
                        Ask::Question(q) => q,
                    },
                    replies: replies(s.ask).to_vec(),
                    focus: 0,
                    note: None,
                };
                let mut row = format!("scale {scale} {:>18}:", s.id);
                for variant in VARIANTS {
                    let lay = step_layout(&mut kit, &v, variant);
                    row += &format!(
                        "  [{}{}{} w{} h{} hint{} q{}]",
                        variant.grid as u8,
                        variant.chips as u8,
                        variant.hint as u8 * 2 + variant.keys as u8,
                        lay.panel.w,
                        lay.panel.h,
                        lay.hint.len(),
                        lay.question.len()
                    );
                }
                println!("{row}");
            }
        }
    }
}
