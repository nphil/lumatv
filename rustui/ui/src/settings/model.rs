//! The Settings model: a pure state machine over what the shell knows (LG's settings, the helper's owners and topics, the answers of
//! the calls the pages made, the theme), the keys and the pointer. It decides which section's page is in front, where the focus is,
//! what a key changes and which calls that sets off; the surface draws [`View`]. The rules are `Settings.qml`'s and `PageList.qml`'s
//! (the QML tests of `tests/test_settings.py` are the cases the tests below restate): two zones (the rail and the page), Right or OK
//! enters the page, Left or Back returns to the rail, a child page returns to the page it was opened from, rows are skipped by the D-pad
//! when they are dimmed or only show, a held key runs a long slider faster, a held OK on a row with help opens the help and the
//! release does nothing more, any key closes the help sheet before anything else sees it.

use super::ctx::{Ctx, Who};
use super::page::{Page, Read};
use super::row::{Arg, Ask, ChoiceSpec, Kind, Op, PageArg, Row, ShellRequest};
use super::src::{Bag, Owner, Tags, Topics, ours};
use super::view::{AnswerView, Bar, PageView, RailRow, SheetView, StopView, View, Zone};
use crate::host::sio::Sio;
use crate::kit::hint::{Action, Hint, Key};
use crate::kit::screen::{Effect, Input, Model as ScreenModel};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// A held OK is a hold after this long (the theme's `motion.holdMs` is the host's to set).
pub const HOLD_MS: u64 = 700;
/// The clock in the bar moves on this often while Settings is up.
const CLOCK_MS: u64 = 15_000;
/// A page's reads are not asked again within this long of the last time (keys and sections are walked quickly).
const READ_AGAIN_MS: u64 = 1500;
/// LG's own Settings app.
pub const LG_APP: &str = "com.palm.app.settings";

/// A key of the remote, as Settings takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
}

impl K {
    pub fn parse(name: &str) -> Option<K> {
        Some(match name {
            "up" => K::Up,
            "down" => K::Down,
            "left" => K::Left,
            "right" => K::Right,
            "ok" => K::Ok,
            "back" => K::Back,
            _ => return None,
        })
    }
}

/// What the pointer clicked.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The way back in the bar.
    Back,
    /// A row of the rail (the last is LG's).
    Rail(usize),
    /// A tab of the page.
    Tab(usize),
    /// A row of the page, at a segment or card, or at a value of a slider.
    Row { id: String, seg: Option<usize>, value: Option<f64> },
    /// The info mark of a row.
    Info(String),
    /// An answer of the question over the page.
    Answer(usize),
    /// An option of the small picker.
    Pick(usize),
    /// Outside the sheet that is up.
    Outside,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stop {
    Unset,
    Tabs,
    Row(String),
}

#[derive(Clone, Debug)]
struct PageState {
    stop: Stop,
    tab: usize,
    seg: usize,
    slider_at: u64,
    slider_run: u32,
    slider_dir: i32,
    read_at: Option<u64>,
}

impl Default for PageState {
    fn default() -> Self {
        PageState { stop: Stop::Unset, tab: 0, seg: 0, slider_at: 0, slider_run: 0, slider_dir: 0, read_at: None }
    }
}

enum Route {
    Call { name: String, subscribe: bool },
    Act,
    Request { name: Option<String> },
    OwnerGet(String),
    OwnerPost(String),
    Theme,
}

enum Sheet {
    Confirm { ask: Ask, index: usize },
    Pick { spec: Rc<ChoiceSpec>, index: usize },
    Help { row: String, title: String, text: String },
    Entry { prompt: String, text: String, secret: bool, then: Rc<dyn Fn(String) -> Vec<Op>> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    Refresh,
    Hold,
    Clock,
}

/// The rows of the page in front as they were last made: the revision of the model, the page, the rows.
type RowCache = (u64, String, Rc<Vec<Row>>);

pub struct Model {
    pages: Vec<Page>,
    sections: Vec<usize>,
    pub lg: Sio,
    /// The sleep timer (the quick panel's own model, read and set through LG's service the same way).
    pub power: crate::host::power::Power,
    /// What the TV can launch: the names and icons of the apps a permission is for.
    pub apps: crate::host::apps::Apps,
    apps_asked: bool,
    pub owners: BTreeMap<String, Owner>,
    pub topics: Topics,
    pub docs: Bag,
    pub local: Bag,
    /// The merged theme as it is in force (the shipped one under the owner's edits).
    pub theme: Value,
    pub layout: Value,
    tags: Tags,
    routes: BTreeMap<u64, Route>,
    open: bool,
    zone: Zone,
    rail_index: usize,
    current: String,
    trail: Vec<String>,
    arg: PageArg,
    states: BTreeMap<String, PageState>,
    sheet: Option<Sheet>,
    ok_down: bool,
    ok_held: bool,
    timers: Vec<(u64, Timer)>,
    /// What waits for a quiet moment (`Op::Later`): when, the key, the operations.
    later: Vec<(u64, String, Vec<Op>)>,
    requests: Vec<ShellRequest>,
    theme_file: Option<Value>,
    hold_ms: u64,
    rev: u64,
    cache: RefCell<Option<RowCache>>,
    now: u64,
    /// The look in force is the light one.
    light: bool,
}

fn merge_section(into: &mut Value, patch: &Value) {
    crate::host::theme::merge_one_level(into, patch);
}

impl Model {
    pub fn new(pages: Vec<Page>, theme: Value, layout: Value) -> Self {
        let sections: Vec<usize> = pages.iter().enumerate().filter(|(_, p)| p.rail).map(|(i, _)| i).collect();
        let current = sections.first().map_or(String::new(), |i| pages[*i].id.to_string());
        Model {
            pages,
            sections,
            lg: Sio::new(super::src::TAG_BASE),
            power: crate::host::power::Power::new(),
            apps: crate::host::apps::Apps::new(super::src::APPS_TAG),
            apps_asked: false,
            owners: BTreeMap::new(),
            topics: Topics::default(),
            docs: Bag::default(),
            local: Bag::default(),
            theme,
            layout,
            tags: Tags::default(),
            routes: BTreeMap::new(),
            open: false,
            zone: Zone::Rail,
            rail_index: 0,
            current,
            trail: Vec::new(),
            arg: PageArg::None,
            states: BTreeMap::new(),
            sheet: None,
            ok_down: false,
            ok_held: false,
            timers: Vec::new(),
            later: Vec::new(),
            requests: Vec::new(),
            theme_file: None,
            hold_ms: HOLD_MS,
            rev: 0,
            cache: RefCell::new(None),
            now: 0,
            light: false,
        }
    }

    /// The look in force is the light one (the pages that say what "Auto" resolved to).
    pub fn set_light(&mut self, light: bool) {
        if self.light != light {
            self.light = light;
            self.bump();
        }
    }

    pub fn set_hold_ms(&mut self, ms: u64) {
        self.hold_ms = ms.max(100);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The rail's rows are the sections and the last is LG's.
    pub fn rail_count(&self) -> usize {
        self.sections.len() + 1
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn zone(&self) -> Zone {
        self.zone
    }

    pub fn rail_index(&self) -> usize {
        self.rail_index
    }

    pub fn trail(&self) -> &[String] {
        &self.trail
    }

    pub fn page_ids(&self) -> Vec<&'static str> {
        self.pages.iter().map(|p| p.id).collect()
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// The pages Home's search lists (`[{id, title, icon, sub}]`, the pages that are not hidden from it), each with the first part of its own summary
    /// in the second line ("Settings \u{b7} Connected").
    pub fn search_pages(&self) -> Vec<Value> {
        self.pages
            .iter()
            .filter(|p| p.search)
            .map(|p| {
                let summary = self.summary_of(p);
                let first = summary.split(" \u{b7} ").next().unwrap_or("");
                let sub = if first.is_empty() { "Settings".to_owned() } else { format!("Settings \u{b7} {first}") };
                json!({"id": p.id, "title": p.title, "icon": p.icon, "sub": sub})
            })
            .collect()
    }

    /// What the surface asks the shell for (closing, launching an app, the layout editor), once.
    pub fn take_requests(&mut self) -> Vec<ShellRequest> {
        std::mem::take(&mut self.requests)
    }

    /// What a row's press would have asked of the shell (the shell's tests raise the requests Settings makes).
    #[cfg(test)]
    pub fn raise(&mut self, r: ShellRequest) {
        self.requests.push(r);
    }

    /// The theme file as it was written after a patch (the shell makes it the look in force and tells [`Model::set_theme`]).
    pub fn take_theme_file(&mut self) -> Option<Value> {
        self.theme_file.take()
    }

    pub fn set_theme(&mut self, theme: Value) {
        self.apps.set_names(&theme);
        self.theme = theme;
        self.bump();
    }

    pub fn set_layout(&mut self, layout: Value) {
        self.layout = layout;
        self.bump();
    }

    fn bump(&mut self) {
        self.rev += 1;
    }

    fn page_idx(&self, id: &str) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    fn page(&self, id: &str) -> Option<&Page> {
        self.page_idx(id).map(|i| &self.pages[i])
    }

    fn ctx<'a>(&'a self, page_id: &'a str, page_title: &'a str, arg: &'a PageArg) -> Ctx<'a> {
        Ctx {
            lg: &self.lg,
            power: &self.power,
            apps: &self.apps,
            owners: &self.owners,
            topics: &self.topics,
            docs: &self.docs,
            local: &self.local,
            theme: &self.theme,
            layout: &self.layout,
            arg,
            page_id,
            page_title,
            now: self.now,
            light: self.light,
        }
    }

    /// The rows of the page in front (every tab), as its function evaluated them over what the shell knows, with the owner's locks
    /// applied: the rows of the household's settings are dimmed for a profile that may not change them.
    fn rows(&self) -> Rc<Vec<Row>> {
        if let Some((rev, id, rows)) = &*self.cache.borrow()
            && *rev == self.rev
            && *id == self.current
        {
            return rows.clone();
        }
        let rows = Rc::new(self.build_rows());
        *self.cache.borrow_mut() = Some((self.rev, self.current.clone(), rows.clone()));
        rows
    }

    fn build_rows(&self) -> Vec<Row> {
        let Some(page) = self.page(&self.current) else { return Vec::new() };
        let title = self.page_title(page);
        let ctx = self.ctx(page.id, &title, &self.arg);
        let mut rows = (page.rows)(&ctx);
        let who = ctx.who();
        if !who.can_admin {
            lock(&mut rows, &who, page.tabs.len().max(1));
        }
        rows
    }

    fn page_title(&self, page: &Page) -> String {
        match page.title_of {
            Some(f) => f(&self.ctx(page.id, page.title, &self.arg)),
            None => page.title.to_string(),
        }
    }

    fn state(&self, id: &str) -> PageState {
        self.states.get(id).cloned().unwrap_or_default()
    }

    fn set_state(&mut self, id: &str, st: PageState) {
        self.states.insert(id.to_string(), st);
        self.bump();
    }

    // ------------------------------------------------------------------------------------------------ opening and closing

    /// Settings opens: the rail's first section (or the one it was left on), or `page` when one is named (a tile of the quick panel, a
    /// card of Home). Reads what the rail's summaries and the page in front show.
    pub fn open(&mut self, page: Option<&str>, now: u64) -> Vec<Effect> {
        self.now = now;
        let mut out = Vec::new();
        self.open = true;
        self.zone = Zone::Rail;
        self.trail.clear();
        self.sheet = None;
        self.ok_down = false;
        self.ok_held = false;
        if self.rail_index < self.sections.len() {
            self.current = self.pages[self.sections[self.rail_index]].id.to_string();
            self.arg = PageArg::None;
        }
        // what is true now: the cable and the Wi-Fi, the pointer
        self.run_reads(
            vec![
                Read::Watch { name: "net", uri: "luna://com.webos.service.connectionmanager/getStatus", payload: json!({}) },
                Read::lg("general", &["remotePointer"]),
            ],
            now,
            &mut out,
        );
        self.arm(Timer::Clock, now + CLOCK_MS);
        if let Some(id) = page {
            self.go_to(id, now, &mut out);
        } else {
            self.read_page(&self.current.clone(), now, true, &mut out);
        }
        // the rail's summaries: each page that keeps a live line reads what it shows
        let ids: Vec<String> = self.sections.iter().map(|i| &self.pages[*i]).filter(|p| p.warm).map(|p| p.id.to_string()).collect();
        for id in ids {
            if id != self.current {
                self.read_page(&id, now, false, &mut out);
            }
        }
        self.bump();
        out.push(Effect::Redraw);
        out
    }

    /// Settings closes: the sheets go, the subscriptions are let go.
    pub fn close(&mut self, now: u64) -> Vec<Effect> {
        self.now = now;
        let mut out = Vec::new();
        self.leave(now, &mut out);
        self.open = false;
        self.sheet = None;
        self.timers.clear();
        let subscribed: Vec<u64> = self.routes.iter().filter(|(_, r)| matches!(r, Route::Call { subscribe: true, .. })).map(|(t, _)| *t).collect();
        for tag in subscribed {
            self.routes.remove(&tag);
            out.push(Effect::Cancel { tag });
        }
        self.bump();
        out.push(Effect::Redraw);
        out
    }

    /// The page in front is left: what it said to do then (stop a search, let go of a subscription) is done.
    fn leave(&mut self, now: u64, out: &mut Vec<Effect>) {
        let Some((id, title, f)) = self.page(&self.current).and_then(|p| p.closed.map(|f| (p.id, p.title, f))) else { return };
        let ops = f(&self.ctx(id, title, &self.arg));
        self.apply(ops, now, out);
    }

    /// Comes to a page from outside (a tile of the quick panel, a card of Home): its section is the one on the rail and the page has
    /// the focus. An id this Settings has no page for leaves it where it was.
    fn go_to(&mut self, id: &str, now: u64, out: &mut Vec<Effect>) {
        let Some(page) = self.page(id) else { return };
        let parent = page.parent.unwrap_or(page.id);
        if let Some(i) = self.sections.iter().position(|s| self.pages[*s].id == parent) {
            self.rail_index = i;
        }
        if self.current != id {
            self.leave(now, out);
        }
        self.trail.clear();
        self.current = id.to_string();
        self.arg = PageArg::None;
        self.zone = Zone::Page;
        self.reset_page(now, out);
    }

    /// The page is brought up or its section entered: the focus is placed, the page reads what it shows (`reset()` and `opened()`).
    fn reset_page(&mut self, now: u64, out: &mut Vec<Effect>) {
        let id = self.current.clone();
        let rows = self.rows();
        let mut st = self.state(&id);
        let placed = match &st.stop {
            Stop::Row(r) => rows.iter().position(|x| x.id == *r).is_some_and(|i| rows[i].tab == st.tab && rows[i].focusable()),
            Stop::Tabs => true,
            Stop::Unset => false,
        };
        if !placed {
            match first(&rows, st.tab) {
                Some(i) => enter(&mut st, &rows, i),
                None => st.stop = Stop::Unset,
            }
        }
        self.set_state(&id, st);
        self.read_page(&id, now, true, out);
        if let Some(f) = self.page(&id).and_then(|p| p.opened) {
            let title = self.page_title(self.page(&id).expect("checked"));
            let ops = f(&self.ctx(&id, &title, &self.arg));
            self.apply(ops, now, out);
        }
    }

    /// What a page reads when it is shown, unless it read a moment ago.
    fn read_page(&mut self, id: &str, now: u64, force: bool, out: &mut Vec<Effect>) {
        let Some(idx) = self.page_idx(id) else { return };
        let mut st = self.state(id);
        if !force && st.read_at.is_some_and(|t| now < t + READ_AGAIN_MS) {
            return;
        }
        st.read_at = Some(now);
        self.states.insert(id.to_string(), st);
        let reads = {
            let page = &self.pages[idx];
            let arg = if id == self.current { &self.arg } else { &PageArg::None };
            (page.reads)(&self.ctx(page.id, page.title, arg))
        };
        self.run_reads(reads, now, out);
    }

    fn run_reads(&mut self, reads: Vec<Read>, now: u64, out: &mut Vec<Effect>) {
        for r in reads {
            match r {
                Read::Lg { category, keys, app } => out.extend(self.lg.read(category, &keys, app.as_deref(), now)),
                Read::Sleep => out.extend(self.power.read()),
                Read::Apps => {
                    if !self.apps_asked {
                        self.apps_asked = true;
                        out.extend(self.apps.subscribe());
                    }
                }
                Read::Call { name, uri, payload } => {
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Call { name: name.to_string(), subscribe: false });
                    out.push(Effect::Luna { tag, uri: uri.to_string(), payload, subscribe: false });
                }
                Read::Watch { name, uri, payload } => {
                    if self.routes.values().any(|r| matches!(r, Route::Call { name: n, subscribe: true } if n == name)) {
                        continue;
                    }
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Call { name: name.to_string(), subscribe: true });
                    let mut payload = payload;
                    payload["subscribe"] = json!(true);
                    out.push(Effect::Luna { tag, uri: uri.to_string(), payload, subscribe: true });
                }
                Read::Get { name, path } => {
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Request { name: Some(name.to_string()) });
                    out.push(Effect::Request { tag, method: "GET".into(), path: path.to_string(), body: None });
                }
                Read::Owner(id) => {
                    let owner = self.owners.entry(id.clone()).or_insert_with(|| Owner::new(&id));
                    let path = owner.path();
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::OwnerGet(id.to_string()));
                    out.push(Effect::Request { tag, method: "GET".into(), path, body: None });
                }
            }
        }
    }

    fn arm(&mut self, t: Timer, at: u64) {
        self.timers.retain(|(_, x)| *x != t);
        self.timers.push((at, t));
    }

    // ------------------------------------------------------------------------------------------------------- the inputs

    /// The page hears that an owner's document was taken (`owner:<id>`) or a helper topic changed (`topic:<name>`); no document comes with it,
    /// the page reads the stores through its context.
    fn page_hook(&mut self, name: &str, now: u64, out: &mut Vec<Effect>) {
        let ops = {
            let page = self.page(&self.current);
            match page.and_then(|p| p.answered.map(|f| (p.id, p.title, f))) {
                Some((id, title, f)) => f(&self.ctx(id, title, &self.arg), name, &Value::Null),
                None => Vec::new(),
            }
        };
        self.apply(ops, now, out);
    }

    fn answered_call(&mut self, name: &str, payload: &Value, now: u64, out: &mut Vec<Effect>) {
        self.docs.set(name, payload.clone());
        let ops = {
            let page = self.page(&self.current);
            match page.and_then(|p| p.answered.map(|f| (p.id, p.title, f))) {
                Some((id, title, f)) => f(&self.ctx(id, title, &self.arg), name, payload),
                None => Vec::new(),
            }
        };
        self.apply(ops, now, out);
    }

    fn fetched(&mut self, tag: u64, status: u16, data: &Value, now: u64, out: &mut Vec<Effect>) {
        match self.routes.remove(&tag) {
            Some(Route::OwnerGet(id)) => {
                if let Some(o) = self.owners.get_mut(&id) {
                    o.fetched(status, data);
                }
                self.page_hook(&format!("owner:{id}"), now, out);
            }
            Some(Route::OwnerPost(id)) => {
                let Some(o) = self.owners.get_mut(&id) else { return };
                let refetch = o.changed(status, data);
                let next = o.next_change();
                let path = o.path();
                self.page_hook(&format!("owner:{id}"), now, out);
                if refetch {
                    self.run_reads(vec![Read::Owner(id.clone())], now, out);
                }
                if let Some(body) = next {
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::OwnerPost(id));
                    out.push(Effect::Request { tag, method: "POST".into(), path, body: Some(body) });
                }
            }
            Some(Route::Request { name: Some(name) }) => {
                self.docs.set(&format!("{name}.status"), json!(status));
                self.answered_call(&name, data, now, out);
            }
            Some(Route::Theme) if status == 200 => self.theme_file = Some(data.clone()),
            _ => {}
        }
        self.bump();
    }

    fn luna(&mut self, tag: u64, payload: &Value, now: u64, out: &mut Vec<Effect>) {
        if self.lg.owns(tag) {
            let before = self.lg.answers;
            out.extend(self.lg.update(&Input::Luna { tag, payload: payload.clone() }, now));
            if self.lg.answers != before {
                self.bump();
            }
            return;
        }
        match self.routes.get(&tag) {
            Some(Route::Call { name, subscribe }) => {
                let (name, subscribe) = (name.clone(), *subscribe);
                if !subscribe {
                    self.routes.remove(&tag);
                }
                self.answered_call(&name, payload, now, out);
                self.bump();
            }
            Some(Route::Act) => {
                self.routes.remove(&tag);
            }
            _ => {}
        }
    }

    fn timer(&mut self, now: u64, out: &mut Vec<Effect>) {
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later).into_iter().partition(|(at, _, _)| *at <= now);
        self.later = waiting;
        for (_, _, ops) in ready {
            self.apply(ops, now, out);
        }
        let due: Vec<Timer> = self.timers.iter().filter(|(at, _)| *at <= now).map(|(_, t)| *t).collect();
        self.timers.retain(|(at, _)| *at > now);
        for t in due {
            match t {
                Timer::Clock => {
                    if self.open {
                        self.arm(Timer::Clock, now + CLOCK_MS);
                        self.bump();
                    }
                }
                Timer::Refresh => {
                    let id = self.current.clone();
                    self.read_page(&id, now, true, out);
                }
                Timer::Hold => {
                    if self.ok_down && self.help_ready() {
                        self.ok_held = true;
                        self.open_help_for_stop();
                    }
                }
            }
        }
    }

    fn key(&mut self, name: &str, down: bool, repeat: bool, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        let Some(k) = K::parse(name) else { return out };
        if k == K::Ok {
            if down && !repeat {
                // any key closes the help sheet before anything else sees it
                if matches!(self.sheet, Some(Sheet::Help { .. })) {
                    self.sheet = None;
                    self.ok_held = true;
                    self.bump();
                    out.push(Effect::Redraw);
                    return out;
                }
                self.ok_down = true;
                self.ok_held = false;
                if self.help_ready() {
                    self.arm(Timer::Hold, now + self.hold_ms);
                }
            } else if !down {
                self.timers.retain(|(_, t)| *t != Timer::Hold);
                let held = std::mem::replace(&mut self.ok_held, false);
                if std::mem::replace(&mut self.ok_down, false) && !held {
                    let ops = self.press(K::Ok, now, &mut out);
                    self.apply(ops, now, &mut out);
                }
            }
            self.bump();
            out.push(Effect::Redraw);
            return out;
        }
        if !down || (repeat && k == K::Back) {
            return out;
        }
        self.ok_held = false;
        let ops = self.press(k, now, &mut out);
        self.apply(ops, now, &mut out);
        self.bump();
        out.push(Effect::Redraw);
        out
    }

    /// One key as `Settings.key(dir)` takes it. The operations a row's action asked for.
    fn press(&mut self, k: K, now: u64, out: &mut Vec<Effect>) -> Vec<Op> {
        match &mut self.sheet {
            Some(Sheet::Help { .. }) => {
                self.sheet = None;
                return Vec::new();
            }
            Some(Sheet::Confirm { ask, index }) => {
                let n = ask.answers.len();
                let pick = match k {
                    K::Left => {
                        *index = index.saturating_sub(1);
                        None
                    }
                    K::Right => {
                        *index = (*index + 1).min(n.saturating_sub(1));
                        None
                    }
                    K::Ok => Some(*index),
                    K::Back => Some(ask.focus.min(n.saturating_sub(1))),
                    _ => None,
                };
                return match pick {
                    Some(i) => self.answer(i),
                    None => Vec::new(),
                };
            }
            Some(Sheet::Pick { spec, index }) => {
                let n = spec.options.len();
                let step = |from: usize, dir: i64| -> usize {
                    let mut i = from as i64 + dir;
                    while i >= 0 && (i as usize) < n {
                        if spec.options[i as usize].enabled {
                            return i as usize;
                        }
                        i += dir;
                    }
                    from
                };
                match k {
                    // the card is a column and the strip a row: either pair of keys walks the options
                    K::Up | K::Left => *index = step(*index, -1),
                    K::Down | K::Right => *index = step(*index, 1),
                    K::Ok => {
                        let i = *index;
                        return self.pick(i);
                    }
                    K::Back => self.sheet = None,
                }
                return Vec::new();
            }
            Some(Sheet::Entry { .. }) => {
                if k == K::Back {
                    self.sheet = None;
                }
                return Vec::new();
            }
            None => {}
        }
        match self.zone {
            Zone::Rail => self.rail_key(k, now, out),
            Zone::Page => {
                if let Some(ops) = self.page_key(k, now) {
                    return ops;
                }
                let section = self.sections.get(self.rail_index).map(|i| self.pages[*i].id);
                if k == K::Back && section != Some(self.current.as_str()) {
                    return self.up_ops();
                }
                if matches!(k, K::Left | K::Back) {
                    self.zone = Zone::Rail;
                }
                Vec::new()
            }
        }
    }

    /// The option `i` of the small picker is chosen: it is written and the picker closes.
    fn pick(&mut self, i: usize) -> Vec<Op> {
        let Some(Sheet::Pick { spec, .. }) = self.sheet.take() else { return Vec::new() };
        match spec.options.get(i).filter(|o| o.enabled) {
            Some(o) => (spec.write)(&o.value),
            None => Vec::new(),
        }
    }

    fn answer(&mut self, i: usize) -> Vec<Op> {
        let Some(Sheet::Confirm { ask, .. }) = self.sheet.take() else { return Vec::new() };
        ask.answers.get(i).map(|a| a.then.clone()).unwrap_or_default()
    }

    fn up_ops(&mut self) -> Vec<Op> {
        let target = match self.trail.last() {
            Some(t) => t.clone(),
            None => self.sections.get(self.rail_index).map(|i| self.pages[*i].id.to_string()).unwrap_or_default(),
        };
        vec![Op::Open { page: target, arg: PageArg::None }]
    }

    fn rail_key(&mut self, k: K, now: u64, out: &mut Vec<Effect>) -> Vec<Op> {
        let last = self.rail_count() - 1;
        match k {
            K::Up => self.rail_to(self.rail_index.saturating_sub(1), now, out),
            K::Down => self.rail_to((self.rail_index + 1).min(last), now, out),
            K::Back => return vec![Op::Leave],
            K::Ok if self.rail_index >= self.sections.len() => return vec![Op::Launch { app: LG_APP.into(), params: json!({}) }],
            K::Ok | K::Right => {
                if self.rail_index < self.sections.len() {
                    self.zone = Zone::Page;
                    self.reset_page(now, out);
                }
            }
            K::Left => {}
        }
        Vec::new()
    }

    /// The rail's focus moved: the page of that section is in front, opened pages are closed, and the page reads what it shows.
    fn rail_to(&mut self, i: usize, now: u64, out: &mut Vec<Effect>) {
        if i == self.rail_index {
            return;
        }
        self.rail_index = i;
        self.trail.clear();
        if i < self.sections.len() {
            self.leave(now, out);
            self.current = self.pages[self.sections[i]].id.to_string();
            self.arg = PageArg::None;
            let id = self.current.clone();
            self.read_page(&id, now, false, out);
        }
        self.bump();
    }

    /// `PageList.key(dir)`: the page's own keys. `None` for the ones it does not use (the host then moves the focus to the rail).
    fn page_key(&mut self, k: K, now: u64) -> Option<Vec<Op>> {
        let id = self.current.clone();
        let (tabbed, ntabs) = self.page(&id).map(|p| (p.tabbed(), p.tabs.len()))?;
        let rows = self.rows();
        let mut st = self.state(&id);
        if st.stop == Stop::Unset
            && let Some(f) = first(&rows, st.tab)
        {
            enter(&mut st, &rows, f);
        }
        let result = page_press(&mut st, &rows, tabbed, ntabs, k, now);
        self.set_state(&id, st);
        result
    }

    // ---------------------------------------------------------------------------------------------------------- the pointer

    /// A click: it focuses what is under it and does what OK would (hover is only a cue, the surface's).
    pub fn click(&mut self, target: Target, now: u64) -> Vec<Effect> {
        self.now = now;
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        match (&self.sheet, &target) {
            (Some(Sheet::Confirm { .. }), Target::Answer(i)) => {
                if let Some(Sheet::Confirm { index, .. }) = &mut self.sheet {
                    *index = *i;
                }
                let ops = self.press(K::Ok, now, &mut out);
                self.apply(ops, now, &mut out);
            }
            (Some(Sheet::Confirm { .. }), Target::Outside) => {
                let ops = self.press(K::Back, now, &mut out);
                self.apply(ops, now, &mut out);
            }
            (Some(Sheet::Confirm { .. }), _) => {}
            (Some(Sheet::Pick { .. }), Target::Pick(i)) => {
                let ops = self.pick(*i);
                self.apply(ops, now, &mut out);
            }
            (Some(Sheet::Pick { .. }), Target::Outside) => self.sheet = None,
            (Some(Sheet::Pick { .. }), _) => {}
            (Some(Sheet::Help { .. }), _) => self.sheet = None,
            (Some(Sheet::Entry { .. }), Target::Outside) => self.sheet = None,
            (Some(Sheet::Entry { .. }), _) => {}
            (None, _) => {
                let ops = self.point(target, now, &mut out);
                self.apply(ops, now, &mut out);
            }
        }
        self.bump();
        out.push(Effect::Redraw);
        out
    }

    fn point(&mut self, target: Target, now: u64, out: &mut Vec<Effect>) -> Vec<Op> {
        match target {
            Target::Back => self.press(K::Back, now, out),
            Target::Rail(i) => {
                if i >= self.rail_count() {
                    return Vec::new();
                }
                self.rail_to(i, now, out);
                self.zone = Zone::Rail;
                if i >= self.sections.len() {
                    return vec![Op::Launch { app: LG_APP.into(), params: json!({}) }];
                }
                self.zone = Zone::Page;
                self.reset_page(now, out);
                Vec::new()
            }
            Target::Tab(i) => {
                let id = self.current.clone();
                let ntabs = self.page(&id).map_or(0, |p| p.tabs.len());
                if i >= ntabs {
                    return Vec::new();
                }
                self.zone = Zone::Page;
                let mut st = self.state(&id);
                st.stop = Stop::Tabs;
                st.tab = i;
                self.set_state(&id, st);
                Vec::new()
            }
            Target::Row { id: rid, seg, value } => {
                let id = self.current.clone();
                let rows = self.rows();
                let Some(i) = rows.iter().position(|r| r.id == rid) else { return Vec::new() };
                let mut st = self.state(&id);
                self.zone = Zone::Page;
                if !(rows[i].tab == st.tab && rows[i].focusable()) {
                    self.set_state(&id, st);
                    return Vec::new();
                }
                enter(&mut st, &rows, i);
                if let Some(s) = seg {
                    st.seg = s.min(rows[i].count().saturating_sub(1));
                }
                let ops = match (rows[i].kind, value) {
                    (Kind::Slider, Some(v)) => call(&rows[i], Arg::Value(v.clamp(rows[i].min, rows[i].max))),
                    (Kind::Slider | Kind::Nudge | Kind::Stepper, _) => Vec::new(),
                    _ => activate(&st, &rows[i]),
                };
                self.set_state(&id, st);
                ops
            }
            Target::Info(rid) => {
                let id = self.current.clone();
                let rows = self.rows();
                if let Some(i) = rows.iter().position(|r| r.id == rid) {
                    let mut st = self.state(&id);
                    self.zone = Zone::Page;
                    if rows[i].focusable() {
                        enter(&mut st, &rows, i);
                        self.set_state(&id, st);
                    }
                    self.open_help(&rid);
                }
                Vec::new()
            }
            Target::Answer(_) | Target::Pick(_) | Target::Outside => Vec::new(),
        }
    }

    // ------------------------------------------------------------------------------------------------------------ the help

    fn help_ready(&self) -> bool {
        if self.zone != Zone::Page || self.sheet.is_some() {
            return false;
        }
        let rows = self.rows();
        let st = self.state(&self.current);
        let Stop::Row(rid) = &st.stop else { return false };
        rows.iter().any(|r| r.id == *rid && r.tab == st.tab && r.focusable() && !r.help.is_empty())
    }

    fn open_help_for_stop(&mut self) {
        let st = self.state(&self.current);
        if let Stop::Row(rid) = st.stop {
            self.open_help(&rid);
        }
    }

    fn open_help(&mut self, rid: &str) {
        let rows = self.rows();
        if let Some(r) = rows.iter().find(|r| r.id == rid).filter(|r| !r.help.is_empty()) {
            self.sheet = Some(Sheet::Help { row: r.id.clone(), title: r.label.clone(), text: r.help.clone() });
            self.bump();
        }
    }

    // ------------------------------------------------------------------------------------------------------- what rows do

    fn apply(&mut self, ops: Vec<Op>, now: u64, out: &mut Vec<Effect>) {
        for op in ops {
            match op {
                Op::Lg { ds, value, app } => out.extend(self.lg.write(&ds, &value, app.as_deref())),
                Op::LgMany { category, settings, app } => out.extend(self.lg.write_many(&category, &settings, app.as_deref())),
                Op::Act { uri, payload } => {
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Act);
                    out.push(Effect::Luna { tag, uri, payload, subscribe: false });
                }
                Op::Call { name, uri, payload, subscribe } => {
                    if subscribe {
                        self.close_call(&name, out);
                    }
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Call { name, subscribe });
                    let mut payload = payload;
                    if subscribe {
                        payload["subscribe"] = json!(true);
                    }
                    out.push(Effect::Luna { tag, uri, payload, subscribe });
                }
                Op::Close(name) => self.close_call(&name, out),
                Op::Send { name, method, path, body } => {
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Request { name });
                    out.push(Effect::Request { tag, method: method.into(), path, body });
                }
                Op::Owner { owner, key, value } => {
                    let o = self.owners.entry(owner.clone()).or_insert_with(|| Owner::new(&owner));
                    let path = o.path();
                    if let Some(body) = o.write(&key, &value) {
                        let tag = self.tags.take();
                        self.routes.insert(tag, Route::OwnerPost(owner));
                        out.push(Effect::Request { tag, method: "POST".into(), path, body: Some(body) });
                    }
                }
                Op::Patch(patch) => {
                    merge_section(&mut self.theme, &patch);
                    let tag = self.tags.take();
                    self.routes.insert(tag, Route::Theme);
                    out.push(Effect::Theme { tag, patch });
                }
                Op::Set(key, v) => self.local.set(&key, v),
                Op::Open { page, arg } => self.open_page(&page, arg, now, out),
                Op::Up => {
                    let ops = self.up_ops();
                    self.apply(ops, now, out);
                }
                Op::Ask(ask) => {
                    let index = ask.focus.min(ask.answers.len().saturating_sub(1));
                    self.sheet = Some(Sheet::Confirm { ask, index });
                }
                Op::Pick(spec) => {
                    let index = spec.options.iter().position(|o| o.value == spec.current && o.enabled).or_else(|| spec.options.iter().position(|o| o.enabled)).unwrap_or(0);
                    self.sheet = Some(Sheet::Pick { spec, index });
                }
                Op::Entry { prompt, text, secret, then } => {
                    self.requests.push(ShellRequest::Text { prompt: prompt.clone(), text: text.clone(), secret });
                    self.sheet = Some(Sheet::Entry { prompt, text, secret, then });
                }
                Op::Launch { app, params } => {
                    self.requests.push(ShellRequest::Launch { app, params });
                    self.requests.push(ShellRequest::Close);
                }
                Op::Shell(r) => self.requests.push(r),
                Op::Refresh => {
                    let id = self.current.clone();
                    self.read_page(&id, now, true, out);
                }
                Op::RefreshIn(ms) => self.arm(Timer::Refresh, now + ms),
                Op::Focus(id) => {
                    let (current, rows) = (self.current.clone(), self.rows());
                    let mut st = self.state(&current);
                    if let Some(i) = rows.iter().position(|r| r.id == id && r.tab == st.tab && r.focusable()) {
                        enter(&mut st, &rows, i);
                        self.set_state(&current, st);
                    }
                }
                Op::Sleep(i) => {
                    out.extend(self.power.set(i, now));
                    self.bump();
                }
                Op::Later { key, ms, then } => {
                    self.later.retain(|(_, k, _)| *k != key);
                    self.later.push((now + ms, key, then));
                }
                Op::Leave => self.requests.push(ShellRequest::Close),
            }
        }
        self.bump();
    }

    fn close_call(&mut self, name: &str, out: &mut Vec<Effect>) {
        let tags: Vec<u64> = self.routes.iter().filter(|(_, r)| matches!(r, Route::Call { name: n, subscribe: true } if n == name)).map(|(t, _)| *t).collect();
        for tag in tags {
            self.routes.remove(&tag);
            out.push(Effect::Cancel { tag });
        }
    }

    /// `host.openPage(id, arg)`: the page the owner was on is remembered so Back comes back to it, whatever the depth.
    fn open_page(&mut self, id: &str, arg: PageArg, now: u64, out: &mut Vec<Effect>) {
        if self.page_idx(id).is_none() {
            return;
        }
        if let Some(at) = self.trail.iter().position(|t| t == id) {
            self.trail.truncate(at);
        } else if self.current != id {
            self.trail.push(self.current.clone());
        }
        if self.current != id {
            self.leave(now, out);
        }
        self.current = id.to_string();
        self.arg = arg;
        self.zone = Zone::Page;
        self.reset_page(now, out);
    }

    /// The text the person typed for an [`Op::Entry`].
    pub fn entered(&mut self, text: &str, now: u64) -> Vec<Effect> {
        self.now = now;
        let mut out = Vec::new();
        if let Some(Sheet::Entry { then, .. }) = self.sheet.take() {
            let ops = then(text.to_string());
            self.apply(ops, now, &mut out);
        }
        self.bump();
        out.push(Effect::Redraw);
        out
    }

    /// Back out of the keyboard: nothing is kept.
    pub fn entry_cancelled(&mut self) {
        if matches!(self.sheet, Some(Sheet::Entry { .. })) {
            self.sheet = None;
            self.bump();
        }
    }

    // ----------------------------------------------------------------------------------------------------------------- view

    fn summary_of(&self, page: &Page) -> String {
        (page.summary)(&self.ctx(page.id, page.title, &PageArg::None))
    }

    pub fn view(&self) -> View {
        let net = self.docs.get("net");
        let rail: Vec<RailRow> = self
            .sections
            .iter()
            .enumerate()
            .map(|(i, pi)| {
                let p = &self.pages[*pi];
                RailRow {
                    id: p.id.to_string(),
                    title: p.title.to_string(),
                    icon: p.icon.to_string(),
                    summary: self.summary_of(p),
                    selected: i == self.rail_index,
                    focused: self.zone == Zone::Rail && i == self.rail_index,
                    lg: false,
                }
            })
            .chain(std::iter::once(RailRow {
                id: "lg".into(),
                title: "More in LG settings".into(),
                icon: "square-arrow-out-up-right".into(),
                summary: String::new(),
                selected: false,
                focused: self.zone == Zone::Rail && self.rail_index >= self.sections.len(),
                lg: true,
            }))
            .collect();
        let page = if self.rail_index < self.sections.len() { self.page_view() } else { None };
        let sheet = self.sheet.as_ref().map(|s| match s {
            Sheet::Confirm { ask, index } => SheetView::Confirm {
                icon: ask.icon.clone(),
                title: ask.title.clone(),
                body: ask.body.clone(),
                answers: ask.answers.iter().map(|a| AnswerView { label: a.label.clone(), danger: a.danger }).collect(),
                index: *index,
            },
            Sheet::Pick { spec, index } => SheetView::Pick {
                title: spec.title.clone(),
                rows: spec
                    .options
                    .iter()
                    .map(|o| {
                        let mut r = Row::link(&format!("o:{}", o.value)).label(&o.label).chevron(false).enabled(o.enabled).selected(o.value == spec.current);
                        r.icon = o.icon.clone();
                        if !o.swatch.is_empty() {
                            r.face = Some(super::row::Face { file: None, letter: String::new(), colour: o.swatch.clone() });
                        }
                        r
                    })
                    .collect(),
                index: *index,
            },
            Sheet::Help { row, title, text } => SheetView::Help { row: row.clone(), title: title.clone(), text: text.clone() },
            Sheet::Entry { prompt, text, secret, .. } => SheetView::Entry { prompt: prompt.clone(), text: text.clone(), secret: *secret },
        });
        let hints = self.hints(page.as_ref(), sheet.as_ref());
        View {
            open: self.open,
            zone: self.zone,
            bar: Bar {
                title: "Settings".into(),
                clock: clock_text(self.now),
                wired: net.pointer("/wired/state").and_then(Value::as_str) == Some("connected"),
                link: net.pointer("/wifi/state").and_then(Value::as_str) == Some("connected") || net.pointer("/wired/state").and_then(Value::as_str) == Some("connected"),
            },
            rail,
            page,
            sheet,
            hints,
        }
    }

    fn page_view(&self) -> Option<PageView> {
        let page = self.page(&self.current)?;
        let ctx = self.ctx(page.id, page.title, &self.arg);
        let st = self.state(&self.current);
        let all = self.rows();
        let rows: Vec<Row> = all.iter().filter(|r| r.tab == st.tab && r.visible).cloned().collect();
        let stop = match &st.stop {
            Stop::Unset => StopView::None,
            Stop::Tabs => StopView::Tabs,
            Stop::Row(rid) => rows.iter().position(|r| r.id == *rid).map_or(StopView::None, StopView::Row),
        };
        let back = Some(page.back.to_string()).filter(|b| !b.is_empty());
        let figure = page.figure.and_then(|f| f(&ctx)).map(|mut fig| {
            // what the focused row holds, or the option its picker is on: the picture shows it before it is chosen
            let focused = match stop {
                StopView::Row(i) => rows.get(i),
                _ => None,
            };
            let choice = match (&self.sheet, focused) {
                (Some(Sheet::Pick { spec, index }), Some(_)) => spec.options.get(*index).map(|o| o.value.clone()),
                _ => None,
            };
            fig.shows = focused.and_then(|r| r.preview.as_ref()).map(|p| (p.icon.clone(), choice.unwrap_or_else(|| p.choice.clone())));
            fig.caption = focused.filter(|r| r.preview.is_some()).map(|r| r.label.clone()).unwrap_or_default();
            fig
        });
        Some(PageView {
            id: page.id.to_string(),
            title: self.page_title(page),
            subtitle: (page.subtitle)(&ctx),
            back,
            chip: (page.chip)(&ctx),
            tabs: page.tabs.iter().map(|t| t.to_string()).collect(),
            tab: st.tab,
            rows,
            stop,
            seg: st.seg,
            active: self.zone == Zone::Page,
            figure,
        })
    }

    /// The glyphs of what the keys do now, as the Magic Remote carries them (no sentence): the way out, and before it the one thing the
    /// focus does that a person would not guess (a row that moves sideways). OK on a row is never hinted, nor is the rail's. Never a legend
    /// of every key.
    fn hints(&self, page: Option<&PageView>, sheet: Option<&SheetView>) -> Vec<Hint> {
        let lr = Hint { keys: vec![(Key::Left, false), (Key::Right, false)], action: Some(Action::Move), label: None };
        let out = |a: Action| Hint::key(Key::Back, a);
        match sheet {
            Some(SheetView::Confirm { .. } | SheetView::Pick { .. } | SheetView::Entry { .. }) => return vec![Hint::key(Key::Ok, Action::Confirm), out(Action::Close)],
            Some(SheetView::Help { .. }) => return vec![out(Action::Close)],
            None => {}
        }
        if self.zone == Zone::Rail || page.is_none() {
            return vec![out(Action::Close)];
        }
        let page = page.expect("checked");
        let first = match page.stop {
            StopView::Tabs => Some(lr),
            StopView::Row(_) => {
                let r = page.focused_row().expect("a row stop has a row");
                matches!(r.kind, Kind::Slider | Kind::Nudge | Kind::Seg | Kind::Cards | Kind::Stepper).then_some(lr)
            }
            StopView::None => None,
        };
        first.into_iter().chain(std::iter::once(out(if page.back.is_some() { Action::Collapse } else { Action::Close }))).collect()
    }

    /// The state for the gate and the host harness: `--eval settings`.
    pub fn debug(&self) -> Value {
        let st = self.state(&self.current);
        json!({
            "open": self.open,
            "zone": if self.zone == Zone::Rail { "rail" } else { "page" },
            "railIndex": self.rail_index,
            "page": self.current,
            "trail": self.trail,
            "tab": st.tab,
            "seg": st.seg,
            "stop": match &st.stop { Stop::Unset => Value::Null, Stop::Tabs => json!("tabs"), Stop::Row(r) => json!(r) },
            "sheet": match &self.sheet { None => Value::Null, Some(Sheet::Confirm { ask, index }) => json!({"confirm": ask.title, "index": index}), Some(Sheet::Help { row, .. }) => json!({"help": row}), Some(Sheet::Pick { spec, index }) => json!({"pick": spec.title, "index": index}), Some(Sheet::Entry { prompt, .. }) => json!({"entry": prompt}) },
            "sio": {"answers": self.lg.answers, "idle": self.lg.idle()},
        })
    }
}

impl ScreenModel for Model {
    type View = View;

    fn update(&mut self, input: &Input, now_ms: u64) -> Vec<Effect> {
        self.now = now_ms;
        let rev = self.rev;
        let mut out = Vec::new();
        match input {
            Input::Topic { topic, data } => {
                let before = self.input_app();
                self.topics.set_channel(topic, data.clone());
                self.bump();
                if self.open && topic == "foreground" && self.input_app() != before {
                    // the per-input rows read for the input now in front
                    let id = self.current.clone();
                    self.read_page(&id, now_ms, true, &mut out);
                }
            }
            Input::Helper { topic, data } => {
                if topic == "up" {
                    let up = data.as_bool().unwrap_or(false);
                    let was = self.topics.helper_up;
                    self.topics.helper_up = up;
                    self.bump();
                    if up && !was && self.open {
                        let id = self.current.clone();
                        self.read_page(&id, now_ms, true, &mut out);
                    }
                } else {
                    let same = self.topics.helper(topic) == Some(data);
                    self.topics.set_helper(topic, data.clone());
                    self.bump();
                    if !same && self.open {
                        self.page_hook(&format!("topic:{topic}"), now_ms, &mut out);
                    }
                }
            }
            Input::Luna { tag, .. } if (super::src::APPS_TAG..=super::src::APPS_TAG + 1).contains(tag) => {
                let (fx, changed) = self.apps.update(input);
                out.extend(fx);
                if changed {
                    self.bump();
                }
            }
            Input::Luna { tag, payload } if ours(*tag) => self.luna(*tag, payload, now_ms, &mut out),
            Input::Luna { tag, .. } if *tag == crate::host::power::READ_TAG => {
                let (fx, changed) = self.power.update(input, now_ms);
                out.extend(fx);
                if changed {
                    self.bump();
                }
            }
            Input::Fetched { tag, status, data } if ours(*tag) => self.fetched(*tag, *status, data, now_ms, &mut out),
            Input::Key { name, down, repeat } => out.extend(self.key(name, *down, *repeat, now_ms)),
            Input::Tick => {
                out.extend(self.lg.update(&Input::Tick, now_ms));
                let (fx, _) = self.power.update(&Input::Tick, now_ms);
                out.extend(fx);
                self.timer(now_ms, &mut out);
            }
            _ => {}
        }
        if self.rev != rev {
            self.place_focus();
        }
        if self.rev != rev && self.open && !out.iter().any(|e| matches!(e, Effect::Redraw)) {
            out.push(Effect::Redraw);
        }
        out
    }

    fn view(&self) -> View {
        Model::view(self)
    }

    fn due_at(&self) -> Option<u64> {
        if !self.open {
            return None;
        }
        [self.lg.due_at(), self.power.due_at(), self.timers.iter().map(|(at, _)| *at).min(), self.later.iter().map(|(at, _, _)| *at).min()].into_iter().flatten().min()
    }
}

impl Model {
    /// A page that has the focus and no stop yet (its rows had not come when it was entered) puts the stop on its first row now.
    fn place_focus(&mut self) {
        if !self.open || self.zone != Zone::Page {
            return;
        }
        let id = self.current.clone();
        let st = self.state(&id);
        if st.stop != Stop::Unset {
            return;
        }
        let rows = self.rows();
        let mut st = st;
        if let Some(f) = first(&rows, st.tab) {
            enter(&mut st, &rows, f);
            self.states.insert(id, st);
        }
    }

    fn input_app(&self) -> Option<String> {
        self.ctx(&self.current, "", &PageArg::None).input_app()
    }
}

/// The rows of the household's settings are dimmed for a profile that may not change them, with the reason where the second line is;
/// the first row of each tab that holds one is a way to switch to the owner.
fn lock(rows: &mut Vec<Row>, who: &Who, tabs: usize) {
    let words = format!("Only {} can change this", who.owner);
    let mut locked_tabs = vec![false; tabs.max(1)];
    for r in rows.iter_mut().filter(|r| r.admin && r.visible && !matches!(r.kind, Kind::Head | Kind::Note | Kind::Empty | Kind::Figure)) {
        r.enabled = false;
        r.detail = words.clone();
        if let Some(t) = locked_tabs.get_mut(r.tab) {
            *t = true;
        }
    }
    for tab in (0..locked_tabs.len()).rev().filter(|t| locked_tabs[*t]) {
        let at = rows.iter().position(|r| r.tab == tab).unwrap_or(0);
        let owner = who.owner_id.clone();
        rows.insert(
            at,
            Row::action(&format!("lock:switch:{tab}"), "Switch")
                .label(&format!("Switch to {}", who.owner))
                .icon("user")
                .tab(tab)
                .does(vec![Op::Send { name: None, method: "POST", path: "/v1/profiles/switch".into(), body: Some(json!({"id": owner})) }]),
        );
    }
}

/// "3:05 PM" in the TV's local time.
pub fn clock_text(ms: u64) -> String {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: `tm` is plain data, `localtime_r` fills it for the time given and keeps nothing.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    let h = tm.tm_hour;
    format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, tm.tm_min, if h < 12 { "AM" } else { "PM" })
}

// ------------------------------------------------------------------------------------------------------- the page's own rules

fn focusable_at(rows: &[Row], tab: usize, i: usize) -> bool {
    rows[i].tab == tab && rows[i].focusable()
}

/// The first row the D-pad can reach on a tab.
fn first(rows: &[Row], tab: usize) -> Option<usize> {
    (0..rows.len()).find(|i| focusable_at(rows, tab, *i))
}

/// The next row that way that the D-pad can reach.
fn step(rows: &[Row], tab: usize, from: usize, dir: i32) -> Option<usize> {
    let mut i = from as i64 + i64::from(dir);
    while i >= 0 && (i as usize) < rows.len() {
        if focusable_at(rows, tab, i as usize) {
            return Some(i as usize);
        }
        i += i64::from(dir);
    }
    None
}

fn enter(st: &mut PageState, rows: &[Row], i: usize) {
    st.stop = Stop::Row(rows[i].id.clone());
    st.seg = match rows[i].kind {
        Kind::Seg | Kind::Stepper | Kind::Cards => rows[i].index.unwrap_or(0).min(rows[i].count().saturating_sub(1)),
        _ => 0,
    };
}

fn call(r: &Row, arg: Arg) -> Vec<Op> {
    r.act.as_ref().map(|a| a(arg)).unwrap_or_default()
}

/// What OK does on a row: a switch flips, a segmented row or a card picks, a link or a button runs.
fn activate(st: &PageState, r: &Row) -> Vec<Op> {
    if !r.focusable() {
        return Vec::new();
    }
    match r.kind {
        Kind::Toggle => call(r, Arg::Toggle(!r.on)),
        Kind::Seg | Kind::Cards => call(r, Arg::Index(st.seg)),
        Kind::Value | Kind::Action => call(r, Arg::Press),
        _ => Vec::new(),
    }
}

/// `Math.round`: halves go up.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

fn page_press(st: &mut PageState, rows: &[Row], tabbed: bool, ntabs: usize, k: K, now: u64) -> Option<Vec<Op>> {
    if st.stop == Stop::Tabs || (st.stop == Stop::Unset && tabbed) {
        // the tab bar: Left and Right change the tab and leave the focus on the bar, so the next press goes to the next tab
        return match k {
            K::Left if st.tab == 0 => None,
            K::Left => {
                st.stop = Stop::Tabs;
                st.tab -= 1;
                Some(Vec::new())
            }
            K::Right => {
                st.stop = Stop::Tabs;
                st.tab = (st.tab + 1).min(ntabs.saturating_sub(1));
                Some(Vec::new())
            }
            K::Down => {
                if let Some(f) = first(rows, st.tab) {
                    enter(st, rows, f);
                }
                Some(Vec::new())
            }
            K::Ok => Some(Vec::new()),
            K::Up | K::Back => None,
        };
    }
    let Stop::Row(rid) = &st.stop else { return None };
    let i = rows.iter().position(|r| r.id == *rid)?;
    let r = &rows[i];
    match k {
        K::Up => {
            if r.kind == Kind::Cards && st.seg >= r.cols {
                st.seg -= r.cols;
                return Some(Vec::new());
            }
            if let Some(p) = step(rows, st.tab, i, -1) {
                enter(st, rows, p);
            } else if tabbed {
                st.stop = Stop::Tabs;
            } else {
                return None;
            }
            Some(Vec::new())
        }
        K::Down => {
            if r.kind == Kind::Cards && st.seg + r.cols < r.count() {
                st.seg += r.cols;
                return Some(Vec::new());
            }
            if let Some(n) = step(rows, st.tab, i, 1) {
                enter(st, rows, n);
            }
            Some(Vec::new())
        }
        K::Left | K::Right => {
            let d: i64 = if k == K::Left { -1 } else { 1 };
            match r.kind {
                Kind::Seg | Kind::Cards => {
                    if st.seg as i64 + d < 0 {
                        return None;
                    }
                    st.seg = ((st.seg as i64 + d) as usize).min(r.count().saturating_sub(1));
                    Some(Vec::new())
                }
                // the value is read back; Left and Right only say which way
                Kind::Nudge => Some(call(r, Arg::Nudge(d as i32))),
                Kind::Stepper => {
                    let n = r.count() as i64;
                    if n > 0 {
                        st.seg = ((st.seg as i64 + d + n) % n) as usize;
                        return Some(call(r, Arg::Index(st.seg)));
                    }
                    Some(Vec::new())
                }
                Kind::Slider => {
                    let (lo, hi, stp) = (r.min, r.max, if r.step > 0.0 { r.step } else { 1.0 });
                    // a key held (or pressed again within a quarter second in the same direction) runs: on a range of more than 40 steps the
                    // step grows to 2 after ten presses and to 5 after twenty-four, so 0 to 100 takes a few seconds, not a hundred presses
                    st.slider_run = if d as i32 == st.slider_dir && now.saturating_sub(st.slider_at) < 250 { st.slider_run + 1 } else { 0 };
                    st.slider_dir = d as i32;
                    st.slider_at = now;
                    let mult = if (hi - lo) / stp > 40.0 {
                        match st.slider_run {
                            24.. => 5.0,
                            10.. => 2.0,
                            _ => 1.0,
                        }
                    } else {
                        1.0
                    };
                    let v = (js_round((r.num + d as f64 * stp * mult) / stp) * stp).clamp(lo, hi);
                    if (v - r.num).abs() > 1e-9 { Some(call(r, Arg::Value(v))) } else { Some(Vec::new()) }
                }
                _ => {
                    if d < 0 {
                        None
                    } else {
                        Some(Vec::new())
                    }
                }
            }
        }
        K::Ok => Some(activate(st, r)),
        K::Back => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::Spec;
    
    fn alpha_rows(c: &Ctx) -> Vec<Row> {
        vec![
            c.toggle(Spec::new("test.eye").tab(0).label("Eye comfort").help("A warmer picture for the evening.")),
            c.seg(Spec::new("test.mode").tab(0).label("Energy saving").options(&["Auto", "Off", "Max"]).values(&["auto", "off", "max"])),
            c.slider(Spec::new("test.light").tab(0).label("Backlight").range(0.0, 100.0, 1.0)),
            c.toggle(Spec::new("test.gone").tab(0).label("Dimmed")),
            Row::link("child").tab(0).label("Fine Tune").does(vec![Op::Open { page: "kid".into(), arg: PageArg::None }]),
            Row::action("reset", "Reset").tab(0).label("Reset").danger().does(crate::settings::ctx::ask("rotate-ccw", "Reset?", "Back to LG's.", "Reset", "Cancel", true, vec![Op::Act { uri: "luna://x/reset".into(), payload: json!({}) }])),
            c.toggle(Spec::new("test.two").tab(1).label("Second tab")),
        ]
    }

    fn beta_rows(c: &Ctx) -> Vec<Row> {
        vec![c.toggle(Spec::new("test.beta").label("Beta switch"))]
    }

    fn kid_rows(_: &Ctx) -> Vec<Row> {
        vec![Row::info("kid:info").label("Info").help("Only info with help"), Row::toggle("kid:t").label("Kid toggle").act(|_| vec![Op::Set("kid".into(), json!(true))])]
    }

    fn alpha_reads(_: &Ctx) -> Vec<Read> {
        vec![Read::lg("test", &["eye", "mode", "light", "two"])]
    }

    fn model() -> Model {
        let pages = vec![
            Page::new("alpha", "Alpha", "image").rail().tabs(&["One", "Two"]).rows(alpha_rows).reads(alpha_reads).summary(|c| c.lg.opt_text("test.mode").unwrap_or_default()),
            Page::new("beta", "Beta", "volume-2").rail().rows(beta_rows),
            Page::new("kid", "Kid", "sliders").parent("alpha").back("Alpha").rows(kid_rows),
        ];
        let mut m = Model::new(pages, json!({}), json!({}));
        m.lg.learn("test", &json!({"settings": {"eye": "off", "mode": "off", "light": "50", "two": "on", "beta": "on"}, "errorKey": ["gone"]}));
        m
    }

    fn press(m: &mut Model, name: &str, t: u64) -> Vec<Effect> {
        let mut out = m.update(&Input::Key { name: name.into(), down: true, repeat: false, }, t);
        out.extend(m.update(&Input::Key { name: name.into(), down: false, repeat: false }, t + 20));
        out
    }

    fn luna_writes(effects: &[Effect]) -> Vec<(String, Value)> {
        effects.iter().filter_map(|e| match e { Effect::Luna { uri, payload, .. } if uri.ends_with("setSystemSettings") => Some((uri.clone(), payload.clone())), _ => None }).collect()
    }

    #[test]
    fn it_opens_on_the_rail_with_the_pages_summaries_and_lgs_row_last() {
        let mut m = model();
        let fx = m.open(None, 1000);
        let v = m.view();
        assert!(v.open && v.zone == Zone::Rail);
        assert_eq!(v.rail.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), vec!["Alpha", "Beta", "More in LG settings"]);
        assert_eq!(v.rail[0].summary, "off", "the rail's second line is the page's own summary");
        assert!(v.rail[0].focused && v.rail[0].selected && v.rail[2].lg);
        // the first reads go out one at a time: the net subscription and the pointer, then the page's keys
        let luna: Vec<&Effect> = fx.iter().filter(|e| matches!(e, Effect::Luna { .. })).collect();
        assert!(luna.iter().any(|e| matches!(e, Effect::Luna { uri, subscribe: true, .. } if uri.ends_with("connectionmanager/getStatus"))));
        assert!(luna.iter().any(|e| matches!(e, Effect::Luna { uri, payload, .. } if uri.ends_with("getSystemSettings") && payload["category"] == "general")), "{luna:?}");
    }

    #[test]
    fn the_rail_moves_with_up_and_down_and_stops_at_its_ends_and_back_closes() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "up", 10);
        assert_eq!(m.rail_index(), 0);
        press(&mut m, "down", 100);
        assert_eq!((m.rail_index(), m.current()), (1, "beta"));
        press(&mut m, "down", 200);
        press(&mut m, "down", 300);
        assert_eq!(m.rail_index(), 2, "the last row is LG's");
        assert!(m.view().page.is_none(), "LG's row has no page");
        press(&mut m, "up", 400);
        assert_eq!(m.current(), "beta");
        press(&mut m, "up", 500);
        press(&mut m, "back", 600);
        assert_eq!(m.take_requests(), vec![ShellRequest::Close]);
    }

    #[test]
    fn ok_on_the_last_rail_row_launches_lgs_settings_and_closes() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "down", 10);
        press(&mut m, "down", 100);
        press(&mut m, "ok", 200);
        assert_eq!(m.take_requests(), vec![ShellRequest::Launch { app: "com.palm.app.settings".into(), params: json!({}) }, ShellRequest::Close]);
    }

    #[test]
    fn right_enters_the_page_on_its_first_row_and_left_or_back_returns_to_the_rail() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        let v = m.view();
        assert_eq!(v.zone, Zone::Page);
        let p = v.page.unwrap();
        assert_eq!(p.focused_row().map(|r| r.id.as_str()), Some("test.eye"));
        assert!(p.active);
        press(&mut m, "left", 100);
        assert_eq!(m.zone(), Zone::Rail);
        press(&mut m, "ok", 200);
        assert_eq!(m.zone(), Zone::Page, "OK enters it too");
        press(&mut m, "back", 300);
        assert_eq!(m.zone(), Zone::Rail, "Back from the page returns to the rail and does not close");
        assert!(m.take_requests().is_empty());
    }

    #[test]
    fn a_dimmed_row_and_a_missing_key_take_no_focus_and_the_focus_walks_the_rows() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        let mut seen = vec![m.view().page.unwrap().focused_row().unwrap().id.clone()];
        for i in 0..5 {
            press(&mut m, "down", 100 + i * 100);
            seen.push(m.view().page.unwrap().focused_row().unwrap().id.clone());
        }
        // test.gone is a key the TV lacks (left out), the rows after it follow; the bottom holds
        assert_eq!(seen, vec!["test.eye", "test.mode", "test.light", "child", "reset", "reset"]);
    }

    #[test]
    fn a_segmented_row_moves_inside_itself_and_ok_writes_the_value_at_the_segment() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        press(&mut m, "down", 100);
        let p = m.view().page.unwrap();
        assert_eq!(p.seg, 1, "the focus starts on the value in force");
        press(&mut m, "left", 200);
        assert_eq!(m.view().page.unwrap().seg, 0);
        let fx = press(&mut m, "ok", 300);
        let w = luna_writes(&fx);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].1["settings"]["mode"], "auto");
        assert!(m.lg.is("test.mode", "auto"), "shown at once");
        assert_eq!(m.view().rail[0].summary, "auto", "and the rail's line follows");
        // Left at the first segment returns to the rail (the row cannot use it)
        press(&mut m, "left", 400);
        assert_eq!(m.zone(), Zone::Rail);
    }

    #[test]
    fn a_switch_flips_on_ok_and_a_slider_steps_and_a_held_key_runs_faster() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        let fx = press(&mut m, "ok", 100);
        assert_eq!(luna_writes(&fx)[0].1["settings"]["eye"], "on");
        press(&mut m, "down", 200);
        press(&mut m, "down", 300);
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "test.light");
        let w = luna_writes(&m.update(&Input::Key { name: "right".into(), down: true, repeat: false }, 1000));
        assert_eq!(w[0].1["settings"]["light"], "51");
        // a key held: the step grows to 2 after ten presses within a quarter second of each other and to 5 after twenty-four
        let mut now = 1000;
        let mut last = 51.0;
        let mut steps = Vec::new();
        for _ in 0..40 {
            now += 40;
            let w = luna_writes(&m.update(&Input::Key { name: "right".into(), down: true, repeat: true }, now));
            let Some(w) = w.first() else { break };
            let v: f64 = w.1["settings"]["light"].as_str().unwrap().parse().unwrap();
            steps.push(v - last);
            last = v;
        }
        assert_eq!(&steps[..9], &[1.0; 9], "ten presses at a step of 1");
        assert_eq!(steps[9], 2.0, "then 2");
        assert!(steps.contains(&5.0), "after twenty-four presses 5: {steps:?}");
        assert_eq!(last, 100.0, "the end holds: nothing is written past it");
        assert_eq!(m.lg.num("test.light", 0.0), 100.0);
    }

    #[test]
    fn the_tab_bar_keeps_the_focus_so_right_walks_through_the_tabs_and_down_enters_a_tab() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        press(&mut m, "up", 100);
        assert_eq!(m.view().page.unwrap().stop, StopView::Tabs, "Up from the first row reaches the tab bar");
        press(&mut m, "up", 200);
        assert_eq!((m.zone(), m.view().page.unwrap().stop), (Zone::Page, StopView::Tabs), "Up from the bar goes nowhere: only Left and Back leave a page");
        press(&mut m, "right", 500);
        let p = m.view().page.unwrap();
        assert_eq!((p.tab, p.stop.clone()), (1, StopView::Tabs), "the bar keeps the focus on the next tab");
        assert_eq!(p.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["test.two"]);
        press(&mut m, "right", 600);
        assert_eq!(m.view().page.unwrap().tab, 1, "the last tab holds");
        press(&mut m, "down", 700);
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "test.two");
        press(&mut m, "up", 800);
        press(&mut m, "left", 900);
        assert_eq!(m.view().page.unwrap().tab, 0);
        press(&mut m, "left", 1000);
        assert_eq!(m.zone(), Zone::Rail, "Left on the first tab leaves");
    }

    #[test]
    fn a_child_page_opens_from_a_row_and_back_returns_to_the_page_it_was_opened_from() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        for t in 1..=3 {
            press(&mut m, "down", 100 * t);
        }
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "child");
        press(&mut m, "ok", 1000);
        assert_eq!((m.current(), m.trail().to_vec()), ("kid", vec!["alpha".to_string()]));
        let v = m.view();
        assert!(v.rail[0].selected, "the parent's section stays lit on the rail");
        let p = v.page.unwrap();
        assert_eq!((p.title.as_str(), p.back.as_deref()), ("Kid", Some("Alpha")));
        assert_eq!(p.focused_row().map(|r| r.id.as_str()), Some("kid:info"), "a row that only shows is reached by the D-pad when it has help");
    }

    #[test]
    fn back_from_a_child_goes_up_one_step_and_the_parent_keeps_its_focus() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        for t in 1..=3 {
            press(&mut m, "down", 100 * t);
        }
        press(&mut m, "ok", 1000);
        press(&mut m, "back", 1100);
        assert_eq!((m.current(), m.trail().len(), m.zone()), ("alpha", 0, Zone::Page));
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "child", "the row that opened it is still focused");
        press(&mut m, "back", 1200);
        assert_eq!(m.zone(), Zone::Rail);
    }

    #[test]
    fn a_held_ok_on_a_row_with_help_opens_it_and_the_release_does_nothing_more() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        m.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 1000);
        assert!(m.due_at().is_some(), "the hold is timed");
        m.update(&Input::Tick, 1000 + HOLD_MS);
        assert!(matches!(m.view().sheet, Some(SheetView::Help { ref title, .. }) if title == "Eye comfort"));
        let fx = m.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 2000);
        assert!(luna_writes(&fx).is_empty(), "the release of the hold flips nothing");
        assert!(matches!(m.view().sheet, Some(SheetView::Help { .. })), "and does not close the sheet");
        // any key closes it before anything else sees it
        let fx = m.update(&Input::Key { name: "down".into(), down: true, repeat: false }, 2100);
        assert!(m.view().sheet.is_none());
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "test.eye", "the key was taken by the sheet");
        assert!(luna_writes(&fx).is_empty());
        // Back closes it before it leaves the page
        m.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        m.update(&Input::Tick, 3000 + HOLD_MS);
        m.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3800);
        m.update(&Input::Key { name: "back".into(), down: true, repeat: false }, 3900);
        assert!(m.view().sheet.is_none());
        assert_eq!(m.zone(), Zone::Page);
    }

    #[test]
    fn a_held_ok_on_a_row_without_help_is_an_ordinary_press() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        press(&mut m, "down", 100);
        press(&mut m, "down", 200);
        press(&mut m, "down", 300);
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "child");
        m.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 1000);
        assert_eq!(m.due_at().filter(|d| *d <= 1000 + HOLD_MS && *d > 1000), None, "no hold is timed where there is no help");
        m.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 1900);
        assert_eq!(m.current(), "kid", "the release is the press");
    }

    #[test]
    fn the_info_mark_of_a_row_opens_its_help_by_the_pointer() {
        let mut m = model();
        m.open(None, 0);
        m.click(Target::Info("test.eye".into()), 100);
        assert!(matches!(m.view().sheet, Some(SheetView::Help { ref row, .. }) if row == "test.eye"));
        assert_eq!((m.zone(), m.view().page.unwrap().focused_row().unwrap().id.as_str()), (Zone::Page, "test.eye"), "a pointer that lands on a row takes the focus as the D-pad would");
    }

    #[test]
    fn a_question_has_the_safe_answer_first_and_back_means_it() {
        let mut m = model();
        m.open(None, 0);
        press(&mut m, "right", 10);
        for t in 1..=5 {
            press(&mut m, "down", 100 * t);
        }
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "reset");
        press(&mut m, "ok", 1000);
        let Some(SheetView::Confirm { answers, index, title, .. }) = m.view().sheet else { panic!("a question is up") };
        assert_eq!((title.as_str(), index, answers.iter().map(|a| (a.label.as_str(), a.danger)).collect::<Vec<_>>()), ("Reset?", 0, vec![("Cancel", false), ("Reset", true)]));
        // keys go to the question, not the page behind it
        press(&mut m, "up", 1100);
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "reset");
        let fx = press(&mut m, "back", 1200);
        assert!(m.view().sheet.is_none());
        assert!(!fx.iter().any(|e| matches!(e, Effect::Luna { uri, .. } if uri == "luna://x/reset")), "Back is the safe answer: nothing is done");
        press(&mut m, "ok", 1300);
        press(&mut m, "right", 1400);
        let fx = press(&mut m, "ok", 1500);
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { uri, .. } if uri == "luna://x/reset")), "the second answer does it");
        assert!(m.view().sheet.is_none());
    }

    #[test]
    fn a_profile_that_may_not_change_the_households_settings_sees_them_dimmed_with_the_owners_name_and_a_way_to_switch() {
        let mut m = model();
        m.topics.set_helper("profiles", json!({"active": "isabel", "owner": "nitin", "canAdmin": false, "profiles": [{"id": "nitin", "name": "Nitin", "owner": true}]}));
        m.open(None, 0);
        press(&mut m, "right", 10);
        let p = m.view().page.unwrap();
        let eye = p.rows.iter().find(|r| r.id == "test.eye").unwrap();
        assert!(!eye.enabled && eye.detail == "Only Nitin can change this");
        assert_eq!(p.rows[0].label, "Switch to Nitin", "the first row of the tab is the way to switch");
        assert_eq!(p.focused_row().unwrap().id, "lock:switch:0", "and it is the first the focus reaches");
        let link = p.rows.iter().find(|r| r.id == "child").unwrap();
        assert!(link.enabled, "a link to a page is not a setting");
    }

    #[test]
    fn a_choice_row_opens_the_small_picker_on_the_value_in_force_and_a_pick_writes_it_and_closes_it() {
        fn gamma_rows(c: &Ctx) -> Vec<Row> {
            vec![c.choice(Spec::new("test.gamma").tab(0).label("Gamma").names(crate::settings::names::GAMMA))]
        }
        let pages = vec![Page::new("alpha", "Alpha", "image").rail().rows(gamma_rows)];
        let mut m = Model::new(pages, json!({}), json!({}));
        m.lg.learn("test", &json!({"settings": {"gamma": "medium"}}));
        m.lg.learn_desc("test", &json!({"results": [{"key": "gamma", "values": {"arrayExt": [{"value": "low"}, {"value": "medium"}, {"value": "high2", "active": false}]}}]}));
        m.open(None, 0);
        press(&mut m, "right", 10);
        press(&mut m, "ok", 100);
        let Some(SheetView::Pick { title, rows, index }) = m.view().sheet else { panic!("the picker is up") };
        assert_eq!((title.as_str(), index), ("Gamma", 1), "the focus starts on the value in force");
        assert_eq!(rows.iter().map(|r| (r.label.as_str(), r.selected, r.enabled)).collect::<Vec<_>>(), vec![("1.9", false, true), ("2.2", true, true), ("BT.1886", false, false)]);
        press(&mut m, "down", 200);
        let Some(SheetView::Pick { index, .. }) = m.view().sheet else { panic!() };
        assert_eq!(index, 1, "the option the TV says is not active is skipped");
        press(&mut m, "up", 300);
        // keys go to the picker, not to the page behind it
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().id, "test.gamma");
        let fx = press(&mut m, "ok", 400);
        assert_eq!(luna_writes(&fx)[0].1["settings"]["gamma"], "low");
        assert!(m.view().sheet.is_none());
        assert_eq!(m.view().page.unwrap().rows[0].value, "1.9");
        // Back closes it and writes nothing
        press(&mut m, "ok", 500);
        let fx = press(&mut m, "back", 600);
        assert!(m.view().sheet.is_none() && luna_writes(&fx).is_empty());
        assert_eq!(m.zone(), Zone::Page, "and does not leave the page");
        // the pointer picks by a click and a click outside closes it
        press(&mut m, "ok", 700);
        let fx = m.click(Target::Pick(1), 800);
        assert_eq!(luna_writes(&fx)[0].1["settings"]["gamma"], "medium");
        press(&mut m, "ok", 900);
        m.click(Target::Outside, 1000);
        assert!(m.view().sheet.is_none());
    }

    #[test]
    fn the_bar_shows_the_way_out_and_before_it_only_a_row_that_moves_sideways_never_a_legend() {
        use crate::kit::hint::{Action, Key};
        let mut m = model();
        m.open(None, 0);
        let names = |m: &Model| m.view().hints.iter().map(|h| (h.keys.iter().map(|k| k.0).collect::<Vec<_>>(), h.action)).collect::<Vec<_>>();
        assert_eq!(names(&m), vec![(vec![Key::Back], Some(Action::Close))], "the rail: the way out, OK is not hinted");
        press(&mut m, "right", 10);
        press(&mut m, "down", 100);
        assert_eq!(m.view().page.unwrap().focused_row().unwrap().kind, Kind::Seg);
        assert_eq!(names(&m), vec![(vec![Key::Left, Key::Right], Some(Action::Move)), (vec![Key::Back], Some(Action::Close))], "a segmented row is moved sideways");
        press(&mut m, "down", 150);
        press(&mut m, "ok", 200);
        for _ in 0..4 {
            assert!(m.view().hints.len() <= 2, "{:?}", m.view().hints);
            assert!(m.view().hints.iter().all(|h| h.keys != vec![(Key::Ok, false)]), "OK is not hinted on a row: {:?}", m.view().hints);
            press(&mut m, "down", 300);
        }
    }

    #[test]
    fn the_calls_a_page_makes_keep_their_answers_by_name_and_a_subscription_is_let_go_when_settings_closes() {
        let mut m = model();
        m.open(None, 0);
        // the net subscription answered
        let tag = m.routes.iter().find(|(_, r)| matches!(r, Route::Call { name, .. } if name == "net")).map(|(t, _)| *t).unwrap();
        m.update(&Input::Luna { tag, payload: json!({"wired": {"state": "connected"}, "wifi": {"state": "disconnected"}}) }, 10);
        let v = m.view();
        assert!(v.bar.wired && v.bar.link);
        let fx = m.close(100);
        assert!(fx.iter().any(|e| matches!(e, Effect::Cancel { tag: t } if *t == tag)));
        assert!(m.due_at().is_none() && !m.view().open);
    }

    #[test]
    fn lg_reads_go_one_at_a_time_and_an_answer_sends_the_next() {
        let mut m = model();
        let fx = m.open(None, 0);
        let first: Vec<u64> = fx.iter().filter_map(|e| match e { Effect::Luna { tag, uri, .. } if uri.contains("getSystemSetting") => Some(*tag), _ => None }).collect();
        assert_eq!(first.len(), 1, "one read in flight");
        let fx = m.update(&Input::Luna { tag: first[0], payload: json!({"settings": {"remotePointer": "on"}}) }, 10);
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Luna { uri, .. } if uri.contains("getSystemSetting"))).count(), 1, "the answer sends the next");
        assert_eq!(m.due_at(), Some(10 + crate::host::sio::READ_MS), "a read that does not answer is let go after two seconds");
    }

    #[test]
    fn a_helper_owner_row_writes_through_the_helper_and_takes_its_answer() {
        let mut m = model();
        m.open(None, 0);
        m.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 5);
        let mut out = Vec::new();
        m.run_reads(vec![Read::owner("base.ambient")], 10, &mut out);
        let Some(Effect::Request { tag, method, path, .. }) = out.first().cloned() else { panic!() };
        assert_eq!((method.as_str(), path.as_str()), ("GET", "/v1/settings/base.ambient"));
        m.update(
            &Input::Fetched { tag, status: 200, data: json!({"ok": true, "version": 1, "effective": {"enabled": {"value": false}}, "schema": {"sections": [{"fields": [{"key": "enabled", "kind": {"type": "bool"}}]}]}}) },
            20,
        );
        let mut out = Vec::new();
        m.apply(vec![Op::Owner { owner: "base.ambient".into(), key: "enabled".into(), value: json!(true) }], 30, &mut out);
        let Some(Effect::Request { tag, method, path, body, .. }) = out.first().cloned() else { panic!() };
        assert_eq!((method.as_str(), path.as_str()), ("POST", "/v1/settings/base.ambient"));
        assert_eq!(body.unwrap()["changes"][0]["value"], json!({"type": "bool", "value": true}));
        assert_eq!(m.owners["base.ambient"].get("enabled"), Some(&json!(true)), "shown at once");
        m.update(&Input::Fetched { tag, status: 409, data: json!({"ok": false, "error": {"message": "refused"}}) }, 40);
        assert_eq!(m.owners["base.ambient"].problem, "refused");
    }

    #[test]
    fn a_patch_of_the_theme_is_applied_at_once_and_written_by_the_host() {
        let mut m = Model::new(vec![Page::new("alpha", "Alpha", "image").rail()], json!({"clock": {"style": "analog", "enabled": true}}), json!({}));
        m.open(None, 0);
        let mut out = Vec::new();
        m.apply(vec![Op::Patch(json!({"clock": {"style": "digital"}}))], 10, &mut out);
        assert_eq!(m.theme["clock"], json!({"style": "digital", "enabled": true}), "merged one level deep: the section's other keys stay");
        let Some(Effect::Theme { tag, patch }) = out.first().cloned() else { panic!() };
        assert_eq!(patch, json!({"clock": {"style": "digital"}}));
        m.update(&Input::Fetched { tag, status: 200, data: json!({"clock": {"style": "digital"}}) }, 20);
        assert_eq!(m.take_theme_file(), Some(json!({"clock": {"style": "digital"}})));
    }
}
