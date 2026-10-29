//! What the model gives the surface to draw, and the tests to read: plain data, nothing that holds a function.

use super::row::Row;
use crate::kit::hint::Hint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Rail,
    Page,
}

/// Where the page's own focus is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopView {
    /// Nothing is focused (an empty page, a page not yet placed).
    None,
    /// The tab bar (its current tab is the highlighted one).
    Tabs,
    /// The row of this index in [`PageView::rows`].
    Row(usize),
}

/// The bar over the panes: the way back, the title, and what is true now.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub title: String,
    /// "3:05 PM".
    pub clock: String,
    pub wired: bool,
    pub link: bool,
    pub pointer: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RailRow {
    pub id: String,
    pub title: String,
    pub icon: String,
    /// The one live line under the title.
    pub summary: String,
    /// The section whose page is in front (its tile in the accent).
    pub selected: bool,
    pub focused: bool,
    /// "More in LG settings": a label only.
    pub lg: bool,
}

/// The picture over a page's rows: the saver as it is drawn (`face`), what the focused row puts on it and how, and the key to the
/// answers' glyphs with the one in force lit.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Figure {
    /// "clock", "art" or "off".
    pub face: String,
    pub dim: bool,
    /// The glyph of what is shown and the answer's id ("over", "inside", "wake", "hide"), when a row is focused.
    pub shows: Option<(String, String)>,
    pub legend: Vec<Legend>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Legend {
    pub id: String,
    pub icon: String,
    pub word: String,
    pub lit: bool,
}

#[derive(Clone, Debug)]
pub struct PageView {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// The page this one was opened from, as the back pill names it.
    pub back: Option<String>,
    pub chip: Option<(String, &'static str)>,
    pub tabs: Vec<String>,
    pub tab: usize,
    /// The rows of the tab in force that are shown, heads and notes included, in order.
    pub rows: Vec<Row>,
    pub stop: StopView,
    /// The inner focus of a segmented row, a stepper or a row of cards.
    pub seg: usize,
    /// The page has the focus (the rail does not).
    pub active: bool,
    pub figure: Option<Figure>,
}

impl PageView {
    pub fn focused_row(&self) -> Option<&Row> {
        match self.stop {
            StopView::Row(i) => self.rows.get(i),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnswerView {
    pub label: String,
    pub danger: bool,
}

#[derive(Clone, Debug)]
pub enum SheetView {
    /// A question over the page: the safe answer first.
    Confirm { icon: String, title: String, body: String, answers: Vec<AnswerView>, index: usize },
    /// The explanation behind a row's info mark, about the row of this id.
    Help { row: String, title: String, text: String },
    /// Text from the person.
    Entry { prompt: String, text: String, secret: bool },
    /// The small picker by the row that asked: its options as rows (the one in force marked), and the focused one.
    Pick { title: String, rows: Vec<Row>, index: usize },
}

#[derive(Clone, Debug)]
pub struct View {
    pub open: bool,
    pub zone: Zone,
    pub bar: Bar,
    pub rail: Vec<RailRow>,
    pub page: Option<PageView>,
    pub sheet: Option<SheetView>,
    /// The keys' glyphs for what the focus is on now.
    pub hints: Vec<Hint>,
}
