//! The library (`panels/AppsGrid.qml`'s logic; `docs/mocks/12-all-apps`): a rail down the left (All apps, a section per kind of app,
//! On the dock, Hidden) and a grid of icons beside it with the name under each. The apps are what the TV can launch, in dock order then
//! by name, with the person's hidden and renamed apps laid over; sections from `theme.apps.categories` first, the word lists for what the
//! theme does not name, "Other" for the rest. The D-pad moves by the focus engine's rules; the rail changes the grid as the focus moves
//! over it; OK opens, a hold asks for the options, the Search button asks for search, Back closes. Three rows are in view; the grid
//! scrolls a row when the focus passes the last.

use super::dock::Dock;
use crate::kit::focus::{Dir, Focus, Id, Moved, Region};
use serde_json::Value;
use ui::geometry::Frame;

pub const CELL_ID: u32 = 400;
pub const RAIL_ID: u32 = 500;
pub const SEARCH_ID: u32 = 550;
pub const CELLS: Region = Region(4);
pub const RAIL: Region = Region(5);
pub const TOP: Region = Region(6);
pub const COLS: usize = 5;
pub const VISIBLE_ROWS: usize = 3;
pub const ROW_PITCH: f32 = 237.0;
pub const COL_PITCH: f32 = 262.0;
pub const CELL_W: f32 = 242.0;
pub const CELL_H: f32 = 221.0;
pub const RAIL_FRAME: Frame = Frame::new(64.0, 136.0, 400.0, 888.0);
pub const PAGE_FRAME: Frame = Frame::new(488.0, 136.0, 1368.0, 888.0);
/// The grid's box in the page: under the heading row.
pub const GRID_X: f32 = PAGE_FRAME.x + 40.0;
pub const MAX_CELLS: u32 = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub params: Value,
    pub hidden: bool,
    pub on_dock: bool,
    pub plate: String,
    pub unmovable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub sub: String,
    pub icon: &'static str,
}

pub enum AppsOut {
    Nothing,
    Close,
    Launch(App),
    Search,
    Menu(Value),
}

#[derive(Default)]
pub struct Apps {
    apps: Vec<App>,
    categories: Value,
    /// The person's text size: the rail's rows and the Search button grow with it.
    pub scale: f32,
    pub section: String,
    pub rail_index: usize,
    pub cell: usize,
    pub scroll_row: usize,
    pub in_grid: bool,
    pub search_focused: bool,
}

const STREAMING: &[&str] = &[
    "netflix", "youtube", "plex", "plx", "apple tv", "appletv", "crunchyroll", "lg movies", "live tv", "livetv", "disney", "hulu", "prime", "amazon", "hbo", "max", "paramount",
    "peacock", "spotify", "tubi", "pluto", "twitch", "vudu", "sling", "philo",
];
const GAMING: &[&str] = &["geforce", "game", "xbox", "stadia", "luna", "boosteroid", "pool", "steam", "moonlight", "punktfunk"];
const UTILITIES: &[&str] = &[
    "browser", "hue", "homebrew", "hyperhdr", "settings", "file", "usb", "cast", "share", "miracast", "airplay", "photo", "music", "input", "guide", "support", "manual",
    "calculator", "store", "update", "bridge", "hub",
];

fn matches(title: &str, words: &[&str]) -> bool {
    let t = title.to_ascii_lowercase();
    words.iter().any(|w| t.contains(w))
}

impl Apps {
    /// The apps from the dock's own list (every launch point, before the person's hiding) and the theme.
    pub fn set_points(&mut self, dock: &Dock, theme: &Value) {
        self.categories = theme.pointer("/apps/categories").cloned().unwrap_or(Value::Null);
        let hidden: Vec<&str> = theme.pointer("/apps/hidden").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let on_dock = dock.ids();
        let mut apps: Vec<App> = dock
            .all_points(theme)
            .into_iter()
            .map(|p| App {
                hidden: hidden.contains(&p.id.as_str()),
                on_dock: on_dock.contains(&p.id),
                id: p.id,
                title: p.title,
                icon: p.icon,
                params: p.params,
                plate: p.plate,
                unmovable: p.unmovable,
            })
            .collect();
        // dock order first, then by name
        apps.sort_by(|a, b| {
            let ra = on_dock.iter().position(|d| *d == a.id).unwrap_or(usize::MAX);
            let rb = on_dock.iter().position(|d| *d == b.id).unwrap_or(usize::MAX);
            ra.cmp(&rb).then_with(|| a.title.to_ascii_lowercase().cmp(&b.title.to_ascii_lowercase()))
        });
        self.apps = apps;
        self.clamp();
    }

    pub fn all(&self) -> &[App] {
        &self.apps
    }

    pub fn by_id(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }

    fn category_of(&self, a: &App) -> &'static str {
        if let Some(c) = self.categories.get(&a.id).and_then(Value::as_str) {
            return match c {
                "streaming" => "streaming",
                "gaming" => "gaming",
                "utilities" => "utilities",
                _ => "other",
            };
        }
        if matches(&a.title, STREAMING) {
            "streaming"
        } else if matches(&a.title, GAMING) {
            "gaming"
        } else if matches(&a.title, UTILITIES) {
            "utilities"
        } else {
            "other"
        }
    }

    fn shown(&self) -> Vec<&App> {
        self.apps.iter().filter(|a| !a.hidden).collect()
    }

    /// The rail: only the sections that have apps in them, All apps first and Hidden (when there are any) last.
    pub fn rail(&self) -> Vec<Section> {
        let shown = self.shown();
        let mut out = vec![Section { id: "all".into(), title: "All apps".into(), sub: format!("{} installed", shown.len()), icon: "layout-grid" }];
        for (id, title, icon) in [("streaming", "Streaming", "clapperboard"), ("gaming", "Gaming", "gamepad-2"), ("utilities", "Utilities", "wrench"), ("other", "Other", "ellipsis")] {
            let n = shown.iter().filter(|a| self.category_of(a) == id).count();
            if n > 0 {
                out.push(Section { id: id.into(), title: title.into(), sub: format!("{n} app{}", if n == 1 { "" } else { "s" }), icon });
            }
        }
        let dock = shown.iter().filter(|a| a.on_dock).count();
        out.push(Section { id: "dock".into(), title: "On the dock".into(), sub: format!("{dock} tiles \u{b7} reorder here"), icon: "star" });
        let hidden = self.apps.iter().filter(|a| a.hidden).count();
        if hidden > 0 {
            out.push(Section { id: "hidden".into(), title: "Hidden".into(), sub: format!("{hidden} app{} \u{b7} restore any time", if hidden == 1 { "" } else { "s" }), icon: "eye-off" });
        }
        out
    }

    /// The apps of the section.
    pub fn list(&self) -> Vec<&App> {
        match self.section.as_str() {
            "all" | "" => self.shown(),
            "dock" => self.shown().into_iter().filter(|a| a.on_dock).collect(),
            "hidden" => self.apps.iter().filter(|a| a.hidden).collect(),
            s => self.shown().into_iter().filter(|a| self.category_of(a) == s).collect(),
        }
    }

    pub fn heading(&self) -> Section {
        let rail = self.rail();
        rail.iter().find(|s| s.id == self.section).cloned().unwrap_or_else(|| rail[0].clone())
    }

    pub fn sub_of(section: &str) -> &'static str {
        match section {
            "all" => "Everything on this TV",
            "streaming" => "Films, series and live TV",
            "gaming" => "Cloud gaming and games",
            "utilities" => "Tools and connections",
            "other" => "Everything else",
            "dock" => "In the order of the dock",
            "hidden" => "Kept out of the way, not removed",
            _ => "",
        }
    }

    pub fn open(&mut self) {
        self.section = "all".into();
        self.rail_index = 0;
        self.cell = 0;
        self.scroll_row = 0;
        self.in_grid = true;
        self.search_focused = false;
    }

    fn clamp(&mut self) {
        let n = self.list().len();
        if self.cell >= n {
            self.cell = n.saturating_sub(1);
        }
        let rail = self.rail();
        if self.rail_index >= rail.len() {
            self.rail_index = rail.len() - 1;
            self.section = rail[self.rail_index].id.clone();
        }
        self.scroll_row = self.scroll_row.min(n.saturating_sub(1) / COLS);
    }

    pub fn focused_app(&self) -> Option<Value> {
        if !self.in_grid || self.search_focused {
            return None;
        }
        self.list().get(self.cell).map(|a| app_json(a))
    }

    pub fn cell_frame(&self, i: usize) -> Frame {
        Frame::new(GRID_X + (i % COLS) as f32 * COL_PITCH, self.grid_y() + (i / COLS) as f32 * ROW_PITCH - self.scroll_row as f32 * ROW_PITCH, CELL_W, CELL_H)
    }

    fn k(&self) -> f32 {
        self.scale.max(1.0)
    }

    /// A rail row: 80 tall at a pitch of 84 at 100 % text, both growing with the text size.
    pub fn rail_frame(&self, i: usize, hidden: bool) -> Frame {
        let (h, pitch) = (80.0 * self.k(), 84.0 * self.k());
        let y = if hidden { RAIL_FRAME.y + RAIL_FRAME.h - 20.0 - h } else { RAIL_FRAME.y + 20.0 + i as f32 * pitch };
        Frame::new(RAIL_FRAME.x + 16.0, y, RAIL_FRAME.w - 32.0, h)
    }

    pub fn search_frame(&self) -> Frame {
        let (w, h) = (260.0 * self.k(), 56.0 * self.k());
        Frame::new(PAGE_FRAME.x + PAGE_FRAME.w - 40.0 - w, PAGE_FRAME.y + 28.0, w, h)
    }

    /// The grid's top: under the heading row, which grows with the text size.
    pub fn grid_y(&self) -> f32 {
        PAGE_FRAME.y + 28.0 + 56.0 * self.k() + 22.0
    }

    /// Everything as focusables: the rail's rows, the grid's cells where they are now, the Search button.
    pub fn register(&self, focus: &mut Focus) {
        let scope = super::model::SCOPE_APPS;
        let rail = self.rail();
        for i in 0..12u32 {
            let id = Id(RAIL_ID + i);
            match rail.get(i as usize) {
                Some(s) => focus.register(id, self.rail_frame(i as usize, s.id == "hidden"), RAIL, scope),
                None => focus.unregister(id),
            }
        }
        let n = self.list().len();
        for i in 0..MAX_CELLS {
            let id = Id(CELL_ID + i);
            if (i as usize) < n {
                focus.register(id, self.cell_frame(i as usize), CELLS, scope);
            } else {
                focus.unregister(id);
            }
        }
        focus.register(Id(SEARCH_ID), self.search_frame(), TOP, scope);
        // Up from the top row is the Search button, as the QML had it, not the rail beside it
        for i in 0..COLS.min(n) {
            focus.guide(Id(CELL_ID + i as u32), Dir::Up, Id(SEARCH_ID));
        }
        let current = if self.search_focused {
            Id(SEARCH_ID)
        } else if self.in_grid && n > 0 {
            Id(CELL_ID + self.cell as u32)
        } else {
            Id(RAIL_ID + self.rail_index as u32)
        };
        focus.focus(current);
    }

    pub fn unregister(&self, focus: &mut Focus) {
        for i in 0..12 {
            focus.unregister(Id(RAIL_ID + i));
        }
        for i in 0..MAX_CELLS {
            focus.unregister(Id(CELL_ID + i));
        }
        focus.unregister(Id(SEARCH_ID));
    }

    fn show_cell(&mut self) {
        let row = self.cell / COLS;
        if row < self.scroll_row {
            self.scroll_row = row;
        } else if row + 1 > self.scroll_row + VISIBLE_ROWS {
            self.scroll_row = row + 1 - VISIBLE_ROWS;
        }
    }

    pub fn set_section(&mut self, i: usize) {
        let rail = self.rail();
        let i = i.min(rail.len() - 1);
        self.rail_index = i;
        self.section = rail[i].id.clone();
        self.cell = 0;
        self.scroll_row = 0;
    }

    /// Takes what the focus engine says: a cell, a rail row or the Search button.
    fn take(&mut self, id: Id) {
        if let Some(i) = id.0.checked_sub(CELL_ID).filter(|i| *i < MAX_CELLS) {
            self.in_grid = true;
            self.search_focused = false;
            self.cell = i as usize;
            self.show_cell();
        } else if let Some(i) = id.0.checked_sub(RAIL_ID).filter(|i| *i < 12) {
            self.in_grid = false;
            self.search_focused = false;
            self.set_section(i as usize);
        } else if id.0 == SEARCH_ID {
            self.search_focused = true;
            self.in_grid = false;
        }
    }

    pub fn key(&mut self, name: &str, dir: Option<Dir>, focus: &mut Focus) -> AppsOut {
        if name == "back" {
            return AppsOut::Close;
        }
        if name == "ok" {
            if self.search_focused {
                return AppsOut::Search;
            }
            if !self.in_grid {
                if !self.list().is_empty() {
                    self.in_grid = true;
                    self.register(focus);
                }
                return AppsOut::Nothing;
            }
            return self.list().get(self.cell).map_or(AppsOut::Nothing, |a| AppsOut::Launch((*a).clone()));
        }
        if name == "menu" {
            return self.focused_app().map_or(AppsOut::Nothing, AppsOut::Menu);
        }
        let Some(d) = dir else { return AppsOut::Nothing };
        if let Moved::To(_, to) = focus.step(d) {
            self.take(to);
            self.register(focus);
        }
        AppsOut::Nothing
    }

    /// A click on a focusable of the library.
    pub fn clicked(&mut self, id: Id, focus: &mut Focus) -> AppsOut {
        self.take(id);
        self.register(focus);
        if id.0 == SEARCH_ID {
            return AppsOut::Search;
        }
        if id.0 >= CELL_ID && id.0 < CELL_ID + MAX_CELLS {
            return self.list().get(self.cell).map_or(AppsOut::Nothing, |a| AppsOut::Launch((*a).clone()));
        }
        AppsOut::Nothing
    }

    pub fn state(&self) -> Value {
        serde_json::json!({"section": self.section, "cell": self.cell, "scrollRow": self.scroll_row, "inGrid": self.in_grid, "search": self.search_focused, "count": self.list().len()})
    }
}

pub fn app_json(a: &App) -> Value {
    serde_json::json!({"id": a.id, "title": a.title, "iconUrl": a.icon, "params": a.params, "hidden": a.hidden, "onDock": a.on_dock, "unmovable": a.unmovable})
}
