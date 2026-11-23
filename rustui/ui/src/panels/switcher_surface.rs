//! The task manager's chrome (`docs/research/task-manager.md`): what is drawn over the apps' pictures. The pictures are the compositor's, drawn
//! under the shell's layers where `Switcher::layout` says; here are the frame of each card (the lit glass of the one in focus, the veil on the
//! others), its icon and name, what is happening to it, an icon on a tile for an app with no picture, the header strip of the TV's numbers
//! across the top, the two buttons below the cards, the System sheet and the confirmations.
//!
//! The model decides what is where and who has the focus; this follows it. A card's chrome is made once per app and moves in place (the
//! cards close up when one leaves, the focused card is larger), so a card keeps its look while the list around it changes. Nothing here
//! repeats: the numbers update once a second as the compositor sends them, everything else moves on a key.

use super::life::Phase;
use super::stage::{Dim, Stage, Touch};
use super::switcher::{ACTIONS, Action, AskState, AskView, Confirm, Entry, Focus as Zone, Picture, Reason, Standing, Stats, View, geo};
use super::ui::{Arrive, WordLine, Words, hints_width, picture_layer, scrim, scrim_set};
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone, faded};
use crate::kit::flow::Lifted;
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action as HintAction, Hint, HintRow, Key, RowAt, glyph_side};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::pointer::Sample;
use crate::kit::text::{role_style, width_of, widest};
use crate::kit::widgets::{Lift, Plate, Spot, edge, focus_wash, lit_edge};
use crate::kit::wrap;
use crate::kit::{Engine, Kit};
use compositor_protocol::{Ease, Fill, GroupPose, Props, Rect, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::{Mode, PaletteRole};
use ui::scene::{ImageLayer, Layer, ShapeLayer};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(205);
const ASK_SCOPE: Scope = Scope(207);
const SHEET_SCOPE: Scope = Scope(213);
const REGION_LOWER: Region = Region(2500);
const REGION_CARDS: Region = Region(2501);
const REGION_ASK: Region = Region(2502);
const REGION_SHEET: Region = Region(2503);
const BASE: u32 = 0x2500_0000;
const ID_CLOSE: Id = Id(BASE + 0x1000);
const ID_SYSTEM: Id = Id(BASE + 0x1001);
const ID_BACKDROP: Id = Id(BASE + 0xFF00);
const ID_SHEET_BACK: Id = Id(BASE + 0xFF01);
const ID_ASK_BACK: Id = Id(BASE + 0xFF02);
/// The icon in a card's foot, and the room the words beside it have.
const ICON: f32 = 48.0;
/// The most cards whose chrome is made: the scene has a budget, and no one keeps more apps than this open.
const MAX_CARDS: usize = 16;
/// The page mark under the cards, when not all are in the window: how tall it is and how wide per app (a mark, not a rule across the screen).
const MARK_H: f32 = 4.0;
const MARK_PER_APP: f32 = 56.0;
/// The safe margins the header strip stands inside (the buttons' left edge is the same), and the width between them.
const EDGE_X: f32 = 96.0;
const EDGE_Y: f32 = 54.0;
const HEAD_W: f32 = 1920.0 - 2.0 * EDGE_X;
/// How far up a card has to be dragged for it to be quit.
const DRAG_PX: f32 = 120.0;
const FULL: Frame = Frame { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };

/// What the pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Card(usize),
    Close,
    System,
    Row(usize),
    Choice(usize),
    /// A card dragged up past the threshold.
    Drag(usize),
    /// The dim behind the sheet or a confirmation.
    Outside,
}

fn card_id(serial: u32) -> Id {
    Id(BASE + serial)
}

fn row_id(r: usize) -> Id {
    Id(BASE + 0x1100 + r as u32)
}

fn choice_id(c: usize) -> Id {
    Id(BASE + 0x1200 + c as u32)
}

fn state_words(s: Standing) -> &'static str {
    match s {
        Standing::Fine => "",
        Standing::Hung => "Not responding",
        Standing::Closing => "Closing",
        Standing::Stuck => "Didn\u{2019}t quit",
    }
}

/// What a System choice does, in the plain words of the sheet: its glyph, its name and what follows from it.
fn choice_words(a: Action) -> (&'static str, &'static str, &'static str) {
    match a {
        Action::RestartLumatv => ("refresh-cw", "Restart LumaTV", "Closes open apps and restarts the interface."),
        Action::LgHome => ("house", "Bring back LG Home", "LumaTV stays off until the TV restarts."),
        Action::Reboot => ("power", "Restart the TV", "Closes everything and reboots the TV."),
    }
}

const REFRESHER: &str = "The panel refresher is running; try again when it is done.";

/// A confirmation's title, its body and the name of its one destructive choice.
fn ask_words(a: &AskView) -> (String, &'static str, &'static str) {
    let (title, body, go) = match a.kind {
        Confirm::CloseAll(n) => (format!("Close all {n} apps?"), "Everything that is open closes. Anything unsaved in them is lost.", "Close all"),
        Confirm::System(Action::RestartLumatv) => ("Restart LumaTV?".to_string(), "Open apps close, then the interface restarts. Anything unsaved in them is lost.", "Restart"),
        Confirm::System(Action::LgHome) => ("Bring back LG Home?".to_string(), "LumaTV turns off until the next restart. LG\u{2019}s own Home takes over.", "Bring back"),
        Confirm::System(Action::Reboot) => ("Restart the TV?".to_string(), "The TV reboots and every open app closes. Anything unsaved is lost.", "Restart TV"),
    };
    let body = match (a.state, a.kind) {
        (AskState::Asking, _) => body,
        (AskState::Working, Confirm::System(Action::LgHome)) => "Switching to LG Home\u{2026}",
        (AskState::Working, Confirm::System(Action::Reboot)) => "Restarting the TV\u{2026}",
        (AskState::Working, _) => "Restarting LumaTV\u{2026}",
        (AskState::Refused(Reason::Refresher), _) => REFRESHER,
        (AskState::Refused(Reason::Busy), _) => "Another system action is already running.",
        (AskState::Refused(Reason::Other), _) => "That did not start. Try again.",
    };
    (title, body, go)
}

/// Where everything stands for the look it is drawn in: none of it depends on the apps, so the compositor can be told where the row of
/// cards is before there is a card.
#[derive(Clone, Copy, Debug)]
struct Metrics {
    /// The top of the row of cards.
    top: f32,
    /// Room under a card for its name (two lines) and what is happening to it.
    foot_h: f32,
    name_w: f32,
    /// The words go under the icon, not beside it: at large text the name beside the icon is a column too narrow for one word.
    stacked: bool,
    btn_h: f32,
    action_y: f32,
    /// The header strip of numbers: its top and height, one line of small type.
    head_y: f32,
    head_h: f32,
    /// The area between the header and the buttons that the row of cards and the empty state are centred in.
    zone_top: f32,
    zone_bottom: f32,
}

fn metrics(kit: &mut Kit) -> Metrics {
    let (s2, s3, s4, s7, s8) = (kit.space(2), kit.space(3), kit.space(4), kit.space(7), kit.space(8));
    let btn_h = Button::height(kit);
    let action_y = 1080.0 - EDGE_Y - btn_h;
    let head_h = kit.line(TypeRole::Caption).ceil();
    let stacked = kit.line(TypeRole::Body) > ICON;
    // the state words wrap to a second line where the card is narrower than they are, which only large text makes it
    let words_h = 2.0 * kit.line(TypeRole::Body) + if stacked { 2.0 } else { 1.0 } * kit.line(TypeRole::Caption);
    let foot_h = if stacked { ICON + s2 + words_h } else { words_h.max(ICON) }.ceil();
    let cards_h = geo::H + s4 + foot_h + s8 + s2;
    let (zone_top, zone_bottom) = (EDGE_Y + head_h + s7, action_y - s7);
    let top = (zone_top + ((zone_bottom - zone_top - cards_h) / 2.0).max(0.0)).round();
    let rest_w = geo::W * geo::REST;
    let name_w = if stacked { rest_w } else { rest_w - ICON - s3 };
    Metrics { top, foot_h, name_w, stacked, btn_h, action_y, head_y: EDGE_Y, head_h, zone_top, zone_bottom }
}

/// The width of the "No preview" line on a tile: the resting picture less its margins.
fn note_w(kit: &Kit) -> f32 {
    (geo::W * geo::REST - 2.0 * kit.space(4)).floor()
}

/// The focus of a card is raised glass (Lucent "Focus": no outline, no underline, no landing bar): over the picture of the card in focus
/// the focus wash with the sheen (light from the upper left) and the lit rim, on the picture's own frame. The flowing focus carries them
/// like any lit plate: they fade from the card the focus left to the one it came to. What grows and loses its veil is the picture itself
/// (`geo::pose`); it is the compositor's, so it neither rises nor casts a shadow from here.
struct Lit {
    id: Id,
    _layers: [ShapeLayer; 2],
    released: bool,
}

impl Lit {
    fn new(kit: &mut Kit, engine: &mut Engine, parent: &Group, id: Id, f: Frame, radius: f32, z: i32) -> std::io::Result<Self> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let rest = Self::rest(f);
        let group = kit.ui.scene.group(Some(parent.id()), rest).map_err(other)?;
        let lit = kit.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let scene = kit.ui.scene.clone();
        let (w, h) = (f.w.round() as u32, f.h.round() as u32);
        let sheen = kit.ui.look.theme.fields.sheen;
        let clear = Rgba(0, 0, 0, 0);
        let wash = Shape { corner_radius: radius, fill: Fill::Solid(focus_wash(kit)), border_width: 0.0, border: clear };
        let glint = Fill::Linear { from: faded(Rgba(255, 255, 255, 255), sheen.peak), to: clear, angle_deg: 45.0 };
        let rim = Shape { corner_radius: radius, fill: glint, border_width: 1.0, border: lit_edge(kit) };
        let layers = scene.adopting(&lit, || [kit.ui.scene.shape(w, h, z, wash, Props::at(0.0, 0.0)), kit.ui.scene.shape(w, h, z + 1, rim, Props::at(0.0, 0.0))]);
        engine.focus.register(id, f, REGION_CARDS, SCOPE);
        engine.flow.lift(id, Lifted { group, rest, scale: 1.0, rise: 0.0, lit: Some(lit) });
        Ok(Self { id, _layers: layers, released: false })
    }

    fn rest(f: Frame) -> GroupPose {
        GroupPose { x: f.x, y: f.y, pivot_x: f.w / 2.0, pivot_y: f.h / 2.0, ..GroupPose::IDENTITY }
    }

    /// The lit glass stands at another footprint.
    fn to(&self, engine: &mut Engine, f: Frame) {
        let _ = engine.flow.place(self.id, Self::rest(f));
    }

    fn release(&mut self, engine: &mut Engine) {
        if !self.released {
            engine.focus.unregister(self.id);
            engine.flow.drop_lift(self.id);
            self.released = true;
        }
    }
}

/// How a layer gets to where it is going: a move glides on the card beat, a fade starts from what the layer was (reduced motion is
/// the fade alone, linear, and nothing moves).
fn put(kit: &Kit, layer: &Layer, was: f32, to: Props, at_once: bool) {
    let ms = kit.beat_ms(Beat::Card);
    if at_once || ms == 0 {
        layer.set(to);
        return;
    }
    let reduced = kit.ui.look.reduced_motion;
    if (was - to.opacity).abs() > f32::EPSILON {
        layer.set(Props { opacity: was, ..to });
        layer.animate(to, ms, if reduced { Ease::Linear } else { Ease::Out });
    } else if reduced {
        layer.set(to);
    } else {
        layer.animate(to, ms, Ease::Out);
    }
}

/// Where a card stands: the footprint its picture has, whether it has the focus, whether it is in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    f: geo::Rect,
    focused: bool,
    shown: bool,
}

/// The foot of a card (icon, name, what is happening) starts where its picture ends: at the footprint when the card has the focus, else at the
/// resting picture.
fn foot_at(f: geo::Rect, focused: bool) -> (f32, f32) {
    let r = geo::pose(f, focused);
    (r.x, r.y + r.h + 16.0)
}

struct Card {
    serial: u32,
    entry: Entry,
    lit: Lit,
    veil: ShapeLayer,
    bg: Option<ShapeLayer>,
    mark: Option<ImageLayer>,
    note: Option<Words>,
    icon: Option<ImageLayer>,
    name: Words,
    state: Option<Words>,
    /// Where the name starts from the foot's top-left (beside the icon, a short name centred on it; stacked, under it).
    name_dx: f32,
    name_dy: f32,
    name_h: f32,
    /// The tile's content: its height and where the icon starts above the centre.
    tile_h: f32,
    pose: Option<Pose>,
    /// Where the words are (they move as layers; a lint needs the offset from the box they were made in).
    at: [(f32, f32); 2],
}

impl Card {
    fn id(&self) -> Id {
        card_id(self.serial)
    }

    fn release(&mut self, engine: &mut Engine) {
        self.lit.release(engine);
    }

    #[allow(clippy::too_many_arguments)]
    fn new(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, serial: u32, e: &Entry, f: geo::Rect, m: &Metrics) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let frame = Frame::new(f.x, f.y, f.w, f.h);
        let lit = Lit::new(kit, engine, root, card_id(serial), frame, f.r, z + 6)?;
        let name_style = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
        let name = wrap::lines(kit, name_style, &e.title, m.name_w, 2);
        let said_style = role_style(kit, TypeRole::Caption, None, ink, Align::Left);
        let said = match state_words(e.standing) {
            "" => Vec::new(),
            s => wrap::lines(kit, said_style, s, m.name_w, 2),
        };
        let (body_line, caption_line) = (kit.line(TypeRole::Body), kit.line(TypeRole::Caption));
        let name_h = name.len() as f32 * body_line;
        let text_h = name_h + said.len() as f32 * caption_line;
        let (name_dx, name_dy) = if m.stacked { (0.0, ICON + kit.space(2)) } else { (ICON + kit.space(3), ((ICON - text_h) / 2.0).max(0.0).round()) };
        let note_w = note_w(kit);
        let tile_h = 80.0 + kit.space(2) + caption_line;
        let (raised, edge_c) = (kit.ui.look.colour(PaletteRole::GlassRaised), edge(kit));
        let veil_look = plate::solid(veil_colour(kit), f.r);
        let (fx, fy) = foot_at(f, false);
        let parts = scene.adopting_view(root, || -> std::io::Result<_> {
            // an app with no picture of its own to show: its icon on a tile, in the card's place
            let (bg, mark, note) = if e.picture == Picture::Tile {
                let bg = plate::shape(&kit.ui, frame, z + 1, Shape { corner_radius: f.r, fill: Fill::Solid(raised), border_width: 1.0, border: edge_c });
                let at = Props::at(f.x + (f.w - 80.0) / 2.0, (f.y + (f.h - tile_h) / 2.0).round());
                let mark = match picture_layer(kit, &e.icon, 80, 80, z + 2, at)? {
                    Some(l) => Some(l),
                    None => icon::layer(kit, "layout-grid", 56, ink2, z + 2, Props::at(at.x + 12.0, at.y + 12.0))?,
                };
                let note = Words::new(kit, f.x + (f.w - note_w) / 2.0, (f.y + (f.h - tile_h) / 2.0).round() + 80.0 + kit.space(2), note_w, z + 2, &[WordLine::new("No preview", TypeRole::Caption, ink2).aligned(Align::Centre)])?;
                (Some(bg), mark, Some(note))
            } else {
                (None, None, None)
            };
            let veil = plate::shape(&kit.ui, frame, z + 4, veil_look);
            // the app's own icon, or the generic one when it has no file: the name never stands alone in a gap
            let icon = match picture_layer(kit, &e.icon, ICON as u32, ICON as u32, z + 3, Props::at(fx, fy))? {
                Some(l) => Some(l),
                None => icon::layer(kit, "layout-grid", 32, ink2, z + 3, Props::at(fx + 8.0, fy + 8.0))?,
            };
            let lines: Vec<WordLine> = name.iter().map(|t| WordLine::new(t, TypeRole::Body, ink).weight(Weight::SemiBold)).collect();
            let name = Words::new(kit, fx + name_dx, fy + name_dy, m.name_w, z + 3, &lines)?;
            let said_lines: Vec<WordLine> = said.iter().map(|t| WordLine::new(t, TypeRole::Caption, ink)).collect();
            let state = if said.is_empty() { None } else { Some(Words::new(kit, fx + name_dx, fy + name_dy + name_h, m.name_w, z + 3, &said_lines)?) };
            Ok((bg, mark, note, veil, icon, name, state))
        })?;
        let (bg, mark, note, veil, icon, name, state) = parts;
        let at = [(name.frame().x, name.frame().y), state.as_ref().map_or((0.0, 0.0), |s| (s.frame().x, s.frame().y))];
        Ok(Self { serial, entry: e.clone(), lit, veil, bg, mark, note, icon, name, state, name_dx, name_dy, name_h, tile_h, pose: None, at })
    }

    /// Takes the card to `pose`. A card out of the window fades where it was.
    fn place(&mut self, kit: &Kit, engine: &mut Engine, pose: Pose, at_once: bool) {
        let used = if pose.shown { pose } else { Pose { f: self.pose.map_or(pose.f, |p| p.f), focused: false, ..pose } };
        if self.pose == Some(used) {
            return;
        }
        let was = self.pose;
        let (f, focused, shown) = (used.f, used.focused, used.shown);
        let scale = if focused { 1.0 } else { geo::REST };
        let (shown_op, was_op) = (if shown { 1.0 } else { 0.0 }, if was.is_some_and(|p| p.shown) { 1.0 } else { 0.0 });
        let veil_op = if shown && !focused { 1.0 } else { 0.0 };
        let was_veil = if was.is_some_and(|p| p.shown && !p.focused) { 1.0 } else { 0.0 };
        let is = |op: f32, x: f32, y: f32| Props { opacity: op, ..Props::at(x, y) };
        let sized = |op: f32| Props { opacity: op, scale, ..Props::at(f.x, f.y) };
        put(kit, &self.veil, was_veil, sized(veil_op), at_once);
        if let Some(bg) = &self.bg {
            put(kit, bg, was_op, sized(shown_op), at_once);
        }
        let top = (f.y + (f.h - self.tile_h) / 2.0).round();
        if let Some(l) = &self.mark {
            put(kit, l, was_op, is(shown_op, f.x + (f.w - 80.0) / 2.0, top), at_once);
        }
        if let Some(l) = self.note.as_ref().and_then(Words::layer) {
            put(kit, l, was_op, is(shown_op, f.x + (f.w - note_w(kit)) / 2.0, top + 80.0 + kit.space(2)), at_once);
        }
        let (fx, fy) = foot_at(f, focused);
        if let Some(l) = &self.icon {
            put(kit, l, was_op, is(shown_op, fx, fy), at_once);
        }
        let name_x = fx + self.name_dx;
        if let Some(l) = self.name.layer() {
            put(kit, l, was_op, is(shown_op, name_x, fy + self.name_dy), at_once);
        }
        self.at = [(name_x, fy + self.name_dy), (name_x, fy + self.name_dy + self.name_h)];
        if let Some(l) = self.state.as_ref().and_then(Words::layer) {
            put(kit, l, was_op, is(shown_op, self.at[1].0, self.at[1].1), at_once);
        }
        let r = geo::pose(f, focused);
        engine.focus.place(self.id(), if shown { Frame::new(r.x, r.y, r.w, r.h) } else { Frame::new(0.0, 0.0, 0.0, 0.0) });
        self.lit.to(engine, Frame::new(f.x, f.y, f.w, f.h));
        self.pose = Some(used);
    }

    /// The card leaves: it flies up (reduced motion: it fades) and its focus is given back.
    fn depart(&mut self, kit: &Kit, engine: &mut Engine, fly: bool) {
        self.release(engine);
        let Some(p) = self.pose else { return };
        let up = if fly && !kit.ui.look.reduced_motion { geo::FLY } else { 0.0 };
        let (f, scale) = (p.f, if p.focused { 1.0 } else { geo::REST });
        let (fx, fy) = foot_at(f, p.focused);
        let gone = |x: f32, y: f32| Props { opacity: 0.0, ..Props::at(x, y - up) };
        let sized = Props { opacity: 0.0, scale, ..Props::at(f.x, f.y - up) };
        put(kit, &self.veil, if p.shown && !p.focused { 1.0 } else { 0.0 }, sized, false);
        if let Some(bg) = &self.bg {
            put(kit, bg, 1.0, sized, false);
        }
        let top = (f.y + (f.h - self.tile_h) / 2.0).round();
        if let Some(l) = &self.mark {
            put(kit, l, 1.0, gone(f.x + (f.w - 80.0) / 2.0, top), false);
        }
        if let Some(l) = self.note.as_ref().and_then(Words::layer) {
            put(kit, l, 1.0, gone(f.x + (f.w - note_w(kit)) / 2.0, top + 80.0 + kit.space(2)), false);
        }
        if let Some(l) = &self.icon {
            put(kit, l, 1.0, gone(fx, fy), false);
        }
        if let Some(l) = self.name.layer() {
            put(kit, l, 1.0, gone(self.at[0].0, self.at[0].1), false);
        }
        if let Some(l) = self.state.as_ref().and_then(Words::layer) {
            put(kit, l, 1.0, gone(self.at[1].0, self.at[1].1), false);
        }
    }
}

/// The dim a card that does not have the focus is under: its picture is the compositor's, so this is a veil over it.
fn veil_colour(kit: &Kit) -> Rgba {
    faded(kit.tokens.extra(kit.mode).scrim_hide, if kit.mode == Mode::Light { 0.47 } else { 0.41 })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Cpu,
    Mem,
    Gpu,
    Temp,
}

/// Which numbers the strip has (the strip is made again when this changes) and how wide the biggest memory figure is.
#[derive(Clone, Debug, PartialEq)]
struct Shape4 {
    cores: usize,
    gpu: bool,
    temp: bool,
    mem: String,
}

impl Shape4 {
    fn of(s: Option<&Stats>) -> Self {
        let mem = s.and_then(|s| s.mem).map_or_else(|| "9.9".to_string(), |m| gb(m.total_kb));
        // the TV has three cores: that is what the bars are made for until the first sample says
        Self { cores: s.map_or(3, |s| s.cpu.len()), gpu: s.is_some_and(|s| s.gpu.is_some()), temp: s.is_some_and(|s| s.temp_c.is_some()), mem }
    }
}

fn gb(kb: u64) -> String {
    format!("{:.1}", kb as f64 / 1_048_576.0)
}

fn average(cpu: &[f32]) -> Option<f32> {
    (!cpu.is_empty()).then(|| cpu.iter().sum::<f32>() / cpu.len() as f32)
}

/// One reading in the header: its label and number and, for the CPU, a bar for each core.
struct Reading {
    kind: Kind,
    frame: Frame,
    label: Words,
    value: Words,
    bars: Bars,
    _tracks: Vec<ShapeLayer>,
    fills: Vec<ShapeLayer>,
}

/// Where a CPU core's bars stand: the left of the first, its top, the size of one and the gap between two.
#[derive(Clone, Copy)]
struct Bars {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    gap: f32,
}

/// The strip of numbers across the top of the task manager: one line of muted small type, no plate, a reading after another with even
/// gaps between the safe margins. Where the line is too long for the width the memory loses its total, then the CPU its bars, so
/// nothing ever overlaps.
struct Strip {
    shape: Shape4,
    frame: Frame,
    mem_short: bool,
    readings: Vec<Reading>,
    shown: Option<Option<Stats>>,
}

impl Strip {
    fn new(kit: &mut Kit, root: &Group, z: i32, m: &Metrics, shape: Shape4) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let ink2 = kit.ui.look.ink2();
        let scaled = |kit: &Kit, n: usize| (kit.space(n) * kit.text_scale).round();
        let (label_gap, bar_w, bar_gap, least_gap) = (scaled(kit, 2), scaled(kit, 2), scaled(kit, 1), scaled(kit, 7));
        let bar_h = (m.head_h - 2.0 * bar_gap).max(bar_w);
        let style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let mut kinds = vec![(Kind::Cpu, "CPU", "100%"), (Kind::Mem, "Memory", ""), (Kind::Gpu, "GPU", "100%"), (Kind::Temp, "Temp", "100 \u{b0}C")];
        kinds.retain(|(k, ..)| match k {
            Kind::Gpu => shape.gpu,
            Kind::Temp => shape.temp,
            Kind::Cpu | Kind::Mem => true,
        });
        let mem_long = format!("{0} / {0} GB", shape.mem);
        let mem_short = format!("{} GB", shape.mem);
        // each reading's label width and the widest its number gets (long, short): a number's box never changes with its digits
        let mut measured = Vec::new();
        for (k, label, widest_value) in &kinds {
            let lw = width_of(kit, style, label);
            let vw = |kit: &mut Kit, v: &str| widest(kit, style, &[v, "\u{2014}"]);
            let (long, short) = if *k == Kind::Mem { (vw(kit, &mem_long), vw(kit, &mem_short)) } else { let v = vw(kit, widest_value); (v, v) };
            measured.push((*k, *label, lw, long, short));
        }
        let bars_w = |n: usize| n as f32 * bar_w + n.saturating_sub(1) as f32 * bar_gap;
        let mut level = 0;
        let (widths, gap) = loop {
            let widths: Vec<f32> = measured
                .iter()
                .map(|(k, _, lw, long, short)| {
                    let vw = if level > 0 && *k == Kind::Mem { *short } else { *long };
                    lw + label_gap + vw + if *k == Kind::Cpu && level < 2 && shape.cores > 0 { label_gap + bars_w(shape.cores) } else { 0.0 }
                })
                .collect();
            let gap = ((HEAD_W - widths.iter().sum::<f32>()) / (widths.len() - 1) as f32).floor();
            if gap >= least_gap || level == 2 {
                break (widths, gap);
            }
            level += 1;
        };
        let (track, fill) = (faded(kit.ui.look.theme.materials.track_off, 0.6), faded(ink2, 0.5));
        let line = kit.line(TypeRole::Caption);
        let text_y = m.head_y + ((m.head_h - line) / 2.0).round();
        let mut readings = Vec::new();
        let mut x = EDGE_X;
        for ((kind, label, lw, long, short), w) in measured.into_iter().zip(widths) {
            let vw = if level > 0 && kind == Kind::Mem { short } else { long };
            let x0 = x.round();
            let n = if kind == Kind::Cpu && level < 2 { shape.cores } else { 0 };
            let bars = Bars { x: x0 + lw + label_gap + vw + label_gap, y: m.head_y + ((m.head_h - bar_h) / 2.0).round(), w: bar_w, h: bar_h, gap: bar_gap };
            let (tracks, fills, label_w, value_w) = scene.adopting_view(root, || -> std::io::Result<_> {
                let (mut tracks, mut fills) = (Vec::new(), Vec::new());
                for b in 0..n {
                    let f = Frame::new(bars.x + b as f32 * (bars.w + bars.gap), bars.y, bars.w, bars.h);
                    tracks.push(plate::shape(&kit.ui, f, z + 3, plate::solid(track, (bars.w / 2.0).floor())));
                    let lit = plate::shape(&kit.ui, f, z + 4, plate::solid(fill, (bars.w / 2.0).floor()));
                    lit.set(Props { opacity: 0.0, ..Props::at(f.x, f.y) });
                    fills.push(lit);
                }
                let label_w = Words::new(kit, x0, text_y, lw, z + 4, &[WordLine::new(label, TypeRole::Caption, ink2)])?;
                let value_w = Words::new(kit, x0 + lw + label_gap, text_y, vw, z + 4, &[WordLine::new("\u{2014}", TypeRole::Caption, ink2)])?;
                Ok((tracks, fills, label_w, value_w))
            })?;
            readings.push(Reading { kind, frame: Frame::new(x0, m.head_y, w, m.head_h), label: label_w, value: value_w, bars, _tracks: tracks, fills });
            x += w + gap;
        }
        Ok(Self { shape, frame: Frame::new(EDGE_X, m.head_y, HEAD_W, m.head_h), mem_short: level > 0, readings, shown: None })
    }

    /// The numbers as the last sample says them (a dash before there is one: nothing is made up).
    fn show(&mut self, kit: &mut Kit, s: Option<&Stats>) {
        if self.shown.as_ref().is_some_and(|w| w.as_ref() == s) {
            return;
        }
        let first = self.shown.is_none();
        self.shown = Some(s.cloned());
        let dash = "\u{2014}".to_string();
        for r in &mut self.readings {
            match r.kind {
                Kind::Cpu => {
                    let avg = s.and_then(|s| average(&s.cpu));
                    r.value.set(kit, 0, &avg.map_or(dash.clone(), |a| format!("{a:.0}%")));
                    let b = r.bars;
                    for (i, fill) in r.fills.iter().enumerate() {
                        let pct = s.and_then(|s| s.cpu.get(i)).copied();
                        let x = b.x + i as f32 * (b.w + b.gap);
                        let h = pct.map_or(0.0, |p| (b.h * p.clamp(0.0, 100.0) / 100.0).clamp(2.0, b.h));
                        glide_to(kit, fill, Props { opacity: if pct.is_some() { 1.0 } else { 0.0 }, clip: Some(Rect { x, y: b.y + b.h - h, w: b.w, h }), ..Props::at(x, b.y) }, first);
                    }
                }
                Kind::Mem => {
                    let text = s.and_then(|s| s.mem).map(|m| if self.mem_short { format!("{} GB", gb(m.used_kb())) } else { format!("{} / {} GB", gb(m.used_kb()), gb(m.total_kb)) });
                    r.value.set(kit, 0, &text.unwrap_or(dash.clone()));
                }
                Kind::Gpu => r.value.set(kit, 0, &s.and_then(|s| s.gpu).map_or(dash.clone(), |g| format!("{g:.0}%"))),
                Kind::Temp => r.value.set(kit, 0, &s.and_then(|s| s.temp_c).map_or(dash.clone(), |c| format!("{c:.0} \u{b0}C"))),
            }
        }
    }
}

fn glide_to(kit: &Kit, layer: &ShapeLayer, to: Props, at_once: bool) {
    if at_once {
        layer.set(to);
    } else {
        kit.glide(layer, to, Beat::Label);
    }
}

/// The sheet of what can be done to the TV: a title and three rows, each with its glyph, its name and what follows.
struct Sheet {
    sig: bool,
    panel: Frame,
    _scrim: ShapeLayer,
    _card: ShapeLayer,
    title: Words,
    rows: Vec<SheetRow>,
}

struct SheetRow {
    frame: Frame,
    plate: Plate,
    _glyph: Option<ImageLayer>,
    label: Words,
    said: Words,
    off: bool,
}

impl Sheet {
    fn new(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, blocked: bool) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (s5, s7, s8) = (kit.space(5), kit.space(7), kit.space(2));
        let z = z + 20;
        let text_w = (720.0 * kit.text_scale).min(1400.0).round();
        let glyph_px = (32.0 * kit.text_scale).clamp(32.0, 56.0).round();
        let text_dx = 24.0 + glyph_px + 20.0;
        let row_w = text_dx + text_w + 24.0;
        let panel_w = row_w + 2.0 * s7;
        let (title_line, body_line, label_line) = (kit.line(TypeRole::Title), kit.line(TypeRole::Body), kit.line(TypeRole::Label));
        let said_style = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
        let mut rows = Vec::new();
        let mut y = 0.0;
        for a in ACTIONS {
            let (glyph, label, said) = choice_words(a);
            let off = blocked && a == Action::Reboot;
            let lines = wrap::lines(kit, said_style, if off { REFRESHER } else { said }, text_w, 4);
            let h = (16.0 + body_line + 4.0 + lines.len() as f32 * label_line + 16.0).max(80.0).ceil();
            rows.push((a, glyph, label, lines, y, h, off));
            y += h + s8;
        }
        let panel_h = (s7 + title_line + s5 + y - s8 + s7).ceil();
        let panel = Frame::new(((1920.0 - panel_w) / 2.0).round(), ((1080.0 - panel_h) / 2.0).round(), panel_w, panel_h);
        let mut sheet_rows = Vec::new();
        let (scrim_layer, card, title) = scene.adopting_view(root, || -> std::io::Result<_> {
            let veil = scrim(kit, z, true);
            scrim_set(&veil, true);
            let card = plate::shape(&kit.ui, panel, z + 1, Shape { corner_radius: kit.tokens.radius(Radius::Sheet), fill: Fill::Solid(kit.ui.look.colour(PaletteRole::Sheet)), border_width: 1.0, border: edge(kit) });
            let title = Words::new(kit, panel.x + s7, panel.y + s7, panel_w - 2.0 * s7, z + 3, &[WordLine::new("System", TypeRole::Title, ink).weight(Weight::SemiBold)])?;
            Ok((veil, card, title))
        })?;
        engine.focus.register(ID_SHEET_BACK, FULL, REGION_SHEET, SHEET_SCOPE);
        engine.focus.set_enabled(ID_SHEET_BACK, false);
        for (i, (_, glyph, label, lines, ry, h, off)) in rows.into_iter().enumerate() {
            let frame = Frame::new(panel.x + s7, panel.y + s7 + title_line + s5 + ry, row_w, h);
            let plate_ = Plate::new(kit, engine, root, Spot { id: row_id(i), screen: frame, local: frame, region: REGION_SHEET, scope: SHEET_SCOPE, z: z + 2 }, Lift::Row, kit.tokens.radius(Radius::Row))?;
            let tone = if off { ink2 } else { ink };
            let (g, l, d) = scene.adopting_view(root, || -> std::io::Result<_> {
                let g = icon::layer(kit, glyph, glyph_px as u32, tone, z + 3, Props::at(frame.x + 24.0, frame.y + ((frame.h - glyph_px) / 2.0).round()))?;
                let l = Words::new(kit, frame.x + text_dx, frame.y + 16.0, text_w, z + 3, &[WordLine::new(label, TypeRole::Body, tone).weight(Weight::SemiBold)])?;
                let ls: Vec<WordLine> = lines.iter().map(|t| WordLine::new(t, TypeRole::Label, ink2)).collect();
                let d = Words::new(kit, frame.x + text_dx, frame.y + 16.0 + body_line + 4.0, text_w, z + 3, &ls)?;
                Ok((g, l, d))
            })?;
            if off {
                engine.focus.set_enabled(row_id(i), false);
            }
            sheet_rows.push(SheetRow { frame, plate: plate_, _glyph: g, label: l, said: d, off });
        }
        Ok(Self { sig: blocked, panel, _scrim: scrim_layer, _card: card, title, rows: sheet_rows })
    }

    fn destroy(self, engine: &mut Engine) {
        engine.focus.unregister(ID_SHEET_BACK);
        for r in self.rows {
            r.plate.destroy(engine);
        }
    }
}

/// A confirmation: the question, what follows and the two answers, the safe one first and in focus.
struct Ask {
    sig: (Confirm, AskState),
    card: Frame,
    /// Two dims: the sheet a question comes from is under it, and one dim leaves its words readable through.
    _scrim: [ShapeLayer; 2],
    _plate: ShapeLayer,
    title: Words,
    body: Words,
    cancel: Button,
    go: Button,
}

impl Ask {
    fn new(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, a: &AskView) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let (s3, s4, s8) = (kit.space(3), kit.space(4), kit.space(8));
        let z = z + 40;
        let w = (880.0 * kit.text_scale).min(1500.0).round();
        let inner = w - 96.0;
        let (words_title, words_body, go_label) = ask_words(a);
        let title_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let body_style = role_style(kit, TypeRole::Body, None, ink2, Align::Left);
        let title = wrap::lines(kit, title_style, &words_title, inner, 3);
        let body = wrap::lines(kit, body_style, words_body, inner, 6);
        let (title_line, body_line, btn_h) = (kit.line(TypeRole::Title), kit.line(TypeRole::Body), Button::height(kit));
        let (title_h, body_h) = (title.len() as f32 * title_line, body.len() as f32 * body_line);
        let h = (s8 + title_h + s3 + body_h + s4 + btn_h + s8).ceil();
        let card = Frame::new(((1920.0 - w) / 2.0).round(), ((1080.0 - h) / 2.0).round(), w, h);
        let by = card.y + h - s8 - btn_h;
        let (gw, cw) = (Button::width_for(kit, go_label, false), Button::width_for(kit, "Cancel", false));
        let right = card.x + w - 48.0;
        let working = a.state == AskState::Working;
        let (sheet, lay_title, lay_body) = (kit.ui.look.colour(PaletteRole::Sheet), title, body);
        let built = scene.adopting_view(root, || -> std::io::Result<_> {
            // the question hides what is behind it, the pictures too
            let veil = [scrim(kit, z, true), scrim(kit, z, true)];
            for v in &veil {
                scrim_set(v, true);
            }
            let shape = Shape { corner_radius: kit.tokens.radius(Radius::Sheet), fill: Fill::Solid(sheet), border_width: 1.0, border: edge(kit) };
            let plate_ = plate::shape(&kit.ui, card, z + 1, shape);
            let tl: Vec<WordLine> = lay_title.iter().map(|t| WordLine::new(t, TypeRole::Title, ink).weight(Weight::SemiBold)).collect();
            let title = Words::new(kit, card.x + 48.0, card.y + s8, inner, z + 2, &tl)?;
            let bl: Vec<WordLine> = lay_body.iter().map(|t| WordLine::new(t, TypeRole::Body, ink2)).collect();
            let body = Words::new(kit, card.x + 48.0, card.y + s8 + title_h + s3, inner, z + 2, &bl)?;
            let mut cancel = Button::new(kit, engine, Place { id: choice_id(0), frame: Frame::new(right - gw - kit.space(5) - cw, by, cw, btn_h), region: REGION_ASK, scope: ASK_SCOPE, z: z + 3 }, ButtonSpec::new("Cancel", Tone::Primary))?;
            cancel.show(kit, true);
            let mut go = Button::new(kit, engine, Place { id: choice_id(1), frame: Frame::new(right - gw, by, gw, btn_h), region: REGION_ASK, scope: ASK_SCOPE, z: z + 3 }, ButtonSpec::new(go_label, Tone::Danger))?;
            go.show(kit, true);
            Ok((veil, plate_, title, body, cancel, go))
        })?;
        let (veil, plate_, title, body, mut cancel, mut go) = built;
        engine.focus.register(ID_ASK_BACK, FULL, REGION_ASK, ASK_SCOPE);
        engine.focus.set_enabled(ID_ASK_BACK, false);
        if working {
            for b in [&mut cancel, &mut go] {
                b.set_enabled(kit, engine, false);
                b.set_dimmed(kit, true);
            }
        }
        Ok(Self { sig: (a.kind, a.state), card, _scrim: veil, _plate: plate_, title, body, cancel, go })
    }

    fn destroy(self, engine: &mut Engine) {
        engine.focus.unregister(ID_ASK_BACK);
        self.cancel.destroy(engine);
        self.go.destroy(engine);
    }
}

/// Which glyph hints the pill shows: they say what the keys do where the focus is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hints {
    Cards,
    Lower,
}

struct HintPill {
    which: Hints,
    _pill: ShapeLayer,
    _rows: Vec<HintRow>,
}

fn hint_pill(kit: &mut Kit, root: &Group, z: i32, m: &Metrics, which: Hints) -> std::io::Result<HintPill> {
    let ink = kit.ui.look.ink();
    let hints = match which {
        Hints::Cards => [Hint::key(Key::Ok, HintAction::Open), Hint::key(Key::Up, HintAction::Close)],
        Hints::Lower => [Hint::key(Key::Ok, HintAction::Confirm), Hint::key(Key::Back, HintAction::Close)],
    };
    let (s3, s6, s7) = (kit.space(3), kit.space(6), kit.space(7));
    let w = hints_width(kit, &hints, TypeRole::Label, s7) + 2.0 * s6;
    let h = kit.tokens.target.max(glyph_side(kit, TypeRole::Label) as f32 + s3).ceil();
    let f = Frame::new(1920.0 - 96.0 - w, m.action_y + ((m.btn_h - h) / 2.0).round(), w, h);
    let scene = kit.ui.scene.clone();
    let built = scene.adopting_view(root, || -> std::io::Result<_> {
        let pill = plate::shape(&kit.ui, f, z + 2, plate::solid(faded(kit.tokens.extra(kit.mode).scrim_hide, 0.88), (f.h / 2.0).floor()));
        let mut rows = Vec::new();
        let mut x = f.x + s6;
        for h in &hints {
            let row = HintRow::new(kit, h, TypeRole::Label, RowAt { x, y: f.y, h: f.h, z: z + 3 }, ink)?;
            x += row.width() + s7;
            rows.push(row);
        }
        Ok((pill, rows))
    })?;
    Ok(HintPill { which, _pill: built.0, _rows: built.1 })
}

/// What is made for one look (theme, mode, text size) and what is shown on it.
struct Built {
    look: (u64, f32),
    base: usize,
    used: usize,
    next: u32,
    cards: Vec<Card>,
    /// Cards that left and are still fading: their chrome goes when `until` (ns) has passed.
    leaving: Vec<(Card, u64)>,
    empty: Option<(Option<ImageLayer>, Words)>,
    strip: Option<Strip>,
    close: Button,
    system: Button,
    close_on: bool,
    hints: Option<HintPill>,
    bar: Option<(ShapeLayer, ShapeLayer, Frame, usize)>,
    sheet: Option<Sheet>,
    ask: Option<Ask>,
    settled: bool,
}

/// One of the two buttons under the cards, at `x` and shown at once.
#[allow(clippy::too_many_arguments)]
fn lower_button(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, m: &Metrics, id: Id, x: f32, label: &str, glyph: &'static str) -> std::io::Result<Button> {
    let scene = kit.ui.scene.clone();
    let w = Button::width_for(kit, label, true);
    scene.adopting_view(root, || -> std::io::Result<_> {
        let place = Place { id, frame: Frame::new(x, m.action_y, w, m.btn_h), region: REGION_LOWER, scope: SCOPE, z: z + 3 };
        let mut button = Button::new(kit, engine, place, ButtonSpec::new(label, Tone::Plain).with_icon(glyph))?;
        button.show(kit, true);
        Ok(button)
    })
}

impl Built {
    fn new(kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, look: (u64, f32), m: &Metrics) -> std::io::Result<Self> {
        let base = kit.room().layers;
        let close = lower_button(kit, engine, root, z, m, ID_CLOSE, 96.0, "Close all", "x")?;
        let system = lower_button(kit, engine, root, z, m, ID_SYSTEM, 96.0 + close.frame().w + kit.space(5), "System", "monitor-cog")?;
        Ok(Self { look, base, used: 0, next: 1, cards: Vec::new(), leaving: Vec::new(), empty: None, strip: None, close, system, close_on: true, hints: None, bar: None, sheet: None, ask: None, settled: false })
    }

    fn serial(&mut self) -> u32 {
        let s = self.next;
        self.next = s % 0xFFF + 1;
        s
    }

    fn destroy(self, engine: &mut Engine) {
        for mut c in self.cards {
            c.release(engine);
        }
        for (mut c, _) in self.leaving {
            c.release(engine);
        }
        if let Some(s) = self.sheet {
            s.destroy(engine);
        }
        if let Some(a) = self.ask {
            a.destroy(engine);
        }
        self.close.destroy(engine);
        self.system.destroy(engine);
    }

    /// Brings what is drawn to what the model says; the scopes that are modal over the stage's (the sheet, the confirmation) come back.
    #[allow(clippy::too_many_arguments)]
    fn update(&mut self, kit: &mut Kit, engine: &mut Engine, root: &Group, z: i32, m: &Metrics, v: &View, now_ns: u64) -> std::io::Result<Vec<Scope>> {
        let at_once = !self.settled;
        let total = v.entries.len();
        let listed = total.min(MAX_CARDS);
        self.leaving.retain(|(_, until)| *until > now_ns);
        let mut old = std::mem::take(&mut self.cards);
        let mut next: Vec<(Card, bool)> = Vec::with_capacity(listed);
        for (i, e) in v.entries.iter().take(listed).enumerate() {
            let found = old.iter().position(|c| c.entry.id == e.id).map(|p| old.remove(p));
            let f = geo::card(i, total, v.first, m.top);
            next.push(match found {
                Some(c) if c.entry == *e => (c, false),
                // an app whose title, icon or standing changed is drawn again where it stands
                Some(mut c) => {
                    c.release(engine);
                    (Card::new(kit, engine, root, z, self.serial(), e, f, m)?, true)
                }
                None => (Card::new(kit, engine, root, z, self.serial(), e, f, m)?, false),
            });
        }
        let linger = u64::from(kit.beat_ms(Beat::Card)) * 1_000_000 + 40_000_000;
        for mut c in old {
            c.depart(kit, engine, v.sent.contains(&c.entry.id));
            self.leaving.push((c, now_ns + linger));
        }
        for (i, (c, once)) in next.iter_mut().enumerate() {
            let pose = Pose { f: geo::card(i, total, v.first, m.top), focused: v.focus == Zone::Cards && v.index == i, shown: geo::in_window(i, v.first) };
            c.place(kit, engine, pose, at_once || *once);
        }
        self.cards = next.into_iter().map(|(c, _)| c).collect();
        if total == 0 {
            if self.empty.is_none() {
                self.empty = Some(empty_state(kit, root, z, m)?);
            }
        } else {
            self.empty = None;
        }
        if total > geo::VISIBLE {
            if self.bar.as_ref().is_none_or(|b| b.3 != total) {
                self.bar = Some(window_bar(kit, root, z, m, total)?);
            }
            if let Some((_, thumb, track, _)) = &self.bar {
                let span = (total - geo::VISIBLE) as f32;
                let w = (track.w * geo::VISIBLE as f32 / total as f32).round();
                glide_to(kit, thumb, Props::at((track.x + (track.w - w) * v.first.min(total - geo::VISIBLE) as f32 / span).round(), track.y), at_once);
            }
        } else {
            self.bar = None;
        }
        let shape = Shape4::of(v.stats.as_ref());
        if self.strip.as_ref().is_none_or(|s| s.shape != shape) {
            self.strip = Some(Strip::new(kit, root, z, m, shape)?);
        }
        if let Some(s) = self.strip.as_mut() {
            s.show(kit, v.stats.as_ref());
        }
        if (total > 0) != self.close_on {
            self.close_on = total > 0;
            if self.close_on {
                self.close.set_enabled(kit, engine, true);
                self.close.show(kit, at_once);
            } else {
                self.close.hide(kit, at_once);
                self.close.set_enabled(kit, engine, false);
            }
            // System takes the left edge when Close all is not there, and does not float where it was
            let x = if self.close_on { 96.0 + self.close.frame().w + kit.space(5) } else { 96.0 };
            self.system.detach(engine);
            self.system = lower_button(kit, engine, root, z, m, ID_SYSTEM, x, "System", "monitor-cog")?;
        }
        let which = if total > 0 && v.focus == Zone::Cards { Hints::Cards } else { Hints::Lower };
        if v.sheet.is_some() || v.ask.is_some() {
            self.hints = None;
        } else if self.hints.as_ref().is_none_or(|h| h.which != which) {
            self.hints = Some(hint_pill(kit, root, z, m, which)?);
        }
        let blocked = v.stats.as_ref().is_some_and(|s| s.refresher);
        match (v.sheet, self.sheet.take()) {
            (Some(_), Some(s)) if s.sig == blocked => self.sheet = Some(s),
            (Some(_), old) => {
                if let Some(s) = old {
                    s.destroy(engine);
                }
                self.sheet = Some(Sheet::new(kit, engine, root, z, blocked)?);
            }
            (None, Some(s)) => s.destroy(engine),
            (None, None) => {}
        }
        match (&v.ask, self.ask.take()) {
            (Some(a), Some(s)) if s.sig == (a.kind, a.state) => self.ask = Some(s),
            (Some(a), old) => {
                if let Some(s) = old {
                    s.destroy(engine);
                }
                self.ask = Some(Ask::new(kit, engine, root, z, a)?);
            }
            (None, Some(s)) => s.destroy(engine),
            (None, None) => {}
        }
        self.used = self.base.abs_diff(kit.room().layers);
        self.settled = true;
        let mut modal = Vec::new();
        if self.sheet.is_some() {
            modal.push(SHEET_SCOPE);
        }
        if self.ask.is_some() {
            modal.push(ASK_SCOPE);
        }
        Ok(modal)
    }

    /// The control a button id belongs to.
    fn button(&mut self, id: Id) -> Option<&mut Button> {
        if id == ID_CLOSE {
            return Some(&mut self.close);
        }
        if id == ID_SYSTEM {
            return Some(&mut self.system);
        }
        let a = self.ask.as_mut()?;
        [&mut a.cancel, &mut a.go].into_iter().zip(0..).find_map(|(b, c)| (choice_id(c) == id).then_some(b))
    }

    fn pick(&self, id: Id) -> Option<Pick> {
        if id == ID_CLOSE {
            return Some(Pick::Close);
        }
        if id == ID_SYSTEM {
            return Some(Pick::System);
        }
        if id == ID_BACKDROP {
            return Some(Pick::Outside);
        }
        if let Some(i) = self.cards.iter().position(|c| c.id() == id) {
            return Some(Pick::Card(i));
        }
        (0..ACTIONS.len()).find(|r| row_id(*r) == id).map(Pick::Row).or_else(|| (0..2).find(|c| choice_id(*c) == id).map(Pick::Choice))
    }
}

/// What stands where no card does: a calm line, so an empty list reads as nothing open and not as a fault.
fn empty_state(kit: &mut Kit, root: &Group, z: i32, m: &Metrics) -> std::io::Result<(Option<ImageLayer>, Words)> {
    let scene = kit.ui.scene.clone();
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let gap = kit.space(4);
    let all = 64.0 + gap + kit.line(TypeRole::Title);
    let y = (((m.zone_top + m.zone_bottom) - all) / 2.0).round();
    scene.adopting_view(root, || -> std::io::Result<_> {
        let mark = icon::layer(kit, "layout-grid", 64, ink2, z + 3, Props::at(928.0, y))?;
        let words = Words::new(kit, 660.0, y + 64.0 + gap, 600.0, z + 3, &[WordLine::new("No apps open", TypeRole::Title, ink).aligned(Align::Centre)])?;
        Ok((mark, words))
    })
}

/// Where the window of cards stands among all of them, when not all are in it: a short, dim mark centred under the cards.
fn window_bar(kit: &mut Kit, root: &Group, z: i32, m: &Metrics, total: usize) -> std::io::Result<(ShapeLayer, ShapeLayer, Frame, usize)> {
    let scene = kit.ui.scene.clone();
    let y = (m.top + geo::H + kit.space(4) + m.foot_h + kit.space(3)).round();
    let width = (total as f32 * MARK_PER_APP).min(1728.0);
    let track = Frame::new(((1920.0 - width) / 2.0).round(), y, width, MARK_H);
    let w = (track.w * geo::VISIBLE as f32 / total as f32).round();
    let (off, thumb) = (faded(kit.ui.look.theme.materials.track_off, 0.6), faded(kit.ui.look.ink2(), 0.5));
    let r = MARK_H / 2.0;
    let (t, th) = scene.adopting_view(root, || (plate::shape(&kit.ui, track, z + 2, plate::solid(off, r)), plate::shape(&kit.ui, Frame::new(track.x, y, w, MARK_H), z + 3, plate::solid(thumb, r))));
    Ok((t, th, track, total))
}

/// An opaque colour as `[r, g, b]`: `c` (premultiplied) over `under`.
fn flat(c: Rgba, under: [u8; 3]) -> [u8; 3] {
    let k = 255 - u32::from(c.3);
    let mix = |top: u8, below: u8| (u32::from(top) + (u32::from(below) * k + 127) / 255).min(255) as u8;
    [mix(c.0, under[0]), mix(c.1, under[1]), mix(c.2, under[2])]
}

pub struct SwitcherSurface {
    stage: Stage,
    built: Option<Built>,
    /// The scopes made modal over the stage's own, bottom first.
    modals: Vec<Scope>,
    metrics: Option<((u64, f32), Metrics)>,
    /// The card a press began on and where, for a drag up.
    press: Option<(Id, f32)>,
    dragged: bool,
    outside: Dim,
}

impl SwitcherSurface {
    pub fn new(z: i32) -> Self {
        Self { stage: Stage::new(z, SCOPE), built: None, modals: Vec::new(), metrics: None, press: None, dragged: false, outside: Dim::default() }
    }

    pub fn is_up(&self) -> bool {
        self.stage.is_up()
    }

    fn metrics(&mut self, kit: &mut Kit) -> Metrics {
        let look = (kit.generation(), kit.text_scale);
        match self.metrics {
            Some((l, m)) if l == look => m,
            _ => {
                let m = metrics(kit);
                self.metrics = Some((look, m));
                m
            }
        }
    }

    /// The top of the row of cards for this look: the compositor places the pictures there.
    pub fn top(&mut self, kit: &mut Kit) -> f32 {
        self.metrics(kit).top
    }

    /// The colours the compositor's own basic screen is drawn in, from the theme: opaque `[r, g, b]`.
    pub fn palette(kit: &Kit) -> Value {
        let look = &kit.ui.look;
        let panel = flat(look.colour(PaletteRole::Sheet), [0, 0, 0]);
        let over = |c: Rgba| flat(c, panel);
        json!({"panel": panel, "text": over(look.ink()), "muted": over(look.ink2()), "accent": over(look.accent()), "danger": over(look.danger())})
    }

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
        let m = self.metrics(kit);
        let look = (kit.generation(), kit.text_scale);
        if self.built.as_ref().is_none_or(|b| b.look != look) {
            self.drop_built(engine);
            self.stage.backdrop(engine, ID_BACKDROP, REGION_LOWER);
            self.built = Some(Built::new(kit, engine, self.stage.group(), self.stage.z(), look, &m)?);
            self.stage.refocus();
        }
        let mut modal = Vec::new();
        if up && let Some(b) = self.built.as_mut() {
            modal = b.update(kit, engine, self.stage.group(), self.stage.z(), &m, v, now_ns)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the task manager could not be drawn whole"));
            }
        }
        // the overlays' scopes are popped before the stage's own closes, pushed after it opens
        let keep = self.modals.iter().zip(&modal).take_while(|(a, b)| a == b).count();
        while self.modals.len() > keep {
            self.modals.pop();
            engine.focus.close_modal();
        }
        let live = self.stage.phase(kit, engine, phase);
        for s in &modal[self.modals.len()..] {
            engine.focus.open_modal(*s);
            self.modals.push(*s);
            self.stage.refocus();
        }
        if live {
            let want = self.want(v);
            self.stage.focus(engine, want, now_ns);
        }
        Ok(true)
    }

    /// The control the model's focus is on.
    fn want(&self, v: &View) -> Option<Id> {
        let b = self.built.as_ref()?;
        if let Some(a) = &v.ask {
            return Some(choice_id(a.choice.min(1)));
        }
        if let Some(r) = v.sheet {
            return Some(row_id(r));
        }
        Some(match v.focus {
            Zone::Cards => b.cards.get(v.index).map_or(ID_SYSTEM, Card::id),
            Zone::Close if b.close_on => ID_CLOSE,
            Zone::Close | Zone::System => ID_SYSTEM,
        })
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        while self.modals.pop().is_some() {
            engine.focus.close_modal();
        }
        if let Some(b) = self.built.take() {
            b.destroy(engine);
        }
    }

    pub fn on_key(&mut self, engine: &mut Engine) {
        self.stage.key(engine);
    }

    /// A sample of the pointer: the cue follows it over the controls, a click is the control it ended on, and a card dragged up is a quit.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Pick> {
        let current = engine.focus.current();
        let (built, press) = (&mut self.built, &mut self.press);
        let click = self.stage.pointer(kit, engine, now_ns, s, |touch| match touch {
            Touch::Hover { from, to } => {
                let b = built.as_mut()?;
                if let Some(btn) = from.and_then(|f| b.button(f)) {
                    btn.set_hover(kit, false);
                }
                if let Some(btn) = to.filter(|t| current != Some(*t)).and_then(|t| b.button(t)) {
                    btn.set_hover(kit, true);
                }
                match b.pick(to?)? {
                    Pick::Card(i) => b.cards.get(i)?.pose.map(|p| geo::pose(p.f, p.focused).r),
                    Pick::Row(_) => Some(kit.tokens.radius(Radius::Row)),
                    _ => None,
                }
            }
            Touch::Press { id, down } => {
                let b = built.as_mut()?;
                if let Some(btn) = b.button(id) {
                    btn.set_pressed(kit, down);
                }
                *press = (down && matches!(b.pick(id), Some(Pick::Card(_)))).then_some((id, s.y));
                None
            }
        });
        let b = self.built.as_ref()?;
        if s.visible
            && s.buttons & 1 != 0
            && let Some((id, _)) = self.press.filter(|(_, y0)| s.y <= y0 - DRAG_PX)
            && let Some(Pick::Card(i)) = b.pick(id)
        {
            self.press = None;
            self.dragged = true;
            return Some(Pick::Drag(i));
        }
        if self.dragged {
            // the release that ends a drag is not a click
            self.dragged = s.buttons & 1 != 0;
            return None;
        }
        if let Some(id) = click {
            return b.pick(id);
        }
        let back = if b.ask.is_some() {
            Some(ID_ASK_BACK)
        } else {
            b.sheet.as_ref().map(|_| ID_SHEET_BACK)
        };
        let on = back.is_some_and(|id| s.visible && engine.focus.hit(s.x, s.y) == Some(id));
        self.outside.feed(on, &s).then_some(Pick::Outside)
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
        self.built.as_ref().map_or(0, |b| b.used)
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let f = |fr: Frame| json!({"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h});
        let cards: Vec<Value> = b
            .cards
            .iter()
            .map(|c| {
                let said = c.state.as_ref().map_or("", |w| w.text(0));
                json!({"id": c.entry.id, "title": c.entry.title, "picture": format!("{:?}", c.entry.picture), "said": said, "frame": f(engine.focus.frame(c.id()).unwrap_or(Frame::new(0.0, 0.0, 0.0, 0.0))), "shown": c.pose.is_some_and(|p| p.shown)})
            })
            .collect();
        let focus = engine.focus.current().and_then(|id| {
            Some(match b.pick(id)? {
                Pick::Card(i) => format!("card:{i}"),
                Pick::Close => "close".to_string(),
                Pick::System => "system".to_string(),
                Pick::Row(r) => format!("row:{r}"),
                Pick::Choice(c) => format!("choice:{c}"),
                Pick::Drag(_) | Pick::Outside => return None,
            })
        });
        let sheet = b.sheet.as_ref().map(|s| json!({"rows": s.rows.iter().map(|r| json!({"frame": f(r.frame), "off": r.off, "label": r.label.text(0), "said": r.said.text(0)})).collect::<Vec<_>>()}));
        let ask = b.ask.as_ref().map(|a| json!({"title": a.title.text(0), "body": a.body.text(0), "go": a.go.spec().label, "state": format!("{:?}", a.sig.1)}));
        json!({
            "cards": cards,
            "leaving": b.leaving.len(),
            "focus": focus,
            "close": b.close_on.then(|| f(b.close.frame())),
            "system": f(b.system.frame()),
            "strip": b.strip.as_ref().map(|s| json!({"frame": f(s.frame), "tiles": s.readings.iter().map(|t| format!("{:?}", t.kind)).collect::<Vec<_>>(), "frames": s.readings.iter().map(|t| f(t.frame)).collect::<Vec<_>>(), "values": s.readings.iter().map(|t| t.value.text(0)).collect::<Vec<_>>()})),
            "sheet": sheet,
            "ask": ask,
            "empty": b.empty.is_some(),
            "layers": self.layers(),
            "scope": SCOPE.0,
        })
    }

    /// Describes what is on the screen to a lint: the dim the compositor draws behind the chrome, the words on it, the strip, the buttons, and
    /// the sheet or the confirmation that hides all of it.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let sheet = kit.ui.look.colour(PaletteRole::Sheet);
        let dim = lint.panel("task manager dim", FULL, kit.tokens.extra(kit.mode).scrim_hide, false);
        if let Some(a) = &b.ask {
            let card = lint.panel_on("confirmation", a.card, sheet, false, dim);
            a.title.feed_lint(kit, lint, "confirmation title", Some(card), (0.0, 0.0));
            a.body.feed_lint(kit, lint, "confirmation words", Some(card), (0.0, 0.0));
            a.cancel.feed_lint(kit, lint, card);
            a.go.feed_lint(kit, lint, card);
            return;
        }
        if let Some(s) = &b.sheet {
            let card = lint.panel_on("system sheet", s.panel, sheet, false, dim);
            s.title.feed_lint(kit, lint, "system title", Some(card), (0.0, 0.0));
            for (i, r) in s.rows.iter().enumerate() {
                r.label.feed_lint(kit, lint, &format!("system row {i}"), Some(card), (0.0, 0.0));
                r.said.feed_lint(kit, lint, &format!("system row {i} words"), Some(card), (0.0, 0.0));
            }
            return;
        }
        let raised = kit.ui.look.colour(PaletteRole::GlassRaised);
        for (i, c) in b.cards.iter().enumerate() {
            let Some(p) = c.pose.filter(|p| p.shown) else { continue };
            if let Some(note) = &c.note {
                let r = geo::pose(p.f, p.focused);
                let tile = lint.panel_on(&format!("card {i} tile"), Frame::new(r.x, r.y, r.w, r.h), raised, false, dim);
                let top = (p.f.y + (p.f.h - c.tile_h) / 2.0).round() + 80.0 + kit.space(2);
                note.feed_lint(kit, lint, &format!("card {i} caption"), Some(tile), (p.f.x + (p.f.w - note_w(kit)) / 2.0 - note.frame().x, top - note.frame().y));
            }
            let name_shift = (c.at[0].0 - c.name.frame().x, c.at[0].1 - c.name.frame().y);
            c.name.feed_lint(kit, lint, &format!("card {i} name"), Some(dim), name_shift);
            if let Some(st) = &c.state {
                st.feed_lint(kit, lint, &format!("card {i} state"), Some(dim), (c.at[1].0 - st.frame().x, c.at[1].1 - st.frame().y));
            }
        }
        if let Some((_, words)) = &b.empty {
            words.feed_lint(kit, lint, "no apps", Some(dim), (0.0, 0.0));
        }
        if let Some(s) = &b.strip {
            for t in &s.readings {
                t.label.feed_lint(kit, lint, "stat label", Some(dim), (0.0, 0.0));
                t.value.feed_lint(kit, lint, "stat value", Some(dim), (0.0, 0.0));
            }
        }
        if b.close_on {
            b.close.feed_lint(kit, lint, dim);
        }
        b.system.feed_lint(kit, lint, dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use crate::panels::K;
    use crate::panels::switcher::{APPS_TAG, Switcher};

    struct Up {
        _h: Option<ui::testing::Harness>,
        kit: Kit,
        engine: Engine,
        s: SwitcherSurface,
        m: Switcher,
        t: u64,
    }

    fn app(id: &str, title: &str, picture: &str, state: &str, hung: bool) -> Value {
        json!({"id": id, "title": title, "picture": picture, "w": 1920, "h": 1080, "state": state, "fg": false, "hung": hung, "pid": 7})
    }

    /// One app in each standing: fine, not responding, with no picture to show, didn't quit, closing.
    fn five() -> Value {
        json!([
            app("a", "Plex", "live", "", false),
            app("b", "YouTube", "live", "", true),
            app("c", "HDMI 2", "none", "", false),
            app("d", "Disney+ with a long name that wraps onto lines", "live", "stuck", false),
            app("e", "Spotify", "live", "quitting", false)
        ])
    }

    fn stats(refresher: bool, extras: bool) -> Value {
        json!({
            "n": 3,
            "cpu": [12.0, 80.0, 40.0, 5.0, 60.0, 33.0],
            "mem": {"totalKb": 3_000_000u64, "availKb": 1_200_000u64, "cachedKb": 400_000u64},
            "gpu": if extras { json!(35) } else { Value::Null },
            "tempC": if extras { json!(61) } else { Value::Null },
            "mhz": 1200,
            "refresher": refresher
        })
    }

    impl Up {
        fn new(mode: ui::lucent::palette::Mode, scale: f32, list: Value) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            Self::over(Some(h), kit, 1000, list, None)
        }

        /// A task manager open over `list` on `kit`, at `t` ms; `points` are the launch points that give the apps their icons.
        fn over(h: Option<ui::testing::Harness>, kit: Kit, t: u64, list: Value, points: Option<Value>) -> Self {
            let engine = Engine::new(&kit).expect("an engine");
            let mut m = Switcher::new();
            if let Some(p) = points {
                m.apps.update(&Input::Luna { tag: APPS_TAG, payload: json!({"launchPoints": p}) });
            }
            m.topic("apps", &list, 0);
            m.topic("switcher", &json!({"open": true, "from": "", "n": 1, "mode": "ui"}), 0);
            let mut me = Self { _h: h, kit, engine, s: SwitcherSurface::new(crate::kit::z::PANELS + 40), m, t };
            me.apply();
            me
        }

        fn apply(&mut self) {
            let v = self.m.view();
            self.s.apply(&mut self.kit, &mut self.engine, &v, Phase::Open, self.t * 1_000_000).expect("the chrome is drawn");
        }

        fn key(&mut self, k: K) {
            self.m.key(k, self.t);
            self.t += 40;
            self.apply();
        }

        fn keys(&mut self, ks: &[K]) {
            for k in ks {
                self.key(*k);
            }
        }

        fn push_stats(&mut self, s: Value) {
            self.m.topic("stats", &s, self.t);
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

    fn centre(f: &Value) -> (f32, f32) {
        let n = |k: &str| f[k].as_f64().expect("a number") as f32;
        (n("x") + n("w") / 2.0, n("y") + n("h") / 2.0)
    }

    #[test]
    fn each_card_stands_where_the_compositor_draws_its_picture_and_the_first_has_the_focus() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        let top = u.s.top(&mut u.kit);
        let g = u.geo();
        for (i, c) in g["cards"].as_array().unwrap().iter().take(geo::VISIBLE).enumerate() {
            let r = geo::pose(geo::card(i, 5, 0, top), i == 0);
            let f = &c["frame"];
            assert_eq!([f["x"].as_f64(), f["y"].as_f64(), f["w"].as_f64(), f["h"].as_f64()], [r.x, r.y, r.w, r.h].map(|v| Some(f64::from(v))), "card {i}");
        }
        assert_eq!(g["cards"][4]["frame"]["w"], 0.0, "the fifth card is out of the window: nothing to focus");
        assert_eq!(g["focus"], "card:0");
        assert_eq!(g["cards"][2]["picture"], "Tile", "an app with no picture of its own gets an icon tile");
    }

    #[test]
    fn captions_say_what_is_happening_to_a_card_in_words() {
        let u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        let said: Vec<_> = u.geo()["cards"].as_array().unwrap().iter().map(|c| c["said"].as_str().unwrap().to_string()).collect();
        assert_eq!(said, ["", "Not responding", "", "Didn\u{2019}t quit", "Closing"]);
    }

    #[test]
    fn the_focus_goes_across_the_cards_down_to_the_buttons_and_back_without_making_a_layer() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        let layers = u.kit.room().layers;
        u.key(K::Right);
        assert_eq!(u.geo()["focus"], "card:1");
        assert!(u.engine.flow.settling_at(u.t * 1_000_000), "the lit glass flows between the cards");
        u.key(K::Down);
        assert_eq!(u.geo()["focus"], "close");
        u.key(K::Right);
        assert_eq!(u.geo()["focus"], "system");
        u.key(K::Up);
        assert_eq!(u.geo()["focus"], "card:1", "up from the buttons returns to the card");
        assert_eq!(u.kit.room().layers, layers, "moving the focus draws nothing new");
    }

    #[test]
    fn a_card_that_is_quit_leaves_and_the_others_close_up() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        let before = u.geo()["cards"][1]["frame"]["x"].as_f64().unwrap();
        u.key(K::Up);
        let g = u.geo();
        assert_eq!(g["cards"].as_array().unwrap().len(), 4, "its chrome is no longer a card");
        assert_eq!(g["leaving"], 1, "it flies up while it fades");
        assert!((g["cards"][0]["frame"]["x"].as_f64().unwrap() - before).abs() > 1.0, "the neighbours reflow");
        u.t += 2000;
        u.apply();
        assert_eq!(u.geo()["leaving"], 0, "and then its layers are gone");
    }

    #[test]
    fn the_system_sheet_has_three_rows_and_a_choice_asks_with_cancel_in_focus() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        u.keys(&[K::Down, K::Right, K::Ok]);
        let g = u.geo();
        assert_eq!(g["sheet"]["rows"].as_array().unwrap().len(), 3);
        assert_eq!(g["focus"], "row:0");
        assert_eq!(g["sheet"]["rows"][2]["label"], "Restart the TV");
        u.keys(&[K::Down, K::Down, K::Ok]);
        let g = u.geo();
        assert_eq!(g["ask"]["title"], "Restart the TV?");
        assert_eq!(g["ask"]["go"], "Restart TV");
        assert_eq!(g["focus"], "choice:0", "the confirmation starts on Cancel");
        u.key(K::Back);
        assert_eq!(u.geo()["ask"], Value::Null, "Back cancels the question");
        assert_eq!(u.geo()["focus"], "row:2", "and goes back one step, to the sheet");
        u.key(K::Back);
        assert_eq!(u.geo()["sheet"], Value::Null);
    }

    #[test]
    fn the_restart_row_says_why_it_is_off_while_the_panel_refresher_runs() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        u.push_stats(stats(true, true));
        u.keys(&[K::Down, K::Right, K::Ok]);
        let rows = u.geo()["sheet"]["rows"].clone();
        assert_eq!(rows[2]["off"], true);
        assert_eq!(rows[2]["said"], "The panel refresher is running; try again when it is done.");
        assert_eq!(rows[0]["off"], false);
    }

    #[test]
    fn no_apps_is_a_calm_empty_state_with_system_still_reachable() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, json!([]));
        u.push_stats(stats(false, false));
        let g = u.geo();
        assert_eq!(g["empty"], true);
        assert_eq!(g["close"], Value::Null, "there is nothing to close");
        assert_eq!(g["focus"], "system");
        assert!(g["strip"]["tiles"].as_array().unwrap().len() >= 2, "the numbers still show");
    }

    #[test]
    fn the_strip_has_a_dash_before_the_first_sample_and_a_tile_for_gpu_and_temperature_only_when_they_are_read() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        assert_eq!(u.geo()["strip"]["values"], json!(["\u{2014}", "\u{2014}"]), "no sample yet: dashes, not zeros");
        u.push_stats(stats(false, false));
        assert_eq!(u.geo()["strip"]["tiles"], json!(["Cpu", "Mem"]));
        assert_eq!(u.geo()["strip"]["values"][1], "1.7 / 2.9 GB");
        u.push_stats(stats(false, true));
        assert_eq!(u.geo()["strip"]["tiles"], json!(["Cpu", "Mem", "Gpu", "Temp"]));
        assert_eq!(u.geo()["strip"]["values"][3], "61 \u{b0}C");
    }

    #[test]
    fn the_numbers_are_one_line_across_the_top_inside_the_margins_and_the_cards_are_centred_in_the_room_below() {
        use ui::lucent::palette::Mode;
        let rect = |v: &Value| Frame::new(v["x"].as_f64().unwrap() as f32, v["y"].as_f64().unwrap() as f32, v["w"].as_f64().unwrap() as f32, v["h"].as_f64().unwrap() as f32);
        for scale in [1.0, 2.0] {
            let mut u = Up::new(Mode::Dark, scale, five());
            let mut s = stats(false, true);
            s["cpu"] = json!([12.0, 80.0, 40.0]);
            u.push_stats(s);
            let m = metrics(&mut u.kit);
            let g = u.geo();
            let items: Vec<Frame> = g["strip"]["frames"].as_array().unwrap().iter().map(rect).collect();
            assert_eq!(items.len(), 4, "CPU, memory, GPU and temperature at {scale}");
            for (i, f) in items.iter().enumerate() {
                assert!(f.x >= 96.0 && f.x + f.w <= 1824.0, "item {i} inside the side margins at {scale}: {f:?}");
                assert!(f.y >= 54.0 && f.y + f.h <= m.zone_top, "item {i} is on the top row, above the cards' room at {scale}: {f:?}");
                assert_eq!((f.y, f.h), (items[0].y, items[0].h), "item {i} is on the same line at {scale}");
                if i > 0 {
                    assert!(f.x > items[i - 1].x + items[i - 1].w, "item {i} does not touch the one before at {scale}");
                }
            }
            let cards = g["cards"].as_array().unwrap().iter().map(|c| rect(&c["frame"])).filter(|f| f.w > 0.0);
            let foot = geo::H + u.kit.space(4) + m.foot_h;
            for (i, c) in cards.enumerate() {
                assert!(c.y >= m.top && c.y >= m.zone_top && c.y + c.h <= m.top + geo::H, "card {i} is in the row, in the room below the strip at {scale}");
                assert!(items.iter().all(|f| f.y + f.h <= c.y), "card {i} is under every item at {scale}");
            }
            let system = rect(&g["system"]);
            assert!(m.top + foot <= m.zone_bottom && m.zone_bottom <= system.y, "the cards and their words end above the buttons at {scale}");
            let (above, below) = (m.top - m.zone_top, m.zone_bottom - (m.top + foot));
            assert!((below - above).abs() <= u.kit.space(8) + u.kit.space(2) + 1.0, "the row is centred in the room, the page mark's space aside, at {scale}");
        }
    }

    #[test]
    fn clicks_land_on_cards_and_buttons_and_a_drag_up_is_a_quit_not_a_click() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        let g = u.geo();
        let (cx, cy) = centre(&g["cards"][1]["frame"]);
        let mut ns = u.t * 1_000_000;
        let mut feed = |u: &mut Up, x: f32, y: f32, b: u32| {
            ns += 40_000_000;
            u.s.pointer(&u.kit, &mut u.engine, ns, at(x, y, b))
        };
        feed(&mut u, cx, cy, 0);
        feed(&mut u, cx, cy, 1);
        assert_eq!(feed(&mut u, cx, cy, 0), Some(Pick::Card(1)));
        let (sx, sy) = centre(&g["system"]);
        feed(&mut u, sx, sy, 0);
        feed(&mut u, sx, sy, 1);
        assert_eq!(feed(&mut u, sx, sy, 0), Some(Pick::System));
        feed(&mut u, cx, cy, 0);
        feed(&mut u, cx, cy, 1);
        assert_eq!(feed(&mut u, cx, cy - DRAG_PX - 10.0, 1), Some(Pick::Drag(1)));
        assert_eq!(feed(&mut u, cx, cy - DRAG_PX - 10.0, 0), None, "the release of a drag is not a click");
    }

    #[test]
    fn what_is_drawn_passes_the_lint_in_both_modes_at_both_text_sizes() {
        use ui::lucent::palette::Mode;
        for (mode, scale) in [(Mode::Dark, 1.0), (Mode::Light, 1.0), (Mode::Dark, 2.0), (Mode::Light, 2.0)] {
            let mut u = Up::new(mode, scale, five());
            u.push_stats(stats(false, true));
            assert_eq!(u.lint(), Vec::<String>::new(), "cards and strip, {mode:?} at {scale}");
            u.keys(&[K::Down, K::Right, K::Ok]);
            assert_eq!(u.lint(), Vec::<String>::new(), "the sheet, {mode:?} at {scale}");
            u.keys(&[K::Ok]);
            assert_eq!(u.lint(), Vec::<String>::new(), "a confirmation, {mode:?} at {scale}");
            let mut empty = Up::new(mode, scale, json!([]));
            empty.push_stats(stats(false, false));
            assert_eq!(empty.lint(), Vec::<String>::new(), "no apps, {mode:?} at {scale}");
        }
    }

    #[test]
    fn dropping_gives_back_every_layer_and_every_key() {
        let mut u = Up::new(ui::lucent::palette::Mode::Dark, 1.0, five());
        u.keys(&[K::Down, K::Right, K::Ok]);
        let v = u.m.view();
        u.s.apply(&mut u.kit, &mut u.engine, &v, Phase::Gone, 0).expect("gone");
        assert_eq!(u.s.layers(), 0);
        assert_eq!(u.geo(), Value::Null);
        assert!(u.engine.focus.current().is_none());
    }

    /// The scenes written as PNGs for looking at, when `TASKMGR_SHOTS` names a directory (and `LIBGL_ALWAYS_SOFTWARE=1` on a machine with a
    /// GPU): the real renderer on software GL. The compositor draws the pictures in the TV, so here fake ones stand in the card rectangles of
    /// the layout the model sends, over the dim the compositor draws. Without the variable the test does nothing.
    mod shots {
        use super::*;
        use crate::kit::icon::Icons;
        use crate::kit::screen::Effect;
        use crate::panels::switcher::SYSTEM_TAG;
        use compositor_protocol::Event;
        use lumatv_render::host::{Engine as Host, HostConfig};
        use lumatv_render::{Gles, now_ns};
        use lumatv_tokens::Tokens;
        use std::io;
        use std::path::Path;
        use std::sync::mpsc::Receiver;
        use std::time::{Duration, Instant};
        use ui::lucent::material::MaterialTier;
        use ui::testing::TempDir;

        const FRAME_NS: u64 = 16_666_667;
        /// Long enough for the fades and glides (a card's beat is a few hundred ms) to land.
        const FRAMES: usize = 45;

        struct Rig {
            host: Host,
            events: Receiver<Event>,
            ns: u64,
            _shm: TempDir,
        }

        fn rig(mode: Mode, scale: f32) -> io::Result<(Rig, Kit)> {
            let shm = TempDir::new("taskmgr-shots");
            let gles = Gles::software_pbuffer(c"libEGL.so.1", 1920, 1080)?;
            let config = HostConfig { width: 1920, height: 1080, refresh_hz: 60.0, shm_dir: shm.path().to_owned(), direct: false, gpu_sync: false, idle_trim: false, pages: 0, reference: false, blit: false, external: false, system_z: i32::MAX };
            let (mut host, end) = Host::new(gles, config)?;
            let (scene, _, events) = ui::Scene::over_with(end, ui::SceneOptions { shm_dir: Some(shm.path().to_owned()), ..Default::default() })?;
            host.finish_handshake()?;
            let mut kit = Kit::new(scene, Tokens::shipped(), mode, MaterialTier::T1, scale)?;
            kit.icons = Icons::new(&testkit::module_dir());
            Ok((Rig { host, events, ns: now_ns(), _shm: shm }, kit))
        }

        impl Rig {
            fn pump(&mut self, kit: &mut Kit) -> io::Result<()> {
                while let Ok(event) = self.events.try_recv() {
                    kit.ui.scene.observe(&event);
                }
                if let Some(atlas) = kit.ui.text_atlas() {
                    atlas.pump();
                }
                if kit.ui.scene.has_pending() {
                    kit.ui.try_commit()?;
                }
                if self.host.pump()?.closed {
                    return Err(io::Error::other("the renderer closed"));
                }
                Ok(())
            }

            /// Until every glyph has landed in the atlas: a label is a picture for good before.
            fn land(&mut self, kit: &mut Kit) -> io::Result<()> {
                let end = Instant::now() + Duration::from_secs(20);
                loop {
                    self.pump(kit)?;
                    self.host.apply_waiting(now_ns());
                    self.host.tidy();
                    if kit.ui.glyph_text_settled() && !kit.ui.scene.has_pending() && !self.host.tidy_more() {
                        return Ok(());
                    }
                    if Instant::now() > end {
                        return Err(io::Error::other("the glyph atlas did not settle"));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }

            /// Draws the scene `FRAMES` refreshes on from the later of the rig's clock and the model's, and returns the page.
            fn shot(&mut self, kit: &mut Kit, at_ms: u64) -> io::Result<Vec<u8>> {
                self.land(kit)?;
                self.ns = self.ns.max(at_ms * 1_000_000);
                for _ in 0..FRAMES {
                    self.ns += FRAME_NS;
                    self.pump(kit)?;
                    let cost = self.host.draw(self.ns);
                    self.host.frame_done(self.ns, &cost);
                    self.host.tidy();
                }
                Ok(self.host.read_page(0))
            }
        }

        fn write_png(path: &Path, rgba: &[u8]) -> io::Result<()> {
            let mut encoder = png::Encoder::new(io::BufWriter::new(std::fs::File::create(path)?), 1920, 1080);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().and_then(|mut w| w.write_image_data(rgba)).map_err(io::Error::other)
        }

        /// An app icon as the launch points name it: a rounded square from `from` to `to` with a disc of `mark` in it.
        fn icon_file(dir: &Path, name: &str, from: Rgba, to: Rgba, mark: Rgba) -> io::Result<String> {
            const S: usize = 96;
            let mut px = Vec::with_capacity(S * S * 4);
            for y in 0..S {
                for x in 0..S {
                    let (cx, cy) = (x as f32 + 0.5 - 48.0, y as f32 + 0.5 - 48.0);
                    let (qx, qy) = (cx.abs() - 26.0, cy.abs() - 26.0);
                    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - 22.0;
                    let t = (x + y) as f32 / (2.0 * S as f32);
                    let (a, b) = if cx.hypot(cy) < 20.0 { (mark, mark) } else { (from, to) };
                    let mix = |p: u8, q: u8| (f32::from(p) + (f32::from(q) - f32::from(p)) * t) as u8;
                    px.extend_from_slice(&[mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2), ((0.5 - d).clamp(0.0, 1.0) * 255.0) as u8]);
                }
            }
            let path = dir.join(format!("{name}.png"));
            let mut encoder = png::Encoder::new(io::BufWriter::new(std::fs::File::create(&path)?), S as u32, S as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().and_then(|mut w| w.write_image_data(&px)).map_err(io::Error::other)?;
            Ok(path.to_string_lossy().into_owned())
        }

        /// The five apps of the scenes: one of each standing, the one with no picture (a game on the video plane) with its icon on a tile.
        fn apps() -> Value {
            json!([
                app("plex", "Plex", "live", "", false),
                app("youtube", "YouTube", "thumb", "", true),
                app("music", "Spotify", "live", "quitting", false),
                app("game", "LumaGame", "none", "", false),
                app("disney", "Disney+ with a long name that wraps onto lines", "live", "stuck", false)
            ])
        }

        /// The readings of the scenes: three cores.
        fn stats(refresher: bool, extras: bool) -> Value {
            let mut s = super::stats(refresher, extras);
            s["cpu"] = json!([12.0, 80.0, 40.0]);
            s
        }

        fn launch_points(kit: &Kit, dir: &Path) -> io::Result<Value> {
            let c = |r| kit.ui.look.colour(r);
            let mark = c(PaletteRole::Ink);
            let mut points = Vec::new();
            for (id, title, from, to) in [
                ("plex", "Plex", PaletteRole::Warning, PaletteRole::Positive),
                ("youtube", "YouTube", PaletteRole::Danger, PaletteRole::Accent),
                ("game", "LumaGame", PaletteRole::Accent, PaletteRole::Info),
                ("disney", "Disney+ with a long name that wraps onto lines", PaletteRole::Info, PaletteRole::Accent),
                ("music", "Spotify", PaletteRole::Positive, PaletteRole::Info),
            ] {
                points.push(json!({"id": id, "title": title, "icon": icon_file(dir, id, c(from), c(to), mark)?}));
            }
            Ok(Value::Array(points))
        }

        /// What the compositor draws under the chrome: a dim over a Home-like backdrop and, when `pictures`, a fake picture in each rectangle the
        /// model's layout names.
        fn stand_in(u: &mut Up, pictures: bool) -> Vec<ShapeLayer> {
            let top = u.s.top(&mut u.kit);
            let palette = SwitcherSurface::palette(&u.kit);
            let cards = match u.m.layout(u.t, top, json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}), false, 200, palette) {
                Some(Effect::Channel { body, .. }) => body["cards"].as_array().cloned().unwrap_or_default(),
                _ => Vec::new(),
            };
            let k = &u.kit;
            let (c, scene) = (|r| k.ui.look.colour(r), k.ui.scene.clone());
            let plain = |r: f32, fill: Fill| Shape { corner_radius: r, fill, border_width: 0.0, border: Rgba(0, 0, 0, 0) };
            let mut out = Vec::new();
            let mut put = |x: f32, y: f32, w: f32, h: f32, z: i32, shape: Shape| out.push(scene.shape(w.round() as u32, h.round() as u32, z, shape, Props::at(x, y)));
            put(0.0, 0.0, 1920.0, 1080.0, 70, plain(0.0, Fill::Linear { from: faded(c(PaletteRole::Info), 0.5), to: c(PaletteRole::Canvas), angle_deg: 135.0 }));
            if !pictures {
                return out;
            }
            put(0.0, 0.0, 1920.0, 1080.0, 71, plain(0.0, Fill::Solid(k.tokens.extra(k.mode).scrim_hide)));
            let looks = [(PaletteRole::Accent, PaletteRole::Info), (PaletteRole::Ink, PaletteRole::Ink2), (PaletteRole::Positive, PaletteRole::Info), (PaletteRole::Danger, PaletteRole::Warning)];
            for (i, card) in cards.iter().enumerate() {
                let n = |key: &str| card[key].as_f64().unwrap_or(0.0) as f32;
                let (x, y, w, h, r) = (n("x"), n("y"), n("w"), n("h"), n("r"));
                let (from, to) = looks[i % looks.len()];
                put(x, y, w, h, 72, plain(r, Fill::Linear { from: c(from), to: c(to), angle_deg: 120.0 }));
                let ink = faded(c(PaletteRole::Canvas), 0.45);
                put(x + w * 0.06, y + h * 0.08, w * 0.4, h * 0.07, 73, plain(4.0, Fill::Solid(ink)));
                put(x + w * 0.06, y + h * 0.5, w * 0.4, h * 0.38, 73, plain(8.0, Fill::Solid(ink)));
                put(x + w * 0.54, y + h * 0.5, w * 0.4, h * 0.38, 73, plain(8.0, Fill::Solid(ink)));
            }
            out
        }

        struct Scene {
            name: &'static str,
            /// Dark and light at 100 % and 200 % text, else dark at 100 % only.
            matrix: bool,
            empty: bool,
            /// The compositor's pictures are there (not in its basic screen).
            pictures: bool,
            prep: fn(&mut Up),
        }

        const SHEET: [K; 3] = [K::Down, K::Right, K::Ok];

        fn scenes() -> Vec<Scene> {
            fn scene(name: &'static str, matrix: bool, empty: bool, pictures: bool, prep: fn(&mut Up)) -> Scene {
                Scene { name, matrix, empty, pictures, prep }
            }
            vec![
                scene("a-apps", true, false, true, |u| u.push_stats(stats(false, true))),
                scene("a-apps-last-focused", false, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&[K::Right, K::Right, K::Right, K::Right]);
                }),
                scene("a-apps-close-all-focused", false, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.key(K::Down);
                }),
                scene("b-system", true, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&SHEET);
                }),
                scene("c-restart-lumatv-confirm", true, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&SHEET);
                    u.key(K::Ok);
                }),
                scene("d-close-all-confirm", false, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&[K::Down, K::Ok]);
                }),
                scene("e-lg-home-confirm", false, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&SHEET);
                    u.keys(&[K::Down, K::Ok]);
                }),
                scene("f-restart-tv-refresher-sheet", false, false, true, |u| {
                    u.push_stats(stats(true, true));
                    u.keys(&SHEET);
                }),
                scene("f-restart-tv-refused", false, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&SHEET);
                    u.keys(&[K::Down, K::Down, K::Ok, K::Right, K::Ok]);
                    u.m.update(&Input::Fetched { tag: SYSTEM_TAG, status: 200, data: json!({"ok": false, "error": "refresher"}) }, u.t);
                    u.apply();
                }),
                scene("g-empty", true, true, true, |u| u.push_stats(stats(false, false))),
                scene("h-working", false, false, true, |u| {
                    u.push_stats(stats(false, true));
                    u.keys(&SHEET);
                    u.keys(&[K::Ok, K::Right, K::Ok]);
                }),
                scene("i-basic-screen-draws-nothing", false, false, false, |u| {
                    u.push_stats(stats(false, true));
                    u.m.topic("switcher", &json!({"open": true, "from": "", "n": 1, "mode": "fallback"}), u.t);
                    let v = u.m.view();
                    u.s.apply(&mut u.kit, &mut u.engine, &v, Phase::Gone, u.t * 1_000_000).expect("gone");
                    assert!(!v.up && u.s.layers() == 0 && u.geo() == Value::Null, "the compositor's own screen is in charge: nothing of ours is drawn");
                }),
            ]
        }

        fn shoot(dir: &Path, s: &Scene, mode: Mode, scale: f32) -> io::Result<()> {
            let (mut rig, kit) = rig(mode, scale)?;
            let icons = TempDir::new("taskmgr-icons");
            let points = launch_points(&kit, icons.path())?;
            let list = if s.empty { json!([]) } else { apps() };
            let mut u = Up::over(None, kit, rig.ns / 1_000_000, list, Some(points));
            (s.prep)(&mut u);
            let _behind = stand_in(&mut u, s.pictures);
            let page = rig.shot(&mut u.kit, u.t)?;
            let (look, text) = (if mode == Mode::Dark { "dark" } else { "light" }, (scale * 100.0) as u32);
            write_png(&dir.join(format!("{}-{look}-{text}.png", s.name)), &page)
        }

        #[test]
        fn the_scenes_are_written_as_pngs_for_looking_at_when_a_directory_is_named() {
            let Ok(dir) = std::env::var("TASKMGR_SHOTS") else { return };
            let dir = Path::new(&dir);
            std::fs::create_dir_all(dir).expect("the shot directory");
            for s in scenes() {
                let looks: &[(Mode, f32)] = if s.matrix { &[(Mode::Dark, 1.0), (Mode::Light, 1.0), (Mode::Dark, 2.0), (Mode::Light, 2.0)] } else { &[(Mode::Dark, 1.0)] };
                for &(mode, scale) in looks {
                    shoot(dir, &s, mode, scale).unwrap_or_else(|e| panic!("{} ({mode:?}, {scale}): {e}", s.name));
                }
            }
        }
    }
}
