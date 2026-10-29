//! The on-screen keyboard every text entry on the device uses: QWERTY letters, two symbol pages, and a bottom row with the page key,
//! the space bar and an action key whose label the screen chooses ("Connect").
//!
//! Every key is a `Key` glass surface (design section 3.4.2): raised, with its inset twin prepared above it, so a press is a crossfade
//! and nothing is drawn. The keys sit on one containing sheet. Legends are text, and text is the expensive part, so each page's
//! legends are one text layer per page (two for the letters: lower case and shifted), made once: the page that shows when the
//! keyboard is built, and the other three between the frames that follow ([`Keyboard::prepare`]), or on demand if a page is shown
//! before its turn, and all again when the theme changes. A page switch or a shift then only shows one legend layer and hides
//! another, and a key press draws no text and makes no layer. As pictures the four legends are a million pixels, nearly half of what
//! a keyboard is (made at once they were the slowest frame of a text entry); with glyph text switched on
//! ([`Ui::enable_glyph_text`]) each is a run of about thirty quads of the shared atlas and writes no pixel, and the same lazy order
//! keeps the frame that builds the keyboard to the one page that shows.
//!
//! The keys are a pool of about forty glass surfaces that every page shares: a page switch moves and shows the surfaces its keys
//! need and hides the rest. (A glass pair for every key of every page would be more than eighty keys, a third of the shell's
//! whole budget of drawable layers.)
//!
//! What typing does is separate from how it looks: [`Model`] takes the touches and says what was typed and which page shows,
//! [`Keyboard`] puts that on the layers. The screen applies a [`KeyOutput`] to its text and updates the text field in the same turn,
//! so a typed character shows by the next frame (call [`Keyboard::warm`] first, so that no glyph is rastered while a finger is down).

use super::icon::Icon;
use super::mark::Mark;
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::text::{Run, ellipsise, line_height};
use super::text_layer::{Item as LayerItem, TextLayer};
use super::{Component, Damage, Group, Part, Pose};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchEvent, TouchPhase};
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::tokens::{motion, space, ty};
use crate::text::Type;
use compositor_protocol::{Props, Rgba};
use std::io;

/// What a committed key asks of the text being typed. A shift or a page change is the keyboard's own and is not output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutput {
    Char(char),
    Backspace,
    /// The action key (the screen names it, "Connect"), only while it is enabled.
    Action,
}

/// Every key is at least 64 units tall (Lucent's smart-display target).
const KEY_H: f32 = 64.0;
const GAP: f32 = 8.0;
const ROWS: usize = 4;
/// Room between the containing sheet's edge and its keys.
const PAD: f32 = space::S3;
/// The tallest a legend grows with the person's text size: what a 64-high key holds with a little air above and below.
const MAX_LEGEND_PX: u32 = 40;
/// A second tap of shift within this long of the first locks capitals.
const DOUBLE_TAP_NS: u64 = 350_000_000;
/// Holding backspace: nothing for this long (a tap is not a repeat), then one every `REPEAT_NS`.
const REPEAT_DELAY_NS: u64 = 400_000_000;
const REPEAT_NS: u64 = 60_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Letters,
    Symbols,
    More,
}

const PAGES: [Page; 3] = [Page::Letters, Page::Symbols, Page::More];
/// The legend layers, in the order `Model::legend_layer` numbers them: letters, letters shifted, symbols, more symbols.
const LEGENDS: [(Page, bool); 4] = [
    (Page::Letters, false),
    (Page::Letters, true),
    (Page::Symbols, false),
    (Page::More, false),
];

impl Page {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyKind {
    /// A character as its page prints it. On the letters page shift makes it a capital.
    Char(char),
    Shift,
    Backspace,
    Space,
    /// Goes to a page.
    To(Page),
    Action,
}

/// The width a key comes in. Keys of one class are the same size, so any of them can stand in for any other on another page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// A character: ten fill a row.
    Unit,
    /// Shift, backspace and the page keys.
    Wide,
    /// A character of the symbols page's third row, which shares it with two wide keys.
    Narrow,
    Action,
    /// The space bar, as wide as the bottom row has room for.
    Space,
}

const CLASSES: [Class; 5] = [
    Class::Unit,
    Class::Wide,
    Class::Narrow,
    Class::Action,
    Class::Space,
];

impl Class {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug)]
struct KeyDef {
    frame: Frame,
    kind: KeyKind,
    class: Class,
    /// The glass surface of the keyboard's pool that shows this key on its page.
    slot: usize,
}

impl KeyDef {
    /// Where a finger counts as on this key: the key and half of each gap around it, so the boxes of a row meet without overlapping
    /// and a finger between two keys still types the nearer one.
    fn hit(&self) -> Frame {
        Frame::new(
            self.frame.x - GAP / 2.0,
            self.frame.y - GAP / 2.0,
            self.frame.w + GAP,
            self.frame.h + GAP,
        )
    }
}

/// One thing in a row.
enum Item {
    Key(KeyKind, Class),
    /// A key that takes the width the row has left: the space bar.
    Fill(KeyKind),
    /// Empty room that takes the width the row has left.
    Room,
}

/// The widths keys come in, from the width of the key area: ten keys of a unit fill a row.
struct Sizes {
    unit: f32,
    wide: f32,
    action: f32,
    narrow: f32,
}

fn sizes(frame_w: f32) -> Sizes {
    let unit = ((frame_w - 9.0 * GAP) / 10.0).floor();
    let wide = (unit * 1.5).floor();
    Sizes {
        unit,
        wide,
        action: unit * 2.0,
        narrow: ((frame_w - 2.0 * wide - 10.0 * GAP) / 9.0).floor(),
    }
}

fn chars(text: &str, class: Class) -> Vec<Item> {
    text.chars()
        .map(|c| Item::Key(KeyKind::Char(c), class))
        .collect()
}

/// The keys of `page` in the key area `frame`: three rows of characters and the bottom row. Every printable ASCII character is on one
/// of the three pages (the tests check it). Slots are given by [`Model::new`].
fn layout(page: Page, frame: Frame) -> Vec<KeyDef> {
    let s = sizes(frame.w);
    let key = |kind| Item::Key(kind, Class::Wide);
    let bottom = |other| {
        vec![
            key(KeyKind::To(other)),
            Item::Fill(KeyKind::Space),
            Item::Key(KeyKind::Action, Class::Action),
        ]
    };
    let rows: [Vec<Item>; ROWS] = match page {
        Page::Letters => [
            chars("qwertyuiop", Class::Unit),
            chars("asdfghjkl", Class::Unit),
            [key(KeyKind::Shift)]
                .into_iter()
                .chain(chars("zxcvbnm", Class::Unit))
                .chain([key(KeyKind::Backspace)])
                .collect(),
            bottom(Page::Symbols),
        ],
        Page::Symbols => [
            chars("1234567890", Class::Unit),
            chars("-/:;()$&@\"", Class::Unit),
            [key(KeyKind::To(Page::More))]
                .into_iter()
                .chain(chars(".,?!'#%+=", Class::Narrow))
                .chain([key(KeyKind::Backspace)])
                .collect(),
            bottom(Page::Letters),
        ],
        Page::More => [
            chars("[]{}^*_", Class::Unit),
            chars("\\|~<>`", Class::Unit),
            vec![
                key(KeyKind::To(Page::Symbols)),
                Item::Room,
                key(KeyKind::Backspace),
            ],
            bottom(Page::Letters),
        ],
    };
    let width_of = |class: Class| match class {
        Class::Unit => s.unit,
        Class::Wide => s.wide,
        Class::Narrow => s.narrow,
        Class::Action => s.action,
        Class::Space => 0.0,
    };
    let mut keys = Vec::new();
    for (row, items) in rows.iter().enumerate() {
        let y = frame.y + row as f32 * (KEY_H + GAP);
        let fixed: f32 = items
            .iter()
            .map(|item| match item {
                Item::Key(_, class) => width_of(*class),
                Item::Fill(_) | Item::Room => 0.0,
            })
            .sum::<f32>()
            + GAP * (items.len() - 1) as f32;
        let flexes = items
            .iter()
            .any(|item| matches!(item, Item::Fill(_) | Item::Room));
        // A row with room to fill spans the frame; any other is centred.
        let mut x = if flexes {
            frame.x
        } else {
            frame.x + ((frame.w - fixed) / 2.0).floor()
        };
        for item in items {
            let (key, w) = match item {
                Item::Key(kind, class) => (Some((*kind, *class)), width_of(*class)),
                Item::Fill(kind) => (Some((*kind, Class::Space)), frame.w - fixed),
                Item::Room => (None, frame.w - fixed),
            };
            if let Some((kind, class)) = key {
                keys.push(KeyDef {
                    frame: Frame::new(x, y, w, KEY_H),
                    kind,
                    class,
                    slot: 0,
                });
            }
            x += w + GAP;
        }
    }
    keys
}

/// How shift stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shift {
    Off,
    /// The next character is a capital; one-shot.
    Armed,
    /// Every letter is a capital until shift is tapped again or the letters are left.
    Locked,
}

/// A finger that is down on the keyboard.
#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u32,
    /// The key it went down on and has not left: its page and index. A finger that slid off a key holds none, so lifting types
    /// nothing, even back on it.
    key: Option<(Page, usize)>,
    /// When the key next repeats (only backspace repeats).
    repeat_at: Option<u64>,
    /// Whether the key has repeated: its release then types nothing more.
    repeated: bool,
}

/// What typing has done so far, without any layer: the page, the shift, the key a finger is holding. Pure, so what a sequence of taps
/// types is checked without a screen.
struct Model {
    /// Where a contact counts as on the keyboard: its sheet.
    bounds: Frame,
    pages: [Vec<KeyDef>; 3],
    /// How many surfaces the pool needs for the pages to take turns.
    slots: usize,
    page: Page,
    shift: Shift,
    /// When shift was last tapped to arm it.
    shift_at: u64,
    contact: Option<Contact>,
    action_enabled: bool,
}

impl Model {
    fn new(bounds: Frame, keys: Frame) -> Self {
        let mut pages = PAGES.map(|page| layout(page, keys));
        // Each page's keys take the pool's surfaces of their class in order; the pool is as big as the busiest page needs.
        let mut pool = [0usize; 5];
        for keys in &pages {
            let mut used = [0usize; 5];
            for key in keys {
                used[key.class.index()] += 1;
            }
            for class in CLASSES {
                pool[class.index()] = pool[class.index()].max(used[class.index()]);
            }
        }
        let mut base = [0usize; 5];
        for class in 1..5 {
            base[class] = base[class - 1] + pool[class - 1];
        }
        for keys in &mut pages {
            let mut used = [0usize; 5];
            for key in keys.iter_mut() {
                key.slot = base[key.class.index()] + used[key.class.index()];
                used[key.class.index()] += 1;
            }
        }
        Self {
            bounds,
            pages,
            slots: pool.iter().sum(),
            page: Page::Letters,
            shift: Shift::Off,
            shift_at: 0,
            contact: None,
            action_enabled: false,
        }
    }

    fn keys(&self) -> &[KeyDef] {
        &self.pages[self.page.index()]
    }

    fn key_at(&self, x: f32, y: f32) -> Option<usize> {
        self.keys().iter().position(|k| k.hit().contains(x, y))
    }

    /// The key a finger holds, with its page.
    fn pressed(&self) -> Option<(Page, usize)> {
        self.contact.and_then(|c| c.key)
    }

    fn set_action_enabled(&mut self, enabled: bool) {
        self.action_enabled = enabled;
        if !enabled
            && let Some(c) = &mut self.contact
            && let Some((page, i)) = c.key
            && self.pages[page.index()][i].kind == KeyKind::Action
        {
            c.key = None;
        }
    }

    /// One contact event. Returns what a committed key typed.
    ///
    /// A key is committed when the finger lifts on the key it went down on. A finger that slid off a key lets it go for good, so
    /// lifting anywhere, even back on it, types nothing; a lost contact types nothing; a second finger is ignored while the first is
    /// down, so a stray palm cannot type.
    fn touch(&mut self, e: &TouchEvent) -> Option<KeyOutput> {
        match e.phase {
            TouchPhase::Down => {
                if self.contact.is_some() || !self.bounds.contains(e.x, e.y) {
                    return None;
                }
                let key = self
                    .key_at(e.x, e.y)
                    .filter(|&i| self.keys()[i].kind != KeyKind::Action || self.action_enabled)
                    .map(|i| (self.page, i));
                let repeats = key.is_some_and(|(page, i)| {
                    self.pages[page.index()][i].kind == KeyKind::Backspace
                });
                self.contact = Some(Contact {
                    id: e.id,
                    key,
                    repeat_at: repeats.then_some(e.at_ns + REPEAT_DELAY_NS),
                    repeated: false,
                });
                None
            }
            TouchPhase::Move => {
                let at = self.key_at(e.x, e.y);
                if let Some(c) = &mut self.contact
                    && c.id == e.id
                    && let Some((_, held)) = c.key
                    && at != Some(held)
                {
                    c.key = None;
                    c.repeat_at = None;
                }
                None
            }
            TouchPhase::Up => {
                let c = self.contact.filter(|c| c.id == e.id)?;
                self.contact = None;
                let (_, held) = c.key?;
                if c.repeated || self.key_at(e.x, e.y) != Some(held) {
                    return None;
                }
                self.commit(held, e.at_ns)
            }
            TouchPhase::Cancel => {
                if self.contact.is_some_and(|c| c.id == e.id) {
                    self.contact = None;
                }
                None
            }
        }
    }

    /// Time passes: a held backspace that is due types again. At most one repeat per call; a screen that ticks once a frame gets one
    /// per frame at most, never a burst after a stall.
    fn tick(&mut self, now_ns: u64) -> Option<KeyOutput> {
        let c = self.contact.as_mut()?;
        let due = c.repeat_at?;
        if now_ns < due {
            return None;
        }
        c.repeated = true;
        let next = due + REPEAT_NS;
        c.repeat_at = Some(if next > now_ns {
            next
        } else {
            now_ns + REPEAT_NS
        });
        Some(KeyOutput::Backspace)
    }

    /// When the held key next repeats, for a screen that schedules its ticks.
    fn next_repeat(&self) -> Option<u64> {
        self.contact.and_then(|c| c.repeat_at)
    }

    fn commit(&mut self, index: usize, at_ns: u64) -> Option<KeyOutput> {
        match self.keys()[index].kind {
            KeyKind::Char(c) => Some(KeyOutput::Char(self.typed(c))),
            KeyKind::Space => Some(KeyOutput::Char(self.typed(' '))),
            KeyKind::Backspace => Some(KeyOutput::Backspace),
            KeyKind::Shift => {
                self.shift = match self.shift {
                    Shift::Locked => Shift::Off,
                    Shift::Armed if at_ns.saturating_sub(self.shift_at) <= DOUBLE_TAP_NS => {
                        Shift::Locked
                    }
                    Shift::Armed => Shift::Off,
                    Shift::Off => Shift::Armed,
                };
                self.shift_at = at_ns;
                None
            }
            KeyKind::To(page) => {
                self.page = page;
                // Shift belongs to the letters: leaving them drops it, locked or not.
                self.shift = Shift::Off;
                None
            }
            KeyKind::Action => self.action_enabled.then_some(KeyOutput::Action),
        }
    }

    /// The character a key types now; an armed shift is spent, a locked one stays.
    fn typed(&mut self, c: char) -> char {
        let c = if self.shift == Shift::Off {
            c
        } else {
            c.to_ascii_uppercase()
        };
        if self.shift == Shift::Armed {
            self.shift = Shift::Off;
        }
        c
    }

    /// Which legend layer shows: letters, letters shifted, symbols, more symbols.
    fn legend_layer(&self) -> usize {
        match (self.page, self.shift != Shift::Off) {
            (Page::Letters, false) => 0,
            (Page::Letters, true) => 1,
            (Page::Symbols, _) => 2,
            (Page::More, _) => 3,
        }
    }
}

/// What a legend is drawn at: the role's size at the person's text size, as far as a key holds it.
fn legend_type(ui: &Ui, role: Type) -> Type {
    let t = ui.look.ty(role);
    Type {
        px: t.px.min(MAX_LEGEND_PX),
        ..t
    }
}

/// The text a key's legend shows, or `None` for a key whose legend is an icon or nothing (the space bar, whose look is its shape
/// alone; the action key, whose label is a layer of its own because its colour follows whether it can be used).
fn legend_text(kind: KeyKind, shifted: bool) -> Option<(String, Type)> {
    Some(match kind {
        KeyKind::Char(c) if shifted => (c.to_ascii_uppercase().to_string(), ty::KEY),
        KeyKind::Char(c) => (c.to_string(), ty::KEY),
        KeyKind::To(Page::Letters) => ("ABC".to_owned(), ty::LABEL),
        KeyKind::To(Page::Symbols) => ("?123".to_owned(), ty::LABEL),
        KeyKind::To(Page::More) => ("#+=".to_owned(), ty::LABEL),
        KeyKind::Shift | KeyKind::Backspace | KeyKind::Space | KeyKind::Action => return None,
    })
}

fn legend_icon(kind: KeyKind) -> Option<Icon> {
    match kind {
        KeyKind::Shift => Some(Icon::Shift),
        KeyKind::Backspace => Some(Icon::Backspace),
        _ => None,
    }
}

/// A line of legend: its text and the box it is centred in, as a [`Run`] says them (it owns the text).
struct Legend {
    text: String,
    ty: Type,
    colour: Rgba,
    x: f32,
    y: f32,
    w: f32,
}

/// What one layer of legends shows: lines of text and marks (icons), in the one colour of ink of the moment. They share a layer
/// and no two overlap, so any order paints the same.
#[derive(Default)]
struct Pieces {
    texts: Vec<Legend>,
    marks: Vec<Mark>,
}

impl Pieces {
    fn items(&self) -> Vec<LayerItem<'_>> {
        let texts = self
            .texts
            .iter()
            .map(|l| LayerItem::Text(Run::centre(&l.text, l.ty, l.colour, l.x, l.y, l.w)));
        texts
            .chain(self.marks.iter().map(|m| LayerItem::Mark(*m)))
            .collect()
    }
}

/// `text` centred in `frame` both ways (in the pixels of the layer that carries it): a line box whose baseline falls where
/// `draw_line` put it, so a legend sits on the pixels it always has.
fn centred(ui: &mut Ui, text: String, ty: Type, colour: Rgba, frame: Frame) -> Legend {
    let extent = ui.fonts.measure(&text, ty);
    let baseline = (frame.y + (frame.h - extent.height()) / 2.0 + extent.ascent).round();
    // A run's baseline is its line box's top plus the rounded offset inside the box; the box goes where that makes `baseline`.
    let y = baseline - ((line_height(ty) - extent.height()) / 2.0 + extent.ascent).round();
    Legend {
        text,
        ty,
        colour,
        x: frame.x,
        y,
        w: frame.w,
    }
}

/// The legends of `keys` for a layer whose top left is `origin` in the keyboard's coordinates: the characters as text, shift and
/// backspace as icons.
fn legend_pieces(ui: &mut Ui, keys: &[KeyDef], origin: (f32, f32), shifted: bool) -> Pieces {
    let ink = ui.look.ink();
    let mut pieces = Pieces::default();
    for key in keys {
        let at = Frame::new(
            key.frame.x - origin.0,
            key.frame.y - origin.1,
            key.frame.w,
            key.frame.h,
        );
        if let Some((text, role)) = legend_text(key.kind, shifted) {
            let ty = legend_type(ui, role);
            pieces.texts.push(centred(ui, text, ty, ink, at));
        } else if let Some(icon) = legend_icon(key.kind) {
            let side = legend_type(ui, ty::KEY).px;
            let (x, y) = (
                at.x + (at.w - side as f32) / 2.0,
                at.y + (at.h - side as f32) / 2.0,
            );
            pieces.marks.push(Mark::icon(
                icon,
                side,
                x.round() as i32,
                y.round() as i32,
                ink,
            ));
        }
    }
    pieces
}

/// What the action key's label is drawn at: the person's text size, taken down a step at a time (to the role's own size) until the
/// whole label fits the key. Cutting the label is the last resort, after it has been made as small as it goes.
fn action_type(ui: &mut Ui, label: &str, key_w: f32) -> Type {
    let room = key_w - 2.0 * space::S3;
    let mut t = legend_type(ui, ty::BUTTON);
    while t.px > ty::BUTTON.px && ui.fonts.measure(label, t).width > room {
        t.px = (t.px - 2).max(ty::BUTTON.px);
    }
    t
}

/// What the action key shows, in a layer of `size` (the key): the label the screen chose, cut with an ellipsis as a last resort if
/// the key cannot hold it, or the enter mark when there is no label. Returns the text shown too.
fn action_pieces(ui: &mut Ui, size: (f32, f32), label: &str, enabled: bool) -> (Pieces, String) {
    let ink = if enabled {
        ui.look.ink()
    } else {
        ui.look.ink2()
    };
    let (w, h) = size;
    let mut pieces = Pieces::default();
    if label.is_empty() {
        let side = legend_type(ui, ty::KEY).px;
        let (x, y) = ((w - side as f32) / 2.0, (h - side as f32) / 2.0);
        pieces.marks.push(Mark::icon(
            Icon::Enter,
            side,
            x.round() as i32,
            y.round() as i32,
            ink,
        ));
        (pieces, String::new())
    } else {
        let ty = action_type(ui, label, w);
        let text = ellipsise(&mut ui.fonts, label, ty, w - 2.0 * space::S3);
        pieces.texts.push(centred(
            ui,
            text.clone(),
            ty,
            ink,
            Frame::new(0.0, 0.0, w, h),
        ));
        (pieces, text)
    }
}

/// What the keyboard shows besides what a finger does.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardModel {
    /// What the action key says ("Connect"); empty shows the enter mark.
    pub action_label: String,
    /// Whether the action key can be used. One that cannot is dimmed, does not press, and types nothing.
    pub action_enabled: bool,
}

impl KeyboardModel {
    /// A model whose action key says `action_label` and cannot be used yet.
    pub fn new(action_label: impl Into<String>) -> Self {
        Self {
            action_label: action_label.into(),
            action_enabled: false,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.action_enabled = enabled;
        self
    }
}

/// What the keys show: the page, whether shift is on and whether the action key can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    page: Page,
    shifted: bool,
    action: bool,
}

/// One glass surface of the pool: a key's rest glass and its inset twin, moved to where the page puts its key.
struct Slot {
    group: Group,
    surface: Surface,
    /// Where it shows now, or `None` hidden.
    at: Option<Frame>,
}

impl Slot {
    fn new(ui: &Ui, frame: Frame, z: i32) -> Self {
        let mut group = Group::new(frame.centre());
        let surface = Surface::add_light(
            ui,
            &mut group,
            frame,
            z,
            SmartMaterial::Key,
            MaterialContext::Nested,
            ControlState::default(),
        );
        Self {
            group,
            surface,
            at: Some(frame),
        }
    }

    /// Shows the surface at `frame` in `state`, or hides it. Only what differs is sent.
    fn show(&mut self, ui: &Ui, frame: Option<Frame>, state: ControlState) {
        if frame != self.at {
            let (rest, pressed) = self.surface.parts();
            let mut moving = vec![rest];
            moving.extend(pressed);
            if let Some(f) = frame {
                for &i in &moving {
                    self.group.parts[i].frame = f;
                }
                self.group.set_parts(&moving, Pose::REST);
            } else {
                self.group.set_parts(&moving, Pose::hidden());
            }
            self.at = frame;
        }
        self.surface.set_state(ui, &mut self.group, state);
    }

    fn layers(&self) -> usize {
        self.group.parts.len()
    }

    /// The key's own glass gives way to the shared pressed one (`down`), or comes back, crossfading with it.
    fn yield_to_press(&mut self, ui: &Ui, down: bool) {
        let (rest, _) = self.surface.parts();
        self.group.parts[rest].rest_opacity = if down { 0.0 } else { 1.0 };
        let ms = ui.look.duration_ms(motion::PRESS_MS);
        self.group
            .animate_parts(&[rest], Pose::REST, ms, motion::SETTLE);
    }
}

/// The pressed glass of one class of key, made once and moved over whichever key of that class a finger is on.
struct PressedGlass {
    class: Class,
    group: Group,
}

impl PressedGlass {
    fn new(ui: &Ui, class: Class, frame: Frame, z: i32) -> Self {
        let glass = Surface::pressed_glass(
            ui,
            SmartMaterial::Key,
            MaterialContext::Nested,
            (frame.w, frame.h),
        );
        let mut group = Group::new(frame.centre());
        let mut part = Part::new(
            ui.scene.glass(
                frame.w.round() as u32,
                frame.h.round() as u32,
                z,
                glass,
                Props {
                    opacity: 0.0,
                    ..Props::at(frame.x, frame.y)
                },
            ),
            frame,
        );
        part.rest_opacity = 1.0;
        group.push(part);
        group.set(Pose::fade(0.0));
        Self { class, group }
    }

    /// Fades in over the key at `frame`, or out where it is.
    fn show(&mut self, ui: &Ui, frame: Option<Frame>) {
        let ms = ui.look.duration_ms(motion::PRESS_MS);
        match frame {
            Some(f) => {
                self.group.parts[0].frame = f;
                self.group.about = f.centre();
                self.group.set(Pose::fade(0.0));
                self.group.animate(Pose::REST, ms, motion::SETTLE);
            }
            None => self.group.animate(Pose::fade(0.0), ms, motion::SETTLE),
        }
    }

    fn restyle(&self, ui: &Ui) {
        let f = self.group.parts[0].frame;
        let glass =
            Surface::pressed_glass(ui, SmartMaterial::Key, MaterialContext::Nested, (f.w, f.h));
        if let Some(layer) = self.group.parts[0].glass() {
            layer.restyle(glass);
        }
    }
}

/// What the keyboard shows right now, so that only what differs from the model is sent.
struct Shown {
    /// The slot of the key pressed and its class.
    pressed: Option<(usize, Class)>,
    view: Option<View>,
    legend: Option<usize>,
}

pub struct Keyboard {
    model: Model,
    /// The containing sheet.
    frame: Frame,
    /// The area of the keys inside the sheet.
    keys: Frame,
    /// The sheet's depth: the keys are a step above it and the legends two.
    z: i32,
    sheet_group: Group,
    sheet: Surface,
    slots: Vec<Slot>,
    /// One pressed glass for each class of key (they differ in width): the key a finger is on crossfades with it.
    pressed_glass: Vec<PressedGlass>,
    /// Letters, letters shifted, symbols, more symbols: each made when it is first shown or prepared.
    legends: [Option<TextLayer>; 4],
    /// The action key's content, one layer for all pages (their action keys share a place), and its size.
    action: TextLayer,
    action_size: (f32, f32),
    action_label: String,
    action_text: String,
    shown: Shown,
}

impl Keyboard {
    /// What the keys take from top to bottom: four rows of 64 and three gaps.
    pub const KEYS_HEIGHT: f32 = ROWS as f32 * KEY_H + (ROWS as f32 - 1.0) * GAP;
    /// What a keyboard measures from top to bottom, its sheet included.
    pub const HEIGHT: f32 = Self::KEYS_HEIGHT + 2.0 * PAD;
    /// The narrowest sheet that keeps every key 64 wide (the narrow keys of the symbols page are the tightest).
    pub const MIN_WIDTH: f32 = 912.0 + 2.0 * PAD;

    /// A keyboard on a sheet covering `frame` (at least [`Keyboard::MIN_WIDTH`] by [`Keyboard::HEIGHT`]), the sheet at `z`, the keys
    /// above it and the legends above those. Builds every key now and the legend of the page that shows; the other legends are
    /// made by [`Keyboard::prepare`] (call it between frames while the keyboard is up) or when their page is first shown.
    pub fn new(ui: &mut Ui, frame: Frame, z: i32, model: &KeyboardModel) -> io::Result<Self> {
        if frame.w < Self::MIN_WIDTH || frame.h < Self::HEIGHT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a keyboard needs at least {} by {} units, not {} by {}",
                    Self::MIN_WIDTH,
                    Self::HEIGHT,
                    frame.w,
                    frame.h
                ),
            ));
        }
        let sheet_frame = Frame::new(frame.x, frame.y, frame.w, Self::HEIGHT);
        let keys = Frame::new(
            frame.x + PAD,
            frame.y + PAD,
            frame.w - 2.0 * PAD,
            Self::KEYS_HEIGHT,
        );
        let mut core = Model::new(sheet_frame, keys);
        core.action_enabled = model.action_enabled;

        let mut sheet_group = Group::new(sheet_frame.centre());
        let sheet = Surface::add(
            ui,
            &mut sheet_group,
            sheet_frame,
            z,
            SmartMaterial::ContainingSheet,
            MaterialContext::Independent,
            ControlState::default(),
        );
        // Each surface of the pool starts where the first key that uses it is; `sync` shows the right ones before the first commit.
        let slots = (0..core.slots)
            .map(|slot| {
                let first = core
                    .pages
                    .iter()
                    .flatten()
                    .find(|k| k.slot == slot)
                    .expect("every slot of the pool is some page's");
                Slot::new(ui, first.frame, z + 1)
            })
            .collect();
        let pressed_glass: Vec<PressedGlass> = CLASSES
            .iter()
            .map(|&class| {
                let first = core
                    .pages
                    .iter()
                    .flatten()
                    .find(|k| k.class == class)
                    .expect("every class of key is on some page");
                PressedGlass::new(ui, class, first.frame, z + 1)
            })
            .collect();
        let mut legends: [Option<TextLayer>; 4] = [None, None, None, None];
        let first = core.legend_layer();
        legends[first] = Some(Self::make_legend(ui, &core, keys, z, first)?);
        let action_key = core.pages[0]
            .iter()
            .find(|k| k.kind == KeyKind::Action)
            .expect("every page has the action key")
            .frame;
        let action_size = (action_key.w.ceil(), action_key.h.ceil());
        let (pieces, action_text) =
            action_pieces(ui, action_size, &model.action_label, model.action_enabled);
        let action = TextLayer::new(ui, action_key, z + 2, &pieces.items())?;
        let mut keyboard = Self {
            model: core,
            frame: sheet_frame,
            keys,
            z,
            sheet_group,
            sheet,
            slots,
            pressed_glass,
            legends,
            action,
            action_size,
            action_label: model.action_label.clone(),
            action_text,
            shown: Shown {
                pressed: None,
                view: None,
                legend: None,
            },
        };
        keyboard.sync(ui);
        Ok(keyboard)
    }

    /// The legend layer `which` (letters, letters shifted, symbols, more), drawn and placed over the keys, hidden until it is shown.
    fn make_legend(
        ui: &mut Ui,
        core: &Model,
        keys: Frame,
        z: i32,
        which: usize,
    ) -> io::Result<TextLayer> {
        let (page, shifted) = LEGENDS[which];
        let pieces = legend_pieces(ui, &core.pages[page.index()], (keys.x, keys.y), shifted);
        let mut layer = TextLayer::new(ui, keys, z + 2, &pieces.items())?;
        layer.set_props(Props {
            visible: false,
            ..Props::at(keys.x, keys.y)
        });
        Ok(layer)
    }

    /// Makes one legend that is not made yet. Whether one remains to make after it: the caller keeps calling between frames while
    /// it does, so that a page switch finds its legend ready.
    pub fn prepare(&mut self, ui: &mut Ui) -> bool {
        if let Some(which) = self.legends.iter().position(Option::is_none) {
            if let Ok(layer) = Self::make_legend(ui, &self.model, self.keys, self.z, which) {
                self.legends[which] = Some(layer);
            }
        }
        self.legends.iter().any(Option::is_none)
    }

    /// Rasters every glyph the keyboard draws at the sizes it draws them, so that no glyph is rastered while a finger is down. Call
    /// it once the person's text size is known, before the first touch.
    pub fn warm(ui: &mut Ui) {
        let types = [
            legend_type(ui, ty::KEY),
            legend_type(ui, ty::LABEL),
            legend_type(ui, ty::BUTTON),
        ];
        ui.warm(&types);
    }

    /// How many layers a built keyboard keeps: the sheet, the pool of key surfaces (one glass each: a key is only ever pressed by a touch, and the pressed glass is one for each class of key, shown over the key a finger is on), the
    /// legend layers made so far (four once prepared) and the action key's content.
    pub fn layer_count(&self) -> usize {
        self.sheet_group.parts.len()
            + self.slots.iter().map(Slot::layers).sum::<usize>()
            + self.pressed_glass.len()
            + self.legends.iter().flatten().count()
            + 1
    }

    /// Time passes: a held backspace that is due returns another [`KeyOutput::Backspace`]. Call it each frame while
    /// [`Keyboard::next_repeat_ns`] is `Some`.
    pub fn tick(&mut self, now_ns: u64) -> Option<KeyOutput> {
        self.model.tick(now_ns)
    }

    /// When a held backspace next repeats, on the input clock.
    pub fn next_repeat_ns(&self) -> Option<u64> {
        self.model.next_repeat()
    }

    /// Ends a press that something else has taken: no key is typed.
    pub fn cancel(&mut self, ui: &mut Ui) {
        self.model.contact = None;
        self.sync(ui);
    }

    /// The look was replaced: the glass is resolved again and every legend drawn in the new ink. Nothing is rebuilt.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.sheet.restyle(ui, &mut self.sheet_group);
        for slot in &mut self.slots {
            slot.surface.restyle(ui, &mut slot.group);
        }
        for glass in &self.pressed_glass {
            glass.restyle(ui);
        }
        let origin = (self.keys.x, self.keys.y);
        for (layer, (page, shifted)) in self.legends.iter_mut().zip(LEGENDS) {
            if let Some(layer) = layer {
                let pieces = legend_pieces(ui, &self.model.pages[page.index()], origin, shifted);
                layer.draw(ui, &pieces.items());
            }
        }
        self.redraw_action(ui);
    }

    fn redraw_action(&mut self, ui: &mut Ui) {
        let (pieces, text) = action_pieces(
            ui,
            self.action_size,
            &self.action_label,
            self.model.action_enabled,
        );
        self.action.draw(ui, &pieces.items());
        self.action_text = text;
    }

    /// The text the action key shows (cut if the label did not fit).
    pub fn action_text(&self) -> &str {
        &self.action_text
    }

    fn view(&self) -> View {
        View {
            page: self.model.page,
            shifted: self.model.shift != Shift::Off,
            action: self.model.action_enabled,
        }
    }

    fn key_state(kind: KeyKind, view: View) -> ControlState {
        match kind {
            KeyKind::Shift => ControlState {
                selected: view.shifted,
                ..ControlState::default()
            },
            KeyKind::Action => ControlState {
                enabled: view.action,
                selected: view.action,
                ..ControlState::default()
            },
            _ => ControlState::default(),
        }
    }

    /// Makes the layers show what the model says, sending only what differs.
    fn sync(&mut self, ui: &mut Ui) {
        // Let go of a key before its page can go away, so it is back at rest whichever page it was on.
        let want = self.model.pressed().map(|(page, i)| {
            let key = &self.model.pages[page.index()][i];
            (key.slot, key.class)
        });
        if self.shown.pressed != want {
            if let Some((slot, class)) = self.shown.pressed {
                self.slots[slot].yield_to_press(ui, false);
                if let Some(glass) = self.pressed_glass.iter_mut().find(|g| g.class == class) {
                    glass.show(ui, None);
                }
            }
            if let Some((slot, class)) = want
                && let Some(frame) = self.slots[slot].at
            {
                self.slots[slot].yield_to_press(ui, true);
                if let Some(glass) = self.pressed_glass.iter_mut().find(|g| g.class == class) {
                    glass.show(ui, Some(frame));
                }
            }
            self.shown.pressed = want;
        }
        let view = self.view();
        if self.shown.view != Some(view) {
            let mut place: Vec<Option<(Frame, ControlState)>> = vec![None; self.slots.len()];
            for key in self.model.keys() {
                place[key.slot] = Some((key.frame, Self::key_state(key.kind, view)));
            }
            for (slot, place) in self.slots.iter_mut().zip(place) {
                match place {
                    Some((frame, state)) => slot.show(ui, Some(frame), state),
                    None => slot.show(ui, None, ControlState::default()),
                }
            }
            self.shown.view = Some(view);
        }
        let layer = self.model.legend_layer();
        if self.shown.legend != Some(layer) {
            // A page shown before its legend's turn has it made now.
            if self.legends[layer].is_none() {
                self.legends[layer] =
                    Self::make_legend(ui, &self.model, self.keys, self.z, layer).ok();
            }
            for (i, legend) in self
                .legends
                .iter_mut()
                .enumerate()
                .flat_map(|(i, l)| l.as_mut().map(|l| (i, l)))
            {
                legend.set_props(Props {
                    visible: i == layer,
                    ..Props::at(self.keys.x, self.keys.y)
                });
            }
            self.shown.legend = Some(layer);
        }
    }

    /// The name a key is announced by.
    fn name(&self, kind: KeyKind) -> String {
        match kind {
            KeyKind::Char(c) if self.model.shift != Shift::Off => {
                c.to_ascii_uppercase().to_string()
            }
            KeyKind::Char(c) => c.to_string(),
            KeyKind::Shift if self.model.shift == Shift::Locked => "Caps lock".to_owned(),
            KeyKind::Shift => "Shift".to_owned(),
            KeyKind::Backspace => "Backspace".to_owned(),
            KeyKind::Space => "Space".to_owned(),
            KeyKind::To(Page::Letters) => "Letters".to_owned(),
            KeyKind::To(Page::Symbols) => "Numbers and symbols".to_owned(),
            KeyKind::To(Page::More) => "More symbols".to_owned(),
            KeyKind::Action if self.action_label.is_empty() => "Enter".to_owned(),
            KeyKind::Action => self.action_label.clone(),
        }
    }
}

impl Component for Keyboard {
    type Model = KeyboardModel;
    type Action = KeyOutput;

    fn update(&mut self, model: &KeyboardModel, ui: &mut Ui) -> Damage {
        let mut damage = Damage::None;
        if model.action_enabled != self.model.action_enabled {
            self.model.set_action_enabled(model.action_enabled);
            self.sync(ui);
            self.redraw_action(ui);
            damage = Damage::Content;
        }
        if model.action_label != self.action_label {
            self.action_label.clone_from(&model.action_label);
            self.redraw_action(ui);
            damage = Damage::Content;
        }
        damage
    }

    /// A touch. Returns what a committed key typed; the keys' look follows.
    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<KeyOutput> {
        let Event::Touch(touch) = event else {
            return None;
        };
        let output = self.model.touch(touch);
        self.sync(ui);
        output
    }

    /// The sheet: a touch on it, even between keys, is the keyboard's.
    fn frame(&self) -> Frame {
        self.frame
    }

    /// The keys of the page that shows; the other pages' keys are absent.
    fn semantics(&self, out: &mut SemanticsSink) {
        for key in self.model.keys() {
            let enabled = key.kind != KeyKind::Action || self.model.action_enabled;
            out.push(
                SemanticNode::new(SemanticKind::Key, self.name(key.kind), key.hit())
                    .actionable()
                    .enabled(enabled)
                    .selected(key.kind == KeyKind::Shift && self.model.shift != Shift::Off),
            );
        }
    }
}

/// Scripting a person's fingers, for the lab's usage loop, the host's previews and the tests of screens that have a keyboard.
impl Keyboard {
    fn keys_of(sheet: Frame) -> Frame {
        Frame::new(
            sheet.x + PAD,
            sheet.y + PAD,
            sheet.w - 2.0 * PAD,
            Self::KEYS_HEIGHT,
        )
    }

    /// The taps (centres of keys) that type `text` on a keyboard built on `sheet`, starting from its letters page.
    pub fn taps_for(sheet: Frame, text: &str) -> Vec<(f32, f32)> {
        taps_for(Self::keys_of(sheet), text)
    }

    /// The middle of the action key on a keyboard built on `sheet`; it is in the same place on every page.
    pub fn action_centre(sheet: Frame) -> (f32, f32) {
        Self::letters_key(sheet, KeyKind::Action)
    }

    /// The middle of the backspace key on a keyboard built on `sheet`, on its letters page (it is on the symbol pages too).
    pub fn backspace_centre(sheet: Frame) -> (f32, f32) {
        Self::letters_key(sheet, KeyKind::Backspace)
    }

    /// The middle of the shift key on a keyboard built on `sheet`, on its letters page.
    pub fn shift_centre(sheet: Frame) -> (f32, f32) {
        Self::letters_key(sheet, KeyKind::Shift)
    }

    /// The middle of the key that goes to the symbols page, on the letters page.
    pub fn symbols_centre(sheet: Frame) -> (f32, f32) {
        Self::letters_key(sheet, KeyKind::To(Page::Symbols))
    }

    fn letters_key(sheet: Frame, kind: KeyKind) -> (f32, f32) {
        let model = Model::new(sheet, Self::keys_of(sheet));
        model
            .keys()
            .iter()
            .find(|k| k.kind == kind)
            .expect("the letters page has the key")
            .frame
            .centre()
    }
}

/// The taps that type `text` on a keyboard whose key area is `keys`, starting from its letters page: shift before a capital, a visit
/// to the page a character is on and back. Planned on a model of its own, so what it plans is what the real keyboard does with the
/// same taps. For the lab's usage loop and the tests that need to type.
pub(crate) fn taps_for(keys: Frame, text: &str) -> Vec<(f32, f32)> {
    let mut model = Model::new(keys, keys);
    let mut taps = Vec::new();
    let mut clock = 0u64;
    let mut tap = |model: &mut Model, kind: KeyKind| {
        let f = model
            .keys()
            .iter()
            .find(|k| k.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} is not on {:?}", model.page))
            .frame;
        let (x, y) = f.centre();
        taps.push((x, y));
        clock += 1_000_000_000;
        for phase in [TouchPhase::Down, TouchPhase::Up] {
            model.touch(&TouchEvent {
                id: 1,
                phase,
                x,
                y,
                at_ns: clock,
            });
        }
    };
    for c in text.chars() {
        if c == ' ' {
            tap(&mut model, KeyKind::Space);
            continue;
        }
        let page = if c.is_ascii_alphabetic() {
            Page::Letters
        } else {
            [Page::Symbols, Page::More]
                .into_iter()
                .find(|&p| layout(p, keys).iter().any(|k| k.kind == KeyKind::Char(c)))
                .unwrap_or_else(|| panic!("{c:?} is on no page"))
        };
        while model.page != page {
            // The letters are two pages from the second symbols page, through the first.
            let via = match (model.page, page) {
                (Page::Letters, Page::More) => Page::Symbols,
                (_, to) => to,
            };
            tap(&mut model, KeyKind::To(via));
        }
        if page == Page::Letters {
            if c.is_ascii_uppercase() {
                tap(&mut model, KeyKind::Shift);
            }
            tap(&mut model, KeyKind::Char(c.to_ascii_lowercase()));
        } else {
            tap(&mut model, KeyKind::Char(c));
        }
    }
    taps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text_field::{TextField, TextFieldModel};
    use crate::context::Look;
    use crate::lucent::palette::Mode;
    use crate::testing::{Harness, harness};
    use KeyKind::{Backspace, Char, Space, To};
    use compositor_protocol::LayerId;
    use compositor_protocol::{Content, Request};
    use std::collections::BTreeSet;

    const SHIFT: KeyKind = KeyKind::Shift;
    const SHEET: Frame = Frame::new(12.0, 192.0, Keyboard::MIN_WIDTH, Keyboard::HEIGHT);
    const MS: u64 = 1_000_000;
    const SECOND: u64 = 1_000 * MS;

    fn keys_of(sheet: Frame) -> Frame {
        Frame::new(
            sheet.x + PAD,
            sheet.y + PAD,
            sheet.w - 2.0 * PAD,
            Keyboard::KEYS_HEIGHT,
        )
    }

    fn new_model() -> Model {
        Model::new(SHEET, keys_of(SHEET))
    }

    fn touch(phase: TouchPhase, (x, y): (f32, f32), at_ns: u64) -> TouchEvent {
        TouchEvent {
            id: 1,
            phase,
            x,
            y,
            at_ns,
        }
    }

    fn frame_of(model: &Model, kind: KeyKind) -> Frame {
        model
            .keys()
            .iter()
            .find(|k| k.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} is not on {:?}", model.page))
            .frame
    }

    fn centre(model: &Model, kind: KeyKind) -> (f32, f32) {
        frame_of(model, kind).centre()
    }

    /// A person typing on a model: each tap is a finger down and up on the key's middle, a second apart unless said otherwise.
    struct Typist {
        model: Model,
        now: u64,
    }

    impl Typist {
        fn new() -> Self {
            Self {
                model: new_model(),
                now: 0,
            }
        }

        fn tap_after(&mut self, kind: KeyKind, wait_ns: u64) -> Option<KeyOutput> {
            self.now += wait_ns;
            let at = centre(&self.model, kind);
            self.model.touch(&touch(TouchPhase::Down, at, self.now));
            self.now += 50 * MS;
            self.model.touch(&touch(TouchPhase::Up, at, self.now))
        }

        fn tap(&mut self, kind: KeyKind) -> Option<KeyOutput> {
            self.tap_after(kind, SECOND)
        }

        fn type_keys(&mut self, text: &mut String, kinds: &[KeyKind]) {
            for &kind in kinds {
                match self.tap(kind) {
                    Some(KeyOutput::Char(c)) => text.push(c),
                    Some(KeyOutput::Backspace) => {
                        text.pop();
                    }
                    Some(KeyOutput::Action) | None => {}
                }
            }
        }
    }

    // ---- the model: what taps type ----

    #[test]
    fn a_typed_sequence_with_shift_pages_symbols_and_backspace_makes_exactly_that_text() {
        let mut typist = Typist::new();
        let mut text = String::new();
        typist.type_keys(
            &mut text,
            &[
                SHIFT,
                Char('h'),
                Char('i'),
                To(Page::Symbols),
                Char('-'),
                Char('5'),
                To(Page::More),
                Char('_'),
                Char('~'),
                Backspace,
                To(Page::Symbols),
                Char('!'),
                To(Page::Letters),
                Char('o'),
                Space,
                SHIFT,
                SHIFT,
                Char('z'),
            ],
        );
        assert_eq!(text, "Hi-5_!o z");
    }

    #[test]
    fn the_taps_planned_for_a_text_type_exactly_that_text_whatever_it_is() {
        let every: String = (' '..='~').collect();
        let backwards: String = every.chars().rev().collect();
        for text in [
            "",
            "hunter2hunter2",
            "Correct Horse Battery Staple 9!",
            "P@ssw0rd! Wi-Fi_2024~ {[(<|>)]} \\ ^ ` ' \"",
            every.as_str(),
            backwards.as_str(),
        ] {
            let mut model = new_model();
            let mut typed = String::new();
            for (i, (x, y)) in taps_for(keys_of(SHEET), text).into_iter().enumerate() {
                let at = (i as u64 + 1) * SECOND;
                model.touch(&touch(TouchPhase::Down, (x, y), at));
                if let Some(KeyOutput::Char(c)) =
                    model.touch(&touch(TouchPhase::Up, (x, y), at + 50 * MS))
                {
                    typed.push(c);
                }
            }
            assert_eq!(typed, text);
        }
    }

    #[test]
    fn shift_is_spent_by_one_character_and_dropped_by_leaving_the_letters() {
        let mut typist = Typist::new();
        typist.tap(SHIFT);
        assert_eq!(typist.tap(Char('a')), Some(KeyOutput::Char('A')));
        assert_eq!(
            typist.tap(Char('a')),
            Some(KeyOutput::Char('a')),
            "the second letter is lower case again"
        );
        typist.tap(SHIFT);
        typist.tap(To(Page::Symbols));
        typist.tap(To(Page::Letters));
        assert_eq!(
            typist.tap(Char('q')),
            Some(KeyOutput::Char('q')),
            "a shift armed before a visit to the symbols is gone"
        );
        typist.tap(SHIFT);
        assert_eq!(
            typist.tap(Space),
            Some(KeyOutput::Char(' ')),
            "a space is a character: it spends shift"
        );
        assert_eq!(typist.tap(Char('m')), Some(KeyOutput::Char('m')));
    }

    #[test]
    fn two_quick_taps_of_shift_lock_capitals_until_shift_or_the_page_ends_them() {
        let mut typist = Typist::new();
        typist.tap(SHIFT);
        typist.tap_after(SHIFT, 100 * MS);
        assert_eq!(typist.model.shift, Shift::Locked);
        for c in ['a', 'b', 'c'] {
            assert_eq!(
                typist.tap(Char(c)),
                Some(KeyOutput::Char(c.to_ascii_uppercase())),
                "a locked shift is not spent"
            );
        }
        assert_eq!(typist.model.legend_layer(), 1, "the capitals legend stays");
        typist.tap(SHIFT);
        assert_eq!(typist.model.shift, Shift::Off, "one tap unlocks");
        assert_eq!(typist.tap(Char('d')), Some(KeyOutput::Char('d')));

        // Locked, then a visit to the symbols: dropped.
        typist.tap(SHIFT);
        typist.tap_after(SHIFT, 100 * MS);
        typist.tap(To(Page::Symbols));
        typist.tap(To(Page::Letters));
        assert_eq!(typist.tap(Char('e')), Some(KeyOutput::Char('e')));

        // Two slow taps are an arm and a disarm, not a lock.
        typist.tap(SHIFT);
        typist.tap_after(SHIFT, SECOND);
        assert_eq!(typist.model.shift, Shift::Off);
    }

    #[test]
    fn every_printable_ascii_character_can_be_typed() {
        let mut typed = BTreeSet::new();
        for shifted in [false, true] {
            for page in PAGES {
                let mut typist = Typist::new();
                typist.model.page = page;
                let wanted: Vec<KeyKind> = typist
                    .model
                    .keys()
                    .iter()
                    .filter(|k| matches!(k.kind, Char(_) | Space))
                    .map(|k| k.kind)
                    .collect();
                for kind in wanted {
                    typist.model.page = page;
                    typist.model.shift = if shifted && page == Page::Letters {
                        Shift::Armed
                    } else {
                        Shift::Off
                    };
                    match typist.tap(kind) {
                        Some(KeyOutput::Char(t)) => {
                            typed.insert(t);
                        }
                        other => panic!("{kind:?} typed {other:?}"),
                    }
                }
            }
        }
        let missing: Vec<char> = (' '..='~').filter(|c| !typed.contains(c)).collect();
        assert!(missing.is_empty(), "cannot be typed: {missing:?}");
        assert!(
            typed.iter().all(|c| c.is_ascii_graphic() || *c == ' '),
            "only printable ASCII: {typed:?}"
        );
    }

    #[test]
    fn the_letters_have_no_digit_row_and_numbers_and_symbols_sit_behind_the_modifier_key() {
        let chars_of = |page: Page| -> String {
            let model = new_model();
            model.pages[page.index()]
                .iter()
                .filter_map(|k| match k.kind {
                    Char(c) => Some(c),
                    _ => None,
                })
                .collect()
        };
        // The letter page is the 26 letters in three rows, at the same 64-high keys as ever, and not a digit on it.
        assert_eq!(chars_of(Page::Letters), "qwertyuiopasdfghjklzxcvbnm");
        let model = new_model();
        let rows: BTreeSet<i32> = model.pages[Page::Letters.index()]
            .iter()
            .map(|k| k.frame.y as i32)
            .collect();
        assert_eq!(rows.len(), 4, "three rows of letters and the bottom row");
        assert!(
            model.pages[Page::Letters.index()]
                .iter()
                .all(|k| k.frame.h == KEY_H)
        );
        // "?123": the digits and the common symbols; "#+=": the rest.
        let numbers = chars_of(Page::Symbols);
        for c in "0123456789-/:;()$&@\".,?!'".chars() {
            assert!(numbers.contains(c), "{c:?} is on the ?123 page");
        }
        let more = chars_of(Page::More);
        for c in "[]{}^*_\\|~<>`".chars() {
            assert!(more.contains(c), "{c:?} is on the #+= page");
        }
        assert!(more.chars().all(|c| !c.is_ascii_alphanumeric()));
        let legend = |kind| legend_text(kind, false).map(|(t, _)| t);
        assert_eq!(legend(To(Page::Symbols)).as_deref(), Some("?123"));
        assert_eq!(legend(To(Page::More)).as_deref(), Some("#+="));
        assert_eq!(legend(To(Page::Letters)).as_deref(), Some("ABC"));
        let mut typist = Typist::new();
        assert_eq!(typist.tap(To(Page::Symbols)), None);
        assert_eq!(typist.model.page, Page::Symbols);
        assert_eq!(typist.tap(To(Page::More)), None);
        assert_eq!(typist.model.page, Page::More);
        assert_eq!(typist.tap(To(Page::Letters)), None);
        assert_eq!(typist.model.page, Page::Letters);
    }

    #[test]
    fn a_password_with_every_kind_of_printable_character_is_typed_exactly_by_the_planned_taps() {
        let password = "P@ss w0rd!#\"'\\[]{}~^|<>`%*+=_-/:;(),.?$&Zz9";
        let keys = keys_of(SHEET);
        let mut model = Model::new(keys, keys);
        let mut typed = String::new();
        let mut at = 0u64;
        for (x, y) in taps_for(keys, password) {
            for phase in [TouchPhase::Down, TouchPhase::Up] {
                at += SECOND;
                if let Some(KeyOutput::Char(c)) = model.touch(&touch(phase, (x, y), at)) {
                    typed.push(c);
                }
            }
        }
        assert_eq!(typed, password);
    }

    #[test]
    fn a_finger_that_slides_off_its_key_types_nothing_even_if_it_lifts_back_on_it() {
        let mut model = new_model();
        let a = centre(&model, Char('a'));
        let s = centre(&model, Char('s'));
        // Down on a, slide to s, lift on s: neither.
        model.touch(&touch(TouchPhase::Down, a, 0));
        assert!(model.pressed().is_some());
        model.touch(&touch(TouchPhase::Move, s, 10 * MS));
        assert_eq!(
            model.pressed(),
            None,
            "the key is let go as the finger leaves"
        );
        assert_eq!(model.touch(&touch(TouchPhase::Up, s, 20 * MS)), None);
        // Down on a, off the keyboard, back on a: still nothing.
        model.touch(&touch(TouchPhase::Down, a, SECOND));
        model.touch(&touch(TouchPhase::Move, (500.0, 20.0), SECOND + 10 * MS));
        model.touch(&touch(TouchPhase::Move, a, SECOND + 20 * MS));
        assert_eq!(
            model.touch(&touch(TouchPhase::Up, a, SECOND + 30 * MS)),
            None
        );
        // A long press that stays on the key types it on release.
        model.touch(&touch(TouchPhase::Down, a, 2 * SECOND));
        let near = (a.0 + 10.0, a.1 + 10.0);
        model.touch(&touch(TouchPhase::Move, near, 2 * SECOND + 10 * MS));
        assert_eq!(
            model.touch(&touch(TouchPhase::Up, near, 2 * SECOND + 900 * MS)),
            Some(KeyOutput::Char('a'))
        );
    }

    #[test]
    fn a_lost_contact_types_nothing_and_a_second_finger_is_ignored_while_the_first_is_down() {
        let mut model = new_model();
        let a = centre(&model, Char('a'));
        let s = centre(&model, Char('s'));
        model.touch(&touch(TouchPhase::Down, a, 0));
        assert_eq!(
            model.touch(&touch(TouchPhase::Cancel, (0.0, 0.0), 10 * MS)),
            None
        );
        assert_eq!(model.pressed(), None);
        assert_eq!(
            model.touch(&touch(TouchPhase::Up, a, 20 * MS)),
            None,
            "nothing is left to commit"
        );

        model.touch(&touch(TouchPhase::Down, a, SECOND));
        let other = |phase, at| TouchEvent {
            id: 2,
            ..touch(phase, s, at)
        };
        assert_eq!(model.touch(&other(TouchPhase::Down, SECOND + MS)), None);
        assert_eq!(
            model.touch(&other(TouchPhase::Up, SECOND + 2 * MS)),
            None,
            "the palm types nothing"
        );
        assert!(
            model.pressed().is_some(),
            "the first finger still holds its key"
        );
        assert_eq!(
            model.touch(&touch(TouchPhase::Up, a, SECOND + 3 * MS)),
            Some(KeyOutput::Char('a'))
        );
    }

    #[test]
    fn a_touch_outside_the_sheet_is_not_the_keyboards() {
        let mut model = new_model();
        let above = (SHEET.x + 100.0, SHEET.y - 5.0);
        assert_eq!(model.touch(&touch(TouchPhase::Down, above, 0)), None);
        assert!(model.contact.is_none());
    }

    #[test]
    fn the_action_key_types_only_while_it_can_be_used_and_does_not_press_when_it_cannot() {
        let mut typist = Typist::new();
        let at = centre(&typist.model, KeyKind::Action);
        typist.model.touch(&touch(TouchPhase::Down, at, 0));
        assert_eq!(typist.model.pressed(), None, "a dimmed key gives no press");
        assert_eq!(
            typist.model.touch(&touch(TouchPhase::Up, at, 10 * MS)),
            None
        );
        typist.model.action_enabled = true;
        typist.model.touch(&touch(TouchPhase::Down, at, SECOND));
        assert!(typist.model.pressed().is_some());
        assert_eq!(
            typist
                .model
                .touch(&touch(TouchPhase::Up, at, SECOND + 10 * MS)),
            Some(KeyOutput::Action)
        );
        assert_eq!(typist.model.pressed(), None);
        // Disabled while a finger is on it: the press is let go and the release types nothing.
        typist.model.touch(&touch(TouchPhase::Down, at, 2 * SECOND));
        typist.model.set_action_enabled(false);
        assert_eq!(typist.model.pressed(), None);
        assert_eq!(
            typist
                .model
                .touch(&touch(TouchPhase::Up, at, 2 * SECOND + MS)),
            None
        );
    }

    // ---- the model: holding backspace ----

    #[test]
    fn holding_backspace_repeats_after_a_delay_and_releasing_after_repeats_types_nothing_more() {
        let mut model = new_model();
        let at = centre(&model, Backspace);
        model.touch(&touch(TouchPhase::Down, at, 0));
        assert_eq!(model.next_repeat(), Some(400 * MS));
        assert_eq!(
            model.tick(399 * MS),
            None,
            "a hold shorter than the delay is not a repeat"
        );
        assert_eq!(model.tick(400 * MS), Some(KeyOutput::Backspace));
        assert_eq!(
            model.tick(401 * MS),
            None,
            "one every interval, not one every tick"
        );
        assert_eq!(model.tick(460 * MS), Some(KeyOutput::Backspace));
        assert_eq!(model.next_repeat(), Some(520 * MS));
        assert_eq!(
            model.touch(&touch(TouchPhase::Up, at, 470 * MS)),
            None,
            "the release of a key that repeated deletes nothing more"
        );
        assert_eq!(
            model.tick(10 * SECOND),
            None,
            "nothing repeats once the finger is up"
        );
    }

    #[test]
    fn a_tap_on_backspace_deletes_once_and_a_stalled_clock_gives_one_repeat_not_a_burst() {
        let mut typist = Typist::new();
        assert_eq!(typist.tap(Backspace), Some(KeyOutput::Backspace));
        assert_eq!(typist.model.tick(10 * SECOND), None);

        let mut model = new_model();
        let at = centre(&model, Backspace);
        model.touch(&touch(TouchPhase::Down, at, 0));
        assert_eq!(model.tick(2 * SECOND), Some(KeyOutput::Backspace));
        assert_eq!(model.tick(2 * SECOND), None);
        assert_eq!(model.tick(2 * SECOND + 60 * MS), Some(KeyOutput::Backspace));
    }

    #[test]
    fn a_repeat_stops_when_the_finger_slides_off_or_the_contact_is_lost() {
        for end in [TouchPhase::Move, TouchPhase::Cancel] {
            let mut model = new_model();
            let at = centre(&model, Backspace);
            model.touch(&touch(TouchPhase::Down, at, 0));
            assert_eq!(model.tick(400 * MS), Some(KeyOutput::Backspace));
            let away = (at.0, at.1 - 300.0);
            model.touch(&touch(end, away, 450 * MS));
            assert_eq!(model.tick(900 * MS), None, "{end:?}");
            assert_eq!(model.next_repeat(), None, "{end:?}");
        }
    }

    #[test]
    fn only_backspace_repeats_a_held_character_types_once_on_release() {
        let mut model = new_model();
        let a = centre(&model, Char('a'));
        model.touch(&touch(TouchPhase::Down, a, 0));
        assert_eq!(model.next_repeat(), None);
        assert_eq!(model.tick(2 * SECOND), None);
        assert_eq!(
            model.touch(&touch(TouchPhase::Up, a, 2 * SECOND)),
            Some(KeyOutput::Char('a'))
        );
    }

    // ---- the layout ----

    #[test]
    fn a_finger_between_two_keys_types_one_of_them_and_no_spot_is_on_two() {
        let model = new_model();
        let q = frame_of(&model, Char('q'));
        let gap_x = q.x + q.w + GAP / 2.0 - 1.0;
        let y = q.y + q.h / 2.0;
        assert_eq!(
            model.key_at(gap_x, y).map(|i| model.keys()[i].kind),
            Some(Char('q'))
        );
        assert_eq!(
            model.key_at(gap_x + 2.0, y).map(|i| model.keys()[i].kind),
            Some(Char('w')),
            "just past the middle of the gap is the next key"
        );
        // The same between rows.
        let between = q.y + q.h + GAP / 2.0;
        let x = q.x + q.w / 2.0;
        assert_eq!(
            model.key_at(x, between - 1.0).map(|i| model.keys()[i].kind),
            Some(Char('q'))
        );
        assert!(
            model
                .key_at(x, between + 1.0)
                .is_some_and(|i| model.keys()[i].kind != Char('q'))
        );
        for page in PAGES {
            let keys = &new_model().pages[page.index()];
            for (i, a) in keys.iter().enumerate() {
                for b in &keys[i + 1..] {
                    let (p, q) = (a.hit(), b.hit());
                    let apart = p.x + p.w <= q.x
                        || q.x + q.w <= p.x
                        || p.y + p.h <= q.y
                        || q.y + q.h <= p.y;
                    assert!(apart, "{:?} and {:?} overlap on {page:?}", a.kind, b.kind);
                }
            }
        }
    }

    #[test]
    fn keys_are_at_least_64_fit_the_frame_span_their_rows_and_sit_8_apart_on_every_page() {
        for sheet_w in [Keyboard::MIN_WIDTH, 944.0, 960.0] {
            let keys_frame = keys_of(Frame::new(0.0, 100.0, sheet_w, Keyboard::HEIGHT));
            for page in PAGES {
                let keys = layout(page, keys_frame);
                for key in &keys {
                    let f = key.frame;
                    assert!(
                        f.w >= 64.0 && f.h >= 64.0,
                        "{:?} on {page:?} is {}x{}",
                        key.kind,
                        f.w,
                        f.h
                    );
                    assert!(
                        f.x >= keys_frame.x
                            && f.right() <= keys_frame.right()
                            && f.y >= keys_frame.y
                            && f.bottom() <= keys_frame.bottom(),
                        "{:?} on {page:?} leaves the frame",
                        key.kind
                    );
                }
                for (i, a) in keys.iter().enumerate() {
                    for b in &keys[i + 1..] {
                        assert!(
                            a.frame.intersect(&b.frame).is_none(),
                            "{:?} overlaps {:?} on {page:?}",
                            a.kind,
                            b.kind
                        );
                    }
                }
                // Neighbours in a row are a gap apart, and rows are a gap apart.
                for pair in keys.windows(2) {
                    let (p, q) = (pair[0].frame, pair[1].frame);
                    if p.y == q.y && q.x > p.x {
                        let between = q.x - p.right();
                        // The room left in a row with nothing in it is wider than a gap; every other pair is 8.
                        assert!(
                            between == GAP || (page == Page::More && between > GAP),
                            "{:?} and {:?} on {page:?} are {between} apart",
                            pair[0].kind,
                            pair[1].kind
                        );
                    }
                }
                let ys: BTreeSet<i32> = keys.iter().map(|k| k.frame.y as i32).collect();
                assert_eq!(ys.len(), ROWS, "four rows on {page:?}");
                let ys: Vec<i32> = ys.into_iter().collect();
                assert!(ys.windows(2).all(|w| w[1] - w[0] == 72));
                // The full rows span the width: the first row of letters and symbols holds ten whole units, so it is centred with at
                // most the remainder of the division (under a unit's ten pixels, half each side) left over; the bottom row of every
                // page (the page key, the space bar, the action key) is flush with both edges.
                for row in [0usize, 3] {
                    let y = keys_frame.y + row as f32 * (KEY_H + GAP);
                    let in_row: Vec<&KeyDef> = keys.iter().filter(|k| k.frame.y == y).collect();
                    if row == 3 || page == Page::Symbols || page == Page::Letters {
                        let left = in_row.iter().map(|k| k.frame.x).fold(f32::MAX, f32::min);
                        let right = in_row.iter().map(|k| k.frame.right()).fold(0.0, f32::max);
                        if row == 3 {
                            assert_eq!(
                                (left, right),
                                (keys_frame.x, keys_frame.right()),
                                "bottom row on {page:?}"
                            );
                        } else {
                            let (l, r) = (left - keys_frame.x, keys_frame.right() - right);
                            assert!(
                                l.max(r) <= 5.0 && (l - r).abs() <= 1.0,
                                "row {row} on {page:?}: {l} and {r} left over"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_pool_gives_every_key_of_a_page_its_own_surface_of_its_own_size() {
        let model = new_model();
        assert_eq!(model.slots, 40, "about forty glass pairs serve every page");
        let mut size_of: Vec<Option<(f32, f32)>> = vec![None; model.slots];
        for page in PAGES {
            let keys = &model.pages[page.index()];
            let slots: BTreeSet<usize> = keys.iter().map(|k| k.slot).collect();
            assert_eq!(
                slots.len(),
                keys.len(),
                "two keys of {page:?} share a surface"
            );
            for key in keys {
                let size = (key.frame.w, key.frame.h);
                assert_eq!(
                    *size_of[key.slot].get_or_insert(size),
                    size,
                    "{:?} on {page:?}",
                    key.kind
                );
            }
        }
        // The action key and the space bar are in one place on every page.
        for kind in [KeyKind::Action, Space] {
            let places: Vec<(Frame, usize)> = PAGES
                .iter()
                .map(|&p| {
                    let k = model.pages[p.index()]
                        .iter()
                        .find(|k| k.kind == kind)
                        .expect("the key");
                    (k.frame, k.slot)
                })
                .collect();
            assert!(
                places.windows(2).all(|w| w[0] == w[1]),
                "{kind:?}: {places:?}"
            );
        }
    }

    #[test]
    fn the_legend_layer_follows_the_page_and_shift() {
        let mut typist = Typist::new();
        assert_eq!(typist.model.legend_layer(), 0);
        typist.tap(SHIFT);
        assert_eq!(typist.model.legend_layer(), 1);
        typist.tap(Char('a'));
        assert_eq!(
            typist.model.legend_layer(),
            0,
            "shift spent: lower case again"
        );
        typist.tap(To(Page::Symbols));
        assert_eq!(typist.model.legend_layer(), 2);
        typist.tap(To(Page::More));
        assert_eq!(typist.model.legend_layer(), 3);
    }

    // ---- the layers ----

    /// A keyboard with every legend made, as one is after the frames that follow its building.
    fn built(h: &mut Harness, model: &KeyboardModel) -> Keyboard {
        let mut kb = Keyboard::new(&mut h.ui, SHEET, 10, model).unwrap();
        while kb.prepare(&mut h.ui) {}
        h.commit();
        kb
    }

    fn legend_id(kb: &Keyboard, which: usize) -> LayerId {
        kb.legends[which].as_ref().expect("the legend is made").id()
    }

    fn send(
        kb: &mut Keyboard,
        h: &mut Harness,
        phase: TouchPhase,
        at: (f32, f32),
        at_ns: u64,
    ) -> Option<KeyOutput> {
        kb.input(&Event::Touch(touch(phase, at, at_ns)), &mut h.ui)
    }

    fn tap_key(kb: &mut Keyboard, h: &mut Harness, kind: KeyKind, at_ns: u64) -> Option<KeyOutput> {
        let at = centre(&kb.model, kind);
        send(kb, h, TouchPhase::Down, at, at_ns);
        send(kb, h, TouchPhase::Up, at, at_ns + 50 * MS)
    }

    fn creates(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Create { .. }))
            .count()
    }

    fn damages(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Damage { .. }))
            .count()
    }

    fn animates(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Animate { .. }))
            .count()
    }

    fn restyles(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Restyle { .. }))
            .count()
    }

    fn visible(h: &Harness, id: compositor_protocol::LayerId) -> bool {
        h.layers()
            .iter()
            .find(|l| l.0 == id)
            .expect("the layer")
            .2
            .visible
    }

    fn brightest(pixels: &[u8]) -> [u8; 4] {
        pixels
            .chunks_exact(4)
            .max_by_key(|p| p[3])
            .map(|p| [p[0], p[1], p[2], p[3]])
            .expect("pixels")
    }

    #[test]
    fn a_built_keyboard_keeps_the_layers_it_reports_and_every_key_is_glass_with_its_depth_pair() {
        let mut h = harness();
        let mut kb = Keyboard::new(&mut h.ui, SHEET, 10, &KeyboardModel::new("Connect")).unwrap();
        let requests = h.commit();
        let made = creates(&requests);
        assert_eq!(made, kb.layer_count());
        // 40 keys with one glass each, the pressed glass of each of the five classes of key, the sheet with its ring, the legend of
        // the page that shows and the action key; the other three legends come one per `prepare`, between the frames that follow.
        assert_eq!(made, 40 + 5 + 2 + 1 + 1);
        assert!(kb.prepare(&mut h.ui));
        assert!(kb.prepare(&mut h.ui));
        assert!(!kb.prepare(&mut h.ui), "the fourth legend was the last");
        assert_eq!(creates(&h.commit()), 3);
        assert_eq!(kb.layer_count(), 40 + 5 + 2 + 4 + 1);
        let glass = h
            .layers()
            .iter()
            .filter(|(_, c, _)| matches!(c, Content::Glass(_)))
            .count();
        assert_eq!(glass, 40 + 5 + 1);
        let mut disabled = 0;
        for slot in &kb.slots {
            let (rest, pressed) = slot.surface.shown();
            assert!(rest.depth.is_some_and(|d| !d.inset) && rest.sheen.is_some());
            assert_eq!(pressed, None, "a key has no pressed twin of its own");
            if !slot.surface.state().enabled {
                disabled += 1;
            }
            assert_eq!(rest.corner_radius, 14.0);
        }
        assert_eq!(disabled, 1, "only the action key waits to be enabled");
        for shared in &kb.pressed_glass {
            let f = shared.group.parts[0].frame;
            let p = Surface::pressed_glass(
                &h.ui,
                SmartMaterial::Key,
                MaterialContext::Nested,
                (f.w, f.h),
            );
            assert!(p.depth.is_some_and(|d| d.inset) && p.sheen.is_none());
            assert_eq!(p.corner_radius, 14.0);
        }
        let sheet = kb.sheet.pair().rest;
        assert_eq!(sheet.corner_radius, 28.0);
        assert!(
            sheet.depth.is_none()
                && sheet.backdrop != crate::lucent::material::FieldTokens::NO_BACKDROP
        );
    }

    #[test]
    fn a_press_is_a_crossfade_of_the_prepared_pair_and_makes_no_layer_and_draws_nothing() {
        let mut h = harness();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let a = centre(&kb.model, Char('a'));
        assert_eq!(send(&mut kb, &mut h, TouchPhase::Down, a, 0), None);
        let down = h.commit();
        assert!(
            animates(&down) >= 2,
            "rest and pressed glass crossfade: {down:?}"
        );
        assert_eq!(
            creates(&down) + damages(&down) + restyles(&down),
            0,
            "{down:?}"
        );
        // The shared pressed glass is moved over the key (a set), then it and the key's own glass crossfade; nothing is made.
        assert!(
            down.iter().all(|r| matches!(
                r,
                Request::Animate { .. } | Request::Set { .. } | Request::Commit { .. }
            )),
            "{down:?}"
        );
        let (_, class) = kb.shown.pressed.expect("a key is down");
        let pressed = kb
            .pressed_glass
            .iter()
            .find(|g| g.class == class)
            .and_then(|g| g.group.parts[0].glass())
            .map(|_| ())
            .is_some();
        assert!(pressed, "the pressed glass of its class is up");

        assert_eq!(
            send(&mut kb, &mut h, TouchPhase::Up, a, 50 * MS),
            Some(KeyOutput::Char('a'))
        );
        let up = h.commit();
        assert!(animates(&up) >= 2, "and fade back");
        assert!(
            up.iter()
                .all(|r| matches!(r, Request::Animate { .. } | Request::Commit { .. })),
            "{up:?}"
        );
    }

    #[test]
    fn a_key_pressed_with_reduced_motion_still_crossfades_and_never_scales() {
        let mut h = harness();
        h.ui.look.reduced_motion = true;
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let a = centre(&kb.model, Char('a'));
        send(&mut kb, &mut h, TouchPhase::Down, a, 0);
        let down = h.commit();
        let times: Vec<(f32, u32)> = down
            .iter()
            .filter_map(|r| match r {
                Request::Animate {
                    to, duration_ms, ..
                } => Some((to.scale, *duration_ms)),
                _ => None,
            })
            .collect();
        assert!(times.len() >= 2);
        assert!(
            times.iter().all(|&(scale, ms)| scale == 1.0 && ms <= 120),
            "{times:?}"
        );
    }

    #[test]
    fn a_page_switch_only_moves_surfaces_and_shows_another_legend_layer() {
        let mut h = harness();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let ids: Vec<_> = (0..4).map(|i| legend_id(&kb, i)).collect();
        assert_eq!(
            ids.iter().map(|&id| visible(&h, id)).collect::<Vec<_>>(),
            [true, false, false, false]
        );
        assert_eq!(tap_key(&mut kb, &mut h, To(Page::Symbols), SECOND), None);
        let requests = h.commit();
        assert_eq!(
            creates(&requests) + damages(&requests),
            0,
            "no layer, no text: {requests:?}"
        );
        assert_eq!(restyles(&requests), 0);
        assert_eq!(
            ids.iter().map(|&id| visible(&h, id)).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        // The symbols page shows its own keys: a "1" can be typed, the letter q cannot.
        assert_eq!(
            tap_key(&mut kb, &mut h, Char('1'), 2 * SECOND),
            Some(KeyOutput::Char('1'))
        );
        assert!(kb.model.keys().iter().all(|k| k.kind != Char('q')));
        // The surfaces of keys the page does not have are hidden, not left behind.
        let shown = kb.slots.iter().filter(|s| s.at.is_some()).count();
        assert_eq!(shown, kb.model.keys().len());
        let hidden_visible = h
            .layers()
            .iter()
            .filter(|(_, c, p)| matches!(c, Content::Glass(_)) && p.visible)
            .count();
        // The sheet, the glass of every shown key and the five shared pressed glasses (at opacity 0, but not hidden).
        assert_eq!(hidden_visible, 1 + shown + 5);
        tap_key(&mut kb, &mut h, To(Page::More), 3 * SECOND);
        tap_key(&mut kb, &mut h, To(Page::Letters), 4 * SECOND);
        h.commit();
        assert_eq!(
            ids.iter().map(|&id| visible(&h, id)).collect::<Vec<_>>(),
            [true, false, false, false]
        );
    }

    #[test]
    fn shift_washes_its_key_and_shows_the_capitals_legend_without_drawing() {
        let mut h = harness();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let shift_slot = kb
            .model
            .keys()
            .iter()
            .find(|k| k.kind == SHIFT)
            .unwrap()
            .slot;
        let plain = kb.slots[shift_slot].surface.pair().rest.tint;
        tap_key(&mut kb, &mut h, SHIFT, SECOND);
        let requests = h.commit();
        assert_eq!(creates(&requests) + damages(&requests), 0, "{requests:?}");
        assert!(
            restyles(&requests) >= 1,
            "the shift glass takes its selected wash"
        );
        let selected = kb.slots[shift_slot].surface.pair().rest.tint;
        assert_ne!(selected, plain);
        assert_eq!(selected, h.ui.look.theme.materials.glass_raised_selected);
        assert!(visible(&h, legend_id(&kb, 1)) && !visible(&h, legend_id(&kb, 0)));
        // A capital is typed, shift is spent, the wash and the legend go back.
        assert_eq!(
            tap_key(&mut kb, &mut h, Char('a'), 2 * SECOND),
            Some(KeyOutput::Char('A'))
        );
        h.commit();
        assert_eq!(kb.slots[shift_slot].surface.pair().rest.tint, plain);
        assert!(visible(&h, legend_id(&kb, 0)));
    }

    #[test]
    fn the_action_key_does_not_press_until_the_model_enables_it() {
        let mut h = harness();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let action = centre(&kb.model, KeyKind::Action);
        send(&mut kb, &mut h, TouchPhase::Down, action, 0);
        assert_eq!(send(&mut kb, &mut h, TouchPhase::Up, action, 50 * MS), None);
        let requests = h.commit();
        assert_eq!(
            requests.len(),
            1,
            "nothing was asked of the compositor: {requests:?}"
        );
        let mut sink = SemanticsSink::new();
        kb.semantics(&mut sink);
        assert!(!sink.find("Connect").unwrap().enabled);

        let enabled = KeyboardModel::new("Connect").enabled(true);
        assert_eq!(kb.update(&enabled, &mut h.ui), Damage::Content);
        assert_eq!(kb.update(&enabled, &mut h.ui), Damage::None);
        h.commit();
        let mut sink = SemanticsSink::new();
        kb.semantics(&mut sink);
        assert!(sink.find("Connect").unwrap().enabled);
        send(&mut kb, &mut h, TouchPhase::Down, action, SECOND);
        assert!(animates(&h.commit()) >= 2);
        assert_eq!(
            send(&mut kb, &mut h, TouchPhase::Up, action, SECOND + 50 * MS),
            Some(KeyOutput::Action)
        );
    }

    #[test]
    fn holding_backspace_on_the_keyboard_repeats_without_asking_anything_of_the_compositor() {
        let mut h = harness();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let at = centre(&kb.model, Backspace);
        send(&mut kb, &mut h, TouchPhase::Down, at, 0);
        h.commit();
        assert_eq!(kb.next_repeat_ns(), Some(400 * MS));
        assert_eq!(kb.tick(400 * MS), Some(KeyOutput::Backspace));
        assert_eq!(h.commit().len(), 1, "a repeat is the same pressed key");
        assert_eq!(send(&mut kb, &mut h, TouchPhase::Up, at, 450 * MS), None);
        assert_eq!(kb.next_repeat_ns(), None);
        // A contact taken by something else lets the key go and types nothing.
        send(&mut kb, &mut h, TouchPhase::Down, at, SECOND);
        kb.cancel(&mut h.ui);
        assert!(animates(&h.commit()) >= 2);
        assert_eq!(
            send(&mut kb, &mut h, TouchPhase::Up, at, SECOND + 10 * MS),
            None
        );
    }

    #[test]
    fn the_keyboard_names_the_keys_of_the_page_that_shows_and_no_others() {
        let mut h = harness();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let names = |kb: &Keyboard| {
            let mut sink = SemanticsSink::new();
            kb.semantics(&mut sink);
            sink.into_nodes()
        };
        let letters = names(&kb);
        assert_eq!(letters.len(), 31);
        assert!(letters.iter().any(|n| n.name == "q") && letters.iter().all(|n| n.name != "1"));
        assert!(
            letters
                .iter()
                .all(|n| n.kind == SemanticKind::Key && n.frame.h >= 64.0 && n.frame.w >= 64.0)
        );
        tap_key(&mut kb, &mut h, SHIFT, SECOND);
        let shifted = names(&kb);
        assert!(shifted.iter().any(|n| n.name == "Q") && shifted.iter().all(|n| n.name != "q"));
        assert!(shifted.iter().find(|n| n.name == "Shift").unwrap().selected);
        tap_key(&mut kb, &mut h, To(Page::Symbols), 2 * SECOND);
        let symbols = names(&kb);
        assert!(
            symbols.iter().any(|n| n.name == "1")
                && symbols.iter().all(|n| n.name.to_lowercase() != "q")
        );
    }

    #[test]
    fn a_sheet_too_small_for_64_unit_keys_is_refused() {
        let mut h = harness();
        let narrow = Frame::new(0.0, 0.0, Keyboard::MIN_WIDTH - 1.0, Keyboard::HEIGHT);
        assert!(Keyboard::new(&mut h.ui, narrow, 10, &KeyboardModel::new("Go")).is_err());
        let short = Frame::new(0.0, 0.0, Keyboard::MIN_WIDTH, Keyboard::HEIGHT - 1.0);
        assert!(Keyboard::new(&mut h.ui, short, 10, &KeyboardModel::new("Go")).is_err());
    }

    // ---- legends: pixels, themes, large text ----

    #[test]
    fn legends_are_drawn_in_the_ink_of_the_theme_light_and_dark_and_again_when_it_changes() {
        let mut h = harness();
        h.ui.look = Look::standard(Mode::Dark);
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let dark_ink = h.ui.look.ink();
        let dark_tint = kb.slots[0].surface.pair().rest.tint;
        let pixels = h.pixels(legend_id(&kb, 0));
        assert_eq!(
            brightest(&pixels),
            [dark_ink.0, dark_ink.1, dark_ink.2, dark_ink.3]
        );

        h.ui.look = Look::standard(Mode::Light);
        let light_ink = h.ui.look.ink();
        assert_ne!(light_ink, dark_ink);
        kb.restyle(&mut h.ui);
        h.commit();
        assert_eq!(
            brightest(&h.pixels(legend_id(&kb, 0))),
            [light_ink.0, light_ink.1, light_ink.2, light_ink.3]
        );
        assert_ne!(
            kb.slots[0].surface.pair().rest.tint,
            dark_tint,
            "the glass follows the theme"
        );

        // A keyboard built in the light look from the start is the same.
        let mut light = harness();
        light.ui.look = Look::standard(Mode::Light);
        let fresh = built(&mut light, &KeyboardModel::new("Connect"));
        assert_eq!(
            light.pixels(legend_id(&fresh, 1)),
            h.pixels(legend_id(&kb, 1))
        );
    }

    #[test]
    fn shift_and_backspace_are_icons_drawn_in_their_keys_and_the_space_bar_is_bare() {
        let mut h = harness();
        let kb = built(&mut h, &KeyboardModel::new("Connect"));
        let pixels = h.pixels(legend_id(&kb, 0));
        let width = kb.keys.w.ceil() as usize;
        let inked_in = |f: Frame| {
            let (x0, y0) = ((f.x - kb.keys.x) as usize, (f.y - kb.keys.y) as usize);
            (y0..y0 + f.h as usize)
                .flat_map(|y| (x0..x0 + f.w as usize).map(move |x| (x, y)))
                .filter(|&(x, y)| pixels[(y * width + x) * 4 + 3] > 0)
                .count()
        };
        let model = &kb.model;
        assert!(inked_in(frame_of(model, SHIFT)) > 20);
        assert!(inked_in(frame_of(model, Backspace)) > 20);
        assert_eq!(
            inked_in(frame_of(model, Space)),
            0,
            "the bar's look is its shape"
        );
        assert_eq!(
            inked_in(frame_of(model, KeyKind::Action)),
            0,
            "the action key's content is its own layer"
        );
        assert!(inked_in(frame_of(model, Char('g'))) > 20);
    }

    #[test]
    fn large_text_grows_the_legends_as_far_as_a_key_holds_and_a_long_action_label_is_cut_last() {
        let mut h = harness();
        assert_eq!(legend_type(&h.ui, ty::KEY).px, 28);
        h.ui.look.text_scale = 2.0;
        assert_eq!(
            legend_type(&h.ui, ty::KEY).px,
            40,
            "a 64-high key holds no more"
        );
        assert_eq!(legend_type(&h.ui, ty::LABEL).px, 36);
        let kb = built(&mut h, &KeyboardModel::new("Connect"));
        assert_eq!(kb.action_text(), "Connect", "it fits whole at 200 percent");
        let key_w = frame_of(&kb.model, KeyKind::Action).w;
        let px = action_type(&mut h.ui, "Connect", key_w).px;
        assert!(
            (ty::BUTTON.px..40).contains(&px),
            "it was made smaller to fit, not cut: {px}"
        );

        let mut h = harness();
        h.ui.look.text_scale = 2.0;
        let kb = built(&mut h, &KeyboardModel::new("Connect to the network now"));
        assert!(kb.action_text().ends_with('…'), "{}", kb.action_text());
        let key_w = frame_of(&kb.model, KeyKind::Action).w;
        let drawn_at = action_type(&mut h.ui, "Connect to the network now", key_w);
        assert_eq!(
            drawn_at.px,
            ty::BUTTON.px,
            "a label that cannot fit is as small as it goes before it is cut"
        );
        let drawn = h.ui.fonts.measure(kb.action_text(), drawn_at).width;
        assert!(drawn <= key_w - 2.0 * space::S3, "{drawn} in {key_w}");
        // An empty label is the enter mark.
        let mut h = harness();
        let kb = built(&mut h, &KeyboardModel::new(""));
        assert_eq!(kb.action_text(), "");
        assert!(h.pixels(kb.action.id()).chunks_exact(4).any(|p| p[3] > 0));
        let mut sink = SemanticsSink::new();
        kb.semantics(&mut sink);
        assert!(sink.find("Enter").is_some());
    }

    // ---- echo and warming ----

    #[test]
    fn a_typed_character_appears_in_the_field_in_the_same_commit_and_rasters_no_new_glyph() {
        let mut h = harness();
        Keyboard::warm(&mut h.ui);
        TextField::warm(&mut h.ui);
        let warmed = h.ui.fonts.cached_glyphs();
        let mut kb = built(&mut h, &KeyboardModel::new("Connect"));
        let field_frame = Frame::new(12.0, 84.0, 936.0, 64.0);
        let mut model = TextFieldModel::new("Name");
        model.focused = true;
        let mut field = TextField::new(&mut h.ui, field_frame, 10, &model).unwrap();
        h.commit();

        for (i, c) in "Pa".chars().enumerate() {
            let kinds: &[KeyKind] = if c.is_ascii_uppercase() {
                &[SHIFT, Char(c.to_ascii_lowercase())]
            } else {
                &[Char(c)]
            };
            let mut typed = None;
            for (j, &kind) in kinds.iter().enumerate() {
                typed = tap_key(
                    &mut kb,
                    &mut h,
                    kind,
                    (2 * i as u64 + j as u64 + 1) * SECOND,
                );
            }
            // The screen applies the output to the text and updates the field in the same turn.
            let Some(KeyOutput::Char(ch)) = typed else {
                panic!("{c:?} was not typed")
            };
            model.text.push(ch);
            field.update(&model, &mut h.ui);
            let commit = h.commit();
            assert_eq!(
                damages(&commit),
                1,
                "the value layer was rewritten in the commit that follows the key: {commit:?}"
            );
            let value_pixels = h.pixels(field.value_layer());
            assert!(value_pixels.chunks_exact(4).any(|p| p[3] > 0));
            assert_eq!(field.shown_text(), model.text);
        }
        assert_eq!(model.text, "Pa");
        assert_eq!(
            h.ui.fonts.cached_glyphs(),
            warmed,
            "every glyph was warmed: no raster on the way to the screen"
        );
    }

    // ---- text layers: the legends as pictures as they always were, and as glyph quads ----

    /// The legends of `keys` painted the way they were before they were text layers (`draw_line` and `paint_over` into a picture).
    fn painted_legend(h: &mut Harness, kb: &Keyboard, which: usize) -> Vec<u8> {
        use crate::components::icon::paint_over;
        use crate::components::label::{Align, draw_line, style};
        use crate::text::Canvas;
        let (page, shifted) = LEGENDS[which];
        let (w, hh) = (kb.keys.w.ceil() as u32, kb.keys.h.ceil() as u32);
        let ink = h.ui.look.ink();
        let mut layer = h.ui.scene.image(w, hh, 0, Props::at(0.0, 0.0)).unwrap();
        for key in &kb.model.pages[page.index()] {
            let at = Frame::new(
                key.frame.x - kb.keys.x,
                key.frame.y - kb.keys.y,
                key.frame.w,
                key.frame.h,
            );
            if let Some((text, role)) = legend_text(key.kind, shifted) {
                let ty = legend_type(&h.ui, role);
                draw_line(
                    &mut h.ui.fonts,
                    &mut Canvas {
                        pixels: layer.pixels_mut(),
                        width: w as usize,
                    },
                    at,
                    &text,
                    style(ty, ink, Align::Centre),
                );
            } else if let Some(icon) = legend_icon(key.kind) {
                let size = legend_type(&h.ui, ty::KEY).px;
                let (x, y) = (
                    at.x + (at.w - size as f32) / 2.0,
                    at.y + (at.h - size as f32) / 2.0,
                );
                paint_over(
                    &mut layer,
                    icon,
                    x.round() as i32,
                    y.round() as i32,
                    size,
                    ink,
                );
            }
        }
        layer.damage(None);
        h.commit();
        h.pixels(layer.id())
    }

    /// The action key painted the way it was before it was a text layer.
    fn painted_action(h: &mut Harness, kb: &Keyboard, label: &str, enabled: bool) -> Vec<u8> {
        use crate::components::icon::paint_over;
        use crate::components::label::{Align, Style, draw_line};
        use crate::text::Canvas;
        let ink = if enabled {
            h.ui.look.ink()
        } else {
            h.ui.look.ink2()
        };
        let (w, hh) = kb.action_size;
        let mut layer =
            h.ui.scene
                .image(w as u32, hh as u32, 0, Props::at(0.0, 0.0))
                .unwrap();
        if label.is_empty() {
            let size = legend_type(&h.ui, ty::KEY).px;
            let (x, y) = ((w - size as f32) / 2.0, (hh - size as f32) / 2.0);
            paint_over(
                &mut layer,
                Icon::Enter,
                x.round() as i32,
                y.round() as i32,
                size,
                ink,
            );
        } else {
            let ty = action_type(&mut h.ui, label, w);
            let text = ellipsise(&mut h.ui.fonts, label, ty, w - 2.0 * space::S3);
            draw_line(
                &mut h.ui.fonts,
                &mut Canvas {
                    pixels: layer.pixels_mut(),
                    width: w as usize,
                },
                Frame::new(0.0, 0.0, w, hh),
                &text,
                Style {
                    ty,
                    colour: ink,
                    align: Align::Centre,
                },
            );
        }
        layer.damage(None);
        h.commit();
        h.pixels(layer.id())
    }

    /// A look and a model the keyboard is checked in.
    type Setup = fn(&mut Harness);

    const LOOKS: [(&str, Setup); 3] = [
        ("dark", |h| h.ui.look = Look::standard(Mode::Dark)),
        ("light", |h| h.ui.look = Look::standard(Mode::Light)),
        ("200 percent", |h| h.ui.look.text_scale = 2.0),
    ];

    fn models() -> Vec<KeyboardModel> {
        vec![
            KeyboardModel::new("Connect"),
            KeyboardModel::new("Connect").enabled(true),
            KeyboardModel::new(""),
            KeyboardModel::new("Connect to the network now").enabled(true),
        ]
    }

    /// Answers every atlas upload the way the compositor does, so that what was prepared counts as shown. It reads the recording,
    /// not one commit: a keyboard `built` has already been through its first commit.
    fn land_uploads(h: &mut Harness) {
        for _ in 0..16 {
            for request in h.all() {
                if let Request::AtlasUpload {
                    id,
                    slot,
                    generation,
                    ..
                } = request
                {
                    h.ui.scene
                        .observe(&compositor_protocol::Event::AtlasUploaded {
                            id,
                            slot,
                            generation,
                        });
                }
            }
            h.commit();
        }
    }

    /// A glyph-text UI whose atlas has seen every keyboard of [`models`] and `model` and been told it landed, as one that showed
    /// them before is.
    fn primed(setup: Setup, model: &KeyboardModel) -> Harness {
        let mut h = harness();
        setup(&mut h);
        h.ui.enable_glyph_text().unwrap();
        for seen in models().iter().chain([model]) {
            let first = built(&mut h, seen);
            land_uploads(&mut h);
            drop(first);
        }
        h.commit();
        h
    }

    fn pictures(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Request::Create {
                        content: Content::Image,
                        ..
                    }
                )
            })
            .count()
    }

    fn pixels_written(requests: &[Request]) -> f32 {
        requests
            .iter()
            .map(|r| match r {
                Request::Damage { rect, .. } => rect.w * rect.h,
                _ => 0.0,
            })
            .sum()
    }

    fn swaps(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::SetGlyphs { .. }))
            .count()
    }

    /// The quads a layer was last given.
    fn quads_of(h: &Harness, id: LayerId) -> Vec<compositor_protocol::Glyph> {
        h.all()
            .into_iter()
            .rev()
            .find_map(|r| match r {
                Request::SetGlyphs { id: of, glyphs, .. } if of == id => Some(glyphs),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn as_pictures_the_legends_and_the_action_key_are_the_pixels_the_text_painters_made() {
        for (look, setup) in LOOKS {
            for model in models() {
                let mut h = harness();
                setup(&mut h);
                let kb = built(&mut h, &model);
                for which in 0..4 {
                    let painted = painted_legend(&mut h, &kb, which);
                    assert!(painted.iter().any(|b| *b != 0));
                    assert_eq!(
                        h.pixels(legend_id(&kb, which)),
                        painted,
                        "legend {which} in {look} differs from what draw_line and paint_over made"
                    );
                }
                let painted =
                    painted_action(&mut h, &kb, &model.action_label, model.action_enabled);
                assert_eq!(
                    h.pixels(kb.action.id()),
                    painted,
                    "action {:?} in {look}",
                    model.action_label
                );
            }
        }
    }

    #[test]
    fn on_glyph_text_the_legends_and_the_action_key_show_the_same_pixels_as_the_pictures() {
        for (look, setup) in LOOKS {
            for model in models() {
                let mut pic = harness();
                setup(&mut pic);
                let kp = built(&mut pic, &model);
                let mut run = primed(setup, &model);
                let kr = built(&mut run, &model);
                assert!(kr.action.is_glyph_run(), "{look}");
                for which in 0..4 {
                    assert!(
                        kr.legends[which].as_ref().expect("made").is_glyph_run(),
                        "legend {which} in {look}"
                    );
                    let (a, b) = (
                        run.layer_pixels(legend_id(&kr, which)),
                        pic.layer_pixels(legend_id(&kp, which)),
                    );
                    assert!(a.iter().any(|v| *v != 0));
                    assert_eq!(a, b, "legend {which} in {look}: the run is not the picture");
                }
                assert_eq!(
                    run.layer_pixels(kr.action.id()),
                    pic.layer_pixels(kp.action.id()),
                    "action {:?} in {look}",
                    model.action_label
                );
                assert_eq!(run.ui.glyph_text_fallbacks, 0);
            }
        }
    }

    #[test]
    fn on_glyph_text_building_a_keyboard_makes_no_picture_and_writes_no_pixel_and_every_page_is_one_run_of_one_ink()
     {
        let model = KeyboardModel::new("Connect");
        let mut h = primed(|_| {}, &model);
        let mut kb = Keyboard::new(&mut h.ui, SHEET, 10, &model).unwrap();
        let sent = h.commit();
        assert_eq!(
            pictures(&sent),
            0,
            "no legend and no action label is a picture"
        );
        assert_eq!(pixels_written(&sent), 0.0);
        assert_eq!(
            creates(&sent),
            kb.layer_count(),
            "the layers are as many as ever"
        );
        assert_eq!(creates(&sent), 40 + 5 + 2 + 1 + 1);
        while kb.prepare(&mut h.ui) {}
        let sent = h.commit();
        assert_eq!(pictures(&sent), 0);
        assert_eq!(pixels_written(&sent), 0.0);
        assert_eq!(creates(&sent), 3);
        assert_eq!(kb.layer_count(), 40 + 5 + 2 + 4 + 1);
        assert_eq!(h.ui.glyph_text_fallbacks, 0);
        // A page is about thirty quads, in the one colour of the ink.
        for which in 0..4 {
            assert!(kb.legends[which].as_ref().unwrap().is_glyph_run());
            let quads = quads_of(&h, legend_id(&kb, which));
            assert!(
                (20..=60).contains(&quads.len()),
                "legend {which}: {} quads",
                quads.len()
            );
            assert!(quads.iter().all(|q| q.ink == 0));
        }
    }

    #[test]
    fn on_glyph_text_a_page_switch_or_a_shift_only_shows_another_layer_and_makes_no_picture() {
        let model = KeyboardModel::new("Connect");
        let mut h = primed(|_| {}, &model);
        let mut kb = built(&mut h, &model);
        let ids: Vec<_> = (0..4).map(|i| legend_id(&kb, i)).collect();
        assert!(visible(&h, ids[0]) && !visible(&h, ids[1]));
        let mut at = SECOND;
        for (key, shows) in [
            (SHIFT, 1),
            (To(Page::Symbols), 2),
            (To(Page::More), 3),
            (To(Page::Letters), 0),
        ] {
            tap_key(&mut kb, &mut h, key, at);
            at += SECOND;
            let sent = h.commit();
            assert_eq!(creates(&sent), 0, "{key:?}: {sent:?}");
            assert_eq!(damages(&sent), 0);
            assert_eq!(swaps(&sent), 0, "a prepared page is shown, not swapped in");
            for (i, id) in ids.iter().enumerate() {
                assert_eq!(visible(&h, *id), i == shows, "{key:?}: legend {i}");
            }
        }
        assert_eq!(h.ui.glyph_text_fallbacks, 0);

        // A page shown before its turn is made then, as a run: one layer and one list of quads, no picture and no pixel.
        let mut early = primed(|_| {}, &model);
        let mut kb = Keyboard::new(&mut early.ui, SHEET, 10, &model).unwrap();
        early.commit();
        tap_key(&mut kb, &mut early, To(Page::Symbols), SECOND);
        let sent = early.commit();
        assert_eq!(creates(&sent), 1, "{sent:?}");
        assert_eq!(pictures(&sent), 0);
        assert_eq!(pixels_written(&sent), 0.0);
        assert_eq!(swaps(&sent), 1);
        assert_eq!(early.ui.glyph_text_fallbacks, 0);
    }

    #[test]
    fn on_glyph_text_a_new_label_or_an_enabled_action_key_is_a_swap_that_writes_no_pixel_and_shows_what_the_picture_does()
     {
        let model = KeyboardModel::new("Connect");
        let mut run = primed(|_| {}, &model);
        let mut kr = built(&mut run, &model);
        let mut pic = harness();
        let mut kp = built(&mut pic, &model);
        run.commit();
        pic.commit();

        // Enabling the key changes the ink of its content: one restyle and the same quads, nothing written.
        let enabled = KeyboardModel::new("Connect").enabled(true);
        kr.update(&enabled, &mut run.ui);
        kp.update(&enabled, &mut pic.ui);
        let sent = run.commit();
        pic.commit();
        assert_eq!(creates(&sent), 0, "{sent:?}");
        assert_eq!(pictures(&sent), 0);
        assert_eq!(pixels_written(&sent), 0.0);
        assert!(
            restyles(&sent) >= 1,
            "the ink of an enabled key is not the dimmed one"
        );
        assert_eq!(
            run.layer_pixels(kr.action.id()),
            pic.layer_pixels(kp.action.id())
        );

        // Words made of glyphs the atlas has are one swap, and the ink stays.
        let cone = KeyboardModel::new("Cone").enabled(true);
        kr.update(&cone, &mut run.ui);
        kp.update(&cone, &mut pic.ui);
        let sent = run.commit();
        pic.commit();
        assert_eq!(swaps(&sent), 1, "{sent:?}");
        assert_eq!(restyles(&sent), 0);
        assert_eq!(creates(&sent) + damages(&sent), 0);
        assert_eq!(
            run.layer_pixels(kr.action.id()),
            pic.layer_pixels(kp.action.id())
        );

        // The enter mark and the label are the same layer, and each is what the picture shows.
        for label in ["", "Connect"] {
            let next = KeyboardModel::new(label).enabled(true);
            kr.update(&next, &mut run.ui);
            kp.update(&next, &mut pic.ui);
            let sent = run.commit();
            pic.commit();
            assert_eq!(creates(&sent), 0);
            assert_eq!(pixels_written(&sent), 0.0);
            assert_eq!(
                run.layer_pixels(kr.action.id()),
                pic.layer_pixels(kp.action.id()),
                "{label:?}"
            );
        }

        // A theme change draws everything again, in the new ink, with nothing written.
        run.ui.look = Look::standard(Mode::Dark);
        pic.ui.look = Look::standard(Mode::Dark);
        kr.restyle(&mut run.ui);
        kp.restyle(&mut pic.ui);
        let sent = run.commit();
        pic.commit();
        assert_eq!(pictures(&sent), 0);
        assert_eq!(pixels_written(&sent), 0.0);
        for which in 0..4 {
            if kr.legends[which].is_some() && kp.legends[which].is_some() {
                assert_eq!(
                    run.layer_pixels(legend_id(&kr, which)),
                    pic.layer_pixels(legend_id(&kp, which)),
                    "legend {which} after the theme changed"
                );
            }
        }
        assert_eq!(
            run.layer_pixels(kr.action.id()),
            pic.layer_pixels(kp.action.id())
        );
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
    }
}
