//! A page of Settings as the rail and the model know it: who it is, where it hangs, how its rows are made from what the shell knows,
//! and what it reads when it is brought up. The QML pages were files of closures; these are functions from [`Ctx`] to rows.

use super::ctx::Ctx;
use super::row::{Op, Row};
use super::view::Figure;
use serde_json::Value;

/// What a page reads when it opens (`refresh()` of a QML page): LG's keys of a category, a luna call whose answer the page keeps by
/// name, a request to the helper, a helper owner.
#[derive(Clone, Debug, PartialEq)]
pub enum Read {
    Lg { category: &'static str, keys: Vec<&'static str>, app: Option<String> },
    Call { name: &'static str, uri: &'static str, payload: Value },
    /// A subscription: the answer is kept under `name` on every change until the page closes it.
    Watch { name: &'static str, uri: &'static str, payload: Value },
    Get { name: &'static str, path: &'static str },
    Owner(String),
    /// The sleep timer's time left (`timer/sleep/getRemainTime`).
    Sleep,
    /// The TV's launch points (for the names and icons of apps), subscribed once.
    Apps,
}

impl Read {
    pub fn lg(category: &'static str, keys: &[&'static str]) -> Self {
        Read::Lg { category, keys: keys.to_vec(), app: None }
    }

    pub fn lg_app(category: &'static str, keys: &[&'static str], app: &str) -> Self {
        Read::Lg { category, keys: keys.to_vec(), app: Some(app.to_string()) }
    }

    pub fn call(name: &'static str, uri: &'static str, payload: Value) -> Self {
        Read::Call { name, uri, payload }
    }

    pub fn owner(id: &str) -> Self {
        Read::Owner(id.to_string())
    }
}

pub type Rows = fn(&Ctx) -> Vec<Row>;
pub type Reads = fn(&Ctx) -> Vec<Read>;
pub type Words = fn(&Ctx) -> String;
pub type Chip = fn(&Ctx) -> Option<(String, &'static str)>;
/// The picture over a page's rows, when it has one (the saver in miniature).
pub type Figures = fn(&Ctx) -> Option<Figure>;
/// What a page does with the answer of one of its calls, after it is kept (`LunaCall.onAnswered`).
pub type Answered = fn(&Ctx, &str, &Value) -> Vec<Op>;
/// What a page does when it is brought up, besides reading (`pending = ""`).
pub type Opened = fn(&Ctx) -> Vec<Op>;

fn no_rows(_: &Ctx) -> Vec<Row> {
    Vec::new()
}

fn no_reads(_: &Ctx) -> Vec<Read> {
    Vec::new()
}

fn no_words(_: &Ctx) -> String {
    String::new()
}

fn no_chip(_: &Ctx) -> Option<(String, &'static str)> {
    None
}

/// A page.
pub struct Page {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    /// The rail section a child page belongs to (its row stays lit while the page is up).
    pub parent: Option<&'static str>,
    pub rail: bool,
    /// Whether Home's search lists the page (the chooser only means something when a row opened it).
    pub search: bool,
    pub tabs: &'static [&'static str],
    /// The page this one is a child of, as the back pill names it ("System").
    pub back: &'static str,
    pub rows: Rows,
    pub reads: Reads,
    /// The rail's second line.
    pub summary: Words,
    pub subtitle: Words,
    /// A title from what the page was opened for (the chooser's is the row's label).
    pub title_of: Option<Words>,
    pub chip: Chip,
    pub figure: Option<Figures>,
    pub answered: Option<Answered>,
    pub opened: Option<Opened>,
    /// Whether the rail shows the summary without the page having been opened (the rail's live line).
    pub warm: bool,
}

impl Page {
    pub fn new(id: &'static str, title: &'static str, icon: &'static str) -> Self {
        Page {
            id,
            title,
            icon,
            parent: None,
            rail: false,
            search: true,
            tabs: &[],
            back: "",
            rows: no_rows,
            reads: no_reads,
            summary: no_words,
            subtitle: no_words,
            title_of: None,
            chip: no_chip,
            figure: None,
            answered: None,
            opened: None,
            warm: false,
        }
    }

    pub fn rail(mut self) -> Self {
        self.rail = true;
        self
    }

    pub fn parent(mut self, parent: &'static str) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn hidden_from_search(mut self) -> Self {
        self.search = false;
        self
    }

    pub fn tabs(mut self, tabs: &'static [&'static str]) -> Self {
        self.tabs = tabs;
        self
    }

    pub fn back(mut self, back: &'static str) -> Self {
        self.back = back;
        self
    }

    pub fn rows(mut self, rows: Rows) -> Self {
        self.rows = rows;
        self
    }

    pub fn reads(mut self, reads: Reads) -> Self {
        self.reads = reads;
        self
    }

    pub fn summary(mut self, summary: Words) -> Self {
        self.summary = summary;
        self.warm = true;
        self
    }

    pub fn subtitle(mut self, subtitle: Words) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn title_of(mut self, title_of: Words) -> Self {
        self.title_of = Some(title_of);
        self
    }

    pub fn chip(mut self, chip: Chip) -> Self {
        self.chip = chip;
        self
    }

    pub fn figure(mut self, figure: Figures) -> Self {
        self.figure = Some(figure);
        self
    }

    pub fn answered(mut self, answered: Answered) -> Self {
        self.answered = Some(answered);
        self
    }

    pub fn opened(mut self, opened: Opened) -> Self {
        self.opened = Some(opened);
        self
    }

    pub fn tabbed(&self) -> bool {
        self.tabs.len() > 1
    }
}
