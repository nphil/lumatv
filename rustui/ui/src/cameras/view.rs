//! What the surfaces draw: the app's [`View`], with every word composed, every time formatted and every picture named by its key, so a
//! surface lays out and paints and never decides anything (plan section 4.4: `update`, `view`, `due_at`).

use super::data::Span;
use super::grid::{Focus as GridFocus, Layout};
use super::strip::{Bar, Item as StripItem};
use crate::kit::hint::{Action, Key};

/// What a hint says the keys do: one of the kit's glyphs, or a Lucide icon of the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    Kit(Action),
    Icon(&'static str),
}

/// One hint: the keys' glyphs, then what they do (never a sentence), and the accessible name.
#[derive(Clone, Debug, PartialEq)]
pub struct HintView {
    pub keys: Vec<Key>,
    /// The keys are held, not pressed (OK held: `key-ok-hold`).
    pub held: bool,
    pub does: Do,
    pub name: String,
}

impl HintView {
    pub fn new(keys: &[Key], does: Do, name: &str) -> Self {
        Self { keys: keys.to_vec(), held: false, does, name: name.to_string() }
    }
}

/// Everything on the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub base: Base,
    pub strip: Option<StripView>,
    pub sheet: Option<SheetView>,
    pub options: Option<OptionsView>,
    /// A refusal's words (3 s), over the screen it came from.
    pub toast: Option<String>,
    /// An app's picture is under the layer: the glass is painted at the reading alpha, and live video is the video plane's.
    pub over_app: bool,
}

impl View {
    pub fn closed() -> Self {
        Self { base: Base::Closed, strip: None, sheet: None, options: None, toast: None, over_app: false }
    }

    pub fn is_up(&self) -> bool {
        !matches!(self.base, Base::Closed)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    Closed,
    Grid(GridView),
    Full(FullView),
}

/// A tile of the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct TileView {
    pub id: String,
    pub name: String,
    /// The key of its picture in the picture store.
    pub pic: String,
    /// Centred over the picture, up to two lines; empty while the picture is fresh.
    pub words: Vec<String>,
    pub dim: bool,
}

/// No tile: the state's words and one primary button, `Settings`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Empty {
    pub icon: &'static str,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridView {
    /// "8 cameras", or nothing while there are none.
    pub count_words: String,
    /// One line with the state's words while tiles exist and the link is not `ready`.
    pub banner: Option<String>,
    pub empty: Option<Empty>,
    pub tiles: Vec<TileView>,
    pub layout: Layout,
    /// The first shown row.
    pub top: usize,
    /// The foot's left: the range left below ("9 to 12") with `chevron-down`.
    pub below: Option<String>,
    pub above: Option<String>,
    pub focus: GridFocus,
    pub hints: Vec<HintView>,
}

/// What the name pill says of the picture: the live dot and "Live", "Starting", "Recorded", "Can't play".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateWord {
    pub word: &'static str,
    pub dot: bool,
}

/// The note over a camera that cannot play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: &'static str,
    pub detail: String,
    pub button: &'static str,
}

/// What is under the chrome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backdrop {
    /// Nothing is drawn: the video plane shows through (the hole), or the layer is black.
    None,
    /// The camera's stills (key of the newest).
    Still(String),
    /// The cursor's thumbnail while a playback is asked for.
    Thumb(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FullView {
    pub name: String,
    /// "2 of 8" when there is more than one camera.
    pub position: Option<String>,
    pub state: StateWord,
    /// `3840×2160 · H.265 · 15 fps`; nothing when the stream gives no member.
    pub info: Option<String>,
    pub note: Option<Note>,
    /// The layer is transparent over the video plane: nothing of the picture is drawn.
    pub hole: bool,
    pub backdrop: Backdrop,
    /// How dark the backdrop is veiled: 0 none, else the alpha of the veil (starting 0.35, failed 0.62).
    pub veil: f32,
    /// The chrome (pills, hints) is up; the surface fades it.
    pub chrome: bool,
    /// "Opens in a few seconds" while a stream that is not prebuffered starts.
    pub line: Option<String>,
    /// Empty while the strip, the sheet or the options page is up.
    pub hints: Vec<HintView>,
    /// Where the focus is: the note's button, when there is a note.
    pub button_focused: bool,
    /// The compositor refused the live picture a video window or took it back: a glyph beside the name says the picture is on the plane.
    pub window_cue: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BubbleView {
    pub at: i64,
    pub time: String,
    pub kind_word: Option<String>,
    /// The key of its picture (the last kept until the next arrives), if one has.
    pub pic: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StripView {
    /// The cursor's clock ("11:12 AM"; with seconds while a playback is where the cursor is) and how far behind live.
    pub when: String,
    pub rel: String,
    pub span_word: String,
    pub rate_word: String,
    pub window: Span,
    pub bar: Bar,
    pub axis: Vec<(i64, String)>,
    /// The cursor's moment, the knob's place.
    pub cursor: i64,
    /// The picture is live: `Live` is selected.
    pub live: bool,
    /// The control says Play (a recording is held) rather than Pause.
    pub play: bool,
    pub focus: StripItem,
    pub bubble: Option<BubbleView>,
    /// A previous and a next event exist to jump to.
    pub can_prev: bool,
    pub can_next: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Events,
    Clips,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetFocus {
    Tab(Tab),
    Card(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardView {
    pub title: String,
    pub line2: String,
    /// The kind's Lucide glyph: the badge on a picture, the glyph on a quiet tile.
    pub icon: &'static str,
    /// A clip's length, "0:30".
    pub length: Option<String>,
    /// The key of its picture, if it has a source for one; the card is the glyph on a quiet tile until the picture is in the store.
    pub pic: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetView {
    /// The camera's name, or "Recent events".
    pub title: String,
    /// `Events` and `Clips`; none in the recent list.
    pub tabs: Option<Tab>,
    pub cards: Vec<CardView>,
    /// The first shown card; four show.
    pub first: usize,
    /// A `chevron-right` says there are more.
    pub more: bool,
    pub focus: SheetFocus,
    /// Nothing to list: the words that say so (the list has been read).
    pub empty: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionRow {
    pub kind: &'static str,
    pub icon: &'static str,
    pub name: &'static str,
    pub on: bool,
}

/// The per-camera pop-up options page: a switch for each kind that may pop up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionsView {
    pub title: String,
    pub group: &'static str,
    pub back: &'static str,
    pub rows: Vec<OptionRow>,
    pub focus: usize,
}
