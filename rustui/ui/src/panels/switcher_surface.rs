//! The app switcher's chrome drawn (`docs/mocks/switcher`, `panels/Switcher.qml`): the header with "Open apps", the count and Quit all, and under
//! each card of the row the app's icon and name, what is happening to it and, on the card in focus, its own Quit. The pictures of the apps are
//! the compositor's, drawn under the shell's layers where `switcher::placed` says; nothing here covers a picture but the ring of the card in
//! focus and the veil on the cards that are not. An app whose picture is on the video plane (it has none) is an icon on a tile in the card's
//! place. More than four cards are a row that moves by a card when the focus leaves the window, with a bar that says where the window is.
//!
//! The model decides which card, its Quit or the header's Quit all has the focus; this follows it. The cards are made when the list says
//! something else; the window, the veils and the focus move in place. Quit all asks first, in a card of its own over everything.

use super::life::Phase;
use super::stage::{Stage, Touch};
use super::switcher::{Entry, Level, View, geo};
use super::ui::{Arrive, Scroll, WordLine, Words, picture_layer};
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone, faded};
use crate::kit::flow::Lifted;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, HintRow, Key, RowAt, glyph_side};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of};
use crate::kit::widgets::edge;
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Fill, GroupPose, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(205);
const ASK_SCOPE: Scope = Scope(207);
const REGION_HEAD: Region = Region(2500);
const REGION_CARDS: Region = Region(2501);
const REGION_ASK: Region = Region(2502);
const BASE: u32 = 0x2500_0000;
const ICON: f32 = 48.0;
/// The most cards whose chrome is made: the scene has a budget, and no one keeps more apps than this open.
const MAX_CARDS: usize = 24;
const ASK_W: f32 = 880.0;
/// The cards' row shows through this window (the header's margins): a card that is leaving or arriving is cut at its edge.
const WINDOW_X: f32 = 96.0;
const WINDOW_W: f32 = 1728.0;
const RING: f32 = 7.0;

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Card(usize),
    Quit(usize),
    All,
    Cancel,
    QuitAll,
    Backdrop,
}

impl Target {
    pub fn id(self) -> Id {
        Id(BASE
            + match self {
                Target::Card(i) => i as u32,
                Target::Quit(i) => 0x100 + i as u32,
                Target::All => 0x1000,
                Target::Cancel => 0x1001,
                Target::QuitAll => 0x1002,
                Target::Backdrop => 0xFF00,
            })
    }

    pub fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            0x1000 => Some(Target::All),
            0x1001 => Some(Target::Cancel),
            0x1002 => Some(Target::QuitAll),
            0xFF00 => Some(Target::Backdrop),
            i if i < 0x100 => Some(Target::Card(i as usize)),
            i if (0x100..0x200).contains(&i) => Some(Target::Quit((i - 0x100) as usize)),
            _ => None,
        }
    }
}

/// One card's chrome laid out for a window that starts at the first card (the row's offset moves it).
#[derive(Clone, Debug, PartialEq)]
struct CardLay {
    pic: Frame,
    foot_icon: Frame,
    name: Vec<String>,
    name_at: (f32, f32, f32),
    state: Vec<String>,
    state_at: (f32, f32, f32),
    quit: Frame,
    quit_label: &'static str,
    /// The icon in the tile that stands in for a picture (cards with none).
    tile_icon: Frame,
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    title: Frame,
    count: Option<(String, Frame)>,
    all: Option<Frame>,
    empty: Option<(Frame, Frame)>,
    cards: Vec<CardLay>,
    bar: Option<(Frame, f32)>,
    hints: Option<Frame>,
    ask: Option<AskLay>,
}

#[derive(Clone, Debug, PartialEq)]
struct AskLay {
    card: Frame,
    title: Vec<String>,
    title_at: (f32, f32, f32),
    body: Vec<String>,
    body_at: (f32, f32, f32),
    cancel: Frame,
    quit_all: Frame,
}

fn state_words(e: &Entry) -> &'static str {
    match e.state.as_str() {
        "quitting" => "Quitting\u{2026}",
        "stuck" => "Couldn\u{2019}t quit",
        _ if e.picture == "video" || e.picture == "none" => "No preview",
        _ => "",
    }
}

fn layout(kit: &mut Kit, entries: &[Entry], asking: bool) -> Layout {
    let (s2, s3, s4, s6) = (kit.space(2), kit.space(3), kit.space(4), kit.space(6));
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let n = entries.len().min(MAX_CARDS);
    let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
    let (title_line, caption_line, body_line) = (kit.line(TypeRole::Title), kit.line(TypeRole::Caption), kit.line(TypeRole::Body));
    let btn_h = Button::height(kit);
    let head_h = kit.tokens.target.max(title_line).ceil();
    let title_w = width_of(kit, title_style, "Open apps").ceil() + 2.0;
    let title = Frame::new(WINDOW_X, 54.0 + ((head_h - title_line) / 2.0).round(), title_w, title_line);
    let count = (!entries.is_empty()).then(|| {
        let text = format!("{} open", entries.len());
        let w = width_of(kit, role_style(kit, TypeRole::Caption, None, ink2, Align::Left), &text).ceil() + 2.0;
        (text, Frame::new(WINDOW_X + title_w + s4, title.y + title_line - caption_line - (title_line * 0.12).round(), w, caption_line))
    });
    let all = (!entries.is_empty()).then(|| {
        let w = Button::width_for(kit, "Quit all", true);
        Frame::new(WINDOW_X + WINDOW_W - w, 54.0 + ((head_h - btn_h) / 2.0).round(), w, btn_h)
    });
    let empty = entries.is_empty().then(|| {
        let w: f32 = 1000.0;
        (Frame::new(((1920.0 - w) / 2.0).round(), 380.0, w, title_line), Frame::new(((1920.0 - w) / 2.0).round(), 380.0 + title_line + s3, w, kit.line(TypeRole::Label)))
    });
    let name_style = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
    let caption_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    let mut cards = Vec::new();
    for (i, e) in entries.iter().take(n).enumerate() {
        let (x, y, w, h, _) = geo::card(i, entries.len(), 0);
        let foot_y = y + h + s4 + 2.0;
        let name_x = x + ICON + s4;
        let name_w = w - ICON - s4;
        let name = wrap::lines(kit, name_style, &e.title, name_w, 3);
        let state_text = state_words(e);
        let state = if state_text.is_empty() { Vec::new() } else { wrap::lines(kit, caption_style, state_text, name_w, 2) };
        let name_h = name.len() as f32 * body_line;
        let words_h = name_h + state.len() as f32 * caption_line;
        let foot_h = words_h.max(ICON);
        let quit_label = if e.state == "stuck" { "Try again" } else { "Quit" };
        let qw = Button::width_for(kit, quit_label, true);
        cards.push(CardLay {
            pic: Frame::new(x, y, w, h),
            foot_icon: Frame::new(x, foot_y + ((foot_h - ICON) / 2.0).round().min(0.0), ICON, ICON),
            name_at: (name_x, foot_y, name_w),
            name,
            state_at: (name_x, foot_y + name_h, name_w),
            state,
            quit: Frame::new(x, foot_y + foot_h + s3, qw, btn_h),
            quit_label,
            tile_icon: Frame::new(x + (w - 104.0) / 2.0, y + (h - 104.0) / 2.0, 104.0, 104.0),
        });
    }
    let bar = (entries.len() > geo::VISIBLE).then(|| (Frame::new(WINDOW_X, 830.0, WINDOW_W, 8.0), (WINDOW_W * geo::VISIBLE as f32 / entries.len() as f32).round()));
    let hints = (!entries.is_empty()).then(|| {
        let hs = [Hint::key(Key::Ok, Action::Open), Hint::key(Key::Back, Action::Close)];
        let w = super::ui::hints_width(kit, &hs, TypeRole::Label, kit.space(7)) + 2.0 * s6;
        let h = kit.tokens.target.max(glyph_side(kit, TypeRole::Label) as f32 + s3).ceil();
        Frame::new(1920.0 - 96.0 - w, 1080.0 - 54.0 - h, w, h)
    });
    let ask = asking.then(|| {
        let s8 = kit.space(8);
        let inner = ASK_W - 96.0;
        let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
        let title = wrap::lines(kit, title_style, &format!("Quit all {} apps?", entries.len()), inner, 3);
        let body = wrap::lines(kit, body_style, "Everything that is open closes. Anything unsaved in them is lost.", inner, 6);
        let (qw, cw) = (Button::width_for(kit, "Quit all", false), Button::width_for(kit, "Cancel", false));
        let title_h = title.len() as f32 * title_line;
        let body_h = body.len() as f32 * body_line;
        let h = (s8 + title_h + s3 + body_h + s4 + btn_h + s8).ceil();
        let card = Frame::new(((1920.0 - ASK_W) / 2.0).round(), ((1080.0 - h) / 2.0).round(), ASK_W, h);
        let by = card.y + h - s8 - btn_h;
        let right = card.x + ASK_W - 48.0;
        let _ = s2;
        AskLay {
            title_at: (card.x + 48.0, card.y + s8, inner),
            title,
            body_at: (card.x + 48.0, card.y + s8 + title_h + s3, inner),
            body,
            quit_all: Frame::new(right - qw, by, qw, btn_h),
            cancel: Frame::new(right - qw - kit.space(5) - cw, by, cw, btn_h),
            card,
        }
    });
    Layout { title, count, all, empty, cards, bar, hints, ask }
}

/// The ring of a card in focus: a hollow outline just outside the picture, in the full ink (3:1 on any picture and on the dim), carried by
/// the flowing focus like any lit plate: it fades from the card the focus left to the one it came to.
struct Ring {
    id: Id,
    _shape: ShapeLayer,
}

impl Ring {
    fn new(kit: &mut Kit, engine: &mut Engine, parent: &Group, id: Id, f: Frame, radius: f32, z: i32) -> std::io::Result<Self> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let rest = GroupPose { x: f.x, y: f.y, pivot_x: f.w / 2.0, pivot_y: f.h / 2.0, ..GroupPose::IDENTITY };
        let group = kit.ui.scene.group(Some(parent.id()), rest).map_err(other)?;
        let lit = kit.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let scene = kit.ui.scene.clone();
        let look = Shape { corner_radius: radius + RING, fill: Fill::Solid(Rgba(0, 0, 0, 0)), border_width: 3.0, border: kit.ui.look.ink() };
        let shape = scene.adopting(&lit, || kit.ui.scene.shape((f.w + 2.0 * RING) as u32, (f.h + 2.0 * RING) as u32, z, look, Props::at(-RING, -RING)));
        engine.focus.register(id, f, REGION_CARDS, SCOPE);
        engine.flow.lift(id, Lifted { group, rest, scale: 1.0, rise: 0.0, lit: Some(lit) });
        Ok(Self { id, _shape: shape })
    }

    fn destroy(self, engine: &mut Engine) {
        engine.focus.unregister(self.id);
        engine.flow.drop_lift(self.id);
    }
}

struct CardParts {
    ring: Ring,
    veil: ShapeLayer,
    _tile: Option<(ShapeLayer, Option<ImageLayer>)>,
    _icon: Option<ImageLayer>,
    name: Words,
    state: Option<Words>,
    quit: Button,
    lay: CardLay,
    entry: Entry,
    /// The card is the one in focus (or its Quit is): its veil is off, its Quit shown.
    current: Option<bool>,
}

struct AskParts {
    _veil: ShapeLayer,
    _card: ShapeLayer,
    title: Words,
    body: Words,
    cancel: Button,
    quit_all: Button,
    lay: AskLay,
}

struct Built {
    sig: String,
    scroll: Scroll,
    _title: Words,
    count: Option<(Words, String)>,
    all: Option<Button>,
    empty: Option<(Words, Words)>,
    cards: Vec<CardParts>,
    bar: Option<(ShapeLayer, ShapeLayer, Frame, f32)>,
    _hints_pill: Option<ShapeLayer>,
    _hints: Vec<HintRow>,
    layers: usize,
    /// The first card of the window as drawn.
    first: Option<usize>,
}

pub struct SwitcherSurface {
    stage: Stage,
    built: Option<Built>,
    ask: Option<AskParts>,
    ask_modal: bool,
}

fn ring_of(c: &CardLay) -> f32 {
    let _ = c;
    geo::RADIUS
}

impl SwitcherSurface {
    /// `z` is the panels' band base.
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None, ask: None, ask_modal: false }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn signature(kit: &Kit, entries: &[Entry]) -> String {
        format!("{}|{}|{}", kit.generation(), kit.text_scale, entries.iter().map(|e| format!("{}|{}|{}|{}|{}", e.id, e.title, e.icon, e.picture, e.state)).collect::<Vec<_>>().join("/"))
    }

    fn veil_colour(kit: &Kit) -> Rgba {
        faded(kit.tokens.extra(kit.mode).scrim_hide, if kit.mode == ui::lucent::palette::Mode::Light { 0.47 } else { 0.41 })
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, entries: &[Entry], lay: Layout, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let n = entries.len();
        let content_w = WINDOW_W + (n.saturating_sub(geo::VISIBLE)) as f32 * (geo::W + geo::GAP);
        let scroll = Scroll::row(kit, root, Frame::new(WINDOW_X, 0.0, WINDOW_W, 1080.0), content_w)?;
        let veil_look = plate::solid(Self::veil_colour(kit), geo::RADIUS);
        let raised = kit.ui.look.colour(PaletteRole::GlassRaised);
        let edge_c = edge(kit);
        let (title, count, all, empty) = scene.adopting_view(root, || -> std::io::Result<_> {
            let title = Words::new(kit, lay.title.x, lay.title.y, lay.title.w, z + 3, &[WordLine::new("Open apps", TypeRole::Title, ink).weight(Weight::SemiBold)])?;
            let count = match &lay.count {
                Some((text, f)) => Some((Words::new(kit, f.x, f.y, f.w, z + 3, &[WordLine::new(text, TypeRole::Caption, ink2)])?, text.clone())),
                None => None,
            };
            let all = match lay.all {
                Some(f) => {
                    let mut b = Button::new(kit, engine, Place { id: Target::All.id(), frame: f, region: REGION_HEAD, scope: SCOPE, z: z + 3 }, ButtonSpec::new("Quit all", Tone::Plain).with_icon("x"))?;
                    b.show(kit, true);
                    Some(b)
                }
                None => None,
            };
            let empty = match &lay.empty {
                Some((a, b)) => Some((
                    Words::new(kit, a.x, a.y, a.w, z + 3, &[WordLine::new("No apps open", TypeRole::Title, ink).weight(Weight::SemiBold).aligned(Align::Centre)])?,
                    Words::new(kit, b.x, b.y, b.w, z + 3, &[WordLine::new("Apps you open stay here until you quit them.", TypeRole::Label, ink2).aligned(Align::Centre)])?,
                )),
                None => None,
            };
            Ok((title, count, all, empty))
        })?;
        let mut cards = Vec::new();
        for (i, (e, cl)) in entries.iter().zip(lay.cards.iter()).enumerate() {
            let p = cl.pic;
            let ring = Ring::new(kit, engine, &scroll.inner, Target::Card(i).id(), p, ring_of(cl), z + 6)?;
            let (tile, icon, veil, name, state, quit) = scene.adopting_view(&scroll.inner, || -> std::io::Result<_> {
                // an app with no picture of its own to show: its icon on a tile
                let tile = if e.picture == "live" {
                    None
                } else {
                    let bg = plate::shape(&kit.ui, p, z + 1, Shape { corner_radius: geo::RADIUS, fill: Fill::Solid(raised), border_width: 1.0, border: edge_c });
                    let big = match picture_layer(kit, &e.icon, 104, 104, z + 2, Props::at(cl.tile_icon.x, cl.tile_icon.y))? {
                        Some(l) => Some(l),
                        None => icon::layer(kit, "layout-grid", 64, ink2, z + 2, Props::at(cl.tile_icon.x + 20.0, cl.tile_icon.y + 20.0))?,
                    };
                    Some((bg, big))
                };
                let veil = plate::shape(&kit.ui, p, z + 4, veil_look);
                // the app's own icon, or the generic one when it has no file: the name never stands alone in a gap
                let icon = match picture_layer(kit, &e.icon, ICON as u32, ICON as u32, z + 3, Props::at(cl.foot_icon.x, cl.foot_icon.y))? {
                    Some(l) => Some(l),
                    None => icon::layer(kit, "layout-grid", 32, ink2, z + 3, Props::at(cl.foot_icon.x + 8.0, cl.foot_icon.y + 8.0))?,
                };
                let lines: Vec<WordLine> = cl.name.iter().map(|t| WordLine::new(t, TypeRole::Body, ink).weight(Weight::SemiBold)).collect();
                let name = Words::new(kit, cl.name_at.0, cl.name_at.1, cl.name_at.2, z + 3, &lines)?;
                let state = if cl.state.is_empty() {
                    None
                } else {
                    let ink = if e.state == "quitting" { ink } else { ink2 };
                    let l: Vec<WordLine> = cl.state.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink)).collect();
                    Some(Words::new(kit, cl.state_at.0, cl.state_at.1, cl.state_at.2, z + 3, &l)?)
                };
                let mut quit = Button::new(kit, engine, Place { id: Target::Quit(i).id(), frame: cl.quit, region: REGION_CARDS, scope: SCOPE, z: z + 5 }, ButtonSpec::new(cl.quit_label, Tone::Plain).with_icon("x"))?;
                quit.set_enabled(kit, engine, false);
                Ok((tile, icon, veil, name, state, quit))
            })?;
            cards.push(CardParts { ring, veil, _tile: tile, _icon: icon, name, state, quit, lay: cl.clone(), entry: e.clone(), current: None });
        }
        let (bar, pill, hints) = scene.adopting_view(root, || -> std::io::Result<_> {
            let bar = lay.bar.map(|(track, thumb_w)| {
                let t = plate::shape(&kit.ui, track, z + 2, plate::solid(kit.ui.look.theme.materials.track_off, 4.0));
                let th = plate::shape(&kit.ui, Frame::new(track.x, track.y, thumb_w, track.h), z + 3, plate::solid(ink2, 4.0));
                (t, th, track, thumb_w)
            });
            let mut hints = Vec::new();
            let pill = match lay.hints {
                Some(f) => {
                    let pill = plate::shape(&kit.ui, f, z + 2, plate::solid(faded(kit.tokens.extra(kit.mode).scrim_hide, 0.88), (f.h / 2.0).floor()));
                    let mut x = f.x + kit.space(6);
                    for h in [Hint::key(Key::Ok, Action::Open), Hint::key(Key::Back, Action::Close)] {
                        let row = HintRow::new(kit, &h, TypeRole::Label, RowAt { x, y: f.y, h: f.h, z: z + 3 }, ink)?;
                        x += row.width() + kit.space(7);
                        hints.push(row);
                    }
                    Some(pill)
                }
                None => None,
            };
            Ok((bar, pill, hints))
        })?;
        let layers = before.saturating_sub(kit.room().layers);
        Ok(Built { sig, scroll, _title: title, count, all, empty, cards, bar, _hints_pill: pill, _hints: hints, layers, first: None })
    }

    fn build_ask(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, a: AskLay) -> std::io::Result<AskParts> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2, sheet) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.colour(PaletteRole::Sheet));
        let z = z + 20;
        let shape = Shape { corner_radius: kit.tokens.radius(Radius::Sheet), fill: Fill::Solid(sheet), border_width: 1.0, border: edge(kit) };
        scene.adopting_view(root, || -> std::io::Result<AskParts> {
            // the question hides what is behind it, the pictures too
            let veil = crate::panels::ui::scrim(kit, z, true);
            crate::panels::ui::scrim_set(&veil, true);
            let card = plate::shape(&kit.ui, a.card, z + 1, shape);
            let tl: Vec<WordLine> = a.title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
            let title = Words::new(kit, a.title_at.0, a.title_at.1, a.title_at.2, z + 2, &tl)?;
            let bl: Vec<WordLine> = a.body.iter().map(|t| WordLine::new(t, TypeRole::Body, ink2)).collect();
            let body = Words::new(kit, a.body_at.0, a.body_at.1, a.body_at.2, z + 2, &bl)?;
            let mut cancel = Button::new(kit, engine, Place { id: Target::Cancel.id(), frame: a.cancel, region: REGION_ASK, scope: ASK_SCOPE, z: z + 3 }, ButtonSpec::new("Cancel", Tone::Primary))?;
            cancel.show(kit, true);
            let mut quit_all = Button::new(kit, engine, Place { id: Target::QuitAll.id(), frame: a.quit_all, region: REGION_ASK, scope: ASK_SCOPE, z: z + 3 }, ButtonSpec::new("Quit all", Tone::Danger))?;
            quit_all.show(kit, true);
            Ok(AskParts { _veil: veil, _card: card, title, body, cancel, quit_all, lay: a.clone() })
        })
        .map_err(std::io::Error::other)
    }

    fn drop_ask(&mut self, engine: &mut Engine) {
        if let Some(a) = self.ask.take() {
            a.cancel.destroy(engine);
            a.quit_all.destroy(engine);
        }
        if self.ask_modal {
            engine.focus.close_modal();
            self.ask_modal = false;
        }
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        self.drop_ask(engine);
        if let Some(b) = self.built.take() {
            for c in b.cards {
                c.ring.destroy(engine);
                c.quit.destroy(engine);
            }
            if let Some(a) = b.all {
                a.destroy(engine);
            }
        }
    }

    /// Fits the chrome to the view and to the phase of its life. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, phase: Phase, now_ns: u64) -> std::io::Result<bool> {
        if phase == Phase::Gone {
            self.drop_all(engine);
            return Ok(false);
        }
        let up = matches!(phase, Phase::Opening | Phase::Open);
        if self.stage.needs_root(Arrive::Fade) {
            self.drop_built(engine);
        }
        self.stage.ensure(kit, phase, Arrive::Fade, false)?;
        let asking = v.asking && up && !v.entries.is_empty();
        let sig = Self::signature(kit, &v.entries);
        if self.built.as_ref().is_none_or(|b| b.sig != sig) {
            self.drop_built(engine);
            let lay = layout(kit, &v.entries, false);
            self.stage.backdrop(engine, Target::Backdrop.id(), REGION_HEAD);
            let built = Self::build(kit, engine, self.stage.group(), self.stage.z(), &v.entries[..v.entries.len().min(MAX_CARDS)], lay, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the app switcher could not be drawn whole"));
            }
            self.built = Some(built);
            self.stage.refocus();
        }
        if !asking && self.ask.is_some() || !up {
            self.drop_ask(engine);
        }
        if up {
            self.update(kit, engine, v, now_ns);
            if asking && self.ask.is_none() {
                let lay = layout(kit, &v.entries, true).ask.expect("the question's layout");
                let parts = Self::build_ask(kit, engine, self.stage.group(), self.stage.z(), lay)?;
                if kit.ui.scene.take_overflow() {
                    return Err(std::io::Error::other("the scene is full: the question could not be drawn"));
                }
                self.ask = Some(parts);
                engine.focus.open_modal(ASK_SCOPE);
                self.ask_modal = true;
                self.stage.refocus();
            }
        }
        if self.stage.phase(kit, engine, phase) {
            let want = if asking {
                Some(if v.ask_index == 1 { Target::QuitAll } else { Target::Cancel }.id())
            } else if v.entries.is_empty() {
                None
            } else {
                Some(
                    match v.level {
                        Level::All => Target::All,
                        Level::Quit => Target::Quit(v.index),
                        Level::Card => Target::Card(v.index),
                    }
                    .id(),
                )
            };
            self.stage.focus(engine, want, now_ns);
        }
        Ok(true)
    }

    /// What moves in place: the row's window, the veils, the Quit of the card in focus, the bar.
    fn update(&mut self, kit: &mut Kit, engine: &mut Engine, v: &View, now_ns: u64) {
        let _ = now_ns;
        let Some(b) = self.built.as_mut() else { return };
        let n = b.cards.len();
        let first = if n > geo::VISIBLE { v.first.min(n - geo::VISIBLE) } else { 0 };
        if b.first != Some(first) {
            let shift = first as f32 * (geo::W + geo::GAP);
            b.scroll.to(engine, shift, b.first.is_none());
            for (i, c) in b.cards.iter().enumerate() {
                let here = geo::in_window(i, first);
                let place = |f: Frame| if here { Frame::new(f.x - shift, f.y, f.w, f.h) } else { Frame::new(0.0, 0.0, 0.0, 0.0) };
                engine.focus.place(c.ring.id, place(c.lay.pic));
                engine.focus.place(c.quit.id, place(c.lay.quit));
            }
            if let Some((_, thumb, track, w)) = &b.bar {
                let x = track.x + (track.w * first as f32 / n.max(1) as f32).round();
                let to = Props::at(x, track.y);
                if b.first.is_none() {
                    thumb.set(to);
                } else {
                    kit.glide(thumb, to, Beat::Scroll);
                }
                let _ = w;
            }
            b.first = Some(first);
        }
        let asking = v.asking;
        for (i, c) in b.cards.iter_mut().enumerate() {
            let current = i == v.index && v.level != Level::All;
            if c.current == Some(current) && !asking {
                continue;
            }
            let dim = !current && !asking;
            let p = c.lay.pic;
            let veil = Props { opacity: if dim { 1.0 } else { 0.0 }, ..Props::at(p.x, p.y) };
            if c.current.is_none() { c.veil.set(veil) } else { kit.glide(&c.veil, veil, Beat::Focus) }
            if let Some(l) = c.name.layer() {
                let at = Props { opacity: if current { 1.0 } else { 0.72 }, ..Props::at(c.lay.name_at.0, c.lay.name_at.1) };
                if c.current.is_none() { l.set(at) } else { kit.glide(l, at, Beat::Focus) }
            }
            if c.current != Some(current) {
                if current {
                    c.quit.set_enabled(kit, engine, c.entry.state != "quitting");
                    c.quit.show(kit, true);
                    c.quit.set_dimmed(kit, c.entry.state == "quitting");
                } else {
                    c.quit.hide(kit, true);
                    c.quit.set_enabled(kit, engine, false);
                }
                c.current = Some(current);
            }
        }
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it on the pictures and the buttons, and a click is the target it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let current = engine.focus.current();
        let (built, ask) = (&mut self.built, &mut self.ask);
        let mut set = |id: Id, f: &dyn Fn(&mut Button)| match Target::from_id(id) {
            Some(Target::All) => {
                if let Some(b) = built.as_mut().and_then(|b| b.all.as_mut()) {
                    f(b);
                }
            }
            Some(Target::Quit(i)) => {
                if let Some(c) = built.as_mut().and_then(|b| b.cards.get_mut(i)) {
                    f(&mut c.quit);
                }
            }
            Some(Target::Cancel) => {
                if let Some(a) = ask.as_mut() {
                    f(&mut a.cancel);
                }
            }
            Some(Target::QuitAll) => {
                if let Some(a) = ask.as_mut() {
                    f(&mut a.quit_all);
                }
            }
            _ => {}
        };
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { from, to } => {
                if let Some(f) = from {
                    set(f, &|b| b.set_hover(kit, false));
                }
                if let Some(t) = to.filter(|t| current != Some(*t)) {
                    set(t, &|b| b.set_hover(kit, true));
                }
                to.and_then(Target::from_id).filter(|t| matches!(t, Target::Card(_))).map(|_| geo::RADIUS)
            }
            Touch::Press { id, down } => {
                set(id, &|b| b.set_pressed(kit, down));
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

    /// How many layers the chrome holds.
    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers) + self.ask.as_ref().map_or(0, |_| 9)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let cards: Vec<Value> = b
            .cards
            .iter()
            .map(|c| json!({"frame": f(engine.focus.frame(c.ring.id).unwrap_or(c.lay.pic)), "quit": f(engine.focus.frame(c.quit.id).unwrap_or(c.lay.quit)), "title": c.entry.title, "picture": c.entry.picture, "state": c.entry.state, "current": c.current}))
            .collect();
        let focus = engine.focus.current().and_then(Target::from_id).map(|t| match t {
            Target::Card(i) => json!({"card": i}),
            Target::Quit(i) => json!({"quit": i}),
            Target::All => json!("all"),
            Target::Cancel => json!("cancel"),
            Target::QuitAll => json!("quitall"),
            Target::Backdrop => json!("backdrop"),
        });
        json!({"cards": cards, "all": b.all.as_ref().map(|a| f(a.frame())), "first": b.first, "focus": focus, "asking": self.ask.is_some(), "layers": self.layers(), "scope": SCOPE.0})
    }

    /// Describes what is on the screen to a lint: the dim the compositor draws behind the chrome, the words on it, the buttons and the question.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        // the question's veil hides everything under it, the pictures too: only it is read
        if let Some(a) = &self.ask {
            let ask_dim = lint.panel("switcher question dim", Frame::new(0.0, 0.0, 1920.0, 1080.0), kit.tokens.extra(kit.mode).scrim_hide, false);
            let card = lint.panel_on("switcher question", a.lay.card, kit.ui.look.colour(PaletteRole::Sheet), false, ask_dim);
            a.title.feed_lint(kit, lint, "question title", Some(card), (0.0, 0.0));
            a.body.feed_lint(kit, lint, "question body", Some(card), (0.0, 0.0));
            a.cancel.feed_lint(kit, lint, card);
            a.quit_all.feed_lint(kit, lint, card);
            return;
        }
        let dim = lint.panel("switcher dim", Frame::new(0.0, 0.0, 1920.0, 1080.0), kit.tokens.extra(kit.mode).scrim_hide, false);
        b._title.feed_lint(kit, lint, "switcher title", Some(dim), (0.0, 0.0));
        if let Some((w, _)) = &b.count {
            w.feed_lint(kit, lint, "switcher count", Some(dim), (0.0, 0.0));
        }
        if let Some(a) = &b.all {
            a.feed_lint(kit, lint, dim);
        }
        if let Some((a, c)) = &b.empty {
            a.feed_lint(kit, lint, "switcher empty", Some(dim), (0.0, 0.0));
            c.feed_lint(kit, lint, "switcher empty line", Some(dim), (0.0, 0.0));
        }
        let shift = b.first.map_or(0.0, |f| f as f32 * (geo::W + geo::GAP));
        for (i, c) in b.cards.iter().enumerate() {
            if !b.first.is_none_or(|f| geo::in_window(i, f)) {
                continue;
            }
            c.name.feed_lint(kit, lint, &format!("card {i} name"), Some(dim), (-shift, 0.0));
            if let Some(s) = &c.state {
                s.feed_lint(kit, lint, &format!("card {i} state"), Some(dim), (-shift, 0.0));
            }
            if c.current == Some(true) && c.entry.state != "quitting" {
                c.quit.feed_lint_at(kit, lint, dim, (-shift, 0.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::switcher::Switcher;
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: SwitcherSurface,
        m: Switcher,
        t: u64,
    }

    fn app(id: &str, title: &str, picture: &str, state: &str) -> Value {
        json!({"id": id, "title": title, "picture": picture, "w": 1920, "h": 1080, "state": state})
    }

    fn three() -> Value {
        json!([app("plex", "Plex", "live", ""), app("youtube.leanback.v4", "YouTube", "live", ""), app("com.webos.app.hdmi2", "HDMI 2", "video", "")])
    }

    fn six() -> Value {
        json!([
            app("a", "Plex", "live", ""),
            app("b", "YouTube", "live", ""),
            app("c", "Netflix", "live", ""),
            app("d", "Disney+ with a long name that wraps onto lines", "live", ""),
            app("e", "Prime Video", "live", ""),
            app("f", "Spotify", "live", "")
        ])
    }

    impl Up {
        fn new(mode: Mode, scale: f32, list: Value) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            let mut m = Switcher::new();
            m.topic("apps", &list, 0);
            m.topic("switcher", &json!({"open": true, "from": "", "n": 1}), 0);
            let mut me = Self { _h: h, kit, engine, s: SwitcherSurface::new(crate::kit::z::PANELS + 40), m, t: 1000 };
            me.apply(Phase::Open);
            me
        }

        fn apply(&mut self, phase: Phase) {
            let v = self.m.view(self.t);
            self.s.apply(&mut self.kit, &mut self.engine, &v, phase, self.t * 1_000_000).expect("the chrome is drawn");
        }

        fn key(&mut self, k: K) {
            self.m.key(k, self.t);
            self.t += 40;
            self.apply(Phase::Open);
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
    fn each_card_is_where_the_compositor_draws_its_picture_and_the_first_has_the_focus() {
        let u = Up::new(Mode::Dark, 1.0, three());
        let g = u.geo();
        let cards = g["cards"].as_array().unwrap();
        assert_eq!(cards.len(), 3);
        for (i, c) in cards.iter().enumerate() {
            let (x, y, w, h, _) = geo::card(i, 3, 0);
            let f = &c["frame"];
            assert_eq!((f["x"].as_f64().unwrap() as f32, f["y"].as_f64().unwrap() as f32, f["w"].as_f64().unwrap() as f32, f["h"].as_f64().unwrap() as f32), (x, y, w, h), "card {i}");
        }
        assert_eq!(g["focus"], json!({"card": 0}));
        assert_eq!(cards[2]["picture"], "video", "the HDMI app's picture is on the video plane: its card is an icon on a tile");
        assert!(g["all"]["w"].as_f64().unwrap() > 0.0, "Quit all is in the header");
    }

    #[test]
    fn the_focus_flies_to_the_next_card_its_own_quit_and_the_headers_quit_all_without_making_a_layer() {
        let mut u = Up::new(Mode::Dark, 1.0, three());
        let layers = u.kit.room().layers;
        u.key(K::Right);
        assert_eq!(u.geo()["focus"], json!({"card": 1}));
        assert!(u.engine.flow.settling_at(u.t * 1_000_000), "the ring flows between the cards");
        u.key(K::Down);
        assert_eq!(u.geo()["focus"], json!({"quit": 1}));
        assert_eq!(u.geo()["cards"][1]["current"], true);
        assert_eq!(u.geo()["cards"][0]["current"], false);
        u.key(K::Up);
        u.key(K::Up);
        assert_eq!(u.geo()["focus"], json!("all"));
        assert_eq!(u.kit.room().layers, layers, "nothing is made or dropped by a move of the focus");
    }

    #[test]
    fn more_than_four_apps_are_a_row_that_moves_a_card_when_the_focus_leaves_the_window() {
        let mut u = Up::new(Mode::Dark, 1.0, six());
        assert_eq!(u.geo()["first"], 0);
        for _ in 0..4 {
            u.key(K::Right);
        }
        assert_eq!(u.geo()["focus"], json!({"card": 4}));
        assert_eq!(u.geo()["first"], 1, "the window followed the focus");
        let first = u.engine.focus.frame(Target::Card(1).id()).expect("a ring");
        let hidden = u.engine.focus.frame(Target::Card(0).id()).expect("a ring");
        assert!(first.w > 0.0 && hidden.w == 0.0, "a card out of the window cannot be pointed at: {first:?} {hidden:?}");
    }

    #[test]
    fn quit_all_asks_first_in_a_card_of_its_own_whose_safe_answer_has_the_focus() {
        let mut u = Up::new(Mode::Dark, 1.0, three());
        u.key(K::Up);
        u.key(K::Ok);
        let g = u.geo();
        assert_eq!(g["asking"], true);
        assert_eq!(g["focus"], json!("cancel"));
        assert_eq!(u.engine.focus.modal(), Some(ASK_SCOPE));
        u.key(K::Right);
        assert_eq!(u.geo()["focus"], json!("quitall"));
        u.key(K::Back);
        assert_eq!(u.geo()["asking"], false);
        assert_eq!(u.engine.focus.modal(), Some(SCOPE), "the question gave the keys back to the cards");
        assert_eq!(u.geo()["focus"], json!("all"), "the focus returns to the button that asked");
    }

    #[test]
    fn a_click_on_a_picture_its_quit_the_header_and_the_dim_name_what_was_hit() {
        let mut u = Up::new(Mode::Dark, 1.0, three());
        let t = |n: u64| (3000 + n) * 1_000_000;
        let (px, py, w, h, _) = geo::card(1, 3, 0);
        let (x, y) = (px + w / 2.0, py + h / 2.0);
        u.s.pointer(&u.kit, &mut u.engine, t(0), at(x, y, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(x, y, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(x, y, 0)), Some(Target::Card(1)));
        u.key(K::Down);
        let q = u.engine.focus.frame(Target::Quit(0).id()).expect("a button");
        let q = (q.x + 8.0, q.y + 8.0);
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(q.0, q.1, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(q.0, q.1, 1));
        let hit = u.s.pointer(&u.kit, &mut u.engine, t(50), at(q.0, q.1, 0));
        assert!(matches!(hit, Some(Target::Quit(_))), "{hit:?}");
        u.s.pointer(&u.kit, &mut u.engine, t(60), at(30.0, 1000.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(70), at(30.0, 1000.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(80), at(30.0, 1000.0, 0)), Some(Target::Backdrop));
    }

    #[test]
    fn what_happens_to_an_app_is_said_under_it_and_a_stuck_app_offers_another_try() {
        let list = json!([app("a", "Plex", "live", "quitting"), app("b", "YouTube", "live", "stuck"), app("c", "HDMI 2", "video", "")]);
        let mut u = Up::new(Mode::Dark, 1.0, list);
        let g = u.geo();
        assert_eq!(g["cards"][0]["state"], "quitting");
        assert_eq!(g["cards"][1]["state"], "stuck");
        u.key(K::Right);
        u.key(K::Down);
        assert_eq!(u.s.built.as_ref().unwrap().cards[1].lay.quit_label, "Try again");
        assert_eq!(u.s.built.as_ref().unwrap().cards[2].lay.state, vec!["No preview".to_string()], "a card with no picture says so");
    }

    #[test]
    fn it_reads_clean_in_both_materials_at_100_and_200_percent_with_three_apps_many_and_the_question() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for (name, list) in [("three", three()), ("six", six())] {
                    let mut u = Up::new(mode, scale, list);
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} {mode:?} {scale}: {f:#?}");
                    u.key(K::Down);
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} with Quit {mode:?} {scale}: {f:#?}");
                    u.key(K::Up);
                    u.key(K::Up);
                    u.key(K::Ok);
                    let f = u.lint();
                    assert!(f.is_empty(), "{name} asking {mode:?} {scale}: {f:#?}");
                }
                let mut none = Up::new(mode, scale, json!([]));
                let f = none.lint();
                assert!(f.is_empty(), "no apps {mode:?} {scale}: {f:#?}");
            }
        }
    }

    #[test]
    fn a_change_of_the_list_makes_the_cards_again_and_leaving_frees_the_keys_and_the_layers() {
        let mut u = Up::new(Mode::Dark, 1.0, three());
        let with = u.kit.room().layers;
        u.m.topic("apps", &json!([app("plex", "Plex", "live", "")]), 5000);
        u.t = 5000;
        u.apply(Phase::Open);
        assert_eq!(u.geo()["cards"].as_array().unwrap().len(), 1);
        u.apply(Phase::Gone);
        assert!(!u.s.is_up() && u.engine.focus.modal().is_none());
        assert!(u.kit.room().layers >= with, "everything it made is given back");
    }

    #[test]
    fn ids_name_what_the_pointer_lands_on() {
        for t in [Target::Card(0), Target::Card(23), Target::Quit(2), Target::All, Target::Cancel, Target::QuitAll, Target::Backdrop] {
            assert_eq!(Target::from_id(t.id()), Some(t));
        }
        assert_eq!(Target::from_id(Id(1)), None);
    }
}
