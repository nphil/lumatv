//! A row: one line of a settings page, a list of networks or a notification list. It has a stable [`RowId`], a title and an optional
//! detail, and a typed trailing control (a chevron, a value, a check or a toggle).
//!
//! Two kinds of row look different on purpose (design section 3.4). A **read-only** row has no glass of its own: it inherits the
//! sheet it sits on and only its text is drawn. An **actionable** row is a button, so it carries the shallow raised pair (the
//! `ActionRow` material: 18 radius, edge, rest lip, sheen, raised depth and no shadow) and presses by scaling to .97 about its
//! centre while its prepared pressed glass fades in. A press commits on release inside and is cancelled by a drag out, a lost
//! contact, or by the list around it taking the contact to scroll.
//!
//! Layers: the title, the detail and the trailing value, icon or signal mark are all one text layer ([`TextLayer`]: one run of
//! glyph quads with glyph text, one picture without), with the icon before the words and the hairline under a flat row; a toggle is
//! two glass layers (its recessed track and its raised thumb); only a picture or a badge at the trailing end is a layer of its own.
//! A row is therefore at most six layers, and a read-only row with no trailing control is one. Text is rastered when the model
//! changes and not otherwise.
//!
//! Sizing: a row is at least 64 units high (a touch target). Its title and its detail each take up to two lines at the person's text
//! size, so [`Row::required_height`] grows with large text and long text; only what is still too long after two lines is cut
//! with an ellipsis. A screen sizes its rows with it.
//!
//! A row can be drawn again for another model without making a layer (that is how [`super::list::VirtualList`] recycles them): the
//! layers it was given at the start ([`Needs`]) are what it can show, and giving it a model that needs more (a glass, a trailing
//! picture, a toggle) adds them in `update`, which is never done while a list moves.

use super::group::{Group, Kind, Part, Pose};
use super::icon::Icon;
use super::mark::Mark;
use super::plate::{CLEAR, solid};
use super::press::{PressTracker, Step};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::text::{Run, ellipsise, line_height, wrap, wrap_to};
use super::text_layer::{Item, TextLayer};
use super::{Component, Damage, target_around};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::palette::PaletteRole;
use crate::lucent::tokens::{motion, radius, smart, space, switch, ty};
use crate::text::Canvas;
use compositor_protocol::{Ease, Edges, Props, Rect, Rgba};
use std::io;
use std::sync::Arc;

/// What a leading icon is drawn at, and the room before the words.
const LEAD_ICON: f32 = 28.0;
const LEAD_GAP: f32 = space::S4;
/// The tinted square an overview tile's icon sits on.
const CHIP: f32 = 56.0;
/// The hairline under a flat row, as a part of the ink's opacity.
const RULE_ALPHA: f32 = 0.2;
/// A rail entry is this high (a touch target with room to stand in a list of many).
const RAIL_H: f32 = 60.0;
/// A tile is at least this high.
const TILE_H: f32 = 72.0;

/// Left and right padding of a row's content.
pub const PAD_X: f32 = space::S5;
/// The room a row keeps at its sides: a row of a thing's layer has the mock's 14, the words of a plate of what is the case have none
/// (the plate keeps the room), every other row [`PAD_X`].
pub fn pad_x(look: RowLook) -> f32 {
    match look {
        RowLook::Compact => 14.0,
        RowLook::State => 0.0,
        _ => PAD_X,
    }
}
/// Top and bottom padding of a row's text.
const PAD_Y: f32 = space::S2;
/// Between the text and the trailing control.
const GAP_TRAIL: f32 = space::S3;
/// The side of a chevron or a check.
const ICON: f32 = 24.0;
/// A toggle, the approved mock-ups' `.sw` (`lucent::tokens::switch`, Nitin's decision 36): its track, its knob and the knob's inset from
/// the track's ends and top; the tick on the knob while it is on is a mark of the row's text layer (no layer, no picture: the settings'
/// layer ceiling and its entrance's pictures are pinned).
const TRACK_W: f32 = switch::WIDTH;
const TRACK_H: f32 = switch::HEIGHT;
const THUMB_D: f32 = switch::KNOB;
const THUMB_INSET: f32 = switch::INSET;
const TICK: f32 = switch::TICK;
/// A signal mark: its four bars (5 wide, 3 apart, 8 to 23 high) and, when secured, the lock 8 before them.
const BAR_W: u32 = 5;
const BAR_GAP: u32 = 3;
const BAR_HEIGHTS: [u32; 4] = [8, 13, 18, 23];
const SIGNAL_W: f32 = ICON + 8.0 + (4 * BAR_W + 3 * BAR_GAP) as f32;
/// A trailing value takes at most this part of the row's width; what is longer is cut with an ellipsis.
const VALUE_MAX_FRACTION: f32 = 0.4;
/// Title and detail are each kept to this many lines.
pub const MAX_TITLE_LINES: usize = 2;
/// The lines of a detail a row keeps unless its model asks for more.
pub const MAX_DETAIL_LINES: usize = 2;

/// Which row it is, for as long as it exists. A list finds its rows by this and never by their position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowId(pub u64);

/// A small opaque picture (premultiplied RGBA, `width * height * 4` bytes) that a row shows at its trailing end: a wallpaper. Two
/// thumbnails are the same when they share their bytes.
#[derive(Clone)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl PartialEq for Thumbnail {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && Arc::ptr_eq(&self.rgba, &other.rgba)
    }
}

impl std::fmt::Debug for Thumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Thumbnail({}x{})", self.width, self.height)
    }
}

/// The corner of a picture at a row's end.
const PICTURE_RADIUS: f32 = 10.0;

/// What a row shows at its trailing end.
#[derive(Clone, Debug, PartialEq)]
pub enum Trailing {
    None,
    /// A chevron: the row opens something.
    Chevron,
    /// The current value, in secondary ink.
    Value(String),
    /// A value with a colour of its own and a small mark before it: `Sending` with a tick, `Checking` with a ring. The words say all
    /// the same; the colour and the mark only add to them.
    Reading {
        text: String,
        tone: Option<ValueTone>,
        mark: Option<ValueMark>,
    },
    /// A value and a chevron after it: the row opens a list that has this many things in it (a count, or a few words).
    Count(String),
    /// A value, then room for a round button the page puts there (a play button: the mock's `.ib` is 56 across with 16 either side of it),
    /// then a chevron: the row opens a list of what it holds and the button plays what it holds now. The row draws the value and the
    /// chevron and leaves the room; the button is the page's own control, not a part of the row.
    Play(String),
    /// A reading against its limit (`0.1 %` and `of 1 %`): the words of one value, the limit in a dimmer ink, the mock's `.mv`.
    Limit {
        value: String,
        of: String,
    },
    /// A pill that says one thing about what the row is about (`On the NAS`): the words in the tone's ink on a faint tint of it, with a
    /// tick before them when the thing holds. Not a control: the words say all the same.
    Chip {
        text: String,
        tone: Option<ValueTone>,
        check: bool,
    },
    /// A check, shown when selected: no meaning is the colour alone.
    Check(bool),
    /// A toggle showing its confirmed value; a tap proposes the opposite.
    Toggle(bool),
    /// A network's signal: four bars, `bars` of them lit, behind a lock when it is secured. Means the same in words (the row is
    /// described as "signal 3 of 4"), so the bars are never the only sign.
    Signal {
        bars: u8,
        locked: bool,
    },
    /// A plain icon of its own (a lock): no action, no state.
    Icon(Icon),
    /// A small disc on the accent with a count in it (one update is ready): a mark of something to attend to.
    Badge(u8),
    /// A small picture of what the row is about (the wallpaper in force). Decoration: the row's words say the same.
    Picture(Thumbnail),
}

/// How a trailing reading is inked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueTone {
    /// The positive colour.
    Good,
    /// The warning colour.
    Warn,
    /// The danger colour.
    Bad,
    /// The tertiary ink: quieter than the usual secondary one.
    Muted,
}

/// A small mark before a trailing reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueMark {
    Check,
    Spinner,
}

impl ValueMark {
    fn icon(self) -> Icon {
        match self {
            Self::Check => Icon::Check,
            Self::Spinner => Icon::Spinner,
        }
    }
}

/// The side of the mark before a trailing reading, and the room after it.
const MARK: f32 = 20.0;
const MARK_GAP: f32 = space::S2;

/// The pill of a chip: its height, the room round its words, its tick and the room after the tick.
const CHIP_PILL_H: f32 = 32.0;
const CHIP_PAD: f32 = 12.0;
const CHIP_ICON: f32 = 18.0;
const CHIP_GAP: f32 = 6.0;
/// The most of a row a chip takes.
const CHIP_MAX_FRACTION: f32 = 0.5;
/// The tint of a chip's pill, as an alpha of its ink: a toned one (12 %), a quiet one (the secondary ink, 16 %).
const CHIP_TINT: u8 = 31;
const CHIP_QUIET_TINT: u8 = 41;
/// The room between a count and its chevron.
const COUNT_GAP: f32 = space::S2;
/// On the row of a settings page's field that has a tile (a [`nav_row`]: `Flat` and `Plain`, the Presence page's) a row that opens
/// something ends as the mock's `nav` row does: its chevron is the mock's `.chev`, a glyph of 28 whose box reaches 4 beyond the row's
/// content (`margin-right: -4px`), and the value before it ends 16 from that box (the row's flex gap). Every other row keeps the 24
/// glyph and the 8.
const NAV_ICON: f32 = 28.0;
const NAV_GAP: f32 = 16.0;
/// A value at the end of a [`nav_row`] stands where the mock's `.val` does. Lucent's run sets its glyphs about 2 px lower in its 1.4
/// line than a browser sets them in a `normal` one (the page's own lines are placed to make up for it), so the value is raised by that
/// much.
const VALUE_RAISE: f32 = 2.0;

/// Whether a row is the row of a settings page's field that has a tile (the approved Presence page's `.row` with its `.ich`): only
/// these have the look the mock gives them below. Every other row (a plugin's, the entry of a thing's layer, a page's row without a
/// tile) keeps the toolkit's own: their pictures are what they were.
fn nav_row(look: RowLook, field: bool) -> bool {
    field && matches!(look, RowLook::Flat | RowLook::Plain)
}

/// The room between a count and its chevron.
fn count_gap(nav: bool) -> f32 {
    if nav { NAV_GAP } else { COUNT_GAP }
}

/// Where the run of a value at a row's end starts from the layer's top: centred on the row, and on a [`nav_row`] raised
/// [`VALUE_RAISE`].
fn value_top(nav: bool, h: f32, ty: crate::text::Type) -> f32 {
    let top = ((h - line_height(ty)) / 2.0).round().max(0.0);
    if nav {
        (top - VALUE_RAISE).max(0.0)
    } else {
        top
    }
}
/// What a [`Trailing::Play`] leaves between its value and its chevron: 16 before the button, the button (56), and 16 after it.
pub const PLAY_ROOM: f32 = 16.0 + 56.0 + 16.0;

/// The type of a pill in a row: the mock's `.chipw`, 18 px in the heavier weight, whose 32 px height, 12 at the sides and 18 px tick
/// (above) are made for it. Pills are the settings' own (`On the NAS`), made at their text size.
pub(crate) fn chip_type(ui: &Ui) -> crate::text::Type {
    ui.look.page_ty(18, crate::text::Weight::SemiBold)
}

/// How a row sets its words: the types of the title and of the detail, the height of a line of each, and the room between the two.
///
/// The rows of a page (`Flat` and `Plain`) follow the approved Settings mock-ups: the label in the heavier weight (the mock's 550; the
/// Show has Regular, Medium and SemiBold, and SemiBold is the one its rasteriser makes as dark as 550), on a 1.2 line, and the detail
/// under it on a 1.25 line with 2 below the label. That is a row of 72 with a detail (16 of padding, 29, 2 and 25), as the mock's.
/// Every other look keeps the toolkit's running text: Regular, Lucent's 1.4 line, nothing between.
#[derive(Clone, Copy, Debug)]
struct Set {
    title: crate::text::Type,
    detail: crate::text::Type,
    /// The type of a value at the row's end.
    value: crate::text::Type,
    title_line: f32,
    detail_line: f32,
    gap: f32,
}

impl Set {
    /// The height of `title` lines of the title and `detail` lines of the detail.
    fn height(&self, title: usize, detail: usize) -> f32 {
        let gap = if detail > 0 { self.gap } else { 0.0 };
        title as f32 * self.title_line + gap + detail as f32 * self.detail_line
    }

    /// Where a run of `ty` starts so that its glyphs stand in the middle of a line `line_h` high that begins at `top`: a run's own
    /// box is Lucent's 1.4 line.
    fn run_y(top: f32, line_h: f32, ty: crate::text::Type) -> f32 {
        top + ((line_h - line_height(ty)) / 2.0).round()
    }
}

fn set_of(ui: &Ui, look: RowLook) -> Set {
    let (title, detail, value) = (
        ui.look.ty(ty::BODY),
        ui.look.ty(ty::CAPTION),
        ui.look.ty(ty::LABEL),
    );
    match look {
        RowLook::Flat | RowLook::Plain => Set {
            title: crate::text::Type {
                weight: crate::text::Weight::SemiBold,
                ..title
            },
            detail,
            value,
            title_line: (title.px as f32 * 1.2).ceil(),
            detail_line: (detail.px as f32 * 1.25).ceil(),
            gap: 2.0,
        },
        RowLook::Compact => {
            let (title, detail) = (
                ui.look.page_ty(21, crate::text::Weight::SemiBold),
                ui.look.page_ty(18, crate::text::Weight::Regular),
            );
            Set {
                title,
                detail,
                value: ui.look.page_ty(20, crate::text::Weight::Medium),
                title_line: title.px as f32 * 1.2,
                detail_line: detail.px as f32 * 1.2,
                gap: 0.0,
            }
        }
        RowLook::State => {
            let (title, detail) = (
                ui.look.page_ty(23, crate::text::Weight::SemiBold),
                ui.look.page_ty(20, crate::text::Weight::Regular),
            );
            Set {
                title,
                detail,
                value: ui.look.page_ty(20, crate::text::Weight::Medium),
                title_line: title.px as f32 * 1.2,
                detail_line: detail.px as f32 * 1.25,
                gap: 2.0,
            }
        }
        RowLook::Card | RowLook::Tile | RowLook::Rail => Set {
            title,
            detail,
            value,
            title_line: line_height(title),
            detail_line: line_height(detail),
            gap: 0.0,
        },
    }
}

/// The title of a page's row that has a pill at its end (`Trailing::Chip`, the saved bugs of Debug) is the mock's `.bug .tx .lb`: 23
/// px, a pixel under the other rows', so that the long ones keep their line beside the pill. Every other row, look or trailing, is
/// [`set_of`] as it stands.
fn set_for(ui: &Ui, look: RowLook, pill: bool) -> Set {
    let mut set = set_of(ui, look);
    if pill && matches!(look, RowLook::Flat | RowLook::Plain) {
        let title = crate::text::Type {
            px: ui.look.page_ty(23, crate::text::Weight::SemiBold).px,
            ..set.title
        };
        set.title = title;
        set.title_line = (title.px as f32 * 1.2).ceil();
    }
    set
}

/// The tinted square of a dressed row: its side, its glyph and the room after it. A list of things (a plugin) has the large one, a
/// thing's layer (what it may do) the mock's `.perm` one.
struct Tile {
    side: f32,
    glyph: f32,
    gap: f32,
}

fn tile_of(look: RowLook) -> Tile {
    match look {
        RowLook::Compact => Tile {
            side: 40.0,
            glyph: 24.0,
            gap: 14.0,
        },
        _ => Tile {
            side: DRESS_TILE,
            glyph: DRESS_ICON,
            gap: LEAD_GAP,
        },
    }
}

/// The room a row keeps above and below its words.
fn pad_y(look: RowLook) -> f32 {
    match look {
        RowLook::Compact => 6.0,
        RowLook::State => 0.0,
        _ => PAD_Y,
    }
}

/// How wide a chip is: its words, the room round them and the tick before them when it has one.
fn chip_width(ui: &mut Ui, text: &str, check: bool) -> f32 {
    let words = ui.fonts.measure(text, chip_type(ui)).width.ceil();
    2.0 * CHIP_PAD + words + if check { CHIP_ICON + CHIP_GAP } else { 0.0 }
}

/// How a row stands on what it sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowLook {
    /// An actionable row is a raised glass button; a row that is not has only its words (the toolkit's default).
    #[default]
    Card,
    /// Words on a sheet with a hairline under them: a row of a settings page. A touch washes it; there is no glass.
    Flat,
    /// A flat row without the hairline, its leading icon on a tinted square: a tile of an overview.
    Tile,
    /// Words (and an icon before them) with nothing else: the text of a strip or a banner.
    Plain,
    /// An entry of a list that is always there: icon and words, and the selected one stands on glass with a bar at its edge.
    Rail,
    /// A row of a thing's layer (what a plugin may do, what it uses): the approved mock's `.perm` and `.meter`. Its words are 21 px
    /// over 18 px on 1.2 lines with nothing between, a reading's value 20 px, 6 above and below, and the tinted square of a dressed
    /// row is 40 with a 24 glyph and 14 after it. Its sides are the column's.
    Compact,
    /// The words of the plate that says how a thing's layer is (the approved mock's `.statebox`): the headline 23 px and the line
    /// under it 20 px, on lines of 1.2 and 1.25 with 2 between, and nothing round them: the plate keeps its own room.
    State,
}

/// What a row shows and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct RowModel {
    pub id: RowId,
    pub title: String,
    pub detail: Option<String>,
    pub trailing: Trailing,
    /// Whether a touch on the row does something. A card that is not has no glass of its own.
    pub actionable: bool,
    /// Selected, enabled, pending and focused.
    pub state: ControlState,
    pub look: RowLook,
    /// An icon before the words.
    pub leading: Option<Icon>,
    /// The most lines the detail takes before it is cut with an ellipsis ([`MAX_DETAIL_LINES`] unless a screen that must never cut
    /// its descriptions asks for more); the row grows with every line.
    pub max_detail_lines: usize,
    /// What dresses a row of a list of things that have a state (a plugin): see [`Dress`].
    pub dress: Option<Dress>,
    /// The row is at least this high instead of what its look needs (a reading's line and its bar are not a touch target).
    pub min_height: Option<f32>,
}

impl RowModel {
    /// A read-only row with a title.
    pub fn new(id: RowId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            detail: None,
            trailing: Trailing::None,
            actionable: false,
            state: ControlState::default(),
            look: RowLook::Card,
            leading: None,
            max_detail_lines: MAX_DETAIL_LINES,
            dress: None,
            min_height: None,
        }
    }

    pub fn look(mut self, look: RowLook) -> Self {
        self.look = look;
        self
    }

    /// The row is at least `height` high, whatever its look needs.
    pub fn min_height(mut self, height: f32) -> Self {
        self.min_height = Some(height);
        self
    }

    /// The row is one of a list of things with a state: its glyph on a tinted square, a tag after the title, a status line.
    pub fn dress(mut self, dress: Dress) -> Self {
        self.dress = Some(dress);
        self
    }
    pub fn leading(mut self, icon: Icon) -> Self {
        self.leading = Some(icon);
        self
    }

    /// The detail may take up to `lines` lines (at least one) before it is cut.
    pub fn detail_lines(mut self, lines: usize) -> Self {
        self.max_detail_lines = lines.max(1);
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn trailing(mut self, trailing: Trailing) -> Self {
        self.trailing = trailing;
        self
    }

    /// The row is a button: it has its own glass and a touch on it does something.
    pub fn actionable(mut self) -> Self {
        self.actionable = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.state.enabled = enabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.state.selected = selected;
        self
    }

    /// A row of no content, for a tile that is waiting for a row to show.
    pub(crate) fn blank() -> Self {
        Self::new(RowId(u64::MAX), "")
    }
}

/// The side of the tinted square a dressed row's glyph sits on, the glyph, and the dot before a status line.
const DRESS_TILE: f32 = 48.0;
const DRESS_ICON: f32 = 28.0;
const DOT: f32 = 10.0;
/// The room a status line gives its dot, and the room between a title and its tag.
const DOT_ROOM: f32 = DOT + space::S2;
const TAG_GAP: f32 = space::S2;

/// How a row of a list of things that have a state is dressed. Everything is in the row's one text layer: the square and its glyph
/// before the words, the tag (a version) after the title on the same line, and the status line under the title with a dot before
/// it. The words always say the state; the colour of the dot and of the words only add to them.
#[derive(Clone, Debug, PartialEq)]
pub struct Dress {
    pub tile: Icon,
    /// What tints the square and its glyph: the state the status line reports (none: the quiet ink).
    pub tint: Option<ValueTone>,
    pub tag: Option<String>,
    pub status: Option<DressStatus>,
    /// The row is at least this high (a list of things is taller than a list of settings).
    pub min_height: f32,
    /// The tile is that of a settings page's field (the approved Presence page): the square is the mock's `.ich` (14 round, 16 % of the
    /// accent) and the row ends as the mock's `nav` row does. A plugin's tile and a layer's entry keep the toolkit's.
    pub field: bool,
}

/// The line under a dressed row's title.
#[derive(Clone, Debug, PartialEq)]
pub struct DressStatus {
    pub text: String,
    pub tone: ValueTone,
    /// The dot is a ring, not a disc: nothing is going on.
    pub ring: bool,
}

/// What a touch on a row asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    /// Released inside: open it, choose it, do it.
    Activated,
    /// Released inside a row with a toggle: the value the person asks for. The row keeps showing the confirmed value until its
    /// model says otherwise.
    Proposed(bool),
}

/// The layers a row has been given: what it can show without making another. A list takes the largest need of all its rows, so
/// that a tile can show any of them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Needs {
    /// Glass of its own (an actionable card, the open entry of a rail).
    pub glass: bool,
    /// A toggle's track and thumb.
    pub toggle: bool,
    /// The width of the layer at the trailing end, which only a picture or a badge needs (a value, an icon or a signal at the end
    /// of a row is in its text layer); none when 0.
    pub trail_w: f32,
    /// How tall that layer must be when a picture goes in it; 0 for the height of a line of text.
    pub trail_h: f32,
    /// The wash a touch leaves on a flat row.
    pub wash: bool,
    /// The bar at the edge of the selected rail entry.
    pub bar: bool,
}

impl Needs {
    /// What serves both `self` and `other`.
    pub fn merge(self, other: Needs) -> Needs {
        Needs {
            glass: self.glass || other.glass,
            toggle: self.toggle || other.toggle,
            trail_w: self.trail_w.max(other.trail_w),
            trail_h: self.trail_h.max(other.trail_h),
            wash: self.wash || other.wash,
            bar: self.bar || other.bar,
        }
    }

    /// Whether a row given `self` can show what `other` needs.
    pub fn covers(&self, other: &Needs) -> bool {
        (self.glass || !other.glass)
            && (self.toggle || !other.toggle)
            && (self.wash || !other.wash)
            && (self.bar || !other.bar)
            && self.trail_w >= other.trail_w
            && self.trail_h >= other.trail_h
    }
}

/// Where a row's words go and how tall that makes the row.
struct Metrics {
    text_w: f32,
    title: Vec<String>,
    detail: Vec<String>,
    /// A trailing value, cut to fit.
    value: Option<String>,
    height: f32,
}

/// How much of a row its trailing words may take: [`VALUE_MAX_FRACTION`], and on a row with a tile (the mock's `.row` with its `.ich`)
/// whatever its own words leave, so that a value is never cut while its label and the line under it have all the room they need.
fn value_share(ui: &mut Ui, model: &RowModel, width: f32) -> f32 {
    let wide = matches!(
        model.trailing,
        Trailing::Value(_) | Trailing::Reading { .. } | Trailing::Count(_) | Trailing::Limit { .. }
    );
    if !model.dress.as_ref().is_some_and(|d| d.field) || !wide {
        return VALUE_MAX_FRACTION;
    }
    let set = set_for(ui, model.look, false);
    let mut words = ui.fonts.measure(&model.title, set.title).width.ceil();
    for line in model.detail.as_deref().unwrap_or_default().lines() {
        words = words.max(ui.fonts.measure(line, set.detail).width.ceil());
    }
    let free = width - 2.0 * pad_x(model.look) - lead_width(model) - words - 2.0 * GAP_TRAIL;
    (free / width).max(VALUE_MAX_FRACTION)
}

fn trailing_extent(
    ui: &mut Ui,
    trailing: &Trailing,
    look: RowLook,
    width: f32,
    share: f32,
    nav: bool,
) -> f32 {
    let value_ty = set_of(ui, look).value;
    match trailing {
        Trailing::None => 0.0,
        Trailing::Chevron | Trailing::Check(_) | Trailing::Icon(_) | Trailing::Badge(_) => ICON,
        Trailing::Toggle(_) => TRACK_W,
        Trailing::Signal { .. } => SIGNAL_W,
        Trailing::Picture(picture) => picture.width as f32,
        Trailing::Value(text) => {
            let wanted = ui.fonts.measure(text, value_ty).width.ceil();
            wanted.clamp(ICON, (width * share).max(ICON))
        }
        Trailing::Reading { text, mark, .. } => {
            let lead = mark.map_or(0.0, |_| MARK + MARK_GAP);
            let wanted = ui.fonts.measure(text, value_ty).width.ceil() + lead;
            wanted.clamp(ICON, (width * share).max(ICON))
        }
        Trailing::Count(text) => {
            let wanted = ui.fonts.measure(text, value_ty).width.ceil() + count_gap(nav) + ICON;
            wanted.clamp(2.0 * ICON, (width * share).max(2.0 * ICON))
        }
        Trailing::Play(text) => {
            let wanted = ui.fonts.measure(text, value_ty).width.ceil() + PLAY_ROOM + ICON;
            wanted.clamp(
                PLAY_ROOM + 2.0 * ICON,
                (width * VALUE_MAX_FRACTION).max(PLAY_ROOM + 2.0 * ICON),
            )
        }
        Trailing::Limit { value, of } => {
            let wanted = ui
                .fonts
                .measure(&format!("{value} {of}"), value_ty)
                .width
                .ceil();
            wanted.clamp(ICON, (width * share).max(ICON))
        }
        Trailing::Chip { text, check, .. } => {
            chip_width(ui, text, *check).min(width * CHIP_MAX_FRACTION)
        }
    }
}

/// `text` in at most `max_lines` lines of `max_w` at `ty`; a word wider than the line is cut as well, so nothing runs into the
/// trailing control.
fn fit_lines(
    ui: &mut Ui,
    text: &str,
    ty: crate::text::Type,
    max_w: f32,
    max_lines: usize,
) -> Vec<String> {
    wrap_to(&mut ui.fonts, text, ty, max_w, max_lines)
        .into_iter()
        .map(|line| ellipsise(&mut ui.fonts, &line, ty, max_w))
        .collect()
}

/// The room an icon before the words takes, with the gap after it.
fn lead_width(model: &RowModel) -> f32 {
    if model.dress.is_some() {
        let tile = tile_of(model.look);
        return tile.side + tile.gap;
    }
    match (model.leading, model.look) {
        (None, _) => 0.0,
        (Some(_), RowLook::Tile) => CHIP + LEAD_GAP,
        (Some(_), _) => LEAD_ICON + LEAD_GAP,
    }
}

/// The least a row of this look is high.
fn least_height(look: RowLook) -> f32 {
    match look {
        RowLook::Rail => RAIL_H,
        RowLook::Tile => TILE_H,
        RowLook::Card | RowLook::Flat | RowLook::Plain => smart::TARGET,
        RowLook::Compact | RowLook::State => 0.0,
    }
}

fn metrics(ui: &mut Ui, model: &RowModel, width: f32) -> Metrics {
    let set = set_for(
        ui,
        model.look,
        matches!(model.trailing, Trailing::Chip { .. }),
    );
    let (title_ty, detail_ty) = (set.title, set.detail);
    let label_ty = set.value;
    let share = value_share(ui, model, width);
    let nav = nav_row(model.look, model.dress.as_ref().is_some_and(|d| d.field));
    let trailing = trailing_extent(ui, &model.trailing, model.look, width, share, nav);
    let gap = if trailing > 0.0 { GAP_TRAIL } else { 0.0 };
    let text_w = (width - 2.0 * pad_x(model.look) - lead_width(model) - trailing - gap).max(0.0);
    // A dressed row's tag stands after its title on the same line, and its status line starts after the dot: both are one line.
    let tag_w = model
        .dress
        .as_ref()
        .and_then(|d| d.tag.as_deref())
        .map_or(0.0, |tag| {
            ui.fonts.measure(tag, detail_ty).width.ceil() + TAG_GAP
        });
    let title_lines = if model.dress.as_ref().is_some_and(|d| d.status.is_some()) {
        1
    } else {
        MAX_TITLE_LINES
    };
    let title = fit_lines(
        ui,
        &model.title,
        title_ty,
        (text_w - tag_w).max(0.0),
        title_lines,
    );
    // A tile of an overview is a glance: its line is one line, cut with an ellipsis, so that the tiles of a row are alike.
    let detail_lines = if model.look == RowLook::Tile {
        1
    } else {
        model.max_detail_lines
    };
    let status = model.dress.as_ref().and_then(|d| d.status.as_ref());
    let detail = match (status, model.detail.as_deref()) {
        (Some(status), _) => {
            fit_lines(ui, &status.text, detail_ty, (text_w - DOT_ROOM).max(0.0), 1)
        }
        (None, Some(d)) if !d.is_empty() => fit_lines(ui, d, detail_ty, text_w, detail_lines),
        _ => Vec::new(),
    };
    let value = match &model.trailing {
        Trailing::Value(text) => Some(ellipsise(&mut ui.fonts, text, label_ty, trailing)),
        Trailing::Reading { text, mark, .. } => {
            let room = trailing - mark.map_or(0.0, |_| MARK + MARK_GAP);
            Some(ellipsise(&mut ui.fonts, text, label_ty, room.max(0.0)))
        }
        Trailing::Count(text) => {
            let room = trailing - ICON - count_gap(nav);
            Some(ellipsise(&mut ui.fonts, text, label_ty, room.max(0.0)))
        }
        Trailing::Play(text) => {
            let room = trailing - ICON - PLAY_ROOM;
            Some(ellipsise(&mut ui.fonts, text, label_ty, room.max(0.0)))
        }
        Trailing::Chip { text, check, .. } => {
            let room = trailing - 2.0 * CHIP_PAD - if *check { CHIP_ICON + CHIP_GAP } else { 0.0 };
            let chip_ty = chip_type(ui);
            Some(ellipsise(&mut ui.fonts, text, chip_ty, room.max(0.0)))
        }
        Trailing::Limit { value, of } => {
            let whole = format!("{value} {of}");
            Some(ellipsise(&mut ui.fonts, &whole, label_ty, trailing))
        }
        _ => None,
    };
    let text_h = set.height(title.len(), detail.len());
    Metrics {
        text_w,
        title,
        detail,
        value,
        height: (2.0 * pad_y(model.look) + text_h)
            .max(model.min_height.unwrap_or_else(|| least_height(model.look)))
            .max(model.dress.as_ref().map_or(0.0, |d| d.min_height))
            .ceil(),
    }
}

/// The height of the trailing text or icon layer at the person's text size.
fn trail_h(ui: &Ui) -> f32 {
    line_height(ui.look.ty(ty::LABEL)).max(ICON)
}

/// The height a trailing layer made for `needs` has.
fn trail_height(ui: &Ui, needs: &Needs) -> f32 {
    trail_h(ui).max(needs.trail_h)
}

/// What a row's text layer shows: the words, and the marks around them. A change in any of it draws the layer again.
#[derive(Clone, Debug, PartialEq)]
struct TextKey {
    title: Vec<String>,
    detail: Vec<String>,
    ink: Rgba,
    ink2: Rgba,
    text_w: f32,
    look: RowLook,
    leading: Option<Icon>,
    /// The accent that tints a leading icon (selected, or on a tile's square).
    accent: Rgba,
    selected: bool,
    trail: TrailMark,
    dress: Option<DressKey>,
}

/// What a dressed row's layer shows besides its words.
#[derive(Clone, Debug, PartialEq)]
struct DressKey {
    tile: Icon,
    tint: Rgba,
    /// How much of the tint fills the square, as an alpha: the mock's `.ich` is 16 % of the accent (and of the danger colour), 20 % of
    /// the warning colour; the quiet square (a thing that is off) keeps the toolkit's own.
    wash: u8,
    /// The tile is a settings page's field's (see [`Dress::field`]).
    field: bool,
    tag: Option<String>,
    /// The ink of the status line and its dot, and whether the dot is a ring; none: the row has no status line.
    status: Option<(Rgba, bool)>,
}

/// What the text layer shows at a row's trailing end.
#[derive(Clone, Debug, PartialEq)]
enum TrailMark {
    None,
    Icon(Icon, Rgba),
    Value(String, Rgba),
    /// A value of its own ink, and the mark before it (the icon, and the ink it is drawn in).
    Reading(String, Rgba, Option<(Icon, Rgba)>),
    /// A value and a chevron after it: the ink of both.
    /// A value and a chevron: the room between them.
    Count(String, Rgba, f32),
    /// A reading and the limit after it: the ink of the reading and the dimmer one of the limit.
    Limit(String, String, Rgba, Rgba),
    /// A pill: the words, their ink, the pill's fill (premultiplied) and whether a tick comes before the words.
    Chip(String, Rgba, Rgba, bool),
    /// Bars lit, locked, the ink of the lit bars and the lock, and the ink of the dim ones.
    Signal(u8, bool, Rgba, Rgba),
    /// The tick of an on toggle, in its ink, over the knob at the track's far end.
    Tick(Rgba),
}

/// What the layer at the trailing end shows, when a row has one.
#[derive(Clone, Debug, PartialEq)]
enum TrailKey {
    Empty,
    Picture(Thumbnail),
    /// A count on a disc: the disc's colour and the count's.
    Badge(u8, Rgba, Rgba),
}

struct Toggle {
    track: usize,
    thumb: usize,
}

pub struct Row {
    frame: Frame,
    z: i32,
    model: RowModel,
    needs: Needs,
    clip: Option<Rect>,
    feather: Edges,
    /// How a page holding the row poses its layers; at rest unless a page says otherwise.
    pose: Pose,
    group: Group,
    surface: Option<Surface>,
    text: usize,
    trail: Option<usize>,
    toggle: Option<Toggle>,
    /// The wash a touch leaves on a flat row, and the selected rail entry's bar.
    wash: Option<usize>,
    bar: Option<usize>,
    /// Whether the row is drawn at all (a tile that waits for a row is hidden).
    shown: bool,
    /// Whether each part of the group was last told to show (so that a part is hidden once, not on every move).
    parts_shown: Vec<bool>,
    /// The parts that show for the row's model: what a move or a glide sets.
    live: Vec<usize>,
    text_drawn: Option<TextKey>,
    trail_drawn: Option<TrailKey>,
    toggle_drawn: Option<(bool, bool)>,
    title_lines: Vec<String>,
    detail_lines: Vec<String>,
    press: PressTracker,
}

impl Row {
    /// A row at `frame` (its height from [`Row::required_height`]) showing `model`, with the layers `model` needs.
    pub fn new(ui: &mut Ui, frame: Frame, z: i32, model: &RowModel) -> io::Result<Self> {
        let needs = Self::needs_of(model);
        Self::build(ui, frame, z, needs, model, true)
    }

    /// A row that is not drawn yet, with the layers `needs` says: a tile waiting for a row. Show a row on it with `update` and
    /// `place`.
    pub fn spare(ui: &mut Ui, frame: Frame, z: i32, needs: Needs) -> io::Result<Self> {
        Self::build(ui, frame, z, needs, &RowModel::blank(), false)
    }

    fn build(
        ui: &mut Ui,
        frame: Frame,
        z: i32,
        needs: Needs,
        model: &RowModel,
        shown: bool,
    ) -> io::Result<Self> {
        let mut group = Group::new(frame.centre());
        let surface = needs.glass.then(|| {
            Surface::add(
                ui,
                &mut group,
                frame,
                z,
                SmartMaterial::ActionRow,
                MaterialContext::Nested,
                model.state,
            )
        });
        let text_frame = Frame::new(
            frame.x + pad_x(model.look),
            frame.y,
            (frame.w - 2.0 * pad_x(model.look)).max(1.0),
            frame.h,
        );
        // The words are made with the layer, so that a layer whose glyphs are not at the compositor yet is a picture at once and
        // never shows them late.
        let first = shown.then(|| text_key(ui, model, frame.w));
        let items = first
            .as_ref()
            .map_or_else(Vec::new, |key| text_items(ui, key, text_frame.w, frame.h));
        let text = group.push(Part::new(
            TextLayer::new(ui, text_frame, z + 1, &items)?,
            text_frame,
        ));
        let trail = if needs.trail_w > 0.0 {
            Some(group.push(trail_part(
                ui,
                frame,
                z,
                needs.trail_w,
                trail_height(ui, &needs),
            )?))
        } else {
            None
        };
        let toggle = needs.toggle.then(|| toggle_parts(ui, &mut group, frame, z));
        // The wash a touch leaves is made by the first touch (`input`), not here: a page of rows is then that many layers lighter
        // in the frame that makes it, and a wash is one small layer at a moment when nothing else is being made.
        let wash = None;
        let bar = needs.bar.then(|| group.push(bar_part(ui, frame, z)));
        let parts = group.parts.len();
        let mut row = Self {
            frame,
            z,
            model: model.clone(),
            needs,
            clip: None,
            feather: Edges::NONE,
            pose: Pose::REST,
            group,
            surface,
            text,
            trail,
            toggle,
            wash,
            bar,
            shown,
            parts_shown: vec![true; parts],
            live: Vec::new(),
            title_lines: first.as_ref().map_or_else(Vec::new, |k| k.title.clone()),
            detail_lines: first.as_ref().map_or_else(Vec::new, |k| k.detail.clone()),
            text_drawn: first,
            trail_drawn: None,
            toggle_drawn: None,
            press: PressTracker::new(),
        };
        if shown {
            row.draw(ui, false);
        }
        row.refresh_live();
        Ok(row)
    }

    /// The layers a row for `model` needs.
    pub fn needs_of(model: &RowModel) -> Needs {
        let rail = model.look == RowLook::Rail;
        Needs {
            // A rail entry stands on glass, with a bar at its edge, only while it is the open one: the others are words and an icon
            // (and the wash a touch leaves), so that a rail of eleven does not hold five layers for each that nothing shows.
            glass: (rail && model.state.selected)
                || (model.look == RowLook::Card && model.actionable),
            toggle: matches!(model.trailing, Trailing::Toggle(_)),
            trail_w: match &model.trailing {
                Trailing::Picture(picture) => picture.width as f32,
                Trailing::Badge(_) => ICON,
                _ => 0.0,
            },
            trail_h: match &model.trailing {
                Trailing::Picture(picture) => picture.height as f32,
                _ => 0.0,
            },
            wash: model.actionable && model.look != RowLook::Card,
            bar: rail && model.state.selected,
        }
    }

    /// The height a row for `model` needs at `width` and the person's text size: at least 64, and tall enough for its title and its
    /// detail, each in up to two lines. Large text makes a row taller and never its text smaller.
    pub fn required_height(ui: &mut Ui, model: &RowModel, width: f32) -> f32 {
        metrics(ui, model, width).height
    }

    /// How many lines the title and the detail of `model` would take at `width` if nothing were cut: what a screen asks to choose a
    /// layout in which the words fit (a control beside them, or under them).
    pub fn lines_needed(ui: &mut Ui, model: &RowModel, width: f32) -> (usize, usize) {
        let share = value_share(ui, model, width);
        let nav = nav_row(model.look, model.dress.as_ref().is_some_and(|d| d.field));
        let trailing = trailing_extent(ui, &model.trailing, model.look, width, share, nav);
        let gap = if trailing > 0.0 { GAP_TRAIL } else { 0.0 };
        let text_w =
            (width - 2.0 * pad_x(model.look) - lead_width(model) - trailing - gap).max(0.0);
        let set = set_for(
            ui,
            model.look,
            matches!(model.trailing, Trailing::Chip { .. }),
        );
        let (title_ty, detail_ty) = (set.title, set.detail);
        let title = wrap(&mut ui.fonts, &model.title, title_ty, text_w).len();
        let detail = match model.detail.as_deref() {
            Some(d) if !d.is_empty() => wrap(&mut ui.fonts, d, detail_ty, text_w).len(),
            _ => 0,
        };
        (title, detail)
    }

    pub fn model(&self) -> &RowModel {
        &self.model
    }

    pub fn id(&self) -> RowId {
        self.model.id
    }

    /// Where the row is drawn (its glass, when it has one).
    pub fn visual(&self) -> Frame {
        self.frame
    }

    /// Where a touch counts: the row, and at least 64 units around its centre in each direction.
    pub fn hit(&self) -> Frame {
        target_around(self.frame, smart::TARGET)
    }

    pub fn group(&self) -> &Group {
        &self.group
    }

    pub fn surface(&self) -> Option<&Surface> {
        self.surface.as_ref()
    }

    /// The layers the row has.
    pub fn needs(&self) -> Needs {
        self.needs
    }

    /// Whether the row is drawn.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// Whether the row is drawn pressed.
    pub fn is_pressed(&self) -> bool {
        self.press.is_pressed()
    }

    /// The lines of the title as drawn, and of the detail.
    pub fn shown_lines(&self) -> (&[String], &[String]) {
        (&self.title_lines, &self.detail_lines)
    }

    /// Only the part of the row inside `clip` (screen units) is drawn: the window of the list the row scrolls in. The edge is hard.
    pub fn set_clip(&mut self, clip: Option<Rect>) {
        self.set_clip_feathered(clip, Edges::NONE);
    }

    /// As [`Row::set_clip`], with the row fading out over `feather` towards the edges of `clip`.
    pub fn set_clip_feathered(&mut self, clip: Option<Rect>, feather: Edges) {
        self.clip = clip;
        self.feather = feather;
        self.apply_clip();
    }

    fn apply_clip(&mut self) {
        for part in &mut self.group.parts {
            part.clip = self.clip;
            part.feather = self.feather;
        }
    }

    /// Ends a press that something else has taken (a list that began to scroll): no activation.
    pub fn cancel(&mut self, ui: &Ui) {
        if self.press.cancel() {
            self.show_down(ui, false);
        }
    }

    /// Shows the row with its top at screen `y`, at once (a finger-follow, or a tile given another row). A press in progress ends.
    pub fn place(&mut self, ui: &Ui, y: f32) {
        self.cancel(ui);
        self.move_to(y);
        if !self.shown {
            self.shown = true;
            self.refresh_live();
        }
        self.group.set_parts(&self.live, self.pose);
    }

    /// Sends the row to where its top is at screen `y` on the compositor's clock: a fling is one `Animate` per layer.
    pub fn glide(&mut self, y: f32, duration_ms: u32, ease: Ease) {
        if !self.shown {
            return;
        }
        self.move_to(y);
        self.group
            .animate_parts(&self.live, self.pose, duration_ms, ease);
    }

    /// How much of its opacity the row keeps in every pose it is put in from now on (a list that fades what is at the edge of its
    /// window): 1 draws the row as posed. Nothing is sent; the next `place` or `glide` carries it.
    pub fn set_alpha(&mut self, alpha: f32) {
        self.group.alpha = alpha;
    }

    /// Moves the layers the row shows to `pose` at once (the whole row arriving or leaving with the overlay or page it sits in) and
    /// keeps it as the pose of every later `place` and `glide`.
    pub fn set_pose(&mut self, pose: Pose) {
        self.pose = pose;
        let live = self.live.clone();
        self.group.set_parts(&live, pose);
    }

    /// Moves the layers the row shows to `pose` over `ms`, on the compositor's clock; later `place` and `glide` keep that pose.
    pub fn animate_pose(&mut self, pose: Pose, ms: u32, ease: Ease) {
        self.pose = pose;
        let live = self.live.clone();
        self.group.animate_parts(&live, pose, ms, ease);
    }

    /// Stops drawing the row (it left the window of its list). Its layers stay for the next row.
    pub fn hide(&mut self, ui: &Ui) {
        self.cancel(ui);
        if self.shown {
            self.shown = false;
            self.refresh_live();
        }
    }

    /// The look was replaced: the glass is restyled and every word and mark is drawn again in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        if let Some(surface) = &mut self.surface {
            surface.restyle(ui, &mut self.group);
        }
        self.text_drawn = None;
        self.trail_drawn = None;
        self.toggle_drawn = None;
        if let Some(i) = self.wash {
            if let Kind::Shape(layer) = &self.group.parts[i].kind {
                layer.reshape(wash_shape(ui));
            }
        }
        if let Some(i) = self.bar {
            if let Kind::Shape(layer) = &self.group.parts[i].kind {
                layer.reshape(bar_shape(ui));
            }
        }
        self.draw(ui, false);
        self.rehide();
    }

    fn move_to(&mut self, y: f32) {
        let dy = y - self.frame.y;
        self.frame.y = y;
        for part in &mut self.group.parts {
            part.frame.y += dy;
        }
        self.group.about = self.frame.centre();
    }

    /// Whether part `i` of the group shows for the row as it is.
    fn wants(&self, i: usize) -> bool {
        if !self.shown {
            return false;
        }
        let model = &self.model;
        if let Some(surface) = &self.surface {
            let (rest, pressed) = surface.parts();
            if i == rest || Some(i) == pressed {
                return match model.look {
                    RowLook::Card => model.actionable,
                    RowLook::Rail => model.state.selected,
                    RowLook::Flat
                    | RowLook::Tile
                    | RowLook::Plain
                    | RowLook::Compact
                    | RowLook::State => false,
                };
            }
            if i == surface.ring_part() {
                return model.actionable && model.state.focus_visible;
            }
        }
        if Some(i) == self.wash {
            return model.actionable && self.press.is_pressed();
        }
        if Some(i) == self.bar {
            return model.look == RowLook::Rail && model.state.selected;
        }
        if Some(i) == self.trail {
            return matches!(model.trailing, Trailing::Picture(_) | Trailing::Badge(_));
        }
        if let Some(toggle) = &self.toggle {
            if i == toggle.track || i == toggle.thumb {
                return matches!(model.trailing, Trailing::Toggle(_));
            }
        }
        true
    }

    /// Works out which parts show, and shows or hides the ones that changed.
    fn refresh_live(&mut self) {
        self.live.clear();
        for i in 0..self.group.parts.len() {
            let want = self.wants(i);
            if want {
                self.live.push(i);
            }
            if want != self.parts_shown[i] {
                let pose = if want {
                    self.pose
                } else {
                    Pose {
                        visible: false,
                        ..self.pose
                    }
                };
                self.group.set_parts(&[i], pose);
                self.parts_shown[i] = want;
            }
        }
    }

    /// Hides every part that is not to show again: a group-wide animation (the press scale) makes all of them visible.
    fn rehide(&mut self) {
        for i in 0..self.group.parts.len() {
            if !self.wants(i) {
                self.group.set_parts(
                    &[i],
                    Pose {
                        visible: false,
                        ..self.pose
                    },
                );
                self.parts_shown[i] = false;
            }
        }
    }

    fn show_down(&mut self, ui: &Ui, down: bool) {
        if let Some(surface) = &mut self.surface {
            surface.set_down(ui, &mut self.group, down);
        }
        self.refresh_live();
        self.rehide();
    }

    /// Gives the row the layers `needs` says, which it did not have. Not for use while a list moves.
    fn grow(&mut self, ui: &mut Ui, needs: Needs) -> io::Result<()> {
        let needs = self.needs.merge(needs);
        if needs.glass && self.surface.is_none() {
            let surface = Surface::add(
                ui,
                &mut self.group,
                self.frame,
                self.z,
                SmartMaterial::ActionRow,
                MaterialContext::Nested,
                self.model.state,
            );
            self.surface = Some(surface);
            self.parts_shown.resize(self.group.parts.len(), true);
        }
        if needs.trail_w > self.needs.trail_w || needs.trail_h > self.needs.trail_h {
            let part = trail_part(
                ui,
                self.frame,
                self.z,
                needs.trail_w.max(self.needs.trail_w),
                trail_height(ui, &needs),
            )?;
            match self.trail {
                Some(i) => self.group.parts[i] = part,
                None => {
                    self.trail = Some(self.group.push(part));
                    self.parts_shown.push(true);
                }
            }
            self.trail_drawn = None;
        }
        if needs.toggle && self.toggle.is_none() {
            self.toggle = Some(toggle_parts(ui, &mut self.group, self.frame, self.z));
            self.parts_shown.resize(self.group.parts.len(), true);
            self.toggle_drawn = None;
        }
        if needs.bar && self.bar.is_none() {
            self.bar = Some(self.group.push(bar_part(ui, self.frame, self.z)));
            self.parts_shown.resize(self.group.parts.len(), true);
        }
        self.needs = needs;
        self.apply_clip();
        Ok(())
    }

    /// Draws what differs from what is drawn: the words and the marks around them, the picture or badge at the end, the toggle.
    /// `animate` lets a toggle's thumb slide.
    fn draw(&mut self, ui: &mut Ui, animate: bool) -> bool {
        let mut drawn = false;

        let key = text_key(ui, &self.model, self.frame.w);
        if self.text_drawn.as_ref() != Some(&key) {
            let part = &mut self.group.parts[self.text];
            let items = text_items(ui, &key, part.frame.w, self.frame.h);
            if let Some(layer) = part.lines_mut() {
                layer.draw(ui, &items);
            }
            drop(items);
            self.title_lines = key.title.clone();
            self.detail_lines = key.detail.clone();
            self.text_drawn = Some(key);
            drawn = true;
        }

        if let Some(index) = self.trail {
            let key = match &self.model.trailing {
                Trailing::Badge(n) => TrailKey::Badge(
                    *n,
                    ui.look.colour(PaletteRole::Accent),
                    ui.look.colour(PaletteRole::AccentInk),
                ),
                Trailing::Picture(picture) => TrailKey::Picture(picture.clone()),
                _ => TrailKey::Empty,
            };
            if self.trail_drawn.as_ref() != Some(&key) {
                if let Some(layer) = self.group.parts[index].image_mut() {
                    paint_trail(ui, layer, &key);
                }
                self.trail_drawn = Some(key);
                drawn = true;
            }
        }

        self.draw_toggle(ui, animate);
        drawn
    }

    fn draw_toggle(&mut self, ui: &Ui, animate: bool) {
        let (Some(toggle), Trailing::Toggle(on)) = (&self.toggle, &self.model.trailing) else {
            return;
        };
        let (track, thumb, on) = (toggle.track, toggle.thumb, *on);
        let enabled = self.model.state.enabled;
        let first = self.toggle_drawn.is_none();
        if self.toggle_drawn == Some((on, enabled)) {
            return;
        }
        self.toggle_drawn = Some((on, enabled));
        let state = ControlState {
            selected: on,
            enabled,
            ..ControlState::default()
        };
        let track_pair = ui
            .look
            .material(SmartMaterial::ToggleTrack, &state, MaterialContext::Nested)
            .sized(TRACK_W, TRACK_H);
        let thumb_pair = ui
            .look
            .material(SmartMaterial::ToggleThumb, &state, MaterialContext::Nested)
            .sized(THUMB_D, THUMB_D);
        if let Some(layer) = self.group.parts[track].glass() {
            layer.restyle(track_pair.rest);
        }
        if let Some(layer) = self.group.parts[thumb].glass() {
            layer.restyle(thumb_pair.rest);
        }
        let x = self.group.parts[track].frame.x
            + if on {
                TRACK_W - THUMB_D - THUMB_INSET
            } else {
                THUMB_INSET
            };
        self.group.parts[thumb].frame.x = x;
        if self.live.contains(&thumb) {
            if animate && !first && ui.look.moves() {
                let ms = ui.look.duration_ms(motion::LABEL_MS);
                self.group
                    .animate_parts(&[thumb], self.pose, ms, motion::SETTLE);
            } else {
                self.group.set_parts(&[thumb], self.pose);
            }
        }
    }

    /// Shows `model` on the row, drawing only what differs from what is drawn (a state change draws nothing; a changed word draws
    /// the text layer once). Layers the model needs and the row lacks are made here, which is why this can fail; a list never
    /// reaches that case, because its tiles are made with the layers of every row.
    pub fn try_update(&mut self, model: &RowModel, ui: &mut Ui) -> io::Result<Damage> {
        if *model == self.model {
            return Ok(Damage::None);
        }
        if model.id != self.model.id || !model.actionable || !model.state.enabled {
            self.cancel(ui);
        }
        let needs = Self::needs_of(model);
        if !self.needs.covers(&needs) {
            self.grow(ui, needs)?;
        }
        let state_changed = model.state != self.model.state;
        // A spare row's text layer was made for the padding of the blank look (the toolkit's 20): a look that keeps another room at
        // its sides (a thing's layer 14, the words of a plate none) has its words where it puts them, and the width it measured.
        if pad_x(model.look) != pad_x(self.model.look) {
            self.reframe_text(ui, model.look)?;
        }
        self.model = model.clone();
        if state_changed {
            if let Some(surface) = &mut self.surface {
                surface.set_state(ui, &mut self.group, model.state);
            }
            self.rehide();
        }
        self.refresh_live();
        Ok(if self.draw(ui, self.shown) {
            Damage::Content
        } else {
            Damage::Properties
        })
    }

    /// The text layer is made again for the room a row of `look` keeps at its sides, to be drawn at the next `draw`.
    fn reframe_text(&mut self, ui: &mut Ui, look: RowLook) -> io::Result<()> {
        let pad = pad_x(look);
        let frame = Frame::new(
            self.frame.x + pad,
            self.frame.y,
            (self.frame.w - 2.0 * pad).max(1.0),
            self.frame.h,
        );
        let mut part = Part::new(TextLayer::new(ui, frame, self.z + 1, &[])?, frame);
        part.clip = self.clip;
        part.feather = self.feather;
        self.group.parts[self.text] = part;
        self.text_drawn = None;
        Ok(())
    }

    fn action(&self) -> RowAction {
        match self.model.trailing {
            Trailing::Toggle(on) => RowAction::Proposed(!on),
            _ => RowAction::Activated,
        }
    }

    fn value_text(&self) -> Option<String> {
        let trailing = match &self.model.trailing {
            Trailing::Value(text)
            | Trailing::Count(text)
            | Trailing::Play(text)
            | Trailing::Reading { text, .. } => Some(text.clone()),
            Trailing::Limit { value, of } => Some(format!("{value} {of}")),
            Trailing::Chip { text, .. } => Some(text.clone()),
            Trailing::Toggle(true) => Some("On".to_owned()),
            Trailing::Toggle(false) => Some("Off".to_owned()),
            Trailing::Signal { bars, .. } => Some(format!("signal {} of 4", (*bars).min(4))),
            Trailing::Badge(n) => Some(format!("{n} waiting")),
            _ => None,
        };
        let detail = self.model.detail.clone().filter(|d| !d.is_empty());
        match (trailing, detail) {
            (Some(t), Some(d)) => Some(format!("{t}, {d}")),
            (t, d) => t.or(d),
        }
    }
}

/// How much of the accent is in the wash a finger leaves: a little over a seventh, so that the wash is the look's own colour, faint on
/// the sheet, and never plain ink (which is white in a dark look).
const WASH_STRENGTH: f32 = 0.16;

/// The wash a touch leaves on a row, a tile or a segment of a control: the accent of the look at a low strength, with corners of
/// `radius` (the shape's own: a row's, an option's).
pub fn press_wash(ui: &Ui, radius: f32) -> compositor_protocol::Shape {
    solid(
        ui.look
            .theme
            .colour_scaled(PaletteRole::Accent, WASH_STRENGTH),
        radius,
    )
}

/// The flat wash a touch leaves on a row.
fn wash_shape(ui: &Ui) -> compositor_protocol::Shape {
    press_wash(ui, radius::ROW)
}

fn wash_part(ui: &Ui, frame: Frame, z: i32) -> Part {
    Part::new(
        ui.scene.shape(
            frame.w.round() as u32,
            frame.h.round() as u32,
            z,
            wash_shape(ui),
            Props::at(frame.x, frame.y),
        ),
        frame,
    )
}

/// The bar at the left edge of the selected rail entry: the accent.
fn bar_shape(ui: &Ui) -> compositor_protocol::Shape {
    solid(ui.look.accent(), 2.0)
}

fn bar_part(ui: &Ui, frame: Frame, z: i32) -> Part {
    let at = Frame::new(frame.x + 6.0, frame.y + (frame.h - 28.0) / 2.0, 4.0, 28.0);
    Part::new(
        ui.scene.shape(
            at.w as u32,
            at.h as u32,
            z + 2,
            bar_shape(ui),
            Props::at(at.x, at.y),
        ),
        at,
    )
}

/// The words of a row's text layer and what is in it besides, for the model: its title and detail wrapped to the width, the inks,
/// and what stands at the trailing end (the value, icon or signal that is in the layer; a picture or badge has a layer of its own).
fn text_key(ui: &mut Ui, model: &RowModel, frame_w: f32) -> TextKey {
    let mut m = metrics(ui, model, frame_w);
    let end_ink = if model.state.enabled {
        ui.look.ink2()
    } else {
        ui.look.disabled_ink()
    };
    // The check of a selected row, and the lit bars of a signal: the primary ink, or the secondary when the row is disabled.
    let lit_ink = if model.state.enabled {
        ui.look.ink()
    } else {
        ui.look.disabled_ink()
    };
    let trail = match (&model.trailing, m.value.take()) {
        (Trailing::Chevron, _) => TrailMark::Icon(Icon::ChevronRight, end_ink),
        (Trailing::Check(true), _) => TrailMark::Icon(Icon::Check, lit_ink),
        (Trailing::Toggle(true), _) => TrailMark::Tick(tick_ink(ui, model.state.enabled)),
        (Trailing::Icon(icon), _) => TrailMark::Icon(*icon, end_ink),
        (Trailing::Value(_), Some(value)) => TrailMark::Value(value, end_ink),
        (Trailing::Reading { tone, mark, .. }, Some(value)) => {
            let ink = if model.state.enabled {
                match tone {
                    None => end_ink,
                    Some(ValueTone::Good) => ui.look.colour(PaletteRole::Positive),
                    Some(ValueTone::Warn) => ui.look.colour(PaletteRole::Warning),
                    Some(ValueTone::Bad) => ui.look.colour(PaletteRole::Danger),
                    Some(ValueTone::Muted) => ui.look.ink3(),
                }
            } else {
                end_ink
            };
            // The mark is in the ink of the words: the colour and the glyph say the same thing together.
            TrailMark::Reading(value, ink, mark.map(|m| (m.icon(), ink)))
        }
        (Trailing::Count(_), Some(value)) => TrailMark::Count(value, end_ink, COUNT_GAP),
        (Trailing::Play(_), Some(value)) => TrailMark::Count(value, end_ink, PLAY_ROOM),
        (Trailing::Limit { value: reading, of }, Some(shown)) => {
            // Whole, the reading and its limit are two inks; cut to fit, they are one.
            let dim = if model.state.enabled {
                ui.look.theme.colour_scaled(PaletteRole::Ink2, 0.7)
            } else {
                end_ink
            };
            if shown == format!("{reading} {of}") {
                TrailMark::Limit(reading.clone(), of.clone(), end_ink, dim)
            } else {
                TrailMark::Value(shown, end_ink)
            }
        }
        (Trailing::Chip { tone, check, .. }, Some(value)) => {
            // A pill: the words in the tone's ink, on a faint tint of the same ink (the quiet ink when there is no tone).
            let ink = if model.state.enabled {
                match tone {
                    Some(ValueTone::Good) => ui.look.colour(PaletteRole::Positive),
                    Some(ValueTone::Warn) => ui.look.colour(PaletteRole::Warning),
                    Some(ValueTone::Bad) => ui.look.colour(PaletteRole::Danger),
                    Some(ValueTone::Muted) | None => end_ink,
                }
            } else {
                end_ink
            };
            let tint = if tone.is_some_and(|t| t != ValueTone::Muted) {
                CHIP_TINT
            } else {
                CHIP_QUIET_TINT
            };
            TrailMark::Chip(value, ink, premultiplied(ink, tint), *check)
        }
        (Trailing::Signal { bars, locked }, _) => TrailMark::Signal(
            (*bars).min(4),
            *locked,
            lit_ink,
            ui.look.colour(PaletteRole::TrackOff),
        ),
        _ => TrailMark::None,
    };
    let dress = model.dress.as_ref().map(|d| {
        let tone_ink = |tone: Option<ValueTone>| match tone {
            Some(ValueTone::Good) => ui.look.colour(PaletteRole::Positive),
            Some(ValueTone::Warn) => ui.look.colour(PaletteRole::Warning),
            Some(ValueTone::Bad) => ui.look.colour(PaletteRole::Danger),
            Some(ValueTone::Muted) | None => ui.look.ink2(),
        };
        DressKey {
            tile: d.tile,
            // A state that holds tints the square with the accent; the other states with their own colour.
            tint: match d.tint {
                Some(ValueTone::Good) => ui.look.accent(),
                other => tone_ink(other),
            },
            field: d.field,
            wash: match (d.field, d.tint) {
                (false, _) => 48,
                (true, Some(ValueTone::Good | ValueTone::Bad)) => 41,
                (true, Some(ValueTone::Warn)) => 51,
                (true, Some(ValueTone::Muted) | None) => 48,
            },
            tag: d.tag.clone(),
            status: d.status.as_ref().map(|s| (tone_ink(Some(s.tone)), s.ring)),
        }
    });
    TextKey {
        title: m.title,
        detail: m.detail,
        ink: if model.state.enabled {
            ui.look.ink()
        } else {
            ui.look.ink2()
        },
        ink2: ui.look.ink2(),
        text_w: m.text_w,
        look: model.look,
        leading: model.leading,
        // The accent and the selection tint only the icon before the words: a row without one draws its words as it did.
        accent: if model.leading.is_some() {
            ui.look.accent()
        } else {
            CLEAR
        },
        selected: model.state.selected && model.leading.is_some(),
        trail,
        dress,
    }
}

/// What a row's text layer shows, in the order it is painted, in a layer `w` wide and `h` high: the icon before the words (on a
/// tinted square for a tile; in the accent for the selected rail entry), the title and detail lines, what stands at the end of the
/// row, and the hairline along the bottom of a flat row. The marks and words of different colours do not overlap, so their order
/// only fixes which colour is which ink.
fn text_items<'a>(ui: &mut Ui, key: &'a TextKey, w: f32, h: f32) -> Vec<Item<'a>> {
    let set = set_for(ui, key.look, matches!(key.trail, TrailMark::Chip(..)));
    let (title_ty, detail_ty) = (set.title, set.detail);
    // The words and the icon are centred in the row's own height; what is placed against the layer's edges (the end of the row,
    // its bottom line) goes by the layer's size in pixels.
    let (px_w, px_h) = (w.ceil(), h.ceil());
    let mut items: Vec<Item> = Vec::new();
    if let Some(icon) = key.leading {
        match key.look {
            RowLook::Tile => {
                let y = ((h - CHIP) / 2.0).round().max(0.0) as i32;
                items.push(tinted_square(0, y, CHIP as u32, 16, 40, key.accent).into());
                let at = ((CHIP - LEAD_ICON) / 2.0).round() as i32;
                items.push(Mark::icon(icon, LEAD_ICON as u32, at, y + at, key.accent).into());
            }
            _ => {
                let top = ((h - LEAD_ICON) / 2.0).round() as i32;
                let colour = if key.selected { key.accent } else { key.ink };
                items.push(Mark::icon(icon, LEAD_ICON as u32, 0, top, colour).into());
            }
        }
    }
    let (title_h, detail_h) = (set.title_line, set.detail_line);
    let block = set.height(key.title.len(), key.detail.len());
    let mut y = ((h - block) / 2.0).round().max(0.0);
    let lead = match (&key.dress, key.leading, key.look) {
        (Some(_), _, _) => tile_of(key.look).side + tile_of(key.look).gap,
        (None, None, _) => 0.0,
        (None, Some(_), RowLook::Tile) => CHIP + LEAD_GAP,
        (None, Some(_), _) => LEAD_ICON + LEAD_GAP,
    };
    if let Some(dress) = &key.dress {
        let tile = tile_of(key.look);
        let top = ((h - tile.side) / 2.0).round().max(0.0) as i32;
        let side = tile.side as u32;
        // A settings page's field has the mock's corner: 14 on a page's row, 12 on the entry of a thing's layer (`.perm .pi`); the
        // toolkit's other tiles keep 12.
        let corner = if dress.field && key.look != RowLook::Compact {
            14
        } else {
            12
        };
        items.push(tinted_square(0, top, side, corner, dress.wash, dress.tint).into());
        let at = ((tile.side - tile.glyph) / 2.0).round() as i32;
        items.push(Mark::icon(dress.tile, tile.glyph as u32, at, top + at, dress.tint).into());
    }
    for line in &key.title {
        let at = Set::run_y(y, title_h, title_ty);
        items.push(Run::left(line, title_ty, key.ink, lead, at, key.text_w).into());
        if let Some(tag) = key.dress.as_ref().and_then(|d| d.tag.as_deref()) {
            let x = lead + ui.fonts.measure(line, title_ty).width.ceil() + TAG_GAP;
            let at = Set::run_y(
                y + ((title_h - detail_h) / 2.0).round(),
                detail_h,
                detail_ty,
            );
            items.push(Run::left(tag, detail_ty, key.ink2, x, at, key.text_w).into());
        }
        y += title_h;
    }
    if !key.detail.is_empty() {
        y += set.gap;
    }
    for line in &key.detail {
        match key.dress.as_ref().and_then(|d| d.status) {
            Some((ink, ring)) => {
                // The dot stands on the line's middle; a ring is the toolkit's empty circle, a disc a mark of its own.
                let mid = y + detail_h / 2.0;
                if ring {
                    let side = DOT as u32 + 4;
                    let at = (mid - side as f32 / 2.0).round() as i32;
                    items.push(Mark::icon(Icon::Circle, side, lead as i32 - 2, at, ink).into());
                } else {
                    let at = (mid - DOT / 2.0).round() as i32;
                    items.push(Mark::disc(lead as i32, at, DOT as u32, ink).into());
                }
                let at = Set::run_y(y, detail_h, detail_ty);
                items.push(Run::left(line, detail_ty, ink, lead + DOT_ROOM, at, key.text_w).into());
            }
            None => {
                let at = Set::run_y(y, detail_h, detail_ty);
                items.push(Run::left(line, detail_ty, key.ink2, lead, at, key.text_w).into());
            }
        }
        y += detail_h;
    }
    let nav = nav_row(key.look, key.dress.as_ref().is_some_and(|d| d.field));
    match &key.trail {
        TrailMark::None => {}
        TrailMark::Icon(Icon::ChevronRight, ink) if nav => {
            // The mock's `.chev`: 28, its box starting where the 24 glyph's did and reaching 4 past the row's content.
            let at = ((h - NAV_ICON) / 2.0).round() as i32;
            items.push(
                Mark::icon(
                    Icon::ChevronRight,
                    NAV_ICON as u32,
                    (px_w - ICON) as i32,
                    at,
                    *ink,
                )
                .into(),
            );
        }
        TrailMark::Icon(icon, ink) => {
            let side = ICON as u32;
            let at = ((h - ICON) / 2.0).round() as i32;
            items.push(Mark::icon(*icon, side, (px_w - ICON) as i32, at, *ink).into());
        }
        TrailMark::Tick(ink) => {
            // Over the knob at the track's far end: the track ends where the row's padding begins (`PAD_X`, which is `pad_x` of the look
            // but for a compact row's and a plate's), the knob stands its inset in from that end and the tick in the middle of it.
            let side = TICK as u32;
            let end = px_w - (PAD_X - pad_x(key.look));
            let x = end - THUMB_INSET - THUMB_D / 2.0 - TICK / 2.0;
            let at = ((h - TICK) / 2.0).round() as i32;
            items.push(Mark::icon(Icon::Tick, side, x.round() as i32, at, *ink).into());
        }
        TrailMark::Value(text, ink) => {
            let ty = set.value;
            let y = value_top(nav, h, ty);
            items.push(Run::right(text, ty, *ink, 0.0, y, px_w).into());
        }
        TrailMark::Limit(value, of, ink, dim) => {
            // The limit at the very end in the dim ink, the reading before it, a space between: the mock's `.mv` with its `of 1 %`
            // at .7.
            let ty = set.value;
            let y = ((h - line_height(ty)) / 2.0).round().max(0.0);
            let whole = ui.fonts.measure(&format!("{value} {of}"), ty).width;
            let (v, o) = (
                ui.fonts.measure(value, ty).width,
                ui.fonts.measure(of, ty).width,
            );
            let gap = (whole - v - o).max(0.0);
            items.push(Run::right(of, ty, *dim, 0.0, y, px_w).into());
            items.push(Run::right(value, ty, *ink, 0.0, y, px_w - o - gap).into());
        }
        TrailMark::Count(text, ink, gap) => {
            let ty = set.value;
            let y = value_top(nav, h, ty);
            // What the row leaves before its chevron is what it says (a play button's room is its own); a row of the Presence look ends
            // as the mock's `nav` row does, 16 before the chevron of 28.
            let gap = if nav { (*gap).max(NAV_GAP) } else { *gap };
            items.push(Run::right(text, ty, *ink, 0.0, y, px_w - ICON - gap).into());
            let (side, at) = if nav {
                (NAV_ICON, ((h - NAV_ICON) / 2.0).round() as i32)
            } else {
                (ICON, ((h - ICON) / 2.0).round() as i32)
            };
            items.push(
                Mark::icon(
                    Icon::ChevronRight,
                    side as u32,
                    (px_w - ICON) as i32,
                    at,
                    *ink,
                )
                .into(),
            );
        }
        TrailMark::Chip(text, ink, fill, check) => {
            let ty = chip_type(ui);
            let w = chip_width(ui, text, *check).min(px_w);
            let (x, top) = ((px_w - w) as i32, ((h - CHIP_PILL_H) / 2.0).round() as i32);
            items.push(
                Mark::round(
                    x,
                    top,
                    w as u32,
                    CHIP_PILL_H as u32,
                    (CHIP_PILL_H / 2.0) as u32,
                    *fill,
                )
                .into(),
            );
            if *check {
                let at = ((h - CHIP_ICON) / 2.0).round() as i32;
                items.push(
                    Mark::icon(Icon::Check, CHIP_ICON as u32, x + CHIP_PAD as i32, at, *ink).into(),
                );
            }
            let y = ((h - line_height(ty)) / 2.0).round().max(0.0);
            items.push(Run::right(text, ty, *ink, 0.0, y, px_w - CHIP_PAD).into());
        }
        TrailMark::Reading(text, ink, mark) => {
            let ty = set.value;
            let y = value_top(nav, h, ty);
            items.push(Run::right(text, ty, *ink, 0.0, y, px_w).into());
            if let Some((icon, mark_ink)) = mark {
                let words = ui.fonts.measure(text, ty).width.ceil();
                let x = (px_w - words - MARK_GAP - MARK).round() as i32;
                let top = ((h - MARK) / 2.0).round() as i32;
                items.push(Mark::icon(*icon, MARK as u32, x, top, *mark_ink).into());
            }
        }
        TrailMark::Signal(bars, locked, lit, dim) => {
            let bars_w = (4 * BAR_W + 3 * BAR_GAP) as i32;
            let bottom = (px_h as i32 + BAR_HEIGHTS[3] as i32) / 2;
            for (i, height) in BAR_HEIGHTS.iter().enumerate() {
                let colour = if (i as u8) < *bars { *lit } else { *dim };
                let x = px_w as i32 - bars_w + i as i32 * (BAR_W + BAR_GAP) as i32;
                items.push(Mark::bar(x, bottom - *height as i32, BAR_W, *height, colour).into());
            }
            if *locked {
                let side = ICON as i32;
                let x = px_w as i32 - (bars_w + 8 + side);
                let top = (px_h as i32 - side) / 2;
                items.push(Mark::icon(Icon::Lock, side as u32, x, top, *lit).into());
            }
        }
    }
    if key.look == RowLook::Flat && px_h > 0.0 {
        items.push(Mark::rule(0, px_h as i32 - 1, px_w as u32, rule_colour(key.ink2)).into());
    }
    items
}

/// The tinted square a tile's glyph stands on: `side` wide and high, its top-left corner at (`x`, `y`), `wash` of 255 of `tint`. With
/// an opaque tint (the accent, a tone's ink) it is a veil of the ink the glyph is drawn in, so that the square takes no ink of its own
/// in the row's run: the glyph, the title, the detail and the hairline are four, which is all a run has, and a dressed row of the
/// Presence page (a tile, a title, a detail with a value and a hairline) with a fifth was a picture. A translucent tint (the quiet ink)
/// is not a level of an ink the glyph has: the square is a colour of its own, as it was.
fn tinted_square(x: i32, y: i32, side: u32, radius: u32, wash: u8, tint: Rgba) -> Mark {
    if tint.3 == 255 {
        Mark::veil(x, y, side, side, radius, wash, tint)
    } else {
        Mark::round(x, y, side, side, radius, premultiplied(tint, wash))
    }
}

/// `colour` (straight) at `alpha`, premultiplied: what a tint of an accent is as a colour of a layer.
fn premultiplied(colour: Rgba, alpha: u8) -> Rgba {
    let scale = |c: u8| (u32::from(c) * u32::from(alpha) / 255) as u8;
    Rgba(scale(colour.0), scale(colour.1), scale(colour.2), alpha)
}

/// The colour of the hairline of a flat row: the secondary ink at a fifth of its opacity (premultiplied).
pub fn rule_colour(ink2: Rgba) -> Rgba {
    let a = (RULE_ALPHA * f32::from(ink2.3)) as u32;
    let premul = |v: u8| (u32::from(v) * a / 255) as u8;
    Rgba(premul(ink2.0), premul(ink2.1), premul(ink2.2), a as u8)
}

/// The layer of a row's trailing text or icon: `width` wide, as tall as a line of its text, at the right end of the row.
fn trail_part(ui: &Ui, frame: Frame, z: i32, width: f32, height: f32) -> io::Result<Part> {
    let h = height;
    let at = Frame::new(
        frame.right() - PAD_X - width,
        frame.y + (frame.h - h) / 2.0,
        width,
        h,
    );
    let layer = ui.scene.image(
        at.w.ceil() as u32,
        at.h.ceil() as u32,
        z + 1,
        Props::at(at.x, at.y),
    )?;
    Ok(Part::new(layer, at))
}

/// The ink of a toggle's tick: the accent on the light knob, the disabled ink when the row is disabled.
fn tick_ink(ui: &Ui, enabled: bool) -> Rgba {
    if enabled {
        ui.look.accent()
    } else {
        ui.look.disabled_ink()
    }
}

/// The track and the knob of a row's toggle, as glass of the toggle materials, resting off; the row's first draw sets them. They are
/// at the row's own depth (made after its glass, so above it) and under its text layer (one above), so that the tick, a mark of that
/// layer, is drawn over the knob.
fn toggle_parts(ui: &Ui, group: &mut Group, frame: Frame, z: i32) -> Toggle {
    let track = Frame::new(
        frame.right() - PAD_X - TRACK_W,
        frame.y + (frame.h - TRACK_H) / 2.0,
        TRACK_W,
        TRACK_H,
    );
    let thumb = Frame::new(
        track.x + THUMB_INSET,
        track.y + (TRACK_H - THUMB_D) / 2.0,
        THUMB_D,
        THUMB_D,
    );
    let state = ControlState::default();
    let mut part = |role: SmartMaterial, at: Frame, z: i32| {
        let pair = ui
            .look
            .material(role, &state, MaterialContext::Nested)
            .sized(at.w, at.h);
        group.push(Part::new(
            ui.scene.glass(
                at.w as u32,
                at.h as u32,
                z,
                pair.rest,
                Props::at(at.x, at.y),
            ),
            at,
        ))
    };
    Toggle {
        track: part(SmartMaterial::ToggleTrack, track, z),
        thumb: part(SmartMaterial::ToggleThumb, thumb, z),
    }
}

/// Paints a filled disc of radius `r` at (`cx`, `cy`) into premultiplied `pixels` `w` by `h`, edges softened over a pixel.
fn fill_disc(pixels: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, r: f32, colour: Rgba) {
    let (x_from, x_to) = (
        (cx - r - 1.0).max(0.0) as usize,
        ((cx + r + 1.0) as usize).min(w),
    );
    let (y_from, y_to) = (
        (cy - r - 1.0).max(0.0) as usize,
        ((cy + r + 1.0) as usize).min(h),
    );
    for y in y_from..y_to {
        for x in x_from..x_to {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cover = (r - d + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let a = (f32::from(colour.3) * cover).round() as u32;
            let premul = |c: u8| (u32::from(c) * a / 255) as u8;
            let at = (y * w + x) * 4;
            pixels[at..at + 4].copy_from_slice(&[
                premul(colour.0),
                premul(colour.1),
                premul(colour.2),
                a as u8,
            ]);
        }
    }
}

/// Copies `picture` into the layer's pixels, centred, with its corners rounded (the edge of a corner is anti-aliased by coverage).
fn paint_picture(pixels: &mut [u8], w: usize, h: usize, picture: &Thumbnail) {
    let (pw, ph) = (picture.width as usize, picture.height as usize);
    if picture.rgba.len() != pw * ph * 4 {
        return;
    }
    let left = w.saturating_sub(pw);
    let top = h.saturating_sub(ph) / 2;
    let r = PICTURE_RADIUS;
    for y in 0..ph.min(h) {
        for x in 0..pw.min(w) {
            // How far this pixel's centre is outside the rounded corner it is nearest to (negative: inside).
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (r - cx).max(cx - (pw as f32 - r)).max(0.0);
            let dy = (r - cy).max(cy - (ph as f32 - r)).max(0.0);
            let outside = (dx * dx + dy * dy).sqrt() - r;
            let cover = (0.5 - outside).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let from = (y * pw + x) * 4;
            let to = ((top + y) * w + left + x) * 4;
            for c in 0..4 {
                pixels[to + c] = (f32::from(picture.rgba[from + c]) * cover).round() as u8;
            }
        }
    }
}

fn paint_trail(ui: &mut Ui, layer: &mut crate::scene::ImageLayer, key: &TrailKey) {
    match key {
        TrailKey::Picture(picture) => {
            let (w, h) = (layer.width() as usize, layer.height() as usize);
            layer.clear();
            paint_picture(layer.pixels_mut(), w, h, picture);
            layer.damage(None);
        }
        TrailKey::Empty => {
            layer.clear();
            layer.damage(None);
        }
        TrailKey::Badge(count, disc, ink) => {
            let (w, h) = (layer.width() as f32, layer.height() as f32);
            layer.clear();
            let d = ICON;
            let (x0, y0) = (w - d, ((h - d) / 2.0).round());
            fill_disc(
                layer.pixels_mut(),
                w as usize,
                h as usize,
                x0 + d / 2.0,
                y0 + d / 2.0,
                d / 2.0,
                *disc,
            );
            // The count goes over the disc: drawn straight on the pixels, since the runs painter starts from a clear layer.
            let ty = ui.look.ty(ty::CAPTION);
            let text = count.to_string();
            let extent = ui.fonts.measure(&text, ty);
            let x = x0 + ((d - extent.width) / 2.0).max(0.0);
            let baseline = y0 + ((d - extent.height()) / 2.0 + extent.ascent).round();
            let width = layer.width() as usize;
            ui.fonts.draw(
                &mut Canvas {
                    pixels: layer.pixels_mut(),
                    width,
                },
                &text,
                ty,
                (x, baseline),
                *ink,
            );
            layer.damage(None);
        }
    }
}

impl Component for Row {
    type Model = RowModel;
    type Action = RowAction;

    fn update(&mut self, model: &RowModel, ui: &mut Ui) -> Damage {
        // A layer that cannot be made leaves the row as it was; `try_update` says why.
        self.try_update(model, ui).unwrap_or(Damage::None)
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<RowAction> {
        let Event::Touch(touch) = event else {
            return None;
        };
        if !self.shown || !self.model.actionable || !self.model.state.enabled {
            return None;
        }
        let was = self.press.is_pressed();
        let step = self.press.feed(&self.hit(), touch);
        let now = step.pressed_after(was);
        if now != was {
            if now && self.needs.wash && self.wash.is_none() {
                self.wash = Some(self.group.push(wash_part(ui, self.frame, self.z)));
                self.parts_shown.resize(self.group.parts.len(), true);
            }
            self.show_down(ui, now);
        }
        (step == Step::Activated).then(|| self.action())
    }

    fn frame(&self) -> Frame {
        self.hit()
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        if !self.shown {
            return;
        }
        let model = &self.model;
        let frame = self
            .clip
            .and_then(|c| self.frame.intersect(&Frame::new(c.x, c.y, c.w, c.h)))
            .unwrap_or(self.frame);
        let kind = if matches!(model.trailing, Trailing::Toggle(_)) {
            SemanticKind::Toggle
        } else {
            SemanticKind::Row
        };
        let selected = model.state.selected
            || matches!(
                model.trailing,
                Trailing::Check(true) | Trailing::Toggle(true)
            );
        let mut node = SemanticNode::new(kind, model.title.clone(), frame)
            .enabled(model.state.enabled)
            .selected(selected);
        if model.actionable {
            node = node.actionable();
        }
        if let Some(value) = self.value_text() {
            node = node.value(value);
        }
        out.push(node);
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    use crate::testing::harness;

    const FRAME: Frame = Frame::new(24.0, 100.0, 912.0, 64.0);

    /// The pixels of the row's text layer, and its width.
    fn text_pixels(h: &crate::testing::Harness, row: &Row) -> (Vec<u8>, usize) {
        let part = &row.group.parts[row.text];
        let Kind::Lines(layer) = &part.kind else {
            panic!("a row's words are a text layer");
        };
        (h.layer_pixels(layer.id()), part.frame.w.ceil() as usize)
    }

    /// The alpha-premultiplied pixel on the bottom line of bar `i`, where every bar is drawn.
    fn bar_base(pixels: &[u8], w: usize, h_rows: usize, i: usize) -> [u8; 4] {
        let x = w - (4 * BAR_W + 3 * BAR_GAP) as usize + i * (BAR_W + BAR_GAP) as usize + 2;
        let y = (h_rows + BAR_HEIGHTS[3] as usize) / 2 - 3;
        let at = (y * w + x) * 4;
        pixels[at..at + 4].try_into().unwrap()
    }

    /// Whether anything is drawn in the room before the bars where a lock stands.
    fn lock(pixels: &[u8], w: usize) -> bool {
        let rows = pixels.len() / 4 / w;
        let bars = (4 * BAR_W + 3 * BAR_GAP) as usize;
        (w - bars - 8 - ICON as usize..w - bars - 8)
            .any(|x| (0..rows).any(|y| pixels[(y * w + x) * 4 + 3] > 0))
    }

    #[test]
    fn a_signal_lights_exactly_its_bars_and_a_secured_one_has_a_lock_before_them_and_an_open_one_none()
     {
        let mut h = harness();
        let model = RowModel::new(RowId(1), "Home").trailing(Trailing::Signal {
            bars: 3,
            locked: true,
        });
        let row = Row::new(&mut h.ui, FRAME, 10, &model).unwrap();
        h.commit();
        let (pixels, w) = text_pixels(&h, &row);
        let rows = pixels.len() / 4 / w;
        let (lit, dim) = (h.ui.look.ink(), h.ui.look.colour(PaletteRole::TrackOff));
        let seen: Vec<[u8; 4]> = (0..4).map(|i| bar_base(&pixels, w, rows, i)).collect();
        assert!(
            seen[..3].iter().all(|p| *p == seen[0]) && seen[0][3] == lit.3,
            "three lit bars alike: {seen:?}"
        );
        assert_ne!(seen[3], seen[0], "the fourth is dim");
        assert_eq!(seen[3][3], dim.3);
        assert!(lock(&pixels, w), "a secured network shows its lock");

        let open = RowModel::new(RowId(2), "Cafe").trailing(Trailing::Signal {
            bars: 4,
            locked: false,
        });
        let row = Row::new(&mut h.ui, FRAME, 10, &open).unwrap();
        h.commit();
        let (pixels, w2) = text_pixels(&h, &row);
        assert_eq!(w2, w);
        assert!(!lock(&pixels, w), "an open network has no lock");
    }

    #[test]
    fn a_signal_is_described_in_words_and_changing_the_bars_changes_only_the_mark() {
        let mut h = harness();
        let mut model =
            RowModel::new(RowId(1), "Home")
                .detail("Saved")
                .trailing(Trailing::Signal {
                    bars: 2,
                    locked: true,
                });
        let mut row = Row::new(&mut h.ui, FRAME, 10, &model).unwrap();
        h.commit();
        let mut sink = SemanticsSink::new();
        row.semantics(&mut sink);
        assert_eq!(
            sink.find("Home").and_then(|n| n.value.as_deref()),
            Some("signal 2 of 4, Saved")
        );
        let (before, w) = text_pixels(&h, &row);
        model.trailing = Trailing::Signal {
            bars: 4,
            locked: true,
        };
        assert_eq!(row.update(&model, &mut h.ui), Damage::Content);
        h.commit();
        let (after, _) = text_pixels(&h, &row);
        assert_ne!(after, before);
        // The words did not change: every pixel that did is in the room of the mark.
        let mark_from = w - (4 * BAR_W + 3 * BAR_GAP) as usize - 8 - ICON as usize;
        for (i, (a, b)) in after
            .chunks_exact(4)
            .zip(before.chunks_exact(4))
            .enumerate()
        {
            assert!(
                a == b || i % w >= mark_from,
                "pixel {} moved outside the mark",
                i % w
            );
        }
        assert_eq!(row.update(&model, &mut h.ui), Damage::None);
        // The mark is in the text layer: a signal needs no layer of its own, whatever the text size.
        assert_eq!(Row::needs_of(&model).trail_w, 0.0);
        assert_eq!(row.group().parts.len(), 1);
    }
}
