//! The channel guide's model (`docs/research/channel-guide.md`): the channels and their programmes as a grid of rows (a channel each) over time,
//! the remote's keys and the pointer, what is asked of LG's guide service and when, and the detail sheet of the programme in focus. Pure: it
//! reads `Input`s on the wall clock in milliseconds (the helper's stamps) and answers `Effect`s; the surface draws what [`Guide`] says and
//! the host joins the two.
//!
//! The model counts in time and in rows, not in pixels: what fits on the screen (rows, the span of time) is told by the surface
//! ([`Guide::set_page`]) because it follows the person's text size. The focus is a row and an instant; the cell in focus is the programme
//! on the air at that instant, or the gap between two programmes. Left and right go to the neighbouring cell, up and down keep the instant,
//! and the view moves only as far as the focus needs (plus a little clearance), never to centre it.
//!
//! What it asks the service for, and no more than is shown: the antenna's channels at once, the internet's when asked for (150 KB), the
//! programmes of the rows on the screen and a row either side for the hours on the screen, more for the row in focus, a few channels and
//! at most six hours a call, two calls at a time, after the focus has been still for a moment. A window that came back is kept; a window
//! that came back empty is an honest "no information", not a call to make again.
//!
//! The same windows are asked of the helper's Guide data plugin (`GET /v1/guide/programmes`): online listings for the zip code, which lie
//! over LG's where they have a programme and leave LG's the gaps (`lumaguide::merge`). A helper whose plugin is off says so, and the guide
//! asks it nothing more until it opens again.

use crate::host::channels::{self as ch, Channel, Fault, Programme, Source};
use crate::host::clock;
use crate::kit::screen::{Effect, Input};
use crate::panels::life::{Change, ENTER_MS, EXIT_MS, Life, Phase};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const MIN: i64 = 60_000;
pub const HOUR: i64 = 60 * MIN;
pub const DAY: i64 = 24 * HOUR;

pub const TAG_LIST: u64 = ch::TAG_BASE + 1;
pub const TAG_LIST_NET: u64 = ch::TAG_BASE + 2;
pub const TAG_LAST: u64 = ch::TAG_BASE + 3;
pub const TAG_TUNE: u64 = ch::TAG_BASE + 4;
/// The tags of the programme calls: this and the next 255.
pub const TAG_EVENTS: u64 = ch::TAG_BASE + 0x100;
/// The tags of the online listings' calls to the helper: this and the next 255.
pub const TAG_ONLINE: u64 = ch::TAG_BASE + 0x200;
/// Where the helper serves the online listings.
pub const ONLINE_PATH: &str = "/v1/guide/programmes";

/// How long the focus is still before the programmes of what it moved to are asked for.
pub const FETCH_DEBOUNCE_MS: u64 = 140;
/// How long a channel that did not answer is left alone.
pub const RETRY_MS: u64 = 15_000;
/// Calls in flight at once, channels in one call, hours in one call.
pub const IN_FLIGHT: usize = 2;
pub const BATCH: usize = 3;
pub const CALL_SPAN: i64 = 6 * HOUR;
/// A programme call that is not answered in this long is given up on (the channel is asked for again after [`RETRY_MS`]).
pub const FLIGHT_MS: u64 = 12_000;
/// How long LG's launch is waited for before the guide leaves anyway (a launch that has not said no has switched).
pub const TUNE_WAIT_MS: u64 = 1500;
/// A guide nobody touches for this long closes (a still screen of bright rows is what the panel should not hold).
pub const IDLE_MS: u64 = 5 * 60_000;
/// Listings older than this are asked for again when the guide opens.
pub const STALE_MS: u64 = 10 * 60_000;
/// The time the grid can show: from six hours before the hour the guide opened to five days after it.
pub const PAST: i64 = 6 * HOUR;
pub const REACH: i64 = 5 * DAY;

/// Which channels the rows are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    All,
    Antenna,
    Internet,
    Favourites,
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Antenna => "Antenna",
            Filter::Internet => "Internet",
            Filter::Favourites => "Favourites",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Antenna => "antenna",
            Filter::Internet => "internet",
            Filter::Favourites => "favourites",
        }
    }
}

/// A chip over the grid: a filter, or the way back to the programmes on the air (the remote has no colour key for it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Filter(Filter),
    Now,
}

/// Where the keys are: the filter chips over the grid, the grid, or the detail sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Chips,
    Grid,
    Sheet,
}

/// What a click or the pointer's press can be on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Chip(usize),
    /// A cell of a row at an instant inside it.
    Cell { row: usize, at: i64 },
    /// A channel in the rail.
    Rail(usize),
    /// The sheet's button: tune.
    Watch,
    /// The sheet is dismissed (the dim behind it).
    Dismiss,
}

/// What fits on the screen: the rows of the grid and the span of time across it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub rows: usize,
    pub span: i64,
}

impl Default for Page {
    fn default() -> Self {
        Self { rows: 8, span: 3 * HOUR }
    }
}

/// A stretch of one row: a programme, or the gap where the guide has none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span<'a> {
    pub start: i64,
    pub end: i64,
    pub programme: Option<&'a Programme>,
    /// A gap whose hours have not been asked for yet or are on their way: not "no information" yet.
    pub loading: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Flight {
    tag: u64,
    channels: Vec<String>,
    from: i64,
    to: i64,
    /// When it went out: a call that is not answered in [`FLIGHT_MS`] is given up on.
    at: u64,
}

/// The detail sheet's words: the channel, the programme in focus with how it stands to now, and what follows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet<'a> {
    pub channel: &'a Channel,
    pub main: Option<&'a Programme>,
    /// `On now`, `Next`, `Later`, `Earlier`, or empty for no information.
    pub when: &'static str,
    pub after: Vec<&'a Programme>,
    pub now: i64,
}

pub struct Guide {
    life: Life,
    open: bool,
    last_input: u64,
    now: i64,
    minute: i64,
    offset_s: i32,
    origin: i64,
    // the channels
    tuner: Option<Vec<Channel>>,
    net: Option<Vec<Channel>>,
    list_fault: Option<Fault>,
    net_fault: Option<Fault>,
    net_asked: bool,
    all: Vec<Channel>,
    rows: Vec<usize>,
    filter: Filter,
    last: Option<String>,
    moved: bool,
    // the grid
    page: Page,
    view: i64,
    top: usize,
    region: Region,
    chip: usize,
    row: usize,
    at: i64,
    sheet: bool,
    // the programmes: LG's as they came, the online listings as they came, and the two laid over each other (what is drawn)
    broadcast: HashMap<String, Vec<Programme>>,
    online: HashMap<String, Vec<Programme>>,
    store: HashMap<String, Vec<Programme>>,
    cover: HashMap<String, Vec<(i64, i64)>>,
    flight: Vec<Flight>,
    /// The online calls on their way, as the luna calls they go with.
    online_flight: Vec<Flight>,
    /// The helper said its Guide data plugin is off: nothing is asked of it until the guide opens again.
    online_off: bool,
    seq: u64,
    fetch_due: Option<u64>,
    retry: HashMap<String, u64>,
    data_fault: Option<Fault>,
    data_at: u64,
    /// What the rows were drawn from changed: per channel, and for all of them (a new list, listings dropped as stale).
    revs: HashMap<String, u32>,
    epoch: u32,
    /// When the channel switch was sent, while its answer is awaited, and whether the last one was refused.
    tuning: Option<u64>,
    tune_fault: bool,
}

fn floor_to(t: i64, step: i64) -> i64 {
    t.div_euclid(step) * step
}

impl Default for Guide {
    fn default() -> Self {
        Self::new()
    }
}

impl Guide {
    pub fn new() -> Self {
        Self {
            life: Life::new(ENTER_MS, EXIT_MS, 0),
            open: false,
            last_input: 0,
            now: 0,
            minute: 0,
            offset_s: 0,
            origin: 0,
            tuner: None,
            net: None,
            list_fault: None,
            net_fault: None,
            net_asked: false,
            all: Vec::new(),
            rows: Vec::new(),
            filter: Filter::Antenna,
            last: None,
            moved: false,
            page: Page::default(),
            view: 0,
            top: 0,
            region: Region::Grid,
            chip: 0,
            row: 0,
            at: 0,
            sheet: false,
            broadcast: HashMap::new(),
            online: HashMap::new(),
            store: HashMap::new(),
            cover: HashMap::new(),
            flight: Vec::new(),
            online_flight: Vec::new(),
            online_off: false,
            seq: 0,
            fetch_due: None,
            retry: HashMap::new(),
            data_fault: None,
            data_at: 0,
            revs: HashMap::new(),
            epoch: 0,
            tuning: None,
            tune_fault: false,
        }
    }

    // ------------------------------------------------------------------------------------------------------------ what the surface reads

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The guide has the keys while it is open.
    pub fn wants(&self) -> bool {
        self.open
    }

    pub fn phase(&self) -> Phase {
        self.life.phase()
    }

    /// How many times what is known of `channel` changed since the guide began: what a drawn row is compared with to know it is stale.
    pub fn rev(&self, channel: &str) -> u64 {
        (u64::from(self.epoch) << 32) | u64::from(self.revs.get(channel).copied().unwrap_or(0))
    }

    /// The pointer moved: the guide is not idle.
    pub fn touch(&mut self, now: u64) {
        self.last_input = now;
    }

    /// A channel is being switched to (LG's launch has not answered yet).
    pub fn tuning(&self) -> bool {
        self.tuning.is_some()
    }

    /// LG's launch said no to the last channel switch.
    pub fn tune_fault(&self) -> bool {
        self.tune_fault
    }

    pub fn built(&self) -> bool {
        self.life.built()
    }

    pub fn filter(&self) -> Filter {
        self.filter
    }

    /// The chips over the grid: the filters this set can show, in order, then Now. The internet's is offered until its list says there is none.
    pub fn chips(&self) -> Vec<Chip> {
        let mut out = vec![Chip::Filter(Filter::All)];
        if self.tuner.as_ref().is_none_or(|t| !t.is_empty()) {
            out.push(Chip::Filter(Filter::Antenna));
        }
        if self.net.as_ref().is_none_or(|n| !n.is_empty()) && self.net_fault.is_none() {
            out.push(Chip::Filter(Filter::Internet));
        }
        if self.all.iter().any(Channel::favourite) {
            out.push(Chip::Filter(Filter::Favourites));
        }
        out.push(Chip::Now);
        out
    }

    /// The word of the antenna chip: what the tuner is (`Antenna`, or `Cable` or `Satellite` when the channels say so).
    pub fn antenna_label(&self) -> &'static str {
        match self.tuner.as_ref().and_then(|t| t.first()).map(|c| c.mode.as_str()) {
            Some("Cable") => "Cable",
            Some("Satellite") => "Satellite",
            _ => "Antenna",
        }
    }

    pub fn chip_index(&self) -> usize {
        self.chip
    }

    pub fn region(&self) -> Region {
        self.region
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn channel(&self, row: usize) -> Option<&Channel> {
        self.rows.get(row).and_then(|i| self.all.get(*i))
    }

    pub fn row(&self) -> usize {
        self.row
    }

    /// The instant the focus is at.
    pub fn at(&self) -> i64 {
        self.at
    }

    /// The first row on the screen and the left edge of the grid once what the focus asked of the view has settled.
    pub fn top(&self) -> usize {
        self.top
    }

    pub fn view(&self) -> i64 {
        self.view
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn now(&self) -> i64 {
        self.now
    }

    pub fn offset_s(&self) -> i32 {
        self.offset_s
    }

    /// The instant of the grid's left edge at x = 0 of its content, and the end of what can be shown.
    pub fn origin(&self) -> i64 {
        self.origin
    }

    pub fn horizon(&self) -> i64 {
        self.origin + PAST + REACH
    }

    pub fn sheet_open(&self) -> bool {
        self.sheet
    }

    pub fn list_fault(&self) -> Option<&Fault> {
        self.list_fault.as_ref()
    }

    pub fn data_fault(&self) -> Option<&Fault> {
        self.data_fault.as_ref()
    }

    /// Whether a list the filter needs is still on its way.
    pub fn loading_list(&self) -> bool {
        match self.filter {
            Filter::Internet => self.net.is_none() && self.net_fault.is_none(),
            Filter::All => (self.tuner.is_none() && self.list_fault.is_none()) || (self.net.is_none() && self.net_fault.is_none() && self.net_asked),
            _ => self.tuner.is_none() && self.list_fault.is_none(),
        }
    }

    /// The programmes held for a channel, by start.
    pub fn programmes(&self, channel: &str) -> &[Programme] {
        self.store.get(channel).map_or(&[], Vec::as_slice)
    }

    fn covered(&self, channel: &str, a: i64, b: i64) -> bool {
        self.cover.get(channel).is_some_and(|v| v.iter().any(|(s, e)| *s <= a && b <= *e))
    }

    /// The stretch of `channel`'s row that holds the instant `t`: the programme on the air then, or the gap round it.
    pub fn span_at(&self, channel: &str, t: i64) -> Span<'_> {
        let list = self.programmes(channel);
        let i = list.partition_point(|p| p.end <= t);
        if let Some(p) = list.get(i).filter(|p| p.start <= t) {
            return Span { start: p.start, end: p.end, programme: Some(p), loading: false };
        }
        let prev = i.checked_sub(1).and_then(|j| list.get(j));
        let next = list.get(i);
        let start = prev.map_or(self.origin, |p| p.end);
        let end = next.map_or(self.horizon(), |p| p.start);
        let probe = t.clamp(start, (end - 1).max(start));
        Span { start, end, programme: None, loading: !self.covered(channel, probe - probe.rem_euclid(30 * MIN), probe - probe.rem_euclid(30 * MIN) + 30 * MIN) }
    }

    /// The stretches of a row that meet `[from, to)`, programmes and gaps in order, so that they tile it. A gap is the whole of the stretch
    /// between the programme before it (or the start of the grid) and the one after it (or the end), whatever part of it is asked for; whether
    /// it is still being loaded is told of the part that is.
    pub fn spans(&self, channel: &str, from: i64, to: i64) -> Vec<Span<'_>> {
        let list = self.programmes(channel);
        let mut out = Vec::new();
        let first = list.partition_point(|p| p.end <= from);
        let mut cursor = first.checked_sub(1).and_then(|j| list.get(j)).map_or(self.origin, |p| p.end.max(self.origin));
        let gap = |start: i64, end: i64| Span { start, end, programme: None, loading: !self.covered(channel, start.max(from), end.min(to)) };
        let mut next = None;
        for p in &list[first..] {
            if p.start >= to {
                next = Some(p);
                break;
            }
            if p.start > cursor && p.start > from {
                out.push(gap(cursor, p.start));
            }
            out.push(Span { start: p.start, end: p.end, programme: Some(p), loading: false });
            cursor = cursor.max(p.end);
        }
        if cursor < to {
            out.push(gap(cursor, next.map_or(self.horizon(), |p| p.start)));
        }
        out
    }

    /// The stretch in focus.
    pub fn focus_span(&self) -> Option<Span<'_>> {
        let c = self.channel(self.row)?;
        Some(self.span_at(&c.id, self.at))
    }

    /// The stretch of time the focus covers: the programme in focus, or for a gap the hour of it.
    pub fn focus_extent(&self) -> Option<(i64, i64)> {
        let c = self.channel(self.row)?;
        Some(Self::extent_of(&self.span_at(&c.id, self.at), self.at))
    }

    fn extent_of(s: &Span<'_>, at: i64) -> (i64, i64) {
        match s.programme {
            Some(_) => (s.start, s.end),
            None => {
                let hour = floor_to(at, HOUR);
                let start = s.start.max(hour);
                (start, s.end.min(hour + HOUR).max(start + MIN))
            }
        }
    }

    /// The broadcast's wall clock, in seconds east of UTC: the TV's zone until a programme's own times say another.
    pub fn set_offset(&mut self, offset_s: i32) {
        self.offset_s = offset_s;
    }

    /// The sheet's words, when it is open.
    pub fn sheet(&self) -> Option<Sheet<'_>> {
        if !self.sheet {
            return None;
        }
        let channel = self.channel(self.row)?;
        let span = self.span_at(&channel.id, self.at);
        let list = self.programmes(&channel.id);
        let after: Vec<&Programme> = list.iter().filter(|p| p.start >= span.end).take(2).collect();
        let (main, when) = match span.programme {
            Some(p) if p.airing(self.now) => (Some(p), "On now"),
            Some(p) if p.start > self.now => (Some(p), if list.iter().any(|q| q.airing(self.now)) && list.iter().find(|q| q.airing(self.now)).is_some_and(|q| q.end == p.start) { "Next" } else { "Later" }),
            Some(p) => (Some(p), "Earlier"),
            None => (None, ""),
        };
        Some(Sheet { channel, main, when, after, now: self.now })
    }

    // ------------------------------------------------------------------------------------------------------------------------ life

    pub fn open(&mut self, now: u64) -> Vec<Effect> {
        if self.open {
            return Vec::new();
        }
        self.open = true;
        self.last_input = now;
        self.moved = false;
        self.sheet = false;
        self.region = Region::Grid;
        self.set_clock(now);
        self.offset_s = clock::system_offset_s(now);
        self.origin = floor_to(self.now, HOUR) - PAST;
        self.view = floor_to(self.now, 30 * MIN).clamp(self.origin, self.horizon() - self.page.span);
        self.at = self.now;
        self.top = 0;
        self.row = 0;
        if now.saturating_sub(self.data_at) > STALE_MS {
            self.broadcast.clear();
            self.online.clear();
            self.store.clear();
            self.epoch += 1;
            self.cover.clear();
            self.retry.clear();
            self.data_fault = None;
        }
        self.data_at = now;
        self.flight.clear();
        self.online_flight.clear();
        self.online_off = false;
        self.tuning = None;
        self.tune_fault = false;
        self.last = None;
        let _ = self.life.show(now);
        let mut out = vec![Effect::Log("guide: opened".into()), Effect::Redraw];
        out.push(Effect::Luna { tag: TAG_LIST, uri: ch::LIST_URI.into(), payload: ch::list_request(Source::Tuner), subscribe: false });
        out.push(Effect::Luna { tag: TAG_LAST, uri: ch::LAST_URI.into(), payload: ch::last_request(false), subscribe: false });
        if self.filter != Filter::Antenna && !self.net_asked {
            out.extend(self.ask_net());
        }
        out
    }

    pub fn close(&mut self, why: &str, now: u64) -> Vec<Effect> {
        if !self.open {
            return Vec::new();
        }
        self.open = false;
        self.tuning = None;
        self.tune_fault = false;
        self.sheet = false;
        self.fetch_due = None;
        let _ = self.life.hide(now);
        let mut out = vec![Effect::Log(format!("guide: closed ({why})")), Effect::Redraw];
        for f in std::mem::take(&mut self.flight) {
            out.push(Effect::Cancel { tag: f.tag });
        }
        self.online_flight.clear();
        out
    }

    fn set_clock(&mut self, now: u64) {
        self.now = now as i64;
        self.minute = self.now.div_euclid(MIN);
    }

    fn ask_net(&mut self) -> Vec<Effect> {
        if self.net_asked {
            return Vec::new();
        }
        self.net_asked = true;
        vec![Effect::Luna { tag: TAG_LIST_NET, uri: ch::LIST_URI.into(), payload: ch::list_request(Source::Internet), subscribe: false }]
    }

    /// The surface says what fits: the rows and the span of the grid. The focus is kept in view when it changed.
    pub fn set_page(&mut self, page: Page) -> bool {
        let page = Page { rows: page.rows.max(1), span: page.span.max(HOUR) };
        if page == self.page {
            return false;
        }
        self.page = page;
        self.reveal();
        self.fetch_due.get_or_insert(self.now as u64);
        true
    }

    // ---------------------------------------------------------------------------------------------------------------------- the rows

    fn rebuild_all(&mut self) {
        let mut all: Vec<Channel> = Vec::new();
        all.extend(self.tuner.iter().flatten().cloned());
        all.extend(self.net.iter().flatten().cloned());
        self.all = ch::merge(all);
    }

    /// The rows the filter shows, keeping the focus on the channel it was on when that is still there.
    fn rebuild_rows(&mut self) {
        let keep = self.channel(self.row).map(|c| c.id.clone());
        let filter = self.filter;
        self.rows = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, c)| match filter {
                Filter::All => true,
                Filter::Antenna => c.source == Source::Tuner,
                Filter::Internet => c.source == Source::Internet,
                Filter::Favourites => c.favourite(),
            })
            .map(|(i, _)| i)
            .collect();
        let at = keep.and_then(|id| self.rows.iter().position(|i| self.all[*i].id == id)).unwrap_or(0);
        self.row = at;
        self.top = self.top.min(self.rows.len().saturating_sub(1));
        self.reveal();
    }

    pub fn set_filter(&mut self, filter: Filter) -> Vec<Effect> {
        if self.filter == filter {
            return Vec::new();
        }
        self.filter = filter;
        let mut out = vec![Effect::Redraw];
        if matches!(filter, Filter::Internet | Filter::All | Filter::Favourites) {
            out.extend(self.ask_net());
        }
        self.rebuild_rows();
        self.fetch_due = Some(self.now as u64);
        out
    }

    // ------------------------------------------------------------------------------------------------------------------------ answers

    fn on_luna(&mut self, tag: u64, payload: &Value, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match tag {
            TAG_LIST => match ch::parse_channels(payload, Source::Tuner) {
                Ok(list) => {
                    self.list_fault = None;
                    let empty = list.is_empty();
                    self.tuner = Some(list);
                    self.rebuild_all();
                    if empty && self.filter == Filter::Antenna {
                        // a set with no tuner channels shows what it has
                        self.filter = Filter::Internet;
                        out.extend(self.ask_net());
                    }
                    self.rebuild_rows();
                    self.focus_last();
                    self.fetch_due = Some(now);
                }
                Err(f) => {
                    self.list_fault = Some(f);
                    self.tuner = None;
                }
            },
            TAG_LIST_NET => match ch::parse_channels(payload, Source::Internet) {
                Ok(list) => {
                    self.net_fault = None;
                    self.net = Some(list);
                    self.rebuild_all();
                    self.rebuild_rows();
                    self.focus_last();
                    self.fetch_due = Some(now);
                }
                Err(f) => {
                    self.net_fault = Some(f);
                    self.net_asked = false;
                }
            },
            TAG_LAST => {
                self.last = ch::parse_last(payload);
                if let Some(id) = self.last.clone()
                    && !self.moved
                    && self.filter == Filter::Antenna
                    && id.starts_with("11_")
                {
                    // the channel the TV was on is an internet one: the guide opens on the internet's list
                    self.filter = Filter::Internet;
                    out.extend(self.ask_net());
                    self.rebuild_rows();
                }
                self.focus_last();
            }
            TAG_TUNE => {
                self.tuning = None;
                if payload.get("returnValue").and_then(Value::as_bool) == Some(true) {
                    out.extend(self.close("a channel was picked", now));
                } else {
                    self.tune_fault = true;
                }
            }
            t if (TAG_EVENTS..TAG_EVENTS + 0x100).contains(&t) => self.on_events(t, payload, now),
            _ => {}
        }
        out.push(Effect::Redraw);
        out
    }

    /// The focus goes to the channel the TV was last on, until the person has moved it.
    fn focus_last(&mut self) {
        if self.moved {
            return;
        }
        let Some(id) = self.last.as_deref() else { return };
        if let Some(r) = self.rows.iter().position(|i| self.all[*i].id == id) {
            self.row = r;
            self.top = r.saturating_sub(self.page.rows / 2).min(self.rows.len().saturating_sub(self.page.rows));
            self.reveal();
        }
    }

    fn on_events(&mut self, tag: u64, payload: &Value, now: u64) {
        let Some(i) = self.flight.iter().position(|f| f.tag == tag) else { return };
        let f = self.flight.remove(i);
        match ch::parse_programmes(payload, self.offset_s) {
            Ok(list) => {
                self.data_fault = None;
                if let Some(off) = ch::offset_of(payload)
                    && off != self.offset_s
                {
                    // the broadcast's day is not the TV's zone: times asked for from now on are on its clock
                    self.offset_s = off;
                }
                for id in &f.channels {
                    let v = self.broadcast.entry(id.clone()).or_default();
                    v.retain(|p| p.end <= f.from || p.start >= f.to);
                    v.extend(list.iter().filter(|p| &p.channel == id).cloned());
                    v.sort_by_key(|p| p.start);
                    v.dedup_by_key(|p| p.start);
                    self.lay(id);
                    let c = self.cover.entry(id.clone()).or_default();
                    c.push((f.from, f.to));
                    c.sort_unstable();
                    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(c.len());
                    for (s, e) in c.iter().copied() {
                        match merged.last_mut() {
                            Some(last) if s <= last.1 => last.1 = last.1.max(e),
                            _ => merged.push((s, e)),
                        }
                    }
                    *c = merged;
                }
            }
            Err(fault) => {
                for id in &f.channels {
                    self.retry.insert(id.clone(), now + RETRY_MS);
                }
                self.data_fault = Some(fault);
            }
        }
        for id in &f.channels {
            *self.revs.entry(id.clone()).or_default() += 1;
        }
        self.fetch_due = Some(now);
    }

    /// The helper's answer to an online call: its programmes for the call's channels and window, or word that its plugin is off (which
    /// takes back what it gave before).
    fn on_online(&mut self, tag: u64, status: u16, data: &Value) -> Vec<Effect> {
        let Some(i) = self.online_flight.iter().position(|f| f.tag == tag) else { return Vec::new() };
        let f = self.online_flight.remove(i);
        if status != 200 || data.get("on").and_then(Value::as_bool) != Some(true) {
            self.online_off = true;
            let had: Vec<String> = self.online.drain().map(|(id, _)| id).collect();
            for id in &had {
                self.lay(id);
            }
            return if had.is_empty() { Vec::new() } else { vec![Effect::Redraw] };
        }
        let list: Vec<Programme> = data.get("programmes").and_then(|p| serde_json::from_value(p.clone()).ok()).unwrap_or_default();
        for id in &f.channels {
            let v = self.online.entry(id.clone()).or_default();
            v.retain(|p| p.end <= f.from || p.start >= f.to);
            v.extend(list.iter().filter(|p| &p.channel == id).cloned());
            v.sort_by_key(|p| p.start);
            v.dedup_by_key(|p| p.start);
            self.lay(id);
        }
        vec![Effect::Redraw]
    }

    /// What is drawn of `channel`: the online programmes over LG's.
    fn lay(&mut self, channel: &str) {
        let none = Vec::new();
        let online = self.online.get(channel).unwrap_or(&none);
        let broadcast = self.broadcast.get(channel).unwrap_or(&none);
        let laid = lumaguide::merge::over(online, broadcast);
        self.store.insert(channel.to_string(), laid);
        *self.revs.entry(channel.to_string()).or_default() += 1;
    }

    // ------------------------------------------------------------------------------------------------------------------------ asking

    /// The hours of a row worth having: the ones on the screen and a margin, the row in focus with more ahead of it.
    fn window(&self, row: usize) -> (i64, i64) {
        let (before, after) = if row == self.row { (HOUR, 2 * HOUR) } else { (30 * MIN, HOUR) };
        (floor_to(self.view - before, 30 * MIN).max(self.origin), (self.view + self.page.span + after).min(self.horizon()))
    }

    /// The part of `[a, b)` of `channel` that is neither held nor on its way: its first and last missing instants.
    fn missing(&self, channel: &str, a: i64, b: i64) -> Option<(i64, i64)> {
        let mut holes = vec![(a, b)];
        let mut cut = |s: i64, e: i64| {
            holes = holes
                .iter()
                .flat_map(|&(h0, h1)| {
                    if e <= h0 || s >= h1 {
                        vec![(h0, h1)]
                    } else {
                        let mut v = Vec::new();
                        if s > h0 {
                            v.push((h0, s));
                        }
                        if e < h1 {
                            v.push((e, h1));
                        }
                        v
                    }
                })
                .collect();
        };
        for (s, e) in self.cover.get(channel).into_iter().flatten() {
            cut(*s, *e);
        }
        for f in self.flight.iter().filter(|f| f.channels.iter().any(|c| c == channel)) {
            cut(f.from, f.to);
        }
        Some((holes.first()?.0, holes.last()?.1))
    }

    /// The calls to make now: the row in focus first, then the rows round it, a few channels and at most six hours to a call.
    fn plan(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.open || self.rows.is_empty() {
            return out;
        }
        while self.flight.len() < IN_FLIGHT {
            let lo = self.top.saturating_sub(1);
            let hi = (self.top + self.page.rows + 1).min(self.rows.len());
            let mut order: Vec<usize> = (lo..hi).collect();
            if !order.contains(&self.row) && self.row < self.rows.len() {
                order.push(self.row);
            }
            order.sort_by_key(|r| (r.abs_diff(self.row), *r));
            let mut lead: Option<(i64, i64)> = None;
            let mut channels: Vec<String> = Vec::new();
            for r in order {
                let id = self.all[self.rows[r]].id.clone();
                if self.retry.get(&id).is_some_and(|t| *t > now) {
                    continue;
                }
                let (w0, w1) = self.window(r);
                let Some((m0, m1)) = self.missing(&id, w0, w1) else { continue };
                match lead {
                    None => {
                        let from = floor_to(m0, 30 * MIN).max(self.origin);
                        let to = (from + CALL_SPAN).min(floor_to(m1 + 30 * MIN - 1, 30 * MIN)).max(from + 30 * MIN);
                        lead = Some((from, to));
                        channels.push(id);
                    }
                    Some((from, to)) => {
                        if channels.len() < BATCH && m0 < to && m1 > from {
                            channels.push(id);
                        }
                    }
                }
            }
            let Some((from, to)) = lead else { break };
            self.seq = (self.seq + 1) % 0x100;
            let tag = TAG_EVENTS + self.seq;
            let ids: Vec<&str> = channels.iter().map(String::as_str).collect();
            out.push(Effect::Luna { tag, uri: ch::EVENTS_URI.into(), payload: ch::events_request(&ids, from, to, self.offset_s), subscribe: false });
            if !self.online_off {
                let mut path = format!("{ONLINE_PATH}?from={from}&to={to}");
                for id in &ids {
                    path.push_str(&format!("&channel={id}"));
                }
                out.push(Effect::Fetch { tag: TAG_ONLINE + self.seq, path });
                self.online_flight.retain(|f| f.tag != TAG_ONLINE + self.seq);
                self.online_flight.push(Flight { tag: TAG_ONLINE + self.seq, channels: channels.clone(), from, to, at: now });
            }
            self.flight.push(Flight { tag, channels, from, to, at: now });
        }
        out
    }

    // ------------------------------------------------------------------------------------------------------------------------- the focus

    /// The view moves only as far as the focus needs: a cell that does not fit is shown from its start; a clearance of a hundredth of the
    /// span is left (the 12 px of `focus.scrollClearance` at 1920).
    fn reveal(&mut self) {
        if self.region == Region::Grid
            && let Some((cs, ce)) = self.focus_extent()
        {
            let clr = self.page.span / 100;
            if cs < self.view {
                self.view = cs - clr;
            } else if ce > self.view + self.page.span {
                self.view = if ce - cs + 2 * clr <= self.page.span { ce + clr - self.page.span } else { cs - clr };
            }
            self.view = self.view.clamp(self.origin, (self.horizon() - self.page.span).max(self.origin));
        }
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + self.page.rows {
            self.top = self.row + 1 - self.page.rows;
        }
        self.top = self.top.min(self.rows.len().saturating_sub(self.page.rows.min(self.rows.len())));
        if self.row < self.top {
            self.top = self.row;
        }
    }

    fn touched(&mut self, now: u64) {
        self.last_input = now;
        self.moved = true;
        self.fetch_due = Some(now + FETCH_DEBOUNCE_MS);
    }

    /// A step along the row: the neighbouring cell (a gap goes an hour at a time).
    fn step_time(&mut self, dir: i64) -> bool {
        let Some(c) = self.channel(self.row).map(|c| c.id.clone()) else { return false };
        let (lo, hi) = (self.origin, self.horizon());
        let (es, ee) = Self::extent_of(&self.span_at(&c, self.at), self.at);
        let to = if dir > 0 {
            if ee >= hi {
                return false;
            }
            ee
        } else {
            if es <= lo {
                return false;
            }
            Self::extent_of(&self.span_at(&c, es - 1), es - 1).0.max(lo)
        };
        let changed = to != self.at;
        self.at = to;
        changed
    }

    fn step_row(&mut self, delta: isize) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let to = (self.row as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        if to == self.row {
            return false;
        }
        // the instant stays inside what is on the screen
        self.at = self.at.clamp(self.view, self.view + self.page.span - 1);
        self.row = to;
        true
    }

    fn move_to_chips(&mut self) {
        self.region = Region::Chips;
        let chips = self.chips();
        self.chip = chips.iter().position(|c| *c == Chip::Filter(self.filter)).unwrap_or(0);
    }

    fn leave_chips(&mut self) {
        self.region = Region::Grid;
        self.reveal();
    }

    /// A chip chosen (OK, or a click): a filter shows its channels, Now goes back to the programmes on the air.
    fn pick(&mut self, chip: Chip) -> Vec<Effect> {
        match chip {
            Chip::Filter(f) => self.set_filter(f),
            Chip::Now => {
                self.leave_chips();
                self.at = self.now;
                self.view = floor_to(self.now, 30 * MIN).clamp(self.origin, (self.horizon() - self.page.span).max(self.origin));
                self.reveal();
                vec![Effect::Redraw]
            }
        }
    }

    // -------------------------------------------------------------------------------------------------------------------------- keys

    pub fn key(&mut self, name: &str, down: bool, repeat: bool, now: u64) -> Vec<Effect> {
        if !self.open || !down {
            return Vec::new();
        }
        self.last_input = now;
        if self.tuning.is_some() && name != "back" {
            // the channel is being switched: only Back (to leave at once) is heard
            return Vec::new();
        }
        let nav = matches!(name, "up" | "down" | "left" | "right" | "chup" | "chdown" | "wheelup" | "wheeldown");
        if repeat && !nav {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.sheet {
            match name {
                "back" | "info" => self.sheet = false,
                "ok" => return self.watch(now),
                _ => return Vec::new(),
            }
            out.push(Effect::Redraw);
            return out;
        }
        let changed = match (self.region, name) {
            (_, "back") => return self.close("Back", now),
            (Region::Chips, "ok") => {
                let chip = self.chips().get(self.chip).copied();
                out.extend(chip.map_or_else(Vec::new, |c| self.pick(c)));
                false
            }
            (Region::Chips, "left") => {
                let moved = self.chip > 0;
                self.chip = self.chip.saturating_sub(1);
                moved
            }
            (Region::Chips, "right") => {
                let moved = self.chip + 1 < self.chips().len();
                if moved {
                    self.chip += 1;
                }
                moved
            }
            (Region::Chips, "down") => {
                self.leave_chips();
                true
            }
            (Region::Grid, "left") => {
                let moved = self.step_time(-1);
                self.reveal();
                moved
            }
            (Region::Grid, "right") => {
                let moved = self.step_time(1);
                self.reveal();
                moved
            }
            (Region::Grid, "up") => {
                if self.row == 0 {
                    self.move_to_chips();
                    true
                } else {
                    let moved = self.step_row(-1);
                    self.reveal();
                    moved
                }
            }
            (Region::Grid, "down") => {
                let moved = self.step_row(1);
                self.reveal();
                moved
            }
            (Region::Grid, "chup") => {
                let moved = self.step_row(-(self.page.rows.saturating_sub(1).max(1) as isize));
                self.reveal();
                moved
            }
            (Region::Grid, "chdown") => {
                let moved = self.step_row(self.page.rows.saturating_sub(1).max(1) as isize);
                self.reveal();
                moved
            }
            // a notch of the Magic Remote's wheel is one row, and never leaves the grid for the chips
            (Region::Grid, "wheelup") => {
                let moved = self.step_row(-1);
                self.reveal();
                moved
            }
            (Region::Grid, "wheeldown") => {
                let moved = self.step_row(1);
                self.reveal();
                moved
            }
            (Region::Grid, "info") => {
                if self.channel(self.row).is_some() {
                    self.sheet = true;
                }
                true
            }
            (Region::Grid, "ok") => return self.activate(now),
            _ => false,
        };
        self.touched(now);
        if changed {
            out.push(Effect::Redraw);
        }
        out
    }

    /// OK on the grid: a programme on the air (or a stretch with nothing in it) tunes; one that is not on the air opens the sheet.
    fn activate(&mut self, now: u64) -> Vec<Effect> {
        let Some(c) = self.channel(self.row).map(|c| c.id.clone()) else { return Vec::new() };
        let on_air = self.span_at(&c, self.at).programme.is_none_or(|p| p.airing(self.now));
        if on_air {
            return self.watch(now);
        }
        self.sheet = true;
        self.touched(now);
        vec![Effect::Redraw]
    }

    /// Tunes the channel of the row in focus, as LG's guide does, and leaves.
    pub fn watch(&mut self, now: u64) -> Vec<Effect> {
        let Some(c) = self.channel(self.row) else { return Vec::new() };
        if self.tuning.is_some() {
            return Vec::new();
        }
        let (id, label) = (c.id.clone(), format!("{} {}", c.number, c.name));
        self.tuning = Some(now);
        self.tune_fault = false;
        self.sheet = false;
        self.last_input = now;
        vec![Effect::Log(format!("guide: tuning {label}")), Effect::Luna { tag: TAG_TUNE, uri: ch::LAUNCH_URI.into(), payload: ch::tune_request(&id), subscribe: false }, Effect::Redraw]
    }

    /// A click or the pointer's press: the target is focused and then acted on (hover is a cue only; the surface keeps it).
    pub fn click(&mut self, target: Target, now: u64) -> Vec<Effect> {
        if !self.open {
            return Vec::new();
        }
        self.last_input = now;
        match target {
            Target::Chip(i) => {
                self.region = Region::Chips;
                self.chip = i;
                let chip = self.chips().get(i).copied();
                self.touched(now);
                let mut out = vec![Effect::Redraw];
                if let Some(c) = chip {
                    out.extend(self.pick(c));
                    self.region = Region::Grid;
                }
                out
            }
            Target::Cell { row, at } => {
                if self.sheet || row >= self.rows.len() {
                    return Vec::new();
                }
                let same = self.region == Region::Grid && self.row == row && self.span_same(row, at);
                self.region = Region::Grid;
                self.row = row;
                self.at = at;
                self.reveal();
                self.touched(now);
                if same { self.activate(now) } else { vec![Effect::Redraw] }
            }
            Target::Rail(row) => {
                if self.sheet || row >= self.rows.len() {
                    return Vec::new();
                }
                self.region = Region::Grid;
                self.row = row;
                self.at = self.at.clamp(self.view, self.view + self.page.span - 1);
                self.reveal();
                self.touched(now);
                vec![Effect::Redraw]
            }
            Target::Watch => self.watch(now),
            Target::Dismiss => {
                self.sheet = false;
                vec![Effect::Redraw]
            }
        }
    }

    fn span_same(&self, row: usize, at: i64) -> bool {
        let Some(c) = self.channel(row) else { return false };
        let (a, b) = (self.span_at(&c.id, self.at), self.span_at(&c.id, at));
        (a.start, a.end) == (b.start, b.end)
    }

    // ---------------------------------------------------------------------------------------------------------------------- the clock

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Luna { tag, payload } => self.on_luna(*tag, payload, now),
            Input::Fetched { tag, status, data } if (TAG_ONLINE..TAG_ONLINE + 0x100).contains(tag) => self.on_online(*tag, *status, data),
            Input::Key { name, down, repeat } => self.key(name, *down, *repeat, now),
            Input::Tick => self.tick(now),
            Input::Topic { topic, .. } if topic == "homekey" && self.open => self.close("Home", now),
            _ => Vec::new(),
        }
    }

    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(Change::Opened | Change::Parked | Change::Drop | Change::Hide | Change::Show | Change::Build) = self.life.tick(now) {
            out.push(Effect::Redraw);
        }
        if !self.open {
            return out;
        }
        if self.tuning.is_some_and(|t| now.saturating_sub(t) >= TUNE_WAIT_MS) {
            // LG's launch has not said no: the switch of the app is its answer
            out.extend(self.close("a channel was picked", now));
            return out;
        }
        if now.saturating_sub(self.last_input) >= IDLE_MS {
            out.extend(self.close("idle", now));
            return out;
        }
        if now as i64 / MIN != self.minute {
            // the now marker and the clock move a minute
            self.set_clock(now);
            self.data_at = now;
            out.push(Effect::Redraw);
        }
        self.online_flight.retain(|f| now.saturating_sub(f.at) < FLIGHT_MS);
        let late: Vec<Flight> = self.flight.iter().filter(|f| now.saturating_sub(f.at) >= FLIGHT_MS).cloned().collect();
        if !late.is_empty() {
            self.flight.retain(|f| now.saturating_sub(f.at) < FLIGHT_MS);
            for f in late {
                for id in &f.channels {
                    self.retry.insert(id.clone(), now + RETRY_MS);
                    *self.revs.entry(id.clone()).or_default() += 1;
                }
                out.push(Effect::Cancel { tag: f.tag });
            }
            self.fetch_due = Some(now);
        }
        if self.fetch_due.is_some_and(|t| now >= t) {
            self.fetch_due = None;
            out.extend(self.plan(now));
        } else if self.fetch_due.is_none() && self.retry.values().any(|t| *t <= now) {
            self.retry.retain(|_, t| *t > now);
            out.extend(self.plan(now));
        }
        out
    }

    /// When the guide next has something to do. `tick` does nothing for a closed guide but the panel's own timer, so a fetch must not be
    /// reported then: `set_page` leaves one owing when the surface is built, with the model's clock at 0, and reported it was due for good, the
    /// shell ticked and committed Home's poses at full speed, and the compositor drew a frame at every refresh of a Home at rest.
    pub fn due_at(&self) -> Option<u64> {
        let fetch = self.open.then_some(self.fetch_due).flatten();
        let minute = self.open.then(|| ((self.minute + 1) * MIN) as u64);
        let idle = self.open.then(|| self.last_input + IDLE_MS);
        let retry = self.open.then(|| self.retry.values().min().copied()).flatten();
        let flight = self.open.then(|| self.flight.iter().map(|f| f.at + FLIGHT_MS).min()).flatten();
        let tuning = self.tuning.map(|t| t + TUNE_WAIT_MS);
        [self.life.due_at(), fetch, minute, idle, retry, flight, tuning].into_iter().flatten().min()
    }

    /// What the gate and the harness read.
    pub fn state(&self) -> Value {
        let c = self.channel(self.row);
        json!({
            "open": self.open, "phase": format!("{:?}", self.life.phase()).to_lowercase(), "filter": self.filter.id(),
            "region": match self.region { Region::Chips => "chips", Region::Grid => "grid", Region::Sheet => "sheet" },
            "rows": self.rows.len(), "row": self.row, "top": self.top, "channel": c.map(|c| c.id.as_str()), "number": c.map(|c| c.number.as_str()),
            "at": self.at, "view": self.view, "sheet": self.sheet, "inFlight": self.flight.len(), "tuning": self.tuning.is_some(), "tuneFault": self.tune_fault,
            "held": self.store.values().map(Vec::len).sum::<usize>(), "online": self.online.values().map(Vec::len).sum::<usize>(),
        })
    }
}
